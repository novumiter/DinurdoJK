//! Runtime shader remapping (OpenJK/TaystJK `R_RemapShader`).
//!
//! Remaps are resolved on the cold mutation path. The resulting `PreparedMap`
//! contains ordinary direct draw batches, so there is no shader-name lookup or
//! remap branch in the renderer's per-frame/per-draw hot path.

use super::{
    material_batches::append_material_batches, materials, AssetSearchPath, BTreeMap, DrawBatch,
    PreparedMap, SurfaceMaterial, Textures,
};
use crate::materials::StageTexture;
use std::path::Path;

/// Canonical renderer shader name. `R_RemapShader` compares the old shader
/// after `COM_StripExtension`, while `R_FindShader` is slash/case insensitive.
pub(crate) fn canonical_shader_name(name: &str) -> String {
    let mut key = name.trim().replace('\\', "/").to_ascii_lowercase();
    let file_start = key.rfind('/').map_or(0, |slash| slash + 1);
    if let Some(dot) = key[file_start..].rfind('.') {
        key.truncate(file_start + dot);
    }
    key
}

/// Renderer-owned logical state corresponding to shader_t::remappedShader and
/// shader_t::timeOffset. Redirects are intentionally one hop: id Tech 3 swaps
/// the shader once when the draw surface is submitted; it does not recursively
/// chase remap chains.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ShaderRemapState {
    redirects: BTreeMap<String, String>,
    time_offsets: BTreeMap<String, f32>,
}

impl ShaderRemapState {
    pub(crate) fn clear(&mut self) {
        self.redirects.clear();
        self.time_offsets.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.redirects.is_empty() && self.time_offsets.is_empty()
    }

    /// Port of R_RemapShader's state mutation. `time_offset=None` is the local
    /// `/remapShader` command and therefore leaves the target shader's existing
    /// animation offset untouched. `old == new` clears only the redirect.
    pub(crate) fn apply(&mut self, old: &str, new: &str, time_offset: Option<f32>) -> bool {
        let old = canonical_shader_name(old);
        let new = canonical_shader_name(new);
        if old.is_empty() || new.is_empty() {
            return false;
        }
        let before_redirect = self.redirects.get(&old).cloned();
        let before_offset = self.time_offsets.get(&new).copied();
        if old == new {
            self.redirects.remove(&old);
        } else {
            self.redirects.insert(old.clone(), new.clone());
        }
        if let Some(offset) = time_offset.filter(|value| value.is_finite()) {
            self.time_offsets.insert(new.clone(), offset);
        }
        before_redirect != self.redirects.get(&old).cloned()
            || before_offset != self.time_offsets.get(&new).copied()
    }

    pub(crate) fn effective_name<'a>(&'a self, source: &'a str) -> std::borrow::Cow<'a, str> {
        if self.redirects.is_empty() {
            return std::borrow::Cow::Borrowed(source);
        }
        let key = canonical_shader_name(source);
        match self.redirects.get(&key) {
            Some(target) => std::borrow::Cow::Borrowed(target.as_str()),
            None if key == source => std::borrow::Cow::Borrowed(source),
            None => std::borrow::Cow::Owned(key),
        }
    }

    pub(crate) fn time_offset_for(&self, effective_shader: &str) -> f32 {
        self.time_offsets
            .get(&canonical_shader_name(effective_shader))
            .copied()
            .unwrap_or(0.0)
    }

    pub(crate) fn has_redirect_for(&self, source: &str) -> bool {
        self.redirects.contains_key(&canonical_shader_name(source))
    }
}

fn offset_index(index: &mut Option<usize>, base: usize) {
    if let Some(index) = index {
        *index += base;
    }
}

fn offset_material_textures(material: &mut SurfaceMaterial, base: usize) {
    for stage in &mut material.stages {
        if let StageTexture::Image(index) = &mut stage.texture {
            *index += base;
        }
        offset_index(&mut stage.enhancements.normal_texture, base);
        offset_index(&mut stage.enhancements.roughness_texture, base);
        offset_index(&mut stage.enhancements.height_texture, base);
        offset_index(&mut stage.enhancements.metallic_texture, base);
        offset_index(&mut stage.enhancements.specular_texture, base);
        offset_index(&mut stage.enhancements.emissive_texture, base);
    }
    if let Some(skybox) = &mut material.skybox {
        for texture in skybox {
            *texture += base;
        }
    }
    for sprite in &mut material.surface_sprite_effects {
        sprite.texture += base;
    }
}

fn same_material_geometry(a: &DrawBatch, b: &DrawBatch) -> bool {
    a.vertices == b.vertices
        && a.bsp_shader_index == b.bsp_shader_index
        && a.material_debug_index == b.material_debug_index
        && a.vertex_lit == b.vertex_lit
        && a.lightmap == b.lightmap
        && a.pvs_signature == b.pvs_signature
        && a.area_signature == b.area_signature
        && a.fog == b.fog
        && a.fog_is_global == b.fog_is_global
}

fn batch_shader_name<'a>(map: &'a PreparedMap, batch: &DrawBatch) -> Option<&'a str> {
    usize::try_from(batch.material_debug_index)
        .ok()
        .filter(|&index| index != u32::MAX as usize)
        .and_then(|index| map.material_debug.entries.get(index))
        .map(|entry| entry.name.as_str())
}

fn copy_geometry_specialization(source: &DrawBatch, replacement: &mut DrawBatch) {
    // These are geometric/map-position facts, not authored stage state. Preserve
    // them when the replacement shader opts into the corresponding feature.
    replacement.planar_plane = source.planar_plane;
    replacement.planar_pvs_origin = source.planar_pvs_origin;
    replacement.reflection_probe = source.reflection_probe;
    replacement.reflection_probe_position_radius = source.reflection_probe_position_radius;
    replacement.reflection_cache_flags = source.reflection_cache_flags;
    replacement.reflection_roughness_hint = source.reflection_roughness_hint;
    replacement.authored_ocean = source.authored_ocean;
}

fn rebuild_batches(
    map: &PreparedMap,
    source: &[DrawBatch],
    state: &ShaderRemapState,
    replacements: &BTreeMap<String, SurfaceMaterial>,
) -> Vec<DrawBatch> {
    let mut out = Vec::with_capacity(source.len());
    let mut index = 0usize;
    while index < source.len() {
        let first = &source[index];
        let mut end = index + 1;
        while end < source.len() && same_material_geometry(first, &source[end]) {
            end += 1;
        }

        let source_name = batch_shader_name(map, first);
        let effective = source_name.map(|name| state.effective_name(name));
        let redirected = source_name.is_some_and(|name| state.has_redirect_for(name));
        if redirected {
            let target = effective.as_deref().unwrap_or_default();
            if let Some(material) = replacements.get(target) {
                let start = out.len();
                append_material_batches(
                    &mut out,
                    material,
                    usize::try_from(first.bsp_shader_index).unwrap_or(usize::MAX),
                    usize::try_from(first.material_debug_index).ok(),
                    first.vertex_lit,
                    first.vertices.clone(),
                    first.lightmap,
                    &first.pvs_signature,
                    first.area_signature,
                    first.fog,
                    first.fog_is_global,
                );
                let time_offset = state.time_offset_for(target);
                for replacement in &mut out[start..] {
                    replacement.shader_time_offset = time_offset;
                    copy_geometry_specialization(first, replacement);
                }
            } else {
                // Tayst/OpenJK's R_RemapShader refuses a default/missing target
                // shader. Preserve the authored source material rather than
                // turning the surface white or dropping its draw entirely.
                let time_offset = source_name.map_or(0.0, |name| state.time_offset_for(name));
                out.extend(source[index..end].iter().cloned().map(|mut batch| {
                    batch.shader_time_offset = time_offset;
                    batch
                }));
            }
        } else {
            let time_offset = effective
                .as_deref()
                .map_or(0.0, |name| state.time_offset_for(name));
            out.extend(source[index..end].iter().cloned().map(|mut batch| {
                batch.shader_time_offset = time_offset;
                batch
            }));
        }
        index = end;
    }
    out
}

/// Re-resolve all currently active redirects into an already-prepared map.
/// The caller must supply an unremapped map: stage flattening is rebuilt from
/// its original batches so `old -> old` restores the authored material exactly.
pub(crate) fn apply_shader_remaps_in_place(
    map: &mut PreparedMap,
    root: &Path,
    game: Option<&Path>,
    options: super::MapPrepareOptions,
    state: &ShaderRemapState,
) -> Result<(), String> {
    if state.is_empty() {
        return Ok(());
    }

    let mut assets = AssetSearchPath::open_game(root, game).map_err(|error| error.to_string())?;
    assets.set_allow_asset_overrides(options.allow_asset_overrides);
    let mut textures = Textures::new();
    let texture_base = map.textures.len();
    let mut replacements = BTreeMap::<String, SurfaceMaterial>::new();

    for target in state.redirects.values() {
        if replacements.contains_key(target) {
            continue;
        }
        let definition = jka_assets::shader::find_shader(&map.shader_library, target);
        let origin_key = canonical_shader_name(target);
        let origin = map.shader_origins.get(&origin_key);
        let image_count_before = textures.images.len();
        let video_count_before = textures.videos.len();
        let mut material = materials::describe(
            target,
            0,
            definition,
            origin,
            &mut assets,
            &mut textures,
            options.gen_normal_maps,
            options.omit_environment_stages,
        );
        // R_RemapShader returns without changing anything when R_FindShader
        // produces the renderer's default shader. For an implicit shader in
        // this renderer, "no definition and no image/video was loaded" is the
        // equivalent failure signal. Explicit shader definitions remain valid
        // even when an individual authored stage has to use its normal fallback.
        let target_exists = definition.is_some()
            || textures.images.len() != image_count_before
            || textures.videos.len() != video_count_before;
        if !target_exists {
            map.warnings.push(format!(
                "shader remap: target {target} was not found; keeping the original shader"
            ));
            continue;
        }
        offset_material_textures(&mut material, texture_base);
        replacements.insert(target.clone(), material);
    }

    // Only these comparatively small flattened stage lists must be copied.
    // Geometry, collision, lightmaps and already-decoded textures stay in the
    // owned PreparedMap. This is also the path used before the first upload, so
    // an initial CS_SHADERSTATE never causes an unnecessary second world upload.
    let source_batches = map.batches.clone();
    let source_pvs_batches = map.pvs_batches.clone();
    let source_inline_batches = map.inline_batches.clone();
    map.batches = rebuild_batches(map, &source_batches, state, &replacements);
    map.pvs_batches = rebuild_batches(map, &source_pvs_batches, state, &replacements);
    map.inline_batches = rebuild_batches(map, &source_inline_batches, state, &replacements);

    if let Some(visibility) = &map.visibility {
        map.portal_draw_plan = super::visibility::build_prepared_portal_draw_plan(
            &map.batches,
            &map.pvs_batches,
            visibility,
            &map.vertices,
            |_| {},
        );
    } else {
        map.portal_draw_plan = Default::default();
    }

    let hashes = textures.content_hashes();
    for video in &mut textures.videos {
        video.texture += texture_base;
    }
    map.textures.extend(textures.images);
    map.videos.extend(textures.videos);
    map.texture_hashes.extend(hashes);
    map.warnings.extend(
        textures
            .warnings
            .into_iter()
            .map(|warning| format!("shader remap: {warning}")),
    );
    Ok(())
}

/// Re-resolve all currently active redirects from the immutable, unremapped
/// restart cache. Runtime remaps use this cold path; ordinary rendering remains
/// identical to a normally prepared world and pays no remap lookup per draw.
pub(crate) fn apply_shader_remaps(
    base_map: &PreparedMap,
    root: &Path,
    game: Option<&Path>,
    options: super::MapPrepareOptions,
    state: &ShaderRemapState,
) -> Result<PreparedMap, String> {
    let mut map = base_map.clone();
    apply_shader_remaps_in_place(&mut map, root, game, options, state)?;
    Ok(map)
}

/// Apply an opt-in OpenJK reverse-lightmap fold to a *derived* PreparedMap.
/// The restart cache must keep the pristine authored two-pass representation.
/// No texture parsing, BSP work or gameplay state is changed here.
pub(in crate::scene) fn fold_reverse_lightmap_batches(source: &[DrawBatch]) -> (Vec<DrawBatch>, usize) {
    let mut out = Vec::with_capacity(source.len());
    let mut folds = 0;
    let mut index = 0;
    while index < source.len() {
        let first = &source[index];
        let mut end = index + 1;
        while end < source.len() && same_material_geometry(first, &source[end]) {
            end += 1;
        }
        let group = &source[index..end];
        if group.len() >= 2
            && group[0].reverse_lightmap_fold_role == 1
            && group[1].reverse_lightmap_fold_role == 2
            && group[0].texture_is_lightmap
            && group[1].texture.is_some()
            && group[0].lightmap.is_some()
        {
            // Equivalent to stage_batch(material, second-without-blend,
            // first=true) in the existing folded forward-modulation path.
            let mut fused = group[1].clone();
            fused.pipeline.class = super::DrawClass::Opaque;
            fused.pipeline.blend = super::BlendMode::Opaque;
            fused.pipeline.depth_write = true;
            fused.modulate_lightmap = true;
            fused.detail_texture_eligible = fused.reverse_lightmap_fold_detail;
            fused.dlight_in_lightmap_stage = false;
            fused.fog_color_override = super::FogColorOverride::None;
            fused.reverse_lightmap_fold_role = 0;
            fused.reverse_lightmap_fold_detail = false;
            out.push(fused);
            for stage in &group[2..] {
                let mut stage = stage.clone();
                // The folded $lightmap pass no longer exists. As in the
                // ordinary append_material_batches wrapper, local dynamic
                // lighting must be evaluated by the first surviving stage.
                stage.dlight_in_lightmap_stage = false;
                stage.reverse_lightmap_fold_role = 0;
                out.push(stage);
            }
            folds += 1;
        } else {
            out.extend_from_slice(group);
        }
        index = end;
    }
    (out, folds)
}

pub(crate) fn apply_reverse_lightmap_collapse_in_place(map: &mut PreparedMap) -> usize {
    let (coarse, count) = fold_reverse_lightmap_batches(&map.batches);
    let (full, full_count) = fold_reverse_lightmap_batches(&map.pvs_batches);
    let (inline, inline_count) = fold_reverse_lightmap_batches(&map.inline_batches);
    map.batches = coarse;
    map.pvs_batches = full;
    map.inline_batches = inline;
    // AUTO 4 plans refer to FULL indices. A recipe count change invalidates
    // their indices, so reconstruct the plan, not just its representative.
    if let Some(visibility) = &map.visibility {
        map.portal_draw_plan = super::build_prepared_portal_draw_plan(
            &map.batches, &map.pvs_batches, visibility, &map.vertices, |_| {},
        );
    } else {
        map.portal_draw_plan = Default::default();
    }
    count + full_count + inline_count
}

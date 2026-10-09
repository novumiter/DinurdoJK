//! Worker-built promotion of legacy EFX visuals into transient lights.
//!
//! Jedi Academy content often draws visually emissive fire/electricity/flash
//! primitives without an authored `Light {}` component.  Promotion is split in
//! two phases:
//!
//! * map-load worker: scan every visible `.efx`, pick at most one driver
//!   primitive, resolve its shader stages, decode/sample the referenced image
//!   once, and build an immutable catalog;
//! * frame time: look up the already-built profile and emit/merge a transient
//!   light.  No texture decoding or image scan is needed on the normal hot path.
//!
//! Explicit `Light {}` components always win and are never augmented.

use super::{
    draw::{FxBlend, FxMaterial},
    system::{FxAutoLightCandidate, FxLight, FxLightKind},
    template::{self, AutoLightProfile, PrimType},
};
use crate::{materials::Textures, scene};
use jka_assets::{
    pk3::AssetSearchPath,
    shader::{AlphaGen, RgbGen, Shader},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

const FINAL_PROMOTION_SCORE: i16 = 7;
const MAX_EFX_BYTES: usize = 65_536;
const MAX_TEXTURE_SAMPLES: usize = 4096;
const MAX_PROMOTED_LIGHTS: usize = 64;

#[derive(Clone, Copy, Debug)]
struct MaterialLightAnalysis {
    score: i16,
    texture_color: [f32; 3],
    rgb_vertex: bool,
    rgb_const: [f32; 3],
    alpha_vertex: bool,
    alpha_const: f32,
    blend: FxBlend,
}

/// Immutable result of the map-load FX promotion pass.  Keys use the same
/// stripped/lower-case effect namespace as `FxSystem::register`, with aliases
/// for both `foo/bar` and `effects/foo/bar` because game/mod code uses both.
#[derive(Clone, Debug, Default)]
pub struct FxLightPromotionCatalog {
    effects: HashMap<String, AutoLightProfile>,
    materials: HashMap<String, Option<MaterialLightAnalysis>>,
    scanned_effects: usize,
    semantic_candidates: usize,
    promotable_effects: usize,
}

impl FxLightPromotionCatalog {
    pub fn effect_profile(&self, key: &str) -> Option<AutoLightProfile> {
        self.effects.get(&effect_key(key)).copied()
    }

    fn material_analysis(&self, shader: &str) -> Option<MaterialLightAnalysis> {
        self.materials
            .get(&scene::canonical_shader_name(shader))
            .copied()
            .flatten()
    }

    pub fn scanned_effects(&self) -> usize {
        self.scanned_effects
    }

    pub fn semantic_candidates(&self) -> usize {
        self.semantic_candidates
    }

    pub fn promotable_effects(&self) -> usize {
        self.promotable_effects
    }

    pub fn analyzed_materials(&self) -> usize {
        self.materials.len()
    }
}

/// Indexed EFX qpaths in the active VFS.  Listing is cheap because
/// `AssetSearchPath` already owns the PK3/loose-file namespace index.
pub fn effect_asset_names(assets: &AssetSearchPath) -> Vec<String> {
    let mut names = assets
        .names()
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            lower.starts_with("effects/") && lower.ends_with(".efx")
        })
        .map(|name| name.replace('\\', "/"))
        .collect::<Vec<_>>();
    names.sort_unstable_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

/// Build every promotion profile on a map worker.  `progress(done, total)` is
/// called after each EFX so the normal map-loading UI can expose the work.
pub fn build_catalog<F>(
    assets: &mut AssetSearchPath,
    shaders: &BTreeMap<String, Shader>,
    effect_names: &[String],
    mut progress: F,
) -> FxLightPromotionCatalog
where
    F: FnMut(u32, u32),
{
    let total = effect_names.len().min(u32::MAX as usize) as u32;
    let mut catalog = FxLightPromotionCatalog::default();
    let mut textures = Textures::new();

    for (index, qpath) in effect_names.iter().enumerate() {
        catalog.scanned_effects += 1;
        let relative = effect_relative_key(qpath);
        let parsed = assets
            .read(qpath, MAX_EFX_BYTES)
            .ok()
            .flatten()
            .map(|asset| template::parse_effect(&relative, &super::gp2::parse(&asset.bytes)));

        if let Some(template) = parsed {
            if let Some(profile) = template.auto_light {
                catalog.semantic_candidates += 1;
                let driver = template.primitives.get(profile.driver_primitive);
                let mut final_candidate = false;
                if let Some(driver) = driver {
                    for shader in &driver.media {
                        let key = scene::canonical_shader_name(shader);
                        let analysis = if let Some(cached) = catalog.materials.get(&key) {
                            *cached
                        } else {
                            let analyzed = analyze_shader_material(
                                assets,
                                &mut textures,
                                shaders,
                                shader,
                            );
                            catalog.materials.insert(key, analyzed);
                            analyzed
                        };
                        if analysis.is_some_and(|analysis| {
                            profile.score + analysis.score >= FINAL_PROMOTION_SCORE
                        }) {
                            final_candidate = true;
                        }
                    }
                }

                // Keep semantic candidates even when their current material is
                // rejected. A later R_RemapShader can turn the same primitive
                // into an emissive stage, and the cold remap path intentionally
                // falls back to live material analysis.
                let prefixed = format!("effects/{relative}");
                catalog.effects.insert(relative, profile);
                catalog.effects.insert(prefixed, profile);
                if final_candidate {
                    catalog.promotable_effects += 1;
                }
            }
        }

        let done = (index + 1).min(u32::MAX as usize) as u32;
        progress(done, total);
    }

    catalog
}

fn effect_relative_key(path: &str) -> String {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let stripped = normalized
        .strip_suffix(".efx")
        .unwrap_or(normalized.as_str());
    stripped
        .strip_prefix("effects/")
        .unwrap_or(stripped)
        .to_owned()
}

fn effect_key(path: &str) -> String {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    normalized
        .strip_suffix(".efx")
        .unwrap_or(normalized.as_str())
        .to_owned()
}

fn analyze_shader_material(
    assets: &mut AssetSearchPath,
    textures: &mut Textures,
    shaders: &BTreeMap<String, Shader>,
    shader_name: &str,
) -> Option<MaterialLightAnalysis> {
    let key = scene::canonical_shader_name(shader_name);
    if let Some(shader) = shaders.get(&key) {
        let mut best: Option<MaterialLightAnalysis> = None;
        for stage in shader.stages.iter().filter(|stage| {
            !stage.image.is_empty()
                && (!stage.image.starts_with('$')
                    || stage.image.eq_ignore_ascii_case("$whiteimage")
                    || stage.image.eq_ignore_ascii_case("$lightmap"))
        }) {
            let blend = FxBlend::from_blend_func(&stage.blend);
            let alpha_matters = matches!(blend, FxBlend::AddAlpha | FxBlend::Alpha);
            let texture_color = if stage.image.eq_ignore_ascii_case("$whiteimage")
                || stage.image.eq_ignore_ascii_case("$lightmap")
            {
                [1.0; 3]
            } else {
                let Some(texture_index) = textures.load(assets, &stage.image, stage.clamp) else {
                    continue;
                };
                let Some(color) =
                    texture_emissive_color(&textures.images[texture_index], alpha_matters)
                else {
                    continue;
                };
                color
            };
            let Some(analysis) = material_analysis(
                blend,
                stage.glow,
                texture_color,
                stage.rgb_gen.uses_vertex_color(),
                if stage.rgb_gen == RgbGen::Const {
                    stage.color.unwrap_or([1.0; 3])
                } else {
                    [1.0; 3]
                },
                matches!(stage.alpha_gen, AlphaGen::Vertex | AlphaGen::OneMinusVertex),
                stage.alpha.unwrap_or(1.0),
            ) else {
                continue;
            };
            if best.map_or(true, |current| analysis.score > current.score) {
                best = Some(analysis);
            }
        }
        return best;
    }

    // Faithful RE_RegisterShader implicit-image fallback: image-only FX use a
    // regular alpha blend when there is no .shader definition.
    let texture_index = textures.load(assets, &key, false)?;
    let texture_color = texture_emissive_color(&textures.images[texture_index], true)?;
    material_analysis(
        FxBlend::Alpha,
        false,
        texture_color,
        true,
        [1.0; 3],
        true,
        1.0,
    )
}

fn material_analysis(
    blend: FxBlend,
    authored_glow: bool,
    texture_color: [f32; 3],
    rgb_vertex: bool,
    rgb_const: [f32; 3],
    alpha_vertex: bool,
    alpha_const: f32,
) -> Option<MaterialLightAnalysis> {
    let mut score = match blend {
        FxBlend::Add => 6,
        FxBlend::AddAlpha => 5,
        FxBlend::Alpha => 0,
        FxBlend::Opaque => -4,
        FxBlend::DstColorAdd | FxBlend::Modulate | FxBlend::Modulate2x | FxBlend::Darken => -8,
    };
    if authored_glow {
        score += 6;
    }
    (score >= 0).then_some(MaterialLightAnalysis {
        score,
        texture_color,
        rgb_vertex,
        rgb_const,
        alpha_vertex,
        alpha_const,
        blend,
    })
}

/// Per-session runtime side.  Normally this is pure catalog lookup.  A dynamic
/// shader remap invalidates only the material side and switches to the existing
/// EntityPresenter resolver as a rare cold-path fallback until the next map
/// load installs a fresh catalog.
pub struct FxLightPromoter {
    catalog: Arc<FxLightPromotionCatalog>,
    runtime_materials: HashMap<String, Option<MaterialLightAnalysis>>,
    resolve_runtime_materials: bool,
}

impl Default for FxLightPromoter {
    fn default() -> Self {
        Self {
            catalog: Arc::new(FxLightPromotionCatalog::default()),
            runtime_materials: HashMap::new(),
            resolve_runtime_materials: false,
        }
    }
}

impl FxLightPromoter {
    pub fn set_catalog(&mut self, catalog: Arc<FxLightPromotionCatalog>) {
        self.catalog = catalog;
        self.runtime_materials.clear();
        self.resolve_runtime_materials = false;
    }

    pub fn invalidate_materials(&mut self) {
        self.runtime_materials.clear();
        self.resolve_runtime_materials = true;
    }

    pub fn promote(
        &mut self,
        candidates: &[FxAutoLightCandidate],
        materials: &mut dyn FnMut(&str) -> Vec<FxMaterial>,
    ) -> Vec<FxLight> {
        struct Pending {
            effect: u32,
            light: FxLight,
        }

        let mut pending: Vec<Pending> = Vec::new();
        for candidate in candidates {
            let key = scene::canonical_shader_name(&candidate.shader);
            let analysis = if self.resolve_runtime_materials {
                if let Some(cached) = self.runtime_materials.get(&key) {
                    *cached
                } else {
                    let resolved = materials(&candidate.shader);
                    let analyzed = analyze_materials(&resolved);
                    self.runtime_materials.insert(key, analyzed);
                    analyzed
                }
            } else {
                self.catalog.material_analysis(&candidate.shader)
            };
            let Some(analysis) = analysis else { continue };
            if candidate.score + analysis.score < FINAL_PROMOTION_SCORE {
                continue;
            }

            let mut color = analysis.texture_color;
            let vertex_rgb = [
                f32::from(candidate.rgba[0]) / 255.0,
                f32::from(candidate.rgba[1]) / 255.0,
                f32::from(candidate.rgba[2]) / 255.0,
            ];
            let tint = if analysis.rgb_vertex {
                vertex_rgb
            } else {
                analysis.rgb_const
            };
            for channel in 0..3 {
                color[channel] *= tint[channel].max(0.0);
            }
            let alpha = if analysis.alpha_vertex {
                f32::from(candidate.rgba[3]) / 255.0
            } else {
                analysis.alpha_const
            };
            let alpha_weight = match analysis.blend {
                FxBlend::AddAlpha | FxBlend::Alpha => alpha.clamp(0.0, 1.0),
                _ => 1.0,
            };
            for channel in &mut color {
                *channel *= alpha_weight;
            }
            let peak = color[0].max(color[1]).max(color[2]);
            if !peak.is_finite() || peak < 0.03 {
                continue;
            }

            // Keep chromaticity from the source texture/vertex tint but avoid
            // treating a dim texture average as a second arbitrary intensity
            // control. Alpha/fades still modulate the light.
            let chroma_peak = peak.max(0.20);
            color = color.map(|value| (value / chroma_peak).clamp(0.0, 1.0));

            let mut origin = candidate.origin;
            if let Some([start, end]) = candidate.segment {
                origin = std::array::from_fn(|i| 0.5 * (start[i] + end[i]));
            }
            let radius = inferred_radius(candidate);
            let source_radius = inferred_source_radius(candidate, radius);
            let light = FxLight {
                kind: FxLightKind::PromotedEffect,
                origin,
                radius,
                source_radius,
                rgb: color,
                spot: candidate.spot,
                segment: candidate.segment,
                blade_segments: None,
            };

            // Continuous JKA effects may be invoked repeatedly while an older
            // driver primitive is still alive. Merge only same-effect lights
            // that are spatially close, preserving distinct explosions/bolts.
            let merge_distance = (radius * 0.35).clamp(24.0, 96.0);
            if let Some(existing) = pending.iter_mut().find(|existing| {
                let directional_match = match (existing.light.spot, light.spot) {
                    (Some(a), Some(b)) => {
                        a.direction[0] * b.direction[0]
                            + a.direction[1] * b.direction[1]
                            + a.direction[2] * b.direction[2]
                            >= 0.94
                    }
                    (None, None) => true,
                    _ => false,
                };
                existing.effect == candidate.effect
                    && directional_match
                    && distance_squared(existing.light.origin, light.origin)
                        <= merge_distance * merge_distance
            }) {
                existing.light.radius = existing.light.radius.max(light.radius);
                existing.light.source_radius = existing.light.source_radius.max(light.source_radius);
                for i in 0..3 {
                    existing.light.rgb[i] = existing.light.rgb[i].max(light.rgb[i]);
                }
                if existing.light.segment.is_none() {
                    existing.light.segment = light.segment;
                }
                match (&mut existing.light.spot, light.spot) {
                    (Some(current), Some(incoming)) => {
                        let summed = [
                            current.direction[0] + incoming.direction[0],
                            current.direction[1] + incoming.direction[1],
                            current.direction[2] + incoming.direction[2],
                        ];
                        let length_sq = summed.iter().map(|v| v * v).sum::<f32>();
                        if length_sq > 1.0e-6 {
                            let inv = length_sq.sqrt().recip();
                            current.direction = summed.map(|v| v * inv);
                        }
                        // Smaller cosine means a wider cone. Keep the wider
                        // authored/inferred envelope when repeated driver
                        // primitives are merged into one effect light.
                        current.inner_cone_cos = current.inner_cone_cos.min(incoming.inner_cone_cos);
                        current.outer_cone_cos = current.outer_cone_cos.min(incoming.outer_cone_cos);
                    }
                    (slot @ None, Some(incoming)) => *slot = Some(incoming),
                    _ => {}
                }
            } else if pending.len() < MAX_PROMOTED_LIGHTS {
                pending.push(Pending {
                    effect: candidate.effect,
                    light,
                });
            }
        }
        pending
            .into_iter()
            .map(|pending| pending.light)
            .collect()
    }
}

fn analyze_materials(materials: &[FxMaterial]) -> Option<MaterialLightAnalysis> {
    materials
        .iter()
        .filter_map(|material| {
            let alpha_matters = matches!(material.blend, FxBlend::AddAlpha | FxBlend::Alpha);
            let texture_color = match material.texture.as_deref() {
                Some(texture) => texture_emissive_color(texture, alpha_matters)?,
                None => [1.0; 3],
            };
            material_analysis(
                material.blend,
                material.authored_glow,
                texture_color,
                material.rgb_vertex,
                material.rgb_const,
                material.alpha_vertex,
                material.alpha_const,
            )
        })
        .max_by_key(|analysis| analysis.score)
}

fn texture_emissive_color(
    texture: &crate::materials::TextureData,
    alpha_matters: bool,
) -> Option<[f32; 3]> {
    if texture.label.eq_ignore_ascii_case("missing texture fallback") || texture.rgba.len() < 4 {
        return None;
    }
    // TextureData may contain appended mip levels. Sample only mip 0 so a
    // long mip chain cannot bias the inferred colour toward tiny levels.
    let base_bytes = (texture.width as usize)
        .saturating_mul(texture.height as usize)
        .saturating_mul(4)
        .min(texture.rgba.len());
    let pixels = base_bytes / 4;
    if pixels == 0 {
        return None;
    }
    let stride = (pixels / MAX_TEXTURE_SAMPLES).max(1);
    let mut sum = [0.0_f64; 3];
    let mut total = 0.0_f64;
    for pixel in (0..pixels).step_by(stride).take(MAX_TEXTURE_SAMPLES) {
        let base = pixel * 4;
        // Plain additive blending ignores source alpha. Respect alpha only
        // for blend modes where it actually modulates rendered energy.
        let alpha = if alpha_matters {
            f64::from(texture.rgba[base + 3]) / 255.0
        } else {
            1.0
        };
        if alpha <= 0.02 {
            continue;
        }
        let mut rgb = [0.0_f64; 3];
        for channel in 0..3 {
            let encoded = f64::from(texture.rgba[base + channel]) / 255.0;
            rgb[channel] = if texture.srgb {
                srgb_to_linear(encoded)
            } else {
                encoded
            };
        }
        let luminance = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        // Squared luminance makes bright cores dominate black/transparent halo
        // backgrounds without hard-coded knowledge of individual textures.
        let weight = alpha * luminance * luminance;
        if weight <= 1.0e-6 {
            continue;
        }
        for channel in 0..3 {
            sum[channel] += rgb[channel] * weight;
        }
        total += weight;
    }
    if total <= 1.0e-6 {
        return None;
    }
    let mut color = sum.map(|value| (value / total) as f32);
    let peak = color[0].max(color[1]).max(color[2]);
    if peak <= 1.0e-5 || !peak.is_finite() {
        return None;
    }
    color = color.map(|value| (value / peak).clamp(0.0, 1.0));
    Some(color)
}

fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn inferred_radius(candidate: &FxAutoLightCandidate) -> f32 {
    let visual = candidate.radius.abs().max(1.0);
    let mut radius = match candidate.kind {
        PrimType::Electricity => 96.0 + visual * 14.0,
        PrimType::Line | PrimType::Tail => 72.0 + visual * 12.0,
        PrimType::Cylinder => 72.0 + visual * 10.0,
        _ => 56.0 + visual * 8.0,
    };
    if let Some([start, end]) = candidate.segment {
        let length = distance_squared(start, end).sqrt();
        radius = radius.max(48.0 + length.min(512.0) * 0.25);
    }
    radius.clamp(48.0, 384.0)
}

/// Estimate the luminous visual's physical size independently from its much
/// larger attenuation radius. RT shadows use this for penumbrae; raster modes
/// deliberately ignore it.
fn inferred_source_radius(candidate: &FxAutoLightCandidate, illumination_radius: f32) -> f32 {
    let visual = candidate.radius.abs().max(0.0);
    let source = match candidate.kind {
        // These already become finite line emitters in RT; this adds thickness,
        // effectively turning the line into a small capsule emitter.
        PrimType::Electricity | PrimType::Line | PrimType::Tail => visual * 0.45,
        // A tapered Cylinder promoted as a spot is a lens/cone: its visible
        // far ring is not the physical source aperture.
        PrimType::Cylinder if candidate.spot.is_some() => visual * 0.15,
        PrimType::Cylinder => visual * 0.50,
        // Fire/flash/explosion billboard drivers are reasonably approximated by
        // a fuzzy spherical source around roughly half their visible radius.
        PrimType::Particle | PrimType::OrientedParticle => visual * 0.60,
        _ => visual * 0.50,
    };
    source
        .clamp(1.5, 64.0)
        .min((illumination_radius * 0.25).max(1.5))
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]) * (a[0] - b[0])
        + (a[1] - b[1]) * (a[1] - b[1])
        + (a[2] - b[2]) * (a[2] - b[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::TextureData;

    #[test]
    fn bright_texture_color_ignores_black_background() {
        let texture = TextureData {
            label: "test".into(),
            source: None,
            width: 2,
            height: 1,
            rgba: vec![0, 0, 0, 255, 255, 64, 0, 255],
            rgba16f: None,
            mip_level_count: 1,
            clamp: false,
            srgb: true,
        };
        let color = texture_emissive_color(&texture, true).unwrap();
        assert!(color[0] > 0.95);
        assert!(color[1] < 0.2);
        assert!(color[2] < 0.05);
    }

    #[test]
    fn effect_catalog_key_accepts_prefixed_and_unprefixed_names() {
        let mut catalog = FxLightPromotionCatalog::default();
        let profile = AutoLightProfile {
            driver_primitive: 2,
            score: 9,
            spot: None,
        };
        catalog.effects.insert("foo/bar".into(), profile);
        catalog.effects.insert("effects/foo/bar".into(), profile);
        assert_eq!(catalog.effect_profile("foo/bar.efx"), Some(profile));
        assert_eq!(catalog.effect_profile("effects/foo/bar"), Some(profile));
    }
}

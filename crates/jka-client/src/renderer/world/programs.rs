//! Stage program policies. Classification happens during compilation, never per pixel.
use crate::renderer::{
    MaterialFragmentKey, MaterialUniform, PipelineKey, WorldShaderFamily, WorldShaderVariantKey,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::renderer) enum WorldProgramMode {
    #[default]
    Specialized,
    Shared,
    Generated,
}

impl WorldProgramMode {
    pub fn from_console(mode: u8) -> Self {
        match mode {
            3 => Self::Shared,
            4 => Self::Generated,
            _ => Self::Specialized,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::renderer) enum StageProgram {
    #[default]
    Specialized,
    Minimal,
    SharedBasic,
    SharedEnhanced,
    GeneratedBasic,
    GeneratedEnhanced,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::renderer) struct VertexRecipe {
    pub tc_mods: [u8; 4],
    pub stretch_funcs: [u8; 4],
    pub wave_funcs: [u8; 2],
    pub surface_material: u8,
}

impl VertexRecipe {
    pub fn from_uniform(material: &MaterialUniform) -> Self {
        let mut recipe = Self {
            surface_material: ((material.header[3] >> 8) & 31) as u8,
            ..Self::default()
        };
        for i in 0..(material.header[1] as usize).min(4) {
            recipe.tc_mods[i] = (material.mods[i * 2][0] + 0.5) as u8;
            if recipe.tc_mods[i] == 6 {
                recipe.stretch_funcs[i] = (material.mods[i * 2 + 1][1] + 0.5) as u8;
            }
        }
        if material.header[2] & 134217728 != 0 {
            recipe.wave_funcs[0] = material.wave_funcs[0] as u8;
        }
        if material.header[2] & 268435456 != 0 {
            recipe.wave_funcs[1] = material.wave_funcs[1] as u8;
        }
        recipe
    }
}

/// The scoped experimental feature envelope. Other configurations retain mode 1.
pub(in crate::renderer) fn supported_variant(v: WorldShaderVariantKey) -> bool {
    v.family() == WorldShaderFamily::Lean
        && !v.source_map_world
        && !v.map_light_simulation
        && !v.vertex_dlights
        && !v.clustered_lite_dlights
        && !v.local_shadows
        && !v.cascaded_shadows
        && !v.ray_traced_shadows
        && !v.ray_traced_sun
        && !v.weather_surface
        && !v.classic_render_flags
        && !v.planar_debug
        && !v.dynamic_light_debug
        && !v.sky_debug
}

/// Exact material-side eliminations in the main base pass. Reflection/fog/dlight
/// redraws retain their original pipelines, including clipping and deformation.
pub(in crate::renderer) fn effective_stage_variant(
    mut v: WorldShaderVariantKey,
    m: MaterialFragmentKey,
) -> WorldShaderVariantKey {
    v = v.base_pass_key();
    if m.flags & (8 | 16 | 32 | 64 | 128 | 256 | 512 | 1024 | 2048 | 262144) == 0 {
        v.pbr = false;
        v.pom = false;
        v.deluxe = false;
        v.deluxe_specular = false;
        v.pbr_shared_material_eval = false;
        v.pbr_shared_tangent_frame = false;
        v.pom_mip_aware = false;
        v.pom_adaptive_steps = false;
        v.pbr_companion_sampler = false;
        v.pbr_vertex_lightgrid = false;
    }
    if m.flags_w & 2 == 0 {
        v.detail_textures_enabled = false;
    }
    if m.flags & 4096 == 0 && m.tc_gen != 3 {
        v.planar_reflections = false;
    }
    if m.vertex_recipe.surface_material != 14 {
        v.surface_deformation = false;
    }
    v
}

fn basic(v: WorldShaderVariantKey) -> bool {
    !v.pbr
        && !v.pom
        && !v.deluxe
        && !v.detail_textures_enabled
        && !v.planar_reflections
        && !v.surface_deformation
}

pub(in crate::renderer) fn program_key(
    v: WorldShaderVariantKey,
    p: PipelineKey,
    m: MaterialFragmentKey,
) -> MaterialFragmentKey {
    let alpha = p.class != crate::renderer::DrawClass::Opaque;
    // Glass shading exists only in fs_main; the minimal, shared and generated
    // entries would draw it as an opaque stage.
    let glass = m.flags_w & super::variants::MATERIAL_FLAGS_W_GLASS != 0;
    let ordinary = !p.auto_sprite2 && !p.legacy_srgb_three_stage && m.tc_gen <= 3 && !glass;
    if !supported_variant(v) || !ordinary || v.program_mode == WorldProgramMode::Specialized {
        let mut key = m.for_variant(v, alpha);
        if !glass && super::pipelines::material_minimal_entry(v, &p) {
            key.program = StageProgram::Minimal;
        }
        return key;
    }
    let effective = effective_stage_variant(v, m);
    match v.program_mode {
        WorldProgramMode::Shared => MaterialFragmentKey {
            // Zero masks leave every material option uniform-driven. Families
            // differ only by the operations their global variant compiles in.
            tc_gen: u32::MAX,
            program: if basic(effective) {
                StageProgram::SharedBasic
            } else {
                StageProgram::SharedEnhanced
            },
            ..MaterialFragmentKey::default()
        },
        WorldProgramMode::Generated => {
            let mut key = m.for_variant(effective, alpha);
            key.tc_gen = m.tc_gen;
            key.vertex_recipe = m.vertex_recipe;
            if !effective.surface_deformation {
                key.vertex_recipe.surface_material = 0;
            }
            key.program = if basic(effective) {
                StageProgram::GeneratedBasic
            } else {
                StageProgram::GeneratedEnhanced
            };
            key
        }
        WorldProgramMode::Specialized => unreachable!(),
    }
}

/// Whether a narrowed key's program uses vs/fs_world_minimal (decided by `program_key`).
pub(in crate::renderer) fn uses_minimal_entry(m: MaterialFragmentKey) -> bool {
    matches!(m.program, StageProgram::Minimal | StageProgram::SharedBasic)
}

pub(in crate::renderer) fn active_mode(v: WorldShaderVariantKey) -> u8 {
    if !v.material_specialization {
        0
    } else if v.minimal_entries {
        2
    } else {
        match v.program_mode {
            WorldProgramMode::Shared => 3,
            WorldProgramMode::Generated => 4,
            _ => 1,
        }
    }
}

pub(in crate::renderer) fn fallback_reason(
    v: WorldShaderVariantKey,
    p: PipelineKey,
    m: MaterialFragmentKey,
) -> &'static str {
    if !supported_variant(v) {
        "features outside Minimal/Low/Medium envelope"
    } else if p.auto_sprite2 {
        "autosprite2 vertex deformation"
    } else if p.legacy_srgb_three_stage {
        "authored three-stage RGB composite"
    } else if m.tc_gen > 3 {
        "special texture-coordinate generator"
    } else if m.flags_w & super::variants::MATERIAL_FLAGS_W_GLASS != 0 {
        "glass primary stage"
    } else {
        "existing special effect entry"
    }
}

pub(in crate::renderer) fn program_variant(
    v: WorldShaderVariantKey,
    m: MaterialFragmentKey,
) -> WorldShaderVariantKey {
    match m.program {
        StageProgram::SharedBasic => {
            let empty = MaterialFragmentKey {
                tc_gen: 0,
                ..MaterialFragmentKey::default()
            };
            effective_stage_variant(v, empty)
        }
        StageProgram::GeneratedBasic | StageProgram::GeneratedEnhanced => {
            effective_stage_variant(v, m)
        }
        _ => v.base_pass_key(),
    }
}

/// Balanced-brace extraction keeps generated entries on the checked-in shading
/// equations. Missing anchors fail loudly instead of silently changing output.
fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source
        .find(&format!("fn {name}("))
        .expect("world shader function anchor");
    let open = start
        + source[start..]
            .find('{')
            .expect("world shader function body");
    let mut depth = 1;
    for (offset, byte) in source.as_bytes()[open + 1..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return &source[open + 1..open + 1 + offset];
        }
    }
    panic!("unterminated world shader function {name}");
}

fn replace_function_body(source: &mut String, name: &str, replacement: &str) {
    let body = function_body(source, name);
    let start = body.as_ptr() as usize - source.as_ptr() as usize;
    let end = start + body.len();
    source.replace_range(start..end, replacement);
}

/// Reuse each authored tcMod equation but emit the actual sequence without a
/// uniform loop or opcode interpreter. Parameters stay in the material buffer.
fn specialized_tc_mods(source: &str, recipe: VertexRecipe) -> String {
    use std::fmt::Write;
    let original = function_body(source, "apply_tc_mods");
    let begin = original
        .find("        let a = material.mods")
        .expect("tcMod operation anchor");
    let end = original
        .rfind("    }\n    return uv;")
        .expect("tcMod loop end");
    let operation = &original[begin..end];
    let mut body = String::from("\n    var uv = base_uv;\n    let time = shader_stage_time();\n");
    for (i, kind) in recipe.tc_mods.into_iter().enumerate() {
        if kind == 0 {
            continue;
        }
        let anchor = format!("if (kind == {kind}u) {{");
        let start = operation.find(&anchor).expect("authored tcMod opcode");
        let open = start + anchor.len();
        let mut depth = 1;
        let mut close = open;
        for (offset, byte) in operation.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                close = open + offset;
                break;
            }
        }
        let fixed = operation[open..close]
            .replace("u32(b.y + 0.5)", &format!("{}u", recipe.stretch_funcs[i]));
        let _ = writeln!(
            body,
            "    {{\nlet a = material.mods[{}u];\nlet b = material.mods[{}u];\n{fixed}\n    }}",
            i * 2,
            i * 2 + 1
        );
    }
    body.push_str("    return uv;\n");
    body
}

/// Source generation is bounded by encountered normalized recipes. The module
/// retains reusable helper definitions; only the generated entries are compiled.
pub(in crate::renderer) fn generated_source(
    v: WorldShaderVariantKey,
    m: MaterialFragmentKey,
) -> String {
    use std::fmt::Write;
    let mut source = crate::renderer::compose_world_shader(include_str!("../../bsp_lean.wgsl"))
        .replace("\r\n", "\n");
    let mods = specialized_tc_mods(&source, m.vertex_recipe);
    replace_function_body(&mut source, "apply_tc_mods", &mods);
    let uv = match m.tc_gen {
        0 => "input.uv".to_owned(),
        1 => "input.lightmap_uv".to_owned(),
        2 => "vec2<f32>(dot(input.position, material.vector_s.xyz), dot(input.position, material.vector_t.xyz))".to_owned(),
        3 => "vec2<f32>(0.5 - reflected.z * 0.5, 0.5 - reflected.y * 0.5)".to_owned(),
        _ => panic!("unsupported generated tcGen"),
    };
    let environment = if m.tc_gen == 3 {
        "let n = normalize(input.normal);\nlet view = normalize(camera.camera_pos_time.xyz - input.position);\nlet reflected = reflect(-view, n);"
    } else {
        ""
    };
    replace_function_body(
        &mut source,
        "generated_uv",
        &format!("\n{environment}\nreturn apply_tc_mods({uv}, input.position);\n"),
    );
    source = source
        .replace(
            "material.wave_funcs.x",
            &format!("{}u", m.vertex_recipe.wave_funcs[0]),
        )
        .replace(
            "material.wave_funcs.y",
            &format!("{}u", m.vertex_recipe.wave_funcs[1]),
        );

    let enhanced = m.program == StageProgram::GeneratedEnhanced;
    let has_wave = m.flags & (134217728 | 268435456) != 0;
    let needs_color = (m.flags & (1 | 524288) != 0 && m.flags & 134217728 == 0)
        || (m.flags & (1048576 | 2097152) != 0 && m.flags & 268435456 == 0);
    let needs_lm = enhanced || m.flags & 2 != 0;
    let needs_position = enhanced || v.legacy_fog || v.jump_shade;
    let mut fields = vec![("uv", "vec2<f32>")];
    if needs_lm {
        fields.push(("lightmap_uv", "vec2<f32>"));
    }
    if needs_color {
        fields.push(("color", "vec4<f32>"));
    }
    if needs_position {
        fields.push(("world_position", "vec3<f32>"));
    }
    if v.static_bsp_ao {
        fields.push(("sky_dir_ao", "vec4<f32>"));
    }
    if enhanced {
        fields.push(("world_normal", "vec3<f32>"));
    }
    if v.surface_deformation {
        fields.extend([("shell_kind", "u32"), ("shell_coverage", "f32")]);
    }
    if v.pbr && v.deluxe && v.pbr_vertex_lightgrid {
        fields.extend([
            ("pbr_lightgrid_direction", "vec4<f32>"),
            ("pbr_lightgrid_lighting", "vec4<f32>"),
        ]);
    }
    if has_wave {
        fields.push(("wave_color", "vec4<f32>"));
    }
    if has_wave {
        // Additive source modulation uses the same rgbGen waveform already
        // carried from the vertex shader. Do not re-run wave_value in the FS:
        // generated stage programs intentionally evaluate it only in the VS.
        let additive_wave_eval = format!(
            "wave = clamp(wave_value({}u, material.wave_rgb, shader_stage_time()), 0.0, 1.0);",
            m.vertex_recipe.wave_funcs[0],
        );
        assert!(source.contains(&additive_wave_eval), "additive waveform must remain vertex-only");
        source = source.replace(&additive_wave_eval, "wave = generated_wave_color.x;");
        // Keep the exact wave evaluator, including remap/clamp time, in the VS.
        // A flat varying carries its per-stage result to the existing FS helper.
        source = source.replacen("fn apply_wave_gens(", "fn vertex_wave_gens(", 1);
        source.push_str("\nvar<private> generated_wave_color: vec4<f32>;\nfn apply_wave_gens(color: vec4<f32>) -> vec4<f32> {\nvar out = color;\nif ((material_flags() & 134217728u) != 0u) { out = vec4<f32>(generated_wave_color.rgb, out.a); }\nif ((material_flags() & 268435456u) != 0u) { out.a = generated_wave_color.a; }\nreturn out;\n}\n");
    }
    let mut generated = String::from(
        "\nstruct GeneratedVertexOut {\n@builtin(position) @invariant clip_position: vec4<f32>,\n",
    );
    for (location, (name, ty)) in fields.iter().enumerate() {
        let flat = if *ty == "u32" || *name == "wave_color" {
            "@interpolate(flat) "
        } else {
            ""
        };
        let _ = writeln!(generated, "@location({location}) {flat}{name}: {ty},");
    }
    generated.push_str("};\n@vertex fn vs_generated(input: VertexIn, @builtin(instance_index) instance_index: u32) -> GeneratedVertexOut {\nvar output: GeneratedVertexOut;\n");
    if enhanced {
        generated.push_str("let surface = world_vertex(input, instance_index, 0xffffffffu);\noutput.clip_position = surface.clip_position;\n");
        for (name, _) in &fields {
            if *name != "wave_color" {
                let _ = writeln!(generated, "output.{name} = surface.{name};");
            }
        }
    } else {
        generated.push_str("output.clip_position = camera.view_proj * vec4<f32>(input.position, 1.0);\noutput.uv = generated_uv(input);\n");
        if needs_lm {
            generated.push_str("output.lightmap_uv = input.lightmap_uv;\n");
        }
        if needs_color {
            generated.push_str("output.color = input.color;\n");
        }
        if needs_position {
            generated.push_str("output.world_position = input.position;\n");
        }
        if v.static_bsp_ao {
            generated.push_str("output.sky_dir_ao = vec4<f32>(vec3<f32>(0.0), input.static_ao);\n");
        }
    }
    if has_wave {
        generated.push_str("output.wave_color = vertex_wave_gens(vec4<f32>(1.0));\n");
    }
    generated.push_str("return output;\n}\n@fragment fn fs_generated(input: GeneratedVertexOut) -> @location(0) vec4<f32> {\n");
    if has_wave {
        generated.push_str("generated_wave_color = input.wave_color;\n");
    }
    if enhanced {
        generated
            .push_str("var surface: VertexOut;\nsurface.clip_position = input.clip_position;\n");
        for (name, _) in &fields {
            if *name != "wave_color" {
                let _ = writeln!(generated, "surface.{name} = input.{name};");
            }
        }
        let entry = if v.legacy_fog {
            "fs_main_legacy_fog"
        } else {
            "fs_main"
        };
        generated.push_str(
            &function_body(&source, entry)
                .replace("input.", "surface.")
                .replace("shade_surface(input)", "shade_surface(surface)"),
        );
    } else {
        let mut body = function_body(&source, "fs_world_minimal").to_owned();
        body = body
            .replace(
                "input.world_ao.xyz",
                if needs_position {
                    "input.world_position"
                } else {
                    "vec3<f32>(0.0)"
                },
            )
            .replace(
                "input.world_ao.w",
                if v.static_bsp_ao {
                    "input.sky_dir_ao.w"
                } else {
                    "1.0"
                },
            );
        if !needs_color {
            body = body.replace("input.color", "vec4<f32>(1.0)");
        }
        if !needs_lm {
            body = body.replace("input.lightmap_uv", "vec2<f32>(-1.0)");
        }
        generated.push_str(&body);
    }
    generated.push_str("\n}\n");
    source.push_str(&generated);
    source
}

#[derive(Default)]
pub(in crate::renderer) struct ProgramModules {
    modules: std::sync::Mutex<
        std::collections::HashMap<(WorldShaderVariantKey, MaterialFragmentKey), wgpu::ShaderModule>,
    >,
}

impl ProgramModules {
    pub fn shader(
        &self,
        device: &wgpu::Device,
        original: &wgpu::ShaderModule,
        v: WorldShaderVariantKey,
        m: MaterialFragmentKey,
    ) -> wgpu::ShaderModule {
        if !matches!(
            m.program,
            StageProgram::GeneratedBasic | StageProgram::GeneratedEnhanced
        ) {
            return original.clone();
        }
        let mut modules = self.modules.lock().unwrap();
        modules
            .entry((v, m))
            .or_insert_with(|| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("JKA generated world stage recipe"),
                    source: wgpu::ShaderSource::Wgsl(generated_source(v, m).into()),
                })
            })
            .clone()
    }

    pub fn pipeline(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        original: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        samples: u32,
        variant: WorldShaderVariantKey,
        key: PipelineKey,
        material: MaterialFragmentKey,
    ) -> wgpu::RenderPipeline {
        let effective = program_variant(variant, material);
        let shader = self.shader(device, original, effective, material);
        let minimal = uses_minimal_entry(material);
        crate::renderer::create_world_pipeline_specialized(
            device,
            layout,
            &shader,
            format,
            key,
            samples,
            effective.legacy_fog,
            false,
            Some(effective),
            false,
            None,
            Some(material),
            minimal,
        )
    }
}

#[cfg(test)]
#[path = "program_tests.rs"]
mod tests;

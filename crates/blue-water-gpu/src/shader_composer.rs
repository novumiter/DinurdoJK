//! Compose the preserved upstream ocean surface WGSL using its original modules.
//! This is a shader validation bridge; JKA bind groups and render passes are
//! integrated separately. The JKA variant adds only a fragment admission test;
//! the preserved source and its shading calculations remain unchanged.
use naga_oil::compose::{
    ComposableModuleDescriptor, Composer, ComposerError, NagaModuleDescriptor, ShaderDefValue,
    ShaderLanguage, ShaderType,
};

const MODULES: &[(&str, &str)] = &[
    (
        "naval-render/shaders/camera.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/shared/render/shaders/camera.wgsl"),
    ),
    (
        "naval-render/shaders/shadow.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/shared/render/shaders/shadow.wgsl"),
    ),
    (
        "naval-render/shaders/motion.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/shared/render/shaders/motion.wgsl"),
    ),
    (
        "naval-render/shaders/lights.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/shared/render/shaders/lights.wgsl"),
    ),
    (
        "naval-render/shaders/aerial_volume.wgsl",
        include_str!(
            "../../../vendor/blue-water-sea-and-sky/shared/render/shaders/aerial_volume.wgsl"
        ),
    ),
    (
        "naval-render/shaders/aerial.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/shared/render/shaders/aerial.wgsl"),
    ),
    (
        "naval-clouds/shaders/shadow.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/clouds/shaders/shadow.wgsl"),
    ),
    (
        "naval-ocean/shaders/water_properties.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/water_properties.wgsl"),
    ),
    (
        "naval-ocean/shaders/noise.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/noise.wgsl"),
    ),
    (
        "naval-ocean/shaders/sky_environment.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/sky_environment.wgsl"),
    ),
    (
        "naval-ocean/shaders/reflection.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/reflection.wgsl"),
    ),
    (
        "naval-ocean/shaders/water_body.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/water_body.wgsl"),
    ),
    (
        "naval-ocean/shaders/foam.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/foam.wgsl"),
    ),
    (
        "naval-ocean/shaders/point_lights.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/point_lights.wgsl"),
    ),
    (
        "naval-ocean/shaders/mx_noise.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/mx_noise.wgsl"),
    ),
    (
        "naval-ocean/shaders/wake.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/wake.wgsl"),
    ),
    (
        "naval-ocean/shaders/waves.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/waves.wgsl"),
    ),
    (
        "naval-ocean/shaders/screen_reflection.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/screen_reflection.wgsl"),
    ),
    (
        "naval-ocean/shaders/underwater_ray.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/underwater_ray.wgsl"),
    ),
    (
        "naval-ocean/shaders/rain.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/rain.wgsl"),
    ),
    (
        "naval-ocean/shaders/spray_haze.wgsl",
        include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/spray_haze.wgsl"),
    ),
];

pub fn composer() -> Result<Composer, Box<ComposerError>> {
    let mut composer = Composer::default();
    for &(file_path, source) in MODULES {
        composer
            .add_composable_module(ComposableModuleDescriptor {
                source,
                file_path,
                language: ShaderLanguage::Wgsl,
                ..Default::default()
            })
            .map_err(Box::new)?;
    }
    Ok(composer)
}

pub fn surface_module(
    composer: &mut Composer,
    point_lights: bool,
) -> Result<wgpu::naga::Module, Box<ComposerError>> {
    compose_surface(composer, point_lights, false, 16)
}

pub fn jka_surface_module(
    composer: &mut Composer,
    reflection_steps: u32,
) -> Result<wgpu::naga::Module, Box<ComposerError>> {
    compose_surface(composer, false, true, reflection_steps.clamp(4, 32))
}

/// Original underwater passes with only the JKA pool-admission/path cap added.
pub fn underwater_module(composer: &mut Composer, pass: &str) -> Result<wgpu::naga::Module, Box<ComposerError>> {
    let replace = |source: &str, from: &str, to: &str| {
        assert_eq!(source.matches(from).count(), 1, "upstream underwater contract changed: {from}");
        source.replace(from, to)
    };
    let (source, path) = match pass {
        "probe" => {
            let original = include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/underwater_probe.wgsl");
            let source = replace(original, "let depth = height_at(ray.start.xz) + wake_height(ray.start.xz) - ray.start.y;",
                "let depth = select(-1.0, height_at(ray.start.xz) + wake_height(ray.start.xz) - ray.start.y, jka_water_contains(ray.start.xz));")
                .replace("vec4<f32>(depth, height_at(eye.xz) - eye.y, 0.0, 1.0)",
                    "vec4<f32>(depth, select(-1.0, height_at(eye.xz) - eye.y, jka_water_contains(eye.xz)), 0.0, 1.0)");
            (format!("{source}\n{}", include_str!("surface_boundary.wgsl")), "original underwater probe with JKA boundary")
        }
        "composite" => {
            let original = include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/underwater.wgsl");
            // JKA's short geometry far plane is not the end of an open water
            // column. A missing receiver may reach optically opaque water;
            // the independent JKA path texture still limits finite pools.
            let source = replace(original, "return -near * far / (depth * (far - near) + near);",
                "return select(-near * far / (depth * (far - near) + near), -10000.0, depth <= 0.0);");
            let source = replace(&source, "let distance = max(farthest / ray.view.z - ray.to_near, 0.0) * clarity;",
                "let distance = min(max(farthest / ray.view.z - ray.to_near, 0.0), textureLoad(jka_water_distance, pixel, 0).r) * clarity;");
            let source = replace(&source, "let film = waterline * MENISCUS_TINT;",
                "let path = textureLoad(jka_water_distance, vec2<i32>(position.xy), 0).r;\n    let film = select(0.0, waterline * MENISCUS_TINT, path > 0.0);");
            (format!("{source}\n@group(1) @binding(4) var jka_water_distance: texture_2d<f32>;"), "original underwater composite with JKA bounded path")
        }
        "motes" => {
            let original = include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/underwater_motes.wgsl");
            let source = replace(original, "let shown = smoothstep(NEAR.x, NEAR.y, range)",
                "let pixel = clamp(vec2<i32>(uv * camera.viewport.xy), vec2<i32>(0), vec2<i32>(camera.viewport.xy) - 1);\n    let bounded = select(0.0, step(range, textureLoad(jka_water_distance, pixel, 0).r), jka_water_contains(world.xz));\n    let shown = bounded * smoothstep(NEAR.x, NEAR.y, range)");
            (format!("{source}\n@group(1) @binding(4) var jka_water_distance: texture_2d<f32>;\n{}", include_str!("surface_boundary.wgsl")), "original underwater motes with JKA bounded path")
        }
        _ => panic!("unknown underwater pass {pass}"),
    };
    composer.make_naga_module(NagaModuleDescriptor {
        source: &source, file_path: path, shader_type: ShaderType::Wgsl,
        shader_defs: [("CASCADES".to_owned(), ShaderDefValue::UInt(3))].into_iter().collect(),
        ..Default::default()
    }).map_err(Box::new)
}

fn compose_surface(
    composer: &mut Composer,
    point_lights: bool,
    jka_boundary: bool,
    reflection_steps: u32,
) -> Result<wgpu::naga::Module, Box<ComposerError>> {
    let mut shader_defs = vec![
        ("CASCADES".to_owned(), ShaderDefValue::UInt(3)),
        ("REFLECTION_STEPS".to_owned(), ShaderDefValue::UInt(reflection_steps)),
    ];
    if point_lights {
        shader_defs.push(("POINT_LIGHTS".to_owned(), ShaderDefValue::UInt(1)));
    }
    let original = include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/surface.wgsl");
    let source = if jka_boundary {
        let entry = "fn fragment(input: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {";
        assert_eq!(original.matches(entry).count(), 1, "upstream fragment contract changed");
        // Upstream's ordinary wave rings fit inside its 30 km far plane. JKA's
        // distanceCull can end within the first few rings, before the skirt.
        // Fit every admitted infinite-water vertex to the host depth range so
        // the rings join the horizon even as distanceCull changes at runtime.
        let far_guard = "if (horizon || fan) && camera.curvature > 0.0 {";
        assert_eq!(original.matches(far_guard).count(), 1, "upstream far-plane contract changed");
        let original = original.replace(far_guard, "if camera.curvature > 0.0 {");
        // JKA exposes the original transmission swatch as its water color.
        // Apply the same relative tint to the upwelling body without tinting
        // scene objects or reflected sky. The calibrated default has gain one.
        let deep = "let deep = upwelling(subsurface_reflectance(absorption, backscatter), sun_direction, sun_radiance, contact.irradiance, lit);";
        assert_eq!(original.matches(deep).count(), 1, "upstream water-body contract changed");
        let tint = crate::original_surface::SurfaceAppearance::default().transmission;
        let original = original.replace(deep, &format!("let deep = upwelling(subsurface_reflectance(absorption, backscatter), sun_direction, sun_radiance, contact.irradiance, lit) * (surface.transmission.rgb / vec3<f32>({}, {}, {}));", tint[0], tint[1], tint[2]));
        // A nearest-depth MSAA scene copy can select the foreground player at a
        // silhouette while the water fragment covers another sample. That is
        // open water, not a zero-depth shoreline with a bright foam outline.
        let column = "let column = select(max(scene_view_z / min(ray_view_z, -1e-4) - distance, 0.0), OPEN_DEEP, depth <= 0.0);";
        assert_eq!(original.matches(column).count(), 1, "upstream water-column contract changed");
        let original = original.replace(column, "let receiver_range = scene_view_z / min(ray_view_z, -1e-4);\n    let column = select(max(receiver_range - distance, 0.0), OPEN_DEEP, depth <= 0.0 || receiver_range + 0.03 < distance);");
        let body = "let body = water_body(textureLoad(scene_color, pixel, 0).rgb, deep, column_transmittance(attenuation, backscatter, column, view));";
        assert_eq!(original.matches(body).count(), 1, "upstream transmission contract changed");
        let original = original.replace(body, "let bent = jka_refracted_scene(input.world, -view, up, 1.0 / 1.333, column * 2.0, true);\n    let transmitted = select(textureLoad(scene_color, pixel, 0).rgb, bent.rgb, bent.a >= 0.0);\n    let through = select(column, bent.a, bent.a >= 0.0);\n    let body = water_body(transmitted, deep, column_transmittance(attenuation, backscatter, through, view));");
        let underside = "color = underside(view, up, pigment, select(absorption, water_beam(optics), surface.water.absorption.w > 0.5), sun_direction, sky_env.light_color);";
        assert_eq!(original.matches(underside).count(), 1, "upstream underside contract changed");
        let original = original.replace(underside, &format!("{underside}\n        let window_ray = refract(-view, -up, 1.333);\n        let window_scene = jka_refracted_scene(input.world, -view, -up, 1.333, 100.0, false);\n        if window_scene.a >= 0.0 {{\n            color += (window_scene.rgb - sky_reflection(window_ray, 0.0)) * WINDOW_GAIN * (1.0 - water_fresnel(max(dot(window_ray, up), 0.0)));\n        }}"));

        format!("{}\n{}", original.replace(entry, &format!(
            "{entry}\n    if !jka_water_contains(input.grid) {{ discard; }}")),
            format!("{}\n{}", include_str!("surface_boundary.wgsl"), include_str!("surface_refraction.wgsl")))
    } else {
        original.to_owned()
    };
    composer
        .make_naga_module(NagaModuleDescriptor {
            source: &source,
            file_path: "naval-ocean/shaders/surface.wgsl",
            shader_type: ShaderType::Wgsl,
            shader_defs: shader_defs.into_iter().collect(),
            ..Default::default()
        })
        .map_err(Box::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_surface_shader_composes_and_validates() {
        for point_lights in [false, true] {
            let mut composer = composer()
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&Composer::default())));
            let module = surface_module(&mut composer, point_lights)
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&composer)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .expect("preserved Blue Water surface validates");
        }
    }

    #[test]
    fn original_surface_pipeline_builds_on_gpu() {
        futures_lite::future::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .expect("GPU adapter");
            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_limits: adapter.limits(),
                    ..Default::default()
                })
                .await
                .expect("GPU device");
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            for point_lights in [false, true] {
                let mut composer = composer().expect("upstream modules");
                let module = surface_module(&mut composer, point_lights).expect("upstream surface");
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Blue Water original surface"),
                    source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
                });
                let compile_started = std::time::Instant::now();
                let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Blue Water original surface validation"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vertex"),
                        compilation_options: Default::default(),
                        buffers: &[Some(crate::geometry::OceanVertex::LAYOUT)],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fragment"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                });
                eprintln!(
                    "[BLUE WATER COMPILE] original surface point_lights={point_lights}: {:.3} ms",
                    compile_started.elapsed().as_secs_f64() * 1000.0
                );
            }
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            assert!(
                scope.pop().await.is_none(),
                "Blue Water original surface pipeline validation"
            );
        });
    }
}

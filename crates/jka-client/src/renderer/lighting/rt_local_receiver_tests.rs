//! Receiver regression: a sampled saber is a point, but its source is still finite.
use super::*;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a Vulkan GPU"]
fn local_temporal_and_svgf_average_stochastic_signal() {
    block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })
            .await
            .unwrap();
        let create = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let raw = create(wgpu::TextureFormat::Rgba32Float);
        let depth = create(wgpu::TextureFormat::R32Float);
        let motion = create(wgpu::TextureFormat::Rg16Float);
        queue.write_texture(
            depth.as_image_copy(),
            bytemuck::cast_slice(&[10.0f32; 64]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: Some(8),
            },
            depth.size(),
        );
        let raw_view = raw.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let motion_view = motion.create_view(&Default::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var signal: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    output[id.y * 8u + id.x] = textureLoad(signal, vec2<i32>(id.xy), 0);
}
"#
                .into(),
            ),
        });
        let transfer = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1024,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1024,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let eye = Vec3::new(0.0, 0.0, 20.0);
        let vp = Mat4::perspective_rh(90f32.to_radians(), 1.0, 0.1, 100.0)
            * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        for mode in [1, 2] {
            let mut filter = rt_radiance::RtSunRadiance::new(&device);
            for frame in 0..2 {
                let signal: Vec<[f32; 4]> = (0..64)
                    .map(|i| {
                        let value = ((i % 8 + i / 8 + frame) % 2) as f32;
                        [value, value, value, 1.0]
                    })
                    .collect();
                queue.write_texture(
                    raw.as_image_copy(),
                    bytemuck::cast_slice(&signal),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(128),
                        rows_per_image: Some(8),
                    },
                    raw.size(),
                );
                filter.prepare(
                    &device,
                    &queue,
                    [8, 8],
                    vp,
                    eye,
                    eye,
                    [1.0; 4],
                    mode,
                    frame as u32,
                    false,
                    &raw_view,
                    &depth_view,
                    &motion_view,
                    true,
                );
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &transfer.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&filter.published),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: output.as_entire_binding(),
                        },
                    ],
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                filter.encode(&mut encoder, mode);
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&transfer);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(8, 8, 1);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 1024);
                let submission = queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: Some(std::time::Duration::from_secs(30)),
                    })
                    .unwrap();
                rx.recv().unwrap().unwrap();
                let data = readback.slice(..).get_mapped_range();
                let actual = bytemuck::cast_slice::<u8, [f32; 4]>(&data)[4 * 8 + 4];
                if frame == 1 {
                    assert!(
                        (actual[0] - 0.5).abs() < 0.01,
                        "mode {mode} should average old 0 and new 1; got {actual:?}"
                    );
                }
                drop(data);
                readback.unmap();
            }
        }
    });
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn local_denoiser_receiver_uses_filtered_saber_and_rgb_signal() {
    block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .unwrap();
        let receiver = include_str!("../../rt_resolution_read.wgsl")
            .replace("\r\n", "\n")
            .replace(
                "@group(3) @binding(15) var<uniform> rt_shadow_flags: RtSunShadowFlags;",
                "var<private> rt_shadow_flags: RtSunShadowFlags;",
            )
            .replace("@group(3) @binding(21)", "@group(0) @binding(1)")
            .replace("@group(3) @binding(22)", "@group(0) @binding(2)")
            .replace("@group(3) @binding(23)", "@group(0) @binding(3)");
        let source = format!(
            r#"
struct Camera {{ camera_pos_time: vec4<f32>, render_flags: vec4<u32> }};
struct VertexOut {{ clip_position: vec4<f32>, world_position: vec3<f32>, world_normal: vec3<f32> }};
struct PointLight {{ position_radius: vec4<f32>, color_intensity: vec4<f32>, emitter: vec4<f32>, shape: vec4<f32>, shadow: vec4<f32> }};
var<private> camera: Camera;
@group(0) @binding(0) var<storage, read> dynamic_lights: array<PointLight>;
@group(0) @binding(4) var<storage, read_write> results: array<vec4<f32>>;
fn local_light_shadow_visibility(light: PointLight, p: vec3<f32>, n: vec3<f32>) -> f32 {{ return 0.875; }}
fn rt_local_shadow_rgb(light: PointLight, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {{ return vec3<f32>(0.875); }}
{receiver}
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    camera = Camera(vec4<f32>(0.0, 0.0, 1.0, 0.0), vec4<u32>(select(0u, 1u, id.x == 6u), 0u, 0u, 0u));
    rt_shadow_flags = RtSunShadowFlags(vec4<u32>(0u, 2u, select(1u, 0u, id.x == 7u), 0u));
    let normal = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), id.x == 5u);
    let input = VertexOut(vec4<f32>(f32(id.x) + 0.5, 0.5, 0.0, 1.0), vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0));
    var sampled = dynamic_lights[id.x];
    sampled.emitter = vec4<f32>(0.0);
    sampled.shadow.z = 0.0;
    if (id.x == 8u || id.x == 9u) {{
        results[id.x] = vec4<f32>(rt_denoised_local_throughput(sampled, input, normal, id.x), 1.0);
    }} else {{
        results[id.x] = vec4<f32>(vec3<f32>(rt_denoised_local_visibility(sampled, input, normal, id.x)), 1.0);
    }}
}}
"#
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("local denoiser receiver regression"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let lights: Vec<[[f32; 4]; 5]> = (0..11)
            .map(|i| {
                [
                    [0.0, 0.0, 2.0, 20.0],
                    [1.0; 4],
                    [0.0, 0.0, 1.0, if i == 1 { 0.0 } else { -1.0 }],
                    [0.0; 4],
                    [0.0, 0.0, 0.0, if i == 4 { 0.0 } else { 1.0 }],
                ]
            })
            .collect();
        let light_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&lights),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut raw = Vec::<[f32; 4]>::new();
        let mut filtered = Vec::<[f32; 4]>::new();
        let mut reference = Vec::<[f32; 4]>::new();
        for i in 0..11 {
            raw.push([0.0, 0.0, 0.0, if i == 2 { 99.0 } else { (i + 1) as f32 }]);
            filtered.push(if i == 8 {
                [0.25, 0.5, 1.5, 1.0]
            } else if i == 9 {
                [0.0, 0.0, 0.25, 1.0]
            } else {
                [0.25, 0.25, 0.25, if i == 3 { 10.0 } else { 1.0 }]
            });
            reference.push(if i == 9 {
                [0.0, 0.0, 1.0, (i + 1) as f32]
            } else if i == 10 {
                [0.0, 0.0, 0.0, (i + 1) as f32]
            } else {
                [1.0, 1.0, 1.0, (i + 1) as f32]
            });
        }
        let texture = |data: &[[f32; 4]]| {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 11,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                tex.as_image_copy(),
                bytemuck::cast_slice(data),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(11 * 16),
                    rows_per_image: Some(1),
                },
                tex.size(),
            );
            tex.create_view(&Default::default())
        };
        let filtered = texture(&filtered);
        let raw = texture(&raw);
        let reference = texture(&reference);
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 11 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 11 * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&filtered),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&raw),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&reference),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(11, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 11 * 16);
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let data = readback.slice(..).get_mapped_range();
        let expected = [
            [0.25; 3],
            [0.875; 3],
            [0.875; 3],
            [0.875; 3],
            [0.875; 3],
            [0.875; 3],
            [0.875; 3],
            [0.875; 3],
            [0.25, 0.5, 1.5],
            [1.0, 1.0, 0.25],
            [0.875; 3],
        ];
        for (i, actual) in bytemuck::cast_slice::<u8, [f32; 4]>(&data)
            .iter()
            .enumerate()
        {
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[i][channel]).abs() < 0.00001,
                    "receiver case {i}: {actual:?} != {:?}",
                    expected[i]
                );
            }
        }
    });
}

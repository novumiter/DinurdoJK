use super::*;
use crate::renderer::{
    block_on, uniform_entry, upload_texture, DynamicWireframeClass, FxGpuSpriteInstance,
    FxGpuSprites, Ghoul2GpuBone, TextureData,
};

#[test]
#[ignore = "requires a Vulkan GPU"]
fn rt_compute_reads_multiple_complete_draw_records() {
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
        let input = Ghoul2RtInputVertex {
            position_uv_x: [1.0, 2.0, 3.0, 0.25],
            normal_uv_y: [0.0, 0.0, 1.0, 0.75],
            bone_indices: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
            params: [1, 0, 0, 0],
        };
        let bone = Ghoul2GpuBone {
            row0: [1.0, 0.0, 0.0, 0.0],
            row1: [0.0, 1.0, 0.0, 0.0],
            row2: [0.0, 0.0, 1.0, 0.0],
        };
        let draws: Vec<_> = [[10.0, 20.0, 30.0], [100.0, 200.0, 300.0]]
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let mut draw = Ghoul2GpuDrawUniform::zeroed();
                draw.axis0 = [1.0, 0.0, 0.0, 0.0];
                draw.axis1 = [0.0, 1.0, 0.0, 0.0];
                draw.axis2 = [0.0, 0.0, 1.0, 0.0];
                draw.origin = [p[0], p[1], p[2], 0.0];
                draw.color = [1.0; 4];
                draw.params[2] = i as u32;
                draw.uv_xform = [1.0, 1.0, 0.0, 0.0];
                draw.source_phong = [13.0, 17.0, 19.0, 23.0];
                draw.source_fresnel = [29.0, 31.0, 37.0, 41.0];
                draw
            })
            .collect();
        let buffer = |data: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: data,
                usage: wgpu::BufferUsages::STORAGE,
            })
        };
        let input = buffer(bytemuck::bytes_of(&input));
        let indices = buffer(bytemuck::cast_slice(&[0u32, 1]));
        let bones = buffer(bytemuck::bytes_of(&bone));
        let draws = buffer(bytemuck::cast_slice(&draws));
        let bytes = (2 * std::mem::size_of::<DynamicModelVertex>()) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("../../ghoul2_skin_compute.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let group = |index, buffers: &[&wgpu::Buffer]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(index),
                entries: &buffers
                    .iter()
                    .enumerate()
                    .map(|(i, b)| wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: b.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            })
        };
        let mesh = group(0, &[&input, &indices]);
        let globals = group(1, &[&bones, &draws, &output]);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &mesh, &[]);
            pass.set_bind_group(1, &globals, &[]);
            pass.dispatch_workgroups(1, 2, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
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
        let vertices = bytemuck::cast_slice::<u8, DynamicModelVertex>(&data);
        assert_eq!(vertices[0].position, [11.0, 33.0, -22.0]);
        assert_eq!(vertices[1].position, [101.0, 303.0, -202.0]);
        assert_eq!(vertices[1].color, [1.0; 4]);
    });
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn fx_survives_raster_rt_and_back_preparation() {
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
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                required_limits: adapter.limits(),
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                ..Default::default()
            })
            .await
            .unwrap();
        let camera = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let white = upload_texture(
            &device,
            &queue,
            &TextureData {
                label: "state-test-white".into(),
                width: 1,
                height: 1,
                rgba: vec![255; 4],
                source: None,
                rgba16f: None,
                mip_level_count: 1,
                clamp: true,
                srgb: true,
            },
        );
        let mut models = DynamicModelRenderer::new(
            &device,
            &queue,
            &camera,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            1,
            &white,
        );
        let mut jobs = PipelineJobManager::new();
        let surface = DynamicModelSurface {
            geometry_source: crate::renderer::ModelGeometrySource::Fx,
            entity_num: 7,
            wireframe_class: DynamicWireframeClass::Entity,
            raster_visible: true,
            vertices: Arc::new(Vec::new()),
            indices: Arc::new(Vec::new()),
            lighting_origin: None,
            rt_rigid: None,
            rt_skinned_key: None,
            ghoul2_gpu: None,
            fx_gpu_sprites: Some(FxGpuSprites {
                instances: Arc::new(vec![FxGpuSpriteInstance::zeroed(); 2]),
                blob_shadow: false,
            }),
            texture: None,
            alpha_mode: DynamicModelAlphaMode::AdditiveOne,
            authored_glow: true,
        };
        for rt in [false, true, false, true] {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                models.prepare(
                    &mut crate::renderer::brightness::BakedBrightness::default(),
                    &mut jobs,
                    &device,
                    &queue,
                    std::slice::from_ref(&surface),
                    None,
                    false,
                    rt,
                    rt,
                    true,
                );
                assert_eq!(
                    models.fx_sprite_draws.len(),
                    1,
                    "sprite draw dropped with RT={rt}"
                );
                assert_eq!(models.fx_sprite_draws[0].instance_count, 2);
                assert!(models.fx_sprite_draws[0].authored_glow);
                if models.rt_prepared == rt && models.fx_sprite_pipeline_ready(surface.alpha_mode) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "FX did not become renderable with RT={rt}"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    });
}

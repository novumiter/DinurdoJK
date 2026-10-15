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
        let data = readback.slice(..).get_mapped_range().expect("GPU readback range");
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
            &crate::ocean::optics::layout(&device),
            wgpu::TextureFormat::Bgra8UnormSrgb,
            1,
            &white,
        );
        let mut jobs = PipelineJobManager::new();
        let surface = DynamicModelSurface {
            geometry_source: crate::renderer::ModelGeometrySource::Fx,
            authored_lighting: false,
            material_trace: None,
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
                resident: None,
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

/// Adaptive instancing must see small groups, scattered two-player groups,
/// and repeats late in a long submission list. The companion cannot be
/// instanced with other players, and transparency remains order-preserving.
#[test]
fn ghoul2_adaptive_batching_detects_real_savings_without_sampling() {
    use crate::renderer::{DynamicModelAlphaMode as Alpha, Ghoul2PendingDraw};
    use bytemuck::Zeroable;

    fn surface(entity: u16, mesh: &str, alpha: Alpha) -> Ghoul2PendingDraw {
        Ghoul2PendingDraw {
            entity_num: entity,
            mesh_key: Arc::from(mesh),
            texture_key: Some(Arc::from("common_skin")),
            alpha_mode: alpha,
            wireframe_class: DynamicWireframeClass::Player,
            authored_glow: false,
            source_material: false,
            texture_binding_id: 1,
            uniform: Ghoul2GpuDrawUniform::zeroed(),
            sequence: 0,
            batch_hash: None,
            batch_group_id: None,
        }
    }
    let mut counts = HashMap::new();
    let mut small = vec![
        surface(1, "body", Alpha::Opaque),
        surface(1, "head", Alpha::Opaque),
        surface(2, "body", Alpha::Opaque),
        surface(2, "head", Alpha::Opaque),
    ];
    assert!(DynamicModelRenderer::should_adaptively_batch(
        &mut small, false, true, None, &mut counts));

    let mut tail: Vec<_> = (0..150)
        .map(|i| surface(i as u16, &format!("unique_mesh_{i}"), Alpha::Opaque))
        .collect();
    tail.extend([surface(201, "late", Alpha::Opaque),
        surface(202, "late", Alpha::Opaque), surface(203, "late", Alpha::Opaque)]);
    assert!(DynamicModelRenderer::should_adaptively_batch(
        &mut tail, false, true, None, &mut counts));

    let mut excluded = vec![surface(9, "body", Alpha::Opaque),
        surface(9, "body", Alpha::Opaque), surface(9, "body", Alpha::Opaque)];
    assert!(!DynamicModelRenderer::should_adaptively_batch(
        &mut excluded, false, true, Some(9), &mut counts));
    for draw in &mut excluded { draw.alpha_mode = Alpha::Blend; draw.batch_hash = None; }
    assert!(!DynamicModelRenderer::should_adaptively_batch(
        &mut excluded, false, true, None, &mut counts));

    // Same surface in different pipeline classes cannot be merged.
    let mut materials = vec![surface(1, "body", Alpha::Opaque),
        surface(2, "body", Alpha::Opaque), surface(3, "body", Alpha::Opaque)];
    materials[1].source_material = true;
    assert!(!DynamicModelRenderer::should_adaptively_batch(
        &mut materials, false, true, None, &mut counts));
    // Glow and wireframe class boundaries are only necessary in their
    // relevant render modes.
    materials[1].source_material = false;
    materials[1].authored_glow = true;
    materials[2].wireframe_class = DynamicWireframeClass::Entity;
    for draw in &mut materials { draw.batch_hash = None; }
    assert!(!DynamicModelRenderer::should_adaptively_batch(
        &mut materials, true, true, None, &mut counts));
    assert!(DynamicModelRenderer::should_adaptively_batch(
        &mut materials, false, false, None, &mut counts));
}

#[test]
fn ghoul2_material_sort_keeps_instance_keys_contiguous_and_ordered() {
    use crate::renderer::{DynamicModelAlphaMode as Alpha, Ghoul2PendingDraw};
    use bytemuck::Zeroable;
    let make = |seq: u32, mesh: &str, texture_id: u64, source: bool, alpha: Alpha| {
        Ghoul2PendingDraw {
            entity_num: seq as u16,
            mesh_key: Arc::from(mesh),
            texture_key: Some(Arc::from(format!("skin{texture_id}"))),
            alpha_mode: alpha,
            wireframe_class: DynamicWireframeClass::Player,
            authored_glow: false,
            source_material: source,
            texture_binding_id: texture_id,
            uniform: Ghoul2GpuDrawUniform::zeroed(),
            sequence: seq,
            batch_hash: None,
            batch_group_id: None,
        }
    };
    let mut draws = vec![
        make(0, "mesh1", 5, false, Alpha::Opaque),
        make(1, "mesh1", 6, false, Alpha::Opaque),
        make(2, "overlay1", 1, false, Alpha::Blend),
        make(3, "mesh1", 5, false, Alpha::Opaque),
        make(4, "mesh2", 5, false, Alpha::Opaque),
        make(5, "mesh1", 5, true, Alpha::Opaque),
        make(6, "mesh1", 5, true, Alpha::Opaque),
        make(7, "overlay2", 1, false, Alpha::Blend),
    ];
    for draw in &mut draws {
        if draw.alpha_mode == Alpha::Opaque {
            draw.batch_hash = Some(DynamicModelRenderer::ghoul2_batch_hash(draw, false));
        }
    }
    DynamicModelRenderer::sort_ghoul2_pending(&mut draws, false, false, None);
    // Source material/pipeline is never interleaved with non-Source.
    assert!(draws[..3].iter().all(|d| !d.source_material && d.texture_binding_id == 5));
    assert_eq!(draws[3].texture_binding_id, 6);
    assert!(draws[4..6].iter().all(|d| d.source_material && d.texture_binding_id == 5));
    // The two copies of mesh1 must stay adjacent (one instanced draw).
    let non_source_mesh1: Vec<usize> = draws.iter().enumerate()
        .filter(|(_, d)| !d.source_material && d.mesh_key.as_ref() == "mesh1" && d.texture_binding_id == 5)
        .map(|(i, _)| i).collect();
    assert_eq!(non_source_mesh1.len(), 2);
    assert_eq!(non_source_mesh1[0] + 1, non_source_mesh1[1]);
    assert_eq!((draws[6].sequence, draws[7].sequence), (2, 7));
    // Even if a textured character is hidden in the companion view, it must
    // stay outside opaque instance groups, and ordered draws stay ordered.
    DynamicModelRenderer::sort_ghoul2_pending(&mut draws, false, false, Some(3));
    assert_eq!(draws.iter().filter(|d| d.alpha_mode == Alpha::Blend)
        .map(|d| d.sequence).collect::<Vec<_>>(), vec![2, 7]);
}

/// Visible grouping refreshes every rendered frame, not on server snapshots.
/// Permanent group IDs are constant; eligibility, companion view, LOD and
/// appearance are evaluated against the current submissions.
#[test]
fn cached_ghoul2_grouping_updates_every_frame_without_full_surface_sort() {
    use crate::renderer::{DynamicModelAlphaMode as Alpha, Ghoul2BatchMode, Ghoul2PendingDraw};
    use bytemuck::Zeroable;
    let make = |entity: u16, id: Option<u64>, alpha: Alpha, texture: u64| Ghoul2PendingDraw {
        entity_num: entity,
        mesh_key: Arc::from(format!("mesh{}", id.unwrap_or(0))),
        texture_key: Some(Arc::from(format!("texture{texture}"))),
        alpha_mode: alpha,
        wireframe_class: DynamicWireframeClass::Player,
        authored_glow: false,
        source_material: false,
        texture_binding_id: texture,
        uniform: Ghoul2GpuDrawUniform::zeroed(),
        sequence: entity as u32,
        batch_hash: None,
        batch_group_id: id,
    };
    let mut lookup = HashMap::new();
    let mut groups = Vec::new();
    let mut sort = Vec::new();
    let mut order = Vec::new();
    let mut frame = vec![
        make(1, Some(41), Alpha::Opaque, 3),
        make(2, Some(90), Alpha::Opaque, 1),
        make(3, Some(41), Alpha::Opaque, 3),
        make(4, Some(90), Alpha::Opaque, 1),
        make(5, None, Alpha::Blend, 6),
        make(6, Some(41), Alpha::Opaque, 3),
    ];
    let batch = |draws: &[Ghoul2PendingDraw], excluded: Option<u16>,
        lookup: &mut HashMap<(u64, u32), usize>,
        groups: &mut Vec<crate::renderer::models::prepare::Ghoul2FrameGroup>,
        sort: &mut Vec<usize>, order: &mut Vec<usize>| {
        DynamicModelRenderer::prepare_cached_ghoul2_order(
            draws, true, excluded, Ghoul2BatchMode::Adaptive, lookup, groups, sort, order)
    };
    assert!(batch(&frame, None, &mut lookup, &mut groups, &mut sort, &mut order));
    assert_eq!(order, [1, 3, 0, 2, 5, 4]); // Texture locality, blend last.
    let first_group_capacity = groups.len();
    // Next frame: same static mesh IDs, different visibility and companion.
    frame.remove(3);
    assert!(!batch(&frame, Some(3), &mut lookup, &mut groups, &mut sort, &mut order));
    assert!(order.is_empty()); // Adaptive preserves incoming order.
    assert!(groups.len() >= first_group_capacity); // Buckets reused.
    // New model/appearance and one dynamic mask pipeline, no snapshot involved.
    frame.push(make(7, Some(91), Alpha::Mask, 7));
    frame.push(make(8, Some(91), Alpha::Mask, 7));
    frame.push(make(9, Some(91), Alpha::Mask, 7));
    assert!(batch(&frame, Some(3), &mut lookup, &mut groups, &mut sort, &mut order));
    let positioned: Vec<u16> = order.iter().map(|&i| frame[i].entity_num).collect();
    let masked = positioned.iter().position(|&entity| entity == 7).unwrap();
    assert_eq!(&positioned[masked..masked + 3], &[7, 8, 9]);
    // Excluded and blended draws retain their original relative order.
    assert_eq!(&positioned[positioned.len() - 2..], &[3, 5]);
    assert!(DynamicModelRenderer::prepare_cached_ghoul2_order(
        &frame, true, None, Ghoul2BatchMode::Force,
        &mut lookup, &mut groups, &mut sort, &mut order,
    ));
    assert_eq!(order.len(), frame.len());
    assert!(order.iter().all(|&index| index < frame.len()));
}

/// Even identical mesh/material IDs cannot cross wireframe classes when the
/// renderer requests separate classes; the fast path must match the old path.
#[test]
fn cached_ghoul2_grouping_honors_wireframe_class_and_alpha() {
    use crate::renderer::{DynamicModelAlphaMode as Alpha, Ghoul2BatchMode, Ghoul2PendingDraw};
    use bytemuck::Zeroable;
    let make = |num: u16, alpha, wireframe_class, group| Ghoul2PendingDraw {
        entity_num: num, mesh_key: Arc::from("model#surface1"),
        texture_key: Some(Arc::from("common_skin")),
        alpha_mode: alpha, wireframe_class, authored_glow: false,
        source_material: false, texture_binding_id: 1,
        uniform: Ghoul2GpuDrawUniform::zeroed(), sequence: num as u32,
        batch_hash: None, batch_group_id: group,
    };
    let mut lookup = HashMap::new();
    let mut groups = Vec::new();
    let mut sort = Vec::new();
    let mut order = Vec::new();
    let a = make(1, Alpha::Opaque, DynamicWireframeClass::Player, Some(42));
    let b = make(2, Alpha::Opaque, DynamicWireframeClass::Entity, Some(42));
    let c = make(3, Alpha::Blend, DynamicWireframeClass::Player, None);
    let frame = [a, b, c];
    assert!(!DynamicModelRenderer::prepare_cached_ghoul2_order(
        &frame, true, None, Ghoul2BatchMode::Adaptive,
        &mut lookup, &mut groups, &mut sort, &mut order,
    ));
    assert!(DynamicModelRenderer::prepare_cached_ghoul2_order(
        &frame, false, None, Ghoul2BatchMode::Force,
        &mut lookup, &mut groups, &mut sort, &mut order,
    ));
    assert_eq!(order, [0, 1, 2]);
    assert_eq!(groups[0].indices.as_slice(), &[0, 1]);
}

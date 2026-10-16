use super::*;

#[test]
#[ignore = "requires a GPU adapter; run explicitly"]
fn caustic_receivers_survive_water_boundary_roundoff() {
    futures_lite::future::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .unwrap();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let ocean_layout = crate::ocean::OceanGpu::create_render_layout(&device);
        let ocean = crate::ocean::OceanGpu::create_inert_render_bind_group(&device, &ocean_layout);
        let output_layout = layout(&device);
        let extent = wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        };
        for samples in [1, 4] {
            let mut resources = Resources::new(
                &device,
                &output_layout,
                &ocean_layout,
                (8, 8, samples, wgpu::TextureFormat::Rgba8Unorm),
            );
            // Observe the actual capture shader's receiver classification without
            // depending on the time-varying FFT field or lighting intensity.
            let source = shader_source(samples > 1).replace(
                "out.color = vec4<f32>(color, 1.0);",
                "out.color = vec4<f32>(vec3<f32>(select(0.0, 1.0, bottom_depth > 2.0)), 1.0);",
            );
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Caustic receiver boundary probe"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            resources.capture = pipeline(
                &device,
                &shader,
                &[&resources.capture_layout, &ocean_layout],
                "fs_capture",
                &[
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureFormat::R32Float,
                ],
                1,
            );
            let texture = |format, count, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Caustic receiver test"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let color = texture(
                wgpu::TextureFormat::Rgba8Unorm,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            );
            let color_view = color.create_view(&Default::default());
            let input = texture(
                wgpu::TextureFormat::Rgba8Unorm,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let input_view = input.create_view(&Default::default());
            let depth = texture(
                wgpu::TextureFormat::Depth32Float,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let depth_view = depth.create_view(&Default::default());
            // hothchallenges pool bounds in renderer coordinates. Its snow wall
            // is exactly z=-7728; inverse projection can land on either side.
            let lo = glam::Vec3::new(-2144.0, 12280.0, -9016.0);
            let hi = glam::Vec3::new(3488.0, 12776.0, -7728.0);
            for axis in [0, 2] {
                for upper in [false, true] {
                    for (error, expected) in
                        [(-0.1, 255), (0.0, 255), (0.1, 255), (0.5, 255), (2.0, 0)]
                    {
                        let mut world = glam::Vec3::new(-586.0, 12636.0, -7801.0);
                        world[axis] = if upper {
                            hi[axis] + error
                        } else {
                            lo[axis] - error
                        };
                        let mut scale = glam::Vec3::splat(32.0);
                        scale[axis] = 0.0;
                        let mut uniform = Uniform::zeroed();
                        uniform.inverse_view_projection =
                            glam::Mat4::from_scale_rotation_translation(
                                scale,
                                glam::Quat::IDENTITY,
                                world,
                            )
                            .to_cols_array_2d();
                        uniform.eye = [-586.0, 12636.0, -7801.0, 1.0];
                        uniform.forward = [0.0, 0.0, 1.0, 0.0];
                        uniform.fog[3] = 2304.0;
                        uniform.params = [2304.0, 0.0, 0.0, 1.0];
                        uniform.minimum[0] = lo.extend(0.0).to_array();
                        uniform.maximum[0] = hi.extend(0.0).to_array();
                        queue.write_buffer(&resources.uniform, 0, bytemuck::bytes_of(&uniform));
                        let mut encoder = device.create_command_encoder(&Default::default());
                        {
                            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &input_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: Some(
                                    wgpu::RenderPassDepthStencilAttachment {
                                        view: &depth_view,
                                        stencil_ops: None,
                                        depth_ops: Some(wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(0.5),
                                            store: wgpu::StoreOp::Store,
                                        }),
                                    },
                                ),
                                ..Default::default()
                            });
                        }
                        resources.capture(
                            &device,
                            &mut encoder,
                            &input_view,
                            &depth_view,
                            &ocean,
                            None,
                        );
                        resources.composite(
                            &mut encoder,
                            if samples == 1 { &color_view } else { &input_view },
                            (samples > 1).then_some(&color_view),
                        );
                        let readback = device.create_buffer(&wgpu::BufferDescriptor {
                            label: None,
                            size: 256 * 8,
                            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        });
                        encoder.copy_texture_to_buffer(
                            color.as_image_copy(),
                            wgpu::TexelCopyBufferInfo {
                                buffer: &readback,
                                layout: wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(256),
                                    rows_per_image: Some(8),
                                },
                            },
                            extent,
                        );
                        queue.submit([encoder.finish()]);
                        readback
                            .slice(..)
                            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
                        device
                            .poll(wgpu::PollType::Wait {
                                submission_index: None,
                                timeout: None,
                            })
                            .unwrap();
                        let bytes = readback.slice(..).get_mapped_range().expect("GPU readback range");
                        for y in 0..8 {
                            for x in 0..8 {
                                assert_eq!(bytes[y * 256 + x * 4], expected,
                                    "samples={samples} axis={axis} upper={upper} error={error} pixel=({x},{y})");
                            }
                        }
                    }
                }
            }
        }
        assert!(scope.pop().await.is_none(), "GPU validation error");
    });
}

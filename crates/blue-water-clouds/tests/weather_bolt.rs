use blue_water_clouds::{CloudView, Targets, lightning::StrikeKind, weather::Weather};
use naval_sky::{
    Sky, SkyQuality, SkyScene, SkyTargets,
    math::{Mat4, Vec3},
};

#[test]
fn ground_lightning_draws_a_channel_beyond_the_host_far_plane() {
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
        let sky = Sky::new(
            &device,
            &queue,
            SkyQuality::Low,
            SkyTargets {
                color: wgpu::TextureFormat::Rgba16Float,
                depth: None,
            },
        );
        let mut weather = Weather::new(
            &device,
            sky.bind_group_layout(),
            SkyQuality::Low,
            Targets {
                color: wgpu::TextureFormat::Rgba8Unorm,
                depth: Some(wgpu::TextureFormat::Depth32Float),
            },
        );
        let mut scene = SkyScene::north_atlantic(35., 135.);
        scene.clouds.altitude = 650.;
        scene.weather.precipitation = 0.;
        scene.weather.lightning = 0.;
        weather.apply(&scene);
        let eye = Vec3::new(0., 2., 0.);
        let view = CloudView {
            position: eye,
            view: Mat4::look_at_rh(eye, Vec3::new(0., 325., -2600.), Vec3::Y),
            projection: Mat4::perspective_rh(52f32.to_radians(), 1., 260., 0.025),
            near: 0.025,
            far: 260.,
            viewport: [512, 512],
        };
        weather.force_strike(
            &queue,
            Vec3::new(0., 0., -2600.),
            0.006,
            StrikeKind::Ground,
            7,
        );
        weather.update(
            &mut naval_render::UploadSink::Queue(&queue),
            &view,
            0.,
            true,
            false,
        );
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bolt image regression"),
                size: wgpu::Extent3d {
                    width: 512,
                    height: 512,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let reactive =
            blue_water_clouds::reactive::WeatherReactive::new(&device, weather.draw_layouts());
        let motion = texture(
            wgpu::TextureFormat::Rg16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let motion_view = motion.create_view(&Default::default());
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 512 * 512 * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for blocked in [false, true] {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(if blocked { 0.25 } else { 0. }),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                weather.draw(&mut pass, sky.bind_group());
            }
            encoder.copy_texture_to_buffer(
                color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(2048),
                        rows_per_image: Some(512),
                    },
                },
                color.size(),
            );
            queue.submit(Some(encoder.finish()));
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            let mapped = read.slice(..).get_mapped_range().unwrap();
            let mut rows = [false; 512];
            let mut lit = 0;
            for (i, pixel) in mapped.chunks_exact(4).enumerate() {
                if pixel[2] > 12 {
                    rows[i / 512] = true;
                    lit += 1;
                }
            }
            if blocked {
                assert_eq!(lit, 0, "near geometry must occlude the channel");
            } else {
                if let Ok(path) = std::env::var("BLUE_WATER_BOLT_PROBE") {
                    image::save_buffer(path, &mapped, 512, 512, image::ColorType::Rgba8).unwrap();
                }
                let vertical = rows.iter().filter(|&&v| v).count();
                eprintln!("ground bolt: {lit} lit pixels across {vertical} rows");
                assert!(
                    vertical > 100,
                    "the output must contain a long channel, not just a point glow"
                );
            }
            drop(mapped);
            read.unmap();
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &motion_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                weather.draw_reactive(&mut pass, sky.bind_group(), &reactive.0);
            }
            encoder.copy_texture_to_buffer(
                motion.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(2048),
                        rows_per_image: Some(512),
                    },
                },
                motion.size(),
            );
            queue.submit(Some(encoder.finish()));
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            let mapped = read.slice(..).get_mapped_range().unwrap();
            let rejected = mapped
                .chunks_exact(4)
                .filter(|p| p[0] == 0 && p[1] == 0x40)
                .count();
            if blocked {
                assert_eq!(rejected, 0);
            } else {
                assert!(
                    rejected > 100,
                    "visible channel pixels must reject old TAA history"
                );
            }
            drop(mapped);
            read.unmap();
        }
    });
}

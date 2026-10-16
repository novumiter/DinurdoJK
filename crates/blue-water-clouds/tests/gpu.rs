use blue_water_clouds::{CloudLayer, CloudView, Targets};
use naval_sky::{
    Sky, SkyQuality, SkyScene, SkyTargets, SkyView,
    math::{Mat4, Vec3},
};
#[test]
fn original_clouds_shadows_aerial_and_wet_mask_run_on_gpu() {
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
        let mut sky = Sky::new(
            &device,
            &queue,
            SkyQuality::Low,
            SkyTargets {
                color: wgpu::TextureFormat::Rgba16Float,
                depth: None,
            },
        );
        let mut scene = SkyScene::north_atlantic(35.0, 135.0);
        scene.clouds.coverage = 0.75;
        sky.apply(&scene);
        let mut clouds = CloudLayer::new(
            &device,
            &queue,
            &sky,
            SkyQuality::Low,
            Targets {
                color: wgpu::TextureFormat::Rgba16Float,
                depth: None,
            },
        );
        sky.attach_clouds(&device, clouds.hooks());
        clouds.apply(&queue, &mut sky, &scene);
        let volume = naval_render::AerialVolume::new(&device);
        let mut aerial = blue_water_clouds::aerial::AerialPerspective::new(
            &device,
            &sky,
            clouds.shadow(),
            naval_render::MakePipelines::Now,
        );
        let position = Vec3::new(0.0, 2.0, 0.0);
        let view = CloudView {
            position,
            view: Mat4::look_at_rh(position, Vec3::new(0.0, 2000.0, 1000.0), Vec3::Y),
            projection: naval_render::camera::reversed_perspective(
                52f32.to_radians(),
                2.0,
                0.01,
                30000.0,
            ),
            near: 0.01,
            far: 30000.0,
            viewport: [32, 16],
        };
        let sky_view = SkyView {
            position,
            camera_to_world: view.view.inverse(),
            projection: view.projection,
            viewport: view.viewport,
        };
        let vp = view.view_projection();
        let camera = naval_render::CameraUniforms {
            view: view.view.to_cols_array(),
            projection: view.projection.to_cols_array(),
            view_projection: vp.to_cols_array(),
            inverse_view: view.view.inverse().to_cols_array(),
            inverse_projection: view.projection.inverse().to_cols_array(),
            inverse_view_projection: vp.inverse().to_cols_array(),
            position: position.to_array(),
            time: 0.0,
            viewport: [32.0, 16.0, 1.0 / 32.0, 1.0 / 16.0],
            near: view.near,
            far: view.far,
            tan_half_fov_y: (26f32.to_radians()).tan(),
            aspect: 2.0,
            dt: 0.0,
            frame: 0,
            curvature: 0.0,
            padding: 0,
        };
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Cloud validation"),
            size: wgpu::Extent3d {
                width: 32,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output_view = output.create_view(&Default::default());
        let probe = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Wet cloud ray probe"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            probe.as_image_copy(),
            bytemuck::cast_slice(&[0x3c00u16, 0, 0, 0]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            probe.size(),
        );
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut sums = Vec::new();
        for wet in [false, true] {
            clouds.set_underwater_probe(wet.then(|| probe.create_view(&Default::default())));
            let mut encoder = device.create_command_encoder(&Default::default());
            sky.update_tables(&queue, &mut encoder, &sky_view, 1.0 / 60.0);
            clouds.prepare(
                &device,
                &mut naval_render::UploadSink::Queue(&queue),
                &view,
                &sky,
                true,
            );
            if let Some(group) = clouds.take_field_group() {
                sky.set_cloud_bind_group(group);
            }
            sky.update_environment(&queue, &mut encoder);
            clouds.encode_shadows(&mut encoder);
            aerial.encode(
                &device,
                &mut naval_render::UploadSink::Queue(&queue),
                &mut encoder,
                &camera,
                &volume,
                &sky,
                clouds.shadow(),
                [clouds.uniforms().heights[0], clouds.uniforms().heights[1]],
                None,
            );
            clouds.encode_march(&device, &mut encoder, &sky, None);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &output_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                clouds.draw_composite(&mut pass);
            }
            encoder.copy_texture_to_buffer(
                output.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(16),
                    },
                },
                output.size(),
            );
            queue.submit([encoder.finish()]);
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            let mut sum = 0u64;
            {
                let bytes = read.slice(..).get_mapped_range().unwrap();
                for v in bytes.chunks_exact(2) {
                    let h = u16::from_le_bytes(v.try_into().unwrap());
                    assert_ne!(h & 0x7c00, 0x7c00);
                    sum += u64::from(h & 0x7fff);
                }
            }
            read.unmap();
            sums.push(sum);
        }
        assert!(sums[0] > 0, "dry view contains original clouds");
        assert_eq!(
            sums[1], 0,
            "wet view suppresses original clouds, shafts and haze"
        );
        assert!(scope.pop().await.is_none());
    });
}

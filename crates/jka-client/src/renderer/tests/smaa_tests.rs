//! Validate upstream GLSL pipelines, queue ordering and resize on native backends.
use std::time::Duration;

fn exercise(backend: wgpu::Backends) {
    crate::renderer::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: backend,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .expect("SMAA GPU adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .expect("SMAA GPU device");
        println!("SMAA 0.21: {:?}", adapter.get_info());

        for format in [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            for mode in [smaa::SmaaMode::Disabled, smaa::SmaaMode::Smaa1X] {
                let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let mut target = smaa::SmaaTarget::new(&device, &queue, 13, 7, format, mode);
                for (width, height) in [(13, 7), (31, 17)] {
                    target.resize(&device, width, height);
                    let output = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("SMAA smoke output"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
                    let view = output.create_view(&wgpu::TextureViewDescriptor::default());
                    let frame = target.start_frame(&device, &queue, &view);
                    let mut encoder = device.create_command_encoder(&Default::default());
                    {
                        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("SMAA smoke scene"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &frame,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            ..Default::default()
                        });
                    }
                    queue.submit([encoder.finish()]);
                    frame.resolve();

                    // The scene submit must precede SMAA's own resolve submit.
                    let readback = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("SMAA smoke readback"),
                        size: u64::from(height) * 256,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    let mut encoder = device.create_command_encoder(&Default::default());
                    encoder.copy_texture_to_buffer(
                        output.as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(256),
                                rows_per_image: Some(height),
                            },
                        },
                        output.size(),
                    );
                    let submission = queue.submit([encoder.finish()]);
                    let (tx, rx) = std::sync::mpsc::channel();
                    readback
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |result| {
                            let _ = tx.send(result);
                        });
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: Some(Duration::from_secs(30)),
                        })
                        .unwrap();
                    rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
                    let bytes = readback.slice(..).get_mapped_range().unwrap();
                    for row in bytes.chunks_exact(256) {
                        assert!(
                            row[..width as usize * 4].iter().all(|&value| value == 255),
                            "SMAA must preserve solid white, including alpha, after resize"
                        );
                    }
                    drop(bytes);
                    readback.unmap();
                }
                assert!(scope.pop().await.is_none(), "SMAA GPU validation failed");
            }
        }
    });
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn smaa21_vulkan_resolves_and_resizes() {
    exercise(wgpu::Backends::VULKAN);
}

#[test]
#[ignore = "requires a DX12 GPU"]
fn smaa21_dx12_resolves_and_resizes() {
    exercise(wgpu::Backends::DX12);
}

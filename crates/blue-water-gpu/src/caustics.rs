//! Original caustic generator and JKA opaque receiver application.
use crate::{original_surface::OriginalSurfaceBindings, scene_copy::SceneCopy, shader_composer};
use bytemuck::{Pod, Zeroable};
use naga_oil::compose::{NagaModuleDescriptor, ShaderDefValue, ShaderType};
pub const TEXELS: u32 = 256;
pub const LEVELS: u32 = 4;
pub const CEILING: f32 = 25.0;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CausticParams {
    centre: [f32; 4],
    level: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ReceiverParams {
    pub sun: [f32; 4],
    pub light: [f32; 4],
    pub plane: [f32; 4],
    pub absorption: [f32; 4],
    pub minimum: [[f32; 4]; 8],
    pub maximum: [[f32; 4]; 8],
}
fn uniform(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
fn buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Blue Water caustic parameters"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn eb(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}
fn et(binding: u32, t: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(t),
    }
}
fn es(binding: u32, s: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::Sampler(s),
    }
}
fn module(device: &wgpu::Device, source: &str, label: &str) -> wgpu::ShaderModule {
    let mut composer = shader_composer::composer().expect("original ocean modules");
    let module = composer
        .make_naga_module(NagaModuleDescriptor {
            source,
            file_path: label,
            shader_type: ShaderType::Wgsl,
            shader_defs: [("CASCADES".to_owned(), ShaderDefValue::UInt(3))]
                .into_iter()
                .collect(),
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&composer)));
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
    })
}
pub struct Caustics {
    levels: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    pipeline: wgpu::ComputePipeline,
    pub view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    _texture: wgpu::Texture,
    camera_layout: wgpu::BindGroupLayout,
    receiver_layout: wgpu::BindGroupLayout,
    receiver_pipeline: wgpu::RenderPipeline,
}
pub struct Receiver {
    camera: wgpu::BindGroup,
    group: wgpu::BindGroup,
    params: wgpu::Buffer,
    pub enabled: bool,
}
impl Caustics {
    pub fn new(
        device: &wgpu::Device,
        waves: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Original four-depth caustics"),
            size: wgpu::Extent3d {
                width: TEXELS,
                height: TEXELS,
                depth_or_array_layers: LEVELS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Original caustic repeating sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let output = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Original caustics output"),
            entries: &[
                uniform(0, wgpu::ShaderStages::COMPUTE),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let levels = (0..LEVELS)
            .map(|layer| {
                let params = buffer(device, std::mem::size_of::<CausticParams>() as u64);
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Original caustic depth"),
                    layout: &output,
                    entries: &[eb(0, &params), et(1, &view)],
                });
                (params, group)
            })
            .collect();
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Unused caustic groups"),
            entries: &[],
        });
        let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Original caustics"),
            bind_group_layouts: &[Some(&empty), Some(&empty), Some(waves), Some(&output)],
            immediate_size: 0,
        });
        let shader = module(
            device,
            include_str!("../../../vendor/blue-water-sea-and-sky/ocean/shaders/caustics.wgsl"),
            "Original caustics",
        );
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Original caustics"),
            layout: Some(&compute_layout),
            module: &shader,
            entry_point: Some("caustics_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Caustic receiver camera"),
            entries: &[uniform(0, wgpu::ShaderStages::FRAGMENT)],
        });
        let receiver_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("JKA caustic receiver"),
            entries: &[
                uniform(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(
                    3,
                    wgpu::TextureViewDimension::D2,
                    wgpu::TextureSampleType::Float { filterable: false },
                ),
                texture_entry(
                    4,
                    wgpu::TextureViewDimension::D2,
                    wgpu::TextureSampleType::Depth,
                ),
                uniform(16, wgpu::ShaderStages::FRAGMENT),
                texture_entry(
                    17,
                    wgpu::TextureViewDimension::D2Array,
                    wgpu::TextureSampleType::Float { filterable: true },
                ),
                sampler_entry(18),
            ],
        });
        let shader = module(
            device,
            &format!(
                "{}\n{}",
                include_str!("caustics_receiver.wgsl"),
                include_str!("surface_boundary.wgsl")
            ),
            "JKA caustic receivers",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("JKA caustic receiver"),
            bind_group_layouts: &[Some(&camera_layout), Some(&receiver_layout), Some(waves)],
            immediate_size: 0,
        });
        let receiver_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("JKA original caustic receiver"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("receiver_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("receiver_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            levels,
            pipeline,
            view,
            sampler,
            _texture: texture,
            camera_layout,
            receiver_layout,
            receiver_pipeline,
        }
    }
    pub fn write(&self, queue: &wgpu::Queue, centre: [f32; 2]) {
        for (index, (buffer, _)) in self.levels.iter().enumerate() {
            let span = 32.0 * (1u32 << index) as f32;
            let depth = 2.0 * 2.5f32.powi(index as i32);
            let params = CausticParams {
                centre: [centre[0], centre[1], span, TEXELS as f32],
                level: [
                    depth,
                    (span / TEXELS as f32).max(0.06 * depth),
                    4.0,
                    if depth < 10.0 { 1.0 } else { 0.0 },
                ],
            };
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
        }
    }
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, waves: &wgpu::BindGroup) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Original caustics"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(2, waves, &[]);
        for (_, group) in &self.levels {
            pass.set_bind_group(3, group, &[]);
            pass.dispatch_workgroups(TEXELS.div_ceil(8), TEXELS.div_ceil(8), 1);
        }
    }
    pub fn receiver(
        &self,
        device: &wgpu::Device,
        surface: &OriginalSurfaceBindings,
        scene: &SceneCopy,
    ) -> Receiver {
        let params = buffer(device, std::mem::size_of::<ReceiverParams>() as u64);
        let camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Caustic receiver camera"),
            layout: &self.camera_layout,
            entries: &[eb(0, &surface.camera)],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Caustic receiver"),
            layout: &self.receiver_layout,
            entries: &[
                eb(0, &params),
                et(3, &scene.color),
                et(4, &scene.depth),
                eb(16, &surface.boundary),
                et(17, &self.view),
                es(18, &self.sampler),
            ],
        });
        Receiver {
            params,
            camera,
            group,
            enabled: false,
        }
    }
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        receiver: &Receiver,
        waves: &wgpu::BindGroup,
    ) {
        if !receiver.enabled {
            return;
        }
        pass.set_pipeline(&self.receiver_pipeline);
        pass.set_bind_group(0, &receiver.camera, &[]);
        pass.set_bind_group(1, &receiver.group, &[]);
        pass.set_bind_group(2, waves, &[]);
        pass.draw(0..3, 0..1);
    }
}
impl Receiver {
    pub fn write(&mut self, queue: &wgpu::Queue, params: &ReceiverParams) {
        self.enabled = params.sun[3] > 0.0 && params.plane[3] > 0.0;
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caustics_and_refraction_gpu_bindings_accept_flat_field() {
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
            let field = crate::WaveField::new(&device, blue_water_core::QualityTier::High);
            let waves = crate::surface_waves::SurfaceWaves::new(&device, &field);
            let surface = crate::original_surface::OriginalSurfacePipeline::new(
                &device,
                &waves,
                wgpu::TextureFormat::Rgba16Float,
                1,
                16,
            );
            let bindings = OriginalSurfaceBindings::new(&device, &queue, &surface);
            let scene = SceneCopy::new(&device, 16, 16, 1);
            let caustics =
                Caustics::new(&device, waves.layout(), wgpu::TextureFormat::Rgba16Float, 1);
            let mut receiver = caustics.receiver(&device, &bindings, &scene);
            let mut params = ReceiverParams::zeroed();
            params.sun = [0.0, 1.0, 0.0, 1.0];
            params.plane = [0.0, 0.0, 40.0, 1.0];
            receiver.write(&queue, &params);
            caustics.write(&queue, [0.0, 0.0]);
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 1024,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            caustics.encode(&mut encoder, waves.group(&field));
            encoder.copy_texture_to_buffer(
                caustics._texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 4,
                },
            );
            let out = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 16,
                        height: 16,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &out,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                caustics.draw(&mut pass, &receiver, waves.group(&field));
            }
            queue.submit([encoder.finish()]);
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |result| result.unwrap());
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            {
                let bytes = read.slice(..).get_mapped_range().unwrap();
                for layer in 0..4 {
                    let offset = layer * 256;
                    assert_eq!(
                        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()),
                        0x3c00,
                        "flat waves give unit caustic gain at every depth"
                    );
                }
            }
            read.unmap();
            assert!(
                scope.pop().await.is_none(),
                "caustic receiver and refractive surface layouts validate"
            );
        });
    }
}

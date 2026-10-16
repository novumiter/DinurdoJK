//! Apply the preserved atmosphere froxels to JKA's opaque receiver before water transmission.
use blue_water_gpu::scene_copy::SceneCopy;
use blue_water_render::CameraUniforms;
pub(super) struct AirComposite {
    copy: SceneCopy,
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}
impl AirComposite {
    pub fn new(
        device: &wgpu::Device,
        key: (u32, u32, u32, wgpu::TextureFormat),
        air: &blue_water_render::AerialVolume,
    ) -> Self {
        let copy = SceneCopy::new(device, key.0, key.1, key.2);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blue Water opaque air camera"),
            size: 96,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tex = |binding, sample_type, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blue Water opaque aerial"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex(
                    1,
                    wgpu::TextureSampleType::Float { filterable: false },
                    wgpu::TextureViewDimension::D2,
                ),
                tex(
                    2,
                    wgpu::TextureSampleType::Depth,
                    wgpu::TextureViewDimension::D2,
                ),
                tex(
                    3,
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D3,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let texture = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water opaque air"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                texture(1, &copy.color),
                texture(2, &copy.depth),
                texture(3, &air.volume_view),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&air.sampler),
                },
            ],
        });
        // Reuse the original binding-free sampling functions unchanged.
        let library = include_str!(
            "../../../../../vendor/blue-water-sea-and-sky/shared/render/shaders/aerial_volume.wgsl"
        )
        .lines()
        .filter(|line| !line.starts_with("#define_import_path"))
        .collect::<Vec<_>>()
        .join("\n");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blue Water original aerial sampling"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{library}\n{}", include_str!("blue_water_air.wgsl")).into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blue Water opaque aerial"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(key.3.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: key.2,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            copy,
            uniforms,
            group,
            pipeline,
        }
    }
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera: &CameraUniforms,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
    ) {
        let mut values = [0.0f32; 24];
        values[..16].copy_from_slice(&camera.inverse_view_projection);
        values[16..19].copy_from_slice(&camera.position);
        values[20..].copy_from_slice(&camera.viewport);
        queue.write_buffer(&self.uniforms, 0, bytemuck::cast_slice(&values));
        self.copy.capture(device, encoder, color, depth);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Blue Water opaque air composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

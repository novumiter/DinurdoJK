//! Hybrid caustic experiment. Generates a cyclic 32-phase interference field on
//! the GPU once per optics-resource lifetime; sampling costs two RGBA8 texture
//! reads instead of FFT normals and finite differences per receiver pixel.

pub(super) struct Pattern {
    pub(super) view: wgpu::TextureView,
    pub(super) sampler: wgpu::Sampler,
    initialized: bool,
}

impl Pattern {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hybrid animated caustics: 8x256x256 RGBA8 (32 phases packed)"),
            size: wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 8 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Hybrid caustics repeat bilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { view, sampler, initialized: false }
    }

    pub(super) fn ensure_generated(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if self.initialized { return; }
        // The shader and pipeline are created only if Hybrid Animated is
        // selected. Other ocean types do not specialize this compute shader.
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hybrid procedural caustics generation"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                }, count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Hybrid procedural caustics storage"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.view) }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Hybrid procedural caustics synthesis"),
            source: wgpu::ShaderSource::Wgsl(include_str!("procedural.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Hybrid procedural caustics compute layout"),
            bind_group_layouts: &[Some(&layout)], immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Hybrid procedural caustics compute"),
            layout: Some(&pipeline_layout), module: &shader, entry_point: Some("main"),
            compilation_options: Default::default(), cache: crate::pipeline_cache::get().as_ref(),
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Hybrid caustics one-time 32 phase generation"),
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(32, 32, 8);
        drop(pass);
        self.initialized = true;
    }
}

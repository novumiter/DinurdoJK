//! Shared spatiotemporal filtering for two independent HDR direct-light signals:
//! sun incident radiance from the visibility cache and albedo-free transient
//! local-light irradiance from the separate ray-query compute pass. Final scene
//! color, material BRDF, reflected/specular and indirect light are never blurred.
//! The local signal remains restricted to the dominant finite transient light
//! at each depth-prepass pixel; other receivers keep the original direct rays.
use super::*;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RadianceParams {
    inverse_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    previous_eye: [f32; 4],
    sun_rgb: [f32; 4],
    dimensions: [u32; 4],
    flags: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FilterParams { dimensions: [u32; 4] }

pub(super) fn texture(device: &wgpu::Device, label: &str, dims: [u32; 2], format: wgpu::TextureFormat) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: dims[0], height: dims[1], depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v)
}
pub(super) fn storage(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format, view_dimension: wgpu::TextureViewDimension::D2,
        },
    }
}
pub(super) fn bind_texture<'a>(binding: u32, view: &'a wgpu::TextureView) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) }
}
pub(super) fn bind_buffer(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}

struct RadianceGpu {
    _history: [wgpu::Texture; 2],
    _moments: [wgpu::Texture; 2],
    _guides: [wgpu::Texture; 2],
    _temporal: wgpu::Texture,
    _raw: wgpu::Texture,
    _scratch: [wgpu::Texture; 2],
    temporal_groups: [wgpu::BindGroup; 2],
    spatial_groups: [[wgpu::BindGroup; 3]; 2],
    temporal_pipeline: wgpu::ComputePipeline,
    spatial_pipeline: wgpu::ComputePipeline,
    _step_uniforms: [wgpu::Buffer; 3],
    pub(super) raw: wgpu::TextureView,
    pub(super) temporal: wgpu::TextureView,
}

pub(super) fn receiver_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
    [unfilterable_texture_entry_stages(17, wgpu::ShaderStages::FRAGMENT),
     uniform_entry(18, wgpu::ShaderStages::FRAGMENT),
     unfilterable_texture_entry_stages(19, wgpu::ShaderStages::FRAGMENT),
     unfilterable_texture_entry_stages(20, wgpu::ShaderStages::FRAGMENT)]
}

pub(super) struct RtSunRadiance {
    pub(super) published: wgpu::TextureView,
    _published_texture: wgpu::Texture,
    pub(super) params: wgpu::Buffer,
    pub(super) sun_uniform: wgpu::Buffer,
    pub(super) enabled: bool,
    pub(super) history_valid: bool,
    pub(super) read: usize,
    pub(super) dims: [u32; 2],
    gpu: Option<RadianceGpu>,
}
impl RtSunRadiance {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let (t, v) = texture(device, "RT HDR sun radiance off fallback", [1, 1], wgpu::TextureFormat::Rgba16Float);
        Self {
            published: v, _published_texture: t,
            params: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RT direct radiance denoise params"),
                contents: bytemuck::bytes_of(&RadianceParams::zeroed()),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }),
            sun_uniform: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RT sun incident radiance receiver RGB"),
                contents: bytemuck::bytes_of(&[1.0f32, 1.0, 1.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }),
            enabled: false, history_valid: false, read: 0, dims: [1, 1], gpu: None,
        }
    }
    pub(super) fn disable(&mut self) {
        self.enabled = false;
        self.history_valid = false;
        // Keep allocations cached for subsequent re-enabling, but never dispatch.
    }
    pub(super) fn invalidate(&mut self) { self.history_valid = false; }
    pub(super) fn raw_view(&self) -> &wgpu::TextureView {
        self.gpu.as_ref().map_or(&self.published, |gpu| &gpu.raw)
    }
    pub(super) fn temporal_view(&self) -> &wgpu::TextureView {
        self.gpu.as_ref().map_or(&self.published, |gpu| &gpu.temporal)
    }
    pub(super) fn prepare(
        &mut self, device: &wgpu::Device, queue: &wgpu::Queue,
        dims: [u32; 2], view_proj: Mat4, eye: Vec3, previous_eye: Vec3, sun: [f32; 4],
        mode: u32, frame: u32, camera_cut: bool,
        shadow_visibility: &wgpu::TextureView, depth: &wgpu::TextureView,
        motion: &wgpu::TextureView, local_signal: bool,
    ) -> bool {
        if mode == 0 { self.disable(); return false; }
        let changed = self.dims != dims || self.gpu.is_none();
        if changed {
            self.dims = dims;
            self.read = 0;
            self.history_valid = false;
            let gpu = RadianceGpu::new(device, dims, &self.params, shadow_visibility, depth, motion, &mut self.published, &mut self._published_texture);
            self.gpu = Some(gpu);
        }
        let was_enabled = self.enabled;
        if !was_enabled || camera_cut { self.history_valid = false; }
        let uniform = RadianceParams {
            inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: eye.extend(0.0).to_array(),
            previous_eye: previous_eye.extend(0.0).to_array(),
            sun_rgb: sun,
            dimensions: [dims[0], dims[1], if mode == 1 { 1 } else { 2 }, frame],
            flags: [u32::from(self.history_valid), u32::from(local_signal), 0, 0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(&self.sun_uniform, 0, bytemuck::bytes_of(&sun));
        self.enabled = true;
        changed
    }
    pub(super) fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, mode: u32) {
        if !self.enabled || mode == 0 { return; }
        let Some(gpu) = self.gpu.as_ref() else { return; };
        let groups = [self.dims[0].div_ceil(8), self.dims[1].div_ceil(8), 1];
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("RT direct radiance temporal"), timestamp_writes: None,
            });
            pass.set_pipeline(&gpu.temporal_pipeline);
            pass.set_bind_group(0, &gpu.temporal_groups[self.read], &[]);
            pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
        }
        if mode >= 2 {
            for group in &gpu.spatial_groups[1 - self.read] {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("RT direct radiance SVGF atrous"), timestamp_writes: None,
                });
                pass.set_pipeline(&gpu.spatial_pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
            }
        }
        self.history_valid = true;
        self.read = 1 - self.read;
    }
}

impl RadianceGpu {
    #[allow(clippy::too_many_arguments)]
    fn new(device: &wgpu::Device, dims: [u32; 2], uniform: &wgpu::Buffer,
        shadow: &wgpu::TextureView, depth: &wgpu::TextureView, motion: &wgpu::TextureView,
        published_view: &mut wgpu::TextureView, published_texture: &mut wgpu::Texture) -> Self {
        use wgpu::TextureFormat::{Rgba16Float, Rgba32Float};
        let (hist0, hv0) = texture(device, "RT radiance history 0", dims, Rgba16Float);
        let (hist1, hv1) = texture(device, "RT radiance history 1", dims, Rgba16Float);
        let (mom0, mv0) = texture(device, "RT luminance moments 0", dims, Rgba16Float);
        let (mom1, mv1) = texture(device, "RT luminance moments 1", dims, Rgba16Float);
        let (guide0, gv0) = texture(device, "RT geometry guide 0", dims, Rgba32Float);
        let (guide1, gv1) = texture(device, "RT geometry guide 1", dims, Rgba32Float);
        let (temporal, temporal_view) = texture(device, "RT temporal direct sun", dims, Rgba16Float);
        let (raw, raw_view) = texture(device, "RT raw direct sun", dims, Rgba16Float);
        let (scratch0, sv0) = texture(device, "RT sun atrous 1", dims, Rgba16Float);
        let (scratch1, sv1) = texture(device, "RT sun atrous 2", dims, Rgba16Float);
        let (publish_tex, publish_view) = texture(device, "RT filtered direct sun", dims, Rgba16Float);
        *published_texture = publish_tex;
        *published_view = publish_view;

        let sampled = |b| unfilterable_texture_entry_stages(b, wgpu::ShaderStages::COMPUTE);
        let temporal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT radiance temporal layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::COMPUTE),
                sampled(1), sampled(2), sampled(3), sampled(4), sampled(5), sampled(6),
                storage(7, Rgba16Float), storage(8, Rgba16Float), storage(9, Rgba32Float),
                storage(10, Rgba16Float), storage(11, Rgba16Float), storage(12, Rgba16Float),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT radiance temporal WGSL"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../rt_radiance_temporal.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RT radiance temporal pipeline layout"), bind_group_layouts: &[Some(&temporal_layout)], immediate_size: 0,
        });
        let temporal_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RT sun HDR temporal"), layout: Some(&layout), module: &shader,
            entry_point: Some("temporal"), compilation_options: Default::default(), cache: None,
        });
        let histories = [&hv0, &hv1];
        let moments = [&mv0, &mv1];
        let guides = [&gv0, &gv1];
        let temporal_groups = std::array::from_fn(|read: usize| {
            let write = 1 - read;
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT sun temporal ping pong"), layout: &temporal_layout,
                entries: &[
                    bind_buffer(0, uniform), bind_texture(1, shadow), bind_texture(2, depth),
                    bind_texture(3, motion), bind_texture(4, histories[read]),
                    bind_texture(5, moments[read]), bind_texture(6, guides[read]),
                    bind_texture(7, histories[write]), bind_texture(8, moments[write]),
                    bind_texture(9, guides[write]), bind_texture(10, &temporal_view),
                    bind_texture(11, published_view), bind_texture(12, &raw_view),
                ],
            })
        });
        let spatial_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT radiance atrous layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::COMPUTE), sampled(1), sampled(2), sampled(3), storage(4, Rgba16Float)],
        });
        let spatial_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT radiance SVGF spatial WGSL"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../rt_radiance_atrous.wgsl").into()),
        });
        let spatial_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RT radiance SVGF layout"), bind_group_layouts: &[Some(&spatial_layout)], immediate_size: 0,
        });
        let spatial_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RT sun variance-guided atrous"), layout: Some(&spatial_pipeline_layout),
            module: &spatial_module, entry_point: Some("atrous"), compilation_options: Default::default(), cache: None,
        });
        let _step_uniforms: [wgpu::Buffer; 3] = std::array::from_fn(|index| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RT atrous stride"),
                contents: bytemuck::bytes_of(&FilterParams { dimensions: [dims[0], dims[1], 1 << index, 0] }),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        let spatial_groups = std::array::from_fn(|write: usize| {
            std::array::from_fn(|index: usize| {
                let source = match index { 0 => &temporal_view, 1 => &sv0, _ => &sv1 };
                let target = match index { 0 => &sv0, 1 => &sv1, _ => published_view };
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("RT direct radiance a trous"), layout: &spatial_layout,
                    entries: &[bind_buffer(0, &_step_uniforms[index]), bind_texture(1, source),
                        bind_texture(2, moments[write]), bind_texture(3, guides[write]), bind_texture(4, target)],
                })
            })
        });
        Self {
            _history: [hist0, hist1], _moments: [mom0, mom1], _guides: [guide0, guide1],
            _temporal: temporal, _raw: raw, _scratch: [scratch0, scratch1],
            temporal_groups, spatial_groups, temporal_pipeline, spatial_pipeline,
            _step_uniforms, raw: raw_view, temporal: temporal_view,
        }
    }
}

//! Screen-space water optics. Capture precedes ocean and debug draws; never
//! sample the live color/depth attachments. See Development Docs/Ocean optics audit.md.
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub fn linear_color(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) })
}

mod cull;
pub use cull::{submerged_cull_radius, WaterCull};

/// Absorption distances of water before culling may remove anything. Blue light
/// is 5% transmitted after 1 absorption distance, 0.2% after 2, 0.007% after 3.
pub const MIN_CULL_MULTIPLE: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub fog_color: [f32; 3],
    pub transparency: f32,
    pub depth_darkening: f32,
    pub refraction: f32,
    pub caustics: f32,
    /// Water-hidden geometry cull, as a multiple of the absorption distance: the
    /// distance past which a submerged view sees nothing, and the water depth that
    /// hides a batch from above. Zero disables. See `cull.rs`.
    pub underwater_cull: f32,
    /// Skip the whole capture when the camera is dry and no ocean clipmap
    /// survived this frame's culling. The capture is read only by the ocean
    /// surface shader (bsp.wgsl) and the submerged composite, so with neither
    /// present its result cannot be observed.
    pub cull: bool,
    /// Restrict the capture to the screen rectangle the visible water covers
    /// instead of the whole framebuffer. The surface shader samples the capture
    /// at its own fragment plus a refraction offset bounded by `refraction`, so
    /// a padded water rect contains every read.
    pub scissor: bool,
    /// Honour each water volume's outline mask in the capture. Off makes every
    /// volume a plain box, which is visually wrong on hollow water (an island
    /// ring's centre counts as submerged) but isolates the cost of
    /// `add_masked_spans`: that runs per pixel, per volume hit, looping every
    /// mask triangle while maintaining a dynamically indexed local array.
    /// Purely a measurement lever - no shader change, the mask slot simply
    /// stops being written into the uniform.
    pub masks: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self { fog_color: [20.0 / 255.0, 40.0 / 255.0, 50.0 / 255.0], transparency: 2304.0,
            depth_darkening: 1.0, refraction: 0.1, caustics: 1.0, underwater_cull: 3.0,
            // The scissor is off by default: measured on a real map, a promoted
            // surface's bounds surround the camera, so there is no tight screen
            // rectangle to find and it falls back to full-screen anyway. Kept
            // only as an A/B lever.
            cull: true, scissor: false, masks: true }
    }
}
impl Settings {
    pub fn sanitize(mut self) -> Self {
        fn valid(x: f32, fallback: f32, lo: f32, hi: f32) -> f32 {
            if x.is_finite() { x.clamp(lo, hi) } else { fallback }
        }
        let d = Self::default();
        for i in 0..3 { self.fog_color[i] = valid(self.fog_color[i], d.fog_color[i], 0.0, 1.0); }
        self.transparency = valid(self.transparency, d.transparency, 1.0, 65536.0);
        self.depth_darkening = valid(self.depth_darkening, d.depth_darkening, 0.01, 32.0);
        self.refraction = valid(self.refraction, d.refraction, 0.0, 0.15);
        self.caustics = valid(self.caustics, d.caustics, 0.0, 8.0);
        self.underwater_cull = valid(self.underwater_cull, d.underwater_cull, 0.0, 32.0);
        self
    }
    pub fn absorption_distance(self) -> f32 { self.transparency }
    /// Distance (in water) past which geometry is culled, or 0 when disabled. The
    /// multiple is floored at `MIN_CULL_MULTIPLE`: below it the cull would remove
    /// geometry that is still plainly visible through the water.
    pub fn cull_distance(self) -> f32 {
        if self.underwater_cull <= 0.0 { 0.0 }
        else { self.absorption_distance() * self.underwater_cull.max(MIN_CULL_MULTIPLE) }
    }
}

/// Render-only ocean continuation. No finite XZ bound or added coastal volume.
/// Every segment represents one sky-contact half-strip; the four corner bits
/// admit quadrants only when both adjoining contacted sky edges meet there.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct SkyFogRegion {
    pub bounds: [f32; 4],
    pub plane: [f32; 4], // top Y, segment count, corner bitmask, enabled
    pub segments: [[f32; 4]; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniform {
    pub inverse_view_projection: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub forward: [f32; 4],
    pub fog: [f32; 4], // linear color, absorption distance
    pub params: [f32; 4], // darkening distance, refraction, caustics, volume count
    pub minimum: [[f32; 4]; 8],
    pub maximum: [[f32; 4]; 8],
    // Only populated for r_oceanSkyExtend and qualifying map sky contacts.
    pub sky_regions: [SkyFogRegion; 8],
    pub sky_info: [f32; 4], // active continuation count; zero for OFF
}

pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    make_layout(device, false, false)
}
fn make_layout(device: &wgpu::Device, capture: bool, msaa: bool) -> wgpu::BindGroupLayout {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None,
    };
    let mut entries = vec![
        entry(0, wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: !(capture && msaa) },
            view_dimension: wgpu::TextureViewDimension::D2, multisampled: capture && msaa,
        }),
        entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        entry(2, wgpu::BindingType::Texture { sample_type: if capture { wgpu::TextureSampleType::Depth } else { wgpu::TextureSampleType::Float { filterable: false } }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: capture && msaa }),
        entry(3, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
    ];
    if !capture {
        entries.push(entry(4, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }));
        for binding in [5, 6, 7] {
            entries.push(entry(binding, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }));
        }
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Ocean optics capture/sampling layout"),
        entries: &entries,
    })
}

pub struct Resources {
    pub key: (u32, u32, u32, wgpu::TextureFormat),
    pub bind_group: wgpu::BindGroup,
    pub uniform: wgpu::Buffer,
    pub glass_uniform: wgpu::Buffer,
    pub edges: crate::glass::edges::Resources,
    capture_color: wgpu::TextureView,
    background: wgpu::TextureView,
    background_ready: bool,
    color: wgpu::TextureView,
    background_capture: wgpu::RenderPipeline,
    blur_first: wgpu::RenderPipeline,
    blur_next: wgpu::RenderPipeline,
    blur_groups: Vec<wgpu::BindGroup>,
    blur_layout: wgpu::BindGroupLayout,
    blur_inputs: Vec<wgpu::TextureView>,
    blur_outputs: Vec<wgpu::TextureView>,
    capture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    depth: wgpu::TextureView,
    capture: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
}
impl Resources {
    pub fn new(device: &wgpu::Device, output_layout: &wgpu::BindGroupLayout,
        ocean_layout: &wgpu::BindGroupLayout, key: (u32, u32, u32, wgpu::TextureFormat)) -> Self {
        let (width, height, samples, format) = key;
        let target = |format, label| device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default());
        // Keep premultiplied blur validity precise near excluded foregrounds.
        let optical_format = wgpu::TextureFormat::Rgba16Float;
        let mip_count = (width.max(height).ilog2() + 1).min(7);
        let color_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shared optical scene / Gaussian blur pyramid"), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: mip_count, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: optical_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let color = color_texture.create_view(&Default::default());
        let mip_view = |mip| color_texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: mip, mip_level_count: Some(1), ..Default::default() });
        let capture_color = mip_view(0);
        let depth = target(wgpu::TextureFormat::R32Float, "Ocean pre-surface linear depth");
        let background = background_target(device, 1, 1);
        let edges = crate::glass::edges::Resources::new(device, width, height, samples, &depth);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Ocean clamp sampler"), mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Ocean optical volume uniform"), contents: bytemuck::bytes_of(&Uniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let glass_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Live glass settings"),
            contents: bytemuck::bytes_of(&crate::glass::settings().uniform([0.1; 3])),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = bind(device, output_layout, &color, &depth, &sampler, &uniform, Some((&glass_uniform, &edges.field, &edges.mask, &background)));
        let capture_layout = make_layout(device, true, samples > 1);
        let capture_source = shader_source(samples > 1);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ocean optical capture"), source: wgpu::ShaderSource::Wgsl(capture_source.into()),
        });
        let capture = pipeline(device, &shader, &[&capture_layout, ocean_layout], "fs_capture", &[optical_format, wgpu::TextureFormat::R32Float], 1);
        let background_capture = pipeline(device, &shader, &[&capture_layout, ocean_layout], "fs_glass_background", &[optical_format], 1);
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ocean optical composite"), source: wgpu::ShaderSource::Wgsl(include_str!("../ocean_composite.wgsl").into()),
        });
        let blit = pipeline(device, &blit_shader, &[output_layout], "fs_main", &[format], samples);
        let texture_entry = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
            }, count: None,
        };
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("Glass Gaussian pyramid inputs"), entries: &[
            texture_entry(0, true), wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            texture_entry(2, false), texture_entry(3, false), texture_entry(4, false),
            wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ] });
        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Foreground-safe glass blur"), source: wgpu::ShaderSource::Wgsl(include_str!("../glass_blur.wgsl").into()) });
        let blur_first = pipeline(device, &blur_shader, &[&blur_layout], "fs_first", &[optical_format], 1);
        let blur_next = pipeline(device, &blur_shader, &[&blur_layout], "fs_next", &[optical_format], 1);
        let mut blur_groups = Vec::new();
        let mut blur_inputs = Vec::new();
        let mut blur_outputs = Vec::new();
        for mip in 1..mip_count {
            let input = mip_view(mip-1);
            blur_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("Glass Gaussian pyramid level"), layout: &blur_layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&input) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&depth) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&edges.mask) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&background) },
                wgpu::BindGroupEntry { binding: 5, resource: glass_uniform.as_entire_binding() },
            ] }));
            blur_inputs.push(input);
            blur_outputs.push(mip_view(mip));
        }
        Self { key, bind_group, uniform, glass_uniform, edges, capture_color, background,
            background_ready: false, color, background_capture, blur_first, blur_next, blur_groups,
            blur_layout, blur_inputs, blur_outputs, capture_layout, sampler, depth, capture, blit }
    }

    /// Disabled recovery keeps only a 1x1 binding placeholder. Toggle allocation
    /// and bindings without rebuilding shaders or resetting optical uniforms.
    pub fn configure_background(&mut self, device: &wgpu::Device, output_layout: &wgpu::BindGroupLayout, enabled: bool) {
        if self.background_ready == enabled { return; }
        self.background = if enabled { background_target(device, self.key.0, self.key.1) }
            else { background_target(device, 1, 1) };
        self.background_ready = enabled;
        self.bind_group = bind(device, output_layout, &self.color, &self.depth, &self.sampler,
            &self.uniform, Some((&self.glass_uniform, &self.edges.field, &self.edges.mask, &self.background)));
        self.blur_groups = self.blur_inputs.iter().map(|input| device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Glass Gaussian pyramid recovery bindings"), layout: &self.blur_layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(input) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.depth) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.edges.mask) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&self.background) },
                wgpu::BindGroupEntry { binding: 5, resource: self.glass_uniform.as_entire_binding() },
            ],
        })).collect();
    }

    /// Save the world before opaque models obscure it. Model-behind-glass color
    /// still comes from the later full capture; only foreground holes use this.
    pub fn capture_background(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::TextureView, depth: &wgpu::TextureView, ocean: &wgpu::BindGroup) {
        let input = bind(device, &self.capture_layout, scene, depth, &self.sampler, &self.uniform, None);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Glass background before opaque models"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.background, depth_slice: None, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })], ..Default::default()
        });
        pass.set_pipeline(&self.background_capture);
        pass.set_bind_group(0, &input, &[]);
        pass.set_bind_group(1, ocean, &[]);
        pass.draw(0..3, 0..1);
    }

    /// `scissor` is `(x, y, width, height)` in pixels, or `None` for the whole
    /// target. The attachments are still cleared in full (a clear is cheap and a
    /// pass-level clear ignores the scissor anyway); the scissor only keeps the
    /// expensive per-pixel volume/mask work inside the rectangle that something
    /// will actually sample.
    pub fn capture(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::TextureView, depth: &wgpu::TextureView, ocean: &wgpu::BindGroup,
        scissor: Option<(u32, u32, u32, u32)>) {
        let input = bind(device, &self.capture_layout, scene, depth, &self.sampler, &self.uniform, None);
        let attachments = [&self.capture_color, &self.depth].map(|view| Some(wgpu::RenderPassColorAttachment {
            view, depth_slice: None, resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        }));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Ocean optical capture"), color_attachments: &attachments, ..Default::default()
        });
        pass.set_pipeline(&self.capture);
        pass.set_bind_group(0, &input, &[]);
        pass.set_bind_group(1, ocean, &[]);
        if let Some((x, y, width, height)) = scissor {
            if width == 0 || height == 0 {
                return;
            }
            pass.set_scissor_rect(x, y, width, height);
        }
        pass.draw(0..3, 0..1);
    }
    pub fn blur(&self, encoder: &mut wgpu::CommandEncoder) {
        for (index, (group, output)) in self.blur_groups.iter().zip(&self.blur_outputs).enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Glass Gaussian pyramid"), color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })], ..Default::default()
            });
            pass.set_pipeline(if index == 0 { &self.blur_first } else { &self.blur_next });
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    pub fn composite(&self, encoder: &mut wgpu::CommandEncoder, color: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Ocean optical composite"), color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color, depth_slice: None, resolve_target: resolve,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })], ..Default::default()
        });
        pass.set_pipeline(&self.blit);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
fn background_target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Optional glass world background"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    }).create_view(&Default::default())
}
fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, color: &wgpu::TextureView,
    depth: &wgpu::TextureView, sampler: &wgpu::Sampler, uniform: &wgpu::Buffer, glass: Option<(&wgpu::Buffer, &wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)>) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(color) },
        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
        wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
    ];
    if let Some((uniform, field, mask, background)) = glass {
        entries.push(wgpu::BindGroupEntry { binding: 4, resource: uniform.as_entire_binding() });
        entries.push(wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(field) });
        entries.push(wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(mask) });
        entries.push(wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(background) });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("Ocean optics bindings"), layout,
        entries: &entries,
    })
}
fn pipeline(device: &wgpu::Device, shader: &wgpu::ShaderModule, layouts: &[&wgpu::BindGroupLayout], entry: &str,
    formats: &[wgpu::TextureFormat], samples: u32) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Ocean optics pipeline layout"), bind_group_layouts: &layouts.iter().map(|l| Some(*l)).collect::<Vec<_>>(), immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry), layout: Some(&layout),
        vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(), depth_stencil: None,
        multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
        fragment: Some(wgpu::FragmentState { module: shader, entry_point: Some(entry), compilation_options: Default::default(),
            targets: &formats.iter().map(|&format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })).collect::<Vec<_>>() }),
        multiview_mask: None, cache: crate::pipeline_cache::get().as_ref(),
    })
}

pub fn shader_source(msaa: bool) -> String {
        let bsp = include_str!("../bsp.wgsl");
        let ocean_struct = &bsp[bsp.find("struct OceanRenderSettings {").unwrap()..];
        let ocean_struct = &ocean_struct[..ocean_struct.find("};").unwrap() + 2];
        let source = include_str!("../ocean_optics.wgsl").replace("// OCEAN_STRUCT", ocean_struct);
        let capture_source = source
            .replace("COLOR_DECLARATION", if msaa {
                "@group(0) @binding(0) var scene_color: texture_multisampled_2d<f32>;"
            } else {
                "@group(0) @binding(0) var scene_color: texture_2d<f32>;"
            })
            .replace("DEPTH_DECLARATION", if msaa {
                "@group(0) @binding(2) var scene_depth: texture_depth_multisampled_2d;"
            } else {
                "@group(0) @binding(2) var scene_depth: texture_depth_2d;"
            })
            .replace("CAPTURE_BODY", if msaa {
                r#"let sample_count = textureNumSamples(scene_depth);
    let first_depth = textureLoad(scene_depth, pixel, 0);
    var min_depth = first_depth;
    var max_depth = first_depth;
    for (var i = 1u; i < sample_count; i += 1u) {
        let d = textureLoad(scene_depth, pixel, i32(i));
        min_depth = min(min_depth, d);
        max_depth = max(max_depth, d);
    }
    // Interior pixels take the old one-shade fast path. Only MSAA depth
    // discontinuities need sample-matched optics; this keeps the full-screen
    // water pass close to its previous cost while fixing silhouette halos.
    if (max_depth - min_depth <= 0.00001) {
        return capture_sample(uv, first_depth, textureLoad(scene_color, pixel, 0).rgb);
    }
    var out: Capture;
    out.color = vec4<f32>(0.0);
    out.depth = 1000000.0;
    for (var i = 0u; i < sample_count; i += 1u) {
        let sample = capture_sample(
            uv,
            textureLoad(scene_depth, pixel, i32(i)),
            textureLoad(scene_color, pixel, i32(i)).rgb,
        );
        out.color += sample.color;
        out.depth = min(out.depth, sample.depth);
    }
    out.color = vec4<f32>(out.color.rgb / f32(sample_count), 1.0);
    return out;"#
            } else {
                r#"return capture_sample(
        uv,
        textureLoad(scene_depth, pixel, 0),
        textureLoad(scene_color, pixel, 0).rgb,
    );"#
            });
        capture_source
}

#[cfg(test)]
#[path = "optics/boundary_tests.rs"]
mod boundary_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optics_shaders_validate() {
        for source in [shader_source(false), shader_source(true), include_str!("../ocean_composite.wgsl").to_owned()] {
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        }
        assert_eq!(std::mem::size_of::<Uniform>(), 384);
    }

    #[test]
    fn invalid_settings_are_finite_and_bounded() {
        let settings = Settings { transparency: -1.0, refraction: f32::INFINITY, ..Default::default() }.sanitize();
        assert_eq!(settings.transparency, 1.0);
        assert_eq!(settings.refraction, 0.1);
    }

    #[test]
    fn cull_distance_follows_transparency_with_a_visibility_floor() {
        let s = Settings { transparency: 2000.0, underwater_cull: 3.0, ..Default::default() };
        assert_eq!(s.cull_distance(), 6000.0);
        // Below 2 absorption distances geometry is still visible: clamp up.
        assert_eq!(Settings { underwater_cull: 0.07, ..s }.cull_distance(), 4000.0);
        assert_eq!(Settings { underwater_cull: 0.0, ..s }.cull_distance(), 0.0);
        // Murkier water culls sooner.
        assert_eq!(Settings { transparency: 500.0, ..s }.cull_distance(), 1500.0);
    }

    #[test]
    #[ignore = "requires a GPU adapter; run explicitly"]
    fn optics_gpu_capture_matches_beer_lambert() {
        futures_lite::future::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance.request_adapter(&Default::default()).await.expect("GPU adapter");
            let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: adapter.limits(), ..Default::default()
            }).await.expect("GPU device");
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let ocean_layout = crate::ocean::OceanGpu::create_render_layout(&device);
            let ocean = crate::ocean::OceanGpu::create_inert_render_bind_group(&device, &ocean_layout);
            let layout = layout(&device);
            let extent = wgpu::Extent3d { width: 8, height: 8, depth_or_array_layers: 1 };
            for samples in [1, 4] {
                let resources = Resources::new(&device, &layout, &ocean_layout, (8, 8, samples, wgpu::TextureFormat::Rgba8Unorm));
                let texture = |format, samples, usage| device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Optics GPU test target"), size: extent, mip_level_count: 1, sample_count: samples,
                    dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[],
                });
                let color = texture(wgpu::TextureFormat::Rgba8Unorm, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
                let color_view = color.create_view(&Default::default());
                let msaa = texture(
                    wgpu::TextureFormat::Rgba8Unorm,
                    samples,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                );
                let msaa_view = msaa.create_view(&Default::default());
                let depth = texture(wgpu::TextureFormat::Depth32Float, samples, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
                let depth_view = depth.create_view(&Default::default());
                let output = if samples == 1 { &color_view } else { &msaa_view };
                let resolve = (samples > 1).then_some(&color_view);
                for (volumes, surface_z, fraction) in [(0, 10.0, 0.0), (1, 10.0, 1.0), (2, 10.0, 1.0), (1, 0.25, 0.5)] {
                    let mut uniform = Uniform::zeroed();
                    uniform.inverse_view_projection = glam::Mat4::IDENTITY.to_cols_array_2d();
                    uniform.forward = [0.0,0.0,1.0,0.0];
                    uniform.fog[3] = 20.0;
                    uniform.params = [20.0,0.0,0.0,volumes as f32];
                    for i in 0..volumes {
                        uniform.minimum[i] = [-10.0,-10.0,-10.0,0.0];
                        uniform.maximum[i] = [10.0,10.0,surface_z,0.0];
                    }
                    queue.write_buffer(&resources.uniform, 0, bytemuck::bytes_of(&uniform));
                    let mut encoder = device.create_command_encoder(&Default::default());
                    {
                        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: output, depth_slice: None, resolve_target: resolve,
                                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
                            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &depth_view,
                                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.5), store: wgpu::StoreOp::Store }), stencil_ops: None }), ..Default::default()
                        });
                    }
                    let capture_color = if samples == 1 { &color_view } else { &msaa_view };
                    resources.capture(&device, &mut encoder, capture_color, &depth_view, &ocean, None);
                    resources.composite(&mut encoder, output, resolve);
                    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 256*8,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
                    encoder.copy_texture_to_buffer(color.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(8) } }, extent);
                    queue.submit([encoder.finish()]);
                    readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
                    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                    let bytes = readback.slice(..).get_mapped_range().expect("GPU readback range");
                    let distance = (0.125_f32.powi(2)*2.0 + 0.5_f32.powi(2)).sqrt() * fraction;
                    for (channel, ratio) in [38.0/7.5, 38.0/22.0, 1.0].iter().enumerate() {
                        let expected = (-4.321928 * ratio * distance / 20.0).exp2();
                        let actual = bytes[4*256+4*4+channel] as f32 / 255.0;
                        assert!((expected-actual).abs() < 0.008, "samples={samples} volumes={volumes} channel={channel}: {actual} != {expected}");
                    }
                }
            }
            assert!(scope.pop().await.is_none(), "GPU validation error");
        });
    }
}

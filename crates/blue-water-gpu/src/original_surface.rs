//! Pipeline layout for the preserved Blue Water surface shader.
//! Bind groups 0, 1 and 3 are supplied by the JKA render bridge; group 2 is
//! `SurfaceWaves`. The upstream files stay unchanged; the JKA variant adds a
//! fragment admission test for the host's footprint and PVS boundaries.
use crate::{geometry, shader_composer, surface_waves::SurfaceWaves};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

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

fn storage(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn sampled(
    binding: u32,
    visibility: wgpu::ShaderStages,
    dimension: wgpu::TextureViewDimension,
    depth: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: if depth {
                wgpu::TextureSampleType::Depth
            } else {
                wgpu::TextureSampleType::Float { filterable: true }
            },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler(
    binding: u32,
    visibility: wgpu::ShaderStages,
    comparison: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(if comparison {
            wgpu::SamplerBindingType::Comparison
        } else {
            wgpu::SamplerBindingType::Filtering
        }),
        count: None,
    }
}

pub struct OriginalSurfacePipeline {
    pub camera_layout: wgpu::BindGroupLayout,
    pub surface_layout: wgpu::BindGroupLayout,
    pub environment_layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
    pub wireframe: Option<wgpu::RenderPipeline>,
    // Keep the existing compiled vertex entry to compile the debug overlay
    // lazily; no wireframe PSO when r_showtris ocean is disabled.
    vertex_shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
}

/// Preserved High-tier clipmap vertex and index data. JKA finite pools omit
/// the upstream horizon/skirt: its exterior belongs to JKA's sky extension
/// gate, not to an unbounded sea in every BSP room.
pub struct OriginalClipmap {
    clipmap: geometry::Clipmap,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    fills: Vec<std::ops::Range<u32>>,
    rings: Vec<u32>,
    horizon_start: u32,
    index_count: u32,
}

/// Material values in the original surface uniform. Colors are linear RGB.
/// Scene-specific colors can be supplied by JKA's promoted water appearance.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceAppearance {
    pub absorption: [f32; 3],
    pub transmission: [f32; 3],
    pub pigment: [f32; 3],
    pub crest_foam: [f32; 4],
    pub surface_foam: [f32; 4],
    pub shoreline_foam: [f32; 4],
    pub wind_stretch: f32,
    pub windrow_coverage: f32,
    pub foam_streak_mean: f32,
    pub rain: f32,
}

impl Default for SurfaceAppearance {
    fn default() -> Self {
        // Original calibration::NORTH_ATLANTIC / scene_parameters, linear RGB.
        // The temporary bridge's cyan tint and clear-blue coefficients were
        // not the upstream optical calibration, especially below the surface.
        let linear = |hex: u32| [16, 8, 0].map(|shift| {
            let srgb = ((hex >> shift) & 255) as f32 / 255.0;
            if srgb <= 0.04045 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) }
        });
        Self {
            absorption: linear(0x945b57),
            transmission: linear(0x49575e),
            pigment: linear(0x2d373c),
            crest_foam: [1.0, 1.0, 1.0, 1.0],
            surface_foam: [1.0, 1.0, 1.0, 0.6],
            shoreline_foam: [0.85, 0.94, 0.98, 0.8],
            wind_stretch: 0.5,
            windrow_coverage: 0.2,
            foam_streak_mean: 0.5,
            rain: 0.0,
        }
    }
}

impl OriginalClipmap {
    pub fn new(device: &wgpu::Device, segments: u32, levels: usize) -> Self {
        let clipmap = geometry::Clipmap::new(segments, levels);
        let mesh = geometry::build(clipmap, 512.0);
        let outer_ring_cells = clipmap.segments * clipmap.segments * 3 / 4;
        let horizon_start = mesh.rings[clipmap.levels - 1] + outer_ring_cells * 6;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blue Water original clipmap vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blue Water original clipmap indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            clipmap,
            vertices,
            indices,
            fills: mesh.fills,
            rings: mesh.rings,
            horizon_start,
            index_count: mesh.indices.len() as u32,
        }
    }

    pub fn clipmap(&self) -> geometry::Clipmap {
        self.clipmap
    }

    /// Draw the finest needed middle and rings through `outer_level`. The
    /// caller must also bind the exact JKA footprint/PVS fragment mask.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, first: usize, outer_level: usize) {
        let first = first.min(self.clipmap.levels - 1);
        let last = outer_level.max(first).min(self.clipmap.levels - 1);
        let end = self
            .rings
            .get(last + 1)
            .copied()
            .unwrap_or(self.horizon_start);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(self.fills[first].clone(), 0, 0..1);
        pass.draw_indexed(self.rings[first]..end, 0, 0..1);
    }

    pub fn triangle_count(&self, first: usize, outer_level: usize) -> u64 {
        let first = first.min(self.clipmap.levels - 1);
        let last = outer_level.max(first).min(self.clipmap.levels - 1);
        let end = self
            .rings
            .get(last + 1)
            .copied()
            .unwrap_or(self.horizon_start);
        u64::from(self.fills[first].end - self.fills[first].start + end - self.rings[first]) / 3
    }

    /// The original horizon and skirt are admitted only for a pool whose JKA
    /// sky contacts permit continuation. Their fragments use the same boundary.
    pub fn draw_horizon<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, first: usize) {
        let first = first.min(self.clipmap.levels - 1);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(self.fills[first].clone(), 0, 0..1);
        pass.draw_indexed(self.rings[first]..self.index_count, 0, 0..1);
    }

    pub fn horizon_triangle_count(&self, first: usize) -> u64 {
        let first = first.min(self.clipmap.levels - 1);
        u64::from(
            self.fills[first].end - self.fills[first].start + self.index_count - self.rings[first],
        ) / 3
    }

    pub fn outer_level_for_radius(&self, radius_metres: f64) -> usize {
        (0..self.clipmap.levels)
            .find(|&level| self.clipmap.half(level) >= radius_metres)
            .unwrap_or(self.clipmap.levels - 1)
    }

    /// The first 25 vec4s of the upstream `Surface` block. The point-light
    /// and geographic water-field tail remains zero when those effects are off.
    pub fn surface_prefix(
        &self,
        centre: [f64; 2],
        standoff_metres: f64,
        appearance: SurfaceAppearance,
    ) -> [[f32; 4]; 25] {
        let mut values = [[0.0; 4]; 25];
        values[..10].copy_from_slice(&self.clipmap.frames(centre[0], centre[1]));
        values[10] = [
            self.clipmap.segments as f32,
            self.clipmap.levels as f32,
            centre[0] as f32,
            centre[1] as f32,
        ];
        let detail = self.clipmap.detail(standoff_metres);
        values[11] = [detail.first as f32, detail.morph, 0.0, 0.0];
        values[13] = [
            appearance.absorption[0],
            appearance.absorption[1],
            appearance.absorption[2],
            0.0,
        ];
        values[14] = [
            appearance.transmission[0],
            appearance.transmission[1],
            appearance.transmission[2],
            0.0,
        ];
        values[15] = appearance.crest_foam;
        values[16] = appearance.surface_foam;
        values[17] = appearance.shoreline_foam;
        values[18] = [
            appearance.wind_stretch,
            appearance.windrow_coverage,
            appearance.foam_streak_mean,
            0.0,
        ];
        // JKA applies its own scene fog after this pass.
        values[23] = [
            appearance.pigment[0],
            appearance.pigment[1],
            appearance.pigment[2],
            0.0,
        ];
        values[24][0] = appearance.rain.clamp(0.0, 1.0);
        values
    }
}

/// `naval::camera::Camera` in the preserved camera.wgsl. JKA supplies scaled
/// metre-space matrices; its ordinary world and depth attachments stay in
/// native map units.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SurfaceCamera {
    pub view: [[f32; 4]; 4],
    pub projection: [[f32; 4]; 4],
    pub view_projection: [[f32; 4]; 4],
    pub inverse_view: [[f32; 4]; 4],
    pub inverse_projection: [[f32; 4]; 4],
    pub inverse_view_projection: [[f32; 4]; 4],
    pub position_time: [f32; 4],
    pub viewport: [f32; 4],
    pub near: f32,
    pub far: f32,
    pub tan_half_fov_y: f32,
    pub aspect: f32,
    pub dt: f32,
    pub frame: u32,
    pub curvature: f32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SurfaceBoundary {
    pub bounds: [f32; 4],
    pub counts: [u32; 4],
    pub scale: [f32; 4],
    pub tiles: [u32; 4],
    pub pvs: [[u32; 4]; 8],
    pub triangles: [[f32; 4]; 512],
    pub sky: [[f32; 4]; 16],
    /// Eight 32-bit candidate masks per 8x8 tile; exact edge tests still decide coverage.
    pub candidates: [[u32; 4]; 128],
}

impl SurfaceBoundary {
    pub fn prepare_candidates(&mut self) {
        self.candidates = [[0; 4]; 128];
        let width = (self.bounds[2] - self.bounds[0]).max(0.0);
        let height = (self.bounds[3] - self.bounds[1]).max(0.0);
        let tolerance = self.scale[1];
        for tile in 0..64 {
            let x = tile % 8;
            let y = tile / 8;
            let lo = [self.bounds[0] + width * x as f32 / 8.0,
                self.bounds[1] + height * y as f32 / 8.0];
            let hi = [self.bounds[0] + width * (x + 1) as f32 / 8.0,
                self.bounds[1] + height * (y + 1) as f32 / 8.0];
            for index in 0..(self.counts[0] as usize).min(256) {
                let ab = self.triangles[index * 2];
                let c = self.triangles[index * 2 + 1];
                let minimum = [ab[0].min(ab[2]).min(c[0]), ab[1].min(ab[3]).min(c[1])];
                let maximum = [ab[0].max(ab[2]).max(c[0]), ab[1].max(ab[3]).max(c[1])];
                if maximum[0] + tolerance >= lo[0] && minimum[0] - tolerance <= hi[0]
                    && maximum[1] + tolerance >= lo[1] && minimum[1] - tolerance <= hi[1] {
                    let word = tile * 8 + index / 32;
                    self.candidates[word / 4][word % 4] |= 1 << (index % 32);
                }
            }
        }
    }
}

fn buffer(device: &wgpu::Device, label: &'static str, size: u64, storage: bool) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: (if storage {
            wgpu::BufferUsages::STORAGE
        } else {
            wgpu::BufferUsages::UNIFORM
        }) | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn placeholder(
    device: &wgpu::Device,
    label: &'static str,
    format: wgpu::TextureFormat,
    dimension: wgpu::TextureViewDimension,
    layers: u32,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: if dimension == wgpu::TextureViewDimension::D3 {
            wgpu::TextureDimension::D3
        } else {
            wgpu::TextureDimension::D2
        },
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(dimension),
        ..Default::default()
    })
}

fn size_of_binding(module: &wgpu::naga::Module, group: u32, binding: u32) -> u64 {
    let (_, global) = module
        .global_variables
        .iter()
        .find(|(_, global)| {
            global
                .binding
                .as_ref()
                .is_some_and(|at| at.group == group && at.binding == binding)
        })
        .unwrap_or_else(|| panic!("Blue Water original surface binding {group}:{binding}"));
    match &module.types[global.ty].inner {
        wgpu::naga::TypeInner::Struct { span, .. } => u64::from(*span),
        _ => panic!("Blue Water original surface uniform type"),
    }
}

fn entry_buffer(binding: u32, value: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: value.as_entire_binding(),
    }
}

fn entry_texture(binding: u32, value: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(value),
    }
}

fn entry_sampler(binding: u32, value: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::Sampler(value),
    }
}

/// Live bindings for the original material. Neutral shadow, wake, cloud and
/// probe resources keep their corresponding upstream branches disabled until
/// JKA has supplied them. Scene copies can be rebound after resize/capture.
pub struct OriginalSurfaceBindings {
    pub camera_group: wgpu::BindGroup,
    pub material_group: wgpu::BindGroup,
    pub environment_group: wgpu::BindGroup,
    pub(crate) camera: wgpu::Buffer,
    camera_aux: CameraAux,
    current_aerial: Option<wgpu::TextureView>,
    current_cloud: Option<wgpu::TextureView>,
    current_scene: Option<(wgpu::TextureView, wgpu::TextureView)>,
    neutral_cloud: (wgpu::TextureView, wgpu::Sampler, wgpu::Buffer),
    surface: wgpu::Buffer,
    environment: wgpu::Buffer,
    ambient_atlas: wgpu::Texture,
    ambient: [f32; 3],
    foam: crate::foam_texture::FoamMap,
    pub(crate) calm_wake: wgpu::Buffer,
    pub(crate) calm_view: wgpu::TextureView,
    pub(crate) wake_sampler: wgpu::Sampler,
    cloud_view: wgpu::TextureView,
    cloud_sampler: wgpu::Sampler,
    cloud_uniform: wgpu::Buffer,
    pub(crate) probe_view: wgpu::TextureView,
    reflection_table: wgpu::Buffer,
    pub(crate) boundary: wgpu::Buffer,
    pub surface_bytes: usize,
}

struct CameraAux {
    shadows: wgpu::Buffer,
    motion: wgpu::Buffer,
    occluder: wgpu::Buffer,
    occluder_uniform: wgpu::Buffer,
    shadow_array: wgpu::TextureView,
    shadow_wide: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    aerial: wgpu::TextureView,
    aerial_sampler: wgpu::Sampler,
}
impl OriginalSurfaceBindings {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &OriginalSurfacePipeline,
    ) -> Self {
        Self::with_foam(
            device,
            queue,
            pipeline,
            crate::foam_texture::FoamMap::new(device, queue),
        )
    }

    pub fn with_foam(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &OriginalSurfacePipeline,
        foam: crate::foam_texture::FoamMap,
    ) -> Self {
        let mut composer = shader_composer::composer().expect("Blue Water modules");
        let module = shader_composer::surface_module(&mut composer, false)
            .expect("original Blue Water surface");
        let camera = buffer(
            device,
            "Blue Water camera",
            std::mem::size_of::<SurfaceCamera>() as u64,
            false,
        );
        let shadows = buffer(
            device,
            "Blue Water disabled shadows",
            size_of_binding(&module, 0, 1),
            false,
        );
        // The color entry point does not import motion; keep the published
        // camera-layout slot for the later motion pipeline.
        let motion = buffer(device, "Blue Water camera motion", 144, false);
        let occluder = buffer(
            device,
            "Blue Water disabled occluder data",
            256 * 256 * 8,
            true,
        );
        let occluder_uniform = buffer(
            device,
            "Blue Water disabled occluders",
            size_of_binding(&module, 0, 9),
            false,
        );
        let shadow_array = placeholder(
            device,
            "Blue Water blank shadow array",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureViewDimension::D2Array,
            7,
        );
        let shadow_wide = placeholder(
            device,
            "Blue Water blank wide shadow",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureViewDimension::D2,
            1,
        );
        let aerial = placeholder(
            device,
            "Blue Water blank aerial volume",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureViewDimension::D3,
            1,
        );
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blue Water disabled shadow comparison"),
            compare: Some(wgpu::CompareFunction::GreaterEqual),
            ..Default::default()
        });
        let aerial_sampler = device.create_sampler(&Default::default());
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water original camera"),
            layout: &pipeline.camera_layout,
            entries: &[
                entry_buffer(0, &camera),
                entry_buffer(1, &shadows),
                entry_texture(2, &shadow_array),
                entry_texture(3, &shadow_wide),
                entry_sampler(4, &shadow_sampler),
                entry_buffer(5, &motion),
                entry_texture(6, &aerial),
                entry_sampler(7, &aerial_sampler),
                entry_buffer(8, &occluder),
                entry_buffer(9, &occluder_uniform),
            ],
        });
        let surface_bytes = size_of_binding(&module, 1, 0) as usize;
        let surface = buffer(
            device,
            "Blue Water original surface parameters",
            surface_bytes as u64,
            false,
        );
        let calm_wake = buffer(
            device,
            "Blue Water calm wake",
            size_of_binding(&module, 1, 5),
            false,
        );
        let calm_view = placeholder(
            device,
            "Blue Water calm wake texture",
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureViewDimension::D2,
            1,
        );
        let wake_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let cloud_view = placeholder(
            device,
            "Blue Water disabled cloud shadow",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureViewDimension::D2,
            1,
        );
        let cloud_sampler = device.create_sampler(&Default::default());
        let cloud_uniform = buffer(
            device,
            "Blue Water disabled cloud shadow uniform",
            size_of_binding(&module, 1, 13),
            false,
        );
        let probe_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Blue Water original underwater probe"),
                size: wgpu::Extent3d {
                    width: 128,
                    height: 72,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let reflection_table = buffer(device, "Blue Water unpruned reflection table", 4096, true);
        let boundary = buffer(
            device,
            "Blue Water JKA admission boundary",
            std::mem::size_of::<SurfaceBoundary>() as u64,
            false,
        );
        let mut admission = SurfaceBoundary::zeroed();
        admission.bounds = [-1e8, -1e8, 1e8, 1e8];
        admission.scale = [1.0, 0.75, 0.0, 0.0];
        queue.write_buffer(&boundary, 0, bytemuck::bytes_of(&admission));
        let color = placeholder(
            device,
            "Blue Water blank opaque scene color",
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureViewDimension::D2,
            1,
        );
        let depth = placeholder(
            device,
            "Blue Water blank opaque scene depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureViewDimension::D2,
            1,
        );
        let material_group = Self::make_material_group(
            device,
            &pipeline.surface_layout,
            &surface,
            &foam,
            &calm_wake,
            &calm_view,
            &wake_sampler,
            &cloud_view,
            &cloud_sampler,
            &cloud_uniform,
            &probe_view,
            &reflection_table,
            &boundary,
            &color,
            &depth,
        );
        let environment = buffer(device, "Blue Water environment", 80, false);
        let atlas_size = wgpu::Extent3d {
            width: 384,
            height: 512,
            depth_or_array_layers: 1,
        };
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Blue Water temporary sky environment atlas"),
            size: atlas_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mut pixels = vec![0u8; atlas_size.width as usize * atlas_size.height as usize * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[117, 154, 186, 255]);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas_size.width * 4),
                rows_per_image: Some(atlas_size.height),
            },
            atlas_size,
        );
        let atlas_view = atlas.create_view(&Default::default());
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let environment_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water temporary environment"),
            layout: &pipeline.environment_layout,
            entries: &[
                entry_texture(0, &atlas_view),
                entry_sampler(1, &atlas_sampler),
                entry_buffer(2, &environment),
            ],
        });
        // SkyEnvironment: atlas texels, max mip, generation, light, sun and moon.
        let mut environment_values = [0.0f32; 20];
        environment_values[0] = 1.0 / atlas_size.width as f32;
        environment_values[1] = 1.0 / atlas_size.height as f32;
        environment_values[2] = 7.0;
        environment_values[5] = 1.0;
        environment_values[7] = 5.0;
        environment_values[8..11].copy_from_slice(&[1.0, 0.95, 0.85]);
        environment_values[13] = 1.0;
        queue.write_buffer(&environment, 0, bytemuck::cast_slice(&environment_values));
        Self {
            camera_group,
            material_group,
            environment_group,
            camera,
            camera_aux: CameraAux {
                shadows,
                motion,
                occluder,
                occluder_uniform,
                shadow_array,
                shadow_wide,
                shadow_sampler,
                aerial,
                aerial_sampler,
            },
            current_aerial: None,
            current_cloud: None,
            current_scene: None,
            neutral_cloud: (
                cloud_view.clone(),
                cloud_sampler.clone(),
                cloud_uniform.clone(),
            ),
            surface,
            environment,
            ambient_atlas: atlas,
            ambient: [117.0 / 255.0, 154.0 / 255.0, 186.0 / 255.0],
            foam,
            calm_wake,
            calm_view,
            wake_sampler,
            cloud_view,
            cloud_sampler,
            cloud_uniform,
            probe_view,
            reflection_table,
            boundary,
            surface_bytes,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_material_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        surface: &wgpu::Buffer,
        foam: &crate::foam_texture::FoamMap,
        calm_wake: &wgpu::Buffer,
        calm_view: &wgpu::TextureView,
        wake_sampler: &wgpu::Sampler,
        cloud_view: &wgpu::TextureView,
        cloud_sampler: &wgpu::Sampler,
        cloud_uniform: &wgpu::Buffer,
        probe_view: &wgpu::TextureView,
        reflection_table: &wgpu::Buffer,
        boundary: &wgpu::Buffer,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water original surface material"),
            layout,
            entries: &[
                entry_buffer(0, surface),
                entry_texture(1, &foam.view),
                entry_sampler(2, &foam.sampler),
                entry_texture(3, color),
                entry_texture(4, depth),
                entry_buffer(5, calm_wake),
                entry_texture(6, calm_view),
                entry_texture(7, calm_view),
                entry_sampler(8, wake_sampler),
                entry_texture(9, calm_view),
                entry_texture(10, calm_view),
                entry_texture(11, cloud_view),
                entry_sampler(12, cloud_sampler),
                entry_buffer(13, cloud_uniform),
                entry_texture(14, probe_view),
                entry_buffer(15, reflection_table),
                entry_buffer(16, boundary),
            ],
        })
    }

    pub fn set_scene(
        &mut self,
        device: &wgpu::Device,
        pipeline: &OriginalSurfacePipeline,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        let next = (color.clone(), depth.clone());
        if self.current_scene.as_ref() == Some(&next) { return; }
        self.material_group = Self::make_material_group(
            device,
            &pipeline.surface_layout,
            &self.surface,
            &self.foam,
            &self.calm_wake,
            &self.calm_view,
            &self.wake_sampler,
            &self.cloud_view,
            &self.cloud_sampler,
            &self.cloud_uniform,
            &self.probe_view,
            &self.reflection_table,
            &self.boundary,
            color,
            depth,
        );
        self.current_scene = Some(next);
    }

    /// Bind the original atmosphere's froxels and cloud shadow resources, restoring clear air when absent.
    pub fn set_atmosphere_resources(
        &mut self,
        device: &wgpu::Device,
        pipeline: &OriginalSurfacePipeline,
        aerial: Option<(&wgpu::TextureView, &wgpu::Sampler)>,
        cloud: Option<(&wgpu::TextureView, &wgpu::Sampler, &wgpu::Buffer)>,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        let next_aerial = aerial.map(|(view, _)| view.clone());
        if self.current_aerial != next_aerial {
            let aux = &self.camera_aux;
            let (view, sampler) = aerial.unwrap_or((&aux.aerial, &aux.aerial_sampler));
            self.camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Blue Water live atmosphere camera"),
                layout: &pipeline.camera_layout,
                entries: &[
                    entry_buffer(0, &self.camera),
                    entry_buffer(1, &aux.shadows),
                    entry_texture(2, &aux.shadow_array),
                    entry_texture(3, &aux.shadow_wide),
                    entry_sampler(4, &aux.shadow_sampler),
                    entry_buffer(5, &aux.motion),
                    entry_texture(6, view),
                    entry_sampler(7, sampler),
                    entry_buffer(8, &aux.occluder),
                    entry_buffer(9, &aux.occluder_uniform),
                ],
            });
            self.current_aerial = next_aerial;
        }
        let next_cloud = cloud.map(|(view, _, _)| view.clone());
        if self.current_cloud != next_cloud {
            let (view, sampler, uniform) = cloud.unwrap_or((
                &self.neutral_cloud.0,
                &self.neutral_cloud.1,
                &self.neutral_cloud.2,
            ));
            self.cloud_view = view.clone();
            self.cloud_sampler = sampler.clone();
            self.cloud_uniform = uniform.clone();
            self.current_scene = None;
            self.set_scene(device, pipeline, color, depth);
            self.current_cloud = next_cloud;
        }
    }
    pub fn underwater_probe(&self) -> &wgpu::TextureView {
        &self.probe_view
    }

    pub fn write_camera(&self, queue: &wgpu::Queue, camera: &SurfaceCamera) {
        queue.write_buffer(&self.camera, 0, bytemuck::bytes_of(camera));
    }

    pub fn write_boundary(&self, queue: &wgpu::Queue, boundary: &SurfaceBoundary) {
        assert!(boundary.counts[0] <= 256 && boundary.counts[1] <= 16);
        queue.write_buffer(&self.boundary, 0, bytemuck::bytes_of(boundary));
    }

    pub fn foam_streak_mean(&self) -> f32 {
        self.foam.streak_mean
    }

    pub fn set_environment_atlas(
        &mut self,
        device: &wgpu::Device,
        pipeline: &OriginalSurfacePipeline,
        atlas: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) {
        self.environment_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water live sky environment"),
            layout: &pipeline.environment_layout,
            entries: &[
                entry_texture(0, atlas),
                entry_sampler(1, sampler),
                entry_buffer(2, &self.environment),
            ],
        });
    }

    pub fn use_ambient_atlas(&mut self, device: &wgpu::Device, pipeline: &OriginalSurfacePipeline) {
        let view = self.ambient_atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        self.set_environment_atlas(device, pipeline, &view, &sampler);
    }

    /// Use the original atmosphere's atlas and complete sun/moon lighting block.
    pub fn set_sky_environment(
        &mut self,
        device: &wgpu::Device,
        pipeline: &OriginalSurfacePipeline,
        atlas: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniforms: &wgpu::Buffer,
    ) {
        self.environment_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water original atmosphere lighting"),
            layout: &pipeline.environment_layout,
            entries: &[
                entry_texture(0, atlas),
                entry_sampler(1, sampler),
                entry_buffer(2, uniforms),
            ],
        });
    }

    /// Map ambient fallback until the directional sky atlas is available.
    pub fn write_ambient(&mut self, queue: &wgpu::Queue, ambient: [f32; 3]) {
        if self.ambient == ambient {
            return;
        }
        self.ambient = ambient;
        let rgb = ambient.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        let mut pixels = vec![0u8; 384 * 512 * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ambient_atlas,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(384 * 4),
                rows_per_image: Some(512),
            },
            wgpu::Extent3d {
                width: 384,
                height: 512,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn write_surface_prefix(&self, queue: &wgpu::Queue, first_25_vec4: &[[f32; 4]; 25]) {
        assert!(self.surface_bytes >= 25 * 16);
        queue.write_buffer(&self.surface, 0, bytemuck::bytes_of(first_25_vec4));
    }

    pub fn write_sun(
        &self,
        queue: &wgpu::Queue,
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
    ) {
        let mut data = [0.0f32; 20];
        data[0] = 1.0 / 384.0;
        data[1] = 1.0 / 512.0;
        data[2] = 7.0;
        data[4..7].copy_from_slice(&direction);
        data[7] = intensity;
        data[8..11].copy_from_slice(&color);
        data[12..15].copy_from_slice(&direction);
        data[15] = 0.5;
        data[17] = -1.0;
        data[19] = 1.0;
        queue.write_buffer(&self.environment, 0, bytemuck::cast_slice(&data));
    }
}

impl OriginalSurfacePipeline {
    pub fn new(
        device: &wgpu::Device,
        waves: &SurfaceWaves,
        color_format: wgpu::TextureFormat,
        samples: u32,
        reflection_steps: u32,
    ) -> Self {
        let vertex = wgpu::ShaderStages::VERTEX;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let both = vertex | fragment;
        let t2 = wgpu::TextureViewDimension::D2;
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blue Water original camera layout"),
            entries: &[
                uniform(0, both),
                uniform(1, fragment),
                sampled(2, fragment, wgpu::TextureViewDimension::D2Array, true),
                sampled(3, fragment, t2, true),
                sampler(4, fragment, true),
                uniform(5, both),
                sampled(6, fragment, wgpu::TextureViewDimension::D3, false),
                sampler(7, fragment, false),
                storage(8, fragment),
                uniform(9, fragment),
            ],
        });
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blue Water original material layout"),
            entries: &[
                uniform(0, both),
                sampled(1, fragment, t2, false),
                sampler(2, fragment, false),
                sampled(3, fragment, t2, false),
                sampled(4, fragment, t2, true),
                uniform(5, both),
                sampled(6, both, t2, false),
                sampled(7, both, t2, false),
                sampler(8, both, false),
                sampled(9, fragment, t2, false),
                sampled(10, fragment, t2, false),
                sampled(11, fragment, t2, false),
                sampler(12, fragment, false),
                uniform(13, fragment),
                sampled(14, fragment, t2, false),
                storage(15, fragment),
                uniform(16, fragment),
            ],
        });
        let environment_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Blue Water original environment layout"),
                entries: &[
                    sampled(0, fragment, t2, false),
                    sampler(1, fragment, false),
                    uniform(2, fragment),
                ],
            });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blue Water original surface"),
            bind_group_layouts: &[
                Some(&camera_layout),
                Some(&surface_layout),
                Some(waves.layout()),
                Some(&environment_layout),
            ],
            immediate_size: 0,
        });
        let mut composer = shader_composer::composer().expect("Blue Water shader modules");
        let source = shader_composer::jka_surface_module(&mut composer, reflection_steps)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&composer)));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blue Water original surface shader"),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(source)),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blue Water original surface"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(geometry::OceanVertex::LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            camera_layout,
            surface_layout,
            environment_layout,
            pipeline,
            wireframe: None,
            vertex_shader: shader,
            pipeline_layout: layout,
        }
    }

    /// Compile this diagnostic variant only when r_showtris enables ocean lines.
    pub fn ensure_wireframe(&mut self, device: &wgpu::Device, color_format: wgpu::TextureFormat, samples: u32) {
        if self.wireframe.is_some() || !device.features().contains(wgpu::Features::POLYGON_MODE_LINE) {
            return;
        }
        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blue Water native wireframe color"),
            source: wgpu::ShaderSource::Wgsl(
                "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.9, 0.1, 1.0); }".into(),
            ),
        });
        self.wireframe = Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blue Water original native wireframe"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.vertex_shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(geometry::OceanVertex::LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &line_shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                polygon_mode: wgpu::PolygonMode::Line,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            multiview_mask: None,
            cache: None,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blue_water_core::QualityTier;

    #[test]
    fn jka_fragment_boundary_preserves_outline_pvs_and_sky_contacts() {
        futures_lite::future::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&Default::default())
                .await
                .expect("GPU adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_limits: adapter.limits(),
                    ..Default::default()
                })
                .await
                .expect("GPU device");
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let points: Vec<[f32; 2]> = (-2..=12)
                .flat_map(|z| (-2..=12).map(move |x| [x as f32 * 10.0, z as f32 * 10.0]))
                .collect();
            let metres: Vec<_> = points.iter().map(|p| [p[0] / 40.0, p[1] / 40.0]).collect();
            let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("JKA boundary sample points"),
                contents: bytemuck::cast_slice(&metres),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let bytes = (points.len() * 4) as u64;
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("JKA boundary results"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("JKA boundary readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let source = format!(
                "{}\n\
                @group(0) @binding(0) var<storage, read> points: array<vec2<f32>>;\n\
                @group(0) @binding(1) var<storage, read_write> results: array<u32>;\n\
                @compute @workgroup_size(64) fn test_boundary(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
                    if id.x < arrayLength(&points) {{ results[id.x] = u32(jka_water_contains(points[id.x])); }}\n\
                }}",
                include_str!("surface_boundary.wgsl")
            );
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("JKA boundary test"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("JKA boundary test"),
                layout: None,
                module: &shader,
                entry_point: Some("test_boundary"),
                compilation_options: Default::default(),
                cache: None,
            });
            let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[entry_buffer(0, &input), entry_buffer(1, &output)],
            });
            for case in 0..5 {
                let mut boundary = SurfaceBoundary::zeroed();
                boundary.bounds = [0.0, 0.0, 100.0, 100.0];
                boundary.scale = [40.0, 0.75, 0.0, 0.0];
                if case == 1 || case == 4 {
                    boundary.counts[0] = 4;
                    boundary.triangles[..8].copy_from_slice(&[
                        [0.0, 0.0, 40.0, 0.0],
                        [40.0, 100.0, 0.0, 0.0],
                        [0.0, 0.0, 40.0, 100.0],
                        [0.0, 100.0, 0.0, 0.0],
                        [40.0, 0.0, 100.0, 0.0],
                        [100.0, 40.0, 0.0, 0.0],
                        [40.0, 0.0, 100.0, 40.0],
                        [40.0, 40.0, 0.0, 0.0],
                    ]);
                    if case == 4 {
                        boundary.counts[3] = 3;
                        boundary.prepare_candidates();
                    }
                } else if case == 2 {
                    boundary.counts[2] = 1;
                    boundary.pvs = [[0xffff0000; 4]; 8];
                } else if case == 3 {
                    boundary.counts[1] = 2;
                    boundary.sky[0] = [0.0, 0.0, 100.0, 1.0];
                    boundary.sky[1] = [0.0, 0.0, 100.0, 3.0];
                }
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::bytes_of(&boundary),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(1),
                    entries: &[entry_buffer(16, &uniform)],
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &inputs, &[]);
                    pass.set_bind_group(1, &group, &[]);
                    pass.dispatch_workgroups((points.len() as u32).div_ceil(64), 1, 1);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &read, 0, bytes);
                queue.submit(Some(encoder.finish()));
                read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: None,
                    })
                    .unwrap();
                {
                    let mapped = read.slice(..).get_mapped_range().unwrap();
                    let actual: &[u32] = bytemuck::cast_slice(&mapped);
                    for (&[x, z], &actual) in points.iter().zip(actual) {
                        let inside = x >= 0.0 && z >= 0.0 && x <= 100.0 && z <= 100.0;
                        let expected = match case {
                            1 | 4 => inside && (x <= 40.0 || z <= 40.0),
                            2 => inside && x >= 50.0,
                            3 => x <= 100.0 && z <= 100.0,
                            _ => inside,
                        };
                        assert_eq!(actual != 0, expected, "case {case}, point ({x}, {z})");
                    }
                }
                read.unmap();
            }
            assert!(scope.pop().await.is_none(), "JKA boundary GPU validation");
        });
    }

    #[test]
    fn original_surface_pipeline_accepts_explicit_jka_bind_group_layouts() {
        futures_lite::future::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .expect("GPU adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_limits: adapter.limits(),
                    ..Default::default()
                })
                .await
                .expect("GPU device");
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let field = crate::WaveField::new(&device, QualityTier::High);
            let waves = SurfaceWaves::new(&device, &field);
            let pipeline =
                OriginalSurfacePipeline::new(&device, &waves, wgpu::TextureFormat::Rgba16Float, 1, 16);
            let bindings = OriginalSurfaceBindings::new(&device, &queue, &pipeline);
            let clipmap = OriginalClipmap::new(&device, 128, 9);
            let mut camera = SurfaceCamera::zeroed();
            for matrix in [
                &mut camera.view,
                &mut camera.projection,
                &mut camera.view_projection,
                &mut camera.inverse_view,
                &mut camera.inverse_projection,
                &mut camera.inverse_view_projection,
            ] {
                for (axis, row) in matrix.iter_mut().enumerate() {
                    row[axis] = 1.0;
                }
            }
            camera.position_time = [0.0, 1.0, 0.0, 0.0];
            camera.viewport = [16.0, 16.0, 1.0 / 16.0, 1.0 / 16.0];
            camera.near = 0.025;
            camera.far = 100.0;
            camera.tan_half_fov_y = 0.5;
            camera.aspect = 1.0;
            bindings.write_camera(&queue, &camera);
            bindings.write_surface_prefix(
                &queue,
                &clipmap.surface_prefix([0.0, 0.0], 1.0, SurfaceAppearance::default()),
            );
            let target = |format| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Blue Water surface draw validation"),
                        size: wgpu::Extent3d {
                            width: 16,
                            height: 16,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            let color = target(wgpu::TextureFormat::Rgba16Float);
            let depth = target(wgpu::TextureFormat::Depth32Float);
            let water_distance = target(wgpu::TextureFormat::R32Float);
            let scene = crate::scene_copy::SceneCopy::new(&device, 16, 16, 1);
            let underwater_pipelines = crate::underwater::UnderwaterPipelines::new(
                &device,
                waves.layout(),
                wgpu::TextureFormat::Rgba16Float,
                1,
            );
            let underwater = crate::underwater::UnderwaterBindings::new(
                &device,
                &underwater_pipelines,
                &bindings,
                &scene.depth,
                &water_distance,
            );
            underwater.write(
                &queue,
                &camera,
                SurfaceAppearance::default(),
                [0.0, 1.0, 0.0],
                [1.0; 3],
                false,
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            underwater.probe(&mut encoder, &underwater_pipelines, waves.group(&field));
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Blue Water original clipmap draw validation"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_bind_group(0, &bindings.camera_group, &[]);
                pass.set_bind_group(1, &bindings.material_group, &[]);
                pass.set_bind_group(2, waves.group(&field), &[]);
                pass.set_bind_group(3, &bindings.environment_group, &[]);
                clipmap.draw(&mut pass, 0, 0);
            }
            scene.capture(&device, &mut encoder, &color, &depth);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Original underwater dry-view draw validation"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                underwater.draw(&mut pass, &underwater_pipelines, true);
            }
            queue.submit(Some(encoder.finish()));
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            assert!(
                scope.pop().await.is_none(),
                "original Blue Water surface layout"
            );
        });
    }
}

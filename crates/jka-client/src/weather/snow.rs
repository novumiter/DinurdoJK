use crate::{
    camera::Camera,
    pipeline_jobs::{hash_config as pipeline_hash, PipelineJobKey, PipelineJobManager},
};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::rain::WeatherOcclusionInfo;

/// Retail `r_we snow` creates 1,000 particles per cloud, but that count was tuned
/// for much larger legacy flakes. Our smaller modern flakes need more samples to
/// preserve an equally full snow field without turning each particle into a card.
pub(crate) const SNOW_PARTICLES_PER_CLOUD: u32 = 1_600;
pub(crate) const SNOW_MAX_PARTICLES: u32 = SNOW_PARTICLES_PER_CLOUD * crate::world_effects::MAX_PARTICLE_CLOUDS as u32;
const SNOW_COMPUTE_GROUP_SIZE: u32 = 128;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
// Keep the slow snow volume on a coarse world-space lattice so flakes do not swim with ordinary camera motion.
const SNOW_WORLD_ANCHOR_STEP: f32 = 128.0;
const SNOW_WORLD_ANCHOR_DEADBAND: f32 = 96.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SnowParticle {
    position_state: [f32; 4],
    velocity_seed: [f32; 4],
    misc: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SnowSimUniform {
    camera_dt: [f32; 4],
    /// radius, below-camera extent, above-camera extent, base fall speed.
    spawn: [f32; 4],
    /// instantaneous weather wind X/Z, time, flutter strength.
    wind_time: [f32; 4],
    /// stable/base weather wind X/Z, live camera Y for roof tests, reserved.
    /// The snow particle field itself uses the persistent world anchor.
    weather: [f32; 4],
    collision_uv: [f32; 4],
    /// active particles, heightfield width, heightfield height, reserved.
    counts: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SnowRenderUniform {
    /// minimum size, maximum size, opacity, viewport height.
    appearance: [f32; 4],
    color: [f32; 4],
    /// reserved, near fade end, far fade start, far fade end.
    motion: [f32; 4],
}

pub(crate) struct SnowGpuResources {
    particle_buffer: wgpu::Buffer,
    sim_uniform_buffer: wgpu::Buffer,
    sim_layout: wgpu::BindGroupLayout,
    sim_bind_group: wgpu::BindGroup,
    sim_pipeline: wgpu::ComputePipeline,
    render_uniform_buffer: wgpu::Buffer,
    render_bind_group: wgpu::BindGroup,
    render_pipeline_layout: wgpu::PipelineLayout,
    render_shader: wgpu::ShaderModule,
    render_pipeline: Option<wgpu::RenderPipeline>,
    render_format: wgpu::TextureFormat,
    render_samples: u32,
}

pub(crate) struct SnowSystem {
    gpu: SnowGpuResources,
    pub enabled: bool,
    /// Number of stacked retail snow clouds, expressed as a density multiplier.
    pub density_scale: f32,
    /// `r_we freeze` keeps the current particle state on screen.
    pub frozen: bool,
    /// Previous precipitation anchor. Falling flakes are carried by ordinary
    /// horizontal camera translation so the local weather volume cannot develop
    /// a sparse leading edge while the player moves through it.
    world_anchor: Option<glam::Vec3>,
}

impl SnowSystem {
    pub(crate) fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
        collision_view: &wgpu::TextureView,
        wind_noise_view: &wgpu::TextureView,
        wind_noise_sampler: &wgpu::Sampler,
    ) -> Self {
        Self {
            gpu: create_snow_gpu_resources(
                device,
                camera_layout,
                surface_format,
                sample_count,
                collision_view,
                wind_noise_view,
                wind_noise_sampler,
            ),
            enabled: false,
            density_scale: 1.0,
            frozen: false,
            world_anchor: None,
        }
    }

    fn world_anchor(&mut self, camera_position: glam::Vec3) -> glam::Vec3 {
        fn snapped(value: f32) -> f32 {
            (value / SNOW_WORLD_ANCHOR_STEP).round() * SNOW_WORLD_ANCHOR_STEP
        }

        fn advance_axis(anchor: f32, target: f32) -> f32 {
            let delta = target - anchor;
            if delta > SNOW_WORLD_ANCHOR_DEADBAND {
                let steps = ((delta - SNOW_WORLD_ANCHOR_DEADBAND) / SNOW_WORLD_ANCHOR_STEP)
                    .ceil()
                    .max(1.0);
                anchor + steps * SNOW_WORLD_ANCHOR_STEP
            } else if delta < -SNOW_WORLD_ANCHOR_DEADBAND {
                let steps = ((-delta - SNOW_WORLD_ANCHOR_DEADBAND) / SNOW_WORLD_ANCHOR_STEP)
                    .ceil()
                    .max(1.0);
                anchor - steps * SNOW_WORLD_ANCHOR_STEP
            } else {
                anchor
            }
        }

        let mut anchor = self.world_anchor.unwrap_or_else(|| {
            glam::Vec3::new(
                snapped(camera_position.x),
                snapped(camera_position.y),
                snapped(camera_position.z),
            )
        });
        anchor.x = advance_axis(anchor.x, camera_position.x);
        anchor.y = advance_axis(anchor.y, camera_position.y);
        anchor.z = advance_axis(anchor.z, camera_position.z);
        self.world_anchor = Some(anchor);
        anchor
    }
    fn particle_count(&self) -> u32 {
        let scale = if self.density_scale.is_finite() {
            self.density_scale
                .clamp(0.0, crate::world_effects::MAX_PARTICLE_CLOUDS as f32)
        } else {
            1.0
        };
        (SNOW_PARTICLES_PER_CLOUD as f32 * scale)
            .round()
            .clamp(0.0, SNOW_MAX_PARTICLES as f32) as u32
    }

    pub(crate) fn rebuild_collision_bind_group(
        &mut self,
        device: &wgpu::Device,
        collision_view: &wgpu::TextureView,
        wind_noise_view: &wgpu::TextureView,
        wind_noise_sampler: &wgpu::Sampler,
    ) {
        self.gpu.sim_bind_group = create_snow_sim_bind_group(
            device,
            &self.gpu.sim_layout,
            &self.gpu.particle_buffer,
            &self.gpu.sim_uniform_buffer,
            collision_view,
            wind_noise_view,
            wind_noise_sampler,
        );
    }

    pub(crate) fn rebuild_render_pipeline(
        &mut self,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
    ) {
        self.gpu.render_format = surface_format;
        self.gpu.render_samples = sample_count;
        self.gpu.render_pipeline = None;
    }

    pub(crate) fn ensure_render_pipeline(
        &mut self,
        jobs: &mut PipelineJobManager,
        device: &wgpu::Device,
    ) {
        if self.gpu.render_pipeline.is_some() {
            return;
        }
        let config = pipeline_hash(&(self.gpu.render_format, self.gpu.render_samples));
        let key = PipelineJobKey::new("snow", 0, config);
        if let Some(pipeline) = jobs.take_ready(key) {
            self.gpu.render_pipeline = Some(pipeline);
            return;
        }
        let device = device.clone();
        let layout = self.gpu.render_pipeline_layout.clone();
        let shader = self.gpu.render_shader.clone();
        let format = self.gpu.render_format;
        let samples = self.gpu.render_samples;
        jobs.request(key, "snow flakes", move || {
            create_snow_render_pipeline(&device, &layout, &shader, format, samples)
        });
    }

    pub(crate) fn prepare_frame(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        frame_time: f32,
        previous_frame_time: f32,
        viewport_height: u32,
        weather_wind: crate::ocean::OceanWind,
        occlusion: Option<WeatherOcclusionInfo>,
        sun_color: [f32; 3],
        sun_intensity: f32,
    ) {
        let dt = if previous_frame_time > 0.0 {
            (frame_time - previous_frame_time).clamp(0.0, 0.05)
        } else {
            1.0 / 120.0
        };
        let (collision_uv, collision_width, collision_height) = occlusion.map_or(
            ([0.0; 4], 0, 0),
            |info| {
                (
                    [
                        info.min_xz[0],
                        info.min_xz[1],
                        info.inv_extent_xz[0],
                        info.inv_extent_xz[1],
                    ],
                    info.width,
                    info.height,
                )
            },
        );
        let wind = weather_wind.at(frame_time);
        let weather = weather_wind.sanitize();
        let angle = weather.direction.to_radians();
        let spawn_radius = 900.0;
        let anchor = self.world_anchor(camera.position);
        let sim = SnowSimUniform {
            // Persistent world anchor; the live camera Y is carried separately for roof tests.
            camera_dt: [anchor.x, anchor.y, anchor.z, dt],
            // A tighter/shallower local volume than rain keeps overdraw low while
            // still surrounding the player with enough slow-moving flakes.
            spawn: [spawn_radius, 300.0, 850.0, 145.0],
            wind_time: [wind[0], wind[1], frame_time, 26.0],
            weather: [
                angle.cos() * weather.speed,
                angle.sin() * weather.speed,
                camera.position.y,
                0.0,
            ],
            collision_uv,
            counts: [self.particle_count(), collision_width, collision_height, 0],
        };
        queue.write_buffer(&self.gpu.sim_uniform_buffer, 0, bytemuck::bytes_of(&sim));

        // Snow is a bright high-albedo medium. Keep the flakes below pure white
        // so HDR/bloom has room for real highlights and does not turn every flake
        // into a glowing dot.
        let sun_scale = (sun_intensity / 250.0).max(0.0).sqrt().clamp(0.55, 1.12);
        let sun = glam::Vec3::from_array(sun_color).max(glam::Vec3::splat(0.10));
        let base = glam::Vec3::new(0.82, 0.87, 0.93);
        let lit = (base * 0.78 + sun * 0.22) * sun_scale;
        let render = SnowRenderUniform {
            appearance: [1.25, 3.35, 0.52, viewport_height.max(1) as f32],
            color: [lit.x, lit.y, lit.z, 1.0],
            motion: [0.0, 52.0, 900.0, 1_650.0],
        };
        queue.write_buffer(
            &self.gpu.render_uniform_buffer,
            0,
            bytemuck::bytes_of(&render),
        );
    }

    pub(crate) fn dispatch_simulation(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.enabled || self.frozen {
            return;
        }
        let count = self.particle_count();
        if count == 0 {
            return;
        }
        let groups = (count + SNOW_COMPUTE_GROUP_SIZE - 1) / SNOW_COMPUTE_GROUP_SIZE;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("JKA GPU snow simulation"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.gpu.sim_pipeline);
        pass.set_bind_group(0, &self.gpu.sim_bind_group, &[]);
        pass.dispatch_workgroups(groups, 1, 1);
    }

    pub(crate) fn render<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bind_group: &'a wgpu::BindGroup,
    ) {
        if !self.enabled {
            return;
        }
        let Some(pipeline) = self.gpu.render_pipeline.as_ref() else {
            return;
        };
        let count = self.particle_count();
        if count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, &self.gpu.render_bind_group, &[]);
        pass.draw(0..4, 0..count);
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
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

fn storage_buffer_entry(
    binding: u32,
    read_only: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn create_snow_sim_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    particle_buffer: &wgpu::Buffer,
    sim_uniform_buffer: &wgpu::Buffer,
    collision_view: &wgpu::TextureView,
    wind_noise_view: &wgpu::TextureView,
    wind_noise_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("JKA GPU snow simulation bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: particle_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: sim_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(collision_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(wind_noise_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(wind_noise_sampler),
            },
        ],
    })
}

fn create_snow_gpu_resources(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    collision_view: &wgpu::TextureView,
    wind_noise_view: &wgpu::TextureView,
    wind_noise_sampler: &wgpu::Sampler,
) -> SnowGpuResources {
    let particles = vec![SnowParticle::zeroed(); SNOW_MAX_PARTICLES as usize];
    let particle_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("JKA GPU snow particle state"),
        contents: bytemuck::cast_slice(&particles),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    let sim_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("JKA GPU snow simulation uniform"),
        contents: bytemuck::bytes_of(&SnowSimUniform {
            camera_dt: [0.0, 0.0, 0.0, 1.0 / 120.0],
            spawn: [900.0, 300.0, 850.0, 145.0],
            wind_time: [0.0, 0.0, 0.0, 26.0],
            weather: [0.0; 4],
            collision_uv: [0.0; 4],
            counts: [SNOW_PARTICLES_PER_CLOUD, 0, 0, 0],
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("JKA GPU snow simulation layout"),
        entries: &[
            storage_buffer_entry(0, false, wgpu::ShaderStages::COMPUTE),
            uniform_entry(1, wgpu::ShaderStages::COMPUTE),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sim_bind_group = create_snow_sim_bind_group(
        device,
        &sim_layout,
        &particle_buffer,
        &sim_uniform_buffer,
        collision_view,
        wind_noise_view,
        wind_noise_sampler,
    );
    let sim_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("JKA GPU snow compute shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../snow_sim.wgsl").into()),
    });
    let sim_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("JKA GPU snow simulation pipeline layout"),
        bind_group_layouts: &[Some(&sim_layout)],
        immediate_size: 0,
    });
    let sim_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("JKA GPU snow simulation pipeline"),
        layout: Some(&sim_pipeline_layout),
        module: &sim_shader,
        entry_point: Some("cs_main"),
        compilation_options: Default::default(),
        cache: crate::pipeline_cache::get().as_ref(),
    });

    let render_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("JKA GPU snow render uniform"),
        contents: bytemuck::bytes_of(&SnowRenderUniform {
            appearance: [1.25, 3.35, 0.52, 1.0],
            color: [0.82, 0.87, 0.93, 1.0],
            motion: [0.0, 52.0, 900.0, 1_650.0],
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("JKA GPU snow render layout"),
        entries: &[
            storage_buffer_entry(0, true, wgpu::ShaderStages::VERTEX),
            uniform_entry(1, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
        ],
    });
    let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("JKA GPU snow render bind group"),
        layout: &render_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: particle_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: render_uniform_buffer.as_entire_binding(),
            },
        ],
    });
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("JKA GPU snow render pipeline layout"),
        bind_group_layouts: &[Some(camera_layout), Some(&render_layout)],
        immediate_size: 0,
    });
    let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("JKA GPU snow render shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../snow.wgsl").into()),
    });

    SnowGpuResources {
        particle_buffer,
        sim_uniform_buffer,
        sim_layout,
        sim_bind_group,
        sim_pipeline,
        render_uniform_buffer,
        render_bind_group,
        render_pipeline_layout,
        render_shader,
        render_pipeline: None,
        render_format: surface_format,
        render_samples: sample_count,
    }
}

fn create_snow_render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let additive = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("JKA GPU snow flake pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(additive),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: crate::pipeline_cache::get().as_ref(),
    })
}

#[cfg(test)]
pub(crate) mod layout_tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn gpu_structs_remain_vec4_aligned() {
        assert_eq!(size_of::<SnowParticle>(), 48);
        assert_eq!(size_of::<SnowSimUniform>(), 96);
        assert_eq!(size_of::<SnowRenderUniform>(), 48);
    }
}

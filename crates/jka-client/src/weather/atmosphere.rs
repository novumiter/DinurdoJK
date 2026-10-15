//! Modern GPU presentation for the legacy `r_we sand` and `r_we spacedust` clouds.
//!
//! Unlike rain these effects do not need per-particle collision state.  Their particles
//! are therefore generated analytically in the vertex shader from stable world-space
//! periodic cells.  That avoids a storage buffer and compute dispatch entirely while
//! preserving the useful legacy behaviour: spacedust stays almost fixed without wind,
//! sand streams with the atmospheric wind, and both remain camera-local in cost.

use crate::{
    pipeline_jobs::{hash_config as pipeline_hash, PipelineJobKey, PipelineJobManager},
    world_effects::AtmosphereOverride,
};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const MAX_SPACE_DUST: u32 = 16_384;
const SAND_PARTICLES_PER_LAYER: u32 = 3_072;
const MAX_SAND_PARTICLES: u32 = 12_288;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AtmosphereUniform {
    /// x sand particles, y space-dust particles, z/w reserved.
    counts: [u32; 4],
    /// instantaneous wind X/Z, effect-local time, absolute frame time.
    wind_time: [f32; 4],
    /// sand strength, spacedust strength, reserved x2.
    strength: [f32; 4],
}

pub(crate) struct AtmosphereSystem {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    pipeline: Option<wgpu::RenderPipeline>,
    render_format: wgpu::TextureFormat,
    render_samples: u32,
    sand_particles: u32,
    space_particles: u32,
    sand_strength: f32,
    space_strength: f32,
    frozen: bool,
    effect_time: f32,
    previous_frame_time: f32,
    held_wind: [f32; 2],
}

impl AtmosphereSystem {
    pub(crate) fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("JKA legacy atmosphere uniform"),
            contents: bytemuck::bytes_of(&AtmosphereUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("JKA legacy atmosphere layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JKA legacy atmosphere bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("JKA legacy atmosphere pipeline layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("JKA legacy atmosphere shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../atmosphere.wgsl").into()),
        });
        Self {
            uniform_buffer,
            bind_group,
            pipeline_layout,
            shader,
            pipeline: None,
            render_format: surface_format,
            render_samples: sample_count,
            sand_particles: 0,
            space_particles: 0,
            sand_strength: 0.0,
            space_strength: 0.0,
            frozen: false,
            effect_time: 0.0,
            previous_frame_time: 0.0,
            held_wind: [0.0; 2],
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.sand_particles != 0 || self.space_particles != 0
    }

    pub(crate) fn sand_strength(&self) -> f32 {
        self.sand_strength
    }

    pub(crate) fn space_strength(&self) -> f32 {
        self.space_strength
    }

    pub(crate) fn set_override(&mut self, effect: AtmosphereOverride) {
        self.sand_particles = effect
            .sand_layers
            .saturating_mul(SAND_PARTICLES_PER_LAYER)
            .min(MAX_SAND_PARTICLES);
        self.space_particles = effect.space_dust_count.min(MAX_SPACE_DUST);
        self.sand_strength = (effect.sand_layers as f32 * 0.72).clamp(0.0, 1.35);
        self.space_strength = (effect.space_dust_count as f32 / 1800.0).clamp(0.0, 1.25);
        self.frozen = effect.frozen;
    }

    pub(crate) fn rebuild_render_pipeline(
        &mut self,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
    ) {
        self.render_format = surface_format;
        self.render_samples = sample_count;
        self.pipeline = None;
    }

    pub(crate) fn ensure_pipeline(&mut self, jobs: &mut PipelineJobManager, device: &wgpu::Device) {
        if !self.active() || self.pipeline.is_some() {
            return;
        }
        let key = PipelineJobKey::new(
            "legacy-atmosphere",
            0,
            pipeline_hash(&(self.render_format, self.render_samples)),
        );
        if let Some(pipeline) = jobs.take_ready(key) {
            self.pipeline = Some(pipeline);
            return;
        }
        let device = device.clone();
        let layout = self.pipeline_layout.clone();
        let shader = self.shader.clone();
        let format = self.render_format;
        let samples = self.render_samples;
        jobs.request(key, "sand/spacedust particles", move || {
            create_pipeline(&device, &layout, &shader, format, samples)
        });
    }

    pub(crate) fn prepare_frame(
        &mut self,
        queue: &wgpu::Queue,
        frame_time: f32,
        weather_wind: crate::ocean::OceanWind,
    ) {
        let dt = if self.previous_frame_time > 0.0 {
            (frame_time - self.previous_frame_time).clamp(0.0, 0.05)
        } else {
            1.0 / 120.0
        };
        self.previous_frame_time = frame_time;
        if !self.frozen {
            self.effect_time += dt;
            self.held_wind = weather_wind.at(frame_time);
        }
        let uniform = AtmosphereUniform {
            counts: [self.sand_particles, self.space_particles, 0, 0],
            wind_time: [self.held_wind[0], self.held_wind[1], self.effect_time, frame_time],
            strength: [self.sand_strength, self.space_strength, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub(crate) fn render<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bind_group: &'a wgpu::BindGroup,
    ) {
        let Some(pipeline) = self.pipeline.as_ref().filter(|_| self.active()) else {
            return;
        };
        let count = self.sand_particles.saturating_add(self.space_particles);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.draw(0..4, 0..count);
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("JKA sand/spacedust particle pipeline"),
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: crate::pipeline_cache::get().as_ref(),
    })
}

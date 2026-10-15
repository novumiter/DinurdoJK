use crate::{
    ocean::AtmosphericWind,
    pipeline_jobs::{hash_config as pipeline_hash, PipelineJobKey, PipelineJobManager},
};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::rain::WeatherOcclusionInfo;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MOTE_COUNT: u32 = 4096;
const WISP_COUNT: u32 = 64;
const VOLUME_RADIUS: f32 = 2_400.0;
const VOLUME_HALF_HEIGHT: f32 = 720.0;
// Wrap by exactly one horizontal particle-volume span, so removing old
// accumulated distance cannot cause the procedural torus to jump.
const ADVECTION_WRAP: f32 = VOLUME_RADIUS * 2.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WindVisualUniform {
    wind_time: [f32; 4],
    base_wind: [f32; 4],
    advection: [f32; 4],
    collision_uv: [f32; 4],
    collision_size: [u32; 4],
    volume: [f32; 4],
}

pub(crate) struct WindVisualSystem {
    uniform_buffer: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    mote_pipeline: Option<wgpu::RenderPipeline>,
    wisp_pipeline: Option<wgpu::RenderPipeline>,
    render_format: wgpu::TextureFormat,
    render_samples: u32,
    pub strength: f32,
    advection: [f32; 2],
    last_time: Option<f32>,
}

impl WindVisualSystem {
    pub(crate) fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
        collision_view: &wgpu::TextureView,
        wind_noise_view: &wgpu::TextureView,
        wind_noise_sampler: &wgpu::Sampler,
    ) -> Self {
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("JKA atmospheric wind visual uniform"),
            contents: bytemuck::bytes_of(&WindVisualUniform {
                wind_time: [0.0; 4],
                base_wind: [0.0; 4],
                advection: [0.0; 4],
                collision_uv: [0.0; 4],
                collision_size: [0; 4],
                volume: [VOLUME_RADIUS, VOLUME_HALF_HEIGHT, 1.0, 1.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("JKA atmospheric wind visual layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = create_bind_group(
            device,
            &layout,
            &uniform_buffer,
            collision_view,
            wind_noise_view,
            wind_noise_sampler,
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("JKA atmospheric wind visual pipeline layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("JKA atmospheric wind visual shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../wind_visuals.wgsl").into()),
        });
        Self {
            uniform_buffer,
            layout,
            bind_group,
            pipeline_layout,
            shader,
            mote_pipeline: None,
            wisp_pipeline: None,
            render_format: surface_format,
            render_samples: sample_count,
            strength: 0.0,
            advection: [0.0; 2],
            last_time: None,
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.strength > 0.001
    }

    pub(crate) fn rebuild_field_bind_group(
        &mut self,
        device: &wgpu::Device,
        collision_view: &wgpu::TextureView,
        wind_noise_view: &wgpu::TextureView,
        wind_noise_sampler: &wgpu::Sampler,
    ) {
        self.bind_group = create_bind_group(
            device,
            &self.layout,
            &self.uniform_buffer,
            collision_view,
            wind_noise_view,
            wind_noise_sampler,
        );
    }

    pub(crate) fn rebuild_render_pipelines(
        &mut self,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
    ) {
        self.render_format = surface_format;
        self.render_samples = sample_count;
        self.mote_pipeline = None;
        self.wisp_pipeline = None;
    }

    pub(crate) fn ensure_render_pipelines(
        &mut self,
        jobs: &mut PipelineJobManager,
        device: &wgpu::Device,
    ) {
        let config = pipeline_hash(&(self.render_format, self.render_samples));
        if self.mote_pipeline.is_none() {
            let key = PipelineJobKey::new("wind_visuals", 0, config);
            if let Some(pipeline) = jobs.take_ready(key) {
                self.mote_pipeline = Some(pipeline);
            } else {
                let device = device.clone();
                let layout = self.pipeline_layout.clone();
                let shader = self.shader.clone();
                let format = self.render_format;
                let samples = self.render_samples;
                jobs.request(key, "wind motes", move || {
                    create_pipeline(&device, &layout, &shader, format, samples, "vs_mote", "fs_mote", "JKA wind motes")
                });
            }
        }
        if self.wisp_pipeline.is_none() {
            let key = PipelineJobKey::new("wind_visuals", 1, config);
            if let Some(pipeline) = jobs.take_ready(key) {
                self.wisp_pipeline = Some(pipeline);
            } else {
                let device = device.clone();
                let layout = self.pipeline_layout.clone();
                let shader = self.shader.clone();
                let format = self.render_format;
                let samples = self.render_samples;
                jobs.request(key, "wind wisps", move || {
                    create_pipeline(&device, &layout, &shader, format, samples, "vs_wisp", "fs_wisp", "JKA wind wisps")
                });
            }
        }
    }

    pub(crate) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        frame_time: f32,
        wind: AtmosphericWind,
        occlusion: Option<WeatherOcclusionInfo>,
        camera_underwater: bool,
    ) {
        let wind = wind.sanitize();
        let dt = self
            .last_time
            .map_or(0.0, |previous| (frame_time - previous).clamp(0.0, 0.1));
        self.last_time = Some(frame_time);
        let instantaneous = wind.at(frame_time);
        self.advection[0] = wrap_advection(self.advection[0] + instantaneous[0] * dt);
        self.advection[1] = wrap_advection(self.advection[1] + instantaneous[1] * dt);
        let base_angle = wind.direction.to_radians();
        let base = [base_angle.cos() * wind.speed, base_angle.sin() * wind.speed];
        let (collision_uv, collision_size) = occlusion.map_or(
            ([0.0; 4], [0, 0, 0, u32::from(camera_underwater)]),
            |info| {
                (
                    [
                        info.min_xz[0],
                        info.min_xz[1],
                        info.inv_extent_xz[0],
                        info.inv_extent_xz[1],
                    ],
                    [info.width, info.height, 1, u32::from(camera_underwater)],
                )
            },
        );
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&WindVisualUniform {
                wind_time: [instantaneous[0], instantaneous[1], frame_time, self.strength.clamp(0.0, 1.0)],
                base_wind: [base[0], base[1], wind.speed, wind.gust],
                advection: [self.advection[0], self.advection[1], 0.0, 0.0],
                collision_uv,
                collision_size,
                volume: [VOLUME_RADIUS, VOLUME_HALF_HEIGHT, 1.0, 1.0],
            }),
        );
    }

    pub(crate) fn render<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bind_group: &'a wgpu::BindGroup,
    ) {
        if !self.active() {
            return;
        }
        let (Some(mote_pipeline), Some(wisp_pipeline)) =
            (self.mote_pipeline.as_ref(), self.wisp_pipeline.as_ref())
        else {
            return;
        };
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_pipeline(mote_pipeline);
        pass.draw(0..4, 0..MOTE_COUNT);
        pass.set_pipeline(wisp_pipeline);
        pass.draw(0..8, 0..WISP_COUNT);
    }
}

fn wrap_advection(value: f32) -> f32 {
    if value.is_finite() {
        value.rem_euclid(ADVECTION_WRAP)
    } else {
        0.0
    }
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    collision_view: &wgpu::TextureView,
    wind_noise_view: &wgpu::TextureView,
    wind_noise_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("JKA atmospheric wind visual bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(collision_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(wind_noise_view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(wind_noise_sampler) },
        ],
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    vertex_entry: &str,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
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
        multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::rain_layout::{offset_of_member, wgsl_struct_layout};
    use std::mem::{offset_of, size_of};

    #[test]
    fn wind_visual_uniform_matches_wgsl() {
        let (size, members) =
            wgsl_struct_layout(include_str!("../wind_visuals.wgsl"), "WindVisualUniform");
        assert_eq!(size as usize, size_of::<WindVisualUniform>());
        for (name, rust_offset) in [
            ("wind_time", offset_of!(WindVisualUniform, wind_time)),
            ("base_wind", offset_of!(WindVisualUniform, base_wind)),
            ("advection", offset_of!(WindVisualUniform, advection)),
            ("collision_uv", offset_of!(WindVisualUniform, collision_uv)),
            ("collision_size", offset_of!(WindVisualUniform, collision_size)),
            ("volume", offset_of!(WindVisualUniform, volume)),
        ] {
            assert_eq!(offset_of_member(&members, name) as usize, rust_offset, "{name}");
        }
    }
}

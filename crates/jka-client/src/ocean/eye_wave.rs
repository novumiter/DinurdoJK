//! Height of the *rasterized* ocean surface straight above/below the camera.
//!
//! The water mesh draws a vertex at `p0 + displacement(p0)`, so the surface over
//! a given XZ is found by inverting the horizontal displacement, not by reading
//! the height texture at that XZ. This one-thread compute pass does that exact
//! solve with the same three cascades the mesh vertex shader reads, and the CPU
//! picks the value up a few frames later through a single 16-byte readback.
//! The CPU uses it for everything the flat BSP plane used to decide: the
//! submerged classification, the underwater distance cull and the Godot/Blue
//! Water mesh choice. Waves move slowly relative to that latency.
use bytemuck::{Pod, Zeroable};

const SHADER: &str = r#"
struct Params {
    map_scales: array<vec4<f32>, 3>,
    ocean_info: vec4<f32>,
    swell: vec4<f32>,
    swell_motion: vec4<f32>,
    eye: vec4<f32>,
};
@group(0) @binding(0) var displacement: texture_2d_array<f32>;
@group(0) @binding(1) var wave_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var<storage, read_write> result: array<vec4<f32>, 1>;

// Mirrors sample_ocean_displacement() in bsp.wgsl for the live (nearest) mesh
// level: every cascade, the Z flip, the horizontal scale and the swell term.
fn displacement_m(world_m: vec2<f32>) -> vec3<f32> {
    var d = vec3<f32>(0.0);
    for (var i = 0u; i < 3u; i = i + 1u) {
        let scale = params.map_scales[i];
        let uv = world_m * vec2<f32>(1.0, -1.0) * scale.xy;
        d += textureSampleLevel(displacement, wave_sampler, uv, i32(i), 0.0).xyz
            * vec3<f32>(1.0, 1.0, -1.0) * scale.z;
    }
    d.x *= params.swell_motion.z;
    d.z *= params.swell_motion.z;
    let phase = dot(world_m, params.swell.xy) * params.swell.z - params.swell_motion.x * params.swell_motion.w;
    let horizontal = params.swell.w * params.swell_motion.y * cos(phase);
    d += vec3<f32>(params.swell.x * horizontal, params.swell.w * sin(phase), params.swell.y * horizontal);
    return d;
}

@compute @workgroup_size(1)
fn main() {
    let units_per_metre = max(params.ocean_info.z, 0.001);
    let target_m = params.eye.xy / units_per_metre;
    // Fixed point: find the undisplaced point q whose displaced position is over the eye.
    var q = target_m;
    var d = displacement_m(q);
    for (var i = 0u; i < 6u; i = i + 1u) {
        q = target_m - d.xz;
        d = displacement_m(q);
    }
    let residual_m = length(q + d.xz - target_m);
    // What ocean_optics.wgsl's jka_wave_height() reads here without the eye term:
    // the two large cascades at the un-inverted position.
    var naive_m = 0.0;
    for (var i = 0u; i < 2u; i = i + 1u) {
        let scale = params.map_scales[i];
        let uv = target_m * vec2<f32>(1.0, -1.0) * scale.xy;
        naive_m += textureSampleLevel(displacement, wave_sampler, uv, i32(i), 0.0).y * scale.z;
    }
    result[0] = vec4<f32>(d.y * units_per_metre, residual_m * units_per_metre, 1.0, naive_m * units_per_metre);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct EyeWaveParams {
    pub map_scales: [[f32; 4]; 3],
    pub ocean_info: [f32; 4],
    pub swell: [f32; 4],
    pub swell_motion: [f32; 4],
    pub eye: [f32; 4],
}

/// Largest acceptable fixed-point residual, in JKA units. A diverged solve (extreme
/// choppiness) is dropped rather than trusted.
const MAX_RESIDUAL_UNITS: f32 = 8.0;

enum Readback {
    Idle,
    Copied,
    Mapping(std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeWaveSample {
    pub slot: usize,
    /// Surface height above the pool's flat plane at the eye XZ, JKA units.
    pub offset: f32,
    /// `offset` minus what the optics shader's two-cascade lookup reads at the eye.
    pub correction: f32,
}

pub struct EyeWaveProbe {
    pipeline: wgpu::ComputePipeline,
    group: wgpu::BindGroup,
    params: wgpu::Buffer,
    result: wgpu::Buffer,
    readback: wgpu::Buffer,
    state: Readback,
    /// Pool the in-flight copy was dispatched for.
    in_flight_slot: usize,
    latest: Option<EyeWaveSample>,
}

impl EyeWaveProbe {
    pub fn new(device: &wgpu::Device, displacement: &wgpu::TextureView, sampler: &wgpu::Sampler) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ocean eye wave probe layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ocean eye wave probe"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Ocean eye wave probe"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Ocean eye wave probe"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ocean eye wave probe parameters"),
            size: std::mem::size_of::<EyeWaveParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let result = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ocean eye wave probe result"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ocean eye wave probe readback"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ocean eye wave probe"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(displacement) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: result.as_entire_binding() },
            ],
        });
        Self {
            pipeline,
            group,
            params,
            result,
            readback,
            state: Readback::Idle,
            in_flight_slot: 0,
            latest: None,
        }
    }

    /// Forgets the last sample (camera left the pool or the mode no longer needs it).
    pub fn clear(&mut self) {
        self.latest = None;
    }

    pub fn latest(&self, slot: usize) -> Option<EyeWaveSample> {
        self.latest.filter(|sample| sample.slot == slot)
    }

    /// Collects a finished readback, dispatches this frame's solve and, when no
    /// copy is outstanding, queues the next readback. No CPU wait anywhere.
    pub fn encode(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        params: &EyeWaveParams,
        slot: usize,
    ) {
        if matches!(self.state, Readback::Copied) {
            let (send, receive) = std::sync::mpsc::channel();
            self.readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
            self.state = Readback::Mapping(receive);
        }
        if let Readback::Mapping(receive) = &self.state {
            if let Ok(status) = receive.try_recv() {
                if status.is_ok() {
                    if let Ok(view) = self.readback.slice(..).get_mapped_range() {
                        let value = |i: usize| f32::from_le_bytes(view[i * 4..i * 4 + 4].try_into().unwrap());
                        let (offset, residual, valid, naive) = (value(0), value(1), value(2), value(3));
                        self.latest = (valid > 0.5
                            && offset.is_finite()
                            && naive.is_finite()
                            && residual.is_finite()
                            && residual <= MAX_RESIDUAL_UNITS)
                            .then_some(EyeWaveSample {
                                slot: self.in_flight_slot,
                                offset,
                                correction: offset - naive,
                            });
                    }
                    self.readback.unmap();
                }
                self.state = Readback::Idle;
            }
        }
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Ocean eye wave probe"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if matches!(self.state, Readback::Idle) {
            encoder.copy_buffer_to_buffer(&self.result, 0, &self.readback, 0, 16);
            self.in_flight_slot = slot;
            self.state = Readback::Copied;
        }
    }
}

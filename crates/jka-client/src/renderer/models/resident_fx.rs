//! Stable GPU-resident visual EFX sprites. This is deliberately a separate
//! resource group from Ghoul2 skinning, weather, RT receivers, and glass.
//! Neither presentation nor the worker performs GPU readback or CPU vertex
//! expansion once a resident alpha pipeline is ready.
use crate::renderer::{
    pipeline_hash, storage_buffer_entry, uniform_entry,
    DynamicModelAlphaMode, PipelineJobKey, PipelineJobManager,
};
use crate::renderer::FxGpuResidentFrame;
use super::pipelines::create_fx_sprite_pipeline;
use crate::fx::gpu_sparks::{GpuSparkClock, GpuSparkGpuState};
use std::sync::Arc;

pub(in crate::renderer) const RESIDENT_SPARK_CAP: u64 = 1800;
const INITIAL_SLOT_BYTES: u64 = 4096;

#[derive(Clone)]
pub(in crate::renderer) struct PreparedResidentFxDraw {
    pub entity_num: u16,
    pub first_instance: u32,
    pub instance_count: u32,
    pub texture_key: Option<Arc<str>>,
    pub alpha_mode: DynamicModelAlphaMode,
    pub authored_glow: bool,
}

pub(in crate::renderer) struct ResidentFxBackend {
    state_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_capacity: u64,
    clock_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: Option<wgpu::ShaderModule>,
    pipelines: [Option<wgpu::RenderPipeline>; DynamicModelAlphaMode::COUNT],
    ready_modes: Option<Arc<std::sync::atomic::AtomicU16>>,
}

impl ResidentFxBackend {
    pub fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("JKA resident FX state + draw-indirection layout"),
            entries: &[
                storage_buffer_entry(0, true, wgpu::ShaderStages::VERTEX),
                storage_buffer_entry(1, true, wgpu::ShaderStages::VERTEX),
                uniform_entry(2, wgpu::ShaderStages::VERTEX),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("JKA resident FX pipeline layout"),
            bind_group_layouts: &[Some(camera_layout), Some(texture_layout), Some(&layout)],
            immediate_size: 0,
        });
        let state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("JKA resident FX immutable spawn state"),
            size: RESIDENT_SPARK_CAP * std::mem::size_of::<GpuSparkGpuState>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("JKA resident FX material-stage slot indirection"),
            size: INITIAL_SLOT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let clock_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("JKA resident FX demo-aware clock"),
            size: std::mem::size_of::<GpuSparkClock>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = Self::create_bind_group(device, &layout, &state_buffer, &index_buffer, &clock_buffer);
        Self {
            state_buffer, index_buffer, clock_buffer, index_capacity: INITIAL_SLOT_BYTES,
            bind_group, layout, pipeline_layout, shader: None,
            pipelines: std::array::from_fn(|_| None), ready_modes: None,
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        state: &wgpu::Buffer,
        indices: &wgpu::Buffer,
        clock: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JKA resident FX storage bindings"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: state.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: clock.as_entire_binding() },
            ],
        })
    }

    pub fn rebuild(&mut self) {
        // Surface resize, MSAA, HDR, or specialized fog changes only invalidate
        // colour PSOs, never the resident particle slots or the FX clock.
        self.pipelines = std::array::from_fn(|_| None);
        if let Some(ready) = &self.ready_modes {
            ready.store(0, std::sync::atomic::Ordering::Release);
        }
    }

    pub fn ready(&self, mode: DynamicModelAlphaMode) -> bool {
        self.pipelines[mode.index()].is_some()
    }

    pub fn pipeline(&self, mode: DynamicModelAlphaMode) -> Option<&wgpu::RenderPipeline> {
        self.pipelines[mode.index()].as_ref()
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup { &self.bind_group }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &FxGpuResidentFrame,
        slots: &[u32],
    ) {
        // This mask is updated only from actual compiled pipeline objects.
        // The backend may survive session changes; the Arc need not.
        self.ready_modes = Some(Arc::clone(&frame.ready_modes));
        let mut available: u16 = 0;
        for mode in crate::renderer::DYNAMIC_MODEL_ALPHA_ORDER {
            if self.ready(mode) { available |= 1u16 << mode.index(); }
        }
        frame.ready_modes.store(available, std::sync::atomic::Ordering::Release);
        // A generation reset never needs a full zero-fill: dead states cannot
        // draw because draw slots are regenerated from the new generation.
        // Only compact birth deltas and 4-byte draw IDs traverse the CPU bus.
        let _ = frame.reset;
        if !frame.writes.is_empty() {
            // A hit can spawn dozens of adjacent slots at once. Avoid making
            // a separate queue.write_buffer submission for every 112-byte
            // particle, while never retransmitting old resident states.
            let mut pending: Vec<_> = frame.writes.iter()
                .filter(|(slot, _)| u64::from(*slot) < RESIDENT_SPARK_CAP)
                .collect();
            pending.sort_unstable_by_key(|(slot, _)| *slot);
            let stride = std::mem::size_of::<GpuSparkGpuState>() as u64;
            let mut run: Vec<GpuSparkGpuState> = Vec::new();
            let mut start = 0u32;
            let mut previous = 0u32;
            for (slot, state) in pending {
                if !run.is_empty() && *slot != previous.saturating_add(1) {
                    queue.write_buffer(&self.state_buffer, u64::from(start) * stride,
                        bytemuck::cast_slice(&run));
                    run.clear();
                }
                if run.is_empty() { start = *slot; }
                run.push(*state);
                previous = *slot;
            }
            if !run.is_empty() {
                queue.write_buffer(&self.state_buffer, u64::from(start) * stride,
                    bytemuck::cast_slice(&run));
            }
        }
        let clock = GpuSparkClock {
            now_ms: frame.time_ms as f32,
            show_saber_impact: u32::from(frame.show_saber_impact),
            reserved: [0; 2],
        };
        queue.write_buffer(&self.clock_buffer, 0, bytemuck::bytes_of(&clock));
        let bytes = bytemuck::cast_slice(slots);
        if bytes.len() as u64 > self.index_capacity {
            self.index_capacity = (bytes.len() as u64).next_power_of_two().max(INITIAL_SLOT_BYTES);
            self.index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("JKA resident FX material-stage slot indirection"),
                size: self.index_capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bind_group = Self::create_bind_group(
                device, &self.layout, &self.state_buffer, &self.index_buffer, &self.clock_buffer,
            );
        }
        if !bytes.is_empty() { queue.write_buffer(&self.index_buffer, 0, bytes); }
    }

    pub fn ensure_pipeline(
        &mut self,
        jobs: &mut PipelineJobManager,
        device: &wgpu::Device,
        mode: DynamicModelAlphaMode,
        surface_format: wgpu::TextureFormat,
        samples: u32,
        legacy_fog: bool,
        zero_alpha_discard: bool,
    ) {
        if self.ready(mode) { return; }
        if self.shader.is_none() {
            let key = PipelineJobKey::new("fx-resident-module", 0, 0);
            if let Some(shader) = jobs.take_ready::<wgpu::ShaderModule>(key) {
                self.shader = Some(shader);
            } else {
                let device = device.clone();
                jobs.request(key, "GPU resident FX shader module", move || {
                    device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("JKA resident spark visual shader"),
                        source: wgpu::ShaderSource::Wgsl(
                            include_str!("../../fx_resident_sprite.wgsl").into(),
                        ),
                    })
                });
                return;
            }
        }
        let config = pipeline_hash(&(surface_format, samples, legacy_fog, zero_alpha_discard));
        let key = PipelineJobKey::new("fx-resident-colour", mode.index() as u32, config);
        if let Some(pipeline) = jobs.take_ready::<wgpu::RenderPipeline>(key) {
            self.pipelines[mode.index()] = Some(pipeline);
            return;
        }
        let layout = self.pipeline_layout.clone();
        let shader = self.shader.as_ref().expect("resident shader ready").clone();
        let device = device.clone();
        jobs.request(key, "GPU resident FX colour variant", move || {
            create_fx_sprite_pipeline(
                &device, &layout, &shader, surface_format, samples,
                mode, legacy_fog, zero_alpha_discard, false, true,
            )
        });
    }
}

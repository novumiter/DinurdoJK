//! Opt-in sparse coastal integration. Geometry stays in JKA XY; solver state
//! uses metres relative to the still-water level. Positive faces have one owner
//! even across tiles. PVS affects drawing only, never fluid connectivity.
use super::{appearance::PromotedOcean, OceanRenderUniform, OCEAN_UNITS_PER_METER};
use bytemuck::{Pod, Zeroable};
use jka_assets::bsp::bathymetry::{Atlas, Boundary, Class};
use std::{collections::HashMap, sync::Arc};
use wgpu::util::DeviceExt;

const NONE: u32 = u32::MAX;
const MAX_CELLS: usize = 65_536;
const MAX_BAKED_CELLS: usize = 262_144;
const MAX_STEPS: usize = 8;

#[cfg(test)]
#[path = "coastal_tests.rs"]
mod tests;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RenderParams {
    pub grid: [f32; 4],    // spacing units, tile axis, level units, promoted slot
    pub control: [u32; 4], // enabled, hash mask, cell count, reserved
    pub limits: [f32; 4],  // presentation interpolation, reserved
}

pub struct RenderResources {
    pub params: wgpu::Buffer,
    pub lookup: wgpu::Buffer,
    pub surface: wgpu::Buffer,
}

impl RenderResources {
    pub fn inert(device: &wgpu::Device) -> Arc<Self> {
        Arc::new(Self {
            params: buffer(
                device,
                "Coastal inactive parameters",
                bytemuck::bytes_of(&RenderParams::zeroed()),
                wgpu::BufferUsages::UNIFORM,
            ),
            lookup: buffer(
                device,
                "Coastal inactive lookup",
                &[0; 16],
                wgpu::BufferUsages::STORAGE,
            ),
            surface: buffer(
                device,
                "Coastal inactive surface",
                &[0; 80],
                wgpu::BufferUsages::STORAGE,
            ),
        })
    }
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents,
        usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
    })
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct CellGpu {
    pub neighbours: [u32; 4], // west, east, south, north; NONE = closed
    pub metric: [f32; 4],     // bed metres, positive-X crest, positive-Y crest, forcing rate
    pub position: [f32; 4],   // JKA x/y units, active, presentation weight
}

pub struct Prepared {
    pub slot: usize,
    pub level: f32,
    pub spacing: f32,
    pub cells: Vec<CellGpu>,
    pub depth: Vec<f32>,
    pub lookup: Vec<[i32; 4]>,
    pub max_depth: f32,
    pub active: usize,
    pub shoreline: usize,
}

fn hash(x: i32, y: i32) -> u32 {
    (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)
}

/// Adapt supported shallow cells, dry banks, and a one-cell FFT interface ring.
/// Intermediate/deep water beyond that ring continues to use the FFT renderer.
pub fn prepare(atlas: &Atlas, ocean: &PromotedOcean) -> Result<Prepared, String> {
    if let Some(band) = atlas
        .shoreline_bands
        .iter()
        .find(|b| b.slot as usize == ocean.index)
    {
        if ocean.heuristic.wavelength.max(1.0) * 0.05 > band.max_depth + 0.001 {
            return Err(format!("wavelength override requires deeper than the {:.1}u screened shoreline band; retain FFT", band.max_depth));
        }
    }
    let grids: Vec<_> = atlas
        .grids
        .iter()
        .filter(|g| g.slot as usize == ocean.index)
        .collect();
    if grids.is_empty() {
        return Err("no supported bathymetry for this pool".into());
    }
    let first = grids[0];
    if grids.iter().any(|g| {
        g.size != [16, 16] || g.cell_size != first.cell_size || (g.level - first.level).abs() > 0.01
    }) {
        return Err("coastal integration requires matching 16x16 footprint tiles".into());
    }
    let count = grids.len() * 256;
    if count > MAX_BAKED_CELLS {
        return Err(format!(
            "{count} tiled cells exceed the static {MAX_BAKED_CELLS}-cell budget"
        ));
    }
    let spacing = first.cell_size;
    let mut locations = HashMap::with_capacity(count);
    let mut sources = Vec::with_capacity(count);
    let mut coordinates = Vec::with_capacity(count);
    for (tile, g) in grids.iter().enumerate() {
        for y in 0..16usize {
            for x in 0..16usize {
                let coordinate = [
                    (g.origin[0] / spacing).round() as i32 + x as i32,
                    (g.origin[1] / spacing).round() as i32 + y as i32,
                ];
                let k = tile * 256 + y * 16 + x;
                if locations.insert(coordinate, k).is_some() {
                    return Err("overlapping coastal allocation tiles".into());
                }
                coordinates.push(coordinate);
                sources.push((
                    g.cells[y * 16 + x],
                    [
                        g.faces_x[y * 17 + x].boundary,
                        g.faces_x[y * 17 + x + 1].boundary,
                        g.faces_y[y * 16 + x].boundary,
                        g.faces_y[(y + 1) * 16 + x].boundary,
                    ],
                    [
                        g.faces_x[y * 17 + x + 1].crest,
                        g.faces_y[(y + 1) * 16 + x].crest,
                    ],
                ));
            }
        }
    }
    let adjacent = |k: usize, axis: usize| {
        let [x, y] = coordinates[k];
        let q = match axis {
            0 => [x - 1, y],
            1 => [x + 1, y],
            2 => [x, y - 1],
            _ => [x, y + 1],
        };
        locations.get(&q).copied()
    };
    let safe = |k: usize| {
        let c = sources[k].0;
        matches!(c.class, Class::Wet | Class::Dry) && c.bed.is_finite()
            && c.bed_min.is_finite() && c.bed_max.is_finite()
            // No unknown ceiling/stacked cells have either accepted class.
            && (c.class == Class::Dry || c.coverage == 5)
    };
    let shallow: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(k, (c, _, _))| {
            safe(k)
                && c.class == Class::Wet
                && c.depth / ocean.heuristic.wavelength.max(1.0) <= 0.05
        })
        .collect();
    let mut active = shallow.clone();
    // Retain only finite dry terrain adjacent to a sampled bank. Do not mark
    // all dry land in a tile active just because a nearby pool exists.
    for k in 0..count {
        if !safe(k) {
            continue;
        }
        if sources[k].0.class == Class::Dry {
            active[k] = (0..4).any(|axis| {
                sources[k].1[axis] == Boundary::Bank
                    && adjacent(k, axis).is_some_and(|q| shallow[q])
            });
        } else if !shallow[k] {
            active[k] = (0..4).any(|axis| {
                sources[k].1[axis] == Boundary::Open
                    && adjacent(k, axis).is_some_and(|q| shallow[q])
            });
        }
    }
    let shoreline = (0..count)
        .filter(|&k| shallow[k] && sources[k].1.contains(&Boundary::Bank))
        .count();
    if shoreline == 0 {
        return Err("no resolved shallow bank cells (h/L <= 0.05) in this pool".into());
    }
    // Allocate GPU storage only for tiles touched by shallow physics, dry
    // banks or the FFT forcing ring. Keep complete tiles for shader lookup,
    // but never charge unused offshore bathymetry against the live budget.
    let retained_tiles: Vec<_> = (0..grids.len())
        .filter(|&tile| active[tile * 256..(tile + 1) * 256].iter().any(|&v| v))
        .collect();
    let live_count = retained_tiles.len() * 256;
    if live_count > MAX_CELLS {
        return Err(format!("{live_count} shallow-band tiled cells exceed the live {MAX_CELLS}-cell budget; retain FFT"));
    }
    let mut remap = vec![NONE; count];
    for (packed, &tile) in retained_tiles.iter().enumerate() {
        for local in 0..256 {
            remap[tile * 256 + local] = (packed * 256 + local) as u32;
        }
    }
    let mut cells = Vec::with_capacity(live_count);
    let mut depth = Vec::with_capacity(live_count);
    let mut max_depth = 0.0f32;
    for k in 0..count {
        if remap[k] == NONE {
            continue;
        }
        let (c, boundaries, crests) = sources[k];
        let bed = if active[k] {
            (c.bed - first.level) / OCEAN_UNITS_PER_METER
        } else {
            0.0
        };
        let h = if active[k] { (-bed).max(0.0) } else { 0.0 };
        max_depth = max_depth.max(h);
        let neighbours = [0, 1, 2, 3].map(|axis| {
            adjacent(k, axis)
                .filter(|&q| {
                    active[k]
                        && active[q]
                        && matches!(boundaries[axis], Boundary::Open | Boundary::Bank)
                        && matches!(sources[q].1[axis ^ 1], Boundary::Open | Boundary::Bank)
                        && (if axis & 1 == 1 {
                            crests[axis / 2]
                        } else {
                            sources[q].2[axis / 2]
                        })
                        .is_finite()
                })
                .map_or(NONE, |q| remap[q])
        });
        let interface = active[k]
            && c.class == Class::Wet
            && (!shallow[k]
                || (0..4).any(|axis| {
                    boundaries[axis] == Boundary::Open
                        && adjacent(k, axis).is_some_and(|q| {
                            safe(q) && !active[q] && sources[q].0.class == Class::Wet
                        })
                }));
        let crest = [0, 1].map(|axis| {
            if crests[axis].is_finite() {
                (crests[axis] - first.level) / OCEAN_UNITS_PER_METER
            } else {
                bed
            }
        });
        let xy = coordinates[k].map(|v| (v as f32 + 0.5) * spacing);
        cells.push(CellGpu {
            neighbours,
            metric: [bed, crest[0], crest[1], if interface { 9.0 } else { 0.0 }],
            position: [
                xy[0],
                xy[1],
                if active[k] { 1.0 } else { 0.0 },
                if active[k] && (shallow[k] || c.class == Class::Dry) {
                    1.0
                } else {
                    0.0
                },
            ],
        });
        depth.push(h);
    }
    if !cells.iter().any(|c| c.metric[3] > 0.0) {
        return Err("shallow bank has no resolved FFT interface; retain FFT until a forcing boundary is available".into());
    }
    let mut lookup = vec![[0i32; 4]; (retained_tiles.len() * 2).next_power_of_two().max(4)];
    let mask = lookup.len() - 1;
    for (tile, &original) in retained_tiles.iter().enumerate() {
        let g = grids[original];
        let x = (g.origin[0] / (spacing * 16.0)).round() as i32;
        let y = (g.origin[1] / (spacing * 16.0)).round() as i32;
        let mut index = hash(x, y) as usize & mask;
        while lookup[index][3] != 0 {
            index = (index + 1) & mask;
        }
        lookup[index] = [x, y, (tile * 256) as i32, 1];
    }
    Ok(Prepared {
        slot: ocean.index,
        level: first.level,
        spacing,
        cells,
        depth,
        lookup,
        max_depth,
        active: active.iter().filter(|&&v| v).count(),
        shoreline,
    })
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StepParams {
    count: u32,
    dt: f32,
    dx: f32,
    depth_bound: f32,
}

pub struct Gpu {
    pub render: Arc<RenderResources>,
    pub slot: usize,
    pub summary: String,
    pipelines: [wgpu::ComputePipeline; 5],
    groups: [wgpu::BindGroup; 2],
    wave: wgpu::Buffer,
    front: usize,
    count: u32,
    dt: f32,
    accumulator: f32,
    pub dropped_time: f64,
    readback: wgpu::Buffer,
    copy_submitted: bool,
    pending: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    health_timer: f32,
    pub failed: bool,
    #[cfg(test)]
    depths: [wgpu::Buffer; 2],
    #[cfg(test)]
    state: wgpu::Buffer,
    #[cfg(test)]
    limiter: wgpu::Buffer,
    #[cfg(test)]
    limited: wgpu::Buffer,
}

impl Gpu {
    pub fn new(
        device: &wgpu::Device,
        prepared: Prepared,
        fft: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        amplitude: f32,
    ) -> Self {
        let cell_count = prepared.cells.len() as u32;
        let active_indices: Vec<u32> = prepared
            .cells
            .iter()
            .enumerate()
            .filter_map(|(k, c)| (c.position[2] > 0.5).then_some(k as u32))
            .collect();
        let count = active_indices.len() as u32;
        let depth_bound =
            (prepared.max_depth + amplitude.abs() / OCEAN_UNITS_PER_METER * 2.0 + 0.25).max(1.0)
                * 4.0;
        let dx = prepared.spacing / OCEAN_UNITS_PER_METER;
        let dt = (0.42 * dx / (2.0 * (5.0 + (9.81 * depth_bound).sqrt()))).min(1.0 / 60.0);
        let render_params = RenderParams {
            grid: [prepared.spacing, 16.0, prepared.level, prepared.slot as f32],
            control: [1, prepared.lookup.len() as u32 - 1, cell_count, 0],
            limits: [0.0; 4],
        };
        // Two presentation levels per cell allow interpolation at render FPS.
        let initial_cells: Vec<[f32; 16]> = prepared
            .cells
            .iter()
            .zip(&prepared.depth)
            .map(|(c, &h)| {
                let sample = [
                    (c.metric[0] + h) * OCEAN_UNITS_PER_METER,
                    h * OCEAN_UNITS_PER_METER,
                    c.metric[0] * OCEAN_UNITS_PER_METER,
                    c.position[3],
                ];
                [
                    sample[0],
                    sample[1],
                    sample[2],
                    sample[3],
                    sample[0],
                    sample[1],
                    sample[2],
                    sample[3],
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    h,
                    0.0,
                    c.neighbours
                        .iter()
                        .enumerate()
                        .fold(0u32, |flags, (axis, &q)| {
                            flags | if q != NONE { 1 << axis } else { 0 }
                        }) as f32,
                ]
            })
            .collect();
        let mut initial = vec![0u8; 16];
        initial.extend_from_slice(bytemuck::cast_slice(&initial_cells));
        let render = Arc::new(RenderResources {
            params: buffer(
                device,
                "Coastal render parameters",
                bytemuck::bytes_of(&render_params),
                wgpu::BufferUsages::UNIFORM,
            ),
            lookup: buffer(
                device,
                "Coastal sparse tile hash",
                bytemuck::cast_slice(&prepared.lookup),
                wgpu::BufferUsages::STORAGE,
            ),
            surface: buffer(
                device,
                "Coastal reconstructed surface",
                &initial,
                wgpu::BufferUsages::STORAGE,
            ),
        });
        let params = buffer(
            device,
            "Coastal step parameters",
            bytemuck::bytes_of(&StepParams {
                count,
                dt,
                dx,
                depth_bound,
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let wave = buffer(
            device,
            "Coastal incoming FFT parameters",
            bytemuck::bytes_of(&OceanRenderUniform::zeroed()),
            wgpu::BufferUsages::UNIFORM,
        );
        let geometry = buffer(
            device,
            "Coastal geometry and tile neighbours",
            bytemuck::cast_slice(&prepared.cells),
            wgpu::BufferUsages::STORAGE,
        );
        let dispatch = buffer(
            device,
            "Coastal compact active cell list",
            bytemuck::cast_slice(&active_indices),
            wgpu::BufferUsages::STORAGE,
        );
        let depths = [0, 1].map(|_| {
            buffer(
                device,
                "Coastal depth ping-pong",
                bytemuck::cast_slice(&prepared.depth),
                wgpu::BufferUsages::STORAGE,
            )
        });
        let state = buffer(
            device,
            "Coastal face velocity and raw flux",
            &vec![0; cell_count as usize * 16],
            wgpu::BufferUsages::STORAGE,
        );
        let limiter = buffer(
            device,
            "Coastal donor positivity limit",
            bytemuck::cast_slice(&vec![1.0f32; cell_count as usize]),
            wgpu::BufferUsages::STORAGE,
        );
        let limited = buffer(
            device,
            "Coastal shared limited flux",
            &vec![0; cell_count as usize * 8],
            wgpu::BufferUsages::STORAGE,
        );
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Coastal compute layout"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform(10),
                storage(11, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Coastal ordered solver"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Coastal sparse conservative solver"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../coastal_compute.wgsl").into()),
        });
        let pipelines = [
            "pressure",
            "limit_outgoing",
            "limit_faces",
            "divergence",
            "reconstruct",
        ]
        .map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let groups = [0, 1].map(|front| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Coastal step bindings"),
                layout: &layout,
                entries: &[
                    super::bg_buffer(0, &params),
                    super::bg_buffer(1, &geometry),
                    super::bg_buffer(2, &depths[front]),
                    super::bg_buffer(3, &depths[front ^ 1]),
                    super::bg_buffer(4, &state),
                    super::bg_buffer(5, &limiter),
                    super::bg_buffer(6, &limited),
                    super::bg_buffer(7, &render.surface),
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(fft),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    super::bg_buffer(10, &wave),
                    super::bg_buffer(11, &dispatch),
                ],
            })
        });
        let summary=format!("pool {}: {} shallow bank cells, {} active / {cell_count} tiled cells; GPU fixed dt {:.3} ms; maximum {MAX_STEPS} steps/frame",prepared.slot+1,prepared.shoreline,prepared.active,dt*1000.0);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Coastal asynchronous safety flag"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            render,
            slot: prepared.slot,
            summary,
            pipelines,
            groups,
            wave,
            front: 0,
            count,
            dt,
            accumulator: 0.0,
            dropped_time: 0.0,
            readback,
            copy_submitted: false,
            pending: None,
            health_timer: 0.0,
            failed: false,
            #[cfg(test)]
            depths,
            #[cfg(test)]
            state,
            #[cfg(test)]
            limiter,
            #[cfg(test)]
            limited,
        }
    }

    pub fn encode(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        delta: f32,
        wave: &OceanRenderUniform,
    ) {
        queue.write_buffer(&self.wave, 0, bytemuck::bytes_of(wave));
        // Map a copy submitted by the previous frame. There is no CPU wait;
        // the shader has already latched FFT fallback on a numerical failure.
        if self.copy_submitted && self.pending.is_none() {
            let (send, receive) = std::sync::mpsc::channel();
            self.readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = send.send(result);
                });
            self.pending = Some(receive);
            self.copy_submitted = false;
        }
        if let Some(Ok(status)) = self.pending.as_ref().map(|receive| receive.try_recv()) {
            self.pending = None;
            if status.is_ok() {
                let view = self.readback.slice(..).get_mapped_range();
                self.failed = u32::from_le_bytes(view[0..4].try_into().unwrap()) != 0;
                drop(view);
                self.readback.unmap();
                if self.failed {
                    queue.write_buffer(&self.render.params, 16, bytemuck::cast_slice(&[0u32]));
                    eprintln!("COASTAL LIVE: pool {} exceeded its numerical safety bound; FFT fallback latched (toggle r_coastal off/on to restart)",self.slot+1);
                }
            }
        }
        if self.failed {
            return;
        }
        let bounded = if delta.is_finite() {
            delta.clamp(0.0, 0.1)
        } else {
            0.0
        };
        self.dropped_time += f64::from((delta - bounded).max(0.0));
        self.accumulator += bounded;
        let steps = (self.accumulator / self.dt).floor() as usize;
        let run = steps.min(MAX_STEPS);
        for _ in 0..run {
            for stage in 0..4 {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Coastal conservative step"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipelines[stage]);
                pass.set_bind_group(0, &self.groups[self.front], &[]);
                pass.dispatch_workgroups(self.count.div_ceil(64), 1, 1);
            }
            self.front ^= 1;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Coastal wet/dry reconstruction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines[4]);
            pass.set_bind_group(0, &self.groups[self.front], &[]);
            pass.dispatch_workgroups(self.count.div_ceil(64), 1, 1);
        }
        self.accumulator -= steps as f32 * self.dt;
        self.dropped_time += (steps - run) as f64 * f64::from(self.dt);
        self.health_timer += bounded;
        if self.health_timer >= 1.0 && self.pending.is_none() && !self.copy_submitted {
            encoder.copy_buffer_to_buffer(&self.render.surface, 0, &self.readback, 0, 16);
            self.copy_submitted = true;
            self.health_timer = 0.0;
        }
        // Only the interpolation lane changes; identity remains immutable.
        queue.write_buffer(
            &self.render.params,
            32,
            bytemuck::cast_slice(&[(self.accumulator / self.dt).clamp(0.0, 1.0), 0.0, 0.0, 0.0]),
        );
    }
}

//! Environment water.
use crate::renderer::{PvsMode, Renderer, WorldBatch};

impl Renderer {
    pub(in crate::renderer) fn set_grass_enabled(&mut self, enabled: bool) {
        self.grass_enabled = enabled;
    }

    pub(in crate::renderer) fn set_grass_precompute(&mut self, enabled: bool) {
        self.grass_precompute_enabled = enabled;
    }

    pub(in crate::renderer) fn set_grass_mid_lod(&mut self, enabled: bool) {
        self.grass_mid_lod_enabled = enabled;
    }

    pub(in crate::renderer) fn set_grass_front_to_back(&mut self, enabled: bool) {
        self.grass_front_to_back_enabled = enabled;
    }

    /// The index range promoted water draws instead of its authored face, or
    /// `None` when the ocean is off or this world carries no clipmap.
    pub(in crate::renderer) fn ocean_clipmap_quality(&self) -> Option<usize> {
        self.ocean_enabled
            .then(|| usize::from(self.ocean_settings.mesh_quality.min(1)))
    }

    /// Keeps promoted-water geometry in sync with the live pool amplitude,
    /// camera distance, footprint and exact BSP visibility. Indices change only
    /// at an LOD threshold, a snapped-lattice crossing, or a BSP cluster change;
    /// ordinary camera motion remains a comparison-only path.
    pub(in crate::renderer) fn update_promoted_ocean_clipmap_lod(&mut self, camera: [f32; 3]) {
        if !self.ocean_enabled {
            return;
        }
        let quality = usize::from(self.ocean_settings.mesh_quality.min(1));
        let overrides = &self.promoted_ocean_overrides;
        let queue = &self.queue;
        let Some(world) = self.world.as_mut() else { return; };
        if world.runtime_ocean_clipmaps.is_empty() {
            return;
        }

        let descriptors = &world.promoted_oceans;
        let masks = &world.ocean_masks;
        let global_spacing = world.ocean_coarsest_spacing;
        let pvs_cluster = if matches!(self.pvs_mode, PvsMode::Off) {
            None
        } else {
            world
                .visibility
                .as_ref()
                .and_then(|visibility| {
                    visibility.cluster_at(crate::scene::jka_position(camera))
                })
        };
        let pvs_visible_batches = pvs_cluster.and_then(|cluster| {
            world.full_visible_batches_by_cluster.get(cluster)
        });
        let pvs_footprints = &world.ocean_pvs_footprints;
        let mut pvs_tiles = None;
        let index_buffer = &world.index_buffer;
        let mut changed_ranges: Option<Vec<(u8, usize, std::ops::Range<u32>)>> = None;
        for runtime in &mut world.runtime_ocean_clipmaps {
            if runtime.quality != quality {
                continue;
            }
            let Some(descriptor) = descriptors.iter().find(|p| p.index == runtime.slot as usize) else {
                continue;
            };
            let amplitude = overrides
                .iter()
                .find(|(slot, _)| *slot == descriptor.index)
                .map_or(descriptor.heuristic.amplitude, |(_, settings)| settings.authored.amplitude);
            let amplitude = if amplitude.is_finite() {
                amplitude.max(crate::ocean::appearance::MIN_PROMOTED_AMPLITUDE)
            } else {
                descriptor
                    .heuristic
                    .amplitude
                    .max(crate::ocean::appearance::MIN_PROMOTED_AMPLITUDE)
            };
            let half_cells = crate::ocean::promoted_clipmap_half_cells(amplitude);
            let distance = crate::ocean::promoted_footprint_distance(
                camera,
                descriptor.surface,
                masks,
                descriptor.index,
            );
            let minimum_level = runtime
                .layout
                .minimum_level_for_distance(half_cells, distance);
            let outer_spacing = runtime.layout.outer_spacing_for_half_cells(half_cells);
            let centre = crate::ocean::promoted_footprint_clipmap_centre(
                camera,
                descriptor.surface,
                global_spacing,
                outer_spacing,
                masks,
                descriptor.index,
            );
            if half_cells == runtime.current_half_cells
                && minimum_level == runtime.current_minimum_level
                && centre == runtime.current_centre
                && pvs_cluster == runtime.current_pvs_cluster
            {
                continue;
            }

            let mask_slot = masks.is_masked(descriptor.index).then_some(descriptor.index);
            if pvs_tiles.is_none() {
                pvs_tiles = pvs_visible_batches
                    .map(|visible_batches| pvs_footprints.visible_tiles(visible_batches));
            }
            let visible_tiles = pvs_tiles
                .as_ref()
                .and_then(|surfaces| surfaces[descriptor.index].as_ref());
            let mut indices = runtime.layout.indices_for_footprint(
                half_cells,
                minimum_level,
                centre,
                descriptor.surface,
                masks,
                mask_slot,
                visible_tiles,
            );
            debug_assert!(indices.len() <= runtime.reserved_range.len());
            for index in &mut indices {
                *index = runtime.vertex_base.saturating_add(*index);
            }
            let byte_offset = u64::from(runtime.reserved_range.start)
                * std::mem::size_of::<u32>() as u64;
            if !indices.is_empty() {
                queue.write_buffer(index_buffer, byte_offset, bytemuck::cast_slice(&indices));
            }
            let end = runtime
                .reserved_range
                .start
                .saturating_add(u32::try_from(indices.len()).unwrap_or(u32::MAX));
            runtime.active_range = runtime.reserved_range.start..end;
            runtime.current_half_cells = half_cells;
            runtime.current_minimum_level = minimum_level;
            runtime.current_centre = centre;
            runtime.current_pvs_cluster = pvs_cluster;
            changed_ranges
                .get_or_insert_with(Vec::new)
                .push((runtime.slot, runtime.quality, runtime.active_range.clone()));
        }

        let centres=std::array::from_fn(|slot| {
            world.runtime_ocean_clipmaps.iter().find(|r|r.quality==quality && r.slot as usize==slot)
                .map(|r|r.current_centre).filter(|c|c.iter().all(|v|v.is_finite()))
        });
        if let Some(ocean)=self.ocean.as_mut() {
            ocean.set_promoted_clipmap_centres(queue,centres);
        }
        let Some(changed_ranges) = changed_ranges else { return; };
        let apply = |batches: &mut [WorldBatch]| {
            for batch in batches {
                let Some(ranges) = batch.ocean_clipmap.as_mut() else { continue; };
                for (slot, quality, range) in &changed_ranges {
                    if batch.ocean_clipmap_id == *slot {
                        ranges[*quality] = range.clone();
                    }
                }
            }
        };
        apply(&mut world.coarse_batches);
        apply(&mut world.full_batches);
        apply(&mut world.auto4_batches);
    }

    pub(in crate::renderer) fn set_ocean_settings(
        &mut self,
        settings: crate::ocean::OceanSettings,
    ) {
        let mut settings = settings.sanitize();
        // Atmospheric wind is renderer-global. Ocean-specific settings may
        // arrive through their legacy API, but they cannot fork the wind field.
        settings.wind = self.weather_wind;
        let recreate = self
            .ocean
            .as_ref()
            .is_some_and(|ocean| ocean.map_size() != settings.map_size);
        self.ocean_settings = settings;
        self.ocean_interaction.set_falloff(settings);
        self.ocean_interaction.configure(&self.device, &self.queue,
            self.ocean_enabled && settings.interaction, settings.interaction_strength,
            settings.interaction_debug, settings.interaction_splash_mode);
        if self.ocean_enabled {
            if recreate {
                let mut ocean = crate::ocean::OceanGpu::new_with_interaction(
                    &self.device,
                    &self.queue,
                    &self.ocean_layout,
                    &self.ocean_spray_albedo.view,
                    self.ocean_settings,
                    &self.ocean_interaction.render,
                );
                ocean.set_enabled(&self.queue, true);
                self.ocean = Some(ocean);
            } else if let Some(ocean) = &mut self.ocean {
                ocean.set_settings(&self.queue, self.ocean_settings);
            }
        }
        let descriptors = self.authored_ocean_definitions.clone();
        self.set_authored_oceans(descriptors);
    }

    pub(in crate::renderer) fn set_authored_oceans(
        &mut self,
        descriptors: Vec<crate::ocean::authoring::AuthoredOcean>,
    ) {
        self.authored_ocean_definitions = descriptors.clone();
        if !self.ocean_enabled {
            self.authored_oceans.clear();
            return;
        }
        let mut previous = std::mem::take(&mut self.authored_oceans);
        for descriptor in descriptors {
            let mut settings = self.ocean_settings;
            settings.authored = descriptor.waves;
            // Wind is global atmospheric state, not per-ocean state. The parsed
            // descriptor retains its original weather metadata for compatibility,
            // but every rendered ocean consumes the same canonical wind.
            settings.wind = self.weather_wind;
            settings.bounds = Some(crate::ocean::OceanSurface {
                plane_height: descriptor.height,
                minimum: [descriptor.mins[0], -descriptor.maxs[1]],
                maximum: [descriptor.maxs[0], -descriptor.mins[1]],
            });
            let mut gpu = if let Some(index) = previous
                .iter()
                .position(|(a, g)| a.index == descriptor.index && g.map_size() == settings.map_size)
            {
                previous.swap_remove(index).1
            } else {
                crate::ocean::OceanGpu::new_with_interaction(
                    &self.device,
                    &self.queue,
                    &self.ocean_layout,
                    &self.ocean_spray_albedo.view,
                    settings,
                    &self.ocean_interaction.render,
                )
            };
            gpu.set_settings(&self.queue, settings);
            gpu.set_enabled(&self.queue, self.ocean_enabled);
            self.authored_oceans.push((descriptor, gpu));
        }
    }

    pub(in crate::renderer) fn ocean_bind_group_for<'a>(
        batch: &WorldBatch,
        authored_oceans: &'a [(
            crate::ocean::authoring::AuthoredOcean,
            crate::ocean::OceanGpu,
        )],
        ocean: &'a Option<crate::ocean::OceanGpu>,
        inert: &'a wgpu::BindGroup,
    ) -> &'a wgpu::BindGroup {
        if batch.source.water_primary {
            let centre = std::array::from_fn(|i| (batch.bounds_min[i] + batch.bounds_max[i]) * 0.5);
            if let Some((_, gpu)) = authored_oceans.iter().find(|(a, _)| {
                batch
                    .source
                    .authored_ocean
                    .map_or_else(|| a.contains_render_point(centre), |id| a.index == id)
            }) {
                return &gpu.render_bind_group;
            }
        }
        ocean
            .as_ref()
            .map(|o| o.render_bind_group_for(batch.ocean_clipmap_id as usize))
            .unwrap_or(inert)
    }

    pub(in crate::renderer) fn set_ocean_enabled(&mut self, enabled: bool) {
        if self.ocean_enabled == enabled {
            return;
        }
        self.ocean_enabled = enabled;
        self.ocean_interaction.set_falloff(self.ocean_settings);
        self.ocean_interaction.configure(&self.device, &self.queue,
            enabled && self.ocean_settings.interaction, self.ocean_settings.interaction_strength,
            self.ocean_settings.interaction_debug, self.ocean_settings.interaction_splash_mode);
        self.set_authored_oceans(self.authored_ocean_definitions.clone());
        if enabled {
            if self.ocean.is_none() {
                let mut ocean = crate::ocean::OceanGpu::new_with_interaction(
                    &self.device,
                    &self.queue,
                    &self.ocean_layout,
                    &self.ocean_spray_albedo.view,
                    self.ocean_settings,
                    &self.ocean_interaction.render,
                );
                ocean.set_enabled(&self.queue, true);
                self.ocean = Some(ocean);
            } else if let Some(ocean) = &mut self.ocean {
                ocean.set_enabled(&self.queue, true);
            }
        } else {
            // Release the large FFT textures/buffers when stock JKA water is selected.
            self.ocean = None;
            self.ocean_optics = crate::ocean::optics::Resources::new(
                &self.device,
                &self.ocean_optics_layout,
                &self.ocean_layout,
                (1, 1, self.msaa_samples, self.scene_format()),
            );
        }
        self.rebuild_planar_reflection_resources();
        self.rebuild_frame_plan();
        self.activate_world_pipeline_variant();
    }
}

/// Upper bound on distinct water planes that get their own clipmap. Each one
/// costs a fixed mesh, so a map with many separate pools keeps the rest on
/// their authored faces rather than growing the vertex buffer without limit.
pub(in crate::renderer) const MAX_OCEAN_SURFACES: usize = crate::ocean::OCEAN_MAX_SURFACES;

pub(in crate::renderer) use crate::ocean::surfaces::{water_surface_extent, ocean_surface_key, is_upward_water_face};

/// Ground-truth "is any ocean on screen" from the GPU.
///
/// Every CPU-side answer to that question was wrong on real maps: a promoted
/// surface's bounding box routinely surrounds the camera, so a frustum test
/// reports "visible" while the clipmap rasterizes almost nothing (measured:
/// `ocean_surface` at 0.04ms with `reject_frustum=0`). The shape mask and world
/// occluders only discard later, in the fragment stage, where no CPU test can
/// see them. An occlusion query counts the water samples that actually survived
/// depth, which is exactly the question, and it costs one query per frame.
///
/// The answer necessarily lags by a frame or two. Consumers must therefore be
/// things that tolerate stale visibility: freezing a simulation nobody is
/// looking at, and skipping a capture nobody is sampling.
pub(in crate::renderer) struct OceanVisibilitySlot {
    readback_buffer: wgpu::Buffer,
    pending: Option<crate::renderer::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

pub(in crate::renderer) struct OceanVisibility {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    slots: Vec<OceanVisibilitySlot>,
    current: Option<usize>,
    encoded: bool,
    /// Samples that passed depth in the most recently read-back ocean pass.
    pub(in crate::renderer) latest_samples: u64,
    /// No readback has completed yet. Until one does, callers must assume the
    /// ocean is visible, or the first frames on a map would skip work wrongly.
    pub(in crate::renderer) primed: bool,
}

const OCEAN_VISIBILITY_SLOTS: usize = 3;
const OCEAN_VISIBILITY_STRIDE: usize = 256;

impl OceanVisibility {
    pub(in crate::renderer) fn new(device: &wgpu::Device) -> Self {
        Self {
            query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("JKA ocean occlusion"),
                ty: wgpu::QueryType::Occlusion,
                count: OCEAN_VISIBILITY_SLOTS as u32,
            }),
            resolve_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("JKA ocean occlusion resolve"),
                // wgpu requires a 256-byte aligned destination offset for
                // resolve_query_set, so each slot gets its own aligned stride
                // even though a single occlusion result is 8 bytes.
                size: (OCEAN_VISIBILITY_SLOTS * OCEAN_VISIBILITY_STRIDE) as u64,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            slots: (0..OCEAN_VISIBILITY_SLOTS)
                .map(|_| OceanVisibilitySlot {
                    readback_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("JKA ocean occlusion readback"),
                        size: 8,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                    pending: None,
                })
                .collect(),
            current: None,
            encoded: false,
            latest_samples: 0,
            primed: false,
        }
    }

    pub(in crate::renderer) fn begin_frame(&mut self, device: &wgpu::Device) {
        if self.slots.iter().any(|slot| slot.pending.is_some()) {
            let _ = device.poll(wgpu::PollType::Poll);
            for slot in &mut self.slots {
                let completed = {
                    let Some(pending) = slot.pending.as_ref() else {
                        continue;
                    };
                    match pending.try_recv() {
                        Ok(Ok(())) => {
                            let Ok(bytes) = slot.readback_buffer.slice(..).get_mapped_range() else {
                                slot.readback_buffer.unmap();
                                slot.pending = None;
                                continue;
                            };
                            let values: &[u64] = bytemuck::cast_slice(&bytes);
                            let samples = values.first().copied().unwrap_or(0);
                            drop(bytes);
                            slot.readback_buffer.unmap();
                            Some(samples)
                        }
                        Ok(Err(_)) | Err(crate::renderer::TryRecvError::Disconnected) => None,
                        Err(crate::renderer::TryRecvError::Empty) => continue,
                    }
                };
                slot.pending = None;
                if let Some(samples) = completed {
                    self.latest_samples = samples;
                    self.primed = true;
                }
            }
        }
        self.current = self.slots.iter().position(|slot| slot.pending.is_none());
        self.encoded = false;
    }

    /// Query index to wrap this frame's ocean draws in, if a slot is free.
    pub(in crate::renderer) fn query_index(&self) -> Option<u32> {
        self.current.map(|current| current as u32)
    }

    pub(in crate::renderer) fn query_set(&self) -> &wgpu::QuerySet {
        &self.query_set
    }

    pub(in crate::renderer) fn encode_resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(current) = self.current else {
            return;
        };
        let index = current as u32;
        let offset = (current * OCEAN_VISIBILITY_STRIDE) as u64;
        encoder.resolve_query_set(&self.query_set, index..index + 1, &self.resolve_buffer, offset);
        encoder.copy_buffer_to_buffer(
            &self.resolve_buffer,
            offset,
            &self.slots[current].readback_buffer,
            0,
            8,
        );
        self.encoded = true;
    }

    pub(in crate::renderer) fn submit(&mut self) {
        if !self.encoded {
            self.current = None;
            return;
        }
        let Some(current) = self.current.take() else {
            return;
        };
        let (sender, receiver) = crate::renderer::mpsc::channel();
        self.slots[current]
            .readback_buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.slots[current].pending = Some(receiver);
        self.encoded = false;
    }
}

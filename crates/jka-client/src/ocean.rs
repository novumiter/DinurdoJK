//! WGPU port/adaptation of 2Retr0/GodotOceanWaves.
//! GodotOceanWaves is MIT licensed:
//! Copyright (c) 2024 Ethan Truong
//! Permission is hereby granted, free of charge, to any person obtaining a copy of this
//! software and associated documentation files (the "Software"), to deal in the Software
//! without restriction, including without limitation the rights to use, copy, modify,
//! merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
//! permit persons to whom the Software is furnished to do so, subject to the following
//! conditions: the above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS",
//! WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE
//! WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const G: f32 = 9.81;
pub mod authoring;
pub mod optics;
pub mod appearance;
pub mod interaction;
pub mod surfaces;
pub mod bathymetry;
pub mod coastal;
pub use authoring::{AtmosphericWind, OceanAuthoring, OceanWind};
pub const OCEAN_CASCADES: usize = 3;
pub const OCEAN_DEFAULT_MAP_SIZE: u32 = 1024;
pub const OCEAN_MAX_SURFACES: usize = 8;
/// Shared world-space sparse-footprint tile size. At the coastal bake's
/// default 16-unit spacing this is a 16x16-cell allocation tile.
pub const WATER_FOOTPRINT_TILE_SIZE: f32 = 256.0;
// Five existing GPU passes per cascade.  No shader/FFT algorithm changes.
const OCEAN_PASSES_PER_CASCADE: usize = 5;
const OCEAN_FFT_PASSES: usize = OCEAN_CASCADES * OCEAN_PASSES_PER_CASCADE;
// At low render FPS a strict one-pass-per-frame limit would turn a 24 Hz ocean
// into a ~4 Hz ocean at 60 FPS. Catch up modestly, never flush all 15 passes
// simply because the next simulation update is due.
const OCEAN_MAX_PASSES_PER_FRAME: usize = 5;
const OCEAN_WINDROW_SIZE: u32 = 256;
const OCEAN_WINDROW_DOMAIN_METERS: f32 = 384.0;
// JKA/jaPRO world scale used by the working C port and its GodotOceanWaves
// clipmap conversion: one metre in the upstream simulation is 40 game units.
pub const OCEAN_UNITS_PER_METER: f32 = 40.0;
// Innermost clipmap cell size for each mesh quality, matching the upstream
// clipmap_high (0.25 m) and clipmap_low (1 m) inner grids.
pub const OCEAN_HIGH_MESH_SPACING_UNITS: f32 = 0.25 * OCEAN_UNITS_PER_METER;
pub const OCEAN_LOW_MESH_SPACING_UNITS: f32 = 1.0 * OCEAN_UNITS_PER_METER;
const DEPTH_METERS: f32 = 20.0;

/// Legacy half-width of every clipmap level, in cells. Authored oceans keep
/// this exact Godot-style footprint. Automatically promoted water may use a
/// smaller camera-centred ring width when its displacement amplitude is low.
const CLIPMAP_HALF_CELLS: i32 = 128;
/// The original six-level design remains the authored-ocean profile and, more
/// importantly, the CPU clipmap-centre snap cadence. Calm promoted water may
/// add two cheaper outer rings so shrinking the inner LOD radii never shrinks
/// the world-space coverage of the mesh.
const CLIPMAP_LEVELS: u32 = 6;
const PROMOTED_CLIPMAP_MAX_LEVELS: u32 = CLIPMAP_LEVELS + 2;
const PROMOTED_CLIPMAP_MIN_HALF_CELLS: i32 = 32;
const PROMOTED_CLIPMAP_CELL_QUANTUM: i32 = 8;
const PROMOTED_CLIPMAP_FULL_DETAIL_AMPLITUDE: f32 = 64.0;

/// Per-frame ocean submission counters.
///
/// These exist because the ocean's cost does not scale with how much water is
/// on screen: the clipmap is camera-centred, so one visible sliver of a
/// promoted surface submits that surface's whole mesh, and the shape mask then
/// discards the fragments outside the real outline. `triangles` is the number
/// that makes that visible; `clipmap_batches` vs `clipmaps_drawn` shows how
/// much of the PVS set the per-surface claim collapsed.
#[derive(Clone, Copy, Debug, Default)]
pub struct OceanDrawStats {
    /// Promoted surfaces the loaded world carries, visible or not.
    pub promoted_surfaces: u32,
    /// Authored (`r_oceanAuthoring`) oceans, each of which owns a *separate*
    /// FFT simulator rather than sharing the promoted one.
    pub authored_oceans: u32,
    /// Batches carrying a clipmap that survived area + frustum culling.
    pub clipmap_batches: u32,
    /// Clipmap draws actually encoded (one per surface id per pass).
    pub clipmaps_drawn: u32,
    pub rejected_area: u32,
    pub rejected_frustum: u32,
    /// Collapsed by `claim_ocean_clipmap` because the surface was already drawn.
    pub rejected_duplicate: u32,
    /// Triangles submitted by those clipmap draws.
    pub triangles: u64,
    /// Finest live clipmap level and ring width, over the drawn surfaces.
    pub minimum_level: u32,
    pub half_cells: i32,
    /// Underwater optics capture/composite ran this frame.
    pub optics_active: bool,
    pub submerged: bool,
    /// Pixels the optics capture actually shaded. Compare against the viewport:
    /// this is the number the scissor is meant to shrink, and 0 means the whole
    /// pass was skipped because nothing could observe it.
    pub optics_pixels: u64,
    /// The FFT/wind-foam encode was skipped: no water seen recently.
    pub sim_hidden: bool,
    /// The capture scissor the frame computed, as x/y/w/h, before clamping.
    pub optics_rect: [i32; 4],
    /// A visible surface's bounds crossed the eye, so no screen rectangle
    /// bounds it and the capture had to stay full-screen.
    pub optics_unbounded: bool,
    /// Ocean samples that passed depth, from the GPU occlusion query. This is
    /// the signal the gates use; compare it against `clipmaps_drawn`, which is
    /// a bounds test and reads 1 even when this is 0.
    pub visible_samples: u64,
    /// No occlusion readback has landed yet, so visibility is assumed.
    pub visibility_primed: bool,
}

/// One clipmap vertex, expressed in the world vertex format so the ocean can be
/// drawn by the existing BSP pipeline instead of needing its own.
///
/// `position.xz` is the grid offset from the clipmap centre in world units,
/// `color.r` is this vertex's own cell size and `color.g` is the parent cell
/// size for vertices that sit on a level's outer boundary (zero elsewhere).
/// Snapping those boundary vertices onto the parent grid in the vertex shader
/// collapses the mismatched half of them into degenerate triangles, which keeps
/// a 2:1 LOD seam watertight without building transition fans.
pub struct OceanClipmap {
    pub vertices: Vec<crate::scene::GpuVertex>,
    pub indices: Vec<u32>,
    pub coarsest_spacing: f32,
}

/// Runtime-rebuildable index topology for automatically promoted water.
///
/// The vertex grids are uploaded once at their maximum useful widths. The
/// index list can then be rewritten cheaply when local amplitude or the
/// camera's true 3D distance from the water selects a different LOD profile.
/// This keeps the expensive vertex allocation out of slider/camera updates
/// while still reducing the triangles the GPU actually submits.
#[derive(Clone, Debug)]
pub struct PromotedClipmapLayout {
    pub base_spacing: f32,
    pub extent: f32,
    pub level_vertex_bases: [u32; 8],
    pub level_vertex_half_cells: [i32; 8],
}

#[derive(Clone, Debug)]
pub struct RuntimePromotedClipmap {
    pub vertices: Vec<crate::scene::GpuVertex>,
    pub indices: Vec<u32>,
    pub index_capacity: usize,
    pub coarsest_spacing: f32,
    pub layout: PromotedClipmapLayout,
}

/// Marks a vertex as belonging to the ocean clipmap. Real BSP vertices carry
/// cached static AO here, which is never negative, so the shader can recognise
/// clipmap geometry from the vertex alone. That matters: the draw call decides
/// whether the clipmap is used, and nothing else in the shader may be able to
/// disagree with it.
pub const OCEAN_CLIPMAP_VERTEX_MARKER: f32 = -1.0;

/// How many clipmap levels a surface needs. Cell size never depends on the size
/// of the water: level 0 always has the quality's own cell size and each level
/// doubles it. What a footprint decides is only how far out the rings must reach.
///
/// The centre is clamped into the footprint and re-snapped to the outermost
/// built level's lattice, so it can sit half that lattice step outside it. The
/// outermost level therefore has to reach `extent + step / 2` from the centre:
/// `HALF * step >= extent + step / 2`. Anything beyond that would be vertices
/// folded onto the footprint edge as zero-area triangles.
fn clipmap_levels_for(base_spacing: f32, extent: f32, half_cells: i32, max_levels: u32) -> u32 {
    if !extent.is_finite() {
        return max_levels;
    }
    let reach = half_cells as f32 - 0.5;
    (1..=max_levels)
        .find(|&levels| base_spacing * (1u32 << (levels - 1)) as f32 * reach >= extent)
        .unwrap_or(max_levels)
}

/// Camera-distance LOD radius for automatically promoted water.
///
/// Amplitude is the useful signal here: a five-unit ripple does not need the
/// same 1280-unit radius of 10-unit triangles as a 64+-unit ocean swell. Keep
/// the exact finest spacing beside the viewer, but make each 2:1 clipmap ring
/// physically narrower for calm water. `sqrt` is deliberately conservative:
/// medium seas recover detail radius faster than a linear rule would.
pub fn promoted_clipmap_half_cells(amplitude: f32) -> i32 {
    if !amplitude.is_finite() {
        return CLIPMAP_HALF_CELLS;
    }
    let minimum_amplitude = appearance::MIN_PROMOTED_AMPLITUDE;
    let t = ((amplitude.max(minimum_amplitude) - minimum_amplitude)
        / (PROMOTED_CLIPMAP_FULL_DETAIL_AMPLITUDE - minimum_amplitude))
        .clamp(0.0, 1.0)
        .sqrt();
    let cells = PROMOTED_CLIPMAP_MIN_HALF_CELLS as f32
        + (CLIPMAP_HALF_CELLS - PROMOTED_CLIPMAP_MIN_HALF_CELLS) as f32 * t;
    (((cells / PROMOTED_CLIPMAP_CELL_QUANTUM as f32).ceil() as i32)
        * PROMOTED_CLIPMAP_CELL_QUANTUM)
        .clamp(PROMOTED_CLIPMAP_MIN_HALF_CELLS, CLIPMAP_HALF_CELLS)
}

fn promoted_level_half_cells(level: u32, half_cells: i32) -> i32 {
    if level < CLIPMAP_LEVELS {
        half_cells
    } else {
        // Extra rings exist only to preserve distant coverage after narrowing
        // the near-field profile. At twice the spacing, half the previous
        // width reaches the same world-space radius; keep at least the calm
        // profile width so low-amplitude water continues expanding outward.
        PROMOTED_CLIPMAP_MIN_HALF_CELLS.max(half_cells / 2)
    }
}

fn promoted_levels_for(base_spacing: f32, extent: f32, half_cells: i32) -> u32 {
    for level in 0..PROMOTED_CLIPMAP_MAX_LEVELS {
        let spacing = base_spacing * (1u32 << level) as f32;
        let half = promoted_level_half_cells(level, half_cells) as f32;
        if spacing * (half - 0.5) >= extent {
            return level + 1;
        }
    }
    PROMOTED_CLIPMAP_MAX_LEVELS
}

/// The coarsest active cell size for a promoted clipmap at this amplitude.
/// Kept public because the per-pool render uniform must use the same lattice
/// as the CPU index topology when the live amplitude slider changes it.
pub fn promoted_clipmap_outer_spacing(base_spacing: f32, extent: f32, amplitude: f32) -> f32 {
    let half_cells = promoted_clipmap_half_cells(amplitude);
    let levels = promoted_levels_for(base_spacing, extent, half_cells);
    base_spacing * (1u32 << (levels.saturating_sub(1))) as f32
}

impl PromotedClipmapLayout {
    fn levels_for_half_cells(&self, half_cells: i32) -> u32 {
        promoted_levels_for(self.base_spacing, self.extent, half_cells)
    }

    pub fn outer_spacing_for_half_cells(&self, half_cells: i32) -> f32 {
        let levels = self.levels_for_half_cells(half_cells);
        self.base_spacing * (1u32 << levels.saturating_sub(1)) as f32
    }

    /// Select the finest level worth drawing from the shortest 3D distance to
    /// the water footprint. This is the piece the old camera-centred clipmap
    /// missed: altitude (JKA Z / renderer Y) must count just like horizontal
    /// separation, otherwise flying directly above a sea pins maximum LOD.
    pub fn minimum_level_for_distance(&self, half_cells: i32, distance: f32) -> u32 {
        let levels = self.levels_for_half_cells(half_cells);
        if levels <= 1 || !distance.is_finite() {
            return 0;
        }
        let mut level = 0;
        while level + 1 < levels {
            let spacing = self.base_spacing * (1u32 << level) as f32;
            let half = promoted_level_half_cells(level, half_cells) as f32;
            let reach = spacing * (half - 0.5);
            if distance < reach {
                break;
            }
            level += 1;
        }
        level
    }

    fn index_count_for(&self, half_cells: i32, minimum_level: u32) -> usize {
        let levels = self.levels_for_half_cells(half_cells);
        let first = minimum_level.min(levels.saturating_sub(1));
        let mut previous_half = 0i32;
        let mut cells = 0usize;
        for level in first..levels {
            let half = promoted_level_half_cells(level, half_cells);
            let hole = if level == first { 0 } else { previous_half / 2 };
            let side_cells = usize::try_from(half * 2).unwrap_or(0);
            let hole_cells = usize::try_from(hole * 2).unwrap_or(0);
            cells = cells.saturating_add(
                side_cells
                    .saturating_mul(side_cells)
                    .saturating_sub(hole_cells.saturating_mul(hole_cells)),
            );
            previous_half = half;
        }
        cells.saturating_mul(6)
    }

    /// Build only the indices used by the selected live profile. The uploaded
    /// vertex grids are shared by all profiles; a lower `minimum_level` fills
    /// the centre at that level and omits every finer grid entirely.
    pub fn indices_for(&self, half_cells: i32, minimum_level: u32) -> Vec<u32> {
        let levels = self.levels_for_half_cells(half_cells);
        let first = minimum_level.min(levels.saturating_sub(1));
        let mut indices = Vec::with_capacity(self.index_count_for(half_cells, minimum_level));
        let mut previous_half = 0i32;
        for level in first..levels {
            let spacing_half = promoted_level_half_cells(level, half_cells);
            let vertex_half = self.level_vertex_half_cells[level as usize];
            debug_assert!(spacing_half <= vertex_half);
            let hole = if level == first { 0 } else { previous_half / 2 };
            let side = (vertex_half * 2 + 1) as u32;
            let base = self.level_vertex_bases[level as usize];
            let index = |c: i32, r: i32| {
                base + (r + vertex_half) as u32 * side + (c + vertex_half) as u32
            };
            for row in -spacing_half..spacing_half {
                for column in -spacing_half..spacing_half {
                    if hole > 0
                        && column >= -hole
                        && column < hole
                        && row >= -hole
                        && row < hole
                    {
                        continue;
                    }
                    let a = index(column, row);
                    let b = index(column + 1, row);
                    let c = index(column, row + 1);
                    let d = index(column + 1, row + 1);
                    indices.extend_from_slice(&[a, c, b, b, c, d]);
                }
            }
            previous_half = spacing_half;
        }
        indices
    }

    /// Builds the same live LOD topology while omitting cells proven dry by
    /// the promoted surface's exact footprint. This runs only when the snapped
    /// clipmap centre or an existing LOD selector changes.
    pub fn indices_for_footprint(
        &self,
        half_cells: i32,
        minimum_level: u32,
        centre: [f32; 2],
        surface: OceanSurface,
        masks: &OceanMasks,
        mask_slot: Option<usize>,
        pvs_tiles: Option<&OceanPvsTileMask>,
    ) -> Vec<u32> {
        let levels = self.levels_for_half_cells(half_cells);
        let first = minimum_level.min(levels.saturating_sub(1));
        let outer_spacing = self.base_spacing * (1u32 << (levels.saturating_sub(1))) as f32;
        let mut indices = Vec::with_capacity(self.index_count_for(half_cells, minimum_level));
        let mut previous_half = 0i32;
        for level in first..levels {
            let spacing_half = promoted_level_half_cells(level, half_cells);
            let vertex_half = self.level_vertex_half_cells[level as usize];
            debug_assert!(spacing_half <= vertex_half);
            let spacing = self.base_spacing * (1u32 << level) as f32;
            let hole = if level == first { 0 } else { previous_half / 2 };
            let side = (vertex_half * 2 + 1) as u32;
            let base = self.level_vertex_bases[level as usize];
            let index = |c: i32, r: i32| {
                base + (r + vertex_half) as u32 * side + (c + vertex_half) as u32
            };
            let position = |c: i32, r: i32| {
                let mut offset = [c as f32 * spacing, r as f32 * spacing];
                let on_outer = c.abs().max(r.abs()) >= spacing_half;
                if on_outer && spacing < outer_spacing - 0.25 {
                    let parent = spacing * 2.0;
                    offset[0] = (offset[0] / parent).round() * parent;
                    offset[1] = (offset[1] / parent).round() * parent;
                }
                [
                    (centre[0] + offset[0]).clamp(surface.minimum[0], surface.maximum[0]),
                    (centre[1] + offset[1]).clamp(surface.minimum[1], surface.maximum[1]),
                ]
            };
            for row in -spacing_half..spacing_half {
                for column in -spacing_half..spacing_half {
                    if hole > 0
                        && column >= -hole
                        && column < hole
                        && row >= -hole
                        && row < hole
                    {
                        continue;
                    }
                    let corners = [
                        position(column, row),
                        position(column + 1, row),
                        position(column, row + 1),
                        position(column + 1, row + 1),
                    ];
                    let lo = [
                        corners.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
                        corners.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                    ];
                    let hi = [
                        corners.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max),
                        corners.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max),
                    ];
                    if hi[0] - lo[0] <= 1.0e-4 || hi[1] - lo[1] <= 1.0e-4 {
                        continue;
                    }
                    if mask_slot.is_some_and(|slot| !masks.rect_may_contain(slot, lo, hi)) {
                        continue;
                    }
                    if pvs_tiles.is_some_and(|tiles| {
                        !rect_intersects_pvs_tiles(surface, tiles, lo, hi)
                    }) {
                        continue;
                    }
                    let a = index(column, row);
                    let b = index(column + 1, row);
                    let c = index(column, row + 1);
                    let d = index(column + 1, row + 1);
                    indices.extend_from_slice(&[a, c, b, b, c, d]);
                }
            }
            previous_half = spacing_half;
        }
        indices
    }

    pub fn maximum_index_count(&self) -> usize {
        let mut maximum = 0usize;
        let mut half = PROMOTED_CLIPMAP_MIN_HALF_CELLS;
        while half <= CLIPMAP_HALF_CELLS {
            let levels = self.levels_for_half_cells(half);
            for first in 0..levels {
                maximum = maximum.max(self.index_count_for(half, first));
            }
            half += PROMOTED_CLIPMAP_CELL_QUANTUM;
        }
        maximum
    }
}

/// Shortest render-space distance from the camera to the undisplaced
/// rectangular water footprint. Renderer Y is JKA Z, so flying above the sea
/// naturally increases this distance even when X/Z remain inside its bounds.
pub fn promoted_surface_distance(camera: [f32; 3], surface: OceanSurface) -> f32 {
    let axis_distance = |value: f32, minimum: f32, maximum: f32| {
        if value < minimum {
            minimum - value
        } else if value > maximum {
            value - maximum
        } else {
            0.0
        }
    };
    let dx = axis_distance(camera[0], surface.minimum[0], surface.maximum[0]);
    let dy = (camera[1] - surface.plane_height).abs();
    let dz = axis_distance(camera[2], surface.minimum[1], surface.maximum[1]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// True distance to the source water footprint when it is available. The
/// AABB calculation remains the conservative fallback for authored/legacy data.
pub fn promoted_footprint_distance(
    camera: [f32; 3],
    surface: OceanSurface,
    masks: &OceanMasks,
    slot: usize,
) -> f32 {
    let p = [camera[0], camera[2]];
    let nearest = masks.closest_point(slot, p).unwrap_or([
        p[0].clamp(surface.minimum[0], surface.maximum[0]),
        p[1].clamp(surface.minimum[1], surface.maximum[1]),
    ]);
    let horizontal_sq = (p[0] - nearest[0]).powi(2) + (p[1] - nearest[1]).powi(2);
    (horizontal_sq + (camera[1] - surface.plane_height).powi(2)).sqrt()
}

/// Builds the clipmap for one promoted water surface. Everything the vertex
/// shader needs is baked in:
///   position.xz  grid offset from the clipmap centre, in world units
///   position.y   this surface's water plane height
///   uv           footprint minimum (world xz)
///   lightmap_uv  footprint maximum (world xz)
///   color.r      this vertex's own cell size
///   color.g      parent cell size on a level's outer boundary, else 0
///   color.b      centre snap step: this clipmap's outermost built cell size
///   color.a      shape-mask slot in `OceanMasks`, or -1 for none
///   alpha_cutoff the clipmap marker
///
/// Only as many levels are built as the footprint can use. A pool that fits
/// inside level 0 is a single level, and a single level is also trimmed to the
/// cells the footprint can reach.
pub fn build_clipmap(
    base_spacing: f32,
    plane_height: f32,
    footprint_minimum: [f32; 2],
    footprint_maximum: [f32; 2],
    mask_slot: Option<usize>,
) -> OceanClipmap {
    build_clipmap_profile(
        base_spacing,
        plane_height,
        footprint_minimum,
        footprint_maximum,
        mask_slot,
        CLIPMAP_HALF_CELLS,
        CLIPMAP_LEVELS,
    )
}

/// Amplitude-aware clipmap used only for ordinary BSP water promoted into the
/// FFT ocean. Authored oceans deliberately keep `build_clipmap`'s reference
/// profile. A calm pool retains the same 0.25 m / 1 m innermost cell spacing,
/// but its fine rings end much sooner; extra very-coarse rings preserve the old
/// maximum world-space reach without paying for huge near-field grids.
pub fn build_promoted_clipmap(
    base_spacing: f32,
    plane_height: f32,
    footprint_minimum: [f32; 2],
    footprint_maximum: [f32; 2],
    mask_slot: Option<usize>,
    amplitude: f32,
) -> OceanClipmap {
    build_clipmap_profile(
        base_spacing,
        plane_height,
        footprint_minimum,
        footprint_maximum,
        mask_slot,
        promoted_clipmap_half_cells(amplitude),
        PROMOTED_CLIPMAP_MAX_LEVELS,
    )
}

/// Upload-once promoted-water clipmap. The maximum vertex grids are retained,
/// while only the current profile's indices are active. Runtime amplitude and
/// camera-distance changes rewrite those indices in-place without rebuilding
/// the rest of the BSP world.
pub fn build_runtime_promoted_clipmap(
    base_spacing: f32,
    plane_height: f32,
    footprint_minimum: [f32; 2],
    footprint_maximum: [f32; 2],
    mask_slot: Option<usize>,
    amplitude: f32,
) -> RuntimePromotedClipmap {
    let extent = (footprint_maximum[0] - footprint_minimum[0])
        .max(footprint_maximum[1] - footprint_minimum[1]);
    let mask_slot = mask_slot.map_or(-1.0, |slot| slot as f32);
    let mut vertices = Vec::new();
    let mut level_vertex_bases = [0u32; PROMOTED_CLIPMAP_MAX_LEVELS as usize];
    let mut level_vertex_half_cells = [0i32; PROMOTED_CLIPMAP_MAX_LEVELS as usize];

    for level in 0..PROMOTED_CLIPMAP_MAX_LEVELS {
        let vertex_half = if level < CLIPMAP_LEVELS {
            CLIPMAP_HALF_CELLS
        } else {
            CLIPMAP_HALF_CELLS / 2
        };
        level_vertex_bases[level as usize] = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        level_vertex_half_cells[level as usize] = vertex_half;
        let spacing = base_spacing * (1u32 << level) as f32;
        let side = (vertex_half * 2 + 1) as usize;
        for row in 0..side {
            for column in 0..side {
                let cell_x = column as i32 - vertex_half;
                let cell_z = row as i32 - vertex_half;
                vertices.push(crate::scene::GpuVertex {
                    position: [cell_x as f32 * spacing, plane_height, cell_z as f32 * spacing],
                    uv: footprint_minimum,
                    lightmap_uv: footprint_maximum,
                    normal: [0.0, 1.0, 0.0],
                    // Dynamic promoted vertices keep their grid capacity in G.
                    // Extra coverage-only levels use a negative sign so the
                    // shader applies max(32, live_half/2) to those levels.
                    color: [
                        spacing,
                        if level < CLIPMAP_LEVELS { vertex_half as f32 } else { -(vertex_half as f32) },
                        0.0,
                        mask_slot,
                    ],
                    alpha_cutoff: OCEAN_CLIPMAP_VERTEX_MARKER,
                });
            }
        }
    }

    let layout = PromotedClipmapLayout {
        base_spacing,
        extent,
        level_vertex_bases,
        level_vertex_half_cells,
    };
    let half_cells = promoted_clipmap_half_cells(amplitude);
    let indices = layout.indices_for(half_cells, 0);
    let index_capacity = layout.maximum_index_count();
    RuntimePromotedClipmap {
        vertices,
        indices,
        index_capacity,
        coarsest_spacing: base_spacing * (1u32 << (CLIPMAP_LEVELS - 1)) as f32,
        layout,
    }
}

fn build_clipmap_profile(
    base_spacing: f32,
    plane_height: f32,
    footprint_minimum: [f32; 2],
    footprint_maximum: [f32; 2],
    mask_slot: Option<usize>,
    half_cells: i32,
    max_levels: u32,
) -> OceanClipmap {
    debug_assert!(half_cells > 0 && half_cells % 2 == 0);
    let mask_slot = mask_slot.map_or(-1.0, |slot| slot as f32);
    let extent = (footprint_maximum[0] - footprint_minimum[0])
        .max(footprint_maximum[1] - footprint_minimum[1]);
    let levels = clipmap_levels_for(base_spacing, extent, half_cells, max_levels);
    let snap_step = base_spacing * (1u32 << (levels - 1)) as f32;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for level in 0..levels {
        let spacing = base_spacing * (1u32 << level) as f32;
        // Every outer level's hole is half its own cell radius. At twice the
        // spacing that lands exactly on the previous level's outer boundary,
        // even when calm-water LOD uses a narrower ring than the reference.
        let hole = if level == 0 { 0 } else { half_cells / 2 };
        // Only a lone level has no neighbour whose hole it must match, so only
        // it may shrink below the selected profile width.
        let half = if levels == 1 {
            (((extent + spacing * 0.5) / spacing).ceil() as i32).clamp(1, half_cells)
        } else {
            half_cells
        };
        let parent = if level + 1 < levels { spacing * 2.0 } else { 0.0 };
        let side = (half * 2 + 1) as usize;
        let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        for row in 0..side {
            for column in 0..side {
                let cell_x = column as i32 - half;
                let cell_z = row as i32 - half;
                // Only the outermost vertex ring of a level meets a coarser
                // neighbour, so only it needs parent-grid snapping.
                let on_outer_edge = cell_x.abs() == half || cell_z.abs() == half;
                let snap = if on_outer_edge { parent } else { 0.0 };
                vertices.push(crate::scene::GpuVertex {
                    position: [cell_x as f32 * spacing, plane_height, cell_z as f32 * spacing],
                    uv: footprint_minimum,
                    lightmap_uv: footprint_maximum,
                    normal: [0.0, 1.0, 0.0],
                    color: [spacing, snap, snap_step, mask_slot],
                    alpha_cutoff: OCEAN_CLIPMAP_VERTEX_MARKER,
                });
            }
        }
        for row in 0..(half * 2) {
            for column in 0..(half * 2) {
                let cell_x = column - half;
                let cell_z = row - half;
                // Cells strictly inside the hole belong to the finer level.
                if hole > 0 && cell_x >= -hole && cell_x < hole && cell_z >= -hole && cell_z < hole {
                    continue;
                }
                let index = |c: i32, r: i32| base + (r + half) as u32 * side as u32 + (c + half) as u32;
                let a = index(cell_x, cell_z);
                let b = index(cell_x + 1, cell_z);
                let c = index(cell_x, cell_z + 1);
                let d = index(cell_x + 1, cell_z + 1);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    OceanClipmap {
        vertices,
        indices,
        // Keep the CPU centre's update/snap cadence at the original design.
        // Adaptive promoted meshes may have extra coarse outer rings, but using
        // those for camera snapping would make the fine inner grid jump by
        // thousands of units. The shader still re-snaps each surface to its
        // actual outermost lattice through color.b.
        coarsest_spacing: base_spacing * (1u32 << (CLIPMAP_LEVELS - 1)) as f32,
    }
}

/// Capacity of the shape-mask triangle store, in vec4s. A triangle takes two.
pub const OCEAN_MASK_VEC4S: usize = 512;
/// Coarse exact-acceleration grid for promoted-water outlines. Each tile uses
/// two bits: 0 means test the source triangles, 1 is definitely outside and 2
/// is definitely inside. Boundary tiles therefore retain the original answer.
pub const OCEAN_MASK_TILE_AXIS: usize = 8;
/// Finer CPU-only grid used when pruning dry cells from the moving clipmap.
/// It never reaches a shader uniform, so improving topology precision costs
/// only 256 bytes per masked surface.
pub const OCEAN_PRUNE_TILE_AXIS: usize = 32;
pub const OCEAN_PRUNE_TILE_WORDS: usize = OCEAN_PRUNE_TILE_AXIS * OCEAN_PRUNE_TILE_AXIS * 2 / 32;
/// One bit per fine tile. PVS footprints need only visible/not-visible rather
/// than the outline accelerator's three states, so one u32 represents a row.
pub type OceanPvsTileMask = [u32; OCEAN_PRUNE_TILE_AXIS];

#[derive(Clone, Debug)]
struct OceanPvsPiece {
    slot: usize,
    tiles: OceanPvsTileMask,
}

/// Exact-PVS water-piece footprints, indexed in parallel with `full_batches`.
/// The renderer unions only the entries in the current cluster's existing FULL
/// visibility list, so memory scales with water pieces rather than with every
/// ocean x every BSP cluster.
#[derive(Clone, Debug, Default)]
pub struct OceanPvsFootprints {
    pieces: Vec<Option<OceanPvsPiece>>,
    known: [bool; OCEAN_MAX_SURFACES],
}

impl OceanPvsFootprints {
    pub fn with_batch_count(batch_count: usize) -> Self {
        Self {
            pieces: vec![None; batch_count],
            known: [false; OCEAN_MAX_SURFACES],
        }
    }

    /// Records the exact water geometry carried by one FULL/PVS batch.
    pub fn add_piece(
        &mut self,
        batch: usize,
        slot: usize,
        surface: OceanSurface,
        triangles: impl IntoIterator<Item = MaskTriangle>,
    ) {
        if batch >= self.pieces.len() || slot >= OCEAN_MAX_SURFACES {
            return;
        }
        let size = [
            surface.maximum[0] - surface.minimum[0],
            surface.maximum[1] - surface.minimum[1],
        ];
        let mut tiles = [0u32; OCEAN_PRUNE_TILE_AXIS];
        if size[0] <= 1.0e-4 || size[1] <= 1.0e-4 {
            tiles.fill(u32::MAX);
        } else {
            let tile = |value: f32, axis: usize| {
                (((value - surface.minimum[axis]) / size[axis]
                    * OCEAN_PRUNE_TILE_AXIS as f32)
                    .floor() as i32)
                    .clamp(0, OCEAN_PRUNE_TILE_AXIS as i32 - 1) as usize
            };
            for triangle in triangles {
                if triangle.iter().flatten().any(|value| !value.is_finite()) {
                    tiles.fill(u32::MAX);
                    break;
                }
                let lo = [0, 1].map(|axis| {
                    triangle
                        .iter()
                        .map(|point| point[axis])
                        .fold(f32::INFINITY, f32::min)
                        - MASK_TOLERANCE
                });
                let hi = [0, 1].map(|axis| {
                    triangle
                        .iter()
                        .map(|point| point[axis])
                        .fold(f32::NEG_INFINITY, f32::max)
                        + MASK_TOLERANCE
                });
                let (x0, x1, y0, y1) =
                    (tile(lo[0], 0), tile(hi[0], 0), tile(lo[1], 1), tile(hi[1], 1));
                for row in &mut tiles[y0..=y1] {
                    for x in x0..=x1 {
                        *row |= 1u32 << x;
                    }
                }
            }
        }
        self.known[slot] = true;
        self.pieces[batch] = Some(OceanPvsPiece { slot, tiles });
    }

    /// Unions water tiles from the renderer's exact visible-batch list. `None`
    /// means this surface lacked exact pieces and must remain conservative;
    /// `Some(all-zero)` means the surface is completely outside this PVS.
    pub fn visible_tiles(
        &self,
        visible_batches: &[usize],
    ) -> [Option<OceanPvsTileMask>; OCEAN_MAX_SURFACES] {
        let mut visible = std::array::from_fn(|slot| self.known[slot].then_some([0; OCEAN_PRUNE_TILE_AXIS]));
        for &batch in visible_batches {
            let Some(piece) = self.pieces.get(batch).and_then(Option::as_ref) else {
                continue;
            };
            let Some(surface) = visible[piece.slot].as_mut() else {
                continue;
            };
            for (destination, source) in surface.iter_mut().zip(piece.tiles) {
                *destination |= source;
            }
        }
        visible
    }
}

/// One world-space xz triangle of a promoted water face.
pub type MaskTriangle = [[f32; 2]; 3];

/// The real outline of each promoted ocean surface. A clipmap is a square
/// grid folded into its footprint's bounding box, so the water fragments it
/// shades are cut back to these triangles; that is what keeps a hole or an
/// L-shape inside contiguous water from being painted over.
///
/// `ranges[slot]` is `[first triangle, triangle count, 0, 0]`. A count of zero
/// means "no mask": the surface fills its whole footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct OceanMasks {
    pub ranges: [[f32; 4]; OCEAN_MAX_SURFACES],
    pub bounds: [[f32; 4]; OCEAN_MAX_SURFACES],
    pub tiles: [[u32; 4]; OCEAN_MAX_SURFACES],
    pub prune_tiles: [[u32; OCEAN_PRUNE_TILE_WORDS]; OCEAN_MAX_SURFACES],
    pub triangles: Vec<[f32; 4]>,
    /// CPU-side exact sparse footprints. These do not enter the shader
    /// uniform; they guide clipmap centering, LOD distance, and index pruning.
    pub footprints: [Option<std::sync::Arc<WaterFootprint>>; OCEAN_MAX_SURFACES],
}

/// CPU twin of the promoted vertex shader's two-stage centre snapping. The
/// global clipmap moves on the legacy coarse lattice; each promoted surface
/// then clamps that point into its bounds and snaps it to its live outer ring.
pub fn promoted_clipmap_centre(
    camera: [f32; 3],
    surface: OceanSurface,
    global_spacing: f32,
    outer_spacing: f32,
) -> [f32; 2] {
    let global = global_spacing.max(1.0);
    let outer = outer_spacing.max(1.0);
    let camera = [
        (camera[0] / global).round() * global,
        (camera[2] / global).round() * global,
    ];
    [
        (camera[0].clamp(surface.minimum[0], surface.maximum[0]) / outer).round() * outer,
        (camera[1].clamp(surface.minimum[1], surface.maximum[1]) / outer).round() * outer,
    ]
}

/// Footprint-aware equivalent of [`promoted_clipmap_centre`]. An L-shaped
/// ocean therefore keeps its finest rings on the nearest real water instead
/// of centering them in the dry corner of its enclosing rectangle.
pub fn promoted_footprint_clipmap_centre(
    camera: [f32; 3],
    surface: OceanSurface,
    global_spacing: f32,
    outer_spacing: f32,
    masks: &OceanMasks,
    slot: usize,
) -> [f32; 2] {
    let global = global_spacing.max(1.0);
    let outer = outer_spacing.max(1.0);
    let snapped = [
        (camera[0] / global).round() * global,
        (camera[2] / global).round() * global,
    ];
    let nearest = masks.closest_point(slot, snapped).unwrap_or([
        snapped[0].clamp(surface.minimum[0], surface.maximum[0]),
        snapped[1].clamp(surface.minimum[1], surface.maximum[1]),
    ]);
    [
        (nearest[0] / outer).round() * outer,
        (nearest[1] / outer).round() * outer,
    ]
}

fn rect_intersects_pvs_tiles(
    surface: OceanSurface,
    tiles: &OceanPvsTileMask,
    lo: [f32; 2],
    hi: [f32; 2],
) -> bool {
    if hi[0] < surface.minimum[0]
        || hi[1] < surface.minimum[1]
        || lo[0] > surface.maximum[0]
        || lo[1] > surface.maximum[1]
    {
        return false;
    }
    let size = [
        (surface.maximum[0] - surface.minimum[0]).max(1.0e-4),
        (surface.maximum[1] - surface.minimum[1]).max(1.0e-4),
    ];
    let tile = |value: f32, axis: usize| {
        (((value - surface.minimum[axis]) / size[axis] * OCEAN_PRUNE_TILE_AXIS as f32).floor()
            as i32)
            .clamp(0, OCEAN_PRUNE_TILE_AXIS as i32 - 1) as usize
    };
    let (x0, x1, y0, y1) = (
        tile(lo[0], 0),
        tile(hi[0], 0),
        tile(lo[1], 1),
        tile(hi[1], 1),
    );
    (y0..=y1).any(|y| {
        (x0..=x1).any(|x| tiles[y] & (1u32 << x) != 0)
    })
}

impl Default for OceanMasks {
    fn default() -> Self {
        Self {
            ranges: [[0.0; 4]; OCEAN_MAX_SURFACES],
            bounds: [[0.0; 4]; OCEAN_MAX_SURFACES],
            tiles: [[0; 4]; OCEAN_MAX_SURFACES],
            prune_tiles: [[0; OCEAN_PRUNE_TILE_WORDS]; OCEAN_MAX_SURFACES],
            triangles: Vec::new(),
            footprints: std::array::from_fn(|_| None),
        }
    }
}

const MASK_TILE_BOUNDARY: u32 = 0;
const MASK_TILE_OUTSIDE: u32 = 1;
const MASK_TILE_INSIDE: u32 = 2;
const MASK_TOLERANCE: f32 = 0.75;

fn mask_triangle_edges(t: MaskTriangle, p: [f32; 2]) -> [f32; 3] {
    let edge = |from: [f32; 2], to: [f32; 2]| {
        let d = [to[0] - from[0], to[1] - from[1]];
        (d[0] * (p[1] - from[1]) - d[1] * (p[0] - from[0]))
            / d[0].hypot(d[1]).max(1.0e-4)
    };
    [edge(t[0], t[1]), edge(t[1], t[2]), edge(t[2], t[0])]
}

fn mask_triangle_contains(t: MaskTriangle, p: [f32; 2]) -> bool {
    let e = mask_triangle_edges(t, p);
    e.iter().all(|&v| v >= -MASK_TOLERANCE)
        || e.iter().all(|&v| v <= MASK_TOLERANCE)
}

fn tile_inside_triangle(t: MaskTriangle, lo: [f32; 2], hi: [f32; 2]) -> bool {
    let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
    // Require one consistent winding branch for all four corners. Each branch
    // is an intersection of half-planes, hence contains the complete tile.
    let edges = corners.map(|p| mask_triangle_edges(t, p));
    edges.iter().all(|e| e.iter().all(|&v| v >= -MASK_TOLERANCE))
        || edges.iter().all(|e| e.iter().all(|&v| v <= MASK_TOLERANCE))
}

fn set_mask_tile(words: &mut [u32], tile: usize, code: u32) {
    let bit = tile * 2;
    words[bit / 32] |= code << (bit % 32);
}

fn build_mask_tiles<const AXIS: usize, const WORDS: usize>(
    triangles: &[MaskTriangle],
    minimum: [f32; 2],
    maximum: [f32; 2],
) -> [u32; WORDS] {
    let size = [maximum[0] - minimum[0], maximum[1] - minimum[1]];
    let mut tiles = [0u32; WORDS];
    for y in 0..AXIS {
        for x in 0..AXIS {
            let lo = [
                minimum[0] + size[0] * x as f32 / AXIS as f32,
                minimum[1] + size[1] * y as f32 / AXIS as f32,
            ];
            let hi = [
                minimum[0] + size[0] * (x + 1) as f32 / AXIS as f32,
                minimum[1] + size[1] * (y + 1) as f32 / AXIS as f32,
            ];
            let overlaps = |t: &MaskTriangle| {
                (0..2).all(|axis| {
                    let tri_lo = t.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min);
                    let tri_hi = t.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max);
                    tri_hi + MASK_TOLERANCE >= lo[axis]
                        && tri_lo - MASK_TOLERANCE <= hi[axis]
                })
            };
            let code = if !triangles.iter().any(overlaps) {
                MASK_TILE_OUTSIDE
            } else if triangles.iter().any(|&t| tile_inside_triangle(t, lo, hi)) {
                MASK_TILE_INSIDE
            } else {
                MASK_TILE_BOUNDARY
            };
            set_mask_tile(&mut tiles, y * AXIS + x, code);
        }
    }
    tiles
}

fn mask_tile_code(bounds: [f32; 4], words: [u32; 4], p: [f32; 2]) -> u32 {
    if p[0] < bounds[0] || p[1] < bounds[1] || p[0] > bounds[2] || p[1] > bounds[3] {
        return MASK_TILE_OUTSIDE;
    }
    let size = [(bounds[2] - bounds[0]).max(1.0e-4), (bounds[3] - bounds[1]).max(1.0e-4)];
    let x = (((p[0] - bounds[0]) / size[0] * OCEAN_MASK_TILE_AXIS as f32).floor() as i32)
        .clamp(0, OCEAN_MASK_TILE_AXIS as i32 - 1) as usize;
    let y = (((p[1] - bounds[1]) / size[1] * OCEAN_MASK_TILE_AXIS as f32).floor() as i32)
        .clamp(0, OCEAN_MASK_TILE_AXIS as i32 - 1) as usize;
    let bit = (y * OCEAN_MASK_TILE_AXIS + x) * 2;
    (words[bit / 32] >> (bit % 32)) & 3
}

fn packed_tile_code(words: &[u32], tile: usize) -> u32 {
    let bit = tile * 2;
    (words[bit / 32] >> (bit % 32)) & 3
}

/// Whether a promoted surface actually needs the per-fragment outline mask.
///
/// BSP water faces on one plane form a non-overlapping planar tessellation. If
/// their summed triangle area fills the cluster's axis-aligned footprint, the
/// clipmap rectangle already is the exact water silhouette and scanning those
/// triangles in the fragment shader is pure overhead. Irregular, rotated and
/// holed surfaces retain the exact triangle mask.
pub fn promoted_outline_needs_mask(
    minimum: [f32; 2],
    maximum: [f32; 2],
    triangles: &[MaskTriangle],
) -> bool {
    if triangles.is_empty() {
        return false;
    }
    let width = f64::from((maximum[0] - minimum[0]).max(0.0));
    let height = f64::from((maximum[1] - minimum[1]).max(0.0));
    let bounds_area = width * height;
    if bounds_area <= 1.0e-8 {
        return false;
    }
    let triangle_area: f64 = triangles
        .iter()
        .map(|t| {
            let ab = [
                f64::from(t[1][0]) - f64::from(t[0][0]),
                f64::from(t[1][1]) - f64::from(t[0][1]),
            ];
            let ac = [
                f64::from(t[2][0]) - f64::from(t[0][0]),
                f64::from(t[2][1]) - f64::from(t[0][1]),
            ];
            (ab[0] * ac[1] - ab[1] * ac[0]).abs() * 0.5
        })
        .sum();
    // Keep this much tighter than any meaningful map-space hole; f64 is used
    // specifically so a huge sea does not need a huge absolute tolerance.
    let tolerance = (bounds_area * 1.0e-8).max(0.01);
    (triangle_area - bounds_area).abs() > tolerance
}

impl OceanMasks {
    pub fn push_footprint(&mut self, slot: usize, footprint: &WaterFootprint) -> bool {
        if slot >= OCEAN_MAX_SURFACES {
            return false;
        }
        self.footprints[slot] = Some(std::sync::Arc::new(footprint.clone()));
        self.push_surface(slot, &footprint.triangles)
    }

    /// Stores `triangles` for `slot`. Returns false, leaving the slot unmasked,
    /// when they do not fit; the surface then fills its bounding box.
    pub fn push_surface(&mut self, slot: usize, triangles: &[MaskTriangle]) -> bool {
        if slot >= OCEAN_MAX_SURFACES || triangles.is_empty()
            || (self.triangles.len() / 2 + triangles.len()) * 2 > OCEAN_MASK_VEC4S
        {
            return false;
        }
        let minimum: [f32; 2] = std::array::from_fn(|axis| {
            triangles.iter().flat_map(|t| t.iter()).map(|p| p[axis]).fold(f32::INFINITY, f32::min)
        });
        let maximum: [f32; 2] = std::array::from_fn(|axis| {
            triangles.iter().flat_map(|t| t.iter()).map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max)
        });
        self.bounds[slot] = [minimum[0], minimum[1], maximum[0], maximum[1]];
        self.tiles[slot] = build_mask_tiles::<OCEAN_MASK_TILE_AXIS, 4>(
            triangles, minimum, maximum,
        );
        self.prune_tiles[slot] = build_mask_tiles::<OCEAN_PRUNE_TILE_AXIS, OCEAN_PRUNE_TILE_WORDS>(
            triangles, minimum, maximum,
        );
        self.ranges[slot] = [(self.triangles.len() / 2) as f32, triangles.len() as f32, 0.0, 0.0];
        for t in triangles {
            self.triangles.push([t[0][0], t[0][1], t[1][0], t[1][1]]);
            self.triangles.push([t[2][0], t[2][1], 0.0, 0.0]);
        }
        true
    }

    /// Whether `slot` carries a real outline. Unmasked slots fill their box.
    pub fn is_masked(&self, slot: usize) -> bool {
        slot < OCEAN_MAX_SURFACES && self.ranges[slot][1] > 0.0
    }

    /// CPU twin of `ocean_mask_contains` in bsp.wgsl, rain_sim.wgsl and
    /// ocean_optics.wgsl: whether the world xz point `p` lies on the slot's
    /// outline, with the same 0.75 unit growth. Unmasked slots contain everything.
    pub fn contains(&self, slot: usize, p: [f32; 2]) -> bool {
        if !self.is_masked(slot) {
            return true;
        }
        match mask_tile_code(self.bounds[slot], self.tiles[slot], p) {
            MASK_TILE_OUTSIDE => return false,
            MASK_TILE_INSIDE => return true,
            _ => {}
        }
        let first = self.ranges[slot][0] as usize;
        let count = self.ranges[slot][1] as usize;
        for i in first..first + count {
            let (Some(t0), Some(t1)) = (self.triangles.get(i * 2), self.triangles.get(i * 2 + 1))
            else {
                break;
            };
            let (a, b, c) = ([t0[0], t0[1]], [t0[2], t0[3]], [t1[0], t1[1]]);
            if (0..2).any(|axis| {
                p[axis] < a[axis].min(b[axis]).min(c[axis]) - MASK_TOLERANCE
                    || p[axis] > a[axis].max(b[axis]).max(c[axis]) + MASK_TOLERANCE
            }) {
                continue;
            }
            if mask_triangle_contains([a, b, c], p) {
                return true;
            }
        }
        false
    }

    /// Conservative cell/footprint overlap used to remove dry clipmap cells.
    /// Fine-grid boundary tiles remain present for the fragment mask to trim;
    /// only cells covered entirely by proven-dry tiles are omitted.
    pub fn rect_may_contain(&self, slot: usize, lo: [f32; 2], hi: [f32; 2]) -> bool {
        if let Some(footprint) = self.footprints.get(slot).and_then(Option::as_deref) {
            let intersects_sparse_tile = footprint.tiles.iter().any(|tile| {
                hi[0] >= tile.minimum[0]
                    && hi[1] >= tile.minimum[1]
                    && lo[0] <= tile.maximum[0]
                    && lo[1] <= tile.maximum[1]
            });
            if !intersects_sparse_tile {
                return false;
            }
        }
        if !self.is_masked(slot) {
            return true;
        }
        let bounds = self.bounds[slot];
        if hi[0] < bounds[0] || hi[1] < bounds[1] || lo[0] > bounds[2] || lo[1] > bounds[3] {
            return false;
        }
        let size = [(bounds[2] - bounds[0]).max(1.0e-4), (bounds[3] - bounds[1]).max(1.0e-4)];
        let cell = |value: f32, axis: usize| {
            (((value - bounds[axis]) / size[axis] * OCEAN_PRUNE_TILE_AXIS as f32).floor() as i32)
                .clamp(0, OCEAN_PRUNE_TILE_AXIS as i32 - 1) as usize
        };
        let x0 = cell(lo[0], 0);
        let x1 = cell(hi[0], 0);
        let y0 = cell(lo[1], 1);
        let y1 = cell(hi[1], 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if packed_tile_code(&self.prune_tiles[slot], y * OCEAN_PRUNE_TILE_AXIS + x)
                    != MASK_TILE_OUTSIDE
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn closest_point(&self, slot: usize, p: [f32; 2]) -> Option<[f32; 2]> {
        let footprint = self.footprints.get(slot)?.as_deref()?;
        let mut best = None::<([f32; 2], f32)>;
        for triangle in &footprint.triangles {
            if mask_triangle_contains(*triangle, p) {
                return Some(p);
            }
            for edge in 0..3 {
                let a = triangle[edge];
                let b = triangle[(edge + 1) % 3];
                let ab = [b[0] - a[0], b[1] - a[1]];
                let length_sq = ab[0] * ab[0] + ab[1] * ab[1];
                let t = if length_sq > 1.0e-8 {
                    (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length_sq)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
                let distance_sq = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                if best.is_none_or(|(_, old)| distance_sq < old) {
                    best = Some((q, distance_sq));
                }
            }
        }
        best.map(|(point, _)| point)
    }
}

/// A box of water the renderer reasons about: optics, rain and the camera's
/// "am I submerged" test. The box is the cluster's bounding rectangle, so a
/// surface with a hole (an island ring) additionally carries its outline slot;
/// without it the hole would count as water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterVolume {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
    /// Index into [`OceanMasks`], only for surfaces that really have an outline.
    pub mask_slot: Option<u8>,
}

impl WaterVolume {
    /// GPU encoding of `mask_slot` for a spare `w` lane: 0 is "no mask", and
    /// slot `n` is `n + 1`, so a zero-filled uniform stays unmasked.
    pub fn mask_lane(&self) -> f32 {
        self.mask_slot.map_or(0.0, |slot| f32::from(slot) + 1.0)
    }

    /// Whether the render-space point is inside this water, not merely its box.
    pub fn contains(&self, p: [f32; 3], masks: &OceanMasks) -> bool {
        (0..3).all(|axis| p[axis] >= self.minimum[axis] && p[axis] <= self.maximum[axis])
            && self.mask_slot.map_or(true, |slot| masks.contains(usize::from(slot), [p[0], p[2]]))
    }
}

pub use jka_assets::bsp::bathymetry::{
    build_water_footprints, cluster_water_faces, WaterFace, WaterFootprint,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeSettings {
    pub tile_length: [f32; 2],
    pub displacement_scale: f32,
    pub normal_scale: f32,
    pub wind_speed: f32,
    pub wind_direction: f32,
    pub fetch_length: f32,
    pub swell: f32,
    pub spread: f32,
    pub detail: f32,
    pub whitecap: f32,
    pub foam_amount: f32,
}

impl CascadeSettings {
    pub const fn upstream_defaults() -> [Self; OCEAN_CASCADES] {
        // Exact values from GodotOceanWaves/main.tscn.
        [
            Self { tile_length:[88.0,88.0], displacement_scale:1.0, normal_scale:1.0, wind_speed:10.0, wind_direction:20.0, fetch_length:150.0, swell:0.8, spread:0.2, detail:1.0, whitecap:0.5, foam_amount:8.0 },
            Self { tile_length:[57.0,57.0], displacement_scale:0.75, normal_scale:1.0, wind_speed:5.0, wind_direction:15.0, fetch_length:150.0, swell:0.8, spread:0.4, detail:1.0, whitecap:0.5, foam_amount:0.0 },
            Self { tile_length:[16.0,16.0], displacement_scale:0.0, normal_scale:0.25, wind_speed:20.0, wind_direction:20.0, fetch_length:550.0, swell:0.8, spread:0.4, detail:1.0, whitecap:0.25, foam_amount:3.0 },
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanSettings {
    pub interaction: bool,
    pub interaction_strength: f32,
    /// 0 off, 1 entry droplets, 2 wake spray, 3 droplets + wake,
    /// 4 entry jets, 5 droplets + jets + wake.
    pub interaction_splash_mode: u8,
    pub interaction_debug: bool,
    pub interaction_lifetime: f32,
    pub interaction_fade: f32,
    pub interaction_damping: f32,
    pub interaction_edge_fade: f32,
    pub optics: optics::Settings,
    pub bounds: Option<OceanSurface>,
    pub authored: OceanAuthoring,
    pub wind: AtmosphericWind,
    pub map_size: u32,
    pub mesh_quality: u8, // 0 low, 1 high (upstream MeshQuality)
    pub updates_per_second: f32,
    pub water_color: [f32; 3],
    pub foam_color: [f32; 3],
    pub roughness: f32,
    pub normal_strength: f32,
    pub sea_spray: bool,
    pub wind_foam_streaks: bool,
    /// Stop encoding FFT cascades and wind-foam transport while no ocean
    /// surface has been visible recently. The field simply holds its last
    /// state: a frozen wave field nobody can see is indistinguishable from an
    /// animated one nobody can see, and because `elapsed` stops advancing with
    /// it, the waves resume from where they left off instead of jumping.
    pub simulate_when_hidden: bool,
    pub cascades: [CascadeSettings; OCEAN_CASCADES],
}

impl Default for OceanSettings {
    fn default() -> Self {
        Self {
            interaction: false,
            interaction_strength: 1.0,
            interaction_splash_mode: 3,
            interaction_debug: false,
            interaction_lifetime: 2.5,
            interaction_fade: 1.0,
            interaction_damping: 2.0,
            interaction_edge_fade: 81.92,
            optics: optics::Settings::default(),
            bounds: None,
            authored: OceanAuthoring::default(),
            wind: AtmosphericWind::default(),
            map_size: OCEAN_DEFAULT_MAP_SIZE,
            mesh_quality: 1,
            updates_per_second: 48.0,
            // GodotOceanWaves water.gd defaults. Godot converts these from
            // sRGB to linear before the shader; the BSP shader does the same.
            water_color: [0.1, 0.15, 0.18],
            foam_color: [0.73, 0.67, 0.62],
            // mat_water.tres overrides the shader's 0.4 default to 0.65.
            roughness: 0.65,
            normal_strength: 1.0,
            sea_spray: true,
            simulate_when_hidden: false,
            wind_foam_streaks: true,
            cascades: {
                let mut cascades = CascadeSettings::upstream_defaults();
                cascades[2].displacement_scale = 0.08;
                cascades
            },
        }
    }
}

impl OceanSettings {
    pub fn sanitize(mut self) -> Self {
        self.interaction_strength = if self.interaction_strength.is_finite() { self.interaction_strength.max(0.0) } else { 1.0 };
        self.interaction_splash_mode = self.interaction_splash_mode.min(5);
        self.interaction_lifetime = if self.interaction_lifetime.is_finite() { self.interaction_lifetime.clamp(0.1,60.0) } else { 2.5 };
        self.interaction_fade = if self.interaction_fade.is_finite() { self.interaction_fade.clamp(0.0,self.interaction_lifetime) } else { 1.0 };
        self.interaction_damping = if self.interaction_damping.is_finite() { self.interaction_damping.max(0.0) } else { 2.0 };
        self.interaction_edge_fade = if self.interaction_edge_fade.is_finite() { self.interaction_edge_fade.clamp(0.0,256.0) } else { 81.92 };
        self.optics = self.optics.sanitize();
        self.authored = self.authored.sanitize();
        self.wind = self.wind.sanitize();
        self.map_size = match self.map_size {
            128 | 256 | 512 | 1024 => self.map_size,
            n if n < 192 => 128,
            n if n < 384 => 256,
            n if n < 768 => 512,
            _ => 1024,
        };
        self.mesh_quality = self.mesh_quality.min(1);
        self.updates_per_second = self.updates_per_second.clamp(0.0, 60.0);
        for value in self.water_color.iter_mut().chain(self.foam_color.iter_mut()) {
            *value = value.clamp(0.0, 1.0);
        }
        self.roughness = self.roughness.clamp(0.0, 1.0);
        self.normal_strength = self.normal_strength.clamp(0.0, 1.0);
        for cascade in &mut self.cascades {
            cascade.tile_length[0] = cascade.tile_length[0].max(0.0001);
            cascade.tile_length[1] = cascade.tile_length[1].max(0.0001);
            cascade.displacement_scale = cascade.displacement_scale.clamp(0.0, 2.0);
            cascade.normal_scale = cascade.normal_scale.clamp(0.0, 2.0);
            cascade.wind_speed = cascade.wind_speed.max(0.0001);
            cascade.wind_direction = cascade.wind_direction.clamp(-360.0, 360.0);
            cascade.fetch_length = cascade.fetch_length.max(0.0001);
            cascade.swell = cascade.swell.clamp(0.0, 2.0);
            cascade.spread = cascade.spread.clamp(0.0, 1.0);
            cascade.detail = cascade.detail.clamp(0.0, 1.0);
            cascade.whitecap = cascade.whitecap.clamp(0.0, 2.0);
            cascade.foam_amount = cascade.foam_amount.clamp(0.0, 10.0);
        }
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CascadeGpu {
    tile: [f32; 4],
    spectrum: [f32; 4],
    shape: [f32; 4],
    foam: [f32; 4],
    seed: [u32; 4],
    time: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OceanState { values: [u32; 4] }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WindrowGpu {
    map_scales: [[f32; 4]; OCEAN_CASCADES],
    wind_time: [f32; 4], // render XZ wind m/s, elapsed seconds, frame delta
    params: [f32; 4], // domain metres, source strength, lifetime seconds, previous layer
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanSurface {
    pub plane_height: f32,
    pub minimum: [f32; 2],
    pub maximum: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OceanEnvironment {
    sun_direction_intensity: [f32; 4],
    sun_color: [f32; 4],
    sky_ambient: [f32; 4],
}

impl Default for OceanEnvironment {
    fn default() -> Self {
        Self {
            // Direction is light travel direction in renderer coordinates.
            sun_direction_intensity: [-0.3, -0.8, -0.2, 1.0],
            sun_color: [1.0, 1.0, 1.0, 1.0],
            // RGB is linear sky irradiance tint; W tells the shader that a real map skybox exists.
            sky_ambient: [0.28, 0.38, 0.52, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct OceanRenderUniform {
    pub map_scales: [[f32; 4]; OCEAN_CASCADES],
    pub water_color: [f32; 4],
    pub foam_color: [f32; 4],
    pub ocean_info: [f32; 4], // enabled, map size, JKA units per meter, windrow domain metres
    pub surface: [f32; 4],    // roughness, normal strength, sea spray enabled, wind foam streaks enabled
    pub clipmap: [f32; 4],    // centre x, centre z, coarsest cell size, active windrow layer
    pub sun_direction_intensity: [f32; 4], // xyz light-travel direction, w relative intensity (250 q3map units = 1)
    pub sun_color: [f32; 4],
    pub sky_ambient: [f32; 4], // rgb linear map-sky average, w map has real skybox
    pub spray_info: [f32; 4],  // surface count, total particles, visible lifetime, lifetime randomness
    pub spray_bounds: [[f32; 4]; OCEAN_MAX_SURFACES], // min xz, max xz in world units
    pub spray_planes: [[f32; 4]; OCEAN_MAX_SURFACES], // height, promoted activity, mask slot, foam gain
    pub swell: [f32; 4], // render XZ direction, wave number / metre, amplitude metres
    pub swell_motion: [f32; 4], // angular speed, primary chop, FFT chop, seconds
    pub wind_motion: [f32; 4], // render XZ wind m/s, spray multiplier, promoted clipmap outer spacing
    pub authored_bounds: [f32;4],
    pub authored_plane: [f32;4], // bounds enabled, height, manual appearance, promoted view
    pub mask_ranges: [[f32; 4]; OCEAN_MAX_SURFACES], // first triangle, triangle count
    pub mask_bounds: [[f32; 4]; OCEAN_MAX_SURFACES], // min xz, max xz for tile lookup
    pub mask_tiles: [[u32; 4]; OCEAN_MAX_SURFACES], // 2-bit 8x8 classification
    pub mask_triangles: [[f32; 4]; OCEAN_MASK_VEC4S], // per triangle: a.xy b.xy / c.xy
}

struct PromotedRenderSurface {
    descriptor: appearance::PromotedOcean,
    settings: OceanSettings,
    manual: bool,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct OceanGpu {
    pub coastal: Option<coastal::Gpu>,
    coastal_render: std::sync::Arc<coastal::RenderResources>,
    coastal_key: Option<(usize, u8, bool, u32, u32)>,
    enabled: bool,
    settings: OceanSettings,
    clipmap_center: [f32; 4],
    environment: OceanEnvironment,
    surfaces: [OceanSurface; OCEAN_MAX_SURFACES],
    surface_count: usize,
    masks: std::sync::Arc<OceanMasks>,
    times: [f32; OCEAN_CASCADES],
    elapsed: f32,
    next_update: f32,
    // One phase/dispatch is encoded at a time; FFT scratch is shared by all
    // cascades, so phase order is strictly cascade 2 -> 1 -> 0.
    phase: usize,
    phase_count: usize, // 0 while idle; 15 FFT passes plus optional spectrum regeneration
    phase_spectrum: bool,
    phase_started_at: f32,
    phase_period: f32,
    spectrum_dirty: bool,
    params_buffer: wgpu::Buffer,
    params_upload_buffer: wgpu::Buffer,
    _spectrum_buffer: wgpu::Buffer,
    _butterfly_buffer: wgpu::Buffer,
    _fft_buffer: wgpu::Buffer,
    _foam_buffer: wgpu::Buffer,
    _displacement: wgpu::Texture,
    _normal: wgpu::Texture,
    compute_bind_groups: Vec<wgpu::BindGroup>,
    spectrum_pipeline: wgpu::ComputePipeline,
    butterfly_pipeline: wgpu::ComputePipeline,
    modulate_pipeline: wgpu::ComputePipeline,
    fft_pipeline: wgpu::ComputePipeline,
    transpose_pipeline: wgpu::ComputePipeline,
    unpack_pipeline: wgpu::ComputePipeline,
    _windrow_field_buffer: wgpu::Buffer,
    windrow_bind_group: wgpu::BindGroup,
    windrow_pipeline: wgpu::ComputePipeline,
    windrow_params_buffer: wgpu::Buffer,
    windrow_front: usize,
    render_buffer: wgpu::Buffer,
    pub render_bind_group: wgpu::BindGroup,
    render_layout: wgpu::BindGroupLayout,
    render_displacement_view: wgpu::TextureView,
    render_normal_view: wgpu::TextureView,
    render_sampler: wgpu::Sampler,
    spray_view: wgpu::TextureView,
    promoted_surfaces: Vec<PromotedRenderSurface>,
    interaction_view: wgpu::TextureView,
    interaction_uniform: wgpu::Buffer,
    interaction_splash_uniform: wgpu::Buffer,
}

fn jonswap_alpha(wind_speed: f32, fetch_m: f32) -> f32 {
    0.076 * (wind_speed*wind_speed/(fetch_m*G)).powf(0.22)
}
fn jonswap_peak(wind_speed: f32, fetch_m: f32) -> f32 {
    22.0 * (G*G/(wind_speed*fetch_m)).powf(1.0/3.0)
}

pub(crate) fn create_spray_pipeline(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
    render_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    let spray_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("GodotOceanWaves sea spray port"),
        source: wgpu::ShaderSource::Wgsl(include_str!("ocean_spray.wgsl").into()),
    });
    let spray_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Ocean sea spray pipeline layout"),
        bind_group_layouts: &[Some(camera_layout), Some(render_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Ocean sea spray pipeline"),
        layout: Some(&spray_layout),
        vertex: wgpu::VertexState {
            module: &spray_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &spray_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl OceanGpu {
    pub fn create_inert_render_bind_group(
        device: &wgpu::Device,
        render_layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        let output_desc = |label: &'static str| wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: OCEAN_CASCADES as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let displacement = device.create_texture(&output_desc("Ocean inert displacement"));
        let normal = device.create_texture(&output_desc("Ocean inert normal"));
        let windrow = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ocean inert wind foam streaks"),
            size: 8,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let spray_albedo = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Ocean inert spray albedo"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let displacement_view = displacement.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let normal_view = normal.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let spray_albedo_view = spray_albedo.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Ocean inert sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let interaction = interaction::RenderResources::inert(device);
        let coastal = coastal::RenderResources::inert(device);
        let settings = OceanSettings::default();
        let render_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Ocean inert render uniform"),
            contents: bytemuck::bytes_of(&make_render_uniform(false, &settings)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ocean inert render bind group"),
            layout: render_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&displacement_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&normal_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                bg_buffer(3, &render_buffer),
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&spray_albedo_view) },
                bg_buffer(5, &windrow),
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&interaction.view) },
                bg_buffer(7, &interaction.uniform),
                bg_buffer(8, &interaction.splash_uniform),
                bg_buffer(9, &coastal.params),
                bg_buffer(10, &coastal.lookup),
                bg_buffer(11, &coastal.surface),
            ],
        })
    }

    pub fn create_render_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:Some("Ocean render layout"), entries:&[
                wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false},count:None},
                wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false},count:None},
                wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
                uniform_layout_entry(3,wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry{binding:4,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
                wgpu::BindGroupLayoutEntry{binding:5,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None},
                wgpu::BindGroupLayoutEntry{binding:6,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false},count:None},
                uniform_layout_entry(7,wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT),
                uniform_layout_entry(8,wgpu::ShaderStages::VERTEX),
                uniform_layout_entry(9,wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry{binding:10,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None},
                wgpu::BindGroupLayoutEntry{binding:11,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None},
            ]
        })
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_layout: &wgpu::BindGroupLayout,
        spray_albedo_view: &wgpu::TextureView,
        requested_settings: OceanSettings,
    ) -> Self {
        let interaction = interaction::RenderResources::inert(device);
        Self::new_with_interaction(device, queue, render_layout, spray_albedo_view, requested_settings, &interaction)
    }

    pub fn new_with_interaction(
        device: &wgpu::Device, queue: &wgpu::Queue, render_layout: &wgpu::BindGroupLayout,
        spray_albedo_view: &wgpu::TextureView, requested_settings: OceanSettings,
        interaction: &interaction::RenderResources,
    ) -> Self {
        let settings = requested_settings.sanitize();
        let map_size = settings.map_size;
        let stages = map_size.ilog2();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GodotOceanWaves faithful compute port"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ocean_compute.wgsl").into()),
        });
        let storage = |label: &str, size: u64| device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation:false
        });
        let n = u64::from(map_size);
        let params_size=(std::mem::size_of::<CascadeGpu>()*OCEAN_CASCADES) as u64;
        let params_buffer = storage("Ocean cascade parameters", params_size);
        let params_upload_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label:Some("Ocean cascade parameter upload"),
            size:params_size,
            usage:wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation:false,
        });
        let spectrum_buffer = storage("Ocean spectrum", n*n*OCEAN_CASCADES as u64*16);
        let butterfly_buffer = storage("Ocean FFT butterfly", n*u64::from(stages)*16);
        let fft_buffer = storage("Ocean FFT pingpong", n*n*4*2*8);
        let foam_buffer = storage("Ocean persistent foam", n*n*OCEAN_CASCADES as u64*4);
        let output_desc = |label: &'static str| wgpu::TextureDescriptor {
            label: Some(label), size: wgpu::Extent3d { width:map_size,height:map_size,depth_or_array_layers:OCEAN_CASCADES as u32 },
            mip_level_count:1, sample_count:1, dimension:wgpu::TextureDimension::D2,
            format:wgpu::TextureFormat::Rgba16Float,
            usage:wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:&[],
        };
        let displacement = device.create_texture(&output_desc("Ocean displacement array"));
        let normal = device.create_texture(&output_desc("Ocean normal/foam array"));
        let displacement_view = displacement.create_view(&wgpu::TextureViewDescriptor { dimension:Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let normal_view = normal.create_view(&wgpu::TextureViewDescriptor { dimension:Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });

        // Separate persistent surface-foam field. Two halves of one storage
        // buffer are ping-ponged. Compute reads only the previous half and writes
        // only the next half, while the BSP shader performs the small bilinear
        // sample itself. This avoids texture read/write aliasing across backends.
        let windrow_field_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ocean wind foam streak field"),
            size: u64::from(OCEAN_WINDROW_SIZE) * u64::from(OCEAN_WINDROW_SIZE) * 2 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:Some("Ocean compute layout"), entries:&[
                storage_entry(0,true), storage_entry(1,false), storage_entry(2,false), storage_entry(3,false), storage_entry(4,false),
                storage_texture_entry(5), storage_texture_entry(6), uniform_entry(7),
            ]
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label:Some("Ocean compute pipeline layout"), bind_group_layouts:&[Some(&compute_layout)], immediate_size:0 });
        let cp = |label:&str, entry:&str| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:Some(label), layout:Some(&pipeline_layout), module:&shader, entry_point:Some(entry), compilation_options:Default::default(), cache:None
        });
        let spectrum_pipeline = cp("Ocean spectrum compute","spectrum_compute");
        let butterfly_pipeline = cp("Ocean FFT butterfly","fft_butterfly");
        let modulate_pipeline = cp("Ocean spectrum modulate","spectrum_modulate");
        let fft_pipeline = cp("Ocean Stockham FFT","fft_compute");
        let transpose_pipeline = cp("Ocean FFT transpose","transpose");
        let unpack_pipeline = cp("Ocean FFT unpack","fft_unpack");
        let state_buffers: Vec<_> = (0..OCEAN_CASCADES).map(|cascade| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some("Ocean cascade state"), contents:bytemuck::bytes_of(&OceanState{values:[cascade as u32,map_size,OCEAN_CASCADES as u32,0]}), usage:wgpu::BufferUsages::UNIFORM
        })).collect();
        let compute_bind_groups = state_buffers.iter().map(|state| device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:Some("Ocean compute bind group"), layout:&compute_layout, entries:&[
                bg_buffer(0,&params_buffer), bg_buffer(1,&spectrum_buffer), bg_buffer(2,&butterfly_buffer), bg_buffer(3,&fft_buffer), bg_buffer(4,&foam_buffer),
                wgpu::BindGroupEntry{binding:5,resource:wgpu::BindingResource::TextureView(&displacement_view)},
                wgpu::BindGroupEntry{binding:6,resource:wgpu::BindingResource::TextureView(&normal_view)},
                bg_buffer(7,state),
            ]
        })).collect::<Vec<_>>();

        let windrow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ocean wind foam streak compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ocean_windrow.wgsl").into()),
        });
        let windrow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ocean wind foam streak compute layout"),
            entries: &[
                storage_entry(0, false),
                wgpu::BindGroupLayoutEntry { binding:1, visibility:wgpu::ShaderStages::COMPUTE, ty:wgpu::BindingType::Texture { sample_type:wgpu::TextureSampleType::Float { filterable:true }, view_dimension:wgpu::TextureViewDimension::D2Array, multisampled:false }, count:None },
                wgpu::BindGroupLayoutEntry { binding:2, visibility:wgpu::ShaderStages::COMPUTE, ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count:None },
                uniform_layout_entry(3, wgpu::ShaderStages::COMPUTE),
            ],
        });
        let windrow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Ocean wind foam streak pipeline layout"),
            bind_group_layouts: &[Some(&windrow_layout)],
            immediate_size: 0,
        });
        let windrow_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Ocean wind foam streak pipeline"),
            layout: Some(&windrow_pipeline_layout),
            module: &windrow_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let windrow_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Ocean wind foam streak parameters"),
            contents: bytemuck::bytes_of(&WindrowGpu::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let render_uniform = make_render_uniform(false,&settings);
        let coastal_render = coastal::RenderResources::inert(device);
        let render_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label:Some("Ocean render uniform"), contents:bytemuck::bytes_of(&render_uniform), usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label:Some("Ocean repeat sampler"), address_mode_u:wgpu::AddressMode::Repeat,address_mode_v:wgpu::AddressMode::Repeat,address_mode_w:wgpu::AddressMode::ClampToEdge,mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,mipmap_filter:wgpu::MipmapFilterMode::Linear,..Default::default() });
        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label:Some("Ocean render bind group"), layout:render_layout, entries:&[
            wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&displacement_view)},
            wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::TextureView(&normal_view)},
            wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::Sampler(&sampler)},
            bg_buffer(3,&render_buffer),
            wgpu::BindGroupEntry{binding:4,resource:wgpu::BindingResource::TextureView(spray_albedo_view)},
            bg_buffer(5, &windrow_field_buffer),
            wgpu::BindGroupEntry{binding:6,resource:wgpu::BindingResource::TextureView(&interaction.view)},
            bg_buffer(7, &interaction.uniform),
            bg_buffer(8, &interaction.splash_uniform),
            bg_buffer(9, &coastal_render.params),
            bg_buffer(10, &coastal_render.lookup),
            bg_buffer(11, &coastal_render.surface),
        ]});
        let windrow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ocean wind foam streak compute bind group"),
            layout: &windrow_layout,
            entries: &[
                bg_buffer(0, &windrow_field_buffer),
                wgpu::BindGroupEntry { binding:1, resource:wgpu::BindingResource::TextureView(&normal_view) },
                wgpu::BindGroupEntry { binding:2, resource:wgpu::BindingResource::Sampler(&sampler) },
                bg_buffer(3, &windrow_params_buffer),
            ],
        });

        let mut ocean = Self { enabled:false,settings,clipmap_center:[0.0;4],environment:OceanEnvironment::default(),surfaces:[OceanSurface{plane_height:0.0,minimum:[0.0;2],maximum:[0.0;2]};OCEAN_MAX_SURFACES],surface_count:0,masks:std::sync::Arc::new(OceanMasks::default()),times:[120.0,120.0+std::f32::consts::PI,120.0+2.0*std::f32::consts::PI],elapsed:0.0,next_update:0.0,phase:0,phase_count:0,phase_spectrum:false,phase_started_at:0.0,phase_period:0.0,spectrum_dirty:true,params_buffer,params_upload_buffer,_spectrum_buffer:spectrum_buffer,_butterfly_buffer:butterfly_buffer,_fft_buffer:fft_buffer,_foam_buffer:foam_buffer,_displacement:displacement,_normal:normal,compute_bind_groups,spectrum_pipeline,butterfly_pipeline,modulate_pipeline,fft_pipeline,transpose_pipeline,unpack_pipeline,_windrow_field_buffer:windrow_field_buffer,windrow_bind_group,windrow_pipeline,windrow_params_buffer,windrow_front:0,render_buffer,render_bind_group, render_layout:render_layout.clone(),
            render_displacement_view:displacement_view, render_normal_view:normal_view,
            render_sampler:sampler, spray_view:spray_albedo_view.clone(), promoted_surfaces:Vec::new(),
            interaction_view: interaction.view.clone(), interaction_uniform: interaction.uniform.clone(),
            interaction_splash_uniform: interaction.splash_uniform.clone(),
            coastal: None, coastal_render, coastal_key: None };
        ocean.upload_params(queue,0.0);
        let mut encoder=device.create_command_encoder(&wgpu::CommandEncoderDescriptor{label:Some("Ocean FFT init")});
        {
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("Ocean butterfly init"),timestamp_writes:None});
            pass.set_pipeline(&ocean.butterfly_pipeline);
            pass.set_bind_group(0,&ocean.compute_bind_groups[0],&[]);
            pass.dispatch_workgroups(map_size/128,stages,1);
        }
        queue.submit(Some(encoder.finish()));
        ocean
    }

    pub fn map_size(&self) -> u32 { self.settings.map_size }

    pub fn set_time(&mut self, seconds: f32) {
        if !seconds.is_finite() { return; }
        if (seconds - self.elapsed).abs() > 0.05 {
            self.elapsed = seconds;
            self.times = std::array::from_fn(|i| seconds + 120.0 + i as f32 * std::f32::consts::PI);
            self.next_update = seconds;
            // Abandon only scratch work; each visible cascade is written in
            // one final unpack dispatch, never a partly computed FFT stage.
            self.phase = 0;
            self.phase_count = 0;
        }
    }

    pub fn set_enabled(&mut self, queue:&wgpu::Queue, enabled:bool) {
        self.enabled=enabled;
        self.write_render_uniform(queue);
    }

    fn write_render_uniform(&self, queue:&wgpu::Queue) {
        let mut uniform=make_render_uniform(self.enabled,&self.settings);
        uniform.clipmap=self.clipmap_center;
        uniform.clipmap[3] = self.windrow_front as f32;
        uniform.swell_motion[3] = self.elapsed;
        let wind = self.settings.wind.at(self.elapsed);
        uniform.wind_motion[0] = wind[0] / OCEAN_UNITS_PER_METER;
        uniform.wind_motion[1] = -wind[1] / OCEAN_UNITS_PER_METER;
        apply_scene_context(&mut uniform, self.environment, &self.surfaces, self.surface_count);
        uniform.mask_ranges = self.masks.ranges;
        uniform.mask_bounds = self.masks.bounds;
        uniform.mask_tiles = self.masks.tiles;
        uniform.mask_triangles[..self.masks.triangles.len()].copy_from_slice(&self.masks.triangles);
        queue.write_buffer(&self.render_buffer,0,bytemuck::bytes_of(&uniform));
        for surface in &self.promoted_surfaces {
            let local = make_promoted_render_uniform(&uniform, &surface.settings,
                &surface.descriptor, surface.manual, self.promoted_surfaces.len());
            queue.write_buffer(&surface.buffer,0,bytemuck::bytes_of(&local));
        }
    }

    pub fn render_bind_group_for(&self, index: usize) -> &wgpu::BindGroup {
        self.promoted_surfaces
            .iter()
            .find(|s| s.descriptor.index == index)
            .map_or(&self.render_bind_group, |s| &s.bind_group)
    }

    /// Rebuild only when the map/selection/amplitude changes. Failed selections
    /// are remembered so unsupported pools do not retry every render frame.
    pub fn configure_coastal(
        &mut self, device: &wgpu::Device, queue: &wgpu::Queue,
        enabled: bool, pool: u8,
        atlas: Option<&std::sync::Arc<jka_assets::bsp::bathymetry::Atlas>>,
        promoted: &[appearance::PromotedOcean], overrides: &[(usize, OceanSettings)],
    ) {
        let pointer=atlas.map_or(0,|a|std::sync::Arc::as_ptr(a) as usize);
        let mut candidates:Vec<_>=promoted.iter().filter(|p|pool==0 || p.index+1==pool as usize).collect();
        let amplitude=|p:&appearance::PromotedOcean|overrides.iter().find(|(slot,_)|*slot==p.index)
            .map_or(p.heuristic.amplitude,|(_,s)|s.authored.amplitude);
        let signature=candidates.iter().fold(0u32,|h,p| {
            let wavelength=overrides.iter().find(|(slot,_)|*slot==p.index)
                .map_or(p.heuristic.wavelength,|(_,s)|s.authored.wavelength);
            h.wrapping_mul(31)^amplitude(p).to_bits()^wavelength.to_bits()
        });
        let key=(pointer,pool,enabled,signature,self.settings.map_size);
        if self.coastal_key==Some(key) {return;}
        self.coastal_key=Some(key);
        self.coastal=None;
        queue.write_buffer(&self.coastal_render.params,16,bytemuck::cast_slice(&[0u32]));
        if !enabled || pointer==0 {return;}
        if pool==0 {
            candidates.sort_by_key(|p| std::cmp::Reverse(atlas.map_or(0,|a| a.grids.iter()
                .filter(|g|g.slot as usize==p.index)
                .flat_map(|g|g.cells.iter().filter(|c|c.class==jka_assets::bsp::bathymetry::Class::Wet
                    && c.depth/p.heuristic.wavelength.max(1.0)<=0.05))
                .count())));
        }
        let mut rejection="no matching promoted pool".to_owned();
        for descriptor in candidates {
            // Use the effective per-pool wavelength for regime eligibility.
            let settings=appearance::surface_settings(self.settings,descriptor,
                overrides.iter().find(|(slot,_)|*slot==descriptor.index).map(|(_,s)|*s));
            let mut effective=descriptor.clone();
            effective.heuristic.wavelength=settings.authored.wavelength;
            match coastal::prepare(atlas.unwrap(),&effective) {
                Ok(prepared)=> {
                    let gpu=coastal::Gpu::new(device,prepared,&self.render_displacement_view,&self.render_sampler,amplitude(descriptor));
                    eprintln!("COASTAL LIVE: {}",gpu.summary);
                    self.coastal_render=std::sync::Arc::clone(&gpu.render);
                    self.coastal=Some(gpu);
                    // Existing promoted groups reference the old field buffers.
                    // The following set_promoted_surfaces rebuilds them once.
                    self.promoted_surfaces.clear();
                    self.render_bind_group=device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label:Some("Ocean render with coastal field"),layout:&self.render_layout,entries:&[
                            wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&self.render_displacement_view)},
                            wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::TextureView(&self.render_normal_view)},
                            wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::Sampler(&self.render_sampler)},
                            bg_buffer(3,&self.render_buffer),
                            wgpu::BindGroupEntry{binding:4,resource:wgpu::BindingResource::TextureView(&self.spray_view)},
                            bg_buffer(5,&self._windrow_field_buffer),
                            wgpu::BindGroupEntry{binding:6,resource:wgpu::BindingResource::TextureView(&self.interaction_view)},
                            bg_buffer(7,&self.interaction_uniform),bg_buffer(8,&self.interaction_splash_uniform),
                            bg_buffer(9,&self.coastal_render.params),bg_buffer(10,&self.coastal_render.lookup),bg_buffer(11,&self.coastal_render.surface),
                        ]});
                    return;
                }
                Err(reason)=>rejection=format!("pool {}: {reason}",descriptor.index+1),
            }
        }
        eprintln!("COASTAL LIVE: FFT fallback; {rejection}");
    }

    pub fn encode_coastal(&mut self,queue:&wgpu::Queue,encoder:&mut wgpu::CommandEncoder,delta:f32) {
        let Some(coastal)=self.coastal.as_mut() else {return;};
        let Some(surface)=self.promoted_surfaces.iter().find(|s|s.descriptor.index==coastal.slot) else {return;};
        let mut shared=make_render_uniform(self.enabled,&self.settings);
        shared.swell_motion[3]=self.elapsed;
        let wave=make_promoted_render_uniform(&shared,&surface.settings,&surface.descriptor,surface.manual,self.promoted_surfaces.len());
        coastal.encode(queue,encoder,delta,&wave);
    }

    pub fn set_promoted_surfaces(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        descriptors: &[appearance::PromotedOcean],
        overrides: &[(usize, OceanSettings)],
    ) {
        let desired_settings = |p: &appearance::PromotedOcean| {
            let local = overrides
                .iter()
                .find(|(index, _)| *index == p.index)
                .map(|(_, s)| *s);
            (
                appearance::surface_settings(self.settings, p, local),
                local.is_some(),
            )
        };
        if self.promoted_surfaces.len() == descriptors.len()
            && self
                .promoted_surfaces
                .iter()
                .zip(descriptors)
                .all(|(s, p)| {
                    let (settings, manual) = desired_settings(p);
                    s.descriptor == *p && s.settings == settings && s.manual == manual
                })
        {
            return;
        }
        let mut previous = std::mem::take(&mut self.promoted_surfaces);
        for descriptor in descriptors {
            let (settings, manual) = desired_settings(descriptor);
            let mut view = if let Some(i) = previous
                .iter()
                .position(|s| s.descriptor.index == descriptor.index)
            {
                previous.swap_remove(i)
            } else {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Promoted ocean render parameters"),
                    size: std::mem::size_of::<OceanRenderUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Promoted ocean shared FFT view"),
                    layout: &self.render_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &self.render_displacement_view,
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&self.render_normal_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.render_sampler),
                        },
                        bg_buffer(3, &buffer),
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&self.spray_view),
                        },
                        bg_buffer(5, &self._windrow_field_buffer),
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.interaction_view) },
                        bg_buffer(7, &self.interaction_uniform),
                        bg_buffer(8, &self.interaction_splash_uniform),
                        bg_buffer(9, &self.coastal_render.params),
                        bg_buffer(10, &self.coastal_render.lookup),
                        bg_buffer(11, &self.coastal_render.surface),
                    ],
                });
                PromotedRenderSurface {
                    descriptor: descriptor.clone(),
                    settings,
                    manual,
                    buffer,
                    bind_group,
                }
            };
            view.descriptor = descriptor.clone();
            view.settings = settings;
            view.manual = manual;
            self.promoted_surfaces.push(view);
        }
        self.write_render_uniform(queue);
    }

    /// Hands the ocean the outlines of the world's promoted surfaces. They are
    /// indexed by the slot baked into each clipmap, which is not the order of
    /// `set_surfaces` (that list drops surfaces an authored ocean covers).
    pub fn set_masks(&mut self, queue: &wgpu::Queue, masks: &std::sync::Arc<OceanMasks>) {
        if std::sync::Arc::ptr_eq(&self.masks, masks) { return; }
        self.masks = std::sync::Arc::clone(masks);
        self.write_render_uniform(queue);
    }

    /// Re-centres the clipmap on the viewer. The centre is snapped to the
    /// coarsest cell size so every level stays aligned to the same lattice;
    /// because the wave field is addressed in world space the surface does not
    /// swim when the grid steps.
    pub fn set_clipmap_center(&mut self, queue:&wgpu::Queue, camera:[f32;3], coarsest_spacing:f32) {
        let step=coarsest_spacing.max(1.0);
        let center=[(camera[0]/step).round()*step,(camera[2]/step).round()*step,step,0.0];
        if center==self.clipmap_center { return; }
        self.clipmap_center=center;
        self.write_render_uniform(queue);
    }

    /// Feeds the FFT water the same authored sun/sky context used by the rest of the map.
    /// `sun_direction` is the renderer-space direction the light travels.
    pub fn set_environment(
        &mut self,
        queue: &wgpu::Queue,
        sun_direction: [f32; 3],
        sun_color: [f32; 3],
        sun_intensity: f32,
        sky_average: [f32; 3],
        has_skybox: bool,
    ) {
        let length = (sun_direction[0]*sun_direction[0] + sun_direction[1]*sun_direction[1] + sun_direction[2]*sun_direction[2]).sqrt();
        let direction = if length > 1.0e-6 {
            [sun_direction[0]/length, sun_direction[1]/length, sun_direction[2]/length]
        } else {
            [-0.3, -0.8, -0.2]
        };
        let environment = OceanEnvironment {
            sun_direction_intensity: [direction[0], direction[1], direction[2], (sun_intensity / 250.0).clamp(0.0, 4.0)],
            sun_color: [sun_color[0].max(0.0), sun_color[1].max(0.0), sun_color[2].max(0.0), 1.0],
            sky_ambient: [sky_average[0].max(0.0), sky_average[1].max(0.0), sky_average[2].max(0.0), if has_skybox { 1.0 } else { 0.0 }],
        };
        if environment == self.environment { return; }
        self.environment = environment;
        self.write_render_uniform(queue);
    }

    pub fn set_surfaces(&mut self, queue: &wgpu::Queue, surfaces: &[OceanSurface]) {
        let count = surfaces.len().min(OCEAN_MAX_SURFACES);
        let mut next = [OceanSurface { plane_height: 0.0, minimum: [0.0; 2], maximum: [0.0; 2] }; OCEAN_MAX_SURFACES];
        next[..count].copy_from_slice(&surfaces[..count]);
        if count == self.surface_count && next == self.surfaces { return; }
        self.surface_count = count;
        self.surfaces = next;
        self.write_render_uniform(queue);
    }

    pub fn set_settings(&mut self, queue:&wgpu::Queue, requested:OceanSettings) {
        let settings=requested.sanitize();
        debug_assert_eq!(settings.map_size,self.settings.map_size,"map-size changes recreate OceanGpu");
        if settings == self.settings { return; }
        let old_updates=self.settings.updates_per_second;
        let spectrum_changed = settings.authored.seed != self.settings.authored.seed
            || settings.authored.direction != self.settings.authored.direction
            || settings.wind != self.settings.wind
            || settings.cascades.iter().zip(self.settings.cascades.iter()).any(|(a,b)| {
            a.tile_length != b.tile_length || a.wind_speed != b.wind_speed || a.wind_direction != b.wind_direction ||
            a.fetch_length != b.fetch_length || a.swell != b.swell || a.spread != b.spread || a.detail != b.detail
        });
        // water.gd preserves the phase of the update scheduler when the UPS
        // slider changes instead of restarting the schedule from "now".
        if settings.updates_per_second != old_updates {
            let old_period=1.0/(old_updates+1e-10);
            let new_period=1.0/(settings.updates_per_second+1e-10);
            self.next_update -= old_period-new_period;
        }
        self.settings=settings;
        self.spectrum_dirty |= spectrum_changed;
        // Whitecap/foam rates and displacement/normal scales do not require a
        // spectrum rebuild, but do need fresh compute/render parameters.
        // Do not replace the GPU parameters halfway through a five-pass FFT:
        // every phase in that cascade must use the same spectrum/time/foam data.
        // Changed settings are staged at the next simulation update instead.
        if self.phase_count == 0 {
            self.upload_params(queue,0.0);
        }
        self.write_render_uniform(queue);
    }

    fn build_params(&mut self, delta:f32) -> [CascadeGpu;OCEAN_CASCADES] {
        let mut gpu=[CascadeGpu::zeroed();OCEAN_CASCADES];
        for (i, gpu_param) in gpu.iter_mut().enumerate() {
            let s=self.settings.cascades[i]; self.times[i]+=delta;
            let authored = self.settings.authored;
            let weather = self.settings.wind;
            let fetch=s.fetch_length.max(0.0001)*1000.0;
            let wind=if i == 2 { (weather.speed / OCEAN_UNITS_PER_METER).max(0.01) } else { s.wind_speed.max(0.0001) };
            let direction = if i == 2 { weather.direction } else { authored.direction + s.wind_direction };
            let resolved_wind = weather.at(self.elapsed);
            let wind_strength = (resolved_wind[0].hypot(resolved_wind[1]) / 800.0).clamp(0.0, 2.0);
            let derivative_scale = if i == 2 { s.displacement_scale * authored.wind_chop * wind_strength } else {
                s.displacement_scale * authored.amplitude / 128.0 * authored.slosh
                    / (authored.wavelength / 1400.0) * authored.steepness / 0.65
            };
            *gpu_param=CascadeGpu {
                tile:[s.tile_length[0],s.tile_length[1],derivative_scale,0.0],
                spectrum:[jonswap_alpha(wind,fetch),jonswap_peak(wind,fetch),wind,(direction + 180.0).to_radians()],
                shape:[DEPTH_METERS,s.swell,s.detail,s.spread],
                foam:[s.whitecap + wind_strength * 0.15,delta*s.foam_amount*7.5*authored.foam*(0.25+wind_strength),delta/authored.foam_lifetime,0.0],
                seed:[authored.seed,authored.seed ^ (i as u32).wrapping_mul(0x9e3779b9),0,0], time:[self.times[i],0.0,0.0,0.0]
            };
        }
        gpu
    }

    fn upload_params(&mut self, queue:&wgpu::Queue, delta:f32) {
        let gpu=self.build_params(delta);
        queue.write_buffer(&self.params_buffer,0,bytemuck::cast_slice(&gpu));
    }

    fn stage_params(&mut self, queue:&wgpu::Queue, encoder:&mut wgpu::CommandEncoder, delta:f32) {
        let gpu=self.build_params(delta);
        queue.write_buffer(&self.params_upload_buffer,0,bytemuck::cast_slice(&gpu));
        encoder.copy_buffer_to_buffer(
            &self.params_upload_buffer,
            0,
            &self.params_buffer,
            0,
            std::mem::size_of_val(&gpu) as u64,
        );
    }

    /// Whether this ocean will draw sea spray, i.e. needs the shared spray pipeline.
    pub fn spray_active(&self) -> bool {
        self.enabled
            && (self.settings.sea_spray
                || (self.settings.interaction && self.settings.interaction_splash_mode > 0)
                || self.promoted_surfaces.iter().any(|s| {
                    s.settings.sea_spray
                        || (s.settings.interaction && s.settings.interaction_splash_mode > 0)
                }))
    }

    pub fn draw_spray<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bind_group: &'a wgpu::BindGroup,
        spray_pipeline: Option<&'a wgpu::RenderPipeline>,
    ) {
        if !self.spray_active() || (self.surface_count == 0 && self.promoted_surfaces.is_empty()) {
            return;
        }
        let Some(spray_pipeline) = spray_pipeline else {
            return;
        };
        pass.set_pipeline(spray_pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        // Each promoted emitter binds the very same wave controls as its
        // surface. Share the original particle budget and FFT textures.
        let splash_start = 32_768;
        let splash_end = splash_start
            + interaction::SPLASH_EVENT_CAPACITY * interaction::SPLASH_INSTANCES_PER_EVENT;
        if self.promoted_surfaces.is_empty() {
            pass.set_bind_group(1, &self.render_bind_group, &[]);
            if self.settings.sea_spray {
                pass.draw(0..6, 0..32_768);
            }
            if self.settings.interaction && self.settings.interaction_splash_mode > 0 {
                pass.draw(0..6, splash_start..splash_end);
            }
        } else {
            let particles = 32768 / self.promoted_surfaces.len().max(1) as u32;
            for surface in &self.promoted_surfaces {
                let has_splashes =
                    surface.settings.interaction && surface.settings.interaction_splash_mode > 0;
                if !surface.settings.sea_spray && !has_splashes {
                    continue;
                }
                pass.set_bind_group(1, &surface.bind_group, &[]);
                if surface.settings.sea_spray {
                    pass.draw(0..6, 0..particles);
                }
                if has_splashes {
                    pass.draw(0..6, splash_start..splash_end);
                }
            }
        }
    }

    pub fn encode_frame(&mut self, queue:&wgpu::Queue, encoder:&mut wgpu::CommandEncoder, delta:f32) {
        if !self.enabled { return; }
        let delta=delta.max(0.0);
        let now=self.elapsed;
        let updates=self.settings.updates_per_second;
        let target=1.0/(updates+1e-10);

        // Begin only when the previous update has finished. In particular,
        // NEVER flush overdue cascades into one render frame: that was the
        // large periodic GPU spike in the original scheduling path.
        if self.phase_count == 0 && (updates == 0.0 || now >= self.next_update) {
            let update_delta=if updates==0.0 { delta } else { target+(now-self.next_update) };
            self.next_update=now+target;
            self.stage_params(queue,encoder,update_delta);
            self.phase=0;
            self.phase_spectrum=self.spectrum_dirty;
            self.phase_count=OCEAN_FFT_PASSES+usize::from(self.phase_spectrum);
            self.phase_started_at=now;
            self.phase_period=if updates==0.0 { 0.0 } else { target };
            self.spectrum_dirty=false;
        }

        if self.phase_count > 0 {
            // Spread the existing 15 passes across the update interval when
            // rendering fast. At lower FPS allow bounded catch-up rather than
            // reducing the ocean to one simulation update per 15 frames.
            let due=ocean_phases_due(now,self.phase_started_at,self.phase_period,self.phase_count);
            // A zero-UPS setting means "run every render frame", matching the
            // original unthrottled mode rather than reducing it to 1/3 speed.
            let budget=due.saturating_sub(self.phase).min(if updates == 0.0 {
                self.phase_count
            } else {
                OCEAN_MAX_PASSES_PER_FRAME
            });
            for _ in 0..budget {
                if self.phase_spectrum && self.phase==0 {
                    let n=self.settings.map_size;
                    let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("Ocean spectrum regenerate"),timestamp_writes:None});
                    pass.set_pipeline(&self.spectrum_pipeline);
                    pass.set_bind_group(0,&self.compute_bind_groups[0],&[]);
                    pass.dispatch_workgroups(n/16,n/16,OCEAN_CASCADES as u32);
                } else {
                    let fft_phase=self.phase-usize::from(self.phase_spectrum);
                    let cascade=OCEAN_CASCADES-1-fft_phase/OCEAN_PASSES_PER_CASCADE;
                    self.encode_fft_phase(encoder,cascade,fft_phase%OCEAN_PASSES_PER_CASCADE);
                }
                self.phase+=1;
            }
            if self.phase==self.phase_count {
                self.phase=0;
                self.phase_count=0;
            }
        }

        self.elapsed=now+delta;
        if self.settings.wind_foam_streaks || self.promoted_surfaces.iter().any(|s| s.settings.wind_foam_streaks) {
            self.encode_windrow(queue, encoder, delta);
        }
        // Each cascade's output is still published only by its complete
        // unpack dispatch. The mesh and ocean texture layout are unchanged.
        self.write_render_uniform(queue);
    }

    fn encode_windrow(&mut self, queue:&wgpu::Queue, encoder:&mut wgpu::CommandEncoder, delta:f32) {
        let wind = self.settings.wind.at(self.elapsed);
        let render_wind = [wind[0] / OCEAN_UNITS_PER_METER, -wind[1] / OCEAN_UNITS_PER_METER];
        let mut map_scales = [[0.0;4]; OCEAN_CASCADES];
        for (i, scale) in map_scales.iter_mut().enumerate() {
            let cascade = self.settings.cascades[i];
            let spatial = if i == 2 { 1.0 } else { self.settings.authored.wavelength / 1400.0 };
            *scale = [
                1.0/(cascade.tile_length[0]*spatial),
                1.0/(cascade.tile_length[1]*spatial),
                0.0, 0.0
            ];
        }
        let params = WindrowGpu {
            map_scales,
            wind_time: [render_wind[0], render_wind[1], self.elapsed, delta.min(0.1)],
            params: [OCEAN_WINDROW_DOMAIN_METERS, 1.0, 12.0, self.windrow_front as f32],
        };
        queue.write_buffer(&self.windrow_params_buffer, 0, bytemuck::bytes_of(&params));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Ocean wind foam streak transport"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.windrow_pipeline);
            pass.set_bind_group(0, &self.windrow_bind_group, &[]);
            pass.dispatch_workgroups(OCEAN_WINDROW_SIZE.div_ceil(8), OCEAN_WINDROW_SIZE.div_ceil(8), 1);
        }
        self.windrow_front = 1 - self.windrow_front;
    }

    fn encode_fft_phase(&self, encoder:&mut wgpu::CommandEncoder, cascade:usize, phase:usize) {
        let n=self.settings.map_size;
        let bind=&self.compute_bind_groups[cascade];
        // FFT stages have storage read-after-write dependencies. The passes
        // remain separate (including across command submissions) and are
        // always encoded in their original upstream order. Since all FFT
        // cascades share one scratch buffer, never interleave cascades.
        let (label, pipeline, dispatch) = match phase {
            0 => ("GodotOceanWaves spectrum modulate", &self.modulate_pipeline, (n/16,n/16,1)),
            1 => ("GodotOceanWaves FFT rows", &self.fft_pipeline, (1,n,4)),
            2 => ("GodotOceanWaves FFT transpose", &self.transpose_pipeline, (n/32,n/32,4)),
            3 => ("GodotOceanWaves FFT columns", &self.fft_pipeline, (1,n,4)),
            4 => ("GodotOceanWaves FFT unpack", &self.unpack_pipeline, (n/16,n/16,1)),
            _ => unreachable!("five FFT phases per cascade"),
        };
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some(label),timestamp_writes:None});
        pass.set_bind_group(0,bind,&[]);
        pass.set_pipeline(pipeline);
        pass.dispatch_workgroups(dispatch.0,dispatch.1,dispatch.2);
    }

}

/// Number of phases that should have been issued by `now`. The first phase
/// starts immediately; remaining phases are paced across one simulation tick.
/// Slower render FPS can require multiple passes in one frame to keep up.
fn ocean_phases_due(now:f32, start:f32, period:f32, count:usize) -> usize {
    if period <= 0.0 { return count; } // 0 UPS means unthrottled simulation
    let fraction=((now-start).max(0.0)/period).min(1.0);
    (1+(fraction*count as f32).floor() as usize).min(count)
}

fn srgb_channel_to_linear(c:f32)->f32 { if c<=0.04045 { c/12.92 } else { ((c+0.055)/1.055).powf(2.4) } }
fn linear_color(rgb:[f32;3])->[f32;4] { [srgb_channel_to_linear(rgb[0]),srgb_channel_to_linear(rgb[1]),srgb_channel_to_linear(rgb[2]),1.0] }
fn make_render_uniform(enabled:bool,settings:&OceanSettings)->OceanRenderUniform {
    let mut scales=[[0.0;4];OCEAN_CASCADES];
    for (i,scale) in scales.iter_mut().enumerate() {
        let cascade=settings.cascades[i];
        let spatial = if i == 2 { 1.0 } else { settings.authored.wavelength / 1400.0 };
        let strength = if i == 2 { settings.authored.wind_chop * (settings.wind.speed / 800.0).clamp(0.0, 2.0) } else { settings.authored.amplitude / 128.0 * settings.authored.slosh };
        let displacement = cascade.displacement_scale * strength;
        *scale=[1.0/(cascade.tile_length[0]*spatial),1.0/(cascade.tile_length[1]*spatial),displacement,cascade.normal_scale*strength/spatial];
    }
    OceanRenderUniform {
        map_scales:scales,
        water_color:linear_color(settings.water_color),
        foam_color:linear_color(settings.foam_color),
        ocean_info:[if enabled{1.0}else{0.0},settings.map_size as f32,OCEAN_UNITS_PER_METER,OCEAN_WINDROW_DOMAIN_METERS],
        surface:[settings.roughness,settings.normal_strength,if settings.sea_spray { 1.0 } else { 0.0 },if settings.wind_foam_streaks { 1.0 } else { 0.0 }],
        clipmap:[0.0,0.0,0.0,0.0],
        sun_direction_intensity:[-0.3,-0.8,-0.2,1.0],
        sun_color:[1.0,1.0,1.0,1.0],
        sky_ambient:[0.28,0.38,0.52,0.0],
        spray_info:[0.0,32768.0,3.0,0.25],
        spray_bounds:[[0.0;4];OCEAN_MAX_SURFACES],
        spray_planes:[[0.0;4];OCEAN_MAX_SURFACES],
        swell: {
            let angle = settings.authored.direction.to_radians();
            [angle.cos(), -angle.sin(), std::f32::consts::TAU * OCEAN_UNITS_PER_METER / settings.authored.wavelength,
                settings.authored.amplitude / OCEAN_UNITS_PER_METER * (1.0-settings.authored.slosh)]
        },
        swell_motion: [std::f32::consts::TAU * settings.authored.speed / settings.authored.wavelength,
            0.9 * settings.authored.steepness / (settings.authored.amplitude * std::f32::consts::TAU / settings.authored.wavelength).max(0.00001),
            settings.authored.steepness / 0.65, 0.0],
        wind_motion: [0.0,0.0,settings.authored.spray,0.0],
        authored_bounds: settings.bounds.map(|b| [b.minimum[0],b.minimum[1],b.maximum[0],b.maximum[1]]).unwrap_or([0.0;4]),
        authored_plane: settings.bounds.map(|b| [1.0,b.plane_height,0.0,0.0]).unwrap_or([0.0;4]),
        mask_ranges: [[0.0; 4]; OCEAN_MAX_SURFACES],
        mask_bounds: [[0.0; 4]; OCEAN_MAX_SURFACES],
        mask_tiles: [[0; 4]; OCEAN_MAX_SURFACES],
        mask_triangles: [[0.0; 4]; OCEAN_MASK_VEC4S],
    }
}

/// Use the same render view for the water and its spray emitter. Finest wind
/// chop must follow local amplitude too, rather than retaining full ocean size.
fn make_promoted_render_uniform(
    shared: &OceanRenderUniform,
    settings: &OceanSettings,
    descriptor: &appearance::PromotedOcean,
    manual: bool,
    surface_count: usize,
) -> OceanRenderUniform {
    let mut local = make_render_uniform(shared.ocean_info[0] > 0.5, settings);
    let amplitude_scale = settings.authored.amplitude / 128.0;
    local.map_scales[2][2] *= amplitude_scale;
    local.map_scales[2][3] *= amplitude_scale;
    local.clipmap = shared.clipmap;
    local.swell_motion[3] = shared.swell_motion[3];
    local.sun_direction_intensity = shared.sun_direction_intensity;
    local.sun_color = shared.sun_color;
    local.sky_ambient = shared.sky_ambient;
    let base_spacing = if settings.mesh_quality > 0 {
        OCEAN_HIGH_MESH_SPACING_UNITS
    } else {
        OCEAN_LOW_MESH_SPACING_UNITS
    };
    let extent = (descriptor.surface.maximum[0] - descriptor.surface.minimum[0])
        .max(descriptor.surface.maximum[1] - descriptor.surface.minimum[1]);
    let half_cells = promoted_clipmap_half_cells(settings.authored.amplitude);
    let outer_spacing = promoted_clipmap_outer_spacing(
        base_spacing, extent, settings.authored.amplitude,
    );
    local.wind_motion = [
        shared.wind_motion[0],
        shared.wind_motion[1],
        settings.authored.spray,
        outer_spacing,
    ];
    // X/Y are unused for promoted views except Y, which carries the live
    // clipmap half-width. Z marks a manual appearance override, W a promoted
    // view. This keeps seam snapping in lockstep with runtime index rewrites.
    local.authored_plane = [0.0, half_cells as f32, if manual { 1.0 } else { 0.0 }, 1.0];
    local.spray_info[0] = 1.0;
    local.spray_info[1] = (32768 / surface_count.max(1)) as f32;
    let s = descriptor.surface;
    local.spray_bounds[0] = [s.minimum[0], s.minimum[1], s.maximum[0], s.maximum[1]];
    local.spray_planes[0] = [
        s.plane_height,
        amplitude_scale.clamp(0.0, 1.0),
        descriptor.index as f32,
        settings.authored.foam,
    ];
    local.mask_ranges = shared.mask_ranges;
    local.mask_bounds = shared.mask_bounds;
    local.mask_tiles = shared.mask_tiles;
    local.mask_triangles = shared.mask_triangles;
    local
}

fn apply_scene_context(
    uniform: &mut OceanRenderUniform,
    environment: OceanEnvironment,
    surfaces: &[OceanSurface; OCEAN_MAX_SURFACES],
    surface_count: usize,
) {
    uniform.sun_direction_intensity = environment.sun_direction_intensity;
    uniform.sun_color = environment.sun_color;
    uniform.sky_ambient = environment.sky_ambient;
    uniform.spray_info[0] = surface_count.min(OCEAN_MAX_SURFACES) as f32;
    for (index, surface) in surfaces.iter().take(surface_count.min(OCEAN_MAX_SURFACES)).enumerate() {
        uniform.spray_bounds[index] = [surface.minimum[0], surface.minimum[1], surface.maximum[0], surface.maximum[1]];
        uniform.spray_planes[index] = [surface.plane_height, 0.0, 0.0, 0.0];
    }
}
fn storage_entry(binding:u32,read_only:bool)->wgpu::BindGroupLayoutEntry { wgpu::BindGroupLayoutEntry { binding,visibility:wgpu::ShaderStages::COMPUTE,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only},has_dynamic_offset:false,min_binding_size:None},count:None } }
fn uniform_entry(binding:u32)->wgpu::BindGroupLayoutEntry { uniform_layout_entry(binding,wgpu::ShaderStages::COMPUTE) }
fn uniform_layout_entry(binding:u32,visibility:wgpu::ShaderStages)->wgpu::BindGroupLayoutEntry { wgpu::BindGroupLayoutEntry { binding,visibility,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None } }
fn storage_texture_entry(binding:u32)->wgpu::BindGroupLayoutEntry { wgpu::BindGroupLayoutEntry { binding,visibility:wgpu::ShaderStages::COMPUTE,ty:wgpu::BindingType::StorageTexture{access:wgpu::StorageTextureAccess::WriteOnly,format:wgpu::TextureFormat::Rgba16Float,view_dimension:wgpu::TextureViewDimension::D2Array},count:None } }
fn bg_buffer<'a>(binding:u32,buffer:&'a wgpu::Buffer)->wgpu::BindGroupEntry<'a> { wgpu::BindGroupEntry{binding,resource:buffer.as_entire_binding()} }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promoted_water_amplitude_controls_all_displacement_and_spray_views() {
        let shared = OceanSettings::default();
        let mut context = make_render_uniform(true,&shared);
        context.swell_motion[3] = 37.0;
        context.mask_ranges[3] = [2.0,1.0,0.0,0.0];
        let pool = appearance::tests::pool(3,0.0);
        let mut settings = pool.settings(shared);
        let calm = make_promoted_render_uniform(&context,&settings,&pool,false,2);
        assert!(calm.swell[3] > 0.0 && calm.map_scales[0][2] > 0.0);
        settings.authored.amplitude *= 2.0;
        let larger = make_promoted_render_uniform(&context,&settings,&pool,true,2);
        assert_eq!(larger.swell[3],calm.swell[3]*2.0);
        for i in 0..3 {
            assert_eq!(larger.map_scales[i][2],calm.map_scales[i][2]*2.0,"cascade {i} height");
            assert_eq!(larger.map_scales[i][3],calm.map_scales[i][3]*2.0,"cascade {i} normals");
        }
        assert_eq!(larger.spray_planes[0][1],calm.spray_planes[0][1]*2.0);
        assert_eq!(larger.spray_planes[0][2],3.0);
        assert_eq!(larger.spray_bounds[0],[0.0,0.0,1024.0,1024.0]);
        assert_eq!(larger.mask_ranges[3],context.mask_ranges[3]);
        assert_eq!(larger.spray_info[1],16384.0);
        assert_eq!(larger.swell_motion[3],37.0);
    }

    #[test]
    fn authored_swell_size_is_independent_of_weather_wind() {
        let mut settings = OceanSettings::default();
        settings.wind.speed = 0.0;
        let calm = make_render_uniform(true, &settings);
        settings.wind.speed = 800.0;
        let windy = make_render_uniform(true, &settings);
        assert_eq!(calm.map_scales[..2], windy.map_scales[..2]);
        assert_eq!(calm.swell, windy.swell);
        assert_eq!(calm.map_scales[2][2], 0.0);
        assert!(windy.map_scales[2][2] > 0.0);
        settings.authored.amplitude *= 2.0;
        settings.authored.wavelength *= 2.0;
        let large = make_render_uniform(true, &settings);
        assert_eq!(large.map_scales[0][2], windy.map_scales[0][2] * 2.0);
        assert_eq!(large.map_scales[0][0], windy.map_scales[0][0] * 0.5);
        assert_eq!(large.map_scales[2], windy.map_scales[2]);
        settings.authored.slosh = 0.0;
        let primary = make_render_uniform(true, &settings);
        assert_eq!(primary.map_scales[0][2], 0.0);
        assert!(primary.swell[3] > 0.0);
    }

    /// Water ringing a dry island: four slabs around a 40x40 hole in a 100x100 box.
    fn donut_masks() -> OceanMasks {
        let quad = |x0: f32, y0: f32, x1: f32, y1: f32| {
            [[[x0, y0], [x1, y0], [x1, y1]], [[x0, y0], [x1, y1], [x0, y1]]]
        };
        let triangles: Vec<MaskTriangle> = [
            quad(0.0, 0.0, 100.0, 30.0),
            quad(0.0, 70.0, 100.0, 100.0),
            quad(0.0, 30.0, 30.0, 70.0),
            quad(70.0, 30.0, 100.0, 70.0),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut masks = OceanMasks::default();
        assert!(masks.push_surface(2, &triangles));
        masks
    }

    #[test]
    fn water_volume_excludes_the_hollow_of_a_masked_surface() {
        let masks = donut_masks();
        let ring = WaterVolume {
            minimum: [0.0, -65536.0, 0.0],
            maximum: [100.0, 50.0, 100.0],
            mask_slot: Some(2),
        };
        assert_eq!(ring.mask_lane(), 3.0);
        // In the ring, below the surface: submerged.
        assert!(ring.contains([10.0, 0.0, 50.0], &masks));
        assert!(ring.contains([50.0, 0.0, 10.0], &masks));
        // Dead centre of the hollow is inside the box but not in the water.
        assert!(!ring.contains([50.0, 0.0, 50.0], &masks));
        // Above the surface and outside the box are dry whatever the mask says.
        assert!(!ring.contains([10.0, 60.0, 50.0], &masks));
        assert!(!ring.contains([150.0, 0.0, 50.0], &masks));
        // Without an outline slot the box is the water, and a zeroed lane is unmasked.
        let plain = WaterVolume { mask_slot: None, ..ring };
        assert_eq!(plain.mask_lane(), 0.0);
        assert!(plain.contains([50.0, 0.0, 50.0], &masks));
        // A slot that was never pushed contains everything.
        assert!(masks.contains(5, [50.0, 50.0]));
        assert!(!masks.is_masked(5));
    }

    #[test]
    fn outline_tiles_are_exact_and_accelerate_interior_and_exterior() {
        let masks = donut_masks();
        let exact = |p: [f32; 2]| {
            let first = masks.ranges[2][0] as usize;
            let count = masks.ranges[2][1] as usize;
            (first..first + count).any(|i| {
                let a = masks.triangles[i * 2];
                let b = masks.triangles[i * 2 + 1];
                mask_triangle_contains(
                    [[a[0], a[1]], [a[2], a[3]], [b[0], b[1]]],
                    p,
                )
            })
        };
        let mut outside = 0;
        let mut inside = 0;
        for y in 0..OCEAN_MASK_TILE_AXIS {
            for x in 0..OCEAN_MASK_TILE_AXIS {
                let p = [(x as f32 + 0.5) * 12.5, (y as f32 + 0.5) * 12.5];
                match mask_tile_code(masks.bounds[2], masks.tiles[2], p) {
                    MASK_TILE_OUTSIDE => outside += 1,
                    MASK_TILE_INSIDE => inside += 1,
                    _ => {}
                }
                assert_eq!(masks.contains(2, p), exact(p), "tile sample {x},{y}");
            }
        }
        for y in 0..=200 {
            for x in 0..=200 {
                let p = [x as f32 * 0.5, y as f32 * 0.5];
                assert_eq!(masks.contains(2, p), exact(p), "dense sample {x},{y}");
            }
        }
        assert!(outside > 0, "the island hollow should have fast dry tiles");
        assert!(inside > 0, "the water slabs should have fast wet tiles");
        assert!(!masks.rect_may_contain(2, [45.0, 45.0], [55.0, 55.0]));
        assert!(masks.rect_may_contain(2, [5.0, 45.0], [15.0, 55.0]));
    }

    #[test]
    fn promoted_clipmap_prunes_dry_cells_only_when_its_snapped_centre_moves() {
        let masks = donut_masks();
        let surface = OceanSurface {
            plane_height: 0.0,
            minimum: [0.0, 0.0],
            maximum: [100.0, 100.0],
        };
        let runtime = build_runtime_promoted_clipmap(1.0, 0.0, surface.minimum,
            surface.maximum, Some(2), appearance::MIN_PROMOTED_AMPLITUDE);
        let half = promoted_clipmap_half_cells(appearance::MIN_PROMOTED_AMPLITUDE);
        let full = runtime.layout.indices_for(half, 0);
        let centre = promoted_clipmap_centre(
            [50.0, 0.0, 50.0],
            surface,
            32.0,
            runtime.layout.outer_spacing_for_half_cells(half),
        );
        let pruned = runtime.layout.indices_for_footprint(
            half, 0, centre, surface, &masks, Some(2), None,
        );
        assert!(pruned.len() < full.len(), "the dry island must remove draw cells");
        assert!(!pruned.is_empty(), "the surrounding water ring remains drawable");

        let same_snap = promoted_clipmap_centre(
            [51.0, 0.0, 51.0],
            surface,
            32.0,
            runtime.layout.outer_spacing_for_half_cells(half),
        );
        assert_eq!(centre, same_snap, "sub-lattice camera motion needs no rewrite");
    }

    #[test]
    fn promoted_clipmap_centres_on_real_l_shaped_water() {
        let rectangle = |lo: [f32; 2], hi: [f32; 2]| {
            vec![
                [lo, [hi[0], lo[1]], [lo[0], hi[1]]],
                [[hi[0], lo[1]], hi, [lo[0], hi[1]]],
            ]
        };
        let faces = [
            WaterFace { plane: 0.0, minimum: [0.0, 0.0], maximum: [100.0, 20.0], authored_ocean: None, material: 1 },
            WaterFace { plane: 0.0, minimum: [0.0, 20.0], maximum: [20.0, 100.0], authored_ocean: None, material: 1 },
        ];
        let triangles = [rectangle([0.0, 0.0], [100.0, 20.0]), rectangle([0.0, 20.0], [20.0, 100.0])];
        let footprint = build_water_footprints(&faces, &triangles, 32.0).remove(0);
        let mut masks = OceanMasks::default();
        assert!(masks.push_footprint(0, &footprint));
        let surface = OceanSurface { plane_height: 0.0, minimum: footprint.minimum, maximum: footprint.maximum };
        let camera = [80.0, 0.0, 80.0];
        assert_eq!(promoted_surface_distance(camera, surface), 0.0);
        assert!(promoted_footprint_distance(camera, surface, &masks, 0) > 59.0);
        let centre = promoted_footprint_clipmap_centre(camera, surface, 1.0, 1.0, &masks, 0);
        assert!(centre[0] <= 21.0 || centre[1] <= 21.0, "centre {centre:?} must lie on an L arm");
    }

    #[test]
    fn promoted_clipmap_prunes_to_the_current_full_pvs_pieces() {
        let surface = OceanSurface {
            plane_height: 0.0,
            minimum: [0.0, 0.0],
            maximum: [100.0, 100.0],
        };
        let rectangle = |lo: [f32; 2], hi: [f32; 2]| {
            vec![
                [lo, [hi[0], lo[1]], [lo[0], hi[1]]],
                [[hi[0], lo[1]], hi, [lo[0], hi[1]]],
            ]
        };
        let mut pvs = OceanPvsFootprints::with_batch_count(2);
        pvs.add_piece(0, 0, surface, rectangle([0.0, 0.0], [40.0, 100.0]));
        pvs.add_piece(1, 0, surface, rectangle([60.0, 0.0], [100.0, 100.0]));

        let left_only = pvs.visible_tiles(&[0]);
        let left = left_only[0].as_ref().expect("surface has exact PVS pieces");
        assert!(rect_intersects_pvs_tiles(surface, left, [5.0, 45.0], [15.0, 55.0]));
        assert!(!rect_intersects_pvs_tiles(surface, left, [85.0, 45.0], [95.0, 55.0]));
        let both = pvs.visible_tiles(&[0, 1]);
        assert!(rect_intersects_pvs_tiles(
            surface,
            both[0].as_ref().unwrap(),
            [85.0, 45.0],
            [95.0, 55.0],
        ));
        let neither = pvs.visible_tiles(&[]);
        assert!(!rect_intersects_pvs_tiles(
            surface,
            neither[0].as_ref().unwrap(),
            [5.0, 45.0],
            [15.0, 55.0],
        ));

        let runtime = build_runtime_promoted_clipmap(
            1.0,
            0.0,
            surface.minimum,
            surface.maximum,
            None,
            appearance::MIN_PROMOTED_AMPLITUDE,
        );
        let half = promoted_clipmap_half_cells(appearance::MIN_PROMOTED_AMPLITUDE);
        let centre = promoted_clipmap_centre(
            [50.0, 0.0, 50.0],
            surface,
            32.0,
            runtime.layout.outer_spacing_for_half_cells(half),
        );
        let full = runtime.layout.indices_for_footprint(
            half,
            0,
            centre,
            surface,
            &OceanMasks::default(),
            None,
            None,
        );
        let visible = runtime.layout.indices_for_footprint(
            half,
            0,
            centre,
            surface,
            &OceanMasks::default(),
            None,
            Some(left),
        );
        assert!(!visible.is_empty());
        assert!(visible.len() < full.len());
    }

    #[test]
    fn clipmap_levels_nest_and_stay_in_bounds() {
        // A footprint wider than the outermost ring's reach needs the full stack.
        let clipmap = build_clipmap(OCEAN_HIGH_MESH_SPACING_UNITS, 100.0, [-100_000.0; 2], [100_000.0; 2], None);
        let side = (CLIPMAP_HALF_CELLS * 2 + 1) as usize;
        assert_eq!(clipmap.vertices.len(), side * side * CLIPMAP_LEVELS as usize);
        for index in &clipmap.indices {
            assert!((*index as usize) < clipmap.vertices.len());
        }
        assert_eq!(clipmap.indices.len() % 3, 0);

        // Level 0 is solid; every later level is an annulus with a quarter of
        // its cells removed, and that hole must land exactly on the previous
        // level's outer extent or the seam cannot close.
        let cells = (CLIPMAP_HALF_CELLS * 2) as usize;
        let hole = CLIPMAP_HALF_CELLS as usize;
        let expected = cells * cells + (CLIPMAP_LEVELS as usize - 1) * (cells * cells - hole * hole);
        assert_eq!(clipmap.indices.len(), expected * 6);

        for level in 1..CLIPMAP_LEVELS {
            let spacing = OCEAN_HIGH_MESH_SPACING_UNITS * (1u32 << level) as f32;
            let hole_extent = (CLIPMAP_HALF_CELLS / 2) as f32 * spacing;
            let inner_extent =
                CLIPMAP_HALF_CELLS as f32 * OCEAN_HIGH_MESH_SPACING_UNITS * (1u32 << (level - 1)) as f32;
            assert_eq!(hole_extent, inner_extent, "level {level} hole must match level {} extent", level - 1);
        }

        // Outer-boundary vertices carry the parent cell size so the shader can
        // snap them; interior vertices must not.
        let mut snapped = 0usize;
        for (index, vertex) in clipmap.vertices.iter().enumerate() {
            let within = index % (side * side);
            let column = (within % side) as i32 - CLIPMAP_HALF_CELLS;
            let row = (within / side) as i32 - CLIPMAP_HALF_CELLS;
            let outer = column.abs() == CLIPMAP_HALF_CELLS || row.abs() == CLIPMAP_HALF_CELLS;
            let level = index / (side * side);
            let expects_snap = outer && level + 1 < CLIPMAP_LEVELS as usize;
            assert_eq!(vertex.color[1] > 0.0, expects_snap);
            if expects_snap {
                assert_eq!(vertex.color[1], vertex.color[0] * 2.0);
                snapped += 1;
            }
        }
        assert!(snapped > 0);
    }

    fn reach(clipmap: &OceanClipmap) -> f32 {
        clipmap.vertices.iter()
            .map(|v| v.position[0].abs().max(v.position[2].abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn calm_promoted_water_shrinks_lod_rings_without_losing_coverage() {
        assert_eq!(promoted_clipmap_half_cells(appearance::MIN_PROMOTED_AMPLITUDE), 32);
        assert_eq!(promoted_clipmap_half_cells(PROMOTED_CLIPMAP_FULL_DETAIL_AMPLITUDE), 128);
        assert!(promoted_clipmap_half_cells(16.0) > 32);
        assert!(promoted_clipmap_half_cells(16.0) < 128);

        let minimum = [0.0; 2];
        let maximum = [40_000.0; 2];
        let reference = build_clipmap(10.0, 0.0, minimum, maximum, None);
        let calm = build_promoted_clipmap(
            10.0,
            0.0,
            minimum,
            maximum,
            None,
            appearance::MIN_PROMOTED_AMPLITUDE,
        );
        // 32-cell rings need two extra coarse levels to retain the old map
        // reach, but still submit dramatically less geometry than six 128-cell
        // levels. This is the actual FPS win the calm-water profile is for.
        assert!(calm.indices.len() * 8 < reference.indices.len());
        let step = calm.vertices[0].color[2];
        assert!(reach(&calm) >= 40_000.0 + step * 0.5);

        // By 64 units of displacement promotion is back on the original mesh
        // profile, so violent water does not lose geometric fidelity.
        let rough = build_promoted_clipmap(
            10.0,
            0.0,
            minimum,
            maximum,
            None,
            PROMOTED_CLIPMAP_FULL_DETAIL_AMPLITUDE,
        );
        assert_eq!(rough.vertices.len(), reference.vertices.len());
        assert_eq!(rough.indices.len(), reference.indices.len());
    }

    #[test]
    fn runtime_promoted_lod_uses_true_3d_distance_and_live_amplitude() {
        let minimum = [-20_000.0; 2];
        let maximum = [20_000.0; 2];
        let surface = OceanSurface { plane_height: 0.0, minimum, maximum };
        let runtime = build_runtime_promoted_clipmap(
            OCEAN_HIGH_MESH_SPACING_UNITS,
            0.0,
            minimum,
            maximum,
            None,
            appearance::MIN_PROMOTED_AMPLITUDE,
        );
        let calm_half = promoted_clipmap_half_cells(appearance::MIN_PROMOTED_AMPLITUDE);
        assert_eq!(calm_half, 32);
        assert_eq!(promoted_surface_distance([0.0, 0.0, 0.0], surface), 0.0);

        // Renderer Y is JKA Z: a camera one thousand units straight above the
        // water has crossed the first two calm-water LOD radii even though its
        // horizontal coordinates are unchanged.
        let sky_distance = promoted_surface_distance([0.0, 1000.0, 0.0], surface);
        assert_eq!(sky_distance, 1000.0);
        let sky_level = runtime.layout.minimum_level_for_distance(calm_half, sky_distance);
        assert_eq!(sky_level, 2);
        assert!(runtime.layout.indices_for(calm_half, sky_level).len() < runtime.indices.len());

        // The same camera distance stays in a finer level when amplitude grows,
        // proving the runtime slider changes the physical LOD thresholds.
        let rough_half = promoted_clipmap_half_cells(16.0);
        assert!(rough_half > calm_half);
        let test_distance = 500.0;
        assert!(runtime.layout.minimum_level_for_distance(calm_half, test_distance)
            > runtime.layout.minimum_level_for_distance(rough_half, test_distance));
        assert!(runtime.layout.indices_for(rough_half, 0).len() > runtime.indices.len());
        assert!(runtime.index_capacity >= runtime.layout.indices_for(rough_half, 0).len());

        // Horizontal separation uses the same shortest 3D distance metric.
        assert_eq!(promoted_surface_distance([30_000.0, 0.0, 0.0], surface), 10_000.0);
    }

    #[test]
    fn clipmap_cell_size_does_not_depend_on_footprint() {
        let finest = |c: &OceanClipmap| c.vertices.iter().map(|v| v.color[0]).fold(f32::INFINITY, f32::min);
        let pool = build_clipmap(10.0, 0.0, [0.0, 0.0], [320.0, 384.0], None);
        let sea = build_clipmap(10.0, 0.0, [-1.0e5; 2], [1.0e5; 2], None);
        assert_eq!(finest(&pool), 10.0);
        assert_eq!(finest(&sea), 10.0);
        // The pool only drops rings it could never show.
        assert!(pool.indices.len() * 50 < sea.indices.len());
        assert_eq!(pool.coarsest_spacing, sea.coarsest_spacing);
    }

    #[test]
    fn clipmap_reaches_the_whole_footprint_from_any_centre() {
        for extent in [100.0f32, 320.0, 1408.0, 1500.0, 5000.0, 40_000.0] {
            let clipmap = build_clipmap(10.0, 0.0, [0.0; 2], [extent; 2], None);
            // The shader snaps the centre to color.b, so it can land half a
            // step outside the footprint.
            let step = clipmap.vertices[0].color[2];
            assert!(
                reach(&clipmap) >= extent + step * 0.5,
                "extent {extent}: reach {} step {step}", reach(&clipmap)
            );
            for index in &clipmap.indices {
                assert!((*index as usize) < clipmap.vertices.len());
            }
        }
    }

    #[test]
    fn contiguous_water_merges_and_biggest_comes_first() {
        let face = |plane: f32, x: [f32; 2], z: [f32; 2]| WaterFace {
            plane, minimum: [x[0], z[0]], maximum: [x[1], z[1]], authored_ocean: None, material: 0,
        };
        let faces = [
            face(0.0, [0.0, 100.0], [0.0, 100.0]),       // small pool
            face(500.0, [0.0, 4000.0], [0.0, 4000.0]),   // sea, piece 1
            face(500.0, [4000.0, 9000.0], [0.0, 4000.0]),// sea, piece 2 (shares an edge)
            face(500.0, [0.0, 100.0], [9000.0, 9100.0]), // same plane, apart
            face(0.0, [90.0, 300.0], [0.0, 100.0]),      // overlaps the small pool
            face(0.4, [300.0, 400.0], [0.0, 100.0]),     // same plane within tolerance, touching
        ];
        let clusters = cluster_water_faces(&faces);
        assert_eq!(clusters.len(), 3);
        assert_eq!(clusters[0].members, vec![1, 2]);
        assert_eq!(clusters[0].minimum, [0.0, 0.0]);
        assert_eq!(clusters[0].maximum, [9000.0, 4000.0]);
        assert_eq!(clusters[1].members, vec![0, 4, 5]);
        assert_eq!(clusters[2].members, vec![3]);
        assert!(clusters.windows(2).all(|w| w[0].area() >= w[1].area()));

        // Different authored oceans, or different planes, never merge.
        let mut other = faces[2];
        other.material = 1;
        assert_eq!(cluster_water_faces(&[faces[1], other]).len(), 2);
        other.material = 0;
        other.authored_ocean = Some(0);
        assert_eq!(cluster_water_faces(&[faces[1], other]).len(), 2);
        assert_eq!(cluster_water_faces(&[faces[1], face(900.0, [0.0, 4000.0], [0.0, 4000.0])]).len(), 2);
    }

    #[test]
    fn rectangular_promoted_water_needs_no_fragment_mask() {
        let triangles: [MaskTriangle; 2] = [
            [[0.0,0.0],[100.0,0.0],[0.0,50.0]],
            [[100.0,0.0],[100.0,50.0],[0.0,50.0]],
        ];
        assert!(!promoted_outline_needs_mask([0.0,0.0],[100.0,50.0],&triangles));
    }

    #[test]
    fn irregular_promoted_water_keeps_fragment_mask() {
        // One quarter of the 100x100 footprint is deliberately missing.
        let triangles: [MaskTriangle; 4] = [
            [[0.0,0.0],[100.0,0.0],[0.0,50.0]],
            [[100.0,0.0],[100.0,50.0],[0.0,50.0]],
            [[0.0,50.0],[50.0,50.0],[0.0,100.0]],
            [[50.0,50.0],[50.0,100.0],[0.0,100.0]],
        ];
        assert!(promoted_outline_needs_mask([0.0,0.0],[100.0,100.0],&triangles));
    }

    #[test]
    fn mask_store_indexes_by_slot_and_refuses_overflow() {
        let tri: MaskTriangle = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let mut masks = OceanMasks::default();
        assert!(masks.push_surface(3, &[tri, tri]));
        assert!(masks.push_surface(0, &[tri]));
        assert_eq!(masks.ranges[3][..2], [0.0, 2.0]);
        assert_eq!(masks.ranges[0][..2], [2.0, 1.0]);
        assert_eq!(masks.triangles.len(), 6);
        assert!(!masks.push_surface(1, &[]), "empty outline means unmasked");
        assert!(!masks.push_surface(OCEAN_MAX_SURFACES, &[tri]));
        let too_many = vec![tri; OCEAN_MASK_VEC4S / 2];
        assert!(!masks.push_surface(2, &too_many), "overflow leaves the slot unmasked");
        assert_eq!(masks.ranges[2], [0.0; 4]);
        assert_eq!(masks.bounds[2], [0.0; 4]);
        assert_eq!(masks.tiles[2], [0; 4]);
        assert_eq!(masks.prune_tiles[2], [0; OCEAN_PRUNE_TILE_WORDS]);
    }

    #[test]
    fn clipmap_triangles_face_up() {
        let clipmap = build_clipmap(10.0, 0.0, [-1e9, -1e9], [1e9, 1e9], None);
        // The renderer culls back faces with a counter-clockwise front face, so
        // the clipmap must wind the same way as an authored +Y water face.
        for triangle in clipmap.indices.chunks_exact(3).take(4096) {
            let p: Vec<[f32; 3]> = triangle.iter().map(|i| clipmap.vertices[*i as usize].position).collect();
            let u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let normal_y = u[2] * v[0] - u[0] * v[2];
            assert!(normal_y > 0.0, "clipmap triangle winds downward");
        }
    }
}

#[cfg(test)]
mod ocean_phase_scheduler_tests {
    use super::*;

    #[test]
    fn spreads_fifteen_passes_over_one_tick() {
        assert_eq!(ocean_phases_due(0.0, 0.0, 1.0/24.0, 15), 1);
        assert_eq!(ocean_phases_due(1.0/48.0, 0.0, 1.0/24.0, 15), 8);
        assert_eq!(ocean_phases_due(1.0/24.0, 0.0, 1.0/24.0, 15), 15);
        assert_eq!(ocean_phases_due(0.005, 0.0, 1.0/24.0, 15), 2);
    }

    #[test]
    fn handles_zero_rate_and_clamps_overdue_phases() {
        assert_eq!(ocean_phases_due(0.0, 0.0, 0.0, 15), 15);
        assert_eq!(ocean_phases_due(10.0, 0.0, 1.0/60.0, 16), 16);
        assert_eq!(ocean_phases_due(-1.0, 0.0, 1.0/60.0, 16), 1);
    }
}

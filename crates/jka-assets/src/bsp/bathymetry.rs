//! Static, Z-up BSP geometry for a future coastal solver. No movement or GPU state.
//! Water volumes are never seabed. Unknown columns and ambiguous height fields
//! remain explicit; they must not be initialized with a guessed water depth.
use super::{
    brush_side_polygon, patch_vertex, Bsp, Plane, SurfaceKind, CONTENTS_SOLID, CONTENTS_TERRAIN,
    SURF_SKY,
};
use std::collections::VecDeque;
use std::time::Instant;

mod cache;
mod promotion;
mod report;
pub use cache::fingerprint;
pub use promotion::{cluster_water_faces, WaterCluster, WaterFace};

#[cfg(test)]
mod tests;

// OpenJK codemp/game/surfaceflags.h, NOT Quake 3's content bit assignments.
pub const CONTENTS_WATER: u32 = 0x0000_0004;
const LIQUIDS: u32 = CONTENTS_WATER | 0x0000_0002 | 0x0002_0000;
const EPS: f64 = 0.001;
pub const CACHE_VERSION: u32 = 1;

pub type FootprintTriangle = [[f32; 2]; 3];

#[derive(Clone, Debug)]
pub struct Region {
    /// Matches the renderer's ocean slot, including slots occupied by authored oceans.
    pub slot: u32,
    pub level: f32,
    pub triangles: Vec<FootprintTriangle>,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub cell_size: f32,
    pub padding_cells: u32,
    pub max_cells_per_region: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cell_size: 16.0,
            padding_cells: 2,
            max_cells_per_region: 262_144,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Class {
    Outside = 0,
    Wet = 1,
    Dry = 2,
    Outline = 3,
    NoFloor = 4,
    Overhang = 5,
    Mixed = 6,
}

impl Class {
    pub const ALL: [Self; 7] = [
        Self::Outside,
        Self::Wet,
        Self::Dry,
        Self::Outline,
        Self::NoFloor,
        Self::Overhang,
        Self::Mixed,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Outside => "outside footprint",
            Self::Wet => "wet",
            Self::Dry => "dry solid",
            Self::Outline => "partial footprint",
            Self::NoFloor => "missing seabed",
            Self::Overhang => "overhang / stacked floor",
            Self::Mixed => "unresolved geometry",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub class: Class,
    /// NaN means unknown, never zero depth. All lengths are JKA world units.
    pub bed: f32,
    pub depth: f32,
    pub ceiling: f32,
    pub bed_min: f32,
    pub bed_max: f32,
    /// Number of covered probes / 5; diagnostic only, not a volume fraction.
    pub coverage: u8,
    /// 0 = no supported wet component. Connectivity uses solid face barriers.
    pub component: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Boundary {
    Open = 0,
    Wall = 1,
    Outline = 2,
    Unsupported = 3,
}

#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub boundary: Boundary,
    /// Highest supported sampled bed at this face. Wall faces remain sealed
    /// independently of depth; a vertical wall is not a deep-water open edge.
    pub crest: f32,
}

#[derive(Clone, Debug)]
pub struct Grid {
    pub slot: u32,
    pub level: f32,
    /// Lower XY corner; cell values are at centers. JKA XY, Z up.
    pub origin: [f32; 2],
    pub size: [u32; 2],
    pub cell_size: f32,
    pub cells: Vec<Cell>,
    /// Staggered faces: (nx+1)*ny and nx*(ny+1). Outer faces are closed.
    pub faces_x: Vec<Face>,
    pub faces_y: Vec<Face>,
    pub components: u32,
    /// Exact source triangles, including holes, retained independently of the
    /// sampled grid. Future surface reconstruction must use this geometry.
    pub footprint: Vec<FootprintTriangle>,
}

#[derive(Clone, Debug, Default)]
pub struct Atlas {
    pub grids: Vec<Grid>,
    pub warnings: Vec<String>,
    pub bake_ms: f64,
}

impl Atlas {
    pub fn memory_bytes(&self) -> usize {
        self.grids
            .iter()
            .map(|g| {
                g.cells.len() * std::mem::size_of::<Cell>()
                    + (g.faces_x.len() + g.faces_y.len()) * std::mem::size_of::<Face>()
                    + g.footprint.len() * std::mem::size_of::<FootprintTriangle>()
            })
            .sum()
    }
}

struct Solid {
    planes: Vec<Plane>,
    sky: Vec<bool>,
    bounds: [[f32; 2]; 2],
}
struct Triangle {
    points: [[f32; 3]; 3],
    normal: [f64; 3],
    bounds: [[f32; 2]; 2],
}

/// Bake-only acceleration source. Inline/moving models are deliberately excluded.
pub struct Geometry {
    solids: Vec<Solid>,
    triangles: Vec<Triangle>,
    liquids: Vec<Solid>,
}

fn bounds(points: impl Iterator<Item = [f32; 2]>) -> [[f32; 2]; 2] {
    let mut b = [[f32::INFINITY; 2], [f32::NEG_INFINITY; 2]];
    for p in points {
        for a in 0..2 {
            b[0][a] = b[0][a].min(p[a]);
            b[1][a] = b[1][a].max(p[a]);
        }
    }
    b
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f64::from(a[i]) - f64::from(b[i]))
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn solid_contents(contents: u32) -> bool {
    contents & (CONTENTS_SOLID | CONTENTS_TERRAIN) != 0 && contents & LIQUIDS == 0
}

impl Geometry {
    pub fn from_bsp(bsp: &Bsp) -> Result<Self, String> {
        let world = bsp.models.first().ok_or("BSP has no world model")?;
        let extent = world
            .mins
            .iter()
            .chain(&world.maxs)
            .map(|v| v.abs())
            .fold(1024.0, f32::max)
            * 2.0;
        let mut result = Self {
            solids: Vec::new(),
            triangles: Vec::new(),
            liquids: Vec::new(),
        };
        for id in world.brushes.clone() {
            let brush = &bsp.brushes[id];
            let contents = bsp.shaders[brush.shader].contents;
            let liquid = contents & CONTENTS_WATER != 0;
            if !solid_contents(contents) && !liquid {
                continue;
            }
            let mut points = Vec::new();
            let mut planes = Vec::new();
            let mut sky = Vec::new();
            for side in brush.sides.clone() {
                planes.push(bsp.planes[bsp.brush_sides[side].plane]);
                sky.push(bsp.shaders[bsp.brush_sides[side].shader].surface_flags & SURF_SKY != 0);
            }
            // Compiled brushes normally carry axial bounding planes. Use those
            // exact conservative bounds, including for zero-volume compiler
            // remnants: column/segment clipping still tests every brush plane.
            let mut b = [[f32::NEG_INFINITY; 2], [f32::INFINITY; 2]];
            for p in &planes {
                for a in 0..2 {
                    if (0..3).all(|i| i == a || p.normal[i].abs() < 1e-9)
                        && p.normal[a].abs() > 1e-9
                    {
                        let v = p.distance / p.normal[a];
                        if p.normal[a] > 0.0 {
                            b[1][a] = b[1][a].min(v);
                        } else {
                            b[0][a] = b[0][a].max(v);
                        }
                    }
                }
            }
            if b.iter().flatten().any(|v| !v.is_finite()) {
                for side in brush.sides.clone() {
                    points.extend(
                        brush_side_polygon(bsp, brush, side, extent).map_err(|e| e.to_string())?,
                    );
                }
                if points.is_empty() {
                    return Err(format!(
                        "unbounded brush {id}; cannot safely classify its influence"
                    ));
                }
                b = bounds(points.iter().map(|p| [p[0], p[1]]));
            }
            if (0..2).any(|a| b[0][a] > b[1][a]) {
                continue;
            } // proven empty convex volume
            let source = Solid {
                planes,
                sky,
                bounds: b,
            };
            if liquid {
                result.liquids.push(source);
            } else {
                result.solids.push(source);
            }
        }
        // Planar detail, triangle terrain and curved patches can have no brush.
        // Include nodraw collision surfaces too. A patch is explicitly sampled
        // at eight segments per quadratic span, not claimed as an exact curve.
        for id in world.surfaces.clone() {
            let surface = &bsp.surfaces[id];
            let shader = &bsp.shaders[surface.shader];
            if !solid_contents(shader.contents) || shader.surface_flags & SURF_SKY != 0 {
                continue;
            }
            let verts = &bsp.vertices[surface.vertices.clone()];
            let mut append = |v: [super::Vertex; 3]| {
                let points = v.map(|p| p.position);
                let mut normal = cross(sub(points[1], points[0]), sub(points[2], points[0]));
                let authored: [f64; 3] =
                    std::array::from_fn(|a| v.iter().map(|v| f64::from(v.normal[a])).sum());
                if dot(normal, authored) < 0.0 {
                    normal = normal.map(|n| -n);
                }
                let len = dot(normal, normal).sqrt();
                if len < EPS {
                    return;
                }
                normal = normal.map(|n| n / len);
                result.triangles.push(Triangle {
                    points,
                    normal,
                    bounds: bounds(points.iter().map(|p| [p[0], p[1]])),
                });
            };
            match surface.kind {
                SurfaceKind::Patch => {
                    let [w, h] = surface.patch_size;
                    for y in (0..h - 2).step_by(2) {
                        for x in (0..w - 2).step_by(2) {
                            for j in 0..8 {
                                for i in 0..8 {
                                    let a = patch_vertex(
                                        verts,
                                        w,
                                        x,
                                        y,
                                        i as f32 / 8.0,
                                        j as f32 / 8.0,
                                    );
                                    let b = patch_vertex(
                                        verts,
                                        w,
                                        x,
                                        y,
                                        (i + 1) as f32 / 8.0,
                                        j as f32 / 8.0,
                                    );
                                    let c = patch_vertex(
                                        verts,
                                        w,
                                        x,
                                        y,
                                        i as f32 / 8.0,
                                        (j + 1) as f32 / 8.0,
                                    );
                                    let d = patch_vertex(
                                        verts,
                                        w,
                                        x,
                                        y,
                                        (i + 1) as f32 / 8.0,
                                        (j + 1) as f32 / 8.0,
                                    );
                                    append([a, c, b]);
                                    append([b, c, d]);
                                }
                            }
                        }
                    }
                }
                SurfaceKind::Planar | SurfaceKind::Triangles => {
                    for t in bsp.indices[surface.indices.clone()].chunks_exact(3) {
                        append([
                            verts[t[0] as usize],
                            verts[t[1] as usize],
                            verts[t[2] as usize],
                        ]);
                    }
                }
                SurfaceKind::Flare => {}
            }
        }
        Ok(result)
    }

    pub fn bake(&self, regions: &[Region], options: Options) -> Result<Atlas, String> {
        if !options.cell_size.is_finite()
            || options.cell_size < 1.0
            || options.padding_cells > 32
            || options.max_cells_per_region > 1_048_576
        {
            return Err("invalid bathymetry bake limits".into());
        }
        let started = Instant::now();
        let mut atlas = Atlas::default();
        for region in regions {
            match self.bake_region(region, options) {
                Ok(grid) => atlas.grids.push(grid),
                Err(e) => atlas
                    .warnings
                    .push(format!("slot {}: {e}; retain legacy water", region.slot)),
            }
        }
        atlas.bake_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(atlas)
    }

    fn bake_region(&self, region: &Region, options: Options) -> Result<Grid, String> {
        if !region.level.is_finite()
            || region.triangles.is_empty()
            || region
                .triangles
                .iter()
                .flatten()
                .flatten()
                .any(|v| !v.is_finite())
        {
            return Err("invalid water footprint".into());
        }
        let b = bounds(region.triangles.iter().flatten().copied());
        let dx = options.cell_size;
        let origin: [f32; 2] = std::array::from_fn(|a| {
            (b[0][a] / dx).floor() * dx - options.padding_cells as f32 * dx
        });
        let dims: [f32; 2] = std::array::from_fn(|a| {
            ((b[1][a] - origin[a]) / dx).ceil() + options.padding_cells as f32
        });
        if dims
            .iter()
            .any(|n| *n < 1.0 || *n > options.max_cells_per_region as f32)
        {
            return Err("grid dimensions exceed budget".into());
        }
        let size = dims.map(|d| d as u32);
        let n = size[0] as usize * size[1] as usize;
        if n > options.max_cells_per_region {
            return Err(format!(
                "{n} cells exceed fixed {}-cell budget at {dx} units",
                options.max_cells_per_region
            ));
        }
        let solid_bins = bins(self.solids.iter().map(|s| s.bounds), origin, size, dx)?;
        let triangle_bins = bins(self.triangles.iter().map(|t| t.bounds), origin, size, dx)?;
        let footprint_bins = bins(
            region.triangles.iter().map(|t| bounds(t.iter().copied())),
            origin,
            size,
            dx,
        )?;
        let sample =
            |p: [f32; 2], k: usize| self.column(p, region.level, &solid_bins[k], &triangle_bins[k]);
        let inside = |p: [f32; 2], k: usize| {
            footprint_bins[k]
                .iter()
                .any(|&t| contains(region.triangles[t], p))
        };
        let mut cells = Vec::with_capacity(n);
        for y in 0..size[1] {
            for x in 0..size[0] {
                let k = (y * size[0] + x) as usize;
                let center = [
                    origin[0] + (x as f32 + 0.5) * dx,
                    origin[1] + (y as f32 + 0.5) * dx,
                ];
                let probes = [
                    center,
                    [center[0] - 0.49 * dx, center[1] - 0.49 * dx],
                    [center[0] + 0.49 * dx, center[1] - 0.49 * dx],
                    [center[0] - 0.49 * dx, center[1] + 0.49 * dx],
                    [center[0] + 0.49 * dx, center[1] + 0.49 * dx],
                ];
                let samples = probes.map(|p| sample(p, k));
                let coverage = probes.iter().filter(|&&p| inside(p, k)).count() as u8;
                let c = samples[0];
                let bed_min = samples
                    .iter()
                    .filter_map(|c| c.bed)
                    .fold(f32::INFINITY, f32::min);
                let bed_max = samples
                    .iter()
                    .filter_map(|c| c.bed)
                    .fold(f32::NEG_INFINITY, f32::max);
                let class = if c.bed.is_some_and(|b| b >= region.level - EPS as f32) {
                    Class::Dry
                } else if coverage == 0 {
                    Class::Outside
                } else if samples.iter().any(|s| s.bed.is_none()) {
                    Class::NoFloor
                } else if samples.iter().any(|s| s.overhang) {
                    Class::Overhang
                } else if coverage < 5 {
                    Class::Outline
                } else if samples
                    .iter()
                    .any(|s| s.bed.is_some_and(|b| b >= region.level - EPS as f32))
                {
                    Class::Mixed
                } else {
                    Class::Wet
                };
                cells.push(Cell {
                    class,
                    bed: c.bed.unwrap_or(f32::NAN),
                    depth: c.bed.map_or(f32::NAN, |b| (region.level - b).max(0.0)),
                    ceiling: c.ceiling.unwrap_or(f32::NAN),
                    bed_min: if bed_min.is_finite() {
                        bed_min
                    } else {
                        f32::NAN
                    },
                    bed_max: if bed_max.is_finite() {
                        bed_max
                    } else {
                        f32::NAN
                    },
                    coverage,
                    component: 0,
                });
            }
        }
        let sealed = Face {
            boundary: Boundary::Outline,
            crest: f32::NAN,
        };
        let mut grid = Grid {
            slot: region.slot,
            level: region.level,
            origin,
            size,
            cell_size: dx,
            cells,
            faces_x: vec![sealed; (size[0] as usize + 1) * size[1] as usize],
            faces_y: vec![sealed; size[0] as usize * (size[1] as usize + 1)],
            components: 0,
            footprint: region.triangles.clone(),
        };
        for y in 0..size[1] {
            for x in 0..size[0] {
                let k = (y * size[0] + x) as usize;
                let p = [
                    origin[0] + (x as f32 + 0.5) * dx,
                    origin[1] + (y as f32 + 0.5) * dx,
                ];
                for axis in 0..2 {
                    if (axis == 0 && x + 1 == size[0]) || (axis == 1 && y + 1 == size[1]) {
                        continue;
                    }
                    let qk = k + if axis == 0 { 1 } else { size[0] as usize };
                    let mut q = p;
                    q[axis] += dx;
                    let mut mid = p;
                    mid[axis] += dx * 0.5;
                    let solid_candidates: Vec<_> = solid_bins[k]
                        .iter()
                        .chain(&solid_bins[qk])
                        .copied()
                        .collect();
                    let triangle_candidates: Vec<_> = triangle_bins[k]
                        .iter()
                        .chain(&triangle_bins[qk])
                        .copied()
                        .collect();
                    let middle =
                        self.column(mid, region.level, &solid_candidates, &triangle_candidates);
                    let a = grid.cells[k];
                    let b = grid.cells[qk];
                    // Exact convex segment clipping catches walls thinner than a
                    // cell, even when every center/corner sample misses the wall.
                    let wall = solid_bins[k]
                        .iter()
                        .chain(&solid_bins[qk])
                        .any(|&s| self.solids[s].blocks(p, q, region.level))
                        || triangle_bins[k]
                            .iter()
                            .chain(&triangle_bins[qk])
                            .any(|&t| self.triangles[t].blocks(p, q, region.level));
                    let boundary = if wall || a.class == Class::Dry || b.class == Class::Dry {
                        Boundary::Wall
                    } else if a.class == Class::Outside
                        || b.class == Class::Outside
                        || a.class == Class::Outline
                        || b.class == Class::Outline
                        || !segment_covered(
                            p,
                            q,
                            footprint_bins[k]
                                .iter()
                                .chain(&footprint_bins[qk])
                                .map(|&t| region.triangles[t]),
                        )
                    {
                        Boundary::Outline
                    } else if a.class != Class::Wet
                        || b.class != Class::Wet
                        || middle.bed.is_none()
                        || middle.overhang
                    {
                        Boundary::Unsupported
                    } else {
                        Boundary::Open
                    };
                    let crest = middle.bed.map_or(f32::NAN, |v| v.max(a.bed).max(b.bed));
                    let face = Face { boundary, crest };
                    if axis == 0 {
                        grid.faces_x[(y * (size[0] + 1) + x + 1) as usize] = face;
                    } else {
                        grid.faces_y[((y + 1) * size[0] + x) as usize] = face;
                    }
                }
            }
        }
        grid.label_components();
        Ok(grid)
    }

    fn column(&self, p: [f32; 2], level: f32, solids: &[usize], triangles: &[usize]) -> Column {
        let mut spans: Vec<_> = solids
            .iter()
            .filter_map(|&s| self.solids[s].column(p))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64, bool)> = Vec::new();
        for span in spans {
            if let Some(last) = merged.last_mut().filter(|last| span.0 <= last.1 + EPS) {
                if span.1 > last.1 {
                    last.1 = span.1;
                    last.2 = span.2;
                }
            } else {
                merged.push(span);
            }
        }
        let level = f64::from(level);
        let containing = merged.iter().find(|s| s.0 <= level && s.1 >= level - EPS);
        let mut bed = containing.filter(|s| !s.2).map(|s| s.1);
        if containing.is_none() {
            bed = merged
                .iter()
                .filter(|s| s.1 < level && !s.2)
                .map(|s| s.1)
                .max_by(f64::total_cmp);
        }
        let mut ceiling = merged
            .iter()
            .filter(|s| s.0 > level)
            .map(|s| s.0)
            .min_by(f64::total_cmp);
        let mut below_ceiling = None::<f64>;
        for &t in triangles {
            let t = &self.triangles[t];
            let Some(z) = t.height(p) else {
                continue;
            };
            if t.normal[2] > EPS && z < level + EPS && bed.is_none_or(|b| z > b) {
                bed = Some(z);
            }
            if t.normal[2] < -EPS {
                if z > level && ceiling.is_none_or(|c| z < c) {
                    ceiling = Some(z);
                }
                if z < level && below_ceiling.is_none_or(|c| z > c) {
                    below_ceiling = Some(z);
                }
            }
        }
        // Multiple separated solid layers below the water surface are not a
        // single-valued bed. Reject rather than treating a submerged bridge as
        // a continuous filled mound and hiding the water below it.
        // A water volume identifies which vertical layer belongs to this body.
        // It never supplies a bed or limits the measured depth. Rooms below the
        // authored volume do not make an otherwise valid basin an overhang.
        let liquid_bottom = self
            .liquids
            .iter()
            .filter(|s| (0..2).all(|a| p[a] >= s.bounds[0][a] && p[a] <= s.bounds[1][a]))
            .filter_map(|s| s.column(p))
            .filter(|s| (s.1 - level).abs() < 1.01)
            .map(|s| s.0)
            .min_by(f64::total_cmp);
        let stacked = merged
            .iter()
            .filter(|s| s.1 < level - EPS && liquid_bottom.is_none_or(|bottom| s.1 > bottom + EPS))
            .count()
            > 1;
        // An overhead bridge is supported above this water layer: retain its
        // ceiling/headroom for the eventual solver and renderer. A submerged
        // underside above the selected bed is an ambiguous fluid column.
        let overhang = stacked || below_ceiling.is_some_and(|c| bed.is_some_and(|b| c > b + EPS));
        Column {
            bed: bed.map(|b| b as f32),
            ceiling: ceiling.map(|c| c as f32),
            overhang,
        }
    }
}

#[derive(Clone, Copy)]
struct Column {
    bed: Option<f32>,
    ceiling: Option<f32>,
    overhang: bool,
}

impl Solid {
    fn column(&self, p: [f32; 2]) -> Option<(f64, f64, bool)> {
        let (mut low, mut high, mut sky) = (f64::NEG_INFINITY, f64::INFINITY, false);
        for (i, plane) in self.planes.iter().enumerate() {
            let rhs = f64::from(plane.distance)
                - f64::from(plane.normal[0]) * f64::from(p[0])
                - f64::from(plane.normal[1]) * f64::from(p[1]);
            let nz = f64::from(plane.normal[2]);
            if nz.abs() < 1e-9 {
                if rhs < -EPS {
                    return None;
                }
            } else if nz > 0.0 {
                let h = rhs / nz;
                if h < high {
                    high = h;
                    sky = self.sky[i];
                }
            } else {
                low = low.max(rhs / nz);
            }
        }
        (low.is_finite() && high.is_finite() && low <= high + EPS).then_some((low, high, sky))
    }
    fn blocks(&self, a: [f32; 2], b: [f32; 2], z: f32) -> bool {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for plane in &self.planes {
            let d = f64::from(plane.normal[0]) * f64::from(b[0] - a[0])
                + f64::from(plane.normal[1]) * f64::from(b[1] - a[1]);
            let rhs = f64::from(plane.distance)
                - f64::from(plane.normal[0]) * f64::from(a[0])
                - f64::from(plane.normal[1]) * f64::from(a[1])
                - f64::from(plane.normal[2]) * f64::from(z - EPS as f32);
            if d.abs() < 1e-9 {
                if rhs < -EPS {
                    return false;
                }
            } else if d > 0.0 {
                hi = hi.min(rhs / d);
            } else {
                lo = lo.max(rhs / d);
            }
            if lo > hi + 1e-9 {
                return false;
            }
        }
        hi >= 0.0 && lo <= 1.0
    }
}

impl Triangle {
    fn height(&self, p: [f32; 2]) -> Option<f64> {
        if self.normal[2].abs() < 1e-9 || !contains(self.points.map(|p| [p[0], p[1]]), p) {
            return None;
        }
        let a = self.points[0];
        Some(
            f64::from(a[2])
                - (self.normal[0] * f64::from(p[0] - a[0])
                    + self.normal[1] * f64::from(p[1] - a[1]))
                    / self.normal[2],
        )
    }
    fn blocks(&self, a: [f32; 2], b: [f32; 2], z: f32) -> bool {
        let origin = [a[0], a[1], z - EPS as f32];
        let end = [b[0], b[1], z - EPS as f32];
        let dir = sub(end, origin);
        let e1 = sub(self.points[1], self.points[0]);
        let e2 = sub(self.points[2], self.points[0]);
        let h = cross(dir, e2);
        let det = dot(e1, h);
        if det.abs() < 1e-9 {
            return false;
        }
        let s = sub(origin, self.points[0]);
        let u = dot(s, h) / det;
        if !(-1e-7..=1.0000001).contains(&u) {
            return false;
        }
        let q = cross(s, e1);
        let v = dot(dir, q) / det;
        let t = dot(e2, q) / det;
        v >= -1e-7 && u + v <= 1.0000001 && (-1e-7..=1.0000001).contains(&t)
    }
}

fn contains(t: FootprintTriangle, p: [f32; 2]) -> bool {
    let mut positive = false;
    let mut negative = false;
    let mut area = 0.0_f64;
    for i in 0..3 {
        let a = t[i];
        let b = t[(i + 1) % 3];
        let e = f64::from(b[0] - a[0]) * f64::from(p[1] - a[1])
            - f64::from(b[1] - a[1]) * f64::from(p[0] - a[0]);
        positive |= e > 1e-6;
        negative |= e < -1e-6;
        area += f64::from(a[0]) * f64::from(b[1]) - f64::from(a[1]) * f64::from(b[0]);
    }
    area.abs() > 1e-6 && !(positive && negative)
}

/// Union of exact triangle/segment intervals. Midpoint tests alone can miss a
/// narrow gap or island between two wet cells and falsely connect the water.
fn segment_covered(
    a: [f32; 2],
    b: [f32; 2],
    triangles: impl Iterator<Item = FootprintTriangle>,
) -> bool {
    let mut intervals = Vec::new();
    for t in triangles {
        let orientation = f64::from(t[1][0] - t[0][0]) * f64::from(t[2][1] - t[0][1])
            - f64::from(t[1][1] - t[0][1]) * f64::from(t[2][0] - t[0][0]);
        if orientation.abs() < 1e-9 {
            continue;
        }
        let sign = orientation.signum();
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        let mut valid = true;
        for i in 0..3 {
            let p = t[i];
            let q = t[(i + 1) % 3];
            let dx = f64::from(q[0] - p[0]);
            let dy = f64::from(q[1] - p[1]);
            let start = sign * (dx * f64::from(a[1] - p[1]) - dy * f64::from(a[0] - p[0]));
            let slope = sign * (dx * f64::from(b[1] - a[1]) - dy * f64::from(b[0] - a[0]));
            if slope.abs() < 1e-12 {
                if start < -1e-8 {
                    valid = false;
                    break;
                }
            } else if slope > 0.0 {
                lo = lo.max(-start / slope);
            } else {
                hi = hi.min(-start / slope);
            }
        }
        if valid && lo <= hi {
            intervals.push((lo, hi));
        }
    }
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reached = 0.0;
    for (lo, hi) in intervals {
        if lo > reached + 1e-7 {
            return false;
        }
        reached = f64::max(reached, hi);
        if reached >= 1.0 - 1e-7 {
            return true;
        }
    }
    false
}

fn bins(
    bounds: impl Iterator<Item = [[f32; 2]; 2]>,
    origin: [f32; 2],
    size: [u32; 2],
    dx: f32,
) -> Result<Vec<Vec<usize>>, String> {
    let mut bins = vec![Vec::new(); size[0] as usize * size[1] as usize];
    let mut references = 0usize;
    for (id, b) in bounds.enumerate() {
        let low: [i64; 2] = std::array::from_fn(|a| ((b[0][a] - origin[a]) / dx).floor() as i64);
        let high: [i64; 2] = std::array::from_fn(|a| ((b[1][a] - origin[a]) / dx).floor() as i64);
        let low = low.map(|n| n.max(0));
        let high: [i64; 2] = std::array::from_fn(|a| high[a].min(i64::from(size[a]) - 1));
        if (0..2).any(|a| low[a] > high[a]) {
            continue;
        }
        references += ((high[0] - low[0] + 1) * (high[1] - low[1] + 1)) as usize;
        if references > 8_000_000 {
            return Err("geometry reference budget exceeded".into());
        }
        for y in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                bins[y as usize * size[0] as usize + x as usize].push(id);
            }
        }
    }
    Ok(bins)
}

impl Grid {
    fn label_components(&mut self) {
        let nx = self.size[0] as usize;
        let ny = self.size[1] as usize;
        for start in 0..self.cells.len() {
            if self.cells[start].class != Class::Wet || self.cells[start].component != 0 {
                continue;
            }
            self.components += 1;
            self.cells[start].component = self.components;
            let mut queue = VecDeque::from([start]);
            while let Some(k) = queue.pop_front() {
                let x = k % nx;
                let y = k / nx;
                let neighbours = [
                    (x > 0, k.wrapping_sub(1), self.faces_x[y * (nx + 1) + x]),
                    (x + 1 < nx, k + 1, self.faces_x[y * (nx + 1) + x + 1]),
                    (y > 0, k.wrapping_sub(nx), self.faces_y[y * nx + x]),
                    (y + 1 < ny, k + nx, self.faces_y[(y + 1) * nx + x]),
                ];
                for (valid, q, face) in neighbours {
                    if valid
                        && face.boundary == Boundary::Open
                        && self.cells[q].class == Class::Wet
                        && self.cells[q].component == 0
                    {
                        self.cells[q].component = self.components;
                        queue.push_back(q);
                    }
                }
            }
        }
    }
}

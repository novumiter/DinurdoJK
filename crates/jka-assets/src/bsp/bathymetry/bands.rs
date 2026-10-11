//! Conservative geometry-first allocation for large promoted pools. This is
//! bake-time work, never a frame/PVS-dependent fluid allocation.
use super::*;
use std::collections::HashSet;

#[path = "screen_report.rs"]
mod screen_report;

/// Explicit fixed-width policy for pools whose complete shallow band cannot
/// fit. Include every geometric waterline seed; never choose a subset of banks.
const PATCH_RADIUS_CELLS: u32 = 8;

struct Candidate {
    polygon: Vec<[f64; 2]>,
    height: [f64; 3],
    source: String,
}

impl Geometry {
    fn shoreline_candidates(
        &self,
        level: f32,
        max_depth: f32,
        pool_bounds: [[f32; 2]; 2],
    ) -> Vec<Candidate> {
        let lower = f64::from(level) - f64::from(max_depth) - EPS;
        let upper = f64::from(level) + EPS;
        let mut candidates = Vec::new();
        for (id, solid) in self.solids.iter().enumerate() {
            if (0..2).any(|a| {
                solid.bounds[1][a] < pool_bounds[0][a] || solid.bounds[0][a] > pool_bounds[1][a]
            }) {
                continue;
            }
            for (face, plane) in solid.planes.iter().enumerate() {
                if solid.sky[face] || f64::from(plane.normal[2]) <= EPS {
                    continue;
                }
                let z = [
                    -f64::from(plane.normal[0]) / f64::from(plane.normal[2]),
                    -f64::from(plane.normal[1]) / f64::from(plane.normal[2]),
                    f64::from(plane.distance) / f64::from(plane.normal[2]),
                ];
                let [a, b] = solid.bounds;
                let mut polygon = vec![
                    a.map(f64::from),
                    [f64::from(b[0]), f64::from(a[1])],
                    b.map(f64::from),
                    [f64::from(a[0]), f64::from(b[1])],
                ];
                for p in &solid.planes {
                    polygon = clip(
                        &polygon,
                        [
                            f64::from(p.normal[0]) + f64::from(p.normal[2]) * z[0],
                            f64::from(p.normal[1]) + f64::from(p.normal[2]) * z[1],
                        ],
                        f64::from(p.distance) - f64::from(p.normal[2]) * z[2] + EPS,
                    );
                    if polygon.is_empty() {
                        break;
                    }
                }
                polygon = clip(&polygon, [-z[0], -z[1]], z[2] - lower);
                polygon = clip(&polygon, [z[0], z[1]], upper - z[2]);
                if !polygon.is_empty() {
                    candidates.push(Candidate {
                        polygon,
                        height: z,
                        source: format!("solid {id} face {face}"),
                    });
                }
            }
        }
        for (id, t) in self.triangles.iter().enumerate() {
            if t.normal[2] <= EPS
                || (0..2).any(|a| {
                    t.bounds[1][a] < pool_bounds[0][a] || t.bounds[0][a] > pool_bounds[1][a]
                })
            {
                continue;
            }
            let p = t.points[0].map(f64::from);
            let z = [
                -t.normal[0] / t.normal[2],
                -t.normal[1] / t.normal[2],
                p[2] + t.normal[0] * p[0] / t.normal[2] + t.normal[1] * p[1] / t.normal[2],
            ];
            let polygon: Vec<_> = t
                .points
                .iter()
                .map(|p| [f64::from(p[0]), f64::from(p[1])])
                .collect();
            let polygon = clip(&polygon, [-z[0], -z[1]], z[2] - lower);
            let polygon = clip(&polygon, [z[0], z[1]], upper - z[2]);
            if !polygon.is_empty() {
                candidates.push(Candidate {
                    polygon,
                    height: z,
                    source: format!("triangle {id}"),
                });
            }
        }
        candidates
    }

    /// Small pools keep their complete diagnostic atlas. Large pools retain
    /// every upward solid/terrain surface in the shallow-depth slab, plus a
    /// whole-tile halo for dry-bank and deeper FFT-interface samples. When this
    /// exceeds the budget, retain fixed-width patches around every candidate
    /// bank crossing, with a guard for validated offshore FFT coupling. A missed
    /// narrow bank is not ruled out by a handful of coarse column probes.
    /// The full rendering footprint is owned separately by the caller.
    pub fn bake_shoreline_bands(
        &self,
        regions: &[Region],
        options: Options,
        max_depth: f32,
    ) -> Result<Atlas, String> {
        if !max_depth.is_finite()
            || max_depth <= 0.0
            || !options.cell_size.is_finite()
            || options.cell_size < 1.0
            || options.max_cells_per_region == 0
            || options.max_cells_per_region > 1_048_576
        {
            return Err("invalid shoreline screening depth".into());
        }
        let started = Instant::now();
        let mut slots: std::collections::BTreeMap<u32, Vec<&Region>> = Default::default();
        for region in regions {
            slots.entry(region.slot).or_default().push(region);
        }
        let mut retained = Vec::new();
        let mut warnings = Vec::new();
        let mut bands = Vec::new();
        for (slot, tiles) in slots {
            let first = tiles[0];
            let Some([min, max]) = first.bounds else {
                return Err("shoreline screening requires explicit footprint tiles".into());
            };
            let tile_size = max[0] - min[0];
            let axis = (tile_size / options.cell_size).round() as usize;
            if !tile_size.is_finite()
                || tile_size <= 0.0
                || axis == 0
                || (tile_size - axis as f32 * options.cell_size).abs() > 0.01
                || tiles.iter().any(|r| {
                    !r.level.is_finite()
                        || r.level != first.level
                        || r.bounds.is_none_or(|b| {
                            b.iter().flatten().any(|v| !v.is_finite())
                                || (0..2).any(|a| {
                                    (b[1][a] - b[0][a] - tile_size).abs() > 0.01
                                        || (b[0][a] / tile_size - (b[0][a] / tile_size).round())
                                            .abs()
                                            > 0.001
                                })
                        })
                })
            {
                return Err("shoreline screening requires aligned square footprint tiles".into());
            }
            let cells_per_tile = axis.saturating_mul(axis);
            let original = tiles.len().saturating_mul(cells_per_tile);
            if original <= options.max_cells_per_region {
                retained.extend(tiles.into_iter().cloned());
                continue;
            }
            let limit = options.max_cells_per_region / cells_per_tile;
            let lookup: HashMap<[i32; 2], usize> = tiles
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let p = r.bounds.unwrap()[0];
                    (p.map(|v| (v / tile_size).round() as i32), i)
                })
                .collect();
            if lookup.len() != tiles.len() {
                return Err("overlapping shoreline footprint tiles".into());
            }
            let pool_bounds = bounds(tiles.iter().flat_map(|r| r.bounds.unwrap()));
            let mut selected = HashSet::new();
            let mut over_budget = false;
            let mut visit = |polygon: &[[f64; 2]]| {
                if over_budget || polygon.is_empty() {
                    return;
                }
                let b = bounds(polygon.iter().map(|p| p.map(|v| v as f32)));
                if (0..2).any(|a| b[1][a] < pool_bounds[0][a] || b[0][a] > pool_bounds[1][a]) {
                    return;
                }
                let lo: [i32; 2] = std::array::from_fn(|a| {
                    ((b[0][a].max(pool_bounds[0][a]) - EPS as f32) / tile_size).floor() as i32
                });
                let hi: [i32; 2] = std::array::from_fn(|a| {
                    ((b[1][a].min(pool_bounds[1][a]) + EPS as f32) / tile_size).floor() as i32
                });
                let mut test = |coordinate: [i32; 2], index: usize| {
                    if over_budget {
                        return;
                    }
                    if !overlaps_polygon(polygon, tiles[index].bounds.unwrap()) {
                        return;
                    }
                    // Include diagonal neighbors: reconstruction and corner
                    // probes must not hit an artificial offshore tile cut.
                    for y in -1..=1 {
                        for x in -1..=1 {
                            if let Some(&q) = lookup.get(&[coordinate[0] + x, coordinate[1] + y]) {
                                selected.insert(q);
                            }
                        }
                    }
                    if selected.len() > limit {
                        over_budget = true;
                    }
                };
                let area = (i64::from(hi[0]) - i64::from(lo[0]) + 1)
                    .saturating_mul(i64::from(hi[1]) - i64::from(lo[1]) + 1);
                // A diagonal terrain face may have a huge rectangle. Never
                // enumerate beyond the actual sparse footprint in that case.
                if area > tiles.len() as i64 {
                    for (&coordinate, &index) in &lookup {
                        if (0..2).all(|a| coordinate[a] >= lo[a] && coordinate[a] <= hi[a]) {
                            test(coordinate, index);
                        }
                    }
                } else {
                    for y in lo[1]..=hi[1] {
                        for x in lo[0]..=hi[0] {
                            if let Some(&index) = lookup.get(&[x, y]) {
                                test([x, y], index);
                            }
                        }
                    }
                }
            };
            let candidates = self.shoreline_candidates(first.level, max_depth, pool_bounds);
            for candidate in &candidates {
                visit(&candidate.polygon);
            }
            let mut patch_radius_cells = 0;
            if over_budget {
                selected.clear();
                over_budget = false;
                // A four-cell guard covers seed sampling offsets, the one-cell
                // FFT reservoir and its exterior validation cell. Allocation
                // cuts stay outside the physical patch and coupling ring.
                let margin = (PATCH_RADIUS_CELLS + 4) as f32 * options.cell_size;
                for c in &candidates {
                    let [x, y, z] = c.height;
                    let slope2 = x * x + y * y;
                    if slope2 < 1e-12 || 1.0 / (1.0 + slope2).sqrt() <= WALL_NORMAL_Z_MAX {
                        continue; // horizontal platforms and rigid walls are not banks
                    }
                    let bank = clip(&c.polygon, [-x, -y], z - f64::from(first.level) + EPS);
                    if bank.is_empty() {
                        continue;
                    }
                    let b = bounds(bank.iter().map(|p| p.map(|v| v as f32)));
                    for (i, tile) in tiles.iter().enumerate() {
                        let rectangle = tile.bounds.unwrap();
                        if (0..2).any(|a| {
                            b[1][a] + margin < rectangle[0][a] || b[0][a] - margin > rectangle[1][a]
                        }) {
                            continue;
                        }
                        let expanded = [
                            rectangle[0].map(|v| v - margin),
                            rectangle[1].map(|v| v + margin),
                        ];
                        if overlaps_polygon(&bank, expanded) {
                            selected.insert(i);
                        }
                    }
                    if selected.len() > limit {
                        over_budget = true;
                        break;
                    }
                }
                patch_radius_cells = PATCH_RADIUS_CELLS;
            }
            if over_budget {
                warnings.push(format!("slot {slot}: shallow bands plus halo exceed budget; all bank-seeded patches also exceed fixed {}-cell budget (full footprint {original} cells); retain FFT water", options.max_cells_per_region));
            } else if selected.is_empty() {
                warnings.push(if patch_radius_cells == 0 {
                    format!("slot {slot}: FFT-only; no bed surfaces within {max_depth:.1}u of water level; full footprint {original} cells")
                } else {
                    format!("slot {slot}: FFT-only; oversized shallow band has no geometric bank crossings; full footprint {original} cells")
                });
            } else {
                bands.push(ShorelineBand {
                    slot,
                    full_tiles: tiles.len() as u32,
                    retained_tiles: selected.len() as u32,
                    max_depth,
                    patch_radius_cells,
                });
                warnings.push(format!("slot {slot}: shoreline bands; {} retained tiles / {} full footprint tiles; screening depth {max_depth:.1}u; bank patch radius {patch_radius_cells} cells (0 = complete shallow band)", selected.len(), tiles.len()));
                for (i, r) in tiles.into_iter().enumerate() {
                    if selected.contains(&i) {
                        retained.push(r.clone());
                    }
                }
            }
        }
        let mut atlas = self.bake(&retained, options)?;
        atlas.warnings.extend(warnings);
        // Missing a required tile would introduce a sealed cut inside the
        // shallow band. Reject that pool atomically, not a partial coastline.
        bands.retain(|band| {
            let count = atlas.grids.iter().filter(|g| g.slot == band.slot).count();
            if count == band.retained_tiles as usize {
                return true;
            }
            atlas.grids.retain(|g| g.slot != band.slot);
            atlas.warnings.push(format!(
                "slot {}: incomplete shoreline-band bake; retain FFT water",
                band.slot
            ));
            false
        });
        atlas.shoreline_bands = bands;
        atlas.bake_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(atlas)
    }
}

// Convex half-plane clipping in f64. Keep even degenerate intersections: a
// sub-cell feature at the waterline still needs its surrounding tiles baked.
fn clip(polygon: &[[f64; 2]], normal: [f64; 2], distance: f64) -> Vec<[f64; 2]> {
    if polygon.is_empty() {
        return Vec::new();
    }
    let signed = |p: [f64; 2]| p[0] * normal[0] + p[1] * normal[1] - distance;
    let mut out = Vec::new();
    let mut a = *polygon.last().unwrap();
    let mut da = signed(a);
    for &b in polygon {
        let db = signed(b);
        if (da <= 0.0) != (db <= 0.0) {
            let t = da / (da - db);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
        if db <= 0.0 {
            out.push(b);
        }
        a = b;
        da = db;
    }
    out
}

fn overlaps_polygon(polygon: &[[f64; 2]], rectangle: [[f32; 2]; 2]) -> bool {
    let mut polygon = polygon.to_vec();
    for axis in 0..2 {
        let mut normal = [0.0; 2];
        normal[axis] = 1.0;
        polygon = clip(&polygon, normal, f64::from(rectangle[1][axis]) + EPS);
        normal[axis] = -1.0;
        polygon = clip(&polygon, normal, -f64::from(rectangle[0][axis]) + EPS);
        if polygon.is_empty() {
            return false;
        }
    }
    true
}

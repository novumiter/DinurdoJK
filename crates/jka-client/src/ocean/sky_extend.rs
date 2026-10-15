//! Render-only, camera-relative continuation of promoted water beyond BSP sky walls.
//! These contacts are deliberately independent from coastal and bathymetry data.
use jka_assets::bsp::bathymetry::WaterFootprint;

pub const SEGMENTS_PER_POOL: usize = 16;
const PER_SIDE: usize = SEGMENTS_PER_POOL / 4;
const TOUCH: f32 = 24.0;
const HEIGHT_TOLERANCE: f32 = 32.0;
const EDGE_EPSILON: f32 = 2.0;

/// Renderer X/Y/Z coordinates (Y up).
pub type SkyTriangle = [[f32; 3]; 3];

/// Each segment: [sky-wall coordinate, tangential minimum, tangential maximum,
/// side+1]. Side order is -X,+X,-Z,+Z; 0 means unused.
/// There is intentionally NO world-space extension distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyExtension {
    pub bounds: [f32; 4],
    pub segments: [[f32; 4]; SEGMENTS_PER_POOL],
    pub count: usize,
}

impl SkyExtension {
    pub fn contains(&self, p: [f32; 2]) -> bool {
        let [min_x, min_z, max_x, max_z] = self.bounds;
        for segment in self.segments.iter().take(self.count) {
            let [wall, lo, hi, side] = *segment;
            match side as usize {
                1 if p[0] <= wall + EDGE_EPSILON && p[1] >= lo && p[1] <= hi => return true,
                2 if p[0] >= wall - EDGE_EPSILON && p[1] >= lo && p[1] <= hi => return true,
                3 if p[1] <= wall + EDGE_EPSILON && p[0] >= lo && p[0] <= hi => return true,
                4 if p[1] >= wall - EDGE_EPSILON && p[0] >= lo && p[0] <= hi => return true,
                _ => {}
            }
        }
        // At a corner, two sky walls opening onto the same water body must
        // admit the entire outside quadrant, not two strips meeting at a point.
        // Require BOTH contacted edges to touch the actual shared corner.
        let x_side = if p[0] < min_x { Some(1) } else if p[0] > max_x { Some(2) } else { None };
        let z_side = if p[1] < min_z { Some(3) } else if p[1] > max_z { Some(4) } else { None };
        if let (Some(xs), Some(zs)) = (x_side, z_side) {
            let x_corner = if xs == 1 { min_x } else { max_x };
            let z_corner = if zs == 3 { min_z } else { max_z };
            let touches = |side, tangent: f32| self.segments.iter().take(self.count).any(|s| {
                s[3] as usize == side && tangent >= s[1] - EDGE_EPSILON && tangent <= s[2] + EDGE_EPSILON
            });
            return touches(xs, z_corner) && touches(zs, x_corner);
        }
        false
    }

    /// Conservative CPU test used only to prune provably dry clipmap cells.
    /// The GPU does the precise water outline and sky contact test.
    pub fn rect_may_contain(&self, lo: [f32; 2], hi: [f32; 2]) -> bool {
        for segment in self.segments.iter().take(self.count) {
            let [wall, a, b, side] = *segment;
            let possible = match side as usize {
                1 => lo[0] <= wall + EDGE_EPSILON && hi[1] >= a && lo[1] <= b,
                2 => hi[0] >= wall - EDGE_EPSILON && hi[1] >= a && lo[1] <= b,
                3 => lo[1] <= wall + EDGE_EPSILON && hi[0] >= a && lo[0] <= b,
                4 => hi[1] >= wall - EDGE_EPSILON && hi[0] >= a && lo[0] <= b,
                _ => false,
            };
            if possible { return true; }
        }
        // A rectangle can lie completely in an open corner, outside both
        // strips. Sample the nearest quadrant point, not the distant corners.
        let [min_x, min_z, max_x, max_z] = self.bounds;
        for &x in &[min_x - EDGE_EPSILON*2.0, max_x + EDGE_EPSILON*2.0] {
            if (x < min_x && lo[0] >= min_x) || (x > max_x && hi[0] <= max_x) { continue; }
            for &z in &[min_z - EDGE_EPSILON*2.0, max_z + EDGE_EPSILON*2.0] {
                if (z < min_z && lo[1] >= min_z) || (z > max_z && hi[1] <= max_z) { continue; }
                if self.contains([x, z]) { return true; }
            }
        }
        false
    }
}

/// Discover vertical, axis-aligned sky walls contacting REAL water triangles.
/// No synthetic water mesh, atlas, bathymetry, or fixed-radius strips are built.
pub fn detect(original: &WaterFootprint, sky_triangles: &[SkyTriangle]) -> Option<SkyExtension> {
    if original.triangles.is_empty() || !original.plane.is_finite() { return None; }
    let mut result = SkyExtension {
        bounds: [original.minimum[0], original.minimum[1], original.maximum[0], original.maximum[1]],
        segments: [[0.0; 4]; SEGMENTS_PER_POOL], count: 0,
    };
    for side in 0..4 {
        let axis = side / 2;
        let tangent = 1 - axis;
        let outward_min = side % 2 == 0;
        let edge = if outward_min { original.minimum[axis] } else { original.maximum[axis] };
        let contacts: Vec<[f32;2]> = original.triangles.iter().filter_map(|water| {
            let e = if outward_min {
                water.iter().map(|v| v[axis]).fold(f32::INFINITY, f32::min)
            } else {
                water.iter().map(|v| v[axis]).fold(f32::NEG_INFINITY, f32::max)
            };
            if (e-edge).abs() > TOUCH { return None; }
            Some([
                water.iter().map(|v| v[tangent]).fold(f32::INFINITY,f32::min),
                water.iter().map(|v| v[tangent]).fold(f32::NEG_INFINITY,f32::max),
            ])
        }).collect();
        if contacts.is_empty() { continue; }
        let mut intervals = Vec::<[f32;2]>::new();
        for sky in sky_triangles {
            let height_lo = sky.iter().map(|v| v[1]).fold(f32::INFINITY, f32::min);
            let height_hi = sky.iter().map(|v| v[1]).fold(f32::NEG_INFINITY, f32::max);
            if height_lo > original.plane + HEIGHT_TOLERANCE || height_hi < original.plane - HEIGHT_TOLERANCE { continue; }
            let coord = |v: &[f32;3], a: usize| v[if a == 0 { 0 } else { 2 }];
            let wall_lo = sky.iter().map(|v| coord(v, axis)).fold(f32::INFINITY,f32::min);
            let wall_hi = sky.iter().map(|v| coord(v, axis)).fold(f32::NEG_INFINITY,f32::max);
            if wall_hi-wall_lo > TOUCH || (0.5*(wall_lo+wall_hi)-edge).abs() > TOUCH { continue; }
            let tang_lo = sky.iter().map(|v|coord(v,tangent)).fold(f32::INFINITY,f32::min);
            let tang_hi = sky.iter().map(|v|coord(v,tangent)).fold(f32::NEG_INFINITY,f32::max);
            if tang_hi-tang_lo < 1.0 { continue; }
            for contact in &contacts {
                let lo = tang_lo.max(contact[0]);
                let hi = tang_hi.min(contact[1]);
                if hi-lo >= 1.0 { intervals.push([lo,hi]); }
            }
        }
        intervals.sort_by(|a,b| a[0].total_cmp(&b[0]));
        let mut merged = Vec::<[f32;2]>::new();
        for interval in intervals {
            if let Some(last) = merged.last_mut() {
                if interval[0] <= last[1] + 8.0 {
                    last[1] = last[1].max(interval[1]);
                    continue;
                }
            }
            merged.push(interval);
        }
        merged.sort_by(|a,b| (b[1]-b[0]).total_cmp(&(a[1]-a[0])));
        for interval in merged.into_iter().take(PER_SIDE) {
            result.segments[result.count] = [edge,
                (interval[0]-EDGE_EPSILON).max(original.minimum[tangent]),
                (interval[1]+EDGE_EPSILON).min(original.maximum[tangent]),
                (side+1) as f32];
            result.count += 1;
        }
    }
    (result.count > 0).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jka_assets::bsp::bathymetry::{build_water_footprints, WaterFace};
    fn water() -> WaterFootprint {
        build_water_footprints(
            &[WaterFace { plane: 0.0, minimum:[0.0,0.0], maximum:[100.0,100.0], authored_ocean:None, material:0 }],
            &[vec![[[0.0,0.0],[100.0,0.0],[0.0,100.0]],[[100.0,0.0],[100.0,100.0],[0.0,100.0]]]],
            256.0,
        ).remove(0)
    }
    #[test]
    fn camera_can_travel_arbitrarily_far_outside_a_sky_wall() {
        let sky = detect(&water(), &[[[100.0,-10.0,0.0],[100.0,100.0,0.0],[100.0,-10.0,100.0]]]).unwrap();
        assert!(sky.contains([1_000_000.0, 50.0]));
        assert!(!sky.contains([-100.0, 50.0]));
        assert!(!sky.contains([1_000_000.0, 150.0]));
    }
    #[test]
    fn touching_sky_walls_fill_the_outer_corner() {
        let sky = detect(&water(), &[
            [[100.0,-10.0,0.0],[100.0,100.0,0.0],[100.0,-10.0,100.0]],
            [[0.0,-10.0,100.0],[100.0,100.0,100.0],[100.0,-10.0,100.0]],
        ]).unwrap();
        assert!(sky.contains([100000.0,100000.0]));
        assert!(sky.rect_may_contain([10000.0,10000.0],[11000.0,11000.0]));
    }
    #[test]
    fn sky_ceiling_and_distant_wall_do_not_extend() {
        assert!(detect(&water(), &[[[100.0,300.0,0.0],[100.0,300.0,100.0],[0.0,300.0,0.0]]]).is_none());
        assert!(detect(&water(), &[[[300.0,-10.0,0.0],[300.0,100.0,0.0],[300.0,-10.0,100.0]]]).is_none());
    }
}

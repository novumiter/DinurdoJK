//! Shared promoted-water footprint construction.
//!
//! A footprint is assembled from the exact upward-facing water triangles.  The
//! enclosing bounds are derived metadata, never the source shape.  Sparse tiles
//! provide a common broad phase for rendering and coastal-grid allocation.
use std::collections::BTreeMap;

pub type WaterTriangle = [[f32; 2]; 3];

#[derive(Clone, Debug, PartialEq)]
pub struct WaterFootprintTile {
    pub coordinate: [i32; 2],
    pub minimum: [f32; 2],
    pub maximum: [f32; 2],
    /// Indices into [`WaterFootprint::triangles`].
    pub triangles: Vec<u32>,
}

/// One connected, coplanar water body. Brush/draw-surface boundaries are
/// retained only in `members`; consumers operate on their exact triangle union.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterFootprint {
    pub plane: f32,
    pub minimum: [f32; 2],
    pub maximum: [f32; 2],
    pub members: Vec<usize>,
    pub triangles: Vec<WaterTriangle>,
    pub tile_size: f32,
    pub tiles: Vec<WaterFootprintTile>,
}

impl WaterFootprint {
    pub fn area(&self) -> f32 {
        self.triangles
            .iter()
            .map(|t| {
                ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1])
                    - (t[1][1] - t[0][1]) * (t[2][0] - t[0][0]))
                    .abs()
                    * 0.5
            })
            .sum()
    }
}
/// One upward authored water face as the promotion pass sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterFace {
    pub plane: f32,
    pub minimum: [f32; 2],
    pub maximum: [f32; 2],
    /// Faces of different map-authored oceans never merge: each has its own
    /// wave settings.
    pub authored_ocean: Option<usize>,
    /// Different shader appearances require separate replacement draws.
    pub material: u32,
}

/// Contiguous water on one plane, promoted as a single ocean surface.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterCluster {
    pub plane: f32,
    pub minimum: [f32; 2],
    pub maximum: [f32; 2],
    /// Indices into the face slice handed to `cluster_water_faces`.
    pub members: Vec<usize>,
}

impl WaterCluster {
    pub fn area(&self) -> f32 {
        (self.maximum[0] - self.minimum[0]).max(0.0) * (self.maximum[1] - self.minimum[1]).max(0.0)
    }
}

/// Faces closer than this count as touching. BSP splits land on shared edges.
const WATER_TOUCH_UNITS: f32 = 1.0;

fn faces_touch(a: &WaterFace, b: &WaterFace) -> bool {
    a.authored_ocean == b.authored_ocean
        && a.material == b.material
        && (a.plane - b.plane).abs() < WATER_TOUCH_UNITS
        && a.minimum[0] <= b.maximum[0] + WATER_TOUCH_UNITS
        && b.minimum[0] <= a.maximum[0] + WATER_TOUCH_UNITS
        && a.minimum[1] <= b.maximum[1] + WATER_TOUCH_UNITS
        && b.minimum[1] <= a.maximum[1] + WATER_TOUCH_UNITS
}

/// Merges faces that share a plane and touch or overlap, then orders the result
/// biggest first so the surface cap keeps the water that matters most.
///
/// Connectivity is pairwise between faces, never between cluster bounds, so two
/// pools that merely sit inside each other's bounding box stay separate. A
/// cluster's footprint is still the bounding box of its faces; a hole inside
/// contiguous water is therefore not preserved.
pub fn cluster_water_faces(faces: &[WaterFace]) -> Vec<WaterCluster> {
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..faces.len() {
        for j in i + 1..faces.len() {
            if faces_touch(&faces[i], &faces[j]) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                if a != b {
                    parent[b.max(a)] = b.min(a);
                }
            }
        }
    }
    let mut clusters: Vec<WaterCluster> = Vec::new();
    let mut slot_of_root = vec![usize::MAX; faces.len()];
    for (index, face) in faces.iter().enumerate() {
        let r = root(&mut parent, index);
        if slot_of_root[r] == usize::MAX {
            slot_of_root[r] = clusters.len();
            clusters.push(WaterCluster {
                plane: face.plane,
                minimum: face.minimum,
                maximum: face.maximum,
                members: Vec::new(),
            });
        }
        let cluster = &mut clusters[slot_of_root[r]];
        cluster.plane = cluster.plane.max(face.plane);
        for axis in 0..2 {
            cluster.minimum[axis] = cluster.minimum[axis].min(face.minimum[axis]);
            cluster.maximum[axis] = cluster.maximum[axis].max(face.maximum[axis]);
        }
        cluster.members.push(index);
    }
    clusters.sort_by(|a, b| b.area().total_cmp(&a.area()));
    clusters
}

fn point_in_triangle(p: [f32; 2], t: WaterTriangle) -> bool {
    let edge = |a: [f32; 2], b: [f32; 2]| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let e = [edge(t[0], t[1]), edge(t[1], t[2]), edge(t[2], t[0])];
    e.iter().all(|&v| v >= -WATER_TOUCH_UNITS)
        || e.iter().all(|&v| v <= WATER_TOUCH_UNITS)
}

fn point_segment_distance_sq(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length_sq = ab[0] * ab[0] + ab[1] * ab[1];
    if length_sq <= 1.0e-12 {
        return (p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2);
    }
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length_sq)
        .clamp(0.0, 1.0);
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
}

fn triangles_touch(a: WaterTriangle, b: WaterTriangle) -> bool {
    if a.iter().any(|&p| point_in_triangle(p, b))
        || b.iter().any(|&p| point_in_triangle(p, a))
    {
        return true;
    }
    let tolerance_sq = WATER_TOUCH_UNITS * WATER_TOUCH_UNITS;
    let segments_cross = |a0: [f32; 2], a1: [f32; 2], b0: [f32; 2], b1: [f32; 2]| {
        let orient = |p: [f32; 2], q: [f32; 2], r: [f32; 2]| {
            (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
        };
        let (a_b0, a_b1) = (orient(a0, a1, b0), orient(a0, a1, b1));
        let (b_a0, b_a1) = (orient(b0, b1, a0), orient(b0, b1, a1));
        (a_b0 <= 0.0 && a_b1 >= 0.0 || a_b1 <= 0.0 && a_b0 >= 0.0)
            && (b_a0 <= 0.0 && b_a1 >= 0.0 || b_a1 <= 0.0 && b_a0 >= 0.0)
    };
    (0..3).any(|i| {
        let (a0, a1) = (a[i], a[(i + 1) % 3]);
        (0..3).any(|j| {
            let (b0, b1) = (b[j], b[(j + 1) % 3]);
            segments_cross(a0, a1, b0, b1)
                || point_segment_distance_sq(a0, b0, b1) <= tolerance_sq
                || point_segment_distance_sq(a1, b0, b1) <= tolerance_sq
                || point_segment_distance_sq(b0, a0, a1) <= tolerance_sq
                || point_segment_distance_sq(b1, a0, a1) <= tolerance_sq
        })
    })
}

fn triangle_intersects_rect(t: WaterTriangle, lo: [f32; 2], hi: [f32; 2]) -> bool {
    // Separating-axis test: rectangle axes plus the normals of all triangle edges.
    let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
    let half = [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5];
    let separated = |axis: [f32; 2]| {
        let projected = t.map(|p| (p[0] - center[0]) * axis[0] + (p[1] - center[1]) * axis[1]);
        let tri_min = projected.into_iter().fold(f32::INFINITY, f32::min);
        let tri_max = projected.into_iter().fold(f32::NEG_INFINITY, f32::max);
        let radius = half[0] * axis[0].abs() + half[1] * axis[1].abs();
        tri_min > radius + WATER_TOUCH_UNITS || tri_max < -radius - WATER_TOUCH_UNITS
    };
    if separated([1.0, 0.0]) || separated([0.0, 1.0]) {
        return false;
    }
    !(0..3).any(|i| {
        let edge = [t[(i + 1) % 3][0] - t[i][0], t[(i + 1) % 3][1] - t[i][1]];
        separated([-edge[1], edge[0]])
    })
}

fn exact_faces_touch(
    a: usize,
    b: usize,
    faces: &[WaterFace],
    triangles: &[Vec<WaterTriangle>],
) -> bool {
    let fa = &faces[a];
    let fb = &faces[b];
    if !faces_touch(fa, fb) {
        return false;
    }
    triangles[a]
        .iter()
        .any(|&ta| triangles[b].iter().any(|&tb| triangles_touch(ta, tb)))
}

/// Builds connected water bodies directly from their real surface triangles.
/// `tile_size` is a world-space broad-phase size and must be finite/positive.
/// No pool-wide rectangle is rasterized while discovering the occupied tiles.
pub fn build_water_footprints(
    faces: &[WaterFace],
    triangles: &[Vec<WaterTriangle>],
    tile_size: f32,
) -> Vec<WaterFootprint> {
    assert_eq!(faces.len(), triangles.len());
    assert!(tile_size.is_finite() && tile_size > 0.0);
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..faces.len() {
        for j in i + 1..faces.len() {
            if exact_faces_touch(i, j, faces, triangles) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                if a != b {
                    parent[b.max(a)] = b.min(a);
                }
            }
        }
    }

    let mut grouped = BTreeMap::<usize, Vec<usize>>::new();
    for i in 0..faces.len() {
        grouped.entry(root(&mut parent, i)).or_default().push(i);
    }
    let mut result = Vec::with_capacity(grouped.len());
    for members in grouped.into_values() {
        let mut exact = Vec::new();
        for &member in &members {
            exact.extend_from_slice(&triangles[member]);
        }
        if exact.is_empty() {
            continue;
        }
        let minimum = [0, 1].map(|axis| {
            exact.iter().flatten().map(|p| p[axis]).fold(f32::INFINITY, f32::min)
        });
        let maximum = [0, 1].map(|axis| {
            exact.iter().flatten().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max)
        });
        let mut occupied = BTreeMap::<[i32; 2], Vec<u32>>::new();
        for (index, &triangle) in exact.iter().enumerate() {
            let lo = [0, 1].map(|axis| {
                (triangle.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min) / tile_size)
                    .floor() as i32
            });
            let hi = [0, 1].map(|axis| {
                let maximum = triangle
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max);
                (((maximum - tile_size * 1.0e-6) / tile_size).floor() as i32).max(lo[axis])
            });
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let tile_lo = [x as f32 * tile_size, y as f32 * tile_size];
                    let tile_hi = [tile_lo[0] + tile_size, tile_lo[1] + tile_size];
                    if triangle_intersects_rect(triangle, tile_lo, tile_hi) {
                        occupied.entry([x, y]).or_default().push(index as u32);
                    }
                }
            }
        }
        let tiles = occupied
            .into_iter()
            .map(|(coordinate, triangles)| {
                let minimum = [coordinate[0] as f32 * tile_size, coordinate[1] as f32 * tile_size];
                WaterFootprintTile {
                    coordinate,
                    minimum,
                    maximum: [minimum[0] + tile_size, minimum[1] + tile_size],
                    triangles,
                }
            })
            .collect();
        let plane = members.iter().map(|&i| faces[i].plane).fold(f32::NEG_INFINITY, f32::max);
        result.push(WaterFootprint {
            plane,
            minimum,
            maximum,
            members,
            triangles: exact,
            tile_size,
            tiles,
        });
    }
    result.sort_by(|a, b| b.area().total_cmp(&a.area()));
    result
}

#[cfg(test)]
mod footprint_tests {
    use super::*;

    fn face(minimum: [f32; 2], maximum: [f32; 2]) -> WaterFace {
        WaterFace {
            plane: 0.0,
            minimum,
            maximum,
            authored_ocean: None,
            material: 7,
        }
    }

    fn rectangle(lo: [f32; 2], hi: [f32; 2]) -> Vec<WaterTriangle> {
        vec![
            [lo, [hi[0], lo[1]], [lo[0], hi[1]]],
            [[hi[0], lo[1]], hi, [lo[0], hi[1]]],
        ]
    }

    #[test]
    fn l_shape_is_one_footprint_without_allocating_its_empty_corner() {
        let faces = [face([0.0, 0.0], [256.0, 64.0]), face([0.0, 64.0], [64.0, 256.0])];
        let triangles = [rectangle([0.0, 0.0], [256.0, 64.0]), rectangle([0.0, 64.0], [64.0, 256.0])];
        let footprints = build_water_footprints(&faces, &triangles, 64.0);
        assert_eq!(footprints.len(), 1);
        assert!(footprints[0].tiles.iter().any(|t| t.coordinate == [0, 3]));
        assert!(!footprints[0].tiles.iter().any(|t| t.coordinate == [3, 3]));
    }

    #[test]
    fn overlapping_face_boxes_do_not_join_separated_triangles() {
        let faces = [face([0.0, 0.0], [100.0, 100.0]), face([0.0, 0.0], [100.0, 100.0])];
        let triangles = [
            vec![[[0.0, 0.0], [20.0, 0.0], [0.0, 20.0]]],
            vec![[[100.0, 100.0], [80.0, 100.0], [100.0, 80.0]]],
        ];
        assert_eq!(build_water_footprints(&faces, &triangles, 32.0).len(), 2);
    }
}

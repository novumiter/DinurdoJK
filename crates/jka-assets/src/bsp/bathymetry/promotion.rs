//! Shared legacy promoted-water clustering. Geometry baking refines connectivity
//! using exact footprint triangles and solid boundaries; draw grouping stays intact.
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

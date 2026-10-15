//! Geometry.
use crate::scene::{jka_position, Arc, BTreeMap, DrawBatch, DrawClass, GpuVertex, Vec3};

#[derive(Default)]
pub(in crate::scene) struct Geometry {
    /// Exact PVS pieces inside one coarse material/lightmap batch, keyed by
    /// camera-cluster visibility and area membership. Pieces are laid out
    /// contiguously so FULL and AUTO 4 reference subranges of the coarse range
    /// without duplicating world vertices.
    pub(in crate::scene) by_pvs_signature: BTreeMap<(Vec<u64>, [u64; 4]), WorldGeometryChunk>,
}

#[derive(Default)]
pub(in crate::scene) struct WorldGeometryChunk {
    pub(in crate::scene) patches: Vec<super::PreparedPatchDraw>,
    pub(in crate::scene) vertices: Vec<GpuVertex>,
    pub(in crate::scene) surface_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::scene) struct GrassPatchKey {
    pub(in crate::scene) cell_x: i32,
    pub(in crate::scene) cell_z: i32,
    pub(in crate::scene) pvs_signature: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::scene) struct GrassWorkerPatchKey {
    pub(in crate::scene) cell_x: i32,
    pub(in crate::scene) cell_z: i32,
    pub(in crate::scene) signature_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::scene) struct GroupKey {
    /// TaystJK orders whole shaders by authored sort then sorted shader index.
    /// Sky normally sorts at 2, opaque at 3, explicit late sorts afterward.
    /// Stage pipeline class is NOT a primary ordering dimension.
    pub(in crate::scene) sort_priority: i32,
    pub(in crate::scene) shader: usize,
    pub(in crate::scene) class: DrawClass,
    pub(in crate::scene) lightmap: Option<usize>,
    pub(in crate::scene) vertex_lit: bool,
    pub(in crate::scene) color_slot: u8,
    pub(in crate::scene) fog_num: i32,
    pub(in crate::scene) transparent_order: usize,
    pub(in crate::scene) planar_group: Option<PlanarGroupKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::scene) struct PlanarGroupKey {
    pub(in crate::scene) normal_x: i32,
    pub(in crate::scene) normal_y: i32,
    pub(in crate::scene) normal_z: i32,
    pub(in crate::scene) distance: i32,
}

/// Markable-surface index for a source-.map world, which has no BSP surface
/// table to read shader flags from: every opaque/mask render triangle that faces
/// up enough is treated like an `SF_FACE`, as the old renderer-side gather did.
pub(in crate::scene) fn mark_surfaces_from_batches(
    vertices: &[GpuVertex],
    batches: &[DrawBatch],
) -> Arc<jka_assets::bsp::MarkSurfaces> {
    let mut triangles = Vec::new();
    for batch in batches {
        if !matches!(batch.pipeline.class, DrawClass::Opaque | DrawClass::Mask) {
            continue;
        }
        let start = batch.vertices.start as usize;
        let end = (batch.vertices.end as usize).min(vertices.len());
        if start >= end {
            continue;
        }
        for triangle in vertices[start..end].chunks_exact(3) {
            let normal = Vec3::from_array(triangle[0].normal)
                + Vec3::from_array(triangle[1].normal)
                + Vec3::from_array(triangle[2].normal);
            let normal = normal.normalize_or_zero();
            if normal == Vec3::ZERO {
                continue;
            }
            triangles.push((
                [
                    triangle[0].position,
                    triangle[1].position,
                    triangle[2].position,
                ]
                .map(jka_position),
                jka_position(normal.to_array()),
            ));
        }
    }
    Arc::new(jka_assets::bsp::MarkSurfaces::from_world_triangles(
        triangles,
    ))
}


/// Pack an independent pair of BSP triangles representing a rectangular
/// autoSprite2 quad. TaystJK's Autosprite2Deform finds its two shortest,
/// opposite edges, leaves their midpoints fixed and turns the short edges
/// around the shared long axis using cross(major, view_forward).
///
/// The BSP world already uses a non-indexed vertex stream. Encoding each
/// vertex with its short-edge midpoint, shared long axis, and signed half
/// width costs no extra world vertex bytes or per-frame CPU geometry updates.
/// These attributes live in position, normal, and unused lightmap_uv.x ONLY
/// for shaders that explicitly request autoSprite2 and have no lightmap.
///
/// If the pair is not an independent quad, leave it unchanged: the vertex
/// shader checks lightmap_uv.y for the marker before interpreting the payload.
pub(in crate::scene) fn encode_autosprite2_quad(vertices: &mut [GpuVertex]) -> bool {
    if vertices.len() != 6 { return false; }
    // Quake 3 draws two indexed triangles using four shared positions.
    let mut points = [[0.0_f32; 3]; 4];
    let mut which = [0_usize; 6];
    let mut count = 0;
    for (i, vertex) in vertices.iter().enumerate() {
        let p = vertex.position;
        let n = (0..count).find(|&n| points[n] == p);
        which[i] = if let Some(n) = n { n } else {
            if count == 4 { return false; }
            points[count] = p;
            count += 1;
            count - 1
        };
    }
    if count != 4 { return false; }
    let edges = [(0,1), (0,2), (0,3), (1,2), (1,3), (2,3)];
    let dist2 = |a: usize, b: usize| -> f32 {
        (0..3).map(|k| (points[a][k] - points[b][k]).powi(2)).sum()
    };
    let mut candidates = edges.map(|(a,b)| (dist2(a,b), a, b));
    candidates.sort_by(|l,r| l.0.total_cmp(&r.0));
    let (_, a, b) = candidates[0];
    // The two narrow edges must be opposite (unlike a triangle fan or a patch).
    let Some(&(_, c, d)) = candidates.iter().find(|(_,c,d)|
        *c != a && *c != b && *d != a && *d != b
    ) else { return false; };
    let width2 = dist2(a,b);
    let width2_other = dist2(c,d);
    if width2 < 1.0e-8 || width2_other < 1.0e-8 { return false; }
    let mids: [[f32; 3]; 2] = [
        std::array::from_fn(|k| (points[a][k] + points[b][k]) * 0.5),
        std::array::from_fn(|k| (points[c][k] + points[d][k]) * 0.5),
    ];
    let major: [f32;3] = std::array::from_fn(|k| mids[1][k] - mids[0][k]);
    let axis_len2: f32 = major.iter().map(|v| v*v).sum();
    if axis_len2 <= width2.min(width2_other) * 0.25 { return false; }
    let axis_len = axis_len2.sqrt();
    let major = major.map(|v| v / axis_len);
    // The minor-edge orientation follows the two triangles' winding, matching
    // TaystJK's lookup into tess.indexes when it decides +/- half the width.
    let oriented = |a: usize, b: usize| -> bool {
        which.chunks_exact(3).any(|t| (0..3).any(|i|
            t[i] == a && t[(i+1)%3] == b
        ))
    };
    let sign_ab = if oriented(a,b) { -1.0 } else { 1.0 };
    // The second narrow edge runs in the opposite winding direction on a
    // normal quad. Preserve the SAME physical side of the billboard at both
    // ends of the long axis; applying tess-index winding separately to both
    // edges can invert the top, folding the two triangles over each other.
    let sign_cd = if dist2(a,c) <= dist2(a,d) { sign_ab } else { -sign_ab };
    let mut midpoint = [[0.0_f32;3];4];
    let mut offset = [0.0_f32;4];
    for (endpoints, center, halfwidth, sign) in [
        ((a,b), mids[0], width2.sqrt() * 0.5, sign_ab),
        ((c,d), mids[1], width2_other.sqrt() * 0.5, sign_cd),
    ] {
        midpoint[endpoints.0] = center;
        midpoint[endpoints.1] = center;
        offset[endpoints.0] = halfwidth * sign;
        offset[endpoints.1] = -halfwidth * sign;
    }
    for (i, vertex) in vertices.iter_mut().enumerate() {
        let index = which[i];
        vertex.position = midpoint[index];
        vertex.normal = major;
        vertex.lightmap_uv[0] = offset[index];
        vertex.lightmap_uv[1] = -1337.0; // packed autoSprite2 marker
    }
    true
}

#[cfg(test)]
mod autosprite2_tests {
    use super::*;
    #[test]
    fn rectangular_quad_keeps_its_long_edge_midpoints() {
        let vertex = |position| GpuVertex {
            position, uv: [0.0,0.0], lightmap_uv: [0.0,0.0],
            normal: [0.0,0.0,1.0], color: [1.0;4], alpha_cutoff: 1.0,
        };
        // 2-wide / 12-high upright tree with two triangles and 4 shared corners.
        let mut vertices = [
            vertex([-1.0,0.0,0.0]), vertex([1.0,0.0,0.0]), vertex([1.0,12.0,0.0]),
            vertex([-1.0,0.0,0.0]), vertex([1.0,12.0,0.0]), vertex([-1.0,12.0,0.0]),
        ];
        assert!(encode_autosprite2_quad(&mut vertices));
        for v in &vertices {
            assert_eq!(v.position[0], 0.0);
            assert_eq!(v.normal[0], 0.0);
            assert_eq!(v.normal[1].abs(), 1.0);
            assert_eq!(v.lightmap_uv[0].abs(), 1.0);
        }
        assert_eq!(vertices[0].position, vertices[1].position);
        assert_eq!(vertices[2].position, vertices[4].position);
        assert_eq!(vertices[5].position, vertices[2].position);
        assert_eq!(vertices[0].lightmap_uv[0], -vertices[1].lightmap_uv[0]);
        assert_eq!(vertices[1].lightmap_uv[0], vertices[2].lightmap_uv[0]);
        assert_eq!(vertices[0].lightmap_uv[0], vertices[5].lightmap_uv[0]);
    }

    #[test]
    fn arbitrary_triangle_pair_is_not_reinterpreted_as_a_quad() {
        let v = |position| GpuVertex { position, uv: [0.0;2],
            lightmap_uv: [0.0;2], normal: [0.0,1.0,0.0],
            color: [1.0;4], alpha_cutoff: 1.0 };
        let mut vertices = [v([0.,0.,0.]), v([1.,0.,0.]), v([0.,1.,0.]),
            v([2.,0.,0.]), v([3.,0.,0.]), v([2.,1.,0.])];
        assert!(!encode_autosprite2_quad(&mut vertices));
    }
}

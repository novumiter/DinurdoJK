//! Shared promotion geometry; map baking and GPU upload use the same selection.
use crate::scene::{DrawBatch, GpuVertex};

pub struct PromotedFace<'a> {
    pub source: &'a DrawBatch,
    pub key: [u32; 6],
    pub face: super::WaterFace,
    pub triangles: Vec<super::MaskTriangle>,
}

pub fn collect<'a>(
    vertices: &[GpuVertex],
    coarse: &'a [DrawBatch],
    full: &'a [DrawBatch],
) -> Vec<PromotedFace<'a>> {
    let mut seen = std::collections::BTreeSet::new();
    coarse
        .iter()
        .chain(full)
        .filter_map(|source| {
            if !source.water_primary || !is_upward_water_face(source, vertices) {
                return None;
            }
            let (plane, minimum, maximum) = water_surface_extent(source, vertices);
            let key = ocean_surface_key(plane, minimum, maximum, source.bsp_shader_index);
            if !seen.insert(key) {
                return None;
            }
            let slice = &vertices[source.vertices.start as usize..source.vertices.end as usize];
            let triangles = slice
                .chunks_exact(3)
                .map(|t| [0, 1, 2].map(|i| [t[i].position[0], t[i].position[2]]))
                .collect();
            Some(PromotedFace {
                source,
                key,
                face: super::WaterFace {
                    plane,
                    minimum,
                    maximum,
                    authored_ocean: source.authored_ocean,
                    material: source.bsp_shader_index,
                },
                triangles,
            })
        })
        .collect()
}

/// Plane height and world-space xz footprint of a promoted water face.
pub fn water_surface_extent(
    source: &DrawBatch,
    vertices: &[GpuVertex],
) -> (f32, [f32; 2], [f32; 2]) {
    let slice = &vertices[source.vertices.start as usize..source.vertices.end as usize];
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for vertex in slice {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(vertex.position[axis]);
            maximum[axis] = maximum[axis].max(vertex.position[axis]);
        }
    }
    (
        maximum[1],
        [minimum[0], minimum[2]],
        [maximum[0], maximum[2]],
    )
}

pub fn ocean_surface_key(
    plane: f32,
    minimum: [f32; 2],
    maximum: [f32; 2],
    material: u32,
) -> [u32; 6] {
    [
        plane.to_bits(),
        minimum[0].to_bits(),
        minimum[1].to_bits(),
        maximum[0].to_bits(),
        maximum[1].to_bits(),
        material,
    ]
}

/// A `surfaceparm water` brush contributes every face it has, so its underside
/// and its sides arrive looking exactly like its top. Only the upward-facing
/// horizontal face is the water surface the FFT ocean should replace.
pub fn is_upward_water_face(source: &DrawBatch, vertices: &[GpuVertex]) -> bool {
    let slice = &vertices[source.vertices.start as usize..source.vertices.end as usize];
    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    let mut facing = 0.0f32;
    for vertex in slice {
        lowest = lowest.min(vertex.position[1]);
        highest = highest.max(vertex.position[1]);
        facing += vertex.normal[1];
    }
    lowest.is_finite() && facing > 0.0 && lowest >= highest - 1.0
}

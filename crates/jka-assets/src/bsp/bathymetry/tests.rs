use super::*;

fn box_solid(min: [f32; 3], max: [f32; 3]) -> Solid {
    Solid {
        planes: vec![
            Plane {
                normal: [1., 0., 0.],
                distance: max[0],
            },
            Plane {
                normal: [-1., 0., 0.],
                distance: -min[0],
            },
            Plane {
                normal: [0., 1., 0.],
                distance: max[1],
            },
            Plane {
                normal: [0., -1., 0.],
                distance: -min[1],
            },
            Plane {
                normal: [0., 0., 1.],
                distance: max[2],
            },
            Plane {
                normal: [0., 0., -1.],
                distance: -min[2],
            },
        ],
        sky: vec![false; 6],
        bounds: [[min[0], min[1]], [max[0], max[1]]],
    }
}
fn rectangle(min: [f32; 2], max: [f32; 2]) -> Vec<FootprintTriangle> {
    vec![[min, [max[0], min[1]], max], [min, max, [min[0], max[1]]]]
}
fn region() -> Region {
    Region {
        slot: 2,
        level: 0.0,
        triangles: rectangle([0., 0.], [64., 64.]),
        bounds: None,
    }
}
fn basin() -> Geometry {
    Geometry {
        solids: vec![box_solid([-64., -64., -128.], [128., 128., -16.])],
        triangles: vec![],
        liquids: vec![],
    }
}
fn bake(g: Geometry, r: Region) -> Grid {
    g.bake(&[r], Options::default()).unwrap().grids.remove(0)
}
fn cell(g: &Grid, p: [f32; 2]) -> Cell {
    let x = ((p[0] - g.origin[0]) / g.cell_size) as usize;
    let y = ((p[1] - g.origin[1]) / g.cell_size) as usize;
    g.cells[y * g.size[0] as usize + x]
}

#[test]
fn analytic_basin_has_real_depth_and_one_component() {
    let grid = bake(basin(), region());
    let c = cell(&grid, [24., 24.]);
    assert_eq!(c.class, Class::Wet);
    assert_eq!(c.depth, 16.);
    assert_eq!(c.bed, -16.);
    assert_eq!(grid.components, 1);
    assert!(grid
        .cells
        .iter()
        .filter(|c| c.class == Class::Wet)
        .all(|c| c.depth == 16.));
}

#[test]
fn water_and_clip_brushes_are_never_seabed() {
    use crate::bsp::{Brush, BrushSide, Model, Shader};
    for contents in [CONTENTS_WATER, CONTENTS_WATER | CONTENTS_SOLID, 0x10] {
        let solid = box_solid([-100., -100., -128.], [100., 100., -16.]);
        let bsp = Bsp {
            collision: None,
            visibility: None,
            shaders: vec![Shader {
                name: b"water".to_vec(),
                surface_flags: 0,
                contents,
            }],
            fogs: vec![],
            vertices: vec![],
            indices: vec![],
            surfaces: vec![],
            models: vec![Model {
                mins: [-100., -100., -128.],
                maxs: [100., 100., 0.],
                surfaces: 0..0,
                brushes: 0..1,
            }],
            planes: solid.planes,
            brushes: vec![Brush {
                sides: 0..6,
                shader: 0,
            }],
            brush_sides: (0..6).map(|plane| BrushSide { plane, shader: 0 }).collect(),
            entities: vec![],
            lightmaps: vec![],
            light_grid: None,
        };
        let geometry = Geometry::from_bsp(&bsp).unwrap();
        let grid = bake(geometry, region());
        let c = cell(&grid, [24., 24.]);
        assert_eq!(c.class, Class::NoFloor);
        assert!(c.bed.is_nan());
        assert!(c.depth.is_nan());
    }
}

#[test]
fn sloping_convex_ground_uses_plane_intersection() {
    let mut g = basin();
    g.solids[0].planes[4] = Plane {
        normal: [-0.25, 0., 1.],
        distance: -16.,
    };
    g.solids[0].bounds = [[-64., -64.], [128., 128.]];
    let grid = bake(g, region());
    let a = cell(&grid, [8., 24.]);
    let b = cell(&grid, [40., 24.]);
    assert_eq!(a.bed, -14.);
    assert_eq!(b.bed, -6.);
    assert_eq!(a.depth, 14.);
    assert_eq!(b.depth, 6.);
}

#[test]
fn subcell_wall_splits_wet_connectivity_in_deep_water() {
    let mut g = basin();
    g.solids
        .push(box_solid([31.99, -64., -128.], [32.01, 128., 64.]));
    let grid = bake(g, region());
    // Wall sits between all centers and inset corner probes.
    assert_eq!(cell(&grid, [24., 24.]).class, Class::Wet);
    assert_eq!(cell(&grid, [40., 24.]).class, Class::Wet);
    assert_ne!(
        cell(&grid, [24., 24.]).component,
        cell(&grid, [40., 24.]).component
    );
    assert_eq!(grid.components, 2);
    let x = ((32. - grid.origin[0]) / grid.cell_size) as usize;
    let y = ((24. - grid.origin[1]) / grid.cell_size) as usize;
    assert_eq!(
        grid.faces_x[y * (grid.size[0] as usize + 1) + x].boundary,
        Boundary::Wall
    );
}

#[test]
fn sloped_waterline_is_an_overtoppable_bank_not_a_wall() {
    let mut g = basin();
    g.solids[0].planes[4] = Plane {
        normal: [-1.0, 0.0, 1.0],
        distance: -16.0,
    };
    let grid = bake(g, region());
    assert_eq!(cell(&grid, [8.0, 24.0]).class, Class::Wet);
    assert_eq!(cell(&grid, [24.0, 24.0]).class, Class::Dry);
    let x = ((16.0 - grid.origin[0]) / grid.cell_size) as usize;
    let y = ((24.0 - grid.origin[1]) / grid.cell_size) as usize;
    assert_eq!(
        grid.faces_x[y * (grid.size[0] as usize + 1) + x].boundary,
        Boundary::Bank
    );
}

#[test]
fn exact_triangle_union_does_not_bridge_a_narrow_gap() {
    let mut triangles = rectangle([0., 0.], [29., 64.]);
    triangles.extend(rectangle([30., 0.], [64., 64.]));
    assert!(!segment_covered(
        [24., 24.],
        [40., 24.],
        triangles.iter().copied()
    ));
    let grid = bake(
        basin(),
        Region {
            triangles,
            ..region()
        },
    );
    assert_ne!(
        cell(&grid, [24., 24.]).component,
        cell(&grid, [40., 24.]).component
    );
}

#[test]
fn footprint_hole_and_separate_heights_are_preserved() {
    let mut triangles = rectangle([0., 0.], [16., 64.]);
    triangles.extend(rectangle([48., 0.], [64., 64.]));
    let grid = bake(
        basin(),
        Region {
            triangles,
            ..region()
        },
    );
    assert_eq!(cell(&grid, [24., 24.]).class, Class::Outside);
    let g = basin();
    let mut high = region();
    high.slot = 3;
    high.level = 8.;
    let atlas = g.bake(&[region(), high], Options::default()).unwrap();
    assert_eq!(cell(&atlas.grids[0], [24., 24.]).depth, 16.);
    assert_eq!(cell(&atlas.grids[1], [24., 24.]).depth, 24.);
}

#[test]
fn ceilings_are_retained_and_submerged_stacked_floors_get_fallback() {
    for (bottom, top) in [(4., 8.), (-8., -4.)] {
        let mut g = basin();
        g.solids
            .push(box_solid([-64., -64., bottom], [128., 128., top]));
        let grid = bake(g, region());
        let c = cell(&grid, [24., 24.]);
        if bottom > 0. {
            assert_eq!(c.class, Class::Wet);
            assert_eq!(c.bed, -16.);
            assert_eq!(c.ceiling, 4.);
        } else {
            assert_eq!(c.class, Class::Overhang);
            assert_eq!(c.component, 0);
        }
    }
}

#[test]
fn cache_checks_identity_integrity_and_roundtrips_unknown_depth() {
    let atlas = Geometry {
        solids: vec![],
        triangles: vec![],
        liquids: vec![],
    }
    .bake(&[region()], Options::default())
    .unwrap();
    let bytes = atlas.encode([123, 456]);
    let decoded = Atlas::decode(&bytes, [123, 456]).unwrap();
    assert!(cell(&decoded.grids[0], [24., 24.]).depth.is_nan());
    assert_eq!(decoded.grids[0].footprint, region().triangles);
    assert!(Atlas::decode(&bytes, [123, 457]).is_err());
    let mut corrupt = bytes.clone();
    corrupt[60] ^= 1;
    assert!(Atlas::decode(&corrupt, [123, 456]).is_err());
    assert!(Atlas::decode(&bytes[..bytes.len() / 2], [123, 456]).is_err());
}

#[test]
fn water_volume_selects_layer_without_becoming_a_depth_limit() {
    let mut g = basin();
    g.solids
        .push(box_solid([-64., -64., -256.], [128., 128., -192.]));
    // The nominal water brush bottom sits ABOVE the real seabed. Depth must
    // still come from solid geometry, and the lower room is another layer.
    g.liquids
        .push(box_solid([-64., -64., -8.], [128., 128., 0.]));
    let grid = bake(g, region());
    let c = cell(&grid, [24., 24.]);
    assert_eq!(c.class, Class::Wet);
    assert_eq!(c.depth, 16.);
}

#[test]
fn nodraw_terrain_patch_is_bed_and_triangle_wall_seals_the_face() {
    use crate::bsp::{Model, Shader, Surface, Vertex, SURF_NODRAW};
    let vertices = (0..3)
        .flat_map(|y| {
            (0..3).map(move |x| Vertex {
                position: [x as f32 * 32., y as f32 * 32., -10.],
                normal: [0., 0., 1.],
                texcoord: [0.; 2],
                lightmap_uv: [[0.; 2]; 4],
                color: [[255; 4]; 4],
            })
        })
        .collect();
    let bsp = Bsp {
        collision: None,
        visibility: None,
        shaders: vec![Shader {
            name: b"terrain".to_vec(),
            surface_flags: SURF_NODRAW,
            contents: CONTENTS_TERRAIN,
        }],
        fogs: vec![],
        vertices,
        indices: vec![],
        surfaces: vec![Surface {
            shader: 0,
            fog_num: -1,
            kind: SurfaceKind::Patch,
            flare: None,
            vertices: 0..9,
            indices: 0..0,
            lightmaps: [-1; 4],
            lightmap_styles: [0; 4],
            vertex_styles: [0; 4],
            patch_size: [3, 3],
        }],
        models: vec![Model {
            mins: [0., 0., -10.],
            maxs: [64., 64., 0.],
            surfaces: 0..1,
            brushes: 0..0,
        }],
        planes: vec![],
        brushes: vec![],
        brush_sides: vec![],
        entities: vec![],
        lightmaps: vec![],
        light_grid: None,
    };
    let mut geometry = Geometry::from_bsp(&bsp).unwrap();
    let grid = geometry
        .bake(&[region()], Options::default())
        .unwrap()
        .grids
        .remove(0);
    assert_eq!(cell(&grid, [24., 24.]).bed, -10.);
    let a = [32., -64., -64.];
    let b = [32., 128., -64.];
    let c = [32., 128., 64.];
    let d = [32., -64., 64.];
    for points in [[a, b, c], [a, c, d]] {
        geometry.triangles.push(Triangle {
            points,
            normal: [1., 0., 0.],
            bounds: bounds(points.iter().map(|p| [p[0], p[1]])),
        });
    }
    let grid = bake(geometry, region());
    assert_ne!(
        cell(&grid, [24., 24.]).component,
        cell(&grid, [40., 24.]).component
    );
}

#[test]
fn oversized_grid_is_rejected_without_coarsening_or_guessing_depth() {
    let r = Region {
        triangles: rectangle([0., 0.], [100000., 100000.]),
        ..region()
    };
    let atlas = basin().bake(&[r], Options::default()).unwrap();
    assert!(atlas.grids.is_empty());
    assert_eq!(atlas.warnings.len(), 1);
    assert!(atlas.warnings[0].contains("retain legacy water"));
}

#[test]
fn sparse_l_tiles_skip_the_empty_envelope_corner_and_stitch_open_faces() {
    let mut exact = rectangle([0.0, 0.0], [512.0, 256.0]);
    exact.extend(rectangle([0.0, 256.0], [256.0, 512.0]));
    let tile = |bounds: [[f32; 2]; 2]| Region {
        slot: 4,
        level: 0.0,
        triangles: exact.clone(),
        bounds: Some(bounds),
    };
    let geometry = Geometry {
        solids: vec![box_solid([-64.0, -64.0, -128.0], [576.0, 576.0, -16.0])],
        triangles: vec![],
        liquids: vec![],
    };
    let atlas = geometry
        .bake(
            &[
                tile([[0.0, 0.0], [256.0, 256.0]]),
                tile([[0.0, 256.0], [256.0, 512.0]]),
                tile([[256.0, 0.0], [512.0, 256.0]]),
            ],
            Options::default(),
        )
        .unwrap();
    assert_eq!(atlas.grids.len(), 3);
    assert_eq!(atlas.grids.iter().map(|g| g.cells.len()).sum::<usize>(), 768);
    let lower_left = atlas.grids.iter().find(|g| g.origin == [0.0, 0.0]).unwrap();
    let nx = lower_left.size[0] as usize;
    let ny = lower_left.size[1] as usize;
    assert_eq!(lower_left.faces_x[8 * (nx + 1) + nx].boundary, Boundary::Open);
    assert_eq!(lower_left.faces_y[ny * nx + 8].boundary, Boundary::Open);
}

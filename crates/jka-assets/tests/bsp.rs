use jka_assets::bsp::{Bsp, SurfaceKind};

fn put(data: &mut [u8], offset: usize, value: i32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn float(data: &mut [u8], offset: usize, value: f32) {
    put(data, offset, value.to_bits() as i32);
}

fn fixture(patch: bool) -> Vec<u8> {
    fixture_with_inline_models(patch, &[])
}

/// `inline` appends models `*1..` as (firstSurface, numSurfaces) records.
fn fixture_with_inline_models(patch: bool, inline: &[(i32, i32)]) -> Vec<u8> {
    let mut lumps: [Vec<u8>; 18] = std::array::from_fn(|_| Vec::new());
    lumps[0] = b"{\n\"classname\" \"worldspawn\"\n}\n// spawn\n{\"classname\" \"info_player_deathmatch\" \"origin\" \"1 2 3\" \"angle\" \"90\"}\0".to_vec();
    lumps[1] = vec![0; 72];
    lumps[1][..5].copy_from_slice(b"stone");
    put(&mut lumps[1], 68, 1);
    lumps[2] = vec![0; 16];
    float(&mut lumps[2], 8, 1.0);
    lumps[7] = vec![0; 40];
    for axis in 0..3 {
        float(&mut lumps[7], 12 + axis * 4, 10.0);
    }
    put(&mut lumps[7], 28, 1);
    put(&mut lumps[7], 36, 1);
    for &(first_surface, surface_count) in inline {
        let mut record = vec![0; 40];
        for axis in 0..3 {
            float(&mut record, 12 + axis * 4, 10.0);
        }
        put(&mut record, 24, first_surface);
        put(&mut record, 28, surface_count);
        lumps[7].extend(record);
    }
    lumps[8] = vec![0; 12];
    put(&mut lumps[8], 4, 1);
    lumps[9] = vec![0; 12];
    let count = if patch { 9 } else { 3 };
    lumps[10] = vec![0; count * 80];
    for i in 0..count {
        float(&mut lumps[10], i * 80, (i % 3) as f32);
        float(&mut lumps[10], i * 80 + 4, (i / 3) as f32);
        float(&mut lumps[10], i * 80 + 8, if i == 4 { 4.0 } else { 0.0 });
        float(&mut lumps[10], i * 80 + 60, 1.0);
        lumps[10][i * 80 + 64..i * 80 + 80].fill(255);
    }
    if !patch {
        lumps[11] = [0i32, 1, 2]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect();
    }
    lumps[13] = vec![0; 148];
    put(&mut lumps[13], 8, if patch { 2 } else { 1 });
    put(&mut lumps[13], 16, count as i32);
    put(&mut lumps[13], 24, if patch { 0 } else { 3 });
    for slot in 0..4 {
        put(&mut lumps[13], 36 + slot * 4, -1);
    }
    if patch {
        put(&mut lumps[13], 140, 3);
        put(&mut lumps[13], 144, 3);
    }
    let mut output = vec![0; 152];
    output[..4].copy_from_slice(b"RBSP");
    put(&mut output, 4, 1);
    for (i, lump) in lumps.into_iter().enumerate() {
        let offset = output.len() as i32;
        put(&mut output, 8 + i * 8, offset);
        put(&mut output, 12 + i * 8, lump.len() as i32);
        output.extend(lump);
    }
    output
}

fn offset(data: &[u8], lump: usize) -> usize {
    i32::from_le_bytes(data[8 + lump * 8..12 + lump * 8].try_into().unwrap()) as usize
}

#[test]
fn loads_world_geometry_collision_data_and_spawn_without_game_assets() {
    let map = Bsp::parse(&fixture(false)).unwrap();
    assert_eq!(map.shaders[0].name, b"stone");
    assert_eq!(map.brushes[0].sides, 0..1);
    assert_eq!(map.planes[0].normal, [0.0, 0.0, 1.0]);
    assert_eq!(map.surfaces[0].kind, SurfaceKind::Planar);
    assert_eq!(map.world_mesh(4).unwrap().indices, [0, 1, 2]);
    let spawn = map.deathmatch_spawns()[0];
    assert_eq!(spawn.origin, [1.0, 2.0, 3.0]);
    assert_eq!(spawn.yaw, 90.0);
}

#[test]
fn inline_models_tessellate_like_world_and_map_batches_to_model_numbers() {
    // *1 reuses the world's triangle, *2 has no surfaces (a trigger), *3
    // again has the triangle: the batch map must skip *2, not shift *3.
    let map = Bsp::parse(&fixture_with_inline_models(false, &[(0, 1), (0, 0), (0, 1)])).unwrap();
    assert_eq!(map.models.len(), 4);
    let world = map.world_mesh(4).unwrap();
    let (inline, batch_models) = map.inline_models_mesh(4).unwrap();
    assert_eq!(batch_models, [1, 3]);
    assert_eq!(inline.batches.len(), 2);
    assert_eq!(inline.indices, [0, 1, 2, 3, 4, 5]);
    assert_eq!(inline.vertices[3].position, world.vertices[0].position);
    assert_eq!(inline.batches[1].indices, 3..6);
    // The world mesh itself is unchanged by inline models.
    assert_eq!(world.indices, [0, 1, 2]);
}

#[test]
fn quadratic_patch_reaches_analytic_midpoint_and_preserves_attributes() {
    let map = Bsp::parse(&fixture(true)).unwrap();
    let mesh = map.world_mesh(2).unwrap();
    assert_eq!(mesh.vertices.len(), 9);
    assert_eq!(mesh.indices.len(), 24);
    // OpenJK r_subdivisions=4 semantics keep this 3x3 control patch as one
    // shared 3x3 grid. The old fixed n=4 path inflated it to 25 verts/96 indices.
    let default_mesh = map.world_mesh(4).unwrap();
    assert_eq!(default_mesh.vertices.len(), 9);
    assert_eq!(default_mesh.indices.len(), 24);
    // Center control is z=4 with Bernstein weight 1/4 at u=v=1/2.
    assert_eq!(mesh.vertices[4].position, [1.0, 1.0, 1.0]);
    assert_eq!(mesh.vertices[4].normal, [0.0, 0.0, 1.0]);
    assert_eq!(mesh.vertices[4].color, [[255; 4]; 4]);
    assert_eq!(mesh.vertices[0].position, [0.0, 0.0, 0.0]);
    assert_eq!(mesh.vertices[8].position, [2.0, 2.0, 0.0]);
    assert!(map.world_mesh(0).is_err());
    assert!(map.world_mesh(17).is_err());
}

#[test]
fn visual_patch_metadata_uses_authored_lod_bounds_and_keeps_fixed_mesh_stable() {
    let mut data = fixture(true);
    let surface = offset(&data, 13);
    for (i,value) in [-100.0,20.0,30.0,100.0,60.0,70.0].into_iter().enumerate() {
        float(&mut data,surface+104+i*4,value);
    }
    let map = Bsp::parse(&data).unwrap();
    let fixed_before = map.world_mesh(4).unwrap();
    for sub in [0.0,1.0,4.0,12.0,80.0] {for stitching in [false,true] {
        let visual = map.world_visual_mesh(sub,stitching).unwrap();
        assert_eq!(visual.patches.len(),1);
        let patch=&visual.patches[0];
        assert_eq!(patch.lod.lod_origin,[0.0,40.0,50.0]);
        assert!((patch.lod.lod_radius-10800.0_f32.sqrt()).abs()<0.0001);
        assert_eq!(patch.lod.width_lod_error.len(),patch.lod.width);
        assert_eq!(patch.lod.height_lod_error.len(),patch.lod.height);
        let fixed_after = map.world_mesh(4).unwrap();
        assert_eq!(fixed_before.indices,fixed_after.indices);
        assert!(fixed_before.vertices.iter().zip(&fixed_after.vertices).all(|(a,b)| a.position==b.position&&a.normal==b.normal&&a.texcoord==b.texcoord&&a.color==b.color));
    }}
    for sub in [f32::NAN,f32::INFINITY,-1.0,81.0] {assert!(map.world_visual_mesh(sub,true).is_err());}
    float(&mut data,surface+116,f32::NAN);
    assert!(Bsp::parse(&data).is_err(),"nonfinite authored LOD bounds must be rejected");
}

#[test]
fn rejects_truncated_files_at_every_byte_boundary() {
    let data = fixture(true);
    for end in 0..data.len() {
        assert!(Bsp::parse(&data[..end]).is_err(), "accepted prefix {end}");
    }
}

#[test]
fn rejects_bad_directory_and_incompatible_formats() {
    for (position, value) in [(4, 46), (8, -1), (12, i32::MAX), (8, 0), (16, 152)] {
        let mut data = fixture(false);
        put(&mut data, position, value);
        assert!(
            Bsp::parse(&data).is_err(),
            "accepted corrupt header at {position}"
        );
    }
}

#[test]
fn rejects_bad_indices_nonfinite_geometry_and_patch_dimensions() {
    for (lump, relative, value) in [
        (11, 0, 3),
        (13, 0, 1),
        (10, 0, f32::NAN.to_bits() as i32),
        (9, 0, 1),
        (7, 28, 2),
    ] {
        let mut data = fixture(false);
        let location = offset(&data, lump) + relative;
        put(&mut data, location, value);
        assert!(Bsp::parse(&data).is_err(), "accepted corrupt lump {lump}");
    }
    let mut data = fixture(true);
    let location = offset(&data, 13) + 140;
    put(&mut data, location, i32::MAX);
    assert!(Bsp::parse(&data).is_err());
}

#[test]
fn external_lightmap_indices_are_valid_but_embedded_indices_are_bounded() {
    let mut data = fixture(false);
    let lightmap_index = offset(&data, 13) + 36;
    put(&mut data, lightmap_index, 0);
    assert!(Bsp::parse(&data).is_ok());
    let page_offset = data.len() as i32;
    put(&mut data, 8 + 14 * 8, page_offset);
    put(&mut data, 12 + 14 * 8, 128 * 128 * 3);
    data.resize(data.len() + 128 * 128 * 3, 0);
    assert!(Bsp::parse(&data).is_ok());
    put(&mut data, lightmap_index, 1);
    assert!(Bsp::parse(&data).is_err());
}

#[test]
fn accepts_embedded_lightmaps_beyond_legacy_eight_mib_design_bound() {
    let mut data = fixture(false);
    // 171 RGB pages are just over OpenJK's historical MAX_MAP_LIGHTING (8 MiB).
    // Runtime RBSP loading is length-driven, so a whole-page lump remains valid.
    const PAGE_BYTES: usize = 128 * 128 * 3;
    const PAGE_COUNT: usize = 171;
    let page_offset = data.len() as i32;
    put(&mut data, 8 + 14 * 8, page_offset);
    put(&mut data, 12 + 14 * 8, (PAGE_BYTES * PAGE_COUNT) as i32);
    data.resize(data.len() + PAGE_BYTES * PAGE_COUNT, 0);

    let map = Bsp::parse(&data).unwrap();
    assert_eq!(map.lightmaps.len(), PAGE_BYTES * PAGE_COUNT);
}

#[test]
fn excludes_nodraw_surfaces_and_inline_models_from_world_mesh() {
    let mut data = fixture(false);
    let location = offset(&data, 1) + 64;
    put(&mut data, location, 0x0020_0000);
    assert!(Bsp::parse(&data)
        .unwrap()
        .world_mesh(4)
        .unwrap()
        .indices
        .is_empty());
    let mut data = fixture(false);
    let location = offset(&data, 7) + 28;
    put(&mut data, location, 0);
    assert!(Bsp::parse(&data)
        .unwrap()
        .world_mesh(4)
        .unwrap()
        .indices
        .is_empty());
}

#[test]
fn malformed_entity_text_is_rejected() {
    let mut data = fixture(false);
    let start = offset(&data, 0);
    data[start] = b'[';
    assert!(Bsp::parse(&data).is_err());
    let mut data = fixture(false);
    data[start + 2] = 0;
    assert!(Bsp::parse(&data).is_err());
}

#[test]
fn unused_lightmap_nan_is_tolerated_but_active_nan_is_rejected() {
    let mut data = fixture(false);
    let location = offset(&data, 10) + 20;
    float(&mut data, location, f32::NAN);
    let map = Bsp::parse(&data).unwrap();
    assert!(map.vertices[0].lightmap_uv[0][0].is_nan());
    assert_eq!(
        map.world_mesh(4).unwrap().vertices[0].lightmap_uv,
        [[0.0; 2]; 4]
    );
    let location = offset(&data, 13) + 36;
    put(&mut data, location, 0);
    let page_start = data.len() as i32;
    put(&mut data, 8 + 14 * 8, page_start);
    put(&mut data, 12 + 14 * 8, 128 * 128 * 3);
    data.resize(data.len() + 128 * 128 * 3, 0);
    assert!(Bsp::parse(&data)
        .unwrap_err()
        .to_string()
        .contains("active lightmap"));
}

/// RBSP flare payload is stored in lightmapOrigin/Vecs, not the vertex lump.
#[test]
fn retains_flare_semantics_without_draw_triangles() {
    let mut raw = fixture(false);
    let surface = offset(&raw, 13);
    put(&mut raw, surface + 8, 4); // MST_FLARE
    // No mesh triangles or vertices are needed by the flare surface.
    put(&mut raw, surface + 16, 0);
    put(&mut raw, surface + 24, 0);
    for (i, value) in [120.0, -8.0, 44.0].into_iter().enumerate() {
        float(&mut raw, surface + 92 + i * 4, value);
    }
    for (i, value) in [0.25, 0.5, 1.0].into_iter().enumerate() {
        float(&mut raw, surface + 104 + i * 4, value);
    }
    for (i, value) in [0.0, 0.0, -1.0].into_iter().enumerate() {
        float(&mut raw, surface + 128 + i * 4, value);
    }
    put(&mut raw, surface + 4, 7);
    let bsp = Bsp::parse(&raw).unwrap();
    let parsed = &bsp.surfaces[0];
    assert_eq!(parsed.kind, SurfaceKind::Flare);
    assert_eq!(parsed.fog_num, 7);
    assert_eq!(parsed.shader, 0);
    let flare = parsed.flare.unwrap();
    assert_eq!(flare.origin, [120.0, -8.0, 44.0]);
    assert_eq!(flare.color, [0.25, 0.5, 1.0]);
    assert_eq!(flare.normal, [0.0, 0.0, -1.0]);
    assert!(bsp.world_mesh(4).unwrap().indices.is_empty());
}

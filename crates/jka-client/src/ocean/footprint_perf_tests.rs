use super::*;
use std::cell::RefCell;
thread_local! {static CAPTURE:RefCell<Option<Vec<([f32;2],[f32;2])>>>=const {RefCell::new(None)};}
pub(super) fn record_query(slot:usize,lo:[f32;2],hi:[f32;2]) {
    if slot==0 {CAPTURE.with(|c|{if let Some(queries)=c.borrow_mut().as_mut() {queries.push((lo,hi));}});}
}
    use std::{hint::black_box,path::Path,time::Instant};

    #[test]
    #[ignore = "requires installed RaceArena assets; benchmarks the actual footprint with and without its spatial index"]
    fn racearena_footprint_hotpath_benchmark() {
        let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let map=crate::scene::prepare_with_options(&workspace.join("target/release/base"),
            Some(&workspace.join("target/release/japro")),"racearena_pro",crate::scene::MapPrepareOptions {
                ocean:true,grass:false,voxel_probe_gi:false,client_physics:false,
                gen_normal_maps:false,steam_audio:false,planar_reflections:false,planar_environment:false,
                ..Default::default()
            }).expect("prepare RaceArena");
        let entries=surfaces::collect(&map.vertices,&map.batches,&map.pvs_batches);
        let faces:Vec<_>=entries.iter().map(|e|e.face).collect();
        let triangles:Vec<_>=entries.iter().map(|e|e.triangles.clone()).collect();
        let footprints=build_water_footprints(&faces,&triangles,WATER_FOOTPRINT_TILE_SIZE);
        let mut indexed=OceanMasks::default();
        for (slot,footprint) in footprints.iter().take(OCEAN_MAX_SURFACES).enumerate() {
            if promoted_outline_needs_mask(footprint.minimum,footprint.maximum,&footprint.triangles) {
                indexed.push_footprint(slot,footprint);
            } else {indexed.set_footprint(slot,footprint);}
            eprintln!("RaceArena pool {}: {} triangles / {} tiles; shader mask={}",
                slot+1,footprint.triangles.len(),footprint.tiles.len(),indexed.is_masked(slot));
        }
        let mut linear=indexed.clone();
        linear.footprint_indices=std::array::from_fn(|_|None);
        let cameras=[
            [4927.71,10767.15,5165.46],[-1630.34,10771.37,13444.17],
            [-11753.18,10767.75,16052.35],[-15292.54,10761.42,5736.82],
            [-16112.91,10761.42,-7311.17],
        ];
        let closest=|masks:&OceanMasks| {
            for camera in cameras {for slot in 0..footprints.len().min(OCEAN_MAX_SURFACES) {
                black_box(masks.closest_point(slot,black_box([camera[0],camera[2]])));
                black_box(masks.closest_point(slot,black_box([(camera[0]/1280.0).round()*1280.0,(camera[2]/1280.0).round()*1280.0])));
            }}
        };
        let sample=|work:&dyn Fn(),repeats:usize| {
            let start=Instant::now();for _ in 0..repeats {work();}start.elapsed().as_secs_f64()*1000.0/repeats as f64
        };
        let old_nearest=sample(&||closest(&linear),30);
        let new_nearest=sample(&||closest(&indexed),30);
        let footprint=&footprints[0];
        let surface=OceanSurface {plane_height:footprint.plane,minimum:footprint.minimum,maximum:footprint.maximum};
        let runtime=build_runtime_promoted_clipmap(OCEAN_LOW_MESH_SPACING_UNITS,footprint.plane,footprint.minimum,footprint.maximum,
            indexed.is_masked(0).then_some(0),appearance::MIN_PROMOTED_AMPLITUDE);
        let half=promoted_clipmap_half_cells(appearance::MIN_PROMOTED_AMPLITUDE);
        let topology=|masks:&OceanMasks| {
            for camera in cameras {
                let distance=promoted_footprint_distance(camera,surface,masks,0);
                let minimum=runtime.layout.minimum_level_for_distance(half,distance);
                // Both paths use the same correct centre to isolate query cost.
                let centre=promoted_footprint_clipmap_centre(camera,surface,1280.0,
                    runtime.layout.outer_spacing_for_half_cells(half),&indexed,0);
                black_box(runtime.layout.indices_for_footprint(half,minimum,centre,surface,masks,masks.is_masked(0).then_some(0),None));
            }
        };
        let old_topology=sample(&||topology(&linear),3);
        let new_topology=sample(&||topology(&indexed),3);
        CAPTURE.with(|c|*c.borrow_mut()=Some(Vec::new()));
        topology(&indexed);
        let queries=CAPTURE.with(|c|c.borrow_mut().take().unwrap());
        let mut fixture=Vec::new();
        for count in [footprint.triangles.len(),footprint.tiles.len(),queries.len()] {fixture.extend((count as u32).to_le_bytes());}
        for triangle in &footprint.triangles {for value in triangle.iter().flatten() {fixture.extend(value.to_le_bytes());}}
        for tile in &footprint.tiles {for value in tile.minimum.into_iter().chain(tile.maximum) {fixture.extend(value.to_le_bytes());}}
        for (lo,hi) in &queries {for value in lo.iter().chain(hi) {fixture.extend(value.to_le_bytes());}}
        let output=workspace.join("target/coastal-perf-20261010");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("racearena-tile-queries.bin"),fixture).unwrap();
        println!("Exported {} actual clipmap cell queries for standalone optimized benchmarking",queries.len());
        println!("RaceArena 5-camera / 8-pool queries: linear {old_nearest:.3} ms; indexed {new_nearest:.3} ms");
        println!("RaceArena 5-camera outer-pool topology rebuilds: linear {old_topology:.3} ms; indexed {new_topology:.3} ms");
        for camera in cameras {
            let minimum=runtime.layout.minimum_level_for_distance(half,promoted_footprint_distance(camera,surface,&indexed,0));
            let centre=promoted_footprint_clipmap_centre(camera,surface,1280.0,runtime.layout.outer_spacing_for_half_cells(half),&indexed,0);
            assert_eq!(runtime.layout.indices_for_footprint(half,minimum,centre,surface,&linear,linear.is_masked(0).then_some(0),None),
                runtime.layout.indices_for_footprint(half,minimum,centre,surface,&indexed,indexed.is_masked(0).then_some(0),None),
                "spatial acceleration must not change the mesh cells that survive pruning");
        }
    }

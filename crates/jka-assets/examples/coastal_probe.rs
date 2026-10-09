//! Headless real-map geometry validation. Uses raw BSP water-content surfaces
//! and the same legacy grouping as the renderer. Runtime shader eligibility
//! is additionally checked by the client's coastal_debug path.
use jka_assets::{
    bsp::{
        bathymetry::{self, Geometry, Options, Region, WaterFace},
        Bsp, MAX_FILE_BYTES,
    },
    pk3::AssetSearchPath,
};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        return Err("usage: coastal_probe BASE (--list | MAP [OUTPUT] [CELL_SIZE])".into());
    }
    let mut assets = AssetSearchPath::open(&PathBuf::from(&args[1]))?;
    if args[2] == "--list" {
        for name in assets
            .names()
            .filter(|n| n.starts_with("maps/") && n.ends_with(".bsp"))
        {
            println!("{name}");
        }
        return Ok(());
    }
    let map = args[2].trim_start_matches("maps/").trim_end_matches(".bsp");
    let source = assets
        .read(&format!("maps/{map}.bsp"), MAX_FILE_BYTES)?
        .ok_or("map not found")?;
    let bsp = Bsp::parse(&source.bytes)?;
    let mesh = bsp.world_mesh(4)?;
    let mut faces = Vec::new();
    let mut footprints = Vec::new();
    for batch in &mesh.batches {
        if bsp.shaders[batch.shader].contents & bathymetry::CONTENTS_WATER == 0 {
            continue;
        }
        let ids = &mesh.indices[batch.indices.clone()];
        let verts: Vec<_> = ids.iter().map(|&i| mesh.vertices[i as usize]).collect();
        let level = verts
            .iter()
            .map(|v| v.position[2])
            .fold(f32::NEG_INFINITY, f32::max);
        if verts.iter().any(|v| v.position[2] < level - 0.01)
            || verts.iter().map(|v| v.normal[2]).sum::<f32>() <= 0.0
        {
            continue;
        }
        let min: [f32; 2] = std::array::from_fn(|a| {
            verts
                .iter()
                .map(|v| {
                    if a == 0 {
                        v.position[0]
                    } else {
                        -v.position[1]
                    }
                })
                .fold(f32::INFINITY, f32::min)
        });
        let max: [f32; 2] = std::array::from_fn(|a| {
            verts
                .iter()
                .map(|v| {
                    if a == 0 {
                        v.position[0]
                    } else {
                        -v.position[1]
                    }
                })
                .fold(f32::NEG_INFINITY, f32::max)
        });
        faces.push(WaterFace {
            plane: level,
            minimum: min,
            maximum: max,
            authored_ocean: None,
            material: batch.shader as u32,
        });
        footprints.push(
            verts
                .chunks_exact(3)
                .map(|t| [0, 1, 2].map(|i| [t[i].position[0], t[i].position[1]]))
                .collect::<Vec<_>>(),
        );
    }
    let regions: Vec<_> = bathymetry::cluster_water_faces(&faces)
        .iter()
        .take(8)
        .enumerate()
        .map(|(slot, c)| Region {
            slot: slot as u32,
            level: c.plane,
            triangles: c
                .members
                .iter()
                .flat_map(|&i| footprints[i].iter().copied())
                .collect(),
        })
        .collect();
    let started = Instant::now();
    let geometry = Geometry::from_bsp(&bsp)?;
    let options = Options {
        cell_size: args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(16.0),
        ..Options::default()
    };
    let mut atlas = geometry.bake(&regions, options)?;
    atlas.bake_ms = started.elapsed().as_secs_f64() * 1000.0;
    let output = args.get(3).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from("target/coastal-checkpoint1/maps").join(map.replace('/', "_"))
    });
    atlas.write_debug(&output)?;
    let key = [bathymetry::fingerprint(&source.bytes), 0];
    let encoded = atlas.encode(key);
    std::fs::write(output.join("bathymetry.bin"), &encoded)?;
    let start = Instant::now();
    let decoded = bathymetry::Atlas::decode(&encoded, key)?;
    let decode_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(decoded.memory_bytes(), atlas.memory_bytes());
    println!("Map: {map}; source: {}; raw water faces: {}; regions: {}\n{}Cache: {} bytes, decode {:.3} ms\nReport: {}",source.source.display(),faces.len(),regions.len(),atlas.summary(),encoded.len(),decode_ms,output.join("bathymetry.html").display());
    Ok(())
}

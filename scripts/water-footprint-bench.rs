//! Optimized CPU reproduction of the actual RaceArena clipmap tile queries.
//! First export the fixture using racearena_footprint_hotpath_benchmark.
#![allow(dead_code)]
use std::{hint::black_box,time::Instant};
type MaskTriangle=[[f32;2];3];
struct Tile {minimum:[f32;2],maximum:[f32;2]}
struct WaterFootprint {triangles:Vec<MaskTriangle>,tiles:Vec<Tile>}
#[path="../crates/jka-client/src/ocean/footprint_index.rs"]
mod footprint_index;
fn main() {
    let path=std::env::args().nth(1).expect("provide exported racearena-tile-queries.bin");
    let data=std::fs::read(path).unwrap();
    let mut offset=0usize;
    let mut word=||{let value=u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());offset+=4;value};
    let counts=[word(),word(),word()].map(|v|v as usize);
    let mut scalar=||f32::from_bits(word());
    let mut triangles=Vec::with_capacity(counts[0]);
    for _ in 0..counts[0] {triangles.push([[scalar(),scalar()],[scalar(),scalar()],[scalar(),scalar()]]);}
    let mut tiles=Vec::with_capacity(counts[1]);
    for _ in 0..counts[1] {tiles.push(Tile {minimum:[scalar(),scalar()],maximum:[scalar(),scalar()]});}
    let mut queries=Vec::with_capacity(counts[2]);
    for _ in 0..counts[2] {queries.push(([scalar(),scalar()],[scalar(),scalar()]));}
    let footprint=WaterFootprint {triangles,tiles};
    let index=footprint_index::FootprintIndex::new(&footprint);
    let linear=|lo:[f32;2],hi:[f32;2]|footprint.tiles.iter().any(|t|
        hi[0]>=t.minimum[0] && hi[1]>=t.minimum[1] && lo[0]<=t.maximum[0] && lo[1]<=t.maximum[1]);
    for &(lo,hi) in &queries {assert_eq!(index.rect_may_contain(lo,hi),linear(lo,hi));}
    let benchmark=|work:&dyn Fn()->usize| {
        let mut timings=Vec::new();
        for _ in 0..5 {let start=Instant::now();black_box(work());timings.push(start.elapsed().as_secs_f64()*1000.0);}
        timings.sort_by(f64::total_cmp);timings[2]
    };
    let old=benchmark(&||queries.iter().filter(|&&(lo,hi)|linear(black_box(lo),black_box(hi))).count());
    let new=benchmark(&||queries.iter().filter(|&&(lo,hi)|index.rect_may_contain(black_box(lo),black_box(hi))).count());
    println!("Optimized RaceArena: {} triangles, {} tiles, {} real clipmap cell queries across 5 camera positions",counts[0],counts[1],counts[2]);
    println!("Tile queries: median linear {old:.3} ms / indexed {new:.3} ms; speedup {:.1}x",old/new);
    println!("Mean per topology rebuild: linear {:.3} ms / indexed {:.3} ms (CPU tile query cost only)",old/5.0,new/5.0);
}


// Scratch A/B tool: both our own auto-rebind of a Source model and a
// hand-ported reference GLM land in the same `_humanoid` base-pose space, so
// their vertices are directly comparable by nearest-neighbor position match
// even though the two meshes are not topologically identical. For each
// hand-port vertex this finds the nearest vertex our own rig produced and
// reports the position gap plus whether the two agree on which GLA bone
// dominates that vertex's skin weight - real ground truth for where the
// automatic rig's bone choice is wrong, instead of guessing from first
// principles. Not wired into any build.
use jka_assets::ghoul2::{parse_gla, parse_glm, GlaAnimation};
use jka_assets::source1::{parse_source1_materials, parse_source1_skeleton};
use jka_assets::source1_geometry::{parse_source_vtx, parse_source_vvd};
use jka_assets::source1_rig::{build_source_rig, SourceRigOptions};
use std::collections::BTreeMap;
use std::path::Path;

struct Point {
    position: [f32; 3],
    bone: usize,
}

fn dominant_bone(weights: &[jka_assets::ghoul2::GlmWeight], bone_references: &[usize]) -> usize {
    weights
        .iter()
        .max_by(|a, b| a.weight.total_cmp(&b.weight))
        .map(|w| bone_references[w.local_bone_index])
        .expect("vertex has at least one weight")
}

fn bone_name(gla: &GlaAnimation, index: usize) -> &str {
    gla.skeleton.get(index).map(|b| b.name.as_str()).unwrap_or("?")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let auto_mdl = args.next().ok_or("usage: compare_juliet_rebind <auto.mdl> <_humanoid.gla> <hand_port.glm> <hand_surface_substr>...")?;
    let gla_path = args.next().ok_or("missing gla path")?;
    let hand_glm_path = args.next().ok_or("missing hand-port glm path")?;
    let filters: Vec<String> = args.collect();

    let path = Path::new(&auto_mdl);
    let stem = path.file_stem().ok_or("missing model filename")?.to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new("."));
    let mdl = std::fs::read(path)?;
    let vvd = parse_source_vvd(&std::fs::read(dir.join(format!("{stem}.vvd")))?)?;
    let vtx = parse_source_vtx(&std::fs::read(dir.join(format!("{stem}.dx90.vtx")))?)?;
    let skeleton = parse_source1_skeleton(&mdl)?;
    let materials = parse_source1_materials(&mdl)?;
    let gla = parse_gla(&std::fs::read(&gla_path)?)?;

    let rig = build_source_rig(
        &auto_mdl,
        &mdl,
        &vvd,
        &vtx,
        &skeleton,
        &materials,
        &gla,
        &SourceRigOptions::default(),
    )?;

    let mut auto_points: Vec<Point> = Vec::new();
    for lod in rig.glm.lods.iter().take(1) {
        for surface in &lod.surfaces {
            let name = &rig.surfaces[surface.surface_index].name;
            if !name.contains("body") {
                continue;
            }
            for v in &surface.vertices {
                auto_points.push(Point {
                    position: v.position,
                    bone: dominant_bone(&v.weights, &surface.bone_references),
                });
            }
        }
    }
    println!("auto rig: {} body vertices pooled", auto_points.len());

    let hand_data = std::fs::read(&hand_glm_path)?;
    let hand_model = parse_glm(&hand_data)?;
    let hand_lod = hand_model.lods.first().ok_or("hand-port GLM has no LOD0")?;

    let mut hand_points: Vec<Point> = Vec::new();
    for surface in &hand_lod.surfaces {
        let hname = hand_model
            .hierarchy
            .get(surface.surface_index)
            .map(|h| h.name.as_str())
            .unwrap_or("?");
        if !filters.iter().any(|f| hname.contains(f.as_str())) {
            continue;
        }
        for v in &surface.vertices {
            hand_points.push(Point {
                position: v.position,
                bone: dominant_bone(&v.weights, &surface.bone_references),
            });
        }
    }
    println!("hand port: {} vertices pooled from {:?}", hand_points.len(), filters);
    if hand_points.is_empty() || auto_points.is_empty() {
        return Err("empty point set; check surface name filters".into());
    }

    let dist2 = |a: [f32; 3], b: [f32; 3]| -> f32 {
        (0..3).map(|k| (a[k] - b[k]).powi(2)).sum()
    };

    let mut distance_hist: BTreeMap<&str, usize> = BTreeMap::new();
    let mut agree = 0usize;
    let mut close_total = 0usize;
    let mut disagreements: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut worst = 0.0f32;

    for hp in &hand_points {
        let mut best_d2 = f32::INFINITY;
        let mut best_bone = usize::MAX;
        for ap in &auto_points {
            let d2 = dist2(hp.position, ap.position);
            if d2 < best_d2 {
                best_d2 = d2;
                best_bone = ap.bone;
            }
        }
        let d = best_d2.sqrt();
        worst = worst.max(d);
        let bucket = if d < 0.5 {
            "<0.5"
        } else if d < 2.0 {
            "0.5-2"
        } else if d < 5.0 {
            "2-5"
        } else {
            ">5"
        };
        *distance_hist.entry(bucket).or_default() += 1;
        if d < 2.0 {
            close_total += 1;
            if best_bone == hp.bone {
                agree += 1;
            } else {
                *disagreements
                    .entry((bone_name(&gla, hp.bone).to_owned(), bone_name(&gla, best_bone).to_owned()))
                    .or_default() += 1;
            }
        }
    }

    println!("\nnearest-neighbor distance histogram: {:?}", distance_hist);
    println!("worst nearest-neighbor distance: {:.3}", worst);
    println!(
        "bone agreement among close matches (<2u): {agree}/{close_total} ({:.1}%)",
        100.0 * agree as f32 / close_total.max(1) as f32
    );
    let mut ranked: Vec<_> = disagreements.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\ntop disagreements (hand-port bone -> nearest auto-rig bone):");
    for ((hand_bone, auto_bone), count) in ranked.iter().take(20) {
        println!("  {hand_bone:>16} -> {auto_bone:<16} x{count}");
    }

    println!("\n== auto rig: weight mass per bone, per body surface ==");
    for lod in rig.glm.lods.iter().take(1) {
        for surface in &lod.surfaces {
            let name = &rig.surfaces[surface.surface_index].name;
            if !name.contains("body") {
                continue;
            }
            let mut mass: BTreeMap<usize, f32> = BTreeMap::new();
            let mut histo: BTreeMap<usize, usize> = BTreeMap::new();
            for v in &surface.vertices {
                *histo.entry(v.weights.len()).or_default() += 1;
                for w in &v.weights {
                    *mass.entry(surface.bone_references[w.local_bone_index]).or_default() += w.weight;
                }
            }
            println!("surface {name} ({} verts), weights-per-vertex {:?}", surface.vertices.len(), histo);
            let mut ranked_mass: Vec<_> = mass.into_iter().collect();
            ranked_mass.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (bone, m) in ranked_mass.iter().take(12) {
                println!("    {:>16} (#{:<3}) mass={:.1}", bone_name(&gla, *bone), bone, m);
            }
        }
    }
    Ok(())
}

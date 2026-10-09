//! Offline verification for the Source 1 -> `_humanoid` rebind.
//!
//! ```text
//! cargo run -p jka-assets --example inspect_source1 -- <model.mdl> <_humanoid.gla>
//! ```
//!
//! Reports the fixup-gathered LOD chain, the ValveBiped bone correspondence, the
//! skeleton alignment fit, and a CPU reference check that the baked rest pose
//! really is the GLA base pose. Nothing here touches the renderer, so a clean
//! run means the asset path is sound before the client is even started.
use jka_assets::source1::{parse_source1_materials, parse_source1_skeleton};
use jka_assets::source1_geometry::{
    assemble_source_lod, parse_source_bodyparts, parse_source_vtx, parse_source_vvd,
    source_lod_vertices,
};
use jka_assets::source1_rig::{build_source_rig, SourceRigOptions};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model = args
        .next()
        .ok_or("usage: inspect_source1 <model.mdl> <_humanoid.gla>")?;
    let gla_path = args
        .next()
        .ok_or("usage: inspect_source1 <model.mdl> <_humanoid.gla>")?;
    let path = Path::new(&model);
    let stem = path.file_stem().ok_or("missing model filename")?.to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new("."));

    let mdl = std::fs::read(path)?;
    let vvd = parse_source_vvd(&std::fs::read(dir.join(format!("{stem}.vvd")))?)?;
    let vtx = parse_source_vtx(&std::fs::read(dir.join(format!("{stem}.dx90.vtx")))?)?;
    let skeleton = parse_source1_skeleton(&mdl)?;
    let materials = parse_source1_materials(&mdl)?;
    let gla = jka_assets::ghoul2::parse_gla(&std::fs::read(&gla_path)?)?;

    println!("== Source model ==");
    println!(
        "{} : {} bones, {} LODs, {} fixups, {} raw vertices",
        skeleton.model_name,
        skeleton.bones.len(),
        vvd.lod_count,
        vvd.fixups.len(),
        vvd.vertices.len()
    );
    if vvd.fixups.is_empty() {
        println!("  no fixup table: the raw vertex table is already in LOD order");
    } else {
        println!(
            "  fixup table present: MDL vertex offsets address the GATHERED array, \
             never the raw one"
        );
    }
    for lod in 0..vvd.lod_count {
        let pool = source_lod_vertices(&vvd, lod)?;
        let draws = assemble_source_lod(&mdl, &vvd, &vtx, lod, &[])?;
        println!(
            "  LOD{lod}: {} gathered vertices, {} mesh sections, {} triangles",
            pool.len(),
            draws.len(),
            draws.iter().map(|d| d.indices.len() / 3).sum::<usize>()
        );
    }

    println!("\n== Bodygroups / skins ==");
    for (index, body) in parse_source_bodyparts(&mdl)?.iter().enumerate() {
        println!(
            "  bodypart {index} {:?}: {}",
            body.name,
            body.models
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    println!(
        "  {} textures, {} skin families, $cdmaterials {:?}",
        materials.textures.len(),
        materials.skin_families.len(),
        materials.directories
    );

    let rig = build_source_rig(
        &model,
        &mdl,
        &vvd,
        &vtx,
        &skeleton,
        &materials,
        &gla,
        &SourceRigOptions::default(),
    )?;
    let d = &rig.diagnostics;

    println!("\n== Rebind onto _humanoid ==");
    println!(
        "  bones: {} total, {} name-mapped, {} inherited from an ancestor",
        d.source_bones,
        d.mapped_bones,
        d.inherited_bones.len()
    );
    for (source, target) in &d.inherited_bones {
        println!("    {source} -> (ancestor) {target}");
    }
    println!(
        "  scale {:.5} ({}), fitted yaw {:.2} deg over {} anchors",
        d.scale,
        if d.scale_was_fitted { "fitted" } else { "configured" },
        d.fit_yaw_degrees,
        d.fit_pairs
    );
    println!("  skeleton alignment RMS {:.3} JKA units", d.fit_rms);
    if let Some((source, target, error)) = &d.fit_worst {
        println!("    worst anchor: {source} vs {target} off by {error:.3}");
    }
    println!(
        "  authored poseToBone vs derived inverse bind: max |delta| {:.6}",
        d.max_inverse_bind_error
    );
    println!(
        "  rest bounds min {:?} max {:?}",
        d.rest_bounds.0.map(|v| (v * 100.).round() / 100.),
        d.rest_bounds.1.map(|v| (v * 100.).round() / 100.)
    );
    println!(
        "  rebind mode {:?}, {} joints direction-corrected",
        d.rebind, d.direction_corrected
    );
    println!(
        "  max rebind displacement {:.3} units (how far the rebind reshaped the mesh)",
        d.max_rebind_displacement
    );
    println!("  vertices whose merged influences exceeded 4: {}", d.weight_truncations);
    for lod in &d.lods {
        println!(
            "  GLM LOD{}: {} surfaces, {} vertices, {} triangles",
            lod.lod, lod.surfaces, lod.vertices, lod.triangles
        );
    }

    println!("\n== Surfaces ==");
    for (index, surface) in rig.surfaces.iter().enumerate() {
        println!(
            "  {index:2} {:<28} {}",
            surface.name,
            surface
                .vmt_candidates
                .first()
                .map(String::as_str)
                .unwrap_or("<no material>")
        );
    }

    // Structural checks the GPU path depends on: every local bone index must
    // address this surface's bone_references, every reference must address the
    // GLA skeleton, and weights must be normalized. A failure here is a bad
    // vertex buffer, which the renderer would silently skin into nonsense.
    println!("\n== Vertex / bone reference check ==");
    let mut worst = 0.0f32;
    let mut checked = 0usize;
    for lod in &rig.glm.lods {
        for surface in &lod.surfaces {
            for (index, vertex) in surface.vertices.iter().enumerate() {
                let total: f32 = vertex.weights.iter().map(|w| w.weight).sum();
                worst = worst.max((total - 1.0).abs());
                for weight in &vertex.weights {
                    if surface.bone_references.get(weight.local_bone_index).is_none() {
                        return Err(format!(
                            "surface {} vertex {index} references local bone {} with only {} refs",
                            surface.surface_index,
                            weight.local_bone_index,
                            surface.bone_references.len()
                        )
                        .into());
                    }
                }
                for &bone in &surface.bone_references {
                    if bone >= gla.skeleton.len() {
                        return Err(format!(
                            "surface {} references GLA bone {bone} but the skeleton has {}",
                            surface.surface_index,
                            gla.skeleton.len()
                        )
                        .into());
                    }
                }
                checked += 1;
            }
        }
    }
    println!("  {checked} vertices: every bone reference in range");
    println!("  worst |sum(weights) - 1| = {worst:.6}");
    if worst > 1e-3 {
        return Err("baked vertex weights are not normalized".into());
    }
    println!("\nOK");
    Ok(())
}

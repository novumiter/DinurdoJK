// Offline model preview: rasterizes a mesh straight to a PNG with no game
// client, no GPU, no map load - just a software z-buffered rasterizer over
// the same base-pose-space vertices the renderer would eventually draw.
// Built specifically to A/B our own Source1 auto-rebind against a hand-ported
// reference GLM as actual pictures, not just vertex statistics.
//
// usage: render_preview <auto.mdl> <_humanoid.gla> <hand_port.glm> <hand_surface_substr>...
// writes auto_preview.png and hand_preview.png next to the cwd.
use image::{Rgb, RgbImage};
use jka_assets::ghoul2::parse_glm;
use jka_assets::source1::{parse_source1_materials, parse_source1_skeleton};
use jka_assets::source1_geometry::{parse_source_vtx, parse_source_vvd};
use jka_assets::source1_rig::{build_source_rig, SourceRebindMode, SourceRigOptions};
use std::path::Path;

struct Tri {
    p: [[f32; 3]; 3],
    n: [f32; 3],
}

fn face_normal(p: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let b = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let c = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt().max(1e-6);
    [c[0] / len, c[1] / len, c[2] / len]
}

/// Front orthographic render: screen x = model x, screen y = -model z
/// (model space is +Z up), depth = model y (camera looks down +Y, so a
/// smaller y is nearer the viewer and wins the z-test). Flat-shaded per
/// triangle against a fixed camera-ish light so silhouette/proportion read
/// clearly; this is a diagnostic, not a renderer.
fn rasterize(tris: &[Tri], width: u32, height: u32, min: [f32; 2], max: [f32; 2]) -> RgbImage {
    let mut img = RgbImage::from_pixel(width, height, Rgb([18, 18, 24]));
    let mut depth = vec![f32::INFINITY; (width * height) as usize];
    let span_x = (max[0] - min[0]).max(1e-3);
    let span_y = (max[1] - min[1]).max(1e-3);
    let scale = (width as f32 / span_x).min(height as f32 / span_y) * 0.92;
    let cx = (min[0] + max[0]) * 0.5;
    let cy = (min[1] + max[1]) * 0.5;
    // Model space is +Z up; screen Y grows downward, so flip Z around the
    // vertical center of the shared bounding box.
    let to_screen = |p: [f32; 3]| -> (f32, f32, f32) {
        let sx = (p[0] - cx) * scale + width as f32 * 0.5;
        let sy = (-(p[2] - cy)) * scale + height as f32 * 0.5;
        (sx, sy, p[1])
    };
    let light = {
        let l = [-0.35f32, -1.0, 0.55];
        let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / len, l[1] / len, l[2] / len]
    };
    for tri in tris {
        let a = to_screen(tri.p[0]);
        let b = to_screen(tri.p[1]);
        let c = to_screen(tri.p[2]);
        let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as i32;
        let max_x = a.0.max(b.0).max(c.0).ceil().min(width as f32) as i32;
        let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as i32;
        let max_y = a.1.max(b.1).max(c.1).ceil().min(height as f32) as i32;
        if min_x >= max_x || min_y >= max_y {
            continue;
        }
        let denom = (b.1 - c.1) * (a.0 - c.0) + (c.0 - b.0) * (a.1 - c.1);
        if denom.abs() < 1e-6 {
            continue;
        }
        let ndotl = (tri.n[0] * -light[0] + tri.n[1] * -light[1] + tri.n[2] * -light[2]).max(0.0);
        let base = 60.0 + 180.0 * ndotl;
        let color = Rgb([base as u8, (base * 0.92) as u8, (base * 0.85) as u8]);
        for y in min_y..max_y {
            for x in min_x..max_x {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b.1 - c.1) * (px - c.0) + (c.0 - b.0) * (py - c.1)) / denom;
                let w1 = ((c.1 - a.1) * (px - c.0) + (a.0 - c.0) * (py - c.1)) / denom;
                let w2 = 1.0 - w0 - w1;
                if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 {
                    continue;
                }
                let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
                let index = (y as u32 * width + x as u32) as usize;
                if z < depth[index] {
                    depth[index] = z;
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
    img
}

fn bounds(tris: &[Tri]) -> ([f32; 2], [f32; 2]) {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for tri in tris {
        for p in &tri.p {
            min[0] = min[0].min(p[0]);
            min[1] = min[1].min(p[2]);
            max[0] = max[0].max(p[0]);
            max[1] = max[1].max(p[2]);
        }
    }
    (min, max)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let auto_mdl = args.next().ok_or("usage: render_preview <auto.mdl> <_humanoid.gla> <hand_port.glm> <hand_surface_substr>...")?;
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
    let gla = jka_assets::ghoul2::parse_gla(&std::fs::read(&gla_path)?)?;

    let mode = match std::env::var("RIG_MODE").as_deref() {
        Ok("translation") => SourceRebindMode::Translation,
        Ok("bone_frame") => SourceRebindMode::BoneFrame,
        _ => SourceRebindMode::Direction,
    };
    let options = SourceRigOptions { rebind: mode, ..SourceRigOptions::default() };
    let rig = build_source_rig(&auto_mdl, &mdl, &vvd, &vtx, &skeleton, &materials, &gla, &options)?;
    let mut auto_tris = Vec::new();
    for lod in rig.glm.lods.iter().take(1) {
        for surface in &lod.surfaces {
            for tri in &surface.triangles {
                let p = tri.map(|i| surface.vertices[i as usize].position);
                let n = face_normal(&p);
                auto_tris.push(Tri { p, n });
            }
        }
    }
    println!("auto rig: {} triangles rendered", auto_tris.len());

    let hand_data = std::fs::read(&hand_glm_path)?;
    let hand_model = parse_glm(&hand_data)?;
    let hand_lod = hand_model.lods.first().ok_or("hand-port GLM has no LOD0")?;
    let mut hand_tris = Vec::new();
    for surface in &hand_lod.surfaces {
        let hname = hand_model.hierarchy.get(surface.surface_index).map(|h| h.name.as_str()).unwrap_or("?");
        if hname.starts_with('*') {
            continue;
        }
        if !filters.is_empty() && !filters.iter().any(|f| hname.contains(f.as_str())) {
            continue;
        }
        for tri in &surface.triangles {
            let p = tri.map(|i| surface.vertices[i as usize].position);
            let n = face_normal(&p);
            hand_tris.push(Tri { p, n });
        }
    }
    println!("hand port: {} triangles rendered", hand_tris.len());

    // Shared framing so proportions compare directly.
    let (amin, amax) = bounds(&auto_tris);
    let (hmin, hmax) = bounds(&hand_tris);
    let min = [amin[0].min(hmin[0]), amin[1].min(hmin[1])];
    let max = [amax[0].max(hmax[0]), amax[1].max(hmax[1])];

    let (w, h) = (700u32, 900u32);
    rasterize(&auto_tris, w, h, min, max).save("auto_preview.png")?;
    rasterize(&hand_tris, w, h, min, max).save("hand_preview.png")?;
    println!("wrote auto_preview.png and hand_preview.png");
    Ok(())
}

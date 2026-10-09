// Renders a Source .mdl's RAW vertex positions with no rebind at all: just
// the authored geometry in its own native proportions. Answers "is the
// skinny/long-limbed look an artifact of our rebind, or just how this
// character was modeled?" before blaming the rebind pipeline for a body type.
use image::{Rgb, RgbImage};
use jka_assets::source1_geometry::{assemble_source_lod, parse_source_vtx, parse_source_vvd, source_lod_vertices};
use std::path::Path;

struct Tri { p: [[f32; 3]; 3], n: [f32; 3] }

fn face_normal(p: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = [p[1][0]-p[0][0], p[1][1]-p[0][1], p[1][2]-p[0][2]];
    let b = [p[2][0]-p[0][0], p[2][1]-p[0][1], p[2][2]-p[0][2]];
    let c = [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];
    let len = (c[0]*c[0]+c[1]*c[1]+c[2]*c[2]).sqrt().max(1e-6);
    [c[0]/len, c[1]/len, c[2]/len]
}

fn rasterize(tris: &[Tri], width: u32, height: u32, min: [f32; 2], max: [f32; 2], vertical_axis: usize, horizontal_axis: usize) -> RgbImage {
    let mut img = RgbImage::from_pixel(width, height, Rgb([18, 18, 24]));
    let mut depth = vec![f32::INFINITY; (width * height) as usize];
    let span_x = (max[0]-min[0]).max(1e-3);
    let span_y = (max[1]-min[1]).max(1e-3);
    let scale = (width as f32/span_x).min(height as f32/span_y) * 0.92;
    let cx = (min[0]+max[0])*0.5;
    let cy = (min[1]+max[1])*0.5;
    let to_screen = |p: [f32;3]| -> (f32,f32,f32) {
        ((p[horizontal_axis]-cx)*scale + width as f32*0.5, (-(p[vertical_axis]-cy))*scale + height as f32*0.5, p[3-vertical_axis-horizontal_axis])
    };
    let light = { let l=[-0.35f32,-1.0,0.55]; let n=(l[0]*l[0]+l[1]*l[1]+l[2]*l[2]).sqrt(); [l[0]/n,l[1]/n,l[2]/n] };
    for tri in tris {
        let a = to_screen(tri.p[0]); let b = to_screen(tri.p[1]); let c = to_screen(tri.p[2]);
        let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as i32;
        let max_x = a.0.max(b.0).max(c.0).ceil().min(width as f32) as i32;
        let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as i32;
        let max_y = a.1.max(b.1).max(c.1).ceil().min(height as f32) as i32;
        if min_x >= max_x || min_y >= max_y { continue; }
        let denom = (b.1-c.1)*(a.0-c.0) + (c.0-b.0)*(a.1-c.1);
        if denom.abs() < 1e-6 { continue; }
        let ndotl = (tri.n[0]*-light[0]+tri.n[1]*-light[1]+tri.n[2]*-light[2]).max(0.0);
        let base = 60.0 + 180.0*ndotl;
        let color = Rgb([base as u8, (base*0.92) as u8, (base*0.85) as u8]);
        for y in min_y..max_y { for x in min_x..max_x {
            let (px,py) = (x as f32+0.5, y as f32+0.5);
            let w0 = ((b.1-c.1)*(px-c.0)+(c.0-b.0)*(py-c.1))/denom;
            let w1 = ((c.1-a.1)*(px-c.0)+(a.0-c.0)*(py-c.1))/denom;
            let w2 = 1.0-w0-w1;
            if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 { continue; }
            let z = w0*a.2+w1*b.2+w2*c.2;
            let index = (y as u32*width+x as u32) as usize;
            if z < depth[index] { depth[index]=z; img.put_pixel(x as u32, y as u32, color); }
        }}
    }
    img
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path_arg = std::env::args().nth(1).expect("usage: render_raw_source <model.mdl> <out.png>");
    let out_path = std::env::args().nth(2).unwrap_or_else(|| "raw_preview.png".into());
    let path = Path::new(&path_arg);
    let stem = path.file_stem().unwrap().to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new("."));
    let vvd = parse_source_vvd(&std::fs::read(dir.join(format!("{stem}.vvd")))?)?;
    let vtx = parse_source_vtx(&std::fs::read(dir.join(format!("{stem}.dx90.vtx")))?)?;
    let mdl = std::fs::read(path)?;
    let pool = source_lod_vertices(&vvd, 0)?;
    let draws = assemble_source_lod(&mdl, &vvd, &vtx, 0, &[])?;
    let mut tris = Vec::new();
    for draw in &draws {
        for chunk in draw.indices.chunks_exact(3) {
            let p = [
                pool[chunk[0] as usize].position,
                pool[chunk[1] as usize].position,
                pool[chunk[2] as usize].position,
            ];
            tris.push(Tri { p, n: face_normal(&p) });
        }
    }
    println!("{} triangles (raw, unrebound)", tris.len());
    // Source model space: Valve uses +Z up, so view straight on with X
    // horizontal, Z vertical (axis indices 0 and 2), same as our rebind space.
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for t in &tris { for p in &t.p {
        min[0]=min[0].min(p[0]); min[1]=min[1].min(p[2]);
        max[0]=max[0].max(p[0]); max[1]=max[1].max(p[2]);
    }}
    rasterize(&tris, 700, 900, min, max, 2, 0).save(&out_path)?;
    println!("wrote {out_path}");
    Ok(())
}

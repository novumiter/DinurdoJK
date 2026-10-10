use super::Solver;

pub const FILM_DEPTH_METRES: f32 = 0.0005;

#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceSample {
    pub elevation: f32,
    pub depth: f32,
    pub slope_x: f32,
    pub slope_y: f32,
}

/// Reconstructs render elevations without extending water into dry cells or
/// averaging across closed faces. This follows Saltreach's wet-neighbour rule.
pub fn reconstruct_surface(solver: &Solver, output: &mut [SurfaceSample]) {
    let domain = solver.domain();
    let grid = domain.grid;
    assert_eq!(output.len(), grid.len());
    let depth = solver.depth();
    let raw: Vec<f32> = domain.bed.iter().zip(depth).map(|(b, h)| b + h).collect();
    let mut eta = vec![0.0_f32; grid.len()];

    for j in 0..grid.ny {
        for i in 0..grid.nx {
            let k = j * grid.nx + i;
            if !domain.active[k] {
                eta[k] = 0.0;
                continue;
            }
            if depth[k] >= FILM_DEPTH_METRES {
                eta[k] = raw[k];
                continue;
            }
            let mut level = domain.bed[k] - 0.025;
            let mut best = f32::INFINITY;
            let mut consider = |q: usize, open: bool| {
                if !open || depth[q] < FILM_DEPTH_METRES {
                    return;
                }
                let score = (raw[q] - domain.bed[k]).abs();
                if score < best {
                    best = score;
                    level = raw[q];
                }
            };
            if i > 0 {
                consider(k - 1, domain.x_face_open(k - 1));
            }
            if i + 1 < grid.nx {
                consider(k + 1, domain.x_face_open(k));
            }
            if j > 0 {
                consider(k - grid.nx, domain.y_face_open(k - grid.nx));
            }
            if j + 1 < grid.ny {
                consider(k + grid.nx, domain.y_face_open(k));
            }
            let dry = (domain.bed[k] - 0.0005).min(level);
            let t = depth[k] / FILM_DEPTH_METRES;
            eta[k] = dry + (raw[k] - dry) * t * t;
        }
    }

    for j in 0..grid.ny {
        for i in 0..grid.nx {
            let k = j * grid.nx + i;
            if !domain.active[k] {
                output[k] = SurfaceSample::default();
                continue;
            }
            let e = eta[k];
            let left = i > 0
                && domain.x_face_open(k - 1)
                && depth[k - 1] >= FILM_DEPTH_METRES
                && e.min(eta[k - 1]) > domain.bed[k].max(domain.bed[k - 1]);
            let right = i + 1 < grid.nx
                && domain.x_face_open(k)
                && depth[k + 1] >= FILM_DEPTH_METRES
                && e.min(eta[k + 1]) > domain.bed[k].max(domain.bed[k + 1]);
            let back = j > 0
                && domain.y_face_open(k - grid.nx)
                && depth[k - grid.nx] >= FILM_DEPTH_METRES
                && e.min(eta[k - grid.nx]) > domain.bed[k].max(domain.bed[k - grid.nx]);
            let front = j + 1 < grid.ny
                && domain.y_face_open(k)
                && depth[k + grid.nx] >= FILM_DEPTH_METRES
                && e.min(eta[k + grid.nx]) > domain.bed[k].max(domain.bed[k + grid.nx]);
            output[k] = SurfaceSample {
                elevation: e,
                depth: (e - domain.bed[k]).max(0.0),
                slope_x: if left || right {
                    ((if right { eta[k + 1] } else { e }) - (if left { eta[k - 1] } else { e }))
                        / (grid.dx as f32 * if left && right { 2.0 } else { 1.0 })
                } else {
                    0.0
                },
                slope_y: if back || front {
                    ((if front { eta[k + grid.nx] } else { e })
                        - (if back { eta[k - grid.nx] } else { e }))
                        / (grid.dy as f32 * if back && front { 2.0 } else { 1.0 })
                } else {
                    0.0
                },
            };
        }
    }
}

use std::fmt;

pub const GRAVITY: f64 = 9.81;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub nx: usize,
    pub ny: usize,
    pub dx: f64,
    pub dy: f64,
}

impl Grid {
    pub fn new(nx: usize, ny: usize, dx: f64, dy: f64) -> Result<Self, DomainError> {
        if nx < 2 || ny < 2 {
            return Err(DomainError("a coastal grid needs at least 2x2 cells"));
        }
        if !dx.is_finite() || !dy.is_finite() || dx <= 0.0 || dy <= 0.0 {
            return Err(DomainError("cell spacing must be finite and positive"));
        }
        nx.checked_mul(ny).ok_or(DomainError("grid is too large"))?;
        Ok(Self { nx, ny, dx, dy })
    }

    pub fn len(self) -> usize {
        self.nx * self.ny
    }

    pub fn is_empty(self) -> bool {
        self.nx == 0 || self.ny == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainError(pub &'static str);

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DomainError {}

/// Static solver geometry. `active=false` is outside/unsupported geometry.
/// `wall_x[k]` blocks the face from cell `k` to `k + 1`; `wall_y[k]` blocks
/// the face from cell `k` to `k + nx`. Outer faces are always closed.
#[derive(Clone, Debug)]
pub struct Domain {
    pub grid: Grid,
    pub bed: Vec<f32>,
    pub active: Vec<bool>,
    pub wall_x: Vec<bool>,
    pub wall_y: Vec<bool>,
}

impl Domain {
    pub fn new(grid: Grid, bed: Vec<f32>) -> Result<Self, DomainError> {
        let n = grid.len();
        if bed.len() != n {
            return Err(DomainError("bed length does not match grid"));
        }
        if bed.iter().any(|v| !v.is_finite()) {
            return Err(DomainError("active bed heights must be finite"));
        }
        Ok(Self {
            grid,
            bed,
            active: vec![true; n],
            wall_x: vec![false; n],
            wall_y: vec![false; n],
        })
    }

    pub fn with_geometry(
        grid: Grid,
        bed: Vec<f32>,
        active: Vec<bool>,
        wall_x: Vec<bool>,
        wall_y: Vec<bool>,
    ) -> Result<Self, DomainError> {
        let n = grid.len();
        if bed.len() != n || active.len() != n || wall_x.len() != n || wall_y.len() != n {
            return Err(DomainError("domain array length does not match grid"));
        }
        if bed
            .iter()
            .zip(&active)
            .any(|(height, active)| *active && !height.is_finite())
        {
            return Err(DomainError("active bed heights must be finite"));
        }
        Ok(Self {
            grid,
            bed,
            active,
            wall_x,
            wall_y,
        })
    }

    #[inline]
    pub fn x_face_open(&self, k: usize) -> bool {
        let i = k % self.grid.nx;
        i + 1 < self.grid.nx && self.active[k] && self.active[k + 1] && !self.wall_x[k]
    }

    #[inline]
    pub fn y_face_open(&self, k: usize) -> bool {
        let j = k / self.grid.nx;
        j + 1 < self.grid.ny && self.active[k] && self.active[k + self.grid.nx] && !self.wall_y[k]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SolverOptions {
    pub gravity: f64,
    pub base_drag: f64,
    pub shallow_drag: f64,
    pub shallow_drag_offset: f64,
    pub face_dry_epsilon: f64,
    pub max_velocity: f64,
    pub artificial_viscosity: bool,
    pub cfl: f64,
    pub minimum_step: f64,
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            gravity: GRAVITY,
            base_drag: 0.065,
            shallow_drag: 0.08,
            shallow_drag_offset: 0.075,
            face_dry_epsilon: 0.001,
            max_velocity: 5.0,
            artificial_viscosity: true,
            cfl: 0.42,
            minimum_step: 1.0e-6,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BoundaryTarget {
    pub cell: usize,
    pub surface: f64,
    /// Exponential response rate in inverse seconds. Saltreach uses
    /// `9 * sponge_weight`.
    pub rate: f64,
    pub velocity_x: f64,
    pub velocity_y: f64,
    pub transverse_velocity_blend: f64,
}

#[derive(Clone, Debug)]
pub struct StepFields {
    pub velocity_x: Vec<f32>,
    pub velocity_y: Vec<f32>,
    pub flux_x: Vec<f32>,
    pub flux_y: Vec<f32>,
    pub limiter: Vec<f32>,
    pub next_depth: Vec<f32>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StepReport {
    pub dt: f64,
    pub source_volume: f64,
    pub volume_before: f64,
    pub volume_after: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AdvanceReport {
    pub requested_time: f64,
    pub simulated_time: f64,
    pub dropped_time: f64,
    pub substeps: usize,
    pub minimum_dt: f64,
    pub maximum_dt: f64,
    pub source_volume: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Diagnostics {
    pub time: f64,
    pub steps: u64,
    pub volume: f64,
    pub accounted_source_volume: f64,
    pub minimum_depth: f64,
    pub maximum_depth: f64,
    pub maximum_velocity: f64,
    pub wet_cells: usize,
    pub active_cells: usize,
    pub non_finite: usize,
}

pub struct Solver {
    domain: Domain,
    options: SolverOptions,
    depth: Vec<f32>,
    fields: StepFields,
    time: f64,
    steps: u64,
    accounted_source_volume: f64,
}

impl Solver {
    pub fn new(domain: Domain, mut depth: Vec<f32>) -> Result<Self, DomainError> {
        let n = domain.grid.len();
        if depth.len() != n {
            return Err(DomainError("depth length does not match grid"));
        }
        for (k, value) in depth.iter_mut().enumerate() {
            if !domain.active[k] {
                *value = 0.0;
            } else if !value.is_finite() || *value < 0.0 {
                return Err(DomainError(
                    "active water depths must be finite and non-negative",
                ));
            }
        }
        Ok(Self {
            domain,
            options: SolverOptions::default(),
            depth,
            fields: StepFields {
                velocity_x: vec![0.0; n],
                velocity_y: vec![0.0; n],
                flux_x: vec![0.0; n],
                flux_y: vec![0.0; n],
                limiter: vec![1.0; n],
                next_depth: vec![0.0; n],
            },
            time: 0.0,
            steps: 0,
            accounted_source_volume: 0.0,
        })
    }

    pub fn with_options(mut self, options: SolverOptions) -> Self {
        self.options = options;
        self
    }

    pub fn domain(&self) -> &Domain {
        &self.domain
    }
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }
    pub fn depth_mut(&mut self) -> &mut [f32] {
        &mut self.depth
    }
    pub fn fields(&self) -> &StepFields {
        &self.fields
    }
    pub fn fields_mut(&mut self) -> &mut StepFields {
        &mut self.fields
    }
    pub fn options(&self) -> SolverOptions {
        self.options
    }
    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn volume(&self) -> f64 {
        let area = self.domain.grid.dx * self.domain.grid.dy;
        self.depth
            .iter()
            .zip(&self.domain.active)
            .filter(|(_, active)| **active)
            .map(|(h, _)| f64::from(*h) * area)
            .sum()
    }

    pub fn stable_timestep(&self) -> f64 {
        let mut maximum_rate = 0.0_f64;
        let gravity = self.options.gravity;
        for k in 0..self.depth.len() {
            if !self.domain.active[k] {
                continue;
            }
            let wave = (gravity * f64::from(self.depth[k])).sqrt();
            let ux = f64::from(self.fields.velocity_x[k]).abs() + wave;
            let uy = f64::from(self.fields.velocity_y[k]).abs() + wave;
            maximum_rate = maximum_rate.max(ux / self.domain.grid.dx + uy / self.domain.grid.dy);
        }
        if maximum_rate == 0.0 {
            f64::INFINITY
        } else {
            self.options.cfl / maximum_rate
        }
    }

    /// Advances one already-stability-limited interval. The arithmetic and
    /// f32 store points intentionally mirror Saltreach's `pressure` kernel.
    pub fn step(&mut self, dt: f64) -> StepReport {
        self.step_with_boundaries(dt, &[])
    }

    pub fn step_with_boundaries(&mut self, dt: f64, targets: &[BoundaryTarget]) -> StepReport {
        assert!(dt.is_finite() && dt > 0.0);
        let before = self.volume();
        self.pressure_flux_divergence(dt);
        let source_volume = self.apply_boundary_targets(dt, targets);
        self.time += dt;
        self.steps += 1;
        self.accounted_source_volume += source_volume;
        StepReport {
            dt,
            source_volume,
            volume_before: before,
            volume_after: self.volume(),
        }
    }

    fn pressure_flux_divergence(&mut self, dt: f64) {
        let Grid { nx, ny, dx, dy } = self.domain.grid;
        let o = self.options;
        let bed = &self.domain.bed;
        let h = &self.depth;
        let f = &mut self.fields;

        for j in 0..ny {
            for i in 0..nx {
                let k = j * nx + i;
                if !self.domain.active[k] {
                    f.velocity_x[k] = 0.0;
                    f.velocity_y[k] = 0.0;
                    f.flux_x[k] = 0.0;
                    f.flux_y[k] = 0.0;
                    continue;
                }
                let eta = f64::from(bed[k]) + f64::from(h[k]);
                if self.domain.x_face_open(k) {
                    let q = k + 1;
                    let eta2 = f64::from(bed[q]) + f64::from(h[q]);
                    let crest = f64::from(bed[k].max(bed[q]));
                    let face_h = 0.0_f64.max(eta.max(eta2) - crest);
                    if face_h < o.face_dry_epsilon {
                        f.velocity_x[k] = 0.0;
                        f.flux_x[k] = 0.0;
                    } else {
                        let drag = o.base_drag + o.shallow_drag / (face_h + o.shallow_drag_offset);
                        f.velocity_x[k] = ((f64::from(f.velocity_x[k])
                            - o.gravity * dt * (eta2 - eta) / dx)
                            / (1.0 + dt * drag))
                            .clamp(-o.max_velocity, o.max_velocity)
                            as f32;
                        let speed = f64::from(f.velocity_x[k]);
                        let donor = if speed > 0.0 {
                            0.0_f64.max(eta - crest)
                        } else {
                            0.0_f64.max(eta2 - crest)
                        };
                        f.flux_x[k] = (speed * donor) as f32;
                        if o.artificial_viscosity && h[k] > 0.008 && h[q] > 0.008 {
                            let slope = (eta2 - eta) / dx;
                            let steep = smoothstep(0.12, 0.48, slope.abs());
                            f.flux_x[k] = (f64::from(f.flux_x[k]) - 0.32 * steep * slope) as f32;
                        }
                    }
                } else {
                    f.velocity_x[k] = 0.0;
                    f.flux_x[k] = 0.0;
                }

                if self.domain.y_face_open(k) {
                    let q = k + nx;
                    let eta2 = f64::from(bed[q]) + f64::from(h[q]);
                    let crest = f64::from(bed[k].max(bed[q]));
                    let face_h = 0.0_f64.max(eta.max(eta2) - crest);
                    if face_h < o.face_dry_epsilon {
                        f.velocity_y[k] = 0.0;
                        f.flux_y[k] = 0.0;
                    } else {
                        let drag = o.base_drag + o.shallow_drag / (face_h + o.shallow_drag_offset);
                        f.velocity_y[k] = ((f64::from(f.velocity_y[k])
                            - o.gravity * dt * (eta2 - eta) / dy)
                            / (1.0 + dt * drag))
                            .clamp(-o.max_velocity, o.max_velocity)
                            as f32;
                        let speed = f64::from(f.velocity_y[k]);
                        let donor = if speed > 0.0 {
                            0.0_f64.max(eta - crest)
                        } else {
                            0.0_f64.max(eta2 - crest)
                        };
                        f.flux_y[k] = (speed * donor) as f32;
                        if o.artificial_viscosity && h[k] > 0.008 && h[q] > 0.008 {
                            let slope = (eta2 - eta) / dy;
                            let steep = smoothstep(0.12, 0.48, slope.abs());
                            f.flux_y[k] = (f64::from(f.flux_y[k]) - 0.32 * steep * slope) as f32;
                        }
                    }
                } else {
                    f.velocity_y[k] = 0.0;
                    f.flux_y[k] = 0.0;
                }
            }
        }

        for j in 0..ny {
            for i in 0..nx {
                let k = j * nx + i;
                if !self.domain.active[k] {
                    f.limiter[k] = 1.0;
                    continue;
                }
                let outgoing = (f64::from(f.flux_x[k]).max(0.0)
                    + if i > 0 {
                        (-f64::from(f.flux_x[k - 1])).max(0.0)
                    } else {
                        0.0
                    })
                    / dx
                    + (f64::from(f.flux_y[k]).max(0.0)
                        + if j > 0 {
                            (-f64::from(f.flux_y[k - nx])).max(0.0)
                        } else {
                            0.0
                        })
                        / dy;
                f.limiter[k] = if outgoing > 0.0 {
                    (1.0_f64.min(f64::from(h[k]) / (dt * outgoing + 1.0e-10))) as f32
                } else {
                    1.0
                };
            }
        }

        for k in 0..h.len() {
            if self.domain.x_face_open(k) {
                let donor = if f.flux_x[k] >= 0.0 { k } else { k + 1 };
                f.flux_x[k] = (f64::from(f.flux_x[k]) * f64::from(f.limiter[donor])) as f32;
            } else {
                f.flux_x[k] = 0.0;
            }
            if self.domain.y_face_open(k) {
                let donor = if f.flux_y[k] >= 0.0 { k } else { k + nx };
                f.flux_y[k] = (f64::from(f.flux_y[k]) * f64::from(f.limiter[donor])) as f32;
            } else {
                f.flux_y[k] = 0.0;
            }
        }

        for j in 0..ny {
            for i in 0..nx {
                let k = j * nx + i;
                if !self.domain.active[k] {
                    f.next_depth[k] = 0.0;
                    continue;
                }
                let divergence = (f64::from(f.flux_x[k])
                    - if i > 0 {
                        f64::from(f.flux_x[k - 1])
                    } else {
                        0.0
                    })
                    / dx
                    + (f64::from(f.flux_y[k])
                        - if j > 0 {
                            f64::from(f.flux_y[k - nx])
                        } else {
                            0.0
                        })
                        / dy;
                f.next_depth[k] = (f64::from(h[k]) - dt * divergence).max(0.0) as f32;
            }
        }
        std::mem::swap(&mut self.depth, &mut self.fields.next_depth);
    }

    fn apply_boundary_targets(&mut self, dt: f64, targets: &[BoundaryTarget]) -> f64 {
        let area = self.domain.grid.dx * self.domain.grid.dy;
        let mut source = 0.0;
        for target in targets {
            let k = target.cell;
            if k >= self.depth.len() || !self.domain.active[k] || target.rate <= 0.0 {
                continue;
            }
            let old = f64::from(self.depth[k]);
            let desired = (target.surface - f64::from(self.domain.bed[k])).max(0.0);
            let blend = 1.0 - (-dt * target.rate).exp();
            let new = old + (desired - old) * blend;
            self.depth[k] = new.max(0.0) as f32;
            self.fields.velocity_x[k] = (f64::from(self.fields.velocity_x[k])
                + (target.velocity_x - f64::from(self.fields.velocity_x[k])) * blend)
                as f32;
            self.fields.velocity_y[k] = (f64::from(self.fields.velocity_y[k])
                + (target.velocity_y - f64::from(self.fields.velocity_y[k]))
                    * blend
                    * target.transverse_velocity_blend.clamp(0.0, 1.0))
                as f32;
            source += (f64::from(self.depth[k]) - old) * area;
        }
        source
    }

    pub fn diagnostics(&self) -> Diagnostics {
        let mut d = Diagnostics {
            time: self.time,
            steps: self.steps,
            volume: self.volume(),
            accounted_source_volume: self.accounted_source_volume,
            minimum_depth: f64::INFINITY,
            ..Diagnostics::default()
        };
        for k in 0..self.depth.len() {
            if !self.domain.active[k] {
                continue;
            }
            d.active_cells += 1;
            let h = f64::from(self.depth[k]);
            let ux = f64::from(self.fields.velocity_x[k]);
            let uy = f64::from(self.fields.velocity_y[k]);
            if !h.is_finite() || !ux.is_finite() || !uy.is_finite() {
                d.non_finite += 1;
                continue;
            }
            d.minimum_depth = d.minimum_depth.min(h);
            d.maximum_depth = d.maximum_depth.max(h);
            d.maximum_velocity = d.maximum_velocity.max(ux.hypot(uy));
            if h >= 0.0005 {
                d.wet_cells += 1;
            }
        }
        if d.minimum_depth == f64::INFINITY {
            d.minimum_depth = 0.0;
        }
        d
    }
}

pub struct FrameStepper {
    pub maximum_substeps: usize,
    pub maximum_frame_time: f64,
}

impl Default for FrameStepper {
    fn default() -> Self {
        Self {
            maximum_substeps: 8,
            maximum_frame_time: 0.25,
        }
    }
}

impl FrameStepper {
    pub fn advance(&self, solver: &mut Solver, frame_time: f64) -> AdvanceReport {
        let requested = frame_time.max(0.0);
        let accepted = requested.min(self.maximum_frame_time);
        let mut remaining = accepted;
        let mut report = AdvanceReport {
            requested_time: requested,
            dropped_time: requested - accepted,
            minimum_dt: f64::INFINITY,
            ..AdvanceReport::default()
        };
        while remaining > 0.0 && report.substeps < self.maximum_substeps {
            let stable = solver.stable_timestep().max(solver.options.minimum_step);
            let dt = remaining.min(stable);
            let step = solver.step(dt);
            remaining -= dt;
            report.simulated_time += dt;
            report.substeps += 1;
            report.minimum_dt = report.minimum_dt.min(dt);
            report.maximum_dt = report.maximum_dt.max(dt);
            report.source_volume += step.source_volume;
        }
        report.dropped_time += remaining.max(0.0);
        if report.substeps == 0 {
            report.minimum_dt = 0.0;
        }
        report
    }
}

#[inline]
fn smoothstep(a: f64, b: f64, value: f64) -> f64 {
    let t = ((value - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

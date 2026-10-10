//! Deterministic synthetic validation and release-build performance probes.

use super::{BoundaryTarget, Domain, FrameStepper, Grid, Solver, SolverOptions};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct HistoryPoint {
    pub time: f64,
    pub volume: f64,
    pub source_volume: f64,
    pub minimum_depth: f64,
    pub maximum_depth: f64,
    pub maximum_velocity: f64,
}

#[derive(Clone, Debug)]
pub struct CaseResult {
    pub name: &'static str,
    pub passed: bool,
    pub duration: f64,
    pub steps: u64,
    pub relative_volume_error: f64,
    pub minimum_depth: f64,
    pub maximum_depth: f64,
    pub maximum_velocity: f64,
    pub non_finite: usize,
    pub primary_error: f64,
    pub tolerance: f64,
    pub note: String,
    pub history: Vec<HistoryPoint>,
    pub snapshot: Option<(usize, usize, Vec<f32>)>,
}

#[derive(Clone, Debug)]
pub struct PerformanceResult {
    pub nx: usize,
    pub ny: usize,
    pub active_cells: usize,
    pub iterations: usize,
    pub milliseconds_per_step: f64,
    pub minimum_milliseconds_per_step: f64,
    pub maximum_milliseconds_per_step: f64,
    pub milliseconds_per_simulated_second: f64,
    pub estimated_bytes: usize,
}

pub fn run_all() -> Vec<CaseResult> {
    vec![
        flat_closed_basin(),
        sloping_bed_at_rest(),
        raised_dry_terrain(),
        small_wave_convergence(),
        vertical_wall(),
        irregular_footprint(),
        wet_front(),
        boundary_forcing(),
        large_frame_gap(),
    ]
}

pub fn performance_suite() -> Vec<PerformanceResult> {
    [
        (64, 64, 1.0),
        (128, 128, 0.5),
        (256, 256, 0.25),
        (256, 256, 0.9),
        (512, 512, 0.25),
    ]
    .into_iter()
    .map(|(nx, ny, active_fraction)| {
        let grid = Grid::new(nx, ny, 0.4, 0.4).unwrap();
        let n = grid.len();
        let mut active = vec![false; n];
        let active_columns = (nx as f64 * active_fraction).round().max(1.0) as usize;
        for j in 0..ny {
            for i in 0..active_columns {
                active[j * nx + i] = true;
            }
        }
        let active_cells = active.iter().filter(|v| **v).count();
        let domain =
            Domain::with_geometry(grid, vec![-1.5; n], active, vec![false; n], vec![false; n])
                .unwrap();
        let mut depth = vec![1.5; n];
        for j in 0..ny {
            for i in 0..nx {
                depth[j * nx + i] +=
                    (0.01 * ((i as f64 * 0.17).sin() * (j as f64 * 0.11).cos())) as f32;
            }
        }
        let mut solver = Solver::new(domain, depth).unwrap();
        for _ in 0..8 {
            solver.step(1.0 / 60.0);
        }
        let iterations = if nx >= 512 {
            20
        } else if nx >= 256 {
            80
        } else {
            240
        };
        let mut samples = [0.0_f64; 5];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..iterations {
                solver.step(1.0 / 60.0);
            }
            *sample = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;
        }
        samples.sort_by(f64::total_cmp);
        let milliseconds_per_step = samples[2];
        PerformanceResult {
            nx,
            ny,
            active_cells,
            iterations,
            milliseconds_per_step,
            minimum_milliseconds_per_step: samples[0],
            maximum_milliseconds_per_step: samples[4],
            milliseconds_per_simulated_second: milliseconds_per_step * 60.0,
            // 6 f32 scratch/state arrays + depth + bed, 3 boolean geometry arrays. Vec
            // allocator overhead is intentionally excluded.
            estimated_bytes: n * (8 * std::mem::size_of::<f32>() + 3 * std::mem::size_of::<bool>()),
        }
    })
    .collect()
}

fn options_without_dissipation() -> SolverOptions {
    SolverOptions {
        base_drag: 0.0,
        shallow_drag: 0.0,
        artificial_viscosity: false,
        max_velocity: 20.0,
        ..SolverOptions::default()
    }
}

fn sample(solver: &Solver, initial_volume: f64, source: f64) -> HistoryPoint {
    let d = solver.diagnostics();
    let _balance = d.volume - initial_volume - source;
    HistoryPoint {
        time: d.time,
        volume: d.volume,
        source_volume: source,
        minimum_depth: d.minimum_depth,
        maximum_depth: d.maximum_depth,
        maximum_velocity: d.maximum_velocity,
    }
}

#[allow(clippy::too_many_arguments)]
fn finish(
    name: &'static str,
    solver: &Solver,
    initial_volume: f64,
    source_volume: f64,
    primary_error: f64,
    tolerance: f64,
    extra_pass: bool,
    note: String,
    history: Vec<HistoryPoint>,
    snapshot: bool,
) -> CaseResult {
    let d = solver.diagnostics();
    let scale = initial_volume.abs().max(1.0);
    let relative_volume_error = (d.volume - initial_volume - source_volume).abs() / scale;
    CaseResult {
        name,
        passed: extra_pass
            && d.non_finite == 0
            && d.minimum_depth >= 0.0
            && relative_volume_error <= 2.0e-5
            && primary_error <= tolerance,
        duration: d.time,
        steps: d.steps,
        relative_volume_error,
        minimum_depth: d.minimum_depth,
        maximum_depth: d.maximum_depth,
        maximum_velocity: d.maximum_velocity,
        non_finite: d.non_finite,
        primary_error,
        tolerance,
        note,
        history,
        snapshot: snapshot.then(|| {
            (
                solver.domain().grid.nx,
                solver.domain().grid.ny,
                solver.depth().to_vec(),
            )
        }),
    }
}

fn flat_closed_basin() -> CaseResult {
    let grid = Grid::new(24, 16, 0.4, 0.4).unwrap();
    let domain = Domain::new(grid, vec![-2.0; grid.len()]).unwrap();
    let mut solver = Solver::new(domain, vec![2.0; grid.len()]).unwrap();
    let initial = solver.volume();
    let mut history = vec![sample(&solver, initial, 0.0)];
    for step in 0..240 {
        solver.step(1.0 / 120.0);
        if step % 30 == 29 {
            history.push(sample(&solver, initial, 0.0));
        }
    }
    let error = solver
        .depth()
        .iter()
        .map(|h| (f64::from(*h) - 2.0).abs())
        .fold(0.0, f64::max);
    finish(
        "flat_closed_basin",
        &solver,
        initial,
        0.0,
        error,
        1.0e-7,
        true,
        "2 m still water; closed reflective perimeter".into(),
        history,
        false,
    )
}

fn sloping_bed_at_rest() -> CaseResult {
    let grid = Grid::new(32, 18, 0.25, 0.25).unwrap();
    let mut bed = vec![0.0; grid.len()];
    let mut depth = vec![0.0; grid.len()];
    for j in 0..grid.ny {
        for i in 0..grid.nx {
            let k = j * grid.nx + i;
            bed[k] = -2.2 + i as f32 * 0.018 + j as f32 * 0.011 + if i >= 16 { 0.25 } else { 0.0 };
            depth[k] = -bed[k];
        }
    }
    let domain = Domain::new(grid, bed).unwrap();
    let mut solver = Solver::new(domain, depth).unwrap();
    let initial = solver.volume();
    for _ in 0..300 {
        solver.step(1.0 / 180.0);
    }
    let max_surface = solver
        .domain()
        .bed
        .iter()
        .zip(solver.depth())
        .map(|(b, h)| f64::from(b + h).abs())
        .fold(0.0, f64::max);
    finish(
        "sloping_stepped_bed_at_rest",
        &solver,
        initial,
        0.0,
        max_surface,
        2.0e-7,
        true,
        "sloping bed with a 0.25 m step and a common zero-metre free surface".into(),
        vec![],
        false,
    )
}

fn raised_dry_terrain() -> CaseResult {
    let grid = Grid::new(60, 4, 0.15, 0.15).unwrap();
    let mut bed = vec![-1.0; grid.len()];
    for j in 0..grid.ny {
        for i in 0..grid.nx {
            let k = j * grid.nx + i;
            if (22..=24).contains(&i) {
                bed[k] = 0.35;
            }
        }
    }
    let domain = Domain::new(grid, bed).unwrap();

    let mut low_depth = vec![0.0; grid.len()];
    for j in 0..grid.ny {
        for i in 0..22 {
            low_depth[j * grid.nx + i] = 1.0;
        }
    }
    let mut low_solver = Solver::new(domain.clone(), low_depth).unwrap();
    for _ in 0..300 {
        low_solver.step(low_solver.stable_timestep().min(0.0035));
    }
    let low_leak = (0..grid.ny)
        .flat_map(|j| (25..grid.nx).map(move |i| (j, i)))
        .map(|(j, i)| f64::from(low_solver.depth()[j * grid.nx + i]))
        .sum::<f64>();

    let mut depth = vec![0.0; grid.len()];
    for j in 0..grid.ny {
        for i in 0..22 {
            depth[j * grid.nx + i] = 1.75;
        }
    }
    let mut solver = Solver::new(domain, depth).unwrap();
    let initial = solver.volume();
    let mut history = vec![sample(&solver, initial, 0.0)];
    for step in 0..500 {
        let dt = solver.stable_timestep().min(0.0035);
        solver.step(dt);
        if step % 50 == 49 {
            history.push(sample(&solver, initial, 0.0));
        }
    }
    let far_wet = (0..grid.ny).any(|j| solver.depth()[j * grid.nx + 35] > 0.001);
    let error = solver.depth().iter().filter(|h| **h < 0.0).count() as f64;
    finish(
        "raised_dry_terrain",
        &solver,
        initial,
        0.0,
        error.max(low_leak),
        0.0,
        far_wet,
        format!("zero-metre surface leak={low_leak:.3e}; 0.75 m surface overtops the 0.35 m sill without negative depth"),
        history,
        true,
    )
}

fn gaussian_error(nx: usize) -> (f64, f64, Solver) {
    let length = 24.0;
    let dx = length / nx as f64;
    let grid = Grid::new(nx, 2, dx, 0.5).unwrap();
    let bed = vec![-1.0; grid.len()];
    let amplitude = 0.002;
    let sigma = 0.8;
    let center = length * 0.5;
    let mut depth = vec![0.0; grid.len()];
    for j in 0..2 {
        for i in 0..nx {
            let x = (i as f64 + 0.5) * dx;
            depth[j * nx + i] = (1.0 + amplitude * (-((x - center) / sigma).powi(2)).exp()) as f32;
        }
    }
    let domain = Domain::new(grid, bed).unwrap();
    let mut solver = Solver::new(domain, depth)
        .unwrap()
        .with_options(options_without_dissipation());
    let initial_volume = solver.volume();
    let end = 0.45;
    while solver.time() < end - 1.0e-12 {
        let dt = solver
            .stable_timestep()
            .min(0.18 * dx / (9.81_f64).sqrt())
            .min(end - solver.time());
        solver.step(dt);
    }
    let c = 9.81_f64.sqrt();
    let mut squared = 0.0;
    for i in 0..nx {
        let x = (i as f64 + 0.5) * dx;
        let exact = 1.0
            + amplitude
                * 0.5
                * ((-((x - center - c * end) / sigma).powi(2)).exp()
                    + (-((x - center + c * end) / sigma).powi(2)).exp());
        squared += (f64::from(solver.depth()[i]) - exact).powi(2);
    }
    ((squared / nx as f64).sqrt(), initial_volume, solver)
}

fn small_wave_convergence() -> CaseResult {
    let (coarse, _, _) = gaussian_error(120);
    let (fine, initial, solver) = gaussian_error(240);
    let d = solver.diagnostics();
    finish(
        "small_wave_convergence",
        &solver,
        initial,
        0.0,
        fine,
        3.0e-4,
        fine < coarse * 0.8 && d.non_finite == 0,
        format!(
            "linear Gaussian wave RMS: coarse={coarse:.6e}, fine={fine:.6e}, ratio={:.3}",
            fine / coarse
        ),
        vec![],
        true,
    )
}

fn vertical_wall() -> CaseResult {
    let grid = Grid::new(72, 6, 0.2, 0.2).unwrap();
    let n = grid.len();
    let mut walls = vec![false; n];
    let barrier_i = 35;
    for j in 0..grid.ny {
        walls[j * grid.nx + barrier_i] = true;
    }
    let domain =
        Domain::with_geometry(grid, vec![-1.0; n], vec![true; n], walls, vec![false; n]).unwrap();
    let mut depth = vec![1.0; n];
    for j in 0..grid.ny {
        for i in 8..18 {
            depth[j * grid.nx + i] += 0.08;
        }
    }
    let mut solver = Solver::new(domain, depth).unwrap();
    let initial = solver.volume();
    let right_before: f64 = (0..grid.ny)
        .flat_map(|j| (barrier_i + 1..grid.nx).map(move |i| (j, i)))
        .map(|(j, i)| f64::from(solver.depth()[j * grid.nx + i]))
        .sum();
    let mut peak = 0.0_f64;
    let mut wall_arrival = 1.0_f64;
    for _ in 0..1000 {
        let dt = solver.stable_timestep().min(0.0025);
        solver.step(dt);
        peak = peak.max(solver.diagnostics().maximum_velocity);
        for j in 0..grid.ny {
            wall_arrival = wall_arrival.max(f64::from(solver.depth()[j * grid.nx + barrier_i]));
        }
    }
    let right_after: f64 = (0..grid.ny)
        .flat_map(|j| (barrier_i + 1..grid.nx).map(move |i| (j, i)))
        .map(|(j, i)| f64::from(solver.depth()[j * grid.nx + i]))
        .sum();
    let leak = (right_after - right_before).abs();
    let wall_flux = (0..grid.ny)
        .map(|j| f64::from(solver.fields().flux_x[j * grid.nx + barrier_i]).abs())
        .fold(0.0, f64::max);
    finish(
        "vertical_and_thin_wall",
        &solver,
        initial,
        0.0,
        leak.max(wall_flux),
        1.0e-7,
        peak > 0.01 && wall_arrival > 1.001,
        format!(
            "one-face barrier, reflected/arriving crest={wall_arrival:.6} m, right-basin depth-sum change={leak:.3e}, wall flux={wall_flux:.3e}"
        ),
        vec![],
        true,
    )
}

fn irregular_footprint() -> CaseResult {
    let grid = Grid::new(48, 32, 0.25, 0.25).unwrap();
    let n = grid.len();
    let mut active = vec![true; n];
    for j in 0..grid.ny {
        active[j * grid.nx + 24] = false;
        for i in 9..15 {
            if (i as isize - 12).pow(2) + (j as isize - 16).pow(2) <= 9 {
                active[j * grid.nx + i] = false;
            }
        }
    }
    let domain = Domain::with_geometry(
        grid,
        vec![-1.0; n],
        active.clone(),
        vec![false; n],
        vec![false; n],
    )
    .unwrap();
    let mut depth = vec![1.0; n];
    for k in 0..n {
        if !active[k] {
            depth[k] = 0.0;
        }
    }
    for j in 0..grid.ny {
        for i in 2..9 {
            depth[j * grid.nx + i] += 0.05;
        }
    }
    let mut solver = Solver::new(domain, depth).unwrap();
    let initial = solver.volume();
    let right_before: f64 = (0..grid.ny)
        .flat_map(|j| (25..grid.nx).map(move |i| (j, i)))
        .map(|(j, i)| f64::from(solver.depth()[j * grid.nx + i]))
        .sum();
    for _ in 0..600 {
        solver.step(solver.stable_timestep().min(0.003));
    }
    let right_after: f64 = (0..grid.ny)
        .flat_map(|j| (25..grid.nx).map(move |i| (j, i)))
        .map(|(j, i)| f64::from(solver.depth()[j * grid.nx + i]))
        .sum();
    let inactive_water = active
        .iter()
        .zip(solver.depth())
        .filter(|(a, _)| !**a)
        .map(|(_, h)| f64::from(*h))
        .sum::<f64>();
    let error = (right_after - right_before).abs().max(inactive_water);
    finish(
        "irregular_footprint_island_separated_basins",
        &solver,
        initial,
        0.0,
        error,
        1.0e-7,
        true,
        "inactive island and one-cell land strip remain impermeable".into(),
        vec![],
        true,
    )
}

fn wet_front() -> CaseResult {
    let grid = Grid::new(100, 4, 0.1, 0.1).unwrap();
    let domain = Domain::new(grid, vec![0.0; grid.len()]).unwrap();
    let mut depth = vec![0.0; grid.len()];
    for j in 0..grid.ny {
        for i in 0..30 {
            depth[j * grid.nx + i] = 1.0;
        }
    }
    let mut solver = Solver::new(domain, depth).unwrap();
    let initial = solver.volume();
    let mut history = vec![sample(&solver, initial, 0.0)];
    while solver.time() < 0.7 {
        let dt = solver.stable_timestep().min(0.002).min(0.7 - solver.time());
        solver.step(dt);
        if solver.diagnostics().steps.is_multiple_of(50) {
            history.push(sample(&solver, initial, 0.0));
        }
    }
    let front = (0..grid.nx)
        .rev()
        .find(|i| (0..grid.ny).any(|j| solver.depth()[j * grid.nx + *i] > 0.001))
        .unwrap_or(0);
    finish(
        "wet_front_dam_break",
        &solver,
        initial,
        0.0,
        0.0,
        0.0,
        front > 35,
        format!("positive wet front advanced from cell 29 to cell {front}"),
        history,
        true,
    )
}

fn boundary_forcing() -> CaseResult {
    let grid = Grid::new(64, 4, 0.2, 0.2).unwrap();
    let domain = Domain::new(grid, vec![-1.0; grid.len()]).unwrap();
    let mut solver = Solver::new(domain, vec![1.0; grid.len()]).unwrap();
    let initial = solver.volume();
    let mut source = 0.0;
    let mut history = vec![sample(&solver, initial, source)];
    let end = 2.0;
    while solver.time() < end - 1.0e-12 {
        let dt = solver
            .stable_timestep()
            .min(1.0 / 240.0)
            .min(end - solver.time());
        let t = solver.time() + dt;
        let eta = 0.025 * (2.0 * std::f64::consts::PI * 0.75 * t).sin();
        let wave_u = -eta * (9.81_f64).sqrt();
        let mut targets = Vec::with_capacity(grid.ny * 2);
        for j in 0..grid.ny {
            targets.push(BoundaryTarget {
                cell: j * grid.nx,
                surface: eta,
                rate: 9.0,
                velocity_x: wave_u,
                velocity_y: 0.0,
                transverse_velocity_blend: 0.5,
            });
            targets.push(BoundaryTarget {
                cell: j * grid.nx + grid.nx - 1,
                surface: 0.0,
                rate: 5.0,
                velocity_x: 0.0,
                velocity_y: 0.0,
                transverse_velocity_blend: 0.5,
            });
        }
        source += solver.step_with_boundaries(dt, &targets).source_volume;
        if solver.diagnostics().steps.is_multiple_of(60) {
            history.push(sample(&solver, initial, source));
        }
    }
    let balance = (solver.volume() - initial - source).abs() / initial;
    finish(
        "boundary_forcing_and_sponge",
        &solver,
        initial,
        source,
        balance,
        2.0e-5,
        solver.diagnostics().maximum_depth < 1.2,
        format!("0.75 Hz sinusoidal boundary; explicit cumulative source={source:.6e} m^3"),
        history,
        true,
    )
}

fn large_frame_gap() -> CaseResult {
    let grid = Grid::new(48, 4, 0.1, 0.1).unwrap();
    let domain = Domain::new(grid, vec![-1.0; grid.len()]).unwrap();
    let mut depth = vec![1.0; grid.len()];
    for j in 0..grid.ny {
        depth[j * grid.nx + 12] += 0.1;
    }
    let mut solver = Solver::new(domain, depth).unwrap();
    let initial = solver.volume();
    let stepper = FrameStepper {
        maximum_substeps: 6,
        maximum_frame_time: 0.25,
    };
    let report = stepper.advance(&mut solver, 2.0);
    let finite = solver.diagnostics().non_finite == 0;
    let bounded =
        report.substeps == 6 && report.dropped_time > 1.7 && report.simulated_time <= 0.25;
    finish(
        "large_timestep_frame_gap",
        &solver,
        initial,
        0.0,
        0.0,
        0.0,
        finite && bounded,
        format!(
            "2 s gap: {} substeps, {:.6} s simulated, {:.6} s dropped",
            report.substeps, report.simulated_time, report.dropped_time
        ),
        vec![],
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::super::{reconstruct_surface, SurfaceSample};
    use super::*;

    #[test]
    fn synthetic_validation_suite_passes() {
        let results = run_all();
        let failures: Vec<_> = results
            .iter()
            .filter(|case| !case.passed)
            .map(|case| {
                format!(
                    "{}: error={} tol={} volume={} {}",
                    case.name,
                    case.primary_error,
                    case.tolerance,
                    case.relative_volume_error,
                    case.note
                )
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn surface_reconstruction_does_not_cross_wall() {
        let grid = Grid::new(3, 2, 1.0, 1.0).unwrap();
        let mut walls = vec![false; grid.len()];
        walls[0] = true;
        walls[1] = true;
        walls[3] = true;
        walls[4] = true;
        let domain =
            Domain::with_geometry(grid, vec![0.0; 6], vec![true; 6], walls, vec![false; 6])
                .unwrap();
        let solver = Solver::new(domain, vec![1.0, 0.0, 2.0, 1.0, 0.0, 2.0]).unwrap();
        let mut surface = vec![SurfaceSample::default(); 6];
        reconstruct_surface(&solver, &mut surface);
        assert!(
            surface[1].elevation < 0.0,
            "dry cell must not inherit through a closed face"
        );
        assert_eq!(surface[0].slope_x, 0.0);
    }

    #[test]
    #[allow(clippy::excessive_precision)]
    fn saltreach_reference_pressure_fixture_matches() {
        let grid = Grid::new(4, 3, 0.3, 0.4).unwrap();
        let bed = vec![
            -1.0, -0.8, -0.2, 0.15, -1.1, -0.7, -0.1, 0.25, -1.2, -0.9, -0.3, 0.35,
        ];
        let depth = vec![
            1.05, 0.83, 0.24, 0.0, 1.08, 0.76, 0.14, 0.0, 1.18, 0.91, 0.27, 0.0,
        ];
        let mut solver = Solver::new(Domain::new(grid, bed).unwrap(), depth).unwrap();
        solver.fields_mut().velocity_x.copy_from_slice(&[
            0.11, -0.07, 0.03, 0.0, -0.04, 0.09, -0.02, 0.0, 0.06, -0.05, 0.01, 0.0,
        ]);
        solver.fields_mut().velocity_y.copy_from_slice(&[
            -0.03, 0.02, 0.04, 0.0, 0.05, -0.06, 0.01, 0.0, 0.0, 0.0, 0.0, 0.0,
        ]);
        solver.step(0.025);
        // Generated by scripts/generate-coastal-reference.mjs from the exact
        // Saltreach kernel order.
        let expected_u = [
            0.125873283,
            -0.0775565654,
            0.0,
            0.0,
            -0.104977950,
            0.105282895,
            0.0,
            0.0,
            0.0353456698,
            -0.0171827748,
            0.0,
            0.0,
        ];
        let expected_v = [
            0.0128748994,
            0.00159979903,
            0.0395676270,
            0.0,
            0.0498328060,
            -0.0292262826,
            0.0523467399,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        let expected_flux_x = [
            0.106992282,
            -0.0186135750,
            0.0,
            0.0,
            -0.110733509,
            0.0168452635,
            0.0,
            0.0,
            0.0311041847,
            -0.00446809735,
            0.0,
            0.0,
        ];
        let expected_flux_y = [
            0.0170405246,
            0.00116785325,
            0.00553946756,
            0.0,
            0.0538194329,
            -0.0207277294,
            0.0108504491,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        let expected_limiter = [1.0; 12];
        let expected_depth = [
            1.04001892,
            0.840394139,
            0.238102645,
            0.0,
            1.08692920,
            0.750736892,
            0.141071841,
            0.0,
            1.18077159,
            0.911668897,
            0.270305812,
            0.0,
        ];
        assert_eq!(solver.fields().velocity_x, expected_u);
        assert_eq!(solver.fields().velocity_y, expected_v);
        assert_eq!(solver.fields().flux_x, expected_flux_x);
        assert_eq!(solver.fields().flux_y, expected_flux_y);
        assert_eq!(solver.fields().limiter, expected_limiter);
        assert_eq!(solver.depth(), expected_depth);
    }
}

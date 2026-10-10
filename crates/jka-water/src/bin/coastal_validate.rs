use jka_water::coastal::validation::{performance_suite, run_all, CaseResult};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

fn main() -> io::Result<()> {
    let mut output = PathBuf::from("target/coastal-checkpoint2/validation");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--output" {
            output = PathBuf::from(args.next().expect("--output requires a path"));
        } else {
            panic!("unknown argument: {arg}");
        }
    }
    fs::create_dir_all(&output)?;
    let results = run_all();
    let performance = performance_suite();
    write_summary(&output.join("summary.json"), &results, &performance)?;
    write_histories(&output.join("histories.csv"), &results)?;
    write_volume_svg(&output.join("volume-balance.svg"), &results)?;
    for case in &results {
        if let Some((nx, ny, depth)) = &case.snapshot {
            write_pgm(&output.join(format!("{}.pgm", case.name)), *nx, *ny, depth)?;
        }
    }
    for case in &results {
        println!(
            "{:<42} {} error={:.4e} tol={:.4e} volume={:.4e} {}",
            case.name,
            if case.passed { "PASS" } else { "FAIL" },
            case.primary_error,
            case.tolerance,
            case.relative_volume_error,
            case.note
        );
    }
    for p in &performance {
        println!(
            "perf {:>3}x{:<3} active={:<6} {:.4} ms/step median [{:.4}, {:.4}] {:.2} ms/sim-s memory={} bytes",
            p.nx,
            p.ny,
            p.active_cells,
            p.milliseconds_per_step,
            p.minimum_milliseconds_per_step,
            p.maximum_milliseconds_per_step,
            p.milliseconds_per_simulated_second,
            p.estimated_bytes
        );
    }
    if results.iter().any(|case| !case.passed) {
        std::process::exit(1);
    }
    Ok(())
}

fn esc(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn write_summary(
    path: &Path,
    results: &[CaseResult],
    performance: &[jka_water::coastal::validation::PerformanceResult],
) -> io::Result<()> {
    let mut out = String::from("{\n  \"schema\": 1,\n  \"units\": {\"distance\": \"metre\", \"time\": \"second\", \"volume\": \"cubic metre\"},\n  \"cases\": [\n");
    for (index, c) in results.iter().enumerate() {
        out.push_str(&format!("    {{\"name\":\"{}\",\"passed\":{},\"duration\":{:.9},\"steps\":{},\"relative_volume_error\":{:.12e},\"minimum_depth\":{:.9},\"maximum_depth\":{:.9},\"maximum_velocity\":{:.9},\"non_finite\":{},\"primary_error\":{:.12e},\"tolerance\":{:.12e},\"note\":\"{}\"}}{}\n",
            c.name, c.passed, c.duration, c.steps, c.relative_volume_error, c.minimum_depth,
            c.maximum_depth, c.maximum_velocity, c.non_finite, c.primary_error, c.tolerance,
            esc(&c.note), if index + 1 == results.len() { "" } else { "," }));
    }
    out.push_str("  ],\n  \"performance\": [\n");
    for (index, p) in performance.iter().enumerate() {
        out.push_str(&format!("    {{\"nx\":{},\"ny\":{},\"active_cells\":{},\"iterations_per_sample\":{},\"samples\":5,\"median_milliseconds_per_step\":{:.9},\"minimum_milliseconds_per_step\":{:.9},\"maximum_milliseconds_per_step\":{:.9},\"median_milliseconds_per_simulated_second\":{:.9},\"estimated_bytes\":{}}}{}\n",
            p.nx, p.ny, p.active_cells, p.iterations, p.milliseconds_per_step,
            p.minimum_milliseconds_per_step, p.maximum_milliseconds_per_step,
            p.milliseconds_per_simulated_second, p.estimated_bytes,
            if index + 1 == performance.len() { "" } else { "," }));
    }
    out.push_str("  ]\n}\n");
    fs::write(path, out)
}

fn write_histories(path: &Path, results: &[CaseResult]) -> io::Result<()> {
    let mut out = String::from(
        "case,time_s,volume_m3,accounted_source_m3,min_depth_m,max_depth_m,max_velocity_m_s\n",
    );
    for case in results {
        for h in &case.history {
            out.push_str(&format!(
                "{},{:.9},{:.12e},{:.12e},{:.9},{:.9},{:.9}\n",
                case.name,
                h.time,
                h.volume,
                h.source_volume,
                h.minimum_depth,
                h.maximum_depth,
                h.maximum_velocity
            ));
        }
    }
    fs::write(path, out)
}

fn write_volume_svg(path: &Path, results: &[CaseResult]) -> io::Result<()> {
    let width = 960.0;
    let height = 520.0;
    let mut out = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><rect width=\"100%\" height=\"100%\" fill=\"#10151d\"/><text x=\"40\" y=\"32\" fill=\"#e8eef8\" font-family=\"sans-serif\" font-size=\"18\">Closed/source-accounted volume error</text>");
    let max = results
        .iter()
        .map(|c| c.relative_volume_error.max(1e-12).log10())
        .fold(-12.0, f64::max)
        .max(-4.0);
    for (i, case) in results.iter().enumerate() {
        let y = 58.0 + i as f64 * 48.0;
        let value = case.relative_volume_error.max(1e-12).log10();
        let bar = ((value + 12.0) / (max + 12.0)).clamp(0.0, 1.0) * 510.0;
        out.push_str(&format!("<text x=\"40\" y=\"{:.1}\" fill=\"#b9c7dc\" font-family=\"monospace\" font-size=\"12\">{}</text><rect x=\"360\" y=\"{:.1}\" width=\"{:.1}\" height=\"16\" fill=\"{}\"/><text x=\"880\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#e8eef8\" font-family=\"monospace\" font-size=\"12\">{:.2e}</text>", y, case.name, y - 13.0, bar.max(1.0), if case.passed { "#46c2a0" } else { "#ee6b6e" }, y, case.relative_volume_error));
    }
    out.push_str("</svg>\n");
    fs::write(path, out)
}

fn write_pgm(path: &Path, nx: usize, ny: usize, depth: &[f32]) -> io::Result<()> {
    let max = depth.iter().copied().fold(0.0_f32, f32::max).max(1.0e-9);
    let mut out = format!("P2\n{} {}\n255\n", nx, ny);
    for j in (0..ny).rev() {
        for i in 0..nx {
            out.push_str(&format!(
                "{} ",
                (depth[j * nx + i] / max * 255.0).clamp(0.0, 255.0) as u8
            ));
        }
        out.push('\n');
    }
    fs::write(path, out)
}

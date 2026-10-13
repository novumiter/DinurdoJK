//! One-command GPU sky breakdown for the Unified renderer. Only active during an
//! explicit benchmark. The shader probe is intentionally not a shipping quality
//! mode; normal mode never changes appearance or pipeline layout.
use crate::renderer::{FrameInfo, GpuPass, Renderer};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const CASES: [&str; 6] = [
    "normal_first", "solid_no_texture", "one_constant_texel",
    "one_face_varying_uv", "fastsky_no_draw", "normal_repeat",
];

#[derive(Clone, Copy)]
struct Row {
    frame_ms: f64,
    acquire_ms: f64,
    encode_ms: f64,
    submit_ms: f64,
    gpu_frame_ms: Option<f64>,
    gpu_world_ms: Option<f64>,
    gpu_post_ms: Option<f64>,
    world_encoded_batches: u32,
}

pub(in crate::renderer) struct SkyPerfSweep {
    stem: PathBuf,
    frames_per_case: usize,
    case: usize,
    armed_at: Instant,
    warmup_remaining: usize,
    rows: [Vec<Row>; 6],
    original_fast_sky: u8,
    original_world_probe: u8,
    original_sky_probe: u8,
    adapter: String,
    backend: String,
    driver: String,
    gpu_timestamps_available: bool,
    resolution: [u32; 2],
}

impl SkyPerfSweep {
    pub(in crate::renderer) fn new(stem: PathBuf, frames_per_case: usize, renderer: &mut Renderer) -> Self {
        let adapter = renderer.adapter.get_info();
        let mut sweep = Self {
            stem,
            frames_per_case: frames_per_case.clamp(60, 1000),
            case: 0,
            armed_at: Instant::now(),
            warmup_remaining: 120,
            rows: std::array::from_fn(|_| Vec::with_capacity(frames_per_case)),
            original_fast_sky: renderer.fast_sky,
            original_world_probe: renderer.world_stage_probe,
            original_sky_probe: renderer.sky_perf_probe,
            adapter: adapter.name,
            backend: format!("{:?}", adapter.backend),
            driver: format!("{} {}", adapter.driver, adapter.driver_info),
            gpu_timestamps_available: renderer.gpu_profiler.enabled(),
            resolution: [renderer.config.width, renderer.config.height],
        };
        // This is the existing authored BSP draw path, without unrelated
        // opaque/masked stages. Do not change PVS, color targets, or MSAA.
        renderer.set_world_stage_debug(renderer.draw_bsp_model_surfaces, 2);
        sweep.configure(renderer);
        sweep
    }

    fn configure(&mut self, renderer: &mut Renderer) {
        let (probe, fastsky) = match self.case {
            0 | 5 => (0, 0), // Original, before and after, for drift detection
            1 => (1, 0), // Fragment color write, no sample
            2 => (2, 0), // Exactly one constant-UV texture sample
            3 => (3, 0), // Single-face sample with perspective-varying UV
            4 => (0, 1), // TaystJK fastsky skip; same world visibility
            _ => unreachable!(),
        };
        renderer.set_sky_perf_probe(probe);
        renderer.set_fast_sky(fastsky);
        eprintln!("[SKY SWEEP] case {}/{}: {} (120 warmup frames)", self.case + 1, CASES.len(), CASES[self.case]);
    }

    pub(in crate::renderer) fn restore(&mut self, renderer: &mut Renderer) {
        renderer.set_sky_perf_probe(self.original_sky_probe);
        renderer.set_fast_sky(self.original_fast_sky);
        renderer.set_world_stage_debug(renderer.draw_bsp_model_surfaces, self.original_world_probe);
    }

    pub(in crate::renderer) fn record(&mut self, info: &FrameInfo, renderer: &mut Renderer) -> bool {
        // Console must close before timing begins; phase switches are covered
        // by warmup to prevent GPU profiler latency and pipeline warmup bleed.
        if self.armed_at.elapsed() < Duration::from_secs(2) { return false; }
        if info.world_path != "unified" { return false; }
        if self.warmup_remaining > 0 {
            self.warmup_remaining -= 1;
            return false;
        }
        let timing = renderer.gpu_profiler.latest;
        let pass = |pass: GpuPass| timing.and_then(|t| t.milliseconds[pass.index()]);
        self.rows[self.case].push(Row {
            frame_ms: info.frame_ms,
            acquire_ms: info.cpu_acquire_ms,
            encode_ms: info.cpu_encode_ms,
            submit_ms: info.cpu_submit_ms,
            gpu_frame_ms: info.gpu_ms,
            gpu_world_ms: pass(GpuPass::World),
            gpu_post_ms: pass(GpuPass::Post),
            world_encoded_batches: info.world_encoded_batches,
        });
        if self.rows[self.case].len() < self.frames_per_case { return false; }
        if self.case == CASES.len() - 1 {
            self.restore(renderer);
            return true;
        }
        self.case += 1;
        self.warmup_remaining = 120;
        self.configure(renderer);
        false
    }

    pub(in crate::renderer) fn finish(self) {
        std::thread::Builder::new().name("jka-sky-sweep-write".to_owned())
            .spawn(move || {
                let mut csv = String::from("case,frame,frame_ms,acquire_ms,encode_ms,submit_ms,gpu_frame_ms,gpu_world_ms,gpu_post_ms,world_encoded_batches\n");
                let mut summaries = Vec::with_capacity(CASES.len());
                for (case, rows) in self.rows.iter().enumerate() {
                    for (i, r) in rows.iter().enumerate() {
                        use std::fmt::Write;
                        let _ = write!(&mut csv, "{},{},{:.6},{:.6},{:.6},{:.6},", CASES[case], i,
                            r.frame_ms, r.acquire_ms, r.encode_ms, r.submit_ms);
                        fn field(v: Option<f64>) -> String { v.filter(|v| v.is_finite()).map(|v| format!("{v:.6}")).unwrap_or_default() }
                        let _ = writeln!(&mut csv, "{},{},{},{}", field(r.gpu_frame_ms), field(r.gpu_world_ms), field(r.gpu_post_ms), r.world_encoded_batches);
                    }
                    let mean = |f: fn(&Row) -> f64| -> f64 {
                        if rows.is_empty() { return f64::NAN; }
                        rows.iter().map(f).sum::<f64>() / rows.len() as f64
                    };
                    let median = |f: fn(&Row) -> Option<f64>| -> Option<f64> {
                        let mut values: Vec<f64> = rows.iter().filter_map(f).filter(|x| x.is_finite()).collect();
                        if values.is_empty() { return None; }
                        values.sort_unstable_by(f64::total_cmp);
                        Some(values[values.len() / 2])
                    };
                    let mean_frame = mean(|row| row.frame_ms);
                    summaries.push(serde_json::json!({
                        "case": CASES[case], "frames": rows.len(),
                        "mean_frame_ms": mean_frame,
                        "fps_from_mean_frame_ms": 1000.0 / mean_frame,
                        "mean_acquire_ms": mean(|row| row.acquire_ms),
                        "mean_encode_ms": mean(|row| row.encode_ms),
                        "mean_submit_ms": mean(|row| row.submit_ms),
                        "median_gpu_frame_ms": median(|row| row.gpu_frame_ms),
                        "median_gpu_world_ms": median(|row| row.gpu_world_ms),
                        "median_gpu_post_ms": median(|row| row.gpu_post_ms),
                        "mean_encoded_world_batches": mean(|row| f64::from(row.world_encoded_batches)),
                    }));
                }
                let json = serde_json::json!({
                    "experiment": "Unified sky full-frame fragment/fill/sampling sweep",
                    "backend": self.backend, "adapter": self.adapter, "driver": self.driver,
                    "resolution": self.resolution,
                    "gpu_timestamps_available": self.gpu_timestamps_available,
                    "world_stage_probe": 2,
                    "cases": summaries,
                    "notes": [
                        "Only Unified frames are sampled; 120 successful warmup frames per case, plus initial 2s console delay.",
                        "Normal sky is measured twice to expose GPU thermal or frequency drift.",
                        "World stage probe 2 suppresses opaque and masked world stages for every case; authored sky mask geometry remains.",
                        "Solid minus fastsky estimates sky raster/fill/color-write cost, NOT framebuffer bandwidth in isolation.",
                        "Constant sample minus solid includes texture fetch and sampler work; cached UV differs from real texel traffic.",
                        "One-face varying UV versus constant sample includes projective UV and varying texel access; it is an intentional diagnostic image.",
                        "Normal versus one-face varying UV includes six-face selection, different sampled faces and any extra shader operations.",
                        "GPU timestamps are asynchronously resolved and may not match each recorded CPU frame; warmup excludes phase boundaries.",
                        "No additional rendering pass is introduced; shaders restore immediately and the benchmark exits cleanly."
                    ]
                });
                let csv_file = self.stem.with_extension("csv");
                let json_file = self.stem.with_extension("json");
                let result = (|| -> std::io::Result<()> {
                    if let Some(parent) = csv_file.parent() { std::fs::create_dir_all(parent)?; }
                    std::fs::write(&csv_file, csv)?;
                    std::fs::write(&json_file, serde_json::to_vec_pretty(&json).expect("serialize sky sweep"))?;
                    Ok(())
                })();
                match result {
                    Ok(()) => eprintln!("[SKY SWEEP] COMPLETE: {} and {}", csv_file.display(), json_file.display()),
                    Err(error) => eprintln!("[SKY SWEEP] write failed: {error}"),
                }
            }).expect("spawn sky sweep writer");
    }
}

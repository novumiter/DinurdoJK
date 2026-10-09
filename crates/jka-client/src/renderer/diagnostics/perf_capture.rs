//! Bounded, opt-in CPU frame-encoding capture. No draw-site locks, allocations,
//! WGPU readbacks, device polls, or per-frame disk IO. The renderer only reads
//! one relaxed boolean while unarmed. File writing runs after capture on a
//! separate OS thread, never in the render/present critical section.
use crate::renderer::{active_batch_selection, FrameInfo, Renderer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);

pub(in crate::renderer) fn enabled() -> bool {
    CAPTURE_ENABLED.load(Ordering::Relaxed)
}

// Deliberately preserve the original inclusive CPU timing fields next to the
// new *non-overlapping* intervals so existing diagnostics are not redefined.
const FIELDS: [&str; 33] = [
    "frame_ms",
    "cpu_prepare_ms",
    "cpu_acquire_ms",
    "cpu_encode_ms",
    "encode_setup_ms",
    "encode_pre_world_ms",
    "encode_scene_ms",
    "encode_post_ui_ms",
    "encoder_finish_ms",
    "profiler_readback_encode_ms",
    "encode_other_ms",
    "primary_submit_api_ms",
    "followup_ms",
    "cpu_submit_legacy_ms",
    "present_ms",
    "gpu_latest_available_ms",
    "world_pvs_batches",
    "world_encoded_batches",
    "world_multidraw_groups",
    "world_multidraw_batches",
    "primary_command_buffers",
    "frame_inline_render_passes_calls",
    "frame_inline_compute_passes_calls",
    "frame_inline_set_pipeline_calls",
    "frame_inline_set_bind_group_calls",
    "frame_inline_set_vertex_buffer_calls",
    "frame_inline_set_index_buffer_calls",
    "frame_inline_draw_calls",
    "frame_inline_draw_indexed_calls",
    "frame_inline_world_helper_draw_indexed_calls",
    "frame_inline_draw_indexed_indirect_calls",
    "frame_inline_multi_draw_indexed_indirect_calls",
    "frame_inline_multi_draw_indexed_indirect_count_calls",
];

#[derive(Clone)]
struct PerfRow {
    path: &'static str,
    numbers: [f64; FIELDS.len()],
}

impl PerfRow {
    fn from_frame(info: &FrameInfo) -> Self {
        let categorized_encode = info.perf_encode_setup_ms
            + info.perf_encode_pre_world_ms
            + info.perf_encode_scene_ms
            + info.perf_encode_post_ui_ms
            + info.perf_encoder_finish_ms
            + info.perf_profiler_readback_encode_ms;
        Self {
            path: info.world_path,
            numbers: [
                info.frame_ms,
                info.cpu_prepare_ms,
                info.cpu_acquire_ms,
                info.cpu_encode_ms,
                info.perf_encode_setup_ms,
                info.perf_encode_pre_world_ms,
                info.perf_encode_scene_ms,
                info.perf_encode_post_ui_ms,
                info.perf_encoder_finish_ms,
                info.perf_profiler_readback_encode_ms,
                (info.cpu_encode_ms - categorized_encode).max(0.0),
                info.perf_primary_submit_api_ms,
                info.perf_followup_ms,
                info.cpu_submit_ms,
                info.cpu_present_ms,
                info.gpu_ms.unwrap_or(f64::NAN),
                f64::from(info.world_pvs_batches),
                f64::from(info.world_encoded_batches),
                f64::from(info.world_multidraw_groups),
                f64::from(info.world_multidraw_batches),
                f64::from(info.perf_primary_command_buffers),
                f64::from(info.perf_encode_calls.render_passes),
                f64::from(info.perf_encode_calls.compute_passes),
                f64::from(info.perf_encode_calls.set_pipeline),
                f64::from(info.perf_encode_calls.set_bind_group),
                f64::from(info.perf_encode_calls.set_vertex_buffer),
                f64::from(info.perf_encode_calls.set_index_buffer),
                f64::from(info.perf_encode_calls.draw),
                f64::from(info.perf_encode_calls.draw_indexed),
                f64::from(info.perf_encode_calls.world_helper_draw_indexed),
                f64::from(info.perf_encode_calls.draw_indexed_indirect),
                f64::from(info.perf_encode_calls.multi_draw_indexed_indirect),
                f64::from(info.perf_encode_calls.multi_draw_indexed_indirect_count),

            ],
        }
    }
}

pub(in crate::renderer) struct PerfCapture {
    stem: PathBuf,
    armed_at: Instant,
    frames: Vec<PerfRow>,
    target_frames: usize,
    warmup_frames: usize,
    warmup_remaining: usize,
    hooks_owned: bool,
    adapter: String,
    driver: String,
    backend: String,
    fingerprint: String,
    /// Map-load classification, NOT a measured per-material GPU cost.
    material_audit: serde_json::Value,
    /// Capture-only, indexed by the *stable* loaded material debug ID.
    pvs_candidate_names: Vec<String>,
    pvs_candidate_stages: Vec<usize>,
    pvs_candidate_batches: Vec<u64>,
    pvs_candidate_indices: Vec<u64>,
}

impl PerfCapture {
    pub(in crate::renderer) fn arm(stem: PathBuf, frames: usize, renderer: &Renderer) -> Self {
        let target_frames = frames.clamp(30, 6_000);
        let adapter = renderer.adapter.get_info();
        let warmup_frames = 120;
        CAPTURE_ENABLED.store(true, Ordering::Relaxed);
        Self {
            stem,
            armed_at: Instant::now(),
            frames: Vec::with_capacity(target_frames),
            target_frames,
            warmup_frames,
            warmup_remaining: warmup_frames,
            hooks_owned: true,
            adapter: adapter.name,
            driver: format!("{} {}", adapter.driver, adapter.driver_info),
            backend: format!("{:?}", adapter.backend),
            material_audit: material_audit(renderer),
            pvs_candidate_names: renderer.world.as_ref().map(|w| w.material_debug.entries.iter().map(|e| e.name.clone()).collect()).unwrap_or_default(),
            pvs_candidate_stages: renderer.world.as_ref().map(|w| w.material_debug.entries.iter().map(|e| e.resolved_stage_count).collect()).unwrap_or_default(),
            pvs_candidate_batches: vec![0; renderer.world.as_ref().map_or(0, |w| w.material_debug.entries.len())],
            pvs_candidate_indices: vec![0; renderer.world.as_ref().map_or(0, |w| w.material_debug.entries.len())],
            fingerprint: format!(
                "{}x{};present={:?};latency={};msaa={};gpu_timestamps={};pvs={:?};world={:?}",
                renderer.config.width,
                renderer.config.height,
                renderer.config.present_mode,
                renderer.config.desired_maximum_frame_latency,
                renderer.msaa_samples,
                renderer.gpu_profiler.enabled(),
                renderer.pvs_mode,
                renderer.frame_plan.world_path,
            ),
        }
    }

    /// Called only after successfully presented frames. Wait two seconds so
    /// users can close the console; then skip 120 additional successful frames
    /// to avoid measuring the first dynamic resource/path warmup. This does not
    /// pretend pipelines are fully prewarmed: repeat the scene before capture.
    pub(in crate::renderer) fn record(&mut self, info: &FrameInfo, renderer: &Renderer) -> bool {
        if self.armed_at.elapsed() < Duration::from_secs(2)
            || info.perf_primary_command_buffers == 0
        {
            return false;
        }
        if self.warmup_remaining != 0 {
            self.warmup_remaining -= 1;
            return false;
        }
        // PVS candidates, deliberately not "draws": GPU culling, areaportals,
        // frustum, reflection passes, occlusion, water and fog can reject or
        // replay these stages. Aggregated off the render hot path only while a
        // bounded capture is active; no allocation or per-draw locks.
        if let Some(world) = renderer.world.as_ref() {
            if world.material_debug.entries.len() == self.pvs_candidate_batches.len() {
                let active = active_batch_selection(world, renderer.pvs_mode);
                for &index in active.indices {
                    let Some(batch) = active.batches.get(index) else { continue; };
                    let Ok(id) = usize::try_from(batch.source.material_debug_index) else { continue; };
                    if let (Some(batches), Some(indices)) = (
                        self.pvs_candidate_batches.get_mut(id),
                        self.pvs_candidate_indices.get_mut(id),
                    ) {
                        *batches = batches.saturating_add(1);
                        *indices = indices.saturating_add(u64::from(
                            batch.indexed_range.end.saturating_sub(batch.indexed_range.start),
                        ));
                    }
                }
            }
        }
        self.frames.push(PerfRow::from_frame(info));
        self.frames.len() >= self.target_frames
    }

    /// Convert the now-complete bounded capture into an off-render-thread
    /// write. No live GPU objects or `Renderer` borrows cross this boundary.
    pub(in crate::renderer) fn finish(mut self) {
        CAPTURE_ENABLED.store(false, Ordering::Relaxed);
        self.hooks_owned = false;
        std::thread::spawn(move || {
            let csv = self.csv();
            let summary = self.summary();
            let csv_path = self.stem.with_extension("csv");
            let json_path = self.stem.with_extension("json");
            let result = (|| -> std::io::Result<()> {
                if let Some(parent) = csv_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&csv_path, csv)?;
                std::fs::write(&json_path, summary)?;
                Ok(())
            })();
            match result {
                Ok(()) => eprintln!(
                    "[RENDER PERF] wrote {} frames to {} and {}",
                    self.frames.len(), csv_path.display(), json_path.display()
                ),
                Err(error) => eprintln!("[RENDER PERF] capture write failed: {error}"),
            }
        });
    }

    fn csv(&self) -> String {
        let mut out = String::from("frame_index,renderer_path,");
        out.push_str(&FIELDS.join(","));
        out.push('\n');
        for (index, row) in self.frames.iter().enumerate() {
            use std::fmt::Write;
            let _ = write!(&mut out, "{index},{},", row.path);
            for (field, value) in row.numbers.iter().enumerate() {
                if field != 0 { out.push(','); }
                if value.is_finite() { let _ = write!(&mut out, "{value:.6}"); }
            }
            out.push('\n');
        }
        out
    }

    fn sampled_pvs_candidates(&self) -> serde_json::Value {
        let mut rows = self.pvs_candidate_names.iter().enumerate().filter_map(|(id, name)| {
            let batches = self.pvs_candidate_batches.get(id).copied().unwrap_or(0);
            if batches == 0 { return None; }
            Some(serde_json::json!({
                "material": name,
                "resolved_stages": self.pvs_candidate_stages[id],
                "pvs_candidate_batch_occurrences": batches,
                "pvs_candidate_index_occurrences": self.pvs_candidate_indices[id],
            }))
        }).collect::<Vec<_>>();
        rows.sort_unstable_by(|a, b| {
            b["pvs_candidate_index_occurrences"].as_u64().cmp(&a["pvs_candidate_index_occurrences"].as_u64())
        });
        let top20 = rows.iter().filter(|r| r["resolved_stages"].as_u64().unwrap_or(0) > 1)
            .take(20).cloned().collect::<Vec<_>>();
        serde_json::json!({
            "sampled_frames": self.frames.len(),
            "method": "PVS-selected material index/batch occurrences AFTER frame completed; no frustum, area, GPU occlusion, ocean replacement, secondary views, fog passes or GPU work inference",
            "top20_multistage_pvs_candidates": top20,
            "all_materials": rows,
        })
    }

    fn summary(&self) -> String {
        let mut stats = serde_json::Map::new();
        for (index, field) in FIELDS.iter().enumerate() {
            let mut values: Vec<f64> = self.frames.iter()
                .map(|row| row.numbers[index])
                .filter(|value| value.is_finite())
                .collect();
            values.sort_unstable_by(f64::total_cmp);
            stats.insert((*field).to_owned(), if values.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!({
                    "p50": percentile(&values, 0.50),
                    "p95": percentile(&values, 0.95),
                    "p99": percentile(&values, 0.99),
                    "min": values[0],
                    "max": values[values.len() - 1],
                    "samples": values.len(),
                })
            });
        }
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "dinurdojk-cpu-renderperf-v1",
            "sample_count": self.frames.len(),
            "warmup_seconds_before_frame_skip": 2,
            "warmup_skipped_frames": self.warmup_frames,
            "adapter": self.adapter,
            "driver": self.driver,
            "backend": self.backend,
            "settings_fingerprint_at_arm": self.fingerprint,
            "material_audit_static_not_gpu_timing": self.material_audit,
            "material_audit_sampled_pvs_candidates_not_gpu_timing": self.sampled_pvs_candidates(),
            "caveats": [
                "encode_pre_world includes shadows, RT, reflection, grass and environment work before the main scene, and is not a pure shadow timer",
                "encode_scene includes opaque, transparent, dynamic models, and FX (not distinct buckets)",
                "encode_post_ui includes all post, screenshot encoding and in-main UI work until encoder.finish",
                "gpu_latest_available_ms is an asynchronous profiler result and need not correspond to the same CSV frame",
                "cpu_submit_legacy_ms includes follow-up work; primary_submit_api_ms is the isolated primary queue call",
                "primary_command_buffers counts the principal frame command buffer and optional profiler-readback command buffer ONLY",
                "world_multidraw_groups counts API groups, not executed per-surface indexed draws",
                "frame_inline_* counters are exact call-site counts for instrumented frame functions, not all nested renderer modules and not executed GPU indirect draw totals",
                "no image parity or hardware benchmarks are implied by this capture"
            ],
            "metrics": stats,
        })).unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"))
    }
}

impl Drop for PerfCapture {
    fn drop(&mut self) {
        if self.hooks_owned {
            CAPTURE_ENABLED.store(false, Ordering::Relaxed);
        }
    }
}

/// Count emitted *static map-load* recipes for each material independently
/// for MINIMAL/OFF and FULL PVS. These are separate alternative representations,
/// never add the two column totals or treat them as visible frame draws.
/// World triangles are represented by index spans; GPU-driven compaction,
/// frustum/PVS, fog replays, secondary cameras and ocean suppression all change
/// the executed work and are explicitly excluded from this audit.
fn material_audit(renderer: &Renderer) -> serde_json::Value {
    let Some(world) = renderer.world.as_ref() else {
        return serde_json::json!({ "available": false, "reason": "no map loaded" });
    };
    #[derive(Default)]
    struct Counts {
        coarse_batches: u64,
        coarse_indices: u64,
        full_batches: u64,
        full_indices: u64,
        coarse_inline_batches: u64,
    }
    let mut counts: Vec<Counts> = (0..world.material_debug.entries.len())
        .map(|_| Counts::default()).collect();
    for (batches, coarse) in [(&world.coarse_batches, true), (&world.full_batches, false)] {
        for batch in batches.iter() {
            let Ok(index) = usize::try_from(batch.source.material_debug_index) else { continue };
            let Some(count) = counts.get_mut(index) else { continue };
            let indices = u64::from(batch.indexed_range.end.saturating_sub(batch.indexed_range.start));
            if coarse {
                count.coarse_batches += 1;
                count.coarse_indices += indices;
                if batch.is_inline_entity { count.coarse_inline_batches += 1; }
            } else {
                count.full_batches += 1;
                count.full_indices += indices;
            }
        }
    }
    let mut entries: Vec<_> = world.material_debug.entries.iter().zip(&counts)
        .filter(|(_, counts)| counts.coarse_batches > 0 || counts.full_batches > 0)
        .map(|(entry, count)| {
            serde_json::json!({
                "name": entry.name,
                "source": entry.source,
                "mtr_override": entry.mtr_override,
                "authored_stages": entry.authored_stage_count,
                "resolved_stages": entry.resolved_stage_count,
                "audit_class": entry.stage_audit_class,
                "stage_sequence": entry.stage_sequence,
                "coarse_batches": count.coarse_batches,
                "coarse_triangle_indices": count.coarse_indices,
                "coarse_inline_batches": count.coarse_inline_batches,
                "full_batches": count.full_batches,
                "full_triangle_indices": count.full_indices,
            })
        }).collect();
    entries.sort_unstable_by(|a,b| {
        let weight = |entry: &serde_json::Value| {
            (entry["coarse_triangle_indices"].as_u64().unwrap_or(0),
             entry["coarse_batches"].as_u64().unwrap_or(0))
        };
        weight(b).cmp(&weight(a))
    });
    let total = entries.len();
    let offenders: Vec<_> = entries.iter()
        .filter(|v| v["resolved_stages"].as_u64().unwrap_or(0) >= 2)
        .take(20).cloned().collect();
    serde_json::json!({
        "available": true,
        "method": "map-loaded source draw batches, ranked by coarse triangle-index spans (NOT shader GPU time, visible pixels or true per-frame draw calls)",
        "alternatives": "coarse = OFF/MINIMAL representation; full = FULL/exact PVS representation; do not sum them",
        "ranked_materials_total": total,
        "top20_multistage_static_candidates": offenders,
        "all_materials_by_static_indices": entries,
    })
}

fn percentile(values: &[f64], p: f64) -> f64 {
    debug_assert!(!values.is_empty());
    let position = p * (values.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    values[lower] + (values[upper] - values[lower]) * (position - lower as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_percentiles_interpolate_sorted_samples() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&values, 0.5), 3.0);
        assert!((percentile(&values, 0.95) - 4.8).abs() < 1e-10);
    }
    #[test]
    fn export_fields_have_unique_names() {
        let mut names = FIELDS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FIELDS.len());
    }
}

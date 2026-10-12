# Fast-world material-table A/B test handoff

## Objective

Determine on DinurdoJK's own renderer and hardware whether a JKR-style material/stage table improves the **FastBaseline** world path at the user's high-FPS target. Compare steady-state CPU frame encoding, GPU world time, and total frame time. Keep the current renderer as A and add an opt-in, same-binary experimental B only if the baseline evidence and code audit support a meaningful, faithful comparison.

This is a performance experiment, not a request to transplant JKR's renderer or file layout. JKR is a reference for the hypothesis and reported tradeoff only. Preserve the user's existing in-progress work.

## User and workspace constraints

- The user cares about roughly 3000 FPS in light scenes (about 0.333 ms/frame) and currently builds with `build.ps1 -Fast`. This is a build-speed switch, not a runtime renderer mode: it enables incremental Release compilation while keeping codegen-units at 16 and leaving LTO unchanged. Do not conflate it with `WorldRenderPath::FastBaseline`; verify the latter from the capture's `world_path` field and renderer settings.
- The expected optimization is reduced CPU submission/bind-group work, potentially exchanged for GPU shader/table lookup cost. Do not equate JKR's reported GPU milliseconds with an expected FPS gain on this PC.
- The working tree is heavily modified. At handoff time, `frame/fast.rs`, `world/bindings.rs`, `api.rs`, `diagnostics/perf_capture.rs`, and many unrelated files are dirty. Inspect `git status` and diffs before editing. Do not reset, stash, checkout, or overwrite unrelated work.
- The user prefers not to have a game window opened, a running game interrupted, or display state changed. Build and prepare the experiment; do not launch/kill the game or alter gamma/display settings. Let the user run controlled captures in their existing setup.

## Code map confirmed so far

DinurdoJK:

- `crates/jka-client/src/renderer/frame/fast.rs`: `Renderer::render_fast_baseline`; the main visible-world loop sets a pipeline as needed, calls `pass.set_bind_group(1, &batch.fast_bind_group, &[])`, and draws each batch. Deferred transparent world batches are drawn separately after dynamic models.
- `crates/jka-client/src/renderer/world/draw.rs`: `draw_world_batch` currently records `draw_indexed` for one batch. Keep the initial experiment confined to FastBaseline's eligible world draws; do not silently change the advanced/unified renderer or unrelated passes.
- `crates/jka-client/src/renderer/world/bindings.rs`: `create_fast_world_bind_group` creates a per-batch group with base texture/sampler, lightmap/sampler, material uniform, sky/overlay resources, marks, and deformation resources. Read the complete layout and `bsp_fast.wgsl` before designing a table: the table must preserve all data and sampling semantics used by eligible draws.
- `crates/jka-client/src/renderer/world/upload.rs` and `world/types.rs`: world batches and their GPU resources are assembled here; inspect ownership/lifetime and material identity before proposing deduplication.
- `crates/jka-client/src/renderer/diagnostics/perf_capture.rs`: existing opt-in CSV/JSON capture records frame/encode timings, broad inline API-call totals, world batch/multidraw counts, and asynchronous GPU timings. Its own caveats say inline API counters cover instrumented call sites, not every nested module, and `gpu_latest_available_ms` may be asynchronous and from another frame.
- `crates/jka-client/src/app/commands/registry/diagnostics.rs`: `render_perf_capture [frames=600] [label]` arms a capture after a 2-second delay and 120 warmup frames. It does not force a GPU stall. Captures are written away from the per-frame render critical section.
- Existing counters are a useful foundation, but current `set_bind_group` totals do not isolate FastBaseline's world material binds. Add explicit FastBaseline counts/timing rather than treating the global total as the table's cost.

Read-only JKR reference: `D:\Code\JKR-main\crates\jkr-viewer\src\stage_table.rs` and its companion `stage_table.wgsl`. JKR's table uses storage records and binding arrays, puts a material/stage index on draws, and aggregates eligible opaque static draws into multi-draw indirect groups. Its own source reports about 1,770 to 190 color-pass bind-group switches and 0.06–0.08 ms GPU cost at render scale 2. It falls back for ineligible draws/devices. Treat those as JKR-specific reported results, not DinurdoJK baselines or acceptance thresholds.

## Required experiment design

1. **Audit and define comparable A/B paths.** A is the unmodified FastBaseline draw path. B must execute a real material-table strategy on the same visible, eligible draws. A counter-only mode, a CPU simulation of avoided calls, or skipping repeated identical `set_bind_group` calls is not a test of a table. If a faithful B is too invasive for the current renderer, report that with the blockers and propose a smaller experiment without labeling it a stage-table A/B.
2. **Scope eligibility conservatively.** Start with the ordinary opaque/masked static world batches that can be represented identically by the table. Keep transparency, sky special cases, animated/multistage materials, deformation, inline movers, and any unsupported resource combination on the existing path unless the audit proves they are safe. Count eligible and fallback draws explicitly. Confirm how FastBaseline currently handles each case before deciding the exact subset.
3. **Keep the toggle comparable.** Prefer one binary with an opt-in runtime/config switch for A versus B, and record the selected mode in every capture's CSV/JSON metadata. If required WGPU features are unavailable, report B as unsupported; do not silently compare A against A. Avoid recompiling/material setup on steady-state frames. Warm B's one-time table and lazy pipelines before measuring steady state; record cold-start cost separately if relevant.
4. **Instrument low overhead.** Extend existing `render_perf_capture` with FastBaseline-specific world batch/draw counts, material bind-group set calls, pipeline switches, multi-draw calls/groups, eligibility/fallback counts, and a narrowly scoped CPU encode interval for the world loop. Only sample the timer when capture is armed; do not add per-draw allocations, locks, device polling, readbacks, logging, or timestamp queries. Preserve existing metric meanings and document any new field limitations. Use the existing asynchronous GPU profiler for world GPU time if it has a valid FastBaseline world bucket; otherwise state clearly that GPU time is unavailable rather than inventing a substitute.
5. **Measure same conditions.** Use the same map, camera/location and view, resolution/render scale, graphics settings, window mode, driver, adapter, and frame cap/vsync settings. Prefer a repeatable demo or static route if one can hold the visible scene stable. Confirm `world_path` is `fast-baseline` in every sample. Do not mix loading/menu frames or the initial table/pipeline build into steady-state results.
6. **Alternate capture order.** Run multiple captures in an A/B/A/B (or randomized balanced) order after warmup to reduce temperature, clock, and background-load bias. A starting run can use `render_perf_capture 1200 tableA1`, then the corresponding B label and repeat. Use the actual console command and labels supported by the checked-out code; capture several rounds if variance is high. Record all settings and raw files, not just a single average.
7. **Compare distributions and correctness.** Report p50/p95/p99 and spread for `frame_ms`, `cpu_encode_ms`, the scoped world encode interval, and valid GPU world time. Include changes in batch/draw/bind-group counts and verify that A and B process the same eligible geometry. Compare screenshot/output at the same camera and look for missing or altered surfaces, texture/lightmap/sampler differences, transparency-order changes, and animation/deformation issues in fallback cases. Don't convert a small millisecond delta directly into an FPS claim; state whether the machine was CPU-bound, GPU-bound, or inconclusive.

## Acceptance and reporting

Keep B experimental and off by default unless the test shows a repeatable improvement in the metric that limits this workload, with no visible correctness regression and no material regression in the other important metric. State hardware/driver/backend, map/scene, resolution/render scale, path/settings, capture labels/sample counts, feature support, and whether timings are same-frame or asynchronous. Include the raw CSV/JSON output paths and summarize p50/p95/p99 for A and B.

If B is slower or indistinguishable at the target scene, recommend leaving it off. If it helps only at high batch counts, identify that threshold and avoid claiming a universal win. If a code or hardware constraint prevents a trustworthy A/B, give the evidence and a concrete next step rather than presenting estimates as measured results.

## Suggested first actions for the next AI

1. Re-check the live working-tree diff and inspect the FastBaseline selection, complete `fast_bind_group` layout, `bsp_fast.wgsl`, world batch/material construction, and profiler bucket semantics.
2. Write down exactly what B changes and which draws remain on A. Estimate implementation scope before editing; do not copy JKR files wholesale.
3. Implement only the opt-in experiment and the metrics needed to judge it. Keep normal gameplay on A by default and keep B isolated to FastBaseline.
4. Run appropriate compile/static checks without launching the game. Then provide the user with a build and exact A/B capture commands/instructions so they can collect live-device measurements without their session being interrupted.
5. Analyze the resulting captures only after both variants have real samples. Report measured result or clearly mark the work as instrumented-but-not-yet-measured.

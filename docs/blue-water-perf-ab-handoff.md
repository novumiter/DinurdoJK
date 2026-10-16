# BlueWater vs Godot ocean performance handoff

Date: 2026-10-16

## Question and setup

The BlueWater ocean was reported at roughly one third the FPS of the older Godot ocean, even on the lowest settings. We compared both implementations on `costadelsol` at 1920×1080 with clouds and sky disabled, using the same camera location and orientation (requested `-743 3654 187 270 -319`; logs reported view `(-743 3627 200)`, yaw 270, pitch 41).

Captures used the rebuilt executable after applying `Development Docs/Patch20/dinurdojk-bluewater-perf-ab.patch`. The build initially exposed an out-of-scope `ocean_drawn` reference in the BlueWater wireframe path. That path was fixed by deriving the admitted mask locally from visible active batches, then `build.ps1 -Fast` passed with warnings only.

The A/B script was updated to accept `-Width` and `-Height` parameters (defaults 1280×720); that edit remains in `scripts/blue-water-ocean-ab.ps1`. The runs below explicitly used 1920×1080. Test configs and captures are under ignored `target/blue-water-ab/`.

## Results

| Run | FPS | GPU frame | Key pass measurements |
|---|---:|---:|---|
| Legacy/Godot ocean (backend 0) | 108.4 | 7.42 ms | world 4.74 ms, surface 1.31 ms, optics 2.89 ms, sim 1.76 ms, 47,550 triangles |
| BlueWater defaults (backend 1, mesh 128×9) | 53.6 | 18.46 ms | world 17.08 ms, surface 8.22 ms, optics 3.08 ms, underwater 3.11 ms, caustics receiver 2.04 ms, sim 0.41 ms, 233,984 triangles |
| BlueWater reduced profile (mesh 64×6, caustics off, SSR off, adapter off; exact boundary checks retained) | 75.6 | 13.11 ms | world 11.60 ms, surface 4.79 ms, optics 3.06 ms, underwater 3.15 ms, caustics near zero, 41,216 triangles |

These are measured run averages, not guaranteed repeatable frame rates. The renderer logs/csvs provide per-run averages and timing details.

### Individual BlueWater toggles

Baseline repeat was 52.5 FPS / 18.91 ms GPU. Each toggle was tested individually against a fresh baseline-like run:

| Toggle | FPS | GPU frame | Observation |
|---|---:|---:|---|
| Caustics mode 1: compute only on FFT updates | 51.9 | 19.07 ms | No useful savings; FFT updates happen close to render frequency in this run. |
| Caustics mode 2: disabled | 58.7 | 16.88 ms | Saves about 2 ms, mostly the caustics receiver pass. |
| SSR disabled | 58.0 | 17.09 ms | Surface timing dropped about 1.9 ms. |
| Mesh 64×6 | 58.8 | 16.83 ms | Surface timing dropped about 2.1 ms; triangles fell to 41,216. |
| BlueWater adapter disabled | 52.2 | 18.91 ms | No measurable improvement. |
| Boundary mode 1 | 59.8 | 16.63 ms | Skips exact water footprint checks while retaining PVS; diagnostic only, can produce visual leaks and is not a safe default. |

Combined reduced profile gained about 42% FPS over BlueWater defaults, but still ran slower than the legacy/Godot path and used visual quality compromises.

## Diagnosis

- This is GPU-bound in the captured runs. The new sample had approximately 14.6 ms GPU time against roughly 0.17–0.19 ms client CPU time. The GPU acquire wait is backpressure while the GPU is busy, not evidence that the main thread is doing expensive ocean culling.
- The culling counters showed `ocean_cull=0b/0t` (no batches/triangles culled) in the supplied sample, so underwater depth culling was not responsible for the slowdown there.
- Comparing the supplied old/new samples, GPU world time increased about 9.16 ms and surface time about 4.13 ms. Optics was slightly faster in the new sample. Newly measured underwater and caustics receiver passes together cost about 5.1 ms in the BlueWater capture; surface also costs substantially more. This explains most of the gap.
- The supplied optics diagnostic said `submerged=0` while the underwater pass still measured about 3.1 ms. Investigate why the underwater pass is issued/measured for this view and whether it can be skipped when it has no visible contribution.
- Mesh density is a factor, but not the whole problem: with 64×6, BlueWater had about 41k triangles, comparable to the legacy path's 47.6k, while its GPU frame remained 13.1 ms versus 7.4 ms.
- Focus next on unnecessary underwater work, caustics receiver cost, and the BlueWater surface shader/pass workload. The culling system needs correctness follow-up for the intermittent missing seabed triangles reported separately; do not use boundary mode 1 as the fix.

## Artifacts

- Toggle run log: `target/blue-water-ab/perf20-bluewater-toggles/costadelsol_ocean_1.log`
- Legacy/Godot CSV: `target/blue-water-ab/perf20-legacy-ocean-single/base/renderperf/renderperf-2026-10-16_14-02-17-costadelsol_ocean_0.csv`
- BlueWater default CSV: `target/blue-water-ab/perf20-ocean-backend-ab/base/renderperf/renderperf-2026-10-16_14-01-31-costadelsol_ocean_1.csv`
- Combined reduced-profile CSV: `target/blue-water-ab/perf20-low-settings/base/renderperf/renderperf-2026-10-16_14-03-58-bw_low_combined.csv`
- Original supplied perf snippets were pasted from attachments under `C:\Users\David\.codex\attachments\` (see the conversation for their full paths).

## Caveats and next measurement

The Godot and BlueWater captures are controlled for resolution, map, camera, and clouds/sky, but they were separate application runs; treat small differences as noise. Repeat each backend several times in alternating order, retain the full CSVs, and compare medians. Record GPU model/driver and confirm the same fullscreen/windowed mode and VSync state. Then profile pass-level GPU costs and verify whether the underwater pass can be conditionally omitted for the `submerged=0` case.

## Continuation: hybrid ocean profile (2026-10-16)

The original measurements above describe the earlier BlueWater defaults. The current experimental profile keeps the BlueWater surface above water and selects Godot's surface mesh/shader, underwater optics, visibility cull and inexpensive optics caustics while submerged. The native BlueWater underwater composite and caustic receiver are off by default. The BlueWater FFT still supplies wave textures to the Godot underwater shader, so its ripple pattern differs from a pure Godot run. This is an appearance tradeoff for review, not pixel parity.

The main A/B cvars remain: `r_oceanBackend` (0 Godot, 1 BlueWater hybrid), `r_blueWaterUnderwaterMode` (4 hybrid, 0 full native, 1 native without motes, 2 no composite, 3 native only when submerged), `r_blueWaterCausticsMode` (2 native receiver off, 0 every frame, 1 FFT updates), `r_blueWaterSSR`, `r_blueWaterMeshSegments`, `r_blueWaterMeshLevels`, `r_blueWaterBoundaryMode` (3 exact binned, 0 original exact), and `r_blueWaterRefraction` (0 by default; independent of `r_oceanRefraction`). `r_oceanCaustics` still controls the Godot optics caustics in hybrid mode.

The exact-binned boundary mode precomputes 8x8 tile candidate masks and still performs exact triangle footprint tests. A focused GPU fixture passed for outline, PVS and sky contacts. This saved about 0.4 ms GPU in the measured view. Native caustic receiver removal saved about 2 ms GPU, SSR removal about 1.9 ms, a 64x6 mesh about 2.1 ms, and disabling BlueWater scene refraction about 2 ms in earlier isolated captures. These per-pass savings are approximate and should not be added to predict a frame time.

Matched 1920x1080 `costadelsol` submerged capture, settled camera `(-2384 11529 -135)`, yaw 292, pitch -12, clouds/sky off, windowed, VSync off, RTX 3060 driver 581.80, thin-LTO iteration build: Godot 111.7 FPS / 7.603 ms GPU; BlueWater hybrid 111.1 FPS / 6.187 ms GPU. Both had 60 specialized and 2 generated world programs. The earlier hybrid with Godot optics but the native BlueWater underside had a very dark ceiling; switching the submerged surface restored the brighter blue view. The new screenshot still has broader, less detailed surface ripples than Godot at this camera. Captures: `target/blue-water-ab/perf21-hybrid-surface-submerged/`.

The final above-water executable (SHA-256 `7195cb46235d30b39c54c7e0b5bc3f3ff651005903dc06c1826064785b6005b3`) was built successfully with `build.ps1 -Fast`. After restricting Godot mesh preparation to submerged hybrid frames, matched above-water captures at settled camera `(-743 3596 174)`, yaw 270, pitch 41 yielded Godot 103.8 FPS / 8.601 ms GPU versus BlueWater 98.8 FPS / 8.380 ms GPU. The reverse-order pair yielded Godot 101.0 FPS / 8.450 ms GPU versus BlueWater 96.7 FPS / 8.515 ms GPU. The BlueWater GPU budget is at parity here, but the final FPS results are consistently about 4-5 FPS behind. This means the overall equal-or-better FPS goal is **not yet established** for the final hybrid profile at this camera. In the render capture, BlueWater median CPU encode was 7.725 ms versus 7.565 ms in the earlier parity capture, while major draw counters and GPU pass timings remained effectively unchanged. Further CPU/frame pacing investigation is needed; the preceding parity results should not be presented as the final hybrid result.

Final above-water artifacts: `target/blue-water-ab/perf21-hybrid-final-above/` and `target/blue-water-ab/perf21-hybrid-final-above-reverse/`. The submerged capture used the immediately preceding build; the final source change only gates Godot mesh updates above water. These captures cover two cameras on one map; waterline transitions, shallow caustics and other weather/maps still need visual and performance review.

For further isolation, `scripts/blue-water-ocean-ab.ps1` now accepts `-ExtraSettings` and records them in `results.json`. A same-executable dry-camera probe with `r_blueWaterUnderwaterMode 3` yielded 101.2 FPS / 8.520 ms GPU, but its 180-frame median CPU frame was 11.08 ms versus 9.54 ms in the preceding mode-4 run. The mixed short-window FPS and CPU results indicate frame pacing or external load variation, so this single probe does not prove that the hybrid branch itself adds the 4-5 FPS cost. Keep the repeated final backend A/B results as the conservative performance status.

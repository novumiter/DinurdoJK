# Blue Water integration status (2026-10-16)

Experimental worktree: `D:\Code\DinurdoJK-BlueWater`, branch `experimental/blue-water` from DinurdoJK `2b2343d`. The original `D:\Code\DinurdoJK` checkout was left unchanged. Reference source: local `D:\Code\blue-water-sea-and-sky` at `16c048`, with copied source under `vendor/blue-water-sea-and-sky/` in its upstream directory structure. Upstream license and third-party notices are included. Upstream `AGENTS.md` instructions were not imported or followed.

## Current water-only status (2026-10-16)

### Immediate screenshot feedback (2026-10-16)

Two fixes are in the experimental working tree. The original Blue Water underwater composite now runs after scene weather and before wireframe and other debug overlays; the legacy Godot path keeps its own render order. The JKA-composed Blue Water surface also treats opaque depth in front of a water sample as open water, avoiding false shoreline foam and foreground-color transmission at partially covered player silhouettes. Preserved upstream shaders remain unchanged.

`cargo check -p DinurdoJK --offline`, the composed surface shader test, the real GPU surface-pipeline test and `build.ps1 -Fast` passed. The Fast playable executable is `target/release/DinurdoJK.exe` (SHA-256 `11d69cab71c2f696105a95244f74800791d7ed44212be42861117c1d23826c68`). An isolated submerged capture with all wireframe categories enabled exited without GPU errors; the captured lines stay bright green through Blue Water. The foreground-outline fix still needs review at the user's exact camera and settings. The linked source checkpoint in `LATEST.md` predates these two fixes.


**The promoted-water path is playable and ready for continued user testing. Full upstream water parity is not complete.** The user reviewed the caustics/refraction build and reported that it looks good. Native wave sliders and transparency/water/fog color controls are now connected. Their final appearance and slider behavior await user review. Water testing stays clouds off until water is finished. This summary supersedes earlier milestone descriptions below where they describe features as pending.

### Connected and working in the port

- Runtime r_oceanBackend 0/1 selection between the existing Unified ocean and Blue Water. The inactive backend does not dispatch its wave simulation.
- Preserved upstream spectrum generation, FFT, displacement/derivative fields, persistent whitecaps and mips. Current quality is High: three 256x256 cascades.
- Original surface shader, original foam texture and clipmap on automatically promoted water. JKA shoreline triangles, PVS admission and actual water footprints restrict drawing. Qualified open water uses the curved horizon/skirt; finite pools stay flat. Infinite-ring clipping was corrected to fit JKA's distanceCull depth range.
- Camera, wave, foam, copied opaque color/reversed depth, screen reflections and environment/light bindings. These include upstream lighting resources when available, with map lighting as the fallback.
- Original underwater probe, underside/Snell window, absorption/scattering, waterline and motes, restricted by JKA water volumes and shoreline admission. The North Atlantic linear-color calibration and original clarity 0.15 replace the earlier temporary optical values. New transparency and color controls adjust that calibration live.
- Unchanged upstream four-depth caustic generator, applied through a JKA receiver pass to submerged opaque world/model surfaces. Depth attenuation, water boundaries and the upstream camera-height fade apply. Caustics run before atmospheric scattering and enter the surface scene copy.
- Scene refraction through both sides of the wave surface. This is a host extension using upstream normals, water index, Fresnel and attenuation; the supplied upstream shader provided refracted sky and same-pixel scene transmission. Invalid or missing screen-space hits use the original fallback.

### Available water controls

Location: **Video > Environment > Blue Water**. Native wave controls are shared across promoted pools, update the running simulation and save to config.

| Control | Current range / behavior |
| --- | --- |
| Wave height | Significant wave height, 0-20 metres |
| Swell direction | -180 to 180 degrees, from +X toward world +Z |
| Peak wavelength | 2-512 metres; native dispersion determines wave speed |
| Choppiness | Horizontal displacement strength, 0-4 |
| Spectrum wind speed | 0.1-40 m/s, independent of weather wind |
| Reset Waves | Previous resolved upstream sea preset: 1.8 m height, 35-degree heading, approximately 70 m peak wavelength, breaking-limit chop and 9 m/s wind |
| FFT update rate | 0-60 Hz; 0 is uncapped |
| Transparency | 0.1-10x optical travel distance; 1x preserves the original calibration, higher is clearer |
| Water color | Color picker for the body and transmitted crest light; default #49575e |
| Underwater fog color | Scattered underwater light and underside pigment; default #2d373c |
| Reset Water Appearance | Restores the upstream transparency and color defaults |
| Foam color | Original crest/surface foam tint |
| Surface refraction | 0-0.15; 0 disables scene bending, 0.1 gives the default physical direction |
| Caustic strength | Menu 0-4; 0 disables receiver modulation, 1 is default |

The five new wave cvars are r_blueWaterHeight, r_blueWaterDirection, r_blueWaterWavelength, r_blueWaterChoppiness and r_blueWaterWindSpeed. Appearance cvars are r_blueWaterTransparency, r_blueWaterWaterColor and r_blueWaterFogColor (colors use three sRGB components, 0-1). Existing r_oceanRefraction and r_oceanCaustics control the new surface/receiver effects. Native settings use explicit WaveParameters so automatic sea-state derivation no longer overrides the wavelength or choppiness sliders. FFT tiles grow with peak wavelength using the upstream rule.

### Outstanding water work and limitations

1. **Original material coverage:** authored water and secondary render paths still retain JKA/adapter paths. The original surface replacement currently targets automatically promoted water.
2. **Cross-sea control:** the old authored-swell / FFT blend is intentionally unconnected to Blue Water. An independently directed secondary sea would need additional spectrum work. Per-pool native sea states are also unimplemented.
3. **Quality controls:** FFT resolution/quality selection remains fixed. Native water appearance now has separate controls; similarly named legacy controls on the Water page continue to configure Unified water.
4. **Receiver/refraction limits:** caustics affect the nearest opaque receiver, not transparent materials. They modulate JKA's baked lighting rather than a separately evaluated upstream direct-light material. Refraction and screen reflections cannot recover scene geometry absent from the screen buffer. A full-screen underwater wobble is not implemented.
5. **Missing upstream host data:** object-shadow, wake, hull and shore bindings remain neutral where JKA has no matching source. Existing host effects do not establish parity with upstream ship/wake/hull interactions.
6. **Transitions and tuning:** slider changes rebuild the spectrum without a temporal morph, so abrupt changes while dragging are possible. Native-slider behavior, extreme settings and remaining waterline/visibility issues need user review. The user's positive caustics/refraction review is not a confirmation of every water path or setting.
7. **Performance:** previous measurements showed a substantial underwater regression from higher water triangle counts and retaining geometry hidden by the legacy absorption cull. Blue Water now uses an extinction-matched cull, but this revision has no fresh full-release A/B timings. Earlier figures below describe older binaries.

### Blue Water absorption-matched depth culling (2026-10-16)

Blue Water now culls opaque world batches and outer promoted-water clipmap rings only after its underwater composite's least-absorbed color channel falls below 0.2% scene transmission. The cutoff includes the composite's 0.15 underwater clarity multiplier, the upstream North Atlantic RGB absorption coefficients, the live Blue Water Transparency multiplier, and JKA's 40 game units per metre. It therefore gets farther when the water is clearer and nearer when it is murkier. The same distance drives underwater radial culling and above-water water-column culling; the existing water-volume and exact footprint masks remain in force. Submerged geometry keeps the existing 256-unit minimum radius. Unified water keeps its separate legacy transparency distance and cull multiplier.

GodotOceanWaves does not contain a scene-geometry depth culler to port. Its water shader tints the screen sample by RGB depth-consumption smoothsteps and fades water near opaque intersections. Blue Water already computes view-path absorption and scattering from the bounded underwater ray, so the new cull follows that native extinction model instead of copying Godot's surface tint formula.

No build, visual capture or performance sweep was run for this change. The cutoff is an optimization based on the current Blue Water calibration and should be reviewed with deep-water and surfacing views; its A/B CPU and triangle impact remains unmeasured.

### Current delivery and verification

- Worktree/branch: D:\Code\DinurdoJK-BlueWater, experimental/blue-water.
- Latest source checkpoint, cumulative patch, manifest and SHA-256: [current artifact index](../artifacts/blue-water/LATEST.md).
- Current executable SHA-256: 11d69cab71c2f696105a95244f74800791d7ed44212be42861117c1d23826c68. This is the Fast thin-LTO iteration build for interactive review.
- Client compile check and iteration build passed for the native sliders and appearance controls. The focused GPU pipeline/binding and flat-field check passed again with the water-body color shader. Older milestones have broader validation recorded below; they do not establish visual/performance validation of the newest revision.
- No new capture/benchmark sweep was run for these feedback revisions. The user is the source of ongoing visual testing.
- Interactive launch: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/blue-water-play.ps1. It opens costadelsol in free flight with clouds disabled. Artifact index: [LATEST.md](../artifacts/blue-water/LATEST.md).

## Historical integration milestones

The sections below retain the chronological integration record, including non-water milestones. Use the water-only summary above for the current water status.

## Current runnable stage

`r_oceanBackend 0|1` switches the Unified renderer between its current ocean field and Blue Water's spectrum, FFT, persistent whitecaps, and mip field. The Blue Water CPU algorithms and three GPU shaders are compiled from the preserved upstream files. The inactive ocean field dispatches no simulation GPU passes. `r_oceanBackend` can be changed at runtime; resource recreation and pipeline compilation currently occur synchronously on the render thread.

Backend 1 now draws automatically promoted pools with the original Blue Water surface shader and High-tier upstream clipmap. Its live bindings include per-pool metre-space cameras, original wave arrays and foam texture, an independent opaque color/reversed-depth copy, the original screen reflection trace, and map sun/sky lighting. A JKA admission function is added to the composed shader at group 1 binding 16; the preserved upstream file is unchanged. This boundary retains exact shoreline triangles, the current FULL PVS tile union, and qualified sky-contact strips/corners. Qualified infinite extensions use the original horizon/skirt and 6360 km curvature. Finite pools remain flat. The temporary texture conversion remains for existing JKA readers, authored-water and secondary paths; those paths have not all been replaced with the original material.

The original underwater probe, underside, attenuation/scattering, waterline and motes are connected for promoted water with JKA volume/shoreline restrictions. Original atmosphere, sun/moon/stars, environment baking, aerial perspective, volumetric clouds, reconstruction and cloud shadows are now connected. Object-shadow, wake, hull and shore resources remain neutral where JKA supplies no matching upstream data. The integration retains authored/secondary water paths and the map's baked world lighting. These host adaptations are not a claim of complete image parity with the standalone upstream renderer.

`r_skyBackend 0|1` selects authored JKA sky or original Bruneton atmosphere; `r_cloudBackend 0|1` selects JKA clouds or the original cloud renderer. All three backend switches default to 0 and apply at runtime. Enable `r_ocean 1`, `r_oceanBackend 1`, `r_skyBackend 1`, `r_clouds 1`, and `r_cloudBackend 1` on the Unified path for the combined integration. Resource/pipeline construction on the first switch is synchronous. Combined full-release validation is complete; the ocean-only full-release gate below was completed before atmosphere work began.

The preserved upstream foam-texture generator and geographic water-optics module compile unchanged, with all six additional upstream tests passing. `blue-water-gpu::foam_texture::FoamMap` uploads the original nine mip levels and creates the original repeating, anisotropic sampler. The original surface allocates and shares this resource between its pool bindings.

## Reproduce the ocean field comparison

From the experimental worktree, build `cargo build -p DinurdoJK --release --offline`, then run `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\blue-water-ocean-ab.ps1`. The script hardlinks existing `.pk3` packages and runtime DLLs into isolated worktree locations, loads `costadelsol`, executes `setviewpos -2397 11562 817 292 -353`, warms for 15 seconds, saves a PNG, and records 180 performance frames for each backend. It writes only under `target/blue-water-ab/`. The displayed camera settled at `(-2369 11491 845) yaw 292 pitch 7` for both runs; pitch 7 is equivalent to the supplied -353.

First repeatable capture (1280x720, Vulkan, RTX 3060, 2026-10-15):

| Metric | Unified field (0) | Blue Water field (1) |
| --- | ---: | ---: |
| FPS, 20-second sample average | 39.3 | 40.9 |
| CPU encoding, p50 ms | 19.837 | 20.317 |
| GPU total latest available, p50 ms | 8.855 | 7.746 |
| GPU ocean simulation, p50 ms | 2.582 | 0.543 |
| GPU ocean surface, p50 ms | 0.779 | 0.727 |
| GPU ocean optics, p50 ms | 1.152 | 1.224 |
| Total GPU used, baseline to observed peak MiB | 2267 to 4566 | 2232 to 4692 |

The GPU timestamps are asynchronous and need not align with a CSV CPU frame. Total GPU memory readings are system-wide `nvidia-smi` snapshots at five-second intervals, so the difference is a coarse observation, not process VRAM attribution. This stage's own known High-tier spectrum, FFT scratch, wave textures, and adapter textures allocate about 30 MiB before driver overhead. GPU timings include the existing JKA surface and optics shaders in both modes. FPS is heavily CPU limited at this map/view and is only one run per mode.

Current captures: `target/blue-water-ab/base/screenshots/shot2026-10-15_19-18-04.png` (0), `shot2026-10-15_19-18-44.png` (1); corresponding CSV and JSON files in `target/blue-water-ab/base/renderperf/` use `costadelsol_ocean_0` and `_1` stems. The run summary and stdout/stderr logs are under `target/blue-water-ab/`.

## Validation and source audit

`cargo test -p blue-water-core -p blue-water-gpu --offline` passed 11 preserved upstream spectrum/sea/whitecap CPU tests, 16 preserved clipmap geometry tests, original-surface shader composition and real-GPU pipeline creation tests, and a real-GPU FFT/foam readback test. The client GPU texture conversion test passed with incremental compilation disabled (the first attempt exhausted disk space in generated Rust cache; removing only that cache resolved it). `cargo check -p DinurdoJK --offline` and `cargo build -p DinurdoJK --release --offline` passed. Both map launches exited normally with no new GPU validation errors, and a same-process 0→1→0 runtime switch also passed. Existing Equal-depth/invariant warnings appeared in both A/B runs. The three Blue Water compute pipelines took 3.832, 12.256, and 1.622 ms in a warmed GPU test; in the client GPU test they took 18.265, 30.244, and 8.355 ms. The original surface pipeline took 44.877 ms without point lights and 46.240 ms with them in a separate GPU test. These are module-plus-pipeline or pipeline creation measurements, not a cold client-wide shader compilation measurement.

The imported Cargo manifests declare the upstream renderer/simulation/cloud dependencies, but they are inert reference files. The new active crates use wgpu/bytemuck/futures-lite and the audited official naga_oil 0.23.0 release (Naga 30 compatible; no build script). Cargo added four lockfile packages including naga_oil and downloaded the crate; no upstream Blue Water build script ran. The only imported script, `sky/data/build.ts`, fetches the Yale star catalog when run without a local path and writes `bright_stars.tsv`; it was inspected and not executed.

A timestamped ZIP, SHA-256 digest, changed-file manifest, and cumulative binary Git patch are in `artifacts/blue-water/`. They are cumulative source checkpoints for the tested field, clipmap-source, and original-surface-shader stages; screenshots and performance captures remain under `target/blue-water-ab/`.

After the surface-shader checkpoint, `cargo test -p blue-water-core --offline` passed 17 tests and `cargo check -p blue-water-gpu --offline` passed with the foam/optics addition. The earlier client release build and map captures predate this addition, which is not yet used by the client render path.

## Surface wave bindings checkpoint

`blue-water-gpu::surface_waves` now owns the original shader's group 2 layout: the current displacement, derivatives and extras arrays, repeating anisotropic field sampler, spectrum-derived wave uniforms, and neutral hull/shore resources. It writes uniforms only when the Blue Water simulation advances and binds the current ping-pong field set. The client allocates this group with its Blue Water ocean and updates it alongside the temporary conversion pass. This adds a shader resource contract but does not alter the visible surface.

The upstream shader's group 2 uniform struct sizes were checked against the composed Naga module (576 bytes for waves, 848 bytes for hull sea). `cargo check -p blue-water-gpu --offline`, `cargo check -p DinurdoJK --offline`, and the real-GPU FFT/foam test with the new group creation and upload passed. The first test build exhausted disk space before running; removing only `target/debug/incremental` and setting `CARGO_INCREMENTAL=0` let it pass. No new screenshot or timing has been taken because the original surface is not drawn.

The next render step needs group 0's camera matrices and reversed-depth conventions, group 1's foam, opaque color/depth copy and surface parameters, and group 3's sky environment. JKA's existing optics capture provides linear depth, whereas the original shader samples reversed depth; use a separate compatible copy. Draw the upstream mesh only where JKA's promoted-water geometry, PVS, and footprint authorize water. The current batch-level PVS gate alone cannot bound the upstream clipmap.

## Original surface bridge in progress

`crates/blue-water-gpu/src/original_surface.rs` supplies explicit surface layouts, live bindings, and original geometry buffers. `crates/jka-client/src/ocean/original_surface.rs` maps the JKA camera around each pool's plane at 40 units/metre, including temporal jitter. The original wave/shading coordinates stay in metres. `scene_copy.rs` captures opaque color and reversed depth separately from JKA's linear-depth optics; 4x MSAA selects the color belonging to the nearest depth sample. The copy is included in the ocean-optics timestamp interval.

`surface_boundary.wgsl` admits only the pool AABB/shoreline and current PVS tiles, with JKA's 0.75-unit triangle tolerance. Sky extension admission uses the same contacted-edge strips and corner rule as JKA, selected through its existing PVS/area gate. The original curved horizon can draw when its nearby finite mesh is outside the frustum. The legacy lower-sky suppression is restricted to backend 0 so it cannot leave a black band above the curved Blue Water limb. A new runtime capture confirms this removes the black band.

`map_environment.rs` samples the map's six sky faces in their JKA orientation into the original 384x512 CubeUV layout, then dispatches the preserved `sky/shaders/prefilter.wgsl`. The atlas is rebuilt when its source views change. Maps without an outer skybox use the map's ambient mean. This is map-sky lighting, not the Bruneton sky backend.

Validation so far: all 26 `blue-water-gpu` tests pass, including actual clipmap draw/binding validation, rectangular and L-shaped footprint/PVS/sky admission readback, color and reversed-depth readback at 1x/4x MSAA, and six-face atlas orientation plus finite output at every prefilter level. The client camera equivalence test and two existing performance-capture schema tests pass. The first default client-test link exceeded available PDB space; rebuilding with `--config profile.test.package.DinurdoJK.debug=0` passed. Only generated worktree build symbols, failed test objects and obsolete test executables were removed to recover disk space.

The initial original-surface full release build passed and its `costadelsol` run exited without GPU validation errors. The later surface/sky iteration uses `cargo rustc -p DinurdoJK --release --offline --bin DinurdoJK -- -C lto=thin -C codegen-units=16` for faster visual iteration; its timings must not be presented as the final full-release result. Its capture is `target/blue-water-ab/original-surface-sky-iteration/base/screenshots/shot2026-10-15_21-22-21.png`. The CSV confirms one original-surface draw and 209408 triangles. The settled camera is exactly `(-2369 11491 845), yaw 292, pitch 7`. The image still showed the lower-sky band that prompted the correction above.

The capture script now accepts `-RunName` to keep stages separate, disables mouse sensitivity/acceleration for camera repeatability, suppresses console notifications, and compares the settled cameras between the two backends. Omit `-Backends` to run both; passing an integer array through Windows PowerShell's external `-File` argument handling does not reliably preserve the array. The main-checkout client was closed with the user's approval for uncontended measurements. `ocean_backend`, `ocean_original_surfaces`, and `ocean_triangles` are recorded in each performance CSV row.

The corrected iteration A/B pair is under `target/blue-water-ab/original-surface-horizon-iteration/`: screenshots `shot2026-10-15_21-26-09.png` (0) and `shot2026-10-15_21-26-48.png` (1), both with camera `(-2369 11491 845), yaw 292, pitch 7`. Both runs exited normally without new GPU validation errors. Their 20-second average FPS is 41.8 and 42.1 respectively. Over each 180-frame CSV, median ocean simulation is 2.411 / 0.465 ms, surface is 0.759 / 2.415 ms, and optics (including Blue Water's scene copy) is 1.083 / 1.072 ms. Submitted water triangles are 36976 / 209408. Total system GPU memory baseline-to-peak is 2052-to-4398 / 2099-to-4393 MiB. These are preliminary thin-LTO iteration measurements, with asynchronous GPU timestamps, one run per backend, and system-wide memory observations.

The same-process 0-to-1-to-0 surface switch also exited normally without GPU validation errors; all three samples retained the fixed camera. Its logs and an additional backend-1 screenshot are in the corrected iteration directory (`runtime-switch.log` and `runtime-switch.stderr.log`).

At this earlier checkpoint, the next required work was to finish the original underwater effects and remaining required surface bindings, then take a fresh identical-camera full-release A/B pair before beginning the atmosphere and clouds. Ship wake/hull and shadow resources remain neutral where JKA has not supplied corresponding data. No finished ocean/sky/cloud parity claim is made at this checkpoint.

## Original underwater integration (2026-10-16)

`blue-water-gpu::underwater` composes the preserved upstream probe, composite and motes shaders. Host additions constrain the probe and motes to the same JKA footprint/PVS/sky admission as the surface, and cap optical distance with a separate R32Float texture computed from JKA's water-volume ray union. The old JKA underwater composite is skipped when this original pass is active. The initial backend 1 integration disabled CPU water-absorption culling because the Unified cutoff did not match Blue Water's extinction model; that limitation is superseded by the absorption-matched cull described below. PVS, area, frustum and shoreline gates remain active.

The 128x72 probe runs after the FFT and before the surface reads it. After transparency and weather, the original scene copy is refreshed with post-water reversed depth; dimming, scattered light, waterline and motes then draw before wireframe and other diagnostics. A separate `blue_water_underwater` timestamp measures this late interval inside World, without adding it twice to fallback frame totals. Performance CSVs now contain 177 fields.

The path cap includes the selected pool's conservative wave-height envelope; actual wet pixels come from the GPU wave probe and the rendered surface still limits receiver depth. A no-receiver underwater ray can continue to optical opacity through qualified sky-contact columns. Its 10 km integration limit is optical only and adds no rendered water extent; finite pool rays still exit their real bounds. The legacy JKA color capture retains its original no-receiver behavior. The distance shader tightens only the interval-merge tolerance for these longer rays.

Validation: 17 core tests and 27 GPU tests pass, including a new wet/dry readback checking probe signs, unchanged dry/outside/zero-length pixels, and original Beer-Lambert attenuation for a bounded wet path. All 11 JKA optics tests pass with ignored GPU tests explicitly enabled: 1x/4x MSAA, empty/full/overlapping/partial water, a crest above the nominal plane, finite no-receiver pool exits, and qualified open-water paths. The existing caustic-boundary fixture needed its capture pipeline format updated to the current Rgba16Float attachment; it passes again. The performance CSV schema tests also pass.

The capture script accepts `-View 'x y z yaw pitch'`, `-Noclip` (joins first and verifies noclip was accepted), and `-BuildProfile`; it records the executable SHA-256. Early underwater iterations exposed a short-far-plane blue band and capture setup issues; they are debugging captures, not final validation or performance results. A Windows CRLF-sensitive shader substitution was corrected and guarded.

Corrected thin-LTO visual runs exited normally without GPU validation errors: `underwater-open-depth-iteration` at settled camera `(-2384 11529 -135), yaw 292, pitch -12`, and `underwater-waterline-crossing-iteration` at `(-2383 11529 3), yaw 292, pitch 0`. Their screenshots are `shot2026-10-16_06-16-39.png` and `shot2026-10-16_06-18-55.png`, beneath each run's `base/screenshots`. The optically open region now darkens properly. The cyan lower image is finite seabed geometry: a BSP ray check identifies `textures/SephTestMap/terrain_wetsand_blend`, rather than missing geometry. The moving-wave still at the mean plane catches the lens above a trough; a separate controlled GPU test verifies unchanged sky pixels above a calm waterline, the original 85% meniscus tint, and attenuated pixels below it. Full-release measurements are still required.

Existing JKA volumetric clouds still use their nominal-plane underwater classification in post; they do not yet consume the original per-pixel probe. Their interaction at the moving waterline remains part of the pending cloud integration. The map-sky atlas and these iteration captures must not be presented as finished sky/cloud parity.

Both a visual build and test link exhausted drive D while another main-checkout test build was running. Only generated symbols/failed objects in this worktree were removed. This worktree's `target/debug` and `target/release` now point through junctions to `C:\Users\David\AppData\Local\Temp\dinurdojk-blue-water-build-cache\{debug,release}`; source, captures and checkpoints remain in their original locations. The main-checkout build was left running. The user subsequently freed additional disk space.

## Ocean full-release gate before atmosphere

The full `cargo build -p DinurdoJK --release --offline` completed in 8m55s. Its executable SHA-256 is `9f16fdc7097bdf4c24f87eb758552d4d8d4d065482b08f654a91e78aeebe17a0`. The following two A/B pairs used that identical binary and exactly matching floating-point PNG camera metadata. Both backends exited normally without GPU errors. Both full-release submerged images were inspected.

| Ocean-only view (backend 0 / 1) | Above water | Submerged |
| --- | ---: | ---: |
| 20-second average FPS | 41.7 / 43.7 | 143.0 / 60.3 |
| Median CPU encode, ms | 20.075 / 19.012 | 3.994 / 11.997 |
| Median available GPU interval, ms | 9.233 / 8.439 | 6.253 / 8.919 |
| Median ocean simulation, ms | 2.673 / 0.506 | 2.090 / 0.414 |
| Median ocean surface, ms | 0.805 / 2.621 | 0.826 / 2.223 |
| Median optics, ms | 1.204 / 1.183 | 1.327 / 1.411 |
| Original late underwater, ms | inactive | 1.967 (p95 2.167) |
| Submitted water triangles | 36976 / 209408 | 38912 / 233984 |

Above-water artifacts are in `target/blue-water-ab/ocean-underwater-full-release/`, screenshots `shot2026-10-16_06-29-25.png` / `shot2026-10-16_06-30-04.png`. Exact camera: `[-2368.515380859375,11491.4970703125,844.6634521484375]`, angles `[7.0037841796875,291.99462890625,0]`, FOV 90. Submerged artifacts are in `ocean-submerged-full-release/`, screenshots `shot2026-10-16_06-31-06.png` / `shot2026-10-16_06-31-46.png`. Exact camera: `[-2383.77685546875,11529.2685546875,-135.49671936035156]`, angles `[-11.9970703125,291.99462890625,0]`, FOV 90.

Backend 1's submerged CPU cost is substantially higher because the legacy absorption cull would hide geometry that the clearer original optics can see. PVS/area/frustum culling still applies. This is a measured regression, not an underwater-shader-only cost. GPU intervals are asynchronous and overlap World; do not sum them. Memory snapshots are system-wide: above-water baseline-to-peak 1157-to-3462 / 1195-to-3509 MiB, submerged 1262-to-3654 / 1262-to-3623 MiB. One run per mode was measured. No competing game client was running, but an unrelated main-checkout Rust build was using CPU; it was left untouched.

## Original atmosphere and clouds (2026-10-16)

New `blue-water-render` and `blue-water-sim` facades provide the preserved standalone code's host services and original cloud/weather/sun types. `blue-water-sky` and `blue-water-clouds` compile the preserved upstream modules through path references. All imported source and shaders remain unchanged. glam 0.33.9 was already locally cached; the bridge converts JKA's glam 0.32 arrays at the boundary. No upstream build script or asset generator was executed.

The original High-tier atmosphere runs on JKA's device and renders to Rgba16Float. It is displayed through actual visible static BSP sky polygons, with main-view PVS/area/hide gates and a separate camera group for sky portals. Main scene geometry occludes it with reversed depth. The original environment atlas and its complete lighting block bind directly to the ocean, including sun/moon light and generation. The original aerial pass fills its froxel volume and sky buffer copy; the ocean samples it directly. A host composite samples the unchanged upstream aerial-volume functions for map geometry. It runs before water captures/transmission on the ocean path; without an optical water pass it runs after the main scene.

The original cloud layer owns its embedded shape/weather/blue-noise textures, GPU detail/turbulence fill, Beer shadow cascades, ground shadow map, quarter-resolution march, temporal reconstruction, shafts, haze and high sheet. Clouds enter the original environment bake through its original hooks. Current cloud shadows bind directly to the ocean. JKA's old cloud march/composite/shadow multiplier are disabled for cloud backend 1. The displayed cloud image uses the BSP sky mask, preventing sky clouds from covering indoor/foreground geometry. The map's baked land lighting remains authoritative; it does not acquire upstream per-material direct-light separation.

The ocean probe is now encoded after its current camera/boundary writes and FFT, before the cloud march. Cloud bindings change only when the probe texture changes; moving waterline values update each frame. The preserved march/composite both read this probe. Backends may be switched independently; the per-wave underwater mask requires ocean backend 1. Original cloud layer placement follows the upstream weather presets; existing coverage and wind controls feed it. JKA rain/snow/lightning remain the host weather path.

Validation so far: 17 core, 27 ocean GPU, 8 sim, 30 sky and 25 cloud unit/shader tests pass, plus separate sky and cloud GPU fixtures. The sky fixture checks finite day/twilight radiance and every environment prefilter level. The cloud fixture runs shadows, march, reconstruction and aerial fill together, then verifies that an all-wet probe removes all cloud/shaft/haze output. Client camera equivalence and both performance CSV tests pass. Current CSV schema has 183 fields, with backend settings and separate sky prepare/display, cloud image, aerial and underwater intervals. All these new intervals are inside World and excluded from fallback frame sums.

Thin-LTO visual checks completed normally without GPU errors: `atmosphere-clear-iteration` isolates sky 0/1 with legacy volumetric clouds off; `clouds-original-iteration` displays original sky/clouds/ocean; `aerial-clouds-iteration` adds the opaque aerial composite. The large old horizon band disappears with the legacy cloud layer. These are iteration captures, not final full-release performance results.

`scripts/blue-water-ocean-ab.ps1` now supports `-Compare ocean|sky|cloud|environment`, independent fixed backend values, `-NoClouds`, and an optional validation CFG. `environment` compares all-zero with all-one backends. `scripts/blue-water-analyze.py` validates normal exits, 180 CSV frames, backend values, binary hashes and exact PNG camera metadata, then emits p50/p95 summaries. `blue-water-runtime.cfg` exercises same-process reversals, mixed modes, HDR, MSAA and waterline transitions. Source ZIPs now read staged Git blobs directly so their contents exactly match the cumulative patch.

The same-process iteration validation completed in `target/blue-water-ab/environment-runtime-validation/` with exit 0 and no GPU validation errors. All runtime checks completed: restored JKA/Blue Water, authored sky with original clouds, original atmosphere with legacy clouds, legacy ocean with original clouds, LDR/HDR, 4x/1x MSAA, submerged, moving waterline and return above. (These labels describe validation steps, not a final performance comparison.) The final release additionally keeps physical sky camera/time updates active when the main PVS contains no sky polygons; BSP admission still bounds its display. This prevents stale aerial coordinates on indoor water. Use `r_fastsky 0` for the integrated path.

## Validated combined release and delivery

The final fully optimized release build passed in 8m54s. Executable SHA-256: `192bc995f9b2079b7ff4c0190dbd56590f537c7518390bfcc87ece8a826372e0`. The planned promoted-water, atmosphere and cloud bridge is connected on the Unified renderer. The experimental switches still default to 0. The limits described above remain: authored/secondary water, host weather, baked map lighting, BSP-masked cloud display, and neutral object-shadow/wake/hull/shore inputs where JKA has no corresponding data. This does not claim complete image parity with every standalone upstream rendering path.

All final pairs used this same binary, 1280x720 Vulkan, 15-second warmup, a 20-second FPS sample and 180 measured CSV frames. Their exact camera, angles, FOV, map and third-person state match within each pair. No GPU errors occurred; all clients exited normally. Representative captures were visually inspected. The final runtime check also passed restored/mixed backends, LDR/HDR, 4x/1x MSAA, submerged, moving waterline and return-above transitions. Camera holds after screenshot commands prevent the next teleport from racing the asynchronous image capture.

| Full-release comparison, backend 0 / 1 | 20-second FPS | Median CPU encode, ms | Median available GPU interval, ms |
| --- | ---: | ---: | ---: |
| Complete environment, above water (repeat) | 40.6 / 41.9 | 22.074 / 19.336 | 9.517 / 9.844 |
| Complete environment, submerged | 162.5 / 76.2 | 3.383 / 10.308 | 6.176 / 9.666 |
| Sky only, clouds disabled | 45.4 / 45.6 | 18.454 / 18.395 | 7.971 / 7.759 |
| Clouds, original sky/ocean fixed | 43.1 / 45.2 | 19.481 / 18.567 | 8.310 / 9.663 |

The first combined above-water pair measured 25.5 / 45.2 FPS and was unusually slow in backend 0 compared with the later same-process check. The repeat above measured 40.6 / 41.9 FPS. Both pairs are retained. Background CPU load was not controlled; a separate main-checkout Rust compiler process was left untouched. Do not generalize these single-pair FPS figures into an overall speed claim. In the repeat, original sky/cloud preparation is 2.120 ms, cloud image+dome 1.933 ms, aerial composite 0.081 ms and sky display 0.018 ms. The original late submerged effects are 1.794 ms median. GPU sub-intervals overlap World and are not additive independent frame costs.

Backend 1 remains substantially slower underwater. It submits 233984 water triangles versus 38912 and retains world geometry the legacy absorption cull hides; median CPU encode is 10.308 versus 3.383 ms. This regression is accepted as a measured limitation of this experimental bridge, not hidden by restoring the incompatible legacy cull.

Final targeted validation totals 123 passing tests: 17 core, 27 ocean GPU, 8 sim, 30 sky, 25 cloud, separate sky/cloud GPU fixtures, 11 client optics (including ignored GPU tests), camera equivalence, and two performance-export tests. Upstream preserved files were compared byte-for-byte against the local reference: no differences. Whitespace diagnostics remaining in the cumulative patch concern four unchanged upstream shader EOF blank lines.

Delivery artifacts are indexed by `artifacts/blue-water/LATEST.md` and `latest.json`. The cumulative source ZIP, binary Git patch, file manifest and SHA-256 digest are created with `blue-water-checkpoint.py`. The validation ZIP contains the full-release screenshots, exact-camera metadata, FPS/CSV/JSON captures, runtime CFGs, relevant build/test logs and per-file hashes. `comparison.html` is an offline interactive wipe viewer for the four final comparison pairs. `blue-water-package-validation.py` creates the viewer/archive and verifies the archived file bytes. The original checkout and its running compiler were left untouched; no commit, merge or push was made.

## Hands-on testing

Run `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/blue-water-play.ps1`. It launches the final release in costadelsol with all three original backends enabled, ordinary mouse input, and free flight. The dedicated profile is `target/blue-water-play/base`; the benchmark scripts, timed quit and zero-sensitivity settings are not used. The console `noclip` command returns to normal movement. Toggle `r_oceanBackend`, `r_skyBackend` and `r_cloudBackend` independently with 0/1. This is ready for user testing; full standalone visual parity and the known gaps above are still outstanding. Automated testing stopped at the user's request.

## First user water review

Water testing now defaults to clouds off, including the interactive launcher and A/B helper (explicit `-NoClouds:$false` is required for future cloud comparisons). Cloud work is deferred until water is finished.

The user reported a distanceCull gap, missing caustics/refraction and poor underwater visibility. The original mesh assumed a far plane long enough to hold its regular rings and only fitted the horizon/skirt to the far plane. JKA can clip its regular rings much sooner, leaving a gap before that skirt. The composed JKA variant now fits all infinite-water ring vertices to the host depth range, preserving physical shading coordinates, finite-pool behavior and BSP admission. It does not expand world-geometry distanceCull.

The original repo includes a four-depth wave-caustic generator, designed for submerged receiver materials. It has not yet been connected to JKA receivers; the existing JKA caustic approximation is a separate path. Original refraction covers the sky in the surface underside/Snell window; scene transmission samples the opaque scene at the same pixel. Old-style underwater scene distortion is not supplied by that upstream pass and needs explicit integration.

The bridge still used temporary absorption/transmission defaults and overrode the underwater pigment with a JKA map tint. That is not the original optical calibration. Backend 1 now uses the original North Atlantic colors (water 0x2d373c, transmission 0x49575e, absorption 0x945b57, converted to linear RGB), with the original underwater clarity 0.15. Underwater visibility remains subject to user review; previous captures are not a baseline for the corrected calibration. No further automated runs were started; a build and user testing follow these fixes.

The feedback build passed in 1m41s using the existing thin-LTO iteration command. It was reopened for the user with clouds off; no automated captures, timings or test suites were run for this revision. Executable SHA-256: `72470c51c29b0a257deac6015d1a48d35a6fb9b19457fbabd99c075443cae98f`.


## Blue Water menu starter (2026-10-16)

Video > Environment > Blue Water now selects the ocean, sky and cloud backends independently, enables the simulated ocean and clouds, and exposes the controls currently consumed by the port: FFT update rate, foam tint, surface refraction, caustic receiver strength, cloud coverage and cloud shadows. The Clouds and Water pages link back to this selector. Cloud coverage and shared Weather wind feed the original cloud scene.

The page calls out the remaining renderer mismatches. Blue Water currently constructs a fixed High-tier, three-cascade 256x256 field with a fixed upstream spectrum and North Atlantic underwater calibration. Existing authored swell amplitude, wavelength and direction, FFT-size choice, water-body tint, underwater fog color and transparency still configure the Unified path only. Blue Water cloud placement/quality likewise follows its upstream North Atlantic preset; the JKA cloud type, height, thickness, render resolution and raymarch controls are not routed into it. The imported Blue Water field still needs user-facing sea-state and quality controls before those settings can be claimed as supported.

The earlier underwater feedback paragraph saying the original caustic generator was not connected is now superseded by the current source: the original surface path encodes the generator and draws it on opaque receivers. This menu exposes its strength; the visible result still needs a fresh user review.

No build or test suite was run for this menu change.


## Original caustics and scene refraction (2026-10-16)

The unchanged upstream caustic compute shader now generates its four FFT-derived depth layers (2, 5, 12.5 and 31.25 metres; 256 square texels each). A new JKA receiver pass reconstructs the nearest opaque BSP/model surfaces from copied depth and applies the original depth-blended gain. It checks the real water volumes, pool bottom, exact footprint/PVS admission and qualified exterior extensions, rejects dry/sky pixels and depth discontinuities, and attenuates the result with depth and incident light. Like upstream, it fades with camera height from 12 to 25 metres above the pool. It runs before opaque aerial lighting; the later surface scene copy includes both caustics and aerial lighting. Transparent receiver materials are not connected. JKA baked lighting is modulated rather than split into the standalone renderer's direct and ambient material lighting.

The JKA surface variant now traces refracted rays against copied opaque scene depth on both sides of the original wave surface. It uses the original FFT normals, water index 1.333, Fresnel/Snell window and optical attenuation. The ray trace is a new host extension: the supplied upstream surface refracted sky and used the same-pixel opaque scene for transmission. Original shaders in vendor remain unchanged. The trace rejects foreground discontinuities and invalid/off-screen hits, then falls back to the original scene/sky treatment. It cannot recover objects absent from the screen depth buffer. This bends views through the surface; it does not add a full-screen underwater wobble.

Controls are live on backend 1: r_oceanCaustics 0 disables receiver modulation (1 is its default strength); r_oceanRefraction 0 disables scene bending and 0.1 selects the default physical direction. The interactive launcher keeps clouds off. Shallow submerged ground and objects are the useful first visual check; stronger depth attenuation makes caustics less apparent in deep water. Visibility, refraction transitions and the final appearance await user review.

Validation was limited to a client compile and one focused GPU check, caustics_and_refraction_gpu_bindings_accept_flat_field: it compiled the surface/refraction and receiver pipelines, dispatched all four original caustic layers, verified flat-field gain 1 and encoded the receiver fixture without GPU validation errors. No benchmark sweep or automated captures were run for this revision. The build uses the thin-LTO iteration profile for user testing. The existing Blue Water menu work is included; its Water-page missing VideoSection import was repaired during the build.


## Native Blue Water wave sliders (2026-10-16)

Video > Environment > Blue Water now exposes five shared native spectrum controls: significant wave height (0-20 m), swell travel direction (-180 to 180 degrees, +X toward world +Z), peak wavelength (2-512 m), horizontal choppiness (0-4) and spectrum wind speed (0.1-40 m/s). Reset Waves restores the previously resolved upstream realistic-sea preset: height 1.8 m, direction 35 degrees, approximately 70 m peak wavelength, upstream breaking-limit choppiness and wind 9 m/s. The controls affect the shared promoted-water FFT and its surface, normals, foam and caustics on subsequent simulation updates. They do not provide independent per-pool sea states.

The bridge now supplies explicit native WaveParameters instead of calling the automatic realistic sea-state policy every update. This allows wavelength and choppiness to remain independent of height and wind. The default keeps the previous preset's resolved spectral shape, gamma and equilibrium-range mode. FFT tile growth uses the upstream wavelength-based rule for explicit parameters as well, so long wavelength settings have room in the existing three cascades without reallocating textures. No preserved vendor source or shaders were changed.

The five values are sanitized, propagated through shared pool settings, and archived through r_blueWaterHeight, r_blueWaterDirection, r_blueWaterWavelength, r_blueWaterChoppiness and r_blueWaterWindSpeed. The existing commit path updates the renderer, marks the config dirty and refreshes the UI. The old authored swell and cross-sea / FFT blend remain separate Unified controls. Wave phase/spectrum changes can be visible immediately while dragging; no temporal morph between spectra is implemented. The fixed High quality tier and previously documented optical controls remain unchanged.

Client cargo check passed. The testing executable uses the existing thin-LTO iteration build; visual slider review is delegated to the user, with clouds off and no benchmark/capture sweep.


## Blue Water transparency and water/fog colors (2026-10-16)

The Blue Water menu now exposes Transparency (0.1-10x), Water Color and Underwater Fog Color, with Reset Water Appearance. All apply live, are shared across promoted pools and save to config. Default values preserve the existing North Atlantic calibration: transparency 1x, water #49575e and underwater fog #2d373c. Native spectrum and optical appearance settings are independent; appearance changes do not rebuild the FFT. The legacy Water-page controls still configure the Unified renderer.

Transparency divides the calibrated absorption coefficients by the selected multiplier. It changes the above-water transmission column, underside attenuation, bounded underwater absorption/scattering and caustic depth attenuation. Original underwater clarity remains 0.15. The surface keeps physical Fresnel reflections, so increasing transparency does not remove reflected sky. The control is an optical travel multiplier, not a constant alpha or visibility measured in metres.

Water color is converted from sRGB into the original transmission swatch, which colors transmitted crest light, submerged foam tint and the waterline film. The JKA-composed surface additionally applies its ratio against the calibrated transmission swatch to deep-water upwelling. This gives a visible water-body tint without recoloring reflected sky or the copied scene objects. The default ratio is one. Underwater fog color supplies the original pigment used by underwater scattered light, motes and the underside's deep reflection. These are material inputs to the original model, not a screen-wide color overlay. Preserved vendor shaders are unchanged.

The new archived cvars are r_blueWaterTransparency, r_blueWaterWaterColor and r_blueWaterFogColor. Color values are three sRGB components in 0-1. Settings are sanitized and propagated through shared promoted-water settings before the surface, caustic receiver and underwater bindings are written.

Client cargo check and the existing focused caustics/refraction GPU fixture passed, including the new composed water-body tint. No benchmark/capture sweep was run. The iteration executable build passed in 1m25s and is ready for user visual review with clouds off; current visual acceptance remains with the user.

## Performance hybrid after underwater review (2026-10-16)

The user preferred Godot's underwater view and caustics, and allowed BlueWater visual reductions with A/B cvars. The default BlueWater profile now uses 64x6 surface geometry, exact boundary checks accelerated with 8x8 tile candidate masks, SSR off, native caustic receiver off, native scene refraction off, and the submerged hybrid mode. Above the surface it keeps BlueWater's original surface. While submerged it uses the Godot clipmap surface, underwater optics, absorption cull and inexpensive optics caustics, with BlueWater FFT wave textures still feeding the legacy shader. The Godot clipmap updates only when the hybrid actually needs it underwater (or when its A/B cvar is enabled). The original native paths remain selectable.

The earlier BlueWater defaults measured 53.6 FPS versus Godot 108.4 FPS at 1920x1080. With the optimized profile before the submerged surface switch, matched above-water pairs were Godot 102.0 and 102.8 FPS versus BlueWater 103.9 and 104.8 FPS. The first hybrid surface revision unnecessarily prepared Godot mesh indices above water and measured Godot 101.5 versus BlueWater 98.0 FPS. That extra update has since been removed, but two final pairs still measured Godot 103.8/101.0 versus BlueWater 98.8/96.7 FPS. GPU time remained approximately equal or faster for BlueWater. The above-water equal-FPS goal is therefore still open; see `docs/blue-water-perf-ab-handoff.md` for exact timings and artifacts.

The matched submerged hybrid surface run at settled camera `(-2384 11529 -135)`, yaw 292, pitch -12 measured Godot 111.7 FPS / 7.603 ms GPU and BlueWater 111.1 FPS / 6.187 ms GPU. This replaced the native submerged BlueWater run of 69.2 FPS / 12.52 ms GPU. The Godot optics and cull alone previously reached 123.9 FPS but left a dark BlueWater underside; the hybrid Godot surface restores the bright blue underwater view. Its ceiling has broader, less detailed wave patterns than pure Godot, because the BlueWater field remains the wave source. The user should review that difference, the waterline switch, and caustics on shallow ground in motion. The performance captures establish parity only at these cameras.

Use `r_blueWaterUnderwaterMode 4` for hybrid (default), `0` for full native or `3` for native effects only when submerged. `r_blueWaterCausticsMode 2` disables only the expensive native receiver (default); `0` restores it. `r_oceanCaustics 1` keeps the inexpensive Godot optics caustics. Other A/B controls are `r_blueWaterSSR`, `r_blueWaterMeshSegments`, `r_blueWaterMeshLevels`, `r_blueWaterBoundaryMode`, `r_blueWaterRefraction` and `r_oceanBackend`. Screenshot, CSV and log artifacts are under `target/blue-water-ab/perf21-hybrid-surface-submerged/` and `target/blue-water-ab/perf21-hybrid-final-above/`.

### Interactive profile correction

After a user reported roughly 200 FPS on Godot versus 100 FPS on BlueWater immediately after joining, the saved `target/blue-water-play/base/DinurdoJK.cfg` was inspected. It had Godot `r_oceanMapSize 256` and `r_oceanMeshQuality 0`, versus the benchmark's 1024 and 1. It also had BlueWater height 20 m, wavelength 370 m and choppiness 1.55, versus the benchmark's 1.8 m, 70.440796 m and 2.2543192. The old interactive launcher selected BlueWater sky, while the ocean A/B held Godot sky and clouds off. These differences mean the previous interactive session did not reproduce the benchmark profile; they do not invalidate the user's 200/100 observation for that profile and location.

`scripts/blue-water-play.ps1` now loads `scripts/blue-water-optimized.cfg`, uses Godot sky with clouds off, disables VSync and defaults to 1920x1080. The preset explicitly applies the optimized BlueWater controls and measured waves, preserving the saved Godot FFT size and mesh quality. It retains normal mouse input and free flight. Its width/height are parameters for other displays. PowerShell parsing passed; no new FPS capture was run for this launcher edit.

The running main-checkout executable was verified by SHA-256 to match the experimental build (`7195cb46235d30b39c54c7e0b5bc3f3ff651005903dc06c1826064785b6005b3`). Its saved main base profile differed further: native caustics mode 1, boundary mode 2, adapter off, 20 m waves, 512 m wavelength and 2.3x transparency. `blue_water_optimized.cfg` was installed in `D:/Code/DinurdoJK/target/release/base/` as well as the existing experimental release/play base directories, so the user can apply it in the current game with `exec blue_water_optimized.cfg`. It does not move the camera, change resolution or raise Godot quality. Compare `r_oceanBackend 0` and `1` at the same stationary camera. The 200/100 report is not yet reproduced in a controlled capture; no benchmark was launched alongside the user's running game.

For an interactive replay of an actual capture, use `scripts/blue-water-replay.ps1 -RunName perf22-user-view-restored-optics`. It verifies the executable hash, copies the capture's archived config into a separate replay base, links its PK3 files (including Costa), restores normal mouse input and replays the capture's requested camera. It has no automatic captures or quit. `r_oceanBackend 0` and `1` switch the ocean in that exact graphics profile. The current capture runs near 40 FPS while the user's ordinary profile is reported near 150 FPS; that discrepancy is unresolved and these timings must not be represented as a reproduction of the user's normal performance. The replay command makes this difference inspectable.

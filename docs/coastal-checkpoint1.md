# Coastal water — Checkpoint 1

Implemented on October 8, 2026. This checkpoint supplies BSP bathymetry and boundary diagnostics. The numerical coastal solver and shoreline rendering have not been started. Promoted and authored water still use the existing rendering, FFT, optics and interaction paths; multiplayer collision is unchanged.

## Deliverable and inspection

The saved checkpoint is `target/coastal-checkpoint1/checkpoint1.zip`, with a patch against the saved **starting working files**, source snapshots, hashes, validation logs and real-map reports. Existing unrelated working-tree changes are excluded from the patch. Resume from the current source and this checkpoint.

Open these self-contained reports in a browser:

- [RaceArena_Pro, resolved client materials/PVS](../target/coastal-checkpoint1/runtime/racearena_pro/bathymetry.html)
- [yavin1, resolved client materials/PVS](../target/coastal-checkpoint1/runtime/yavin1/bathymetry.html)
- [yavin1 preview](../target/coastal-checkpoint1/yavin1-preview.png)

Select a slot, switch between classification, depth, bed elevation and connected wet components, and hover to inspect coordinates, measured depth, ceiling, bed variation and footprint coverage. Wall faces, unsupported boundaries and original footprint triangles have independent switches. The debug visualization is an exported map view, not a WGPU overlay.

After rebuilding the client, enable the existing ocean option, load a BSP, and enter `coastal_debug`. It prints worker status, measurements and the map's report path. The static worker never changes the water draw. Disabling ocean skips the coastal job on the next preparation; already-running static work can finish in the background.

The packaged headless `coastal_probe.exe` accepts:

```powershell
target/coastal-checkpoint1/coastal_probe.exe target/release/base yavin1 target/coastal-checkpoint1/maps/yavin1
```

This tool discovers raw BSP water-content surfaces. The ignored client test additionally verifies actual runtime shader resolution, primary water stages and PVS batches. On both tested maps, classifications, depths, face boundaries and connected components matched between these paths.

## Geometry contract

`jka-assets::bsp::bathymetry` is independent of movement and WGPU. Inputs and outputs use JKA XY with Z up. Client footprints convert from render XZ with `[x, -z]`; one ocean metre is 40 world units.

The client uploader and background job share face selection and the existing clustering algorithm. Shader identities, authored-ocean separation, slot ordering and the eight-surface draw cap retain their previous behavior. Exact footprint triangles are retained independently of cluster bounding boxes.

Solid and terrain brushes are intersected analytically with vertical columns using every convex brush plane. Axial planes provide conservative acceleration bounds. Caulk/nodraw solids participate. Solid planar surfaces, triangle terrain and nodraw patches also contribute; patches use eight segments per quadratic span. Water/lava/slime and movement-only clip brushes do not provide seabed. Sky surfaces do not supply a fabricated bottom.

Water brushes identify the relevant vertical layer for stacked map geometry. Their bottom is never used as a floor or a depth limit. Overhead structures retain a measured ceiling. Ambiguous submerged layers retain explicit fallback classification.

The grid defaults to 16 world units (0.4 m), with two padding cells and a 262,144-cell budget per body. Each cell records center bed/depth, five-probe bed extrema, ceiling, footprint coverage, classification and supported wet component. Unknown bed/depth are NaN in the binary data and `null` in the report, never an assumed deep ocean.

Staggered X/Y face records distinguish open connections, solid walls, footprint boundaries and unsupported geometry. Exact convex segment clipping and triangle intersections catch thin walls between cell samples. Exact unions of triangle/segment intervals prevent connectivity across narrow footprint gaps. Wet connectivity uses these face barriers. External grid boundaries are sealed.

Partial footprint, mixed geometry, missing floor and ambiguous overhang cells are deliberately excluded from supported wet components. The five probes are **not** an exact area/volume fraction, and subcell geometry is not fully resolved by this grid. Future simulation/reconstruction must refine these cells or use the retained footprint geometry. Moving inline models are excluded from static bathymetry.

No wave attenuation, horizontal shoreline-distance fade, mesh skirt or physics change is present.

## Actual map results

Depths below are in JKA world units. Component counts describe conservative supported cells, including fragmentation around unresolved edges.

| Map | Slot | Water Z | Supported wet cells | Measured depth | Wet components |
|---|---:|---:|---:|---:|---:|
| RaceArena_Pro | 1 | 48 | 19,956 | 24 | 8 |
| RaceArena_Pro | 2 | -383 | 9,670 | 33 | 6 |
| RaceArena_Pro | 3 | 221 | 8,791 | 13–29 | 3 |
| RaceArena_Pro | 4 | 81 | 4,132 | 17 | 6 |
| RaceArena_Pro | 5 | 89 | 11,856 | 25 | 1 |
| RaceArena_Pro | 6 | -1251 | 11,218 | 21 | 2 |
| RaceArena_Pro | 7 | 441 | 7,175 | 25 | 1 |
| yavin1 | 0 | 312 | 37,797 | 24 | 6 |
| yavin1 | 1 | 136 | 4,503 | 0.627–8 | 53 |

RaceArena_Pro slot 0 would require 12,787,384 cells at this spacing. It is explicitly rejected with a legacy-water fallback warning. Tiled/sparse treatment of that large body remains future work. RaceArena slot 1 also has 286 missing-floor cells; the report shows them instead of inventing a bed. The yavin river has explicit overhang and mixed cells, and dry terrain/islands stay separate from supported water.

`mp/ffa3` supplies a no-water control: no raw water faces and no coastal grids. Its compiled zero-volume brush remnants no longer abort geometry extraction.

## Measurements and validation

Optimized headless measurements include geometry extraction and grid baking, excluding BSP parsing, asset loading, report export and cache IO. Release used optimization with LTO disabled and 16 codegen units. These are individual local runs, not a distribution or frame-time benchmark.

| Map | Geometry bake | Retained grid/footprint data | Binary cache | Decode |
|---|---:|---:|---:|---:|
| RaceArena_Pro | 622.326 ms | 6,840,244 bytes (6.52 MiB) | 5,593,646 bytes | 7.099 ms |
| yavin1 | 337.222 ms | 5,012,936 bytes (4.78 MiB) | 4,103,683 bytes | 5.228 ms |

Client cache-hit validation measured 0.316 ms / 0.734 ms of face selection and descriptor preparation on the map loader. The unoptimized background worker, including decode and report IO, took 193.797 ms / 149.554 ms. Earlier unoptimized cold workers took about 6.69 s / 2.26 s. Map preparation returned without waiting for this work. There are no per-frame coastal traces, GPU passes or GPU readbacks in this checkpoint.

Memory above measures retained arrays, not total process or peak transient memory. The worker temporarily retains the parsed BSP and acceleration data. Geometry reference bins are bounded at eight million references per bin set; oversized inputs fall back explicitly.

Validation completed:

- 11 focused bathymetry tests: analytic basin/slope, water/clip exclusion, nodraw terrain patch, brush and triangle walls, narrow gaps, holes, separate heights, stacked layers/ceilings, bounds, cache corruption and identity.
- 13 existing ocean tests, including grouping, masks, clipmap bounds, winding and LOD behavior.
- Resolved client map test on both maps, with cold generation and subsequent cache hits.
- Binary-cache round trips, report data agreement, browser render and screenshot inspection.
- Final full-client `cargo check` passed with existing warnings. Renderer edits made concurrently during this task were preserved and excluded from the checkpoint patch.
- Broader asset-library run: 132 passed, 1 failed, 5 ignored. The unrelated failure is `skin::tests::comma_parser_and_off_rules_match_register_individual_skin`; this checkpoint does not edit `skin.rs`.

Drive D had about 638 MB free, so validation builds used `C:\Users\David\AppData\Local\Temp\dinurdojk-coastal-build`, debug symbols disabled and incremental compilation disabled. The existing local Steam Audio libraries were reused in that temporary target; no SDK or dependency changes were made.

## References and next checkpoint

Saltreach was read from the supplied local `D:\Code\coastal-simulation-main`: `simulation.js`, `solver-kernels.ts`, `surface.js`, `coast.js` and terrain setup. Its bed/state separation, shared flux/outflow limiter and wet-neighbor surface reconstruction guide the next solver checkpoint. These algorithms have not yet been ported. [Upstream Saltreach](https://github.com/iamtechartist/coastal-simulation).

Crest's static cache records terrain height for later depth calculation. Its `OceanDepthCache.cs` capture path and `LodDataMgrSeaFloorDepth.cs` storage were inspected; the BSP bake follows that separation while explicitly handling solid volumes and ambiguous geometry. [Crest cache source](https://github.com/wave-harmonic/crest/blob/master/crest/Assets/Crest/Crest/Scripts/LodData/OceanDepthCache.cs), [depth storage](https://github.com/wave-harmonic/crest/blob/master/crest/Assets/Crest/Crest/Scripts/LodData/LodDataMgrSeaFloorDepth.cs).

Depth relative to wavelength must govern later wave model selection. Intermediate depth requires dispersive treatment; solid walls remain barriers independently of depth. The future one-way FFT/coastal interface must replace or blend the incoming free surface consistently, rather than adding full FFT displacement twice. [Crest shorelines documentation](https://crest.readthedocs.io/en/stable/user/shallows-and-shorelines.html).

Checkpoint 2 should port and validate the conservative solver independently in synthetic basins before connecting any of these map regions. GPU layout, CFL stepping, exact wet/dry reconstruction, refinement of partial cells and handling of large sparse regions remain subsequent work. This checkpoint does not claim to fix exposed displaced-water edges yet.

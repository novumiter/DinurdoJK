# DinurdoJK coastal water — Phase 2 handoff

## Task to execute

Implement **Checkpoint 2: independently validated coastal solver correctness**. Continue from the current working tree and the completed geometry checkpoint. Port the supplied Saltreach solver faithfully, validate it in synthetic domains, and save a reviewable on-disk checkpoint before starting live FFT coupling or shoreline rendering.

The user has requested this handoff, not implementation of Phase 2 during the handoff-writing turn.

The overall goal remains physically grounded coastal water for the Rust/WGPU Jedi Academy client: existing FFT behavior in deep water, appropriate depth/wavelength treatment toward shallows, solid banks, and eventual wet/dry shoreline movement without exposed water edges. Preserve authored water, reflections/refraction, foam, fog, underwater optics, interactions, LOD, masks and multiplayer physics. Do not introduce a shoreline-distance fade as a substitute for physical foundations.

## Workspace and references

- Main workspace: `D:\Code\DinurdoJK`.
- User-supplied local Saltreach source: `D:\Code\coastal-simulation-main`.
- [Completed geometry checkpoint report](coastal-checkpoint1.md).
- Saved checkpoint: `target/coastal-checkpoint1/checkpoint1.zip`.
- Starting-working-file snapshots, patch, before/after hashes and patch replay evidence are in `target/coastal-checkpoint1`.
- Saltreach source hashes are in `target/coastal-checkpoint1/reference-manifest.json`; inspect the current reference against them before porting.
- [Saltreach upstream](https://github.com/iamtechartist/coastal-simulation).
- [Crest shorelines/depth documentation](https://crest.readthedocs.io/en/stable/user/shallows-and-shorelines.html).
- Crest reference files: `crest/Assets/Crest/Crest/Scripts/LodData/OceanDepthCache.cs` and `LodDataMgrSeaFloorDepth.cs` in [wave-harmonic/crest](https://github.com/wave-harmonic/crest).

The working tree already contains substantial unrelated changes, including renderer work performed concurrently with Checkpoint 1. Inspect status and any applicable repository instructions first. Preserve those changes. Do not reset the repository, replace files with upstream originals, or replay the old checkpoint patch over the current implementation. Take a fresh baseline for Phase 2 so its deliverable excludes prior work.

## Current implementation

Checkpoint 1 is static geometry and diagnostics only. There is no coastal fluid solver, no FFT/coastal coupling, and no new coastal draw or compute pass.

Read these files before making changes:

| File | Purpose |
|---|---|
| `crates/jka-assets/src/bsp/bathymetry.rs` | BSP solid/terrain extraction, depth grids, face barriers and wet connectivity |
| `crates/jka-assets/src/bsp/bathymetry/{cache,report,promotion,tests}.rs` | Cache identity/integrity, diagnostics, shared legacy grouping and tests |
| `crates/jka-assets/examples/coastal_probe.rs` | Standalone actual-map validation |
| `crates/jka-client/src/ocean/bathymetry.rs` | Background bake/cache/report job; runtime map test |
| `crates/jka-client/src/ocean/surfaces.rs` | Shared promoted-face selection used by baking and upload |
| `crates/jka-client/src/scene/prepare.rs` | Starts the background job at the end of BSP preparation |
| `crates/jka-client/src/renderer/world/upload.rs` | Existing promoted clipmaps, surfaces and masks |
| `crates/jka-client/src/ocean.rs` | Existing FFT resources, settings and scheduling |
| `crates/jka-client/src/ocean/appearance.rs` | Existing promoted-water appearance |
| `crates/jka-client/src/bsp.wgsl` | Existing water shading/displacement |
| `crates/jka-client/src/ocean/interaction.rs` and `ocean_interaction.wgsl` | Existing visual interaction field; a separate system |
| `crates/jka-client/src/renderer/environment/water.rs` | Existing water enable/disable and LOD behavior |
| `crates/jka-assets/src/bsp/collision.rs` | BSP collision-tree representation; gameplay collision is unchanged |

`coastal_debug` prints static bake status, classification counts, depths, memory and an interactive HTML report path. It does **not** enable fluid simulation. There is no independent coastal toggle yet: job creation follows the existing `r_ocean` option at map preparation. Turning ocean off prevents new jobs on subsequent preparation; a running static job can finish.

## Geometry contract and limitations

Bathymetry uses **JKA XY, Z up**, in world units. Renderer positions are `[x, z, -y]`; render XZ footprints convert back to JKA XY with `[x, -z]`. The existing ocean uses **40 world units per metre**. Choose and document a single solver unit convention. SI units are suitable for the upstream `9.81 m/s²` gravity; convert all bed heights, depths, grid spacing, forcing and velocities consistently. Use a local water-level origin where appropriate to avoid large coordinate offsets reducing precision.

`Grid` carries a slot, nominal level, XY origin, dimensions, spacing, cells, staggered `faces_x`/`faces_y`, connected-component labels and retained exact footprint triangles. `Cell` includes classification, bed, depth, ceiling, bed probe range, five-probe footprint coverage and component. The classes are `Outside`, `Wet`, `Dry`, `Outline`, `NoFloor`, `Overhang` and `Mixed`. Unknown heights/depths are NaN in binary data and null in exported reports.

The default grid spacing is 16 world units (0.4 m), with two padding cells and a 262,144-cell budget per region. Solid and terrain brushes supply analytically intersected bed heights; water brushes identify vertical layers but **never supply a floor or a depth limit**. Curved patches are tessellated at eight segments per quadratic span. Moving inline models are outside the static bake.

Important constraints for the next agent:

- Five footprint probes are not exact area/volume fractions. Corner extrema are samples, not a proof that arbitrary subcell geometry is fully resolved.
- Partial footprints, missing floors, mixed cells and ambiguous submerged layers are excluded from supported wet components. Never replace unknown depths with zero or a guessed deep-water value.
- Face barriers preserve thin solid walls and narrow footprint gaps; exact source triangles remain available for later refinement/reconstruction.
- **Current `Wall` diagnostics also include faces adjacent to dry solid cells and solid intersection at nominal water height. They are not yet a complete distinction between an impermeable vertical wall and an overtoppable bed step/sloping bank.** Before map run-up integration, refine that distinction using actual geometry. Do not seal every wet/dry terrain interface merely because it is labelled `Wall`, and do not open a true wall merely because the adjacent water is deep.
- Separate draw clusters are not a complete physical-connectivity model across shader boundaries. Supported components are conservative grid connectivity, not a guarantee that every physical fluid connection has been resolved.
- Ceilings are retained; this does not implement closed-roof fluid physics. Overhangs that cannot be represented safely need fallback.

Phase 2 can validate the numerical solver on fully specified synthetic domains without resolving every map-adapter limitation first. Document those limitations explicitly and do not claim live shoreline readiness.

## Actual user validation and performance priorities

The user ran `coastal_debug` on **RaceArena_Pro** in the active jaPRO installation and confirmed the map identity. Their log reported:

- Seven baked regions, 6,840,244 retained bytes (6.523 MiB).
- Geometry bake: **663.214 ms**.
- Background worker including additional work: **714.67 ms**.
- Face selection on the loader: **0.038 ms**.
- Report: `D:\Code\DinurdoJK\target\release\japro\cache\coastal\v1-e1bc5f53cd98e325-d0c58bcf311aff17\bathymetry.html`.
- RaceArena slot 0 would require **12,787,384 cells** and remains on the legacy path because it exceeds the budget.

These are static bake measurements, not solver timings or an FPS benchmark. After baking, Checkpoint 1 adds no per-frame coastal compute/draws/traces/readbacks. Background CPU/memory contention can still transiently affect frame times; it was not measured. Cached loads avoid geometry rebaking.

The user cares strongly about extremely high frame rates and low latency. A recurring solver cost must be measured separately from rendering and from map preprocessing. Do not infer an acceptable FPS loss from the static bake numbers.

Two preferences were asked but remain **unanswered**:

1. Which RaceArena water area should be the first live integration target, especially whether the large slot-0 body is the priority.
2. What minimum FPS the user wants to maintain with coastal water enabled.

These do not block independent numerical validation. Do not invent a user-approved FPS budget or live-pool choice. Slot 6 has two supported components and zero unresolved cells in the recorded diagnostics; slot 5 has one component with some unresolved edge cells. They are candidates for later inspection, not selected integration targets.

## Faithful solver port

Read the complete local Saltreach implementation, especially:

- `src/simulation.js`: `ShoreSimulation`, `step`, `prepareBoundary`, and the separation of fluid integration from transport/material effects.
- `src/solver-kernels.ts`: pressure/face-velocity updates, donor flux, positivity limiter and divergence update. This uses f64 intermediates with f32 stores to match JavaScript typed arrays.
- `src/surface.js`: wet/dry surface reconstruction and wet-neighbor normal rules.
- `src/coast.js`: grid, wave and unit assumptions.
- `src/solver-accelerator.js` and `src/worker.js`: accelerated/reference execution and worker scheduling.
- `LICENSE`: MIT attribution requirements. If code/equations are adapted, record provenance and include the required notice; update the project's third-party documentation without overwriting existing entries.

Port the conservative staggered formulation: cell water depth, face velocities, terrain-dependent face depth/crest, donor-cell fluxes, a shared outgoing-volume limiter, and conservative divergence. A face flux must be shared by its two neighboring cells. Avoid independent cell-side clamps that silently destroy conservation.

Establish a reference-parity path first. Keep the upstream constants, stage order, f64/f32 behavior and optional terms identifiable. Generate reproducible fixtures from the supplied source where practical. Compare intermediate face velocities, fluxes, limiter values and next depths as well as final surfaces. Explain any necessary boundary-index/layout translation.

Then make justified, separately tested adaptations for arbitrary basin geometry, explicit walls, configurable forcing/damping, robust timestep selection and eventual GPU execution. Do not mistake the upstream velocity clamp for a complete CFL condition. Derive stability limits using propagation speed and relevant velocity bounds, support bounded substeps, and handle long frame gaps without an unbounded catch-up stall.

Upstream sponge forcing, damping, foam/material transport and tiny-residual drainage are not all water-conserving. Disable sources/sinks for closed-basin conservation tests, or explicitly account for their injected/removed volume. Do not hide numerical loss behind drainage or damping. Surface reconstruction must not create water in dry cells or average water levels across solid obstacles.

Keep the pure solver independent of renderer and network state. A worker-thread CPU reference is acceptable for this checkpoint. Choose module/crate boundaries using repository conventions; do not add a large dependency or modify player movement to implement visual water.

## Required numerical validation

Use deterministic initial conditions, documented units, boundary conditions, timesteps, durations and justified error tolerances. Report error values and convergence where appropriate; do not choose tolerances solely to make the implementation pass.

| Case | What it must establish |
|---|---|
| Flat closed basin | Resting free surface, zero flow and water conservation |
| Sloping/stepped bed at rest | Lake-at-rest behavior without terrain-generated currents |
| Raised dry terrain beside water | Positive depth, stable wetting/drying and overtopping only when physically allowed |
| Small wave or localized disturbance | Propagation and convergence in the shallow-water regime |
| Vertical wall, including a thin barrier | No through-wall flux; sensible reflected-wave behavior |
| Irregular footprint / island / separated basins | No leakage or artificial connectivity |
| Wet front / dam-break style initial state | Stability and positivity under steep gradients |
| Boundary forcing and sponge | Known phase/time input, bounded response and accounted volume changes |
| Large timestep or frame gap | Bounded work, finite state and explicit stability handling |

Track total volume, source/sink volume, minimum/maximum depth, maximum velocity, non-finite count, timestep/substep counts and numerical error. Add a headless executable or reproducible harness that exports time histories and diagnostic plots/visualization of synthetic results.

## Performance and GPU path

Measure the CPU reference in an optimized build across representative grid sizes and active-cell counts. Report time per step, time per simulated second, fixed-step frequency, substep behavior, memory and hardware/build settings. Separate initialization from recurring work. Avoid logging or allocating large scratch arrays every step.

Evaluate the WGPU layout and dispatch sequence for the final scalable implementation. Preserve dependencies between pressure/flux construction, outgoing limit, shared flux limiting and divergence updates. Specify ping-pong ownership and synchronization. GPU parity tests are useful if feasible within this checkpoint; only claim an implemented GPU solver when it is actually validated.

Do not require blocking per-frame GPU readbacks for CFL decisions or rendering. A later runtime scheduler must use fixed simulation steps, smooth presentation interpolation, bounded work and a measured budget. GPU timings must be reported separately from CPU submission and water rendering when available.

No live ocean integration is required to complete Phase 2. If adding a diagnostic GPU prototype, keep it opt-in and independently disableable. Leave the default game path unchanged. The later live coastal toggle must be independent of `r_ocean` so coastal cost can be compared while retaining FFT rendering.

## Subsequent integration boundaries

FFT coupling, live shoreline reconstruction and large-body tiling belong to later checkpoints. Preserve their requirements in the solver design:

- Wavelength relative to actual depth governs bottom influence; shallow-water physics must not be applied indiscriminately in intermediate depth.
- Solid-wall behavior applies independently of wave depth regime.
- One-way FFT-to-coastal forcing is acceptable initially, with documented limitations and consistent time/phase.
- The coastal free surface must replace/blend the incoming wave field at a controlled interface. Do not add a full FFT displacement on top of an already forced coastal solution.
- Unsupported geometry remains on a safe legacy path. Refining wet/dry cells and handling slot 0 need deliberate geometry work, not automatic coarsening that erases important banks.

## Verification and completion

The existing geometry tests and ocean regressions must continue to pass. At Checkpoint 1, 11 focused bathymetry tests, 13 existing ocean tests and actual runtime map validation passed. The broader asset-library run had an unrelated failure in `skin::tests::comma_parser_and_off_rules_match_register_individual_skin`; record it separately if still present.

Useful commands, from the workspace:

```powershell
cargo test -p jka-assets --offline --lib bathymetry
cargo test -p DinurdoJK --offline --bin DinurdoJK ocean::tests
cargo check -p DinurdoJK --offline
```

The actual-map test is ignored by default because it requires local assets:

```powershell
cargo test -p DinurdoJK --offline --bin DinurdoJK coastal_checkpoint_real_runtime_maps -- --ignored --nocapture
```

Check disk space before building. Previous validation used `C:\Users\David\AppData\Local\Temp\dinurdojk-coastal-build` with `--config profile.dev.debug=0 --config profile.dev.incremental=false`, because D was nearly full. Its temporary target already reused local Steam Audio libraries. Do not delete unrelated build trees or modify registry dependencies to work around the environment.

Checkpoint 2 is complete when it has a faithful, independently tested solver; quantitative correctness results; measured recurring CPU cost/memory; a documented GPU implementation plan or validated prototype; and reproducible diagnostic artifacts. Save source snapshots, a patch against the new Phase 2 baseline, hashes, test/benchmark logs and the porting/provenance notes on disk.

Present that checkpoint and its remaining limitations to the user **before proceeding to Checkpoint 3**. Do not claim the exposed-water-edge problem is fixed by a successful synthetic solver test.

# Coastal water Checkpoint 2: independently validated solver

Completed October 9, 2026. This checkpoint adds a pure CPU coastal solver,
reference-parity and synthetic correctness tests, a reproducible headless
validation/benchmark harness, and a concrete GPU execution plan. It does not
connect the solver to live maps, FFT waves, rendering, networking, or player
physics. The exposed-water-edge problem is therefore not yet fixed.

## Delivered implementation

`jka-water::coastal` is a dependency-free solver module. All solver quantities
use SI units: metres, seconds, metres per second, and `9.81 m/s^2`. A future BSP
adapter must convert JKA world units with the established scale of 40 world
units per metre and should subtract a local water-level origin before storing
bed elevations.

The state is cell water depth plus staggered positive-X and positive-Y face
velocities. Each step performs, in order:

1. hydrostatic free-surface pressure updates at open faces;
2. terrain-crest face depth and donor-cell volume flux construction;
3. one outgoing-volume limiter per donor cell;
4. one shared limited flux per face;
5. conservative cell divergence into a ping-pong depth buffer; and
6. optional, explicitly volume-accounted boundary/sponge targets.

The Saltreach stage order, constants, f64 intermediates, and f32 store points
are retained in the default parity mode. Its local steep-front dissipation is
independently switchable. Non-fluid foam/material advection and the
sub-millimetre drainage sink are intentionally outside the conservative core.
The reference source remains unchanged and matches all hashes recorded by
Checkpoint 1.

Arbitrary geometry is represented by an active-cell mask and independent X/Y
face-wall masks. Unsupported/PVS-filtered/dry mesh cells can be inactive;
inactive cells cannot source pressure or flux. A wall blocks its single shared
face even when both neighbouring cells are deep. A raised bed remains
overtoppable when the free surface physically exceeds its crest. This is the
intended ingestion boundary for the newer PVS/dry-cell mesh filtering, but the
adapter is deliberately deferred with live integration.

`FrameStepper` uses a multidimensional gravity-wave/velocity CFL bound,
`cfl / ((|u| + sqrt(g h))/dx + (|v| + sqrt(g h))/dy)`, a configurable maximum
frame interval, and a hard substep cap. Excess wall-clock time is reported as
dropped rather than causing unbounded catch-up. Surface reconstruction follows
wet neighbours only across open, actually submerged faces; it does not create
water in dry/inactive cells or average through obstacles.

## Saltreach parity and provenance

The port adapts `src/simulation.js`, `src/solver-kernels.ts`, and
`src/surface.js` from the supplied Saltreach snapshot at
`D:\Code\coastal-simulation-main` (MIT, Copyright 2026 Techartist). The notice
is preserved in `crates/jka-water/LICENSE-Saltreach-MIT` and the project-wide
record is in `THIRD_PARTY.md`.

`scripts/generate-coastal-reference.mjs` reproduces the upstream pressure
kernel's typed-array store order on a deterministic 4x3 domain. The Rust test
compares every face velocity, X/Y donor flux, limiter, and next depth exactly
as f32 values. Boundary layout translates directly: element `k` stores the
face from `k` to `k+1` or `k+nx`; the final column/row is a closed zero-flux
face. The Rust extension also closes faces adjacent to inactive cells or an
explicit wall.

Verified local SHA-256 values:

| Saltreach file | SHA-256 |
|---|---|
| `src/simulation.js` | `76eeff17acfd66e46c1db5669e69c6752186da1c745b4a85cc39d4a7f94719e1` |
| `src/solver-kernels.ts` | `28920f3693edcec2dab000b4975e638bb7d4d5f1956b1b376d79702cf28fd04b` |
| `src/surface.js` | `ca29cdb8b7ba7f3af8be0166be6437675c6af86144503ca277813b16e8d6f9be` |
| `src/coast.js` | `eeed29437c71321b9bf2fb398cfcb94fd281e112a8f732b71360ee2362aa0771` |
| `src/terrain-grid.js` | `b4cc2fd3982478442e4da748b43f958c634542a502fd9359112f8d2945325293` |
| `LICENSE` | `b0e0e711c04aac69e316e9c79a3119b722dc7d65e129990cdbe7a35dea6fec8b` |

## Synthetic validation

The release harness is deterministic and writes machine-readable results,
time histories, a volume-error SVG, and depth PGM snapshots under
`target/coastal-checkpoint2/validation`. Run it with:

```powershell
cargo run --release -p jka-water --offline --bin coastal-validate -- --output target/coastal-checkpoint2/validation
```

All cases passed. General closed/source-accounted volume tolerance is 20 ppm;
the observed worst error was 0.103 ppm. This allows f32 storage accumulation
without masking order-percent loss. Rest-state tolerances are near one f32 ULP.
The wave tolerance is `3e-4 m` RMS, 15% of its deliberately small 2 mm linear
amplitude; the measured fine-grid error was `2.792e-6 m` (0.14% of amplitude)
and decreased under refinement.

| Case | Deterministic setup | Quantitative result |
|---|---|---|
| Flat closed basin | 24x16, 0.4 m cells, 2 m depth, 2 s | exact rest and exact reported volume |
| Sloping/stepped bed at rest | 32x18, 0.25 m cells, 0.25 m bed step, common zero-metre surface | zero surface/velocity error |
| Raised dry terrain | 0.35 m sill; zero then 0.75 m water level | no low-level leak; high level overtopped; non-negative |
| Small wave | 2 mm Gaussian, 1 m depth, 0.45 s | RMS `4.731e-6` at 0.2 m and `2.792e-6` at 0.1 m; ratio `0.590` |
| Vertical/thin wall | one face thick, 2.5 s | zero wall flux and zero opposite-basin change; 1.088572 m arriving/reflected crest |
| Island/separated basins | inactive circular island and one-cell separator | zero inactive water and zero cross-basin change |
| Wet front | 1 m-to-dry dam break, 0.1 m cells, 0.7 s | front advanced from cell 29 to 48; minimum depth zero; finite |
| Forcing/sponge | 0.75 Hz, 25 mm boundary forcing, 2 s | source-accounted balance error `7.264e-8`; bounded 0.996-1.004 m depth |
| Frame gap | 2 s request, 0.25 s frame cap, six-step cap | six steps, 0.038950 s simulated, 1.961050 s explicitly dropped; finite |

The exact values are in `validation/summary.json`; `histories.csv` contains
volume/source/depth/velocity time histories. PGM files are portable grayscale
diagnostic images and can be opened directly or converted by common image
tools.

## CPU performance

Measured in the release profile (`lto=true`, one codegen unit) with Rust
1.98.1/LLVM 22.1.8 on Windows x86-64, an AMD64 Family 25 Model 97 processor
with 24 logical processors. The solver is currently single-threaded. Each row
uses a 60 Hz fixed step after eight unmeasured warm-up steps and reports the
median of five timed samples. Initialization, file output, and rendering are
excluded.

| Allocation | Active cells | Median time/step | CPU time/simulated second | State/geometry bytes |
|---|---:|---:|---:|---:|
| 64x64 | 4,096 (100%) | 0.1189 ms | 7.13 ms | 143,360 |
| 128x128 | 8,192 (50%) | 0.2814 ms | 16.88 ms | 573,440 |
| 256x256 | 16,384 (25%) | 0.7218 ms | 43.31 ms | 2,293,760 |
| 256x256 | 58,880 (90%) | 1.7186 ms | 103.12 ms | 2,293,760 |
| 512x512 | 65,536 (25%) | 2.8906 ms | 173.43 ms | 9,175,040 |

The byte estimate includes eight f32 arrays and three byte-sized geometry
masks, excluding small `Vec` headers/allocator metadata. Timings are a local
sample, not an FPS guarantee. The 512x512 result shows that the reference still
scans the allocated rectangle even with only 25% active cells. Production GPU
work should therefore compact active tiles/dispatch lists so PVS and dry-cell
filtering reduces dispatch work as well as arithmetic inside a cell.
Per-sample minima/maxima are retained in `validation/summary.json`; the widest
observed range was 1.6900-1.7384 ms for the 90%-active 256x256 case.

## GPU implementation plan

No GPU solver is claimed in this checkpoint. The proposed WGPU layout is:

- immutable f32 bed and packed active/X-wall/Y-wall flags;
- two f32 depth buffers (`h_read`, `h_write`);
- X/Y velocity and X/Y shared flux buffers;
- one f32 outgoing limiter buffer;
- an optional compact active-tile list and indirect dispatch arguments;
- a small diagnostics reduction buffer copied asynchronously only for tools,
  never as a blocking per-frame CFL readback.

Use four ordered compute passes in one command encoder: (1) pressure and donor
flux, (2) outgoing limiter, (3) shared face-flux limiting, and (4) conservative
divergence/boundary application into `h_write`. Separate passes make storage
dependencies explicit in WGPU. Swap only the two depth bindings after the
encoder is submitted; velocities and fluxes retain face ownership. A fifth
optional pass reconstructs surface elevation/normals into renderer-facing
buffers using the same open-face test.

Runtime scheduling should accumulate wall time into a fixed simulation step,
interpolate presentation between the two most recent surfaces, cap substeps,
and drop excess debt. CFL should use conservative configured bounds initially
or a delayed asynchronous reduction; it must not stall on a GPU readback.
Parity validation should run the 4x3 fixture and all synthetic domains through
CPU and GPU, compare intermediate buffers after each pass, and then compare
long-run volume/positivity before the GPU path can replace the reference.

## Regression verification

- `cargo test -p jka-water --offline --release`: 3 passed, including all
  nine validation cases and exact Saltreach intermediate parity.
- `cargo test -p jka-assets --offline --lib bathymetry`: 11 passed.
- `cargo test -p DinurdoJK --offline --bin DinurdoJK ocean::tests`: 16 passed,
  including the newer PVS/dry promoted-clipmap filtering tests.
- `cargo check -p DinurdoJK --offline`: passed.

The client commands used the handoff's temporary target directory after the
repository `target/debug` fingerprint directory returned Windows access denied.
Warnings originate in pre-existing unrelated work and are retained in the
checkpoint logs.

## Remaining boundaries

- There is no bathymetry-to-solver adapter yet. Phase 2.5 now distinguishes
  impermeable vertical `Wall` faces from overtoppable `Bank` terrain and makes
  that decision inspectable with `r_coastalDebug`; real-map review is still
  required before live use.
- PVS/dry mesh filtering is recognized by the active-mask design. Map ingestion
  now supplies sparse 16x16-cell footprint tiles, while compact GPU dispatch
  scheduling remains unimplemented.
- Solver-state execution across the new stitched tile atlas, unresolved geometry, moving inline models, overhang fallback,
  live FFT boundary forcing, blending/replacement, shoreline mesh generation,
  rendering, and an independent coastal toggle remain later work.
- Saltreach's velocity clamp is preserved for parity but is not treated as the
  stability condition; the separate CFL/substep logic is authoritative.
- Source/sponge volume is accounted explicitly. Non-conservative residual
  drainage is not present, and visual foam/material transport remains a
  separate future layer.
- No user-approved FPS floor or first RaceArena live water body has been
  selected. The timings above must not be interpreted as approval for a live
  performance cost.

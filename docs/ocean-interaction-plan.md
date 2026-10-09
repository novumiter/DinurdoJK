# Ocean interaction implementation plan

Planning checkpoint: October 8, 2026. Read the supplied water-interaction handoff, inspected the current worktree and both local reference snapshots, and subsequently traced the user's jaPRO sailing implementation. This work is planning only; no renderer/gameplay implementation has been changed and no builds or visual tests have been run.

## Objective and first acceptance gate

Add optional, client-side interactive water height and normals to the existing FFT ocean. Preserve movement, collision, network prediction, puddle wakes, and the FFT scheduler.

First working checkpoint: a player crosses an actual rendered water surface, injects a bounded disturbance, and produces propagating deformation plus matching shading normals on that same ocean mesh. Include debug contact markers and deterministic test injection. Continuous wakes, persistent foam, and enhanced particles follow this checkpoint.

## Verified findings

- Source files live under `crates/jka-client/src/`, rather than the shortened paths in the handoff.
- `scene/coordinates.rs` converts JKA `[x,y,z]` to render `[x,z,-y]`. Ocean simulation/render conversion uses 40 world units per metre.
- `bsp.wgsl::sample_ocean_displacement` samples undisplaced XZ in metres and returns displacement in metres. The vertex path converts it to world units. `ocean_uv_m` retains the undisplaced sampling coordinates for shading.
- Ocean shading sums FFT gradients and an analytic swell gradient before building a normal. Add interaction slopes at this location; avoid summing normalized normals.
- `OceanSurface` describes a plane and XZ bounds. `OceanMasks::contains` already matches shader triangle masks, including holes, with a 0.75 world-unit tolerance. `WaterVolume::contains` checks bounds and optional masks.
- World upload clusters contiguous faces, assigns clipmap slots, and builds promoted masks. Authored oceans have separate `OceanGpu` instances and descriptor bounds. Promoted surfaces share an FFT instance with individual render uniforms.
- `frame/draw.rs` filters authored surfaces out of the shared ocean's surface list. These filtered indices must not be mistaken for world mask or clipmap slots.
- Puddle collection uses model lighting origins, a fixed 24-unit sole offset, and proximity-based local-player deduplication. It runs under weather/puddle conditions. Reuse useful tracking concepts, but use a separate ocean collector with explicit actor identity and contact evidence.
- Native movement output exposes `water_level` and `water_type`. Existing footsteps have a collision-world water trace. These are possible contact inputs; they are not currently carried in the renderer's player-position field.
- Presentation samples already include local client identity, velocity, presentation time and teleport status. Remote actors have entity identity and teleport flags. These are better sources than scanning every rendered model surface.
- Splash requests use `UserEvent::WaterSplashes`; `graphics.rs` converts back to JKA coordinates and plays `env/water_splash` or `env/water_impact`. The current request has position and landing status, without intensity.
- `frame/plan.rs` makes `FastBaseline` eligible only when oceans are off. Interaction rendering belongs in the existing ocean-capable path. FastBaseline acceptance is an OFF-path regression check.
- FFT computation is already distributed across frames. Hidden-water simulation gating, GPU timing infrastructure, shader validation, and `bench/plans/ocean.bench` already exist.
- The worktree has extensive existing edits, including affected files. Implementation patch generation must compare against a saved pre-implementation baseline, not repository HEAD.

## Reference port scope

Project-specific wiring reference: `D:\Code\jaPRO`, local branch `sailing`, revision `20172b38dcde9469d055ff3e2349018712636f9e`. The checked-out branch is `main`, so the sailing source was read with `git show`/`git grep` without switching branches. See [the jaPRO source audit](ocean-interaction-japro-audit.md) for exact entry points, existing refinements and port caveats. Use this implementation as the first reference for entity/contact collection, bounded input descriptors, projectile/explosion event lifetime and actual ocean integration. Keep the WebGPU WGSL solver as the baseline wave-kernel source and compare jaPRO's existing nine-point refinement for visual isotropy.

Primary solver source: `D:\Code\GodotOceanWaves\webgpu-water-main`.

Its actual kernels are fragment shaders, not compute entry points:

- `src/shaders/water/update.frag.wgsl`: four-neighbour height average; velocity increment `(average-height)*2`; damping `0.995`; height integration.
- `normal.frag.wgsl`: forward height differences, tangent cross product, normal X/Z stored in B/A.
- `drop.frag.wgsl`: smooth cosine disturbance falloff.
- `sphere.frag.wgsl`: restores displaced volume at the old sphere position and subtracts it at the new position. This gives moving sources a useful positive/negative disturbance and yields zero for unchanged positions.

`water.ts` uses RGBA height/velocity/normal ping-pong textures and fullscreen render passes. `main.ts` steps twice per animation frame. Port the stencil and input shapes to WGPU compute, retaining source attribution. Replace frame-dependent stepping with a fixed timestep, explicitly documenting coefficient/unit conversion and stability. Precreate pipelines, texture views and bind groups; encode into the existing renderer submission rather than copying the demo's per-pass resource creation/submissions.

Architectural source: `D:\Code\GodotOceanWaves\crest-master`.

- `UpdateDynWaves.compute`: previous/current world transforms, four-neighbour wave propagation, Courant speed clamp, edge damping and invalid-state protection.
- `LodDataMgrPersistent.cs`: persistent ping-pong domains, source transforms for scrolling, and separation of coordinate reprojection from time advancement.
- `LodDataMgrDynWaves.cs`: fixed simulation frequency and accumulated time.
- `SphereWaterInteraction.cs`: motion/contact weighting, speed clamps, teleport rejection and substep input positions.
- `UpdateFoam.compute`: persistent foam, world-space transport and time-based decay.

Use these patterns without bringing over Unity gameplay, buoyancy, its complete LOD system, or readback. The Unity WebGPU project remains a secondary effects reference; its own README identifies the large-ocean path as newer and wake/spray shaping as ongoing work.

Both supplied local reference snapshots carry MIT licenses. Preserve the full license notices for copied/adapted code and add exact provenance to `THIRD_PARTY.md`. The WebGPU README credits Evan Wallace's original WebGL Water; retain applicable original attribution when tracing copied algorithms. These directories appear to be extracted snapshots; record source hashes instead of inventing a commit ID.

## Proposed implementation

### 1. Contact sources and stable water-body identity

Add a compact presentation source record once per unique player, independent of model surface count and raster visibility. Carry entity/client ID, origin, velocity where available, hull/contact samples, presentation timestamp, continuity token and known water-contact information. Use local client identity to exclude the local third-person model, preserving nearby remote players.

Feed this through the existing retained scene/frame mailbox. Do not enqueue one event per render frame: the renderer may repeat a scene or skip intermediate scenes. Track consumed timestamps and motion segments so a sample is processed once and crossing intervals survive ordinary frame skipping.

Follow jaPRO's distinction between continuous player/vehicle descriptors and retained one-shot inputs. Its player collector uses explicit predicted local identity, interpolated remote origins, water-contents checks, feet/head intersection, a smooth wading/swimming blend and fixed surface coupling. Adapt these rules to existing Dinurdo water-body masks and its visual-only FFT waterline; do not import sailing pmove or network changes. Retain world motion separately from water-relative forcing velocity for correct substep positions.

Create a water-body registry from the surfaces actually rendered by the active ocean pipeline. Keep world clipmap/mask slots, authored descriptor IDs, and interaction texture layers distinct. Use stable IDs plus a map generation; layer reuse clears old state. Coplanar disconnected pools remain separate bodies. Authored selection follows existing draw selection/bounds rules.

Contact requires both body footprint membership and vertical player-hull intersection with the surface. Use existing water contents/level when available; use bounded collision queries at presentation cadence for missing information. An authored visual ocean without a matching gameplay water brush needs a deliberate geometric fallback. Document that distinction rather than changing gameplay physics. A nominal water plane initially approximates the FFT waterline; no GPU height readback is added.

Track entry, exit, landing and movement separately. Use swept crossings for impacts and old/new positions for movement. Reset on map changes, respawn, teleport flags, time rewind, stale history and implausible discontinuities. Use hysteresis and travel/contact thresholds to suppress idle jitter. Bound source/event counts, radius, strength and segment subdivision; preserve impacts when coalescing movement.

Verify contact markers against holes, stacked planes, third person and actual contents before enabling solver injection.

### 2. Persistent local GPU simulation

Use a renderer-owned interaction resource shared by ocean render bindings, with independent layers for active bodies. Avoid duplicating this field for every FFT/material instance. Start with a small bounded pool; provisionally four active body domains, selected by nearby interaction and retained while their waves decay. Overflow is diagnosed and omitted rather than sharing a body layer. Maximum active count is a measured implementation choice.

Initial hypothesis: 256x256 cells, 4 world units (0.1 m) per cell, covering 1024 units (25.6 m) per domain. This resolves player-scale disturbances while leaving existing 10/40-unit ocean meshes unchanged. Resolution/coverage is independent of FFT texture settings.

Snap domain origins to whole cells, with a margin/hysteresis before recentering. Convert each destination cell to world XZ, then to the previous domain's source cell. Preserve overlap exactly; clear newly exposed cells and fully clear on nonoverlapping shifts. Keep the published texture's origin with that texture until replacement is complete, including frames where the camera moves but no simulation tick runs. Camera rotation never changes the grid orientation. Do not use wrapping or `fract` for local sampling.

Start at 60 Hz and compare 30 Hz only after establishing the timestep/cell-spacing/wave-speed relationship. Use a bounded catch-up budget and discard excessive backlog with diagnostics; never integrate one oversized timestep or inject an event again during catch-up. Use elapsed time for decay. Hidden/inactive domains avoid dispatches; long suspensions age out state rather than producing a burst on return.

Suggested storage: two RG32Float height/velocity arrays sampled through `textureLoad`, plus a filterable RGBA16Float published height/slope field. This is approximately 1.5 MiB per body, or 6 MiB for four bodies, before masks/buffers. Verify adapter support before finalizing formats. Normals/slopes are generated from the final updated height, with physical cell spacing.

Cache a cell coverage mask per body/domain instead of scanning all outline triangles every tick. Mask both injection and propagation to prevent waves travelling through dry holes or between disconnected portions. Rebuild coverage on map/domain changes. Use Crest-style damping at artificial domain edges, and explicitly select a conservative shoreline boundary treatment for the first increment.

Batch bounded disturbance inputs by body and spatial region. The demo's whole-grid pass per drop is unsuitable for many players. Use a batched injection pass with bounded relevant input lists/regions, then propagation and slope publication. Preallocate resources and input/substep buffer capacity. Multiple dispatches in one submission must receive distinct parameter snapshots; repeated writes to one uniform range cannot stand in for per-dispatch values.

### 3. Integrate actual ocean height and shading

Extend the existing ocean bind group with the shared published texture and domain/body metadata. Binding 5 already contains windrow storage; do not reuse it. Keep authored, promoted and inert bind groups structurally compatible. Compile variants on the cold path.

At the existing ocean-only vertex branch, sample interaction height using undisplaced XZ and explicit body/layer selection. Add it to FFT/swell displacement with correct metre/world conversion and adjust wave-height outputs consistently. Retain current material, appearance and mask behavior.

In ocean shading, use the same coordinate/body selection and add matching slopes to the existing gradient. Apply interaction strength consistently to height and slopes, with screen-space fading for unresolved detail. Include boundary fade in the derivative calculation. Low mesh quality still shows fragment normal detail, while geometric ripple detail remains limited by mesh spacing.

Audit displaced bounds/culling, waterline/optics consumers and render passes using wave height. First increment retains nominal-plane CPU contact and existing underwater classification; assess waterline artifacts visually. Reflections/refraction/fog retain the existing integration rather than gaining a second water plane.

Interaction OFF selects an inert field, stops collection/injection/simulation and clears pending state. Reenabling starts cleanly. Do not alter FFT textures, appearance controls or scheduling.

### 4. Directional wakes, foam and splashes

After the first vertical slice passes, inject continuous old/new movement segments so a fast swimmer produces a coherent wake rather than separated circular drops. Use Crest's directional interaction patterns and the reference sphere displacement as sources; a V-shaped wake is a visual acceptance target to tune, not guaranteed by a circular impact kernel.

jaPRO already implements elliptical paired inside/outside forces for players and hulls, a retained 100 ms projectile-entry source, and zero-area radial explosion forcing. Keep input records extensible for these sources. Hook future projectile crossings into the existing missile presentation path and explosions into already-deduplicated weapon/event presentation, rather than adding full-frame scans or detecting explosions from rendered particles. Vehicle integration follows once its actual presentation metadata is available; do not copy jaPRO's hull-exclusion dependency into a player-only first increment.

Add a separately persistent foam field with speed/impact sources, bounded accumulation and timestep-based decay. Blend with existing ocean foam and windrows. Preserve body isolation and scrolling rules.

Reuse the existing JKA splash bridge for entries and large impacts. Extend the request only if intensity can actually be expressed by the FX pipeline. Audit existing gameplay/footstep water effects to suppress duplicate presentation. Consider enhanced ocean spray only after validating the existing result.

## Checkpoints and validation

1. Contact debug checkpoint: stationary actor, moving actor, swept entry/exit, strong landing, two actors including nearby local/remote, holes/outside bounds, stacked bodies, teleports/respawn/map/time resets. These are focused pure tracking/contact tests.
2. First visible checkpoint: GPU impulse propagates and dissipates, then real player injection changes ocean geometry and normals. Verify stationary actors inject nothing repeatedly. Save an implementation progress file with changed files, actual checks and exact next steps.
3. Stability/domain checkpoint: deterministic test impulse retains world location after integer-cell shifts, newly exposed cells are zero, bodies cannot contaminate each other, low-FPS backlog is bounded, 60/144/1000+ render FPS share simulation-time behavior. Diagnostic GPU readback is allowed only in a standalone test/probe, never the frame loop.
4. Presentation checkpoint: continuous slow/fast wakes, localized fading foam and one scaled splash per appropriate contact event.
5. Performance checkpoint: compare ocean ON/interaction OFF against idle interaction ON, one actor, many actors, moving domain and hidden water. Measure CPU collection/encoding, GPU injection/solver/publication, memory, event counts and frametime tails. Keep mesh/FFT/optics settings identical between comparisons. Use the existing ocean benchmark as a starting point.

Build checks during implementation: `cargo check -p DinurdoJK --offline`; focused unit tests; `all_renderer_shaders_validate` with new compute kernels and composed BSP variants; headless GPU solver checks if an adapter is available. Establish relevant baseline failures first. Run the full client suite at the final implementation checkpoint where practical. Do not label unperformed in-game checks as passing.

Visual acceptance covers promoted and authored oceans, both mesh qualities, water boundaries, camera translation/rotation, SSR/refraction/underwater transitions, puddle wakes/FX and interaction OFF. Test FastBaseline with oceans OFF and test ocean ON through the existing general path; no plan to expand FastBaseline's feature envelope.

For every implemented milestone, save its progress checkpoint and patch against the saved starting worktree. Final implementation deliverables include the unified patch, ZIP, source/license notes, changed-file list, actual build/validation results, manual test steps and remaining limitations. No implementation patch or ZIP is claimed for this planning checkpoint.

## Remaining choices

- First visual acceptance map and priority: shallow running/impact versus surface swimming. If unspecified, start with shallow movement and entry impacts, then validate swimming. `RaceArena_Pro` is already a useful mask/performance stress case, not necessarily the easiest first visual test.
- Confirm the narrowest collision/contact adapter and source sampling cadence during implementation. Remote water contact is not an existing renderer field.
- Measure domain count, coverage, wave speed, decay and scheduling costs before exposing tuning controls. Initially expose only `r_oceanInteraction`, `r_oceanInteractionStrength`, and optional `r_oceanInteractionDebug` through the existing cvar/UI architecture, defaulting the feature OFF during development.

## Pinned local solver hashes (SHA-256)

| File in `src/shaders/water/` | Hash |
|---|---|
| `update.frag.wgsl` | `F2EC8350B1BE7DE64C4E1B0C7903B837AA605135F3BA36BEE31398E584B7B64C` |
| `normal.frag.wgsl` | `12DF94CD55A08C63EA2B6664BCA55659D69F27D6F81E989862E59D87A4DF691B` |
| `drop.frag.wgsl` | `C101A3A84075C04C1455501C9411E976DECAC2BEA6718FF1CF54459762A17DF0` |
| `sphere.frag.wgsl` | `56DF3D9C7F896C506BE377D1607FA21AFE49767D430C1EC5B8A6C1DF9E5CDFEC` |

Online architectural/effects cross-checks: [Crest Dynamic Waves](https://docs.crest.waveharmonic.com/Manual/Simulation/Ripples.html) and [Unity WebGPU Water README](https://github.com/AbstractOcclusion-gif/unity6-webgpu-interactive-water). The actual initial solver and Crest code study above used the supplied local snapshots.

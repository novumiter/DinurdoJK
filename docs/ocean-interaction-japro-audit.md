# jaPRO ocean interaction source audit

Inspected October 8, 2026, for the DinurdoJK water-interaction plan. Source inspection only: no builds or visual tests were performed and no jaPRO source or branch state was changed.

## Source identity

Repository: `D:\Code\jaPRO`. Current checkout: `main` at `dcda9d6556abe0e33e63a24c573d4dcfb433e472`. Wake deformation source is present on local branch `sailing`, pinned here at `20172b38dcde9469d055ff3e2349018712636f9e` (August 10, 2026, `checkpoint commit before river wake deformation`). Read this source with `git show <revision>:<path>`; the sailing files are absent from the current main checkout.

`docs/sailing_dynamic_waves_handoff.md` on that branch describes the boat, player, projectile and explosion progression. Source code is newer than some details in the handoff: the inspected dynamic texture is RGBA16F, contains transported vortex foam, and the solver already uses a nine-point stencil. Do not implement from the handoff's older RG16F/four-neighbour description alone.

## Actual wiring

| Input | Collection and validation | Renderer input |
|---|---|---|
| Boats | `codemp/cgame/cg_sailing.c:3144`, `CG_SailingBuildBoatExclusions`; hull descriptors, predicted ridden velocity/remote trajectory velocity, analytic wave/current subtraction | Hull origin, forward/up, half extents, waterline, relative velocity and amplitude |
| Players | `cg_sailing.c:3339`, `CG_SailingBuildPlayerWakes`; predicted local player and interpolated remote players; reject dead/mounted/spectator/invalid actors; contents and feet/head contact checks | Independent player descriptors, not dry hull exclusions |
| Projectiles | `cg_sailing.c:3693`, `CG_SailingTrackProjectile`; called inside `cg_ents.c` missile presentation; entity-generation tracking and first dry-to-wet transition | Up to eight retained entry descriptors |
| Explosions | `cg_sailing.c:3935`, `CG_SailingExplosionEvent`; called from missile hit/miss, concussion-alt and selected `EV_PLAY_EFFECT` events in `cg_event.c` | Up to eight retained radial descriptors |

`codemp/rd-common/tr_types.h` defines the render records: 16 hulls, 16 players, eight projectile ripples and eight explosion ripples. `cg_sailing.c` filters descriptors by the owning ocean while filling `sailingOceanRender_t`. `shared/rd-rend2/tr_surface.cpp:4703` calls `R_UpdateSailingOceanDynamicWaves`. The solver and sampling metadata reside in `shared/rd-rend2/tr_sailing_fft.cpp`; final geometry/normal/foam integration is in `shared/rd-rend2/glsl/sailingOcean.glsl`.

### Player rules worth preserving

- The local actor uses predicted origin and velocity without requiring snapshot membership; the source explicitly documents a previous local acceptance bug caused by that requirement.
- Remote actors use interpolated origins and network trajectory velocity. Explicit client identity avoids first/third-person double counting.
- Ocean XY selection is followed by a real water-contents sample eight units below the calm plane. Feet receive a second contents check, while the moving analytic surface handles crests/troughs.
- Foot position is `origin.z + MINS_Z + 1`; the top sample uses actual local view height or the remote default. A player above the surface or with the head sample submerged is rejected.
- A middle-body waterline transition smoothly scales sustained swimming to 75% of wading strength. Entry remains stronger.
- Relative velocity subtracts analytic wave/current motion. Thresholds reject very slow inputs. An elliptical 24-by-15-unit half-extent represents the player.
- GPU player coupling is a constant 0.65, rather than deeper hull immersion. A separate 10x player calibration exists. These are Quake-unit tuning values, not constants to copy into a finer metre-based Dinurdo field.
- Local-first/nearest prioritization bounds the accepted descriptors. The debug path explains the exact rejected contact test once per second.

### Weapon and explosion rules worth preserving

- Missile tracking rides the existing presentation loop; it adds no broad entity scan. Stationary missiles, saber/grapple-like entities and unsupported weapons are excluded.
- Tracking distinguishes reused entity slots by start time, weapon and owner, with time/gap checks. A trajectory lookback supports newly observed missiles.
- A first dry-to-wet crossing is refined with seven binary-search iterations against water validity and the moving analytic surface. Only one entry ripple is emitted for that tracked projectile.
- A projectile source lives 100 ms so a fixed 60 Hz tick can see a crossing that existed for only one presentation frame.
- Explosion presets cover rockets, thermals, repeater alt, concussion and selected effect IDs including detpack, tripmine, flechette and DEMP2 alt. Each defines radius, strength and vertical reach.
- Explosion XY must lie over valid BSP water. Distance to the moving surface attenuates strength smoothly; above/below surface controls impulse sign.
- A small recent-event cache matches effect/weapon class, nearby position and time to suppress duplicate explosion inputs.
- GPU explosion injection uses `(1 - r²/2) * exp(-r²/2)`: a central force and opposing annulus with zero continuous-plane area integral. This avoids the accumulating volume of a positive-only height drop. Discrete-grid/truncated-domain balance still needs verification.

## Solver and rendering

`sailingDynamicWaveSource` starts at `tr_sailing_fft.cpp:360`. It is already a Crest-derived GPU implementation, separate from the FFT:

- 256x256 RGBA16F ping-pong state: height, vertical velocity, persistent vortex foam and spare channel.
- Normal span 8192 world units; 32-unit cells. Visible vortex coverage can enlarge/reanchor that span.
- 60 Hz fixed stepping, maximum six catch-up steps, elapsed time capped at 100 ms, and bounded remainder.
- Source/target world bounds preserve overlap under camera movement. The published metadata remains associated with the field actually computed when no step occurs.
- Wave speed from wavelength/gravity with Courant clamp 0.70, damping 0.65/s and Quake-unit gravity 400.
- Nine-point Laplacian: `(4 * axial_sum + diagonal_sum - 20 * center) / 6`. The source explicitly identifies cross/star artifacts from the prior cardinal-only stencil as its motivation.
- Crest-like elliptical signed-distance forcing, with paired inside/outside horizontal forces and vertical motion forcing. Players use surface coupling; hulls use immersion weighting.
- World-space flow backtraces height, velocity and foam through authored vortices. This is unrelated to the initial Dinurdo interaction scope.
- Edge damping, invalid-state clearing, and clamps to height +/-48 units, velocity +/-800 units and foam 0..2.

In `sailingOcean.glsl`, `SailingDynamicWaveSample` derives smoothed gradients from neighbouring heights in both vertex and fragment stages. At approximately line 1042 it adds dynamic height and bounded gradient-derived horizontal displacement to the existing ocean deformation. At line 2835 it adds the dynamic slope to the analytic geometric slope. At line 3474 slope/velocity generate a wake foam mask blended with existing foam; a hull-ahead suppression reduces white leading waves.

The stored foam channel is injected by vortex shear/FFT-whitecap sources. Ordinary player/boat wake foam is still derived from instantaneous slope/velocity, so its persistence can differ from the vortex history.

## What this changes in the Dinurdo plan

1. Use jaPRO as the primary wiring reference for entity/contact/event adapters. It already answers how local prediction, remote presentation and weapon events reach a bounded GPU input set. Keep the WebGPU WGSL solver as the initial wave-kernel reference and compare its baseline to jaPRO's existing nine-point refinement.
2. Separate continuously sampled actors from one-shot disturbances. Carry event IDs/times through Dinurdo's retained mailbox so repeated render frames cannot re-trigger events and skipped frames cannot lose them. Retained pulses must have defined total impulse when ticks are delayed; do not copy a render-lifetime mechanism without checking integration over time.
3. Improve player resolution first. A player is smaller than one jaPRO 32-unit cell, while the proposed initial Dinurdo cell is four units. A 25.6 m window trades distant coverage for local detail; test whether it feels sufficient before adopting it.
4. Preserve stable world-space sampling and add actual body isolation. jaPRO has one global dynamic field; its handoff acknowledges multiple oceans/mirrors/portals as a limitation. Dinurdo must bind the correct body layer and keep auxiliary views from advancing/recentering simulation independently.
5. Apply dry-cell masks inside propagation. jaPRO validates the source against BSP water but its shown solver does not mask dry cells. A render mask alone cannot prevent propagation through land and reappearance elsewhere.
6. Precompute height gradients at simulation cadence. jaPRO samples neighbouring heights separately per vertex and fragment; a published height/slope texture can reduce repeated render-time texture fetches at Dinurdo's target FPS. Keep it consistent with the final height.
7. Keep world transport velocity separate from relative-water force velocity. jaPRO reconstructs earlier source positions by subtracting relative velocity. With appreciable current/wave motion that differs from the actor's actual world path; Dinurdo should interpolate world positions for placement and use relative velocity only for forcing.
8. Give wake foam its own temporal history once geometry works. Avoid using higher wave strength to repair a short or flickering foam trail.
9. Reevaluate calibrated multipliers, clamps and shoreline behavior with the finer domain; preserve physical units and use diagnostics before raising strength.
10. Preserve the original task's stationary-player rule. Analytic water-relative motion can produce forcing for a world-stationary actor in jaPRO; explicitly distinguish real entry/exit/contact changes from background motion so Dinurdo does not continuously inject artificial idle wakes.

Dinurdo hook candidates already exist: `cgame/weapon_fx.rs::missile`, `entity_event_prepared` and `prepare_entity_event`, plus the existing player/entity presentation and scene mailbox. Adapt at these points without changing movement/network authority or deriving events from particle geometry. Vehicles/projectiles/explosions can share the core solver later; the first implementation checkpoint remains player contact through actual ocean displacement and normals.

jaPRO carries GPL-2.0 source licensing. Record its exact revision and retain applicable file notices for any code ported from it. Its upstream Crest references remain MIT-attributed; do not infer that all jaPRO adaptations inherit an MIT-only license.

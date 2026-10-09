# Player ocean deformation: first increment

Build the client through the existing build workflow, or `cargo build -p DinurdoJK --release --offline`. Load a map with simulated ocean enabled and apply video settings if that map was prepared without ocean resources.

In the console:

```
r_oceanInteraction 1
r_oceanInteractionStrength 1
r_oceanInteractionFx 3
r_oceanUpdates 48
```

The Water menu provides Player water deformation, Deformation strength and an Interaction FX A/B selector. The interaction feature defaults OFF. `r_oceanInteractionFx` selects 0 Off, 1 A: Entry droplets, 2 B: Wake spray, 3 A + B, 4 Entry jets, or 5 All FX. Droplets and jets are generated on downward entries; wake spray follows sustained movement. Count and size rise with speed and `r_oceanInteractionStrength`. There is no separate wake-frequency cvar: it inherits `r_oceanUpdates`. A value of zero retains the ocean's unthrottled meaning, with at most one wake phase encoded per rendered frame. At high FPS coverage, propagation and publication are spread across the inherited interval. At low FPS a bounded three-phase budget can complete a normal single-body tick together; overloaded/multiple-body cases may update more slowly rather than flushing a backlog.

1. Walk/run through shallow promoted water and swim at the nominal waterline. Movement should add local deformation and normal changes to the existing ocean.
2. Stop. New forcing should stop; existing waves decay, fade during their final second, and the field retires after 2.5 seconds without movement.
3. Compare walking, a small jump, a high fall and a fast diagonal entry into the same pool. Entries capture full incoming native/presentation velocity before contact slows it. The normal component creates a depression and expanding ring; tangential components create a directional dipole. Total speed increases impact energy and footprint, with radius normalization. Retained one-shot impulses survive between ocean ticks.
4. Repeat the same high fall with `r_oceanInteractionFx 1`, `4`, and `5` to compare entry droplets and jets. Then move steadily through the pool with mode `2`, followed by `3` to compare wake spray and entry droplets together. Stronger deformation and faster movement should produce broader, taller and denser effects. Mode 0 disables these interaction FX; the existing wind/crest spray setting remains independent.
5. Walk on dry land beside water, through a masked hole, and above/below a stacked water plane. These must not produce surface forcing on an unrelated body.
6. Use third person and a nearby remote player. Each entity contributes once regardless of model surface count. Mounted players, dead players and known spectator entities are excluded.
7. Translate/rotate the camera while observing an existing ripple. Overlapping cells keep their world position. Newly exposed cells start clear; the edge of the approximately 25.6 m window fades out. Ripples outside the window are intentionally discarded.
8. Compare both mesh qualities. Low quality has limited geometric ripple detail, but its fragment shading still samples interaction slopes.
9. Compare `r_oceanUpdates 24` and `48`, and several render FPS caps. The selected rate owns wake scheduling. Very low rates reduce effective wave speed through the stability clamp.
10. Toggle `r_oceanInteraction 0`. Interactive deformation and its player-entry bursts disappear immediately; ordinary FFT water remains. Turn it back on: no stale wake should return.
11. Check authored oceans, reflections/refraction and underwater transitions. CPU contact uses the nominal water plane, not a GPU FFT-height query; crest/trough waterline mismatch remains an explicit limitation.
12. Verify existing rain-puddle footsteps/ripples/splashes, plus the ocean-OFF FastBaseline path.

Strength is now uncapped (nonnegative finite values) in the console, saved configuration and menu. The original +/-8 world-unit height and +/-400 velocity clipping have also been removed. Non-finite-state rejection and the published half-float representable range remain.

Falloff controls are archived and take effect on subsequent wake ticks:

| Cvar | Default | Meaning |
|---|---|---|
| `r_oceanInteractionLifetime` | `2.5` | Seconds since the last moving input before the entire body field retires; range 0.1..60. |
| `r_oceanInteractionFade` | `1` | Final smooth-fade duration in seconds, bounded by lifetime. Zero gives retirement without a time fade. |
| `r_oceanInteractionDamping` | `2` | Continuous velocity decay per second; height decay uses 0.175 times this rate. Zero disables this bulk damping. |
| `r_oceanInteractionEdgeFade` | `81.92` | Width in world units of the window-edge fade; an absorbing sponge spans 1.5 times this width. Range 0..256; zero disables both. |

With defaults, the idle fade begins at 1.5 seconds and ends at 2.5. Velocity retains `exp(-2*dt)` per step (half-life about 0.35 seconds); height uses `exp(-0.35*dt)` (half-life about 1.98 seconds). The 1024-unit square window fades over its outer 81.92 units (2.05 m), with a 122.88-unit sponge. Dry cells stay masked and waves leaving the window are discarded. A new input resets the entire body's idle timer; individual waves still experience continuous damping.

For a stronger, longer-lived first comparison:

```
r_oceanInteractionStrength 30
r_oceanInteractionLifetime 4
r_oceanInteractionFade 1.5
r_oceanInteractionDamping 0.7
```

For diagnostics:

```
r_oceanInteractionDebug 1
```

An active interaction region displays signed height (red/blue) and slope (green). The application log prints input count, active domains, published updates, inherited Hz, captured impacts and injected impulses once per second. No active region means the normal ocean remains visible. Turn debug OFF to evaluate the normal shading. Visual calibration is still pending.

Use the existing `r_gpuTimings`/performance trace workflow. Wake work is included in `ocean_sim`; compare ocean ON/interaction OFF against idle ON, one moving player, multiple players/bodies and camera recentering, keeping all FFT, mesh and optics settings identical. The headless GPU tests verify correctness only and do not establish an in-game performance budget.

Implementation scope: four independent nearby body layers, 256x256 at four world units per cell, bounded to 16 inputs per body. Additional active bodies are omitted until a slot expires. Shared published textures remain allocated when OFF; state/coverage buffers and compute pipelines are released. Splash/spray inputs are player-only; weapon, vehicle and explosion sources plus persistent foam remain future increments.

Patch packaging compares against the saved working files from the beginning of implementation, preserving prior unrelated dirty edits. Verify/apply with `git apply --check ocean-interaction.patch` and `git apply ocean-interaction.patch` in a matching starting worktree; do not apply it again over this already-edited workspace.

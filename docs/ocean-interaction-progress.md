# Ocean interaction progress

October 8, 2026. First player-to-GPU-to-ocean implementation checkpoint completed; user confirmed visible deformation; appearance tuning and profiling remain pending.

Splash follow-up: retained downward entry events and rate-limited moving-player samples now feed a bounded GPU billboard burst. The Water menu and archived `r_oceanInteractionFx` cvar compare A: entry droplets with B: spray along the sustained wake; combined mode, vertical jets and all-effects modes are also available. Burst density and size scale with player speed and interaction strength. This reuses the Godot ocean spray pipeline and follows the Unity reference's separation of entry droplets and jets. Implementation is in the working tree; no build or in-game visual check has been run for this follow-up.

Tuning follow-up: user confirmed seeing deformation in-game, but found it too weak. Removed the strength ceiling from runtime/config/UI sanitization and removed the small height/velocity state clamps. Added archived lifetime, fade-duration, damping and edge-fade controls; original falloff defaults are preserved. Numerical/half-float format protections remain. The updated GPU test checks that strong inputs exceed the former eight-unit ceiling and published values stay finite. Appearance at these new settings and FPS impact are still unmeasured.

Tuning validation completed: normal cargo check passed, both focused config/shader tests passed, and the extended GPU test passed on the RTX 3060. The broad suite was not rerun for this tuning follow-up; its previous 16 failures remain documented below. Updated patch/ZIP include the tuning changes and validation logs.

Impact follow-up: user prefers approximately strength 30 for movement and reported that high falls were no stronger than walking. The old continuous vertical-force sample could miss entry between solver ticks or observe already-stopped velocity. Player records now carry full native/presentation velocity. Each valid downward feet crossing is located along the motion segment, recorded before solver scheduling, and retained for one injection. Normal velocity generates a radial depression/opposing annulus; tangential velocity generates a directional dipole. Total velocity controls footprint and energy; amplitude is normalized by footprint radius so energy scales with speed squared for a fixed player mass. This is a calibrated visual approximation, still using the nominal water plane. Continuous downward heave no longer repeats entry energy. Radius and event count are bounded; there is no extra compute pass or independent tick rate. Teleport, map/time resets and dry-mask rejection remain in place.

Impact validation completed: nine CPU tests passed, including pre-contact velocity after motion stops, high/low impact energy scaling, equal-speed diagonal versus vertical entries, and rejected teleports/dry contacts. Focused WGSL validation and normal cargo check passed. The GPU test passed on RTX 3060/Vulkan: a tangential impact raises the forward side and depresses the trailing side; a brief crossing followed by a stopped pose survives until an ocean tick, produces real GPU displacement, and is injected exactly once across repeated renders. Existing scroll/isolation/rate/retirement checks also passed. The new in-game appearance remains to be compared at the user's strength 30.

Implemented: optional player presentation records, explicit local/remote identity, plane/hull/footprint contact, bounded motion inputs, four independent 256x256 nearby domains with dry-cell coverage, persistent height/velocity state, separately published height/slopes, integer-cell reprojection, ocean-rate scheduled phases and actual BSP ocean displacement/normal integration. Cvars and water-menu controls are wired. FFT and rain-puddle paths are unchanged.

Scheduling follows the user's correction: `r_oceanUpdates` owns the interaction rate. The existing `ocean_phases_due` calculation spreads coverage, solve and publication over that interval. There is no independent 60 Hz wake setting. Low rates use a Courant speed limit; work is bounded to one phase at high render FPS and up to three phases at low FPS, without an unbounded catch-up loop. State fades and retires after 2.5 seconds without motion. Geometry and slopes publish together.

Validation:

- Baseline and final `cargo check -p DinurdoJK --offline` passed. Final build has warnings (49), including existing warnings and the legacy/test-only `OceanGpu::new` wrapper.
- All six interaction CPU tests passed: stationary/discontinuous motion, presentation-time speed, hull contact, dry holes, fast vertical crossing and stacked planes.
- Focused composed BSP + interaction WGSL validation passed.
- Dedicated cvar load/archive/nonfinite-strength test passed. The feature and strength archive; debug is session-only.
- Headless GPU test passed on NVIDIA GeForce RTX 3060, Vulkan: real nonzero height/slopes, dry-cell rejection, exact world-coordinate preservation after a 64-unit scroll, two independent overlapping body layers, inherited 24/48 Hz at 200/1000 render FPS, and retirement after stopping. Synchronous readback exists only in this test.
- Existing promoted-water GPU views/spray test passed with the expanded render bindings.
- Prior full client suite, before the tuning controls: 779 passed, 16 failed, 40 ignored. The failures include fast shader fixtures missing `experimental_star_nest_sky`, frozen cvar archive/fingerprint/count expectations and an existing duplicate Steam Audio archive position, invulnerability/FX assertions, and material/scene amplitude expectations. These broad-suite failures remain unresolved; no claim that the full suite is green. The two new archive positions are unique, and the new defaults are projected out of the frozen compatibility snapshot.
- All seven existing puddle-wake tests passed in the full run.
- No in-game visual test or representative FPS/frametime measurement has been performed.

Artifacts are under `target/ocean-interaction`: unified patch against saved starting working files, ZIP, changed-file list and a before/after SHA-256 manifest. Packaging checks patch applicability against that saved baseline. Prior unrelated dirty edits are excluded. The ZIP also includes manual test steps and validation logs.

Next: build/run the release client and perform the checks in `ocean-interaction-manual.md`; measure the incremental ocean-sim cost with FFT/mesh/optics settings held constant; calibrate source radius/strength and decay from actual appearance. Only then extend directional shapes/foam and weapon/vehicle/explosion adapters.

Current limits: deformation and splash/spray inputs are player-only; contact approximates the nominal water plane rather than reading FFT heights. Persistent foam and weapon/vehicle/explosion adapters remain future work. Low mesh quality limits geometric detail, although fragment normals still include the local field.

# Local RT denoiser receiver fix — October 8, 2026

`r_rtDenoiser` is intended to filter finite runtime lights, including saber
line/capsule emitters and finite promoted FX sources, as well as RT sunlight.
The unchanged baseline lighting/shadow path remains selected when it is Off.

The receiver was rejecting finite emitters because it examined an already sampled
`PointLight`. The emitter sampler deliberately clears the line/source-size fields
after resolving a point/aperture sample. The denoiser now classifies the original
light in `dynamic_lights[light_index]`, retaining safe fallback checks for source ID,
depth, normal agreement, reflections, disabled filtering and negligible signal.

Other corrections:

- Local-only RT publishes the denoiser/debug mode even when RT sun is disabled.
- Glass-enabled receivers consume independently filtered per-channel throughput
  instead of bypassing the denoiser. The trace pass accumulates RGB transmission
  with the same existing segment bounds and receiver bias.
- Temporal local sampling advances without requiring post-TAA, with the trace
  and material receiver sharing the same sample seed.
- HDR reconstruction permits gain above one relative to a dim instantaneous
  sample, rather than clipping the filtered mean to that sample and biasing the
  result darker. This changes denoiser-enabled reconstruction only.

The filter currently selects the strongest eligible finite runtime emitter per
depth pixel. Other sources, mismatched surfaces and strongly perturbed normals
retain direct RT. It filters the RT lighting signal rather than the saber sprite
itself. Ordinary mathematical point lights have no finite-emitter stochastic
signal to select through this local path.

Restart `target/release/DinurdoJK.exe`, then compare:

```text
r_verbose 1
r_dynamicLights ray_traced
r_rtDenoiser 0
r_rtDenoiser 2
```

Mode 1 is temporal accumulation; mode 2 adds variance-guided spatial filtering.
Wait for the first requested denoiser world variant to become active; the console
reports its compilation/activation. Modes 8, 9 and 10 display local raw visibility,
filtered visibility and their difference; mode 11 identifies the selected source.
Return to 0/1/2 for normal rendering. Glass can remain enabled.

Validation on NVIDIA RTX 3060 / Vulkan driver 581.80:

- GPU receiver readback: sampled-saber metadata, colored reconstruction and
  zero-energy wavelength handling passed; invalid-source/geometry/off cases safely
  retained direct-RT fallback.
- Actual temporal and SVGF GPU passes averaged alternating local signal samples;
  the second frame reconstructed 0.5 from samples 0 and 1.
- Revised denoiser/glass graphics pipeline compiled successfully against the
  native driver.
- Renderer regression suite: 61 passed, with the previously documented unrelated
  minimal-shader performance audit excluded and hardware tests run separately.
- Release client and matching PDB rebuilt and installed in `target/release`.

Validation logs and the incremental source checkpoint are in
`target/rt-local-denoiser-fix/`. These checks establish signal filtering and receiver
consumption; the exact in-game wall/saber view still needs a visual comparison.

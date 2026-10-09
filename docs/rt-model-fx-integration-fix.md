# RT model/FX integration repair — October 8, 2026

Baseline: `Development Docs/Patch20/jka-movement(20261008-124825).zip`.
The intended scope is to preserve its lighting/shadow behavior while adding
denoising and optional tinted-glass transmission.

The normal release EXE/PDB have been rebuilt with these integration repairs:

- `SkinDraw` in the RT compute shader now has the two trailing Source-material
  vectors present in Rust and the raster shader. Its storage-array stride changes
  from 288 to the required 320 bytes. Incorrect later-record reads caused wrong
  transforms; a GPU readback test verifies two independently transformed draws.
- Raster and RT preparation now share GPU FX sprite/blob packing and upload.
  The previous RT branch cleared the FX list and never repopulated it. The GPU
  regression switches raster → RT → raster → RT and checks preserved instances
  and renderable FX pipelines.
- Asynchronously installed RT skinning resources rebind the current bone/draw
  buffers, rather than keeping potentially replaced buffers captured by a worker.
- Classic entity dynamic lighting again follows the ZIP's exact condition:
  suppress it only when the ocean-split pass actually binds RT model receivers.
  The newer code suppressed it outside that pass without a replacement, leaving
  models unlit. The repair restores that condition rather than expanding model
  lighting to an additional path.
- BSP world variants compile on a separate worker so lengthy lighting/shadow
  compilation cannot occupy both model/FX workers. A blocked-world-job regression
  confirms a visible-FX job still finishes.
- Denoiser Off selects a raw RT receiver module without the denoiser/debug
  receiver code. The common resource interface remains compatible; the source
  identity is included in the world variant key. Changing Off ↔ enabled activates
  the matching variant. Temporal/SVGF filtering and tinted-glass traversal remain
  available independently.

Compared directly with the ZIP, `rt_local_sample_count`, `rt_sample_local_emitter`,
`ray_traced_local_shadow_visibility`, `rt_trace_local_visibility`,
`rt_sample_directional_emitter`, and `rt_uncached_sun_visibility` match exactly.
The lighting-state upload differs only by the glass enable/debug bits. Light
strength, falloff, emitter sampling, receiver bias and baseline shadow selection
are preserved. No whole-tree rollback was performed.

Validation:

- GPU multi-record RT skinning readback: passed.
- GPU raster/RT FX preparation transitions: passed.
- Raw RT graphics pipeline driver probe: passed on RTX 3060, Vulkan 581.80.
- World compiler/FX worker isolation: passed.
- Regular renderer suite: 60 passed, 13 ignored; the previously documented,
  unrelated minimal-shader performance audit was explicitly excluded.
- Release build completed; installed EXE and matching PDB are in `target/release`.

For mode-switch diagnostics:

```text
r_verbose 1
r_rtDenoiser 0
r_tintedGlass 0
r_dynamicLights off
r_dynamicLights forward_plus
r_dynamicLights ray_traced
```

The console reports whether requested world shaders are compiling and when the
matching shaders become active. First-use compilation can still delay lighting
activation; FX/model rendering has independent worker capacity. Restart the rebuilt
client and use the same map for the visual check. The open temporary benchmark log
belongs to a separate executable copy, so inspect the release client's `latest.log`
when running `target/release/DinurdoJK.exe`.

Snapshots, known-good comparisons and validation logs are in `target/rt-state-fix/`.

# Tinted-glass RT native crash fix — October 8, 2026

The reported `nvoglv64.dll+0x146B2FC` pipeline compiler crash is resolved in the
rebuilt `target/release/DinurdoJK.exe`. Glass transmission stays enabled in the
shader. The diagnostic launcher and shader-removal switches are unnecessary for
normal use of this build.

Enable dynamic-light RT and colored transmission in the normal client:

```text
r_dynamicLights ray_traced
r_tintedGlass 1
```

`r_rtDenoiser 0|1|2` remains independent. Raster shadowed clustered/local lights
continue using their existing depth cubemaps plus per-receiver glass transmission.

## Change and evidence

The previous scalar visibility and RGB throughput functions each declared their
own `ray_query`. The glass variant crashed even with denoiser receiver code absent,
before any scene draw or compute dispatch. Changing the RGB query's ray flags,
early return, loop control flow, or hit-color calculation did not resolve it.

A single shared `rt_trace_shadow_query(..., colored)` implements both modes now.
The scalar wrapper calls it with `false`, retaining the old `0x204` first-hit flags,
alpha tests and disabled-mode glass classification. The RGB wrapper calls it with
`true`, using `0x200`, accumulating linear RGB through non-opaque panes and returning
zero at an opaque/alpha-tested blocker. Near-emitter bounds and receiver bias remain
the same. The zero-throughput exit terminates the query before leaving its loop.
No light's stored color is changed.

This source-level consolidation removed the reproducible native compiler fault.
It is a driver compiler workaround; these tests do not identify the driver's
internal defect. The model, scalar sun-cache compute, and local denoiser shader
extractors all include the shared helper and its required material functions.

Candidate order is not guaranteed by Vulkan. Thin-pane RGB multiplication is
commutative, and an opaque blocker always produces zero, so an unsorted candidate
walk remains correct for this transmission model. Finite ray bounds exclude panes
behind the receiver or beyond the emitter.

## Reference comparison

- [NVIDIA Q2RTX, pinned shader source](https://github.com/NVIDIA/Q2RTX/blob/f2526e9a165949f66e91e82f0d63aa7bb2567b4d/src/refresh/vkpt/shader/path_tracer_rgen.h#L482-L624):
  `trace_shadow_ray` handles blockers; `trace_caustic_ray` applies texture/material
  color to per-receiver light contribution. Its glass caustic approximation resolves
  a closest transparent hit. This implementation keeps the handoff's additional
  requirement to accumulate multiple thin panes. Q2RTX's GLSL ray-query structure
  was evaluated as a comparison; the final WGSL workaround uses shared query state.
- [EternalJK, pinned shader parser](https://github.com/JKSunny/EternalJK/blob/a4ab639bdfb9da2a3d63e9748eb57a72fb988552/codemp/rd-vanilla/tr_shader.cpp#L2020-L2088):
  the legacy shader-stage and `surfaceparm` context, including generic `trans`
  content semantics. Generic translucency remains insufficient evidence for glass;
  this worktree's authored `lightfilter`, overrides, and conservative heuristics
  remain the material classifier.
- [Vulkan ray traversal specification](https://docs.vulkan.org/spec/latest/chapters/raytraversal.html):
  candidate ordering, opacity, and confirmation rules used to check the traversal.

The optics, glass detection, caster collection, raster cache invalidation, and
debug controls are preserved. This repair does not expand existing support for
animated glass casters or change the bounded raster-caster representation.

## Validation on the user's GPU

NVIDIA GeForce RTX 3060, Vulkan driver 581.80:

- All 24 BSP graphics pipeline probes passed with **both glass and denoiser receiver
  code present**: lean/enhanced, classic/PBR, RT local with/without RT sun, and main/
  legacy-fog/fog-pass entry points. The first of these previously crashed at the
  same NVIDIA module offset as the supplied dump.
- RT model GPU pipelines, emitter sampling, and real sun-shadow compute/readback
  checks passed.
- RT readback: 12 real BLAS/TLAS ray scenes, tested on → off → on. Covered a single
  pane, receiver in front of a pane, stacked colors, opaque walls before/after glass,
  pane past the light, opaque alpha mask, alpha hole, ordinary opaque wall, authored
  opaque lightfilter, transformed inline glass, and sampled sRGB-to-linear tint.
- Raster readback: eight finite light/receiver segments, actual depth-comparison
  cubemaps, populated caster tables, two-pane multiplication, front/behind receivers,
  opaque-depth rejection, a non-glass lane, reversed light direction, and off mode.
- Renderer WGSL fixtures now append the same shared sky module as the runtime fast
  shader. These test-only omissions previously prevented broader renderer checks.

The regular renderer suite finished with **60 passed, one failed, 11 ignored**.
The remaining failure is `minimal_world_variant_shader_audit`, outside this fix:
`runtime_dynamic_light_debug` branches on `camera.render_flags` in the minimal
lean base shader. This repair does not edit that base-shader function or relax its
performance audit. Full renderer WGSL validation passes. The final RT filter has
37 passing tests; the four glass-material/cache tests also pass.

Logs, shader bisection candidates, the scoped patch, and a source checkpoint are
saved under `target/rt-glass-fix/`. Original pre-investigation EXE/PDB remain in
`target/rt-isolation/original-client/`.

The checks exercise compilation and GPU transmission directly. They do not replace
a visual run through the exact map and settings that originally crashed.

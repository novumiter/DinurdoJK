# RT regional TAA A/B test

`r_rtTaa 1` enables a temporal resolve inside conservative runtime-light
influence volumes. `r_rtTaa 0` disables it; `r_rtTaa 2` shows green coverage
against black. It requires hardware RT dynamic lights. Enabling through the
console/menu disables global TAA, FXAA, SMAA and the radiance denoiser, while
retaining the current MSAA sample count. Global TAA takes precedence if both
settings are subsequently enabled. The default is off.

This is **regional scene-color filtering**, including textures/FX within the
covered patches. It is not an isolated lighting buffer. The mask includes
receivers inside a light's radius even when an opaque wall blocks that light;
obtaining an exact visibility mask would require another source of visibility.
Sky and surfaces outside all eligible runtime-light volumes skip temporal
sampling and neighborhood filtering. Authored static lights and RT sun are not
part of this mask. The debug view lets you inspect these limits directly.

The ordinary forward RT material shading supplies every lighting sample. No
additional ray-query pass or denoiser pass is introduced. The camera projection
stays unjittered for MSAA; only the existing finite-emitter random seed advances
each resolved frame. A bilinear reprojected history sample is clipped to the
current 3x3 YCoCg variance box. History alpha contains normalized log depth (or
zero outside the mask), allowing approximate disocclusion/depth rejection in
both existing LDR and HDR history formats. Toggle, target/format change, map
buffer replacement and camera translation cuts reset history.

The resolve is fused into the existing post/presentation pass, reusing its
ping-pong history. There are no new full-resolution textures or additional
fullscreen passes. It still pays for a depth/motion prepass, light-mask checks
and history-target writes across the screen. Its advantage over global TAA is
skipping the temporal neighborhood outside the light regions and using one
history tap instead of five. MSAA also has a cost. Faster FPS than global TAA
is **not established**; compare in the same executable and scene.

The release `base` folder contains three console profiles:

```text
exec rt_aa_msaa.cfg
exec rt_aa_hybrid.cfg
exec rt_aa_taa.cfg
```

They retain the same RT ray budget and other visual settings. The first two
select 2x MSAA; the global TAA setter selects single-sample raster as usual.
Warm up each profile before measuring so asynchronous pipeline compilation and
the first few history frames do not contaminate the comparison. Use the same
position, saber pose, resolution, FPS cap and vsync setting. Compare both a
mostly unaffected view and a wall covered by saber lighting. `r_gpuTimings 1`
and the existing performance capture can separate post/prepass GPU costs.

Validation includes WGSL/Naga validation, configuration archive/MSAA retention,
and a Vulkan GPU probe exercising the production resolve: affected pixels
converge while unaffected pixels remain identical, and empty/removed lights,
invalid history, depth mismatches and reset history return current-frame color.
Both LDR/HDR production post pipelines are compiled in the GPU check. The exact
map/saber view and real FPS comparison remain user A/B checks.

The renderer suite passed 62 tests (16 hardware/diagnostic tests ignored), with
the previously identified unrelated minimal-world shader audit excluded. The
broader frozen compatibility snapshots still fail at the existing sky-fractal
archive insertion and config/network hash mismatches; those snapshots were not
rewritten. The new option's load/archive/MSAA-retention test passed.

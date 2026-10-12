# Native HDR display output

In **Setup → Video → Color & HDR**, choose **Display output → HDR Auto**, then
**Apply Video Settings**. Enable Windows HDR on an HDR-capable monitor first.
The status below the selector shows the actual output mode or SDR fallback.
SDR remains the default; existing configurations keep their SDR output.

| Control | Meaning |
| --- | --- |
| Display output | SDR, HDR Auto, HDR10/PQ, or HDR scRGB. Requires Apply/vid_restart and uses the existing keep/revert timer. |
| HDR paper white | Brightness of diffuse white and UI, in nits; default 200, range 80–500. Limited to the selected peak. Updates live. |
| HDR peak brightness | Zero uses reported display peak, with a 1000-nit fallback. A positive value overrides it, up to 10,000 nits. Updates live. |
| Tone mapping | SDR uses existing ACES; native HDR uses a highlight shoulder bounded by the display peak. Disabling it hard-clips at the peak. |

Console equivalents:

```text
r_hdrOutput 1
r_hdrPaperWhite 200
r_hdrPeak 0
vid_restart
```

`r_hdrOutput`: `0` SDR, `1` Auto, `2` HDR10/PQ, `3` scRGB. `sdr`, `auto`,
`hdr10`/`pq`, and `scrgb` aliases are accepted. These preferences are archived.
The older `r_hdr` controls scene precision and does not by itself enable display
HDR. Native output automatically requires floating-point scene/composition
buffers. Hardware gamma is SDR-only; native output uses shader brightness.

Auto prefers FP16 scRGB, then 10-bit HDR10/PQ. Explicit HDR choices also fall
back to SDR when unavailable. Selection checks each format's advertised color
spaces and the live display information; DX12 advertising scRGB alone does not
prove the current monitor is in HDR mode. Capabilities are polled every two
seconds so monitor/Windows HDR changes can update the final surface without
rebuilding world pipelines. The main window uses native HDR; companion windows
keep their existing SDR presentation.

Scene, post effects, SMAA, HUD and menus compose before final output encoding.
Native output keeps highlights above reference white through post-processing
and LUT grading. Egui paints into its usual gamma-space layer; it is then
unpremultiplied, decoded, and premultiplied in linear light for HDR composition.
This keeps translucent UI at the configured paper white. PNG/JPEG screenshots
and video readbacks use an SDR tone-mapped conversion rather than interpreting
PQ or FP16 bytes as ordinary 8-bit pixels.

The final pass encodes linear BT.709 to scRGB (`1.0 = 80 nits`) or converts to
BT.2020 and ST-2084 PQ for HDR10. This follows [Microsoft's Advanced Color
guidance](https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range)
and [WGPU's surface color-space API](https://docs.rs/wgpu/30.0.0/wgpu/#surface-color-spaces-and-hdr-output).

## Validation

All-target compiler validation, config persistence, shader validation, HDR pixel
transfer, FP16 SMAA, opaque/translucent UI, SDR capture, and native surface
configuration were tested. Full renderer initialization, egui painting and
resize were exercised on Vulkan and DX12 on an RTX 3060.

The display used by the surface test reported SDR, 8-bit output and 270-nit
peak. SDR fallback was therefore verified on real surfaces; HDR signal math was
verified by GPU readback. No claim of HDR-active panel calibration or an in-game
HDR visual/FPS comparison is made. The renderer regression suite had 84 passes,
22 ignored tests and the existing `minimal_world_variant_shader_audit` failure.

```powershell
cargo check -p DinurdoJK --all-targets --offline --locked
cargo test -p DinurdoJK hdr_ --offline --locked
cargo test -p DinurdoJK native_hdr_ --offline --locked -- --include-ignored --nocapture --test-threads=1
```

Validation logs are under the ignored `target/native-hdr-*.log` paths.

The optimized `build.ps1 -Fast` Release build also passed, retaining LTO.
Both `target/release/DinurdoJK.exe` and a checksum-verified dedicated copy,
`target/release/DinurdoJK-NativeHDR.exe`, contain the implementation. Runtime
DLL/asset staging was verified. All seven focused HDR tests passed; the new
settings default to SDR until the user selects and applies an HDR output mode.

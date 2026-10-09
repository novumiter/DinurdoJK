# DinurdoJK RGB Glass Transmission — consolidated handoff

Rebased worktree: `jka-movement(20261008-180443).zip`. The previous RGB glass+sun changes originate from `jka-probe(20261008-162113).zip` and are ported here without reverting the newer worktree changes.

## What is connected

- Material classification: authored `surfaceparm lightfilter` takes precedence. Opt-in exact material overrides using `tinted_glass_overrides.txt`; conservative `glass`/`windowpane` shader-name heuristics are the fallback. Generic `trans` alone does not qualify. Water, sky, alpha-shadow leaves/fences and portals stay out of this path.
- Hardware RT local lights: static BSP and inline BSP candidate BLAS geometry, RGB texture sampling at actual ray hits, multiplicative transmission over multiple panes, opaque and alpha-tested blockers retained. A 0/1 switch preserves scalar visibility when disabled. World and RT model receivers use RGB throughput.
- Raster shadowed local lights: existing opaque depth cubemap retained. At shadow-cache update time, a separate RGB per-light static+inline-BSP triangle list is uploaded. BSP fragment lighting tests the finite *light -> receiver* segment and multiplies all penetrated pane colors, after the normal opaque-depth shadow visibility. Light color is not modified globally. Both lean and enhanced BSP pipelines are handled. An inline model pose change updates the glass slots without re-rendering all six opaque cubemap faces.
- Authored shaderSun directional light now transmits per-receiver RGB through glass on both raster CSM and hardware RT sun shadows. Raster cascades skip eligible glass in the opaque depth pass while still rejecting intervening walls; 8 extra sun-only caster slots (512 triangles total) are bounded per group to reject unrelated pixels cheaply, and rebuilt after inline-glass poses change. Hardware RT uses existing RGB BLAS candidate traversal with the authored sun direction and soft angular sampling; with RGB glass enabled, it bypasses the scalar rg32float sun-visibility history because that format cannot represent independent RGB wavelengths. With mode off, it keeps the original scalar cache.
- Direct ocean sunlight (GGX and water diffuse) uses the same RGB sun shadow term, multiplied by its existing shaderSun color/intensity. BSP classic lightmaps use the existing sun-facing shadow-strength modulation per RGB channel rather than globally recoloring baked lighting. This does not rewrite baked BSP lightmaps.
- Shadow toggle invalidates occupied cubemap slots. Added `r_tintedGlassDebug` visualization (1: magenta affected receivers; 2: absorption RGB). CPU counts/timing at verbose render-log levels and additional Rust unit tests.

## Newer worktree / RT denoiser compatibility

- This rebase preserves the newer RT radiance denoiser. With RGB glass OFF, receiver shadow visibility still follows the denoised scalar path; with glass ON, RT local-light receiver code uses per-channel ray throughput rather than the scalar denoiser output. Scalar sun-history generation is suppressed with RGB sun tint, while the independent local denoiser state remains available.
- Rebased patch applies directly to the October 8 18:04 `jka-movement` ZIP. Earlier tinted-glass patches must **not** be applied on top.

## Settings

- `r_tintedGlass 0` — default/off, restores original local-light/sun scalar shadow behavior and raster opaque caster handling.
- `r_tintedGlass 1` — RGB transmission for supported shadowed direct local lights AND directional shaderSun illumination.
- `r_tintedGlassDebug 0|1|2` — no debug, affected pixels, absorption strength. Use with RGB mode enabled.
- Place `tinted_glass_overrides.txt` in the working directory *before loading the map*, following `tinted_glass_overrides.example.txt`. Exact texture labels: `textures/name/texture = on|off`; `auto = off` disables fallback material-name detection. Overrides read once per process startup (restart after editing). Authored lightfilter takes precedence.

## Constraints and what this is not

- A Rust or WGSL compile could not be performed in this environment: `cargo`/`rustc` are unavailable. The unified patch roundtrip, archive integrity, resource layout invariants, and independent reference segment tests were validated without invoking WGPU.
- The raster implementation intentionally limits each cached light to 64 intersecting **triangles** to bound fragment cost and memory; if exceeded, that slot's tint is bypassed (logged in verbose mode). It is not an unlimited transparent geometry renderer. Raster tint is interpolated from texture-sampled vertex colors, whereas RT samples RGB at the actual hit UV.
- Raster supports world/BSP fragment receivers under shadowed clustered/Forward+ lights. Non-RT skeletal-model entity lighting that bypasses the BSP shadowed direct-light path remains unchanged. Direct unshadowed vertex lights cannot receive spatial colored shadows without a visibility mechanism.
- Raster sunlight supports up to 512 BSP glass triangles over the whole map; if more are present, that map's raster sun tint is deliberately bypassed rather than incorrectly applying only a partial subset. Raster shadows still prevent opaque-wall light leakage. Sun tracing only affects live shadowed contributions, not lightmaps baked into map files.
- RT sun transmission currently traces per receiver while enabled (up to `r_rtSamples` cone samples). Scalar temporal sun cache is retained when off and suspended when RGB tint is on to prevent duplicate shadow rays; RGB temporal caching is not implemented.
- BSP raster and RT world receivers plus direct promoted-ocean sunlight use RGB directional tint. Standalone grass shaders and ordinary non-RT skeletal receivers are not yet wired to directional RGB transmission.
- RT currently only promotes static and inline BSP glass casters, not animated MD3/Ghoul2 glass surfaces. No automatic per-material optical-thickness model is inferred beyond nominal thin-glass throughput.

## Review areas for local Windows compilation

- WGPU resource interface: `@group(2) @binding(17)` shared local/sun glass storage buffer in both `bsp.wgsl` and `bsp_lean.wgsl`, corresponding bind-group layout and resources in `renderer/initialize.rs` / `renderer/world/upload.rs`.
- New material collection: `renderer/lighting/glass_transmission.rs`, `renderer/lighting/ray_tracing.rs`, `renderer/world/inline_models.rs`, `renderer/lighting/rt_resolution.rs`, `renderer/frame/draw.rs`.
- New CPU/GPU settings propagation: `r_tintedGlass`, `r_tintedGlassDebug`.

Deliverable includes full worktree, applyable unified patch, and changed-file checkpoint. This rebased cumulative patch targets `jka-movement(20261008-180443).zip`, not the earlier jka-probe ZIP. No earlier tinted-glass patch needs to be applied.

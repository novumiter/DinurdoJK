# World shader program modes

The console control is `r_fragmentShaderMode 0..4`. Changes compile in the
background and activate a complete pipeline set without a map reload. Mode 1
remains the default; quality presets leave this experiment control alone.

| Mode | Policy |
| --- | --- |
| 0 | Existing general world program. |
| 1 | Existing material specialization through override constants. |
| 2 | Existing specialization plus globally compatible compact entry points. |
| 3 | Shared basic/enhanced families, each with alpha-tested and untested variants. |
| 4 | Generated vertex/fragment pairs for normalized stage recipes encountered by the map. |

Modes 3 and 4 target Minimal Unified, Low and Medium. Their eligibility follows
effective renderer features rather than a preset name. Unhandled configurations,
autosprite2, special tcGen and three-stage legacy RGB composites use the existing
specialized program. Sky and fused effect programs retain their dedicated paths.
High and Ultra settings have not been changed or included in the validation scope.

Four common mode-3 families do not mean four pipeline objects: blend/depth/cull
state, vertex entry, format and MSAA remain independent pipeline dependencies.

Mode 4 generates matching varying declarations, fixes tcGen, emits the actual
tcMod opcode sequence, specializes wave kinds and evaluates uniform colour/alpha
waves in the vertex shader with flat output. Numeric parameters, texture identities,
remap offsets and clamp time remain data. It keeps the existing vertex storage.
Enhanced stages reuse the existing shading equations and retain conservative
inputs for those operations. It does not promise an instruction-count or FPS gain.

## Trying the modes

Select Minimal Unified, Low or Medium and keep a stationary view. Enter:

```
r_fragmentShaderMode 3
r_fragmentShaderMode 4
```

Wait until the requested mode is active before interpreting performance. For a
short scripted comparison, use `exec shader_modes_ab`; the builder installs that
config into the runtime base folder. It changes only shader mode and ends on mode 1.
Sorting remains independently controlled by `r_worldSortMode`.

`perfsample` prints `shader_mode=requested/active` and actual general, specialized,
minimal, shared and generated main-pass program binds, including cached transparent
bundle commands. The older spec/general aggregate is retained for compatibility.
`render_perf_capture` includes the same counters, sorting mode, active variant,
HDR output and presentation-copy controls. GPU timings remain opt-in.

Cold `[WORLD STAGE RECIPES]` logs show prepared unique recipes, program pairs,
pipeline objects, basic/enhanced coverage and fallback reasons. Prepared recipe
counts are not executed draw counts or GPU timings.

## Implementation and validation

Stage data and policy live in `programs.rs`; tests live in `program_tests.rs`.
Program tag and vertex recipe participate in compilation, reuse and binding
identities. Existing reflection/fog/dlight/glow redraw implementations remain in use.

The final renderer regression run passed 113 tests, with 24 hardware tests
ignored. The explicit Vulkan image test passed 85 renders across modes 0..4,
including alpha tests, vertex colours, tcGen/tcMods, all six wave types, active fog
and Medium feature combinations. Maximum compared 8-bit channel difference was
at most one. Hardware validation used an RTX 3060, not an iGPU.

Map smoke validation uses `scripts/test-world-program-modes.ps1`, deriving the
three scoped presets from `app/quality.rs` and sharing only read-only asset
packages with an isolated config/screenshot/capture directory. It checks active
shared/generated program use and switching back to a cached variant on mp/ffa3
and costadelsol. Logs and captures are under `target/shader-mode-validation`.
All six map/preset checks passed on the final release executable, including mode switching and cached reuse. The machine-readable summary is target/shader-mode-validation/validation-summary.json. Those short 640x480 runs are smoke checks, not comparable iGPU benchmarks.

Medium costadelsol exposed an existing deferred transparent bundle format mismatch:
its descriptor used swapchain BGRA8 while scene pipelines targeted FP16 for authored
glow. The descriptor now follows the actual scene attachment format. An explicit
GPU regression covers FP16 bundle creation, reuse and program-bind accounting.

The full client suite run had 1011 passing, 18 failing and 62 ignored tests. The
failing assertions concern other cvar/config, material/scene, input and presentation
tests; none was in the renderer test group. Their log is
`target/shader-modes-full-tests.log`. This change does not claim a green full suite.

The requested `DinurdoJK_TextureCompression_UI_export_compile_fix.patch` was also
applied: `VIDEO_ROW_TEXTURE_COMPRESSION` is exported by `ui.rs`.

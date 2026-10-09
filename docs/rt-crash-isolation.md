# Dynamic-light RT crash isolation

**Update:** the glass compiler crash is now fixed in the normal release client,
with colored transmission retained. See [the fix and validation report](rt-glass-native-fix.md).
The results below describe the original failing implementation and its isolation.

## Reproduced result, October 8, 2026

On this machine's NVIDIA GeForce RTX 3060, Vulkan driver 581.80:

| Receiver shader | Headless graphics-pipeline result |
| --- | --- |
| Both integrations removed | 24/24 pipelines passed |
| Denoiser present, glass removed | 24/24 pipelines passed |
| Glass present, denoiser removed | first pipeline crashed |
| Both present | first pipeline crashed |

Both crashing processes faulted in **`nvoglv64.dll+0x146B2FC`**, with exception
`0xC0000005`, matching the original dump's driver module and offset. Windows
Application Error events are saved in `target/rt-isolation/nvidia-fault-events.json`.
These probes are sequential, with no denoiser compute dispatches or scene draws.
The failing permutation is the enhanced BSP shader, PBR off, RT local shadows on,
RT sun off, `vs_main` / `fs_main`, and an inferred pipeline layout.

Replacing only `rt_trace_shadow_throughput` with a call to the existing scalar
`rt_trace_shadow_visibility` made that pipeline compile. Terminating the query and
breaking instead of the throughput early return still crashed; adding the
terminate-on-first-hit ray flag also still crashed. The remaining suspect is the
new RGB traversal loop or its called glass-hit helper, rather than the denoiser
receiver interface or a need for concurrent pipeline compilation. This isolates a
shader input sufficient to trigger the same driver fault; the precise driver
compiler defect is not established.

The production traversal is retained for further diagnosis. Use `-Mode denoiser`
to remove glass shader code while retaining denoiser support, or `-Mode baseline`
to remove both integrations. Ordinary `r_tintedGlass 0` cannot exclude this compiler
failure because the driver still compiles its uniform-controlled branch.

The diagnostic EXE/PDB were built and placed in `target/release`. Original binaries
are preserved together in `target/rt-isolation/original-client`. The build used a
temporary target directory on C: because D: ran out of space; incremental output
was disabled. Logs and shader comparisons are under `target/rt-isolation/`.

Final checks: 36 RT-related tests passed, including all four receiver variants and
the local denoiser's extracted compute shader. The broader existing
`all_renderer_shaders_validate` test fails because its unchanged `bsp_fast` fixture
omits `sky_fractal.wgsl`, leaving `experimental_star_nest_sky` undefined; the runtime
fast shader builder already appends that module. Its failure is separate from the
RT compiler crash. PowerShell launcher syntax validation passed.

## In-game comparison

The supplied symbolication places the native crash in NVIDIA's Vulkan graphics
pipeline compiler for a lazy BSP world variant. It does not identify the guilty
shader change. Ordinary `r_rtDenoiser 0` and `r_tintedGlass 0` leave their receiver
code in the compiled shader, so they are not sufficient to rule either patch out.

Launch a fresh diagnostic client for each case, using the map
that crashes. Replace `mp/ffa5` below with that map. Exit each surviving run before
starting the next one.

```powershell
.\scripts\rt-isolate.ps1 -Mode baseline -Map mp/ffa5
.\scripts\rt-isolate.ps1 -Mode denoiser -Map mp/ffa5
.\scripts\rt-isolate.ps1 -Mode glass -Map mp/ffa5
.\scripts\rt-isolate.ps1 -Mode full -Map mp/ffa5
```

| Case | Denoiser receiver source/bindings | RGB glass traversal |
| --- | --- | --- |
| baseline | removed; original scalar sun-cache interface | removed |
| denoiser | present | removed |
| glass | removed | present |
| full | present | present |

All four initially run with denoiser **execution** off and glass **transmission**
off. This holds the runtime flags constant while isolating shader compilation.
The source switches are frozen at startup and included in the world pipeline key.
Denoiser removal drops group-3 bindings 17..23 as well as receiver/debug functions
and dispatches. Glass removal strips both raster receiver loops and RGB ray-query
traversal from RT shader modules, including RT model receivers. Alpha/RGBA decoding
and CPU caster tables stay compatible with the current worktree. The baseline case
is therefore a controlled receiver isolation, not a complete historical rollback
of all lighting, geometry, or emitter changes.

- Baseline survives, denoiser crashes: investigate denoiser receiver integration.
- Baseline survives, glass crashes: investigate glass receiver/traversal integration.
- Both individual cases survive, full crashes: investigate the combined compiler input.
- Baseline also crashes: these two receiver integrations are insufficient to explain it.

Repeat any failing case with `-Serial` to serialize only RT BSP pipeline creation
across workers. It leaves model and other job families alone. Stability with this
switch implicates compilation overlap or timing; it is a diagnostic, not proof of
a final fix. Repeat runs to check reproducibility.

Only after the shader comparison, test execution separately:

```powershell
.\scripts\rt-isolate.ps1 -Mode denoiser -Map mp/ffa5 -DenoiserMode 2
.\scripts\rt-isolate.ps1 -Mode glass -Map mp/ffa5 -TintedGlass
```

Each launch saves `latest.log`, generated WGSL, new crash dumps, the settings used,
and `in-flight-pipelines.txt` under `target/rt-isolation/runs/`. Each flushed
`[RT pipeline BEGIN]` contains the material-state key, world specialization key,
shader family/hash, entry points, format, samples, thread, and pipeline hash.
A BEGIN without its END is a candidate operation that was still in the driver.
Multiple unfinished entries can exist when compilation overlaps.

The launcher backs up existing base/japro settings and restores them after exit.
The environment switches are restored in its parent shell after exit.

Also corrected during this investigation: the local denoiser's shader extractor
requested the old `rt_alpha_texel` name after the glass patch introduced
`rt_rgba_texel` / `rt_sample_rgba`. That would panic during local denoiser creation,
separately from the supplied native graphics-pipeline crash.

For manual launches, the switches are `JKA_RT_RECEIVER_DENOISER=0|1`,
`JKA_RT_RECEIVER_GLASS=0|1`, `JKA_RT_PIPELINE_TRACE=1`,
`JKA_RT_SHADER_DUMP_DIR=<directory>`, and `JKA_RT_SERIALIZE_WORLD_PIPELINES=1`.
Unset receiver switches preserve the current full shader path.

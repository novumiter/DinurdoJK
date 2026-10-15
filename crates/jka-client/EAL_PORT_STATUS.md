# Authored EAL acoustics

The Windows EAL path is integrated and tested with OpenAL Soft 1.23.1. It adds
authored room reverberation to Rodio's existing mix. Steam Audio continues to
own direct spatialization/HRTF and geometry-based occlusion/transmission.

## Use

`s_eal` is archived and **defaults to 0**. Enable it with `s_eal 1` and inspect
`soundinfo`. Maps without EAL data retain their existing sound. A missing or
incompatible library disables authored effects and appears in diagnostics.

The executable loads `dinurdo_openal_soft.dll` from its own directory, using an absolute
path. The library must support loopback, EFX EAX reverb, floating-point PCM and
thread-local contexts. The original Creative JKA DLL is insufficient. No new
Cargo dependency or OpenAL vendor source tree is added. The distinct filename
allows JAMP/OpenJK's `OpenAL32.dll` to coexist without being replaced.

## Implementation

- PK3/loose-file loading includes retail `eagle/<map>.eal` and nested MP paths.
- EAX2 and EAX3 presets retain their complete raw fields. Geometry and material
  indices are validated, cyclic/deep trees rejected, and historical understated
  geometry sizes recovered without reading outside the asset buffer.
- Listener zones and source/listener obstacle-matrix lookup follow EalMan's
  one-unit partition tolerance and matrix branch without diffraction.
- World sounds have mono sends before HRTF. Music and local/UI sounds have no
  room send. Authored scalar room attenuation affects the return; Steam direct
  filtering is preserved.
- One persistent worker owns all native initialization, rendering and teardown.
  It uses thread-local OpenAL contexts and two effects processors for a 250 ms
  room crossfade. The old room's tail is faded during that transition.
- Fixed-size SPSC rings exchange 256-frame blocks without waiting for native
  DSP in physical playback. The wet path has a fixed two-block delay (10.67 ms
  at 48 kHz); the direct path has no added delay. Late blocks are discarded,
  faded to silence, and reported as underruns rather than replayed out of time.
- Runtime toggles preserve continuing source clocks. First activation reconnects
  existing sounds at their PCM position. Loops keep normal pitch across demo
  timescale changes; one-shots follow their direct voice's playback speed.
- Map changes, seeks/clear, sound restart and offline export restore/reset the
  appropriate EAL state. Offline export waits for the worker's result while
  retaining the same fixed wet delay; physical playback never takes that wait.
- `soundinfo` includes runtime errors, maximum worker render time, underruns,
  dropped blocks and wet latency. AL/ALC errors and non-finite output fail the
  effects path while ordinary sound continues.

## Validation on 2026-10-14

- Full debug application build succeeded.
- The optimized `build.ps1 -Fast` build succeeded and was copied to
  `target/release/DinurdoJK.exe`. The tested 1.23.1 DLL was placed beside the
  debug and release executables (1,590,272 bytes each).
- All 58 retail JKA EAL files and all 33 supplied EalMan UT fixtures decoded.
- 38,599 sampled listener/source position pairs, including authored plane
  boundaries, matched compiled EalMan v2/v3 listener IDs and matrix attributes
  with `EMFLAG_NODIFFRACTION`.
- All 856 default/environment presets were accepted by native OpenAL Soft EFX.
- Native 44.1/48 kHz impulse tests verified wet-only output and continuing tails;
  concurrent renderers verified context isolation.
- Native worker smoke testing covered room changes, disable/re-enable, offline
  timing and paced live input: no underruns or dropped blocks in that run.
- Regression tests cover malformed geometry, queue ownership/order, slow-worker
  fallback, loop timescale/pause, continuing sends after toggles, late activation
  and local-sound exclusion. Existing audio tests also pass.

See `Development Docs/Patch20/eal-*.log` and `artifacts/eal-validation/` for
local validation evidence and the oracle harness. Broad substring test filters
also selected renderer/asset tests outside this change; those had failures and
are recorded separately. This is not a claim that the entire workspace suite
passes.

## Remaining fidelity work

This remains an authored-reverb integration rather than full EaxMan parity.
Source models, listener globals, diffraction, frequency-dependent per-source
authored filtering and wet directionality are not fully ported. Shared looping
channels still inherit the existing merged-loop approximation. OpenAL Soft
output is not claimed to be sample-identical to Creative hardware.

In-game listening comparisons against original JKA and representative frame-time
benchmarks remain necessary before making EAL the default. The measured worker
smoke test does not establish a game-wide FPS result.

Reference sources: `D:/Code/EalMan-main`, https://github.com/senneschall/EalMan,
and https://github.com/kcat/openal-soft. Retain applicable upstream license and
distribution notices when distributing derived code or the native runtime.

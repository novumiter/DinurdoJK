# SMAA 0.21 upgrade

The Patch20 bundle assumed a WGPU 30 / egui 0.36.2 checkpoint. This checkout
started with WGPU 29.0.4 / egui 0.34.3, so the implementation includes that
prerequisite migration rather than applying the patch verbatim.

- Client: WGPU 30.0.0, egui/egui-wgpu/egui-winit 0.36.2, SMAA 0.21.0.
- Shader validation: Naga 30.0.0, matching the renderer's Naga dependency.
- Real published SMAA source is vendored; the old 0.20 vendor is removed.
- Shared PowerShell helper preserves `glsl` and disables WGPU defaults.
  Its downloads verify the published SHA-256 before extraction.
- Both builders bootstrap a missing vendor and refresh an older graphics
  lockfile once. The committed lockfile already contains the new graph.
- WGPU call sites use optional vertex layouts, fallible buffer ranges and
  `Queue::present`. Readback failures propagate or discard diagnostic samples.
- Egui texture uploads replay each delta for an ID in order, including
  superseded companion-window frames. Texture frees borrow the new set.
- Egui panels use the current `show` API.

This upgrade preserves the existing surface configuration and post-processing
settings. It does not add OS HDR output or the separate HDR Stage 1 handoff.
The subsequent native display-output implementation is documented in
[NATIVE_HDR_OUTPUT.md](NATIVE_HDR_OUTPUT.md).

## Build

Rust 1.95 or newer is required by egui 0.36.2. Normal builds use the committed
vendor and lockfile offline:

```powershell
.\build.ps1 -Fast
```

Use `-Fetch` when dependencies are missing on a fresh machine. The first build
also needs Steam Audio, as before; its build script may download its native SDK.

## Validation

```powershell
.\scripts\test-smaa-vendor.ps1
cargo tree -e features -i wgpu --offline --locked
cargo check -p DinurdoJK --all-targets --offline --locked
cargo test --workspace --offline --locked
cargo test -p DinurdoJK smaa21_ --offline --locked -- --ignored --nocapture
```

The GPU tests exercise SMAA enabled/disabled, BGRA/RGBA sRGB output, resolve
ordering and resize to odd dimensions, with GPU validation and pixel readback.
They require native Vulkan/DX12 hardware. See the validation results below for
the exact limits of the local run.

## Local results (2026-10-11)

- Rust 1.98.1: client/all-target compiler check and complete workspace Debug
  build passed. Test builds disabled debug symbols and incremental compilation.
- NVIDIA RTX 3060: both native SMAA tests passed, including disabled mode,
  both sRGB formats, odd-size resize and exact solid-white pixel readback.
- PowerShell helper tests passed. Its real download/bootstrap also passed;
  every installed vendor file matched the checksum-verified fresh bootstrap.
- The WGPU feature graph contains `dx12`, `vulkan`, `wgsl`, `glsl` and `std`;
  no WGPU default/GLES feature is enabled. Renderer WGSL validation and variant
  SPIR-V tests passed with Naga 30; the existing minimal-world audit still fails.
- Client suite: 936 passed, 17 failed, 58 ignored. All 17 failures also occur
  in an isolated unchanged `HEAD` snapshot (935 passed, 18 failed, 56 ignored;
  that run was serialized and also failed the existing ocean event-loop test).
- Asset suite: 164 passed, one existing skin-parser failure, six ignored.
  Movement integration suite: 24 passed, one existing saber-stance failure.
  Both failures were reproduced individually in the unchanged snapshot.
  Protocol, probe, water, cloth and the other movement suites tested passed.
- `git diff --check` passed. `cargo fmt --all -- --check` still reports existing
  repository formatting differences, beginning in the unchanged
  `jka-assets/examples/compare_juliet_rebind.rs`; no repository-wide reformat
  was applied. The new Rust GPU test file is rustfmt formatted.

The linked Debug binary and test cache are in
`%TEMP%\DinurdoJK-smaa21-target`, on C:, because D: initially lacked room for
the separate test build. After disk cleanup, `build.ps1 -Fast` also completed
the optimized Release build in `target\release\DinurdoJK.exe`, preserving LTO
and using 16 codegen units. Its Steam Audio runtime DLL and staged asset package
were verified. The previous running executable was preserved under a
timestamped `DinurdoJK.smaa21-prev-*.exe` backup in the same directory.
An independent build subsequently moved the completed executable to `.prev`
while rebuilding the normal filename. A checksum-verified copy of the completed
LTO build is available as `target\release\DinurdoJK-SMAA21.exe`.
Logs remain under the workspace's ignored `target\smaa21-*.log` paths. No
in-game visual comparison or Release FPS benchmark was performed.

# SMAA vendor

The client uses the published **smaa 0.21.0** release from crates.io. Its Rust
implementation, GLSL shaders, lookup tables, README and license are unchanged.
Only `Cargo.toml` and `Cargo.toml.orig` disable WGPU's default features. The
upstream `glsl` input feature remains enabled; the client selects Vulkan/DX12.

Published archive:
`https://static.crates.io/crates/smaa/smaa-0.21.0.crate`

Archive SHA-256:
`70f1733b8fc96f93ad2ed99e81f6a3f1fc58a23ad1d8d764ed70016911005c27`

The source is included in this checkout, so normal builds are offline. Both
PowerShell builders call `scripts/ensure-smaa-vendor.ps1`. If the vendor is
missing, the helper copies Cargo's cached upstream source, or downloads and
verifies the pinned archive before installing it from a staging directory.
Already-correct manifests are not rewritten. Incomplete or incorrect vendor
trees cause an explicit error. GLSL input support and WGPU 30 are checked.

# wgpu-core vendor

`wgpu-core-30.0.1` is the crates.io release with one change in
`src/device/queue.rs` (`prepare_surface_texture_for_present`, marked
"DinurdoJK patch"). It is wired in through `[patch.crates-io]` in the root
`Cargo.toml`.

wgpu 30 added a step to `Queue::present` that moves the surface texture to
PRESENT and skips its own queue submit when nothing changed. PRESENT is not an
"ordered" texture use on Vulkan, so the tracker always reports a
PRESENT -> PRESENT transition. The skip never fires, and every frame pays a
redundant barrier plus a second `vkQueueSubmit`: about 20 us per present.
That is about 6% FPS on mp/ffa3 at roughly 2500 fps. The patch drops that no-op
transition. Clearing a never-rendered surface still works as upstream intends.

Measured 2026-10-11 with a bare clear+present loop (Vulkan, Immediate):
wgpu 29 present 72 us, wgpu 30 present 93 us, wgpu 30 patched present 72 us.
Remove the vendor and the patch entry once upstream wgpu fixes this.

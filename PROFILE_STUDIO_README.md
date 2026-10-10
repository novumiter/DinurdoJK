# Profile Studio — second pass (A/B)

This is a source-code revision of the experimental Profile Studio. It uses the
existing WGPU renderer, Ghoul2 player preview, PK3 model/skin icons, saber
catalog and jaPRO cosmetics. It does not replace the original Profile page.

## Installation

- If you **already installed Profile Studio v1**, extract the v2 overlay at your
  project root (the directory containing `crates` in your Windows checkout may
  correspond to the `jka-client` directory in this source bundle; check the
  directory structure first), or apply `DinurdoJK_Profile_Studio_V2.patch`
  against the first Studio source snapshot.
- If you want a full reference snapshot, use `DinurdoJK_Profile_Studio_V2_Full_Source.zip`.
  Like the uploaded source, this is not an entire separately buildable Windows
  repository; retain your own workspace/vendor/asset installation.
- Open via `uipage profilestudio` or the separate Profile Studio main-menu item.
  Classic Profile remains under `uipage profile` / the normal Profile entry.

## What's changed from Studio v1

- Simplified and condensed the top and left UI; removed oversized cut-off title,
  redundant branding, implementation labels, constant loadout/zoom readouts,
  and the Studio-to-Classic switch button (the normal A/B entry points remain).
- Model tint is under LOOK; the NAME page now handles name only.
- Model browser uses search + compact team-filter dropdown and restores quick
  variant selection on hovering a model with multiple ordinary skins.
- For JKA multipart characters, the browser preserves `head_*`, `torso_*` and
  `lower_*` skin names/icons and exposes separate Head/Torso/Legs choices.
  These produce a full `model/head_*|torso_*|lower_*` macro. Entries must exist
  in installed game archives (including their portrait icons).
- Avatar rotates only when dragged; releasing the drag imparts velocity with
  damping. Wheel zoom remains, without a numerical readout. Hovering the stage
  smoothly directs cervical/cranium Ghoul2 head bones toward cursor position.
- Body-area callouts open the same jaPRO cosmetic mask controls, with per-hand
  saber shortcuts to the existing Saber editor. The callout locations are
  stage-relative estimates, not projections from actual skeleton bolts.
- Periodic authored stand-idle animations and a normal-based studio key/fill
  illumination pass remain active. The preview also applies blue/red/etc local
  saber light to *player preview vertices*, evaluated against actual emitted
  saber blade origin, direction and length. This does not change game-world
  lighting, and is not yet a clustered/real-time shadow-casting light.

## Controls

- Drag the character viewport left or right; release to coast and slow down.
- Hover the stage to turn the head toward the cursor.
- Mouse wheel over stage to zoom.
- Hover a model tile to pick its skin (models with variants).
- Use Head/Torso/Legs controls for supported multipart JKA models.
- Click Headwear, Face, Back/Cape around the preview for cosmetics; click a
  saber shortcut to open the existing Saber controls; toggle the visible blade.
- APPLY CHANGES keeps existing non-model apply behavior. The staged player
  model follows the original profile's apply-on-exit behavior.

## Remaining work / boundaries

The **actual `mp/duel3` BSP background and the game's depth-of-field pipeline
are not embedded in the isolated Studio preview**. There is no fake map
screenshot substituted for a live scene. Per-hand shortcuts go to the shared
Saber editor rather than independent floating hilt pickers. Head direction is
a bounded cursor-driven approximation, not a line-of-sight IK solve.

## Verification status

The modified files were statically checked, and the incremental patch was
verified to apply cleanly to the v1 source snapshot. This environment does NOT
have Cargo/rustc, the full project workspace/vendor dependencies, or JKA game
archives, so no successful compilation, real-time visual/animation inspection,
or JKA map rendering is claimed.

## Windows smoke test

1. Compile as usual and open Studio at 1080p/1440p; check top and bottom fit.
2. Spin/fling, zoom, and move the cursor to inspect head bone conventions.
3. Toggle blade with several saber colors; inspect character lighting.
4. Switch to `jedi_hm` / `jedi_hf`; test Head, Torso and Legs independently.
5. Hover a standard model with multiple skins, select a variant in the popup.
6. Click all three cosmetic callouts and the two saber shortcuts.
7. Apply/exit profile and verify model/userinfo/Force/saber behavior still
   matches Classic; retest the original Profile entry for regressions.

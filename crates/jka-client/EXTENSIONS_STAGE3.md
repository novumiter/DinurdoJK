# DinurdoJK Rhai Stage 3 — Custom HUD widgets

This patch applies **after** Stage 2 and its public-fields compile fix.

## Public Rhai API

Call `hud_register(widget_id, title, width, height)` at script top level. Width
and height are **widget-local pixels**, not virtual 640x480 units. The function
returns a boolean and is limited to 16 widgets per extension. Widget IDs are
namespaced automatically by extension (`myext/speed`); they survive reloads.

Inside `on_update()` (or accepted event callbacks) use:

* `hud_text(id, text, x, y, glyph_scale, [r,g,b,a])`
* `hud_rect(id, x, y, width, height, [r,g,b,a])`
* `hud_line(id, x1, y1, x2, y2, thickness, [r,g,b,a])`
* `hud_circle(id, center_x, center_y, radius, [r,g,b,a])`

Color components are in 0..1. These emit a maximum of 256 commands per widget
per script tick; no Rhai code runs in the render thread. Drawing is cached and
sent through the existing UI dynamic vertex pipeline (no extra render pass).
Global drawing is limited to 64 widgets and 4095 extension vertices. Source
scripts are recompiled on the extension worker as before, and broken edits keep
previous working widgets and callbacks active.

Image/texture-atlas loading, custom fonts, precise clip regions, custom widget
anchors and full world-space drawing are **not** included yet.

## Example: speedometer

Paste into Setup → Extensions → main.rhai:

```rhai
hud_register("speed", "Script Speedometer", 170.0, 45.0);

fn on_update() {
    let ps = playerstate();
    hud_rect("speed", 0.0, 0.0, 170.0, 45.0, [0.03, 0.04, 0.07, 0.82]);
    if ps != () {
        let v = ps.velocity;
        let ups = (v[0]*v[0] + v[1]*v[1]).sqrt().round();
        hud_text("speed", `UPS: ${ups}`, 8.0, 12.0, 2.0, [0.4, 0.9, 1.0, 1.0]);
    } else {
        hud_text("speed", "NO PLAYER", 8.0, 12.0, 2.0, [1.0, 1.0, 1.0, 1.0]);
    }
}
```

In HUD Edit, the `EXT: Script Speedometer` overlay rectangle can be dragged.
Select it to show the bottom toolbar: Visible, Scale and RESET. Scroll scales
when selected. Layouts persist in `base/extensions/hud-layout.toml`; uninstalling
or disabling an extension hides its widgets, but saved placements remain for a
future reinstall. The old HUD enum/layout/cvars are unchanged.

## Validation

Run `cargo test -p DinurdoJK hud_` and `cargo check -p DinurdoJK`
from your workspace root after applying. Also test:

1. Join server and move: the number changes, including when spectating.
2. Drag/scale in HUD Edit; quit/relaunch to verify placement persistence.
3. Toggle extension off/on: widget disappears/reappears without stale geometry.
4. Save a changed `.rhai` file: HUD changes without rejoining.
5. Deliberately introduce a syntax error: old widget remains until fixed.
6. Test with no extensions: verify no extra UI rendering pass or draw activity.

Source is built from the last supplied Stage 2 source snapshot; any changes
made locally since then require reconciliation. No Rust toolchain was available
in the patch-creation environment, so compiler validation must occur locally.

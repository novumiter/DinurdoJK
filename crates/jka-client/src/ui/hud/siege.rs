//! Siege-specific HUD overlays ported from TaystJK/OpenJK semantics.

use crate::ui::{fixed_charset_text, rect, rect_outline, UiSnapshot, UiVertex};

fn centered_text(
    out: &mut Vec<UiVertex>,
    value: &str,
    y: f32,
    scale: f32,
    color: [f32; 4],
    w: u32,
    h: u32,
) {
    let glyph_w = 8.0 * scale;
    let glyph_h = 12.0 * scale;
    let advance = 8.0 * scale;
    let x = ((w as f32 - value.chars().count() as f32 * advance) * 0.5).max(4.0);
    fixed_charset_text(out, value, x + 1.0, y + 1.0, glyph_w, glyph_h, advance, [0.0, 0.0, 0.0, 0.75], true, w, h);
    fixed_charset_text(out, value, x, y, glyph_w, glyph_h, advance, color, true, w, h);
}

pub(in crate::ui) fn build_siege_hud(out: &mut Vec<UiVertex>, ui: &UiSnapshot, w: u32, h: u32) {
    let Some(siege) = ui.siege_hud.as_ref() else { return; };

    if let Some(banner) = siege.banner.as_deref() {
        centered_text(out, banner, h as f32 * 0.19, 1.55, [1.0, 1.0, 1.0, 1.0], w, h);
    }

    if let Some(timer) = siege.timer.as_deref() {
        centered_text(out, timer, h as f32 * 0.075, 1.75, siege.timer_color, w, h);
    }

    if let Some(respawn) = siege.respawn.as_deref() {
        centered_text(out, respawn, h as f32 * 0.66, 1.35, [1.0, 0.78, 0.28, 1.0], w, h);
    }

    let Some(stat) = siege.stat_viewer.as_ref() else { return; };
    let scale = (h as f32 / 1080.0).clamp(0.72, 2.0);
    let panel_w = 250.0 * scale;
    let panel_h = 74.0 * scale;
    let x = (w as f32 - panel_w) * 0.5;
    let y = h as f32 * 0.56;
    rect(out, x, y, panel_w, panel_h, [0.015, 0.02, 0.03, 0.80], w, h);
    rect_outline(out, x, y, panel_w, panel_h, (1.0 * scale).max(1.0), [0.45, 0.65, 0.82, 0.70], w, h);

    let name = stat.name.chars().take(24).collect::<String>();
    fixed_charset_text(out, &name, x + 10.0 * scale, y + 8.0 * scale, 7.0 * scale, 10.0 * scale, 7.0 * scale, [1.0; 4], true, w, h);

    let health_max = stat.max_health.max(1);
    let health_frac = (stat.health.max(0) as f32 / health_max as f32).clamp(0.0, 1.0);
    let bar_x = x + 10.0 * scale;
    let bar_w = panel_w - 20.0 * scale;
    let bar_h = 10.0 * scale;
    let health_y = y + 31.0 * scale;
    rect(out, bar_x, health_y, bar_w, bar_h, [0.18, 0.18, 0.18, 0.90], w, h);
    rect(out, bar_x, health_y, bar_w * health_frac, bar_h, [0.28, 0.88, 0.36, 0.95], w, h);
    let health = format!("HP {}/{}", stat.health.max(0), health_max);
    fixed_charset_text(out, &health, bar_x, health_y + 12.0 * scale, 5.5 * scale, 8.0 * scale, 5.5 * scale, [0.90, 0.94, 0.98, 1.0], true, w, h);

    if stat.ammo >= 0 && stat.ammo_max > 0 {
        let ammo_frac = (stat.ammo as f32 / stat.ammo_max as f32).clamp(0.0, 1.0);
        let ammo_y = y + 55.0 * scale;
        rect(out, bar_x, ammo_y, bar_w, bar_h, [0.18, 0.18, 0.18, 0.90], w, h);
        rect(out, bar_x, ammo_y, bar_w * ammo_frac, bar_h, [0.30, 0.62, 1.0, 0.95], w, h);
    }
}

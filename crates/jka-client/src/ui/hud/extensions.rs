//! Fast, read-only Rhai HUD draw-command consumer. Shares the existing UI pass.
use crate::extensions::{HudCommand, PlacedHudWidget};
use crate::ui::{disc, rect, text, UiVertex};

pub(in crate::ui) fn build_extension_hud(
    out: &mut Vec<UiVertex>, widgets: &[PlacedHudWidget], width: u32, height: u32,
) {
    if width == 0 || height == 0 { return; }
    // Reserve space in the preallocated UI dynamic buffer for the stock HUD.
    // Hard ceiling is 4096 extension vertices (1365 triangles), total.
    const MAX_EXTENSION_VERTICES: usize = 4095;
    const UI_DYNAMIC_BUFFER_BYTES: usize = 512 * 1024;
    let extension_start = out.len();
    let gpu_capacity = UI_DYNAMIC_BUFFER_BYTES / std::mem::size_of::<UiVertex>();
    let available = gpu_capacity.saturating_sub(extension_start);
    let limit = MAX_EXTENSION_VERTICES.min(available - available % 3);
    for placed in widgets.iter().take(64) {
        if out.len() - extension_start >= limit { break; }
        let p = placed.placement;
        if !p.visible || !p.scale.is_finite() || p.scale <= 0.0 { continue; }
        let widget = &placed.widget;
        let max_x = widget.width;
        let max_y = widget.height;
        let start = out.len();
        for command in &widget.commands {
            if out.len() - extension_start >= limit { break; }
            match command {
                HudCommand::Text { text: value, x, y, size, color } => {
                    if *x < 0.0 || *y < 0.0 || *x >= max_x || *y + 8.0 * size > max_y { continue; }
                    // The HUD atlas is a 6px monospace font. Clip text by chars
                    // before emitting geometry, including UTF-8/JKA safety.
                    let max_chars = ((max_x - x) / (6.0 * size)).max(0.0) as usize;
                    if max_chars == 0 { continue; }
                    let clipped: String = value.chars().take(max_chars.min(256)).collect();
                    text(out, &clipped, *x, *y, *size, *color, width, height);
                }
                HudCommand::Rect { x, y, w, h, color } => {
                    let x0 = x.max(0.0); let y0 = y.max(0.0);
                    let x1 = (*x + *w).min(max_x); let y1 = (*y + *h).min(max_y);
                    if x1 > x0 && y1 > y0 { rect(out, x0, y0, x1-x0, y1-y0, *color, width, height); }
                }
                HudCommand::Line { x1, y1, x2, y2, thickness, color } => {
                    // Clamp endpoints to the widget rectangle. A true segment
                    // clip is not required for the v1 bounded line primitives.
                    let (x1, y1, x2, y2) = (
                        x1.clamp(0.0, max_x), y1.clamp(0.0, max_y),
                        x2.clamp(0.0, max_x), y2.clamp(0.0, max_y),
                    );
                    let dx=x2-x1; let dy=y2-y1;
                    let len=(dx*dx + dy*dy).sqrt();
                    if len < 0.001 { continue; }
                    let nx = -dy / len * thickness * 0.5;
                    let ny = dx / len * thickness * 0.5;
                    let point = |x: f32,y: f32| UiVertex {
                        position: [x / width as f32 * 2.0 - 1.0, 1.0 - y / height as f32 * 2.0],
                        uv: [0.0, 0.0], color: *color, textured: 0.0,
                    };
                    let a=point(x1+nx,y1+ny); let b=point(x1-nx,y1-ny);
                    let c=point(x2-nx,y2-ny); let d=point(x2+nx,y2+ny);
                    out.extend_from_slice(&[a,b,c,a,c,d]);
                }
                HudCommand::Circle { x, y, radius, color } => {
                    if *x - *radius < 0.0 || *x + *radius > max_x || *y - *radius < 0.0 || *y + *radius > max_y {
                        continue; // no unbounded circle geometry outside the widget
                    }
                    disc(out, *x, *y, *radius, 32, *color, width, height);
                }
            }
        }
        // Do not permit an extension to grow the dynamic HUD buffer without
        // bound, even when it fills every primitive slot with circles/text.
        let remaining = limit.saturating_sub(start - extension_start);
        let after = (out.len() - start).min(remaining);
        out.truncate(start + after - after % 3);
        // Widget-local coordinates -> framebuffer pixels -> NDC.
        let wf=width as f32; let hf=height as f32;
        for v in &mut out[start..] {
            let x=(v.position[0]+1.0)*0.5*wf;
            let y=(1.0-v.position[1])*0.5*hf;
            v.position=[(p.x+x*p.scale)/wf*2.0-1.0, 1.0-(p.y+y*p.scale)/hf*2.0];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{HudPlacement, HudWidget};
    #[test]
    fn widget_rect_uses_existing_ui_triangles() {
        let widget = PlacedHudWidget {
            widget: HudWidget { key: "test/rect".into(), name: "Rect".into(),
                width: 50.0, height: 40.0,
                commands: vec![HudCommand::Rect { x: 2.0, y: 3.0, w: 20.0, h: 10.0, color: [1.0; 4] }] },
            placement: HudPlacement { x: 30.0, y: 25.0, scale: 2.0, visible: true },
        };
        let mut out=Vec::new();
        build_extension_hud(&mut out, &[widget], 640, 480);
        assert_eq!(out.len(), 6);
        assert!((out[0].position[0] - ((34.0 / 640.0) * 2.0 - 1.0)).abs() < 0.0001);
    }
}

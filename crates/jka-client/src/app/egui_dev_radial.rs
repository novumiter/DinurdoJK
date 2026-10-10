//! `/dev`: compact mouse-driven developer wheel drawn over the live scene.
//! A mouse-down on any sector activates it; a mouse-down in the center, the
//! separators, or anywhere outside the wheel dismisses it. Escape is not a
//! dismissal shortcut (the same `/dev` bind may still toggle the wheel).

use super::egui_theme as theme;
use super::*;
use std::f32::consts::{FRAC_PI_2, PI};

const WHEEL_GAP: f32 = 0.045; // radians on either side of each sector
const SEGMENTS_PER_SECTOR: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DevTool {
    Trace,
    DrawEntities,
    EntityViewer,
    RemapShader,
}

impl DevTool {
    const ALL: [Self; 4] = [
        Self::Trace,
        Self::DrawEntities,
        Self::EntityViewer,
        Self::RemapShader,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::DrawEntities => "3D ENTITIES",
            Self::EntityViewer => "ENTITY VIEWER",
            Self::RemapShader => "REMAP SHADER",
        }
    }

    fn subtitle(self, entities_enabled: bool) -> &'static str {
        match self {
            Self::Trace => "INSPECT SURFACE",
            Self::DrawEntities if entities_enabled => "OVERLAY ON",
            Self::DrawEntities => "OVERLAY OFF",
            Self::EntityViewer => "GRAPH EDITOR",
            Self::RemapShader => "COMING SOON",
        }
    }
}

/// The painted annulus, *not* the enclosing rectangle, is clickable.
/// Gaps and the central hub deliberately count as outside clicks.
fn tool_at(pos: egui::Pos2, center: egui::Pos2, inner: f32, outer: f32) -> Option<DevTool> {
    let delta = pos - center;
    let distance_sq = delta.length_sq();
    if distance_sq < inner * inner || distance_sq > outer * outer {
        return None;
    }
    let angle = delta.y.atan2(delta.x);
    let sector = (angle + 3.0 * PI / 4.0).rem_euclid(2.0 * PI) / FRAC_PI_2;
    let index = sector.floor() as usize;
    let progress = sector.fract();
    if !(WHEEL_GAP / FRAC_PI_2..=1.0 - WHEEL_GAP / FRAC_PI_2).contains(&progress) {
        return None;
    }
    DevTool::ALL.get(index).copied()
}

impl App {
    pub(super) fn egui_dev_radial(&mut self, root: &mut egui::Ui) {
        let rect = root.max_rect();
        let center = rect.center();
        let outer = (rect.width().min(rect.height()) * 0.37).clamp(125.0, 205.0);
        let inner = outer * 0.32;
        let label_radius = inner + (outer - inner) * 0.60;
        let painter = root.painter();

        let pointer = root.ctx().input(|input| input.pointer.hover_pos());
        let hovered = pointer.and_then(|pos| tool_at(pos, center, inner, outer));

        // A modest backdrop retains the rendered world as useful context.
        painter.rect_filled(rect, egui::CornerRadius::ZERO, egui::Color32::from_black_alpha(92));
        painter.circle_filled(center, outer + 14.0, egui::Color32::from_black_alpha(54));
        painter.circle_filled(center, outer + 5.0, egui::Color32::from_black_alpha(90));

        for (index, tool) in DevTool::ALL.iter().copied().enumerate() {
            let start = -3.0 * PI / 4.0 + index as f32 * FRAC_PI_2 + WHEEL_GAP;
            let end = start + FRAC_PI_2 - 2.0 * WHEEL_GAP;
            let highlighted = hovered == Some(tool);
            let planned = tool == DevTool::RemapShader;
            let fill = if highlighted && !planned {
                egui::Color32::from_rgba_unmultiplied(38, 94, 151, 240)
            } else if planned {
                egui::Color32::from_rgba_unmultiplied(24, 30, 39, 219)
            } else {
                egui::Color32::from_rgba_unmultiplied(17, 25, 37, 234)
            };
            paint_annular_sector(painter, center, inner, outer, start, end, fill);
            let arc_color = if highlighted && !planned {
                theme::ACCENT
            } else if planned {
                theme::LINE
            } else {
                theme::LINE_STRONG
            };
            paint_arc(painter, center, outer, start, end, egui::Stroke::new(1.5, arc_color));
            paint_arc(painter, center, inner, start, end, egui::Stroke::new(1.0, theme::LINE));

            let angle = -FRAC_PI_2 + index as f32 * FRAC_PI_2;
            let anchor = center + egui::vec2(angle.cos(), angle.sin()) * label_radius;
            let foreground = if planned {
                theme::TEXT_DISABLED
            } else if highlighted {
                egui::Color32::WHITE
            } else {
                theme::TEXT
            };
            let muted = if planned { theme::TEXT_DISABLED } else { theme::TEXT_FAINT };
            paint_tool_icon(painter, anchor + egui::vec2(0.0, -23.0), tool, foreground);
            painter.text(
                anchor + egui::vec2(0.0, 8.0),
                egui::Align2::CENTER_CENTER,
                tool.label(),
                egui::FontId::proportional(if outer < 165.0 { 11.5 } else { 14.0 }),
                foreground,
            );
            painter.text(
                anchor + egui::vec2(0.0, 26.0),
                egui::Align2::CENTER_CENTER,
                tool.subtitle(self.video.draw_entities),
                egui::FontId::proportional(if outer < 165.0 { 9.0 } else { 10.5 }),
                muted,
            );
        }

        painter.circle_filled(center, inner - 3.0, theme::CHROME);
        painter.circle_stroke(center, inner - 2.0, egui::Stroke::new(1.0, theme::LINE_STRONG));
        painter.text(
            center + egui::vec2(0.0, -8.0),
            egui::Align2::CENTER_CENTER,
            "DEV",
            egui::FontId::proportional(25.0),
            theme::TEXT,
        );
        painter.text(
            center + egui::vec2(0.0, 17.0),
            egui::Align2::CENTER_CENTER,
            "TOOLS",
            egui::FontId::proportional(11.0),
            theme::ACCENT,
        );
        painter.text(
            center + egui::vec2(0.0, outer + 28.0),
            egui::Align2::CENTER_CENTER,
            "CLICK OUTSIDE TO CLOSE",
            egui::FontId::proportional(11.0),
            theme::TEXT_DIM,
        );

        // Use press, not click/release. A press on the hub, a separator, or the
        // empty screen dismisses immediately and must not reach gameplay binds.
        let pressed_at = root.ctx().input(|input| {
            input.pointer.any_pressed().then(|| input.pointer.interact_pos()).flatten()
        });
        if let Some(pos) = pressed_at {
            let choice = tool_at(pos, center, inner, outer);
            // Close first so tool entry can change overlays without the radial
            // undoing it. Trace also needs the previous overlay gone before it
            // starts the async crosshair inspection.
            self.close_dev_radial();
            match choice {
                Some(DevTool::Trace) => self.trace_surface_center(),
                Some(DevTool::DrawEntities) => {
                    self.set_draw_entities(!self.video.draw_entities);
                }
                Some(DevTool::EntityViewer) => self.open_entity_graph(None),
                Some(DevTool::RemapShader) | None => {}
            }
        }
    }
}

fn paint_annular_sector(
    painter: &egui::Painter,
    center: egui::Pos2,
    inner: f32,
    outer: f32,
    start: f32,
    end: f32,
    color: egui::Color32,
) {
    let mut mesh = egui::Mesh::default();
    for segment in 0..SEGMENTS_PER_SECTOR {
        let a0 = start + (end - start) * segment as f32 / SEGMENTS_PER_SECTOR as f32;
        let a1 = start + (end - start) * (segment + 1) as f32 / SEGMENTS_PER_SECTOR as f32;
        let dir0 = egui::vec2(a0.cos(), a0.sin());
        let dir1 = egui::vec2(a1.cos(), a1.sin());
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(center + dir0 * inner, color);
        mesh.colored_vertex(center + dir0 * outer, color);
        mesh.colored_vertex(center + dir1 * outer, color);
        mesh.colored_vertex(center + dir1 * inner, color);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    start: f32,
    end: f32,
    stroke: egui::Stroke,
) {
    let points = (0..=SEGMENTS_PER_SECTOR)
        .map(|step| {
            let angle = start + (end - start) * step as f32 / SEGMENTS_PER_SECTOR as f32;
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));
}

fn paint_tool_icon(painter: &egui::Painter, center: egui::Pos2, tool: DevTool, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.7, color);
    match tool {
        DevTool::Trace => {
            painter.circle_stroke(center, 9.0, stroke);
            painter.circle_filled(center, 2.0, color);
            for (start, end) in [
                (egui::vec2(-15.0, 0.0), egui::vec2(-6.0, 0.0)),
                (egui::vec2(6.0, 0.0), egui::vec2(15.0, 0.0)),
                (egui::vec2(0.0, -15.0), egui::vec2(0.0, -6.0)),
                (egui::vec2(0.0, 6.0), egui::vec2(0.0, 15.0)),
            ] {
                painter.line_segment([center + start, center + end], stroke);
            }
        }
        DevTool::DrawEntities => {
            let pts = [
                egui::vec2(0.0, -13.0), egui::vec2(12.0, -6.0),
                egui::vec2(12.0, 8.0), egui::vec2(0.0, 15.0),
                egui::vec2(-12.0, 8.0), egui::vec2(-12.0, -6.0),
            ];
            for i in 0..6 {
                painter.line_segment([center + pts[i], center + pts[(i + 1) % 6]], stroke);
            }
            for (a, b) in [
                (egui::vec2(0.0, 0.0), pts[1]),
                (egui::vec2(0.0, 0.0), pts[5]),
                (egui::vec2(0.0, 0.0), pts[3]),
            ] {
                painter.line_segment([center + a, center + b], stroke);
            }
        }
        DevTool::EntityViewer => {
            for (a, b) in [
                (egui::vec2(-11.0, -9.0), egui::vec2(12.0, -7.0)),
                (egui::vec2(-11.0, -9.0), egui::vec2(-2.0, 12.0)),
                (egui::vec2(12.0, -7.0), egui::vec2(-2.0, 12.0)),
            ] {
                painter.line_segment([center + a, center + b], stroke);
            }
            for at in [egui::vec2(-11.0, -9.0), egui::vec2(12.0, -7.0), egui::vec2(-2.0, 12.0)] {
                painter.circle_filled(center + at, 4.0, color);
            }
        }
        DevTool::RemapShader => {
            let rect = egui::Rect::from_center_size(center, egui::vec2(24.0, 20.0));
            painter.rect_stroke(rect, egui::CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            painter.line_segment([center + egui::vec2(-9.0, 5.0), center + egui::vec2(9.0, -5.0)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_picks_only_live_sectors() {
        let center = egui::pos2(300.0, 300.0);
        let inner = 64.0;
        let outer = 200.0;
        assert_eq!(tool_at(center + egui::vec2(0.0, -140.0), center, inner, outer), Some(DevTool::Trace));
        assert_eq!(tool_at(center + egui::vec2(140.0, 0.0), center, inner, outer), Some(DevTool::DrawEntities));
        assert_eq!(tool_at(center + egui::vec2(0.0, 140.0), center, inner, outer), Some(DevTool::EntityViewer));
        assert_eq!(tool_at(center + egui::vec2(-140.0, 0.0), center, inner, outer), Some(DevTool::RemapShader));
        assert_eq!(tool_at(center, center, inner, outer), None);
        assert_eq!(tool_at(center + egui::vec2(250.0, 0.0), center, inner, outer), None);
        assert_eq!(tool_at(center + egui::vec2(100.0, -100.0), center, inner, outer), None);
    }
}

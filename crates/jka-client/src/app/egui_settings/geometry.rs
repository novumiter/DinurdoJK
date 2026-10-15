//! Curved BSP surface quality and diagnostics on Video -> Geometry.
use crate::app::egui_settings::{optional_quality_table_row, theme, ui, App};

impl App {
    pub(in crate::app) fn set_patch_menu_cvar(&mut self, name: &str, value: &str) {
        // An explicit menu choice replaces a previously latched console choice.
        if name == "r_subdivisions" {
            self.latched_console_cvars.remove("r_subdivisions");
        }
        let _ = self.set_console_cvar(name, value);
    }

    pub(in crate::app::egui_settings) fn egui_patches(&mut self, ui: &mut egui::Ui) {
        theme::section(
            ui,
            "CURVED SURFACES",
            "Patch smoothness, distance detail and shared edges.",
        );

        theme::row(ui, "Draw curved surfaces",
            "Shows the map's curved surfaces. Turning this off is a rendering diagnostic and does not change collision.",
            theme::Reset::Video(ui::VIDEO_ROW_DRAW_CURVES), |ui| {
                if let Some(enabled) = theme::switch(ui, !self.video.no_curves) {
                    let _ = self.set_console_cvar("r_nocurves", if enabled { "0" } else { "1" });
                }
            });

        const DETAIL: [(f32, &str); 5] = [
            (12.0, "Minimal"),
            (8.0, "Low"),
            (4.0, "Medium"),
            (2.0, "High"),
            (1.0, "Ultra"),
        ];
        let requested_subdivisions = f32::from_bits(self.active_map_prepare_options().patch_subdivisions_bits);
        if let Some(index) = optional_quality_table_row(ui, "Curve detail",
            "Makes curved walls and surfaces smoother. Higher quality generates more geometry when the map loads. Applies with Apply Video Settings or the next map load. Console: r_subdivisions; lower values give more detail.",
            theme::Reset::Video(ui::VIDEO_ROW_PATCH_SUBDIVISIONS), Some(requested_subdivisions), &DETAIL) {
            self.set_patch_menu_cvar("r_subdivisions", &DETAIL[index].0.to_string());
        }

        const DISTANCE: [(f32, &str); 4] = [
            (125.0, "Short"),
            (250.0, "Normal"),
            (1000.0, "Far"),
            (2500.0, "Very far"),
        ];
        if let Some(index) = optional_quality_table_row(ui, "Curve detail distance",
            "Retains more curved-surface detail farther from the camera. Higher settings draw more triangles. Changes immediately. Console: r_lodCurveError.",
            theme::Reset::Video(ui::VIDEO_ROW_PATCH_LOD_ERROR), Some(self.video.lod_curve_error), &DISTANCE) {
            let _ = self.set_console_cvar("r_lodCurveError", &DISTANCE[index].0.to_string());
        }

        theme::row(ui, "Stitch patch edges",
            "Repairs compatible mismatched edges between neighboring curved surfaces to avoid gaps. Recommended on at every quality level. Applies with Apply Video Settings or the next map load.",
            theme::Reset::Video(ui::VIDEO_ROW_PATCH_STITCHING), |ui| {
                let preview = theme::preview_toggle_value(ui.ctx(), theme::Reset::Video(ui::VIDEO_ROW_PATCH_STITCHING), self.video.patch_stitching);
                if let Some(enabled) = theme::switch(ui, preview) {
                    let _ = self.set_console_cvar("r_patchStitching", if enabled { "1" } else { "0" });
                }
            });

        theme::row(ui, "Patch statistics",
            "Prints patch triangle counts, index uploads and CPU LOD time to the console once per second. Diagnostic only; presets leave this preference unchanged.",
            theme::Reset::Video(ui::VIDEO_ROW_PATCH_STATS), |ui| {
                if let Some(enabled) = theme::switch(ui, self.video.speeds == 8) {
                    let _ = self.set_console_cvar("r_speeds", if enabled { "8" } else { "0" });
                }
            });
    }
}

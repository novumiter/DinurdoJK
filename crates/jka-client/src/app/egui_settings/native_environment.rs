//! Native Blue Water atmosphere, clouds and weather controls.
use super::{App, theme};
fn slider(ui: &mut egui::Ui, label: &str, value: &mut f32, low: f32, high: f32) -> bool {
    let mut changed = false;
    theme::row(ui, label, label, theme::Reset::None, |ui| {
        let text = format!("{:.2}", *value);
        changed = theme::slider(ui, value, low..=high, &text);
    });
    changed
}
fn choice(ui: &mut egui::Ui, label: &str, value: &mut u8, choices: &[&str]) -> bool {
    let old = *value;
    theme::row(ui, label, label, theme::Reset::None, |ui| {
        egui::ComboBox::from_id_salt(label)
            .selected_text(choices[(*value as usize).min(choices.len() - 1)])
            .show_ui(ui, |ui| {
                for (i, name) in choices.iter().enumerate() {
                    ui.selectable_value(value, i as u8, *name);
                }
            });
    });
    old != *value
}
impl App {
    pub(super) fn egui_native_clouds(&mut self, ui: &mut egui::Ui) {
        let mut s = self.video.blue_water_environment;
        let mut changed = false;
        theme::section(
            ui,
            "BLUE WATER CLOUDS",
            "Native cloud controls. Heights use metres above the map's main sea plane.",
        );
        changed |= choice(
            ui,
            "Native quality",
            &mut s.quality,
            &["Low", "Medium", "High", "Ultra"],
        );
        changed |= choice(
            ui,
            "Native cloud type",
            &mut s.genus,
            &[
                "Automatic weather",
                "Cumulus",
                "Stratus",
                "Cumulonimbus",
                "Stratocumulus",
                "Nimbostratus",
                "Altocumulus",
                "Altostratus",
            ],
        );
        changed |= slider(ui, "Cloud base (m)", &mut s.altitude, 20.0, 15000.0);
        changed |= slider(ui, "Cloud thickness (m)", &mut s.thickness, 50.0, 12000.0);
        changed |= slider(ui, "Horizon coverage", &mut s.horizon, 0.0, 1.0);
        changed |= slider(ui, "Cloud ambient light", &mut s.ambient, 0.0, 3.0);
        changed |= slider(ui, "Cloud base shadow", &mut s.base_shadow, 0.0, 1.0);
        changed |= choice(
            ui,
            "High clouds",
            &mut s.high_genus,
            &["Automatic", "Cirrus", "Cirrostratus", "Cirrocumulus"],
        );
        changed |= slider(ui, "High cloud amount", &mut s.high_amount, 0.0, 1.0);
        changed |= slider(
            ui,
            "High cloud altitude (m)",
            &mut s.high_altitude,
            2000.0,
            20000.0,
        );
        if changed {
            self.video.blue_water_environment = s.sanitize();
            self.sync_post_effects();
            self.mark_config_dirty();
            self.publish_ui();
        }
    }
    pub(super) fn egui_native_sky(&mut self, ui: &mut egui::Ui) {
        let mut s = self.video.blue_water_environment;
        let mut changed = false;
        theme::section(
            ui,
            "BLUE WATER SKY",
            "Uses the Sun source, direction and intensity controls above. Atmosphere changes recompute its lookup tables.",
        );
        changed|=ui.checkbox(&mut s.world_lighting,"Sky world lighting (approximate)").on_hover_text("Use the native sky's ambient and celestial light on opaque terrain/models. Baked lighting cannot be separated exactly; dark regions are preserved but this is not a replacement for baked indoor/GI lighting.").changed();
        changed |= ui.checkbox(&mut s.filmic, "Native highlight mapping")
            .on_hover_text("Preserve the difference between bright lightning channels and sky/cloud glow with a filmic display curve. Applies to the scene while native sky or clouds are selected.").changed();
        changed |= slider(ui, "Moon phase", &mut s.moon_phase, 0.0, 1.0);
        ui.label("Moon phase: 0/1 new, 0.5 full.");
        changed |= ui
            .checkbox(&mut s.moon_custom, "Set moon position manually")
            .changed();
        ui.add_enabled_ui(s.moon_custom, |ui| {
            changed |= slider(ui, "Moon elevation", &mut s.moon_elevation, -90.0, 90.0);
            changed |= slider(ui, "Moon azimuth", &mut s.moon_azimuth, 0.0, 360.0);
        });
        changed |= slider(ui, "Rayleigh / blue scattering", &mut s.rayleigh, 0.0, 1.5);
        changed |= slider(ui, "Atmospheric turbidity", &mut s.turbidity, 1.0, 10.0);
        changed |= slider(ui, "Mie / sun aureole", &mut s.mie, 0.0, 2.0);
        changed |= slider(ui, "Mie directionality", &mut s.mie_g, 0.0, 0.95);
        changed |= slider(ui, "Multiple scattering", &mut s.multiple, 0.0, 3.0);
        if changed {
            self.video.blue_water_environment = s.sanitize();
            self.sync_post_effects();
            self.mark_config_dirty();
            self.publish_ui();
        }
    }
    pub(super) fn egui_native_weather(&mut self, ui: &mut egui::Ui) {
        let mut s = self.video.blue_water_environment;
        let mut changed = false;
        theme::section(
            ui,
            "WEATHER RENDERER",
            "Choose one weather renderer. Saved JKA weather settings are retained when switching.",
        );
        changed |= choice(
            ui,
            "Weather renderer",
            &mut s.weather_backend,
            &["JKA weather", "Blue Water weather", "Off"],
        );
        if s.weather_backend == 1 {
            changed |= slider(ui, "Native rainfall", &mut s.rain, 0.0, 1.0);
            changed |= slider(ui, "Rain fall speed (x)", &mut s.rain_speed, 0.25, 4.0);
            changed |= slider(
                ui,
                "Lightning strikes / minute",
                &mut s.lightning,
                0.0,
                30.0,
            );
            changed |= slider(ui, "Sea fog banks", &mut s.fog, 0.0, 1.0);
            changed |= ui.checkbox(&mut s.strike_preview, "Hold ground lightning preview")
                .on_hover_text("Hold one seeded, branched ground strike 2.6 km in front of your current position. Turn off to resume the natural storm. This diagnostic is not saved.")
                .changed();
            changed |= slider(
                ui,
                "Haze visibility (m)",
                &mut s.visibility,
                200.0,
                100000.0,
            );
            ui.label(
                "Set rainfall or lightning to zero to disable each. Fog banks use native clouds.",
            );
        }
        if changed {
            self.video.blue_water_environment = s.sanitize();
            self.sync_post_effects();
            self.mark_config_dirty();
            self.publish_ui();
        }
    }
}

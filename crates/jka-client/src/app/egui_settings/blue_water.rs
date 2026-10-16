//! Blue Water backend selection and the controls currently consumed by the port.
use crate::app::egui_settings::{theme, App};

impl App {
    pub(in crate::app::egui_settings) fn egui_blue_water(&mut self, ui: &mut egui::Ui) {
        let hybrid = self.video.ocean_settings.backend == 2;
        theme::section(
            ui,
            if hybrid { "HYBRID OCEAN" } else { "BLUE WATER OCEAN" },
            if hybrid { "Native Blue Water surface above; JKA/Godot optics and surface while submerged." }
            else { "Original Blue Water ocean surface, waves and optical material." },
        );
        let mut ocean = self.video.ocean_settings;
        let mut changed = false;
        theme::section(ui, "WAVES", "Native Blue Water parameters, shared across promoted pools. Changes apply live.");
        super::ocean_slider(ui, "Wave height", "Significant wave height in metres (4 times the surface height standard deviation). Zero is flat.", theme::Reset::None,
            &mut ocean.blue_water_waves.height, 0.0..=20.0, &mut changed);
        super::ocean_slider(ui, "Swell direction", "Direction waves travel, in degrees from +X toward world +Z.", theme::Reset::None,
            &mut ocean.blue_water_waves.direction, -180.0..=180.0, &mut changed);
        super::ocean_slider(ui, "Peak wavelength", "Dominant crest spacing in metres. Controls wave speed through the native dispersion relation.", theme::Reset::None,
            &mut ocean.blue_water_waves.wavelength, 2.0..=512.0, &mut changed);
        super::ocean_slider(ui, "Choppiness", "Horizontal displacement strength. Zero keeps only vertical waves.", theme::Reset::None,
            &mut ocean.blue_water_waves.choppiness, 0.0..=4.0, &mut changed);
        super::ocean_slider(ui, "Spectrum wind speed", "Reference wind speed in metres per second. Changes the native spectral shape and fine surface roughness; independent of weather wind.", theme::Reset::None,
            &mut ocean.blue_water_waves.wind_speed, 0.1..=40.0, &mut changed);
        if theme::ghost_button(ui, "RESET WAVES").clicked() {
            ocean.blue_water_waves = crate::ocean::BlueWaterWaves::default();
            changed = true;
        }
        theme::row(
            ui,
            "FFT update rate",
            "How often the Blue Water wave field advances. Zero removes the rate cap.",
            theme::Reset::None,
            |ui| {
                let mut value = ocean.updates_per_second;
                let readout = if value == 0.0 {
                    "Uncapped".to_owned()
                } else {
                    format!("{value:.0} Hz")
                };
                if theme::slider(ui, &mut value, 0.0..=60.0, &readout) {
                    ocean.updates_per_second = value;
                    changed = true;
                }
            },
        );
        theme::section(ui, "NATIVE MESH & PERFORMANCE A/B", "Godot mesh quality does not affect this native Blue Water mesh. Each change applies live.");
        theme::row(ui, "Native mesh segments", "Blue Water only: 64, 128 (reference), 256. Rebuilds native mesh buffers.", theme::Reset::None, |ui| {
            for segments in [64, 128, 256] {
                if theme::chip(ui, &segments.to_string(), ocean.blue_water_mesh_segments == segments).clicked() {
                    ocean.blue_water_mesh_segments = segments;
                    changed = true;
                }
            }
        });
        theme::row(ui, "Native clipmap levels", "Blue Water only: 6 to 9, reference 9. Fewer rings reduce horizon reach.", theme::Reset::None, |ui| {
            for levels in [6, 7, 8, 9] {
                if theme::chip(ui, &levels.to_string(), ocean.blue_water_mesh_levels == levels).clicked() {
                    ocean.blue_water_mesh_levels = levels;
                    changed = true;
                }
            }
        });
        theme::row(ui, "Screen-space reflections", "Trace nearby scene reflections. Environment and sky remain when disabled.", theme::Reset::None, |ui| {
            let value = if hybrid { &mut ocean.hybrid_ssr } else { &mut ocean.blue_water_ssr };
            if ui.checkbox(value, "Trace scene reflections").changed() { changed = true; }
        });
        theme::row(ui, "SSR trace steps", "Compile-time ray budget. 16 is upstream; fewer steps can speed up water-heavy views but reduce hit quality. Rebuilds only the native surface shader.", theme::Reset::None, |ui| {
            for count in [4, 8, 12, 16, 24, 32] {
                if theme::chip(ui, &count.to_string(), ocean.blue_water_ssr_steps == count).clicked() {
                    ocean.blue_water_ssr_steps = count; changed = true;
                }
            }
        });
        theme::row(ui, "JKA boundary tests", "Exact binned uses per-tile triangle candidates and keeps the precise footprint/PVS test.", theme::Reset::None, |ui| {
            for (mode, name) in [(0, "Exact"), (3, "Exact binned"), (1, "Box/PVS"), (2, "Off/test")] {
                if theme::chip(ui, name, ocean.blue_water_boundary_mode == mode).clicked() {
                    ocean.blue_water_boundary_mode = mode; changed = true;
                }
            }
        });
        theme::row(ui, "Caustics", "Godot carries the inexpensive optics caustics into BlueWater transmission; native modes use its separate receiver pass.", theme::Reset::None, |ui| {
            for &(mode, name) in if hybrid { &[(3, "Godot"), (0, "Native"), (1, "Native FFT only"), (2, "Off")][..] } else { &[(0, "Native"), (1, "Native FFT only"), (2, "Off")][..] } {
                if theme::chip(ui, name, (if hybrid { ocean.hybrid_caustics_mode } else { ocean.blue_water_caustics_mode }) == mode).clicked() {
                    if hybrid { ocean.hybrid_caustics_mode = mode; } else { ocean.blue_water_caustics_mode = mode; } changed = true;
                }
            }
        });
        theme::row(ui, "Refraction method", "Screen offset restores seabed bending with the inexpensive Godot approach. Ray trace is the more expensive physical scene trace.", theme::Reset::None, |ui| {
            for &(mode, name) in if hybrid { &[(1, "Screen offset"), (2, "Ray trace"), (0, "Off")][..] } else { &[(2, "Ray trace"), (1, "Screen offset"), (0, "Off")][..] } {
                if theme::chip(ui, name, (if hybrid { ocean.hybrid_refraction_mode } else { ocean.blue_water_refraction_mode }) == mode).clicked() {
                    if hybrid { ocean.hybrid_refraction_mode = mode; } else { ocean.blue_water_refraction_mode = mode; } changed = true;
                }
            }
        });
        theme::row(ui, "Underwater effects", "Hybrid uses the Godot surface and underwater optics while submerged; native modes keep Blue Water scattering and waterline. Above water, Hybrid keeps the Blue Water surface.", theme::Reset::None, |ui| {
            for &(mode, name) in if hybrid { &[(4, "Godot submerged"), (0, "Native"), (3, "Native submerged"), (1, "No motes"), (2, "Off/test")][..] } else { &[(0, "Native"), (3, "Native submerged"), (1, "No motes"), (2, "Off/test")][..] } {
                if theme::chip(ui, name, (if hybrid { ocean.hybrid_underwater_mode } else { ocean.blue_water_underwater_mode }) == mode).clicked() {
                    if hybrid { ocean.hybrid_underwater_mode = mode; } else { ocean.blue_water_underwater_mode = mode; } changed = true;
                }
            }
        });
        theme::row(ui, "Update Godot mesh", "Diagnostic A/B. Legacy clipmap indices are not drawn by Blue Water. OFF avoids their CPU updates/uploads.", theme::Reset::None, |ui| {
            if ui.checkbox(&mut ocean.blue_water_prepare_godot_mesh, "Legacy updates (A/B)").changed() { changed = true; }
        });
        theme::row(ui, "Skip duplicate scene copy", "A/B: native water already has a scene copy; when no authored water, submerged optics, Godot caustics or glass need it, omit the separate Godot capture. Default OFF.", theme::Reset::None, |ui| {
            if ui.checkbox(&mut ocean.blue_water_dedup_capture, "Deduplicate safe captures").changed() { changed = true; }
        });
        theme::row(ui, "Legacy wave adapter", "Diagnostic A/B. OFF leaves old JKA water textures stale; native surface continues using Blue Water waves.", theme::Reset::None, |ui| {
            if ui.checkbox(&mut ocean.blue_water_adapter, "Convert to Godot textures").changed() { changed = true; }
        });
        theme::banner(ui, "For native ocean wireframe use r_showtris 32. render_perf_capture CSV includes native caustic timing and mesh resolution.", theme::TEXT_DIM);
        theme::section(ui, "WATER APPEARANCE", "Changes apply live above and below the surface.");
        theme::row(ui, "Transparency", if hybrid {
            "Hybrid clarity multiplier: controls both native surface transmission and Godot underwater fog distance. 1x leaves Godot's saved distance unchanged."
        } else {
            "Higher values let light travel farther through water. 1x preserves native calibration."
        }, theme::Reset::None, |ui| {
            let mut value = if hybrid { ocean.hybrid_transparency } else { ocean.blue_water_optics.transparency };
            let readout = format!("{value:.2}x");
            if theme::slider(ui, &mut value, 0.1..=10.0, &readout) {
                if hybrid { ocean.hybrid_transparency = value; } else { ocean.blue_water_optics.transparency = value; }
                changed = true;
            }
        });
        if hybrid {
            super::color_row(ui, "Water color", "Hybrid surface body tint; does not modify native Blue Water's saved color.", theme::Reset::None,
                &mut ocean.hybrid_water_color, &mut changed);
            super::color_row(ui, "Underwater fog color", "Hybrid color for both Godot submerged optics and Blue Water scattering; independent of the Godot and native presets.", theme::Reset::None,
                &mut ocean.hybrid_fog_color, &mut changed);
        } else {
            super::color_row(ui, "Water color", "Tint of the native water body and crest transmission.", theme::Reset::None,
                &mut ocean.blue_water_optics.water_color, &mut changed);
            super::color_row(ui, "Underwater fog color", "Color of native Blue Water scattered light below the surface.", theme::Reset::None,
                &mut ocean.blue_water_optics.fog_color, &mut changed);
        }
        if theme::ghost_button(ui, "RESET WATER APPEARANCE").clicked() {
            if hybrid {
                ocean.hybrid_transparency = 1.0;
                ocean.hybrid_fog_color = crate::ocean::optics::Settings::default().fog_color;
                ocean.hybrid_water_color = crate::ocean::BlueWaterOptics::default().water_color;
            } else {
                ocean.blue_water_optics = crate::ocean::BlueWaterOptics::default();
            }
            changed = true;
        }
        if hybrid {
            theme::section(ui, "HYBRID CAUSTIC SHAPE", "The 181m Blue Water mid-cascade makes larger patterns than Godot's 57m cascade. These controls only affect Hybrid's Godot receiver caustics (mode: Godot). No extra passes or textures.");
            super::ocean_slider(ui, "Pattern density", "Higher values create smaller caustic markings by tiling the existing Blue Water slope texture. 3x roughly compensates for the 181m vs 57m wave tile lengths.", theme::Reset::None,
                &mut ocean.hybrid_caustics_scale, 1.0..=6.0, &mut changed);
            super::ocean_slider(ui, "Focus gain", "Additional Hybrid-only brightness/contrast after wave curvature calculation. Godot caustic strength is still the master control.", theme::Reset::None,
                &mut ocean.hybrid_caustics_gain, 0.0..=4.0, &mut changed);
            theme::row(ui, "Fast receiver", "A/B: forward differences use 3 normal texture taps per caustic pixel instead of 4, with slightly less stable focusing. Keeps exact outline and scene-depth tests.", theme::Reset::None, |ui| {
                if ui.checkbox(&mut ocean.hybrid_caustics_fast, "3-tap caustics (A/B)").changed() { changed = true; }
            });
        }
        super::color_row(
            ui,
            "Foam color",
            "Tint for the original Blue Water crest and surface foam.",
            theme::Reset::None,
            &mut ocean.foam_color,
            &mut changed,
        );
        super::ocean_slider(
            ui,
            "Surface refraction",
            "Wave distortion used by the original surface's scene transmission.",
            theme::Reset::None,
            &mut ocean.blue_water_refraction,
            0.0..=0.15,
            &mut changed,
        );
        super::ocean_slider(
            ui,
            "Caustic strength",
            "Strength of the selected Godot or native Blue Water caustics.",
            theme::Reset::None,
            &mut ocean.optics.caustics,
            0.0..=4.0,
            &mut changed,
        );
        if hybrid {
            super::ocean_slider(ui, "Caustic view distance", "Godot-style Hybrid caustics: skip distant terrain receivers and fade the last 25% of range, in metres. 0 disables culling. Native caustics use their own height gate.", theme::Reset::None,
                &mut ocean.optics.caustic_max_distance_m, 0.0..=500.0, &mut changed);
        }
        if changed {
            self.video.ocean_settings = ocean;
            self.commit_ocean_settings();
        }

        theme::banner(ui, "Use render_perf_capture for ocean GPU pass timings. r_showtris 32 draws this backend's actual mesh. Sky/cloud backends live in Clouds.", theme::TEXT_DIM);
    }
}

//! Native display output preferences, kept separate from r_hdr scene precision.
use super::cvar_prelude::*;

fn mode(value: &str) -> Result<u32, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "off" | "sdr" => Ok(0),
        "1" | "auto" => Ok(1),
        "2" | "hdr10" | "pq" => Ok(2),
        "3" | "scrgb" => Ok(3),
        _ => Err("r_hdrOutput: expected 0 SDR, 1 Auto, 2 HDR10 or 3 scRGB".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_hdr_preferences_archive_and_reload_without_changing_scene_controls() {
        let mut video = ui::VideoSettings::default();
        let mut context = VideoLoadState::default();
        for (name, value) in [
            ("r_hdrOutput", "hdr10"),
            ("r_hdrPaperWhite", "250"),
            ("r_hdrPeak", "1400"),
        ] {
            let def = crate::app::commands::registry::cvar_definition(name).unwrap();
            def.config.load_video.unwrap()(&mut video, name, value, &mut context);
        }
        assert_eq!(video.hdr_output, 2);
        assert_eq!(video.hdr_paper_white, 250.0);
        assert_eq!(video.hdr_peak, 1400.0);
        assert!(
            !video.hdr && !video.tone_mapping,
            "display output must not rewrite independent scene settings"
        );
        let presentation = ClientPresentationSettings::default();
        let audio = AudioSettings::default();
        let state = ArchiveState {
            video: &video,
            presentation: &presentation,
            audio: &audio,
        };
        let mut loaded = ui::VideoSettings::default();
        for name in ["r_hdrOutput", "r_hdrPaperWhite", "r_hdrPeak"] {
            let def = crate::app::commands::registry::cvar_definition(name).unwrap();
            let value = def.config.archive.unwrap().1(&state);
            def.config.load_video.unwrap()(&mut loaded, name, &value, &mut context);
        }
        assert_eq!(
            (loaded.hdr_output, loaded.hdr_paper_white, loaded.hdr_peak),
            (2, 250.0, 1400.0)
        );
        let output = crate::app::commands::registry::cvar_definition("r_hdrOutput").unwrap();
        assert_eq!(output.runtime.entry.latch, LatchScope::VidRestart);
        output.config.load_video.unwrap()(&mut loaded, "r_hdrOutput", "invalid", &mut context);
        let white = crate::app::commands::registry::cvar_definition("r_hdrPaperWhite").unwrap();
        white.config.load_video.unwrap()(&mut loaded, "r_hdrPaperWhite", "NaN", &mut context);
        assert_eq!((loaded.hdr_output, loaded.hdr_paper_white), (2, 250.0));
    }
}
app_cvars! {
    APP_R_HDROUTPUT {
        runtime: CvarDef::new("r_hdrOutput", "0", "0..3", "Display output: SDR, HDR Auto, HDR10/PQ or scRGB. Apply Video Settings/vid_restart; unsupported HDR falls back to SDR.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::VidRestart,
            |app: &App| CvarValue::String(app.video.hdr_output.to_string()),
            |app, value| app.cvar_set_hdr_output("r_hdrOutput", &value.to_string()))
            .with_text_setter(App::cvar_set_hdr_output, Some(|_, value| mode(value).map(|m| m.to_string()))),
        config: CvarBinding {
            load_video: Some(|settings, _, value, _| { if let Ok(m) = mode(value) { settings.hdr_output = m; } }),
            apply_latched: Some(|settings, value| { if let Ok(m) = mode(value) { settings.hdr_output = m; } }),
            archive: Some((26000, |state| state.video.hdr_output.to_string())), ..CvarBinding::NONE
        },
        implementations: [impl App {
            pub(in crate::app) fn cvar_set_hdr_output(&mut self, _: &str, value: &str) -> Result<(), String> {
                self.video.hdr_output = mode(value)?;
                self.latched_console_cvars.remove("r_hdroutput");
                if self.video.hdr_output != 0 && self.video.gamma_method == crate::gamma::GammaMethod::Hardware {
                    self.set_gamma_method(crate::gamma::GammaMethod::Shader);
                }
                self.mark_config_dirty(); self.publish_ui(); Ok(())
            }
        }]
    }
    APP_R_HDRPAPERWHITE {
        runtime: CvarDef::new("r_hdrPaperWhite", "200", "80..500", "Native HDR luminance in nits; peak zero uses display metadata.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.hdr_paper_white.to_string()),
            |app, value| app.cvar_set_hdr_paper_white("r_hdrPaperWhite", &value.to_string())).with_text_setter(App::cvar_set_hdr_paper_white, None),
        config: CvarBinding {
            load_video: Some(|settings, _, value, _| { if let Ok(v) = value.parse::<f32>() { if v.is_finite() { settings.hdr_paper_white = v.clamp(80.0, 500.0); } } }),
            archive: Some((26001, |state| state.video.hdr_paper_white.to_string())), ..CvarBinding::NONE
        },
        implementations: [impl App {
            pub(in crate::app) fn cvar_set_hdr_paper_white(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.hdr_paper_white = Self::parse_cvar_finite(name, value)?.clamp(80.0, 500.0);
                self.sync_post_effects(); self.mark_config_dirty(); self.publish_ui(); Ok(())
            }
        }]
    }
    APP_R_HDRPEAK {
        runtime: CvarDef::new("r_hdrPeak", "0", "0 (auto), 80..10000", "Native HDR luminance in nits; peak zero uses display metadata.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.hdr_peak.to_string()),
            |app, value| app.cvar_set_hdr_peak("r_hdrPeak", &value.to_string())).with_text_setter(App::cvar_set_hdr_peak, None),
        config: CvarBinding {
            load_video: Some(|settings, _, value, _| { if let Ok(v) = value.parse::<f32>() { if v.is_finite() { settings.hdr_peak = v.clamp(0.0, 10000.0); } } }),
            archive: Some((26002, |state| state.video.hdr_peak.to_string())), ..CvarBinding::NONE
        },
        implementations: [impl App {
            pub(in crate::app) fn cvar_set_hdr_peak(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.hdr_peak = Self::parse_cvar_finite(name, value)?.clamp(0.0, 10000.0);
                self.sync_post_effects(); self.mark_config_dirty(); self.publish_ui(); Ok(())
            }
        }]
    }
}

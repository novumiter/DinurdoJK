//! Display encoding preference for unified r_hdr 0/1/2. No legacy HDR output cvars.
use super::cvar_prelude::*;

fn parse_hdr_style(value: &str) -> Result<u32, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "auto" => Ok(0),
        "1" | "hdr10" | "pq" => Ok(1),
        "2" | "scrgb" => Ok(2),
        _ => Err("r_hdrStyle: expected 0 Auto, 1 HDR10/PQ, or 2 scRGB".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_hdr_cvars_roundtrip_without_legacy_controls() {
        assert!(crate::app::commands::registry::cvar_definition("r_hdrOutput").is_none());
        assert!(crate::app::commands::registry::cvar_definition("r_hdrOutputStyle").is_none());
        let mut video = ui::VideoSettings::default();
        let mut context = VideoLoadState::default();
        for (name, value) in [("r_hdr", "2"), ("r_hdrStyle", "1"), ("r_hdrPaperWhite", "250")] {
            let def = crate::app::commands::registry::cvar_definition(name).unwrap();
            def.config.load_video.unwrap()(&mut video, name, value, &mut context);
        }
        assert_eq!((video.hdr_mode(), video.hdr_style, video.hdr_output), (2, 1, 2));
        assert_eq!(video.hdr_paper_white, 250.0);
        video.set_hdr_mode(1);
        assert_eq!((video.hdr_mode(), video.hdr_output), (1, 0));
        video.set_hdr_mode(0);
        assert_eq!((video.hdr_mode(), video.hdr_output), (0, 0));
        video.set_hdr_mode(2);
        assert_eq!(video.hdr_output, 2);
    }
}

app_cvars! {
    APP_R_HDRSTYLE {
        runtime: CvarDef::new("r_hdrStyle", "0", "0..2", "Full HDR output style: 0 Auto, 1 HDR10/PQ, 2 scRGB. Does not enable HDR output by itself.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.hdr_style.to_string()),
            |app, value| app.cvar_set_hdr_style("r_hdrStyle", &value.to_string()))
            .with_text_setter(App::cvar_set_hdr_style, Some(|_, value| parse_hdr_style(value).map(|m| m.to_string()))),
        config: CvarBinding {
            load_video: Some(|settings, _, value, _| { if let Ok(style) = parse_hdr_style(value) { settings.set_hdr_style(style); } }),
            archive: Some((26003, |state| state.video.hdr_style.to_string())), ..CvarBinding::NONE
        },
        implementations: [impl App {
            pub(in crate::app) fn cvar_set_hdr_style(&mut self, _: &str, value: &str) -> Result<(), String> {
                let style = parse_hdr_style(value)?;
                let output = style + 1;
                if self.video.hdr_mode() == 2 && self.hdr_modes_known
                    && self.video.renderer_backend == self.applied_renderer_backend
                    && self.video.gpu_adapter == self.applied_gpu_adapter
                    && self.hdr_available_modes & (1 << output) == 0
                {
                    return Err("Selected native HDR format is unavailable for this display/backend".to_owned());
                }
                self.video.set_hdr_style(style);
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

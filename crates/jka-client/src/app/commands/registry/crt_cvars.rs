//! Crt cvars: declarative runtime and config bindings.
use super::cvar_prelude::*;

app_cvars! {
    APP_R_CRT {
        runtime: CvarDef::new("r_crt", "", "0|1|2|3 (off|scene|scene+hud|everything)", "CRT presentation scope. Off submits no CRT pass; Scene keeps HUD/UI clean; Scene + HUD filters the gameplay HUD; Everything filters the final composited window.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None, |app| CvarValue::String(App::cvar_get_r_crt(app)), |app, value| App::cvar_set_r_crt(app, APP_R_CRT.entry.name, &value.to_string())).with_text_setter(App::cvar_set_r_crt, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_scope = CrtScope::from_config(value).unwrap_or(settings.crt_scope)
                    }
                }),
                archive: Some((115, |state| {
                    let settings = state.video;
                    format!("{}", settings.crt_scope.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crt(&self) -> String {
                    self.video.crt_scope.config_value().to_string()
                }
                fn cvar_set_r_crt(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let scope = CrtScope::from_config(value).ok_or_else(|| {
                            format!("{name}: expected off|scene|scene+hud|everything (0..3)")
                        })?;
                        self.set_crt_scope(scope);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTCORNER {
        runtime: CvarDef::new(
                "r_crtCorner",
                "",
                "0..11",
                "CRT corner rounding parameter.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtcorner(app)),
                |app, value| App::cvar_set_r_crtcorner(app, APP_R_CRTCORNER.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_crtcorner, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_corner = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(0.0, 11.0))
                            .unwrap_or(settings.crt_corner)
                    }
                }),
                archive: Some((124, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.crt_corner)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtcorner(&self) -> String {
                    format!("{:.3}", self.video.crt_corner)
                }
                fn cvar_set_r_crtcorner(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_crt_corner(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTCURVATURE {
        runtime: CvarDef::new(
                "r_crtCurvature",
                "",
                "0..0.25",
                "CRT glass curvature; zero is flat.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtcurvature(app)),
                |app, value| {
                    App::cvar_set_r_crtcurvature(app, APP_R_CRTCURVATURE.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_crtcurvature, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_curvature = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(0.0, 0.25))
                            .unwrap_or(settings.crt_curvature)
                    }
                }),
                archive: Some((123, |state| {
                    let settings = state.video;
                    format!("{:.4}", settings.crt_curvature)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtcurvature(&self) -> String {
                    format!("{:.4}", self.video.crt_curvature)
                }
                fn cvar_set_r_crtcurvature(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_crt_curvature(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTMASK {
        runtime: CvarDef::new(
                "r_crtMask",
                "",
                "0..3",
                "CRT phosphor mask: 0 off, 1 grille-lite, 2 grille, 3 shadow mask.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtmask(app)),
                |app, value| App::cvar_set_r_crtmask(app, APP_R_CRTMASK.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_crtmask, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_mask = CrtMask::from_config(value).unwrap_or(settings.crt_mask)
                    }
                }),
                archive: Some((119, |state| {
                    let settings = state.video;
                    format!("{}", settings.crt_mask.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtmask(&self) -> String {
                    self.video.crt_mask.config_value().to_string()
                }
                fn cvar_set_r_crtmask(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let mask = CrtMask::from_config(value).ok_or_else(|| {
                            format!("{name}: expected off|grille_lite|grille|shadow (0..3)")
                        })?;
                        self.set_crt_mask(mask);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTMASKINTENSITY {
        runtime: CvarDef::new(
                "r_crtMaskIntensity",
                "",
                "0..1",
                "CRT phosphor mask intensity.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtmaskintensity(app)),
                |app, value| {
                    App::cvar_set_r_crtmaskintensity(
                        app,
                        APP_R_CRTMASKINTENSITY.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_crtmaskintensity, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_mask_intensity = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.crt_mask_intensity)
                    }
                }),
                archive: Some((122, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.crt_mask_intensity)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtmaskintensity(&self) -> String {
                    format!("{:.3}", self.video.crt_mask_intensity)
                }
                fn cvar_set_r_crtmaskintensity(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_crt_mask_intensity(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTQUALITY {
        runtime: CvarDef::new(
                "r_crtQuality",
                "",
                "0|1 (fast|quality)",
                "crt-lottes-fast reconstruction path: 2-tap Fast or 8-tap Quality. Ignored by full Lottes.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtquality(app)),
                |app, value| {
                    App::cvar_set_r_crtquality(app, APP_R_CRTQUALITY.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_crtquality, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_quality =
                            CrtQuality::from_config(value).unwrap_or(settings.crt_quality)
                    }
                }),
                archive: Some((117, |state| {
                    let settings = state.video;
                    format!("{}", settings.crt_quality.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtquality(&self) -> String {
                    self.video.crt_quality.config_value().to_string()
                }
                fn cvar_set_r_crtquality(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let quality = CrtQuality::from_config(value)
                            .ok_or_else(|| format!("{name}: expected fast|quality (0|1)"))?;
                        self.set_crt_quality(quality);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTREFERENCE {
        runtime: CvarDef::new("r_crtReference", "", "480|600|720|native", "Virtual low-resolution source grid used by the CRT reconstruction without reducing render resolution.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None, |app| CvarValue::String(App::cvar_get_r_crtreference(app)), |app, value| App::cvar_set_r_crtreference(app, APP_R_CRTREFERENCE.entry.name, &value.to_string())).with_text_setter(App::cvar_set_r_crtreference, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_reference =
                            CrtReferenceResolution::from_config(value).unwrap_or(settings.crt_reference)
                    }
                }),
                archive: Some((118, |state| {
                    let settings = state.video;
                    format!("{}", settings.crt_reference.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtreference(&self) -> String {
                    self.video.crt_reference.config_value().to_string()
                }
                fn cvar_set_r_crtreference(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let reference = CrtReferenceResolution::from_config(value)
                            .ok_or_else(|| format!("{name}: expected 480|600|720|native"))?;
                        self.set_crt_reference(reference);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTSCANLINE {
        runtime: CvarDef::new(
                "r_crtScanline",
                "",
                "0..1",
                "CRT scanline thinness/strength.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtscanline(app)),
                |app, value| {
                    App::cvar_set_r_crtscanline(app, APP_R_CRTSCANLINE.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_crtscanline, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_scanline = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.crt_scanline)
                    }
                }),
                archive: Some((120, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.crt_scanline)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtscanline(&self) -> String {
                    format!("{:.3}", self.video.crt_scanline)
                }
                fn cvar_set_r_crtscanline(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_crt_scanline(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTSCOPE {
        runtime: CvarDef::new(
                "r_crtscope",
                "",
                "custom",
                "Controls r_crtscope.",
                CvarType::String,
                CvarFlags::NONE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtscope(app)),
                |app, value| App::cvar_set_r_crtscope(app, APP_R_CRTSCOPE.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_crtscope, None),
        config: CvarBinding {
                default: Some(|state| {
                    state.video.crt_scope.config_value().to_string()
                 }),
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_scope = CrtScope::from_config(value).unwrap_or(settings.crt_scope)
                    }
                }),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtscope(&self) -> String {
                    self.video.crt_scope.config_value().to_string()
                }
                fn cvar_set_r_crtscope(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let scope = CrtScope::from_config(value).ok_or_else(|| {
                            format!("{name}: expected off|scene|scene+hud|everything (0..3)")
                        })?;
                        self.set_crt_scope(scope);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTSHARPNESS {
        runtime: CvarDef::new(
                "r_crtSharpness",
                "",
                "1..6",
                "CRT horizontal Gaussian reconstruction sharpness.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_crtsharpness(app)),
                |app, value| {
                    App::cvar_set_r_crtsharpness(app, APP_R_CRTSHARPNESS.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_crtsharpness, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_sharpness = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(1.0, 6.0))
                            .unwrap_or(settings.crt_sharpness)
                    }
                }),
                archive: Some((121, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.crt_sharpness)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtsharpness(&self) -> String {
                    format!("{:.3}", self.video.crt_sharpness)
                }
                fn cvar_set_r_crtsharpness(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_crt_sharpness(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CRTSTYLE {
        runtime: CvarDef::new("r_crtStyle", "", "fast|lottes", "CRT algorithm: performance-oriented crt-lottes-fast or the original full crt-lottes filter.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None, |app| CvarValue::String(App::cvar_get_r_crtstyle(app)), |app, value| App::cvar_set_r_crtstyle(app, APP_R_CRTSTYLE.entry.name, &value.to_string())).with_text_setter(App::cvar_set_r_crtstyle, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.crt_style = CrtStyle::from_config(value).unwrap_or(settings.crt_style)
                    }
                }),
                archive: Some((116, |state| {
                    let settings = state.video;
                    format!("{}", settings.crt_style.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_crtstyle(&self) -> String {
                    self.video.crt_style.config_value().to_owned()
                }
                fn cvar_set_r_crtstyle(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let style = CrtStyle::from_config(value)
                            .ok_or_else(|| format!("{name}: expected fast|lottes (0|1)"))?;
                        self.set_crt_style(style);
                    }
                    Ok(())
                }
            }
        ]
    }
}

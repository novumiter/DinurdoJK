//! Ocean cvars: declarative runtime and config bindings.
use super::cvar_prelude::*;

app_cvars! {
    APP_R_OCEAN {
        runtime: CvarDef::new(
                "r_ocean",
                "",
                "0|1",
                "GPU ocean. Enabling on a world loaded without ocean requires applying video settings.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_ocean(app)),
                |app, value| App::cvar_set_r_ocean(app, APP_R_OCEAN.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_ocean, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean = parse_bool(value).unwrap_or(settings.ocean)
                }),
                archive: Some((93, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.ocean))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_ocean(&self) -> String {
                    bool_value(self.video.ocean)
                }
                fn cvar_set_r_ocean(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean = Self::parse_cvar_bool(name, value)?;
                        // OFF releases the FFT resources immediately, and ON can be
                        // restored live only when the loaded world already carries its
                        // promoted ocean clipmap.
                        if !self.video.ocean || self.applied_ocean {
                            self.render_command(RenderCommand::SetOceanEnabled(self.video.ocean));
                        }
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANAUTHORING {
        runtime: CvarDef::new(
                "r_oceanAuthoring",
                "",
                "custom",
                "Controls r_oceanAuthoring.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanauthoring(app)),
                |app, value| {
                    App::cvar_set_r_oceanauthoring(app, APP_R_OCEANAUTHORING.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanauthoring, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    let v: Vec<_> = value.split_whitespace().collect();
                    if v.len() == 11 {
                        let floats: Option<Vec<f32>> = v[..10].iter().map(|s| s.parse().ok()).collect();
                        if let (Some(f), Ok(seed)) = (floats, v[10].parse()) {
                            settings.ocean_settings.authored = crate::ocean::OceanAuthoring {
                                amplitude: f[0],
                                wavelength: f[1],
                                speed: f[2],
                                direction: f[3],
                                steepness: f[4],
                                slosh: f[5],
                                wind_chop: f[6],
                                foam: f[7],
                                foam_lifetime: f[8],
                                spray: f[9],
                                seed,
                            }
                            .sanitize();
                        }
                    }
                }),
                archive: Some((10419, |state| {
                    let settings = state.video;
                    let a = settings.ocean_settings.authored;
                    format!(
                        "{} {} {} {} {} {} {} {} {} {} {}",
                        a.amplitude,
                        a.wavelength,
                        a.speed,
                        a.direction,
                        a.steepness,
                        a.slosh,
                        a.wind_chop,
                        a.foam,
                        a.foam_lifetime,
                        a.spray,
                        a.seed
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanauthoring(&self) -> String {
                    {
                        let a = self.video.ocean_settings.authored;
                        format!(
                            "{} {} {} {} {} {} {} {} {} {} {}",
                            a.amplitude,
                            a.wavelength,
                            a.speed,
                            a.direction,
                            a.steepness,
                            a.slosh,
                            a.wind_chop,
                            a.foam,
                            a.foam_lifetime,
                            a.spray,
                            a.seed,
                        )
                    }
                }
                fn cvar_set_r_oceanauthoring(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let words: Vec<&str> = value.split_whitespace().collect();
                        let expected = || format!("{name}: expected ten numbers and a seed");
                        if words.len() != 11 {
                            return Err(expected());
                        }
                        let mut f = [0.0_f32; 10];
                        for (slot, word) in f.iter_mut().zip(&words) {
                            *slot = word
                                .parse::<f32>()
                                .ok()
                                .filter(|number| number.is_finite())
                                .ok_or_else(expected)?;
                        }
                        let seed = words[10].parse().map_err(|_| expected())?;
                        self.video.ocean_settings.authored = crate::ocean::OceanAuthoring {
                            amplitude: f[0],
                            wavelength: f[1],
                            speed: f[2],
                            direction: f[3],
                            steepness: f[4],
                            slosh: f[5],
                            wind_chop: f[6],
                            foam: f[7],
                            foam_lifetime: f[8],
                            spray: f[9],
                            seed,
                        }
                        .sanitize();
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANCASCADE1 {
        runtime: CvarDef::new(
                "r_oceanCascade1",
                "",
                "custom",
                "Controls r_oceanCascade1.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceancascade1(app)),
                |app, value| {
                    App::cvar_set_r_oceancascade1(app, APP_R_OCEANCASCADE1.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceancascade1, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.ocean_settings.cascades[0] =
                            parse_ocean_cascade(value, settings.ocean_settings.cascades[0])
                    }
                }),
                archive: Some((103, |state| {
                    let settings = state.video;
                    format!(
                        "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                        settings.ocean_settings.cascades[0].tile_length[0],
                        settings.ocean_settings.cascades[0].tile_length[1],
                        settings.ocean_settings.cascades[0].displacement_scale,
                        settings.ocean_settings.cascades[0].normal_scale,
                        settings.ocean_settings.cascades[0].wind_speed,
                        settings.ocean_settings.cascades[0].wind_direction,
                        settings.ocean_settings.cascades[0].fetch_length,
                        settings.ocean_settings.cascades[0].swell,
                        settings.ocean_settings.cascades[0].spread,
                        settings.ocean_settings.cascades[0].detail,
                        settings.ocean_settings.cascades[0].whitecap,
                        settings.ocean_settings.cascades[0].foam_amount
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceancascade1(&self) -> String {
                    let name = "r_oceancascade1";
                    {
                        let digit = name.as_bytes()[name.len() - 1];
                        let c = self.video.ocean_settings.cascades[usize::from(digit - b'1')];
                        format!(
                            "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                            c.tile_length[0],
                            c.tile_length[1],
                            c.displacement_scale,
                            c.normal_scale,
                            c.wind_speed,
                            c.wind_direction,
                            c.fetch_length,
                            c.swell,
                            c.spread,
                            c.detail,
                            c.whitecap,
                            c.foam_amount,
                        )
                    }
                }
                fn cvar_set_r_oceancascade1(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_oceancascade1".to_owned();
                    {
                        let v: [f32; 12] = Self::parse_cvar_floats(name, value)?;
                        let index = usize::from(lower.as_bytes()[lower.len() - 1] - b'1');
                        self.video.ocean_settings.cascades[index] = crate::ocean::CascadeSettings {
                            tile_length: [v[0], v[1]],
                            displacement_scale: v[2],
                            normal_scale: v[3],
                            wind_speed: v[4],
                            wind_direction: v[5],
                            fetch_length: v[6],
                            swell: v[7],
                            spread: v[8],
                            detail: v[9],
                            whitecap: v[10],
                            foam_amount: v[11],
                        };
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANCASCADE2 {
        runtime: CvarDef::new(
                "r_oceanCascade2",
                "",
                "custom",
                "Controls r_oceanCascade2.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceancascade2(app)),
                |app, value| {
                    App::cvar_set_r_oceancascade2(app, APP_R_OCEANCASCADE2.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceancascade2, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.ocean_settings.cascades[1] =
                            parse_ocean_cascade(value, settings.ocean_settings.cascades[1])
                    }
                }),
                archive: Some((104, |state| {
                    let settings = state.video;
                    format!(
                        "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                        settings.ocean_settings.cascades[1].tile_length[0],
                        settings.ocean_settings.cascades[1].tile_length[1],
                        settings.ocean_settings.cascades[1].displacement_scale,
                        settings.ocean_settings.cascades[1].normal_scale,
                        settings.ocean_settings.cascades[1].wind_speed,
                        settings.ocean_settings.cascades[1].wind_direction,
                        settings.ocean_settings.cascades[1].fetch_length,
                        settings.ocean_settings.cascades[1].swell,
                        settings.ocean_settings.cascades[1].spread,
                        settings.ocean_settings.cascades[1].detail,
                        settings.ocean_settings.cascades[1].whitecap,
                        settings.ocean_settings.cascades[1].foam_amount
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceancascade2(&self) -> String {
                    let name = "r_oceancascade2";
                    {
                        let digit = name.as_bytes()[name.len() - 1];
                        let c = self.video.ocean_settings.cascades[usize::from(digit - b'1')];
                        format!(
                            "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                            c.tile_length[0],
                            c.tile_length[1],
                            c.displacement_scale,
                            c.normal_scale,
                            c.wind_speed,
                            c.wind_direction,
                            c.fetch_length,
                            c.swell,
                            c.spread,
                            c.detail,
                            c.whitecap,
                            c.foam_amount,
                        )
                    }
                }
                fn cvar_set_r_oceancascade2(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_oceancascade2".to_owned();
                    {
                        let v: [f32; 12] = Self::parse_cvar_floats(name, value)?;
                        let index = usize::from(lower.as_bytes()[lower.len() - 1] - b'1');
                        self.video.ocean_settings.cascades[index] = crate::ocean::CascadeSettings {
                            tile_length: [v[0], v[1]],
                            displacement_scale: v[2],
                            normal_scale: v[3],
                            wind_speed: v[4],
                            wind_direction: v[5],
                            fetch_length: v[6],
                            swell: v[7],
                            spread: v[8],
                            detail: v[9],
                            whitecap: v[10],
                            foam_amount: v[11],
                        };
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANCASCADE3 {
        runtime: CvarDef::new(
                "r_oceanCascade3",
                "",
                "custom",
                "Controls r_oceanCascade3.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceancascade3(app)),
                |app, value| {
                    App::cvar_set_r_oceancascade3(app, APP_R_OCEANCASCADE3.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceancascade3, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.ocean_settings.cascades[2] =
                            parse_ocean_cascade(value, settings.ocean_settings.cascades[2])
                    }
                }),
                archive: Some((105, |state| {
                    let settings = state.video;
                    format!(
                        "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                        settings.ocean_settings.cascades[2].tile_length[0],
                        settings.ocean_settings.cascades[2].tile_length[1],
                        settings.ocean_settings.cascades[2].displacement_scale,
                        settings.ocean_settings.cascades[2].normal_scale,
                        settings.ocean_settings.cascades[2].wind_speed,
                        settings.ocean_settings.cascades[2].wind_direction,
                        settings.ocean_settings.cascades[2].fetch_length,
                        settings.ocean_settings.cascades[2].swell,
                        settings.ocean_settings.cascades[2].spread,
                        settings.ocean_settings.cascades[2].detail,
                        settings.ocean_settings.cascades[2].whitecap,
                        settings.ocean_settings.cascades[2].foam_amount
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceancascade3(&self) -> String {
                    let name = "r_oceancascade3";
                    {
                        let digit = name.as_bytes()[name.len() - 1];
                        let c = self.video.ocean_settings.cascades[usize::from(digit - b'1')];
                        format!(
                            "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                            c.tile_length[0],
                            c.tile_length[1],
                            c.displacement_scale,
                            c.normal_scale,
                            c.wind_speed,
                            c.wind_direction,
                            c.fetch_length,
                            c.swell,
                            c.spread,
                            c.detail,
                            c.whitecap,
                            c.foam_amount,
                        )
                    }
                }
                fn cvar_set_r_oceancascade3(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_oceancascade3".to_owned();
                    {
                        let v: [f32; 12] = Self::parse_cvar_floats(name, value)?;
                        let index = usize::from(lower.as_bytes()[lower.len() - 1] - b'1');
                        self.video.ocean_settings.cascades[index] = crate::ocean::CascadeSettings {
                            tile_length: [v[0], v[1]],
                            displacement_scale: v[2],
                            normal_scale: v[3],
                            wind_speed: v[4],
                            wind_direction: v[5],
                            fetch_length: v[6],
                            swell: v[7],
                            spread: v[8],
                            detail: v[9],
                            whitecap: v[10],
                            foam_amount: v[11],
                        };
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANCAUSTICS {
        runtime: CvarDef::new(
                "r_oceanCaustics",
                "",
                "0..8",
                "Experimental FFT curvature caustics; not shadow-aware yet.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceancaustics(app)),
                |app, value| {
                    App::cvar_set_r_oceancaustics(app, APP_R_OCEANCAUSTICS.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceancaustics, None),
        config: CvarBinding {
                load_video: Some(|settings, name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            let o = &mut settings.ocean_settings.optics;
                            match name.to_ascii_lowercase().as_str() {
                                "r_oceanfogdistance" => o.fog_distance = v,
                                "r_oceantransparency" => o.transparency = v,
                                "r_oceandepthdarkening" => o.depth_darkening = v,
                                "r_oceanrefraction" => o.refraction = v,
                                "r_oceancaustics" => o.caustics = v,
                                _ => o.underwater_cull = v,
                            }
                            *o = o.sanitize();
                        }
                    }
                }),
                archive: Some((10417, |state| {
                    let settings = state.video;
                    let o = settings.ocean_settings.optics;
                    format!("{}", o.caustics)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceancaustics(&self) -> String {
                    self.video.ocean_settings.optics.caustics.to_string()
                }
                fn cvar_set_r_oceancaustics(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let v = value
                            .parse::<f32>()
                            .map_err(|_| "Expected a finite number".to_owned())?;
                        if !v.is_finite() {
                            return Err("Expected a finite number".to_owned());
                        }
                        let o = &mut self.video.ocean_settings.optics;
                        match name.to_ascii_lowercase().as_str() {
                            "r_oceanfogdistance" => o.fog_distance = v,
                            "r_oceantransparency" => o.transparency = v,
                            "r_oceandepthdarkening" => o.depth_darkening = v,
                            "r_oceanrefraction" => o.refraction = v,
                            "r_oceancaustics" => o.caustics = v,
                            _ => o.underwater_cull = v,
                        }
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANDEPTHDARKENING {
        runtime: CvarDef::new(
                "r_oceanDepthDarkening",
                "",
                "0.01..32",
                "Underwater sunlight penetration multiplier.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceandepthdarkening(app)),
                |app, value| {
                    App::cvar_set_r_oceandepthdarkening(
                        app,
                        APP_R_OCEANDEPTHDARKENING.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceandepthdarkening, None),
        config: CvarBinding {
                load_video: Some(|settings, name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            let o = &mut settings.ocean_settings.optics;
                            match name.to_ascii_lowercase().as_str() {
                                "r_oceanfogdistance" => o.fog_distance = v,
                                "r_oceantransparency" => o.transparency = v,
                                "r_oceandepthdarkening" => o.depth_darkening = v,
                                "r_oceanrefraction" => o.refraction = v,
                                "r_oceancaustics" => o.caustics = v,
                                _ => o.underwater_cull = v,
                            }
                            *o = o.sanitize();
                        }
                    }
                }),
                archive: Some((10415, |state| {
                    let settings = state.video;
                    let o = settings.ocean_settings.optics;
                    format!("{}", o.depth_darkening)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceandepthdarkening(&self) -> String {
                    self.video.ocean_settings.optics.depth_darkening.to_string()
                }
                fn cvar_set_r_oceandepthdarkening(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    {
                        let v = value
                            .parse::<f32>()
                            .map_err(|_| "Expected a finite number".to_owned())?;
                        if !v.is_finite() {
                            return Err("Expected a finite number".to_owned());
                        }
                        let o = &mut self.video.ocean_settings.optics;
                        match name.to_ascii_lowercase().as_str() {
                            "r_oceanfogdistance" => o.fog_distance = v,
                            "r_oceantransparency" => o.transparency = v,
                            "r_oceandepthdarkening" => o.depth_darkening = v,
                            "r_oceanrefraction" => o.refraction = v,
                            "r_oceancaustics" => o.caustics = v,
                            _ => o.underwater_cull = v,
                        }
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANFOAMCOLOR {
        runtime: CvarDef::new(
                "r_oceanFoamColor",
                "",
                "custom",
                "Controls r_oceanFoamColor.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanfoamcolor(app)),
                |app, value| {
                    App::cvar_set_r_oceanfoamcolor(app, APP_R_OCEANFOAMCOLOR.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanfoamcolor, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.foam_color =
                        parse_vec3(value).unwrap_or(settings.ocean_settings.foam_color);
                }),
                archive: Some((102, |state| {
                    let settings = state.video;
                    format!(
                        "{:.4} {:.4} {:.4}",
                        settings.ocean_settings.foam_color[0],
                        settings.ocean_settings.foam_color[1],
                        settings.ocean_settings.foam_color[2]
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanfoamcolor(&self) -> String {
                    {
                        let c = self.video.ocean_settings.foam_color;
                        format!("{:.4} {:.4} {:.4}", c[0], c[1], c[2])
                    }
                }
                fn cvar_set_r_oceanfoamcolor(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.foam_color = Self::parse_cvar_floats(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANFOGCOLOR {
        runtime: CvarDef::new(
                "r_oceanFogColor",
                "",
                "R G B (0..1)",
                "Water scattering tint.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanfogcolor(app)),
                |app, value| {
                    App::cvar_set_r_oceanfogcolor(app, APP_R_OCEANFOGCOLOR.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanfogcolor, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.ocean_settings.optics.fog_color =
                            parse_vec3(value).unwrap_or(settings.ocean_settings.optics.fog_color)
                    }
                }),
                archive: Some((10412, |state| {
                    let settings = state.video;
                    let o = settings.ocean_settings.optics;
                    format!("{} {} {}", o.fog_color[0], o.fog_color[1], o.fog_color[2])
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanfogcolor(&self) -> String {
                    {
                        let c = self.video.ocean_settings.optics.fog_color;
                        format!("{} {} {}", c[0], c[1], c[2])
                    }
                }
                fn cvar_set_r_oceanfogcolor(&mut self, _name: &str, value: &str) -> Result<(), String> {
                    {
                        let values = value
                            .split_whitespace()
                            .map(str::parse::<f32>)
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|_| "Expected three finite color components".to_owned())?;
                        if values.len() != 3 || !values.iter().all(|v| v.is_finite()) {
                            return Err("Expected three finite color components".to_owned());
                        }
                        self.video.ocean_settings.optics.fog_color = [values[0], values[1], values[2]];
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANFOGDISTANCE {
        runtime: CvarDef::new(
                "r_oceanFogDistance",
                "",
                "1..65536",
                "Base water absorption distance in game units.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanfogdistance(app)),
                |app, value| {
                    App::cvar_set_r_oceanfogdistance(
                        app,
                        APP_R_OCEANFOGDISTANCE.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceanfogdistance, None),
        config: CvarBinding {
                load_video: Some(|settings, name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            let o = &mut settings.ocean_settings.optics;
                            match name.to_ascii_lowercase().as_str() {
                                "r_oceanfogdistance" => o.fog_distance = v,
                                "r_oceantransparency" => o.transparency = v,
                                "r_oceandepthdarkening" => o.depth_darkening = v,
                                "r_oceanrefraction" => o.refraction = v,
                                "r_oceancaustics" => o.caustics = v,
                                _ => o.underwater_cull = v,
                            }
                            *o = o.sanitize();
                        }
                    }
                }),
                archive: Some((10413, |state| {
                    let settings = state.video;
                    let o = settings.ocean_settings.optics;
                    format!("{}", o.fog_distance)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanfogdistance(&self) -> String {
                    self.video.ocean_settings.optics.fog_distance.to_string()
                }
                fn cvar_set_r_oceanfogdistance(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let v = value
                            .parse::<f32>()
                            .map_err(|_| "Expected a finite number".to_owned())?;
                        if !v.is_finite() {
                            return Err("Expected a finite number".to_owned());
                        }
                        let o = &mut self.video.ocean_settings.optics;
                        match name.to_ascii_lowercase().as_str() {
                            "r_oceanfogdistance" => o.fog_distance = v,
                            "r_oceantransparency" => o.transparency = v,
                            "r_oceandepthdarkening" => o.depth_darkening = v,
                            "r_oceanrefraction" => o.refraction = v,
                            "r_oceancaustics" => o.caustics = v,
                            _ => o.underwater_cull = v,
                        }
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANMAPSIZE {
        runtime: CvarDef::new(
                "r_oceanMapSize",
                "",
                "custom",
                "Controls r_oceanMapSize.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanmapsize(app)),
                |app, value| {
                    App::cvar_set_r_oceanmapsize(app, APP_R_OCEANMAPSIZE.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanmapsize, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(v) = value.parse::<u32>() {
                        settings.ocean_settings.map_size = v;
                    }
                }),
                archive: Some((94, |state| {
                    let settings = state.video;
                    format!("{}", settings.ocean_settings.map_size)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanmapsize(&self) -> String {
                    self.video.ocean_settings.map_size.to_string()
                }
                fn cvar_set_r_oceanmapsize(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let requested = value
                            .trim()
                            .parse::<u32>()
                            .map_err(|_| format!("{name}: expected 128|256|512|1024"))?;
                        if !matches!(requested, 128 | 256 | 512 | 1024) {
                            return Err(format!("{name}: expected 128|256|512|1024"));
                        }
                        self.video.ocean_settings.map_size = requested;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANMESHQUALITY {
        runtime: CvarDef::new(
                "r_oceanMeshQuality",
                "",
                "custom",
                "Controls r_oceanMeshQuality.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanmeshquality(app)),
                |app, value| {
                    App::cvar_set_r_oceanmeshquality(
                        app,
                        APP_R_OCEANMESHQUALITY.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceanmeshquality, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(v) = value.parse::<u8>() {
                        settings.ocean_settings.mesh_quality = v;
                    }
                }),
                archive: Some((95, |state| {
                    let settings = state.video;
                    format!("{}", settings.ocean_settings.mesh_quality)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanmeshquality(&self) -> String {
                    self.video.ocean_settings.mesh_quality.to_string()
                }
                fn cvar_set_r_oceanmeshquality(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.mesh_quality =
                            match value.trim().to_ascii_lowercase().as_str() {
                                "0" | "low" => 0,
                                "1" | "high" => 1,
                                _ => return Err(format!("{name}: expected 0|1 or low|high")),
                            };
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANNORMALSTRENGTH {
        runtime: CvarDef::new(
                "r_oceanNormalStrength",
                "",
                "custom",
                "Controls r_oceanNormalStrength.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceannormalstrength(app)),
                |app, value| {
                    App::cvar_set_r_oceannormalstrength(
                        app,
                        APP_R_OCEANNORMALSTRENGTH.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceannormalstrength, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            settings.ocean_settings.normal_strength = v;
                        }
                    }
                }),
                archive: Some((98, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.ocean_settings.normal_strength)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceannormalstrength(&self) -> String {
                    format!("{:.3}", self.video.ocean_settings.normal_strength)
                }
                fn cvar_set_r_oceannormalstrength(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    {
                        self.video.ocean_settings.normal_strength = Self::parse_cvar_finite(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANREFRACTION {
        runtime: CvarDef::new(
                "r_oceanRefraction",
                "",
                "0..0.15",
                "Wave distortion of transmitted scene color.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanrefraction(app)),
                |app, value| {
                    App::cvar_set_r_oceanrefraction(
                        app,
                        APP_R_OCEANREFRACTION.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceanrefraction, None),
        config: CvarBinding {
                load_video: Some(|settings, name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            let o = &mut settings.ocean_settings.optics;
                            match name.to_ascii_lowercase().as_str() {
                                "r_oceanfogdistance" => o.fog_distance = v,
                                "r_oceantransparency" => o.transparency = v,
                                "r_oceandepthdarkening" => o.depth_darkening = v,
                                "r_oceanrefraction" => o.refraction = v,
                                "r_oceancaustics" => o.caustics = v,
                                _ => o.underwater_cull = v,
                            }
                            *o = o.sanitize();
                        }
                    }
                }),
                archive: Some((10416, |state| {
                    let settings = state.video;
                    let o = settings.ocean_settings.optics;
                    format!("{}", o.refraction)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanrefraction(&self) -> String {
                    self.video.ocean_settings.optics.refraction.to_string()
                }
                fn cvar_set_r_oceanrefraction(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let v = value
                            .parse::<f32>()
                            .map_err(|_| "Expected a finite number".to_owned())?;
                        if !v.is_finite() {
                            return Err("Expected a finite number".to_owned());
                        }
                        let o = &mut self.video.ocean_settings.optics;
                        match name.to_ascii_lowercase().as_str() {
                            "r_oceanfogdistance" => o.fog_distance = v,
                            "r_oceantransparency" => o.transparency = v,
                            "r_oceandepthdarkening" => o.depth_darkening = v,
                            "r_oceanrefraction" => o.refraction = v,
                            "r_oceancaustics" => o.caustics = v,
                            _ => o.underwater_cull = v,
                        }
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANROUGHNESS {
        runtime: CvarDef::new(
                "r_oceanRoughness",
                "",
                "custom",
                "Controls r_oceanRoughness.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanroughness(app)),
                |app, value| {
                    App::cvar_set_r_oceanroughness(app, APP_R_OCEANROUGHNESS.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanroughness, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            settings.ocean_settings.roughness = v;
                        }
                    }
                }),
                archive: Some((97, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.ocean_settings.roughness)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanroughness(&self) -> String {
                    format!("{:.3}", self.video.ocean_settings.roughness)
                }
                fn cvar_set_r_oceanroughness(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.roughness = Self::parse_cvar_finite(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANSEASPRAY {
        runtime: CvarDef::new(
                "r_oceanSeaSpray",
                "",
                "custom",
                "Controls r_oceanSeaSpray.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanseaspray(app)),
                |app, value| {
                    App::cvar_set_r_oceanseaspray(app, APP_R_OCEANSEASPRAY.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanseaspray, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.sea_spray =
                        parse_bool(value).unwrap_or(settings.ocean_settings.sea_spray);
                }),
                archive: Some((99, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.ocean_settings.sea_spray))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanseaspray(&self) -> String {
                    bool_value(self.video.ocean_settings.sea_spray)
                }
                fn cvar_set_r_oceanseaspray(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.sea_spray = Self::parse_cvar_bool(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANTRANSPARENCY {
        runtime: CvarDef::new(
                "r_oceanTransparency",
                "",
                "0.1..16",
                "Water absorption-distance multiplier.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceantransparency(app)),
                |app, value| {
                    App::cvar_set_r_oceantransparency(
                        app,
                        APP_R_OCEANTRANSPARENCY.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceantransparency, None),
        config: CvarBinding {
                load_video: Some(|settings, name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            let o = &mut settings.ocean_settings.optics;
                            match name.to_ascii_lowercase().as_str() {
                                "r_oceanfogdistance" => o.fog_distance = v,
                                "r_oceantransparency" => o.transparency = v,
                                "r_oceandepthdarkening" => o.depth_darkening = v,
                                "r_oceanrefraction" => o.refraction = v,
                                "r_oceancaustics" => o.caustics = v,
                                _ => o.underwater_cull = v,
                            }
                            *o = o.sanitize();
                        }
                    }
                }),
                archive: Some((10414, |state| {
                    let settings = state.video;
                    let o = settings.ocean_settings.optics;
                    format!("{}", o.transparency)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceantransparency(&self) -> String {
                    self.video.ocean_settings.optics.transparency.to_string()
                }
                fn cvar_set_r_oceantransparency(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let v = value
                            .parse::<f32>()
                            .map_err(|_| "Expected a finite number".to_owned())?;
                        if !v.is_finite() {
                            return Err("Expected a finite number".to_owned());
                        }
                        let o = &mut self.video.ocean_settings.optics;
                        match name.to_ascii_lowercase().as_str() {
                            "r_oceanfogdistance" => o.fog_distance = v,
                            "r_oceantransparency" => o.transparency = v,
                            "r_oceandepthdarkening" => o.depth_darkening = v,
                            "r_oceanrefraction" => o.refraction = v,
                            "r_oceancaustics" => o.caustics = v,
                            _ => o.underwater_cull = v,
                        }
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANUNDERWATERCULL {
        runtime: CvarDef::new(
                "r_oceanUnderwaterCull",
                "",
                "0..32",
                "Separate submerged BSP cull distance, as an absorption-distance multiple. Zero disables.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanunderwatercull(app)),
                |app, value| {
                    App::cvar_set_r_oceanunderwatercull(
                        app,
                        APP_R_OCEANUNDERWATERCULL.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceanunderwatercull, None),
        config: CvarBinding {
                load_video: Some(|settings, name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            let o = &mut settings.ocean_settings.optics;
                            match name.to_ascii_lowercase().as_str() {
                                "r_oceanfogdistance" => o.fog_distance = v,
                                "r_oceantransparency" => o.transparency = v,
                                "r_oceandepthdarkening" => o.depth_darkening = v,
                                "r_oceanrefraction" => o.refraction = v,
                                "r_oceancaustics" => o.caustics = v,
                                _ => o.underwater_cull = v,
                            }
                            *o = o.sanitize();
                        }
                    }
                }),
                archive: Some((10418, |state| {
                    let settings = state.video;
                    let o = settings.ocean_settings.optics;
                    format!("{}", o.underwater_cull)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanunderwatercull(&self) -> String {
                    self.video.ocean_settings.optics.underwater_cull.to_string()
                }
                fn cvar_set_r_oceanunderwatercull(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    {
                        let v = value
                            .parse::<f32>()
                            .map_err(|_| "Expected a finite number".to_owned())?;
                        if !v.is_finite() {
                            return Err("Expected a finite number".to_owned());
                        }
                        let o = &mut self.video.ocean_settings.optics;
                        match name.to_ascii_lowercase().as_str() {
                            "r_oceanfogdistance" => o.fog_distance = v,
                            "r_oceantransparency" => o.transparency = v,
                            "r_oceandepthdarkening" => o.depth_darkening = v,
                            "r_oceanrefraction" => o.refraction = v,
                            "r_oceancaustics" => o.caustics = v,
                            _ => o.underwater_cull = v,
                        }
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANUPDATES {
        runtime: CvarDef::new(
                "r_oceanUpdates",
                "",
                "custom",
                "Controls r_oceanUpdates.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanupdates(app)),
                |app, value| {
                    App::cvar_set_r_oceanupdates(app, APP_R_OCEANUPDATES.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanupdates, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(v) = value.parse::<f32>() {
                        if v.is_finite() {
                            settings.ocean_settings.updates_per_second = v;
                        }
                    }
                }),
                archive: Some((96, |state| {
                    let settings = state.video;
                    format!("{:.2}", settings.ocean_settings.updates_per_second)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanupdates(&self) -> String {
                    format!("{:.3}", self.video.ocean_settings.updates_per_second)
                }
                fn cvar_set_r_oceanupdates(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.updates_per_second =
                            Self::parse_cvar_finite(name, value)?.clamp(0.0, 60.0);
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANWATERCOLOR {
        runtime: CvarDef::new(
                "r_oceanWaterColor",
                "",
                "custom",
                "Controls r_oceanWaterColor.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanwatercolor(app)),
                |app, value| {
                    App::cvar_set_r_oceanwatercolor(
                        app,
                        APP_R_OCEANWATERCOLOR.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_oceanwatercolor, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.water_color =
                        parse_vec3(value).unwrap_or(settings.ocean_settings.water_color);
                }),
                archive: Some((101, |state| {
                    let settings = state.video;
                    format!(
                        "{:.4} {:.4} {:.4}",
                        settings.ocean_settings.water_color[0],
                        settings.ocean_settings.water_color[1],
                        settings.ocean_settings.water_color[2]
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanwatercolor(&self) -> String {
                    {
                        let c = self.video.ocean_settings.water_color;
                        format!("{:.4} {:.4} {:.4}", c[0], c[1], c[2])
                    }
                }
                fn cvar_set_r_oceanwatercolor(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.water_color = Self::parse_cvar_floats(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANWINDFOAM {
        runtime: CvarDef::new(
                "r_oceanWindFoam",
                "",
                "custom",
                "Controls r_oceanWindFoam.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanwindfoam(app)),
                |app, value| {
                    App::cvar_set_r_oceanwindfoam(app, APP_R_OCEANWINDFOAM.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanwindfoam, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.wind_foam_streaks =
                        parse_bool(value).unwrap_or(settings.ocean_settings.wind_foam_streaks);
                }),
                archive: Some((100, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.ocean_settings.wind_foam_streaks))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanwindfoam(&self) -> String {
                    bool_value(self.video.ocean_settings.wind_foam_streaks)
                }
                fn cvar_set_r_oceanwindfoam(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.wind_foam_streaks = Self::parse_cvar_bool(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }
}

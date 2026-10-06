//! Weather cvars: declarative runtime and config bindings.
use super::cvar_prelude::*;

app_cvars! {
    APP_R_DRAWFOG {
        runtime: CvarDef::new("r_drawfog", "", "0|1|2|3", "Fog rendering: 0 off, 1 legacy (authored fog redrawn in post), 2 legacy (per-surface EXP2 fog, JKA default), 3 volumetric.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None, |app| CvarValue::String(App::cvar_get_r_drawfog(app)), |app, value| App::cvar_set_r_drawfog(app, APP_R_DRAWFOG.entry.name, &value.to_string())).with_text_setter(App::cvar_set_r_drawfog, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.fog_mode = FogMode::from_drawfog(value).unwrap_or(settings.fog_mode);
                }),
                archive: Some((54, |state| {
                    let settings = state.video;
                    format!("{}", settings.fog_mode.drawfog_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_drawfog(&self) -> String {
                    self.video.fog_mode.drawfog_value().to_string()
                }
                fn cvar_set_r_drawfog(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.fog_mode = FogMode::from_drawfog(value)
                            .ok_or_else(|| format!("{name}: expected 0|1|2|3"))?;
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_FOGSTRENGTH {
        runtime: CvarDef::new(
                "r_fogStrength",
                "",
                "0..10",
                "Fog density/strength multiplier.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_fogstrength(app)),
                |app, value| {
                    App::cvar_set_r_fogstrength(app, APP_R_FOGSTRENGTH.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_fogstrength, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(strength) = value.parse::<f32>() {
                        if strength.is_finite() {
                            settings.fog_strength = strength.clamp(0.0, MAX_FOG_STRENGTH);
                        }
                    }
                }),
                archive: Some((55, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.fog_strength)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_fogstrength(&self) -> String {
                    format!("{:.3}", self.video.fog_strength)
                }
                fn cvar_set_r_fogstrength(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let strength = value
                            .trim()
                            .parse::<f32>()
                            .map_err(|_| format!("{name}: expected a number"))?;
                        self.set_fog_strength(strength);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_GRASS {
        runtime: CvarDef::new(
                "r_grass",
                "",
                "0..1",
                "Procedural per-blade grass on surfaceSprites vegetation.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_grass(app)),
                |app, value| App::cvar_set_r_grass(app, APP_R_GRASS.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_grass, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.grass = parse_bool(value).unwrap_or(settings.grass)
                }),
                archive: Some((92, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.grass))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_grass(&self) -> String {
                    bool_value(self.video.grass)
                }
                fn cvar_set_r_grass(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.grass = Self::parse_cvar_bool(name, value)?;
                        self.mark_config_dirty();
                        self.console_status = format!(
                            "PROCEDURAL GRASS: {} (PRESS APPLY VIDEO SETTINGS OR RUN VID_RESTART)",
                            if self.video.grass { "ON" } else { "OFF" }
                        );
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_GRASSFRONTTOBACK {
        runtime: CvarDef::new(
                "r_grassFrontToBack",
                "",
                "0..1",
                "A/B front-to-back opaque grass ordering for early-Z (runtime diagnostic; not persisted).",
                CvarType::String,
                CvarFlags::NONE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_grassfronttoback(app)),
                |app, value| {
                    App::cvar_set_r_grassfronttoback(
                        app,
                        APP_R_GRASSFRONTTOBACK.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_grassfronttoback, None),
        config: CvarBinding {
                default: Some(|state| {
                    bool_value(state.video.grass_front_to_back)
                 }),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_grassfronttoback(&self) -> String {
                    bool_value(self.video.grass_front_to_back)
                }
                fn cvar_set_r_grassfronttoback(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.grass_front_to_back = Self::parse_cvar_bool(name, value)?;
                        self.render_command(RenderCommand::SetGrassFrontToBack(
                            self.video.grass_front_to_back,
                        ));
                        println!(
                            "Grass A/B: front_to_back={}",
                            u8::from(self.video.grass_front_to_back)
                        );
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_GRASSMIDLOD {
        runtime: CvarDef::new(
                "r_grassMidLod",
                "",
                "0..1",
                "A/B 5-triangle middle grass blade LOD (runtime diagnostic; not persisted).",
                CvarType::String,
                CvarFlags::NONE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_grassmidlod(app)),
                |app, value| {
                    App::cvar_set_r_grassmidlod(app, APP_R_GRASSMIDLOD.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_grassmidlod, None),
        config: CvarBinding {
                default: Some(|state| {
                    bool_value(state.video.grass_mid_lod)
                 }),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_grassmidlod(&self) -> String {
                    bool_value(self.video.grass_mid_lod)
                }
                fn cvar_set_r_grassmidlod(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.grass_mid_lod = Self::parse_cvar_bool(name, value)?;
                        self.render_command(RenderCommand::SetGrassMidLod(self.video.grass_mid_lod));
                        println!("Grass A/B: mid_lod={}", u8::from(self.video.grass_mid_lod));
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_GRASSPRECOMPUTE {
        runtime: CvarDef::new(
                "r_grassPrecompute",
                "",
                "0..1",
                "A/B grass root wind/clump compute prepass (runtime diagnostic; not persisted).",
                CvarType::String,
                CvarFlags::NONE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_grassprecompute(app)),
                |app, value| {
                    App::cvar_set_r_grassprecompute(
                        app,
                        APP_R_GRASSPRECOMPUTE.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_grassprecompute, None),
        config: CvarBinding {
                default: Some(|state| {
                    bool_value(state.video.grass_precompute)
                 }),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_grassprecompute(&self) -> String {
                    bool_value(self.video.grass_precompute)
                }
                fn cvar_set_r_grassprecompute(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.grass_precompute = Self::parse_cvar_bool(name, value)?;
                        self.render_command(RenderCommand::SetGrassPrecompute(
                            self.video.grass_precompute,
                        ));
                        println!(
                            "Grass A/B: precompute={}",
                            u8::from(self.video.grass_precompute)
                        );
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_RAIN {
        runtime: CvarDef::new(
                "r_rain",
                "",
                "0..1",
                "Camera-local GPU rain rendering.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_rain(app)),
                |app, value| App::cvar_set_r_rain(app, APP_R_RAIN.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_rain, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.rain = parse_bool(value).unwrap_or(settings.rain)
                }),
                archive: Some((87, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.rain))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_rain(&self) -> String {
                    bool_value(self.video.rain)
                }
                fn cvar_set_r_rain(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_rain".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_hdr" => self.video.hdr = enabled,
                            "r_tonemap" => self.video.tone_mapping = enabled,
                            "r_autoexposure" => self.video.auto_exposure = enabled,
                            "r_halation" => self.video.halation = enabled,
                            "r_ssao" => {
                                self.video.ssao = enabled;
                                if enabled {
                                    self.video.static_bsp_ao = false;
                                }
                            }
                            "r_staticbspao" => {
                                self.video.static_bsp_ao = enabled;
                                if enabled {
                                    self.video.ssao = false;
                                }
                            }
                            "r_fxaa" => {
                                self.video.fxaa = enabled;
                                if enabled {
                                    self.video.smaa = false;
                                    self.video.taa = false;
                                    self.video.msaa_samples = 1;
                                    self.render_command(RenderCommand::SetMsaa(1));
                                }
                            }
                            "r_smaa" => {
                                self.video.smaa = enabled;
                                if enabled {
                                    self.video.fxaa = false;
                                    self.video.taa = false;
                                    self.video.msaa_samples = 1;
                                    self.render_command(RenderCommand::SetMsaa(1));
                                }
                            }
                            "r_taa" => {
                                self.video.taa = enabled;
                                if enabled {
                                    self.video.fxaa = false;
                                    self.video.smaa = false;
                                    self.video.msaa_samples = 1;
                                    self.render_command(RenderCommand::SetMsaa(1));
                                }
                            }
                            "r_contactshadows" => self.video.contact_shadows = enabled,
                            "r_cloudshadows" => self.video.cloud_shadows = enabled,
                            "r_cloudtemporal" => self.video.cloud_temporal = enabled,
                            "r_cloudtemporaldepthfix" => self.video.cloud_temporal_depth_fix = enabled,
                            "r_cloudshapeevolution" => self.video.cloud_shape_evolution = enabled,
                            "r_cloudterraininteraction" => self.video.cloud_terrain_interaction = enabled,
                            "r_cloudemptyskip" => self.video.cloud_empty_skip = enabled,
                            "r_rain" => self.video.rain = enabled,
                            "r_sunoverride" => {
                                self.set_sun_override(enabled, enabled);
                                return Ok(());
                            }
                            "r_entitysunlighting" => self.video.entity_sun_lighting = enabled,
                            "r_reflectiondebug" => self.video.reflection_debug = enabled,
                            "r_vignette" => self.video.vignette = enabled,
                            _ => unreachable!(),
                        }
                        self.sync_post_effects();
                        if lower != "r_reflectiondebug" {
                            self.mark_config_dirty();
                        }
                        // r_floatLightmap is only effective while HDR is active, matching
                        // Rend2. If that changes the prepared lightmap representation,
                        // video_restart_required() exposes the normal renderer-restart path.
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_RAINGLASS {
        runtime: CvarDef::new(
                "r_rainGlass",
                "",
                "0..1",
                "Rain on the camera lens: refracted droplets and rivulets over the finished scene.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_rainglass(app)),
                |app, value| App::cvar_set_r_rainglass(app, APP_R_RAINGLASS.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_rainglass, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.rain_glass = parse_bool(value).unwrap_or(settings.rain_glass)
                }),
                archive: Some((416, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.rain_glass))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_rainglass(&self) -> String {
                    bool_value(self.video.rain_glass)
                }
                fn cvar_set_r_rainglass(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.video.rain_glass = Self::parse_cvar_bool(name, value)?;
                    self.sync_post_effects();
                    self.mark_config_dirty();
                    self.publish_ui();
                    Ok(())
                }
            }
        ]
    }

    APP_R_RAINGRADE {
        runtime: CvarDef::new(
                "r_rainGrade",
                "",
                "0..1",
                "Wet-weather colour grade (cool shadows, warm highlights, richer neon) while it rains.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_raingrade(app)),
                |app, value| App::cvar_set_r_raingrade(app, APP_R_RAINGRADE.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_raingrade, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.rain_grade = value
                            .parse::<f32>()
                            .ok()
                            .filter(|strength| strength.is_finite())
                            .map(|strength| strength.clamp(0.0, 1.0))
                            .unwrap_or(settings.rain_grade)
                    }
                }),
                archive: Some((91, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.rain_grade)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_raingrade(&self) -> String {
                    format!("{:.3}", self.video.rain_grade)
                }
                fn cvar_set_r_raingrade(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let strength = value
                            .trim()
                            .parse::<f32>()
                            .map_err(|_| format!("{name}: expected 0..1"))?;
                        self.set_rain_grade(strength);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_RAININTENSITY {
        runtime: CvarDef::new(
                "r_rainIntensity",
                "",
                "light|rain|heavy",
                "Rain preset used by the shared precipitation renderer.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_rainintensity(app)),
                |app, value| {
                    App::cvar_set_r_rainintensity(app, APP_R_RAININTENSITY.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_rainintensity, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.rain_intensity =
                            RainIntensity::from_config(value).unwrap_or(settings.rain_intensity)
                    }
                }),
                archive: Some((88, |state| {
                    let settings = state.video;
                    format!("{}", settings.rain_intensity.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_rainintensity(&self) -> String {
                    self.video.rain_intensity.config_value().to_owned()
                }
                fn cvar_set_r_rainintensity(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.rain_intensity = RainIntensity::from_config(value)
                            .ok_or_else(|| format!("{name}: expected light|rain|heavy"))?;
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_SUNCOLOR {
        runtime: CvarDef::new(
                "r_sunColor",
                "",
                "R,G,B (0..1)",
                "Custom q3 sun chromaticity; use comma-separated RGB in the console.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_suncolor(app)),
                |app, value| App::cvar_set_r_suncolor(app, APP_R_SUNCOLOR.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_suncolor, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Some(color) = parse_vec3(value) {
                        settings.sun_color = color.map(|channel| channel.clamp(0.0, 1.0));
                    }
                }),
                archive: Some((62, |state| {
                    let settings = state.video;
                    format!(
                        "{:.4} {:.4} {:.4}",
                        settings.sun_color[0], settings.sun_color[1], settings.sun_color[2]
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_suncolor(&self) -> String {
                    format!(
                        "{:.4} {:.4} {:.4}",
                        self.video.sun_color[0], self.video.sun_color[1], self.video.sun_color[2]
                    )
                }
                fn cvar_set_r_suncolor(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let values = value
                            .split([',', ' '])
                            .filter(|part| !part.is_empty())
                            .map(str::parse::<f32>)
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|_| format!("{name}: expected R G B in the 0..1 range"))?;
                        if values.len() != 3 || values.iter().any(|v| !v.is_finite()) {
                            return Err(format!("{name}: expected R G B in the 0..1 range"));
                        }
                        self.set_sun_color([
                            values[0].clamp(0.0, 1.0),
                            values[1].clamp(0.0, 1.0),
                            values[2].clamp(0.0, 1.0),
                        ]);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_SUNINTENSITY {
        runtime: CvarDef::new(
                "r_sunIntensity",
                "",
                "0..",
                "Custom q3 sun intensity for runtime sun-aware renderer effects.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_sunintensity(app)),
                |app, value| {
                    App::cvar_set_r_sunintensity(app, APP_R_SUNINTENSITY.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_sunintensity, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(intensity) = value.parse::<f32>() {
                        if intensity.is_finite() {
                            settings.sun_intensity = intensity.max(0.0);
                        }
                    }
                }),
                archive: Some((61, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.sun_intensity)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_sunintensity(&self) -> String {
                    format!("{:.3}", self.video.sun_intensity)
                }
                fn cvar_set_r_sunintensity(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let intensity = value
                            .trim()
                            .parse::<f32>()
                            .map_err(|_| format!("{name}: expected a non-negative number"))?;
                        if !intensity.is_finite() || intensity < 0.0 {
                            return Err(format!("{name}: expected a non-negative finite number"));
                        }
                        self.set_sun_intensity(intensity);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_SUNOVERRIDE {
        runtime: CvarDef::new(
                "r_sunOverride",
                "",
                "0..1",
                "Use a runtime custom q3 shader sun instead of the map-authored sky sun.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_sunoverride(app)),
                |app, value| {
                    App::cvar_set_r_sunoverride(app, APP_R_SUNOVERRIDE.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_sunoverride, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.sun_override = parse_bool(value).unwrap_or(settings.sun_override)
                    }
                }),
                archive: Some((56, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.sun_override))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_sunoverride(&self) -> String {
                    bool_value(self.video.sun_override)
                }
                fn cvar_set_r_sunoverride(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_sunoverride".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_hdr" => self.video.hdr = enabled,
                            "r_tonemap" => self.video.tone_mapping = enabled,
                            "r_autoexposure" => self.video.auto_exposure = enabled,
                            "r_halation" => self.video.halation = enabled,
                            "r_ssao" => {
                                self.video.ssao = enabled;
                                if enabled {
                                    self.video.static_bsp_ao = false;
                                }
                            }
                            "r_staticbspao" => {
                                self.video.static_bsp_ao = enabled;
                                if enabled {
                                    self.video.ssao = false;
                                }
                            }
                            "r_fxaa" => {
                                self.video.fxaa = enabled;
                                if enabled {
                                    self.video.smaa = false;
                                    self.video.taa = false;
                                    self.video.msaa_samples = 1;
                                    self.render_command(RenderCommand::SetMsaa(1));
                                }
                            }
                            "r_smaa" => {
                                self.video.smaa = enabled;
                                if enabled {
                                    self.video.fxaa = false;
                                    self.video.taa = false;
                                    self.video.msaa_samples = 1;
                                    self.render_command(RenderCommand::SetMsaa(1));
                                }
                            }
                            "r_taa" => {
                                self.video.taa = enabled;
                                if enabled {
                                    self.video.fxaa = false;
                                    self.video.smaa = false;
                                    self.video.msaa_samples = 1;
                                    self.render_command(RenderCommand::SetMsaa(1));
                                }
                            }
                            "r_contactshadows" => self.video.contact_shadows = enabled,
                            "r_cloudshadows" => self.video.cloud_shadows = enabled,
                            "r_cloudtemporal" => self.video.cloud_temporal = enabled,
                            "r_cloudtemporaldepthfix" => self.video.cloud_temporal_depth_fix = enabled,
                            "r_cloudshapeevolution" => self.video.cloud_shape_evolution = enabled,
                            "r_cloudterraininteraction" => self.video.cloud_terrain_interaction = enabled,
                            "r_cloudemptyskip" => self.video.cloud_empty_skip = enabled,
                            "r_rain" => self.video.rain = enabled,
                            "r_sunoverride" => {
                                self.set_sun_override(enabled, enabled);
                                return Ok(());
                            }
                            "r_entitysunlighting" => self.video.entity_sun_lighting = enabled,
                            "r_reflectiondebug" => self.video.reflection_debug = enabled,
                            "r_vignette" => self.video.vignette = enabled,
                            _ => unreachable!(),
                        }
                        self.sync_post_effects();
                        if lower != "r_reflectiondebug" {
                            self.mark_config_dirty();
                        }
                        // r_floatLightmap is only effective while HDR is active, matching
                        // Rend2. If that changes the prepared lightmap representation,
                        // video_restart_required() exposes the normal renderer-restart path.
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_SUNPITCH {
        runtime: CvarDef::new(
                "r_sunPitch",
                "",
                "-90..90",
                "Custom q3 sun elevation above the horizon.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_sunpitch(app)),
                |app, value| App::cvar_set_r_sunpitch(app, APP_R_SUNPITCH.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_sunpitch, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(pitch) = value.parse::<f32>() {
                        if pitch.is_finite() {
                            settings.sun_pitch = pitch.clamp(-90.0, 90.0);
                        }
                    }
                }),
                archive: Some((60, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.sun_pitch)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_sunpitch(&self) -> String {
                    format!("{:.3}", self.video.sun_pitch)
                }
                fn cvar_set_r_sunpitch(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let pitch = value
                            .trim()
                            .parse::<f32>()
                            .map_err(|_| format!("{name}: expected degrees"))?;
                        if !pitch.is_finite() {
                            return Err(format!("{name}: expected finite degrees"));
                        }
                        self.set_sun_pitch(pitch);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_SUNVISIBILITY {
        runtime: CvarDef::new(
                "r_sunVisibility",
                "",
                "custom",
                "Controls r_sunVisibility.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_sunvisibility(app)),
                |app, value| {
                    App::cvar_set_r_sunvisibility(app, APP_R_SUNVISIBILITY.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_sunvisibility, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.sun_visibility =
                        SunVisibilityMode::from_config(value).unwrap_or(settings.sun_visibility);
                }),
                archive: Some((57, |state| {
                    let settings = state.video;
                    format!("{}", settings.sun_visibility.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_sunvisibility(&self) -> String {
                    self.video.sun_visibility.config_value().to_owned()
                }
                fn cvar_set_r_sunvisibility(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.sun_visibility = SunVisibilityMode::from_config(value)
                            .ok_or_else(|| format!("{name}: expected legacy|sky|filtered"))?;
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_SUNYAW {
        runtime: CvarDef::new(
                "r_sunYaw",
                "",
                "degrees",
                "Custom q3 sun azimuth: 0 east, 90 north.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_sunyaw(app)),
                |app, value| App::cvar_set_r_sunyaw(app, APP_R_SUNYAW.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_sunyaw, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    if let Ok(yaw) = value.parse::<f32>() {
                        if yaw.is_finite() {
                            settings.sun_yaw = yaw.rem_euclid(360.0);
                        }
                    }
                }),
                archive: Some((59, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.sun_yaw)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_sunyaw(&self) -> String {
                    format!("{:.3}", self.video.sun_yaw)
                }
                fn cvar_set_r_sunyaw(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let yaw = value
                            .trim()
                            .parse::<f32>()
                            .map_err(|_| format!("{name}: expected degrees"))?;
                        if !yaw.is_finite() {
                            return Err(format!("{name}: expected finite degrees"));
                        }
                        self.set_sun_yaw(yaw);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_WEATHERWIND {
        runtime: CvarDef::new(
                "r_weatherWind",
                "",
                "speed direction gust direction_variation",
                "Shared atmospheric wind used by clouds, rain, grass and ocean systems.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_weatherwind(app)),
                |app, value| {
                    App::cvar_set_r_weatherwind(app, APP_R_WEATHERWIND.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_weatherwind, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    let v: Option<Vec<f32>> = value.split_whitespace().map(|s| s.parse().ok()).collect();
                    if let Some(v) = v.filter(|v| v.len() == 4) {
                        settings.weather_wind = crate::ocean::OceanWind {
                            speed: v[0],
                            direction: v[1],
                            gust: v[2],
                            shift: v[3],
                        }
                        .sanitize();
                    }
                }),
                archive: Some((70, |state| {
                    let settings = state.video;
                    format!(
                        "{:.2} {:.1} {:.3} {:.1}",
                        settings.weather_wind.speed,
                        settings.weather_wind.direction,
                        settings.weather_wind.gust,
                        settings.weather_wind.shift
                    )
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_weatherwind(&self) -> String {
                    format!(
                        "{:.2} {:.1} {:.3} {:.1}",
                        self.video.weather_wind.speed,
                        self.video.weather_wind.direction,
                        self.video.weather_wind.gust,
                        self.video.weather_wind.shift,
                    )
                }
                fn cvar_set_r_weatherwind(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        let values = value
                            .split_whitespace()
                            .map(str::parse::<f32>)
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|_| {
                                format!("{name}: expected speed direction gust direction_variation")
                            })?;
                        if values.len() != 4 || !values.iter().all(|v| v.is_finite()) {
                            return Err(format!("{name}: expected four finite numbers"));
                        }
                        self.set_weather_wind(crate::ocean::OceanWind {
                            speed: values[0],
                            direction: values[1],
                            gust: values[2],
                            shift: values[3],
                        });
                    }
                    Ok(())
                }
            }
        ]
    }
}

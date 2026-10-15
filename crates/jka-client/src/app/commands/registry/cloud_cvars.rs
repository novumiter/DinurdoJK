//! Cloud cvars: declarative runtime and config bindings.
use super::cvar_prelude::*;

app_cvars! {
    APP_R_CLOUDAERIAL {
        runtime: CvarDef::new(
                "r_cloudAerial",
                "",
                "custom",
                "Controls r_cloudAerial.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudaerial(app)),
                |app, value| {
                    App::cvar_set_r_cloudaerial(app, APP_R_CLOUDAERIAL.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudaerial, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_aerial = value
                            .parse::<f32>()
                            .ok()
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_aerial)
                    }
                }),
                archive: Some((80, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_aerial)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudaerial(&self) -> String {
                    format!("{:.3}", self.video.cloud_aerial)
                }
                fn cvar_set_r_cloudaerial(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudaerial".to_owned();
                    {
                        let amount = Self::parse_cvar_finite(name, value)?;
                        let video = &mut self.video;
                        match lower.as_str() {
                            "r_cloudshear" => video.cloud_shear = amount.clamp(0.0, 1.0),
                            "r_cloudbasevariation" => video.cloud_base_variation = amount.clamp(0.0, 1.0),
                            "r_cloudaerial" => video.cloud_aerial = amount.clamp(0.0, 1.0),
                            "r_cloudhistoryblend" => video.cloud_history_blend = amount.clamp(0.0, 0.98),
                            "r_cloudmotionreject" => video.cloud_motion_reject = amount.clamp(0.0, 1.0),
                            "r_cloudthicknessvariation" => {
                                video.cloud_thickness_variation = amount.clamp(0.0, 1.0)
                            }
                            _ => video.cloud_size = amount.clamp(0.0, 1.0),
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDBASEVARIATION {
        runtime: CvarDef::new(
                "r_cloudBaseVariation",
                "",
                "custom",
                "Controls r_cloudBaseVariation.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudbasevariation(app)),
                |app, value| {
                    App::cvar_set_r_cloudbasevariation(
                        app,
                        APP_R_CLOUDBASEVARIATION.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudbasevariation, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_base_variation = value
                            .parse::<f32>()
                            .ok()
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_base_variation)
                    }
                }),
                archive: Some((76, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_base_variation)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudbasevariation(&self) -> String {
                    format!("{:.3}", self.video.cloud_base_variation)
                }
                fn cvar_set_r_cloudbasevariation(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudbasevariation".to_owned();
                    {
                        let amount = Self::parse_cvar_finite(name, value)?;
                        let video = &mut self.video;
                        match lower.as_str() {
                            "r_cloudshear" => video.cloud_shear = amount.clamp(0.0, 1.0),
                            "r_cloudbasevariation" => video.cloud_base_variation = amount.clamp(0.0, 1.0),
                            "r_cloudaerial" => video.cloud_aerial = amount.clamp(0.0, 1.0),
                            "r_cloudhistoryblend" => video.cloud_history_blend = amount.clamp(0.0, 0.98),
                            "r_cloudmotionreject" => video.cloud_motion_reject = amount.clamp(0.0, 1.0),
                            "r_cloudthicknessvariation" => {
                                video.cloud_thickness_variation = amount.clamp(0.0, 1.0)
                            }
                            _ => video.cloud_size = amount.clamp(0.0, 1.0),
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDCOVERAGE {
        runtime: CvarDef::new(
                "r_cloudCoverage",
                "",
                "custom",
                "Controls r_cloudCoverage.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudcoverage(app)),
                |app, value| {
                    App::cvar_set_r_cloudcoverage(app, APP_R_CLOUDCOVERAGE.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudcoverage, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_coverage = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_coverage)
                    }
                }),
                archive: Some((67, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_coverage)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudcoverage(&self) -> String {
                    format!("{:.3}", self.video.cloud_coverage)
                }
                fn cvar_set_r_cloudcoverage(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_cloud_coverage(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDEMPTYSKIP {
        runtime: CvarDef::new(
                "r_cloudEmptySkip",
                "",
                "0..1",
                "A/B adaptive empty-space skipping in the primary volumetric cloud march.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudemptyskip(app)),
                |app, value| {
                    App::cvar_set_r_cloudemptyskip(app, APP_R_CLOUDEMPTYSKIP.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudemptyskip, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_empty_skip = parse_bool(value).unwrap_or(settings.cloud_empty_skip)
                    }
                }),
                archive: Some((79, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_empty_skip))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudemptyskip(&self) -> String {
                    bool_value(self.video.cloud_empty_skip)
                }
                fn cvar_set_r_cloudemptyskip(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudemptyskip".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_tonemap" => self.video.tone_mapping = if enabled { 1.0 } else { 0.0 },
                            "r_autoexposure" => self.video.auto_exposure = if enabled { 1.0 } else { 0.0 },
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

    APP_R_CLOUDHEIGHT {
        runtime: CvarDef::new(
                "r_cloudHeight",
                "",
                "custom",
                "Controls r_cloudHeight.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudheight(app)),
                |app, value| {
                    App::cvar_set_r_cloudheight(app, APP_R_CLOUDHEIGHT.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudheight, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_height = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(CLOUD_HEIGHT_MIN, CLOUD_HEIGHT_MAX))
                            .unwrap_or(settings.cloud_height)
                    }
                }),
                archive: Some((68, |state| {
                    let settings = state.video;
                    format!("{:.1}", settings.cloud_height)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudheight(&self) -> String {
                    format!("{:.1}", self.video.cloud_height)
                }
                fn cvar_set_r_cloudheight(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_cloud_height(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDHISTORYBLEND {
        runtime: CvarDef::new(
                "r_cloudHistoryBlend",
                "",
                "custom",
                "Controls r_cloudHistoryBlend.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudhistoryblend(app)),
                |app, value| {
                    App::cvar_set_r_cloudhistoryblend(
                        app,
                        APP_R_CLOUDHISTORYBLEND.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudhistoryblend, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_history_blend = value
                            .parse::<f32>()
                            .ok()
                            .map(|v| v.clamp(0.0, 0.98))
                            .unwrap_or(settings.cloud_history_blend)
                    }
                }),
                archive: Some((82, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_history_blend)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudhistoryblend(&self) -> String {
                    format!("{:.3}", self.video.cloud_history_blend)
                }
                fn cvar_set_r_cloudhistoryblend(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudhistoryblend".to_owned();
                    {
                        let amount = Self::parse_cvar_finite(name, value)?;
                        let video = &mut self.video;
                        match lower.as_str() {
                            "r_cloudshear" => video.cloud_shear = amount.clamp(0.0, 1.0),
                            "r_cloudbasevariation" => video.cloud_base_variation = amount.clamp(0.0, 1.0),
                            "r_cloudaerial" => video.cloud_aerial = amount.clamp(0.0, 1.0),
                            "r_cloudhistoryblend" => video.cloud_history_blend = amount.clamp(0.0, 0.98),
                            "r_cloudmotionreject" => video.cloud_motion_reject = amount.clamp(0.0, 1.0),
                            "r_cloudthicknessvariation" => {
                                video.cloud_thickness_variation = amount.clamp(0.0, 1.0)
                            }
                            _ => video.cloud_size = amount.clamp(0.0, 1.0),
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDHISTORYDEPTHREJECT {
        runtime: CvarDef::new(
                "r_cloudHistoryDepthReject",
                "",
                "custom",
                "Controls r_cloudHistoryDepthReject.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudhistorydepthreject(app)),
                |app, value| {
                    App::cvar_set_r_cloudhistorydepthreject(
                        app,
                        APP_R_CLOUDHISTORYDEPTHREJECT.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudhistorydepthreject, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_history_depth_reject =
                            parse_bool(value).unwrap_or(settings.cloud_history_depth_reject)
                    }
                }),
                archive: Some((84, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_history_depth_reject))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudhistorydepthreject(&self) -> String {
                    bool_value(self.video.cloud_history_depth_reject)
                }
                fn cvar_set_r_cloudhistorydepthreject(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    let lower = "r_cloudhistorydepthreject".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        if lower == "r_cloudskyambient" {
                            self.video.cloud_sky_ambient = enabled;
                        } else {
                            self.video.cloud_history_depth_reject = enabled;
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDMOTIONREJECT {
        runtime: CvarDef::new(
                "r_cloudMotionReject",
                "",
                "custom",
                "Controls r_cloudMotionReject.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudmotionreject(app)),
                |app, value| {
                    App::cvar_set_r_cloudmotionreject(
                        app,
                        APP_R_CLOUDMOTIONREJECT.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudmotionreject, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_motion_reject = value
                            .parse::<f32>()
                            .ok()
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_motion_reject)
                    }
                }),
                archive: Some((83, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_motion_reject)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudmotionreject(&self) -> String {
                    format!("{:.3}", self.video.cloud_motion_reject)
                }
                fn cvar_set_r_cloudmotionreject(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudmotionreject".to_owned();
                    {
                        let amount = Self::parse_cvar_finite(name, value)?;
                        let video = &mut self.video;
                        match lower.as_str() {
                            "r_cloudshear" => video.cloud_shear = amount.clamp(0.0, 1.0),
                            "r_cloudbasevariation" => video.cloud_base_variation = amount.clamp(0.0, 1.0),
                            "r_cloudaerial" => video.cloud_aerial = amount.clamp(0.0, 1.0),
                            "r_cloudhistoryblend" => video.cloud_history_blend = amount.clamp(0.0, 0.98),
                            "r_cloudmotionreject" => video.cloud_motion_reject = amount.clamp(0.0, 1.0),
                            "r_cloudthicknessvariation" => {
                                video.cloud_thickness_variation = amount.clamp(0.0, 1.0)
                            }
                            _ => video.cloud_size = amount.clamp(0.0, 1.0),
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDQUALITY {
        runtime: CvarDef::new(
                "r_cloudQuality",
                "",
                "custom",
                "Controls r_cloudQuality.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudquality(app)),
                |app, value| {
                    App::cvar_set_r_cloudquality(app, APP_R_CLOUDQUALITY.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudquality, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_quality = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_quality)
                    }
                }),
                archive: Some((66, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_quality)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudquality(&self) -> String {
                    format!("{:.3}", self.video.cloud_quality)
                }
                fn cvar_set_r_cloudquality(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.set_cloud_quality(Self::parse_cvar_finite(name, value)?);
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDRENDERRESOLUTION {
        runtime: CvarDef::new(
                "r_cloudRenderResolution",
                "",
                "custom",
                "Controls r_cloudRenderResolution.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudrenderresolution(app)),
                |app, value| {
                    App::cvar_set_r_cloudrenderresolution(
                        app,
                        APP_R_CLOUDRENDERRESOLUTION.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudrenderresolution, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_render_resolution = CloudRenderResolution::from_config(value)
                            .unwrap_or(settings.cloud_render_resolution)
                    }
                }),
                archive: Some((72, |state| {
                    let settings = state.video;
                    format!("{}", settings.cloud_render_resolution.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudrenderresolution(&self) -> String {
                    {
                        self.video.cloud_render_resolution.config_value().to_owned()
                    }
                }
                fn cvar_set_r_cloudrenderresolution(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    {
                        self.video.cloud_render_resolution = CloudRenderResolution::from_config(value)
                            .ok_or_else(|| format!("{name}: expected full|half|quarter"))?;
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDS {
        runtime: CvarDef::new(
                "r_clouds",
                "",
                "custom",
                "Controls r_clouds.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_clouds(app)),
                |app, value| App::cvar_set_r_clouds(app, APP_R_CLOUDS.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_clouds, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.clouds = parse_bool(value).unwrap_or(settings.clouds)
                }),
                archive: Some((64, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.clouds))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_clouds(&self) -> String {
                    bool_value(self.video.clouds)
                }
                fn cvar_set_r_clouds(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.clouds = Self::parse_cvar_bool(name, value)?;
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDSHADOWS {
        runtime: CvarDef::new(
                "r_cloudShadows",
                "",
                "custom",
                "Controls r_cloudShadows.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudshadows(app)),
                |app, value| {
                    App::cvar_set_r_cloudshadows(app, APP_R_CLOUDSHADOWS.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudshadows, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_shadows = parse_bool(value).unwrap_or(settings.cloud_shadows)
                    }
                }),
                archive: Some((71, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_shadows))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudshadows(&self) -> String {
                    bool_value(self.video.cloud_shadows)
                }
                fn cvar_set_r_cloudshadows(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudshadows".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_tonemap" => self.video.tone_mapping = if enabled { 1.0 } else { 0.0 },
                            "r_autoexposure" => self.video.auto_exposure = if enabled { 1.0 } else { 0.0 },
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

    APP_R_CLOUDSHAPEEVOLUTION {
        runtime: CvarDef::new(
                "r_cloudShapeEvolution",
                "",
                "0..1",
                "Enable slow relative evolution of cloud detail and billows.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudshapeevolution(app)),
                |app, value| {
                    App::cvar_set_r_cloudshapeevolution(
                        app,
                        APP_R_CLOUDSHAPEEVOLUTION.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudshapeevolution, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_shape_evolution =
                            parse_bool(value).unwrap_or(settings.cloud_shape_evolution)
                    }
                }),
                archive: Some((77, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_shape_evolution))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudshapeevolution(&self) -> String {
                    bool_value(self.video.cloud_shape_evolution)
                }
                fn cvar_set_r_cloudshapeevolution(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    let lower = "r_cloudshapeevolution".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_tonemap" => self.video.tone_mapping = if enabled { 1.0 } else { 0.0 },
                            "r_autoexposure" => self.video.auto_exposure = if enabled { 1.0 } else { 0.0 },
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

    APP_R_CLOUDSHEAR {
        runtime: CvarDef::new(
                "r_cloudShear",
                "",
                "custom",
                "Controls r_cloudShear.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudshear(app)),
                |app, value| {
                    App::cvar_set_r_cloudshear(app, APP_R_CLOUDSHEAR.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudshear, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_shear = value
                            .parse::<f32>()
                            .ok()
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_shear)
                    }
                }),
                archive: Some((75, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_shear)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudshear(&self) -> String {
                    format!("{:.3}", self.video.cloud_shear)
                }
                fn cvar_set_r_cloudshear(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudshear".to_owned();
                    {
                        let amount = Self::parse_cvar_finite(name, value)?;
                        let video = &mut self.video;
                        match lower.as_str() {
                            "r_cloudshear" => video.cloud_shear = amount.clamp(0.0, 1.0),
                            "r_cloudbasevariation" => video.cloud_base_variation = amount.clamp(0.0, 1.0),
                            "r_cloudaerial" => video.cloud_aerial = amount.clamp(0.0, 1.0),
                            "r_cloudhistoryblend" => video.cloud_history_blend = amount.clamp(0.0, 0.98),
                            "r_cloudmotionreject" => video.cloud_motion_reject = amount.clamp(0.0, 1.0),
                            "r_cloudthicknessvariation" => {
                                video.cloud_thickness_variation = amount.clamp(0.0, 1.0)
                            }
                            _ => video.cloud_size = amount.clamp(0.0, 1.0),
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDSIZE {
        runtime: CvarDef::new(
                "r_cloudSize",
                "",
                "custom",
                "Controls r_cloudSize.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudsize(app)),
                |app, value| App::cvar_set_r_cloudsize(app, APP_R_CLOUDSIZE.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_cloudsize, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_size = value
                            .parse::<f32>()
                            .ok()
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_size)
                    }
                }),
                archive: Some((86, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_size)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudsize(&self) -> String {
                    format!("{:.3}", self.video.cloud_size)
                }
                fn cvar_set_r_cloudsize(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudsize".to_owned();
                    {
                        let amount = Self::parse_cvar_finite(name, value)?;
                        let video = &mut self.video;
                        match lower.as_str() {
                            "r_cloudshear" => video.cloud_shear = amount.clamp(0.0, 1.0),
                            "r_cloudbasevariation" => video.cloud_base_variation = amount.clamp(0.0, 1.0),
                            "r_cloudaerial" => video.cloud_aerial = amount.clamp(0.0, 1.0),
                            "r_cloudhistoryblend" => video.cloud_history_blend = amount.clamp(0.0, 0.98),
                            "r_cloudmotionreject" => video.cloud_motion_reject = amount.clamp(0.0, 1.0),
                            "r_cloudthicknessvariation" => {
                                video.cloud_thickness_variation = amount.clamp(0.0, 1.0)
                            }
                            _ => video.cloud_size = amount.clamp(0.0, 1.0),
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDSKYAMBIENT {
        runtime: CvarDef::new(
                "r_cloudSkyAmbient",
                "",
                "custom",
                "Controls r_cloudSkyAmbient.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudskyambient(app)),
                |app, value| {
                    App::cvar_set_r_cloudskyambient(
                        app,
                        APP_R_CLOUDSKYAMBIENT.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudskyambient, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_sky_ambient = parse_bool(value).unwrap_or(settings.cloud_sky_ambient)
                    }
                }),
                archive: Some((81, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_sky_ambient))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudskyambient(&self) -> String {
                    bool_value(self.video.cloud_sky_ambient)
                }
                fn cvar_set_r_cloudskyambient(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudskyambient".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        if lower == "r_cloudskyambient" {
                            self.video.cloud_sky_ambient = enabled;
                        } else {
                            self.video.cloud_history_depth_reject = enabled;
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDTEMPORAL {
        runtime: CvarDef::new(
                "r_cloudTemporal",
                "",
                "custom",
                "Controls r_cloudTemporal.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudtemporal(app)),
                |app, value| {
                    App::cvar_set_r_cloudtemporal(app, APP_R_CLOUDTEMPORAL.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudtemporal, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_temporal = parse_bool(value).unwrap_or(settings.cloud_temporal)
                    }
                }),
                archive: Some((73, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_temporal))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudtemporal(&self) -> String {
                    bool_value(self.video.cloud_temporal)
                }
                fn cvar_set_r_cloudtemporal(&mut self, name: &str, value: &str) -> Result<(), String> {
                    let lower = "r_cloudtemporal".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_tonemap" => self.video.tone_mapping = if enabled { 1.0 } else { 0.0 },
                            "r_autoexposure" => self.video.auto_exposure = if enabled { 1.0 } else { 0.0 },
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

    APP_R_CLOUDTEMPORALDEPTHFIX {
        runtime: CvarDef::new(
                "r_cloudTemporalDepthFix",
                "",
                "custom",
                "Controls r_cloudTemporalDepthFix.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudtemporaldepthfix(app)),
                |app, value| {
                    App::cvar_set_r_cloudtemporaldepthfix(
                        app,
                        APP_R_CLOUDTEMPORALDEPTHFIX.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudtemporaldepthfix, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_temporal_depth_fix =
                            parse_bool(value).unwrap_or(settings.cloud_temporal_depth_fix)
                    }
                }),
                archive: Some((74, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_temporal_depth_fix))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudtemporaldepthfix(&self) -> String {
                    bool_value(self.video.cloud_temporal_depth_fix)
                }
                fn cvar_set_r_cloudtemporaldepthfix(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    let lower = "r_cloudtemporaldepthfix".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_tonemap" => self.video.tone_mapping = if enabled { 1.0 } else { 0.0 },
                            "r_autoexposure" => self.video.auto_exposure = if enabled { 1.0 } else { 0.0 },
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

    APP_R_CLOUDTERRAININTERACTION {
        runtime: CvarDef::new(
                "r_cloudTerrainInteraction",
                "",
                "0..1",
                "Enable terrain-driven lifting of low cloud shapes.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudterraininteraction(app)),
                |app, value| {
                    App::cvar_set_r_cloudterraininteraction(
                        app,
                        APP_R_CLOUDTERRAININTERACTION.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudterraininteraction, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_terrain_interaction =
                            parse_bool(value).unwrap_or(settings.cloud_terrain_interaction)
                    }
                }),
                archive: Some((78, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.cloud_terrain_interaction))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudterraininteraction(&self) -> String {
                    bool_value(self.video.cloud_terrain_interaction)
                }
                fn cvar_set_r_cloudterraininteraction(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    let lower = "r_cloudterraininteraction".to_owned();
                    {
                        let enabled = Self::parse_cvar_bool(name, value)?;
                        match lower.as_str() {
                            "r_tonemap" => self.video.tone_mapping = if enabled { 1.0 } else { 0.0 },
                            "r_autoexposure" => self.video.auto_exposure = if enabled { 1.0 } else { 0.0 },
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

    APP_R_CLOUDTHICKNESS {
        runtime: CvarDef::new(
                "r_cloudThickness",
                "",
                "custom",
                "Controls r_cloudThickness.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudthickness(app)),
                |app, value| {
                    App::cvar_set_r_cloudthickness(app, APP_R_CLOUDTHICKNESS.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_cloudthickness, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_thickness = value
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .map(|v| v.clamp(CLOUD_THICKNESS_MIN, CLOUD_THICKNESS_MAX))
                            .unwrap_or(settings.cloud_thickness)
                    }
                }),
                archive: Some((69, |state| {
                    let settings = state.video;
                    format!("{:.1}", settings.cloud_thickness)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudthickness(&self) -> String {
                    format!("{:.1}", self.video.cloud_thickness)
                }
                fn cvar_set_r_cloudthickness(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.set_cloud_thickness(Self::parse_cvar_finite(name, value)?);
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDTHICKNESSVARIATION {
        runtime: CvarDef::new(
                "r_cloudThicknessVariation",
                "",
                "custom",
                "Controls r_cloudThicknessVariation.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudthicknessvariation(app)),
                |app, value| {
                    App::cvar_set_r_cloudthicknessvariation(
                        app,
                        APP_R_CLOUDTHICKNESSVARIATION.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_r_cloudthicknessvariation, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_thickness_variation = value
                            .parse::<f32>()
                            .ok()
                            .map(|v| v.clamp(0.0, 1.0))
                            .unwrap_or(settings.cloud_thickness_variation)
                    }
                }),
                archive: Some((85, |state| {
                    let settings = state.video;
                    format!("{:.3}", settings.cloud_thickness_variation)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudthicknessvariation(&self) -> String {
                    format!("{:.3}", self.video.cloud_thickness_variation)
                }
                fn cvar_set_r_cloudthicknessvariation(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    let lower = "r_cloudthicknessvariation".to_owned();
                    {
                        let amount = Self::parse_cvar_finite(name, value)?;
                        let video = &mut self.video;
                        match lower.as_str() {
                            "r_cloudshear" => video.cloud_shear = amount.clamp(0.0, 1.0),
                            "r_cloudbasevariation" => video.cloud_base_variation = amount.clamp(0.0, 1.0),
                            "r_cloudaerial" => video.cloud_aerial = amount.clamp(0.0, 1.0),
                            "r_cloudhistoryblend" => video.cloud_history_blend = amount.clamp(0.0, 0.98),
                            "r_cloudmotionreject" => video.cloud_motion_reject = amount.clamp(0.0, 1.0),
                            "r_cloudthicknessvariation" => {
                                video.cloud_thickness_variation = amount.clamp(0.0, 1.0)
                            }
                            _ => video.cloud_size = amount.clamp(0.0, 1.0),
                        }
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_CLOUDTYPE {
        runtime: CvarDef::new(
                "r_cloudType",
                "",
                "custom",
                "Controls r_cloudType.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_cloudtype(app)),
                |app, value| App::cvar_set_r_cloudtype(app, APP_R_CLOUDTYPE.entry.name, &value.to_string()),
            )
            .with_text_setter(App::cvar_set_r_cloudtype, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    {
                        settings.cloud_type = CloudType::from_config(value).unwrap_or(settings.cloud_type)
                    }
                }),
                archive: Some((65, |state| {
                    let settings = state.video;
                    format!("{}", settings.cloud_type.config_value())
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_cloudtype(&self) -> String {
                    self.video.cloud_type.config_value().to_owned()
                }
                fn cvar_set_r_cloudtype(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.cloud_type = ui::CloudType::from_config(value)
                            .ok_or_else(|| format!("{name}: expected cumulus|stratus|storm"))?;
                        self.sync_post_effects();
                        self.mark_config_dirty();
                        self.publish_ui();
                    }
                    Ok(())
                }
            }
        ]
    }
    APP_R_SKYFRACTAL {
        runtime: CvarDef::new(
                "r_skyFractal", "", "0|1",
                "Experimental Star Nest sky override: 0 map sky, 1 procedural fractal sky. Affects BSP sky faces only. High GPU cost; A/B test.",
                CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
                |app: &App| CvarValue::String(bool_value(app.video.sky_fractal_override)),
                |app, value| App::cvar_set_r_skyfractal(app, APP_R_SKYFRACTAL.entry.name, &value.to_string()),
            ).with_text_setter(App::cvar_set_r_skyfractal, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.sky_fractal_override = parse_bool(value).unwrap_or(settings.sky_fractal_override)
                }),
                archive: Some((66, |state| format!("{}", u8::from(state.video.sky_fractal_override)))),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_set_r_skyfractal(&mut self, name: &str, value: &str) -> Result<(), String> {
                    self.video.sky_fractal_override = Self::parse_cvar_bool(name, value)?;
                    self.sync_post_effects();
                    self.mark_config_dirty();
                    self.publish_ui();
                    Ok(())
                }
            }
        ]
    }
}

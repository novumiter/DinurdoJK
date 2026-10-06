//! Race ghost cvars: declarative runtime and config bindings.
use super::cvar_prelude::*;

app_cvars! {
    APP_CG_RGHOSTALPHA {
        runtime: CvarDef::new(
                "cg_rGhostAlpha",
                "",
                "0.02..1",
                "Opacity of /rGhost recorded race players.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_cg_rghostalpha(app)),
                |app, value| {
                    App::cvar_set_cg_rghostalpha(app, APP_CG_RGHOSTALPHA.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_cg_rghostalpha, None),
        config: CvarBinding {
                load_presentation: Some(|settings, _name, value| {
                    let finite = || {
                        value
                            .parse::<f32>()
                            .ok()
                            .filter(|number| number.is_finite())
                    };
                    {
                        settings.race_ghost_alpha = finite()
                            .unwrap_or(settings.race_ghost_alpha)
                            .clamp(0.02, 1.0)
                    }
                }),
                archive: Some((281, |state| {
                    let presentation = state.presentation;
                    format!("{:.3}", presentation.race_ghost_alpha)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_cg_rghostalpha(&self) -> String {
                    format!("{:.3}", self.race_ghost_alpha)
                }
                fn cvar_set_cg_rghostalpha(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.race_ghost_alpha = Self::parse_cvar_finite(name, value)?.clamp(0.02, 1.0);
                        if let Some(session) = self.game_session.as_mut() {
                            session.race_ghost_alpha = self.race_ghost_alpha;
                        }
                        self.mark_config_dirty();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_CG_RGHOSTDEMOBASEURL {
        runtime: CvarDef::new(
                "cg_rGhostDemoBaseUrl",
                "",
                "URL",
                "Base URL of the public race-demo archive; the client reads its /index JSON catalog.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_cg_rghostdemobaseurl(app)),
                |app, value| {
                    App::cvar_set_cg_rghostdemobaseurl(
                        app,
                        APP_CG_RGHOSTDEMOBASEURL.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_cg_rghostdemobaseurl, None),
        config: CvarBinding {
                load_presentation: Some(|settings, _name, value| {
                    if !value.trim().is_empty() {
                        settings.race_ghost_demo_base_url = value.trim().to_owned();
                    }
                }),
                archive: Some((286, |state| {
                    let presentation = state.presentation;
                    format!("{}", presentation.race_ghost_demo_base_url)
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_cg_rghostdemobaseurl(&self) -> String {
                    self.race_ghost_demo_base_url.clone()
                }
                fn cvar_set_cg_rghostdemobaseurl(
                    &mut self,
                    _name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    {
                        self.set_race_ghost_demo_base_url(value)?;
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_CG_RGHOSTDISTANCEDELTA {
        runtime: CvarDef::new(
                "cg_rGhostDistanceDelta",
                "",
                "custom",
                "Controls cg_rGhostDistanceDelta.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_cg_rghostdistancedelta(app)),
                |app, value| {
                    App::cvar_set_cg_rghostdistancedelta(
                        app,
                        APP_CG_RGHOSTDISTANCEDELTA.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_cg_rghostdistancedelta, None),
        config: CvarBinding {
                load_presentation: Some(|settings, _name, value| {
                    {
                        settings.race_ghost_distance_delta =
                            parse_bool(value).unwrap_or(settings.race_ghost_distance_delta)
                    }
                }),
                archive: Some((285, |state| {
                    let presentation = state.presentation;
                    format!("{}", u8::from(presentation.race_ghost_distance_delta))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_cg_rghostdistancedelta(&self) -> String {
                    bool_value(self.race_ghost_distance_delta)
                }
                fn cvar_set_cg_rghostdistancedelta(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    {
                        self.race_ghost_distance_delta = Self::parse_cvar_bool(name, value)?;
                        self.mark_config_dirty();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_CG_RGHOSTNAME {
        runtime: CvarDef::new(
                "cg_rGhostName",
                "",
                "custom",
                "Controls cg_rGhostName.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_cg_rghostname(app)),
                |app, value| {
                    App::cvar_set_cg_rghostname(app, APP_CG_RGHOSTNAME.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_cg_rghostname, None),
        config: CvarBinding {
                load_presentation: Some(|settings, _name, value| {
                    {
                        settings.race_ghost_name = parse_bool(value).unwrap_or(settings.race_ghost_name)
                    }
                }),
                archive: Some((282, |state| {
                    let presentation = state.presentation;
                    format!("{}", u8::from(presentation.race_ghost_name))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_cg_rghostname(&self) -> String {
                    bool_value(self.race_ghost_name)
                }
                fn cvar_set_cg_rghostname(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.race_ghost_name = Self::parse_cvar_bool(name, value)?;
                        self.mark_config_dirty();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_CG_RGHOSTTRAIL {
        runtime: CvarDef::new(
                "cg_rGhostTrail",
                "",
                "custom",
                "Controls cg_rGhostTrail.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_cg_rghosttrail(app)),
                |app, value| {
                    App::cvar_set_cg_rghosttrail(app, APP_CG_RGHOSTTRAIL.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_cg_rghosttrail, None),
        config: CvarBinding {
                load_presentation: Some(|settings, _name, value| {
                    {
                        settings.race_ghost_trail = parse_bool(value).unwrap_or(settings.race_ghost_trail)
                    }
                }),
                archive: Some((283, |state| {
                    let presentation = state.presentation;
                    format!("{}", u8::from(presentation.race_ghost_trail))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_cg_rghosttrail(&self) -> String {
                    bool_value(self.race_ghost_trail)
                }
                fn cvar_set_cg_rghosttrail(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.race_ghost_trail = Self::parse_cvar_bool(name, value)?;
                        self.mark_config_dirty();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_CG_RGHOSTVELOCITYDELTA {
        runtime: CvarDef::new(
                "cg_rGhostVelocityDelta",
                "",
                "custom",
                "Controls cg_rGhostVelocityDelta.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_cg_rghostvelocitydelta(app)),
                |app, value| {
                    App::cvar_set_cg_rghostvelocitydelta(
                        app,
                        APP_CG_RGHOSTVELOCITYDELTA.entry.name,
                        &value.to_string(),
                    )
                },
            )
            .with_text_setter(App::cvar_set_cg_rghostvelocitydelta, None),
        config: CvarBinding {
                load_presentation: Some(|settings, _name, value| {
                    {
                        settings.race_ghost_velocity_delta =
                            parse_bool(value).unwrap_or(settings.race_ghost_velocity_delta)
                    }
                }),
                archive: Some((284, |state| {
                    let presentation = state.presentation;
                    format!("{}", u8::from(presentation.race_ghost_velocity_delta))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_cg_rghostvelocitydelta(&self) -> String {
                    bool_value(self.race_ghost_velocity_delta)
                }
                fn cvar_set_cg_rghostvelocitydelta(
                    &mut self,
                    name: &str,
                    value: &str,
                ) -> Result<(), String> {
                    {
                        self.race_ghost_velocity_delta = Self::parse_cvar_bool(name, value)?;
                        self.mark_config_dirty();
                    }
                    Ok(())
                }
            }
        ]
    }
}

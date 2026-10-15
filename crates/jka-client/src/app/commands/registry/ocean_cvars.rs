//! Ocean cvars: declarative runtime and config bindings.
use super::cvar_prelude::*;

app_cvars! {
    APP_CG_WATERWAKE {
        runtime: CvarDef::new("cg_waterwake", "1", "0|1|2",
            "0 off; 1 original JKA water wake on vanilla water; 2 enhanced vanilla wake and enabled Godot water interaction.",
            CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.water_wake.to_string()),
            |app, value| App::cvar_set_water_wake(app, APP_CG_WATERWAKE.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_water_wake, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(mode) = value.trim().parse::<u8>() {
                    settings.water_wake = mode.min(2);
                }
            }),
            archive: Some((25007, |state| state.video.water_wake.to_string())),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_water_wake(&mut self, name: &str, value: &str) -> Result<(), String> {
                let mode: u8 = value.trim().parse()
                    .map_err(|_| format!("{name} expects 0, 1 or 2"))?;
                if mode > 2 { return Err(format!("{name} expects 0, 1 or 2")); }
                self.video.water_wake = mode;
                if let Some(session) = self.game_session.as_mut() {
                    session.water_wake_mode = mode;
                }
                // The stored r_oceanInteraction preference remains untouched.
                // Only the renderer-facing copy is gated by mode 2.
                self.commit_ocean_settings();
                Ok(())
            }
        }]
    }
    APP_R_OCEANSKYEXTEND {
        runtime: CvarDef::new("r_oceanSkyExtend", "1", "0|1",
            "Camera-relative promoted clipmap beyond qualifying sky walls (default on); no infinite plane or coastal changes.",
            CvarType::Bool, CvarFlags::NONE, LatchScope::None,
            |app: &App| CvarValue::Bool(app.ocean_sky_extend),
            |app, value| {
                let CvarValue::Bool(enabled) = value else { unreachable!() };
                if app.ocean_sky_extend != enabled {
                    app.ocean_sky_extend = enabled;
                    app.render_command(RenderCommand::SetOceanSkyExtend(enabled));
                }
                Ok(())
            }),
        config: CvarBinding::NONE,
        implementations: []
    }
    APP_R_COASTAL {
        runtime: CvarDef::new("r_coastal", "0", "0|1",
            "Opt-in GPU shallow-bank solver for one promoted pool; r_coastalPool selects its 1-based pool number (0 automatic).",
            CvarType::Bool, CvarFlags::NONE, LatchScope::None,
            |app: &App| CvarValue::Bool(app.coastal_enabled),
            |app, value| {
                let CvarValue::Bool(enabled)=value else {unreachable!()};
                app.coastal_enabled=enabled;app.coastal_live_sent=None;Ok(())
            }),
        config: CvarBinding::NONE,
        implementations: []
    }
    APP_R_COASTALPOOL {
        runtime: CvarDef::new("r_coastalPool", "0", "0..8",
            "Live coastal pool: 0 selects an eligible shallow-bank pool automatically; 1..8 match the debug Pool numbers.",
            CvarType::Int {min:0,max:8}, CvarFlags::NONE, LatchScope::None,
            |app: &App| CvarValue::Int(app.coastal_pool.into()),
            |app, value| {
                let CvarValue::Int(pool)=value else {unreachable!()};
                app.coastal_pool=pool as u8;app.coastal_live_sent=None;Ok(())
            }),
        config: CvarBinding::NONE,
        implementations: []
    }
    APP_R_COASTALDEBUG {
        runtime: CvarDef::new(
                "r_coastalDebug",
                "0",
                "0..3",
                "Phase 2.5 promoted-water evaluation: 0 off, 1 treatment tint, 2 boundary faces, 3 bathymetry/depth cells.",
                CvarType::Int { min: 0, max: 3 },
                CvarFlags::NONE,
                LatchScope::None,
                |app: &App| CvarValue::Int(app.coastal_debug_mode.into()),
                |app, value| {
                    let CvarValue::Int(mode) = value else { unreachable!() };
                    app.coastal_debug_mode = mode as u8;
                    app.coastal_debug_sent = None;
                    Ok(())
                },
            ),
        config: CvarBinding::NONE,
        implementations: []
    }
    // A single switch for the opt-in visual corrections; does not mutate
    // the individual A/B CVar values or the simulation state.
    APP_R_OCEANVISUALFIXES {
        runtime: CvarDef::new("r_oceanVisualFixes", "0", "0..2",
            "0 original; 1 balanced whitecaps + lit foam + promoted-water body; 2 stronger Godot-like look. No FFT reset.",
            CvarType::Int { min: 0, max: 2 }, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::Int(app.video.ocean_settings.visual_fixes as i32),
            |app, value| {
                let CvarValue::Int(v) = value else { unreachable!() };
                app.video.ocean_settings.visual_fixes = v as u8;
                app.commit_ocean_settings();
                Ok(())
            }),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<u8>() { settings.ocean_settings.visual_fixes = v.min(2); }
            }),
            archive: Some((25018, |state| state.video.ocean_settings.visual_fixes.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANFOAMSOURCE {
        runtime: CvarDef::new("r_oceanFoamSource", "0", "0..2",
            "FFT Jacobian foam generation: 0 original, 1 slightly earlier crests and 1.3x growth, 2 stronger earlier crests and 1.6x growth. Bundle selects minimum level.",
            CvarType::Int { min: 0, max: 2 }, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::Int(app.video.ocean_settings.foam_source_mode as i32),
            |app, value| {
                let CvarValue::Int(v) = value else { unreachable!() };
                app.video.ocean_settings.foam_source_mode = v as u8;
                app.commit_ocean_settings();
                Ok(())
            }),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<u8>() { settings.ocean_settings.foam_source_mode = v.min(2); }
            }),
            archive: Some((25019, |state| state.video.ocean_settings.foam_source_mode.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    // Which swell controls follow the master amplitude (see ocean::links).
    APP_R_OCEANLINKS {
        runtime: CvarDef::new("r_oceanLinks", "127", "0..127",
            "Bitmask of ocean controls that follow the swell amplitude: 1 wavelength, 2 speed, 4 choppiness, 8 wind chop, 16 foam, 32 spray, 64 roughness. 127 all linked (default), 0 none.",
            CvarType::Int { min: 0, max: 127 }, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::Int(app.video.ocean_settings.links as i32),
            |app, value| {
                let CvarValue::Int(v) = value else { unreachable!() };
                app.video.ocean_settings.links = v as u8;
                app.commit_ocean_settings();
                Ok(())
            }),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<u8>() { settings.ocean_settings.links = v & crate::ocean::links::ALL; }
            }),
            archive: Some((25020, |state| state.video.ocean_settings.links.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    // Independent surface-shading A/B controls. Defaults retain the current look.
    APP_R_OCEANFOAMDETAIL {
        runtime: CvarDef::new("r_oceanFoamDetail", "1", "0|1",
            "0 keep whitecaps when normals LOD out, 1 (default) sample all cascades at any distance like Godot and keep far normals.",
            CvarType::Int { min: 0, max: 1 }, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::Int(app.video.ocean_settings.foam_detail_mode as i32),
            |app, value| {
                let CvarValue::Int(v) = value else { unreachable!() };
                app.video.ocean_settings.foam_detail_mode = v as u8;
                app.commit_ocean_settings();
                Ok(())
            }),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<u8>() { settings.ocean_settings.foam_detail_mode = v.min(1); }
            }),
            archive: Some((25010, |state| state.video.ocean_settings.foam_detail_mode.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANFOAMACTIVITY {
        runtime: CvarDef::new("r_oceanFoamActivity", "0", "0..3",
            "Promoted water foam attenuation: 0 original, 1 omit texture heuristic, 2 omit amplitude gate, 3 omit both. No FFT reset.",
            CvarType::Int { min: 0, max: 3 }, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::Int(app.video.ocean_settings.foam_activity_mode as i32),
            |app, value| {
                let CvarValue::Int(v) = value else { unreachable!() };
                app.video.ocean_settings.foam_activity_mode = v as u8;
                app.commit_ocean_settings();
                Ok(())
            }),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<u8>() { settings.ocean_settings.foam_activity_mode = v.min(3); }
            }),
            archive: Some((25011, |state| state.video.ocean_settings.foam_activity_mode.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANFOAMGAIN {
        runtime: CvarDef::new("r_oceanFoamGain", "1", "0..16",
            "Post-simulation breaker foam strength before smoothstep. 1 unchanged, 4 or 8 shows marginal foam.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.foam_gain.to_string()),
            |app, value| App::cvar_set_ab_foam_gain(app, APP_R_OCEANFOAMGAIN.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ab_foam_gain, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() {
                    if v.is_finite() { settings.ocean_settings.foam_gain = v.clamp(0.0, 16.0); }
                }
            }),
            archive: Some((25012, |state| state.video.ocean_settings.foam_gain.to_string())),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_ab_foam_gain(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.ocean_settings.foam_gain = Self::parse_cvar_finite(name, value)?.clamp(0.0, 16.0);
                self.commit_ocean_settings();
                Ok(())
            }
        }]
    }
    APP_R_OCEANCOLORMODE {
        runtime: CvarDef::new("r_oceanColorMode", "0", "0..3",
            "Color A/B: 0 original, 1 override authored base with configured ocean color, 2 opaque lit water, 3 both.",
            CvarType::Int { min: 0, max: 3 }, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::Int(app.video.ocean_settings.color_mode as i32),
            |app, value| {
                let CvarValue::Int(v) = value else { unreachable!() };
                app.video.ocean_settings.color_mode = v as u8;
                app.commit_ocean_settings();
                Ok(())
            }),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<u8>() { settings.ocean_settings.color_mode = v.min(3); }
            }),
            archive: Some((25014, |state| state.video.ocean_settings.color_mode.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANFOAMBRIGHTNESS {
        runtime: CvarDef::new("r_oceanFoamBrightness", "1", "0..8",
            "Multiplier on linear foam RGB in ocean surface illumination, 1 unchanged.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.foam_brightness.to_string()),
            |app, value| App::cvar_set_ab_foam_brightness(app, APP_R_OCEANFOAMBRIGHTNESS.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ab_foam_brightness, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() {
                    if v.is_finite() { settings.ocean_settings.foam_brightness = v.clamp(0.0, 8.0); }
                }
            }),
            archive: Some((25015, |state| state.video.ocean_settings.foam_brightness.to_string())),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_ab_foam_brightness(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.ocean_settings.foam_brightness = Self::parse_cvar_finite(name, value)?.clamp(0.0, 8.0);
                self.commit_ocean_settings();
                Ok(())
            }
        }]
    }
    APP_R_OCEANSUNGAIN {
        runtime: CvarDef::new("r_oceanSunGain", "1", "0..8",
            "Multiplier on direct sun diffuse/specular contribution (not sky ambient), 1 unchanged.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.sun_gain.to_string()),
            |app, value| App::cvar_set_ab_sun_gain(app, APP_R_OCEANSUNGAIN.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ab_sun_gain, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() {
                    if v.is_finite() { settings.ocean_settings.sun_gain = v.clamp(0.0, 8.0); }
                }
            }),
            archive: Some((25016, |state| state.video.ocean_settings.sun_gain.to_string())),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_ab_sun_gain(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.ocean_settings.sun_gain = Self::parse_cvar_finite(name, value)?.clamp(0.0, 8.0);
                self.commit_ocean_settings();
                Ok(())
            }
        }]
    }
    APP_R_OCEANFOAMDEBUG {
        runtime: CvarDef::new("r_oceanFoamDebug", "0", "0..3",
            "Foam diagnostic: 0 shaded water, 1 raw sampled FFT foam, 2 after promoted gates, 3 final breaker + windrows.",
            CvarType::Int { min: 0, max: 3 }, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::Int(app.video.ocean_settings.foam_debug as i32),
            |app, value| {
                let CvarValue::Int(v) = value else { unreachable!() };
                app.video.ocean_settings.foam_debug = v as u8;
                app.commit_ocean_settings();
                Ok(())
            }),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<u8>() { settings.ocean_settings.foam_debug = v.min(3); }
            }),
            archive: Some((25017, |state| state.video.ocean_settings.foam_debug.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANINTERACTION {
        runtime: CvarDef::new("r_oceanInteraction", "", "0|1",
            "Nearby player water deformation; inherits r_oceanUpdates.", CvarType::String,
            CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(bool_value(app.video.ocean_settings.interaction)),
            |app, value| App::cvar_set_ocean_interaction(app, APP_R_OCEANINTERACTION.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                settings.ocean_settings.interaction = parse_bool(value).unwrap_or(settings.ocean_settings.interaction);
            }),
            archive: Some((25000, |state| bool_value(state.video.ocean_settings.interaction))),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_ocean_interaction(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.ocean_settings.interaction = Self::parse_cvar_bool(name, value)?;
                self.commit_ocean_settings(); Ok(())
            }
        }]
    }
    APP_R_OCEANINTERACTIONSTRENGTH {
        runtime: CvarDef::new("r_oceanInteractionStrength", "", "nonnegative finite number",
            "Nearby water interaction force scale.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.interaction_strength.to_string()),
            |app, value| App::cvar_set_ocean_interaction_strength(app, APP_R_OCEANINTERACTIONSTRENGTH.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction_strength, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() { if v.is_finite() { settings.ocean_settings.interaction_strength = v.max(0.0); } }
            }),
            archive: Some((25001, |state| state.video.ocean_settings.interaction_strength.to_string())),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_ocean_interaction_strength(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.ocean_settings.interaction_strength = Self::parse_cvar_finite(name,value)?.max(0.0);
                self.commit_ocean_settings(); Ok(())
            }
        }]
    }
    APP_R_OCEANINTERACTIONFX {
        runtime: CvarDef::new("r_oceanInteractionFx", "", "0..5",
            "Player water FX: 0 off, 1 entry droplets, 2 wake spray, 3 both, 4 entry jets, 5 all.", CvarType::String,
            CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.interaction_splash_mode.to_string()),
            |app, value| App::cvar_set_ocean_interaction_fx(app, APP_R_OCEANINTERACTIONFX.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction_fx, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(mode) = value.parse::<u8>() {
                    settings.ocean_settings.interaction_splash_mode = mode.min(5);
                }
            }),
            archive: Some((25006, |state| state.video.ocean_settings.interaction_splash_mode.to_string())),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_ocean_interaction_fx(&mut self, name: &str, value: &str) -> Result<(), String> {
                let mode = value.parse::<u8>()
                    .map_err(|_| format!("{name} must be in the range 0..5"))?;
                if mode > 5 {
                    return Err(format!("{name} must be in the range 0..5"));
                }
                self.video.ocean_settings.interaction_splash_mode = mode;
                self.commit_ocean_settings();
                Ok(())
            }
        }]
    }
    APP_R_OCEANINTERACTIONLIFETIME {
        runtime: CvarDef::new("r_oceanInteractionLifetime", "", "0.1..60 seconds",
            "Seconds after the last moving input before retiring the local field.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.interaction_lifetime.to_string()),
            |app, value| App::cvar_set_ocean_interaction_falloff(app, APP_R_OCEANINTERACTIONLIFETIME.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction_falloff, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() { if v.is_finite() { settings.ocean_settings.interaction_lifetime = v.clamp(0.1,60.0); } }
            }),
            archive: Some((25002, |state| state.video.ocean_settings.interaction_lifetime.to_string())),
            ..CvarBinding::NONE
        },
        implementations: [impl App {
            fn cvar_set_ocean_interaction_falloff(&mut self, name: &str, value: &str) -> Result<(), String> {
                let v = Self::parse_cvar_finite(name,value)?;
                match name.to_ascii_lowercase().as_str() {
                    "r_oceaninteractionlifetime" => self.video.ocean_settings.interaction_lifetime = v.clamp(0.1,60.0),
                    "r_oceaninteractionfade" => self.video.ocean_settings.interaction_fade = v.clamp(0.0,60.0),
                    "r_oceaninteractiondamping" => self.video.ocean_settings.interaction_damping = v.max(0.0),
                    "r_oceaninteractionedgefade" => self.video.ocean_settings.interaction_edge_fade = v.clamp(0.0,256.0),
                    _ => unreachable!(),
                }
                self.commit_ocean_settings(); Ok(())
            }
        }]
    }
    APP_R_OCEANINTERACTIONFADE {
        runtime: CvarDef::new("r_oceanInteractionFade", "", "0..60 seconds",
            "Duration of the final smooth fade before field retirement; zero disables temporal fading.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.interaction_fade.to_string()),
            |app, value| App::cvar_set_ocean_interaction_falloff(app, APP_R_OCEANINTERACTIONFADE.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction_falloff, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() { if v.is_finite() { settings.ocean_settings.interaction_fade = v.clamp(0.0,60.0); } }
            }),
            archive: Some((25003, |state| state.video.ocean_settings.interaction_fade.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANINTERACTIONDAMPING {
        runtime: CvarDef::new("r_oceanInteractionDamping", "", "nonnegative finite number",
            "Velocity decay per second; zero disables bulk damping; height decay is 0.175 times this rate.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.interaction_damping.to_string()),
            |app, value| App::cvar_set_ocean_interaction_falloff(app, APP_R_OCEANINTERACTIONDAMPING.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction_falloff, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() { if v.is_finite() { settings.ocean_settings.interaction_damping = v.max(0.0); } }
            }),
            archive: Some((25004, |state| state.video.ocean_settings.interaction_damping.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANINTERACTIONEDGEFADE {
        runtime: CvarDef::new("r_oceanInteractionEdgeFade", "", "0..256 world units",
            "Width of the local-window edge fade; sponge width is 1.5 times this value; zero disables both.", CvarType::String, CvarFlags::ARCHIVE, LatchScope::None,
            |app: &App| CvarValue::String(app.video.ocean_settings.interaction_edge_fade.to_string()),
            |app, value| App::cvar_set_ocean_interaction_falloff(app, APP_R_OCEANINTERACTIONEDGEFADE.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction_falloff, None),
        config: CvarBinding {
            load_video: Some(|settings, _name, value, _context| {
                if let Ok(v) = value.parse::<f32>() { if v.is_finite() { settings.ocean_settings.interaction_edge_fade = v.clamp(0.0,256.0); } }
            }),
            archive: Some((25005, |state| state.video.ocean_settings.interaction_edge_fade.to_string())),
            ..CvarBinding::NONE
        },
        implementations: []
    }
    APP_R_OCEANINTERACTIONDEBUG {
        runtime: CvarDef::new("r_oceanInteractionDebug", "", "0|1",
            "Show nearby interaction height (red/blue) and slopes (green).", CvarType::String,
            CvarFlags::NONE, LatchScope::None,
            |app: &App| CvarValue::String(bool_value(app.video.ocean_settings.interaction_debug)),
            |app, value| App::cvar_set_ocean_interaction_debug(app, APP_R_OCEANINTERACTIONDEBUG.entry.name, &value.to_string()))
            .with_text_setter(App::cvar_set_ocean_interaction_debug, None),
        config: CvarBinding::NONE,
        implementations: [impl App {
            fn cvar_set_ocean_interaction_debug(&mut self, name: &str, value: &str) -> Result<(), String> {
                self.video.ocean_settings.interaction_debug = Self::parse_cvar_bool(name,value)?;
                self.commit_ocean_settings(); Ok(())
            }
        }]
    }
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
                        if let Some(session) = self.game_session.as_mut() {
                            session.ocean_interaction_enabled = self.video.ocean && self.video.ocean_settings.interaction && self.video.water_wake == 2;
                        }
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

    APP_R_OCEANTRANSPARENCY {
        runtime: CvarDef::new(
                "r_oceanTransparency",
                "",
                "1..65536",
                "Water transparency: how far light travels through the water before it is absorbed, in game units. Lower is murkier, above and below the surface.",
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

    APP_R_OCEANUNDERWATERCULL {
        runtime: CvarDef::new(
                "r_oceanUnderwaterCull",
                "",
                "0..32",
                "Cull geometry that water absorption hides, as an absorption-distance multiple (floored at 2): everything past it (min 256) when underwater, and anything under that much water when viewed from above. Zero disables.",
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

    APP_R_OCEANOPTICSCULL {
        runtime: CvarDef::new(
                "r_oceanOpticsCull",
                "",
                "0|1",
                "Skip the underwater optics capture entirely when the camera is dry and no ocean surface is on screen. Nothing can sample it then.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanopticscull(app)),
                |app, value| {
                    App::cvar_set_r_oceanopticscull(app, APP_R_OCEANOPTICSCULL.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanopticscull, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.optics.cull =
                        parse_bool(value).unwrap_or(settings.ocean_settings.optics.cull);
                }),
                archive: Some((10424, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.ocean_settings.optics.cull))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanopticscull(&self) -> String {
                    bool_value(self.video.ocean_settings.optics.cull)
                }
                fn cvar_set_r_oceanopticscull(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.optics.cull = Self::parse_cvar_bool(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANOPTICSSCISSOR {
        runtime: CvarDef::new(
                "r_oceanOpticsScissor",
                "",
                "0|1",
                "Restrict the underwater optics capture to the screen rectangle the visible water covers instead of the whole framebuffer.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanopticsscissor(app)),
                |app, value| {
                    App::cvar_set_r_oceanopticsscissor(app, APP_R_OCEANOPTICSSCISSOR.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanopticsscissor, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.optics.scissor =
                        parse_bool(value).unwrap_or(settings.ocean_settings.optics.scissor);
                }),
                archive: Some((10425, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.ocean_settings.optics.scissor))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanopticsscissor(&self) -> String {
                    bool_value(self.video.ocean_settings.optics.scissor)
                }
                fn cvar_set_r_oceanopticsscissor(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.optics.scissor = Self::parse_cvar_bool(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

    APP_R_OCEANSIMWHENHIDDEN {
        runtime: CvarDef::new(
                "r_oceanSimWhenHidden",
                "",
                "0|1",
                "Keep advancing the FFT wave simulation even while no water is visible. Off freezes the field, which costs nothing and cannot be seen.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceansimwhenhidden(app)),
                |app, value| {
                    App::cvar_set_r_oceansimwhenhidden(app, APP_R_OCEANSIMWHENHIDDEN.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceansimwhenhidden, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.simulate_when_hidden =
                        parse_bool(value).unwrap_or(settings.ocean_settings.simulate_when_hidden);
                }),
                archive: Some((10426, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.ocean_settings.simulate_when_hidden))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceansimwhenhidden(&self) -> String {
                    bool_value(self.video.ocean_settings.simulate_when_hidden)
                }
                fn cvar_set_r_oceansimwhenhidden(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.simulate_when_hidden = Self::parse_cvar_bool(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }


    APP_R_OCEANOPTICSMASKS {
        runtime: CvarDef::new(
                "r_oceanOpticsMasks",
                "",
                "0|1",
                "Honour water outline masks in the underwater optics capture. Off is visually wrong on hollow water; it exists to measure the mask cost.",
                CvarType::String,
                CvarFlags::ARCHIVE,
                LatchScope::None,
                |app| CvarValue::String(App::cvar_get_r_oceanopticsmasks(app)),
                |app, value| {
                    App::cvar_set_r_oceanopticsmasks(app, APP_R_OCEANOPTICSMASKS.entry.name, &value.to_string())
                },
            )
            .with_text_setter(App::cvar_set_r_oceanopticsmasks, None),
        config: CvarBinding {
                load_video: Some(|settings, _name, value, _context| {
                    settings.ocean_settings.optics.masks =
                        parse_bool(value).unwrap_or(settings.ocean_settings.optics.masks);
                }),
                archive: Some((10427, |state| {
                    let settings = state.video;
                    format!("{}", u8::from(settings.ocean_settings.optics.masks))
                })),
                ..CvarBinding::NONE
            },
        implementations: [
            impl App {
                fn cvar_get_r_oceanopticsmasks(&self) -> String {
                    bool_value(self.video.ocean_settings.optics.masks)
                }
                fn cvar_set_r_oceanopticsmasks(&mut self, name: &str, value: &str) -> Result<(), String> {
                    {
                        self.video.ocean_settings.optics.masks = Self::parse_cvar_bool(name, value)?;
                        self.commit_ocean_settings();
                    }
                    Ok(())
                }
            }
        ]
    }

}

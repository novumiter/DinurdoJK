//! Declarative command handlers.
use crate::app::{demo_recording_timestamp, App, GameSession, RenderCommand, SessionPhase};

use super::{CommandDef, CommandResult, CommandScope};
commands! {
    cmd_coastal_debug ["coastal_debug", ] ("coastal_debug", "Print background BSP bathymetry results and the interactive debug report path.", CommandScope::Always) |self, command| {
        if command.split_whitespace().count() != 1 {
            self.push_console_line("usage: coastal_debug".to_owned());
            return CommandResult::Handled;
        }
        let lines = self.prepared_map_cache.as_ref()
            .and_then(|cache| cache.map.coastal_bathymetry.as_ref())
            .map(|job| job.report_lines())
            .unwrap_or_else(|| vec!["No coastal geometry job for this map (enable ocean and load a BSP with promoted water).".to_owned()]);
        for line in lines { self.push_console_line(line); }
        CommandResult::Handled
    }

    cmd_configstrings ["configstrings", ] ("configstrings", "Print the active client configstrings list.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                // TaystJK CL_Configstrings_f: only an active CGame owns the
                // printable gameState table, and empty configstrings are skipped.
                let active = self
                    .game_session
                    .as_ref()
                    .filter(|session| session.phase == SessionPhase::Playing)
                    .map(|session| {
                        session
                            .client_game
                            .configstrings()
                            .iter()
                            .filter(|(_, value)| !value.is_empty())
                            .map(|(&index, value)| {
                                (index, String::from_utf8_lossy(value).into_owned())
                            })
                            .collect::<Vec<_>>()
                    });
                let Some(strings) = active else {
                    self.push_console_line("Not connected to a server.".to_owned());
                    return CommandResult::Handled;
                };
                for (index, value) in strings {
                    self.push_console_line(format!("{index:4}: {value}"));
                }
                CommandResult::Handled
            }
            [_verb, ..] => {
                self.push_console_line("^3usage:^7 configstrings".to_owned());
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_version ["version", ] ("version", "Print the DinurdoJK version and UTC compile time embedded in this executable.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                self.push_console_line(format!(
                    "^2DinurdoJK^7 {}  ^8compiled^7 {}",
                    env!("CARGO_PKG_VERSION"),
                    env!("DINURDOJK_BUILD_UTC")
                ));
                CommandResult::Handled
            }
            [_verb, ..] => {
                self.push_console_line("^3usage:^7 version".to_owned());
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_predsettings ["predsettings", ] ("predsettings", "Print the pmove settings the predictor is running with: race/style, pmove_float/fixed/msec, velocity snapping, command chopping.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                let command_rate = format!(
                    "{} ({:.2} ms usercmd gaps)",
                    self.network.command_rate,
                    1000.0 / f64::from(self.network.command_rate.max(15))
                );
                for line in self.predictor.regime_report(&command_rate) {
                    self.push_console_line(line);
                }
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_hitchmark ["hitchmark", ] ("hitchmark [label]", "Write the last ~5 s of per-frame prediction/view state to <game>/hitch/*.csv. Bind it and tap it right after a hitch (needs cg_hitchrecord 1).", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                self.hitch_mark(None);
                CommandResult::Handled
            }
            [_verb, label] => {
                self.hitch_mark(Some(label));
                CommandResult::Handled
            }
            [_verb, ..] => {
                self.push_console_line("^3usage:^7 hitchmark [label]".to_owned());
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_render_perf_capture ["render_perf_capture", "renderperfcapture"] ("render_perf_capture [frames=600] [label]", "Record per-frame renderer CPU stage timings to CSV/JSON after a 2s delay and 120 warmup frames. Opt-in; no GPU stalls.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        if words.len() > 3 {
            self.push_console_line("^3usage:^7 render_perf_capture [frames=600] [label]".to_owned());
            return CommandResult::Handled;
        }
        let frames = if let Some(arg) = words.get(1) {
            match arg.parse::<usize>() {
                Ok(value) if (30..=6000).contains(&value) => value,
                _ => {
                    self.push_console_line("^1render_perf_capture:^7 frames must be 30..6000".to_owned());
                    return CommandResult::Handled;
                }
            }
        } else { 600 };
        let label: String = words.get(2).copied().unwrap_or("")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(32)
            .collect();
        let stem_name = if label.is_empty() {
            format!("renderperf-{}", demo_recording_timestamp())
        } else {
            format!("renderperf-{}-{label}", demo_recording_timestamp())
        };
        let stem = self.game_write_path(std::path::PathBuf::from("renderperf").join(stem_name));
        let result = self.render.as_ref().ok_or("renderer is unavailable")
            .and_then(|render| render.start_render_perf_capture(stem.clone(), frames));
        match result {
            Ok(()) => self.push_console_line(format!(
                "^2RENDER PERF ARMED^7 {frames} samples after 2s + 120-frame warmup. Close console; output: {}.csv/.json",
                stem.display()
            )),
            Err(error) => self.push_console_line(format!("^1render_perf_capture:^7 {error}")),
        }
        CommandResult::Handled
    }

    cmd_inputdiag ["inputdiag", "framediag"] ("inputdiag [label]", "Arm a 5-second raw-mouse/Win32/event-loop/render capture. Recording starts on the first gameplay mouse movement and stops/writes automatically.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] | [_verb, _] => {
                let label = words.get(1).copied().unwrap_or("");
                let label: String = label
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .take(32)
                    .collect();
                let stem_name = if label.is_empty() {
                    format!("inputdiag-{}", demo_recording_timestamp())
                } else {
                    format!("inputdiag-{}-{label}", demo_recording_timestamp())
                };
                let stem = self.game_write_path(std::path::PathBuf::from("inputdiag").join(stem_name));
                if let Some(parent) = stem.parent() {
                    if let Err(error) = std::fs::create_dir_all(parent) {
                        self.push_console_line(format!(
                            "^1inputdiag:^7 couldn't create {}: {error}",
                            parent.display()
                        ));
                        return CommandResult::Handled;
                    }
                }
                let result = self
                    .render
                    .as_ref()
                    .ok_or("renderer is unavailable")
                    .and_then(|render| render.arm_input_diagnostic(stem));
                match result {
                    Ok(()) => self.push_console_line(
                        "^2INPUT DIAG ARMED^7 - close the console, then move the mouse in steady circles continuously. Capture starts on the first gameplay mouse movement, records 5 seconds, and stops automatically."
                            .to_owned(),
                    ),
                    Err(error) => self.push_console_line(format!("^1inputdiag:^7 {error}")),
                }
                CommandResult::Handled
            }
            [_verb, ..] => {
                self.push_console_line("^3usage:^7 inputdiag [label]".to_owned());
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_cg_eventstats ["cg_eventStats", ] ("cg_eventStats [clear]", "Summarize accepted demo events; use 'cg_eventStats clear' to reset counters.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                let lines = self
                    .game_session
                    .as_ref()
                    .map(GameSession::event_stats_lines)
                    .unwrap_or_else(|| vec!["^3CG EVENT STATS:^7 no demo is playing".into()]);
                for line in lines {
                    self.push_console_line(line);
                }
                CommandResult::Handled
            }
            [_verb, argument] if argument.eq_ignore_ascii_case("clear") => {
                if let Some(playback) = self.game_session.as_mut() {
                    playback.clear_event_stats();
                    self.console_status = "CG EVENT STATS CLEARED".into();
                } else {
                    self.console_status = "NO DEMO IS PLAYING".into();
                }
                CommandResult::Status
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_r_dumpmaterials ["r_dumpmaterials", "dumpmaterials"] ("r_dumpmaterials [argument]", "Dump material diagnostics; optional filter or 'all'.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                self.render_command(RenderCommand::DumpMaterials {
                    filter: None,
                    all: false,
                });
                self.console_status =
                    "MATERIAL DEBUG DUMPED TO TERMINAL (ENHANCED MATERIALS ONLY)".into();
                CommandResult::Status
            }
            [_verb, argument] => {
                self.render_command(RenderCommand::DumpMaterials {
                    filter: (!argument.eq_ignore_ascii_case("all")).then(|| (*argument).to_owned()),
                    all: argument.eq_ignore_ascii_case("all"),
                });
                self.console_status = if argument.eq_ignore_ascii_case("all") {
                    "ALL MATERIAL DEBUG DUMPED TO TERMINAL".into()
                } else {
                    format!("MATERIAL DEBUG FILTER '{argument}' DUMPED TO TERMINAL")
                };
                CommandResult::Status
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_viewpos ["viewpos", ] ("viewpos", "Print the view origin and angles: (x y z) : yaw (pitch p).", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                self.print_viewpos();
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_perfsample ["perfsample", ] ("perfsample [arguments]", "perfsample <seconds> [label]: hold the command buffer, then print the average render FPS.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb, arguments @ ..] => {
                self.begin_perf_sample(arguments);
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_puddle_debug ["puddle_debug", ] ("puddle_debug [mode]", "puddle_debug [0-8]: wetness, puddle-coat, runoff, material and flow diagnostic views; no argument toggles view 1.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        let mode = match words.as_slice() {
            // Bare verb keeps the old toggle so the PUDDLE DEBUG keybind still works.
            [_verb] => u32::from(self.puddle_debug_mode == 0),
            [_verb, argument] => match argument.parse::<u32>() {
                Ok(mode) if mode <= 8 => mode,
                _ => {
                    self.console_status = "puddle_debug: mode must be 0-8".into();
                    return CommandResult::Status;
                }
            },
            _ => return CommandResult::Unhandled,
        };
        self.puddle_debug_mode = mode;
        self.render_command(RenderCommand::SetPuddleDebug(mode));
        self.console_status = match mode {
            0 => "PUDDLE DEBUG OFF".into(),
            1 => "PUDDLE DEBUG 1: coverage (blue=standing water, green=wet film)".into(),
            2 => "PUDDLE DEBUG 2: coat source (red=skybox, green=reflection probe)".into(),
            3 => "PUDDLE DEBUG 3: reflection-probe radiance the coat reflects".into(),
            4 => "PUDDLE DEBUG 4: skybox radiance the coat reflects".into(),
            5 => "PUDDLE DEBUG 5: runoff (red=rivulets, blue=across-slope tilt)".into(),
            6 => "PUDDLE DEBUG 6: material (red=porosity, green=gloss, blue=receiver)".into(),
            7 => "PUDDLE DEBUG 7: runoff albedo darkening at 3x (white = a third darker)".into(),
            _ => "PUDDLE DEBUG 8: flow (red = impossible uphill, green = normal drift, blue = direction)".into(),
        };
        CommandResult::Status
    }

    cmd_puddle_sky ["puddle_sky", ] ("puddle_sky [window] [ceiling]", "puddle_sky <window> <ceiling>: where the puddle coat hands over from the reflection probe to the skybox, and the skybox's largest share.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {}
            [_verb, window, ceiling] => {
                match (window.parse::<f32>(), ceiling.parse::<f32>()) {
                    (Ok(window), Ok(ceiling)) => {
                        self.puddle_sky_blend =
                            (window.clamp(0.02, 1.0), ceiling.clamp(0.0, 1.0));
                        self.render_command(RenderCommand::SetPuddleSkyBlend {
                            window: self.puddle_sky_blend.0,
                            ceiling: self.puddle_sky_blend.1,
                        });
                    }
                    _ => {
                        self.console_status = "puddle_sky: expected two numbers".into();
                        return CommandResult::Status;
                    }
                }
            }
            _ => return CommandResult::Unhandled,
        }
        let (window, ceiling) = self.puddle_sky_blend;
        self.console_status = format!(
            "puddle_sky: window {window:.3} (hands over to skybox by {:.1} degrees), ceiling {ceiling:.3}",
            window.min(1.0).asin().to_degrees()
        );
        CommandResult::Status
    }

    cmd_dev ["dev", ] ("dev", "Toggle the mouse-driven developer radial: Trace, 3D Entities, Entity Viewer, and Remap Shader (planned).", CommandScope::Always) |self, command| {
        match command.split_whitespace().collect::<Vec<_>>().as_slice() {
            [_verb] => {
                self.toggle_dev_radial();
                CommandResult::Handled
            }
            _ => CommandResult::Unhandled,
        }
    }

    cmd_trace ["trace", ] ("trace", "jaPRO: Trace what you are looking at.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                self.trace_surface_center();
                CommandResult::Status
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_trace_clear ["trace_clear", ] ("trace_clear", "Clear the pinned surface inspection and triangle highlight.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                self.clear_surface_inspection();
                CommandResult::Status
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_entities ["entities", "entgraph"] ("entities [targetname|classname]", "Open the 2D entity blueprint: entities [targetname|classname] selects the first match.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb, query @ ..] => {
                let query = query.join(" ");
                self.open_entity_graph((!query.is_empty()).then_some(query.as_str()));
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_uipage ["uipage", ] ("uipage [page]", "Inspect a UI page.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb, page @ ..] => {
                self.open_ui_page(page);
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_fxinfo ["fxinfo", ] ("fxinfo", "Print current effects diagnostics.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                match self.game_session.as_ref() {
                    Some(playback) => {
                        let stats = playback.weapon_fx.stats();
                        self.push_console_line(format!(
                            "^3FX:^7 effects {} | live primitives {} | scheduled {} | dropped {} | unsupported spawns {} | lod culled {} | lod saved {} | surfaces {}",
                            stats.registered,
                            stats.active,
                            stats.scheduled,
                            stats.dropped,
                            stats.unsupported_spawns,
                            stats.lod_culled,
                            stats.lod_saved,
                            playback.fx_surfaces.len(),
                        ));
                    }
                    None => self.push_console_line("^3FX:^7 no active CGame (start a demo first)."),
                }
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

    cmd_soundinfo ["soundinfo", ] ("soundinfo", "Show active output format, device buffer, voice count and pre-limiter overload diagnostics.", CommandScope::Always) |self, command| {
        let words: Vec<_> = command.split_whitespace().collect();
        match words.as_slice() {
            [_verb] => {
                let sound_state = self
                    .game_session
                    .as_ref()
                    .and_then(|playback| playback.sound_presenter.as_ref())
                    .and_then(|sound| sound.info().map(|info| (info, sound.source_rate_summary())));
                if let Some((info, source_rates)) = sound_state {
                    self.push_console_line(format!(
                        "^3AUDIO OUTPUT:^7 {} ch | {} Hz | {} | buffer {}",
                        info.channels, info.sample_rate, info.sample_format, info.buffer_size
                    ));
                    self.push_console_line(format!(
                        "^3AUDIO MIX:^7 voices {} / 32 | loops {} ({} requests) | pre-limit peak {:.3} | overload samples {}",
                        info.active_voices, info.active_loops, info.loop_requests, info.peak_before_limiter, info.overload_samples
                    ));
                    self.push_console_line(format!(
                        "^3AUDIO SOURCES:^7 {} (each retains its decoded rate; mixer converts to {} Hz output)",
                        source_rates, info.sample_rate
                    ));
                    self.push_console_line(format!(
                        "^3AUDIO LEVELS:^7 s_volume {:.3} | s_volumeVoice {:.3} | s_musicvolume {:.3} | s_separation {:.3}",
                        self.audio.effects_volume,
                        self.audio.voice_volume,
                        self.audio.music_volume,
                        self.audio.separation
                    ));
                    self.push_console_line(format!(
                        "^3AUDIO PAN:^7 largest per-frame gain step {:.3} (glided per sample, not stepped)",
                        info.largest_gain_step
                    ));
                    if info.overload_samples > 0 {
                        self.push_console_line(
                            "^3AUDIO NOTE:^7 the game mix exceeded full scale before the output limiter; this is a likely crackle/clipping source.",
                        );
                    }
                } else {
                    self.push_console_line(
                        "^3AUDIO:^7 no active CGame audio backend (start a demo/map presentation first).",
                    );
                }
                CommandResult::Handled
            }
            _ => return CommandResult::Unhandled,
        }
    }

}

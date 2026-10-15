use super::*;
use crate::cvar_compatibility::{assert_snapshot, digest, Fixture, INPUTS};
use std::fs;

#[test]
fn cvar_runtime_setters_and_latches_match_baseline() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let event_loop = winit::event_loop::EventLoop::<crate::runtime::UserEvent>::with_user_event()
        .with_any_thread(true)
        .build()
        .unwrap();
    let fixture = Fixture::new();
    let directory = fixture.0.clone();
    fs::write(directory.join("DinurdoJK.cfg"), "").unwrap();
    let mut app = App::new(
        directory.clone(),
        None,
        crate::scene::MapSource::Bsp("mp/ffa3".into()),
        false,
        event_loop.create_proxy(),
    )
    .unwrap();
    assert_command_encoding_ab_controls(&mut app);
    let names: Vec<_> = include_str!("../../../testdata/cvars/runtime-matrix.txt")
        .lines()
        .map(|row| row.split_whitespace().next().unwrap())
        .collect();
    let mut snapshot = String::new();
    // The frozen runtime baseline includes two removed prediction cvars in its
    // vector of current values. Reconstruct only their historical entries to
    // compare the remaining cvar behavior without rebaselining every hash.
    let mut old_predict_backend = -1i32;
    let mut old_snap_mode = -1i32;
    for name in &names {
        if *name == "cg_predictbackend" {
            for value in INPUTS { old_predict_backend = jka_protocol::commands::atoi(value.as_bytes()).clamp(-1, 1); }
            continue;
        }
        if *name == "cg_snapmode" {
            for value in INPUTS { old_snap_mode = jka_protocol::commands::atoi(value.as_bytes()).clamp(-1, 4); }
            continue;
        }
        snapshot.push_str(name);
        for value in INPUTS {
            app.config_dirty = false;
            app.japro_camera_userinfo_dirty = false;
            app.japro_net_userinfo_dirty = false;
            app.next_command_time = Some(1.0);
            let result = app.set_console_cvar(name, value);
            let values: Vec<_> = names
                .iter()
                .map(|name| match *name {
                    "cg_predictbackend" => Some(old_predict_backend.to_string()),
                    "cg_snapmode" => Some(old_snap_mode.to_string()),
                    _ => app.console_cvar_value(name),
                })
                .collect();
            snapshot.push(' ');
            snapshot.push_str(&digest(&format!(
                "{result:?}|{values:?}|{}|{}|{}|{}",
                app.config_dirty,
                app.japro_camera_userinfo_dirty,
                app.japro_net_userinfo_dirty,
                app.next_command_time.is_some()
            )));
        }
        snapshot.push('\n');
    }
    assert_snapshot("runtime-matrix.txt", &snapshot);
    let mut snapshot = String::new();
    for name in [
        "r_fullscreen",
        "r_backend",
        "r_customwidth",
        "r_customheight",
        "r_pbr",
        "r_genNormalMaps",
        "r_floatLightmap",
        "fs_allowAssetOverrides",
        "r_picmip",
    ] {
        snapshot.push_str(name);
        for value in INPUTS {
            snapshot.push(' ');
            snapshot.push_str(&digest(&format!(
                "{:?}",
                app.normalize_latched_console_value(name, value)
            )));
        }
        snapshot.push('\n');
    }
    assert_snapshot("latches.txt", &snapshot);
    drop(app);
    drop(event_loop);
}

#[test]
fn removed_prediction_controls_are_not_registered() {
    for name in ["cg_predictBackend", "cg_snapMode"] {
        assert!(crate::app::cvar_registry().find(name).is_none(), "{name} must not select physics");
        assert!(crate::net::NetworkSettings::default().cvar_value(name).is_none());
    }
}

#[test]
fn ocean_enable_is_archived_before_apply() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let event_loop = winit::event_loop::EventLoop::<crate::runtime::UserEvent>::with_user_event()
        .with_any_thread(true).build().unwrap();
    let fixture = Fixture::new();
    let path = fixture.0.join("DinurdoJK.cfg");
    fs::write(&path, "seta r_ocean \"0\"\n").unwrap();
    let mut app = App::new(fixture.0.clone(), None,
        crate::scene::MapSource::Bsp("mp/ffa3".into()), false,
        event_loop.create_proxy()).unwrap();
    assert!(!app.applied_ocean);
    // The menu can stage ON while the current world has no ocean resources.
    app.environment_selected = crate::ui::ENV_ROW_OCEAN;
    app.change_environment_setting(1);
    assert!(app.video.ocean);
    app.flush_config();
    assert!(crate::config::load_video_settings(&path).ocean,
        "a pending ocean enable must survive quitting before Apply");
    // The console uses the same saved preference, including immediate OFF.
    app.set_console_cvar("r_ocean", "0").unwrap();
    app.flush_config();
    assert!(!crate::config::load_video_settings(&path).ocean);
    app.set_console_cvar("r_ocean", "1").unwrap();
    app.flush_config();
    assert!(crate::config::load_video_settings(&path).ocean);
}

#[test]
fn stage_collapse_extra_is_live_and_archived() {
    use crate::console::LatchScope;
    let definition = crate::app::cvar_registry()
        .find("r_stageCollapseExtra")
        .expect("renderer stage-collapse cvar registered");
    assert_eq!(definition.entry.latch, LatchScope::None);
    assert!(!crate::ui::VideoSettings::default().stage_collapse_extra);
}

#[test]
fn additive_stage_fusion_is_live_and_archived() {
    use crate::console::{cvars::CvarFlags, LatchScope};
    let definition = crate::app::cvar_registry()
        .find("r_additiveStageFusion")
        .expect("additive-stage fusion cvar registered");
    assert_eq!(definition.entry.latch, LatchScope::None);
    assert!(definition.flags.contains(CvarFlags::ARCHIVE));
    assert!(crate::ui::VideoSettings::default().additive_stage_fusion);
}

fn assert_command_encoding_ab_controls(app: &mut App) {
    use crate::console::{cvars::CvarFlags, LatchScope};
    app.config_dirty = false;
    for (name, baseline, experimental) in [
        ("r_modelStateCache", "0", "1"),
        ("r_dynamicBatchDraws", "1", "0"),
        ("r_worldCompactMinDraws", "1", "2"),
        ("r_modelPrepareThreads", "2", "0"),
        ("r_md3GeometryCache", "1", "0"),
        ("r_modelRenderBundles", "0", "1"),
        ("r_encoderFinishThreads", "2", "0"),
        ("r_modelIndexUploadCache", "0", "1"),
        ("r_ghoul2BatchKeyCache", "1", "0"),
    ] {
        let definition = crate::app::cvar_registry().find(name).unwrap();
        assert_eq!(definition.entry.latch, LatchScope::None);
        assert!(!definition.flags.contains(CvarFlags::ARCHIVE));
        assert_eq!(app.console_cvar_value(name).as_deref(), Some(baseline));
        app.set_console_cvar(name, experimental).unwrap();
        assert_eq!(app.console_cvar_value(name).as_deref(), Some(experimental));
        assert!(!app.config_dirty);
        assert!(app.set_console_cvar(name, "invalid").is_err());
        assert_eq!(app.console_cvar_value(name).as_deref(), Some(experimental));
        app.set_console_cvar(name, baseline).unwrap();
    }
    for value in ["0", "3", "-1", "1.5"] {
        assert!(app.set_console_cvar("r_worldCompactMinDraws", value).is_err());
        assert_eq!(app.video.world_compact_min_draws, 1);
    }
    app.set_console_cvar("r_encoderFinishThreads", "1").unwrap();
    assert_eq!(app.video.encoder_finish_threads, 1);
    app.set_console_cvar("r_encoderFinishThreads", "0").unwrap();
    for value in ["3", "-1", "1.5"] {
        assert!(app.set_console_cvar("r_encoderFinishThreads", value).is_err());
        assert_eq!(app.video.encoder_finish_threads, 0);
    }
    for value in ["3", "8", "-1", "1.5"] {
        assert!(app.set_console_cvar("r_modelPrepareThreads", value).is_err());
        assert_eq!(app.video.model_prepare_threads, 2);
    }
}

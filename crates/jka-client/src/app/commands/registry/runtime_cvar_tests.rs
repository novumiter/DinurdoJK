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
fn stage_collapse_extra_is_live_and_archived() {
    use crate::console::LatchScope;
    let definition = crate::app::cvar_registry()
        .find("r_stageCollapseExtra")
        .expect("renderer stage-collapse cvar registered");
    assert_eq!(definition.entry.latch, LatchScope::None);
    assert!(!crate::ui::VideoSettings::default().stage_collapse_extra);
}

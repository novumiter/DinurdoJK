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
    for name in &names {
        snapshot.push_str(name);
        for value in INPUTS {
            app.config_dirty = false;
            app.japro_camera_userinfo_dirty = false;
            app.japro_net_userinfo_dirty = false;
            app.next_command_time = Some(1.0);
            let result = app.set_console_cvar(name, value);
            let values: Vec<_> = names
                .iter()
                .map(|name| app.console_cvar_value(name))
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

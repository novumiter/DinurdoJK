//! Frozen pre-migration behavior: config parsing/defaults/archives and subsystem setters.
use crate::{config, japro_cg::JaproCgame, net::NetworkSettings};
use std::{fs, path::PathBuf};

pub(crate) const INPUTS: &[&str] = &[
    "",
    "0",
    "1",
    "2",
    "3",
    "7",
    "-0",
    "-1",
    "256",
    "10000000000",
    "0.5",
    "1.9",
    " 1 ",
    "yes",
    "off",
    "NaN",
    "inf",
    "invalid",
    "1 2 3",
    "255 128 0 64",
];
pub(crate) const NAMES: &str = include_str!("testdata/cvars/names.txt");
pub(crate) fn digest(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    // This field was added concurrently after the original settings snapshot.
    let projected = text.replace(", console_notify_lines: 3", "");
    for byte in projected.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
pub(crate) struct Fixture(pub(crate) PathBuf);
impl Fixture {
    pub(crate) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dinurdo-cvar-compat-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(crate) fn assert_snapshot(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/testdata/cvars")
        .join(name);
    let mut expected = fs::read_to_string(&path).unwrap();
    let projected_archive;
    let actual = if name.ends_with(".cfg") {
        projected_archive = actual
            .lines()
            .filter(|line| !line.starts_with("seta con_notifylines "))
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        projected_archive.as_str()
    } else {
        actual
    };
    if name == "config-matrix.txt" {
        expected = expected
            .lines()
            .filter(|line| {
                let name = line.split_whitespace().next().unwrap_or_default();
                name != "r_hdr" && crate::app::cvar_registry().find(name).is_some()
            })
            .map(|line| format!("{line}\n"))
            .collect();
    }
    if name == "changed.cfg" {
        // The removed r_volumetricFog import used to override current r_drawfog.
        expected = expected.replace("seta r_drawfog \"0\"", "seta r_drawfog \"2\"");
    }
    for (row, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
        assert_eq!(a, e, "{name} row {}", row + 1);
    }
    assert_eq!(
        actual.lines().count(),
        expected.lines().count(),
        "{name} row count"
    );
}
#[test]
fn cvar_compatibility_config_input_matrix() {
    let fixture = Fixture::new();
    let path = fixture.0.join("input.cfg");
    let mut actual = String::new();
    for name in NAMES
        .lines()
        .filter(|name| *name != "r_hdr" && crate::app::cvar_registry().find(name).is_some())
    {
        actual.push_str(name);
        for value in INPUTS {
            fs::write(
                &path,
                format!("seta {} \"{value}\"\n", name.to_ascii_uppercase()),
            )
            .unwrap();
            let video = config::load_video_settings(&path);
            let audio = config::load_audio_settings(&path);
            let client = config::load_client_presentation_settings(&path);
            actual.push(' ');
            actual.push_str(&digest(&format!("{video:?}|{audio:?}|{client:?}")));
        }
        actual.push('\n');
    }
    assert_snapshot("config-matrix.txt", &actual);
}
#[test]
fn cvar_compatibility_network_and_japro_setters() {
    let mut actual = String::new();
    for name in NAMES.lines() {
        if NetworkSettings::default().cvar_value(name).is_none()
            && JaproCgame::default().cvar_value(name).is_none()
        {
            continue;
        }
        actual.push_str(name);
        for value in INPUTS {
            let mut network = NetworkSettings::default();
            let result = network.set_cvar(name, value);
            let get = network.cvar_value(name);
            let mut japro = JaproCgame::default();
            let jp_result = japro.set_cvar(name, value);
            let jp_get = japro.cvar_value(name);
            actual.push(' ');
            actual.push_str(&digest(&format!(
                "{result:?}|{get:?}|{network:?}|{jp_result:?}|{jp_get:?}|{japro:?}"
            )));
        }
        actual.push('\n');
    }
    assert_snapshot("subsystem-matrix.txt", &actual);
}
#[test]
fn cvar_compatibility_archive_text_and_defaults() {
    let fixture = Fixture::new();
    let path = fixture.0.join("archive.cfg");
    let defaults = (
        crate::ui::VideoSettings::default(),
        config::ClientPresentationSettings::default(),
        config::AudioSettings::default(),
    );
    config::save_video_settings(
        &path,
        defaults.0,
        &crate::keybinds::Bindings::default(),
        &defaults.1,
        &defaults.2,
    )
    .unwrap();
    let default_text = fs::read_to_string(&path).unwrap();
    assert_snapshot("default.cfg", &default_text);
    // Load every recognized name in sequence to exercise cross-cvar conversions.
    let mut input = default_text;
    for name in NAMES
        .lines()
        .filter(|name| crate::app::cvar_registry().find(name).is_some())
    {
        input.push_str(&format!("seta {name} \"2\"\n"));
    }
    fs::write(&path, input).unwrap();
    let video = config::load_video_settings(&path);
    let audio = config::load_audio_settings(&path);
    let client = config::load_client_presentation_settings(&path);
    config::save_video_settings(
        &path,
        video,
        &crate::keybinds::Bindings::default(),
        &client,
        &audio,
    )
    .unwrap();
    assert_snapshot("changed.cfg", &fs::read_to_string(&path).unwrap());
}

#[test]
fn current_config_ignores_obsolete_keys_and_keeps_hdr_independent_of_tonemap() {
    let fixture = Fixture::new();
    let path = fixture.0.join("input.cfg");
    fs::write(&path, "seta r_hdr 1\nseta r_bakedBrightness 1\nseta r_fogMode volumetric\nseta r_cloudWindSpeed 0\nseta r_oceanWeather \"0 0 0 0\"\n").unwrap();
    let actual = config::load_video_settings(&path);
    let mut expected = crate::ui::VideoSettings::default();
    expected.hdr = true;
    expected.ocean_settings.wind = expected.weather_wind;
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    let absent = fixture.0.join("absent.cfg");
    fs::write(fixture.0.join("jampconfig.cfg"), "seta r_gamma 2\n").unwrap();
    assert_eq!(
        format!("{:?}", config::load_video_settings(&absent)),
        format!("{:?}", crate::ui::VideoSettings::default())
    );
    assert_eq!(
        config::load_audio_settings(&absent),
        config::AudioSettings::default()
    );
    assert_eq!(
        config::load_client_presentation_settings(&absent),
        config::ClientPresentationSettings::default()
    );
}

#[test]
fn notification_cvar_addition_loads_and_archives_its_current_default() {
    let fixture = Fixture::new();
    let path = fixture.0.join("input.cfg");
    fs::write(&path, "seta con_notifylines 8\n").unwrap();
    let settings = config::load_client_presentation_settings(&path);
    assert_eq!(settings.console_notify_lines, 8);
    config::save_video_settings(
        &path,
        crate::ui::VideoSettings::default(),
        &crate::keybinds::Bindings::default(),
        &settings,
        &config::AudioSettings::default(),
    )
    .unwrap();
    assert!(fs::read_to_string(&path)
        .unwrap()
        .contains("seta con_notifylines \"8\""));
    assert_eq!(
        crate::app::cvar_registry()
            .find("con_notifylines")
            .unwrap()
            .entry
            .default,
        "3"
    );
    assert!(!crate::app::cvar_registry()
        .find("con_notifytime")
        .unwrap()
        .flags
        .contains(crate::console::cvars::CvarFlags::ARCHIVE));
}

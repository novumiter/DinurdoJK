//! Disk-backed, scene-specific *visual* placeholders for the egui frontend.
//! This cache never replaces the live WGPU scene or controls gameplay state.
use super::App;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Cached-scene startup fade (black -> intended brightness).
pub(super) const FRONTEND_STAGE_FADE_IN_SECS: f32 = 0.9;
/// Menu chrome fades in over the same clock, a little ahead of the scene.
pub(super) const FRONTEND_CHROME_FADE_IN_SECS: f32 = 0.38;
/// If no presentation ack ever arrives, start anyway rather than stay black.
const FRONTEND_STAGE_ACK_TIMEOUT: Duration = Duration::from_millis(2500);

pub(super) fn smoothstep01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The one clock behind the menu's startup darkness. It starts when the
/// renderer reports that a frame carrying the (dark) stage was presented, not
/// when the CPU built it, so decode/BSP/GPU stalls cannot consume the fade.
#[derive(Default)]
pub(super) struct FrontendStage {
    /// Bumped by every scene selection; doubles as the present token.
    epoch: u64,
    /// Snapshot JPEG is being decoded; the stage stays black until it lands.
    pub(super) cache_pending: bool,
    /// A scene has been selected, so there is something to fade in.
    scene_selected: bool,
    first_built: Option<Instant>,
    presented_at: Option<Instant>,
}

impl FrontendStage {
    pub(super) fn begin_scene(&mut self, cache_pending: bool) {
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.cache_pending = cache_pending;
        self.scene_selected = true;
        self.first_built = None;
        self.presented_at = None;
    }

    /// Token to attach to the next egui frame, once the stage is meaningful to
    /// present: a scene is selected and its snapshot decode (if any) resolved.
    pub(super) fn token_for_frame(&mut self, now: Instant) -> Option<u64> {
        if self.presented_at.is_some() {
            return None;
        }
        let first = *self.first_built.get_or_insert(now);
        let waiting = !self.scene_selected || self.cache_pending;
        if waiting && now.saturating_duration_since(first) < FRONTEND_STAGE_ACK_TIMEOUT {
            return None;
        }
        Some(self.epoch)
    }

    /// Returns true when this ack started the clock.
    pub(super) fn on_presented(&mut self, token: u64, now: Instant) -> bool {
        if token != self.epoch || self.presented_at.is_some() {
            return false;
        }
        self.presented_at = Some(now);
        true
    }

    pub(super) fn elapsed_secs(&self, now: Instant) -> Option<f32> {
        self.presented_at.map(|at| now.saturating_duration_since(at).as_secs_f32())
    }

    /// 0 until the first presented dark frame, then eased up to 1.
    pub(super) fn brightness(&self, now: Instant, fade_secs: f32) -> f32 {
        self.elapsed_secs(now).map_or(0.0, |secs| smoothstep01(secs / fade_secs))
    }

    /// Still animating, or waiting to start: callers must keep repainting.
    pub(super) fn animating(&self, now: Instant) -> bool {
        self.elapsed_secs(now).is_none_or(|secs| secs < FRONTEND_STAGE_FADE_IN_SECS)
    }
}

/// Stable per-scene key, deliberately independent of any hardcoded menu map.
/// The selected map and active mod namespace both participate in the key.
fn scene_hash(scene: &str, mod_name: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in scene.bytes().chain([0])
        .chain(mod_name.bytes()).chain([0])
        .chain(super::FRONTEND_FOREGROUND_MODEL.bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub(super) fn path(base: &Path, game: Option<&Path>, scene: &str) -> PathBuf {
    let mod_name = game.map_or("base", |path| path.to_str().unwrap_or("mod"));
    base.join("cache").join("frontend_scenes").join(format!(
        "v5_{:016x}.jpg", scene_hash(scene, mod_name),
    ))
}

impl App {
    pub(super) fn load_frontend_scene_cache(&mut self) {
        self.frontend_scene_texture = None;
        self.frontend_scene_cache_generation = self.frontend_scene_cache_generation.wrapping_add(1);
        let generation = self.frontend_scene_cache_generation;
        let Some(path) = self.frontend_scene_cache_path.clone() else { return; };
        self.frontend_stage.cache_pending = path.exists();
        let proxy = self.proxy.clone();
        // JPEG decoding and filesystem access must never hold up the first
        // clickable menu frame. Only texture registration happens on main.
        let _ = std::thread::Builder::new()
            .name("frontend-scene-cache".into())
            .spawn(move || {
                remove_legacy_snapshots(&path);
                let bytes = std::fs::read(&path).ok();
                let image = bytes.as_ref()
                    .and_then(|bytes| image::load_from_memory(bytes).ok())
                    .map(|image| {
                        let rgba = image.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
                    });
                if bytes.is_some() && image.is_none() {
                    // Malformed/truncated file: drop it so it is recaptured.
                    let _ = std::fs::remove_file(&path);
                }
                let _ = proxy.send_event(crate::runtime::UserEvent::FrontendSceneCacheReady {
                    path, generation, image,
                });
            });
    }

    pub(super) fn select_frontend_scene(&mut self, scene: &str) {
        let path = path(&self.base, self.game.as_deref(), scene);
        // Every entry to the menu scene restarts the one stage timeline.
        self.frontend_stage.begin_scene(false);
        if self.frontend_scene_cache_path.as_ref() != Some(&path) {
            self.frontend_scene_cache_path = Some(path);
            self.load_frontend_scene_cache();
        } else if self.frontend_scene_texture.is_none() && path.exists() {
            // The very first visit just populated this cache. Keep its cold
            // reveal undisturbed; load it when the menu is next requested.
            self.load_frontend_scene_cache();
        }
        // A texture that is already resident is not pending.
        self.frontend_stage.cache_pending &= self.frontend_scene_texture.is_none();
        // Prevent reusing the previous world's animation phase while the
        // replacement BSP is still loading in the background.
        self.frontend_cinematic = None;
    }

    /// Token for the egui frame being built (see `FrontendStage`).
    pub(super) fn frontend_stage_token(&mut self) -> Option<u64> {
        if !self.front_end {
            return None;
        }
        self.frontend_stage.token_for_frame(Instant::now())
    }

    pub(super) fn on_frontend_stage_presented(&mut self, token: u64) {
        let now = Instant::now();
        if self.frontend_stage.on_presented(token, now) {
            println!(
                "[MENU FADE] first dark frame presented: cached_texture={} cinematic_fade_started={}",
                self.frontend_scene_texture.is_some(),
                self.frontend_cinematic.is_some_and(|c| c.fade_started.is_some()),
            );
            self.egui_repaint_requested = true;
        }
    }
}

/// Best-effort removal of snapshots written by older cache formats.
/// v4 inherited gameplay FOV and can no longer be used for the fixed menu
/// projection; v3 and earlier could contain console/HUD pixels.
fn remove_legacy_snapshots(current: &Path) {
    let Some(dir) = current.parent() else { return; };
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name.starts_with("v4_") || name.starts_with("v3_")
            || name.starts_with("v2_") || name.starts_with("v1_"))
            && name.ends_with(".jpg")
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_clock_starts_only_on_matching_presentation() {
        let t0 = Instant::now();
        let mut stage = FrontendStage::default();
        assert_eq!(stage.token_for_frame(t0), None, "no scene selected yet");
        assert!(stage.token_for_frame(t0 + Duration::from_secs(3)).is_some(), "timeout fallback");
        stage.begin_scene(true);
        assert_eq!(stage.token_for_frame(t0), None, "waiting on snapshot decode");
        stage.cache_pending = false;
        let token = stage.token_for_frame(t0).expect("armed once snapshot resolved");
        assert_eq!(stage.brightness(t0, 0.9), 0.0);
        assert!(!stage.on_presented(token + 1, t0), "stale token ignored");
        assert!(stage.on_presented(token, t0));
        assert!(!stage.on_presented(token, t0), "clock never restarts");
        assert_eq!(stage.brightness(t0, 0.9), 0.0, "first presented frame is dark");
        let mid = stage.brightness(t0 + Duration::from_millis(450), 0.9);
        assert!((mid - 0.5).abs() < 1e-4);
        assert_eq!(stage.brightness(t0 + Duration::from_secs(2), 0.9), 1.0);
        assert_eq!(stage.token_for_frame(t0), None);
    }

    #[test]
    fn stage_ack_timeout_unblocks_pending_snapshot() {
        let t0 = Instant::now();
        let mut stage = FrontendStage::default();
        stage.begin_scene(true);
        assert_eq!(stage.token_for_frame(t0), None);
        assert!(stage.token_for_frame(t0 + Duration::from_secs(3)).is_some());
    }

    #[test]
    fn new_scene_restarts_the_clock() {
        let t0 = Instant::now();
        let mut stage = FrontendStage::default();
        stage.begin_scene(false);
        let first = stage.token_for_frame(t0).unwrap();
        stage.on_presented(first, t0);
        stage.begin_scene(false);
        assert_eq!(stage.brightness(t0 + Duration::from_secs(5), 0.9), 0.0);
        assert_ne!(stage.token_for_frame(t0), Some(first));
    }
    #[test]
    fn cache_keys_are_scene_and_mod_specific() {
        assert_eq!(scene_hash("mp/duel3", "base"), scene_hash("mp/duel3", "base"));
        assert_ne!(scene_hash("mp/duel3", "base"), scene_hash("mp/duel1", "base"));
        assert_ne!(scene_hash("mp/duel3", "base"), scene_hash("mp/duel3", "japro"));
    }
}

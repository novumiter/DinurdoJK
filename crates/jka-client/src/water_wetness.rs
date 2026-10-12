//! Snapshot-rate BSP immersion queries and per-player body-region drying.
//! No GPU queries, render passes, or dedicated workers. A player's water history
//! is independent of camera position/rain exposure and lives with the CGame session.
use std::{collections::HashMap, time::Instant};
use jka_movement::{CollisionWorld, TraceWorld};

const CONTENTS_WATER: i32 = 0x4; // OpenJK/TaystJK CONTENTS_WATER
pub(crate) const BAND_COUNT: usize = 8;
pub(crate) const BAND_STEP: f32 = 8.0;
pub(crate) const BAND_FIRST_Z: f32 = -21.0;
const DRY_SECONDS: f32 = 12.0; // Same as the rain model's drying time.

/// The render-thread payload. Heights are in JKA world coordinates (Z-up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WaterProfile {
    pub first_z: f32,
    pub bands: [f32; BAND_COUNT],
}

impl WaterProfile {
    pub fn wetness_at_z(&self, z: f32) -> f32 {
        let band = ((z - self.first_z) / BAND_STEP).clamp(0.0, (BAND_COUNT - 1) as f32);
        let lower = band as usize;
        let upper = (lower + 1).min(BAND_COUNT - 1);
        self.bands[lower] + (self.bands[upper] - self.bands[lower]) * band.fract()
    }
}

#[derive(Clone, Debug)]
struct WaterHistory {
    /// When each vertical body band was most recently observed submerged.
    last_submerged: [Option<Instant>; BAND_COUNT],
    first_z: f32,
    last_present: Instant,
}

/// A single CGame/game session's immersion history. `sample_snapshot` may be
/// called every main-thread tick but queries BSP only on a new snapshot.
#[derive(Default)]
pub(crate) struct WetnessTracker {
    pub(crate) snapshot_message: Option<i32>,
    pub(crate) session_source: String,
    pub(crate) map_name: String,
    players: HashMap<u16, WaterHistory>,
}

impl WetnessTracker {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn sample_snapshot(
        &mut self, world: &mut CollisionWorld, message_num: i32,
        players: &[(u16, [f32; 3])], now: Instant,
    ) {
        if self.snapshot_message == Some(message_num) { return; }
        self.snapshot_message = Some(message_num);
        for &(entity, origin) in players {
            if !origin.iter().all(|component| component.is_finite()) { continue; }
            let first_z = origin[2] + BAND_FIRST_Z;
            let state = self.players.entry(entity).or_insert(WaterHistory {
                last_submerged: [None; BAND_COUNT], first_z, last_present: now,
            });
            // A respawn/teleport at a different height must not smear a previous
            // waterline up the new body. Drying is stored in body-relative bands.
            state.first_z = first_z;
            state.last_present = now;
            for (i, last) in state.last_submerged.iter_mut().enumerate() {
                let sample_z = first_z + i as f32 * BAND_STEP;
                if world.point_contents([origin[0], origin[1], sample_z], i32::from(entity)) & CONTENTS_WATER != 0 {
                    *last = Some(now);
                }
            }
        }
        // A vanished entity must not leak an old wet body into a reused slot.
        self.players.retain(|_, state| now.saturating_duration_since(state.last_present).as_secs_f32() < DRY_SECONDS);
    }

    pub fn profiles(&self, now: Instant) -> HashMap<u16, WaterProfile> {
        let mut result = HashMap::with_capacity(self.players.len());
        for (&id, state) in &self.players {
            let bands = state.last_submerged.map(|last| {
                last.map_or(0.0, |then| (1.0 - now.saturating_duration_since(then).as_secs_f32() / DRY_SECONDS).clamp(0.0, 1.0))
            });
            if bands.iter().any(|&v| v > 0.001) {
                result.insert(id, WaterProfile { first_z: state.first_z, bands });
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn waterline_blends_between_body_regions() {
        let p = WaterProfile { first_z: 100.0, bands: [1., 1., 1., 0., 0., 0., 0., 0.] };
        assert_eq!(p.wetness_at_z(100.0), 1.0);
        assert_eq!(p.wetness_at_z(112.0), 1.0);
        assert_eq!(p.wetness_at_z(120.0), 0.5);
        assert_eq!(p.wetness_at_z(140.0), 0.0);
    }
    #[test]
    fn drying_is_independent_per_band() {
        let now = Instant::now();
        let mut tracker = WetnessTracker::default();
        tracker.players.insert(5, WaterHistory {
            last_submerged: [Some(now), None, Some(now - Duration::from_secs(6)), None, None, None, None, None],
            first_z: 0.0, last_present: now,
        });
        let p = tracker.profiles(now)[&5];
        assert_eq!(p.bands[0], 1.0);
        assert_eq!(p.bands[1], 0.0);
        assert!((p.bands[2] - 0.5).abs() < 0.01);
        assert!(tracker.profiles(now + Duration::from_secs(13)).is_empty());
    }
}

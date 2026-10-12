//! CGame-side, snapshot-rate water immersion checks. No renderer-thread collision queries.
use crate::app::{App, Instant, ET_PLAYER};
use std::collections::HashSet;

impl App {
    pub(super) fn tick_water_wetness(&mut self, now: Instant) {
        let Some(session) = self.game_session.as_ref() else {
            self.water_wetness.clear();
            return;
        };
        let Some(snapshot) = session.current_snapshot.as_ref() else { return; };
        let map = session.map_name.as_deref().unwrap_or("");
        if self.water_wetness.session_source != session.source
            || self.water_wetness.map_name != map
        {
            self.water_wetness.clear();
            self.water_wetness.session_source.clone_from(&session.source);
            self.water_wetness.map_name = map.to_owned();
        }
        if self.water_wetness.snapshot_message == Some(snapshot.message_num) { return; }
        let Some(world) = self.map_collision.as_mut() else { return; };

        // Remote players use the current interpolated CGame origin at snapshot
        // arrivals; the locally presented player isn't in presented_entities.
        let mut positions = Vec::with_capacity(session.presented_entities.len() + 1);
        let mut seen = HashSet::with_capacity(session.presented_entities.len() + 1);
        for e in &session.presented_entities {
            if e.entity_type == ET_PLAYER && seen.insert(e.number) {
                positions.push((e.number, e.origin));
            }
        }
        if let Some(e) = session.audio_followed_entity.as_ref() {
            if e.entity_type == ET_PLAYER && seen.insert(e.number) {
                positions.push((e.number, e.origin));
            }
        }
        if let (Some(ps), Some(client)) = (
            session.presented_player_state.as_ref(),
            session.client_game.presentation_client_num(),
        ) {
            if let Ok(entity) = u16::try_from(client) {
                if seen.insert(entity) {
                    let p = [ps.field_f32("origin[0]").unwrap_or(0.0),
                             ps.field_f32("origin[1]").unwrap_or(0.0),
                             ps.field_f32("origin[2]").unwrap_or(0.0)];
                    positions.push((entity, p));
                }
            }
        }
        self.water_wetness.sample_snapshot(world, snapshot.message_num, &positions, now);
    }
}

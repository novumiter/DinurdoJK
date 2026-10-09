//! Runtime Siege helpers. These keep the TaystJK/OpenJK wire semantics out of
//! generic menu/HUD code while sharing one path across live, local and demos.

use crate::app::session::types::GameSession;
use crate::cgame::siege::SiegeExtendedDataEntry;
use jka_assets::siege::{parse_siege_map, SiegeMapDefinition, SIEGE_TEAM1, SIEGE_TEAM2};

const GT_SIEGE: i32 = 7;
const PERS_TEAM: usize = 3;
const TEAM_SPECTATOR: i32 = 3;

impl GameSession {
    pub(in crate::app) fn active_siege_map(&self) -> Option<SiegeMapDefinition> {
        if self.client_game.gametype() != GT_SIEGE {
            return None;
        }
        let map_name = self
            .client_game
            .serverinfo_value(b"mapname")
            .or_else(|| self.map_name.clone())?;
        let base = self.siege_catalog.map(&map_name)?;
        let team1 = self.client_game.serverinfo_value(b"g_siegeTeam1");
        let team2 = self.client_game.serverinfo_value(b"g_siegeTeam2");
        parse_siege_map(
            &base.map_name,
            base.source_text.as_bytes(),
            team1.as_deref(),
            team2.as_deref(),
        )
        .ok()
        .flatten()
    }

    /// Team seen by cgame. In follow mode the snapshot/playerState belongs to
    /// the followed player, which is also what TaystJK uses for Siege HUD data.
    pub(in crate::app) fn siege_viewer_team(&self) -> i32 {
        self.presented_player_state
            .as_ref()
            .map(|ps| ps.persistant[PERS_TEAM])
            .or_else(|| self.current_snapshot.as_ref().map(|snap| snap.player_state.persistant[PERS_TEAM]))
            .unwrap_or(TEAM_SPECTATOR)
    }

    pub(in crate::app) fn siege_viewer_client(&self) -> Option<i32> {
        self.presented_player_state
            .as_ref()
            .and_then(|ps| ps.field_i32("clientNum"))
            .or_else(|| self.current_snapshot.as_ref().and_then(|snap| snap.player_state.field_i32("clientNum")))
    }

    pub(in crate::app) fn siege_map_team(&self, team: i32) -> Option<jka_assets::siege::SiegeMapTeam> {
        let map = self.active_siege_map()?;
        match team {
            SIEGE_TEAM1 => Some(map.team1),
            SIEGE_TEAM2 => Some(map.team2),
            _ => None,
        }
    }

    pub(in crate::app) fn apply_siege_extended_data(&mut self, entries: &[SiegeExtendedDataEntry]) {
        for entry in entries {
            let Ok(client) = usize::try_from(entry.client) else { continue; };
            let Some(slot) = self.siege_extended.get_mut(client) else { continue; };
            let weapon = self
                .presented_entities
                .iter()
                .find(|entity| usize::from(entity.number) == client)
                .and_then(|entity| entity.state.field_i32("weapon"))
                .unwrap_or(-1);
            slot.health = entry.health;
            slot.max_health = entry.max_health;
            slot.ammo = entry.ammo;
            // TaystJK records the current weapon only for usable ammo data.
            slot.weapon = if entry.ammo >= 0 { weapon } else { -1 };
            slot.last_updated = self.cgame_time;
        }
    }
}

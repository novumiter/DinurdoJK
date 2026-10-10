//! Shared Profile Studio helpers and protocol-26 Force editor.
use crate::app::egui_menu::{
    theme, ui_catalog, App, Instant, ProfileForceConfig,
    ProfileModelEntry, ProfileSaberEntry, FORCE_DARK_ORDER, FORCE_LIGHT_ORDER, FORCE_MASTERY_NAMES,
    FORCE_NAMES, FORCE_NEUTRAL_ORDER,
};

impl App {
    // Shared asset-label helpers and Force editing for the single Profile UI.

    pub(in crate::app::egui_menu) fn profile_readout(ui: &mut egui::Ui, label: &str, value: &str) {
        ui.horizontal(|ui| {
            ui.set_min_height(22.0);
            theme::label(ui, theme::plain(label, 11.5, theme::TEXT_FAINT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::label(ui, theme::plain(value, 11.5, theme::TEXT));
            });
        });
    }

    pub(in crate::app::egui_menu) fn profile_model_icon_texture(
        &mut self,
        entry: &ProfileModelEntry,
    ) -> Option<egui::TextureHandle> {
        let key = entry.value.to_ascii_lowercase();
        if let Some(texture) = self.profile_model_icon_textures.get(&key) {
            return Some(texture.clone());
        }
        if self.ui_catalog.icons_missing.contains(&key)
            || self.ui_catalog.icons_inflight.contains(&key)
        {
            return None;
        }
        self.ui_catalog.icons_inflight.insert(key.clone());
        self.ui_catalog.request(
            &self.base,
            self.game.as_deref(),
            ui_catalog::CatalogRequest::ProfileIcon {
                key,
                model: entry.model_name.clone(),
                skin: entry.skin_name.clone(),
            },
        );
        None
    }

    pub(in crate::app::egui_menu) fn profile_saber_label(entry: &ProfileSaberEntry) -> &str {
        // Stock saber files may use @MENUS_* localization tokens. Until the
        // string-package browser is wired into egui, the block identifier is a
        // cleaner fallback than exposing the raw token.
        if entry.display_name.is_empty() || entry.display_name.starts_with('@') {
            &entry.name
        } else {
            &entry.display_name
        }
    }

    pub(in crate::app::egui_menu) fn profile_saber_is_single(entry: &ProfileSaberEntry) -> bool {
        entry.saber_type.eq_ignore_ascii_case("SABER_SINGLE")
    }

    pub(in crate::app::egui_menu) fn profile_saber_is_staff(entry: &ProfileSaberEntry) -> bool {
        entry.saber_type.eq_ignore_ascii_case("SABER_STAFF")
    }

    pub(in crate::app::egui_menu) fn profile_server_force_limits(
        &self,
    ) -> (u8, u32, i32, bool, Option<u8>) {
        let info = if let Some(net) = self.net.as_ref() {
            crate::net::mod_support::server_info(&net.session().decoder().configstrings)
        } else if let Some(server) = self.local_server.as_ref() {
            server
                .configstrings()
                .get(&crate::cgame::CS_SERVERINFO)
                .map(Vec::as_slice)
                .unwrap_or_default()
        } else {
            &[]
        };
        let value = |key: &[u8], fallback: i32| {
            jka_protocol::commands::info_value(info, key)
                .map(jka_protocol::commands::atoi)
                .unwrap_or(fallback)
        };
        let max_rank = value(b"g_maxForceRank", 7).clamp(0, 7) as u8;
        let disabled = value(b"g_forcePowerDisable", 0) as u32;
        let gametype = value(b"g_gametype", 0);

        // TaystJK UI_HasSetSaberOnly, ported verbatim in behavior. Jedi Master
        // never grants the free saber; duel modes use g_duelWeaponDisable and
        // other modes use g_weaponDisable. WP_NONE=0, WP_SABER=3, WP_NUM=19.
        let free_saber = if gametype == 2 {
            false
        } else {
            let weapon_disable = if matches!(gametype, 3 | 4) {
                value(b"g_duelWeaponDisable", 0)
            } else {
                value(b"g_weaponDisable", 0)
            } as u32;
            (0..19).all(|weapon| {
                weapon == 0 || weapon == 3 || (weapon_disable & (1u32 << weapon)) != 0
            })
        };

        // UI_DrawForceSide: force-based teams pin Red to Dark and Blue to Light.
        let forced_side = if value(b"g_forceBasedTeams", 0) != 0 {
            self.game_session.as_ref().and_then(|session| {
                let client_num = session
                    .current_snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.player_state.field_i32("clientNum"))
                    .and_then(|client| usize::try_from(client).ok())?;
                let team = session
                    .client_game
                    .client_info(client_num, &session.siege_classes)?
                    .team;
                match team {
                    1 => Some(2), // TEAM_RED -> FORCE_DARKSIDE
                    2 => Some(1), // TEAM_BLUE -> FORCE_LIGHTSIDE
                    _ => None,
                }
            })
        } else {
            None
        };
        (max_rank, disabled, gametype, free_saber, forced_side)
    }

    pub(in crate::app::egui_menu) fn egui_profile_force(&mut self, ui: &mut egui::Ui) {
        let (max_rank, disabled, gametype, free_saber, forced_side) =
            self.profile_server_force_limits();
        let mut force = ProfileForceConfig::parse(&self.network.forcepowers);
        force.rank = max_rank;
        if let Some(side) = forced_side {
            force.side = side;
        }
        force.normalize(max_rank, disabled, gametype, free_saber);
        let before = force.serialize();

        theme::section(
            ui,
            "ALIGNMENT",
            "Light and Dark powers follow stock JKA restrictions",
        );
        ui.horizontal(|ui| {
            let light = theme::chip(ui, "LIGHT", force.side == 1);
            let dark = theme::chip(ui, "DARK", force.side == 2);
            if forced_side.is_none() {
                if light.clicked() {
                    force.side = 1;
                }
                if dark.clicked() {
                    force.side = 2;
                }
            }
        });
        if forced_side.is_some() {
            theme::label(
                ui,
                theme::plain(
                    "Side is fixed by g_forceBasedTeams for your current team.",
                    10.5,
                    theme::TEXT_FAINT,
                ),
            );
        }

        theme::section(
            ui,
            "SERVER FORCE BUDGET",
            "g_maxForceRank controls the available points",
        );
        force.normalize(max_rank, disabled, gametype, free_saber);
        let used = force.used(free_saber);
        let budget = force.budget();
        Self::profile_readout(
            ui,
            "MASTERY",
            &format!("{} ({max_rank})", FORCE_MASTERY_NAMES[max_rank as usize]),
        );
        Self::profile_readout(ui, "POINTS", &format!("{used} / {budget}"));

        theme::section(
            ui,
            "FORCE POWERS",
            "Neutral powers first, then the selected alignment",
        );
        let aligned = if force.side == 2 {
            &FORCE_DARK_ORDER[..]
        } else {
            &FORCE_LIGHT_ORDER[..]
        };
        let order = FORCE_NEUTRAL_ORDER
            .iter()
            .copied()
            .chain(aligned.iter().copied());
        let mut clicked_power = None;
        for power in order {
            let server_disabled = disabled & (1u32 << power) != 0 && !matches!(power, 1 | 15 | 16);
            let team_disabled = gametype < 6 && matches!(power, 11 | 12);
            let saber_dependency = matches!(power, 16 | 17) && force.powers[15] == 0;
            let locked = server_disabled || team_disabled || saber_dependency;
            ui.horizontal(|ui| {
                ui.set_width(ui.available_width());
                let color = if locked {
                    theme::TEXT_DISABLED
                } else {
                    theme::TEXT
                };
                theme::label(ui, theme::plain(FORCE_NAMES[power], 11.5, color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let min_level = if power == 1 || (free_saber && matches!(power, 15 | 16)) {
                        1
                    } else {
                        0
                    };
                    for level in (min_level..=3).rev() {
                        let response =
                            theme::chip(ui, &level.to_string(), force.powers[power] == level as u8);
                        if response.clicked() && !locked && force.powers[power] != level as u8 {
                            force.powers[power] = level as u8;
                            clicked_power = Some(power);
                        }
                    }
                });
            });
            ui.add_space(3.0);
        }

        if let Some(power) = clicked_power {
            // Delay the animation until allocation clicks settle. Repeated level
            // clicks for one power therefore restart a single preview, not a
            // full Ghoul2 animation setup on every UI event.
            self.profile_force_preview_pending = Some((power, Instant::now()));
        }

        force.normalize(max_rank, disabled, gametype, free_saber);
        let after = force.serialize();
        if after != before {
            if let Err(error) = self.profile_set_cvar("forcepowers", &after) {
                self.console_status = error;
            } else {
                self.profile_force_pending = true;
            }
        }
    }

}

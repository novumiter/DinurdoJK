//! Game.
use crate::app::egui_settings::{segmented_row, theme, App};

impl App {
    pub(in crate::app) fn egui_game_settings(&mut self, ui: &mut egui::Ui) {
        theme::page_title(
            ui,
            "GAME / VIEW",
            "Weapon and view-effect options. Camera controls live on the Camera tab.",
        );

        egui::ScrollArea::vertical()
            .id_salt("jka_game_view_settings")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                theme::section(ui, "WEAPONS", "Weapon selection helpers.");
                if let Some(value) = segmented_row(
                    ui,
                    "Auto switch",
                    "cg_autoSwitch. Switch to a weapon you pick up if it is better than the current one (never away from the saber), and to your best weapon when one runs dry. Safe skips rockets, thermal detonators and mines.",
                    theme::Reset::None,
                    self.audio.game.auto_switch,
                    &[(0, "Never"), (1, "Safe weapons"), (2, "Any weapon")],
                ) {
                    let _ = self.set_console_cvar("cg_autoSwitch", &value.to_string());
                }

                theme::section(
                    ui,
                    "PLAYER ANIMATIONS",
                    "Local rendering preferences; server gameplay and target selection are unchanged.",
                );
                if let Some(mode) = segmented_row(
                    ui,
                    "Automatic head tracking",
                    "cg_headLook. Off: no automatic look-target head turns. On: unchanged TaystJK/OpenJK behavior. Smoothed: gradually rotate toward new targets, without changing their selection or gameplay aim.",
                    theme::Reset::None,
                    self.head_look,
                    &[(0, "Off"), (1, "On"), (2, "Smoothed")],
                ) {
                    let _ = self.set_console_cvar("cg_headLook", &mode.to_string());
                }

                theme::section(
                    ui,
                    "SPECTATING & DEMOS",
                    "Presentation options shared by every server/mod and demo playback.",
                );
                theme::row(
                    ui,
                    "Player outlines",
                    "cg_specHud. Outline other living players through walls while spectating or watching demos. Team players are red/blue; neutral/FFA players are yellow.",
                    theme::Reset::None,
                    |ui| {
                        if let Some(on) = theme::switch(ui, self.japro_cg.spec_hud) {
                            let _ = self.set_console_cvar("cg_specHud", if on { "1" } else { "0" });
                        }
                    },
                );

                theme::section(ui, "VIEW EFFECTS", "Camera effects driven by the game.");
                theme::row(
                    ui,
                    "Score numbers",
                    "cg_scorePlums. Floating score numbers where you score.",
                    theme::Reset::None,
                    |ui| {
                        if let Some(value) = theme::switch(ui, self.audio.game.score_plums) {
                            let _ = self.set_console_cvar("cg_scorePlums", if value { "1" } else { "0" });
                        }
                    },
                );
                if let Some(value) = segmented_row(
                    ui,
                    "Gibs",
                    "cg_blood. Gibbed players: off (a death voice plays instead), skull or brain only, or full gibs. Needs the jaPRO models/gibs assets; jaPRO itself defaults to off.",
                    theme::Reset::None,
                    self.audio.game.blood,
                    &[(0, "Off"), (1, "Skull / brain"), (2, "Full")],
                ) {
                    let _ = self.set_console_cvar("cg_blood", &value.to_string());
                }
                if let Some(value) = segmented_row(
                    ui,
                    "Screen shake",
                    "cg_screenShake. Effects: shake from explosions and creature stomps. Effects + weapons: also the kick of firing rockets, alt repeater, flechette and charged bryar/demp2/bowcaster shots. Server-triggered shake events (rancors, scripted quakes) are unaffected, as in OpenJK.",
                    theme::Reset::None,
                    self.screen_shake,
                    &[(0, "Off"), (1, "Effects"), (2, "Effects + weapons")],
                ) {
                    let _ = self.set_console_cvar("cg_screenShake", &value.to_string());
                }
            });
    }
}

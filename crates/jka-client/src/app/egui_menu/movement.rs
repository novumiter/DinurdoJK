//! Movement settings.
use crate::app::egui_menu::{theme, App};

impl App {
    pub(in crate::app) fn egui_movement_page(&mut self, ui: &mut egui::Ui) {
        theme::page_title(
            ui,
            "MOVEMENT",
            "Client movement/input cadence and movement-specific preferences for the active mod.",
        );

        egui::ScrollArea::vertical()
            .id_salt("jka_movement_settings")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                theme::section(
                    ui,
                    "CLIENT MOVEMENT",
                    "Authoritative server movement stays unchanged; this controls how often the client authors user commands.",
                );
                theme::row(
                    ui,
                    "Player movement rate",
                    "cl_commandRate. How often this client authors movement/input commands. With normal pmove_fixed 0 play, those command timestamps drive Pmove. If a remote server enables pmove_fixed, prediction automatically obeys that server's pmove_msec instead. This never caps render FPS or the event-rate subframe/late-latched mouse-look path.",
                    theme::Reset::MovementCommandRate,
                    |ui| {
                        let mut rate = self.network.command_rate;
                        if ui
                            .add(
                                egui::DragValue::new(&mut rate)
                                    .range(15..=1000)
                                    .speed(1.0)
                                    .suffix(" Hz")
                                    .update_while_editing(false),
                            )
                            .changed()
                        {
                            self.set_player_movement_rate(rate);
                        }
                    },
                );

                self.egui_mod_movement_settings(ui);
            });
    }
}

//! Account.
use crate::app::egui_settings::{theme, App};

impl App {
    /// jaPRO account login plus secure, exact-endpoint saved credentials.
    ///
    /// Saved credentials are keyed by the resolved UDP `IP:port`, never by a
    /// hostname or friendly label. Nothing is sent automatically on connect;
    /// the user explicitly presses LOG IN for the current server.
    pub(in crate::app::egui_settings) fn egui_japro_account(&mut self, ui: &mut egui::Ui) {
        theme::section(
            ui,
            "ACCOUNT",
            "Log in to your jaPRO account on the connected server. Register creates one if the server allows it.",
        );
        // jaPRO keeps both at 15 characters and takes them as single arguments.
        const LIMIT: usize = 15;
        let clean = |text: &mut String| {
            text.retain(|c| !c.is_whitespace() && !matches!(c, ';' | '"' | '\\'));
        };

        let mut username_changed = false;
        theme::row(
            ui,
            "Username",
            "ui_username. Up to 15 characters, no spaces. Saved to the config.",
            theme::Reset::None,
            |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.network.ui_username)
                        .char_limit(LIMIT)
                        .desired_width(220.0),
                );
                username_changed = response.changed();
            },
        );
        let mut submit = false;
        theme::row(
            ui,
            "Password",
            "ui_password. Up to 15 characters, no spaces. Kept for this session only and never written to the config.",
            theme::Reset::None,
            |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.network.ui_password)
                        .password(true)
                        .char_limit(LIMIT)
                        .desired_width(220.0),
                );
                submit = response.lost_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
            },
        );
        clean(&mut self.network.ui_username);
        clean(&mut self.network.ui_password);
        if username_changed {
            self.mark_config_dirty();
        }

        let connected = self.live_connected();
        let ready = !self.network.ui_username.is_empty() && !self.network.ui_password.is_empty();
        let mut command = None;
        theme::row(ui, "Account", "", theme::Reset::None, |ui| {
            ui.add_enabled_ui(connected, |ui| {
                ui.add_enabled_ui(ready, |ui| {
                    if theme::primary_button(ui, "LOG IN").clicked()
                        || (submit && ready && connected)
                    {
                        command = Some(format!(
                            "login {} {}",
                            self.network.ui_username, self.network.ui_password
                        ));
                    }
                    ui.add_space(8.0);
                    if theme::ghost_button(ui, "REGISTER").clicked() {
                        command = Some(format!(
                            "register {} {}",
                            self.network.ui_username, self.network.ui_password
                        ));
                    }
                });
                ui.add_space(8.0);
                if theme::ghost_button(ui, "LOG OUT").clicked() {
                    command = Some("logout".to_owned());
                }
            });
        });
        if !connected {
            theme::label(
                ui,
                theme::plain("Join a jaPRO server to log in.", 11.5, theme::TEXT_FAINT),
            );
        }
        if let Some(command) = command {
            self.forward_command_to_server(&command);
        }

        ui.add_space(16.0);
        theme::section(
            ui,
            "SAVED LOGINS",
            "Passwords are stored by the operating system, not in DinurdoJK.cfg. Saved credentials never log in automatically; LOG IN is always an explicit action for the exact current IP:port.",
        );

        if !crate::credential_store::supported() {
            theme::label(
                ui,
                theme::plain(
                    "Secure saved logins are unavailable on this platform.",
                    11.5,
                    theme::TEXT_FAINT,
                ),
            );
            return;
        }

        let current = self.current_japro_server_endpoint();
        let current_saved = current.and_then(|server| {
            self.japro_saved_logins
                .iter()
                .find(|entry| entry.server == server)
                .cloned()
        });

        let mut login = None;
        let mut edit = None;
        let mut forget = None;
        let mut new_login = None;

        if let Some(server) = current {
            theme::row(
                ui,
                "Current server",
                "The exact resolved UDP endpoint is the credential identity. A friendly server name is display-only and can never redirect a login.",
                theme::Reset::None,
                |ui| {
                    theme::glow_label(ui, &server.to_string(), 12.5, theme::TEXT);
                },
            );

            theme::row(
                ui,
                "Saved login",
                "A saved credential is only sent when you press LOG IN while connected to this exact endpoint.",
                theme::Reset::None,
                |ui| {
                    if let Some(saved) = &current_saved {
                        let display = saved
                            .server_name
                            .as_deref()
                            .filter(|name| !name.is_empty())
                            .map(|name| format!("{name}  ·  {}", saved.username))
                            .unwrap_or_else(|| saved.username.clone());
                        theme::glow_label(ui, &display, 12.5, theme::TEXT);
                        ui.add_space(10.0);
                        if theme::primary_button(ui, "LOG IN").clicked() {
                            login = Some(server);
                        }
                        ui.add_space(6.0);
                        if theme::ghost_button(ui, "EDIT").clicked() {
                            edit = Some(server);
                        }
                        ui.add_space(6.0);
                        if theme::ghost_button(ui, "FORGET").clicked() {
                            forget = Some(server);
                        }
                    } else if theme::primary_button(ui, "SAVE LOGIN").clicked() {
                        new_login = Some(server);
                    }
                },
            );
        } else {
            theme::label(
                ui,
                theme::plain(
                    "Connect to a jaPRO server to save or use a login for its exact IP:port.",
                    11.5,
                    theme::TEXT_FAINT,
                ),
            );
        }

        let other_logins: Vec<_> = self
            .japro_saved_logins
            .iter()
            .filter(|entry| Some(entry.server) != current)
            .cloned()
            .collect();
        if !other_logins.is_empty() {
            ui.add_space(10.0);
            theme::section(
                ui,
                "OTHER SAVED LOGINS",
                "These credentials are stored locally. Connect to the matching endpoint before LOG IN becomes available.",
            );
            for entry in other_logins {
                let label = entry
                    .server_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| entry.server.to_string());
                theme::row(
                    ui,
                    &label,
                    "Saved in Windows Credential Manager. The password is never displayed here.",
                    theme::Reset::None,
                    |ui| {
                        theme::glow_label(
                            ui,
                            &format!("{}  ·  {}", entry.username, entry.server),
                            12.5,
                            theme::TEXT,
                        );
                        ui.add_space(10.0);
                        if theme::ghost_button(ui, "EDIT").clicked() {
                            edit = Some(entry.server);
                        }
                        ui.add_space(6.0);
                        if theme::ghost_button(ui, "FORGET").clicked() {
                            forget = Some(entry.server);
                        }
                    },
                );
            }
        }

        if let Some(server) = login {
            if let Err(error) = self.login_with_saved_japro_credential(server) {
                self.japro_credential_error = Some(error);
            } else {
                self.japro_credential_error = None;
            }
        }
        if let Some(server) = new_login {
            self.begin_new_japro_login(server);
        } else if let Some(server) = edit {
            self.begin_edit_japro_login(server);
        }
        if let Some(server) = forget {
            match self.forget_japro_login(server) {
                Ok(()) => {
                    self.japro_credential_error = None;
                    self.console_status = format!("JAPRO SAVED LOGIN FORGOTTEN FOR {server}");
                }
                Err(error) => self.japro_credential_error = Some(error),
            }
        }

        // Work on a clone so rendering the editor never holds a mutable borrow
        // of an App field while a save/cancel action needs &mut self.
        let mut editor = self.japro_login_editor.clone();
        let mut save_editor = None;
        let mut cancel_editor = false;
        if let Some(state) = editor.as_mut() {
            ui.add_space(14.0);
            theme::section(
                ui,
                if state.new_entry {
                    "SAVE LOGIN"
                } else {
                    "EDIT SAVED LOGIN"
                },
                if state.new_entry {
                    "Give this exact endpoint an optional friendly name and save its jaPRO credentials securely."
                } else {
                    "Change the friendly name or username. Leave Password blank to keep the existing saved password."
                },
            );
            theme::row(
                ui,
                "Endpoint",
                "Security identity. This cannot be edited; save a separate login while connected to another endpoint.",
                theme::Reset::None,
                |ui| {
                    theme::glow_label(ui, &state.server.to_string(), 12.5, theme::TEXT);
                },
            );
            theme::row(
                ui,
                "Server name",
                "Optional local display name. It is never used to choose where credentials are sent.",
                theme::Reset::None,
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.server_name)
                            .char_limit(96)
                            .desired_width(260.0)
                            .hint_text("e.g. My jaPRO server"),
                    );
                },
            );
            theme::row(
                ui,
                "Username",
                "jaPRO username stored with this endpoint.",
                theme::Reset::None,
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.username)
                            .char_limit(LIMIT)
                            .desired_width(220.0),
                    );
                },
            );
            theme::row(
                ui,
                "Password",
                if state.new_entry {
                    "jaPRO password. Stored in Windows Credential Manager, never DinurdoJK.cfg."
                } else {
                    "Enter a new password, or leave blank to keep the existing saved password."
                },
                theme::Reset::None,
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.password)
                            .password(true)
                            .char_limit(LIMIT)
                            .desired_width(220.0)
                            .hint_text(if state.new_entry {
                                "password"
                            } else {
                                "keep existing"
                            }),
                    );
                },
            );
            clean(&mut state.username);
            clean(&mut state.password);

            theme::row(ui, "", "", theme::Reset::None, |ui| {
                let ready = !state.username.is_empty()
                    && (!state.new_entry || !state.password.is_empty());
                ui.add_enabled_ui(ready, |ui| {
                    if theme::primary_button(ui, "SAVE").clicked() {
                        save_editor = Some(state.clone());
                    }
                });
                ui.add_space(8.0);
                if theme::ghost_button(ui, "CANCEL").clicked() {
                    cancel_editor = true;
                }
            });
        }
        self.japro_login_editor = editor;

        if let Some(state) = save_editor {
            match self.save_japro_login_edit(
                state.server,
                &state.server_name,
                &state.username,
                &state.password,
                state.new_entry,
            ) {
                Ok(()) => {
                    self.japro_login_editor = None;
                    self.japro_credential_error = None;
                    self.console_status = if state.new_entry {
                        format!("JAPRO SAVED LOGIN SAVED FOR {}", state.server)
                    } else {
                        format!("JAPRO SAVED LOGIN UPDATED FOR {}", state.server)
                    };
                }
                Err(error) => self.japro_credential_error = Some(error),
            }
        } else if cancel_editor {
            self.japro_login_editor = None;
            self.japro_credential_error = None;
        }

        if let Some(error) = &self.japro_credential_error {
            theme::label(
                ui,
                theme::plain(
                    &format!("Credential store: {error}"),
                    11.5,
                    theme::TEXT_FAINT,
                ),
            );
        }
    }
}

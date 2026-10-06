use super::{CommandDef, CommandResult, CommandScope};
use crate::app::{keybinds, App};
commands! {
    cmd_bind ["bind", "bindspec"] ("bind <key> [command]", "Bind a key to a command; bind <key> queries the current binding.", CommandScope::Always) |self, command| {
        let words = keybinds::split_command_words(command.trim_start_matches('/'));
        let Some(verb) = words.first() else {
            return CommandResult::Unhandled;
        };

        let spectator = verb.eq_ignore_ascii_case("bindspec");
        let label = if spectator { "bindspec" } else { "bind" };
        match words.as_slice() {
            [_] => self.push_console_line(format!(
                "^3{label} <key> [command]^7 - attach a {}command to a key",
                if spectator { "spectator-only " } else { "" }
            )),
            [_, key_name] => match keybinds::parse_key(key_name) {
                Some(key) => {
                    let binding = if spectator {
                        self.bindings.get_spectator(key)
                    } else {
                        self.bindings.get(key)
                    }
                    .map(str::to_owned);
                    match binding {
                        Some(binding) => self.push_console_line(format!(
                            "^7\"{}\" = \"{}\"{}",
                            keybinds::key_name(key),
                            binding,
                            if spectator { " ^5[spectator]" } else { "" }
                        )),
                        None if spectator => self.push_console_line(format!(
                            "^7\"{}\" has no spectator override (inherits normal bind)",
                            keybinds::key_name(key)
                        )),
                        None => self.push_console_line(format!(
                            "^7\"{}\" is not bound",
                            keybinds::key_name(key)
                        )),
                    }
                }
                None => self.push_console_line(format!("^1\"{key_name}\" isn't a valid key")),
            },
            [_, key_name, rest @ ..] => match keybinds::parse_key(key_name) {
                Some(key) => {
                    if spectator {
                        self.bindings.set_spectator(key, rest.join(" "));
                    } else {
                        self.bindings.set(key, rest.join(" "));
                    }
                    self.refresh_bound_state();
                    self.mark_config_dirty();
                }
                None => self.push_console_line(format!("^1\"{key_name}\" isn't a valid key")),
            },
            [] => unreachable!(),
        }
        self.publish_ui();
        return CommandResult::Handled;
    }

    cmd_unbind ["unbind", "unbindspec"] ("unbind <key>", "Remove a key binding.", CommandScope::Always) |self, command| {
        let words = keybinds::split_command_words(command.trim_start_matches('/'));
        let Some(verb) = words.first() else {
            return CommandResult::Unhandled;
        };

        let spectator = verb.eq_ignore_ascii_case("unbindspec");
        let label = if spectator { "unbindspec" } else { "unbind" };
        if words.len() != 2 {
            self.push_console_line(format!(
                "^3{label} <key>^7 - remove a {}key binding",
                if spectator { "spectator-only " } else { "" }
            ));
        } else if let Some(key) = keybinds::parse_key(&words[1]) {
            if spectator {
                self.bindings.unbind_spectator(key);
            } else {
                self.bindings.unbind(key);
            }
            self.refresh_bound_state();
            self.mark_config_dirty();
        } else {
            self.push_console_line(format!("^1\"{}\" isn't a valid key", words[1]));
        }
        self.publish_ui();
        return CommandResult::Handled;
    }

    cmd_unbindall ["unbindall", "unbindspecall"] ("unbindall", "Remove all key bindings.", CommandScope::Always) |self, command| {
        let words = keybinds::split_command_words(command.trim_start_matches('/'));
        let Some(verb) = words.first() else {
            return CommandResult::Unhandled;
        };

        if verb.eq_ignore_ascii_case("unbindspecall") {
            self.bindings.clear_spectator();
        } else {
            self.bindings.clear();
        }
        self.refresh_bound_state();
        self.mark_config_dirty();
        self.publish_ui();
        return CommandResult::Handled;
    }

    cmd_bindlist ["bindlist", "bindspeclist"] ("bindlist", "List all current key bindings.", CommandScope::Always) |self, command| {
        let words = keybinds::split_command_words(command.trim_start_matches('/'));
        let Some(verb) = words.first() else {
            return CommandResult::Unhandled;
        };

        let spectator = verb.eq_ignore_ascii_case("bindspeclist");
        let entries = if spectator {
            self.bindings.spectator_sorted()
        } else {
            self.bindings.sorted()
        };
        for (key, binding) in entries {
            self.push_console_line(format!(
                "^7{} \"{}\"{}",
                keybinds::key_name(key),
                binding,
                if spectator { " ^5[spectator]" } else { "" }
            ));
        }
        return CommandResult::Handled;
    }

}

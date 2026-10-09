//! Jedi Academy renderer world-effect compatibility commands.
use crate::app::App;

use super::{CommandDef, CommandResult, CommandScope};

commands! {
    cmd_r_we ["r_we", ] (
        "r_we <die|clear|freeze|zone|wind|constantwind|gustingwind|lightrain|rain|acidrain|heavyrain|snow|spacedust|sand|fog|heavyrainfog|light_fog|outsideshake|outsidepain> [args]",
        "Send a Jedi Academy renderer world-effect command. Rain variants and snow use DinurdoJK's GPU weather renderer; other retail effects are compatibility placeholders until their renderer backends are implemented.",
        CommandScope::Always
    ) |self, command| {
        let Some((_, args)) = command.split_once(char::is_whitespace) else {
            self.push_console_line(format!("^3r_we: {}", crate::world_effects::HELP));
            return CommandResult::Handled;
        };
        let args = args.trim();
        if args.is_empty() {
            self.push_console_line(format!("^3r_we: {}", crate::world_effects::HELP));
            return CommandResult::Handled;
        }
        self.apply_world_effect_command(args, true);
        CommandResult::Status
    }
}

//! TaystJK `cosmetics` command. jaPRO retains `cp_cosmetics`; on other
//! servers the selection is carried by the color1 / color2 userinfo suffixes.
use crate::{app::App, global_cosmetics::{self, Slot}};

impl App {
    pub(in crate::app) fn command_cosmetics(&mut self, words: &[&str]) {
        if self.active_server_mod() == crate::net::mod_support::ServerMod::Japro {
            if words.len() == 1 {
                for (index, item) in crate::japro_cg::COSMETICS.iter().enumerate() {
                    self.push_console_line(format!("{index:2} [{}] {}",
                        if self.network.cosmetics & item.bit != 0 { 'X' } else { ' ' }, item.label));
                }
                return;
            }
            let Some(item) = words.get(1).and_then(|id| id.parse::<usize>().ok())
                .and_then(|index| crate::japro_cg::COSMETICS.get(index)) else {
                self.push_console_line("Usage: cosmetics [0..31] (jaPRO)".to_owned());
                return;
            };
            let mask = crate::japro_cg::toggle_cosmetic(self.network.cosmetics, item.menu_group(), Some(item.bit));
            if let Err(error) = self.set_console_cvar("cp_cosmetics", &(mask as i32).to_string()) {
                self.push_console_error(error);
            }
            return;
        }
        let slot = match words.get(1).copied() {
            Some(word) if word.eq_ignore_ascii_case("hats") => Slot::Hat,
            Some(word) if word.eq_ignore_ascii_case("capes") => Slot::Cape,
            Some(word) if word.eq_ignore_ascii_case("clear") => {
                for (cvar, color) in [("color1", self.network.color1), ("color2", self.network.color2)] {
                    if let Err(error) = self.set_console_cvar(cvar, &color.to_string()) {
                        self.push_console_error(error);
                    }
                }
                return;
            }
            _ => {
                self.push_console_line("Usage: cosmetics <hats/capes/clear> [id]".to_owned());
                return;
            }
        };
        let cvar = match slot { Slot::Hat => "color1", Slot::Cape => "color2" };
        let (color, current) = match slot {
            Slot::Hat => (self.network.color1, self.network.global_hat.clone()),
            Slot::Cape => (self.network.color2, self.network.global_cape.clone()),
        };
        let mut names = Vec::new();
        if let Ok(assets) = jka_assets::pk3::AssetSearchPath::open_game(&self.base, self.game.as_deref()) {
            let dir = format!("{}/", slot.model_dir());
            for path in assets.names() {
                if let Some(basename) = path.strip_prefix(&dir).and_then(|value| value.strip_suffix(".md3")) {
                    if global_cosmetics::valid_name(basename) { names.push(basename.to_owned()); }
                }
            }
        }
        names.sort_unstable();
        names.dedup();
        if words.len() == 2 {
            if names.is_empty() { self.push_console_line("No installed cosmetics in this category.".to_owned()); }
            for (index, name) in names.iter().enumerate() {
                self.push_console_line(format!("{index:2} [{}] {name}",
                    if name.eq_ignore_ascii_case(&current) { 'X' } else { ' ' }));
            }
            return;
        }
        let Some(selected) = words.get(2).and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| names.get(index)) else {
            self.push_console_line(format!("Invalid cosmetic ID. Use cosmetics {} to list IDs.", words[1]));
            return;
        };
        let next = if selected.eq_ignore_ascii_case(&current) { "" } else { selected.as_str() };
        if let Err(error) = self.set_console_cvar(cvar, &global_cosmetics::encode(color, next)) {
            self.push_console_error(error);
        } else {
            self.push_console_line(format!("{cvar} cosmetic: {}", if next.is_empty() { "none" } else { next }));
        }
    }
}

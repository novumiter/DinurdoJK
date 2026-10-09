//! Jedi Academy Siege data ported from OpenJK/TaystJK `bg_saga.c` + `cg_saga.c`.
//!
//! Siege is deliberately data-driven. Stock and custom servers can replace class/team themes and maps
//! without changing the client, so this module keeps the original brace/pair grammar and loads the same
//! `.scl`, `.team`, and `.siege` assets instead of inventing a second format.

use crate::pk3::AssetSearchPath;

const CLASS_DIRECTORY: &str = "ext_data/siege/classes/";
const CLASS_EXTENSION: &str = ".scl";
const TEAM_DIRECTORY: &str = "ext_data/siege/teams/";
const TEAM_EXTENSION: &str = ".team";
const MAP_DIRECTORY: &str = "maps/";
const MAP_EXTENSION: &str = ".siege";
const MAX_CLASS_FILE: usize = 4095; // BG_SiegeParseClassFile rejects len >= 4096.
const MAX_SIEGE_INFO_SIZE: usize = 16383; // MAX_SIEGE_INFO_SIZE is 16384 incl. NUL.

pub const SIEGE_TEAM1: i32 = 1;
pub const SIEGE_TEAM2: i32 = 2;
pub const SIEGE_ROUND_BEGIN_TIME: i32 = 5000;
pub const CFL_MORESABERDMG: u32 = 1 << 0;
pub const CFL_STRONGAGAINSTPHYSICAL: u32 = 1 << 1;
pub const CFL_FASTFORCEREGEN: u32 = 1 << 2;
pub const CFL_STATVIEWER: u32 = 1 << 3;
pub const CFL_HEAVYMELEE: u32 = 1 << 4;
pub const CFL_SINGLE_ROCKET: u32 = 1 << 5;
pub const CFL_CUSTOMSKEL: u32 = 1 << 6;
pub const CFL_EXTRA_AMMO: u32 = 1 << 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiegeClassVisual {
    pub name: String,
    pub forced_model: String,
    pub forced_skin: String,
    pub forced_saber_color: Option<i32>,
    pub forced_saber2_color: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SiegeClass {
    pub name: String,
    pub description: String,
    pub forced_model: String,
    pub forced_skin: String,
    pub saber1: String,
    pub saber2: String,
    pub saber_style: String,
    pub forced_saber_color: Option<i32>,
    pub forced_saber2_color: Option<i32>,
    pub weapons: String,
    pub force_powers: String,
    pub class_flags: u32,
    pub max_health: i32,
    pub start_health: i32,
    pub max_armor: i32,
    pub start_armor: i32,
    pub speed: f32,
    pub ui_shader: String,
    pub class_shader: String,
    pub holdables: String,
    pub powerups: String,
}

impl SiegeClass {
    pub fn visual(&self) -> SiegeClassVisual {
        SiegeClassVisual {
            name: self.name.clone(),
            forced_model: self.forced_model.clone(),
            forced_skin: self.forced_skin.clone(),
            forced_saber_color: self.forced_saber_color,
            forced_saber2_color: self.forced_saber2_color,
        }
    }

    pub fn has_flag(&self, flag: u32) -> bool { self.class_flags & flag != 0 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiegeTeamTheme {
    /// `name` in the `.team` file; this is what a map's `UseTeam` references.
    pub name: String,
    pub class_names: Vec<String>,
    pub friendly_shader: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SiegeObjective {
    pub number: usize,
    pub description: String,
    pub graphic: String,
    pub map_icon: String,
    pub lit_map_icon: String,
    pub done_map_icon: String,
    pub map_pos: String,
    pub goal_name: String,
    pub final_objective: bool,
    pub message_team1: String,
    pub message_team2: String,
    pub sound_team1: String,
    pub sound_team2: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SiegeMapTeam {
    /// Name of the map-local group selected by `Teams { teamN ... }`.
    pub group_name: String,
    pub team_icon: String,
    pub timed_ms: i32,
    pub use_team: String,
    pub friendly_shader: String,
    pub briefing: String,
    pub won_round: String,
    pub lost_round: String,
    pub roundover_sound_wewon: String,
    pub roundover_sound_welost: String,
    pub objectives: Vec<SiegeObjective>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SiegeMapDefinition {
    /// Lower-case qpath without `maps/` or `.siege`, matching server `mapname`.
    pub map_name: String,
    /// Original `.siege` text retained so `g_siegeTeam1`/`g_siegeTeam2` can select
    /// alternate map-local team groups exactly like TaystJK's `cg_siegeTeam*` cvars.
    pub source_text: String,
    pub map_graphic: String,
    pub mission_name: String,
    pub team1: SiegeMapTeam,
    pub team2: SiegeMapTeam,
}

#[derive(Debug, Clone, Default)]
pub struct SiegeCatalog {
    pub classes: Vec<SiegeClass>,
    pub teams: Vec<SiegeTeamTheme>,
    pub maps: Vec<SiegeMapDefinition>,
}

impl SiegeCatalog {
    pub fn class(&self, name: &str) -> Option<&SiegeClass> {
        self.classes.iter().find(|class| class.name.eq_ignore_ascii_case(name))
    }

    pub fn team_theme(&self, name: &str) -> Option<&SiegeTeamTheme> {
        self.teams.iter().find(|team| team.name.eq_ignore_ascii_case(name))
    }

    pub fn map(&self, name: &str) -> Option<&SiegeMapDefinition> {
        let normalized = normalize_map_name(name);
        self.maps.iter().find(|map| map.map_name.eq_ignore_ascii_case(&normalized))
    }

    pub fn visuals(&self) -> Vec<SiegeClassVisual> {
        self.classes.iter().map(SiegeClass::visual).collect()
    }
}

pub fn load_siege_catalog(assets: &mut AssetSearchPath) -> Result<SiegeCatalog, String> {
    let class_names = direct_asset_names(assets, CLASS_DIRECTORY, CLASS_EXTENSION);
    let team_names = direct_asset_names(assets, TEAM_DIRECTORY, TEAM_EXTENSION);
    let map_names = assets
        .names()
        .filter(|name| name.starts_with(MAP_DIRECTORY) && name.ends_with(MAP_EXTENSION))
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let mut classes = Vec::new();
    for name in class_names {
        let Some(asset) = assets.read(&name, MAX_CLASS_FILE).map_err(|e| format!("{name}: {e}"))? else { continue; };
        if let Some(class) = parse_siege_class(&asset.bytes)? { classes.push(class); }
    }

    let mut teams = Vec::new();
    for name in team_names {
        let Some(asset) = assets.read(&name, MAX_SIEGE_INFO_SIZE).map_err(|e| format!("{name}: {e}"))? else { continue; };
        if let Some(team) = parse_siege_team(&asset.bytes)? { teams.push(team); }
    }

    let mut maps = Vec::new();
    for name in map_names {
        let Some(asset) = assets.read(&name, MAX_SIEGE_INFO_SIZE).map_err(|e| format!("{name}: {e}"))? else { continue; };
        let fallback = name
            .strip_prefix(MAP_DIRECTORY)
            .and_then(|n| n.strip_suffix(MAP_EXTENSION))
            .unwrap_or(&name);
        if let Some(map) = parse_siege_map(fallback, &asset.bytes, None, None)? { maps.push(map); }
    }
    Ok(SiegeCatalog { classes, teams, maps })
}

fn direct_asset_names(assets: &AssetSearchPath, directory: &str, extension: &str) -> Vec<String> {
    assets.names()
        .filter(|name| name.starts_with(directory) && name.ends_with(extension) && !name[directory.len()..].contains('/'))
        .map(str::to_owned)
        .collect()
}

pub fn load_siege_class_visuals(
    assets: &mut AssetSearchPath,
) -> Result<Vec<SiegeClassVisual>, String> {
    // FS_GetFileList returns files directly in ext_data/Siege/Classes. AssetSearchPath names are
    // normalized to lowercase, which is equivalent for JKA's case-insensitive qpath lookup.
    let names = assets
        .names()
        .filter(|name| {
            name.starts_with(CLASS_DIRECTORY)
                && name.ends_with(CLASS_EXTENSION)
                && !name[CLASS_DIRECTORY.len()..].contains('/')
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let mut classes = Vec::new();
    for name in names {
        let Some(asset) = assets
            .read(&name, MAX_CLASS_FILE)
            .map_err(|error| format!("{name}: {error}"))?
        else {
            continue;
        };
        if let Some(class) = parse_siege_class(&asset.bytes)? {
            classes.push(class.visual());
        }
    }
    Ok(classes)
}

pub fn parse_siege_class_visual(bytes: &[u8]) -> Result<Option<SiegeClassVisual>, String> {
    Ok(parse_siege_class(bytes)?.map(|class| class.visual()))
}

pub fn parse_siege_class(bytes: &[u8]) -> Result<Option<SiegeClass>, String> {
    if bytes.len() >= 4096 { return Ok(None); }
    let text = std::str::from_utf8(bytes).map_err(|_| "Siege class file is not valid UTF-8/ASCII".to_owned())?;
    let description = siege_paired_value(text, "description")?
        .unwrap_or_else(|| "DESCRIPTION UNAVAILABLE".to_owned());
    let class_info = siege_value_group(text, "ClassInfo")?
        .ok_or_else(|| "Siege class missing ClassInfo group".to_owned())?;
    let get = |key: &str| siege_paired_value(&class_info, key);
    let name = get("name")?.ok_or_else(|| "Siege class without name entry".to_owned())?;
    let parse_i32 = |key: &str, default: i32| -> Result<i32, String> {
        Ok(get(key)?.map(|v| v.trim().parse::<i32>().unwrap_or(0)).unwrap_or(default))
    };
    let forced_saber_color = get("sabercolor")?.map(|v| v.trim().parse::<i32>().unwrap_or(0));
    let forced_saber2_color = get("saber2color")?.map(|v| v.trim().parse::<i32>().unwrap_or(0));
    let max_health = parse_i32("maxhealth", 100)?;
    let start_health = parse_i32("starthealth", max_health)?;
    let mut max_armor = parse_i32("maxarmor", 0)?;
    let start_armor = parse_i32("startarmor", max_armor)?;
    if max_armor == 0 && start_armor != 0 {
        max_armor = start_armor;
    }
    let class_flags = get("classflags")?
        .map(|value| parse_class_flags(&value))
        .unwrap_or(0);
    Ok(Some(SiegeClass {
        name, description,
        forced_model: get("model")?.unwrap_or_default(),
        forced_skin: get("skin")?.unwrap_or_default(),
        saber1: get("saber1")?.unwrap_or_default(),
        saber2: get("saber2")?.unwrap_or_default(),
        saber_style: get("saberstyle")?.unwrap_or_default(),
        forced_saber_color, forced_saber2_color,
        weapons: get("weapons")?.unwrap_or_default(),
        force_powers: get("forcepowers")?.unwrap_or_default(),
        class_flags,
        max_health,
        start_health,
        max_armor,
        start_armor,
        speed: get("speed")?.and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0),
        ui_shader: get("uishader")?.unwrap_or_default(),
        class_shader: get("class_shader")?.unwrap_or_default(),
        holdables: get("holdables")?.unwrap_or_default(),
        powerups: get("powerups")?.unwrap_or_default(),
    }))
}


fn parse_class_flags(value: &str) -> u32 {
    value
        .split(|c: char| c == '|' || c.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .fold(0u32, |flags, name| {
            flags | match name.to_ascii_uppercase().as_str() {
                "CFL_MORESABERDMG" => CFL_MORESABERDMG,
                "CFL_STRONGAGAINSTPHYSICAL" => CFL_STRONGAGAINSTPHYSICAL,
                "CFL_FASTFORCEREGEN" => CFL_FASTFORCEREGEN,
                "CFL_STATVIEWER" => CFL_STATVIEWER,
                "CFL_HEAVYMELEE" => CFL_HEAVYMELEE,
                "CFL_SINGLE_ROCKET" => CFL_SINGLE_ROCKET,
                "CFL_CUSTOMSKEL" => CFL_CUSTOMSKEL,
                "CFL_EXTRA_AMMO" => CFL_EXTRA_AMMO,
                _ => 0,
            }
        })
}

pub fn parse_siege_team(bytes: &[u8]) -> Result<Option<SiegeTeamTheme>, String> {
    if bytes.len() >= MAX_SIEGE_INFO_SIZE + 1 { return Ok(None); }
    let text = std::str::from_utf8(bytes).map_err(|_| "Siege team file is not valid UTF-8/ASCII".to_owned())?;
    let Some(name) = siege_paired_value(text, "name")? else { return Ok(None); };
    let friendly_shader = siege_paired_value(text, "FriendlyShader")?.unwrap_or_default();
    let classes_group = siege_value_group(text, "Classes")?.unwrap_or_default();
    let mut class_names = Vec::new();
    for index in 1..=16 {
        let key = format!("class{index}");
        match siege_paired_value(&classes_group, &key)? {
            Some(value) if !value.is_empty() => class_names.push(value),
            _ => break,
        }
    }
    Ok(Some(SiegeTeamTheme { name, class_names, friendly_shader }))
}

pub fn parse_siege_map(
    map_name: &str,
    bytes: &[u8],
    team1_override: Option<&str>,
    team2_override: Option<&str>,
) -> Result<Option<SiegeMapDefinition>, String> {
    if bytes.len() >= MAX_SIEGE_INFO_SIZE + 1 { return Ok(None); }
    let text = std::str::from_utf8(bytes).map_err(|_| "Siege map file is not valid UTF-8/ASCII".to_owned())?;
    let teams = siege_value_group(text, "Teams")?.ok_or_else(|| "Siege map missing Teams group".to_owned())?;
    let map_team1 = siege_paired_value(&teams, "team1")?.unwrap_or_default();
    let map_team2 = siege_paired_value(&teams, "team2")?.unwrap_or_default();
    let team1_name = usable_team_override(team1_override).unwrap_or(map_team1.as_str());
    let team2_name = usable_team_override(team2_override).unwrap_or(map_team2.as_str());
    let team1 = parse_map_team(text, team1_name)?;
    let team2 = parse_map_team(text, team2_name)?;
    Ok(Some(SiegeMapDefinition {
        map_name: normalize_map_name(map_name),
        source_text: text.to_owned(),
        map_graphic: siege_paired_value(text, "mapgraphic")?.unwrap_or_else(|| "gfx/mplevels/siege1_hoth".to_owned()),
        mission_name: siege_paired_value(text, "missionname")?.unwrap_or_else(|| " ".to_owned()),
        team1, team2,
    }))
}

fn usable_team_override(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("none"))
}

fn parse_map_team(text: &str, group_name: &str) -> Result<SiegeMapTeam, String> {
    if group_name.is_empty() { return Err("Siege map team has no group name".to_owned()); }
    let group = siege_value_group(text, group_name)?.ok_or_else(|| format!("No Siege team entry for '{group_name}'"))?;
    let timed_ms = siege_paired_value(&group, "Timed")?
        .and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0).saturating_mul(1000);
    let mut objectives = Vec::new();
    for number in 1..32 {
        let key = format!("Objective{number}");
        let Some(obj) = siege_value_group(&group, &key)? else { break; };
        let value = |key: &str| siege_paired_value(&obj, key);
        objectives.push(SiegeObjective {
            number,
            description: value("objdesc")?.unwrap_or_else(|| "UNSPECIFIED".to_owned()),
            graphic: value("objgfx")?.unwrap_or_else(|| "UNSPECIFIED".to_owned()),
            map_icon: value("mapicon")?.unwrap_or_else(|| "UNSPECIFIED".to_owned()),
            lit_map_icon: value("litmapicon")?.unwrap_or_else(|| "UNSPECIFIED".to_owned()),
            done_map_icon: value("donemapicon")?.unwrap_or_else(|| "UNSPECIFIED".to_owned()),
            map_pos: value("mappos")?.unwrap_or_else(|| "0 0 32 32".to_owned()),
            goal_name: value("goalname")?.unwrap_or_default(),
            final_objective: value("final")?.and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) != 0,
            message_team1: value("message_team1")?.unwrap_or_default(),
            message_team2: value("message_team2")?.unwrap_or_default(),
            sound_team1: value("sound_team1")?.unwrap_or_default(),
            sound_team2: value("sound_team2")?.unwrap_or_default(),
        });
    }
    Ok(SiegeMapTeam {
        group_name: group_name.to_owned(),
        team_icon: siege_paired_value(&group, "TeamIcon")?.unwrap_or_default(),
        timed_ms,
        use_team: siege_paired_value(&group, "UseTeam")?.unwrap_or_default(),
        friendly_shader: siege_paired_value(&group, "FriendlyShader")?.unwrap_or_default(),
        briefing: siege_paired_value(&group, "briefing")?.unwrap_or_default(),
        won_round: siege_paired_value(&group, "wonround")?.unwrap_or_default(),
        lost_round: siege_paired_value(&group, "lostround")?.unwrap_or_default(),
        roundover_sound_wewon: siege_paired_value(&group, "roundover_sound_wewon")?.unwrap_or_default(),
        roundover_sound_welost: siege_paired_value(&group, "roundover_sound_welost")?.unwrap_or_default(),
        objectives,
    })
}

fn normalize_map_name(name: &str) -> String {
    let normalized = name.replace('\\', "/");
    let normalized = normalized.strip_prefix("maps/").unwrap_or(&normalized);
    normalized.strip_suffix(".siege").or_else(|| normalized.strip_suffix(".bsp")).unwrap_or(normalized).to_ascii_lowercase()
}

pub fn find_siege_class_visual<'a>(
    classes: &'a [SiegeClassVisual],
    name: &str,
) -> Option<&'a SiegeClassVisual> {
    classes
        .iter()
        .find(|class| class.name.eq_ignore_ascii_case(name))
}

/// Safe Rust equivalent of `BG_SiegeGetValueGroup` for the syntax used by stock class files.
fn siege_value_group(text: &str, group: &str) -> Result<Option<String>, String> {
    let data = text.as_bytes();
    let mut i = 0usize;
    while i < data.len() {
        skip_space_and_line_comments(data, &mut i);
        if i >= data.len() {
            break;
        }
        if data[i] == b'{' {
            skip_group(data, &mut i)?;
            continue;
        }
        if data[i] == b'}' {
            i += 1;
            continue;
        }

        let token_start = i;
        while i < data.len()
            && !matches!(data[i], b' ' | b'\n' | b'\r' | b'\t' | b'{')
        {
            if data[i] == b'/' && i + 1 < data.len() && data[i + 1] == b'/' {
                break;
            }
            i += 1;
        }
        let token = &text[token_start..i];

        if i + 1 < data.len() && data[i] == b'/' && data[i + 1] == b'/' {
            skip_line_comment(data, &mut i);
        }
        while i < data.len() && matches!(data[i], b' ' | b'\n' | b'\r' | b'\t') {
            i += 1;
        }

        if i < data.len() && data[i] == b'{' {
            if token.eq_ignore_ascii_case(group) {
                return extract_group_contents(text, i).map(Some);
            }
            skip_group(data, &mut i)?;
        } else {
            while i < data.len() && !matches!(data[i], b'\n' | b'\r') {
                i += 1;
            }
        }
    }
    Ok(None)
}

/// Safe Rust equivalent of `BG_SiegeGetPairedValue`. It deliberately does not recurse into
/// subgroups, and quoted values may contain spaces exactly as in OpenJK.
fn siege_paired_value(text: &str, key: &str) -> Result<Option<String>, String> {
    let data = text.as_bytes();
    let mut i = 0usize;
    while i < data.len() {
        while i < data.len() && matches!(data[i], b' ' | b'{' | b'}' | b'\n' | b'\r' | b'\t') {
            if data[i] == b'{' {
                skip_group(data, &mut i)?;
            } else {
                i += 1;
            }
        }
        if i >= data.len() {
            break;
        }
        if i + 1 < data.len() && data[i] == b'/' && data[i + 1] == b'/' {
            skip_line_comment(data, &mut i);
            continue;
        }

        let key_start = i;
        while i < data.len() && !matches!(data[i], b' ' | b'\n' | b'\r' | b'\t') {
            if data[i] == b'/' && i + 1 < data.len() && data[i + 1] == b'/' {
                break;
            }
            i += 1;
        }
        let candidate = &text[key_start..i];

        let mut look = i;
        while look < data.len() && matches!(data[look], b' ' | b'\n' | b'\r') {
            look += 1;
        }
        if look < data.len() && data[look] == b'{' {
            i = look;
            skip_group(data, &mut i)?;
            continue;
        }

        if i + 1 < data.len() && data[i] == b'/' && data[i + 1] == b'/' {
            return Err(format!("found comment, expected value for '{key}'"));
        }

        if candidate.eq_ignore_ascii_case(key) {
            while i < data.len() && matches!(data[i], b' ' | b'\n' | b'\r' | b'\t') {
                i += 1;
            }
            if i >= data.len() {
                return Err(format!("unexpected EOF while looking for value '{key}'"));
            }
            if data[i] == b'"' {
                i += 1;
                let start = i;
                while i < data.len() && data[i] != b'"' {
                    if data[i] == b'/' && i + 1 < data.len() && data[i + 1] == b'/' {
                        break;
                    }
                    i += 1;
                }
                if i >= data.len() {
                    return Err(format!("unexpected EOF while looking for endquote for '{key}'"));
                }
                return Ok(Some(text[start..i].to_owned()));
            }
            let start = i;
            while i < data.len() && !matches!(data[i], b' ' | b'\n' | b'\r') {
                if data[i] == b'/' && i + 1 < data.len() && data[i + 1] == b'/' {
                    break;
                }
                i += 1;
            }
            return Ok(Some(text[start..i].to_owned()));
        }

        // OpenJK skips the rest of the line after a non-matching key so a value cannot be
        // mistaken for another key.
        while i < data.len() && data[i] != b'\n' {
            i += 1;
        }
    }
    Ok(None)
}

fn extract_group_contents(text: &str, opening: usize) -> Result<String, String> {
    let data = text.as_bytes();
    let mut depth = 0i32;
    let mut i = opening;
    let content_start = opening + 1;
    while i < data.len() {
        match data[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(text[content_start..i].replace('\t', " "));
                }
                if depth < 0 {
                    return Err("unexpected closing bracket in Siege group".to_owned());
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err("Siege group is missing a closing bracket".to_owned())
}

fn skip_group(data: &[u8], i: &mut usize) -> Result<(), String> {
    if *i >= data.len() || data[*i] != b'{' {
        return Ok(());
    }
    let mut depth = 0i32;
    while *i < data.len() {
        match data[*i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                *i += 1;
                if depth == 0 {
                    return Ok(());
                }
                if depth < 0 {
                    return Err("unexpected closing bracket in Siege file".to_owned());
                }
                continue;
            }
            _ => {}
        }
        *i += 1;
    }
    Err("opening bracket without matching closing bracket in Siege file".to_owned())
}

fn skip_space_and_line_comments(data: &[u8], i: &mut usize) {
    loop {
        while *i < data.len() && matches!(data[*i], b' ' | b'\n' | b'\r' | b'\t') {
            *i += 1;
        }
        if *i + 1 < data.len() && data[*i] == b'/' && data[*i + 1] == b'/' {
            skip_line_comment(data, i);
            continue;
        }
        break;
    }
}

fn skip_line_comment(data: &[u8], i: &mut usize) {
    while *i < data.len() && !matches!(data[*i], b'\n' | b'\r') {
        *i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siege_theme_resolves_classes_and_map_objectives() {
        let theme = parse_siege_team(br#"
            name "Rebels"
            FriendlyShader sprites/rebel_attack
            Classes {
                class1 "Rebel Tech"
                class2 "Rebel Soldier"
            }
        "#).unwrap().unwrap();
        assert_eq!(theme.class_names, ["Rebel Tech", "Rebel Soldier"]);
        assert_eq!(theme.friendly_shader, "sprites/rebel_attack");
        let text = br#"
            Teams {
                team1 Rebels
                team2 Imperials
            }
            Rebels {
                UseTeam Rebels
                Timed 1200
                Objective1 {
                    final 1
                    objdesc "Capture the base"
                    message_team2 @SIEGE_LOST_BASE
                }
            }
            Imperials {
                UseTeam Imperials
            }
        "#;
        let map = parse_siege_map("maps/mp/siege_test.siege", text, Some("none"), None)
            .unwrap().unwrap();
        assert_eq!(map.map_name, "mp/siege_test");
        assert_eq!(map.team1.timed_ms, 1_200_000);
        assert_eq!(map.team1.objectives[0].message_team2, "@SIEGE_LOST_BASE");
        assert!(map.team1.objectives[0].final_objective);
        let switched = parse_siege_map("mp/siege_test", text, Some("Imperials"), Some("Rebels"))
            .unwrap().unwrap();
        assert_eq!(switched.team2.timed_ms, 1_200_000);
    }

    #[test]
    #[ignore = "requires JKA_TEST_BASE with stock PK3s"]
    fn stock_siege_catalog_resolves_all_three_maps() {
        let base = std::env::var_os("JKA_TEST_BASE").expect("set JKA_TEST_BASE");
        let mut assets = AssetSearchPath::open(std::path::Path::new(&base)).unwrap();
        let catalog = load_siege_catalog(&mut assets).unwrap();
        for name in ["mp/siege_hoth", "mp/siege_desert", "mp/siege_korriban"] {
            let map = catalog.map(name).expect(name);
            for team in [&map.team1, &map.team2] {
                assert!(!team.objectives.is_empty(), "{name}: missing objectives");
                let theme = catalog.team_theme(&team.use_team).expect(&team.use_team);
                assert!(!theme.class_names.is_empty(), "{name}: missing classes");
                for class in &theme.class_names {
                    assert!(catalog.class(class).is_some(), "{name}: missing {class}");
                }
            }
        }
        println!("Loaded {} classes, {} teams, {} maps", catalog.classes.len(), catalog.teams.len(), catalog.maps.len());
    }

    #[test]
    fn extracts_visual_fields_from_classinfo_only() {
        let file = br#"
            description "A class outside the group"
            Other { model wrong }
            ClassInfo
            {
                name "Jedi Guardian"
                model jedi_hm
                skin |head_a1|torso_a1|lower_a1
                sabercolor 3
                saber2color 5
                weapons WP_SABER
                Nested { model ignored }
            }
        "#;
        assert_eq!(
            parse_siege_class_visual(file).unwrap(),
            Some(SiegeClassVisual {
                name: "Jedi Guardian".to_owned(),
                forced_model: "jedi_hm".to_owned(),
                forced_skin: "|head_a1|torso_a1|lower_a1".to_owned(),
                forced_saber_color: Some(3),
                forced_saber2_color: Some(5),
            })
        );
    }

    #[test]
    fn class_name_lookup_is_case_insensitive() {
        let classes = vec![SiegeClassVisual {
            name: "Heavy Weapons".to_owned(),
            forced_model: "stormtrooper".to_owned(),
            forced_skin: "default".to_owned(),
            forced_saber_color: None,
            forced_saber2_color: None,
        }];
        assert_eq!(
            find_siege_class_visual(&classes, "heavy weapons")
                .unwrap()
                .forced_model,
            "stormtrooper"
        );
    }
}

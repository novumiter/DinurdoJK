//! TaystJK non-jaPRO global cosmetics compatibility.
//!
//! The wire format is the original numeric `color1`/`color2` saber value
//! immediately followed by a <=14-byte MD3 basename (hat/cape respectively).
//! Stock servers relay these strings in CS_PLAYERS `c1`/`c2` unchanged.  Keep
//! color and cosmetic separately in Rust so RGB fallback never loses either.

pub const MAX_NAME_LEN: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot { Hat, Cape }

impl Slot {
    pub fn model_dir(self) -> &'static str {
        match self { Self::Hat => "models/cosmetics/hats", Self::Cape => "models/cosmetics/capes" }
    }
    pub fn settings_dir(self) -> &'static str {
        match self { Self::Hat => "settings/cosmetics/hats", Self::Cape => "settings/cosmetics/capes" }
    }
}

/// Cosmetic basenames, not paths. Reject userinfo injection and traversal before
/// constructing a virtual-filesystem qpath. TaystJK's MAX_COSMETIC_LENGTH is 15
/// including the NUL terminator (14 actual characters).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_LEN
        && !name.as_bytes()[0].is_ascii_digit()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        && !name.eq_ignore_ascii_case("none")
}

/// Strict for locally authored values; the network reader intentionally tolerates
/// unknown suffixes and simply doesn't render them.
pub fn parse_cvar(value: &str) -> Result<(u8, String), String> {
    let value = value.trim();
    let end = value.bytes().take_while(|b| b.is_ascii_digit()).count();
    if end == 0 { return Err("expected a saber colour followed by an optional cosmetic basename".into()); }
    let color = value[..end].parse::<u8>()
        .map_err(|_| "saber colour must be 0..255".to_owned())?;
    let name = &value[end..];
    if !name.is_empty() && !valid_name(name) {
        return Err("cosmetic name must be 1..14 ASCII letters/digits/_/- (starting with a letter or _)".into());
    }
    Ok((color, name.to_owned()))
}

pub fn encode(color: u8, name: &str) -> String {
    if valid_name(name) { format!("{color}{name}") } else { color.to_string() }
}

/// Equivalent to TaystJK CG_NewClientInfo's atoi + REMOVE_DIGITS_INITIAL.
/// Never trust an arbitrary remote player name as an asset path.
pub fn decode_wire(value: &[u8]) -> (i32, String) {
    let end = value.iter().take_while(|b| b.is_ascii_digit()).count();
    let color = std::str::from_utf8(&value[..end]).ok()
        .and_then(|s| s.parse::<i32>().ok()).unwrap_or(4);
    let name = std::str::from_utf8(&value[end..]).ok().unwrap_or("");
    (color, if valid_name(name) { name.to_owned() } else { String::new() })
}

pub fn model_path(slot: Slot, name: &str) -> Option<String> {
    valid_name(name).then(|| format!("{}/{}.md3", slot.model_dir(), name))
}

/// `CG_LoadCustomCosmeticOffsets`: exact case-sensitive model/skin first,
/// otherwise longest matching prefix before '*'; modelFallback defaults false.
/// The values are world-axis XYZ offsets, *not* offsets in the attachment axes.
pub fn offsets(json: &serde_json::Value, model: &str, skin: &str) -> [f32; 3] {
    fn choose<'a>(object: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        let map = object.as_object()?;
        if let Some(exact) = map.get(key).filter(|v| v.is_object()) { return Some(exact); }
        map.iter().filter_map(|(pattern, v)| {
            let (prefix, _) = pattern.split_once('*')?;
            (v.is_object() && !prefix.is_empty() && key.starts_with(prefix))
                .then_some((prefix.len(), v))
        }).max_by_key(|(len, _)| *len).map(|(_, v)| v)
    }
    fn xyz(node: &serde_json::Value) -> [f32; 3] {
        let number = |name: &str| node.get(name).and_then(serde_json::Value::as_f64)
            .filter(|n| n.is_finite()).map(|n| n.trunc() as f32);
        match (number("xOffset"), number("yOffset"), number("zOffset")) {
            (Some(x), Some(y), Some(z)) => [x, y, z],
            _ => [0.0; 3],
        }
    }
    let Some(model_node) = choose(json, model) else { return [0.0; 3] };
    if let Some(skin_node) = choose(model_node, skin) { return xyz(skin_node); }
    if model_node.get("modelFallback").and_then(serde_json::Value::as_bool).unwrap_or(false) {
        xyz(model_node)
    } else { [0.0; 3] }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tayst_wire_format_and_rgb() {
        assert_eq!(encode(4, "santahat"), "4santahat");
        assert_eq!(decode_wire(b"6vadercape"), (6, "vadercape".into()));
        assert_eq!(decode_wire(b"2"), (2, String::new()));
        assert_eq!(parse_cvar("4santahat"), Ok((4, "santahat".into())));
        assert_eq!(parse_cvar("6"), Ok((6, String::new())));
    }
    #[test]
    fn never_interpret_a_qpath_as_a_cosmetic() {
        for bad in ["../../x", "a\\b", "a;b", "hat.md3", "abcdefghijklmnop", "2foo"] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert_eq!(decode_wire(b"4../../evil"), (4, String::new()));
    }
    #[test]
    fn tayst_offset_selection() {
        let json = serde_json::json!({"ky*": {"modelFallback": true,
           "xOffset": 1, "yOffset": 2, "zOffset": 3,
           "de*": {"xOffset": 4, "yOffset": 5, "zOffset": 6},
           "defa*": {"xOffset": 7, "yOffset": 8, "zOffset": 9}}});
        assert_eq!(offsets(&json, "kyle", "default"), [7.0,8.0,9.0]);
        assert_eq!(offsets(&json, "kyle", "blue"), [1.0,2.0,3.0]);
        assert_eq!(offsets(&json, "desann", "default"), [0.0;3]);
    }
}

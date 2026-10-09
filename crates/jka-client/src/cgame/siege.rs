//! Wire-visible Siege state shared by live play and demos.
//! Semantics follow TaystJK/OpenJK rather than interpreting the strings ad hoc in UI code.

use super::parse_i32_ascii;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SiegeRoundState {
    pub state: i32,
    pub server_time: i32,
    /// OpenJK assigns `cgSiegeRoundBeganTime` when state is 0 or 2.
    pub round_began_time: Option<i32>,
}

pub fn parse_round_state(value: &[u8], now: i32) -> SiegeRoundState {
    let mut fields = value.splitn(2, |b| *b == b'|');
    let state = fields.next().and_then(parse_i32_ascii).unwrap_or(0);
    let server_time = fields.next().and_then(parse_i32_ascii).unwrap_or(now);
    SiegeRoundState {
        state,
        server_time,
        round_began_time: matches!(state, 0 | 2).then_some(server_time),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiegeObjectiveStatus {
    pub team1: Vec<bool>,
    pub team2: Vec<bool>,
}

/// Exact `CS_SIEGE_OBJECTIVES` grammar from CG_ParseSiegeObjectiveStatus:
/// every '-' starts an objective entry, the following byte is complete iff it is '1', and '|'
/// switches from Siege team 1 to Siege team 2.
pub fn parse_objective_status(value: &[u8]) -> SiegeObjectiveStatus {
    let mut out = SiegeObjectiveStatus::default();
    let mut team2 = false;
    let mut i = 0usize;
    while i < value.len() {
        match value[i] {
            b'|' => team2 = true,
            b'-' => {
                i += 1;
                let complete = value.get(i) == Some(&b'1');
                if team2 { out.team2.push(complete); } else { out.team1.push(complete); }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiegeExtendedDataEntry {
    pub client: i32,
    pub health: i32,
    pub max_health: i32,
    pub ammo: i32,
}

/// One TaystJK `sxd` argv: `client|health|maxhealth|ammo`.
pub fn parse_extended_data_entry(value: &[u8]) -> Option<SiegeExtendedDataEntry> {
    let mut fields = value.split(|b| *b == b'|');
    let client = fields.next().and_then(parse_i32_ascii)?;
    let health = fields.next().and_then(parse_i32_ascii)?;
    let max_health = fields.next().and_then(parse_i32_ascii)?;
    let ammo = fields.next().and_then(parse_i32_ascii)?;
    (fields.next().is_none()).then_some(SiegeExtendedDataEntry {
        client, health, max_health, ammo,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_is_state_pipe_server_time() {
        assert_eq!(parse_round_state(b"2|12345", 9), SiegeRoundState { state: 2, server_time: 12345, round_began_time: Some(12345) });
        assert_eq!(parse_round_state(b"1|77", 9).round_began_time, None);
    }

    #[test]
    fn objectives_follow_legacy_dash_pipe_format() {
        assert_eq!(parse_objective_status(b"-1-0-1|-0-1"), SiegeObjectiveStatus { team1: vec![true, false, true], team2: vec![false, true] });
    }

    #[test]
    fn sxd_entry_has_four_pipe_fields() {
        assert_eq!(parse_extended_data_entry(b"7|83|100|12"), Some(SiegeExtendedDataEntry { client: 7, health: 83, max_health: 100, ammo: 12 }));
        assert_eq!(parse_extended_data_entry(b"7|83|100"), None);
    }
}

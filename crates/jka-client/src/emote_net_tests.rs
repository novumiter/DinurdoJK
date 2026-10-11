use super::{Receiver, RemoteEmoteStart};
use crate::emote_net_codec;
use jka_assets::animation::animation_index;
use jka_protocol::{gamestate::{EntityState, ENTITY_FIELDS}, server::{PlayerState, Snapshot}};

fn set(es: &mut EntityState, name: &str, bits: u32) {
    let index = ENTITY_FIELDS.iter().position(|(key, _)| *key == name).unwrap();
    es.fields[index] = bits;
}

fn animation(name: &str) -> u32 {
    animation_index(name).unwrap() as u32
}

fn snapshot(time: i32, torso: &str, legs: &str) -> Snapshot {
    let mut es = EntityState { number: 1, fields: [0; ENTITY_FIELDS.len()] };
    set(&mut es, "eType", 1);
    set(&mut es, "clientNum", 1);
    set(&mut es, "torsoAnim", animation(torso));
    set(&mut es, "legsAnim", animation(legs));
    let mut angles = [0; 3];
    assert!(emote_net_codec::encode_cmd_angles(&mut angles, [0; 3], 42));
    for (axis, short) in angles.into_iter().enumerate().take(2) {
        let degrees = (short as i16) as f32 * (360.0 / 65536.0);
        set(&mut es, &format!("apos.trBase[{axis}]"), degrees.to_bits());
    }
    Snapshot {
        server_time: time, message_num: time, delta_num: 0, snap_flags: 0,
        ping: 0, server_command_num: 0, area_mask: [0; 32],
        player_state: PlayerState::default(), vehicle_player_state: None, entities: vec![es],
    }
}

fn idle(time: i32) -> Snapshot {
    snapshot(time, "BOTH_STAND1", "BOTH_STAND1")
}

fn taunt(time: i32) -> Snapshot {
    snapshot(time, "BOTH_ENGAGETAUNT", "BOTH_STAND1")
}

#[test]
fn new_taunt_decodes_once_and_flip_restarts_it() {
    let mut receiver = Receiver::default();
    assert!(receiver.on_snapshot(&idle(100), 0).is_empty());
    assert_eq!(receiver.on_snapshot(&taunt(150), 0), vec![RemoteEmoteStart {
        client_num: 1, emote_id: 42, server_time: 150,
    }]);
    assert!(receiver.on_snapshot(&taunt(200), 0).is_empty());
    let mut restarted = taunt(250);
    set(&mut restarted.entities[0], "torsoFlip", 1);
    assert_eq!(receiver.on_snapshot(&restarted, 0).len(), 1);
    assert!(receiver.on_snapshot(&restarted, 0).is_empty());
}

#[test]
fn unrelated_legs_change_does_not_restart_held_torso_taunt() {
    let mut receiver = Receiver::default();
    receiver.on_snapshot(&idle(100), 0);
    assert_eq!(receiver.on_snapshot(&taunt(150), 0).len(), 1);
    let mut changed = taunt(200);
    set(&mut changed.entities[0], "legsFlip", 1);
    assert!(receiver.on_snapshot(&changed, 0).is_empty());
    changed.server_time = 250;
    set(&mut changed.entities[0], "legsAnim", animation("BOTH_RUN1"));
    assert!(receiver.on_snapshot(&changed, 0).is_empty());
}

#[test]
fn unrelated_torso_change_does_not_restart_held_legs_taunt() {
    let mut receiver = Receiver::default();
    receiver.on_snapshot(&idle(100), 0);
    let mut next = snapshot(150, "BOTH_STAND1", "BOTH_ENGAGETAUNT");
    assert_eq!(receiver.on_snapshot(&next, 0).len(), 1);
    next.server_time = 200;
    set(&mut next.entities[0], "torsoFlip", 1);
    assert!(receiver.on_snapshot(&next, 0).is_empty());
    next.server_time = 250;
    set(&mut next.entities[0], "legsFlip", 1);
    assert_eq!(receiver.on_snapshot(&next, 0).len(), 1);
}

#[test]
fn servercount_restart_acquires_baseline_even_at_same_server_time() {
    for time in [100, 150] {
        let mut receiver = Receiver::default();
        receiver.on_snapshot(&idle(100), 0);
        let mut restarted = taunt(time);
        restarted.snap_flags = super::SNAPFLAG_SERVERCOUNT;
        assert!(receiver.needs_reset(&restarted));
        assert!(receiver.on_snapshot(&restarted, 0).is_empty());
        assert!(!receiver.needs_reset(&restarted));
        let mut next = idle(time + 50);
        next.snap_flags = restarted.snap_flags;
        receiver.on_snapshot(&next, 0);
        next = taunt(time + 100);
        next.snap_flags = restarted.snap_flags;
        assert_eq!(receiver.on_snapshot(&next, 0).len(), 1);
    }
}

#[test]
fn time_rollback_and_explicit_reset_acquire_baselines() {
    let mut receiver = Receiver::default();
    receiver.on_snapshot(&idle(100), 0);
    assert!(receiver.needs_reset(&taunt(50)));
    assert!(receiver.on_snapshot(&taunt(50), 0).is_empty());
    receiver.on_snapshot(&idle(100), 0);
    receiver.reset();
    assert!(receiver.on_snapshot(&taunt(150), 0).is_empty());
}

#[test]
fn pvs_entry_and_reentry_do_not_start_existing_taunts() {
    let mut receiver = Receiver::default();
    assert!(receiver.on_snapshot(&taunt(100), 0).is_empty());
    let mut hidden = idle(150);
    hidden.entities.clear();
    receiver.on_snapshot(&hidden, 0);
    assert!(receiver.on_snapshot(&taunt(200), 0).is_empty());
}

#[test]
fn spectator_follow_uses_connection_identity_to_exclude_self() {
    let mut receiver = Receiver::default();
    let mut baseline = idle(100);
    baseline.player_state.set_field_bits("clientNum", 1);
    baseline.player_state.set_field_bits("pm_flags", 4096); // PMF_FOLLOW
    receiver.on_snapshot(&baseline, 0);
    let mut next = taunt(150);
    next.player_state = baseline.player_state;
    assert_eq!(receiver.on_snapshot(&next, 0).len(), 1);

    let mut receiver = Receiver::default();
    receiver.on_snapshot(&idle(100), 1);
    assert!(receiver.on_snapshot(&taunt(150), 1).is_empty());
}

#[test]
fn npcs_and_bad_signatures_never_emit_candidates() {
    let mut receiver = Receiver::default();
    receiver.on_snapshot(&idle(100), 0);
    let mut npc = taunt(150);
    set(&mut npc.entities[0], "eType", 2); // ET_NPC
    assert!(receiver.on_snapshot(&npc, 0).is_empty());
    receiver.on_snapshot(&idle(200), 0);
    let mut ordinary = taunt(250);
    set(&mut ordinary.entities[0], "apos.trBase[0]", 0.0_f32.to_bits());
    assert!(receiver.on_snapshot(&ordinary, 0).is_empty());
    // Seeing the signature later in a held animation must still not trigger.
    assert!(receiver.on_snapshot(&taunt(300), 0).is_empty());
}

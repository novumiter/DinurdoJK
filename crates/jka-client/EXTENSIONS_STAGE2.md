# DinurdoJK Rhai extensions — Stage 2 (game state + accepted events)

**Applies on top of Stage 1.** No new Cargo dependencies. `api_version = 1` remains valid.
Run `cargo check -p DinurdoJK` after applying; this checkpoint was not compiled in the sandbox (no Rust toolchain).

## Run-time scheduling

- The app queues a snapshot at most once every 16 ms (~62.5 Hz). It does not call Rhai every render frame.
- The bounded two-frame channel uses `try_send`, so a slow script never blocks the main/render thread.
- The extension worker performs Rhai callbacks. A slow script may delay its **own** callbacks. Accepted events are queued up to 1,024; if that queue overflows, the oldest event is discarded. Callbacks run in delivery order within each delivered batch.
- Each extension gets its own engine/scope. A hook with a runtime error reports it once and is disabled until that extension's next successful reload; other hooks/extensions continue running.
- Top-level code executes only on activation/reload. Callbacks do **not** re-evaluate top-level code.
- Events are taken from CGame's accepted presentation event path, not raw predicted effects. Demo seeking clears old queued events and damage deduplication; seek replay history is not sent as live events.

## Read-only functions

The following host calls can be used within Rhai. Each returns a fresh, modifiable Rhai map (it cannot mutate game state), or `()` when unavailable:

- `playerstate()` — current **presented** playerstate (interpolated/predicted for local/followed player). Returns `()` for freecam or if snapshot player ownership doesn't match the viewer.
- `snapshot_playerstate()` — currently accepted snapshot's playerstate, only if it belongs to the effective viewer. It may lag predicted display state.
- `session_info()` — map: `map`, `server_time` (ms), `viewer_client` (-1 if unavailable), `is_live`, `is_demo`.

Playerstate properties: `health`, `armor`, `weapon`, `client_num`, `pm_type`, `duel_time`, `origin` `[x,y,z]`, `velocity` `[x,y,z]`, `viewangles` `[pitch,yaw,roll]`, `stats`, `persistant` (stock JKA spelling), `ammo`, `powerups` (all arrays), and `fields` (map of *protocol field names* to values). Float netfields in `fields` are converted from IEEE-754 bits, and integral netfields are signed integers; for instance `ps.fields["fd.forcePower"]`, `ps.fields["velocity[0]"]` and `ps.fields["duelTime"]`.

**Do not confuse** `playerstate()` with the server snapshot, especially in demos and prediction. This stage does not yet expose the state of *other* players separately (that requires entity/client snapshot APIs).

## Callback functions

Define any of these optional Rhai functions:

- `fn on_update() { ... }` — updates at the capped extension rate.
- `fn on_event(e) { ... }` — every accepted protocol event plus synthesized `damage_taken` transitions.
- `fn on_player_killed(e) { ... }` — authoritative `EV_OBITUARY` only.
- `fn on_damage_taken(e) { ... }` — committed `damageEvent` + nonzero `damageCount` transition for the snapshot/predicted playerstate, deduplicated.
- `fn on_item_picked_up(e) { ... }` — `EV_ITEM_PICKUP`/`EV_GLOBAL_ITEM_PICKUP`.
- `fn on_weapon_fired(e) { ... }` — `EV_FIRE_WEAPON`/`EV_ALT_FIRE`.
- `fn on_saber_hit(e) { ... }` — `EV_SABER_HIT` (contact effect, **not** a guaranteed hit confirmation).

Event fields: `kind` (semantic name or `raw_event`), `name` (`EV_...`), `id` (stock event id or -1 for synthetic damage), `sequence` (accepted queue sequence, 0 for synthesized damage), `server_time`, `entity_num`, `parm`, `position` (JKA XYZ). The obituary events also expose `attacker`, `victim`, `attacker_name`, `victim_name`. Damage events include `amount` and `attacker` (from `PERS_ATTACKER`, not necessarily a valid player). Missing optional values are `()`.

### Example to paste into an existing extension

```rhai
let last_second = -1;

fn on_update() {
    let ps = playerstate();
    if ps == () { return; }
    let info = session_info();
    if info == () { return; }
    let second = info.server_time / 1000;
    if second != last_second {
        last_second = second;
        let v = ps.velocity;
        print(`Health ${ps.health}, armor ${ps.armor}, vx ${v[0]}, vy ${v[1]}`);
    }
}

fn on_player_killed(e) {
    print(`Obituary: ${e.attacker_name} -> ${e.victim_name}, MOD=${e.parm}`);
}

fn on_damage_taken(e) {
    print(`Received ${e.amount} damage`);
}

fn on_event(e) {
    if e.name == "EV_JUMP" { print("Jump!"); }
}
```

**HUD widgets and world drawing are not part of Stage 2.** Their producers will use this same bridge, in a future patch.

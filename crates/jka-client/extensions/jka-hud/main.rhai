// Retail Jedi Academy multiplayer HUD, drawn from the stock gfx/hud art.
// Layout is ui/hud.menu ("lefthud"/"righthud") and the tic/number logic is
// CG_DrawHealth/CG_DrawArmor/CG_DrawAmmo/CG_DrawForcePower/CG_DrawSaberStyle.
// Everything is laid out on the stock 640x480 canvas and scaled by height/480,
// so one virtual unit is `S` pixels. Both clusters hug their screen corner.
//
// Not reproduced (no data reaches scripts yet): the force-denied red flash and
// the ammo number's firing/pickup colours.
const PAD_Y = 12.0;      // widget-local room above the menu rect (armor tics poke out)
const W = 128.0;         // widget width in virtual units

// [image, x, y, w, h] in the menus' own coordinates. Left x is absolute inside
// "lefthud"; right x is relative to the screen's right edge (negative = inward).
const HEALTH_TICS = [
    ["gfx/hud/health_tic_1", 20.0, 24.0, 28.0, 28.0],
    ["gfx/hud/health_tic_2", 38.0, 33.0, 28.0, 28.0],
    ["gfx/hud/health_tic_3", 52.0, 47.0, 28.0, 28.0],
    ["gfx/hud/health_tic_4", 60.0, 65.0, 28.0, 28.0],
];
const ARMOR_TICS = [
    ["gfx/hud/armor_tic_1", 9.0, -6.0, 56.0, 56.0],
    ["gfx/hud/armor_tic_2", 34.0, 3.0, 56.0, 56.0],
    ["gfx/hud/armor_tic_3", 54.0, 24.0, 56.0, 56.0],
    ["gfx/hud/armor_tic_4", 63.0, 48.0, 56.0, 56.0],
];
const AMMO_TICS = [
    ["gfx/hud/ammo_tic_1", -48.0, 25.0, 28.0, 28.0],
    ["gfx/hud/ammo_tic_2", -66.0, 33.0, 28.0, 28.0],
    ["gfx/hud/ammo_tic_3", -80.0, 47.0, 28.0, 28.0],
    ["gfx/hud/ammo_tic_4", -88.0, 65.0, 28.0, 28.0],
];
const FORCE_TICS = [
    ["gfx/hud/force_tic_1", -65.0, -5.0, 56.0, 56.0],
    ["gfx/hud/force_tic_2", -89.0, 4.0, 56.0, 56.0],
    ["gfx/hud/force_tic_3", -109.0, 24.0, 56.0, 56.0],
    ["gfx/hud/force_tic_4", -119.0, 48.0, 56.0, 56.0],
];
// SS_FAST=1, SS_MEDIUM=2, SS_STRONG=3 (dual/staff have no art in the stock menu).
const SABER_STYLES = [
    (),
    ["gfx/hud/saber_fast", -85.0, 69.0, 24.0, 24.0],
    ["gfx/hud/saber_med", -70.0, 43.0, 26.0, 26.0],
    ["gfx/hud/saber_strong", -49.0, 24.0, 26.0, 26.0],
];
// ammoData[].max, indexed by weaponData[].ammoIndex.
const AMMO_MAX = [0, 100, 300, 300, 300, 25, 800, 10, 10, 10];

const HEALTH_COLOR = [0.835, 0.015, 0.015, 1.0];
const ARMOR_COLOR = [0.0, 0.613, 0.097, 1.0];
const FORCE_COLOR = [0.359, 0.524, 0.722, 1.0];
const AMMO_COLOR = [1.0, 0.658, 0.062, 1.0];
const RED = [1.0, 0.0, 0.0, 1.0];

// Placeholder registration so HUD Edit lists both clusters before the first
// tick; on_update re-registers at the real resolution.
hud_register("left", "JKA HUD: health / armor", W * 2.25, (112.0 + PAD_Y) * 2.25, "bottom-left");
hud_register("right", "JKA HUD: force / ammo", W * 2.25, (112.0 + PAD_Y) * 2.25, "bottom-right");

// Load every image now (one atlas write at startup) instead of the first time
// each tic or digit appears, which would stall a frame mid-fight.
for set in [HEALTH_TICS, ARMOR_TICS, AMMO_TICS, FORCE_TICS] {
    for t in set { hud_preload(t[0]); }
}
for t in SABER_STYLES { if t != () { hud_preload(t[0]); } }
hud_preload("gfx/hud/hudleft");
for name in ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "minus"] {
    hud_preload("gfx/2d/numbers/t_" + name);
}

let screen_h = 0;       // last resolution the widgets were registered for

// Only on_update itself can see top-level variables and constants (Rhai
// functions called from it are pure), so helpers take what they need or repeat
// the literal: 128.0 is W and 12.0 is PAD_Y.
// Widget-local pixels: left widgets use the menu's x as is, right widgets
// measure from their right edge, which is W units into the widget.
fn lx(x, s) { x * s }
fn rx(x, s) { (128.0 + x) * s }
fn ly(y, s) { (y + 12.0) * s }

fn tint_for(ps) {
    // jaPRO cg_tintHud: red/blue team tint, white otherwise.
    let team = ps.persistant[3];
    if team == 1 { [1.0, 0.5, 0.5, 1.0] } else if team == 2 { [0.5, 0.5, 1.0, 1.0] } else { [1.0, 1.0, 1.0, 1.0] }
}

// The tic loop shared by health, armor, ammo and force: draw from the last tic
// backwards, subtract one increment per tic, fade the partial one.
fn tics(id, set, value, inc, tint, right, skip_partial_last, s) {
    let v = value;
    let inc = if inc < 1 { 1 } else { inc };
    let i = set.len() - 1;
    while i >= 0 {
        if v <= 0 { break; }
        let a = tint[3];
        if v < inc { a *= v.to_float() / inc.to_float(); }
        let t = set[i];
        let x = if right { rx(t[1], s) } else { lx(t[1], s) };
        if !(skip_partial_last && i == 3 && v < inc) {
            hud_image(id, t[0], x, ly(t[2], s), t[3] * s, t[4] * s, [tint[0], tint[1], tint[2], a]);
        }
        v -= inc;
        i -= 1;
    }
}

// CG_DrawNumField(width 3, NUM_FONT_SMALL, no zero fill): right-aligned in three
// cells, one pixel gap between glyphs, range -99..999.
fn number(id, value, x, y, cw, ch, color, right, s) {
    let v = value;
    if v > 999 { v = 999; }
    if v < -99 { v = -99; }
    let digits = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
    let text = `${v}`;
    let px = x + 2.0 + cw * (3 - text.len()).to_float();
    for c in text.chars() {
        let name = if c == '-' { "gfx/2d/numbers/t_minus" } else { "gfx/2d/numbers/t_" + digits[c.to_int() - 48] };
        let ox = if right { rx(px, s) } else { lx(px, s) };
        hud_image(id, name, ox, ly(y, s), cw * s, ch * s, color);
        px += cw + 1.0;
    }
}

fn on_update() {
    let info = session_info();
    if info == () { return; }
    let h = info.screen_height;
    if h <= 0 { return; }
    let s = h.to_float() / 480.0;
    if h != screen_h {
        screen_h = h;
        hud_register("left", "JKA HUD: health / armor", W * s, (112.0 + PAD_Y) * s, "bottom-left");
        hud_register("right", "JKA HUD: force / ammo", W * s, (112.0 + PAD_Y) * s, "bottom-right");
    }

    let ps = playerstate();
    // pm_type 4 = spectator, 7/8 = intermission: the stock HUD is not drawn.
    if ps == () || ps.pm_type == 4 || ps.pm_type == 7 || ps.pm_type == 8 { return; }
    let tint = tint_for(ps);

    // ---- left cluster: health + armor --------------------------------------
    hud_image("left", "gfx/hud/hudleft", lx(0.0, s), ly(0.0, s), 112.0 * s, 112.0 * s, tint);

    let max = ps.max_health;
    tics("left", HEALTH_TICS, ps.health, max / 4, tint, false, false, s);

    // Armor's last tic blinks (400 ms) while armor is under a quarter of max.
    let quarter = max.to_float() / 4.0;
    let armor_visible = !(ps.armor > 0 && ps.armor.to_float() < quarter)
        || (info.server_time / 400) % 2 == 0;
    tics("left", ARMOR_TICS, ps.armor, max / 4, tint, false, !armor_visible, s);

    number("left", ps.armor, 85.0, 98.0, 6.0, 12.0, ARMOR_COLOR, false, s);
    number("left", ps.health, 59.0, 98.0, 6.0, 12.0, HEALTH_COLOR, false, s);

    // ---- right cluster: force + ammo / saber style --------------------------
    hud_image("right", "gfx/hud/hudleft", rx(0.0, s), ly(0.0, s), -112.0 * s, 112.0 * s, tint);

    tics("right", FORCE_TICS, ps.force_power, 25, [1.0, 1.0, 1.0, 1.0], true, false, s);
    number("right", ps.force_power, -109.0, 98.0, 6.0, 12.0, FORCE_COLOR, true, s);

    if ps.weapon == 0 { return; }
    if ps.ammo_infinite {
        // Saber, melee and stun baton: no ammo; sabers show the current style.
        hud_text("right", "--", rx(-75.0, s), ly(87.0, s), 2.0 * s / 2.25, AMMO_COLOR);
        if ps.weapon == 3 && ps.saber_style >= 1 && ps.saber_style <= 3 {
            let t = SABER_STYLES[ps.saber_style];
            hud_image("right", t[0], rx(t[1], s), ly(t[2], s), t[3] * s, t[4] * s, [1.0, 1.0, 1.0, 1.0]);
        }
    } else if ps.ammo_current != () {
        let ammo = ps.ammo_current;
        let top = AMMO_MAX[ps.ammo_index];
        tics("right", AMMO_TICS, ammo, top / 4, tint, true, false, s);
        number("right", ammo, -83.0, 98.0, 6.0, 12.0, if ammo > 0 { AMMO_COLOR } else { RED }, true, s);
    }
}

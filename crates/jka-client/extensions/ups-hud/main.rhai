// Replacement speedometer: horizontal UPS, a short-window peak and a speed bar.
// Copy this folder to <base>/extensions/ups-hud/ to try it. Position and scale
// are set in HUD Edit (the "EXT: UPS speedometer" box).
const W = 150.0;
const H = 46.0;
const BAR_MAX = 1200.0;   // UPS at which the bar is full
const PEAK_HOLD_MS = 2000;

hud_register("ups", "UPS speedometer", W, H);

let peak = 0.0;
let peak_time = 0;

fn speed_color(ups) {
    if ups < 320.0 { [0.85, 0.90, 1.00, 1.0] }
    else if ups < 700.0 { [0.45, 0.95, 0.60, 1.0] }
    else { [1.00, 0.75, 0.30, 1.0] }
}

fn on_update() {
    let ps = playerstate();
    let info = session_info();
    hud_rect("ups", 0.0, 0.0, W, H, [0.03, 0.04, 0.07, 0.78]);
    if ps == () || info == () {
        hud_text("ups", "--", 8.0, 8.0, 2.5, [0.6, 0.6, 0.6, 1.0]);
        return;
    }
    let v = ps.velocity;
    let ups = (v[0] * v[0] + v[1] * v[1]).sqrt();
    let now = info.server_time;
    // Server time restarts on map change / demo seek.
    if now < peak_time || ups >= peak || now - peak_time > PEAK_HOLD_MS {
        peak = ups;
        peak_time = now;
    }
    let color = speed_color(ups);
    hud_text("ups", `${ups.round().to_int()}`, 8.0, 6.0, 3.0, color);
    hud_text("ups", `peak ${peak.round().to_int()}`, 8.0, 30.0, 1.0, [0.65, 0.70, 0.80, 1.0]);
    let fill = (ups / BAR_MAX).min(1.0) * (W - 16.0);
    hud_rect("ups", 8.0, H - 8.0, W - 16.0, 3.0, [1.0, 1.0, 1.0, 0.12]);
    if fill > 0.0 { hud_rect("ups", 8.0, H - 8.0, fill, 3.0, color); }
}

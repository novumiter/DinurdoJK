// Host Legacy fog for the native surface. Distances use metres, as do the
// camera and Surface.fog_color.w. Mode 1 is authored global EXP2; 2 manual.
// Authored Legacy 1 is composited once in host post using completed scene depth.
fn jka_surface_legacy_fog(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let mode = surface.fog.x;
    if mode < 0.5 { return color; }
    let delta = world - camera.position;
    let depth = max(surface.fog_color.w, 1e-4);
    var amount = 0.0;
    if mode < 1.5 {
        let forward = normalize((camera.inverse_view * vec4<f32>(0.0, 0.0, -1.0, 0.0)).xyz);
        let scaled = max(dot(delta, forward), 0.0) / depth * surface.fog.y;
        amount = 1.0 - exp(-5.5412635 * scaled * scaled);
    } else {
        amount = 1.0 - exp(-length(delta) / depth * surface.fog.y * 0.26);
    }
    return mix(color, surface.fog_color.rgb, clamp(amount, 0.0, 1.0));
}

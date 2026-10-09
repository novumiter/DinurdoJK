// Sun-only HDR incident radiance signal.  This is NOT scene color, local lights,
// the reduced-rate visibility cache, or TAA history.  The source visibility is
// promoted to spectrally weighted lighting before reprojection.
struct RadianceParams {
    inverse_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    previous_eye: vec4<f32>,
    sun_rgb: vec4<f32>, // RGB = linear incident sun radiance; w unused
    dimensions: vec4<u32>, // width, height, mode (1 temporal / 2 SVGF), frame
    flags: vec4<u32>, // history valid, reset, reserved, reserved
};
@group(0) @binding(0) var<uniform> settings: RadianceParams;
@group(0) @binding(1) var shadow_visibility: texture_2d<f32>;
@group(0) @binding(2) var linear_depth: texture_2d<f32>;
@group(0) @binding(3) var motion: texture_2d<f32>;
@group(0) @binding(4) var previous_radiance: texture_2d<f32>;
@group(0) @binding(5) var previous_moments: texture_2d<f32>;
@group(0) @binding(6) var previous_guide: texture_2d<f32>;
@group(0) @binding(7) var next_radiance: texture_storage_2d<rgba16float, write>;
@group(0) @binding(8) var next_moments: texture_storage_2d<rgba16float, write>;
@group(0) @binding(9) var next_guide: texture_storage_2d<rgba32float, write>;
@group(0) @binding(10) var temporal_signal: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var published_signal: texture_storage_2d<rgba16float, write>;
@group(0) @binding(12) var raw_signal: texture_storage_2d<rgba16float, write>;

fn within(p: vec2<i32>) -> bool {
    return all(p >= vec2<i32>(0)) && all(p < vec2<i32>(settings.dimensions.xy));
}
fn depth_at(p: vec2<i32>) -> f32 {
    let hi = vec2<i32>(settings.dimensions.xy) - vec2<i32>(1);
    return textureLoad(linear_depth, clamp(p, vec2<i32>(0), hi), 0).x;
}
fn point_at(p: vec2<i32>) -> vec3<f32> {
    let uv = (vec2<f32>(p) + vec2<f32>(0.5)) / vec2<f32>(settings.dimensions.xy);
    let h = settings.inverse_view_proj * vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.5, 1.0);
    let dir = normalize(h.xyz / h.w - settings.eye.xyz);
    return settings.eye.xyz + dir * depth_at(p);
}
fn normal_at(p: vec2<i32>) -> vec3<f32> {
    let c = point_at(p);
    let lx = point_at(p + vec2<i32>(-1, 0)) - c;
    let rx = point_at(p + vec2<i32>(1, 0)) - c;
    let uy = point_at(p + vec2<i32>(0, -1)) - c;
    let dy = point_at(p + vec2<i32>(0, 1)) - c;
    let dx = select(rx, -lx, dot(lx, lx) < dot(rx, rx));
    let d_y = select(dy, -uy, dot(uy, uy) < dot(dy, dy));
    let n = cross(dx, d_y);
    if (dot(n, n) < 1e-10) { return vec3<f32>(0.0); }
    let outward = normalize(n);
    return outward * select(-1.0, 1.0, dot(outward, settings.eye.xyz - c) >= 0.0);
}
fn luminance(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }

@compute @workgroup_size(8, 8)
fn temporal(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= settings.dimensions.xy)) { return; }
    let p = vec2<i32>(id.xy);
    let depth = depth_at(p);
    let valid_depth = depth > 0.0 && depth < 999999.0;
    let raw_sample = textureLoad(shadow_visibility, p, 0);
    let is_local = settings.flags.y != 0u;
    // When local, this buffer holds an independently RT-traced HDR diffuse
    // irradiance signal; alpha identifies the selected transient light.
    let raw_vis = clamp(raw_sample.x, 0.0, 1.0);
    let signal_id = select(0.0, raw_sample.a, is_local);
    let sun_radiance = settings.sun_rgb.xyz * raw_vis;
    let local_radiance = raw_sample.rgb;
    let current = select(vec3<f32>(0.0), select(sun_radiance, local_radiance, is_local), valid_depth);
    var normal = vec3<f32>(0.0);
    if (valid_depth) { normal = normal_at(p); }
    // The separate source image permits signal-only debugging and reference
    // comparisons; RGB is linear HDR incident radiance, alpha is depth.
    let current_rgba = vec4<f32>(current, select(0.0, min(depth, 65000.0), valid_depth));
    textureStore(raw_signal, p, current_rgba);

    var count = 1.0;
    var filtered = current;
    var first = luminance(current);
    var second = first * first;
    if (settings.flags.x != 0u && valid_depth && dot(normal, normal) > 0.5) {
        let uv = (vec2<f32>(p) + vec2<f32>(0.5)) / vec2<f32>(settings.dimensions.xy);
        let old_uv = uv - textureLoad(motion, p, 0).xy;
        if (all(old_uv >= vec2<f32>(0.0)) && all(old_uv < vec2<f32>(1.0))) {
            let old_p = clamp(vec2<i32>(old_uv * vec2<f32>(settings.dimensions.xy)), vec2<i32>(0), vec2<i32>(settings.dimensions.xy) - vec2<i32>(1));
            let old_g = textureLoad(previous_guide, old_p, 0);
            // Camera/view discontinuity, silhouettes, and moving geometry
            // invalidate independently of RT sample changes.
            let old_depth_expected = distance(settings.previous_eye.xyz, point_at(p));
            let depth_ok = abs(old_g.w - old_depth_expected) < max(0.5, depth * 0.02);
            let normal_ok = dot(normal, old_g.xyz) > 0.85;
            let light_ok = !is_local || (signal_id > 0.0 && abs(textureLoad(previous_moments, old_p, 0).z - signal_id) < 0.25);
            if (depth_ok && normal_ok && light_ok) {
                let old = textureLoad(previous_radiance, old_p, 0);
                let old_m = textureLoad(previous_moments, old_p, 0).xy;
                let temporal_delta = abs(luminance(current) - luminance(old.rgb));
                let signal_scale = select(max(luminance(settings.sun_rgb.xyz), 0.05), max(luminance(current), 0.05), is_local);
                // Quickly shed stale light when a saber or caster moves; do
                // not smear yesterday's lighting onto newly visible surfaces.
                let reactive = temporal_delta > 0.3 * signal_scale;
                count = min(old.a + 1.0, select(32.0, 3.0, reactive));
                let blend = 1.0 / count;
                filtered = mix(old.rgb, current, blend);
                first = mix(old_m.x, luminance(current), blend);
                second = mix(old_m.y, luminance(current) * luminance(current), blend);
            }
        }
    }
    textureStore(next_radiance, p, vec4<f32>(filtered, count));
    textureStore(next_moments, p, vec4<f32>(first, second, signal_id, 0.0));
    textureStore(next_guide, p, vec4<f32>(normal, select(0.0, depth, valid_depth)));
    let output = vec4<f32>(filtered, current_rgba.a);
    textureStore(temporal_signal, p, output);
    // SVGF's next dispatch overwrites this with the final spatial result.
    // Temporal-only needs no extra copy/compute pass.
    if (settings.dimensions.z == 1u) { textureStore(published_signal, p, output); }
}

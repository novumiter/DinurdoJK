struct RtSunShadowFlags {
    flags: vec4<u32>, // enabled, reserved, reserved, reserved
};

@group(3) @binding(15) var<uniform> rt_shadow_flags: RtSunShadowFlags;
@group(3) @binding(16) var rt_sun_shadow: texture_2d<f32>;
// HDR incident direct-sun radiance, reconstructed independently of final scene
// color.  Local RT diffuse, indirect, and specular are not filtered here.
@group(3) @binding(17) var rt_filtered_sun: texture_2d<f32>;
struct RtSunRadianceColor { rgb: vec4<f32>, };
@group(3) @binding(18) var<uniform> rt_sun_radiance_color: RtSunRadianceColor;
@group(3) @binding(19) var rt_sun_raw: texture_2d<f32>;
@group(3) @binding(20) var rt_sun_temporal: texture_2d<f32>;
@group(3) @binding(21) var rt_filtered_local: texture_2d<f32>;
@group(3) @binding(22) var rt_raw_local: texture_2d<f32>;
@group(3) @binding(23) var rt_reference_local: texture_2d<f32>;

// Renderer diagnostic modes 3..7 use the REAL RT signal textures rather than
// post-TAA scene color. This is deliberately in the RT material shader so
// enabling the diagnostic does not introduce an extra fullscreen render pass.
fn rt_denoiser_debug(input: VertexOut) -> vec4<f32> {
    let mode = rt_shadow_flags.flags.y;
    if (mode < 3u || mode > 11u || camera.render_flags.x != 0u) {
        return vec4<f32>(-1.0);
    }
    if (mode >= 8u) {
        let size = vec2<i32>(textureDimensions(rt_filtered_local));
        let px = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), size - vec2<i32>(1));
        let source = textureLoad(rt_raw_local, px, 0);
        let filtered = textureLoad(rt_filtered_local, px, 0);
        let reference = textureLoad(rt_reference_local, px, 0);
        let depth = distance(input.world_position, camera.camera_pos_time.xyz);
        if (source.a <= 0.0 || filtered.a <= 0.0 || abs(filtered.a - depth) > max(0.5, depth * 0.005)) {
            return vec4<f32>(0.1, 0.1, 0.1, 1.0); // no eligible finite source
        }
        let denominator = max(dot(reference.rgb, reference.rgb), 1.0e-6);
        let raw_vis = clamp(dot(source.rgb, reference.rgb) / denominator, 0.0, 1.0);
        let filtered_vis = clamp(dot(filtered.rgb, reference.rgb) / denominator, 0.0, 1.0);
        if (mode == 8u) { return vec4<f32>(vec3<f32>(raw_vis), 1.0); }
        if (mode == 9u) { return vec4<f32>(vec3<f32>(filtered_vis), 1.0); }
        if (mode == 10u) { return vec4<f32>(vec3<f32>(clamp(abs(raw_vis - filtered_vis) * 4.0, 0.0, 1.0)), 1.0); }
        let phase = fract(source.a * 0.61803398875);
        return vec4<f32>(phase, fract(phase + 0.33333), fract(phase + 0.66667), 1.0);
    }
    let dims = vec2<i32>(textureDimensions(rt_filtered_sun));
    let p = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let expected = distance(input.world_position, camera.camera_pos_time.xyz);
    let filtered = textureLoad(rt_filtered_sun, p, 0);
    let raw = textureLoad(rt_sun_raw, p, 0);
    let temporal = textureLoad(rt_sun_temporal, p, 0);
    if (abs(filtered.a - expected) > max(2.0, expected * 0.015) || filtered.a <= 0.0) {
        return vec4<f32>(0.85, 0.05, 0.65, 1.0); // magenta = prepass mismatch/fallback
    }
    let sun = rt_sun_radiance_color.rgb.xyz;
    let scale = 1.0 / max(dot(sun, sun), 1e-6);
    if (mode == 3u) { return vec4<f32>(vec3<f32>(clamp(dot(raw.rgb, sun) * scale, 0.0, 1.0)), 1.0); }
    if (mode == 4u) { return vec4<f32>(vec3<f32>(clamp(dot(temporal.rgb, sun) * scale, 0.0, 1.0)), 1.0); }
    if (mode == 5u) { return vec4<f32>(vec3<f32>(clamp(dot(filtered.rgb, sun) * scale, 0.0, 1.0)), 1.0); }
    if (mode == 6u) { return vec4<f32>(clamp(abs(raw.rgb - temporal.rgb) * 4.0, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0); }
    return vec4<f32>(normalize(input.world_normal) * 0.5 + vec3<f32>(0.5), 1.0);
}

fn rt_filtered_sun_visibility(input: VertexOut) -> f32 {
    if (rt_shadow_flags.flags.y == 0u || camera.render_flags.x != 0u) { return -1.0; }
    let dims = vec2<i32>(textureDimensions(rt_filtered_sun));
    let pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let sample = textureLoad(rt_filtered_sun, pixel, 0);
    let expected = distance(input.world_position, camera.camera_pos_time.xyz);
    // fp16 alpha records the depth; retain the existing direct-ray fallback on
    // mismatched receivers (water, alpha silhouettes, deformed surfaces).
    if (sample.a <= 0.0 || abs(sample.a - expected) > max(2.0, expected * 0.015)) { return -1.0; }
    let sun = rt_sun_radiance_color.rgb.xyz;
    let denominator = max(dot(sun, sun), 1e-8);
    return clamp(dot(sample.rgb, sun) / denominator, 0.0, 1.0);
}


// The temporal trace pass (rt_resolution_trace.wgsl) has already resolved this
// frame's sun visibility for every full-resolution pixel, so the receiver side
// is a same-pixel lookup: no spatial neighbor search, no per-light state.
//
// The cache is keyed by screen pixel and holds the surface the depth prepass
// saw there. A receiver that is not that surface (ocean and other surfaces the
// prepass skips, MSAA silhouette samples, vertex-deformed geometry) would read
// the shadow of whatever is behind it, so each texel also carries the camera
// distance it was traced for; on a mismatch return -1 and let the caller trace
// this fragment directly.
fn rt_cached_sun_visibility(input: VertexOut) -> f32 {
    let filtered = rt_filtered_sun_visibility(input);
    if (filtered >= 0.0) { return filtered; }
    if (rt_shadow_flags.flags.x == 0u || camera.render_flags.x != 0u) { return -1.0; }
    let dims = vec2<i32>(textureDimensions(rt_sun_shadow));
    let pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let cached = textureLoad(rt_sun_shadow, pixel, 0).xy;
    let expected = distance(input.world_position, camera.camera_pos_time.xyz);
    if (abs(cached.y - expected) > max(expected * 0.005, 0.5)) { return -1.0; }
    return cached.x;
}

// Only a single finite runtime emitter is independently traced per depth pixel.
// The existing stochastic BRDF/attenuation/emitter sampling still runs in the
// receiver. Filtered *visibility* is applied only where source identity and
// receiver geometry agree. All other emitters/surfaces retain direct RT rays.

// Classify the original emitter: sampled point/aperture lights deliberately
// have emitter.w/source radius cleared by rt_sample_local_emitter.
fn rt_local_filtered_signal(input: VertexOut, normal: vec3<f32>, light_index: u32) -> vec4<f32> {
    let source = dynamic_lights[light_index];
    if (rt_shadow_flags.flags.z == 0u || camera.render_flags.x != 0u || source.shadow.w < 0.5
        || (source.emitter.w >= -0.5 && source.shadow.z <= 1.0e-4)) {
        return vec4<f32>(-1.0);
    }
    if (dot(normalize(normal), normalize(input.world_normal)) < 0.75) {
        return vec4<f32>(-1.0);
    }
    let dims = vec2<i32>(textureDimensions(rt_filtered_local));
    let pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let selected = textureLoad(rt_raw_local, pixel, 0).a;
    if (abs(selected - f32(light_index + 1u)) > 0.25) { return vec4<f32>(-1.0); }
    let filtered = textureLoad(rt_filtered_local, pixel, 0);
    let reference = textureLoad(rt_reference_local, pixel, 0);
    let expected = distance(input.world_position, camera.camera_pos_time.xyz);
    if (filtered.a <= 0.0 || abs(filtered.a - expected) > max(0.5, expected * 0.005)
        || abs(reference.a - selected) > 0.25 || dot(reference.rgb, reference.rgb) <= 1.0e-6) {
        return vec4<f32>(-1.0);
    }
    return filtered;
}

fn rt_denoised_local_visibility(
    light: PointLight, input: VertexOut, normal: vec3<f32>, light_index: u32
) -> f32 {
    let filtered = rt_local_filtered_signal(input, normal, light_index);
    if (filtered.a < 0.0) {
        return local_light_shadow_visibility(light, input.world_position, normal);
    }
    let dims = vec2<i32>(textureDimensions(rt_reference_local));
    let pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let reference = textureLoad(rt_reference_local, pixel, 0).rgb;
    // Reconstruct filtered direct radiance, not only a [0,1] occlusion term.
    // A dim stochastic sample can need gain >1 to recover the unbiased mean.
    return max(dot(filtered.rgb, reference) / dot(reference, reference), 0.0);
}

fn rt_denoised_local_throughput(
    light: PointLight, input: VertexOut, normal: vec3<f32>, light_index: u32
) -> vec3<f32> {
    let filtered = rt_local_filtered_signal(input, normal, light_index);
    if (filtered.a < 0.0) { return rt_local_shadow_rgb(light, input.world_position, normal); }
    let dims = vec2<i32>(textureDimensions(rt_reference_local));
    let pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let reference = textureLoad(rt_reference_local, pixel, 0).rgb;
    // A zero-energy wavelength contributes no light; use neutral throughput
    // there instead of dividing by a vanishing channel or losing glass hue.
    return select(vec3<f32>(1.0), max(filtered.rgb / max(reference, vec3<f32>(1.0e-8)),
        vec3<f32>(0.0)), reference > vec3<f32>(1.0e-6));
}

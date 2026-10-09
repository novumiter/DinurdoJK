// RBSP world flares, batched per authored shader stage. Visibility is updated
// by the same-frame GPU center depth probe. World materials' stage textures,
// blend modes, stage colors and alpha cutoffs are honored here.
struct FlareCamera {
    view_proj: mat4x4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    eye: vec4<f32>,
    viewport_time: vec4<f32>,  // width, height, seconds, legacy fog flag
    frame: vec4<u32>,
};
struct Flare {
    origin_radius: vec4<f32>,
    color: vec4<f32>,
    normal: vec4<f32>,
    fog: vec4<f32>,
};
struct FlareState {
    visible: u32,
    last_frame: u32,
    fade_start: f32,
    pad: f32,
};
struct FlareStage {
    rgb: vec4<f32>,
    alpha: vec4<f32>,
    tc: array<vec4<f32>, 4>,
    tc_extra: array<vec4<f32>, 4>,
    fade_control: vec4<f32>,
    rgb_wave: vec4<f32>,
    alpha_wave: vec4<f32>,
    wave_funcs: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: FlareCamera;
@group(0) @binding(1) var<storage, read> flares: array<Flare>;
@group(0) @binding(2) var<storage, read> states: array<FlareState>;
@group(1) @binding(0) var flare_texture: texture_2d<f32>;
@group(1) @binding(1) var flare_sampler: sampler;
@group(1) @binding(2) var<uniform> stage: FlareStage;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) facing: f32,
    @location(2) fade: f32,
    @location(3) distance: f32,
    @location(4) fog_depth: f32,
    @location(5) fog_is_global: f32,
    @location(6) forward_distance: f32,
    @location(7) world_pos: vec3<f32>,
    @location(8) flare_color: vec3<f32>,
};
@vertex fn vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    let flare = flares[instance];
    let visibility = states[u32(flare.normal.w)];
    let toward_eye = camera.eye.xyz - flare.origin_radius.xyz;
    let distance = max(length(toward_eye), 0.0001);
    let normal = flare.normal.xyz;
    let safe_normal = select(vec3(0.0, 1.0, 0.0), normal, dot(normal, normal) > 0.000001);
    let facing = select(1.0, abs(dot(toward_eye / distance, normalize(safe_normal))), dot(normal, normal) > 0.000001);
    // JKSunny RB_RenderFlare: authored portalRange (or 30u), scale below 512u; floor 5u.
    let radius = max(5.0, flare.origin_radius.w * min(distance / 512.0, 1.0));
    let origin = flare.origin_radius.xyz + normal * 3.0;
    let corner = corners[vertex];
    let position = origin + radius * (corner.x * camera.right.xyz + corner.y * camera.up.xyz);
    var out: VsOut;
    out.clip = camera.view_proj * vec4(position, 1.0);
    out.uv = corner * 0.5 + vec2(0.5);
    let current = visibility.visible != 0u && visibility.last_frame == camera.frame.x;
    out.fade = select(0.0, clamp((camera.viewport_time.z - visibility.fade_start) / 0.5, 0.0, 1.0), current);
    out.facing = facing;
    out.distance = distance;
    out.fog_depth = flare.fog.w;
    out.fog_is_global = flare.color.w;
    let forward = normalize(cross(camera.up.xyz, camera.right.xyz));
    out.forward_distance = max(dot(origin - camera.eye.xyz, forward), 0.0);
    out.world_pos = position;
    out.flare_color = flare.color.rgb;
    return out;
}
fn texcoords(input: vec2<f32>, world_pos: vec3<f32>) -> vec2<f32> {
    var uv = input;
    let time = camera.viewport_time.z;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let tc = stage.tc[i];
        if (tc.x == 1.0) { uv += tc.yz * time; }
        if (tc.x == 2.0) { uv *= tc.yz; }
        if (tc.x == 3.0) {
            let angle = radians(tc.y * time);
            let cs = cos(angle);
            let sn = sin(angle);
            let centered = uv - vec2(0.5);
            uv = vec2(centered.x * cs - centered.y * sn,
                      centered.x * sn + centered.y * cs) + vec2(0.5);
        }
        if (tc.x == 4.0) {
            let p = stage.tc_extra[i];
            uv = vec2(uv.x * tc.y + uv.y * tc.z + p.z,
                      uv.x * p.x + uv.y * p.y + p.w);
        }
        if (tc.x == 5.0) {
            let p = stage.tc_extra[i];
            // Same world-space sin offsets as the regular BSP shader.
            let now = p.z + time * p.w;
            uv += vec2(
                sin((now + (world_pos.x + world_pos.y) * 0.0009765625) * 6.2831853) * p.y,
                sin((now - world_pos.z * 0.0009765625) * 6.2831853) * p.y
            );
        }
    }
    return uv;
}
// Shared verbatim in behavior with the regular BSP world shader wave evaluator.
fn wave_noise(t: f32) -> f32 {
    let i = floor(t);
    let u = fract(t);
    let sm = u * u * (3.0 - 2.0 * u);
    let a = fract(sin(i * 127.1) * 43758.5453) * 2.0 - 1.0;
    let b = fract(sin((i + 1.0) * 127.1) * 43758.5453) * 2.0 - 1.0;
    return mix(a, b, sm);
}
fn wave_value(func: u32, p: vec4<f32>, time: f32) -> f32 {
    let x = fract(p.z + time * p.w);
    var value = 0.0;
    if (func == 0u) { value = sin(x * 6.28318530718); }
    else if (func == 1u) {
        value = select(select(2.0 - 4.0 * x, 4.0 * x - 4.0, x >= 0.75), 4.0 * x, x < 0.25);
    } else if (func == 2u) { value = select(-1.0, 1.0, x < 0.5); }
    else if (func == 3u) { value = x; }
    else if (func == 4u) { value = 1.0 - x; }
    else { value = wave_noise((time + p.z) * p.w); }
    return p.x + value * p.y;
}
// OpenJK global fog lookup ported from the existing BSP stage implementation.
fn openjk_fog_texel_alpha(texel_x: f32) -> f32 {
    let x = clamp(texel_x, 0.0, 255.0);
    let table_index = floor(clamp(x / 32.0, 0.0, 1.0) * 255.0);
    return floor(sqrt(table_index / 255.0) * 255.0) / 255.0;
}
fn openjk_global_fog_alpha(forward_normalized: f32) -> f32 {
    let texel = max(forward_normalized, 0.0) * 32.0;
    let x0 = floor(texel);
    return mix(openjk_fog_texel_alpha(x0),
               openjk_fog_texel_alpha(x0 + 1.0), fract(texel));
}
@fragment fn fs(v: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(flare_texture, flare_sampler, texcoords(v.uv, v.world_pos));
    // Carry the RBSP-authored flare tint all the way to shading. The normal
    // attenuation is used only for rgbGen vertex, as in RB_AddQuadStamp.
    var rgb = texel.rgb * v.flare_color;
    if (stage.rgb.w == 1.0) { rgb *= v.facing; }
    if (stage.rgb.w == 2.0) { rgb *= 1.0 - v.facing; }
    if (stage.rgb.w == 3.0) { rgb *= stage.rgb.rgb; }
    if (stage.rgb.w == 4.0) { rgb *= clamp(wave_value(u32(stage.wave_funcs.x), stage.rgb_wave, camera.viewport_time.z), 0.0, 1.0); }
    if (stage.rgb.w == 5.0) { rgb = vec3<f32>(0.0); } // worldEntity CGEN_LIGHTING_DIFFUSE
    var alpha = texel.a * stage.alpha.x;
    if (stage.alpha.z == 1.0) { alpha *= v.facing; }
    if (stage.alpha.z == 2.0) { alpha *= 1.0 - v.facing; }
    if (stage.alpha.z == 3.0) { alpha *= clamp(v.distance / max(stage.fade_control.z, 0.001), 0.0, 1.0); }
    if (stage.alpha.z == 4.0) { alpha *= clamp(wave_value(u32(stage.wave_funcs.y), stage.alpha_wave, camera.viewport_time.z), 0.0, 1.0); }
    if (stage.alpha.w > 0.5) {
        let r = length(v.uv * 2.0 - vec2(1.0));
        alpha *= 1.0 - smoothstep(0.12, 1.0, r);
    }
    if (alpha < stage.alpha.y) { discard; }
    // Match the BSP legacy fog implementation rather than inventing an
    // exponential approximation: OpenJK fog texture for mode 1; EXP2 for
    // global mode 2; authored local brush fog uses the established linear ramp.
    if (camera.viewport_time.w > 0.5 && v.fog_depth > 0.0) {
        var transmission = max(1.0 - v.distance / max(v.fog_depth, 0.001), 0.0);
        if (v.fog_is_global > 0.5) {
            let forward = v.forward_distance / max(v.fog_depth, 0.001);
            if (camera.viewport_time.w >= 1.5) {
                transmission = exp(-5.5412635 * forward * forward);
            } else {
                transmission = 1.0 - openjk_global_fog_alpha(forward);
            }
        }
        rgb *= clamp(transmission, 0.0, 1.0);
    }
    rgb *= select(1.0, v.fade, stage.fade_control.x > 0.5);
    alpha *= select(1.0, v.fade, stage.fade_control.y > 0.5);
    return vec4(rgb, alpha);
}

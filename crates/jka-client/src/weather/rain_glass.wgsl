// Native WGPU port of Toadstorm's RainyGlassShader "Rain/RainScroller".
// Upstream: https://github.com/toadstorm/RainyGlassShader
// Source package: toadstorm_RainyGlassShader.unitypackage
// Modified/ported for DinurdoJK on 2026-10-05. Upstream is GNU LGPL 2.1;
// see ../../LICENSE-RainyGlassShader-LGPL-2.1.
//
// This preserves the upstream droplet erosion, dual-phase rivulet flow-map,
// world-up rivulet mask, material constants, tint, and GrabPass refraction.
// The engine integration is a fullscreen scene-only pass: `source_texture` is
// the already post-processed scene that Unity's GrabPass would have captured.
// HUD/UI is deliberately drawn afterward and therefore is not refracted.

struct RainGlassSettings {
    // x: _Time.y, y: rain reaching camera (integration opacity),
    // z: abs(dot(lens normal, world up)), w: reserved.
    time_amount_up: vec4<f32>,
};

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var droplet_mask: texture_2d<f32>;
@group(0) @binding(3) var rivulet_mask: texture_2d<f32>;
@group(0) @binding(4) var mask_sampler: sampler;
@group(0) @binding(5) var<uniform> settings: RainGlassSettings;

// RainOnGlass.mat, not the shader's generic property defaults.
//
// TILING is the one deliberate deviation. Upstream's (8, 6) was authored for a
// quad at whatever size Unity gave it; used directly as fullscreen UV it tiles a
// 1024 mask eight times across the width, so at 2560 px each mask texel covers
// under a third of a pixel. The sampler is clamped to mip 0 (upstream mipBias
// -100), so that minification cannot be filtered away: it both shrinks every
// authored droplet to a third of its drawn size and aliases the mask into
// scattered speckles. Deriving the tiling from the actual viewport and mask size
// instead pins one mask texel to one screen pixel, which is the size the masks
// were painted at, keeps drops round on any aspect ratio, and stays correct at
// any resolution. GLASS_SCALE below 1 magnifies further; above 1 re-introduces
// the aliasing.
const GLASS_SCALE: f32 = 1.0;
const DISTORTION: f32 = 0.05;
const TINT: vec3<f32> = vec3<f32>(0.86240655, 0.8931519, 0.8962264);
const DROPLETS_STRENGTH: f32 = 0.99280274;
const GLOBAL_ROTATION_DEGREES: f32 = 0.0;
const RIVULET_ROTATION_DEGREES: f32 = 0.0;
const RIVULET_SPEED: f32 = 0.0075;
const RIVULETS_STRENGTH: f32 = 0.6;
const DROPLETS_GRAVITY: f32 = 0.0;
const DROPLETS_STRIKE_SPEED: f32 = 0.08362373;
const FLOW_TIME_SCALE: f32 = 0.23;
const RIVULET_VERTICAL_FADE_START: f32 = 0.8500038;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let p = positions[vertex_index];
    var out: VertexOut;
    out.position = vec4<f32>(p, 0.0, 1.0);
    // WGPU scene UV: (0,0) is the top-left of the resolved image.
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

// Amplify emitted `mul(p, float2x2(c,-s,s,c))`. Keep the same row-vector
// multiplication rather than replacing it with a differently-conventioned
// matrix helper.
fn rotate_ase(p: vec2<f32>, degrees: f32) -> vec2<f32> {
    let radians_value = radians(degrees);
    let c = cos(radians_value);
    let s = sin(radians_value);
    return vec2<f32>(p.x * c + p.y * s, -p.x * s + p.y * c);
}

fn rest_ratio(timer: f32) -> f32 {
    // Exact simplification of the Amplify remap from [-1,1] to [0,1].
    let wave = (timer - floor(timer + 0.5)) * 2.0;
    return (wave + 1.0) * 0.5;
}

fn rivulet_bias(timer: f32) -> f32 {
    // Exact shape of node `bias114` in the generated shader.
    let wave = abs((timer - floor(timer + 0.5)) * 2.0) * 2.0 - 1.0;
    let remapped = (wave + 1.0) * 0.5;
    return remapped * remapped;
}

fn rivulet_vertical_mask(up_alignment: f32) -> f32 {
    // SampleGradient(white @ .8500038, black @ 1.0). Endpoints are 0/1 so
    // Unity's linear-color gradient conversion is numerically identical here.
    return 1.0 - clamp(
        (up_alignment - RIVULET_VERTICAL_FADE_START) /
            (1.0 - RIVULET_VERTICAL_FADE_START),
        0.0,
        1.0,
    );
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let source_uv = clamp(input.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let amount = clamp(settings.time_amount_up.y, 0.0, 1.0);
    if (amount <= 0.001) {
        let base = textureSample(source_texture, source_sampler, source_uv).rgb;
        return vec4<f32>(base, 1.0);
    }

    let time = settings.time_amount_up.x;

    // Unity mesh UV has +Y upward; WGPU's fullscreen source UV has +Y down.
    // Flipping only the authored mask domain keeps the original TIFFs upright.
    let glass_uv = vec2<f32>(input.uv.x, 1.0 - input.uv.y);
    let tiling = vec2<f32>(textureDimensions(source_texture))
        / vec2<f32>(textureDimensions(droplet_mask))
        * GLASS_SCALE;
    let droplet_panner = glass_uv * tiling + vec2<f32>(0.0, time * DROPLETS_GRAVITY);
    let global_uv = rotate_ase(droplet_panner, GLOBAL_ROTATION_DEGREES);

    let droplets = textureSample(droplet_mask, mask_sampler, global_uv);
    let droplet_vector = droplets.rg * 2.0 - vec2<f32>(1.0);

    // Upstream alpha erosion/impact timing. B is the per-drop time offset.
    let droplet_phase = fract((-1.0 + droplets.b * 2.0) + time * DROPLETS_STRIKE_SPEED);
    let erosion = clamp(
        ceil(
            (droplets.a - droplet_phase - (1.0 - DROPLETS_STRENGTH)) /
                DROPLETS_STRENGTH,
        ),
        0.0,
        1.0,
    );
    let droplet_distortion = droplet_vector * erosion;

    let rivulet_uv = rotate_ase(global_uv, RIVULET_ROTATION_DEGREES);
    let rivulet_base = textureSample(rivulet_mask, mask_sampler, rivulet_uv);

    // B/A are the upstream flow map: horizontal [-.1,.1], vertical [0,3].
    let flow = vec2<f32>(
        -0.1 + rivulet_base.b * 0.2,
        rivulet_base.a * 3.0,
    );
    let flow_time = time * FLOW_TIME_SCALE;
    let rest1 = rest_ratio(flow_time);
    let rest2 = rest_ratio(flow_time + 0.5);
    let bias = rivulet_bias(flow_time);
    let scroll = vec2<f32>(0.0, time * RIVULET_SPEED);

    let rivulet1 = textureSample(
        rivulet_mask,
        mask_sampler,
        flow * rest1 + rivulet_uv + scroll,
    );
    let rivulet2 = textureSample(
        rivulet_mask,
        mask_sampler,
        flow * rest2 + rivulet_uv + scroll,
    );
    let flowing_rivulet = mix(rivulet1, rivulet2, bias);
    let vertical_mask = rivulet_vertical_mask(abs(settings.time_amount_up.z));
    let rivulet_distortion =
        (flowing_rivulet.rg * 2.0 - vec2<f32>(1.0)) *
        RIVULETS_STRENGTH * vertical_mask;

    let glass_offset = (droplet_distortion + rivulet_distortion) * DISTORTION;
    // Convert +Y-up glass distortion back to the source texture's +Y-down UV.
    let source_offset = vec2<f32>(glass_offset.x, -glass_offset.y);
    let refracted = textureSample(
        source_texture,
        source_sampler,
        clamp(source_uv + source_offset, vec2<f32>(0.0), vec2<f32>(1.0)),
    ).rgb * TINT;

    // `amount` is DinurdoJK's existing camera-rain exposure placeholder. The
    // RainyGlass result itself is untouched at 1.0; this only fades the lens
    // layer as the camera moves under/out from shelter. Avoid the otherwise
    // redundant undistorted scene lookup at full exposure.
    if (amount >= 0.999) {
        return vec4<f32>(refracted, 1.0);
    }
    let base = textureSample(source_texture, source_sampler, source_uv).rgb;
    return vec4<f32>(mix(base, refracted, amount), 1.0);
}

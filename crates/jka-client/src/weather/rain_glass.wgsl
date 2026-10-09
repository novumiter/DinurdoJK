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
    // Instantaneous shared weather wind in lens space, JKA units/sec:
    // x camera-right, y camera-up, z toward the lens, w world-space speed.
    // This is deliberately NOT accumulated into mask UVs. Wind only biases the
    // local water flow/distortion below, so the whole RainyGlass image never pans.
    wind_lens: vec4<f32>,
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

// The weather system uses JKA world units/sec. 600 gives ordinary authored
// winds a visible but subordinate lens response while leaving headroom for
// stronger storm settings. These only alter local water behaviour; no mask UV
// receives a time-integrated wind offset.
const WIND_REFERENCE_SPEED: f32 = 600.0;
// UV travel is expressed in one authored 1024x1024 mask tile. At the reference
// wind this moves a mature droplet by ~60 mask texels: obvious in a stress test
// without turning normal 180 u/s weather into a screen-wide smear.
const DROPLET_WIND_TRAVEL: f32 = 0.060;
const DROPLET_HEADWIND_TRAVEL: f32 = 0.060;
const DROPLET_WIND_STRETCH: f32 = 0.16;
const HEADWIND_CLEAR_RADIUS: f32 = 0.28;
const HEADWIND_CLEAR_SOFTNESS: f32 = 0.18;
const HEADWIND_CLEAR_STRENGTH: f32 = 0.72;
const RIVULET_WIND_FLOW: f32 = 0.85;
const RIVULET_HEADWIND_FLOW: f32 = 0.60;
const RIVULET_STEER_STRENGTH: f32 = 1.35;
const RIVULET_STEER_SCROLL: f32 = 0.85;
const RIVULET_SHEAR: f32 = 0.06;

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

fn fast_normalize_or_zero(v: vec2<f32>) -> vec2<f32> {
    let len_sq = dot(v, v);
    let inv_len = inverseSqrt(max(len_sq, 1.0e-8));
    return select(vec2<f32>(0.0), v * inv_len, len_sq > 1.0e-8);
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

    // Camera/lens-space wind. `plane_wind` is the part that can physically
    // shear water sideways/upward across the glass. Wind travelling directly
    // toward the camera projects to almost zero in the lens plane, so its
    // normal component is kept separately as pressure that pushes water
    // radially away from screen centre.
    // Keep direction and strength separate. A stress-test wind may exceed the
    // reference speed, so permit up to 2x response instead of silently clamping
    // each axis to one. This also makes `r_weatherWind 1200 ...` unmistakable.
    let plane_wind_raw = settings.wind_lens.xy / WIND_REFERENCE_SPEED;
    let plane_len_sq = dot(plane_wind_raw, plane_wind_raw);
    let plane_strength = min(sqrt(plane_len_sq), 2.0);
    let plane_dir = fast_normalize_or_zero(plane_wind_raw);
    let plane_wind = plane_dir * plane_strength;
    let headwind = clamp(settings.wind_lens.z / WIND_REFERENCE_SPEED, 0.0, 2.0);

    // Unity mesh UV has +Y upward; WGPU's fullscreen source UV has +Y down.
    // Flipping only the authored mask domain keeps the original TIFFs upright.
    let glass_uv = vec2<f32>(input.uv.x, 1.0 - input.uv.y);
    let tiling = vec2<f32>(textureDimensions(source_texture))
        / vec2<f32>(textureDimensions(droplet_mask))
        * GLASS_SCALE;
    let droplet_panner = glass_uv * tiling + vec2<f32>(0.0, time * DROPLETS_GRAVITY);
    let global_uv = rotate_ase(droplet_panner, GLOBAL_ROTATION_DEGREES);

    // First sample only the authored per-drop timing field. Unlike the previous
    // attempt, wind now changes the *mask lookup position*, not merely the
    // refraction vector after the drop has already been sampled. B is Toadstorm's
    // per-drop time offset, so neighboring drops naturally receive different
    // travel phases instead of the complete texture sliding as one sheet.
    let droplet_seed = textureSample(droplet_mask, mask_sampler, global_uv);
    let seed_phase = fract((-1.0 + droplet_seed.b * 2.0) + time * DROPLETS_STRIKE_SPEED);

    let lens_from_centre = vec2<f32>(input.uv.x - 0.5, 0.5 - input.uv.y);
    let radial_length = length(lens_from_centre);
    let radial = fast_normalize_or_zero(lens_from_centre);
    let centre_clear =
        (1.0 - smoothstep(
            HEADWIND_CLEAR_RADIUS,
            HEADWIND_CLEAR_RADIUS + HEADWIND_CLEAR_SOFTNESS,
            radial_length,
        )) *
        smoothstep(0.15, 0.85, headwind) *
        HEADWIND_CLEAR_STRENGTH;

    // Ease-in travel over the lifetime. The phase reset coincides with the
    // upstream erosion/respawn cycle, hiding the otherwise discontinuous return
    // to the authored start location. Sampling with the inverse displacement is
    // the normal texture-advection convention: positive lens wind makes the
    // visible water move with that wind.
    let travel_phase = seed_phase * seed_phase;
    let centre_pressure = centre_clear * (0.55 + 0.45 * travel_phase);
    let droplet_motion =
        plane_wind * (DROPLET_WIND_TRAVEL * travel_phase) +
        radial * headwind * (DROPLET_HEADWIND_TRAVEL * (travel_phase + centre_pressure));
    let droplet_uv = global_uv - droplet_motion;
    let droplets = textureSample(droplet_mask, mask_sampler, droplet_uv);
    let droplet_vector = droplets.rg * 2.0 - vec2<f32>(1.0);

    // Keep Toadstorm's erosion timing tied to the drop that was actually sampled
    // at the advected coordinate. This preserves the original appear/rest cycle.
    let droplet_phase = fract((-1.0 + droplets.b * 2.0) + time * DROPLETS_STRIKE_SPEED);
    let erosion = clamp(
        ceil(
            (droplets.a - droplet_phase - (1.0 - DROPLETS_STRENGTH)) /
                DROPLETS_STRENGTH,
        ),
        0.0,
        1.0,
    );

    // A small local stretch makes crosswind readable on round drops, but the
    // important change above is spatial: the authored drop itself now moves.
    let droplet_surface = smoothstep(0.06, 0.45, length(droplet_vector));
    let droplet_stretch =
        (plane_wind + radial * headwind) * DROPLET_WIND_STRETCH *
        droplet_surface * erosion;
    let droplet_visibility = erosion * (1.0 - centre_clear * 0.78);
    let droplet_distortion = droplet_vector * droplet_visibility + droplet_stretch;

    let rivulet_uv = rotate_ase(global_uv, RIVULET_ROTATION_DEGREES);
    let rivulet_base = textureSample(rivulet_mask, mask_sampler, rivulet_uv);

    // B/A are the upstream flow map: horizontal [-.1,.1], vertical [0,3].
    let authored_flow = vec2<f32>(
        -0.1 + rivulet_base.b * 0.2,
        rivulet_base.a * 3.0,
    );

    // Use the authored A flow channel as the local streak-presence signal. The
    // previous patch gated on RG normal magnitude, which can be near-flat along
    // the body of a rivulet and therefore suppress almost all wind response. A
    // retains the upstream vertical-flow information and is a much better gate.
    let rivulet_water = smoothstep(0.01, 0.20, rivulet_base.a);
    // Do not let perfectly flat/low-A portions become completely wind-immune; a
    // small floor keeps the two-phase flow lookup moving coherently through the
    // complete authored streak while the strong response stays on wet pixels.
    let rivulet_wind_response = 0.20 + 0.80 * rivulet_water;

    // Instead of merely shoving the streaks sideways, steer their motion vector.
    // `authored_flow` already encodes each streak's default gravity-driven path.
    // Blend that local direction with wind/shear so the streak angle itself can
    // lean into crosswind while headwind opens the centre outward.
    let authored_dir = fast_normalize_or_zero(vec2<f32>(authored_flow.x * 0.30, max(authored_flow.y, 0.08)));
    let steer_wind =
        plane_wind * RIVULET_WIND_FLOW +
        radial * headwind * RIVULET_HEADWIND_FLOW;
    let streak_dir = fast_normalize_or_zero(
        authored_dir + steer_wind * (RIVULET_STEER_STRENGTH * rivulet_wind_response),
    );
    let flow_speed = max(length(authored_flow), 0.08);

    // Keep the upstream Toadstorm gravity flow as the exact zero-wind baseline.
    // The previous steering path negated `streak_dir` and then only added part
    // of `authored_flow` back in. With no wind that left a negative Y flow, so
    // the visible rivulets climbed up the lens instead of draining downward.
    // Wind now rotates the authored flow direction without reversing gravity.
    let wind_activity = smoothstep(0.0, 0.03, max(plane_strength, headwind));
    let steered_flow = streak_dir * flow_speed;
    let flow = mix(authored_flow, steered_flow, wind_activity);
    let flow_time = time * FLOW_TIME_SCALE;
    let rest1 = rest_ratio(flow_time);
    let rest2 = rest_ratio(flow_time + 0.5);
    let bias = rivulet_bias(flow_time);
    // This extra scroll exists only to make wind motion readable. It must vanish
    // at zero wind or it changes the authored gravity-only animation.
    let streak_scroll = streak_dir * (
        time * RIVULET_SPEED * wind_activity *
        (1.0 + plane_strength * RIVULET_STEER_SCROLL)
    );
    let streak_shear = plane_wind * (RIVULET_SHEAR * rivulet_wind_response);

    let rivulet1 = textureSample(
        rivulet_mask,
        mask_sampler,
        flow * rest1 + rivulet_uv + streak_scroll + streak_shear,
    );
    let rivulet2 = textureSample(
        rivulet_mask,
        mask_sampler,
        flow * rest2 + rivulet_uv + streak_scroll - streak_shear,
    );
    let flowing_rivulet = mix(rivulet1, rivulet2, bias);
    let vertical_mask = rivulet_vertical_mask(abs(settings.time_amount_up.z));
    let rivulet_visibility = vertical_mask * (1.0 - centre_clear * 0.45);
    let rivulet_distortion =
        (flowing_rivulet.rg * 2.0 - vec2<f32>(1.0)) *
        RIVULETS_STRENGTH * rivulet_visibility;

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

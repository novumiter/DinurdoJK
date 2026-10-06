// Adapted for JKA's existing scene-color + linear-depth renderer from:
// - Bevy SSR hybrid ray marcher: crates/bevy_pbr/src/ssr/raymarch.wesl
//   https://github.com/bevyengine/bevy
// - AMD FidelityFX SSSR/DNSR temporal reflection history:
//   https://github.com/GPUOpen-Effects/FidelityFX-SSSR
// This intentionally omits FidelityFX's general-purpose normal/roughness/variance
// G-buffer to keep the lightweight forward renderer intact. Rain wetness follows
// the useful Screen Space Wetness idea instead: one shared weather-surface field
// modifies the effective normal/smoothness response before SSR traces reflections.

struct SsrTemporalSettings {
    viewport_history: vec4<f32>, // full width, full height, history valid, frame index
    camera_pos_time: vec4<f32>,
    previous_camera_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var scene_texture: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var linear_depth_texture: texture_2d<f32>;
@group(0) @binding(3) var history_radiance_texture: texture_2d<f32>;
@group(0) @binding(4) var history_depth_texture: texture_2d<f32>;
@group(0) @binding(5) var<uniform> settings: SsrTemporalSettings;
@group(0) @binding(6) var weather_occlusion_height: texture_2d<f32>;
@group(0) @binding(7) var weather_occlusion_sampler: sampler;
@group(0) @binding(8) var<uniform> weather_surface: WeatherSurfaceSettings;
@group(0) @binding(9) var reflection_policy_texture: texture_2d<f32>;
@group(0) @binding(10) var ssr_visibility_texture: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct SsrSample {
    radiance: vec3<f32>,
    weight: f32,
};

struct SsrFragmentOut {
    @location(0) radiance: vec4<f32>,
    @location(1) depth: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0)
    );
    let p = positions[vertex_index];
    var out: VertexOut;
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

fn valid_depth(depth: f32) -> bool {
    return depth > 0.0 && depth < 999999.0;
}

fn scene(uv: vec2<f32>) -> vec3<f32> {
    return textureSample(
        scene_texture,
        scene_sampler,
        clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0))
    ).rgb;
}

fn depth_at_pixel(pixel: vec2<i32>) -> f32 {
    let dims = textureDimensions(linear_depth_texture);
    let max_pixel = vec2<i32>(i32(dims.x) - 1, i32(dims.y) - 1);
    return textureLoad(linear_depth_texture, clamp(pixel, vec2<i32>(0), max_pixel), 0).x;
}

fn depth_at_uv(uv: vec2<f32>) -> f32 {
    let dims = textureDimensions(linear_depth_texture);
    let dims_f = vec2<f32>(f32(dims.x), f32(dims.y));
    let pixel = vec2<i32>(
        i32(clamp(uv.x, 0.0, 0.999999) * dims_f.x),
        i32(clamp(uv.y, 0.0, 0.999999) * dims_f.y)
    );
    return depth_at_pixel(pixel);
}

fn reflection_policy_at_uv(uv: vec2<f32>) -> vec4<f32> {
    let dims = textureDimensions(reflection_policy_texture);
    let dims_f = vec2<f32>(f32(dims.x), f32(dims.y));
    let max_pixel = vec2<i32>(i32(dims.x) - 1, i32(dims.y) - 1);
    let pixel = vec2<i32>(
        i32(clamp(uv.x, 0.0, 0.999999) * dims_f.x),
        i32(clamp(uv.y, 0.0, 0.999999) * dims_f.y)
    );
    return textureLoad(reflection_policy_texture, clamp(pixel, vec2<i32>(0), max_pixel), 0);
}

fn ssr_frontmost_at_uv(uv: vec2<f32>) -> bool {
    let bsp_depth = depth_at_uv(uv);
    if (!valid_depth(bsp_depth)) {
        return false;
    }

    let dims = textureDimensions(ssr_visibility_texture);
    let dims_f = vec2<f32>(f32(dims.x), f32(dims.y));
    let max_pixel = vec2<i32>(i32(dims.x) - 1, i32(dims.y) - 1);
    let pixel = clamp(
        vec2<i32>(
            i32(clamp(uv.x, 0.0, 0.999999) * dims_f.x),
            i32(clamp(uv.y, 0.0, 0.999999) * dims_f.y)
        ),
        vec2<i32>(0),
        max_pixel
    );
    let device_depth = textureLoad(ssr_visibility_texture, pixel, 0).x;
    if (device_depth <= 0.0) {
        return false;
    }

    let pixel_uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / dims_f;
    let ndc = vec4<f32>(
        pixel_uv.x * 2.0 - 1.0,
        1.0 - pixel_uv.y * 2.0,
        device_depth,
        1.0
    );
    var world = settings.inv_view_proj * ndc;
    if (abs(world.w) <= 1.0e-6) {
        return false;
    }
    world = world / world.w;
    let scene_depth = distance(world.xyz, settings.camera_pos_time.xyz);
    let tolerance = max(2.0, bsp_depth * 0.002);
    return scene_depth + tolerance >= bsp_depth;
}

fn history_depth_at_uv(uv: vec2<f32>) -> f32 {
    let dims = textureDimensions(history_depth_texture);
    let dims_f = vec2<f32>(f32(dims.x), f32(dims.y));
    let max_pixel = vec2<i32>(i32(dims.x) - 1, i32(dims.y) - 1);
    let pixel = vec2<i32>(
        i32(clamp(uv.x, 0.0, 0.999999) * dims_f.x),
        i32(clamp(uv.y, 0.0, 0.999999) * dims_f.y)
    );
    return textureLoad(history_depth_texture, clamp(pixel, vec2<i32>(0), max_pixel), 0).x;
}

fn world_ray(uv: vec2<f32>) -> vec3<f32> {
    // Reversed-Z: unproject on the far plane (see post.wgsl world_ray).
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    var world_far = settings.inv_view_proj * ndc;
    world_far = world_far / max(abs(world_far.w), 1e-6);
    return normalize(world_far.xyz - settings.camera_pos_time.xyz);
}

fn world_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    return settings.camera_pos_time.xyz + world_ray(uv) * depth;
}

fn project_uv(world: vec3<f32>) -> vec3<f32> {
    let clip = settings.view_proj * vec4<f32>(world, 1.0);
    if (clip.w <= 1e-5) {
        return vec3<f32>(-10.0, -10.0, -1.0);
    }
    let ndc = clip.xyz / clip.w;
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}

fn previous_uv(world: vec3<f32>) -> vec3<f32> {
    let clip = settings.prev_view_proj * vec4<f32>(world, 1.0);
    if (clip.w <= 1e-5) {
        return vec3<f32>(-1.0, -1.0, -1.0);
    }
    let ndc = clip.xyz / clip.w;
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}

fn surface_normal(uv: vec2<f32>, centre_depth: f32) -> vec3<f32> {
    let dims = textureDimensions(linear_depth_texture);
    let dims_f = vec2<f32>(f32(dims.x), f32(dims.y));
    let centre_pixel = vec2<i32>(
        i32(clamp(uv.x, 0.0, 0.999999) * dims_f.x),
        i32(clamp(uv.y, 0.0, 0.999999) * dims_f.y)
    );
    let px = centre_pixel + vec2<i32>(1, 0);
    let py = centre_pixel + vec2<i32>(0, 1);
    let dx_depth = depth_at_pixel(px);
    let dy_depth = depth_at_pixel(py);
    if (!valid_depth(dx_depth) || !valid_depth(dy_depth)) {
        return normalize(-world_ray(uv));
    }
    let uvx = (vec2<f32>(px) + vec2<f32>(0.5)) / dims_f;
    let uvy = (vec2<f32>(py) + vec2<f32>(0.5)) / dims_f;
    // All three taps must share one convention. The neighbours are reconstructed
    // at full-res texel centres, so the centre has to be as well: `uv` is the
    // half-resolution fragment's own coordinate, and a half-res fragment centre
    // lands exactly on a full-res texel *boundary*. Reconstructing p0 there put
    // it half a texel away from the depth sample it was paired with, so each
    // tangent spanned 1.5 texels of screen against a depth delta measured over
    // 1.0 - a skew with screen-grid periodicity baked into it.
    let uv0 = (vec2<f32>(centre_pixel) + vec2<f32>(0.5)) / dims_f;
    let p0 = world_position(uv0, centre_depth);
    let p1 = world_position(uvx, dx_depth);
    let p2 = world_position(uvy, dy_depth);
    var n = normalize(cross(p1 - p0, p2 - p0));
    let toward_camera = normalize(settings.camera_pos_time.xyz - p0);
    if (dot(n, toward_camera) < 0.0) {
        n = -n;
    }
    return n;
}

// What the screen-space trace needs to know about a wet surface. Shares the
// puddle shape, ripples and water normal with the material pass (weather_surface.wgsl),
// so the reflection and the surface it lies on always agree.
struct SsrWeather {
    wetness: f32,
    puddle: f32,
    water_normal: vec3<f32>,
};

fn ssr_weather(world: vec3<f32>, normal: vec3<f32>) -> SsrWeather {
    var result: SsrWeather;
    result.wetness = 0.0;
    result.puddle = 0.0;
    result.water_normal = vec3<f32>(0.0, 1.0, 0.0);
    if ((weather_surface.amount_distance.x <= 0.001 && weather_surface.puddle.x <= 0.001)
        || weather_surface.occlusion_size.z == 0u
        || weather_surface.occlusion_size.x == 0u
        || weather_surface.occlusion_size.y == 0u) {
        return result;
    }

    let to_surface = world - settings.camera_pos_time.xyz;
    let fade_start = weather_surface.film_fade.x;
    let distance_weight = 1.0 - smoothstep(
        fade_start,
        max(weather_surface.film_fade.y, fade_start + 1.0),
        length(to_surface)
    );
    let field = weather_sample_field(
        weather_occlusion_height,
        world,
        weather_surface.occlusion_uv.xy,
        weather_surface.occlusion_uv.zw,
        normalize(normal).y
    );

    let normal_y = normalize(normal).y;
    let orientation = smoothstep(-0.45, 0.12, normal_y);
    var wetness = clamp(weather_surface.amount_distance.x * weather_surface.amount_distance.w
        * distance_weight * field.exposure * orientation, 0.0, 1.0);

    let accumulation = clamp(weather_surface.puddle.x, 0.0, 1.0);
    if (accumulation > 0.001 && field.exposure > 0.001) {
        let shape = weather_puddle_shape(field, world.xz, accumulation, weather_surface.look.x);
        let upright = smoothstep(0.75, 0.95, normal_y);
        let coverage = shape.coverage * field.exposure * upright;
        result.puddle = coverage;
        wetness = max(wetness, max(coverage * 0.96, shape.damp * field.exposure * upright * 0.85));
        if (coverage > 0.001) {
            // Must match weather_surface_world.wgsl exactly: the reflection and the
            // surface it lies on have to be shaped by the same water, fine layers
            // and distance fade included.
            let detail = weather_surface.look.z
                * (1.0 - smoothstep(1200.0, 2800.0, length(to_surface)));
            let gradient = (weather_water_gradient(
                world.xz,
                weather_surface.puddle.y,
                clamp(weather_surface.puddle.z, 0.0, 1.0),
                weather_surface.wind.xy,
                detail
            ) + weather_wake_gradient(world)) * smoothstep(0.0, 0.5, shape.depth);
            result.water_normal = weather_water_normal(gradient, mix(0.07, 0.11, detail));
        }
    }
    result.wetness = wetness;
    return result;
}

fn scene_level(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(
        scene_texture,
        scene_sampler,
        clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)),
        0.0
    ).rgb;
}

// A reflection in wet ground smears along the screen-space vertical: ripples and
// the roughness of the wet film tilt the reflected ray mostly in the view plane, so
// a neon tube or a lit window becomes a long soft streak rather than a mirror image.
//
// Five taps only read as a blur while they overlap. Spaced a quarter of the span
// apart they are a comb: at 1440p a 43 pixel streak put them ~11 pixels apart, so
// instead of smearing the scene it printed five ghost copies of it, and reflected
// rain - dense, bright, vertical - came back as a regular lattice of dashes. One
// jitter shared by all five taps could not break that up either, because it slid
// the whole comb as a unit. So: cap the spacing at the width where taps still
// overlap, and give each tap its own stratified offset inside its own slot, which
// turns what is left into per-pixel noise for the temporal pass below to resolve.
//
// The cap is what the scene texture can actually support: it has no mip chain
// (targets.rs builds it with mip_level_count 1), so five point samples cannot
// cover a long span without gapping. A genuinely long streak needs either a mip
// chain or a separate blur pass; until then the streak is a short honest blur
// rather than a long dishonest comb.
const SSR_STREAK_MAX_SPACING_PX: f32 = 2.0;
const SSR_STREAK_GOLDEN: f32 = 0.6180339887;

fn ssr_streak_offset(index: f32, jitter: f32) -> f32 {
    return index + fract(jitter + index * SSR_STREAK_GOLDEN) - 0.5;
}

fn ssr_streaked_scene(hit_uv: vec2<f32>, hit_world: vec3<f32>, length_px: f32, jitter: f32) -> vec3<f32> {
    let viewport = max(settings.viewport_history.xy, vec2<f32>(1.0));
    var axis_px = (project_uv(hit_world + vec3<f32>(0.0, 96.0, 0.0)).xy - hit_uv) * viewport;
    let axis_length = length(axis_px);
    if (length_px < 1.0 || axis_length < 0.5) {
        return scene_level(hit_uv);
    }
    axis_px = axis_px / axis_length;
    let spacing_px = min(length_px * 0.25, SSR_STREAK_MAX_SPACING_PX);
    let spacing_uv = axis_px * spacing_px / viewport;
    return scene_level(hit_uv + spacing_uv * ssr_streak_offset(-2.0, jitter)) * 0.06
        + scene_level(hit_uv + spacing_uv * ssr_streak_offset(-1.0, jitter)) * 0.24
        + scene_level(hit_uv + spacing_uv * ssr_streak_offset(0.0, jitter)) * 0.40
        + scene_level(hit_uv + spacing_uv * ssr_streak_offset(1.0, jitter)) * 0.24
        + scene_level(hit_uv + spacing_uv * ssr_streak_offset(2.0, jitter)) * 0.06;
}

// r_ssrDebug 7 paints a flat colour per outcome instead of radiance, at full
// weight so the view is not modulated by how strong the reflection would be:
//   dark grey  surface is not an SSR receiver at all (map-load policy)
//   white      surface won a planar reflection slot this frame, so SSR stood down
//   dark red   material not glossy enough to admit a dry ray
//   dark blue  admitted, but the Fresnel/strength term came out negligible
//   red        marched the full reach and never crossed a depth surface
//   brown      the ray walked off the side of the screen mid-march (kept well
//              away from magenta: both mean "left the screen" and tonemapping
//              was making the two shades hard to tell apart)
//   orange     no crossing, resolved to the sky fallback
//   magenta    crossing refined to a point off screen
//   cyan       crossing refined onto a pixel with no valid depth
//   yellow     crossing found, then rejected as too deep (thickness)
//   green      accepted
// r_ssrDebug 9 reuses the same channel for the reconstructed normal instead.
fn ssr_label(sample: SsrSample, use_label: bool, colour: vec3<f32>) -> SsrSample {
    if (!use_label) {
        return sample;
    }
    var labelled: SsrSample;
    labelled.radiance = colour;
    labelled.weight = 1.0;
    return labelled;
}

fn current_ssr(uv: vec2<f32>, centre_depth: f32, frag_coord: vec2<f32>) -> SsrSample {
    var result: SsrSample;
    result.radiance = vec3<f32>(0.0);
    result.weight = 0.0;
    if (!valid_depth(centre_depth)) {
        return result;
    }

    let packed_quality = u32(settings.previous_camera_pos.w + 0.5);
    let quality = packed_quality & 7u;
    // r_ssrDebug 7 replaces radiance with a flat label saying how this pixel's
    // ray ended, so a structured artifact can be read off the *reason* rather
    // than guessed at from the result. Post displays it unfiltered, like mode 3.
    let label_march = packed_quality >= 56u && packed_quality < 64u;
    // r_ssrDebug 8 pins the per-pixel step jitter to its midpoint. If a lattice
    // survives that, the stochastic start is not what builds it.
    let fixed_jitter = packed_quality >= 64u && packed_quality < 72u;
    // r_ssrDebug 9 shows the reconstructed geometric normal itself. Everything
    // the trace does is downstream of it, so if the lattice is already here it is
    // not a ray marching problem at all.
    let label_normal = packed_quality >= 72u && packed_quality < 80u;
    if (quality < 2u) {
        return result;
    }

    // Static material/geometry eligibility was cached at map load and rasterized
    // into the depth prepass. Reject before reconstructing normals or ray marching.
    let reflection_policy = reflection_policy_at_uv(uv);
    if (reflection_policy.x < 0.5) {
        return ssr_label(result, label_march, vec3<f32>(0.15, 0.15, 0.15));
    }
    // A surface that actually won a planar slot this frame already has the
    // highest-quality source. Candidates that lost the finite planar budget keep
    // y=0 in the prepass policy mask and are therefore free to fall back to SSR.
    if (quality >= 3u && reflection_policy.y >= 0.5) {
        // The only gate here that can change from frame to frame: the planar
        // budget is finite and re-allocated per frame, so a surface can win a
        // slot, lose SSR, then lose the slot again. Worth knowing on sight.
        return ssr_label(result, label_march, vec3<f32>(1.0, 1.0, 1.0));
    }

    let world = world_position(uv, centre_depth);
    let geometric_normal = surface_normal(uv, centre_depth);
    if (label_normal) {
        return ssr_label(result, true, geometric_normal * 0.5 + vec3<f32>(0.5));
    }
    let weather = ssr_weather(world, geometric_normal);
    let wetness = weather.wetness;
    let puddle = weather.puddle;

    // A dry surface reflects only when the material gives a reason to: it is a
    // mirror, water, an authored tcGen environment stage, or it carries metallic,
    // specular or roughness data. `reflection_policy.z` is the map-load hint that
    // records which (scene/reflections.rs), and `dry_gloss` turns it into one 0..1
    // answer that drives both the admission below and the dry strength further
    // down, so the two can never drift apart.
    //
    // The hint's last bucket, 0.85, is the default for a plain JKA surface with no
    // reflection data of any kind. Promoting that to a reflector is never logical
    // while it is dry, so no tier admits it - the old threshold ladder topped out
    // at 1.0 for Ultra, which let the whole map in and, because the hint was then
    // thrown away rather than used as a weight, let it all in as a full mirror.
    // Quality now buys reach and accuracy (ray length, march steps, refinement),
    // not a wider set of materials, and Ultra reaches a little further down the
    // hint only for materials that authored a roughness of their own.
    //
    // Rain still overrides all of it: wetness is view- and time-dependent and
    // cannot be answered by a map-load cache, and water standing on rough ground
    // really is a mirror.
    let dry_gloss = 1.0 - smoothstep(0.18, 0.95, reflection_policy.z);
    var min_dry_gloss = 0.70;
    if (quality >= 3u) {
        min_dry_gloss = 0.55;
    }
    if (quality >= 4u) {
        min_dry_gloss = 0.30;
    }
    let weather_reflective = wetness > 0.01 || puddle > 0.01;
    if (!weather_reflective && dry_gloss < min_dry_gloss) {
        return ssr_label(result, label_march, vec3<f32>(0.25, 0.0, 0.0));
    }

    // Screen Space Wetness modifies the normal/smoothness buffers before later
    // screen-space lighting consumes them. This forward-renderer adaptation does
    // the same thing locally for SSR: standing rain film slightly flattens an
    // upward-facing response normal and adds a low-roughness dielectric Fresnel
    // term. Dry SSR remains byte-for-byte equivalent in the wetness == 0 case.
    let high_quality = weather_surface.look.z > 0.5;
    let water = smoothstep(0.05, 0.85, puddle);
    let upward = smoothstep(0.25, 0.90, geometric_normal.y);
    // Thin wet film respects the reconstructed surface slope. Puddle placement
    // has already been validated against the cached physical BSP plane, so its
    // water surface replaces a noisy/interpolated screen normal outright, exactly
    // as it does in the material pass.
    var normal = normalize(mix(
        geometric_normal,
        vec3<f32>(0.0, 1.0, 0.0),
        clamp(wetness * upward * 0.14, 0.0, 0.92)
    ));
    normal = normalize(mix(normal, weather.water_normal, water * 0.985));
    if (high_quality && puddle < 0.5 && geometric_normal.y > 0.97) {
        // Level wet asphalt is never optically flat: static micro-relief breaks the
        // reflection into the fine grain seen on wet streets. Slopes are left
        // alone: world-space grain stretches along them into visible bands.
        let grain = vec2<f32>(
            weather_value_noise(world.xz / 9.0),
            weather_value_noise(world.xz / 9.0 + vec2<f32>(53.1, 17.7))
        ) - vec2<f32>(0.5);
        normal = normalize(normal + vec3<f32>(grain.x, 0.0, grain.y) * (0.12 * wetness * upward));
    }
    let view_dir = normalize(settings.camera_pos_time.xyz - world);
    let facing = clamp(dot(normal, view_dir), 0.0, 1.0);
    let grazing = pow(1.0 - facing, 5.0);
    let horizontal = clamp(normal.y * 1.4, 0.0, 1.0);
    // The same hint that admitted this surface also says how hard it reflects, so
    // a rough-but-admitted material gets a faint grazing sheen instead of the full
    // mirror every admitted surface used to get. Wet and puddle strength below
    // deliberately ignore it, for the reason given at the admission test.
    // Schlick's approximation is F0 + (1 - F0) * (1 - cos)^5. Only the second
    // term was here, which sets the head-on reflectance of every dry material to
    // zero: a mirror disappeared when looked at straight on and returned as you
    // turned away, so neighbouring facets of one object reflected or did not
    // purely by their angle to the camera. Everything that reaches this line was
    // admitted *because* it is a mirror, env-mapped or metallic, so it has a
    // substantial F0, and dry_gloss is already this renderer's one 0..1 answer
    // for how much. The grazing ceiling is unchanged at 0.40, so angles that
    // already reflected are untouched; this only lifts the head-on and vertical
    // cases that were being zeroed.
    let dry_f0 = 0.04 + 0.56 * dry_gloss;
    let dry_fresnel = dry_f0 + (1.0 - dry_f0) * grazing;
    let dry_strength = dry_fresnel * (0.22 + horizontal * 0.18) * dry_gloss;

    // Water-like F0 (~2%) keeps head-on reflections restrained while making
    // wet grazing angles much more obvious. Wetness is already intensity-,
    // shelter-, and distance-weighted by the shared weather surface system.
    let wet_fresnel = 0.02 + 0.98 * grazing;
    let wet_smoothness = mix(0.72, 0.96, pow(wetness, 1.20));
    let puddle_smoothness = mix(wet_smoothness, 0.999, water);
    let wet_strength = wetness * wet_fresnel * puddle_smoothness
        * (0.10 + horizontal * 0.56) * weather_surface.look.w;
    let puddle_strength = smoothstep(0.03, 0.75, puddle) * wet_fresnel * (0.44 + horizontal * 0.78);
    let strength = dry_strength + wet_strength + puddle_strength;
    if (strength < 0.01) {
        return ssr_label(result, label_march, vec3<f32>(0.0, 0.0, 0.35));
    }

    // Bevy's SSR ray marcher uses a hybrid root finder: a cheap linear search
    // first, then bisection (and optionally secant) once the depth crossing is
    // bracketed. At half resolution, 10 stochastic linear steps + 3 bisections
    // give us a more accurate hit than the old 18 fixed world-space taps while
    // doing far less total work over the frame.
    var ray_dir = normalize(reflect(-view_dir, normal));
    // A ray that dips into the surface it leaves (a perturbed normal at a grazing
    // angle, a sloped floor) would hit that surface again at once and reflect
    // itself in stripes: keep every ray above the geometry it starts on.
    let lift = dot(ray_dir, geometric_normal);
    if (lift < 0.05) {
        ray_dir = normalize(ray_dir + geometric_normal * (0.05 - lift));
    }
    let ray_origin = world + geometric_normal * max(4.0, centre_depth * 0.004);
    var linear_steps = 6u;
    var refinement_steps = 2u;
    var distance_scale = 0.75;
    if (quality >= 3u) {
        linear_steps = 10u;
        refinement_steps = 3u;
        distance_scale = 1.0;
    }
    if (quality >= 4u) {
        linear_steps = 16u;
        refinement_steps = 4u;
        distance_scale = 1.25;
    }
    var max_ray_distance = max(216.0, centre_depth * mix(0.22, mix(0.28, 0.34, puddle), wetness)) * distance_scale;
    var step_curve = 1.15;
    if (high_quality && weather_reflective) {
        // A wet street mirrors things a block away: facades, signs, the far end
        // of the road. Reach that far, and bunch the steps toward the surface so
        // nearby objects still resolve.
        max_ray_distance = max(max_ray_distance, (600.0 + centre_depth * 0.9) * distance_scale);
        step_curve = 1.6;
    }
    let frame = settings.viewport_history.w;
    var jitter = weather_hash12(frag_coord + vec2<f32>(frame * 17.0, frame * 11.0));
    if (fixed_jitter) {
        jitter = 0.5;
    }

    var miss_t = 0.0;
    var hit_t = 0.0;
    var found_hit = false;
    var left_screen = false;
    for (var i = 0u; i < linear_steps; i = i + 1u) {
        let linear_t = (f32(i) + jitter) / f32(linear_steps);
        let candidate_t = pow(clamp(linear_t, 0.0, 1.0), step_curve);
        let sample_world = ray_origin + ray_dir * max_ray_distance * candidate_t;
        let projected = project_uv(sample_world);
        if (projected.x <= 0.0 || projected.x >= 1.0 ||
            projected.y <= 0.0 || projected.y >= 1.0) {
            left_screen = true;
            break;
        }
        let sampled_depth = depth_at_uv(projected.xy);
        if (!valid_depth(sampled_depth)) {
            miss_t = candidate_t;
            continue;
        }
        let expected_depth = distance(settings.camera_pos_time.xyz, sample_world);
        let penetration = expected_depth - sampled_depth;
        // Bracket the *first* depth crossing, then validate thickness after
        // refinement. This mirrors Bevy's root finder more closely than only
        // accepting a coarse step that already happens to fall inside the
        // thickness window.
        if (penetration > 0.0) {
            hit_t = candidate_t;
            found_hit = true;
            break;
        }
        miss_t = candidate_t;
    }

    if (!found_hit) {
        if (high_quality && weather_reflective && ray_dir.y > 0.03) {
            // The ray left the screen or the world without touching anything
            // visible: what the water mirrors is the sky. Look for the point on
            // screen where that sky is.
            let sky_uv = project_uv(ray_origin + ray_dir * 60000.0);
            if (sky_uv.x > 0.0 && sky_uv.x < 1.0 && sky_uv.y > 0.0 && sky_uv.y < 1.0
                && !valid_depth(depth_at_uv(sky_uv.xy))) {
                let edge = min(min(sky_uv.x, 1.0 - sky_uv.x), min(sky_uv.y, 1.0 - sky_uv.y));
                result.radiance = scene_level(sky_uv.xy);
                let max_sky_weight = mix(mix(0.42, 0.60, wetness), 0.92, smoothstep(0.05, 0.80, puddle));
                result.weight = clamp(strength * smoothstep(0.01, 0.10, edge), 0.0, max_sky_weight);
            }
        }
        return ssr_label(
            result,
            label_march,
            select(
                select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.45, 0.20, 0.0), left_screen),
                vec3<f32>(1.0, 0.55, 0.0),
                result.weight > 0.0
            )
        );
    }

    // Narrow the crossing. This is the bisection refinement used by Bevy's
    // hybrid root finder, adapted to this renderer's linear-depth convention.
    var lo = miss_t;
    var hi = hit_t;
    for (var step = 0u; step < refinement_steps; step = step + 1u) {
        let mid = (lo + hi) * 0.5;
        let sample_world = ray_origin + ray_dir * max_ray_distance * mid;
        let projected = project_uv(sample_world);
        if (projected.x <= 0.0 || projected.x >= 1.0 ||
            projected.y <= 0.0 || projected.y >= 1.0) {
            hi = mid;
            continue;
        }
        let sampled_depth = depth_at_uv(projected.xy);
        if (!valid_depth(sampled_depth)) {
            lo = mid;
            continue;
        }
        let expected_depth = distance(settings.camera_pos_time.xyz, sample_world);
        let penetration = expected_depth - sampled_depth;
        if (penetration > 0.0) {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    let final_world = ray_origin + ray_dir * max_ray_distance * hi;
    let hit_projection = project_uv(final_world);
    if (hit_projection.x <= 0.0 || hit_projection.x >= 1.0 ||
        hit_projection.y <= 0.0 || hit_projection.y >= 1.0) {
        return ssr_label(result, label_march, vec3<f32>(1.0, 0.0, 1.0));
    }

    let final_depth = depth_at_uv(hit_projection.xy);
    if (!valid_depth(final_depth)) {
        return ssr_label(result, label_march, vec3<f32>(0.0, 1.0, 1.0));
    }
    let expected_final_depth = distance(settings.camera_pos_time.xyz, final_world);
    let final_penetration = expected_final_depth - final_depth;
    // The thickness test asks "did the ray hit this surface, or pass behind it",
    // and it may never be stricter than the search that located the crossing. The
    // bisection leaves the true crossing somewhere in [lo, hi], so that span is the
    // smallest penetration this march can actually distinguish from a hit.
    //
    // Nothing enforced that. The wet high-quality path stretches max_ray_distance
    // to 600 + 0.9 x depth without raising the step or refinement budget, so at a
    // floor 25 m away the residual span is about 47 units at Medium and 19 at High
    // against an 18 unit window: real hits were rejected, in a lattice that follows
    // wherever the coarse steps happened to land. Only Ultra, with 16 steps and 4
    // bisections, resolved finely enough to pass - which is exactly the Medium worse
    // than High worse than Ultra ordering the artifact showed.
    //
    // Taking the residual as a floor also makes the quality tiers mean something
    // honest: more refinement narrows the span, which tightens the test by itself.
    let residual_span = (hi - lo) * max_ray_distance;
    let thickness = max(max(18.0, expected_final_depth * 0.008), residual_span);
    if (final_penetration <= 0.0 || final_penetration >= thickness) {
        return ssr_label(result, label_march, vec3<f32>(1.0, 1.0, 0.0));
    }
    if (label_march) {
        return ssr_label(result, true, vec3<f32>(0.0, 1.0, 0.0));
    }

    let edge_distance = min(
        min(hit_projection.x, 1.0 - hit_projection.x),
        min(hit_projection.y, 1.0 - hit_projection.y)
    );
    // SSR's confidence falls off toward the viewport border, because a hit found
    // near the edge is one the screen barely contained - and a little further in
    // that direction the ray leaves entirely and the pixel gets nothing at all.
    // Fading over only the outer 8% made that handover a visible line across the
    // ground at constant depth: full reflection on one side, none on the other.
    // Over a quarter of the frame it becomes a ramp, and the probe/sky coat in
    // weather_surface_world.wgsl - which is already bound on every wet surface
    // for exactly this case - carries the pixels the screen cannot answer for.
    let edge_fade = smoothstep(0.0, 0.24, edge_distance);
    // Long reach must not dim far facades away: only the last stretch of a
    // high-quality wet ray fades with travel.
    let travel_fade = select(1.0 - hi, 1.0 - hi * hi, high_quality && weather_reflective);
    result.radiance = scene(hit_projection.xy);
    // The streak belongs to the rough wet *film* and to nothing else. `look.y` is
    // the r_puddleQuality flag, not a wetness value, so gating on it alone smeared
    // every reflection on a bone-dry map - invisible at lower settings only because
    // they reject rough materials from SSR outright, and glaring at Ultra, which
    // admits every surface. Weight it by how much of this pixel actually is wet
    // film: dry drops to a single sharp tap, and so does standing water, which is
    // the one thing on screen that should mirror properly.
    let film_streak = clamp(wetness, 0.0, 1.0) * (1.0 - water) * weather_surface.look.y;
    if (high_quality && film_streak > 0.01) {
        // Rougher film smears further than still water; a patchy noise keeps
        // neighbouring stretches of street from streaking identically.
        let patchiness = 0.6 + 0.8 * weather_value_noise(world.xz / 140.0);
        let length_px = settings.viewport_history.y * 0.030 * film_streak * patchiness;
        result.radiance = ssr_streaked_scene(hit_projection.xy, final_world, length_px, jitter);
    }
    let max_weight = mix(mix(0.42, 0.60, wetness), 0.92, smoothstep(0.05, 0.80, puddle));
    result.weight = clamp(strength * travel_fade * edge_fade, 0.0, max_weight);
    return result;
}

fn temporal_ssr(uv: vec2<f32>, centre_depth: f32, current: SsrSample) -> SsrSample {
    if (settings.viewport_history.z <= 0.5 || !valid_depth(centre_depth)) {
        return current;
    }

    let world = world_position(uv, centre_depth);
    let reprojection = previous_uv(world);
    if (reprojection.x <= 0.0 || reprojection.x >= 1.0 ||
        reprojection.y <= 0.0 || reprojection.y >= 1.0) {
        return current;
    }

    // FidelityFX DNSR keeps previous depth alongside reflection radiance so
    // reprojected samples from a different surface can be rejected before they
    // form trails. Do the same here, using source-surface linear depth.
    let previous_depth = history_depth_at_uv(reprojection.xy);
    if (!valid_depth(previous_depth)) {
        return current;
    }
    let expected_previous_depth = distance(settings.previous_camera_pos.xyz, world);
    let depth_tolerance = max(10.0, expected_previous_depth * 0.018);
    if (abs(previous_depth - expected_previous_depth) > depth_tolerance) {
        return current;
    }

    var previous = textureSample(
        history_radiance_texture,
        scene_sampler,
        reprojection.xy
    );
    let history_dims = vec2<f32>(textureDimensions(history_radiance_texture));
    let motion_pixels = length((reprojection.xy - uv) * history_dims);
    var history_weight = clamp(0.90 - motion_pixels * 0.055, 0.0, 0.90);

    // A fresh hit should appear immediately instead of being diluted by empty
    // history. Conversely, a stochastic miss is allowed to retain some valid
    // history for a frame or two, which is the whole point of temporal SSR.
    if (current.weight > 0.001 && previous.a <= 0.001) {
        history_weight = 0.0;
    } else if (current.weight <= 0.001) {
        history_weight *= 0.68;
    }

    if (current.weight > 0.001 && previous.a > 0.001) {
        // Compact temporal clamp. FidelityFX uses a fuller variance-guided
        // denoiser; this keeps the same anti-firefly principle without adding
        // another spatial pass or variance texture to this lightweight client.
        let current_luma = dot(current.radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
        let previous_luma = dot(previous.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let max_luma = current_luma * 3.5 + 0.20;
        if (previous_luma > max_luma && previous_luma > 1e-5) {
            previous = vec4<f32>(previous.rgb * (max_luma / previous_luma), previous.a);
        }
    }

    var result: SsrSample;
    result.radiance = mix(current.radiance, previous.rgb, history_weight);
    result.weight = mix(current.weight, previous.a, history_weight);
    return result;
}

@fragment
fn fs_main(input: VertexOut) -> SsrFragmentOut {
    var out: SsrFragmentOut;
    // The policy/linear-depth buffers describe BSP geometry.  If the final
    // opaque scene has a closer depth owner at this pixel (player, vehicle,
    // ocean, grass/snow shell, etc.), erase SSR history here rather than merely
    // returning a current-frame miss that temporal accumulation could revive.
    if (!ssr_frontmost_at_uv(input.uv)) {
        out.radiance = vec4<f32>(0.0);
        out.depth = 0.0;
        return out;
    }
    let centre_depth = depth_at_uv(input.uv);
    if (!valid_depth(centre_depth)) {
        out.radiance = vec4<f32>(0.0);
        out.depth = 0.0;
        return out;
    }

    let current = current_ssr(input.uv, centre_depth, input.position.xy);
    let resolved = temporal_ssr(input.uv, centre_depth, current);
    // Standing water may reflect almost fully; everything else is capped lower by
    // current_ssr itself.
    out.radiance = vec4<f32>(max(resolved.radiance, vec3<f32>(0.0)), clamp(resolved.weight, 0.0, 0.94));
    out.depth = centre_depth;
    return out;
}

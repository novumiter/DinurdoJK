// Conservative runtime-light influence mask. No shadow rays are traced here:
// the forward material pass remains the sole source of RT lighting samples.
@group(1) @binding(0) var<storage, read> rt_taa_lights: array<PointLight>;
@group(1) @binding(1) var<storage, read> rt_taa_clusters: array<ClusterRecord>;
@group(1) @binding(2) var<uniform> rt_taa_lighting: LightingSettings;

fn rt_taa_mask(uv: vec2<f32>, depth: f32) -> bool {
    if (!valid_depth(depth) || rt_taa_lighting.values.y == 0u) { return false; }
    let world = world_position(uv, depth);
    let tile = min(vec2<u32>(uv * vec2<f32>(16.0, 9.0)), vec2<u32>(15u, 8u));
    let slice = min(u32(clamp(log(max(depth, 1.0)) / log(65536.0), 0.0, 0.999999) * 24.0), 23u);
    let cluster_index = slice * 144u + tile.y * 16u + tile.x;
    for (var i = 0u; i < min(rt_taa_clusters[cluster_index].count, 32u); i += 1u) {
        let index = rt_taa_clusters[cluster_index].indices[i];
        if (index >= rt_taa_lighting.values.y) { continue; }
        let light = rt_taa_lights[index];
        if (light.shadow.w < 0.5 || light.color_intensity.a <= 0.0
            || max(max(light.color_intensity.r, light.color_intensity.g), light.color_intensity.b) <= 0.0) { continue; }
        let delta = world - light.position_radius.xyz;
        let radius = light.position_radius.w + max(light.shadow.z, 0.0)
            + select(0.0, length(light.emitter.xyz), light.emitter.w < -0.5);
        if (light.position_radius.w <= 0.0 || dot(delta, delta) >= radius * radius) { continue; }
        if (light.emitter.w > 2.5 && dot(delta, delta) > 1e-8) {
            if (dot(normalize(light.emitter.xyz), normalize(delta)) <= light.shape.y) { continue; }
        }
        return true;
    }
    return false;
}

// Fits both LDR and HDR history alpha. Zero explicitly means unfiltered.
fn rt_taa_pack_depth(depth: f32) -> f32 {
    return clamp(log2(max(depth, 1.0)) / 20.0 + 0.02, 0.02, 1.0);
}

fn rt_taa_resolve(uv: vec2<f32>) -> TaaResolve {
    let dims = textureDimensions(scene_texture);
    let pixel = taa_pixel_from_uv(uv, dims);
    var current = textureLoad(scene_texture, pixel, 0).rgb;
    if (settings.taa_params.x > 0.5) { current = taa_tonemap(current); }
    let depth = depth_at_pixel(pixel);
    if (!rt_taa_mask(uv, depth)) { return TaaResolve(current, 0.0); }
    let packed_depth = rt_taa_pack_depth(depth);
    if (settings.scene.w <= 0.5) { return TaaResolve(current, packed_depth); }

    let motion = taa_motion_nearest(uv);
    let history_uv = uv - motion;
    if (any(history_uv < vec2<f32>(0.0)) || any(history_uv > vec2<f32>(1.0))) {
        return TaaResolve(current, packed_depth);
    }
    let previous_depth = textureLoad(history_texture, taa_pixel_from_uv(history_uv, dims), 0).a;
    let world = world_position(uv, depth);
    let expected_depth = rt_taa_pack_depth(distance(world, settings.prev_camera_pos_time.xyz));
    // Includes the 8-bit alpha quantization error on the LDR scene path.
    if (previous_depth <= 0.0 || abs(previous_depth - expected_depth) > 0.008) {
        return TaaResolve(current, packed_depth);
    }

    // One bilinear history tap instead of global TAA's five Catmull-Rom taps.
    // A 3x3 variance box limits trails from moving lights, sabers and particles.
    let texel = 1.0 / vec2<f32>(dims);
    var sum = vec3<f32>(0.0);
    var square_sum = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let sample = taa_sample_current_nearest(uv + vec2<f32>(f32(x), f32(y)) * texel);
            sum += sample;
            square_sum += sample * sample;
        }
    }
    let mean = sum / 9.0;
    let sigma = sqrt(max(square_sum / 9.0 - mean * mean, vec3<f32>(0.0)));
    let history = textureSampleLevel(history_texture, scene_sampler, history_uv, 0.0).rgb;
    let clipped = taa_ycocg_to_rgb(taa_clip_towards_aabb_center(
        taa_rgb_to_ycocg(history), taa_rgb_to_ycocg(current), mean - sigma, mean + sigma));
    let moving = length(motion * vec2<f32>(dims));
    let weight = mix(0.1, 0.25, clamp(moving / 8.0, 0.0, 1.0));
    return TaaResolve(mix(clipped, current, weight), packed_depth);
}

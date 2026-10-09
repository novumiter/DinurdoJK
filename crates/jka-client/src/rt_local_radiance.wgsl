// Separate, albedo-free stochastic RT irradiance for finite runtime lights.
// The material BRDF remains in the authoritative forward shader. This pass
// traces the dominant finite transient emitter at the depth-prepass surface.
// raw.a is its stable frame-local light index + 1; 0 means no eligible light.
struct LocalRadianceParams {
    inverse_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    previous_eye: vec4<f32>,
    sun_rgb: vec4<f32>,
    dimensions: vec4<u32>,
    flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> local_params: LocalRadianceParams;
@group(1) @binding(1) var local_depth: texture_2d<f32>;
@group(1) @binding(2) var<storage, read> dynamic_lights: array<PointLight>;
@group(1) @binding(3) var<storage, read> light_clusters: array<ClusterRecord>;
@group(1) @binding(4) var<uniform> lighting_settings: LightingSettings;
@group(1) @binding(5) var local_raw: texture_storage_2d<rgba16float, write>;
@group(1) @binding(6) var local_reference: texture_storage_2d<rgba16float, write>;

fn local_depth_at(pixel: vec2<i32>) -> f32 {
    let q = clamp(pixel, vec2<i32>(0), vec2<i32>(local_params.dimensions.xy) - vec2<i32>(1));
    return textureLoad(local_depth, q, 0).x;
}
fn local_position_at(pixel: vec2<i32>) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / vec2<f32>(local_params.dimensions.xy);
    let h = local_params.inverse_view_proj * vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.5, 1.0);
    return local_params.eye.xyz + normalize(h.xyz / h.w - local_params.eye.xyz) * local_depth_at(pixel);
}
fn local_normal_at(pixel: vec2<i32>) -> vec3<f32> {
    let c = local_position_at(pixel);
    let l = local_position_at(pixel + vec2<i32>(-1, 0)) - c;
    let r = local_position_at(pixel + vec2<i32>(1, 0)) - c;
    let u = local_position_at(pixel + vec2<i32>(0, -1)) - c;
    let d = local_position_at(pixel + vec2<i32>(0, 1)) - c;
    let dx = select(r, -l, dot(l, l) < dot(r, r));
    let dy = select(d, -u, dot(u, u) < dot(d, d));
    let n = cross(dx, dy);
    if (dot(n, n) < 1e-10) { return vec3<f32>(0.0); }
    let out = normalize(n);
    return out * select(-1.0, 1.0, dot(out, local_params.eye.xyz - c) >= 0.0);
}
fn local_luminance(value: vec3<f32>) -> f32 {
    return dot(value, vec3<f32>(0.2126, 0.7152, 0.0722));
}
fn local_trace_throughput(light: PointLight, world_position: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let delta = light.position_radius.xyz - world_position;
    let distance_squared = dot(delta, delta);
    let radius = max(light.position_radius.w, 0.0);
    if (distance_squared <= 1e-8 || distance_squared >= radius * radius) { return vec3<f32>(1.0); }
    let direction = delta * inverseSqrt(distance_squared);
    let surface_normal = normal * inverseSqrt(max(dot(normal, normal), 1e-12));
    let oriented_normal = select(-surface_normal, surface_normal, dot(surface_normal, direction) >= 0.0);
    // Limit the offset near an emitter, and recompute the direction from the
    // biased origin so the finite segment still ends exactly at that emitter.
    let origin = world_position + oriented_normal * min(0.5, sqrt(distance_squared) * 0.25);
    let to_emitter = light.position_radius.xyz - origin;
    let distance = length(to_emitter);
    if (distance <= 0.5) {
        return vec3<f32>(1.0);
    }
    return rt_trace_shadow_query(origin, to_emitter / distance, distance - 0.25,
        (lighting_settings.feature_flags.z & 0x200u) != 0u);
}


@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= local_params.dimensions.xy)) { return; }
    let pixel = vec2<i32>(id.xy);
    let z = local_depth_at(pixel);
    if (z <= 0.0 || z >= 999999.0 || lighting_settings.values.y == 0u) {
        textureStore(local_raw, pixel, vec4<f32>(0.0));
        textureStore(local_reference, pixel, vec4<f32>(0.0));
        return;
    }
    let position = local_position_at(pixel);
    let normal = local_normal_at(pixel);
    if (dot(normal, normal) < 0.5) {
        textureStore(local_raw, pixel, vec4<f32>(0.0));
        textureStore(local_reference, pixel, vec4<f32>(0.0));
        return;
    }
    let input = VertexOut(vec4<f32>(vec2<f32>(id.xy) + vec2<f32>(0.5), 0.0, 1.0), position);
    let cluster = cluster_for_fragment(input);
    var chosen = 0u;
    var best = 0.0;
    // Only transient finite emitters (saber capsules, finite blaster/FX lights)
    // contain stochastic RT samples. Point and authored lights keep their exact
    // legacy/RT material path, never silently replace it with this estimate.
    for (var i = 0u; i < min(cluster.count, 32u); i += 1u) {
        let li = cluster.indices[i];
        if (li >= lighting_settings.values.y) { continue; }
        let source = dynamic_lights[li];
        if (source.shadow.w < 0.5 || (source.emitter.w >= -0.5 && source.shadow.z <= 1.0e-4)) { continue; }
        let delta = source.position_radius.xyz - position;
        let dist = length(delta);
        if (dist <= 0.0001 || dist >= source.position_radius.w) { continue; }
        let direction = delta / dist;
        let facing = max(dot(normal, direction), 0.0);
        let score = local_luminance(source.color_intensity.rgb) * max(source.color_intensity.a, 0.0)
            * local_light_attenuation(source, dist) * emitter_visibility(source, direction) * facing;
        if (score > best) { best = score; chosen = li; }
    }
    if (best <= 1.0e-6) {
        textureStore(local_raw, pixel, vec4<f32>(0.0));
        textureStore(local_reference, pixel, vec4<f32>(0.0));
        return;
    }
    let emitter = dynamic_lights[chosen];
    let count = rt_local_sample_count(emitter);
    var shadowed = vec3<f32>(0.0);
    var unshadowed = vec3<f32>(0.0);
    for (var j = 0u; j < count; j += 1u) {
        let sample = rt_sample_local_emitter(emitter, input, chosen, j, count);
        let delta = sample.position_radius.xyz - position;
        let dist = length(delta);
        if (dist <= 0.0001 || dist >= sample.position_radius.w) { continue; }
        let direction = delta / dist;
        let facing = max(dot(normal, direction), 0.0);
        let w = sample.color_intensity.rgb * sample.color_intensity.a
            * local_light_attenuation(sample, dist) * emitter_visibility(sample, direction) * facing;
        unshadowed += w;
        shadowed += w * local_trace_throughput(sample, position, normal);
    }
    let selected_id = f32(chosen + 1u);
    textureStore(local_raw, pixel, vec4<f32>(shadowed, selected_id));
    textureStore(local_reference, pixel, vec4<f32>(unshadowed, selected_id));
}

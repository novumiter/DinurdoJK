struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos_time: vec4<f32>,
    clip_plane: vec4<f32>,
    render_flags: vec4<u32>,
    camera_forward: vec4<f32>,
};

struct WindVisualUniform {
    // Instantaneous atmospheric wind X/Z (JKA units/s), absolute time, user strength.
    wind_time: vec4<f32>,
    // Stable base wind X/Z (JKA units/s), accumulated X/Z advection is separate.
    base_wind: vec4<f32>,
    // Integrated instantaneous atmospheric transport in X/Z.
    advection: vec4<f32>,
    // Weather-heightfield min X/Z and reciprocal X/Z extents.
    collision_uv: vec4<f32>,
    // Heightfield width/height/active, then camera-underwater flag.
    collision_size: vec4<u32>,
    // Camera-centred volume radius, half-height, mote size scale, wisp width scale.
    volume: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> wind_visuals: WindVisualUniform;
@group(1) @binding(1) var collision_height: texture_2d<f32>;
@group(1) @binding(2) var wind_noise: texture_2d<f32>;
@group(1) @binding(3) var wind_noise_sampler: sampler;

const TAU: f32 = 6.28318530717958647692;
const JKA_TO_GODOT_METERS: f32 = 1.0 / 64.0;

fn hash_u32(x0: u32) -> u32 {
    var x = x0;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

fn hash01(seed: u32) -> f32 {
    return f32(hash_u32(seed) & 0x00ffffffu) / 16777215.0;
}

fn positive_mod(v: vec3<f32>, span: vec3<f32>) -> vec3<f32> {
    return v - floor(v / span) * span;
}

// A deterministic world-space torus around the camera. The camera can move freely
// without dragging the particles with it; only particles that cross a volume edge
// wrap to the opposite side.
fn wrapped_particle_position(index: u32, salt: u32, y_bias: f32) -> vec3<f32> {
    let radius = wind_visuals.volume.x;
    let half_height = wind_visuals.volume.y;
    let span = vec3<f32>(radius * 2.0, half_height * 2.0, radius * 2.0);
    let seed = hash_u32((index + 1u) ^ salt);
    let base = vec3<f32>(
        hash01(seed ^ 0xa341316cu) * span.x,
        hash01(seed ^ 0xc8013ea4u) * span.y,
        hash01(seed ^ 0xad90777du) * span.z
    );
    let transported = base + vec3<f32>(wind_visuals.advection.x, 0.0, wind_visuals.advection.y);
    let camera_relative = positive_mod(transported - camera.camera_pos_time.xyz + span * 0.5, span) - span * 0.5;
    var world = camera.camera_pos_time.xyz + camera_relative;
    world.y += y_bias;
    return world;
}

fn local_atmospheric_wind(world_xz: vec2<f32>) -> vec2<f32> {
    // This deliberately matches the rain/grass field: current gust/veer defines
    // the prevailing vector while the canonical GodotGrass texture provides broad
    // spatial direction and strength variation advected by the stable base wind.
    let instantaneous = wind_visuals.wind_time.xy;
    let speed = length(instantaneous);
    if (speed <= 0.001) {
        return vec2<f32>(0.0);
    }
    let prevailing = instantaneous / speed;
    let root_m = world_xz * JKA_TO_GODOT_METERS;
    let advected_root = root_m
        - wind_visuals.base_wind.xy * wind_visuals.wind_time.z * JKA_TO_GODOT_METERS;
    let direction_noise = textureSampleLevel(
        wind_noise,
        wind_noise_sampler,
        advected_root.yx * 0.005,
        0.0
    ).x;
    let strength_noise = textureSampleLevel(
        wind_noise,
        wind_noise_sampler,
        advected_root * 0.025,
        0.0
    ).x;
    let direction_offset = (direction_noise * 2.0 - 1.0) * (TAU / 12.0);
    let s = sin(direction_offset);
    let c = cos(direction_offset);
    let local_dir = vec2<f32>(
        prevailing.x * c - prevailing.y * s,
        prevailing.x * s + prevailing.y * c
    );
    var local_strength = mix(0.25, 1.0, strength_noise);
    local_strength *= local_strength;
    return local_dir * speed * mix(0.75, 1.25, local_strength);
}

fn outside_exposure(position: vec3<f32>) -> f32 {
    if (wind_visuals.collision_size.w != 0u) {
        return 0.0;
    }
    let width = wind_visuals.collision_size.x;
    let height = wind_visuals.collision_size.y;
    if (wind_visuals.collision_size.z == 0u || width == 0u || height == 0u) {
        return 1.0;
    }
    let uv = (position.xz - wind_visuals.collision_uv.xy) * wind_visuals.collision_uv.zw;
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return 1.0;
    }
    let pixel = min(
        vec2<u32>(uv * vec2<f32>(f32(width), f32(height))),
        vec2<u32>(width - 1u, height - 1u)
    );
    let field = textureLoad(collision_height, vec2<i32>(i32(pixel.x), i32(pixel.y)), 0);
    // x is the rain-blocking roof/cover height, -1e20 when unmapped. A soft
    // vertical band prevents a hard pop right at eaves and roof edges.
    if (field.x < -1.0e19) {
        return 1.0;
    }
    return smoothstep(-18.0, 54.0, position.y - field.x);
}

fn camera_side(direction: vec3<f32>, position: vec3<f32>) -> vec3<f32> {
    let to_camera_raw = camera.camera_pos_time.xyz - position;
    var to_camera = camera.camera_forward.xyz;
    if (dot(to_camera_raw, to_camera_raw) > 1e-6) {
        to_camera = normalize(to_camera_raw);
    }
    var side = cross(direction, to_camera);
    if (dot(side, side) < 1e-6) {
        side = cross(direction, vec3<f32>(0.0, 1.0, 0.0));
    }
    if (dot(side, side) < 1e-6) {
        side = vec3<f32>(1.0, 0.0, 0.0);
    }
    return normalize(side);
}

struct MoteOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) seed: f32,
};

@vertex
fn vs_mote(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> MoteOut {
    var out: MoteOut;
    let seed = hash_u32((instance_index + 1u) ^ 0x9e3779b9u);
    let bob_phase = hash01(seed ^ 0x7f4a7c15u) * TAU;
    let bob = sin(wind_visuals.wind_time.z * mix(0.35, 0.75, hash01(seed ^ 0x94d049bbu)) + bob_phase)
        * mix(5.0, 22.0, hash01(seed ^ 0x5bd1e995u));
    let world = wrapped_particle_position(instance_index, 0x68bc21ebu, bob);
    let local_wind_xz = local_atmospheric_wind(world.xz);
    let speed = length(local_wind_xz);
    let dir_xz = select(vec2<f32>(1.0, 0.0), local_wind_xz / max(speed, 0.001), speed > 0.001);
    let direction = normalize(vec3<f32>(dir_xz.x, mix(0.02, 0.12, hash01(seed ^ 0x27d4eb2du)), dir_xz.y));
    let distance = length(camera.camera_pos_time.xyz - world);
    let size = mix(1.2, 3.2, hash01(seed ^ 0x165667b1u)) * wind_visuals.volume.z;
    let streak = size * mix(1.0, 4.0, smoothstep(180.0, 1000.0, speed));
    let side_world = camera_side(direction, world);

    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0)
    );
    let corner = corners[vertex_index];
    let world_vertex = world + direction * (corner.y * streak * 0.5) + side_world * (corner.x * size * 0.5);
    let clip = camera.view_proj * vec4<f32>(world_vertex, 1.0);
    let exposure = outside_exposure(world);
    let near_fade = smoothstep(56.0, 110.0, distance);
    let far_fade = 1.0 - smoothstep(wind_visuals.volume.x * 0.70, wind_visuals.volume.x, distance);
    let speed_gain = mix(0.22, 1.0, smoothstep(80.0, 650.0, speed));
    out.position = clip;
    out.local = corner;
    out.opacity = wind_visuals.wind_time.w * exposure * near_fade * far_fade * speed_gain
        * mix(0.12, 0.32, hash01(seed ^ 0xd3a2646cu));
    out.seed = hash01(seed ^ 0xfd7046c5u);
    return out;
}

@fragment
fn fs_mote(input: MoteOut) -> @location(0) vec4<f32> {
    let radial = length(vec2<f32>(input.local.x, input.local.y * 0.45));
    let profile = 1.0 - smoothstep(0.28, 1.0, radial);
    let alpha = input.opacity * profile;
    if (alpha < 0.004) {
        discard;
    }
    let warm = mix(vec3<f32>(0.72, 0.70, 0.65), vec3<f32>(0.88, 0.90, 0.91), input.seed);
    return vec4<f32>(warm, alpha);
}

struct WispOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) world_xz: vec2<f32>,
    @location(3) seed: f32,
};

@vertex
fn vs_wisp(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> WispOut {
    var out: WispOut;
    let seed = hash_u32((instance_index + 1u) ^ 0x243f6a88u);
    let centre = wrapped_particle_position(instance_index, 0xb7e15162u, mix(-80.0, 180.0, hash01(seed ^ 0x13198a2eu)));
    let local_wind_xz = local_atmospheric_wind(centre.xz);
    let speed = length(local_wind_xz);
    let dir_xz = select(vec2<f32>(1.0, 0.0), local_wind_xz / max(speed, 0.001), speed > 0.001);
    let direction = normalize(vec3<f32>(dir_xz.x, mix(-0.025, 0.05, hash01(seed ^ 0x03707344u)), dir_xz.y));
    let perpendicular = normalize(vec3<f32>(-dir_xz.y, 0.0, dir_xz.x));

    let segment = vertex_index / 2u;
    let side_sign = select(-1.0, 1.0, (vertex_index & 1u) != 0u);
    let along = f32(segment) / 3.0;
    let centered = along - 0.5;
    let length_world = mix(260.0, 760.0, hash01(seed ^ 0xa4093822u))
        * mix(0.72, 1.20, smoothstep(250.0, 1400.0, speed));
    let width_world = mix(12.0, 34.0, hash01(seed ^ 0x299f31d0u)) * wind_visuals.volume.w;
    let phase = wind_visuals.wind_time.z * mix(0.65, 1.15, hash01(seed ^ 0x082efa98u))
        + hash01(seed ^ 0xec4e6c89u) * TAU;
    let bend = sin(along * TAU + phase) * mix(8.0, 34.0, hash01(seed ^ 0x452821e6u));
    let lift = sin(along * 3.14159265 + phase * 0.47) * mix(3.0, 18.0, hash01(seed ^ 0x38d01377u));
    let centerline = centre + direction * (centered * length_world) + perpendicular * bend + vec3<f32>(0.0, lift, 0.0);
    let side_world = camera_side(direction, centerline);
    let world_vertex = centerline + side_world * (side_sign * width_world * 0.5);
    let distance = length(camera.camera_pos_time.xyz - centerline);

    out.position = camera.view_proj * vec4<f32>(world_vertex, 1.0);
    out.local = vec2<f32>(side_sign, along);
    out.opacity = wind_visuals.wind_time.w
        * outside_exposure(centerline)
        * smoothstep(230.0, 720.0, speed)
        * smoothstep(90.0, 180.0, distance)
        * (1.0 - smoothstep(wind_visuals.volume.x * 0.68, wind_visuals.volume.x, distance))
        * mix(0.10, 0.24, hash01(seed ^ 0xbe5466cfu));
    out.world_xz = centerline.xz;
    out.seed = hash01(seed ^ 0xc0ac29b7u);
    return out;
}

@fragment
fn fs_wisp(input: WispOut) -> @location(0) vec4<f32> {
    let across = abs(input.local.x);
    let edge = 1.0 - smoothstep(0.20, 1.0, across);
    let ends = smoothstep(0.0, 0.14, input.local.y) * (1.0 - smoothstep(0.74, 1.0, input.local.y));
    let noise_uv = input.world_xz * JKA_TO_GODOT_METERS * 0.018
        + vec2<f32>(input.local.y * 1.7, input.seed * 7.0);
    let n0 = textureSample(wind_noise, wind_noise_sampler, noise_uv).x;
    let n1 = textureSample(wind_noise, wind_noise_sampler, noise_uv * 2.31 + vec2<f32>(3.1, 1.7)).x;
    let broken = smoothstep(0.28, 0.72, n0 * 0.68 + n1 * 0.32);
    let alpha = input.opacity * edge * ends * mix(0.18, 1.0, broken);
    if (alpha < 0.003) {
        discard;
    }
    let tint = mix(vec3<f32>(0.68, 0.67, 0.63), vec3<f32>(0.82, 0.84, 0.84), input.seed);
    return vec4<f32>(tint, alpha);
}

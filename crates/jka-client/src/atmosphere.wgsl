struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos_time: vec4<f32>,
    clip_plane: vec4<f32>,
    render_flags: vec4<u32>,
    camera_forward: vec4<f32>,
};

struct AtmosphereUniform {
    counts: vec4<u32>,       // sand count, spacedust count
    wind_time: vec4<f32>,    // wind X/Z, effect-local time, absolute time
    strength: vec4<f32>,     // sand strength, spacedust strength
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> atmosphere: AtmosphereUniform;

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

fn nearest_periodic(base: vec3<f32>, period: vec3<f32>) -> vec3<f32> {
    let cell = floor((camera.camera_pos_time.xyz - base) / period + vec3<f32>(0.5));
    return base + cell * period;
}

fn triangle_wave(cycles: f32) -> f32 {
    // Cheap periodic -1..1 motion for thousands of particles. The visual job is
    // irregular drift, not a physically meaningful sine, so avoid transcendental
    // work in a vertex shader that executes four times per particle.
    return 1.0 - 4.0 * abs(fract(cycles) - 0.5);
}

fn safe_right() -> vec3<f32> {
    var right = cross(camera.camera_forward.xyz, vec3<f32>(0.0, 1.0, 0.0));
    if (dot(right, right) < 1e-5) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    }
    return normalize(right);
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) data: vec4<f32>, // kind, opacity, grain seed, distance
    @location(2) color: vec3<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOut {
    var out: VertexOut;
    let sand = instance_index < atmosphere.counts.x;
    let local_index = instance_index - select(atmosphere.counts.x, 0u, sand);
    let seed = hash_u32(local_index * 747796405u + select(2891336453u, 1181783497u, sand));
    let r0 = hash01(seed ^ 0x68bc21ebu);
    let r1 = hash01(seed ^ 0x02e5be93u);
    let r2 = hash01(seed ^ 0x967a889bu);
    let r3 = hash01(seed ^ 0x368cc8b7u);
    let time = atmosphere.wind_time.z;
    let wind = atmosphere.wind_time.xy;
    let wind_speed = length(wind);

    var world = vec3<f32>(0.0);
    var axis_x = vec3<f32>(1.0, 0.0, 0.0);
    var axis_y = vec3<f32>(0.0, 1.0, 0.0);
    var half_size = vec2<f32>(1.0);
    var opacity = 0.1;
    var color = vec3<f32>(1.0);
    var kind = 0.0;

    if (sand) {
        kind = 1.0;
        // Sand is an atmospheric stream, so even a map with no explicit legacy
        // wind gets a restrained default drift rather than a frozen smoke wall.
        var flow = wind;
        if (wind_speed < 18.0) {
            flow = vec2<f32>(155.0, 42.0);
        }
        let speed = max(length(flow), 1.0);
        let flow_dir = flow / speed;
        let cross_dir = vec2<f32>(-flow_dir.y, flow_dir.x);
        let period = vec3<f32>(2300.0, 820.0, 2300.0);
        let phase = vec3<f32>(r0 * period.x, r1 * period.y, r2 * period.z);
        let stream = flow * (0.58 + r3 * 0.54) * time;
        let broad = triangle_wave(time * (0.10 + r2 * 0.11) + r0) * (28.0 + 46.0 * r1);
        let lift = triangle_wave(time * (0.067 + r3 * 0.080) + r1) * (12.0 + 22.0 * r2);
        let base = phase + vec3<f32>(stream.x + cross_dir.x * broad, lift, stream.y + cross_dir.y * broad);
        world = nearest_periodic(base, period);

        let motion = normalize(vec3<f32>(flow_dir.x, 0.035 + 0.08 * (r2 - 0.5), flow_dir.y));
        let to_camera = normalize(camera.camera_pos_time.xyz - world);
        var side = cross(motion, to_camera);
        if (dot(side, side) < 1e-5) {
            side = safe_right();
        } else {
            side = normalize(side);
        }
        axis_x = motion;
        axis_y = side;
        let wisp = r3 > 0.72;
        half_size = select(
            vec2<f32>(34.0 + 78.0 * r0, 2.2 + 5.2 * r1),
            vec2<f32>(85.0 + 130.0 * r0, 10.0 + 22.0 * r1),
            wisp
        );
        opacity = select(0.075 + 0.075 * r2, 0.026 + 0.038 * r2, wisp)
            * clamp(atmosphere.strength.x, 0.0, 1.35);
        color = mix(vec3<f32>(0.64, 0.48, 0.28), vec3<f32>(0.88, 0.72, 0.46), r1);
    } else {
        kind = 2.0;
        let period = vec3<f32>(2500.0, 1450.0, 2500.0);
        let phase = vec3<f32>(r0 * period.x, r1 * period.y, r2 * period.z);
        // Stock spacedust is stationary until wind is supplied. Keep that identity,
        // but add a tiny non-translating float so modern motes do not look pasted on.
        let drift = wind * 0.22 * time;
        let flutter = vec3<f32>(
            triangle_wave(time * (0.027 + r1 * 0.030) + r2),
            triangle_wave(time * (0.021 + r2 * 0.027) + r3),
            triangle_wave(time * (0.024 + r0 * 0.033) + r1 + 0.25)
        ) * (2.0 + 5.0 * r3);
        let base = phase + vec3<f32>(drift.x, 0.0, drift.y) + flutter;
        world = nearest_periodic(base, period);
        axis_x = safe_right();
        axis_y = normalize(cross(axis_x, camera.camera_forward.xyz));
        let puff = r3 > 0.86;
        let size = select(2.8 + 8.0 * r0, 14.0 + 28.0 * r0, puff);
        half_size = vec2<f32>(size);
        opacity = select(0.16 + 0.20 * r1, 0.028 + 0.045 * r1, puff)
            * clamp(atmosphere.strength.y, 0.0, 1.25);
        color = mix(vec3<f32>(0.66, 0.74, 0.84), vec3<f32>(0.92, 0.95, 1.0), r2);
    }

    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0)
    );
    let local = corners[vertex_index];
    let world_vertex = world + axis_x * local.x * half_size.x + axis_y * local.y * half_size.y;
    let clip = camera.view_proj * vec4<f32>(world_vertex, 1.0);
    let distance = length(world - camera.camera_pos_time.xyz);
    let near_fade = smoothstep(select(18.0, 46.0, sand), select(70.0, 110.0, sand), distance);
    let far_fade = 1.0 - smoothstep(select(1500.0, 1350.0, sand), select(2200.0, 2050.0, sand), distance);

    out.position = clip;
    out.local = local;
    out.data = vec4<f32>(kind, opacity * near_fade * far_fade, r3, distance);
    out.color = color;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    var alpha = input.data.y;
    if (input.data.x < 1.5) {
        // Long sand filaments: rounded ends with a noisy, dusty core.  The noise
        // is analytic and tiny compared with a texture fetch per particle.
        let longitudinal = 1.0 - smoothstep(0.64, 1.0, abs(input.local.x));
        let transverse = 1.0 - smoothstep(0.20, 1.0, abs(input.local.y));
        let grain = 0.76 + 0.24 * triangle_wave(input.local.x * 3.66 + input.local.y * 2.71 + input.data.z * 3.02);
        alpha *= longitudinal * transverse * grain;
    } else {
        let radius = length(input.local);
        let core = 1.0 - smoothstep(0.16, 0.92, radius);
        let halo = (1.0 - smoothstep(0.55, 1.0, radius)) * 0.34;
        alpha *= max(core, halo);
    }
    if (alpha < 0.0025) {
        discard;
    }
    return vec4<f32>(input.color, alpha);
}

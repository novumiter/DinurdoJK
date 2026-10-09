struct SnowParticle {
    position_state: vec4<f32>, // xyz world position, w: 0 uninitialized / 1 falling
    velocity_seed: vec4<f32>,  // xyz velocity, w stable seed
    misc: vec4<f32>,           // generation, size seed, brightness seed, phase
};

struct SnowSimUniform {
    // xyz is a persistent world-space anchor; w is dt.
    camera_dt: vec4<f32>,
    spawn: vec4<f32>,
    wind_time: vec4<f32>,
    // xy is base weather wind X/Z; z is live camera Y for roof tests.
    weather: vec4<f32>,
    collision_uv: vec4<f32>,
    counts: vec4<u32>,
};

@group(0) @binding(0) var<storage, read_write> particles: array<SnowParticle>;
@group(0) @binding(1) var<uniform> sim: SnowSimUniform;
@group(0) @binding(2) var collision_height: texture_2d<f32>;
@group(0) @binding(3) var wind_noise: texture_2d<f32>;
@group(0) @binding(4) var wind_noise_sampler: sampler;

var<workgroup> shared_wind_velocity: vec2<f32>;

const TAU: f32 = 6.28318530717958647692;
const GOLDEN_ANGLE: f32 = 2.39996322972865332;
const JKA_TO_GODOT_METERS: f32 = 1.0 / 64.0;
const SNOW_GROUP_SIZE: u32 = 128u;
const SNOW_WEATHER_WIND_INHERITANCE: f32 = 0.72;
const SNOW_RADIAL_BIAS: f32 = 1.16;

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

fn snow_group_count() -> u32 {
    return max((sim.counts.x + SNOW_GROUP_SIZE - 1u) / SNOW_GROUP_SIZE, 1u);
}

struct SnowCell {
    area: vec2<f32>,
    angle: vec2<f32>,
};

fn snow_ring_count(groups: u32) -> u32 {
    return max(u32(sqrt(f32(groups) / 3.14159265)), 1u);
}

fn snow_ring_first_cell(ring: u32, groups: u32, rings: u32) -> u32 {
    return (2u * groups * ring * ring + rings * rings) / (2u * rings * rings);
}

fn snow_cell(group_index: u32) -> SnowCell {
    let groups = snow_group_count();
    let rings = snow_ring_count(groups);
    let cell_index = min(group_index, groups - 1u);
    var ring = 0u;
    var first = 0u;
    var next = snow_ring_first_cell(1u, groups, rings);
    for (var i = 1u; i < rings; i = i + 1u) {
        if (cell_index < next) { break; }
        ring = i;
        first = next;
        next = snow_ring_first_cell(i + 1u, groups, rings);
    }
    let ring_cells = f32(max(next - first, 1u));
    let sector = f32(cell_index - first);
    let ring_offset = f32(ring) * GOLDEN_ANGLE;
    var cell: SnowCell;
    cell.area = vec2<f32>(f32(first), f32(next)) / f32(groups);
    cell.angle = ring_offset + TAU * vec2<f32>(sector, sector + 1.0) / ring_cells;
    return cell;
}

fn snow_cell_point(cell: SnowCell, u: vec2<f32>) -> vec2<f32> {
    let angle = mix(cell.angle.x, cell.angle.y, u.y);
    let radius = pow(sqrt(mix(cell.area.x, cell.area.y, u.x)), SNOW_RADIAL_BIAS) * sim.spawn.x;
    return sim.camera_dt.xz + vec2<f32>(cos(angle), sin(angle)) * radius;
}

fn snow_group_center(group_index: u32) -> vec2<f32> {
    return snow_cell_point(snow_cell(group_index), vec2<f32>(0.5));
}

fn weather_wind_offset_jka(time: f32) -> vec2<f32> {
    return sim.weather.xy * time;
}

fn snow_airflow(world_xz: vec2<f32>) -> vec2<f32> {
    // Two noise reads per 128 flakes. Broad eddies are coherent in space and
    // slowly evolve even with zero authored wind, while Weather wind remains the
    // dominant transport when a map/server provides it.
    let root_m = world_xz * JKA_TO_GODOT_METERS;
    let time_drift = vec2<f32>(sim.wind_time.z * 0.0035, -sim.wind_time.z * 0.0027);
    let advected_root = root_m
        - weather_wind_offset_jka(sim.wind_time.z) * JKA_TO_GODOT_METERS
        + time_drift;
    let direction_noise = textureSampleLevel(
        wind_noise, wind_noise_sampler, advected_root.yx * 0.006, 0.0
    ).x;
    let strength_noise = textureSampleLevel(
        wind_noise, wind_noise_sampler, advected_root * 0.021, 0.0
    ).x;

    let weather = sim.wind_time.xy;
    let weather_speed = length(weather);
    let prevailing = weather / max(weather_speed, 0.0001);
    let direction_offset = (direction_noise * 2.0 - 1.0) * (TAU / 9.0);
    let s = sin(direction_offset);
    let c = cos(direction_offset);
    let local_prevailing = vec2<f32>(
        prevailing.x * c - prevailing.y * s,
        prevailing.x * s + prevailing.y * c
    );
    let authored = local_prevailing * weather_speed * SNOW_WEATHER_WIND_INHERITANCE;

    let eddy_angle = direction_noise * TAU + sim.wind_time.z * 0.055;
    let eddy_direction = vec2<f32>(cos(eddy_angle), sin(eddy_angle));
    let eddy_speed = mix(8.0, sim.wind_time.w, strength_noise * strength_noise);
    return authored + eddy_direction * eddy_speed;
}

fn respawn(index: u32, generation: f32, wind_velocity: vec2<f32>) -> SnowParticle {
    let initial_spawn = generation < 0.5;
    let gen = u32(max(generation, 0.0)) + 1u;
    let seed_u = hash_u32((index + 1u) ^ (gen * 0x9e3779b9u));
    let seed = hash01(seed_u ^ 0xa511e9b3u);
    let size_seed = hash01(seed_u ^ 0x63d83595u);
    let brightness_seed = hash01(seed_u ^ 0x1b873593u);
    let phase = hash01(seed_u ^ 0x85ebca6bu) * TAU;
    let spawn_xz = snow_cell_point(
        snow_cell(index / SNOW_GROUP_SIZE),
        vec2<f32>(hash01(seed_u ^ 0xb7e15162u), hash01(seed_u ^ 0x243f6a88u))
    );
    let height_jitter = hash01(seed_u ^ 0x3c6ef372u);
    let fall_jitter = mix(0.72, 1.28, hash01(seed_u ^ 0xa4093822u));
    let spawn_y = select(
        sim.camera_dt.y + sim.spawn.z * mix(0.68, 1.0, height_jitter),
        sim.camera_dt.y + mix(-sim.spawn.y, sim.spawn.z, height_jitter),
        initial_spawn
    );

    var particle: SnowParticle;
    particle.position_state = vec4<f32>(spawn_xz.x, spawn_y, spawn_xz.y, 1.0);
    particle.velocity_seed = vec4<f32>(
        wind_velocity.x,
        -sim.spawn.w * fall_jitter,
        wind_velocity.y,
        seed
    );
    particle.misc = vec4<f32>(f32(gen), size_seed, brightness_seed, phase);
    return particle;
}

fn collision_surface(position: vec3<f32>) -> f32 {
    let width = sim.counts.y;
    let height = sim.counts.z;
    if (width == 0u || height == 0u) { return -1.0e20; }
    let uv = (position.xz - sim.collision_uv.xy) * sim.collision_uv.zw;
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) { return -1.0e20; }
    let pixel_u = min(
        vec2<u32>(uv * vec2<f32>(f32(width), f32(height))),
        vec2<u32>(width - 1u, height - 1u)
    );
    return textureLoad(collision_height, vec2<i32>(i32(pixel_u.x), i32(pixel_u.y)), 0).x;
}

@compute @workgroup_size(128)
fn cs_main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
    @builtin(workgroup_id) workgroup_id: vec3<u32>
) {
    if (local_index == 0u) {
        // Sample airflow at a living flake so it stays tied to world space as the anchor advances.
        let representative_index = min(workgroup_id.x * SNOW_GROUP_SIZE, sim.counts.x - 1u);
        let representative = particles[representative_index];
        let wind_sample_xz = select(
            snow_group_center(workgroup_id.x),
            representative.position_state.xz,
            representative.position_state.w >= 0.5
        );
        shared_wind_velocity = snow_airflow(wind_sample_xz);
    }
    workgroupBarrier();

    let index = global_id.x;
    if (index >= sim.counts.x) { return; }

    var particle = particles[index];
    if (particle.position_state.w < 0.5) {
        particle = respawn(index, particle.misc.x, shared_wind_velocity);
    }

    let dt = clamp(sim.camera_dt.w, 0.0, 0.05);
    let seed = particle.velocity_seed.w;
    let phase = particle.misc.w;
    particle.misc.w = phase + dt * mix(0.85, 1.75, seed);
    // Cheap individual flutter: no particle-local texture sample. This rides on
    // the broad two-sample/workgroup eddy so neighbouring flakes move together
    // without looking mechanically identical.
    let flutter = vec2<f32>(
        sin(phase + particle.position_state.y * 0.011),
        cos(phase * 0.79 + particle.position_state.x * 0.006)
    ) * mix(7.0, 24.0, particle.misc.y);
    let lateral_target = shared_wind_velocity + flutter;
    let follow = 1.0 - exp(-dt * mix(1.2, 2.7, seed));
    particle.velocity_seed.x = mix(particle.velocity_seed.x, lateral_target.x, follow);
    particle.velocity_seed.z = mix(particle.velocity_seed.z, lateral_target.y, follow);

    let fall_target = -sim.spawn.w * mix(0.72, 1.28, particle.misc.z)
        + sin(phase * 0.53) * 9.0;
    particle.velocity_seed.y = mix(particle.velocity_seed.y, fall_target, 1.0 - exp(-dt * 1.35));

    let old_y = particle.position_state.y;
    let delta = particle.velocity_seed.xyz * dt;
    particle.position_state.x += delta.x;
    particle.position_state.y += delta.y;
    particle.position_state.z += delta.z;

    let relative = particle.position_state.xyz - sim.camera_dt.xyz;
    let outside = length(relative.xz) > sim.spawn.x
        || relative.y < -sim.spawn.y
        || relative.y > sim.spawn.z * 1.30;
    if (outside) {
        particles[index] = respawn(index, particle.misc.x, shared_wind_velocity);
        return;
    }

    let surface_y = collision_surface(particle.position_state.xyz);
    if (surface_y > -1.0e19) {
        // If the camera is below top-down cover, keep flakes above that cover so
        // ordinary scene depth hides them. Merely respawning would occasionally
        // put a new flake below the same roof for one visible indoor frame.
        if (surface_y > sim.weather.z + 12.0) {
            let parked = surface_y + max(sim.spawn.z * 0.12, 24.0);
            if (parked - sim.camera_dt.y <= sim.spawn.z) {
                particle.position_state.y = parked;
            } else {
                particle.position_state.w = 0.0;
            }
        } else if (old_y < surface_y - 2.0 || particle.position_state.y <= surface_y + 1.5) {
            // Outdoor flakes die at terrain contact. The first-frame volume fill
            // may seed below nearby ground; that path immediately becomes a top
            // respawn because its generation has already advanced. Persistent
            // accumulation belongs to the separate snow-deformation/footprint path.
            particle = respawn(index, particle.misc.x, shared_wind_velocity);
        }
    }

    particles[index] = particle;
}

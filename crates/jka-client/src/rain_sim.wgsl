struct RainParticle {
    position_state: vec4<f32>, // xyz world position, w: 0 uninitialized, 1 falling, 2 splash
    velocity_age: vec4<f32>,   // xyz velocity, w: splash age seconds
    misc: vec4<f32>,           // x: respawn generation, y: basin score, z: flat-area score of the impact surface
};

struct RainSimUniform {
    camera_dt: vec4<f32>,      // camera xyz, dt seconds
    spawn: vec4<f32>,          // horizontal radius, below camera, above camera, base fall speed
    wind_time: vec4<f32>,      // instantaneous weather wind X/Z JKA/s, absolute time, splash duration
    weather: vec4<f32>,        // base weather wind X/Z JKA/s, horizontal camera delta X/Z
    collision_uv: vec4<f32>,   // min render X/Z, inverse map width/depth
    counts: vec4<u32>,         // active particles, heightfield width/height, splash subset
    water_info: vec4<f32>,     // x: water volume count, y: use water field, z: fast bounds
    water_min: array<vec4<f32>, 8>, // water volume minimum corners (render space)
    water_max: array<vec4<f32>, 8>, // water volume maximum corners; y is the surface
};

@group(0) @binding(0) var<storage, read_write> particles: array<RainParticle>;
@group(0) @binding(1) var<uniform> sim: RainSimUniform;
@group(0) @binding(2) var collision_height: texture_2d<f32>;
@group(0) @binding(3) var wind_noise: texture_2d<f32>;
@group(0) @binding(4) var wind_noise_sampler: sampler;

// Real outlines of the promoted water surfaces (OceanMasks). A water volume's
// box is only the bounding rectangle of its water, so a ring around a dry
// island must not catch rain over the hollow. `water_min[i].w` is the volume's
// slot + 1, 0 meaning "the whole box is water".
struct WaterMasks {
    ranges: array<vec4<f32>, 8>,       // first triangle, triangle count
    triangles: array<vec4<f32>, 512>,  // per triangle: a.xy b.xy / c.xy
};
@group(0) @binding(5) var<uniform> water_masks: WaterMasks;
@group(0) @binding(6) var water_height_field: texture_2d<f32>;

// One broad GodotGrass wind sample is shared by an entire 128-drop workgroup.
// This deliberately keeps rain wind low-frequency and extremely cheap instead
// of doing one or more noise-texture reads per particle.
var<workgroup> shared_wind_velocity: vec2<f32>;

const TAU: f32 = 6.28318530717958647692;
const GOLDEN_ANGLE: f32 = 2.39996322972865332;
const JKA_TO_GODOT_METERS: f32 = 1.0 / 64.0;
const RAIN_GROUP_SIZE: u32 = 128u;
const RAIN_WEATHER_WIND_INHERITANCE: f32 = 1.00;
// Radius exponent for the spawn disc. 1.0 is uniform per unit ground area, which
// leaves almost nothing near the camera (volume grows with r^3); 1.25 gives about
// 2.4x average density at the centre falling to 0.83x at the rim.
const RAIN_RADIAL_BIAS: f32 = 1.25;

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

fn rain_group_count() -> u32 {
    return max((sim.counts.x + RAIN_GROUP_SIZE - 1u) / RAIN_GROUP_SIZE, 1u);
}

// Each 128-drop workgroup owns one cell of an equal-area polar partition of the
// camera-local rain disc, so the whole disc is covered at an exactly uniform
// density and a workgroup's single wind sample still describes a small patch.
//
// (The previous layout took the cell centre radius from fract(g * phi^-1) and its
// angle from g * GOLDEN_ANGLE. Since GOLDEN_ANGLE / TAU = 1 - phi^-1, angle was a
// function of radius: every centre sat on ONE spiral arm, leaving ~70% of the
// disc without rain, with the empty part fixed in world axes and dragged along
// with the camera.)
//
// Ring k holds cells first(k) .. first(k+1)-1 with first(k) = round(G * k^2 / M^2),
// so ring areas are proportional to their cell counts and every cell has the
// area pi R^2 / G. M = floor(sqrt(G / pi)) keeps cells roughly square.
struct RainCell {
    area: vec2<f32>,  // inner/outer radius^2 as a fraction of the disc, equal-area space
    angle: vec2<f32>, // start/end angle, radians
};

fn rain_ring_count(groups: u32) -> u32 {
    return max(u32(sqrt(f32(groups) / 3.14159265)), 1u);
}

fn rain_ring_first_cell(ring: u32, groups: u32, rings: u32) -> u32 {
    return (2u * groups * ring * ring + rings * rings) / (2u * rings * rings);
}

fn rain_cell(group_index: u32) -> RainCell {
    let groups = rain_group_count();
    let rings = rain_ring_count(groups);
    let cell_index = min(group_index, groups - 1u);

    var ring = 0u;
    var first = 0u;
    var next = rain_ring_first_cell(1u, groups, rings);
    for (var i = 1u; i < rings; i = i + 1u) {
        if (cell_index < next) {
            break;
        }
        ring = i;
        first = next;
        next = rain_ring_first_cell(i + 1u, groups, rings);
    }

    let ring_cells = f32(max(next - first, 1u));
    let sector = f32(cell_index - first);
    // Rotate each ring so the sector seams of neighbouring rings do not line up.
    let ring_offset = f32(ring) * GOLDEN_ANGLE;
    var cell: RainCell;
    cell.area = vec2<f32>(f32(first), f32(next)) / f32(groups);
    cell.angle = ring_offset + TAU * vec2<f32>(sector, sector + 1.0) / ring_cells;
    return cell;
}

// World XZ of a point in the cell; u.x picks the radius, u.y the angle (both 0..1).
// Uniform in area, then pulled toward the camera by RAIN_RADIAL_BIAS.
fn rain_cell_point(cell: RainCell, u: vec2<f32>) -> vec2<f32> {
    let angle = mix(cell.angle.x, cell.angle.y, u.y);
    let radius = pow(sqrt(mix(cell.area.x, cell.area.y, u.x)), RAIN_RADIAL_BIAS) * sim.spawn.x;
    return sim.camera_dt.xz + vec2<f32>(cos(angle), sin(angle)) * radius;
}

fn rain_group_center(group_index: u32) -> vec2<f32> {
    return rain_cell_point(rain_cell(group_index), vec2<f32>(0.5));
}

fn weather_wind_offset_jka(time: f32) -> vec2<f32> {
    return sim.weather.xy * time;
}

fn godot_grass_wind(world_xz: vec2<f32>) -> vec2<f32> {
    // Shared Weather wind is the prevailing atmospheric flow. The GodotGrass
    // field adds broad local variance and follows the stable base transport path;
    // instantaneous gust/veer still drive the mean rain slant through wind_time.xy.
    let weather_wind_jka = sim.wind_time.xy;
    let weather_speed_jka = length(weather_wind_jka);
    let prevailing_dir = weather_wind_jka / max(weather_speed_jka, 0.0001);
    let root_m = world_xz * JKA_TO_GODOT_METERS;
    let advected_root = root_m
        - weather_wind_offset_jka(sim.wind_time.z) * JKA_TO_GODOT_METERS;

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
    let offset_sin = sin(direction_offset);
    let offset_cos = cos(direction_offset);
    let local_dir = vec2<f32>(
        prevailing_dir.x * offset_cos - prevailing_dir.y * offset_sin,
        prevailing_dir.x * offset_sin + prevailing_dir.y * offset_cos
    );
    var strength = mix(0.25, 1.0, strength_noise);
    strength = strength * strength;

    // Shared Weather wind supplies the mean slant. Perlin only adds broad +/-30 degrees
    // direction variance and a subtle gust-strength variation around that mean.
    let inherited_speed = weather_speed_jka * RAIN_WEATHER_WIND_INHERITANCE;
    let lateral_speed = inherited_speed * mix(0.75, 1.25, strength);
    return local_dir * lateral_speed;
}

fn respawn(index: u32, generation: f32, wind_velocity: vec2<f32>) -> RainParticle {
    let initial_spawn = generation < 0.5;
    let gen = u32(max(generation, 0.0)) + 1u;
    let seed = hash_u32((index + 1u) ^ (gen * 0x9e3779b9u));

    // Particles are spatially grouped by compute workgroup: each group spawns
    // inside its own cell of the equal-area partition, so the single shared wind
    // sample corresponds to a real patch of rain and the groups tile the disc.
    let spawn_xz = rain_cell_point(
        rain_cell(index / RAIN_GROUP_SIZE),
        vec2<f32>(hash01(seed ^ 0xb7e15162u), hash01(seed ^ 0x243f6a88u))
    );

    let height_jitter = hash01(seed ^ 0x3c6ef372u);
    let speed_jitter = mix(0.82, 1.18, hash01(seed ^ 0xa4093822u));
    let spawn_y = select(
        sim.camera_dt.y + sim.spawn.z * mix(0.50, 1.0, height_jitter),
        sim.camera_dt.y + mix(-sim.spawn.y, sim.spawn.z, height_jitter),
        initial_spawn
    );

    var particle: RainParticle;
    particle.position_state = vec4<f32>(spawn_xz.x, spawn_y, spawn_xz.y, 1.0);
    particle.velocity_age = vec4<f32>(
        wind_velocity.x,
        -sim.spawn.w * speed_jitter,
        wind_velocity.y,
        0.0
    );
    particle.misc = vec4<f32>(f32(gen), 0.0, 0.0, 0.0);
    return particle;
}

// The weather field texel under a position: x rain-blocking height, y top surface
// height, z flat-area score, w basin score. x is -1e20 where nothing is mapped.
fn collision_field(position: vec3<f32>) -> vec4<f32> {
    let width = sim.counts.y;
    let height = sim.counts.z;
    let unmapped = vec4<f32>(-1.0e20, -1.0e20, 0.0, 0.0);
    if (width == 0u || height == 0u) {
        return unmapped;
    }

    let uv = (position.xz - sim.collision_uv.xy) * sim.collision_uv.zw;
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return unmapped;
    }

    let pixel_u = min(
        vec2<u32>(uv * vec2<f32>(f32(width), f32(height))),
        vec2<u32>(width - 1u, height - 1u)
    );
    return textureLoad(
        collision_height,
        vec2<i32>(i32(pixel_u.x), i32(pixel_u.y)),
        0
    );
}

// Whether the xz point lies on the slot's outline; same test and 0.75 unit
// growth as ocean_mask_contains in bsp.wgsl. Slots below zero are unmasked.
fn water_mask_contains(slot: i32, p: vec2<f32>) -> bool {
    if (slot < 0 || slot >= 8) {
        return true;
    }
    let range = water_masks.ranges[slot];
    let triangle_count = u32(range.y);
    if (triangle_count == 0u) {
        return true;
    }
    let first = u32(range.x);
    let tolerance = 0.75;
    for (var i = 0u; i < triangle_count; i = i + 1u) {
        let t0 = water_masks.triangles[(first + i) * 2u];
        let t1 = water_masks.triangles[(first + i) * 2u + 1u];
        let a = t0.xy;
        let b = t0.zw;
        let c = t1.xy;
        let lo = min(a, min(b, c)) - vec2<f32>(tolerance);
        let hi = max(a, max(b, c)) + vec2<f32>(tolerance);
        if (any(p < lo) || any(p > hi)) {
            continue;
        }
        let ab = b - a;
        let bc = c - b;
        let ca = a - c;
        let e0 = (ab.x * (p.y - a.y) - ab.y * (p.x - a.x)) / max(length(ab), 1.0e-4);
        let e1 = (bc.x * (p.y - b.y) - bc.y * (p.x - b.x)) / max(length(bc), 1.0e-4);
        let e2 = (ca.x * (p.y - c.y) - ca.y * (p.x - c.x)) / max(length(ca), 1.0e-4);
        if ((e0 >= -tolerance && e1 >= -tolerance && e2 >= -tolerance)
            || (e0 <= tolerance && e1 <= tolerance && e2 <= tolerance)) {
            return true;
        }
    }
    return false;
}

// O(1) A/B path. The field stores [bottom_y, top_y, 0, 0] in the same XZ
// grid as collision_height. It is rebuilt on the CPU only when promoted water
// changes, never per frame.
fn water_surface_from_field(position: vec3<f32>) -> f32 {
    let width = sim.counts.y;
    let height = sim.counts.z;
    if (width == 0u || height == 0u) {
        return -1.0e20;
    }
    let uv = (position.xz - sim.collision_uv.xy) * sim.collision_uv.zw;
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return -1.0e20;
    }
    let pixel_u = min(
        vec2<u32>(uv * vec2<f32>(f32(width), f32(height))),
        vec2<u32>(width - 1u, height - 1u)
    );
    let water = textureLoad(
        water_height_field,
        vec2<i32>(i32(pixel_u.x), i32(pixel_u.y)),
        0
    );
    if (water.y > -1.0e19 && position.y >= water.x && position.y <= water.y) {
        return water.y;
    }
    return -1.0e20;
}

// Legacy A path: top of the water volume this position is inside, or -1e20.
fn water_surface_containing(position: vec3<f32>) -> f32 {
    let count = min(u32(sim.water_info.x), 8u);
    for (var i = 0u; i < count; i = i + 1u) {
        let lo = sim.water_min[i].xyz;
        let hi = sim.water_max[i].xyz;
        if (position.x >= lo.x && position.x <= hi.x
            && position.z >= lo.z && position.z <= hi.z
            && position.y >= lo.y && position.y <= hi.y
            && water_mask_contains(i32(round(sim.water_min[i].w)) - 1, position.xz)) {
            return hi.y;
        }
    }
    return -1.0e20;
}

@compute @workgroup_size(128)
fn cs_main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
    @builtin(workgroup_id) workgroup_id: vec3<u32>
) {
    // Exactly two source-noise reads per 128-drop workgroup, regardless of rain
    // intensity. Every invocation must reach the barrier, including tail lanes.
    if (local_index == 0u) {
        shared_wind_velocity = godot_grass_wind(rain_group_center(workgroup_id.x));
    }
    workgroupBarrier();

    let index = global_id.x;
    if (index >= sim.counts.x) {
        return;
    }

    var particle = particles[index];
    let carry_with_camera = particle.position_state.w >= 0.5 && particle.position_state.w < 1.5;
    if (particle.position_state.w < 0.5) {
        // Seed the initial frame throughout the whole camera-local volume so
        // rain reaches full density immediately instead of filling from the top.
        particle = respawn(index, particle.misc.x, shared_wind_velocity);
    }

    let dt = clamp(sim.camera_dt.w, 0.0, 0.05);
    if (particle.position_state.w < 1.5) {
        // Hybrid local/world simulation: the falling field follows ordinary
        // camera translation so walking cannot outrun the camera-centred emitter,
        // but splash particles stay attached to the world surface they hit.
        // Newly seeded particles already use the current camera centre.
        if (carry_with_camera) {
            particle.position_state.x += sim.weather.z;
            particle.position_state.z += sim.weather.w;
        }
        // Existing drops ease toward the broad workgroup wind, so gust changes
        // travel smoothly instead of snapping only when a particle respawns.
        let wind_follow = clamp(dt * 2.5, 0.0, 0.125);
        particle.velocity_age.x = mix(particle.velocity_age.x, shared_wind_velocity.x, wind_follow);
        particle.velocity_age.z = mix(particle.velocity_age.z, shared_wind_velocity.y, wind_follow);

        let old_y = particle.position_state.y;
        let delta = particle.velocity_age.xyz * dt;
        particle.position_state.x += delta.x;
        particle.position_state.y += delta.y;
        particle.position_state.z += delta.z;

        let relative = particle.position_state.xyz - sim.camera_dt.xyz;
        var horizontal_outside = false;
        if (sim.water_info.z > 0.5) {
            horizontal_outside = dot(relative.xz, relative.xz) > sim.spawn.x * sim.spawn.x;
        } else {
            horizontal_outside = length(relative.xz) > sim.spawn.x;
        }
        let outside = horizontal_outside
            || relative.y < -sim.spawn.y
            || relative.y > sim.spawn.z * 1.35;
        if (outside) {
            particles[index] = respawn(index, particle.misc.x, shared_wind_velocity);
            return;
        }

        // Water is a floor for rain. A drop that reaches the surface splashes on
        // it; one that is already inside the volume (spawned there, or carried in
        // by the camera moving) is recycled instead of raining underwater.
        var water_top = -1.0e20;
        if (sim.water_info.y > 0.5) {
            water_top = water_surface_from_field(particle.position_state.xyz);
        } else {
            water_top = water_surface_containing(particle.position_state.xyz);
        }
        if (water_top > -1.0e19) {
            if (old_y > water_top && index < sim.counts.w) {
                particle.position_state.y = water_top + 1.5;
                particle.position_state.w = 2.0;
                particle.misc.y = 0.0;
                particle.misc.z = 0.0;
                particle.velocity_age = vec4<f32>(0.0);
                particles[index] = particle;
            } else {
                particles[index] = respawn(index, particle.misc.x, shared_wind_velocity);
            }
            return;
        }

        let field = collision_field(particle.position_state.xyz);
        let surface_y = field.x;
        if (surface_y > -1.0e19) {
            // If the camera moves under a roof while a drop is already below
            // that roof, do not let the stale drop rain inside. Put it above
            // the top-down cover so ordinary camera depth keeps it invisible.
            if (old_y < surface_y - 2.0) {
                let parked = surface_y + max(sim.spawn.z * 0.20, 24.0);
                if (parked - sim.camera_dt.y > sim.spawn.z) {
                    // Cover far above the rain column: parking the drop there puts
                    // it out in the void, where it draws for one frame before the
                    // bounds test recycles it. Recycle it now instead.
                    particles[index] = respawn(index, particle.misc.x, shared_wind_velocity);
                    return;
                }
                particle.position_state.y = parked;
            } else if (particle.position_state.y <= surface_y + 2.0) {
                if (index < sim.counts.w) {
                    // Only a representative subset needs visible splash state.
                    // The rest recycle immediately, keeping splash vertex cost
                    // small while impacts still read as dense in heavy rain.
                    particle.position_state.y = surface_y + 1.5;
                    particle.position_state.w = 2.0;
                    // Puddle scores only describe the surface the drop landed on
                    // when that surface is the rain-blocking top (not a roof brush
                    // over a lower floor).
                    let on_top_surface = abs(field.y - field.x) < 8.0;
                    particle.misc.y = select(0.0, field.w, on_top_surface);
                    particle.misc.z = select(0.0, field.z, on_top_surface);
                    particle.velocity_age.x = 0.0;
                    particle.velocity_age.y = 0.0;
                    particle.velocity_age.z = 0.0;
                    particle.velocity_age.w = 0.0;
                } else {
                    particle = respawn(index, particle.misc.x, shared_wind_velocity);
                }
            }
        }
    } else {
        particle.velocity_age.w += dt;
        if (particle.velocity_age.w >= sim.wind_time.w) {
            particles[index] = respawn(index, particle.misc.x, shared_wind_velocity);
            return;
        }
    }

    particles[index] = particle;
}

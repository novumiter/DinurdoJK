struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos_time: vec4<f32>,
    clip_plane: vec4<f32>,
    render_flags: vec4<u32>,
    camera_forward: vec4<f32>,
};

struct SnowParticle {
    position_state: vec4<f32>,
    velocity_seed: vec4<f32>,
    misc: vec4<f32>,
};

struct SnowRenderUniform {
    appearance: vec4<f32>, // min size, max size, opacity, viewport height
    color: vec4<f32>,
    motion: vec4<f32>,     // reserved, near fade end, far fade start, far fade end
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<storage, read> particles: array<SnowParticle>;
@group(1) @binding(1) var<uniform> snow: SnowRenderUniform;

struct SnowOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) sparkle: f32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> SnowOut {
    var out: SnowOut;
    let particle = particles[instance_index];
    if (particle.position_state.w < 0.5) {
        out.position = vec4<f32>(2.5, 2.5, 0.0, 1.0);
        out.local = vec2<f32>(0.0);
        out.opacity = 0.0;
        out.sparkle = 0.0;
        return out;
    }

    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0)
    );
    let local = corners[vertex_index];
    let position = particle.position_state.xyz;
    let to_camera_raw = camera.camera_pos_time.xyz - position;
    let distance = max(length(to_camera_raw), 0.001);
    let to_camera = to_camera_raw / distance;

    var right = cross(vec3<f32>(0.0, 1.0, 0.0), to_camera);
    if (dot(right, right) < 1.0e-6) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    } else {
        right = normalize(right);
    }
    let up = normalize(cross(to_camera, right));

    // Tumble phase advances in the compute state, so `r_we freeze` freezes
    // rotation as well as translation instead of leaving a shader-time animation.
    let angle = particle.misc.w;
    let s = sin(angle);
    let c = cos(angle);
    let rotated = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);

    let focal = max(camera.view_proj[1][1], 0.1);
    let pixel_world = 2.0 * distance / (focal * max(snow.appearance.w, 1.0));
    let authored_size = mix(snow.appearance.x, snow.appearance.y, particle.misc.y);
    // Keep distant flakes at least ~1 pixel wide, but never inflate them enough
    // to become giant white cards. Opacity below compensates for this LOD floor.
    let size = min(max(authored_size, pixel_world * 1.05), authored_size * 2.0);
    let world = position + (right * rotated.x + up * rotated.y) * size;
    let clip = camera.view_proj * vec4<f32>(world, 1.0);
    if (clip.w <= 0.01) {
        out.position = vec4<f32>(2.5, 2.5, 0.0, 1.0);
        out.local = vec2<f32>(0.0);
        out.opacity = 0.0;
        out.sparkle = 0.0;
        return out;
    }

    let near_fade = smoothstep(18.0, snow.motion.y, distance);
    let far_fade = 1.0 - smoothstep(snow.motion.z, snow.motion.w, distance);
    let size_compensation = sqrt(clamp(authored_size / max(size, 0.001), 0.25, 1.0));
    out.position = clip;
    out.local = local;
    out.opacity = snow.appearance.z * near_fade * far_fade * size_compensation;
    out.sparkle = mix(0.78, 1.08, particle.misc.z);
    return out;
}

@fragment
fn fs_main(input: SnowOut) -> @location(0) vec4<f32> {
    let p = input.local;
    let radius = length(p);

    // Three crossed arms read as a flake nearby; at distance MSAA/pixel coverage
    // naturally collapses them into the soft bright speck snow should become.
    let d0 = abs(p.y);
    let d1 = abs(dot(p, vec2<f32>(0.8660254, 0.5)));
    let d2 = abs(dot(p, vec2<f32>(0.8660254, -0.5)));
    let arm_distance = min(d0, min(d1, d2));
    let aa = clamp(fwidth(arm_distance) * 1.5, 0.025, 0.16);
    let arms = 1.0 - smoothstep(0.075 - aa, 0.19 + aa, arm_distance);
    let radial = 1.0 - smoothstep(0.58, 1.0, radius);
    let core = 1.0 - smoothstep(0.0, 0.34, radius);
    let profile = max(arms * radial, core * 0.72);
    let alpha = input.opacity * profile;
    // Keep derivatives above the non-uniform discard for WGSL derivative-uniformity.
    if (radius > 1.0 || alpha < 0.004) { discard; }
    return vec4<f32>(snow.color.rgb * input.sparkle, alpha);
}

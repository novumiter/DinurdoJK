// One-time GPU synthesis of a cyclic, genuinely shape-changing caustic field.
// The output is a compact 8-layer RGBA8 array (32 phases packed into channels); no per-frame simulation or FFT.
@group(0) @binding(0) var frames: texture_storage_2d_array<rgba8unorm, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= 256u || gid.y >= 256u || gid.z >= 8u) { return; }
    let tau = 6.283185307179586;
    let p = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) * (tau / 256.0);
    var output = vec4<f32>(0.0);
    for (var channel = 0u; channel < 4u; channel += 1u) {
    let t = f32(gid.z * 4u + channel) * (tau / 32.0);
    // Periodic spatial warp evolving in both directions. The interference
    // contours buckle, divide and merge, instead of translating a fixed decal.
    let warp = vec2<f32>(
        sin(p.y * 3.0 + t) + 0.45 * sin(p.x * 5.0 - t * 2.0),
        cos(p.x * 4.0 - t) + 0.38 * sin(p.y * 7.0 + t * 3.0)
    ) * 0.16;
    let q = p + warp;
    let a = 0.67 * sin(q.x * 4.0 + q.y * 3.0 + t)
          + 0.56 * sin(q.x * 7.0 - q.y * 4.0 - t * 2.0)
          + 0.44 * cos(q.x * 2.0 + q.y * 8.0 + t * 3.0);
    let b = 0.72 * cos(q.x * 5.0 - q.y * 6.0 - t)
          + 0.48 * sin(q.x * 9.0 + q.y * 2.0 + t * 2.0);
    let web = exp2(-14.0 * a * a) * 0.86 + exp2(-24.0 * b * b) * 0.40;
    let brightness = clamp(web, 0.0, 1.0);
    output[channel] = brightness;
    }
    textureStore(frames, vec2<i32>(gid.xy), i32(gid.z), output);
}

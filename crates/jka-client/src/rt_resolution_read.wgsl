// Scalar, temporally rate-limited RT sun visibility cache.
struct RtSunShadowFlags {
    flags: vec4<u32>,
};
@group(3) @binding(15) var<uniform> rt_shadow_flags: RtSunShadowFlags;
@group(3) @binding(16) var rt_sun_shadow: texture_2d<f32>;

fn rt_cached_sun_visibility(input: VertexOut) -> f32 {
    if (rt_shadow_flags.flags.x == 0u || camera.render_flags.x != 0u) { return -1.0; }
    let dims = vec2<i32>(textureDimensions(rt_sun_shadow));
    let pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let cached = textureLoad(rt_sun_shadow, pixel, 0).xy;
    let expected = distance(input.world_position, camera.camera_pos_time.xyz);
    if (abs(cached.y - expected) > max(expected * 0.005, 0.5)) { return -1.0; }
    return cached.x;
}

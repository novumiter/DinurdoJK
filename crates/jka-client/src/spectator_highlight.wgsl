@group(0) @binding(0) var player_mask: texture_2d<f32>;
@group(0) @binding(1) var player_mask_sampler: sampler;

struct FullscreenOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let p = positions[vertex_index];
    var output: FullscreenOut;
    output.position = vec4<f32>(p, 0.0, 1.0);
    output.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return output;
}

fn stronger(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    if (b.a > a.a) {
        return b;
    }
    return a;
}

@fragment
fn fs_outline(input: FullscreenOut) -> @location(0) vec4<f32> {
    // The preceding visibility-erase pass leaves only player fragments hidden
    // by the final scene depth. Never tint that silhouette itself; expand it
    // outward into a narrow halo.
    let center = textureSample(player_mask, player_mask_sampler, input.uv);
    if (center.a > 0.02) {
        discard;
    }

    let dims_u = textureDimensions(player_mask);
    let dims = vec2<f32>(f32(dims_u.x), f32(dims_u.y));
    let texel = 1.0 / max(dims, vec2<f32>(1.0));
    // Four bilinear cardinal taps are enough for a soft ~2 px edge and keep
    // this fullscreen pass cheap enough for live spectator use.
    let r = 1.65;
    var best = textureSample(player_mask, player_mask_sampler, input.uv + vec2<f32>( r, 0.0) * texel);
    best = stronger(best, textureSample(player_mask, player_mask_sampler, input.uv + vec2<f32>(-r, 0.0) * texel));
    best = stronger(best, textureSample(player_mask, player_mask_sampler, input.uv + vec2<f32>(0.0,  r) * texel));
    best = stronger(best, textureSample(player_mask, player_mask_sampler, input.uv + vec2<f32>(0.0, -r) * texel));

    if (best.a <= 0.02) {
        discard;
    }

    // Linear filtering makes edge RGB effectively premultiplied by alpha.
    let color = best.rgb / max(best.a, 0.001);
    return vec4<f32>(color, min(best.a * 1.20, 0.96));
}

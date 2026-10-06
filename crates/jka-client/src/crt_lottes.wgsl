// Native WGSL port of Timothy Lottes' crt-lottes.glsl.
// Original work: PUBLIC DOMAIN, Timothy Lottes.
// Source: libretro/glsl-shaders/crt/shaders/crt-lottes.glsl
//
// This preserves the original default parameter values and filter structure:
// 11 source fetches for Tri() plus 31 for Bloom(). WGPU sRGB texture/view
// conversion replaces the GLSL shader's explicit sRGB <-> linear helpers.

struct CrtUniform {
    // xy = physical output size, zw = virtual source size.
    output_input_size: vec4<f32>,
    // Kept layout-compatible with the fast CRT uniform; unused here.
    filter_params: vec4<f32>,
    mask_corner: vec4<f32>,
};

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> crt: CrtUniform;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var out: VsOut;
    let p = positions[vertex_index];
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = p * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

// Upstream crt-lottes defaults.
const HARD_SCAN: f32 = -8.0;
const HARD_PIX: f32 = -3.0;
const WARP_X: f32 = 0.031;
const WARP_Y: f32 = 0.041;
const MASK_DARK: f32 = 0.5;
const MASK_LIGHT: f32 = 1.5;
const SHADOW_MASK: u32 = 3u;
const BRIGHT_BOOST: f32 = 1.0;
const HARD_BLOOM_PIX: f32 = -1.5;
const HARD_BLOOM_SCAN: f32 = -2.0;
const BLOOM_AMOUNT: f32 = 0.15;
const KERNEL_SHAPE: f32 = 2.0;

fn fetch_crt(pos_in: vec2<f32>, off: vec2<f32>) -> vec3<f32> {
    let source_size = crt.output_input_size.zw;
    let sample_uv = (floor(pos_in * source_size + off) + vec2<f32>(0.5)) / source_size;
    return textureSampleLevel(source_tex, source_sampler, sample_uv, 0.0).rgb * BRIGHT_BOOST;
}

fn dist_to_texel(pos_in: vec2<f32>) -> vec2<f32> {
    let p = pos_in * crt.output_input_size.zw;
    return -((p - floor(p)) - vec2<f32>(0.5));
}

fn gaus(pos: f32, scale: f32) -> f32 {
    return exp2(scale * pow(abs(pos), KERNEL_SHAPE));
}

fn horz3(pos: vec2<f32>, off: f32) -> vec3<f32> {
    let b = fetch_crt(pos, vec2<f32>(-1.0, off));
    let c = fetch_crt(pos, vec2<f32>( 0.0, off));
    let d = fetch_crt(pos, vec2<f32>( 1.0, off));
    let dst = dist_to_texel(pos).x;
    let wb = gaus(dst - 1.0, HARD_PIX);
    let wc = gaus(dst, HARD_PIX);
    let wd = gaus(dst + 1.0, HARD_PIX);
    return (b * wb + c * wc + d * wd) / (wb + wc + wd);
}

fn horz5(pos: vec2<f32>, off: f32) -> vec3<f32> {
    let a = fetch_crt(pos, vec2<f32>(-2.0, off));
    let b = fetch_crt(pos, vec2<f32>(-1.0, off));
    let c = fetch_crt(pos, vec2<f32>( 0.0, off));
    let d = fetch_crt(pos, vec2<f32>( 1.0, off));
    let e = fetch_crt(pos, vec2<f32>( 2.0, off));
    let dst = dist_to_texel(pos).x;
    let wa = gaus(dst - 2.0, HARD_PIX);
    let wb = gaus(dst - 1.0, HARD_PIX);
    let wc = gaus(dst, HARD_PIX);
    let wd = gaus(dst + 1.0, HARD_PIX);
    let we = gaus(dst + 2.0, HARD_PIX);
    return (a * wa + b * wb + c * wc + d * wd + e * we) / (wa + wb + wc + wd + we);
}

fn horz7(pos: vec2<f32>, off: f32) -> vec3<f32> {
    let a = fetch_crt(pos, vec2<f32>(-3.0, off));
    let b = fetch_crt(pos, vec2<f32>(-2.0, off));
    let c = fetch_crt(pos, vec2<f32>(-1.0, off));
    let d = fetch_crt(pos, vec2<f32>( 0.0, off));
    let e = fetch_crt(pos, vec2<f32>( 1.0, off));
    let f = fetch_crt(pos, vec2<f32>( 2.0, off));
    let g = fetch_crt(pos, vec2<f32>( 3.0, off));
    let dst = dist_to_texel(pos).x;
    let wa = gaus(dst - 3.0, HARD_BLOOM_PIX);
    let wb = gaus(dst - 2.0, HARD_BLOOM_PIX);
    let wc = gaus(dst - 1.0, HARD_BLOOM_PIX);
    let wd = gaus(dst, HARD_BLOOM_PIX);
    let we = gaus(dst + 1.0, HARD_BLOOM_PIX);
    let wf = gaus(dst + 2.0, HARD_BLOOM_PIX);
    let wg = gaus(dst + 3.0, HARD_BLOOM_PIX);
    return (a * wa + b * wb + c * wc + d * wd + e * we + f * wf + g * wg) /
        (wa + wb + wc + wd + we + wf + wg);
}

fn scan(pos: vec2<f32>, off: f32) -> f32 {
    return gaus(dist_to_texel(pos).y + off, HARD_SCAN);
}

fn bloom_scan(pos: vec2<f32>, off: f32) -> f32 {
    return gaus(dist_to_texel(pos).y + off, HARD_BLOOM_SCAN);
}

fn tri(pos: vec2<f32>) -> vec3<f32> {
    let a = horz3(pos, -1.0);
    let b = horz5(pos,  0.0);
    let c = horz3(pos,  1.0);
    return a * scan(pos, -1.0) + b * scan(pos, 0.0) + c * scan(pos, 1.0);
}

fn crt_bloom(pos: vec2<f32>) -> vec3<f32> {
    let a = horz5(pos, -2.0);
    let b = horz7(pos, -1.0);
    let c = horz7(pos,  0.0);
    let d = horz7(pos,  1.0);
    let e = horz5(pos,  2.0);
    return a * bloom_scan(pos, -2.0) +
           b * bloom_scan(pos, -1.0) +
           c * bloom_scan(pos,  0.0) +
           d * bloom_scan(pos,  1.0) +
           e * bloom_scan(pos,  2.0);
}

fn warp(pos_in: vec2<f32>) -> vec2<f32> {
    var pos = pos_in * 2.0 - vec2<f32>(1.0);
    pos = pos * vec2<f32>(
        1.0 + pos.y * pos.y * WARP_X,
        1.0 + pos.x * pos.x * WARP_Y,
    );
    return pos * 0.5 + vec2<f32>(0.5);
}

fn crt_mask(pos_in: vec2<f32>) -> vec3<f32> {
    var pos = pos_in;
    var mask = vec3<f32>(MASK_DARK);
    if SHADOW_MASK == 1u {
        var line = MASK_LIGHT;
        var odd = 0.0;
        if fract(pos.x * (1.0 / 6.0)) < 0.5 { odd = 1.0; }
        if fract((pos.y + odd) * 0.5) < 0.5 { line = MASK_DARK; }
        pos.x = fract(pos.x * (1.0 / 3.0));
        if pos.x < 0.333 { mask.r = MASK_LIGHT; }
        else if pos.x < 0.666 { mask.g = MASK_LIGHT; }
        else { mask.b = MASK_LIGHT; }
        mask = mask * line;
    } else if SHADOW_MASK == 2u {
        pos.x = fract(pos.x * (1.0 / 3.0));
        if pos.x < 0.333 { mask.r = MASK_LIGHT; }
        else if pos.x < 0.666 { mask.g = MASK_LIGHT; }
        else { mask.b = MASK_LIGHT; }
    } else if SHADOW_MASK == 3u {
        pos.x = pos.x + pos.y * 3.0;
        pos.x = fract(pos.x * (1.0 / 6.0));
        if pos.x < 0.333 { mask.r = MASK_LIGHT; }
        else if pos.x < 0.666 { mask.g = MASK_LIGHT; }
        else { mask.b = MASK_LIGHT; }
    } else if SHADOW_MASK == 4u {
        pos = floor(pos * vec2<f32>(1.0, 0.5));
        pos.x = pos.x + pos.y * 3.0;
        pos.x = fract(pos.x * (1.0 / 6.0));
        if pos.x < 0.333 { mask.r = MASK_LIGHT; }
        else if pos.x < 0.666 { mask.g = MASK_LIGHT; }
        else { mask.b = MASK_LIGHT; }
    } else {
        return vec3<f32>(1.0);
    }
    return mask;
}

@fragment
fn fs_lottes(in: VsOut) -> @location(0) vec4<f32> {
    let pos = warp(in.uv);
    // The source shader's intent is black outside the warped CRT face rather
    // than stretching edge texels through the curved border.
    if pos.x <= 0.0001 || pos.x >= 0.9999 || pos.y <= 0.0001 || pos.y >= 0.9999 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    var color = tri(pos);
    color = color + crt_bloom(pos) * BLOOM_AMOUNT;
    if SHADOW_MASK > 0u {
        color = color * crt_mask(in.position.xy * 1.000001);
    }
    return vec4<f32>(color, 1.0);
}

// Native WGSL port of Timothy Lottes' crt-lottes-fast.glsl / CRTS.
// Original work: PUBLIC DOMAIN / UNLICENSE, Timothy Lottes, 2018-01-20b.
// RetroArch adaptation: hunterk / libretro glsl-shaders.
//
// Porting notes for DinurdoJK:
// - The source filter explicitly expects linear input and produces linear output.
//   WGPU's sRGB texture/view conversion handles that for the swapchain format, so
//   the Shadertoy-only FromSrgb/ToSrgb helpers are intentionally not reproduced.
// - The original CRTS_2_TAP compile-time option is exposed as fs_fast; the normal
//   eight-fetch path is fs_quality. The math/order below follows CRTS.

struct CrtUniform {
    // xy = physical output size, zw = virtual low-resolution input size.
    output_input_size: vec4<f32>,
    // x = SCANLINE_THINNESS (0..1), y = SCAN_BLUR (1..6),
    // z = MASK_INTENSITY (0..1), w = CURVATURE.
    filter_params: vec4<f32>,
    // x = CORNER (0..11), y = MASK (0 none, 1 grille-lite, 2 grille, 3 shadow).
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

fn fetch_linear(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(source_tex, source_sampler, uv, 0.0).rgb;
}

fn crt_mask(pos_in: vec2<f32>, dark: f32, mask_kind: u32) -> vec3<f32> {
    var pos = pos_in;
    if mask_kind == 2u {
        var m = vec3<f32>(dark);
        let x = fract(pos.x * (1.0 / 3.0));
        if x < (1.0 / 3.0) {
            m.r = 1.0;
        } else if x < (2.0 / 3.0) {
            m.g = 1.0;
        } else {
            m.b = 1.0;
        }
        return m;
    }
    if mask_kind == 1u {
        var m = vec3<f32>(1.0);
        let x = fract(pos.x * (1.0 / 3.0));
        if x < (1.0 / 3.0) {
            m.r = dark;
        } else if x < (2.0 / 3.0) {
            m.g = dark;
        } else {
            m.b = dark;
        }
        return m;
    }
    if mask_kind == 0u {
        return vec3<f32>(1.0);
    }

    // CRTS_MASK_SHADOW. Keep the original 2.9999 offset that prevents exact
    // integer alignment from collapsing the staggered pattern.
    pos.x += pos.y * 2.9999;
    var m = vec3<f32>(dark);
    let x = fract(pos.x * (1.0 / 6.0));
    if x < (1.0 / 3.0) {
        m.r = 1.0;
    } else if x < (2.0 / 3.0) {
        m.g = 1.0;
    } else {
        m.b = 1.0;
    }
    return m;
}

fn crt_tone(thin: f32, mask_in: f32, mask_kind: u32) -> vec4<f32> {
    var mask = mask_in;
    if mask_kind == 0u {
        mask = 1.0;
    }
    if mask_kind == 1u {
        mask = 0.5 + mask * 0.5;
    }
    let contrast = 1.0;
    let saturation = 0.0;
    let mid_out = 0.18 / ((1.5 - thin) * (0.5 * mask + 0.5));
    let p_mid_in = pow(0.18, contrast);
    return vec4<f32>(
        contrast,
        ((-p_mid_in) + mid_out) / ((1.0 - p_mid_in) * mid_out),
        ((-p_mid_in) * mid_out + p_mid_in) / (mid_out * (-p_mid_in) + mid_out),
        contrast + saturation,
    );
}

struct FilterSetup {
    pos: vec2<f32>,
    y0: f32,
    scan_a: f32,
    scan_b: f32,
    vignette: f32,
};

fn filter_setup(ipos: vec2<f32>, input_size: vec2<f32>, output_size: vec2<f32>, thin: f32, curvature: f32, corner: f32) -> FilterSetup {
    // CRTS_WARP is enabled in crt-lottes-fast.glsl.
    var pos = ipos * (2.0 / output_size) - vec2<f32>(1.0);
    let warp = vec2<f32>(curvature, (3.0 / 4.0) * curvature);
    pos *= vec2<f32>(
        1.0 + (pos.y * pos.y) * warp.x,
        1.0 + (pos.x * pos.x) * warp.y,
    );
    var vin = (1.0 - ((1.0 - clamp(pos.x * pos.x, 0.0, 1.0)) *
                     (1.0 - clamp(pos.y * pos.y, 0.0, 1.0)))) *
              (0.998 + (0.001 * corner));
    vin = clamp((-vin) * input_size.y + input_size.y, 0.0, 1.0);
    pos = pos * (0.5 * input_size) + (0.5 * input_size);

    let y0 = floor(pos.y - 0.5) + 0.5;
    let off = pos.y - y0;
    let pi2 = 6.28318530717958;
    let scan_a = cos(min(0.5, off * thin) * pi2) * 0.5 + 0.5;
    let scan_b = cos(min(0.5, (-off) * thin + thin) * pi2) * 0.5 + 0.5;
    return FilterSetup(pos, y0, scan_a, scan_b, vin);
}

fn finish_crt(color_in: vec3<f32>, ipos: vec2<f32>, thin: f32, mask: f32, mask_kind: u32) -> vec3<f32> {
    var color = color_in * crt_mask(ipos, mask, mask_kind);
    let tone = crt_tone(thin, mask, mask_kind);
    let peak = max(1.0 / (256.0 * 65536.0), max(color.r, max(color.g, color.b)));
    let ratio = color / peak;
    let toned_peak = peak / (peak * tone.y + tone.z);
    return ratio * toned_peak;
}

fn crt_fast(ipos: vec2<f32>) -> vec3<f32> {
    let output_size = crt.output_input_size.xy;
    let input_size = crt.output_input_size.zw;
    let rcp_input = 1.0 / input_size;
    let thin = 0.5 + 0.5 * crt.filter_params.x;
    let mask = 1.0 - crt.filter_params.z;
    let mask_kind = u32(round(crt.mask_corner.y));
    let setup = filter_setup(ipos, input_size, output_size, thin, crt.filter_params.w, crt.mask_corner.x);

    // CRTS_2_TAP: Inigo Quilez' improved texture interpolation in X, then one
    // bilinear sample from each of the two nearest scanlines.
    var pos = setup.pos;
    pos.x += 0.5;
    let xi = floor(pos.x);
    var xf = pos.x - xi;
    xf = xf * xf * xf * (xf * (xf * 6.0 - 15.0) + 10.0);
    let x0 = xi + xf - 0.5;
    var p = vec2<f32>(x0 * rcp_input.x, setup.y0 * rcp_input.y);
    let col_a = fetch_linear(p);
    p.y += rcp_input.y;
    let col_b = fetch_linear(p);
    let color = (col_a * setup.scan_a + col_b * setup.scan_b) * setup.vignette;
    return finish_crt(color, ipos, thin, mask, mask_kind);
}

fn crt_quality(ipos: vec2<f32>) -> vec3<f32> {
    let output_size = crt.output_input_size.xy;
    let input_size = crt.output_input_size.zw;
    let rcp_input = 1.0 / input_size;
    let thin = 0.5 + 0.5 * crt.filter_params.x;
    let blur = -crt.filter_params.y;
    let mask = 1.0 - crt.filter_params.z;
    let mask_kind = u32(round(crt.mask_corner.y));
    let setup = filter_setup(ipos, input_size, output_size, thin, crt.filter_params.w, crt.mask_corner.x);

    let x0 = floor(setup.pos.x - 1.5) + 0.5;
    var p = vec2<f32>(x0 * rcp_input.x, setup.y0 * rcp_input.y);
    let col_a0 = fetch_linear(p);
    p.x += rcp_input.x;
    let col_a1 = fetch_linear(p);
    p.x += rcp_input.x;
    let col_a2 = fetch_linear(p);
    p.x += rcp_input.x;
    let col_a3 = fetch_linear(p);
    p.y += rcp_input.y;
    let col_b3 = fetch_linear(p);
    p.x -= rcp_input.x;
    let col_b2 = fetch_linear(p);
    p.x -= rcp_input.x;
    let col_b1 = fetch_linear(p);
    p.x -= rcp_input.x;
    let col_b0 = fetch_linear(p);

    let off0 = setup.pos.x - x0;
    let off1 = off0 - 1.0;
    let off2 = off0 - 2.0;
    let off3 = off0 - 3.0;
    let pix0 = exp2(blur * off0 * off0);
    let pix1 = exp2(blur * off1 * off1);
    let pix2 = exp2(blur * off2 * off2);
    let pix3 = exp2(blur * off3 * off3);
    let pix_t = setup.vignette / (pix0 + pix1 + pix2 + pix3);
    let scan_a = setup.scan_a * pix_t;
    let scan_b = setup.scan_b * pix_t;
    let color =
        (col_a0 * pix0 + col_a1 * pix1 + col_a2 * pix2 + col_a3 * pix3) * scan_a +
        (col_b0 * pix0 + col_b1 * pix1 + col_b2 * pix2 + col_b3 * pix3) * scan_b;
    return finish_crt(color, ipos, thin, mask, mask_kind);
}

@fragment
fn fs_fast(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(crt_fast(in.position.xy), 1.0);
}

@fragment
fn fs_quality(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(crt_quality(in.position.xy), 1.0);
}

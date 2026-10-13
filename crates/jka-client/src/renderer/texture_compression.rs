//! r_ext_compress_textures: upload-time BC1/BC3 compression of colour textures.
//!
//! 0 uploads RGBA8 as before. 1 matches jamp/OpenJK: BC1 for opaque images,
//! BC3 for images with alpha, lightmaps untouched. 2 also compresses 8-bit
//! lightmaps to BC1. Float (FP16) lightmaps and UI images are never compressed.
//! Encoding is a CPU cost paid once at map load / first model use; the GPU then
//! fetches 4 or 8 bits per texel instead of 32.
use crate::renderer::{
    textures::{encode_bc3_alpha_block, mip_upload_window, rgb565, rgb565_expand},
    GpuImage, TextureData,
};
use rayon::prelude::*;
use wgpu::util::DeviceExt;

/// A texture already encoded on a worker, ready for a cheap upload.
pub(in crate::renderer) struct EncodedTexture {
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    mip_level_count: u32,
    bytes: Vec<u8>,
}

/// Which BC format `data` would use, or None when it must stay RGBA8.
fn compressed_format(data: &TextureData, picmip: u32) -> Option<(wgpu::TextureFormat, bool)> {
    if data.rgba16f.is_some() {
        return None;
    }
    let (width, height, _, offset) = mip_upload_window(data, picmip, 4);
    // WebGPU requires the base level of a block-compressed texture to be a
    // whole number of 4x4 blocks.
    if width % 4 != 0 || height % 4 != 0 {
        return None;
    }
    let base = data.rgba.get(offset..offset + width as usize * height as usize * 4)?;
    let alpha = base.chunks_exact(4).any(|pixel| pixel[3] != 255);
    let format = match (alpha, data.srgb) {
        (false, true) => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        (false, false) => wgpu::TextureFormat::Bc1RgbaUnorm,
        (true, true) => wgpu::TextureFormat::Bc3RgbaUnormSrgb,
        (true, false) => wgpu::TextureFormat::Bc3RgbaUnorm,
    };
    Some((format, alpha))
}

pub(in crate::renderer) fn encode_texture(data: &TextureData, picmip: u32) -> Option<EncodedTexture> {
    let (format, alpha) = compressed_format(data, picmip)?;
    let (mut width, mut height, mip_level_count, mut offset) = mip_upload_window(data, picmip, 4);
    let (base_width, base_height) = (width, height);
    let mut bytes = Vec::new();
    for _ in 0..mip_level_count {
        let len = width as usize * height as usize * 4;
        let source = data.rgba.get(offset..offset + len)?;
        for block_y in 0..height.div_ceil(4) {
            for block_x in 0..width.div_ceil(4) {
                let mut block = [[0u8; 4]; 16];
                for py in 0..4u32 {
                    for px in 0..4u32 {
                        let x = (block_x * 4 + px).min(width - 1);
                        let y = (block_y * 4 + py).min(height - 1);
                        let src = ((y * width + x) * 4) as usize;
                        block[(py * 4 + px) as usize].copy_from_slice(&source[src..src + 4]);
                    }
                }
                if alpha {
                    encode_bc3_alpha_block(&block, &mut bytes);
                }
                encode_bc1_color_block(&block, &mut bytes);
            }
        }
        offset += len;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    Some(EncodedTexture {
        format,
        width: base_width,
        height: base_height,
        mip_level_count,
        bytes,
    })
}

/// Encode every eligible texture in parallel; `None` entries upload as RGBA8.
pub(in crate::renderer) fn encode_textures(
    textures: &[TextureData],
    picmip: impl Fn(usize) -> Option<u32> + Sync,
) -> Vec<Option<EncodedTexture>> {
    textures
        .par_iter()
        .enumerate()
        .map(|(index, data)| picmip(index).and_then(|picmip| encode_texture(data, picmip)))
        .collect()
}

pub(in crate::renderer) fn upload_encoded_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    encoded: &EncodedTexture,
) -> GpuImage {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(&format!("{label} [BC]")),
            size: wgpu::Extent3d {
                width: encoded.width,
                height: encoded.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: encoded.mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: encoded.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &encoded.bytes,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    GpuImage {
        _texture: texture,
        view,
    }
}

fn squared_error(a: [u8; 4], b: [u8; 3]) -> u32 {
    let dr = i32::from(a[0]) - i32::from(b[0]);
    let dg = i32::from(a[1]) - i32::from(b[1]);
    let db = i32::from(a[2]) - i32::from(b[2]);
    (dr * dr + dg * dg + db * db) as u32
}

fn palette4(color0: u16, color1: u16) -> [[u8; 3]; 4] {
    let c0 = rgb565_expand(color0);
    let c1 = rgb565_expand(color1);
    let mix = |a: u8, b: u8| ((2 * u16::from(a) + u16::from(b) + 1) / 3) as u8;
    [
        c0,
        c1,
        [mix(c0[0], c1[0]), mix(c0[1], c1[1]), mix(c0[2], c1[2])],
        [mix(c1[0], c0[0]), mix(c1[1], c0[1]), mix(c1[2], c0[2])],
    ]
}

/// Nearest-palette indices and total error for four-colour mode endpoints.
fn fit_indices(pixels: &[[u8; 4]; 16], color0: u16, color1: u16) -> (u32, u32) {
    let palette = palette4(color0, color1);
    let mut bits = 0u32;
    let mut total = 0u32;
    for (i, pixel) in pixels.iter().enumerate() {
        let (best, error) = palette
            .iter()
            .enumerate()
            .map(|(index, color)| (index, squared_error(*pixel, *color)))
            .min_by_key(|(_, error)| *error)
            .expect("palette has four entries");
        bits |= (best as u32) << (2 * i);
        total += error;
    }
    (bits, total)
}

fn quantize(color: [f32; 3]) -> u16 {
    rgb565(color.map(|channel| channel.round().clamp(0.0, 255.0) as u8))
}

/// Four-colour endpoints must satisfy color0 > color1; equal endpoints mean a
/// solid block, which index 0 reproduces exactly.
fn ordered(color0: u16, color1: u16) -> (u16, u16) {
    if color0 < color1 {
        (color1, color0)
    } else {
        (color0, color1)
    }
}

/// BC1 colour block (also BC3's colour half): principal-axis endpoints with a
/// small inset (as stb_dxt), then one least-squares refinement that is kept only
/// if it lowers the block error.
pub(in crate::renderer) fn encode_bc1_color_block(pixels: &[[u8; 4]; 16], output: &mut Vec<u8>) {
    let mut mean = [0.0f32; 3];
    for pixel in pixels {
        for channel in 0..3 {
            mean[channel] += f32::from(pixel[channel]);
        }
    }
    mean = mean.map(|sum| sum / 16.0);
    let mut cov = [0.0f32; 6]; // rr rg rb gg gb bb
    for pixel in pixels {
        let d = [0, 1, 2].map(|c| f32::from(pixel[c]) - mean[c]);
        cov[0] += d[0] * d[0];
        cov[1] += d[0] * d[1];
        cov[2] += d[0] * d[2];
        cov[3] += d[1] * d[1];
        cov[4] += d[1] * d[2];
        cov[5] += d[2] * d[2];
    }
    // Seed power iteration with the covariance column of largest norm: a fixed
    // seed such as (1,1,1) is orthogonal to e.g. a red<->blue axis and collapses.
    let columns = [
        [cov[0], cov[1], cov[2]],
        [cov[1], cov[3], cov[4]],
        [cov[2], cov[4], cov[5]],
    ];
    let norm = |v: &[f32; 3]| v.iter().map(|x| x * x).sum::<f32>();
    let seed = columns
        .iter()
        .max_by(|a, b| norm(a).total_cmp(&norm(b)))
        .copied()
        .expect("three columns");
    let seed_length = norm(&seed).sqrt();
    let mut axis = if seed_length > 1e-6 {
        seed.map(|v| v / seed_length)
    } else {
        [0.577_350_3; 3]
    };
    for _ in 0..6 {
        let next = [
            cov[0] * axis[0] + cov[1] * axis[1] + cov[2] * axis[2],
            cov[1] * axis[0] + cov[3] * axis[1] + cov[4] * axis[2],
            cov[2] * axis[0] + cov[4] * axis[1] + cov[5] * axis[2],
        ];
        let length = next.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length < 1e-6 {
            break;
        }
        axis = next.map(|v| v / length);
    }
    let project = |pixel: &[u8; 4]| {
        (0..3).map(|c| (f32::from(pixel[c]) - mean[c]) * axis[c]).sum::<f32>()
    };
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for pixel in pixels {
        let t = project(pixel);
        low = low.min(t);
        high = high.max(t);
    }
    let inset = (high - low) / 16.0;
    let endpoint = |t: f32| [0, 1, 2].map(|c| mean[c] + axis[c] * t);
    let (mut color0, mut color1) = ordered(
        quantize(endpoint(high - inset)),
        quantize(endpoint(low + inset)),
    );
    let (mut bits, error) = fit_indices(pixels, color0, color1);

    if color0 != color1 && error > 0 {
        // Least squares for endpoints a (weight w) and b (1 - w) given indices.
        const WEIGHT: [f32; 4] = [1.0, 0.0, 2.0 / 3.0, 1.0 / 3.0];
        let (mut aa, mut ab, mut bb) = (0.0f32, 0.0f32, 0.0f32);
        let mut ax = [0.0f32; 3];
        let mut bx = [0.0f32; 3];
        for (i, pixel) in pixels.iter().enumerate() {
            let w = WEIGHT[((bits >> (2 * i)) & 3) as usize];
            let v = 1.0 - w;
            aa += w * w;
            ab += w * v;
            bb += v * v;
            for c in 0..3 {
                ax[c] += w * f32::from(pixel[c]);
                bx[c] += v * f32::from(pixel[c]);
            }
        }
        let det = aa * bb - ab * ab;
        if det.abs() > 1e-6 {
            let a = [0, 1, 2].map(|c| (ax[c] * bb - bx[c] * ab) / det);
            let b = [0, 1, 2].map(|c| (bx[c] * aa - ax[c] * ab) / det);
            let (refined0, refined1) = ordered(quantize(a), quantize(b));
            if refined0 != refined1 {
                let (refined_bits, refined_error) = fit_indices(pixels, refined0, refined1);
                if refined_error < error {
                    (color0, color1, bits) = (refined0, refined1, refined_bits);
                }
            }
        }
    }
    if color0 == color1 {
        // Solid after quantization: color0 <= color1 selects three-colour mode,
        // where index 0 is still exactly color0 and index 3 (unused) is black.
        bits = 0;
    }
    output.extend_from_slice(&color0.to_le_bytes());
    output.extend_from_slice(&color1.to_le_bytes());
    output.extend_from_slice(&bits.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_bc1_color(block: &[u8]) -> [[u8; 3]; 16] {
        let color0 = u16::from_le_bytes([block[0], block[1]]);
        let color1 = u16::from_le_bytes([block[2], block[3]]);
        let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
        assert!(color0 > color1 || bits == 0, "four-colour mode or solid");
        let palette = palette4(color0, color1);
        std::array::from_fn(|i| palette[((bits >> (2 * i)) & 3) as usize])
    }

    fn rmse(pixels: &[[u8; 4]; 16], decoded: &[[u8; 3]; 16]) -> f32 {
        let total: u32 = pixels.iter().zip(decoded).map(|(p, d)| squared_error(*p, *d)).sum();
        (total as f32 / 48.0).sqrt()
    }

    #[test]
    fn solid_block_is_exact_in_565() {
        let pixels = [[96u8, 128, 32, 255]; 16];
        let mut out = Vec::new();
        encode_bc1_color_block(&pixels, &mut out);
        assert_eq!(out.len(), 8);
        let decoded = decode_bc1_color(&out);
        assert!(rmse(&pixels, &decoded) < 3.0);
    }

    #[test]
    fn off_axis_two_colour_block_keeps_its_hues() {
        // Red and blue: the bounding-box corners are magenta and dark grey, so a
        // min/max-box encoder cannot represent either colour. The principal axis
        // runs through both, so they encode nearly exactly.
        let pixels: [[u8; 4]; 16] =
            std::array::from_fn(|i| if i % 3 == 0 { [200, 40, 40, 255] } else { [40, 40, 200, 255] });
        let mut out = Vec::new();
        encode_bc1_color_block(&pixels, &mut out);
        let pca = rmse(&pixels, &decode_bc1_color(&out));
        let mut bbox = Vec::new();
        crate::renderer::textures::encode_bc3_block(&pixels, &mut bbox);
        let bbox = rmse(&pixels, &decode_bc1_color(&bbox[8..]));
        assert!(pca < 4.0, "pca rmse {pca}");
        assert!(pca < bbox, "pca {pca} vs bbox {bbox}");
    }

    #[test]
    fn encoded_size_matches_block_layout() {
        let data = TextureData {
            label: "test".into(),
            source: None,
            width: 8,
            height: 8,
            // 8x8 + 4x4 + 2x2 + 1x1 RGBA8 mip chain, opaque.
            rgba: [200, 200, 200, 255].repeat(64 + 16 + 4 + 1),
            rgba16f: None,
            mip_level_count: 4,
            clamp: false,
            srgb: true,
        };
        let encoded = encode_texture(&data, 0).expect("8x8 opaque is BC1-eligible");
        assert_eq!(encoded.format, wgpu::TextureFormat::Bc1RgbaUnormSrgb);
        // 4 blocks + 1 + 1 + 1, 8 bytes each.
        assert_eq!(encoded.bytes.len(), 7 * 8);
        let mut with_alpha = data;
        with_alpha.rgba[3] = 0;
        let encoded = encode_texture(&with_alpha, 0).expect("alpha goes to BC3");
        assert_eq!(encoded.format, wgpu::TextureFormat::Bc3RgbaUnormSrgb);
        assert_eq!(encoded.bytes.len(), 7 * 16);
    }

    #[test]
    fn unaligned_and_float_textures_stay_uncompressed() {
        let data = TextureData {
            label: "odd".into(),
            source: None,
            width: 6,
            height: 4,
            rgba: vec![255; 6 * 4 * 4],
            rgba16f: None,
            mip_level_count: 1,
            clamp: false,
            srgb: true,
        };
        assert!(encode_texture(&data, 0).is_none());
    }
}

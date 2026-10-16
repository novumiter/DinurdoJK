//! Pixel transfer for reusable frontend snapshots. All PNG/cache image bytes are
//! sRGB, just as egui's ColorImage uploads expect. The screenshot readback is
//! taken AFTER display gamma, so undo its gamma on the screenshot worker and
//! apply the *current* gamma once when decoding the PNG on the cache worker.
//! Never run this code from the gameplay frame loop.

#[inline]
fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

#[inline]
fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 { value * 12.92 } else { 1.055 * value.powf(1.0 / 2.4) - 0.055 }
}

/// `exponent` operates in the fragment shader's *linear* output values.
/// At capture: exponent = the captured gamma (undo `pow(linear, 1/gamma)`).
/// At load: exponent = 1/current gamma. For non-sRGB UNORM readbacks,
/// convert from linear UNORM to the sRGB byte encoding required by egui.
/// Premultiply/alpha is never touched: the menu screenshot is opaque.
pub(crate) fn transform_rgba(bytes: &mut [u8], exponent: f32, input_is_srgb: bool) {
    let exponent = if exponent.is_finite() { exponent.clamp(0.05, 8.0) } else { 1.0 };
    if (exponent - 1.0).abs() < 0.00001 && input_is_srgb {
        return;
    }
    let mut lookup = [0u8; 256];
    for (i, output) in lookup.iter_mut().enumerate() {
        let encoded = i as f32 / 255.0;
        let linear = if input_is_srgb { srgb_to_linear(encoded) } else { encoded };
        let adjusted = linear.powf(exponent);
        *output = (linear_to_srgb(adjusted.clamp(0.0, 1.0)) * 255.0).round() as u8;
    }
    for pixel in bytes.chunks_exact_mut(4) {
        pixel[0] = lookup[pixel[0] as usize];
        pixel[1] = lookup[pixel[1] as usize];
        pixel[2] = lookup[pixel[2] as usize];
    }
}

/// v7 cache metadata is written before the PNG is atomically published, so
/// readers never accept a partially created entry. File identity is also
/// verified against the current display/scene-settings hash in the filename.
pub(crate) fn cache_metadata_valid(path: &std::path::Path, neutral: bool) -> bool {
    let Ok(bytes) = std::fs::read(path.with_extension("json")) else { return false; };
    let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return false; };
    metadata["version"].as_u64() == Some(7)
        && metadata["neutral_gamma"].as_bool() == Some(neutral)
        && metadata["cache_file"].as_str() == path.file_name().and_then(|s| s.to_str())
        && metadata["transfer"].as_str() == Some("srgb8")
}

pub(crate) fn write_cache_metadata(
    path: &std::path::Path,
    neutral: bool,
    captured_output_gamma: f32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let metadata = serde_json::json!({
        "version": 7,
        "cache_file": path.file_name().and_then(|s| s.to_str()),
        "transfer": "srgb8",
        "neutral_gamma": neutral,
        "captured_output_gamma": captured_output_gamma,
        "width": width,
        "height": height,
        "note": "postprocessed PNG approximately gamma-neutralized for shader gamma when neutral_gamma=true; exact-settings cache otherwise",
    });
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(temp, path.with_extension("json")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_replay_tracks_the_output_gamma_curve() {
        for gamma in [0.5, 0.8, 1.0, 1.3, 2.0, 3.0] {
            let mut rgba = Vec::new();
            for value in (48..=240u8).step_by(8) { rgba.extend([value, value, value, 255]); }
            let mut expected = rgba.clone();
            transform_rgba(&mut expected, 1.0 / gamma, true); // true shader output
            transform_rgba(&mut rgba, 1.0 / gamma, true); // screenshot readback
            transform_rgba(&mut rgba, gamma, true); // neutral cache at capture
            transform_rgba(&mut rgba, 1.0 / gamma, true); // regrade at load
            // sRGB8 quantizes before the readback, so exact recovery at the
            // blackest levels is impossible. Midtones should match closely.
            for (actual, target) in rgba.iter().zip(expected.iter()) {
                assert!(actual.abs_diff(*target) <= 3, "gamma={gamma}: {actual} vs {target}");
            }
        }
    }

    #[test]
    fn neutral_srgb_png_keeps_original_at_gamma_one() {
        let mut rgba = vec![0, 45, 128, 255, 200, 250, 17, 255];
        let original = rgba.clone();
        transform_rgba(&mut rgba, 1.0, true);
        assert_eq!(rgba, original);
    }

    #[test]
    fn unorm_capture_converts_linear_bytes_to_srgb() {
        let mut rgba = [128, 128, 128, 255];
        transform_rgba(&mut rgba, 1.0, false);
        assert!(rgba[0] > 180 && rgba[0] < 191);
        assert_eq!(rgba[3], 255);
    }
}

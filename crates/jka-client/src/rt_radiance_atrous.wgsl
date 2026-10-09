// SVGF-inspired, variance-guided, depth/normal bilateral a-trous filter.
// Deliberately restricted to the separate RT direct-sun radiance signal.
struct FilterParams {
    dimensions: vec4<u32>, // width, height, step (1,2,4), reserved
};
@group(0) @binding(0) var<uniform> config: FilterParams;
@group(0) @binding(1) var source_light: texture_2d<f32>;
@group(0) @binding(2) var source_moments: texture_2d<f32>;
@group(0) @binding(3) var source_guide: texture_2d<f32>;
@group(0) @binding(4) var output_light: texture_storage_2d<rgba16float, write>;
fn luma(rgb: vec3<f32>) -> f32 { return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722)); }
@compute @workgroup_size(8, 8)
fn atrous(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= config.dimensions.xy)) { return; }
    let p = vec2<i32>(id.xy);
    let center = textureLoad(source_light, p, 0);
    let guide = textureLoad(source_guide, p, 0);
    if (guide.w <= 0.0) { textureStore(output_light, p, center); return; }
    let moment_data = textureLoad(source_moments, p, 0);
    let moment = moment_data.xy;
    let variance = max(moment.y - moment.x * moment.x, 0.0);
    let scale_l = max(0.02, sqrt(variance) * 3.0);
    var total = vec3<f32>(0.0);
    var weights = 0.0;
    let kernel = array<f32, 3>(1.0, 0.6666667, 0.16666667);
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let q = p + vec2<i32>(x, y) * i32(config.dimensions.z);
            if (any(q < vec2<i32>(0)) || any(q >= vec2<i32>(config.dimensions.xy))) { continue; }
            let g = textureLoad(source_guide, q, 0);
            if (g.w <= 0.0) { continue; }
            // Do not mix unrelated moving saber/FX lights across texels.
            if (abs(textureLoad(source_moments, q, 0).z - moment_data.z) > 0.25) { continue; }
            let n = max(dot(guide.xyz, g.xyz), 0.0);
            let dz = abs(guide.w - g.w);
            let d_scale = max(0.5, guide.w * 0.015 * f32(config.dimensions.z));
            let sample = textureLoad(source_light, q, 0);
            let kernel_w = kernel[u32(abs(x))] * kernel[u32(abs(y))];
            let normal_w = pow(n, 32.0);
            let depth_w = exp(-dz / d_scale);
            let light_w = exp(-abs(luma(center.rgb) - luma(sample.rgb)) / scale_l);
            let w = kernel_w * normal_w * depth_w * light_w;
            total += sample.rgb * w;
            weights += w;
        }
    }
    textureStore(output_light, p, vec4<f32>(total / max(weights, 1e-6), center.a));
}

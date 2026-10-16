struct HostCamera { view_projection: mat4x4<f32> }
@group(0) @binding(0) var<uniform> host: HostCamera;
@group(0) @binding(1) var sky_image: texture_2d<f32>;
@vertex fn sky_mask(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    var clip = host.view_projection * vec4<f32>(position, 1.0);
    clip.z = 0.0;
    return clip;
}
@fragment fn sky_color(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(sky_image, vec2<i32>(pixel.xy), 0);
}

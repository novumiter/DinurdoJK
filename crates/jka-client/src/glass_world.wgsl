struct Camera {
    view_proj: mat4x4<f32>, camera_pos_time: vec4<f32>, clip_plane: vec4<f32>,
    render_flags: vec4<u32>, camera_forward: vec4<f32>,
};
// Prefix of the existing MaterialUniform; numeric tint/F0 stay map-authored.
struct Material {
    header: vec4<u32>, vector_s: vec4<f32>, vector_t: vec4<f32>, mods: array<vec4<f32>,8>,
    color: vec4<f32>, params: vec4<f32>, pbr_params0: vec4<f32>, pbr_params1: vec4<f32>,
    reflection_probe: vec4<f32>, planar_plane: vec4<f32>, wave_rgb: vec4<f32>,
    wave_alpha: vec4<f32>, wave_funcs: vec4<u32>, ocean_color: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(4) var<uniform> material: Material;
struct VertexIn { @location(0) position: vec3<f32>, @location(5) normal: vec3<f32>, };
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>, @location(1) normal: vec3<f32>,
};
@vertex fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = camera.view_proj * vec4<f32>(input.position,1.0);
    out.world = input.position;
    out.normal = input.normal;
    return out;
}
@fragment fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return glass_shade(input.world,input.normal,input.position,camera.view_proj,
        camera.camera_pos_time.xyz,camera.camera_forward.xyz,camera.camera_pos_time.w,
        material.ocean_color.rgb,material.ocean_color.a);
}

@fragment fn fs_glass_mask(input: VertexOut) -> @location(0) vec4<f32> { return glass_mask_value(input.world,input.normal,camera.camera_pos_time.xyz,camera.camera_forward.xyz); }

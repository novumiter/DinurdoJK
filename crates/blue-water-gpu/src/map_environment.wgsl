// JKA map skyboxes baked into the original Blue Water CubeUV atlas.
#import naval_sky::environment::cube_uv_direction
@group(0) @binding(0) var sky_rt: texture_2d<f32>;
@group(0) @binding(1) var sky_bk: texture_2d<f32>;
@group(0) @binding(2) var sky_lf: texture_2d<f32>;
@group(0) @binding(3) var sky_ft: texture_2d<f32>;
@group(0) @binding(4) var sky_up: texture_2d<f32>;
@group(0) @binding(5) var sky_dn: texture_2d<f32>;
@group(0) @binding(6) var sky_sampler: sampler;
@group(1) @binding(0) var destination: texture_storage_2d<rgba16float, write>;
fn sky_uv(s: f32, t: f32) -> vec2<f32> {
    return clamp(vec2<f32>((s + 1.0) * 0.5, (1.0 - t) * 0.5), vec2<f32>(0.001), vec2<f32>(0.999));
}
fn face_color(image: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    let mip = max(log2(f32(textureDimensions(image).x) / 126.0), 0.0);
    return textureSampleLevel(image, sky_sampler, uv, mip).rgb;
}
fn map_sky(d: vec3<f32>) -> vec3<f32> {
    let a = abs(d);
    if a.x >= a.y && a.x >= a.z {
        if d.x >= 0.0 { return face_color(sky_rt, sky_uv(-d.y / a.x, d.z / a.x)); }
        return face_color(sky_lf, sky_uv(d.y / a.x, d.z / a.x));
    }
    if a.y >= a.x && a.y >= a.z {
        if d.y >= 0.0 { return face_color(sky_bk, sky_uv(d.x / a.y, d.z / a.y)); }
        return face_color(sky_ft, sky_uv(-d.x / a.y, d.z / a.y));
    }
    if d.z >= 0.0 { return face_color(sky_up, sky_uv(-d.y / a.z, -d.x / a.z)); }
    return face_color(sky_dn, sky_uv(-d.y / a.z, d.x / a.z));
}
@compute @workgroup_size(8, 8)
fn bake(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 384u || id.y >= 256u { return; }
    let face = f32(id.x / 128u + 3u * (id.y / 128u));
    let local = vec2<f32>(id.xy % vec2<u32>(128u));
    let direction = normalize(cube_uv_direction((local - 0.5) / 126.0, face));
    let color = map_sky(vec3<f32>(direction.x, -direction.z, direction.y));
    textureStore(destination, id.xy, vec4<f32>(color, dot(color, vec3<f32>(0.2126, 0.7152, 0.0722))));
}

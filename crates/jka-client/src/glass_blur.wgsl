// A Gaussian mip pyramid with premultiplied background validity. Foreground
// actors are excluded before blur, rather than creating a sharp fallback halo.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var scene_depth: texture_2d<f32>;
@group(0) @binding(3) var glass_coverage: texture_2d<f32>;
@group(0) @binding(4) var world_background: texture_2d<f32>;
// Matches the nine vec4 fields of GlassSettings. Recovery is highlights.w.
@group(0) @binding(5) var<uniform> glass_settings: array<vec4<f32>,9>;
struct VertexOut { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    let positions = array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    var out: VertexOut; out.position=vec4<f32>(positions[i],0.0,1.0);
    out.uv=positions[i]*vec2<f32>(0.5,-0.5)+0.5; return out;
}
fn coverage(uv: vec2<f32>) -> vec4<f32> {
    let dims=vec2<i32>(textureDimensions(glass_coverage));
    return textureLoad(glass_coverage,clamp(vec2<i32>(uv*vec2<f32>(dims)),vec2<i32>(0),dims-1),0);
}
fn valid_source(uv: vec2<f32>, first: bool, center_inside: bool) -> vec4<f32> {
    let mask=coverage(uv);
    if (center_inside && mask.w<0.01) { return vec4<f32>(0.0); }
    let color=textureSampleLevel(source,linear_sampler,uv,0.0);
    if (first) {
        let dims=vec2<i32>(textureDimensions(scene_depth));
        let depth=textureLoad(scene_depth,clamp(vec2<i32>(uv*vec2<f32>(dims)),vec2<i32>(0),dims-1),0).r;
        if (mask.w>0.01 && depth<=mask.z/max(mask.w,0.001)+0.5) {
            if (glass_settings[8].w>0.5) {
                let background=textureLoad(world_background,clamp(vec2<i32>(uv*vec2<f32>(dims)),vec2<i32>(0),dims-1),0);
                if (background.a>mask.z/max(mask.w,0.001)+0.5) { return vec4<f32>(background.rgb,1.0); }
            }
            return vec4<f32>(0.0);
        }
        let color_dims=vec2<i32>(textureDimensions(source));
        let exact=textureLoad(source,clamp(vec2<i32>(uv*vec2<f32>(color_dims)),vec2<i32>(0),color_dims-1),0);
        return vec4<f32>(exact.rgb,1.0);
    }
    return color;
}
fn gaussian(uv: vec2<f32>, first: bool) -> vec4<f32> {
    let step=1.0/vec2<f32>(textureDimensions(source));
    let inside=coverage(uv).w>0.01;
    var sum=vec4<f32>(0.0);
    for (var y=-1;y<=1;y+=1) { for (var x=-1;x<=1;x+=1) {
        let weight=select(1.0,2.0,x==0)*select(1.0,2.0,y==0);
        sum+=valid_source(clamp(uv+vec2<f32>(f32(x),f32(y))*step,step*0.5,1.0-step*0.5),first,inside)*weight;
    } }
    return sum/16.0;
}
@fragment fn fs_first(input: VertexOut) -> @location(0) vec4<f32> { return gaussian(input.uv,true); }
@fragment fn fs_next(input: VertexOut) -> @location(0) vec4<f32> { return gaussian(input.uv,false); }

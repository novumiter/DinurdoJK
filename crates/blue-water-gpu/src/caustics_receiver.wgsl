// JKA receiver bridge for the unchanged upstream four-depth caustic field.
#import naval::camera::camera
#import naval_ocean::waves::height_at
#import naval_ocean::underwater_ray::diffuse_attenuation
struct ReceiverParams {
    sun: vec4<f32>, // toward active light, user gain
    light: vec4<f32>, // linear color, active intensity
    plane: vec4<f32>, // native map plane, volume count, native units/metre, camera fade
    absorption: vec4<f32>,
    minimum: array<vec4<f32>,8>,
    maximum: array<vec4<f32>,8>,
}
@group(1) @binding(0) var<uniform> receiver: ReceiverParams;
@group(1) @binding(3) var scene_color: texture_2d<f32>;
@group(1) @binding(4) var scene_depth: texture_depth_2d;
@group(1) @binding(17) var caustic_field: texture_2d_array<f32>;
@group(1) @binding(18) var caustic_sampler: sampler;

@vertex fn receiver_vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}
fn in_water(world:vec3<f32>)->bool {
    if !jka_water_contains(world.xz) { return false; }
    let native=world*receiver.plane.z+vec3<f32>(0.0,receiver.plane.x,0.0);
    for(var i=0u;i<u32(receiver.plane.y);i+=1u) {
        if all(native>=receiver.minimum[i].xyz-vec3<f32>(0.75)) &&
           all(native<=receiver.maximum[i].xyz+vec3<f32>(0.75)) {
            return abs(receiver.maximum[i].y-receiver.plane.x)<=0.75;
        }
    }
    // Only qualified exterior sky strips continue below the sea outside finite volumes.
    let bounds=jka_boundary.bounds;
    return jka_boundary.counts.y>0u && native.y<receiver.plane.x &&
        (any(native.xz<bounds.xy)||any(native.xz>bounds.zw));
}
fn caustic_gain(world:vec3<f32>,depth:f32)->f32 {
    // Upstream caustics::{level_of,span}: 2,5,12.5,31.25m and 32,64,128,256m.
    let level=clamp(log(max(depth,2.0)/2.0)/log(2.5),0.0,3.0);
    let lo=i32(floor(level));let hi=min(lo+1,3);
    let a=textureSampleLevel(caustic_field,caustic_sampler,world.xz/(32.0*exp2(f32(lo))),lo,0.0).r;
    let b=textureSampleLevel(caustic_field,caustic_sampler,world.xz/(32.0*exp2(f32(hi))),hi,0.0).r;
    return mix(a,b,fract(level));
}
@fragment fn receiver_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let p=vec2<i32>(pixel.xy);
    let depth=textureLoad(scene_depth,p,0);
    let uv=pixel.xy*camera.viewport.zw;
    let clip=camera.inverse_view_projection*vec4<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0,depth,1.0);
    var world=clip.xyz/clip.w;
    let across=world.xz-camera.position.xz;
    world.y+=camera.curvature*dot(across,across);
    // Derivatives precede the admission branch.
    let dx=dpdx(world);let dy=dpdy(world);
    if depth<=0.0 || receiver.sun.w<=0.0 || !in_water(world) { discard; }
    let below=max(height_at(world.xz)-world.y,0.0);
    if below<=0.02 {discard;}
    let n=cross(dx,dy);let nlength=length(n);
    if nlength<1e-7 {discard;}
    let normal=n/nlength;
    let sun=normalize(receiver.sun.xyz);
    let daylight=smoothstep(0.02,0.15,sun.y)*clamp(receiver.light.w/5.0,0.0,1.0);
    let pixel_span=max(length(dx),length(dy));
    let expected=max(distance(world,camera.position)*4.0/min(camera.viewport.x,camera.viewport.y),0.01);
    let edge=1.0-smoothstep(expected*4.0,expected*12.0,pixel_span);
    let down=exp(-diffuse_attenuation(receiver.absorption.rgb)*below/max(sun.y,0.2));
    // JKA retains baked lighting; modulate the existing lit receiver instead of adding light through shadows.
    let direct=down*(daylight*abs(dot(normal,sun))*receiver.sun.w*receiver.plane.w*edge);
    let multiplier=max(vec3<f32>(0.05),1.0+(caustic_gain(world,below)-1.0)*direct);
    let color=textureLoad(scene_color,p,0);
    return vec4<f32>(color.rgb*multiplier,color.a);
}

struct HostAir { inverse_view_projection: mat4x4<f32>, eye: vec4<f32>, viewport: vec4<f32> }
@group(0) @binding(0) var<uniform> host: HostAir;
@group(0) @binding(1) var scene_color: texture_2d<f32>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var air_volume: texture_3d<f32>;
@group(0) @binding(4) var air_sampler: sampler;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
 let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
 return vec4<f32>(p[i],0.0,1.0);
}
@fragment fn fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
 let p=vec2<i32>(pixel.xy);
 let color=textureLoad(scene_color,p,0);
 let depth=textureLoad(scene_depth,p,0);
 if depth<=0.0 {return color;}
 let uv=pixel.xy*host.viewport.zw;
 let point=host.inverse_view_projection*vec4<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0,depth,1.0);
 let world=point.xyz/point.w;
 let air=aerial_sample(air_volume,air_sampler,uv,distance(world,host.eye.xyz));
 return vec4<f32>(color.rgb*air.transmittance+air.inscatter,color.a);
}

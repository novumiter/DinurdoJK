// Ocean mesh LOD displacement snapshots. Frequency cascades are NOT changed.
// Output is periodic in FFT UV space, so coarse mesh vertices can sample with
// the same world-space UV as the live displacement array.
@group(0) @binding(0) var fft_displacement: texture_2d_array<f32>;
@group(0) @binding(1) var fft_sampler: sampler;
@group(0) @binding(2) var snapshot: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let extent=textureDimensions(snapshot);
    if (gid.x >= extent.x || gid.y >= extent.y || gid.z >= 3u) {return;}
    let uv=(vec2<f32>(gid.xy)+vec2<f32>(0.5))/vec2<f32>(extent);
    let displaced=textureSampleLevel(fft_displacement,fft_sampler,uv,i32(gid.z),0.0);
    textureStore(snapshot,vec2<i32>(gid.xy),i32(gid.z),displaced);
}

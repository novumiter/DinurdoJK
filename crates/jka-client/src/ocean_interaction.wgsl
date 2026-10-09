// Height/velocity wave propagation adapted from jeantimex/webgpu-water
// update.frag.wgsl (MIT). World reprojection, Courant clamp, signed interaction
// falloff and edge sponge follow Crest (MIT). See THIRD_PARTY.md.
struct Input { position: vec4<f32>, motion: vec4<f32> };
struct Params {
    region: vec4<f32>, source: vec4<f32>, bounds: vec4<f32>, indices: vec4<u32>,
    tuning: vec4<f32>,
    inputs: array<Input, 16>,
};
@group(0) @binding(0) var previous: texture_2d_array<f32>;
@group(0) @binding(1) var next: texture_storage_2d_array<rg32float, write>;
@group(0) @binding(2) var published: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(3) var<storage, read_write> wet: array<u32>;
@group(0) @binding(4) var<storage, read> triangles: array<vec4<f32>>;
@group(0) @binding(5) var<uniform> p: Params;

fn index(cell: vec2<i32>) -> u32 { return p.indices.x * 65536u + u32(cell.y) * 256u + u32(cell.x); }
fn inside(cell: vec2<i32>) -> bool { return all(cell >= vec2<i32>(0)) && all(cell < vec2<i32>(256)); }
fn is_wet(cell: vec2<i32>) -> bool {
    if (!inside(cell)) { return false; }
    return wet[index(cell)] != 0u;
}
fn outline(world: vec2<f32>) -> bool {
    if (any(world < p.bounds.xy) || any(world > p.bounds.zw)) { return false; }
    if (p.indices.w == 0u) { return true; }
    for (var i = 0u; i < p.indices.w; i++) {
        let t = triangles[(p.indices.z + i) * 2u];
        let a = t.xy; let b = t.zw; let c = triangles[(p.indices.z + i) * 2u + 1u].xy;
        if (any(world < min(min(a,b),c) - vec2<f32>(0.75)) || any(world > max(max(a,b),c) + vec2<f32>(0.75))) { continue; }
        let ab = b-a; let bc = c-b; let ca = a-c;
        let e = vec3<f32>((ab.x*(world.y-a.y)-ab.y*(world.x-a.x))/max(length(ab),0.0001),
            (bc.x*(world.y-b.y)-bc.y*(world.x-b.x))/max(length(bc),0.0001),
            (ca.x*(world.y-c.y)-ca.y*(world.x-c.x))/max(length(ca),0.0001));
        if (all(e >= vec3<f32>(-0.75)) || all(e <= vec3<f32>(0.75))) { return true; }
    }
    return false;
}
@compute @workgroup_size(8,8)
fn coverage(@builtin(global_invocation_id) id: vec3<u32>) {
    let cell = vec2<i32>(id.xy);
    let world = p.region.xy + (vec2<f32>(id.xy) + vec2<f32>(0.5)) * p.region.w;
    wet[index(cell)] = select(0u, 1u, outline(world));
}
fn source_state(cell: vec2<i32>) -> vec2<f32> {
    if (p.source.z > 0.5 || !is_wet(cell)) { return vec2<f32>(0.0); }
    let shift = vec2<i32>(round((p.region.xy - p.source.xy) / p.region.w));
    let source_cell = cell + shift;
    if (!inside(source_cell)) { return vec2<f32>(0.0); }
    return textureLoad(previous, source_cell, i32(p.indices.x), 0).xy;
}
fn falloff(a: f32, x: f32) -> f32 {
    let ax = a*x; let ax2 = ax*ax;
    return ax / (1.0 + ax2*ax2*ax2);
}
@compute @workgroup_size(8,8)
fn step(@builtin(global_invocation_id) id: vec3<u32>) {
    let cell = vec2<i32>(id.xy); let layer = i32(p.indices.x);
    if (!is_wet(cell)) { textureStore(next, cell, layer, vec4<f32>(0.0)); return; }
    let state = source_state(cell); let h = state.x;
    let dt = p.source.w; let dx = p.region.w;
    // Nine-point isotropic refinement to the reference four-neighbour stencil.
    let axial = source_state(cell+vec2<i32>(1,0)).x + source_state(cell+vec2<i32>(-1,0)).x
        + source_state(cell+vec2<i32>(0,1)).x + source_state(cell+vec2<i32>(0,-1)).x;
    let diagonal = source_state(cell+vec2<i32>(1,1)).x + source_state(cell+vec2<i32>(-1,1)).x
        + source_state(cell+vec2<i32>(1,-1)).x + source_state(cell+vec2<i32>(-1,-1)).x;
    let c = min(80.0, 0.65 * dx / max(dt,0.0001));
    var velocity = state.y + dt*c*c/(dx*dx) * (4.0*axial + diagonal - 20.0*h)/6.0;
    let world = p.region.xy + (vec2<f32>(id.xy)+vec2<f32>(0.5))*dx;
    for (var i = 0u; i < p.indices.y; i++) {
        let input = p.inputs[i]; let offset = world-input.position.xy;
        if (input.motion.w > 0.5) {
            // Normal momentum drives a depression/opposing annulus; tangential
            // momentum drives a directional dipole. These Gaussian derivative
            // bases have equal integrated squared amplitude before truncation.
            // A one-shot velocity jump has no render/tick-duration multiplier.
            let sigma=max(input.position.z*0.4,dx);
            let r2=dot(offset,offset)/(sigma*sigma);
            if (r2 < 12.25) {
                let normal=input.motion.z*(1.0-0.5*r2);
                let tangential=dot(offset/sigma,input.motion.xy);
                velocity += input.position.w*(normal+tangential)*exp(-0.5*r2);
            }
            continue;
        }
        let distance = length(offset); let signed_distance = distance-input.position.z;
        if (abs(signed_distance) > 3.5*dx) { continue; }
        let normal = offset/max(distance,0.0001);
        var horizontal = sign(signed_distance) * dot(normal,input.motion.xy)
            * falloff(1.43/(2.0*dx),abs(signed_distance));
        if (signed_distance < 0.0) { horizontal *= 1.55; }
        let vertical = -input.motion.z * falloff(1.67/(2.0*dx),signed_distance);
        velocity += dt*input.position.w*0.65*(horizontal+vertical)*1.65;
    }
    velocity *= exp(-p.tuning.x*dt);
    var height = (h+dt*velocity)*exp(-p.tuning.y*dt);
    let uv = (vec2<f32>(id.xy)+vec2<f32>(0.5))/256.0;
    let edge = min(min(uv.x,uv.y),min(1.0-uv.x,1.0-uv.y));
    var sponge = 1.0;
    if (p.tuning.z > 0.0) {
        sponge = pow(mix(0.80,1.0,smoothstep(0.0,p.tuning.z*1.5,edge)),dt*48.0);
    }
    height *= sponge; velocity *= sponge;
    // This first visual field deliberately has a short, bounded lifetime.
    if (!(abs(height) < 1e10) || !(abs(velocity) < 1e10)) { height=0.0; velocity=0.0; }
    textureStore(next,cell,layer,vec4<f32>(height,velocity,0.0,0.0));
}
fn final_height(cell: vec2<i32>) -> f32 {
    if (!is_wet(cell)) { return 0.0; }
    return textureLoad(previous,cell,i32(p.indices.x),0).x;
}
fn faded_height(cell: vec2<i32>) -> f32 {
    if (!inside(cell)) { return 0.0; }
    let uv=(vec2<f32>(cell)+vec2<f32>(0.5))/256.0;
    let edge=min(min(uv.x,uv.y),min(1.0-uv.x,1.0-uv.y));
    if (p.tuning.z <= 0.0) { return final_height(cell); }
    return final_height(cell)*smoothstep(0.0,p.tuning.z,edge);
}
@compute @workgroup_size(8,8)
fn publish(@builtin(global_invocation_id) id: vec3<u32>) {
    let cell=vec2<i32>(id.xy);
    let slope=vec2<f32>(faded_height(cell+vec2<i32>(1,0))-faded_height(cell+vec2<i32>(-1,0)),
        faded_height(cell+vec2<i32>(0,1))-faded_height(cell+vec2<i32>(0,-1)))/(2.0*p.region.w);
    // Preserve the finite representable range of the published half floats.
    textureStore(published,cell,i32(p.indices.x),vec4<f32>(clamp(vec3<f32>(faded_height(cell),slope),vec3<f32>(-65504.0),vec3<f32>(65504.0)),0.0));
}

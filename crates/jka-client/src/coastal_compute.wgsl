// Conservative Saltreach stage ordering adapted to sparse JKA tile neighbours.
// MIT provenance: crates/jka-water/LICENSE-Saltreach-MIT and THIRD_PARTY.md.
struct OceanRenderSettings {
    map_scales: array<vec4<f32>, 3>, // xy inverse tile length, z displacement, w normal scale
    water_color: vec4<f32>,
    foam_color: vec4<f32>,
    ocean_info: vec4<f32>, // enabled, map size, JKA units per meter, windrow domain metres
    surface: vec4<f32>, // roughness, normal strength, sea spray enabled, wind foam streaks enabled
    clipmap: vec4<f32>, // centre x, centre z, coarsest cell size, active windrow layer
    sun_direction_intensity: vec4<f32>, // xyz light travel direction, w relative q3map intensity
    sun_color: vec4<f32>,
    sky_ambient: vec4<f32>, // rgb linear map-sky average, w real skybox present
    spray_info: vec4<f32>, // surface count, total particles, visible lifetime, lifetime randomness
    spray_bounds: array<vec4<f32>, 8>,
    spray_planes: array<vec4<f32>, 8>,
    swell: vec4<f32>,
    swell_motion: vec4<f32>,
    wind_motion: vec4<f32>,
    authored_bounds: vec4<f32>,
    authored_plane: vec4<f32>,
    // Outline of each promoted surface (see OceanMasks): per slot, first
    // triangle and triangle count; per triangle, two vec4s a.xy b.xy / c.xy.
    mask_ranges: array<vec4<f32>, 8>,
    mask_bounds: array<vec4<f32>, 8>,
    mask_tiles: array<vec4<u32>, 8>,
    mask_triangles: array<vec4<f32>, 512>,
};
struct Params { count:u32, dt:f32, dx:f32, depth_bound:f32 };
struct Cell { neighbours:vec4<u32>, metric:vec4<f32>, position:vec4<f32> };
struct Presentation { previous:vec4<f32>, current:vec4<f32>, slopes:vec4<f32>, accounting:vec4<f32> };
struct Surface { failed:atomic<u32>, pad0:u32, pad1:u32, pad2:u32, cells:array<Presentation> };
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var<storage,read> cells:array<Cell>;
@group(0) @binding(2) var<storage,read> depth:array<f32>;
@group(0) @binding(3) var<storage,read_write> next_depth:array<f32>;
@group(0) @binding(4) var<storage,read_write> state:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> limiter:array<f32>;
@group(0) @binding(6) var<storage,read_write> limited:array<vec2<f32>>;
@group(0) @binding(7) var<storage,read_write> presentation:Surface;
@group(0) @binding(8) var fft:texture_2d_array<f32>;
@group(0) @binding(9) var fft_sampler:sampler;
@group(0) @binding(10) var<uniform> ocean:OceanRenderSettings;
@group(0) @binding(11) var<storage,read> active_cells:array<u32>;
const NONE=0xffffffffu;
fn eta(k:u32)->f32 { return cells[k].metric.x+depth[k]; }
fn face(k:u32,q:u32,old:f32,crest:f32)->vec2<f32> {
    if (q==NONE) { return vec2<f32>(0.0); }
    let e=eta(k); let e2=eta(q);
    let top=max(crest,max(cells[k].metric.x,cells[q].metric.x));
    let h=max(0.0,max(e,e2)-top);
    if (h<0.001) { return vec2<f32>(0.0); }
    let drag=0.065+0.08/(h+0.075);
    let velocity=clamp((old-9.81*params.dt*(e2-e)/params.dx)/(1.0+params.dt*drag),-5.0,5.0);
    let donor=max(0.0,select(e2,e,velocity>0.0)-top);
    var flux=velocity*donor;
    if (depth[k]>0.008 && depth[q]>0.008) {
        let slope=(e2-e)/params.dx;
        flux-=0.32*smoothstep(0.12,0.48,abs(slope))*slope;
    }
    return vec2<f32>(velocity,flux);
}
@compute @workgroup_size(64)
fn pressure(@builtin(global_invocation_id) id:vec3<u32>) {
    if (id.x>=params.count) {return;} let k=active_cells[id.x];
    if (atomicLoad(&presentation.failed)!=0u) {state[k]=vec4<f32>(0.0);return;}
    if (cells[k].position.z<0.5) { state[k]=vec4<f32>(0.0);return; }
    let x=face(k,cells[k].neighbours.y,state[k].x,cells[k].metric.y);
    let y=face(k,cells[k].neighbours.w,state[k].y,cells[k].metric.z);
    state[k]=vec4<f32>(x.x,y.x,x.y,y.y);
}
fn raw_incoming(q:u32,axis:u32)->f32 {
    if (q==NONE) { return 0.0; }
    return state[q][axis+2u];
}
@compute @workgroup_size(64)
fn limit_outgoing(@builtin(global_invocation_id) id:vec3<u32>) {
    if (id.x>=params.count) {return;} let k=active_cells[id.x];
    let outgoing=(max(state[k].z,0.0)+max(-raw_incoming(cells[k].neighbours.x,0u),0.0)
        +max(state[k].w,0.0)+max(-raw_incoming(cells[k].neighbours.z,1u),0.0))/params.dx;
    limiter[k]=1.0;
    if (outgoing>0.0) {limiter[k]=min(1.0,depth[k]/(params.dt*outgoing+1e-10));}
}
@compute @workgroup_size(64)
fn limit_faces(@builtin(global_invocation_id) id:vec3<u32>) {
    if (id.x>=params.count) {return;} let k=active_cells[id.x];
    var result=vec2<f32>(0.0);
    for (var axis=0u;axis<2u;axis++) {
        let q=cells[k].neighbours[axis*2u+1u];
        if (q!=NONE) {
            let flux=state[k][axis+2u];
            let donor=select(q,k,flux>=0.0);
            result[axis]=flux*limiter[donor];
        }
    }
    limited[k]=result;
}
fn incoming(q:u32,axis:u32)->f32 {
    if (q==NONE) {return 0.0;}
    return limited[q][axis];
}
fn fft_height(position:vec2<f32>)->f32 {
    let world_m=position/max(ocean.ocean_info.z,0.001);
    var h=0.0;
    for (var i=0u;i<3u;i++) {
        let scale=ocean.map_scales[i];
        h+=textureSampleLevel(fft,fft_sampler,world_m*scale.xy,i32(i),0.0).y*scale.z;
    }
    // Match BSP's renderer XZ phase (JKA y is negative render z).
    let render_m=world_m*vec2<f32>(1.0,-1.0);
    let phase=dot(render_m,ocean.swell.xy)*ocean.swell.z-ocean.swell_motion.x*ocean.swell_motion.w;
    return h+ocean.swell.w*sin(phase);
}
@compute @workgroup_size(64)
fn divergence(@builtin(global_invocation_id) id:vec3<u32>) {
    if (id.x>=params.count) {return;} let k=active_cells[id.x];
    if (atomicLoad(&presentation.failed)!=0u) {next_depth[k]=depth[k];return;}
    if (cells[k].position.z<0.5) {next_depth[k]=0.0;return;}
    let change=(limited[k].x-incoming(cells[k].neighbours.x,0u)
        +limited[k].y-incoming(cells[k].neighbours.z,1u))/params.dx;
    var h=max(0.0,depth[k]-params.dt*change);
    let unforced=h;
    if (cells[k].metric.w>0.0) {
        let forced_depth=max(0.0,fft_height(cells[k].position.xy)-cells[k].metric.x);
        let response=1.0-exp(-cells[k].metric.w*params.dt);
        h=mix(h,forced_depth,response);
        state[k]=vec4<f32>(state[k].xy*(1.0-response),state[k].zw);
    }
    // Latch the whole pool back to FFT if the configured CFL envelope is
    // exceeded. No silent depth clamp or NaN propagation through neighbours.
    if (!(h>=0.0 && h<=params.depth_bound)) {
        atomicStore(&presentation.failed,1u);next_depth[k]=depth[k];return;
    }
    presentation.cells[k].accounting.x+= (h-unforced)*params.dx*params.dx;
    next_depth[k]=h;
}
@compute @workgroup_size(64)
fn reconstruct(@builtin(global_invocation_id) id:vec3<u32>) {
    if (id.x>=params.count) {return;} let k=active_cells[id.x];
    let c=cells[k]; let h=depth[k];
    var elevation=c.metric.x+h;
    // Wet-neighbour reconstruction follows only physically connected faces.
    if (h<0.0005 && c.position.z>0.5) {
        var best=1e20; var neighbour_level=c.metric.x-0.025;
        for (var axis=0u;axis<4u;axis++) {
            let q=c.neighbours[axis];
            if (q!=NONE && depth[q]>=0.0005) {
                let level=eta(q);let score=abs(level-c.metric.x);
                if (score<best) {best=score;neighbour_level=level;}
            }
        }
        let dry=min(c.metric.x-0.0005,neighbour_level);
        elevation=mix(dry,elevation,pow(h/0.0005,2.0));
    }
    var weight=c.position.w;
    // A configured conservative CFL bound is checked on-GPU. Cells outside
    // the bound fall back visually; no depth clamp silently loses water.
    if (!(h>=0.0 && h<=params.depth_bound && abs(elevation)<1e10)) {weight=0.0;}
    presentation.cells[k].previous=presentation.cells[k].current;
    presentation.cells[k].current=vec4<f32>(elevation*40.0,max(0.0,elevation-c.metric.x)*40.0,c.metric.x*40.0,weight);
    var slope=vec2<f32>(0.0);
    for (var axis=0u;axis<2u;axis++) {
        let a=c.neighbours[axis*2u];let b=c.neighbours[axis*2u+1u];
        var low=elevation;var high=elevation;var span=0.0;
        if (a!=NONE && depth[a]>=0.0005 && min(elevation,eta(a))>max(c.metric.x,cells[a].metric.x)) {low=eta(a);span+=params.dx;}
        if (b!=NONE && depth[b]>=0.0005 && min(elevation,eta(b))>max(c.metric.x,cells[b].metric.x)) {high=eta(b);span+=params.dx;}
        if (span>0.0) {slope[axis]=(high-low)/span;}
    }
    presentation.cells[k].slopes=vec4<f32>(presentation.cells[k].slopes.zw,slope*vec2<f32>(1.0,-1.0));
    presentation.cells[k].accounting.y=h;
    presentation.cells[k].accounting.z=length(state[k].xy);
}


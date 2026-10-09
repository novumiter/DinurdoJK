// RBSP flare CENTER test against the same-frame opaque/dynamic scene depth.
// CPU never reads these results. Reversed-Z: larger device depth is nearer.
// The host replaces DEPTH_DECL and DEPTH_LOAD to match the render target's
// multisample count. The two WGSL variants use an identical buffer layout.
struct FlareCamera {
    view_proj: mat4x4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    eye: vec4<f32>,
    viewport_time: vec4<f32>,  // width, height, seconds, unused
    frame: vec4<u32>,          // monotonically increasing frame number
};
struct Flare {
    origin_radius: vec4<f32>,
    color: vec4<f32>,
    normal: vec4<f32>,
    fog: vec4<f32>,
};
struct FlareState {
    visible: u32,
    last_frame: u32,
    fade_start: f32,
    pad: f32,
};
@group(0) @binding(0) var<uniform> camera: FlareCamera;
@group(0) @binding(1) var<storage, read> flares: array<Flare>;
@group(0) @binding(2) DEPTH_DECL;
@group(0) @binding(3) var<storage, read_write> states: array<FlareState>;

fn nearest_depth(pixel: vec2<i32>) -> f32 {
    DEPTH_LOAD
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) index: vec3<u32>) {
    let i = index.x;
    if (i >= camera.frame.y) { return; }
    let flare = flares[i];
    let id = u32(flare.normal.w);
    let projected = camera.view_proj * vec4<f32>(flare.origin_radius.xyz + flare.normal.xyz * 3.0, 1.0);
    var passes = false;
    if (projected.w > 0.0) {
        let ndc = projected.xyz / projected.w;
        if (abs(ndc.x) <= 1.0 && abs(ndc.y) <= 1.0 && ndc.z >= 0.0 && ndc.z <= 1.0) {
            let coord = vec2<f32>((ndc.x * 0.5 + 0.5) * camera.viewport_time.x,
                                    (0.5 - ndc.y * 0.5) * camera.viewport_time.y);
            let size = vec2<i32>(camera.viewport_time.xy);
            let pixel = clamp(vec2<i32>(coord), vec2<i32>(0), size - vec2<i32>(1));
            // Same physical center as the reference's point probe. Small
            // tolerance prevents a point on its own BSP polygon being hidden.
            passes = ndc.z >= nearest_depth(pixel) - 0.000015;
        }
    }
    let prior = states[id];
    if (passes) {
        if (prior.visible == 0u || prior.last_frame != camera.frame.x - 1u) {
            states[id].fade_start = camera.viewport_time.z;
        }
        states[id].visible = 1u;
    } else {
        // Occlusion hides immediately; re-entry fades over 500ms.
        states[id].visible = 0u;
    }
    states[id].last_frame = camera.frame.x;
}

// Bounded Beer-Lambert medium, captured before water and diagnostic overlays.
struct Optics {
    inverse_view_projection: mat4x4<f32>,
    eye: vec4<f32>,
    forward: vec4<f32>,
    fog: vec4<f32>,
    params: vec4<f32>,
    minimum: array<vec4<f32>, 8>,
    maximum: array<vec4<f32>, 8>,
};
COLOR_DECLARATION
@group(0) @binding(1) var scene_sampler: sampler;
DEPTH_DECLARATION
@group(0) @binding(3) var<uniform> optics: Optics;
// OCEAN_STRUCT
@group(1) @binding(1) var wave_normal: texture_2d_array<f32>;
@group(1) @binding(2) var wave_sampler: sampler;
@group(1) @binding(3) var<uniform> ocean: OceanRenderSettings;

// Depth-unprojected receivers can fall just outside their water bounds. Pool
// walls commonly lie exactly on those bounds (e.g. hothchallenges y=7728), so
// strict containment made caustics toggle per pixel like a coplanar overlay.
// Match the outline's existing tolerance, in map units. Only receiver tests
// use this margin; ray intervals and absorption retain the actual volume.
const CAUSTIC_RECEIVER_MARGIN: f32 = 0.75;

@vertex fn vs_main(@builtin(vertex_index) id: u32) -> @builtin(position) vec4<f32> {
    let xy = vec2<f32>(f32((id << 1u) & 2u), f32(id & 2u));
    return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}
fn interval(a: vec3<f32>, delta: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> vec2<f32> {
    var near = 0.0;
    var far = 1.0;
    for (var axis = 0u; axis < 3u; axis += 1u) {
        if (abs(delta[axis]) < 0.00001) {
            if (a[axis] < lo[axis] || a[axis] > hi[axis]) { return vec2<f32>(1.0, 0.0); }
        } else {
            let t0 = (lo[axis] - a[axis]) / delta[axis];
            let t1 = (hi[axis] - a[axis]) / delta[axis];
            near = max(near, min(t0, t1));
            far = min(far, max(t0, t1));
        }
    }
    return vec2<f32>(near, far);
}

// The surface outline (see OceanMasks): a box is only the bounding rectangle of
// its water, so an island ring's hollow must not count as water. A volume's
// `minimum.w` carries its mask slot + 1, 0 meaning "the box is the water".
fn ocean_mask_contains(slot: i32, p: vec2<f32>) -> bool {
    if (slot < 0 || slot >= 8) {
        return true;
    }
    let range = ocean.mask_ranges[slot];
    let count = u32(range.y);
    if (count == 0u) {
        return true;
    }
    let bounds = ocean.mask_bounds[slot];
    let outside_bounds = any(p < bounds.xy) || any(p > bounds.zw);
    if (any(p < bounds.xy - vec2<f32>(CAUSTIC_RECEIVER_MARGIN))
        || any(p > bounds.zw + vec2<f32>(CAUSTIC_RECEIVER_MARGIN))) {
        return false;
    }
    let tile_xy = vec2<u32>(clamp(
        floor((p - bounds.xy) / max(bounds.zw - bounds.xy, vec2<f32>(1.0e-4)) * 8.0),
        vec2<f32>(0.0), vec2<f32>(7.0)));
    let tile = tile_xy.y * 8u + tile_xy.x;
    let code = (ocean.mask_tiles[slot][tile >> 4u] >> ((tile & 15u) * 2u)) & 3u;
    // Cached tile classifications cover only the exact bounds. Points in the
    // margin must still pass the tolerant triangle test, including hole edges.
    if (!outside_bounds && code == 1u) { return false; }
    if (!outside_bounds && code == 2u) { return true; }
    let first = u32(range.x);
    let tolerance = 0.75;
    for (var i = 0u; i < count; i = i + 1u) {
        let t0 = ocean.mask_triangles[(first + i) * 2u];
        let t1 = ocean.mask_triangles[(first + i) * 2u + 1u];
        let a = t0.xy;
        let b = t0.zw;
        let c = t1.xy;
        let lo = min(a, min(b, c)) - vec2<f32>(tolerance);
        let hi = max(a, max(b, c)) + vec2<f32>(tolerance);
        if (any(p < lo) || any(p > hi)) {
            continue;
        }
        let ab = b - a;
        let bc = c - b;
        let ca = a - c;
        let e0 = (ab.x * (p.y - a.y) - ab.y * (p.x - a.x)) / max(length(ab), 1.0e-4);
        let e1 = (bc.x * (p.y - b.y) - bc.y * (p.x - b.x)) / max(length(bc), 1.0e-4);
        let e2 = (ca.x * (p.y - c.y) - ca.y * (p.x - c.x)) / max(length(ca), 1.0e-4);
        if ((e0 >= -tolerance && e1 >= -tolerance && e2 >= -tolerance)
            || (e0 <= tolerance && e1 <= tolerance && e2 <= tolerance)) {
            return true;
        }
    }
    return false;
}

// Water spans along the current ray, as fractions of its eye->surface delta,
// sorted by entry. Private state so masked volumes can add several spans.
// The capacity is deliberately small. This is a `var<private>` array indexed by
// a runtime value, so it cannot live in registers: every invocation reserves
// scratch memory for the whole array, which costs occupancy for the entire
// shader whether or not the spans are used. Measured on hothchallenges looking
// across water, capacity 16 vs 8 was 1.85ms vs 1.33ms in this pass, and 4 was
// no better than 8, so 8 is the knee. Eight also covers reality comfortably: an
// unmasked volume contributes one span, a ray crossing a ring-shaped outline two.
const OCEAN_MAX_SPANS: u32 = 8u;
var<private> g_spans: array<vec2<f32>, OCEAN_MAX_SPANS>;
var<private> g_span_count: u32;
var<private> g_eye_depth: f32;

fn add_span(span: vec2<f32>, top: f32, delta_y: f32) {
    if (span.y <= span.x) {
        return;
    }
    if (g_span_count >= OCEAN_MAX_SPANS) {
        // Overflow widens the last span rather than discarding this one, so a
        // ray crossing more than eight water pieces over-estimates absorption
        // slightly instead of silently losing a layer. `add_masked_spans`
        // already folds its own runs the same way.
        g_spans[OCEAN_MAX_SPANS - 1u] = vec2<f32>(
            min(g_spans[OCEAN_MAX_SPANS - 1u].x, span.x),
            max(g_spans[OCEAN_MAX_SPANS - 1u].y, span.y),
        );
        g_eye_depth = max(g_eye_depth, max(top - (optics.eye.y + delta_y * span.x), 0.0));
        return;
    }
    // Insertion sort allows exact union of overlapping bounds (no double fog).
    var j = g_span_count;
    loop {
        if (j == 0u) { break; }
        if (g_spans[j - 1u].x <= span.x) { break; }
        g_spans[j] = g_spans[j - 1u];
        j -= 1u;
    }
    g_spans[j] = span;
    g_span_count += 1u;
    g_eye_depth = max(g_eye_depth, max(top - (optics.eye.y + delta_y * span.x), 0.0));
}

// Part of `span` where the ray's xz line is inside one outline triangle (grown
// by the surface mask's 0.75 unit tolerance). x >= y means the ray misses it.
fn mask_triangle_interval(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, origin: vec2<f32>, direction: vec2<f32>, span: vec2<f32>) -> vec2<f32> {
    let miss = vec2<f32>(1.0, 0.0);
    let tolerance = 0.75;
    let p0 = origin + direction * span.x;
    let p1 = origin + direction * span.y;
    let lo = min(a, min(b, c)) - vec2<f32>(tolerance);
    let hi = max(a, max(b, c)) + vec2<f32>(tolerance);
    if (any(max(p0, p1) < lo) || any(min(p0, p1) > hi)) {
        return miss;
    }
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if (abs(area) < 1.0e-4) {
        return miss;
    }
    let winding = select(-1.0, 1.0, area > 0.0);
    var t = span;
    for (var edge_index = 0u; edge_index < 3u; edge_index += 1u) {
        var start = a;
        var finish = b;
        if (edge_index == 1u) {
            start = b;
            finish = c;
        } else if (edge_index == 2u) {
            start = c;
            finish = a;
        }
        let edge = finish - start;
        let inv_length = 1.0 / max(length(edge), 1.0e-4);
        // Signed distance to the edge (positive inside) is linear in t.
        let at_origin = winding * (edge.x * (origin.y - start.y) - edge.y * (origin.x - start.x)) * inv_length + tolerance;
        let slope = winding * (edge.x * direction.y - edge.y * direction.x) * inv_length;
        if (abs(slope) < 1.0e-9) {
            if (at_origin < 0.0) {
                return miss;
            }
        } else {
            let bound = -at_origin / slope;
            if (slope > 0.0) {
                t.x = max(t.x, bound);
            } else {
                t.y = min(t.y, bound);
            }
        }
    }
    return t;
}

// Adds the runs of `span` that lie inside the volume's real outline.
fn add_masked_spans(slot: i32, span: vec2<f32>, top: f32, delta: vec3<f32>) {
    let range = ocean.mask_ranges[slot];
    let count = u32(range.y);
    let first = u32(range.x);
    var runs: array<vec2<f32>, 8>;
    var run_count = 0u;
    for (var i = 0u; i < count; i = i + 1u) {
        let t0 = ocean.mask_triangles[(first + i) * 2u];
        let t1 = ocean.mask_triangles[(first + i) * 2u + 1u];
        let piece = mask_triangle_interval(t0.xy, t0.zw, t1.xy, optics.eye.xz, delta.xz, span);
        if (piece.y <= piece.x) {
            continue;
        }
        // Neighbouring triangles abut, so fold every run this piece touches
        // into one. Overflow past eight runs widens the last one.
        var merged = piece;
        var kept = 0u;
        for (var k = 0u; k < run_count; k = k + 1u) {
            let run = runs[k];
            if (run.y >= merged.x - 1.0e-4 && run.x <= merged.y + 1.0e-4) {
                merged = vec2<f32>(min(merged.x, run.x), max(merged.y, run.y));
            } else {
                runs[kept] = run;
                kept += 1u;
            }
        }
        if (kept < 8u) {
            runs[kept] = merged;
            kept += 1u;
        } else {
            runs[7u] = vec2<f32>(min(runs[7u].x, merged.x), max(runs[7u].y, merged.y));
        }
        run_count = kept;
    }
    for (var k = 0u; k < run_count; k = k + 1u) {
        add_span(runs[k], top, delta.y);
    }
}

fn slopes(p: vec2<f32>) -> vec2<f32> {
    let scale = ocean.map_scales[1];
    let uv = p / ocean.ocean_info.z * vec2<f32>(1.0, -1.0) * scale.xy;
    return textureSampleLevel(wave_normal, wave_sampler, uv, 1, 0.0).xy * vec2<f32>(1.0, -1.0) * scale.w;
}
struct Capture {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
};
fn capture_sample(uv: vec2<f32>, depth: f32, sample_color: vec3<f32>) -> Capture {
    let h = optics.inverse_view_projection * vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), depth, 1.0);
    let world = h.xyz / h.w;
    let delta = world - optics.eye.xyz;
    let ray_length = length(delta);
    // Derivatives must remain outside pixel-dependent branches for WGSL
    // derivative uniformity; other caustics work is gated below.
    let world_dx = dpdx(world);
    let world_dy = dpdy(world);
    var color = sample_color;
    g_span_count = 0u;
    g_eye_depth = 0.0;
    var bottom_depth = 0.0;
    for (var i = 0u; i < min(u32(optics.params.w), 8u); i += 1u) {
        let lo = optics.minimum[i].xyz;
        let hi = optics.maximum[i].xyz;
        let slot = i32(round(optics.minimum[i].w)) - 1;
        let span = interval(optics.eye.xyz, delta, lo, hi);
        if (span.y > span.x) {
            if (slot >= 0 && slot < 8) {
                add_masked_spans(slot, span, hi.y, delta);
            } else {
                add_span(span, hi.y, delta.y);
            }
        }
        let receiver_margin = vec3<f32>(CAUSTIC_RECEIVER_MARGIN);
        if (all(world >= lo - receiver_margin) && all(world <= hi + receiver_margin)
            && ocean_mask_contains(slot, world.xz)) {
            bottom_depth = max(bottom_depth, max(hi.y - world.y, 0.0));
        }
    }
    let eye_depth = g_eye_depth;
    var water_fraction = 0.0;
    var end = 0.0;
    for (var i = 0u; i < g_span_count; i += 1u) {
        water_fraction += max(g_spans[i].y - max(g_spans[i].x, end), 0.0);
        end = max(end, g_spans[i].y);
    }
    // Optional low-cost wave-curvature caustics. Approximation, not photon tracing.
    // No caustic-specific normal reconstruction or wave samples when disabled.
    if (optics.params.z > 0.0 && bottom_depth > 2.0 && depth > 0.0) {
        // Scope to the bound simulation; distinct authored oceans never borrow its waves.
        let own_margin = vec2<f32>(CAUSTIC_RECEIVER_MARGIN);
        let own_water = ocean.authored_plane.x < 0.5 || (all(world.xz >= ocean.authored_bounds.xy - own_margin) && all(world.xz <= ocean.authored_bounds.zw + own_margin));
        let sun = normalize(-ocean.sun_direction_intensity.xyz);
        if (own_water && sun.y > 0.05) {
            let normal_cross = cross(world_dx, world_dy);
            let normal_length = length(normal_cross);
            // No invented world-up normal when gradients are degenerate.
            // In that case the receiver's orientation is unknown: skip caustics.
            if (normal_length > 0.000001) {
                let scene_normal = normal_cross / normal_length;
                let world_pixel_span = max(length(world_dx), length(world_dy));
                let viewport = vec2<f32>(textureDimensions(scene_color));
                let expected_pixel_span = max(ray_length * 4.0 / max(min(viewport.x, viewport.y), 1.0), 0.25);
                let caustic_surface_confidence = 1.0 - smoothstep(
                    expected_pixel_span * 4.0,
                    expected_pixel_span * 12.0,
                    world_pixel_span,
                );
                let refracted = refract(-sun, vec3<f32>(0.0, 1.0, 0.0), 1.0 / 1.333);
                let surface = world.xz - refracted.xz * bottom_depth / max(-refracted.y, 0.1);
                let step = 8.0;
                let dx = (slopes(surface + vec2<f32>(step,0.0)) - slopes(surface - vec2<f32>(step,0.0))) / (2.0*step);
                let dz = (slopes(surface + vec2<f32>(0.0,step)) - slopes(surface - vec2<f32>(0.0,step))) / (2.0*step);
                let focus = clamp(1.0 / max(0.25, 1.0 + bottom_depth * 0.25 * (dx.x + dz.y)) - 1.0, -0.4, 2.0);
                let fade = exp2(-4.321928 * bottom_depth / max(optics.fog.w, 1.0));
                let caustic = focus * optics.params.z * fade * abs(dot(scene_normal, sun)) * min(ocean.sun_direction_intensity.w, 1.0);
                // Preserve the existing discontinuity fade and brightness clamp.
                let caustic_multiplier = max(1.0 + caustic, 0.65);
                color *= mix(1.0, caustic_multiplier, caustic_surface_confidence);
            }
        }
    }
    if (water_fraction > 0.0) {
        // log2(20) corresponds to FIVE percent transmission, not four.
        let ratios = vec3<f32>(38.0/7.5, 38.0/22.0, 1.0);
        let transmission = exp2(-4.321928 * ratios * ray_length * water_fraction / max(optics.fog.w, 1.0));
        let downwelling = exp2(-4.321928 * ratios * eye_depth / max(optics.params.x, 1.0));
        color = color * transmission + optics.fog.rgb * downwelling * (1.0 - transmission);
    }
    var out: Capture;
    out.color = vec4<f32>(color, 1.0);
    out.depth = select(max(dot(delta, optics.forward.xyz), 0.0), 1000000.0, depth <= 0.0);
    return out;
}

@fragment fn fs_capture(@builtin(position) p: vec4<f32>) -> Capture {
    let pixel = vec2<i32>(p.xy);
    let uv = p.xy / vec2<f32>(textureDimensions(scene_color));
    CAPTURE_BODY
}

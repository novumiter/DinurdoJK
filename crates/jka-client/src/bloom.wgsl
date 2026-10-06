@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var authored_texture: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0)
    );
    let p = positions[vertex_index];
    var out: VertexOut;
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn source(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(
        source_texture,
        source_sampler,
        clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0))
    );
}

fn authored(uv: vec2<f32>) -> vec3<f32> {
    return textureSample(
        authored_texture,
        source_sampler,
        clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0))
    ).rgb;
}

fn tent_downsample(uv: vec2<f32>) -> vec4<f32> {
    let dims_u = textureDimensions(source_texture);
    let texel = 1.0 / vec2<f32>(f32(dims_u.x), f32(dims_u.y));
    // Much wider neutral veiling glare with the same five taps and same three
    // pyramid passes. Each lower-resolution stage compounds this footprint.
    let diagonal = texel * 4.25;
    var color = source(uv) * 0.25;
    color += source(uv + vec2<f32>(-diagonal.x, -diagonal.y)) * 0.1875;
    color += source(uv + vec2<f32>( diagonal.x, -diagonal.y)) * 0.1875;
    color += source(uv + vec2<f32>(-diagonal.x,  diagonal.y)) * 0.1875;
    color += source(uv + vec2<f32>( diagonal.x,  diagonal.y)) * 0.1875;
    return color;
}

fn modern_extract(uv: vec2<f32>) -> vec3<f32> {
    let color = tent_downsample(uv).rgb;
    let bright = smoothstep(0.52, 0.86, luminance(color));
    return color * bright;
}

fn authored_downsample(uv: vec2<f32>) -> vec3<f32> {
    let dims_u = textureDimensions(authored_texture);
    let texel = 1.0 / vec2<f32>(f32(dims_u.x), f32(dims_u.y));
    let diagonal = texel * 4.25;
    var color = authored(uv) * 0.25;
    color += authored(uv + vec2<f32>(-diagonal.x, -diagonal.y)) * 0.1875;
    color += authored(uv + vec2<f32>( diagonal.x, -diagonal.y)) * 0.1875;
    color += authored(uv + vec2<f32>(-diagonal.x,  diagonal.y)) * 0.1875;
    color += authored(uv + vec2<f32>( diagonal.x,  diagonal.y)) * 0.1875;
    return color;
}

@fragment
fn fs_extract_modern(input: VertexOut) -> @location(0) vec4<f32> {
    let modern = modern_extract(input.uv);
    // Alpha carries the modern red-highlight signal independently through the
    // shared pyramid so film halation remains luminance-driven in every Glow mode.
    return vec4<f32>(modern, modern.r);
}

@fragment
fn fs_extract_authored(input: VertexOut) -> @location(0) vec4<f32> {
    // Halation disabled: avoid paying for modern scene extraction in authored-only mode.
    return vec4<f32>(authored_downsample(input.uv), 0.0);
}

@fragment
fn fs_extract_authored_halation(input: VertexOut) -> @location(0) vec4<f32> {
    let modern = modern_extract(input.uv);
    return vec4<f32>(authored_downsample(input.uv), modern.r);
}

@fragment
fn fs_extract_combined(input: VertexOut) -> @location(0) vec4<f32> {
    // Avoid a 2x hotspot when the same bright authored stage also crosses the
    // modern luminance threshold. Preserve the stronger RGB contribution.
    let modern = modern_extract(input.uv);
    let jka = authored_downsample(input.uv);
    return vec4<f32>(max(modern, jka), modern.r);
}

@fragment
fn fs_downsample(input: VertexOut) -> @location(0) vec4<f32> {
    // Downsample all four channels: RGB is the selected Glow source, while A
    // remains the independent modern-highlight channel used only by halation.
    return tent_downsample(input.uv);
}

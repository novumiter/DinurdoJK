// Experimental sky override adapted from "Star Nest" by Pablo Roman Andrioli (Kali).
// Original shader: https://www.shadertoy.com/view/XlfGRj
// Original work is MIT licensed. Copyright (c) Pablo Roman Andrioli.
// Changes: JKA world-space rays/camera translation, no Shadertoy mouse/camera,
// bounded fractal denominator and a smaller per-pixel budget (12 x 10).
// This routine is called ONLY from the BSP sky fragment, never on world geometry.
// Shadertoy's final vec4 is display-encoded (drawn directly to an LDR canvas).
// JKA's BSP stage writes linear light into an sRGB or float scene attachment.
// Decode before writing, so the swapchain's sRGB transfer doesn't lift blacks.
fn star_nest_display_to_linear(channel: f32) -> f32 {
    if (channel <= 0.04045) {
        return channel / 12.92;
    }
    return pow((channel + 0.055) / 1.055, 2.4);
}
fn experimental_star_nest_sky(jka_direction: vec3<f32>) -> vec4<f32> {
    // Convert the WGPU camera translation back to JKA coordinates (Z up).
    // Scaling defines the apparent depth of this *experimental* sky: a 1000u
    // translation moves the fractal eye about 0.12 field units.
    let eye = vec3<f32>(
        camera.camera_pos_time.x,
        -camera.camera_pos_time.z,
        camera.camera_pos_time.y
    ) * 0.00012;
    let t = camera.camera_pos_time.w * 0.003;
    let origin = vec3<f32>(1.0, 0.5, 0.5) + eye + vec3<f32>(t * 2.0, t, -0.5);
    let direction = normalize(jka_direction);
    let tile = 0.85;
    let period = tile * 2.0;
    var distance = 0.1;
    var fade = 1.0;
    var radiance = vec3<f32>(0.0);
    for (var step = 0u; step < 12u; step = step + 1u) {
        var point = origin + distance * direction * 0.5;
        point = abs(vec3<f32>(tile) - (point - floor(point / period) * period));
        var previous_length = 0.0;
        var accumulated = 0.0;
        for (var i = 0u; i < 10u; i = i + 1u) {
            point = abs(point) / max(dot(point, point), 0.0001) - vec3<f32>(0.53);
            let current_length = length(point);
            accumulated += abs(current_length - previous_length);
            previous_length = current_length;
        }
        let dark = max(0.0, 0.3 - accumulated * accumulated * 0.001);
        if (step > 6u) {
            fade *= 1.0 - dark;
        }
        let contrast = accumulated * accumulated * accumulated;
        radiance += vec3<f32>(fade);
        radiance += vec3<f32>(distance, distance * distance,
            distance * distance * distance * distance) * contrast * 0.0015 * fade;
        fade *= 0.73;
        distance += 0.1;
    }
    radiance = mix(vec3<f32>(length(radiance)), radiance, 0.85);
    // Restore the Shadertoy brightness (0.01, not 0.016). Its output is LDR
    // display-encoded RGB; convert that to linear-light RGB for this renderer.
    // Clamping matches the original canvas's normalized framebuffer.
    let display_rgb = clamp(radiance * 0.01, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(
        star_nest_display_to_linear(display_rgb.r),
        star_nest_display_to_linear(display_rgb.g),
        star_nest_display_to_linear(display_rgb.b),
        1.0
    );
}

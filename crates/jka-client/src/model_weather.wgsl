// Wet weather on entities: players, props, weapons and the view model.
//
// The world shaders read wetness from the map-wide weather field through a bind
// group of their own (weather_surface_world.wgsl). Models have nothing like it:
// md3.wgsl and ghoul2_skin.wgsl bind a camera, one texture and the shared
// entity-lighting uniform, and that is all. Rather than add a bind group to every
// model pipeline - and to the ray-traced receiver variants, which already occupy
// groups 2 and 3 - the handful of values a wet model actually needs ride in the
// entity-lighting uniform that every model draw already binds. This costs no new
// binding, no pipeline permutation and no change to the draw path.
//
// What that gives up is per-fragment cover: a model cannot ask the field whether
// it is standing under a roof. The CPU's camera exposure stands in, which is
// right whenever the camera and the model share shelter and covers the case a
// player actually notices - walking indoors and drying off.
//
// There is deliberately no override constant. The whole path sits behind one
// uniform test with the same answer for every fragment in the frame, so the
// branch is perfectly coherent; specializing it would buy nothing and cost a
// variant of every model pipeline.

const MODEL_WET_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// How wet this fragment is. Rain lands on whatever faces the sky, so a shoulder
// or a helmet is soaked while the underside of an arm stays dry - but water also
// runs down a body, so vertical surfaces keep most of it and only true undersides
// dry out.
fn model_wetness(world_normal: vec3<f32>) -> f32 {
    let wet = entity_lighting.weather_sun_radiance.w;
    if (wet <= 0.001) {
        return 0.0;
    }
    return wet * mix(0.22, 1.0, smoothstep(-0.55, 0.65, normalize(world_normal).y));
}

// Water in the pores darkens a surface, never shifts its hue: wet cloth goes
// darker, not browner. Same bounded multiply as weather_wet_albedo in
// weather_surface_world.wgsl, so a player and the ground they stand on darken
// together instead of drifting apart. Models carry no material id to look a
// porosity up from, and a character is a mix of cloth, leather and armour, so
// they take a middling share rather than the ground's full soak.
fn model_wet_albedo(dry: vec3<f32>, wet: f32) -> vec3<f32> {
    return dry * max(1.0 - 0.38 * wet, 0.08);
}

// A wet model reads almost entirely through its highlight: a sharp sun glint on
// the soaked upper surfaces and a grazing sheen everywhere else. Nothing on a
// model samples an environment, so the sheen borrows the light the surface is
// already standing in rather than inventing a colour of its own.
fn model_wet_color_with_water(dry: vec4<f32>, world_normal: vec3<f32>, world_position: vec3<f32>, water_wet: f32) -> vec4<f32> {
    // Submersion wets undersides too, unlike directional falling rain.
    // MAX avoids double darkening where the two sources overlap.
    let wet = max(model_wetness(world_normal), clamp(water_wet, 0.0, 1.0));
    if (wet <= 0.001) {
        return dry;
    }
    let lit = model_wet_albedo(dry.rgb, wet);
    let n = normalize(world_normal);
    let v = normalize(camera.camera_pos_time.xyz - world_position);
    let n_dot_v = max(dot(n, v), 0.0);

    let fresnel = 0.02 + 0.98 * pow(1.0 - n_dot_v, 5.0);
    var sheen = vec3<f32>(dot(lit, MODEL_WET_LUMA) * fresnel * wet * 0.55);

    // The sun glint shares weather_surface.wgsl's water BRDF, so a wet shoulder
    // and the puddle under it are the same liquid. `weather_sun_direction` is the
    // light's travel direction, as in the world weather shaders. The roughness
    // floor keeps a highlight that survives being resolved across an animated,
    // interpolated model normal instead of flickering down to single pixels.
    let sun = entity_lighting.weather_sun_radiance.rgb * entity_lighting.weather_sun_direction.w;
    if (max(sun.r, max(sun.g, sun.b)) > 0.001) {
        let l = normalize(-entity_lighting.weather_sun_direction.xyz);
        sheen += sun * (water_specular(n, v, l, 0.22) * wet * 0.6);
    }
    // Alpha is the material's own: wetness must never make a masked or blended
    // surface more or less present than the artist drew it.
    return vec4<f32>(lit + sheen, dry.a);
}

// Props and the MD3 reference path have no per-player water payload.
fn model_wet_color(dry: vec4<f32>, world_normal: vec3<f32>, world_position: vec3<f32>) -> vec4<f32> {
    return model_wet_color_with_water(dry, world_normal, world_position, 0.0);
}

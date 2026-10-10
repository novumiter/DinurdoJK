//! Frame plan.
use crate::renderer::{
    weather, CullDebugMode, DynamicLightsMode, DynamicShadowsMode,
    PlanarReflectionDebugMode, Renderer, WorldRenderPath,
};

impl Renderer {
    pub(in crate::renderer) fn rebuild_frame_plan(&mut self) {
        let fog_enabled = self.weather.fog.volumetric_effective();
        let legacy_fog_enabled = self.weather.fog.legacy_effective();
        let legacy1_global_post = self.weather.fog.legacy1_global_post_active();
        // Legacy GL kept r_drawfog 2 smooth with framebuffer dithering. Our
        // equivalent uses a float scene plus one dithered presentation pass so
        // the fog blend is not irreversibly quantized before presentation.
        let legacy2_precision_post = self.weather.fog.legacy2_precision_active();
        let gamma_enabled = (self.output_gamma() - 1.0).abs() > 0.001;
        let lut_enabled = self.color_lut_effective_strength > 0.001;
        let manual_exposure = self.color_grade.exposure_ev.abs() > 0.001;
        // HDR by itself only changes the scene target to Rgba16Float. It does
        // not need the heavyweight spatial post shader; the compact presentation
        // pass can resolve the float scene to the swapchain. Keep HDR out of the
        // full-post bucket so forced Unified/Advanced + r_hdr 1 pays only the
        // unavoidable 16F bandwidth and one tiny fullscreen resolve.
        let hdr_resolve = self.hdr_enabled;
        let rain_spatial_post = self.weather.rain.enabled && !self.weather.rain.fast_post;
        let rain_lens_copy = self.weather.rain.enabled
            && self.weather.rain.glass_enabled
            && self.weather.rain.fast_post;
        let other_full_post_effects = self.ocean_enabled
            || self.tone_mapping_enabled
            || self.glow_mode.enabled()
            || self.halation_enabled
            || self.ssao_enabled
            || self.fxaa_enabled
            || self.temporal_resolve_active()
            || self.contact_shadows_enabled
            || fog_enabled
            || legacy1_global_post
            || self.clouds_enabled
            || self.weather.snow.enabled
            || self.weather.atmosphere.active()
            || self.ssr_enabled
            || self.reflection_debug_enabled
            || self.chromatic_aberration_strength > 0.001
            || self.film_grain_strength > 0.001
            || self.motion_blur_strength > 0.001
            || self.depth_of_field_strength > 0.001;
        let rain_full_post_effects = rain_spatial_post
            || (self.weather.rain.puddle_amount > weather::RAIN_PUDDLE_EPSILON
                && !self.weather.rain.fast_post);
        let full_post_effects = other_full_post_effects || rain_full_post_effects;
        let rain_only_post = self.weather.rain.lean_post
            && rain_full_post_effects
            && !other_full_post_effects;
        // SMAA is a separate reference-quality resolve after the renderer's
        // chosen output path. It does not by itself require our post-process
        // intermediate: with gamma/effects off the world can render directly
        // into the SMAA color target before SMAA resolves to the surface.
        let compact_post_reason = hdr_resolve
            || gamma_enabled
            || lut_enabled
            || manual_exposure
            || legacy2_precision_post
            || rain_lens_copy;
        let use_post = compact_post_reason || full_post_effects;
        // Gamma and color LUTs affect only the final image. Keep the fast BSP
        // shaders/frame recorder and reuse one compact post pass for both.
        let use_hiz = self.gpu_driven_enabled && self.hiz_occlusion_enabled;

        // Render settings are resolved here, on the cold path. The maximum-FPS
        // frame loop should not rediscover that every advanced feature is OFF
        // hundreds of times per frame/per draw. This path intentionally matches
        // the feature envelope of the known ~2100 FPS renderer: basic BSP, PVS,
        // texture filtering, MSAA, optional wireframe, optional gamma, and UI.
        let fast_baseline = !self.force_unified_world
            && !self.dynamic_light_debug
            && !self.hdr_enabled
            && !self.tone_mapping_enabled
            && !self.glow_mode.enabled()
            && !self.halation_enabled
            && !self.ssao_enabled
            && !self.fxaa_enabled
            && !self.smaa_enabled
            && !self.temporal_resolve_active()
            && !self.contact_shadows_enabled
            && !fog_enabled
            && !legacy_fog_enabled
            && !self.clouds_enabled
            && !self.weather.rain.enabled
            && !self.weather.snow.enabled
            && !self.weather.atmosphere.active()
            && !self.ocean_enabled
            && self.weather.rain.surface_wetness <= weather::RAIN_WETNESS_EPSILON
            && self.weather.rain.puddle_amount <= weather::RAIN_PUDDLE_EPSILON
            && !self.ssr_enabled
            && !self.reflection_debug_enabled
            && self.chromatic_aberration_strength <= 0.001
            && !self.vignette_enabled
            && self.film_grain_strength <= 0.001
            && self.motion_blur_strength <= 0.001
            && self.depth_of_field_strength <= 0.001
            && !self.gpu_driven_enabled
            && !self.hiz_occlusion_enabled
            && !self.detail_textures_enabled
            && !self.point_lighting_enabled()
            && !matches!(
                self.dynamic_lights_mode,
                DynamicLightsMode::Legacy
                    | DynamicLightsMode::Vertex
                    | DynamicLightsMode::ClusteredLite
            )
            && !self.any_area_lights_enabled()
            && !self.irradiance_volume_enabled
            && !self.voxel_probe_gi_enabled
            && !self.local_light_shadows_enabled
            && !self.pbr_enabled
            && !self.cascaded_shadows_enabled
            && !(self.hardware_rt_requested() && self.ray_traced_shadows.is_some())
            && self.cull_debug_mode == CullDebugMode::Off
            && !self.planar_reflection.active
            && self.planar_reflection_debug_mode == PlanarReflectionDebugMode::Off
            && !self.jump_shade.engaged();
        // Hi-Z reads the camera depth buffer, so it no longer forces the full
        // linear-depth prepass: with no other consumer it runs a cheaper
        // depth-only early pass instead (see `FramePlan::hiz_early`).
        let other_linear_depth = self.ssao_enabled
            || (self.rt_reduced_shadows
                && self.cascaded_shadow_mode == DynamicShadowsMode::RayTraced
                && self.ray_tracing_supported)
            || self.temporal_resolve_active()
            || self.contact_shadows_enabled
            || fog_enabled
            || legacy1_global_post
            || self.clouds_enabled
            || self.weather.snow.enabled
            || self.weather.atmosphere.active()
            || self.ssr_enabled
            || self.reflection_debug_enabled
            || self.motion_blur_strength > 0.001
            || self.depth_of_field_strength > 0.001;
        let needs_linear_depth = other_linear_depth || rain_spatial_post;
        let rain_only_linear_depth = rain_spatial_post && !other_linear_depth;
        self.frame_plan = FramePlan {
            world_path: if fast_baseline {
                WorldRenderPath::FastBaseline
            } else {
                WorldRenderPath::Advanced
            },
            use_post,
            lut_post: lut_enabled,
            // The tiny post path handles display gamma and/or one trilinear 3-D
            // LUT sample. If that path is already required it also applies the
            // vignette, but vignette alone does not force an offscreen scene copy.
            // Legacy 2 only needs a precision-preserving copy plus final dither;
            // do not compile/run the heavyweight full-post shader just for fog.
            // Despite the historical field name, this is the compact color-only
            // presentation path. HDR-only belongs here too: the world remains on
            // the selected (including forced Unified/Advanced) path, but we avoid
            // the full depth-aware post shader when no effect actually needs it.
            gamma_only_post: compact_post_reason && !full_post_effects,
            needs_linear_depth,
            rain_only_linear_depth,
            rain_only_post,
            use_hiz,
            hiz_early: use_hiz && !needs_linear_depth,
            use_gpu_culling: self.gpu_driven_enabled || self.cull_debug_mode != CullDebugMode::Off,
            // Area lights use the same clustered lists as entity/point lights,
            // but remain independently toggleable. The active BSP pipeline is
            // specialized for the requested source classes when settings change.
            use_clustered_lighting: self.point_lighting_enabled()
                || self.dynamic_lights_mode == DynamicLightsMode::ClusteredLite
                || self.any_area_lights_enabled(),
        };
        let plan = self.frame_plan;
        self.ensure_post_pipeline(plan.use_post && !plan.gamma_only_post && !plan.rain_only_post);
        self.ensure_rain_post_pipeline(plan.rain_only_post);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::renderer) struct FramePlan {
    pub(in crate::renderer) world_path: WorldRenderPath,
    pub(in crate::renderer) use_post: bool,
    /// Select the already-compiled compact grading shader on the fast path.
    /// Color-only effects never require the advanced world renderer.
    pub(in crate::renderer) lut_post: bool,
    pub(in crate::renderer) gamma_only_post: bool,
    pub(in crate::renderer) needs_linear_depth: bool,
    /// Rain is the sole consumer of the full-resolution linear-depth prepass.
    pub(in crate::renderer) rain_only_linear_depth: bool,
    /// Rain is the only spatial post effect and the rain-only PSO is requested.
    pub(in crate::renderer) rain_only_post: bool,
    pub(in crate::renderer) use_hiz: bool,
    /// Hi-Z is the only consumer of depth this frame, so no full prepass is
    /// drawn: a depth-only early pass of last frame's visible batches builds
    /// the pyramid instead (Bevy's two-phase occlusion culling, minus the late
    /// prepass nothing here would read).
    pub(in crate::renderer) hiz_early: bool,
    pub(in crate::renderer) use_gpu_culling: bool,
    pub(in crate::renderer) use_clustered_lighting: bool,
}

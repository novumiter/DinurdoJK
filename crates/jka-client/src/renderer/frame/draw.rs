//! Frame draw.
use crate::renderer::{
    aabb_behind_plane, aabb_intersects_clip_frustum, aabb_intersects_shadow_frustum,
    aabb_screen_rect,
    active_batch_selection, batch_area_visible, bevy_cascade_shadow_matrices, block_on,
    camera_depth_clear, claim_ocean_clipmap, draw_snow_shell, draw_world_batch,
    effective_area_mask, inline_run_fog_depth, inline_stage_info, inspector_index_range,
    legacy_dlight_batch_selection, legacy_dlight_receives, ocean_pipeline_key,
    ocean_suppresses_batch, pack_inline_draws, refresh_auto4_lazy_collapse, same_world_surface,
    sample_view_rotation, scene, select_planar_reflection_views, snow_shell_draw_near_center,
    temporal_jitter, ui, update_active_cull_indices, update_visibility, weather, Arc,
    AutoExposureUniform, BlendMode, Camera, CameraUniform, CompanionSceneView, CullDebugMode,
    DofUniform, DrawClass, DrawIndexedIndirectArgs, DynamicLightsMode, DynamicModelSurface,
    DynamicShadowsMode, FootprintMode, FrameInfo, GlowMode, GpuPass, Instant, LatestViewState, Mat4,
    PlanarReflectionDebugMode, PlanarReflectionMode, PlanarReflectionUniform, PvsMode, RenderError,
    Renderer, SpectatorHighlight, SunVisibilityMode, TcGen, TransientLight, Vec3, ViewLatchMode, WorldBatch,
    WorldBatchSet, WorldRenderPath, WorldShaderFamily, CLOUD_INTERLEAVE_GRID, CLUSTER_COUNT,
    ENTITY_SHADOW_CAMERAS, PLANAR_REFLECTION_SLOTS, SHADOW_CASCADES,
};
use bytemuck::Zeroable;

impl Renderer {
    pub(in crate::renderer) fn render(
        &mut self,
        camera: &Camera,
        player_position: Option<Vec3>,
        area_mask: Option<&[u8; 32]>,
        dynamic_models: &[DynamicModelSurface],
        spectator_highlights: &[SpectatorHighlight],
        transient_lights: &[TransientLight],
        companion_scene: Option<&CompanionSceneView>,
        dof_focus_target: f32,
        dynamic_crosshair_world: Option<[f32; 3]>,
        motion_reference_camera: &Camera,
        motion_camera_sample_dt: f32,
        motion_camera_sample_age: f32,
        view_latch: ViewLatchMode,
        latest_view: Option<&LatestViewState>,
    ) -> Result<FrameInfo, RenderError> {
        // Every frame path (fast baseline, asset preview, full) is entered from
        // here, so one check covers them all. On a dead device the first queue
        // write of the frame fails silently and the panic surfaces wherever that
        // happens to be - historically inside egui's buffer upload, with a
        // message about buffer sizes that were never the problem.
        if crate::renderer::device_lost() {
            return Err(RenderError::DeviceLost);
        }
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(FrameInfo::default());
        }

        if self.baked_brightness.busy {
            self.tick_baked_brightness();
        }

        if self.asset_preview_mode {
            return self.render_asset_preview(camera, dynamic_models);
        }
        self.dynamic_model_renderer
            .set_companion_excluded_entity(companion_scene.map(|view| view.target_entity));
        if companion_scene.is_some() && self.world.is_some() {
            // The second camera deliberately reuses the low-overhead world PSO
            // family even when the primary view is on the advanced path. Lazy
            // compilation means enabling the feature never blocks this frame.
            let _ = self.ensure_fast_world_pipelines();
        }
        self.build_blob_shadow_marks(dynamic_models);

        // Resolved before any pass borrows the renderer mutably.
        let update_weather_surface = self.weather.rain.enabled
            || self.weather.rain.surface_wetness > weather::RAIN_WETNESS_EPSILON
            || self.weather.rain.puddle_amount > weather::RAIN_PUDDLE_EPSILON;
        if update_weather_surface {
            let wetness_time = self.started.elapsed().as_secs_f32();
            if self.advance_surface_wetness(wetness_time, camera.position) {
                self.update_weather_surface_uniform();
                self.rebuild_frame_plan();
            }
            self.update_wake(camera, player_position, dynamic_models);
        }
        self.update_entity_weather();

        self.sync_runtime_variant_flags();

        // Pipeline variants compile asynchronously. While a requested setting is
        // waiting on its new PSO, the previous fully valid variant remains active.
        // Structural render decisions must therefore follow the active variant,
        // not the desired settings. Mixing an Enhanced bind group with an active
        // Lean pipeline is a wgpu validation error because bindings 10..14 differ.
        let active_world_variant = self
            .world
            .as_ref()
            .map(|world| world.active_pipeline_variant);
        let enhanced_world_shader =
            active_world_variant.is_some_and(|key| key.family() == WorldShaderFamily::Enhanced);
        // Ocean resources are created/released immediately by SetOceanEnabled.
        // Promote authored water only once both the setting and active compiled
        // PSO agree that ocean support is live; until then stock water remains.
        let render_ocean = self.ocean_enabled && active_world_variant.is_some_and(|key| key.ocean);
        let ocean_clipmap =
            render_ocean.then(|| usize::from(self.ocean_settings.mesh_quality.min(1)));
        if render_ocean {
            self.update_promoted_ocean_clipmap_lod(camera.position.to_array());
        }

        let menu_backdrop_requested = self.menu_backdrop_requested();
        if menu_backdrop_requested {
            self.ensure_menu_backdrop_resources();
        }
        // The first menu frame is allowed to render directly while the backdrop
        // PSO/resources compile in the background; never block gameplay/UI.
        let menu_backdrop_active = menu_backdrop_requested && self.menu_backdrop.is_some();

        let crt_active = self.crt_scope != ui::CrtScope::Off;
        if crt_active {
            self.ensure_crt_resources();
        }
        // Clone the lightweight WGPU view handle so the rest of the frame can
        // mutably borrow Renderer without holding a borrow into crt_resources.
        let crt_view = self.crt_resources.as_ref().map(|resources| resources.view.clone());
        let crt_everything = self.crt_scope == ui::CrtScope::Everything;
        let crt_before_ui = crt_active && !crt_everything;

        // CGame/FX submits RE_AddLightToScene-style lights at simulation rate.
        // Keep them in a transient tail of the clustered-light buffer.
        self.set_transient_lights(transient_lights);

        // Snowflow state advances only when new brushes/window movement/relaxation
        // require it. This is deliberately before the fast-path early return.
        self.surface_deformation
            .prepare_frame(&self.device, &self.queue);
        self.update_world_videos();
        self.ensure_lazy_scene_pipelines();

        // Pipeline/render-plan selection happens only when settings change. The
        // hot frame path can therefore jump directly into the known-fast renderer
        // without walking every advanced feature branch. Surface/frame
        // diagnostics deliberately use the full path; GPU timestamps are
        // recorded by both paths so they can be compared.
        if self.frame_plan.world_path == WorldRenderPath::FastBaseline
            && !self.surface_diag_pending
            && !self.frame_diag_pending
            && !self.video_depth_capture_requested()
        {
            if self.ensure_fast_world_pipelines() {
                return self.render_fast_baseline(
                    camera,
                    player_position,
                    area_mask,
                    dynamic_models,
                    spectator_highlights,
                    companion_scene,
                    dynamic_crosshair_world,
                    view_latch,
                    latest_view,
                );
            }
            // First selection of FastBaseline compiles its map-specific PSO set
            // in the background. Keep rendering through the already-hot advanced
            // path until that set is ready instead of stalling this frame.
        }

        // Keep glow pipeline polling entirely outside the maximum-FPS path.
        self.ensure_bloom_pipelines();

        let frame_started = Instant::now();
        let frame_time = self.started.elapsed().as_secs_f32();
        let frame_delta = if self.previous_frame_time > 0.0 {
            (frame_time - self.previous_frame_time).clamp(0.0, 0.25)
        } else {
            0.0
        };
        if self.depth_of_field_strength > 0.001 {
            let target = dof_focus_target.clamp(32.0, 16384.0);
            if !self.dof_focus_valid || frame_delta <= 0.0 {
                self.dof_focus_distance = target;
                self.dof_focus_valid = true;
            } else {
                // Smooth autofocus like a physical focus pull instead of snapping
                // to every tiny collision-depth change under the crosshair.
                let alpha = 1.0 - (-frame_delta / 0.16).exp();
                self.dof_focus_distance += (target - self.dof_focus_distance) * alpha;
            }
        }
        if self.depth_of_field_strength > 0.001 {
            self.queue.write_buffer(
                &self.dof_buffer,
                0,
                bytemuck::bytes_of(&DofUniform {
                    focus_strength: [
                        self.dof_focus_distance,
                        self.depth_of_field_strength,
                        self.config.width.max(1) as f32,
                        self.config.height.max(1) as f32,
                    ],
                    quality: [self.dof_quality.shader_value(), 0.0, 0.0, 0.0],
                }),
            );
        }
        self.gpu_profiler.begin_frame(&self.device);
        if self.cull_debug_mode != CullDebugMode::Off {
            self.cull_diagnostics_readback.begin_frame(&self.device);
        }
        if render_ocean {
            self.ocean_visibility.begin_frame(&self.device);
        }
        let mut plan = self.frame_plan;
        let rt_taa_active = self.rt_taa_active();
        if self.video_depth_capture_requested() {
            // Export the exact linear depth texture used by the compositor/DOF
            // path without making ordinary maximum-FPS frames pay for it.
            plan.needs_linear_depth = true;
            plan.hiz_early = false;
        }
        let jitter = if self.taa_enabled {
            temporal_jitter(self.taa_frame_index)
        } else {
            [0.0, 0.0]
        };
        // Full/advanced rendering has many camera-derived CPU uniforms and
        // reflection/visibility decisions. Latch once immediately before the
        // first of those so every dependent pass sees one coherent view.
        let mut render_camera = *camera;
        let late_view_sample = latest_view
            .and_then(|state| sample_view_rotation(&mut render_camera, state, view_latch));
        let camera = &render_camera;
        let view_proj = if self.taa_enabled {
            camera.view_projection_jittered(self.config.width, self.config.height, jitter)
        } else {
            camera.view_projection(self.config.width, self.config.height)
        };
        self.refresh_water_boxes(camera.position);
        let mut optical_volumes = None;
        if render_ocean
            && self
                .world
                .as_ref()
                .is_some_and(|w| !w.ocean_surfaces.is_empty())
        {
            let key = (
                self.config.width.max(1),
                self.config.height.max(1),
                self.msaa_samples,
                self.scene_format(),
            );
            if self.ocean_optics.key != key {
                self.ocean_optics = crate::ocean::optics::Resources::new(
                    &self.device,
                    &self.ocean_optics_layout,
                    &self.ocean_layout,
                    key,
                );
            }
            let optical = self.ocean_settings.optics;
            let fog = crate::ocean::optics::linear_color(optical.fog_color);
            let mut uniform = crate::ocean::optics::Uniform {
                inverse_view_projection: view_proj.inverse().to_cols_array_2d(),
                eye: camera.position.extend(1.0).to_array(),
                forward: camera.forward().extend(0.0).to_array(),
                fog: [fog[0], fog[1], fog[2], optical.absorption_distance()],
                params: [
                    optical.absorption_distance() * optical.depth_darkening,
                    optical.refraction,
                    optical.caustics,
                    0.0,
                ],
                minimum: [[0.0; 4]; 8],
                maximum: [[0.0; 4]; 8],
            };
            for (i, volume) in self.water_boxes.iter().enumerate() {
                let [x, y, z] = volume.minimum;
                uniform.minimum[i] = [
                    x,
                    y,
                    z,
                    if optical.masks { volume.mask_lane() } else { 0.0 },
                ];
                let [x, y, z] = volume.maximum;
                uniform.maximum[i] = [x, y, z, 0.0];
            }
            uniform.params[3] = self.water_boxes.len() as f32;
            self.queue
                .write_buffer(&self.ocean_optics.uniform, 0, bytemuck::bytes_of(&uniform));
            optical_volumes = Some(uniform);
        }
        let cloud_view_proj = if self.clouds_enabled {
            camera.view_projection(self.config.width, self.config.height)
        } else {
            Mat4::IDENTITY
        };
        let cloud_inv_view_proj = if self.clouds_enabled {
            cloud_view_proj.inverse()
        } else {
            Mat4::IDENTITY
        };
        let cloud_prev_view_proj = if self.clouds_enabled && self.cloud_history_valid {
            self.previous_cloud_view_proj
        } else {
            cloud_view_proj
        };
        let motion_prev_view_proj =
            motion_reference_camera.view_projection(self.config.width, self.config.height);
        let motion_blur_scale =
            if self.motion_blur_strength > 0.001 && motion_camera_sample_age <= (1.0 / 60.0) {
                // A conventional 180-degree shutter at 60 Hz is ~1/120 s. Scale
                // camera-snapshot displacement to that exposure, then decay after
                // the last camera update so stopping the mouse doesn't leave a smear.
                let exposure = 1.0 / 120.0;
                let sample_dt = motion_camera_sample_dt.clamp(1.0 / 10_000.0, 0.05);
                let fade = (1.0 - motion_camera_sample_age / (1.0 / 60.0)).clamp(0.0, 1.0);
                (self.motion_blur_strength * exposure / sample_dt * fade).clamp(0.0, 32.0)
            } else {
                0.0
            };
        self.motion_blur_runtime_scale = motion_blur_scale;
        let inv_view_proj = if plan.use_post && !plan.gamma_only_post {
            view_proj.inverse()
        } else {
            Mat4::IDENTITY
        };
        let camera_pos_time = [
            camera.position.x,
            camera.position.y,
            camera.position.z,
            frame_time,
        ];
        let unjittered_view_proj = camera.view_projection(self.config.width, self.config.height);
        let crosshair_world = late_view_sample
            .map(|measurement| measurement.crosshair_world)
            .unwrap_or(dynamic_crosshair_world);
        self.rebuild_player_name_tail(unjittered_view_proj, crosshair_world);
        let previous_unjittered_view_proj = if self.camera_history_valid {
            self.previous_unjittered_view_proj
        } else {
            unjittered_view_proj
        };
        let uniform = CameraUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos_time,
            clip_plane: [0.0; 4],
            render_flags: [
                0,
                (u32::from(self.static_bsp_ao_enabled) | (u32::from(self.sky_fractal_override) << 1)),
                self.classic_world_render_flags(),
                if self.temporal_resolve_active() {
                    self.taa_frame_index
                } else if self.rt_local_radiance_wanted() {
                    self.ray_traced_shadows.as_ref().map_or(0, |rt| rt.sun_shadow_history.sampling_frame())
                } else {
                    0
                },
            ],
            camera_forward: camera.forward().extend(0.0).to_array(),
            unjittered_view_proj: unjittered_view_proj.to_cols_array_2d(),
            previous_unjittered_view_proj: previous_unjittered_view_proj.to_cols_array_2d(),
            jump_shade: self.jump_shade.uniform(),
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        // `misc_skyportal`: the same view re-anchored at the sky camera. Only the
        // translation differs from the main view, so shifting the (possibly
        // jittered) main matrix keeps TAA jitter and projection identical.
        let sky_portal_view =
            self.world
                .as_ref()
                .and_then(|world| world.sky_portal)
                .map(|portal| {
                    let position =
                        Vec3::from_array(portal.camera_position(camera.position.to_array()));
                    let shift = Mat4::from_translation(camera.position - position);
                    let portal_view_proj = view_proj * shift;
                    let portal_unjittered = (unjittered_view_proj * shift).to_cols_array_2d();
                    let portal_uniform = CameraUniform {
                        view_proj: portal_view_proj.to_cols_array_2d(),
                        camera_pos_time: [position.x, position.y, position.z, frame_time],
                        previous_unjittered_view_proj: portal_unjittered,
                        unjittered_view_proj: portal_unjittered,
                        jump_shade: [0.0; 4],
                        ..uniform
                    };
                    self.queue.write_buffer(
                        &self.sky_portal_camera_buffer,
                        0,
                        bytemuck::bytes_of(&portal_uniform),
                    );
                    (position, portal_view_proj)
                });
        self.ensure_wireframe_pipelines();
        self.update_sun_ray_preview(camera, player_position);
        self.prepare_rt_sun_shadow(view_proj, camera.position);
        if self.weather.rain.enabled {
            self.prepare_rain_frame(camera, frame_time);
        }
        if self.weather.snow.enabled {
            self.prepare_snow_frame(camera, frame_time);
        }
        if self.weather.atmosphere.active() {
            self.weather
                .atmosphere
                .ensure_pipeline(&mut self.pipeline_jobs, &self.device);
            self.weather
                .atmosphere
                .prepare_frame(&self.queue, frame_time, self.weather_wind);
        }
        if self.weather.wind_visuals.active() {
            self.prepare_wind_visuals_frame(frame_time);
        }
        if update_weather_surface {
            self.update_weather_surface_uniform();
        }
        let previous_view_proj =
            if !plan.gamma_only_post && plan.use_post && self.camera_history_valid {
                self.previous_view_proj
            } else {
                view_proj
            };
        if plan.use_post && !plan.gamma_only_post {
            self.write_post_uniform(
                view_proj,
                inv_view_proj,
                cloud_inv_view_proj,
                cloud_prev_view_proj,
                previous_view_proj,
                motion_prev_view_proj,
                motion_blur_scale,
                camera_pos_time,
                self.temporal_resolve_active() && self.history_valid
                    && (!self.rt_taa_active() || camera.position.distance(self.previous_camera_position) <= 96.0),
            );
        }
        if self.ssao_enabled {
            self.write_ssao_temporal_uniform(view_proj, inv_view_proj, camera_pos_time);
        }
        if self.ssr_enabled {
            self.write_ssr_temporal_uniform(view_proj, inv_view_proj, camera_pos_time);
        }

        let prep_shadow_started = Instant::now();
        let prep_camera_ms = prep_shadow_started
            .duration_since(frame_started)
            .as_secs_f64()
            * 1000.0;
        let entity_shadow_frame = (self.cascaded_shadow_mode == DynamicShadowsMode::EntityMap)
            .then(|| self.prepare_entity_shadow(camera, player_position, frame_delta));
        let shadow_matrices = if entity_shadow_frame.is_some() {
            // No cascades: the map is entity-only, so froxel fog and the BSP/grass
            // caster loop below correctly see "no cascade matrices". The receiver
            // uniform is written after dynamic models are prepared, because
            // whether anything casts decides if receivers sample the map at all.
            None
        } else if self.cascaded_shadows_enabled {
            let sun = self.active_sun();
            let (matrices, splits, texel_sizes) = bevy_cascade_shadow_matrices(
                camera,
                self.config.width,
                self.config.height,
                Vec3::from_array(sun.direction),
            );
            self.write_shadow_uniforms(
                &matrices,
                &splits,
                &texel_sizes,
                true,
                camera.forward(),
                camera_pos_time,
                true,
            );
            Some(matrices)
        } else if self.cascaded_shadow_mode == DynamicShadowsMode::RayTraced
            && self.ray_traced_shadows.is_some()
        {
            // RT uses the same authoritative map sun direction/intensity data as
            // CSM, but does not render or sample any cascade shadow maps.
            self.write_shadow_uniforms(
                &[Mat4::IDENTITY; SHADOW_CASCADES],
                &[0.0; SHADOW_CASCADES],
                &[0.0; SHADOW_CASCADES],
                true,
                camera.forward(),
                camera_pos_time,
                false,
            );
            None
        } else {
            None
        };
        if self.weather.fog.volumetric_effective()
            || (self.weather.rain.enabled && !self.rain_uses_2d_haze())
            || self.weather.snow.enabled
        {
            self.write_froxel_uniform(inv_view_proj, camera_pos_time, shadow_matrices.as_ref());
        }

        let prep_world_started = Instant::now();
        let mut prep_shadow_ms = prep_world_started
            .duration_since(prep_shadow_started)
            .as_secs_f64()
            * 1000.0;
        // One relaxed check per frame; the disabled path does no per-call work.
        let capture_api_calls = crate::renderer::diagnostics::perf_capture::enabled();
        let mut info = FrameInfo {
            world_path: "unified",
            prep_camera_ms,
            ..FrameInfo::default()
        };
        info.late_latch = late_view_sample;
        info.gpu_ms = self.gpu_profiler.latest_frame_ms();
        if self.cull_debug_mode != CullDebugMode::Off {
            info.cull_visible = self.cull_diagnostics_readback.latest[0];
            info.cull_frustum_rejected = self.cull_diagnostics_readback.latest[1];
            info.cull_hiz_rejected = self.cull_diagnostics_readback.latest[2];
        }
        let mut active_selection_changed = false;
        let mut prepared_grass = None;
        let mut prepared_surface_sprite_effects = None;
        let snow_deform_center = self.surface_deformation.field_center();
        if let Some(world) = &mut self.world {
            // Shell fragments are discarded unless footprints are in 3D mode, so
            // outside it there is nothing to build, hold or draw.
            if self.surface_deformation.mode() == FootprintMode::ThreeD {
                world.snow_shell.stream(&self.device, snow_deform_center);
            } else {
                world.snow_shell.release();
            }
            update_visibility(world, camera);
            let auto4_area_mask = effective_area_mask(world, self.pvs_mode, area_mask).copied();
            refresh_auto4_lazy_collapse(
                &self.queue,
                world,
                self.pvs_mode,
                auto4_area_mask,
                self.cull_debug_mode != CullDebugMode::Off,
            );
            let active = active_batch_selection(world, self.pvs_mode);
            let effective_area_mask = effective_area_mask(world, self.pvs_mode, area_mask);
            if self.cull_debug_mode != CullDebugMode::Off {
                info.cull_pvs_rejected =
                    u32::try_from(active.candidate_count.saturating_sub(active.indices.len()))
                        .unwrap_or(u32::MAX);
                info.cull_area_rejected = u32::try_from(
                    active
                        .indices
                        .iter()
                        .filter(|&&batch_index| {
                            !batch_area_visible(&active.batches[batch_index], effective_area_mask)
                        })
                        .count(),
                )
                .unwrap_or(u32::MAX);
            }
            if self.gpu_driven_enabled || self.cull_debug_mode != CullDebugMode::Off {
                active_selection_changed = update_active_cull_indices(
                    &self.queue,
                    world,
                    self.pvs_mode,
                    area_mask,
                    self.cull_debug_mode != CullDebugMode::Off,
                );
            }
            if self.grass_enabled {
                let pvs_cluster = world.last_cluster.flatten();
                if let Some(grass) = world.grass.as_ref() {
                    prepared_grass = Some(self.grass_renderer.prepare_draw(
                        &mut self.pipeline_jobs,
                        &self.device,
                        &self.queue,
                        grass,
                        camera.position,
                        player_position,
                        view_proj,
                        pvs_cluster,
                        frame_time,
                        self.grass_precompute_enabled,
                        self.grass_mid_lod_enabled,
                        self.grass_front_to_back_enabled,
                    ));
                }
            }
            if !world.surface_sprite_effects.is_empty() {
                let pvs_cluster = if matches!(self.pvs_mode, PvsMode::Off) {
                    None
                } else {
                    world.last_cluster.flatten()
                };
                prepared_surface_sprite_effects =
                    Some(self.surface_sprite_effect_renderer.prepare_draw(
                        &mut self.pipeline_jobs,
                        &self.device,
                        &self.queue,
                        &world.surface_sprite_effects,
                        camera,
                        pvs_cluster,
                        frame_time * 1000.0,
                    ));
            }
        }
        if active_selection_changed {
            self.update_gpu_cull_settings();
        }
        let prep_reflection_started = Instant::now();
        info.prep_world_ms = prep_reflection_started
            .duration_since(prep_world_started)
            .as_secs_f64()
            * 1000.0;

        let planar_views = if self.planar_reflection.active {
            self.world
                .as_ref()
                .map_or([None; PLANAR_REFLECTION_SLOTS], |world| {
                    select_planar_reflection_views(
                        world,
                        camera,
                        view_proj,
                        self.pvs_mode,
                        area_mask,
                        self.planar_reflection_mode,
                        self.reflection_quality,
                        render_ocean,
                        &self.planar_slot_history,
                    )
                })
        } else {
            [None; PLANAR_REFLECTION_SLOTS]
        };
        for (slot, planar) in planar_views.iter().enumerate() {
            if self.planar_slot_history[slot] != planar.map(|view| view.plane) {
                self.world_flares_planar[slot].invalidate_history();
            }
        }
        self.planar_slot_history = planar_views.map(|view| view.map(|view| view.plane));

        if self.planar_reflection_debug_mode != PlanarReflectionDebugMode::Off {
            let selected: Vec<usize> = planar_views
                .iter()
                .flatten()
                .map(|view| view.coarse_batch_index)
                .collect();
            if self.last_planar_debug_selection.as_ref() != Some(&selected) {
                if selected.is_empty() {
                    println!("[PLANAR DEBUG] no eligible visible reflector selected");
                } else if let Some(world) = self.world.as_ref() {
                    println!(
                        "[PLANAR DEBUG] mode={} active slots={}",
                        self.planar_reflection_debug_mode.label(),
                        selected.len()
                    );
                    for (slot, view) in planar_views.iter().enumerate() {
                        let Some(view) = view else { continue };
                        let reflector = world.planar_reflectors.iter().find(|reflector| {
                            reflector.coarse_batch_index == view.coarse_batch_index
                        });
                        let source = world.coarse_batches.get(view.coarse_batch_index);
                        if let Some(reflector) = reflector {
                            println!(
                                "[PLANAR DEBUG] slot={} selected {:?} batch={} plane=[{:.4}, {:.4}, {:.4}, {:.2}] area={:.1} cluster={:?} tcGen={:?} authored={} env_candidate={} texture={:?} bounds=[{:.1},{:.1},{:.1}]..[{:.1},{:.1},{:.1}]",
                                slot,
                                view.kind,
                                view.coarse_batch_index,
                                view.plane[0],
                                view.plane[1],
                                view.plane[2],
                                view.plane[3],
                                reflector.area,
                                view.cluster,
                                source.map(|batch| batch.source.tc_gen),
                                source.is_some_and(|batch| batch.source.planar_reflection),
                                source.is_some_and(|batch| batch.source.planar_environment_candidate),
                                source.and_then(|batch| batch.source.texture),
                                reflector.bounds_min[0],
                                reflector.bounds_min[1],
                                reflector.bounds_min[2],
                                reflector.bounds_max[0],
                                reflector.bounds_max[1],
                                reflector.bounds_max[2],
                            );
                        }
                    }
                }
                self.last_planar_debug_selection = Some(selected);
            }
        } else {
            self.last_planar_debug_selection = None;
        }

        if self.planar_reflection.active {
            let mut planes = [[0.0; 4]; PLANAR_REFLECTION_SLOTS];
            let mut active_count = 0usize;
            for (slot, view) in planar_views.iter().enumerate() {
                if let Some(view) = view {
                    planes[slot] = view.plane;
                    active_count += 1;
                }
            }
            let settings = PlanarReflectionUniform {
                planes,
                // Portal fragments use their main-frame pixel coordinates to
                // address the downscaled reflection texture array.
                viewport: [
                    self.config.width.max(1) as f32,
                    self.config.height.max(1) as f32,
                    active_count as f32,
                    if self.planar_reflection_mode == PlanarReflectionMode::Environment {
                        1.0
                    } else {
                        0.0
                    },
                ],
                debug: [
                    self.planar_reflection_debug_mode.shader_value(),
                    if self.reflection_debug_enabled {
                        1.0
                    } else {
                        0.0
                    },
                    self.reflection_quality.shader_value() as f32,
                    0.0,
                ],
            };
            self.queue.write_buffer(
                &self.planar_reflection.uniform_buffer,
                0,
                bytemuck::bytes_of(&settings),
            );
            for (slot, view) in planar_views.iter().enumerate() {
                let Some(view) = view else { continue };
                let reflected_camera = CameraUniform {
                    view_proj: view.view_proj.to_cols_array_2d(),
                    camera_pos_time: [
                        view.camera_position.x,
                        view.camera_position.y,
                        view.camera_position.z,
                        camera_pos_time[3],
                    ],
                    clip_plane: view.plane,
                    render_flags: [
                        1,
                        (u32::from(self.static_bsp_ao_enabled) | (u32::from(self.sky_fractal_override) << 1)),
                        self.classic_world_render_flags(),
                        0,
                    ],
                    camera_forward: view.camera_forward.extend(0.0).to_array(),
                    unjittered_view_proj: view.view_proj.to_cols_array_2d(),
                    previous_unjittered_view_proj: view.view_proj.to_cols_array_2d(),
                    jump_shade: [0.0; 4],
                };
                self.queue.write_buffer(
                    &self.planar_camera_buffers[slot],
                    0,
                    bytemuck::bytes_of(&reflected_camera),
                );
            }
        } else if self.reflection_debug_enabled {
            let settings = PlanarReflectionUniform {
                planes: [[0.0; 4]; PLANAR_REFLECTION_SLOTS],
                viewport: [
                    self.config.width.max(1) as f32,
                    self.config.height.max(1) as f32,
                    0.0,
                    0.0,
                ],
                debug: [
                    self.planar_reflection_debug_mode.shader_value(),
                    1.0,
                    self.reflection_quality.shader_value() as f32,
                    0.0,
                ],
            };
            self.queue.write_buffer(
                &self.planar_reflection.uniform_buffer,
                0,
                bytemuck::bytes_of(&settings),
            );
        }

        let prep_local_shadow_started = Instant::now();
        info.prep_reflection_ms = prep_local_shadow_started
            .duration_since(prep_reflection_started)
            .as_secs_f64()
            * 1000.0;
        self.update_local_shadow_selection(camera);
        self.ensure_local_shadow_cache();
        prep_shadow_ms += prep_local_shadow_started.elapsed().as_secs_f64() * 1000.0;
        info.prep_shadow_ms = prep_shadow_ms;

        // Prepare dynamic geometry before command encoding. With RT Shadows enabled,
        // Ghoul2 is compute-skinned once into the shared raster/BLAS buffer; the
        // ordinary RT-off vertex-shader skinning path remains unchanged.
        let dynamic_prepare_started = Instant::now();
        self.ensure_rt_model_receivers();
        self.dynamic_model_renderer.entity_sun_relight = self.entity_sun_relight();
        self.sync_entity_cloud_shadow_sun();
        let ray_traced_shadows = self.hardware_rt_active();
        // Mirrors the draw_phase calls below: the RT receivers (which add these
        // lights per pixel) are bound only in the ocean-split branch.
        let rt_model_receivers = optical_volumes.is_some()
            && ray_traced_shadows
            && self.dynamic_lights_mode == DynamicLightsMode::RayTracedHardware
            && self
                .ray_traced_shadows
                .as_ref()
                .is_some_and(|rt| rt.model_receivers.is_some());
        self.sync_entity_dlights(rt_model_receivers);
        let entity_light_grid = self
            .world
            .as_ref()
            .and_then(|world| world.entity_light_grid.as_ref());
        let split_dynamic_wireframe = self.wireframe_requires_dynamic_class_split();
        self.dynamic_model_renderer.prepare(
            &mut self.baked_brightness,
            &mut self.pipeline_jobs,
            &self.device,
            &self.queue,
            dynamic_models,
            entity_light_grid,
            split_dynamic_wireframe,
            ray_traced_shadows,
            ray_traced_shadows && self.dynamic_lights_mode == DynamicLightsMode::RayTracedHardware,
            self.glow_mode.authored(),
        );
        self.ensure_prepared_wireframe_pipelines(
            prepared_grass.as_ref(),
            prepared_surface_sprite_effects.as_ref(),
        );
        info.dynamic_model_prepare_ms = dynamic_prepare_started.elapsed().as_secs_f64() * 1000.0;
        let (ghoul2_gpu_instances, ghoul2_gpu_draw_calls) =
            self.dynamic_model_renderer.ghoul2_draw_stats();
        info.ghoul2_gpu_instances = ghoul2_gpu_instances;
        info.ghoul2_gpu_draw_calls = ghoul2_gpu_draw_calls;
        info.dynamic_model_surfaces = u32::try_from(dynamic_models.len()).unwrap_or(u32::MAX);
        info.dynamic_model_vertices = dynamic_models
            .iter()
            .map(|surface| surface.vertex_count() as u64)
            .sum();
        info.dynamic_model_indices = dynamic_models
            .iter()
            .map(|surface| surface.index_count() as u64)
            .sum();

        info.cpu_prepare_ms = frame_started.elapsed().as_secs_f64() * 1000.0;
        // One-shot diagnostic for CurrentSurfaceTexture::Validation. Wgpu's
        // Validation variant means a validation error occurred inside
        // Surface::get_current_texture(). Capture that exact call in a validation
        // scope so we can print the real error rather than guessing from the
        // generic surface status. This path is completely dormant unless
        // JKA_SURFACE_DIAG=1 is set.
        let surface_scope = self
            .surface_diag_pending
            .then(|| self.device.push_error_scope(wgpu::ErrorFilter::Validation));
        if self.surface_diag_pending {
            eprintln!(
                "[JKA SURFACE DIAG] before acquire: config={}x{} format={:?} present={:?} latency={} usage={:?} alpha={:?} view_formats={:?}",
                self.config.width,
                self.config.height,
                self.config.format,
                self.config.present_mode,
                self.config.desired_maximum_frame_latency,
                self.config.usage,
                self.config.alpha_mode,
                self.config.view_formats,
            );
        }
        let acquire_started = Instant::now();
        let surface_result = self.surface.get_current_texture();
        info.cpu_acquire_ms = acquire_started.elapsed().as_secs_f64() * 1000.0;
        if let Some(scope) = surface_scope {
            let scoped_error = block_on(scope.pop());
            match scoped_error {
                Some(error) => {
                    eprintln!("[JKA SURFACE DIAG] get_current_texture validation: {error}");
                    eprintln!("[JKA SURFACE DIAG] debug: {error:?}");
                }
                None => {
                    eprintln!("[JKA SURFACE DIAG] get_current_texture scope: no validation error captured");
                }
            }
            eprintln!("[JKA SURFACE DIAG] returned: {surface_result:?}");
            self.surface_diag_pending = false;
        }
        let frame = match surface_result {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.flush_staged_queue_writes();
                return Err(RenderError::Timeout);
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                self.flush_staged_queue_writes();
                return Err(RenderError::Occluded);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.flush_staged_queue_writes();
                return Err(RenderError::Outdated);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.flush_staged_queue_writes();
                return Err(RenderError::Lost);
            }
            wgpu::CurrentSurfaceTexture::Validation => return Err(RenderError::Validation),
        };
        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut screenshot_readback = self.prepare_screenshot_readback();

        // One-shot validation capture for the first diagnostic frame. The first
        // surface acquisition succeeds, while later acquisitions report
        // CurrentSurfaceTexture::Validation. That strongly indicates that a
        // validation error is being generated while recording/submitting the
        // preceding frame. Keep one scope around the entire first timed frame
        // so wgpu can report the exact offending API/resource.
        let frame_validation_scope = self
            .frame_diag_pending
            .then(|| self.device.push_error_scope(wgpu::ErrorFilter::Validation));
        if self.frame_diag_pending {
            eprintln!(
                "[JKA FRAME DIAG] begin first diagnostic frame: plan={:?} gpu_profiler={} config={}x{}",
                plan,
                self.gpu_profiler.enabled(),
                self.config.width,
                self.config.height,
            );
        }

        let perf_timing = crate::renderer::diagnostics::perf_capture::enabled();
        let encode_started = Instant::now();
        let perf_setup_started = perf_timing.then(Instant::now);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("JKA frame encoder"),
            });
        if let Some(started) = perf_setup_started {
            info.perf_encode_setup_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let perf_pre_world_started = perf_timing.then(Instant::now);
        self.gpu_profiler
            .write_encoder_timestamp(&mut encoder, GpuPass::Frame, false);
        // Hardware RT skinned casters are deformed once before acceleration
        // structures consume the shared skinned vertex buffer. The following
        // dynamic-caster build then reconstructs deforming BLASes/TLAS before
        // the first ray-querying world pass.
        if self.hardware_rt_active() {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RtBuild, false);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RtSkin, false);
            self.dynamic_model_renderer.encode_rt_skinning(&mut encoder);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RtSkin, true);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RtAcceleration, false);
            self.encode_ray_traced_dynamic_casters(dynamic_models, &mut encoder);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RtAcceleration, true);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RtBuild, true);
        }

        // Encode the companion POV before any long-lived post-process borrows
        // (notably SMAA) are established. It shares the prepared world/model
        // resources but has its own color/depth target and camera bind groups.
        let companion_scene_frame = companion_scene
            .and_then(|view| self.encode_companion_scene(&mut encoder, view, area_mask));

        // Set when this frame's ocean pass wrapped its draws in an occlusion
        // query, so the resolve after the last render pass knows to run.
        let mut ocean_query_encoded = false;
        let ocean_sun = self.active_sun();
        let ocean_scene_context = self.world.as_ref().map(|world| {
            (
                world.sky_average,
                world.primary_skybox.is_some(),
                world.ocean_surfaces.clone(),
                Arc::clone(&world.ocean_masks),
                world.promoted_oceans.clone(),
            )
        });
        // The FFT cascade generation and the wind-foam transport are their own
        // frame interval: they run whether or not any water is on screen, so
        // they must be measurable separately from the surface draw
        // (`GpuPass::OceanSurface`) and the underwater capture
        // (`GpuPass::OceanOptics`), both of which only cover visible water.
        // Freeze the wave field while no water has been on screen for a moment.
        // `ocean_hidden_time` is last frame's answer because the cascades are
        // encoded here, before this frame's culling; OCEAN_SIM_GRACE_SECONDS of
        // slack means turning past a pond does not stall the waves.
        const OCEAN_SIM_GRACE_SECONDS: f32 = 0.25;
        let ocean_sim_hidden = !self.ocean_settings.simulate_when_hidden
            && self.ocean_hidden_time > OCEAN_SIM_GRACE_SECONDS;
        let ocean_sim_measured =
            !ocean_sim_hidden && (self.ocean.is_some() || !self.authored_oceans.is_empty());
        if ocean_sim_measured {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::OceanSim, false);
        }
        if render_ocean && !ocean_sim_hidden && self.ocean_settings.interaction {
            if let Some((_, _, surfaces, masks, _)) = ocean_scene_context.as_ref() {
                let empty = crate::ocean::OceanSurface { plane_height: 0.0, minimum: [0.0;2], maximum: [0.0;2] };
                let mut bodies = [(empty, None); crate::ocean::OCEAN_MAX_SURFACES * 2];
                let mut body_count = 0;
                for (i, surface) in surfaces.iter().enumerate() {
                    let centre = [(surface.minimum[0]+surface.maximum[0])*0.5,
                        surface.plane_height, (surface.minimum[1]+surface.maximum[1])*0.5];
                    if !self.authored_oceans.iter().any(|(a,_)| a.contains_render_point(centre)) && body_count < bodies.len() {
                        bodies[body_count] = (*surface, Some(i)); body_count += 1;
                    }
                }
                // Authored rendering uses descriptor bounds and independent FFT
                // instances; the local interaction field is renderer-global.
                for (authored, _) in &self.authored_oceans {
                    if body_count < bodies.len() {
                        bodies[body_count] = (crate::ocean::OceanSurface { plane_height: authored.height,
                            minimum: [authored.mins[0], -authored.maxs[1]], maximum: [authored.maxs[0], -authored.mins[1]] }, None);
                        body_count += 1;
                    }
                }
                self.ocean_interaction.encode_frame(&self.queue, &mut encoder, frame_delta,
                    self.ocean_settings.updates_per_second, camera.position, &bodies[..body_count], masks);
            }
        }
        if let Some(ocean) = &mut self.ocean {
            if let Some((sky_average, has_skybox, surfaces, masks, promoted)) = ocean_scene_context.as_ref() {
                ocean.set_masks(&self.queue, masks);
                ocean.set_environment(
                    &self.queue,
                    ocean_sun.direction,
                    ocean_sun.color,
                    ocean_sun.intensity,
                    *sky_average,
                    *has_skybox,
                );
                let fallback: Vec<_> = surfaces
                    .iter()
                    .copied()
                    .filter(|s| {
                        let centre = [
                            (s.minimum[0] + s.maximum[0]) * 0.5,
                            s.plane_height,
                            (s.minimum[1] + s.maximum[1]) * 0.5,
                        ];
                        !self
                            .authored_oceans
                            .iter()
                            .any(|(a, _)| a.contains_render_point(centre))
                    })
                    .collect();
                ocean.set_surfaces(&self.queue, &fallback);
                let promoted: Vec<_> = promoted.iter().filter(|p| fallback.contains(&p.surface)).cloned().collect();
                ocean.set_promoted_surfaces(&self.device, &self.queue, &promoted, &self.promoted_ocean_overrides);
            }
            // Re-centre the clipmap before the wave step so the geometry the
            // frame draws and the field it samples come from the same tick.
            let coarsest = self
                .world
                .as_ref()
                .map(|world| world.ocean_coarsest_spacing)
                .unwrap_or(0.0);
            if coarsest > 0.0 {
                ocean.set_clipmap_center(&self.queue, camera.position.to_array(), coarsest);
            }
            if !ocean_sim_hidden {
                ocean.encode_frame(&self.queue, &mut encoder, frame_delta);
            }
        }
        for (authored, ocean) in &mut self.authored_oceans {
            if let Some((sky_average, has_skybox, _, _, _)) = ocean_scene_context.as_ref() {
                ocean.set_environment(
                    &self.queue,
                    ocean_sun.direction,
                    ocean_sun.color,
                    ocean_sun.intensity,
                    *sky_average,
                    *has_skybox,
                );
                let selected = [crate::ocean::OceanSurface {
                    plane_height: authored.height,
                    minimum: [authored.mins[0], -authored.maxs[1]],
                    maximum: [authored.maxs[0], -authored.mins[1]],
                }];
                ocean.set_surfaces(&self.queue, &selected);
            }
            let coarsest = self
                .world
                .as_ref()
                .map(|w| w.ocean_coarsest_spacing)
                .unwrap_or(0.0);
            ocean.set_clipmap_center(&self.queue, camera.position.to_array(), coarsest);
            if !ocean_sim_hidden {
                ocean.encode_frame(&self.queue, &mut encoder, frame_delta);
            }
        }
        if ocean_sim_measured {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::OceanSim, true);
        }
        info.ocean.authored_oceans =
            u32::try_from(self.authored_oceans.len()).unwrap_or(u32::MAX);
        info.ocean.promoted_surfaces = self
            .world
            .as_ref()
            .map(|world| u32::try_from(world.promoted_oceans.len()).unwrap_or(u32::MAX))
            .unwrap_or(0);
        info.ocean.sim_hidden = ocean_sim_hidden;
        info.ocean.visible_samples = self.ocean_visibility.latest_samples;
        info.ocean.visibility_primed = self.ocean_visibility.primed;

        // Grass preparation has to precede the first sun-shadow cascade because
        // nearby blades reuse the prepared wind/trample transform as a cheap
        // one-triangle shadow caster. This is the same compute work the color
        // pass already needed; it is merely scheduled earlier.
        if self.grass_enabled {
            if let (Some(world), Some(prepared)) = (&self.world, prepared_grass.as_ref()) {
                if let Some(grass) = world.grass.as_ref() {
                    self.gpu_profiler.write_encoder_timestamp(
                        &mut encoder,
                        GpuPass::GrassPrepare,
                        false,
                    );
                    self.grass_renderer.encode_prepare(
                        &mut encoder,
                        grass,
                        prepared,
                        &self.shadow_resources.receiver_bind_group,
                    );
                    self.gpu_profiler.write_encoder_timestamp(
                        &mut encoder,
                        GpuPass::GrassPrepare,
                        true,
                    );
                }
            }
        }

        if let Some(frame) = entity_shadow_frame.as_ref() {
            let casters = frame.active && self.dynamic_model_renderer.has_depth_casters();
            self.write_entity_shadow_uniforms(frame, camera, casters);
            self.encode_entity_shadow_pass(&mut encoder, frame);
        }
        let csm_entity_casters = match shadow_matrices.as_ref() {
            Some(matrices) => self.prepare_entity_cascade_casters(matrices),
            None => false,
        };

        if let Some(shadow_matrices) = shadow_matrices.as_ref() {
            if let Some(world) = &self.world {
                for cascade in 0..SHADOW_CASCADES {
                    if self.sun_visibility != SunVisibilityMode::Legacy {
                        if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                        let mut sky_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("JKA shader-sun sky admission"),
                            color_attachments: &[],
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &self.shadow_resources.sky_layer_views[cascade],
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(0.0),
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        sky_pass.set_pipeline(&self.shadow_resources.bevy_sky_pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        sky_pass.set_bind_group(
                            0,
                            &self.shadow_resources.caster_bind_groups[cascade],
                            &[],
                        );
                        if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                        sky_pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                        if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                        sky_pass
                            .set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                        if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                        sky_pass.set_index_buffer(
                            world.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        for batch in &world.coarse_batches {
                            // Cloud-layer batches share the outer box's geometry;
                            // one admission draw per sky surface is enough.
                            if batch.source.pipeline.class != DrawClass::Sky
                                || matches!(batch.source.tc_gen, TcGen::SkyCloud(_))
                            {
                                continue;
                            }
                            if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                            draw_world_batch(&mut sky_pass, batch, 0..1, None);
                        }
                    }

                    if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("JKA cascaded sun shadow"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.shadow_resources.layer_views[cascade],
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(&self.shadow_resources.bevy_pipeline);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, &self.shadow_resources.caster_bind_groups[cascade], &[]);
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                    pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    let mut mask_pipeline_active = false;
                    for batch in &world.coarse_batches {
                        if self.tinted_glass_enabled && crate::renderer::lighting::glass_transmission::classify_glass(
                            &batch.source,
                            batch.source.texture.and_then(|index| world.glass_texture_names.get(index)).map(String::as_str),
                            None, true,
                        ).is_some() { continue; }
                        if (batch.source.pipeline.blend != BlendMode::Opaque
                            && self.sun_visibility != SunVisibilityMode::Filtered)
                            || !aabb_intersects_shadow_frustum(
                                batch.bounds_min,
                                batch.bounds_max,
                                shadow_matrices[cascade],
                                true,
                            )
                        {
                            continue;
                        }
                        match batch.source.pipeline.class {
                            DrawClass::Opaque if batch.source.alpha_cutoff <= 0.0 => {
                                if mask_pipeline_active {
                                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                    pass.set_pipeline(&self.shadow_resources.bevy_pipeline);
                                    mask_pipeline_active = false;
                                }
                                if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                                draw_world_batch(&mut pass, batch, 0..1, None);
                            }
                            DrawClass::Mask if batch.source.alpha_cutoff > 0.0 => {
                                if !mask_pipeline_active {
                                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                    pass.set_pipeline(&self.shadow_resources.bevy_mask_pipeline);
                                    mask_pipeline_active = true;
                                }
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &batch.bind_group, &[]);
                                if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                                draw_world_batch(&mut pass, batch, 0..1, None);
                            }
                            DrawClass::Transparent
                                if self.sun_visibility == SunVisibilityMode::Filtered =>
                            {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(&self.shadow_resources.bevy_translucent_pipeline);
                                mask_pipeline_active = true;
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &batch.bind_group, &[]);
                                if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                                draw_world_batch(&mut pass, batch, 0..1, None);
                            }
                            _ => {}
                        }
                    }

                    // The source GodotGrass demo enables full grass shadow mapping,
                    // but explicitly notes its high cost. Only the closest grass
                    // contributes here, through the existing first sun cascade and
                    // with the source one-triangle low blade as a depth-only proxy.
                    if cascade == 0 && self.grass_enabled {
                        if let (Some(grass), Some(prepared)) =
                            (world.grass.as_ref(), prepared_grass.as_ref())
                        {
                            self.grass_renderer.draw_shadow(
                                &mut pass,
                                &self.shadow_resources.caster_bind_groups[cascade],
                                grass,
                                prepared,
                                true,
                            );
                        }
                    }

                    // Players, NPCs and models cast into the near/mid cascades too.
                    if csm_entity_casters && cascade < ENTITY_SHADOW_CAMERAS {
                        self.dynamic_model_renderer.draw_entity_shadow(
                            &mut pass,
                            &self.entity_shadow.cameras[cascade].1,
                            true,
                        );
                    }
                }
            }
        }

        if self.weather.rain.enabled && !self.weather.rain.frozen {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RainSim, false);
        }
        self.weather.rain.dispatch_simulation(&mut encoder);
        if self.weather.rain.enabled && !self.weather.rain.frozen {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RainSim, true);
        }
        self.weather.snow.dispatch_simulation(&mut encoder);

        if plan.needs_linear_depth {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::Depth, false);
            let lean_rain_depth = plan.rain_only_linear_depth && self.weather.rain.lean_depth;
            let lean_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.targets.linear_depth_render_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 1_000_000.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let full_attachments = [
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.linear_depth_render_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1_000_000.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.motion_vector_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.reflection_mask_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ];
            let color_attachments = if lean_rain_depth {
                &lean_attachments[..]
            } else {
                &full_attachments[..]
            };
            if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if lean_rain_depth {
                    "JKA rain lean depth prepass"
                } else {
                    "JKA AO depth prepass"
                }),
                color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.ao_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(camera_depth_clear()),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
            pass.set_pipeline(if lean_rain_depth {
                &self.rain_depth_prepass_pipeline
            } else {
                &self.depth_prepass_pipeline
            });
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            // The reflection-policy attachment records whether this particular
            // surface actually won a planar slot this frame. Surfaces that were
            // merely eligible but lost the budget remain free to fall back to SSR.
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(2, &self.planar_reflection.bind_group, &[]);
            if let Some(world) = &self.world {
                let active = active_batch_selection(world, self.pvs_mode);
                let effective_area_mask = effective_area_mask(world, self.pvs_mode, area_mask);
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                let mut mask_pipeline_active = false;
                for &batch_index in active.indices {
                    let batch = &active.batches[batch_index];
                    if !batch_area_visible(batch, effective_area_mask) {
                        continue;
                    }
                    if ocean_suppresses_batch(render_ocean, batch) {
                        continue;
                    }
                    if batch.source.pipeline.class != DrawClass::Sky
                        && !aabb_intersects_clip_frustum(
                            batch.bounds_min,
                            batch.bounds_max,
                            view_proj,
                        )
                    {
                        continue;
                    }
                    match batch.source.pipeline.class {
                        DrawClass::Opaque => {
                            if mask_pipeline_active {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(if lean_rain_depth {
                                    &self.rain_depth_prepass_pipeline
                                } else {
                                    &self.depth_prepass_pipeline
                                });
                                mask_pipeline_active = false;
                            }
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(1, &batch.bind_group, &[]);
                            if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                            draw_world_batch(&mut pass, batch, 0..1, None);
                        }
                        DrawClass::Mask if batch.source.alpha_cutoff > 0.0 => {
                            if plan.rain_only_linear_depth && self.weather.rain.opaque_depth {
                                continue;
                            }
                            if !mask_pipeline_active {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(if lean_rain_depth {
                                    &self.rain_depth_prepass_mask_pipeline
                                } else {
                                    &self.depth_prepass_mask_pipeline
                                });
                                mask_pipeline_active = true;
                            }
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(1, &batch.bind_group, &[]);
                            if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                            draw_world_batch(&mut pass, batch, 0..1, None);
                        }
                        _ => {}
                    }
                }
            }
            // Players, vehicles and other depth-writing entities. Without them
            // every post consumer saw the BSP behind an entity, which washed
            // third-person players in background fog.
            if lean_rain_depth {
                if self.weather.rain.lean_models && !self.weather.rain.static_depth {
                    self.dynamic_model_renderer
                        .draw_rain_prepass(&mut pass, &self.camera_bind_group);
                }
            } else if !(plan.rain_only_linear_depth && self.weather.rain.static_depth) {
                self.dynamic_model_renderer
                    .draw_prepass(&mut pass, &self.camera_bind_group);
            }
            drop(pass);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::Depth, true);
        }

        let hiz_world_active = self
            .world
            .as_ref()
            .is_some_and(|world| world.active_cull_count > 0 && world.cull_bind_group.is_some());

        // Bevy-style early phase. Hi-Z is the only depth consumer, so instead
        // of the full prepass draw just last frame's visible batches (a
        // compute pass turns the visibility `cs_main` left in `instance_count`
        // into an indirect draw list) into the depth buffer, depth-only. The
        // pyramid built from that is a conservative subset of the true depth,
        // and everything the main-pass cull then keeps is drawn by the main
        // pass itself; there is no late prepass because nothing reads it.
        if plan.hiz_early && hiz_world_active {
            if let Some(world) = &self.world {
                if let Some(cull_bind_group) = &world.cull_bind_group {
                    self.gpu_profiler
                        .write_encoder_timestamp(&mut encoder, GpuPass::Depth, false);
                    {
                        if capture_api_calls { info.perf_encode_calls.compute_passes = info.perf_encode_calls.compute_passes.saturating_add(1); }
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("JKA Hi-Z early draw list"),
                            timestamp_writes: None,
                        });
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(&self.gpu_cull_early_pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(0, &self.camera_bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(1, cull_bind_group, &[]);
                        pass.dispatch_workgroups(world.active_cull_count.div_ceil(64), 1, 1);
                    }
                    if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("JKA Hi-Z early depth"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.targets.ao_depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(camera_depth_clear()),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                    pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    let active = active_batch_selection(world, self.pvs_mode);
                    let effective_area_mask = effective_area_mask(world, self.pvs_mode, area_mask);
                    let mut current_is_mask: Option<bool> = None;
                    for &batch_index in active.indices {
                        let batch = &active.batches[batch_index];
                        if !batch_area_visible(batch, effective_area_mask) {
                            continue;
                        }
                        if ocean_suppresses_batch(render_ocean, batch) {
                            continue;
                        }
                        if !aabb_intersects_clip_frustum(
                            batch.bounds_min,
                            batch.bounds_max,
                            view_proj,
                        ) {
                            continue;
                        }
                        let is_mask = match batch.source.pipeline.class {
                            DrawClass::Opaque => false,
                            DrawClass::Mask if batch.source.alpha_cutoff > 0.0 => true,
                            _ => continue,
                        };
                        if current_is_mask != Some(is_mask) {
                            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                            pass.set_pipeline(if is_mask {
                                &self.hiz_prepass_mask_pipeline
                            } else {
                                &self.hiz_prepass_pipeline
                            });
                            current_is_mask = Some(is_mask);
                        }
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(1, &batch.bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.draw_indexed_indirect = info.perf_encode_calls.draw_indexed_indirect.saturating_add(1); }
                        pass.draw_indexed_indirect(
                            &world.early_indirect_buffer,
                            u64::from(batch.cull_index)
                                * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                        );
                    }
                    drop(pass);
                    self.gpu_profiler
                        .write_encoder_timestamp(&mut encoder, GpuPass::Depth, true);
                }
            }
        }

        if plan.use_hiz && hiz_world_active {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::HiZ, false);
            {
                if capture_api_calls { info.perf_encode_calls.compute_passes = info.perf_encode_calls.compute_passes.saturating_add(1); }
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("JKA Hi-Z build"),
                    timestamp_writes: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(&self.hiz_build_pipeline);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, &self.hiz_build_bind_group, &[]);
                pass.dispatch_workgroups(
                    self.targets.hiz_width.div_ceil(8),
                    self.targets.hiz_height.div_ceil(8),
                    1,
                );
            }
            for (mip_index, bind_group) in self.hiz_reduce_bind_groups.iter().enumerate() {
                let destination_mip = u32::try_from(mip_index + 1).unwrap_or(u32::MAX);
                let width = (self.targets.hiz_width >> destination_mip).max(1);
                let height = (self.targets.hiz_height >> destination_mip).max(1);
                if capture_api_calls { info.perf_encode_calls.compute_passes = info.perf_encode_calls.compute_passes.saturating_add(1); }
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("JKA Hi-Z reduce"),
                    timestamp_writes: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(&self.hiz_reduce_pipeline);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, bind_group, &[]);
                pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
            }
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::HiZ, true);
        }

        if plan.use_gpu_culling {
            if let Some(world) = &self.world {
                let debug_culling = self.cull_debug_mode != CullDebugMode::Off;
                if debug_culling {
                    encoder.clear_buffer(&world.cull_debug_count_buffer, 0, None);
                }
                if world.active_cull_count > 0 {
                    if let Some(cull_bind_group) = &world.cull_bind_group {
                        if self.gpu_compaction_supported && !world.compact_groups.is_empty() {
                            encoder.clear_buffer(&world.compact_count_buffer, 0, None);
                        }
                        self.gpu_profiler.write_encoder_timestamp(
                            &mut encoder,
                            GpuPass::Cull,
                            false,
                        );
                        {
                            if capture_api_calls { info.perf_encode_calls.compute_passes = info.perf_encode_calls.compute_passes.saturating_add(1); }
                            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("JKA GPU world culling"),
                                timestamp_writes: None,
                            });
                            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                            pass.set_pipeline(&self.gpu_cull_pipeline);
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(0, &self.camera_bind_group, &[]);
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(1, cull_bind_group, &[]);
                            pass.dispatch_workgroups(world.active_cull_count.div_ceil(64), 1, 1);
                        }
                        self.gpu_profiler.write_encoder_timestamp(
                            &mut encoder,
                            GpuPass::Cull,
                            true,
                        );
                    }
                }
                if debug_culling {
                    self.cull_diagnostics_readback
                        .encode_copy(&mut encoder, &world.cull_debug_count_buffer);
                }
            }
        }

        if plan.use_clustered_lighting {
            if let Some(world) = &self.world {
                if self.active_light_count() > 0 {
                    self.gpu_profiler.write_encoder_timestamp(
                        &mut encoder,
                        GpuPass::Cluster,
                        false,
                    );
                    if capture_api_calls { info.perf_encode_calls.compute_passes = info.perf_encode_calls.compute_passes.saturating_add(1); }
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("JKA clustered-light build"),
                        timestamp_writes: None,
                    });
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(&self.cluster_compute_pipeline);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(1, &world.cluster_compute_bind_group, &[]);
                    pass.dispatch_workgroups(CLUSTER_COUNT.div_ceil(64), 1, 1);
                    drop(pass);
                    self.gpu_profiler
                        .write_encoder_timestamp(&mut encoder, GpuPass::Cluster, true);
                }
            }
        }

        self.encode_rt_sun_shadow(&mut encoder);

        if self.weather.fog.volumetric_effective()
            || (self.weather.rain.enabled && !self.rain_uses_2d_haze())
            || self.weather.snow.enabled
        {
            if let Some(world) = &self.world {
                if let Some(bind_group) = &world.froxel_bind_group {
                    self.gpu_profiler
                        .write_encoder_timestamp(&mut encoder, GpuPass::WeatherFog, false);
                    if capture_api_calls { info.perf_encode_calls.compute_passes = info.perf_encode_calls.compute_passes.saturating_add(1); }
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("JKA froxel volumetric fog build"),
                        timestamp_writes: None,
                    });
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(&self.weather.fog.resources.pipeline);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, bind_group, &[]);
                    pass.dispatch_workgroups(
                        weather::FROXEL_X.div_ceil(8),
                        weather::FROXEL_Y.div_ceil(8),
                        1,
                    );
                    drop(pass);
                    self.gpu_profiler
                        .write_encoder_timestamp(&mut encoder, GpuPass::WeatherFog, true);
                }
            }
        }

        // Render up to four distinct visible planar views at half resolution.
        // Coplanar surfaces share one texture-array layer. Each reflection pass
        // still receives the recursion-disabled bind group, so mirrors do not
        // recursively explode when several reflective planes face each other.
        if let Some(world) = self.world.as_ref() {
            let active_pipeline_variant = &world.pipeline_variants[&world.active_pipeline_variant];
            let planar_binding_test =
                self.planar_reflection_debug_mode == PlanarReflectionDebugMode::BindingTest;
            // Same conditions as the main pass's CPU compaction: single-surface,
            // single-state opaque/masked batches in a compact group share one
            // representative bind group, so they can be drawn as one multi-draw.
            let use_reflection_compaction = !planar_binding_test
                && !render_ocean
                && self.gpu_compaction_supported
                && !world.compact_groups.is_empty();
            let use_reflection_inline_grouping =
                !planar_binding_test && !render_ocean && self.gpu_compaction_supported;
            let ocean_enabled = render_ocean;
            let mut reflection_packed = 0_u32;
            for (slot, reflection_view) in planar_views.iter().enumerate() {
                let Some(reflection_view) = reflection_view else {
                    continue;
                };
                let indices: &[usize] = if self.pvs_mode == PvsMode::Off {
                    &world.coarse_all_batches
                } else if let Some(cluster) = reflection_view.cluster {
                    world
                        .coarse_visible_batches_by_cluster
                        .get(cluster)
                        .unwrap_or(&world.coarse_all_batches)
                } else {
                    &world.coarse_all_batches
                };
                let reflection_area_mask =
                    if self.pvs_mode == PvsMode::Off || reflection_view.cluster.is_none() {
                        None
                    } else {
                        area_mask
                    };
                // Omit this reflector's own geometry. Other reflective planes
                // render through the recursion-disabled path.
                let selected_range = world.coarse_batches[reflection_view.coarse_batch_index]
                    .source
                    .vertices
                    .clone();
                let reflection_visible = |batch: &WorldBatch| {
                    let sky = batch.source.pipeline.class == DrawClass::Sky;
                    batch_area_visible(batch, reflection_area_mask)
                        && !ocean_suppresses_batch(ocean_enabled, batch)
                        // The reflection shader discards every fragment behind
                        // the mirror plane (OpenJK's portal clip plane), so a
                        // batch wholly behind it cannot contribute a pixel.
                        && (sky || !aabb_behind_plane(batch.bounds_min, batch.bounds_max, reflection_view.plane))
                        && batch.source.vertices != selected_range
                        && (sky
                            || aabb_intersects_clip_frustum(
                                batch.bounds_min,
                                batch.bounds_max,
                                reflection_view.cull_view_proj,
                            ))
                };
                let compact_group_of = |position: usize, batch: &WorldBatch| {
                    let group = batch.compact_group? as usize;
                    let same_surface_neighbour = (position > 0
                        && same_world_surface(batch, &world.coarse_batches[indices[position - 1]]))
                        || indices.get(position + 1).is_some_and(|&next| {
                            same_world_surface(batch, &world.coarse_batches[next])
                        });
                    (!batch.is_inline_entity
                        && batch.source.pipeline.class != DrawClass::Sky
                        && !same_surface_neighbour)
                        .then_some(group)
                };
                self.reflection_compact_groups.clear();
                if use_reflection_compaction {
                    self.reflection_compact_groups
                        .resize(world.compact_groups.len(), (0, 0));
                    // Two passes keep each group's members contiguous: count,
                    // assign packed offsets, then fill.
                    let mut members = 0_u32;
                    for (position, &batch_index) in indices.iter().enumerate() {
                        let batch = &world.coarse_batches[batch_index];
                        if let Some(group) = compact_group_of(position, batch) {
                            if reflection_visible(batch) {
                                self.reflection_compact_groups[group].1 += 1;
                                members += 1;
                            }
                        }
                    }
                    if reflection_packed.saturating_add(members) > world.reflection_compact_capacity
                    {
                        self.reflection_compact_groups.clear();
                    } else {
                        let mut next = reflection_packed;
                        for entry in &mut self.reflection_compact_groups {
                            entry.0 = next;
                            next += entry.1;
                            entry.1 = 0;
                        }
                        self.reflection_compact_scratch.clear();
                        self.reflection_compact_scratch
                            .resize(members as usize, DrawIndexedIndirectArgs::zeroed());
                        for (position, &batch_index) in indices.iter().enumerate() {
                            let batch = &world.coarse_batches[batch_index];
                            let Some(group) = compact_group_of(position, batch) else {
                                continue;
                            };
                            if !reflection_visible(batch) {
                                continue;
                            }
                            let entry = &mut self.reflection_compact_groups[group];
                            let packed = (entry.0 + entry.1 - reflection_packed) as usize;
                            self.reflection_compact_scratch[packed] = DrawIndexedIndirectArgs {
                                index_count: batch
                                    .indexed_range
                                    .end
                                    .saturating_sub(batch.indexed_range.start),
                                instance_count: 1,
                                first_index: batch.indexed_range.start,
                                base_vertex: 0,
                                first_instance: 0,
                            };
                            entry.1 += 1;
                        }
                        if members != 0 {
                            self.queue.write_buffer(
                                &world.reflection_compact_indirect_buffer,
                                u64::from(reflection_packed)
                                    * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                bytemuck::cast_slice(&self.reflection_compact_scratch),
                            );
                        }
                        reflection_packed += members;
                    }
                }
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA planar reflection"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.planar_reflection.color_views[slot],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(if planar_binding_test {
                                wgpu::Color {
                                    r: 0.0,
                                    g: 1.0,
                                    b: 0.12,
                                    a: 1.0,
                                }
                            } else {
                                self.weather.fog.clear_color()
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.planar_reflection.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(camera_depth_clear()),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, &self.planar_camera_bind_groups[slot], &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(
                    2,
                    if enhanced_world_shader {
                        &world.lighting_bind_group
                    } else {
                        &world.lighting_bind_group_lean
                    },
                    &[],
                );
                let shadow_receiver_bind_group = if world.active_pipeline_variant.ray_traced_shadows
                {
                    self.ray_traced_shadows
                        .as_ref()
                        .map(|rt| &rt.receiver_bind_group)
                        .unwrap_or(&self.shadow_resources.receiver_bind_group)
                } else {
                    &self.shadow_resources.receiver_bind_group
                };
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(3, shadow_receiver_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(4, &self.planar_reflection.reflection_pass_bind_group, &[]);
                if enhanced_world_shader {
                    let ocean_bind_group = self
                        .ocean
                        .as_ref()
                        .map(|ocean| &ocean.render_bind_group)
                        .unwrap_or(&self.ocean_inert_bind_group);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(6, ocean_bind_group, &[]);
                }
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                let mut current_pipeline = None;
                // Movers are grouped per draw state exactly like the main pass.
                let inline_grouping =
                    use_reflection_inline_grouping && world.inline_group_count != 0;
                let mut inline_visible = std::mem::take(&mut self.inline_visible_scratch);
                inline_visible.clear();
                if !planar_binding_test {
                    for (reflection_pos, &batch_index) in indices.iter().enumerate() {
                        let batch = &world.coarse_batches[batch_index];
                        if !reflection_visible(batch) {
                            continue;
                        }
                        if inline_grouping
                            && batch.is_inline_entity
                            && batch.inline_group != u32::MAX
                        {
                            inline_visible.push(batch_index);
                            continue;
                        }
                        if let Some(group) = compact_group_of(reflection_pos, batch)
                            .filter(|_| !self.reflection_compact_groups.is_empty())
                        {
                            // Members share this batch's pipeline state; the
                            // group's representative owns the shared bind group.
                            let (first, count) = self.reflection_compact_groups[group];
                            if count != 0 {
                                let compact = &world.compact_groups[group];
                                let representative = match compact.representative_set {
                                    WorldBatchSet::Coarse => {
                                        &world.coarse_batches[compact.representative_index]
                                    }
                                    WorldBatchSet::Full => {
                                        &world.full_batches[compact.representative_index]
                                    }
                                };
                                if current_pipeline != Some(representative.source.pipeline) {
                                    let Some(pipeline) = active_pipeline_variant
                                        .reflection_pipelines
                                        .get(&representative.source.pipeline)
                                    else {
                                        continue;
                                    };
                                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                    pass.set_pipeline(pipeline);
                                    current_pipeline = Some(representative.source.pipeline);
                                }
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &representative.bind_group, &[]);
                                if enhanced_world_shader {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            representative,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect = info.perf_encode_calls.multi_draw_indexed_indirect.saturating_add(1); }
                                pass.multi_draw_indexed_indirect(
                                    &world.reflection_compact_indirect_buffer,
                                    u64::from(first)
                                        * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                    count,
                                );
                                self.reflection_compact_groups[group].1 = 0;
                            }
                            continue;
                        }
                        if current_pipeline != Some(batch.source.pipeline) {
                            if let Some(pipeline) = active_pipeline_variant
                                .reflection_pipelines
                                .get(&batch.source.pipeline)
                            {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(pipeline);
                            } else {
                                continue;
                            }
                            current_pipeline = Some(batch.source.pipeline);
                        }
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(1, &batch.bind_group, &[]);
                        if enhanced_world_shader {
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(
                                6,
                                Self::ocean_bind_group_for(
                                    batch,
                                    &self.authored_oceans,
                                    &self.ocean,
                                    &self.ocean_inert_bind_group,
                                ),
                                &[],
                            );
                        }
                        if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                        draw_world_batch(&mut pass, batch, 0..1, ocean_clipmap);

                        let next_is_same_surface =
                            indices.get(reflection_pos + 1).is_some_and(|&next_index| {
                                same_world_surface(batch, &world.coarse_batches[next_index])
                            });
                        let surface_writes_depth = indices[..=reflection_pos]
                            .iter()
                            .rev()
                            .map(|&index| &world.coarse_batches[index])
                            .take_while(|sibling| same_world_surface(batch, sibling))
                            .any(|sibling| sibling.source.pipeline.depth_write);
                        let needs_fog_pass = !next_is_same_surface
                            && batch.source.pipeline.class != DrawClass::Sky
                            // GodotOcean fogs itself (bsp.wgsl apply_ocean_legacy_fog).
                            && !(render_ocean && batch.ocean_clipmap.is_some())
                            && self.weather.fog.legacy_uses_separate_pass(
                                batch.source.fog_is_global,
                                batch.source.legacy2_fog_in_stage_safe,
                                batch.source.global_fog_post_eligible,
                            )
                            && (batch.source.fog[3] > 0.001
                                || self.weather.fog.legacy_drawfog_value() == 1);
                        if needs_fog_pass {
                            if let Some(fog_pipeline) = active_pipeline_variant
                                .reflection_fog_pass_pipelines
                                .get(&(batch.source.pipeline, surface_writes_depth))
                            {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(fog_pipeline);
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &batch.bind_group, &[]);
                                if enhanced_world_shader {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            batch,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                                draw_world_batch(&mut pass, batch, 0..1, ocean_clipmap);
                                current_pipeline = None;
                            }
                        }
                    }
                }
                if !inline_visible.is_empty() {
                    let region_base = world.inline_indirect_capacity * (slot as u32 + 1);
                    let packed = pack_inline_draws(
                        &self.queue,
                        &world.coarse_batches,
                        &inline_visible,
                        &mut self.inline_draw_scratch,
                        world.inline_group_count,
                        &world.inline_indirect_buffer,
                        region_base,
                        world.inline_indirect_capacity,
                    );
                    let scratch = &self.inline_draw_scratch;
                    // Unpacked fallback: one run per batch, in list order.
                    let fallback: Vec<(u32, u32)> = if packed {
                        Vec::new()
                    } else {
                        (0..inline_visible.len() as u32)
                            .map(|position| (position, 1))
                            .collect()
                    };
                    let runs: &[(u32, u32)] = if packed { &scratch.runs } else { &fallback };
                    let fallback_kinds = if packed {
                        Vec::new()
                    } else {
                        inline_stage_info(&world.coarse_batches, &inline_visible).1
                    };
                    for (run_index, &(first, count)) in runs.iter().enumerate() {
                        let position = if packed {
                            scratch.order[first as usize]
                        } else {
                            first
                        };
                        let batch = &world.coarse_batches[inline_visible[position as usize]];
                        if current_pipeline != Some(batch.source.pipeline) {
                            let Some(pipeline) = active_pipeline_variant
                                .reflection_pipelines
                                .get(&batch.source.pipeline)
                            else {
                                continue;
                            };
                            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                            pass.set_pipeline(pipeline);
                            current_pipeline = Some(batch.source.pipeline);
                        }
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(1, &batch.bind_group, &[]);
                        if enhanced_world_shader {
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(
                                6,
                                Self::ocean_bind_group_for(
                                    batch,
                                    &self.authored_oceans,
                                    &self.ocean,
                                    &self.ocean_inert_bind_group,
                                ),
                                &[],
                            );
                        }
                        if count == 1 {
                            if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                            pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                        } else {
                            if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect = info.perf_encode_calls.multi_draw_indexed_indirect.saturating_add(1); }
                            pass.multi_draw_indexed_indirect(
                                &world.inline_indirect_buffer,
                                u64::from(region_base + first)
                                    * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                count,
                            );
                        }
                        // Legacy fog redraws every surface whose last stage is in
                        // this run, so the run repeats as one multi-draw.
                        let kind = if packed {
                            scratch.run_class[run_index] as u8 & 3
                        } else {
                            fallback_kinds[position as usize]
                        };
                        if let Some(writes_depth) =
                            inline_run_fog_depth(&self.weather.fog, render_ocean, batch, kind)
                        {
                            if let Some(fog_pipeline) = active_pipeline_variant
                                .reflection_fog_pass_pipelines
                                .get(&(batch.source.pipeline, writes_depth))
                            {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(fog_pipeline);
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &batch.bind_group, &[]);
                                if enhanced_world_shader {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            batch,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if count == 1 {
                                    if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                                    pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                                } else {
                                    if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect = info.perf_encode_calls.multi_draw_indexed_indirect.saturating_add(1); }
                                    pass.multi_draw_indexed_indirect(
                                        &world.inline_indirect_buffer,
                                        u64::from(region_base + first)
                                            * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                        count,
                                    );
                                }
                                current_pipeline = None;
                            }
                        }
                    }
                }
                self.inline_visible_scratch = inline_visible;
                drop(pass);
                // Per-view world flares use this reflection's just-rendered depth
                // before the next slot clears the shared depth attachment.
                if self.world_flares_enabled {
                    let flare_format = self.scene_format();
                    self.world_flares_planar[slot].draw_planar(
                        &self.device,
                        &self.queue,
                        &mut encoder,
                        &self.planar_reflection.color_views[slot],
                        &self.planar_reflection.depth_view,
                        flare_format,
                        reflection_view,
                        self.planar_reflection._width,
                        self.planar_reflection._height,
                        world.visibility.as_ref(),
                        self.pvs_mode != PvsMode::Off,
                        self.weather.fog.legacy_drawfog_value(),
                    );
                }
            }
        }

        // SMAA owns a single-sample color target and resolves it to the
        // swapchain with the reference three-pass SMAA 1x algorithm. Keep it
        // completely out of the frame when disabled. If no renderer post pass
        // is needed, the world can render straight into this target and avoid
        // an otherwise-pointless fullscreen copy.
        if self.smaa_enabled {
            self.ensure_smaa_target();
        }
        let direct_presentation_output = if menu_backdrop_active {
            &self
                .menu_backdrop
                .as_ref()
                .expect("menu backdrop resources must exist while active")
                .view
        } else {
            &frame_view
        };
        // Scene and Scene + HUD run CRT before the native menu/egui layers.
        // Everything with no menu can render straight into the CRT source; with
        // a menu, the backdrop first has to finish composing the scene.
        let crt_scene_target = crt_active && !(crt_everything && menu_backdrop_active);
        let presentation_output = if crt_scene_target {
            crt_view.as_ref().expect("CRT resources must exist while enabled")
        } else {
            direct_presentation_output
        };
        // Computed before the mutable `smaa_target` borrow below starts, since
        // `scene_format()` takes `&self` and the borrow lives until the frame
        // resolves much later.
        let flare_scene_format = if plan.use_post {
            self.scene_format()
        } else {
            self.config.format
        };
        let mut smaa_frame = if self.smaa_enabled {
            self.smaa_target
                .as_mut()
                .map(|target| target.start_frame(&self.device, &self.queue, presentation_output))
        } else {
            None
        };
        let final_output: &wgpu::TextureView = match smaa_frame.as_ref() {
            Some(smaa_frame) => smaa_frame,
            None => presentation_output,
        };
        // RainyGlass is the WGPU equivalent of the upstream GrabPass: normal
        // post first resolves into its private full-resolution source, then the
        // lens pass refracts that finished scene into the real presentation
        // target. HUD/debug/UI remain after it and therefore stay undistorted.
        let rain_glass_active = plan.use_post && self.weather.rain.lens_glass_active();

        let scene_output = if plan.use_post {
            &self.targets.scene_view
        } else {
            final_output
        };

        let mut compact_group_drawn = std::mem::take(&mut self.compact_group_scratch);
        if let Some(started) = perf_pre_world_started {
            info.perf_encode_pre_world_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let perf_scene_started = perf_timing.then(Instant::now);
        self.gpu_profiler
            .write_encoder_timestamp(&mut encoder, GpuPass::World, false);
        if let Some(world) = &self.world {
            let active_pipeline_variant = &world.pipeline_variants[&world.active_pipeline_variant];
            // SMAA keeps self.smaa_target mutably borrowed until the frame resolves.
            // Borrow only the disjoint shadow fields instead of all of self.
            let world_shadow_receiver_bind_group =
                if world.active_pipeline_variant.ray_traced_shadows {
                    self.ray_traced_shadows
                        .as_ref()
                        .map(|rt| &rt.receiver_bind_group)
                        .unwrap_or(&self.shadow_resources.receiver_bind_group)
                } else {
                    &self.shadow_resources.receiver_bind_group
                };
            let (color_view, resolve_target) = if let Some(msaa_view) = &self.targets.msaa_view {
                (msaa_view, Some(scene_output))
            } else {
                (scene_output, None)
            };
            let active = active_batch_selection(world, self.pvs_mode);
            let effective_area_mask = effective_area_mask(world, self.pvs_mode, area_mask);
            info.world_pvs_batches = u32::try_from(active.indices.len()).unwrap_or(u32::MAX);

            // OpenJK batches visible drawsurfs by shader/fog state after BSP/PVS
            // visibility has been resolved. Mirror that submission property on
            // the normal CPU-culling path with our existing same-state groups:
            // retain per-batch frustum precision, then pack only safe, single-
            // stage opaque/masked survivors into indirect multi-draws. Authored
            // multi-stage and transparent ordering remains on the conservative
            // per-draw path.
            let use_cpu_compaction = !self.gpu_driven_enabled
                && !render_ocean
                && self.gpu_compaction_supported
                && self.planar_reflection_debug_mode == PlanarReflectionDebugMode::Off
                && !world.compact_groups.is_empty();
            if use_cpu_compaction {
                let compact_capacity = world
                    .compact_groups
                    .iter()
                    .map(|group| group.output_base.saturating_add(group.max_count))
                    .max()
                    .unwrap_or(0) as usize;
                self.cpu_compact_indirect_scratch
                    .resize(compact_capacity.max(1), DrawIndexedIndirectArgs::zeroed());
                self.cpu_compact_count_scratch
                    .resize(world.compact_groups.len().max(1), 0);
                self.cpu_compact_count_scratch.fill(0);

                for (active_pos, &batch_index) in active.indices.iter().enumerate() {
                    let batch = &active.batches[batch_index];
                    let Some(group_index) = batch.compact_group.map(|value| value as usize) else {
                        continue;
                    };
                    if batch.is_inline_entity
                        || !batch_area_visible(batch, effective_area_mask)
                        || batch.source.pipeline.class == DrawClass::Sky
                        || !aabb_intersects_clip_frustum(
                            batch.bounds_min,
                            batch.bounds_max,
                            view_proj,
                        )
                    {
                        continue;
                    }
                    let previous_same = active_pos > 0
                        && same_world_surface(
                            batch,
                            &active.batches[active.indices[active_pos - 1]],
                        );
                    let next_same = active
                        .indices
                        .get(active_pos + 1)
                        .is_some_and(|&next_index| {
                            same_world_surface(batch, &active.batches[next_index])
                        });
                    if previous_same || next_same {
                        continue;
                    }
                    let Some(group) = world.compact_groups.get(group_index) else {
                        continue;
                    };
                    let count = self.cpu_compact_count_scratch[group_index];
                    if count >= group.max_count {
                        continue;
                    }
                    let output = group.output_base.saturating_add(count) as usize;
                    let Some(slot) = self.cpu_compact_indirect_scratch.get_mut(output) else {
                        continue;
                    };
                    *slot = DrawIndexedIndirectArgs {
                        index_count: batch
                            .indexed_range
                            .end
                            .saturating_sub(batch.indexed_range.start),
                        instance_count: 1,
                        first_index: batch.indexed_range.start,
                        base_vertex: 0,
                        first_instance: 0,
                    };
                    self.cpu_compact_count_scratch[group_index] = count.saturating_add(1);
                }

                if compact_capacity != 0 {
                    self.queue.write_buffer(
                        &world.compact_indirect_buffer,
                        0,
                        bytemuck::cast_slice(
                            &self.cpu_compact_indirect_scratch[..compact_capacity],
                        ),
                    );
                    self.queue.write_buffer(
                        &world.compact_count_buffer,
                        0,
                        bytemuck::cast_slice(
                            &self.cpu_compact_count_scratch[..world.compact_groups.len()],
                        ),
                    );
                }
            }

            // Sky portal view (OpenJK renders it before the main scene and the
            // main view sky surfaces then draw nothing). It owns its own PVS
            // cluster and depth; the main pass loads its color.
            let sky_portal_drawn = if let Some((portal_position, portal_view_proj)) =
                sky_portal_view
            {
                let cluster = world.visibility.as_ref().and_then(|vis| {
                    vis.cluster_at(scene::jka_position(portal_position.to_array()))
                });
                let indices: &[usize] = match cluster {
                    Some(cluster) if self.pvs_mode != PvsMode::Off => world
                        .coarse_visible_batches_by_cluster
                        .get(cluster)
                        .unwrap_or(&world.coarse_all_batches),
                    _ => &world.coarse_all_batches,
                };
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut sky_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA sky portal"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(self.weather.fog.clear_color()),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.targets.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(camera_depth_clear()),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                sky_pass.set_bind_group(0, &self.sky_portal_camera_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                sky_pass.set_bind_group(
                    2,
                    if enhanced_world_shader {
                        &world.lighting_bind_group
                    } else {
                        &world.lighting_bind_group_lean
                    },
                    &[],
                );
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                sky_pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                sky_pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                if enhanced_world_shader {
                    let ocean_bind_group = self
                        .ocean
                        .as_ref()
                        .map(|ocean| &ocean.render_bind_group)
                        .unwrap_or(&self.ocean_inert_bind_group);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    sky_pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    sky_pass.set_bind_group(6, ocean_bind_group, &[]);
                }
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                sky_pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                sky_pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                sky_pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                let mut current_pipeline = None;
                for &batch_index in indices {
                    let batch = &world.coarse_batches[batch_index];
                    if batch.is_inline_entity
                        || ocean_suppresses_batch(render_ocean, batch)
                        || (batch.source.pipeline.class != DrawClass::Sky
                            && !aabb_intersects_clip_frustum(
                                batch.bounds_min,
                                batch.bounds_max,
                                portal_view_proj,
                            ))
                    {
                        continue;
                    }
                    if current_pipeline != Some(batch.source.pipeline) {
                        let Some(pipeline) = active_pipeline_variant
                            .pipelines
                            .get(&batch.source.pipeline)
                        else {
                            continue;
                        };
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        sky_pass.set_pipeline(pipeline);
                        current_pipeline = Some(batch.source.pipeline);
                    }
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    sky_pass.set_bind_group(1, &batch.bind_group, &[]);
                    if enhanced_world_shader {
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        sky_pass.set_bind_group(
                            6,
                            Self::ocean_bind_group_for(
                                batch,
                                &self.authored_oceans,
                                &self.ocean,
                                &self.ocean_inert_bind_group,
                            ),
                            &[],
                        );
                    }
                    if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                    draw_world_batch(&mut sky_pass, batch, 0..1, None);
                }
                drop(sky_pass);
                if self.world_flares_enabled {
                    let flare_format = flare_scene_format;
                    self.world_flares_sky.draw_sky(
                        &self.device,
                        &self.queue,
                        &mut encoder,
                        color_view,
                        &self.targets.depth_view,
                        flare_format,
                        self.msaa_samples,
                        portal_position,
                        portal_view_proj,
                        camera,
                        self.config.width,
                        self.config.height,
                        world.visibility.as_ref(),
                        self.pvs_mode != PvsMode::Off,
                        self.weather.fog.legacy_drawfog_value(),
                    );
                }
                true
            } else {
                false
            };
            if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("JKA world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: if sky_portal_drawn {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(self.weather.fog.clear_color())
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(camera_depth_clear()),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(
                2,
                if enhanced_world_shader {
                    &world.lighting_bind_group
                } else {
                    &world.lighting_bind_group_lean
                },
                &[],
            );
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
            if enhanced_world_shader {
                let ocean_bind_group = self
                    .ocean
                    .as_ref()
                    .map(|ocean| &ocean.render_bind_group)
                    .unwrap_or(&self.ocean_inert_bind_group);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(6, ocean_bind_group, &[]);
            }
            if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
            pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
            if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
            pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
            if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
            pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            let mut current_pipeline = None;
            // Grass no longer interleaves mid-loop (see the comment at its one
            // remaining call site below), so this never flips true inside the
            // loop; it only gates that single post-loop draw.
            let grass_drawn = false;
            let mut snow_shell_drawn = false;
            // A separate OpenJK-style fog redraw must occur after the final
            // material stage of each surface. GPU compaction can merge/reorder
            // those stages, so keep it only when this frame needs no fog pass.
            let separate_legacy_fog_pass = match self.weather.fog.legacy_drawfog_value() {
                1 => active.indices.iter().any(|&batch_index| {
                    let batch = &active.batches[batch_index];
                    batch.source.fog[3] > 0.001
                        && (!batch.source.fog_is_global || !batch.source.global_fog_post_eligible)
                }),
                2 => active.indices.iter().any(|&batch_index| {
                    let batch = &active.batches[batch_index];
                    batch.source.fog[3] > 0.001
                }),
                _ => false,
            };
            let use_compaction = self.gpu_driven_enabled
                && !render_ocean
                // Compact groups share one representative material bind group.
                // Legacy carries a per-draw source-triangle base in that bind
                // group, so compacting different source ranges would alias the
                // OpenJK-style surface dlight masks. Keep Legacy correct until
                // compaction has an explicit per-draw metadata index.
                && self.dynamic_lights_mode != DynamicLightsMode::Legacy
                && self.gpu_compaction_supported
                && self.planar_reflection_debug_mode == PlanarReflectionDebugMode::Off
                && !separate_legacy_fog_pass
                && !world.compact_groups.is_empty();
            compact_group_drawn.clear();
            compact_group_drawn.resize(world.compact_groups.len(), false);
            // Movers are drawn after the world/grass as one multi-draw per draw
            // state (see `plan_inline_runs`); GPU-driven culling, the ocean,
            // underwater optics and planar debug views keep the ordered path.
            let inline_grouping = !self.gpu_driven_enabled
                && !render_ocean
                && self.gpu_compaction_supported
                && optical_volumes.is_none()
                && self.planar_reflection_debug_mode == PlanarReflectionDebugMode::Off
                && world.inline_group_count != 0;
            let mut inline_visible = std::mem::take(&mut self.inline_visible_scratch);
            inline_visible.clear();
            // Godot water is a depth-writing surface drawn after the scene-color
            // capture. Alpha-blended BSP stages cannot be drawn before it because
            // they intentionally leave no depth behind for the later water test.
            // Partition them while walking the already-selected visible list, then
            // consume only this retained queue after ocean depth exists. This moves
            // the same draw encoding work without another active/PVS traversal or
            // per-frame allocation churn.
            let defer_transparent_for_ocean = optical_volumes.is_some();
            let mut post_ocean_transparent =
                std::mem::take(&mut self.post_ocean_transparent_scratch);
            post_ocean_transparent.clear();
            for (active_pos, &batch_index) in active.indices.iter().enumerate() {
                let batch = &active.batches[batch_index];
                if !batch_area_visible(batch, effective_area_mask) {
                    continue;
                }
                if ocean_suppresses_batch(render_ocean, batch) {
                    continue;
                }
                // The sky portal view already filled the background.
                if sky_portal_drawn && batch.source.pipeline.class == DrawClass::Sky {
                    continue;
                }
                // OpenJK performs view-frustum rejection after PVS marking.
                // Do the CPU equivalent whenever indirect GPU culling is off.
                // This must apply to compiled BSP maps too; limiting it to the
                // direct source-.map path made large-PVS maps encode thousands
                // of off-camera draws.
                if !self.gpu_driven_enabled
                    && batch.source.pipeline.class != DrawClass::Sky
                    && !aabb_intersects_clip_frustum(batch.bounds_min, batch.bounds_max, view_proj)
                {
                    info.world_frustum_rejected = info.world_frustum_rejected.saturating_add(1);
                    continue;
                }
                info.world_encoded_batches = info.world_encoded_batches.saturating_add(1);
                if let Some(volumes) = &optical_volumes {
                    let optical = self.ocean_settings.optics;
                    let wave_margin = self
                        .authored_ocean_definitions
                        .iter()
                        .map(|a| a.waves.amplitude)
                        .fold(self.ocean_settings.authored.amplitude, f32::max)
                        * 4.0;
                    if batch.source.pipeline.class != DrawClass::Sky
                        && crate::ocean::optics::cull_submerged(
                            camera.position.to_array(),
                            batch.bounds_min,
                            batch.bounds_max,
                            volumes,
                            optical.absorption_distance() * optical.underwater_cull,
                            wave_margin,
                        )
                    {
                        continue;
                    }
                }
                if inline_grouping && batch.is_inline_entity && batch.inline_group != u32::MAX {
                    inline_visible.push(batch_index);
                    continue;
                }
                // Grass previously interleaved its draw here, at the first
                // transparent batch, sharing this RenderPass's bind-group slots
                // with the BSP batch-draw state machine below (compact groups,
                // cpu compaction, inline grouping) right as it switches pipelines
                // and bind groups for other batches. That interleaving is a
                // reproducible source of a wgpu bind-group/pipeline mismatch
                // panic (grass's pipeline left paired with a stale BSP surface
                // bind group), which a double-panic during the resulting cleanup
                // then escalates to a hard process abort. Keep grass fully out of
                // this loop; the unconditional `!grass_drawn` draw after the loop
                // (below) is now its only call site. Grass now draws just after
                // the world's opaque+masked geometry instead of mid-transparency;
                // since grass blades are alpha-tested/depth-writing, not blended,
                // this is a minor depth-sort difference, not a visual regression.
                if !snow_shell_drawn && batch.source.pipeline.class == DrawClass::Transparent {
                    draw_snow_shell(
                        &mut pass,
                        world,
                        &active_pipeline_variant.pipelines,
                        view_proj,
                        snow_deform_center,
                        &mut current_pipeline,
                    );
                    snow_shell_drawn = true;
                }
                if matches!(
                    self.planar_reflection_debug_mode,
                    PlanarReflectionDebugMode::Candidates
                        | PlanarReflectionDebugMode::SelectedPlane
                        | PlanarReflectionDebugMode::AppliedSample
                        | PlanarReflectionDebugMode::BindingTest
                ) {
                    // In surface-debug modes draw only the stage that can
                    // actually own the planar source substitution. Do not rely
                    // on propagated sibling-plane metadata here: AUTO/FULL PVS
                    // may render a finer sub-batch than the coarse reflector
                    // selected for the reflected camera.
                    let planar_stage = batch.source.planar_reflection
                        || (self.planar_reflection_mode == PlanarReflectionMode::Environment
                            && matches!(batch.source.tc_gen, TcGen::Environment));
                    if !planar_stage {
                        continue;
                    }
                }
                if defer_transparent_for_ocean
                    && batch.source.pipeline.class == DrawClass::Transparent
                {
                    post_ocean_transparent.push(active_pos);
                    continue;
                }
                if use_cpu_compaction {
                    if let Some(group_index) = batch.compact_group {
                        let group_index = group_index as usize;
                        let previous_same = active_pos > 0
                            && same_world_surface(
                                batch,
                                &active.batches[active.indices[active_pos - 1]],
                            );
                        let next_is_same_surface =
                            active
                                .indices
                                .get(active_pos + 1)
                                .is_some_and(|&next_index| {
                                    same_world_surface(batch, &active.batches[next_index])
                                });
                        let cpu_compactable = !batch.is_inline_entity
                            && !previous_same
                            && !next_is_same_surface
                            && self
                                .cpu_compact_count_scratch
                                .get(group_index)
                                .copied()
                                .unwrap_or(0)
                                != 0;
                        if cpu_compactable {
                            if !compact_group_drawn[group_index] {
                                let group = &world.compact_groups[group_index];
                                let representative = match group.representative_set {
                                    WorldBatchSet::Coarse => {
                                        &world.coarse_batches[group.representative_index]
                                    }
                                    WorldBatchSet::Full => {
                                        &world.full_batches[group.representative_index]
                                    }
                                };
                                if current_pipeline != Some(representative.source.pipeline) {
                                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                    pass.set_pipeline(
                                        &active_pipeline_variant.pipelines
                                            [&representative.source.pipeline],
                                    );
                                    current_pipeline = Some(representative.source.pipeline);
                                }
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &representative.bind_group, &[]);
                                if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect_count = info.perf_encode_calls.multi_draw_indexed_indirect_count.saturating_add(1); }
                                pass.multi_draw_indexed_indirect_count(
                                    &world.compact_indirect_buffer,
                                    u64::from(group.output_base)
                                        * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                    &world.compact_count_buffer,
                                    group_index as u64 * std::mem::size_of::<u32>() as u64,
                                    group.max_count,
                                );
                                let compacted_count = self.cpu_compact_count_scratch[group_index];
                                info.world_multidraw_groups =
                                    info.world_multidraw_groups.saturating_add(1);
                                info.world_multidraw_batches =
                                    info.world_multidraw_batches.saturating_add(compacted_count);

                                // A single-stage surface can carry its Legacy fog
                                // redraw as the same multi-draw immediately after
                                // the material pass. This preserves OpenJK's
                                // material-then-fog ordering without disabling
                                // batching for an entire foggy map.
                                let needs_group_fog = batch.source.pipeline.class != DrawClass::Sky
                                    && self.weather.fog.legacy_uses_separate_pass(
                                        batch.source.fog_is_global,
                                        batch.source.legacy2_fog_in_stage_safe,
                                        batch.source.global_fog_post_eligible,
                                    )
                                    && (batch.source.fog[3] > 0.001
                                        || self.weather.fog.legacy_drawfog_value() == 1);
                                if needs_group_fog {
                                    if let Some(fog_pipeline) =
                                        active_pipeline_variant.fog_pass_pipelines.get(&(
                                            batch.source.pipeline,
                                            batch.source.pipeline.depth_write,
                                        ))
                                    {
                                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                        pass.set_pipeline(fog_pipeline);
                                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                        pass.set_bind_group(1, &representative.bind_group, &[]);
                                        if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect_count = info.perf_encode_calls.multi_draw_indexed_indirect_count.saturating_add(1); }
                                        pass.multi_draw_indexed_indirect_count(
                                            &world.compact_indirect_buffer,
                                            u64::from(group.output_base)
                                                * std::mem::size_of::<DrawIndexedIndirectArgs>()
                                                    as u64,
                                            &world.compact_count_buffer,
                                            group_index as u64 * std::mem::size_of::<u32>() as u64,
                                            group.max_count,
                                        );
                                        current_pipeline = None;
                                    }
                                }
                                compact_group_drawn[group_index] = true;
                            }
                            continue;
                        }
                    }
                }

                if use_compaction {
                    if let Some(group_index) = batch.compact_group {
                        let group_index = group_index as usize;
                        if !compact_group_drawn[group_index] {
                            let group = &world.compact_groups[group_index];
                            let representative = match group.representative_set {
                                WorldBatchSet::Coarse => {
                                    &world.coarse_batches[group.representative_index]
                                }
                                WorldBatchSet::Full => {
                                    &world.full_batches[group.representative_index]
                                }
                            };
                            if current_pipeline != Some(representative.source.pipeline) {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(
                                    &active_pipeline_variant.pipelines
                                        [&representative.source.pipeline],
                                );
                                current_pipeline = Some(representative.source.pipeline);
                            }
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(1, &representative.bind_group, &[]);
                            if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect_count = info.perf_encode_calls.multi_draw_indexed_indirect_count.saturating_add(1); }
                            pass.multi_draw_indexed_indirect_count(
                                &world.compact_indirect_buffer,
                                u64::from(group.output_base)
                                    * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                &world.compact_count_buffer,
                                group_index as u64 * std::mem::size_of::<u32>() as u64,
                                group.max_count,
                            );
                            compact_group_drawn[group_index] = true;
                        }
                        continue;
                    }
                }
                if current_pipeline != Some(batch.source.pipeline) {
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(&active_pipeline_variant.pipelines[&batch.source.pipeline]);
                    current_pipeline = Some(batch.source.pipeline);
                }
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(1, &batch.bind_group, &[]);
                if enhanced_world_shader {
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(
                        6,
                        Self::ocean_bind_group_for(
                            batch,
                            &self.authored_oceans,
                            &self.ocean,
                            &self.ocean_inert_bind_group,
                        ),
                        &[],
                    );
                }
                if self.gpu_driven_enabled && !(render_ocean && batch.ocean_clipmap.is_some()) {
                    let offset = u64::from(batch.cull_index)
                        * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
                    if capture_api_calls { info.perf_encode_calls.draw_indexed_indirect = info.perf_encode_calls.draw_indexed_indirect.saturating_add(1); }
                    pass.draw_indexed_indirect(&world.indirect_buffer, offset);
                } else {
                    // Promoted water uses the separately generated dense index
                    // range, which is not represented by the stock indirect record.
                    if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                    draw_world_batch(&mut pass, batch, 0..1, ocean_clipmap);
                }

                // OpenJK r_drawfog 1 redraws fog after all material stages.
                // r_drawfog 2 does the same only for local brush fog. Batches for
                // a multi-stage shader share the exact vertex range, so emit the
                // pass only after the last sibling stage instead of fogging each
                // stage independently.
                let next_is_same_surface =
                    active
                        .indices
                        .get(active_pos + 1)
                        .is_some_and(|&next_index| {
                            same_world_surface(batch, &active.batches[next_index])
                        });
                let surface_writes_depth = active.indices[..=active_pos]
                    .iter()
                    .rev()
                    .map(|&index| &active.batches[index])
                    .take_while(|sibling| same_world_surface(batch, sibling))
                    .any(|sibling| sibling.source.pipeline.depth_write);
                let needs_fog_pass = !next_is_same_surface
                    && batch.source.pipeline.class != DrawClass::Sky
                    // The displaced GodotOcean cannot pass this depth-EQUAL
                    // redraw; it fogs itself (bsp.wgsl apply_ocean_legacy_fog).
                    && !(render_ocean && batch.ocean_clipmap.is_some())
                    && self.weather.fog.legacy_uses_separate_pass(
                        batch.source.fog_is_global,
                        batch.source.legacy2_fog_in_stage_safe,
                        batch.source.global_fog_post_eligible,
                    )
                    && (batch.source.fog[3] > 0.001
                        || self.weather.fog.legacy_drawfog_value() == 1);
                if needs_fog_pass {
                    if let Some(fog_pipeline) = active_pipeline_variant
                        .fog_pass_pipelines
                        .get(&(batch.source.pipeline, surface_writes_depth))
                    {
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(fog_pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(1, &batch.bind_group, &[]);
                        if enhanced_world_shader {
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(
                                6,
                                Self::ocean_bind_group_for(
                                    batch,
                                    &self.authored_oceans,
                                    &self.ocean,
                                    &self.ocean_inert_bind_group,
                                ),
                                &[],
                            );
                        }
                        if self.gpu_driven_enabled
                            && !(render_ocean && batch.ocean_clipmap.is_some())
                        {
                            let offset = u64::from(batch.cull_index)
                                * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
                            if capture_api_calls { info.perf_encode_calls.draw_indexed_indirect = info.perf_encode_calls.draw_indexed_indirect.saturating_add(1); }
                            pass.draw_indexed_indirect(&world.indirect_buffer, offset);
                        } else {
                            if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                            draw_world_batch(&mut pass, batch, 0..1, ocean_clipmap);
                        }
                        // The next authored stage uses a different pipeline object
                        // even when its PipelineKey matches this batch. Force the
                        // normal pipeline to be rebound after the fog pass.
                        current_pipeline = None;
                    }
                }
            }
            if !snow_shell_drawn {
                draw_snow_shell(
                    &mut pass,
                    world,
                    &active_pipeline_variant.pipelines,
                    view_proj,
                    snow_deform_center,
                    &mut current_pipeline,
                );
            }
            if !grass_drawn && self.grass_enabled {
                if let (Some(grass), Some(prepared)) = (&world.grass, prepared_grass.as_ref()) {
                    self.gpu_profiler
                        .write_render_pass_timestamp(&mut pass, GpuPass::Grass, false);
                    info.grass = self.grass_renderer.draw(
                        &mut pass,
                        &self.camera_bind_group,
                        &self.shadow_resources.receiver_bind_group,
                        grass,
                        prepared,
                    );
                    self.gpu_profiler
                        .write_render_pass_timestamp(&mut pass, GpuPass::Grass, true);
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                    pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    // Keep the parent world pass in a valid state even when grass is
                    // the final opaque draw. This also protects later debug/wireframe
                    // draws from inheriting grass's bind-group slot 2.
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(
                        2,
                        if enhanced_world_shader {
                            &world.lighting_bind_group
                        } else {
                            &world.lighting_bind_group_lean
                        },
                        &[],
                    );
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                    // Grass replaces the RenderPass pipeline, so the cached BSP key is
                    // no longer authoritative. Force the next BSP/inline draw to bind
                    // its pipeline even when its PipelineKey matches the pre-grass one.
                    current_pipeline = None;
                    if enhanced_world_shader {
                        let ocean_bind_group = self
                            .ocean
                            .as_ref()
                            .map(|ocean| &ocean.render_bind_group)
                            .unwrap_or(&self.ocean_inert_bind_group);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(6, ocean_bind_group, &[]);
                    }
                }
            }
            if !inline_visible.is_empty() {
                let packed = pack_inline_draws(
                    &self.queue,
                    active.batches,
                    &inline_visible,
                    &mut self.inline_draw_scratch,
                    world.inline_group_count,
                    &world.inline_indirect_buffer,
                    0,
                    world.inline_indirect_capacity,
                );
                let scratch = &self.inline_draw_scratch;
                if packed {
                    for (run_index, &(first, count)) in scratch.runs.iter().enumerate() {
                        let batch =
                            &active.batches[inline_visible[scratch.order[first as usize] as usize]];
                        if current_pipeline != Some(batch.source.pipeline) {
                            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                            pass.set_pipeline(
                                &active_pipeline_variant.pipelines[&batch.source.pipeline],
                            );
                            current_pipeline = Some(batch.source.pipeline);
                        }
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(1, &batch.bind_group, &[]);
                        if enhanced_world_shader {
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(
                                6,
                                Self::ocean_bind_group_for(
                                    batch,
                                    &self.authored_oceans,
                                    &self.ocean,
                                    &self.ocean_inert_bind_group,
                                ),
                                &[],
                            );
                        }
                        if count == 1 {
                            if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                            pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                        } else {
                            if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect = info.perf_encode_calls.multi_draw_indexed_indirect.saturating_add(1); }
                            pass.multi_draw_indexed_indirect(
                                &world.inline_indirect_buffer,
                                u64::from(first)
                                    * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                count,
                            );
                        }
                        info.world_encoded_batches = info.world_encoded_batches.saturating_add(1);
                        // Legacy fog redraws every surface whose last stage is in
                        // this run, so the run repeats as one multi-draw.
                        if let Some(writes_depth) = inline_run_fog_depth(
                            &self.weather.fog,
                            render_ocean,
                            batch,
                            scratch.run_class[run_index] as u8 & 3,
                        ) {
                            if let Some(fog_pipeline) = active_pipeline_variant
                                .fog_pass_pipelines
                                .get(&(batch.source.pipeline, writes_depth))
                            {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(fog_pipeline);
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &batch.bind_group, &[]);
                                if enhanced_world_shader {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            batch,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if count == 1 {
                                    if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                                    pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                                } else {
                                    if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect = info.perf_encode_calls.multi_draw_indexed_indirect.saturating_add(1); }
                                    pass.multi_draw_indexed_indirect(
                                        &world.inline_indirect_buffer,
                                        u64::from(first)
                                            * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                        count,
                                    );
                                }
                                current_pipeline = None;
                                info.world_encoded_batches =
                                    info.world_encoded_batches.saturating_add(1);
                            }
                        }
                    }
                } else {
                    let kinds = inline_stage_info(active.batches, &inline_visible).1;
                    for (position, &batch_index) in inline_visible.iter().enumerate() {
                        let batch = &active.batches[batch_index];
                        if current_pipeline != Some(batch.source.pipeline) {
                            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                            pass.set_pipeline(
                                &active_pipeline_variant.pipelines[&batch.source.pipeline],
                            );
                            current_pipeline = Some(batch.source.pipeline);
                        }
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(1, &batch.bind_group, &[]);
                        if enhanced_world_shader {
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(
                                6,
                                Self::ocean_bind_group_for(
                                    batch,
                                    &self.authored_oceans,
                                    &self.ocean,
                                    &self.ocean_inert_bind_group,
                                ),
                                &[],
                            );
                        }
                        if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                        pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                        info.world_encoded_batches = info.world_encoded_batches.saturating_add(1);
                        if let Some(writes_depth) = inline_run_fog_depth(
                            &self.weather.fog,
                            render_ocean,
                            batch,
                            kinds[position],
                        ) {
                            if let Some(fog_pipeline) = active_pipeline_variant
                                .fog_pass_pipelines
                                .get(&(batch.source.pipeline, writes_depth))
                            {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(fog_pipeline);
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &batch.bind_group, &[]);
                                if enhanced_world_shader {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            batch,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                                pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                                current_pipeline = None;
                            }
                        }
                    }
                }
            }
            self.inline_visible_scratch = inline_visible;
            // OpenJK projects legacy dynamic lights as another rendering pass
            // after the authored material stages. The normal BSP pipelines above
            // compile with ENABLE_LEGACY_DLIGHTS=false; redraw only authored
            // surface runs whose CPU mirror of the dlight mask is non-zero.
            if world.active_pipeline_variant.legacy_dlights
                && self.planar_reflection_debug_mode == PlanarReflectionDebugMode::Off
                && !active_pipeline_variant.legacy_dlight_pipelines.is_empty()
            {
                if let Ok(surface_masks) = world.legacy_dlight_surface_masks_cpu.lock() {
                    if surface_masks.iter().any(|&mask| mask != 0) {
                        self.gpu_profiler.write_render_pass_timestamp(
                            &mut pass,
                            GpuPass::LegacyDlights,
                            false,
                        );
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(0, &self.camera_bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(
                            2,
                            if enhanced_world_shader {
                                &world.lighting_bind_group
                            } else {
                                &world.lighting_bind_group_lean
                            },
                            &[],
                        );
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                        if enhanced_world_shader {
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                        }
                        if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                        pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                        if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                        pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                        if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                        pass.set_index_buffer(
                            world.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );

                        let dlight_active = legacy_dlight_batch_selection(world, self.pvs_mode);
                        let mut current_dlight_pipeline = None;
                        for &batch_index in dlight_active.indices {
                            let batch = &dlight_active.batches[batch_index];
                            if !legacy_dlight_receives(&batch.source)
                                || !batch_area_visible(batch, effective_area_mask)
                                || ocean_suppresses_batch(render_ocean, batch)
                                || !aabb_intersects_clip_frustum(
                                    batch.bounds_min,
                                    batch.bounds_max,
                                    view_proj,
                                )
                            {
                                continue;
                            }
                            let Some(dlight_pipeline) = active_pipeline_variant
                                .legacy_dlight_pipelines
                                .get(&batch.source.pipeline)
                            else {
                                continue;
                            };

                            let has_touched_run = batch.is_inline_entity
                                || batch.legacy_dlight_runs.iter().any(|run| {
                                    run.surface_id != u32::MAX
                                        && surface_masks
                                            .get(run.surface_id as usize)
                                            .is_some_and(|&mask| mask != 0)
                                });
                            if !has_touched_run {
                                continue;
                            }

                            if current_dlight_pipeline != Some(batch.source.pipeline) {
                                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                pass.set_pipeline(dlight_pipeline);
                                current_dlight_pipeline = Some(batch.source.pipeline);
                            }
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(1, &batch.bind_group, &[]);
                            if enhanced_world_shader {
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(
                                    6,
                                    Self::ocean_bind_group_for(
                                        batch,
                                        &self.authored_oceans,
                                        &self.ocean,
                                        &self.ocean_inert_bind_group,
                                    ),
                                    &[],
                                );
                            }

                            if batch.is_inline_entity {
                                // Inline BSP models use the existing conservative
                                // unknown-surface fallback (all transient dlights).
                                if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                                pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                            } else {
                                for run in &batch.legacy_dlight_runs {
                                    if run.surface_id == u32::MAX
                                        || !surface_masks
                                            .get(run.surface_id as usize)
                                            .is_some_and(|&mask| mask != 0)
                                    {
                                        continue;
                                    }
                                    if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                                    pass.draw_indexed(run.indexed_range.clone(), 0, 0..1);
                                }
                            }
                        }
                        self.gpu_profiler.write_render_pass_timestamp(
                            &mut pass,
                            GpuPass::LegacyDlights,
                            true,
                        );
                    }
                }
            }

            if let Some(prepared) = prepared_surface_sprite_effects.as_ref() {
                self.surface_sprite_effect_renderer.draw(
                    &mut pass,
                    &self.camera_bind_group,
                    &world.surface_sprite_effects,
                    prepared,
                );
            }
            let split_dynamic_for_ocean = optical_volumes.is_some();
            self.gpu_profiler
                .write_render_pass_timestamp(&mut pass, GpuPass::DynamicModels, false);
            if split_dynamic_for_ocean {
                // Only depth-writing entities belong in the pre-water scene.
                // Additive/alpha FX (especially saber core + glow) must be
                // composited after the ocean surface and spray, otherwise the
                // opaque water pass overwrites them because they do not write
                // depth themselves.
                self.dynamic_model_renderer.draw_phase(
                    &mut pass,
                    &self.camera_bind_group,
                    true,
                    self.ray_traced_shadows
                        .as_ref()
                        .filter(|_| {
                            self.dynamic_lights_mode == DynamicLightsMode::RayTracedHardware
                        })
                        .and_then(|rt| {
                            rt.model_receivers
                                .as_ref()
                                .map(|models| (models, &rt.receiver_bind_group))
                        }),
                    Some(&mut self.gpu_profiler),
                    None,
                    false,
                );
            } else {
                for depth_writing_phase in [true, false] {
                    self.dynamic_model_renderer.draw_phase(
                        &mut pass,
                        &self.camera_bind_group,
                        depth_writing_phase,
                        None,
                        Some(&mut self.gpu_profiler),
                        None,
                        false,
                    );
                }
            }
            self.gpu_profiler
                .write_render_pass_timestamp(&mut pass, GpuPass::DynamicModels, true);

            if optical_volumes.is_some() {
                drop(pass);
                self.gpu_profiler.write_encoder_timestamp(
                    &mut encoder,
                    GpuPass::OceanOptics,
                    false,
                );
                let optical_ocean = self
                    .authored_oceans
                    .iter()
                    .find(|(a, _)| {
                        camera.position.x >= a.mins[0]
                            && camera.position.x <= a.maxs[0]
                            && -camera.position.z >= a.mins[1]
                            && -camera.position.z <= a.maxs[1]
                    })
                    .map(|(_, gpu)| &gpu.render_bind_group)
                    .or_else(|| self.ocean.as_ref().map(|gpu| &gpu.render_bind_group))
                    .unwrap_or(&self.ocean_inert_bind_group);
                // Inside the water's real outline, not just its bounding box
                // (`refresh_water_boxes` ran this frame for the same camera).
                let submerged = optical_volumes.is_some()
                    && self.camera_water_surface > crate::renderer::NO_WATER_SURFACE;
                // The capture serves exactly two readers: the ocean surface
                // shader, which samples it where water is drawn, and the
                // submerged composite, which reads all of it. So the screen area
                // it must cover is the visible water's, or everything when the
                // camera is under water. Scan the same batches the surface pass
                // below will draw, with the same two culling tests, so the
                // rectangle can never be smaller than what gets sampled.
                let optical = self.ocean_settings.optics;
                let mut water_rect: Option<(f32, f32, f32, f32)> = None;
                let mut water_unbounded = false;
                let mut water_visible = false;
                for &batch_index in active.indices {
                    let batch = &active.batches[batch_index];
                    if batch.ocean_clipmap.is_none()
                        || !batch_area_visible(batch, effective_area_mask)
                        || !aabb_intersects_clip_frustum(
                            batch.bounds_min,
                            batch.bounds_max,
                            view_proj,
                        )
                    {
                        continue;
                    }
                    water_visible = true;
                    match aabb_screen_rect(
                        batch.bounds_min,
                        batch.bounds_max,
                        view_proj,
                        self.config.width.max(1),
                        self.config.height.max(1),
                    ) {
                        Some((x0, y0, x1, y1)) => {
                            let merged = water_rect
                                .map(|(ax0, ay0, ax1, ay1)| {
                                    (ax0.min(x0), ay0.min(y0), ax1.max(x1), ay1.max(y1))
                                })
                                .unwrap_or((x0, y0, x1, y1));
                            water_rect = Some(merged);
                        }
                        // Straddles the eye: nothing tighter than the full
                        // target is correct for this surface.
                        None => water_unbounded = true,
                    }
                }
                // Nothing can observe the capture: no water on screen to sample
                // it and no submerged composite to blit it. `water_visible` (a
                // bounds test) cannot answer this - it reads true even when the
                // ocean pass puts zero samples on screen - so the occlusion
                // query from a previous frame decides, with the same grace
                // period the simulation gate uses.
                let water_hidden = self.ocean_visibility.primed
                    && self.ocean_visibility.latest_samples == 0
                    && self.ocean_hidden_time > OCEAN_SIM_GRACE_SECONDS;
                let skip_capture = optical.cull && !submerged && water_hidden;
                let _ = water_visible;
                let scissor = if optical.scissor && !submerged && !water_unbounded {
                    water_rect.map(|(x0, y0, x1, y1)| {
                        // The surface shader offsets its sample by at most
                        // `refraction` in normalized screen space (bsp.wgsl),
                        // so grow the rectangle by that much before clamping.
                        let width = self.config.width.max(1) as f32;
                        let height = self.config.height.max(1) as f32;
                        let pad_x = optical.refraction * width + 1.0;
                        let pad_y = optical.refraction * height + 1.0;
                        let x0 = (x0 - pad_x).clamp(0.0, width);
                        let y0 = (y0 - pad_y).clamp(0.0, height);
                        let x1 = (x1 + pad_x).clamp(0.0, width);
                        let y1 = (y1 + pad_y).clamp(0.0, height);
                        (
                            x0 as u32,
                            y0 as u32,
                            (x1 - x0).ceil().max(0.0) as u32,
                            (y1 - y0).ceil().max(0.0) as u32,
                        )
                    })
                } else {
                    None
                };
                // Pair each MSAA color sample with the matching depth sample in
                // the underwater optics capture. Sampling the resolved color with
                // a nearest-depth resolve produces a one-pixel halo at silhouettes.
                if !skip_capture {
                    self.ocean_optics.capture(
                        &self.device,
                        &mut encoder,
                        color_view,
                        &self.targets.depth_view,
                        optical_ocean,
                        scissor,
                    );
                }
                info.ocean.optics_unbounded = water_unbounded;
                if let Some((x0, y0, x1, y1)) = water_rect {
                    info.ocean.optics_rect =
                        [x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32];
                }
                info.ocean.optics_active = !skip_capture;
                info.ocean.submerged = submerged;
                info.ocean.optics_pixels = scissor
                    .map(|(_, _, w, h)| u64::from(w) * u64::from(h))
                    .unwrap_or_else(|| {
                        if skip_capture {
                            0
                        } else {
                            u64::from(self.config.width) * u64::from(self.config.height)
                        }
                    });
                if submerged {
                    self.ocean_optics
                        .composite(&mut encoder, color_view, resolve_target);
                }
                self.gpu_profiler
                    .write_encoder_timestamp(&mut encoder, GpuPass::OceanOptics, true);
                // Only run the query when something consumes it. An occlusion
                // query can suppress early-Z on a depth-writing, discarding
                // pass like this one, so turning both consumers off has to turn
                // the query off too - otherwise its cost is unmeasurable.
                let ocean_query = (optical.cull || !self.ocean_settings.simulate_when_hidden)
                    .then(|| self.ocean_visibility.query_index())
                    .flatten();
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA ocean surface and diagnostics"),
                    occlusion_query_set: ocean_query.map(|_| self.ocean_visibility.query_set()),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: color_view,
                        depth_slice: None,
                        resolve_target,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.targets.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                self.gpu_profiler
                    .write_render_pass_timestamp(&mut pass, GpuPass::OceanSurface, false);
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(&active_pipeline_variant.pipelines[&ocean_pipeline_key()]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(2, &world.lighting_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                // Count the water samples that actually reach the framebuffer.
                // This is the only honest answer to "is the ocean visible": the
                // shape mask discards and world geometry occludes after every
                // CPU test has already said yes.
                if let Some(index) = ocean_query {
                    pass.begin_occlusion_query(index);
                    ocean_query_encoded = true;
                }
                let mut ocean_drawn = 0u8;
                for &batch_index in active.indices {
                    let batch = &active.batches[batch_index];
                    if batch.ocean_clipmap.is_none() {
                        continue;
                    }
                    if !batch_area_visible(batch, effective_area_mask) {
                        info.ocean.rejected_area =
                            info.ocean.rejected_area.saturating_add(1);
                        continue;
                    }
                    // Note: this culls the water *faces*' bounds, not the mesh
                    // that gets drawn. The clipmap is camera-centred and far
                    // larger than the footprint, so any surviving sliver
                    // submits the whole clipmap for that surface.
                    if !aabb_intersects_clip_frustum(batch.bounds_min, batch.bounds_max, view_proj)
                    {
                        info.ocean.rejected_frustum =
                            info.ocean.rejected_frustum.saturating_add(1);
                        continue;
                    }
                    info.ocean.clipmap_batches =
                        info.ocean.clipmap_batches.saturating_add(1);
                    if !claim_ocean_clipmap(&mut ocean_drawn, batch) {
                        info.ocean.rejected_duplicate =
                            info.ocean.rejected_duplicate.saturating_add(1);
                        continue;
                    }
                    info.ocean.clipmaps_drawn = info.ocean.clipmaps_drawn.saturating_add(1);
                    if let Some(ranges) = batch.ocean_clipmap.as_ref() {
                        let range = &ranges[ocean_clipmap.unwrap_or(0).min(1)];
                        info.ocean.triangles = info
                            .ocean
                            .triangles
                            .saturating_add(u64::from(range.end.saturating_sub(range.start)) / 3);
                    }
                    let stats_quality = ocean_clipmap.unwrap_or(0).min(1);
                    if let Some(runtime) = world.runtime_ocean_clipmaps.iter().find(|runtime| {
                        runtime.slot == batch.ocean_clipmap_id && runtime.quality == stats_quality
                    }) {
                        info.ocean.minimum_level =
                            info.ocean.minimum_level.max(runtime.current_minimum_level);
                        info.ocean.half_cells =
                            info.ocean.half_cells.max(runtime.current_half_cells);
                    }
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(1,batch.ocean_surface_bind_group.as_ref().unwrap_or(&batch.bind_group),&[]);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(
                        6,
                        Self::ocean_bind_group_for(
                            batch,
                            &self.authored_oceans,
                            &self.ocean,
                            &self.ocean_inert_bind_group,
                        ),
                        &[],
                    );
                    if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                    draw_world_batch(&mut pass, batch, 0..1, ocean_clipmap);
                }
                if ocean_query.is_some() {
                    pass.end_occlusion_query();
                }
                self.gpu_profiler
                    .write_render_pass_timestamp(&mut pass, GpuPass::OceanSurface, true);
            }

            if defer_transparent_for_ocean && !post_ocean_transparent.is_empty() {
                // The ocean pass replaced the BSP pipeline. Re-establish the world
                // bindings once, then consume only the transparent queue collected
                // during the original visibility walk. The batches are not culled or
                // classified again here; their original active-list order is retained.
                current_pipeline = None;
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(
                    2,
                    if enhanced_world_shader {
                        &world.lighting_bind_group
                    } else {
                        &world.lighting_bind_group_lean
                    },
                    &[],
                );
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                if enhanced_world_shader {
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                }
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

                for &active_pos in &post_ocean_transparent {
                    let batch_index = active.indices[active_pos];
                    let batch = &active.batches[batch_index];
                    if current_pipeline != Some(batch.source.pipeline) {
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(
                            &active_pipeline_variant.pipelines[&batch.source.pipeline],
                        );
                        current_pipeline = Some(batch.source.pipeline);
                    }
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(1, &batch.bind_group, &[]);
                    if enhanced_world_shader {
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(
                            6,
                            Self::ocean_bind_group_for(
                                batch,
                                &self.authored_oceans,
                                &self.ocean,
                                &self.ocean_inert_bind_group,
                            ),
                            &[],
                        );
                    }
                    if self.gpu_driven_enabled {
                        let offset = u64::from(batch.cull_index)
                            * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
                        if capture_api_calls { info.perf_encode_calls.draw_indexed_indirect = info.perf_encode_calls.draw_indexed_indirect.saturating_add(1); }
                        pass.draw_indexed_indirect(&world.indirect_buffer, offset);
                    } else {
                        if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                        draw_world_batch(&mut pass, batch, 0..1, None);
                    }

                    // Preserve the existing OpenJK material-then-fog ordering.
                    // `active_pos` points back into the original ordered list, so a
                    // depth-writing sibling stage that stayed pre-water still counts
                    // when selecting the depth-equal vs depth-test fog pipeline.
                    let next_is_same_surface =
                        active
                            .indices
                            .get(active_pos + 1)
                            .is_some_and(|&next_index| {
                                same_world_surface(batch, &active.batches[next_index])
                            });
                    let surface_writes_depth = active.indices[..=active_pos]
                        .iter()
                        .rev()
                        .map(|&index| &active.batches[index])
                        .take_while(|sibling| same_world_surface(batch, sibling))
                        .any(|sibling| sibling.source.pipeline.depth_write);
                    let needs_fog_pass = !next_is_same_surface
                        && self.weather.fog.legacy_uses_separate_pass(
                            batch.source.fog_is_global,
                            batch.source.legacy2_fog_in_stage_safe,
                            batch.source.global_fog_post_eligible,
                        )
                        && (batch.source.fog[3] > 0.001
                            || self.weather.fog.legacy_drawfog_value() == 1);
                    if needs_fog_pass {
                        if let Some(fog_pipeline) = active_pipeline_variant
                            .fog_pass_pipelines
                            .get(&(batch.source.pipeline, surface_writes_depth))
                        {
                            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                            pass.set_pipeline(fog_pipeline);
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(1, &batch.bind_group, &[]);
                            if enhanced_world_shader {
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(
                                    6,
                                    Self::ocean_bind_group_for(
                                        batch,
                                        &self.authored_oceans,
                                        &self.ocean,
                                        &self.ocean_inert_bind_group,
                                    ),
                                    &[],
                                );
                            }
                            if self.gpu_driven_enabled {
                                let offset = u64::from(batch.cull_index)
                                    * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
                                if capture_api_calls { info.perf_encode_calls.draw_indexed_indirect = info.perf_encode_calls.draw_indexed_indirect.saturating_add(1); }
                                pass.draw_indexed_indirect(&world.indirect_buffer, offset);
                            } else {
                                if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                                draw_world_batch(&mut pass, batch, 0..1, None);
                            }
                            current_pipeline = None;
                        }
                    }
                }
            }
            self.post_ocean_transparent_scratch = post_ocean_transparent;
            // Ground truth, for the next frame's gates. `clipmaps_drawn` is NOT
            // usable here: a promoted surface's bounds routinely surround the
            // camera, so it reads 1 while the mask and world occluders leave
            // nothing on screen. Until the first readback lands, assume visible.
            let water_on_screen = !self.ocean_visibility.primed
                || self.ocean_visibility.latest_samples > 0
                || info.ocean.submerged;
            if water_on_screen {
                self.ocean_hidden_time = 0.0;
            } else {
                self.ocean_hidden_time += frame_delta.max(0.0);
            }

            if let Some(ocean) = self.ocean.as_ref() {
                ocean.draw_spray(
                    &mut pass,
                    &self.camera_bind_group,
                    self.ocean_spray_pipeline.as_ref(),
                );
            }
            for (_, ocean) in &self.authored_oceans {
                ocean.draw_spray(
                    &mut pass,
                    &self.camera_bind_group,
                    self.ocean_spray_pipeline.as_ref(),
                );
            }

            if split_dynamic_for_ocean {
                self.gpu_profiler.write_render_pass_timestamp(
                    &mut pass,
                    GpuPass::DynamicModelsTranslucent,
                    false,
                );
                self.dynamic_model_renderer.draw_phase(
                    &mut pass,
                    &self.camera_bind_group,
                    false,
                    self.ray_traced_shadows
                        .as_ref()
                        .filter(|_| {
                            self.dynamic_lights_mode == DynamicLightsMode::RayTracedHardware
                        })
                        .and_then(|rt| {
                            rt.model_receivers
                                .as_ref()
                                .map(|models| (models, &rt.receiver_bind_group))
                        }),
                    Some(&mut self.gpu_profiler),
                    None,
                    false,
                );
                self.gpu_profiler.write_render_pass_timestamp(
                    &mut pass,
                    GpuPass::DynamicModelsTranslucent,
                    true,
                );
            }

            // DynamicModelRenderer binds its own transient entity vertex/index
            // buffers. Everything below this point (wireframe, surface inspector, PVS
            // debug) addresses BSP/world index ranges, so restore the world geometry
            // bindings unconditionally before any of those debug draws run.
            if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
            pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
            if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
            pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
            if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
            pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

            if self.wireframe_mask != 0 {
                let mask = self.wireframe_mask;
                if mask & (ui::wireframe::MAP | ui::wireframe::ENTITIES) != 0 {
                    if let Some(pipeline) = &self.wireframe_pipeline {
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(0, &self.camera_bind_group, &[]);
                        for &batch_index in active.indices {
                            let batch = &active.batches[batch_index];
                            let category_enabled = if batch.is_inline_entity {
                                mask & ui::wireframe::ENTITIES != 0
                            } else {
                                mask & ui::wireframe::MAP != 0
                            };
                            if category_enabled
                                && (!render_ocean || batch.ocean_clipmap.is_none())
                                && batch.source.pipeline.class != DrawClass::Sky
                                && aabb_intersects_clip_frustum(
                                    batch.bounds_min,
                                    batch.bounds_max,
                                    view_proj,
                                )
                            {
                                if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                                draw_world_batch(&mut pass, batch, 0..1, None);
                            }
                        }
                    }
                }

                let variant_key = world.active_pipeline_variant;
                let variant_family = variant_key.family();
                if let Some(pipeline) = self.world_wireframe_pipelines.get(&variant_key) {
                    if mask & ui::wireframe::OCEAN != 0
                        && render_ocean
                        && variant_family == WorldShaderFamily::Enhanced
                    {
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(0, &self.camera_bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(2, &world.lighting_bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                        pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                        if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                        pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                        if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                        pass.set_index_buffer(
                            world.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        let mut ocean_drawn = 0u8;
                        for &batch_index in active.indices {
                            let batch = &active.batches[batch_index];
                            if batch.ocean_clipmap.is_none()
                                || !batch_area_visible(batch, effective_area_mask)
                                || !aabb_intersects_clip_frustum(
                                    batch.bounds_min,
                                    batch.bounds_max,
                                    view_proj,
                                )
                                || !claim_ocean_clipmap(&mut ocean_drawn, batch)
                            {
                                continue;
                            }
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(1, &batch.bind_group, &[]);
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            pass.set_bind_group(
                                6,
                                Self::ocean_bind_group_for(
                                    batch,
                                    &self.authored_oceans,
                                    &self.ocean,
                                    &self.ocean_inert_bind_group,
                                ),
                                &[],
                            );
                            if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                            draw_world_batch(&mut pass, batch, 0..1, ocean_clipmap);
                        }
                    }

                    if mask & ui::wireframe::DEFORMATION != 0 {
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(0, &self.camera_bind_group, &[]);
                        match variant_family {
                            WorldShaderFamily::Lean => {
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(2, &world.lighting_bind_group_lean, &[]);
                            }
                            WorldShaderFamily::Enhanced => {
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(2, &world.lighting_bind_group, &[]);
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                            }
                        }
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                        pass.set_vertex_buffer(
                            1,
                            world.snow_shell.legacy_dlight_surface_id_buffer.slice(..),
                        );
                        for chunk in world.snow_shell.chunks.values() {
                            let Some(vertex_buffer) = &chunk.vertex_buffer else {
                                continue;
                            };
                            if !chunk.near_center(snow_deform_center) {
                                continue;
                            }
                            if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                            for draw in &chunk.draws {
                                if !snow_shell_draw_near_center(draw, snow_deform_center)
                                    || !aabb_intersects_clip_frustum(
                                        draw.bounds_min,
                                        draw.bounds_max,
                                        view_proj,
                                    )
                                {
                                    continue;
                                }
                                let Some(batch) = world.coarse_batches.get(draw.coarse_batch_index)
                                else {
                                    continue;
                                };
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                pass.set_bind_group(1, &batch.bind_group, &[]);
                                if variant_family == WorldShaderFamily::Enhanced {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            batch,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                                pass.draw(draw.vertices.clone(), 1..2);
                            }
                        }
                    }
                }

                if mask & ui::wireframe::GRASS != 0 {
                    if let (Some(grass), Some(prepared)) = (&world.grass, prepared_grass.as_ref()) {
                        self.grass_renderer.draw_wireframe(
                            &mut pass,
                            &self.camera_bind_group,
                            &self.shadow_resources.receiver_bind_group,
                            grass,
                            prepared,
                        );
                    }
                }

                if mask & ui::wireframe::EFFECTS != 0 {
                    if let Some(prepared) = prepared_surface_sprite_effects.as_ref() {
                        self.surface_sprite_effect_renderer.draw_wireframe(
                            &mut pass,
                            &self.camera_bind_group,
                            &world.surface_sprite_effects,
                            prepared,
                        );
                    }
                }

                let dynamic_mask =
                    ui::wireframe::PLAYERS | ui::wireframe::ENTITIES | ui::wireframe::EFFECTS;
                if mask & dynamic_mask != 0 {
                    self.dynamic_model_renderer.draw_wireframe(
                        &mut pass,
                        &self.camera_bind_group,
                        mask,
                    );
                }

                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            }
            if self.debug_volumes.draw(&mut pass, &self.camera_bind_group) {
                // The overlay bound its own buffers; the inspector below addresses BSP ranges.
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                pass.set_index_buffer(world.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            }
            if let (Some(vertices), Some(pipeline)) = (
                &self.inspector_vertex_range,
                self.surface_inspector_pipeline.as_ref(),
            ) {
                if let Some(indices) = inspector_index_range(active.batches, vertices) {
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(pipeline);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                    pass.draw_indexed(indices, 0, 0..1);
                }
            }
            if let (CullDebugMode::RejectionReasons, Some(cull_debug_pipeline)) =
                (self.cull_debug_mode, self.cull_debug_pipeline.as_ref())
            {
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(cull_debug_pipeline);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(1, &world.cull_debug_bind_group, &[]);
                // Draw the entire PVS representation, not merely its active
                // subset. The shader discards visible records and colors only
                // the rejection stage recorded for each batch.
                for batch in active.batches {
                    if batch.source.pipeline.class == DrawClass::Sky {
                        continue;
                    }
                    let first_instance = batch.cull_index;
                    if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                    draw_world_batch(
                        &mut pass,
                        batch,
                        first_instance..first_instance.saturating_add(1),
                        None,
                    );
                }
            }
            self.weather
                .atmosphere
                .render(&mut pass, &self.camera_bind_group);
            if self.weather.rain.enabled {
                self.gpu_profiler
                    .write_render_pass_timestamp(&mut pass, GpuPass::RainDraw, false);
            }
            self.weather.rain.render(&mut pass, &self.camera_bind_group);
            if self.weather.rain.enabled {
                self.gpu_profiler
                    .write_render_pass_timestamp(&mut pass, GpuPass::RainDraw, true);
            }
            self.weather.snow.render(&mut pass, &self.camera_bind_group);
            self.weather
                .wind_visuals
                .render(&mut pass, &self.camera_bind_group);

            // OpenJK Dynamic Glow is authored at shader-stage granularity: only
            // stages carrying the exact `glow` marker are replayed into a black
            // glow buffer while testing against the already-rendered scene depth.
            // Reuse the normal material pipelines so rgbGen/alphaGen, tcMods,
            // animation, blending and fog stay identical to the visible stage.
            if self.glow_mode.authored() {
                drop(pass);
                if let Some(glow_target) = self.targets.authored_glow.as_ref() {
                    let (glow_color_view, glow_resolve_target) =
                        if let Some(msaa_view) = glow_target.msaa_view.as_ref() {
                            (msaa_view, Some(&glow_target.view))
                        } else {
                            (&glow_target.view, None)
                        };
                    if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                    let mut glow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("JKA authored glow source"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: glow_color_view,
                            depth_slice: None,
                            resolve_target: glow_resolve_target,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.targets.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    glow_pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    glow_pass.set_bind_group(
                        2,
                        if enhanced_world_shader {
                            &world.lighting_bind_group
                        } else {
                            &world.lighting_bind_group_lean
                        },
                        &[],
                    );
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    glow_pass.set_bind_group(3, world_shadow_receiver_bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    glow_pass.set_bind_group(4, &self.planar_reflection.bind_group, &[]);
                    if enhanced_world_shader {
                        let ocean_bind_group = self
                            .ocean
                            .as_ref()
                            .map(|ocean| &ocean.render_bind_group)
                            .unwrap_or(&self.ocean_inert_bind_group);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        glow_pass.set_bind_group(5, &self.ocean_optics.bind_group, &[]);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        glow_pass.set_bind_group(6, ocean_bind_group, &[]);
                    }
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    glow_pass.set_vertex_buffer(0, world.vertex_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_vertex_buffer = info.perf_encode_calls.set_vertex_buffer.saturating_add(1); }
                    glow_pass.set_vertex_buffer(1, world.legacy_dlight_surface_id_buffer.slice(..));
                    if capture_api_calls { info.perf_encode_calls.set_index_buffer = info.perf_encode_calls.set_index_buffer.saturating_add(1); }
                    glow_pass.set_index_buffer(
                        world.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut current_glow_pipeline = None;
                    for &batch_index in active.indices {
                        let batch = &active.batches[batch_index];
                        if !batch.source.authored_glow
                            || batch.is_inline_entity
                            || !batch_area_visible(batch, effective_area_mask)
                            || ocean_suppresses_batch(render_ocean, batch)
                        {
                            continue;
                        }
                        if batch.source.pipeline.class != DrawClass::Sky
                            && !aabb_intersects_clip_frustum(
                                batch.bounds_min,
                                batch.bounds_max,
                                view_proj,
                            )
                        {
                            continue;
                        }
                        if current_glow_pipeline != Some(batch.source.pipeline) {
                            let Some(pipeline) = active_pipeline_variant
                                .pipelines
                                .get(&batch.source.pipeline)
                            else {
                                continue;
                            };
                            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                            glow_pass.set_pipeline(pipeline);
                            current_glow_pipeline = Some(batch.source.pipeline);
                        }
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        glow_pass.set_bind_group(
                            1,
                            batch
                                .authored_glow_bind_group
                                .as_ref()
                                .unwrap_or(&batch.bind_group),
                            &[],
                        );
                        if enhanced_world_shader {
                            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                            glow_pass.set_bind_group(
                                6,
                                Self::ocean_bind_group_for(
                                    batch,
                                    &self.authored_oceans,
                                    &self.ocean,
                                    &self.ocean_inert_bind_group,
                                ),
                                &[],
                            );
                        }
                        if capture_api_calls { info.perf_encode_calls.world_helper_draw_indexed = info.perf_encode_calls.world_helper_draw_indexed.saturating_add(1); }
                        draw_world_batch(&mut glow_pass, batch, 0..1, None);
                    }

                    // Inline BSP movers are submitted as ordinary authored stages too.
                    // Re-pack just the visible `glow` stages so their normal mover
                    // transforms/order are preserved without a separate material path.
                    let glow_inline_visible = self
                        .inline_visible_scratch
                        .iter()
                        .copied()
                        .filter(|&index| active.batches[index].source.authored_glow)
                        .collect::<Vec<_>>();
                    if !glow_inline_visible.is_empty() {
                        let packed = pack_inline_draws(
                            &self.queue,
                            active.batches,
                            &glow_inline_visible,
                            &mut self.inline_draw_scratch,
                            world.inline_group_count,
                            &world.inline_indirect_buffer,
                            0,
                            world.inline_indirect_capacity,
                        );
                        let scratch = &self.inline_draw_scratch;
                        if packed {
                            for &(first, count) in &scratch.runs {
                                let batch = &active.batches[glow_inline_visible
                                    [scratch.order[first as usize] as usize]];
                                if current_glow_pipeline != Some(batch.source.pipeline) {
                                    if let Some(pipeline) = active_pipeline_variant
                                        .pipelines
                                        .get(&batch.source.pipeline)
                                    {
                                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                        glow_pass.set_pipeline(pipeline);
                                        current_glow_pipeline = Some(batch.source.pipeline);
                                    } else {
                                        continue;
                                    }
                                }
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                glow_pass.set_bind_group(
                            1,
                            batch
                                .authored_glow_bind_group
                                .as_ref()
                                .unwrap_or(&batch.bind_group),
                            &[],
                        );
                                if enhanced_world_shader {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    glow_pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            batch,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if count == 1 {
                                    if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                                    glow_pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                                } else {
                                    if capture_api_calls { info.perf_encode_calls.multi_draw_indexed_indirect = info.perf_encode_calls.multi_draw_indexed_indirect.saturating_add(1); }
                                    glow_pass.multi_draw_indexed_indirect(
                                        &world.inline_indirect_buffer,
                                        u64::from(first)
                                            * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
                                        count,
                                    );
                                }
                            }
                        } else {
                            for &batch_index in &glow_inline_visible {
                                let batch = &active.batches[batch_index];
                                if current_glow_pipeline != Some(batch.source.pipeline) {
                                    let Some(pipeline) = active_pipeline_variant
                                        .pipelines
                                        .get(&batch.source.pipeline)
                                    else {
                                        continue;
                                    };
                                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                                    glow_pass.set_pipeline(pipeline);
                                    current_glow_pipeline = Some(batch.source.pipeline);
                                }
                                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                glow_pass.set_bind_group(
                            1,
                            batch
                                .authored_glow_bind_group
                                .as_ref()
                                .unwrap_or(&batch.bind_group),
                            &[],
                        );
                                if enhanced_world_shader {
                                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                                    glow_pass.set_bind_group(
                                        6,
                                        Self::ocean_bind_group_for(
                                            batch,
                                            &self.authored_oceans,
                                            &self.ocean,
                                            &self.ocean_inert_bind_group,
                                        ),
                                        &[],
                                    );
                                }
                                if capture_api_calls { info.perf_encode_calls.draw_indexed = info.perf_encode_calls.draw_indexed.saturating_add(1); }
                                glow_pass.draw_indexed(batch.indexed_range.clone(), 0, 0..1);
                            }
                        }
                    }

                    self.dynamic_model_renderer
                        .draw_authored_glow(&mut glow_pass, &self.camera_bind_group);
                }
            }
        } else {
            // Worldless live/demo sessions intentionally have no BSP, but their
            // CGame still produces players, sabers, missiles and FX. Render those
            // against an empty depth buffer instead of replacing the scene with
            // a clear-only pass.
            let (color_view, resolve_target) = if let Some(msaa_view) = &self.targets.msaa_view {
                (msaa_view, Some(scene_output))
            } else {
                (scene_output, None)
            };
            if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("JKA worldless dynamic scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.weather.fog.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(camera_depth_clear()),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.gpu_profiler
                .write_render_pass_timestamp(&mut pass, GpuPass::DynamicModels, false);
            for depth_writing_phase in [true, false] {
                self.dynamic_model_renderer.draw_phase(
                    &mut pass,
                    &self.camera_bind_group,
                    depth_writing_phase,
                    None,
                    Some(&mut self.gpu_profiler),
                    None,
                    false,
                );
            }
            self.gpu_profiler
                .write_render_pass_timestamp(&mut pass, GpuPass::DynamicModels, true);
            let dynamic_mask =
                ui::wireframe::PLAYERS | ui::wireframe::ENTITIES | ui::wireframe::EFFECTS;
            if self.wireframe_mask & dynamic_mask != 0 {
                self.dynamic_model_renderer.draw_wireframe(
                    &mut pass,
                    &self.camera_bind_group,
                    self.wireframe_mask,
                );
            }
            self.debug_volumes.draw(&mut pass, &self.camera_bind_group);
            if self.glow_mode.authored() {
                drop(pass);
                if let Some(glow_target) = self.targets.authored_glow.as_ref() {
                    let (glow_color_view, glow_resolve_target) =
                        if let Some(msaa_view) = glow_target.msaa_view.as_ref() {
                            (msaa_view, Some(&glow_target.view))
                        } else {
                            (&glow_target.view, None)
                        };
                    if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                    let mut glow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("JKA worldless authored glow source"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: glow_color_view,
                            depth_slice: None,
                            resolve_target: glow_resolve_target,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.targets.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    self.dynamic_model_renderer
                        .draw_authored_glow(&mut glow_pass, &self.camera_bind_group);
                }
            }
        }
        self.gpu_profiler
            .write_encoder_timestamp(&mut encoder, GpuPass::World, true);
        // World flares are separate from BSP material geometry; sample the
        // completed opaque depth before screen-space post processing.
        if self.world_flares_enabled && self.world.is_some() {
            let (flare_color_view, flare_resolve) = if let Some(view) = &self.targets.msaa_view {
                (view, Some(scene_output))
            } else {
                (scene_output, None)
            };
            let flare_format = flare_scene_format;
            self.world_flares.draw(
                &self.device,
                &self.queue,
                &mut encoder,
                flare_color_view,
                flare_resolve,
                &self.targets.depth_view,
                flare_format,
                self.msaa_samples,
                &render_camera,
                self.config.width,
                self.config.height,
                self.world.as_ref().and_then(|world| world.visibility.as_ref()),
                self.pvs_mode != PvsMode::Off,
                self.weather.fog.legacy_drawfog_value(),
            );
        }
        self.compact_group_scratch = compact_group_drawn;
        if let Some(started) = perf_scene_started {
            info.perf_encode_scene_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let perf_post_ui_started = perf_timing.then(Instant::now);

        self.gpu_profiler
            .write_encoder_timestamp(&mut encoder, GpuPass::Post, false);

        // Fog does not need this: entities are in the linear-depth prepass, so
        // it already is the authoritative opaque scene depth. Only SSR still
        // uses the resolve, to reject reflections under non-prepass geometry
        // (grass, promoted ocean) drawn over a reflective BSP surface.
        let needs_resolved_scene_depth = self.ssr_enabled || self.reflection_debug_enabled;
        if needs_resolved_scene_depth {
            // Resolve the final opaque device depth after players, vehicles and
            // promoted ocean have rendered. SSR compares its reconstructed
            // world-space distance with the linear-depth prepass.
            if let (Some(view), Some(pipeline), Some(bind_group)) = (
                self.targets.ssr_visibility_view.as_ref(),
                self.ssr_visibility_pipeline.as_ref(),
                self.ssr_visibility_bind_group.as_ref(),
            ) {
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA resolved final scene depth"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if self.world.is_some() {
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(pipeline);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, bind_group, &[]);
                    if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                    pass.draw(0..3, 0..1);
                }
            }
        }

        if self.ssao_enabled {
            if let (Some(history), Some(bind_groups)) =
                (&self.targets.ssao_history, &self.ssao_temporal_bind_groups)
            {
                let history_write_index = 1 - self.ssao_history_read_index;
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA half-resolution temporal SSAO"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &history.views[history_write_index],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 1.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(&self.ssao_temporal_pipeline);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, &bind_groups[self.ssao_history_read_index], &[]);
                if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                pass.draw(0..3, 0..1);
                drop(pass);
                self.ssao_history_read_index = history_write_index;
            }
        }

        if self.ssr_enabled {
            if let (Some(history), Some(bind_groups)) =
                (&self.targets.ssr_history, &self.ssr_temporal_bind_groups)
            {
                let history_write_index = 1 - self.ssr_history_read_index;
                let color_attachments = [
                    Some(wgpu::RenderPassColorAttachment {
                        view: &history.radiance_views[history_write_index],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &history.depth_views[history_write_index],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ];
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA half-resolution temporal SSR"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(&self.ssr_temporal_pipeline);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, &bind_groups[self.ssr_history_read_index], &[]);
                if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                pass.draw(0..3, 0..1);
                drop(pass);
                self.ssr_history_read_index = history_write_index;
            }
        }

        if self.hdr_enabled && self.tone_mapping_enabled && self.auto_exposure_enabled {
            // Inlined rather than called via &self methods: a live borrow of
            // self.smaa_target (through smaa_frame) is in scope here, and a
            // method call would reborrow all of *self.
            let uniform = AutoExposureUniform {
                params: [1.0, frame_delta.clamp(0.0, 0.25), -2.0, 2.0],
                // 18% middle gray, with slower opening into darkness than closing
                // down on a suddenly bright scene. These are intentionally fixed
                // policy constants for r_autoExposure rather than extra user knobs.
                adaptation: [0.18, 1.5, 3.0, 0.0],
            };
            self.queue.write_buffer(
                &self.auto_exposure_settings_buffer,
                0,
                bytemuck::bytes_of(&uniform),
            );
            if capture_api_calls { info.perf_encode_calls.compute_passes = info.perf_encode_calls.compute_passes.saturating_add(1); }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JKA auto-exposure luminance/adaptation"),
                timestamp_writes: None,
            });
            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
            pass.set_pipeline(&self.auto_exposure_pipeline);
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(0, &self.auto_exposure_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        if self.depth_of_field_strength > 0.001 {
            if let (Some(target), Some(bind_group)) = (&self.targets.dof, &self.dof_bind_group) {
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA Gaussian DOF horizontal"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(&self.dof_pipeline);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(0, bind_group, &[]);
                if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                pass.draw(0..3, 0..1);
            }
        }

        if self.glow_mode.enabled() || self.halation_enabled {
            if let (Some(bloom), Some(bind_groups)) = (&self.targets.bloom, &self.bloom_bind_groups)
            {
                let labels = [
                    "JKA bloom half-resolution extract",
                    "JKA bloom quarter-resolution downsample",
                    "JKA bloom eighth-resolution downsample",
                ];
                let extraction = match self.glow_mode {
                    GlowMode::JkaAuthored if self.halation_enabled => {
                        self.bloom_extract_authored_halation_pipeline.as_ref()
                    }
                    GlowMode::JkaAuthored => self.bloom_extract_authored_pipeline.as_ref(),
                    GlowMode::JkaAndBloom => self.bloom_extract_combined_pipeline.as_ref(),
                    // Off still needs modern highlights when halation is enabled.
                    GlowMode::ModernBloom | GlowMode::Off => {
                        self.bloom_extract_modern_pipeline.as_ref()
                    }
                };
                let pipelines = extraction.zip(self.bloom_downsample_pipeline.as_ref());
                for level in 0..3 {
                    if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some(labels[level]),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &bloom.views[level],
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    // Clear every level until both worker results are installed;
                    // a mode change must not display stale or partial glow.
                    let Some((extraction, downsample)) = pipelines else {
                        continue;
                    };
                    let pipeline = if level == 0 { extraction } else { downsample };
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(pipeline);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, &bind_groups[level], &[]);
                    if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                    pass.draw(0..3, 0..1);
                }
            }
        }

        if self.weather.rain.enabled
            && !self.weather.rain.fast_post
            && plan.use_post
            && !plan.gamma_only_post
        {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RainHaze, false);
            self.weather.rain.dispatch_haze(
                &mut encoder,
                self.targets.rain_haze_mask_width,
                self.targets.rain_haze_mask_height,
            );
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RainHaze, true);
        }

        if plan.use_post && self.clouds_enabled {
            // Raymarch visible clouds into an independent transfer buffer. RGB
            // carries in-scattered cloud light and A carries sky transmittance.
            // The pass can therefore run below display resolution and be
            // bilinearly composited over the full-resolution sky later.
            //
            // Temporal mode uses a 2x2 interleave: after the first complete
            // history frame, only one quarter of the cloud-target pixels execute
            // the expensive raymarch each frame. The other three quarters are
            // reconstructed by reprojection from the previous ping-pong target.
            // Always ping-pong because sampling and writing one texture in the
            // same render pass is invalid in wgpu regardless of shader control
            // flow.
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::Clouds, false);
            let cloud_write_index = 1 - self.cloud_history_read_index;
            let post_bind_group = &self.post_bind_groups[self.history_read_index]
                [self.ssao_history_read_index][self.ssr_history_read_index]
                [self.cloud_history_read_index];
            let cloud_clear = wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            };

            // Pass one: march. With the interleave on, only one pixel per block
            // writes a real sample here.
            let march_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.targets.cloud_march_view,
                depth_slice: None,
                resolve_target: None,
                ops: cloud_clear,
            })];
            if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("JKA volumetric cloud march"),
                color_attachments: &march_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
            pass.set_pipeline(&self.cloud_march_pipeline);
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(0, post_bind_group, &[]);
            if self.cloud_temporal_enabled && self.cloud_history_valid {
                // Interleaved: draw only the compact block grid so every lane
                // in every wave does a real march (see fs_cloud_march). The
                // grid size is the same constant the post uniform carries.
                let grid = CLOUD_INTERLEAVE_GRID;
                let blocks_x = self.targets.cloud_width().div_ceil(grid).max(1);
                let blocks_y = self.targets.cloud_height().div_ceil(grid).max(1);
                pass.set_viewport(0.0, 0.0, blocks_x as f32, blocks_y as f32, 0.0, 1.0);
                pass.set_scissor_rect(0, 0, blocks_x, blocks_y);
            }
            if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
            pass.draw(0..3, 0..1);
            drop(pass);

            // Pass two: resolve. Reconstructs every pixel from the sparse
            // lattice, then blends reprojected history clamped into that
            // lattice's range.
            let resolve_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.targets.cloud_transfer_views[cloud_write_index],
                depth_slice: None,
                resolve_target: None,
                ops: cloud_clear,
            })];
            if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("JKA volumetric cloud resolve"),
                color_attachments: &resolve_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
            pass.set_pipeline(&self.cloud_resolve_pipeline);
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(0, post_bind_group, &[]);
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(1, &self.cloud_resolve_bind_group, &[]);
            if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
            pass.draw(0..3, 0..1);
            drop(pass);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::Clouds, true);

            self.cloud_history_read_index = cloud_write_index;
            self.cloud_history_valid = true;
            if self.cloud_temporal_enabled {
                self.cloud_temporal_frame_index = (self.cloud_temporal_frame_index + 1) & 3;
            } else {
                self.cloud_temporal_frame_index = 0;
            }
        }

        let post_output = if rain_glass_active {
            self.weather
                .rain
                .lens_glass_source_view()
                .expect("active RainyGlass source view")
        } else {
            final_output
        };
        let mut taa_resolved_this_frame = false;
        if plan.use_post {
            let regional = rt_taa_active.then_some(self.rt_taa_resources.as_ref()).flatten()
                .filter(|r| r.input.is_some() && r.pipeline.is_some());
            let temporal_pipeline = if self.taa_enabled { self.taa_post_pipeline.as_ref() }
                else { regional.and_then(|r| r.pipeline.as_ref()) };
            if let Some(taa_post_pipeline) = temporal_pipeline {
                // Persist the *resolved* temporal color, not the raw jittered
                // scene. Ping-pong history avoids sampling and writing the same
                // texture while keeping TAA inside the existing fullscreen pass.
                let history_write_index = 1 - self.history_read_index;
                let color_attachments = [
                    Some(wgpu::RenderPassColorAttachment {
                        view: post_output,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.history_views[history_write_index],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ];
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA TAA/post-process resolve"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                pass.set_pipeline(taa_post_pipeline);
                if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                pass.set_bind_group(
                    0,
                    &self.post_bind_groups[self.history_read_index][self.ssao_history_read_index]
                        [self.ssr_history_read_index][self.cloud_history_read_index],
                    &[],
                );
                if let Some(resources) = regional {
                    pass.set_bind_group(1, resources.input.as_ref().unwrap(), &[]);
                }
                if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                pass.draw(0..3, 0..1);
                drop(pass);
                self.history_read_index = history_write_index;
                taa_resolved_this_frame = true;
            } else {
                let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                    view: post_output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })];
                if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JKA brightness/gamma post-process"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if plan.rain_only_post {
                    if let Some(pipeline) = self.rain_post_pipeline.as_ref() {
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(
                            0,
                            &self.post_bind_groups[self.history_read_index]
                                [self.ssao_history_read_index][self.ssr_history_read_index]
                                [self.cloud_history_read_index],
                            &[],
                        );
                    } else if let Some(pipeline) = self.post_pipeline.as_ref() {
                        // Keep visuals intact while the A/B PSO compiles.
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(
                            0,
                            &self.post_bind_groups[self.history_read_index]
                                [self.ssao_history_read_index][self.ssr_history_read_index]
                                [self.cloud_history_read_index],
                            &[],
                        );
                    } else {
                        if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                        pass.set_pipeline(&self.gamma_post_pipeline);
                        if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                        pass.set_bind_group(0, &self.gamma_post_bind_group, &[]);
                    }
                } else if plan.gamma_only_post || self.post_pipeline.is_none() {
                    // Full post may still be compiling on a worker. Never block a
                    // gameplay frame for it; the basic copy/gamma path is a safe
                    // temporary presentation fallback.
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(&self.gamma_post_pipeline);
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(0, &self.gamma_post_bind_group, &[]);
                } else {
                    if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
                    pass.set_pipeline(self.post_pipeline.as_ref().expect("checked above"));
                    if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
                    pass.set_bind_group(
                        0,
                        &self.post_bind_groups[self.history_read_index]
                            [self.ssao_history_read_index][self.ssr_history_read_index]
                            [self.cloud_history_read_index],
                        &[],
                    );
                }
                if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
                pass.draw(0..3, 0..1);
            }
        }
        if rain_glass_active {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RainLens, false);
            self.weather.rain.render_lens_glass(&mut encoder, final_output);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::RainLens, true);
        }
        self.gpu_profiler
            .write_encoder_timestamp(&mut encoder, GpuPass::Post, true);

        if matches!(
            self.planar_reflection_debug_mode,
            PlanarReflectionDebugMode::ReflectionTexture | PlanarReflectionDebugMode::BindingTest
        ) && self.planar_reflection.active
            && planar_views.iter().any(|view| view.is_some())
        {
            let frame_width = self.config.width.max(1) as f32;
            let frame_height = self.config.height.max(1) as f32;
            let margin = 16.0_f32.min(frame_width * 0.02);
            let preview_width = (frame_width * 0.36)
                .clamp(160.0, 560.0)
                .min(frame_width - margin * 2.0);
            let aspect = frame_height / frame_width;
            let preview_height = (preview_width * aspect).min(frame_height * 0.42);
            let preview_x = (frame_width - preview_width - margin).max(0.0);
            let preview_y = margin.max(0.0);
            if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("JKA planar reflection debug texture preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: final_output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if capture_api_calls { info.perf_encode_calls.set_pipeline = info.perf_encode_calls.set_pipeline.saturating_add(1); }
            pass.set_pipeline(&self.planar_debug_pipeline);
            if capture_api_calls { info.perf_encode_calls.set_bind_group = info.perf_encode_calls.set_bind_group.saturating_add(1); }
            pass.set_bind_group(0, &self.planar_reflection.bind_group, &[]);
            pass.set_viewport(
                preview_x,
                preview_y,
                preview_width.max(1.0),
                preview_height.max(1.0),
                0.0,
                1.0,
            );
            if capture_api_calls { info.perf_encode_calls.draw = info.perf_encode_calls.draw.saturating_add(1); }
            pass.draw(0..3, 0..1);
        }

        if taa_resolved_this_frame {
            self.history_valid = true;
            self.taa_frame_index = self.taa_frame_index.wrapping_add(1);
        } else {
            self.history_valid = false;
            self.taa_frame_index = 0;
            self.history_read_index = 0;
        }
        if self.ssao_enabled {
            self.ssao_history_valid = true;
        } else {
            self.ssao_history_valid = false;
            self.ssao_history_read_index = 0;
        }
        if self.ssr_enabled {
            self.ssr_history_valid = true;
            self.ssr_frame_index = self.ssr_frame_index.wrapping_add(1);
        } else {
            self.ssr_history_valid = false;
            self.ssr_history_read_index = 0;
            self.ssr_frame_index = 0;
        }
        self.previous_view_proj = view_proj;
        self.previous_unjittered_view_proj = unjittered_view_proj;
        self.previous_cloud_view_proj = cloud_view_proj;
        self.previous_camera_position = camera.position;
        self.previous_frame_time = camera_pos_time[3];
        self.camera_history_valid = true;

        self.spectator_highlight.encode(
            &self.device,
            &mut encoder,
            final_output,
            &self.dynamic_model_renderer,
            &self.camera_bind_group,
            &self.targets.depth_view,
            spectator_highlights,
            self.config.width,
            self.config.height,
            self.msaa_samples,
        );

        // Decide whether the native JKA overlay belongs before or after CRT.
        // Scene keeps it crisp. Scene + HUD always sends it through CRT.
        // Everything preserves the old egui layering by deferring the native
        // overlay when egui is active, but both still land in the CRT source.
        let native_ui_in_main = !menu_backdrop_active
            && match self.crt_scope {
                ui::CrtScope::Off => !self.egui_active,
                ui::CrtScope::Scene => false,
                ui::CrtScope::SceneHud => true,
                ui::CrtScope::Everything => !self.egui_active,
            };
        if native_ui_in_main
            && (self.screen_fx_vertex_count != 0
                || self.ui_vertex_count != 0
                || self.ui_dynamic_vertex_count != 0
                || self.ui_transient_vertex_count != 0)
        {
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::Ui, false);
            if capture_api_calls { info.perf_encode_calls.render_passes = info.perf_encode_calls.render_passes.saturating_add(1); }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("JKA overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: final_output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            Self::draw_screen_fx_batches(
                &mut pass,
                &self.screen_fx_vertex_buffer,
                self.screen_fx_vertex_count,
                &self.screen_fx_batches,
                &self.screen_fx_pipelines,
                &self.screen_fx_textures,
                &self.screen_fx_white_bind_group,
            );
            Self::draw_ui_batches(
                &mut pass,
                &self.ui_pipeline,
                &self.ui_bind_group,
                &self.ui_dynamic_vertex_buffer,
                self.ui_dynamic_vertex_count,
                &self.ui_transient_vertex_buffer,
                self.ui_transient_vertex_count,
                &self.ui_vertex_buffer,
                self.ui_vertex_count,
            );
            drop(pass);
            self.gpu_profiler
                .write_encoder_timestamp(&mut encoder, GpuPass::Ui, true);
        }

        // Avoid an extra queue submission when no external SMAA resolve sits
        // between the scene and CRT. This is the common low-latency path.
        let crt_encoded_in_main = crt_before_ui && !self.smaa_enabled;
        if crt_encoded_in_main {
            if let Some(resources) = self.crt_resources.as_ref() {
                // `smaa_frame` keeps a mutable borrow of `self.smaa_target` alive
                // until resolve. Borrow only the disjoint CRT fields here instead
                // of calling a method that would immutably borrow all of `self`.
                Self::encode_crt_resources(
                    resources,
                    self.crt_style,
                    self.crt_quality,
                    &mut encoder,
                    direct_presentation_output,
                );
            }
        }

        self.gpu_profiler
            .write_encoder_timestamp(&mut encoder, GpuPass::Frame, true);
        self.gpu_profiler.resolve(&mut encoder);
        // If the swapchain image is already the final target, fold the readback
        // copy into the frame command buffer instead of paying another submit.
        // SMAA and the menu backdrop both submit work after this encoder, so
        // those paths intentionally do a tiny late copy submit instead.
        if !self.smaa_enabled && !menu_backdrop_active && !self.egui_active && !crt_active {
            if let Some(readback) = screenshot_readback.as_mut() {
                let staging_buffer = &self
                    .screenshot_readback_buffer
                    .as_ref()
                    .expect("screenshot staging buffer must exist after preparation")
                    .buffer;
                Self::encode_screenshot_copy(
                    &mut encoder,
                    &frame.texture,
                    staging_buffer,
                    readback,
                );
                if readback.depth_requested {
                    let depth_staging_buffer = &self
                        .video_depth_readback_buffer
                        .as_ref()
                        .expect("video depth staging buffer must exist after preparation")
                        .buffer;
                    Self::encode_screenshot_copy(
                        &mut encoder,
                        &self.targets._linear_depth_texture,
                        depth_staging_buffer,
                        readback,
                    );
                }
                readback.inline_copy = true;
            }
        }
        // Resolve the ocean occlusion query once every render pass has ended.
        // `encode_resolve` is a no-op unless the ocean pass actually began a
        // query this frame, so an ocean-free frame costs nothing here.
        if ocean_query_encoded {
            self.ocean_visibility.encode_resolve(&mut encoder);
        }
        if let Some(started) = perf_post_ui_started {
            info.perf_encode_post_ui_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let perf_finish_started = perf_timing.then(Instant::now);
        let command_buffer = encoder.finish();
        if let Some(started) = perf_finish_started {
            info.perf_encoder_finish_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let perf_readback_started = perf_timing.then(Instant::now);
        let profiler_readback_copy = self.gpu_profiler.encode_readback_copy(&self.device);
        if let Some(started) = perf_readback_started {
            info.perf_profiler_readback_encode_ms = started.elapsed().as_secs_f64() * 1000.0;
            info.perf_primary_command_buffers = 1 + u32::from(profiler_readback_copy.is_some());
        }
        info.cpu_encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;
        let submit_started = Instant::now();
        info.submit_call_at = Some(submit_started);
        let perf_submit_api_started = perf_timing.then(Instant::now);
        let frame_submission = if let Some(readback_copy) = profiler_readback_copy {
            self.queue.submit([command_buffer, readback_copy])
        } else {
            self.queue.submit([command_buffer])
        };
        if let Some(started) = perf_submit_api_started {
            info.perf_primary_submit_api_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let perf_followup_started = perf_timing.then(Instant::now);
        if let Some(scene_frame) = companion_scene_frame {
            self.companion_scene_ready.push(scene_frame);
        }
        if let Some(readback) = screenshot_readback
            .as_mut()
            .filter(|readback| readback.inline_copy)
        {
            readback.submission_index = Some(frame_submission.clone());
        }
        // The SMAA crate records/submits its three reference passes here. Queue
        // ordering guarantees they consume the color target after the main JKA
        // command buffer and complete before presentation.
        if let Some(smaa_frame) = smaa_frame.take() {
            smaa_frame.resolve();
        }
        // Release the SMAA target borrow before re-borrowing `self` below.
        drop(smaa_frame);
        if crt_before_ui && !crt_encoded_in_main {
            // SMAA resolves in its own submit, so only that path needs a tiny
            // follow-up CRT submit here.
            self.submit_crt(direct_presentation_output);
        }

        let ui_composite_target = if crt_everything {
            crt_view.as_ref().expect("CRT resources must exist while enabled")
        } else {
            &frame_view
        };
        if menu_backdrop_active {
            // In Everything mode the backdrop itself is part of the CRT source;
            // otherwise it goes straight to the swapchain as before.
            self.submit_menu_backdrop(ui_composite_target);
        }
        self.submit_egui(ui_composite_target);
        let late_native_ui = !native_ui_in_main
            && (menu_backdrop_active || self.egui_active || self.crt_scope == ui::CrtScope::Scene);
        if late_native_ui {
            self.submit_ui_overlay(ui_composite_target);
        }
        if crt_everything {
            self.submit_crt(&frame_view);
        }
        if let Some(readback) = screenshot_readback
            .as_mut()
            .filter(|readback| !readback.inline_copy)
        {
            self.submit_screenshot_copy(&frame.texture, readback);
        }
        if let Some(started) = perf_followup_started {
            info.perf_followup_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        info.cpu_submit_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
        let present_started = Instant::now();
        self.window.pre_present_notify();
        frame.present();
        let present_completed_at = Instant::now();
        self.model_frame_log.presented(
            present_completed_at,
            view_proj,
            unjittered_view_proj,
            camera.position,
            [self.config.width, self.config.height],
        );
        info.cpu_present_ms = present_completed_at
            .saturating_duration_since(present_started)
            .as_secs_f64()
            * 1000.0;
        info.present_call_completed_at = Some(present_completed_at);
        if let Some(readback) = screenshot_readback {
            self.finish_screenshot_readback(readback);
        }
        // Start readback mapping only after the swapchain image has been
        // presented. This keeps diagnostic readback bookkeeping out of the
        // acquire/submit/present critical sequence.
        self.gpu_profiler.submit();
        if self.cull_debug_mode != CullDebugMode::Off {
            self.cull_diagnostics_readback.submit();
        }
        self.ocean_visibility.submit();

        if let Some(scope) = frame_validation_scope {
            // Validation is normally generated synchronously during command
            // recording/finish/submit. Poll once so any queued device-side
            // validation is also surfaced before the scope is popped.
            let _ = self.device.poll(wgpu::PollType::Poll);
            match block_on(scope.pop()) {
                Some(error) => {
                    eprintln!("[JKA FRAME DIAG] VALIDATION ERROR: {error}");
                    eprintln!("[JKA FRAME DIAG] debug: {error:?}");
                }
                None => {
                    eprintln!(
                        "[JKA FRAME DIAG] first diagnostic frame produced no validation error"
                    );
                }
            }
            self.frame_diag_pending = false;
        }

        info.frame_ms = frame_started.elapsed().as_secs_f64() * 1000.0;
        Ok(info)
    }
}

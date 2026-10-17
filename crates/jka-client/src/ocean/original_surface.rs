//! JKA host inputs for Blue Water's original ocean material and clipmap.
use super::{OCEAN_MAX_SURFACES, OCEAN_UNITS_PER_METER, OceanGpu, OceanPvsTileMask};
use blue_water_gpu::{
    foam_texture::FoamMap,
    original_surface::{
        OriginalClipmap, OriginalSurfaceBindings, OriginalSurfacePipeline, SurfaceAppearance,
        SurfaceBoundary, SurfaceCamera,
    },
    scene_copy::SceneCopy,
    underwater::{UnderwaterBindings, UnderwaterPipelines},
};
use bytemuck::Zeroable;
use glam::{Mat4, Vec3};

struct SurfaceSlot {
    bindings: OriginalSurfaceBindings,
    first: usize,
    outer: usize,
    ready: bool,
    horizon: bool,
    underwater: UnderwaterBindings,
    caustic_receiver: blue_water_gpu::caustics::Receiver,
    rain: Option<blue_water_gpu::rain_heights::SurfaceRead>,
    probe_due: bool,
    probe_active: bool,
    envelope: [f32; 4],
}

pub(super) struct SurfaceRenderer {
    key: (u32, u32, u32, wgpu::TextureFormat),
    reflection_steps: u32,
    pixel_detail_mode: u8,
    pipeline: OriginalSurfacePipeline,
    clipmap: OriginalClipmap,
    scene: SceneCopy,
    map_environment: Option<blue_water_gpu::map_environment::MapEnvironment>,
    sky_faces: Option<[wgpu::TextureView; 6]>,
    external_sky: Option<wgpu::TextureView>,
    foam: FoamMap,
    slots: [Option<SurfaceSlot>; OCEAN_MAX_SURFACES],
    underwater: UnderwaterPipelines,
    caustics: blue_water_gpu::caustics::Caustics,
    rain_heights: Option<blue_water_gpu::rain_heights::RainHeights>,
    underwater_slot: Option<usize>,
}

/// Translate the water plane to upstream y=0 and convert map units to metres.
/// This preserves exactly the host's projection, including reversed depth and
/// temporal jitter, while keeping the upstream wave and optical scales intact.
fn surface_camera(
    camera: &crate::camera::Camera,
    plane: f32,
    width: u32,
    height: u32,
    jitter: [f32; 2],
    time: f32,
    dt: f32,
    frame: u32,
) -> SurfaceCamera {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let aspect = width / height;
    let fov = camera.fov_y_for_viewport(width as u32, height as u32);
    let position = (camera.position - Vec3::Y * plane) / OCEAN_UNITS_PER_METER;
    let view = Mat4::look_to_rh(position, camera.forward(), Vec3::Y);
    let near = camera.near_distance() / OCEAN_UNITS_PER_METER;
    let far = camera.far_distance() / OCEAN_UNITS_PER_METER;
    let projection = Mat4::from_translation(Vec3::new(
        2.0 * jitter[0] / width,
        -2.0 * jitter[1] / height,
        0.0,
    )) * Mat4::perspective_rh(fov, aspect, far, near);
    let view_projection = projection * view;
    SurfaceCamera {
        view: view.to_cols_array_2d(),
        projection: projection.to_cols_array_2d(),
        view_projection: view_projection.to_cols_array_2d(),
        inverse_view: view.inverse().to_cols_array_2d(),
        inverse_projection: projection.inverse().to_cols_array_2d(),
        inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
        position_time: [position.x, position.y, position.z, time],
        viewport: [width, height, width.recip(), height.recip()],
        near,
        far,
        tan_half_fov_y: (fov * 0.5).tan(),
        aspect,
        dt,
        frame,
        curvature: 0.0,
        padding: 0,
    }
}

impl OceanGpu {
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_original_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera: &crate::camera::Camera,
        key: (u32, u32, u32, wgpu::TextureFormat),
        jitter: [f32; 2],
        dt: f32,
        frame: u32,
        show_wireframe: bool,
        show_waterline_marker: bool,
        pvs: &[Option<OceanPvsTileMask>; OCEAN_MAX_SURFACES],
        sky_faces: Option<[&wgpu::TextureView; 6]>,
        water_distance: &wgpu::TextureView,
        camera_water_surface: f32,
        sky_environment: Option<&::blue_water_sky::EnvironmentMap>,
        sky_light: Option<::blue_water_sky::CelestialLight>,
        aerial: Option<&::blue_water_render::AerialVolume>,
        cloud: Option<&::blue_water_clouds::CloudShadow>,
        volumes: &super::optics::Uniform,
        godot_receiver_color: &wgpu::TextureView,
        legacy_fog: Option<([f32; 4], [f32; 4])>,
        rain: f32,
    ) {
        let Some(blue) = self.blue_water.as_mut() else {
            return;
        };
        if self.promoted_surfaces.is_empty() {
            return;
        }
        if blue
            .original_surface
            .as_ref()
            .is_none_or(|renderer| renderer.key != key
                || renderer.reflection_steps != self.settings.blue_water_ssr_steps
                || renderer.pixel_detail_mode != self.settings.pixel_detail_mode)
        {
            let pipeline = OriginalSurfacePipeline::new_with_pixel_detail(
                device, &blue.surface_waves, key.3, key.2,
                self.settings.blue_water_ssr_steps, self.settings.pixel_detail_mode,
            );
            blue.original_surface = Some(SurfaceRenderer {
                reflection_steps: self.settings.blue_water_ssr_steps,
                pixel_detail_mode: self.settings.pixel_detail_mode,
                underwater: UnderwaterPipelines::new(
                    device,
                    blue.surface_waves.layout(),
                    key.3,
                    key.2,
                ),
                underwater_slot: None,
                rain_heights: None,
                caustics: blue_water_gpu::caustics::Caustics::new(
                    device,
                    blue.surface_waves.layout(),
                    key.3,
                    key.2,
                ),
                key,
                pipeline,
                clipmap: OriginalClipmap::new(device, self.settings.blue_water_mesh_segments, self.settings.blue_water_mesh_levels as usize),
                scene: SceneCopy::new(device, key.0, key.1, key.2),
                map_environment: None,
                sky_faces: None,
                external_sky: None,
                foam: FoamMap::new(device, queue),
                slots: std::array::from_fn(|_| None),
            });
        }
        let renderer = blue
            .original_surface
            .as_mut()
            .expect("Blue Water surface renderer");
        let surface_color = if matches!(self.settings.blue_water_caustics_mode, 3 | 4) {
            godot_receiver_color
        } else { &renderer.scene.color };
        if show_wireframe {
            renderer.pipeline.ensure_wireframe(device, key.3, key.2);
        }
        if show_waterline_marker {
            renderer.pipeline.ensure_waterline_marker(device, key.3, key.2);
        }
        let current_mesh = renderer.clipmap.clipmap();
        if current_mesh.segments != self.settings.blue_water_mesh_segments
            || current_mesh.levels != self.settings.blue_water_mesh_levels as usize
        {
            renderer.clipmap = OriginalClipmap::new(
                device, self.settings.blue_water_mesh_segments,
                self.settings.blue_water_mesh_levels as usize,
            );
        }
        let next_external_sky = sky_environment.map(|sky| sky.view.clone());
        let external_sky_changed = renderer.external_sky != next_external_sky;
        renderer.external_sky = next_external_sky;
        let next_faces = sky_faces.map(|faces| faces.map(Clone::clone));
        let sky_changed = renderer.sky_faces != next_faces;
        if sky_changed {
            if let Some(faces) = sky_faces {
                let environment = renderer.map_environment.get_or_insert_with(|| {
                    blue_water_gpu::map_environment::MapEnvironment::new(device)
                });
                environment.bake(device, encoder, faces);
            }
            renderer.sky_faces = next_faces;
        }
        renderer.caustics.write(
            queue,
            [
                camera.position.x / OCEAN_UNITS_PER_METER,
                camera.position.z / OCEAN_UNITS_PER_METER,
            ],
        );
        for slot in renderer.slots.iter_mut().flatten() {
            slot.ready = false;
        }
        renderer.underwater_slot = None;
        let mut underwater_score = f32::INFINITY;
        for promoted in &self.promoted_surfaces {
            let index = promoted.descriptor.index;
            if index >= OCEAN_MAX_SURFACES {
                continue;
            }
            let source = promoted.descriptor.surface;
            let extension = self
                .sky_extend_enabled
                .then_some(self.sky_extend_regions[index])
                .flatten();
            let centre_units = self.promoted_centres[index].unwrap_or_else(|| {
                let point = [camera.position.x, camera.position.z];
                self.masks.closest_point(index, point).unwrap_or([
                    point[0].clamp(source.minimum[0], source.maximum[0]),
                    point[1].clamp(source.minimum[1], source.maximum[1]),
                ])
            });
            let centre = centre_units.map(|v| f64::from(v / OCEAN_UNITS_PER_METER));
            let distance = super::promoted_footprint_distance(
                camera.position.to_array(),
                source,
                &self.masks,
                index,
            ) / OCEAN_UNITS_PER_METER;
            let standoff = if extension.is_some() {
                (camera.position.y - source.plane_height).abs() / OCEAN_UNITS_PER_METER
            } else {
                distance
            };
            let detail = renderer.clipmap.clipmap().detail(f64::from(standoff));
            let radius = if extension.is_some() {
                self.promoted_far_radii[index].unwrap_or(camera.far_distance())
            } else {
                (0..2)
                    .flat_map(|axis| {
                        [
                            source.minimum[axis] - centre_units[axis],
                            source.maximum[axis] - centre_units[axis],
                        ]
                    })
                    .map(f32::abs)
                    .fold(0.0, f32::max)
            } / OCEAN_UNITS_PER_METER;
            // Snapping displaces the outer frame by up to one cell.
            let outer = renderer
                .clipmap
                .outer_level_for_radius(f64::from(radius) * 1.05 + 1.0);
            let new_slot = renderer.slots[index].is_none();
            let slot = renderer.slots[index].get_or_insert_with(|| {
                let mut bindings = OriginalSurfaceBindings::with_foam(
                    device,
                    queue,
                    &renderer.pipeline,
                    renderer.foam.clone(),
                );
                bindings.set_scene(
                    device,
                    &renderer.pipeline,
                    surface_color,
                    &renderer.scene.depth,
                );
                let underwater = UnderwaterBindings::new(
                    device,
                    &renderer.underwater,
                    &bindings,
                    &renderer.scene.depth,
                    water_distance,
                );
                let caustic_receiver =
                    renderer
                        .caustics
                        .receiver(device, &bindings, &renderer.scene);
                SurfaceSlot {
                    rain: None,
                    bindings,
                    caustic_receiver,
                    underwater,
                    probe_due: true,
                    probe_active: false,
                    envelope: [0.0; 4],
                    first: 0,
                    outer: 0,
                    ready: false,
                    horizon: false,
                }
            });
            if sky_changed || new_slot || external_sky_changed {
                if let Some(sky) = sky_environment {
                    slot.bindings.set_sky_environment(
                        device,
                        &renderer.pipeline,
                        &sky.view,
                        &sky.sampler,
                        &sky.uniforms,
                    );
                } else if renderer.sky_faces.is_some() {
                    let map = &renderer
                        .map_environment
                        .as_ref()
                        .expect("map sky environment")
                        .map;
                    slot.bindings.set_environment_atlas(
                        device,
                        &renderer.pipeline,
                        &map.view,
                        &map.sampler,
                    );
                } else {
                    slot.bindings.use_ambient_atlas(device, &renderer.pipeline);
                }
            }
            slot.bindings.set_atmosphere_resources(
                device,
                &renderer.pipeline,
                aerial.map(|air| (&air.volume_view, &air.sampler)),
                cloud.map(|cloud| (&cloud.view, &cloud.sampler, &cloud.uniforms)),
                surface_color,
                &renderer.scene.depth,
            );
            slot.bindings.set_scene(device, &renderer.pipeline, surface_color, &renderer.scene.depth);
            let mut boundary = SurfaceBoundary::zeroed();
            boundary.counts[3] = u32::from(self.settings.blue_water_boundary_mode);
            boundary.bounds = [
                source.minimum[0],
                source.minimum[1],
                source.maximum[0],
                source.maximum[1],
            ];
            boundary.scale = [
                OCEAN_UNITS_PER_METER,
                0.75,
                self.settings.pixel_detail_mode.min(5) as f32,
                self.settings.pixel_detail_distance.max(1.0),
            ];
            let first = self.masks.ranges[index][0] as usize * 2;
            let count = self.masks.ranges[index][1] as usize;
            assert!(count <= 256 && first + count * 2 <= self.masks.triangles.len());
            boundary.counts[0] = count as u32;
            boundary.tiles = self.masks.tiles[index];
            boundary.triangles[..count * 2]
                .copy_from_slice(&self.masks.triangles[first..first + count * 2]);
            if self.settings.blue_water_boundary_mode == 3 {
                boundary.prepare_candidates();
            }
            if let Some(sky) = extension {
                boundary.counts[1] = sky.count as u32;
                boundary.sky = sky.segments;
            } else if let Some(visible) = pvs[index] {
                boundary.counts[2] = 1;
                for (target, values) in boundary.pvs.iter_mut().zip(visible.chunks_exact(4)) {
                    target.copy_from_slice(values);
                }
            }
            slot.bindings.write_boundary(queue, &boundary);
            let mut surface_view = surface_camera(
                camera,
                source.plane_height,
                key.0,
                key.1,
                jitter,
                self.elapsed,
                dt,
                frame,
            );
            slot.horizon = extension.is_some() && self.sky_infinite_enabled;
            if slot.horizon {
                // Original spherical-sea horizon/skirt, with the reference
                // atmosphere's 6360 km planet. Finite BSP pools remain flat.
                surface_view.curvature = 1.0 / (2.0 * 6_360_000.0);
            }
            slot.bindings.write_camera(queue, &surface_view);
            let foam = super::linear_color(promoted.settings.foam_color);
            let appearance = SurfaceAppearance {
                rain,
                crest_foam: foam,
                surface_foam: [foam[0], foam[1], foam[2], 0.6],
                foam_streak_mean: renderer.foam.streak_mean,
                ..self.settings.blue_water_optics.appearance()
            };
            let mut prefix =
                renderer
                    .clipmap
                    .surface_prefix(centre, f64::from(standoff), appearance);
            // The upstream material owns aerial perspective for the water;
            // the earlier opaque-air pass only sees submerged receivers.
            prefix[20][3] = f32::from(aerial.is_some());
            if let Some((color_depth, params)) = legacy_fog {
                prefix[19] = color_depth;
                prefix[19][3] /= OCEAN_UNITS_PER_METER;
                prefix[20][0] = params[0];
                prefix[20][1] = params[1];
            }
            let point = [camera.position.x, camera.position.z];
            let closest = self.masks.closest_point(index, point).unwrap_or([
                point[0].clamp(source.minimum[0], source.maximum[0]),
                point[1].clamp(source.minimum[1], source.maximum[1]),
            ]);
            let horizontal =
                ((closest[0] - point[0]).powi(2) + (closest[1] - point[1]).powi(2)).sqrt();
            // Include the near plane's reach across an outline. The probe
            // still checks each ray against the exact GPU admission mask.
            let near_reach = camera.near_distance()
                * (1.0 + surface_view.tan_half_fov_y * (1.0 + surface_view.aspect.powi(2)).sqrt());
            let crest_bound = (2.0 * blue.field.input_parameters().significant_height + 8.1) as f32;
            let near_surface = surface_view.position_time[1]
                <= crest_bound + near_reach / OCEAN_UNITS_PER_METER
                && (horizontal <= near_reach || extension.is_some_and(|sky| sky.contains(point)));
            slot.probe_due = new_slot || near_surface || slot.probe_active;
            slot.probe_active = near_surface;
            slot.envelope = [
                source.plane_height,
                crest_bound * OCEAN_UNITS_PER_METER,
                0.0,
                0.0,
            ];
            prefix[23][3] = if near_surface { 1.0 } else { 0.0 };
            // High tier's original 16-step reflection trace reads the same
            // opaque color and reversed depth as surface transmission.
            prefix[22] = [
                if self.settings.blue_water_ssr { 1.0 } else { 0.0 },
                400.0,
                self.settings.blue_water_refraction,
                self.settings.blue_water_refraction_mode as f32,
            ];
            // Native clipmaps already have crack-free fill/ring transitions.
            // The screen-space option advances the *first* complete ring only
            // when its cell projects below the requested pixel width at the
            // nearest possible water. This keeps all topology upstream-defined
            // and avoids allocating/rebuilding indices when the camera moves.
            // Distant outer rings are already geometrically coarser, so this
            // is deliberately a conservative first test rather than per-tile LOD.
            let mut first = detail.first;
            let screen_target = self.settings.screen_lod_pixels;
            if screen_target > 0.0 && standoff > 1.0 {
                let half_fov = (camera.fov_y_for_viewport(key.0, key.1) * 0.5).tan().max(0.001);
                let pixels_per_metre = key.1.max(1) as f64 / (2.0 * half_fov as f64 * f64::from(standoff));
                let mesh = renderer.clipmap.clipmap();
                while first + 1 < mesh.levels {
                    let cell_metres = mesh.half(first) * 2.0 / f64::from(mesh.segments);
                    if cell_metres * pixels_per_metre >= f64::from(screen_target) { break; }
                    first += 1;
                }
            }
            // The original vertex shader uses surface.detail.x to geomorph
            // its selected first ring. Match it to the actual submitted ring;
            // otherwise an LOD-adjusted draw can produce cracks or jumps.
            prefix[11][0] = first as f32;
            if first != detail.first { prefix[11][1] = 0.0; }
            slot.bindings.write_surface_prefix(queue, &prefix);
            let sun = self.environment.sun_direction_intensity;
            let tint = self.environment.sun_color;
            let (direction, color) = sky_light.map_or(
                ([-sun[0], -sun[1], -sun[2]], [tint[0], tint[1], tint[2]]),
                |light| (light.direction.to_array(), light.color.to_array()),
            );
            let intensity = sky_light.map_or(sun[3] * 5.0, |light| light.intensity);
            slot.caustic_receiver.write(
                queue,
                &blue_water_gpu::caustics::ReceiverParams {
                    sun: [
                        direction[0],
                        direction[1],
                        direction[2],
                        if self.settings.blue_water_caustics_mode >= 2 { 0.0 } else { self.settings.optics.caustics },
                    ],
                    light: [color[0], color[1], color[2], intensity],
                    plane: [
                        source.plane_height,
                        volumes.params[3],
                        OCEAN_UNITS_PER_METER,
                        1.0 - ((surface_view.position_time[1] - 12.0) / 13.0).clamp(0.0, 1.0),
                    ],
                    absorption: [
                        appearance.absorption[0],
                        appearance.absorption[1],
                        appearance.absorption[2],
                        0.0,
                    ],
                    minimum: volumes.minimum,
                    maximum: volumes.maximum,
                },
            );
            slot.underwater.write(
                queue,
                &surface_view,
                appearance,
                direction,
                color,
                near_surface,
            );
            let score = if (source.plane_height - camera_water_surface).abs() < 0.75 {
                0.0
            } else {
                (camera.position.y - source.plane_height).abs() + 1.0
            };
            if near_surface && score < underwater_score {
                underwater_score = score;
                renderer.underwater_slot = Some(index);
            }
            if sky_environment.is_none() {
                slot.bindings
                    .write_sun(queue, direction, color, sun[3] * 5.0);
            }
            let ambient = self.environment.sky_ambient;
            slot.bindings
                .write_ambient(queue, [ambient[0], ambient[1], ambient[2]]);
            slot.first = first;
            slot.outer = outer.max(first);
            slot.ready = true;
        }
    }

    pub fn original_surface_triangles(&self, index: usize) -> Option<u64> {
        let renderer = self.blue_water.as_ref()?.original_surface.as_ref()?;
        let slot = renderer.slots.get(index)?.as_ref()?;
        slot.ready.then(|| {
            if slot.horizon {
                renderer.clipmap.horizon_triangle_count(slot.first)
            } else {
                renderer.clipmap.triangle_count(slot.first, slot.outer)
            }
        })
    }

    pub fn apply_original_caustics(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        if let Some(renderer) = self
            .blue_water
            .as_ref()
            .and_then(|blue| blue.original_surface.as_ref())
        {
            let blue = self.blue_water.as_ref().expect("original ocean");
            if renderer
                .slots
                .iter()
                .flatten()
                .any(|slot| slot.ready && slot.caustic_receiver.enabled)
            {
                renderer.scene.capture(device, encoder, color, depth);
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Original caustics on JKA opaque receivers"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: color,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    for slot in renderer.slots.iter().flatten().filter(|slot| slot.ready) {
                        renderer.caustics.draw(
                            &mut pass,
                            &slot.caustic_receiver,
                            blue.surface_waves.group(&blue.field),
                        );
                    }
                }
            }
        }
    }

    /// Sample the same native FFT and admitted footprints used by Hybrid's sea.
    pub fn prepare_original_rain_heights(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        weather_eye: [f32; 3],
    ) -> Option<wgpu::TextureView> {
        let blue = self.blue_water.as_mut()?;
        let renderer = blue.original_surface.as_mut()?;
        let heights = renderer.rain_heights.get_or_insert_with(||
            blue_water_gpu::rain_heights::RainHeights::new(device, blue.surface_waves.layout()));
        heights.begin(queue, encoder, weather_eye);
        for slot in renderer.slots.iter_mut().flatten().filter(|slot| slot.ready) {
            let read = slot.rain.get_or_insert_with(|| heights.surface(device, &slot.bindings));
            heights.encode(encoder, read, blue.surface_waves.group(&blue.field));
        }
        Some(heights.view.clone())
    }

    /// Capture the lit scene after opaque caustics and aerial lighting.
    pub fn capture_original_scene(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        if let Some(renderer) = self
            .blue_water
            .as_ref()
            .and_then(|blue| blue.original_surface.as_ref())
        {
            renderer.scene.capture(device, encoder, color, depth);
        }
    }

    /// Caustics A/B: 0 original full-rate, 1 only freshly published FFT data,
    /// 2 off. Mode 1 follows wave steps and can lag the camera-centered texture.
    pub fn prepare_original_caustics(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.settings.blue_water_caustics_mode >= 2 { return; }
        if let Some(blue) = &self.blue_water {
            if let Some(renderer) = &blue.original_surface {
                if self.settings.blue_water_caustics_mode == 0 || blue.updated_this_frame {
                    if renderer.slots.iter().flatten().any(|slot| slot.ready && slot.caustic_receiver.enabled) {
                        renderer.caustics.encode(encoder, blue.surface_waves.group(&blue.field));
                    }
                }
            }
        }
    }

    /// Underwater ray probe, independent of caustic FFT gating.
    pub fn prepare_original_probe(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(blue) = &self.blue_water {
            if let Some(renderer) = &blue.original_surface {
                for slot in renderer
                    .slots
                    .iter()
                    .flatten()
                    .filter(|slot| slot.ready && slot.probe_due)
                {
                    slot.underwater.probe(
                        encoder,
                        &renderer.underwater,
                        blue.surface_waves.group(&blue.field),
                    );
                }
            }
        }
    }
    pub fn original_underwater_probe(&self) -> Option<&wgpu::TextureView> {
        let renderer = self.blue_water.as_ref()?.original_surface.as_ref()?;
        let slot = renderer
            .underwater_slot
            .and_then(|index| renderer.slots[index].as_ref())?;
        Some(slot.bindings.underwater_probe())
    }

    pub fn original_underwater_active(&self) -> bool {
        self.blue_water
            .as_ref()
            .and_then(|blue| blue.original_surface.as_ref())
            .is_some_and(|renderer| renderer.underwater_slot.is_some())
    }

    pub fn original_underwater_envelope(&self) -> [f32; 4] {
        self.blue_water
            .as_ref()
            .and_then(|blue| blue.original_surface.as_ref())
            .and_then(|renderer| {
                renderer
                    .underwater_slot
                    .and_then(|index| renderer.slots[index].as_ref())
            })
            .map_or([0.0; 4], |slot| slot.envelope)
    }

    pub fn draw_original_underwater(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
    ) {
        let Some(renderer) = self
            .blue_water
            .as_ref()
            .and_then(|blue| blue.original_surface.as_ref())
        else {
            return;
        };
        let Some(slot) = renderer
            .underwater_slot
            .and_then(|index| renderer.slots[index].as_ref())
        else {
            return;
        };
        // Refresh after water and transparency. No effect samples a live attachment.
        renderer.scene.capture(device, encoder, color, depth);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Blue Water original underwater effects"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        slot.underwater.draw(&mut pass, &renderer.underwater, self.settings.blue_water_underwater_mode != 1);
    }

    /// r_showtris OCEAN: draw the REAL Blue Water clipmap, not the host's
    /// Godot mesh. Uses the same admitted surfaces and selected first/outer LOD.
    pub fn draw_original_wireframe<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, admitted: u8) {
        let Some(blue) = self.blue_water.as_ref() else { return; };
        let Some(renderer) = blue.original_surface.as_ref() else { return; };
        let Some(pipeline) = renderer.pipeline.wireframe.as_ref() else { return; };
        for (index, slot) in renderer.slots.iter().enumerate() {
            let Some(slot) = slot.as_ref().filter(|slot|
                slot.ready && (admitted >> index & 1 != 0 || slot.horizon)) else { continue; };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &slot.bindings.camera_group, &[]);
            pass.set_bind_group(1, &slot.bindings.material_group, &[]);
            pass.set_bind_group(2, blue.surface_waves.group(&blue.field), &[]);
            pass.set_bind_group(3, &slot.bindings.environment_group, &[]);
            if slot.horizon {
                renderer.clipmap.draw_horizon(pass, slot.first);
            } else {
                renderer.clipmap.draw(pass, slot.first, slot.outer);
            }
        }
    }

    /// Mode 3: native GPU wave data sampled at CAMERA XZ and shown 100 units ahead,
    /// even if Hybrid currently selects the Godot submerged surface.
    pub fn draw_original_waterline_marker<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> u32 {
        let Some(blue) = &self.blue_water else { return 0; };
        let Some(renderer) = &blue.original_surface else { return 0; };
        let Some(pipeline) = &renderer.pipeline.waterline_marker else { return 0; };
        let mut count = 0;
        for slot in renderer.slots.iter().flatten().filter(|slot| slot.ready) {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &slot.bindings.camera_group, &[]);
            pass.set_bind_group(1, &slot.bindings.material_group, &[]);
            pass.set_bind_group(2, blue.surface_waves.group(&blue.field), &[]);
            pass.set_bind_group(3, &slot.bindings.environment_group, &[]);
            pass.draw(0..36, 0..1);
            count += 1;
        }
        count
    }

    pub fn draw_original_surfaces<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        admitted: u8,
    ) -> (u32, u32, u64) {
        let Some(blue) = self.blue_water.as_ref() else {
            return (0, 0, 0);
        };
        let Some(renderer) = blue.original_surface.as_ref() else {
            return (0, 0, 0);
        };
        let mut drawn = 0;
        let mut horizon_only = 0;
        let mut horizon_triangles = 0;
        for (index, slot) in renderer.slots.iter().enumerate() {
            // A qualified distant horizon remains visible when its nearby
            // finite mesh is outside the frustum. The sky/PVS gate was applied
            // when selecting this slot's sky contacts, before preparation.
            let Some(slot) = slot
                .as_ref()
                .filter(|slot| slot.ready && (admitted >> index & 1 != 0 || slot.horizon))
            else {
                continue;
            };
            if admitted >> index & 1 == 0 {
                horizon_only += 1;
                horizon_triangles += renderer.clipmap.horizon_triangle_count(slot.first);
            }
            pass.set_pipeline(&renderer.pipeline.pipeline);
            pass.set_bind_group(0, &slot.bindings.camera_group, &[]);
            pass.set_bind_group(1, &slot.bindings.material_group, &[]);
            pass.set_bind_group(2, blue.surface_waves.group(&blue.field), &[]);
            pass.set_bind_group(3, &slot.bindings.environment_group, &[]);
            if slot.horizon {
                renderer.clipmap.draw_horizon(pass, slot.first);
            } else {
                renderer.clipmap.draw(pass, slot.first, slot.outer);
            }
            drawn += 1;
        }
        (drawn, horizon_only, horizon_triangles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metre_camera_matches_jka_projection_at_each_water_plane() {
        let mut camera = crate::camera::Camera::new([1200.0, 900.0, -600.0], 0.7);
        camera.pitch = -0.2;
        let jitter = [0.375, -0.25];
        let host = camera.view_projection_jittered(1280, 720, jitter);
        for plane in [-1024.0, 0.0, 800.0] {
            let uniform = surface_camera(&camera, plane, 1280, 720, jitter, 0.0, 0.016, 3);
            let projection = Mat4::from_cols_array_2d(&uniform.view_projection);
            for point in [
                Vec3::new(900.0, plane, -1600.0),
                Vec3::new(2000.0, plane + 16.0, 0.0),
            ] {
                let a = host * point.extend(1.0);
                let b =
                    projection * ((point - Vec3::Y * plane) / OCEAN_UNITS_PER_METER).extend(1.0);
                let a = a.truncate() / a.w;
                let b = b.truncate() / b.w;
                assert!(
                    (a - b).abs().max_element() < 2e-5,
                    "plane {plane}: {a:?} != {b:?}"
                );
            }
        }
    }
}

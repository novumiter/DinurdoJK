//! The original atmosphere on JKA's device; BSP sky polygons authorize display.
use crate::{camera::Camera, scene::GpuVertex};
use ::blue_water_sky::{Sky, SkyQuality, SkyScene, SkyTargets, SkyView, math};

pub(in crate::renderer) struct AtmosphereRenderer {
    pub sky: Sky,
    pub clouds: ::blue_water_clouds::CloudLayer,
    pub aerial: ::blue_water_render::AerialVolume,
    air_composite: super::blue_water_air::AirComposite,
    aerial_pass: ::blue_water_clouds::aerial::AerialPerspective,
    frame_camera: Option<::blue_water_render::CameraUniforms>,
    last_view: Option<(math::Vec3, math::Vec3, f32)>,
    last_scene: Option<SkyScene>,
    probe: Option<wgpu::TextureView>,
    show_atmosphere: bool,
    clouds_active: bool,
    key: (u32, u32, u32, wgpu::TextureFormat),
    image: wgpu::TextureView,
    camera: wgpu::Buffer,
    portal_camera: wgpu::Buffer,
    portal_group: wgpu::BindGroup,
    group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}
impl AtmosphereRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: (u32, u32, u32, wgpu::TextureFormat),
    ) -> Self {
        let mut sky = Sky::new(
            device,
            queue,
            SkyQuality::High,
            SkyTargets {
                color: wgpu::TextureFormat::Rgba16Float,
                depth: None,
            },
        );
        let clouds = ::blue_water_clouds::CloudLayer::new(
            device,
            queue,
            &sky,
            SkyQuality::High,
            ::blue_water_clouds::Targets {
                color: wgpu::TextureFormat::Rgba16Float,
                depth: None,
            },
        );
        sky.attach_clouds(device, clouds.hooks());
        let aerial = ::blue_water_render::AerialVolume::new(device);
        let air_composite = super::blue_water_air::AirComposite::new(device, key, &aerial);
        let aerial_pass = ::blue_water_clouds::aerial::AerialPerspective::new(
            device,
            &sky,
            clouds.shadow(),
            ::blue_water_render::MakePipelines::Now,
        );
        let (image, camera, group, pipeline) = Self::targets(device, key);
        let (portal_camera, portal_group) = Self::portal_binding(device, &image, &pipeline);
        Self {
            sky,
            clouds,
            aerial,
            air_composite,
            aerial_pass,
            frame_camera: None,
            last_view: None,
            last_scene: None,
            probe: None,
            show_atmosphere: true,
            clouds_active: false,
            key,
            image,
            camera,
            portal_camera,
            portal_group,
            group,
            pipeline,
        }
    }
    fn portal_binding(
        device: &wgpu::Device,
        image: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blue Water sky portal camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water sky portal display"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(image),
                },
            ],
        });
        (camera, group)
    }
    pub fn write_portal_camera(&self, queue: &wgpu::Queue, matrix: glam::Mat4) {
        queue.write_buffer(
            &self.portal_camera,
            0,
            bytemuck::bytes_of(&matrix.to_cols_array_2d()),
        );
    }
    pub fn bind_portal<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.portal_group, &[]);
    }
    fn targets(
        device: &wgpu::Device,
        key: (u32, u32, u32, wgpu::TextureFormat),
    ) -> (
        wgpu::TextureView,
        wgpu::Buffer,
        wgpu::BindGroup,
        wgpu::RenderPipeline,
    ) {
        let image = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Blue Water atmosphere screen"),
                size: wgpu::Extent3d {
                    width: key.0.max(1),
                    height: key.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blue Water BSP sky camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blue Water BSP sky display"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water BSP sky display"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&image),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blue Water BSP sky display"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blue Water BSP sky display"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blue_water_sky.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blue Water BSP sky display"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("sky_mask"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("sky_color"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: key.3,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: key.2,
                ..Default::default()
            },
            multiview_mask: None,
            cache: crate::pipeline_cache::get().as_ref(),
        });
        (image, camera, group, pipeline)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        key: (u32, u32, u32, wgpu::TextureFormat),
        camera: &Camera,
        jitter: [f32; 2],
        time: f32,
        toward_sun: [f32; 3],
        sea_plane: f32,
        show_atmosphere: bool,
        clouds_enabled: bool,
        coverage: f32,
        wind: [f32; 2],
    ) {
        if self.key != key {
            let (image, camera, group, pipeline) = Self::targets(device, key);
            (self.portal_camera, self.portal_group) =
                Self::portal_binding(device, &image, &pipeline);
            self.air_composite =
                super::blue_water_air::AirComposite::new(device, key, &self.aerial);
            self.image = image;
            self.camera = camera;
            self.group = group;
            self.pipeline = pipeline;
            self.key = key;
        }
        let sun = math::Vec3::from_array(toward_sun).normalize_or_zero();
        let elevation = f64::from(sun.y.clamp(-1.0, 1.0).asin().to_degrees());
        let azimuth = f64::from(sun.x.atan2(sun.z).to_degrees());
        self.show_atmosphere = show_atmosphere;
        self.clouds_active = clouds_enabled;
        let mut scene = SkyScene::north_atlantic(elevation, azimuth);
        if clouds_enabled {
            scene.clouds.coverage = f64::from(coverage);
            scene.clouds.wind_speed = f64::from(wind[0] / crate::ocean::OCEAN_UNITS_PER_METER);
            scene.clouds.wind_heading = f64::from(wind[1]);
        } else {
            scene = scene.cloudless();
        }
        if self.last_scene != Some(scene) {
            self.sky.apply(&scene);
            self.clouds.apply(queue, &mut self.sky, &scene);
            self.last_scene = Some(scene);
        }
        self.clouds.set_enabled(clouds_enabled);
        self.sky.seek(f64::from(time));
        let position =
            (camera.position - glam::Vec3::Y * sea_plane) / crate::ocean::OCEAN_UNITS_PER_METER;
        let view = glam::Mat4::look_to_rh(position, camera.forward(), glam::Vec3::Y);
        let fov = camera.fov_y_for_viewport(key.0, key.1);
        let projection = glam::Mat4::from_translation(glam::Vec3::new(
            2.0 * jitter[0] / key.0.max(1) as f32,
            -2.0 * jitter[1] / key.1.max(1) as f32,
            0.0,
        )) * glam::Mat4::perspective_rh(
            fov,
            key.0 as f32 / key.1.max(1) as f32,
            camera.far_distance() / crate::ocean::OCEAN_UNITS_PER_METER,
            camera.near_distance() / crate::ocean::OCEAN_UNITS_PER_METER,
        );
        let sky_view = SkyView {
            position: math::Vec3::from_array(position.to_array()),
            camera_to_world: math::Mat4::from_cols_array_2d(&view.inverse().to_cols_array_2d()),
            projection: math::Mat4::from_cols_array_2d(&projection.to_cols_array_2d()),
            viewport: [key.0, key.1],
        };
        self.sky.update_tables(queue, encoder, &sky_view, 0.0);
        // Cloud temporal reconstruction owns its subpixel pattern, so its view is unjittered.
        let cloud_projection = math::Mat4::from_cols_array_2d(
            &glam::Mat4::perspective_rh(
                fov,
                key.0 as f32 / key.1.max(1) as f32,
                camera.far_distance() / crate::ocean::OCEAN_UNITS_PER_METER,
                camera.near_distance() / crate::ocean::OCEAN_UNITS_PER_METER,
            )
            .to_cols_array_2d(),
        );
        let cloud_view = ::blue_water_clouds::CloudView {
            position: sky_view.position,
            view: sky_view.camera_to_world.inverse(),
            projection: cloud_projection,
            near: camera.near_distance() / crate::ocean::OCEAN_UNITS_PER_METER,
            far: camera.far_distance() / crate::ocean::OCEAN_UNITS_PER_METER,
            viewport: sky_view.viewport,
        };
        let forward = math::Vec3::from_array(camera.forward().to_array());
        let cut = self.last_view.is_none_or(|(position, direction, focal)| {
            position.distance(cloud_view.position) > 30.0
                || direction.dot(forward) < 0.94
                || (fov / focal - 1.0).abs() > 0.2
        });
        self.last_view = Some((cloud_view.position, forward, fov));
        self.clouds.prepare(
            device,
            &mut ::blue_water_render::UploadSink::Queue(queue),
            &cloud_view,
            &self.sky,
            cut,
        );
        if let Some(group) = self.clouds.take_field_group() {
            self.sky.set_cloud_bind_group(group);
        }
        self.sky.update_environment(queue, encoder);
        self.clouds.encode_shadows(encoder);
        let vp = sky_view.projection * cloud_view.view;
        let camera_uniforms = ::blue_water_render::CameraUniforms {
            view: cloud_view.view.to_cols_array(),
            projection: sky_view.projection.to_cols_array(),
            view_projection: vp.to_cols_array(),
            inverse_view: sky_view.camera_to_world.to_cols_array(),
            inverse_projection: sky_view.projection.inverse().to_cols_array(),
            inverse_view_projection: vp.inverse().to_cols_array(),
            position: sky_view.position.to_array(),
            time,
            viewport: [
                key.0 as f32,
                key.1 as f32,
                1.0 / key.0.max(1) as f32,
                1.0 / key.1.max(1) as f32,
            ],
            near: cloud_view.near,
            far: cloud_view.far,
            tan_half_fov_y: (fov * 0.5).tan(),
            aspect: key.0 as f32 / key.1.max(1) as f32,
            dt: 0.0,
            frame: 0,
            curvature: 0.0,
            padding: 0,
        };
        self.aerial_pass.enabled = show_atmosphere;
        self.aerial_pass.encode(
            device,
            &mut ::blue_water_render::UploadSink::Queue(queue),
            encoder,
            &camera_uniforms,
            &self.aerial,
            &self.sky,
            self.clouds.shadow(),
            [
                self.clouds.uniforms().heights[0],
                self.clouds.uniforms().heights[1],
            ],
            None,
        );
        self.frame_camera = Some(camera_uniforms);
        queue.write_buffer(
            &self.camera,
            0,
            bytemuck::bytes_of(
                &camera
                    .view_projection_jittered(key.0, key.1, jitter)
                    .to_cols_array_2d(),
            ),
        );
    }
    /// The probe is encoded after the ocean's current camera/boundary writes and before this march.
    pub fn render_image(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        probe: Option<&wgpu::TextureView>,
    ) {
        let next_probe = probe.cloned();
        if self.probe != next_probe {
            self.clouds.set_underwater_probe(next_probe.clone());
            self.probe = next_probe;
        }
        if self.clouds_active {
            self.clouds.encode_march(device, encoder, &self.sky, None);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Blue Water original atmosphere dome"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.image,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if self.show_atmosphere {
            self.sky.draw_dome(&mut pass);
        }
        if self.clouds_active {
            self.clouds.draw_composite(&mut pass);
        }
    }
    pub fn apply_opaque_air(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
    ) {
        if self.show_atmosphere {
            if let Some(camera) = &self.frame_camera {
                self.air_composite
                    .draw(device, queue, encoder, camera, color, resolve, depth);
            }
        }
    }
    pub fn bind<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
    }
}

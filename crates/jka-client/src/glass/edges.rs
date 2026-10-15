//! Geometry coverage and nearest-boundary coordinates, shared by all glass paths.
//! The private depth target contains glass only: foreground actors cannot invent
//! pane boundaries. Normal discontinuities also seed bevels on faceted models.
use wgpu::util::DeviceExt;

pub struct Resources {
    pub mask: wgpu::TextureView,
    _mask_texture: wgpu::Texture,
    pub mask_msaa: Option<wgpu::TextureView>,
    pub depth: wgpu::TextureView,
    pub field: wgpu::TextureView,
    textures: [wgpu::Texture; 2],
    seed: wgpu::ComputePipeline,
    jump: wgpu::ComputePipeline,
    seed_group: wgpu::BindGroup,
    steps: Vec<(u32, [wgpu::BindGroup; 2])>,
    size: (u32, u32),
}

impl Resources {
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        samples: u32,
        scene_depth: &wgpu::TextureView,
    ) -> Self {
        let texture = |width, height, format, samples, usage, label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let mask_usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST;
        let mask_texture = texture(
            width,
            height,
            wgpu::TextureFormat::Rgba16Float,
            1,
            mask_usage,
            "Glass geometry coverage",
        );
        let mask = mask_texture.create_view(&Default::default());
        let mask_msaa = (samples > 1).then(|| {
            texture(
                width,
                height,
                wgpu::TextureFormat::Rgba16Float,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                "Glass geometry coverage MSAA",
            )
            .create_view(&Default::default())
        });
        let depth = texture(
            width,
            height,
            wgpu::TextureFormat::Depth32Float,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            "Glass-only depth",
        )
        .create_view(&Default::default());
        let size = (width.div_ceil(2), height.div_ceil(2));
        let usage = wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        let textures = std::array::from_fn(|_| {
            texture(
                size.0,
                size.1,
                wgpu::TextureFormat::Rgba16Float,
                1,
                usage,
                "Glass nearest boundary",
            )
        });
        let views: [wgpu::TextureView; 2] =
            std::array::from_fn(|i| textures[i].create_view(&Default::default()));
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let read = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Glass edge field layout"),
            entries: &[
                entry(0, read),
                entry(1, read),
                entry(4, read),
                entry(
                    2,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(
                    3,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Glass silhouette distance"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../glass_edges.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Glass edge compute layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: crate::pipeline_cache::get().as_ref(),
            })
        };
        let seed = pipeline("seed_edges");
        let jump = pipeline("jump_edges");
        let bind = |input: usize, output: usize, uniform: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Glass edge compute inputs"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&mask),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(scene_depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[input]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&views[output]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            })
        };
        let mut steps = Vec::new();
        let mut seed_group = None;
        let mut step = 1;
        while step <= size.0.max(size.1).next_power_of_two() {
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Glass edge propagation step"),
                contents: bytemuck::cast_slice(&[step, width, height, 0u32]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            if step == 1 {
                seed_group = Some(bind(1, 0, &uniform));
            }
            steps.push((step, [bind(0, 1, &uniform), bind(1, 0, &uniform)]));
            step *= 2;
        }
        Self {
            mask,
            _mask_texture: mask_texture,
            mask_msaa,
            depth,
            field: views[0].clone(),
            textures,
            seed,
            jump,
            seed_group: seed_group.unwrap(),
            steps,
            size,
        }
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder, radius_pixels: u32) {
        let grid = (self.size.0.div_ceil(8), self.size.1.div_ceil(8));
        let mut current = 0usize;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Glass pane boundaries"),
                ..Default::default()
            });
            pass.set_pipeline(&self.seed);
            pass.set_bind_group(0, &self.seed_group, &[]);
            pass.dispatch_workgroups(grid.0, grid.1, 1);
            pass.set_pipeline(&self.jump);
            let limit = radius_pixels.div_ceil(2).max(1).next_power_of_two();
            for (step, groups) in self.steps.iter().rev().filter(|(step, _)| *step <= limit) {
                let _ = step;
                pass.set_bind_group(0, &groups[current], &[]);
                pass.dispatch_workgroups(grid.0, grid.1, 1);
                current = 1 - current;
            }
            // JFA+1 improves the nearest-coordinate selection at corners.
            pass.set_bind_group(0, &self.steps[0].1[current], &[]);
            pass.dispatch_workgroups(grid.0, grid.1, 1);
            current = 1 - current;
        }
        if current != 0 {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.textures[current],
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.textures[0],
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: self.size.0,
                    height: self.size.1,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

#[cfg(test)]
#[path = "edge_tests.rs"]
mod tests;

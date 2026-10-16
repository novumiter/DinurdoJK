//! JKA map sky lighting through Blue Water's original CubeUV layout and
//! preserved Gaussian prefilter. Atmosphere and cloud backends are separate.
use crate::sky_environment::{self, CubeUvLayout, EnvironmentMap};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use naga_oil::compose::{Composer, NagaModuleDescriptor, ShaderType};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Blur {
    weights: [[f32; 4]; 5], pole_axis: [f32; 3], d_theta: f32,
    texel_size: [f32; 2], max_mip: f32, mip_int: f32,
    samples: u32, latitudinal: u32, out_size: u32, padding: u32,
    out_origin: [u32; 2], padding_tail: [u32; 2],
}

// Host uniform packing from upstream sky/src/sky.rs::blur_chain/half_blur.
// Its preserved shader performs every sample of the original Gaussian chain.
fn blur_chain(layout: &CubeUvLayout) -> Vec<Blur> {
    let phi = (1.0 + 5f32.sqrt()) / 2.0;
    let inv = phi.recip();
    let poles = [Vec3::new(-phi, inv, 0.0), Vec3::new(phi, inv, 0.0),
        Vec3::new(-inv, 0.0, phi), Vec3::new(inv, 0.0, phi),
        Vec3::new(0.0, phi, -inv), Vec3::new(0.0, phi, inv),
        Vec3::new(-1.0, 1.0, -1.0), Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, 1.0), Vec3::new(1.0, 1.0, 1.0)]
        .map(|pole| (pole * Vec3::new(1.0, -1.0, 1.0)).normalize());
    let levels = layout.levels();
    let mut chain = Vec::new();
    for i in 1..levels.len() {
        let sigma = (levels[i].sigma.powi(2) - levels[i - 1].sigma.powi(2)).sqrt();
        for latitudinal in [true, false] {
            let input = if latitudinal { i - 1 } else { i };
            let d_theta = std::f32::consts::PI / (2.0 * (levels[input].size - 1) as f32);
            let sigma_pixels = sigma / d_theta;
            let samples = (1 + (3.0 * sigma_pixels).floor() as usize).min(20);
            let mut weights = [0f32; 20];
            let mut sum = 0.0;
            for (sample, weight) in weights.iter_mut().enumerate() {
                let x = sample as f32 / sigma_pixels;
                *weight = (-x * x / 2.0).exp();
                if sample == 0 { sum += *weight; }
                else if sample < samples { sum += 2.0 * *weight; }
            }
            let mut packed = [[0.0; 4]; 5];
            for (sample, weight) in weights.iter().enumerate() { packed[sample / 4][sample % 4] = weight / sum; }
            chain.push(Blur {
                weights: packed, pole_axis: poles[(levels.len() - i - 1) % poles.len()].to_array(), d_theta,
                texel_size: [1.0 / layout.width as f32, 1.0 / layout.height as f32],
                max_mip: layout.max_mip as f32, mip_int: layout.max_mip as f32 - input as f32,
                samples: samples as u32, latitudinal: u32::from(latitudinal),
                out_size: levels[i].size, padding: 0, out_origin: [levels[i].x, levels[i].y], padding_tail: [0; 2],
            });
        }
    }
    chain
}

pub struct MapEnvironment {
    pub map: EnvironmentMap,
    bake: wgpu::ComputePipeline,
    blur: wgpu::ComputePipeline,
    bake_output: wgpu::BindGroup,
    blur_steps: Vec<(wgpu::BindGroup, wgpu::BindGroup, u32)>,
}

impl MapEnvironment {
    pub fn new(device: &wgpu::Device) -> Self {
        let map = EnvironmentMap::new(device, CubeUvLayout::from_max_mip(7));
        let mut composer = Composer::default();
        sky_environment::add_environment_module(&mut composer).expect("original environment module");
        let module = |composer: &mut Composer, source, file_path| {
            let module = composer.make_naga_module(NagaModuleDescriptor {
                source, file_path, shader_type: ShaderType::Wgsl, ..Default::default()
            }).unwrap_or_else(|error| panic!("{}", error.emit_to_string(composer)));
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(file_path), source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
            })
        };
        let bake_shader = module(&mut composer, include_str!("map_environment.wgsl"), "JKA map environment bake");
        let blur_shader = module(&mut composer,
            include_str!("../../../vendor/blue-water-sea-and-sky/sky/shaders/prefilter.wgsl"), "original sky prefilter");
        let pipeline = |shader: &wgpu::ShaderModule, entry| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: None, module: shader, entry_point: Some(entry),
            compilation_options: Default::default(), cache: None,
        });
        let bake = pipeline(&bake_shader, "bake");
        let blur = pipeline(&blur_shader, "blur_main");
        let ping = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Blue Water original sky prefilter scratch"),
            size: wgpu::Extent3d { width: map.layout.width, height: map.layout.height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: sky_environment::ENVIRONMENT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        }).create_view(&Default::default());
        let output_group = |layout: &wgpu::BindGroupLayout, view| device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blue Water environment output"), layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }],
        });
        let bake_output = output_group(&bake.get_bind_group_layout(1), &map.view);
        let mut blur_steps = Vec::new();
        for params in blur_chain(&map.layout) {
            let latitudinal = params.latitudinal != 0;
            let (source, destination) = if latitudinal { (&map.view, &ping) } else { (&ping, &map.view) };
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Blue Water original prefilter parameters"), contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let input = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Blue Water original prefilter input"), layout: &blur.get_bind_group_layout(0), entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&map.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                ],
            });
            blur_steps.push((input, output_group(&blur.get_bind_group_layout(1), destination), params.out_size));
        }
        Self { map, bake, blur, bake_output, blur_steps }
    }

    pub fn bake(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder,
        faces: [&wgpu::TextureView; 6]) {
        let mut entries: Vec<_> = faces.iter().enumerate().map(|(binding, face)| wgpu::BindGroupEntry {
            binding: binding as u32, resource: wgpu::BindingResource::TextureView(face),
        }).collect();
        entries.push(wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.map.sampler) });
        let input = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JKA skybox environment input"), layout: &self.bake.get_bind_group_layout(0), entries: &entries,
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JKA skybox to original CubeUV atlas"), timestamp_writes: None,
            });
            pass.set_pipeline(&self.bake);
            pass.set_bind_group(0, &input, &[]);
            pass.set_bind_group(1, &self.bake_output, &[]);
            pass.dispatch_workgroups(384u32.div_ceil(8), 256u32.div_ceil(8), 1);
        }
        for (input, output, size) in &self.blur_steps {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Original Blue Water sky prefilter"), timestamp_writes: None,
            });
            pass.set_pipeline(&self.blur);
            pass.set_bind_group(0, input, &[]);
            pass.set_bind_group(1, output, &[]);
            pass.dispatch_workgroups((3 * size).div_ceil(8), (2 * size).div_ceil(8), 1);
        }
        self.map.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_sky_faces_keep_orientation_and_original_prefilter_stays_finite() {
        futures_lite::future::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance.request_adapter(&Default::default()).await.expect("GPU adapter");
            let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: adapter.limits(), ..Default::default()
            }).await.expect("GPU device");
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255],
                [255, 255, 0, 255], [255, 0, 255, 255], [0, 255, 255, 255]];
            let faces = colors.map(|color| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("JKA sky face orientation fixture"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
                });
                queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0,
                    origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, &color,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
                    wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
                texture.create_view(&Default::default())
            });
            let mut environment = MapEnvironment::new(&device);
            let bytes = 384 * 512 * 8;
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("JKA sky atlas readback"), size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            environment.bake(&device, &mut encoder, std::array::from_fn(|i| &faces[i]));
            encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {
                texture: &environment.map.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            }, wgpu::TexelCopyBufferInfo { buffer: &read, layout: wgpu::TexelCopyBufferLayout {
                offset: 0, bytes_per_row: Some(384 * 8), rows_per_image: Some(512),
            } }, wgpu::Extent3d { width: 384, height: 512, depth_or_array_layers: 1 });
            queue.submit(Some(encoder.finish()));
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            assert!(scope.pop().await.is_none(), "original map sky prefilter validation");
            let mapped = read.slice(..).get_mapped_range().unwrap();
            let data: &[u16] = bytemuck::cast_slice(&mapped);
            // CubeUV +X,+Y,+Z,-X,-Y,-Z maps to JKA rt,up,ft,lf,dn,bk.
            for (face, expected) in [0, 4, 3, 2, 5, 1].into_iter().enumerate() {
                let x = face % 3 * 128 + 64;
                let y = face / 3 * 128 + 64;
                let pixel = &data[(y * 384 + x) * 4..][..3];
                for channel in 0..3 {
                    assert_eq!(pixel[channel], if colors[expected][channel] == 0 { 0 } else { 0x3c00 },
                        "face {face} channel {channel}");
                }
            }
            for level in environment.map.layout.levels() {
                let x = level.x as usize + level.size as usize / 2;
                let y = level.y as usize + level.size as usize / 2;
                let pixel = &data[(y * 384 + x) * 4..][..3];
                assert!(pixel.iter().all(|v| v & 0x7c00 != 0x7c00), "non-finite blurred level {level:?}");
                assert!(pixel.iter().any(|&v| v != 0), "black blurred level {level:?}");
            }
        });
    }
}

//! GPU upload of the preserved Blue Water foam texture. The texels and mip
//! generation are compiled directly from upstream in `blue-water-core`.

use blue_water_core::foam_texture::{FOAM_ANISOTROPY, FoamTexture};

#[derive(Clone)]
pub struct FoamMap {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub streak_mean: f32,
}

impl FoamMap {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let generated = FoamTexture::generate();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Blue Water original ocean foam"),
            size: wgpu::Extent3d {
                width: generated.levels[0].0,
                height: generated.levels[0].0,
                depth_or_array_layers: 1,
            },
            mip_level_count: generated.levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (size, pixels)) in generated.levels.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(*size),
                },
                wgpu::Extent3d {
                    width: *size,
                    height: *size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blue Water original ocean foam"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: FOAM_ANISOTROPY,
            ..Default::default()
        });
        Self {
            texture,
            view,
            sampler,
            streak_mean: generated.streak_mean,
        }
    }
}

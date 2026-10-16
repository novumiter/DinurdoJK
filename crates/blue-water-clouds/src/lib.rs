//! Preserved Blue Water volumetric clouds on the JKA device.
pub use glam as math;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/aerial.rs"]
pub mod aerial;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/cascades.rs"]
pub mod cascades;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/detail_noise.rs"]
pub mod detail_noise;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/gpu.rs"]
pub mod gpu;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/key_light.rs"]
pub mod key_light;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/layer.rs"]
pub mod layer;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/layers.rs"]
pub mod layers;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/quality.rs"]
pub mod quality;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/shaders.rs"]
pub mod shaders;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/textures.rs"]
pub mod textures;
#[path = "../../../vendor/blue-water-sea-and-sky/clouds/src/weathers.rs"]
pub mod weathers;
pub use layer::{CloudLayer, CloudShadow, CloudView};
#[derive(Clone, Copy, Debug)]
pub struct Targets {
    pub color: wgpu::TextureFormat,
    pub depth: Option<wgpu::TextureFormat>,
}
impl Default for Targets {
    fn default() -> Self {
        Self {
            color: naval_render::HDR_FORMAT,
            depth: Some(naval_render::DEPTH_FORMAT),
        }
    }
}
pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

//! Blue Water ocean CPU algorithms, compiled directly from the preserved upstream source.
//! See `vendor/blue-water-sea-and-sky/LICENSE` for the upstream MIT notice.
//!
//! The small types below reproduce the upstream ocean facade's quality and cascade
//! contracts. GPU resources and renderer-specific types remain in the client adapter.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeInfo {
    pub size: f64,
    pub resolution: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QualityTier {
    Low,
    Medium,
    High,
    Ultra,
}

impl QualityTier {
    pub fn cascades(self) -> Vec<CascadeInfo> {
        let (count, resolution) = match self {
            Self::Low => (1, 256),
            Self::Medium => (2, 256),
            Self::High => (3, 256),
            Self::Ultra => (3, 512),
        };
        [1024.0, 181.0, 31.0][..count]
            .iter()
            .map(|&size| CascadeInfo { size, resolution })
            .collect()
    }
}

#[path = "../../../vendor/blue-water-sea-and-sky/ocean/src/spectrum.rs"]
pub mod spectrum;
pub use spectrum::WaveParameters;
#[path = "../../../vendor/blue-water-sea-and-sky/ocean/src/groups.rs"]
pub mod groups;
#[path = "../../../vendor/blue-water-sea-and-sky/ocean/src/gusts.rs"]
pub mod gusts;
#[path = "../../../vendor/blue-water-sea-and-sky/ocean/src/sea_state.rs"]
pub mod sea_state;
#[path = "../../../vendor/blue-water-sea-and-sky/ocean/src/whitecaps.rs"]
pub mod whitecaps;
#[path = "../../../vendor/blue-water-sea-and-sky/ocean/src/foam_texture.rs"]
pub mod foam_texture;
#[path = "../../../vendor/blue-water-sea-and-sky/ocean/src/optics.rs"]
pub mod optics;

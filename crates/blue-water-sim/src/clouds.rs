#[path = "../../../vendor/blue-water-sea-and-sky/shared/sim-clouds/types.rs"]
mod types;
pub use types::{CloudGenus, HighCloudGenus, HighClouds};
#[path = "../../../vendor/blue-water-sea-and-sky/shared/sim-clouds/layers.rs"]
pub mod layers;
#[path = "../../../vendor/blue-water-sea-and-sky/shared/sim-clouds/weathers.rs"]
pub mod weathers;
pub use weathers::CloudWeather;
pub const LOCAL_WEATHER_PNG: &[u8] =
    include_bytes!("../../../vendor/blue-water-sea-and-sky/shared/sim-clouds/local_weather.png");

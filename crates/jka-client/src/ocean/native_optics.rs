//! User controls over Blue Water's calibrated optical material.
use blue_water_gpu::original_surface::SurfaceAppearance;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlueWaterOptics {
    /// Optical travel distance multiplier. One preserves upstream calibration.
    pub transparency: f32,
    /// sRGB transmission/body tint and underwater scattering pigment.
    pub water_color: [f32; 3],
    pub fog_color: [f32; 3],
}
impl Default for BlueWaterOptics {
    fn default() -> Self {
        Self {
            transparency: 1.0,
            water_color: [73.0 / 255.0, 87.0 / 255.0, 94.0 / 255.0],
            fog_color: [45.0 / 255.0, 55.0 / 255.0, 60.0 / 255.0],
        }
    }
}
impl BlueWaterOptics {
    pub fn sanitize(mut self) -> Self {
        let defaults = Self::default();
        self.transparency = if self.transparency.is_finite() {
            self.transparency.clamp(0.1, 10.0)
        } else { defaults.transparency };
        for (color, fallback) in [(&mut self.water_color, defaults.water_color),
                                  (&mut self.fog_color, defaults.fog_color)] {
            for (value, default) in color.iter_mut().zip(fallback) {
                *value = if value.is_finite() { value.clamp(0.0, 1.0) } else { default };
            }
        }
        self
    }
    pub fn appearance(self) -> SurfaceAppearance {
        let settings = self.sanitize();
        let mut appearance = SurfaceAppearance::default();
        appearance.absorption = appearance.absorption.map(|v| v / settings.transparency);
        let water = super::linear_color(settings.water_color);
        let fog = super::linear_color(settings.fog_color);
        appearance.transmission = [water[0], water[1], water[2]];
        appearance.pigment = [fog[0], fog[1], fog[2]];
        appearance
    }

    /// Game-unit distance at which even Blue Water's least-absorbed channel
    /// transmits only 0.2% of the scene. This follows the calibrated absorption,
    /// transparency multiplier, and underwater clarity used by the composite.
    /// A non-positive coefficient disables this optimization rather than
    /// risking visible geometry disappearing.
    pub fn absorption_cull_distance_units(self) -> f32 {
        const MIN_TRANSMISSION: f32 = 0.002;
        let least_absorption = self.appearance().absorption.into_iter().fold(f32::INFINITY, f32::min);
        let effective_absorption = least_absorption * blue_water_gpu::underwater::UNDERWATER_CLARITY;
        if !effective_absorption.is_finite() || effective_absorption <= 0.0 {
            return 0.0;
        }
        -MIN_TRANSMISSION.ln() / effective_absorption * crate::ocean::OCEAN_UNITS_PER_METER
    }
}

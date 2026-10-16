//! Preserved cloud profiles and celestial clock.
pub mod clouds;
pub mod theatre;
pub mod geometry {
    pub fn wrap_angle(angle: f64) -> f64 {
        (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    }
}

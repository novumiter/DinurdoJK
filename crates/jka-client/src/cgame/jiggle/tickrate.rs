//! Dinurdo input conversion for a 60 Hz reference response.
//! The KawaiiPhysics and JigglePhysics solve equations remain source-shaped.
//! Convert the authored per-step inputs, not the saved slider values.

use super::{add, distance, mul, sub};

pub(super) const REFERENCE_HZ: f32 = 60.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct Response {
    pub stiffness: f32,
    pub damping: f32,
    pub force_scale: f32,
    pub impulse_scale: f32,
}

/// Resample the reference Verlet recurrence's decay and oscillation poles.
/// Merely multiplying stiffness by dt does not preserve bounce frequency;
/// damping and the velocity loss caused by the spring projection are coupled.
pub(super) fn reference_response(stiffness: f32, damping: f32, dt: f32) -> Response {
    let h = REFERENCE_HZ * dt;
    if (h - 1.0).abs() < 0.000001 {
        return Response {
            stiffness,
            damping,
            force_scale: 1.0,
            impulse_scale: 1.0,
        };
    }
    let (k, d, h) = (stiffness as f64, damping as f64, h as f64);
    let q = 1.0 - k;
    let a = q * (2.0 - d);
    let b = q * (1.0 - d);
    let discriminant = a * a - 4.0 * b;
    let bh = b.powf(h);
    let (ah, impulse_basis) = if discriminant < -1.0e-12 {
        let radius = b.sqrt();
        let angle = (a / (2.0 * radius)).clamp(-1.0, 1.0).acos();
        let radius_h = radius.powf(h);
        (
            2.0 * radius_h * (h * angle).cos(),
            radius.powf(h - 1.0) * (h * angle).sin() / angle.sin(),
        )
    } else {
        let root = discriminant.max(0.0).sqrt();
        let plus = ((a + root) * 0.5).max(0.0);
        let minus = ((a - root) * 0.5).max(0.0);
        let plus_h = plus.powf(h);
        let minus_h = minus.powf(h);
        let basis = if root > 1.0e-8 {
            (plus_h - minus_h) / root
        } else if plus > 1.0e-8 {
            h * plus.powf(h - 1.0)
        } else {
            0.0
        };
        (plus_h + minus_h, basis)
    };
    let qh = ah - bh;
    let step_k = (1.0 - qh).clamp(0.0, 1.0);
    let step_d = if qh > 1.0e-8 { 1.0 - bh / qh } else { 1.0 };
    // The spring projection also attenuates velocity. At coarse rates its
    // paired damping input can be negative while the total retained momentum
    // is still bh <= 1. Resampling that coupled response avoids silently adding
    // damping when the user selected a low damping value.
    // If qh is nonpositive, the reference oscillation is under-resolved within
    // this solve form. Use a bounded response until more ticks resolve it.
    let resolved = qh > 1.0e-8;
    let force_scale = if !resolved {
        1.0
    } else if k > 1.0e-6 {
        q * step_k / (k * qh * h * h)
    } else if d > 1.0e-6 {
        step_d / (d * h)
    } else {
        1.0
    };
    let impulse_scale = if resolved {
        q * impulse_basis / (qh * h)
    } else {
        1.0
    };
    Response {
        stiffness: step_k as f32,
        damping: step_d as f32,
        force_scale: force_scale.max(0.0) as f32,
        impulse_scale: impulse_scale.max(0.0) as f32,
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct JiggleInputs {
    pub stiffness: f32,
    pub drag: f32,
    pub air_drag: f32,
    pub stretch: f32,
    pub soften: f32,
    pub force_scale: f32,
    pub impulse_scale: f32,
}

pub(super) fn jiggle_inputs(
    stiffness: f32,
    drag: f32,
    air_drag: f32,
    stretch: f32,
    dt: f32,
) -> JiggleInputs {
    let h = REFERENCE_HZ * dt;
    if (h - 1.0).abs() < 0.000001 {
        return JiggleInputs {
            stiffness,
            drag,
            air_drag,
            stretch,
            soften: 0.0,
            force_scale: 1.0,
            impulse_scale: 1.0,
        };
    }
    let angle = stiffness * stiffness;
    let length = (1.0 + (stiffness - 1.0) * stretch).powi(2);
    constraint_inputs(angle, length, drag, air_drag, stretch, dt)
}

fn constraint_inputs(
    angle: f32,
    length: f32,
    drag: f32,
    air_drag: f32,
    stretch: f32,
    dt: f32,
) -> JiggleInputs {
    let h = REFERENCE_HZ * dt;
    let angular = reference_response(angle, drag, dt);
    let radial = reference_response(1.0 - (1.0 - angle) * (1.0 - length), drag, dt);
    let step_stiffness = angular.stiffness.sqrt();
    let step_length = if angular.stiffness < 0.999999 {
        ((radial.stiffness - angular.stiffness) / (1.0 - angular.stiffness)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let step_stretch = if step_stiffness < 0.999999 {
        ((1.0 - step_length.sqrt()) / (1.0 - step_stiffness)).clamp(0.0, 1.0)
    } else {
        stretch
    };
    // Parent motion enters as (Drag - AirDrag) * parent_delta. Delta already
    // scales with dt; scale that difference once more for a consistent force.
    let step_air_drag = angular.damping - (drag - air_drag) * h * angular.force_scale;
    JiggleInputs {
        stiffness: step_stiffness,
        drag: angular.damping,
        air_drag: step_air_drag,
        stretch: step_stretch,
        soften: 0.0,
        force_scale: angular.force_scale,
        impulse_scale: angular.impulse_scale,
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct JiggleReference {
    pub stiffness: f32,
    pub drag: f32,
    pub air_drag: f32,
    pub stretch: f32,
    pub soften: f32,
    pub gravity: f32,
}

/// Soften changes the spring according to its predicted error. Convert that
/// effective reference spring, rather than a fully stiff spring followed by
/// an unconverted error multiplier; otherwise low-drag/high-Soften controls
/// pick up artificial damping as the tickrate rises.
#[allow(clippy::too_many_arguments)]
pub(super) fn softened_jiggle_inputs(
    reference: JiggleReference,
    position: [f32; 3],
    previous: [f32; 3],
    root: [f32; 3],
    previous_root: [f32; 3],
    target: [f32; 3],
    kick: [f32; 3],
    impulse: bool,
    dt: f32,
) -> JiggleInputs {
    let h = REFERENCE_HZ * dt;
    if reference.soften <= 0.0 || (h - 1.0).abs() < 0.000001 {
        let mut inputs = jiggle_inputs(
            reference.stiffness,
            reference.drag,
            reference.air_drag,
            reference.stretch,
            dt,
        );
        inputs.soften = reference.soften;
        return inputs;
    }
    let particle_delta = sub(position, previous);
    let parent_delta = sub(root, previous_root);
    let parent_reference = mul(parent_delta, 1.0 / h);
    let local_reference = mul(sub(particle_delta, parent_delta), 1.0 / h);
    let reference_dt = 1.0 / REFERENCE_HZ;
    let gravity = [
        0.0,
        0.0,
        -800.0 * reference.gravity * reference_dt * reference_dt,
    ];
    let reference_kick = mul(kick, 1.0 / if impulse { h } else { h * h });
    let prediction = add(
        add(
            position,
            add(
                mul(parent_reference, 1.0 - reference.air_drag),
                mul(local_reference, 1.0 - reference.drag),
            ),
        ),
        add(gravity, reference_kick),
    );
    let error = (distance(prediction, target) / distance(root, target).max(0.001)).clamp(0.0, 1.0);
    let effective_angle =
        reference.stiffness * reference.stiffness * error.powf(reference.soften * 2.0);
    // Length still uses the user's full stiffness/stretch controls, matching
    // the source; only the angle constraint is softened.
    let length = (1.0 + (reference.stiffness - 1.0) * reference.stretch).powi(2);
    // Soften is encoded in the converted stiffness input. The source solve
    // receives a zero exponent here, retaining the same constraint equations.
    constraint_inputs(
        effective_angle,
        length,
        reference.drag,
        reference.air_drag,
        reference.stretch,
        dt,
    )
}

/// Match the reference recurrence's anchor-input phase. Higher rates would
/// otherwise remove part of the reference's one-tick feed-forward and change
/// walking/animated bounce despite matching the free spring response.
pub(super) fn reference_anchor(current: [f32; 3], previous: [f32; 3], dt: f32) -> [f32; 3] {
    let h = REFERENCE_HZ * dt;
    if (h - 1.0).abs() < 0.000001 {
        return current;
    }
    let lead = 1.0 / h - 1.0;
    [
        current[0] + (current[0] - previous[0]) * lead,
        current[1] + (current[1] - previous[1]) * lead,
        current[2] + (current[2] - previous[2]) * lead,
    ]
}

/// Keep the catch-up time budget of the reference tick count. At 240 Hz,
/// four reference ticks allow sixteen actual ticks, still about 67 ms.
pub(super) fn catch_up_steps(reference_steps: u32, hz: u32) -> usize {
    ((reference_steps.clamp(1, 8) * hz + 59) / 60).max(1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_rate_preserves_controls_exactly() {
        let r = reference_response(0.1, 0.18, 1.0 / 60.0);
        assert_eq!(
            (r.stiffness, r.damping, r.force_scale, r.impulse_scale),
            (0.1, 0.18, 1.0, 1.0)
        );
        let j = jiggle_inputs(0.4, 0.4, 0.1, 0.8, 1.0 / 60.0);
        assert_eq!(
            (j.stiffness, j.drag, j.air_drag, j.stretch),
            (0.4, 0.4, 0.1, 0.8)
        );
        assert_eq!((j.force_scale, j.impulse_scale), (1.0, 1.0));
    }

    #[test]
    fn common_spring_decay_and_frequency_match_reference() {
        for hz in [30.0, 120.0, 240.0] {
            let h = 60.0 / hz;
            let r = reference_response(0.1, 0.18, 1.0 / hz);
            let old_b = 0.9_f32 * 0.82;
            let new_b = (1.0 - r.stiffness) * (1.0 - r.damping);
            assert!((new_b - old_b.powf(h)).abs() < 0.00001);
            let old_angle = (0.9_f32 * 1.82 / (2.0 * old_b.sqrt())).acos();
            let new_angle = ((1.0 - r.stiffness) * (2.0 - r.damping) / (2.0 * new_b.sqrt())).acos();
            assert!((new_angle / h - old_angle).abs() < 0.00001);
        }
    }

    #[test]
    fn low_damping_keeps_coupled_momentum_loss_at_coarse_rate() {
        let r = reference_response(0.06, 0.0, 1.0 / 30.0);
        assert!(r.damping < 0.0);
        let retained = (1.0 - r.stiffness) * (1.0 - r.damping);
        assert!((retained - 0.94_f32.powi(2)).abs() < 0.00001);
    }

    #[test]
    fn moving_anchor_keeps_reference_input_phase() {
        assert_eq!(
            reference_anchor([1.0, 2.0, 3.0], [0.0; 3], 1.0 / 60.0),
            [1.0, 2.0, 3.0]
        );
        assert_eq!(
            reference_anchor([1.0, 2.0, 3.0], [0.0; 3], 1.0 / 120.0),
            [2.0, 4.0, 6.0]
        );
    }

    #[test]
    fn softened_zero_drag_uses_the_effective_reference_spring() {
        let reference = JiggleReference {
            stiffness: 0.4,
            drag: 0.0,
            air_drag: 0.1,
            stretch: 0.8,
            soften: 1.0,
            gravity: 0.0,
        };
        let at_reference = softened_jiggle_inputs(
            reference,
            [0.0, -5.0, 0.0],
            [-0.05, -5.0, 0.0],
            [0.0; 3],
            [0.0; 3],
            [0.0, -5.0, 0.0],
            [0.0; 3],
            false,
            1.0 / 60.0,
        );
        assert_eq!(
            (
                at_reference.stiffness,
                at_reference.drag,
                at_reference.soften
            ),
            (0.4, 0.0, 1.0)
        );
        let finer = softened_jiggle_inputs(
            reference,
            [0.0, -5.0, 0.0],
            [-0.025, -5.0, 0.0],
            [0.0; 3],
            [0.0; 3],
            [0.0, -5.0, 0.0],
            [0.0; 3],
            false,
            1.0 / 120.0,
        );
        assert_eq!(finer.soften, 0.0);
        assert!(finer.drag.abs() < 0.001);
        assert!(finer.stiffness < 0.01);
    }

    #[test]
    fn catch_up_budget_scales_with_actual_steps() {
        assert_eq!(catch_up_steps(4, 30), 2);
        assert_eq!(catch_up_steps(4, 60), 4);
        assert_eq!(catch_up_steps(4, 120), 8);
        assert_eq!(catch_up_steps(4, 240), 16);
    }
}

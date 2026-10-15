//! Per-entity logic from TaystJK cg_turret.c TurretClientRun/CreepToPosition.
//! No model-name guesses: CG_General selects this only for Ghoul2 + bolt1.

#[derive(Clone, Debug)]
pub(super) struct TurretState {
    pub model_index: i32,
    pub started_at: i32,
    pub last_time: i32,
    pub angles: [f32; 3], // PITCH, YAW, ROLL in JKA order
    last_fireflag: i32,
}

impl TurretState {
    pub fn new(model_index: i32, time: i32) -> Self {
        Self { model_index, started_at: time, last_time: time,
            angles: [90.0, 0.0, 0.0], last_fireflag: 0 }
    }

    /// Returns true only when a new nonzero fireflag requests a muzzle flash.
    pub fn update(&mut self, time: i32, origin: [f32; 3],
                  target: Option<[f32; 3]>, fireflag: i32) -> bool {
        let dt = (time - self.last_time).clamp(0, 12000);
        self.last_time = time;
        // fireflag==2 is the imminent-explosion marker: freeze the hinge.
        if fireflag == 2 { return false; }
        let flash = fireflag != 0 && fireflag != self.last_fireflag;
        self.last_fireflag = fireflag;
        if let Some(target) = target {
            let d = [target[0]-origin[0], target[1]-origin[1], target[2]-origin[2]];
            let horiz = (d[0]*d[0]+d[1]*d[1]).sqrt();
            if horiz > 0.0001 || d[2].abs() > 0.0001 {
                let ideal = [90.0 - d[2].atan2(horiz).to_degrees(),
                    d[1].atan2(d[0]).to_degrees().rem_euclid(360.0), 0.0];
                creep_to_position(ideal, &mut self.angles);
            }
        } else {
            // TaystJK advances idle yaw by 0.03 degrees/ms, including when
            // returning from target tracking. Preserve its 90-degree clamping.
            let ideal = [90.0, self.angles[1] + (dt as f32 * 0.03).min(360.0), 0.0];
            creep_to_position(ideal, &mut self.angles);
        }
        flash
    }
}

/// TaystJK CreepToPosition: chooses shortest wraparound direction, moving up
/// to 90 degrees per invocation, then snaps when within the 180-degree reach.
fn creep_to_position(ideal: [f32; 3], current: &mut [f32; 3]) {
    for index in [1, 0] {
        let wanted = ideal[index].rem_euclid(360.0);
        let now = current[index].rem_euclid(360.0);
        let positive = (wanted - now).rem_euclid(360.0);
        let negative = (now - wanted).rem_euclid(360.0);
        // TaystJK branches on truncated integer angles. Here we retain the
        // fractions needed for high-FPS idle stepping without 1-degree jitter.
        current[index] = if negative < positive {
            if negative <= 90.0 { wanted } else { (now - 90.0).rem_euclid(360.0) }
        } else if positive <= 90.0 { wanted }
        else { (now + 90.0).rem_euclid(360.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn idle_sweep_uses_elapsed_milliseconds() {
        let mut t = TurretState::new(21, 1000);
        t.update(1100, [0.0; 3], None, 0);
        assert!((t.angles[1] - 3.0).abs() < 0.01);
    }
    #[test] fn target_tracks_and_fireflag_is_edge_triggered() {
        let mut t = TurretState::new(21, 1000);
        assert!(t.update(1001, [0.0; 3], Some([0.0, 100.0, 0.0]), 1));
        assert!(!t.update(1002, [0.0; 3], None, 1));
        assert!((t.angles[1] - 90.0).abs() < 0.01);
        assert!(!t.update(1003, [0.0; 3], None, 2));
        t.update(1004, [0.0; 3], None, 0);
        assert!(t.update(1005, [0.0; 3], None, 1));
    }
    #[test] fn angle_wrap_shortest_path() {
        let mut a = [90.0, 350.0, 0.0];
        creep_to_position([90.0, 10.0, 0.0], &mut a);
        assert!((a[1]-10.0).abs()<0.01);
    }
}

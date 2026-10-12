//! CG_PlayerSplash from TaystJK / OpenJK, with optional low-cost enhanced
//! animation. This is ONLY for collision-backed vanilla liquid, never the
//! Godot-promoted water, which owns its own wake/deformation renderer.
use std::collections::HashMap;
use jka_movement::{CollisionWorld, TraceQuery, TraceWorld, ENTITY_NONE};
use crate::{fx::system::FxDraw, ocean::{appearance::PromotedOcean, authoring::AuthoredOcean}};
use crate::ocean::interaction::ActorSample;

// surfaceflags.h: JKA CONTENTS_LAVA / WATER / SLIME.
const LIQUID: i32 = 0x2 | 0x4 | 0x8;
const SOLID: i32 = 0x1;
const SAMPLE_MS: i32 = 50; // max 20 collision polls / second / player
const HALF_QUAD: f32 = 32.0; // exact stock wake quad size

#[derive(Clone, Copy, Default)]
struct Contact {
    checked_at: i32,
    origin: [f32; 3],
    surface_z: Option<f32>,
}

#[derive(Default)]
pub(in crate::app) struct WaterWakeTracker {
    contacts: HashMap<u16, Contact>,
}

impl WaterWakeTracker {
    pub fn clear(&mut self) {
        self.contacts.clear();
    }

    pub fn append_draws(
        &mut self,
        mode: u8,
        time_ms: i32,
        actors: &[ActorSample],
        mut collision: Option<&mut CollisionWorld>,
        promoted: &[PromotedOcean],
        authored: &[AuthoredOcean],
        output: &mut Vec<FxDraw>,
    ) {
        if mode == 0 { self.clear(); return; }
        let Some(world) = collision.as_deref_mut() else { return; };
        for actor in actors.iter().take(64) {
            // Simulation actors have render-space coordinates; liquid traces
            // and wake vertices must use original JKA XYZ coordinates.
            let position = crate::scene::jka_position(actor.origin);
            if !position.iter().all(|v| v.is_finite()) { continue; }
            let contact = self.contacts.entry(actor.id).or_default();
            let dx = position[0] - contact.origin[0];
            let dy = position[1] - contact.origin[1];
            if time_ms < contact.checked_at || time_ms - contact.checked_at >= SAMPLE_MS
                || dx*dx + dy*dy > 48.0*48.0
            {
                contact.checked_at = time_ms;
                contact.origin = position;
                contact.surface_z = liquid_surface(world, position);
            }
            let Some(z) = contact.surface_z else { continue; };
            // Strictly exclude all promoted/authored water from the legacy FX
            // path; any wave displacements stay owned by the Godot renderer.
            let at_surface = [actor.origin[0], z, actor.origin[2]];
            if promoted.iter().any(|pool| {
                let surface = pool.surface;
                at_surface[0] >= surface.minimum[0] && at_surface[0] <= surface.maximum[0]
                    && at_surface[2] >= surface.minimum[1] && at_surface[2] <= surface.maximum[1]
                    && (z - surface.plane_height).abs() <= 64.0
            }) || authored.iter().any(|body| body.contains_render_point(at_surface)) {
                continue;
            }
            let velocity = actor.velocity.map(crate::scene::jka_position).unwrap_or([0.0;3]);
            let speed = velocity[0].hypot(velocity[1]);
            let enhanced = mode == 2;
            // Mode 1 matches TaystJK CG_PlayerSplash: horizontal 64x64 quad,
            // uv corners (0,0)-(1,1), full vertex RGBA and shader "wake".
            // Mode 2 adds cheap vertex-only motion (no new material/pass).
            let (side, backward, heading) = if enhanced {
                let normalized_speed = (speed / 320.0).clamp(0.0, 1.0);
                let oscillation = (time_ms as f32 * 0.0035 + f32::from(actor.id)).sin();
                let heading = if speed > 3.0 {
                    [velocity[0]/speed, velocity[1]/speed]
                } else { [0.0, 1.0] };
                (HALF_QUAD * (1.0 + 0.08 * oscillation),
                    HALF_QUAD * (1.0 + 0.52 * normalized_speed + 0.07 * oscillation),
                    heading)
            } else { (HALF_QUAD, HALF_QUAD, [0.0, 1.0]) };
            let across = [heading[1], -heading[0]];
            let corners = [[-1.0,-1.0],[-1.0,1.0],[1.0,1.0],[1.0,-1.0]];
            let positions = corners.map(|[x,y]| {
                // Motion stretches the texture backward, like a narrow wake.
                let trail = if enhanced { -0.18 * (speed/320.0).clamp(0.0, 1.0)*backward } else {0.0};
                [position[0] + across[0]*x*side + heading[0]*(y*backward+trail),
                 position[1] + across[1]*x*side + heading[1]*(y*backward+trail), z]
            });
            // Keep the authored texture's 0..1 UVs in both modes. Rotating UVs
            // would introduce repeat seams in stock/custom JKA wake assets.
            let uvs = [[0.0,0.0],[0.0,1.0],[1.0,1.0],[1.0,0.0]];
            output.push(FxDraw::Quad {
                positions, uvs, rgba: [255;4], shader: "wake".into(),
            });
        }
        // Prevent stale cache entries from following recycled player ids.
        self.contacts.retain(|_, state| time_ms >= state.checked_at
            && time_ms - state.checked_at < 1000);
    }
}

fn liquid_surface(world: &mut impl TraceWorld, origin: [f32;3]) -> Option<f32> {
    let foot = [origin[0], origin[1], origin[2] - 24.0];
    if world.point_contents(foot, ENTITY_NONE) & LIQUID == 0 { return None; }
    let head = [origin[0], origin[1], origin[2] + 32.0];
    if world.point_contents(head, ENTITY_NONE) & (SOLID | LIQUID) != 0 { return None; }
    let hit = world.trace(TraceQuery {
        start: head, end: foot, mins: [0.0;3], maxs: [0.0;3],
        pass_entity: ENTITY_NONE, mask: LIQUID,
    });
    (hit.fraction < 1.0 && hit.end[2].is_finite()).then_some(hit.end[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use jka_movement::TraceResult;
    struct Water;
    impl TraceWorld for Water {
        fn point_contents(&mut self, at: [f32;3], _: i32) -> i32 { if at[2] < 0.0 { LIQUID } else { 0 } }
        fn trace(&mut self, query: TraceQuery) -> TraceResult {
            TraceResult { fraction: (query.start[2]/(query.start[2]-query.end[2])).clamp(0.0,1.0),
                end: [query.start[0],query.start[1],0.0], ..TraceResult::clear(query.end) }
        }
    }
    #[test]
    fn wake_requires_wading_not_full_submersion() {
        assert_eq!(liquid_surface(&mut Water, [0.0,0.0,12.0]), Some(0.0));
        assert_eq!(liquid_surface(&mut Water, [0.0,0.0,40.0]), None);
        assert_eq!(liquid_surface(&mut Water, [0.0,0.0,-40.0]), None);
    }
}

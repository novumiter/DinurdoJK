# Current player presentation

`GameSession::presented_player_state()` maps to TaystJK's
`cg.predictedPlayerState`. `App::current_player_state()` is the app gateway;
`live_player_state()` retains the live-only contract for gameplay callers.
Read-only getters reuse the cached full `PlayerState` for the logical frame.

`advance_to_with_prediction` transitions snapshots, advances the existing
Predictor (including its provisional display command), and publishes the
canonical player state before audio, models, weapons, camera and HUD consume it.
Without prediction, `cgame::player_state::interpolate_player_state_at` supplies
the full state at the same presentation time used for packet entities.
`PresentedEntity` for the viewed player is converted once from that state and
shared by rendering and spatial audio. The separate followed-player lerp is gone.

## Source semantics

The authority is TaystJK's
[`CG_InterpolatePlayerState`](https://github.com/taysta/TaystJK/blob/master/codemp/cgame/cg_predict.c)
and [`CG_SetNextSnap`](https://github.com/taysta/TaystJK/blob/master/codemp/cgame/cg_snapshot.c).
Start with a clone of current snapshot state; interpolate only origin,
velocity, viewangles and bobCycle. Angles use the legacy LerpAngle adjustment,
including its signed exact-180-degree tie. Bob adds 256 to the next cycle when
it decreases, then truncates the float result to int without masking it:
250 -> 5 produces 255 at fraction .5 and 258 at .75. Viewheight, commandTime,
stats, animation, events and other fields remain current-snapshot values.
Eye height is derived afterward, with the existing intermission rule.

No next snapshot, non-increasing snapshot time, a changed teleport bit,
a changed followed client, or a server-count restart suppress interpolation.
The snapshot loop advances at the boundary before sampling; its fractional
presentation clock remains intact. There is no followed-player extrapolation.

## Subframe input and camera effects

Committed prediction and provisional display state remain owned by Predictor.
Presentation clones display, including its provisional origin and velocity.
It applies mouse movement accumulated since the display command before deriving
camera, weapon and body transforms, without committing that angle update to Pmove.
Local play keeps full-precision subframe server angles. No-predict mode applies
current input plus current snapshot delta_angles. Followed, dead and forced remote
views keep their input ownership. Renderer late input remains enabled on its
existing direct/third-person paths; orbit, damping, prediction error, view kick
and camera shake remain camera effects. Prediction error offsets the camera,
while body and movement HUD continue to read the canonical player origin.

## Snapshot reads retained intentionally

Snapshot transitions still own follow/demo events and their protocol timestamps.
Follow-name/team and scoreboard focus still use snapshot identity as TaystJK does.
Collision/prediction inputs, network/usercmd synchronization, area masks, recorded
strafe-trail logging and replay indexing remain authoritative snapshot consumers.
Arbitrary remote entities and alternate demo POVs retain their EntityState
presentation; race ghosts keep their separate auxiliary interpolation.

Crosshair tracing still runs before RenderSnapshot publication. Canonical state
is initialized on the first active snapshot, refreshed on demo seek, and cleared
on stream reset. Tests cover interpolation boundaries, field preservation,
shared camera/body transforms, provisional movement, short-angle wrap, immediate
input, no-predict delta angles, forced views and spectator input isolation.

# Siege patch integration review

Reviewed on 2026-10-08 against the local `D:\Code\JAPRO\codemp` client and UI sources.

## Applied change

The supplied `Development Docs/Patch20/DinurdoJK-siege-cumulative-from-20261007.patch`
is an incremental 21-file patch, despite its filename. It assumes Siege modules
and fields that were absent from this working tree.

Its prerequisites were recovered by comparing the adjacent
`ctf-checkpoint-20261007.zip` and `DinurdoJK-siege-current-checkpoint.zip`.
Every file hash named as an input by the supplied patch matches the Siege ZIP.
The prerequisite delta and supplied patch were merged with the current working
tree using the repository's `crates/` prefix. Existing uncommitted work and old
reject files were preserved. Before-images and integration patches are under
`target/siege-integration/`.

Integration corrections:

- Preserved the newer CTF scoreboard statistics, ready mask, and existing
  powerup fields instead of duplicating fields from the older checkpoint.
- Retained the existing `player_presenter/powerups.rs` carrier renderer, including
  first-person suppression, flag alpha, CTY models, and powerup lights. Applying
  the patch's second renderer would have duplicated flags and lights. Extended
  the existing renderer to accept a neutral flag in Siege.
- Added the missing Siege class to indexed demo score rows.
- Reset new extended-stat and respawn state during map restart and demo seek;
  reject stats timestamped in the future after a clock reset.
- Load StringEd before opening a briefing/class menu.
- Updated two existing ocean GPU test calls with the new splash-mode argument
  so the client test target compiles. These are test-only changes.

## Presentation now connected

- `.scl` classes, `.team` themes, `.siege` map groups, and server team overrides
  resolve through the existing asset search path.
- `CS_SIEGE_STATE`, objectives, time override, and winning team feed native HUD
  and scoreboard presentation.
- `sb`, `scl`, and `spc` open native briefing, class-selection, and profile
  dialogs. `sxd` supplies teammate health/ammo data for stat-viewer classes.
- `EV_SIEGESPEC` sets the viewed player's respawn deadline. Objective-complete
  and round-over events resolve team-specific messages and announcer sounds.
- Objective messages resolve their single-leading-`@` StringEd keys separately
  from generic server center-print syntax. Round-over text opens its own dialog.
- Siege scoreboard rows use carried-flag icons before class badges. Their icons
  share the same atlas as the team overlay and duel mini scoreboard.
- `O` defaults to `ui_opensiegemenu ingame_objectives`. Existing saved binds can
  override that default.

## Remaining client presentation work

These findings come from tracing consumers, rather than assuming that a parsed
field is already drawn. This is still a partial Siege client presentation port.

| Priority | Missing or partial behavior | Evidence and next implementation |
| --- | --- | --- |
| High | Carried objective attachment and first-person item preview | JAPRO `cg_ents.c:CG_RenderTimeEntBolt` attaches `boltToPlayer` entities to the carrier's hand; `cg_draw.c:CG_DrawSiegeHUDItem` draws the local carried model on the HUD. Neither path is implemented here. Flag powerups are a separate mechanism and do not cover these objects. |
| High | Graphical objectives screen | `mapgraphic`, `objgfx`, `mapicon`, `litmapicon`, `donemapicon`, and `mappos` are parsed, but `app/siege.rs` draws briefing text and completion markers only. Port the map image, positioned objective buttons/icons, completed/selected states, and objective detail graphics from JAPRO `cg_saga.c` and `ui_main.c:UI_UpdateSiegeObjectiveGraphics`. |
| High | Team selection within the class menu | The native dialog lists the current viewer's team, falling back to team 1 for spectators. It has no team selector or coordinated team/class change flow. Port the stock UI's team selection and `setsiegeclassandteam` behavior. |
| Medium | Map/theme-specific teammate symbols | `FriendlyShader` is loaded from maps/themes but never submitted above players. JAPRO `cg_players.c` uses the selected team's shader, with standard red/blue symbols as fallback, and respects `cg_drawFriend`. |
| Medium | Full class-selection/profile visuals | `uishader` portraits, player/model preview, class categories, weapon/inventory/force icons and descriptions, class ability details, and complete player configuration are absent. The current native profile shows selected textual stats and raw weapon identifiers. Compare JAPRO `ui_main.c:UI_SiegeSetCvarsForClass` and the Siege menu assets. |
| Medium | Countdown audio and localized HUD labels | The patch adds `siege_last_countdown` but no code plays the stock 3/2/1 announcer sounds. Waiting/countdown/respawn strings are English literals, and the timer/respawn widgets use new text placement rather than stock graphics. Compare JAPRO `cg_draw.c:CG_Draw2D`, `CG_DrawSiegeTimer`, and `CG_DrawSiegeDeathTimer`. |
| Medium | Complete stat-viewer ammo semantics | `app/session/siege.rs` accepts any nonnegative ammo count and `app/siege.rs` uses ordinary ammo maxima. JAPRO validates ammo against the weapon maximum and doubles that maximum for `EF_DOUBLE_AMMO`; ammo-free weapons show a full bar. The current widget is a basic native panel. |
| Lower | Legacy Siege UI command coverage | Only `ui_opensiegemenu ingame_objectives` is implemented; other menu names are handled no-ops. Client console aliases `briefing`, `siegeCvarUpdate`, and `siegeCompleteCvarUpdate` and their old UI-cvar consumers are absent. Decide which aliases are useful for native UI compatibility. |

Additional integration considerations:

- The catalog eagerly parses every mounted Siege map when any CGame is created.
  A malformed unrelated custom `.siege` file can therefore block a non-Siege
  session. Stock assets passed; custom asset failure isolation remains to do.
- `active_siege_map` reparses map text for each caller. Cache its result by map
  name and the two server team overrides before expanding graphical consumers.
- Dedicated live-server/demo smoke tests are still needed for class/team changes,
  round transitions, carried objectives, extended-stat timing during seeking, and
  actual texture appearance. Compilation and data tests do not establish visual
  parity.

## Validation

- Offline client compilation, including the test target, passed during integration.
- Siege asset tests passed, including real installed PK3s: 57 classes, 10 themes,
  all three stock maps, and each map's referenced classes/objectives resolved.
- Client validation passed: nine Siege tests, seven binding tests, one stock
  score-command parsing test, and one real-asset carried-flag rendering test.
  This covers state/objective/extended-data parsing, selective action draining,
  forced class models, carrier model selection, flag-over-class icon priority,
  shared atlas offsets, and first-person flag suppression.
- The normal client test link encountered MSVC `LNK1318` while generating the
  PDB. The successful build used
  `cargo rustc --offline -p DinurdoJK --tests --profile test -- -C link-arg=/DEBUG:NONE -C link-arg=/INCREMENTAL:NO`.
  The generated test executable was run with the existing Steam Audio DLL
  directory on the process-local `PATH`. Failed-link outputs were removed after
  they exhausted the build drive's free space.

Suggested order: carried objectives, graphical objective map, team/class menu,
teammate markers, then remaining class widgets and HUD/audio parity.

# Console and cvar definitions

All local commands and cvars are declared in `crates/jka-client/src/app/commands/registry/`, grouped by their subsystem. There is no separate local cvar metadata table, name-based App getter/setter dispatcher, or manual config-save list.

`commands!` supplies a command's name, aliases, usage, description, scope and handler. The macro generates registration and dispatch together. Gameplay protocol actions are attached to the same definition. Server commands keep their connection-specific forwarding and registration rules.

`app_cvars!` supplies a cvar's runtime descriptor, config binding and implementation together. `settings_cvars!` additionally binds the declaration to typed network or JAPRO settings and generates that subsystem's registry. A new declaration automatically participates in console lookup, completion, help, cvarlist and config loading/saving. The HUD's small typed `video_cvars!` declarations are also included in the complete registry.

State stays in typed Rust fields. Simple cvars use the common typed parser. Custom callbacks parse console text at the boundary and retain the current numeric ranges, clamping, formatting and explicit renderer/UI/network effects. They do not introduce a string-backed state store.

The complete namespace contains 460 cvars, including previously hidden executable cvars. 421 carry ARCHIVE. Each archived declaration supplies its settings getter and config setter; save iterates these definitions in their declared order, retaining the existing quoting, precision and binding placement. The fixed `r_mode = -1` marker has no mutable settings field. Diagnostics, read-only userinfo mirrors, passwords and other runtime-only values retain their existing archive policy.

Archived defaults are read from the typed settings defaults through their own archive getters. Network/JAPRO default metadata comes from their typed owner. HUD defaults remain declared by the typed HUD definitions. This avoids another list of archived default literals. Runtime-only video/audio/presentation defaults also use their typed settings getters. Session-only defaults are supplied explicitly in their definitions.

Latch scope, validation/normalization and the inactive video-settings callback are attached to the cvar. The App retains the pending-value lifecycle, restart notifications and cancellation behavior. Renderer and network callbacks remain explicit in the declarations.

Config loading reads the requested current config file only. Startup uses `DinurdoJK.cfg`; it does not fall back to `jampconfig.cfg`. Obsolete fog, cloud-wind, ocean-weather and baked-brightness keys are ignored. HDR and tone mapping load independently. Missing files use the current typed defaults. The current set/seta/sets/setu grammar and binding syntax remain supported.

Validation includes unique names and aliases, valid defaults/domains, archive getter/setter completeness, stable archive positions, latch completeness and screenshot discovery. Frozen regression fixtures compare 20 representative inputs per cvar against pre-conversion behavior, exact default/changed archive text and pure network/JAPRO setters. A Windows headless App test additionally checks all exposed runtime getters, setter results, dirty/debounce/pacing effects and latch normalization. The temporary original implementation used to capture those fixtures is not compiled or shipped.

Run `cargo check -p DinurdoJK --offline` and `cargo test -p DinurdoJK --offline -- --test-threads=1`. Tests exercise the registry and inactive/headless state; rendered gameplay and GPU side effects still require an in-game smoke test.

Conversion validation: `cargo check` passes. The full client test run has 659 passing tests, 34 ignored and six failures outside the registry regression checks: the invulnerability-sphere transform, FX quad index count, two material-stage cases, JAPRO server-configstring reset and RT-model vertex binding collision. All registry, config, subsystem-setter and headless runtime/latch regression tests pass.

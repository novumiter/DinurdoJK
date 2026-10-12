# DinurdoJK extensions — stage 1 (Rhai runtime)

### Location and authoring

Extensions live in `<your JKA base directory>/extensions/<id>/` (next to the normal game files in `base/`). `id` contains ASCII letters/digits, `_` or `-` and is at most 48 characters. Each extension contains:

```
base/extensions/hello/
  extension.toml
  main.rhai
```

`extension.toml`:

```toml
api_version = 1
name = "My first extension"
entry = "main.rhai"
```

`main.rhai`:

```rhai
print("Hi from DinurdoJK");
let answer = 40 + 2;
print(`The answer is ${answer}`);
```

Open **Setup -> EXTENSIONS** in the client. Click **CREATE** to write a template on disk, select it, edit its real source in the text editor and click **SAVE & RELOAD**. Files saved from external editors are polled twice per second and hot-reloaded automatically. **RELOAD FROM DISK** forces compilation even if no file bytes changed; **RESCAN** checks the extension directory. **Enabled** toggles a `.disabled` marker in that extension directory.

### Stage 1 behavior

- Compiles Rhai into an AST and evaluates top-level statements **only upon successful load/reload** on one dedicated worker thread. No script code runs in a render callback. Extensions cannot currently access playerstate, HUD drawing, 3D drawing, game events, network or filesystem through host functions.
- Each extension has its own `Engine`, AST and global `Scope`, retained on the worker for future event dispatch. No tracing garbage collector is required. The engine uses bounded instruction count, recursion, expressions, collections and string lengths. There is **not yet** a wall-clock timeout or operating-system isolation.
- Syntax/runtime errors are reported in Setup and console. If a newer edit fails, the previously active AST remains loaded; disabled extensions are unloaded. File sizes: scripts <= 256 KiB, manifests <= 16 KiB; at most 64 extensions are enumerated.
- Main/UI thread only polls responses and sends commands. Script compilation and execution remain on the extension worker.
- No new WGPU render pass, no shader specialization, no new HUD components, and no protocol/network state changes are introduced by this patch.

### Next patches

1. Immutable playerstate/snapshot bridge (predicted vs authoritative data and spectator/demo identity).
2. Event queue from accepted presentation events with sequence numbers and demo seek/reset handling.
3. Dynamic HUD widget registration and editor integration; cached bounded draw command buffers.
4. World-space lines/labels/markers with late-latched camera transform and batched rendering.

**Build note:** Added Cargo dependencies (`rhai`, `serde`, `toml`). If your workspace uses Cargo.lock, regenerate it using your existing build toolchain. This environment does not have Rust/Cargo or the omitted workspace/vendor directories, so the patch could not be compiled here.

# BlueWater build and run handoff

Updated: 2026-10-16.

## Repository and branch

- Working directory: `D:\Code\DinurdoJK-BlueWater`
- Branch: `experimental/blue-water`
- This is a Git worktree of the same repository as `D:\Code\DinurdoJK`.
- `D:\Code\DinurdoJK` is the separate `main` worktree. Building there does not build the experimental worktree's changes.
- Both branches currently point at commit `2b2343d`; the BlueWater implementation and this session's changes are uncommitted, with both staged and unstaged files. The working files in the experimental directory are the source of truth. Preserve them.

## Build

Close the game first so Windows permits replacing its executable. In PowerShell:

```powershell
Set-Location "D:\Code\DinurdoJK-BlueWater"
.\build.ps1 -Fast
```

`-Fast` is the optimized ThinLTO iteration build used during this session. It builds into `target\fast` and copies the successful executable into:

```text
D:\Code\DinurdoJK-BlueWater\target\release\DinurdoJK.exe
```

For the full release build and final performance measurements:

```powershell
Set-Location "D:\Code\DinurdoJK-BlueWater"
.\build.ps1
```

Both commands publish to the same executable path. `-Faster` is a separate development profile with a significant FPS penalty; performance comparisons should use `-Fast` consistently or full release consistently.

## Run with the user's current medium preset and tweaks

The active game profile the user edited is:

```text
D:\Code\DinurdoJK-BlueWater\target\blue-water-replay\perf22-user-view-restored-optics\base
```

It contains the user's `DinurdoJK.cfg` and linked game packages, including `costadelsol.pk3`. The executable does not need the map beside it when `--base` names this folder.

Launch directly, retaining that profile:

```powershell
Set-Location "D:\Code\DinurdoJK-BlueWater"
& ".\target\release\DinurdoJK.exe" --base "D:\Code\DinurdoJK-BlueWater\target\blue-water-replay\perf22-user-view-restored-optics\base" --map costadelsol
```

The replay launcher copies an old capture config over this profile each time it runs. The direct command above preserves the user's current tweaks. Loading `blue_water_optimized.cfg` also resets several wave and ocean controls; the user's current config is now the agreed testing baseline instead.

Console comparison, at the same stationary camera:

```text
r_oceanBackend 0
```

Godot; then:

```text
r_oceanBackend 1
```

BlueWater. Keep all other settings unchanged.

## Current state and remaining issues

- Last built experimental executable: ThinLTO `-Fast`, SHA-256 `ad27aed77ca0e9779fc5ca528aa2b715304fb69eb20bff676063cd025c85bf2e`.
- The added second Godot wave simulation was rejected by the user and reverted before any build. Follow-on short-wave sampling edits were also reverted. No additional simulation remains.
- The user's screenshots still show strong Godot seabed caustics and essentially no BlueWater caustics despite selecting the Godot option. That issue is unresolved. The current option uses the Godot approximation with BlueWater wave inputs; it does not reproduce Godot's wave field.
- Surface refraction now has an inexpensive screen-offset mode and a physical ray-trace mode. The caustics image connection and refraction changes are in the last built executable, but visual parity has not been established.
- The previous factory-config test ran near 40 FPS while the user's tuned profile was much faster. Those timings are not a reproduction of the user's current baseline. Use the current profile above going forward.
- An earlier snapshot of that medium-preset profile is `target\blue-water-baselines\medium-user-20261016.cfg` (SHA-256 `70a3d949917c56190c3715d9bac3d4bc074b33b22e90d787e18c4b67bad9db9f`). The live profile above may contain later user adjustments.
- No commit, merge or push was made.

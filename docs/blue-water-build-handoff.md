# BlueWater build and run handoff

Updated: 2026-10-16.

## Repository and branch

- Working directory going forward: `D:\Code\DinurdoJK`
- Live branch: `main`.
- BlueWater was committed on `experimental/blue-water` as `2c191d5` and merged with the live changes from `f3a2ce4`.
- `D:\Code\DinurdoJK-BlueWater` remains the separate experimental worktree for reference. Normal builds and development now use the live directory above.

## Build

Close the game first so Windows permits replacing its executable. In PowerShell:

```powershell
Set-Location "D:\Code\DinurdoJK"
.\build.ps1 -Fast
```

`-Fast` is the optimized ThinLTO iteration build used during this session. It builds into `target\fast` and copies the successful executable into:

```text
D:\Code\DinurdoJK\target\release\DinurdoJK.exe
```

For the full release build and final performance measurements:

```powershell
Set-Location "D:\Code\DinurdoJK"
.\build.ps1
```

Both commands publish to the same executable path. `-Faster` is a separate development profile with a significant FPS penalty; performance comparisons should use `-Fast` consistently or full release consistently.

## Run from the live directory

The normal live profile is:

```text
D:\Code\DinurdoJK\target\release\base
```

The merge preserves the live profile and game assets. Source integration does not replace its `DinurdoJK.cfg` with an experimental preset.

Launch directly, retaining that profile:

```powershell
Set-Location "D:\Code\DinurdoJK"
& ".\target\release\DinurdoJK.exe" --base "D:\Code\DinurdoJK\target\release\base" --map costadelsol
```

The earlier medium-preset testing profile remains at `D:\Code\DinurdoJK-BlueWater\target\blue-water-replay\perf22-user-view-restored-optics\base`; it was not copied over the live profile. The replay launcher resets its profile from an old capture when run. Use the ordinary live launch above for ongoing work.

Console comparison, at the same stationary camera:

```text
r_oceanBackend 0
```

Godot; then:

```text
r_oceanBackend 1
```

BlueWater. Keep all other settings unchanged.

## Historical experimental state before the live merge

- Last built experimental executable: ThinLTO `-Fast`, SHA-256 `ad27aed77ca0e9779fc5ca528aa2b715304fb69eb20bff676063cd025c85bf2e`.
- The added second Godot wave simulation was rejected by the user and reverted before any build. Follow-on short-wave sampling edits were also reverted. No additional simulation remains.
- The user's screenshots still show strong Godot seabed caustics and essentially no BlueWater caustics despite selecting the Godot option. That issue is unresolved. The current option uses the Godot approximation with BlueWater wave inputs; it does not reproduce Godot's wave field.
- Surface refraction now has an inexpensive screen-offset mode and a physical ray-trace mode. The caustics image connection and refraction changes are in the last built executable, but visual parity has not been established.
- The previous factory-config test ran near 40 FPS while the user's tuned profile was much faster. Those timings are not a reproduction of the user's current baseline. Use the current profile above going forward.
- An earlier snapshot of that medium-preset profile is `target\blue-water-baselines\medium-user-20261016.cfg` (SHA-256 `70a3d949917c56190c3715d9bac3d4bc074b33b22e90d787e18c4b67bad9db9f`). The live profile above may contain later user adjustments.
- The experimental notes above predate the live merge. The checkpoint commit is `2c191d5`; the merge does not add further ocean behavior changes. No push was made.

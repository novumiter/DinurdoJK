# Coastal water Phase 2.5: promoted-ocean evaluation

Phase 2.5 is a classification and visualization checkpoint. It does not run
the coastal solver and does not change the normal ocean rendering path when
its debug cvar is off.

## What “wet/dry bank” means

The land does not move. The waterline can move across a sloped beach, ramp,
shore, or low step: rising water wets terrain farther inland and falling water
exposes it. Such a boundary is stored as `Boundary::Bank`. A steep obstruction
(75 degrees or more from horizontal) is stored as `Boundary::Wall` and remains
no-through-flow regardless of depth. A future solver can overtop a bank when
the free surface exceeds its crest; it must never treat a rigid wall that way.

This replaces the earlier overloaded `Wall` classification, which also
included ordinary dry terrain. Cache version 3 additionally introduces the
shared exact `WaterFootprint` and sparse footprint-tile bake, so older dense
rectangle caches are rebaked rather than misread.

## Shared exact water footprints

Touching, coplanar faces with the same material remain one logical ocean and
one continuous FFT field. Connectivity is now confirmed from their actual
upward-facing triangles, not inferred solely from overlapping face AABBs. The
triangle union is the authoritative `WaterFootprint`; its overall AABB is only
derived broad-phase metadata.

The footprint is conservatively rasterized directly into 256-world-unit
sparse tiles (16x16 coastal cells at the default spacing). Bathymetry allocates
only those tiles and stitches matching faces at tile borders. An L-shaped body
therefore does not allocate the empty corner of its enclosing rectangle. The
262,144-cell safety budget now counts occupied tiles rather than the enclosing
rectangle.

The promoted renderer consumes the same footprint construction. It remains a
single camera-centred LOD clipmap rather than one draw per brush, but dry index
cells and PVS pieces are pruned, fragment boundaries retain exact triangle
clipping, and clipmap distance/centering uses the closest point on actual water
instead of the enclosing AABB. This preserves continuous waves across authored
brush seams without spending finest LOD in a dry concavity.

## Runtime inspection

Load a map with `r_ocean 1`, open the console, and set:

```text
r_coastalDebug 1
```

The values are:

| Value | View |
|---:|---|
| `0` | Off (default) |
| `1` | Exact promoted-water footprints tinted by selected treatment |
| `2` | Individual boundary faces touching supported wet cells |
| `3` | Bathymetry cells and depth regime |

Colors:

- green: wet/dry bank, eligible for shoreline/run-up work;
- blue: rigid vertical wall, eligible only for wall attenuation/reflection;
- yellow: a promoted ocean containing both;
- gray: FFT-only because no movable bank was found;
- magenta: unsupported or unresolved and therefore not solver-safe;
- cyan: exact footprint/outline boundary.

The console prints one assessment for every promoted ocean, including oceans
which did not produce a bathymetry grid. Each accepted line compares sparse
allocated cells with the old enclosing-envelope count, then reports bank,
wall, outline, and unresolved boundary length plus
shallow/transition/deep wet-cell counts. Rejected oceans retain and display
their exact triangle footprint rather than painting their entire AABB magenta.
Depth regimes use the promoted ocean's wavelength: shallow is `h/L <= 0.05`,
transition is `0.05 < h/L <= 0.5`, and deep is `h/L > 0.5`.

`coastal_debug` remains available for the full static bake summary and the
interactive HTML report path. The report now draws banks in green and rigid
walls in red. The in-game diagnostic geometry floats just above the promoted
wave envelope so it remains visible through normal depth testing.

## Classification policy

Convex BSP brushes use the actual plane which limits the horizontal segment at
the still-water elevation. Triangle geometry uses its face normal. Wall wins
when multiple blockers overlap. A dry cell without an intersected vertical
obstruction is a bank, not a wall. Unsupported segments remain magenta and
must be inactive/fallback boundaries later, but they do not disqualify the
supported parts of an otherwise usable pool.

This remains intentionally conservative. Moving inline models, unresolved
overhangs, missing beds, and oversized/rejected grids stay unsupported. The
overlay is generated only when the cvar is enabled and uploaded when the
background geometry job changes state; it adds no recurring coastal solver
work.

## Verification

- `cargo test -p jka-assets bathymetry --lib`: 12 passed.
- `cargo test -p DinurdoJK --bin DinurdoJK ocean::tests`: 16 passed.
- `cargo check -p DinurdoJK`: passed.

The new focused geometry test proves that a sloped waterline becomes a bank;
the existing thin-wall test continues to prove that a sub-cell vertical
barrier seals connectivity as a wall.

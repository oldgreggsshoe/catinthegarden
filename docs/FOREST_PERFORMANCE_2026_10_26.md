# Denser forests, less wasted GPU work — 26 October 2026

## Changes

- Compact accepted trees into three bounded GPU instance ranges, with one indexed
  indirect draw per existing mesh tier. Rejected density, biome, slope and road
  candidates no longer submit mesh vertices. Atomic counters reset whenever the
  cached generation is invalidated, including road revisions; no CPU readback
  or per-frame regeneration was added. First-instance remains zero; vertex
  buffer slices select each tier without a new GPU feature requirement.
- Double medium-tier candidates from 512 to 1,024 per cell (canonical stride
  24 to 12). This retains the original candidates and adds trees between them.
  Close/full candidates remain 4,096; sparse candidates remain 64. The terrain
  medium-population shading constant follows the new 1/12 canonical fraction.
- Reject fully off-screen tree bounding spheres in the vertex shader before
  procedural mesh construction and lighting. The deliberately generous bound
  includes crowns and a 2 m rounding margin. It uses the current camera each
  frame, not the cached generation camera, and retains spheres crossing the
  eye plane. Visible mesh, lighting, depth and fragment shading are unchanged.

This is not a forest mesh/art overhaul. The added density is medium-distance,
not a global doubling of close trees. Placement exclusions and road clearance
are unchanged. The unfinished elevated-lake work remains isolated in stash@{0};
the active planet bake is untouched.

## Measurement method

Quadro M1000M, release, High, X11, Immediate; requested viewport 1280x800,
actual captured PNGs 1280x647. One renderer at a time, no GPU timestamps.
Frame pacing excludes startup's first 120 frames and capture-adjacent frames.
Final repeated timings and run pointers are saved under
`test-runs/forest-speed-2026-10-26/` (`final-runs.tsv`). Its immutable
`before-app` contains compaction and increased medium density, but not the
additional per-tree visibility optimization. Thus the final before/after
screenshots compare identical density and quality.

Earlier stages at the fixed forest view:

| Stage | p50 frame time |
|---|---:|
| Original forest | 53.603 ms |
| Compaction alone | 51.373 ms |
| Compaction + doubled medium candidates | 51.961 ms |
| Same view, forests disabled | 48.937 ms |

Compaction-only startup captures `forest_startup/1793020611-930860` match the
original `1793013513-920496` pixel-for-pixel. The denser result
`1793029591-943718` was opened: midground stands are fuller, foreground
unchanged. Before the final repeated runs, the per-tree visibility test gave
51.935 -> 50.906 ms with both startup screenshots pixel-identical
(`1793042065-961810` -> `1793042161-962010`).

Rejected experiments: broader density masks put large crowns across the
camera; increasing full-tier candidates erased most/all savings. Skipping
empty terrain canopy samples and skipping already-degenerate conifer branch
vertices did not demonstrate an additional gain and were removed.

The approximately 49 ms forest-disabled frame is the dominant remaining cost;
these changes do not make this whole scene a 60 FPS scene. Timing conclusions
are specific to these measured views, not a promise across the planet.

## Regression checks

- Focused `cargo test -p planet-app forest -- --test-threads=1`: **39 passed,
  1 ignored**. Includes both shader validators, indirect command layout/counts,
  the updated canonical medium stride and conservative frustum side-plane
  sphere tests across narrow/wide FOVs, aspect ratios and behind-eye centres.
- Full `cargo test -p planet-app -- --test-threads=1`: **781 passed, 6 failed,
  71 ignored**. The same six previously recorded failures remain: three optical
  zoom tests, the scenario FOV-range expectation, the shallow-water shader
  string assertion and the flooded/sinking HUD counter. No new failure.
- `cargo build --release -p planet-app` passes (existing dead-code warning).
- `git diff --check` passes.

## Final repeated timings and image comparison

| Replay | Before visibility guard | Final | Saving |
|---|---:|---:|---:|
| Forest performance, pair 1 | 51.975 ms | 50.905 ms | 1.070 ms |
| Forest performance, reversed order pair 2 | 52.014 ms | 50.894 ms | 1.120 ms |
| Boundary movement | 62.294 ms | 61.839 ms | 0.455 ms |
| Night | 52.116 ms | 51.010 ms | 1.106 ms |
| Startup | 51.935 ms | 50.887 ms | 1.048 ms |

Both capture-free performance pairs agree: approximately **1.1 ms extra saved**,
with the same increased density. Against the earlier original 53.603 ms,
the final ~50.9 ms is approximately **2.7 ms saved overall** (~18.7 -> 19.6 FPS).

All nine final replay runs pass their scenario assertions. Seven paired PNGs
(two startup, four boundary-movement, one night) are pixel-identical, and all
seven final images were opened. The movement sequence retains the same trees
at the screen edges; this is sampled replay evidence, not exhaustive live
rotation/flicker validation. The night image is nearly black except stars, so
it proves output equality but is weak evidence for detailed tree appearance.
Exact paths and comparisons are in `final-runs.tsv` and
`final-image-comparison.txt` in the evidence directory. Final release built;
no test renderer left running.

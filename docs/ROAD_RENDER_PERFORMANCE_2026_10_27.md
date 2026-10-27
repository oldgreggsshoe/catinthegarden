# Road rendering performance — 27 October 2026

## Scope and measurement

Road geometry, routing, cuts, collision, material textures, lighting, draw
ranges and subpixel road coverage are unchanged. This pass targets wasted GPU
work in the existing terrain/road excavation masks, not a quality reduction.

Measured on Quadro M1000M/Vulkan, release, High, X11, Immediate, normal road
presentation synchronisation enabled. Requested viewport 1280x800; captured
images 1280x647 after window sizing. No GPU timestamps. Frame-pacing p50 excludes
startup's first 120 frames and capture-adjacent frames. One renderer at a time;
CPU tests/builds do not overlap final benchmark measurements.

Evidence is in `test-runs/road-speed-2026-10-27/`; immutable `before-app` is
commit b75e464. Both versions use `PLANET_ROAD_EXPERIMENT=1` and
`PLANET_ROAD_SIGNS_WAIT=1`; coastal/tunnel scenes use
`PLANET_ROAD_NETWORK_SITE=longest_undersea`. Mountain scenes use `mountain`
(the existing default mountain site), **not** the unrelated undersea start.
The lake stash and active outmap are untouched.

## Attribution and retained implementation

Initial stationary undersea entrance median: **56.048 ms**. Hiding only road
meshes gives 55.002 ms; disabling excavation gives 40.666 ms. These are diagnostic
ablations, not deliverable improvements: hiding cuts closes the portal and
changes the geometry/visible workload. They identify excavation as the more
promising target than reducing deck mesh quality.

- Rendering asks whether a point is inside a bore, not for the full union's
  maximum signed distance. New boolean containment stops on the first covering
  segment; a conservative padded XY box skips impossible candidates before
  expensive distance/roof arithmetic. Surviving candidates use the original
  plane calculations and exact 0.0/0.55 thresholds. The original signed-distance
  query is retained for callers needing its magnitude and for regression.
- Terrain bore and deck/court masks now share the same local projection and
  spatial hash lookup. The original cosine, no-bore, mesh-window, per-cell type
  and road-enable gates remain; outer gates precede the hash lookup. The open
  cut ownership predicate is shared, not replaced by an approximate mask.
- Shader regression for bathymetry ordering now checks the combined mask remains
  before seabed shading. No frame scheduling, GPU readback or new render pass.

Intermediate entrance trials: boolean mask 55.443 ms; shared lookup 54.943 ms;
original 56.048 ms. Both inspected comparisons are pixel-identical. A first-pass
normal bypass measured 55.060 ms and extra per-plane early exits 54.905 ms: neither
showed an additional meaningful saving, so both trials were removed. The
unused intermediate wrapper was also removed.

## Tests

The new Vulkan regression evaluates 16,500 point/threshold cases around five
rotated 25%-grade bores, crossing junctions, portal end caps, wall/roof/floor
boundaries and mixed open-road/court cells, with enabled/disabled mesh windows
(33,000 cases total). It compares the boolean mask to the original full union
and the shared terrain predicate to the original pair of predicates. The first
run failed because the new helper did not yet exist; the implementation passes.
The existing exact excavation regression still compares independent fast and
reference ownership helpers, including height/normal bit patterns and flags.

## Final timings and visual validation

| Replay, p50 frame time | Before | Final | Saving |
|---|---:|---:|---:|
| Undersea entrance, initial pair | 56.048 ms | 54.881 ms | 1.167 ms |
| Undersea entrance, repeated pair | 55.824 ms | 54.840 ms | 0.984 ms |
| Coastal portal movement/look-around | 40.554 ms | 40.140 ms | 0.414 ms |
| Deep tunnel | 3.248 ms | 3.241 ms | effectively unchanged |
| Mountain road walking | 76.830 ms | 75.538 ms | 1.292 ms |
| Mountain village parking court | 88.397 ms | 86.926 ms | 1.471 ms |

The entrance gain repeats at approximately 1 ms; other scenes have one paired
run each, not statistical significance claims for tiny differences. These
are whole-frame measurements, not isolated GPU timer durations. Larger road
costs remain, especially outdoors; this is not an all-planet FPS guarantee.

All 10 final replay runs pass their assertions. Nine paired PNGs are
pixel-identical and all nine final images were opened: entrance 1, coastal
movement 3, deep tunnel 1, mountain walking 3, parking 1. The real roads, junctions,
signs and court are present in the mountain captures. The coastal sequence
covers outside-to-inside and looking back through the portal; the last view
is a close-up of the lining. Existing terrain bank faceting remains unchanged.
This is sampled movement evidence, not exhaustive live driving acceptance.

Exact run pointers, logs and pacing reports are in `final-runs.tsv` and the
associated files; `final-image-comparison.txt` records all 9 comparisons.

- Focused road suite: **114 passed, 26 ignored**.
- Explicit production Vulkan suite: **10 passed**, including bore/deck/court
  masks, height/normal parity, forest exclusions, far-road coverage and glow.
- Release build passes with the existing unused-method warning.
- `git diff --check` passes.
- Full app suite: **781 passed, 6 failed, 72 ignored**. The same six recorded
  failures remain: three optical-zoom tests, scenario FOV-range expectation,
  shallow-water shader-string assertion and flooded/sinking HUD counter.
  No new failure; the extra ignored test is the explicitly executed Vulkan
  mask regression. Full/focused/GPU/build logs are saved with the evidence.

Final release is ready; no renderer left running. Larger remaining excavation
costs need a separate measured pass rather than silently reducing terrain or
road quality. No changes to the lake stash, bake, driving or presentation.

# Exact-output performance pass — 25 October 2026

## Scope and method

Four bounded changes remain. The before binary is an immutable build of `7493306`.
The verified replay binary has the four fixes; the delivery binary was rebuilt
after a final non-rendering completion-channel correction to the pacing worker.
Hashes, release build logs, complete pacing reports and replay run pointers are in
`test-runs/perf-audit-2026-10-25/`. All GPU measurements used the same Quadro
M1000M, X11 (`env -u WAYLAND_DISPLAY`), Immediate present, High quality and
1280x800 internal viewport, with one renderer at a time. Frame-pacing reports
exclude the first 120 frames and captures plus their following frames. GPU
timestamp profiling was not used on this driver. The table is one paired run
per scene, not a promise for every location or a statistically significant
claim about sub-millisecond differences.

| Fixed replay, p50 frame ms | Before | Verified | Result |
|---|---:|---:|---|
| Forest performance | 55.766 | 53.616 | -2.150 ms, ~4% more FPS |
| Forest startup | 55.806 | 53.610 | -2.196 ms |
| Forest boundary transition | 65.688 | 63.775 | -1.913 ms |
| Forest night | 55.751 | 53.600 | -2.151 ms |
| Undersea entrance | 59.525 | 58.150 | -1.375 ms |
| Road walking | 57.827 | 56.029 | -1.798 ms |
| Coastal portal travel | 43.923 | 42.891 | -1.032 ms; mixed indoor/outdoor replay |
| Deep tunnel | 3.261 | 3.242 | effectively unchanged |

The isolated forest-only entrance build measured 58.224 ms. A trial that also
skipped overwritten excavated-vertex normals measured 58.156 ms: the ~0.07 ms
increment was indistinguishable from noise, so **that shader trial was removed**.
Likewise, gating the HUD's wave-range statistic showed no meaningful frame-time
gain and was removed. The retained forest change accounts for essentially all
of the measured entrance improvement.

## Retained fixes

- **Rejected forest instances:** GPU generation writes a zero-height record for
  rejected density/biome/road candidates, but the draw still submits every
  indexed candidate. `forest.wgsl::vs_main` now sends zero-height instances
  off-clip before procedural mesh construction, shadow, fog and irradiance
  work. Valid trees take the original path unchanged. This is the measured
  forest/entrance rendering gain; no tree count, geometry, density or lighting
  quality was reduced.
- **Lazy terrain-height fallback:** `TerrainRenderer::raster_surface_height_meters_at`
  asks the rendered triangle first, and computes the continuous sampled field
  only on a mesh miss. Previously it computed both and discarded the sample on
  every hit. Normal driving can call it for multiple wheels twice per 120 Hz
  step, plus camera contact queries. The isolated entrance CPU simulation
  sample median fell from 9.80 to 9.12 ms, while total frame time there stayed
  GPU-bound. Returned mesh and fallback values follow the original arithmetic.
- **Lazy FFT ship orbital query:** anchored afloat ships now use their held
  water-label velocity directly. The expensive local FFT horizontal query runs
  only for a broken anchor or a sinking/seabed hull, where the old result was
  actually used. In the FFT ship replay, isolated sampled CPU simulation time
  was 9.40 -> 7.20 ms; isolated whole-frame medians 33.829 -> 33.896 ms were
  indistinguishable, so this is CPU headroom, **not** an FPS-gain claim.
- **Asynchronous frame-pacing summary:** every 600 samples the render thread
  used to clone, sort and write its entire growing frame history. One ordered
  worker now does the sort/write; exit waits for its final flush. Synthetic
  record-call cost at 600,000 samples was 20.078 -> 1.487 ms, and at 1,048,200
  samples 29.006 -> 1.714 ms. Full old/new final summaries were byte-identical.
  This prevents a periodic long-session render-thread hitch, rather than
  increasing steady-state GPU throughput.

## Output and tests

`verified-image-diffs.txt` records **22/22 pixel-identical before/verified PNGs**
across forest startup, boundary travel, night, undersea entrance, deep tunnel,
coastal portal travel, road walking, FFT ship float and FFT hybrid-close. The
forest travel, portal, walk and ship captures were also opened and inspected.
The final delivery binary recaptured forest startup (two images) and the
undersea entrance (one image); all three also matched the original exactly.
The paired replays passed their authored scenario assertions. The final full
app suite had 779 passes, the same six pre-existing zoom/FOV/shallow-water/HUD
failures, and 71 ignored. All nine explicit Vulkan road checks, 28 focused
forest checks and the opt-in FFT broken-anchor fallback test pass. The release
build succeeds. No texture, mesh, weather, exposure or presentation policy
changed.

## Remaining opportunities, not implemented

Some tree lighting is still evaluated per indexed vertex even though it depends
only on the tree centre, but moving it to a per-frame compute buffer would add
synchronization and an extra pass; it needs separate GPU attribution and exact
lighting-buffer comparison. Excavation fragment queries and FFT CPU lattice
sampling have similar larger redesign options. Neither was changed without a
measured, quality-preserving implementation.

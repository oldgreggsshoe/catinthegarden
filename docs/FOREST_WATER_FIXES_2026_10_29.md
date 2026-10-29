# Forest continuity, lake motion and night-beach check — 29 October

## Forest: reproduced global flashing, not a density problem

`forest_regeneration` holds a forest view, then flies at 80 m/s, capturing 24
consecutive 30 Hz frames. Two unmodified-renderer runs independently lose the
**entire** forest in captures 10 and 18:

- `test-runs/forest_regeneration/1793308711-124856`
- `test-runs/forest_regeneration/1793308726-124988`

Both missing frames are pixel-identical to the corresponding `PLANET_FOREST=0`
control (`1793308742-125120`). This separates missing geometry from lighting,
individual tree LOD or changing forest ownership. The ordinary CPU forest counts
remain populated, which is why their 0.5 s log samples did not expose the defect.

### Cause and minimal fix

`State` encoded `ForestRenderer::encode_gpu_generation` **before** calling this
frame's `update_patch`. That update can change batch layouts and clear every
indirect instance count (camera travel over 25 m, cell/tier/source/road changes).
If the old dirty flag was false, no generator was encoded, leaving all counts at
zero for the draw. If it was true, the old encoded dispatch could instead read
newly uploaded descriptors with old batch/source bindings. wgpu queue writes are
flushed before the submission, not at the point in Rust where encoding happened.

Generation now runs after the frame's uploads and before any scene draws. Nothing
about tree density, range, meshes, culling or shaders was reduced. A debug invariant
requires generation before a nonempty resident draw; empty batches need no dispatch.
A source-order regression at the actual frame call site failed before the fix and
passes after it. It is an ordering guard, not a substitute for the GPU captures.

### GPU verification

- Two fixed replays (`1793308854-125904`, `1793308868-126055`) retain trees in
  **48/48 consecutive frames**. The land-ROI dark-pixel count never falls to zero
  (minimum 18,798), versus zero twice in each baseline run. Opened the actual
  disappearing baseline frame and both corresponding corrected frames.
- The final assertion-enabled run is `1793309294-129787`: all 24 captures and
  the lower-half luminance bands pass. The bands distinguish the lost-forest
  frames (0.468–0.469) from this fixed view with trees (0.371–0.427).
- Both matched stationary `forest_startup` PNGs are **pixel-identical**:
  `1793308884-126181` before / `1793308903-126354` after. This is a continuity
  repair, not a change to the established steady forest appearance.
  Final release (including the lake shader), `1793309609-132108`, also has both
  PNGs pixel-identical to the original; final view opened.

## Lakes: the ripple phase really was static

The three existing 29/13/7 m ripple trains depended only on position. The held
shore camera in new `lake_motion` proves it: from 1 s to 4 s, **0 of 222,460**
water-ROI pixels change (`test-runs/lake_motion/1793309041-127168`).

Each train now advances at its deep-water gravity-wave frequency,
`sqrt(g / (2*pi*wavelength))`, using `camera.projection.z`, the existing running
water/presentation clock. It continues with planet rotation paused. Spatial and
temporal phases are reduced separately before subtraction so large planetary
coordinates cannot quantise away a small time step. Frequencies constant-fold;
there are no new resources, FFT dispatches, mesh waves, shoreline/height changes
or changes to the sea. This animates the existing normal-map-scale ripples; it
is not a lake wave-physics simulation.

Final `lake_motion/1793309314-129942` passes and the same held water ROI changes
**213,160 of 222,460 pixels** between 1 s and 4 s (mean absolute RGBA difference
2.733 display levels). The images were opened: wave bands/reflections move while
the view, sun and shoreline stay put. The dedicated water-clock regression fails
on the old shader; it and full composed WGSL validation pass on the fix.

## Beaches: moonlight, not emissive dry sand at the tested coast

No beach material/lighting was changed. A fixed-exposure, deep-night coast replay
has black dry sand with `PLANET_MOON=0` (`beach_night/1793309051-127213`), whereas
the same held view is naturally visible with `PLANET_MOON=1`
(`1793309060-127260`). Both were opened. The centre luminance (a patch crossing
sand/water) changes from 0.00176 to 0.14497; most of the small moonless remainder
is blue water, not dry beach. The material multiplies the same sun, sky and moon
irradiance as adjacent terrain; it has no sand emission term.

New `beach_night` has a moonless centre-luminance bound of 0..0.006 and passes
with `PLANET_MOON=0` (`1793309326-130047`). Its default scenario moon is off;
turning the moon on intentionally invalidates that darkness expectation.
This checks one real coast, not every beach or every interactive exposure state.
Legitimate moonlight/twilight was deliberately not blackened to hide visibility.

## Tests / reproducibility

- `cargo check -p planet-app`: passes.
- Focused forest filter: **40 pass, 1 ignored**.
- Water-clock regression and composed planet shader validation: pass.
- Full app suite: **824 pass, 78 ignored, same six existing failures** (three
  optical-zoom expectations, out-of-range scenario FOV, shallow-water shader
  weighting assertion and flooded-ship HUD counter). No new failing test.
- Release builds. New scenario registry size is 163.
- Evidence, immutable before/after binaries, failed-before test logs and run lists
  are in `test-runs/forest-flash-2026-10-29/` (ignored local artifacts).

For visual replays use `env -u WAYLAND_DISPLAY PLANET_ROAD_EXPERIMENT=0
PLANET_ROAD_NETWORK=0 PLANET_PRESENT_MODE=immediate PLANET_QUALITY=high` with
`--scenario forest_regeneration`, `lake_motion` or `beach_night`.
Use `PLANET_MOON=0` for the beach darkness assertion. Forest flight was captured
at 640x360; lake and beach at requested 1280x800 (actual client 1280x647).

## Matched steady performance

High/X11/Immediate, requested1280x800/actual1280x647, one renderer and no Cargo
jobs during the four clean runs, same stationary `forest_performance` scenario,
order before/after/after/before. Every run passes, with the same44 cells and
19,328 CPU-side candidate instances. Per-frame pacing drops the first120 frames.

| Run | p50 ms | mean ms |
|---|---:|---:|
| clean-perf-before1 | 62.307 | 62.296 |
| clean-perf-after1 | 60.128 | 60.122 |
| clean-perf-after2 | 60.167 | 60.171 |
| clean-perf-before2 | 62.316 | 62.296 |

No steady-view slowdown was observed. Do not attribute the ~2ms difference solely
to moving a cached dispatch: this compares the complete before/final binaries,
including the lake shader edit, and is not a general FPS-gain claim. No forest
quality/density reduction was made. Earlier perf entries in `after-runs.json`
overlapped a Cargo build and are deliberately discarded; only the clean entries
in `final-runs.json` support this table. Raw summaries are `clean-perf-*.pacing`.


## Prevention / limits

The dependency is **update/upload -> generate -> draw**, not simply "there is a
compute pass somewhere earlier". The frame-call-site regression, debug invariant
and dense temporal scenario guard it without introducing a frame-graph rewrite.
This does not claim that every individual forest LOD transition is perfectly
invisible; it fixes the demonstrated whole-forest disappearances. Lake animation
has image/time evidence; its incremental GPU cost has not been separately measured.

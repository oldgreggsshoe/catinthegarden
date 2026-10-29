# Timber villages and pedestrian desire lines — 29 October 2026

## Scope

Village sites, house scatter/seeds, spacing, road alignments, court dimensions,
terrain geometry and presentation policy are unchanged. Buildings now use the
existing mipmapped/anisotropic oak material with painted vertical siding, dark
wood shingles, framed non-emissive glazing, doors, projecting sills, corner
joinery, a little entrance porch and stone footings. The four established paint
colours remain. Close houses have solid joinery; beyond 450m the same textured
shell avoids submitting subpixel trim. No new asset or dependency.

### An existing placement defect exposed by the paths

The original `village_pov` nearest house stood at +5087.748m; the drawn triangle
under its direction was +4620.069m. Its apparent contact shadow floated with it.
The previous continuous forest sampler was not the drawn terrain surface.

Siting still reads the same camera-independent dense geography. Visible contact
now reads actual raster triangles, using the **real eye position** for the
shader's distance filter, rather than a hypothetical eye over the house. At most
32 nearby buildings plus a rotating batch of 16 refresh between visible-set
rebuilds. The existing walking collision query delegates with its original
radial-eye argument: its behaviour is unchanged. Deeper footings cover small
local slopes; there is no new village-wide excavation or building collision.

## Paths

`village_paths.rs` owns one cancellable background worker with a replaceable
request slot and generation-checked publication. Requests copy the small dense
siting-tile cache and already accepted house positions, not the terrain renderer.
No route search or source I/O runs in a movement frame.

* Routes start at actual porch/door positions and avoid expanded oriented house
  footprints. Rotated doors snap to a reachable grid node, not through a wall.
* A bounded 2m grid (385 squared) uses eight-neighbour A*, disallowing diagonal
  building-corner cuts. Stable 8m-filtered relief prices hills and valleys;
  severe slopes and water are excluded. This is a footpath heuristic, not NPC
  movement simulation or a guarantee that every procedural site is walkable.
* Outlying houses establish lanes first. Existing footfall lowers subsequent
  route cost (0.55 versus 1), encouraging sensible shared trunks instead of a
  star of independent lines. Shared traffic widens half-widths from roughly
  0.65m to at most 1.4m. Collinear grid steps coalesce before GPU indexing.
* Parking destinations use the **engineered terminal/profile**, not the nearest
  settlement. The village-facing court half is surveyed for a gentler entry
  rather than blindly painting up the steepest cut bank. An exact final
  connector reaches inside the court edge, independent of grid rounding.
* Both near-indexed and farther drawn courts retain metadata. Before a court
  is resident, or with roads disabled, doors share a small internal gathering
  point; the path worker rebuilds when the road set publishes.

The shader paints muted, softly edged/mottled compacted earth **in the terrain
material**, before rain/snow and ordinary illumination. It adds no floating
ribbon, depth layer, light/glow or draw call. CPU and GPU use a local frame with
camera-relative reconstruction and height normalization, avoiding planet-sized
float coordinates. Existing GPU tree generation excludes the paths.

The bounded sparse mask has 8192 buckets, at most 32 probes/128 local candidates,
16384 capsules and 131072 references. A 16KB coarse occupancy mask cheaply
rejects empty ground. Overflow rejects a whole publication and logs it, rather
than silently dropping individual paths. `PLANET_VILLAGE_PATHS=0` is the A/B
mask/clearance override; normal play defaults on.

## Road-precompute compatibility

The engineering source and precompute fingerprint inputs were **not changed**.
Court metadata is attached in `road_network_stream.rs` to a CPU-only field member,
using the published profile's existing terminal/court construction. No GPU cut
buffer layout, route/profile/cache payload or source fingerprint change is needed.
The expensive planetary precompute therefore remains reusable. The lake WIP
stash and baked outmap were not touched.

## Validation and evidence

Evidence directory: `test-runs/village-upgrade-2026-10-29/`.
`before-app` pins baseline `a1874b4`. All captures use Vulkan/X11, High,
1280x800 requested (actual client 1280x647), Immediate, no concurrent renderer.
Changed visual output is intentional; this is not an exact-output optimisation.

* Focused villages/routing/material checks: 33 pass, 8 diagnostic tests ignored.
* Both new Vulkan tests pass: 6887 capsule samples across positive/negative
  spatial cells, plus elevated-ground/camera-motion reconstruction at 0/5/18km.
* Final regular full suite: **800 pass, same six pre-existing failures,
  76 ignored** (`validation/full-suite.log` in the evidence directory). Focused tests and the two
  path Vulkan checks were rerun after the final worker/body-context changes.
* An explicit broader road Vulkan run has 11 passes and one failure in the old
  `gpu_far_road_retains_subpixel_coverage_without_black_specks` expectation
  (measured ~0.192 versus expected ~0.088). Its existing test body, road shader,
  material and pipeline are unchanged by this work. It is not hidden or retuned
  as part of a village feature. The two new path Vulkan checks pass separately.
* Captured routing probes connect **778/778 drawn doorways in 40 villages**.
  Publication logs also compare engineered court metadata against the actual
  near excavation-index courts. This is a test-area result, not an exhaustive
  global village/terrain survey.
* New `village_paths` replay supplies near/overview/return views, finite-metric
  and four-capture assertions. Existing `village_pov` and
  `road_network_parking` remain unchanged and were replayed before/after.

### Matched frame pacing (milliseconds)

First 120 frames, captures and following frames excluded; p50, not a 60FPS claim.

| Existing replay | Baseline | New buildings/grounding/paths | Difference |
|---|---:|---:|---:|
| village_pov | 68.557 | 71.456 | +2.899 |
| road_network_parking, village site | 96.341 | 99.838 | +3.497 |

The unoptimised prototype was ~75ms at village_pov. Bounded background routing,
distant shell draws and mask rejection reduced its cost. The final quality
upgrade still costs roughly 3ms; it is not performance-neutral. See `final-runs.tsv`,
`shipping-runs.tsv` and `cache-safe-runs.tsv` for captures and individual pacing
files; `verified-runs.tsv` and `verified-*-pacing.txt` are the final matched
measurements after the cache-safe metadata change. That change does not alter
road geometry or invalidate the precompute.

Replay command:

```sh
env -u WAYLAND_DISPLAY PLANET_ROAD_EXPERIMENT=1 \
  PLANET_ROAD_NETWORK_SITE=village PLANET_PRESENT_MODE=immediate \
  PLANET_VIEWPORT=1280x800 cargo run --release -p planet-app -- \
  --scenario village_paths --profile-render
```

Not claimed: photorealistic terrain, full house interiors/collision, physical
footfall erosion, stairs at every possible procedural cliff, or exhaustive
planet-wide path feasibility. No grass/forest/road/terrain-quality reduction
was used to pay for this village upgrade.

Opened final captures:

* Baseline floating houses: `test-runs/village_pov/1793271770-977795/screenshots/`.
* Same old camera, correctly grounded houses:
  `test-runs/village_pov/1793273466-983386/screenshots/`.
* Ground-height close views, overhead path merging and return view:
  **`test-runs/village_paths/1793272980-980918/screenshots/`** (all four opened).
* Final court entry:
  `test-runs/road_network_parking/1793273489-983459/screenshots/capture-001.png`.

The old camera was deliberately not moved for matched timings. The new village
replay is ~468m lower to inspect the buildings at their real ground position.
Court metadata verified 31/31 near-indexed courts, with 39 near/far destinations
in the captured resident plans. All 778/778 doors connected to the available
parking/internal network. No controller-driving changes or live walk acceptance
are claimed by these camera replays.

## Path ground shaping (29 October, later)

Ground under the paths is smoothed and ramped to each court's floor at the path
mouth (bounded, faded out 200-400 m away), on the GPU mesh and the CPU walking
height alike. `PLANET_VILLAGE_PATH_SHAPING=0` turns it off for A/B. Measured
+1.9/+2.2 ms in the `village_paths` replay (two interleaved pairs, command
above plus the variable); see HANDOFF for the breakdown.

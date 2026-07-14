# Cat in the Garden handoff

> **KEEP THIS DOCUMENT UP TO DATE AT ALL TIMES!**

This is the canonical read-first handoff for the repository. Read `AGENTS.md`
first, then read this file before changing code. Any change to behavior,
architecture, controls, commands, generated-data assumptions, validation,
known risks, or the next action must update this file in the same commit.
Never record a test or scenario as passing unless it was actually run.
Do not remove or weaken the maintenance requirement above.

## Handoff metadata

- Repository: `/home/dad/catinthegarden`
- Branch: `experiment/composition-debug`
- Remote branch: `origin/main`
- Implementation baseline reviewed: current working tree based on `f993fd8`
  (`Add canonical project handoff`)
- Last full source review: 2026-07-13
- Current phase status: Phases 0 through 6, including 5.5, are complete.
- Remaining planned phase: Phase 7, polish and final regression.
- Current bounded engineering issue: none. The orbital optical-zoom LOD jump
  described by the previous handoff is resolved and covered by a deterministic
  L2-through-L18 round-trip regression. Extreme-zoom non-cardinal camera
  precision is also resolved by CPU f64 view-space rebasing and camera-local
  atmosphere/sun rays.

The change containing this document adds the optical-zoom implementation,
regression scenario, and matching documentation on top of the baseline above.
Always use `git log -1 --oneline` and `git status --short` rather than assuming
this snapshot is still HEAD.

## Mandatory resume procedure

1. Read `AGENTS.md` and this entire document.
2. Run `git status --short` before editing.
3. Preserve pre-existing user changes and captures; do not stage, delete, or
   overwrite them unless the user explicitly asks.
4. Run `git log -5 --oneline --decorate` to catch work newer than this handoff.
5. Confirm whether `assets/outmaps/test-planet/manifest.json` exists. The app
   silently uses placeholder terrain if the default outmap is absent.
6. Read only the modules named by the current task before expanding scope.
7. Run `cargo check --workspace` after meaningful Rust changes.
8. Update this file and the phase summary in `AGENTS.md` when relevant.
9. Commit and push the focused change after completing each user prompt, as
   required by `AGENTS.md`.

## Executive summary

Cat in the Garden is a Rust `wgpu`/`winit`/`egui` planet renderer. It currently
renders a rotating 8,000 km-diameter procedural planet with:

- f64 planet-centered world/camera math and f32 camera-relative GPU data;
- a six-face cube-sphere quadtree with screen-space-error LOD and skirts;
- 0.5-second dithered parent/child LOD transitions;
- offline-baked, streamed height/biome/moisture outmap tiles;
- height-derived terrain normals and Earth-like biome/ocean colors;
- analytic Rayleigh/Mie atmosphere and aerial perspective without LUTs;
- a visual HDR sun disc/corona, luminance mip chain, auto-exposure, and ACES;
- a six-wave spherical Gerstner ocean with daylight-gated reflection;
- deterministic scenarios, JSONL logging, PNG capture, assertions, and
  opt-in CPU/GPU render profiling.

The planned `render` and `planet` crates were not split out. Their functionality
is currently organized as modules inside `crates/app`. Do not create new crates
merely to match an old diagram; refactor only when a task calls for it and tests
pin behavior first.

## Non-negotiable implementation constraints

These constraints come from `AGENTS.md` and current code. Preserve them.

### Precision

- Absolute world positions remain `glam::DVec3`/f64 on the CPU.
- Planet radius, camera orbit, quadtree anchors, and bounds are f64.
- Never upload an absolute world-space f32 position.
- Chunk vertices are anchor-local f32 values. Each frame, subtract the f64
  camera from the f64 chunk anchor, then cast the small relative vector to f32.
- The GPU treats the camera as the origin.

### Projection and depth

- Reversed-Z infinite-far projection is mandatory.
- Depth clears to `0.0`; terrain compares with `Greater` and writes depth.
- Near clip is `(altitude * 0.01).clamp(0.05, 10.0)` metres.

### Terrain generation and streaming

- The normal outmap path never generates macro geography, erosion, hydrology,
  climate, or biome data at runtime; the baker owns that work. Phase 7 adds a
  deliberately bounded four-octave direction field only as land microrelief
  over the baked height, so sparse ancestor fallback has useful detail at any
  longitude without changing the baked coastline or biome.
- `--terrain placeholder` remains a Phase-2 diagnostic fallback and evaluates
  its analytic multiscale sine height at runtime.
- Runtime normals are central differences from height. Do not add a baked
  normal channel without revisiting the format decision.
- Missing tiles resolve to the nearest available parent with UV remapping.
- Skirts remain the crack-hiding fallback at mixed LOD boundaries.

### Atmosphere, sun, and HDR

- Atmosphere remains analytic and LUT-free.
- Terrain aerial perspective and sky/space raymarch must use matching constants.
- `atmosphere.wgsl` and `planet.wgsl` duplicate atmosphere constants; update
  both together.
- The sun disc is visual-only. Its exaggerated apparent brightness must not
  alter terrain, ocean, or atmospheric illumination.
- HDR scene rendering remains `Rgba16Float`, followed by luminance reduction,
  smoothed exposure, ACES tonemapping, then egui.

### Source-control safety

- Do not delete files unless explicitly requested.
- Do not stage unrelated changes.
- Generated `test-runs/` and `assets/outmaps/` remain ignored.
- Commit and push the focused changes made for every completed user prompt.

## Actual workspace layout

```text
Cargo.toml                         workspace manifest
AGENTS.md                          architecture rules and phase list
docs/HANDOFF.md                    this canonical live handoff
assets/README.md                   generated-outmap usage
assets/outmaps/test-planet/        generated default outmap; ignored
crates/
  app/                             executable and all runtime/render modules
    scenarios/*.json               embedded deterministic scenarios
    src/main.rs                     event loop and frame orchestration
    src/planet.rs                   camera, cube-sphere, quadtree, mesh math
    src/terrain.rs                  chunk GPU data and outmap streaming
    src/outmap.rs                   runtime manifest/tile reader
    src/planet.wgsl                 terrain/ocean/material/aerial shader
    src/atmosphere.rs/.wgsl         fullscreen atmospheric raymarch
    src/sun.rs/.wgsl                visual sun disc and corona
    src/hdr.rs/.wgsl                HDR, luminance, exposure, ACES
    src/ocean.rs                    CPU mirror for wave assertions
    src/scenario.rs                 scenario parser and fixed-step runner
    src/debug.rs                    artifacts, logs, screenshots, assertions
  baker/                            offline terrain generator and exporter
  coretypes/                        shared outmap schema, tile and biome types
test-runs/                          generated run artifacts; ignored
```

## Runtime architecture

### `crates/app/src/main.rs`

`State` owns the window surface, device/queue, depth and HDR targets, all
renderers, camera, shared uniform, scenario runner, debug artifacts, egui, and
profilers. `State::render` is the main integration seam.

Important constants:

- `DEFAULT_OUTMAP_PATH = "assets/outmaps/test-planet"`
- default auto-orbit speed: `0.4 rad/s` in a 28.5-degree inclined plane
- mouse-look sensitivity: `0.0006 rad/pixel`
- visible HUD refresh: 100 ms
- hidden HUD refresh bookkeeping: 500 ms
- GPU timestamp readback ring: 3 slots
- presentation: FIFO, desired maximum latency 2

Per-frame flow:

1. Collect completed GPU timestamps and luminance readbacks.
2. Advance fixed scenario time or interactive elapsed time.
3. Apply scenario pose/FOV/sun or automatic inclined-plane orbit.
4. Compute the planet's 600-second axial rotation.
5. Transform camera position/direction into the rotating planet-local frame.
6. Update smoothed exposure using the previous luminance result.
7. Update quadtree selection, chunk transitions, tile cache, draw items, and
   upload the per-frame terrain instances.
8. Log exposure every rendered frame and spatial state about every 0.5 s.
9. Refresh cached egui geometry only when the HUD is due.
10. Acquire the swapchain and upload the camera uniform.
11. Render atmosphere, sun, and terrain/ocean into the HDR/depth scene.
12. Build the luminance mip chain and schedule its readback.
13. ACES-tonemap HDR to the swapchain.
14. Render egui after tonemapping.
15. Optionally copy the swapchain texture for a PNG screenshot.
16. Submit, map asynchronous buffers, present, and finalize captures/scenarios.

Render order:

```text
Rgba16Float HDR scene + reversed-Z Depth32Float
  atmosphere fullscreen triangle: replace background, no depth write
  sun fullscreen triangle: additive, no depth write
  terrain/ocean indexed chunks: depth compare Greater, depth write
  luminance extraction and mip downsample
  ACES tone map to swapchain
  egui directly to swapchain
  optional post-tonemap/post-egui screenshot readback
```

### Camera and input

`planet::OrbitCamera` stores an f64 orbit and an optical FOV:

- default center radius: 10,000,000 m, therefore 6,000,000 m altitude;
- default orbit begins at its equatorial ascending node; interactive auto-orbit
  has a 28.5-degree inclination;
- FOV default: 45 degrees; maximum: 75 degrees; the 640px reference minimum
  is 0.00005 degrees and scales with viewport height as described below;
- mouse look is a yaw/pitch offset in the local frame whose forward/down vector
  always points from the camera toward the planet;
- mouse-look sensitivity scales by current FOV / default FOV below 45 degrees,
  so aiming remains usable rather than becoming hypersensitive at telescope
  magnifications;
- the mouse wheel changes FOV only and never moves the camera;
- interactive mode auto-orbits while retaining the mouse look offset;
- planet rotation period is 600 simulation seconds.

Camera yaw/pitch, the planet-local forward vector, and its orthonormal
forward/right/up basis remain f64 on the CPU. Terrain chunk anchors are
subtracted from the f64 camera position and transformed into view space with
f64 dot products before the resulting camera-relative value is packed as f32.
The GPU projection is therefore projection-only; it rotates only the much
smaller anchor-local vertex offset. Fullscreen atmosphere and sun shaders form
rays directly in camera-local coordinates and consume CPU-transformed radial
and sun directions. Do not reintroduce a planet-frame f32 look vector or rotate
multi-megameter f32 anchor offsets in a view matrix: both quantize by tens of
pixels at the narrowest FOV.

Controls:

| Input | Effect |
|---|---|
| Mouse motion | Captured, unbounded free look relative to planet-down |
| Mouse wheel | Optical FOV zoom |
| Left/Right arrows | Orbit azimuth by 0.08 radians |
| Up/Down arrows | Orbit elevation by 0.05 radians |
| F3 | Toggle debug HUD |
| F4 | Toggle orbit / Mach 10 level-flight camera, held 5,000 ft above the streamed terrain plus mirrored global microrelief (or sea level over ocean); it starts level with a horizontal horizon, and mouse yaw/pitch maps to local left/right and sky/ground without changing the flight path |
| F6/F7/F8 | Toggle blur/bloom/HDR filmic effect |
| F9 | Cycle composition debug: raw albedo, surface lighting, aerial contribution, sky-only, final HDR |
| F10 | Freeze/resume interactive scene time (orbit, rotation, ocean, exposure adaptation) for matched diagnostics |
| F12 | Capture PNG into the current run directory |
| Escape or Q | Quit |

The wheel is deliberately routed before egui consumption. Focus changes grab
or release the cursor. `ControlFlow::Poll` keeps interactive rendering display
paced; do not reintroduce the old idle 10 FPS scheduling bug.

Wheel zoom is multiplicative (`exp(-wheel_delta * 0.12)`). A line-wheel step is
small enough to cross at most one normal SSE level boundary; the complete 75
degree to 0.00005 degree range takes 119 such steps at 640px height. The
minimum's half-FOV tangent scales by `viewport_height / 640`, keeping the
maximum-zoom physical size per screen pixel constant when the window is
resized. The extreme endpoint is not an arbitrary LOD override: at the default
6,000 km altitude it covers about 5.24 m vertically at 640px and naturally
makes the 2-pixel SSE policy request L18. A regression exercises heights 1,
240, 640, and 2160 and verifies that each still traverses every L2-L18 level.

## Cube-sphere and LOD

### Supported levels

- The quadtree addresses L0 through L18 inclusive: 19 structural levels.
- `MINIMUM_LOD_LEVEL = 2`, so active rendered leaves are L2 through L18:
  17 renderable levels. L0 and L1 are internal ancestors.
- Every leaf uses 33x33 vertices / 32x32 quads plus skirts.
- Maximum active leaf budget is 1,024.
- Split threshold is 2.0 projected pixels.
- Merge threshold is 1.25 pixels, providing hysteresis.
- Skirt depth is 7.5% of the chunk edge length.

`PlanetLod` starts from visible face roots, horizon/frustum-culls
placeholder-based approximate node spheres, ranks split candidates by demand,
retains previous splits inside hysteresis, respects the leaf budget, and caches
identical selection inputs.

### What the HUD LOD means

The HUD now prints the active range, for example `L5-L8`, rather than only
`TerrainStats::max_level`. Mixed levels can coexist because the horizon and
viewport edges need less detail than the focal patch. The full 19-slot
histogram remains present in spatial JSONL records. FOV formatting adds decimal
places below one degree, so telescope values no longer misleadingly display as
`0.0 degrees`.

### Orbital optical zoom

Optical zoom is now governed by the same SSE policy as altitude changes. There
is no FOV-specific minimum-level floor. The prior 8-degree rule that forced a
single-frame L2-to-L4 jump was removed, and the old `projected_error_pixels`
lower clamp of 0.01 radians was replaced by the actual camera minimum FOV. The
old clamp would otherwise have capped refinement around L5 even after widening
the camera's zoom range.

At the default camera and 960x640 viewport, representative zoom-in thresholds
are approximately L3 at 3.699 degrees, L4 at 1.671 degrees, L10 at 0.02348
degrees, L17 at 0.000183 degrees, and L18 at 0.00009155 degrees. Zoom-out occurs
around 1.6 times wider because the normal 2.0-pixel split / 1.25-pixel merge
hysteresis remains in control. No second FOV ladder or per-frame forced level
exists.

The unit regression drives actual one-step wheel input from 75 degrees to the
minimum and back, asserting the exact maximum-level sequence L2 through L18
and L17 back through L2, zero thrash, and no leaf-budget pressure. A second
selector regression repeats the zoom-in ladder at 1, 240, 640, and 2160 pixels
high, proving that the viewport-aware endpoint still reaches every level after
a resize. The embedded `orbital_zoom_lod` GPU scenario independently uses
log-space 640px-reference FOV waypoints at a fixed non-cardinal orbit aimed at
the sparse +X validation patch and asserts the same per-frame sequence. The
scenario runner's reference half-FOV tangent is scaled to the actual viewport,
and a 240px scenario-level regression proves the exact round trip remains
portable rather than stopping at L17. A separate
one-physical-pixel regression at the minimum FOV verifies about 0.49 pixels of
smooth screen movement rather than a stuck direction followed by a multi-pixel
f32 jump.

### LOD transitions and GPU representation

`TerrainRenderer` keeps parent and child render nodes together for 0.5
simulation seconds. `planet.wgsl` uses a 4x4 Bayer discard pattern to dither
between them. Skirts remain active. All chunks share one index topology.

Chunk vertices are static anchor-local f32 data. Each chunk anchor and the
camera basis are f64; a per-frame instance carries the anchor transformed in
f64 into camera-relative view space and only then cast to f32. `planet.wgsl`
adds the small anchor-local offset after an f32 basis rotation and applies the
projection-only reversed-Z matrix. Terrain aerial perspective and ocean view
lighting use this precise view-space position while planet normals, height,
biomes, and direct-sun occlusion remain planet-frame. Rendering currently
issues one indexed draw per render node because the tile bind group can differ.

## Outmap runtime and generated data

### Shared schema

`crates/coretypes/src/lib.rs` owns `CubeFace`, `TileKey`, `BiomeId`, and
`OutmapManifest` validation.

Current schema facts:

- schema version: 2
- planet radius: 4,000,000 m
- levels: L0-L18
- logical samples per tile: 129x129
- one-sample gutter on every edge
- stored samples: 131x131
- channels: signed little-endian `height.r32f`, `biome.r8`, `moisture.r8`
- ten stable biome IDs
- normals are not stored
- available tile keys must be sorted, unique, and parent-complete

### Current local default outmap

`assets/outmaps/test-planet/manifest.json` currently reports:

- generator: `catinthegarden-baker 0.1.0`
- seed: `0x000C471A` / 804634
- working grid: 4096x2048
- global dense coverage: L0-L4
- sparse +X refinement: parent-complete through L18, radius 1
- height range: -5,000 m to +9,000 m
- listed tiles: 2,172
- local disk use at review time: about 254 MiB

Of these tiles, 2,046 are global L0-L4 coverage. Only nine tiles per level from
L5 through L18 refine the shrinking +X landing patch. Elsewhere, geometry may
reach a deeper LOD while texture requests fall back to L4. More triangles then
cost performance without adding height/biome/moisture detail.

The sparse +X path is a validation/landing region, not a globally detailed
planet. The base bake forces -10 m inside 2 degrees of +X and blends back to
generated terrain by 6 degrees for the 10 m descent scenario. A separate 500 m
ramp suppresses the added L3+ 300 m terrain-detail term near the exact landing
point. Sparse microrelief starts its ramp at zero on L12, becomes non-zero on
L13, and reaches a bounded +/-2 m at L18; it is exactly zero only at +X. Full
dense L18 is intentionally infeasible; future detail must remain
region-sparse/page-streamed.

### Runtime loading

`outmap::Outmap` validates the manifest, resolves a requested key to its nearest
available ancestor, reads and validates all three raw files, and returns CPU
vectors. `TerrainRenderer` uploads three textures per resident tile:

- `R32Float` height;
- `R8Uint` biome;
- `R8Unorm` moisture.

The cache holds at most 384 tiles and evicts by last-used tick. Disk reads and
GPU texture creation/upload are synchronous in `TerrainRenderer::update`.
Cached CPU height data is also used for seam measurements.

### Current geometric-error caveat

`projected_error_pixels` and `node_bounds` still use placeholder procedural
error/height estimates. The manifest does not yet contain measured per-tile
geometric error or min/max bounds. Baked terrain roughness therefore does not
directly drive SSE or culling, which can over- or under-refine extreme regions.

## Baker

`catinthegarden-baker` is both a library and CLI. `bake` validates the config,
generates terrain, exports tiles/previews, then reopens and validates output.

Default generation order:

1. spherical 3D domain-warped fBm continents;
2. ridged mountains masked by tectonic-belt noise;
3. iterative hydraulic erosion;
4. interleaved talus/thermal erosion;
5. D8 flow direction and accumulation;
6. flow-scaled river carving;
7. priority-flood lake identification;
8. U-shaped glacial valley carving;
9. deterministic +X landing patch;
10. moisture distance field and blur;
11. latitude/height/moisture biome classification;
12. preview and raw tile export with parent-constrained borders.

Export adds L3+ terrain/material breakup up to 300 m and a sparse microrelief
ramp that is zero at L12 and reaches +/-2 m at L18. Validation checks file
sizes, finite/ranged height, biome IDs, required parents, fallback-edge
continuity, and landing height.

Generated outmaps are ignored and must be rebuilt in a fresh checkout.

## Surface, atmosphere, sun, HDR, and ocean

### Terrain/material shader

`planet.wgsl` handles height displacement, central-difference normals,
biome/moisture material lookup, coastline blending, terrain lighting, ocean,
aerial perspective, and LOD dither.

- Height is bilinear sampled.
- Biome/moisture remain categorical at the 129x129 material resolution.
- Authored palette values are converted from sRGB to linear before HDR light.
- Beach tint uses continuous bilinear height rather than nearest biome class.
- Ocean coverage blends roughly from -80 m to +120 m.
- Surface direct sunlight has a 2x artistic scale and clamps at zero on the
  terminator, preventing direct light from washing terrain materials white.
- Terrain and ocean receive diffuse fill from a bounded local-atmosphere sample
  directly above the surface normal. This keeps nearby sky colour on the
  surface through sunset independently of direct-sun visibility without the
  unstable energy and vertex cost of sparsely sampled near-horizon paths.
- Fully occulted direct and sky contributions become zero.

### Atmosphere

`atmosphere.wgsl` is a 16-sample fullscreen raymarch. Terrain aerial
perspective is computed per vertex in `planet.wgsl`; ocean recomputes it per
fragment. Current shared constants are:

- atmosphere shell: 720 km;
- top-edge density fade: 480 km;
- Rayleigh scale height: 36 km;
- Mie scale height: 4.8 km;
- Rayleigh coefficient: `(5.8, 13.5, 33.1)e-6 / m`;
- Mie coefficient: `0.01e-6 / m`;
- Mie g: 0.76;
- visual solar radiance: 1.25.
- fullscreen and aerial scattering use the shared physical coefficients with no
  separate visual brightness, forward-Mie, or artificial limb multiplier.

The raymarch excludes space before atmosphere entry from optical depth, stops
at the solid-planet intersection, and uses a density/spacing-aware penumbra
for smooth directional occultation. Deep dark-side samples receive no leaked
direct in-scattering or far-side shell contribution. Aerial in-scatter uses the
same scale-height- and air-mass-bounded Rayleigh/Mie path lengths as aerial
extinction, and is limited to the finite fraction removed from the view ray;
an opaque horizon chord therefore cannot add unbounded light. Screen rays are
generated as camera-local `(x, y, -1)` directions, and
the f64 CPU basis transforms camera radial/sun vectors into that frame before
f32 upload; rebuilding tiny ray offsets from large planet-frame f32 basis
vectors would quantize the maximum optical zoom.

### Sun

`sun.wgsl` draws an additive camera-facing angular disc:

- physical angular radius: 0.004625 radians;
- visual size multiplier: 3x, about 1.59 degrees apparent diameter;
- corona radius: eight visual radii;
- camera-only radiance multiplier: 5x;
- HDR core: `(72, 65, 52)` before the visual multiplier;
- HDR halo: `(6, 5.5, 4.5)` before the visual multiplier.

Sun-disc angular distance uses `atan2(length(cross(ray, sun)), dot(ray, sun))`
rather than `acos(dot)`, retaining sub-microradian separation near alignment.

It changes appearance only, not scene illumination.

### HDR and exposure

`HdrRenderer` owns the `Rgba16Float` scene, luminance mip chain, triple-buffered
1x1 readback, exposure uniform, and ACES pass.

- exposure key: 0.18
- adaptation speed: 1.5
- minimum exposure: 0.05
- maximum exposure: 4.0
- tone map: Narkowicz ACES fitted approximation

Disabling the HDR effect bypasses ACES but continues to apply the current
auto-exposure value. This keeps dim atmospheric outlines visible without
changing the surface-lighting inputs.

The 4x cap prevents a mostly black orbital frame from washing out the visible
planet. Egui is rendered after tonemapping and is not affected by exposure.

### Ocean

The rendered ocean is in `planet.wgsl`; `ocean.rs` mirrors its wave heights for
deterministic assertions.

- six spherical Gerstner waves;
- sea level is height zero;
- per-pixel displaced normal;
- Blinn-Phong sun glint;
- Schlick Fresnel;
- daylight/sky-driven blue body scattering;
- static cubemap reflection, gated by direct daylight;
- no SSR and no moonlight baseline;
- CPU-mirrored observed wave range is about 5.166 m.

## Debug, screenshots, scenarios, and profiling

### Artifact layout

Every launch creates a run directory, including ordinary manual launches:

```text
test-runs/{scenario-or-manual}/{unix-seconds}-{pid}/
  manifest.json
  log.jsonl
  screenshots/
    manifest.json
    capture-001.png
    ...
```

Manual/interrupted runs normally retain `passed: null`. A completed scenario
records assertion results and exits with status 1 if it fails.

Spatial records are emitted about every 0.5 simulation seconds and include:

- sim time and f64 camera xyz;
- latitude, longitude, altitude, velocity, orientation, and FOV;
- sun direction and planet rotation;
- complete L0-L18 histogram;
- chunk/tile load, unload, resident, fallback, draw, seam, thrash, and
  leaf-budget metrics;
- frame time, exposure, and ocean wave range.

Exposure records are emitted every rendered frame. Logs store event fields
under `.fields`; filter by `.target` with `jq`.

Spatial JSONL remains sampled every 0.5 simulation seconds. The scenario
assertion tracker separately observes maximum active LOD, resident-chunk
bounds, thrash events, and leaf-budget pressure every rendered frame, without
increasing log volume. This is what lets a short zoom scenario prove that no
transient LOD or one-frame budget/count violation was skipped.

Screenshots are post-tone-map, post-egui swapchain readbacks. Capture completion
waits synchronously, so capture frames must not be used as normal performance
samples.

### Embedded scenarios

Scenario JSON is compiled with `include_str!`; rebuild after editing JSON.
All use a fixed 1/60 s simulation step. Unless a terrain flag is supplied, the
app uses the default outmap when its manifest exists and placeholder otherwise.
Vertical-FOV waypoints are authored as 640px-reference values; the runtime
scales their half-FOV tangent to framebuffer height before applying them. CLI
and interactive FOV values remain actual viewport FOVs, not reference values.

| Scenario | Purpose | Duration | PNGs | Objective checks |
|---|---:|---:|---:|---|
| `still_5s` | Harness/solid clear | 5 s | 3 | finite/count and solid-color PNGs |
| `orbit_once` | Cube-sphere orbit | 4 s | 4 | chunk bounds and CPU seam delta |
| `descent_to_10m` | Full LOD descent | 12 s | 7 | reaches L18, monotonic, no thrash, seam/fallback bounds |
| `sunset_sweep` | Sunset color | 8 s | 4 | red/blue growth and warm final sample |
| `night_side_atmosphere` | Occlusion | 2 s | 1 | dark sky and >=5x day/night surface ratio |
| `limb_atmosphere` | Orbital limb | 1 s | 1 | finite/count/seam/fallback; still visual-only |
| `ground_to_orbit` | Sky-to-space/HDR | 8 s | 7 | continuous sky luminance and stable bounded exposure |
| `stare_at_sun` | Exposure response | 4 s | 3 | bounded, smooth, non-oscillating exposure |
| `ocean_flyover` | Gerstner ocean | 6 s | 5 | >=0.5 m mirrored wave-height range |
| `orbital_zoom_lod` | Fixed-orbit optical zoom | 14 s | 5 | exact L2->L18->L2 sequence, no budget pressure/thrash, seam/fallback bounds |

Current scenario files have screenshot `seam_gap_check` disabled or defaulted
off. CPU `max_seam_delta_m` assertions still run where configured. Do not claim
that current PNGs perform the background-gap seam scan unless this flag is
explicitly enabled and rerun.

### Render profiling

`--profile-render` adds sampled CPU stage records and GPU timestamps.

`catinthegarden::render_profile` fields include simulation, egui CPU, surface
acquire, egui upload, vertex upload, encode, submit, present, capture readback,
timestamp scheduling, and total render time. `vertex_rebase_ms` is currently
always zero, and terrain update work is included in `simulation_ms`.

`catinthegarden::render_profile.fields.gpu_render_ms` is always the `-1.0`
placeholder. Actual GPU time is a separate `catinthegarden::gpu_profile` event.
Its timestamp interval spans the HDR scene through the end of tone mapping,
including the luminance chain, but excludes egui and present. FIFO frame pacing
can appear in acquire, submit, or present depending on the backend; inspect all
three rather than misdiagnosing pacing as CPU encode time.

## Commands

Run commands from the repository root.

### Formatting, build, and tests

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo build --release --bin catinthegarden-app
```

If `/tmp` is constrained:

```bash
TMPDIR=/home/dad/.cache/citg-tmp cargo test --workspace
```

### Interactive app

```bash
cargo run --release --bin catinthegarden-app
cargo run --release --bin catinthegarden-app -- --profile-render
cargo run --release --bin catinthegarden-app -- --vertical-fov-degrees 8
cargo run --release --bin catinthegarden-app -- --vertical-fov-degrees 0.00005
cargo run --release --bin catinthegarden-app -- --terrain placeholder
cargo run --release --bin catinthegarden-app -- --terrain outmap
cargo run --release --bin catinthegarden-app -- --outmap /path/to/outmap
```

There is currently no app `--help`; invalid arguments panic during startup.

### Scenario execution

```bash
cargo run --release --bin catinthegarden-app -- \
  --scenario sunset_sweep --terrain outmap
```

Recommended local GUI command template (the verified form used 120 s; 240 s is
safer for slow scenarios):

```bash
DISPLAY=:0 XDG_DATA_HOME=/home/dad/.cache/citg-xdg \
  timeout 240s ./target/release/catinthegarden-app --scenario sunset_sweep
```

Some scenarios take much longer in wall time than simulated duration. Increase
the timeout and inspect the manifest rather than assuming a timeout is a shader
failure.

Inspect the newest result:

```bash
name=sunset_sweep
latest=$(find "test-runs/$name" -mindepth 1 -maxdepth 1 -type d | sort | tail -1)
jq . "$latest/manifest.json"
jq 'select(.target == "catinthegarden::spatial") | .fields' "$latest/log.jsonl"
jq 'select(.target == "catinthegarden::render_profile") | .fields' "$latest/log.jsonl"
jq 'select(.target == "catinthegarden::gpu_profile") | .fields' "$latest/log.jsonl"
```

### Baker

```bash
# Full default bake; expensive and writes ignored data.
cargo run --release -p catinthegarden-baker

# Small development bake. Put --quick before overrides because it resets config.
cargo run --release -p catinthegarden-baker -- \
  --quick --output /tmp/citg-quick-outmap

# Validate existing output without rebaking.
cargo run --release -p catinthegarden-baker -- \
  --validate assets/outmaps/test-planet
```

Other baker flags: positional output or `--output`, `--seed`, `--width`,
`--height`, `--dense-level`, `--max-level`, `--sparse-radius`, and
`--erosion-iterations`. The baker supports `-h`/`--help`.

## Phase status

| Phase | Status | Current result |
|---|---|---|
| 0 | Complete | wgpu/winit/egui skeleton and FPS HUD |
| 0.5 | Complete | JSONL logs, HUD toggle, PNG capture, fixed-step harness |
| 1 | Complete | Fixed cube-sphere and f64-to-f32 rebased orbit camera |
| 2 | Complete | Six quadtrees, SSE L2-L18 altitude/optical LOD, skirts, transitions, descent and zoom regressions |
| 3 | Complete | Deterministic full baker and preview/raw export |
| 4 | Complete | Runtime outmap streaming, height displacement, biome materials |
| 5 | Complete | Analytic atmosphere, aerial perspective, transition scenarios |
| 5.5 | Complete | Sun, HDR target, luminance chain, exposure, ACES |
| 6 | Complete | Gerstner ocean, reflection, Fresnel, ocean scenario |
| 7 | Not complete | LOD-range/FOV HUD polish is done; ice-cap visuals and final performance/all-scenario regression remain |

## Verification snapshot

Verified on 2026-07-13 against the optical-zoom implementation working tree
based on `f993fd8`:

- `cargo fmt --all`: passed.
- `TMPDIR=/home/dad/.cache/citg-tmp cargo check --workspace`: passed; 17
  existing warnings.
- `TMPDIR=/home/dad/.cache/citg-tmp cargo test --workspace`: 80 passed, 0
  failed (app 53, baker library 17, baker binary 1, baker integration 4,
  coretypes 5).
- `TMPDIR=/home/dad/.cache/citg-tmp cargo build --release --bin
  catinthegarden-app`: passed.
- `orbital_zoom_lod`: passed all nine assertions and captured five PNGs. The
  observed per-frame maximum-level sequence exactly matched L2 through L18 and
  L17 back through L2; maximum per-frame resident chunks 43, budget-limited
  frames 0, thrash events 0, maximum sampled seam delta 0 m, maximum sampled
  fallbacks 6. The scenario uses a non-cardinal camera aimed at +X, so it also
  exercises the f64 view-rebase path that the previous cardinal pose masked.
  On the verified 1.5x display scale the framebuffer was 1440x960 and the
  viewport-aware endpoint became 0.000075 degrees while retaining the exact
  ladder.
- `orbit_once`: passed; four PNGs, maximum per-frame resident chunks 45,
  maximum sampled seam delta 0 m.
- `descent_to_10m`: passed; reached L18 monotonically, seven PNGs, maximum
  per-frame resident chunks 48, thrash 0, maximum sampled seam delta 0 m,
  maximum sampled fallbacks 2.
- `ground_to_orbit`: passed; seven PNGs, maximum per-frame resident chunks 48,
  maximum sampled fallbacks 30, sky-luminance delta 0.307, 481 exposure samples
  within 0.05-4.0, and no exposure oscillation.
- `sunset_sweep`: passed; four PNGs, maximum per-frame resident chunks 48,
  maximum sampled fallbacks 38, maximum seam delta 0.00000191 m, red/blue
  growth 15.961, and final red/blue ratio 11.000.
- `night_side_atmosphere`: passed; one PNG, sampled sky luminance 0.000 and
  rendered day/night surface ratio 626.327.
- `limb_atmosphere`: passed; one PNG, maximum per-frame resident chunks 44,
  maximum sampled fallbacks 0, and maximum sampled seam delta 0 m. It still
  has no dedicated tier-2 limb-shape assertion.
- `stare_at_sun`: passed; three PNGs, 241 bounded exposure samples, maximum
  per-frame exposure step 0.0187, and no oscillation.
- `ocean_flyover`: passed; five PNGs, maximum per-frame resident chunks 8,
  maximum sampled fallbacks 4, maximum sampled seam delta 0 m, and a 5.166 m
  mirrored Gerstner wave-height range.

Those scenario manifests record `f993fd8` because the implementation was dirty
when exercised; the manifest format does not record dirty state. The source
content tested is the content described here, but this is not a clean-HEAD
replay. The pre-existing `.gitignore` and root PNG changes remained untouched
throughout.

Known warnings are the deprecated `winit` `EventLoop::create_window` and
`EventLoop::run` APIs plus phase-leftover dead code/unused fields. They are not
current test failures.

## Working-tree safety snapshot

At the start of this optical-zoom task, the following pre-existing user state
was intentionally left untouched and must not be swept into unrelated commits:

```text
 M .gitignore
?? capture-001.png
?? capture-002.png
?? limb-after-2.png
?? limb-after-3.png
?? limb-after-crop.png
?? limb-after.png
?? limb-crop.png
```

The `.gitignore` edit adds `.env`, `.env.*`, and the `!.env.example` exception.
The PNGs are user/reference captures. Re-run `git status --short`; this section
is a snapshot, not permission to alter those files.

Verification during this task also produced an unexplained zero-byte untracked
file named `::call_once::h52f3341f83ee0b20`. It is not implementation input,
was excluded from the commit, and was left in place because repository rules
forbid deleting files without an explicit request.

## Known limitations and actionable risks

1. Baked outmap detail above L4 exists only around +X. Deep geometry elsewhere
   still samples L4 macro height/material fallback, but now gains bounded
   seam-safe land microrelief and material breakup from planet-local direction;
   it is not equivalent to fully baked erosion or biome detail.
2. The maximum orbital zoom covers about 5.24 m vertically at the 640px
   reference height (the physical span scales with viewport height to preserve
   metres per pixel). CPU f64 view-space rebasing and the one-pixel camera
   regression remove the previous 0.25-0.5 m non-cardinal quantization, but
   there is not yet a GPU temporal pixel-motion assertion; the current
   maximum-zoom screenshot is uniform ocean and cannot measure subpixel
   tracking from frame to frame.
3. The sparse +X L18 validation patch lies inside the intentionally flat ocean
   landing region. It proves geometry/streaming selection but is not a visually
   rich demonstration of metre-scale land detail; the maximum-zoom scenario
   capture is a uniform ocean-color image and must not be cited as pixel-level
   proof of added surface detail.
4. SSE still uses placeholder geometric-error and distance estimates rather
   than measured per-tile baked error. Horizon/frustum culling no longer uses
   placeholder-only world-space spheres: it conservatively tests each node's
   angular cone across the manifest's complete radial height range plus the
   111.5m procedural amplitude. This fixes elevated near-field holes without
   the leaf-budget explosion caused by the failed blanket 12.5km sphere margin.
5. Runtime tile disk reads and GPU uploads are synchronous.
6. Terrain currently performs one draw per active/fading render node.
7. `limb_atmosphere` has no tier-2 pixel assertion and still relies on visual
   review beyond finite/count/seam/fallback checks.
8. Screenshot seam-gap scans are disabled in current scenario JSON, despite
   historical wording about seam-checked screenshots.
9. Generated outmaps and test runs do not exist in a fresh clone until rebuilt.
10. Atmosphere constants are duplicated between two shaders and can drift.
11. `triangle.wgsl` and fixed `CubeSphereMesh` are Phase-0/1 leftovers; do not
    assume they drive the current terrain renderer.
12. Phase 7 has not had a single clean-HEAD all-scenario regression run.
13. The zero-byte untracked `::call_once::h52f3341f83ee0b20` artifact described
    above remains in the working tree pending explicit permission to remove it.

## Next action

Manual run `1784065154-40230` exposed large quadtree-shaped holes near the
Mach-10 camera over roughly 3,012m baked terrain. The captured camera regression
proved that a +Z root could be culled while its descendant intersected the
visible elevated shell: placeholder-only world-space bounds did not contain
the baked surface. `TerrainHeightRange` now comes from the active outmap
manifest (expanded by global microrelief), and horizon/frustum tests maximise
each plane over the node's angular cone and radial range rather than inflating
every fine node tangentially. The exact captured view and the general low-flight
ray-coverage test both pass without leaf-budget pressure; `polar_ice_cap` also
passes as a rendered wgpu smoke regression. Human replay of the same free-flight
route remains the final visual sign-off.

The low-flight detail experiment now covers the whole planet rather than a
pre-baked corridor. `planet.wgsl` layers four bounded, direction-based relief
bands (about 6km through 12m wavelength) over baked land above the coastline
and uses the same value for modest material variation. It costs four sine
evaluations per height sample, with detail interpolated to the fragment stage
instead of recomputed per pixel. `planet.rs` mirrors the exact field, and
`TerrainRenderer::surface_height_meters_at` applies it to the highest resident
tile while resolving below-sea samples to sea level. Focused CPU tests cover
the amplitude, coastline/ocean behavior, and low-flight screen-ray coverage.
The next visual check should fly away from +X and compare terrain readability
and frame-stage profile samples before changing amplitudes or frequencies.

Phase 7 now includes full-screen blur and bloom stages. Their startup defaults
are `BLUR_ENABLED` and `BLOOM_ENABLED` in `crates/app/src/planet.rs`; F6 and F7
toggle them at runtime. Bloom now thresholds and blurs the HDR scene in its own
target before compositing it over the original (or standalone-blurred) scene,
so neither effect disables the other. `--profile-render` logs separate
asynchronous GPU times for scene, luminance, blur, bloom, tone-map, and egui
stages. Terrain aerial perspective evaluates solar visibility at a
representative air sample and adds bounded sunward skylight, retaining
low-sun horizon haze when the terrain endpoint has just entered shadow. The
debug panel also shows live effect state plus split/merge/cull work.

The active experiment branch, `experiment/composition-debug`, adds F9
composition diagnostics. They are deliberately rendered from the normal camera
uniform and captured through the normal HDR path, so equivalent F12 captures
can isolate whether an artifact originates in source material, surface
lighting, aerial contribution, or the fullscreen sky before changing visual
constants again.

The first F9 capture set found detailed source albedo and surface lighting, but
an overwhelmingly bright aerial-contribution view and nonphysical sky-only
bands. The temporary 8× fullscreen brightness, 32× Mie forward lobe, and
fullscreen artificial limb term were therefore removed. Re-capture the same
five F9 modes before further colour tuning; terrain palette and direct surface
lighting should remain untouched unless their own diagnostic modes regress.

F10 freezes the interactive scene time before a diagnostic capture set, keeping
the camera, planet rotation, ocean phase, and exposure fixed while F9/F12 are used. The
sky raymarch retains its 16-sample budget but distributes samples cubically
around a ray's lowest atmospheric point, where exponential density changes
fastest. It also terminates at the solid planet rather than rendering the
far-side atmosphere through it. Aerial in-scatter now uses the phase-weighted
finite scattered view fraction rather than an unbounded emissive `beta *
length` term. It uses two density-weighted samples along the bounded view
column, each with local solar visibility and camera-to-sample transmittance,
instead of applying one midpoint air sample to the whole column. These changes
preserve the current fullscreen sample budget. `AERIAL_IN_SCATTER_GAIN` is a
separate 3.0x visual control applied only after that bounded aerial result; it
does not change extinction, direct terrain/ocean lighting, or the sky pass.

The bounded polar slice is implemented in `polar_ice_cap`: baked Ice overrides
ocean at the poles, receives a cool diffuse floor, and a center-pixel assertion
checks that the visible cap is bright and sufficiently neutral. The focused
scenario passes. A full regression attempt was started, but the existing unit
suite currently fails six LOD/rotation expectations unrelated to post effects;
do not describe Phase 7 as fully regressed until those policy/test mismatches
are resolved and every named scenario completes from one clean HEAD.

### Previous next action

Begin Phase 7 with one bounded ice-cap visual slice:

1. Verify how `BiomeId::Ice`, `MountainSnow`, latitude, and height currently
   reach the terrain shader.
2. Add a deterministic polar camera scenario with at least one objective pixel
   or material assertion; do not rely only on a screenshot review.
3. Implement only the missing polar/high-altitude ice-cap presentation needed
   by that assertion, preserving the current Earth-like palette and lighting.
4. Run the new polar scenario plus `sunset_sweep`, `night_side_atmosphere`, and
   `ground_to_orbit` before expanding into the rest of Phase 7.

Read first for that task:

- `crates/coretypes/src/lib.rs`: biome IDs and schema contract;
- `crates/baker/src/`: snowline and biome classification output;
- `crates/app/src/planet.wgsl`: biome palette and terrain lighting;
- `crates/app/src/scenario.rs`, `crates/app/src/debug.rs`, and existing scenario
  JSON: smallest deterministic polar assertion path.

Do not reopen the now-pinned optical LOD policy, atmosphere constants, ocean,
or baker erosion pipeline unless the polar regression exposes direct evidence
that one of them is involved.

## Longer-term follow-ups after the next action

These are separate tasks, not permission to scope-creep:

1. Store per-tile geometric error and height bounds in the outmap manifest and
   drive SSE/culling from baked data.
2. Add tier-2 limb/terminator image assertions.
3. Decide whether to batch/indirect terrain draws without changing tile binding
   behavior.
4. Move synchronous tile I/O/upload behind a bounded streaming queue.
5. Complete Phase 7 ice-cap visuals, remaining debug-panel additions, and
   performance polish.
6. Run every named scenario from a clean HEAD and record the regression result
   here.

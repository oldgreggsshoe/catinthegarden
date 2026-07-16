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
- Remote branch: `origin/experiment/composition-debug`
- Implementation baseline reviewed and validated: `27ebd43` (`Preserve
  twilight contrast without HDR`).
- Last full source review: 2026-07-16
- Current phase status: Phases 0 through 6, including 5.5, are complete.
- Phase 7 status: in progress. Blur, bloom, per-stage profiling, HUD additions,
  the polar ice slice, free flight, and bounded terrain streaming are
  implemented. The clean all-scenario regression is complete.
- Current bounded engineering issue: objective validation is green. Phase 7
  still needs final human visual sign-off before promotion to `main`.

This handoff synchronizes the canonical sections with the current experiment
branch. Always use `git log -1 --oneline` and `git status --short` rather than
assuming this snapshot is still HEAD.

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
- independently toggleable full-screen blur and HDR bright-pass bloom;
- a six-wave spherical Gerstner ocean with daylight-gated reflection;
- orbit and terrain-relative Mach 300 free-flight cameras;
- deterministic scenarios, JSONL logging, PNG capture, assertions, and
  opt-in CPU/GPU render profiling broken down by render stage.

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
  camera from the f64 chunk anchor and transform the result into view space
  with the f64 camera basis before casting the relative value to f32.
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
  longitude without changing the baked coastline or biome. Positive baked land
  height and that microrelief are visually exaggerated 4x after coastline and
  material classification; ocean sea level remains zero.
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
- Give temporary or exact-staged-tree checkouts a separate `CARGO_TARGET_DIR`.
  Reusing this worktree's `target/` can leave its runnable binary compiled from
  the other checkout even when a subsequent worktree `cargo build` reports it
  as fresh; if that happens, touch the differing source and rebuild here.
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
- default render surface: 640x427 physical pixels, preserving the previous 3:2
  framing without display-scale multiplication
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
11. Render atmosphere and terrain/ocean into the HDR/depth scene.
12. Build the luminance mip chain from that physical scene and schedule its readback.
13. Add the depth-tested visual sun disc/corona to HDR, after metering but before post effects.
14. Optionally run full-screen blur and independent HDR bright-pass bloom.
15. Apply exposure and optionally ACES-tonemap HDR to the swapchain.
16. Render egui after tonemapping.
17. Optionally copy the swapchain texture for a PNG screenshot.
18. Submit, map asynchronous buffers, present, and finalize captures/scenarios.

Render order:

```text
Rgba16Float HDR scene + reversed-Z Depth32Float
  atmosphere fullscreen triangle: replace background, no depth write
  terrain/ocean indexed chunks: depth compare Greater, depth write
  luminance extraction and mip downsample
  sun fullscreen triangle: additive, depth-equal background-only, after luminance
  optional 5x5 full-screen blur
  optional HDR >1.0 bright-pass blur + 0.75 additive bloom composite
  exposure + optional ACES tone map to swapchain
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
- planet rotation period is 600 simulation seconds; interactive mode advances
  it at 0.3×, so its apparent day is 2,000 wall-clock seconds and the
  planet-relative flight camera sees the world-space sun move 2.7° per 15 s.
- the shared interactive/default sun direction uses Earth's 23.439281° northern
  solstice declination relative to the planet's Y spin axis. There is no annual
  orbital revolution yet, so the declination remains fixed rather than
  inventing an incomplete season cycle; scenarios with authored sun waypoints
  continue to override it.

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
| F4 | Toggle orbit / Mach 300 free-flight camera; it starts level 5,000 ft above resident terrain, retains a terrain-aware minimum clearance, and restores the orbital pose when toggled back |
| W / S | While in flight mode, move at Mach 300 exactly along / opposite the current camera-facing vector; releasing both stops forward/backward translation |
| A / D | While in flight mode, strafe camera-left / camera-right at Mach 300; diagonal input is normalized |
| F6/F7/F8 | Toggle blur/bloom/HDR filmic effect |
| F9 | Cycle composition debug: raw albedo, surface lighting, aerial contribution, sky-only, final HDR |
| F10 | Freeze/resume scene time (orbit, rotation, ocean, exposure adaptation); low-flight camera movement remains active for framing |
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
- `MINIMUM_LOD_LEVEL = 2`, so active rendered leaves are L2 through L18: 17
  renderable levels.
- Every leaf uses 33x33 vertices / 32x32 quads plus skirts.
- Maximum active leaf budget is 256.
- Split threshold is 2.0 projected pixels.
- Merge threshold is 1.25 pixels, providing hysteresis.
- Skirt depth is 7.5% of the chunk edge length, capped at 50m so coarse
  fallback skirts cannot become exposed planet-scale walls in low flight.

`PlanetLod` starts from face roots, horizon/frustum-culls each node's angular
footprint across the outmap's conservative radial height range, ranks split
candidates by demand, retains previous splits inside hysteresis, respects the
leaf budget, and caches identical selection inputs.

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

Under the L2, 2.0/1.25-pixel policy at the default camera and
640x427 viewport, representative zoom-in thresholds were approximately L3 at
2.468 degrees, L4 at 1.115 degrees, L10 at 0.01567 degrees, L17 at 0.000122
degrees, and L18 at 0.0000611 degrees.

The intended regression drives actual one-step wheel input from 75 degrees to
the minimum and back, requiring the exact maximum-level sequence L2 through
L18 and L17 back through L2 with zero thrash. A second
selector regression repeats the zoom-in ladder at 1, 240, 640, and 2160 pixels
high. The embedded `orbital_zoom_lod` GPU scenario uses log-space
640px-reference FOV waypoints at a fixed non-cardinal orbit aimed at the sparse
+X validation patch and requires the same per-frame sequence. The 256-leaf
ceiling intentionally reports budget pressure during telescope zoom rather
than restoring the former unbounded draw workload; the scenario instead
asserts at most 256 resident and fallback chunks. The scenario runner's
reference half-FOV tangent remains viewport-scaled. A separate
one-physical-pixel regression at the minimum FOV still covers smooth camera
motion rather than an f32 direction jump.

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
- baked height range: -5,000 m to +9,000 m; visible positive land is currently
  exaggerated 4x at runtime, up to roughly +36,000 m before microrelief
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

`projected_error_pixels` still uses a placeholder geometric-error ratio because
the manifest does not contain measured per-tile error. Culling and distance no
longer use placeholder-only node spheres: they evaluate each node's angular
cone across the conservative outmap height range, including exaggerated land
and runtime microrelief. Selection can still over- or under-refine because its
error term is not derived from baked terrain roughness.

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
- Below 100km camera altitude, terrain beyond 2km additionally blends toward
  analytic local sky radiance, reaching full fog at 60km for grazing
  terrain-to-camera rays. Steeper ground views remain clear. This softens the
  low-flight terrain/sky horizon without changing ocean or orbital views.

### Atmosphere

`atmosphere.wgsl` is a 16-sample fullscreen raymarch. Terrain aerial
perspective is computed per vertex in `planet.wgsl`; ocean recomputes it per
fragment. Current shared constants are:

- atmosphere shell: 720 km;
- top-edge density fade: 480 km;
- Rayleigh scale height: 36 km;
- Mie scale height: 4.8 km;
- Rayleigh coefficient: `(5.8, 13.5, 33.1)e-6 / m`;
- Mie coefficient: `0.5e-6 / m`;
- Mie g: 0.76;
- visual solar radiance: 1.25;
- HDR-off anti-solar twilight minimum scatter: 0.48, measured at 1.538x
  solar/anti-solar display luminance.
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
- visual size multiplier: 0.3x, about 0.159 degrees apparent diameter (one
  tenth of the former presentation);
- corona radius: eight visual radii;
- camera-only radiance multiplier: 5x;
- HDR core: `(72, 65, 52)` before the visual multiplier;
- HDR halo: `(6, 5.5, 4.5)` before the visual multiplier.

Sun-disc angular distance uses `atan2(length(cross(ray, sun)), dot(ray, sun))`
rather than `acos(dot)`, retaining sub-microradian separation near alignment.

It changes appearance only, not scene illumination.

Verified on 2026-07-15: `cargo check --workspace` passed and `stare_at_sun`
passed all five exposure/capture assertions after the diameter reduction.

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

`planet.rs` owns the committed startup switches `BLUR_ENABLED`,
`BLOOM_ENABLED`, and `HDR_EFFECT_ENABLED`; all three are `true` at `d3ccdf4`.
F6/F7/F8 change the live renderer state without recompiling. Blur is one 5x5
full-resolution HDR pass. Bloom independently extracts HDR values above 1.0,
applies the same 5x5 kernel, and composites the result at 0.75 over either the
original scene or the standalone-blurred scene. Disabling bloom skips both its
bright-pass and composite passes; disabling blur does not disable bloom.

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
| `twilight_directionality` | Solar/anti-solar twilight | 2 s | 2 | >=1.5x solar/anti-solar sky luminance |
| `night_side_atmosphere` | Occlusion | 2 s | 1 | dark sky and >=5x day/night surface ratio |
| `limb_atmosphere` | Orbital limb | 1 s | 1 | finite/count/seam/fallback; still visual-only |
| `ground_to_orbit` | Sky-to-space/HDR | 8 s | 7 | continuous sky luminance and stable bounded exposure |
| `stare_at_sun` | Exposure response | 4 s | 3 | bounded, smooth, non-oscillating exposure |
| `ocean_flyover` | Gerstner ocean | 6 s | 5 | >=0.5 m mirrored wave-height range |
| `orbital_zoom_lod` | Fixed-orbit optical zoom | 14 s | 5 | exact L2->L18->L2 sequence, no budget pressure/thrash, seam/fallback bounds |
| `polar_ice_cap` | Polar ice presentation | 1 s | 1 | bright, sufficiently neutral center pixel plus finite/count checks |

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
GPU samples report scene, luminance, camera-only sun overlay, blur, bloom,
tone-map, and egui separately; their sum excludes present. FIFO frame pacing
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
| 2 | Complete | Six quadtrees, skirts, transitions, L2/2.0px SSE selection, and the exact L2-L18-L2 zoom ladder are reconciled |
| 3 | Complete | Deterministic full baker and preview/raw export |
| 4 | Complete | Runtime outmap streaming, height displacement, biome materials |
| 5 | Complete | Analytic atmosphere, aerial perspective, transition scenarios |
| 5.5 | Complete | Sun, HDR target, luminance chain, exposure, ACES |
| 6 | Complete | Gerstner ocean, reflection, Fresnel, ocean scenario |
| 7 | In progress | Implementation and clean objective regression are complete; final human visual sign-off remains |

## Verification snapshot

Latest clean-HEAD checks on 2026-07-16 at `27ebd43`, using the separate
`CARGO_TARGET_DIR=/home/dad/.cache/citg-target-a47112c`:

- `cargo fmt --all -- --check`: passed.
- `cargo check --workspace`: passed without warnings.
- `cargo test --workspace`: passed: 76 app tests, 22 baker tests, and 5
  coretypes tests.
- All 12 named outmap scenarios passed from the same clean HEAD. Run IDs:
  `still_5s` `1784222988-47173`, `orbit_once` `1784222993-47317`,
  `descent_to_10m` `1784222998-47430`, `sunset_sweep`
  `1784223011-47623`, `twilight_directionality` `1784223019-47771`,
  `night_side_atmosphere` `1784223022-47882`, `limb_atmosphere`
  `1784223025-47994`, `ground_to_orbit` `1784223026-48092`,
  `stare_at_sun` `1784223035-48239`, `ocean_flyover`
  `1784223040-48367`, `orbital_zoom_lod` `1784223046-48499`, and
  `polar_ice_cap` `1784223061-48692`.
- The clean `orbital_zoom_lod --terrain outmap` replay passed all eight
  configured assertions. It observed the exact L2-L18-L2 ladder, no thrash,
  at most 256 resident chunks, at most 248 fallback chunks, eight or fewer
  actual GPU builds per frame, and a maximum seam delta of
  `0.00000762939453125m`.
- Clean profile run `still_5s` `1784223073-48883` passed and emitted 11 GPU
  samples. Every sample contained all seven stage fields: scene, luminance,
  sun, blur, bloom, tone-map, and egui. The first/last total GPU samples were
  1.449/1.852ms on the current local path.
- Temporary deterministic low-flight profiles bounded the former unbounded LOD
  workload: one settled stationary replay measured 11.3ms median / 12.0ms max
  GPU render at 254 draws, and a moving replay reached L18 with at most eight
  actual GPU chunk builds per frame and a 17.6ms maximum sampled frame.

The old 80/80 optical-zoom result predates the Phase 7 experiment and is useful
historical evidence only; the clean results above are the current branch
baseline.

The current 15-second rotation, 40x terrain exaggeration, and disabled startup
post effects are included in the passing unit suite and all-scenario
regression.

The subsequent Mach 300 WASD and low-altitude terrain-fog change passed
`cargo fmt --all -- --check`, `cargo check --workspace`, and all workspace
tests (77 app, 22 baker, 5 coretypes). Focused outmap replays also passed:
`ground_to_orbit` `1784231212-111929`, `night_side_atmosphere`
`1784231220-112074`, `descent_to_10m` `1784231173-111541`, and
`sunset_sweep` `1784231223-112184`.

## Working-tree safety snapshot

At the start of the LOD reconciliation, `git status --short` was clean. Commit
`e6e7a4c` had already committed the following user visual experiments and
reference captures:

```text
capture-001.png through capture-005.png
flying.png
crates/app/src/planet.rs
```

The PNGs are user/reference captures. The committed `planet.rs` experiment
changes blur/bloom/HDR startup defaults from `true` to `false`, rotation from
600s to 15s, and terrain height exaggeration from 4x to 40x. The LOD work did
not revert those values. Re-run `git status --short`; this section is a
snapshot, not permission to delete or replace the captures.

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
4. SSE still uses placeholder geometric-error rather than measured per-tile
   baked error. Distance, horizon, and frustum evaluation no longer use
   placeholder-only world-space spheres: they conservatively test each node's
   angular cone across the complete exaggerated radial height range. This fixes
   elevated near-field holes and keeps 4x land near the camera from being
   treated as if it were still at the unscaled baked radius, without the
   leaf-budget explosion caused by the failed blanket sphere margin.
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
12. Objective Phase 7 validation is complete, but the current visual
    presentation still needs the human sign-off listed under Next action.
13. The current 40x visual terrain exaggeration gives every outmap node a
    conservative possible height range up to roughly 364km. Telescope zoom
    therefore reaches the 256-leaf ceiling and can render through as many as
    248 resident ancestors while the eight-build queue catches up. The focused
    scenario verifies this remains bounded; per-tile height bounds are the
    proper future tightening mechanism.

## Phase 7 implementation chronology and evidence

The 640x427 manual captures from run `1784105961-158941` showed kilometre-scale
rectangular biome regions and repeated terrain ribs at about 12.45km zero-datum
altitude. Their logs confirm the primary colour-block cause: 44-51 of roughly
50-57 visible chunks were sampling ancestor tiles because dense baked material
coverage ends at L4 outside the sparse +X corridor. That limitation remains;
4x vertical exaggeration cannot invent finer categorical biome data.

Positive baked land height and global microrelief are now multiplied by 4x for
rendering, normals, atmospheric surface altitude, CPU flight clearance, radial
LOD bounds, and SSE distance. Classification still uses unscaled macro height,
so coastlines, beaches, biomes, ocean sea level, and Gerstner waves do not move.
The focused exaggeration/shoreline and two low-flight shell regressions pass,
`cargo check --workspace` is warning-free, and rendered run
`test-runs/polar_ice_cap/1784106809-165655` passed all three assertions at
640x427. The full dirty-working-tree suite has the same five known LOD-policy
failures plus the sun-motion expectation affected by the user's separate
uncommitted rotation-period override; none is introduced by height scaling.
The next human check should repeat the same low-flight route. If rectangular
colour blocks remain unacceptable, the next fix is higher-resolution global
material coverage or continuous procedural material breakup, not more height.

The default window now requests a 640x427 physical-pixel render surface instead
of a 960x640 logical window. This preserves the previous 3:2 composition to the
nearest whole pixel, reduces the unscaled default pixel count by 55.5%, and
prevents a high-DPI scale factor from silently increasing the requested render
resolution. Scenario captures therefore use the same 640x427 surface unless
the window is explicitly resized.

Verified on 2026-07-15: `cargo check --workspace` passed, and the rendered
`polar_ice_cap` run at `test-runs/polar_ice_cap/1784105963-159411` passed all
three assertions and wrote a 640x427 PNG.

An accidental 60,000-second axial period made the world-space sun appear
stationary from the planet-relative flight camera: the newest manual log showed
only 0.027° of rotation in about 15 seconds. The documented/original
600-second base period is restored. Interactive mode retains its explicit 0.3×
time scale, yielding 2.7° of relative sun motion per 15 seconds; deterministic
scenarios retain their authored time scales. A focused regression pins this
interactive rate so it cannot silently become imperceptible again. F10 still
intentionally freezes all scene animation time while preserving low-flight
camera navigation for frozen-frame composition work.

Verified on 2026-07-14 with
`test-runs/manual/1784067862-68133/log.jsonl`: the normal interactive render
path advanced the planet-relative sun angle by 3.049° over 16.939 seconds.
`cargo check --workspace` and the focused sun-motion regression pass. The full
workspace suite runs 66 app tests: 61 pass and the same five LOD-policy
mismatches fail (`orbit_selection_stays_coarse_and_bounded`,
`quadtree_children_tile_the_parent_node`,
`two_kilometer_selection_stays_below_finest_lod_and_budget`,
`maximum_zoom_reaches_every_lod_at_any_viewport_height`, and
`orbital_zoom_scenario_keeps_the_full_ladder_in_a_short_viewport`).

The flight camera no longer advances automatically around a latitude parallel.
Its planet-local position remains unchanged when no WASD key is held. W/S move
exactly along/opposite the mouse-controlled view vector at 102,090m/s (Mach 300),
A/D strafe along camera-right/left, and diagonal input is normalized. Movement
may climb or descend because pitch is part of the view vector, but an endpoint
clamp retains at least 5,000ft clearance above the highest resident CPU-sampled
terrain surface. Key releases are processed even when egui consumes keyboard
input, and focus loss clears held movement state to avoid a stuck camera.

Manual run `1784065154-40230` exposed large quadtree-shaped holes near the
Mach-10 camera over roughly 3,012m baked terrain. The captured camera regression
proved that a +Z root could be culled while its descendant intersected the
visible elevated shell: placeholder-only world-space bounds did not contain
the baked surface. `TerrainHeightRange` now comes from the active outmap
manifest, expands the positive limit by global microrelief and the 4x visual
land scale, and leaves the conservative below-sea bound unscaled.
Horizon/frustum tests maximise each plane over the node's angular cone and
radial range rather than inflating every fine node tangentially. SSE distance
uses that same angular/radial shell. The exact captured view and the general
low-flight ray-coverage test both pass without leaf-budget pressure;
`polar_ice_cap` also passes as a rendered wgpu smoke regression. Human replay
of the same free-flight route remains the final visual sign-off.

The low-flight detail experiment now covers the whole planet rather than a
pre-baked corridor. `planet.wgsl` layers four bounded, direction-based relief
bands (about 6km through 12m wavelength) over baked land above the coastline
and uses the same value for modest material variation. It costs four sine
evaluations per height sample, with detail interpolated to the fragment stage
instead of recomputed per pixel. `planet.rs` mirrors the exact field, and
`TerrainRenderer::surface_height_meters_at` applies it to the highest resident
tile, multiplies positive land plus microrelief by 4x, and resolves below-sea
samples to sea level. The shader mirrors the same ordering so biome/beach
classification and coastlines remain based on unscaled baked height. Focused
CPU tests cover the amplitude, coastline/ocean behavior, exaggeration, and
low-flight screen-ray coverage.
`OUTMAP_TERRAIN_HEIGHT_SCALE` now has one source of truth in `planet.rs` and is
uploaded through the terrain-settings uniform. The vertex displacement and
displaced-normal samples therefore use the same value as CPU camera clearance,
culling bounds, and screen-error distance; do not restore a separate WGSL
constant.
The mismatch was reproduced with a local 40x CPU value while WGSL remained at
4x: low-flight clearance followed the taller CPU surface but the rendered mesh
did not. A local 40x `polar_ice_cap --terrain outmap` run now completes and
visibly uses the stronger displacement. Exact-staged-tree formatting and
workspace check pass; the app suite runs 71/76 with only the same five known
LOD-policy failures, and the staged 4x `polar_ice_cap` smoke passes all three
assertions. Human low-flight sign-off at the local 40x value remains useful.
The next visual check should fly away from +X and compare terrain readability
and frame-stage profile samples before changing amplitudes or frequencies.

Manual run `1784106903-166364` exposed a separate LOD performance failure while
zooming the low-flight camera. At 2.236 degrees vertical FOV the selector grew
to 1,277 active chunks, retained as many as 1,664 terrain draws, created 425
chunks in one sampled frame, and still reported `budget_limited=false`. Its
sampled frame-time p95 was 110ms and the worst sample was 704.9ms (1.42 FPS).
Almost every selected leaf used the same coarse ancestor material tile, so the
cost did not improve the blocky material presentation.

The one-draw-call-per-chunk renderer now has a production leaf ceiling of 256
rather than 2,048. The priority queue still sends the budget to the nearest,
deepest visible demands, while unsplit parents retain complete coarse coverage.
Cross-fades remain enabled for small adjustments, but a change above 64 loaded
plus unloaded nodes, or one that would retain more than 64 fading chunks,
snaps to the complete active topology. This avoids both transition holes and
an unbounded half-second tail of obsolete draw calls.

Two temporary deterministic profile replays validated the guard and left the
tracked scenario definitions unchanged. Run `1784107791-178486` reproduced the
captured 2.236-degree stationary view at 254 chunks/draws: settled GPU render
was 11.3ms median and 12.0ms maximum, with a one-time first-frame CPU/upload
hitch. Run `1784107849-179134` moved the same low camera about 10km/s at 60
degrees, reached L18, and recorded a 17.2ms maximum spatial sample; its sampled
draw count stayed at or below 138. These short replays prove the former 1 FPS
failure is bounded, not that long free-flight performance or visual quality
has final human sign-off.

The remaining rectangular colour regions are not a leaf-count problem. Dense
global material data still ends at L4 outside the sparse +X bake, so more
geometry repeatedly samples the same categorical source. The next material
quality step needs higher-resolution global data or continuous procedural
material breakup. Independently, replacing per-node vertex buffers/draws with
a shared procedural grid and batched instances is the next structural terrain
performance improvement; do not raise the leaf ceiling to hide either issue.

Manual run `1784108282-187260` then showed that the 256-leaf ceiling alone did
not bound movement cost: at Mach 30 it selected and synchronously built as many
as 183 chunks in one sampled frame, causing 150-182ms frames. `TerrainRenderer`
now keeps one root chunk per face as complete fallback coverage, builds at most
eight nearest parent-to-child descendants per frame, and retains up to 512
recent GPU chunks. A desired leaf that is not ready renders through its nearest
resident ancestor instead of leaving a hole. The `chunks_loaded` spatial metric
now means actual GPU chunk builds rather than requested LOD leaves.

The deterministic 10km/s replay `1784109581-193023` reached L18 while keeping
actual chunk builds at or below eight per frame. Its sampled frame maximum was
17.6ms, GPU render median/maximum were 9.53/12.16ms, and draw calls topped out
at 60 while descendants streamed in. Run `1784109783-194021` additionally
verified the fallback-detail presentation after the same movement loop at a
16.95ms sampled-frame maximum. This is a bounded-degradation result: motion is
responsive and complete coverage remains, but unbuilt regions intentionally
remain coarser until the camera stops long enough for the queue to catch up.

The repeated ribbed pattern in low-flight captures was procedural relief being
sampled by a coarse 33x33 fallback mesh. A shared L8-L11 weight was insufficient
because it introduced all four frequencies together, including wavelengths the
mesh could not resolve. `planet.wgsl` now fades each octave in separately around
its first resolvable level: L8, L11, L14, and L17. Displacement and normals use
the same octave weights, while the CPU clearance/culling shell retains the
conservative full 40x field; this visual fade cannot make the camera enter
terrain.

The low-altitude sunset sky was separately dark because `atmosphere.wgsl`
estimated solar transmittance from each dense raymarch sample to the top of the
720km shell using endpoint-average density. At low solar elevation that erased
all direct sky illumination, while ocean reflection used the already-correct
local scale-height air-mass approximation. The fullscreen sky now uses that
same bounded column and solar visibility, preserving warm attenuation without
turning the ground-level horizon black. It requires visual sign-off with a
frozen F10/F9 sky-only capture; do not compensate by raising terrain light,
`AERIAL_IN_SCATTER_GAIN`, or global sky brightness.

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

F10 freezes interactive scene time before a diagnostic capture set, keeping
planet rotation, ocean phase, and exposure fixed while F9/F12 are used.
Low-flight WASD movement and mouse-look continue using wall-frame time so a
frozen world can still be framed from another camera position. The
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

The visual sun disc/corona is now composited in a depth-equal background-only
HDR pass after luminance metering, rather than in the physical scene pass.
This preserves terrain occultation and bloom while keeping camera-only glare
out of auto-exposure: when the disc falls behind the horizon, the meter no
longer jumps from its overbright halo to dark terrain and spuriously brightens
the fading sunset sky. The GPU timestamp profile reports this pass separately
as `gpu_sun_ms`.

The atmospheric shadow penumbra now begins at the geometric limb and extends
only into the planet shadow. Previously its wide anti-banding interval was
centred on the limb, so sky and aerial samples fell immediately from full sun
to roughly half strength when their solar direction crossed the local tangent
plane. Keeping unoccluded air fully lit preserves a bright orange horizon at
sunset, while the existing shadow-side taper carries progressively redder
single scattering into twilight without changing direct terrain lighting.

The twilight solar column now starts at 8x air mass at the geometric horizon
and reaches the existing 12x red-extinction column by about 7° solar depression;
the shadow-side taper base is 36 km rather than 24 km. This delays the dark sky
without adding samples or a global brightness multiplier: sunset stays brighter
and orange, then reddens as the sun descends before fading to night. The shared
Mie coefficient is a still-conservative `0.5e-6/m` (far below the original
physical-order value), restoring a modest forward lobe. That lobe alone was
not enough to overcome Rayleigh's symmetric forward/back phase at sunset: a
paired-direction render measured only 1.074x solar/anti-solar luminance. The
fullscreen sky now applies a bounded twilight-only back-hemisphere weight,
fading from unchanged at 90° to 0.55 directly anti-solar while the sun drops
below about 14° elevation. This approximates the missing directional contrast
from the rising planetary shadow and higher-order atmospheric transport without
changing the sunset-facing sky, terrain/ocean lighting, or sample count.

`twilight_directionality` captures the same 100m-altitude sky first toward and
then away from a fixed low sun before exposure can materially adapt. The new
weight raises its luminance ratio from 1.074x (`[110, 50, 1]` versus
`[101, 47, 1]`) to 1.739x; the solar sample is unchanged and only the
anti-solar sample falls to `[66, 28, 0]`. Removing the spacing-dependent shadow
taper during diagnosis produced exactly the same 1.074x baseline, ruling that
taper out as the source.

Verified on 2026-07-15 from the exact staged tree: `cargo check --workspace`
passed; the app suite ran 69/74 tests with only the five already-documented LOD
policy expectation failures. `sunset_sweep` passed all seven assertions and
its four sampled sky colours progressed from `[166, 183, 139]` through warm
`[157, 114, 19]` / `[184, 125, 8]` to twilight `[90, 10, 0]`.
`still_5s --profile-render` also passed and emitted finite `gpu_sun_ms` samples
around 0.13-0.14 ms on the Xvfb software path, proving the expanded timestamp
layout and post-meter sun pass execute without validation errors. The latest
manual 18-frame sequence visually confirmed the exposure rebound is gone;
human sign-off remains needed for the latest anti-solar twilight contrast.

Latest exact-staged-tree verification on 2026-07-15: formatting and
`cargo check --workspace` passed. The app suite ran 70/75 tests;
only the same five documented LOD-policy expectations failed.
`twilight_directionality`, `sunset_sweep`, and `night_side_atmosphere` all
passed every assertion from the staged 600-second rotation baseline. The
sunset-facing sequence retained its previous four sky samples, confirming the
new weight does not globally darken or shorten the tuned sunset.

The bounded polar slice is implemented in `polar_ice_cap`: baked Ice overrides
ocean at the poles, receives a cool diffuse floor gated by the actual surface
irradiance, and a center-pixel assertion checks that the daylight cap is bright
and sufficiently neutral. The gate reaches zero with direct and atmospheric
illumination, so snow has no emissive night floor. The focused scenario passes.

The LOD policy was reconciled on 2026-07-16 by restoring the L2 floor and
2.0/1.25px split/merge hysteresis. Tests now pin those exact defaults. The
2km placeholder regression was corrected to require L18 because that camera is
inside the conservative +/-3.503km terrain shell, while the captured narrow
flight regression now correctly expects the restored policy to remain below
the leaf ceiling. The 256-leaf and eight-build budgets were not raised.
`cargo fmt --all -- --check`, `cargo check --workspace`, and all workspace
tests passed. Rendered outmap run `1784222171-26736` traversed the exact
L2-L18-L2 ladder, reached 256 resident chunks and 248 ancestor fallbacks, had
zero thrash, and passed its bounded assertions. Do not describe Phase 7 as
fully regressed until every named scenario completes from one clean HEAD.

The first clean-HEAD scenario sweep at `a47112c` passed 11 of 12 scenarios.
Only `twilight_directionality` failed: the committed HDR-off presentation
measured 1.428x solar/anti-solar display luminance against the existing 1.5x
criterion. The isolated twilight-only anti-solar minimum was tightened from
0.55 to 0.48; it leaves the solar-facing sky and terrain/ocean paths unchanged.
Focused replay measured 1.538x from `[96, 57, 3]` versus `[63, 37, 1]`.
`sunset_sweep` and `night_side_atmosphere` also remained green after the
change, motivating the clean-HEAD complete sweep recorded below.

Clean HEAD `27ebd43` then passed formatting, workspace check/tests, all 12
named outmap scenarios, and `still_5s --profile-render`. The profile emitted
11 samples with all seven GPU stage fields. This completes the objective Phase
7 regression; remaining sign-off is visual and must not be inferred from the
automated results.

Manual near-surface capture `1784230809-107189` showed that the initial
20-140km terrain-fog transition still left nearby skyline ridges sharply
defined. The transition now begins at 2km and reaches full strength at 60km,
but is weighted toward grazing terrain-to-camera rays so it does not blanket
steeper ground views. Focused terrain, ascent, sunset, and night-side scenarios
remain green; the exact interactive camera path is not deterministic, so the
skyline softness still requires a new manual capture rather than being inferred
from those scenario assertions.

Manual flight run `1784231285-112895` then exposed two separate issues. Its
close captures showed high-frequency procedural octaves aliasing into repeated
33x33-grid spikes as streamed LOD increased; each octave is now introduced only
near its resolvable L8/L11/L14/L17 mesh level. Night capture `capture-001.png`
also exposed the unconditional ice colour floor, which is now multiplied by
the real surface irradiance. Workspace formatting/checks and all tests pass
(77 app, 22 baker, 5 coretypes). Focused rendered runs pass:
`night_side_atmosphere` `1784231702-117655`, `polar_ice_cap`
`1784231705-117769`, `descent_to_10m` `1784231707-117877`, and
`orbital_zoom_lod` `1784231720-118055`. The exact frozen-world manual flight
path was not logged after simulation time stopped, so a fresh interactive
capture remains the honest visual verification for the LOD transition.

Follow-up manual run `1784231904-119973` confirmed the snow lighting fix to the
user's satisfaction, so night snow is visually signed off. It also showed that
per-octave gating alone did not remove the dominant low-flight sheets and teeth:
their straight chunk-edge walls identify the remaining cause as coarse skirts.
Generated skirts remain 7.5% of chunk edge length but are now capped at 50m.
The full workspace suite passes; rendered seam/LOD runs `orbit_once`
`1784232186-123405` and `descent_to_10m` `1784232190-123529` pass. An
`orbital_zoom_lod` attempt `1784232380-125013` reached simulation time 4.017s
without an assertion failure but hit the 300s wall timeout before completion,
so it is not recorded as a pass. A fresh low-flight capture is still required
to verify that the exposed sheets are gone.

Manual capture `1784236902-158013` on the 2,000m cap still showed a large
foreground skirt wall at 10,491m camera altitude. The diagnosis remains
unchanged, but the bound was visually much too deep; it is now 50m, retaining
shallow crack coverage without creating a terrain-sized face. This narrower
bound passes the full workspace suite plus rendered `orbit_once`
`1784237068-160253` and `descent_to_10m` `1784237072-160372`; its subsequent
manual confirmation is recorded below.

Manual run `1784237334-162486` visually confirms the 50m cap removed the
dominant kilometre-scale vertical skirt sheets. The broad terrain, distant
silhouettes, haze, and scale read clearly. Remaining close-range issues are now
separate: the procedural field forms repeated smooth knife-ridges, some
chunk/LOD boundaries read as dark slits or abrupt shape changes, and nearby
lighting can become heavily green/cyan. No further renderer change was made
from this review; treat those as the next terrain-presentation decisions rather
than continuing to tune skirt depth.

## Next action

Obtain final human sign-off before promoting `experiment/composition-debug` to
`main`:

1. Decide whether the next terrain pass should first reduce/reshape runtime
   microrelief or isolate the remaining dark chunk/LOD boundary slits. The 50m
   skirt cap itself is visually confirmed.
2. Confirm the 2-60km terrain fog removes the low-flight horizon line without
   obscuring too much nearby terrain.
3. Review sunset and the 1.538x solar/anti-solar twilight contrast.
4. Toggle F6/F7 and HDR in an interactive capture set to approve blur, bloom,
   and the current HDR-off startup presentation.
5. If those views are accepted, promote the branch to `main`; otherwise make
   only the specifically requested visual adjustment and rerun its focused
   scenarios.

Read first: the verification snapshot and current visual-experiment notes
above. Preserve the user's committed constants and reference captures unless
explicitly asked to change them.

## Longer-term follow-ups after the next action

These are separate tasks, not permission to scope-creep:

1. Store per-tile geometric error and height bounds in the outmap manifest and
   drive SSE/culling from baked data.
2. Add tier-2 limb/terminator image assertions.
3. Decide whether to batch/indirect terrain draws without changing tile binding
   behavior.
4. Replace the remaining per-chunk vertex buffers/draw calls with a shared
   grid and batched instances without changing tile-binding behaviour.
5. Replace synchronous tile I/O/upload with a bounded asynchronous streaming
   path after the current LOD contract is stable.
6. Add a retained profiling fixture so timestamp-layout regressions are
   objectively checked rather than depending on temporary run artifacts.

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
- Branch: `main`
- Remote branch: `origin/main`
- Implementation baseline reviewed: `75b5d1d` (`Double atmospheric scattering intensity`)
- Last full source review: 2026-07-13
- Current phase status: Phases 0 through 6, including 5.5, are complete.
- Remaining planned phase: Phase 7, polish and final regression.
- Current bounded engineering issue: orbital optical zoom jumps from HUD LOD 2
  directly to HUD LOD 4 instead of traversing LOD 3.

The commit containing this document follows the implementation baseline above
and changes documentation only. Always use `git log -1 --oneline` and
`git status --short` rather than assuming this snapshot is still HEAD.

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

- The normal outmap path never generates baked terrain noise or erosion at
  runtime; the baker owns that work.
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
- default auto-orbit speed: `0.4 rad/s`
- mouse-look sensitivity: `0.0006 rad/pixel`
- visible HUD refresh: 100 ms
- hidden HUD refresh bookkeeping: 500 ms
- GPU timestamp readback ring: 3 slots
- presentation: FIFO, desired maximum latency 2

Per-frame flow:

1. Collect completed GPU timestamps and luminance readbacks.
2. Advance fixed scenario time or interactive elapsed time.
3. Apply scenario pose/sun or automatic azimuth orbit.
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
- default elevation: 20 degrees;
- FOV default: 45 degrees; clamp: 2 to 75 degrees;
- mouse look is a yaw/pitch offset in the local frame whose forward/down vector
  always points from the camera toward the planet;
- the mouse wheel changes FOV only and never moves the camera;
- interactive mode auto-orbits while retaining the mouse look offset;
- planet rotation period is 600 simulation seconds.

Controls:

| Input | Effect |
|---|---|
| Mouse motion | Captured, unbounded free look relative to planet-down |
| Mouse wheel | Optical FOV zoom |
| Left/Right arrows | Orbit azimuth by 0.08 radians |
| Up/Down arrows | Orbit elevation by 0.05 radians |
| F3 | Toggle debug HUD |
| F12 | Capture PNG into the current run directory |
| Escape or Q | Quit |

The wheel is deliberately routed before egui consumption. Focus changes grab
or release the cursor. `ControlFlow::Poll` keeps interactive rendering display
paced; do not reintroduce the old idle 10 FPS scheduling bug.

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

The HUD prints `TerrainStats::max_level`, the highest active leaf level. It is
not the only active level and it is not the complete histogram. Mixed levels
can coexist while the HUD shows a single number. The full 19-slot histogram is
present in spatial JSONL records.

### Known orbital zoom defect

`LodPolicy::minimum_level_for_view` currently has a binary rule:

- vertical FOV greater than 8 degrees: minimum active level L2;
- vertical FOV at or below 8 degrees: minimum active level L4.

At the default orbital distance, screen-space error usually leaves the planet
at L2 until this rule fires. The selector can recurse through L3 and produce L4
in one update, so L3 never has to become an active leaf. The 0.5-second dither
fade softens the L2-to-L4 replacement but does not create an L3 stage. This is
why zooming from space visibly alternates between HUD values 2 and 4.

Intermediate/high levels are otherwise used: at a 2 km altitude, a test bounds
the maximum selected level between L9 and L13; at 10 m, a test reaches L18. The
immediate fix should be a staged, hysteretic L2/L3/L4 optical-zoom ladder, not
forcing levels above globally available data.

### LOD transitions and GPU representation

`TerrainRenderer` keeps parent and child render nodes together for 0.5
simulation seconds. `planet.wgsl` uses a 4x4 Bayer discard pattern to dither
between them. Skirts remain active. All chunks share one index topology.

Chunk vertices are static anchor-local f32 data. Each chunk anchor is f64; a
per-frame instance carries the f32 camera-relative anchor. Rendering currently
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
- Surface direct sunlight has a 5x artistic scale.
- Terrain receives a small blue-weighted analytic sky-diffuse fill.
- Fully occulted direct and sky contributions become zero.

### Atmosphere

`atmosphere.wgsl` is a 16-sample fullscreen raymarch. Terrain aerial
perspective is computed per vertex in `planet.wgsl`; ocean recomputes it per
fragment. Current shared constants are:

- atmosphere shell: 360 km;
- top-edge density fade: 240 km;
- Rayleigh scale height: 36 km;
- Mie scale height: 4.8 km;
- Rayleigh coefficient: `(5.8, 13.5, 33.1)e-6 / m`;
- Mie coefficient: `21e-6 / m`;
- Mie g: 0.76;
- visual solar radiance: 4.0, doubled from 2.0 in `75b5d1d`.

The raymarch excludes space before atmosphere entry from optical depth and uses
a density/spacing-aware penumbra for smooth directional occultation. Deep dark
side samples receive no leaked direct in-scattering. The 4.0 scale changes only
visible atmospheric in-scatter; it does not change extinction or direct surface
light.

### Sun

`sun.wgsl` draws an additive camera-facing angular disc:

- physical angular radius: 0.004625 radians;
- visual size multiplier: 3x, about 1.59 degrees apparent diameter;
- corona radius: eight visual radii;
- camera-only radiance multiplier: 5x;
- HDR core: `(72, 65, 52)` before the visual multiplier;
- HDR halo: `(6, 5.5, 4.5)` before the visual multiplier.

It changes appearance only, not scene illumination.

### HDR and exposure

`HdrRenderer` owns the `Rgba16Float` scene, luminance mip chain, triple-buffered
1x1 readback, exposure uniform, and ACES pass.

- exposure key: 0.18
- adaptation speed: 1.5
- minimum exposure: 0.05
- maximum exposure: 4.0
- tone map: Narkowicz ACES fitted approximation

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
- chunk/tile load, unload, resident, fallback, draw, seam, and thrash metrics;
- frame time, exposure, and ocean wave range.

Exposure records are emitted every rendered frame. Logs store event fields
under `.fields`; filter by `.target` with `jq`.

Screenshots are post-tone-map, post-egui swapchain readbacks. Capture completion
waits synchronously, so capture frames must not be used as normal performance
samples.

### Embedded scenarios

Scenario JSON is compiled with `include_str!`; rebuild after editing JSON.
All use a fixed 1/60 s simulation step. Unless a terrain flag is supplied, the
app uses the default outmap when its manifest exists and placeholder otherwise.

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
| 2 | Complete | Six quadtrees, SSE LOD, skirts, transitions, L18 descent |
| 3 | Complete | Deterministic full baker and preview/raw export |
| 4 | Complete | Runtime outmap streaming, height displacement, biome materials |
| 5 | Complete | Analytic atmosphere, aerial perspective, transition scenarios |
| 5.5 | Complete | Sun, HDR target, luminance chain, exposure, ACES |
| 6 | Complete | Gerstner ocean, reflection, Fresnel, ocean scenario |
| 7 | Not complete | Ice-cap polish, HUD improvements, performance/regression pass |

## Verification snapshot

Verified on 2026-07-13 against implementation content represented by
`75b5d1d`:

- `cargo check --workspace`: passed; 16 warnings.
- `TMPDIR=/home/dad/.cache/citg-tmp cargo test --workspace`: 71 passed,
  0 failed (app 44, baker library 17, baker binary 1, baker integration 4,
  coretypes 5).
- `sunset_sweep`: passed; red/blue growth 15.961, final ratio 11.0.
- `night_side_atmosphere`: passed; sampled dark sky 0.0 and day/night surface
  ratio 626.327.
- `limb_atmosphere`: passed configured assertions; image remains visual-only.
- `ground_to_orbit`: passed; maximum adjacent sampled sky luminance delta
  0.307, 481 exposure samples bounded to 0.05-4.0, no oscillation.
- `stare_at_sun`: passed; maximum per-frame exposure delta 0.0187 across 241
  samples, no oscillation.

The atmosphere scenario manifests record previous HEAD `405b8cc` because they
were run with the final atmosphere change dirty immediately before it was
committed as `75b5d1d`. The tested shader content matches the commit, but the
manifest logger does not record dirty state. Do not represent those hashes as a
clean-checkout replay. The documentation task itself reran the 71 workspace
tests with implementation source matching `75b5d1d`; the pre-existing
`.gitignore` and PNG working-tree changes remained present.

Known warnings are the deprecated `winit` `EventLoop::create_window` and
`EventLoop::run` APIs plus phase-leftover dead code/unused fields. They are not
current test failures.

## Working-tree safety snapshot

At the start of this documentation task, the following pre-existing user state
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

## Known limitations and actionable risks

1. Orbital optical zoom uses the hard 8-degree L2-to-L4 floor described above.
   It has no separate FOV hysteresis and can churn near the threshold.
2. The HUD displays only maximum active LOD, which can be mistaken for the only
   active level. The histogram exists in logs but not in the panel.
3. Outmap detail above L4 exists only around +X. Deep geometry elsewhere often
   samples L4 fallback and provides no new visual information.
4. SSE and approximate bounds still use placeholder estimates instead of
   measured per-tile baked error/min/max height.
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

## Next action

Implement one bounded optical-zoom LOD correction before broad Phase 7 polish:

1. Add a deterministic fixed-orbit FOV sweep that zooms in and back out.
2. Replace the binary 8-degree floor with a staged, hysteretic L2 -> L3 -> L4
   ladder. Do not force orbital levels above L4 until deeper data coverage is
   available for the viewed region.
3. Assert that L3 becomes active, progression is 2/3/4 then 4/3/2, L2 is
   restored after zoom-out, thrash remains bounded, seams remain within 0.1 m,
   and chunk/fallback counts remain bounded.
4. Show the active LOD range or compact histogram in the HUD so `max_level` is
   unambiguous.
5. Rerun the new zoom scenario plus `orbit_once`, `descent_to_10m`, and
   `ground_to_orbit` before committing.

Read first for that task:

- `crates/app/src/planet.rs`: constants, `LodPolicy::minimum_level_for_view`,
  `PlanetLod::split_candidate`, and LOD tests;
- `crates/app/src/terrain.rs`: `TerrainRenderer::update` and `TerrainStats`;
- `crates/app/src/main.rs`: wheel routing and HUD LOD label;
- `crates/app/src/scenario.rs` and `crates/app/scenarios/`: extend deterministic
  scenario data only as much as required for FOV waypoints;
- `crates/coretypes/src/lib.rs`: current outmap coverage contract.

Do not reopen atmosphere, sun, ocean, terrain palette, or baker generation for
this LOD correction unless a regression provides direct evidence. The problem
is currently localized to zoom policy, scenario coverage, and HUD reporting.

## Longer-term follow-ups after the next action

These are separate tasks, not permission to scope-creep:

1. Store per-tile geometric error and height bounds in the outmap manifest and
   drive SSE/culling from baked data.
2. Add tier-2 limb/terminator image assertions.
3. Decide whether to batch/indirect terrain draws without changing tile binding
   behavior.
4. Move synchronous tile I/O/upload behind a bounded streaming queue.
5. Complete Phase 7 ice-cap visuals and HUD polish.
6. Run every named scenario from a clean HEAD and record the regression result
   here.

# Planet simulator handoff

**Date of handoff:** 19 July 2026  
**User's working repository:** `~/sfasfas/catingard`  
**Current source-only delivery:** `catinthegarden-app-src-v11.1-continuous-low-flight-geometry.tar.gz`  
**SHA-256:** `6d366bafac8e1c68120b6f2c9f3a8e9ca1ff361580ad2d5a8ff5a94b17028591`

## Executive summary

The low-flight geometry and performance failure has been corrected in source and in the local outmap. The deterministic 1.7 km validation view now renders **88 chunks / 202,752 triangles** with no budget pressure, LOD thrash, or seam delta. On the Intel HD 530 it settles around **17 ms/frame**, with the physical scene taking about **12.8 ms GPU**, instead of the former 4–8 FPS. The corrected capture no longer exposes the giant fan triangles.

Near-surface and orbital selection now use the same persistent view-frustum, screen-error quadtree. SSE distance is measured from the camera to the locally sampled terrain surface; the conservative global height shell is used only for horizon/frustum culling. A source-depth cap prevents geometry from outrunning streamed data, and a balancing pass keeps adjacent visible levels within the stitcher's supported two-level gap. LOD fades and interactive telemetry use monotonic presentation time, so F10 cannot freeze transitions or logging.

The existing dense L0–L4 bake was preserved and expanded to **3,252 tiles** with an adaptive sparse corridor and band-limited high-level height detail. Close material projection is 2,048 m and normal sampling scales continuously with camera distance. The remaining gate is a human run on the Quadro M1000M after the pending driver reboot; the application now prefers discrete adapters and reports the selected GPU in the HUD/log.

### Current validation

- `cargo test --workspace`: 134 tests passed.
- F4 flight uses stateful acceleration rather than fixed-speed translation. Holding any WASD direction doubles acceleration every 0.75s from 50m/s², capped at 4,000km/s² and 8,000km/s; Shift applies 4× acceleration. Releasing every movement key halves speed every 80ms, resets the acceleration ramp, and briefly coasts along the last direction. Current speed is visible in the debug HUD.
- `low_flight_performance` run `1784453186-430747`: passed all tier-1 assertions; maximum 88 chunks, 36 fallbacks, zero thrash, zero seam delta.
- Reproduce on Intel: `env WGPU_ADAPTER_NAME=Intel target/release/catinthegarden-app --scenario low_flight_performance --profile-render`.
- Reproduce on Quadro after reboot: `__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia RUST_BACKTRACE=full cargo run --release --bin catinthegarden-app`.
- Regenerate only the sparse refinement without rerunning the full erosion bake: `cargo run --release -p catinthegarden-baker -- --refine-existing assets/outmaps/test-planet`.

The failure measurements below are retained as historical baseline, not current state.

## What the user wants

The simulator should support a continuous trip from orbit to near the surface of a procedural/baked planet. Near the surface it should:

- look like credible terrain, not one huge triangle or a quilt of flat square slabs;
- keep LOD changes stable as the camera moves or rotates;
- avoid cracks, holes, spikes, vertical walls, and sudden “everything became detailed” events;
- maintain usable performance at 1920×1080;
- put the F4 inspection camera over useful land, not empty ocean;
- preserve a coherent planet-scale appearance in orbit.

A reasonable first acceptance target is stable **30 FPS or better at the F4 1.5 km view**, with no visible holes or giant level boundaries. Confirm the actual hardware and target with the user before promising 60 FPS.

## Current visible result

The three latest captures are the strongest evidence of the present failure:

| Capture | Altitude | LOD range | Chunks | Fallback chunks | LOD work | FPS |
|---|---:|---:|---:|---:|---:|---:|
| `capture-003(9).png` | 1,524 m | L2–L18 | 243 | 230 | 0 splits / 0 merges | 4 |
| `capture-002(10).png` | 1,524 m | L4–L18 | 236 | 223 | 0 splits / 0 merges | 8 |
| `capture-001(11).png` | 1,524 m | L4–L18 | 236 | 229 | 39 splits / 42 merges / 74 culled | 4 |

All three were captured with:

- animation frozen;
- blur off;
- bloom off;
- final HDR scene selected;
- exposure 4.0;
- 60° vertical FOV;
- 10 m/s inspection camera, Shift for 250 m/s.

The images show broad dark-green/orange planes, straight boundaries, flat or over-smoothed land, and a coarse pale water/shore boundary. Earlier holes are somewhat reduced, but geometry continuity came at a severe performance cost and did not produce credible terrain.

For comparison, previous builds showed this broad pattern:

| View | Typical workload/result |
|---|---|
| 1.5–100 km, unrestricted/high-detail builds | 247–255 chunks, often 1–6 FPS |
| Around 68 km | 255 chunks, 243 fallback chunks, 6 FPS |
| Around 382 km | 16 chunks, about 51 FPS |
| Around 2,010 km | 10 chunks, about 62 FPS |
| Source-capped low-flight experiments | Often 9–21 chunks and 46–49 FPS, but enormous adjacent coarse/fine slabs and holes |
| Pre-v11 land-targeted F4 | About 75–141 chunks and 19–48 FPS, still broad slabs and flat coastlines |
| Current v11/v11.1 low flight | 236–243 chunks, 4–8 FPS |

This strongly suggests workload scales badly with the near-field node/tile set, but the exact bottleneck has not been measured well enough. It may be a combination of duplicate transition draws, one draw per source tile, fragment cost, and sheer node count.

## Important newly identified bug: frozen simulation freezes LOD transitions

This should be the first code-level issue investigated.

The latest screenshots say `Animation: frozen`, but camera movement remains active. Terrain LOD cross-fades use the **simulation clock**, not an unfreezable presentation/render clock:

- `main.rs::interactive_sim_time()` returns a constant `frozen_sim_time` while F10 freeze is active.
- That value is passed into `TerrainRenderer::update` as `sim_time`.
- `terrain.rs` stores `FadingChunk::started_at_sim_time` and fade-in start times.
- `purge_expired_lod_transitions`, `update_lod_transitions`, and `lod_transition_progress` all subtract from that same simulation time.
- Transition duration is 0.5 seconds.

Therefore, if the user moves or rotates the camera while animation is frozen, obsolete fading-out chunks and incomplete fade-ins may never age out. Further camera changes can accumulate presentation state that should have expired. This is a plausible direct contributor to the 4 FPS captures and must be tested before drawing conclusions from them.

**Required fix:** drive LOD transition lifetime from a monotonic presentation clock that does not freeze, such as `started_at.elapsed().as_secs_f64()` or a frame-accumulated render clock. Simulation time should control planet/ocean animation, not renderer housekeeping. An alternative is to snap all terrain transitions to completion whenever simulation is frozen, but the presentation-clock solution is cleaner.

Add a regression test: keep simulation time fixed, change the camera/active-node set, advance presentation time, and verify all fading nodes expire after 0.5 seconds.

Also make the HUD show both active leaves and total drawn terrain instances, including fading nodes. The present `Chunks` figure may not expose accumulated transition draws.

## Architecture snapshot

- Rust 2024 workspace.
- `wgpu` 29.
- `winit` 0.30.13.
- `egui` 0.35.
- `glam` 0.30.
- Workspace crates:
  - `crates/app`
  - `crates/baker`
  - `crates/coretypes`
- Planet radius: 4,000,000 m.
- Cube-sphere quadtree: L0–L18; current rendered range generally begins at L2.
- Canonical chunk mesh: 33×33 vertices / 32×32 quads, with skirts.
- Default maximum active chunks: 256.
- Near-field path below 250 km also uses clipmap-ring logic.
- Outmap channels:
  - R32F height;
  - R8 biome;
  - R8 moisture.
- Default outmap: `assets/outmaps/test-planet`.

## Actual source-data footprint

The current outmap manifest reports:

- working source: 4096×2048;
- dense levels through L4;
- maximum sparse level L18;
- logical tile size 129, stored size 131 with one-pixel gutter;
- height range −5,000 m to 9,000 m;
- sparse landing direction `[1.0, 0.0, 0.0]`;
- 2,172 total tiles;
- dense counts: L0=6, L1=24, L2=96, L3=384, L4=1,536;
- sparse counts: only 9 tiles per level from L5 through L18, a 3×3 chain around the landing direction.

That last point is central. A fixed 3×3 footprint at every level covers less and less physical area as level increases. At L18 a tile is only on the order of tens of metres across. The finest 3×3 region is tiny relative to what a 60° camera at 1.5 km altitude can see.

Most of the F4 view therefore cannot have meaningful L18 source data. Geometry may subdivide to L18, but most fine nodes merely remap and resample a much coarser ancestor tile. More triangles do not create new terrain information.

## The landing site is deliberately flattened

`crates/baker/src/export.rs` currently contains:

- `BAKED_DETAIL_MAX_AMPLITUDE_METERS = 300.0`;
- `LANDING_DETAIL_PROTECTION_METERS = 500.0`;
- baked detail begins at L3;
- microrelief begins at L12 and reaches only about 2 m amplitude.

The baker multiplies high-frequency surface detail by a smooth landing ramp that is zero at the exact landing direction and reaches full strength at 500 m. F4 intentionally places the camera at that landing direction. In other words, the deterministic inspection camera looks directly at a deliberately flattened half-kilometre area, while the highest-resolution tile footprint is itself very small.

This is a major reason the result looks like a smooth slab. Replace the 500 m suppression with either:

- a much smaller safe landing pad, roughly 20–50 m; or
- a selected naturally moderate piece of terrain that does not need broad flattening.

Do not simply remove all safety without checking spawn height and camera collision.

## Runtime detail is currently disabled in the shader

`crates/app/src/planet.wgsl` currently assigns:

```wgsl
let terrain_detail_meters = 0.0;
```

The shader comments say baked tiles now own terrain detail. `terrain_height` uses the sampled macro height and altitude-dependent scale without the former runtime global procedural detail.

However, `crates/app/src/planet.rs` still has CPU-side `global_terrain_detail_meters` code and tests. This leaves an architectural inconsistency: the CPU code suggests global detail exists, while the rendered terrain explicitly disables it.

Earlier attempts at global shader noise caused needle/cone-like mountains, visual instability, or excessive cost. Do not restore that implementation wholesale. If runtime microdetail is used again, it must be:

- deterministic in planet/world coordinates;
- band-limited by projected pixel footprint and source/geometry resolution;
- slope-aware and amplitude-limited;
- continuous across cube faces and tile boundaries;
- included consistently in normals and height queries where needed;
- faded by wavelength bands, not abruptly switched by node level.

The current zero-detail shader guarantees that resampling a coarse ancestor onto fine geometry will still look smooth.

## Current v11 near-field LOD change

The latest source changed `TerrainRenderer::update` in `crates/app/src/terrain.rs` so that:

- placeholder terrain uses unrestricted view-based LOD;
- outmap terrain above 250 km uses `update_for_view_with_up_and_level_limit` with an outmap source-level cap;
- outmap terrain below 250 km uses unrestricted `update_for_view_with_up`.

The helper is effectively:

```rust
fn outmap_geometry_uses_source_level_limit(camera_world: DVec3) -> bool {
    camera_world.length() - PLANET_RADIUS_METERS
        > NEAR_FIELD_LOD_MAX_DATUM_ALTITUDE_METERS
}
```

This was intended to keep near-field geometry spatially continuous even when it had to sample ancestor textures. In practice, it pushed low flight back toward the 256-chunk budget and single-digit FPS without adding source information.

Treat this as a failed experiment, not a design to preserve. It may need to be reverted or replaced after the transition-clock bug is measured.

## What has already been tried

1. **Original quadtree renderer.** Near the surface it could show one enormous triangle, then jump to very high detail after a small camera move. It had cracks, spikes, and severe FPS drops.
2. **wgpu pipeline validation fix.** Terrain settings binding 5 was made visible to `VERTEX_FRAGMENT`, fixing the missing fragment-stage resource error.
3. **Ocean amplitude reduction.** Gerstner amplitude was reduced from the excessively choppy version. Current HUD wave range is roughly 1 m.
4. **Global procedural terrain detail.** Several versions added GPU/global detail. They produced needle/cone mountains or unacceptable cost and were later retired from the shader.
5. **Near-field clipmap and instancing work.** Added camera-centred near-field logic, instancing, background tile loading, F4 low-flight mode, and an F4 dry-land landing direction.
6. **Sparse outmap retargeting.** Re-baked the high-detail chain around dry land and moved F4 there. This fixed the “spawn in open ocean” usability problem but did not produce credible ground detail.
7. **Source-level-plus-two geometry cap.** Limiting geometry to roughly source level + 2 improved FPS but allowed huge adjacent topology differences. The result showed giant slabs, holes, and square boundaries.
8. **Unrestricted near-field geometry (v11).** Removed the source-level cap below 250 km, allowing fine nodes to sample ancestors. Geometry holes reduced somewhat, but low-flight workload rose to 236–243 chunks and 4–8 FPS.
9. **Test/import repair (v11.1).** Added the missing `glam::DVec3` import in the terrain tests. The runtime build had already compiled; the source archive's test module had failed to compile before this repair.

## Build and test status

The user's real repository is `~/sfasfas/catingard`.

The v11 archive produced:

- a successful runtime compile and launch;
- a harmless warning that `OUTMAP_TERRAIN_HEIGHT_SCALE` was unused;
- a test compile failure because the terrain test module did not import `DVec3`.

The v11.1 archive fixes that import. **A complete `cargo test -p catinthegarden-app` result for v11.1 has not been reported, so do not claim the full suite passes.** Run it first.

Earlier failures that were fixed in the current source include:

- the missing fragment visibility for bind-group binding 5;
- test scope/import trouble around `OUTMAP_TERRAIN_HEIGHT_BLEND_START_METERS`;
- two brittle sunlight shader tests that expected exact whitespace;
- a low-flight selector test whose old budget assumption no longer matched the source-limited branch.

Recommended baseline commands:

```bash
cd ~/sfasfas/catingard
cargo clean -p catinthegarden-app
cargo test -p catinthegarden-app
cargo run -p catinthegarden-app
```

Before editing, also record:

```bash
git status --short
git log -5 --oneline
```

There was no Git metadata in the transferred scratch copy, so the handoff cannot state which user changes are committed.

## Files to inspect first

| File | Why it matters |
|---|---|
| `crates/app/src/terrain.rs` | GPU terrain renderer, tile streaming, transitions, batching, LOD branch, draw submission |
| `crates/app/src/main.rs` | Frame orchestration, F4/F10 handling, simulation clock passed to terrain |
| `crates/app/src/planet.rs` | Quadtree selection, near-field rings, CPU height/detail helpers, camera and LOD policy |
| `crates/app/src/planet.wgsl` | Terrain/ocean/material/lighting shader; runtime terrain detail is currently zero |
| `crates/app/src/outmap.rs` | Manifest handling and ancestor tile fallback/remapping |
| `crates/app/src/debug.rs` | Logging; draw calls appear in logs but not clearly in the HUD |
| `crates/baker/src/config.rs` | Dense/sparse level and radius configuration |
| `crates/baker/src/export.rs` | Tile selection, 500 m landing-detail suppression, baked detail and microrelief |
| `assets/outmaps/test-planet/manifest.json` | Actual tile inventory and landing direction |
| `docs/HANDOFF.md` | Prior handoff; useful history, but its phase-success claims should be treated skeptically |
| `AGENTS.md` | Project conventions; do not assume it proves the renderer works |

## Shortest credible route forward

### 1. Establish a trustworthy baseline

At one fixed F4 camera pose, capture four cases:

1. animation running, camera stationary;
2. animation frozen, camera stationary;
3. animation frozen, camera moved/rotated for 10 seconds, then stationary;
4. animation running again after case 3.

For every case record:

- active quadtree leaves;
- total drawn terrain instances, including fade-outs;
- draw calls;
- triangles;
- resident tiles;
- fallback-node count;
- source-level delta histogram (`geometry level - sampled source level`);
- GPU timestamp per pass if supported;
- CPU update and submit time;
- frame time, not only integer FPS.

Inspect the JSONL logs under the test-run output directory. Add missing figures to the HUD so screenshots are diagnostically useful.

### 2. Fix the presentation clock

Move terrain LOD cross-fades and their expiry to a monotonic presentation clock. Confirm that frozen animation no longer retains old transition draws. Re-run the four-case baseline before changing any LOD policy.

### 3. Replace the binary LOD choice with a balanced source-aware quadtree

The current choice is binary and both sides fail:

- source-capped leaves give good performance but giant level discontinuities;
- unrestricted leaves preserve geometric coverage but explode the workload and merely oversample coarse source data.

Use a graded/balanced quadtree instead:

1. Select desired leaves by screen-space geometric error.
2. Limit useful refinement according to available source data and an explicit allowed oversubdivision amount.
3. Run a balancing pass so edge-neighbour levels differ by at most one (or at most two if proven visually safe).
4. Insert intermediate geometry rings around finer regions instead of allowing L6 to touch L16 directly.
5. Let intermediate/fine geometry sample the same ancestor tile with correct UV remapping where necessary.
6. Use skirts only as a small safety measure, not as a substitute for topology balance.

This should prevent both giant topology cliffs and the current 236-node brute-force response. Add tests for neighbour-level balance across cube-face boundaries as well as within one face.

### 4. Redesign the outmap footprint in physical units

A fixed 3×3 sparse block at all levels is inappropriate. Define the coverage required by the F4 camera in metres and derive tile counts per level from it.

A practical hierarchy could be:

- dense global data through L4;
- broader mid-level sparse rings that cover the full 1.5 km-altitude view and surroundings;
- progressively smaller high-level rings near the landing site;
- only a small L18 core where metre/tens-of-metres detail is actually useful.

Before baking, print a table for every level containing:

- metres per tile at the landing latitude/direction;
- tile radius/count;
- total physical diameter covered;
- sample spacing in metres;
- expected camera altitude range served.

Do not bake the whole planet to L18. Do not assume “L18 exists” means the visible area has L18 coverage.

### 5. Make the landing site visually representative

Reduce the 500 m flattening radius to a small pad or pick a naturally safe location. F4 should start high enough to avoid clipping but low enough to judge terrain. A useful validation view should include:

- a coastline or lake;
- a moderate hill/ridge;
- flatter land;
- visible mid-scale structure.

The user should not have to fly hundreds of kilometres at 10 m/s to find a useful test scene.

### 6. Restore detail deliberately

The 4096×2048 global source cannot supply metre-scale terrain everywhere. Choose one of these explicit strategies:

- bake genuinely higher-resolution height data for the physical landing region; or
- add deterministic, filtered runtime microrelief over the macro outmap.

For runtime detail, use several wavelength bands with amplitudes appropriate to scale. Fade each band based on projected footprint and available geometry, and keep displacement/normal evaluation consistent. Avoid sharp ridged noise with hundreds of metres of amplitude at small wavelengths—the previous “forest of cones” result came from treating noise as geometry without adequate filtering or scale control.

### 7. Profile and reduce render overhead

After transitions are fixed and selection is balanced, determine the actual bottleneck:

- If draw-call bound: the renderer currently groups instances by source tile, but distinct tile bind groups still require separate draws. Consider a texture array/atlas or another indexing scheme that allows more tiles per draw.
- If fragment bound: isolate atmosphere, aerial perspective, water, terrain normal reconstruction, and material evaluation with GPU timestamps/debug modes. Do not guess.
- If fill-rate bound: test a temporary lower render resolution to prove it before implementing dynamic resolution.
- If vertex bound: compare total instances/triangles and remove pointless fine geometry that only resamples coarse sources.
- If CPU update bound: time quadtree selection, balancing, tile lookup, buffer uploads, and transition bookkeeping separately.

Blur and bloom are already off in the bad captures, so disabling them is not a solution.

## Acceptance tests

Do not call the issue fixed until all of these pass:

1. **F4 low flight, 1.5 km, 1920×1080:** stable agreed FPS target (initially 30+), no giant slabs, no holes, and clear terrain structure at metre-to-kilometre scales.
2. **Frozen animation camera test:** moving/rotating the camera while F10 is frozen does not accumulate fading nodes or reduce FPS over time.
3. **Small-motion stability:** a slight camera translation or rotation does not change the view from one giant triangle to maximum detail.
4. **Balanced topology:** adjacent rendered leaves differ by no more than the chosen level bound, including across cube faces.
5. **Source consistency:** a node and its children sampling the same ancestor source produce the same macro height along shared positions.
6. **Transition continuity:** no cracks, sudden height changes, or lingering duplicate surfaces during split/merge fades.
7. **Altitude suite:** captures at approximately 1.5 km, 10 km, 50 km, 250 km, 1,000 km, and orbit, each with frame time, draw calls, active/drawn nodes, triangles, and fallback statistics.
8. **Test suite:** `cargo test -p catinthegarden-app` passes in the exact source delivered to the user.
9. **Build provenance:** archive SHA and changed-file list are supplied with every handoff.

## Things not to do again

- Do not raise the chunk limit or simply enable unrestricted L18 near the ground.
- Do not treat finer geometry resampling a coarse ancestor as actual high detail.
- Do not use a source-level+2 cap without a topology-balancing pass.
- Do not add more global fragment/vertex noise before measuring and filtering it.
- Do not tie renderer transition lifetimes to a pausable simulation clock.
- Do not call square slabs or giant straight boundaries “expected fallback.”
- Do not rebake the entire planet at L18.
- Do not optimise only the orbit view; orbit is already much faster than low flight.
- Do not claim tests ran when the available environment lacked Rust or when only the runtime compiled.
- Do not trust a phase-complete checklist over the screenshots and measured frame time.

## Questions the next AI should ask once, early

1. What GPU and driver are being used, and is the app running on the discrete GPU rather than an integrated/software adapter?
2. Is 30 FPS at 1920×1080 the minimum acceptable low-flight target, or is 60 required?
3. Is the intended closest view around 1.5 km altitude, or must the camera reach walking height?
4. Is deterministic procedural microdetail acceptable, or must all displacement come from baked assets?

These answers affect budgets and source design. They do not change the fact that the current 4–8 FPS/slab result is unacceptable.

## Handoff package recommendation

Give the next AI:

1. the **entire repository**, not only `crates/app/src`, because the baker, manifest, and outmap assets are part of the problem;
2. this document;
3. the three latest screenshots;
4. the current v11.1 source archive and SHA above for comparison;
5. fresh output from `cargo test -p catinthegarden-app`;
6. one JSONL performance log from a running-animation F4 session and one from a frozen-camera-movement session.

## Final assessment

The project has a workable foundation—cube-sphere chunks, baked outmaps, ancestor fallback, a low-flight camera, and an orbit-to-surface path—but the current renderer is not close to the requested outcome. The near-field workload is brute-forcing geometry without matching source resolution, the sparse bake covers too little physical area at high levels, the chosen inspection centre is broadly flattened, runtime detail is disabled, and LOD presentation state appears to use a clock that stops while the user can still move the camera.

The next attempt should begin with measurement and the frozen-transition fix, then implement a balanced source-aware quadtree and a physically specified landing-region bake. Anything less is likely to repeat the same cycle of trading holes for FPS or FPS for smooth slabs.

## Latest emergency terrain fix

Manual sunset run `test-runs/manual/1784454154-441407` showed improved orange/red sky, but close terrain at ~16km altitude was still only L2-L6 with visible giant red triangle gaps. The immediate cause was the outmap source-level cap: sparse resolved source tiles stopped geometry refinement even in low flight. `crates/app/src/terrain.rs` now bypasses that source cap below 250km, allowing screen-error-selected fine geometry to draw from cached ancestor tiles while better source tiles stream in. Focused tests `low_flight_lod_is_not_capped_by_sparse_source_tiles` and `terrain_source_level_limit_prevents_empty_refinement`, plus `cargo check -p catinthegarden-app`, pass. Fresh manual capture is still needed.

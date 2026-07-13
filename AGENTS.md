# AGENTS.md

Planet renderer, Rust + wgpu + egui. Read this before doing anything. It's the whole architecture in one page so you don't need the full design doc pasted in every session.

## Version control

-- You are working in active repo with a remote set. After each set of changes you make, following a prompt, commit and push the changes

## What exists now
*(update this section at the end of every session — one line per phase completed)*

- Phase 0 complete: workspace app opens a wgpu/winit window with an egui FPS overlay, dark-grey clear, and rotating salmon-pink triangle.
- Phase 0.5 complete: tracing JSONL runs, F3 debug-overlay toggle, F12 PNG capture, and the fixed-step `still_5s` scenario with tier-1 log/manifest assertions.
- Phase 1 complete: 6-face 32x32 cube-sphere with flat shading, f64 orbit camera and f32 camera-relative upload, arrows for orbit, Esc/Q quit, and seam-checked `orbit_once` screenshots.
- Phase 2 complete: persistent six-face screen-error quadtrees enforce L2 as the coarsest active detail, reach L18 near the surface, render fixed 33x33 chunks with proportional skirts and multiscale sine height, cross-fade matched split/merge chunks over 0.5 simulation seconds, and pass the deterministic orbit-to-10m LOD/seam/thrash assertions.
- Phase 3 complete: deterministic baker runs the full terrain/erosion/hydrology/climate pipeline and exports validated previews plus R32F/R8/R8 guttered tiles. The default test bake uses a 1024x512 working grid, global L0-L4 coverage, baked L3+ height/material detail, and parent-complete sparse +X refinement through L18.
- Phase 4 complete: the renderer streams baked outmap tiles with ancestor fallback, performs camera-relative GPU height displacement and central-difference normals, colors biomes/moisture, and passes the L18 descent tile/LOD/seam regression with at most two fallbacks.
- Phase 5 complete: analytic altitude-aware Rayleigh/Mie single scattering provides per-vertex terrain aerial perspective plus an 8-sample sky/space shell raymarch, with deterministic sunset and ground-to-orbit pixel assertions. The default camera auto-orbits while mouse offsets remain planet-down-relative; the planet rotates in a 600s axial frame. LOD selection is camera-frustum-aware; ≤8° optical zoom uses L4 detail only inside the visible patch, and the bounded tile cache avoids moving-camera texture reuploads.

## Planet constants (test planet)

- Diameter: 8,000,000 m (radius 4,000,000 m)
- LOD range: 200,000 m/triangle (widest) down to 1 m/triangle (closest) — ratio 200,000:1
- Quadtree depth needed: **18 levels** per cube face (2^18 ≈ 262,144, covers the ratio above)
- Chunk grid: 33x33 vertices (32x32 quads) per quadtree node, fixed at every level
- Split/merge threshold: screen-space error ~2px (project chunk's worst-case geometric error at current LOD to screen pixels via camera distance; split above threshold, merge below with hysteresis to avoid flicker)
- Max mountain height: ~9,000 m above sea level (Everest-scale ceiling)
- Ice cap trigger: latitude > ~66° OR altitude above a per-latitude snowline (snowline drops toward poles, roughly linear interpolate from ~5000m at equator to 0m at pole)
- Star: G2V (sun-like), radius 696,000 km, ~5778K surface temp, warm-white color (not yellow)
- Orbit distance: 1 AU (149,600,000 km) — real Earth-Sun numbers, trivially in the habitable zone for this star type
- Sun angular size: ~0.53° (derived from radius/distance above, don't hand-pick it)
- Sunlight treated as directional (no meaningful parallax at 1 AU vs 4000km planet radius) for all lighting math; only the sun disc render needs the real distance

## Non-negotiable architecture decisions

**Precision:** World positions are f64 (`glam::DVec3`), planet-centered. Every frame: subtract camera's f64 world position from vertex data to get small f32 offsets before upload to GPU. Never send raw f64 world coords to the GPU. Never store absolute world position in an f32 anywhere.

**Depth:** Reversed-Z (1.0 near, 0.0 far), infinite far plane. Near clip scales with camera altitude: `clamp(altitude * 0.01, 0.05, 10.0)`.

**Planet mesh:** Cube-sphere, 6 faces, each face a quadtree per the constants above. Split/merge by screen-space error. Skirts on chunk edges to hide LOD cracks (small vertical drop at edge verts, not full geomorphing — cheaper, good enough). Skirt depth: proportional to chunk's world-space size at that LOD level, roughly 5-10% of chunk edge length.

**Terrain data ("outmap"):** Never generate terrain at runtime. Baked once by the `baker` crate into tiled per-face textures, one tile per quadtree node per face:
  - height: R32F, meters, signed (negative = below sea level, for ocean floor)
  - biome id: R8 (enum: ocean, lake, ice, tundra, temperate_forest, temperate_grassland, tropical_forest, desert, mountain_rock, mountain_snow — extend as needed)
  - moisture: R8, 0-255 normalized
  - normal: derive on GPU from height texture (central-difference sample), don't bake — saves texture memory
  Runtime only streams and displaces, never computes noise/erosion live.

**Atmosphere:** No precomputed LUTs. Altitude-aware single-scattering, analytic, based on the Preetham/Hoffman model extended for non-constant density (Nielsen 2003). Density `ρ(h) = ρ0 · e^(-h/H)`, Rayleigh H≈8000m, Mie H≈1200m. Optical depth estimated as average-endpoint-density × path length (no ray integral). Two render paths, same math: (1) terrain aerial perspective computed per-vertex on the existing LOD terrain mesh, interpolated by GPU; (2) sky/space view via a short 4-16 sample single-scattering raymarch through the atmosphere shell — same shader covers ground sky and orbital limb view, just different ray length. Match sky horizon optical depth to terrain far-plane optical depth to avoid a seam. No multiple scattering (keeps it cheap; fake it later with Mie g-parameter/sun-intensity nudge only if needed). Rayleigh coefficient ~(5.8, 13.5, 33.1)e-6/m, Mie ~21e-6/m. Sunset/sunrise colors emerge automatically from Rayleigh wavelength dependence.

**Sun & HDR:** Sun is a camera-facing disc drawn along the sun direction at fixed far depth (always-pass depth test, skybox-style), no LOD, no mesh — its apparent size barely changes across the game's altitude range. Day/night from planet axial rotation, not simulated orbital revolution. Render to HDR float target (Rgba16Float), values can exceed 1.0. Auto-exposure: manual mip-chain luminance downsample to 1x1, smoothed adaptation (`exposure = lerp(exposure, target, 1 - exp(-dt*adapt_speed))`), target from `key / (avg_luminance + eps)`, key ≈ 0.18. Tonemap: ACES filmic (Narkowicz fitted approximation, no LUT). Log exposure value into `log.jsonl` every frame — same tier-1 testing as everything else, assert it's bounded and doesn't oscillate.

**Ocean:** Sum of 4-8 Gerstner waves at varying wavelength/direction/amplitude (bigger/slower waves dominate, smaller add chop). Blinn-Phong + Fresnel (Schlick approximation) using the real sun direction/color, cubemap reflection (not SSR) for now. Sea level = height 0 in the outmap.

## Debug & test infrastructure

Built right after Phase 0, used by every phase after that. Don't skip or defer this.

- **Logging:** `tracing` → JSON-lines log file, one line per ~0.5s (not per render frame). Fields: sim_time, camera world pos (f64 xyz), lat/lon/altitude, velocity, orientation, LOD level histogram (counts per level 0-17), chunks loaded/unloaded this tick, frame_time_ms, draw_calls.
- **Debug overlay:** egui panel, toggleable, shows FPS/LOD stats/camera position. Exists from Phase 0 onward.
- **Test scenarios:** data-defined waypoint lists (time_s, position, look_at), run with a **fixed simulation timestep** (not wall-clock dt) for determinism — same scenario must produce identical frames every run.
- **Screenshots:** wgpu texture readback → PNG, triggered at scenario waypoints or a debug-mode keypress.
- **Storage:**
  ```
  /test-runs/{scenario_name}/{run_id}/
    manifest.json          -- scenario, git commit, timestamp, pass/fail
    log.jsonl               -- per-frame spatial log (fields above)
    screenshots/*.png
    screenshots/manifest.json  -- maps PNG filename -> corresponding log.jsonl entry
  ```
**Check order — three tiers, cheapest and most objective first:**
1. *Log assertions* against `log.jsonl` — NaN/inf checks, chunk-count bounds, frame time budget, LOD thrash detection (rapid split/merge on the same chunk), `max_seam_delta_m` under tolerance when a chunk changes LOD. Fully automated, no image involved.
2. *Programmatic pixel/image analysis* — still fully objective, no judgment call, just reads numbers out of a screenshot: crack detection via a flat-color-per-chunk debug pass scanned for background pixels inside the planet silhouette; atmosphere sunset check via sampling fixed sky pixel coordinates across a sun-angle sweep and asserting red-channel-over-blue grows near the horizon; ocean check via sampling wave-height statistics from a heightfield readback rather than looking at the water. Prefer this over vision whenever the thing you're checking can be reduced to "read this pixel/region and compare a number."
3. *Vision review* (Codex or a human looks at the screenshot) — last resort only, for cases where you don't have a predefined numeric criterion yet or something looks wrong in a way nobody thought to assert against. Don't default to this tier; if a check keeps getting done by vision, that's a sign it should be converted to tier 2.

Every phase from Phase 2 onward adds/reuses at least one named test scenario checked into the repo, so old scenarios catch regressions from new phases. Aim for every scenario to have at least one tier-1 or tier-2 assertion — a scenario that only produces screenshots for someone to look at isn't a real test yet.

## Crate versions (pinned, July 2026)

```toml
wgpu = "29.0"
winit = "0.30.13"
egui = "0.35"
egui-winit = "0.35"
egui-wgpu = "0.35"
glam = "0.30"
bytemuck = "1.25"
rayon = "1"
image = "0.25"
noise = "0.9"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json"] }
```
If a `cargo add` resolves something newer, that's fine — these are floors, not hard locks.

## Workspace layout

```
/crates
  /app        binary: winit window, wgpu device, egui, main loop
  /render     wgpu pipelines: terrain, atmosphere, ocean, egui pass
  /planet     cube-sphere mesh gen, quadtree LOD, chunk streaming
  /baker      offline CLI: noise -> erosion -> rivers -> climate -> outmap files
  /coretypes  shared math (f64 world coords, camera-relative rebasing), shared structs
/assets/outmaps   baked planet data, gitignored, regenerated by baker
```

## Terrain gen pipeline (baker crate only)

Order matters, each step consumes the previous:

1. **Base shape** — fbm (5-8 octaves) sampled in 3D on the sphere direction vector, not 2D UV (avoids cube-face seams). Domain-warp with a second, lower-frequency fbm for less "noisy" continent edges.
2. **Mountains** — ridged multifractal noise, masked/multiplied by a low-frequency (1-2 octave) "tectonic belt" noise field so ranges read as coherent chains, not scattered bumps. Clamp to the 9,000m ceiling above.
3. **Hydraulic erosion** — grid-based, stream power law: `erosion_rate ∝ (flow_accumulation)^m * (slope)^n`, typical m≈0.5, n≈1. Run in tiles with rayon, a few thousand iterations, diminishing step size over iterations.
4. **Thermal erosion** — interleave with hydraulic: any cell exceeding talus angle (~35° rock, ~45° hard rock) sheds material downhill toward the lowest neighbor until under threshold.
5. **Flow accumulation / rivers** — steepest-descent flow direction per cell (D8 algorithm), accumulate upstream contributing area, threshold accumulated flow to mark river cells, carve channel depth/width as a function of accumulated flow (wider/deeper downstream), merge tributaries.
6. **Lakes** — priority-flood depression filling on cells that don't drain to ocean; filled cells above original height + below fill level get flagged biome=lake.
7. **Glacial valleys** — on drainage basins where latitude or altitude crosses a glaciation threshold, apply a U-shaped cross-section filter (parabolic, not V) perpendicular to flow direction, replacing the river's V-cut in that stretch.
8. **Biome classification** — per cell: latitude (from cube direction) + altitude + moisture (moisture = inverse distance-to-water, blurred) → biome table lookup (Whittaker-style: temp axis from latitude+altitude, wet axis from moisture). Ice cap overrides everything per the snowline rule above.
9. **Export** — tiled height/biome/moisture textures matching quadtree structure, plus one equirectangular PNG preview per channel for sanity-checking without the renderer.

Test the baker standalone with the PNG preview before ever wiring it into the renderer.

## Session rules

- One phase per session (see phase list below). Don't scope-creep into the next one.
- Run `cargo check` after meaningful changes, not just at the end.
- Update "What exists now" at the top of this file before ending the session.
- If you hit a design question not answered here, make the smallest reasonable call, note it under "What exists now", don't block waiting for input.

## Phase list

0. Skeleton — wgpu+winit+egui window, clear screen, FPS counter.
0.5. Debug/test infra — tracing JSON-lines logger, debug overlay toggle, manual screenshot capture, fixed-timestep test-mode scaffolding. Prove the harness with a trivial scenario before anything else exists.
1. Cube-sphere mesh, fixed res, no LOD, no height. Orbit camera. Prove f64→f32 rebasing works. Test scenario: one orbit, 4 screenshots, check for seams at cube face boundaries.
2. Quadtree LOD, split/merge, skirts. Placeholder sine-wave height to visualize popping/morphing. Test scenario: orbit-to-10m descent, log LOD histogram + screenshots at several altitudes.
3. Baker CLI — full terrain pipeline above, PNG preview output. Checked via preview PNGs directly, no game-side scenario yet.
4. Wire outmap into renderer — tile streaming matched to quadtree, GPU height displacement, biome coloring. Reuse Phase 2's descent scenario, compare against its placeholder-terrain baseline.
5. Atmosphere — per-vertex terrain aerial perspective + raymarch sky/space shader, altitude-aware density, no LUTs. Test scenarios: fixed camera + swept sun angle (sunset color check), plus ground-to-orbit ascent (check sky-to-space transition has no seam).
5.5. Sun & HDR pipeline — real sun disc, HDR float target, luminance downsample, smoothed auto-exposure, ACES tonemap. Test: log exposure in ground-to-orbit ascent, assert bounded/no oscillation; add "stare at sun" scenario, check smooth adapt not snap.
6. Ocean — Gerstner shader, specular/fresnel using real sun direction, swap in below sea level. Test scenario: low-altitude ocean flyover.
7. Polish — ice cap visuals, egui debug panel additions, perf pass. Rerun all prior scenarios as a regression pass.

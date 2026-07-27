# Handoff — ground readability / render modernisation

**Branch:** `experiment/ground-readability` (32 commits ahead of `origin`, **not pushed**)
**Head:** `1847f63` "Assert the probe on p90, not max: the maximum was measuring the horizon"
**Written:** 27 July 2026
**Supersedes:** `PLANET_SIM_HANDOFF.md` at the repo root, which describes the 19 July low-flight
state and is now history. Read `AGENTS.md` for the architecture; read this for where the work is.

---

## 1. The goal, in Ian's words

A renderer that "looks like a modernish (2015 on) game", consistent from orbit to ground, with
**1 m ground detail** and **realistic seamless textures over the whole planet**.

### Constraints that are not up for renegotiation

1. **We must be able to stand on the ground — not below it, not above it.** Terrain truth (what the
   CPU collides the camera with) and the rendered surface must be the same surface. This is what the
   probe in §4 exists to enforce.
2. **Hybrid generation.** 1 m samples over a 4000 km planet is 804 TB. Baking below macro scale is
   categorically impossible; everything finer than the outmap is synthesised at runtime. The baked
   planet is fully disposable and can be re-baked.
3. **30 fps / ~33 ms** on the Quadro M1000M. *Currently breached — see §6.*
4. **Both render paths, and always name which one.** Ian evaluates in raymarch mode. Parity is the
   goal. Never present a measurement without saying raster or ray.

---

## 2. State: what is green

```
cargo test --workspace   →  182 passed, 0 failed, 1 ignored
                            (app 150, baker lib 20, baker bin 1, baker integration 5, coretypes 6)
```

Scenario probe results, worst frame, from `test-runs/*/*/manifest.json` at `8aaafeb`:

| scenario | path | p90 delta | median delta | tolerance | clearance |
|---|---|---:|---:|---:|---:|
| `stand_on_ground` | raster | **0.25 m** | 0.10 m | 6 m | 2.0000 m |
| `stand_on_ground` | ray | **4.48 m** | 0.55 m | 6 m | 2.0000 m |
| `path_parity_ridge` | raster | **1.93 m** | 0.97 m | 8 m | 133 m |
| `path_parity_ridge` | ray | **7.25 m** | 2.93 m | 8 m | 133 m |

The camera stands exactly 2.0 m above the ground it is drawn on, in both paths. That was the point
of the whole exercise.

**Do not read `max_abs_delta_meters` as a surface measurement.** At 2 m eye height the horizon is
4 km and the probe compares out to 4 km; a ray arriving at a fraction of a degree turns a metre of
ground into hundreds of metres of reconstructed height. `stand_on_ground` ray shows max 194 m from
exactly two grazing points out of 77. p90 is the assertion that means something.

## 3. State: what is red

Six scenarios fail. **All six were failing before this branch and are unchanged by it** — each was
verified by building the older commit and diffing the number. Do not re-investigate them as
regressions from this work.

| scenario | failing assertion | observed |
|---|---|---|
| `descent_to_10m` | `fallback_chunk_count_is_bounded` | 256 vs 128 allowed |
| `ocean_flyover` | `fallback_chunk_count_is_bounded` | 254 vs 192 |
| `sunset_sweep` | `fallback_chunk_count_is_bounded` | 256 vs 192 |
| `ground_to_orbit` | `seam_delta` + `fallback_chunk_count` | 707 m seam; 254 fallbacks |
| `low_flight_performance` | `lod_stays_within_chunk_budget` + seam + fallback | 241 budget-limited frames |
| `orbital_zoom_lod` | `lod_reaches_required_level` | peak L16, required L18 |

**Read that table again — five of the six are one bug.** Every one of them saturates the chunk
budget and falls back to ancestor tiles. That is item 1 of §6.

`orbit_once` in ray mode also fails `resident_chunk_count_is_bounded` because ray mode used to
suspend the raster quadtree. That is a harness assumption, not a bug.

---

## 4. The instrument: `crates/app/src/probe.rs`

Everything on this branch is judged by this, not by screenshots. It copies the depth attachment on
screenshot frames, reconstructs each hit in planet coordinates (reversed-Z infinite perspective
writes exactly `near / forward_distance`), and compares against
`Terrain::surface_height_breakdown_at`. One code path serves both renderers because both write depth.

It measures the **drawn** surface, so it also sees tessellation, LOD choice and the ray marcher's hit
refinement — not just the height function.

Reported per screenshot frame, into `manifest.json` under `surface_probes`:

- `camera_clearance_meters` — the stand-on-the-ground number.
- `p90_abs_delta_meters` / `median_` / `max_` — rendered vs CPU truth.
- `delta_from_macro_meters` — against baked data only, which separates "drew no detail" from "drew
  the wrong detail".
- `detail_correlation` — **Pearson r between the CPU's relief and the renderer's.** This is the
  whole point of the instrument. Two independent noise fields of matching amplitude agree on every
  other statistic and on every screenshot while being different terrain.

Scenario assertions: `max_surface_probe_p90_delta_m` (the meaningful one),
`max_surface_probe_delta_m` (loose outlier guard), `min/max_camera_clearance_m`,
`min_surface_probe_points`. A delta tolerance is **refused** unless the scenario also states a point
floor — a run that saw only sky would otherwise pass on no evidence.

**Trap:** schedule the depth copy *before* the visual sun overlay pass; that pass has
`StoreOp::Discard` on depth.

`p90` is in the manifest but **not** in the `"surface probe"` tracing line — worth adding.

---

## 5. How to run things

```bash
# Tests
cargo test --workspace

# A scenario, raster path
target/release/catinthegarden-app --scenario stand_on_ground

# The same scenario, raymarch path
CATINGARDEN_RENDER_PATH=ray target/release/catinthegarden-app --scenario stand_on_ground

# Real frame times (Fifo pins everything to 16.67 ms and hides the truth)
CATINGARDEN_PRESENT_MODE=immediate target/release/catinthegarden-app --scenario tour_mountains
```

Results land in `test-runs/<scenario>/<unix>-<id>/{manifest.json,log.jsonl,screenshots/}`.
**Always check `git_commit` in the manifest before trusting a number.**

- **Never pass `--profile-render` on the Quadro.** It enables `TIMESTAMP_QUERY`, which makes
  `present()` block forever on frame ~3 on driver 550.163.01. Days were lost blaming PRIME for this.
- **Measure frame times only on an idle machine.** `pgrep -f catinthegarden-app` first — Ian often
  has his own instance running, and contended readings run 2–10× high. A 3.9 ms figure quoted to him
  was contaminated this way; the clean number was 2.5 ms.
- Benchmarks build to `/home/dad/catingard-target`, not the in-repo `target/`.
- Other flags: `--terrain placeholder|outmap`, `--outmap <path>`, `--vertical-fov-degrees`,
  `CATINGARDEN_RAY_EXPERIMENTS`, `WGPU_ADAPTER_NAME`.

There is no env hook for the debug shading modes (F9 cycles them interactively, but scenarios cannot
press keys). Add a temporary `CATINGARDEN_DEBUG_MODE` match on `render_debug_mode` in `main.rs` when
you need one.

---

## 6. Next, in order

### 1. The chunk budget — Ian asked for this and it was not started

`planet.rs:30` — `DEFAULT_MAX_ACTIVE_CHUNKS = 256`. `budget_limited` is true on **16 of 17**
`tour_mountains` frames (it was 8/17 before roughness went to 0.06). The selector wants more chunks
than it may have, so the finer tessellation the LOD budget now asks for is not actually delivered.

This is also the likely root of five of the six red scenarios in §3, which makes it the highest-value
thing on the list: it is both a feature and a test-suite repair.

### 2. Materials

`terrain_material_weights_for_biome` has `rock_amount = smoothstep(0.10, 0.42, slope)` where slope is
`1 - dot(normal, radial)`. Measured over 90 000 samples at 1 m spacing: p50 0.0041, p99 0.0217,
max **0.159** at roughness 0.06 — which crosses 0.10 for the first time, but over only **0.004%** of
the surface. The rock path is wired in and has essentially never fired; rock appears only via the
`biome == 8` override.

**Do not retune the 0.10 threshold.** It is ~26°, near the angle of repose, and physically right.
Lowering it paints rock on gentle hillsides. The terrain needs to be steeper, or materials need a
driver other than slope.

The landing site is **biome 6, tropical forest, moisture 0.84**, where `biome_vegetation_amount`
gives ~0.97 vegetation — so uniform dense green is largely *correct* there. Do not tune materials
against that site expecting variety; check grassland, desert and rock too.

### 3. Near-field window streaming rate

The window needs 64 L12 tiles and `MAX_TILE_UPLOADS_PER_FRAME = 4` is shared with the raster
quadtree, so it takes **over a second**. Until it lands, the raymarch path draws the coarse pyramid's
ground ~50 m low — a visible pop in interactive flight. `stand_on_ground` starts its probes at 2 s so
it measures the surface rather than the streaming, which is why this does not show up as a red test.

*(This item replaces "nested near-field window levels", which was on the list to fix a raymarch
regression that measurement showed does not exist. The window at L12 already delivers sub-metre
agreement. The streaming warm-up is the part that is real.)*

### 4. The frame budget, which Ian said to absorb "for now"

| view | raster | ray |
|---|---:|---:|
| `tour_mountains` | **37.9 ms** | **34.5 ms** |
| landing site | ~33.6 ms | ~35 ms |

Both paths are over the 33 ms budget at the mountains. Ian was told and said to carry on, so this is
a known accepted debt, not an unreported breach — but it needs a ruling before more cost is added.
The ray path's jump (21.5 → 34.5 ms) is four more octaves evaluated at every march step.

---

## 7. What the terrain actually is now

`shared_planet.wgsl`, mirrored in `planet.rs`, guarded by
`shader_detail_ladder_matches_the_cpu_clearance_ladder` (which reads the constants back out of the
shader source so the two cannot drift).

```
TERRAIN_DETAIL_ROUGHNESS            0.06      amplitude = roughness × wavelength
TERRAIN_DETAIL_START_WAVELENGTH     4096 m    starts where the baked data stops
TERRAIN_DETAIL_OCTAVES              13        down to 1 m
TERRAIN_DETAIL_RIDGE_SOFTNESS       0.15      sqrt(n² + s²), not abs(n)
TERRAIN_DETAIL_RIDGE_CENTRE         0.348609  ┐ properties of the *softened* fold —
TERRAIN_DETAIL_RIDGE_SCALE          2.063534  │ re-derive these if the fold changes
TERRAIN_DETAIL_RIDGE_NORMALISATION  1.313064  ┘
TERRAIN_DETAIL_RIDGE_STRENGTH       0.7
TERRAIN_DETAIL_ATTENUATION_SLOPE    0.25      multifractal damping by slope so far
TERRAIN_DETAIL_HEADROOM_FACTOR      8.0       per-octave land weighting
```

Four things about it that are load-bearing and non-obvious:

- **The hash is integer arithmetic, and it has to be.** Anything folded by `fract` needs bit-exact
  inputs to be shared between CPU and GPU. The old `fract(sin(dot(cell,k))*43758)` ran f32 in the
  shader and f64 on the CPU; at 1e9 the sin argument's consecutive f32 values are 64 radians apart,
  so the two sides computed **unrelated** fields — measured correlation 0.02 — while agreeing on
  amplitude, character, slope statistics and every screenshot.
- **Cell index and in-cell fraction are passed separately.** The cell comes from
  `anchor_direction * frequency` (huge, but an exact integer); the fraction comes from a short
  anchor-local offset. Without this f32 quantises the fraction to 0.25 at 1 m wavelength.
- **The ladder is bounded above by `baked_sample_spacing_meters(source_level)`.** Without that high
  cut, a 4096 m start stacks a 246 m octave on top of the corridor's own erosion — the same hills
  twice. Raster reads the source level out of its packed `terrain_info`; ray derives it from the
  dense faces or the near-field window.
- **Headroom is per-octave, not a scalar land weight.** A 492 m ladder gated on its total reach
  strips a 40 m plain of the 4 m hummocks it does have room for. Each octave asks separately, which
  gives relief-correlated amplitude for free. Factor 8 because each octave alone is safe at 2 but
  thirteen together are not; a test walks elevations asserting the ladder can never reach sea level.

The LOD error budget knows about all this: `OUTMAP_GEOMETRIC_ERROR_RATIO` is
`0.0536 + ROUGHNESS * 2.9395` (`terrain.rs:84-90`), derived rather than tuned. Before that it was a
flat 0.15 — a function of level and nothing else — so a louder ladder never made the selector split
further, which is why raising roughness used to produce stair-stepping.

**The raymarch path's near-field window** (`terrain.rs` / `foveated.rs`) is an 8×8 block of L12 tiles
resampled into a 1025² R32Float texture, because L12 already reads within 0.5 m of L18 at the landing
site and an L12 tile spans 1.5 km. It refuses to enable unless every block is finer than
`dense_level`, so outside the sparse corridor it costs nothing. It rebuilds on `NearFieldSources`
(the resolved tile keys), **not** on camera position — a stationary camera keeps the same square
while streaming replaces coarse ancestors underneath it.

---

## 8. Do not redo these

Each was built, measured, and rejected on evidence.

- **Cast shadows.** Prototyped and reverted: +8.2 ms for 0.12% of pixels changed. Shadows need grade
  > tan(sun elevation); the synthesised ladder's characteristic grade is 6.6%, so it casts only below
  3.8°, and below ~8° the atmosphere has already taken the ground to near-black. There is nothing to
  shadow. Do not rebuild without new terrain.
- **A multiply-free integer hash.** 2 ms slower in raster, 13 ms in ray, and it failed its own
  quality test on an adjacent-cell correlation of −0.09.
- **Per-block max-height ceilings for the marcher.** Measured no faster and reverted. An earlier
  "3.4 ms" reading was a bad diagnostic — returning a tiny ceiling made the marcher take *bigger*
  steps.
- **`1/sin(incidence)` filter widening in the ray path.** Textbook grazing correction; it washes
  relief out of grazing views while the raster path does fine without it. Parity means matching the
  raster filter.
- **Retuning the 0.10 rock slope threshold.** See §6.2.
- **`GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE = 0.0`** gates the *retired* CPU direction-noise field. That
  is intentional. Do not re-enable it; the live ladder is `terrain_detail_meters`.

---

## 9. How to not waste a day here

These are the mistakes that actually cost time on this branch.

- **Judge renders by pixel values, not by eye.** Counting near-black pixels per capture took one
  script and settled in seconds what three rounds of plausible visual interpretation got wrong. I
  twice described a screenshot as showing something it did not, and Ian caught both. A plausible
  physical story for an artefact is not evidence.
- **Read the distribution, not the tolerance verdict.** I reported the ray path as regressed on a
  single breached maximum. It had improved from 87.96 m to 0.55 m median. A failing assertion tells
  you a number moved, not what happened.
- **Correlation, not amplitude.** "Non-bit-exact but statistically equivalent" is a claim requiring
  evidence, and the evidence is Pearson r. Means and standard deviations agree between two completely
  unrelated fields.
- **Check `cargo test` counts after touching attributes.** The one guard keeping the CPU clearance
  ladder and the shader displacement on the same planet silently lost its `#[test]` and did not run
  for several commits. The tell is `cargo test <filter>` reporting "0 passed, N filtered out" for a
  test you know exists.
- **Bisect finds what changes a probability, not what causes a fault.** A good feature was reverted
  on a correct bisect and a wrong attribution. The driver "crash" was `std::process::exit(1)` on the
  scenario-failure path running no destructors, so the wgpu device was never dropped. The tell was
  measuring *when* it died: the crashing run had already written every capture and a complete
  manifest.
- **Only pass a value `flat` if it is constant over the whole primitive-generating unit**, not merely
  slowly varying. A camera-distance-derived cutoff passed `flat` became a step function per triangle
  and shaded as hard facets.
- **Probe the baked outmap directly rather than bisecting with screenshots.** Tiles are at
  `assets/outmaps/test-planet/tiles/<face>/l<NN>/x%06d_y%06d/{height.r32f,biome.r8,moisture.r8}`,
  row-major over `tile_stored_size`, offset by `tile_gutter`. Face `nx` uv is `(z, y) / -x` mapped to
  [0,1]. The manifest carries `available_tiles` and `sparse_landing_direction`.
- **Tests that pin render-derived literals go stale every time the field changes.** Two did. Both are
  now property checks. Do not add another literal read off a screenshot — the probe measures
  continuously, in both paths, what those literals were reaching for.
- **Re-author scenario camera heights after any ladder change.** Changing the field moves the ground.
  `landing_site_eye_level` ended 2 m underground after one such change; the clearance assertion
  caught it. Landing-site ground is currently **915.87 m**.
- Shader gotchas: `active` is a reserved WGSL keyword; the inter-stage location limit is 16;
  `shared_planet.wgsl` is concatenated **ahead** of `planet.wgsl`, so tests that slice the shader by
  splitting on a function name silently run to end of file — bound on `"\nfn "`.

---

## 10. Working agreements

- **Commit and push after each set of changes** (`AGENTS.md`). The branch is currently **32 commits
  ahead of `origin` and unpushed** — decide whether to push before starting new work.
- Update `AGENTS.md` "What exists now" at the end of a session, and keep **this file** current in the
  same change as any behaviour, architecture, command, risk or next-action change.
- Give every temporary/staged checkout its own `CARGO_TARGET_DIR`. Sharing the worktree's `target/`
  can replace the runnable binary with a different source tree while Cargo reports it fresh.
- **The Codex relay:** `response/claude.txt` is written *by Claude*, `response/codex.txt` *by Codex* —
  the file is named after its author, not its recipient. On "ok ready", read `codex.txt` as the next
  prompt. Codex will not accept relayed authorization for destructive or scope-changing actions; that
  is a legitimate anti-impersonation safeguard, so tell Ian to instruct it directly rather than
  arguing. **Benchmarking needs an exclusive tree** — Codex once committed a feature removal midway
  through a build-and-measure cycle and silently invalidated the run.

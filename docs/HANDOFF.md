# Handoff — ground readability / render modernisation

**Branch:** `experiment/ground-readability` (pushed; `origin` is level with local)
**Head:** "Charge the baked error only where a split can still resolve it"
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
cargo test --workspace   →  186 passed, 0 failed, 4 ignored
                            (app 154, baker lib 20, baker bin 1, baker integration 5, coretypes 6)
                            the 4 ignored are the relief_survey instruments -- run them with
                            `cargo test -- --ignored --nocapture <name>`
```

Scenario probe results, worst frame, from `test-runs/*/*/manifest.json`:

| scenario | path | p90 delta | median delta | tolerance | clearance |
|---|---|---:|---:|---:|---:|
| `stand_on_ground` | raster | **0.25 m** | 0.10 m | 2 m | 2.0000 m |
| `stand_on_ground` | ray | **0.89 m** | 0.62 m | 2 m | 2.0000 m |
| `path_parity_ridge` | raster | **4.24 m** | 1.25 m | 6 m | 133 m |
| `path_parity_ridge` | ray | **10.68 m** | 2.17 m | 6 m — **FAILS, see §6b** | 133 m |
| `tour_mountains` | ray | 22.6 m | 11.7 m | none | ~1 km |

These moved with the mountain work in §6b: the terrain now has three times the relief, so the same
mesh disagrees with truth by more in absolute metres. Raster still holds well inside tolerance
everywhere; the ray path does not, on the ridge.

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

**These are not one bug, and `fallback_chunk_count_is_bounded` is largely measuring the
architecture rather than a defect.** An earlier revision of this document claimed all five fallback
failures shared a chunk-budget root cause. Per-frame logs say otherwise — they have three separate
causes:

1. **Frame 0 warm-up.** `low_flight_performance` reads 256/256 fallback at t=0 with 6 tiles resident,
   then settles to a steady **66** once streaming catches up. The assertion takes a maximum over all
   frames, so one unstreamed frame fails the run.
2. **No fine data exists there, and none ever will.** `ocean_flyover` holds 10 resident tiles and 254
   fallbacks for its whole run. The bake is dense L0–L4 globally plus one sparse L5–L18 corridor, so
   any near-ground camera outside that corridor *must* draw from ancestors. That is the hybrid design
   (§1.2), not a failure — the runtime ladder is what fills the gap, and the probe confirms it does
   (`path_parity_ridge` sits outside the corridor and agrees to 1.93 m p90 raster).
3. **Descent outrunning streaming.** `descent_to_10m` pins at 262 resident tiles while altitude falls
   5958 km → 0.

**These assertions encode the superseded "baked displacement only" architecture.** They ask for
resolved baked tiles at levels the planet was never baked at. Rewriting them to assert what actually
matters — probe agreement — is real work worth doing, but it is a harness repair, not a renderer fix,
and it should not be confused with the chunk budget.

Only `low_flight_performance` genuinely fails `lod_stays_within_chunk_budget`.

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
- **`CATINGARDEN_PRESENT_MODE=immediate` is not optional for any timing measurement.** The default
  `Fifo` pins to the 60 Hz refresh (~16.7 ms floor, hiding anything cheaper) **and throttles to ~1 Hz
  when the window is not visible** — a blanked screen or an unfocused window turns every frame into
  a flat ~1000 ms. That reads exactly like a catastrophic regression. If you see suspiciously round
  frame times near 1000 ms with `nvidia-smi` showing 0% util and P8, it is the throttle, not the
  renderer. Confirm by re-running with `immediate` before reporting anything.
- Benchmarks build to `/home/dad/catingard-target`, not the in-repo `target/`.
- **`CATINGARDEN_DEBUG_MODE=albedo|lighting|aerial|sky`** selects a render debug mode for a scenario,
  which F9 could previously only reach interactively. `albedo` is how you tell a material problem
  from a lighting one, and at the mountains it says the warm tan is lighting.
- Other flags: `--terrain placeholder|outmap`, `--outmap <path>`, `--vertical-fov-degrees`,
  `CATINGARDEN_RAY_EXPERIMENTS`, `WGPU_ADAPTER_NAME`.
- **`CATINGARDEN_MAX_ACTIVE_CHUNKS` lifts the chunk budget** (selector and instance buffer together)
  so a run can show what the selector actually wants rather than what the cap allows. `budget_limited`
  going to 0 is how you know demand is satisfied and the number is real. Do not read a demand
  reduction as a frame-time saving without checking this: at the default 256 the cap binds on every
  frame of every scenario, and a change that cuts demand by a third can leave the frame untouched.

There is no env hook for the debug shading modes (F9 cycles them interactively, but scenarios cannot
press keys). Add a temporary `CATINGARDEN_DEBUG_MODE` match on `render_debug_mode` in `main.rs` when
you need one.

---

## 6. Next, in order

### 1. The chunk budget — DONE, and it bought less at the cap than the demand figure suggests

**Ian took the call and the baked term is now dropped where it cannot resolve anything.** The
selector carries its error in two parts (`GeometricErrorRatio` in `planet.rs`): the baked macro
surface's own curvature and resampling error, and the runtime ladder's. A node is charged the baked
term only while its children still have unread source texels — `source_level + 2`, the same bound
`outmap_node_level_limit` enforces when it is enforced at all. Past that a split resamples the same
bilinear patch and returns its parent's surface exactly, so the demand was for geometry that
provably could not differ. The ladder term is charged all the way down, because the ladder really
does have another octave.

The limit is asked for every evaluated node, which is thousands per update, and `resolve_tile` is a
binary search per level walked. `BakedErrorLimit` memoises every key each walk passes through, so a
sibling checks itself, hits the shared parent, and stops. It still tests each key itself before
consulting an ancestor's entry, so it assumes nothing about the tile pyramid.

**Measured, raster, idle machine, `PRESENT_MODE=immediate`, one binary per column:**

| | cap 256 before | cap 256 after | cap 1024 before | cap 1024 after |
|---|---:|---:|---:|---:|
| `tour_mountains` frame | 39.2 ms | **36.9 ms** | 66.4 ms | **45.4 ms** |
| `tour_mountains` chunks / tri | 256 / 679 k | 255 / **604 k** | 530 / 1.29 M | **356** / 848 k |
| `tour_mountains` budget-limited | 14/14 | **11/14** | 0/14 | 0/14 |
| `low_flight_performance` frame | 30.4 ms | 30.7 ms | 38.2 ms | **37.3 ms** |
| `low_flight_performance` chunks | 256 | 256 | 352 | **345** |

**Read the two halves of that table differently.** With the budget lifted clear of demand the change
is large and does what the model predicted: mountain demand falls 530 → 356 chunks (0.67×, against a
0.59 prediction from ratio² — the gap is the balancing pass, which adds graded nodes the ratio does
not govern) and 21 ms comes off the frame. **At the shipping cap of 256 almost none of that reaches
the frame, because the cap was already binding and still is.** The mountains keep 2.3 ms, all of it
from the balancing overshoot shrinking (679 k → 604 k triangles); every other scenario is pinned at
256 chunks exactly as before and does not move at all.

**`low_flight_performance` barely moved even at cap 1024 — 352 → 345 — and that is the change working
correctly, not failing.** It flies the sparse corridor, where fine baked data really exists, so the
baked term is still legitimately charged there. The saving appears only where the data has run out.

Quality, raster: `tour_mountains` probe p90 **4.04 → 3.83 m** at cap 256 (the same budget spent
where it resolves something), `stand_on_ground` unchanged at 0.25 m, `path_parity_ridge` 1.93 → 1.97 m
with its median improving 0.97 → 0.88 m. `detail_correlation` stays 1.000. At cap 1024 the mountains
cost 3.70 → 3.83 m, which is the honest price of the removed demand and is 0.13 m.

Ray path: `stand_on_ground` unchanged at 0.64 / 0.45 m; `tour_mountains` 37.3 → 36.7 ms.

**What is still true:** the cap is load-bearing and must not be raised — cap 1024 is 45 ms even after
this. The mountains are still over budget at 36.9 ms against 33 ms. The remaining levers are the
2 px split threshold and the ladder term itself, and both are quality trades with no free win in
them. The measurement below is why.

**Ray `tour_mountains` reads p90 1529 m, median 1377 m, `detail_correlation` −0.501.** That is
pre-existing — it re-measures bit-identically with this change reverted — and nothing asserts on it,
so it has never been looked at. A negative correlation is the instrument saying the marcher's relief
runs *opposite* to the CPU's, which is not what altitude or grazing geometry alone would do. It
deserves its own investigation; see §9 on not reading a single statistic as a verdict.

### 1b. The measurement hook this needed

`CATINGARDEN_MAX_ACTIVE_CHUNKS` overrides `DEFAULT_MAX_ACTIVE_CHUNKS` and the instance buffer
together, so "is the cap binding, and by how much" is now one run rather than an edited constant and
a rebuild. The two have to move together or a lifted budget silently draws only the first 256 chunks.

### 1c. The original diagnosis, kept because the reasoning still governs

`planet.rs:30` — `DEFAULT_MAX_ACTIVE_CHUNKS = 256`. `budget_limited` is true on every
`tour_mountains` and `low_flight_performance` frame, so the selector is permanently suppressed. It is
tempting to read that as a ceiling to lift. **It is not. It is the only thing holding the mountains
at 38 ms instead of 59 ms.** Measured on an idle machine, `CATINGARDEN_PRESENT_MODE=immediate`,
raster, cap 256 vs a temporary 1024:

| | cap 256 | cap 1024 (demand satisfied) |
|---|---|---|
| `low_flight_performance` | 256 chunks, 590 k tri, **30.5 ms** | 352 chunks, 811 k tri, **38.3 ms** |
| `tour_mountains` | 256–318 chunks, 645 k tri, **38.0 ms** | 512 median / 677 peak, 1.18 M tri, **58.6 ms** (p90 72.3) |

**Cost is triangles, linearly, at ~50 ns each.** `draw_calls` stays at 12 across the entire
266→677-chunk range — the renderer is instanced, so there is no per-chunk overhead to reclaim and no
batching win available. Cost per triangle is flat at 48–52 ns over that whole range.

**1.18 M triangles for a 921,600-pixel frame is 1.3 triangles per pixel.** Even the 256-chunk
baseline is already 0.7. The selector is asking for sub-pixel geometry.

**The demand is legitimate under the current model — this was checked, there is no free win.**
`tour_mountains` reads 5800 m altitude, but that is above the *reference sphere*; the surface is at
4721 m, so true clearance is **1079 m**. An L13 node splits at `spacing × ratio × projection / d > 2 px`,
i.e. within 1905 m. 1079 < 1905, so the 151 chunks observed at L14 are exactly what the model orders.
The arithmetic is consistent; the model is simply expensive.

**Where the expense comes from.** `OUTMAP_GEOMETRIC_ERROR_RATIO = 0.0536 + ROUGHNESS × 2.9395` = 0.23
at roughness 0.06, of which the ladder term is **77%**. At the mountains, 151 of 256 chunks sit at
L14 with a source-level delta of **10** — an L14 mesh (12 m vertices) sampling L4 baked data (~1953 m
texels). Everything at that scale is the runtime ladder, and the model credits every further split
with resolving more of it, all the way down.

**So the real lever is the ratio or the 2 px split threshold, and both are quality-versus-cost
trades, not bug fixes.** Reducing error demand by ×0.77 (dropping the baked term, which cannot
resolve anything past L4 out here anyway) would cut demand to ~0.59 of current. That is the most
defensible saving available because it removes work that provably produces nothing.

*(Done — see §6.1. The prediction was close: measured 0.67, not 0.59. The lesson worth keeping is
that a demand reduction is not a frame-time saving while the cap is binding, and it was binding on
every frame of every scenario. Only the mountains saw any of it at cap 256.)*

### 2. Materials — and the ambient idea that this measurement killed

An earlier revision of this file said the biggest remaining gap was that unlit ground goes to
near-black, with no ambient term worth the name, and recommended adding one. **That is measurably
false and the opposite of the truth.** Across every ground scene there are **zero pixels below 0.05
luminance, and none below 0.02**; the darkest 1% of a frame sits at 0.09–0.42. The ranges are narrow,
not dark:

| scene | p01 | p50 | p95 |
|---|---:|---:|---:|
| `stand_on_ground` | 0.256 | 0.373 | 0.440 |
| `tour_grassland` | 0.422 | 0.505 | 0.699 |
| `tour_mountains` | 0.094 | 0.564 | 0.600 |

Under a stop of range across the whole ground. Ambient already exists — `sky_diffuse_irradiance`,
`SKY_DIFFUSE_LIGHT_SCALE` 0.18 against `SURFACE_SUNLIGHT_SCALE` 2.0, roughly **4% of direct** — and
adding more would flatten the picture further, which is the actual defect. It is not auto-exposure
either: that sits pinned at its 1.0–4.0 rail in these scenes.

`tour_mountains` at 1 km renders as a uniform yellow-tan dune field, with regular diagonal moiré
across the ground that wants its own investigation.

**Coastlines are now the coarsest thing in the picture, and that is structural.** The pale angular
patches in that capture are *water* — not, as an earlier revision of this file guessed, a material
seam on a tile boundary. What makes them read as wrong is that their edges are straight lines at
kilometre scale. The ray path decides land-versus-water from the cube-face height texture at
`face_quads = 128 × 2⁴ = 2048`, i.e. **3906 m per texel**, and the raster path is no better outside
the sparse corridor (`baked_sample_spacing_meters(4)` is the same 3906 m). A shoreline can therefore
only change direction every 3.9 km, and inside one texel the bilinear zero-crossing is a straight
line — which is exactly the polygon shape seen.

**Runtime detail cannot rescue it, by construction.** `ray_terrain_detail` returns zero when
`scaled_macro_height_meters <= 0.0`, and the per-octave headroom `smoothstep(0, 2A, h)` takes the
ladder to nothing as macro height approaches sea level. That gate is deliberate — it is the safety
proof that the ladder can never push the shoreline (§7) — but its consequence is that **every other
surface on the planet got 1 m detail while the coastline stayed at bake resolution**. A shoreline
needs relief that is *symmetric* about sea level, able to cut inland as well as build seaward, and
the current one-sided proof forbids exactly that. Resolving this needs a different safety argument,
not a bigger amplitude.

**The cause is the same one this branch keeps rediscovering** — gentle slopes mean `N·L` barely
varies, and nothing casts a shadow, so no dark region can exist. **So the lever worth pulling is the
one that does not depend on steepness: albedo variation.** Patchiness at 5–50 m driven by moisture,
curvature and noise, since slope-driven rock is measured dead below.



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

### 2b. Materials — the slope premise inverted, but not where it was checked

The mountain work moved the material slope metric by a factor of 45. Measured over 40,000 samples,
and the probe spacing matters because the shader central-differences its normal over
`camera_distance × 0.01` clamped to [0.5, 256] m:

| slope `1 − N·radial`, mountain | p50 | max | past the 0.10 rock threshold |
|---|---:|---:|---:|
| §6.2's figure, before | 0.0041 | 0.159 | **0.004%** |
| at a 4 m probe (50 m away) | 0.1935 | 0.4117 | **89.6%** |
| at a 30 m probe (3 km away) | 0.1298 | 0.4380 | 59.9% |
| at a 256 m probe (the cap) | 0.0662 | 0.4360 | **34.5%** |
| the 300 m plain, 4 m probe | 0.0251 | 0.2517 | 4.0% |

So §6.2's "the rock path is wired in and has essentially never fired" is **no longer true**, and it
survives every probe spacing the shader uses — the normal footprint is not swallowing it.

**But this changes nothing at `tour_mountains`, and that was an error in the first recommendation
built on it.** That site is **biome 8**, 517 of 600 samples, with the remaining 83 biome 9. Biome 8
already forces `rock_amount = max(rock_amount, 0.78)` and biome 9 forces `snow_amount ≥ 0.88`
regardless of slope. The site was fully rock-and-snow before the mountain work and still is. The
inverted slope distribution matters on *ordinary* land, which is where it should now be checked.

**What the site actually renders is not what the final image suggests.** `CATINGARDEN_DEBUG_MODE=albedo`
shows the ground as pale desaturated blue-white — saturation p50 0.100, p99 0.183, nothing above
0.25, against earth's palette saturation of ~0.61. The warm yellow-tan of the final image is
therefore **lighting and atmosphere, not ground albedo**. Any attempt to fix the monotony by moving
albedo at this site will be working on the wrong end of the pipe.

*(Also: the pale angular patches here are biome 9, not water as §6.2 guessed. Whether §6.2's
coastline patches elsewhere are water or ice was not re-checked and should not be assumed either way.)*

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

## 6b. The mountains — Cairngorm to Ben Nevis

Ian: *"What we have now is cairngorm mountain or black mountain. What we need is ben nevis or
yr wyddfa."* Measured, that was exactly right, and the number is almost comic:

| at the `tour_mountains` site | before | after | real |
|---|---:|---:|---|
| max relief within 2 km | **313 m** | **1001 m** | Ben Nevis ~1200 m, Cairn Gorm ~300 m |
| max relief within 1 km | 206 m | 669 m | |
| slope p50 / p90 / max | 4.6 / 11.8 / 23.9° | **14.1 / 33.5 / 50.0°** | Ben Nevis flanks 30–40° |
| ground steeper than 25° | **0.000%** | **23.6%** | |
| ground steeper than 35° | 0.000% | 8.5% | |

**Where the relief had to come from.** The baker's working grid is 4096 × 2048 on a 25,000 km
circumference — **6.1 km per cell** — so a Ben Nevis (~5 km across) is below the bake's resolution
entirely and the macro can only ever make 12 km swells. It does: 843 m of relief over 60 km at that
site, a high plateau at 3792–4635 m with nothing sharp in it. Prominence at the scale a mountain is
judged by is therefore the *ladder's* job, not the baker's, and no re-bake at this grid would help.

**What was actually holding it flat, in the order the measurements found it.**

1. **A single roughness makes a self-similar field** — the same character on a plain as on a summit,
   because amplitude is proportional to wavelength at every scale. Real ranges are far steeper at
   massif scale than at boulder scale. Fixed by a **spectral tilt**: `TERRAIN_DETAIL_LONG_GAIN` 8.0
   tapering to nothing by `TERRAIN_DETAIL_TILT_TAPER_METERS` 256 m. The long end is deliberately the
   half that is free — see the cost note below.
2. **The ridge fold was mixed back with the smooth noise it folds** at strength 0.7, rounding off
   every crease. A mountain's defining feature is that its ridgelines are not rounded. Now 1.0.
3. **`TERRAIN_DETAIL_ATTENUATION_SLOPE` was 0.25**, halving every octave on ground past ~14° — it was
   smoothing precisely the crags it was meant to leave alone. Now 4.0. *On its own this is a weak
   knob* (0.25 → 8.0 moves relief 313 → 335 m); it matters only under a raised ladder.
4. **The headroom gate was the real ceiling.** At gain 8 the 4 km octave was being asked for 15.7 km
   of elevation beneath it — more than the planet's highest ground — and ran at 22% amplitude on a
   4.7 km mountain. `TERRAIN_DETAIL_HEADROOM_FACTOR` 8.0 → **5.5**.

**5.5 is not a taste setting; it is the tightest value that keeps the sea-level proof.** The worst
case is not on the mountain but at the shoreline, around 4 m of elevation, where the fine octaves are
all fully admitted and the tilted long ones are gated off entirely. There the ladder admits 0.77 of
the elevation it stands on. At 4.0 it admits **1.06** — a coastline cut below its own sea. The walk
in `outmap_detail_preserves_ocean_and_coastline` is what enforces this; it caught the 4.0 attempt.

**It cost no LOD demand, which was a surprise worth keeping.** `what_the_mesh_drops` measures the
chord residual between vertices — which *is* the geometric error — at 0.027–0.064 of a vertex
spacing, against the 0.230 the selector charges. **The analytic ladder term overstates the real
error by about 4×**, so the new ladder fits inside the existing budget with 3.6× to spare and
`LADDER_GEOMETRIC_ERROR_PER_ROUGHNESS` needed no change. Raster frame time is unmoved: the mountains
are 36.3 ms against 36.9 before. *(That 4× conservatism is a real saving sitting there, but §6.1's
lesson applies — do not spend it without checking the silhouettes, since the flat-constant version of
this budget is what caused the stair-stepping in the first place.)*

**What it cost in the picture, measured on the captures rather than described.** Over the ground
region the luminance p01 falls **0.452 → 0.186** and the spread widens **0.157 → 0.504**, while the
fine shading-gradient RMS is flat (0.00371 → 0.00361). Broad light-and-shade appeared without adding
fine crumple — and that is the same defect §6.2 diagnosed from the other end. Its conclusion was that
no dark region could exist because gentle slopes make `N·L` barely vary, so the only lever left was
albedo. **Steepness turned out to be available after all**, and it delivered the dark end that
ambient could not. §6.2's albedo work is still worth doing; it is no longer the only option.

**Two things this broke, both fixed, both predicted by §9.**

- `path_parity_ridge` put its camera **24.9 m underground** — fixed-position waypoints against ground
  that rose 158 m. Re-authored, clearance back to 133 m. This is §9's "re-author scenario camera
  heights after any ladder change", and it will happen again. Note the scenarios are `include_str!`d,
  so editing the JSON without rebuilding silently re-runs the old one.
- The ray path's hit comb takes `TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS` as its search ceiling and
  spreads a fixed sample count across it. Two faults: the constant was `start × roughness × 2`, which
  the tilt makes wrong by 1.49× because the series no longer halves at the long end (now the measured
  sum, 2646.4 m, re-derived by a test); and six samples across three times the relief is three times
  coarser. `RAY_DETAIL_HIT_STEPS` 6 → 12.

**Open: the ray path is not at parity on the ridge.** `path_parity_ridge` in ray mode reads p90
**10.68 m** against a 6 m tolerance (was 20.2 m before the comb fix, and the raster path holds at
4.24 m on the identical terrain). The comb change also cost 5–8 ms in ray mode. Do not widen the
tolerance to make it pass — the gap between 4.24 raster and 10.68 ray is exactly what the scenario
exists to measure. `RAY_DETAIL_HIT_STEPS` is the obvious next knob and it trades directly against
that 5–8 ms.

**Unrelated but now much better:** ray `tour_mountains`, which §6.1 flagged as reading p90 1529 m at
`detail_correlation` −0.501, now reads **22.6 m at 0.999**. The anomaly was real and terrain-shaped,
not an instrument artefact.

## 7. What the terrain actually is now

`shared_planet.wgsl`, mirrored in `planet.rs`, guarded by
`shader_detail_ladder_matches_the_cpu_clearance_ladder` (which reads the constants back out of the
shader source so the two cannot drift).

```
TERRAIN_DETAIL_ROUGHNESS            0.06      amplitude = roughness × wavelength × tilt
TERRAIN_DETAIL_START_WAVELENGTH     4096 m    starts where the baked data stops
TERRAIN_DETAIL_OCTAVES              13        down to 1 m
TERRAIN_DETAIL_LONG_GAIN            8.0       ┐ spectral tilt: the field is not
TERRAIN_DETAIL_TILT_TAPER_METERS    256 m     ┘ self-similar, massifs beat plains
TERRAIN_DETAIL_RIDGE_SOFTNESS       0.15      sqrt(n² + s²), not abs(n)
TERRAIN_DETAIL_RIDGE_CENTRE         0.348609  ┐ properties of the *softened* fold —
TERRAIN_DETAIL_RIDGE_SCALE          2.063534  ┘ re-derive these if the fold changes
TERRAIN_DETAIL_RIDGE_STRENGTH       1.0       fully folded: creases stay creases
TERRAIN_DETAIL_RIDGE_NORMALISATION  1.0       DERIVED: 1/sqrt((1-s)² + s²)
TERRAIN_DETAIL_ATTENUATION_SLOPE    4.0       multifractal damping by slope so far
TERRAIN_DETAIL_HEADROOM_FACTOR      5.5       per-octave land weighting
TERRAIN_DETAIL_TOTAL_AMPLITUDE      2646.4 m  DERIVED: the tilted series, not 2× its head
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
- **The ridge normalisation is derived from the strength and is not a free knob.** It exists only to
  undo the variance a two-field blend loses, `1/sqrt((1-s)² + s²)`. Setting the strength without
  following it here adds silent amplitude to every octave — a parameter sweep that missed this
  overstated its own result by 31% and nearly became the design premise.
- **Headroom is per-octave, not a scalar land weight.** A 492 m ladder gated on its total reach
  strips a 40 m plain of the 4 m hummocks it does have room for. Each octave asks separately, which
  gives relief-correlated amplitude for free. Factor 8 because each octave alone is safe at 2 but
  thirteen together are not; a test walks elevations asserting the ladder can never reach sea level.

The LOD error budget knows about all this: `OUTMAP_GEOMETRIC_ERROR_RATIO` is
`0.0536 + ROUGHNESS * 2.9395` (`terrain.rs:84-90`), derived rather than tuned. Before that it was a
flat 0.15 — a function of level and nothing else — so a louder ladder never made the selector split
further, which is why raising roughness used to produce stair-stepping.

**The ray path's detail walk is sized by the relief actually present, not the ladder's maximum.**
`refine_detail_hit` finds the detailed surface by combing outward from the macro hit, and the comb
used to span `TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS / incidence`. The hill band took that amplitude
from 16.8 m to **491 m** without rescaling the walk, so a grazing ray combed **1365 m at a time**
hunting a crossing of a field with tens of metres of relief, and landed its hit within ±512 m of the
truth. Because incidence is fixed by viewing angle, and therefore by distance, the resulting quality
steps are **camera-locked while the terrain flows through them** — which is what Ian saw as bands
sliding under one another in a low forward pass.

The first `detail_surface_function` evaluation at the macro hit already returns minus the local
detail height, and a ray closes a height gap at a rate set by its incidence, so `|value| / incidence`
is where the crossing should sit. That is now the span, with the global amplitude kept only as a
ceiling (the ladder can stand taller further along the ray) and an early-out when there is no relief
to find. Measured, ray path, `PRESENT_MODE=immediate`:

| | p90 before | p90 after | frame before | frame after |
|---|---:|---:|---:|---:|
| `stand_on_ground` | 4.48 m | **0.64 m** | 19.9 ms | 31.5 ms |
| `path_parity_ridge` | 7.25 m | **3.47 m** | 31.5 ms | 34.4 ms |
| `low_pass_bands` | 7.24 m pooled | **5.05 m pooled** | 31.9 ms | 33.8 ms |

**The landing site costs +11.6 ms because it is now doing work it previously failed at.** With the
old span the comb never bracketed a crossing and fell back to the macro hit after six wasted
evaluations; with the correct span it brackets and bisects, so the three refinements actually run.
Raster is untouched by all of this (the code is ray-only) and re-measures bit-identically at 0.25 and
1.93.

**Do not assume this closed Ian's complaint — the rendered image says otherwise.** Per-row horizontal
detail energy over the `low_pass_bands` captures is *unchanged* by the fix: the same rows (343, 350,
353, 363–369, 375) carry the same 15–24% jumps before and after. Height accuracy improved a lot; the
visible row structure did not move. Either the metric is measuring the site's terrain gradient rather
than an artefact, or the bands have a second cause. **A static per-row profile also cannot see the
symptom as described** — Ian's report is about motion, detail flowing toward the camera faster than
the edge it flows under. The instrument that would settle it asks whether the detail-quality profile
stays fixed in screen space while image content shifts between frames; that has not been built.
Cheapest oracle remains asking Ian to fly it.

**The raymarch path's near-field window** (`terrain.rs` / `foveated.rs`) is an 8×8 block of L12 tiles
resampled into a 1025² R32Float texture, because L12 already reads within 0.5 m of L18 at the landing
site and an L12 tile spans 1.5 km. It refuses to enable unless every block is finer than
`dense_level`, so outside the sparse corridor it costs nothing. It rebuilds on `NearFieldSources`
(the resolved tile keys), **not** on camera position — a stationary camera keeps the same square
while streaming replaces coarse ancestors underneath it.

---

## 8. Do not redo these

Each was built, measured, and rejected on evidence.

- **Cast shadows — but this verdict is now stale evidence, see below.** Prototyped and reverted:
  +8.2 ms for 0.12% of pixels changed. Shadows need grade > tan(sun elevation), and below ~8° the
  atmosphere has already taken the ground to near-black, so the window was empty.
  **That was measured at roughness 0.0328, before the 0.06 change and the hill band — i.e. the "new
  terrain" its own do-not-rebuild condition asked for has since arrived.** Slope statistics
  (`1 − dot(normal, radial)`) roughly quadrupled at the tail:

  | | p50 grade | p99 grade | max grade | p99 casts below |
  |---|---:|---:|---:|---:|
  | roughness 0.0328 | 9.1% | 21.2% | 29.6% | 12.0° sun |
  | roughness 0.06 | 15.4% | 34.2% | 64.3% | **18.9° sun** |

  So the top 1% of terrain now has a genuine ~8–19° window instead of essentially none. **Re-measure
  before rebuilding, and expect the cost objection to have got worse, not better** — 8.2 ms lands on
  a frame already ~5 ms over budget (§6.4). The geometric argument weakened; the budget argument
  hardened. Do not rebuild on the strength of the table alone.
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

- **Commit and push after each set of changes** (`AGENTS.md`). The branch is pushed and `origin` is
  level with local; keep it that way rather than letting a stack build up again.
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

# Handoff — ground readability / render modernisation

**Branch:** `experiment/flat-triangle-wireframe`
**Branch base:** current `diagnose/ocean-terrain-blockiness`; this session's work is isolated on
the experimental branch and is to be pushed to `origin/experiment/flat-triangle-wireframe`
**Renderer state:** current branch — positive baked macro land uses one fixed 4x presentation scale
at every camera altitude; sea level and negative bathymetry remain physical. CPU truth, raster, ray,
LOD/culling bounds, shader normals, and scenario cameras share that transform. The former 8x
long-wave procedural-detail gain is disabled (1x) so the observed ETOPO shape can be evaluated
without a dominant pattern of random basins; the finer detail system remains intact. F4 starts over the measured
highest-prominence summit. Moving raster flight sweeps the camera through every concurrently drawn
active/transition patch, rechecks the newly selected destination frontier before presentation, and
retains a 5m moving collision envelope; F4 entry is 2m and idle inspection can settle to 0.75m. The raster culling shell includes
the complete live detail ladder, so elevated near-camera vertices cannot disappear outside a
macro-only bound. Key 6 switches between auto exposure and fixed 1.0.
The fullscreen sky removes the direct-scattering model's green-dominant colour crossover and adds
a bounded indirect Rayleigh approximation for a blue hour that fades into night.
Raster terrain now carries aerial transmittance and in-scatter independently from vertex to
fragment, so low-sun shadows cannot cross a per-channel reconstruction threshold and become bright
islands with dark outlines. Terrain ambient remains the local overhead sky radiance scaled by 0.18.
Raster land currently uses the displaced smoothed vertex normal for close-snow lighting. The prior
flat geometric-normal trial is retired because fallback facets became repeated black strips at
122m altitude; height, LOD, source sampling and the ray path are unchanged.
**Latest evidence:** the orientation-corrected, fixed-4x ETOPO terrain passes raster
`orbit_once/1785599097-127619`, ray `orbit_once/1785599236-129103`, raster
`stand_on_ground/1785599119-127881`, raster `landing_site_ground_detail/1785599181-128543`, raster
`landing_site_eye_level/1785599299-129789`, raster
`highest_prominence_peak/1785599150-128156`, and ray
`highest_prominence_peak/1785599507-134390`. A cold raster orbit first exposed the known fallback
warm-up seam at 0.5-1.0s, then the immediate repeat passed with zero seam. The lifted-budget raster
`low_flight_performance/1785599348-130207` remains a known failure: 420 resident chunks, 334
fallbacks and a 2,071.204m warm-up seam. Older renderer-only evidence is retained in the relevant
sections below but uses a previous macro/detail presentation. The later raster low-poly trial passes
`orbit_once/1785600758-149017`, `landing_site_ground_detail/1785600646-146796`, and
`highest_prominence_peak/1785600765-149139`; see its section below for the visual findings.
**Written:** 6 August 2026
**Supersedes:** `PLANET_SIM_HANDOFF.md` at the repo root, which describes the 19 July low-flight
state and is now history. Read `AGENTS.md` for the architecture; read this for where the work is.

### Experimental branch — fixed-L6 flat triangle wireframe baseline (4 August 2026)

Branch `experiment/flat-triangle-wireframe` is an isolated visual experiment from the current
diagnosis branch. It intentionally fixes the raster terrain and ocean quadtree at the minimum
`L6` level (four refinements above the normal L2 floor) and disables the 40x40 near-field mesh, leaving the stable 32x32 chunk topology. The
fragment path bypasses the material/albedo texture stack and assigns one categorical biome (or
ocean) palette colour to each triangle, with a dark antialiased edge, so the planet reads as
filled wireframe rather than interpolated textured terrain. A derivative-based geometric face
normal now supplies analytic diffuse sky/direct-sun lighting and a direct-sun specular lobe for
both land and ocean; the flat path samples neither material nor environment textures. Height and
biome outmaps remain CPU/GPU data sources for geometry and ownership; this mode does not pretend
the baked data disappeared.

The branch defaults to `flat L7 triangles` (`RenderDebugMode::FlatTriangles`). Press `O` to
toggle the dark per-triangle outlines at runtime; they start enabled and the HUD reports the
current state. This only changes the edge mask; flat fills, geometric normals, lighting, ocean
shading, and fixed-L7 geometry remain unchanged. Set
`CATINGARDEN_FLAT_TRIANGLES=0`/`false`/`off` to restore normal LOD selection, or set
`CATINGARDEN_DEBUG_MODE=final` to inspect the normal material shader while keeping the branch's
fixed-L7 policy. The ray renderer is not replaced by this raster-only presentation experiment.

The L3 baseline and L4 follow-up used identical release settings and four deterministic camera
scenarios. FPS is `1000 / median logged spatial frame time`; samples include the scenario's normal
warm-up and streaming frames.

| camera position | L3 run | L3 median ms / FPS | L4 run | L4 median ms / FPS | FPS change |
|---|---|---:|---|---:|---:|
| orbit, 6,000km | `1785834959-63633` | 16.081 / **62.19** | `1785840997-116022` | 24.534 / **40.76** | −34.5% |
| landing-site ground detail | `1785835058-64558` | 7.890 / **126.74** | `1785841004-116101` | 6.563 / **152.37** | +20.2% |
| landing-site eye level | `1785835065-64615` | 6.893 / **145.07** | `1785841010-116163` | 6.400 / **156.26** | +7.7% |
| highest-prominence pose | `1785835071-64683` | 6.867 / **145.62** | `1785841016-116218` | 8.095 / **123.53** | −15.2% |

The orbit L3/L4 runs fail the existing seam assertion during source streaming (2,273.963m and
2,663.899m maxima respectively); the prominence runs fail their fixed 150m clearance assertion
because a globally fixed coarse mesh does not reproduce the resident summit height (2,120.676m at
L3 and 123.713m at L4). The landing-detail and eye-level runs pass. These are diagnostics, not
reasons to weaken either assertion. The focused shader, debug-mode, fixed-policy, and speed tests
pass; `cargo test --workspace` passes **233 tests** (192 app, 27 baker library, 2 baker binary,
6 baker integration, 6 coretypes; 6 ignored diagnostics). F4 flight speed is globally 5x the prior
fixed altitude-scaled value: 250mph at ground level and a 40,000km/s cap, while Shift retains its
existing 4x multiplier.
This branch is for visual evaluation only and must not be merged as the normal textured renderer
without an explicit decision.

The previous fixed-level trial was **L5** (`FLAT_TRIANGLE_LOD_LEVEL = MINIMUM_LOD_LEVEL + 3`). Orbit
replay `1785841288-120629` reaches the 256 active-chunk budget, with a median **23.220ms / 43.07
FPS** across nine logged spatial frames. It still fails the seam assertion at **2,273.963m** during
source streaming; finite metrics and the full workspace test suite remain green. This confirms that
raising the global triangle LOD does not repair the underlying source-residency discontinuity.

The L6 run is historical; the current trial is recorded below.

### Game-terrain / fixed-L7 density trial — 6 August 2026

The active ETOPO outmap now uses the baker's opt-in `--game-terrain` profile. It leaves the planet
radius, coastlines, sea level, bathymetry, and water ownership unchanged, amplifies positive land by
**1.6x**, and adds deterministic ridged bands at 150x and 520x direction frequency with a 2,400m
cap. The relief is baked into the shared height source so CPU clearance, raster displacement,
normals, ray height fields, and LOD bounds consume the same denser mountain surface. The active
manifest is `70855c129545aead5e69731cfc77988d697b76e317e8623845ba0f44c1b9937f`; the prior active
outmap is preserved at `assets/outmaps/test-planet.pre-game-terrain-backup-20260806-101349`.

The global summit survey now measures a raw 9,000m macro summit and a 36,200.028m presented
summit/prominence at 35.251182N, 77.095728E. F4 and `highest_prominence_peak` were re-authored to
that mountain; the deterministic summit run passes at 151.711m clearance after the fixed-L7
camera pose uses the rendered L4 source height.

The flat-triangle experiment now fixes geometry at **L7** (`FLAT_TRIANGLE_LOD_LEVEL =
MINIMUM_LOD_LEVEL + 5`) and bypasses the source-repeat cap only for this presentation. The L4
source remains authoritative; L7 simply supplies a denser 32x32 facet grid so game-terrain peaks
span materially more than a handful of triangles. Raster `orbit_once/1786008682-1190692` passes
at 23.884ms settled with 0.000488m seam, `highest_prominence_peak/1786008708-1190914` passes at
20.087ms with 151.711m clearance, and `landing_site_ground_detail/1786008753-1191247` passes its
finite/LOD assertions. Fresh low-flight visual sign-off is still pending because the known
mixed-LOD foreground dark facets remain visible in the flat experiment.

### Zoomed game-terrain rebake — 6 August 2026

The previous game bake still read as Earth at planetary scale, so the baker now has an explicit
`--zoomed-terrain` profile. It remaps a compact **48° longitude by 36° latitude** window centred
on the Himalaya/India/Tibet source (35°N, 77°E) across the planet with smooth periodic sine
coordinates: four repeats around longitude and two around latitude. Repeat boundaries return to
the source-window centre rather than introducing hard equirectangular seams. The profile implies
`--game-terrain`, then applies a deliberately exaggerated **2.6x** positive-land lift and a
**7,000m** ridged uplift gated from 300m to 2,100m source elevation; oceans, sea level, and
bathymetry remain untouched.

The active outmap is the validated 3,252-tile L0-L18 bake at
`assets/outmaps/test-planet`, with manifest SHA-256
`998e55e8b89e88de3feef90c7d9547e5655d745399e726a85d1aaa1552a01b9c`. On the dense L4 samples,
the fraction above 5,000m increased from 1.76% in the prior game bake to 2.33% in this profile;
the raw export ceiling remains 9,000m and the existing runtime 4x presentation makes those peaks
up to roughly 36km above sea level. The prior active copies are retained at
`assets/outmaps/test-planet.pre-zoomed-game-terrain-backup-20260806-104956`,
`assets/outmaps/test-planet.pre-zoomed-game-terrain-v2-backup-20260806-110444`, and
`assets/outmaps/test-planet.pre-zoomed-game-terrain-v3-backup-20260806-111636`.

The global prominence survey now finds 36,200.491m at 19.296266°N, 93.023061°W. F4 and
`highest_prominence_peak` use that repeated mountain; the summit scenario passes at 151.711m
clearance with 0m seam delta. Raster `orbit_once/1786011609-1231134` passes at 21.624ms settled
and raster `highest_prominence_peak/1786011581-1230898` passes at 13.604ms. The screenshots still
show the experimental flat-triangle renderer's known mixed-LOD foreground facets; this bake pass
changes the macro geography/relief, not that separate presentation fault.

### Anti-symmetry domain warp — 6 August 2026

The first zoomed bake was seam-safe but visibly too regular: each periodic window was an identical
mirror-like copy. The mapping now adds low-frequency global phase terms and cross-coupled harmonics
to both source coordinates. These terms are themselves globally periodic, so the dateline and all
sample joins remain continuous, but each longitude/latitude repeat receives a different phase and
orientation instead of repeating the same mirrored silhouette. The baker regression explicitly checks
both boundary continuity and non-identical repeat samples.

The active validated outmap is `assets/outmaps/test-planet`, with 3,252 L0-L18 tiles and manifest
SHA-256 `f18e1b7302f934c27d4a67f64a5a1484a2329d2e645f8ae1de4a1e6227524008`. The prior active
anti-symmetry bake is preserved at
`assets/outmaps/test-planet.pre-domain-warp-backup-20260806-114240`; earlier zoomed backups remain
available above. The global summit survey now measures 36,199.808m at 18.035969°N, 72.011034°E.
F4 and `highest_prominence_peak` were re-authored to that copy; the summit run passes at 151.711m
clearance and 0m seam. Raster `orbit_once/1786013213-1254038` passes at 25.193ms settled, and
`highest_prominence_peak/1786013173-1253680` passes at 17.823ms. The orbit capture now has varied
repeat shapes rather than exact mirrored bands, while retaining the intentionally game-like global
zoom and sharp mountain relief.

### Procedural eroded game planet — 6 August 2026

The repeated Earth/ETOPO source is no longer the active terrain. The baker now
has an explicit `--procedural-terrain` profile that samples continuous 3D
direction-domain noise on the sphere: warped continent masks, broad highlands,
regional mountain fields, ridged peaks, and fine breakup. It has no geographic
ellipses, ETOPO input, compact-window remapping, or mirrored/repeated source;
the existing hydraulic erosion, thermal erosion, drainage, rivers, lakes,
glacial valleys, moisture, and biome stages still run afterward. The profile
rejects `--etopo`, is deterministic/seam-safe, and has a regression for finite,
bounded, asymmetric land/ocean and mountain variation. The procedural shape is
already game-scaled, so the ETOPO-only relief multiplier is not applied a second
time.

The active validated 3,252-tile outmap is `assets/outmaps/test-planet`, with
manifest SHA-256
`e26a75faec284c3d519f84ed2c9b97b397001be71dfb91a95d62237763efa2fa`.
The 4096x2048 bake used 256 erosion iterations (enough to retain the full
erosion/hydrology pipeline without the multi-minute 2,048-iteration thermal
run). The prior repeated procedural-relief pass is retained at
`assets/outmaps/test-planet.procedural-relief-pass-backup-20260806-130200`,
and the prior anti-symmetric ETOPO-derived surface remains at
`assets/outmaps/test-planet.pre-procedural-backup-20260806-125500`.
The height and biome previews show non-repeating continents, coherent mountain
regions, and no global mirror axis.

The standard global summit survey now measures a 31,449.742m presented summit
at 56.684679°N, 20.618054°E (raw L4 maximum 7,821.087m). F4 and
`highest_prominence_peak` were re-authored to this summit; the raster scenario
passes at 152.400m clearance with zero seam delta in
`highest_prominence_peak/1786018012-1292807`. All 196 non-ignored app tests,
all 33 baker-library, 5 baker-binary, and 6 baker-integration tests pass. The
full workspace suite was not rerun because the repository contains several
large untracked target trees and the session is disk-pressure constrained.

### Narrow-ridge prominence pass — 6 August 2026

The first procedural bake had the right global asymmetry but its mountains still
spread their height over broad, smooth regions. The procedural generator now
keeps a broad mountain-region mask and adds a separate thresholded, cubed
ridged fold at frequency 11 with a 7,000m cap. That narrow spine is added before
the same erosion/hydrology pipeline, so the new peaks are steep and locally
prominent rather than a second low-frequency blanket. The baker test pins a
greater-than-250m neighbour-scale summit prominence on the deterministic field;
the continuous direction-domain sampling remains seam-safe.

The active validated outmap is `assets/outmaps/test-planet`, with manifest
SHA-256 `c0bb366d115d14abbff71917a4b8f3cf93e82b1437166e6aba7ad190127bcf59`.
The generator profile is now recorded as `procedural-game-terrain narrow-ridge
relief v2`. The immediately prior procedural surfaces remain at
`assets/outmaps/test-planet.procedural-narrow-5500-backup-20260806-140234`,
`assets/outmaps/test-planet.pre-narrow-mountains-backup-20260806-135650`, and
the earlier backups above.

The global summit survey now measures **35,302.559m** presented prominence at
23.485719°N, 62.546713°W (raw L4 maximum 8,820.148m), up from 31,449.742m on
the first procedural bake. F4 and `highest_prominence_peak` were re-authored;
raster `highest_prominence_peak/1786021735-1323655` passes at 152.400m
clearance and zero seam. The final height/biome previews show narrow bright
mountain spines over the non-repeating continents. Baker tests (33 library, 5
binary, 6 integration) and all 196 non-ignored app tests pass.

### Polar cap and flat-water correction — 4 August 2026

The imported ETOPO classification no longer lets the old `|latitude| > 66°` rule turn polar ocean
into a circular ice-coloured land cap. Ocean and lake ownership now wins before land-ice
classification. ETOPO's single elevation band has no explicit lake mask, so positive priority-flood
basin cells are not promoted to water; disconnected negative components use cardinal connectivity
and a 512-cell production floor, retaining major lakes while removing sub-resolution square ponds.
The active corrected bake is `assets/outmaps/test-planet`; its previous active copy is preserved at
`assets/outmaps/test-planet.pre-polar-water-repair-20260804-164036-active`, and the active manifest
SHA-256 is `2659f33bbcbcda5171ca851669007d93115347c44fe04bfdad99a26bc080dcce`.

The land-ice fallback is now an authored Greenland/Antarctica footprint over positive observed ETOPO
land, with the existing elevation snowline retained for high terrain. In flat-triangle mode the ice
palette fades by the provoking triangle latitude (each triangle still has one final colour). Flat
mode and the shared raster/ray water path now use a zero-displacement, radial-normal shell: ocean
and retained lakes are exactly sea level, with no Gerstner vertical/horizontal displacement or
raised/angled water facets. The CPU wave diagnostic remains for comparison, but GPU water is flat.
The old active bake remains available for comparison; fresh GPU/manual capture sign-off is still
required.

Flat-triangle terrain specular is evaluated once in the vertex stage and carried in the flat
interpolant's scalar component, so every land triangle uses one specular value rather than a
per-pixel highlight. Diffuse lighting and derivative face normals remain per the prior flat
experiment; ocean specular keeps its existing path.

### Game-terrain macro rebake — 6 August 2026

The active ETOPO outmap now uses the baker's opt-in `--game-terrain` relief profile. It leaves the
planet radius, coastlines, sea level, bathymetry, and water ownership unchanged, amplifies positive
land by **1.6x**, and adds deterministic ridged bands at 150x and 520x direction frequency with a
2,400m cap. The extra relief is baked into the shared height source rather than added only in the
renderer, so CPU clearance, raster displacement/normals, ray height fields, and LOD bounds all see
the same denser mountain surface. The profile is recorded in the manifest as `game-terrain relief
v1`; reproduce it with `--etopo ... --game-terrain`.

The validated active bake is `assets/outmaps/test-planet` with 3,252 L0-L18 tiles and manifest
SHA-256 `70855c129545aead5e69731cfc77988d697b76e317e8623845ba0f44c1b9937f`. Its raw working-grid
range is -5,000m to the 9,000m export ceiling; positive land is 34.059%, and 4.002% of samples are
above 5,000m (the preceding ETOPO bake had 0.166%). As a density proxy at the 4096x2048 source
resolution, the new >5,000m mask contains 501 connected components of at least 10 samples and 167
of at least 30 samples, versus 43 and 11 respectively before the profile. This is intended to make
mountain masses occupy many more fixed-L7 surface facets instead of appearing as isolated handfuls.

The prior active outmap is preserved at
`assets/outmaps/test-planet.pre-game-terrain-backup-20260806-101349`; do not delete retained bake
directories without explicit instruction. The global-summit instrument now measures a raw 9,000m
macro summit and a 36,200.028m presented summit/prominence at 35.251182N, 77.095728E. F4's
highest-prominence start constants and `highest_prominence_peak` scenario were re-authored to this
location and its 152.4m entry pose. Fresh GPU/manual capture sign-off remains required; the source
and baker validation are green.

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
cargo test --workspace   →  226 passed, 0 failed, 6 ignored
                            (app 186, baker lib 26, baker bin 2, baker integration 6, coretypes 6)
                            the 6 ignored are the relief_survey/terrain instruments -- run them with
                            `cargo test -- --ignored --nocapture <name>`
```

### Active NOAA ETOPO 2022 baked planet — 1 August 2026

`assets/outmaps/test-planet` now uses NOAA's whole-world **ETOPO 2022 Ice Surface, 60 arc-second**
GeoTIFF as its observed macro height source. Full provenance, DOI, reproduction command, source
limits and attribution are in `docs/ETOPO_2022_SOURCE.md`. The ignored local source is
`assets/source-data/etopo-2022/ETOPO_2022_v1_60s_N90W180_surface.tif`, 465,969,062 bytes, with
SHA-256 `9d27d4b8ea8e76977e2988bca667d7c8fa68b927355feffcddd6b4875a7fd08e`.

The exact pre-ETOPO active bake is preserved at:

```
assets/outmaps/test-planet.pre-etopo-backup-20260801-140800
```

It is 370 MB / 9,760 files, validates as schema 2 with 3,252 tiles, and its manifest SHA-256
`be1b352f157c55eeb9d91e8601504db9a004f43c27e4c481cbee648b623cdfe1` matches the former active
manifest recorded before promotion. Do not delete it or the earlier retained bake directories
without Ian's explicit instruction.

The rejected all-positive-land peak-envelope bake is also retained, rather than deleted, at
`assets/outmaps/test-planet.etopo-all-land-peak-retired-20260801-142959`. Its low-flight replay made
the reason for the high-range gate objective: max-filtering ordinary land produced broad terraces.

The first ETOPO bake, before correcting its east/west display orientation, is preserved at:

```
assets/outmaps/test-planet.pre-etopo-orientation-fix-backup-20260801-163936
```

It is 371 MB / 9,760 files and validates with manifest SHA-256
`f1915868a3be65be1ab07ed2d8713241a658fbf91efab58f8d601e18d681f88f`.

`--etopo PATH` is optional: without it the authored generator remains byte-compatible. With it, the
baker validates a signed, finite, whole-world 2:1 grayscale grid, reverses NOAA's north-up rows onto
the baker's south-up grid, and reverses its west-to-east columns onto the renderer's geographic-east
= -Z convention. This is necessary because a north-up camera looking inward from +X has screen-right
along -Z; copying source columns forward put India west of Africa in orbit. Biome aridity masks use
the same real-world longitude transform. The importer bilinearly resamples the coastline,
bathymetry and ordinary land.
Peak retention fades in only from 4,000-6,000m, reaching the highest observed source elevation in a
target footprint for the major ranges; this prevents a narrow summit from disappearing between the
4096x2048 working-grid sample points without max-filtering lower terrain into broad terraces. ETOPO
is already naturally eroded, so this path keeps its heights unchanged while deriving flow, river/lake masks,
moisture and biomes; it does not run the authored hydraulic/thermal erosion or river/glacier height
carving. The existing -5,000m bathymetry floor, +9,000m ceiling, L3+ seam-safe baked detail and sparse
detail export remain in force.

The active bake uses seed `0xEA272026` (`3928432678`), a 4096x2048 working grid, global dense L4
coverage and sparse parent-complete refinement through L18. It validates all 3,252 schema-2 tiles,
occupies 371 MB / 9,760 files, has manifest SHA-256
`2659f33bbcbcda5171ca851669007d93115347c44fe04bfdad99a26bc080dcce`, and selected sparse centre
`[-0.504183, 0.008437, -0.863556]`. Preview measurements are unchanged because the correction is an
exact horizontal mirror:

- positive land: 34.053%; full working-grid range: -5,000m to 8,157m;
- 553,673 pixels above 2,400m, including 13,925 above 5,000m and 37 above 7,000m;
- biome coverage: ocean 53.082%, ice 27.365%, temperate forest 9.842%, tropical forest 4.436%, lake
  2.873%, desert 1.276%, grassland 0.708%, tundra 0.221%, mountain rock 0.145%, and mountain snow
  0.054%.

The baker's preview row zero is the south pole and target x follows the engine's +Z axis, so flip it
vertically and horizontally for a conventional north-up/east-right review. Pixel comparison proves
the corrected height, biome and moisture previews are exact horizontal mirrors of the first bake
after accounting for those display transforms. The full-resolution height and biome previews and
raster orbit captures show the real continent silhouettes and coherent Andes, Rockies, Himalaya and
other ranges. The existing `latitude > 66° => Ice` rule classifies polar ocean as ice as well as
polar land, which explains the large ice percentage and is not new to this bake.

Reproduce into a staging path and validate before promotion:

```bash
CARGO_TARGET_DIR=/home/dad/catingard-target cargo build --release -p catinthegarden-baker
RAYON_NUM_THREADS=1 nice -n 10 /home/dad/catingard-target/release/catinthegarden-baker \
  --output assets/outmaps/test-planet.etopo-staging-YYYYMMDD-HHMMSS \
  --etopo assets/source-data/etopo-2022/ETOPO_2022_v1_60s_N90W180_surface.tif \
  --width 4096 --height 2048 --dense-level 4 --max-level 18
/home/dad/catingard-target/release/catinthegarden-baker \
  --validate assets/outmaps/test-planet.etopo-staging-YYYYMMDD-HHMMSS
```

The orientation-corrected bake took 82s with one Rayon worker for thermal safety. The outmap and source data are
intentionally gitignored; the importer, provenance, tests and reproduction command are the durable
repository state. The new sparse-site L18 ground is approximately 909m raw / 1,818m at the fixed-2x
presentation. The four manifest-relative landing scenarios preserve their relative framing after
this surface move. Raster `stand_on_ground/1785599119-127881` measures 1.973m clearance, 0.169m
worst-frame p90 and 1.272m maximum surface delta. The ground-detail and eye-level scenarios also
pass; the latter spans 1.973-64.395m clearance.

The old `low_flight_performance` budget/fallback/seam defect is not fixed by the rebake. At the new
sparse coast, lifted-budget run `1785599348-130207` is finite and has zero budget-limited frames and
zero LOD thrash, but fails its established limits at 420 resident chunks, 334 fallbacks and a
2,071.204m warm-up seam. Its settled mean is 40.155ms versus the first ETOPO bake's 40.463ms, so the
change is timing-neutral. Its settled capture no longer has the rejected broad all-land terraces,
but still shows the known source/LOD transition boundary; do not present this scenario as signed
off.

### Fixed 2x ETOPO and unboosted detail baseline — historical

Positive ETOPO height now presents at exactly **2x** at every altitude; bathymetry and sea level stay
physical. The CPU surface, raster displacement, raster central-difference normal probes, ray hit
shells, ray centre/east/north normal probes, LOD/culling bounds and scenario cameras all consume the
same scale. Regressions pin the 2x uniform and prove all four raster normal samples plus all three ray
normal samples pass through `scaled_terrain_macro_height`, so lighting sees the steeper 2x gradient
rather than the raw source gradient.

### Fixed 4x ETOPO and fixed-speed altitude-scaled flight — 1 August 2026

The requested second elevation doubling is now active: positive baked ETOPO land presents at exactly
**4x** at every altitude. Sea level and negative bathymetry remain unchanged. CPU clearance,
raster/ray displacement and normals, LOD/culling bounds, and camera scenarios all consume the
shared positive-only transform. The global highest summit now measures **30,853.047m ASL/prominence**
at **27.990111N, 86.981339E**; F4 and landing scenarios were re-authored and pass the updated
clearance checks (`stand_on_ground/1785601781-159156`, `landing_site_ground_detail/1785601794-159268`,
`landing_site_eye_level/1785601979-161573`, `highest_prominence_peak/1785601895-160271`).

Interactive F4 WASD no longer accelerates, coasts, or brakes: holding a movement key applies a
fixed speed immediately and releasing it stops immediately. The baseline is 50 mph at ground level;
speed scales as `50 mph * (1 + altitude / 100m)`, with Shift multiplying by four and a finite
8,000km/s cap. This keeps local angular/apparent motion approximately comparable while allowing
rapid planetary travel from high altitude.

F4 now enters at **10m** above the resident summit surface instead of the former 152.4m/500ft
entry height. The 30m moving collision envelope remains conservative until the mixed-LOD source
frontier is fully resident.

The active outmap was rebaked from the preserved NOAA ETOPO source after classifying disconnected
negative-height components as inland lakes; the previous active bake is preserved at
`assets/outmaps/test-planet.pre-inland-lake-repair-20260801`.

The follow-up shading pass makes terrain distance fog use the camera's actual sky ray, clamps the
fine-detail normal relight gain to **0.55–1.75**, and gates lake/ocean body sky diffuse by direct
daylight. This removes the horizon fog mismatch, bright coarse fallback patches, and moonless lake
glow without changing the baked heights or lake coast geometry.

### Terrain detail step 1 — bounded raster near-field source priority

Low-flight raster updates now opportunistically prefetch up to four tiles per frame from the existing
bounded 8×8 near-field source window, after ordinary visible-node loads claim the queue. The request
is keyed to camera direction and clearance and is active only below the existing 250km source-limit
bypass. Inside the sparse high-resolution corridor it gradually warms nearby fine sources; outside
that corridor the manifest resolves the request to the already resident dense ancestor, so it is
effectively free. Visible raster geometry is never starved by this prefetch.
This is intentionally a source-residency step, not a geometry or height-scale change: the quadtree,
mesh density, runtime detail ladder, fallback policy, and ray window remain unchanged. The next
validation is a raster low-flight replay comparing fallback count, source-level histogram, seam delta,
probe p90/max, and frame time at 10m/100m/1km/10km before expanding the clipmap or adding directional
relief.

### Terrain detail step 2 — altitude ladder capture

The deterministic raster scenario `terrain_detail_altitude_ladder` now holds the sparse landing
direction at approximately 10m, 100m, 1km, and 10km clearance and captures each level. Committed
run `1785675632-700618` passes finite-metric and zero-thrash checks. Measured screenshot/probe rows:

| clearance | frame ms | fallback chunks | seam delta | probe p90 | probe max |
|---:|---:|---:|---:|---:|---:|
| 10m | 39.31 | 256 | 0.000m | 7.645m | 11.958m |
| 100m | 35.47 | 58 | 0.000m | 0.539m | 1.801m |
| 1km | 38.63 | 84 | 4.530m | 1.351m | 4.222m |
| 10km | 28.68 | 139 | 0.000m | 8.112m | 29.034m |

The 10m frame is a cold-streaming frame; the 1km row briefly crosses a 4.53m seam during source
replacement; and the 10km screenshots visibly expose coarse rectangular source/LOD transitions.
The ladder therefore confirms the measured source-coverage limit rather than signing off more
procedural noise. Step 3 should address those transitions with bounded near-flight geometry/source
coverage before adding directional relief.

### Terrain detail step 3 — bounded raster near-field source window

Raster low flight now reuses the existing camera-centred 8x8 near-field assembly rather than binding
each fine chunk to whichever sparse ancestor happens to resolve for it. A 1025x1025 R32F/R8/R8
window is uploaded when its resident source set changes; every fully covered fine chunk remaps its
face UV into that common window and batches against one bind group. Orbit, ocean ownership, and
chunks outside the window remain on the ordinary per-tile path, and the source/fallback counters
still describe the quadtree rather than pretending the sparse bake became denser.

Replay `1785677861-724323` removes the settled 1km/10km rectangular source blocks in the screenshots.
At the settled 10km row, draw calls fall from 115 to 10 and frame time from 28.68ms to 24.06ms;
the diagnostic fallback count remains 139 because it intentionally measures the underlying sparse
source frontier. Cold first frames can still show the old fallback until all 64 window blocks are
resident. The next review should check low-altitude residency latency and the window's material
continuity before adding directional relief.

### Terrain detail step 4 — bounded near-field geometry density

The source window removed the largest rectangular source blocks, but the fixed 33x33 chunk grid
still under-sampled a covered source patch at the coarser near-flight levels. Raster terrain batches
whose nodes are fully inside the near-field window and are at L10 or coarser now use a shared 40x40
grid (41x41 top vertices plus skirts). Ordinary per-tile chunks, orbit, and the analytic ocean shell
remain on the original 32x32 grid. The selector, 256-leaf budget, height data, LOD transitions and
CPU clearance are unchanged; the dense grid is capped at L10 so ground-level L18 flight does not
pay a fourfold triangle cost.

Replay `1785681886-771172` passes the altitude ladder with zero LOD thrash and zero seam delta. At
the equal 13.6km settled view, the warm settled mean is **25.20ms** versus **24.42ms** for the
step-3 baseline (**+0.78ms / +3.2%**); terrain triangles rise only **460,800 -> 491,200** and draw
calls **10 -> 11**. The change is therefore a bounded geometry-density improvement, not a global
mesh rewrite. Fresh visual review should decide whether the modest extra sampling is visible before
raising the cap or moving to directional relief.

To expose the observed Earth form before designing procedural terrain with geographic direction,
`TERRAIN_DETAIL_LONG_GAIN` is **1.0 instead of 8.0**. This removes the extra long-wave spectral boost
without deleting the detail ladder, hash, ridge shaping, source handoff, normal perturbation or CPU/GPU
agreement. The longest 4,096m octave falls from 1,966.08m to 245.76m maximum amplitude (−87.5%), and
the complete 13-octave absolute bound falls from 2,646.4m to **491.5m** (−81.4%). This is deliberately
an evaluation baseline, not the final mountain-detail design; add large relief back later with
terrain-aware direction rather than another global random gain.

### Raster low-poly gradient trial — 1 August 2026

Raster land now derives the rendered triangle's geometric normal from `dpdx`/`dpdy` of the
camera-relative view position, transforms it back into planet space, and orients it outward. That
one face normal drives direct light, local-sky fill, rock/snow slope ownership, triplanar projection
and the base for finer-than-mesh detail relighting. It therefore stops interpolating the displaced
central-difference normal across triangle edges and makes the current mesh gradients explicit.

This is deliberately a shading-only trial. It does **not** change baked or runtime height, bilinear
height reconstruction, the 2x positive macro scale, LOD selection/topology, skirts, CPU clearance,
coastline ownership, aerial perspective, or the outmap. The foveated ray renderer has no raster
triangles and remains on its existing height-field normal path. Raster skirts and almost perfectly
vertical gap-closing faces retain the displaced fallback normal rather than presenting filler
geometry as authored cliffs.

Current Quadro raster runs, release binary and `CATINGARDEN_PRESENT_MODE=immediate`:

| scenario | run | result | visual finding |
|---|---|---|---|
| `orbit_once` | `1785600758-149017` | pass, four captures | global presentation remains stable |
| `landing_site_ground_detail` | `1785600646-146796` | pass, two captures | fine facets plus a conspicuous tall source/LOD transition face |
| `highest_prominence_peak` | `1785600765-149139` | pass, two captures, 152.4m clearance | broad mountain facets are visible; dense foreground facets and 252 fallback chunks remain conspicuous |

The first controlled ground-detail pair was timing-neutral (smooth **31.913ms**, faceted
**31.898ms** mean from 4-8s); the single latest samples vary normally and are not a performance
claim. The important result is visual: hypothesis 1 was correct, because removing normal
interpolation exposes sharp triangle boundaries without moving the silhouette. It also exposes
implementation structure that the smooth normal hid. The ground scenario still reports zero LOD
thrash and zero logged seam delta, so its tall central face is not evidence that the ETOPO source
contains a corresponding ridge. Do not tune or rebake the source to fit that face. Human review now
decides whether this deliberately hard treatment is the desired style; if it is, source/LOD
transition presentation is the next isolated repair. All 226 workspace tests pass with six ignored
diagnostic instruments.

### Historical fixed 3x positive-ASL presentation — 30 July 2026

This section records the earlier experiment. It was superseded on 1 August 2026 by the current
fixed-2x presentation described above; its measurements are historical, not current.

Commit `60ab772` replaces the former 1x-near/4x-orbit altitude blend with one fixed **3x** transform
for positive baked macro height. It is deliberately a runtime presentation change, not a rebake:
the outmap, biomes, moisture, coastline, sea level, lakes, ocean waves, and negative bathymetry are
unchanged. The active ETOPO working-grid maximum is approximately 8,157m and can therefore present
at about 24.47km ASL before L4 resampling and bounded detail. Surface and low-flight positive land
remain three times their raw elevation.

The positive-only transform is shared by CPU clearance/probes, raster displacement and normals, ray
shell bounds and hit evaluation, and the conservative LOD/culling land bound. The original 3x
change moved the then-active landing scenarios outward by +2,022.55078125m. The later ETOPO rebake
re-authors them to its own raw landing ground of 855.17919921875m / 2,565.53759765625m presented.
Negative height is never multiplied, which avoids deepening the ocean floor or moving the
zero-metre coastline.

Current Quadro M1000M evidence, `PRESENT_MODE=immediate`, same release binary and settled spatial
samples:

| scenario/path | previous profile | fixed 3x | result |
|---|---:|---:|---|
| `orbit_once` raster mean | 18.291ms | 18.457ms | pass; +0.9%, normal run noise |
| `stand_on_ground` raster mean | 31.808ms | 34.157ms | pass; exact 2.000000001m clearance |
| `low_flight_performance` raster mean | 32.219ms | 31.714ms | historical budget/fallback/seam failure remains |
| `stand_on_ground` ray mean, ray-only after 1.5s | 64.432ms | 67.857ms | p90 failure remains; +5.3% |

Raster ground truth remains tight: 360 comparisons, 0.198m worst-frame p90 and 0.598m maximum. Ray
ground p90 moves from the immediately preceding Earth-like baseline's 2.567m to 3.199m against the
unchanged 2m tolerance; do not widen the tolerance. The low-flight run still binds all 256 chunks
for 241 frames, and the warm-up maximum seam moves from 549.982m to 612.707m. Its capture shows the
same pre-existing exposed mixed-LOD/ocean edge at the right, made taller by the stronger land
relief. Those are concrete costs of the experiment and still need repair/sign-off.

Raster orbit and ground pass, all nine raster/ray final/albedo/lighting/aerial/ray-hit parity stages
pass, and the other two shifted landing scenarios pass with 70.19m ground-detail clearance and
2.00–64.42m eye-level descent clearance. The release build succeeds, and all 206 workspace tests
pass. Visually, the low-flight landscape gains clearly legible hills and distant ranges; globally
dense L4 source remains only 3.906km per sample, so the scale also makes its existing facets and
cliffs more conspicuous rather than adding detail.

### ETOPO highest-prominence survey and F4 start — 1 August 2026

The current surface was surveyed using the standard Earth topographic-prominence definition:
**summit elevation minus the elevation of its key col**. The planet's global highest summit has no
higher parent summit, so, as for Everest, its reference key col is sea level and its prominence is
equal to its elevation ASL.

The reusable ignored `global_highest_summit` instrument scanned all 1,536 globally dense L4 height
tiles, selected all 18 cells capable of exceeding the current maximum after the conservative
491.5m runtime-detail allowance, and refined 16 to sub-metre spacing with the same CPU macro and
runtime-detail functions used for camera clearance. It found:

- highest raw L4 macro sample: **7,720.434m ASL**;
- highest current presented surface: **15,448.904m ASL/prominence**, near Everest at
  **27.989286°N, 86.943824°E**;
- at the refined summit the fixed-2x macro contributes 15,286.436m and runtime detail contributes
  162.468m.

F4 now enters free flight at that direction, preserving the established 500ft / 152.4m entry
clearance and existing downward pitch, but facing back across the clean snow-covered summit bowl.
Because the summit lies outside the sparse L5-L18 corridor, F4 synchronously loads its guaranteed
global L4 tile before computing the camera radius. Ordinary flight following remains
resident-cache-only. This one-time resolution prevents the camera from jumping by hundreds of
metres when streaming replaces a coarser ancestor after entry.

The deterministic `highest_prominence_peak` scenario is re-authored to the measured pose and checks
150–155m clearance in both paths. Raster `1785599150-128156` and ray
`1785599507-134390` pass with finite metrics, zero LOD thrash, two captures and 152.374m / 152.400m
clearance respectively. The raster capture shows the lower-amplitude snow-covered range. Ray mode retains its
known fixed-L4 spatial limitation, but its terrain ownership and clearance remain correct.

Fixed world-space manual captures and terrain-performance baselines below describe the previous
macro planet and are no longer visual golden locations. Manifest-relative scenarios remain the
appropriate first smoke tests; re-author any hard-coded terrain location before using it as evidence
about this new planet.

Scenario probe results, worst frame, from `test-runs/*/*/manifest.json`. The first two rows are
historical fixed-3x pre-ETOPO evidence; the other rows are retained as pre-rebake renderer history and
must be rerun before they are quoted as current terrain evidence:

| scenario | path | p90 delta | median delta | tolerance | clearance |
|---|---|---:|---:|---:|---:|
| `stand_on_ground` | raster | **0.20 m** | 0.09 m | 2 m | 2.0000 m |
| `stand_on_ground` | ray | **3.20 m** | 2.22 m | 2 m — **FAILS** | 2.0000 m |
| `path_parity_ridge` | raster | **4.24 m** | 1.25 m | 6 m | 133 m |
| `path_parity_ridge` | ray | **10.68 m** | 2.17 m | 6 m — **FAILS, see §6b** | 133 m |
| `tour_mountains` | ray | 22.6 m | 11.7 m | none | ~1 km |

Except for the refreshed `stand_on_ground` rows, these are the pre-rebake post-mountain results. The
3.47 m ray result recorded later in §7 is the historical pre-mountain measurement from the
local-span hit-walk change; the increased mountain relief subsequently moved that previous outmap's
result to 10.68 m.

These moved with the mountain work in §6b: the terrain now has three times the relief, so the same
mesh disagrees with truth by more in absolute metres. Raster still holds well inside tolerance
everywhere; the ray path does not, on the ridge.

The camera stands exactly 2.0 m above the ground it is drawn on, in both paths. That was the point
of the whole exercise.

**Do not read `max_abs_delta_meters` as a surface measurement.** At 2 m eye height the horizon is
4 km and the probe compares out to 4 km; a ray arriving at a fraction of a degree turns a metre of
ground into hundreds of metres of reconstructed height. `stand_on_ground` ray shows max 194 m from
exactly two grazing points out of 77. p90 is the assertion that means something.

### Exact manual W-flight replay and raster collision repair — 30 July 2026

The reported failure is preserved as `manual_forward_clearance`: it starts at the exact position
`[963666.5873397837, 2669549.1557218134, 2856170.063578058]`, looks along
`[-0.25419213683927866, -0.7168602258826009, 0.6492285992750381]`, freezes the planet, waits three
seconds for residency, then holds the real W-flight input and captures seven frames through 4.5s.
Scenario runs ignore live `DeviceEvent::MouseMotion`, so unrelated desktop mouse movement cannot
change this camera again.

The exact pre-fix replay `1785428386-445023` reproduced the camera passing beneath the visible
terrain/transition layers. Its endpoint-only collision followed one finest cached point sample:
that is not conservative when signed runtime relief is filtered differently across simultaneously
drawn parent/child patches, and it can also tunnel past a higher surface between frame endpoints.
CPU clearance reported only 3.802–15.619m while the worst depth-probe p90 disagreement reached
25.828m.

Raster collision now records every drawn active, incoming, and outgoing node with its actual source
tile, edge stitch, source fade, and distance filter; queries take the highest candidate. Each moving
frame is swept at 0.5m spacing, bounded to 64 samples. Movement now uses a 5m camera-sized
envelope so close inspection is below 30m; the CPU clearance floor is 0.75m and F4 enters at 2m.
Ray flight keeps its existing ray-surface truth.

The committed-HEAD final Quadro replay `1785437799-519300` passes finite metrics, zero LOD thrash,
245 compared points, 13.734–43.664m CPU clearance, and all seven captures. The dangerous early
W-held frames have nearest visible hits at least 23.788m away, and visual inspection shows the
camera above the snow basin rather than beneath stacked patch faces. The depth probe still reports
up to 21.395m p90 at the moving mixed-LOD transition: it intersects raster triangle/skirt geometry
whereas the point evaluator samples the analytic radial surface, so it is retained as a diagnostic
rather than misrepresented as a 2m collision assertion. The collision envelope prevents that
residual representation difference from admitting the camera into visible geometry.

Current regressions: raster `stand_on_ground/1785435672-497949` passes at exactly 2m clearance and
0.198m worst p90; the pre-existing ray p90 failure is unchanged at 3.199m in
`stand_on_ground/1785435686-498076`; both raster and ray highest-prominence scenarios pass in
`1785435713-498256` and `1785435731-498377`. Release build, formatting, diff checks, and all 215
workspace tests pass.

#### High-speed destination-frontier follow-up

The user's later manual run `manual/1785438019-521226` held W for long enough to accelerate from
about 6km/s to 5,776km/s while taking 24 half-second captures. Captures 18–19 expose the underside
of the terrain. This was not culling or near clipping: their rendered-surface probes measured
**-36.771m** and then **-159.433m camera clearance**, despite 255–256 chunks remaining drawn and
CPU/render detail correlation staying above 0.998.

The first collision repair still queried the previous frame's `surface_detail_nodes`. Slow movement
remained inside that frontier and passed, but a high-speed destination could lie completely outside
it. The renderer then selected the destination LOD frontier later in the same frame, after the
camera correction, so the visual ground could rise around an already placed camera.

Raster low flight now performs a second point-clearance check immediately after destination terrain
selection. If the new frontier lifts the camera, the terrain update runs once more to rebuild the
camera-relative chunk anchors against the corrected pose; frames needing no correction retain the
single update. The existing 30m moving envelope and 2m idle height are unchanged. This ordering is
important: correcting the camera without rebuilding camera-relative anchors would move the camera
and geometry in different coordinate bases.

`manual_high_speed_clearance` preserves the recorded summit pose and real acceleration path, then
probes 21 captures every 0.5s from 5s through 15s of held W, reaching the finite 8,000km/s speed cap.
The committed Quadro run `1785439098-533024` passes all 21 frames with minimum rendered clearance
**29.9999999995m**, 724 compared surface points, and no visible terrain undersides. It records a
38.256ms spatial mean and 44.379ms p90 during the deliberately extreme planet-spanning flight.
Ordinary views show no measurable cost: the short W replay is 36.760ms versus 37.131ms before this
follow-up, the nine-heading culling sweep is 34.703ms versus 34.506ms, and the static summit is
32.702ms versus 32.696ms. Those sub-millisecond movements are run noise, not a performance claim.
Formatting, release build, and all 216 workspace tests pass.

### Near-terrain culling and fixed-exposure inspection — 30 July 2026

Manual run `manual/1785436299-502648` captured nine headings from the F4 summit camera. Eight showed
jagged cleared-background polygons across the foreground: the closest terrain chunks were selected
but their real displaced vertices lay outside the radial shell used by frustum and horizon culling.
This was not near clipping. At the time, the shell stopped at **26,538m** — 8,846m maximum baked macro height
times the fixed 3x presentation scale — and omitted the live detail ladder's independently declared
**2,646.4m** positive bound. The actual shader can therefore reach 29,184.4m while the culler was
proving visibility against the lower macro-only surface.

`manual_near_terrain_culling` preserves the peak position, 60-degree FOV, and nine headings spanning
180 degrees. With the old bound, run `1785437126-510328` reproduces the missing foreground in eight
captures; near-black pixels occupy up to 38.63% of the bottom 35% of a frame. The culling height
range now adds `TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS` to its maximum. Fixed run
`1785437649-517600` has zero near-black foreground pixels in all nine captures, zero LOD thrash,
152.514m clearance, and the same 254–256 active-chunk range. Its settled mean is 34.946ms versus
36.952ms in the single-variable old-bound baseline, so the conservative radial correction did not
add a measured cost in this replay. A unit regression requires the culling shell to contain both
macro and live ladder displacement.

F8 has always meant **ACES/HDR display curve off**, not auto exposure off; `hdr.wgsl` deliberately
multiplies the linear colour by the adapted exposure in both F8 states. That is why the snow still
darkened the screen in the user's F8-off inspection, and the manual log confirms exposure ranging
from 0.294 to 3.858. Key **6** now independently toggles the presented exposure between auto and a
literal fixed **1.0**. The meter keeps adapting behind the fixed view so returning to auto does not
snap, but it has no effect on presented pixels while fixed; the HUD shows applied exposure, meter,
mode, and HDR-curve state separately. In `highest_prominence_peak/1785437750-518841`, a synthetic
key-6 press changes logged applied exposure from 0.335 to exactly 1.0 while the hidden meter remains
0.335. For a raw inspection with neither exposure adaptation nor ACES, use **F8 off + 6 fixed**;
F6/F7 still control blur and bloom independently.

### Sunset colour crossover and blue hour — 30 July 2026

Yes: after the direct red sunset, a clear real sky normally passes through a dim purple/deep-blue
blue hour before astronomical darkness. The renderer instead went red to dark red and then black,
and during the earlier yellow-to-blue transition it produced a broad green interval.

This was visible with exposure fixed at exactly 1.0 in the user's
`manual/1785438019-521226`, so exposure adaptation was not the cause. Measurements across its upper
sky found green-dominant pixels in 68-100% of the sampled region in several consecutive frames,
with green exceeding both red and blue by up to 22 display values. The direct fullscreen
single-scattering source in `atmosphere.wgsl` was multiplied by solar visibility at every sample:
after the planet shadow covered those samples it had no indirect source left, so it could only
produce red -> black. The existing 1.3x sky-only saturation then made the direct model's narrow
green crossover more conspicuous.

`sunset_blue_hour` preserves one fixed ground camera and sweeps the sun through +15, +2, -4, -9,
-14, and -20 degrees with presented exposure fixed at 1.0. Before the repair, run
`1785443166-565178` sampled RGB `(182,142,0)`, `(162,62,0)`, `(142,41,0)`, `(98,14,0)`,
`(26,1,0)`, `(4,0,0)`: red simply decayed toward black.

The fullscreen sky now:

- caps only a green-dominant result at the larger red/blue channel after sky saturation, preventing
  the simplified model's unphysical green interval while preserving its yellow, red, cyan, and
  blue results;
- adds a bounded analytic Rayleigh view column for the omitted indirect/multiple-scattered blue
  twilight, rising from about 6 degrees solar depression, peaking near 10 degrees, and fading from
  about 16 to 21 degrees;
- adds no raymarch samples and changes neither terrain/ocean lighting, terrain aerial perspective,
  the sun overlay, nor deep night.

Committed Quadro run `sunset_blue_hour/1785444485-580099` passes all new image assertions. Its same
fixed-exposure samples are `(111,74,0)`, `(69,17,0)`, `(55,9,0)`, `(43,36,52)`, `(22,39,63)`,
and `(2,4,7)`: red -> dim purple -> blue -> fading deep blue. Maximum sampled green dominance is
zero, peak blue/red is 2.864 at luminance 0.146, and final/peak luminance is 0.102 while remaining
blue-dominant. The controlled pre/post release runs measured 27.121ms and 27.105ms spatial means;
that 0.016ms change is noise, so the no-extra-sample approximation is timing-neutral.

Committed regressions `twilight_directionality/1785444539-581134` and
`night_side_atmosphere/1785444546-581133` pass at 1.851x solar/anti-solar luminance and zero sampled
night-sky luminance respectively. The earlier `sunset_sweep` sky-colour assertions also pass; its
overall run retains only the known, unrelated §3 fallback-count failure. Release build, formatting,
diff checks, and all 218 workspace tests pass.

### Near-surface sunrise, midday, and sunset comparison — 5 August 2026

Earth references used for this pass agree on four useful visual cues: the long low-sun path removes
blue and leaves orange/red near the solar direction, the horizon is paler than overhead blue, clear
blue hour is a short purple-to-deep-blue interval after the red sunset, and aerosol/haze can add a
soft halo around the bright disk. See NASA's [Earth Observatory scattering notes](https://eol.jsc.nasa.gov/Collections/EarthObservatory/articles/CrepuscularRaysandLightScattering.htm),
[NASA's blue-sky explanation](https://spaceplace.nasa.gov/blue-sky/en/), NOAA's [horizon and sunset
colour guide](https://www.weather.gov/fgz/SkyBlue), and the real-Earth [Red sky during sunset
photograph](https://commons.wikimedia.org/wiki/File%3ARed_sky_during_sunset.jpg). These are
comparison references, not pixel targets: the renderer has no clouds, aerosol maps, or camera lens
model, so the last cue is intentionally a bounded presentation effect.

The new deterministic `sunrise_midday_surface` scenario holds a 100m camera near the equator,
tracks the sun-facing pose, fixes exposure at 1.0, and captures at 5° sunrise, 45° midday, 5°
sunset, and -12° blue hour. Full-budget software-GL run
`sunrise_midday_surface/1785946181-791182` captured all four frames and passed its manifest. The
captures show the expected warm sunrise/sunset, neutral bright midday, and a blue post-sunset frame;
the horizon remains intentionally simple because the scene has no cloud/aerosol layer. The earlier
16-chunk diagnostic run is retained as `1785936513-728671`.

The bounded implementation changes are:

- `atmosphere.wgsl` reduces the sky-only saturation pass from 1.30 to 1.18 and keeps the blue-hour
  curve in the photographic range: it starts near -4°, reaches full weight near -7°, fades near
  -16°, and remains available through about -21° so astronomical darkness is not abrupt.
- `sun.wgsl` restores the physical 0.53° apparent solar diameter, adds a compact 6.5-radius HDR
  corona plus a 2.5-radius inner glare, and keeps the clipped core near white while tinting the
  corona toward bounded yellow/orange at low elevation. Terrain/ocean solar lighting is unchanged.

The new sun shader parses and validates in a focused regression; `cargo test --workspace` passes
with the new scenario. A Quadro/manual capture remains useful for final visual sign-off, but the
normal-budget software-GL replay confirms the tint/glare change without the earlier chunk cap.

### Sunrise intensity and terrain fill — 5 August 2026

The first comparison still had a real ordering defect: in fixed-exposure `sunset_blue_hour`, the
reverse sunrise luminance was `0.011, 0.125, 0.154, 0.061, 0.091, 0.263`, so blue hour was brighter
than the first strong red band and the sequence dipped twice. The cause was the indirect blue-hour
term outliving the weak direct red transition. A no-extra-sample red bridge now rises from about -8°
solar elevation through the horizon and fades by roughly +5°. The replay now measures
`0.011, 0.125, 0.154, 0.170, 0.193, 0.263` in reverse sunrise order: black -> blue -> strong red ->
daylight with no dip. Direct terrain/ocean solar transmittance remains unchanged.

The same report's dark terrain was partly an ambient/normal problem. `SKY_DIFFUSE_LIGHT_SCALE` is
now **0.40** (from 0.18), and the flat-L6 path explicitly flips its derivative normal outward when
it disagrees with the planet radial direction before evaluating sky fill, direct sunlight, or
specular. This makes visible-sun terrain receive light instead of treating a camera-facing inward
facet as the night side. The remaining large black regions in the flat-L6 captures are source/LOD
holes visible even in raw albedo and are not hidden by increasing ambient.

### Orbital red-limb repair — 5 August 2026

The latest manual Quadro captures `manual/1785952608-832998` showed a saturated red annulus around
the planet at roughly 8,000km altitude. The new red-twilight bridge was being added uniformly to
every atmosphere-shell pixel, bypassing the shell density taper; this made the full 720km shell
read as a flat red circle instead of a thin sunset limb. `atmosphere.wgsl` now weights that bridge by
the Rayleigh density at each ray's lowest point. Near-surface sunrise is unchanged, while orbital
views retain only a narrow, density-shaped limb. The replayed `limb_atmosphere/1785953439-840058`
shows the thin blue/orange edge without the red annulus, and the fixed-exposure
`sunset_blue_hour/1785953412-839804` still passes its ordered colour/luminance assertions.
The shader-focused regression and all 194 app plus 29 baker, 6 bake, and 6 core workspace tests
pass (6 ignored).

### Altitude-aware blue-hour timing — 5 August 2026

The next manual capture showed a dark-blue upper sky arriving while the solar disk was still
visible at about 52km altitude. The blue-hour approximation was keyed to radial solar depression;
at that altitude the geometric horizon is already about 9 degrees below the radial horizontal. The
fullscreen atmosphere now computes the tangent-horizon solar cosine from the camera radius and
starts the blue-hour curve only after the sun is below that actual horizon. Near-surface timing is
unchanged, while high-flight sunset no longer advances the blue transition. The fixed-exposure
`sunset_blue_hour/1785959648-882409` replay still passes all colour/luminance assertions, and the
full workspace suite remains green.

### Interactive presentation defaults — 5 August 2026

The normal interactive launch now starts in borderless fullscreen while keeping the existing
internal render size; deterministic `--scenario` runs remain windowed so their capture dimensions
and timing stay stable. The HUD/egui panel starts hidden and remains available with F3. The HDR
curve and auto-exposure are both disabled at startup (F8 and 6 still toggle them independently),
while the HDR scene target and luminance meter remain available for diagnostics. The default-state
regression covers the interactive/fullscreen split and the presentation constants; the full
workspace suite passes.

Interactive launches now render one windowed surface frame before entering borderless fullscreen.
This avoids an X11/wgpu first-present race observed on the Quadro M1000M where the compositor could
leave the fullscreen window showing the desktop until F toggled fullscreen again. Deterministic
scenarios remain windowed; the transition is logged as `deferring fullscreen until the first surface
frame` followed by `entered fullscreen after the first surface frame`.

## 3. State: what was red before the Earth-like rebake

The six results below are pre-rebake evidence and now require a fresh matrix. Before the Earth-like
outmap replacement, **all six were failing before this branch and were unchanged by it** — each was
verified by building the older commit and diffing the number. Do not present the table as current,
and do not re-investigate an identical failure as a regression from the generator without first
comparing it with that historical evidence.

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

`p90_abs_delta_m` is also emitted on the `"surface probe"` tracing line.

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

# Deterministic paired raster/ray composition and hit-status matrix
CARGO_TARGET_DIR=/home/dad/catingard-target scripts/run-render-path-parity.sh
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
- **`CATINGARDEN_DEBUG_MODE=albedo|lighting|aerial|sky|ray_hit`** selects a render debug mode for a
  scenario. `albedo` is how you tell a material problem from a lighting one; `ray_hit` is an
  env-only ray diagnostic where green is a bracketed detail hit, red is macro fallback, yellow is
  no local relief, blue is ocean, and black is no hit.
- Other flags: `--terrain placeholder|outmap`, `--outmap <path>`, `--vertical-fov-degrees`,
  `CATINGARDEN_RAY_EXPERIMENTS`, `WGPU_ADAPTER_NAME`.
- **`CATINGARDEN_MAX_ACTIVE_CHUNKS` lifts the chunk budget** (selector and instance buffer together)
  so a run can show what the selector actually wants rather than what the cap allows. `budget_limited`
  going to 0 is how you know demand is satisfied and the number is real. Do not read a demand
  reduction as a frame-time saving without checking this: at the default 256 the cap binds on every
  frame of every scenario, and a change that cuts demand by a third can leave the frame untouched.

### Manual raster fault repair — complete, final human sign-off still requested

The newest manual set `test-runs/manual/1785253016-2703201` contained two views of one serious
raster fault:

| capture | rotating-world pose / direction | frozen local pose / direction | clearance | fault |
|---|---|---|---:|---|
| 001 | `[-1423884, 313839, 3728369]` / `[0.701, -0.671, 0.241]` | `[-3960082.052, 313838.931, -495914.630]` / `[-0.067302, -0.671093, 0.738312]` | 468 m | long black channels through land |
| 002 | `[-1422920, 314984, 3728210]` / `[0.286, 0.897, -0.337]` | `[-3959697.937, 314983.998, -495015.947]` / `[0.395375, 0.897012, 0.197608]` | 106 m | vertical and overhead terrain sheets; 173.8 m probe outlier |

`manual_render_faults` freezes planet rotation, replays those two local poses, waits for streaming
and transitions to settle, and captures at 3.0 and 7.5 seconds:

```bash
CARGO_TARGET_DIR=/home/dad/catingard-target cargo build --release -p catinthegarden-app
CATINGARDEN_PRESENT_MODE=immediate \
  /home/dad/catingard-target/release/catinthegarden-app --scenario manual_render_faults
```

The fault was **not** horizon/frustum culling, edge stitching, a lake predicate, or an LOD
transition. Disabling skirts removed the walls but left the channels; disabling the runtime detail
ladder removed both. The baker constrains every sparse child border to its parent, but the amplified
runtime ladder chose its high-frequency cutoff from the resolved source level as a hard integer.
At a child/fallback boundary the baked heights therefore met while the added displacement did not;
the normal 10 m skirt then made that discontinuity look like a wall.

Raster and CPU terrain truth now derive a continuous effective source level. Each parent-complete
sparse level fades from its parent over two of that level's 128 source texels, so the exact border
evaluates the same displacement from either side and the original interior field is unchanged.
The manifest's dense level is uploaded rather than hard-coded. A packed per-instance bit keeps the
extra WGSL hierarchy walk off chunks that do not intersect a source-edge fade.

Final deterministic run `1785256951-2743312` removes the black openings and floating sheets. It
measures p90 **2.956 m**, max **8.265 m**, and seam delta **0.000244 m**, against the broken replay's
94.425 m deterministic maximum (173.8 m in the rounded manual pose). The established 10 m skirt cap
is unchanged, and 197 workspace tests pass with five diagnostic tests ignored. This replay is a
correctness regression, not a new performance baseline; its
106 m-clearance pose remains near the 33 ms Quadro budget and should be rechecked in the next
exclusive benchmark set. Ian has not yet signed off the repaired captures.

### Manual mountain mixed-LOD repair — deterministic replay clean, human sign-off requested

Manual set `test-runs/manual/1785265652-40830` exposed a second, distinct raster fault in three
mountain views. These are preserved by `mountain_render_faults`; the manual world pose is transformed
into the scenario's frozen planet frame, and the reference FOV is adjusted to reproduce the exact
physical-window FOV:

| capture | latitude / longitude | manual world pose / direction | clearance | vertical FOV |
|---|---|---|---:|---:|
| 001 | `31.595415 N, 18.344120 W` | `[3237706.065, 2098111.805, -1073535.448]` / `[-0.193, -0.519, -0.833]` | 224.56 m | 34.290° |
| 002 | `31.551518 N, 18.540050 W` | `[3235111.597, 2095220.959, -1084968.342]` / `[0.483, 0.055, -0.874]` | 9.97 m | 17.201° |
| 003 | `31.688138 N, 18.702487 W` | `[3227657.898, 2103596.673, -1092658.086]` / `[-0.505, -0.312, -0.805]` | 772.68 m | 52.675° |

The symptoms were black voids, long horizontal terrain sheets, false terrain overhead in capture
002, and giant fan polygons in capture 003. All three frames were at the 256-leaf cap and reported a
zero baked seam delta, which hid the actual failure. Raising the cap alone to 1024 still left the
holes and cost roughly 95–98 ms per frame. Removing only the mesh-level component of the runtime
detail filter removed the holes, proving that adjacent grids were evaluating the amplified
mountain-scale displacement with incompatible cutoffs rather than omitting a draw.

There were two coupled faults. The selector filled all 256 leaves with primary screen-error demand
before attempting its two-level balancing pass, so balancing had no budget left. The shader then
filtered runtime displacement from each node's own vertex spacing; a fine edge beside that much
coarser node therefore did not occupy the same height even though their baked samples agreed.

The selector now trials each requested split together with its recursively required coarse-neighbour
splits, admitting the group only when the complete balanced frontier fits the existing 256-leaf
limit. It checks only the newly touched boundary, avoiding the all-pairs cost of a full balance scan
per candidate. The packed edge metadata retains the actual neighbour-level delta, while coordinate
collapse remains capped at two levels; the fine displacement filter fades to the neighbour's filter
over the representable coarse footprint. This preserves the anti-alias filter in chunk interiors
without reopening a height wall at the edge.

Final committed-HEAD raster run `1785271707-90557` passes all assertions and visually removes the
voids, sheets, false overhead terrain, and giant fans. Capture 002 is correctly sky-only in its
strongly outward-looking orientation. Captures 001/003 measure p90 **9.509/9.349 m** and maxima
**12.918/17.793 m**; their capture frames were **37.77/34.27 ms**, with capture 002 at **29.03 ms**.
The exact-pose balanced-frontier regression, packed-edge/WGSL validation, and all **198** workspace
tests pass. A preceding identical-source run measured capture 001 at 32.84 ms, so the first view is
near the budget but variable; capture 003 remains about 1 ms over it. This is a correctness repair,
not a claim that the mountain performance work is finished.

### Raster all-land ocean submission culling

The independent raster ocean shell used the terrain's complete instance list. Its fragment shader
correctly discarded raised land, but all 256 chunks still ran the six-wave ocean vertex path and
submitted another 589,824 triangles in each mountain view.

`GpuTile` now precomputes whether its complete logical R32F footprint is strictly positive. Fallback
sub-rectangles test only the texels which can contribute to their bilinear samples. An ocean
instance is omitted only when every contributing height is finite and above zero; a zero, negative,
invalid, placeholder, coastline, or otherwise uncertain footprint keeps the former shader-owned
path. Possible-ocean instances are sorted to the front of each resolved-tile group, so the ocean
draw batches reference contiguous subsets of the existing terrain instance buffer rather than
duplicating uploads. Spatial logs and the HUD expose `ocean_chunks` and `ocean_triangles`.

Two immediate-mode runs on each side, using the exact three `mountain_render_faults` poses and
discarding the first two spatial samples after each cut:

| view | before settled mean | after settled mean | change | ocean chunks / triangles after |
|---|---:|---:|---:|---:|
| 001 | 33.567 ms | **32.633 ms** | −0.934 ms / −2.8% | 1 / 2,304 |
| 002 | 31.173 ms | **28.541 ms** | −2.632 ms / −8.4% | 0 / 0 |
| 003 | 33.724 ms | **32.603 ms** | −1.121 ms / −3.3% | 1 / 2,304 |
| equal-view mean | 32.821 ms | **31.259 ms** | **−1.562 ms / −4.8%** | |

Before runs were `1785271707-90557` and `1785272791-99965`; after runs were
`1785273124-104321` and `1785273186-104898`. Terrain stays at 256 chunks/589,824 triangles, while
ocean submission falls from the former 256 chunks/589,824 triangles to the table above. Baseline
and culled captures differ by at most one 8-bit value, confined to normal exposure/frame-timing
drift; visual inspection finds no structural change. Raster `ocean_flyover`
`1785273243-105393` conservatively retains all 256 ocean chunks and visible waves, and its wave
range passes. The scenario as a whole still fails only its pre-existing fallback limit
(256 observed vs 192 allowed), as its preceding runs did.

Across all three post-change runs, the per-view means are **32.703/28.661/32.844 ms** and their
equal-view mean is **31.403 ms**, an aggregate **1.418 ms / 4.3%** below the two-run baseline.
Final committed-HEAD run `1785273384-106648` passes at **32.844/28.902/33.328 ms** with the same
1/0/1 ocean chunks. Its captures remain within one 8-bit value of the pre-change frames with no
structural difference.

The focused bilinear-footprint regression and all **199** workspace tests pass with five diagnostic
tests ignored.

---

## 6. Next, in order

**Current next session: continue §6d with the mixed-source ray window, then first-visible-crossing
hit refinement.** The numbered material, LOD and frame-budget entries below retain the measurements
and decisions that produced the current renderer; they are not a newer priority list than §6d.

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

### 2c. The mountains are flat because there is no aerial perspective, not because of materials

Following §2b's finding to the lighting, with `CATINGARDEN_DEBUG_MODE` at the `tour_mountains`
mid-tour frame, sampled in three distance bands so near ground cannot mask the horizon:

| band | albedo hue | lit hue | aerial contribution (lum) | `final` vs `lighting` |
|---|---:|---:|---:|---|
| far (horizon) | 199° | 41° | **0.015** | 0.559 vs 0.558 |
| mid | 199° | 47° | **0.000** | identical |
| near | 198° | 48° | 0.004 | identical |

**Two things, and the second is the actionable one.**

1. The warm tan is the *surface lighting* term, not the atmosphere. Albedo sits at hue 198–199° —
   pale blue-white — at every distance, and the lit result is 41–48°. The lighting swings hue by
   about 150°. §6.2's instinct to reach for albedo would not have touched this.
2. **Aerial perspective contributes essentially nothing at any distance.** `RENDER_DEBUG_AERIAL_
   CONTRIBUTION` returns `max(aerial − lighting, 0)`, so it shows only in-scatter; but `final`
   equals `lighting` to three decimals in every band, which rules out extinction as well. Ground
   luminance is flat with range — far 0.502, mid 0.565, near 0.533 — where a real range at tens of
   km lifts toward the sky and desaturates with distance.

**Found, fixed, and it revealed the real problem rather than solving it.**

The vertex stage computes `aerial_color = lit x transmittance + in_scatter`. The fragment stage was
reconstructing that onto the re-textured albedo as an *additive residue*:

```wgsl
textured_aerial_color = textured_surface_lighting
    + max(input.aerial_color - input.surface_lighting, vec3<f32>(0.0));
```

That difference is `in_scatter - lit x (1 - T)`. **The extinction is the negative half, so it was
discarded on the floor, and the clamp then zeroed the entire term wherever extinction exceeded
in-scatter** — which over bright ground is everywhere. The first repair carried that transform as a
ratio, with the old additive form selected where the vertex surface was too dark to define one. It
restored extinction but left a second defect at that near-black switch.

**This defect was raster-only.** The ray path calls `aerial_perspective` per pixel and never had the
reconstruction step, so the fix is also a parity fix.

**Follow-up, 1 August: the ratio repair caused the outlined low-sun shadows.** The reported manual
capture put adjacent RGB channels across a hard per-channel `surface_lighting > 0.001` selection.
The ratio side could amplify a dark shadow by up to 16x while the fallback side stayed dark, making
the shadow interior lighter than its outline. `outlined_shadows` preserves the reported camera,
orientation, 37.22177174009007° rendered FOV, and 18.303833° local solar elevation. Its F9 captures
showed byte-identical raw albedo and surface-lighting before/after; the fault first appeared in the
aerial stage, ruling out geometry, the correctly 3x-steepened height normals, and material lighting.

Raster vertices now pass the actual affine components separately and fragments apply the exact
continuous transform:

```wgsl
textured_aerial_color = textured_surface_lighting * aerial_transmittance + aerial_in_scatter;
```

Distance fog is composed into those same components without changing its result. There is no ratio,
near-black threshold, or amplification clamp. The local-sky ambient path is deliberately unchanged:
`sky_diffuse_irradiance` samples above the terrain normal and applies
`SKY_DIFFUSE_LIGHT_SCALE = 0.18`; material absorption can still shift the reflected hue.

Controlled same-pose captures are baseline `1785539126-17480` through `1785539154-17722` and fixed
`1785539594-22226` through `1785539627-22541`; a simple dark-red-outline count in the final terrain
falls 15,960 to 5,288 (66.9%). The same short raster replay improved 29.542ms to 24.829ms mean because
the now-unused duplicate vertex material evaluation was removed; treat the single ten-sample timing
as directional, not a general renderer benchmark. Committed exact-FOV raster run
`outlined_shadows/1785585729-12608` passes, as do the ray replay, `sunset_blue_hour`,
`twilight_directionality`, `night_side_atmosphere`, and all 220 workspace tests.

Measured after, `tour_mountains` mid-tour frame:

| far-band | before | after | ray |
|---|---:|---:|---:|
| luminance | 0.502 (= lighting exactly) | **0.434** | 0.400 |
| saturation | 0.212 | **0.323** | 0.342 |

**But the horizon now goes darker and more saturated, where real haze goes lighter and less.** That
is extinction arriving correctly with nothing to balance it, and the arithmetic says both terms are
being computed right: measured blue ratio 0.710 against `exp(-33.1e-6 x 0.88 x 30km)` = 0.417 of
extinction plus ~0.28 of in-scatter. Over 30 km at 4.7 km altitude this model removes more from a
bright surface than it adds back.

**Then the instrument was built, and it overturned the paragraph above.** `haze.rs` bins the drawn
surface by distance and scores how far it has travelled toward the sky *measured just above the
silhouette at the same azimuth*. Reported per screenshot frame into `manifest.json` under
`haze_probes`. Measured at `tour_mountains`:

| band | raster t=4 rgb | distance to sky |
|---|---|---:|
| 2–5 km | [0.613 0.559 0.340] | 0.635 |
| 12–30 km | [0.542 0.463 0.264] | 0.493 |
| 30–80 km | [0.210 0.167 0.062] | **0.081** |

**Convergence 0.79–0.81 raster, 0.83–0.87 ray.** Terrain does approach the sky, in both paths, and
the earlier "aerial contributes nothing" reading was against the wrong reference — it used the
`SkyOnly` debug pass averaged over the ground region, which is not the sky the terrain is seen
against. The instrument exists precisely so that reference cannot be picked by hand again.

**What that leaves is the sky itself.** The horizon sky it converges *to* measures
`[0.310 0.185 0.000]` — a dark, fully desaturated-of-blue orange-brown, at a sun elevation around
45°. A daytime horizon should be pale and blue-white. So the terrain is behaving; the thing it is
converging onto is wrong, which is why distance reads as dimming rather than as haze. **Look at the
sky model at low elevation angles before touching `AERIAL_IN_SCATTER_GAIN`** — the gain would only
push terrain harder toward a colour that is itself the defect.

### 2d. The sky's scattering did not saturate, and a saturation boost clips what is left

Two stacked faults, found by following §2c's sky reading.

**One, fixed.** `sky_radiance` returned `view_transmittance * scattering_coefficient *
path_length` — a term growing linearly in path multiplied by one decaying exponentially in it, so
the product peaks and then collapses back toward zero. The channel with the largest coefficient
enters that collapse first, which is blue, so the horizon lost precisely the wavelength that should
dominate it. It now uses `1 - exp(-optical_depth)`, which is **the form `aerial_perspective` has
been using all along**: these are one model and disagreed about it. Measured effect on terrain
haze at `tour_mountains`, 80–200 km band: saturation **0.461 → 0.194**, i.e. distance now
desaturates as it should. 159 tests pass; `night_side_atmosphere` and `polar_ice_cap` — the two most
sensitive to sky brightness — both still pass, and `sunset_sweep` / `ground_to_orbit` fail only their
pre-existing §3 assertions.

**Two, not fixed, because it is an authored knob and not ours to set.** The visible sky comes through
`atmosphere.wgsl`, whose `saturate_sky_color` pushes colour away from its own luminance by
`SKY_ATMOSPHERE_SATURATION = 2.0` and clamps at zero. On an already-warm horizon that drives blue
**negative, and it clips to exactly 0.000** — which is the literal value the haze probe reads for the
horizon sky, before and after the fix above. AGENTS.md records that 2× as a deliberate visual choice.
It was measurably destroying the blue end at low elevation. **Swept, and set to 1.3.**

| `SKY_ATMOSPHERE_SATURATION` | horizon sky RGB | blue/red | `sunset_red_over_blue_grows` |
|---:|---|---:|---|
| 2.0 (was) | [0.259 0.155 **0.000**] | 0.00 | passes |
| 1.6 | — | — | passes |
| **1.3 (now)** | [0.229 0.159 0.028] | **0.12** | **passes** |
| 1.0 | [0.215 0.161 0.065] | 0.30 | **fails**: required 1.100, observed 1.000 |

1.0 gives the best horizon and costs the sunset outright — the boost is what makes red and blue
diverge as the sun goes down, which is the job it was added for. 1.3 is the most that can come off
while that still holds. Ray reads blue/red 0.35 at the same setting against raster's 0.12, which is
a parity gap in the sky worth its own look.

**Read the convergence numbers here with care.** They rise 0.704 → 0.758 → 0.810 across the sweep,
but the far terrain band is *identical* at all three (saturation 0.219, luminance 0.211). The score
moved because the sky reference moved toward the terrain, not because the terrain hazed better. The
metric is symmetric by construction — it asks whether the two agree — so it must not be read as
"haze improved" without checking which end moved.

The haze probe is the way to judge any change to it: convergence and the far-band saturation are
both in `manifest.json` now, so the question is a number rather than an argument.

*(Superseded reasoning, kept because the measurement below is still valid on its own terms:* In-scatter lands around 0.10 against a surface at 0.5; a distant range reads pale because
in-scatter dominates. `AERIAL_IN_SCATTER_GAIN` is 3.0 and §8 records that it affects neither
extinction nor the sky, which makes it the isolated lever — but it is a tuning knob on a physical
model, so raising it is a deliberate choice about realism versus appearance and wants Ian's eye, not
a unilateral number. `AERIAL_IN_SCATTER_SAMPLE_COUNT` is also only 2.*)

*Caveat on method: the first pass at this sampled only the bottom 65% of the frame, which is all
near ground where little haze is correct, and would have supported the same conclusion for the wrong
reason. The bands above are the measurement that means something.*

### 2e. Colours and textures: the fine scale is missing, the coarse scale is not

First measurement of the material system across the biomes §6.2 said to check and nobody had.
Rendered with `CATINGARDEN_DEBUG_MODE=albedo` so lighting is excluded; `fine` is mean
adjacent-pixel luminance difference, `coarse sd` is the spread between 32x32 block means.

| scene (albedo) | sat sd | hue sd | fine | coarse sd |
|---|---:|---:|---:|---:|
| `tour_grassland` | 0.054 | 27.0 | **0.0001** | 0.0335 |
| `tour_mountains` | 0.022 | 34.2 | 0.0023 | 0.0812 |
| `tour_coast` | 0.193 | 60.1 | 0.0011 | 0.0585 |
| `tour_tundra` | 0.273 | 38.2 | 0.0025 | 0.1132 |
| `terrain_material_preview` | 0.019 | 1.1 | 0.0005 | 0.0187 |

**Coarse variation exists; fine variation is essentially absent everywhere** — 0.0001 to 0.0025,
against 0.0014–0.0051 in the same frames *with* lighting. So nearly all the metre-scale texture in
the rendered image is shading, not albedo. Grassland is the extreme: 2,549 distinct ground colours
in the frame, and adjacent pixels differing by 0.0001.

**The leading suspect is documented behaviour rather than a defect** — and it is unverified, so
verify it before building on it. `TERRAIN_MATERIAL_DETAIL_NEAR_METERS` 150 and
`TERRAIN_MATERIAL_DETAIL_FAR_METERS` 900 fade the close-range material tile out past 900 m, and the
shader's own comment says the remaining 2 km tile then "mips to its own average". If that is what is
happening, everything past 900 m is a per-biome flat colour by construction, which is exactly the
distance band a tour or a low pass spends its time in.

**`tour_desert` is not a desert.** Its albedo frame is a single colour, (30, 133, 226), which is
`debug_ocean_albedo` — the camera is over water. It was nearly reported here as the worst material
result in the set. Re-author or re-aim it before using it to judge anything.

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

## 6c. The 28 July ocean/block diagnosis, and the visual-free work it exposed

Manual run `test-runs/manual/1785231815-2501476` is the reproduction. `capture-001`/`002` show the
raster ocean rising in broad hills at 232 m and 1,043 m clearance; `capture-003` shows the angular,
cell-like land and water ownership at 105.5 km. The run is current head `69cd04d9`, raster, frozen
scene time, and the wave diagnostic spans only **1.011 m** (`-0.485..+0.526 m`).

### The raster ocean is not mountainous because of its waves

`planet.wgsl::vs_main` classifies each mesh vertex independently as land or ocean. It puts a land
vertex at its displaced terrain height, an ocean vertex at sea level, and passes the result through
the ordinary interpolated `@location(5) ocean: f32`. A triangle crossing the coast therefore has
both its position and its land/water flag interpolated. The fragment becomes water when that flag
crosses 0.5, but it is still standing on the triangle between the zero-metre water vertex and the
raised land vertex. On a zero-to-2,000 m edge, the first water fragment can consequently be about
1,000 m high. The logged one-metre Gerstner range cannot produce the silhouette in the capture.

The ray path does not have this fault: `foveated_debug.wgsl::ocean_hit` intersects an analytic
sea-level shell, iterates the wave height, and chooses it only when it is in front of the terrain
hit. **The correct raster fix is the same separation:** draw land/bathymetry as terrain, then draw a
dedicated sea-level ocean shell/pass with its own depth, clipped by the sampled coast and depth-tested
against land. This is real render-path work, not a constant change. Do not try to hide it by
reducing the already-one-metre waves, making the interpolant `flat`, or spending more global LOD on
coastal triangles; those leave the mixed geometry wrong.

That separation is now implemented in raster. `vs_main` always produces land/bathymetry geometry;
the terrain fragment stage discards open-sea ownership. A separate `vs_ocean` projects the same
canonical, instanced, edge-stitched patches onto the analytic Gerstner shell. Its fragment stage
uses the exact complementary sampled-height/biome predicate, excluding polar ice and elevated
lakes, and it is drawn after terrain with the existing reversed-Z `Greater` depth test and depth
writes. The existing positive-height shallow-beach colour blend is retained, but it can no longer
raise the sea silhouette because all true open sea comes from the shell. LOD morphs and transition
dither are shared by both stages; the HUD's draw-call metric now counts both real batch draws.

`raster_ocean_uses_a_separate_analytic_shell` locks down the separated shader entry points and
complementary coast clipping. Deterministic scenario `ocean_coastline` replays the camera position,
view direction, 75° FOV, sun, and frozen planet-local location from the original 232m-clearance
manual capture. Run `1785237137-2559195` passes and its two stable probes read p90 **1.765m**,
maximum **3.264m**, across 43 compared points; the captured ocean is a sea-level shell rather than
the former land-to-water ramp.

### The high view has run out of source data, not mesh LOD

At the 105.5 km capture the raster selector draws L3–L8, but its source-delta histogram shows the
fine nodes falling back to the globally dense **L4** data. L4 is 128 × 2⁴ = 2,048 samples per cube
face edge, or **3,906.25 m per height/biome sample**. The baker's 4,096 × 2,048 working grid is
coarser still at about 6.1 km per cell. Splitting the mesh past that point only resamples the same
bilinear height cell and categorical biome neighbourhood, so it cannot round the coastline or
invent a less angular macro shape. At the time of the 105,492m capture the old 100–1,000km
altitude blend was only **1.00033×**, so it did not cause that capture. The current fixed 2x
experiment still adds no source information and instead makes this existing cell relief more
conspicuous.

The defensible quality options, in order:

1. Re-bake genuinely denser global source data. L5 plus an 8,192 × 4,096 working grid halves the
   block scale; exporting L5 from the existing working grid would only interpolate the same input.
   Validate memory before committing to it: the ray path's six height/max-height/biome/moisture face
   textures grow from about **0.286 GB at L4 to 1.142 GB at L5**. L6 is about 4.57 GB and is not
   viable on the 2 GB Quadro.
2. If L5 memory is too high, first quantise only the ray path's stitched height and conservative
   max-height textures to a tested fixed-range 16-bit representation. This can retain sub-metre
   height precision while making room for L5; the depth probe must prove raster/ray surface parity.
3. A monotone bicubic height reconstruction could soften L4 cell facets without more data, but it
   does not add coastline/biome information, costs more samples, and must be mirrored in CPU
   clearance plus both render paths. Treat it as a fallback, not as a replacement for source detail.

### Output-identical shader work removed now

Three calculations were paid after their contribution was mathematically zero:

- below-sea vertices ran the integer runtime-detail noise walk even though every octave's headroom
  was exactly zero;
- every land vertex evaluated all six Gerstner waves before selecting the non-water arm;
- raster and ray fragments built the close-material warped coordinate even after its weight reached
  zero.

Those are now explicit early/conditional paths in `shared_planet.wgsl`, `planet.wgsl`, and
`foveated_debug.wgsl`. Three-run Quadro means, raster, idle GPU,
`CATINGARDEN_PRESENT_MODE=immediate`, same release target:

| scenario | before | after | change |
|---|---:|---:|---:|
| `ocean_flyover` | 35.134 ms | **26.695 ms** | **−24.0%** |
| `orbit_once` | 23.584 ms | **22.431 ms** | **−4.9%** |

The ocean case isolates the important bailout: it has no land material work to skip and its water
vertices still need the six waves, so the removed cost is the zero-amplitude terrain-detail walk.
Before/after captures differ by 0–3 8-bit values, while independent repeat runs already differ by
0–2 from exposure/frame timing; later ocean captures are pixel-identical. There is no structural
image change.

The bailout-only commit validated **192 workspace tests, with 5 diagnostic instruments ignored**.
Raster `orbit_once` passed. Ray `orbit_once` passed and ray `ocean_flyover` remained finite; the
latter failed only the pre-existing §3 fallback assertion (256 observed vs 192), not a shader or
image assertion.

After adding the ocean shell, three-run Quadro raster means remain below the output-identical
bailout measurements rather than regressing: `ocean_flyover` is **24.990ms** and `orbit_once` is
**20.543ms**. The current full workspace result is **194 passed, 5 ignored**. Raster
`ocean_coastline` and `orbit_once` pass; raster `ocean_flyover` remains finite and still fails only
the same pre-existing fallback-count assertion.

## 6d. The 28 July paired raster/ray parity diagnosis — diagnostic complete, solver next

Manual run `test-runs/manual/1785238000-2567254` contains 16 captures arranged as eight frozen
raster/ray pairs. Within each pair the camera, 60-degree FOV, scene clock, exposure, blur/bloom,
HDR composition mode and all optional M8 experiments match. This is a good comparison, not two
nearby but different flights.

The pairs, in order:

| captures | height | result |
|---|---:|---|
| `001` ray / `002` raster | 316 m clearance | ray p90 **72.743 m**, correlation **-0.190**; raster p90 **1.156 m** |
| `003` ray / `004` raster | 2.57 km clearance | ray p90 **142.474 m**, correlation **0.045**; raster p90 **5.307 m**, correlation **0.835** |
| `005` raster / `006` ray | 162.6 km altitude | visually close; non-HUD luminance correlation **0.961** |
| `007` raster / `008` ray | 70.8 km altitude | ray-only contour/hatching structure begins around the blocky coast |
| `009` raster / `010` ray | 29.9 km altitude | the ray contour structure strengthens |
| `011` raster / `012` ray | 14.0 km altitude | the strongest ray-only banding in the set |
| `013` raster / `014` ray | 5.0 km altitude | closer in silhouette, but ray material/relief remains smoother |
| `015` raster / `016` ray | 392 m clearance | ray p90 **56.790 m**, correlation **0.478**; raster p90 **2.289 m**, correlation **0.998** |

The original probe reported zero comparable points for the 5–163 km captures because its diagnostic
distance cap excluded those hits. The deterministic scenario below opts into a 200 km comparison
limit; ordinary probe scenarios retain the conservative 4 km default.

### What the low pairs prove

No `near-field window built` event occurs before the first two ray captures. The CPU/raster surface
resolves local L10–L17 sources across those compared points while ray remains on the six globally
dense L4 faces. Their 72–142 m error and near-zero/negative detail correlation therefore begin with
different source surfaces, before lighting or presentation is considered.

An L12 near-field window is built immediately before `016`. In that capture ray rendered height
minus CPU macro height averages **0.156 m**, so the window has closed the broad macro offset. But the
CPU surface contains **-24.154 m mean** detail relative to that macro surface and ray still reads
56.790 m p90 from truth. Raster reads 2.289 m p90 at correlation 0.998. The final image is
correspondingly much flatter in ray. This isolates a second problem after residency:
`refine_detail_hit` is finding the wrong detailed crossing or returning its macro fallback.

### The concrete parity gaps

1. **Near-field residency is all-or-nothing.** `Terrain::near_field_sources` refuses the whole 8×8
   window if any block resolves only to `dense_level` or coarser. Raster can still use fine sources
   over the visible part of the same view.
2. **The window carries height only.** `NearFieldWindow` and the ray binding upload one R32Float
   height texture. Ray biome and moisture sampling always uses the global L4 arrays, so close
   material ownership cannot match raster even after height residency improves.
3. **The detailed hit search assumes too much topology.** `refine_detail_hit` chooses one direction
   from the detailed function's sign at the macro hit, walks 12 samples, then bisects three times.
   The synthesised field is non-monotonic along a grazing ray. A local sign does not prove that the
   chosen side contains the first visible crossing; failure returns the macro hit.
4. **Ray macro normals retain the dense-face footprint.** `terrain_normal` always uses
   `2 / face_quads` even when `sample_height` is reading the L12 window. Raster instead clamps its
   normal footprint by camera distance and actual source-texel spacing. The ray consequently loses
   baked slope and feeds different normals into both lighting and slope-based material weights.
5. **Ocean ownership differs again.** Raster now uses the exact complementary
   `is_open_ocean_surface` predicate and a separately depth-tested shell. Ray still evaluates soft
   `outmap_ocean_coverage` at the shell, then can mix ocean again at the terrain hit. This is a
   definite coast mismatch; whether it accounts for all of the 14–71 km hatching still needs the
   staged capture below.
6. **The final ray presentation is deliberately lower resolution.** Ray final output passes through
   the 75%-per-axis warp/unwarp path while raster is direct. It cannot explain 57–142 m depth error,
   and the close 162.6 km pair shows it is secondary, but it sets the final pixel-parity ceiling.

### Deterministic staged diagnostic — DONE

`render_path_parity.json` replays four static planet-relative poses from the manual run at 70.8 km,
29.9 km, 14.0 km, and 738 m altitude. `scripts/run-render-path-parity.sh` builds into the isolated
target and runs raster and ray through final, raw-albedo, surface-lighting, and aerial modes, then
runs the env-only ray hit-status view. The exact committed `6328a7d` Quadro set is:

| path/mode | run |
|---|---|
| raster final / albedo / lighting / aerial | `1785249825-2674511` / `1785249848-2674713` / `1785249865-2674850` / `1785249885-2675034` |
| ray final / albedo / lighting / aerial | `1785249906-2675210` / `1785249934-2675469` / `1785249979-2675856` / `1785250026-2676228` |
| ray hit status | `1785250076-2676645` |

All nine scenario runs pass their finite/screenshot/sample-floor assertions. The extended probe
makes the geometric disagreement explicit:

| altitude | raster p90 | direct ray p90 | ray near-field state |
|---:|---:|---:|---|
| 70.8 km | 51.422 m | 4416.316 m | no requested window |
| 29.9 km | 45.530 m | 4358.699 m | requested L5; 9/64 blocks above L4; rejected |
| 14.0 km | 24.472 m | 4332.534 m | requested L6; 30/64 blocks above L4; rejected |
| 738 m | 2.585 m | 51.247 m | active L12 window; all blocks resolve only to L5 |

The final warped ray p90 values are 4368.859, 4368.185, 4338.407, and 50.094 m. Their difference
from the direct debug path is negligible beside the 4.3 km high-view and 51 m low-view errors, so
the warp is not the primary defect. Raw-albedo full-frame raster/ray correlation falls to 0.510 at
14 km and the contour pattern is already present there, before lighting or aerial composition.

Hit status resolves the remaining ambiguity. At 70.8/29.9/14.0 km, every probe sample at or above
p90 is a red macro fallback (9 of 81 at each pose), directly tying the kilometre errors to failed
detail brackets while the all-or-nothing window is unavailable. At 738 m, none of the 54 comparable
probe points is fallback: 48 are reported bracketed and six have no relief, yet all seven samples at
or above the 51.247 m p90 are green “bracketed” hits. The loaded window therefore does not cure the
geometry; the one-sided comb is also accepting the wrong crossing. This is measured evidence for
both of the next two solver changes below, not a speculative shader rewrite.

### Implement and validate in this order

1. **DONE: deterministic paired parity scenario and staged diagnostics.** The committed run set and
   conclusions are above. Keep this harness unchanged as the solver regression loop.
2. **DONE: build one unified ray surface window:** height, biome, moisture and actual resolved source
   level/coverage, assembled through the same ancestor resolver as raster. Permit mixed fine/coarse
   blocks rather than disabling the whole window. The source-level channel is required so an L4
   block resampled into the window cannot pretend to be L12 and suppress the runtime ladder.
3. **DONE: replace the one-sided comb with a first-visible-crossing search.** Search front-to-back over a
   conservative detailed-surface interval, then refine the first sign change. Raising
   `RAY_DETAIL_HIT_STEPS` alone may reduce error, but preserves the topology bug and directly spends
   the already-tight ray budget.
4. **DONE for macro normals: share the raster normal-footprint rule with ray**, using camera distance and resolved sample
   spacing. After hit correctness, derive the ray detail filter from the raster transfer function
   rather than independently tuning `RAY_DETAIL_FILTER_OVERSAMPLE`. Do not restore the rejected
   `1/sin(incidence)` filter widening from §8.
5. **DONE: make ray ocean ownership identical to raster:** exact open-sea predicate, analytic shell
   compared against terrain depth, lake/ice exclusions, and shallow beach blending only on positive
   terrain.
6. **DONE: compare direct full-resolution ray with warped ray.** A difference remaining only in
   the warped output is a measured foveation quality/performance trade-off, not terrain disagreement.

Acceptance is geometric before aesthetic: ray p90 no more than a few metres above raster at the low
poses, detail correlation at least 0.95, no bracket/fallback bands during motion, matching F9 raw
albedo ownership, and then Quadro timing against the 33 ms target. Keep the shared high-altitude L4
block shape separate: perfect path parity makes both paths agree on it, but improving it still needs
the L5/rebake decision in §6c.

### The four parity repairs — implemented and measured

The mixed window now remains enabled whenever all 64 requested blocks have resident ancestors. It
uploads R32F height, categorical R8Uint biome, bilinear R8Unorm moisture, and the actual source level
of each 8×8 block. Height/material sampling and runtime-ladder filtering therefore follow the same
ancestor that raster resolved instead of treating a resampled L4/L5 block as requested-level data.
The ray hit refinement searches front-to-back, expands toward the ladder's conservative amplitude
bound when the local interval starts inside or ends outside the detailed surface, then bisects the
first outside-to-inside bracket four times. Ray macro normals use the raster 1%-of-camera-distance
footprint with the resolved sample-spacing floor and the shared 0.5–256m clamp. Finally,
`is_open_ocean_surface` is shared by both shaders: ray open sea is an exact ice/lake-excluding shell
hit compared against terrain depth, while only positive terrain retains the shallow-beach blend.

The final Quadro M1000M `PRESENT_MODE=immediate` matrix is:

| path/mode | run |
|---|---|
| raster final / albedo / lighting / aerial | `1785281717-169735` / `1785281737-169876` / `1785281754-170001` / `1785281773-170151` |
| ray final / albedo / lighting / aerial | `1785281793-170295` / `1785281841-170612` / `1785281917-171129` / `1785281996-171643` |
| ray hit status | `1785282078-172268` |

All nine runs pass. At the 738m pose, final warped ray p90 falls **50.094 → 3.574m** while raster
remains 2.585m; direct full-resolution ray is 3.273m. Detail correlation rises **0.394 → 0.997**
(0.998 direct), and the direct raw-albedo captures correlate 0.9983 with only 0.0008 mean absolute
RGB error. A simple blue-vs-green ownership mask agrees on 99.91% of pixels there. The remaining
warp penalty is only 0.301m, so foveation is no longer the close-range geometry fault.

This correctness work is **not a speed win**. The settled low-pose final-ray mean rises from
25.74ms to 64.05ms on the same adapter, because the real L5-backed window correctly re-enables the
long runtime-detail octaves and the first-crossing search now performs work that the old fallback
skipped. Direct diagnostic modes are intentionally unwarped and measure about 99.9–102.8ms at that
pose. The 33ms target is therefore missed and needs a separately measured solver optimization; do
not undo source-level truth to recover it.

Nor is high-view parity closed. At 70.8/29.9/14.0km, final ray p90 remains
4368.859/4357.376/4337.762m versus raster 51.422/45.530/24.472m. The 14km raw-albedo correlation is
0.904 and the hit-status capture still contains fallback bands. The mixed window fixed the
all-or-nothing residency decision, but blocks still backed by the globally dense L4 source retain
the old kilometre-scale source mismatch; exact ocean ownership cannot make displaced coastlines
coincide. Treat the 738m result as the completed four-repair proof and the high-view L5/rebake/source
coverage decision as the next geometry task, not as a shading or foveation problem.

### 6e. Near-field texture-size regression — fixed

Manual replay `test-runs/manual/1785679238-738888` showed large dark/grey rectangular holes across
the planet in the high-altitude captures. The raster near-field path had classified ordinary
131x131 tile textures as the 1025x1025 near-field window because `near_field_texture()` tested
`width > 129`; the shader then applied 1024-quad coordinates to 129-quad tiles, causing invalid
out-of-range reads. The test now requires the exact near-field width (`width == 1025`), leaving
ordinary tiles on their gutter-aware coordinate path. Orbit replay
`test-runs/orbit_once/1785679670-742426` passes with an intact planet and no rectangular holes.
The orange stippled oval visible in one high-altitude frame is a separate LOD transition/dither
artifact and remains a later visual-polish item.

## 7. What the terrain actually is now

`shared_planet.wgsl`, mirrored in `planet.rs`, guarded by
`shader_detail_ladder_matches_the_cpu_clearance_ladder` (which reads the constants back out of the
shader source so the two cannot drift).

```
TERRAIN_DETAIL_ROUGHNESS            0.06      amplitude = roughness × wavelength × tilt
TERRAIN_DETAIL_START_WAVELENGTH     4096 m    starts where the baked data stops
TERRAIN_DETAIL_OCTAVES              13        down to 1 m
TERRAIN_DETAIL_LONG_GAIN            1.0       long-wave random boost disabled for evaluation
TERRAIN_DETAIL_TILT_TAPER_METERS    256 m     dormant until LONG_GAIN exceeds 1
TERRAIN_DETAIL_RIDGE_SOFTNESS       0.15      sqrt(n² + s²), not abs(n)
TERRAIN_DETAIL_RIDGE_CENTRE         0.348609  ┐ properties of the *softened* fold —
TERRAIN_DETAIL_RIDGE_SCALE          2.063534  ┘ re-derive these if the fold changes
TERRAIN_DETAIL_RIDGE_STRENGTH       1.0       fully folded: creases stay creases
TERRAIN_DETAIL_RIDGE_NORMALISATION  1.0       DERIVED: 1/sqrt((1-s)² + s²)
TERRAIN_DETAIL_ATTENUATION_SLOPE    4.0       multifractal damping by slope so far
TERRAIN_DETAIL_HEADROOM_FACTOR      5.5       per-octave land weighting
TERRAIN_DETAIL_TOTAL_AMPLITUDE      491.5 m   DERIVED: finite unboosted octave sum
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
  gives relief-correlated amplitude for free. The retained 5.5 factor comes from the sea-level safety
  proof; a test walks elevations asserting the ladder can never reach sea level.

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
- **Re-author scenario camera heights after any ladder or outmap change.** Changing the field moves
  the ground. `landing_site_eye_level` ended 2 m underground after one earlier ladder change, and
  the Earth-like rebake moved the sparse centre again; the clearance assertion caught both.
  Landing-site ground is currently approximately **909m raw / 3,636m presented**.
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

## Latest continuation — flat water ownership and narrower local relief (6 August 2026)

The newest manual capture (`test-runs/manual/1786024172-1341075`) showed a black square and
repeated vertical fins at the grazing terrain line. This was not a new terrain hole: fixed L7 flat
triangles were sourced from mixed L4 land/water footprints while the analytic sea shell was also
submitted, so the two passes had no common per-triangle owner. In `experiment/flat-triangle-wireframe`,
flat mode now retains the categorical terrain fragment and suppresses ocean-shell batches; normal
render modes retain the separate shell. This removes the ownership race without changing ordinary
coastline/ocean rendering.

The procedural game shape now reduces the broad highland/lowland base and adds a thresholded,
cubed frequency-360 ridge field (plus the existing range-scale ridges). The result is intended to
replace kilometre-wide plateaus with narrower local summits and enclosed valleys. Because the full
4096x2048 export exceeded the disk/time budget on this machine, a validated 1024x512 dense-L4 bake
was promoted for this flat-L7 experiment. Manifest SHA-256 is
`010fd7cecb15edf1e0d91a0e63cbcf062113b08615bcb27bdf5783578c67d0ea`; the prior active L0-L18 data is
backed up at `assets/outmaps/test-planet.pre-local-mountain-backup-20260806-1540`. The old sparse files
remain physically in the active directory but are not listed by the new manifest, so the renderer
resolves the new dense L4 data consistently.

The global-summit instrument reports raw L4 6,277.5546875m and presented/prominence 25,255.25958947m
at 23.360523290N, 62.414690487W. `highest_prominence_peak` and the F4 constants were re-authored to
that direction and a 152.4m clearance pose. Baker procedural-shape tests, flat/ocean shader tests,
WGSL validation, and `--validate` pass. The first post-bake run correctly caught the stale 13km
clearance; the pose was then corrected. A fresh GPU rerun is still required after freeing disk space.

The next manual set (`test-runs/manual/1786040127-1473748`) confirmed that the renderer was not
using stale terrain: its log names the active `assets/outmaps/test-planet` source and its local
CPU probes are approximately 1,000-1,200m. The view was simply a broad lowland, and the prior L4
surface contained only 5.012% sampled values above 1,000m. To make the game-scale terrain read as
mountainous across more of the planet, the generator now widens the mountain-region mask and adds
a resolvable foothill field, while retaining the narrow and local peak thresholds. The validated
1024x512 staging bake `assets/outmaps/test-planet.foothills-20260806-1835` is now active with
manifest SHA-256 `3e1d773bcbc1d2050d84db30bbdc0c1417d3bd14c8bcd63cfab95355f66126e`; the immediately
prior active directory is preserved at `assets/outmaps/test-planet.pre-foothills-backup-20260806-192315`.
Sampled dense-L4 coverage is now 17.723% above 1,000m, 3.376% above 2,000m, and 0.611% above
3,000m (previously 5.012%, 0.745%, and 0.098%). `cargo test -p catinthegarden-baker
procedural_game_shape_is_deterministic_asymmetric_and_varied` and baker `--validate` pass. A fresh
GPU/manual capture remains the final visual check.
## Performance report P0 pass — 6 August 2026

The root `PERFORMANCE_REVIEW.pdf` P0 recommendations are implemented. In flat-triangle mode,
`vs_main` keeps baked displacement, geometric normals, and triangle specular but supplies identity
aerial components instead of evaluating aerial perspective and terrain distance fog whose varyings
are not consumed by the categorical fragment path. Fixed exposure now skips luminance extraction,
all downsample passes, readback mapping, and adaptation; profiler metadata records zero luminance
rather than stale timestamp queries, and the HDR API guards the same invariant. Interactive exposure
assertions still observe every frame, while JSONL serialization is limited to scenario evidence or
the existing spatial cadence; logs use a buffered writer and flush after captures, manifests, and
state drop. Terrain edge-neighbour stitching, mixed incoming/outgoing surface probes, and resident
tile fallback now probe dyadic level indexes, retaining a full-scan fallback at ambiguous boundaries.
A frontier-index regression covers mixed-level containment. `cargo fmt --all`, `cargo check -p
catinthegarden-app`, all 197 app tests (6 ignored), and all baker tests (44) pass. Headless Xvfb
cannot provide a valid DRI3 presentation surface, so a Quadro A/B GPU timing comparison is still
pending; P1 atmosphere/background and low-density water experiments remain intentionally deferred
until that measurement identifies a bound.

## Mountain visibility gate and sub-30m flight — 6 August 2026

Screenshots alone did not make the requested mountain character measurable, so the new ignored
`relief_survey::tests::mountain_visibility_metric` instrument defines a geometry-facing gate. It
samples a 9x9 grid of candidate ground positions around the active summit, casts 16 azimuths at
2, 4, 6, 8, and 10km, and measures the target surface's elevation above the candidate's local
tangent. A mountain pass requires at least three qualifying rays at **30 degrees or more** and
at least **2,000m** range; this avoids passing on a single close noisy triangle. Run it with:

```bash
TMPDIR=/home/dad/catingard/tmp-rust CARGO_TARGET_DIR=/home/dad/catingard/.target-metric \
  cargo test -p catinthegarden-app --bin catinthegarden-app \
  relief_survey::tests::mountain_visibility_metric -- --ignored --nocapture
```

The current active bake measures 56 qualifying rays and a 44.30-degree maximum. F4 now enters at
2m above the synthesized surface, idle clearance is 0.75m, and moving flight uses a 5m safety
envelope instead of 30m. The high-speed scenario assertion is lowered to 4.5m accordingly; the
focused scenario and close-flight unit tests pass.

## Mountain visibility check throughput — 6 August 2026

The ignored baker instrument
`terrain::tests::mountain_visibility_throughput` runs the proposed eight azimuths and five target
ranges against an in-memory 512x256 generated height field, avoiding filesystem tile I/O while
retaining spherical bilinear lookups and elevation calculations. Three optimized runs measured
1.49M, 1.51M, and 1.41M rays/s (7.35M profile samples/s average). At that rate the exhaustive
100m global scan for the current 4,000km-radius planet would take about 30.4 hours; the number
excludes terrain generation, erosion, export, and any parallelisation beyond the benchmark thread.

## Mountain-coverage candidate bake — 6 August 2026

The procedural generator now includes a separate seam-safe 1,500x ridge family with a bounded
8,500m contribution. This is intended to distribute steep game-scale peaks instead of relying on
one rare massif. The baker's `--mountain-coverage` option reports an area-weighted survey over all
positive source-grid positions, using eight evenly spaced azimuths and 2/4/6/8/10km target ranges;
a position passes if any direction reaches 30 degrees elevation. It is deliberately a fast
generation-time gate: the 400m exhaustive scan would repeatedly interpolate a source grid that is
much coarser than 400m.

One 1024x512 procedural bake with 256 erosion iterations was produced and promoted to the active
`assets/outmaps/test-planet`. Its manifest SHA-256 is
`8311569a2b2d5d8c0f565e38ad44a935d4c3c1c13954ebe583d467df7c73bb67`; the previous active surface
is preserved at `assets/outmaps/test-planet.pre-mountain-coverage-backup-20260806-220003`. The
matching deterministic snapshot reports **1.37%** of area-weighted land positions passing (18,909
of 275,654 source positions; 161,034 qualifying rays), below the requested 25%. This candidate is
intentionally not being auto-tuned or rebaked; the next action is manual in-game visual review.

## Flat-triangle raised-water-cliff repair — 6 August 2026

Manual capture set `test-runs/manual/1786050593-90025` showed two related flat-mode artefacts:
raised vertical faces coloured as water and broad cliffs apparently rising out of the sea. The
flat terrain vertex path now forces ocean and lake-owned vertices to sea level, not only lakes.
The categorical triangle colour path samples the three triangle corners and chooses a non-water
biome whenever a mixed land/water triangle contains positive macro height, preventing land cliffs
from inheriting the blue ocean palette. This remains limited to the flat-triangle experiment;
normal terrain/ocean ownership is unchanged. All 48 terrain shader/unit tests pass; a fresh GPU
capture is still the visual confirmation step.

The follow-up manual set `test-runs/manual/1786051220-100974` confirmed the blue palette fault
was gone, but exposed the remaining cause of the broad vertical walls: positive source samples
whose coarse categorical owner was water were still being flattened to sea level. The flat vertex
path now applies that flattening only when the sampled macro height is non-positive, keeping
positive mixed shoreline/ancestor-fallback samples continuous with adjacent land. The focused
terrain suite remains green (48 tests) and `cargo check -p catinthegarden-app` passes; a fresh GPU
capture is still required to verify the wall reduction visually.

The next manual run `test-runs/manual/1786051770-106236` logged the fixed-L7 view while the
camera was moving (up to 300 drawn chunks against the 256 active budget), but its screenshot
manifest was empty. The depth-fighting report is consistent with the flat experiment's outgoing
and incoming LOD grids being rendered together during camera-driven frontier changes. Flat mode
now disables those cross-fades and clears stale transition nodes; the normal renderer retains its
existing dithered transitions. Focused terrain tests and app checking pass; a new PNG capture is
still needed for visual confirmation.

Capture `test-runs/manual/1786055251-139165` still showed salt-and-pepper seams around large flat
facets. The image was taken near 150m clearance over the rendered surface; the run's settled
frames had no extra drawn chunks, so the remaining coverage was not a live parent/child overlap.
Flat fragment entry points now bypass the LOD discard entirely (terrain and ocean), in addition to
the CPU-side transition disable. This prevents stale binaries or a transient transition pipeline
from exposing competing depth-writing facets in the diagnostic view. Focused tests and checking
pass; another rebuilt capture is required.

## Checkout metadata relocation and flat frontier skirt repair — 7 August 2026

The checkout's `.git` directory was still the old 67MB metadata store at `fe24c5b` after the
active 69MB metadata directory moved to `/home/dad/catingard-tmp/catingard-git`. The old directory
is preserved at `/home/dad/catingard-tmp/catingard-dotgit-stale-20260807`; the checkout now uses a
`.git` file pointing at the relocated store and default `git status` resolves to `101c433`.
Manual capture `test-runs/manual/1786098424-478229` still showed stippled mixed-L3/L7 frontier
bands. Flat terrain now discards skirt filler fragments, leaving only actual surface triangles in
the diagnostic path. Forty-eight terrain tests and app checking pass; fresh visual confirmation
remains required.

The LOD frontier remains intentionally mixed under the 256-leaf budget, but its split priority
now applies a bounded near-camera weight. When the cap binds, L6/L7 demand near the camera wins
over level-normalised horizon demand, moving lower-detail boundaries outward without increasing
the draw budget. The priority regression passes alongside the existing terrain suite.

## Flat snow/grass stipple repair — 7 August 2026

Manual capture `test-runs/manual/1786110636-583021` still showed salt-and-pepper colour changes
where categorical snow met grass. The flat fragment path was reconstructing the triangle-centre
and corner source UVs from perspective-interpolated `source_uv`/`tile_uv` varyings; nearest
biome samples could therefore change inside one rendered triangle. The vertex output now carries
the instance-constant source UV offset as a flat varying, and the flat palette path reconstructs
all centre/corner samples from that offset plus the flat source scale. Each triangle consequently
keeps one stable biome colour while its geometric normal and per-triangle lighting remain intact.
The focused flat shader test, 49 planet tests (one ignored), and app checking pass; a fresh GPU
capture is still the final visual confirmation.

## Terrain ambient-fill repair — 7 August 2026

The latest manual frames `test-runs/manual/1786110636-583021` (`capture-003/004`) showed steep
mountain facets collapsing to near-black while the sun-facing snow was bright. `sky_diffuse_irradiance`
was sampling only the facet normal and could therefore return zero for back-facing terrain even
under daylight. It now retains a bounded 35% contribution from the analytic zenith sky radiance,
then applies the existing irradiance cap. This lifts shadowed terrain without adding moonless
night-side emission and is shared by flat and ordinary terrain/ocean lighting. The focused terrain
suite (48 tests) and app checking pass; a fresh daylight GPU capture is required to tune/sign off
the visual level.

## Flat ambient varying inter-stage limit correction — 7 August 2026

The first ambient-fill follow-up accidentally added vertex output location 16. wgpu 29 permits
locations 0 through 15 on this adapter, so the renderer panicked while creating the LOD terrain
pipeline before drawing. The extra varying is removed: flat mode reuses the existing flat
`detail_anchor_direction` slot to carry the source UV offset, while normal mode still carries the
anchor direction. The shader remains within the 15-location limit; terrain tests and release
compilation pass. A GPU launch on this headless shell exits before window creation, so Quadro
visual validation remains pending.

## Daylight ambient boost — 7 August 2026

Manual capture `test-runs/manual/1786112204-598838/capture-001` showed a bright blue sky and
near-black terrain facets. The prior bounded zenith fill (35% share, `SKY_DIFFUSE_LIGHT_SCALE`
0.40) was too weak at this high-altitude view. The shared sky diffuse response is now raised to
`0.70`, with the zenith contribution increased to 75% before the existing peak cap. It remains
fully analytic-sky-driven, so the moonless/night-side path still receives no artificial baseline.
The 48-test terrain suite and app checking pass; fresh Quadro capture is required to confirm the
new daylight balance and ensure snow highlights remain controlled.

## Twilight horizon ambient fill — 7 August 2026

The follow-up capture `test-runs/manual/1786112609-603233` still showed nearly black terrain
under a bright orange sunrise/sunset sky (average luminance 0.033, exposure at 3.998). The local
and zenith sky samples used by terrain lighting remained dark at these low solar elevations. The
ambient path now adds a bounded sunward-horizon analytic sky sample (65% before the existing cap),
while retaining the 70% diffuse scale and 75% zenith share. This is still zero on the deep night
side and only supplies twilight-coloured fill to shadowed facets. The 48-test terrain suite and
app checking pass; fresh Quadro capture is required to verify the result.

## Screen-centre LOD priority repair — 7 August 2026

Manual capture `test-runs/manual/1786113396-610508` showed a conspicuous coarse square around the
screen centre while farther surface regions were L7. The run was orbiting at 6,000km with the
256-leaf budget binding and an L2–L7 frontier. Split priority already favoured depth and near
camera distance, but it could still spend the cap on off-centre L7 candidates. The selector now
intersects the camera centre ray with the conservative terrain shell and applies a bounded 4x
priority boost to the leaf containing that hit, propagated through its split ladder. The budget
and normal horizon falloff are unchanged. Fifty planet tests (one ignored) and app checking pass;
a fresh orbital capture is required to confirm the square is gone.

## Wide/tall procedural mountain rebake — 7 August 2026

The active procedural mountain-coverage bake was backed up before replacement at
`assets/outmaps/test-planet.pre-wide-tall-mountains-backup-20260807-154700` (358 MB). The
procedural generator now doubles the broad mountain-belt wavelength and doubles the mountain
amplitudes; the sharp summit-spine frequencies remain unchanged so the wider ranges do not become
flat caps. The export ceiling is raised from 9,000m to 18,000m so the requested height increase is
not clipped.

The active replacement was baked with seed `0xEA272026`, 1024x512 working grid, dense/max L4,
256 erosion iterations, and `--procedural-terrain --mountain-coverage`. It validates 2,046 tiles;
manifest SHA-256 is `618b3a76a60f0d0ce9947c9b5c9ee89585530c810b3ec09c1ff829637e72e847`, generated
height range is -4,427.945m to 17,636.729m, and the area-weighted mountain gate reports 8.54%
passing land positions (42,451/275,906; 481,178 qualifying rays), up from 1.37% on the prior
candidate. Workspace checking and all 35 baker tests (33 passed, 2 ignored) pass. Renderer/F4
poses and fresh GPU/manual visual sign-off remain pending because the source height ceiling and
summit location changed.

## Twilight terrain balance and distant facet lift — 7 August 2026

The latest manual capture `test-runs/manual/1786114540-629317/capture-001.png` was the fixed-L7
flat-triangle presentation at 146.7km altitude, with exposure fixed at 1.0 and solar elevation
about -1.54 degrees. Flat mode intentionally bypassed both aerial perspective and distance fog,
so the foreground inherited the full synthetic red sunward-horizon ambient while distant
back-facing facets had no atmospheric fill and fell nearly black. This was not an exposure or
fog defect.

Terrain sky-fill weighting now reduces the sunward-horizon contribution from 0.65 to 0.49
(approximately 75% of the prior red ambience) and raises the neutral overhead fallback from 0.75
to 0.90. Flat terrain now composes its categorical face lighting with the existing affine aerial
transmittance/in-scatter path for facets beyond 80km; near flat-mode facets retain the cheaper
path, and smooth terrain behaviour is unchanged apart from the shared ambient weighting.

Same-pose `outlined_shadows` capture comparison: the prior binary logged 15.309ms for the final
frame and remained nearly black away from the sunward strip; the corrected build logged 18.476ms
and visibly lifted the distant terrain while reducing the foreground red cast. The corrected run
passed its finite-metric and capture assertions. `cargo fmt --check`, workspace `cargo check`,
the focused shader test, and the full app suite otherwise pass; the only full-suite failure is the
pre-existing `flight_collision_sweep_catches_ground_between_safe_endpoints` assertion.

## Continuous flat-mode aerial handoff and triple-width mountain rebake — 7 August 2026

The latest manual capture `test-runs/manual/1786119079-698000/capture-001.png` straddled the
80km flat-mode aerial cutoff introduced by the previous lighting fix: camera clearance was
61,473.9m, the centre hit was 74,620.3m, and farther vertices crossed 80km. Flat facets therefore
jumped from identity lighting to full aerial transmittance/in-scatter. The cutoff is removed;
flat terrain now evaluates the bounded aerial model continuously at every distance while still
skipping only the optional smooth-mode horizon fog overlay. The fixed-L7 categorical fill remains
unchanged.

The active outmap was backed up at
`assets/outmaps/test-planet.pre-triple-width-mountains-backup-20260807-172229` before rebaking.
The procedural mountain-region frequency changed 0.875 -> 0.2916667 and the primary ridge
frequency 1.675 -> 0.5583333, tripling broad range width while retaining the 11x/360x sharp
summit families and 18,000m ceiling. The new v3 triple-width bake has manifest SHA-256
`fdd816aa0240405855ed26525ab3b19debb308c78998aff811fefe92680926ee`, height range
-4,427.945m .. 17,681.004m, and 8.16% mountain-visibility coverage (42,174/275,827 positions;
470,541 qualifying rays).

The active highest-prominence pose was re-authored to direction
[0.903665234, -0.419385975, -0.086628801] (24.795828S, 5.475859E). Runtime landing height is
70,537.257m; the F4/highest-prominence scenario now starts at 152.4m clearance and passes.
Baker tests pass, the app suite reports 198 passed / 1 pre-existing collision-sweep failure / 7
ignored, and the GPU highest-prominence scenario passes with 152.400m measured clearance.
Stale generated Cargo target caches were cleaned, leaving only `.target-ambient` and
`.target-baker-triple`.

## Wide-spaced mountain bases and daytime horizon correction — 7 August 2026

Manual set `test-runs/manual/1786120502-723576` confirmed two separate issues. Captures 001 and
003 showed the visually dominant coverage term as undersampled one-sample cones: its 1500x
frequency was below the roughly 24.5km equatorial spacing of the 1024x512 bake. Capture 002 was
not a sky-colour failure—the sky probe was blue while the daytime terrain horizon was orange. The
new all-distance flat aerial path was injecting high-gain forward Mie in-scatter into the flat
terrain while the fullscreen sky remained blue.

The coverage family now uses frequency 750 (twice the base width and fewer major ranges). The
separate local family uses frequency 1500, is gated by land interior rather than the major
mountain belt, and is reduced to 3,500m so smaller mountains fill the gaps without becoming
competing peaks. The replacement was baked at the production 4096x2048 grid, not the aliased
1024x512 preview. The prior v3 active outmap is backed up at
`assets/outmaps/test-planet.pre-wide-spaced-mountains-backup-20260807-174725`.

The v4 active bake manifest SHA-256 is
`6c329c435316ff43501a1117a77d2caff6d09ec8040670ba7eba121b6169ad60`; height range is
-4,428.371m .. 17,754.176m and the visibility survey reports 36.09% passing land positions
(1,699,827/4,423,187; 22,439,038 qualifying rays). The summit/F4 pose was re-authored to
10.846446N, 105.864680W; the runtime landing height is 71,000.415m and the GPU summit scenario
passes at 152.400m.

Flat-mode aerial transmittance remains continuous, but in-scatter now fades smoothly from zero at
20km to full strength at 180km. This removes the orange daytime terrain band without changing the
normal renderer's twilight constants. Focused shader, baker, summit, orbit, workspace-check, and
full app validation pass; the only full-app failure remains the pre-existing flight collision
sweep assertion (198 passed, 1 failed, 7 ignored).

## Natural mountain relief and glacial valleys — 7 August 2026

The latest manual capture `test-runs/manual/1786121751-736278/capture-001.png` still showed
needle-like peaks. The cause was source-grid aliasing: the v4 750/1500 relief families reached
undersampled higher octaves at the production grid. The procedural profile now uses resolved
250/500/1000m-scale coverage and 600/1200m local relief, smoother thresholds, a mountain-belt
range gate, and lower narrow-spine amplitudes. Broad ranges remain high while isolated all-land
teeth are suppressed.

The bake also now carves sustained high-flow rivers into a 3-cell-wide U-shaped glacial valley.
The centre drops by 12% of local height, bounded to 250–1,200m, with a 1,000m shoulder rise;
small runoff traces are ignored. Flow is recomputed after carving before moisture and biome
classification so the valley floor remains hydrologically authoritative.

The previous active outmap is preserved at
`assets/outmaps/test-planet.pre-glacial-valley-v5-backup-20260807-180657`. The replacement is a
4096x2048 procedural/mountain-coverage bake with 256 erosion iterations, manifest SHA-256
`4e4feb6dbd3573242a1773289578bd6abffd553d7ac32d465950bb5d19822181`, height range
-4,428.371m .. 17,963.693m, and 13.76% mountain-visibility coverage (555,989/4,405,469
positions; 6,365,555 qualifying rays).

The global summit survey now measures 72,022.334m ASL at 65.822892S, 147.056036E; the runtime
surface at the re-authored pose is 71,851.459m. F4/highest-prominence starts 152.4m above that
surface and the GPU scenario passes. Orbit also passes. Baker tests (33 passed, 2 ignored),
workspace check, and app tests (198 passed, 1 pre-existing collision-sweep failure, 7 ignored)
complete successfully. A fresh low-flight human capture is still the final visual sign-off for
valley width and naturalness.

## Double-wide and double-tall mountain rebake — 7 August 2026

Following the v5 valley pass, the procedural relief bands were widened again: the mountain belt
and primary ridge frequencies were halved, as were the resolved 125/250/500m coverage and
300/600m local families. The narrow summit spine remains resolved so local prominence is not
flattened. Positive relief amplitudes were doubled, including the glacial valley depth/shoulder
bounds. The export ceiling is now 72,000m to avoid clipping overlapping ridge families into
unnatural plateaus.

The prior active outmap is preserved at
`assets/outmaps/test-planet.pre-double-wide-tall-backup-20260807-184048`. The replacement
4096x2048 bake has manifest SHA-256
`a0c393c0d843eb527f333b79064ae8bf0ee1bf86542d65ef446012af0417f0af`, height range
-4,428.371m .. 53,262.195m, and 21.47% mountain-visibility coverage (907,631/4,403,618
positions; 10,072,959 qualifying rays).

The global summit survey now measures 213,085.256m ASL at 19.024872S, 42.641923W; the runtime
surface at the re-authored pose is 213,049.325m. F4/highest-prominence starts 152.4m above that
surface and the GPU summit scenario passes. Orbit passes. Baker tests (33 passed, 2 ignored),
workspace check, and app tests (198 passed, 1 pre-existing collision-sweep failure, 7 ignored)
complete successfully. A fresh low-flight capture remains required for visual sign-off because
this is a deliberately extreme game-scale height change.

## Wider snow-covered summit peaks — 8 August 2026

The latest manual capture `test-runs/manual/1786218516-25893/capture-001.png` showed that the
large mountain range itself read well, but its white summit peaks were too needle-like. Those
peaks are the baked `narrow_peak` family, not runtime local detail (`GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE`
remains zero). Its base frequency is now 4.4 instead of 11.0, widening the snow-covered summit
features by approximately 2.5x and reducing their count while leaving the broad range height and
coverage families unchanged.

The prior active outmap is preserved at
`assets/outmaps/test-planet.pre-wider-snow-peaks-backup-20260808-205513`. The replacement
4096x2048 bake has manifest SHA-256
`c2eeb24ad2b353c6819890b4c7fbb0933ca204dc67edcc184f24f75f3e122e0a`, height range
-4,428.371m .. 51,685.105m, and 21.46% mountain-visibility coverage (907,105/4,403,613
positions; 10,068,976 qualifying rays).

The global summit survey now measures 206,722.059m ASL at 28.400852S, 35.558844W; runtime
surface at the re-authored pose is 206,615.545m. F4/highest-prominence starts 152.4m above
that surface and passes. Orbit passes. Baker tests (33 passed, 2 ignored), focused scenario
validation, and workspace check pass. The existing full-app collision-sweep failure remains
unrelated and pre-existing.

## Active summit calibration after coverage retune — 8 August 2026

The latest active outmap was rebaked after the mountain-coverage change. The retained
`relief_survey::tests::global_highest_summit` instrument now binds the checked-in F4 and
`highest_prominence_peak` calibration values to the bake instead of comparing constants only
against themselves. It measures a standard global-summit prominence of 178,134.275165m at
20.349651S, 51.995568W, direction `[0.577293926784803, -0.347748275323568,
0.738784717699863]`, with 44,531.544983m raw macro elevation at that summit. The global raw
L4 maximum is 44,531.671875m at a different sample.

F4/highest-prominence was re-authored to the measured runtime surface: the scenario starts
152.400m above the surface (runtime surface 178,125.699371m), with two screenshots. The
focused scenario passes finite-metric, no-thrash, clearance, and screenshot assertions.

## Broad lowland rolling hills — 8 August 2026

The procedural lowland field now adds one signed, low-frequency rolling-hill octave at
520m amplitude, gated by continental interior. It is added only to `lowland`; the mountain
region gate, narrow peaks, coverage ridges, and their amplitudes are unchanged. The interior
gate and a 1m floor prevent the new term from creating inland water holes near coasts.

The active 4096x2048, 256-erosion bake was regenerated with
`--procedural-terrain --mountain-coverage --erosion-iterations 256`. Its manifest SHA-256 is
`c969c79fc772249b8cfb29e53e69a5f73d1249d8b4c5dd2e04aa1abe303c18bc`; the prior active outmap
is preserved at `assets/outmaps/test-planet.pre-rolling-hills-backup-20260808-231451`.
The bake completed with 164/4,369,479 land samples passing the existing steep-mountain
coverage instrument (0.01% rounded), so this change targets broad green hills rather than
trying to inflate the major-mountain coverage metric.

The summit survey remains at the same direction, now measuring 180,943.291156m prominence
and 45,233.798981m raw macro elevation. F4/highest-prominence was re-authored to the new
runtime surface and passes at 152.400m clearance. Baker tests (33 library, 5 binary, 6
integration), workspace checking, and all 201 non-ignored app tests pass.

## Physical sky-view world-horizon correction — 13 August 2026

Manual run `test-runs/manual/1786627396-456527` held the camera at the same 169,071m datum
altitude while pitching upward. Its bright atmospheric band moved upward through the frame,
opposite the terrain and sun. Two coordinate errors caused that motion: the generated sky-view
LUT alone omitted the render-target V flip used by every other atmosphere data LUT, while the
fullscreen display alone negated screen Y instead of using the camera-ray convention shared by
the sun and ray renderer.

The remaining ocean-like line was the optical model's false ground horizon. Its deliberate 9:1
height compression treated the high-mountain camera as only about 18.8km above the optical
surface, so both the haze band and sunset timing followed the wrong horizon. Sky-view LUT rows
still retain the established optical scattering solution, but their view and solar zenith angles
are now monotonically remapped so the optical ground horizon lands on the actual world-space
horizon for the camera's true altitude. Rays that miss the real 1,440km world atmosphere shell
are rejected before optical integration. Terrain fog already samples this same sky-view LUT, so
it receives the corrected world-space alignment without a second approximation.

The exact reported position and two logged pitch directions were replayed temporarily through
`sunrise_midday_surface/1786628624-470427`: at the 169,071m datum altitude the true horizon is
about 16.4 degrees below level, and the corrected band moves down behind terrain and then out of
frame as the view pitches upward. The ordinary checked-in scenarios also pass unchanged:
`sunrise_midday_surface/1786628718-471743`, `sunset_blue_hour/1786628496-469465`, and
`night_side_atmosphere/1786628520-469653`. Focused LUT-coordinate and full physical-atmosphere
WGSL parse/validation tests pass. The one-off exact-pose scenario content was restored after the
capture; only its run artefact remains for comparison.

## Orbital atmosphere profile and physical sun-disc tint — 13 August 2026

Manual altitude sweep `test-runs/manual/1786640461-8499` showed a separate high-altitude defect:
from 38,248km down through the orbital views, the 4,000km planet sat inside an almost solid blue
shell whose outside radius matched the full 5,440km atmosphere radius. The surface correction
above was still right. The orbital artefact came from applying its compressed horizon mapping and
surface-only perceptual twilight lift to every camera altitude, making very thin upper-air
radiance occupy and visibly fill the full 1,440km gameplay shell.

Sky generation is therefore split only by camera altitude. Up to 200km it executes the signed-off
compressed optical path unchanged, which includes the 169km mountain replay. From 200-400km it
smoothly transitions to direct world-space integration around the 160km optical atmosphere; above
400km only that path runs. The fullscreen surface-twilight perceptual lift uses the identical
transition and is absent in orbit. The full 1,440km gameplay shell remains the near-surface
visibility/horizon bound, while orbital radiance presents as a thin atmospheric limb rather than
an opaque enclosing sphere. Outside the narrow transition, sample count and pass cost are
unchanged.

New fixed-exposure scenario `orbital_atmosphere_profile` looks at the planet from 16,000km datum
altitude and samples the former bright annulus. Its encoded luminance fell from 0.452 in the
reported presentation to 0.016 in `orbital_atmosphere_profile/1786642069-31422`, below the 0.080
regression ceiling. `ground_to_orbit/1786642071-31454` also passes and retains the established
surface views before resolving to the thinner space limb.

The camera-only sun disc no longer uses its separate authored orange tint. `SunRenderer` binds the
same physical transmittance LUT and sampler used by terrain and ocean direct sunlight, normalises
against the local zenith column to retain midday brightness, and preserves that wavelength-
dependent chromaticity as the low sun dims. An achromatic glare rolloff prevents the deliberately
overbright HDR core from clipping the physical red shift back to white. This changes only the
visual disc/corona; it does not feed exposure or alter sky, terrain, or ocean illumination.
`sunrise_midday_surface/1786642079-31545` passes with the final tint path. The older
`sunset_sweep` red/blue-growth assertion still fails on its off-axis sky sample under the already
committed physical atmosphere (`0.000` observed); the current changes are branch-identical below
200km at that sample and do not claim that pre-existing assertion as repaired.

## Orbital atmosphere angular-resolution correction — 13 August 2026

Manual approach sequence `test-runs/manual/1786642982-43527` exposed a second orbital-only
problem after the oversized-shell correction: as the planet grew between roughly 15,000km and
6,000km datum altitude, the thin atmosphere held one apparent radius, jumped outward, then
repeated. Camera FOV and scene time were fixed. The discontinuity came from sky-view LUT angular
quantisation, not LOD or atmosphere geometry. With the former linear-cosine vertical mapping, the
complete projected 160km atmosphere occupied only 0.118 of one 128-row LUT texel at 15,000km and
0.458 texel at 6,000km. Bilinear sampling therefore snapped between adjacent LUT rows as the
planet's angular size changed.

The LUT now applies a matched, analytic piecewise direction warp in all three consumers: sky-view
generation, fullscreen sky display, and terrain aerial/fog sampling. Above 400km, the atmosphere
tangent and solid-planet tangent map to V=0.72 and V=0.88, reserving 20.48 rows for the physical
atmosphere at every orbital distance. Between 200km and 400km those anchors blend continuously;
at and below 200km they reduce exactly to the previous linear-cosine mapping, so the signed-off
near-surface sky and mountain-horizon correction do not change. The physical integration,
160km optical height, sample count, LUT dimensions, atmosphere brightness, and post-processing
are unchanged.

New fixed-exposure scenario `orbital_atmosphere_continuity` moves directly from 15,000km to
6,000km in twelve half-second captures. Run `1786643763-54807` passes and shows the limb expanding
continuously with the planet rather than alternating between LUT rows. Regression runs also pass:
`orbital_atmosphere_profile/1786643906-55942`, `ground_to_orbit/1786643907-55971`,
`sunrise_midday_surface/1786643916-56057`, `sunset_blue_hour/1786643917-56086`,
`night_side_atmosphere/1786643929-56212`, and `limb_atmosphere/1786643931-56246`.
`cargo fmt --all -- --check` and `cargo check --workspace` pass. The full workspace test run reaches
204 passed and seven ignored but still reports the two unrelated pre-existing LOD-transition
timing failures in the already-dirty `terrain.rs`; neither failed test or implementation is part
of this atmosphere change.

## Visual sun limb and sunset presentation — 13 August 2026

The camera-only sun overlay was still perceived to shrink near the horizon. Its geometric
coverage was already the correct fixed 0.53-degree solar disc, but the old shader multiplied the
core, glare, and halo by a steep `pow(transmittance, 10)` visibility term. As the atmospheric
column reddened, that term erased the outer disc/halo first, making the sun look smaller rather
than merely dimmer.

The overlay now separates geometry from radiance. The core retains its constant angular
coverage and receives wavelength-dependent physical transmittance. The halo/glare uses its own
bounded, smoother visibility floor. At low positive solar elevation, a camera-only second optical
column plus a bounded red limb tint keeps the transmitted core from clipping back to white while
preserving the physical LUT-driven colour ordering. A 0.12 core visibility floor is only for
camera presentation; it does not feed terrain, ocean, sky lighting, or exposure. The extra work is
limited to a handful of arithmetic operations in the existing fullscreen sun pass.

Focused source regressions pin the separated core/glare paths and fixed angular coverage. Fresh
`sunset_blue_hour/1786655142-151491` passes all existing atmosphere assertions and shows the sun
remaining present and warm at the low-sun sample. The same build also passes
`sunrise_midday_surface/1786655279-154429`, `orbital_atmosphere_profile/1786655280-154453`,
`ground_to_orbit/1786655281-154481`, `night_side_atmosphere/1786655290-154569`, and
`limb_atmosphere/1786655292-154620`. No atmosphere LUT dimensions, sample counts, or
near-surface terrain lighting were changed.

## Optical atmosphere extension — 13 August 2026

Space captures showed the highest mountains silhouetted against black because the optical sky
profile ended at 160km while the currently presented summit reaches 180,943m. The gameplay
atmosphere shell itself is 1.44Mm, so this was a mismatch between the visual LUT mapping and the
world shell rather than a terrain or camera-height fault.

The optical profile is now 320km: the shared LUT mapping changes from 9:1 to 4.5:1, its edge
fade doubles to 213,333.334m, and the fullscreen sky, terrain sky/irradiance, and sun
transmittance paths use the same 320km/4.5 values. Rayleigh/Mie scale heights, the 1.44Mm
gameplay shell, near-surface lighting, LUT dimensions, and sample counts are unchanged. This
puts the presented summit inside the visible atmosphere without changing the signed-off ground
appearance. The focused `optical_atmosphere_covers_the_presented_mountain_summit` regression
guards the cross-shader contract.

Validation from the dedicated release build: `orbital_atmosphere_profile/1786656657-175207`,
`ground_to_orbit/1786656666-175299`, and the latest `sunrise_midday_surface`,
`sunset_blue_hour`, `night_side_atmosphere`, and `limb_atmosphere` runs pass. The full app suite
reports 206 passed, seven ignored, and the two pre-existing dirty-worktree LOD-transition timing
failures; `highest_prominence_peak` also remains blocked by its existing flat-mode clearance pose
rather than this atmosphere change.

## Solar-disc visibility floor — 14 August 2026

Manual set `1786661970-210103` confirmed the intended distinction: the corona and glare should
collapse as atmospheric intensity falls, but the physical solar disc must remain identifiable at
its fixed angular coverage until the planet geometrically occludes it. The previous camera-only
core floor was multiplied by a 0.20 low-sun radiance scale, leaving the final disc too dim at the
last above-horizon samples and making it read as a shrinking point.

The sun shader now keeps a bounded `SUN_CORE_RADIANCE_FLOOR` of 0.50 for the core only. Glare and
halo visibility still follow the atmospheric transmittance and can fade substantially. The floor
is camera presentation only; it does not affect sky, terrain, ocean lighting, or exposure, and the
existing depth-equal overlay continues to let the planet occlude the sun after sunset/before
sunrise. The sunset blue-hour regression remains passing, with the low-sun disc still visibly
red/orange rather than white.

## Horizon solar-disc hold — 14 August 2026

The next manual set `1786695385-425559` showed that the core floor fixed the disc’s visibility,
but the atmospheric transmittance continued decreasing for negative solar elevations. The disc
therefore changed again after the horizon sample: the glow collapsed in captures 5–6 and the
camera-only overlay could disappear before the intended occultation point.

`sun_disc_atmospheric_transmittance` now shifts the optical sample by 0.05 radians and clamps it
to the capture-4 endpoint (`-0.05` radians). That makes the red/dim stage arrive at the geometric
horizon, then holds it while the existing depth-equal overlay still removes the sun when the planet
actually covers its ray. This is camera-only; the physical sky, terrain/ocean lighting, and exposure
still use the unclamped atmosphere. Focused sun tests and the sunset, sunrise, night-side, and limb
scenarios pass.

## Horizon solar-disc LUT underflow repair — 14 August 2026

The timing correction in `f303f00` moved the visual sun's transmittance sample in the wrong
direction. Subtracting 0.05 from the solar-direction cosine selected the LUT's opaque horizon rows
roughly three degrees early. The existing 0.12 camera-only visibility floor then multiplied an RGB
value which had already underflowed to zero, so it could not keep any disc visible.

The camera-only overlay now samples the last useful physical red column by adding 0.05, presents
that column at the geometric horizon, and holds it below the horizon until the unchanged depth-equal
planet/terrain test occludes the disc. The core preserves the physical transmitted hue while the LUT
value is representable and has a limiting red hue only for half-float underflow. Atmosphere, terrain,
ocean lighting, exposure, angular sun size, and depth occultation are unchanged.

A new fixed-exposure `sun_horizon_visibility` scenario tracks the sun at 9,251.631m altitude through
5, 3, 1, 0, -1, -2, -3, and -3.5 degree samples; the sea-level geometric horizon there is about
-3.89 degrees. It compares the centred disc against adjacent sky rather than mistaking a bright sky
for a visible sun. Before the correction, centred contrast collapsed to about 0.004 once the LUT
underflowed. Run `1786702181-501440` passes all eight captures with minimum channel contrast 0.447
against the 0.050 floor and visibly retains the fixed-size disc. Regression runs also pass:
`sunset_blue_hour/1786702265-502173`, `sunrise_midday_surface/1786702283-502352`,
`night_side_atmosphere/1786702284-502395`, and `limb_atmosphere/1786702288-502506`.

## Interactive F4/F6/F10 startup — 14 August 2026

Ordinary interactive launches now finish initialization by calling the same toggle helpers as one
press each of F4, F6, and F10, in that order. The game therefore starts in summit low-flight mode
with blur enabled and scene animation frozen; WASD/mouse framing remains responsive under the
existing frozen-flight clock rule. Automated scenarios skip these startup toggles and retain their
authored camera, post-processing, and simulation time.

Debug Xvfb smoke run `manual/1786704906-526933` logs low-flight camera mode, blur enabled, and
animation frozen on startup, then keeps simulation time fixed across spatial samples. Control run
`still_5s/1786704929-527139` passes and contains no interactive-startup toggle event. Formatting,
`cargo check -p catinthegarden-app`, and the focused fullscreen/frozen-flight tests pass. The full app
suite remains at 207 passed and seven ignored with the two unrelated dirty-worktree LOD-transition
timing failures.

## Interactive planet-rotation speed controls — 14 August 2026

F1 now halves the interactive planet rotation time scale and F2 doubles it. Repeated presses clamp
the scale to 1/32x through 32x of `INTERACTIVE_PLANET_ROTATION_TIME_SCALE`. Changing rate preserves
the accumulated rotation phase, so it does not jump the planet, terrain, ocean, or sun; this also
works while the F10-frozen startup state is active, taking effect continuously when animation is
resumed. Scenario time scales remain authored and ignore these interactive controls.

The focused continuity/bounds test and existing world-space-sun rotation test pass. Debug scenario
`still_5s/1786705608-538095` passes unchanged, formatting and `cargo check` pass, and the full app
suite reports 208 passed and seven ignored with only the same two unrelated dirty-worktree
LOD-transition timing failures.

## Solid low-poly cloud systems — 14 August 2026

The rejected fullscreen procedural shells were replaced before commit by actual solid cloud
geometry. Two deterministic altitude bands contain independently moving cloud systems, each built
from overlapping, non-uniform 20-face icosahedron puffs. The puffs retain flat per-face normals and
write reversed-Z depth after raster terrain or the foveated-ray unwarp, so they are real opaque
scene objects; overlapping systems can visually merge and the shared depth buffer lets terrain,
clouds, and the camera-only sun occlude one another in the expected order.

Every system has a deterministic tangent wind direction and speed. The lower band moves at 24-60m/s
around 110-150km altitude, while the thinner upper band moves at 55-110m/s around 220-270km. GPU vertex
motion rotates whole puff groups around the planet on great-circle wind paths using simulation time,
so systems drift and cross without CPU uploads and F10 freezes them with the rest of the scene.

Cloud illumination has no authored day/sunset palette. Every flat face samples the same RGB physical
atmosphere transmittance LUT as terrain/ocean direct sunlight and the same generated
surface-irradiance LUT for diffuse sky fill. Daylight is therefore naturally near-white, the long
sunrise/sunset optical column dims and reddens the clouds, and the night side loses direct light.
Flat-mesh, deterministic-wind, physical-lighting-contract, and WGSL validation tests pass. Raster
captures pass at `sunset_blue_hour/1786712168-604722`,
`sunrise_midday_surface/1786712180-604830`,
`night_side_atmosphere/1786712180-604855`, and
`orbital_atmosphere_profile/1786712183-604898`; foveated-ray
`orbit_once/1786712184-604924` also passes.

## Brighter sky and complete solar-halo occultation — 14 August 2026

The camera-visible sky is presented at a fixed 2x radiance scale after the physical sky-view LUT
lookup. The gain is deliberately absent from atmosphere generation, surface/cloud irradiance,
extinction, and exposure, so this changes sky appearance without doubling world lighting or
altering the scattering solution.

The camera-only sun overlay now compares the complete physical solar disc against the solid
planet's angular silhouette. Existing reversed-Z depth equality still clips the disc and halo
fragment-by-fragment while the sun is partially occulted; once the final edge of the disc is behind
the planet, the entire overlay is discarded so the larger halo cannot remain visible around the
dark limb.

Focused atmosphere/sun tests and shader validation pass. Raster scenarios pass at
`sun_horizon_visibility/1786717296-655672`, `sunset_blue_hour/1786717304-655780`, and
`orbital_atmosphere_profile/1786717316-655893`; foveated-ray `orbit_once/1786717318-655919` also
passes. The full app suite reports 214 passed and seven ignored, with only the same two unrelated
dirty-worktree LOD-transition timing failures.

## Smaller, denser solid clouds — 14 August 2026

Both deterministic cloud bands now distribute twice as many distinct systems globally: 84 lower
and 48 upper, versus 42 and 24. Every system's lobe radii and internal offsets are scaled to 50%,
so the former sparse giant formations become smaller, more numerous formations without changing
their altitude ranges, wind speeds, flat geometry, depth behaviour, or physical atmosphere light.

To avoid paying twice for overlapping puff geometry, lower systems use 3-4 lobes and upper systems
2-3. This yields 419 drawn puff instances, below the former 449, despite doubling the formation
count. The focused deterministic size/density/lighting/WGSL tests pass. Raster captures pass at
`sunrise_midday_surface/1786729263-750971` and
`orbital_atmosphere_profile/1786729288-751249`; the latter visibly shows the finer global cloud
distribution. The foveated-ray path produced the expected finer-cloud capture, but its orbit run
later stalled in the current GPU/session state; an immediate A/B rerun of the unchanged committed
baseline stalled identically, while the new population contains fewer total puff instances. The
full app suite reports 215 passed and seven ignored with only the same two unrelated dirty-worktree
LOD-transition timing failures.

## Wider, altitude-varied drifting clouds — 15 August 2026

The cloud population doubles again to 168 lower and 96 upper independently distributed systems.
Each formation is 1.5x wider in both tangent axes than the preceding pass while retaining its
smaller vertical thickness. Lower centers now span 80-190km and upper centers 190-310km, making the
two solid layers visibly less uniform in altitude. Deterministic albedo brightness now ranges from
0.58-0.98, so some physically illuminated formations read as grey while others remain white; sun
and sky colour still come exclusively from the atmosphere LUTs.

Each system uses only one or two puffs, leaving 399 total instances versus the preceding 419 even
though the formation count doubled. Existing great-circle simulated wind remains active at 24-60m/s
in the lower population and 55-110m/s above, with independent tangent direction per system and F10
still freezing motion through scene time. A focused one-minute displacement regression now binds
the CPU wind data to the shader's simulation-time rotation path.

All five focused cloud tests and WGSL validation pass. Raster captures pass at
`sunrise_midday_surface/1786769717-1010100`, `sunset_blue_hour/1786769718-1010135`, and
`orbital_atmosphere_profile/1786769734-1010374`; the surface/orbit images show the wider, denser,
mixed-grey population. The full app suite reports 215 passed and seven ignored with only the same
two unrelated dirty-worktree LOD-transition timing failures.

## Altitude-aware direct sunlight and unfrozen cloud wind — 15 August 2026

Clouds and elevated terrain no longer inherit the transmittance LUT's sea-level solid-planet
occlusion. Their direct-sun path now performs one explicit geometric flat-horizon test at the
actual world altitude: the horizon cosine is derived from the 4,000km planet radius plus the
surface/cloud altitude, with only a solar-disc-width transition to avoid a visible pop. While that
test says the sun remains above the hypothetical horizon, the wavelength-dependent LUT is sampled
at its grazing column rather than its black below-horizon row. Terrain face-normal diffuse remains
unchanged; no terrain/cloud ray occlusion or authored sunset timing was added.

Cloud great-circle wind now uses presentation time rather than the F10-frozen scene clock. Clouds
therefore keep drifting during the default frozen-world startup while planet rotation, ocean phase,
and the other deliberately frozen simulation state remain stopped; deterministic scenarios still
use their authored clock. The focused altitude thresholds cover sea level and the 80km, 190km, and
310km cloud heights, and a camera-uniform regression proves wind time is independent from simulation
time. Debug raster `sunset_blue_hour/1786772828-1037962` passes all six colour/luminance captures and
visibly keeps the elevated clouds illuminated after the ground has darkened;
`sunrise_midday_surface/1786773053-1040143` also passes all four captures. The exact staged snapshot
passes 215 tests with seven ignored; its two failures are the pre-existing active-bake prominence
constant and collision-clearance expectations, while the broader dirty worktree passes 217 tests
and retains its two unrelated LOD-transition timing failures.

## Experimental weather branch removes the legacy cloud renderer - 20 August 2026

Branch `experimental-weather` intentionally removes the previous deterministic solid low-poly
cloud renderer and its WGSL shader, GPU pipeline, instance generation, render pass, and
presentation-time camera plumbing. Atmosphere LUTs, terrain lighting, the sun, and all existing
terrain/ocean rendering remain intact. No replacement weather simulation has been started yet.

The accompanying `planetary-weather-implementation.pdf` proposes a substantially better next
architecture: a coarse six-face cube-sphere field for synoptic state, CPU-first fixed-step
simulation, seam-safe world-space tangent velocity, and render-time local detail. Its strongest
advice is the separation of simulation scale from visual scale and the explicit conservation/debug
instrumentation. Before implementation, the proposed equal-angle warp, 600-second timestep/CFL
budget, pressure units, latent-heat feedback, and terrain-to-weather sampling contract should be
validated against this renderer rather than adopted as untested constants.

## Weather grid diagnostics foundation - 20 August 2026

The first replacement-weather slice is now in place on `experimental-weather`. A CPU-owned
six-face cube-sphere grid contains 64x64 cells per face (24,576 total), using the renderer's
existing seam-safe raw cube-face mapping. Each cell precomputes its world direction, orthonormal
east/north tangent basis, spherical surface area, and four cross-face neighbour indices. The
implementation deliberately does not introduce the proposed equal-angle warp yet: that mapping
must first be compared against renderer coordinates before it becomes a simulation contract.

The grid has deterministic tests for complete planetary area, tangent orthonormality, reciprocal
seam neighbours, cell count, and bounded latitude diagnostics. HUD key `7` toggles a six-face
latitude field diagnostic and reports cell-area range, tangent error, and a stable neighbour
fingerprint. Wind arrows are intentionally labelled idle until the momentum state exists; no
weather physics or rendering has been added in this slice. The next slice should add field storage
and conservation/CFL telemetry before transfer terms are implemented.

## Weather initial fields and diagnostics - 20 August 2026

The weather state now owns deterministic initial near-surface fields for temperature, pressure,
specific humidity, and tangent east/north wind on every grid cell. The initial field is deliberately
an inspectable hypothesis rather than a production climate model: temperature follows latitude,
pressure and humidity have small longitude structure, and wind is a smooth zonal/meridional test
pattern. No transport, heating, precipitation, or terrain coupling runs yet.

The HUD weather section now reports weighted field ranges/means, maximum wind speed, the proposed
600-second-step CFL estimate, and pressure/humidity area-integral conservation error. A six-face
overlay draws the initial tangent wind vectors over the latitude colour field, making direction and
seam placement visible before momentum exists. The transfer helper uses exponential relaxation,
`1-exp(-dt/tau)`, so later 600-second transfers cannot overshoot. Nine focused weather tests cover
field determinism/bounds, diagnostics/CFL/conservation, wind overlays, and relaxation behaviour.

## Weather fixed-step humidity transport - 20 August 2026

The weather state now has a fixed 600-second clock (`WeatherState::advance_to`). Render or scenario
time can arrive at any cadence; only complete weather steps are consumed, so partial frames cannot
change the field. Each step performs one conservative humidity mass-flux pass: local tangent wind
chooses the best of the four seam-safe neighbours, source mass is reduced, and the exact same mass
is added to the destination before converting back to area-normalised humidity. Humidity remains
bounded and the area integral is monitored against its baseline. Temperature, pressure, wind,
heating, precipitation, and terrain coupling remain unchanged.

The clock is exposed in the HUD as weather seconds and completed steps. Twelve focused tests now
cover fixed-step cadence, bounded humidity, area-integral conservation, cross-face transport, and
the existing topology/field diagnostics. Runtime wiring to the renderer's frame clock is kept as a
small follow-up because the checkout still contains the unrelated uncommitted render-loop
refactor; the transport API is ready for that integration without mixing the two changes.

## Weather render-clock integration - 20 August 2026

The render path now feeds its existing scene time into `WeatherState::advance_to` once per frame.
Interactive F10 freeze therefore holds weather exactly as it holds planet rotation and ocean phase;
a scenario's authored fixed time advances weather deterministically. Repeated timestamps and backward
time cannot double-step or rewind the weather clock. The existing dirty render-loop refactor is not
staged; the clean branch integration is a one-line call at the shared scene-time boundary.

## Weather humidity visualisation - 20 August 2026

The `7` overlay now colours each face by area-averaged specific humidity (brown/dry through blue/
saturated) while retaining white tangent-wind arrows. A text legend removes the previous ambiguity
about the latitude background. Humidity bins are built from the same 4x4-cell groups as the arrows,
so a completed 600-second transport step changes the displayed field without changing topology.
The HUD continues to report humidity range/mean and conservation drift. Deterministic tests verify
bounded bins and that transport produces a visible bin change.

## Weather pressure-gradient momentum - 20 August 2026

Each completed 600-second weather step now updates tangent east/north wind from the prescribed
surface-pressure gradients sampled through seam-safe neighbours, then advects humidity using the
updated wind. A 7,200-second exponential momentum damping and 60m/s speed cap keep this deliberately
small first closure stable; pressure remains static and there is no Coriolis, heating, or terrain
feedback yet. The existing white arrows therefore change after each weather step, while the HUD max
wind and CFL values expose the stability margin.

The multi-step regression runs ten steps deterministically, checks finite bounded wind, verifies CFL
stays below one, and retains humidity conservation within the current f32 field tolerance. This is
a tunable hypothesis, not a production pressure solver; the constants remain explicitly measured
telemetry targets for the next calibration pass.

## Experimental weather insolation and thermal response - 20 August 2026

The next weather checkpoint adds a fixed-step surface energy balance. Each cell now applies
short-wave insolation from the renderer's planet-local sun direction, albedo-weighted absorption,
Stefan-Boltzmann radiative cooling, and a bounded humidity greenhouse factor before pressure is
diagnosed and wind momentum is updated. A deterministic land/ocean surface proxy supplies five
times more heat capacity to ocean-like cells than land-like cells, so ocean temperatures respond
more slowly without importing renderer tile state into this CPU-first slice.

`WeatherState::advance_to_with_sun` is used by the render loop after the scenario sun and planet
rotation are resolved; the legacy `advance_to` helper remains deterministic for tests with a slow
day-phase fallback. Temperature stays bounded to 180-340K, pressure is explicitly diagnostic rather
than conserved, and humidity conservation remains unchanged. Fourteen focused weather tests pass,
including day/night ordering, bounded thermal response, and heat-capacity separation. Terrain
sampling, Coriolis, temperature advection, and moisture source/sink terms remain deferred to their
later milestones.

## Experimental weather Coriolis momentum - 20 August 2026

The pressure-gradient wind update now adds the projected Coriolis acceleration using the
renderer planet's Y spin axis and an Earth angular velocity of `7.2921159e-5 rad/s`. The
world-space acceleration is projected back into each cell's tangent plane before conversion to
the local east/north components, so cube-face seams and the poles do not introduce a radial
velocity component. The existing exponential damping, 60m/s cap, and CFL telemetry remain in
force.

The 15 focused weather tests include a zero-pressure-gradient regression that drives equal
eastward winds at matched northern/southern latitudes: the meridional deflection reverses across
the equator and vanishes at the equator itself. Workspace check passes. Temperature advection,
terrain slope/drag, moisture sources, and rendered weather fields remain deferred.

## Experimental weather manual stepping - 20 August 2026

Interactive debugging now exposes key `9` as a manual weather step. It executes exactly one
600-second simulation tick using the current planet-local sun direction, updates the HUD counters,
and works while F10 holds the normal scene clock. The production fixed-step clock and all scenario
timelines are unchanged; this is only a deterministic inspection control for temperature, pressure,
wind, humidity, and Coriolis changes without waiting ten real minutes.

## Experimental weather temperature advection - 20 August 2026

Each fixed weather step now semi-Lagrangian-advects temperature after the thermal and momentum
passes. Backtraces use the updated seam-safe tangent wind, and bilinear samples use fractional
cube-face coordinates with the existing cross-face neighbour mapping, so warm/cold tongues move
across seams without a face-aligned lookup discontinuity. The result is clamped to the established
180-340K safety bounds; pressure on the next tick is diagnosed from the advected field.

The deterministic weather suite now has 17 focused tests, including bounded temperature transport
and repeatability. Workspace check passes. Humidity remains on its conservative mass-flux path;
MacCormack sharpening, terrain drag, and rendered weather fields are still deferred.

## Experimental weather evaporation - 20 August 2026

The moisture stage now adds bounded evaporation before advection. Ocean-like cells act as a wet
surface source; land-like cells draw from a normalized ground-moisture reservoir initialized by
the same deterministic surface proxy used for heat capacity. Evaporation scales with wind speed,
temperature, and humidity deficit, uses exponential relaxation over 1,800 seconds, adds humidity,
depletes land moisture, and applies a small latent cooling term. Humidity and ground moisture stay
within `[0,1]`.

The fixed-step sequence is now thermal balance -> pressure/momentum -> evaporation -> temperature
advection -> humidity advection. Eighteen focused weather tests pass, including source bounds and
cooling. This is still a normalized diagnostic moisture field: precipitation, condensation, and
terrain-derived water are deferred.

## Experimental weather condensation/cloud field - 20 August 2026

Each fixed 600-second weather step now performs a local phase-change pass after temperature and
humidity transport. A bounded temperature-dependent saturation proxy converts supersaturated
normalized vapour into `cloud_water` with a 900-second exponential relaxation; undersaturated cells
re-evaporate existing cloud water. Vapour plus cloud water is conserved per cell by this stage, and
both fields remain bounded in `[0,1]`.

The HUD reports cloud-water range/mean, and the `7` six-face diagnostic overlay now whitens its
humidity colour according to area-averaged cloud water while retaining tangent wind arrows. This is
still a CPU/debug field only: precipitation, terrain-derived water, GPU upload, and rendered cloud
geometry remain deferred. Nineteen focused weather tests and a workspace check pass.

## Experimental weather lapse rate and orographic uplift - 20 August 2026

The fixed-step sequence now includes a bounded orographic pass after pressure/Coriolis wind
momentum. Each cell carries a deterministic seam-safe low-resolution surface-relief proxy until
terrain sampling is part of the weather contract. Tangent wind dotted with the local elevation
gradient gives vertical uplift; a 6.5 K/km lapse rate cools rising air and warms descending air.
The response uses a bounded fraction of the resolved vertical displacement so the one-layer proxy
cannot run away before a true vertical column model exists.

HUD diagnostics report proxy relief and maximum uplift, and the existing `7` overlay remains the
humidity/cloud/wind view. Uplift cooling feeds evaporation, temperature advection, and then
condensation, so rising cells can reach saturation without an authored cloud schedule. Twenty
focused weather tests and a workspace check pass; real baked terrain coupling and precipitation
remain deferred.

## Experimental weather precipitation and ground moisture - 20 August 2026

After condensation, each fixed 600-second step now removes a bounded fraction of cloud water above a
normalized 0.18 threshold using a 3,600-second relaxation. The removed cloud water is reported as a
rain-equivalent millimetres-per-hour diagnostic. Land cells route it into their bounded ground-
moisture reservoir; ocean cells treat it as an outlet until runoff and ocean coupling are defined.

The HUD now reports ground-moisture range/mean and precipitation range/mean. The sequence is
thermal balance -> pressure/wind -> lapse/orographic uplift -> evaporation -> temperature advection
-> humidity advection -> condensation -> precipitation. Twenty-one focused weather tests and a
workspace check pass. Snow partitioning, runoff/rivers, terrain-derived water, and rendered rain
remain deferred.

## Experimental weather latent heat and storm diagnostic - 20 August 2026

Condensation now releases a bounded latent-temperature tendency, while cloud re-evaporation applies
its equal-and-opposite cooling. The tendency is applied locally before precipitation, so the next
fixed step's pressure diagnosis can respond to the thermal anomaly without an unstable same-step
feedback loop.

Each cell also reports a normalized storm intensity built from cloud water, positive orographic
uplift, and latent-heating magnitude. It is a diagnostic signal only for now; no pressure vortex,
GPU cloud rendering, or storm visuals are introduced in this milestone. The HUD reports maximum and
area-mean storm intensity plus peak latent temperature change. Twenty-two focused weather tests and
a workspace check pass.

## Experimental weather GPU cloud shell - 21 August 2026

Milestone 11 uploads the weather cloud/storm/humidity field as six native 64x64 RGBA8 cube-face layers.
The renderer keeps two resident field textures and cross-fades the previous/current field over 1.5
seconds after each fixed weather tick. A camera-relative 14km shell renders the field with alpha
blending, shared atmospheric transmittance/irradiance lighting, altitude-aware geometric solar
visibility, and a very small humid-air precursor so a new run is not visually empty before the first
condensation tick. The shell is an additive scene pass after terrain/ray unwarp and before exposure;
terrain, ocean, atmosphere, and sun paths are unchanged. Two shader/mesh tests and the weather field
packing regression pass; interactive GPU capture remains the next manual sign-off.

## Experimental weather stacked cloud shells - 21 August 2026

Milestone 12 keeps the same weather field upload and temporal pair, but renders two instances of the
shell: a denser lower layer at 14km and a thinner upper layer at 90km. Both layers are depth-aware and
share the physical atmospheric lighting path. A deterministic tangent flow warp and trilinear hash
noise break up the cube-face bins without another texture or CPU pass; the upper layer uses a weaker,
greyer response so the two altitudes do not read as one opaque sheet. The cloud shader now validates
its instance, flow, and noise paths; all 25 focused weather tests, workspace check, and raster
`orbit_once` pass with four captures and zero seam violations.

## Experimental weather transport/noise correction - 21 August 2026

Condensed `cloud_water` now follows the same conservative seam-safe tangent-wind mass-flux
transport as humidity before condensation and precipitation. This removes the previous source-pinning
bug: humidity could move while the rendered cloud reservoir stayed in its original cells. A focused
regression seeds a cloud parcel, verifies movement into its wind target, and checks area-integral
conservation. The fixed-step exponential relaxation was already present and remains unchanged.

The cloud breakup shader now uses five rotated value-noise octaves, a 32-cycle base scale, and an
independent upper-shell domain offset. Noise modulates cloud coverage instead of subtracting directly
from density, avoiding regular circular holes. Cloud water remains the authoritative posterised
contribution; startup uses a separate continuous low-alpha veil so no coarse humidity bins are
turned into discs. Twenty-six
focused weather tests, workspace check, and raster `orbit_once` replay pass. Manual verification
should unfreeze weather or press `9` for exact 600s ticks before judging transport.

The GPU upload keeps the weather simulation's native 64x64 face samples rather than averaging each
face to 16x16. This prevents the humidity precursor from appearing as repeated circular patches at
orbital distance; the debug `7` overlay remains intentionally coarse.

## Experimental weather startup visibility - 21 August 2026

The initial weather field now seeds both temporal GPU textures with `blend = 1`, so the humidity
precursor is rendered on the first frame instead of fading up from an all-zero previous buffer.
Subsequent fixed-tick uploads retain the 1.5-second cross-fade. Focused weather tests, workspace
check, and a raster `orbit_once` replay now show the weather shell immediately.

## Experimental weather shared density source - 21 August 2026

The cloud field lookup, tangent flow warp, rotated five-octave FBM, coverage modulation, and
posterisation now live in `crates/app/src/weather_cloud_density.wgsl` as one `cloudDensity(dir, t)`
function. The weather shell shader is composed from its render code plus this shared source at
runtime; its fragment stage no longer reimplements density evaluation. `t` selects the lower/upper
shell contract (0/1), while the shared function consumes the including shader's weather field
bindings. Future terrain-shadow and impostor-spawn shaders must compose this same source and provide
that binding contract rather than copy the evaluation. The WGSL parser test validates the composed
source and the shared function; the two focused weather-render tests pass.

## Experimental weather cloud-shell depth repair - 21 August 2026

Manual orbital review showed regularly spaced circular gaps even after field filtering, proving
the pattern was not weather density. The 48x24 lower shell's planar triangle faces sagged 3,134m
below sea level despite their vertices being at 14km, so the planet depth buffer clipped one round
hole into every coarse mesh cell. The shell is now 96x48: its lowest face stays 9,705m above sea
level while retaining the authored 14km vertex altitude. A geometric regression measures the
minimum triangle-plane radius, and a constant-density raster replay shows a continuous shell. The
misdirected field blur was removed, returning cloud density to two texture fetches per fragment.

## Experimental weather elevated sunset lighting - 21 August 2026

Cloud direct light now samples the atmosphere transmittance LUT at the non-negative optical horizon
and applies solid-planet visibility separately at each shell's real altitude. This matches the
terrain lighting contract without inheriting the LUT's sea-level below-horizon occlusion: the 14km
layer remains sunlit after the ground enters shadow, and the 90km layer remains lit deeper into
twilight. Because the sampled transmittance is RGB, the surviving light naturally shifts through
yellow/orange/red rather than using an authored sunset tint. Sky irradiance still samples the true
signed solar angle for ambient fill.

The former constant low-alpha precursor covered both shells globally and could read as sea-level
fog. Startup visibility is now gated by local humidity and rotated FBM detail, leaving real gaps
while condensed cloud water remains authoritative. The shell altitudes remain 14km and 90km.
Twenty-eight focused weather tests, workspace check, raster sunset/blue-hour, orbit, and a
constant-density sunset-light visual diagnostic confirm the path. The diagnostic intentionally
obscures the sky and therefore is not expected to satisfy the normal sky-colour assertions.

## Experimental weather cubemap seam repair - 21 August 2026

Manual orbit capture `manual/1787341490-431395` exposed a straight bright boundary where two
weather cube faces met. The field was stored as six layers of a 2D array and sampled through a
manual face/UV lookup with clamp-to-edge filtering, so bilinear filtering could not cross the face
boundary. The same six layers are now exposed as a native WebGPU cubemap and sampled directly by
planet direction. Upload rows are vertically flipped to reconcile the weather grid's tangent V
axis with WebGPU cubemap orientation.

A focused upload-orientation regression and composed-WGSL checks pin the cubemap contract. A
high-density diagnostic orbit `orbit_once/1787349576-494773` exercises successive face boundaries
without the former hard line; the restored normal presentation replay
`orbit_once/1787349838-497054` passes all scenario assertions.

## Experimental weather terrain cloud shadows - 22 August 2026

Milestone 13 now projects both weather layers onto raster terrain. Each lit terrain fragment starts
at its real displaced surface position, marches toward the sun to the 14km and 90km shell
intersections, and evaluates the same field/flow/noise/posterisation source used by the visible
clouds. This keeps a low sun's shadow displaced in the correct direction instead of sampling the
cloud directly above the ground. Terrain above a shell correctly receives no shadow from that
lower layer.

The shared density evaluator accepts an octave budget: visible clouds retain all five FBM octaves,
while each shadow lookup uses three. The two layers combine conservatively and are quantised into
four hard coverage bands before attenuating direct sunlight by at most 72%; atmospheric sky fill is
unchanged, so a cloud shadow cannot turn daylight terrain black. Flat and smooth raster land share
the path, while ocean and the alternate foveated-ray renderer remain unchanged.

The focused weather suite passes all 29 tests, the relevant physical-sun and shared-density tests
pass, and workspace checking succeeds. The full 51-test terrain filter has 49 passing tests plus the
two pre-existing LOD-transition-duration expectation failures in the unrelated dirty terrain work.
Restored-source orbit `orbit_once/1787354268-533181` passes all assertions at a 16.143ms median
logged frame time, versus 16.680ms before this milestone. A temporary high-density, cloud-hidden
diagnostic at `highest_prominence_peak/1787354116-532072` makes the sun-projected hard shadow bands
visible across the ground; its existing stale summit-clearance assertion is unrelated to rendering.

## Experimental weather planet occlusion and doubled atmosphere - 22 August 2026

Cloud fragments now reject any camera-to-shell segment that intersects the solid 4,000km planet,
in addition to the normal terrain depth test. A clipped or deliberately coarse terrain facet can
therefore no longer expose weather on the opposite side of the world. Low-flight raster clearance
also measures the piecewise-planar triangle actually drawn at the camera radial rather than only
the continuous height field sampled at that direction; this prevents a fixed coarse-LOD facet from
passing above a nominally ground-level camera. The captured fault position now remains above the
drawn facet and the former far-side cloud pattern is absent.

The gameplay atmosphere shell is doubled from 1,440km to 2,880km and its compressed optical domain
from 320km to 640km. The 4.5 world-to-optical mapping and established 8km Rayleigh/1.2km Mie density
scale heights remain unchanged, preserving the signed-off near-surface atmosphere instead of
making the lower air twice as dense. LUT integration sample budgets double with the path length so
the extension does not introduce altitude-step darkening. A 1.5x ozone column scale strengthens
physical Chappuis-band absorption on long grazing paths, making the horizon redder without a timed
sunset colour. Raster sunset/blue-hour, horizon-sun, orbital-continuity, and ground-to-orbit
scenarios pass; the sunset sequence remains monotonic and the visible sun remains depth-occluded.

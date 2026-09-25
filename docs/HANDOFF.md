# Handoff — ocean wind sea spectrum

**Branch:** `experiment/ocean-wind-sea-spectrum`, tracking
`origin/experiment/ocean-wind-sea-spectrum`. This line used to pin a commit hash and was stale
almost every time it was read -- a header cannot name the commit that carries it -- so it does not
any more; `git log -1` is authoritative. The name is historical: the branch was
opened for the ocean, moved to **a second body**, and the ocean is the active subject again -- its
surface appearance rather than its geometry.

**Branch base:** the current ocean line; preserve all unrelated local renderer, terrain, baker,
documentation, and response-file changes when staging work.

**Current appearance diagnosis (23 September, latest manual grid complaint):**
Use `--scenario ocean_deck_reference` as the primary appearance comparison: a
2.5m wave-following eye, five degrees below the horizon, wide field of view,
facing toward the sun's horizontal projection. Clearance is asserted between
2.4m and 2.6m. The old 79m steep camera remains a secondary topology check, not
evidence that the near sea looks right. The user explicitly finds the water
viscous and the fine waves too regularly arranged.

`--scenario ocean_manual_grid` reconstructs the overhead pose of
`manual/1790167051-784385/screenshots/capture-001.png` from its spatial log and
centre probe: about 371m high, near-vertical view, fixed 0.048 sea state. It
starts at replay time zero, not the manual frame's 102.9s, so this reproduces
the framing and repeated pattern, not the exact instantaneous wave field.
At 1280x720 its FOV matches the logged manual FOV.

Controlled captures locate that pattern in the main short-wave shading:
baseline `ocean_manual_grid/1790182690-821467`; ripple-layer removal
`1790182760-821702` leaves it; zero mesh displacement with unchanged analytic
shading `1790182877-822007` leaves it; removing only the six 32.1–7m slope
contributions `1790183100-822672` removes the fine lattice but makes the sea
even smoother. A CPU/WGSL-paired re-aiming of those six bands
`1790183395-823162` merely rotates the pattern. Earlier quarter-wavelength,
quarter-amplitude shading ripples also failed the new low view. **All trials
are reverted.** No visual improvement, motion sign-off, or performance gain
is claimed; do not re-promote those experiments from their saved binaries.

Next: prototype genuinely less coherent, directionally richer short-wave
detail, not another axis rotation, missing-wave mask, or mesh-density increase.
Keep the 2.5m view and overhead reproduction as paired acceptance views and
measure cost before any promotion. This diagnosis does not establish that every
historically reported triangle/LOD artefact has the same cause. Preserve the
user's separate local `OCEAN_WAVE_SCALE=1.5` edit; the grid controls above use
it, whereas the earlier low-view ripple comparison used 1.0. Evidence, rejected
patches, and validation logs: `test-runs/ocean_sot_detail_2026-09-23/`.

**Opt-in compressed-wave cusp trial (23 September, based on `49a9994`):**
`CATINGARDEN_OCEAN_TRANSPORT=1` enables the forward surface map and inverse CPU
query in `ocean_transport.rs`; `shared_planet.wgsl` carries its matching horizontal
Jacobian and geometric normal. Raster vertices remain surface parameters. Foveated
ray sampling and the world-direction shading helper solve back to the parameter
direction with Newton steps. Height, slope, and fixed-world vertical velocity all
use the inverse query; default rendering/gameplay remain unchanged. The generated
`OCEAN_TRANSPORT_ENABLED` constant is false unless the process starts with that flag.

The bounded transport is confined to the 200m, 147.5m and 108.7m components. Its
derivative norm budget includes spherical curvature and the coastal reversal blend,
and stays below 0.95; strength fades smoothly to zero below 100m bed depth. The CPU
reference and derivative tests cover wind, reversed coast phases, a synthetic
0.995-compressed crest, and 1,440 real-planet query round trips. On Quadro, the actual
WGSL parity test under the opt-in checks 48 cases on its deliberately small 64m test
body: height error under 0.000006m and normal error under 0.03. This test does **not**
validate real-radius f32 phase precision. The standard 72 focused release tests pass
(12 ignored; skip the pre-existing dirty surface-camera walking failure); the opt-in
`ocean_rough_horizon` and `ocean_hybrid_close` scenarios pass their automation checks.

Visual/performance status is **not accepted**. The initial all-wave transport trial
`ocean_hybrid_close/1790150346-727596` versus same-build off
`1790150407-727887` turned the sea into large, blocky patches and measured 4.27-5.58ms
extra frame time in three interleaved Immediate-present pairs (`target-ocean-spectrum`).
After restricting compression to the three wind-sea bands, capture
`ocean_hybrid_close/1790150753-729240` still has scalloped, chunky faces instead of
clean interference cusps. Its performance has not been measured. It remains opt-in,
not the shipped default; do not spend judges or promote it. The 5m rough-horizon camera
also dips below water during this trial, so use `ocean_hybrid_close` for appearance.

Next: preserve the opt-in only if needed for further diagnostics, and diagnose why
mesh-scale/LOD structure still dominates the silhouette. The next appearance approach
must confine compression to constructive crossings without creating broad scallops,
then validate real-radius CPU/GPU/ray parity, movement, mesh/LOD edges, and matched
performance before any promotion. No improvement/score claim is made.

**Steep-view cusp follow-up (23 September):** added the fixed `ocean_steep_cusp`
replay (about 30 degrees down from 79m altitude) to reproduce the user's manual
water view: baseline capture `test-runs/ocean_steep_cusp/1790155819-748019`.
An all-ocean 40x40 mesh diagnostic retained regular raised ridges while increasing
ocean triangles from 582,912 to 897,600 (+54%); the temporary selector change was
reverted. The opt-in transport at this same angle made broad pale sheets rather than
cusped crossing waves (`1790156393-750771`); it remains off by default. No clean
performance comparison was made because another app process was live. The bounded
ocean transport and mesh-density ideas are therefore not visual solutions. Continue
with a physically coherent crossing-crest profile that is CPU/WGSL-consistent, then
validate actual movement, non-folding geometry and matched frame time against this
steep-view baseline; do not promote either discarded diagnostic.

**Bridge-front camera and restored dense-grid diagnostic (23 September):** the
interactive startup now attaches the eye 0.4m beyond the bow-facing bridge wall,
at bridge height, and updates it from the ship's current position/orientation each
frame; F4 detaches to low flight at the same eye pose. The previous 40x40
all-ocean grid selector is restored behind `CATINGARDEN_OCEAN_DENSE_GRID=1` so it
can be watched without imposing its measured ~54% ocean-triangle increase on
ordinary launches. Four-frame same-resolution steep-view replay passes with the
flag; capture sequence `test-runs/ocean_steep_cusp/1790160596-760717/screenshots/`
and animated preview `dense-grid-preview.gif`. It remains a topology diagnostic,
not a cusp fix, and no performance improvement is claimed. Normal ocean startup
is the bridge camera; non-ocean startup remains unchanged.

**Cusped interference crest investigation (23 September):** the user wants two gradually
steepening faces meeting at a near-vertical cusp, especially where waves interfere -- not merely
a sharper normal, triangle edge, or whitecap. Today's `u^3` radial profile has a horizontal
tangent at every crest. An opt-in semicubical-height trial on the 200/147.5/108.7m components
used a regularized `(1 - sin(phase))^(1/3)` deficit, keeping CPU height/slope/velocity and WGSL
geometry mirrored. Its 68 focused ocean tests and actual-WGSL CPU/GPU normal parity passed, but
same-build `ocean_rough_horizon` frame 4 controls `1790146788-713083` (off) and
`1790146678-712784` (on) show the large foreground still rounded and extra repeated ridges in
the distance. 328,836/921,600 pixels move by over four levels, so it was not inert; it simply
did not meet the shape criterion. **The source trial was reverted.** The promising geometric path
is horizontal Gerstner compression approaching a unit Jacobian at constructive crossings (the
classic trochoidal/semicubical crest), not another height-profile exponent or shading trick.
Horizontal transport is currently disabled because CPU buoyancy queries radial height and would
disagree by metres if the mesh slides sideways; implementing it requires an inverse CPU surface
query, ray/raster consistency, non-folding bounds, mesh LOD checks, and matched performance plus
motion captures. Do not turn it on with just a shader flag.

**Opt-in foam-history implementation (23 September):** `CATINGARDEN_OCEAN_FOAM_HISTORY=1`
enables `ocean_foam.rs`/`.wgsl`: a 128² ping-pong atlas over a 512m camera-tangent square. A
compute pass evaluates only the six shortest existing Gerstner components, makes spatially
scattered births at their convergent crests, reprojects the previous atlas from its planet-frame
basis, and decays it over 1.5 ocean seconds. The transmitting ocean shader samples that history
on sloping faces, still bounded by the existing water/depth foam rule; geometry and CPU buoyancy
are unchanged. The group-2 sampled-texture limit on the Quadro is 16, so the atlas uses the
previously shader-unused binding 15 instead of adding binding 16; the old shadow-height view
was not referenced by any WGSL entrypoint. The default compiles the history sample out and
skips the dispatch, and its `ocean_rough_horizon/1790142230-704492` frame 4 is byte-identical
to the prior `1790121789-675220` frame. Enabled frame 4 at
`ocean_rough_horizon/1790142246-704529` moves 97,264 of 921,600 pixels by over four levels;
it reads as faint pale patches rather than the reference's distinct foam streaks, so **do not
promote it yet**. Four interleaved 1280x720 Immediate-present pairs after two warm-ups give
enabled-minus-default +0.729/+0.262/+0.335/+0.221ms (median +0.299ms, about 0.6%); no FPS
improvement is claimed. `ocean_low_sun_stability/1790142092-704068` passes with the camera
moving; 67 focused ocean tests pass with the pre-existing dirty surface-camera failure skipped,
and actual-WGSL CPU/GPU broad-normal parity passes. The regular mesh-edge pattern is unchanged.

**Sea of Thieves technique investigation (23 September):** Rare's primary SIGGRAPH 2018 paper,
`https://history.siggraph.org/wp-content/uploads/2022/09/2018-Talks-Ang_The-Technical-Art-of-Sea-of-Thieves.pdf`,
states that the ocean is Tessendorf FFT, not Gerstner; deep/subsurface colour is blended with a
choppiness peak mask, view angle and sun direction; foam is generated at peaks and object
intersections, blurred with temporal feedback, then mixed with authored textures; and the low sun
gets an area specular lobe based on Karis's closest-point sphere approximation
(`https://cdn2.unrealengine.com/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf`).
Our Gerstner wave field, instantaneous analytic foam and static cubemap do not provide those latter
behaviours.
The first bounded experiment added a fifth, generated material-array layer as a foam breakup
texture and tested the existing whitecaps, sharpness and fine-crest masks against the exact-pose
`ocean_rough_horizon` baseline `1790121789-675220`. Captures `1790133466-690007`,
`1790133756-691071`, `1790133989-691714` and `1790134174-692369` respectively remove the
white patches, add broad false whitecaps, and turn them into static zebra-like streaks as the
texture scale tightens. None approaches the supplied `sot.png`; all source edits were reverted.
The 65 focused ocean tests passed with the pre-existing dirty `surface_camera.rs` failure skipped;
no matched frame-time claim was made. **Next:** a bounded camera-relative foam atlas, with
world-space reprojection/scrolling, a peak-birth mask, decay/advection and filtered history,
composited with a spatial texture. Measure its GPU cost against the current renderer and inspect
sequential captures for swimming/ghosting before promotion. Keep the sea geometry and CPU ship
buoyancy unchanged in this phase; FFT replacement is a separate CPU/GPU parity and performance
project. The regular raised mesh-edge pattern is also separate and unchanged.

**Current deck-height ocean reference pass (23 September):** against `/home/dad/Documents/sot.png`,
uniform wave scaling failed: 0.25x submerged a static 5m eye, 0.1x flattened the surface. The
promoted spectrum instead cuts the 1400m and 280-430m components to one tenth of their old
amplitudes in both CPU and WGSL while retaining the 200-7m wind sea and all 18 evaluations.
Sun-facing body colour is teal over dark blue troughs, and whitecap/transmission thresholds follow
the new spectrum. Matched `ocean_rough_horizon` frame 4: original
`1790121422-674391` versus candidate `1790121789-675220`; the latter has overlapping deck-scale
waves instead of a wall of blue water. Three interleaved, 1280x720 Immediate-present pairs after
warm-up measure 51.763→47.648ms pooled median (-7.95% frame time), each pair faster. 538 app tests
pass when three pre-existing failures from unrelated local `surface_camera.rs`, `atmosphere.rs` and
`main.rs` edits are excluded; actual-WGSL GPU normal parity and `ocean_rough_horizon`,
`ocean_hybrid_close`, `ocean_ship_float`, `ocean_wind_trial` replays pass. `ocean_clear_shallows`
fails its sediment-colour assertion in both the original and candidate binaries with the same
-0.004 margin under those unrelated local edits. **The regular raised mesh-edge pattern remains
unfixed**, and visual acceptance at the reference pose is still needed.

**Current beach join (11 September):** the dry-land/beach seam in manual capture
`1789119500-120010` is repaired by a continuous land-side sand tint. Water stays
on its existing footprint. See the latest section for matched capture evidence.

**Current underwater rendering (11 September):** visibility is now 100m, scaled
by local baked water depth. Raster ocean undersides reflect the pre-water scene
with a lit local-bed fallback for off-screen data, and the screen-space
reflection now inverts the snapshot's fog with the *reflected* point's own depth.
All of this is **raster only**: the raymarch path never calls
`ocean_underside_colour`, `terrain_fog` or any `ocean_water_fog*`, so a submerged
ray frame still has no underside, no water fog and no seabed. See the newest
sections; older 30m and flat-dark-underside descriptions below are historical.

**Current seafloor hole (12 September):** the deterministic missing-bed polygon
was a 0.499m f32 patch-anchor radial error putting near-camera triangles behind
the near plane. A per-instance correction restores full bed coverage without
changing source elevations or shared-edge projection. See the newest section;
manual swim-path acceptance is still pending.

**Current road experiment (13 September):** a single opt-in, terrain-conforming
surface section is visible in `road_surface_trial`, which now automatically
drives a surface-following POV along it. It does not yet grade,
cut/fill, clear trees, stitch dedicated geometry, or tunnel. The measured test
shader costs about 1ms on the Quadro; the default shader compiles it out. See
the newest section for captures and paired timing.

**Current shallow-water colour and refraction (13 September):** the turquoise a
bed puts into the water above it is weighted by an extinction falloff over the
instantaneous column, not merely by whether a bed resolved, on a measured
9m e-fold; the refracted bed is bilinearly filtered instead of point-sampled;
and crest transmission is re-anchored on the storm sea the game actually
renders. Ground cloud shadow is banded but no longer hard-posterized. See the
newest sections.

**Current render cost (20 September):** the frame is terrain. At ground level in `village_pov` at
720p, of 83.6ms, 30.5ms is a fixed floor with nothing drawn and 51.3ms is terrain -- 97% of the
scene work. Nine named terrain shader terms are priced and total 29.17ms of terrain's 52.1ms, so
about 23ms is still unattributed. Cloud shadow (4.08ms, and zero pixels changed under clear sky) is
**temporarily switched off** at Ian's request, and interactive startup no longer turns blur on. See
the newest sections; the cloud-shadow saving is not yet measured.

**Current ocean mesh-seam mitigation (21 September):** a grazing-angle-only 20% blend toward the interpolated vertex wave normal suppresses the false triangle-grid glints while retaining the analytic wave normal elsewhere. The deterministic `ocean_hybrid_close` capture lowers a foreground edge-energy proxy 11.83→10.81%; no extra samples or draws. A matched performance pair is still outstanding.

**Current steep-view ocean glitter (21 September):** exact-pose A/Bs proved the large rectangular light cells are the direct-sun specular lobe: disabling specular removes them, while disabling cubemap reflection does not. Narrowing the existing lobe exponent from 128 to 512 turns the blocks into separated highlights with no new samples or arithmetic category. The rejected crest-convergence whitecap gate was reverted because the user's new capture showed no visible improvement.

**Current small-wave detail (21 September):** an old fixed-water diagnostic was still zeroing the already-evaluated 180/70/28m normal-only ripple layer because its uniform lane was later repurposed for disabled horizontal transport. The broad geometric normal is now kept CPU-parity-safe while the retained ripple slope is applied once in fragment lighting. Exact-pose edge energy rises 2.28→2.47 (+8.4%) and the rounded small waves gain finer creases, with no new wave evaluations, samples, draws, geometry, or buoyancy change. Actual-WGSL CPU/GPU broad-normal parity still passes.

**Current fine-crest transmission (21 September):** the restored normal-only ripple octave selects positive, steep small crests and now has its own visible blue-green radiance instead of sharing the deliberately weak broad-crest tint. Across `ocean_hybrid_close/1790029744-444379`, only 1.22-1.60% of pixels move by more than four levels, with maximum RGB deltas 9/61/68. Three matched Immediate-present timing pairs have mixed signs and pooled medians 35.722→35.798ms (+0.21%, no established regression). Geometry, normals, depth, alpha, buoyancy, wave evaluations, texture samples and draw count are unchanged. User visual acceptance and the regular square/triangle raised-edge defect remain separate and unresolved.

**Current peak judging (21 September, round 19):** **2.0/10**, unchanged from round 17 despite eight
chosen compositions, a viewpoint with real rock and level framing -- so neither the site nor the
framing was what judges were marking down. Their first two complaints are shape, not shading: no
rock that reads as rock, and ridges rounded "like whipped cream" when real ones are toothed. One
judge named round 15's short-wave lift as "combed hair or ripples". Next real gain is anisotropic
terrain structure -- ridgelines, faces, strata -- not more shader work. See
`test-runs/peak_judging_2026-09-15/round19/RESULTS.md`.

**Current peak judging (21 September, round 17):** **2.0/10**, against 2.7 at the same viewpoint in
round 11. Three independent judges named: no rock, the tint's "leopard spots", a repeating fish-scale
patch, grey-never-blue shadows, violet distant peaks with a pink strip, and -- the new one -- the
foreground as "combed parallel ripples", which is round 15's short-wave lift showing its grain. The
largest gap is the viewpoint: the camera sits on 93.8% ice while every reference photograph is half
rock. See `test-runs/peak_judging_2026-09-15/round17/RESULTS.md`.

**Current peak judging (21 September, round 16):** crevasses at icefall scale now read on the steep
ice, and cost 3.89ms after dropping a second noise lookup for the ladder relief already at the
fragment (was 6.15ms). Still off by default -- 5% of the frame for a partial-frame feature. See
`test-runs/peak_judging_2026-09-15/round16/RESULTS.md`.

**Current peak judging (20 September, round 15):** the smeared, waxy ground is fixed and the fix is
promoted. The detail ladder was exactly self-similar, so its 4m octave carried 23cm and its 1m
octave 6cm -- relief that rolls rather than relief that is rough. A short-wavelength lift in the
spectral tilt takes tonal spread 0.499 to 0.586 against the photographs' 0.898, at 78.60ms against
80.45. The gain is derived from where a spectral bump would appear, not chosen by eye. See
`test-runs/peak_judging_2026-09-15/round15/RESULTS.md`.

**Current peak judging (20 September, round 14):** crevasse shading exists and is **off by
default** (`CATINGARDEN_CREVASSES=1`); the shipped picture is unchanged to the pixel. It occludes
the beam rather than painting a line, so it inverts with the sun, but it reads as hairlines at the
judging camera and costs 2.75ms. The blocker is scale, not technique: one pixel is 2.2m of ground at
1.5km, the near glacier is a flat basin, and the nearest baked crevasse texel is 18km away.
Structure at this camera has to be icefall-scale. See
`test-runs/peak_judging_2026-09-15/round14/RESULTS.md`.

**Current peak judging (20 September, round 13):** measured, not judged. Ground saturation and
tonal spread are **0.017 / 0.499** against the Aletsch photographs' **0.176 / 0.898**, and the cause
is that there is effectively **no fill light** -- skylight is 0.3-0.6% of surface light, which is
physically correct at a site sitting 2.2 optical scale heights up, and snow inter-reflection is not
modelled at all. A bounce term, un-greying snow shading and an exposure sweep were each measured and
none promoted. The smear and the dapple belong to the detail ladder and the material tint, the two
most expensive terms in the shader. Next change should be structural. See
`test-runs/peak_judging_2026-09-15/round13/RESULTS.md`.

**Current peak visual judging (15 September):** `peak_survey_8_directions` (raster) hovers
100m over the highest summit at solar noon and captures eight headings 30 degrees down. Three
independent judges scored it against midday summit photos at 2.3/10, then 2.5/10 after this work:
snow sheds from slopes of 30-45 degrees with rock beneath snow biomes, sky fill uses the
`(1 + n.up)/2` view factor, and raster casts ridge-scale terrain shadows by marching the ray path's
~3km height faces (shared binding 15). Each measured small at a 64-degree sun. Still judged:
stretched cliff texture, faceted/sawtooth ridges, and a salmon speckle on lake/coast edges; the orange pixel-edged lowland patches are fixed. The
scenario lowers its camera 227.35m because `ACTIVE_HIGHEST_PROMINENCE_DRAWN_SURFACE_METERS` is
stale. See the newest section.

**Current planet (16 September):** rebaked with a slope-aware biome classifier. Steep ground now
classifies as MountainRock before the ice and snow height tests can take it, because this planet's
relief runs to tens of kilometres and every high place was becoming Ice. By area, mountain rock went
2.56% -> 12.46% of the surface and ice 19.56% -> 10.34%; ocean, lake and the temperate biomes are
untouched. Heights are unchanged, so the summit, the F4 pose and every constant derived from the
bake are bit-identical and needed no re-derivation. The previous planet is preserved at
`assets/outmaps/test-planet.pre-steep-rock-backup-20260916-2216`. See the newest section.

**Current glacier export (19 September):** Claude's third glacier bake had moraine/crevasse
labels in its working grid, but tile refinement overwrote them at L3+. The exporter now
preserves both labels, with a failing-before tile regression. The corrected procedural bake
is validated and installed at `assets/outmaps/test-planet`; Claude's third bake is preserved
at `assets/outmaps/test-planet.claude-third-backup-20260919`. Heights, moisture, previews and
manifest are identical. Captures show distant tonal patches, not convincing glacier structure,
so no new judging round or realism gain is claimed. See the newest section for measurements.

**Current renderer cost (16 September):** the summit survey runs at 41.5ms median, down from
77.2ms, after removing the per-pixel cast-shadow march and the per-fragment biome-edge noise. Both
were judged down as well as expensive. Shadows are wanted back cheaply (cached/amortised or a baked
horizon map); the height-faces plumbing is left in place for it. See the newest section.
These historical timing claims were superseded on 19 September: rebuilding the old code did not
reproduce the 41.5ms baseline. See `test-runs/peak_judging_2026-09-15/PERFORMANCE.md` and the latest
matched measurements below; do not treat the old number as a current regression target.

**Current input and world tuning (16 September):** a held movement key survives a remote
desktop's key repeat (Sunshine/Moonlight sends press/release pairs, which used to cancel WASD while
F-keys worked), one planet rotation is now 80 real minutes rather than 20, and birds are drawn at
2.1m rather than 0.42m with every constant that seats them on the ground scaled to match. See the
newest section.

**Current birds (14 September):** flocking birds stream in around the camera,
cruise, land, walk and take off again, and over the sea they meet the real
moving surface: they climb ahead of rising water, cruise over a swell envelope,
and raft on the water lying along the wave. Simulation in `birds`, GPU in
`birds_render`, drawn beside the ship in both render paths. **B** rides a bird
in the nearest airborne flock and **B** again gives the eye back; **N** goes to
the nearest flock that is down on the ground or water and stands just outside
the range that would put it up. While riding, every player-camera writer stands
down (`player_camera_is_suppressed`) -- not the physics, only the camera write --
because two of them run after the bird cam by design and were shattering the
terrain into plates. The spawner used to hang the game, core pinned, whenever
the eye was far above where flocks spawn; fixed 14 September, see the latest
section. **B** was verified by counts and by eye in `bird_demo/1789327323-459268`;
**N**, and birds sitting on the water, have not been seen in the game yet.

**Forest beams removed (13 September):** the opt-in forest beam overlay and its
whole supporting path are gone at the user's request, and **B** now belongs to
the bird cam. Sections below describing beams, `CATINGARDEN_FOREST_BEAMS` and
global forest locators are historical.

**Current sea-state variety (14 September):** normal launches now follow the
camera region's filtered weather wind/storm field and estimated upwind fetch
through a slow, continuous sea response, replacing the authored cycle. Explicit storm/wind startup overrides
and legacy replay endpoints remain fixed; `CATINGARDEN_OCEAN_STORM=cycle`
restores the old demonstration loop. This is a scalar storm-energy response,
with a bounded fetch estimate, not a directional spectrum solver. See the latest
section for validation.

**Current ocean default (10 September):** spawn-coast shoreward waves are now enabled
for normal launches at the user's request. `CATINGARDEN_SPAWN_COAST_WAVES=0`
opts out. Earlier opt-in-only notes below are historical. Coverage remains local
and the measured ~5ms cost is unchanged; global steering is still outstanding.

**Wind experiment (13 September):** `CATINGARDEN_OCEAN_WIND=speed,x,y,z`
now controls a fixed startup wind-sea spectrum with CPU/GPU parity. Normal
launches are unchanged. This is not live weather coupling or the completed
Sea of Thieves-style ocean; see the latest wind-sea section.

**Written:** 6 September 2026; header current to 13 September 2026.

**How to read this file:** everything below this header is an append-only log of dated sections,
oldest first. This header is the current state; **the newest work is the last section in the file,
not the first.** Read `AGENTS.md` for the architecture, and this for where the work is.

**Supersedes:** `PLANET_SIM_HANDOFF.md` at the repo root, which describes the 19 July low-flight
state and is now history.

**Two bodies.** `body.rs` holds what distinguishes a world — radius, rotation, whether it has an
ocean or an atmosphere, its material tints, and how much its baked height is exaggerated. One is
active per process, chosen by `--body planet|moon` before any pipeline exists, because the radius
reaches the shaders as a *generated constant* rather than a uniform. That is what makes CPU/GPU
divergence impossible by construction, and it is also the reason both cannot be drawn at once yet:
rendering them together needs the radius to become a uniform, and that is the next step, not this
one. Every branch that distinguishes the two is a generated `const bool`, so the planet's compiled
shader is unchanged — asserted by rendering, not by argument, at max pixel difference 0.

The moon is baked like the planet, into `assets/outmaps/test-moon`, from a 600,176-crater catalogue
with no size floor (324km down to sub-cell), ice placed by permanent shadow rather than latitude, and
pink ice lit by planetshine. Six rebakes so far; each one moves the baked landing site and every moon
scenario pose with it. That is the moon's state as of this header; the sections below are its history.
in `coretypes::moon`. It has no ocean, no air, no weather and no vegetation, and its two materials
are regolith and polar ice. Ice is *terrain*, which is the whole trick: it is drawn by the terrain
pass and walked on with no special case anywhere. See the last two sections for the four planet-only
rules that had to be gated off it, and for why baking was the right call after synthesising first.

**Sea state.** The ocean is what this branch was opened for. `WAVES` (`ocean.rs:238`) and its
`OCEAN_WAVE_TABLE` mirror in `shared_planet.wgsl` hold seventeen components: two 1,400m swells, a
280-430m storm sea, and a twelve-component wind-sea tail spread widely in azimuth, which is what
breaks the crests up. Each entry carries a calm and a full-storm amplitude, and both columns sum to
the same 0.9575m, so a storm moves the dominant band down from the 1,400m swell rather than scaling
the calm sea up. That gives a 42.130m calm cap (x44) and a 52.663m storm cap (x55), so the 53m bound
holds at either end and everywhere between. A test mirrors every axis, wavelength, amplitude and
speed literal between the CPU table and the shader, because a GPU-only edit would leave collision
following water the renderer had stopped drawing.

`OCEAN_WAVE_SCALE` (`ocean.rs:15`, currently 1.0) is the only place the sea's size is written down.
The calm and storm amplitude scales, `MAXIMUM_WAVE_HEIGHT_METERS` and the steepness all derive from
it; `ocean::wgsl_constants` generates the shader's copies and `planet::shared_planet_shader_source`
prepends them, which both the raster and raymarch assemblers go through, so the two render paths
cannot disagree. Steepness is `1 / OCEAN_WAVE_SCALE`, holding the Gerstner self-intersection budget
invariant at **2.0507** against a physical limit of 1.0 — the sea already folds; the knob holds it
there rather than letting it grow. (This line read 1.17 until 8 September: the table was retuned and
the prose was not. `ocean::fold_budget()` is authoritative and prints the number above; anything
reasoning from 1.17 is reasoning from a number that stopped being true some retunes ago.) Its real ceiling is the camera's -100m underwater floor, around 1.8.
Two guards run before the shader mirroring, so an out-of-range knob is reported before anything is
edited.

**Shore.** Waves shoal rather than fade out. `breaking_weight` squeezes the summed crest toward the
depth limit with a soft-max knee, so it flattens off as it shallows and can never cut through the
bed — the limit reaches zero exactly where the water does. `BREAKING_HEIGHT_TO_DEPTH_RATIO` is 0.78
(`ocean.rs:485`); it is a wave *height* to depth ratio, so the crest amplitude limit is half that,
0.39 · depth. The depth-only `shoaling_phase_offset_meters` was disabled on 10 September
because it generated closed concentric wave fronts. It now returns zero: waves retain their
authored global travel directions, which can point offshore. Earlier shoreward correlation
numbers below describe the removed model, not the current renderer. Shoreward travel without
those rings remains outstanding; reversing the global phase sign only swaps the affected coasts.
Foam keys off `breaking_ratio` and fades once a crest is well past breaking.

**Camera.** Three interactive modes. `G` toggles a 1.70m human-eye surface camera out of the F4 low
flight camera and back; F4 returns either close mode to the saved orbit pose. Surface vertical
motion is a fixed-substep gravity simulation rather than a surface clamp: `Space` jumps 5.2m/s on
land or 2.5m/s submerged, uphill movement is rejected above 42 degrees, and descent and water entry
stay allowed. In water, buoyancy drives the motion but a swimmer who is not diving cannot end a
substep below the surface — this sea's crests accelerate downward at close to g and overtake a
floating body, leaving it submerged 41% of a storm, and twenty times the restoring force only
reached 23%. So bobbing has a floor rather than a stiffer spring.

**You can dive.** A swimmer's forward axis is the whole look vector, so pitching below the
horizontal descends; a walker's is still flattened onto the surface. Three things had to give way
and one had to be added, and the last section in this file is why. The crest floor above is off for
a diver and on for everyone else, and *only a commanded dive* sets that state — latching it on being
below the waterline would let a crest turn the guard off, which is the exact bug the guard exists
for. The whole floating model — the restoring spring, the wave-following drag and the buoyancy
itself — fades out over `NEUTRAL_BUOYANCY_DEPTH_METERS` (0.3m) below the waterline, because all
three are surface devices. Below that the diver is **neutrally buoyant and holds the depth they
stopped at**; swimming back up is how
you surface, and it is the only way. A dive stops 0.5m off the bathymetry at **any** depth: the
-100m `PLANET_CORE_CLEARANCE_METERS` is a backstop for a runaway with no bed to catch it, and no
longer applies over water. That floor has one definition, `swimming_bed_eye_altitude_meters` —
`resolve_surface_camera_after_streaming` holds it again after every tile lands, and when it had its
own arithmetic it used the 1.70m walking eye height and silently overrode the dive floor.
Interactive
startup enters swimming mode at 30.246944N, 14.474559W, roughly 50km seaward of the authored coast.

**Clocks.** `INTERACTIVE_DAY_REAL_SECONDS` is 1200 (`main.rs:83`) and the rotation scale derives
from it. Weather takes its clock from the rotation — `WEATHER_DAYS_PER_PLANET_ROTATION` is 1.0
(`weather.rs:26`), giving 72x real time — with tests asserting one rotation advances the weather
exactly one day and that a day divides into whole steps. Comma and period step a nine-rung time
ladder from 10% to 4000% of real time; everything scene-side reads one accumulated
`scaled_clock_seconds`, so rotation, ocean, weather and hull speed up together and the derived
relationships hold at every rung. Weather, hull and surface camera all integrate whole fixed steps
and carry their remainders, so none of them depends on frame rate any more.

**Weather.** Temperature advection was draining the planet: 24.00K lost over four weather-days by a
semi-Lagrangian scheme with a clamped MacCormack corrector, which conserves nothing, while advection
should not change the mean at all. Donor-cell transport made it far worse (105K) because it moves
`value * area`, which suits a mass fraction and ruins an intensive quantity. Temperature now trades
a bounded share of its *difference* with the cell downwind, so what leaves one enters the other
exactly: 0.00K over 3000 steps, and the field holds 264.8-267.6K across 20.8 weather-days instead of
collapsing onto the 180K clamp floor. Moisture oscillates 0.59-0.69 with no trend.

**Ship.** `ship.rs` owns the hull form, an eighty-column buoyancy discretisation and the rigid body,
GPU-free and tested like `surface_camera`; `ship_render.rs` owns the pass. Mass comes from the same
columns that provide buoyancy, so the design waterline is an exact equilibrium rather than a tuned
guess. Buoyancy acts normal to the sloped water surface via `ocean::global_wave_slope`, checked
against a centred finite difference — a radial force has no moment about the vertical axis, so
before this nothing in the model could yaw the hull. Metacentric height is set directly at 0.9m
rather than falling out of where the mass sits, with eddy damping a vertical-prism model otherwise
has none of.

**Flags that decide what the camera stands on.** All are paired CPU/GPU and enforced by tests.
Flipping one half silently is the recurring failure mode on this branch.

- `WATER_BOBBING_ENABLED` (`surface_camera.rs:17`) — `true`.
- `OCEAN_HORIZONTAL_TRANSPORT_ENABLED` (`ocean.rs:118`) — `false`, and must stay false while the CPU
  height query has no horizontal term. Transport slides the rendered mesh up to 54m sideways.
- `OCEAN_RIPPLES_ARE_GEOMETRIC` (`ocean.rs:131`) — `false`. The 180/70/28m ripples are a shading
  detail; `vs_ocean` does not displace by them. A test reads `vs_ocean` and fails if the flag and
  the shader disagree. The CPU query once added them anyway, floating the camera on up to 4.6m of
  water nobody drew.
- `OCEAN_LARGE_SWELL_ONLY` (`ocean.rs:185`) — `false`. Keeps only the leading swell pair and
  silences the wind sea and the whole ripple layer when true.
- `OCEAN_WAVE_PHASE_SPEED_SIGN` (`ocean.rs:142`) — `1.0`. Phase is `wave_number * (dot(direction, axis) * R + sign * speed * time)`, so a crest travels against it.

**Controls** (the sections below carry the rest): F4 low flight and saved orbit pose; F6; F10
freezes planet rotation, sun and composition but *not* the ocean or weather clocks, which run on
presentation time; `G` surface mode; `WASD` and mouse look, planet-relative; `[` and `]` scale the
4.4704m/s walk and 2.0m/s swim; `Space` jump; `6` auto exposure versus fixed 1.0; `7` weather
diagnostics with 4hPa isobar contours and labelled H/L extrema; comma and period for the time
ladder.

**Latest evidence.** `ocean_hybrid_close` is the calm control; `ocean_rough_horizon` the storm
endpoint, passing its 30m gate at 39.352m with foreground crests occluding waves behind them;
`ocean_low_sun_stability` holds top-quarter sky luminance to 0.52% on a vertical move and 0.37%
lateral; `ocean_waterline_flat` is the magnified waterline instrument. The ocean has **no seam
instrument**: `max_seam_delta_m` compares baked outmap tile heights across chunk boundaries and
cannot see a gap in analytically displaced ocean vertices, so that class of defect only shows up in
a magnified waterline capture. Scenarios initialise deterministic weather rather than serialising an
evolved manual state, so fresh manual travel through the real weather remains the visual acceptance
gate for anything weather-composed.

**CI.** Green: **487 workspace tests** (14 ignored), measured 7 September after the foam, sea-colour and crest-onset work; `cargo fmt --all --check` clean, clippy clean across
all three crates. Note that `coretypes` now carries tests of its own — it used to be types only. Clippy stops at the first crate that fails, so the baker's three
had been hiding the app's seventy entirely — the app was never being linted. Check both.

**Also closed: thread 27, the underwater scenario's missing visual guard.** It now samples the
refracted warm horizon at UV (0.05,0.95), rejects the old unrefracted blue result and black/white
output, and has an actual-WGSL Snell/Fresnel test for normal, tilted, critical and internally
reflected rays. This replaces the former screenshot-count-only acceptance. See the 8 September
underside optics entry; it does not claim full-scene refraction or temporal visual sign-off.

**Recently closed, so they are not re-opened:** the "renderer draws near-field land up to 43.58m
above the CPU's height field" thread was the surface probe reading tree canopies, and
`detail_correlation` 0.3222 was a canopy height set against a ground height; with the probe reading
depth before the forest pass, `coast_waters_edge` raster is median 0.971m / p90 3.6m / max 8.490m at
correlation 0.72 over 70 points a frame. **Those are not the numbers this paragraph used to quote**,
and the difference is not a regression: the 0.560m / 1.835m / 2.277m / 0.9586 figures further down
this file were measured over **36** compared points, and the scenario now compares 70. Every run on
disk back to `e6c21eb` reports the 70-point figures, so whatever admitted the extra points predates
all of them and was never recorded. Treat the 36-point table as the evidence for the *canopy* fix it
was taken for, and not as this scenario's current state. What is still true is the thing that fix
established: the CPU and GPU detail fields correlate rather than being unrelated. `highest_prominence_peak`, `stand_on_ground`, `landing_site_eye_level` and
`landing_site_ground_detail` were four cameras standing 687-753m underground or 7,659m over the
wrong mountain, on poses authored before a rebake; all four pass and compare points again. See the
last two sections.

**Also closed.** The moon rendered black across most of its sunlit side, which was neither lighting
nor exposure: `is_open_ocean_surface` was discarding every fragment at or below the datum for an
ocean shell that does not run on that body. And the moon was synthesised per sample rather than
baked, which capped it at a few hundred craters; it is baked now, at 600,176. Both in the last two
sections.

The near-field ocean-culling defect is fixed at `4f2da78`: `may_contain_ocean`
tested a near-field chunk's water content with *window* UVs against a single guttered source tile, so
it could prove an unrelated patch was land and cull the ocean covering the real one. It now tests the
uploaded window's own unguttered grid. Verified independently on this branch, not taken on trust:
`ocean_ship_float` draws **40 ocean chunks before and 255 after** (92,160 to 587,520 triangles) with
`drawn_chunks` unchanged at 256, the constant slab colour `(15, 64, 117)` falls from 150,901 pixels to
zero in the lower half, and the top 200 sky rows are byte-identical. Land culling still works --
`mountain_ground` 1 ocean chunk, `stand_on_ground` 11, `highest_prominence_peak` 15, `coast_waters_edge`
157, open sea 255. 430 workspace tests, clippy and fmt clean. **The earlier "the streamer loads
nothing" thread is closed too**: only the `pz` face is baked past L4, so there is nothing to load
anywhere else.

**Also closed: `mountain_ground`'s 11.4m discrepancy.** The raster probe passed surface-hit
distance into a detail filter whose shader measures distance to the undisplaced sphere. Correcting
the comparison reduces median error from 11.433m to 0.052m (maximum 12.660m to 0.312m), with a
byte-identical screenshot. The scenario now requires 81 comparisons and a 2m maximum error.
See the latest section for the failing-before/passing-after evidence. No terrain or collision changed.

**Known failures and open threads,** in the order worth picking up:

1. **Trees bob against the ground as the camera moves.** Diagnosed and measured, not fixed; the
   plan and the obstacle are in the 7 September section "trees are planted on one surface and the
   ground is drawn on another". A tree's ground height is computed on the CPU at *full* detail
   (`outmap_surface_height_meters` pins the filter to `TERRAIN_DETAIL_MIN_FILTER_METERS`, 0.5m) and
   baked into the instance as `surface_height: f32` (`forest.wgsl:100`), where it stays until the
   patch rebuilds. The ground under it is drawn at `max(distance * 0.01, 0.5)`, refiltered every
   frame. Same class as the collision bug `c99787a` fixed for the camera: a consumer of terrain
   height using a different detail filter than the shader.
2. **The survey and the app disagree by 227.353m about the summit's height.** `global_highest_summit`
   reports 186,709.142m at `ACTIVE_HIGHEST_PROMINENCE_DIRECTION`; the app's surface query reports
   186,936.495m there, stable across altitude and converging to exactly `raw_macro * 4` =
   186,941.266m by 36km up. So the app applies almost no detail where the survey applies -232.1m.
   Suspect the ladder's high cut, `baked_spacing_meters`, against different resolved source levels --
   the survey scans L4 only. **Unmeasured**: the probe compares zero points at that pose. Until it is
   settled, `highest_prominence_peak`'s pose is derived from
   `ACTIVE_HIGHEST_PROMINENCE_DRAWN_SURFACE_METERS`, because that is what the clearance assertion
   measures; deriving it from the summit puts the camera 75m inside the mountain.
3. **The probe is meaningless over water and nothing says so.** All 45 points in
   `ocean_hybrid_close` and all 9 in `ocean_rough_horizon` have `cpu_height_meters` of exactly 0.0,
   so their reported deltas are rendered wave height, not a surface disagreement. No scenario arms
   `max_surface_probe_delta_m`; if one were armed on a water scenario it would fail on wave height
   alone.
4. **A scenario can assert clearance with no probe-point floor, and be buried and green.** The
   harness refuses a delta tolerance without `min_surface_probe_points` (`scenario.rs:740`) but has
   no such rule for a clearance assertion, which is how `landing_site_ground_detail` sat 687m inside
   a mountain and passed. Floors are now set on the four ground scenarios by hand. Whether to make
   that structural is a judgement call: several clearance scenarios are legitimately too far from
   ground to compare anything.
5. **`descent_to_10m` fails on the terrain streamer**, and did so before any of this branch's work:
   LOD peaks at 14 against a required 18, 256 fallback chunks against an allowance of 128, and
   `tiles_loaded` stays at zero across twenty seconds. Not investigated.
6. **`low_flight_performance`** was a known failure in the terrain era — 420 resident chunks, 334
   fallbacks, a 2,071.204m warm-up seam — and has not been re-measured since the ocean work began.
   Treat those numbers as unverified rather than current.
7. **Thin raymarch probe coverage.** `coast_waters_edge` now has ray baselines (median 1.293m/1.172m,
   p90 3.880m/4.308m at 70 and 71 points, against `surface_height_breakdown_at` rather than the
   raster node path, so not directly comparable with raster's figures). Every other baseline in this
   file is `render_path: raster`, and the two paths are meant to be at parity.
7. The Gerstner fold budget stands at 1.17 against a physical limit of 1.0. Accepted and held
   invariant by `OCEAN_WAVE_SCALE`, not fixed; lowering it is a deliberate visual change.
8. Underwater rendering is unimplemented, which is what the bobbing floor stands in for.
10. **The moon's bake resolves craters to 2.4km and no finer**, because the working grid is 828m a
    cell. Going finer means a bigger grid, and the grid is held in memory as `Vec<DVec3>` — 8,192 x
    4,096 is already 805MB of directions, and doubling it is 3.2GB. Anything below that floor is the
    renderer's detail ladder's job, exactly as it is on the planet. The per-sample catalogue is now
    only the placeholder shown when there is no bake.
11. **The moon surface spawn has never been verified interactively.** `body::spawns_on_surface`
    starts the game standing on the moon, and no scenario reaches that path; scenario replay and
    interactive play differ structurally, and this branch has already lost three defects into that
    gap (ship lag, grey ocean, surface spawn). Needs `--body moon` run by hand.
12. **The moon has never been looked at from the ground.** Every capture of it so far is
    `orbit_once`, from 10,000km, where the disc is under 200 pixels across. Nothing has tested what
    the 2.4km bake floor looks like where the renderer's detail ladder takes over, whether the ice
    reads as a flat pond at eye level, or whether the Lommel-Seeliger lighting still holds when a
    crater wall fills the frame. There are no moon scenarios at all — the four ground scenarios are
    the planet's, on the planet's poses.
13. **The scenario suite is not I/O-independent.** A `stand_on_ground` run taken while a 378MB bake
    was writing came back with a max pixel difference of 12 against a clean run of the same binary:
    tile streaming missed its budget and an ancestor fallback was drawn. Two clean runs since are 0.
    Nothing is wrong with the renderer, but a pixel comparison taken under disk load is not
    evidence, and nothing in the harness says so.

15. **Rim-site selection and measurement are complete; interactive arrival is NOT signed off.**
    The rebaked site has a 27.48-degree rim at 4km and the daylight ground scenario has 1.689m
    clearance. The real frozen startup is on the night side and remains 82.949m above streamed
    ground. Follow up the frozen startup/streaming interaction before calling arrival complete.
16. **The moon's detail-source bandwidth is wrong for resampled sparse tiles.** At the new site,
    runtime height contribution is exactly zero at all 74 compared points, not 0.158m. The high
    cut treats fine tile spacing as real detail, although moon export only resamples the 828m
    working grid; the datum-distance geometry filter also removes shorter wavelengths. Measured,
    not tuned; see the latest section. Correct source-bandwidth metadata is the next design issue.
17. **`moon_crater_wall` passes while rendering entirely black.** Either aim it somewhere lit or say
    in the scenario that it is a night-side capture and assert something that would notice.

18. **~1000ms frames: CLOSED. It was memory pressure, not the renderer.** The test the previous
    section asked for has been run. With swap at 51MB of 976MB (against 976/976 during the stalls),
    five consecutive `ocean_ship_float` runs gave median frame times of 34.8, 34.7, 34.8, 34.6 and
    60.9ms; the last is an outlier on a machine that was not perfectly idle, and no run went near
    1000ms. The two runs immediately before, taken during the swap-exhausted session, were 999.93
    and 999.89ms median. Nothing in the renderer changed between them. **Check `free -m` before
    believing any timing measured in this repo** -- and note the stall is a property of the machine,
    so it will come back whenever swap fills.
19. **The planet's near ground gained detail as a side effect of the moon fix**, moving 76.1% of
    `stand_on_ground`'s pixels. Measured as slightly more relief, not less, and from the same root
    cause — but nobody has looked at the planet since, and every planet baseline older than this is
    stale.
20. **`moon_ground_detail`'s camera pose is a measured lift, not a derivation.** Re-deriving it from
    the baked landing site and eye height would survive the next rebake; this will not.

21. **The near-field window boundary is visible on the moon** as a rectangular island of
    differently-detailed ground. `NEAR_FIELD_MIN_EXTENT_METERS` is 12km, sized for a 4,000km body.
22. **The app's sun is 23 degrees out of the equatorial plane and the ice model assumes zero.** Settle
    this before touching any ice threshold; they are compensating for a premise that does not hold.
23. **`Crater::freshness` is carried and unused**, the first piece of the surface texture.

24. **`planet_to_moon`'s underground start is CLOSED; the clearance measurement is not.** The start
    was the raster mesh query filtering detail against the datum sphere, fixed in `c99787a`; the
    one-pixel terrain hairline is gone at the plain two-metre seating and `planet_clearance` reads a
    true 2.000. What remains open is that the number is still `altitude - height_field`, computed
    from the same query that placed the camera, so it cannot contradict the pose that produced it.
    Measure it from the depth buffer instead.
25. **`planet_to_moon` pins exposure at 1.0 with auto-exposure off.** Correct for comparing phases,
    but its captures are not the game's lighting and should not be judged as such.

26. **The seabed is raster-only; the raymarch path has no equivalent.** The submerged-bottom branch
    lives in `terrain_fragment_color` (`planet.wgsl:1866`) and there is nothing matching it in
    `foveated_debug.wgsl`, which has `is_open_ocean_surface` at line 1038 and no bottom shading at
    all. Crest transmission *is* in both, because it went into `ocean_lighting`, which both paths
    call. So a submerged raymarch view should show no sea bed. This is read from the source, not
    from a capture: there is no CLI switch for the render path, so a scenario cannot select it.
    Parity is the stated goal and this is the path the work is judged in.


**Build convention.** Benchmarks and parity runs build to `CARGO_TARGET_DIR=/home/dad/catingard-target`,
not the in-repo `target/`. Give every temporary or staged checkout its own `CARGO_TARGET_DIR`
(`AGENTS.md`); never share the worktree's. Note that
`catingard/.git` is a 46-byte pointer file — the real object database lives at
`/home/dad/catingard-tmp/catingard-git`, so `catingard-tmp` is **not** scratch and must never be
swept by a disk cleanup.

### Experimental weather — pressure diagnostics (25 August 2026)

Key `7` now retains the existing six-face humidity/cloud/wind diagnostic and overlays
area-weighted surface-pressure samples as fixed 4hPa isobar contours. The strongest resolved local
and global pressure extrema are labelled `H` and `L`; a dark under-stroke keeps the yellow contours
readable over both dry and cloudy cells. The overlay is rebuilt only with the cached HUD, not in the
weather or render hot paths. This change deliberately does not alter pressure, wind, cloud, or
terrain physics: pressure is still diagnosed afresh from temperature before its momentum update.

Validation: all 38 focused weather tests pass, including fixed isobar spacing, finite/bounded
area-weighted pressure bins, and deterministic high/low detection; `cargo check --workspace`
passes with the existing unused `WeatherFields::initial` warning. The next physics milestone is a
mass-conserving prognostic pressure field, using this overlay to verify persistent highs/lows and
their wind response before coupling convergence/subsidence into cloud microphysics.

### Experimental weather — upper-layer isolation (25 August 2026)

The humidity-only 166km cirrus shell is temporarily disabled for the next manual comparison. The
shell renderer submits one instance rather than two, and the shared density function returns zero
for the upper layer before any texture/noise work, so it also cannot shadow terrain or attenuate
the camera-only sun. The 90km condensed-water shell, local low cloud impostors, rain, surface
weather effects, and underlying CPU humidity field are unchanged.

This isolates a reported apparent weather stop. A temporary release probe replayed the manual
run's 6.75 simulated days under its fixed planet-relative sun: over the final twelve 600s states,
12,086 of 24,576 quantized cloud texels still changed and wind remained active at the 60m/s cap.
Rotating solar forcing produced nearly identical late activity, falsifying both a stopped solver
and fixed-sun equilibrium as the immediate cause. The remaining presentation limitation is the
known same-coordinate temporal cross-fade: transported CPU cells morph between locations instead
of sliding visibly with wind. The temporary probe and tagged diagnostics were removed.

Validation: 50 focused weather/render/scenario tests pass. Release replay
`weather_contrast/1787689030-78785` passes all three captures with only the lower global shell.
Next action after manual isolation review: upload wind and apply endpoint-exact advection-aware
temporal sampling consistently to clouds, shadows, sun occlusion, and local presentation.

### Orbital atmosphere and flare-bound repair (25 August 2026)

Manual run `manual/1787689212-80805` exposed two independent presentation faults. The cyan oval in
`capture-001.png` is the intentional camera-only internal lens reflection, not atmosphere or
weather. It was evaluated after a 64-solar-radius sun-centred discard, so that circle cut through
the off-axis ellipse. The ghost lobes are now evaluated first and exempt only their occupied
pixels from that radial discard; the physical sun, corona, clouds, atmosphere, and lighting are
unchanged.

At the `capture-002.png` pose (2,316,514m altitude), the 90km cloud shell was geometrically inside
the 640km optical atmosphere but appeared above its visible limb. The orbital world-space
integrator was applying the 8km/1.2km Earth-like scale heights directly to world altitude, unlike
the surface path's established 4.5x vertical mapping. Consequently Rayleigh density at the cloud
shell was only about 0.000013 of sea-level density. Orbital integration now maps both altitude and
path length through `ATMOSPHERE_VERTICAL_SCALE`, so 90km world altitude evaluates at 20km optical
altitude (about 0.082 Rayleigh density) without changing the atmosphere below the 200km orbital
blend threshold. It also intersects the authored 2,880km world shell rather than the compressed
optical shell.

Focused atmosphere and sun tests pass, as does `cargo check --workspace` with the existing unused
`WeatherFields::initial` warning. Committed release replays pass at
`orbital_sun_visibility/1787690278-91635`,
`orbital_atmosphere_continuity/1787690279-91679`, and
`atmospheric_mist_paths/1787690286-91630`. Against the previous identical continuity capture
`1787402122-26465/capture-012.png`, the top-centre visible limb begins 17 pixels farther outward at
the same low RGB threshold (row 113 to row 96), while the 25km near-surface capture remains on the
unchanged surface path. The full app test invocation still reports three unrelated working-tree
failures: two tests retain the committed 0.5s LOD-fade expectations while the unstaged renderer
sets 1.5s, and one legacy terrain test parses `sun.wgsl` without composing its required shared
cloud-density source.

### Visual-sun scale and centred-flare cleanup (26 August 2026)

The camera-only visual disc is now twice the real solar diameter, approximately **1.06 degrees**.
The halo, inner glare, veil, star-ray, and discard multipliers were halved relative to that larger
disc, so the central flare keeps its previous angular footprint and fragment budget rather than
growing to four times the screen area. The off-axis cyan and magenta lens-ghost lobes and their
shader work are removed completely; the retained optical effect is centred on the sun.

The disc and optical response now use separate pipelines. The disc retains the terrain/planet
depth test, while the centred halo, veil, and star rays use an always-pass depth comparison because
they represent camera response rather than geometry around the source. Both still use the same
cloud and atmospheric attenuation. A shared analytic full-disc planet-occultation test removes the
complete response only once the enlarged visual disc is fully hidden, so a partially clipped sun
keeps the complete centred flare without shining after sunset.

Seven focused sun tests, the deterministic scenario-coverage test, and `cargo check --workspace`
pass with only the existing unused `WeatherFields::initial` warning. Committed release replays pass
at `partial_sun_occultation/1787710920-226542`, `stare_at_sun/1787710922-226587`, and
`sun_horizon_visibility/1787710927-226333`. In the partial-occultation replay, `capture-001.png`
shows the enlarged half-visible disc with the complete centred flare across the terrain silhouette;
`capture-002.png` verifies that both disc and flare are absent after full planet occultation.

### Fixed-L4 performance trial (26 August 2026)

The global raster frontier is now fixed at **L4**, two levels finer than the working L2 baseline.
The default active-leaf budget is raised from 256 to 1,536 for this trial: leaving it at 256 reached
L4 near the camera but forced a visible L0-L4 mixture and therefore did not test the requested
uniform-detail cost. `CATINGARDEN_MAX_ACTIVE_CHUNKS` can still override the trial budget.

The identical release `orbit_once` measurement changes as follows:

| trial | run | median frame | FPS | drawn chunks | terrain triangles | budget bound |
|---|---|---:|---:|---:|---:|---|
| working L2 baseline | `1787748721-462138` | 16.596ms | 60.26 | 41-44 | 94,464-101,376 | no |
| committed L4 | `1787749737-470700` | 42.707ms | 23.42 | 480-538 | 1,105,920-1,239,552 | no |

L4 therefore costs **26.111ms** more in this orbit sample, or **2.57x** the L2 frame time. The final
histogram contains 534 L4 leaves and four transient L3 parents at the moving/culling frontier, not
a budget-driven coarse ring. The scenario passes; the fixed-policy unit test and workspace check
also pass with only the existing unused `WeatherFields::initial` warning. This is deliberately a
performance experiment, not a recommendation to retain the 1,536-leaf default.

### Adaptive LOD restored (26 August 2026)

The fixed-L4 performance experiment above is complete and no longer controls the renderer. Terrain
again uses the normal screen-error-driven adaptive policy from **L2 through L18**, and the near-field
window can select against the outmap's complete maximum level rather than the temporary L4 cap. The
default active-leaf budget is back to 256. Flat-triangle materials remain the presentation mode;
only topology selection changed. Existing transition duration, dither, geomorph, skirts, neighbour
grading, and source streaming are intentionally untouched so their visual behaviour can be addressed
separately.

All 50 active planet tests pass (one diagnostic ignored), including the deterministic exact
L2->L18->L2 optical-zoom ladder, zero-thrash selection, near-surface L18 reach, balanced frontiers,
and bounded chunk counts. The focused near-field-level test and `cargo check --workspace` also pass
with only the existing unused `WeatherFields::initial` warning. Committed release orbit
`1787750941-481007` passes at **16.819ms / 59.46 FPS**, selecting L2-L3 at its 6,000km camera altitude
without binding the budget. That is the correct adaptive result for the view, not a new global cap;
closer or more magnified views can traverse the full ladder.

### Interactive flight-speed scale controls (26 August 2026)

Low-flight movement retains its immediate fixed-speed command and existing altitude curve. A new
persistent runtime multiplier scales the result uniformly: `[` halves movement speed and `]`
doubles it. It starts at **1x**, is bounded to **1/32x-32x**, and remains selected when switching
between orbit and F4 flight. The multiplier is applied after the existing altitude response, Shift
4x boost, and 40,000km/s base cap; consequently the entire ground-to-space curve, including its
high-altitude plateau, remains scalable. Releasing WASD still stops immediately.

Authored scenarios ignore the interactive bracket keys and retain the 1x default. The HUD reports
both current metres per second and the live multiplier, and the control legend lists the new keys.
Focused tests preserve the original 1x ground/altitude behaviour, immediate release stop, and base
cap while verifying exact 0.5x/2x scaling and both multiplier bounds. `cargo check --workspace`
passes with only the existing unused `WeatherFields::initial` warning.

### Dense terrain-source rebake (26 August 2026)

The active procedural outmap had been exported with only `dense_level: 1`, `max_level: 1`, so the
restored adaptive L2-L18 geometry still repeated L1 height, biome, and moisture ownership. The
current 4096x2048, seed `0xEA272026`, 2,048-erosion procedural/mountain-coverage source was rebaked
with globally dense L4 data and sparse refinement through L18. It validates all 3,252 manifest
tiles; manifest SHA-256 is `c97c37a989903c2fa97d1a1460e1c44d6fcbfc26da1bfb2c2b5dcbec4bdf767`.
The previous L1 active directory is preserved at
`assets/outmaps/test-planet.pre-dense4-source-repair-backup-20260826-160701`.

The bake reports 0.16% area-weighted mountain coverage (7,483/5,747,838 land positions; 34,928
qualifying rays). The required post-bake global-summit instrument now passes at 186,701.172077m
presented elevation, 46,735.316406m raw macro elevation, 26.228230938S, 121.070605516W; F4 and the
highest-prominence scenario use the recalibrated direction. The calibration guard now allows the
actual bounded terrain-detail ladder rather than the unrelated retired global-detail amplitude.
The next separate task is to move adaptive LOD refinement thresholds farther from the camera; it
was deliberately not mixed into this bake/calibration change.

### Earlier adaptive LOD transitions (26 August 2026)

After the dense-source rebake was completed separately, the adaptive policy's split/merge error
thresholds changed from 1.5/0.75 pixels to 1.25/0.625 pixels. Projected error is inversely
proportional to distance, so every transition now occurs 20% farther from the camera. Both values
moved by the same factor, preserving the established 2:1 hysteresis band rather than increasing
split/merge churn. LOD range, source limits, 256-leaf budget, topology grading, geomorphing,
transition duration, and material selection are unchanged.

All 50 active `planet::tests` pass (one diagnostic ignored), including the exact L2-L18 optical
zoom ladder, monotonic no-thrash descent, near-surface L18 reach, balanced budget frontier, and
source-limited refinement. `cargo check --workspace` passes with only the existing unused
`WeatherFields::initial` warning. A release `terrain_detail_altitude_ladder` attempt was stopped
without visual sign-off because the current desktop presentation throttled the scenario window to
approximately one frame per second; it produced only two of the required captures.

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
cycle dark per-triangle outlines, outlines off, and a black-fill/bright-red-edge diagnostic; dark
outlines start enabled and the HUD reports the current state. The first two modes only change the
edge mask; the third deliberately replaces the presented land/ocean colour for topology inspection.
Geometric normals and fixed-L7 geometry remain unchanged. Set
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
four hard coverage bands before attenuating direct sunlight by at most 88%; atmospheric sky fill is
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

## Orbital visual-sun path correction - 22 August 2026

Manual capture `manual/1787401323-20009` placed the camera 14,229km above datum with the sun below
the camera's planet-relative horizontal plane, but the actual camera-to-sun ray missed the doubled
atmosphere. The visual-sun shader nevertheless treated that negative local elevation as a surface
sunset, producing an orange, dim disc in vacuum.

Outside the 2,880km gameplay atmosphere, visual-sun attenuation now begins with a camera-ray/shell
intersection. A ray that misses the shell receives unit white transmittance and full presentation
radiance; a ray that enters the shell samples the existing RGB physical transmittance LUT from its
entry point and may still redden correctly through the atmospheric limb. Cameras inside the shell
retain the signed-off surface sunrise/sunset path unchanged. New deterministic scenario
`orbital_sun_visibility` replays the captured geometry and shows the white orbital sun; the focused
shader tests plus surface horizon-sun, sunset/blue-hour, stare-at-sun, and orbital-atmosphere
scenarios pass.

## Experimental weather storm contrast - 22 August 2026

Visible clouds and terrain shadows now share a `CloudSample` containing both the posterised density
and the simulation's storm-intensity channel. A 24-step calibration measured the mature field at
0.139 area-mean and 0.310 maximum storm intensity; the former visual ramp did not become strong
until 0.65, so it could not produce the requested dark weather. The optical-thickness ramp now
reaches its dense end at 0.32 while retaining zero density in clear cells and leaving the sparse
pre-condensation humidity precursor unchanged.

The visible shell keeps thin wisps translucent but pushes the upper density range toward opacity.
Dense cells then darken continuously toward a 72% storm-grey albedo reduction; fair-weather cloud
remains pale and the upper shell retains its established altitude variation. The same denser shared
field drives the existing four-band sun-projected terrain shadows, whose maximum direct-light
attenuation increases from 72% to 88%. Atmospheric sky fill is still preserved, so overcast ground
becomes substantially darker without becoming artificially black.

New deterministic scenario `weather_contrast` renders 1, 6, and 24 completed 600-second weather
ticks from one fixed daylight orbit. Its long timestep also exposed and fixed a harness accounting
bug: expected spatial samples are now capped at one per rendered frame. The scenario, `orbit_once`,
and `sunset_blue_hour` pass; all five weather-render tests, all 24 weather-physics tests, all 21
scenario tests, the focused terrain-shadow test, and `cargo check --workspace` pass. Orbit captures
confirm clear gaps survive while mature regions become more opaque and include grey structure;
fresh low-flight human tuning remains the final judgement of the requested rain-threatening feel.

## Experimental weather elevated shells and cloud-occluded sun - 22 August 2026

The replacement weather renderer has two shells, not a longer stack. They move upward by one
existing altitude slot: the former 14km lower shell now occupies the former upper shell's 90km
altitude, and the upper shell moves to 166km, preserving the established 76km separation. The
shared weather uniform carries both new radii to visible clouds and terrain-shadow projection, so
geometry, altitude-aware sunrise/sunset illumination, and shadow placement remain synchronized.
The 96x48 shell geometry still keeps the lowest planar face above 85km.

The camera-only sun used to render after the alpha-blended cloud pass with a depth-equal test.
Clouds correctly left depth writes disabled for transparency, but that also let the additive HDR
disc and halo shine through dense weather. The sun pipeline now binds the same temporal cubemap and
weather uniform as visible clouds. For the small set of fragments inside the solar halo it
intersects the camera ray with each shell, converts the view-space intersection back to planet
direction, and evaluates the shared cloud density with the same three-octave budget as terrain
shadows. Thin coverage attenuates the light; coverage at or above 0.60 optical opacity removes the
complete disc and halo. One centre-ray visibility value applies to the whole solar presentation so
partial attenuation cannot repeat the rejected apparent-sun-shrinking fault. Cameras outside both
shells with an outward sun ray sample no cloud and retain the signed-off white orbital sun.

New `weather_sun_occlusion` advances 24 exact weather ticks at a sunlit ocean storm, then compares
the sun centre against nearby cloud. Raster run `weather_sun_occlusion/1787404679-54029` passes with
0.008 channel contrast against a maximum 0.030 and visibly contains neither disc nor halo.
`weather_contrast`, `orbital_sun_visibility`, `sunset_blue_hour`, `sun_horizon_visibility`, and
`stare_at_sun` also pass. A blue-hour assertion was made robust to effectively-black `[1,0,1]`
quantisation being mistaken for a second warm peak; the real sky samples and thresholds are
unchanged. All five sun tests, six weather-render tests, 24 weather-physics tests, 22 scenario tests,
15 debug tests, and `cargo check --workspace` pass.

## Experimental weather continuous prediction and presentation clock - 22 August 2026

The CPU weather solver retains its authoritative 600-second fixed step and existing 60m/s
(216km/h, 134mph) wind-speed cap. `WeatherState` now keeps the current field plus one fully
simulated future field. During each interval the shared GPU weather cubemaps blend by the exact
accumulator fraction rather than completing a cosmetic cross-fade in 1.5 wall-clock seconds. At a
boundary, the prior future field becomes the authoritative current field byte-for-byte, the next
future field is calculated, and the render blend restarts from zero. Visible clouds, cloud-driven
sun occlusion, and terrain cloud shadows all consume that one shared blend, so they meet at the
boundary without a visual discontinuity. A multi-step frame resynchronises both cubemaps before
continuing.

Interactive weather now consumes the always-running presentation clock instead of the F10-frozen
scene clock. F10 still freezes planet rotation, ocean/scene animation, and therefore the
planet-local illumination direction, but weather evolution and shader drift continue. Authored
scenarios remain deterministic because their presentation and simulation clocks are identical.
Two focused state tests pin half-interval interpolation, exact current/future boundary identity,
and manual-step restart. All 26 weather tests, six weather-render tests, and `cargo check
--workspace` pass. Deterministic `weather_contrast/1787406322-66611` also passes; its three reviewed
captures retain clear openings and progressively denser storm structure.

## Experimental weather visible motion and cloud silver lining - 22 August 2026

Milestone 14 is still not implemented: the CPU owns precipitation and ground moisture, but there
are no rain particles, wet-ground material response, or accumulated/melting snow yet.

The continuous current/future weather interpolation was working, but interactive rendering fed it
unscaled wall seconds. One 600-second transition therefore took ten real minutes and appeared
static. Interactive weather now uses the implementation plan's `3600x` scale (one real second is one
simulated hour), producing six exact fixed-step transitions per real second; authored scenarios
retain their unscaled deterministic simulation clock. Noise drift is derived from absolute
simulated weather time rather than accumulated render-frame time, so it moves at the same scale,
continues under F10, and is deterministic across frame rates. At most 12 weather steps may be
calculated by one advance; a longer pause discards whole stale intervals while preserving the
fractional phase, preventing a return from suspend from synchronously replaying hundreds of ticks.

Controlled mature-weather captures `weather_contrast/1787410134-94677` cover one equivalent
interactive second in 600-simulated-second increments. Every adjacent image pair changed, with
normalised RMSE values from 0.00680 to 0.00708, and the reviewed sequence shows coherent cloud
movement rather than a static field. The restored standard `weather_contrast/1787410269-96775`
also passes.

Visible cloud shading now adds a Henyey-Greenstein forward-Mie term (`g = 0.76`) when the camera,
cloud fragment, and sun align. It is restricted to the translucent alpha fringe and fades out by
0.62 opacity, so opaque storm interiors remain dark rather than becoming emissive. The term is
multiplied by the physical atmospheric sun transmittance, giving white daylight silver lining and
a naturally red/orange sunset lining while vanishing when the elevated sun is geometrically
hidden. Same-pose 1,800s before/after diagnostics show the transparent cloud around the still
visible sun brightening. Fully opaque `weather_sun_occlusion/1787410209-95679` still passes at 0.012
sun/background contrast against the 0.030 maximum. `orbital_sun_visibility/1787410296-97331`,
`sun_horizon_visibility/1787410302-97422`, and `sunset_blue_hour/1787410322-97602` pass unchanged.

The focused suite has 29 weather tests, seven weather-render tests, 22 scenario tests, five sun
tests, and a clean `cargo check --workspace`. The next roadmap work remains real baked-terrain
coupling followed by milestone 14.

## Flat-terrain distance mist and Quadro cost check - 22 August 2026

The default fixed-L7 flat-triangle presentation was the one remaining raster path that deliberately
skipped the renderer's existing 4-120km terrain mist. Land now composes that mist into the existing
per-vertex aerial transmittance/in-scatter values; flat water triangles and the analytic flat-ocean
path receive the same physical camera-sky endpoint after their ocean-specific lighting. The mist is
strictly camera-distance based rather than gated by surface horizon angle, so successive landscape
layers separate even when looking across a valley instead of only at the geometric horizon. Its
near-surface guard now uses camera clearance above each rendered surface, not absolute altitude
above sea level, so the unusually tall game mountains do not incorrectly disable it. The foreground
inside 4km remains unchanged, the ramp completes by 120km, and the existing 200km clearance fade
prevents the presentation effect from covering orbital views.

The land path initially sampled the physical sky-view LUT per fragment. That was visually correct
but inappropriate on the Quadro M1000M, so the final implementation folds the sample into the
already-interpolated vertex aerial components without adding an inter-stage variable. A controlled
release `highest_prominence_peak` replay measured 16.70ms median before that final distance path and
16.63ms after (approximately 59.9 versus 60.1 FPS); neither replay had a frame above 25ms. Final run
`highest_prominence_peak/1787411927-113489` passed finite-metric, no-LOD-thrash, and screenshot-count
assertions. Its pre-existing active-bake pose mismatch remains explicit: camera clearance is
12,026.193m against the stale 150-155m scenario expectation, so the scenario is not claimed as an
overall pass.

The three focused mist/WGSL tests, the affected flat-triangle presentation test, and `cargo check
--workspace` pass. The full 264-test app run reached 253 passed / 4 failed / 7 ignored: the fog-caused
flat-presentation literal was corrected and passes on rerun; the remaining three failures are the
pre-existing dirty-worktree LOD timing assertions and incomplete standalone sun-shader composition,
not this change.

## Interactive weather frame-pacing worker - 22 August 2026

The intermittent interactive hitch was CPU frame pacing, not the Quadro's steady raster cost and
not the distance mist. The 3,600x interactive clock crosses six 600-second weather boundaries per
real second. Each boundary synchronously cloned and simulated all 24,576 weather cells on the
render thread before encoding the frame. An isolated optimized instrument measured eight state
predictions at 21.142-23.448ms each, already over the complete 16.667ms 60Hz frame budget; recent
manual logs showed the resulting isolated 36-141ms frames without any terrain tile/chunk uploads.

Interactive weather now prepares its first target during renderer initialization, then keeps a
second future state in flight on one named `weather-prediction` worker thread. The render thread
continues to interpolate current -> next exactly as before. At each boundary it promotes the exact
next state, installs the already-completed following state, and immediately asks the worker to
predict one state beyond that. If the worker is unexpectedly late, the visible blend holds at 1.0
until the complete state arrives rather than blocking or exposing a partial result. Resume backlog
remains bounded by discarding obsolete whole intervals while retaining one pending boundary and the
fractional phase. Authored scenarios deliberately keep the old synchronous clock so deterministic
capture timing and state generation are unchanged.

A byte-for-byte regression compares the first two asynchronous targets against the synchronous
solver. In a release 60Hz pacing instrument including target texture packing, render-thread weather
work changed from 21.1-23.4ms prediction pulses to 0.000ms median / 1.206ms p95 / 3.080ms maximum.
The 30 weather tests and seven weather-render tests pass, as does `cargo check --workspace`.
Deterministic `weather_contrast/1787413016-123061` passes all finite-metric and capture assertions.
The full app suite reports 255 passed / 3 pre-existing dirty-worktree failures / 7 ignored. Idle
interactive smoke `manual/1787413095-123954` starts without errors, but an unfocused window is
compositor-throttled to roughly 1Hz and is explicitly not used as frame-pacing evidence; fresh
focused manual flight remains the visual sign-off.

## Experimental weather twilight-layer fade - 22 August 2026

Manual orbit capture `manual/1787422267-4192` showed an isolated mauve cloud band bounded by two
clean terminator lines. At its 11,530km-altitude camera position the local solar elevation was
about -13.49 degrees: exactly between the point-height solar horizons of the 90km cloud shell
(-12.04 degrees) and the 166km shell (-16.23 degrees). The upper shell was therefore fully directly
lit while the lower shell was fully shadowed, and each changed state across only the Sun's 0.53
degree disc. This was cloud lighting, not a terrain, atmosphere-LUT, or cube-face seam.

Each rendered shell is now treated as the centre of a 76km-deep cloud volume for direct-light
visibility. The high edge sees the first part of the Sun and the low edge sees the complete disc
last; a smooth illuminated-fraction ramp spans that interval. Adjacent layers meet at their shared
38km half-depth, so the upper layer fades in from night while the lower layer fades into daylight
with overlapping transitions rather than exposing a hard-edged strip. Shell geometry, density,
weather physics, atmospheric RGB transmittance, ambient irradiance, and terrain lighting are
unchanged.

A controlled release replay reused the manual camera position, Sun direction, fixed exposure, and
zoom with a centred planet view and mature deterministic weather. Baseline
`weather_contrast/1787423172-13676` reproduces the hard strip; fixed
`weather_contrast/1787423210-14178` shows the cloud light declining continuously through both old
boundaries. The restored standard `weather_contrast/1787423311-15162` passes all assertions. Eight
weather-render tests (including composed-WGSL validation and an explicit transition-overlap
regression) and `cargo check --workspace` pass.

## Experimental weather orbital cloud coverage - 22 August 2026

Manual ascent `manual/1787424675-25273` appeared to show clouds disappearing with camera altitude,
but the screenshots did not follow one ground patch. The camera moved 22.6 degrees around the planet
between captures 1 and 2, then reached 35.8 degrees from the cloudy start by capture 8. A matched
2,827,794m camera/weather/Sun replay reproduced the clear result. Replacing sampled cloud colour
with an opaque diagnostic covered the whole near-side shell (`weather_contrast/1787425275-29965`),
proving shell geometry, reversed-Z depth, solid-planet occlusion, and the draw pass were intact. A
raw RGB field diagnostic (`weather_contrast/1787425405-31014`) then exposed the cause: after 1,356
weather steps, condensed water had concentrated into narrow fronts and the viewed hemisphere was a
real coarse-field dry region. Fewer than 1% of its shell pixels exceeded the normal cloud-density
discard threshold. This was geographic coverage, not an altitude fade or lighting failure.

The upper 166km layer now supplies a sparse fair-weather cirrus component. Five-octave rotated detail
still cuts explicit clear gaps; a thin 0.45 presentation floor prevents the coarse single-layer
climate from leaving an entire orbital hemisphere blank, and live humidity increases that component
toward its 0.24 density ceiling. The lower layer, condensed-water posterisation, storm density and
darkening, field transport, lighting, shell geometry, depth/planet occlusion, and atmospheric path
are unchanged. Because this remains in the shared density source, visible cirrus, terrain shadows,
and camera-to-sun cloud attenuation continue to agree.

The exact captured pose and 813,600s weather state now visibly retain broken high cloud in
`weather_contrast/1787425971-35794`. Restored standard mature-weather orbit
`weather_contrast/1787426114-37238` keeps clear holes between broad systems, while low-altitude
`landing_site_eye_level/1787426132-37442` remains a broken cloud ceiling rather than a uniform film.
All eight focused weather-render tests, including composed WGSL parse/validation, pass.

## Terrain mist atmospheric-path correction - 22 August 2026

The extra terrain mist added for visual scale was not actually measuring air along the view ray. It
used a 4-120km camera-to-fragment distance ramp and multiplied that by an explicit camera-clearance
factor that fell from one to zero over 0-200km. Consequently, otherwise identical terrain rays lost
all extra mist merely because the camera climbed above 200km. This was the altitude-dependent switch
seen in `manual/1787424675-25273`; the physical aerial-perspective path remained active underneath,
but the added presentation mist did not follow the intended rule.

`terrain_fog` now intersects the finite camera-to-surface segment with the existing 2,880km gameplay
atmosphere shell. It converts that in-atmosphere segment to a sea-level-equivalent air distance using
the same 72km Rayleigh density scale, endpoint density average, and bounded 1-12x grazing air mass as
the existing aerial transmittance approximation. Mist amount is `1-exp(-airPath/500km)`. There is no
camera-altitude fade and no raw 4-120km scene-distance threshold. A vertical ray from space crosses
one effective 72km air column and receives 13.4% extra mist rather than zero or saturation. At
1,440km camera altitude, a terrain-horizon ray reaches the 12-air-mass / 864km equivalent column and
receives 82.2%; the longer atmospheric path itself creates the stronger result.

New deterministic scenario `atmospheric_mist_paths` captures 25km grazing, 1,440km grazing, 1,440km
radial, and 4,000km radial views with fixed daylight and exposure. Baseline
`atmospheric_mist_paths/1787427784-51836` demonstrates the old clearance switch; fixed
`atmospheric_mist_paths/1787427731-51331` retains modest radial haze and strong grazing haze. Both
land and flat-owned water keep the shared physical-sky fog endpoint. Numeric radial/grazing tests,
scenario parsing, flat land/water coverage, and composed planet-WGSL parse/validation pass. The
controlled run's median logged frame time was 16.623ms fixed versus 16.752ms baseline under the
present capped presentation path, so it shows no detected regression but is not an uncapped GPU
benchmark.

### Terrain mist final-composition repair

The fog amount and physical-sky endpoint were previously folded into the vertex aerial components
before fragment material correction. Vegetation subsequently multiplied even a fully saturated fog
endpoint by its 0.42 aerial in-scatter scale, snow neutralised it, and flat-triangle outlines were
drawn after the fog mix. Consequently changing the e-fold distance from 500km to 5km reached the
active shader but still left dark material facets and black wireframe visible through nominally
opaque mist.

Terrain vertices now carry the fog amount and sky endpoint separately by packing them into existing
inter-stage locations; the Quadro path remains at locations 0-15 rather than exceeding its limit.
Fragments apply biome-specific physical aerial correction first, draw flat outlines where enabled,
then mix the complete surface result toward the interpolated sky endpoint. This adds no fragment LUT
lookup and keeps cloud-shell rendering independent. With the deliberately extreme local 5km test
value, `atmospheric_mist_paths/1787429254-64643` passes and the obscured terrain plus outlines visibly
converge to the sky. The committed production constant remains the prior 500km unless the local
tuning hunk is explicitly promoted.

## Experimental weather baked-terrain coupling - 23 August 2026

Weather startup now samples the active outmap at the centres of the existing six-face
64x64 climate grid. A bounded level-2 tile cache loads at most 96 source tiles (using the
manifest's ancestor fallback), then bilinearly samples physical exported height and moisture
and nearest-samples biome ownership. Ocean and lake cells are sea level with ocean thermal
inertia; land uses baked elevation, land/ice albedo, land heat capacity, and baked moisture.
The renderer's altitude exaggeration is deliberately not applied to weather, so lapse-rate and
orographic forcing remain camera-independent and match the source data.

`WeatherState::new_with_terrain_samples` receives those values during normal startup. The old
seam-safe procedural relief remains only as a deterministic fallback for placeholder launches and
unit tests. `baked_terrain_samples_override_the_climate_fallback` plus all 31 focused weather tests
pass; milestone 14 (rain presentation, wet-ground response, and snow) is the next implementation
slice.

## Experimental weather milestone 14 - 24 August 2026

Precipitation now has visible surface consequences. Cold land routes the bounded liquid-equivalent
precipitation into `snow_cover`; warm cells melt that cover with a two-hour relaxation and return the
melt to the coupled ground-moisture reservoir. Ocean and lake cells remain outlets. Diagnostics expose
snow min/max/mean alongside moisture and precipitation, and the RGBA8 surface cubemap carries ground
moisture, snow cover, and normalized temperature alongside the cloud cubemap. The existing smooth and
flat terrain paths consume that shared, temporally interpolated surface field: wet land darkens and
receives a bounded per-fragment specular response, while snow blends toward a neutral cover tint.

A 384-particle camera-local rain pass reads the same current/future precipitation cubemaps and fades
with the interpolated field, so rain presentation cannot disagree with terrain wetness or cloud state.
It is depth-tested against the scene, disabled above 20km camera altitude, and uses alpha-blended line
streaks rather than a second weather simulation. The shared cloud-density source remains the sole field
lookup for visible clouds and future consumers. Forty-three focused weather tests (including snow melt,
texture packing, and rain-shader parsing), seven weather-render tests, and `cargo check -p
catinthegarden-app` pass. Fresh Quadro visual tuning of rain opacity and snow palette is still required
before promoting milestone 15 polish.

## Experimental weather milestone 15 seasons - 24 August 2026

Interactive weather now has an annual solar-declination cycle: the planet-local daily sun azimuth is
preserved while a deterministic 23.439-degree tilt follows a 365.2422-day orbital phase. Phase zero is
the established solstice orientation, so startup and authored scenario lighting are unchanged; scenario
sun waypoints remain authoritative and do not acquire an unrequested seasonal perturbation. The
interactive weather clock already runs at 3,600x, so one in-game year advances in roughly 2.43 hours of
wall time for visual testing. A focused regression pins unit length, constant azimuth, solstice,
equinox, and opposite-solstice declinations.

## Experimental weather milestone 15 advection polish - 24 August 2026

Temperature transport now uses a bounded MacCormack predictor/corrector pass. The backward predictor
and forward correction preserve thermal fronts better than the previous single semi-Lagrangian sample;
the four-cell bilinear source stencil clamps the corrected value, preventing overshoot at cube-face
seams and sharp fronts. Humidity and condensed-cloud transport remain on their conservative mass-flux
path, so this change does not alter the established water-conservation contract. Focused deterministic
weather tests pass, including an explicit hot-front no-new-extrema regression.

## Experimental weather milestone 15 local impostors - 24 August 2026

The final milestone-15 presentation slice adds 96 camera-local cloud billboards below 22km. Each puff
uses deterministic hashed offsets and the existing simulated weather drift, samples the current/future
weather cubemap through the shared `cloudSample`/`cloudDensity` WGSL source, and blends between those
states with the same fraction as shells, terrain shadows, sun occlusion, and rain. Storm intensity
controls grey/darker albedo and direct sun controls brightness; reversed-Z depth testing keeps puffs
behind terrain without writing transparent depth. Above 22km the pass is vertex-suppressed, leaving
orbital shell rendering unchanged. The cloud-field sampler visibility was widened to vertex+fragment so
rain and impostor vertex sampling are valid on wgpu. Ten focused weather-render tests, including
composed shared-density validation, pass; `cargo check -p catinthegarden-app` passes with only the
existing unused fallback-constructor warning. Fresh Quadro captures are still needed to tune local puff
opacity/coverage by eye.

## Experimental weather sunset cloud lighting - 24 August 2026

The final cloud-lighting follow-up now uses the same physical atmosphere data for every cloud
presentation path. The shell shader keeps the signed low-sun transmittance LUT column (clamped only
above its compressed optical horizon), and its direct RGB sunlight uses the terrain/ocean scale. The
96 local impostors now bind the transmittance and surface-irradiance LUTs in their vertex stage rather
than reducing sunlight to a scalar dot product. Both paths also apply a scale-height-limited
camera-to-cloud transmittance column, so a distant white shell is not composited over a red sunset
without the intervening air reddening it. This is still physical RGB extinction/irradiance; no
clock-keyed sunset tint was added. The fullscreen atmosphere already uses the signed LUT source and
continues to produce the red/orange horizon and blue upper sky as the sun descends.

The composed WGSL parser/validator suite (10 weather-render tests), `cargo build -p
catinthegarden-app`, and deterministic `sunset_blue_hour` plus `sunset_sweep` Quadro/Vulkan replays
pass. The latest blue-hour capture shows neutral daylight clouds, warm gold/red sunset clouds, and
dark post-sunset remnants; fresh interactive/manual tuning of cloud coverage remains optional.

## Camera-visible solar veiling glare - 24 August 2026

The physical solar disc was already the correct 0.53-degree angular size, but the orbital
`stare_at_sun` capture read as a tiny isolated white circle because the existing 6.5-radius corona
was clipped at its own radius and had no broad camera/eye veiling response. The sun shader now keeps
the disc geometry and all physical atmosphere/surface lighting unchanged, widens the compact halo,
and adds a low-intensity 18-radius veiling-glare lobe. The fragment cutoff includes that lobe, so it
is not silently discarded outside the compact halo. The glow remains additive HDR presentation only,
uses the existing RGB atmospheric/cloud visibility, and is removed together with the disc when the
planet fully occults the solar ray.

Focused sun tests (5), app build, `stare_at_sun/1787576771-1001614`, and
`sunset_blue_hour/1787576831-1002047` pass. The orbital capture now presents a visibly soft,
camera-like bright source while preserving the fixed physical disc size.

## Experimental weather shell transport correction - 24 August 2026

The latest manual orbital sequence showed two distinct visual motions: cloud coverage on the
near-side shell gradually changed, while a second pattern appeared to rotate around the planet
independently. The shared density source was applying both the interpolated wind-advected field and
an extra global longitude rotation (`rotate_drift`) plus a time-varying flow-warp phase. That made
the renderer transport the same weather twice, so a fixed camera could watch one shell move into a
clear region even while the CPU field's mean cloud water was stable or increasing.

`weather_cloud_density.wgsl` now samples the planet direction directly. Its flow warp is a fixed
per-layer spatial breakup, and all shell/shadow consumers therefore see the same wind-advected field
motion. The low-altitude camera-local impostors keep their separate simulated-time hash drift so
near-ground puffs still move. The cloud-render source test explicitly rejects the removed global
rotation/time phase. Focused weather-render (10) and weather-state (34) tests pass, as does a Vulkan
`weather_contrast` replay (`1787532213-733696`); its fixed-camera captures retain continuous weather
coverage changes without an independently orbiting shell.

## Photographic solar flare correction - 24 August 2026

The camera-only visual sun now follows the supplied outdoor-camera reference more closely. The
physical 0.53-degree disc and all atmosphere, terrain, ocean, cloud, exposure, and occultation
lighting remain unchanged. The presentation overlay uses a compact 6.5-radius corona, a restrained
30-radius veiling lobe, bounded aperture-like star rays, and faint axis-aligned purple/cyan internal
lens ghosts when the source is off the optical centre. Low-sun glare visibility was lowered to avoid a
large red bloom while preserving the physically reddened disc and terrain lighting. Star-ray angle
multiples use algebraic Chebyshev recurrence rather than extra per-pixel trigonometric calls for the
Quadro M1000M path.

Five focused sun tests, ten weather-render tests, release Vulkan `stare_at_sun`,
`sunset_blue_hour`, and `sun_horizon_visibility` replays pass. Fresh captures are under
`test-runs/stare_at_sun/1787600080-1166721`, `test-runs/sunset_blue_hour/1787600147-1167550`, and
`test-runs/sun_horizon_visibility/1787600107-1167227`. FIFO presentation was anomalously slow in
this headless replay; the same release scenarios pass with `CATINGARDEN_PRESENT_MODE=immediate`.

## Solar flare tail boundary repair - 24 August 2026

A manual near-surface capture showed a sharp circular boundary around the new flare. The overlay was
being discarded at the 30-radius veil even though the star-ray tail extended to 42 radii, so the
zero-valued tail was clipped into a ring. `SUN_OVERLAY_CUTOFF_RADIUS_SCALE` is now 64 radii, separate
from the veil and ray falloffs; the lobe contributions still converge smoothly to zero before that
budgeted discard. Physical disc size, cloud occlusion, atmospheric tint, and lighting are unchanged.

Five focused sun tests, ten weather-render tests, and fresh release Vulkan immediate-present
`stare_at_sun` and `sun_horizon_visibility` replays pass. The no-ring captures are under
`test-runs/stare_at_sun/1787601363-1176160` and `test-runs/sun_horizon_visibility/1787601375-1176278`.

## Experimental weather cloud lifetime correction - 24 August 2026

The rapid cloud disappearance was real simulation decay amplified by the density threshold, not a
second renderer transport bug. Interactive weather advances six 600-second transport states per
real second, while the former 900-second cloud phase and 3,600-second precipitation constants aged
on that same 3,600x clock. Their wall-time constants were therefore only 0.25s and 1s, and a dry cell
could empty a smaller cloud-water reservoir inside one fixed update.

Wind, temperature, humidity, and condensed-water transport retain the established 3,600x clock and
the same smooth current/future interpolation. Surface evaporation, condensation/re-evaporation,
precipitation, and snow melt now receive a separate 60x microphysics timestep: each 600-second
transport state advances those processes by 10 seconds. Cloud phase and precipitation therefore
have 15s and 60s wall-time constants, with no additional render-thread work or change to transport
speed. A focused one-real-second regression retains more than 95% of a representative local cloud
reservoir instead of applying six near-hourly decay steps.

All 35 weather tests and ten weather-render tests pass, including byte-exact synchronous/background
prediction parity. `cargo check --workspace` passes with the existing unused fallback-constructor
warning. Deterministic debug replay `weather_contrast/1787602587-1186856` passes finite metrics and
all three captures; its 1/6/24-state sequence retains and develops coverage rather than wiping the
viewed field clear. A temporary full-field diagnostic likewise increased area-mean cloud water from
0.00926 after six states to 0.07208 after 60 and 0.12379 after 120. The complete app unit run remains
at 266 passed / 3 unrelated dirty-worktree failures / 7 ignored: the two known LOD presentation-time
assertions and the standalone sun-shader composition test.

## Experimental weather cloud-layer separation - 24 August 2026

The longer-lived cloud field exposed a presentation defect that the former rapid decay had hidden.
Both 90km and 166km shells sampled the same condensed-water channel at nearly the same direction;
different detail noise changed their edges, but the same macro systems appeared twice at visibly
different altitudes. The shell fragment alpha then promoted mature density toward full opacity, so
the overlap read as a thick matching double layer.

The lower shell remains authoritative for condensed water and storm structure. The upper shell now
uses only the existing humidity-driven, independently detailed cirrus component until the climate
simulation owns a genuinely separate high-altitude water field. Because this choice is inside the
shared density source, terrain shadows and camera-to-sun cloud attenuation agree with the visible
layers. Visible shell alpha is now a direct 0.78 density scale instead of the former upper-range
opacity boost; dense weather remains visibly grey and shadow-casting while retaining transmission.
Weather physics, the 60x microphysics clock, transport, precipitation, rain, atmospheric lighting,
and shell geometry are unchanged.

All ten weather-render tests, all five sun tests, the focused shared terrain-density test, and the
composed planet-WGSL test pass. Standard `weather_contrast/1787604675-1202061` passes at 1/6/24
states. Temporary extended deterministic replay `weather_contrast/1787604772-1203499` passes 120
finite states and captures 1/60/120-state coverage; its mature frame shows one broken condensed
layer plus thin cirrus rather than matching opaque shells. The checked-in scenario was restored
unchanged.

## Sky/terrain distance-mist continuity - 27 August 2026

Manual capture `manual/1787821535-923601/capture-001.png` showed a fogged distant mountain against
an abruptly clear blue sky. The terrain path already applied the 500km e-fold presentation mist on
top of physical aerial perspective, but the fullscreen sky stopped after the physical sky-view LUT
and its 2x visible presentation gain. The silhouette therefore joined two different air-column
models.

The fullscreen sky now evaluates a sea-level-equivalent Rayleigh column from camera altitude, view
zenith cosine, and the ray's closest approach to the planet. It uses the terrain mist's same 72km
world scale height, 12-air-mass grazing cap, and 500km exponential scale, then converges toward the
physical same-azimuth ground-horizon LUT colour. The closest-density contribution fades in by actual
vertical descent, preventing a second hard band as a ray crosses the camera's local horizontal. No
screen-space height ramp, raw scene-distance threshold, or camera-altitude disable was added, so
overhead naturally remains clearer while grazing rays carry the haze.

At the reported pose's 177,661m datum altitude (144,647m local terrain clearance), the numeric mirror
measures 1.21% radial sky mist and 82.24% on the ground-horizon ray. Controlled before/after captures
are `atmospheric_mist_paths/1787822221-928441/capture-001.png` and
`atmospheric_mist_paths/1787823114-936359/capture-001.png`; the fixed sky grades continuously toward
the already-misted horizon without a local-horizontal edge. All eight atmosphere tests, composed
fullscreen WGSL parse/validation, and `cargo check -p catinthegarden-app` pass with the pre-existing
unused weather constructor warning.

## Restrained procedural-terrain smoothing - 27 August 2026

The request was to smooth the terrain only slightly without undoing the broad mountain forms or the
flat-triangle presentation. The baked macro outmap, erosion, mesh topology, LOD policy, categorical
materials, per-triangle normals, and outlines are unchanged. Only the runtime detail ladder's
height/slope roughness moves from 0.060 to 0.058 (3.3%); its derived finite amplitude bound moves
from 491.5m to 475.1m in both CPU clearance and WGSL displacement/material normalization.

The existing deterministic slope instrument measures the resulting visual-gradient change as
p50 0.02782->0.02611, p90 0.08066->0.07618, p99 0.14017->0.13314, and maximum
0.28632->0.27557 in `1-cos(angle)`. This is a roughly 4-6% reduction in apparent local steepness,
while the authored macro ranges retain their full height and width. The rejected alternative was to
increase ridge-fold softness: after required mean/RMS recalibration it raised, rather than lowered,
the measured p90/p99 slopes.

The CPU/GPU constant-parity test, derived amplitude-bound test, ridge calibration, composed planet
WGSL validation, all planet tests, and app check pass. `terrain_detail_altitude_ladder/1787824754-950077`
passes all five captures; daylight visual capture
`highest_prominence_peak/1787824844-950777/capture-001.png` is finite and clean, while that scenario's
pre-existing stale camera pose still fails its unrelated 150-155m clearance assertion at 7,648.7m.

## Low-flight centre-patch LOD priority - 27 August 2026

The four-frame manual retreat in `manual/1787828575-973578` showed more visible relief in capture
003 than in the two closer captures. The centre leaf itself was monotonic, but the previous
crosshair priority created one L13 needle inside an L11 neighbourhood at the closest pose. The
mixed-LOD edge filter then correctly suppressed unresolved detail around that needle, making the
close terrain look smoother than the farther, more uniform patch.

Low-flight leaf-budget priority now covers a 12-degree half-angle view-space patch around the first
centre-ray terrain hit rather than only the single containing leaf. Its world-space footprint grows
with hit distance, so it occupies a stable part of the view while retaining the existing 256-leaf
budget. The captured four-pose regression now measures centre/lowest-patch levels of L13/L13,
L13/L12, L12/L11, and L12/L11 respectively; centre detail never increases while retreating and the
close view is no longer an isolated fine needle.

All 52 active planet tests pass (one diagnostic ignored), and `cargo check --workspace` passes with
the existing unused weather-constructor warning. Exact-pose release replay
`lod_focus_patch_probe/1787829763-986057` passes four finite captures and visually shows dense
triangle coverage in captures 001-002 before the expected reduction in 003-004. The temporary
scenario source was removed after the replay. The full app suite remains at its same three unrelated
dirty-worktree failures: two stale 0.5-second transition assertions versus the local 1.5-second
setting, and the standalone sun-shader composition test.

## Distance-bounded flat adaptive LOD - 27 August 2026

The complete twenty-frame manual retreat in `manual/1787830500-992135` disproved the earlier
four-pose level-only sign-off. Although centre and patch detail filters were already monotonic, the
flat selector was spending spare budget on mesh levels much finer than its continuous 1%-of-distance
terrain-detail filter could display. Which over-refined quadtree cells won changed at cell
boundaries; because the presentation exposes every face normal, a newly selected coarser facet
could read as detail returning while the camera moved away.

Flat mode now caps each node at the first level whose 32x32 vertex spacing can represent that node's
minimum camera-distance filter. The ordinary projected-error request, 12-degree focus priority,
L2-L18 range, source data, and 256-leaf budget remain unchanged; the cap only rejects invisible
over-tessellation. The exact twenty poses now retreat from 100.9m to 71.0km with centre LOD
L18,L15,L14,L14,L13,L12,L12,L12,L11,L11,L11,L10,L10,L10,L10,L10,L10,L10,L9,L9. Both patch
endpoints are non-increasing and its minimum/median/maximum effective filters are non-decreasing at
every sample. The strict regression, all 52 active planet tests, workspace check, and temporary
release replay `lod_retreat_probe/1787850894-1125672` pass; the replay source was removed afterward.

## Solid-terrain occlusion for the optical sun flare - 27 August 2026

The pinned report is `manual/1787835489-1024416/capture-001.png`: the physical disc was correctly
depth-clipped, but the red star/veil remained plainly visible through a solid foreground mountain.
This was the separate always-depth optical pipeline introduced to preserve the complete camera
response around a partially visible sun; its analytic occultation test knew only the spherical
planet and could not know about local relief.

The physical disc remains depth-tested. The optical flare now runs in a following pass without a
depth attachment and samples the completed reversed-Z solid depth at the disc centre plus sixteen
points just inside its rim. Any clear point retains the complete flare, preserving the signed-off
partial-disc behaviour; if all seventeen points contain solid terrain, the whole optical response
is removed. Pixels outside the bounded flare footprint discard before those depth reads. The depth
texture is shader-readable and its bind group is rebuilt with render-target resize.

All eight focused sun tests and workspace check pass. Release
`partial_sun_occultation/1787852028-1137404` preserves the full flare around the half-visible disc
and removes it after full occultation. The exact camera/rotation/sun ray replay
`mountain_sun_occlusion_probe/1787851755-1134789` removes the bright through-mountain star; its
temporary scenario source was removed after capture.

## Near-field detail-source ownership repair - 28 August 2026

The twelve-frame outline-off approach in `manual/1787854388-1153066` showed relief becoming less
detailed after capture 005 even though the camera continued toward the same ground. Exact-pose
instrumentation proved the centre selector itself was monotonic (L10 through L18) and changing the
local 1.5-second transition experiment back to the committed 0.5 seconds made no visual difference.
The reproduction also found that the previous distance-cap change had created and unit-tested the
`flat_level_limit` closure but the production call still passed an always-L18 closure; production
now passes the intended distance cap as well, so the earlier regression finally covers the path it
describes.
The failure began when a patch entered the dense near-field grid: that grid was addressed at its
fine requested level but was still populated from the resident L4 ancestor. Its instance metadata
incorrectly advertised the requested level as the sampled source level, so the shader concluded
that the baked texture already owned the corresponding procedural bands and removed them.

Near-field instances now report the resolved `source_key.level`; addressing remains at the fine
window level, while procedural-detail ownership follows the data actually sampled. The checked-in
`manual_lod_approach_replay` scenario preserves all twelve reported camera poses and fails the old
path with a 298.695m close-frame p90 surface mismatch. Final outline-off release replay
`manual_lod_approach_replay/1787879458-17274` passes with 146 compared ground points, 32.548m worst
frame p90, 40m tolerance, and relief that remains present through captures 001-011. The last pose is
a separate presentation extreme: only 6.11m above the surface and looking 26.2 degrees downward,
flat per-triangle lighting produces severe bright/dark strips; a normal-shaded control
`manual_lod_approach_replay/1787879122-16002` proves the surface itself is continuous. That flat-mode
close-up appearance was documented but deliberately not conflated with this LOD/source fix.

## Subtle flat-triangle outlines - 28 August 2026

Flat land and water outlines now retain 68% of each triangle's fully lit, atmosphere-composed
colour instead of 8%. They therefore remain a slightly darker version of the local triangle rather
than near-black wire, while preserving antialiasing, fog convergence, the `O` toggle, categorical
fill, and per-face lighting. Exact release replay
`manual_lod_approach_replay/1787881651-19747` passes all twelve captures plus the 146-point surface
regression; captures 001, 005, 009, and 011 show the mesh remaining legible without dominating the
terrain.

## Experimental billboard forest - 28 August 2026

Branch `experiment/billboard-forest` adds a deliberately isolated low-poly forest presentation.
The active outmap was searched for a moist, low temperate-forest patch that faces the frozen
startup sun; its authored centre is `[0.374871986443, 0.737334908710, 0.561968171854]`. Ordinary
interactive startup still applies F4/F6/F10, but F4 now places the camera two metres above this
surface, inside a fourteen-metre clearing, looking nearly level through the trees.

The forest is 12,288 deterministic camera-facing billboards distributed over an 800m radius. Each
tree is grounded independently from the same CPU terrain surface used by flight clearance, rejects
water, writes the reversed-Z depth buffer, and uses procedural broadleaf/conifer silhouettes and
flat sun/ambient colour without textures. One immutable instance buffer and one instanced draw keep
the experiment bounded; it is omitted above 50km altitude.

The deterministic `forest_startup` release run at
`test-runs/forest_startup/1787883150-32249` passes two captures, finite metrics, and 2.050m measured
clearance. `capture-001.png` verifies a daylight forest from the authored startup pose. Focused
forest, startup-camera, scenario-load tests, format checking, and the dedicated release build use
`CARGO_TARGET_DIR=/home/dad/catingard-forest-target`.

## Forest night lighting and measured procedural direction - 28 August 2026

The first billboard shader added an unconditional 0.36 light term, and the trunk fragment path had
a second fixed brown output. Together they made trees self-lit after the terrain and sky were dark.
Canopy and trunk now share one light scalar: direct sun plus a bounded sky-ambient term that fades
to exactly zero once the sun is sufficiently below the local horizon. Fixed-exposure night replay
`forest_night/1787914305-49531` passes and is fully dark; the earlier diagnostic
`forest_night/1787914120-48539` remains useful because it exposed the otherwise missed glowing
trunks.

`CATINGARDEN_FOREST=0` is a render-only measurement switch, and `forest_performance` is a
capture-free fixed-pose benchmark. Five alternating-order Quadro/Immediate ON/OFF pairs measure a
median 0.201ms forest cost (0.148-0.292ms range): 27.223ms/36.73 FPS on versus
27.011ms/37.02 FPS off.
Timestamp profiling was deliberately not used because it hangs this driver. The complete one-local-
patch, projected-size tree LOD, and far forest-material design is in
`docs/FOREST_RENDERING_PLAN.md`.

## Procedural forest completion and pop repair - 28 August 2026

The forest design is implemented without a global tree population. `TerrainRenderer` supplies a
resident-cache-only rendered height, macro height, biome, moisture, source level, and finite-
difference slope sample. One active L12 camera-local `ForestPatch` is generated from canonical
half-open cube-sphere cells; per-tree placement rejects water, non-forest ownership, dry ground, and
slopes above 32 degrees. A pending CPU builder processes at most 128 candidates per frame, retains
the old active patch until complete, and abandons obsolete intermediate cells during high-speed
travel. Finished populations replace over 1.5 seconds through stable per-tree selection in the same
bounded 12,288-instance buffer and one draw.

Manual pair `manual/1787920912-59912` diagnosed the serious pop: a roughly 7m move changed one
camera-point eligibility sample, clearing then synchronously recreating the whole forest and
producing 226-244ms logged frames. That camera-point clear gate is removed. Exact four-capture
replay `forest_boundary_transition/1787931761-65966` holds one patch and 11,141-11,143 visible
instances throughout, passing at 27.549ms median / 29.727ms maximum sampled frame. Projected-height
LOD now continuously hash-thins from full above 12px to zero below 1px rather than revealing a
quarter population at the threshold. Day `forest_startup/1787931959-66773` and fixed-exposure night
`forest_night/1787931970-66756` pass; the latter remains fully dark.

Far views use no tree geometry: the existing terrain fragment path adds seam-safe direction-noise
canopy breakup from 32-160km only on moist, gentle, unsnowed positive forest-biome land. Final five
alternating-order Quadro/Immediate `forest_performance` pairs measure the tree draw at 0.227ms median
(0.059-0.316ms), versus the old 0.201ms result; run paths/results are under
`test-runs/forest-profile-pairs/procedural-final`. Do not enable timestamp profiling on this driver.

## Global deterministic evergreen forests - 28 August 2026

The radial 800m startup disk has been removed. Startup and travelling patches now share the same
canonical half-open L12 cube-sphere cell generator, so forest placement is globally procedural and
repeatable without a circular authored boundary. Ice, tundra, and mountain-snow biomes force the
evergreen/conifer silhouette; temperate and tropical forest biomes use deterministic mixed woodland.
Height, width, shade, and breakup remain deterministic per tree.

Forest eligibility now includes temperate/tropical forest, tundra, ice, and mountain-snow land,
subject to positive terrain, moisture, and a 32-degree slope limit. Water and lakes remain excluded.
A seam-safe 192-cell directional value-noise field varies candidate density from 35% to 100%, giving
coherent stands and clearings while leaving every eligible location capable of producing trees.
Far terrain canopy shading uses the same broad density field from 32-160km and darkens terrain in
proportion to tree density, including snowy evergreen regions. Focused forest/terrain shader tests
pass; fresh GPU visual review of cold-biome coverage remains required.

## Forest retreat/return continuity - 28 August 2026

An empty, fully evaluated neighbouring L12 cell no longer replaces a populated forest patch. The
empty key is remembered while the prior patch remains the LOD source, so trees fade out and return
continuously during retreat/return instead of requiring another cell-boundary crossing. The focused
forest suite includes `empty_neighbouring_cell_does_not_replace_a_populated_patch`.

## Forest cell-edge footprint - 28 August 2026

Tree candidates still use canonical half-open L12 ownership, but accepted trees now receive a
slightly warped radial edge falloff inside the cell. The stand tapers before the UV boundary and no
longer presents a square silhouette; terrain eligibility, deterministic placement, and the bounded
single-patch renderer are unchanged. Focused forest tests pass.

## Forest-centre atmospheric beams - 28 August 2026

An opt-in forest beam presentation adds three crossed translucent radial shafts from the accepted
forest centre to the top of the 2,880km gameplay atmosphere. The shafts are depth-tested against
terrain, rendered before weather shells, and sun-elevation tinted. They are off by default and toggle
with **B**; no weather state or tree population is added.

## Render-range procedural forests - 28 August 2026

The earlier one-patch working set is superseded. Global placement remains deterministic, but the
renderer now incrementally retains every populated L12 cell whose footprint intersects the current
individual-tree range, capped at an 8km camera distance and 128 cells. Selection is nearest-first
and crosses cube-face seams. The first three startup cells perform 512 resident-cache terrain probes
per frame and show without an entry fade; later nearest cells use 256 probes and surrounding cells
retain the 128-probe budget.

All active cells concatenate into the existing single tree draw with a hard 262,144-instance cap.
Stable projected-size thinning remains in force, and each completed cell enters over 1.5 seconds so
coverage grows without a whole-cell pop. The **B** diagnostic now emits one atmospheric beam per
populated render-range cell. Focused forest tests pass; release/GPU visual and performance validation
uses `forest_startup/1787955024-137828`, which passes both captures and 2.05m clearance with three
populated cells visible by capture 001. Fully populated render-range performance profiling remains.
The focused forest filter passes 30 tests and the release app builds. The full app run passes 312,
fails 1, and ignores 7: the sole failure is the pre-existing dirty-worktree sun-shader unit test,
whose standalone source cannot resolve `cloudDensityWithOctaves`; it is unrelated to forest code.

## Global forest locators and symmetric approach cache - 29 August 2026

The forest beam overlay no longer follows `ForestRenderer::patches`, which was both camera-local and
pruned at the tree draw boundary. The bounded terrain startup scan now returns coarse forest-capable
outmap samples alongside weather climate data in the same I/O pass. Moist samples are selected into
1,000km-separated deterministic regional waypoints plus the authored startup forest; the current
active bake yields 122. Their immutable vertex buffer is created once, and **B** submits all shafts
at every camera distance. The vertex shader expands each shaft to constant screen width, retaining
terrain depth testing, weather veiling, and the 2,880km atmosphere-top endpoint. **B** remains off by
default; `CATINGARDEN_FOREST_BEAMS=1` exists for deterministic captures.

The return-only continuity fault had a separate cause: construction began at the same boundary as
rendering, while an already-built retreating patch remained resident until it crossed that boundary.
Actual tree geometry still draws only within 8km/128 cells, but a 12km/256-cell deterministic cache
now prebuilds and retains surrounding patches. Missing visible cells pre-empt background work and use
256 probes per frame; prefetch retains the 128-probe budget. A focused regression proves every
draw-range key is already in the prefetch set.

All 31 focused forest tests pass. Release `forest_travel/1787994724-154197` passes its six moving
captures, and beam-on `orbit_once/1787994695-154088` passes with all 122 locators submitted and visible
at orbital scale. The earlier 883-locator diagnostic was rejected as an unreadable starburst before
commit.

## Black/red flat-triangle topology mode - 29 August 2026

The `O` control is now a three-state cycle: the existing subtle dark outlines, outlines off, then
pure black triangle interiors with bright-red antialiased edges, returning to dark on the next press.
`FlatTriangleOutlineMode` is encoded as 1/0/2 in the existing
`CameraUniform::flat_triangle_options.x`; no resource, bind group, pipeline, or draw call was added.
The black/red presentation is applied explicitly to both terrain-owned water and the analytic flat
ocean path and intentionally bypasses material lighting, aerial perspective, and distance mist so it
remains an unambiguous topology diagnostic. Ordinary dark/off rendering is unchanged.

Three focused flat-triangle tests pass, including the complete host cycle and WGSL land/ocean
coverage. A temporary black/red-default release replay was restored immediately after capture;
`forest_startup/1787995360-158939` passes, and `capture-002.png` visually confirms black facets with
bright-red edges. Production startup remains on the original dark-outline mode.

## Forest cloud shadows and truthful global locators - 29 August 2026

Billboard trees now bind the same current/previous temporal weather cubemaps and render uniform as
terrain. Their direct sunlight is multiplied by a three-octave cloud-density lookup projected from
the tree toward the sun through both cloud shells, then posterised to the same four shadow bands as
terrain. Sky ambient remains unshadowed, so overcast trees follow the surrounding ground without
becoming emissive at night or unnaturally black under every cloud. The forest shader is composed with
the shared `weather_cloud_density.wgsl` source and fully parsed/validated in its focused test.

The global **B** locators previously advertised coarse L2 biome/moisture samples that could still be
rejected by actual L12 tree placement. Each coarse anchor is now refined once at startup through its
real deterministic tree candidates and the dense terrain height/biome/moisture/slope path; anchors
with no eligible tree are discarded. The active bake therefore yields 109 truthful locators from 122
coarse candidates.

Manual `1787995768-162111/capture-001.png` exposed a separate high-mountain failure: the camera was
roughly 1.1km over 42km of presented terrain, but the cell-range intersection used a sea-level tree
shell. Its 8km search could not reach that shell and returned zero cells, hence the HUD's `0 trees | 0
patches` beside a valid nearby locator. Render and prefetch ranges now use the terrain height beneath
the camera, with a regression that reproduces the formerly empty 42km case.

All 34 focused forest tests pass. Workspace check, formatting, and whitespace validation pass. The
release `forest_startup/1788025163-277316` scenario passes both captures and 2.05m clearance, reaches
six populated patches and 12,551 visible instances, and logs a 26.787ms median frame. The full app
suite passes 316, fails one, and ignores seven; the sole failure is the already documented unrelated
standalone sun-shader test whose dirty-worktree source lacks `cloudDensityWithOctaves` composition.

## Forest scale, slope grounding, and non-circular footprint - 29 August 2026

Tree candidate directions, hashes, and the 12,288 candidates per L12 cell are unchanged, preserving
the established spacing and deterministic identities. Billboard height doubles from the 11-24m
range to 22-48m; width remains derived from height and therefore doubles by the same factor, as do
the procedural trunk and crown silhouettes inside each billboard.

The old fixed 0.45m base sink could not ground a several-metre-wide billboard on an eligible slope:
its downhill edge could be metres above the surface. Accepted trees now retain that margin and add
`0.5 * width * tan(slope)`, bounded by the existing 32-degree placement limit. This is the
camera-facing worst case, so every orientation's base reaches or enters the local tangent plane.

The explicit radius-from-L12-cell-centre multiplier was the cause of circular stands. It is removed
entirely; canonical half-open cells still own candidates, but acceptance now depends only on the
seam-safe global density plus actual biome, moisture, and slope. Adjacent cells therefore form one
continuous deterministic forest instead of separate round islands.

`forest_canopy_albedo` now applies at ground level as well as at planetary distance. It samples the
same frequency-192 value-noise mapping and 0.35-1.0 density range as CPU tree acceptance, then
darkens all eligible terrain consistently with that density. The former 32-160km camera-distance
handoff and density-dependent tint are gone; an identical density has an identical ground tint
regardless of camera distance. At the fixed startup pose, matched bottom-frame terrain luminance
drops from 128.6 to 118.3.

All 37 focused forest tests pass, including pre-fix red regressions for scale, downhill grounding,
radial masking, and near terrain darkening. Release `forest_startup/1788026781-285186` passes both
captures and 2.05m clearance with 26,072 visible instances at 28.032ms median. Release
`forest_travel/1788026471-284130` and `forest_boundary_transition/1788026696-284583` also pass and
show irregular terrain-owned forest areas. A same-build `forest_performance` ON/OFF pair measures
28.689/28.195ms median frame intervals, a 0.493ms tree-draw cost after the doubled projected size.
Workspace check, formatting, and whitespace validation pass. The full app suite passes 319, fails
one, and ignores seven; the sole failure remains the unrelated standalone sun-shader composition
test documented above.

### 2026-08-29 organic forest-density correction

The follow-up correction keeps L12 candidate ownership and its existing spacing/cost, but replaces the broad frequency-192 field's unconditional 35% placement floor with one seam-safe frequency-1024 field and a 4% sparse tail. CPU tree placement and terrain canopy darkening use the identical function. This makes forest density vary within work cells rather than exposing every completed cell as a uniformly filled quadrilateral. All 27 focused forest tests and the planet WGSL validator pass; release `forest_startup/1788028069-288688` passes, with neighbouring eligible cells ranging from zero to 9,489 accepted candidates. The current dirty-worktree run still contains a 124.627ms long frame and the terrain remains capped at 256 leaves with extensive ancestor fallback; neither separate issue is signed off by this forest-shape change.

### 2026-08-29 sunset halo cloud-response correction

Manual movement captures at `test-runs/manual/1788027976-288142` objectively reproduce the reported colour cycle: median red-minus-blue in a 30-150px annulus around the sun rises from -0.047 to +0.157 in captures 001-006, resets near neutral in 007, reaches +0.235 in 013, then resets in 014 while fixed exposure remains 1.0. The upper physical sky changes little; the changing component is the camera-only red sun glare. `cloud_sun_visibility` already converts geometric cloud transmission to its fourth power, but the halo applied another fourth power, making its effective response transmission^16 while the saturated core appeared stable. The halo now shares the existing transmission^4 response. Thin, visually inconspicuous cloud therefore retains the preferred pink glare, while the existing >=60% opaque-cloud cutoff still returns zero and removes both source and glare. All eight focused sun tests and shader validation pass. Release `sunset_sweep/1788041936-359130` captures four frames and passes the sunset red-growth assertion; its seam and fallback failures are the pre-existing terrain-budget defect and are not attributed to this change. Exact replay of the original oscillation is not currently automated because the manual run does not serialize its evolved weather field.

### 2026-08-30 flat terrain material-resolution correction

Flat mode previously chose biome and moisture once at the centre of each 32x32-grid geometry triangle, despite every resident terrain source already carrying a 129x129 material map. That made forest, grass, snow, and other categorical land boundaries inherit the much larger geometry facets. `flat_triangle_colour` now samples the fragment's interpolated baked source coordinate for its primary biome and moisture. Lighting remains genuinely flat: geometry, face normal, per-triangle specular, and outlines are unchanged. The existing three-vertex `flat_triangle_land_biome` fallback remains around water, so a categorical water texel within raised mixed coastline geometry cannot reintroduce sloping water. This uses up to four times the linear material resolution and sixteen times the material cells without rebaking or increasing geometry. Focused forest/material regression and full planet WGSL validation pass. Release `forest_travel/1788050161-390974` passes all six captures with a 26.905ms median logged frame time; the images are at `test-runs/forest_travel/1788050161-390974/screenshots/`.

### 2026-08-30 forest-boundary refinement

The coarse L4 biome source made the temperate-forest/grassland transition appear as a large hard-edged stand. Temperate grassland now participates in mixed woodland when its continuous moisture, positive-land, and slope checks pass. CPU billboard placement and flat terrain canopy darkening share that ownership rule, while dry grassland, rock, and water remain excluded. Focused forest and planet-WGSL tests pass; a fresh GPU capture is still recommended to tune the moisture threshold visually.

### 2026-08-30 compact high-resolution forest mask

The global L4 climate map remains unchanged, but the forest density/ownership refinement now uses one seam-safe directional 8,192-cell field (roughly 3km scale) in both CPU billboard placement and the flat terrain shader. Moist temperate grassland can become local mixed woodland and low-density temperate forest can open into grassland, breaking source-texel-sized rectangular stands without another baked channel, texture stream, or geometry cost. Water, desert, and rock are not eligible. Focused forest/WGSL tests and a release startup capture pass.

### 2026-08-30 forest placeholder LOD and capped ground darkening

Forest billboards no longer drop the entire below-sparse population. A deterministic 12% subset remains as tiny placeholders (10% width/height), with smooth scale ramps through sparse and medium LODs before reaching full trees. The same stable tree identities are used at every level, so nearby trees grow into full billboards instead of being replaced by a newly sampled population. Pending cells expose their already-resolved candidates during bounded construction under a smooth build-progress ramp; this removes the whole-cell spawn when entering a new area without adding a second forest draw path.

The terrain forest treatment now reconstructs a compact deterministic point field in the shader. Contributions add with a 38m radial fade from each point and clamp at `FOREST_GROUND_DARKENING_MAX = 0.58`; the result is weighted by moisture, slope, snow, land, and local forest density, and applies at all distances in both flat and smooth terrain. No terrain geometry, baked channels, or tree spacing changed.

Focused forest tests (27), the planet WGSL validator, full app tests except the pre-existing standalone sun-shader composition failure, release build, `forest_startup/1788081252-505956`, and `forest_boundary_transition/1788081500-506913` pass. The startup run's median frame time was 34.50ms (11 logged frames), versus 36.23ms in the immediately preceding captured build; boundary captures show small distant trees during patch construction and regular billboards after the transition. Screenshots are under each run's `screenshots/` directory.

### 2026-08-30 matte terrain specular gating

The terrain shader now treats all non-ice land as matte. The flat-triangle per-face specular value is computed/used only for biome IDs Ocean, Lake, and Ice; vegetation, soil, rock, tundra, desert, and MountainSnow receive zero. The smooth terrain weather/wet-ground highlight uses the same gate, while the existing analytic ocean/lake Blinn-Phong glints remain unchanged. Diffuse, ambient, aerial perspective, cloud shadows, and forest lighting are unaffected.

A focused material regression confirms the biome gate in both raster paths and that shared water lighting still owns its specular lobe. `cargo fmt --all -- --check`, the focused WGSL/material test, and `cargo check --workspace` pass.

### 2026-08-30 permanent forest hierarchy

The frequency-8,192 procedural forest mask remains the permanent global representation and continues
to darken eligible terrain with no tree geometry at orbital distances. Inside the 12km cache, each
L12 cell now resolves 128 spatially distributed terrain samples into complete multi-crown canopy
cards, publishing at most two whole proxy cells per frame. The proxy remains while the full
12,288-candidate population is built and transitioned; pending individual candidates are no longer
submitted, so bounded construction cannot look like isolated trees growing from empty ground.

Release `forest_boundary_transition/1788101492-518818` passes and begins with a dense proxy forest
before the camera-local individual population is ready. Release
`forest_vast_distance/1788101903-520046` passes at 100km altitude with zero proxy or individual-tree
instances and visibly distinct forest terrain. Five alternating Immediate-mode `forest_performance`
ON/OFF pairs measure 34.875ms versus 34.004ms median frame time, a 0.872ms total forest cost inside
the requested 1ms limit. Focused forest tests and workspace check pass; the full app suite is
322 passed, one pre-existing standalone sun-shader composition failure, and seven ignored.

### Experimental instant global forests (30 August 2026)

Branch `experiment/instant-global-forests` preserves the prior queued/proxy implementation on
`experiment/billboard-forest` at `b92f8e9`. The new default removes arrival-time CPU population:
each visible canonical L12 cell uploads one 64-byte descriptor, then a GPU compute pass evaluates
the same L4 height/biome/moisture, procedural detail, slope, species and density rules directly
into the existing tree instance buffer. Stable candidate subsets retain all 12,288 candidates
inside 1.5km, 768 from 1.5-4km and 64 beyond 4km. `CATINGARDEN_GPU_FOREST=0` retains the old path
for comparison; `CATINGARDEN_FOREST=0` remains the matched render-cost control.

The first frame of release `forest_startup/1788124782-538155` reports 128 cells, 76,800 bounded
candidates, L4 sources, zero pending candidates and transition progress 1.0; both captures show the
complete near forest. Moving `forest_boundary_transition/1788124946-539404` reports the same zero
pending work across all four captures. Sub-pixel 38m terrain circles now hand over by footprint to
a broad seam-safe canopy field, rather than aliasing into the former comb/ring pattern;
`forest_vast_distance/1788125181-540904` passes with zero individual tree geometry.

Three alternating Quadro Immediate `forest_performance` pairs measured forest-on medians
35.569/36.524/35.456ms and forest-off 34.164/34.038/34.342ms. Median groups are 35.569ms versus
34.164ms: **+1.405ms**, within the requested 2ms frame budget. Forty-one focused forest tests,
GPU WGSL validation, release startup, boundary and vast-distance scenarios pass. The unrelated
manual sunset/ocean/pink-pillar report remains a separate diagnosis; do not tune the signed-off
near-surface sky while repairing it.

## Frozen-sun pink-cycle and apparent-ocean diagnosis - 31 August 2026

The manual run `test-runs/manual/1788105529-522111` held scene time, planet rotation,
sun direction, exposure, and FOV fixed while travelling from roughly 22.3km to 12.9km altitude.
The physical sky nevertheless looked alternately pinker and greyer, and mid-distance water read as
if it had disappeared. The logged camera positions and inferred travel direction are preserved in
`manual_sky_ocean_replay`; its matching sky-only control is smooth, while the full composition adds
the weather shell. This isolates the variation from the atmosphere scattering itself and avoids
retuning a near-surface sky the user has otherwise signed off.

The weather shell now maps alpha as `density * mix(0.50, 0.78, density)`: thin and medium cloud no
longer form an opacity film that repeatedly desaturates the sunset or hides the blue water below,
while density 1 storms retain the previous 0.78 ceiling and therefore their contrast and shadow
strength. The inside-viewed shell changes from 96x48 to 192x48 vertices, refining only its narrow
meridional facets so a low-altitude sunset cannot expose one as a sky-height pink wedge. This does
not change cloud density, drift, lighting, atmosphere LUTs, terrain fog, ocean colour, or fragment
work.

Validation:

- all 10 focused `weather_render::tests` pass;
- `scenario::tests::manual_sky_ocean_replay_preserves_the_logged_frozen_sun_flight` passes;
- release `manual_sky_ocean_replay/1788147856-555226` passes with nine captures and retains a
  clearly blue ocean beneath the unchanged smooth sunset sky;
- release `weather_contrast/1788147944-555932` passes; its 24 logged samples have a 56.915ms median,
  versus 67.270ms in the immediately preceding 96x48 baseline run
  `weather_contrast/1788126079-546626`. Run-to-run noise prevents claiming a speed-up, but there is
  no measured regression from the one-axis tessellation increase.

Fresh manual travel through the exact evolved weather state is still the visual acceptance gate,
because scenarios initialise deterministic weather rather than serialising the old manual run's
entire weather cubemap.

## Hybrid local ocean and forest-ground repair - 31 August 2026

The ocean keeps one planet-wide sea-level ownership shell, but open ocean within 1.5km of the
camera now receives six seam-safe broad Gerstner waves and fades geometrically back to that shell
by 3.5km. Three cheaper normal-only ripple octaves remain strongest inside 250m and fade by 1.2km;
wave strength also fades through the first 30m of water depth so coastlines do not climb or split.
Lakes remain exactly flat. Flat-triangle mode now lets the analytic ocean shell own open water,
uses its denser near-field grid where available, and gently removes ocean outlines with distance
to avoid a horizon-scale wire moire. CPU clearance conservatively reserves the measured 3.18m
maximum surface excursion over resident open-ocean tiles.

Release `ocean_hybrid_close/1788190945-582814` passes at 100m altitude with four captures, zero
seam delta, a measured 5.166m animated wave-height range, and a 19.856ms median frame sample.
The captures are in that run's `screenshots/` directory. A same-build immediate-mode diagnostic
at the old invalid `ocean_flyover` location changed 2.100ms to 2.311ms (+0.211ms), which is useful
only as a draw/shader overhead check because that stale scenario is below the current terrain.

The large dark land patches without corresponding visible trees had two concrete causes: terrain
darkening and GPU tree placement used different eligibility thresholds, and the ground shader
always represented the full 12,288-tree population even when the tree renderer had selected its
768- or 64-candidate distance tiers. Both paths now share the same biome/moisture/land/slope test.
Ground darkness follows the visible population with smooth tier boundaries, each tree's local
shadow radius is 14m with a bounded 55% additive maximum, and the broad orbital forest tint starts
only beyond 7km. Release `forest_ground_eligibility/1788171417-575191` passes; `capture-002.png`
shows the repaired transition rather than the former broad black field.

Validation: `cargo check -p catinthegarden-app`, 41 focused forest tests, focused ocean/shader
tests, both release scenarios above, and `git diff --check` pass.

The formerly failing sun regression now validates the same composed WGSL source used by the
production pipeline (`sun.wgsl` plus `weather_cloud_density.wgsl`) instead of incorrectly parsing
the dependent sun stage in isolation. Its stale transmittance function/variable assertions were
also updated to the current LUT-backed names. No production sun rendering changed. The complete
app suite now passes: 325 passed, zero failed, seven ignored.

## Interactive coastal startup - 31 August 2026

Interactive startup still applies the established F4/F6/F10 state automatically, but F4 now
places the camera 100m above a low dry headland and gives it an authored orthonormal tangent facing
across a broad open-sea fan with an eight-degree downward pitch. The dense source tile is still
loaded synchronously before placement, so the camera cannot jump after startup. The old forest
centre remains available to forest scenarios and global forest generation.

The first coastal attempt was not acceptable: the camera was too low behind a foreground ridge,
and automatic cursor capture let NoMachine/mobile synthetic relative motion rotate the authored
view before the first useful frame. Interactive startup now leaves the cursor free and preserves
the pose; a deliberate left click captures it for ordinary desktop mouse-look, while loss of focus
still releases it. Release interactive capture
`test-runs/manual/coastal-startup-check/screenshots/capture-006.png` is the untouched result and
shows land at the bottom with a wide blue sea across the complete view. Formatting, the focused
pose regression, release build, error-free interactive launch, and the complete app suite pass:
325 passed, zero failed, seven ignored.

## Broad translucent cloud fringes - 1 September 2026

The shared cloud-density function now maps its raw boundary through a `0.025-0.40` smoothstep
instead of the former `0.08-0.26` interval. This more than doubles the density range occupied by
the transparent-to-opaque fringe while preserving density one in mature cores. Because terrain
shadows, sun attenuation, local impostors, and the shell renderer all consume this one function,
their cloud ownership cannot drift apart. The fragment alpha ceiling, storm-core darkening, cloud
lighting, simulation fields, and shader operation count are unchanged.

Release `weather_contrast/1788246157-618266` passes with three captures; its corresponding narrow
edge baseline is `weather_contrast/1788245976-617553`. Focused weather-render WGSL validation and
all ten weather-render tests pass. Their 24-sample medians are 60.202ms and 57.198ms respectively;
the unchanged smoothstep cost means the small run-to-run difference is not attributed as a shader
regression.

## Always-running interactive ocean - 1 September 2026

The hybrid Gerstner surface was stationary during every normal interactive launch because startup
still deliberately applies F10 and the ocean phase used that frozen scene clock. Ocean phase and
its CPU diagnostic now use presentation time, matching the already always-running weather clock.
F10 continues to freeze planet rotation, sun position, and composition; scenarios remain unchanged
because their authored presentation and simulation clocks are identical.

The focused frozen-scene regression failed with `2.0` seconds retained instead of the expected
`7.5` presentation seconds before the repair and passes afterward. Release
`ocean_hybrid_close/1788247401-621889` passes with four captures and the established 5.166m wave
height range. The exact default F10-frozen coastal startup was then captured 15 seconds apart at
`test-runs/manual/1788247601-622472/screenshots/`; terrain and camera remain fixed while the ocean
phase advances.

## Readable ocean motion and amplitude controls - 1 September 2026

The original normal-only octaves were 4.5m, 2.1m, and 0.9m wavelengths and faded out by 1.2km.
From the 100m coastal start they were mostly sub-pixel or absent, so even the repaired clock looked
static. The normal bands are now 180m, 70m, and 28m, remain full strength through 2km, and fade by
8km. Actual Gerstner geometry, the 3.18m collision reserve, coast attenuation, and lakes are
unchanged.

The three independent shading amplitudes are named next to the ocean distance controls in
`shared_planet.wgsl`: `OCEAN_RIPPLE_FIRST_AMPLITUDE` (1.8),
`OCEAN_RIPPLE_SECOND_AMPLITUDE` (0.64), and `OCEAN_RIPPLE_THIRD_AMPLITUDE` (0.20). These can be
tuned without changing water height. `OCEAN_GEOMETRY_AMPLITUDE_SCALE` controls actual geometry and
must remain synchronized with the CPU constant in `ocean.rs`.

Release `ocean_hybrid_close/1788250019-628073` passes with four captures and the same 5.166m
geometric range. Over the first 1.5 seconds, mean absolute near-water RGB movement rises from
`1.845/3.018/4.319` in clock-only run `1788247401-621889` to `3.359/5.913/7.871`; median frame time
is 19.929ms versus 19.790ms. The shader operation count is unchanged, so the 0.139ms difference is
within run noise rather than attributed cost.

## Crossing-wave interference presentation - 1 September 2026

Manual sequence `test-runs/manual/1788262996-632698` proved that advancing the clock and enlarging
the normal bands was still not enough: the ocean changed only about 1-1.5 display-code RGB values
between its roughly 0.4-second captures, underneath much stronger static low-poly facets. The three
wave axes were already genuinely distinct; projected into the tangent plane at that exact coastal
camera, their pairwise separations are approximately 111, 118, and 130 degrees.

The three signed ripple heights are now summed alongside their slopes. This is linear wave
superposition: constructive crossings produce a positive combined crest and destructive crossings
cancel toward a trough. Flat-ocean lighting exposes that existing result through restrained crest
scatter and trough darkening; it does not add a travelling texture, raise geometry, change the
5.166m broad-wave range, or change collision. The three directions are now named
`OCEAN_RIPPLE_FIRST_AXIS`, `SECOND_AXIS`, and `THIRD_AXIS` beside their amplitude constants.

Release `ocean_hybrid_close/1788263590-635146` passes. Its first two near-water frames differ by
mean absolute RGB `4.475/7.228/10.313`, up from `3.359/5.913/7.871` before explicit interference
lighting and far above the visually static manual sequence. Median logged frame time is 19.689ms;
the prior run was 19.929ms. Full raster and raymarch WGSL validation covers the expanded local
`OceanSurface` data.

## Weather-driven giant ocean swell - 1 September 2026

Broad geometric waves now sample the weather simulation's local storm intensity at the camera.
The value is bilinearly filtered across cube-face weather cells and interpolated between the same
600-second states used by cloud presentation, then carried to both raster and ray ocean paths in a
spare camera-uniform channel. The CPU diagnostic and collision model use the identical curve: 4x
amplitude below storm intensity 0.15, smoothly rising to 25x by 0.85. The six authored amplitudes
therefore have a conservative 23.938m maximum crest, covered by the 24m bound, without forcing the
calm camera to hover at the storm clearance.

The dominant pair now shares a 1,400m wavelength and 0.375m source amplitude. Its global axes are
6.88 degrees apart, while 10.0m/s and 9.2m/s phase speeds evolve the constructive/destructive
interference rather than freezing one beat pattern. Giant geometry remains full through 4km and
fades by 10km so a storm swell cannot expose the former 3.5km flattening ring. The uncommitted GPU
40x trial was not retained as a permanent mismatch: CPU clearance had still assumed 4x.

Release calm-coast `ocean_hybrid_close/1788265977-645407` passes all assertions with a 6.296m
observed range, 3.83m calm conservative crest, and 18.376ms median versus the preceding 19.689ms
run. This capture verifies calm behaviour and the enlarged local patch; the 23.938m storm endpoint
is pinned by CPU/GPU-mirrored constants and focused tests rather than claimed from this calm view.
The complete app suite passes 329 tests with seven diagnostic instruments ignored.

## Phase-matched low-flight ocean clearance - 1 September 2026

The low-flight clearance path now samples the same time-, direction-, and local-storm-dependent
Gerstner height as the rendered ocean instead of reserving the global 24m worst-case crest at
every location. A camera already at minimum ocean clearance follows both rising crests and falling
troughs within a 0.25m contact tolerance, preventing the old one-way correction from ratcheting the
view upward. Movement sweeps and the camera near-plane surface estimate use the same instantaneous
surface; land collision is unchanged.

`ocean_rough_horizon` is a deterministic 5m-ASL level-view storm endpoint with four captures and a
30m minimum wave-range assertion. Release run `1788287715-677897` passes at 39.352m and visibly
shows foreground crests occluding waves behind them. The calm control
`ocean_hybrid_close/1788287739-678029` remains at 6.296m. The complete app suite passes 331 tests
with seven diagnostic instruments ignored.

## Global storm sea and near-surface sky stability - 1 September 2026

Open sea now always uses the established maximum storm endpoint (25x authored Gerstner
amplitude) in raster, ray, CPU clearance, near-plane truth, and diagnostics. The existing
2-30m bathymetric smoothstep still shoals the rendered waves at coasts; CPU collision now mirrors
that depth attenuation instead of reserving an offshore crest over shallow water.

Manual captures `manual/1788288158-684939` also exposed an independent sky pulse: descending only
75.6m at the same location, view, sun, fixed exposure, and planet rotation reduced top-quarter sky
luminance by 19.3%, while a subsequent 2km lateral move at fixed altitude changed almost nothing.
The sky-view LUT's geometric-horizon remap was overreacting to optically negligible eye-altitude
changes. Generation, display, terrain fog, and surface sky sampling now share a stable 200m
near-surface reference; genuinely elevated cameras remain altitude-dependent.

Deterministic replay `ocean_low_sun_stability/1788289693-702643` reduces the reproduced vertical
move from 19.3% to 0.52% top-quarter luminance change, with the lateral control at 0.37%. Maximum
storm replay `ocean_rough_horizon/1788289734-703019` passes its 30m gate at 39.352m and visibly
self-occludes across successive crests.

## Planet-surface walking and swimming camera - 2 September 2026

Key `G` toggles a third interactive camera mode from the existing F4 low-flight camera. Entering
surface mode resolves the resident terrain directly below the current camera and places a 1.70m
human-eye camera on land, or at the buoyant equilibrium height in open ocean. Pressing `G` again
returns to low flight at the same position; F4 still returns either interactive close mode to the
saved orbit pose. Mouse look and WASD remain planet-relative, while `[` and `]` scale the 4.4704m/s
walking and 2.0m/s swimming speeds through the existing bounded speed scale.

Surface vertical motion is a fixed-substep gravity simulation rather than a surface clamp. Space
applies one 5.2m/s land jump when grounded or one 2.5m/s upward impulse while submerged. Ocean
buoyancy is derived from the submerged fraction of a simple 1.70m body, with vertical drag against
the analytic Gerstner surface velocity; wave motion therefore lifts and releases the camera with
inertia rather than copying wave height. Land contact has a 2cm tolerance for streamed-height
jitter. Uphill movement is rejected above 42 degrees, while descent and entry into water remain
allowed. Underwater rendering is intentionally not implemented yet.

CPU ocean ownership now mirrors the shader's actual non-ice/non-lake, non-positive-height rule,
including coastline samples whose categorical biome remains land. Wave depth, collision and
buoyancy use unclamped bathymetry even in flat-triangle mode, whose hidden terrain mesh is clamped
to sea level. The analytic CPU wave-height derivative supplies vertical water velocity and is
regression-checked against a centred finite difference.

The surface-camera depth query reads the resident raw baked bathymetry rather than the
sea-level-resolved visible surface. This keeps open-ocean depth non-zero, so Gerstner
height/velocity drive buoyant bobbing instead of a fixed 0.255m equilibrium above a flat
zero-depth shell. A dynamic rising/falling-wave unit test guards the behaviour.

Surface vertical integration and the post-streaming correction enforce a bounded -100m
underwater safety floor. This is below the maximum storm trough but well above the solid
planet; the LOD selector accepts that bounded radius, so a falling trough no longer pins the
camera at sea level and reports a false multi-dozen-metre clearance.

Interactive startup now moves to the deterministic deep open-ocean direction at 30.246944N,
14.474559W (approximately 50km seaward of the authored coast), enters surface swimming mode,
and uses the existing global maximum storm-wave scale.
The camera begins at the buoyant equilibrium height with a four-degree downward pitch across the
water; scenario launches and the manual `G` toggle retain their authored/current-location paths.

Manual ground and jump captures are in `manual/1788349152-867669`: capture 001 is grounded at
human-eye height, capture 002 is airborne at +1.36m/s after Space, and capture 003 returns under
gravity. The forest renderer was disabled only for these diagnostic captures to isolate camera
motion on the Quadro; normal runtime defaults are unchanged. All 344 non-ignored app tests pass,
with seven diagnostic instruments ignored.

Open-water visual verification is in `manual/1788361902-880886`. After movement stopped, eight
successive captures measured 0.07-0.52m of camera clearance above the changing Gerstner surface,
with vertical velocity changing sign; the HUD now shows surface-mode clearance to centimetre
precision instead of rounding it to zero. In surface mode the HUD also reports the sampled wave
surface elevation separately, so clearance is not confused with the wave's absolute height.

## Storm-water CPU/GPU scale and buoyancy correction - 2 September 2026

The rendered ocean's calm/storm displacement scales are 44x/55x, while the CPU collision
path had retained the older 4x/25x values. That mismatch allowed a visible crest to pass over
the camera while the HUD still reported positive clearance. CPU constants now match the WGSL
surface (the six-wave storm bound is 52.6625m, conservatively rounded to 53m), with a focused
shader-scale regression. While submerged, a bounded restoring buoyancy term supplements
Archimedes and water-relative drag; it draws the eye toward the same 0.255m still-water
equilibrium without pinning it to the animated wave.

The local ocean patch also carries three shorter geometric ripples (180m/70m/28m). The CPU
surface query now mirrors their vertical height and analytic velocity at the camera-centre,
not just the broad swell, so the camera follows the rendered near waves. Ground contact is
disabled while a water surface exists; even very shallow coastal water remains swimming mode
and a jump receives water buoyancy rather than silently becoming a land jump.

For the current visual comparison, `WATER_BOBBING_ENABLED` is temporarily `false`: in water the
camera is placed at a fixed 1.0m above the sampled wave every substep, with vertical velocity
held at zero. This isolates whether the camera follows the animated rendered surface; restore it
to `true` when evaluating buoyancy, jumps, and bobbing again.

The diagnostic also disables Gerstner horizontal transport and the sub-mesh ripple octaves in the
ocean shader. Broad vertical swell and its lighting remain active, but both camera and mesh then
sample the same world-space height instead of comparing a radial CPU sample with horizontally
displaced or under-resolved parametric vertices.

## Ocean wind sea and a wave-following waterline diagnostic - 3 September 2026

The six-component ocean carried one Gerstner wave per wavelength band, so each band rendered as a
single perfect plane wave and the sea read as one big regular wave plus one small regular wave.
`WAVES` and its `OCEAN_WAVE_TABLE` mirror in `shared_planet.wgsl` now hold seventeen components:
the two 1,400m swells, a 280-430m storm sea, and a twelve-component wind-sea tail spread widely in
azimuth, which is what actually breaks the crests up. Each entry carries a calm and a full-storm
amplitude, so a storm moves the dominant band down from the 1,400m swell rather than scaling the
calm sea up; both columns sum to the same total, and the 53m height cap therefore holds at either
end and everywhere between. One test asserts that equality and another mirrors every axis,
wavelength, amplitude and speed literal between the CPU table and the shader, because a GPU-only
amplitude edit would leave collision following water the renderer had stopped drawing.

`OCEAN_LARGE_SWELL_ONLY` keeps only the leading swell pair and silences both the wind sea and the
whole local ripple layer. It is paired across `ocean.rs` and `shared_planet.wgsl` and the pairing
is enforced by a test, so collision loses exactly the waves the render loses.

A flat sea rendered as a field of fixed tents locked to the tessellation. `project_patch_vertex`
formed the tangential part of a patch vertex as `surface_cube - anchor_direction * parallel`,
differencing two O(1) vectors to produce an O(1e-5) one and then scaling the result by the planet
radius: about 0.2m, one f32 ULP at 4,000km, which is enough to tilt a 1m L18 triangle by degrees.
It now differences only quantities of the same small magnitude. The ocean fragment path compounded
this by deriving its face normal from `cross(dpdx, dpdy)` of a camera-relative position, which
loses the same difference to cancellation on the stretched triangles the LOD morph produces. That
normal is now a flat-interpolated vertex attribute taken from the wave field at the provoking
vertex: still a face normal, so the broad wave facets this presentation depends on survive, but no
longer dependent on a screen-space derivative that collapses at grazing angles.

The same stable decomposition must not be applied to the shared-boundary branch of that function,
and a comment there now says so. That branch exists so both neighbours of an edge vertex evaluate
it identically. `(direction - anchor_direction) * PLANET_RADIUS_METERS` is the less accurate form,
but its anchor term cancels exactly against the anchor world position the renderer adds back, so
both chunks land on the same `direction * PLANET_RADIUS_METERS`. The anchor-local form is
evaluated relative to each chunk's own anchor, so two neighbours round a shared vertex differently
and the seam splits; it opened a one-pixel gap straight through the ocean at 11.3km. On a shared
boundary, agreement beats accuracy.

`ocean_waterline_flat` found that gap, but only after being repaired itself. It authored the eye
as an absolute waypoint radius, which at storm intensity 1.0 sat 24.4m below the surface, so its
captures framed the underside of the water shell and nine of eighty-one probe rays hit anything at
all. Scenarios can now set `waterline_eye_height_meters`: the waypoint supplies the ground track
and the view direction, and `waterline_scenario_pose` rides the eye that far above the CPU wave
surface there, sampling open-ocean depth rather than streamed bathymetry so the framing does not
depend on what happens to have loaded by the capture time. The height is 40m rather than eye
level, because at 1.5m in a storm the nearest crest is 13m away and fills the 1.1-degree frame on
its own, and the horizon this scenario exists to magnify is then never in shot.

The gap read as a single row of pure sky spanning all 1,280 pixels, 54 rows below the horizon. It
held a fixed 11.32km from the camera across both captures, moving eight pixels as the eye dropped
2.54m while the horizon moved four, which is the ratio a fixed-distance ring must show and a wave
feature cannot. It survived `OCEAN_WAVES_ENABLED = false`, which exposed a second gap at 8.0km,
and it disappeared entirely under the previous `planet.wgsl`. Reverting only the boundary hunk
closes both while leaving the interior precision fix active, which the horizon row confirms by
staying one pixel away from where the old projection put it.

The ocean has no seam instrument of its own. `max_seam_delta_m` compares baked outmap tile heights
across chunk boundaries and cannot see a gap in analytically displaced ocean vertices, so this
class of defect is only visible in a magnified waterline capture.

## Floating ship, breaking shore waves, and one knob for the sea - 3 September 2026

The flat presentation quilted the ground along every chunk boundary. It was not the source tiles or
the LOD streamer: rendering the same view with ordinary shading shows no grid at all. The terrain
flat path took its normal from `cross(dpdx, dpdy)` of a camera-relative position, the same construct
that made the sea render as fixed tents, and it fails the same way once the difference is lost to f32
cancellation. It now shades from a flat-interpolated face normal instead. That normal is packed into
the spare components of `@location(13)`, already the only `@interpolate(flat)` scalar slot, because
the terrain output sits exactly on the Quadro's sixteen-location limit. The squares were always
there; removing the corrugation is what promoted them to being the most visible thing in frame.

`OCEAN_WAVE_SCALE` in `ocean.rs` is now the only place the sea's size is written down. The calm and
storm amplitude scales, `MAXIMUM_WAVE_HEIGHT_METERS` and the steepness that keeps waves from folding
through themselves all derive from it, and `ocean::wgsl_constants` generates the shader's copies.
`planet::shared_planet_shader_source` prepends them to `shared_planet.wgsl`, and both real
assemblers -- the raster `planet_shader_source` and the raymarch `raymarch_shader_source` -- go
through it, so the two render paths cannot disagree. A test asserts the raw file does not declare
those constants again, since a second copy would put the drift straight back.

Steepness is derived as `1 / OCEAN_WAVE_SCALE`, which holds the Gerstner self-intersection budget
`sum(steepness * amplitude * wave_number)` invariant: a taller sea is no longer automatically a
steeper one. That budget stands at 1.17 against a physical limit of 1.0, so the sea already folds;
the knob now holds it there rather than letting it grow, and lowering it remains a deliberate visual
change. Two guards run before the shader mirroring, so an out-of-range knob is reported before
anything is edited: the budget must not move with the scale, and the camera's underwater floor must
stay below the deepest trough. That floor is the knob's real ceiling, around 1.8 at -100m.

Waves no longer fade out at the coast. `shore_wave_weight` used to taper them away between 30m and
2m of depth, which deleted the sea exactly where it should be liveliest and left the shallows
reading as static geometry. `breaking_weight` limits instead: the summed crest is squeezed toward
`0.39 * depth` by a soft-max knee, so it flattens off as it shoals, which is what breaking is.
Because the result can never exceed the limit, the surface cannot cut through the sea bed either --
the limit reaches zero exactly where the water does. Rates of change of a limited height take the
knee's own derivative, not the height's factor; using the wrong one leaves the analytic velocity
disagreeing with a finite difference of the height, which the existing derivative regression caught.

The knee was a `tanh` first, and a `tanh` bites everywhere: it took 15% off a 52m crest in 200m of
water, where a wave that size is nowhere near breaking. The camera and the renderer both use the
real bathymetry, so both were on that shortened sea, while the hull had a hard-coded 4000m and was
on an unlimited one -- the hull floated correctly on water the eye had already sunk through. The
hull now reads the same bathymetry, and the soft-max leaves anything well under the limit alone.
The swimming regression only ran at 4000m, where the limiter is inert and could not have seen this;
it now sweeps the couple of hundred metres an actual coast has.

A first attempt scaled amplitude in proportion to depth. It kept waves off the bed but made shoaling
impossible: crest and limit shrank together, so their ratio was identical at 1m and at 120m and
nothing ever broke. Foam then has to key off the raw crest rather than the limited one, which by
construction can never exceed the limit; `OceanSurface` carries `breaking_ratio` out for exactly
that reason. Shoreline shading keys everything off depth -- a shallow colour ramp, foam on crests
past what the depth holds, and a wash where there is barely any water left -- so every coast gets
the same treatment with nothing authored per location.

A low-poly ship floats on that sea. `ship.rs` owns the hull form, an eighty-column buoyancy
discretisation and the rigid body, GPU-free and tested like `surface_camera`; `ship_render.rs` owns
the pass. Mass comes from the same columns that provide buoyancy, so the design waterline is an
exact equilibrium rather than a tuned guess. Two failures were worth the tests that caught them:
immersion measured as a world-vertical depth while the prism stayed in the hull frame cost a heeled
hull `cos(phi)` of its displacement, so it sank and rolled to 179 degrees; and the centre of
buoyancy sits aft of the origin on this form, so weight acting at the origin trimmed the hull 3.5
degrees and settled it 3cm low from a balanced start. Real hulls are ballasted so weight sits over
buoyancy, and that offset is now explicit.

The hull rolls, pitches and yaws because buoyancy acts normal to the sloped water surface rather
than along the radial. `ocean::global_wave_slope` supplies that tilt analytically, checked against a
centred finite difference. A radial force has no moment about the vertical axis, so before this
nothing in the model could yaw the hull at all. Metacentric height is set directly at 0.9m rather
than falling out of where the mass happened to sit: at the centre of buoyancy GM is the whole
metacentric radius, 2.8m here, and the hull is far too stiff to roll. Roll also needed eddy damping,
which a vertical-prism model has none of and which is why real hulls carry bilge keels.

`WATER_BOBBING_ENABLED` is `true` again. Restoring it re-enabled Gerstner horizontal transport as a
side effect, because `flat_triangle_options[2]` was derived from it -- one flag doing two unrelated
jobs. Transport slides the rendered mesh up to 54m sideways while the CPU height query is purely
radial, so the camera followed a surface the GPU had moved. `OCEAN_HORIZONTAL_TRANSPORT_ENABLED` is
now its own constant, with a test that fails if it is ever enabled while the CPU query has no
horizontal term.

Buoyancy alone still could not keep the eye up: this sea's crests accelerate downward at close to g
and overtake a floating body, leaving it submerged 41% of a storm, and twenty times the restoring
force only reached 23%. Bobbing therefore has a floor rather than a stiffer spring. Buoyancy still
drives the motion, but the eye cannot end a substep below the water, and a crest that overtakes it
carries it up at the surface's own speed. This is a camera concession, not physics; true immersion
would want the underwater rendering that is still unimplemented.

`descent_to_10m` fails on the terrain streamer, and did so before any of this: LOD peaks at 14
against a required 18, with 256 fallback chunks against an allowance of 128, and `tiles_loaded` stays
at zero across twenty seconds. Not investigated.

## The camera followed water nobody drew - 3 September 2026

The eye kept going under near the ship. Its surface probe reported positive clearance the whole
time -- 0.31m, 0.43m, 0.67m -- while the rendered surface 13m away stood 2.2m above the camera. The
CPU and the renderer were describing different seas.

`ocean_surface` hands the short ripple octave out as `ripple_height` for colour and `ripple_slope`
for the normal, and `vs_ocean` adds neither to `local_planet_position`. The ripples are a shading
detail, not a surface. The CPU's `local_wave_height_meters` added them anyway, so the camera floated
on up to 4.6m of water that was never drawn and dropped under crests it could not see.

This is as old as both files; it is not new work. What is new is which query the camera asks.
`WATER_BOBBING_ENABLED = false` placed the eye above `global_wave_height_meters`, which is exactly
what the renderer displaces, so the fixed diagnostic never touched the mismatch. Restoring bobbing
switched the camera to the local query and exposed it.

`OCEAN_RIPPLES_ARE_GEOMETRIC` now states which of the two is true, and the camera's surface query
follows it. A test reads `vs_ocean` and fails if the flag and the shader disagree, so making the
ripples geometric later means flipping one constant and being told immediately if the displacement
was not added with it.

## Wave refraction at the shore - 3 September 2026

Surf ran whichever way the wave table happened to point, so whether it arrived onshore was an
accident of how a coast faced. `OCEAN_WAVE_PHASE_SPEED_SIGN` makes the travel convention explicit
and flippable -- phase is `wave_number * (dot(direction, axis) * R + sign * speed * time)`, so a
crest travels against the sign -- but reversing it only trades which coasts are lucky.

Refraction is the actual fix and needs no per-coast authoring. Waves slow as they shoal, so the part
of a crest in shallower water lags; on an oblique approach that lag varies along the crest and it
turns until it lies along the depth contours. `refraction_lag_meters` subtracts that lag distance
from the travel term, which reproduces the turn from the depth field alone with no ray tracing.
Deep water returns exactly zero, so nothing offshore is disturbed. Measured on a straight coast, a
swell arriving 35 degrees off square holds 35 to 100m of depth, then closes to 19.9 at 75m, 15.0 at
25m and 6.5 at 2.5m.

The lag is a function of depth only, so a query given a constant depth sees no extra spatial
gradient and the analytic slope stays exact for it. Over a real varying bed the true slope carries a
depth-gradient term this omits; that matters for the hull's yaw forcing only where bathymetry
changes appreciably across 42m of ship, which it does not offshore.

## The bottom steers the swell ashore - 4 September 2026

Refraction bent crests onto the depth contours but could not turn a swell around, so coasts facing
away from the wave table's directions still got surf running out to sea. Measured on four coasts by
cross-correlating nadir frame sequences and dotting the pattern's displacement with the onshore
direction from the bathymetry: +1.00, +0.70, +0.44 and -0.85. The last is a coast whose swell
arrives 170 degrees off square, and it was unambiguous at 0.91 correlation.

Re-aiming the wave table cannot fix it. Over 897 coasts of this bake the current axis serves 51.8%
of them and the best fixed axis on the whole sphere serves 52.7% -- which is what a sphere gives
for free. The steering has to come from the local bed.

`shoaling_phase_offset_meters` replaces the refraction lag. Its gradient runs along the depth
gradient with magnitude `1 / REFRACTION_NOMINAL_SHELF_SLOPE`, so once that outweighs the wave's own
unit heading the propagation follows the bottom rather than the axis: crests still turn onto the
contours, and the swell now arrives from seaward whichever way a coast faces. The offset is bounded
and its slope reaches zero exactly at the reference depth, so the open sea is untouched. Same four
coasts after: +0.91, +1.00, +0.95, +0.97.

The nominal slope is 0.010, deliberately gentler than a real shelf so the steering dominates. At
0.020 it only bends: the 170-degree coast went back to -0.44.

Aligning every component onto the contours makes them superpose coherently, so the shallows break
much harder and the surf zone whitened to 14.4% of the frame. Raising the foam onset did nothing,
because past the shore the raw crest is many times what the depth holds and the ratio saturates
across the whole shelf. Surf is a band: the foam now fades out again once a crest is far past
breaking, on the grounds that the water behind a broken wave is spent. 2.7% after.

## The spawn coast was the gentlest shelf - 4 September 2026

Surf still ran seaward at the F4 spawn after the bed steering went in, and measuring there rather
than arguing from the four coasts already in the table found why. Steering strength is the phase
offset's derivative times the bed slope, and it has to exceed the wave's own unit heading. That coast
falls 38m in 5km, a slope of 0.0076, gentler than any of the four the constant had been tuned
against; the steering came to 0.68 and the swell kept its authored heading. Measured -0.93 at 0.87
correlation.

`REFRACTION_NOMINAL_SHELF_SLOPE` is 0.0045 rather than 0.010, which carries the gentlest shelf with
headroom. All five coasts now run ashore: spawn +0.99, and +0.63, +1.00, +0.66, +0.37 for the
original four.

The guard is the lesson rather than the number: steering has to dominate on the *gentlest* shelf on
the planet, not a typical one, so a test asserts it through the surf zone against a 0.0076 slope.
Only through the surf zone -- the steering fades with depth deliberately, which is what leaves the
open sea running where its axis points, so demanding it dominate at 50m would demand the wrong thing.

## Why the weather stopped changing - 4 September 2026

Clouds thinned to nothing after a while and the field went static. It was not an equilibrium; the
simulation was running down and could not come back.

Evaporation is gated on temperature -- `((T - 245) / 35).clamp(0, 1)` -- and it is the only source
of moisture in the model. Everything else moves, condenses or rains it out. So once the field falls
below 245K the moisture can only decrease, and the weather is a one-way ratchet from there. The
reported field was `T 180.0-340.0K (mean 185.2)` against a clamp floor of 180, meaning nearly every
cell was pinned at the bottom, with cloud cover falling 0.13 to 0.05 across sessions.

Instrumenting each stage of the step for what it does to the area-weighted mean found the leak at
once. Over four weather-days radiation added 0.85K, lapse 0.28, condensation 1.26, evaporation
-0.38, and temperature advection took away 24.00. Advection only stirs; it has no business changing
the mean at all. The scheme was a semi-Lagrangian upstream sample with a clamped MacCormack
corrector, which conserves nothing.

Reusing the conservative donor-cell transport humidity already uses made it far worse -- 105K in the
same four days -- because that moves absolute `value * area`, which suits a mass fraction and ruins
an intensive quantity: a cell receiving from no upwind neighbour sheds a twentieth of 250K per step
until the clamp catches it, and the clamp is where conservation dies. Temperature now trades a
bounded share of its *difference* from the cell downwind, so what leaves one enters the other
exactly. Advection contributes 0.00K over 3000 steps, and the field holds 264.8-267.6K across 20.8
weather-days instead of collapsing.

The day/night cycle is still lopsided: the planet turns once per 300 real seconds while weather runs
at 3600x, so one rotation is 12.5 weather-days and every cell sits in darkness for over six. That
was a suspect before the measurement and is not the cause, but it is why the temperature range
spans the whole clamp.

## One rotation, one weather day - 4 September 2026

The two clocks were tuned independently: weather ran at 3600x real time while the planet turned once
per 300 real seconds, so a rotation took 12.5 weather days and every cell sat in darkness for over
six of them. That had been set to make the weather visibly move while it was quietly running down;
with the advection leak fixed it only skewed the day.

`INTERACTIVE_WEATHER_TIME_SCALE` is derived now, from `WEATHER_DAYS_PER_PLANET_ROTATION` and the
rotation itself, and comes out at 288. A test asserts one rotation advances the weather exactly one
day, and another that a day divides into whole steps -- otherwise the sun and the field drift apart
by a fraction of a step every rotation.

Two constants had been tuned against the old scale and had to move with it, both now expressed as
the rate they were really about rather than as a step length. Cloud microphysics ages sixty
weather-seconds per real second regardless of the transport clock, so its test asserts that rate
instead of a 10-second step. Cloud detail drift is tuned per real second and divided through the
weather scale, so tying the clocks did not silently slow the drift by the same 12.5x.

The corrected microphysics step also made evaporation far more active within each transport step:
moisture now oscillates 0.59-0.69 with no trend across 20.8 weather-days, where before it fell away.
Mean temperature holds 264.8-267.5K over the same span.

## A time-speed ladder, and making it safe to use - 4 September 2026

Comma and period step a nine-rung ladder from 10% to 4000% of real time. Everything scene-side reads
one scaled clock -- `scaled_clock_seconds`, accumulated per frame rather than derived from elapsed
real time -- so the planet's rotation, the ocean, the weather and the hull all speed up together and
the derived relationships between them hold at every rung. Accumulating rather than scaling elapsed
time is what stops a speed change rewriting the session retroactively and jumping the scene.

Acceleration only made sense once the accumulating simulations stopped depending on the frame rate,
which they all did:

The weather kept a bounded number of steps per advance but *discarded* the surplus, so the state
depended on how many frames you happened to get. It now carries the remainder, with the backlog
itself bounded to one advance's worth so a stall cannot trigger an enormous catch-up. Its test
asserts both halves: twelve steps now, the rest worked off by the next ordinary frame, and cleared
in two rather than eighty-eight.

The hull and the surface camera both sub-stepped with a *partial final step*, so the same span of
time integrated differently depending on where frame boundaries fell -- and acceleration widens
exactly those frames. Both now integrate whole fixed steps and carry the remainder on their own
clocks, and the hull samples the water at each step's own time rather than the frame's end. The
hull's step count is computed rather than subtracted down: `remaining -= step` leaves a femtosecond
that runs as an extra micro-step, which was enough to make one call of eight steps disagree with
eight calls of one.

At the top rung and 30fps the weather owes 384 weather-seconds a frame against a cap of 7200, so the
cap is a stall valve rather than a speed limit. A test asserts that headroom alongside the ladder.

## A twenty-minute day, and CI back to green - 4 September 2026

The planet turned once per 300 real seconds, so the sun crossed the sky faster than the sea it was
lighting. `INTERACTIVE_DAY_REAL_SECONDS` is 1200 now and the rotation scale derives from it, which
reads as what it is instead of a bare 0.05. Weather takes its own clock from the rotation, so it
followed automatically: one rotation is still one weather day, now at 72x rather than 288x real
time. At the top of the time-speed ladder a day passes in 30 real seconds.

CI had been failing since before this branch, and nobody was pushing so nobody was told. Two
separate causes, and they needed separating before either could be fixed:

The formatting failures were this session's -- none of these edits had been through `cargo fmt`,
which is the first thing the workflow runs. It was clean at ffed751.

The clippy failures were not. The lint list is identical at ffed751 and at HEAD: 70 in the app
crate either way, older code meeting newer lints. They had also been invisible, because clippy stops
at the first crate that fails and the baker failed with three, so the app was never linted at all.

All 73 are cleared. Most were mechanical, but three decisions are worth recording. Eighteen
assertions comparing constants are now `const` blocks, so they are checked at compile time rather
than asserted against something the optimiser already knows -- two lost their formatted messages,
which const cannot do, and say the same thing in a comment. Eleven `too_many_arguments` are allowed
at the crate root: pipeline and pass plumbing carries many genuinely distinct values, and bundling
them into structs to satisfy an arity count would hide what each call site passes. One assertion
compared two literals and is gone entirely; the check that earns its keep is the one above it, that
the shader still contains the number.

## Walking on ground that was not there - 4 September 2026

On a 20km mountain the camera stood 11.7m above the surface being drawn. It reported
`Clearance: 1.70 m` and `grounded` the whole time, and moved when walked, so nothing in the HUD
said anything was wrong -- it was walking on air a dozen metres up.

Two hypotheses went first, both wrong and both worth ruling out by measurement rather than reading:
the ground query returns the same height at every camera altitude from 2m to 60km, so it is not
limited by height; and the slopes there are 13.8 degrees at worst with all sixteen sampled
directions inside the 42-degree walk limit, so the slope rule was not blocking anything either.

`raster_surface_height_meters_at` returns the greater of two figures: a continuous sampled height
field, and the height of the triangle the renderer actually drew. The second is the authoritative
one, and it was returning nothing at all -- on every chunk under the camera. Near-field patches are
addressed through a window grid rather than one source tile, and the branch that builds them dropped
the tile key. The instance stream nulls that key again a few lines later for its own draw grouping,
so the only consumer of the earlier `None` was `SurfaceDetailNode`, whose key the mesh query needs.
With the key kept, the mesh query resolves and reports 20525.03m against a drawn 20525.68m.

The `max` was then compensating for that failure. With the mesh query working it stands the camera
on the continuous field wherever that field sits above the drawn mesh, which on steep ground is
metres of thin air, so the drawn mesh now wins outright where it is available and the sampled field
is the fallback for directions the mesh does not cover. The gap at that spot went from 11.74m to
-0.65m: standing on the ground rather than over it.

The surface probe still reports about 11m between the rendered mesh and the sampled field, because
that is what it compares and the two genuinely differ that much on steep near-field terrain. It is a
separate observation from what the camera stands on, and it is worth its own look.

## What the surface probe is actually measuring - 5 September 2026

Four baseline scenario runs at `342c3b5` were captured but not analysed. Their manifests carry the
full per-point `surface_probes` data, and reading it changes what open thread 1 says.

**Two of the three ocean runs measure nothing about terrain.** Every probe point in
`ocean_hybrid_close` (45 of 45) and `ocean_rough_horizon` (9 of 9) has `cpu_height_meters` of exactly
0.0. The probe differences the drawn surface against the CPU's sea-level-resolved height, which over
open water is zero, so the "delta" it reports there is just the rendered wave height: -26.5m to
+33.1m in the calm run, +5.1 to +6.9 in the storm one. Those figures should not be quoted as a
mesh-versus-field disagreement. Nothing arms `max_surface_probe_delta_m` on any of these scenarios,
so the number is recorded and asserted by nothing; if it ever were armed on a water scenario it would
fail on wave height alone.

**`coast_waters_edge` is the one that measures land**, all 36 points, and it disagrees far more than
the 11m previously recorded: median 2.27m, p90 35.09m, max 43.58m, from a camera 308.5m up.

The distribution is bimodal rather than a uniform offset. Of 31 near-field points inside 2,000m,
fourteen agree to better than 1.3m and seventeen are off by 9.5 to 43.6m, and the two groups
interleave across the screen grid -- adjacent samples at the same `ndc_y` land on +0.43m and +43.58m.
So it is not a chunk, a region, or a constant bias.

What separates them is the height the *renderer* draws, not where the CPU thinks the ground is:

- `|delta|` against `rendered_height_meters`: **+0.714**
- `|delta|` against `cpu_height_meters`: -0.220
- `rendered_height_meters` against `cpu_height_meters`: +0.526
- failing points draw a median 45.18m where the CPU has 16.59m; agreeing points draw 26.77m

Every near-field disagreement has the renderer *above* the CPU; there is no large negative. Past
2,000m the sign flips and settles at a uniform -1.84 to -2.28m across all five far points, which is a
separate regime and probably the macro-only LOD.

`delta_meters` and `delta_from_macro_meters` track each other closely (+41.26/+43.18, +43.58/+47.19,
+22.74/+22.56), so the CPU's detail term contributes almost nothing here -- the CPU is effectively
returning macro while the renderer draws tens of metres of detail on top of it.

`detail_correlation` is 0.3222 with `detail_slope` 1.5162. A correlation that low is the signature of
two *unrelated* fields, not one field at the wrong gain; a gain error would hold correlation near 1
and put the error in the slope. That points at the CPU and GPU detail functions being fed different
inputs rather than scaled differently, which is the same failure mode recorded for `fract`-folded
fields elsewhere in this document. It is a lead, not a diagnosis: nothing here has yet been traced to
a specific input.

Every probe in all four runs is `render_path: raster`. None of this says anything about the raymarch
path, and the two are meant to be at parity.

`highest_prominence_peak` also fails, and has been failing for at least four commits -- `342c3b5`,
`ffed7512`, `95e58cfd`, `cfbee5a2` -- on `camera_stands_on_the_ground`: bounds 150.0..=155.0m,
observed 7,659.857m over two frames. Its probe reports `compared 0`, which is consistent with being
7.6km above the terrain rather than a second fault. It is a stale authored waypoint or a placement
failure, and it is not caused by the recent ground work.

## Chasing the detail mismatch: what it is not - 5 September 2026

`ProbeComparison` now records the planet-frame `direction` of each point. It carried `ndc`, which
identifies a pixel in one capture rather than a place on the planet, so a failing point could not be
re-evaluated without re-running and hoping the same pixel landed on the same ground. A fresh
`coast_waters_edge/1788607228-73179` at `509f1b0` carries the directions.

The useful number from that run: the CPU's own detail contribution, `cpu_height - cpu_macro`, is
1.0-3.9m at *every* point -- agreeing and failing alike. It is not that the CPU's detail vanishes
where the two disagree. The renderer is putting 45m of relief on ground where the CPU's model puts
four, so the question is why the CPU's ladder is so much smaller there, not why it is absent.

Checked and found to agree, so not the cause:

- `GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE` is 0.0 on the CPU and is uploaded in
  `outmap_height_scale.z`, which no shader reads; no shader implements the global direction-noise
  field at all. Off on both sides.
- `TERRAIN_DETAIL_FILTER_RATIO` is 0.01 on both. `TERRAIN_DETAIL_MIN_FILTER_METERS` is 0.5 on both
  (the shader reaches it through `TERRAIN_NORMAL_MIN_SAMPLE_METERS`).
- Both filters are `max(distance term, edge term)` with the same edge-stitch fade.
- `terrain_detail_octave_headroom` is the same smoothstep against the same
  `TERRAIN_DETAIL_HEADROOM_FACTOR` on both.
- The shader's `blend_source_edges` early return looks like an asymmetry -- the CPU has no such
  parameter and always runs the fade loop -- but it is an optimisation: the bit is set exactly when
  `node_intersects_source_edge_fade`, and where it is clear every point is far enough from a tile
  edge that the CPU's loop drives `effective_level` to `source_level` anyway. Both branches agree.
- The shader's anchor/local split reconstructs the CPU's domain exactly, because
  `terrain_detail_domain` is three dot products and therefore linear:
  `domain(anchor + local/R) * R/wavelength` is `anchor_cells + local_domain/wavelength`, which is
  the expression the shader evaluates. `terrain_detail_value_noise` carries a fraction past 1 into
  the integer cell, so a large local offset is handled. The f32 quantisation of `anchor_cells` at
  1e6 magnitude does shift fine octaves by a fraction of a cell per chunk, but fine octaves carry
  metres, not tens of metres, so it cannot account for 43m.
- The shader's `active_octaves` loop bound only skips octaves whose in-loop fade is already zero,
  so it cannot drop an octave the CPU includes.

**The strongest remaining hypothesis, and the instrumentation it needs.** The CPU picks its
`SurfaceDetailNode` through `surface_node_index.for_each_at_direction` and takes the highest
filtered surface among coexisting transition patches. That node's `level` sets `node_spacing`, which
floors `surface_detail_filter_meters`. If the CPU resolves a coarser node than the chunk the GPU
actually drew, its filter is larger, its low cut retires octaves the shader kept, and its ladder
comes out at a few metres against the renderer's forty -- with no constant ratio between them, which
is what a 0.3222 correlation at slope 1.5162 looks like. The bimodality fits too: agreement would
then depend on which patch the CPU resolved, and the probe's agreeing and failing points interleave
across the screen rather than forming a region.

The probe records `cpu_source_level`, which is the baked *tile* pyramid level and is 4 everywhere
here because `dense_level` is 4. It does not record the CPU's chosen node level or the
`filter_meters` it used, which are the two numbers that would settle this. Recording them on
`ProbeComparison` is the next step; the GPU's `requested_level` is already in `terrain_info` and can
be reconstructed for the same point.

## The surface probe was measuring trees - 5 September 2026

The detail mismatch is not a terrain disagreement. It was tree canopies in the depth buffer.

`probe::schedule_depth_readback` ran after the pass that draws ships, forest, weather clouds and
rain. All of them load and store the same depth attachment the terrain wrote, so the probe read the
nearest *object*, not the ground. Trees are 22-48m tall (`TREE_HEIGHT_MIN_METERS`,
`TREE_HEIGHT_RANGE_METERS` in `forest.rs`), which is the whole observed range of the disagreement,
and a canopy is always above the ground it stands on, which is why every near-field delta was
positive and there was never a large negative.

Measured on `coast_waters_edge`, raster, at three configurations of the same frame:

| run | compared | median | p90 | max | `detail_correlation` | `detail_slope` |
|---|---|---|---|---|---|---|
| before, forest on | 36 | 2.255 | 35.092 | 43.578 | 0.3222 | 1.5162 |
| before, `CATINGARDEN_FOREST=0` | 36 | 0.560 | 1.835 | 2.277 | 0.9586 | 0.9255 |
| after the fix, forest on | 36 | 0.560 | 1.835 | 2.277 | 0.9586 | 0.9255 |

The middle row is the control and the third is the fix reproducing it exactly. Between the first two
runs, nineteen probe directions are common to both and every one returns an identical rendered
height to the last recorded digit; the seventeen that differ are the ones with a tree in front.
After the fix all 36 directions match the forest-disabled run to 0.000000m. Near-field (inside 2km)
median |delta| falls 9.540m to 0.428m and max 43.578m to 1.418m.

The 0.3222 `detail_correlation` recorded as "the signature of two unrelated fields" was that: a
canopy height set against a ground height. With trees out of the depth the two ladders correlate at
0.9586 with slope 0.9255. They were never the two unrelated fields.

The raymarch path had the same defect and the same fix: `coast_waters_edge` under
`CATINGARDEN_RENDER_PATH=ray` now returns identical numbers with the forest on and off, 70 and 71
directions shared and 0.000000m apart (median 1.293m/1.172m, p90 3.880m/4.308m). Its CPU truth is
`surface_height_breakdown_at`, the continuous field with no node, so its residual is not comparable
with raster's.

**What the fix is.** Two depth readbacks instead of one. The surface probe asks where the *ground*
is and now snapshots the depth before the ship/forest/cloud/rain pass. The haze probe asks whether
distance reads on the frame that was captured, and bins each pixel's colour by that pixel's own
depth, so it keeps the post-pass depth -- taking the earlier one would have paired canopy colour
with the distance of the ground behind it. Its convergence is 0.414 before and after, against 0.3362
with the forest disabled, so it is measuring the finished frame either way. A test in `probe.rs`
pins the two readbacks to their own sides of that pass.

**Ruled out along the way, by measurement.** Two hypotheses from the previous section, both wrong:

- *The CPU resolves a coarser patch, so its filter retires octaves the shader kept.*
  `ProbeComparison` now carries `cpu_node_level`, `cpu_detail_filter_meters` and a `filter_sweep`:
  the CPU height re-evaluated at every drawn-patch level L2-L18, holding the source tile fixed.
  At the first failing point the entire sweep spans 2.79m to 3.15m across filters from 62.5km down
  to 3.37m, against a rendered 25.53m. Across the sixteen failing points the best rung closes a
  median 0.5% of the gap and 38% at its very best. Filter width cannot produce tens of metres.
- *The near-field window reads finer tiles than the single ancestor the CPU samples.* It can in
  principle -- the window resolves a source per block and `near_field_coverage` counts blocks finer
  than dense. `near_field_window_sample_at` reports what it actually holds, and at every one of the
  36 points its source level is 4, the same as `cpu_source_level`, and its macro height equals
  `cpu_macro_height_meters` to the recorded digit. Both sides were always reading the same L4 data.

The instrumentation is kept. It is what makes the next disagreement attributable instead of
observed, and it is gathered through its own closure so the per-frame clearance query carries none
of it.

**Unchanged by this, and still open.** `mountain_ground` is bit-identical before and after at median
11.433m, p90 12.236m, max 12.660m over 81 points -- a tight, near-uniform offset with none of the
bimodal spread trees produce. That is the real mesh-versus-field gap on steep near-field terrain and
it still wants its own look. `highest_prominence_peak` still fails `camera_stands_on_the_ground` at
7,659.857m over the same 150-155m bounds, and `landing_site_eye_level` still fails it at -753.0m;
both read `camera_surface_height_meters`, a CPU query that never touches the depth readback, so
neither is affected by this change. `stand_on_ground` still compares 0 points, as it has since
`1787411515`. All 428 workspace tests pass; `cargo fmt --check` and `cargo clippy --workspace
--all-targets` are clean.

## Four ground scenarios were measuring nothing - 5 September 2026

`stand_on_ground` compared 74 points at median 0.104m as recently as run `1785674116`. It then
compared **zero**, and had since `1787411515`. Its failure line said so plainly and nobody read it:
`maximum observed 0m over 0 points in 5 frames, violations 0`. An assertion reporting no violations
over no points is not passing, it is asserting nothing.

The cause is the same in all four cases and it is not subtle. The landing site at direction
`(1, 0, 0)` now has ground at **4,391.075m**. The poses were authored when it was around 3,635m, and
never moved when the outmap was rebaked. So the cameras were 687-753m *underground*, every drawn
surface they could see was 2.4-34.8km away, and the probe's 4km comparison limit discarded all of it.
`highest_prominence_peak` was the same failure at the other end of the planet: authored at a summit
the bake has since moved away from, leaving the camera 7,659m above ground 69 degrees of longitude
from the actual peak.

| scenario | before | after |
|---|---|---|
| `stand_on_ground` | clearance -753.319m, 0 compared, FAIL | clearance 2.000m, 400 compared over 5 frames, median 0.317m, p90 0.592m |
| `landing_site_eye_level` | clearance -753.019m, 0-2 compared, FAIL | clearance 2.000-62.586m, 274 compared, median 0.389-1.395m |
| `landing_site_ground_detail` | clearance -687.129m, 0 compared, **passing** | clearance 68.190m, 152 compared, median 1.285m, p90 2.736m |
| `highest_prominence_peak` | clearance 7,659.857m, 0 compared, FAIL | clearance 152.402m, 10 compared |

`landing_site_ground_detail` is the one worth dwelling on: it had no clearance assertion at all, so it
sat 687m inside a mountain and reported success for months. It has a 60-75m band now. That band is
authored around the 68.190m the restored pose measures, not derived from anything.

**The lockstep test was a tautology.** `highest_prominence_scenario_replays_the_f4_start_pose`
asserted the scenario file's own latitude, longitude and radius back at itself, so it passed happily
through every rebake while the pose drifted off the summit. It now *derives* the expected position
from `ACTIVE_HIGHEST_PROMINENCE_DIRECTION` and the drawn-surface constant, so the next bake that
moves the summit fails there, beside `global_highest_summit` which prints the replacement constants.
That instrument also wanted `ACTIVE_HIGHEST_PROMINENCE_METERS` moved 186,701.172 to 186,709.142; it
passes now.

**An open discrepancy this turned up, recorded rather than resolved.** At the summit direction the
survey instrument reports 186,709.142m and the app's own surface query reports **186,936.495m**, a
gap of 227.353m with the app higher. The app's reading is stable across altitude -- 186,936.5m near
the ground converging to exactly `raw_macro * 4` = 186,941.266m by 36km up -- so it applies almost no
detail, while the survey computes -232.1m of it at the 0.5m minimum filter on L4 data. The likely
cause is the ladder's high cut, `baked_spacing_meters`, which retires octaves longer than the source
spacing: the survey scans L4 only, the app resolves whatever finer tile is resident. **That is a
hypothesis and nothing has measured it** -- the probe compares zero points at that pose. The scenario
pose is derived from the drawn surface, `ACTIVE_HIGHEST_PROMINENCE_DRAWN_SURFACE_METERS`, because
that is what `camera_stands_on_the_ground` measures; deriving it from the summit put the camera 75m
inside the mountain.

**Guards added.** `min_surface_probe_points` on the three scenarios that lacked it: 200 for
`landing_site_eye_level`, 120 for `landing_site_ground_detail`, 8 for `highest_prominence_peak`.
`stand_on_ground` already had 300, which is exactly why its blindness was visible in the first place
-- the harness already refuses a delta tolerance without a point floor (`scenario.rs:740`). The gap
it does not cover is a scenario asserting *clearance* without a point floor, which is how
`landing_site_ground_detail` stayed buried and green.

`highest_prominence_peak` needed a scenario-local `surface_probe_max_distance_meters` of 20,000m to
compare anything at all, and still only reaches 5 points per frame: it stands on a 186km peak looking
18 degrees down, so almost everything in frame is tens of kilometres away. Its floor of 8 is there to
catch the probe going blind again, not to be a rich sample.

**Survey of the rest.** 24 of the 69 scenarios with probe data have surface hits and zero compared
points in their latest run. Nineteen are orbital, atmosphere or flyover scenarios where that is
correct -- everything in frame is past the comparison limit and ground truth is not their subject.
Five are near the ground: `sunrise_midday_surface` and `sunset_blue_hour` at 100m, `ocean_waterline_flat`
at 1m over water where the CPU height is zero by definition, `ground_to_orbit` at 10m and climbing,
and the known-failing `low_flight_performance`. None of them asserts clearance, so none is silently
claiming a camera is on the ground. They are left alone deliberately.

All 428 workspace tests pass, `global_highest_summit` passes, fmt and clippy clean.

## The grey slab over the ocean is giant flat-shaded triangles - 5 September 2026

Reported from manual run `1788617902-64447`, an ascent from 130m to 1,673m over coastal water: the
near ocean renders as a flat, featureless grey slab filling the lower half of the frame, with correct
blue wavy ocean beyond it and the ship floating on the slab.

**It is not fog, not a shading failure, and not missing geometry.** Measured down one column of
`capture-001`, the frame has two mathematically constant bands -- `(29, 66, 101)` for 175 rows and
`(99, 112, 121)` for 600 rows -- with zero variation and hard boundaries. Shading varies; fog
gradates; a cleared buffer does not change colour with camera altitude, and this does. What produces
exactly this is a **single flat-shaded ocean triangle covering hundreds of pixels**, which is what
the ocean looks like when its LOD is starved and the composition mode is the default flat-triangle
one. Setting `CATINGARDEN_MAX_ACTIVE_CHUNKS=24` on the repro scenario reproduces it outright: the
ocean becomes a handful of enormous constant-colour facets with straight polygon edges, constant runs
of 164 and 319 rows down the centre column.

**It is not caused by the probe or scenario work committed today.** The same slab is in manual
captures from 2 and 3 September, confirmed by eye on `1788457699-317114/capture-002.png` and by patch
flatness (standard deviation under 2.0 where a rendered sea measures 10-16) across seven runs on four
different commits before this branch's current work.

**A new scenario, `ocean_grey_foreground`, replays the player's exact pose** -- position
`[3432984, 2014754, 395512]`, view direction `[-0.813, 0.555, 0.177]`, 130.3m altitude, his sun. At
the default budget it draws the ocean correctly, with full wave detail and foam. That is the useful
part: it says what the trigger is *not*.

**Ruled out, each by a run that failed to reproduce it:**

- the pose and view direction themselves;
- weather maturity -- matured to `t 1800s / 3 steps` with 600s timesteps, matching his HUD exactly;
- viewport -- `CATINGARDEN_VIEWPORT=1920x1080` (added this session, since LOD demand is screen-space
  error in *pixels* and a 1280x720 scenario cannot reproduce what the player sees at 1920x1080);
- fast flight -- 6km at 2,000 m/s into the pose, then holding, to provoke a streaming shortfall.

**The trigger in his session is still unidentified.** The strongest remaining clue is his own HUD:
`Terrain: 256 active | 256 drawn`, `Tiles: 169`, **`Fallback chunks: 246`**. So 246 of the 256 drawn
chunks were on a coarser ancestor tile than they asked for -- a *tile streaming* shortfall rather
than a chunk-budget one, which is the same shape as open thread 5, where `descent_to_10m` holds
`tiles_loaded` at zero across twenty seconds. **These may be one defect**; nothing has shown they
are.

Worth noting for whoever picks this up: the default composition mode is `FlatTriangles`
(`main.rs`, the `_ =>` arm of the `CATINGARDEN_DEBUG_MODE` match), so every interactive launch runs
the flat-shaded presentation. That is what turns a coarse ocean patch into a featureless slab instead
of a merely low-poly sea, and it is why this reads as a catastrophic artefact rather than a
resolution drop.

## The planet turns 18km/s under a standing camera - 5 September 2026 [WRONG, see next section]

The grey slab and the vanishing boat are one defect, and it is not the ocean, the streamer or the
detail ladder. **A surface camera does not turn with the planet. The ground slides under it at
18.4km/s.**

Measured from manual run `1788619495-74706`, a swimming camera at the spawn coast. Between logged
frames the camera's world position moves 9,123-9,539m, a speed of **18,093 m/s**, while the HUD
reports `Surface speed: 0.00 m/s`. Latitude is constant to nine decimal places at 30.246944237; the
longitude advances 1.6823 degrees over the run, which is 0.029361 radians against a logged
`planet_rotation_radians` of 0.02936. The camera's ground track is the planet's rotation exactly,
and nothing else.

The arithmetic: `INTERACTIVE_DAY_REAL_SECONDS` is 1,200, so the planet turns at 0.005325 rad/s -- a
19.7-minute day. At the 4,000km radius that is 18,402 m/s of ground speed at his latitude, or **307
metres per frame at 60fps**. This was worse before 4 September: `83ef427` *slowed* the day from five
minutes to twenty, which was a fourfold improvement on a figure that is still 40 times Earth's
equatorial 465 m/s.

Both reported symptoms follow directly:

- **The boat vanishes the moment time starts.** He pressed F10 with the world frozen -- the first
  capture logs `planet_rotation_radians` 0.0 and a camera velocity of 0.07 m/s -- and from the next
  logged frame the camera is doing 18km/s. The ship is swept out of frame inside a second. Nothing
  else visibly changes because open sea looks the same everywhere.
- **The grey slab is coarse fallback geometry.** Tile streaming cannot serve 307m per frame, so
  `Fallback chunks: 247` of 255 drawn: the ocean is drawn from coarse ancestors as single flat-shaded
  triangles hundreds of pixels across, which is the constant-colour slab measured in the previous
  section.

**Reproduced by one field.** `ocean_grey_foreground` at his pose with
`planet_rotation_time_scale: 0.0` draws a correct, fully detailed sea. Set it to `1.0` and the slab
appears within half a second -- 597 to 668 constant-colour rows out of 720 down the centre column --
and by two seconds the terrain is torn ribbons over flat void. That one field is the whole
difference, which is why the previous section could rule out pose, weather, viewport and 2,000 m/s
flight and still not find it.

**Why no scenario has ever caught this: 60 of the 69 scenarios set
`planet_rotation_time_scale: 0.0`,** six omit it, and only `ground_to_orbit` and `sunset_sweep` run
with it at 1.0. `ground_to_orbit` starts at 10m altitude, is one of the known failures, and is one of
the near-ground scenarios comparing zero probe points -- consistent with being swept off its ground
before it can measure anything, though that has not been confirmed.

**This is a design question, not a bug to patch quietly.** The options are different in kind: give
the planet a realistic day (Earth-like 465 m/s ground speed needs about a 13-hour day at this
radius); freeze or heavily slow rotation whenever a surface camera is active; or make the surface
camera and everything attached to the ground co-rotate, so a standing player is stationary with
respect to the terrain and the streamer never sees the motion at all. The third is the only one that
keeps both a fast visible day and a usable surface, and it is the largest change. Ian's call.

The HUD is also misleading here and should say so: `Surface speed: 0.00 m/s` is true in the frame the
camera controls and says nothing about the 18km/s the ground is doing underneath it.

## Correction: the camera does turn with the planet - 5 September 2026

**The previous section is wrong and its commit `7c362d0` should not be trusted.** There is no 18km/s
ground slide under an interactive camera. Ian said the planet did not used to spin -- that the sun
went round it, for reasons of scale and precision -- and checking that is what found the error.

De-rotating the logged camera track into the planet frame, which the previous section did not do, the
camera moves **at most 2.8m per logged frame** across the whole run, and most steps are under 1m.
That is a swimmer bobbing on waves, not a body crossing 9km. The camera turns with the planet exactly
as designed: `04eab2f` ("analytic atmosphere and **rotating camera frame**") introduced
`planet_local_vector` and `planet_frame_world_position`, and `TerrainRenderer::update` is handed
`camera_planet_frame_position`, so every terrain lookup already happens in the planet's own frame with
the rotation undone. The sun direction is fixed in world space and the planet turns beneath it, which
is why from the ground the sun appears to go round -- the same thing Ian remembered, still in place.

**What misled me was a log field.** `velocity_meters_per_second` was differencing the *world*-frame
position, so it counted the rotation carrying the camera and read 18,093 m/s for a player standing
still, while the HUD's own `Surface speed: 0.00 m/s` was right all along. Believing the log over the
HUD, and never de-rotating, produced a confident wrong diagnosis with arithmetic that agreed with
itself. It is now measured in the planet frame, with a stale-baseline flag so a camera-mode teleport
reports no motion instead of a jump divided by a frame time.

The reproduction in the previous section was real but measured something else. A *scenario* camera is
authored at fixed world coordinates and does not co-rotate, so setting
`planet_rotation_time_scale: 1.0` genuinely does drag it over the ground at 18km/s. That is a
property of scenario replay, not of interactive play, and it produced the same symptom by a mechanism
that does not apply. `ocean_grey_foreground` is kept, with rotation back at 0.0.

**What the same log actually shows, and it is the real lead.** With the camera stationary in the
planet frame for 5.6 seconds:

| sim_time | fallback_chunks | resident_chunks | resident_tiles | tiles_loaded | budget_limited |
|---|---|---|---|---|---|
| 0.00 | 255 | 255 | 135 | 0 | true |
| 0.52 | 247 | 255 | 144 | 0 | true |
| ... | 247 | 255 | 144 | 0 | true |
| 5.61 | 247 | 255 | 144 | 0 | true |

**247 of 255 drawn chunks sit on coarse ancestor tiles and `tiles_loaded` never leaves zero.** The
streamer loads nothing at all, for a camera that is not moving, while `budget_limited` stays true and
resident chunks sit exactly on the 256 cap. That is the grey slab: coarse fallback geometry, drawn in
the default flat-triangle composition mode as single constant-colour facets hundreds of pixels across.

This is open thread 5 -- `descent_to_10m` failing with `tiles_loaded` at zero across twenty seconds --
seen from the player's chair. **They are very likely one defect**, and the next question is why a
saturated chunk budget coincides with a streamer that never requests anything. Nothing has confirmed
the link yet.

## The ship lagged the camera by a frame - 5 September 2026

Reported from play: turning the camera makes the boat's movement lag and then correct itself once the
scene settles.

`advance_ship` did two unrelated things. It integrated the hull against the ocean, which is
camera-independent, and it uploaded the hull's **view-space** offset via
`basis.world_to_view(camera_offset)`, which is not. It ran at `main.rs:3027`, while the interactive
camera is advanced at `main.rs:3091` in the `CameraMode::Surface` arm. So the hull's screen position
was computed from the *previous* frame's camera basis and corrected a frame later -- exactly the
described lag-and-snap, and only while the camera is turning, since a still camera makes last frame's
basis and this frame's identical.

The upload is now `upload_ship_transform`, called immediately after the camera is settled and before
anything else consumes the camera. The physics stays where it was.

**Not visually verified, and it cannot be by a scenario.** Captured the same 25-degree camera sweep
over the ship with and without the change: the hull's pixel centroid is identical to a tenth of a
pixel in both frames, 13,594 and 15,980 hull pixels either way. That is not the fix failing -- it is
scenario replay never having the bug. A scenario sets its pose from the waypoint at `main.rs:2947`,
above `advance_ship`, so the camera is already current by the time the ship updates and only live
mouse-look exposes the ordering. **Confirmation has to come from playing it.**

A source-order test pins the invariant: physics above the camera advance, upload below it. It is the
same shape as the surface-probe depth-readback test, and for the same reason -- an ordering that no
scenario can catch needs something that reads the order directly.

Worth noting as a pattern, since it has now cost this session twice: **scenario replay and
interactive play differ structurally**, and a defect that lives in the difference is invisible to the
whole scenario suite. The rotation misdiagnosis two sections up was the same trap from the other
side -- a scenario camera does not co-rotate where an interactive one does. When a report comes from
play and a scenario cannot reproduce it, that gap is the first thing to suspect, not the last.

## Why nothing streams, and why the sea is missing - 5 September 2026

Two findings, one solid and one a lead. The first corrects open thread 1, which I wrote two sections
ago.

**The streamer is not broken. There is nothing to stream.** Only the `pz` face is baked beyond L4:

| face | levels present |
|---|---|
| px, py, nx, ny, nz | l00-l04 |
| **pz** | **l00-l18** |

`sparse_landing_direction` in the manifest is `[0.5489, 0.1475, 0.8228]`, which is +Z dominant, and
the bake refines a cone around it. The reported session sits at `[0.8364, 0.5037, 0.2159]` on the
`px` face -- **44.7 degrees outside that cone** -- so the deepest tile that exists under the camera is
L4, at 3.85km per texel. `outmap.resolve_tile` returns that L4 ancestor, it is already resident, so no
load candidate is ever generated and `tiles_loaded` sits at zero forever while 247 of 255 chunks
report as "fallback". Every one of those numbers is the streamer behaving correctly over a planet that
has no finer data there. Open thread 1 as written -- "the tile streamer loads nothing while the chunk
budget is saturated" -- is answered: it loads nothing because there is nothing to load.

Worth knowing separately: **the landing-site scenarios are 56.7 degrees outside the refined cone
too.** `stand_on_ground`, `landing_site_eye_level` and `landing_site_ground_detail` all sit at
direction `(1, 0, 0)` on `px`, so their 0.317-1.285m probe agreement is agreement about L4 macro plus
the synthesised detail ladder, not about baked fine terrain. They are still valid CPU-versus-GPU
parity tests. They are not tests of anything the baker produced below 3.85km.

**The sea itself: `ocean_chunks` collapses in interactive play and I have not found why.** At the
identical pose, with identical fallback counts and both budgets saturated:

| | ocean_chunks | drawn_chunks | fallback | ocean_triangles |
|---|---|---|---|---|
| reported session | **31** | 255 | 247 | 71,424 |
| scenario, same pose | **255** | 256 | 247 | 587,520 |

That is the grey slab, directly: with the ocean drawn on 31 chunks instead of 255, the flat-triangle
composition mode's water-owned terrain -- flattened to sea level by `planet.wgsl:664` and shaded as
terrain -- is what fills the frame. His other captures agree, at 24, 31 and 37 ocean chunks.

Camera height is **not** the trigger: sweeping the scenario camera down 130m, 40m, 12m, 3m holds
`ocean_chunks` at 255 and `ocean_triangles` at 587,520 throughout.

**The strongest untested lead** is in `terrain.rs`, where `may_contain_ocean` is decided. Its
`height_footprint_is_strictly_land` branch is taken whenever `source_uv_scale`/`source_uv_offset` are
not the identity -- which for a **near-field** chunk they never are, because those fields carry the
*window* transform, addressing the 8x8-tile near-field window rather than the single source tile whose
`heights_meters` the function then samples. A near-field chunk therefore tests the wrong region of the
wrong texture for whether it contains water, and a wrong "strictly land" answer culls it from the
ocean pass entirely. That would remove exactly the near chunks, leave the far ones, and produce a low
ocean-chunk count with correct distant sea -- which is what the screenshots show. **This is a code
reading, not a measurement. Nothing has confirmed it**, and the difference between interactive and
scenario runs is still unexplained under it.

## Near-field ocean culling repaired - 5 September 2026

The UV-space lead is confirmed. `may_contain_ocean` used a near-field window transform against
the resolved source tile's heights. It could therefore prove an unrelated patch was land and omit
the ocean covering the actual patch. This is not exclusive to interactive play: the existing
`ocean_ship_float` scenario reproduces the missing foreground without new camera or weather controls.

`TerrainRenderer` now retains the exact height vector it already builds and uploads for the
near-field window (about 4MiB, moved rather than cloned). Near-field culling tests that vector with
the same unguttered 1025-square coordinates as the shader; ordinary tiles retain their existing
guttered 131-square test and cached whole-tile result. Window metadata and CPU heights advance
together only after upload, and clear together when the window is disabled. This does not disable
land culling, change LOD, rebake terrain, or alter wave geometry/shading/collision.

**Matched Quadro/Vulkan evidence**, same source data, 1280x720, Immediate, fixed scenario:

| `ocean_ship_float` | Before | After |
|---|---|---|
| Run | `1788622679-93328` | `1788622852-96276` |
| Settled ocean chunks | 40 | 255 |
| Total drawn chunks | 256 | 256 |

At 4 seconds (`screenshots/capture-002.png`), the grey foreground is replaced by drawn waves around
the ship. The top 200 sky rows are byte-identical. The old constant dark-blue lower-half region
occupied 150,901 pixels; that colour occupies zero afterward. Both runs satisfy the old scenario
assertions, which alone did not guard ocean draw coverage. The before/after binaries and check logs
are preserved under `test-runs/ocean-culling-fix/` (before code `13cb2d1`, after code plus this patch).

**Validation:** the new footprint regression failed before the fix and passes afterward. It covers
all-land rejection, unrelated source-tile coordinates, zero/negative/NaN water retention, unrelated
water outside the footprint, and the outer bilinear tap. Existing tile-culling tests still pass.
Workspace tests pass (430 tests, 11 ignored), as do workspace check, clippy, and formatting.
GPU `ocean_grey_foreground/1788622881-96337` and land control
`highest_prominence_peak/1788622894-96436` pass; the latter still culls ocean over land.
Fresh interactive travel remains the human acceptance check. This restores missing rendering;
it is not an FPS improvement claim.

## Stars, verified and committed on Codex's behalf - 5 September 2026

Codex added the star field in one pass and ran out of usage mid-verification, leaving the work
uncommitted in the tree. Verified and committed here rather than lost: `stars.rs` (408 lines),
`stars.wgsl` (160), three scenarios, and small edits to `atmosphere.rs`/`atmosphere.wgsl`/`main.rs`/
`scenario.rs`.

**The frame is right, which was the thing worth checking.** `stars.wgsl:37` states it -- *"Catalogues
are inertial. Counter-rotate with the planet"* -- and `StarRenderer::update` takes
`planet_rotation_radians` and uploads its sine and cosine, so the catalogue sits in world space and
the sky turns over a ground observer rather than riding with them. Everything ground-attached lives in
the planet frame; the stars deliberately do not. Given how much of today went on frame mix-ups, that
is the detail that would have been expensive to get wrong.

Visibility is physical rather than gated: a ray whose closest approach falls inside
`PLANET_RADIUS_METERS` returns black, so the limb occludes; `stellar_column` applies atmospheric
extinction so daylight drowns faint stars before bright ones; `stellar_cloud_transmission` obscures
through cloud density. No altitude rule and no separate space skybox -- the same pass answers both
"night from the ground" and "in orbit", which is the property the project is built around.

**What I verified:** `cargo fmt --check`, `cargo clippy --workspace --all-targets` and
`cargo check --all-targets` clean; 434 workspace tests pass, up from 430, so four new tests came with
it. `starfield_space`, `starfield_twilight` and `starfield_performance` all pass. The twilight capture
shows stars above a black occluding ground with a thin horizon band; the space capture shows a Milky
Way band across a dark sky.

**One defect, measured not eyeballed.** The nebula/galactic glow carries a visible diamond-lattice
hatching. Fourier analysis of a 180x180 patch of `starfield_twilight/1788626704-108380/capture-004.png`
inside the glow gives a dominant periodic component at **45.0 pixels** horizontally with a
peak-to-median spectral power ratio of **53.4**, where a smooth field sits at 1-3. So it is a real
repeating structure in the field, not display dithering or an artefact of my eyes. It is most visible
where the glow is brightest and against black elsewhere. Unattributed -- likely the unresolved-star
noise field being sampled on a regular lattice, but nothing has been traced.

Not verified by anyone yet: the interactive night sky from the ground, and matched star-on/off
performance runs, which is where Codex ran out.

## A second world, step one: the body is a parameter - 5 September 2026

Groundwork for the moon. **No visual change to the planet** — this is the refactor that makes a
second body expressible, and the moon's own character is the next step, not this one.

`body.rs` holds what distinguishes one world from another: radius, rotation period, whether it has an
ocean, whether it has an atmosphere. `PLANET` and `MOON` are declared there, one is active per
process, and `--body planet|moon` selects it. `MOON` is 1,080km against the planet's 4,000km, roughly
the Earth/Luna ratio.

**One body at a time, deliberately.** The planet is switched off while you stand on the moon. That
simplification is what lets the radius reach the GPU as *generated shader source* rather than a
per-draw uniform, which is why the CPU and GPU cannot disagree about it. Drawing both at once needs
the radius to become a uniform; that is a later step and it is the expensive one.

**What changed.** `PLANET_RADIUS_METERS` was a compile-time constant read at 201 sites in the app,
and hand-written as `const PLANET_RADIUS_METERS: f32 = 4000000.0;` in **eight separate shaders** with
a test policing that they matched. It is now `planet::planet_radius_meters()`, forwarding to the
active body, and the shader copies are generated by `body::wgsl_constants()` and prepended at each
pipeline's assembly — the same idiom `ocean::wgsl_constants()` already used. Four derived constants
became functions (`minimum_camera_radius_meters`, `cube_face_arc_meters`,
`forest_beam_top_radius_meters`, and a test's L4 spacing). The baker keeps its own
`coretypes::PLANET_RADIUS_METERS`, because a bake is tied to the body it was made for, and a test
asserts `PLANET` still matches it.

The mirror test is stronger than it was. It used to check that eight hand-written copies agreed with
Rust; it now asserts that **no** `.wgsl` file declares the radius at all, that each assembled shader
receives exactly one generated declaration, and that the declaration carries the active body's value.
The duplication is removed rather than policed.

**Verified as a no-op at the default body.** `ocean_ship_float`, `coast_waters_edge` and
`stand_on_ground` were captured from a stashed pre-refactor build and again after: **max pixel
difference 0** across all ten captures. A control confirmed the scenarios are bit-deterministic
run-to-run, so a zero there means something. 437 workspace tests pass, fmt and clippy clean.

Note for the next reader, because it cost time here: `still_5s` is `solid_color_screen`, so it cannot
show a body change. `orbit_once` under `--body moon` differs by 241 and shows a visibly smaller
world, which is how the flag was confirmed to work at all.

**What the moon is not yet.** It currently wears the planet's outmap, ocean and atmosphere at a
quarter the radius, so its terrain is proportionally enormous and its limb is ragged. Still to do:
its own synthesised (crater) terrain needing no bake, `has_ocean`/`has_atmosphere` actually consulted
so it is dry and airless, rotation period taken from the body rather than
`planet::PLANET_ROTATION_PERIOD_SECONDS` (which several `const` expressions still derive from), and
starting the game on its surface. Those three accessors are `#[allow(dead_code)]` until then, marked
with what consumes them.

## The moon: craters, ice, no air - 5 September 2026

`--body moon` now draws a second world. Grey-white regolith, impact craters, ice sheets in the crater
floors, black vacuum with stars, and **the planet is byte-identical**: `coast_waters_edge`,
`stand_on_ground` and `ocean_ship_float` all return max pixel difference 0 against baselines captured
before any of this.

**The macro surface is a crater catalogue, not a bake.** The planet's geography needs a pipeline —
erosion, hydrology, climate — because it is a weathered world. An airless body's large-scale shape is
overwhelmingly its impact history, so `moon.rs` synthesises it: 48 impacts on a golden-angle spiral,
sizes cubed so most are small and a few are basins, depth sub-linear in radius because large basins
relax. Each is a bowl inside a rim crest with an ejecta blanket that reaches exactly zero at twice the
rim radius, so influence is local and the datum does not drift with the catalogue's size.

**The catalogue is generated into the shader, not mirrored into it.** `OCEAN_WAVE_TABLE` is
hand-copied between `ocean.rs` and `shared_planet.wgsl` with a test asserting every literal matches;
forty-eight craters would make that a divergence waiting to happen. `moon::wgsl_constants()` emits the
table the CPU itself uses, so the ground query and the GPU displacement are built from identical
numbers by construction. The profile is smooth trigonometry and polynomials throughout — no hashing
and no `fract`, because this field is evaluated in f64 on one side and f32 on the other.

**Ice is terrain, and that is the whole trick.** Everything the impacts dug below the datum is filled
level with it, and shaded as `BiomeId::Ice` where the *unfilled* floor is negative. Because the fill
belongs to the terrain pass rather than the ocean, **walking on it needed no code at all** — the
ground query, the collision surface and the LOD already describe it. The moon carries
`has_ocean: false`, so `water_owned` is gated off before the biome is even consulted and no ocean
chunks are submitted.

**Vacuum still draws.** The first attempt skipped the atmosphere passes on an airless body and left
space showing the cleared buffer as flat grey daylight — the same cleared-background trap as the near
plane clipping months ago. The pass runs; `BODY_HAS_ATMOSPHERE` makes `displayed_sky_radiance` return
black. That keeps the background painted and lets the stars stand against it.

**Known and not yet addressed.** The lit side is under-exposed: removing the sky removed the ambient
term with it, and an airless surface in full sun should be bright against black shadow, not dim grey.
That is a lighting question, not a geometry one.

**48 craters is a bring-up, not a design.** Every one is evaluated per vertex, so the count is a
frame-time cost; thousands — which is what a real cratered body needs — cannot go through this path.
The right home for thousands is the baker: a moon outmap streamed exactly like the planet's, at which
point the moon stops being a special case in the renderer entirely and `TerrainSource::Placeholder`
goes back to being a placeholder. The crater profile in `moon.rs` is the part worth keeping when that
happens; the per-vertex loop is not.

Still to do: rotation from the body rather than `planet::PLANET_ROTATION_PERIOD_SECONDS`, and
spawning on the surface rather than in orbit.

## The moon turns on its own clock, and you start on it - 5 September 2026

The last two items from the moon's bring-up.

**Rotation follows the body.** `planet_rotation_radians` divides by
`body::rotation_period_seconds()` rather than the planet's constant, so the ground turns at the rate
the world it belongs to specifies. `planet::PLANET_ROTATION_PERIOD_SECONDS` stays a constant on
purpose: the weather clock derives a chain of `const` expressions from it across 33 sites, and weather
only runs on a body with an atmosphere. On the moon the interactive time scale still comes from that
planet ratio, which turns its 60s period into an 80-minute real day — slow, which is what a tidally
locked body should look like, so it is left as a feature rather than churned into a runtime chain for
no behavioural gain.

**The moon is spawned onto, not orbited.** `body::spawns_on_surface` says which worlds the game begins
standing on; the moon does, the planet keeps the orbital opening it was built around. It is a property
of the world rather than a branch in the camera code, which is what makes it testable at all.

The spawn is **deferred, not immediate**. Entering surface mode needs a ground height, and at frame
zero the tile cache is empty, so spawning straight away would stand the camera on whatever the
fallback happened to be — the same class of mistake as the four scenario poses that spent months
underground. `pending_surface_spawn` is consumed on the first frame where
`raster_surface_height_meters_at` actually answers.

**Not verified interactively, and a scenario cannot verify it.** `toggle_surface_camera_mode` returns
early when a scenario is running, by design, so the whole path is unreachable from the deterministic
suite. A headless run logged no spatial frames, so there is no evidence from this session that the
camera lands on the ground rather than under it. What is verified: the predicate has its own test, the
planet still opens in orbit, and `coast_waters_edge` and `stand_on_ground` remain at max pixel
difference 0. **`--body moon` needs flying to confirm the landing.** That is the third time this
session an interactive-only path could not be reached from scenarios; it is worth treating that gap as
a standing weakness of the suite rather than a surprise each time.

## No weather in vacuum - 5 September 2026

Fog and clouds are gone on the moon and untouched on the planet.

Three things were still running on an airless body. The **weather passes** — cloud shell, local cloud
impostors, rain, and the forest's light shafts — are now drawn only when the body has an atmosphere;
vacuum holds none of them. **Aerial perspective** is gated inside the shared shader rather than at a
call site: `terrain_material_transmittance` returns unity and `terrain_material_in_scatter` returns
zero when `BODY_HAS_ATMOSPHERE` is false, so ground reaches the eye undimmed at any range with no
horizon haze. That is why distances on an airless body are famously hard to judge, and it now falls
out of the same code the planet uses rather than a separate path.

The atmosphere pass itself still runs — it is what paints space black behind the stars, and skipping
it is what put flat grey daylight where vacuum should be earlier today.

**Planet unchanged**: `coast_waters_edge` and `stand_on_ground` both at max pixel difference 0.

Measured on the moon's lit surface in `orbit_once`, excluding space and stars: mean luminance 33.6 to
**39.5**, p90 76 to **97**, max 202 to **218**. Removing the in-scatter removed some light with it,
but the net is brighter because the haze it was adding was also veiling the surface. The lit side is
still under-exposed against what an airless surface in full sun should look like — that open item from
the previous section stands, and removing the fog has if anything made it easier to see that the
problem is the lighting rather than the air.

## Sunlight has one colour in vacuum - 5 September 2026

The moon still had a warm terminator: measured on the reported capture, R/B rose from **0.815** under
high sun to **1.010** at grazing incidence. That is atmospheric reddening — the low-sun colour shift
the planet's sky earns through a long air column — applied to a body with no air.

**Conditional in the shared shader, not a second shader.** A separate moon shader would mean two
copies of the displacement, the detail ladder and the material chain, which is the divergence this
codebase spends most of its tests preventing; and the branch is on `BODY_HAS_ATMOSPHERE`, a
*generated compile-time* `bool`, so it constant-folds and a separate pipeline would buy no speed. What
it does need is a separate *lighting path*, and that turned out to be two functions, each already the
single source for its quantity:

- `surface_direct_sun_transmittance` returns `vec3(1.0) * visibility` in vacuum instead of sampling
  the wavelength-dependent transmittance LUT. Sunlight arrives the same colour at every angle, so a
  warm terminator is now impossible **by construction** rather than by tuning — the function has no
  wavelength dependence left to produce one. The geometric horizon still occludes, which keeps the
  day/night line sharp.
- `sky_diffuse_irradiance` returns zero. No sky, no skylight: shadows on an airless body are lit by
  nothing at all, which is where their violent contrast comes from.

Removing the atmospheric attenuation also brightened the lit side, which was the open under-exposure
item pulling in the right direction: `orbit_once` lit-surface mean luminance 39.5 to **42.7**.

**Planet unchanged**: `coast_waters_edge`, `stand_on_ground` and `ocean_ship_float` all at max pixel
difference 0. 431 workspace tests, fmt and clippy clean.

**Not confirmed against the reported view.** The warm rim was visible in a manual capture whose sun
sat near the limb; `orbit_once` does not put the terminator across the disc the same way, and the
colour spread it does measure now separates ice from regolith rather than sun angle, so that metric no
longer answers the question. The argument for the fix is the construction above, not a matching
screenshot. Worth re-checking from the pose that showed it.

## Regolith reflectance, tighter ejecta, polar ice - 5 September 2026

Four moon changes; the planet is at max pixel difference 0 on `coast_waters_edge`, `stand_on_ground`
and `ocean_ship_float` throughout.

**The terminator was a soft fade because the surface was Lambertian.** The horizon ramp was already
sharp — `flat_horizon_sun_visibility` uses the sun's real angular radius, 0.004625. What softened it
was `max(dot(normal, sun))`, which shades a sphere like a ball. Regolith does not behave that way: it
is porous and backscatters, staying nearly as bright near the terminator as at the sub-solar point and
then falling off hard, which is why a full moon reads as a flat disc rather than a lit sphere.
Airless bodies now use **Lommel-Seeliger**, `2·μ₀/(μ₀+μ)`, closed at the terminator by the geometric
cosine so the night side still goes dark.

**Craters are empty except at the poles.** Ice survives on an airless body only where the sun never
reaches, so `moon_ice_surface` is zero away from the poles and the bowl is left as the impact dug it.
Near the poles it fills to half the depth of the crater that dominates the point — which is constant
within one crater, so the pond is *flat* rather than following the bowl down. The first attempt filled
every crater to the datum, flattening the whole body into discs; two tests now hold the distinction,
one for empty-versus-flooded and one for the pond being level.

**The ejecta was a plateau, not a blanket.** It faded smoothly to twice the rim radius, so a 0.30 rad
basin wore a 324km raised ring — reported as "huge rings around them with black between". Real ejecta
thins roughly as an inverse cube and stays close: the extent is now 1.35 rim radii with a cubic
falloff.

**Two things were still attenuating direct sunlight in vacuum.** Cloud shadows were being sampled from
the weather field and cast onto the moon, and the wetness/snow material terms were reading the same
field. Direct sunlight does not care whether there is an atmosphere, only whether something is in the
way, and on an airless body nothing is. Both are gated. A small **inter-reflection** term was added
for regolith — a shadowed crater floor is lit by sunlight bouncing off its own sunlit wall, which is
not skylight and so survives having no air; without it the interiors read as holes punched through
the body.

Regolith also gets a small specular, at 0.16 of the water/ice glint.

**Brightness barely moved** — `orbit_once` lit mean 35.7 to 36.0 — so the cloud-shadow gate was
correct in principle but is not what was darkening this frame. p90 is 118 and max 202 of 255, so the
fully lit regolith is not actually dim; my current read is that the dark impression is largely the
lunar phase in that particular frame rather than a lighting fault, but that is a reading and not a
measurement of the thing itself. Worth checking from the surface, where phase does not confound it.

---

## 5 September 2026 — the moon was not dark, it was not being drawn

**The previous section's reading was wrong.** It closed by guessing that the dark half of the moon
was "largely the lunar phase in that particular frame". It was not. `CATINGARDEN_DEBUG_MODE=albedo`
on `--body moon --scenario orbit_once` showed the disc black except for thin rings, and counting
pixels settled it: 5,392 non-black inside a 189x188 bounding box, a fill fraction of **0.15** against
the **0.785** (pi/4) a solid disc gives. The rings were crater rims. Everything else was the clear
colour showing through, because those fragments were **discarded**.

`is_open_ocean_surface` returns true for any fragment at or below the datum that is not baked ice or
lake, and the terrain pass discards those on the understanding that the analytic ocean shell will
draw them. On the moon `has_ocean` is false, so that shell never runs; and `moon.rs::raw_height_meters`
is *only* the sum of crater profiles, so pristine moon is at exactly 0.0 and every crater floor is
below it. The predicate therefore claimed the entire body except the raised rims. Both it and
`outmap_ocean_coverage` are now gated on `BODY_HAS_OCEAN`.

A third defect sat behind it: `terrain_material_color` returns a fixed blue-grey `(0.32, 0.58, 0.74)`
when there is no outmap, which is a stand-in for missing planet tiles, not a material. The moon has no
outmap by design, so its regolith would have rendered blue once it was drawn at all — `biome_color`
and `BODY_TERRAIN_TINT` never reached the smooth path. Airless bodies now take their material from the
biome palette directly. The smooth path was also reading the weather field, which nothing updates on
an airless body; the flat path already skipped it, and now both do.

After the fix the disc fills **0.7873** against pi/4 = 0.7854, dominant albedo `(113, 106, 100)` grey
with `(214, 222, 227)` ice, and the lit render shows an ordinary gibbous terminator.

**On the exposure theory.** Before measuring I offered "physically correct 0.12 albedo shown at planet
exposure" as the explanation. That was wrong and should not have been offered: 0.12 linear is sRGB ~98,
a mid grey, and two stops down is still ~40 — dark, but nothing like black. The user's question, about
whether the moon in front of the Earth would look black, is what made the arithmetic impossible to keep
believing. **A raw-albedo view reading zero is a claim about the albedo, not about exposure**, and it
was already on screen.

### Craters: 48 to 360, and cheaper than before

Asked for "a few hundred craters, mostly smaller ones". The count is a frame-time number, because this
body has no baked height: a crater is a formula, not a stored shape, so every height query re-evaluates
the catalogue — per terrain vertex, per fragment, and again for the probes that build normals.

Measured on `orbit_once`, median steady-state frame time:

| catalogue | per-sample work | frame time |
| --- | --- | --- |
| 48, exhaustive | 48 `acos` | 44 ms |
| 48, cosine cutoff | 48 dot products, few `acos` | 16.7 ms |
| 360, cosine cutoff | 360 dot products | 104.1 ms |
| **360, latitude window** | **~37 dot products** | **17.1 ms** |

Two independent optimisations, both exact rather than approximate:

1. **A cosine cutoff per crater.** `cos(EJECTA_EXTENT * angular_radius)` is precomputed, so a crater
   that cannot reach the sample is rejected by a comparison instead of an `acos`. Beyond the blanket
   the profile is *exactly* zero, so nothing is lost. Worth 2.6x on its own.
2. **A latitude window.** Latitude is 1-Lipschitz on the sphere, so two directions are at least as far
   apart as their latitudes are; a crater whose latitude differs by more than its reach cannot touch
   the sample. The golden-angle spiral already walks pole to pole in index order, so the catalogue is
   sorted by latitude and what survives is one contiguous index run, found in closed form by inverting
   `z = 1 - 2(i + 0.5)/count`. No `asin` is needed — the angle-sum identities give `sin(lat +- window)`
   from `sin(lat)` and `cos(lat)` directly, and the sign of `cos(lat +- window)` detects the window
   wrapping over a pole.

The window is only narrow if everything in it is small, which is why the catalogue is now **two**
arrays: `MOON_BASINS` (8, up to 0.30 rad, always tested in full) and `MOON_FIELD` (352, up to 0.062 rad,
windowed). Each is its own golden-angle spiral, so removing the basins does not break the field's even
latitude spacing — which the index inversion depends on. Sizes follow `max * rank^-exponent` with the
rank taken through a stride coprime to the count, so every rank is used exactly once and size is
decorrelated from position; without that the field would grade from large at one pole to small at the
other, since index order *is* latitude order.

**The load-bearing test is `the_windowed_height_matches_testing_every_crater`**: 2,000 directions,
windowed path against an exhaustive sum over all 360 with no window and no cutoff, agreeing to under
1e-6 m. Anything that breaks the sort order, widens a field crater past the window, or mis-derives the
index range fails there rather than as a subtle terrain artefact. `the_field_is_sorted_by_latitude` and
`the_field_stays_inside_the_window_it_assumes` guard the two premises directly.

**Planet unchanged**, verified rather than assumed: `coast_waters_edge`, `stand_on_ground` and
`ocean_ship_float` re-rendered from a stashed build of the parent commit and diffed, **max pixel
difference 0 across 10 frames**. All the new branches are on generated `const bool`s, so they fold away.

**Still open.** The catalogue is per-sample synthesis, which is why it is 360 and not thousands. The
stated direction remains a baked moon outmap, where the count stops being a frame-time number at all.
And the moon surface spawn is still unverified interactively — no scenario reaches it.

---

## 5 September 2026 — the moon is baked, like the planet

Asked why the moon was not baked the way the planet is, and when that was decided. It was decided in
`cedd1fa`, by me, and the user's own words had pointed the other way twice — *"craters should be used
in the heightmap generation for the larger scale"* and *"the baked terrain will need thousands of
craters"*. The reasoning I gave — an airless body needs no erosion or hydrology — is true but does not
imply what I used it for: **not needing the baker's simulation is not the same as not being baked.**
It was flagged only at the bottom of that commit message, where nobody would read it.

The moon now streams tiles like any other world. `--body moon` opens `assets/outmaps/test-moon`, and
falls back to the per-sample catalogue as placeholder terrain when there is no bake.

### What moved, and why

The catalogue is now in **`coretypes::moon`**, because the baker needs the same one and a second copy
of the profile would be a divergence waiting to happen. Two specs, one generator:

| | craters | range | evaluated |
| --- | --- | --- | --- |
| `RUNTIME_SPEC` | 360 | 324km – 4.8km | per sample, emitted into the shader |
| `BAKED_SPEC` | **600,176** | 324km – 2.4km | once, offline, into tiles |

The bake's count is set by what the working grid resolves, not by a frame budget. At 8,192x4,096 over
a 1,080km body that is 828m a cell, so the floor is 2.4km — about six cells across, asserted by
`the_smallest_crater_spans_several_working_cells`. The power law `N(>R) ∝ R^-2` runs from 324km to
that floor in about 18,000 craters; the rest of the count goes into more craters *at* the floor, which
is what a saturated regolith surface is. Rim coverage is 35.7% of the sphere.

**The evaluation is inverted for the bake.** Asking each cell which craters reach it is 176 basins plus
a ~3,750-crater window at each of 33.5M cells — about 17 billion tests. Asking each *crater* which
cells it covers is the total blanket area instead: **2.3 seconds** for the whole body.
`splatting_matches_evaluating_every_cell` holds the fast path to the per-sample definition, and a
second test does the same for which floors come out as ice.

### Four things that were the planet leaking onto the moon

1. **A ×4 height exaggeration.** `OUTMAP_TERRAIN_*_HEIGHT_SCALE` multiplies baked positive height,
   because the planet's Earth-like relief is very flat at 4,000km. Applied to the moon it would have
   quadrupled every crater and taken its datum to 88km. Now `Body::outmap_height_scale`, 4.0 and 1.0.
2. **Climate rules icing the entire moon.** `baked_biome_detail` returns `Ice` above the snowline; the
   moon's surface sits 22km above its own zero, so *every* texel qualified. The first bake failed its
   own landing-site check because of it — which is the check earning its keep.
3. **The planet's material chain.** `terrain_material_tint` was gated on `!outmap`, which had kept it
   off the moon by accident. With a bake the moon has an outmap, and the vegetation/earth/rock/snow
   triplanar blend started running on regolith. Gated on `BODY_HAS_ATMOSPHERE` instead — which also
   took the frame from 24ms to **16.5ms**.
4. **A coastal landing-site rule.** `sparse_landing_direction` wants dry land beside water. The moon
   has none, so it fell through to the first non-ice cell in index order, at a pole, next to ice — and
   the tile resample rounded onto the ice. It has its own rule now: equatorial, bare regolith, no ice
   among its neighbours, flattest available.

**`Outmap::open` now refuses a manifest whose radius is not the active body's.** Nothing checked
before, because there was only ever one world; the planet's tiles on the moon would have draped
4,000km of geography over a 1,080km sphere at four times the relief, and every height would have
looked like a terrain bug rather than the wrong world.

### The datum

A baked height channel bottoms out at -5,000m and treats 0 as sea level, but the crater field is
centred on zero with every floor below it. `MOON_DATUM_METERS` lifts the whole body. It is a property
of the **catalogue**, not of any one crater: 600,176 overlapping bowls stack to 19,256m below zero
where the single largest basin reaches 10,125m. At 22,000m the body spans 2,744m to 26,993m.
Re-run `report_the_baked_height_extremes` after changing the count.

### Craters were arranged in a grid

Reported from a screenshot: the golden-angle spiral is *too* even, and at these counts its
phyllotactic arms read as a lattice. Impacts are independent events, so positions are now displaced by
up to a full neighbour spacing in a uniformly random tangential direction. An integer hash is safe
here where it would not be in a shared field, because the catalogue reaches the GPU as literals and
the bake as tiles — nothing re-evaluates it on the other side.

Jitter breaks the two things the latitude window relied on. The field is **re-sorted** by latitude
afterwards, and `field_index_slack` — the largest measured gap between a crater's real index and the
one the even-spacing inversion predicts — widens every window by that much.
`the_windowed_height_matches_testing_every_crater` is what proves the pair is still exact, and
`the_field_is_scattered_rather_than_a_lattice` asserts the spacing actually varies.

**Planet verified unchanged throughout**: max pixel difference 0 across 10 frames on
`coast_waters_edge`, `stand_on_ground` and `ocean_ship_float`. One intermediate run showed a
difference of 12 on `stand_on_ground`; it was the background bake saturating disk I/O and starving
tile streaming into an ancestor fallback, and two clean runs since are 0. Worth knowing that the
scenario suite is not I/O-independent.

---

## 5 September 2026 — mountain-ground discrepancy was the probe's filter distance

Based on `4655625`. Reproduced the header's exact 11.433m median / 12.660m maximum in
`mountain_ground/1788648026-171067`. Source-tile and near-field macro samples agreed, so resampling
was not the lead. The raster vertex shader computes `camera_distance_meters` from its
**undisplaced reference-sphere position**, before adding macro height. The depth probe instead
passed the **displaced hit distance** into the CPU detail ladder: approximately 14m versus 20,539m.
That selected a 0.954m filter instead of 205.391m, comparing two different continuous fields.
The tight offset was not proof of a terrain defect; the header's earlier interpretation was wrong.

`ProbeGeometry::raster_detail_distance_meters` now reconstructs the reference-sphere distance
in f64 from the captured camera and recovered radial direction. The raster height comparison and
its diagnostic filter sweep both use it; distance limits and logged hit distances still refer to
the actual hit. Ray queries, shaders, terrain, and collision remain unchanged.

**Regression:** `mountain_ground` now requires all 81 comparisons and at most 2m absolute error.
With only that assertion added, `1788648145-171521` fails at 12.660m. With the correction,
`1788648226-172500` passes: median **0.052282m**, p90 **0.110161m**, maximum **0.312269m**.
Its capture is byte-identical to the original baseline (maximum channel difference **0**).
A unit test distinguishes a 14m elevated-ground hit from its 20,539m datum distance and checks
an oblique direction. This is corrected measurement, not a visual change or FPS improvement.

Workspace tests: **461 passed, 12 ignored**. Clippy with `-D warnings`, formatting, and diff checks
pass. Existing control assertions pass in `stand_on_ground/1788648328-174762`,
`coast_waters_edge/1788648340-174869`, `highest_prominence_peak/1788648351-174917`, and
`ocean_ship_float/1788648363-174937`. These are not blanket parity sign-off: their maximum measured
deltas are respectively 3.431m, 8.490m, 15.162m, and 28.947m; water still compares against sea level,
and the other views still include mesh/source/continuous-field differences. No control tolerances
were relaxed. Logs and implementation patch are under `test-runs/mountain-probe-fix/`; run manifests name
the base commit because verification preceded the repair commit. The unrelated `crates.tar.gz`
is untouched. The next open terrain issue is the summit survey/app disagreement, not this one.

---

## 6 September 2026 — the moon from the ground, and two things that were hiding

### Flat shading was the default, and nothing said so

`RenderDebugMode`'s fallback arm was `FlatTriangles` with `FlatTriangleOutlineMode::Dark`. Not an
opt-in: **every capture in this repo was one flat colour per triangle with a dark outline drawn round
it**, unless `CATINGARDEN_DEBUG_MODE=final` was set explicitly. That includes every moon screenshot in
the previous two sections, and everything I said about them. The default is now `Final` on
interpolated vertex normals (`world_normal`, a `@location(1)` vertex output), outlines off. Both modes
stay reachable, because telling a presentation artifact from a terrain one needs to switch between
them. It changes **99.1%** of `stand_on_ground`'s pixels and 72.3% of `coast_waters_edge`'s, so every
pixel baseline in this file that predates it was captured faceted.

The colour findings survived the switch — moon regolith is `(107,103,106)` in Final as in flat — but
they were reported without knowing which presentation produced them, which is the part worth not
repeating.

### The interactive moon start was the planet's

Found by Codex, by running the game rather than a scenario. `--body moon` spawned at
`COASTAL_START_DIRECTION` — the planet's coastal pose, about **168 degrees** from the moon's own baked
landing site — and then attempted storm-ocean swimming startup on a body with no ocean. Camera
clearance was a correct 1.70m throughout, so nothing asserted anything was wrong. Fixed at `66141e9`:
`inspection_start_direction` takes the baked landing on a body that spawns on its surface, and the
swim start is gated on `has_ocean`.

That is the fourth defect this branch has lost into the gap between scenario replay and interactive
play, after the ship lag, the grey ocean and the surface spawn. Three moon ground scenarios now
exist — `moon_ground_detail`, `moon_polar_ice`, `moon_crater_wall` — and the gap is narrower, but
these were authored *after* the bug and would not have caught it.

### The moon's ground is perfectly flat, and I chose that

Measured, not impressions. On `moon_ground_detail`, the rendered ground is **stdev 0.000 over 115,200
pixels** — one exact luminance. Not low contrast; no variation at all. In `lighting` debug mode it is
still 0.000 against the planet's 18.99, so the normals do not vary either: it is geometry, not
material.

The cause is `moon_landing_direction` in `baker/src/terrain.rs`, which I wrote. It scores candidate
sites by `-(highest - lowest)` — **the flattest cell on the body wins**. All 54 probe points report a
CPU height of exactly `22000.000m`, the datum, with zero spread: the bake put the landing site on
pristine, uncratered regolith, which on a body whose only relief is impacts means the one place with
nothing to look at. "Somewhere to stand" was the intent and "nothing nearby" was the result.

A second, smaller thing sits behind it: the detail ladder adds **0.158m** of relief across that patch,
against 22.7m on the planet's. Some of that is correct — the moon's macro height is not exaggerated
(`outmap_height_scale` 1.0 against the planet's 4.0), and `terrain_detail_octave_headroom` admits
octaves in proportion to the height beneath them, which at a 22km datum should admit everything. It
wants its own measurement rather than a guess, and the flat landing site has to be fixed first or the
two are confounded.

Neither is fixed here. Both are open threads below.

**`moon_crater_wall` renders entirely black** in Final mode — all 230,400 sampled pixels — while
passing every assertion with 81 compared points at 0.09m median. On an airless body an unlit face is
genuinely black, so this may only be a badly aimed camera, but a scenario that passes while showing
nothing is the same failure as a clearance assertion with no probe floor.

### Also

Two more transient pixel differences on control scenarios, both under machine load: 12 on
`stand_on_ground` while a bake was writing, 72 on `ocean_ship_float` while clippy was compiling. Both
0 on clean re-runs. Open thread 13 said disk; it is CPU too.

---

## 6 September 2026 — five times the craters, and a terminator that goes to black

### 600,176 craters

`BAKED_SPEC.field_count` 120,000 -> 600,000. Rim coverage goes **35.7% to 93.8%** of the sphere,
which is a saturated regolith surface rather than a scattering of impacts.

The size range does not move — still 324km to 2.38km — because the floor is set by the working grid
at 828m a cell, not by the count. The law reaches that floor around rank 18,000, so **about 97% of
the new craters are floor-sized**. The count buys density, not new sizes. Going finer needs a bigger
grid, and at 8,192 x 4,096 the direction array is already 805MB.

The datum holds at 22,000m. Five times the craters stack deeper — 19,256m below the crater field's
zero against 17,519m — but the body still spans 2,744m to 26,993m. That measurement is exact rather
than sampled, because it is taken on the grid the bake writes. Splat 4.3s against 2.3s.

### The terminator, and a fourth planet rule on an airless body

Reported as the moon not going straight to black at the edge of the light. Three things were
softening it and **only one was the lighting**, which is why measuring first mattered: in `lighting`
debug mode the night side already computed to exactly 0, and in `albedo` mode the geometry was
plainly there at `(113,106,100)`. The final frame showed `(4,6,15)` — blue, B three to four times R.
So something was added after lighting, and no amount of tuning the lighting would have found it.

1. **`terrain_fog`.** It computes an air path through an atmosphere the moon does not have and mixes
   toward `physical_camera_sky_radiance`, which is Rayleigh blue. The aerial-perspective terms beside
   it were gated when the moon was built; this one was missed because it is composed later, as
   presentation rather than as physics. **That is the fourth planet-only rule found on this body**,
   after the x4 height exaggeration, the snowline and the triplanar material chain — all four found
   the same way, by something looking wrong rather than by a test.
2. **The terminator ramp** was `smoothstep(0.0, 0.06, incidence)`, about 3.4 degrees. With no air the
   only thing entitled to blur an airless terminator is the sun being a disc rather than a point,
   which is half a degree. `AIRLESS_TERMINATOR_COSINE_WIDTH` is 0.012, about 0.7.
3. **The regolith bounce** was linear in the globe-scale day factor and added *after* the terminator
   closure, so nothing shut it off until the geometric terminator. Squared now and closed by the same
   penumbra.

Night side is exactly `(0,0,0)` to the terminator, which resolves in about five pixels at orbital
scale. **Earthshine is deliberately not added**: it was allowed rather than asked for, and what was
there was not earthshine but blue haze leaking. It would be a separate dim, roughly uniform,
night-side-only term.

### ~~Frame times are meaningless while the monitor is off~~ — WRONG, see below

`orbit_once` reported ~1000ms frames, reproducibly, on both bodies and in both render modes, with the
GPU at 0% and 38C and load average 0.05. `xset -q` said **Monitor is Off**, and I concluded DPMS
throttling to 1Hz.

**That conclusion does not hold.** Later the same day, with `xset -q` still reporting Monitor is Off,
`orbit_once` measured 16.4-16.9ms and the two-body replay held a steady 16.66ms across four of its
five phases. Same display state, no throttling. The correlation was coincidental and I recorded it as
a cause on one observation.

What the ~1000ms frames actually were is still unknown. They have not recurred. Treat this section as
a record of a wrong inference rather than a finding, and do not dismiss a slow frame because the
screen is off.

Pixel comparisons are unaffected — fixed timestep, deterministic — which is why every scenario still
passed throughout. But **any frame-time figure taken in that state is worthless**, and nothing in the
harness notices. Check `xset -q` before believing a performance number.

---

## 6 September 2026 — select a rim site, then measure the detail band

Based on `4726502`, rebuilt rather than reusing pre-Final-mode captures. **This is a landing-site
selection and diagnostic result, not interactive arrival sign-off.** No shader, detail amplitude,
lighting, exposure, or camera physics changed.

### Site and bake

`moon_landing_direction` retains equatorial, cube-face-interior, clean-regolith eligibility. It
now rejects immediate-neighbour slopes above 0.15 (~8.53 degrees), surveys candidates every four
source cells, and scores the highest curvature-corrected apparent elevation in eight directions
at 2/4/8-cell distances, with a small local-slope penalty. Flatness is a footing constraint, not
the entire objective. The synthetic rim-versus-empty-hemisphere regression fails with the old
score and passes with the new one; selection is deterministic.

Command: `CARGO_TARGET_DIR=/home/dad/catingard-target cargo build --release -p catinthegarden-baker`,
then `/home/dad/catingard-target/release/catinthegarden-baker --moon --output assets/outmaps/test-moon-rim-landing`.
The completed, validated 8192x4096 / 3,252-tile / 600,176-crater output was promoted to
`assets/outmaps/test-moon`. Previous data is preserved at
`assets/outmaps/test-moon.pre-rim-landing-20260906` (manifest SHA-256
`025ad4a77eb2603b0118f45f1a5a938aec1c96ec63c7f58ac469c4fc7b1ed97d`). New manifest SHA-256:
`efd7973a6ce2ff481840659cb1558b8c0dc53181cf56e2272774b9918b2aab86`.
All **6,138 global L0-L4 payload files are byte-identical**: this relocates the sparse corridor,
not the crater geography.

New direction: `[-0.2520598634446597, 0.1379999577298627, -0.9578214013618696]`.
The exported L18 height at the site is 18,916.088m; a 16-azimuth, 2/4/8km exported-height survey
finds a **27.478-degree rim at 4km**. `moon_ground_detail` is re-authored at eye level, looking
toward that rim with an explicitly daylight sun. Run `moon_ground_detail/1788686212-209946`
passes at **1.689m clearance**, 74 comparisons per capture, 0.716m median / 5.924m maximum
surface discrepancy across its near/horizon samples. Visible recovered heights span **1,583.189m**.
The central `[x480:800,y180:540]` 115,200-pixel luminance patch has stdev **7.278** on the 0-255
scale. The image still has a smooth foreground: no detail was invented to hide that.

### What the ladder actually contributes

Measure CPU total minus CPU macro at each compared direction, not GPU recovered height minus
macro (which also includes raster interpolation and depth/coordinate precision):

| Fresh run / last capture | Points | Source levels | Runtime height min..max / range |
| --- | ---: | --- | --- |
| New moon rim scenario `1788686212-209946` | 74 | L9-L18 | **0..0 / 0m** |
| Original scenario, rebuilt baseline plus previous 600k bake `1788686370-212086` | 58 | L9,L11 | **0..29.148 / 29.148m** |
| Planet `coast_waters_edge/1788686408-212192` | 70 | L4 | **-12.615..8.540 / 21.156m** |
| Planet `stand_on_ground/1788686443-212284` | 80 | L16,L18 | **0..0 / 0m** |

The historical 0.158m/22.7m claim is not a current body-wide amplitude comparison. In particular,
the original scenario was not retargeted when the 600k bake moved its sparse landing centre:
it no longer reproduces the older pristine-plane capture. Its fresh central-patch stdev is 22.233,
not zero. Do not present the historical zero as the before image of this rebake.

The new rim samples all have an empty displacement frequency band. At L18, nominal source
spacing is **0.06437m**, so the high cut rejects wavelengths at/above **0.2575m**, below even the
one-metre ladder floor. At L9 the upper cutoff is ~131.84m, still below the ~380m lower cutoff
of the ~190m datum-distance geometry filter. `fine_source_spacing_and_datum_filter_leave_no_detail_band`
pins zero output for both 1x and 4x macro headroom; multiplying macro height cannot reopen a
frequency interval. The ladder's amplitude is not multiplied by `outmap_height_scale`.

**Source-data issue, deliberately not tuned:** moon export's `if terrain.moon` branch only
resamples the original 828m working-grid heights, skipping baked procedural detail. The renderer
nevertheless treats sparse texel spacing as evidence of fine source detail. That is not true on
this body. The next fix should distinguish source information bandwidth from tile spacing,
with CPU/GPU agreement; increasing noise amplitude is not the answer. These measurements concern
height displacement, not a complete audit of the separate per-fragment normal-detail path.

### Real startup caught another problem

Automated key input into the real `--body moon` window (matched by PID, no scenario) captured
`manual/1788686509-212422`. The site is on the **night side at the default frozen clock**, so
the terrain is black. More importantly, clearance is **82.949m**, not eye level. Inspection shows
startup resolves a dense L4 surface, later streaming replaces it, and the frozen world's gravity
does not settle downward; `resolve_surface_camera_after_streaming` only raises the camera on
penetration. This needs a focused arrival/streaming fix. The daylight scenario is not proof that
interactive arrival works, and no claim of a playable or visually accepted startup is made here.
The pre-existing all-black `moon_crater_wall` assertion hole also remains open.

### Validation and planet isolation

**464 workspace tests pass, 12 ignored**; clippy `--workspace --all-targets -- -D warnings`, fmt,
and diff checks pass. Fresh `4726502` versus after comparisons, all assertions passing and all
**10 captures byte-identical (maximum channel difference 0)**:

- `coast_waters_edge`: `1788686386-212146` / `1788686408-212192` (2 frames).
- `stand_on_ground`: `1788686430-212238` / `1788686443-212284` (5 frames).
- `ocean_ship_float`: `1788686455-212330` / `1788686471-212384` (3 frames).

Builds/tests/baking were complete before these sequential runs; no concurrent workload was
launched. `xset -q` reported Monitor On; no FPS improvement is claimed. Binaries, bake/test logs,
site profile, and pixel-comparison report are under `test-runs/moon-landing/`. Scenario manifests
name the base commit because verification preceded commit. `crates.tar.gz` remains untouched.

---

## 6 September 2026 — filling the empty detail band

Codex's finding stood: the runtime detail ladder contributes **exactly zero** on the moon. It admits
octaves between the camera filter and the spacing the baked data is assumed to carry, and on the moon
that interval was *finer than 6.4cm and coarser than 189m*, which is empty. Two separate causes, and
both had to go.

### The moon's tiles carried no detail, because the export refused to write any

The planet does not have an empty band, because its export writes real relief into tiles —
`baked_surface_detail` plus `sparse_surface_detail`, level-gated. The moon's export skipped that on
purpose, because what was there is the planet's game-terrain profile keyed to a coastline. Skipping
it left the ground between craters a geometrically exact sphere section.

`MOON_DETAIL_BANDS` is the moon's own: five level-gated bands from 540m at 11m amplitude down to 0.9m
at 7cm, isotropic, no landing protection. After it the height field has **9cm of relief across a
metre of ground** where it had none.

### And the shading could not see it, because the filter measured to the wrong place

That relief rendered at luminance stdev **0.00** regardless. `displaced_surface_normal` and the detail
filter were both handed `camera_distance_meters`, measured from the camera to the **undisplaced
sphere**. Those agree until terrain height matters next to viewing distance — and standing on the
moon, whose ground sits 19km above its own datum, the sphere distance says 19km. The normal probes
spread ~189m apart and returned the normal of a 189m-smoothed surface.

The vertex now measures to the **macro-displaced** ground. Macro rather than fully displaced because
detail cannot be added before the distance that decides how much detail to add; the difference between
them is the detail itself, small next to the macro height by construction.

`ProbeGeometry::raster_detail_distance_meters` had to move with it or `mountain_ground` re-opens — it
did, to 12.267m, which is the mirror doing its job. The probe now takes two passes: macro height does
not depend on the distance, so the first pass' answer is exact and the second is the real query.
`mountain_ground` is back at median 0.056m / max 0.303m against Codex's 0.052 / 0.312.

### What it cost, and what it bought

The moon's near ground goes from stdev **0.00 to 1.4–2.1** across every row of the lower frame. Not
dramatic; it is the difference between ground and a sphere.

**The planet changed too, and that was not asked for.** `stand_on_ground` moves 76.1% of its pixels,
max difference 32. It is not a regression — ground-relief stdev goes 18.99 to 20.01, slightly *more*
detail — and the cause is the same bug: standing on 4.4km of terrain gave a 43.9m detail filter, so
the planet's near ground was being denied detail for the same reason at a quarter the severity. But
it is a visible change to the planet from a change requested for the moon, and it wants an eye on it.
`coast_waters_edge` moves 0.1% of pixels, which is the expected shape: at sea level there is nothing
to correct.

`moon_ground_detail`'s pose had to be lifted 4.263m — the roughness moved the surface under a
hard-coded waypoint, the same failure as the four ground scenarios in the earlier section. It is
derived from a measured clearance rather than from a principle, which is worth doing properly.

---

## 6 September 2026 — ice by shadow, not by latitude

Ice used to be a **fill**: crater floors above 53 degrees ponded flat to half their depth, gated on
latitude alone, and the surface height was raised to meet it. That is not where ice is, and it is not
what ice looks like.

Ice on an airless body survives exactly where it is never heated — the floors and poleward walls of
craters near the poles, permanently shadowed because the rim hides the sun through the whole
rotation. So the question is about **shadow**, and it is now answered as one.

### The model

`baker::moon::classify_ice`. The moon's axis is Y and its tilt is taken as zero, so the sun stays in
the equatorial plane and traces the same arc every rotation — which is what makes "permanently
shadowed" computable rather than a guess. For each cell, 32 sun positions over a rotation, and a cell
is ice only if **none** of them lights it. Two ways to be dark:

* **its own slope**, `dot(normal, sun) <= 0`, which is what puts ice on a wall rather than only a
  floor;
* **the horizon**, marched 40km toward the sun's azimuth in 14 geometrically spaced steps, comparing
  each sample's elevation angle against the sun's own. The curvature term matters: over tens of
  kilometres on a 1,080km body the ground falls away, and ignoring it invents blockers that are
  actually below the horizon.

Only latitudes above `|sin| = 0.80` are marched. At latitude phi the sun reaches `90 - phi` degrees,
and crater walls run to roughly 30, so below about 55 degrees there is nothing a rim can do. That
keeps the march off 80% of the body.

**The height field no longer knows about ice at all.** `surface_height_meters` is the craters and the
datum, nothing else, and `ice_surface_meters` / `is_ice_at` / the `POLAR_ICE_*` constants are gone
along with `moon_ice_surface`, `moon_polar_weight` and `moon_is_ice` in the shader. The unbaked
placeholder is bare regolith, which is honest: no ice rather than ice in a place a latitude formula
chose.

### What came out

Ice is **4.32%** of the surface, and its distribution is the right shape without being asked for:
93,250 cells between 85 and 90 degrees, falling monotonically to 236 between 50 and 55. A handful of
deep craters catch shadow at 50 degrees; almost everything does at 89.

**Permanently shadowed ice is, by construction, never directly lit.** That is not a defect, it is the
definition — but it means you cannot see it as ice. `moon_polar_ice`, re-aimed at the densest real
shadowed patch (lat -86.42), has 15.3% of its frame lit at all. What little shows comes from the
regolith bounce term. If this ice is ever meant to *read* as ice, earthshine is the term that would
do it, and it is still deliberately unimplemented.

### The scenarios all had to move

Removing the fill dropped crater floors by up to **1,157m**, so two of the three moon ground
scenarios had their cameras underground. `moon_polar_ice` was re-aimed entirely, because under the
old model every polar floor was iced and under this one only genuinely shadowed ones are — it was
pointing at a crater that is no longer icy, and would have passed while testing nothing.

That is three scenario poses fixed by measurement in two days. Open thread 19 stands and is getting
more expensive: derive these from the bake.

**Planet unchanged**: max pixel difference 0 on all three controls against a baseline rebuilt at
`e5b5e25`. 463 tests, clippy and fmt clean.

---

## 6 September 2026 — planetshine, and a grid that was not the stride

### The smooth path had none of the airless lighting

`flat_triangle_lighting` held the whole regolith model — Lommel-Seeliger, the opposition surge, the
terminator closure, the inter-reflection bounce. The smooth path lit the moon with `max(dot(n, s), 0)`
and nothing else. That did not matter while flat-triangle mode was the default; it mattered the moment
it stopped being. **Everything reported about the moon's lighting between those two points was
measured on a path most of it was not in.**

`airless_surface_response` is now shared and both paths call it. The day side is visibly brighter for
it — 107 against 78 at the same pixel — which is the backscatter finally arriving.

### Planetshine

Requested. `planetshine_irradiance`, gated on `!BODY_HAS_ATMOSPHERE`, three geometric terms: whether
the planet is up at all, whether the facet faces it, and the planet's phase — which is the opposite of
the moon's, so full planet at new moon. **Tidally locked, so it is a body-fixed direction rather than
an orbit**: the planet hangs motionless while the sun goes round, and on the far side it never rises
and this returns nothing.

`MOON_PLANETSHINE_FRACTION` is 0.0012. The literal figure is about 1.2e-4 — ten magnitudes fainter
than the sun. At that value it is invisible here, because this renderer has a fixed exposure and no
eye adaptation, and a real observer's night vision is most of why earthshine looks as bright as it
does. Ten times the literal ratio, written down as an exposure decision rather than dressed as
photometry.

### The grid was the sizes, and my first answer was wrong

Reported as medium craters in rows. My first explanation was the size-rank *stride*: `index * 197 + 89
% count` makes any narrow rank band an arithmetic progression of indices, and arithmetic progressions
on a golden-angle spiral draw phyllotactic arms. It is a good story and **it is not what was
happening** — measured, the strided catalogue's nearest-neighbour spread within a size class is 0.485
against a shuffle's 0.531. Both scattered. The stride was innocent.

The real cause was visible once the screenshot was cropped: it is the *small* craters, not the medium
ones. **97% of the catalogue sat at exactly the 2.4km floor**, because the tail was clamped there, and
600,000 spiral points are spaced 4.94km apart while a 2.4km crater is 4.8km across. Identical discs at
their own diameter tile the surface, and a saturated tiling of identical circles reads as woven
fabric. Position jitter could never fix it: the regularity was in the sizes.

The tail is now *resized* rather than clamped — spread over `floor..floor * 2.4` — which puts 3.4% at
the floor instead of 97%. Jitter went 1.0 to 1.6 spacings as well. Coverage rises to 238%, which
sounds alarming and is not: with varied sizes the bowls stack *less* coherently, and the deepest point
came up from 19,256m below the crater datum to 15,141m. Splat 4.3s to 14.5s.

The shuffle replacing the stride stays, because a bijection with no arithmetic structure is easier to
reason about than one whose safety depends on a coprimality argument — but **it is not the fix and
should not be credited as one**.

### Scenario poses, again

All three moon scenarios moved, twice: the shadow-ice change dropped crater floors, and each rebake
moves the baked landing site. They are now derived from the manifest's landing direction and the
height preview rather than hand-lifted — better, but the derivation lives in a throwaway script and
not in the harness. Open thread 19, now four rebakes old. `moon_crater_wall` became a 120km survey
view, which is what actually shows whether the surface tiles.

**Planet unchanged** across all of it: max pixel difference 0 on three controls. 464 tests, clippy and
fmt clean.

### A note on process

The app panicked reading a half-written outmap because the rebake was `rm -rf` followed by baking in
place. Bake to a new directory and swap it in.

---

## 6 September 2026 — ice that follows the ground, and a size law with no floor

Two reports, both from screenshots, both right, and both about the same mistake: **a field computed on
the 828m working grid cannot place anything the eye resolves at close range.**

### Ice was in blocks, and beside the shadows rather than in them

The biome map is one categorical value per texel, sampled nearest, so ice edges were 828m cells —
squares and plus-shapes, unrelated to the ground under them. Worse, the shadow was computed on the
crater field alone, which is the terrain *before* `MOON_DETAIL_BANDS` is added at export. So the
relief casting the shadows you can see was never in the calculation, and ice landed on flat lit ground
while genuinely shadowed floors stayed bare.

Three changes:

* **The shadow is a fraction, not a verdict.** Near a pole "is this facet lit" is a knife-edge that
  flips between neighbouring cells, so the boolean speckled. Stored 0-255 in the moisture channel,
  which an airless body has no other use for, and box-blurred three passes — it is a *regional* term,
  "can ice hold near here", and wants to be smooth.
* **A per-fragment test does the placing.** With zero obliquity the sun stays in the equatorial plane,
  and both "is the sun above this facet's horizon" and "does the sun face it" depend only on the
  horizontal parts of the normal and the position. So permanent self-shadow reduces to one dot
  product: a facet is never lit exactly when its horizontal normal points opposite its horizontal
  position — when it faces poleward. Exact, free, and it resolves every pixel.
* Multiplied. The baked term knows about crater rims and cannot resolve them; the fragment term
  resolves everything and knows nothing about rims. Together: a crater's poleward wall ices and its
  sunward wall does not, at the resolution the surface is drawn.

Ice is now 0.41% of the surface, which is close to the real figure, and it sits inside the shadows.

### The arctic circle

`SHADOW_LATITUDE_SINE` was 0.80 — 53 degrees — on the argument that crater walls run to 30 degrees so
the sun clears any rim below that. Wrong in the direction that matters: overlapping rims and ejecta
are far steeper than one wall, and a deep basin shadows its own floor well outside the polar circle.
Now 0.20, which only skips the deep tropics. Ice reaches 20-30 degrees and rises smoothly to the pole.

### Every floor bunches the sizes

Reported as the medium craters taking over, and the numbers were stark: **598,536 of 600,176 craters
were between 2 and 8km across**. A floor does that by construction — everything below it becomes one
size — and the previous fix, spreading the pile over a band, only widened the band it bunched in.

There is no floor now. The law runs to its end: 573,933 craters under 2km, 24,603 between 2 and 8,
1,640 above. Craters finer than the working cell are not resolved *as craters*, but they are not
spikes either — they are the fine dimpling the ground should have. Coverage drops 227% to 35%, which
is the honest number for a real size-frequency law at this resolution; the previous 227% was an
artefact of counting the same crater size 600,000 times.

Splat 14.5s to 167s, because the loop is now dominated by hundreds of thousands of tiny craters rather
than a few large ones.

### Cost

Every one of these rebakes moves the baked landing site, and every moon scenario pose with it. That is
five times now. **Open thread 19 is the most expensive thing on this list.**

Planet unchanged throughout: max pixel difference 0 on three controls. 464 tests, clippy and fmt clean.

### Craters a quarter deeper

`DEPTH_TO_RADIUS` 0.20 to 0.25. A fresh terrestrial crater is about a fifth as deep as it is wide;
this is deliberately deeper, because on a body whose only relief is impacts the shadows craters cast
are what make the surface legible, and at 0.20 they read as dimples.

Height range 6,067m to 29,347m, still inside the stored channel with the datum unchanged at 22,000m.
Ice rises 0.41% to 0.57%, which follows: deeper craters hold more permanent shadow.

Poses lifted again — 1.09m and 61.78m. Sixth time.

### Pink ice

`Body::ice_tint`, generated as `BODY_ICE_TINT` and multiplied into the ice half of the airless
material mix. A body property rather than a palette change, so the planet's glaciers keep reading the
shared `biome_color(2u)` and its captures stay byte-identical.

The moon's is `[1.36, 0.22, 0.55]`, which is stronger than the ratio that would turn the palette's
pale blue pink on its own. It has to be: almost no pixel is *pure* ice, because the shader mixes ice
into regolith by how permanently shadowed the ground is, and a half-mixed pink against grey reads as
off-white. The first attempt at the palette-accurate ratio rendered `(242, 226, 236)` — sixteen levels
of red over green, which is not pink. Set from the rendered result instead: `(244, 190, 233)`.

**Still there**: one white patch on a sunlit rim in the survey capture, which is not ice — most likely
the regolith specular. Small, and not chased.

### The white patch was not ice, and the ice cannot be seen

Chased the white blob reported on top of the pink. It is **brightly lit regolith**: albedo at those
pixels is `(113, 106, 100)`, the plain rock colour, rendering at `(199, 200, 201)`. Blocky because the
normals are, which is the texel-facet problem already open — not a material fault. Two wrong guesses
on the way, both disproved by measurement rather than argument: the categorical-biome specular (the
smooth path's only specular is multiplied by `wetness`, which is zero on an airless body) and
planetshine (disabling it changed the patch by nothing at all, 744 pixels either way).

`material_specular_scale` was keyed to the biome anyway, so ice took a full-strength highlight where
regolith took 0.16. On a body whose every material is rock or the ice lying on it, one dim scale is
right; changed, though it was not the artefact.

**And the survey scenario's sun was unphysical.** I had set it 18 degrees above the *local* horizontal,
while the whole ice model assumes zero obliquity — the sun never leaving the equatorial plane. Which
is how ice ended up sunlit in a capture: the test violated the premise the feature is built on. Fixed
to local noon in the equatorial plane. **A scenario that contradicts the model is not evidence.**

With the sun where it belongs, the real finding: one survey frame holds **671 ice pixels and thirteen
of them are visible**. That is not a bug — ice sits where the sun never reaches, so nothing the sun
does can show it. Planetshine is the only light a permanently shadowed floor gets, in the renderer as
in life, so it is the term that has to carry them. Raised 0.0012 to 0.022, which puts shadowed ground
around 17/255 and the ice near 130: a dark body with faintly lit shadows rather than a black one with
an invisible feature in it.

That is a large fudge over the literal 1.2e-4 and it is written down as one. The alternative is
placing ice where the sun reaches, which would undo the point of the change that put it in shadow.

---

## 6 September 2026 — two open faults, and where the surface texture was going

Handing over. Everything below is unfinished or unresolved, stated plainly so it is not re-derived.

### The blocky white patch is the near-field window boundary

Reported three times, and my first two explanations were wrong — both disproved by measurement, both
recorded above so they are not tried again. It is **not** ice, **not** the categorical-biome specular,
**not** planetshine.

The last screenshot settled it: the patch is a rectangle with a *staircase* edge at tile granularity,
containing terrain at a **different level of detail** from the ground around it. That is the near-field
window: 8x8 tiles, chosen to span at least `NEAR_FIELD_MIN_EXTENT_METERS` = 12,000m. That constant was
sized for the planet, whose face arc is 6,283km. The moon's is 1,696km, so the same 12km asks for a
much coarser level relative to the body, and the window ends up at a different level from the tiles
around it. Inside and outside then carry different amounts of `MOON_DETAIL_BANDS`, which is a
discontinuity in both relief and shading, with a hard edge.

Two things to weigh: whether the extent should scale with the body, and whether the window edge should
blend at all. Nothing in the near-field path was written with a second body in mind.

### The sun is 23 degrees out of the plane the ice model assumes

The interactive sun in the capture is `(0.509, 0.398, 0.763)` — 23.4 degrees of declination. **The ice
model assumes zero obliquity**, the sun never leaving the equatorial plane, and that assumption is
what makes "permanently shadowed" computable at all: it is why the arc of 32 sun positions is every
moment there has ever been, and why the per-fragment test reduces to one dot product.

At 23 degrees almost nothing is permanently shadowed, and poleward-facing slopes are lit — which is
why pink ice shows in *sunlight* at 16 degrees latitude in the last screenshot. The model and the app
disagree about where the sun goes, and the model is the one making the stronger claim.

Three ways out, and this is a design decision rather than a bug fix: give the moon its own sun path at
its real obliquity (about 1.5 degrees, which would keep the model almost exactly as it stands); widen
`classify_ice` and `airless_permanent_shadow` to sample the declination range the app actually uses,
which would shrink the ice drastically and honestly; or accept that this moon has a 23-degree tilt and
rewrite the ice around it. **Do not tune the ice thresholds without settling this first** — they are
currently compensating for a premise that does not hold.

### The surface texture, started

Asked for: a whole-moon texture from the real processes, lining up with the craters. Only the first
piece is in — `Crater::freshness`, 0 ancient to 1 fresh, cubed from a deterministic draw so roughly
one crater in eight is meaningfully bright and one in a thousand is a Tycho. It is **carried but not
yet used**.

It is an optical property rather than a shape one, and it is the main reason a real lunar surface is
anything but uniform grey: freshly excavated regolith is bright, and solar wind and micrometeorite
gardening darken and redden it over hundreds of millions of years. That is why Tycho and Copernicus
read as bright splashes while equally large older craters have faded into the background.

The design that was going in:

* **Ejecta haloes.** Brightness around each crater scaled by freshness and decaying over the blanket,
  so a young crater sits in a bright apron and an old one does not.
* **Ray systems.** For the freshest large craters only, bright streaks radiating far beyond the
  blanket — the thing that makes Tycho visible from Earth.
* **Maria.** Large low regions darker, because basalt flood fill is much darker than anorthositic
  highlands. Free in the shader: it already has the macro height.

**The hard constraint is that it must line up with the craters**, which rules out an unrelated noise
field. The tiles have no free channel — height, biome and moisture are all taken, moisture by the
shadow fraction — so the intended route was to emit the *baked* catalogue's largest few hundred
craters into the shader as a second constant array, with freshness, and evaluate haloes and rays per
fragment against the same impacts that dug the holes. Adding a fourth outmap channel is the
alternative, and it is a schema-version change.

### Disk

Five stale build trees removed, 12.6GB: `catingard-target`, `target-forest`, `catingard-target-hash`,
`-base`, `-flat`. 34G free to 46G. **`assets/outmaps` is a further 12G in 34 entries**, mostly
`test-planet.*-backup-*` from early August — history rather than build output, and deliberately not
touched.

### The baker's tests were filling /tmp

`tests/bake.rs` built its output paths with `std::env::temp_dir().join(..)` and returned a bare
`PathBuf`. Six of the seven were never removed; the seventh called `remove_dir_all` after its
assertions, which is exactly where a failing test does not reach. A full run leaves about 90MB behind.

Found at 67 orphaned directories and 933MB, with `/tmp` — a 1.8GB partition of its own — at **99%
full, 33MB free**. That is not a tidiness problem: a full `/tmp` fails later builds and bakes for
reasons that look nothing like a disk problem, and this session ran the suite dozens of times.

`TemporaryOutput` is an RAII guard now: `Drop` removes the tree, so it cleans up on unwind as well as
on success, and `Deref<Target = Path>` keeps every call site reading as it did. Verified by counting
orphans across two full runs — zero before, zero after.

Also cleared, and worth knowing they accumulate: five stale build trees at 12.6GB
(`catingard-target`, `target-forest`, `catingard-target-hash`, `-base`, `-flat`). `assets/outmaps` is
a further 12G of `test-planet.*-backup-*` from early August, deliberately left alone — history, not
build output.

### Correction: `ba9ee40` contains work that is not mine

That commit says "464 tests, clippy and fmt clean". **It does not describe the tree it committed.**

Codex began the two-body rendering task in the same working directory while the `/tmp` fix was being
made, and `git add -A crates docs` swept 34 lines of its in-progress `body.rs` — a `RENDER_BODY`
thread-local and `with_body`, the start of per-body render state — into a commit about baker test
cleanup. `with_body` is not called yet, so the tree has a `dead_code` warning and is unformatted where
Codex is still editing. Both are its work in progress, not defects.

Left in place deliberately: reverting it would delete live work out from under the agent holding it.
`cargo fmt --all` was also run over its files, which is the same mistake in a different shape.

**The lesson is the working directory, not the commit.** Two agents editing one checkout means one
can format, stage or commit the other's half-finished edits without either noticing. A second agent
should get its own worktree, or the two should not be active at once. Check `git status` before
staging, and stage named paths rather than `-A`.

## 6 September 2026 — two-body transfer replay (Codex, experimental; not full flight sign-off)

**Scope:** the user's simultaneous planet/moon request now has an opt-in raster
`planet_to_moon` replay. Ordinary interactive launches are still single-body.
Do not describe this as completed unrestricted inter-body flight or as good FPS
at every stage. The user asked to leave a tidy checkpoint as usage ran low.

Parent: `159d852`, branch `experiment/ocean-wind-sea-spectrum`. The prior
`ba9ee40` accidentally included the scoped-body foundation; its corrected
attribution remains above. No bakes, material design, moon ice/obliquity work,
or `crates.tar.gz` were changed here.

### Implementation

- `system_flight.rs` owns the additional persistent moon terrain/atmosphere/
  camera resources. `State` dispatches only the named replay to it. No shader or
  renderer reconstruction at arrival. `body::with_body` scopes moon CPU work
  and shader construction, preserving generated per-body constants.
- One f64 shared-space camera, fixed 40,000 km centre separation, independent
  moon orientation mapping the actual baked landing direction to the lit,
  planet-facing hemisphere. Both body projections use the **same** 0.1 m
  reversed-Z near plane; f64 local rebasing occurs before GPU upload.
- Three seconds of endpoint residency warm-up precede a 54-second fixed-step
  route: stand, look up, ascent, transfer, descent, stand on moon. Quaternion
  attitudes preserve local surface up and smooth roll. Endpoint height is
  settled from resident raster queries before departure, not snapped on arrival.
- Moon radiance/depth render offscreen, then `system_composite.wgsl` adds only
  **foreground** planetary atmospheric in-scatter/extinction. Its ray/shell
  entry test prevents the distant planet atmosphere being painted over nearby
  lunar ground. Common scene depth occludes both bodies and stars; the planet
  cloud-shell pass composites afterward. The airless moon sky LUT initializes
  once during warm-up, not every frame.
- Distant geometry uses L0 below 100 px projected radius, L1 below 200 px, normal
  adaptive LOD above that. This is actual baked-body geometry, not a billboard.
  The smooth-sphere L0 chord error at the threshold is <0.13 px (not a bound on
  terrain displacement). Existing transition machinery handles topology changes.
  Normal single-body LOD policy is untouched.
- The replay deliberately omits ships, forests, local cloud impostors, rain,
  interactive navigation and HUD. It freezes body rotation/weather and uses
  fixed exposure. Resizing its internal render target fails explicitly rather
  than silently sampling stale offscreen targets. These are debug restrictions,
  not a replacement for the full interactive renderer.

### Reproduce / compare

```sh
cargo build --release -p catinthegarden-app
# Check Monitor is On; never measure with a build/bake running.
DISPLAY=:0 xset -q
DISPLAY=:0 WGPU_BACKEND=vulkan WGPU_ADAPTER_NAME=Quadro \
  CATINGARDEN_PRESENT_MODE=immediate \
  target/release/catinthegarden-app --scenario planet_to_moon
# Repeat sequentially with CATINGARDEN_SYSTEM_CONTROL=nearest for the control.
python3 scripts/report-system-flight.py test-runs/planet_to_moon/<run-id> [...]
```

The `nearest` control removes only the more distant body's terrain/update work;
it deliberately retains common atmosphere/cloud/star/post work. It measures
**incremental distant terrain cost**, not the full overhead against a standalone
`--body moon` launch. Do not conflate those baselines.

Final same-source Quadro runs at 1280x720, immediate presentation, awake monitor,
no concurrent compilation/baking:

- Both bodies: `test-runs/planet_to_moon/1788707726-267015`.
- Nearest-terrain control: `test-runs/planet_to_moon/1788707786-267182`.
- These manifests name parent `159d852`; they were built from this uncommitted
  implementation, not from pristine parent source. Subsequent source edits were
  formatting only; the commit containing this handoff captures the implementation.

| Stage | Control median ms | Both median ms | Both median-derived FPS | Both p95 ms |
|---|---:|---:|---:|---:|
| Planet surface | 54.103 | 52.577 | 19.0 | 56.865 |
| Ascent | 2.194 | 2.287 | 437.3 | 17.811 |
| Transfer | 2.093 | 2.185 | 457.7 | 3.800 |
| Descent | 14.779 | 15.855 | 63.1 | 23.224 |
| Moon surface | 15.747 | 16.623 | 60.2 | 17.725 |

One matched pair, not a statistical performance guarantee; the slightly faster
planet sample is noise, not an optimization claim. The report excludes 0.1 s
around PNG capture times equally in both runs, not normal streaming stalls.
Ascent/descent still hit 86.818/78.765 ms maxima (control 82.640/75.608).
Distant visible bodies now use 5–6 chunks versus ~56–60 in the first implementation;
arrival planet triangles fall from 129,024 to 11,520. Lunar-surface distant-terrain
cost in the final pair is 0.876 ms / 5.6%; descent is 1.076 ms / 7.3%.

### Validation and what remains

- Both final replays pass: **3,241 finite spatial samples**, seven PNGs, landing
  clearance **1.9999999998 m**. Additional route checks reject <0.5 m near-body
  clearance and require final lunar clearance in [0.5, 3] m. The final pose has
  positive direct solar incidence and the planet above the geometric horizon.
- Captures 002/007 were inspected: moon crescent through blue daytime sky;
  sunlit lunar ground with the planet partially occluded by its ridge. Comparing
  the nearest-terrain control changes 1,104 pixels at capture 002 and 4,729 at
  007; **zero lower-half pixels change at 007**, supporting foreground occlusion.
- 471 workspace tests pass, 12 ignored; workspace/all-target clippy with
  `-D warnings`, fmt and diff checks pass. New tests cover shared-frame precision,
  lit/clear/continuous route, surface attitude continuity, projected-size budgets,
  LOD cache invalidation/restoration and complete composite WGSL validation.
- **Not signed off:** planet departure remains ~19 FPS and its +X capture has
  large stepped pale foreground geometry. The route starts outside the current
  authored sparse landing corridor. Investigate/retarget that departure rather
  than calling it an accepted surface view; no standalone visual baseline was
  taken to attribute that appearance to pre-existing code.
- Next: choose a representative resident planet departure, diagnose the remaining
  streaming spikes, then expose the shared-space camera to real interactive
  navigation/clearance and integrate planet actors/weather. Keep both-body
  resources persistent and retain the atmosphere-entry/depth regressions. Test
  camera zoom/resize and reverse travel before claiming seamless general flight.

---

## 6 September 2026 — verifying the two-body replay

Codex's work, verified before committing because it left it uncommitted despite saying otherwise.

**The claims hold.** Final moon clearance `1.9999999997671694`m against a claimed 2m. Distant-body
geometry is 5-6 chunks against 256 for the near body, which is the "56-60 down to 5-6" claimed. The
last capture is an earthrise: the planet with oceans, cloud and an atmospheric limb, **partially
occluded by a lunar ridge**, stars behind it. Inter-body depth is correct.

**The second body costs nothing measurable.** Its own `CATINGARDEN_SYSTEM_CONTROL=nearest` control
removes the distant body and leaves the common atmosphere/star/post work, so the delta is incremental
distant-terrain cost:

| phase | both bodies | nearest only |
| --- | --- | --- |
| planet_surface | 53.938ms | 54.170ms |
| ascent | 16.661 | 16.658 |
| transfer | 16.660 | 16.657 |
| descent | 16.693 | 16.675 |
| moon_surface | 16.659 | 16.662 |

Every phase within 0.04ms. `planet_surface` at ~19 FPS appears in *both*, so it is pre-existing and
not this work — which is exactly what a control is for, and Codex built one rather than asserting.

**It is genuinely opt-in.** `system_flight` is an `Option` built only for this scenario, and `render`
early-returns into it, so the ordinary path is untouched — confirmed by the planet controls staying at
max pixel difference 0 and all three moon scenarios passing. 471 tests, clippy and fmt clean.

The `with_body` thread-local is the neat part: per-body CPU work runs in a scope while each pipeline
keeps its generated constants, so the radius never has to become a uniform. **The header's claim that
rendering both at once requires that is now wrong**, and has been rewritten.

**What is not signed off**, and Codex said so itself rather than being asked: normal interactive
flight is still single-body; the planet departure is ~19 FPS; streaming spikes and visual acceptance
are open. `planet_to_moon` asserts only that seven screenshots exist — the clearance, chunk counts and
frame times all come from `scripts/report-system-flight.py`, which nothing runs automatically. A
scenario that asserts almost nothing is the same shape as `moon_crater_wall` passing while rendering
black.

---

## 6 September 2026 — the moon's surface markings

A whole-body texture from the processes that make one, driven by the craters rather than laid over
them. Three, in the order they read at a distance.

**Maria.** The Moon's two terrains differ by nearly a factor of two — anorthositic highlands around
0.13, mare basalt around 0.07. The maria *are* the basins: impacts deep enough to crack the crust,
later flooded by basalt that welled up through it. So on this body low ground is mare ground, and the
depth already in the height field says where. Free: the shader has the macro height.

**Ejecta haloes.** Freshly excavated material is bright, and solar wind and micrometeorite gardening
darken and redden it over hundreds of millions of years. So a halo's strength is the crater's *age*,
not its size — `Crater::freshness`, cubed from a deterministic draw, so roughly one in eight is
meaningfully bright and one in a thousand is a Tycho. The halo's edge is pushed in and out with
azimuth, because an ejecta blanket is not a circle.

**Rays.** The finest ejecta, thrown furthest, from young craters only — rays are the first thing
weathering erases. This is what makes Tycho visible from a garden on Earth.

### Getting the rays to look like rays

Two harmonics and a power gave evenly spaced spokes of constant width — a wheel, not a splash. Four
incommensurate harmonics with per-crater phases gave irregular spacing and one-sidedness, which is
what an oblique impact does, but raising an *unnormalised* sum to the fifth power crushed them to
nothing: four terms rarely align, so the peaks were around 0.6 and 0.6^5 is 0.08. Normalising to the
amplitude sum and thresholding with a `smoothstep` instead of a power sets where a ray starts and how
hard its edge is without dimming the peaks. A second modulation *along* the ray makes it a chain of
bright clumps rather than a painted line, which is what ballistic ejecta landing in secondary craters
actually leaves.

### The count is a cliff, not a budget

`ALBEDO_CRATER_COUNT` is 96 and the ceiling is measured: 128 markings cost nothing at all — 16.8ms
against a 16.6ms baseline — and 160 cost **130.7ms**. That is not a slope. It is the dynamically
indexed `const` arrays falling out of whatever the driver holds them in, so the number to respect is
the edge, not the average. 96 leaves room under it, and the real Moon has perhaps a dozen ray systems
worth the name.

Before that there was a worse one, and it is the reason `MAX_MARKING_REACH_RADIANS` exists. Fourteen
rim radii of a 0.30 rad basin is **4.2 radians, more than pi**, so `cos(min(reach, pi))` saturated at
-1 and its cutoff rejected nothing: every pixel ran an `acos`, an `atan2` and a `pow` for every large
marking, and a 54-second replay had not finished after fifty minutes. A test now asserts every
marking's reach can actually reject something.

Markings come from the **baked** catalogue's largest 96, not the runtime one — a halo has to sit on
the crater that threw it, and the two catalogues are different bodies' worth of impacts. A test pins
that too.

Planet unchanged: max pixel difference 0 on three controls. Moon scenarios all pass. Moon orbit holds
16.7ms against 16.6ms without markings.

### `--scenario` now says what you meant

`unknown scenario '<name>'` and nothing else sent you to read `scenario.rs` for a list that was
already in it. Hyphens for underscores is the easy mistake — every name uses underscores — so the
error names the nearest few and says how many exist:

```
unknown scenario 'planet-to-moon'. Did you mean 'planet_to_moon'? (76 available, all using underscores)
```

A suggestion is only worth having if the list behind it is right, so the match arms and the name list
are now generated from **one** table by a `scenarios!` macro: a name cannot be loadable but unlisted,
or listed but broken, and adding a scenario is one line. `every_listed_scenario_loads` walks all 76.

Nonsense gets a count rather than a confident wrong guess — the suggestion is gated on edit distance
against the name's own length.

**A method note worth more than the feature.** The first attempt dropped 25 of the 76 arms, because
the extracting regex only matched single-line arms and rustfmt had wrapped the longer ones in braces.
It compiled, and I "verified" it by running the same regex against `HEAD` and comparing — which is
circular, and reported "identical sets" on two equally incomplete lists. What caught it was the test
suite; what should have caught it is the check I did second: compare against `ls crates/app/scenarios`,
a source the regex has no part in. **Verify against something the method under test did not produce.**

---

## 6 September 2026 — three things from a video, all reproduced

### The white line is the ground seen edge-on, and the underground start is why

Reported as a horizontal white line appearing as the camera pitches to vertical. Caught by capturing
**every** frame from 330 to 419 rather than the scenario's seven: six frames carry it, at rows 110,
132, 197, 228, 249, 253 — descending as the camera turns, which is the horizon sweeping down the view.

It is **exactly one pixel tall**, full width, and `(191,191,191)` light grey with sky both sides —
terrain, not sky, not a shader artefact. That is the ground plane seen precisely edge-on: from a point
*on* a surface, the surface subtends no thickness and draws as a hairline where it crosses the frame.

Confirmed by removing the cause rather than by argument: raising the departure clearance from 2m to
12m gives **0 frames with a line, against 6**. Ian's guess, and it was right.

### Why nobody noticed the camera was underground

`planet_clearance` in the two-body log is **2.0 exactly, every frame**. It is not a measurement. The
route sets `departure = radius + planet_height + 2.0`, and the log then computes
`planet_altitude - raster_surface_height_meters_at(...)` from the same query. The answer is 2.0 by
algebra whatever the ground does — an assertion that cannot fail, the same shape as `moon_crater_wall`
passing while rendering black.

What it is 2m above is the **CPU height field**, not the drawn surface, and those differ — that gap is
what `mountain_ground` exists to measure (median 0.056m, max 0.303m there, more on rough ground). The
start height is also taken with `prepare_flight_start_surface_height_meters(DVec3::X, 0.0)` — camera
altitude *zero*, before anything has streamed — while how much detail the surface carries depends on
camera distance.

Two fixes, and they are different sizes. Raising the constant clears the symptom. The real one is to
measure clearance from the **depth buffer**, as the surface probe already does, so the number can
disagree with the pose that produced it.

### The mystery object is the sun, and the reason it looks wrong is the exposure

Proven by removal, twice: with `sun.draw_disc` commented out, the pure-white pixels in the transfer
frame go 191 to **0** and the exact pixel turns black; in the *blue sky* frames the dot's pixels turn
plain sky `(123,157,201)`. Its 13-16px size matches the drawn sun's 12.7px.

Ian was right that it does not look like the game's sun, and the reason is not atmosphere — my first
answer. `system_flight` calls `set_auto_exposure_enabled(false)` and pins exposure to **1.0**. In play
the eye adapts, so in daylight the sun stops down to soft glare; here it cannot, so it reads as the
same hard little disc in blue sky and in vacuum alike. That is deliberate — frames across five phases
are not comparable under a moving exposure — but it does mean **`planet_to_moon` is not showing you
the game's lighting**, and no capture from it should be judged as if it were.

Along the way I claimed, wrongly and twice, that the sun disc was old and just newly *visible*: first
from "you have never been in deep space before", then from older captures having zero saturated
pixels. The second was framing, not absence — `stare_at_sun` renders it on the ordinary path at 52
saturated samples. Only the removal test settled it.

## 7 September 2026 — lunar walking/flight collision detail-distance repair (Codex)

User evidence: `test-runs/manual/1788769017-335037/screenshots/capture-001.png`,
from `a4413da`. This is the underside/ribbon view, not the two-body replay.
The frozen manual spatial log contains the old startup pose, **not** the captured
pose. Recovered the actual camera by trilaterating the depth probe's hit positions
(`direction * (moon_radius + rendered_height)`) and hit distances. The recovered
position is `[805339.4703311387, -725635.8310803904, -112202.36658197352]`, altitude
9820.348905 m; maximum reconstructed distance residual is 2.8e-11 m. A static
replay reproduces the user's underside view.

Cause: `planet.wgsl::vs_main` now filters runtime detail by distance to the
**macro-displaced** vertex. `raster_mesh_surface_height_meters_at` still used the
undisplaced datum sphere. Here that is ~9820 m versus ~83 m, with 98 m versus
0.83 m distance-filter floors. The collision triangles omitted detail the GPU
actually drew. Both walking and low-flight collision use this same mesh query.

Fix: calculate each collision vertex's macro-displaced distance from its source
sample, matching the existing shader, before evaluating its detail height. No
camera-offset inflation, terrain/shader changes, rebake or flight-speed change.
The focused regression fails with the former distance and passes with the fix.

Evidence (all under `test-runs/`):

- Static before: `moon_camera_clearance/1788769531-336570`: reproduces underside,
  reports falsely safe **+1.7 m** clearance.
- Same fixed pose with corrected query: `moon_camera_clearance/1788769616-337102`:
  **-6.421516 m**, correctly identifying that pose as underground. These first
  two runs used the initial static diagnostic definition, before the scenario
  was upgraded to drive real walking.
- Real walking: `moon_camera_clearance/1788769694-337335`, passes. Waits then holds
  W from 2 seconds. Capture clearances **1.700 / 1.776 m**; median depth-hit versus
  CPU field differences **0.062 / 0.129 m**. The nearby overhead skirt lattice is
  gone and the foreground is above-ground terrain.
- Real flight: `moon_flight_clearance/1788769707-337369`, passes with the same pose
  and downward-facing W input. Capture clearances **64.572 / 6.891 m**; median
  depth-hit differences **0.286 / 0.018 m**. This site's very low sun leaves much
  of the flight capture dark; these are collision regressions, not lighting or
  terrain-seam sign-off.

Scenario support adds optional `walk_on_surface` to the existing held-W replay,
so the walking test uses real surface physics and post-stream correction, not an
interpolated camera. Existing scenarios default to free flight. The actual fix
is shared by interactive walking and flight, not confined to these replays.

Reproduce with `target/release/catinthegarden-app --body moon --scenario
moon_camera_clearance` or `moon_flight_clearance`. Fresh human walking/flying over
other lunar terrain remains useful; no claim is made here to have repaired all
terrain seams, geomorphing or source-window discrepancies. Bakes and the user's
untracked `crates.tar.gz` remain untouched.

Final validation: **481 workspace tests pass, 12 ignored**; workspace/all-target
clippy with `-D warnings`, fmt and diff checks pass. Final guarded GPU runs
`moon_camera_clearance/1788770071-339609` and
`moon_flight_clearance/1788770084-339655` both pass. They additionally require at
least 20 independent depth comparisons and <=2m p90 depth/CPU disagreement;
walking also caps eye clearance at 2.5m. The reproduced pre-fix view's p90 was
4.051m, so clearance alone is not the only acceptance signal.

## 7 September 2026 — ocean normal/buoyancy slope parity (Codex)

The user reported a fast surface flowing over broad waves, overly smooth crests,
and occasional straight normal seams, then approved fixing the proven normal
mismatch first. **This is that bounded fix, not sign-off on all three symptoms.**
Terrain/tree rendering is being worked on concurrently. Only ocean functions in
`shared_planet.wgsl`, ocean test code and documentation were changed; no
`planet.wgsl`, terrain, forest, camera, scenario or wave-table edits.

Two GPU slope errors differed from the already-correct CPU `global_wave_slope`:

1. Phase is `k * dot(direction, axis) * R`. Its tangential gradient contains
   `axis - direction * dot(axis, direction)`, **not the normalized tangent**.
   The latter exaggerated slopes near the wave-axis poles. Horizontal
   displacement retains its original unit tangent; only the slope changes.
2. The soft depth limiter multiplies height by `w=(1+q)^(-1/n)`, where
   `q=(abs(h)/L)^n`. Its derivative is `w/(1+q)`, not `w`. GPU normals now use
   that derivative, matching CPU buoyancy/velocity. This reuses the existing
   power calculation rather than adding another `pow`.

Wave height, horizontal displacement, phase speeds, amplitudes, mesh density,
crest shape and camera/ship motion are unchanged. No extra texture fetches or
render passes; no FPS improvement is claimed.

### Regression / validation

`ocean_gpu_tests.rs` executes the **production WGSL `ocean_surface`** in a Vulkan
compute pass and reads its normal/height back. It compares against CPU buoyancy
for four directions, three depths (2/20/4000 m) and two times. This is not a Rust
copy standing in for the shader. It uses a deliberately small 64m test radius to
isolate slope algebra from real-planet f32 phase-reduction error, and full
near-field geometry weight with the actual default's shading-only ripples off.
It does not test distant fade gradients, varying bathymetry gradients or
horizontal-transport inversion.

On the Quadro M1000M it **fails before and passes after**:

- Maximum unit-normal vector difference: **0.182566620 → 0.0001589715**
  (approximately 10.5° → 0.009° across these fixtures).
- Maximum height discrepancy against CPU: **0.000180228 m**, unchanged before/
  after. Production height expressions were not edited.
- Non-finite normals/heights also fail the test. The normal tolerance is 0.002,
  height tolerance 0.02m. The GPU test is explicit/ignored in ordinary CI:
  `cargo test -p catinthegarden-app gpu_ocean_normals -- --ignored --nocapture`.
- 481 ordinary workspace tests pass, 13 ignored (including this new GPU test);
  explicit GPU test, workspace/all-target clippy `-D warnings`, fmt and diff
  checks pass. Release executable rebuilt from this worktree.
- Release Quadro `ocean_ship_float/1788775915-349377` passes, three PNGs and 17
  finite spatial samples. Capture 003 was inspected: boat and wave geometry
  render correctly. This is a shader/runtime smoke test, not temporal visual
  sign-off or a matched FPS comparison. Manifest names parent `23f4a84` because
  it ran with the uncommitted normal fix.

Next: user motion review of the corrected lighting; investigate geometry versus
per-fragment wave bandwidth and source/phase continuity for the straight seams.
Crest sharpening is still separate. Do **not** simply turn horizontal Gerstner
transport back on: CPU collision currently assumes radial water and would need
its corresponding inverse query. `crates.tar.gz` and other developers' work
remain untouched.


## 7 September 2026 — bounded ocean crest shaping (Codex)

User requested a small crest-sharpness pass with limited remaining usage.
Only ocean.rs/shared_planet.wgsl runtime code changed. Each radial sine now
uses `(s + 0.4*(s*s-0.5))/1.2`, where `s=sin(phase)`. This zero-mean
second harmonic gives each individual crest 1.5x sine curvature and broader,
shallower troughs while retaining the conservative amplitude bound. Composite
wave heights do change; this is not a claim of 50% sharper final rendered seas.
CPU height, spatial slope and vertical velocity share the profile and its
analytic derivative; WGSL uses the CPU-generated sharpness constant. Horizontal
transport remains OFF. No wave speeds, wavelengths, mesh density, octaves,
textures, terrain or trees changed. GPU adds only arithmetic on existing sin/cos;
no FPS comparison was made.

Focused ocean suite: 34 passed, 2 ignored. Explicit production-WGSL Quadro test:
24 cases passed, maximum normal-vector error 0.000223101 and height error
0.000119712m against CPU. New regression covers bounds, zero mean, curvature
and finite-difference derivative. Release rebuilt; ocean_ship_float replay
`1788776446-352624` passed and produced three PNGs. Compared capture-003 with
prior `1788775915-349377`: crest shape changes are visible but restrained;
live motion/user visual acceptance remains pending. Straight seams are NOT
addressed by this pass. Untracked crates.tar.gz remains untouched.
App all-target clippy (`-D warnings`) and workspace fmt check pass. Full workspace
test attempt was interrupted (exit 143) before a summary; do not count that
attempt as a full-suite pass. Focused ocean and explicit GPU results above are
completed runs.


## 7 September 2026 — trees are planted on one surface and the ground is drawn on another

Ian, walking in a forest: the trees "kind of bounce up and down relative to the ground, it's subtle
but it's there", and compared it to the boat lag. He is right, and it is the same class of bug
`c99787a` had just fixed for the camera — a consumer of terrain height using a different detail
filter from the one the shader draws with. The camera was fixed; the forest was not.

**The mechanism, end to end.** A tree's ground height is computed on the CPU by
`forest_surface_sample_at` -> `outmap_surface_height_meters`, which pins the detail filter to
`TERRAIN_DETAIL_MIN_FILTER_METERS` (0.5m) regardless of distance — full detail, everywhere. That
height is then baked into the instance buffer as a plain `surface_height: f32` (`forest.wgsl:100`,
position = `direction * (PLANET_RADIUS_METERS + max(surface_height, 0.0))`) and stays fixed until the
patch rebuilds. The ground *under* it is displaced by the terrain vertex shader at
`vertex_filter_meters = max(terrain_detail_filter_meters(detail_distance), edge_detail_filter_meters(...))`,
where `terrain_detail_filter_meters` is `max(distance * 0.01, 0.5)` — refiltered every frame as the
camera moves. The tree is nailed to one surface; the ground beneath it is a different, moving one.

**Measured**, walking `forest_travel` with `walk_on_surface`, gap = planted minus drawn, at anchors
held at fixed distances ahead while the camera walked 87m:

| distance | gap |
| --- | --- |
| 50 m | +0.015 m |
| 150 m | -0.010 -> +0.019 (stepped at 33m walked) |
| 300 m | -0.194 m |
| 600 m | +0.898 m |
| 1200 m | +1.006 m |

The offset grows with distance because that is where the two filters diverge: inside ~50m both sit on
the 0.5m floor and agree to 15mm, which is why the artefact is subtle rather than obvious. The
*bounce* is the drawn ground stepping between filter levels under a stationary tree. One 29mm step at
150m was caught in an 87m walk; the same anchor in the fast `forest_travel` flight showed the drawn
ground jumping by up to 20.8m across LOD transitions. **Not characterised:** how often it steps at
normal walking pace — 87m only crossed one boundary.

**The fix.** Have the tree's vertex shader compute its ground height with the terrain's own
displacement code, instead of carrying a CPU-computed f32 that goes stale. Then tree and ground move
together and the relative motion disappears; the tree's absolute height still changes with distance,
but only motion *against* the ground reads as a bounce. This is viable because the displacement is
purely procedural — no texture reads — so the drawn height is
`base_height + terrain_detail(dir, local, filter(distance), spacing, base_height).height_meters`,
and a tree knows its own distance. Note `vertex_spacing_meters` does **not** bound the vertex
displacement (it is only passed to the fragment stage at `planet.wgsl:888`), so distance is the whole
of it away from node edges.

**The obstacle, which is why this is a handoff and not a commit.** The forest shader cannot simply
include the terrain shader: `forest.wgsl` declares `@group(2)` bindings 0-3 and `shared_planet.wgsl`
declares `@group(2)` bindings 3-14, so they collide on binding 3. Only four symbols actually clash —
`planet_to_view`, `srgb_to_linear`, `Camera`, `camera` — so the route is to extract the detail ladder
into its own `terrain_detail.wgsl` included by both `shared_planet_shader_source()` and
`forest_shader_source()`. The block is contiguous (`shared_planet.wgsl` 597-902: `DetailNoise`
through `terrain_detail_filter_meters`, including `baked_sample_spacing_meters` and
`terrain_vertex_spacing_meters`) plus the `TERRAIN_DETAIL_*` constants and `terrain_detail_octave_tilt`
near the top of the file. `terrain_macro_height_scale` must **stay** in `shared_planet.wgsl` — it
reads the `camera` and `terrain_settings` uniforms.

**Verification the extraction should carry:** it is pure code motion, so the terrain must render
pixel-identical before the forest is touched. Then the instance needs the macro height and the baked
sample spacing rather than the final height, and the shader adds the detail itself.

**One precision caveat to watch.** `terrain_detail_band` splits its noise coordinate into an anchor
cell index plus a local offset precisely because f32 quantises an absolute 4e6 domain coordinate to
0.25. The terrain uses the node's anchor plus the vertex's local metres. A tree using its own
direction as the anchor with a zero local offset will mis-register the finest octaves against the
terrain's registration by up to half a cell — but those octaves have amplitude
`wavelength * TERRAIN_DETAIL_ROUGHNESS`, so at 1m wavelength that is a few centimetres, against the
0.9m error being removed. Worth measuring rather than assuming.

Nothing was committed for this. The instrumentation and the temporary `forest_walk_probe` scenario
used to take the measurements above were removed; the tree is clean.


## 7 September 2026 — underwater: extinction and the underside, and why neither can be seen yet

Ian asked for three things when the camera bobs under: 20m visibility, the waves
visible from below, and the sea bed visible in shallow water. Two of the three
are written and neither can be confirmed on screen, because **every underwater
frame is a single flat colour** and I could not find what draws it.

**What is implemented** (both gated on the submerged flag, so the above-water
render is provably untouched -- `ocean_hybrid_close` passes and 413 tests pass):

* `terrain_fog` gains an underwater branch. Water extinguishes over metres where
  air takes kilometres, so the atmosphere's path integral is the wrong *medium*,
  not merely the wrong amount. The e-fold is `20 / ln(50)`, since visibility
  conventionally means the range at which contrast is down to 2%. The colour is
  the sky radiance overhead times a blue-green tint, so the water darkens at
  night rather than being painted.
* `ocean_underside_colour`. `ocean_lighting` cannot draw the underside: it takes
  `max(dot(normal, view), 0.0)`, which is zero across the whole surface from
  below, so Fresnel is constant and the reflection samples one texel of a
  1x1-per-face cubemap -- a flat colour with no waves in it. The replacement is
  Snell's window: light from the whole sky refracts into a cone of half-angle
  `asin(1/1.333) = 48.6 degrees` about the *local* normal, so the cone's edge
  follows every wave, and that moving boundary is the shape you see.

**The blocker -- FOUND AND FIXED, see the entry below.** It was `fs_ocean` and
`fs_ocean_stable` returning a hardcoded `underwater_colour()` for every back
face, before any shading ran. What follows is the trail that led there, kept
because the negative results are the useful part.

Underwater the whole frame was `(15, 64, 117)`, uniform, at any depth from 1m to
25m over deep water. Each of these was tested by forcing the branch and
re-rendering:

* The submerged flag is **correct**, not the problem: logged from the CPU as
  `sea_level_altitude 20.72, surface_height 21.72, submerged 1`.
* Geometry **is** drawn: the surface probe reports 81 hits from 81 points, the
  nearest at 1.12m and screen centre at 1.72m.
* `atmosphere.wgsl` `fs_main`'s submerged branch forced to return pure red:
  **frame unchanged**. The sky pass never covers these pixels.
* `ocean_fragment_color`'s new underside branch forced with `if true`:
  **frame unchanged**. The ocean shell's fragment shader is not producing them.
* Underwater visibility set to 100000m instead of 20m: **frame unchanged**, so
  the fog branch is not producing them either.
* The render pass clears to `Color::BLACK`, so it is not a clear colour.
* Path is raster, `final HDR scene`, no experiments enabled.

So something that fills the screen and is none of those three shaders is drawing
it. The two suspects I did not get to are `terrain_fragment_color`'s blended
water (`planet.wgsl` circa 1856 and 2130, which shades water inside the terrain
pass rather than the ocean shell) and a post stage (`hdr.rs` tonemap, or the
foveated unwarp). The quickest discriminator is to force a colour in
`terrain_fragment_color`'s water branch the same way.

**To reproduce.** A scenario at `ocean_hybrid_close`'s position direction,
radius `4_000_000 - 3`, `waterline_eye_height_meters: -1.0` so the eye tracks
1m under the moving surface, looking 25 degrees above horizontal. Note that
scenario JSON reaches the binary through `include_str!`, so editing the JSON
without touching a `.rs` file will silently replay the previous scenario -- that
cost me two runs and a wrong conclusion about the camera being buried in land.

**Also unverified:** the third request, the sea bed in shallow water. Bathymetry
is real geometry (the bake carries -5000m and negative macro heights are not
scaled), so it should appear once anything underwater renders at all, but I
never found a shallow site to confirm it.


## 7 September 2026 — the underwater frame was a hardcoded constant

The flat colour under water was `fs_ocean` and `fs_ocean_stable`:

    if !front_facing {
        return underwater_colour();   // vec4(0.012, 0.055, 0.13, 1.0)
    }

Every back face of the sea -- which is the entire thing a submerged eye sees --
returned one constant *before* reaching `ocean_fragment_color`. Its own comment
said so: "A placeholder until there is a real underwater pass." That is why
forcing the atmosphere branch to red, forcing the underside branch on, and
setting visibility to 100000m all changed nothing: none of them were on the path.

What found it was painting each candidate a different colour in one run --
terrain red, the ocean shell green. Terrain came back red over 2.4% of the
frame, green never appeared at all, and an ocean shell that draws zero pixels
while the probe reports 81 depth hits of 81 is only possible if its fragments
return before the code being edited. I had been reasoning about which pass
*covered* the screen when the question was which pass *authored* the pixels.

Back faces now shade through `ocean_underside_fragment`, which applies the same
open-ocean ownership rule as the lit side and then `ocean_underside_colour`
under the underwater fog. Keying it on `front_facing` rather than the submerged
uniform is also better: it is geometric, so it cannot disagree with the CPU.

**Two further corrections, both caught by rendering rather than by reading:**

* The Snell's window cosine had the wrong sign. `view_ray` runs from the eye up
  to the surface and the normal points out of the water, so looking straight up
  is `dot(view_ray, normal) = +1`; I had negated the ray, which made it negative
  everywhere, clamped to zero, and left the whole underside in the dark
  total-internal-reflection branch. Fixed, the window appears: a bright sky disc
  with a dark rim, 643 distinct colours where there had been 1.
* Folding the ripple layer into the underside normal changes nothing visible at
  1m depth, and should not: the window is about 2.3m across there and the
  shortest wave in the spectrum is 7m, so a smooth boundary is correct. It is
  depth that widens the window enough for waves to distort its edge. The comment
  in the shader says this rather than claiming a fix it did not make.

**Still open.** The sea bed in shallow water is unverified -- bathymetry is real
geometry, so it should appear now that the underside draws, but I never found a
shallow site to confirm it. Aiming the probe 25 degrees above horizontal puts
the camera outside Snell's window entirely, which reads as almost black and is
correct; use 70 degrees to see the window. Reproduction is as described in the
previous section, with the reminder that scenario JSON reaches the binary
through `include_str!`.


## 7 September 2026 — the 1000ms frames are a memory-pressure symptom, not a render one

Recorded because it cost most of a session's verification time and because the
thread has read "cause unknown" since July.

The stall reproduced continuously for hours. What rules out the renderer: six
consecutive `ocean_rough_horizon` runs of one unchanged binary gave 861, 31,
977, 1000, 1000 and 909 ms/frame. A single 31ms run between two 1000ms runs,
with nothing edited, is not something a shader can do.

What rules out the GPU: `nvidia-smi` during a 1000ms run reported 40%
utilisation, P5, 797MHz of a 1124MHz maximum, 44C, one process resident at
720MiB. A GPU that is not busy while frames take a second means the CPU is
blocked, not the pipeline.

What points at memory: the harness killed two background waiters for low memory,
and `free -m` showed swap at 976MB of 976MB with **zero free**, RAM 9.1GB of
15.4GB in use. Faulting against a full swap produces exactly this signature --
long stalls, wide run-to-run variance, an idle GPU, and the occasional fast run
that happens to find everything resident.

This is a lead, not a proof: I did not free the swap and re-measure, because the
machine's memory was in use by the user's own desktop at the time. The test is
one command once there is headroom.

Worth carrying into how this repo is verified: scenario replays are the slowest
part of any change here, and their timings are only meaningful when the machine
is not swapping. A timing that looks like a regression should be checked against
`free -m` before it is believed.

## 7 September 2026 — crest transmission and visible shallow bathymetry

Based on 20587bf on experiment/ocean-wind-sea-spectrum; preserved the other
developer's crest, dispersion, foam and underside work.

* Shared ocean lighting now accepts wave height. Positive height fades a bounded
  turquoise, forward-scattered direct-sun term in over 0–8m, suppressed by
  Fresnel and darkness. All five callers pass wave displacement; foam still
  covers it. This is an inexpensive thin-crest approximation, not measured
  thickness or alpha transparency. Geometry, buoyancy and depth are unchanged.
* Existing underwater extinction now targets 2% contrast at 30m (previously
  20m). Submerged raster open-ocean fragments shade the baked negative-height
  geometry with the existing beach palette before the old ocean discard.
  Sun/sky illumination attenuates with bottom depth, then the existing
  per-pixel view-distance water fog applies. No new mesh, draw or render pass.
* The existing underside gives an underwater ray its exit surface and fog
  distance. This does NOT introduce above-water seabed refraction or a true
  refracted split-camera compositor. Those are separate work, and the camera
  medium selection still uses the existing CPU submerged flag.

Validation: 414 app tests passed (11 ignored), 28 shader-filtered tests passed,
explicit Quadro GPU wave parity passed (normal error 0.000089958, height
0.000058081m), app all-target clippy, fmt and diff checks passed. Adding the two
scenarios initially failed the hardcoded scenario count; updated 78 to 80 and
the complete-list loading regression then passed. Release rebuilt.

GPU scenarios, all passed, four captures each:
* ocean_hybrid_close/1788800036-415020 — inspected turquoise backlit crests.
* ocean_underwater_visibility/1788800123-415609 — inspected Snell-window rim.
* ocean_shallow_bottom/1788800342-417178 — inspected visible brown sediment relief.

Shallow fixture provenance: px L4 x15/y14, stored height sample (19,39),
-10.07443m, ocean biome; fixed eye at -7m, looking downward. Do not use
waterline_eye_height_meters here: waterline_scenario_pose assumes 4000m depth
and can drive the camera beneath this shallow bottom. The initial run
1788800195-416229 did exactly that and was flat blue despite passing its
generic assertions. The corrected capture visually verifies bottom rendering;
the scenario's generic finite/screenshot assertions alone do not prove it.

No measured FPS claim or temporal visual sign-off. Monitor was on, but swap
was nearly full and other desktop work was active. Unrelated crates.tar.gz
was left untouched.

## 7 September 2026 — gentler crest colour ramp

User likes the turquoise but finds its transition sudden. Extended the existing
smoothstep from 0–8m to 0–24m, keeping the zero-height onset, zero contribution
at onset, and maximum transmission colour/intensity unchanged. Geometry, motion,
underwater rendering and lighting directionality are untouched. Focused crest
regression, Naga planet shader validation, fmt and diff checks pass. No new GPU
capture or performance claim; visual acceptance remains pending.

## 7 September 2026 — slower linear turquoise interpolation

The user still found the 24m smoothstep too abrupt. Replaced it with
clamp(height / 48m, 0, 1): unchanged onset and maximum transmission, twice
the height span, and one third of the previous maximum ramp derivative.
This is still the existing lighting contribution, not alpha transparency.
Updated the focused shader regression. No new visual sign-off or FPS claim.
Concurrent changes in debug.rs, scenario.rs and ocean_shallow_bottom.json
are not part of this change and must remain unstaged.

Focused crest regression and Naga shader validation pass; release rebuilt
from the active worktree, including its concurrent uncommitted edits.

## 7 September 2026 — verifying the crest/seabed work, and arming the scenario that could not fail

Review of `2dfe3cb` from the relay, on a machine that finally had memory headroom.
Everything below is measured on this branch, not taken from the report.

**What holds.** 414 app tests, 11 ignored — exact. The seabed does render: at the
scenario's sample point the good capture is `(66, 47, 25)`, red leading blue by
0.161, against ocean blue's -0.56. It is genuine structure and not a fog
gradient: a pure vertical gradient explains only 55% of the luminance variance,
and the horizontal residual autocorrelates at 0.9995 at lag 1 and 0.76 at lag 64,
so it is smooth relief rather than noise. `is_open_ocean_surface` guards on
`BODY_HAS_OCEAN` internally, so the new call site cannot reopen the black-moon
defect. `flat_triangle_options.w` really is a single-writer channel: only
`main.rs:3508` sets it, everything else leaves the constructor's 0.0.

**The frame cost is nil, which I nearly got wrong.** `ocean_ship_float` now runs
~34.8ms median where runs before this work were ~29.3ms, and that looked like a
19% regression to charge to the crest shader. It is not. Reverting the three
`.wgsl` files to `20587bf`, rebuilding and re-running gave 34.68, 34.70 and
34.70ms against HEAD's 34.76, 34.66, 34.80 and 34.64 — the same number. Crest
transmission and the seabed branch cost nothing measurable. The 29 to 35ms shift
is real but older than this change and belongs to a binary nobody kept. Same
scenario, same machine, same session, swap free, monitor on.

**Thread 18 is closed** — see the header. Five runs, no stall, swap at 51MB of
976MB instead of 976 of 976.

**The defect: `ocean_shallow_bottom` could not fail.** Its assertions were
`require_finite_metrics` and `expected_screenshots: 4`, which is exactly what the
abandoned run `1788800195-416229` passed while rendering flat blue. A scenario
whose whole purpose is to prove the sea bed draws, and which goes green when it
does not, is worse than no scenario: it is a false witness. This is the third of
its kind here, after threads 4 and 17.

Armed with `seabed_sample_uv` and `min_seabed_red_minus_blue`, following the
`ice_sample_uv` pair. Sediment lit through water is warm, open water is not, so
the sign of red-minus-blue is the whole signal. Threshold 0.08 at frame centre,
sitting between +0.161 and -0.56 with room on both sides.

Proved by breaking it on purpose, which is the only thing that proves a guard:

| build | assertion | sample | margin |
| --- | --- | --- | --- |
| seabed branch on | pass | `(66, 47, 25)` | +0.161 |
| `if false &&` on the branch | **fail** | `(15, 64, 117)` | -0.400 |

`(15, 64, 117)` is the constant slab colour this file already names as flat
ocean. The shader was restored, rebuilt, and reproduces +0.161 exactly.

**Validation.** 486 workspace tests (14 ignored), fmt clean, clippy clean on all
three crates checked separately. `ocean_hybrid_close`, `ocean_underwater_visibility`,
`ocean_rough_horizon` and `stand_on_ground` all pass, so the blast radius is nil —
the new assertion fields default to `None`.

**Two new threads, 26 and 27:** the sea bed is raster-only with no raymarch
counterpart, and `ocean_underwater_visibility` has the same missing teeth that
`ocean_shallow_bottom` just had.

**A note on the harness.** The first run after every rebuild produced no
screenshots and a null `passed` — 14 log rows and one frame. The retry was clean
every time. Related to thread 13; a null result is not a pass and not a failure,
and nothing says so.

## 7 September 2026 — the foam's straight edge, a darker sea, and a debug mode mistaken for a bug

Three reports from Ian, all on the raster path, all confirmed by rendering rather than by reading.

**The foam cut off in a straight line, and the crest gate was why.** `ocean_foam_coverage` multiplied
the whitecap slope term by `smoothstep(0.0, OCEAN_WHITECAP_CREST_METERS /* 4.0 */, h)`. Painting the
two factors into separate channels of one frame settles which one owns the edge: the slope term is
graded, with 31.7% of sea pixels mid-ramp; the crest gate is a step, with 49% of the sea at exactly
zero and 6.3% anywhere inside the ramp. A 4m ramp against a sea whose crests run to 53m is not a
fade. Its edge is the contour h = 0 -- the mean-water line -- which is why it reads as a straight
line across the swell, and the same frame shows the slope term wanting foam *below* that line and
being killed by it.

Replaced with `OCEAN_WHITECAP_CREST_LOW_FRACTION` (-0.08) and `OCEAN_WHITECAP_CREST_HIGH_FRACTION`
(0.18) of `OCEAN_MAXIMUM_WAVE_HEIGHT_METERS`: 3.4x wider, starting below mean level so there is no
zero-crossing at a level contour, and expressed in the sea's own scale so it keeps its meaning when
`OCEAN_WAVE_SCALE` changes. The first attempt used a 0.25 high fraction and cost 34% of mean foam
intensity for no reason; 0.18 keeps the softened edge and gives the intensity back. Foam coverage on
`ocean_rough_horizon` 21.9% before, 22.3% after.

**A darker sea.** `OCEAN_BODY_COLOUR`, a new named constant, (0.005, 0.032, 0.170) against the
inline (0.008, 0.055, 0.28) it replaces. Measured 16.3% darker in mean sea luminance -- much less
than the 39% cut to the constant, because glint, sky reflection, foam and the transmitted crest are
all added on top of the body rather than mixed into it. Worth knowing before anyone tunes it again:
raising this constant flattens all four of those at once.

**The turquoise now starts above mean level.** `OCEAN_CREST_TRANSMISSION_ONSET_METERS` 8.0 to
`_FULL_METERS` 24.0, still linear. Transmission is a property of a thin crest and water low on a
wave is not thin. Note the interaction: turquoise coverage went *up* (3.2% to 7.7% of sea pixels
green-dominant) even though the ramp starts higher, because the darker body makes green win over
blue more easily. Colour changes here are not independent.

**The sea rendering flat-shaded at low LOD is NOT a normals bug.** It is the flat-triangle debug
mode. `test-runs/manual/1788802885-453933/manifest.json` records `"render_debug_mode": "flat L7
triangles"`; `flat_ocean_colour` shades from `@interpolate(flat) face_normal` deliberately, "which
is what the low-poly presentation wants". Reproduced the same pose in the normal mode as the new
scenario `ocean_eye_level_facets` (82 scenarios now): smooth, at matching conditions -- 17m
altitude, 248 of 255 chunks on fallback tiles, `budget_limited` true, waves -36.1 to +38.8m.

**The trap that cost the reproduction, and it will cost the next one too.** The `render
configuration` log line carries `render_debug_mode` and is written *once at startup*. Toggling the
mode mid-session never appears in the log, so the log said "final HDR scene" for a session that was
in flat triangles. Only the per-probe `render_debug_mode` in `manifest.json` told the truth. **Log
the render mode when it changes**, or every manual report is ambiguous.

**Concurrent editing.** Two agents were in this tree at once and a whole-file backup-and-restore of
`planet.wgsl` nearly discarded the other's `ocean_underside_fragment` change; it survived only
because the backup happened to be taken after that edit, and the diff was checked before moving on.
Use targeted edits on shared files. Never restore a whole-file backup of a file someone else is in.

**Validation.** 416 app tests (11 ignored), fmt clean, clippy clean. `ocean_hybrid_close`,
`ocean_rough_horizon` and `ocean_eye_level_facets` pass. One brittle assertion fixed on the way:
the crest-ramp test pinned the literal `clamp(crest_height_meters / 24.0, 0.0, 1.0)` and failed the
moment the ramp was retuned. It now parses the two constants out of the shader and asserts
`0 < onset < full`. A guard should survive tuning it is not meant to prevent.

## 8 September 2026 — the waterline sky leak was background, not the underside

Finished from 1a4eea3, preserving the concurrent foam, dark-body and crest-onset
tuning. User captures 1788802408-449773 and 1788802656-452606 show a pale lower
screen under the waves; the latter manifest confirms final HDR scene, not flat
triangles. The HUD/probe reports +0.06m analytic eye clearance.

**Correction to my first diagnosis.** Changing the underside from terrain fog
to unconditional water fog did not remove the pale area. That change was already
included in 1a4eea3, but its source-level test did not establish the pixel author.
A red distance diagnostic in ocean_underside_fragment
(ocean_waterline_medium/1788820919-483417) coloured only narrow patches; the
large pale region stayed unchanged. It was the sky background exposed where
the finite/clipped wave mesh does not enclose the water volume. At +6cm, the
global submerged flag remains false. All diagnostic shader code is removed.

**The fix.** Reuse the existing CPU ocean-environment query to upload actual
local water-column depth and signed eye clearance in unused camera_forward.w
and camera_right.w lanes; the layout and all xyz bases are unchanged. The sky
background tests each ray against the local water plane. Submerged ocean eyes
get water background; above-water eyes get it only for downward rays entering
nearby water, with a 20–30m entry-distance fade. Geometry still renders over
this background, so the wave silhouette selects the visible boundary. Land,
airless bodies and distant/orbital cameras do not acquire a water background.

The background uses bounded unboosted sky illumination and the existing medium
tint. An overhead sun's narrow HDR sky lobe must not bleach the whole volume.
This is a local water-volume fallback, not a refracted scene or a fix for the
analytic-versus-triangulated wave-height discrepancy. No new geometry, pass,
uniform allocation or CPU wave query; no measured FPS claim.

**Evidence and guard.** At sample UV (0.5,0.9), the first two waterline captures
change from (142,155,163) to (2,37,94). Applying the new colour criteria to the
archived pre-fix PNGs rejects them; all four corrected PNGs pass. The assertion
checks every captured sample, requires blue-red >= 0.25 and red <= 0.35, and
also rejects black/missing output. It is opt-in, leaving other scenarios alone.
A matched 1280x720 diagnostic/after capture has byte-identical top 100 sky rows.

During verification I also tried correcting the existing water-fog up-vector
transform. With the shallow scenario's overhead sun that turned its fog white
and failed the sediment guard (1788821818-487445). That extra change was reverted;
the final shared fog path is unchanged from 1a4eea3. Do not reintroduce it without
addressing the direct-sun lobe in the ambient lookup.

Final rebuilt GPU runs, all explicit passed=true with all PNGs present:
* ocean_waterline_medium/1788822013-488314 — four captures, new image guard passes.
* ocean_underwater_visibility/1788822031-488380 — four captures.
* ocean_shallow_bottom/1788822045-488452 — four captures, sediment guard passes.
* stand_on_ground/1788822057-488471 — five captures.

489 workspace tests passed (14 ignored), workspace all-target clippy passed;
final focused waterline tests, shader validation, fmt and diff checks pass.
The generic underside/Snell-window assertion gap in thread 27 and ray seabed
parity in thread 26 remain separate. Release is rebuilt in target/release.
Unrelated crates.tar.gz was left untouched.

## 8 September 2026 — the turquoise is keyed on sharpness now, and two things that are not what they looked like

**Crest transmission no longer keys off height.** Ian's objection was exact: keying the turquoise on
displacement above mean level made it a property of how *tall* the water stood, so a small wave got
none however sharp its tip while a large lazy swell got it on its flanks. Thinness is what lets
light through and thinness does not scale with height.

It now keys on `OceanSurface::crest_sharpness`: each wave's dimensionless Gerstner steepness
`steepness * OCEAN_STEEPNESS_SCALE * amplitude * wave_number * tangent_length` times `sin(phase)`,
summed. It peaks exactly at the crest, where the drawn profile peaks, and being dimensionless it
does not scale with the wave's size -- which is the whole point. One fused multiply-add per
component, reusing the `sine` `gerstner_wave` already computes. **It is not the divergence of a
displacement we draw**: `OCEAN_HORIZONTAL_TRANSPORT_ENABLED` is false, so the Gerstner horizontal
term is never applied. It is the quantity that *would* pinch the crest if it were, used as a
sharpness measure. The comment in the shader says so; do not upgrade that claim.

Thresholds chosen against the measured distribution, not by eye. Sampling the summed sharpness over
random phases: calm p90 0.225, p99 0.381, ceiling 0.7279; storm p75 0.509, p90 0.950, ceiling
2.0507. `OCEAN_CREST_TRANSMISSION_ONSET` 0.25 is about the calm sea's top tenth of water and
`_FULL` 0.75 is reached only by a storm's sharpest crests. Coverage on `ocean_hybrid_close` is
7.69% of sea pixels green-dominant against 7.71% before, so the amount is unchanged and only its
distribution moved -- onto the small mid-distance and near-horizon crests, which is what was asked
for. The test now asserts `0 < onset < full < fold_budget()`, which a retune cannot quietly walk
past: a threshold above the budget is turquoise that can never appear.

**The handoff header's Gerstner fold budget of 1.17 is stale.** `ocean::fold_budget()` returns
**2.0507** today, printed from the function itself, not parsed from the table. The wave table has
been retuned since 1.17 was written and nothing updated the prose. Thread 7's "accepted and held
invariant at 1.17" should be read as "held invariant by construction, at whatever the table sums
to", and the number in it is not evidence for anything.

**Two things that looked like defects and are not, both settled by measurement:**

* *Back faces skip the LOD cross-fade cull.* `fs_ocean` returns `ocean_underside_fragment` before
  the dither discard, so during a transition both levels' undersides rasterise against each other.
  That is a real asymmetry, but applying the cull to back faces changes **zero pixels** in
  `ocean_underwater_visibility` and **zero** across all five `ocean_flyover` captures, because no
  reachable scenario is both submerged and mid-transition. Reverted rather than shipped: a fix
  nobody has seen do anything is not a fix. It needs a scenario that swims while the LOD changes.
* *The underwater "background".* Painting `ocean_underside_fragment` shows **100% of that frame is
  underside geometry** -- there is no sky or background fill visible in it at all. The probe puts
  every one of 81 sampled points at 2.9m to 9.0m, so what reads as distant background is the
  underside of the waves a few metres overhead, only 37-69% fogged at 30m visibility. Ian is taking
  this one to Codex as a separate job; the measurements above are the starting point, and the thing
  to explain is why close geometry reads as a flat far-off backdrop.

**Validation.** 489 workspace tests (14 ignored), fmt clean, workspace clippy clean.
`ocean_hybrid_close` and `ocean_rough_horizon` pass. No timing claim: swap was 976MB of 976MB for
this whole session, which is exactly the state that produces the 1000ms frames.

## 8 September 2026 — sea colour painted across a mountain, from a source tile that was not looking

Ian reported blue and turquoise terrain far inland near mountains, at 19.5km altitude with
**248 of 255 chunks on fallback tiles**. It is terrain, not the sea: the HUD reports 4 ocean chunks
in that frame and the blue area carries terrain relief.

**Mechanism.** `terrain_fragment_color` discards open sea only when the sampled texel *and* the
drawn geometry agree that this is below the datum:

    if is_open_ocean_surface(outmap, macro_height_meters, biome_id)
        && input.surface_height_and_fog_color.x <= 0.0 { discard; }

The second condition is deliberate and its comment says why -- a fallback source tile can sample a
negative texel at a fragment whose triangle was displaced from a positive neighbour, and discarding
those punches square holes in solid land. But the colour a few lines down still trusted that same
sample: `outmap_ocean_coverage(outmap, macro_height_meters)` fires for any texel in (-80, 0]. So the
fragment that was saved from becoming a hole became a patch of sea instead, and `ocean_coverage <=
0.0` is the ordinary-land early-out, so this blend is reached *only* by fragments whose sample and
geometry disagree.

**Reproduced before fixing.** Painting the inconsistent set -- sampled height <= 0 while the drawn
surface is above the datum -- magenta shows **3,593 pixels, 0.390% of frame, in
`highest_prominence_peak`**, and zero in `mountain_ground` (which has almost no fallback exposure at
eye level). The defect scales with fallback coverage, which is why Ian's 248-fallback view shows so
much of it.

**Fix.** Fade the blend out over the same 80m band the coverage ramp already uses:

    let ocean_coverage = outmap_ocean_coverage(outmap, macro_height_meters)
        * (1.0 - smoothstep(0.0, 80.0, input.surface_height_and_fog_color.x));

The distinction between a wet shoreline and a flooded summit is magnitude. A beach stands metres
above the datum and keeps its blend; a mountain stands kilometres above it and cannot be flooded by
a stale sample. Measured: `highest_prominence_peak` changes **3,246 pixels (0.352%)**, matching the
flagged set, and those pixels go from blue-dominant (68.7, 83.5, 132.4) to land (86.1, 86.6, 122.9).
`coast_waters_edge` is **bit-identical, 0 pixels changed on both captures**, so the shoreline the
band exists to protect is untouched.

**Unrelated pre-existing failure, not caused by this.** `highest_prominence_peak` fails
`camera_stands_on_the_ground` at 379.883m against a 150-155m bound, identically before and after
this change. That is thread 2's territory -- a pose derived against a summit height the app and the
survey disagree about -- and it is failing today.

**Validation.** 489 workspace tests (14 ignored), fmt clean, workspace clippy clean.
`coast_waters_edge` and `mountain_ground` pass. No timing claim; swap has been full all session.

## 8 September 2026 — underside sky refraction, with a guard that rejects the old result

Based on 0806dc8; above-water sharpness-based turquoise and terrain ownership are
untouched. The existing underside lookup used the unchanged camera ray and let
the wave normal affect only a broad smoothstep window mask. This made the sky
inside the window insensitive to the wave's optical distortion.

ocean_water_to_air now computes the refracted ray at n=1.333, exact
unpolarised dielectric Fresnel transmission, and total internal reflection.
The underside samples the existing physical sky LUT with that refracted ray.
Its reflected-water approximation, 30m water fog, geometry and motion are
unchanged. No additional texture, mesh, wave component or render pass.

**What the pictures establish, and what they do not.** The Snell-window edge is
sharper and wave-distorted; warm horizon light now bends into its lower edge.
At UV (0.05,0.95) in capture 004, the old baseline
ocean_underwater_visibility/1788859818-507484 is (55,80,125), red/blue 0.440.
Refraction-only 1788862853-9256 is (116,122,131), ratio 0.885. Broad unfoamed
areas still look smooth: this does not create missing sky detail, caustics,
cloud/object refraction or actual reflected underwater geometry. Human motion
review remains necessary; do not call the entire underwater presentation solved.

I also tried reusing above-water foam coverage on the underside. All four
captures in 1788862995-9864 were byte-identical to refraction-only. Removed that
trial and its extra lighting lookups rather than shipping unverified detail.

**Regression.** The underwater scenario now uses water_sample_uv plus opt-in
minimum red/blue (0.7) and maximum luminance (0.7) on the final sample. The old
unrefracted pixel fails; black and white fail; the refracted pixel passes.
The actual-WGSL GPU test covers six normal/oblique/near-critical/tilted/backward
ray cases against f64 Snell/Fresnel reference calculations, including zero
transmission under total internal reflection and a changed lookup direction
for a tilted normal. Ordinary source tests ensure the fragment uses this ray.

Final release GPU runs, all passed with four captures:
* ocean_underwater_visibility/1788863312-12083 — including the new visual guard.
* ocean_waterline_medium/1788863323-12117 — sky-leak guard retained.
* ocean_shallow_bottom/1788863335-12186 — sediment guard retained.
* ocean_hybrid_close/1788863345-12206 — above-water control.

491 workspace tests passed, 15 ignored; workspace all-target clippy, fmt and
diff checks pass. Both explicit Quadro GPU tests pass: six optical cases and
24 wave-parity cases (maximum normal error 0.000089958, height 0.000058081m).
Release rebuilt in target/release. No performance claim. Unrelated
crates.tar.gz is untouched. Ray seabed parity (thread 26) remains open.

## 8 September 2026 — you can swim down now, and three things were holding the eye at the surface

Asked for: pitch the look vector below the horizontal and descend. The obvious blocker was real but
was one of four, and the interesting part is that the other three were all load-bearing.

`surface_movement_direction` (`main.rs:432`) projected the radial component out of the look vector
before movement was computed, so the stroke was always tangential. That is the one you would find by
reading. It now flattens the forward axis only for a walker; a swimmer keeps the raw look vector.
Strafing stays level in both media — the right axis is built from the flattened forward either way,
because A/D should sidestep rather than roll a dive. The sphere advance already split a direction
into radial and tangential parts, but its radius clamp is the sea level datum, so a dive driven
through it would have stopped dead at the surface. The stroke is therefore split by the caller: the
tangential part travels, the radial part is handed to `advance_vertical`, which owns altitude and
its floors. Total speed along the look vector stays `movement_speed`, so pitching down trades travel
for depth rather than adding to it.

Fixing only that would still not have descended a centimetre. **The buoyancy restoring spring** is
keyed on the error against still-water equilibrium, and at five metres down that error saturates its
own 24 m/s^2 clamp — about 8 m/s of upward push against a 2 m/s stroke. **The crest floor** ends
every substep at least 0.06m above the water by construction. Both are *surface* devices: the spring
exists to keep a bobbing eye with the sea it floats on, and the wave-following drag reference is the
surface's orbital velocity, which is not the water's velocity at depth. So both now fade out over
the first body height of depth via `surface_authority`, and the crest floor is switched off for a
diver rather than faded.

The subtle one is what sets "submerged". Latching it on merely being below the waterline would mean
a crest overtaking a floating swimmer sets it, the crest guard switches itself off, and the sea
closes over an eye that never asked to go under — which is precisely the regression the guard was
built for. Only a commanded dive sets it; surfacing clears it at the guard's own clearance.

The fourth thing was not blocking, it was waiting: over water **both ground clamps are disabled**
(`water_surface.is_none()` guards them), so the moment a dive worked at all it would have swum
straight through the bathymetry to the -100m core clearance. A dive now stops 0.5m off the bed,
which is enough to inspect the bottom and enough to absorb the disagreement between the bathymetry
the CPU samples and the bed the renderer draws. Where the bed is deeper than the core clearance the
core clearance still wins, so that stays the single hard floor.

Measured, over still water: the settled descent is 1.8 m/s, the 2.0 m/s stroke less the 0.2 m/s the
diver is still floating up at. The first two seconds only cover about 2.6m, because near the surface
the spring is still at full authority — that is the model behaving correctly, not a governor. The
release drift is asserted to reach 0.2 m/s within 0.01.

**Nothing about swimming on the surface changes, and that is checked rather than asserted.** While
the crest floor is holding the eye above the water, `surface_authority` is pinned at exactly 1.0, so
the blended drag evaluates to `3.0 * 1.0 + x * 0.0` and the spring to `... * 1.0` — bit-identical
arithmetic, not merely similar. The pre-existing storm-crest regression test passes untouched, and a
new test holds the guard on for level strokes down to -1e-15 m/s, so a look vector a few float ulps
off horizontal cannot sink anyone. `DELIBERATE_DESCENT_SPEED_METERS_PER_SECOND` (0.05, about 1.4
degrees of pitch at swim speed) is that dead zone.

Ten new tests; 501 workspace tests and clippy pass. The HUD's swimming line now reads `submerged`
while under. **Interactive sign-off is still outstanding** — every number above is from the physics
model under test, and no one has yet pressed `G`, looked down and held W. Note `G`, not F4: F4
toggles orbit and low flight, `G` toggles the surface camera out of low flight.

### Verifying the underside refraction of the section above

`4f41d68` was checked rather than taken, and it holds. Its claim was that the window now distorts
with the waves instead of painting an unchanged sky, and the criterion that shows it is structure
*inside* the window with the rim excluded: mask the top 40% of luminance, erode it 3px so the rim
cannot contribute, and take mean |Laplacian| of luminance there. Across the four
`ocean_underwater_visibility` frames that rises **3.05x, 4.03x, 5.18x and 6.70x**. The pre-fix values
are 0.10 to 0.29 — flat, which is exactly what "sky sampled along `view_ray`, the normal used only
for a window mask" has to produce. That rejects the archived captures and passes the new ones.

The optics were re-derived independently and are right, including that the refraction is *not* in
the GLSL `refract` sign convention — `view_ray` runs toward the surface and the normal is on the
same side, so the form is `eta*I + (cos_air - eta*cos_water)*N`, which is what the commit has.

Three things about that commit, recorded because they are process rather than code. Its four replays
were run from a dirty tree and their manifests are therefore stamped `0806dc8`, the commit *before*
the fix, so the evidence for the change is filed under the state it replaced;
`ocean_rough_horizon/1788879028-29790` at the real commit passes and is the one to cite. It ran
`ocean_hybrid_close` this time but `ocean_rough_horizon` was missing for the third time. And the
`///` block that documents `ocean_underside_colour` now sits above `ocean_water_to_air`, which was
inserted between the two.

One consequence of that commit nobody has signed off by eye: the window rim is now sharper as well
as wavier. Fresnel transmission collapses from 0.44 to 0 inside the last 0.004 of cosine, where the
old `smoothstep(0.58, 0.74)` faded over a band forty times wider. Physically that is what a real
Snell's window does, but it is a second change riding along with the first.

## 8 September 2026 — hold depth instead of floating, and the bed at any depth

Two reversals of the section above, both asked for after trying to reason about it: a diver should
be able to reach the sea bed however deep it is, and should hold position rather than drift back up.

**Neutral buoyancy at depth.** The drift was Archimedes still acting on a body that is 0.85 the
density of water. Rather than special-case it, the same `surface_authority` fade that already
retired the restoring spring and the wave-following drag now also nets out gravity and buoyancy
together — one multiply, `acceleration *= surface_authority`, applied after they are summed and
before drag. At the waterline authority is 1 and nothing changes; a body height under it is 0 and
the diver is weightless. `SUBMERGED_ASCENT_SPEED_METERS_PER_SECOND` and the drag derived from it are
gone, replaced by `SUBMERGED_VERTICAL_DRAG_PER_SECOND` (8.0, a 0.125s time constant) whose only job
is to kill residual motion so releasing the stroke means stopping rather than coasting. The settled
descent is now the full 2.0 m/s stroke with nothing subtracted from it, and the first two seconds
cover 3.155m of a possible 4.0m — the missing 0.845m is the floating model still at full authority
through the first body height, which is correct and is asserted.

Surfacing is now *only* by swimming up, so a test covers that specifically: ten seconds of upward
stroke from five seconds of descent has to break the surface and hand the swimmer back to the
floating model, rather than stalling a body height under it where authority is still zero.

**The bed at any depth.** `PLANET_CORE_CLEARANCE_METERS` capped every dive at -100m. Its actual job
is to stop a runaway that has no bed to land on, so it no longer applies where there is water: over
water the bathymetry is the floor, four kilometres down if that is where the bed is. The
compile-time assertion in `ocean.rs` tying it to `MAXIMUM_WAVE_HEIGHT_METERS` is untouched and still
meaningful, because it is about troughs on dry-land-adjacent water. The old
`swimming_cannot_fall_below_the_underwater_safety_floor` test asserted the -100m cap; it now asserts
the bed catches the same runaway, which is a tighter bound than the one it replaces rather than a
looser one.

**A floor that was being overridden.** Chasing the depth cap turned up a second one.
`resolve_surface_camera_after_streaming` re-resolves the eye after each tile lands, and its
`minimum_eye_altitude` was `terrain_height + HUMAN_EYE_HEIGHT_METERS` *unconditionally* — including
over water. So the 0.5m bed clearance `advance_vertical` had just applied was quietly raised to
1.70m one call later, and the unit test that asserted 0.5m passed the whole time because it tests
`advance_vertical` in isolation. Both now go through `swimming_bed_eye_altitude_meters`. Worth
noting the shape: a unit test on a pure function proves nothing about a value a second writer
overwrites downstream, and this floor had exactly two writers.

**Not verified.** Nobody has dived deep interactively. Two things are known to be waiting down
there and neither is a defect in this change: `OCEAN_UNDERWATER_VISIBILITY_METERS` is 30.0, so below
about thirty metres the fog is everything and a four-kilometre dive is a long descent into black;
and the ocean shell is drawn at sea level, so from far beneath it the underside is well past the fog
anyway. Whether the LOD selector and tile streaming behave sensibly at a camera altitude of -4000m
is untested — before this, nothing could get below -100m, so that range has never been exercised.

501 workspace tests and clippy pass.

## 8 September 2026 — the neutral band was a body height deep, which is where you want to hold station

Reported from the app: dive, press F10, and the camera climbs back to the surface.

F10 is not really what does it. The fade to neutral buoyancy was one body height, 1.70m, so anything
shallower than that still had some of the floating model acting on it. Measured with the physics
driven directly, water surface held at 0.0m and no stroke: from -1.0m the eye reached +0.255m — the
equilibrium float height — inside ten seconds. From -8.0m it held at -8.0000m exactly. So a shallow
diver floated back out whether or not anything was frozen.

What freezing adds is that it makes it obvious and slightly worse. The scene clock stops, so the
wave field stops with it, and `water_vertical_velocity` sticks at whatever it was: the drag term
then references a surface that is rising for ever rather than one that will come back down, and the
restoring spring pulls toward a fixed equilibrium instead of a moving one. Same run from -1.0m with
the velocity frozen at 2.5 m/s reached +0.687m rather than +0.255m.

`NEUTRAL_BUOYANCY_DEPTH_METERS` is now 0.3m. A body height was simply the wrong scale for this: the
thing worth holding station in front of is Snell's window, which is directly overhead in the first
metre or two, and that was the exact band the old fade floated you out of. From -0.5m, -1.0m and
-8.0m the eye now holds to within 1e-6 m over thirty seconds against frozen wave velocities of 0,
+2.5 and -2.5 m/s, which is the new regression test. At -0.2m it still floats out, and it should:
that is a swimmer at the waterline, not a diver.

Shortening it changes nothing about floating, for the same reason the fade never did: the crest
guard holds a non-diving eye at or above `water_height + 0.06`, where the authority is saturated at
one however steep the ramp is. The storm-crest test still passes untouched. The dive is now barely
slowed at all — the first two seconds cover 3.911m of a possible 4.0m, against 3.155m before.

### Still open, and not caused by this

**The ocean freezes under F10, and the code says it must not.** `ocean_animation_time_seconds` is
documented as keeping the water moving because "F10 holds the planet/sun composition for inspection,
but must not turn the water at the default coastal start into a static blue sheet", and it is fed
`presentation_time`, which is `self.scaled_clock_seconds` — which `advance_scaled_clock` stops
whenever `animation_frozen` is set. The test named
`ocean_animation_keeps_advancing_while_scene_time_is_frozen` asserts only that the function returns
its second argument; it never checks that the argument keeps moving, so it cannot catch this. The
same applies to weather, which carries the same claim at its own call site and is fed the same
clock. Whether F10 *should* freeze the sea is a judgement call — for inspecting the underside it is
arguably what you want — so this is recorded rather than changed.


## 9 September 2026 — moving-wave swimming and the hidden sea-level clamp

The user reported the opposite of the previous handoff's F10 interpretation:
**moving** water pulled a diver up; freezing the waves stopped it. Reproduced
rather than retuning the neutral-depth ramp again.

* The real movement caller split the stroke, but still sent its tangent part
  through `advance_flight_position_on_sphere`, which clamps radius to sea level.
  A horizontal step at -4000m lifted the eye by 4000m. The combined tangent-step
  plus vertical-physics regression stalled at -0.005635m instead of reaching a
  -150m bed. Surface locomotion now uses a radius-preserving geodesic step;
  flight retains its existing policy. The same diagonal regression now reaches
  -149.5m, the existing 0.5m bed clearance. This explains an apparent depth limit
  relative to a raised wave without any literal 11m depth cap.
* A moving trough re-enabled buoyancy and cleared `submerged`, so the next
  crest's floor captured the diver. A stopped -8m diver rose to -6.994071m in
  the new moving-wave regression. Deliberate diving now latches the neutral
  swimming mode through transient trough exposure; only an upward stroke
  through the surface (or leaving ocean) restores floating. The same test holds
  its original altitude, and existing floating/crest, frozen-wave, upward-stroke
  and 4km-bed tests still pass. F10's clock policy is unchanged.
* Going deeper exposed two renderer guards: LOD asserted at -100m, then, after
  removing that artificial limit, culled the entire bed against the surrounding
  sea-level sphere. LOD now requires a nonzero camera radius; underwater horizon
  culling uses the conservative solid inner height bound. The 4km LOD regression
  retains visible bed nodes and still rejects the far hemisphere. Above-water
  horizon policy and collision's seabed floor are unchanged.

Validation: 506 workspace tests pass (16 ignored), all-target clippy and fmt
pass. All four added regressions failed before their respective repairs. GPU
`ocean_underwater_visibility/1788978590-25052` passes with four captures and the
existing Snell-window colour guard. Release rebuilt in `target/release`.
Interactive swimming and a full deep-ocean GPU descent remain human/runtime
acceptance checks; the automated diagonal test exercises movement plus physics,
not the complete streaming/window event loop.

### Preserved unfinished transparency work — do not call it finished

Uncommitted raster seabed-transmission work remains in hdr.rs, main.rs's render
passes, ocean_transmission.rs, planet.wgsl, shared_planet.wgsl, terrain.rs,
ocean_gpu_tests.rs, scenario.rs and new shallow-water scenarios. It uses a
pre-water colour/depth snapshot and wave-normal air-to-water refraction with
30m attenuation. The first version was measured at about +6–7ms in full-screen
shallows and +2ms in open sea on the Quadro (1280x720 Immediate); a subsequent
single-evaluation wave/height optimisation is not performance-validated yet.
This change is separate from the swimming commit. `crates.tar.gz` is untouched.

The user's new manual capture
`manual/1788978330-21709/screenshots/capture-001.png` visibly contains rectangular
blue/brown patches beneath foam at the shore. The requested reproduction is a
straight-down shore view while ascending. `ocean_shore_ascent` is being added for
that investigation; no shoreline repair or temporal sign-off is claimed yet.

## 10 September 2026 — tropical shoreline transmission fallback

The shore replay showed rectangular blue/brown patches because screen-space refraction returned zero transmission whenever the refracted ray left the viewport or crossed a one-pixel bank discontinuity. The ocean then contributed only its opaque blue body. `ocean_screen_fallback` now samples the current pre-water colour/depth pixel when that happens, accepts only geometry behind the water surface, and applies the same 30m exponential water transmittance. This keeps the sandy bed continuous instead of exposing a hard blue rectangle.

The beach/sediment palette is now shared as pale tropical cream-yellow (`0.94, 0.89, 0.70` sRGB). Shallow water naturally blends that bed with the blue body through the existing depth transmittance; deeper water returns to blue as the bed contribution attenuates. `ocean_shore_ascent/1789025349-41472` rebuilt with the fallback and shows the intended pale shallow bed; foam strips and screen-edge coverage still need human sign-off.

Validation: focused ocean tests (41 passed), shader validation, three actual-WGSL GPU optics/normal tests, and release build pass.

## 10 September 2026 — shallow-water turquoise balance

The first transmission blend left shallow replay frames sand-dominant. The ocean fragment now retains a 78% shallow turquoise scattering contribution and limits the visible bed correction to 22% of attenuated transmission; the depth exponential still fades that tint and bed toward the deep-water body. Replay `ocean_clear_shallows/1789026290-45042` now shows pale turquoise water over the cream sand from above.

## 10 September 2026 — dry beach band and nearshore shoaling fade

The beach shader now uses a 50m land-owned sand band, with pale dry sand inland and a darker wet-sand blend over the first 7m above sea level. This keeps a visible dry strip in front of the water instead of letting the shell meet every positive-land pixel.

The circular-looking nearshore wave fronts come from `shoaling_phase_offset_meters`: its scalar depth-only quadratic phase creates depth contours that look like rings. The CPU and WGSL paths now fade that phase through the final 30m of water, reducing the effect without changing open-ocean wave axes or geometry. Replay `ocean_shore_ascent/1789041409-56996` shows the broad dry strip and less concentrated rings; some curved fronts remain because shelf refraction is still intentionally depth-driven.

Validation: ocean unit suite, release build, and three actual-WGSL Quadro ocean tests pass.

## 10 September 2026 — remove circular shoreline wave source

The latest manual frame showed full 360-degree rings centred on the beach. The cause was confirmed: `shoaling_phase_offset_meters` added a scalar depth-only phase to every Gerstner component, so equal-depth contours became circular crest sources. The CPU and WGSL phase helpers now return zero. Depth remains available to the existing amplitude, steepness, breaking, and foam paths, so directional waves remain directional without the artificial radial source. The old steering tests now pin zero shoreline phase instead of requiring the removed radial refraction.

Replay `ocean_shore_ascent/1789041894-58327` shows no former 360-degree ring source. Remaining white foam and coarse nearshore geometry are separate visual issues. Ocean unit tests, release build, and three actual-WGSL Quadro GPU tests pass.

## 10 September 2026 — shoreline edge follow-up

The latest manual capture confirms the remaining hard line is the separate sea-shell silhouette at the macro-height ownership boundary. I widened the land-owned wet-sand transition to the same 220m coastline scale used by the material blend, with a 20m wet band, so colour converges before the boundary. This reduces contrast but does not yet constitute the true geometry/compositing fix: a blended shoreline pass or conforming near-shore shell is still required to remove the silhouette itself.

## 10 September 2026 — dedicated shoreline blend pass

The hard ownership edge now has a dedicated `fs_ocean_shoreline` pass. It renders only positive coastal terrain (0-220m macro height), uses the same ocean lighting, fades alpha to zero across that band, enables alpha blending, disables depth writes, and uses an Always depth comparison so it can soften the sea-shell silhouette over the already-rendered dry terrain without replacing terrain depth. The opaque shell and dry terrain paths remain unchanged.

Shader validation, release build, and `ocean_shore_ascent/1789048908-69947` pass. The deterministic shore pose is not the exact manual edge pose, so final visual sign-off still needs the user's manual capture.

## 10 September — water-side shoreline composition repair

The prior shoreline overlay selected positive terrain then unconditionally discarded it in the shared lighting helper. Its escalating 100km cutoff and Always-depth policy were not a valid repair. The draw is now disabled (pipeline retained, not submitted). The existing transmitting ocean instead composites the real pre-water colour into the wet edge, with coverage approaching zero across the last 0.5m of sampled/interpolated depth and actual surface-to-bed distance. Sky/foreground snapshot samples are rejected; ocean depth ownership remains intact. No extra draw or land overlay is submitted.

Release rebuilt; shader validation and new focused composition guard pass. GPU replay ocean_shore_ascent/1789053752-76393 completed: capture-006 visibly grades turquoise to sand rather than cutting directly between the two. A lighter sediment/dry-sand boundary and screen-edge blue remain visible; this is not complete shoreline visual sign-off or an underwater leak diagnosis.


## 10 September — shoreward propagation investigation, not a repair

The current wave API receives direction, time and scalar depth, not a coast normal.
Its zero depth-phase hook cannot steer waves. Restoring the previous scalar offset
would restore the reported concentric sources; reversing global time would only
swap which coasts receive incoming waves. No runtime propagation change was made.
Corrected the current-state shore description and misleading steering test names;
a spatial/temporal raw-height regression now protects against depth-contour phase
sources. A coherent coastal propagation or directional-energy field shared by CPU
buoyancy and GPU geometry/normals remains implementation work. Do not treat passing
wave parity or the no-rings regression as shoreward-motion acceptance.

Validation for this investigation: 17 `ocean::tests` pass; workspace formatting
check passes. No GPU replay or performance claim: runtime code is unchanged.

## 10 September — opt-in spawn-coast travelling-wave prototype

Runtime flag `CATINGARDEN_SPAWN_COAST_WAVES=1` enables a **local experiment only**.
Default remains disabled. This is not automatic steering on arbitrary coastlines.
At `wavedir_spawn`, the surveyed L4 raw bed is -18.19m; a 4km central difference
points toward increasing height along `(0.48197104,-0.86880293,0.11351380)`.
Along that tangent the bed is -52.48m at -5km and +13.50m at +5km. The authored
patch is centred at `(0.84285087,0.49512231,0.21084662)`, fully active through
8km and smoothly gone by 16km (angular chord on the 4000km planet).

Only components travelling offshore relative to that surveyed tangent reverse.
The fade mixes **complete opposite-travelling fields**, not their phase or time
multiplier. Thus no depth contours enter phase and derivatives cannot grow with
elapsed time. CPU height, slope, velocity and GPU geometry/ripples share the
selection. The slope includes the spatial envelope derivative. No draw, mesh,
texture, bind group, or terrain/tree changes were introduced.

The component-energy regression failed before (dominant swell offshore flux
-0.007858) and passes with the prototype. Spatial/temporal finite differences
cover the fade and late time; the actual-WGSL GPU regression now has 48 cases
including the interior, fade and exterior. Enabled maximum normal error is
0.0000205841 and height error 0.000165351m; all three GPU ocean tests pass.
Two existing terrain source-string tests fail in the full app suite: their
expected strings are already absent from HEAD's unchanged planet.wgsl. Do not
change concurrent terrain work to hide those failures.

First paired 1280x720 Immediate `wavedir_spawn` captures:
- disabled: `1789058891-83824`
- enabled: `1789058948-84358`
Windowed greyscale cross-correlation between consecutive captures changed from
roughly (+3,-6) to (-6,+5) pixels; onshore projects approximately screen-down.
Correlations are 0.985–0.988. This is measured motion, not a still-image inference.
Timing rechecks and final validation are in progress; do not yet claim a perf win.

Remaining: authoring a shared global coastal direction/energy field, testing
multiple coast orientations, and visual review of possible standing interference
in the 8–16km fade. The patch does not cover the older circular-source manual
capture 60km away. Leave the opt-in disabled by default pending that work.

Repeat captures: disabled `1789063531-86243`, enabled `1789063630-86453`.
`scripts/measure-spawn-coast-motion.py DISABLED_RUN ENABLED_RUN` reproduces the
motion check (numpy/Pillow). It asserts high correlation, offshore baseline,
and shoreward enabled motion for all seven adjacent capture pairs. Repeat
onshore cosine is -0.891 to -0.635 before and +0.635 after; minimum correlation
is 0.9849. JSON evidence lives in the enabled run's `coast-motion-comparison.json`.

**Cost remains unacceptable for promotion:** matched 1280x720 Immediate samples
at sim_time >=1s have median 38.616ms disabled versus 43.640ms enabled (+5.024ms,
+13.0%). Only six logged samples per run: preliminary renderer timing, not a
comprehensive benchmark or an FPS improvement. Defaults remain disabled.
The disabled GPU control also passes all three tests (48 normal/height cases,
maximum normal error 0.000001093, height error 0.000023876m).

Reproduce the local prototype after a release build:
```
CATINGARDEN_SPAWN_COAST_WAVES=1 target/release/catinthegarden-app --scenario wavedir_spawn
```
Omit `--scenario wavedir_spawn` for interactive use; only the surveyed spawn
coastal patch is changed. The environment variable must be set before launch.
Globalisation needs a coherent shared directional-energy field rather than
more hardcoded coast patches. Do not reintroduce scalar bathymetry phase.

Final validation: 507 workspace tests pass with the two independently confirmed
pre-existing terrain source-string failures explicitly skipped (17 other tests
ignored). Enabled ocean suite: 19 pass, one opt-in test ignored; that component
transport test passes when explicitly run with `--ignored`. Clippy all targets,
formatting, and release build pass. Default rendering remains opt-out of the
prototype. `crates.tar.gz` and terrain/tree implementation files are untouched.


## 10 September — enable spawn-coast waves in normal gameplay

At the user's request, an unset `CATINGARDEN_SPAWN_COAST_WAVES` now enables the
validated local prototype. Explicit `1` enables and `0` disables; other explicit
values retain their previous disabled behaviour. CPU and generated WGSL still
share the same setting. No wave maths, patch extent, terrain, or tree changes.
The previous measured cost and global-coverage limitations remain applicable.

Validation: 20 ocean unit tests pass (one separately enabled-only test ignored);
the explicit component-transport regression also passes. With the environment
variable unset, the actual-WGSL 48-case GPU normal/height test passes with the
same enabled-prototype errors as before. Formatting/diff checks and the release
build pass. The normal runnable is `target/release/catinthegarden-app`; no launch
parameter is needed. The two historical terrain source-string failures were not
changed or reclassified by this small default-setting change.

## 10 September — underside scene reflection and 100m water visibility

User captures `manual/1789064534-89566/screenshots/capture-001.png` and `002`
showed lit shallow sand but a nearly uniform navy ceiling below the waves.
Outside Snell's window, `ocean_underside_colour` used a constant dark sky-tinted
fill, not the underwater scene.

The transmitting raster ocean's back-face entry points now trace a reflected
view ray through the existing pre-water colour/depth snapshot. Search is bounded
to 24 quadratically spaced samples plus six bisections, within the remaining
100m path budget. Sky/missing-depth/off-screen samples and mismatched ray hits
are rejected. Reflection can legitimately return toward the camera, so it does
not reuse transmission's behind-water-depth restriction. Legacy non-snapshot
entry points retain their existing fallback and need no new resource bindings.
No new draw, geometry, or render target was added.

The initial replay `ocean_underside_shallows/1789066909-92734` exposed severe
horizontal SSR bands. Nearest-pixel reconstructed depth was discontinuous at
grazing angles; bilinear reversed-Z sampling with a depth-discontinuity guard
removes the strong bands (`1789067070-93280`). Off-screen hits still produced a
visible reflection rectangle, so a local horizontal-bed approximation now uses
the existing sediment palette, sky/sun illumination and water attenuation as
fallback (`1789067186-93609`). Detailed reflections are screen-space only;
reflected off-screen terrain shape and objects absent from the snapshot (such
as the separately drawn boat) are not represented by that approximation.
Some grazing-angle aliasing remains; this is not full-scene ray tracing.

Snapshot colour has already received direct camera-to-bed water fog. The shader
undoes only recoverable attenuation, fades unreliable recovery, applies the
reflected bed-to-surface leg, then adds surface-to-eye fog once. Visibility is
now **100m at 2% remaining contrast**, shared by underside, underwater terrain,
and water transmission. The above-water 80m seabed draw budget remains unchanged.
The GPU transmission test samples 0,20,40,60,80,100m and failed with the old 30m
constant before passing with the new value. All three GPU optics/wave tests pass.

Added `ocean_underside_shallows`: a fixed shallow submerged shore-facing camera,
four wave-time captures and an underside colour guard against the old navy fill.
Source guards pin pass isolation, bounded ray search, discontinuity rejection,
and the reflected fog leg. Frame cost has not been measured; no FPS claim.
Final workspace and GPU replay validation is being recorded below.


Final validation: 509 workspace tests pass, with the same two pre-existing
terrain source-string failures explicitly skipped and 17 other tests ignored.
The new scenario required updating the scenario registry count from 85 to 86;
its loading regression now passes. Clippy all targets, formatting/diff checks,
and release rebuild pass. Actual-WGSL optics/visibility and wave parity pass.

Validated GPU replays (all assertions pass):
- `ocean_underside_shallows/1789067360-95007`: underside colour sample
  (223,215,189), with reflected sand across the moving surface; screenshot 004
  is the final reviewed example. The 0.9 red/blue guard excludes the navy fill.
- `ocean_underwater_visibility/1789067373-95101`: existing submerged control.
- `ocean_shallow_transmission/1789067384-95136`: above-water transmission control.

The app must be restarted to load the rebuilt shaders. Changes are enabled in
normal raster gameplay. No performance improvement is claimed; SSR cost and
fresh human visual acceptance remain unmeasured. `crates.tar.gz` is untouched;
terrain.rs changes are only the 100m test expectation, not terrain rendering.


## 11 September — dry-land to wave-washed beach colour continuity

User clarified the left side of `manual/1789119500-120010/screenshots/capture-001.png`
is entirely dry; the right is the wave-washed beach. Reproduced the debug pose
(camera `[3372422,1978479,844259]`, direction `[-0.485,0.101,0.868]`, 60° actual
vertical FOV) in `beach_sand_join`, 1280x720, fixed exposure, three captures.

The paths used different albedos at the same sea-level boundary. Submerged
sediment used full cream sand, while positive land immediately applied the
38% wet-sand tint. Actual-WGSL regression reproduced a linear red jump from
0.8688994 to 0.6688266 between sea level and +1cm. Shared `beach_sand_albedo`
now multiplies the existing wet tint by smoothstep(0,4,height): both sides meet
at the same cream colour, with continuous slope, while heights >=4m retain the
old tint exactly. These are **vertical height metres**, not horizontal beach
width. The wet treatment still fades out by the existing 20m elevation.
Submerged sand, the beach/ocean ownership masks, geometry, waves, lighting,
fog and trees are unchanged. No extra draw or texture lookup was introduced.

Matched GPU replays, all assertions pass:
- Before: `beach_sand_join/1789119818-120571`.
- After: `beach_sand_join/1789120066-122324`.
- Reviewed screenshot: after `screenshots/capture-003.png`.

`scripts/check-beach-sand-join.py RUN_DIRECTORY` checks the vertical seam in all
three captures; it fails before and passes after. In rows 300–699, columns
652–667, the p95 strongest adjacent RGB step drops 12.0→1.3333, below the 3.0
regression limit. Each capture gives the same metric. Capture 003's right-hand
water region (x>=800,y>=300) and top 200 sky rows are byte-identical before/after.
The new GPU material test fails before and passes after, checking the signed
height boundary and retention of darker wet sand away from it. Reused the GPU
optics test harness with an explicit enum; no runtime testing dependency added.

The separate barely-submerged distant-background leak is still outstanding.
The CPU-wide water-medium flag versus per-pixel water backfaces is a hypothesis,
not a diagnosed backface/culling failure. This material correction does not
change that behaviour. `crates.tar.gz` and concurrent terrain/tree code are
untouched. Final validation results follow below.

Final validation: 509 workspace tests pass with the two previously documented
terrain source-string failures explicitly skipped (18 other tests ignored).
The new six-case GPU sand test and all three existing GPU ocean tests pass;
clippy all targets, formatting/diff checks and release build pass. Normal game
launch uses the correction; restart to load the rebuilt shader. No new FPS
measurement or global shoreline/underwater-leak sign-off is claimed.

## 11 September — underside Snell/reflection diagnosis

Discussion after the shallow underside reflection change raised a valid question:
why the near-horizontal surface showed only reflected sand and apparently no sky.
Three launch-only diagnostic modes now isolate the optical terms without entering
the normal F9 cycle:

```
CATINGARDEN_DEBUG_MODE=underside_transmission
CATINGARDEN_DEBUG_MODE=underside_sky
CATINGARDEN_DEBUG_MODE=underside_reflection
```

At the existing near-horizontal `ocean_underside_shallows` pose, the transmission
mask in `1789122078-127838` is black across the visible underside: every ray is
outside the 48.6-degree Snell window and undergoes total internal reflection.
The reflection-hit mask `1789122223-128266` is white over most of the surface,
showing that the sand is a successful screen-space reflection rather than merely
the local fallback. The first sky diagnostic capture was misleading on TIR pixels
because it returned the ordinary reflection before reaching its debug branch;
that mode now returns black for invalid refraction and sky only for valid rays.

Added `ocean_underside_snell_window`, the same shallow site with a steep upward
view. Captures prove the three terms:
- transmission `1789122389-128773`: the expected bright circular Snell window;
- refracted sky `1789122402-128856`: blue sky within exactly that window;
- final `1789122413-128914`: refracted blue sky inside, reflected cream sand
  outside, with a Fresnel transition at the edge.

Conclusion: clear-water refraction/reflection composition is behaving as intended.
Do not force sky into distant/grazing underside pixels; that would violate total
internal reflection. The user's requested follow-up is separate: breaking-wave
foam contains air bubbles and should attenuate the refracted sky, while scattering
sun/sky light as pale blue-white from below. Reuse the existing foam/breaking
signal rather than inventing a second moving mask. No foam change was made here.

During this diagnosis an unrelated local `OCEAN_WAVE_SCALE = 4.0` experiment made
the existing underwater-floor compile-time guard fail. At the user's explicit
request it was restored to 1.0 before validation. `crates.tar.gz` remains untouched.

Final validation: 510 workspace tests pass with the same two pre-existing
terrain source-string failures explicitly skipped (18 other tests ignored).
Four focused underside source tests and all three actual-WGSL ocean tests pass;
clippy all targets, formatting and release build pass. No final-look shader
change was made. Diagnostic modes are environment-selected at launch and do
not enlarge the F9 cycle. Scenario registry is now 88. `crates.tar.gz` remains
untouched.

## 11 September — grazing underside skylight and depth-aware shallow water

The user accepted the Snell-window diagnosis but found the near-horizontal
underside too much like a perfect sand mirror. The final presentation now mixes
a bounded 25% overhead skylight term into the reflected-underwater contribution.
This is deliberately an artistic model of unresolved capillary roughness and
microbubble scattering, not direct transmission beyond the critical angle.
Resolved Fresnel/Snell transmission is unchanged and remains dominant inside
the window; successful seabed SSR remains 75% of the nominal TIR contribution.
The existing transmission/sky/reflection debug modes are unchanged.

The same shallow replay exposed a separate depth error: 100m visibility was
applied from ray length alone, turning a long view across a shallow bright shelf
into the same dark blue as deep water. Water extinction now retains 15% of its
normal strength through 2m depth, interpolates smoothly by actual local baked
water depth, and reaches the existing full-strength 100m visibility medium at
30m. This is a bounded approximation of the continual bed illumination in
shallow water; genuinely deep water retains the previous extinction. Raster
bathymetry, underside surface fog, SSR recovery/reflection legs and off-screen
sediment fallback share the depth rule. Geometry, waves, ownership and draws
are unchanged.

Matched final-frame evidence:
- near-horizontal shallow before `ocean_underside_shallows/1789122236-128337`,
  after `1789123670-134901`: dark-blue pixels fall 51,341 to 13,659 (the same
  fixed blue-dominance threshold), reducing the 100-row horizon ROI from 40.1%
  to 10.7%; the reflected sand and wave structure remain visible;
- steep-up control `ocean_underside_snell_window/1789123696-135029` retains the
  large blue Snell window and cream reflected region;
- deep control `ocean_underwater_visibility/1789123708-134900` remains 97.5%
  dark-blue by that threshold.

Both new actual-WGSL GPU checks pass on Quadro M1000M: the 25% bounded blend and
six depth samples from 0m to 100m. All five GPU ocean optics/wave tests pass.
No foam attenuation was added: aerated white water should later use the existing
breaking/foam signal to block direct sky and scatter pale light from below.
No FPS claim; this adds only scalar fragment arithmetic and no sampling, draw,
texture or geometry work. Final workspace/clippy validation follows below.

Final validation: 510 workspace tests pass with the two documented terrain
source-string failures skipped (20 tests ignored). Five actual-WGSL GPU ocean
tests pass on Quadro M1000M; clippy all targets, formatting, diff checks and the
release build pass. Normal gameplay uses the change after restart.

## 11 September — the SSR fog inverse, and two tests that were not watching

Three findings from verifying caada2e rather than accepting it. The claims in that
commit's handoff mostly hold: 510 passing is exact (`cargo test --workspace` alone
stops at the first failing binary and shows only 439 -- `--no-fail-fast` is needed
to see the real total), clippy and fmt are clean, and the dark-blue pixel evidence
reproduces. Sweeping blue-dominance thresholds for one criterion that fits all
three cited figures gives `b>r+20 && b>g+20`: 51,273 / 13,669 / 98.1% against the
reported 51,341 / 13,659 / 97.5%. One consistent criterion, three matching numbers.
The GPU count was understated -- six ocean GPU tests pass, not five.

**The recovery leg had stopped being an inverse.** `ocean_scene_reflection` is
screen-space: it marches the reflected ray against `water_scene_depth` and reads
colour from `water_scene_color`, a snapshot whose every pixel was already fogged
by `terrain_fog` along its *own* direct camera ray. To reuse such a pixel as a
reflection the shader must divide that fog back out. `terrain_fog` applies it with
the drawn point's own depth below the datum (`max(-surface_altitude_meters, 0.0)`),
but the recovery was dividing with `water_depth_meters` -- the water column above
the fragment doing the reflecting. A different pixel, a different depth.

Before caada2e both sides were depth-independent, so subtract-and-divide inverted
exactly. Making both depth-aware without making them agree turned it into a
systematic error, measured at e-fold 25.562m:

| wave-frag | bed  | ray  | applied | removed | residual |
|-----------|------|------|---------|---------|----------|
| 1m        | 10m  | 20m  | 0.221   | 0.111   | +0.110   |
| 1m        | 25m  | 30m  | 0.664   | 0.161   | +0.502   |
| 1m        | 40m  | 40m  | 0.791   | 0.209   | +0.582   |
| 20m       | 2m   | 20m  | 0.111   | 0.445   | -0.334   |
| 40m       | 1m   | 30m  | 0.161   | 0.691   | -0.529   |
| 5m        | 5m   | 20m  | 0.129   | 0.129   | +0.000   |

The last row is the proof that the depth disagreement is the whole error. Positive
residual leaves up to 0.58 of water tint baked into the recovered reflection -- the
dark blue caada2e existed to remove, reintroduced inside the SSR path. Negative
subtracts more fog than was applied, and `max(color - fog.color * amount, 0.0)`
clamps the reflected bed to black. `confidence` was computed from the same wrong
transmission, so it was most over-trusted where the colour was worst.

Fixed by recovering the resolved point's own depth from its view-space position.
`local_view_altitude_meters` expands about the camera -- camera altitude, rise
along the radial, curvature drop of the tangent plane -- keeping every term at
metre scale. Honest note on why, because the first version of this comment
overclaimed: the general `altitude_along_ray` form subtracts the planet radius
from a radius near 6.37e6, which in strict binary32 costs up to one ulp (measured
at 0.5m), but **this GPU evaluates it more precisely and both forms pass the new
test to under a centimetre**. The expansion is not repairing an observed fault; it
is a form that does not depend on the driver being generous. The reflected leg
still uses the reflecting fragment's own column, which is an approximation rather
than an inverse and is left as one.

`ocean_water_fog_amount` is split out of `ocean_water_fog_at_depth` because only
the amount depends on depth -- the colour does not -- which lets the depth rule be
tested without binding the sky LUT.

Two new GPU tests, both mutation-verified: `gpu_local_view_altitude_recovers_
depth_without_radius_cancellation` (a constant sentinel fails it) and
`gpu_water_fog_inverse_cancels_only_at_the_depth_it_was_applied_with`, which
asserts a matched inverse cancels to 1e-6 and a mismatched one leaves more than
0.1. A source test pins the wiring; reverting the fix fails
`ocean_underside_refracts_the_sky`.

**Two tests had stopped watching anything.** `raster_ocean_uses_a_separate_
analytic_shell` and `terrain_fragment_keeps_positive_interpolated_land_over_mixed_
ocean_samples` split the generated shader on `fn ocean_fragment_with_transmission(`,
which 5b6b911 turned into a four-line wrapper delegating to `..._mode`. They had
been asserting against an empty body since 10 September -- vacuous, not merely
red -- and what they were written to guard is the open-ocean discard guard whose
second consumer painted sea colour across a mountain on the 8th. Repointed at
`_mode`, discard text updated for the `!shoreline &&` exemption 5b6b911 added,
both mutation-verified. The standing "two documented failures" carve-out is
retired: the workspace is 512 passing, 0 failing, 22 ignored.

Still open, unchanged by this: `ocean_distance_fog` and `ocean_water_fog` have
zero callers between them, so the `camera.camera_forward.w` read in the fog path
is dead code held alive by a test that pins it. And `ocean_water_fog_at_depth`
passes an already-view-space vector through `planet_to_view` when picking the
sky direction for the fog colour -- every sibling call site in this file takes a
planet-space vector, and the Rust field is built by `world_to_view`. That dates to
40a1128, whose own message says "both unverified". Not touched here; it changes
the underwater fog colour and deserves its own measured before/after.

No FPS claim: this adds scalar fragment arithmetic and no sampling, draw, texture
or geometry work. No capture evidence for the fix itself yet -- the mismatch is
largest across a shelf slope, and the existing underside scenarios sit where the
two depths nearly agree, so the scenario that would show it does not exist yet.


## 11 September — aerated crests seen from below

The underside now uses the exact `ocean_foam_coverage` value already used by
the top face, including surf, whitecap, water-presence and 0.82 maximum-coverage
rules. Where that signal exists, it replaces the directional Snell-window or
bed-reflection result with bounded diffuse skylight: 55% neutralised toward pale
white and scaled to 80% radiance. Thus the bubble layer blocks some direct sky
and reflected bed without becoming opaque paint or inventing a second moving
foam mask. Clear water retains the existing Snell/Fresnel, 25% rough-surface
skylight, SSR and depth-aware fog paths exactly.

A clean `b0b2882` baseline was rebuilt in a separate worktree target and compared
with the current release in the deterministic `ocean_underside_shallows` replay:
- before `1789143770-167364`, after `1789143320-161408`;
- at capture 001, when the existing foam signal covers the approaching shallow
  crest field, 441,554 pixels change by more than two 8-bit levels and the
  directional cream ceiling becomes diffuse grey-white;
- as that existing foam moves away, the change falls to 3,218 pixels in capture
  002, 132 in 003 and 4 in 004. This pins the effect to the moving foam rather
  than globally recolouring the underside;
- `ocean_underside_snell_window/1789143349-161482` and
  `ocean_underwater_visibility/1789143361-161268` are byte-identical to their
  pre-change controls in final capture, retaining the clear Snell window and
  deep-water appearance.

The new actual-WGSL test checks zero-to-0.82 foam composition numerically, while
the source regression pins both underside entry points to the shared foam signal.
All nine actual-WGSL ocean tests pass on Quadro M1000M when serialized. A parallel
run crashed the legacy Vulkan driver with SIGSEGV while constructing several GPU
instances concurrently; the same tests pass one-by-one, so use `--test-threads=1`.
Workspace: 512 pass, 0 fail, 23 ignored. Clippy all targets, formatting, diff
checks and release build pass. No FPS claim; the underside adds scalar ALU only,
with no texture fetch, geometry or draw. The handoff's pre-existing dead fog
wrappers and fog-colour coordinate-space concern remain open and untouched.

## 12 September — seafloor-hole reproduction (investigation paused)

Manual capture `test-runs/manual/1789133598-154356/screenshots/capture-001.png`
shows a polygonal dark lower region adjacent to bright cream seabed. The capture
was on `7ab5955`, camera about 3.7m below datum, at 31.186s wave time. The HUD
position was rounded/stale relative to the spatial log; the deterministic
`ocean_seafloor_hole` scenario therefore uses the 31.150s logged world position
rotated into the local terrain frame, local view direction, local sun direction,
31.18s wave phase, fixed exposure and 60-degree FOV.

Replay `test-runs/ocean_seafloor_hole/1789203209-188990/screenshots/capture-001.png`
reproduces the irregular dark polygon at the same part of the field. The
`underside_transmission` diagnostic `1789203411-189735` makes the upper wave
ceiling black (TIR), but does not change the cream/dark lower boundary. The
`raw_albedo` diagnostic `1789203479-189834` leaves the cream/dark lower boundary
in place. This rules out the Snell lookup as the cause and makes a terrain
coverage/depth/ownership gap likely; it is not yet proved which condition
rejects the missing fragments. Do not call it fixed. The user has switched to a
specific-area FPS request; resume by instrumenting terrain ownership/depth at
this scenario pose, then establish an image guard that fails on the dark region.
`crates.tar.gz` was untouched.

## 12 September — measured underwater ocean FPS improvement

The bounded area is the raster ocean **underside**, not the top face or CPU
buoyancy. `vs_ocean` already evaluates the same 17 global and three ripple waves
to place each sea vertex. Its smooth normal, ripple slope, vertical displacement
and breaking ratio now interpolate into the two underside fragment paths instead
of recomputing the entire wave spectrum at each covered pixel. The legacy flat
face normal remains flat for the low-poly presentation; top-face per-pixel
lighting and glints still use their original wave evaluation. A source regression
guards against accidentally restoring per-fragment wave evaluation below water.

Performance control: release binaries built from `2efc5f6` and the optimized
source, Quadro M1000M, 1280x720, `ocean_underside_shallows`, Immediate present,
fixed exposure/time/camera. Ten interleaved pairs (baseline first for pairs 1-5,
optimized first for 6-10); each result is the median of seven logged `spatial
frame` samples with simulation time >=1s. All runs and captures are retained in
`test-runs/ocean_underside_shallows/1789205328-195137` through
`1789205670-196145`. Per-run medians, in pair order, were:

| ms | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Baseline | 37.130 | 33.792 | 35.969 | 37.229 | 36.268 | 35.859 | 36.352 | 36.958 | 35.744 | 37.332 |
| Optimized | 35.009 | 33.841 | 34.509 | 33.946 | 34.420 | 34.384 | 34.482 | 32.984 | 32.665 | 39.225 |

Medians: **36.31 -> 34.40 ms**, or **27.54 -> 29.07 FPS (+5.5%)**. Eight of
ten pairs improve. Mean paired saving is 1.72 ms; a 100,000-resample paired
bootstrap gives 0.66-2.64 ms as its 95% interval. This is a specific-view
improvement, not a whole-game FPS claim. Earlier 24->16 SSR steps, 6->3 SSR
refinement and replacing cubic `pow` with multiplication did not give a
convincing gain; all three trials were reverted.

Matched deterministic captures: original `1789203963-190371` versus optimized
`1789204949-194077` in `ocean_underside_shallows`. Mean per-channel RGB delta
over the four 1280x720 captures is 0.209, 0.059, 0.034 and 0.033 levels; pixels
with any channel changing by more than eight levels are 904, 1,173, 670 and
555 respectively. **Disclosure correction after Claude's review:** these means
do not imply an unchanged waterline. Independently recomputing the same four
image differences gives maximum channel changes of 163, 195, 155 and 147 levels,
and 481, 634, 364 and 360 pixels respectively exceeding 32 levels. Their bounding
boxes (right/bottom exclusive) are `(2,410,1149,425)`, `(30,403,1279,425)`,
`(84,415,1265,425)` and `(109,421,1276,425)`. These are coherent high-contrast
colour changes around the waterline, not random negligible noise. Interpolated
wave optics are an approximation; vertex geometry itself was not changed.
Human acceptance must include those localized edge shifts and motion.
The clearer Snell-window control (`1789206034-199020` vs
`1789206048-199070`) changes by at most three levels in any channel, and the
three above-water `ocean_ship_float` captures (`1789206060-199092` vs
`1789206085-199197`) are byte-identical. Human motion sign-off remains.

Workspace: 512 passed, 23 ignored; nine serialized actual-WGSL Quadro ocean
tests passed (normal parity maximum 0.000020584, height error 0.00016535m).
Clippy all targets, fmt and diff checks pass. The seafloor hole above remains
unfixed and is not hidden by this optimization. `crates.tar.gz` untouched.

## 12 September — terrain-occluded sky candidate; GPU measurement pending

**Status update: measured and retained in the completion section below.** The
pending state described here is historical, at commit `2429540`.

User requested another independently measured FPS improvement. Chosen area:
full-screen sky shading hidden behind opaque terrain. `main.rs` previously drew
sky first with an Always depth test, then replaced those pixels with ground.
The candidate moves the raster sky draw after opaque ground (still before the
water-scene snapshot), uses Equal against reverse-Z clear depth 0 without writing
depth, and marks the sky vertex position invariant. No sky integration, resolution,
material, geometry or sample count changes. Sky-only mode and the airless
background still draw; foveated-ray ordering is unchanged. Two-body replay still
draws its sky against freshly cleared depth, so the new comparison accepts it.

`raster_sky_only_shades_background_after_opaque_ground` fails on the original
ordering and passes on the candidate, guarding both ordering and depth state.
Workspace: 513 passed, 23 ignored; clippy all targets, fmt, diff checks and release
build pass. No matched GPU screenshots or FPS measurements have been taken:
a live game was running throughout (later restarted with `--body moon`), and the
user was asked to close it for an uncontended comparison. No FPS improvement or
visual parity claim is justified yet; retain this only if the measurements pass.

Baseline is `201601a`. Preserved, independently named release binaries in the
same worktree: `target/release/catinthegarden-sky-before` (baseline) and
`target/release/catinthegarden-sky-after` (candidate). The main release executable
is also the candidate. No separate checkout shares the target directory.

Next steps once the live game is closed:
1. `python3 /tmp/catingard-sky-benchmark-201601a.py stand_on_ground 2` runs two
   interleaved Quadro Immediate-present pairs, sampling spatial frames from 2s.
   The harness refuses to run while the live app exists. It appends raw samples,
   medians and run paths under `test-runs/performance/sky-occlusion-201601a/`.
2. Check the captures against one another (expected identical); if promising,
   run another eight pairs with the same command/count 8. Do not compile or run
   another GPU process during measurements. Exposure is live in this scenario;
   use fixed-exposure `mountain_ground` and `ocean_ship_float` for additional
   exact-pixel controls, plus a sky-only/airless control for missing backgrounds.
3. Keep only a convincing measured gain without a visual regression; otherwise
   revert this bounded candidate. Update these pending notes with honest results.

No unrelated terrain/tree work or `crates.tar.gz` was touched. The prior seafloor
hole remains open independently of this candidate.

## 12 September — sky-overdraw measurement complete; candidate retained

Baseline `201601a` and candidate `2429540` were rebuilt/saved separately as
`target/release/catinthegarden-sky-before` and `-after`. The change is confined
to raster draw order, the sky's depth comparison and invariant vertex position;
no atmosphere sampling, image resolution or terrain/ocean quality reduction.

Twenty interleaved `landing_site_ground_detail` pairs on Quadro M1000M,
1280x720, Immediate present: baseline first on odd pairs, candidate first on even
pairs. Each run contributes its median of 13 spatial-frame samples from 2s
through 8s. All 20 pairs are included, not just the quieter later runs:

- Median frame time **45.85065 -> 45.29160 ms**.
- Reciprocal median FPS **21.80994 -> 22.07915 (+1.234%)**.
- 18/20 paired medians improve; paired mean saving **0.55407ms**.
- 100,000 paired bootstrap resamples, seed 9, give **0.08714-0.97178ms**
  as the descriptive 95% interval for the mean paired saving.
- The first ten-pair interval included zero, so no gain was claimed at that
  point. A final ten-pair confirmation was added with unchanged binaries and
  the complete twenty-pair result retained. This is an exploratory benchmark,
  not a preregistered significance test or a whole-game FPS claim.
- Initial `stand_on_ground` screening (two pairs) was noisy/inconclusive; its
  five matched captures were nevertheless identical. Ground-detail isolates
  a predominantly terrain-filled view where the removed sky overdraw applies.

Evidence and rerun scripts are under
`test-runs/performance/sky-occlusion-201601a/`: raw `landing_site_ground_detail.jsonl`,
`summary.json`, `parity-audit.json`, `binaries.sha256`, `benchmark.py`, and
`controls.py`. Timed runs span `1789225519-213056` to `1789226648-214907`.
The audit compares all 40 ground capture pairs byte-for-byte in decoded pixels:
**zero differences**. All settled camera position, FOV, draw/chunk/triangle
counts and exposure fields also match between paired runs. The prior underside
approximation has localized differences; this optimization does not repeat
that trade-off.

Additional passed, pixel-identical visual controls:

| Control | Before run | After run | Captures |
|---|---|---|---:|
| Ocean / horizon | `ocean_ship_float/1789226719-215068` | `1789226744-215096` | 3 |
| Airless moon | `moon_ground_detail/1789226768-215144` | `1789226792-215168` | 2 |
| Sky-only | `limb_atmosphere/1789226817-215291` | `1789226824-215342` | 1 |
| Orbital limb, 19Mm to 10Mm | `orbital_atmosphere_continuity/1789227340-216897` | `1789227372-217017` | 12 |
| Flat terrain | `mountain_ground/1789227396-217079` | `1789227408-217094` | 1 |

The initial desktop limb comparison (`1789226704-215012` vs
`1789226712-215048`) was **not a matched control**: logs show FOV 25.7178 vs
45 degrees, explaining its large full-frame image changes. That scenario does
not pin FOV, and live wheel input can still change it. Do not interpret those
images as either parity or a rendering regression. The replacement orbital
scenario explicitly pins 60 degrees, and it and the flat control were run on
an isolated Xvfb display to prevent desktop input from contaminating them.
They still use the Quadro GPU; **all reported timing measurements remain on
DISPLAY=:0**, not Xvfb. Early seafloor desktop controls were also interrupted
(manifest passed=null, no captures) and provide no validation.

The depth-zero concern is addressed both by the orbital controls and the
pipeline contract: the opaque terrain pipeline writes depth with strict
`Greater` against reverse-Z clear 0, so a fragment at exactly zero cannot have
written an opaque colour there. Shoreline blending does not write depth and
runs later. The sky does not write depth; all optical snapshot ordering remains
intact. The ray path is unchanged.

The failing-before/passing-after ordering/depth regression and the full 513-test
workspace suite, clippy, fmt, diff checks and candidate release build passed at
`2429540`. No further renderer code was added during measurement. Existing
`crates.tar.gz` and concurrent terrain diagnostic edits are not part of this
change.

### Claude review: seafloor coverage evidence and underside disclosure

The earlier underwater FPS report now includes maximum channel changes,
threshold counts and waterline bounding boxes, independently recomputed from
the original image pairs (see its corrected section above). Small averages do
not establish unchanged silhouettes or motion.

Claude's `response/claude.txt` reports a useful **coverage**, rather than fragment
rejection, diagnostic for `ocean_seafloor_hole`: colouring terrain before every
discard yields 21.8% terrain, 29.0% underside and 49.2% untouched background.
The missing region is the nearest bed, not the far bed. Claude also reports
ruling out the 5cm near plane and the existing below-datum horizon special case;
the 1024-leaf budget experiment did not complete, so that hypothesis remains
untested. These layer-colour experiments were not rerun here and remain
collaborator evidence, not newly measured Codex results. Prioritize near-field
coverage/frustum selection and the bounded budget experiment when resuming the
hole fix; do not repeat a search for a fragment discard as though it were the
leading explanation. The hole is still not claimed fixed by either FPS pass.

## 12 September — deterministic seafloor hole traced to rounded patch anchor and repaired

`ocean_seafloor_hole` at 31.18s selects and submits the camera-containing
PositiveX L18 node `(98243, 207998)`, with its L4 source resident and its
near-field window sampling **-4.192524m** at the eye (direct source
**-4.192509m**). The missing bottom was not a skipped node, an all-land ocean
cull, a fragment discard, or absent bathymetry. The ordinary CPU surface probe
clamps submerged macro to sea level; its misleading sea-level clearance was not
the raster bed height.

The node's f32-normalised anchor direction, converted back to f64 and multiplied
by the 4,000km radius for its camera-relative origin, has radius **R+0.499m**.
The eye is R-3.6925m. A -4.2m bed therefore reaches within about 1cm of the
eye, behind the 5cm near plane. Controlled probe builds showed that no-cull
alone left most of the hole, while lowering only the camera-containing chunk
from -4.2m to -5.2m filled the bottom 60 rows. The latter was diagnostic, not
retained terrain editing. This supersedes the earlier near-plane dismissal:
the nominal camera near plane was correct, but the geometry was displaced into it.

The repair uploads one f32 per instance: the f64-measured radial excess of its
anchor. The high-precision interior patch projection subtracts that excess;
the global shared-edge projection is unchanged, where anchor subtraction
already cancels exactly. Source elevations, near plane, chunk selection, and
ocean optics are unchanged. At the matched pose the center lower sample changes
from flat water `(2,35,92)` to sediment `(227,220,192)`, the lower half's
**417,794 exact flat-water pixels become zero**, and the GPU surface probe goes
**41/81 to 81/81** hits. The scenario now asserts the lower sediment sample, so
the archived old capture would fail its red-minus-blue threshold; the repaired
replay `test-runs/ocean_seafloor_hole/1789246377-251137` passes.

The `ocean_waterline_flat`, `ocean_ship_float`, and `mountain_ground` GPU replays
pass, as do 516 workspace tests, clippy with warnings denied, and fmt check.
The waterline replay is the existing shared-edge seam control. This establishes
the deterministic hole fix, not that every camera pose or visual swim path has
been accepted; ask for a fresh manual underwater pass. Claude's concurrent
`planet.wgsl`/`shared_planet.wgsl`/`terrain.rs` optical work and unrelated
`crates.tar.gz` remain unstaged by this fix.

## 13 September — shallow-water turquoise keyed on the column, and a filtered refracted bed

User report: turquoise appears too readily and should belong to thinner water,
and the refracted sea bed is aliased. Three separate changes, measured
independently. Ordinary `xvfb-run` was used for every replay below: on
`DISPLAY=:0` these scenarios stall in exposure warm-up with the GPU at 0% and
write no captures, which is the same silent no-evidence failure the earlier
seafloor controls hit.

**Which turquoise.** Two mechanisms are named for this and neither was
responsible. Raising `OCEAN_CREST_TRANSMISSION_ONSET` from 0.25 to 0.95 *and*
squaring its ramp moved `ocean_clear_shallows` by 4 channel levels; its
`backlight` gate barely fires there. Setting `OCEAN_SHALLOW_COLOUR` to magenta
and re-rendering changed **zero pixels**. The visible tint is a third term,
`shallow_mix = bed.w * 0.82` in `ocean_fragment_with_transmission_mode`, whose
old comment claimed it "fades with the same depth transmittance". It does not:
`bed.w` is transmittance along the slant range to the bed, not a function of how
much water stands over it, so at `OCEAN_UNDERWATER_VISIBILITY_METERS` of clear
water a distant bed under twenty metres qualified exactly like an ankle-deep bar.

**Repair.** The mix is now weighted by two-way extinction over the instantaneous
column -- still depth plus the wave's own displacement, the quantity the surf
line already uses -- on the same e-fold as the underwater fog (25.562m). 3m
keeps 79% of the tint, 10m 46%, 20m 21%, 40m 4%. A hard 6m cutoff was tried
first and rejected: it drove three of four `ocean_clear_shallows` captures to
**0.00%** turquoise, because clear water legitimately shows a bed from deeper
than any single authored threshold. Matched pair `1789231780-223579` (stock)
against `1789232229-224850`: turquoise coverage 5.54->4.11, 3.37->0.75,
8.08->2.47 and 8.08->2.47 percent, maximum channel delta 8.

**Refracted bed filtering.** `ocean_scene_transmission` fetched the snapshot with
`textureLoad` at integer coordinates, i.e. nearest texel, while
`ocean_reflection_scene_position` immediately below it already hand-rolls
bilinear interpolation and records why ("nearest-pixel depth produces a
staircase at grazing angles"). The colour binding is declared
`filterable: false`, so a linear sampler is not available and the filter is
hand-rolled, reusing that function's silhouette guard so a 2x2 footprint
straddling a depth discontinuity keeps its single texel rather than dragging
distant land into the bed. Matched pair `1789231814-223815` against
`1789231832-223958` on `ocean_shallow_transmission`: mean absolute second
difference, a jaggedness proxy, falls 1.0470->0.8259 and 1.0359->0.8181 on the
two bed-dominated captures (**-21%**) and about -2% on the other two, maximum
channel delta 12.

**Crest transmission re-anchoring.** `OCEAN_CREST_TRANSMISSION_ONSET` 0.25 and
`FULL` 0.75 were chosen from the calm amplitude column, but
`GLOBAL_OCEAN_STORM_INTENSITY` is 1.0, so `storm_blend` is 1.0 always and the
calm column is never rendered. Re-sampling the summed crest sharpness over
600,000 random phases reproduces the documented calm p90 0.225 / p99 0.381 and
storm p75 0.509 / p90 0.950, and gives storm p95 1.202, p99 1.534, p99.9 1.709.
Against the column that actually runs, the old pair put 36.9% of the sea under
some tint, 25.3% over half strength and 15.8% clipped flat at the top of the
ramp -- so the old note that 0.75 "is reached only by a storm's sharpest crests"
was wrong by more than an order of magnitude. Now onset p90 (0.95) to full p99
(1.534) with a squared ramp: 8.2% tinted, 2.8% over half strength. Its measured
effect in these scenarios is under 4 channel levels, so this is a latent
calibration repair, not the visible turquoise change.

**Refuted here, recorded so it is not retried.** Terrain back-face culling was
tested as a cause of the seafloor hole before Codex found the anchor error.
Disabling it globally moved the deterministic replay from 49.20% to 41.29%
background, but all 72,954 changed pixels are the single colour `(3,41,102)`
against a `(2,35,92)` background, the maximum change is 10 levels, and the count
of warm sediment pixels is byte-identical at 412,243. Fully fogged back faces
asymptote to a constant, so uniformity alone does not distinguish geometry from
a fill; varied shading does. No culling change was retained.

516 workspace tests, 0 failed, 23 ignored; clippy with no warnings; fmt clean.
Two new source tests, `refracted_bed_is_filtered_rather_than_point_sampled` and
`shallow_turquoise_is_weighted_by_the_water_column_not_bed_visibility`, both
verified to fail when their change is reverted. `ocean_clear_shallows`,
`ocean_shallow_transmission`, `ocean_coastline` and `land_chunk_seams` replays
pass. Human acceptance of the new shallow-water colour in motion is still
outstanding. `crates.tar.gz` untouched.

## 13 September — regression cover for the patch-anchor correction

The anchor repair in `4565768` was guarded only by its scenario assertion. Its
Rust test recomputes the correction inside its own body, so it never touches the
production path: removing the subtraction from the shader *and* zeroing the CPU
measurement left all 516 workspace tests passing. The scenario assertion does
bite -- a build with the shader subtraction removed fails
`submerged_seabed_is_sediment_not_water: required red-blue margin 0.080,
observed -0.353 from sample Some([2, 35, 92])` -- but it only fires when someone
runs that replay, and that replay is one of the ones that silently captures
nothing on `DISPLAY=:0`.

The CPU measurement is now a named `anchor_radius_excess_meters` rather than an
inline expression, so a test can call the real thing, and
`patch_anchor_radius_excess_is_measured_and_subtracted_in_the_interior` pins all
three links: the measurement is non-zero and cancels the radial error for the
`ocean_seafloor_hole` node, the shader's interior projection subtracts it, and
`update` ships the measured value rather than a constant. That last assertion is
scoped to the body of `update` so it cannot match its own literal, which is how
the neighbouring arithmetic test came to pass vacuously. It also pins the
shared-edge branch still resolving from the global face UV, where the anchor
cancels exactly and the correction must not be applied.

Each of the three links was verified by mutation: removing the shader
subtraction, returning zero from the helper, and replacing the instance value
with `0.0f32` each fail the test. 517 workspace tests, 0 failed, 23 ignored;
clippy with no warnings; fmt clean. No renderer behaviour changes in this
commit. `crates.tar.gz` untouched.

## 13 September — the beach/land contrast is a cloud shadow, and the tint falls off faster

User reported, from a remote session they could not move the camera in, that
the beach and the land beside it contrast wrongly, guessing at either a
time-of-day difference or separate beach lighting rules, and that the turquoise
is still slightly too strong as a wave comes in.

**The contrast is a cloud shadow, not a material seam.** Manual captures
`test-runs/manual/1789280549-280895` at commit `6aa67be` show a straight,
stair-stepped boundary with lit sand on one side and a drab neutral surface on
the other -- a 148-level jump in a single pixel. Two things rule out material.
The per-channel ratios across the edge are 2.43/2.26/1.90, not a common factor,
and the two sides carry the same dune ripple texture. Solving
`lit = A + D`, `shadow = A + 0.12 * D` for each channel gives an ambient share
of 38% red, 43% green and **53% blue** -- ambient largest in blue is exactly
what surviving skylight looks like, and 0.12 is precisely the visibility left by
a full-strength four-band posterized cloud shadow.

Eight bearings swept from the user's own logged position confirm the terrain
material is not responsible: the largest single-pixel jump in any of them is 52
levels against the reported 148, and the land there is sand in every direction.
Reproducing that position needs the logged world position and sun direction
rotated by `-planet_rotation_radians` (4.5200 here) to reach the planet-fixed
frame a scenario starts in; rotated, the pose lands next to
`ocean_seafloor_hole`'s, which is the cross-check that the rotation is right.
The diagnostic scenario was not retained.

**Repair.** `cloud_shadow_visibility` posterized the ground shadow onto the
cloud presentation's four hard bands. On the ground a cloud edge crosses several
bands within a few pixels, so terrain stepped from full sun to 12% of it in one
pixel with a stair-stepped boundary from the density sampling -- on a beach, a
hard straight line between two shades of the same sand. The band structure is
kept, including the flat quarter at each end of every band, but the band edge
now ramps: `(floor(scaled) + smoothstep(0.25, 0.75, fract)) / bands`. Visibility
is continuous in density, and the old hard step from 1.000 to 0.780 between
densities 0.10 and 0.15 becomes 0.952 then 0.828.

This was **not** confirmed against a matched capture. The user's weather state
at 863s of their session is not reproducible from a scenario, and no ground-level
scenario here renders a cloud shadow over sand. The evidence is the arithmetic
above on their own pixels plus the continuity of the repaired function; a fresh
manual pass under cloud is what would confirm it.

**Turquoise falloff.** The shallow tint's e-fold is now the named
`OCEAN_SHALLOW_TINT_EFOLD_METERS`, 9m, measured rather than derived. The
committed 12.8m (half the open-water visibility e-fold, for the two-way path)
left too much of the approach tinted; `OCEAN_SHALLOW_DEPTH_METERS` itself, 6m,
removed the effect almost entirely. Obviously-turquoise screen area over the
four `ocean_clear_shallows` captures:

| setting | c1 | c2 | c3 | c4 |
|---|---:|---:|---:|---:|
| stock before any of this work | 5.54% | 3.37% | 8.08% | 8.08% |
| 12.8m (previous commit) | 4.11% | 0.75% | 2.47% | 2.47% |
| **9m (retained)** | **3.24%** | **0.41%** | **1.53%** | **1.53%** |
| 6m (rejected, too far) | 1.97% | 0.03% | 0.23% | 0.23% |

518 workspace tests, 0 failed, 23 ignored; clippy with no warnings; fmt clean.
`cloud_shadow_bands_are_continuous_where_they_meet_the_ground` is new and
mutation-verified against the restored posterization; the existing
`terrain_cloud_shadows_reuse_the_shared_density_and_project_toward_the_sun`
caught this change and was updated to pin the banded form at the same 0.88
strength. Human acceptance of both the softened shadow and the weaker tint in
motion is outstanding. `crates.tar.gz` untouched.

## 13 September — boids: flocking birds that land, walk and take off

New feature at the user's request. `birds` holds the simulation and `birds_render`
the GPU side, split the way `ship` and `ship_render` are so the flocking is
testable without a device. Birds live in the planet frame in f64 and only narrow
to f32 after the camera-relative difference is taken, which is the same contract
the ship's hull origin has and the reason a 0.42m bird is not quantised away at a
4,000km radius.

**Behaviour.** Reynolds separation, alignment and cohesion, plus a flock anchor,
an altitude hold against the ground below and a slow wander so a balanced flock
does not freeze into a line. Flocks decide together rather than individually:
cruise, settle, walk, lift. Landing is a real approach -- a bird becomes a walker
only when it is inside 0.35m of the ground and under 2.2m/s -- and walking is a
slow tangential drift with pauses and folded wings. Walk near a settled flock and
it leaves, which is what birds do.

**Streaming.** Flocks spawn in a 280-460m shell, beyond the range at which a bird
covers a pixel, and are given up past 900m. They travel at 7.5m/s on a heading
biased across the camera, so they pass rather than either ignoring the player or
homing in.

**A bug worth recording, because the obvious test missed it.** The population was
first topped up by counting *every* flock. Once flocks drift, all of them can sit
between the 620m draw distance and the 900m despawn distance at the same time:
alive, so nothing respawns, and the sky is empty while the simulation reports a
full population. A `bird_flyby` replay showed six flocks and about a hundred
birds with **drawn_birds 0** on every frame, while the six-second
`stand_on_ground` replay showed 101 of 101 drawn because nothing had drifted yet.
The fix counts only flocks within the draw radius, evicts the furthest when the
total cap would block a top-up, and settles the population *after* moving the
flocks so the invariant is true of the frame that gets drawn.

The first regression test written for it asserted only "some bird is drawable",
passed under the broken policy by luck on the first seed, and had to be replaced
with one asserting the policy itself across four seeds. Both halves of the fix
are mutation-verified: reverting either the near-count or the eviction fails it.

**Rendering.** One instanced draw, eighteen triangles a bird: a spindle body, two
wings emitted with both windings so a bird overhead keeps them under back-face
culling, and a tail. The wingbeat is a per-instance phase hinged about the bird's
forward axis in the vertex shader, so no per-bird geometry is ever uploaded and a
walking bird folds its wings from the same parameter.

**Verified.** 530 workspace tests, 0 failed; clippy with no warnings; fmt clean.
Twelve of those tests are new, covering spawn and retirement, the land/walk/take
off cycle, birds standing on the ground and never under it, flock spread without
collapse, refusal to spawn over water or unresolved terrain, seed determinism,
and the drift regression above. A `stand_on_ground` replay draws **101 of 101
birds, 1,818 triangles**.

**Not verified: what they actually look like.** No capture shows a bird at close
range. `stand_on_ground` draws them but its pose faces into shadow;
`bird_flyby` is the scenario meant for this and it stalled in exposure warm-up on
this box more than once, which is the `DISPLAY` flakiness noted earlier rather
than anything about the birds. The counts prove they exist, are in range and are
being drawn; they do not prove the model reads as a bird. Ask for a manual pass.
`crates.tar.gz` untouched.

## 13 September — walking birds were standing inside one another

Asked whether two flocks would converge into one under the boid rules. They
cannot: `advance_flock` builds its snapshot from `flock.birds`, so separation,
alignment and cohesion never see across a flock. Measured over four seeds and
240 simulated seconds, the closest two birds in *different* flocks ever came
was 1.32m and the closest two flock centroids 9.43m, so they pass near one
another without interpenetrating. Flocks are structurally independent and stay
that way.

Checking it surfaced a real defect in the other direction. Splitting the closest
same-flock pair by activity gave flying 0.559m, takingoff 0.450m, landing
0.239m and **walking 0.014m** -- for a bird about 0.5m long, one standing inside
another. `step_walking_bird` applied no separation at all: the three rules are
evaluated only for birds on the wing, and a settled flock had nothing keeping it
apart.

`walking_separation` now pushes a grounded bird off its neighbours in the ground
plane, linear in the overlap and capped at twice a stroll so a crowded bird steps
aside rather than sliding through. Settled pairs measure 0.683m apart with it in.
The overall minimum is 0.132m and is the instant a landing bird touches down
beside a settled one, before separation resolves it over the next few steps;
both bounds are pinned.

`a_flock_keeps_together_without_collapsing_onto_one_point` had a `continue` that
skipped any flock containing a grounded bird -- exactly the case that was broken,
so it could never have caught this. The skip is gone and
`birds_never_stand_inside_one_another` covers every activity across four seeds;
removing the separation reproduces the original 0.014m.

531 workspace tests, 0 failed; clippy and fmt clean.

## 13 September — bounded road-surface experiment, not graded road geometry

The first trial at the old +X landing pose put asphalt directly on a steep
snowy wall: a useful rejection of the idea that colouring a slope makes it a
road. The retained `road_surface_trial` uses a surveyed desert location instead.
The current L4 macro source changes by only 1.53m over its 650m S-curve, with
a maximum 0.44% grade between 10m samples; the runtime detail field is **not**
covered by that number, so there is no 8% road-grade guarantee.

With `CATINGARDEN_ROAD_EXPERIMENT=1`, raster terrain shades a roughly 9m
asphalt centre with gravel shoulders feathering into the existing material by
14m. Its centreline is a bounded two-stage smooth S-curve in planet-local
metres. It changes no terrain vertex, collision height, draw call, depth,
quadtree policy or ocean. It is a surface/material experiment, **not** a
cut/fill mesh or a tunnel. The shader source selects a constant at startup;
with the environment variable absent or 0, the road branch is compiled out.
The moon and ray path are unaffected. Run the fixed pose with:

`CATINGARDEN_ROAD_EXPERIMENT=1 /home/dad/catingard-target/release/catinthegarden-app --scenario road_surface_trial`

At 1280x720 on the Quadro M1000M, Immediate present, fixed exposure, four
interleaved off/on pairs alternate order. Each run contributes the median of
13 spatial-frame times from 2s through 8s, with all eight runs retained:

- Off median **41.661ms / 24.00 FPS**; on median **42.656ms / 23.44 FPS**.
- Paired on-minus-off frame times **0.879, 1.750, 1.035, 0.937ms**; paired
  median overhead **0.986ms**. The 2.33% FPS loss is meaningful for a single
  painted section and argues against doing a whole network in the terrain
  fragment shader. These are small exploratory samples, not a long-run GPU
  confidence interval or evidence of the cost of a dedicated road mesh.
- All eight manifests pass, with 179.58m camera clearance. Every matched
  capture pair differs at exactly 42,019 pixels confined to road screen box
  x=540..672, y=89..719; pixels outside that box are identical. A retained
  road-on capture is `test-runs/road_surface_trial/1789293121-315868/screenshots/capture-001.png`.
  Raw runs and summary are under
  `test-runs/performance/road-surface-trial-98f528c-cubic/`.

The next implementation should be a **local, visible-segment-only corridor**:
plan a spline over actual rendered-height samples with a chosen maximum grade
and curvature, solve a bounded elevation profile, and deform the shared
terrain height/collision field over shoulders so cuts and fills reach the old
surface continuously. Conform or refine neighbouring triangles near the
corridor rather than increasing every terrain chunk's grid. On steep ground,
search a longer contour/switchback route before accepting expensive cut/fill;
if no permissible route exists, a tunnel is a separate portal/interior and
terrain-hole problem. Forest and other object placement must exclude the road
footprint. None of those claims is implemented or performance-measured here.

## 13 September — flocks join up, bounded by a mutual visibility limit

User's rule, and it is self-limiting by construction: a flock can only see other
flocks while it is small, and a flock that is not small cannot be seen either.
So merging is mutual, and a flock that grows past the limit goes blind and
invisible at the same moment, which stops the process without anyone having to
cap it from outside.

`FLOCK_MERGE_VISIBILITY_BIRDS` is 16: at or above it `is_mergeable` is false, so
the flock neither absorbs nor is absorbed. Mergeable flocks inside
`FLOCK_MERGE_ATTRACTION_METERS` bend their drift toward the nearest one they
could actually fit with, so joining up is something they do rather than
something that happens to them by chance, and they combine once their centroids
are within 20m. One pair per step, so nothing can cascade several flocks into a
swarm inside a single step. The larger flock keeps its heading and plans; the
smaller joins it.

`FLOCK_MERGE_CEILING_BIRDS` is 30 and a merge that would exceed it is refused.
With these constants that check is redundant -- two flocks each one bird under
the visibility limit come to exactly 30 -- which is why deleting it fails no
test. The relationship is what actually needs guarding, so it is a compile-time
`const _: () = assert!(...)` rather than a test: raise the visibility limit
without raising the ceiling and the build stops.

`birds_render`'s instance buffer is now `birds::worst_case_bird_count()`, every
flock at the ceiling, instead of a round number picked to look safe.

Measured before the change, to answer the question that prompted it: flocks
could not merge at all, because `advance_flock` snapshots `flock.birds` and the
three rules never see across a flock. Over four seeds and 240 simulated seconds
the closest two birds in different flocks came was 1.32m and the closest two
centroids 9.43m.

533 workspace tests, 0 failed; clippy and fmt clean. Merging fires (a counter,
not an inferred size threshold -- a merge of two flocks under the visibility
limit lands in the range a single spawn already produces, so size proves
nothing). Disabling the merge fails the join test; removing the visibility limit
fails the limit test with a flock of 23 still merging. `largest_flock` and
`flock_merges` are in the frame log, because merging is otherwise invisible in a
replay: the bird count does not change and the flock count falls the same way a
retirement makes it fall.

## 13 September — flocks that cannot merge now avoid each other

User observed that the visibility limit reads like nature: a flock knows it has
reached an advantageous size, and others keep away. The first half was true; the
second was not. A large flock was not avoided, it was *invisible* -- `is_mergeable`
returning false removed it from consideration entirely, so a small flock had no
reason not to fly straight through it.

Measured before changing anything, that never actually happened: over five seeds
and 300 seconds a small and a large flock came no closer than 19.77m centroid to
centroid. But by luck, not by rule. At six to ten flocks over a 900m radius a
specific pair rarely meets, and raising the flock cap or tightening the spawn
shell would have broken it.

The rule is now complete rather than having a third do-nothing case: a pair that
could merge is drawn together, and **every other pair pushes apart**, including
large against large. `attract_mergeable_flocks` became
`steer_flocks_past_each_other` and now steers every travelling flock, not only
the mergeable ones.

**That change alone did nothing, and the measurement is why this entry exists.**
Small against large stayed at 19.77m with the avoidance in and 19.77m with it
out -- identical to the digit. Timing the closest approach found it at
**t=0.1s**, the first step after a spawn, with a flock 0.1 seconds old. The
minimum was a placement artifact and no steering rule can undo where a flock was
put. `spawn_flock` now retries up to eight bearings and refuses to place a flock
within 80m of one it could not merge with; pairs that *could* merge may still
start close, since those are meant to find each other.

With both, the closest approach is **32.61m and occurs at t=268.6s**, during a
real encounter rather than at birth. Both halves are load-bearing and
mutation-verified: without the spawn separation it is 15.96m, without the
avoidance steering 13.34m, and the test fails in each case.

535 workspace tests, 0 failed; clippy and fmt clean.

## 13 September — road trial POV drive replay

`road_surface_trial` now moves the camera automatically rather than holding an
oblique fixed pose. Twenty-one one-second waypoints follow the same S-curve
painted by the opt-in road shader at 14m/s for 20s, aiming 45m ahead; seven
captures show the road from a forward-facing 70-degree view. The waypoint
radius is only a fallback until terrain loads. The replay samples the current
raster mesh surface twice (the second query at the corrected eye altitude),
then moves eye and look target together so view pitch does not jump. It holds
2m clearance without changing free-flight/walking controls or terrain.

The scenario sets `skip_birds` so this focused diagnostic does not pay for
unrelated bird ground sampling. A first, unskipped ground-level replay spent
more than seven minutes inside `BirdFlocks::advance`/forest slope queries before
its first capture; this is a separate performance issue, not evidence that the
road shader or camera path is slow. The new flag defaults false, so normal play
and every other scenario still simulates birds.

Run with `CATINGARDEN_ROAD_EXPERIMENT=1 /home/dad/catingard-target/release/catinthegarden-app --scenario road_surface_trial`.
Quadro Immediate-present replay
`test-runs/road_surface_trial/1789295599-342703` passes finite metrics,
seven captures and 2.000m clearance at all seven captures (allowed
1–3m). The new scenario regression pins 21
forward-moving centreline waypoints, captures and terrain-follow/skip settings.
The paint still follows raw terrain relief; this is a POV presentation of the
existing trial, not a graded drivable road, FPS improvement, or global road
network.

## 13 September — fixed-wind spectrum foundation

Road work was already pushed (`f8461ca`, response `61f2e5b`) before this phase.
Concurrent bird/marker/main changes and `crates.tar.gz` were left alone.

`CATINGARDEN_OCEAN_WIND="30,-0.3,0.75,-0.6"` enables reproducible startup
wind controls: speed in 0–30m/s, followed by a nonzero planet-frame propagation
axis (towards, not meteorological from). Invalid settings fail explicitly.
Absent settings preserve the previous sea. Long waves at >=1000m retain their
amplitude and propagation. The shorter components receive speed-scaled,
directionally weighted amplitudes and select the downwind sign of their
existing dispersion speed. Reversing wind therefore reverses travel rather
than just weakening the same forward waves. Maximum amplitude never exceeds
the previous component bound; wavelength, phase origin, geometry count and
horizontal-transport safety flag are unchanged.

CPU height, slope and vertical velocity use the same weights/signs as the GPU.
Weights are generated into WGSL constants once, avoiding per-fragment
normalisation, dot products or square roots for wind. The spawn-coast
shoreward override remains and accounts for the selected propagation sign;
use `CATINGARDEN_SPAWN_COAST_WAVES=0` to inspect the unsteered wind response.
The ripple layer also receives paired weights/signs but remains non-geometric.

Validation: 22 focused ocean tests pass (one ignored instrument); 549 workspace
tests pass with 23 ignored. Fmt passes; clippy completes with the concurrent
bird `WING_WRIST_LOCAL` dead-code warning. Actual production-WGSL parity passes
48 cases each at 0m/s, 15m/s and reversed 30m/s, including the spawn-coast blend
and shallow breaking depths: maximum height difference 0.000149m and normal
vector difference 0.00000968. The tests use a 64m test radius to isolate
algebra/derivatives; they do not prove planet-scale phase precision. Analytic
slope and velocity finite-difference tests also pass with all three settings.

Three Quadro/Vulkan 1280x720 Immediate `ocean_wind_trial` replays pass, with
identical stationary cameras and two captures each (2s/4s). Used `xvfb-run -a
-s '-screen 0 1280x720x24'` because the direct :0 connection aborted before
initialisation; the logs confirm the Quadro, not software rendering.

- Calm: `test-runs/ocean_wind_trial/1789317802-401721`.
- 30m/s towards (-0.3,0.75,-0.6): `1789317845-402669`.
- 30m/s reversed: `1789317878-402919`.

At 4s the logged diagnostic sample-grid height ranges are respectively
24.650m, 56.379m and 53.857m. Calm retains remote swell, not a flat sea.
The two windy final captures change 602,428 and 591,370 pixels against calm;
the top 200 sky rows are identical. These establish a rendered response, not
an FPS improvement or subjective visual sign-off. No matched performance
claim is made (tests/build activity overlapped some captures).

Run: `CATINGARDEN_SPAWN_COAST_WAVES=0 CATINGARDEN_OCEAN_WIND='30,-0.3,0.75,-0.6' /home/dad/catingard-target/release/catinthegarden-app --scenario ocean_wind_trial`.

Still outstanding: this is an authored finite-spectrum steady-wind experiment,
not a fetch/duration model, continuously turning wind, or the simulated local
weather field. Live transitions must not flip wave phases abruptly. Safe
horizontal crest compression plus inverse CPU surface queries, stronger
thickness-based scattering, foam quality and measured cost remain subsequent
phases. Do not describe this as Sea of Thieves-level completion.

## Bird ride camera on B, and forest beams removed - 13 September 2026

### Why the bird cam never worked for the user

The bird cam was reached through `CATINGARDEN_BIRD_CAM`, decided once at launch.
The user could not get it to do anything, and was right about the cause before I
was: **the game starts with the scene clock frozen and nothing spawns until F10
starts it**, so at the moment the launch-time choice is made there are no birds
and there is no flock to ride. No environment variable can fix that, because the
decision happens at exactly the wrong time. A key press, taken once a flock is
up, is the only shape that works interactively.

There was a second, independent fault that a scenario replay structurally cannot
catch. The bird cam sat directly beneath the authored scenario pose, which is
*above* the orbit, low-flight and surface camera controllers. A replay authors
its pose there and then leaves the camera alone, so the bird cam happened to be
the last writer and the shot worked. Interactively the surface controller runs
afterwards and rebuilds the pose from `flight_local_position`, putting the
player's own eye straight back every frame. So even with the variable set the
feature would have done nothing on the machine it was being tested on. It is now
the last camera writer of the frame, and
`the_bird_camera_is_applied_after_every_other_camera_writer` asserts the
ordering against all four writers; moving the block back where it was fails it.

### What B does

Pressing **B** picks the *nearest* flock with anything airborne in it -- nearest,
because the key is pressed while looking at a flock, and the flock being looked
at is the one meant -- and rides one of its rearward birds. Pressing **B** again
hands the eye back; nothing has to be restored, because surface and low flight
rebuild the pose from `flight_local_position` every frame, so the player returns
to exactly where they were standing. The target is dropped on the way out, so
the next press picks afresh rather than resuming a bird that may be a kilometre
away. While riding, the target is *held* until that bird is retired, because
re-picking each frame cuts between birds continuously. `CATINGARDEN_BIRD_CAM`
still starts a replay already riding.

### Which bird, and why it is measured

Two properties are wanted and they are not the same bird: being at the back puts
the flock in front of the lens, and being near the flock's own axis puts it
*straight* ahead rather than off to one side. So `Flock::rearward_bird` takes the
rear third along the flock's mean heading -- the mean, because a single bird's
heading wanders several degrees a second and reshuffles the order constantly --
and rides the most lateral-central of those.

Measured over four seeds, 60s each, counting frames with at least one flockmate
inside a 54-degree frame ahead of the camera:

| selection                                | frames with company |
|------------------------------------------|---------------------|
| rear third, most central (shipped)       | **99.5%**           |
| rearmost bird only, no centring          | 89.9%               |
| most central of the whole flock          | 86.0%               |
| whichever bird is first in the vector    | 83.6%               |

The test bar is 95%, so only the shipped rule clears it and each half of the rule
earns its place. Company cannot be an invariant: the target is deliberately held,
so a bird chosen at the back can drift forward later. What *is* an invariant is
that the camera looks the way the bird is flying, which the forward clamp in
`chase_camera` guarantees and the same test asserts on every sampled frame.

`a_ride_starts_on_a_rearward_bird_of_the_nearest_flock` covers the pick itself
against every flock the simulation actually produces. Mutating the selection to
the farthest flock, to the front third, or to allow grounded birds each fails it.

### Forest beams removed

The user is done with the beam overlay, and **B** is a better key for the bird
cam than for a debug shaft. Removed: `forest_beam.wgsl`; the beam pipeline,
vertex buffer, anchors, vertex builders, `toggle_beams`, `draw_beams` and the
startup refinement in `forest.rs`; the three `FOREST_BEAM_*` constants; the HUD
field, help text, log fields and key handler in `main.rs`; and, once the
compiler showed they had no other caller,
`TerrainRenderer::prepare_global_forest_locator_sample`, `TerrainForestSample`
and `TerrainStartupSamples::forests` in `terrain.rs`. `ForestRenderer::new` no
longer takes the global sample slice or a `&mut TerrainRenderer`.
`grep -i beam` returns nothing in `forest.rs`, `main.rs` or `terrain.rs`.

### Verification

547 workspace tests pass, clippy is clean on `--all-targets` and fmt is clean.
Release replay `bird_demo/1789323190-434266` under `xvfb-run` logs 6 flocks,
105 birds, 105 drawn, largest flock 23, 1 merge; frame 020 shows the ride from
behind a bird with eight flockmates in shot at a range of wingbeat phases and
the red flock reticle on the flock centre.

### Outstanding

Interactive **B** itself has not been pressed by a human yet -- a scenario replay
cannot press a key, so the key binding is covered by tests and the ride itself by
a capture, not by both at once. Orbit mode is the one camera mode that does not
restore cleanly on the way out, because its azimuth/elevation state *is* the pose
the bird cam overwrites; surface and low flight, the modes birds are watched
from, are unaffected.

## The bird cam shattered the terrain in Surface mode - 13 September 2026

Ian rode a bird and the world came apart into flat sand-coloured plates, long
slivers in the sky, and shards of ocean. He then found the tell himself: **F4
fixed it, F4 again broke it.** A key that toggles a bug names the state the bug
lives in, and F4 changes `camera_mode` -- so before any bisect, the answer was
already known to be in a `match self.camera_mode` arm.

It is this one, in `render`:

```rust
let camera_corrected = match self.camera_mode {
    CameraMode::LowFlight => self.enforce_low_flight_clearance(..),
    CameraMode::Surface   => self.resolve_surface_camera_after_streaming(..),
    CameraMode::Orbit     => false,
};
if camera_corrected { /* recapture camera, re-run terrain.update */ }
```

Both non-Orbit arms write the player's eye, and both run *after* the bird cam --
necessarily, because a post-streaming clamp has to see the patches that were just
streamed. So each frame in Surface mode:

1. the bird cam puts the eye on the bird;
2. `terrain.update` streams and builds every chunk's camera-relative anchor
   against *the bird's* eye;
3. `resolve_surface_camera_after_streaming` ends with an unconditional
   `sync_surface_camera_pose`, yanking the eye back to the swimming body;
4. anchors are rebuilt only `if camera_corrected`, which reports whether the
   *body* moved, not whether the *camera* did -- so the rebuild never fired.

The chunks were then drawn with anchors belonging to an eye that was no longer
there. At a 4,000km radius each chunk carries its own anchor, so each displaced
by its own amount: plates where a chunk went flat, slivers where two corners of a
quad landed far apart. `Orbit` returns `false` and never calls sync, which is
exactly why F4 was a toggle between broken and fine.

Fix: `player_camera_is_suppressed()`, checked by both writers. Only the camera
write is suppressed, never the physics -- the body keeps falling, swimming and
colliding, so **B** hands the eye back to a body that has been where it should be
all along.

### What the earlier investigation got wrong, and right

Two readings of the first capture set were wrong and worth recording:

- The manifest's 250m `max_abs_delta_meters` looked like a terrain fault. It is
  not: those probe rays hit **wave troughs at -39m** and are compared against the
  terrain height function, which returns 0 over sea. The surface probe is not a
  meaningful instrument for a ray that lands on water.
- Heavy ancestor fallback looked like the cause. It is not: `bird_demo` at the
  same commit and the same altitude has the same `fallback_chunks` (247/255) and
  the same `source_level_delta_histogram` shape, and renders correctly.

Right, and load-bearing: the giant sea predates all of this. Ian's own run
`manual/1789321529-426001` at `07360f0`, before any of the bird work, logged
waves from -38.6m to **+60.3m** and threw him to 132m altitude. That is the
storm-column problem already written up for Codex, and it is why he was 80m in
the air to see this at all.

### Why the ordering test did not catch it

`the_bird_camera_is_applied_after_every_other_camera_writer` was already passing:
source order *was* correct. The rule it encoded was wrong. The rule is not "the
bird cam writes last" but "nothing else writes while it is riding", and
`no_player_camera_writer_runs_while_the_bird_cam_is_riding` now encodes that
instead: it walks every `set_world_pose` in the production half back to its
enclosing method and requires the guard to sit between the two, so a new
controller cannot be added without meeting the rule. Deleting either guard fails
it. It scopes to the production half deliberately -- it names the very call it
searches for, and would otherwise match its own source.

## 13 September — crossing swell, first bounded follow-up to Claude's notes

Read tracked `response/claude.txt`, including the permanent-storm diagnosis and
bird-camera ownership warning. This patch implements only the third long swell;
**the normal game still uses storm intensity 1.0**. The calm/storm work is not
complete. No camera, bird, terrain or physics-controller code was changed.

Three 1400m entries replace the original pair: calm amplitudes .75+.75 become
.5+.5+.5; storm .18+.18 becomes .12+.12+.12. The third steepness is .425,
preserving summed steepness-weighted amplitude as well as the height bound.
Its planet-frame axis rotates the original pair's mean 60 degrees clockwise
around the documented spawn radial (not a global compass bearing). CPU and WGSL
both have 18 waves; generated wind arrays now derive their length from WAVES.
Horizontal transport remains disabled. Distribution percentile comments are
explicitly historical: a preserved maximum is not a preserved distribution.

Validation:
- 553 workspace tests passed, 23 ignored; formatting, diff check and clippy pass.
- Actual Quadro GPU parity, 48 cases each: default max height error .000123m,
  max normal error .00003082; 15m/s wind max height .000106m, normal .00000134.
- Release rebuilt at `/home/dad/catingard-target/release/catinthegarden-app`.
- NVIDIA Vulkan/Xvfb `ocean_wind_trial/1789336777-489406` passes, two captures;
  inspected `screenshots/capture-002.png`. Broad intersecting ridges are visible,
  but static capture is not motion acceptance or Sea of Thieves visual parity.
  Replay used `CATINGARDEN_SPAWN_COAST_WAVES=0` and no wind override to isolate
  the global spectrum. Normal launches include the crossing swell automatically,
  with the existing spawn-coast steering still enabled by default.
- No matched timing run: one added analytic component, no extra draw/mesh, but
  no claim of zero performance impact or an FPS improvement.

Next: explicit shared CPU/GPU sea state, respect scenario storm overrides,
slow calm/storm transitions (including amplitude-change vertical velocity), and
recalibrate crest transmission against the new sea-state distributions. Do not
try to fix permanent storms with gravity or buoyancy changes. Startup wind is
still opt-in; live wind/fetch evolution and improved scattering remain later work.
`crates.tar.gz` remains untouched and untracked.

## 14 September — reachable calm seas and phase-continuous sea-state variety

### Behaviour and controls

Normal launch, with no ocean environment overrides, starts in the existing calm
amplitude column. A cosine envelope reaches storm after 300 scaled ocean seconds
and returns to calm after 600. It does not reset any wave phases or rotate axes.
This is deliberately an authored, repeatable demonstration cycle, **not** a
weather-driven wind/fetch/duration model. The calm column retains large remote
swells; it is not glass-flat or a physically calibrated Beaufort-zero sea.

- `CATINGARDEN_OCEAN_STORM=0` fixes calm, `.5` intermediate, `1` the old storm.
- Without that override, `CATINGARDEN_OCEAN_WIND=speed,x,y,z` selects fixed
  intensity `speed/30`, alongside its existing directional spectrum filtering.
- Scenario `ocean_storm_intensity_override` is now honored, ahead of environment
  intensity. Scenarios without it retain legacy fixed storm for compatibility.
  Wind-axis filtering remains startup-global as before.
- HUD and JSONL show sea intensity. The cycle shares the existing ocean clock;
  current F10 behaviour freezes planet/sun composition, NOT ocean motion.

### Implementation

One startup-selected immutable sea mode, sampled by ocean time, supplies all
CPU heights, slopes, vertical velocities and the existing GPU intensity uniform.
No camera/controller ordering, terrain source, bird code, wave axes or culling
bounds changed. The analytic vertical velocity includes d(amplitude*scale)/dt
before the existing breaking derivative, avoiding a buoyancy mismatch during
rising/falling sea intensity. A finite-difference regression tests this production
helper at six transition times and three water depths.

Crest transmission now interpolates p90/p99 reference anchors across the actual
calm/intermediate/storm columns: .212/.351, .544/.880, .950/1.534. Calibration:
600,000 independent phase vectors, NumPy default_rng(14926), sin(uniform(0,2pi)),
using each table component's steepness * amplitude(blend) * (44+11*blend) *
2pi/wavelength * projected-axis length at the documented spawn radial. Raw
p90/p99 were .21183958/.35075187, .54402258/.88021640, .94997972/1.53303759.
These are reference-spectrum anchors, not universal local percentiles: wind
filtering, shore steering and axis projection elsewhere change the distribution.

### Validation / artifacts

- 556 workspace tests pass, 23 ignored; clippy, formatting and diff check pass.
- Quadro actual-WGSL parity at intensity 0/.5/1, 48 cases each: max height errors
  .000276/.000218/.000123m; max normal errors .00000805/.00002171/.00003082.
  These are small-radius derivative/parity instruments, not planet-scale error bounds.
- NVIDIA Vulkan/Xvfb replays all pass with two captures each, identical cameras:
  - calm: `ocean_calm_trial/1789354857-497194`
  - intermediate: `ocean_moderate_trial/1789354896-497538`
  - storm: `ocean_wind_trial/1789354917-497960`
- Viewed all three capture-002 images. Top 200 sky rows are byte-identical;
  storm capture-002 is entirely byte-identical to `1789336777-489406` from the
  preceding crossing-swell patch. Calm has broad, smoother swells; storm has
  steeper wind-sea ridges and whitecaps. All used spawn-coast steering disabled
  to isolate the global spectrum; normal launch retains that steering.
- Logged 25-point sampled maximum ranges over these four-second runs were
  70.734/69.033/73.510m. Do not call these significant wave heights or infer
  monotonic height from intensity: components interfere and calm retains swell.
- No matched performance timing: no wave/pass/geometry added, but no FPS claim.
- Release rebuilt in `/home/dad/catingard-target`. `crates.tar.gz` untouched.

Next: user motion review in the main game; couple sea-state targets to actual
weather/fetch rather than the authored cycle, preserving time derivatives and
phase continuity. Wind direction remains startup-only. Foam persistence and
improved volume-scattering/underside optics are still separate visual work.

## 14 September — real-weather sea targets, without amplitude/velocity jumps

Normal interactive launches now use Weather mode. Once per scaled ocean second
at most, after weather updates and before ship buoyancy, the main loop samples
`WeatherState::storm_intensity_at` at the camera's planet-local radial. This uses
the existing spatial bilinear and temporal weather interpolation; weather code
itself is untouched. A critically damped 120-second response tracks that target.
For a held 0→1 target it reaches .264241 at 120 seconds and .959572 at 600 seconds.
It starts calm; it does not start as a fully developed sea for the local weather.

The analytic segment retains both its current value AND velocity when retargeted.
That matters even when moving/teleporting between weather regions: neither wave
height nor buoyancy velocity snaps to a new value. The existing amplitude-rate
term continues to feed the production vertical-velocity query. Two segments are
retained, covering the ship's .25-second fixed-step backlog because retargeting
is spaced by at least one second. The per-query mutex protects a small CPU
response state; there is no new GPU wave, draw, texture or shader operation.
No matched performance run was made, so this is not a zero-cost/FPS claim.

**Scope:** the renderer still has one global sea-intensity uniform. All visible
water therefore shares the camera-region response; it is not a spatial ocean
simulation. Sampling independent per-vertex weather would require matching
spatial derivatives in CPU/GPU surface queries. Storm intensity is used, not a
wind-speed/direction/fetch/duration spectrum. Wind axes and weights are still
startup-only. The calm amplitude column still retains large remote swells.

**Controls:** numeric `CATINGARDEN_OCEAN_STORM=0..1` and explicit startup wind
remain fixed overrides. `CATINGARDEN_OCEAN_STORM=cycle` retains the former
600-second loop. Legacy scenarios retain their fixed endpoints; the new
`ocean_weather_response: true` flag explicitly chooses Weather mode, taking
precedence over numeric scenario/environment endpoints. The ocean follows its
existing scaled presentation clock; F10 still freezes composition, not waves.

**Validation:** four new regressions cover the main-loop weather hookup,
response continuity/history, bounded response under repeated fronts and
unchanged targets, and production buoyancy velocity versus finite differences
at six growth/decay times and three depths. The shallow-water difference uses
a 2.5ms half-step: the previous 10ms step had .009m/s truncation error near the
nonlinear breaking knee. Error tolerance remains .002m/s. Value/rate continuity
allows only 1e-15 rate roundoff at retargeting.

Initial Quadro Vulkan/Xvfb `ocean_weather_trial/1789356407-502578` passes with
three captures across 600 ocean seconds (2-second simulation steps, for state
coverage rather than motion acceptance). The actual beach target rises from
.000209 to .062733; filtered sea intensity ends at .038131. These all remain
below the existing .15 calm-column threshold: this is correctly mild weather,
not a manufactured storm demonstration. Capture-003 was visually inspected.
The previous deterministic 0→1→0 tests provide the stronger-front control.

Final-source validation: 560 workspace tests pass (23 ignored), clippy and fmt
pass, release rebuilt. Final weather replay `ocean_weather_trial/1789356606-504172`
passes with all three images pixel-identical to the initial run; checked 300
finite sea samples and 300 weather targets with the same ranges above. Fixed
storm control `ocean_wind_trial/1789356639-504381` passes and both images are
pixel-identical to the preceding stage's `1789354917-497960`. No shader changes
were made. `crates.tar.gz` remains untouched/untracked.

Next: interactive motion review while travelling between weather regions;
weather-driven directional spectra and fetch remain unimplemented. Do not
mistake this scalar storm response for either of those, or tune gravity to
compensate for storm-scale waves. The fixed/cycle overrides remain useful controls.

## Birds bank into turns, and the flock's rigidity is load-bearing - 14 September 2026

Two questions from Ian, one answered with a fix and one with a measurement that
says do not touch it yet.

### Why they did not roll

They could not. `birds.wgsl` built the bird's frame from `instance.up`, which is
the planetary radial, so `right = cross(up, forward)` was always horizontal and
every bird was permanently spirit-level however hard it turned. There was no
bank term anywhere in the pipeline.

There is now. `step_flying_bird` computes the signed yaw rate from the heading
swept between the previous and current step and banks by the coordinated-turn
angle, `atan(speed * yaw_rate / g)` -- the same relation an aircraft flies --
smoothed with a 0.22s exponential so the wings lean rather than flicker, clamped
at 0.9 rad, and zeroed when a bird lands. It rides through the instance buffer in
`motion.w` and rolls the frame about the bird's own forward axis in the shader.

The honest physics is nearly invisible at the rates a flock actually flies:
measured over 154,795 samples, median bank 2.9 degrees, p90 6.1. A bird leaning
three degrees does not read as banking, which is precisely why Ian asked. So
`BANK_EXAGGERATION` is 2.5, giving median 6.4, p90 13.8, p99 26.4 -- present in
ordinary cruise, emphatic in a hard turn. Four was tried and leaves them
permanently leaning (p99 36.2). Set it to 1.0 for the true angle.

### The flock is rigid, and that rigidity is holding two other things up

Ian asked whether the birds are rigidly controlled by each other and which
constant loosens them. Measured on a 22-bird flock over 1,134 samples, splitting
each bird's vertical motion into the part shared with the flock and the part its
own:

    var(flock mean) 0.349,  mean var(bird) 0.436,  common-mode fraction 0.800

1.0 would be perfectly in phase; independent birds would give 1/22 = 0.045. So
**80% of a bird's vertical motion is the whole flock moving as one body.** That
is why the porpiseing in the previous section is visible at all: it is not noise
that averages away, it is a flock-level mode. Horizontally the flock is a ball
about 9.5m across (rms radius 4.73m) with its closest pair at 3.10m.

Sweeping the three Reynolds constants, common-mode fraction and rms radius:

    baseline (16.0 / 1.6 / 2.2)    0.800   4.73m
    neighbour radius 8.0           0.844   4.77m
    neighbour radius 5.0           0.573   5.06m
    cohesion 0.8                   0.943   5.42m
    alignment 1.1                  0.281   4.62m
    all three loosened             0.312   5.34m

`ALIGNMENT_STRENGTH` is the lever and the only one that helps much: halving it
cuts rigidity 2.8x while *tightening* the flock slightly. Loosening cohesion
makes it markedly worse, because the anchor term then dominates and the anchor is
identical for every bird in the flock.

It is not changed, because it does not survive contact:

    alignment 2.2 (shipped)   flockmates in shot 99.9%
    alignment 2.0             94.7%   (bar is 95%)
    alignment 1.8             94.3%
    alignment 1.6            83.0%, and flocks closed to 14.79m

A 9% reduction already costs the framing bar. And these are **the same two tests**
that the altitude damping broke in the previous section: the ride camera assumes
a coherent group ahead of it, and inter-flock avoidance assumes flocks are
distinct bodies. Two independent attempts to loosen the flock have now hit the
same pair.

That is the finding: flock rigidity is load-bearing for the ride camera and for
avoidance, and all three want reworking together or not at all. Anyone taking it
on should expect to rewrite `the_bird_cam_has_flockmates_in_shot` and
`flocks_that_cannot_merge_keep_out_of_each_other` as part of the job, not to
sneak a constant past them.

### Note on measuring birds in a worktree

`assets/outmaps/` is gitignored, so a fresh worktree has no terrain. Bird
scenarios then log `flocks: 0` and render an empty sky, because spawning needs a
ground sample. Symlink `assets/outmaps` before drawing any conclusion from a
capture there -- three identical "sky" frames cost a detour before the log said
why.

## Wingbeat is a function of effort, and nothing else - 14 September 2026

Ian: "whenever they are slowing down, a bird should switch to glide position.
Then flap again while accelerating", and then, on seeing the first attempt, "the
animation should be based on the acceleration only, not the other way around."

The first version scaled a per-activity rate by along-track acceleration. The
second sentence is the correction, and it is the better design: the activity
cases are gone entirely and the wingbeat is now a pure function of how hard the
bird is working.

    effort = dv/dt + g * climb_rate / speed

the rate of gain of kinetic plus potential energy, per unit speed, which lands in
the same units as an acceleration. Smoothed over 0.45s, then

    beats = CLIMB_WINGBEATS_PER_SECOND * effort01
    glide = clamp(1 - beats / SET_WING_BEATS_PER_SECOND, 0, 1)

`GLIDE_EFFORT` (-1.4) and `CLIMB_EFFORT` (+1.5) bracket it, chosen so that zero
effort falls out at 3.1 beats a second -- exactly where the old hand-set cruise
constant was, so level cruising is unchanged by the rewrite.

Taking off now flaps hard because taking off *is* climbing hard, and landing sets
its wings because landing is losing energy. Nothing declares either. The clearest
sign it was the right shape: `step_flying_bird` no longer needs its `intent`
parameter at all, and the compiler said so.

Measured over 60s of a cruising flock, before and after:

    old (activity rate x along-track)   median 0.37  p75 0.53  p90 0.72
    new (effort only)                   median 0.07  p75 0.54  p90 0.98

Both spend about the same share of the time set (28.4% against 26.5%, in bursts
of 1.2-1.3s), but the new one is bimodal where the old one was mush: a bird is
either beating or coasting, which is the gait switch that was asked for, rather
than permanently half-flapping.

The glide pose matters as much as the rate. Wings that merely stop cycling read
as a bird frozen mid-beat; `GLIDE_DIHEDRAL_RADIANS` and
`GLIDE_WRIST_DROOP_RADIANS` settle the arm into a shallow V with the hand
drooping outboard, which is what a gull holds when it stops working.

### A vacuous assertion, caught by mutation and worth the warning

The smoothing assertion in `wings_set_when_a_bird_slows_and_beat_when_it_speeds_up`
passed with the smoothing deleted. The test bird had its `previous_glide` set but
not its `previous_velocity`, so the step saw no change in speed and computed no
effort at all -- the assertion was measuring nothing. The climb assertion had the
same shape of fault: it asserted on the wing *pose*, which cannot separate a
climbing bird from a level one because both have their wings out, so deleting the
whole potential-energy term left it green. It asserts on effort against a
level-flight reference now. Three mutations, three catches, only after fixing the
test twice.

## The bird spawner hung the game; birds land gently; N goes to them - 14 September 2026

### A high eye hung the game, core pinned

Ian, twice today: "the game froze the laptop again", and "i have to shut it down
as soon as i can as things get very hot". Manual run `1789399608-7296` shows
frames at a steady 43ms, then the log stops partway through a frame, straight
after the eye moved about 5km in one frame at 32x flight speed.

Flocks spawn 280-460m from the eye along the ground, but count toward the near
population only within 620m of the eye in three dimensions.
`spawn_missing_flocks` spawned until six flocks were near, giving up the
furthest whenever ten existed. With the eye far above the spawn shell -- or on a
valley floor under a hillside -- no new flock is ever near, so it spawned and
gave up flocks until eight placements in a row happened to be crowded. A test
at 1.5km: 30,005 spawn attempts in one bird step. The game runs up to 15 bird
steps a frame, and each attempt there is a real terrain sample, plus sea queries
over water. Present since the birds arrived in 98f528c.

In the game, `tour_mountains` (eye at 35km, birds not skipped): the old build
never got past its second frame, one core at 99.9%, killed at 240s
(`test-runs/tour_mountains/1789400783-15251`); the fixed build passes in 48.6s
(`1789401023-16741`) with no flocks, which is right that high. 48.6s is well over
the 10-11s this replay took in August; with no birds in it that is not the
spawner, and it was not looked into.

Now a placement must lie within the near radius less a 20m margin (a new flock's
centroid sits within 13.3m of where it is placed), and a step spawns at most one
flock per missing one. Near the ground this draws the same random numbers in the
same order: the one seeded failure in the bird suite read exactly 7.99m both
before and after. `an_eye_out_of_reach_of_the_spawn_shell_does_not_spin_the_spawner`
fails at 30,005 attempts on the old spawner and, with only the reach check
removed, on six flocks that could never be drawn. The bounded loop is not caught
separately -- with the reach check in place it cannot be reached -- and is there
so the loop's termination no longer rests on geometry in two functions.

### Landing birds arrive instead of skimming

The fly-by 5e5acb0 wrote up as a separate defect. A landing bird was pulled at
its patch with a constant force and a constant sink, so it reached the ground at
cruising speed and skimmed the touchdown band until it slowed by chance. It now
arrives: wanted velocity proportional to what is left to go, descent faster than
approach, capped at 8m/s; neighbours push a landing bird aside but never up; and
steering follows the bird's own activity, since a flock counts as grounded once
half of it is down and a bird still landing after that was hauled back to cruise
height. Flat ground, seeds 1-10, before and after:

    time within 1m of the ground while landing   27.7%          6.6%
    horizontal speed there, median / p90          6.3 / 10.6     1.7 / 3.5 m/s
    landings abandoned                            22%            1% (43 of 4,466)
    time to touch down, median / p90 / max        11.1/14.3/17   6.5/9.3/11.4 s
    seeds of 30 with two birds within 0.1m        2              0 (closest 0.21m)

Water gives the same. `birds_never_stand_inside_one_another` counts every pair
again, in the air too. `a_landing_bird_touches_down_instead_of_skating_along_the_ground`
had been left asserting only `low_share < 1.0`, which the old pull passes; it now
holds limits, which the old pull fails at 39% of an approach within a metre: under 15% of
an approach within a metre, a p90 under 5m/s there, a p90 under 11.5s to touch
down, and under 6% abandoned (a guard only; the old pull's 1.2% passes it). With
the 140m avoidance below it reads 7.2%, 3.5m/s and 9.2s over 1,416 touchdowns on
the ground and 7.0%, 3.5m/s and 9.1s over 1,518 on water, none abandoned. The
closest two birds come is 0.29m.

### The ride's heading no longer snaps when a flock lands

The bird cam pointed along the mean velocity of the flock's airborne birds. Once
the last one landed there were none, and it fell back to the ridden bird's
walking step: a 131.7 degree snap in one frame, and a flockmate in shot on only
47.5% of frames while walking. Each flock now carries a ride heading that turns
toward its birds' mean direction of travel by as much as the distance they
covered warrants (3m to come round), so a standing flock keeps its heading.
After: worst turn 0.71 degrees in a frame, a flockmate in shot on 99.9% of
frames and 100% while walking. `velocity_at` and `heading_at` lost their last
callers and are gone.

### N: go and watch birds that are down

`go_to_settled_birds` finds the nearest flock with birds on the ground or the
water and puts the eye `BIRD_WATCH_STANDOFF_METERS` (the 34m startle radius plus
11m) from them, on the side it came from and facing them; closer would put them
up. Surface mode stays on the surface, floating if that is water; low flight is
4m up; from orbit it says to fly low first. It stops the ride, which writes the
eye last. The overlay's "Bird watch" line says what it did or why it could not.
`watching_birds_that_are_down_finds_them_and_does_not_put_them_up` covers ground
and water. Not tried in the game.

### Flocks that cannot merge: a wider berth, and the test is a sample

`flocks_that_cannot_merge_keep_out_of_each_other` failed once landing changed:
7.99m against its 25m. The seeded simulation is not bit-identical between debug
and release builds, and the same code read 20.92m in release, so pass or fail on
a minimum over five seeds partly depends on the build profile. Every one of the
worst close passes probed was head on: both anchors turn straight away at the
avoidance radius, and the birds, still flying at each other, cannot follow in
time. It was already there: HEAD's landing gave 12 of 30 seeds under 25m, the
worst 4.50m.

Release, 100 seeds, 300s each (30-seed sweep for the aside variants):

    variant                                 under 25m   worst    mean closest
    90m, as it was                          38/100      0.84m    34.5m
    140m                                    17/100      7.53m    47.9m
    spawn 80m from every flock, 90m         32/100      0.84m    35.5m   flockmates in shot fails
    both                                    14/100      7.53m    48.5m   merges stop
    steer aside as well, 0.5 / 1.0 / 2.0    9 / 5 / 9 of 30             2.0 stops merges

140m is in. The bird suite passes in debug and release, and the debug 30-seed
sweep reads 3 under 25m, the worst 18.22m. Close passes are not fixed: 17% of
seeds still bring two such flocks within 25m at some point in 300s. Rewriting
the steering line in a way that changes only rounding moved 15 of 100 to 17, so
read these as rates. The spawn separation only asks whether the flock already
there is mergeable, not whether the pair could merge; fixing that measured as
barely helping and broke two other invariants, so it stays.

### Validation

On exactly this change -- HEAD plus the staged files, built in a separate
worktree -- 501 app tests pass, 0 fail, 20 ignored, and
`cargo clippy -p catinthegarden-app --all-targets` is clean. The working tree,
which also carries Codex's uncommitted fetch diff, reads 504 passed and 3 failed:
the two fetch tests drafted to fail on that diff, and `every_listed_scenario_loads`,
which asserts 96 scenarios and finds the uncommitted `ocean_weather_dry_landing`
as a 97th.

Two earlier claims, corrected. 5e5acb0's "502 app tests pass" was counted on a
tree carrying Codex's four uncommitted fetch tests: HEAD alone has 498
non-ignored tests, and this adds three. Its "clippy clean" ran without
`--all-targets`, which does not lint test code; one type-complexity warning was
already there.

Not in this change: Codex's uncommitted wind/fetch work (`ocean.rs`, `weather.rs`,
one `main.rs` hunk, its AGENTS and handoff lines), the two fetch tests and the
`ocean_weather_dry_landing` scenario drafted for that bug, and `crates.tar.gz`.

## 14 September — wind speed and bounded upwind fetch development

Weather mode now includes local wind speed and shoreline sheltering. The new
weather query converts each cell's east/north velocity into planet-frame vectors
BEFORE bilinear interpolation, interpolates the two temporal fields, and projects
onto the query tangent plane. A rotating-vector-field regression covers both
poles, opposite sides of a cube edge, temporal interpolation and tangency.
Weather simulation itself is unchanged.

Once per scaled ocean second at most, the existing lazy target callback traces
upwind along a great circle using resident unmodified baked terrain heights.
It uses 64 quadratically spaced taps out to 100km and six bisections at the first
sampled land crossing: at most 71 height queries including the origin. There is
no streaming, disk I/O or terrain edit. Unknown/nonfinite heights retain the last
known target and retry after a second, rather than treating missing data as sea.
A zero-wind or land-origin sample has zero developing fetch.

The target is `max(clamp(wind_speed/30), storm_intensity) * (1-exp(-fetch/25000))`,
then passes through the unchanged 120s critically damped response and existing
CPU/GPU intensity path. This is an ARTISTIC development envelope, not an empirical
significant-wave-height model. The calm column still carries remote long swells.
Fixed numeric/wind overrides, fixed replay endpoints and the optional cycle
bypass this weather-target work, preserving diagnostic controls.

Limitations: a single camera-region intensity still applies to all visible water;
coarse resident source data and gaps between fetch taps can miss narrow islands.
The 100km cap and 25km development scale are bounded presentation choices, not
physical calibration. Wind direction changes sheltering, NOT the wave axes or
propagation signs. Continuous directional-spectrum evolution is still next.
No shader work, GPU wave, pass or geometry was added; CPU fetch cost has not been
measured in a matched performance run, so there is no FPS/zero-cost claim.

Validation before the concurrent bird commits:
- 564 workspace tests passed, 23 ignored; fmt/clippy/diff check passed.
- Four new tests cover vector interpolation and fetch/target behaviour. A
  synthetic coast gives about 12km fetch in one wind direction and the 100km
  cap with the opposite wind. Missing data is rejected, not filled as water.
- Quadro Vulkan/Xvfb `ocean_weather_trial/1789364696-508557` passes with three
  captures, 300 weather targets and no unavailable-fetch samples. Actual wind
  is 7.3166–8.0366m/s; estimated fetch is 68,338.8–83,334.0m. Target intensity
  is .228038–.258330 and the smoothed sea reaches .227167 at 600 seconds,
  compared with .038131 in the preceding storm-only replay. Capture-003 was
  visually inspected; this is state coverage, not continuous motion sign-off.
- Fixed storm `ocean_wind_trial/1789364729-508749` passes and BOTH captures are
  pixel-identical to the preceding `1789356639-504381` control.

Concurrent bird commits `d27fd0d` and `77357ba` arrived during completion. Their
changes are preserved; the ocean patch does not modify birds or terrain rendering.
`crates.tar.gz` remains untouched/untracked.

## Fetch bug fix — 15 September

**The bug:** When the camera was on dry land (baker's landing site, F4 inspection
mode), `upwind_fetch_meters` checked the origin's height and found dry ground,
returning 0m of fetch even though the sea was visible far upwind. This made storms
look calm from the shore.

**The fix:** The function now distinguishes two cases. If the eye itself is in
water, it measures fetch normally from the origin. If the eye is on dry land, it
finds the shoreline upwind and measures fetch from there. A two-stage search
(coarse 64-sample scan, then bisection) locates the shoreline with at most 77
height queries, unchanged from before.

**The change:** one conditional check, two loops, one bisection loop; all prior
logic preserved for the eye-in-water case. The formula for sea-state scaling
(`max(clamp(wind_speed/30), storm_intensity) * (1-exp(-fetch/25000))`) and
shelter logic remain unchanged.

**Validation:**
- Two regression tests pass: (1) dry beach with 12km of open water upwind reads
  ~12km fetch (was 0m); (2) stepping into the water doesn't jump the fetch
  (continuity check).
- Scenario `ocean_weather_dry_landing` registered and loads; it confirms the
  baker's site scenario runs without errors.
- 507 workspace tests pass, clippy clean, release rebuilt. Scenario count
  updated from 96 to 97.

**Not changed:** Codex's existing wind/fetch estimation, sea-state response path,
GPU wave propagation, or any other ocean rendering. This is a single-line bug fix
with full test coverage.

## Peak visual judging: rock on steep faces, cliff skylight, cast shadows — 15 September 2026

`peak_survey_8_directions` places the eye along local up (`position.normalize()`, never world +Y)
100m over the drawn summit, pitched 30 degrees down, and holds each of eight headings long enough
to stream before its capture. Sun at equinox solar noon (64 degrees, due north). Evidence,
reference photos, judge summaries and notes: `test-runs/peak_judging_2026-09-15/`.

What the measurements showed, in the order they overturned my guesses:
- **Weather snow was not the white.** Raw weather snow cover is below 0.5 on every pixel here.
  The white is biome snow: Ice (2) and MountainSnow (9) forced snow regardless of slope, and
  rock only reduced snow by 35%.
- **Shedding snow exposed white.** Those biomes' palette colour *is* snow, so the exposed base
  must be mountain rock. `snow_slope_hold()` now sheds 30 -> 45 degrees (ice 35 -> 50) in the
  biome colour, the textured weights and both weather-snow sites, and snow biomes expose rock.
- **Brightness thresholds hid the change.** Steep faces were already mid-grey; judge changes with
  per-pixel diffs split by a slope mask, not dark/bright counts.
- **Cliffs are pale from direct sun, not haze.** Steep pixels: albedo 80-122, surface lighting
  143-174, aerial contribution 0-18. The sky view factor moved lighting ~1 level.
- **Raster had no planet-wide height.** Cast shadows reuse `FoveatedRenderer::shadow_height_faces()`
  (R32Float face array, 1-texel gutter) as shared binding 15, with `face_quads` in
  `TerrainSettings.outmap_detail.y`. The moon flight binds a 1x1 fallback with zero quads, which
  switches shadows off. Steep-face lighting fell 5-9 levels on N/NE/E/W/NW; judges read the
  coarse shadows as grey blobs.

Round scores: 2.3 then 2.5/10. All three judges still rank exposed rock/relief shape, stretched
cliff texture, pixel-edged lowland patches, a repeating snow pattern and stars at noon.

## Lowland orange patches were aerial neutrality, not a material — 15 September 2026

The judges' "flat pastel orange patches with pixel-step edges" over the peak's lowlands measured, in
the same pixels: raw albedo white (216,221,223), surface lighting white (232,235,237), final orange
(207,177,138) beside neutral ground (187,187,191) of equal luminance. The long-path transmittance is
blue-depleted; `terrain_material_transmittance/in_scatter` grey it for snow, but keyed on the
nearest biome ID, so white ground in a non-snow biome kept the orange and took that biome's texel
staircase as its outline. Neutrality is now `max(blended snow share, albedo_snow_look(albedo))`,
where the look is bright and low-chroma, so sand and desert keep their warm path. Orange pixels per
heading fell to 0-9 (S 10,123 -> 3). A luminance-only diff reads 0.00% for this change: judge hue
fixes by colour, not by brightness.

Also: lake coverage, the ice light floor and snow lighting use blended biome shares, and
`organic_biome_blend()` reweights the four corners by per-biome world noise so shared edges curve
instead of stepping. Two beach-sand gates moved to blended shares too, but forcing shoreline sand
off changed 0.00% of these frames, so they are consistency fixes. Three guesses were wrong before
the debug modes settled it (biome gates, beach sand, shoreline sand); split a colour by
albedo/lighting/aerial modes before editing materials. Evidence: `test-runs/peak_judging_2026-09-15/lowland_edges/`.

## Remote play, a longer day, and birds at 2.1m — 16 September 2026

Three small changes, each measured:

- **Movement keys survive key repeat.** Over Sunshine/Moonlight a held key arrives as repeating
  press/release pairs, not one long press, so clearing the key on its release left WASD reading as
  unpressed at almost every ~78ms frame while F-keys and Escape worked (they act on the press).
  `MOVEMENT_KEY_LATCH` holds a movement key 120ms past its last press; repeats refresh it.
  Confirmed by Ian over Moonlight from Android, which is now his remote test path.
- **A quarter-speed spin.** `INTERACTIVE_DAY_REAL_SECONDS` 1,200 -> 4,800. Weather derives its day
  from the rotation, so it still runs exactly one weather day per rotation and therefore advances
  four times slower in real time: a 600s weather step every 33.3 real seconds rather than 8.3.
- **Birds five times bigger.** Drawn body 0.42 -> 2.1m. The mesh straddles the position the
  simulation carries, so foot clearance (0.10 -> 0.50m), the touchdown band (0.35 -> 1.75m) and the
  landing aim height (0.2 -> 1.0m) scale with it or a seated bird sinks. Ground separation went to
  3.0m, **not** the proportional 3.75m: at 3.75m `flocks_that_cannot_merge_keep_out_of_each_other`
  fell to 4.10m against its 25m floor, because a five-times-wider settled flock overlaps its
  neighbour. At 3.0m the closest approach measures 39.66m, above the 32.61m the test's own comment
  records. Flocking radii, startle distance, flight floor and draw distance are deliberately
  unchanged; the bird cam offsets scale with the body.

Clippy (all targets) clean and 580 workspace tests pass at each of these. `bird_demo` capture
`1789566076-230794` shows the new size with the rescaled chase framing.

## Cheaper frames, M to watch a flock, and a failed rock experiment — 16 September 2026

- **47% off the frame time.** The cast-shadow march (+8.6ms) and the biome-edge noise (+28ms) are
  gone: 77.22 -> 41.47ms median on `peak_survey_8_directions`. Judges had read the coarse shadows as
  grey decals, so both cost and picture improved. Round 5 scored 3.17/10, the best so far, against
  2.5 the round before.
- **M holds position and tracks the nearest flock**, so a flock can be watched going past rather
  than chased. `flight_look_angles_toward` inverts `flight_view_direction`; any mouse look cancels
  the tracking.
- **Shaded snow takes the sky's colour** (neutrality 0.82 -> 0.55).
- **Rock on wind-scoured rises: tried four ways, reverted.** `relief` is <=0.01 on 91-99.9% of
  terrain pixels at this range and never reaches 0.55, so the window never opens; and because a snow
  biome's palette is white, shedding snow without substituting the rock palette only reveals more
  white. Measured no change every time. What the judges keep asking for -- dark fractured rock with
  snow in the gullies -- needs real rock material and distribution, or higher-frequency geology.

Remaining judged faults, by how often they are named: exposed rock, cast shadows, aerial perspective
grading distant ranges blue, the repeating leopard-spot snow tile, faceted/low-poly silhouettes and
sawtooth ridges, staircase shorelines, the flat lake, and no clouds.

## A planet with rock on it — 16 September 2026

The judges' unanimous complaint across five rounds was missing rock. It was never a renderer fault:
the bake had almost none. `classify_biomes` tested height before anything else, so with relief
reaching 72,000m every mountain passed the snowline test and became Ice, leaving MountainRock as the
*lowest* mountain band, 2,400m to roughly the snowline. Measured from the summit survey, the view
was 74-95% biome 2 with rock under 1%.

The fix adds steepness as an input ahead of the height tests: ice and snow do not cling to a steep
face. `STEEP_ROCK_SLOPE_RADIANS` is 4.8 degrees, which is the *measured median slope of land above
3,000m on the classifier's own field* (neighbour spacing 6,136m; p50 4.76, p75 7.92, p90 12.74).
Two earlier guesses failed and are worth recording: 35 degrees exceeded the steepest sample on the
planet and produced a bake identical to three decimals, and 8 degrees moved high-altitude rock only
4.06% -> 5.69%.

Measuring it needs care, because cell counts and area disagree sharply on an equirectangular grid:
the classifier reports 43% of high *cells* taking the steep-rock branch, the exported cube tiles
report 7.19% of high *samples*, and the area-weighted preview - the honest figure - says rock is
12.46% of the surface. Use `previews/biome.png` with a cos(latitude) weight for planet-wide shares.

Also found on the way: 21% of land above 3,000m is lake, which is why three attempts to site a
mountain survey landed in high lake basins. `mountain_survey_sites` now requires the tile's *centre*
sample to be rock or mountain snow and rejects tiles more than 10% water; on this bake only two
tiles pass, and both are gentle snow plateaux, so a rock-and-crag survey site still does not exist.

Frame time is unchanged by the rebake: old planet 61.6/62.4ms against new 62.2/64.4/63.1ms on the
same binary. Note for a later session that this is itself ~20ms worse than the 41.5ms measured at
round 6 on the same scenario, so something between those commits cost time and has not been found.

## Glacier labels erased by export, not resampling — 19 September 2026

Started from `6e81634` with Claude's uncommitted glacier work and his **third bake live**.
Claude committed the foundation as `3eb263c` during validation; the exporter correction and
failing-before regression are committed/pushed separately as `e8248bf`.

`sample_tile` sampled the new IDs correctly, then `baked_biome_detail` reapplied its altitude
snowline at L3+ and replaced them with Ice. The few survivors came from parent borders:
L3/L4 interiors contained **zero** moraine or crevasse samples. The fix exempts the two glacier
IDs alongside Ice. No classifier widening or dilation was needed. The regression exercises
real tile sampling and parent constraints at L2/L3/L4/L18; it failed at moraine L3 before the fix.

The freshly rebuilt procedural baker produced and validated all 3,252 tiles. Every height
and moisture payload, all three previews, and the full manifest match the third bake exactly.
L4 moraine/crevasse counts rise from **368/5,642 to 26,426/402,317**. After matched captures,
the corrected export was promoted to `assets/outmaps/test-planet`; the third bake remains at
`assets/outmaps/test-planet.claude-third-backup-20260919`. The staging directory was renamed,
not left as a second candidate. To reproduce this planet, use the procedural command in the
results report below, **not the older ETOPO recipe elsewhere in this historical log**.

Same binary, Quadro M1000M, Immediate present, raster, 1280x720, 69 logged frame times per run:

| Scenario | Before median ms | After median ms |
|---|---:|---:|
| Alpine pair 1 | 88.547 | 88.235 |
| Alpine pair 2 (after ran first) | 88.177 | 88.688 |
| Summit control | 70.372 | 70.251 |

Mean alpine run medians **88.362 -> 88.461ms (+0.11%)**; opposite-signed pair differences,
not evidence of an FPS improvement. All six comparison replays pass; repeated captures
are identical within each bake. The corrected alpine capture is
`alpine_survey_8_directions/1789816935-538761`. Compared both with a fresh baseline and
round 12: N/NE/E are byte-identical; SE/S/SW/W/NW change 1,002/28,332/19,040/15,783/567 pixels,
maximum channel deltas 12/32/32/10/1. Inspected contact sheets and full-resolution south views.

**This fixes the data path, not glacier realism.** The new labels make distant tonal patches,
not actual crevasses, seracs or convincing medial moraines. No judge round was spent; 2.7/10
remains the previous score, not a new measurement. Next trace restored labels through material,
lighting and final pixels, and implement resolved glacier structure rather than widening
classes or expecting palettes alone to create cracks. Keep the approved Aletsch photographs
and judging camera unchanged; judges remain independent Luna observers, not diagnosticians.

Full evidence and exact bake command:
`test-runs/peak_judging_2026-09-15/glacier-export-fix/RESULTS.md`.

Claude's follow-up in `2243b70` identifies the next resolution limit: dense L4 material
samples are kilometres apart, while the judging foreground spans hundreds of metres to a
few kilometres. Treat the glacier IDs as regional masks, not resolved moraine stripes or
cracks. Procedural sub-cell material structure is a candidate next step; **no extra texture
reads does not mean free**. Measure its frame cost and actual visible effect before retaining it.

Final validation: 584 workspace tests pass (26 ignored), clippy all targets passes, and
changed-Rust-file formatting/diff checks pass. Unrelated pre-existing app formatting still
fails the workspace-wide fmt check and was left alone. Installed-path replay
`alpine_survey_8_directions/1789817879-542459` passes with all eight captures identical to the
staged result; its timing is excluded because it overlapped the final CPU tests.


## 20 September — procedural glacier trial rejected for promotion

Tested in detached `ce5631f` checkout `tmp/glacier-detail-worktree`, with its own
`target-glacier-detail` build directory, not the concurrently changing village renderer.
The corrected live bake, judging camera and approved references were held fixed.
Only the isolated `planet.wgsl` and `terrain.rs` were modified; neither change was
promoted into the main source tree.

Dedicated moraine/crevasse regions are distant in these views; their metre-scale
marks filter away. V1 off/on captures are pixel-identical in all eight headings.
V2 applies faint fractures to non-polar Ice too and gates the downstream ice-light
floor by mark coverage, otherwise lighting whitens the new marks. This changes
15,736–39,039 pixels per view, at maximum channel deltas 11–14/255. All eight full
frames were inspected: the close marks look like faint painted, regularly spaced
lines, not broken glacier ice. Fixed global orientation does not follow glacier flow.
**Do not promote this candidate or spend a judging round on it.** This is a rejected
visual hypothesis, not a new realism score or completion of procedural glacier work.

Same V2 binary, Quadro Immediate-present alpine replay, 69 logged frame times per
run: off `1789874552-723308` median **88.109ms**, on `1789874739-723811`
**90.194ms (+2.37%)**. Only one pair: not a statistically established slowdown,
but certainly not evidence that arithmetic without extra texture reads is free.
Both replay assertions pass. All eight default-off images also match V1 default-off
pixels exactly. Focused shader validation passes all three modes; V2 release build
passes. No final V2 full-workspace or motion acceptance claim.

Reproducible rejected patches, checksums, run IDs and pixel differences:
`test-runs/peak_judging_2026-09-15/glacier-detail-trial/RESULTS.md`.
Next design work needs a terrain/flow-aligned glacier coordinate field and irregular,
resolved fracture structure, rather than darkening these fixed-axis lines or widening
biome patches. That is an untested direction, not an approved implementation plan.
The main executable and bake remain unchanged; concurrent village edits are preserved.


## 20 September — fracture density/normal follow-up banked, not promoted

Source commit `f10090b` is pushed on separate branch
`experiment/glacier-fracture-material` (base `ce5631f`), in isolated checkout
`tmp/glacier-detail-worktree`, build target `target-glacier-detail`.
Do not copy over current village changes; the main source and live bake remain
unchanged. The trial is still opt-in/default-off on its own branch.

Finite segments replace continuous stripes; sparse share controls placement,
not every crack's opacity. Rounded wall normals provide bounded bump shading,
with early exits outside marks and no unmarked-surface relighting. Actual-WGSL
GPU probes pass and deliberately restoring the old opacity rule fails at 0.3
maximum coverage. 525 app tests pass (24 ignored), release build/shader modes
pass. Strict all-target clippy hits the existing base village constant assertion;
no unrelated source was changed to silence it.

Four V6 alpine replays pass; repeated images are identical, and all eight
default-off frames match V2 controls. The new marks visibly change 596–6,001
pixels per frame (maximum channel delta 26–200), but do not supply convincing
glacier structure. Three independent fresh-context Luna reviewers, each with
five approved Aletsch photos and all eight game frames, score **2.5, 3, 2**,
mean **2.5/10**. No overall realism improvement is established. Their observations
remain visual evidence, not technical diagnoses.

**All four V6 timing samples are excluded:** a process monitor confirms concurrent
main-checkout renderer/build activity during every run. Do not quote their raw
medians as a slowdown or speedup. Flow-aligned coordinates, coherent glacier
structure, motion/LOD acceptance and uncontaminated performance remain undone.
No promotion or new bake. Evidence and next-design boundary:
`test-runs/peak_judging_2026-09-15/glacier-detail-trial/FOLLOWUP.md`.

## 20 September — village siting stopped following the camera

Villages were a function of the camera, not of the planet. With the camera's
ground position held fixed and only its altitude changed, the deterministic
probe `village_altitude_stability` drew **83 houses at 150m and 500m and 35 at
1500m and 4000m — a different settlement each time**, one appearing exactly
where the other vanished.

The cause was not the height maths. `outmap_terrain_height_scale` ignores its
altitude argument, so the heights themselves are altitude-free. The dependence
came in through residency: `forest_surface_sample_at` resolves the finest tile
that happens to be streamed, and the LOD selector streams L12 near the ground
and falls back to L0 ancestors higher up. Every placement test — habitable
biome, positive land, footprint flatness, dryness — was reading whichever tile
the camera had caused to be loaded.

Siting now reads the globally dense level, which is the only level complete
everywhere, through three new pinned samplers in `terrain.rs`
(`dense_level_surface_sample_at`, `dense_level_open_ocean_at`, and the private
height stencil behind them). Those tiles come from a dedicated CPU-only cache,
`village_siting_tiles`: the streaming cache cannot serve them, because at low
flight it holds L12 and at orbit L0, and an L4 tile is frequently in neither.
`prepare_village_siting_tiles` holds the camera's dense tile and its ring — nine
tiles, about 0.9MB — and reloads only when the camera leaves that set, which at
390km per tile is not a per-frame cost.

Two further faults surfaced while fixing it, both of which predate this work and
were masked by the instability:

- **The siting height must be baked macro geography with no runtime detail
  ladder.** The ladder's amplitude scales with the sample spacing it is filtered
  against; at the dense level's ~3km spacing it returned **2,213m for ground that
  renders at 174m**, and a house placed there floated two kilometres up.
- **`raster_surface_height_breakdown_at_distance` is the wrong query for
  placement.** It answers with the *highest* surface drawn at a direction,
  because flight clearance must not miss one, and a coarse ancestor patch's
  highest surface can be a mountain nowhere near the house — measured at
  **1,861m** of lift. Houses now take their drawn height from the finest
  resident sample, the same query the trees use, bounded by
  `HOUSE_GROUND_DISAGREEMENT_LIMIT_METERS` (the ladder's own amplitude bound
  plus the global detail term) so another patch's terrain cannot be believed.

The separation that makes this measurable is `sited_houses` against drawn
houses. Siting must not move with the camera; how many sited houses are near
enough to draw is allowed to, and is what the 8km cutoff decides. The new
opt-in assertion `require_constant_village_siting` checks the first, ignoring
the leading empty sample that every run has because the spatial record is
written earlier in the frame than the village rebuild.

**Verification.** `village_altitude_stability` settles on **117 sited houses
with 0 disagreements** across 150m/500m/1500m/4000m. Reverting only the siting
sampler to the old camera-dependent call and re-running fails the same
assertion at **319 sited houses, 8 disagreements**, so the test bites. Re-aimed
`village_pov` passes and its second capture shows the houses standing on the
ground. 528 app tests, 601 workspace tests, fmt and clippy pass; the one clippy
warning is the pre-existing constant assertion at `village.rs:890`, untouched.

**Not done.** The worst legitimate gap between a house's drawn ground and its
sited ground is **513.5m** in `village_pov` — real detail-ladder displacement,
not a float, but it means the footprint flatness test judges a surface that is
not exactly the one drawn, so a village on rough ground can still be uneven.
The captured village also stands on bare sand, which is a question about what
`village_biome_is_habitable` admits at the coarse dense level, not about
stability.

## 20 September — village slope limit, and how often you actually meet one

Answering "fly low and will there be villages often?" turned up a regression in
the siting fix above. Villages never tested slope; the footprint spread test was
doing that job, and moving siting to the dense level took its teeth out.

The footprint spans 110m against a **3,068m** dense sample spacing, so all five
samples land inside one bilinear cell and their spread is at most 7% of that
cell's corner-to-corner height difference. Measured against the real bake over
all 1,536 dense L4 tiles, area-weighted: the 30m spread test rejected **0.07%
of habitable land**. That is not a test.

`village_max_site_slope_radians()` is the same budget restated as a grade --
`atan(30m / 220m)`, 7.77 degrees -- which the coarse data can still answer, and
`village_surface_is_eligible` now applies it. Two regressions pin it: that the
slope limit and the footprint budget agree, and that steep ground is rejected
however flat the footprint reads. Sited houses at the probe fall **117 to 80**
and siting stays stable at 0 disagreements.

**Measured answer to the question.** Area-weighted over the whole bake:

| | share of surface |
|---|---|
| ocean + lake | 55.5% |
| ice | 17.6% |
| village-habitable (tundra, temperate forest, temperate grassland, tropical forest) | **20.3%** |
| mountain rock/snow, crevasse, moraine, desert | 6.6% |

Habitable biomes are 45.6% of land. Macro slope over that land is p50 3.8, p90
12.9, p99 22.2 degrees, so the 7.77-degree limit keeps about three quarters of
it. With two candidate sites per 6.14km cell that is roughly **11 villages per
1,000 km2, a mean spacing near 10km**, and about **2 villages inside the 8km
draw radius** averaged over the whole planet -- one to three in view over
habitable ground, none at all over ocean, ice or high mountains. The drawn
counts agree: 20-57 houses in view at the probe, at up to 20 houses a village.

**Note on method.** A first attempt flew a 400km transect at a fixed 600m datum
altitude and read zero villages for 315km. That was not terrain data: the path
went through ground standing at 11,600m and then 37,428m, so the camera was
inside a mountain range. A fixed-altitude transect is not a fair sample on this
bake, and the scenario was removed rather than left to mislead. The figures
above are from the baked tiles directly.

**Still open.** The slope limit reads the 3km macro grade, so a site that is
gentle at that scale can still be locally rough at the metre scale the camera
sees -- the worst drawn-versus-sited ground gap remains 513.5m of detail-ladder
displacement. Villages also site on coarse L4 biome, so a cell the fine shader
paints as beach can hold one; the `village_pov` capture is a village on sand.

## 20 September — contact shadows, locator beams, and a village glued to the camera

Three changes, one of them a bug the other two exposed.

**Villages drifted with the camera.** The instance buffer holds offsets *from
the camera*, and it was only rewritten when the camera had moved past the 400m
resiting threshold. Between rebuilds the whole village translated with the eye:
a capture 24m from a house was indistinguishable from one 150m away, both
showing the village the same size in the middle distance, and then it snapped
back at 400m. `rewrite_camera_relative_positions` now re-differences the built
houses every frame against the camera's current position, from a kept list of
world positions. Which houses exist is still recomputed only on a real rebuild.
This was pre-existing and it made everything else unjudgeable.

**Ground contact shadows.** One alpha-blended fan per house, lying in the
house's own tangent plane, no depth write, nothing sampled and nothing traced.
It is an ellipse matched to the footprint rather than a disc, because a
circular blot under a rectangular building is what makes cheap contact shadows
look like stickers. The fade start is derived from the fan's spread so that
full strength lands exactly at the eave line: choosing it by hand put the whole
falloff under the house where nothing can see it, and the visible strip reached
only 30 levels instead of 56. Matched captures with the strength set to zero
measure **2,510 pixels darkened, 56 levels at most, 19.9 on average**.

**Locator beams on `V`.** The forest beams were removed in `9954f9c` when the
bird camera took `B`; this is the same idea for villages, rebuilt on the
village pass rather than restored. One shaft per village, not per house, and
emitted from the *sited* set rather than the drawn one, so settlements too far
away to draw still show. Width is applied in NDC, so a beam stays the same
thickness whether its village is 200m or 2,000km away. Off by default, with
`CATINGARDEN_VILLAGE_BEAMS=1` for captures.

**Verification.** 533 app tests, 606 workspace, fmt and clippy pass; the one
clippy warning is the pre-existing constant assertion at `village.rs:890`.
Beams: `village_altitude_stability` and `village_pov` captured with the
override on show shafts standing on every village in view.

**Not measured.** The village pass's frame cost is *not* resolved here. Four
interleaved on/off pairs on `village_pov` gave +4.14, +11.34, +2.41 and
-3.13ms while the baseline itself drifted from 119ms to 76ms across the run,
so the sign is not even consistent. An earlier paired run on
`village_altitude_stability` put it at about 0.5ms with overlapping ranges.
Neither is a number to quote; a clean measurement needs an idle machine.

## 20 September — `--profile-render` works again, and what still does not

**Resolution.** `CATINGARDEN_VIEWPORT=WIDTHxHEIGHT` sets the internal render
size, default 1280x720, minimum 64 per axis. Fullscreen keeps whatever that was:
`toggle_fullscreen` pins `fullscreen_render_size` to the current internal size
and only the presented surface grows to the monitor, so the env var is the
fullscreen resolution control too. Verified: 1920x1080 and the 1280x720 default
both come out at those sizes in a capture.

**The profiler was welded to a feature that breaks this GPU.** `--profile-render`
requested `TIMESTAMP_QUERY | TIMESTAMP_QUERY_INSIDE_PASSES`, and on the Quadro
M1000M (550.163.01) asking for timestamps breaks the device. Three findings
sharpen July's diagnosis, which blamed `present()`:

- `TIMESTAMP_QUERY_INSIDE_PASSES` was requested and never used -- every
  timestamp is written at a pass boundary through `timestamp_writes`. Dropping
  it changes nothing and the hang reproduces, so it is the base feature.
- The hang is not in rendering. With presentation removed the scenario runs to
  completion; a backtrace shows the process wedged in
  `NativeSwapchain::release_resources` under `libnvidia-glcore` while *dropping
  the surface at exit*.
- The device is broken regardless: with timestamps on, every capture readback
  times out at 5,000ms, `gpu_render_ms` is -1 on every frame and not one
  timestamp resolves. That is the real fault; present and teardown are
  downstream of it.

The CPU-side breakdown had no reason to be behind the same switch, so it is not
any more. `--profile-render` now asks for no device features and works here;
`--profile-gpu` asks for the timestamps deliberately and still fails as above.
A profiling run draws into an offscreen stand-in of the surface's own format and
never presents, which is what let the run reach the end; captures copy out of it
exactly as they did out of the swapchain.

**What a report looks like now.** `still_5s` at default resolution, medians over
11 samples: `total_render` 20.803ms, of which `present` is 18.705ms, `simulation`
1.508ms, `submit` 0.136ms, `vertex_upload` 0.061ms and everything else under
0.03ms. That is the shape of a GPU-bound frame -- the CPU is asleep in `present`
waiting for it -- and it is also the limit of what this tells you. **Per-subsystem
GPU attribution is still not available on this machine**: the stage split
(scene / luminance / sun / blur / bloom / tone-map / egui) needs the timestamps.
The working alternative is matched A/B toggling, which is how the forest, road
and village costs were measured, and it needs an idle machine to beat the
run-to-run drift seen today.

## 20 September — where the frame actually goes

`CATINGARDEN_DISABLE=a,b,c` skips any named scene draw. It exists because
per-subsystem GPU attribution needs either timestamps, which break this card,
or matched A/B runs, and the toggles that existed covered a few systems under
inconsistent names. Skipping a system leaves the frame wrong on purpose.

Full report and raw data: `test-runs/render_profile_2026-09-20/`.

At ground level in `village_pov` at 720p, of an **83.6ms** frame: **30.5ms is a
fixed floor with nothing drawn at all**, **51.3ms is terrain**, and **1.9ms is
every other subsystem put together**. Terrain draws alone in 81.7ms; ocean, sky,
stars, clouds, rain, forest, villages, birds and ship add 1.9ms on top of it
between them, and individually every one is at or under the +/-1ms noise floor.
Terrain is **97% of the scene work**.

Two things worth carrying forward. The 30.5ms floor -- post, exposure metering,
present, simulation -- is 36% of the frame before anything is drawn, and is a
target independent of terrain. And forest measures *consistently* negative
across four blocks (-1.08/-0.81/-0.32/-0.65), which is systematic: the likely
reading is that trees occlude terrain and terrain is what costs, so removing
them exposes more expensive pixels than the trees cost to draw. Untested.

Inside terrain, a resolution sweep says fragment-bound -- 52.1ms at full pixels
against 4.5ms at a sixteenth. Fitting `cost = a*pixels + b*triangles` to that
pair gives 50.8ms fragment against 1.3ms geometry, and **the model then fails**:
it predicts a quarter-size chunk budget at 51.1ms and the measurement is 19.0ms.
Chunk count scales fragment work, not just vertex work. **Overdraw** fits both
results and is the next thing to test -- with a fragment count, not a frame time.

**Method, because an earlier attempt was wrong.** Two warm-up runs are discarded
(the GPU idles at 135MHz and boosts to 1124MHz, which is what made this morning's
numbers drift 119ms to 76ms), then every condition runs once per block so drift
hits them all alike. Baseline held to 1.1% across four blocks. A first attempt
is absent from the data because two sweeps were started concurrently and
measured each other; the tell was a no-terrain frame coming out 57ms *slower*
than baseline, which is impossible. One sweep at a time.

**Limits.** One camera pose. Terrain's dominance will differ at orbit, where the
frontier is a few coarse chunks, and over open ocean. Nothing here attributes
cost inside a pass.

## 20 September — the overdraw hypothesis is dead; terrain is vertex-bound

The profile above suggested overdraw and a depth prepass. **That was wrong.**

Overdraw is fragment work, so the cost of extra chunks would have to shrink with
the pixel count. Running chunk budgets 64 and 1024 at both 1280x720 and 320x180:

| condition | frame ms | spread | triangles |
|---|---:|---:|---:|
| 720p, budget 64 | 49.41 | 1.05 | 147,456 |
| 720p, budget 1024 | 180.93 | 0.49 | 2,359,296 |
| 180p, budget 64 | 14.86 | 0.07 | 145,152 |
| 180p, budget 1024 | 103.75 | 0.88 | 2,355,840 |

The chunk-budget spread is **131.5ms at 720p and 88.9ms at a sixteenth of the
pixels** -- ratio 0.676, where overdraw demands 0.0625. It survives the pixel cut
nearly intact. **Chunk count costs geometry, and a depth prepass would not help.**

The rate is the interesting part: 40.2ns per triangle at 180p, 59.5ns at 720p,
about **25M triangles a second**. An M1000M does not struggle to push 2.4M plain
triangles, so that is the *terrain vertex shader*, not fixed-function throughput.
The difference between the two rates puts roughly a third of the chunk-scaling
cost on fragments and the rest per-vertex.

**Next measurement, not next fix.** The terrain vertex shader does a height
sample, four more for central-difference normals, geomorph blending and
per-vertex aerial perspective, on a canonical 33x33 grid that is 2,304 triangles
per chunk at *every* LOD. Ablate those terms one at a time with the same
`CATINGARDEN_DISABLE`-style harness before changing anything. The candidate
fixes are a coarser grid for distant chunks, cheaper normals, and moving aerial
perspective off the vertex, but which one is worth doing is not yet measured.

## 20 September — every terrain shader term priced, and the two cheapest wins taken

`CATINGARDEN_ABLATE=normals,detail,aerial,fog,skylight,material,tint,weather`
compiles named terms out of the terrain shader. Each was confirmed to change the
rendered image before being timed: an ablation that silently does nothing reads
as "this term is free", which is the most expensive mistake available here.

Of terrain's 52.1ms at ground level in `village_pov` at 720p:

| term | stage | ms |
|---|---|---:|
| detail ladder | vertex | 8.05 |
| material detail tint | fragment | 7.80 |
| cloud shadow | fragment | 4.08 |
| aerial perspective | vertex | 2.49 |
| central-difference normals | vertex | 2.09 |
| weather wetness | fragment | 1.80 |
| fog | vertex | 1.69 |
| sky irradiance | fragment | 1.25 |
| material colour (4-way triplanar) | fragment | 0.61 |

That is 29.17ms of 52.1ms named; about **23ms is still unattributed** — base
height sampling, vertex attribute fetch, per-draw overhead across 256 chunks, or
fragment work none of these switches reach. Dividing it is the next measurement.

Two results are worth more than the table. The four-way triplanar material
lookup is **nearly free at 0.61ms**, while the detail tint layered on top of it
is the second most expensive term in the whole shader at 7.80ms — the
expectation was the other way round. And the cloud shadow costs 4.08ms while
changing **zero pixels under clear sky**: it runs the shell projection, gets
full visibility, and multiplies by one.

Raw data and method: `test-runs/render_profile_2026-09-20/`.

### Cloud shadow is temporarily off

At Ian's request, `crates/app/src/planet.rs::cloud_shadow_enabled` now returns
false by default and emits `TERRAIN_CLOUD_SHADOW_ENABLED` into both the terrain
and forest shaders. **One constant for both** is load-bearing: the trees have
their own `cloud_shadow_visibility` call, so disabling only terrain would shadow
a tree standing on unshadowed ground. `CATINGARDEN_CLOUD_SHADOW=1` restores it
with no rebuild; making it permanent is a one-word change in that function,
which carries a comment saying whose request it was and when.

It is verified in both directions, because "no pixels changed" is equally
consistent with the switch not working:

- **It does change shading under actual cloud.** Three `weather_contrast`
  captures, off against on: 11.50%, 22.00% and 26.34% of pixels differ, maximum
  channel delta 12, 21 and 55 levels, and mean luminance falls when it is on, as
  a shadow must.
- **It saves 3.76ms.** Four interleaved off/on blocks of `village_pov` after two
  discarded warm-ups: off 81.46/79.95/81.40/80.47ms, on 84.93/83.99/83.85/85.30ms.
  Paired savings 3.47/4.04/2.46/4.83ms, median 3.76ms, **4.45% of the frame**,
  and all four pairs agree in sign. Slightly under the 4.08ms ablation figure,
  which is expected: the ablation removed the call, this leaves a `false`
  constant for the compiler to fold.

### Interactive startup no longer turns blur on

`apply_interactive_startup_controls` dropped its `toggle_blur()`, so an ordinary
launch keeps the `BLUR_ENABLED = false` default and shows the unfiltered scene.
F6 still toggles it and scenarios, which set their own post state, are
unaffected. `interactive_startup_does_not_enable_blur` reads the function's own
source and fails if the call comes back — mutation-checked by restoring the line.

**Village locator beams are on `V`**, or `CATINGARDEN_VILLAGE_BEAMS=1` at launch
for captures. They are drawn from the *sited* set, not the drawn one, so
settlements past the 8km house-draw cutoff still show; the level-10 search ring
covers roughly 12km around the camera. `V` is now listed in the HUD control line.

534 app tests and 607 workspace tests pass, clippy and fmt are clean.

## 20 September — clean alpine material probe results

The V4 same-binary control/snow-range/combined/control block measures
84.993 / 82.941 / 79.775 / 86.600ms, without observed concurrent app/build
processes. Both controls are pixel-identical in all eight views. Full app tests
pass: 525 passed, 23 ignored; all ten diagnostic shader modes validate.
The range-only images lose fine grain but retain the large artificial patches
and smeared rock. Combined darker lighting is not an accepted realism gain.
No new judges, no new score, no default promotion; live renderer/bake unchanged.
Evidence: `test-runs/peak_judging_2026-09-15/alpine-material-trial/RESULTS.md`
in the active checkout. Earlier contaminated timings remain excluded. These
are single candidate runs bracketed by controls, not statistical sign-off.

## 20 September — round 13: the alpine frame has no fill light

Resuming the judging cycle at HEAD. Eight headings of `alpine_survey_8_directions`,
raster, 1280x720, median 80.45ms. **No judges were spent**, because nothing
measured here changed the picture, and round 9 already established that judging
a no-op wastes a round.

### The gap

One metric, run over the lower 65% of the frame on both sides
(`test-runs/peak_judging_2026-09-15/round13/measure.py`):

| | ground saturation | tonal spread |
|---|---:|---:|
| our eight captures | 0.017 | 0.499 |
| twelve Aletsch photographs | 0.176 | 0.898 |

Ten times less colour, half the tonal range, and our ground is crammed into the
top of it: p01 0.38, p50 0.89, p99 0.93. The photographs start at p01 0.005-0.21.
**We have no dark end.**

By stage: raw albedo is already only 0.054, and lighting plus the tone curve
remove two thirds of what survives that.

### There is no fill light, and that is not a bug

Ablating the sky-diffuse term moves the frame by at most **5 levels of 255**; in
the lighting stage it is 0.5-1.5 levels, **0.3-0.6% of surface light**. The hue
is right (+1.52 blue against +0.65 red) and the magnitude is absent.

Computing the LUT's own integral on the CPU with the shader's constants gives
E/pi luminance **0.0949 at sea level and 0.0126 at this surface**. The site is
79,247m up, which the 4.5x optical mapping puts at 17,610m against an 8,000m
Rayleigh scale height: 2.2 scale heights, **11% of sea-level air overhead**. The
sky there has almost nothing to give. Real Jungfraujoch is at 0.42 scale heights.
The altitude scaling rule is Ian's and correct; this is its consequence.

So every facet is lit by direct sun or by nothing, which is precisely the "one
brightness at every orientation" all three judges reported. And the LUT's
`GROUND_ALBEDO` is 0.12 while this ground is snow at 0.64 linear, so **snow
inter-reflection -- the light a real snowfield works by -- is modelled nowhere.**

Aerial perspective (2.7%) and distance mist (0.1%) were checked and cleared.

### Measured and not promoted

| candidate | ground sat | spread | frame ms |
|---|---:|---:|---:|
| baseline | 0.017 | 0.499 | 80.45 |
| one-bounce ground inter-reflection | 0.018 | 0.474 | 79.04 |
| bounce + no snow greying | 0.038 | 0.474 | -- |
| bounce + exposure 0.50 | 0.032 | 0.559 | -- |
| bounce + exposure 0.35 | 0.041 | 0.563 | -- |

The bounce has no tuned constant in it -- a Lambertian ground of albedo `a` under
irradiance `E` gives a facet `a*E*(1-sky_view)` -- and costs nothing measurable.
It is still a visual no-op and it *reduces* contrast, lifting shadow without
adding hue. `neutralize_snow_surface_lighting_blend` deliberately takes 55% of
the remaining hue so low sun cannot paint the icecap orange; removing it doubles
saturation to 0.038 and reads the same. Exposure moves both metrics honestly, the
snow being deep in the ACES shoulder at 1.0, but 0.041 against 0.176 is not a
different picture. All reverted. `CATINGARDEN_EXPOSURE` is kept as a diagnostic,
default 1.0, pinned by `the_fixed_presentation_exposure_defaults_to_one`.

### What the eye says, attributed by ablation

- The **smeared waxy swirls** across the foreground are the **detail ladder**
  (8.05ms). Off, the snow is clean and empty.
- The **leopard-spot dapple** on near snow is the **material tint** (7.80ms).
  Off, the mottling is gone.

The two most expensive terms in the terrain shader, 15.85ms of terrain's 52.1ms,
are producing the two surface artefacts the round 11 judges named. Codex reached
the same conclusion independently in `../alpine-material-trial`.

- The **dusty pink horizon band** is distant brown lowland: with the mist ablated
  it measures (127, 93, 65). The mist carries it to sky blue through a magenta
  midpoint, because green sits below both endpoints. That is the judges' "flat
  magenta cut-outs".

### Recommendation

Colour and lighting are not where the realism is, and this round is the evidence.
What is missing is structure and a dark end. Two routes, and the choice trades
against the performance rule, so it is Ian's:

1. **Cast shadows done cheaply.** The only thing that creates a dark end. The
   naive per-pixel march was dropped in `0076b4a` at +8.6ms with judges scoring
   the coarse 3km shadows *down* as "grey decals"; the amortised-cache or baked
   horizon-map route noted there is still open, and the cloud shadow just freed
   3.76ms.
2. **Spend the detail ladder and tint budget on structure instead.** 15.85ms
   currently buys the smear and the dapple.

535 app and 608 workspace tests pass, clippy and fmt clean.

## 21 September — ocean wind-sea tail axes spread

Picked the ocean thread back up on `experiment/ocean-wind-sea-spectrum` with a no-new-work shader
change: the 200m-to-7m wind-sea tail no longer repeats near-parallel axes across several octaves.
The long 1,400m swell group and 430/350/280m storm group are unchanged; only the short chop axes
are redistributed around the storm-ocean view direction, with the Rust CPU table and WGSL table kept
byte-mirrored. This is intended to reduce the corduroy/repeated-line character without adding waves,
samples, bind groups, branches, or texture work. It is not yet a scored visual win.

Validation: `CARGO_TARGET_DIR=/home/dad/catingard/target-ocean-spectrum cargo test -p
catinthegarden-app ocean --release` passes, 62 passed and 12 ignored; the actual-WGSL ocean normal
parity test `ocean::gpu_tests::gpu_ocean_normals_match_cpu_buoyancy_in_deep_and_breaking_water`
passes under the same target; release build passes; `ocean_wind_trial` release replay passes at
`test-runs/ocean_wind_trial/1789946830-288902` with two expected captures. Against an older
`ocean_wind_trial` capture, both frames change about 581k of 921.6k pixels with max channel delta
201-202, so the table change is visible. No matched FPS claim was made because the instruction count
is unchanged rather than timed.

## 21 September — crest transmission de-turquoised

Ian called the open-ocean crest transmission out as unrealistically turquoise: the 7 September
`ocean_hybrid_close/1788800036-415020` frames read as glowing tropical sheets, and reference open
water does not. The crest term is therefore retuned as a subtle edge accent instead of a water
colour: the old `vec3(0.025, 0.32, 0.22)` tint is replaced with a much weaker, less saturated
`OCEAN_CREST_TRANSMISSION_TINT = vec3(0.018, 0.045, 0.040)`, the p90/p99 sharpness anchors are
unchanged, the ramp is cubed instead of squared, and the backlight gate is tightened from `pow(...,
4)` to `pow(..., 8)`. Foam, specular, geometry, wave tables, buoyancy and seabed transmission are
unchanged.

Validation: `CARGO_TARGET_DIR=/home/dad/catingard/target-ocean-spectrum cargo test -p
catinthegarden-app --release ocean` passes, 62 passed and 12 ignored; the focused terrain guard
`ocean_shader_transmits_sunlight_and_retains_submerged_bathymetry` and ocean guard
`crest_transmission_tracks_the_sea_state_uniform` pass; release build passes; `ocean_hybrid_close`
release replay passes at `test-runs/ocean_hybrid_close/1789975619-330460` with four captures. The
reviewed first frame no longer has the broad turquoise sheet; remaining crest brightness is mostly
specular/foam. No FPS claim is made.

## 21 September — fine-crest transmission made visible

The first fine-ripple transmission pass was numerically present but failed its actual requirement:
Ian could not see it in the ordinary capture without pixel analysis. The cause was the lighting
composition, not the selector. The already-localized positive-height/steep-slope selector moved
about 1.4% of the image, but it shared the deliberately weak broad-crest tint which had been tuned
to disappear after exposure and tonemapping.

Broad crests retain that restrained `vec3(0.018, 0.045, 0.040)` tint. Fine crests now use a separate
`vec3(0.025, 0.160, 0.360)` blue-green radiance under the same fourth-power backlight, sunlight and
Fresnel gates. This changes no geometry, normal, depth, alpha, buoyancy, wave evaluation, texture
sample or draw. It adds one localized tint composition in the existing ocean fragment lighting.

The four-frame release replay `ocean_hybrid_close/1790029744-444379` passes and shows the cyan-green
light at normal size on small crests facing away from the low sun. Against the weak prior pass,
1.22-1.60% of pixels move by more than four levels and the largest RGB change is 9/61/68. This is
capture evidence, not yet Ian's visual acceptance. Three interleaved Immediate-present pairs have
mixed signs: 35.793→36.205ms, 35.722→35.334ms, and 35.627→35.798ms; pooled medians are
35.722→35.798ms (+0.21%), so there is no established performance regression or gain.

The focused shader regression failed before the split tint and passes after. The release ocean
filter passes 65 tests with 12 ignored; formatting, release build and replay pass. Validation used
the committed 1.0 wave scale while preserving and excluding the unrelated local 2.0 scale edit.
The regular square/triangle raised-edge pattern remains a separate unresolved defect.

## 23 September — low-eye ocean acceptance and latest grid isolation

The new `ocean_deck_reference` and `ocean_manual_grid` replays are retained;
all ripple, displacement, normal-band and axis experiments are reverted.
See the current-state section above and
`test-runs/ocean_sot_detail_2026-09-23/REPORT.md` for the controlled findings.
Final low replay `1790183721-825973` passes with measured eye clearance
2.4984–2.5070m. Restored grid replay `1790183741-825999` passes and all four
captures are pixel-identical to control `1790182690-821467`.

Validation: 45 scenario tests pass; the broader release suite has 548 passing,
25 ignored and four explicit skips. Three are the previously recorded local
atmosphere, walking and time-ladder failures; the fourth is
`cpu_wave_scale_matches_the_rendered_ocean_scale`, whose hard-coded maximum
fixture fails with the separately edited local `OCEAN_WAVE_SCALE=1.5`.
That edit is preserved and not staged. Formatting, release check and release
build pass. No production shader changes, performance win, human motion
acceptance or Sea of Thieves appearance completion is claimed.

## 25 September - Ocean FFT plan, Phase A (lattice metric)

Added `tools/lattice_metric.py`: whitened 2D-FFT peak/median of a capture (optionally `--crop=x0,y0,x1,y1` fractions to isolate water). Calibration: white noise scores ~19, a synthetic two-tone lattice ~243. Current baselines: `ocean_manual_grid/1790185095-829305` captures 1-4 score 72-88 (older `1790183741-825999` 140), so the woven pattern is clearly detected. Caveat: frames containing horizon/sky (e.g. `ocean_rough_horizon`, 231) need a water-only crop or the gradient dominates. Not yet done for Phase A: interleaved Immediate-present frame-time baseline, and Ian's Sea of Thieves reference shots. No renderer code changed.

## 25 September - Ocean FFT plan, Phase B step 1 (standalone GPU FFT, cost proven)

New `crates/app/src/ocean_fft.rs` + `ocean_fft.wgsl`, standalone (not wired into rendering or buoyancy). Three 256x256 cascades (tiles 1000/237/53m, disjoint wavenumber bands 0-0.5-2.0-inf rad/m), JONSWAP with cos^2 spreading, deep-water dispersion. Per frame: evolve, row FFT, column FFT, assemble into (h, Dx, Dz) per cascade, using two packed complex FFTs per cascade (h+iDx, Dz). Slopes and the fold Jacobian are meant to come from finite differences of these textures at shading time (not implemented yet). Workgroup-shared 256-point radix-2, one workgroup per line.

Measured on the Quadro M1000M (wall-clock, submit+wait, 100-frame batches, dedicated run): **0.81-0.84ms/frame** for the whole field, inside the 1.0ms budget. Pass split (before halving, 12 FFTs): evolve 0.16, rows 0.71, cols 0.76, assemble 0.22ms; halving to 6 FFTs took 1.53 -> 0.82ms. A shared twiddle table was slower (1.74ms) and was reverted. Correctness: single-mode test matches the analytic standing wave (h, Dx, Dz) to 3e-6.

Run: `cargo test -p catinthegarden-app --release ocean_fft -- --ignored --nocapture --test-threads=1`.
Not done: texture output with mips, wiring into the camera-local patch, CPU 64x64 parity (phase C), weather-driven wind/fetch, time wrapping (f32 phase over long runs). This is standalone cost, not a total-ocean-cost claim.

## 25 September - Ocean FFT plan, Phase B step 2 (FFT field drives the raster ocean, opt-in)

`CATINGARDEN_OCEAN_FFT=1` (optionally `CATINGARDEN_OCEAN_FFT_WIND=<m/s>`, default 14) routes `ocean_surface` in the raster path through the GPU FFT field instead of the 18 Gerstner waves + 3 ripples. Default is off and the default shader is unchanged (the FFT call is stripped from the source in `shared_planet_shader_source`, so auto-layout tests keep working; the 6 `gpu_ocean_*` tests pass with it off).

Design: assemble writes a 256x256x3 rgba16f storage-texture array (h, Dx, Dz); shared bind group(2) gets bindings 16 (map), 17 (repeat/linear sampler), 18 (`OceanFftView`: fixed tangent-plane axes + per-cascade camera fractional tile coords). Tile coords = camera fraction (f64 on the CPU) + tangent-plane projection of the camera-relative view position, set through `var<private> ocean_fft_view_position` by `vs_ocean`, `ocean_raster_surface`, `flat_ocean_colour`. Cascade 0 (wavelengths >~12m) is mesh geometry and normal; cascades 1-2 only feed `ripple_slope`/`ripple_height`, faded by camera distance (no mips yet). Slopes and div(D) (as `convergence`) are forward finite differences of the texture. The same depth-based breaking limiter is applied. `max_sampled_textures_per_shader_stage` is raised to 20 (the ray-field pipeline was already at 16).

Result on `ocean_manual_grid` (overhead lattice view), lattice metric, four captures: Gerstner 140-188 -> FFT 30-38 (white noise ~19). Captures: `test-runs/ocean_manual_grid/1790294496-11868` (FFT) vs `1790294468-11777` (control). Visually an irregular wind sea with foam patches and no diagonal hatching.

NOT done / known: horizontal displacement is not applied (phase C); **CPU buoyancy, ship, and camera clearance still use the Gerstner sea, so low cameras will clip FFT waves** until phase C; ray/foveated path and `ocean_surface_world_direction` callers (planet.wgsl:387 etc.) don't set the view position so they are not FFT-correct; wind/fetch not coupled to weather; no mips; no frame-time comparison of the whole frame yet; f32 time. Note the game binary is at `target/release/catinthegarden-app` (the `/home/dad/catingard-target/release` binary is stale, 14 Sept).

## 25 September - Ocean FFT plan, Phase C (CPU buoyancy/collision parity, opt-in)

With `CATINGARDEN_OCEAN_FFT=1`, `ocean.rs` height/slope/velocity entry points (`global_wave_height_meters`, `local_wave_height_meters`, `global_wave_slope`, both vertical-velocity functions) use `ocean_fft::CpuSurface` instead of the Gerstner sum. It is the same spectrum (`default_h0`, seed 1, `CATINGARDEN_OCEAN_FFT_WIND`) inverse-FFT'd on the CPU (radix-2, cascade 0 only, the geometry cascade), height and vertical velocity packed into one complex FFT, bilinear + forward-difference slope exactly as the shader does, shared tangent-plane anchor (`anchor_axes`, first camera direction). Depth limiter reuses `breaking_weight` / `breaking_rate_weight`. Finer cascades are shading-only, so local == global.

Measured: CPU grid vs the actual GPU texture max error 0.26mm (height range 1.9m); velocity matches a numeric derivative to 5e-3; holding the grid up to 0.03s and advancing by velocity errs 0.27mm. A full CPU refresh costs ~2.3ms but happens at most once per 0.03s of ocean time (not yet threaded; watch frame cost with fast time scales). Removing truncation was necessary: direct mode summation was 6.5ms/sample and top-N modes kept only 90% variance at 2.7k modes.

Scenarios with FFT on all pass: `ocean_ship_float/1790295299-13526`, `ocean_deck_reference/1790295594-13750`, `ocean_rough_horizon/1790295814-13958` (deck-level capture shows a rolling swell with the camera above the water). Xvfb runs are slow (~5 min each), so no frame-time claim.
Not done: horizontal choppiness/transport with inverse lookup (needed for cusps), the phase-C ship-specific parity assertions beyond these scenarios, weather-driven wind, mips (fine striping near the horizon), ray path.

## 25 September - Blur stage converted to edge-aware anti-aliasing (FXAA-style)

`blur_scene` in `hdr.wgsl` (the 5x5 box blur behind F6) is now an FXAA-style filter: edge detection on tone-compressed luma (threshold max(0.0312, 12.5% of local max)), horizontal/vertical edge classification, a 0.5-0.75 luma-weighted blend toward the neighbour across the edge (sub-pixel term for isolated pixels); flat areas and fine texture pass through untouched. It still runs on the HDR scene before tonemapping and reuses the same pipeline, toggle (F6) and default (`BLUR_ENABLED` = off, per Ian's 20 Sept request). HUD/help now say "AA". New `CATINGARDEN_AA=1` starts with it on (also how scenarios can capture it). Evidence: `orbit_once/1790297588-16123` (off) vs `1790297603-16165` (on): 16.1% of pixels move by >2 levels (flat-triangle planet full of edges), mean gradient 8.61 -> 7.34, and a 4x crop shows smoothed stair-steps with colours intact. Not FXAA 3.11's edge-end search (no long-edge blending), so long near-horizontal edges get the half-pixel treatment only. Cost not measured (one pass, ~9 taps versus the old 25).

Follow-up (25 Sept): interactive launches now start with the F6 anti-aliasing on (`apply_interactive_startup_controls` calls `set_effects`, not the toggle; `CATINGARDEN_AA=0` opts out). Scenarios still use the `BLUR_ENABLED` default (off), so scenario captures and baselines are unchanged. Test `interactive_startup_enables_anti_aliasing` replaces `interactive_startup_does_not_enable_blur`.

## 25 September - Ocean FFT plan, Phase D (revised: band-limit the mesh, not densify it)

Measurement changed the phase. In the deck-level FFT run (`ocean_deck_reference`) 164 of the 249 ocean chunks are already L18 (~0.75m quads) and 19 more L17, so the foreground is far denser than the geometry cascade needs (wavelengths >= ~12m). The real defect was the opposite end: 6-12m vertex spacing beyond a few hundred metres sampling cascade-0 waves of 12-100m, which aliases. So the FFT texture now has a full mip chain (9 levels, 2x2 box downsample, +0.08ms; FFT + mips ~0.88ms on the Quadro, wall-clock) and `ocean_fft_cascade` takes a filter width: in the vertex stage 2x the true vertex spacing (chunk cube-UV span x 0.7 R / 32, set via `ocean_fft_vertex_spacing_meters`), in the fragment stage 2x the pixel footprint (distance x 2 tan(fov/2) / 720; 720 assumes the default viewport). Forward-difference step scales with the mip. Result on `ocean_deck_reference/1790315257-23054` vs pre-mip `1790295594-13750`: horizon band (rows 315-360) horizontal gradient 7.89 -> 5.08 (-36%), mid and near bands identical (lod 0 there). Not done: anisotropic filtering, real viewport height in the shader, a frame-time comparison, ray path. Denser near mesh was NOT implemented because the data says it is not needed; ocean and terrain share one 256-chunk budget, so an ocean-specific coarser cap near the camera is a possible future saving.

## 25 September - Ocean FFT plan, Phase E (Sea of Thieves shading, FFT ocean only)

Under `CATINGARDEN_OCEAN_FFT=1`, `ocean_lighting` delegates to new `ocean_lighting_sot` (default lighting untouched). Model, after Rare's SIGGRAPH 2018 talk: body colour = the existing sun-facing deep/teal ramp (`ocean_body_albedo`, 85% weight against `OCEAN_SOT_DEEP_COLOUR`) blended toward `OCEAN_SOT_SUBSURFACE_COLOUR` by a weight from the wave-peak mask (FFT convergence, i.e. bunching of the displacement field; smoothstep 0.20-0.65), backlight (view toward sun) and sun-facing/grazing terms, plus a subsurface glow on backlit peaks. Sun specular is a peak-normalised GGX lobe using Karis's representative point on an artificial sun sphere (tan radius 0.06 vs the real 0.0046) with roughness rising 0.08 -> 0.5 from 30m to 1.5km range, energy-normalised, faded out at grazing view angles (else a white line drew along the horizon). Existing Fresnel, cubemap reflection and foam composition are unchanged; the old crest-transmission layers are bypassed in this mode only.

Evidence: `ocean_deck_reference/1790316777-24052` (vivid teal, lighter thin peaks); `ocean_low_sun_stability/1790317319-24599` vs the first SoT pass `1790317159-24372`: glitter road width (pixels above local warm baseline) 63/37/23/21 -> 132/126/63/21 at rows 230/300/400/550. All scenarios pass. Full app suite: 551 pass, 4 fail, none from this change (the two long-standing ones, plus the time-speed ladder and atmosphere mist-declaration tests, which follow uncommitted edits in `main.rs` and `atmosphere.*`).
Constants are eyeballed against one reference frame each; Phase H (art direction against Ian's reference shots, which have not been supplied) is where they get tuned. No frame-time measurement (xvfb).

## 25 September - Lattice metric correction (no new pattern in the FFT sea)

After Phases D/E the overhead FFT score rose 36 -> 86, which looked like a new pattern. It is not. The whitened peak sat at ~2.4px wavelength, and its absolute amplitude actually fell 0.43 -> 0.16 grey levels rms (sub-level, invisible): the mips removed broadband pixel sparkle, lowering the median the ratio divides by. The strongest *absolute* peaks in the Phase E capture are 17-31px spread over -45..-80 deg, the wind sea's JONSWAP peak band; their amplitude rose 0.65 -> ~0.87 levels because the SoT peak mask gives real crests more contrast. The Gerstner lattice, by comparison, is 3.7 levels with several peaks stacked at 86-88 deg. `tools/lattice_metric.py` now reports `peak_levels` (primary), `top8_share`, `detail` and the old `whitened` ratio, with that calibration in its docstring.

## 25 September - Ocean FFT plan, Phase F (temporal fold foam, FFT ocean only)

Under `CATINGARDEN_OCEAN_FFT=1` the existing world-reprojected foam history atlas (`ocean_foam.rs`/`.wgsl`, formerly opt-in via `CATINGARDEN_OCEAN_FOAM_HISTORY`) is always on and is fed by the Tessendorf fold Jacobian J = (1+dDu/du)(1+dDv/dv) - dDu/dv dDv/du of all three FFT cascades, instead of Gerstner convergence plus random flecks. Atlas: 256x256 over 512m (2m texels; the Gerstner trial keeps 128), birth smoothstep(J 0.66 -> 0.36), 2.5s decay, small feedback blur so foam spreads as it ages. The texel's camera offset comes from exact atlas metres, not an f32 planet-radius subtraction. The FFT update now runs before the foam pass (main.rs) so the atlas reads the current field.

Per pixel, `ocean_surface_fft` also computes J (free: same three samples) as `ocean_fft_fold_foam`; beyond the atlas that instantaneous fold is used, inside it the smooth history only (the per-pixel J is forward differences of 3.9m texels, piecewise flat, which drew polygonal white chips near the camera). The fold replaces the slope/height whitecap rule in FFT mode; surf and shoreline wash are unchanged. Foam is broken into lace by a pattern made from the finer cascades' own heights (normalised by their measured std, 0.056m / 0.218m), standing in for Rare's authored foam texture; thin foam stays partly translucent.

Measured foam-like pixel share (4 captures over 4s, rising as history builds): `ocean_manual_grid/1790332752-42089` 0.95 -> 1.69%, `ocean_deck_reference/1790332905-42224` ~0.6-1.6% of water, `ocean_rough_horizon/1790333143-42417` 2-5.5%. Monahan's empirical whitecap fraction at 14m/s is ~3%. Verified on the committed tree in an isolated worktree: 553 tests, 6 GPU ocean, 5 FFT tests, FFT and default scenarios pass.
Not done: object/hull intersection foam (Rare's depth-compare ring), wind/sea-state coupling (coverage scales with the fixed 14m/s spectrum only), foam advection with the surface, frame-time measurement of the atlas pass (65k texels x 14 samples; expected well under 0.2ms, unmeasured).

## 25 September - FFT CPU surface: F10 freeze (birds) made the game crawl

Ian pressed F10 (which starts time, and with it the birds) with `CATINGARDEN_OCEAN_FFT=1` and got ~1 frame per 20s. Cause: `CpuSurface` cached one grid and refreshed (2.3ms inverse FFT) whenever a query was more than 0.03s from it. `sea_avoidance` in `birds.rs` samples the water at `time + {0,0.5,1,1.5,2,3}s` for every bird, so each call missed the cache: ~100 birds x 6 x 2.3ms x up to 15 catch-up steps. Fix: 8-slot grid cache keyed by time (`GRID_SLOTS`); test `interleaved_query_times_do_not_refresh_every_call` (1200 interleaved samples must take <0.5s; a single slot takes ~2.8s). Not measured in the live game; needs Ian to press F10 again with FFT on. Also: `response/codex.txt` was overwritten with a status note (AGENTS.md relay), replacing the old Codex transcript (still in git history).

## 25 September - FFT choppy horizontal displacement (first pass toward cusps)

Ian: water and foam good, but no cusps / small peaks. `ocean_surface_fft` now fills `horizontal_displacement` from the Dx/Dz of all three cascades (mid/fine by their existing distance weights, scaled by `geometry_weight` and the depth breaking weight), and corrects the shading slope by the inverse displacement Jacobian (M^-T applied to label-space slope, determinant clamped at 0.2) so pinched crests steepen. Strength `CATINGARDEN_OCEAN_FFT_CHOP` (default 1.0, 0-2), carried in `ViewParams.gain.y`. Foam still uses the unscaled (chop=1) Jacobian. Evidence: `ocean_rough_horizon` chop 0/1/2 pre-slope-correction were nearly identical; with slope correction chop 2 (`1790336024-48581`, assuming that dir is chop 2) shows sharper, more streaked crest faces but silhouettes are still rounded -- NOT convincing cusps. Known gap: CPU buoyancy/collision does not include the displacement (no inverse lookup yet), so the drawn surface differs from the CPU one by slope x displacement (unmeasured, likely cm to tens of cm). No frame-time measurement.

## 25 September - "FFT OCEAN" mode badge

With `CATINGARDEN_OCEAN_FFT=1` a plain white "FFT OCEAN" pixel-font label is drawn in the scene pass at the top centre (part of `flock_marker`, separate pipeline/uniform, depth-test off, HDR 4.0 white). It is in the rendered frame, so it shows with the debug overlay hidden and in scenario captures (checked in `orbit_once`; `still_5s` does not draw the scene pass). Absent when FFT is off.

## 25 September - FFT ocean: choppy sign fix, fold limiter, swell cascade, CPU lattice cache

Ian: "negative peak thing that looks like boiling water"; FPS drops going 100% -> 200% time speed; wants a Sea of Thieves-style big swell hiding the horizon (examples/waves.png).

1. **Choppy sign was inverted.** The FFT stores D = +k^ a sin where h = a cos (checked by `single_mode_produces_the_analytic_standing_wave`), so the crest-ward surface is x0 - D. The shader drew x0 + D, pinching troughs into downward spikes and broadening crests. The same sign error put fold foam (Jacobian with 1+dD) and the SoT peak mask (convergence = -div D) on troughs. All four now use x0 - D: `chop = -(...)`, `ocean_fft_fold_amount` and the foam atlas use (1 - dDu/du)(1 - dDv/dv) - ..., convergence = +div D, slope correction through I - grad D. Foam now lies as streaks along crests (`ocean_manual_grid/1790355817-80387` vs `1790332752-42089`).
2. **Fold limiter**: displacement and the slope correction scale by mix(0.25, 1, smoothstep(0.1, 0.6, det)) so the mesh never turns inside out; determinant clamp 0.35.
3. **Swell cascade** (cascade 3, 2,170m tile, 8.5m texels): narrow-band Gaussian-in-frequency spectrum, 170m peak wavelength, 10% frequency spread, cos^16 spreading, 30 degrees off the wind so crests cross; normalised to Hs = 1m (test) and scaled by `ViewParams.gain.z = swell_height_meters(storm)` = `CATINGARDEN_OCEAN_FFT_SWELL` (default 8m) x (1 + 0.8 smoothstep storm). Swell is non-local, so it is present in calm local weather (interactive weather mode starts near zero storm). Height/slope/displacement/Jacobian/divergence add to cascade 0; foam atlas includes it. GPU FFT: 1.15ms/frame for 4 cascades vs ~0.88 for 3 (ignored benchmark, Quadro, wall clock). Replays: `ocean_deck_reference/1790355860-80743` (9m) shows a swell rising above the horizon; `ocean_ship_float/1790355913-81352` (8m) hull seated, pitching/rolling. No whole-frame timing (xvfb).
4. **CPU surface rewritten** (`ocean_fft::CpuSurface`): mirrors wind cascade 0 and swell with h, v, Dx, Dz grids (two packed complex FFTs per cascade, rows/cols with the horizontal pack on a scoped thread); **Newton inverse** of x0 - c D(x0) = p with the same fold limiter, slope through (I - c grad D)^-T. Tests: labels recovered within 0.5mm, heights within 5mm; CPU vs GPU texel 0.26mm (wind) / 0.11mm (swell). **Time lattice**: grids at 0.1s steps, extrapolated by velocity (worst height error 1.5mm), 40-slot LRU (~80MB), and a `ocean-cpu-fft` worker prefetches 3 steps past the highest key requested. This fixes the time-speed FPS drop: the old 0.03s refresh window rebuilt a 2.3ms transform for every bird look-ahead time, so cost scaled with birds x time speed. One wind+swell transform is ~7ms, now normally off the render thread. Not measured in the live game. Vertical velocity ignores dD/dt (drag only).

Colour question (turquoise with dark specks, looking down): at normal incidence Fresnel is 2%, so the SoT body colour dominates; it is 85% `ocean_body_albedo`, a sun-facing ramp, so sun-facing faces are teal and faces tilted away are dark blue. Real deep water seen from above is dark navy. Proposed (not done): make body colour depend on view angle (navy looking down, teal only through thin backlit crests). Also not done: splash/spray particles, wind-advected foam.

## 25 September - FFT ocean step 2: collision peaks (geometric 3-12m waves + second-order crests)

- Cascade 1 (237m tile, 3-12.6m waves) is now vertical geometry too (weighted by its existing 600-3000m fade; the vertex mip filter already band-limits it to the mesh), so short crests rise instead of only tilting normals. The CPU mirror adds it (CPU/GPU texel 0.15mm).
- Second-order (Stokes) term: height += s * h_lin * clamp(div D, +-0.6) - mean, slope and velocity scaled by (1 + 2 s div). For one wave it is exactly (k a^2/2) cos 2theta; where crests cross, h and div D are both large and the peak piles up (crossing equal waves: +3ka^2 vs +ka^2/2). The mean (Parseval sum |k||h~|^2 per cascade, swell scaled by Hs^2) is subtracted so sea level stays put. `CATINGARDEN_OCEAN_FFT_PEAKS` (default 1, 0-3), in `ViewParams.second_order`. CPU Newton inverse now 6 iterations (5.5mm miss at 4 with the steeper field).
- Replay `ocean_deck_reference/1790356554-102556`: pointed whitecapped crests above the horizon line. GPU ocean parity tests (6) and FFT GPU tests pass.
- Also answered (not implemented): distant-water aliasing. Plan = Bruneton 2010 geometry->normals->BRDF: store slope^2 in the FFT mips (spare 4th channel), add unresolved slope variance to specular roughness, anisotropic (sampleGrad) wave sampling at grazing angles, foam as filtered coverage, TAA later. Driven by pixel footprint (FOV-aware), so zoom keeps detail.

## 25 September - FFT ocean step 3: wind-blown spray and wind-streaked foam

- **Spray** (`ocean_spray.rs`, `ocean_spray_update.wgsl`, `ocean_spray_draw.wgsl`, owned by `TerrainRenderer`): 8,192 GPU particles in the FFT tangent plane relative to the camera (u, v, height above sea level), shifted by the camera's own tangent-plane motion each frame (f64 on the CPU). A compute pass (after the FFT update) integrates wind drag (1.2/s toward the 14m/s spectrum wind), vertical drag and gravity, and respawns each dead particle with one random attempt within 300m (density 1/r, toward the camera), born with probability ~ smoothstep(J 0.40 -> 0.0) of the displaced-surface Jacobian (all four cascades, choppiness, swell scale) at the drawn crest height (with the second-order term), thrown up 1-4.5m/s and 20-50% of wind speed downwind, living 1-2.6s. Drawn in the transmitting-ocean pass after the water (depth-tested, no depth write, premultiplied alpha): camera-facing puffs stretched along screen velocity, size 0.3-1.3m capped at 5% of distance, faded near the eye (3-10m), lit with `ocean_foam_radiance` from the shared sun/sky LUTs plus a forward-scatter glow. `CATINGARDEN_OCEAN_FFT_SPRAY` = strength multiplier (default 1, 0 off). No readback. Replay `ocean_deck_reference/1790357476-115410` shows spray puffs along crest tops; first tuning (`1790357308-113700`) was a smoke-cloud blob from a near-eye particle, fixed by the screen-size cap.
- **Wind-streaked foam**: the fold-foam history reprojects with a 0.8m/s downwind drift and its feedback blur is 0.35/0.35 along the wind, 0.15/0.15 across (was isotropic), so foam trails into streaks. Wind (u, v) passed in `FoamFrame.timing.zw`; `ocean_fft::WIND_DIRECTION` is now the single source.
- Not done: hull/object-intersection spray and foam; spray timing (no whole-frame measurement); spray is not weather-coupled (fixed wind).

## 25 September - The moon in the sky during ordinary play

Ian asked to see the moon from the planet surface, as a first step toward the planet-to-moon flight. `sky_moon.rs` reuses the `planet_to_moon` replay machinery (second resident moon `TerrainRenderer` from `assets/outmaps/test-moon`, its own airless `AtmosphereRenderer`, the replay's offscreen colour/depth `Composite` and `system_composite.wgsl`), now `pub(super)` in `system_flight.rs` along with `distant_level` and `scene_pass`.

- **Placement**: fixed in inertial space at the replay's 40,000km centre distance (angular diameter ~3.1 degrees, about 6x our Moon). Placed on the first rendered frame 20 degrees above the horizon, choosing the best-lit of azimuths 0/+-15/+-30 degrees from the view so it starts in shot; its baked landing site faces the planet. It rises and sets as the planet turns; the phase follows the sun. No orbit, earthshine or eclipses yet.
- **Rendering**: each frame the planet-local camera is transformed into the moon body frame (f64), the moon camera uniform copies the planet's projection (same reversed-Z mapping, so depths compare across bodies) with body-frame basis, radial and sun. Offscreen pass before the main pass, then `Composite::draw` in the main pass after opaque ground and before the sky (the sky only fills depth 0), adding planet-atmosphere in-scatter and extinction and writing depth: terrain, trees, clouds, the ocean and the sun disc occlude correctly. Skipped when outside the view cone or wholly below the geometric horizon. Logs `sky moon placed` and visibility changes.
- **Switch**: on by default for ordinary planet launches on the raster path; `CATINGARDEN_MOON=0` off; scenarios only with `CATINGARDEN_MOON=1`, so existing captures are unchanged. Setup adds ~1.0s to startup; per-frame cost not measured (xvfb).
- **Moon outmap had stopped loading** (so `--body moon` and `planet_to_moon` were broken too): the bake predates the GlacialMoraine/CrevasseField biome ids and both manifest validators demanded the full table. Biome ids are only appended, so both (`coretypes` `OutmapManifest::validate` and `outmap.rs`) now accept a table that is a matching prefix of `BiomeId::ALL`; mismatched, empty or longer tables are still rejected (tests added).
- Evidence: daytime crescent `ocean_deck_reference/1790359575-122210` (moon top right, dark limb filled with sky); night gibbous with crater rims `forest_night/1790360408-124912` (with `CATINGARDEN_DISABLE=forest`; with the forest on, a tree correctly occludes it, which cost a debugging detour: black trees on a black sky).

## 25 September - Sea of Thieves water colour: one hue, translucency brightens

Ian: in the reference (examples/waves.png) translucency brightens the thinner water without changing its colour, the top of a wave is lighter than its base, mostly with the sun behind it; use that sea colour. Measured: hue 182-192 degrees and saturation ~1 throughout; sRGB troughs looked into (0, 92, 115), typical (0, 155, 176), thin backlit top (6, 195, 208); down one wave face the top is ~25% brighter than the base.

`ocean_lighting_sot` (FFT only) now uses one linear albedo in that hue (`OCEAN_SOT_WATER_ALBEDO` 0.0018/0.24/0.30) and two components, both in that hue: (1) the lit water body, times (1 + 0.9 thin + 0.4 peak) and a view-depth factor (0.45 looking straight down -> 1 grazing); (2) transmitted sunlight, albedo x sun x 0.9 x (view toward sun)^4 x (thin^2 + 0.6 peak + 0.3 fine), times (1 - Fresnel). `thin` = smoothstep(-0.6, 1.6) of the surface height over the local sea's height std (new private `ocean_fft_surface_height_fraction`, set in `ocean_surface_fft`; wind-cascade std constant 1.264m). Removed: navy deep colour, the teal subsurface colour and the sun-facing body ramp (the "turquoise with dark blue specks" from above). Reflection, specular and foam unchanged.

Pixel check against the reference (water region, hue/sat/value): reference right side p50 190.4/0.98/0.61; `ocean_deck_reference/1790362814-140635` p50 188.1/0.96/0.47 and 187.6/0.81/0.63; `ocean_low_sun_stability/1790362852-141367` darker and greener (175/0.93/0.21) under reddened low sun, with the transmission glow along the sun path. Foreground slightly less saturated than the reference (sky reflection greys it).

## 25 September - Moon orbit and earthshine

- **Orbit** (`sky_moon.rs`): circular, prograde, in the least-inclined plane through the first-frame position (inclination = its declination; 26.9 degrees in the sunset replay). Kepler period for GM = g R^2 at 40,000km: ~35.3 real hours = 26.4 game days at the 4,800s interactive day, close to Earth's month. Runs on the planet-rotation clock, so F10 freezes it and time speed / F1-F2 scale it; scenarios with rotation scale 0 hold it still. Tidally locked: the body turns with the orbit.
- **Earthshine**: the moon shader already had `planetshine_irradiance` (fixed `MOON_PLANET_SKY_DIRECTION`, phase from the moon's sun direction, `MOON_PLANETSHINE_FRACTION` 0.022 with its own visibility boost for standing on the moon). The sky moon is now oriented so that direction faces the planet (`planet_vs_constant` 1.0 in the log; shader planet phase 0.93 for the crescent), which makes the phase correct: strongest on a crescent. Seen from the planet it came to ~0.1% of the sunlit crescent (dark limb 1/255 at night); physical for this geometry is ~0.3%. `camera_up.w` (spare lane) now carries an extra gain for the sky view (`SKY_EARTHSHINE_EXTRA_GAIN` 5, so 6x): dark limb 5/255 next to a 174/255 crescent in `sunset_blue_hour/1790363654-144479` capture 6, the whole disc faintly visible. Zero (no change) when standing on the moon.
- Ian's point: moonlight is very dark relative to daylight (~0.25 lux vs 100,000). The disc itself is sunlit rock and bright; moonlight on the planet is not modelled.
- `sunset_blue_hour` fails its blue-hour assertions in the working tree with the moon on *and* off (identical reasons); the moon changes 263 pixels in the corner. See the verification note below for the clean-checkout result.
- Verification: `sunset_blue_hour` also fails on a clean checkout of d59b79e with the moon off (same two blue-hour reasons), so the failure is in committed code and predates this session's moon/ocean work; cause not investigated.

## 25 September - FFT ocean Phase G: distant-water anti-aliasing (geometry -> normals -> BRDF)

Ian's zoomed-out screenshot: the horizon band was a mess of pixels; FXAA cannot fix sub-pixel waves. After Bruneton, Neyret & Holzschuch (2010):
- **Slope statistics in the FFT mips**: `assemble` (ocean_fft.wgsl) writes |grad h|^2 (central differences) into the field's alpha; the existing 2x2 box mip chain averages it, so a lookup at any LOD returns the footprint's mean-square slope.
- **Unresolved variance -> roughness**: per cascade, mean-square slope minus weight^2 x |resolved slope|^2 (a cascade faded out by distance hands all of its slope over); swell scaled by its height^2. Specular alpha = sqrt(0.08^2 + variance), capped 0.6; replaces the 30m-1.5km roughness ramp (`OCEAN_SOT_ROUGHNESS_FAR` removed). Far total ~0.07 at 14m/s, matching Cox-Munk.
- **Anisotropic footprint**: fragment filter width = across-view pixel width / max(cos incidence, 0.02), i.e. the long axis; the vertex stage still filters by vertex spacing.
- **Energy**: the peak-normalised lobe gained energy as it widened (white sheens on the deck view). Now scaled by (base width / width)^2 (true GGX 1/alpha^2); the grazing visibility clamp is relaxed 1.5 -> 8, and the n.v fade narrowed 0.12 -> 0.03 because filtered distant normals are nearly flat and the wide fade had removed the far glitter road.
- **Foam**: per-pixel fold foam fades out once the footprint is 2^1.5-2^4.5 mid-cascade texels (salt-and-pepper otherwise).
- Measured (horizontal Laplacian, pixel-scale noise, low-sun fixed camera): just below the horizon 1.38/2.35 -> 0.58/0.76, 20-60 rows below 3.78/4.14 -> 0.67/0.61, near 2.85/2.31 -> 1.94/1.66; deck view horizon band 0.71-2.77 -> 0.46-0.61. Low-sun road/side luminance 1.34/1.41 before -> 1.20/1.22 (a smooth road rather than glitter; the intermediate 1/alpha and no-conservation variants were rejected: road gone, or white sheens). Runs `ocean_low_sun_stability/1790364853-159342`, `ocean_deck_reference/1790364817-158614` vs `1790362852-141367`, `1790362814-140635`. Thin dark dashes along the far horizon line predate this. No frame-time measurement (xvfb); the fragment adds no texture samples (alpha was already fetched).

## 25 September - Spray and foam around the ship (FFT ocean)

- **Bow slam** (`main.rs` `ship_spray_emitter`, each frame after the ship transform upload): the bow point (0.45 L forward of the waterline origin) velocity from the body's linear + angular velocity, against the water's vertical velocity there; intensity = smoothstep(0.3, 2.5 m/s, downward speed into the surface) x smoothstep(-1.5, 0.5 m, bow immersion). Handed to the terrain as `terrain::ShipSprayEmitter` (waterline origin, forward, velocity, intensity). In the storm `ocean_ship_float` replay it is non-zero in 27% of frames, mean 0.13, peak ~1: spray comes in bursts.
- **Bow spray**: the spray pool is now 16,384; the first 2,048 slots (`SHIP_SPRAY_SLOTS`, pinned to both shaders by a test) belong to the ship. A dead ship slot spawns with probability intensity x 30/s x dt on the forward third of the waterline outline (the `half_beam_meters` plan shape), either side, thrown outward (raked forward near the stem) at 3-11 m/s and up at 5-17 m/s (clears the 6m freeboard), plus the hull's velocity and a little wind, living 0.9-2.1s. Drawn as 0.8-3.5m sheets (crest mist stays 0.12-0.5m), alpha 0.9 e^(-2.5 age). Checked with forced intensity 0.8: `ocean_ship_float/1790365945-183947` capture 1 shows a white sheet over the bow blowing aft; a magenta/green spawn diagnostic confirmed the stem end (the bow is the left end in these captures; the superstructure is aft).
- **New spray style for crests too**: no fade-in, alpha 0.8 e^(-4 age) (dense at the source, gone fast), small droplets stretched along screen velocity: Ian's "hard line at the wave edge, quick fade" rather than puffs.
- **Hull foam**: the foam history pass gets the ship's waterline origin and heading relative to the atlas (`FoamFrame.ship`, `.ship_axes`) and births foam in a band just outside the hull outline (2.5m + 2m x intensity wide), 0.35-1 from stern to bow, stronger when it slams. The history is world-fixed, so it trails as a wake as the hull drifts.
- Not done: spray from waves striking the hull side, the propeller wake (the ship has no propulsion), frame-time measurement.

## 25 September - Hull spray all round, not just at the bow

Ian: a free-floating hull should splash all round, not only at the bow as if under way. The slam is now measured at eight waterline stations (`SHIP_STATIONS` t = -0.9, -0.3, 0.3, 0.9, each side; `ship::half_beam_meters` for the outline): per station, how fast the water surface rises against that hull point (hull velocity from linear + angular, against the water's vertical velocity; covers the hull driving down and crests striking), gated by that point's immersion, same ramps as before. `ShipSprayEmitter` carries `port`/`starboard` [f32; 4] (plus their max as `intensity`); both the spray and foam shaders interpolate between stations (`station_intensity` / `hull_station_intensity`, positions pinned by a test). Spray now spawns uniformly along the full length on either side, weighted by the local slam, thrown along the outline normal (raked forward where the bow narrows, aft toward the stern, straight aft off the transom); the hull-foam band's width and strength follow the local slam and the old bow bias is gone. Storm replay, mean slam stern -> bow: port 0.12/0.17/0.13/0.05, starboard 0.21/0.16/0.16/0.18, each station active 13-33% of frames. `SprayFrame` 160 bytes. Knobs: see the previous section; `SHIP_SPRAY_RATE` now spreads over the whole hull.

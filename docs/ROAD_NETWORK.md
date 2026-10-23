# Planet-wide village roads — implementation status

## Scope and current limit

The requested network covers the **whole planet**, with shared roads and real
junctions rather than every village connected separately to every other village.
**Island connections must be tunnels below the seabed, not ferries or surface
bridges.** The existing elevated deck, terrain-led alignment, <=25% physical
grade and hillside tunnels remain the intended construction rules.

The network is now **rendered, streamed and walkable**, enabled by the usual
`PLANET_ROAD_EXPERIMENT=1` launch. It includes exact village enumeration, shared
regional gateways, fitted decks on legs, cuts, covered bores and tree clearance.
Generation is on demand: this does **not** claim that every region of the world
has already passed a complete engineering audit.

## Wooden sign boards and natural lettering, 23 October

All junction/distance/branch, village and tunnel boards now use warm oak grain
and black EB Garamond12 Bold lettering instead of green/white panels and white
sans-serif text. The unmodified font is embedded with its OFL1.1 license;
existing egui fallback fonts remain for missing symbols. Village plaques retain
a slightly lighter wooden rim. Boards have a closed6cm wooden body; posts and
mounts, board centres/sizes, text, arrows and measured route distances remain.

One generated albedo tile is decoded at renderer startup, reduced to512² and
uploaded once as a10-level sRGB mip chain (~1.33MiB). Grain uses board-local
metres and a repeating filtered sampler, rather than camera/world movement.
Negative-U vertices distinguish wood from glyph UVs and solid hardware without
changing the44-byte vertex layout. The wood stays cached across sign uploads;
font layout/mesh preparation still run on the worker, in the same bounded
single draw. Existing independent tunnel-board illumination is retained.

Assets, license/provenance and full built-in image-generation prompt are in
`assets/road_signs/`. Daylight junction and both tunnel sign faces were opened;
kind counts, glyph alpha/winding, black ink, finite UVs and mip data are tested.

## Stronger lantern flutter and precise highway steering, 23 October

Partial steering now uses a speed-dependent progressive response, separate
from available wheel lock/grip. Low-speed behaviour is unchanged through
36km/h; between36 and108km/h the near-centre gain smoothly decreases to30%
of its old value. At full input the response is still full lock. There is no
new dead zone or added delay:15ms mouse filtering,8rad/s countersteering,
6.5m/s² lateral limit, brakes, acceleration and speed caps are retained.
The L/R indicator continues to show actual wheel position after this curve.

Lantern strength now combines1.7Hz slow noise and7Hz irregular flutter in a
bounded0.45..1.37 envelope, replacing0.80..1.02. Expected mean remains0.91;
quarter-intensity red-orange colour and indoor exposure are unchanged. Flame
and cast light use the same seeded noise, independently per lantern. No lamp
switches off; no new pass, per-frame upload or extra noise evaluations.
The existing close-up replay now records21 frames at0.1s intervals from2–4s
for motion/brightness review instead of only two isolated images.

## Cornering grip and steering feedback, 22 October

The controller now uses one6.5m/s² lateral-acceleration limit for both the
speed-dependent steering lock and the final curvature safety clamp. This
replaces the previous4m/s² comfort-like limit: at100km/h the minimum radius
is now~119m, not193m. A50m bend at50km/h and100m bend at90km/h are reachable;
tighter bends still require braking. No road following or grip adhesion was
added. Existing8rad/s wheel slew,15ms mouse filter, acceleration/braking,
speed caps, gravity and four-contact suspension remain unchanged.

The normal driving HUD adds a compact L/R bar below km/h, with a centre tick
and amber marker for actual wheel angle divided by the current available
lock. It is read-only, bounded during speed changes, and disappears outside
driving. While driving it refreshes at most20Hz independently of mouse-look
or the debug overlay, instead of postponing its deadline every hidden-HUD
frame. It does not show only the latest mouse movement/target, which could
hide delayed countersteering. Controller tests cover bend radii, proportional
highway input, countersteering, indicator direction and speed/grip limits.

## Mouse driving, land HUD and lanterns, 22 October

Horizontal captured mouse motion steers a bounded virtual wheel; moving back
straightens it. A/D override and reset the mouse wheel. Hold right mouse for
independent free-look; release recentres the view. Vertical mouse motion and
Q/E can still look. Focus/capture loss resets mouse controls, and the existing
steering slew/speed-dependent lateral limits remain. The ordinary compact
compass HUD now shows actual car speed in km/h (R for reverse), even with the
debug overlay hidden. Sea settings require known dense-level open water and
are excluded while driving or inside a bore. Road glow radiance is60% of the
previous value; reflective paint still responds to lighting.

Fixtures are now small three-dimensional wall lanterns: dark iron cage/hood,
a wall bracket, tapered oil reservoir and a visible teardrop flame. Clear panes
are implicit rather than opaque glowing panels. The186 vertices per lantern
remain in the existing cached road batch, with no extra draw call. Each carries
a stable world-position seed shared by its flame and local light lists. Amber hues vary slightly redder;
smooth independent low-frequency value noise creates bounded80..102% flame
flutter, never blackout/strobe flicker. Fixed indoor exposure remains. All
variation runs in the existing road shader using the existing animation clock,
with no added light update pass, CPU light search or per-frame buffer upload.

A33x33 shoreline grid in the existing FFT tangent frame covers the spray pool.
Dense open-ocean ownership is sampled from resident CPU tiles only when the
32m grid window changes; incomplete data retries at most once a second. GPU
birth/integration/draw checks require all four neighbouring samples wet, giving
a conservative shore buffer and no imaginary inland wave sources. Source and
displaced birth positions are checked. Rain shading is unchanged. Camera-local
bubble/depth/audio checks now reuse physical open-ocean ownership rather than
treating a global virtual wave over dry ground as water around the eye.

## Tunnel lighting and enclosed views, 21 October

Network snapshots now bake warm wall fixtures every20m, alternating sides
(40m on each wall), at4.5m above the deck. Complete regional source alignments
station them before residency cropping, so a joined-mesh rebuild cannot slide
the lamps along the road. Regional phases can still reset at gateways. Missing
junction walls suppress their fixtures. Worker-built local light lists contain
at most24 nearby, topologically connected lamps per span, with smooth34m
cutoff and softened inverse-square falloff; disconnected bores do not light
through intervening rock. One immutable storage buffer joins the existing
cached road draw, not a new pass or per-frame light search. The vertex stride
is40bytes, with a flat light-group ID. Far covered decks retain independent
interior fill; detailed decks/liners use actual nearby lamps. Tunnel information
boards are backlit independently of sun/clouds. Legacy pilot fill is unchanged.

Automatic exposure eases to1 inside the existing bore field instead of metering
passing lamps, green paint or daylight beyond a mouth. Manual exposure controls
remain available. The user's bright/dark captures in
`manual/1792602903-1012989` show4x auto versus1x fixed exposure; the old lining
shader itself had no cloud-shadow input. This does not prove that every reported
in-motion brightness change had that same cause.

`tunnel_visibility.rs` builds a conservative overlap/portal graph from inflated
bore boxes in the same atomic snapshot as the mesh and collision/cut field.
Screen-rectangle beams overestimate possible sightlines to every actual or
unknown/cropped mouth. Visible mouths, invalid inputs, non-opaque/missing cells,
construction/query budgets and distances approaching the25km draw cutoff fail
open. Opaque lining/floors remain drawn independently of far deck LOD; treating
4km deck LOD as absent lining was an over-conservative first draft and prevented
the measured deep-view optimisation. This is not sparse ray sampling or a
camera-distance-only darkness switch.

Only a proven enclosed Final view skips terrain LOD/draw, forest/village siting
and draws, sky/atmosphere/sun/moon, ocean FFT/foam/spray and outdoor weather
rendering. Roads, lining, signs, controls, physics and postprocessing remain.
World/weather/ship time still advances; this does not freeze the outside world.
Debug modes, disabled road/cut subsystems and uncertain entrances keep the normal
path. `PLANET_TUNNEL_CULL=0` is the matched A/B override. Periodic
`planet::tunnel` logs report the classification; DEBUG
`planet::tunnel_visibility` explains conservative failures.

`road_network_tunnel_performance` reproduces a deep manual view with fixed
exposure. `road_network_tunnel_lighting` changes the sun at two manual positions
with automatic exposure constrained near1; captures wait inside pose holds so
floating-point capture scheduling cannot move the eye before comparison.

## Interior distance markers and driving, 21 October

`road_navigation` now stations paired ceiling boards every500m, anchored to a
canonical real exit so reversed edge ordering and regional gateways do not
restart the phase. Coincident gateway stations are de-duplicated. Each face
uses the true engineered distance in its own direction to a real exit, even
beyond residency; branched bores use the nearest directed exit. Values>=500m
round to500m (including half-kilometres above10km); shorter remainders keep10m
precision, never0. Headers cover the last75m near mouths. This replaces the
1km village-reassurance boards inside tunnels, not the village boards outside.
The existing single cached atlas batch is retained; navigation/layout stays
on the cancellable worker. Test board:20.5km inward,500m outward; next inward
marker20.0km. Both faces and the next marker opened in final1792592187-23337.

`driving.rs` is a deliberately small first-person test car. `PLANET_ROAD_DRIVE=1`
starts it at the usual road entrance; C enters/exits during normal exploration,
G returns to flight. W/S throttle/brake/reverse, A/D steer, Space brakes,
mouse/Q/E look, brackets set the speed cap (30m/s default,8m/s reverse).
The120Hz bounded controller samples four wheels on the same raster terrain/
exact road floors as walking. A critically damped suspension includes gravity,
limited compression, ballistic falls and low-pass pitch/roll; relative support
velocity follows sustained grades without compression-stop snapping. Stationary auto-hold prevents rolling away during startup. Bicycle
steering has a slew limit and4m/s^2 lateral-acceleration cap. There is no road
adhesion or artificial bob. Existing bore wall/roof sweeps remain; this is NOT
full tyre/chassis/parapet collision, an autopilot, or a rendered car. Unknown
terrain stops travel rather than integrating against an invented floor. No
extra routing, I/O or render pass is added to a movement frame.

The32s `road_network_driving` replay runs the actual controller, not interpolated
waypoints. Its first draft extrapolated a flat pose90m into an uphill section
below the deck and was rejected (1792591476-20904). The corrected real in-bore
pose passes65 finite spatial samples and four captures with1.391935..1.400057m
road clearance (final current-binary1792592907-26831, all4 opened). Five physics tests cover stillness, free fall,
roughness filtering,25% sustained grade and speed/steering/reverse/parked limits;
a signed spherical-step regression also pins actual reverse movement;
canonical/reversed/split gateway marker and actual sea-region checks pass.
Final actual keyboard run1792592740-26600 (all5 opened) confirms parked startup,
S reverse, D steering off pavement onto cut terrain beneath the legs, Space
braking, W forward, C walking and G flight. Grounded driving clearance~1.38..1.41m;
~3.80m during the off-road fall, then1.7m walking. Signed planet-frame motion
confirms actual reverse, rather than mistaking planet rotation for travel.
Full serial suite743 pass/same6 unrelated failures/66 ignored; all7 road Vulkan
checks pass. No FPS improvement claimed. See HANDOFF for final interactive
and cut-wall capture evidence.

## Single-owner excavation banks, 21 October

The photographed jagged moving patches came from wide cached offset bank
strips folding over bends and competing with the displaced terrain. Open road
banks now belong solely to the existing cut terrain; decks and court-specific
banks retain their own ownership masks, evaluated at actual raster height.
No vertex-cut bypass is used. Removed the redundant road bank generator; court
geometry, excavation profiles, roof cover,25% grades and collision floors are
unchanged. The production Vulkan apron test also retains high bank fragments
outside the pavement instead of discarding them for a removed mesh.

The blue portal wall is opaque terrain (water disabled gives the same wall;
categorical material/height probe has no ocean or below-zero ownership flags).
Excavated steep faces now use the actual triangle normal, exposed-earth rather
than shoreline coating, and no unrelated finer-than-mesh terrain relighting.
Untouched land retains its original smoothed normal and material path. This
addresses misleading cut-wall shading rather than changing the ocean colour.
The two manual poses and35cm camera movement are retained as
`road_network_cut_walls` and `road_network_cut_walls_exit`; the latter uses
`PLANET_ROAD_NETWORK_SITE=coastal_exit` to load its real opposite endpoint.

## Coastal tunnel mouths, 21 October

Undersea approaches now remain covered beneath low coastal ground, rather than
emerging at the first land-labelled sample. Canonical shoulder/125m/250m radial
shore probes impose the existing9m crown+5m roof cover, also on shared gateway
samples. Along long low beaches those caps extend through connected ground
until a dry emergence is possible. High ground (>=80m) avoids the extra coast
survey. Mouths below5m deck, or low mouths inside the sampled shoreline guard, are
rejected, never disguised with extra lining/water masks. Cliff decks>=40m are
already above the supported swell and need no low-shore setback. Land roofs
include diagonal taps and0.5m coastal cover margin. Infeasible fixed
anchors remain rejected rather than disconnected or moved. This conservative
rule also keeps low coastal cuts covered; it is not an exact coastline-distance
field or a guarantee that all planetary regions are feasible.

The selected start has moved~450m inland: deck+10.252m, eye+14.896m on its real
open approach; the opposite sea exit is+19.954m. Grade25% and vertical-rate
0.005/m limits remain unchanged. Opened final replay1792578084-1166275 shows dry land/sky out of the mouth
and a solid ceiling at the old water-exposed location. Actual flight/walking
1792577953-1165113 retain2.0/1.7m clearance. Both real sea endpoints, shared
gateway, legacy sea/mountain/village workers and7 Vulkan road checks pass.
Full serial suite735 pass/same6 unrelated failures,65 ignored. Replay: PLANET_ROAD_EXPERIMENT=1 PLANET_ROAD_NETWORK_SITE=longest_undersea
with --scenario road_network_coastal_portal. No per-frame surveying or FPS claim.
Old precompute geometry/statistics describe old engineering; the existing source
fingerprint invalidates them on a new run. User artifact folders are untouched.

## Previous interactive entrance start, 20 October

Ordinary road-experiment startup now uses the verified mouth of the largest
recorded regional undersea component (19.14km in PositiveX152/152). Tracing its
unbranched continuation through152/151 yields20.06km to the opposite real exit,
not merely a cache boundary. The site is pinned to the current survey; it does
not assert that the unfinished whole-planet survey has found its global maximum.
The camera stands3m inside the dry mouth, looking along the road. At that stage both
mouths were below sea level; an outside pose floated to the sea and was rejected.
Their exposed approaches were an engineering defect, not repaired by spawning
inside; the coastal phase above supersedes that workaround. Opened actual flight/walking captures1792536909-130927 retain2.0/1.7m
floor clearance. Legacy site overrides and default scenario starts are retained.
No per-frame work or routing/grade/cover change. See HANDOFF for tests/evidence.

## Shared fork topology before curves, 20 October

The photographed under-running branch was a duplicate horizontal trunk at
separate elevations, with upper-deck piers obstructing the lower carriageway.
Height-state routing supplied duplicate connected waypoint states. Before
curve fitting, `road_network_align::share_prefixes` now shares those outgoing
prefixes and merges their endpoint branches. Curves and heights then fit one
physical road; unrelated crossings remain separate. Terminal identities and
height-preference indexing survive compaction. This replaces the earlier
post-fillet prototype, without expanding flat aprons or relaxing engineering.
The original real-worker duplicate-span regression fails; the corrected worker
checks28 nearby spans and22 unobstructing piers. All four opened matched captures
`1792500947-44571` pass: one continuous trunk and an actual side parking access.
The trunk-only side image does not show the split itself. The extended six-view
replay `1792525990-99922` adds opened oblique captures5/6 of the actual junction,
showing both roads from opposite sides; capture6 is the clearest separation view.
Post-Claude86 focused road /7 road Vulkan checks pass; full724 pass/same6
unrelated failures. Dense-neighbour validation still times out at600s under the
active seven-thread precompute load; regional offline files exist for all244
villages, but a live residency/performance sign-off is NOT established. An
unloaded comparison is pending user approval to pause precompute; see HANDOFF.
No changes to frame rendering, presentation or the25% grade limit from the fork
fix; Claude's separate paint glow/widening remains intact.

## Road traces and viable tunnel preference, 20 October

**R** toggles traces, off by default; **PLANET_ROAD_LINES=1** starts on.
Resident engineered roads/bores are green. Above50km the whole planned backbone
is shown SCHEMATICALLY via region centres/gateways, red until prepared by the
streaming worker and green afterward for this session, independent of eviction.
Known occupancy-pruned links are omitted. This is not an exact planet-wide
village-access drawing. Far-side strokes are horizon-clipped; cached worker-built
geometry needs no frame-loop routing/layout and no overlay GPU draws/uploads
while off. `road_network_lines` captures100km and two whole-globe distances.

Regional planning favours viable tunnel shortcuts with a1.25 land-bore hint
rather than the conservative2.0. Global gateway topology and height-state search
keep their established conservative costs; sea/depth costs and all25% grades,
vertical curves, roof cover and minimum bore-extent checks remain unchanged.
A bounded preferred attempt (100,000 ground probes/256 no-progress cancellation
polls) falls back to the original conservative survey/engineering when it cannot
be built. Streaming and distant sign navigation share `prepare_local`, so an
optional preference does not discard a previously feasible regional network.
The controlled ridge geometry regression reduces road length19,281.6m ->
15,447.6m with4,740.7m covered, while checking grade and curvature limits.
This is fixture evidence, not a planet-wide tunnel inventory.

Nameplate posts remain20.75m from their20m-radius court centre, with the complete
board outside pavement and facing arriving traffic. The unsuccessful broader
shared-fork/extended-apron prototype from dc1ca92 is not retained: original
alignment/profiling/court siting is restored. The draft survives in that commit
and test-runs archives; the photographed unusable fork and general junction /
support-envelope work are still pending. Do not infer a global geometry audit
from the debug overlay or these real-worker checks.

## Narrow through-road junction seam, 19 October

The photographed junction (`manual/1792415387-1209990/capture-001.png`) had a
2.14degree through bend: skipping all branch miters left an18.67cm outer wedge.
Mutually unambiguous nearly opposite arms now share the same bounded miter as
ordinary smooth bends, while retaining branch-mouth metadata. Deck, parapet,
paint and walking sections agree; the side-road mouth is still clipped open.
This is a cached worker geometry correction, not a general acute-fork or
support-clearance solution. `road_network_join` is the matched camera replay
(use1024x640 for its reference-height60degree FOV).
The photographed-vector regression first fails with0.18672m seam separation.
All78 focused road tests and the scenario registry check now pass. Both opened
matched replay captures (`1792427441-5584`, baseline `1792416377-1222838`) show
continuous asphalt/parapet; the crack ROI has0 tan pixels (was110). No FPS claim.

## Bores versus detours, destinations and signs, 19 October

The first terrain-weighted planner now lets estimated tunnel cost compete with
surface-road length, climbing, cutting and supports. Previously only infeasible
alignments reached the tunnel-capable height search, leaving feasible but very
long detours untouched. `road_construction.rs` shares relative planning units
(not real currency) with height search; tunnels cost more per metre, but can
beat substantially longer switchbacks. Final grade25%, vertical-rate0.005/m,
curves, fixed gateways and sampled cover are still verified, not relaxed.

Village names derive deterministically from original sited directions. White
arrival nameplates, green junction approach boards, double-sided branch-mouth
name/distance fingers, and reassurance boards roughly every1km follow the road
network. Branch arrows point towards the same destination on both faces.
Traffic is left-hand (right-hand-drive cars); boards stand outside the lane or
are ceiling-mounted inside bores. Existing walking collision does not yet include
sign posts or a complete future vehicle envelope.

Displayed distances are to arrival courts along engineered roads, not straight
lines or coarse backbone costs. The worker can load neighbouring navigation
profiles beyond GPU residency and has no kilometre cutoff. Search/metadata budgets
and failed regional engineering can leave a destination unresolved; it is omitted,
never fabricated. This does not establish complete global road feasibility.
Tunnel headers use the same bore classification as the mesh, measuring to a real
exit across regional boundaries. Detected branched bores show NEXT EXIT rather
than pretending the entire multi-exit network has one length.

Roads publish before potentially lengthy sign planning. Cached signs are tagged
with their road generation and share its local frame; stale batches never attach
to a new road field. Font layout and route searches run on the existing worker,
not in movement/render frames. One bounded atlas/vertex draw renders boards and
glyphs; no new dependency or per-sign draw call. Initial signs can take time to
appear while the resident road neighbourhood is being engineered. Normal roads
flags enable signs too. `road_network_signs` is the inspection replay; optional
PLANET_ROAD_SIGNS_WAIT=1 waits for readiness before starting the replay clock.

## Empty regional destinations, 19 October

Coarse macro occupancy is conservative, not proof that a region contains a
village. The snowy manual road-to-nowhere was a gateway into an empty leaf.
Before local routing, gateway components are now checked lazily against actual
village siting, with memoized, iterative and cancellable worker traversal.
This is independent of residency and has no distance cutoff: empty branches
are not built, but empty transit regions linking real settlements remain.
The coarse backbone is retained for surveying; detailed local roads filter its
false-positive destinations. Synthetic and exact photographed-position tests
cover the distinction. `PLANET_ROAD_NETWORK_SITE=manual_snow` with scenario
`road_network_empty_spur` replays the photographed view (640x400 matches its
original internal aspect). No frame-loop routing or presentation change.

## Village entrance courts and resident updates, 19 October

Village approaches now end at **40m-diameter circular paved courts** for future
parking and turning. Their centres are 15m beyond the end of the approach, so
only the existing flat last5m overlap the pavement: the 25% grade limit and
vertical-curve cap are unchanged. A 64-sided cached mesh supplies the exact
walking floor. Terrain above the court is excavated, with finite circular cut
banks; where the ground falls away, a solid retaining skirt meets it rather
than leaving a floating sheet or adding parking-area legs. No parking-bay
markings, vehicle simulation, or retaining-wall collision are claimed yet.

Access siting reserves the court and bank footprint outside protected houses
and checks for water under the pavement. If the first dry siting cannot fit the
coupled road profile, the worker can retry two farther setbacks; it never moves
a fixed gateway, relaxes engineering limits, or publishes a partial network.
The normal road flag enables courts automatically. `road_network_parking` is a
fixed overview replay (`PLANET_ROAD_NETWORK_SITE=village` or the default mountain
site). No new per-frame routing or object search is introduced.

Courts are explicit point shapes in the existing CPU/GPU spatial index, with a
reserved metadata tag (not inferred from f32-coincident road vertices). They
share excavation, raster ownership, crown clearance and walking queries, using
the same buffers and unchanged32-probe/64-candidate limits. The existing cut
query returns pavement ownership too, so cached soil triangles cannot cross a
court; this adds no extra fragment lookup. Paved courts remain separate from
through-road junction engineering.

The latest manual flight `manual/1792407704-1125404` contains107 rejected road
updates. Both photographed positions were still using the old resident set:
the second was roughly41km beyond its last published camera neighbourhood.
Two topology defects are addressed: a gateway's first physical arm can lie
behind zero-length aliases (the canonical seam tangent must still be fitted),
and an unloaded/cropped third arm must not turn a genuine fork into a rejected
degree-two bend. Truly unrounded degree-two bends are still rejected. Exact
flight-position worker regressions and the existing village/mountain/undersea
resident checks cover this path; final validation is recorded in HANDOFF.

The earlier acute-fork and tunnel-side support defects are **not fixed by this
phase**. The measured tunnel site is a shared split with overlapping branches,
not an engineered overpass; baseline mesh inspection found four supports in the
lower carriageway. Proper shared horizontal/vertical fork geometry, curved
exterior junction boundaries and support clearance remain follow-up work.

## Horizontal alignments and edge protection, 19 October

Ordinary roads now follow actual tangent-continuous curves, not a square route
hidden by rounded end pads. Worker-side `road_network_align.rs` rounds each
non-branching chain before height fitting, preferring100m radius, sharing trim
space between adjacent bends, and rejecting radii below15m or reversals. Smaller
60/30/15m targets may be tried when the full coupled profile cannot fit; this is
not a universal100m minimum or a highway design-speed guarantee. Arc samples
are <=8 degrees/20m. Surveyed heights are resampled and the final curved route
is verified against the existing25% grade,0.005/m vertical-rate and cover caps.
Search carries terminal approach headings rather than allowing an initial or
final step to double back. Village accesses are leaf spurs when a fixed access
would otherwise force a through-road kink. Gateway inward tangents agree across
region/cube seams; coincident aliases retain graph connectivity.

Global shared cross-sections join the deck, parapets, paint, cut banks, far-AA
ribbons and bore lining. Geometric interval subtraction leaves exterior walls
right up to real mouths; it does not suppress an entire junction-adjacent span
or infer falls from excavation floor depth. Paint is removed at crossing mouths.
Dashes retain their chain phase and supports remain roughly40m apart even on
densely sampled curves. Walking queries intersect the rendered ribbon's two
triangles, rather than extrapolating a straight centre-line plane at miters.
The bounded hash index still has32 probes/64 local candidates; overflow still
rejects atomically. No per-frame routing/curve solve or presentation change.

Actual-height tunnel masking is also required: raster LOD skirts and chord sag
can put a fragment inside a bore while its interpolated authoring terrain height
is above it. The terrain bore test now uses stable camera-relative raster height
when network bores exist. This fixes blue terrain intruding ahead of intact
lining, not a missing roof mesh. Per-end junction caps/extensions do not pull a
neighbouring smooth join apart. The shared shader helper has a Vulkan regression.

This is NOT the finished vehicle/junction phase: branch surfaces remain joined
strips with mouth-aware walls/paint, not one rounded, crossfall-engineered T/Y
mesh or tunnel chamber. Circular bends are tangent-continuous but do not yet
ramp steering curvature like clothoids. Vehicle envelopes, turning-lane swept
paths and parapet/support collision remain to implement and validate.

### Verified horizontal-pass evidence

- `cargo check` and release build pass. Serial app suite: **721 passed, the
  same six unrelated failures,57 ignored**; failures remain the three optical
  zoom/FOV checks, scenario FOV range, shallow tint and water-counter assertion.
  All six production Vulkan road tests pass, including the new drawn-height
  bore fixture. All four real startup/residency checks pass: all244 villages
  in the village window,15 mountain,76 undersea. Village resident mesh24,233
  spans/854,202 vertices; initial39-village mesh432,756 vertices. No budgets
  were raised. Logs: `/tmp/stuff-road-horizontal-verified-{check,build,full,assets}.log`,
  `/tmp/stuff-road-horizontal-raster-height-gpu.log`.
- Original manual captures: `manual/1792390010-1078115`. Replay of their three
  camera positions: baseline `road_network_manual_corners/1792391361-1084572`,
  final `1792394427-1096308`. The initial diagnostic's first two poses were
  interpolated; that earlier artifact is NOT the accepted baseline. Final
  captures show real broad curves and continuous exterior parapets. True
  junctions are still distinguishable from ordinary bends; they are not signed
  off as final vehicle turning geometry.
- All19 captures across five final replays pass their assertions. Opened final
  manual002, undersea002, village overview003, mountain walk003 and pilot004:
  `road_network_walk/1792394458-1096406` (undersea),
  `road_network_overview/1792394492-1096506`,
  `road_network_walk/1792394542-1096640` (mountain),
  `road_surface_trial/1792394584-1096738` (unchanged legacy pilot).
  Walking clearance is1.7m. A straight walking replay is not a vehicle-route
  traversal or user confirmation of motion/presentation quality.
- Bore regression: curved pre-fix walk `1792393444-1092779` contains170/208/0
  sky-like blue pixels; actual-height-mask fix `1792394133-1094416` and final
  `1792394458-1096406` both contain0/0/0. RGB criterion: B>1.35R,
  B>1.12G,B>60 (8-bit). CPU mesh rays found intact front-facing lining at the
  affected pixels; ocean-off ablation left the defect, terrain-off removed it.
  Per-end lining extension alone did not fix the defect. Do not misidentify
  this as an ocean or missing roof-mesh fix. Temporary mesh probe removed.
- Recorded same-pose X11/Immediate640x360 synchronized manual replay medians:
  **71.465ms before /72.145ms after**, nine steady samples each (~14.0/~13.9fps).
  One pair, ~0.68ms/~1% difference: approximately flat, NOT an FPS improvement
  or a statistically strong benchmark. Network excavation cost remains a
  separate performance problem. Final village overview56.033ms (23 samples),
  undersea48.815ms, mountain63.816ms, pilot40.625ms; do not compare these to
  older runs with different automatically derived inspection poses.
  Binary hash, paths, assertions and timings:
  `test-runs/road-horizontal-curves-validation.json`. No GPU timestamps used.

## Vertical-curve follow-up, 18 October

The user's **25% maximum grade is unchanged**. `road_vertical.rs` additionally
limits the difference in signed grades between neighbouring segment midpoints
to **0.005 per metre**, using their actual, possibly unequal station spacing.
This is at most 10 percentage points over 20m, rather than a crest flipping
from +25% to -25% in one station. Flat-to-25% needs at least 50m; broad filtering
normally gives gentler transitions. The original 0.0005/m proposal could not
retain the existing short approaches/junctions; it is not the shipped limit.

The bounded solver runs only during generation, for the network and legacy
pilot. Flat mouths are collapsed into one height variable; terminal/gateway
anchors stay exact, and grade, pier and sampled roof-cover caps remain hard
constraints. Filtered heights are projected onto these constraints together
with the station-curvature constraints, not smoothed and then clipped back
into sharp ramps. Final physical grade/curvature are checked and curvature is
included in the survey JSON. Rejection/budget exhaustion stays explicit.

Only actual flat mouths get extra apron stations. Their length is the 5m pad
radius plus any acute-angle road-rectangle overlap, no greater than the old
20m extent. Ordinary straight lattice nodes retain the usual <=20m spacing.
Searched interior elevations now seed the profile rather than pinning every
10m-quantized lattice height; all incident branches still share one node
height. Meshes, excavation, forest clearing and walking use the resulting
stations together. Cancellation is checked during filtering/projection;
source identity includes the curve code. No new render pass, per-frame curve
work, shader dependency or presentation change is introduced.

Validation: **712 pass /same six unrelated failures /56 ignored** in the final
serial app suite; all five production Vulkan road tests pass; all four real
startup/residency checks retain all244 village,15 mountain and76 undersea-site
villages. The final village set has22,968 spans/853,380 vertices; its initial
39-village plan has4,093 spans/428,070 vertices. Maximum station spacing remains
20m, with extra points only at real mouths. Logs:
`/tmp/stuff-road-curves-{full-serial-final,gpu-final,assets-final-adaptive}.log`.

Opened village overview `road_network_overview/1792364126-1032938`, undersea
walk `road_network_walk/1792364087-1032779` and legacy pilot
`road_surface_trial/1792364222-1033260`: assertions pass. As before, the straight
undersea walk eventually faces a bend wall; this is not vehicle-route following
or finished tunnel-junction acceptance. A mountain replay interrupted by
Escape is not counted as a complete test; the completed rerun `road_network_walk/1792364327-1033561` passes, with capture003 opened.

Current X11/Immediate640x360 synchronized same-binary village pair,13 steady
samples each: **56.304ms visible /39.670ms roads+cuts hidden**. The remaining
render cost is ~16.63ms. No new render-time solve/pass is introduced, but this
is **not an FPS improvement claim**, nor a matched comparison with the old
inspection pose. Timings, binary hash and captures:
`test-runs/road-vertical-curves-validation.json`.

Next requested pass (not implemented here): continuous horizontal bends with
minimum radius/curvature transitions, shared seam headings, vehicle swept-path
checks and real unified junction meshes/markings. Tunnel intersections need one
open chamber, not overlapping tubes with obstructing internal faces. Refit and
resurvey final curved alignments before generating excavation/collision/trees;
never merely round the rendered strip. Junction geometry should be cached and
chunked, not new per-frame routing or one draw call per intersection.

## Live integration, 18 October

- `road_network_stream.rs`: one body-pinned CPU worker owns source I/O, the
  global backbone and a 32-region RAM plan cache. Requests/results are bounded
  latest-only slots with generation cancellation. A projected 3x3 neighbourhood
  includes adjacent cube faces. Plans use the finest available baked source;
  production village eligibility still uses the same pinned dense siting source
  as houses. No filesystem I/O or routing runs in a movement/render frame.
- `road_network_mesh.rs`: cached decks, real shared junction mouths, flat
  turn/terminal aprons, piers, sampled banks, portal shells and union-clipped
  tunnel lining. Village access ends outside the houses and meets natural
  ground; trunk routes retain protected cover under settlement cores. The
  router reserves flat-apron run length and broadens a failed terminal approach
  by at most three lattice rings without changing its height or grade limit.
- `road_network_render.rs`: shared near/far pipelines, camera-relative geometry,
  chunk culling and coalesced ranges. Near geometry is cached to 8 km, far decks
  to 25 km; the 10 km field covers the 8 km forest plus refresh margin. Banks
  own excavated raster fragments only inside a conservative geometry window.
  At most 1.5 million vertices are accepted in one resident snapshot.
- Open-only cells carry a fractional bore-presence flag alongside their exact
  integer reference count; no-bore regions/cells bypass the expensive tunnel
  candidate walk. This retains the existing buffer size and query bounds.
- Vertex staging is capped at 1 MiB/frame; the final fixed field is about
  2.62 MB. Mesh, CPU collision and GPU excavation are published together after
  vertex upload, never as a partially connected set. Initial startup builds one
  complete region (and any immediately visible sea-gateway continuation); the
  worker then fills the complete neighbouring set.
- CPU/GPU excavation also masks full trees and CPU proxy cards, with a 25 m
  crown/shoulder clearance plus actual open banks. Forest caches invalidate
  when the resident road field changes. Covered bores do not clear the forest
  above them. Existing pilot roads use the same tree-exclusion entry point.
- Junction end faces close the outside corner of intersecting convex bores;
  the same union shader removes their internal portions, retaining open turns.
  Open-ended tubes alone leaked sky at the real sea gateway. Bubble and sound
  depth now share the dry-air exclusion as well as rain and camera corrections.
- Deck-aware walking and swept bore wall/roof collision retain terrain above
  the roof. Covered sea bores suppress swimmer/sea-surface corrections and
  underwater eye fog/bubbles; they remain dry air below the sea.
- No road work starts in ordinary non-road launches. Explicit
  `PLANET_ROAD_NETWORK=0` retains the old interactive 30 km pilot; existing
  pilot replays select it automatically. Network presets: default mountain,
  `PLANET_ROAD_NETWORK_SITE=village` and `undersea`.
- The user confirmed the road jitter workaround works. Automatic synchronized
  presentation and CPU/GPU overlap are retained unchanged.

Current asset-backed checks build complete nine-region sets: **244 villages**
at the village start, **15** in mountains, **76** at the real undersea cube-face
seam. All four startup/residency instruments pass, with bounded meshes and
positive bore air/deck clearance. Logs:
`/tmp/stuff-road-network-final-assets-5.log`. The village set contains 22,023
spans / 763,830 vertices; the underwater set contains 32 covered spans. These
are tested local windows of the global connected topology, not a world bake.

Runtime cache identity includes the source manifest and engineering code; it
is process-local, not a persistent disk mesh cache. Source tiles (64 fine tiles)
and village siting cells (1024) are bounded. Search/index/mesh failures reject
explicitly and hold the previous complete resident set. Missing regions are
not silently declared connected. Continuous unsampled cover, all planetary
engineering cases and ray-renderer exterior terrain are not signed off.

### Validation and measured cost of the live network

- Standard app suite: **706 pass, the same six unrelated failures, 56 ignored**.
  The failures remain optical-zoom/FOV, shallow-water tint and water-counter
  assertions; none is a new road/forest failure. Final logs:
  `/tmp/stuff-road-network-release-{full,build}.log` and
  `/tmp/stuff-road-network-far-camera-plane-green.log`.
- Five production Vulkan road tests pass: 26,460 network samples (max height
  difference 0.260 m), 77 pilot samples (0.349 m), sub-pixel coverage/depth,
  crown/bank/bore tree exclusion, and shallow-apron raster ownership.
- Opened village overview/walk, mountain walk and undersea cube-seam junction
  captures pass scenario assertions. Walking clearance is 1.7 m. Shallow apron
  regression: `road_network_walk/1792356584-985780` versus
  `1792356932-989959`; foreground road ROI green intrusions **4997 -> 0**.
  Undersea before/after: `1792356108-980175` versus `1792356449-984313`;
  blue sky/bubble pixels **11329/34516/351 -> 0/0/0**. These are opened local
  checks, not visual sign-off on every generated planetary route.
- Final visibility regression: batches now split at graph-edge spatial jumps
  and cached near/far boundaries; LOD uses the nearest bounding-sphere distance,
  not just its centre. Nearby decks cannot accidentally become far ribbons.
  Far AA falls back to the real physical quad near the eye plane, rather than
  using a singular perspective derivative. The Vulkan regression reproduces
  an entirely offscreen road 5 km below the eye painting black polygons (red),
  then verifies four near-plane depths remain clear (green), with all original
  24 sub-pixel phase/angle/depth checks retained. `PLANET_DISABLE=road_far` is
  the network far-pass diagnostic ablation. Opened fixed undersea capture:
  `road_network_walk/1792359557-1001663`; no black polygons or sky/bubbles.
- The original pilot still runs via its existing scenario names. All seven
  `road_surface_trial/1792359726-1002200` captures pass; capture 004 was opened.
  Its run starts no network worker.
- Two initial matched X11/Immediate 640x360 synchronized pairs at the village
  overview: **56.027/56.061 ms visible**, **39.543/39.271 ms with
  `PLANET_DISABLE=roads,road_cuts`**. Draw-only ablation is **55.70 ms**:
  excavation queries, not mesh draw calls, dominate this cost.
  `test-runs/road-network-live-before-lookup-flags.json` retains the baseline.
- Bore-presence flags reduce the initial visible trial to **54.35 ms**; its
  final PNG is byte-identical to both original visible runs. This is a small
  improvement, **not a claim that network FPS is solved**. Remaining overhead
  is substantial. Generation/I/O remains off-frame and uploads are bounded.
- Final corrected-visibility build, same binary/pose/flags, 13 steady samples
  each: **54.572 ms visible / 39.492 ms roads+cuts hidden** (~18.3 / 25.3 FPS).
  The remaining network rendering cost is **~15.08 ms**, not solved. Evidence:
  `test-runs/road-network-release-verification.json`; opened final overview
  `road_network_overview/1792359629-1001910`. Do not substitute headless planning
  timings or the cheap mesh draw ablation for this full rendering cost.
- Rejected experiments: a per-cell minimum-height cache and 1cm-centreline
  span coalescing made no useful timing difference and were removed. Skipping
  supposedly redundant terrain vertex deformation reached ~47 ms, but exposed
  **103 new blue sky pixels in distant cuts**: fine-ground bank meshes do not
  always cover every discarded coarse raster facet. That prototype, including
  its CPU collision changes, was reverted. Never repeat it without proving
  complete raster bank coverage and opened close/distant transition captures.

The remaining sections retain the earlier groundwork and historical evidence.

## Implemented

- `road_network.rs`: a deterministic terrain-cost minimum spanning tree on
  adjacent level-8 cube cells, including cross-face neighbours. All six faces
  contain 393,216 survey cells. A shared canonical edge direction identifies
  each gateway; averaging unit cell-centre vectors would not lie on that edge.
- Conservative village occupancy uses the **existing six hashed candidates per
  level-10 village cell**, plus the existing biome/beach prerequisites. A real
  accepted village cannot be excluded by those prerequisites. False positives
  deliberately retain roads rather than strand a village. Transit-only leaves
  are pruned; empty cells may still carry necessary trunk connections.
- Local plans enumerate every village cell inside their region, using the
  **same footprint/slope/ocean eligibility code as the live houses and beams**.
  Camera altitude, render distance, visible-house caps and resident render LOD
  do not decide whether a settlement belongs in the network.
- One multi-source Dijkstra over a 129x129 terrain lattice supplies candidate
  connections. A terminal MST, union of shared paths, cycle removal and spur
  pruning produce one shared local tree. Terminals retain their exact
  directions, including nearby/coincident villages; they are not silently
  snapped together. Nodes of degree >=3 are actual graph junctions. Alternating
  single diagonals avoid two independent diagonals crossing inside one square.
- Regional gateways form one connected global graph after pruning. This is a
  structural connectivity guarantee for the survey algorithm, **not a claim
  that every village worldwide has already been enumerated and engineered**.
  Fine planning is intended to be generated/cached region by region.
- Ocean is expensive but traversable in the cost graph. Coarse water links and
  detailed water segments explicitly require undersea tunnels. The survey
  samples negative bathymetry, not the visible sea-level shell.
- `road_network_profile.rs`: 20 m subdivision and a graph-wide Lipschitz
  extension fit one elevation per shared node. Independent per-edge fits would
  produce vertically disconnected junctions. Village approaches and gateways
  have fixed elevations; grade, bounded deck clearance and sampled seabed
  cover constrain the rest. Physical grade is capped at 24.9%, retaining a small
  margin below the user-requested 25%; the propagation uses a conservative lower physical radius,
  including underwater depth. Land deck clearance is capped at 32 m.
- Undersea profiles retain a 9 m crown plus at least 5 m sampled roof cover,
  using a centre and four 8 m radial stencil samples. Gateway anchors use that
  same deterministic stencil on both sides of a boundary. **This is not yet a
  continuous bore-side/portal cover test.** An infeasible alignment returns
  `needs_reroute`; it is not steepened, given giant supports, or omitted.
- `road_network_survey.rs`: CPU-only baked-terrain source and a headless CLI.
  The exact production village sampler was factored out of `terrain.rs`; no
  placement rule, constant or camera-dependent village behaviour was changed.

- `road_network_route.rs`: bounded height-aware A* reroutes alignments rejected
  by the initial shared profile. States include lattice position, height and
  incoming heading; new branches join a trunk at exactly its existing height.
  Terminal/gateway anchors do not move, and sampled spans retain grade,
  clearance and undersea-cover constraints. A sea-aware planar lower-bound
  guide avoids exhausting the search on cheap-looking land before a dive.
  Search/memory limits reject explicitly; a failed branch is never accepted as
  a complete network. The weighted search is not a global optimality claim.

- `road_network_field.rs` and network mode in `road_cuts.wgsl`: bounded hashed
  excavation for arbitrary orientations and branching bore unions, with CPU and
  production GPU lookups. A query reads one local cell, at most 32 hash probes
  and 64 candidate segments. Capacity/probe overflow rejects the entire field;
  it never silently discards a road. Finite banks blend out before their index
  footprint ends, avoiding a sharp edge under radius-sized f32 rounding.
  `TerrainRenderer::set_network_road_cuts` is the upload/CPU-query hook.
  The live streamer now calls this hook atomically with mesh publication; the
  pilot retains its original corridor field.

The MST minimises a sampled cost proxy **on the candidate cell/lattice graph**,
not worldwide engineering cost or a globally optimal Steiner tree. The coarse
MST initially spans all cells, then is pruned. Conservative occupancy can keep
unused land roads. Cost weights and geometric smoothing need visual review.

## Run the survey

From the repository root, use a **new** output filename:

```sh
cargo run --release -p planet-app -- --road-network-survey test-runs/road-network-new.json
```

`--outmap PATH` selects another bake. This command does not open a window,
create a GPU device or start audio. It refuses to overwrite an existing output.
It cannot be combined with `--scenario`. The JSON is diagnostic evidence,
**not a versioned runtime road cache or a complete baked world mesh**.
It contains global counts and exact local topology/profile data for the known
village region, the current mountain pilot region, and both sides of a sampled
real undersea gateway. A profile can be feasible while its final tunnel/portal
geometry is still unvalidated. The diagnostic CLI source is pinned to dense baked terrain with close-range
detail. The live worker instead resolves the finest available baked ancestor;
these are deliberately distinct diagnostics, not interchangeable mesh caches.

## Evidence, 18 October

Current 25% survey: `test-runs/road-network-25-grade-2026-10-18.json`,
console `/tmp/stuff-network-25-cli.log`: all four detailed samples are feasible,
including all 39 villages at PositiveX (43,1) and both sides of the real undersea
link. Maximum sampled physical grade 24.9%; total headless time 9.30 s. These
four cases do not establish feasibility of every planetary region.
Six height-aware rerouting tests cover longer climbs, shared branch heights,
coincident terminals, island dives, explicit budgets and exact seam anchors.
Pilot regression checks a dry ~29.993 km mountain route, ~6.414 km ground relief,
five covered tunnels totalling ~1.567 km, with the 25% cap checked physically.
Opened pilot captures: `road_viaduct_overview/1792339098-938892`,
`road_tunnel_trial/1792339309-939529`, `road_surface_trial/1792339344-939615`.
The first overview views the roof rather than exposing the buried road.

Network excavation validation: six CPU tests; 26,460 production Vulkan samples
across three planet-frame orientations (maximum cut-height difference 0.260 m),
plus the existing 77-sample pilot GPU regression (maximum 0.349 m). All 42 shader
checks pass. Full app suite: 694 pass / same six unrelated failures / 50 ignored.
Release build passes. Logs `/tmp/stuff-road-network-field-{tests,gpu,pilot-gpu,shaders,full-tests,build}.log`.
This tests cut heights and bore sign/roof retention, not meshed tunnel junctions,
walking, continuous source cover or actual undersea rendering.

Matched pilot regression (network mode remains inactive): two X11/Immediate
640x360 synchronized pairs,12 non-capture samples/run:41.678/41.693ms before
versus41.995/42.001ms after (~0.31ms plumbing cost,about0.75%;~24fps at this
heavier mountain site). All four final captures are byte-identical. Evidence:
`test-runs/road-network-field-pilot-timings.json`. This is NOT a measurement of
rendered planetary network cost.

Earlier 10% evidence (historical):

Verified survey: `test-runs/road-network-survey-final-2026-10-18.json`;
console/resources: `/tmp/stuff-network-cli-final.log`.

- All 393,216 cells surveyed; 106,296 potentially inhabited; 143,802 retained
  cells and 143,801 trunk links after pruning, of which 2,666 **coarse links**
  require undersea routing. These are not 2,666 finished portal-to-portal bores.
- About 6.68 s wall time and 277 MiB peak RSS in the standalone process. This
  measures the coarse global survey and four detailed samples, not a full
  world bake or in-game streaming/FPS cost.
- PositiveX (43,1): all 39 villages, 10 junctions, 68.71 km shared alignment.
  Its proposed profile is **rejected**: it still needs engineering-led rerouting.
- PositiveZ (217,223), mountain pilot region: 6 villages, 1 junction,
  35.64 km; sampled profile feasible below 9.9% physical grade, max deck
  clearance ~16.3 m. This does not relocate or replace the pilot road.
- NegativeX (0,184): proposed undersea approach also needs rerouting;
  NegativeZ (255,184): sampled undersea alignment feasible. No visible tunnel
  or traversal claim is made for either.
- Twelve focused tests pass when including the asset-backed instrument. Cube
  seam reciprocity/gateways, determinism, shared-trunk savings, exact terminals,
  empty/coincident cases, conservative pruning, water routing, grade rejection,
  shared heights and sampled seabed cover are covered. The real-world occupancy
  instrument retained 681 accepted villages and checked 104 rejected regions.
- Village test filter: 24 passed / 2 ignored. Full standard app suite: 675
  passed / the same six unrelated failures / 47 ignored. The failures are the
  existing optical-zoom/FOV, shallow-water tint and water-counter assertions
  already listed in HANDOFF; this phase does not repair them.

The final release CLI also passed overwrite refusal (existing report hash unchanged)
and placeholder-source rejection. Binary SHA256:
`fe99ac2dabc95ce390eee4594b79082ebce26a163a7406ae46df5988c19c4bc8`.

## Earlier implementation checklist (superseded by live integration above)

1. Height-aware rerouting is implemented and tested. Validate more real regions,
   including budget failures, before claiming the whole planet is engineered.
   The requested maximum is now 25%; never relax it or move terminal heights to
   disguise an infeasible connection.
2. Reconcile dense survey heights with finest baked source and the drawn
   terrain/detail ladder. Smooth horizontal/vertical curves while retaining
   shared junction and cross-region endpoint constraints. Add village-edge
   approaches rather than running parapets through houses at their centres.
3. Add cached near/far deck, supports, open junction mouths and tunnel/cavern
   geometry. Existing `road.rs`/`road_tunnel.rs` mesh conventions and
   `road_render.rs` coverage-AA pipeline should be reused; do not duplicate
   pipelines per road. A subterranean junction cannot be several closed
   overlapping tunnel liners.
4. Feed actual engineered resident segments into the implemented bounded
   network field, atomically with their meshes. Add matching deck/wall collision;
   the current field only supplies terrain heights and bore air queries. Never
   loop through the global network per pixel.
5. Undersea integration must exclude the tunnel interior from sea/water fog,
   bathymetry and underwater-camera corrections; simply drawing a liner below
   water is not sufficient. Preserve both the seabed and the sea above its roof.
6. Add bounded background generation, source/version-keyed caches, seam-safe
   neighbour requests, cancellation and capped per-frame uploads. Only nearby
   regions should be resident. The present CLI is not that streaming system.
7. Inspect junction/portal/roof/undersea/walking captures and real transitions;
   then paired X11/Immediate timings with identical poses, no GPU timestamps on
   the M1000M. Do not treat topology tests or a feasible sampled profile as
   rendered acceptance.

### Enclosed-view validation, 21 October

At the real deep manual pose, X11/Immediate1280x800 cull off/on final runs
1792608165-1046854 /1792608188-1046896 measure47.131 /6.927ms median
(~21 /144FPS), with pixel-identical images. The earlier640x400 pair measures
40.836 /6.537ms, also identical. Both held-pose sun pairs in lighting replay
1792607754-1046149 are identical at auto exposure1. All3 coastal pairs match
with culling disabled/enabled; the visible sky is retained, while a closed-roof
view enables culling. Driving1792607840-1046329 retains65 finite samples and
1.391935..1.400057m clearance; all4 captures opened,65/65 enclosed samples.

Regular tests:750 pass, same6 unrelated failures,66ignored. Four conservative
visibility tests (including near-wall clipping), local-light/phase/exposure and
vertex-layout checks,7 Vulkan checks and the real mountain/undersea neighbour
checks pass. The optional dense244-village neighbour test was stopped after
more than3minutes; its earlier planning timeout remains unresolved, not signed
off by this lighting/rendering phase. No routing/grade/cover or presentation
policy changes are included.

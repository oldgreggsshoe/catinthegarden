# Civic buildings and coastal outposts — 30 October 2026

## Scope

Added the physical destinations from the traveller/trade proposal: timber inns,
carpenter workshops, market stalls, harbour warehouses/piers and lighthouses.
Normal play enables them by default; `PLANET_CIVIC_BUILDINGS=0` is the A/B override.
No dependency, texture asset, outmap bake or road/precompute fingerprint changed.
Claude's player cart, driving seat and hourglass engine changes are retained.

This is the building/world-layout layer, **not** shop interactions, interiors,
trading goods, named NPCs, a photo journal, ship docking, new road links or netcode.

## Placement

- Three central existing village footprints become a market, inn and workshop.
  Assignment ranks the full deterministic layout before visibility filtering, so
  flying around cannot change a building's role. Civic fronts face the village
  centre. The village sites, house scatter, occupied cells and building count
  remain unchanged; this does not reroute existing roads.
- Ordinary village placement intentionally avoids beaches. Ports therefore have a
  separate sparse, deterministic L10 coastal survey: a quarter of cells are
  eligible, and a valid shore must actually cross zero-height ocean, not a lake
  edge or an ocean-coloured cliff. Dry terminal approaches, shallow T-landings
  (1–24m seabed depth) and sufficiently flat footprints are checked. Nearby high,
  dry headlands are searched for the lighthouse.
- The port warehouse and each optional service/tower also pass the actual raster
  triangle footprint check, not just the smoother height survey. This matters:
  before terrain residency settled, the initial port contact read roughly 395m
  instead of its final 4.79m. Up to six one-second residency retries are bounded
  to nearby missing port components; otherwise rebuilds retain the existing 400m
  camera movement threshold. Missing survey data is not permanently rejected at
  the initial centre-data check.
- Ports are sparse and not guaranteed at every beach. The added coastal services
  form a small waterfront outpost, but it is **not yet a road-network village**.
  Road connection, cargo transfer and starter-town selection remain future work.

## Models, paths and performance bounds

- Inn: two storeys, chimney, window trim, porch and benches. Workshop: open timber
  bay, bench, sawhorses and lumber stacks. Market: striped canvas, counters and
  crates. Harbour: warehouse, 6m-wide timber pier, T-head, piles, rails and crane.
  Lighthouse: tapered timber tower, stone foot, gallery and nighttime amber beacon.
- Six cached mesh kinds, each with near/far geometry; the existing 450m detail
  threshold and 4,096 instance cap remain. One pass buckets instances into at most
  twelve draws, using retained vectors. No per-frame mesh generation, per-building
  draw calls or render-thread text layout. Kind occupies an existing padding slot;
  house vertex/instance buffer layouts remain unchanged.
- Existing house material tags 0–6 retain their shading. New tags are beacon glass
  (7), canvas (8) and pier planks (9). Only the beacon emits after sunset; there is
  no new ocean, spotlight, shadow or volumetric pass.
- Building labels use the existing worker-built wooden sign/font atlas. Existing
  paths link public doors to parking; harbour paths meet the warehouse front door,
  not a point in water. Civic clearings reuse ordinary-width overlapping capsules,
  so the terrain/tree masks and path atlas retain their original bounds. Optional
  clearings are dropped if they would overflow a budget; actual access paths and
  signs are retained. No larger GPU buffers.
- Coast proposals are cached over the local 5x5-cell ring. Pier floor contacts use
  a cached short list, not a scan of all houses. The CPU tangent-plane floor agrees
  with the drawn pier and T-head, and will not clamp someone up from below.
  **Pier rails, building walls and interiors do not gain full collision physics.**

## Validation

- `cargo check -p planet-app` and release build pass; the two existing dead-code
  warnings remain. Focused village checks: **44 passed, 10 ignored**. Full app suite: **836 passed, 6 known failures, 79 ignored**.
  The failures remain optical zoom (3), scenario FOV range (1), shallow-turquoise
  shader expectation (1), flooded-ship HUD (1); no claimed green full suite.
- Regression coverage: stable distinct civic roles, bounded finite meshes/normals,
  roof/eaves above the inn's upper storey, dry sea/lake/cliff placement, pier/T-head
  and below-pier contacts, compatible forecourt capsules, budget fallback and
  negative-control building-presence assertions. Scenario registry loads all 166.
- Asset-backed ignored coast survey passes and finds three real suitable ports
  and headlands near PositiveX L10 (367,805), (360,817) and (360,812).
- Final `village_services`: **PASS**, all three required kinds; opened all three
  captures in `test-runs/village_services/1793394446-209355`.
- Final `coastal_services`: **PASS**, harbour/lighthouse required, all five civic
  kinds observed; opened all four captures in
  `test-runs/coastal_services/1793394482-209638`. Pier eye clearance is 1.75m.
- Earlier diagnostics recorded 778/778 door connections across 40 village groups,
  4,295 path segments, 147 signboards/gates and 3,640/4,096 atlas tiles. Coast:
  5/5 doors, one group, 30 segments, five boards and 35 atlas tiles.
- First previews exposed an incorrectly stretched inn roof and an over-wide
  clearing capsule that rejected the path worker; both are fixed and covered by
  regression tests. Early failed captures remain in the ignored evidence folder.
  The final coastal captures also show a pre-existing sky/ocean shadow wedge;
  this task does not claim to fix that unrelated rendering defect.

Evidence/logs and paired timings: `test-runs/civic-2026-10-30/` (ignored artifacts).
The inspection scenes intentionally move between buildings; do not use those
moving views for off/on FPS comparisons. `coastal_services_performance` is the
fixed-camera port scene; `village_pov` is the established fixed village view.

### Matched timings

Quadro M1000M, High, X11/Immediate, requested 1280x800 (actual captured surface
1280x647), moon disabled, no roads/traffic, same final binary. Four runs per fixed
view in **off/on/on/off** order. Frame pacing excludes the first 120 frames,
captures and their following frames. Values below are per-run frame-time medians
in milliseconds, not startup or isolated GPU-pass estimates.

| Fixed view | Off 1 | On 1 | On 2 | Off 2 | Mean of on medians minus off medians |
|---|---:|---:|---:|---:|---:|
| village_pov | 74.599 | 74.834 | 74.833 | 74.671 | +0.199 ms |
| coastal_services_performance | 64.267 | 65.590 | 65.575 | 64.190 | +1.354 ms |

Average frame-time means change by +0.110ms and +1.340ms respectively. Village
cost is small relative to scene/frame noise; the port's roughly 2.1% increase is
repeatable and includes buildings plus paths/signs/terrain masks. This is **not**
an FPS improvement or an FPS-neutral claim. The off/on pictures intentionally
change. Dynamic inspection screenshots are visual checks, not timing evidence.
Raw rows, run paths and pacing summaries are in `final-runs.json` and `perf-*.pacing`
under the evidence directory; all ten final functional/performance runs passed.

## Multiplayer direction — design only

A persistent shared planet fits this game. Prove a small shared town before
promising an MMO player count: start a handful of players together around a
public harbour, inn, market and road access, leaving most wilderness untouched.
Starter facilities/access should be protected from claims; allow modest nearby
plots and expand settlement boundaries as demand grows, rather than scattering
new arrivals across the planet or letting claims block common routes.

Plan for a server-authoritative world (ownership, cargo, vehicles and shared
clock), interest-based nearby replication, and sleeping/lightweight remote
settlements. Client-local time acceleration cannot independently retime shared
traffic/daylight. Keep rendering client-side; headless simulation/storage is a
separate architectural step. Capacity, sharding and hosting costs need measured
multiplayer prototypes, not assumptions based on the planet's area.

Authoritative-server terminology follows the official [networking overview](https://dev.epicgames.com/documentation/unreal-engine/networking-overview-for-unreal-engine),
not a proposal to replace Rust/wgpu or adopt Unreal. No networking, account,
persistence or land-claim system was implemented in this building pass.

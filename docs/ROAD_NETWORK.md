# Planet-wide village roads — implementation status

## Scope and current limit

The requested network covers the **whole planet**, with shared roads and real
junctions rather than every village connected separately to every other village.
**Island connections must be tunnels below the seabed, not ferries or surface
bridges.** The existing elevated deck, terrain-led alignment, <=25% physical
grade and hillside tunnels remain the intended construction rules.

This first phase implements a **headless topology survey and shared profile
feasibility checks**. It is **not a visible, walkable road network yet**. Normal
rendering and `PLANET_ROAD_EXPERIMENT=1` still use the existing single pilot.
No network survey, additional geometry or GPU upload runs in a movement frame.
Do not claim an FPS result for road-network rendering before it exists.

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
  **No live network producer calls this hook yet.** The pilot retains its own
  original corridor field; this is tested integration plumbing, not live roads.

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
geometry is still unvalidated. The engineering source is pinned to dense baked
terrain with close-range detail; sparse-source and actual rendered-LOD grounding
must be reconciled before meshing.

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

## Next implementation steps — not yet done

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

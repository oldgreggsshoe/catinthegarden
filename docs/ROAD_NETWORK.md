# Planet-wide village roads — implementation status

## Scope and current limit

The requested network covers the **whole planet**, with shared roads and real
junctions rather than every village connected separately to every other village.
**Island connections must be tunnels below the seabed, not ferries or surface
bridges.** The existing elevated deck, terrain-led alignment, <=10% physical
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
  cover constrain the rest. Physical grade is capped at 9.9%, retaining a small
  margin below 10%; the propagation uses a conservative lower physical radius,
  including underwater depth. Land deck clearance is capped at 32 m.
- Undersea profiles retain a 9 m crown plus at least 5 m sampled roof cover,
  using a centre and four 8 m radial stencil samples. Gateway anchors use that
  same deterministic stencil on both sides of a boundary. **This is not yet a
  continuous bore-side/portal cover test.** An infeasible alignment returns
  `needs_reroute`; it is not steepened, given giant supports, or omitted.
- `road_network_survey.rs`: CPU-only baked-terrain source and a headless CLI.
  The exact production village sampler was factored out of `terrain.rs`; no
  placement rule, constant or camera-dependent village behaviour was changed.

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

1. Feed profile rejection back into alignment search. A cheap 2D route can be
   too short to climb, descend or dive beneath a coast within 10%. Do not solve
   that by relaxing grade, moving settlement heights, or silently dropping a
   connection. A height-aware route search/longer tunnel approach is required;
   independently refitting edge endpoints cannot fix it.
2. Reconcile dense survey heights with finest baked source and the drawn
   terrain/detail ladder. Smooth horizontal/vertical curves while retaining
   shared junction and cross-region endpoint constraints. Add village-edge
   approaches rather than running parapets through houses at their centres.
3. Add cached near/far deck, supports, open junction mouths and tunnel/cavern
   geometry. Existing `road.rs`/`road_tunnel.rs` mesh conventions and
   `road_render.rs` coverage-AA pipeline should be reused; do not duplicate
   pipelines per road. A subterranean junction cannot be several closed
   overlapping tunnel liners.
4. Replace the pilot-only monotonic-y cut buffer with a bounded spatial index
   for active network segments. CPU/GPU cuts, bore voids and collision must
   share the same field. Never loop through the global network per pixel.
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

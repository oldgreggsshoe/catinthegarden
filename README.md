# planet

A planet renderer in Rust and WGSL: a 4,000 km-radius cube-sphere with baked
macro terrain, quadtree LOD, analytic atmospheric scattering, Gerstner-wave
ocean, and a raster renderer.

Not a game engine. It is a rendering testbed built around a scenario harness,
so every visual claim is reproducible from a command rather than a screenshot.

## Layout

| Crate | What it is |
|---|---|
| `crates/app` | The renderer and its scenario harness. The binary you run. |
| `crates/baker` | Offline terrain pipeline. Generates the "outmap" the app streams. |
| `crates/coretypes` | Constants and tile types shared by both. |

`AGENTS.md` holds the architecture decisions and phase plan; `docs/HANDOFF.md`
tracks current renderer state. Read those before changing rendering behaviour —
much of what looks arbitrary in the shaders is load-bearing and explained there.

## Running

Requires a GPU with Vulkan/Metal/DX12 support via `wgpu`, and a baked outmap at
`assets/outmaps/test-planet` (found from the working directory or any parent).

```sh
cargo run --release -p planet-app                    # interactive, fullscreen
cargo run --release -p planet-app -- --terrain placeholder   # no outmap needed
```

Use `--release` for interactive play and performance measurements. A plain
`cargo run` builds unoptimised debug code and is not representative of renderer
performance.

Without an outmap the app falls back to procedural placeholder height
automatically, so a fresh clone runs before it bakes.

### Linux presentation workaround

On this project's Quadro M1000M / KDE Wayland machine, matched fullscreen
640x360 runs measured about 60 ms between frames with native Wayland versus
33 ms using X11/XWayland. The extra wait is in the native presentation path;
this is not a debug-build or internal-resolution mismatch. To use the verified
faster path on this machine, with an X11 display available:

```sh
env -u WAYLAND_DISPLAY cargo run --release -p planet-app
```

This leaves resolution, shaders and quality settings unchanged. It is a
hardware/session-specific workaround, not a global change of backend defaults.
Switching native Wayland from FIFO to Mailbox did not remove the stall.

### Flags

| Flag | Effect |
|---|---|
| `--scenario NAME` | Run a scripted camera path from `crates/app/scenarios/`, capture screenshots, exit |
| `--outmap PATH` | Use a specific baked outmap |
| `--terrain placeholder\|outmap` | Choose the terrain source |
| `--profile-render` | Record CPU-side frame-stage timings, without GPU timestamp queries |
| `--vertical-fov-degrees N` | Override the vertical field of view |

GPU timestamp queries require the separate `--profile-gpu` flag. Do not use
that flag on the Quadro M1000M: timestamp queries can hang its driver. The
CPU-only `--profile-render` path does not request them.

For an interactive timing log, run `cargo run --release -p planet-app -- --profile-render`.
Logs are written to `test-runs/manual/<timestamp>-<pid>/log.jsonl`. Startup
records whether debug assertions are enabled; startup and resize entries show
internal and display resolutions.

### Planet-wide village road network (planning phase)

The new headless planner builds a shared planet-wide backbone and local village
junction trees, with island links designated as **undersea tunnels**. It also
checks shared elevations, physical grade and sampled seabed cover. This is
**not yet a visible/drivable network**: height-aware rerouting is implemented,
but mesh/cut/collision and background-streaming integration remain outstanding. See [implementation status](docs/ROAD_NETWORK.md).

Run the CPU-only survey with a new output filename (no display/GPU required):

```sh
cargo run --release -p planet-app -- --road-network-survey test-runs/road-network-new.json
```

### Planet-wide village road network

`PLANET_ROAD_EXPERIMENT=1` now starts the **streamed village network**. Every
accepted village belongs to the global graph, independently of visible houses
or render distance; shared trunks and junctions avoid all-to-all road building.
Nearby regions are engineered off-thread as you explore. Island crossings use
covered seabed tunnels, not surface bridges. Roads retain legs, local cuts,
coverage-AA at distance and the **25% maximum grade**. Village approaches meet
the ground outside the houses. Trees are excluded from decks, shoulders and
open cuts; trees over covered tunnels remain.

```sh
env -u WAYLAND_DISPLAY PLANET_ROAD_EXPERIMENT=1 cargo run --release -p planet-app
```

Omit `--scenario` for ordinary walking/free flight. Mouse/WASD and **G** work as
below. The default interactive start is on the **dry landward approach** to the
undersea tunnel containing the longest sea section recorded so far (about19 km
in one region). Its entrance is now about450m farther inland; the road enters
covered ground before passing beneath the beach and sea. This is a pinned
verified candidate, not an automatically updated global maximum as the survey
continues. Coastal engineering has changed the old entrance-to-exit length.
`PLANET_ROAD_NETWORK_SITE=mountain`, `village` or `undersea` keeps the legacy
inspection sites; `longest_undersea` explicitly selects the new entrance.
Unset this variable to use the new default. Scripted replay starts are unchanged. `road_network_trial`,
`road_network_overview` and `road_network_walk` are automated replays.
**R** toggles constant-pixel-width road traces (including tunnels): **green**
for generated roads, **red** for ungenerated planned trunks. Above 50 km altitude,
the whole-planet view uses schematic region-centre/gateway connections, not
final engineered bends or individual village accesses. Generated regions stay
green for the session even after leaving the resident cache; close-up green
traces follow the exact resident roads.
`PLANET_ROAD_LINES=1` starts the overlay on; it is off by default.
Regional planning favours viable tunnel shortcuts, but retains conservative
routing as fallback. Roof cover, the 25% grade cap and smooth transitions remain
mandatory; an unsuccessful preference alone cannot discard a buildable region.
`PLANET_ROAD_NETWORK=1` also enables the network directly. Set
`PLANET_ROAD_NETWORK=0` to retain the old single-road interactive experiment;
its existing replay names select the pilot automatically.

Only nearby geometry is resident: at most nine requested regions, a 32-region
CPU plan cache and staged vertex uploads. This is on-demand world generation,
not an exhaustive engineering bake of every planetary region. A failed search
or capacity check is logged explicitly and retains the previous complete set;
it never silently claims a missing village was connected. See
[`docs/ROAD_NETWORK.md`](docs/ROAD_NETWORK.md) for limits and validation.

### Offline road precompute and live status

The CPU-only precompute writes one atomic result per retained region. It can be
resumed **after the previous process has stopped** using the same command:

```sh
target/release/planet-app --road-network-precompute /home/dad/roadnet --road-precompute-threads 7
python3 scripts/road-precompute-status.py /home/dad/roadnet --expected-regions 145422
```

The status script is read-only and works before `summary.json` exists; add
`--json` for the full report, including failed region names. Processed files
include failures, not just buildable roads. Lengths/counts are **regional covered
components**, possibly branched or split at gateways, not unique global tunnels.

New launches reuse parseable successful records for the same region/source
version; failed, incomplete and mismatched records are retried. This upgrade
does not change an already-running process. Existing files stay in place until
an atomic replacement is ready. The source version includes the profile/route/
mesh code, so changes to those engineering rules invalidate old records on
resume. A fresh output folder is useful when preserving the previous survey.

The default command stores statistics only. `--road-precompute-plans` additionally
saves the engineered geometry data; adding it later recomputes stats-only
regions. Those plans are **not yet loaded by the interactive renderer**. Do not
run two precomputes against the same output folder concurrently.

### Original 30 km terrain-planned road pilot

The original opt-in pilot is approximately **30 km** of terrain-led bends on
variable-height legs. A one-time corridor search penalises steep natural
ground, route length and sharp turns; a fitted deck profile has a hard **25%
maximum grade**. Local hillside cuts make room beneath the deck, with exposed
earth on cut faces. Cuts are reversible runtime deformation, not edits to the
baked outmap. Meshes/cuts are cached, with local batches and a distant deck LOD.
Distant decks use analytic pixel coverage: a sub-pixel road contributes a faint
line instead of flickering solid pixels. This works independently of F6 post-AA;
close geometry is unchanged and foreground terrain still occludes the road.

The trial now starts in **steeper alpine ridge country**, crossing about 6.4 km
of natural relief. Sustained cuts deeper than 25 m
for at least 100 m become tunnels where a five-point cross-road survey confirms
at least 5 m of cover above the 9 m crown. Portals extend toward the hillside;
sections still buried at a survey endpoint are not given false portals. The
current ~30 km route includes five tunnels totalling about 1.57 km. Its deck stays
on legs, with cached lining, portal collars and small marker strips; walking
uses the deck beneath the hill, with swept wall/roof collision.
`road_tunnel_trial` shows entry/interior/exit/roof views; `road_tunnel_walk`
walks through the entrance. `PLANET_ROAD_TUNNELS=0` keeps the same route but
uses the previous open-cut treatment, for comparisons. `PLANET_ROAD_HEADING`
rotates the local survey corridor in degrees (default 270, westbound); other alignments
are experimental and may not produce a tunnel.
Ordinary runs build none of this. For **free exploration**, omit `--scenario`:
this starts on the deck in free flight instead of on the ship.

```sh
env -u WAYLAND_DISPLAY PLANET_ROAD_EXPERIMENT=1 PLANET_ROAD_NETWORK=0 cargo run --release -p planet-app
```

Click to enable mouse-look; WASD moves, Shift accelerates, `[`/`]` adjust
flight speed. Look up/down and move forward to climb/descend. **G** switches
between walking and flying; Space jumps while walking. There is no replay
timer or automatic exit in this mode.

Road launches automatically enable the synchronized-presentation workaround
previously used for scene jitter, retaining CPU/GPU preparation overlap.
`PLANET_SYNC_PRESENT=0` explicitly disables it for comparisons; `=1` forces it
on even without roads. The startup log records `sync_present`. The user has
confirmed the default workaround resolves the reported jitter; its scheduling
is unchanged by the network integration.

Run the driving replay, or replace the scenario with
`road_viaduct_overview` to inspect a deep cut, taller legs and an aerial view.
`road_route_survey` captures six locations across the route, including the
deepest cut:

```sh
env -u WAYLAND_DISPLAY PLANET_ROAD_EXPERIMENT=1 cargo run --release -p planet-app -- --scenario road_surface_trial
```

For matched cost measurements use scenario `road_viaduct_performance`,
environment `PLANET_PRESENT_MODE=immediate PLANET_VIEWPORT=640x360`, and
CLI flag `--profile-render`.
Use `road_tunnel_performance` for a fixed camera in the tunnel, with
`PLANET_ROAD_TUNNELS=0` versus `1` to compare open-cut and tunnel versions.
This measures the complete change of terrain treatment, not lining draw cost
alone. Keep other flags, binary and camera identical, and the GPU otherwise idle.
Keep `PLANET_ROAD_EXPERIMENT=1` in both runs and add `PLANET_DISABLE=roads`
only to the hidden run: this retains the identical elevated camera/collider.
To price the road **and terrain-cut rendering** together, use
`PLANET_DISABLE=roads,road_cuts` for the hidden run. `road_cuts` is a shader
ablation only; the surveyed plan/camera/collision field deliberately stays
fixed for this diagnostic comparison.
The pilot has no approach ramps, connected network, or pier/parapet collision.

### Controls

`W`/`A`/`S`/`D` move, mouse looks, `Shift` accelerates. `Esc` or `Q` quits.

| Key | Toggle |
|---|---|
| `F3` | Debug overlay |
| `F4` | Orbit ⇄ low flight (2 m above the surface) |
| `F6` / `F7` / `F8` | Blur / bloom / HDR effect |
| `F9` | Cycle debug mode (final, albedo, lighting, aerial, …) |
| `F10` | Freeze animation |
| `F12` | Screenshot |
| `F` | Fullscreen |
| `O` | Flat-triangle outlines |
| `N` | Go and watch the nearest birds down on the ground or water, from just outside the range that would put them up |
| `6` | Auto exposure |

## Baking terrain

The app never generates macro terrain at runtime — the baker owns geography,
erosion, hydrology, climate, and biomes; the runtime only streams tiles and adds
bounded procedural microrelief.

```sh
./scripts/rebake-test-planet.sh          # bake, validate, install (keeps a backup)
cargo run --release -p planet-baker -- --help
cargo run --release -p planet-baker -- --validate assets/outmaps/test-planet
```

Baking at full resolution is slow; `--quick` trades resolution for turnaround.

## Tests

```sh
cargo test --workspace
```

The suite is mostly CPU-side invariants plus WGSL parse/validation, so it needs
no GPU. Several tests exist specifically to stop the CPU and GPU descriptions of
the surface from drifting apart — if you change a constant that appears in both
Rust and WGSL, expect one of them to tell you:

- `every_shader_places_the_surface_on_the_coretypes_sphere` — the planet radius
- `shader_detail_ladder_matches_the_cpu_clearance_ladder` — the microrelief ladder

## Environment variables

| Variable | Effect |
|---|---|
| `PLANET_DEBUG_MODE` | `final`, `albedo`, `lighting`, `aerial` |
| `PLANET_MAX_ACTIVE_CHUNKS` | Lift or lower the quadtree leaf budget |
| `PLANET_FLAT_TRIANGLES` | Flat-triangle experiment (`0`/`false`/`off` disables) |
| `PLANET_PRESENT_MODE` | `immediate` to bypass vsync when measuring |
| `PLANET_FRAME_LATENCY` | Desired frame latency |
| `PLANET_SYNC_PRESENT` | GPU-completion fence before presenting; automatic for road experiments, otherwise off; `0`/`1` override |

## Build trees

Give every staged checkout its own `CARGO_TARGET_DIR`. Sharing the worktree's
`target/` can leave a stale binary that Cargo reports as fresh, so you end up
measuring a different source tree than the one you edited:

```sh
CARGO_TARGET_DIR=/path/to/scratch-target cargo test --workspace
```

## License

MIT OR Apache-2.0

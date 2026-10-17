# planet

A planet renderer in Rust and WGSL: a 4,000 km-radius cube-sphere with baked
macro terrain, quadtree LOD, analytic atmospheric scattering, Gerstner-wave
ocean, and two interchangeable render paths (raster and foveated raymarch)
that resolve to the same surface.

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

### Controls

`W`/`A`/`S`/`D` move, mouse looks, `Shift` accelerates. `Esc` or `Q` quits.

| Key | Toggle |
|---|---|
| `F3` | Debug overlay |
| `F4` | Orbit ⇄ low flight (2 m above the surface) |
| `F5` | Raster ⇄ raymarch render path |
| `F6` / `F7` / `F8` | Blur / bloom / HDR effect |
| `F9` | Cycle debug mode (final, albedo, lighting, aerial, …) |
| `F10` | Freeze animation |
| `F11` | Foveation warp debug |
| `F12` | Screenshot |
| `F` | Fullscreen |
| `O` | Flat-triangle outlines |
| `N` | Go and watch the nearest birds down on the ground or water, from just outside the range that would put them up |
| `1`–`5` | Raymarch experiment toggles (3 is reserved but unimplemented) |
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
- `shader_experiment_bits_match_the_host_bits` — raymarch experiment bitflags

Render-path parity across raster and ray needs a display:

```sh
./scripts/run-render-path-parity.sh
```

## Environment variables

| Variable | Effect |
|---|---|
| `PLANET_RENDER_PATH` | `raster` or `ray` |
| `PLANET_DEBUG_MODE` | `final`, `albedo`, `lighting`, `aerial`, `ray_hit` |
| `PLANET_MAX_ACTIVE_CHUNKS` | Lift or lower the quadtree leaf budget |
| `PLANET_FLAT_TRIANGLES` | Flat-triangle experiment (`0`/`false`/`off` disables) |
| `PLANET_RAY_EXPERIMENTS` | Raymarch experiment bitmask |
| `PLANET_PRESENT_MODE` | `immediate` to bypass vsync when measuring |
| `PLANET_FRAME_LATENCY` | Desired frame latency |

## Build trees

Give every staged checkout its own `CARGO_TARGET_DIR`. Sharing the worktree's
`target/` can leave a stale binary that Cargo reports as fresh, so you end up
measuring a different source tree than the one you edited:

```sh
CARGO_TARGET_DIR=/path/to/scratch-target cargo test --workspace
```

## License

MIT OR Apache-2.0

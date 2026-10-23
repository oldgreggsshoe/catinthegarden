# Stationary undersea entrance investigation — 23 October 2026

## Completed targeted improvement

Claude's work was paused by the user for the following measurements. Based on
commit a9cc600 (including Claude's ship/terrain CPU improvements), with no
quality changes, GPU timestamps, routing changes or new rendering passes.

### Measurements

Quadro M1000M, X11, Immediate, High,1280x800. The new
`road_network_entrance_performance` replay exercises the real driving
controller, holds throttle until the final frame, and records360 frames with
120 warm-up frames excluded. One final capture is outside the timing sample.
It uses a fixed sun/clock progression and the pinned dry approach. The authored
60-degree reference FOV becomes71.635degrees at this viewport; the separate
interactive check below retains the real60-degree starting FOV.

| Replay, p50 ms | Before | After |
|---|---:|---:|
| Stationary entrance, first pair |68.062|63.934|
| Stationary entrance, repeat |68.140|63.841|
| Baseline rerun after both optimized runs |68.341|—|
| Actual interactive parked start |72.775|68.284|

About4.3–4.5ms saved, or6–7% more FPS. The interactive pair is approximately
13.7 ->14.6FPS, **not** a claim that the entrance is now fast. Its clock,
weather and UI remain live; the fixed replay is the stronger matched evidence.
Per-frame pacing excludes startup/capture frames; the interactive logs also
retain a30-second settled sample window after road publication. Both verify
0km/h,589824 terrain triangles and97344 ocean triangles. No competing renderer
was observed in these accepted runs. Uncontrolled desktop activity can still
contribute noise, particularly to small-sample tail percentiles.

Original replay ablations (one variable disabled, deliberately wrong images):
terrain14.795ms, ocean66.466ms, roads66.572ms, road_cuts45.677ms versus68.062ms
baseline. Terrain is dominant; lamps and ocean are not the main entrance cost.
Temporary narrower probes: removing vertex excavation51.741ms, fragment masks
56.979ms; costs are not additive because geometry/depth change. Early opaque
road drawing57.193ms and an identical entrance image, but **not retained**:
it changes the ground-only depth-probe contract and ordering with water/clouds.
All temporary switches/source changes were removed.

### Implementation

- `road_network_field.rs` adds an open-road/court flag alongside the existing
  bore flag in each cell's fractional reference-count metadata. Integer count,
  buffer size, candidate order and query budgets remain unchanged. Both flags
  are exactly representable through the full64-reference capacity.
- `road_cuts.wgsl` skips open-cut work in cells containing no open road/court.
  If the first terrain-cut iteration leaves the height unchanged, it returns:
  the second projection and scan would repeat exactly the same inputs.
- No terrain density/detail, gradients, geometry, lanterns, material textures,
  camera/physics, presentation synchronization or portal-culling changes.

### Validation and artifacts

- Metadata regression failed on the old encoding, then passed. Full-capacity
  open-only, bore-only, mixed and parking cells are covered.
-37960 GPU points compare the optimized and original two-pass arithmetic in
  one shader: bit-identical heights/normals, identical lowered/paved flags and
  pavement ownership. Eight explicit Vulkan road checks pass, including
  existing CPU/GPU bore/cut/forest/court and subpixel-road coverage checks.
- Full app suite:769 pass, the same6 unrelated zoom/FOV/shallow-water/HUD
  failures,69 ignored. Registry and stationary-controller test pass.
- Both entrance after captures are pixel-identical to all3 before captures.
  Also all6 portal/walking before/after captures are pixel-identical and were
  opened as a comparison sheet: entrance, looking back out, roof and walking
  across the mouth into the bore. All replay assertions pass.
- Baseline replay: `road_network_entrance_performance/1792767245-227451`;
  repeats1792767515-228318 and1792774446-242372. Optimized1792774306-242009
  and1792774379-242201.
- Interactive before/after: `manual/1792774608-242717` and
  `manual/1792774704-243235`. Optimized0km/h screenshot opened.
- Portal before/after: `road_network_coastal_portal/1792774967-245396` and
  `1792775032-245533`; walk: `road_network_walk/1792775094-245671` and
  `1792775160-245844`.
- Logs, immutable before/after executable hashes, pacing summaries and image
  differences are in `test-runs/entrance-profile/`. Comparison sheet:
  `portal-walk-comparison.png`; all these are local ignored evidence.

Reproduce with:

```bash
env -u WAYLAND_DISPLAY PLANET_ROAD_EXPERIMENT=1 \
  PLANET_ROAD_NETWORK_SITE=longest_undersea PLANET_ROAD_SIGNS_WAIT=1 \
  PLANET_QUALITY=high PLANET_VIEWPORT=1280x800 PLANET_PRESENT_MODE=immediate \
  target/release/planet-app --scenario road_network_entrance_performance --profile-render
```

### Remaining options

The scene still submits the same terrain geometry and remains GPU-bound.
A substantially larger gain needs conservative outdoor/terrain occlusion or
more involved caching of terrain vertex/excavation work. Early-road depth is
worth a separate, properly validated follow-up, not silently folded into this
exact-output change. Lower quality is an available user trade-off; no preset
was lowered here. Do not force deep-tunnel culling on at the outdoor approach.

## Initial investigation (historical; superseded below)

At the first investigation, isolated timing/ablation work was **not complete**.
Claude is concurrently doing general performance work. Do not treat these
captures as a baseline or quote their FPS: other GPU benchmark processes
started during both attempted runs. Our test renderer has been closed.
No renderer, driving, lighting, routing or quality behaviour was changed.

## Reproduction and evidence

- Interactive default undersea approach, `PLANET_ROAD_EXPERIMENT=1`,
  `PLANET_ROAD_DRIVE=1`, `PLANET_ROAD_NETWORK_SITE=longest_undersea`.
- High quality, X11, Immediate, internal1280x800, normal full-screen UI.
- Frozen executable copy at `test-runs/entrance-profile/planet-entrance-profile`;
  SHA256 `9afe639bf746d9e5ff79c78cc40aff37faae0a1bff9aa2d359a448212d0c7221`.
  HEAD at copy4c07fd7; source-at-copy.patch records concurrent dirty source,
  **not a guarantee that every dirty edit was in that binary**. No build was
  performed and the shared target/release executable was not replaced.
- Opened `test-runs/manual/1792761038-198724/screenshots/capture-001.png`:
  actual parked0km/h view, road parapets in foreground,21KM wood header,
  tunnel mouth/cut banks and lit bore ahead. This first experiment froze
  animation; rejected for timings (concurrency and frozen logging).
- Normal-clock second attempt `test-runs/manual/1792761192-199904` retains
  zero vehicle velocity,256 drawn chunks,589824 terrain triangles and97344
  ocean triangles in its settled samples. Both tunnel_interior and
  tunnel_enclosed are false. Counts/visibility flags remain useful even though
  timing measurements are contaminated. Concurrent process evidence lives in
  `test-runs/entrance-profile/baseline-live/contamination.txt`.
- Test helpers/logs/provenance under `test-runs/entrance-profile/` are local
  ignored artifacts. No valid p50/p99 result is published. The key-driven
  harness also had focus/close issues when the other benchmark appeared;
  do not mistake an incomplete run's global frame_pacing_latest.txt for its
  own result. Prefer a dedicated stationary scenario for follow-up timing.

## Confirmed mechanism versus hypotheses

`Prepared::inspection_pose` in road_network_mesh.rs deliberately starts18m
along the dry outdoor approach. `position_road_start` enters driving there.
`tunnel_visibility::Visibility::enclosed` requires an actual tunnel-void
sample and a proven interior core before propagating portal visibility.
`main.rs` skips outdoor terrain/sky/ocean/weather/forest only for a proven
fully enclosed view. Thus this entrance cannot use the existing deep-tunnel
shortcut. That is correct for safety, not proof of a visibility bug: the
outdoor cut bank and approach must stay visible.

The scene still issues substantial outdoor geometry. Which part dominates
this location must be measured; the counts alone do not establish cost.
Road/tunnel geometry is drawn after the ground/water passes. A tunnel depth
prepass could reject some later fragment work if reordered, but cannot by
itself avoid terrain/ocean vertex processing.

## Options and required experiments

1. **Entrance-aware occlusion/culling**: retain the visible approach and cut
   bank, conservatively reject outdoor chunks hidden behind them/the bore.
   Candidate for a location-specific, quality-preserving improvement. Must
   validate moving/turning across the mouth; do not simply force the existing
   enclosed flag while outside (it would erase visible surroundings).
2. **Reduce excavation lookup cost**: first compare `PLANET_DISABLE=road_cuts`
   with normal rendering. This deliberately changes the picture/portal and
   measures an upper-bound attribution, not an acceptable fix. If material,
   investigate skipping unaffected chunks or caching more of the cut field,
   retaining CPU/GPU and portal/seam agreement.
3. **Road/lantern work**: compare `PLANET_DISABLE=roads` at this outdoor pose
   (enclosed is already false). If expensive, separate geometry, cut-field
   clipping and per-fragment nearby-lamp evaluation. Do not dim/remove lamps
   on the assumption that they are the culprit.
4. **Existing quality presets**: compare High/Medium/Low at this exact pose.
   This is the immediate available trade-off, but its entrance-specific FPS
   and visible terrain differences have not yet been measured. Claude owns
   the broader quality and CPU/LOD performance work; avoid duplicating it.

## Resume when the GPU is available

Reserve a benchmark window with Claude/user before starting more replays.
Keep an immutable executable and record hash, effective viewport/quality,
X11/Immediate and source provenance. Obtain a normal-clock stationary
baseline after nine-region road publication and settle; confirm0km/h and
same planet-local eye/view. Then run one-variable ablations: ocean, terrain,
road_cuts, roads, followed by repeat baseline and Medium/Low if useful.
Monitor for other renderer processes throughout, abort/invalidate on overlap.
Capture pictures outside timing windows, inspect them, and repeat the largest
contrasts. Use per-frame pacing for p99, not sparse0.5s logs; no GPU timestamps
on this Quadro. A frozen clock suppresses the existing interactive periodic
profile logs, so it is not sufficient for the proposed CPU/GPU split.

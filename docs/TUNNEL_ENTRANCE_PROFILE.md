# Stationary undersea entrance investigation — 23 October 2026

Status: location reproduced; isolated timing/ablation work **not complete**.
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

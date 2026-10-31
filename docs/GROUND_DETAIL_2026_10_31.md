# Close ground texture — 31 October 2026

## Report and reproduction

The user's latest manual screenshot was
`test-runs/manual/1793446215-7196/screenshots/capture-001.png`: green, smooth clay-like
foreground with vague hills, and similarly smooth brown paths. Opened it before
editing. The ground was being lit and displaced, but had little visible fine
material variation.

Reconstructed the terrain-local eye from 58 recorded surface-hit directions,
heights and ray distances using sphere-difference least squares; maximum distance
residual <6e-10m, altitude 4610.862760713m, 1.7m above the ground. Fitted the view
ray from the recorded screen probes. New `ground_detail_village` holds that eye;
`ground_detail_walk` strafes 45cm through four captures. The replay is a window
(1280x647), not the original fullscreen 1280x720, so framing is slightly different.
Matched before/after **replays** share all camera/lighting/quality settings.

## Diagnosis

Ranked before probing: post-sample suppression, excessive mip filtering, and broad
geometry/normal shading overwhelming the material. The existing close texture
was already warped, mipmapped and anisotropically filtered. However:

1. The fragment shader reduced the sampled material tint to **8%** at all ranges.
2. The grass colour generator put 64% of colour into broad noise, 28% into medium
   noise, 8% into texel grain, and **none** into its already computed fine noise.
   Reusing that landscape texture at 6m largely produced more smooth patches.
3. Path colouring replaced most of the textured ground with flat brown soil.

The first gain-only trial mostly darkened the same blobs; its foreground 3px
high-pass luminance standard deviation rose only 1.423 to 1.446 display levels.
It was **not** accepted as the fix. Adding genuinely fine close layers raised it
to 2.119, with visible fine mottling on grass and the compacted paths. These are
supporting image measurements, not a substitute for viewing the captures.

## Change and bounds

- Keep the original four landscape texture layers/means unchanged. Add two
  close-only grass/soil layers, using the same material palettes and deterministic
  tileable fields but putting most colour into decimetre clumps/centimetre grain.
  Existing close triplanar fetches select these layers; **no extra texture fetches,
  geometry, draw calls or fragment noise evaluations**. Texture memory increases
  about 0.67MiB at 256x256 including mips. Generated/uploaded once, not per frame.
- Close tint strength is 0.65 rather than 0.08, fading with the existing 150–900m
  close-tile weight. Beyond it the original 0.08 distant strength remains. No
  change to tile scale, warp, texture filtering, relief, normals, LOD, shadows,
  sky/weather, biome ownership, village positions or roads.
- Path soil retains the luminance grain already fetched for the ground, clamped
  to 0.6–1.4. Its additional grain also fades out with the close-tile weight. No
  second path texture or terrain mask lookup; the flat debug path supplies unity.
- No new dependency, asset bake, lockfile, height data or GPU layout change.

**Limits:** this restores procedural material grain, not photogrammetry, grass
blades or normal-mapped vegetation. The broad faceting/rounded relief visible in
the original remains; increasing texture contrast must not be described as fixing
that separate geometry/shading issue. Slow captured strafes are not an exhaustive
LOD-flight/temporal acceptance test.

## Tests and evidence

Full app suite: **838 passed, 6 existing failures, 79 ignored**. Failures remain
three optical-zoom tests, scenario FOV range, shallow-turquoise shader expectation
and flooded-ship HUD. Both new checks pass; all168 scenario entries load. This is
not a claim that the full suite is green. Check and release build pass (same two
existing dead-code warnings).

- New contrast-budget regression fails on the old shader, then passes after the
  fix. Guard preserves the far budget and requires paths to retain filtered grain.
- New CPU frequency-energy test requires both close layers to have over twice
  their landscape counterpart's adjacent-texel energy. Existing tileability/mip,
  broad-layer mean and shader validation tests remain relevant.
- Before: `test-runs/ground_detail_village/1793446565-8176/screenshots/capture-001.png`.
- Gain-only diagnostic: `ground_detail_village/1793446737-9339` (insufficient).
- Fine-layer diagnostic: `ground_detail_village/1793447052-10668`, opened. The sky
  and inn-wall ROIs remain pixel-identical to the before capture.
- Four slow-strafe frames opened in `ground_detail_walk/1793447169-11070`: detail
  moves with the ground, not as an overlay stuck to the camera.
- Orbital comparison `orbit_once/1793447194-11215` vs `1793447218-12052`: captures
  1,3,4 pixel-identical. Early capture2 differs in one localized 119x115-pixel patch;
  a concurrent Claude build was active, so do not call this a clean all-four
  parity/timing result (streaming timing is a hypothesis, not a confirmed cause).
  Both manifests pass; the job was interrupted after the
  second renderer completed. Retain the captures, discard its timings.

- Additional `mountain_ground` before/after manifests pass, but both captures are
  featureless pale views, so they do **not** validate close rock/snow appearance.
  Their short runs did not publish new frame-pacing summaries; stale copied timing
  fields are explicitly labelled invalid in the evidence and are not used below.

Raw logs, immutable test binaries, pose, timings and rejected trial evidence are
under `test-runs/ground-detail-2026-10-31/` (ignored artifacts).

## Final controlled validation

Final before: `ground_detail_village/1793447640-19572/screenshots/capture-001.png`.
Final after: `ground_detail_village/1793447685-19812/screenshots/capture-001.png`.
Opened the final after capture; the grass/path grain is present, with the same
large-scale mounds/faceting still apparent. Both repeated before images are
pixel-identical to each other; both after images are also pixel-identical.

`python3 scripts/check-ground-detail.py BEFORE_RUN AFTER_RUN` checks this exact
Low/windowed fixture. It requires at least 1.35x foreground 3px high-pass contrast
and identical sky/inn-wall control ROIs. Final: **1.489x**, controls identical;
gain-only rejected with the texture-suppression assertion. This does not certify
photorealism or replace the opened stills/slow-motion inspection.

Quadro M1000M, Low, X11/Immediate, captured 1280x647, traffic/moon off, roads on,
fixed pose and scene clock. Waited for all compilers/tests to finish, then ran
**before/after/after/before** with immutable binaries. The other actor's later
build was verified in `/home/dad/catingard-people` (not this checkout) and started
11:57:49, after the final control completed at 11:57:01. No timing claim from the
initial concurrent-build orbit run is used here. Frame-pacing excludes the first
120 frames and capture-related intervals.

| Run | Mean ms | Median ms | p95 ms | p99 ms |
|---|---:|---:|---:|---:|
| Before 1 | 58.762 | 58.630 | 60.136 | 60.777 |
| After 1 | 58.908 | 58.703 | 60.330 | 61.213 |
| After 2 | 58.743 | 58.566 | 60.184 | 60.944 |
| Before 2 | 58.699 | 58.494 | 60.337 | 61.160 |

Mean of medians: **58.562 -> 58.6345ms (+0.0725ms / 0.12%)**. Frame-time means
change by +0.095ms. This is effectively unchanged within run variation, not a
speedup or a whole-planet performance guarantee. All four final manifests pass;
raw rows and run locations in `final-pairs.json` and `final-*.pacing` in the evidence
folder. The existing terrain triangle counts, LOD rules and draw path are retained.

## Viewing

From the repository root, using the rebuilt release:

```sh
env -u WAYLAND_DISPLAY PLANET_ROAD_EXPERIMENT=1 PLANET_ROAD_NETWORK_SITE=village PLANET_TRAFFIC=0 PLANET_QUALITY=low PLANET_PRESENT_MODE=immediate PLANET_VIEWPORT=1280x720 target/release/planet-app --scenario ground_detail_village
```

Use `--scenario ground_detail_walk` for the short strafe review. These are scripted
previews; normal walking/flying also receives the close material change.

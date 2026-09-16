# Frame cost of the 15 September peak-realism changes (raster)

Same scenario and poses (`peak_survey_8_directions`, final HDR scene, immediate present, 69 logged
frames each). `frame_time_ms` from each run's log.jsonl; nothing was profiled with --profile-render.

| stage | run | median ms | p90 ms | step |
|---|---|---|---|---|
| round 1 baseline | 1789482286-124175 | 37.96 | 44.00 | |
| snow slope hold + thresholds + rock + sky view | ...1789487149-131626 | 40.10 | 46.17 | +2.1 total |
| **cast shadows** (a186019) | 1789488348-133511 | 48.69 | 56.57 | **+8.6** |
| blend-weighted gates | 1789489299-135918 | 49.43 | 57.62 | +0.7 |
| **organic_biome_blend, 2 octaves** | 1789489562-136221 | 88.38 | 102.60 | **+39.0** |
| organic_biome_blend, 1 octave (shipped in d9c85dd) | 1789489857-137004 | 77.94 | 90.47 | -10.4 |
| sand gate, snow-look neutrality | 1789490766-139093 | 77.58 | 89.89 | -0.4 total |
| steep-face detail fade 35-55 (uncommitted) | 1789492837-142899 | 77.75 | 90.00 | +0.2 total |

Recommendation (not applied; Ian asked for no more changes this week):
- Remove `organic_biome_blend` (~+28 ms). It only reshapes lake shorelines; the orange-patch fix is
  the snow-look neutrality (+0.5 ms), which stays.
- Remove or redesign the terrain cast-shadow march (+8.6 ms); judges read its coarse output as grey
  decal blobs. Any replacement must be near-free (per-vertex or baked), not a per-pixel march.
- Expected result: ~41 ms median, keeping the rock, sky-view, gate, neutrality and streak fixes.

## Round 5, 16 September: the two expensive changes removed

Same scenario and poses, median/p90 `frame_time_ms` over 69 logged frames.

| stage | median ms | p90 ms |
|---|---|---|
| HEAD before this round (streak fix included) | 77.22 | 89.64 |
| minus the per-pixel cast-shadow march and the noise-shaped biome edges | 41.90 | 65.21 |
| plus height in the triplanar material coordinate | 41.48 | 47.39 |

**47% off the frame time.** The shadow march and the biome-edge noise were the whole of it, as the
stage-by-stage table above predicted (+8.6 and +28ms when they went in).

The triplanar coordinate change is free but measures as a visual no-op at this range: E cliff
high-pass texture energy 3.89 -> 3.91, SE 4.84 -> 4.82, mean colours unchanged. The coarse material
tile is 2048m, so a ~300m cliff spans a seventh of a tile and the added height barely moves the
lookup. It is kept as a correctness fix for close range, where a vertical face otherwise samples one
texel for its whole height; it is not what will make cliffs read as rock.

Shadows are wanted back, cheaply: amortise the march into a cached shadow texture refreshed a slice
per frame (the sun moves 0.075 deg/s with the 80-minute day), or bake a per-texel horizon map and
compare the sun's elevation against it. The Rust plumbing for the ray path's height faces is
deliberately left in place for that.

## Round 6, 16 September

Median 41.47ms, p90 46.72 -- the best measured. Only change kept this round is the snow-lighting
neutrality (0.82 -> 0.55), which costs nothing. A relief-driven "snow keeps to hollows, rock on the
rises" rule was tried four ways and reverted: `relief` is <=0.01 on 91-99.9% of terrain pixels here
and never exceeds 0.55, so there is no rises-versus-hollows signal at this scale, and on a snow
biome the palette under the snow is itself white. Exposed rock needs its own material and
distribution, not a reinterpretation of the detail field.

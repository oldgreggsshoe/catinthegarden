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

## RESOLVED, 19 September: the 38/41ms baselines are not reproducible

Everything below this line about a "41 -> 62ms regression" was chasing a number that cannot
be reproduced. The decisive measurement, which should have been the first one:

| build | measured today | measured 16 September |
|---|---|---|
| `7c17fb1` (round 6), rebuilt | **77.39ms** | 41.47ms |
| `18e92b6` (HEAD) | **65.78ms** | -- |

**The old code is slower than current HEAD on the same machine and the same scene.** No commit
regressed anything; HEAD is ~15% faster than the round-6 code.

The 16 September baselines were built from a **dirty working tree**. A manifest records the
repository HEAD, not the code that ran: `f2770d9` -- the commit stamped on the 37.96ms
baseline -- does not even contain `peak_survey_8_directions.json`, which was uncommitted then
and only landed in `a186019`. So every "same commit, different speed" comparison in this file
is comparing against code that no longer exists.

Five hypotheses died by measurement before this one was tested:

| hypothesis | how it died |
|---|---|
| the rebaked planet | A/B: pre-rebake planet measured *slower* (68.14 vs 66.38) |
| a specific commit | rebuilt round-6 commit measures 77.39, slower than HEAD |
| machine load | quiet machine 65.86 vs busy 65.22 -- identical |
| present mode | `CATINGARDEN_PRESENT_MODE=immediate` 65.78 -- identical |
| sky coverage in frame | baseline 35.4% sky vs round 8 38.5% -- same view |

Workload was provably identical throughout: `budget_limited` 1.00, `fallback_chunks` 240,
`resident_chunks` 255, `resident_tiles` 61, ~29 draw calls, 587k triangles across every run.
The GPU boosts to its full 1124MHz at 43C and sits at 61-63% utilisation, so roughly a third
of each frame is not drawing -- that is the real standing question, and it is a property of
the renderer today rather than a regression from anything.

**Method rules this cost a day to learn.** Rebuild the old commit before proposing a cause.
Never trust a manifest's commit as a description of the binary. Give any measurement script a
freshness guard -- one of mine silently reported the previous run's directory as if it were a
new result. And do not pipe a gated command into `tail`; the pipeline returns the last
command's status and a failing test reached a commit that way.

---

## Round 8, 18 September: the 41 -> 62ms step is the planet, not the shaders

Median 66.38ms, p90 74.61 at `78a25cd`. Round 6 measured 41.47ms. Walking every run's
`git_commit` against its median shows the step does **not** land on a commit:

| run | commit | median ms | wall clock |
|---|---|---|---|
| 1789568684-237056 | 7c17fb101 | 41.47 | 09-16 15:04 |
| 1789569854-239516 | 7a2056df5 | 41.07 | 09-16 15:24 |
| 1789570069-239752 | 7a2056df5 | 41.03 | 09-16 15:47 |
| 1789570504-240297 | 7a2056df5 | 23.32 | 09-16 15:55 |
| **1789593571-271022** | **7a2056df5** | **62.20** | **09-16 22:19** |
| 1789593846-272233 | 7a2056df5 | 64.39 | 09-16 22:24 |
| 1789599172-279400 | b55c732a5 | 61.76 | 09-16 23:52 |
| 1789601036-282744 | 27d6bfbca | 62.10 | 09-17 00:23 |
| 1789735429-415647 | 78a25cda9 | 66.38 | 09-18 13:43 |

**Same commit, same scenario, 41ms then 62ms six hours later.** The only thing that changed in
between is the planet on disk: every tile under `assets/outmaps/test-planet/tiles` was rewritten at
**09-16 19:12** by the slope-aware rock rebake, between the last 41ms run (15:55) and the first
62ms run (22:19). Tile count is identical (9756) and size barely moved (372M -> 374M), so this is
the same shape of data with different biome content -- 2.56% -> 12.46% rock by area, which puts far
more of the frame on biome boundaries and mixed-material pixels than the old near-uniform ice.

**RETRACTED -- the A/B killed it.** Same scenario, same commit `78a25cd`, same session:

| planet | median ms | p90 ms |
|---|---|---|
| rebaked (active) | 66.38 | 74.61 |
| pre-rebake backup | **68.14** | 75.43 |

The pre-rebake planet is *slower*, so the rebake did not cost the 21ms. The biome content of the
baked data is not the driver. Both planets cost ~67ms today while the same commit measured 41ms on
16 September, which makes this **environmental, not data and not code**: something outside the
binary changed between 15:55 and 22:19 that day and has not changed back. Sunshine/Moonlight remote
play was set up in that window ([[reference_moonlight_remote_play]]) and a streaming host holds the
GPU; that is the next thing to check, along with clocks and thermal throttling. Do not attribute
this to any commit or to the bake until something is measured with it turned off.

## Glacier export correction, 19 September: matched current-binary controls

`e8248bf` preserves the new glacier biome labels through export. No runtime shader change in
this fix; the same binary renders Claude's third bake and the corrected export. Quadro M1000M,
Immediate present, raster, 1280x720, 69 logged frame times per run, no timestamp profiling:

| Scenario/pair | Before median ms | After median ms |
|---|---:|---:|
| Alpine 1 | 88.547 | 88.235 |
| Alpine 2 (after first) | 88.177 | 88.688 |
| Summit control | 70.372 | 70.251 |

Mean alpine run medians 88.362 -> 88.461ms (+0.11%). Opposite pair signs; no FPS gain is
claimed. All six replays pass, with pixel-identical repeats within each bake. All heights and
moisture payloads are byte-identical. See `glacier-export-fix/RESULTS.md` for exact run IDs,
round-12 pixel differences, diagnosis and the intentionally deferred judging round.

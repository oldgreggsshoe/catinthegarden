# Render profile — 20 September 2026

`village_pov`, ground level in inhabited grassland, 1280x720, Quadro M1000M.
Matched A/B: every condition runs once per block so drift hits them all alike,
after two discarded warm-up runs because the GPU idles at 135MHz and boosts to
1124MHz under load. `sweep.sh`, `floor.sh` and `terrain.sh` produced the
matching `.jsonl` files.

GPU timestamps would answer this directly and cannot: asking for
`TIMESTAMP_QUERY` breaks this device. See the handoff for that diagnosis.

## Where the frame goes

| | ms | share |
|---|---:|---:|
| fixed floor, nothing drawn at all | 30.5 | 36.4% |
| terrain, above that floor | 51.3 | 61.3% |
| everything else, above terrain | 1.9 | 2.2% |
| **total** | **83.6** | |

**Of the 53.1ms of scene work, terrain is 97%.** Terrain alone draws in 81.7ms;
adding ocean, sky, stars, clouds, rain, forest, villages, birds and ship on top
of it costs 1.9ms between them. Individually every one of those is at or under
the +/-1ms noise floor, and several measure negative, which is the tell that
they are unmeasurable this way rather than free.

Forest measures *consistently* negative, -1.08/-0.81/-0.32/-0.65 across four
blocks. That is systematic, not noise. The likely reading is that trees occlude
terrain and terrain is what costs, so removing them exposes more expensive
pixels than the trees cost to draw. Untested.

The 30.5ms floor -- post, exposure metering, present and simulation with an
empty scene -- is 36% of the frame and is a target in its own right,
independent of anything terrain does.

## What inside terrain

Terrain cost is measured above the 30.5ms floor.

| condition | frame ms | spread | terrain ms | pixels | triangles |
|---|---:|---:|---:|---:|---:|
| baseline 1280x720 | 82.55 | 0.86 | 52.1 | x1 | 589,824 |
| 640x360 | 48.85 | 1.22 | 18.3 | x0.25 | 589,824 |
| 320x180 | 35.01 | 0.06 | 4.5 | x0.0625 | 587,520 |
| chunk budget 64 | 49.47 | 1.32 | 19.0 | x1 | 147,456 |
| chunk budget 1024 | 181.31 | 0.82 | 150.8 | x1 | 2,359,296 |

Terrain looks fragment-bound from the resolution pair: fitting
`cost = a*pixels + b*triangles` to the 1x and 0.0625x points gives 50.8ms of
fragment work against 1.3ms of geometry. **That model then fails.** It predicts
quarter-chunks at 51.1ms and the measurement is 19.0ms, so cutting the chunk
budget saves far more than its triangle count can explain at unchanged
resolution.

The hypothesis that fits both is **overdraw**: at ground level the 256-chunk
frontier shades the same pixels several times over, so chunk count scales
fragment work and not just vertex work. It is a hypothesis, not a result -- the
way to settle it is to count shaded fragments, not frames.

## Limits

One camera pose. Terrain's dominance will be different at orbit, where the
frontier is a few coarse chunks, and over open ocean. Nothing here was measured
with GPU timestamps, so none of it attributes cost inside a pass.

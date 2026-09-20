# Alpine material removal trials — 20 September 2026

## Status and provenance

Exploratory, **not promoted**. Main renderer, live corrected bake, judging camera
and reference photographs are unchanged by this work. Code is banked on
`experiment/alpine-material-contrast`, based on `ce5631f`, commit `9e41d7a`.
Its checkout is `tmp/glacier-detail-worktree`, with its own
`CARGO_TARGET_DIR=/home/dad/catingard/target-glacier-detail`.
The V4 binary predates that commit but contains its source changes; its SHA256
is `b179127ace4e1051764938be28d12f7b586cafb2ad23372db69c57277123c1aa`.
Do not identify its source solely from the manifest's older/dirty revision.
Later Rust formatting changes do not change the shader used in these captures.

## Clean measurements

Same Quadro, 1280x720 alpine eight-heading replay, Immediate presentation, same
live outmap. Each number is the median of 69 `frame_time_ms` entries from that
run's own `log.jsonl`. These are whole-frame times, not GPU timestamp queries.
The harness polls for external renderer/cargo/rustc processes every second;
none was observed in either block below. This is not exhaustive system-load
monitoring or a confidence interval. User confirmed Claude stood down.

| Ordered run | Mode | Median ms |
|---|---|---:|
| mesh_off1 | control | 85.033 |
| mesh_on1 | omit sub-mesh fragment noise | 80.571 |
| mesh_light1 | same, land sunlight 2→1 | 80.613 |
| mesh_off2 | control | 85.206 |
| snow_off1 | control, V4 binary | 84.993 |
| snow_grain1 | shorter snow fine-texture range | 82.941 |
| snow_combo1 | shorter range + omit fragment noise + sunlight 1 | 79.775 |
| snow_off2 | control, V4 binary | 86.600 |

Within each block controls and candidates use the same binary. The second
control pair is pixel-identical in all eight frames despite a 1.61ms timing
spread. Candidates are faster than both surrounding controls, but there is
only one sample run per candidate here, not a repeated paired statistical
acceptance test. Relative to the mean surrounding controls, fragment-noise
removal reduces frame time about 5.3%; snow range about 3.3%; the combination
about 7.0%. These are exploratory scenario-specific results, not promoted FPS
gains or a guarantee elsewhere.

Earlier floor/grain/lighting runs that overlapped main-checkout activity are
explicitly ineligible in `runs.json`; retain their concurrency logs, never pool
them into these results. The first off1 attempt exited 137 and has no accepted
manifest/result. quiet_off1 is clean but belongs to an earlier binary/block.

## Visual findings and decision

Snow fine texture previously faded over 150–900m; the trial smoothly scales
distance by snow-biome share so pure snow fades over 15–90m instead. It retains
underfoot sampling and the original range in other biomes, with no new texture
or geometry. All eight snow-range frames were inspected. Fine mottling becomes
smoother, but broad irregular patches, smeared rock and distant faceting remain.
The combined variant is markedly greyer/flatter; inspected headings 1,3,5,7 do
not establish a convincing realism improvement. Darkening is not proof of an
exposure defect, and texture removal is not equivalent to better snow.

`pixel-diffs.json` records all eight frames for both candidates and the control
repeat. Snow range changes 261,907–812,273 pixels per frame, maximum channel
change 21–30/255; combined maximum change 63–70/255. These numbers describe
change, not quality. Original PNGs remain in each run directory in `runs.json`.
No image was edited for assessment.

No new judges were spent on these visibly unconvincing changes, no new score
is claimed, and none is enabled by default. The useful result is a measured
performance-saving direction and evidence that the dominant visual problem
survives removal of both fine fragment noise and distant fine snow texture.
Coherent rock/glacier structure remains unresolved. Near-ground motion, moon,
ocean and other biome controls are required before any scope-wide promotion.

## Validation

All ten compile-time modes pass Naga validation and the release build passed.
Full application test results are recorded separately in `validation.txt`.

525 application tests pass, 23 ignored. Strict clippy still rejects the existing
constant assertion at `village.rs:653` in the base; allowing only
`clippy::assertions_on_constants` passes. No unrelated village code was changed.
A mistaken `--lib` test invocation had no library target; an initial debug test
build was interrupted to reuse the existing release dependencies. The release
test above completed successfully. No tests/builds overlapped accepted timings.

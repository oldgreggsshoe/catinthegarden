# Procedural glacier trial — rejected, 20 September 2026

## Decision

Do not promote either patch. V1 is a visual no-op; V2 draws faint, regularly spaced
painted-looking lines rather than convincing fractures. All eight V2 frames were
inspected, including direct full-frame north comparison. No judges were run and
no realism/FPS improvement is claimed. The main renderer and installed bake were
not changed. The export fix remains necessary and banked.

## Reproduction

Both patches apply independently to `ce5631f`. `candidate.patch` is V1;
`candidate-v2.patch` is V2, not an incremental patch. Use a separate checkout with
its own CARGO_TARGET_DIR; never share the main runnable build directory.

The local trial checkout is `tmp/glacier-detail-worktree`, its build directory
`target-glacier-detail`. Build with `cargo build --release -p catinthegarden-app`.
The recorded `run.sh` runs that binary from the project root against the corrected
`assets/outmaps/test-planet`, using Xvfb, Immediate present and the unchanged
`alpine_survey_8_directions` scenario. Run with a unique label and mode `0`, `on`
or `mask`. `runs.json` contains capture directories and median `frame_time_ms`
from the same run's `log.jsonl`; each normal replay has 69 logged frame times.
Raw PNGs/logs remain local, not added to Git. Binary SHA-256 files identify the
captured V1 and V2 builds. The V2 shader-mode regression and release build pass.
The full V1 app suite passed 525 tests, 23 ignored; that is not V2 full-suite evidence.

## Mechanism and limitations

Source biome IDs delimit kilometre-scale regions, not individual cracks. V1
adds anchored, footprint-filtered markings within IDs 10/11, after weather albedo
and before lighting. Its mask diagnostic shows distant dedicated regions rather
than close foreground coverage. All eight V1 off/on images are pixel-identical.
Mask mode is diagnostic only and bypasses normal lighting: exclude its timing.

V2 also draws 30%-strength fractures in non-polar Ice (ID 2), slope-gated, and
holds back the downstream ice-light floor under their coverage. The markings
use a fixed global projection, not real glacier flow, and change no geometry,
collision or texture fetch count. They do not supply relief, broken ice or
credible medial moraine structure. Lack of texture fetches does not guarantee
lack of runtime cost.

## Results

| Trial | Off median ms | On median ms | Interpretation |
|---|---:|---:|---|
| V1 | 87.796 | 88.641 | Eight pixel-identical views; one pair only |
| V2 | 88.109 | 90.194 | Faint painted lines; one pair only (+2.37%) |

V2 changed pixels, N through NW:
39,039 / 36,306 / 32,535 / 32,198 / 31,544 / 17,714 / 15,736 / 16,462.
Maximum channel delta is 14/255 except SW (11/255). Per-view details are in
`v2-pixel-diff.json`; `v2-default-control.json` confirms all eight V2 off images
match V1 off. All four normal replays passed their scenario assertions.
No repeated reverse-order pair, summit control or motion acceptance was spent
on this visually rejected candidate. Timing is indicative, not statistically
established performance regression evidence.

Next hypothesis: terrain/flow-aligned coordinates and irregular fracture
structure, rather than larger/darker fixed-axis bands. Still unimplemented and
unvalidated; the previous realism score is unchanged, not remeasured.

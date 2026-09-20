# Fracture-material follow-up — 20 September 2026

## Status

The local pattern/opacity bugs are repaired and banked, **not a finished glacier
realism fix**. Code is committed and pushed as `f10090b` on
`experiment/glacier-fracture-material`, based on `ce5631f`, in isolated checkout
`tmp/glacier-detail-worktree` with `CARGO_TARGET_DIR=target-glacier-detail`.
The capture binary was built from dirty `ce5631f` plus the exact source diff
subsequently committed as `f10090b`; the manifest commit alone is not its source
identity. The preserved V6 checksum identifies the runnable.
The main renderer, live bake, approved camera and reference photos are unchanged.
No candidate code was copied over concurrent village edits.

V3 replaces continuous phase stripes with finite cell-contained segments. V4
separates placement density from strength: previously 0.3 Ice share made every
crack translucent, rather than selecting fewer solid cracks. V5 adds analytic
rounded-wall bump normals, not geometry. V6 skips empty cells, segment ends and
out-of-fissure work, and does not renormalize/re-light unmarked surfaces.

These remain fixed-axis surface markings, not a flow-aligned glacier model.
Normals do not change silhouettes or collision. The earlier instruction to
reject V1/V2 still stands; these follow-up changes do not imply promotion.

## Actual-WGSL regression and build validation

`gpu_sparse_glacier_fractures_keep_solid_interiors_and_fade_at_cell_edges` runs
131,072 probes of the actual helper on Quadro M1000M. It verifies solid interiors
at 0.3 region share, bounded coverage/finite slopes, zero cell-boundary coverage
and slope, zero unresolved coverage and no markings outside the region.
The regression passes. Restoring only `shares.y * crack` coverage makes it fail
at exactly 0.3 maximum coverage; see the preserved mutation-failure and restored
pass logs. This is a mutation test against the trial's old rule, not a claim that
this helper existed in the main renderer.

525 app tests pass, 24 ignored; release build and all three shader modes pass.
Strict all-target clippy fails on the pre-existing constant assertion in
`village.rs:653`, verified in base `ce5631f`; it is not changed by this work.
Clippy passes with only that existing lint allowed (`-A clippy::assertions_on_constants`);
that is not an unconditional strict-clippy pass. Existing unrelated
formatting is left untouched; the changed Rust test block is rustfmt-formatted.
These checks do not prove visual realism or motion/LOD continuity.

## Captures

Four V6 replays pass all scenario assertions, two off and two on. Repeated
captures are pixel-identical within each mode. All eight off images also match
the earlier V2 off baseline exactly. `v6-pixel-diff.json` records on/off deltas:
N..NW changed pixels = 2,653 / 4,519 / 2,305 / 6,001 / 3,923 / 2,668 / 596 / 844;
maximum channel differences = 199 / 200 / 67 / 184 / 154 / 103 / 26 / 60.
The primary agent inspected all eight complete V6 on images. Marks are visibly
localized and stronger than the old faint lines, but few isolated fissures do
not produce convincing glacier structure across the scene.

Three independent fresh-context Luna reviewers each inspected five unchanged
Aletsch photos plus all eight V6 frames. Scores **2.5, 3, 2; mean 2.5/10**.
See `judges-v6.md` for verbatim summaries. No overall realism win is established,
and none of the visual observations should be misrepresented as a proven
technical diagnosis.

## Performance: all V6 samples excluded

Same binary, Immediate present, unchanged scenario, 69 logged frames per run:

| Label | Run | Raw median ms | Status |
|---|---|---:|---|
| v6_off1 | 1789894179-9530 | 89.688 | Exclude: concurrent build/render work |
| v6_on1 | 1789894404-10279 | 93.287 | Exclude: concurrent build/render work |
| v6_on2 | 1789894637-11152 | 133.809 | Exclude: concurrent build/render work |
| v6_off2 | 1789894924-13705 | 103.089 | Exclude: concurrent build/render work |

A two-second process monitor recorded other main-checkout renderer/build
processes during every run. `v6-concurrency-summary.json` matches samples to
run-start/run-end JSONL timestamps. Raw monitor data is preserved alongside it.
The first run's monitoring starts after the run start; this does not weaken the
observed overlap, and it is still excluded. No cost, neutrality or speedup
conclusion can be drawn from this batch. No timestamp profiling was used.

## Next work

Do not merge or enable by default. The useful local correction is density versus
opacity, covered by an actual GPU regression. The larger missing design is
terrain/glacier-flow alignment and coherent fracture/debris structure; baker
`Terrain::flow_to` exists but is not available to this material path. Adding a
flow field has data-path and performance implications and must be designed,
not disguised by arbitrary rotation or more dark lines. The judges also still
see washed-out snow, blurred rock and coarse mountain forms; those observations
need their own diagnoses rather than another palette-only patch.

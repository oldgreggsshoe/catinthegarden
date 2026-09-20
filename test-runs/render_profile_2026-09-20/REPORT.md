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

The hypothesis that fits both was **overdraw**. It is wrong, and
`overdraw.jsonl` is the measurement that killed it. Overdraw is fragment work,
so the cost of extra chunks would have to shrink with the pixel count. Run the
same two budgets at a sixteenth of the pixels:

| condition | frame ms | spread | triangles |
|---|---:|---:|---:|
| 720p, budget 64 | 49.41 | 1.05 | 147,456 |
| 720p, budget 1024 | 180.93 | 0.49 | 2,359,296 |
| 320x180, budget 64 | 14.86 | 0.07 | 145,152 |
| 320x180, budget 1024 | 103.75 | 0.88 | 2,355,840 |

The chunk-budget spread is 131.5ms at 720p and **88.9ms at a sixteenth of the
pixels** -- a ratio of 0.676 where overdraw demands 0.0625. It survives the
pixel cut nearly intact, so chunk count costs **geometry, not fragments**, and
**a depth prepass would not help**.

What it does cost is startling: 40.2ns per triangle at 180p and 59.5ns at 720p,
or about 25M triangles a second. An M1000M does not struggle to push 2.4M plain
triangles; that rate is the *terrain vertex shader*, not fixed-function
throughput. The gap between the two rates says roughly a third of the
chunk-scaling cost is fragment-correlated and the rest is per-vertex.

That makes the terrain vertex shader the next thing to measure. It is doing
real work per vertex: a height sample, four more for central-difference
normals, geomorph blending and per-vertex aerial perspective, at 2,304
triangles per chunk on a canonical 33x33 grid that is the same at every LOD.

## Limits

One camera pose. Terrain's dominance will be different at orbit, where the
frontier is a few coarse chunks, and over open ocean. Nothing here was measured
with GPU timestamps, so none of it attributes cost inside a pass.

## Which per-vertex term costs what

`CATINGARDEN_ABLATE=normals,detail,aerial,fog` compiles each term out of the
terrain vertex shader. Each was confirmed to change the rendered image before
being timed -- an ablation that silently does nothing reads as "this term is
free", which is the wrong conclusion. Four blocks, baseline spread 0.36ms.

| condition | frame ms | spread | saving | per-block |
|---|---:|---:|---:|---|
| baseline | 82.28 | 0.36 | | |
| no detail ladder | 74.16 | 0.64 | **8.05** | +7.7 +8.4 +7.6 +8.4 |
| no aerial perspective | 79.80 | 0.29 | 2.49 | +2.4 +2.6 +2.0 +2.6 |
| no normals (4 height samples) | 80.18 | 1.46 | 2.09 | +1.4 +2.9 +1.2 +2.8 |
| no fog | 80.46 | 0.71 | 1.69 | +1.3 +2.0 +1.4 +2.1 |
| none of the four | 68.80 | 1.04 | **13.51** | +13.3 +13.8 +12.5 +13.7 |

The four sum to 14.32ms individually against 13.51ms measured together, so they
are near-additive and the numbers are consistent.

**This partly refutes the previous section.** The chunk-scaling rate suggested
the terrain vertex shader was the cost. It is not, or not mostly: every
per-vertex term that can be named here comes to **13.5ms of terrain's 52.1ms,
about a quarter**. The other **38.6ms is somewhere else** -- the base height
sample and displacement, vertex attribute fetch (the terrain `VertexInput` is
wide), per-draw overhead across 256 chunks, or the fragment shader. That is the
next thing to divide, and nothing here says which.

**On replacing the per-vertex normal with `cross(dpdx, dpdy)`:** the term it
would remove is **2.09ms of an 82ms frame, 2.5%**. Both terrain and ocean were
moved *off* screen-space derivative normals deliberately -- `planet.wgsl:1565`
records f32 cancellation quilting the ground along every chunk boundary, and
`planet.wgsl:1659` records collapse at grazing angles. A true geometric normal
would also shade the skirts as the near-vertical walls they are, at every chunk
edge. 2.5% does not buy that.

**The detail ladder at 8.05ms is four times the normals** and is the per-vertex
term worth looking at, if any of them are.

## Every term priced

`CATINGARDEN_ABLATE` now covers nine terms across the terrain vertex and
fragment shaders. Each was confirmed to change the rendered image before being
timed, except `cloudshadow` -- see the note below. Four blocks each.

| term | stage | ms | of terrain |
|---|---|---:|---:|
| detail ladder | vertex | **8.05** | 15.5% |
| material tint | fragment | **7.80** | 15.0% |
| cloud shadow | fragment | **4.08** | 7.8% |
| aerial perspective | vertex | 2.49 | 4.8% |
| normals (4 height samples) | vertex | 2.09 | 4.0% |
| weather surface sample | fragment | 1.80 | 3.5% |
| fog | vertex | 1.69 | 3.2% |
| sky irradiance | fragment | 1.25 | 2.4% |
| material colour (4-way triplanar) | fragment | 0.61 | 1.2% |
| **all nine together** | | **29.17** | **56%** |
| **still unattributed** | | **~23** | **44%** |

Summed individually the nine come to 29.86ms against 29.17ms measured together,
so they are near-additive and internally consistent.

**The three worth acting on are the detail ladder, the material tint and the
cloud shadow: 19.9ms, a quarter of the whole frame.**

Two results that were not expected:

- **`terrain_material_color` is nearly free at 0.61ms** while
  `terrain_material_tint` is 7.80ms. The four-material triplanar blend was the
  obvious suspect and is not the problem; the close-range detail tint on top of
  it is. The material blend's zero-weight bailouts appear to be doing their job.
- **The cloud shadow costs 4.08ms and changes zero pixels here.** Under a clear
  sky `cloud_shadow_visibility` ray-projects both shells at a three-octave
  budget and returns 1.0. An image diff would have called it free; only the
  timing shows it. Anything that skips it when the weather field is empty along
  the ray is close to a free 5% of frame.

**A methodological note.** The first fragment ablations were placed in
`flat_triangle_lighting` and changed nothing, because that function is never
called: the flat-triangle *fragment* path is gated on the F9 debug mode, not on
`CATINGARDEN_FLAT_TRIANGLES`, and the scenario manifests say
`render_debug_mode: final HDR scene`. Forcing the function to return magenta
produced zero magenta pixels, which is what proved it. Default terrain is
**smooth-shaded** through `input.world_normal`; the faceted look comes from
geometry density, not flat normals.

That also re-prices the `dpdx`/`dpdy` idea: it would not be swapping one flat
normal for another, it would be converting smooth shading into faceted shading,
which is an art change rather than an optimisation, for 2.09ms.

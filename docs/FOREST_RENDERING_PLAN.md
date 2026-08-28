# Procedural forest rendering plan

Date: 28 August 2026
Branch: `experiment/billboard-forest`

## Measured cost of the current forest

The Quadro M1000M driver must not be run with `--profile-render`: enabling timestamp queries is a
known `present()` hang. The forest was therefore measured by five balanced ON/OFF pairs of the
capture-free `forest_performance` scenario in Immediate present mode. `CATINGARDEN_FOREST=0`
suppresses only the draw; instance construction, grounding, upload, and per-frame uniform work stay
active in both cases.

Each run contributed 327 settled frame intervals from simulation time 2.5-7.95s. All ten runs used
the Quadro/Vulkan adapter, Immediate present mode, and passed their finite-metric assertions.

| Pair | Forest on median | Forest off median | Paired cost |
|---:|---:|---:|---:|
| 1 | 27.215 ms | 26.964 ms | 0.251 ms |
| 2 | 27.190 ms | 27.011 ms | 0.179 ms |
| 3 | 27.236 ms | 26.944 ms | 0.292 ms |
| 4 | 27.295 ms | 27.094 ms | 0.201 ms |
| 5 | 27.223 ms | 27.075 ms | 0.148 ms |

The median paired cost is **0.201ms**, with a 0.148-0.292ms range. Median run medians are 27.223ms
(36.73 FPS) with the forest and 27.011ms (37.02 FPS) without it. The present 12,288-tree draw costs
about 0.79% of the frame at this pose. It is already cheap; the procedural system should remain
within the same one-patch/tree budget rather than chasing a misleading micro-optimisation now.

The likely forest cost order is fragment fill/overdraw first, repeated billboard vertex maths
second. The single draw call, 384KiB immutable instance buffer, lack of texture sampling, and opaque
depth writes are not concerns.

Final, alternating-order runs are listed in `test-runs/forest-profile-pairs/final-runs.tsv`; their
artifacts are under `test-runs/forest_performance/1787914342-49660` through
`test-runs/forest_performance/1787914475-50017`.

## One camera-local procedural forest

There should never be a global tree population. The renderer owns exactly one
`Option<ForestPatch>` centred near the player:

1. `TerrainRenderer` exposes a resident-cache-only forest sample containing rendered-compatible
   height, categorical biome, moisture, source level, and a small finite-difference slope. It must
   never perform synchronous disk I/O during movement.
2. A canonical cube-sphere cell key plus planet seed generates stable candidate positions. The
   camera selects a key; camera coordinates must never seed placement. Half-open cell ownership
   prevents duplicates at cube-face and cell seams.
3. Candidates survive only when their footprint is positive land, forest biome, sufficiently
   moist, and below the shoreline/slope limits. Every accepted base uses the same surface-height
   path as camera clearance.
4. Keep the old patch until every source needed by the next patch is resident, then replace the one
   instance buffer atomically. Inner/outer radii, a minimum rebuild interval, and key hysteresis
   prevent rebuild churn. High-speed flight skips intermediate cells.
5. If the camera is outside forest ownership, the patch becomes `None`; no distant forest retains
   geometry.

This makes forest location procedural everywhere the baked biome/moisture field permits while
retaining the current bounded draw cost.

## Tree LOD

LOD should use projected tree height, not terrain source LOD or camera altitude alone:

`projected_pixels = tree_height * viewport_height / (2 * tan(vertical_fov / 2) * distance)`

- **Above 12px:** full deterministic population and current procedural silhouette.
- **3-12px:** stable hash thinning to half/quarter density; keep the same surviving tree identities.
- **1-3px:** sparse cluster billboards or one representative per cell.
- **Below 1px:** no tree geometry.

Use smooth distance bands plus stable hashed thresholds so trees leave at different distances rather
than an entire ring popping at once. CPU patch construction should group instances by cell/LOD tier;
submit only visible ranges. GPU culling is unwarranted unless the paired benchmark later shows CPU
submission or vertex work becoming material.

## Far forest appearance

At distance, the planet already has the right macro ownership data: categorical forest biomes and
bilinear moisture in the terrain shader. Add a distance-faded, direction-hashed canopy colour
breakup only inside forest biome ownership, suppressed on water, snow, and steep slopes. This is a
fragment material treatment, not tree geometry and not a second forest draw. It gives orbit views
the shapes of forests while the local `ForestPatch` provides individual trees only where a player
can resolve them.

No new bake channel is required for the first pass. A later authored forest-density channel is only
justified if biome-plus-moisture masks lack enough control.

## Implementation order and acceptance

1. Add/test the resident forest surface sample.
2. Replace the fixed centre with one deterministic patch key and rebuild hysteresis.
3. Add projected-size LOD and bounded tier counts.
4. Add the forest-only far material breakup.
5. Add a travelling scenario that asserts active patch count <= 1, bounded rebuild rate, no
   water/slope trees, finite positions, and stable LOD counts; repeat the paired Quadro benchmark.

The current authored forest remains the visual reference until each step passes independently.

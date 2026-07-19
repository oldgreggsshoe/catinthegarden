# Generated planet assets

`assets/outmaps/` is generated and gitignored. Build the deterministic test
planet with:

```bash
cargo run --release -p catinthegarden-baker
```

The default bake writes `assets/outmaps/test-planet/manifest.json`, three
equirectangular PNG previews, and raw cube-face terrain tiles. The manifest is
the source of truth for available tile keys. Levels 0-4 are globally dense;
the baker-selected dry coastal inspection area has a parent-complete sparse
refinement chain through level 18. Runtime lookup may fall back to the nearest
available parent.

Each tile contains a 129x129 logical grid stored as 131x131 samples with a one
sample gutter on every side:

- `height.r32f`: little-endian signed `f32` meters;
- `biome.r8`: `BiomeId` values from `catinthegarden-coretypes`;
- `moisture.r8`: normalized 0-255 moisture.

Normals are intentionally absent and must be derived from height samples on
the GPU. Every non-root logical border is recursively constrained to bilinear
values from its immediate parent tile. Fine tiles therefore meet a neighboring
chunk's nearest available ancestor without a height discontinuity; biome and
moisture borders use the corresponding nearest/bilinear parent values. After
terrain generation, the baker deterministically chooses a dry, non-polar
coastal site with useful local relief and centres the sparse refinement chain
there. A 500m ramp suppresses added L3+ relief at the exact inspection point,
and sparse tile interiors gain seamless 3D microrelief whose ramp is zero at
level 12 and reaches a bounded +/-2m at level 18. The microrelief is exactly
zero at the selected centre and is applied before the parent-border constraint.

Validate an existing bake with:

```bash
cargo run --release -p catinthegarden-baker -- \
  --validate assets/outmaps/test-planet
```

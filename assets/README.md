# Generated planet assets

`assets/outmaps/` is generated and gitignored. Build the deterministic test
planet with:

```bash
cargo run --release -p catinthegarden-baker
```

The default bake writes `assets/outmaps/test-planet/manifest.json`, three
equirectangular PNG previews, and raw cube-face terrain tiles. The manifest is
the source of truth for available tile keys. Levels 0-2 are globally dense;
the `+X` landing area has a parent-complete sparse refinement chain through
level 18. Runtime lookup may fall back to the nearest available parent.

Each tile contains a 33x33 logical grid stored as 35x35 samples with a one
sample gutter on every side:

- `height.r32f`: little-endian signed `f32` meters;
- `biome.r8`: `BiomeId` values from `catinthegarden-coretypes`;
- `moisture.r8`: normalized 0-255 moisture.

Normals are intentionally absent and must be derived from height samples on
the GPU. Every non-root logical border is recursively constrained to bilinear
values from its immediate parent tile. Fine tiles therefore meet a neighboring
chunk's nearest available ancestor without a height discontinuity; biome and
moisture borders use the corresponding nearest/bilinear parent values. The
baker also smooths the exact `+X` landing point to -10m so the nominal 10m
descent scenario remains above terrain while Phase 4 is integrated. Sparse
tile interiors gain deterministic seamless 3D microrelief from level 12,
ramping to a bounded +/-2m at level 18; the term is exactly zero at `+X` and
is applied before the parent-border constraint.

Validate an existing bake with:

```bash
cargo run --release -p catinthegarden-baker -- \
  --validate assets/outmaps/test-planet
```

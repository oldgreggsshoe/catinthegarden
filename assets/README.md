# Generated planet assets

`assets/outmaps/` is generated and gitignored. Build the deterministic test
planet with:

```bash
cargo run --release -p catinthegarden-baker
```

The default bake writes `assets/outmaps/test-planet/manifest.json`, three
equirectangular PNG previews, and raw cube-face terrain tiles. The manifest is
the source of truth for available tile keys. Levels 0-4 are globally dense;
the `+X` landing area has a parent-complete sparse refinement chain through
level 18. Runtime lookup may fall back to the nearest available parent.

Each tile contains a 129x129 logical grid stored as 131x131 samples with a one
sample gutter on every side:

- `height.r32f`: little-endian signed `f32` meters;
- `biome.r8`: `BiomeId` values from `catinthegarden-coretypes`;
- `moisture.r8`: normalized 0-255 moisture.

Normals are intentionally absent and must be derived from height samples on
the GPU. Every non-root logical border is recursively constrained to bilinear
values from its immediate parent tile. Fine tiles therefore meet a neighboring
chunk's nearest available ancestor without a height discontinuity; biome and
moisture borders use the corresponding nearest/bilinear parent values. The
baker forces terrain within 2 degrees of `+X` to -10m and blends back to the
generated surface by 6 degrees so the nominal 10m descent scenario remains
above terrain. A separate 500m ramp suppresses added L3+ detail near the exact
landing point. Sparse tile interiors gain deterministic seamless 3D
microrelief whose ramp is zero at level 12 and reaches a bounded +/-2m at level
18; the term is exactly zero at `+X` and is applied before the parent-border
constraint.

Validate an existing bake with:

```bash
cargo run --release -p catinthegarden-baker -- \
  --validate assets/outmaps/test-planet
```

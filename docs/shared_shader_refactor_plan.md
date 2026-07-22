# Refactor Plan: `shared_planet.wgsl`

Companion to the foveated raytrace guide. Execute this immediately after M0 and
before building the ray field arrays: pull the reusable lighting, scattering,
ocean, and material math out of `planet.wgsl` so the raymarch shader can later
call the exact same functions, without changing a single pixel of raster output.
During all three stages below, raster remains the only consumer.

This is the one refactor in the whole project with real regression risk, because
it moves code the rasteriser depends on. So the whole plan is built around one
guarantee: **the raster path stays byte-identical, and we prove it with a golden
image after every stage.**

The good news, after reading the actual shader: the seam is narrow. Almost
everything in `planet.wgsl` is pure math that only reads the `camera` uniform and
constants. Only six functions touch textures, and only four of those touch the
per-tile textures that actually differ between the two paths.

---

## 1. The seam, measured

Every function in `planet.wgsl`, classified by what it touches. This is the whole
map. The three buckets are the plan.

### Bucket A: pure math (move to shared, verbatim)

These read only the `camera` uniform, the `terrain_settings` uniform, and
module constants. No texture sampling. They are safe to move as-is.

| Function | Notes |
|---|---|
| `planet_to_view`, `view_to_planet` | camera basis rotates |
| `placeholder_octave`, `placeholder_height` | procedural, pure |
| `gerstner_wave`, `ocean_surface` | wave displacement, pure |
| `density`, `phase_rayleigh`, `phase_mie`, `twilight_solar_air_mass` | scattering primitives |
| `transmittance`, `atmosphere_interval`, `atmosphere_exit_distance`, `altitude_along_ray` | ray/atmosphere geometry |
| `sun_is_occluded`, `sun_visibility`, `surface_direct_sun_transmittance` | sun occlusion |
| `sky_radiance`, `sky_diffuse_irradiance` | sky scattering |
| `aerial_view_transmittance`, `aerial_density_sample_fraction`, `aerial_perspective` | aerial perspective |
| `ocean_aerial_perspective`, `terrain_distance_fog` | fog/aerial over surfaces |
| `face_tangent_u`, `face_tangent_v`, `face_normal`, `face_component` | cube face geometry |
| `srgb_to_linear`, `biome_color`, `biome_vegetation_amount` | colour helpers, pure |
| `terrain_macro_height_scale` | reads `terrain_settings` uniform, pure otherwise |
| `terrain_material_weights_for_biome`, `height_blend_material_weights` | material weight math, pure |
| `outmap_ocean_coverage`, `debug_ocean_albedo` | pure |
| `ocean_with_aerial_perspective` | pure, calls shared functions only |

### Bucket B: touches a shared resource (move to shared, needs a shared binding)

These sample textures that are the same data for both paths (a reflection cubemap
and a material array), not per-tile data. They can be shared once those two
resources live in a bind group both pipelines bind.

| Function | Resource touched | Line |
|---|---|---|
| `triplanar_material_sample_at_position`, `triplanar_material_sample` | `terrain_material_map` (array, keyed by planet direction) | ~1002 |
| `ocean_lighting` | `environment_map` (reflection cube) | ~1254 |

### Bucket C: path-specific (do NOT move, each path implements its own)

These sample the per-tile field textures via `source_uv`. This is the only real
difference between raster and ray. Raster reads 2D tile textures. Ray reads
six-layer, guttered 2D field arrays by direction.

| Function | Resource | Line |
|---|---|---|
| `sample_height` | `height_map` | 176 |
| `sample_biome` | `biome_map` | 754 |
| `sample_biome_blend` | `biome_map` | 768 |
| `sample_moisture` | `moisture_map` | 798 |
| `displaced_surface_normal` | calls `sample_height` at offsets | 855 |

### Bucket D: raster-only, stays in `planet.wgsl`

Instance unpacking, LOD stitching, entry points. The ray path has no equivalent.

`uses_outmap`, `cube_face`, `requested_level`, `edge_stitch_level_delta`,
`snap_edge_coordinate`, `stitched_tile_uv`, `lod_morphed_tile_uv`,
`stitched_surface_direction`, `lod_dither_threshold`, `vs_main`, `fs_main`,
`fs_main_stable`.

### The awkward four (mostly pure, sample only at the top)

These live in Bucket A after a tiny change. Right now they take `source_uv` and
call a Bucket C sampler on the first line, then do pure math. Lift the sample to
the caller, pass the value in, and they become pure and shareable.

| Function | Samples at top | Becomes pure if it takes |
|---|---|---|
| `blended_biome_color` | `sample_biome_blend` | a `BiomeBlendSample` param |
| `terrain_material_color` | `sample_biome`, `sample_moisture`, `blended_biome_color` | `biome`, `moisture`, `base_color` params |
| `terrain_material_weights` | `sample_biome_blend` | a `BiomeBlendSample` param |
| `terrain_material_tint` | `sample_moisture` (+ triplanar, which is Bucket B) | a `moisture` param |

Lifting a sample to the caller does not change any result value. Texture
sampling is deterministic, so sampling the same texel once and passing the value
is identical to sampling it inside. This is what keeps the output byte-identical.

---

## 2. The design: sample locally, shade shared

The rule that falls out of the seam:

> **Path-specific code samples the terrain fields. Shared code never samples the
> terrain fields.** Everything shared operates on values already fetched.

Concretely, each path fills a small struct, then hands it to shared shading.

```wgsl
// lives in shared_planet.wgsl
struct SurfaceSample {
    direction:      vec3<f32>,   // unit planet direction of the shaded point
    normal:         vec3<f32>,   // surface normal, planet space
    height_meters:  f32,         // macro terrain height at the point
    detail_meters:  f32,         // bounded relief for material break-up
    biome:          u32,
    biome_blend:    BiomeBlendSample,
    moisture:       f32,
    view_position:  vec3<f32>,   // camera-relative view position of the point
    is_outmap:      bool,
}
```

Raster fills this inside `fs_main` from its interpolants and `source_uv`
samples. The raymarcher fills it from its hit point and face-array samples. From
there the shared shading is identical.

```wgsl
// lives in shared_planet.wgsl, called by both paths
fn shade_terrain_surface(s: SurfaceSample, sun_direction: vec3<f32>) -> vec3<f32> {
    let base = terrain_material_color(
        s.is_outmap, s.biome, s.moisture, blended_biome_color(s.biome_blend),
        s.height_meters, s.detail_meters, s.normal, s.direction,
    );
    let tint = terrain_material_tint(
        s.is_outmap, s.moisture, s.biome_blend, s.height_meters,
        base, s.direction, s.normal, s.view_position,
    );
    // ... surface irradiance, aerial perspective, exactly as fs_main does today
    return /* aerial-composited colour */;
}
```

Note `terrain_material_color` and `terrain_material_tint` here take fetched
values (`biome`, `moisture`, `biome_blend`, `base_color`) instead of `source_uv`.
That is the awkward-four change from section 1.

---

## 3. Module assembly

wgpu has no `#include`. You build each shader module by concatenating source
strings before `create_shader_module`. Two modules get built from shared parts.

### File layout

```
shared_planet.wgsl     Bucket A + B functions, SurfaceSample, shared bindings, constants
raster_impl.wgsl       Bucket C samplers (2D tile textures) + Bucket D + vs_main + fs_main
ray_impl.wgsl          Bucket C samplers (six-layer field arrays) + raymarch entry
```

### Assembled modules

```
raster module = shared_planet.wgsl  ++  raster_impl.wgsl
ray module    = shared_planet.wgsl  ++  ray_impl.wgsl
```

### Rust side

```rust
fn build_module(device: &wgpu::Device, label: &str, parts: &[&str]) -> wgpu::ShaderModule {
    let source = parts.join("\n");
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

const SHARED: &str = include_str!("shared_planet.wgsl");
const RASTER: &str = include_str!("raster_impl.wgsl");
const RAY:    &str = include_str!("ray_impl.wgsl");

let raster_module = build_module(&device, "planet-raster", &[SHARED, RASTER]);
let ray_module    = build_module(&device, "planet-ray",    &[SHARED, RAY]);
```

WGSL is order-independent at module scope, so a shared function can reference a
binding or sampler function that the impl file declares later in the assembled
string. That is what lets `shade_terrain_surface` (shared) call `sample_biome`
(impl) even though the shared text comes first.

If the string-concat line numbers make shader errors hard to read, that is the
moment to adopt `naga_oil` and its `#import`. Not before.

### Binding groups

This is the part that needs care, because moving a binding changes the Rust
pipeline layout even when it does not change any sampled value.

- `group(0)`: `camera`. Unchanged, both paths.
- `group(1)`: **path-specific field textures.** Raster binds
  `height_map`, `biome_map`, `moisture_map` (the current per-tile 2D textures).
  Ray binds `height_faces`, `biome_faces`, `moisture_faces`. Same group index,
  different layout, different pipeline, which is fine.
- `group(2)`: **shared resources.** `environment_map`, `environment_sampler`,
  `terrain_material_map`, `terrain_material_sampler`, `terrain_settings`. Both
  paths bind this identically. `shared_planet.wgsl` declares these.

Today those five shared resources live in `group(1)` mixed with the per-tile
textures. Moving them to `group(2)` is the only binding renumber in the whole
refactor, and it is why the staged plan below does it last, alone, behind the
golden test.

---

## 4. Function transforms, before and after

The awkward four, with exact signature changes. Move the bodies verbatim. Do not
tidy the math. Reordering float operations is the one way to break byte-identity.

### `blended_biome_color`

```wgsl
// before (samples inside)
fn blended_biome_color(source_uv: vec2<f32>) -> vec3<f32> {
    let blend = sample_biome_blend(source_uv);
    // ... mix biome_color(blend.ids.*) by blend.weights.* ...
}

// after (pure, caller samples)
fn blended_biome_color(blend: BiomeBlendSample) -> vec3<f32> {
    // ... identical body from `let ... =` onward, using the `blend` param ...
}
```

### `terrain_material_weights`

```wgsl
// before
fn terrain_material_weights(source_uv, moisture, macro_height, normal, direction) -> vec4<f32> {
    let blend = sample_biome_blend(source_uv);
    return terrain_material_weights_for_biome(blend.ids.x, ...);
}

// after
fn terrain_material_weights(blend: BiomeBlendSample, moisture, macro_height, normal, direction) -> vec4<f32> {
    return terrain_material_weights_for_biome(blend.ids.x, ...);   // body unchanged
}
```

### `terrain_material_color`

```wgsl
// before: takes source_uv, calls sample_biome / sample_moisture / blended_biome_color
fn terrain_material_color(outmap, source_uv, macro_height, detail, normal, direction) -> vec3<f32>

// after: takes the fetched values
fn terrain_material_color(
    outmap: bool,
    biome: u32,
    moisture: f32,
    base_color: vec3<f32>,     // = blended_biome_color(blend), fetched by caller
    macro_height_meters: f32,
    terrain_detail_meters: f32,
    surface_normal: vec3<f32>,
    surface_direction: vec3<f32>,
) -> vec3<f32> {
    var color = vec3<f32>(0.32, 0.58, 0.74);
    if !outmap { return color; }
    color = base_color * mix(0.88, 1.06, moisture);   // was blended_biome_color(source_uv) * ...
    // ... rest of the body verbatim, `biome` and `moisture` now params ...
}
```

### `terrain_material_tint`

```wgsl
// after: moisture passed in; still calls the shared triplanar (Bucket B)
fn terrain_material_tint(
    outmap: bool,
    moisture: f32,
    blend: BiomeBlendSample,
    macro_height_meters: f32,
    base_albedo: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_normal: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
) -> vec3<f32> {
    // body unchanged except:
    //   `let moisture = sample_moisture(source_uv);`  is removed (now a param)
    //   `terrain_material_weights(source_uv, ...)`    becomes `terrain_material_weights(blend, ...)`
}
```

The raster caller (`fs_main`) fetches once and passes:

```wgsl
let blend    = sample_biome_blend(input.source_uv);
let biome    = blend.ids.x;                    // or sample_biome, whichever fs used
let moisture = sample_moisture(input.source_uv);
let base     = blended_biome_color(blend);
let albedo   = terrain_material_color(input.outmap > 0.5, biome, moisture, base, ...);
let tint     = terrain_material_tint(input.outmap > 0.5, moisture, blend, ..., albedo, ...);
```

One subtlety to preserve: `fs_main` today samples biome and moisture more than
once (once inside `terrain_material_color`, again for the ice check, again via
coverage). After the refactor you sample once and reuse. Same values, fewer
fetches. That is a small perf win and it does not change output.

---

## 5. The two sampler implementations

Bucket C, one version per path. Same names, same signatures, different bodies.
Shared code calls these by name and does not care which is linked.

### Raster (`raster_impl.wgsl`) keeps the current bodies

```wgsl
@group(1) @binding(0) var height_map:   texture_2d<f32>;
@group(1) @binding(1) var biome_map:    texture_2d<u32>;
@group(1) @binding(2) var moisture_map: texture_2d<f32>;

fn sample_height(source_uv: vec2<f32>) -> f32 { /* current body, textureLoad on height_map */ }
fn sample_biome(source_uv: vec2<f32>) -> u32 { /* current body */ }
fn sample_biome_blend(source_uv: vec2<f32>) -> BiomeBlendSample { /* current body */ }
fn sample_moisture(source_uv: vec2<f32>) -> f32 { /* current body */ }
```

### Ray (`ray_impl.wgsl`) samples face arrays by direction

The ray path has a planet direction, not a tile uv. So its samplers take a
direction. To keep the shared code path-agnostic, define the samplers on
`direction` and have the raster ones ignore the extra arg, or (cleaner) keep the
shared `SurfaceSample` filling entirely inside each impl so the shared code only
ever sees the finished struct. Recommended: each impl provides one function
`sample_surface(...) -> SurfaceSample`, and the shared code starts from the
struct.

Use one `texture_2d_array` layer per cube face and include a one-texel gutter
filled from the neighboring face. Port `direction_to_face_uv` from `coretypes`
to WGSL so each sample resolves to an explicit `(face, uv)`.

The portable height path is `R32Float` with four `textureLoad` taps and manual
bilinear interpolation. Filtered `R32Float` requires `FLOAT32_FILTERABLE`, and
`R16Unorm` requires `TEXTURE_FORMAT_16BIT_NORM` in wgpu 29, so neither may be
assumed. Moisture uses filterable `R8Unorm`; biome uses `R8Uint`, nearest for
ownership and an explicit four-tap blend.

```wgsl
@group(1) @binding(0) var height_faces:   texture_2d_array<f32>;
@group(1) @binding(1) var biome_faces:    texture_2d_array<u32>;
@group(1) @binding(2) var moisture_faces: texture_2d_array<f32>;
@group(1) @binding(3) var field_sampler:  sampler;

fn sample_surface_ray(dir: vec3<f32>, view_pos: vec3<f32>, mip: f32) -> SurfaceSample {
    var s: SurfaceSample;
    s.direction     = dir;
    s.height_meters = sample_height_faces_manual_bilinear(dir, mip)
                        * terrain_macro_height_scale();
    s.biome         = sample_biome_face_nearest(dir);
    s.biome_blend   = sample_biome_faces_four_tap(dir);
    s.moisture      = sample_moisture_faces(dir, mip);
    s.normal        = /* gradient of height_faces, section 8.4 of the main guide */;
    s.detail_meters = /* optional runtime detail, or 0 to start */;
    s.view_position = view_pos;
    s.is_outmap     = true;
    return s;
}
```

Match the ray biome blend to the raster `sample_biome_blend` shape (four
neighbor taps, same weight math) so `blended_biome_color` produces consistent
color across both paths. The gutter keeps all four taps valid at face edges. It
will not be pixel-identical because the data source differs, but the blend and
coastline semantics must match.

---

## 6. Staged migration, in order, each stage golden-tested

Do not do this in one commit. Three stages, each independently verifiable. The
first two change zero bindings, so they carry almost no risk.

### Stage 1: extract pure math (Bucket A), zero binding changes

- Create `shared_planet.wgsl` with the `camera` struct + `group(0)` binding, all
  module constants, and every Bucket A function moved verbatim.
- Delete those functions from `planet.wgsl`.
- Build the raster module as `SHARED ++ planet_impl.wgsl` where
  `planet_impl.wgsl` is the remainder of the old `planet.wgsl` (buckets B, C, D).
- No binding moves yet. The shared-resource samplers (Bucket B) and the field
  samplers (Bucket C) all stay in `planet_impl.wgsl` for now with their current
  `group(1)` bindings.

**Golden test.** Use the target Quadro baseline captured before Stage 1. Render
the fixed scenario, hash every PNG, and compare with the pre-refactor capture.
Must match. If it does not, diff the images and find the operation or binding
that changed.

### Stage 2: parameterize the awkward four (Bucket A completion)

- Apply the signature changes from section 4. The awkward four move to
  `shared_planet.wgsl`. Their callers in `fs_main` fetch and pass values.
- Still no binding moves. Triplanar and `ocean_lighting` stay in
  `planet_impl.wgsl` for the raster module.

**Golden test.** Same scenario, same hash. Must still match.

### Stage 3: move the shared resources to `group(2)`, share Bucket B

- Move `environment_map`, `environment_sampler`, `terrain_material_map`,
  `terrain_material_sampler`, `terrain_settings` from `group(1)` to a new
  `group(2)` declared in `shared_planet.wgsl`.
- Move `triplanar_material_sample*` and `ocean_lighting` into
  `shared_planet.wgsl`.
- Update the Rust side: split the old terrain bind group layout into a
  `group(1)` per-tile layout and a `group(2)` shared layout. Update
  `create_gpu_tile` and wherever the terrain bind group is built (in
  `terrain.rs`) to bind `group(2)` once and `group(1)` per tile.
- The raster pipeline layout is now `[camera, per-tile, shared]`. The ray
  pipeline layout is `[camera, face-arrays, shared]`.

**Golden test.** This is the stage that can break, because you rewired bind
groups. Same scenario, same hash. If colour is right but slightly off, you likely
swapped a sampler or a binding index. Diff and check.

After Stage 3, `shared_planet.wgsl` is complete and the ray module can be built
as `SHARED ++ ray_impl.wgsl`. Milestone M3 of the main guide can now call the
shared shading from the raymarcher.

---

## 7. The golden test, concretely

You need a deterministic rendered scene to compare. The repo already has a
scenario system (`scenario.rs`) and a capture path. Do **not** use `still_5s`:
that scenario deliberately renders a solid test colour and cannot detect a
terrain shader regression. Use `polar_ice_cap`, which fixes its camera, sun, and
planet rotation and schedules one capture after exposure settles. Capture the
baseline on the target Quadro, with the same driver, executable settings,
internal render size, and scenario data that will be used after every stage.

```bash
export CARGO_TARGET_DIR=/home/dad/catingard-target

# Baseline before Stage 1, then repeat after every stage.
cargo run --release -p catinthegarden-app -- \
    --scenario polar_ice_cap
```

The run is written under `test-runs/polar_ice_cap/<run_id>/`. Record the baseline
manifest and hashes of all files under `screenshots/` outside the run directory,
then compare the corresponding captures after each stage. Do not use a baseline
from another GPU or driver.

If the hash differs, do not panic and do not assume it is wrong. Make a diff
image:

```python
from PIL import Image, ImageChops
a = Image.open("baseline.png"); b = Image.open("after.png")
diff = ImageChops.difference(a, b)
print("max channel diff:", max(diff.getextrema(), key=lambda e: e[1]))
diff.save("diff.png")
```

- Zero diff: done, move on.
- A diff of 1 on a few pixels, scattered: almost certainly float reassociation
  from moving a function. Find the moved function, restore its exact operation
  order, re-test. Aim for zero. Accept 1/255 on isolated pixels only if you have
  confirmed the cause is benign reassociation and not a logic change.
- A structured diff (a whole region, a colour shift): a real bug. You passed the
  wrong value, sampled the wrong texel, or swapped a binding. Fix it, do not
  accept it.

Run the golden test on the same GPU each time. Float results can differ between
drivers, so the baseline and the after-shot must come from the same machine.

The current tests parse `planet.wgsl` directly. Once the source is split, add a
single assembly helper used by both runtime pipeline creation and tests. Naga
must parse and validate the exact `SHARED ++ RASTER` string after every stage;
tests must not continue validating only one fragment of the assembled module.
Keep existing source-content regressions pointed at the assembled raster source
where their symbols have moved.

---

## 8. Pitfalls specific to this refactor

- **Do not reorder float math.** Moving `a + b + c` to `a + (b + c)` changes the
  low bit. Move bodies verbatim. This is the single most likely cause of a failed
  golden test.
- **Constants must move with their functions, and only once.** If a constant like
  `GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS` is used by a shared function, move it
  to `shared_planet.wgsl`. Do not leave a copy in `planet_impl.wgsl`, WGSL will
  error on the duplicate at module scope. Grep for each constant before you move
  it.
- **`BiomeBlendSample` is now shared.** It is used by shared `blended_biome_color`
  and `terrain_material_weights`, so its struct definition moves to
  `shared_planet.wgsl`. The ray impl fills it too.
- **`terrain_settings` is shared.** `terrain_macro_height_scale` reads it and both
  paths need the same height scaling, so it moves to `group(2)`. The ray path must
  bind the same buffer.
- **Sampler count is not correctness.** After the refactor you sample biome and
  moisture fewer times per fragment. Values are identical, so output is identical.
  Do not "preserve" the redundant samples to make the golden test pass, that hides
  a different bug.
- **Watch the `input.*` coupling in `fs_main`.** The raster surface fill reads
  interpolants (`input.world_normal`, `input.surface_direction`,
  `input.terrain_detail_meters`, `input.surface_irradiance`,
  `input.aerial_color`, `input.surface_lighting`). These are vertex-interpolated
  and have no ray equivalent. Keep the assembly of `SurfaceSample` from
  interpolants inside `fs_main`, not in shared code. Shared code starts at the
  filled struct.

---

## 9. Final map

```
shared_planet.wgsl
  struct Camera            @group(0) @binding(0)
  group(2): environment_map/sampler, terrain_material_map/sampler, terrain_settings
  constants (all shared)
  struct BiomeBlendSample, OceanSurface, SurfaceSample
  Bucket A functions (scattering, ocean surface, face geom, colour, weights, ...)
  Bucket B functions (triplanar_material_sample*, ocean_lighting)
  shade_terrain_surface(SurfaceSample, sun) -> vec3   // the shared entry both paths call

raster_impl.wgsl
  group(1): height_map, biome_map, moisture_map        (per-tile 2D)
  sample_height/biome/biome_blend/moisture             (Bucket C, current bodies)
  displaced_surface_normal, LOD stitching (Bucket D)
  vs_main, fs_main  -> fills SurfaceSample from interpolants + samples, calls shade_terrain_surface

ray_impl.wgsl
  group(1): height_faces, biome_faces, moisture_faces, field_sampler
  sample_surface_ray(dir, view_pos, mip) -> SurfaceSample
  raymarch entry -> fills SurfaceSample at the hit, calls shade_terrain_surface
```

Build order for the whole thing: Stage 1, golden. Stage 2, golden. Stage 3,
golden. Then, and only then, wire `shade_terrain_surface` into the raymarcher for
M3 of the main guide. Do not add ray consumers during these three stages. If any
golden test fails, fix it before the next stage, so a break is always one small,
recent change away.

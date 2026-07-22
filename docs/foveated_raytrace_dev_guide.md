# Cat in the Garden: Foveated Raytrace Render Mode

A build guide to go from the current raster planet renderer to a second,
toggleable rendering path: a low-resolution foveated raymarcher. The raster
path stays exactly as it is and remains the default. The new path is a parallel
pipeline you switch on with a key.

This is written to be built in order. Every milestone leaves you with something
that runs. If a milestone breaks, you back up one step, you do not have a
half-rewritten renderer on your hands.

---

## 0. What we are building and the ground rules

**The feature.** Press a key (this guide uses F5, which is currently free) and
the planet is drawn by a raymarcher instead of the rasteriser. The raymarcher
renders into a small buffer that is dense in the middle and sparse toward the
edges, then unwarps that buffer up to full screen. Center stays sharp, the
periphery goes soft, and you lean on peripheral vision to not notice.

**Three rules that keep this sane:**

1. **Never touch the raster path's behaviour.** All new code sits behind the
   toggle. With the toggle off, the frame is byte-for-byte what it is today.
2. **Reuse the back half of the frame.** The raymarcher writes HDR linear colour
   into the same `hdr.scene_view()` target the raster path writes into. That
   means auto-exposure, bloom, tone mapping, the sun overlay, and egui all keep
   working with zero changes. You are only replacing the front of the frame (the
   part that fills the HDR scene), not the whole thing.
3. **Build the plain version before the clever version.** Full-res raymarch
   first, then warp. Separable warp first, then log-polar. Fixed steps first,
   then adaptive. Get each layer boring and correct before stacking the next.

**Target.** Quadro M1000M, vsync. The current window requests 1280x720, while
`self.size` is the authoritative internal render size; borderless fullscreen
preserves that internal size and scales only the final presentation. All ray and
warp resources therefore derive from `self.size` and are rebuilt alongside HDR
and depth in `resize_render_targets()`. Foveation buys progressively more as the
internal resolution rises, so adaptive stepping must be measured before the
warp is justified.

---

## 1. The frame you have today (recap, with real symbols)

From `crates/app/src/main.rs`, `render()` does roughly this each frame:

1. Acquire the swapchain texture.
2. Begin the "cube-sphere pass" into `self.hdr.scene_view()` (format
   `HdrRenderer::SCENE_FORMAT` = `Rgba16Float`) with `self.depth_view`
   (`Depth32Float`, reversed-Z, cleared to `0.0`).
   - `self.atmosphere.draw(...)` fills the sky as a fullscreen pass.
   - `self.terrain.draw(...)` draws the instanced terrain chunks on top.
3. `self.hdr.encode_luminance(...)` and the readback ring drive auto-exposure.
4. A "visual sun overlay pass" draws the sun disc into the scene target with
   `LoadOp::Load`, depth-tested against the same `depth_view`.
5. The HDR chain (`encode_blur`, `encode_bloom`, `encode_tone_map`) resolves the
   scene into the swapchain.
6. egui draws the debug overlay.

The camera comes from `planet::CameraUniform::from_camera(...)`, whose fields
you will lean on:

```rust
pub struct CameraUniform {
    pub projection_matrix: [[f32; 4]; 4],
    pub camera_forward: [f32; 4],
    pub camera_right: [f32; 4],
    pub camera_up: [f32; 4],
    pub camera_planet_direction_view_altitude: [f32; 4], // xyz = planet dir, w = altitude
    pub sun_direction: [f32; 4],
    pub sun_direction_view: [f32; 4],
    pub projection: [f32; 4],   // packs render_debug_mode in .w, time in .z, etc.
}
```

Note two gifts already in that struct: the camera's planet direction and its
altitude. The raymarcher wants both.

Keybinds are a flat set of match arms at the bottom of `main.rs`. Each one calls
a small method (`toggle_blur`, `cycle_render_debug_mode`, and so on) that flips a
field and calls `self.mark_hud_dirty()`. You will add one more arm exactly like
these.

---

## 2. The frame you are building (ray path)

With the toggle on, step 2 above is replaced by two new sub-steps, and
everything from step 3 onward is untouched:

- **2a. Raymarch pass.** A fullscreen fragment shader marches the planet and
  writes HDR colour into a small **warped** offscreen HDR texture
  (`warp_color`, `Rgba16Float`, at dimensions derived from the active render
  size). It also writes a hit distance
  into a second small target (`warp_dist`, `R32Float`) so the unwarp pass can
  reconstruct depth.
- **2b. Unwarp pass.** A fullscreen fragment shader reads `warp_color` /
  `warp_dist`, applies the inverse warp per screen pixel, writes colour into
  `hdr.scene_view()`, and writes reversed-Z depth into `self.depth_view` via
  `@builtin(frag_depth)`.

After 2b, `hdr.scene_view()` and `depth_view` look just like the raster path
filled them, so the sun overlay, exposure, bloom, and tone map all just work.

```
raster path:   [atmosphere fullscreen] -> [terrain instanced]  ->  scene + depth
ray path:      [raymarch -> warp_color/warp_dist] -> [unwarp]   ->  scene + depth
                                                    (rest of frame identical)
```

---

## 3. The decisions that matter, and why

### 3.1 Where does the ray shader get terrain height?

This is the real design fork. The raster path binds one tile texture per draw
call. A ray marches arbitrary directions and needs height for any direction at
any moment. You cannot bind per-tile textures for that.

**Chosen approach: whole-planet six-face texture arrays.** Build one
`texture_2d_array` per field with six layers, one per cube face, and a one-texel
gutter around each face. Build it once at load by sampling the outmap. This fits
the data reality: the planet is dense only to level 4 (~6 km per sample), so a
2048-per-face array already over-resolves the real signal.

Do not rely on filtered `R32Float`: wgpu requires `FLOAT32_FILTERABLE`, which is
not guaranteed. `R16Unorm` is also not a universal fallback in wgpu 29 because
it requires `TEXTURE_FORMAT_16BIT_NORM`. The portable baseline is:

- Height: `R32Float`, sampled with four `textureLoad` taps and manual bilinear
  interpolation. Store metres directly.
- Moisture: `R8Unorm`, filtered within the selected array layer.
- Biome: `R8Uint`, nearest for ownership and the same explicit four-tap blend
  used by the raster path.

An adapter-probed filtered-height fast path can be added later, after the
portable path is correct and measured.

- Pros: one field bind group, portable format behavior, explicit face mapping,
  and full control over interpolation and max-height mips.
- Cons: it flattens away the sparse deep-detail landing tiles. For a first
  version that is fine, the whole planet is coarse anyway. You can add a virtual
  texture later if you want the landing site crisp in ray mode too.

Port `direction_to_face_uv` from `coretypes` to WGSL. Runtime sampling maps a
direction to one layer plus UV. Build each gutter from directions just beyond
that face edge so its texels already contain the adjacent face's values; manual
height and biome taps then remain local to one layer without runtime cross-face
neighbor logic.

**Scale-up path (later, optional):** replace the fixed face arrays with a
clipmap or a tile-atlas plus indirection texture so the ray can reach sparse
deep tiles. Note it and move on.

### 3.2 Fragment shader or compute?

**Start with a fullscreen fragment shader** writing to offscreen attachments. It
needs no storage-image features, it runs anywhere wgpu runs, and it slots into
the same render-pass machinery you already use. Move to a compute shader only if
you later want to scatter rays in a pattern that does not map to a pixel grid.
For a warped buffer the pixel grid is exactly what you want, so fragment is the
right first tool.

### 3.3 Precision at 4,000 km

Marching from a camera at ~4e6 m with f32 loses the low bits. The fix is to
never form the big cancelling quantities directly. Instead of `length(C + tD)`,
use the quadratic in `t`:

```
r(t)^2 = |C|^2 + 2 t (C . D) + t^2
```

and get `|C|` from the altitude the uniform already carries
(`|C| = PLANET_RADIUS + altitude`, both precise) rather than from a giant world
vector. At low altitude this lands you around 1 m of radial precision, which is
plenty for a low-res look. If you see banding right at the deck, upgrade to a
two-float (hi/lo) split of the camera position for the dot products. Detail is
in section 9. Do not pre-optimise this, ship the simple version and only split
if you actually see the banding.

### 3.4 Sharing shader code with the raster path

`planet.wgsl` already has the functions you want to reuse: atmosphere scatter,
ocean surface and lighting, terrain material colour, sun transmittance, sky
diffuse. Duplicating them is how they drift out of sync.

wgpu has no native `#include`, so do one of:

- **Simple:** pull the shared functions into `shared_planet.wgsl`, and at
  pipeline-build time concatenate that string in front of both `planet.wgsl` and
  the new `raymarch.wgsl` before calling `create_shader_module`.
- **Nicer:** adopt `naga_oil` and use its `#import` directives.

Start with string concatenation. It is ten lines and it works today.

One catch: the shared functions currently assume raster inputs (interpolated
`source_uv`, per-tile textures). Complete the protected refactor in
`shared_shader_refactor_plan.md` immediately after M0, while raster is still the
only consumer. Only after its target-GPU golden captures pass should ray data or
shaders depend on it. Shared shading takes already-fetched values; each path
keeps its own field sampling implementation.

---

## 4. New GPU resources and uniforms (concrete)

Add a `foveated` module (`crates/app/src/foveated.rs`) holding a struct that
owns all of this, mirroring how `hdr.rs` and `atmosphere.rs` own theirs.

**Textures**
- `height_faces`, `biome_faces`, `moisture_faces`: six-layer 2D arrays with a
  one-texel gutter, built at load. Give the height data a max-height mip chain
  or companion array for section 8.5.
- `warp_color`: `Rgba16Float`, size `warp_dims`, calculated from the active
  internal render size rather than hardcoded.
- `warp_dist`: `R32Float`, same size. Stores ray hit distance, or a sentinel
  (e.g. `-1.0`) for sky.

**Pipelines**
- `raymarch_pipeline`: fullscreen triangle vertex + `raymarch.wgsl` fragment.
  Two colour targets (`warp_color`, `warp_dist`). No depth. No blend.
- `unwarp_pipeline`: fullscreen triangle + `unwarp.wgsl` fragment. One colour
  target (`SCENE_FORMAT`), depth-write on into `Depth32Float`, depth-compare
  `Always` (you are authoring depth via `frag_depth`, not testing it).

**Uniform additions.** Add a small second uniform buffer for the ray path so you
do not disturb `CameraUniform`. Something like:

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct FoveationUniform {
    fovea_px:        [f32; 2],  // fovea center in screen pixels
    screen_dims:     [f32; 2],
    warp_dims:       [f32; 2],
    strength_l:      f32,       // foveation strength
    core_radius:     f32,       // linear (unwarped) core, in warped units
    // precomputed for precision (section 9)
    cam_radius:      f32,       // |C| = PLANET_RADIUS + altitude
    cam_radius_sq:   f32,       // |C|^2, computed in f64 on CPU
    r_max_shell:     f32,       // PLANET_RADIUS + max scaled height
    r_min_shell:     f32,       // PLANET_RADIUS + min scaled height (sea floor)
}
```

Do not duplicate height-scale constants in this uniform. The current source of
truth is `TerrainSettings`: near terrain is 1x, far terrain is 4x, blended from
100km to 1,000km. The ray shader calls the shared
`terrain_macro_height_scale()`, and the CPU computes tight `r_min_shell` /
`r_max_shell` values from the same settings and current camera altitude.

Put `warp_color`, `warp_dist`, the three field arrays, their samplers, the
existing `camera_bind_group`, and this `FoveationUniform` into bind groups the
two new pipelines share where sensible.

---

## 5. The toggle (do this first, it is the smallest real change)

Add the render-path switch before any shader exists, so you can prove the branch
and the toggle without risk.

**State.** In the app struct in `main.rs`:

```rust
#[derive(Copy, Clone, PartialEq)]
pub enum RenderPath { Raster, FoveatedRay }
// field:
render_path: RenderPath,   // init to RenderPath::Raster
```

**Toggle method**, next to `toggle_blur` and friends:

```rust
fn toggle_render_path(&mut self) {
    self.render_path = match self.render_path {
        RenderPath::Raster => RenderPath::FoveatedRay,
        RenderPath::FoveatedRay => RenderPath::Raster,
    };
    self.mark_hud_dirty();
}
```

**Keybind**, a new match arm copied from the F9 arm, using F5:

```rust
WindowEvent::KeyboardInput { event, .. }
    if event.state.is_pressed()
        && event.physical_key == PhysicalKey::Code(KeyCode::F5) =>
{
    state.toggle_render_path();
    window.request_redraw();
}
```

**Render branch.** In `render()`, wrap the cube-sphere pass:

```rust
match self.render_path {
    RenderPath::Raster => {
        // existing atmosphere.draw + terrain.draw block, unchanged
    }
    RenderPath::FoveatedRay => {
        self.foveated.render(&mut encoder, &self.camera_bind_group,
                             self.hdr.scene_view(), &self.depth_view);
    }
}
```

**Overlay.** Extend the HUD string near the existing `render_debug_mode.label()`
line to add `Render path: raster | foveated ray (F5)` and, later, foveation
stats. Add F5 to the keybind help line.

**Checkpoint M0.** For now, make `foveated.render` just clear `scene_view` to
your sky colour and clear depth. Toggling F5 should give you a blank sky, and
toggling back should be visually identical to today. If raster mode changed at
all, you touched something you should not have.

---

## 6. Milestones

Each milestone below is a stopping point. Build to the checkpoint, then continue.

### Protected prerequisite: shared shader refactor

Before M1, execute all three stages in `shared_shader_refactor_plan.md`. Capture
the raster baseline on the target Quadro before editing, validate the assembled
WGSL module in tests, and require the raster golden comparison to pass after
every stage. Do not add ray consumers during this prerequisite.

### M1. Whole-planet field source

**Goal:** six-face arrays you can sample for any direction.

- Build `height_faces` at load. Simplest correct version: for each face and
  texel, convert to a planet direction (you already have `face_uv_to_direction`
  in `coretypes`), find the covering tile at a fixed level (4 or 5), sample its
  height, write it. You can do this on the CPU into a staging buffer and upload,
  or in a small compute pass that samples the resident tiles. CPU-at-load is
  fine, it happens once.
- Do the same for biome and moisture, including a one-texel cross-face gutter.
- Add a temporary debug view in `raymarch.wgsl` that just outputs
  `height_faces` sampled by the primary ray direction as greyscale. No marching
  yet, just sample the face array in the direction each screen ray points and
  show it.

**Checkpoint:** toggling F5 shows a greyscale planet-height panorama that lines
up with where the real continents are. If it is smeared or rotated, your
face-to-direction mapping is off, fix it here where it is easy to see.

**Pitfalls:** cube face seams and orientation. Use the WGSL port of
`direction_to_face_uv`, and validate every face plus each gutter edge in the
debug view. Do not hand-roll equirectangular; the poles will bite you.

### M2. Basic raymarch, full resolution, flat shading

**Goal:** a recognisable planet from the ray path, correctness over speed.

- Render at full screen resolution for now (skip the warp entirely). The
  raymarch fragment shader runs per screen pixel.
- Per pixel: build the world ray from `camera_forward/right/up` and the pixel's
  NDC (same basis the raster camera uses).
- Intersect the `r_max_shell` sphere (section 8.2). Miss means sky, output the
  atmosphere colour along that ray (you can stub this as a flat gradient for
  now).
- March between shell entry and exit (section 8.3), fixed step count (start at
  192), find the sign change of `r(t) - surface_radius(dir)`, refine with a few
  secant iterations.
- Shade the hit flat: albedo by the biome face array, times a simple
  `dot(normal, sun)`. Normal comes from the height-field gradient (section 8.4).
- Write colour to `scene_view`, and write hit distance to a full-res
  `warp_dist`-equivalent (for now).

**Checkpoint:** from orbit you see a planet. From low altitude you see terrain,
probably slow and steppy. That is fine. Compare silhouettes against raster mode
by toggling F5 back and forth at the same camera pose.

**Pitfalls:** if the planet is inside-out or the horizon is wrong, check your
ray basis handedness against the raster projection. If terrain punches through
in stripes, your step count is too low for the range, that is expected until M5.

### M3. Real lighting, atmosphere, ocean

**Implemented on `experiment/ground-readability` (2026-07-22).** The full-res
ray path now uses the shared terrain materials, direct-sun transmittance, sky
diffuse irradiance, aerial perspective, ocean waves/lighting, F9 composition
modes, and the 16-sample atmospheric sky. A cheap outer sea-shell test and
height coverage check gate wave/ocean work on land; sea, lake, and coast hits
write the same reversed-Z depth convention as raster. Quadro M1000M captures
at the fixed global L4 field measured about 58 FPS at orbit and 49 FPS at
1.7 km (spatial-log means, presentation-limited and not GPU timestamps).
Low-altitude field resolution remains intentionally coarse until later
milestones. Existing raster LOD scenario assertions report zero chunks while
F5 ray mode is active; captures and finite-metric checks still complete.

**Goal:** ray mode looks close to raster mode from orbit.

- Call the already extracted `shared_planet.wgsl` functions from
  `raymarch.wgsl`: `terrain_material_color`, sun transmittance,
  `sky_diffuse_irradiance`, aerial perspective, ocean surface and lighting.
- Ocean: intersect a sea-level sphere (`PLANET_RADIUS + wave range`). If the
  ocean hit is nearer than the terrain hit, or terrain missed inside the ocean
  shell, shade ocean. Blend at the coast using the two hit distances.
- Sky: run the atmosphere scatter along the ray for the sky and as aerial
  perspective on the surface, same as raster.

**Checkpoint:** side-by-side at orbit, ray and raster should be close in colour
and tone. They will not be pixel-identical, and that is fine, exposure ties them
together anyway.

**Pitfalls:** the coastal double-cost from the raster analysis is worse here if
you are not careful, because every ray can touch both terrain and ocean. Keep
the ocean branch behind the shell test so open land never runs it. You will
foveate this further in the experiments.

### M4. Precision hardening

**Completed on `experiment/ground-readability` (2026-07-22).** Stage A was
already present in the M2 implementation: the CPU computes camera radius and
its square in f64 before upload, and WGSL evaluates radius with the
quadratic-in-t form rather than `length(C + tD)`. Regression coverage now
emulates the shader over 0.1 m to 10 km camera altitudes and radial-to-grazing
rays, bounding surface-radius error to 0.5 m; it also prevents the radius from
being cast to f32 before squaring. A Quadro descent captured every milestone
through 10 m altitude with finite metrics and no visible depth bands. Stage B's
hi/lo split was deliberately not added because its documented trigger was not
observed. The fixed L4 field remains visually coarse and is unrelated to
camera-position precision.

**Goal:** kill low-altitude wobble.

- Switch the radius test to the quadratic-in-`t` form using `cam_radius` and
  `cam_radius_sq` from the uniform (section 9).
- If banding remains at very low altitude, add the hi/lo split of the camera
  position and use it for `C . D`.

**Checkpoint:** fly down to a few km. The surface should be stable, no shimmering
depth bands as the camera moves slightly.

### M5. Adaptive stepping

**Completed on `experiment/ground-readability` (2026-07-22).** Startup now
builds and uploads a conservative odd-dimension max-height mip pyramid for all
six field faces. Hit rays retain M2's large first baseline interval. After a
miss, candidate intervals grow exponentially to 64x; one maxmip texel covering
four times the candidate footprint supplies a conservative local shell, and
the next analytic sphere crossing bounds the skip. Cube-face or mip-cell
boundary cases fall back to the global height maximum and baseline interval.
This final design followed rejected 9-tap and 4-tap radial-clearance versions
that regressed the target GPU. On the Quadro orbit scenario, spatial-log mean
frame time fell from 16.753 ms to 14.952 ms (10.8%, about 59.7 to 66.9 FPS).
The 1.7 km case remained effectively unchanged at 20.220 ms because surface-hit
rays already terminate after the baseline step. All four orbit views and the
low-flight view retained their silhouettes with no visible holes; 113 app tests
pass. Adaptive stepping changes sample count only, not fixed-L4 field detail.

**Goal:** the same picture, far fewer steps.

- Build or reuse a max-height mip pyramid for `height_faces` (or a companion
  `max_height_faces` array). Use it to take big
  steps when the ray is well above the local max terrain, small steps only near
  the surface. This is empty-space skipping / cone stepping for a heightfield.
- Simpler intermediate step if the maxmip is fiddly: grow `dt` with distance and
  with height-above-terrain, clamp to a min. Gets you most of the win.

**Checkpoint:** step counts drop hard, silhouettes stay crisp. This is your big
pre-foveation speed win. Measure it. If M5 alone gets you to a comfortable frame
time at full res, foveation becomes pure headroom for effects.

### M6. Introduce the warp

**Goal:** center sharp, edges cheap.

- Render the raymarch into `warp_color` / `warp_dist` at runtime-derived
  `warp_dims` instead of full res. Start with a configurable fraction of the
  active internal render size. The raymarch fragment shader now maps its
  warped texel to a screen position (section 7), builds the ray from that screen
  position, and marches as before.
- Add the `unwarp` pass: fullscreen over the real screen, inverse-warp each pixel
  to a warped uv, sample `warp_color`, write to `scene_view`, and reconstruct
  reversed-Z depth from `warp_dist` (section 11) into `depth_view`.
- Start with the **separable** warp (section 7.1). No angular seam, no center
  singularity, one 1D curve per axis.

**Checkpoint:** the center of the screen is as sharp as M5 was, the edges are
visibly softer, and the frame time drops a lot because you shaded a fraction of
the rays. Toggle a debug key to visualise `warp_color` directly so you can see
the warp shape.

**Pitfalls:** bilinear across the warp smears near the fovea, keep a small linear
`core_radius`. Make sure the unwarp writes depth or the sun overlay will float in
front of terrain.

### M7. Fovea follows where you are going

**Goal:** the sharp spot tracks travel, not screen center.

- Compute the focus of expansion: project the camera velocity direction onto the
  screen (section 10). Feed it into `fovea_px`. Ease it back to screen center
  when speed is low or in orbit mode.

**Checkpoint:** accelerate along a heading and the sharp region sits where you
are flying toward. Stop and it drifts back to center.

### M8. Experiments

The fun part, section 12. Each is a sub-toggle so you can A/B them:

- Horizon-aligned sample density.
- Temporal reprojection for the periphery.
- Content-adaptive ray budget from the maxmip.
- Foveated shading (waves and shadows only in the fovea).
- Radial blur as intentional style.

### M9. Polish

- Overlay stats: warp dims, foveation strength, fovea position, approximate rays
  per frame, ray-path frame time.
- A capture path parity check so F12 still works in ray mode.
- A safe fallback: if the field arrays are not built yet, fall back to raster
  for that frame rather than showing a hole.
- If you use scenarios (`scenario.rs`) for automated runs, add a ray-path flag so
  a scenario can assert on the ray path too.

---

## 7. The warp math

Both versions map a warped-buffer coordinate to a screen position (used in the
raymarch pass) and its inverse (used in the unwarp pass). `p > 1` and `L > 0`
control how hard the center is favoured.

### 7.1 Separable (start here)

Work per axis. Let `c` be a warped coordinate in `[-1, 1]` measured from the
fovea, and `s` the screen coordinate in `[-1, 1]` from the fovea to that axis
edge. Forward (warped to screen), used to place each ray:

```wgsl
fn warp_axis(c: f32, p: f32) -> f32 {
    return sign(c) * pow(abs(c), p);   // p > 1 => dense near center
}
```

Inverse (screen to warped), used in unwarp:

```wgsl
fn unwarp_axis(s: f32, p: f32) -> f32 {
    return sign(s) * pow(abs(s), 1.0 / p);
}
```

Because the derivative `d(screen)/d(warped) = p * |c|^(p-1)` is small near the
center and large at the edges, warped texels land close together in the middle
and far apart at the edges. That is exactly dense-center sampling.

Handle an off-center fovea by normalising each side separately: for a screen
pixel, `s_x = (px - fovea_px.x) / half_extent_x`, where `half_extent_x` is the
distance from the fovea to the left or right edge depending on sign. Do the same
for y. The warped buffer stores each side in half of its axis.

Add the linear core so the very center does not over-compress: blend `warp_axis`
with the identity for `|c| < core_radius`.

The foveation region comes out diamond-shaped. On a flat monitor that reads
fine. Ship it, then decide if you want circular.

### 7.2 Log-polar (the circular upgrade)

Warped coords `(a, b)` in `[0,1]^2`, `a` is angle, `b` is log radius. `R` is the
distance from the fovea to the far screen corner.

Forward, warped to screen offset from fovea:

```wgsl
let theta = 6.2831853 * a;
let r     = R * (exp(b * L) - 1.0) / (exp(L) - 1.0);
let screen_offset = vec2<f32>(r * cos(theta), r * sin(theta));
```

Inverse, screen offset to warped:

```wgsl
let d     = pixel - fovea_px;
let r     = length(d);
let theta = atan2(d.y, d.x);
let a     = fract(theta / 6.2831853 + 1.0);   // wrap
let b     = log(1.0 + r * (exp(L) - 1.0) / R) / L;
```

`dr/db` is small at `b=0` and large at `b=1`, so again dense center. Two
gotchas: sample the angular axis with wrap (address mode repeat on `a`), and keep
a linear core near `b=0` so the singularity does not swim. This is the version
from the visual-polar path-tracing papers, worth doing once the separable
version has taught you the pipeline.

---

## 8. The raymarch, in detail

### 8.1 Building the ray

For warped texel, map to screen NDC via the forward warp and the fovea, then:

```wgsl
let ndc = /* screen position in [-1,1], from warp */;
let dir = normalize(
    camera.camera_forward.xyz
  + ndc.x * tan_half_fov * aspect * camera.camera_right.xyz
  + ndc.y * tan_half_fov          * camera.camera_up.xyz
);
```

Match `tan_half_fov` and aspect to whatever the raster projection uses so the two
paths line up when you A/B them.

### 8.2 Shell intersection

Origin is the camera. The planet center is at the world origin. With
`b = dot(C, dir)` and radius `R`:

```
t = -b +/- sqrt(b^2 - (|C|^2 - R^2))
```

Compute `|C|^2 - R^2` carefully (section 9). Intersect the `r_max_shell` to find
the march interval `[t_enter, t_exit]`. If the discriminant is negative and the
camera is outside, the ray is pure sky. If the camera is inside the shell (low
altitude), `t_enter` clamps to 0.

### 8.3 The march loop

```wgsl
var t = max(t_enter, 0.0);
var prev_f = radius_at(t) - surface_radius(dir_at(t));   // > 0 above ground
loop {
    t += step_size(t);              // fixed at M2, adaptive at M5
    if (t > t_exit) { /* sky */ break; }
    let f = radius_at(t) - surface_radius(dir_at(t));
    if (f < 0.0) {                  // crossed the surface
        t = refine(t - step_size, t);   // secant, 4-6 iters
        /* shade hit */ break;
    }
    prev_f = f;
}
```

`radius_at(t)` uses the quadratic form. `dir_at(t) = normalize(C + t*dir)`,
which is fine in f32 (direction tolerates the precision loss, the radius does
not). `surface_radius(dir) = PLANET_RADIUS + terrain_macro_height_scale() *
sampleHeight(dir)`. This is the same shared `TerrainSettings`-driven scale used
by raster terrain; do not copy the current 1x/4x constants into ray code.

### 8.4 Normal from the face array

Sample height at `dir` and at two small tangent offsets on the sphere (east and
north), difference to get slopes, cross for the normal:

```wgsl
let e = tangent_east(dir);
let n = tangent_north(dir);
let h  = sampleHeight(dir);
let he = sampleHeight(normalize(dir + e * eps));
let hn = sampleHeight(normalize(dir + n * eps));
let height_scale = terrain_macro_height_scale();
let normal = normalize(cross(
    e * eps * PLANET_RADIUS + dir * (he - h) * height_scale,
    n * eps * PLANET_RADIUS + dir * (hn - h) * height_scale
));
```

Scale `eps` with the mip level you sampled so the normal roughness matches the
detail you actually resolved. Coarse peripheral rays should get coarse normals,
which also hides their undersampling.

### 8.5 Adaptive stepping with the maxmip

Keep a max-height mip pyramid. At the current point, look up the local max
terrain radius at a mip chosen by how far the ray has travelled (coarser mip
when far). If the ray's current radius is well above that local max, jump most of
the way to it. Only drop to fine steps when you are within a margin of the
surface. This is the difference between 192 steps and maybe 20 to 40 over most of
the screen.

---

## 9. Precision, staged

**Stage A (do this first).** Pass `cam_radius = PLANET_RADIUS + altitude` and
`cam_radius_sq` (computed in f64 on the CPU) in the uniform. Then:

```wgsl
fn radius_at(t: f32) -> f32 {
    // |C|^2 + 2 t (C.D) + t^2, with C.D folded into b_dot passed per-frame
    return sqrt(cam_radius_sq + 2.0 * t * b_dot + t * t);
}
```

where `b_dot = dot(C, dir)` computed per ray. At low altitude this gives ~1 m
radial resolution. Good enough for a low-res planet.

**Stage B (only if you see banding at the deck).** Split the camera world
position into hi and lo f32 vectors on the CPU (Dekker split), and compute
`b_dot` as `dot(C_hi, dir) + dot(C_lo, dir)`. This recovers the bits the single
f32 dropped. Do not do this until the banding is actually visible, it is easy to
add later and it clutters the shader.

---

## 10. Fovea placement (focus of expansion)

You know the camera velocity from the flight controller. Project its direction
onto the screen:

```
v_dir = normalize(camera_velocity)                     // world space
s = dot(v_dir, forward)
if s > small:                                          // moving roughly forward
    screen = fovea from (dot(v_dir,right)/s, dot(v_dir,up)/s) mapped to pixels
else:
    screen = screen center
fovea_px = lerp(fovea_px, clamp(screen, margins), follow_rate * dt)
```

The lerp keeps it from snapping. Clamp inside a margin so the fovea never sits
right on the edge, which would waste half the warp. In orbit or auto-orbit mode,
just pin it to center. This is the cheap stand-in for eye tracking, and for a
flight sim it is a better gaze guess than assuming dead center.

---

## 11. Depth output and the sun overlay

The sun overlay pass depth-tests against `depth_view`. If the ray path does not
write depth, the sun disc will not be occluded by terrain. So the unwarp pass
reconstructs depth from `warp_dist`.

For a screen pixel, sample `warp_dist` at its warped uv to get hit distance `t`.
Reconstruct the world hit point `P = C + t*dir`, transform by
`camera.projection_matrix` (which already bakes the reversed-Z convention), and
write `clip.z / clip.w` to `@builtin(frag_depth)`:

```wgsl
let P_view = to_view(P);                 // using camera basis
let clip   = camera.projection_matrix * vec4<f32>(P_view, 1.0);
return FragOut(color, clip.z / clip.w);  // frag_depth
```

wgpu depth is already in `[0, 1]`; do not apply the OpenGL
`clip_z * 0.5 + 0.5` remap. The unwarp pass must use the same projection matrix
and camera-forward convention as raster or the visual sun's depth test will
disagree.

Sky pixels (sentinel distance) write the cleared far value (`0.0` under
reversed-Z) so the sun composits correctly against sky. Set the unwarp pipeline's
depth compare to `Always` and depth-write on, since you are authoring depth, not
testing it.

---

## 12. Experiments (going past the VR playbook)

These are where this stops being a port and starts being yours. Each is a
sub-toggle so you can measure it.

### 12.1 Horizon-aligned density

VR foveation assumes a uniform world, so cutting edge samples is free. On a
planet the screen edges are often the horizon, where a lot of world is crushed
into few pixels at a grazing angle. That is the worst place to starve. You know
the horizon analytically from altitude and planet radius, so compute where the
limb projects on screen each frame and bias the warp to keep a ridge of density
along it. This is a foveation the headset papers cannot do, because they do not
know their content geometry the way you do. Try it as a modification to the warp
radius that pulls samples toward the horizon curve.

### 12.2 Temporal reprojection for the periphery

Peripheral rays look at far features with tiny parallax, so they barely change
frame to frame. Keep last frame's `warp_color`, reproject it by the camera delta,
and only re-shoot rays where reprojection fails (disocclusion, big colour delta).
Fresh rays in the fovea every frame, recycled rays at the edges, spend the saved
budget on the center. Far equals slow equals reprojection-friendly is a synergy
that basically only exists for planets and space scenes. Watch for peripheral
smear during fast turns, that is your failure mode, gate reprojection on angular
velocity.

### 12.3 Content-adaptive ray budget

The maxmip from section 8.5 already tells you roughly how rough each region is.
Flat ocean and empty sky need almost no rays, jagged ridges need many. Modulate
sample density by content on top of eccentricity. This stacks with the horizon
idea. It is very planet-specific and it is close to free once the maxmip exists.

### 12.4 Foveated shading, not just foveated pixels

Cost in a raymarcher is also steps and secondary effects. Let peripheral rays
take coarser steps and skip the expensive extras. Ocean Gerstner waves only in
the fovea, flat tinted water at the edges. Shadows and any secondary rays only in
the fovea. This multiplies with the ray-count savings instead of adding to them,
and it directly attacks the coastal double-cost you found in the raster
analysis.

### 12.5 Radial blur as style

The log warp already softens the edges whether you want it or not. Lean in. Tune
it to read as depth of field or speed blur, and the undersampling hides inside
something that looks intentional and a bit cinematic. A cheap extra radial blur
in the unwarp pass, strength rising with eccentricity, can turn the artifact into
the aesthetic.

---

## 13. Performance budget for the M1000M

Rough back-of-envelope so you know what to aim for.

- Full-res raster low-altitude today: single digit to low teens FPS, fill-bound.
- Full-res M2 is a correctness milestone. At 1280x720 and 192 fixed steps it can
  issue roughly 177 million height evaluations before refinement, normals, or
  shading. Portable manual bilinear height sampling makes each evaluation four
  texture loads, so this version may run at only a handful of FPS.
- M5 adaptive stepping must produce a measured reduction in raymarch GPU time
  and average steps per ray before M6 begins. No-overdraw does not make a long
  march automatically cheap.
- The warp's savings depend on the active internal size and selected scale. A
  480x270 buffer is about a 2.1x reduction from 640x427, 7.1x from 1280x720, and
  16x from 1920x1080. Record the actual dimensions in every profile.
- The unwarp pass is one cheap fullscreen sample-and-write, negligible.

The target remains vsync (59) at low altitude with the fovea sharp. If adaptive
stepping already meets budget at the active size, test whether ray mode can
instead render a sharper center at a higher internal resolution while the warp
pays for its periphery.

Measure with the existing GPU profiler path (the `flush_gpu_profile` timestamp
machinery) rather than eyeballing the FPS counter. Put a timestamp around the
raymarch pass and the unwarp pass separately so you know which one to chase.

---

## 14. Testing and validation

- **A/B toggle.** The single most useful test is flipping F5 at a fixed pose.
  Colour and silhouette should track between paths. Divergence tells you which
  shared function you adapted wrong.
- **Warp visualiser.** A debug key that blits `warp_color` straight to screen so
  you can see the sample distribution and catch seams or a mis-placed fovea.
- **Shimmer check.** Pan and translate slowly at low altitude and watch the
  periphery. Crawling means the peripheral undersampling needs the temporal or
  content-adaptive help from section 12.
- **Precision check.** Descend to the deck and nudge the camera by centimetres.
  Depth bands mean go to precision stage B.
- **Depth parity.** At one fixed pose, render raster and ray depth as grayscale
  and compare them. Confirm numerically that near values approach `1.0`, far/sky
  values approach `0.0`, and do not apply an OpenGL `* 0.5 + 0.5` remap.
- **Sun occlusion.** Put the visual sun behind a terrain ridge and assert that
  the ray-written depth clips it exactly as the raster depth does.
- **Scenario runs.** If you wire the ray path into `scenario.rs`, add assertions
  on ray-path frame time so a regression shows up in an automated run rather than
  by feel.

### Render-mode parity matrix

- **F9 debug modes:** ray mode returns the same Final, RawAlbedo,
  SurfaceLighting, and AerialContribution quantities. Render debug views at full
  resolution without the warp so inspection is not contaminated by upscaling.
- **SkyOnly:** skip terrain and ocean marching, shade atmosphere only, and still
  run the final full-resolution output/depth pass.
- **F12 capture:** keep using the existing final-image capture path and add a
  ray-mode scenario assertion so parity is tested rather than assumed.
- **Resize/fullscreen:** call `foveated.resize()` from
  `resize_render_targets()` so `warp_color`, `warp_dist`, and size-dependent
  bindings rebuild from `self.size` exactly when HDR and depth rebuild.
- **Exposure/HDR/bloom:** remain downstream of the ray output. Luminance meters
  the post-unwarp HDR scene before the visual sun overlay, exactly as raster does.

---

## 15. Rollout and risk

- The whole thing is behind F5 and defaults off, so it cannot regress the raster
  experience. Merge it early and often.
- The one shared-code risk is factoring functions out of `planet.wgsl`. Do that
  refactor as its own change, verify raster is byte-identical after it, then
  build the ray path on top. Do not refactor and add the new path in one go.
- The six-face arrays flatten the sparse landing detail. That is a known,
  accepted limitation of version one. If and when it bothers you, the
  virtual-texture upgrade in section 3.1 is a contained follow-up, not a rewrite.

---

## 16. Build order at a glance

```
M0  toggle + branch, ray path clears sky            (no shaders)
R0  protected shared-shader refactor, raster only   (three golden-tested stages)
M1  height/biome/moisture face arrays + debug view  (data plumbing)
M2  full-res raymarch, flat shading                 (correctness)
M3  consume shared lighting/atmosphere/ocean        (looks right)
M4  precision stage A (+ B if needed)               (stable at the deck)
M5  adaptive stepping via maxmip                    (big speed win)
M6  separable warp + unwarp + depth                 (foveation on)
M7  fovea follows focus of expansion                (better gaze guess)
M8  experiments, each a sub-toggle                  (go past the papers)
M9  overlay stats, capture parity, fallbacks        (polish)
```

Do not begin M6 until M5 demonstrates a real GPU-time and step-count reduction.
Ship M0 through M6 and you have the intended feature: a sharp center, a cheap
periphery, and a raymarched planet with no raster overdraw. Everything after
that is you making it yours.

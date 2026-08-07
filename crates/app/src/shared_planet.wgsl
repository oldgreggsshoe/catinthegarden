const PLANET_RADIUS_METERS: f32 = 4000000.0;
// Material/height tiles are intentionally denser than the fixed 32x32 chunk
// grid, so material detail and coastline transitions do not inherit mesh size.
const MATERIAL_TILE_LOGICAL_QUADS: f32 = 128.0;
const NEAR_FIELD_WINDOW_LOGICAL_QUADS: f32 = 1024.0;
const TILE_GUTTER: f32 = 1.0;
const MATERIAL_TILE_LAST_STORED_COORD: i32 = 130;
const GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS: f32 = 111.5;
// Must track TERRAIN_DETAIL_* in planet.rs.
// Amplitude/wavelength for every detail octave, so the field is self-similar
// and every octave contributes the same characteristic slope.
//
// 0.10 starting from 1024m was tried early and is the cautionary case: the top
// octave alone is 102m and the ladder sums to ~118m RMS, the same order as the
// entire baked terrain, so it stops being detail and becomes a second planet
// laid over the baker's erosion and hydrology. **The start wavelength was the
// real culprit there, not the roughness.**
//
// 0.06 from 256m was then tried twice. The first attempt produced hard
// stair-stepped silhouettes and was reverted as "gated on mesh density" -- the
// wrong diagnosis. The density was available; the LOD selector's error budget
// was a flat constant that knew nothing about this ladder and so never asked
// for it. With the budget derived from ROUGHNESS (OUTMAP_GEOMETRIC_ERROR_RATIO
// in terrain.rs) the stepping goes: over the mountain view, p999 of the
// ground's luminance gradient falls 7.07 to 4.47 while p50 and p90 hold.
//
// At the current 4096m start the unboosted finite ladder has a 491.46m absolute
// amplitude ceiling before the per-octave land headroom gate. The former 8x
// long-wave boost is disabled below so observed ETOPO shapes dominate.
const TERRAIN_DETAIL_ROUGHNESS: f32 = 0.06;
// Floor is 1m. The octave ladder is evaluated from an anchor-local offset, so
// the in-cell fraction never has to survive an absolute 4e6 domain coordinate
// where f32 would quantise it to 0.25 -- see terrain_detail_value_noise.
// Starts at the scale the *coarsest* baked pyramid runs out at, not at the
// scale the finest one does. Away from the sparse corridor the baked data is
// L4, whose texels are 3.9km, so it carries nothing below about 7.8km -- and
// the ladder used to start at 256m. That left over three octaves with nothing
// in them at all, which is why a mountain read as a smooth ramp with fine
// texture on it and no hills in between.
//
// The amplitude law needed no change to cover them. Measured off the baker's
// own eroded corridor, RMS height difference is 0.080 of the separation at
// every scale from 12m to 767m -- self-affine, and within a factor of the
// ROUGHNESS the ladder already uses.
const TERRAIN_DETAIL_START_WAVELENGTH_METERS: f32 = 4096.0;

// The former 8x spectral tilt made the longest procedural octaves dominate the
// observed ETOPO terrain as a repeating pattern of large random basins and
// ridges. Leave the function and finer ladder intact, but disable that boost
// while the macro terrain is evaluated. Directional shaping can be added back
// later as a separate, attributable change.
const TERRAIN_DETAIL_LONG_GAIN: f32 = 1.0;
const TERRAIN_DETAIL_TILT_TAPER_METERS: f32 = 256.0;

fn terrain_detail_octave_tilt(wavelength_meters: f32) -> f32 {
    return 1.0
        + (TERRAIN_DETAIL_LONG_GAIN - 1.0)
            * smoothstep(
                TERRAIN_DETAIL_TILT_TAPER_METERS,
                TERRAIN_DETAIL_START_WAVELENGTH_METERS,
                wavelength_meters,
            );
}

const TERRAIN_DETAIL_OCTAVES: i32 = 13;
// What the whole ladder can reach: amplitude halves with wavelength, so the
// geometric series sums to twice its first term. Anything normalising against
// the detail field has to use this and not the retired CPU field's 111.5m,
// which is an order of magnitude larger and silently scales the result away.
// With the long-wave boost disabled this is the ordinary finite halving sum,
// 4096 * 0.06 * (1 + 1/2 + ... + 1/4096) = 491.46m, rounded upward. The ray
// path and culling shell use it as a conservative bound.
const TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS: f32 = 491.5;
// Erosion-like structure. Two knobs, both mirrored in planet.rs.
//
// The fold: `|n|` creases the field at every zero crossing of the noise, and
// creases are what eroded ground is made of -- a plain fBm sum only ever makes
// rounded blobs, whatever its amplitude. The value stays continuous across the
// crease and only the slope flips, which is exactly what a ridgeline is.
//
// CENTRE and SCALE come from the *softened* fold's measured distribution
// (mean = 0.348609, sd(n) / sd(fold) = 2.063534, over 216000 samples; the
// test `the_ridge_fold_is_centred_against_the_noise_it_folds` re-derives them).
// They matter: folding naively leaves a mean of ~0.6 per octave, which would
// lift all land by ~10m and, worse, lift it by a *varying* amount wherever the
// land weight ramps, inventing slopes along every coastline. Centred and
// rescaled this way, the fold changes character while leaving mean and RMS
// exactly where the amplitude discipline above put them.
// The fold uses a softened absolute value, sqrt(n*n + softness*softness),
// rather than abs(n). A hard fold creases every octave at its zero crossing
// with a slope discontinuity, and stacked across nine octaves that reads as
// contour terracing -- flat shelves bounded by hard edges -- rather than as
// landform. Softening rounds each crease over a fixed width and, as a bonus,
// makes the octave's analytic gradient continuous, which the shading normals
// were previously getting a sign flip from.
const TERRAIN_DETAIL_RIDGE_SOFTNESS: f32 = 0.15;
const TERRAIN_DETAIL_RIDGE_CENTRE: f32 = 0.348609;
const TERRAIN_DETAIL_RIDGE_SCALE: f32 = 2.063534;
// Fully folded. At 0.7 the fold was mixed back with the smooth noise it folds,
// which rounds every crease off; a mountain's defining feature is that its
// ridgelines are *not* rounded. Measured at the mountains this is what turns
// relief into steepness: at gain 8 it takes ground past 25 degrees from 8.6%
// to 16.6%, and the aretes are the visible part.
const TERRAIN_DETAIL_RIDGE_STRENGTH: f32 = 1.0;
// Blending two uncorrelated fields of equal variance shrinks the result to
// sqrt((1-s)^2 + s^2) of it -- 76% at s = 0.7. Undo that, or the ladder
// quietly loses a quarter of its relief the moment the fold is switched on and
// the loss silently tracks the strength knob.
//
// This is *derived from* the strength above and is not free to set: at s = 1.0
// there is no blend left to undo and the factor is exactly 1.0. Leaving 1.313
// here while raising the strength would add 31% of unaccounted amplitude to
// every octave. `the_ridge_normalisation_follows_the_strength_it_undoes`
// re-derives it.
const TERRAIN_DETAIL_RIDGE_NORMALISATION: f32 = 1.0;
// Multifractal attenuation: the slope at which the next octave is halved.
// Fine relief is suppressed where the accumulated surface is already steep and
// left to run where it is flat, which is what separates a smooth valley wall
// and a flat plain from uniform crumple.
//
// At 0.25 this was halving every octave on any ground past about 14 degrees,
// which is precisely the ground a mountain is made of -- the term was rounding
// off the crags it was meant to leave alone. Measured on its own it is a weak
// knob (0.25 -> 8.0 moves relief 313m -> 335m), but under a raised ladder it
// is the difference between the long octaves carrying the fine ones and
// smothering them. 4.0 keeps the smooth-valley behaviour for genuinely gentle
// ground while letting a face stay a face.
const TERRAIN_DETAIL_ATTENUATION_SLOPE: f32 = 4.0;
const TERRAIN_SKIRT_DEPTH_RATIO: f32 = 0.075;
const MAX_TERRAIN_SKIRT_DEPTH_METERS: f32 = 10.0;
const ATMOSPHERE_HEIGHT_METERS: f32 = 720000.0;
const ATMOSPHERE_EDGE_FADE_METERS: f32 = 480000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 36000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 4800.0;
const RAYLEIGH_COEFFICIENT: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFFICIENT: vec3<f32> = vec3<f32>(0.5e-6);
const MIE_G: f32 = 0.76;
const SOLAR_RADIANCE: f32 = 2.0;
// Artistic surface exposure only: this does not alter sky scattering or the
// camera-facing sun disc.
const SURFACE_SUNLIGHT_SCALE: f32 = 2.0;
// Local overhead sky fill is intentionally stronger than the former 0.18
// artistic scale so terrain remains readable while the sun is low/visible.
const SKY_DIFFUSE_LIGHT_SCALE: f32 = 0.70;
const TWILIGHT_RED_RADIANCE: vec3<f32> = vec3<f32>(0.30, 0.012, 0.001);
const AERIAL_IN_SCATTER_SAMPLE_COUNT: u32 = 2u;
const AERIAL_DENSITY_SAMPLE_EXPONENT: f32 = 3.0;
// Artistic aerial-only control, applied after physically bounded integration.
// It does not alter extinction, direct terrain/ocean lighting, or the sky pass.
const AERIAL_IN_SCATTER_GAIN: f32 = 3.0;
// Keep the intentionally strong global aerial effect from washing the ocean
// body colour to grey in the final composition. Terrain and sky stay unchanged.
const OCEAN_AERIAL_PERSPECTIVE_WEIGHT: f32 = 0.35;
const OCEAN_REFLECTION_SCALE: f32 = 0.35;
const OCEAN_SUN_GLINT_SCALE: f32 = 3.0;
const TWILIGHT_SHADOW_TRANSITION_METERS: f32 = 36000.0;
const TERRAIN_FOG_START_METERS: f32 = 2000.0;
const TERRAIN_FOG_END_METERS: f32 = 60000.0;
const TERRAIN_FOG_MAX_CAMERA_ALTITUDE_METERS: f32 = 100000.0;
const TERRAIN_FOG_FULL_HORIZON_COSINE: f32 = 0.05;
const TERRAIN_FOG_CLEAR_HORIZON_COSINE: f32 = 0.35;
const TERRAIN_MATERIAL_TILE_METERS: f32 = 2048.0;
// Close-range material repeat. The 2km tile above covers a whole landscape, so
// standing on the ground it is one flat colour; this is the tile that actually
// reads as ground texture underfoot. It cannot be formed from an absolute
// planet coordinate -- 4e6/8 needs 5e5 tiles, where f32 quantises the lookup to
// whole texels -- so it is built anchor-locally, the same split the detail noise
// uses. See terrain_material_fine_position.
// 8m read as wallpaper: the layer textures carry strong 64-cell content, which
// at that scale becomes a 2m motif repeating on a plainly visible lattice. Kept
// small enough that the coarsest thing it can repeat is sub-metre grain, and
// mixed in as a brightness ratio rather than as colour, so what tiles is the
// texture's contrast and not its hue.
const TERRAIN_MATERIAL_DETAIL_TILE_METERS: f32 = 6.0;
// The lookup is warped by a noise this long before it is tiled, so the repeat
// no longer lands on a regular lattice -- which is what actually gives tiling
// away. One noise evaluation is much cheaper than the second set of triplanar
// fetches an incommensurate second scale would need, and its gradient is a
// smooth 3D offset that comes free with the value.
const TERRAIN_MATERIAL_DETAIL_WARP_WAVELENGTH_METERS: f32 = 37.0;
const TERRAIN_MATERIAL_DETAIL_WARP_TILES: f32 = 0.85;
// How far the close-range grain may push the albedo either side of the colour
// the biome and the 2km tile already agreed on.
const TERRAIN_MATERIAL_DETAIL_STRENGTH: f32 = 0.55;
// How much of the layer-blend height comes from the close-range tile rather
// than the 2km one. This is what varies the material boundaries themselves at
// metre scale instead of only shading a single material.
const TERRAIN_MATERIAL_DETAIL_HEIGHT_SHARE: f32 = 0.7;
// How far relief may shift the vegetation/bare-ground split either way.
const TERRAIN_MATERIAL_RELIEF_VEGETATION: f32 = 0.34;
// Where the fine tile hands back to the 2km one. Past the far end an 8m repeat
// is below a pixel and mips to its own average, so blending it out costs
// nothing visually and saves the second set of triplanar fetches.
const TERRAIN_MATERIAL_DETAIL_NEAR_METERS: f32 = 150.0;
const TERRAIN_MATERIAL_DETAIL_FAR_METERS: f32 = 900.0;
// The probe spacing normals are central-differenced over. This is the sharpest
// relief the surface can ever show: an 8m floor discarded everything finer than
// ~16m, which flattened both the 0.375m baked tiles and any synthesised detail.
const TERRAIN_NORMAL_MIN_SAMPLE_METERS: f32 = 0.5;
const TERRAIN_NORMAL_MAX_SAMPLE_METERS: f32 = 256.0;
// Normal probes are camera_distance * this, and detail octaves are filtered to
// the same spacing so displacement and shading never disagree about an octave.
const TERRAIN_DETAIL_FILTER_RATIO: f32 = 0.01;
const TERRAIN_DETAIL_MIN_FILTER_METERS: f32 = TERRAIN_NORMAL_MIN_SAMPLE_METERS;
// Must track CHUNK_GRID_QUADS in planet.rs.
const TERRAIN_CHUNK_QUADS: f32 = 32.0;
// How much sub-mesh relief darkens and lightens the albedo, on top of the
// shading it already drives. Surface texture, not shadowing, so keep it modest.
const TERRAIN_DETAIL_ALBEDO_STRENGTH: f32 = 0.18;
const TERRAIN_MATERIAL_VEGETATION: i32 = 0;
const TERRAIN_MATERIAL_EARTH: i32 = 1;
const TERRAIN_MATERIAL_ROCK: i32 = 2;
const TERRAIN_MATERIAL_SNOW: i32 = 3;
const RENDER_DEBUG_FINAL: u32 = 0u;
const RENDER_DEBUG_RAW_ALBEDO: u32 = 1u;
const RENDER_DEBUG_SURFACE_LIGHTING: u32 = 2u;
const RENDER_DEBUG_AERIAL_CONTRIBUTION: u32 = 3u;
const RENDER_DEBUG_FLAT_TRIANGLES: u32 = 6u;

struct Camera {
    projection_matrix: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_view_altitude: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_direction_view: vec4<f32>,
    projection: vec4<f32>,
    flat_triangle_options: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(2) @binding(3)
var environment_map: texture_cube<f32>;

@group(2) @binding(4)
var environment_sampler: sampler;

struct TerrainSettings {
    outmap_height_scale: vec4<f32>,
    outmap_height_blend: vec4<f32>,
    outmap_detail: vec4<f32>,
}

@group(2) @binding(5)
var<uniform> terrain_settings: TerrainSettings;

@group(2) @binding(6)
var terrain_material_map: texture_2d_array<f32>;

@group(2) @binding(7)
var terrain_material_sampler: sampler;

struct OceanWaveContribution {
    horizontal_displacement: vec3<f32>,
    vertical_displacement: f32,
    slope: vec3<f32>,
}

struct OceanSurface {
    horizontal_displacement: vec3<f32>,
    vertical_displacement: f32,
    normal: vec3<f32>,
}

// The current experimental presentation intentionally keeps every water
// surface level. Retain the Gerstner implementation below for diagnostics and
// a future visual branch, but do not displace raster or ray water here.
const OCEAN_WAVES_ENABLED: bool = false;

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

fn view_to_planet(vector: vec3<f32>) -> vec3<f32> {
    return camera.camera_right.xyz * vector.x
        + camera.camera_up.xyz * vector.y
        - camera.camera_forward.xyz * vector.z;
}

fn placeholder_octave(direction: vec3<f32>, frequency: f32, amplitude: f32) -> f32 {
    let wave = sin(frequency * direction.x) - direction.x * sin(frequency)
        + sin(1.375 * frequency * direction.y)
        + sin(1.75 * frequency * direction.z);
    return amplitude * wave / 4.0;
}

fn placeholder_height(direction: vec3<f32>) -> f32 {
    return placeholder_octave(direction, 8.0, 2800.0)
        + placeholder_octave(direction, 512.0, 600.0)
        + placeholder_octave(direction, 32768.0, 100.0)
        + placeholder_octave(direction, 2097152.0, 3.0);
}

// Fractal relief continuing below whatever the baked outmap can store. The
// outmap cannot carry this: 1m samples over a 4000km planet would be 800TB, so
// everything finer than the macro terrain has to be synthesised here.
//
// Amplitude is a fixed fraction of wavelength, so the field is self-similar and
// every octave contributes the same characteristic slope. That is what keeps
// orbit and ground looking like the same planet rather than two different ones.
//
// THE HASH MUST BE EXACT, not merely equivalent. This was `fract(sin(dot(cell,
// k)) * 43758)`, in f32 here and f64 in planet.rs. The sin argument reaches 1e9
// at the finest octave, where consecutive f32 values are 64 radians apart, so
// the two evaluations were unrelated numbers. Both produced correct-looking
// noise of the right amplitude, and the surface probe measured the correlation
// between them at 0.02: the ground the camera collided with was not the ground
// it could see. Anything folded by `fract` amplifies a last-bit difference into
// a completely different value, so "close enough" is not a category that exists
// here -- only integer arithmetic is specified identically on both sides.
//
// Two multiplies per corner rather than a long shift/add chain. Maxwell runs
// 32-bit integer multiply at quarter rate, which is why the multiply-free form
// was tried first -- but it needed so many full-rate operations to diffuse
// properly that it measured 2ms slower in the raster path and up to 13ms slower
// in the raymarch path, which put the ground-level scenarios over budget. One
// wide multiply diffuses further than a dozen shifts.
fn detail_mix(value: u32) -> u32 {
    var h = value * 0x9e3779b1u;
    h = h ^ (h >> 15u);
    return h;
}

fn detail_avalanche(value: u32) -> u32 {
    var h = value * 0x85ebca6bu;
    h = h ^ (h >> 16u);
    return h;
}

fn detail_rotate_left(value: u32, amount: u32) -> u32 {
    return (value << amount) | (value >> (32u - amount));
}

/// Per-axis hashes for a cell and its successor, so the eight corners of a
/// value-noise cell cost six mixes between them rather than eight apiece.
struct DetailAxisHashes {
    lower: u32,
    upper: u32,
}

fn detail_axis_hashes(coordinate: i32, salt: u32) -> DetailAxisHashes {
    return DetailAxisHashes(
        detail_mix(bitcast<u32>(coordinate) ^ salt),
        detail_mix(bitcast<u32>(coordinate + 1) ^ salt),
    );
}

/// Combines three per-axis hashes into one corner value in [-1, 1). The
/// rotations are what stop `x ^ y ^ z` being symmetric in its arguments, which
/// would put a visible diagonal lattice through the whole planet.
fn detail_corner(x: u32, y: u32, z: u32) -> f32 {
    let combined = detail_avalanche(
        x ^ detail_rotate_left(y, 11u) ^ detail_rotate_left(z, 22u),
    );
    // Top 24 bits, which is all an f32 mantissa can hold anyway.
    return f32(combined >> 8u) * (2.0 / 16777216.0) - 1.0;
}

struct DetailNoise {
    value: f32,
    // d(value) / d(position), in cell units.
    gradient: vec3<f32>,
}

/// Cell index and in-cell fraction are supplied separately because the caller
/// cannot form `position` at metre wavelengths without losing it: the domain
/// coordinate reaches 4e6 there, where f32 quantises the fraction to 0.25.
/// Splitting keeps the cell exact (integers are exact well past 4e6) and lets
/// the fraction be built from a short anchor-local offset at full precision.
///
/// Returns the analytic gradient alongside the value. Central-differencing this
/// instead costs four more evaluations of the whole octave ladder per normal.
fn terrain_detail_value_noise(cell_index: vec3<i32>, cell_fraction: vec3<f32>) -> DetailNoise {
    let carry = floor(cell_fraction);
    let cell = cell_index + vec3<i32>(carry);
    let amount = cell_fraction - carry;
    let fade = amount * amount * (vec3<f32>(3.0) - amount * 2.0);
    let fade_slope = 6.0 * amount * (vec3<f32>(1.0) - amount);
    // Distinct salts per axis, so a cell on the diagonal does not hash the
    // same value three times over.
    let hx = detail_axis_hashes(cell.x, 0x27d4eb2fu);
    let hy = detail_axis_hashes(cell.y, 0x9e3779b9u);
    let hz = detail_axis_hashes(cell.z, 0x85ebca6bu);
    let a = detail_corner(hx.lower, hy.lower, hz.lower);
    let b = detail_corner(hx.upper, hy.lower, hz.lower);
    let c = detail_corner(hx.lower, hy.upper, hz.lower);
    let d = detail_corner(hx.upper, hy.upper, hz.lower);
    let e = detail_corner(hx.lower, hy.lower, hz.upper);
    let f = detail_corner(hx.upper, hy.lower, hz.upper);
    let g = detail_corner(hx.lower, hy.upper, hz.upper);
    let h = detail_corner(hx.upper, hy.upper, hz.upper);
    let k1 = b - a;
    let k2 = c - a;
    let k3 = e - a;
    let k4 = a - b - c + d;
    let k5 = a - c - e + g;
    let k6 = a - b - e + f;
    let k7 = -a + b + c - d + e - f - g + h;
    let value = a
        + k1 * fade.x
        + k2 * fade.y
        + k3 * fade.z
        + k4 * fade.x * fade.y
        + k5 * fade.y * fade.z
        + k6 * fade.z * fade.x
        + k7 * fade.x * fade.y * fade.z;
    let gradient = fade_slope
        * vec3<f32>(
            k1 + k4 * fade.y + k6 * fade.z + k7 * fade.y * fade.z,
            k2 + k5 * fade.z + k4 * fade.x + k7 * fade.z * fade.x,
            k3 + k6 * fade.x + k5 * fade.y + k7 * fade.x * fade.y,
        );
    return DetailNoise(value, gradient);
}

/// Folds one octave toward a ridged form. Must stay identical to
/// `terrain_detail_ridge` in planet.rs -- the camera stands on this.
fn terrain_detail_ridge(noise: DetailNoise) -> DetailNoise {
    let softened = sqrt(
        noise.value * noise.value
            + TERRAIN_DETAIL_RIDGE_SOFTNESS * TERRAIN_DETAIL_RIDGE_SOFTNESS,
    );
    let folded_value = (TERRAIN_DETAIL_RIDGE_CENTRE - softened) * TERRAIN_DETAIL_RIDGE_SCALE;
    let folded_gradient =
        noise.gradient * (-TERRAIN_DETAIL_RIDGE_SCALE * noise.value / softened);
    return DetailNoise(
        mix(noise.value, folded_value, TERRAIN_DETAIL_RIDGE_STRENGTH)
            * TERRAIN_DETAIL_RIDGE_NORMALISATION,
        mix(noise.gradient, folded_gradient, TERRAIN_DETAIL_RIDGE_STRENGTH)
            * TERRAIN_DETAIL_RIDGE_NORMALISATION,
    );
}

fn terrain_detail_domain(direction: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(direction, vec3<f32>(0.80, 0.48, -0.36)),
        dot(direction, vec3<f32>(-0.30, 0.85, 0.43)),
        dot(direction, vec3<f32>(0.52, -0.21, 0.82)),
    );
}

/// The domain map is linear, so a world offset transforms the same way a
/// direction does. That is what lets the fine octaves be built from a short
/// anchor-local offset instead of an absolute direction.
fn terrain_detail_domain_transpose(gradient: vec3<f32>) -> vec3<f32> {
    return gradient.x * vec3<f32>(0.80, 0.48, -0.36)
        + gradient.y * vec3<f32>(-0.30, 0.85, 0.43)
        + gradient.z * vec3<f32>(0.52, -0.21, 0.82);
}

struct TerrainDetail {
    height_meters: f32,
    // d(height) / d(world offset), metres per metre, i.e. a slope.
    slope: vec3<f32>,
}

/// `local_meters` is the offset from `anchor_direction`'s surface point, in
/// metres. Supplying it separately is what makes metre-scale octaves possible:
/// `anchor_direction * frequency` is huge but lands on an exact integer cell,
/// and the fraction is then carried entirely by `local_meters / wavelength`,
/// which stays small and keeps full f32 precision.
///
/// `filter_meters` is the spacing this height is about to be sampled at.
/// Octaves shorter than the filter are faded out rather than dropped, so pulling
/// the camera back retires them smoothly instead of aliasing.
/// `coarsest_meters` excludes octaves at or above it, so the mesh and the pixel
/// can split the ladder between them without either double-counting: the vertex
/// takes everything it can actually represent, the pixel takes the rest.
fn terrain_detail_band(
    anchor_direction: vec3<f32>,
    local_meters: vec3<f32>,
    filter_meters: f32,
    coarsest_meters: f32,
    scaled_macro_height_meters: f32,
) -> TerrainDetail {
    // Every octave's headroom is exactly zero at and below sea level. Return
    // before the integer noise walk instead of evaluating a field whose final
    // amplitude is guaranteed to be zero.
    if scaled_macro_height_meters <= 0.0 {
        return TerrainDetail(0.0, vec3<f32>(0.0));
    }
    let anchor_domain = terrain_detail_domain(anchor_direction);
    let local_domain = terrain_detail_domain(local_meters);
    // Everything below the filter contributes nothing, so bound the loop rather
    // than iterating the full ladder and multiplying by zero. From orbit this is
    // one or two octaves; only at ground level does it run the whole set.
    let span = TERRAIN_DETAIL_START_WAVELENGTH_METERS / max(filter_meters * 2.0, 1.0e-6);
    let active_octaves = clamp(i32(ceil(log2(max(span, 1.0)))) + 1, 1, TERRAIN_DETAIL_OCTAVES);
    var total = 0.0;
    var gradient = vec3<f32>(0.0);
    var wavelength = TERRAIN_DETAIL_START_WAVELENGTH_METERS;
    for (var octave = 0; octave < active_octaves; octave = octave + 1) {
        // Two samples per wavelength is the Nyquist limit; fade across an octave
        // above it so the cut is never visible.
        // Complementary fades: the low cut retires octaves the sampling cannot
        // carry, the high cut hands coarse octaves back to whoever owns them.
        // The two are mirror images so a split ladder sums to the whole.
        let fade = smoothstep(filter_meters * 2.0, filter_meters * 4.0, wavelength)
            * (1.0 - smoothstep(coarsest_meters * 2.0, coarsest_meters * 4.0, wavelength));
        if fade > 0.0 {
            let inverse_wavelength = 1.0 / wavelength;
            let anchor_cells = anchor_domain * (PLANET_RADIUS_METERS * inverse_wavelength);
            let cell_floor = floor(anchor_cells);
            let cell_index = vec3<i32>(cell_floor);
            let cell_fraction = (anchor_cells - cell_floor)
                + local_domain * inverse_wavelength;
            let noise = terrain_detail_ridge(
                terrain_detail_value_noise(cell_index, cell_fraction),
            );
            // Damp this octave where the surface built so far is already steep.
            // The amplitude therefore depends on the accumulated gradient,
            // which strictly adds a product-rule term to the gradient itself;
            // that term needs second derivatives of the whole ladder and is
            // omitted. It perturbs shading normals only -- the *height*, which
            // is what the camera stands on and what the surface probe measures,
            // is exact either way.
            let attenuation = 1.0
                / (1.0 + length(gradient) / TERRAIN_DETAIL_ATTENUATION_SLOPE);
            let octave_amplitude =
                wavelength * TERRAIN_DETAIL_ROUGHNESS * terrain_detail_octave_tilt(wavelength);
            let amplitude = octave_amplitude
                * fade
                * attenuation
                * terrain_detail_octave_headroom(
                    scaled_macro_height_meters,
                    octave_amplitude,
                );
            total = total + noise.value * amplitude;
            gradient = gradient + noise.gradient * (amplitude * inverse_wavelength);
        }
        wavelength = wavelength * 0.5;
    }
    return TerrainDetail(total, terrain_detail_domain_transpose(gradient));
}

/// Spacing of the baked samples a node is drawing from. This is the scale the
/// baked data stops carrying information at, and therefore the scale the
/// synthesised ladder has to start at if the two are not to describe the same
/// hills twice.
fn baked_sample_spacing_meters(source_level: u32) -> f32 {
    return 2.0 * PLANET_RADIUS_METERS / (exp2(f32(source_level)) * MATERIAL_TILE_LOGICAL_QUADS);
}

fn continuous_baked_sample_spacing_meters(
    face_uv: vec2<f32>,
    source_level: u32,
    blend_source_edges: bool,
) -> f32 {
    let dense_level = min(u32(terrain_settings.outmap_detail.x + 0.5), source_level);
    if !blend_source_edges {
        return baked_sample_spacing_meters(source_level);
    }
    var effective_level = f32(dense_level);
    var tile_coordinate =
        (face_uv + vec2<f32>(1.0)) * 0.5 * exp2(f32(source_level));
    var level = source_level;
    loop {
        if level <= dense_level {
            break;
        }
        let tile_uv = fract(tile_coordinate);
        let edge_distance = min(
            min(tile_uv.x, 1.0 - tile_uv.x),
            min(tile_uv.y, 1.0 - tile_uv.y),
        );
        effective_level += smoothstep(
            0.0,
            2.0 / MATERIAL_TILE_LOGICAL_QUADS,
            edge_distance,
        );
        tile_coordinate *= 0.5;
        level -= 1u;
    }
    return 2.0 * PLANET_RADIUS_METERS
        / (exp2(effective_level) * MATERIAL_TILE_LOGICAL_QUADS);
}

fn terrain_detail(
    anchor_direction: vec3<f32>,
    local_meters: vec3<f32>,
    filter_meters: f32,
    baked_spacing_meters: f32,
    scaled_macro_height_meters: f32,
) -> TerrainDetail {
    return terrain_detail_band(
        anchor_direction,
        local_meters,
        filter_meters,
        // The high cut removes octaves the baked data already carries. Without
        // it, extending the ladder to 4096m would stack a 246m octave on top of
        // the sparse corridor's own erosion, which has real structure down to
        // 0.24m -- the same hills twice, at two hundred metres of amplitude.
        baked_spacing_meters,
        scaled_macro_height_meters,
    );
}

/// Distance between mesh vertices for a node at this level. Relief finer than
/// this cannot exist in the geometry, so it is the handover point between the
/// vertex ladder and the per-pixel one.
fn terrain_vertex_spacing_meters(level: u32) -> f32 {
    return (2.0 / exp2(f32(level))) * PLANET_RADIUS_METERS / TERRAIN_CHUNK_QUADS;
}

/// Tilts a surface normal by a detail slope. Only the tangential part matters;
/// the radial component is the normal already.
fn terrain_detail_perturbed_normal(
    normal: vec3<f32>,
    direction: vec3<f32>,
    slope: vec3<f32>,
) -> vec3<f32> {
    let tangential_slope = slope - direction * dot(slope, direction);
    return normalize(normal - tangential_slope);
}

/// How much of one octave this ground can carry without being pushed into the
/// sea.
///
/// Per octave, not per ladder. A single scalar weight is what a 16m ladder could
/// get away with; a 492m one cannot, because gating every octave on the whole
/// ladder's reach means a 40m coastal plain loses its 4m hummocks along with the
/// 4km hills it genuinely has no room for. Asking each scale separately gives
/// mountains their big hills and plains their small ones, which is the same
/// answer relief-correlated amplitude would give and costs nothing extra.
///
/// The factor is what keeps the *sum* safe. Each octave alone would be safe at
/// two, but thirteen of them are not; a test walks the heights to confirm the
/// worst case stays under the elevation it is standing on.
///
/// Eight was the figure while every octave had the same roughness. Under the
/// spectral tilt the ladder is dominated by its longest octaves, so eight was
/// asking a 4km octave for 15.7km of elevation beneath it -- more than the
/// planet's highest ground -- and running it at 22% amplitude on a 4.7km
/// mountain. That gate, not the amplitude, was what held the mountains at
/// Cairngorm scale.
///
/// 5.5 is the tightest value that keeps the proof with margin. The worst case
/// is not on the mountain but at the shoreline, around 4m of elevation, where
/// the fine octaves are all fully admitted and the tilted long ones are gated
/// off entirely; there the ladder admits 0.77 of the elevation it stands on.
/// Four admits 1.06 of it, which is a coastline cut below its own sea.
const TERRAIN_DETAIL_HEADROOM_FACTOR: f32 = 5.5;

fn terrain_detail_octave_headroom(
    scaled_macro_height: f32,
    octave_amplitude_meters: f32,
) -> f32 {
    return smoothstep(
        0.0,
        octave_amplitude_meters * TERRAIN_DETAIL_HEADROOM_FACTOR,
        scaled_macro_height,
    );
}

/// The spacing detail is about to be sampled at. Tracks camera distance the
/// same way the normal probes do, so displacement and shading never disagree
/// about which octaves exist here.
fn terrain_detail_filter_meters(camera_distance_meters: f32) -> f32 {
    return max(
        camera_distance_meters * TERRAIN_DETAIL_FILTER_RATIO,
        TERRAIN_DETAIL_MIN_FILTER_METERS,
    );
}

fn terrain_macro_height_scale() -> f32 {
    let camera_altitude_meters = max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let blend = smoothstep(
        terrain_settings.outmap_height_blend.x,
        terrain_settings.outmap_height_blend.y,
        camera_altitude_meters,
    );
    return mix(
        terrain_settings.outmap_height_scale.x,
        terrain_settings.outmap_height_scale.y,
        blend,
    );
}

fn scaled_terrain_macro_height(macro_height_meters: f32) -> f32 {
    return select(
        macro_height_meters,
        macro_height_meters * terrain_macro_height_scale(),
        macro_height_meters > 0.0,
    );
}

fn gerstner_wave(
    direction: vec3<f32>,
    wave_axis: vec3<f32>,
    wavelength_meters: f32,
    amplitude_meters: f32,
    speed_meters_per_second: f32,
    steepness: f32,
    time_seconds: f32,
) -> OceanWaveContribution {
    let axis = normalize(wave_axis);
    let tangent_unnormalized = axis - direction * dot(axis, direction);
    let tangent_length = length(tangent_unnormalized);
    if tangent_length < 1.0e-4 {
        return OceanWaveContribution(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    }
    let tangent = tangent_unnormalized / tangent_length;
    let wave_number = 6.2831853 / wavelength_meters;
    let phase = wave_number
        * (dot(direction, axis) * PLANET_RADIUS_METERS + speed_meters_per_second * time_seconds);
    return OceanWaveContribution(
        tangent * (steepness * amplitude_meters * cos(phase)),
        amplitude_meters * sin(phase),
        tangent * (amplitude_meters * wave_number * cos(phase)),
    );
}

fn ocean_surface(direction: vec3<f32>, time_seconds: f32) -> OceanSurface {
    if !OCEAN_WAVES_ENABLED {
        return flat_ocean_surface(direction);
    }
    let first = gerstner_wave(direction, vec3<f32>(0.9, 0.1, 0.4), 900.0, 0.375, 4.0, 0.45, time_seconds);
    let second = gerstner_wave(direction, vec3<f32>(-0.3, 0.4, 0.85), 420.0, 0.2125, 5.0, 0.40, time_seconds);
    let third = gerstner_wave(direction, vec3<f32>(0.55, -0.75, 0.35), 160.0, 0.1125, 6.5, 0.34, time_seconds);
    let fourth = gerstner_wave(direction, vec3<f32>(-0.75, -0.2, 0.63), 65.0, 0.055, 8.0, 0.28, time_seconds);
    let fifth = gerstner_wave(direction, vec3<f32>(0.2, 0.95, -0.24), 24.0, 0.0275, 10.0, 0.20, time_seconds);
    let sixth = gerstner_wave(direction, vec3<f32>(-0.5, 0.7, -0.5), 9.0, 0.0125, 12.0, 0.14, time_seconds);
    let horizontal = first.horizontal_displacement + second.horizontal_displacement
        + third.horizontal_displacement + fourth.horizontal_displacement
        + fifth.horizontal_displacement + sixth.horizontal_displacement;
    let vertical = first.vertical_displacement + second.vertical_displacement
        + third.vertical_displacement + fourth.vertical_displacement
        + fifth.vertical_displacement + sixth.vertical_displacement;
    let slope = first.slope + second.slope + third.slope + fourth.slope + fifth.slope + sixth.slope;
    return OceanSurface(horizontal, vertical, normalize(direction - slope));
}

fn flat_ocean_surface(direction: vec3<f32>) -> OceanSurface {
    return OceanSurface(vec3<f32>(0.0), 0.0, normalize(direction));
}

fn density(altitude_meters: f32, scale_height_meters: f32) -> f32 {
    let clamped_altitude_meters = max(altitude_meters, 0.0);
    let edge_fade = 1.0 - smoothstep(
        ATMOSPHERE_HEIGHT_METERS - ATMOSPHERE_EDGE_FADE_METERS,
        ATMOSPHERE_HEIGHT_METERS,
        clamped_altitude_meters,
    );
    return exp(-clamped_altitude_meters / scale_height_meters) * edge_fade;
}

fn phase_rayleigh(cos_theta: f32) -> f32 {
    return 3.0 * (1.0 + cos_theta * cos_theta) / (16.0 * 3.14159265);
}

fn phase_mie(cos_theta: f32) -> f32 {
    let g_squared = MIE_G * MIE_G;
    let denominator = max(1.0 + g_squared - 2.0 * MIE_G * cos_theta, 1.0e-4);
    return 3.0 * (1.0 - g_squared) * (1.0 + cos_theta * cos_theta)
        / (8.0 * 3.14159265 * (2.0 + g_squared) * pow(denominator, 1.5));
}

fn twilight_solar_air_mass(solar_zenith_cosine: f32, sample_altitude_meters: f32) -> f32 {
    let grazing_air_mass = min(1.0 / max(solar_zenith_cosine, 0.125), 8.0);
    let twilight_depth = smoothstep(0.0, 0.12, max(-solar_zenith_cosine, 0.0));
    let base_air_mass = mix(grazing_air_mass, 12.0, twilight_depth);
    let horizon_amount = 1.0 - smoothstep(0.08, 0.30, solar_zenith_cosine);
    let upper_atmosphere_amount = smoothstep(30000.0, 120000.0, sample_altitude_meters);
    return base_air_mass * mix(1.0, 8.0, horizon_amount * upper_atmosphere_amount);
}

fn transmittance(
    start_altitude_meters: f32,
    end_altitude_meters: f32,
    distance_meters: f32,
) -> vec3<f32> {
    let rayleigh_density = 0.5
        * (density(start_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS)
            + density(end_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS));
    let mie_density = 0.5
        * (density(start_altitude_meters, MIE_SCALE_HEIGHT_METERS)
            + density(end_altitude_meters, MIE_SCALE_HEIGHT_METERS));
    return exp(-(RAYLEIGH_COEFFICIENT * rayleigh_density + MIE_COEFFICIENT * mie_density)
        * max(distance_meters, 0.0));
}

fn atmosphere_interval(radius_meters: f32, radial_dot_ray: f32) -> vec2<f32> {
    let discriminant = radial_dot_ray * radial_dot_ray
        + ATMOSPHERE_RADIUS_METERS * ATMOSPHERE_RADIUS_METERS
        - radius_meters * radius_meters;
    if discriminant <= 0.0 {
        return vec2<f32>(-1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-radial_dot_ray - root, -radial_dot_ray + root);
}

fn atmosphere_exit_distance(radius_meters: f32, radial_dot_ray: f32) -> f32 {
    return max(atmosphere_interval(radius_meters, radial_dot_ray).y, 0.0);
}

fn altitude_along_ray(radius_meters: f32, radial_dot_ray: f32, distance_meters: f32) -> f32 {
    return sqrt(
        radius_meters * radius_meters
            + 2.0 * radial_dot_ray * distance_meters
            + distance_meters * distance_meters,
    ) - PLANET_RADIUS_METERS;
}

fn sun_is_occluded(radius_meters: f32, radial_dot_sun: f32) -> bool {
    let discriminant = radial_dot_sun * radial_dot_sun
        - (radius_meters * radius_meters - PLANET_RADIUS_METERS * PLANET_RADIUS_METERS);
    return radial_dot_sun < 0.0 && discriminant >= 0.0;
}

fn sun_visibility(
    radius_meters: f32,
    radial_dot_sun: f32,
    transition_meters: f32,
) -> f32 {
    if radial_dot_sun >= 0.0 {
        return 1.0;
    }
    let closest_approach_meters = sqrt(max(
        radius_meters * radius_meters - radial_dot_sun * radial_dot_sun,
        0.0,
    ));
    let clearance_meters = closest_approach_meters - PLANET_RADIUS_METERS;
    // Preserve full illumination to the geometric limb, then use the broad
    // anti-banding transition only inside the planet shadow. Centring it on
    // zero made both aerial haze and the fullscreen sky fade too early.
    return smoothstep(-transition_meters, 0.0, clearance_meters);
}

fn surface_direct_sun_transmittance(
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
    sun_direction: vec3<f32>,
) -> vec3<f32> {
    let surface_radius = PLANET_RADIUS_METERS + surface_altitude_meters;
    let solar_elevation = dot(surface_direction, sun_direction);
    let radial_dot_sun = surface_radius * solar_elevation;
    // The RGB transmittance below progressively removes blue at low solar
    // elevation. Start reducing its intensity before geometric sunset too, so
    // terrain diffuse and ocean glints become dim red rather than staying
    // bright until they abruptly disappear behind the planet.
    let solar_visibility = smoothstep(
        -0.01,
        0.08,
        solar_elevation,
    );

    // The generic endpoint-average estimate spans the full 360km shell for
    // a noon surface point. That makes the near-zero density at its top count
    // as half the density of the entire path, nearly extinguishing direct
    // daylight before the existing surface-only intensity scale can matter.
    // Estimate a local scale-height air mass instead. It retains directional
    // warm attenuation near the terminator without altering sky scattering.
    let sun_zenith_cosine = max(solar_elevation, 0.0);
    let air_mass = min(1.0 / max(sun_zenith_cosine, 0.08), 12.0);
    let rayleigh_optical_depth = RAYLEIGH_COEFFICIENT
        * density(surface_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS)
        * RAYLEIGH_SCALE_HEIGHT_METERS
        * air_mass;
    let mie_optical_depth = MIE_COEFFICIENT
        * density(surface_altitude_meters, MIE_SCALE_HEIGHT_METERS)
        * MIE_SCALE_HEIGHT_METERS
        * air_mass;
    let transmitted_sunlight = exp(-(rayleigh_optical_depth + mie_optical_depth));
    // Keep the physically wavelength-dependent extinction, then make its last
    // visible range read as two distinct ground-light bands: orange first,
    // then red as the existing visibility fade takes the sun below the limb.
    let orange_amount = 1.0 - smoothstep(0.08, 0.30, max(solar_elevation, 0.0));
    let red_amount = 1.0 - smoothstep(-0.01, 0.08, solar_elevation);
    let orange_tint = vec3<f32>(1.20, 0.55, 0.16);
    let red_tint = vec3<f32>(1.35, 0.12, 0.03);
    let low_sun_tint = mix(
        mix(vec3<f32>(1.0), orange_tint, orange_amount),
        red_tint,
        red_amount,
    );
    return transmitted_sunlight * low_sun_tint * solar_visibility;
}

fn sky_radiance(
    normal: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
    sun_direction: vec3<f32>,
) -> vec3<f32> {
    let surface_radius = PLANET_RADIUS_METERS + surface_altitude_meters;
    let ray = normalize(normal + surface_direction * 0.05);
    let radial_dot_ray = surface_radius * dot(surface_direction, ray);
    let ray_length = atmosphere_exit_distance(surface_radius, radial_dot_ray);
    if ray_length <= 0.0 {
        return vec3<f32>(0.0);
    }

    // Use one density-weighted representative for each local sky direction.
    // A terrain-vertex raymarch multiplied by every visible chunk is too costly;
    // scale-height path lengths retain the same colour-producing coefficients
    // while keeping the work bounded to three analytic sky samples per vertex.
    let zenith_cosine = max(dot(surface_direction, ray), 0.08);
    let rayleigh_path_length = min(
        ray_length,
        RAYLEIGH_SCALE_HEIGHT_METERS / zenith_cosine,
    );
    let mie_path_length = min(
        ray_length,
        MIE_SCALE_HEIGHT_METERS / zenith_cosine,
    );
    let sample_distance = 0.5 * rayleigh_path_length;
    let sample_position = surface_direction * surface_radius + ray * sample_distance;
    let sample_radius = length(sample_position);
    let sample_direction = sample_position / sample_radius;
    let sample_altitude = sample_radius - PLANET_RADIUS_METERS;
    let sample_radial_dot_sun = sample_radius * dot(sample_direction, sun_direction);
    let lower_atmosphere_weight = density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS);
    let shadow_transition_meters = TWILIGHT_SHADOW_TRANSITION_METERS
        * mix(1.0, 2.0, lower_atmosphere_weight);
    let view_transmittance = transmittance(
        surface_altitude_meters,
        sample_altitude,
        sample_distance,
    );
    let sun_air_mass = twilight_solar_air_mass(
        dot(sample_direction, sun_direction),
        sample_altitude,
    );
    let sun_transmittance = exp(-(
        RAYLEIGH_COEFFICIENT
            * density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
            * RAYLEIGH_SCALE_HEIGHT_METERS
            * sun_air_mass
            + MIE_COEFFICIENT
                * density(sample_altitude, MIE_SCALE_HEIGHT_METERS)
                * MIE_SCALE_HEIGHT_METERS
                * sun_air_mass
    )) * sun_visibility(
        sample_radius,
        sample_radial_dot_sun,
        shadow_transition_meters,
    );
    let cos_theta = dot(ray, sun_direction);
    // Optical depth of the representative column, then the fraction of it that
    // actually scatters. This has to saturate, and the form it replaces did
    // not: `transmittance * coefficient * path_length` multiplies a term
    // growing linearly in path by one decaying exponentially in it, so the
    // product peaks and then collapses back toward zero. The channel with the
    // largest coefficient enters that collapse first, which is blue -- so the
    // horizon, where the column is longest, lost precisely the wavelength that
    // should dominate it. Measured before this: the horizon sky read
    // [0.640 0.627 0.083], a blue-to-red ratio of 0.13 under a 45-degree sun,
    // and terrain hazing correctly toward it therefore read as dimming rather
    // than as distance.
    //
    // `1 - exp(-optical_depth)` is the same form `aerial_perspective` has been
    // using all along; these two are one model and disagreed about it.
    let rayleigh_optical_depth = RAYLEIGH_COEFFICIENT
        * density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
        * rayleigh_path_length;
    let mie_optical_depth = MIE_COEFFICIENT
        * density(sample_altitude, MIE_SCALE_HEIGHT_METERS)
        * mie_path_length;
    let total_optical_depth = rayleigh_optical_depth + mie_optical_depth;
    let phase_weight = (
        rayleigh_optical_depth * phase_rayleigh(cos_theta)
            + mie_optical_depth * phase_mie(cos_theta)
    ) / max(total_optical_depth, vec3<f32>(1.0e-6));
    let scattered_fraction = vec3<f32>(1.0) - exp(-total_optical_depth);
    // No view transmittance here: the saturating fraction already accounts for
    // attenuation along the column it integrates. Applying both extinguished
    // the column twice, which is what drove the collapse.
    let solar_elevation = dot(surface_direction, sun_direction);
    let red_rising = smoothstep(-0.14, -0.03, solar_elevation);
    let red_fading = 1.0 - smoothstep(0.0, 0.09, solar_elevation);
    let red_transition = red_rising * red_fading;
    let sunward_red = mix(
        0.35,
        1.0,
        smoothstep(0.0, 1.0, max(cos_theta, 0.0)),
    );
    return sun_transmittance * phase_weight * scattered_fraction * SOLAR_RADIANCE
        + TWILIGHT_RED_RADIANCE * red_transition * sunward_red;
}

fn sky_diffuse_irradiance(
    normal: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
    sun_direction: vec3<f32>,
) -> vec3<f32> {
    // Sample the atmosphere directly above the surface. Near-horizontal rays
    // have extremely long optical paths and caused unstable, overbright bands
    // when evaluated sparsely per terrain vertex. Preserve the overhead sky's
    // colour while bounding its irradiance before HDR exposure and bloom.
    let local_sky = max(
        sky_radiance(normal, surface_direction, surface_altitude_meters, sun_direction),
        vec3<f32>(0.0),
    );
    let sunward_tangent = sun_direction
        - surface_direction * dot(surface_direction, sun_direction);
    let sunward_sky = max(
        sky_radiance(
            normalize(normal + sunward_tangent * 0.45),
            surface_direction,
            surface_altitude_meters,
            sun_direction,
        ),
        vec3<f32>(0.0),
    );
    // A steep or back-facing terrain facet can point below the visible sky
    // hemisphere even though the surface is still surrounded by daylight.
    // Keep a bounded fraction of the zenith radiance as ambient fill so those
    // faces do not collapse to black; sky_radiance remains zero on the night
    // side, so this does not create moonless self-emission.
    let overhead_sky = max(
        sky_radiance(
            surface_direction,
            surface_direction,
            surface_altitude_meters,
            sun_direction,
        ),
        vec3<f32>(0.0),
    );
    // At sunrise/sunset the visible sky is concentrated near the sunward
    // horizon, while the zenith sample can remain dark. Use that horizon
    // radiance as a bounded fill source for facets whose normals miss it.
    let sunward_horizon_direction = normalize(surface_direction + sunward_tangent * 1.25);
    let sunward_horizon_sky = max(
        sky_radiance(
            sunward_horizon_direction,
            surface_direction,
            surface_altitude_meters,
            sun_direction,
        ),
        vec3<f32>(0.0),
    );
    let sky = max(
        local_sky,
        max(
            sunward_sky * 0.65,
            max(overhead_sky * 0.75, sunward_horizon_sky * 0.65),
        ),
    );
    let peak = max(max(sky.x, sky.y), sky.z);
    let bounded_sky = sky / max(1.0, peak / 0.35);
    return bounded_sky * SKY_DIFFUSE_LIGHT_SCALE;
}

fn aerial_view_transmittance(
    start_altitude_meters: f32,
    end_altitude_meters: f32,
    atmospheric_view_length_meters: f32,
    surface_to_camera_zenith_cosine: f32,
) -> vec3<f32> {
    let rayleigh_density = 0.5
        * (density(start_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS)
            + density(end_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS));
    let mie_density = 0.5
        * (density(start_altitude_meters, MIE_SCALE_HEIGHT_METERS)
            + density(end_altitude_meters, MIE_SCALE_HEIGHT_METERS));
    let air_mass = min(1.0 / max(surface_to_camera_zenith_cosine, 0.08), 12.0);

    // This remains an endpoint-average optical-depth estimate, but a radial
    // space-to-ground ray must not count the entire tall shell as half-dense.
    // Two local scale heights reproduce that column using the same endpoint
    // average, while the air-mass factor retains long, opaque horizon paths.
    let rayleigh_path_length = min(
        atmospheric_view_length_meters,
        2.0 * RAYLEIGH_SCALE_HEIGHT_METERS * air_mass,
    );
    let mie_path_length = min(
        atmospheric_view_length_meters,
        2.0 * MIE_SCALE_HEIGHT_METERS * air_mass,
    );
    return exp(-(
        RAYLEIGH_COEFFICIENT * rayleigh_density * rayleigh_path_length
            + MIE_COEFFICIENT * mie_density * mie_path_length
    ));
}

fn aerial_density_sample_fraction(fraction: f32, closest_fraction: f32) -> f32 {
    if closest_fraction <= 0.05 {
        return pow(fraction, AERIAL_DENSITY_SAMPLE_EXPONENT);
    }
    if closest_fraction >= 0.95 {
        return 1.0 - pow(1.0 - fraction, AERIAL_DENSITY_SAMPLE_EXPONENT);
    }
    if fraction <= 0.5 {
        let local_fraction = fraction * 2.0;
        return closest_fraction
            * (1.0 - pow(1.0 - local_fraction, AERIAL_DENSITY_SAMPLE_EXPONENT));
    }
    let local_fraction = (fraction - 0.5) * 2.0;
    return closest_fraction
        + (1.0 - closest_fraction) * pow(local_fraction, AERIAL_DENSITY_SAMPLE_EXPONENT);
}

struct AerialPerspectiveComponents {
    transmittance: vec3<f32>,
    in_scatter: vec3<f32>,
}

fn aerial_perspective_components(
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> AerialPerspectiveComponents {
    let distance_meters = length(camera_relative_view_position);
    let camera_altitude_meters = camera.camera_planet_direction_view_altitude.w;
    let view_direction = normalize(camera_relative_view_position);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_direction_view = normalize(camera.sun_direction_view.xyz);
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude_meters;
    let radial_dot_view = camera_radius
        * dot(camera.camera_planet_direction_view_altitude.xyz, view_direction);
    let view_interval = atmosphere_interval(camera_radius, radial_dot_view);
    let view_start = max(view_interval.x, 0.0);
    let view_end = min(view_interval.y, distance_meters);
    if view_end <= view_start {
        return AerialPerspectiveComponents(vec3<f32>(1.0), vec3<f32>(0.0));
    }
    let atmospheric_view_length = view_end - view_start;
    let atmospheric_view_start_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_view,
        view_start,
    );
    let atmospheric_view_end_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_view,
        view_end,
    );
    let surface_to_camera_zenith_cosine = max(
        dot(planet_to_view(surface_direction), -view_direction),
        0.0,
    );
    let view_transmittance = aerial_view_transmittance(
        atmospheric_view_start_altitude,
        atmospheric_view_end_altitude,
        atmospheric_view_length,
        surface_to_camera_zenith_cosine,
    );
    // Use the same scale-height-limited columns as extinction. Applying the
    // full horizon chord here added light from atmosphere that the matching
    // transmittance had already treated as opaque, washing the surface out.
    let view_air_mass = min(
        1.0 / max(surface_to_camera_zenith_cosine, 0.08),
        12.0,
    );
    let rayleigh_in_scatter_path_length = min(
        atmospheric_view_length,
        2.0 * RAYLEIGH_SCALE_HEIGHT_METERS * view_air_mass,
    );
    let mie_in_scatter_path_length = min(
        atmospheric_view_length,
        2.0 * MIE_SCALE_HEIGHT_METERS * view_air_mass,
    );
    let cos_theta = dot(view_direction, sun_direction_view);
    let closest_distance = clamp(-radial_dot_view, view_start, view_end);
    let closest_fraction = (closest_distance - view_start) / atmospheric_view_length;
    var in_scatter = vec3<f32>(0.0);
    for (var index = 0u; index < AERIAL_IN_SCATTER_SAMPLE_COUNT; index += 1u) {
        let interval_start = f32(index) / f32(AERIAL_IN_SCATTER_SAMPLE_COUNT);
        let interval_end = f32(index + 1u) / f32(AERIAL_IN_SCATTER_SAMPLE_COUNT);
        let sample_start = aerial_density_sample_fraction(interval_start, closest_fraction);
        let sample_end = aerial_density_sample_fraction(interval_end, closest_fraction);
        let sample_fraction = 0.5 * (sample_start + sample_end);
        let in_scatter_distance = view_start + sample_fraction * atmospheric_view_length;
        let in_scatter_position_view = camera.camera_planet_direction_view_altitude.xyz
            * camera_radius + view_direction * in_scatter_distance;
        let in_scatter_radius = length(in_scatter_position_view);
        let in_scatter_direction = view_to_planet(in_scatter_position_view / in_scatter_radius);
        let in_scatter_altitude = in_scatter_radius - PLANET_RADIUS_METERS;
        let radial_dot_sun = in_scatter_radius * dot(in_scatter_direction, sun_direction);
        let solar_visibility = sun_visibility(
            in_scatter_radius,
            radial_dot_sun,
            TWILIGHT_SHADOW_TRANSITION_METERS * mix(
                1.0,
                2.0,
                density(in_scatter_altitude, RAYLEIGH_SCALE_HEIGHT_METERS),
            ),
        );
        let sun_zenith_cosine = dot(in_scatter_direction, sun_direction);
        let sun_air_mass = twilight_solar_air_mass(sun_zenith_cosine, in_scatter_altitude);
        let sun_transmittance = exp(-(
            RAYLEIGH_COEFFICIENT
                * density(in_scatter_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
                * RAYLEIGH_SCALE_HEIGHT_METERS
                * sun_air_mass
                + MIE_COEFFICIENT
                    * density(in_scatter_altitude, MIE_SCALE_HEIGHT_METERS)
                    * MIE_SCALE_HEIGHT_METERS
                    * sun_air_mass
        )) * solar_visibility;
        let view_transmittance_to_sample = aerial_view_transmittance(
            atmospheric_view_start_altitude,
            in_scatter_altitude,
            sample_fraction * atmospheric_view_length,
            surface_to_camera_zenith_cosine,
        );
        let rayleigh_optical_depth = RAYLEIGH_COEFFICIENT
            * density(in_scatter_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
            * rayleigh_in_scatter_path_length
            / f32(AERIAL_IN_SCATTER_SAMPLE_COUNT);
        let mie_optical_depth = MIE_COEFFICIENT
            * density(in_scatter_altitude, MIE_SCALE_HEIGHT_METERS)
            * mie_in_scatter_path_length
            / f32(AERIAL_IN_SCATTER_SAMPLE_COUNT);
        let total_optical_depth = rayleigh_optical_depth + mie_optical_depth;
        let phase_weight = (
            rayleigh_optical_depth * phase_rayleigh(cos_theta)
                + mie_optical_depth * phase_mie(cos_theta)
        ) / max(total_optical_depth, vec3<f32>(1.0e-6));
        let scattered_fraction = vec3<f32>(1.0) - exp(-total_optical_depth);
        in_scatter += view_transmittance_to_sample
            * sun_transmittance
            * phase_weight
            * scattered_fraction;
    }
    in_scatter *= SOLAR_RADIANCE * AERIAL_IN_SCATTER_GAIN;
    return AerialPerspectiveComponents(view_transmittance, in_scatter);
}

fn aerial_perspective(
    lit_surface_color: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> vec3<f32> {
    let components = aerial_perspective_components(
        camera_relative_view_position,
        surface_direction,
        surface_altitude_meters,
    );
    return lit_surface_color * components.transmittance + components.in_scatter;
}

fn ocean_aerial_perspective(
    water_surface_color: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> vec3<f32> {
    let aerial_color = aerial_perspective(
        water_surface_color,
        camera_relative_view_position,
        surface_direction,
        surface_altitude_meters,
    );
    return mix(
        water_surface_color,
        aerial_color,
        OCEAN_AERIAL_PERSPECTIVE_WEIGHT,
    );
}

struct TerrainFog {
    amount: f32,
    color: vec3<f32>,
}

fn terrain_fog(
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> TerrainFog {
    let distance_amount = smoothstep(
        TERRAIN_FOG_START_METERS,
        TERRAIN_FOG_END_METERS,
        length(camera_relative_view_position),
    );
    let low_altitude_amount = 1.0 - smoothstep(
        0.0,
        TERRAIN_FOG_MAX_CAMERA_ALTITUDE_METERS,
        camera.camera_planet_direction_view_altitude.w,
    );
    let surface_to_camera_direction = view_to_planet(
        normalize(-camera_relative_view_position),
    );
    let horizon_cosine = max(
        dot(surface_direction, surface_to_camera_direction),
        0.0,
    );
    let horizon_amount = 1.0 - smoothstep(
        TERRAIN_FOG_FULL_HORIZON_COSINE,
        TERRAIN_FOG_CLEAR_HORIZON_COSINE,
        horizon_cosine,
    );
    let fog_amount = distance_amount * low_altitude_amount * horizon_amount;
    if fog_amount <= 0.0 {
        return TerrainFog(0.0, vec3<f32>(0.0));
    }
    let local_fog_color = sky_radiance(
        surface_to_camera_direction,
        surface_direction,
        surface_altitude_meters,
        normalize(camera.sun_direction.xyz),
    );
    // Match the fog endpoint to the same camera sky ray used by the
    // fullscreen atmosphere. Sampling from the terrain point instead makes
    // high-relief surfaces use a different optical column, producing a pale
    // horizontal band against the actual sky background.
    let camera_surface_direction = normalize(
        view_to_planet(camera.camera_planet_direction_view_altitude.xyz),
    );
    let camera_fog_color = sky_radiance(
        surface_to_camera_direction,
        camera_surface_direction,
        camera.camera_planet_direction_view_altitude.w,
        normalize(camera.sun_direction.xyz),
    );
    return TerrainFog(
        fog_amount,
        mix(local_fog_color, camera_fog_color, 0.75),
    );
}

fn terrain_distance_fog(
    aerial_color: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> vec3<f32> {
    let fog = terrain_fog(
        camera_relative_view_position,
        surface_direction,
        surface_altitude_meters,
    );
    return mix(aerial_color, fog.color, fog.amount);
}

fn terrain_distance_fog_components(
    components: AerialPerspectiveComponents,
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> AerialPerspectiveComponents {
    let fog = terrain_fog(
        camera_relative_view_position,
        surface_direction,
        surface_altitude_meters,
    );
    return AerialPerspectiveComponents(
        components.transmittance * (1.0 - fog.amount),
        mix(components.in_scatter, fog.color, fog.amount),
    );
}

fn face_tangent_u(face: u32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(0.0, 0.0, -1.0); }
        case 1u: { return vec3<f32>(0.0, 0.0, 1.0); }
        case 2u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 3u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 4u: { return vec3<f32>(1.0, 0.0, 0.0); }
        default: { return vec3<f32>(-1.0, 0.0, 0.0); }
    }
}

fn face_tangent_v(face: u32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 1u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 2u: { return vec3<f32>(0.0, 0.0, -1.0); }
        case 3u: { return vec3<f32>(0.0, 0.0, 1.0); }
        case 4u: { return vec3<f32>(0.0, 1.0, 0.0); }
        default: { return vec3<f32>(0.0, 1.0, 0.0); }
    }
}

fn face_normal(face: u32) -> vec3<f32> {
    return cross(face_tangent_u(face), face_tangent_v(face));
}

fn face_component(direction: vec3<f32>, face: u32) -> f32 {
    if face <= 1u {
        return abs(direction.x);
    }
    if face <= 3u {
        return abs(direction.y);
    }
    return abs(direction.z);
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

fn biome_color(biome: u32) -> vec3<f32> {
    var display_color: vec3<f32>;
    switch biome {
        case 0u: { display_color = vec3<f32>(20.0, 65.0, 150.0) / 255.0; }
        case 1u: { display_color = vec3<f32>(45.0, 115.0, 190.0) / 255.0; }
        case 2u: { display_color = vec3<f32>(218.0, 238.0, 250.0) / 255.0; }
        case 3u: { display_color = vec3<f32>(130.0, 145.0, 120.0) / 255.0; }
        case 4u: { display_color = vec3<f32>(45.0, 105.0, 55.0) / 255.0; }
        case 5u: { display_color = vec3<f32>(105.0, 145.0, 65.0) / 255.0; }
        case 6u: { display_color = vec3<f32>(25.0, 125.0, 55.0) / 255.0; }
        case 7u: { display_color = vec3<f32>(205.0, 180.0, 105.0) / 255.0; }
        case 8u: { display_color = vec3<f32>(105.0, 100.0, 95.0) / 255.0; }
        default: { display_color = vec3<f32>(236.0, 240.0, 242.0) / 255.0; }
    }
    return srgb_to_linear(display_color);
}

fn biome_vegetation_amount(biome: u32, moisture: f32) -> f32 {
    switch biome {
        case 3u: { return mix(0.22, 0.48, moisture); }
        case 4u: { return mix(0.68, 0.92, moisture); }
        case 5u: { return mix(0.55, 0.82, moisture); }
        case 6u: { return mix(0.78, 1.0, moisture); }
        case 7u: { return mix(0.0, 0.10, moisture); }
        case 8u: { return mix(0.02, 0.16, moisture); }
        default: { return 0.0; }
    }
}

fn terrain_material_weights_for_biome(
    biome: u32,
    moisture: f32,
    macro_height_meters: f32,
    surface_normal: vec3<f32>,
    surface_direction: vec3<f32>,
    // Synthesised relief at this point, normalised to the ladder's own range.
    // Hollows hold water, so they carry the vegetation and the exposed rises
    // are where bare ground shows. Without this the split is a function of
    // biome and moisture alone, both of which vary over kilometres, so a
    // grassland renders as one unbroken colour from any distance.
    relief: f32,
) -> vec4<f32> {
    let slope = 1.0 - clamp(
        dot(normalize(surface_normal), surface_direction),
        0.0,
        1.0,
    );
    var rock_amount = smoothstep(0.10, 0.42, slope);
    if biome == 8u {
        rock_amount = max(rock_amount, 0.78);
    }

    let latitude_amount = abs(surface_direction.y);
    let snowline_meters = mix(6200.0, 2200.0, latitude_amount);
    var snow_amount = smoothstep(
        snowline_meters,
        snowline_meters + 900.0,
        macro_height_meters,
    ) * (1.0 - rock_amount * 0.35);
    if biome == 2u {
        snow_amount = 1.0;
    } else if biome == 9u {
        snow_amount = max(snow_amount, 0.88);
    }

    let exposed_amount = 1.0 - snow_amount;
    let base_amount = exposed_amount * (1.0 - rock_amount);
    let vegetation_amount = clamp(
        biome_vegetation_amount(biome, moisture)
            - relief * TERRAIN_MATERIAL_RELIEF_VEGETATION,
        0.0,
        1.0,
    );
    let weights = vec4<f32>(
        base_amount * vegetation_amount,
        base_amount * (1.0 - vegetation_amount),
        exposed_amount * rock_amount,
        snow_amount,
    );
    return weights / max(dot(weights, vec4<f32>(1.0)), 1.0e-5);
}

fn height_blend_material_weights(
    weights: vec4<f32>,
    material_heights: vec4<f32>,
) -> vec4<f32> {
    // The alpha channel carries small-scale material height. It perturbs the
    // continuous biome/slope weights so soil gathers in hollows and snow/rock
    // edges break up naturally without changing geometry or ownership.
    let candidates = weights + material_heights * 0.22;
    let highest = max(max(candidates.x, candidates.y), max(candidates.z, candidates.w));
    let blended = max(candidates - vec4<f32>(highest - 0.18), vec4<f32>(0.0)) * weights;
    return blended / max(dot(blended, vec4<f32>(1.0)), 1.0e-5);
}

fn debug_ocean_albedo() -> vec3<f32> {
    return vec3<f32>(0.008, 0.055, 0.28);
}

fn is_open_ocean_surface(outmap: bool, macro_height_meters: f32, biome_id: u32) -> bool {
    let ice = outmap && biome_id == 2u;
    let lake = outmap && biome_id == 1u;
    return macro_height_meters <= 0.0 && !ice && !lake;
}

fn outmap_ocean_coverage(outmap: bool, height_meters: f32) -> f32 {
    if !outmap {
        return select(0.0, 1.0, height_meters <= 0.0);
    }
    return 1.0 - smoothstep(-80.0, 120.0, height_meters);
}

// Lakes use the same shallow 200m coast transition as open ocean. Their
// positive basin floor is still terrain data, so height-based coverage avoids
// a hard categorical biome edge at the shoreline.
fn lake_coast_coverage(biome_id: u32, macro_height_meters: f32) -> f32 {
    return select(
        0.0,
        1.0 - smoothstep(0.0, 200.0, macro_height_meters),
        biome_id == 1u,
    );
}

struct BiomeBlendSample {
    ids: vec4<u32>,
    weights: vec4<f32>,
}

fn blended_biome_color(blend: BiomeBlendSample) -> vec3<f32> {
    return biome_color(blend.ids.x) * blend.weights.x
        + biome_color(blend.ids.y) * blend.weights.y
        + biome_color(blend.ids.z) * blend.weights.z
        + biome_color(blend.ids.w) * blend.weights.w;
}

fn terrain_material_color(
    outmap: bool,
    biome: u32,
    moisture: f32,
    base_color: vec3<f32>,
    macro_height_meters: f32,
    terrain_detail_meters: f32,
    surface_normal: vec3<f32>,
    surface_direction: vec3<f32>,
) -> vec3<f32> {
    var color = vec3<f32>(0.32, 0.58, 0.74);
    if !outmap {
        return color;
    }

    color = base_color * mix(0.88, 1.06, moisture);
    if biome != 2u {
        // Use bilinear terrain height, not a nearest biome class, for the
        // coast. This gives a continuous shallow-water/beach transition.
        let beach = 1.0 - smoothstep(20.0, 220.0, macro_height_meters);
        color = mix(color, srgb_to_linear(vec3<f32>(0.48, 0.40, 0.23)), beach * 0.65);
    }
    // Break up a coarse ancestor material tile at flight altitude without
    // changing its biome or coastline. Correlating this with the bounded
    // relief keeps ridges readable under both direct and aerial lighting.
    let detail_weight = smoothstep(100.0, 400.0, macro_height_meters);
    let detail = clamp(
        terrain_detail_meters / TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
        -1.0,
        1.0,
    );
    color *= 1.0 + detail * detail_weight * 0.22;

    // Preserve the baked biome as the base material, then use the rendered
    // displacement normal and physical altitude to make nearby slopes read as
    // rock and high ridges collect snow. These are continuous at tile edges
    // and add no runtime macro geography.
    let slope = 1.0 - clamp(dot(normalize(surface_normal), surface_direction), 0.0, 1.0);
    let rock_amount = smoothstep(0.10, 0.42, slope);
    let rock_color = srgb_to_linear(vec3<f32>(0.30, 0.28, 0.25));
    color = mix(color, rock_color, rock_amount * 0.72);
    let latitude_amount = abs(surface_direction.y);
    let snowline_meters = mix(6200.0, 2200.0, latitude_amount);
    let snow_amount = smoothstep(
        snowline_meters,
        snowline_meters + 900.0,
        macro_height_meters,
    ) * (1.0 - rock_amount * 0.35);
    let snow_color = srgb_to_linear(vec3<f32>(0.82, 0.87, 0.90));
    color = mix(color, snow_color, snow_amount);
    return color;
}

fn terrain_material_weights(
    blend: BiomeBlendSample,
    moisture: f32,
    macro_height_meters: f32,
    surface_normal: vec3<f32>,
    surface_direction: vec3<f32>,
    relief: f32,
) -> vec4<f32> {
    let weights = terrain_material_weights_for_biome(
        blend.ids.x,
        moisture,
        macro_height_meters,
        surface_normal,
        surface_direction,
        relief,
    ) * blend.weights.x + terrain_material_weights_for_biome(
        blend.ids.y,
        moisture,
        macro_height_meters,
        surface_normal,
        surface_direction,
        relief,
    ) * blend.weights.y + terrain_material_weights_for_biome(
        blend.ids.z,
        moisture,
        macro_height_meters,
        surface_normal,
        surface_direction,
        relief,
    ) * blend.weights.z + terrain_material_weights_for_biome(
        blend.ids.w,
        moisture,
        macro_height_meters,
        surface_normal,
        surface_direction,
        relief,
    ) * blend.weights.w;
    return weights / max(dot(weights, vec4<f32>(1.0)), 1.0e-5);
}

fn terrain_material_tint(
    outmap: bool,
    moisture: f32,
    blend: BiomeBlendSample,
    macro_height_meters: f32,
    base_albedo: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_normal: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    // Synthesised relief here, already filtered to this pixel's scale.
    terrain_detail_meters: f32,
    // Close-range tile coordinate and how much of it to use. Supplied by the
    // caller as an exact anchor/local split. The coordinate itself is built
    // only when its close-range contribution is non-zero.
    fine_anchor_direction: vec3<f32>,
    fine_local_meters: vec3<f32>,
    fine_weight: f32,
) -> vec3<f32> {
    if !outmap {
        return vec3<f32>(1.0);
    }
    // The tileable close-range texture is useful below a few kilometres, but
    // its 2 km repeat becomes a visible checkerboard while climbing away from
    // the landing site. Let the baked biome/material data take over before
    // that repetition reaches the orbital views.
    let fade = 1.0 - smoothstep(
        4000.0,
        32000.0,
        length(camera_relative_view_position),
    );
    if fade <= 0.0 {
        return vec3<f32>(1.0);
    }
    var fine_position = vec3<f32>(0.0);
    if fine_weight > 0.0 {
        fine_position = terrain_material_fine_position(
            fine_anchor_direction,
            fine_local_meters,
        );
    }
    let base_weights = terrain_material_weights(
        blend,
        moisture,
        macro_height_meters,
        surface_normal,
        surface_direction,
        clamp(
            terrain_detail_meters / TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
            -1.0,
            1.0,
        ),
    );
    var vegetation = vec4<f32>(0.0);
    var earth = vec4<f32>(0.0);
    var rock = vec4<f32>(0.0);
    var snow = vec4<f32>(0.0);
    // Most ground uses only two layers. Coherent weight branches avoid paying
    // three triplanar samples for a layer which contributes nothing.
    if base_weights.x > 1.0e-4 {
        vegetation = triplanar_material_sample(
            TERRAIN_MATERIAL_VEGETATION,
            surface_direction,
            surface_normal,
            fine_position,
            fine_weight,
        );
    }
    if base_weights.y > 1.0e-4 {
        earth = triplanar_material_sample(
            TERRAIN_MATERIAL_EARTH,
            surface_direction,
            surface_normal,
            fine_position,
            fine_weight,
        );
    }
    if base_weights.z > 1.0e-4 {
        rock = triplanar_material_sample(
            TERRAIN_MATERIAL_ROCK,
            surface_direction,
            surface_normal,
            fine_position,
            fine_weight,
        );
    }
    if base_weights.w > 1.0e-4 {
        snow = triplanar_material_sample(
            TERRAIN_MATERIAL_SNOW,
            surface_direction,
            surface_normal,
            fine_position,
            fine_weight,
        );
    }
    let weights = height_blend_material_weights(
        base_weights,
        vec4<f32>(vegetation.a, earth.a, rock.a, snow.a),
    );
    let material_albedo = vegetation.rgb * weights.x
        + earth.rgb * weights.y
        + rock.rgb * weights.z
        + snow.rgb * weights.w;
    let tint = clamp(
        material_albedo / max(base_albedo, vec3<f32>(0.015)),
        vec3<f32>(0.35),
        vec3<f32>(2.4),
    );
    return mix(vec3<f32>(1.0), tint, fade * 0.95);
}

fn triplanar_material_sample_at_position(
    layer: i32,
    texture_position: vec3<f32>,
    weights: vec3<f32>,
) -> vec4<f32> {
    let x_projection = textureSample(
        terrain_material_map,
        terrain_material_sampler,
        texture_position.yz,
        layer,
    );
    let y_projection = textureSample(
        terrain_material_map,
        terrain_material_sampler,
        texture_position.xz,
        layer,
    );
    let z_projection = textureSample(
        terrain_material_map,
        terrain_material_sampler,
        texture_position.xy,
        layer,
    );
    return x_projection * weights.x
        + y_projection * weights.y
        + z_projection * weights.z;
}

/// Tile coordinate for the close-range repeat, kept exact by never forming the
/// absolute one. The texture wraps, so the whole tile index is irrelevant and
/// only the fraction matters: take that from the node anchor, then add the
/// short anchor-relative offset, which is a handful of metres and so keeps full
/// f32 precision. The anchor fraction is itself quantised to a few centimetres,
/// which shows only as a small registration step between neighbouring nodes.
fn terrain_material_fine_position(
    anchor_direction: vec3<f32>,
    local_meters: vec3<f32>,
) -> vec3<f32> {
    let anchor_tiles = anchor_direction
        * (PLANET_RADIUS_METERS / TERRAIN_MATERIAL_DETAIL_TILE_METERS);
    // Warp built the same way the detail octaves are, so it reconstructs the
    // same absolute cell from any anchor and stays continuous across node
    // boundaries -- unlike the tile fraction below, which inherits the anchor
    // direction's own ~0.2m quantisation.
    let inverse_warp = 1.0 / TERRAIN_MATERIAL_DETAIL_WARP_WAVELENGTH_METERS;
    let warp_cells = terrain_detail_domain(anchor_direction)
        * (PLANET_RADIUS_METERS * inverse_warp);
    let warp_cell_floor = floor(warp_cells);
    let warp = terrain_detail_value_noise(
        vec3<i32>(warp_cell_floor),
        (warp_cells - warp_cell_floor)
            + terrain_detail_domain(local_meters) * inverse_warp,
    );
    return fract(anchor_tiles)
        + local_meters / TERRAIN_MATERIAL_DETAIL_TILE_METERS
        + terrain_detail_domain_transpose(warp.gradient)
            * TERRAIN_MATERIAL_DETAIL_WARP_TILES;
}

fn terrain_material_fine_weight(camera_distance_meters: f32) -> f32 {
    return 1.0 - smoothstep(
        TERRAIN_MATERIAL_DETAIL_NEAR_METERS,
        TERRAIN_MATERIAL_DETAIL_FAR_METERS,
        camera_distance_meters,
    );
}

fn triplanar_material_sample(
    layer: i32,
    surface_direction: vec3<f32>,
    surface_normal: vec3<f32>,
    fine_position: vec3<f32>,
    fine_weight: f32,
) -> vec4<f32> {
    // Planet-local metre scale makes every LOD evaluate the same material at
    // the same surface point. Triplanar projection avoids cube-face UV seams.
    let axis_weights = pow(abs(normalize(surface_normal)), vec3<f32>(6.0));
    let weights = axis_weights / max(dot(axis_weights, vec3<f32>(1.0)), 1.0e-5);
    // One seam-safe triplanar lookup per axis is enough at flight speed. The
    // retired domain warp and second scale repeated 24 sine hashes and six
    // texture samples for every contributing material layer.
    let texture_position = surface_direction
        * (PLANET_RADIUS_METERS / TERRAIN_MATERIAL_TILE_METERS);
    let coarse = triplanar_material_sample_at_position(layer, texture_position, weights);
    if fine_weight <= 0.0 {
        return coarse;
    }
    let fine = triplanar_material_sample_at_position(layer, fine_position, weights);
    // Modulate rather than replace. Both samples come from the same layer, so
    // their brightness ratio has a mean of one whatever that layer's palette
    // is, and the close-range tile can add grain without dragging the hue --
    // and without its repeat showing up as repeating colour.
    let luminance = vec3<f32>(0.2126, 0.7152, 0.0722);
    let ratio = clamp(
        dot(fine.rgb, luminance) / max(dot(coarse.rgb, luminance), 1.0e-4),
        0.4,
        2.2,
    );
    let gain = 1.0 + TERRAIN_MATERIAL_DETAIL_STRENGTH * fine_weight * (ratio - 1.0);
    // The alpha channel decides which layer wins the height blend, so taking it
    // partly from the fine tile is what puts soil in metre-scale hollows and
    // lets earth break through grass at all. Partly, not wholly: at full
    // strength the material boundaries follow the tile, and a tiled boundary is
    // far more visible than tiled grain.
    let height = mix(
        coarse.a,
        fine.a,
        fine_weight * TERRAIN_MATERIAL_DETAIL_HEIGHT_SHARE,
    );
    return vec4<f32>(coarse.rgb * gain, height);
}

fn ocean_lighting(
    normal: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    sun_transmittance: vec3<f32>,
    sky_diffuse: vec3<f32>,
) -> vec3<f32> {
    let view_direction = normalize(-camera_relative_view_position);
    let normal_view = normalize(planet_to_view(normal));
    let sun_direction_view = normalize(camera.sun_direction_view.xyz);
    let reflection_direction = view_to_planet(reflect(-view_direction, normal_view));
    let reflected_color = textureSampleLevel(
        environment_map,
        environment_sampler,
        reflection_direction,
        0.0,
    ).rgb;
    let facing = max(dot(normal_view, view_direction), 0.0);
    let fresnel = vec3<f32>(0.02) + vec3<f32>(0.98) * pow(1.0 - facing, 5.0);
    let half_vector = normalize(sun_direction_view + view_direction);
    let specular = pow(max(dot(normal_view, half_vector), 0.0), 128.0);
    let daylight = max(max(sun_transmittance.x, sun_transmittance.y), sun_transmittance.z);
    // Keep the water body a dark blue; direct sunlight and reflection still
    // provide the daylight highlights and glints.
    let diffuse = vec3<f32>(0.008, 0.055, 0.28)
        * (sky_diffuse * daylight
            + sun_transmittance * (0.4 * SURFACE_SUNLIGHT_SCALE));
    // The Phase 6 cubemap is static. It represents daytime sky reflection, so
    // gate it by direct daylight instead of reflecting a bright blue sky from
    // the fully occluded hemisphere.
    return diffuse
        + reflected_color * fresnel * daylight * OCEAN_REFLECTION_SCALE
        + sun_transmittance
            * specular
            * fresnel
            * (OCEAN_SUN_GLINT_SCALE * SURFACE_SUNLIGHT_SCALE);
}

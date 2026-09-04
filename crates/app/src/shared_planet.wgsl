const PLANET_RADIUS_METERS: f32 = 4000000.0;
const TERRAIN_AERIAL_UPPER_HORIZON_AIR_MASS_SCALE: f32 = 0.42;
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
// At the current 4096m start the unboosted finite ladder has a 475.08m absolute
// amplitude ceiling before the per-octave land headroom gate. A restrained
// 3.3% reduction from the former 0.06 roughness softens local gradients while
// leaving baked macro mountains unchanged. The former 8x long-wave boost is
// disabled below so observed ETOPO shapes dominate.
const TERRAIN_DETAIL_ROUGHNESS: f32 = 0.058;
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
// 4096 * 0.058 * (1 + 1/2 + ... + 1/4096) = 475.078m, rounded upward. The ray
// path and culling shell use it as a conservative bound.
const TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS: f32 = 475.1;
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
const ATMOSPHERE_HEIGHT_METERS: f32 = 2880000.0;
const PHYSICAL_ATMOSPHERE_PI: f32 = 3.141592653589793;
const SKY_VIEW_OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 640000.0;
const SKY_VIEW_ORBITAL_BLEND_START_METERS: f32 = 200000.0;
const SKY_VIEW_ORBITAL_BLEND_END_METERS: f32 = 400000.0;
const SKY_VIEW_ORBITAL_ATMOSPHERE_LUT_V: f32 = 0.72;
const SKY_VIEW_ORBITAL_GROUND_LUT_V: f32 = 0.88;
const ATMOSPHERE_EDGE_FADE_METERS: f32 = 1920000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 72000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 9600.0;
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
const AERIAL_IN_SCATTER_SAMPLE_COUNT: u32 = 2u;
const AERIAL_DENSITY_SAMPLE_EXPONENT: f32 = 3.0;
// Artistic aerial-only control, applied after physically bounded integration.
// It does not alter extinction, direct terrain/ocean lighting, or the sky pass.
const AERIAL_IN_SCATTER_GAIN: f32 = 3.0;
// Keep only a restrained fraction of the global aerial effect on water. The
// full in-scatter is deliberately retained for terrain and sky, but its warm
// high-altitude contribution can wash a blue ocean toward green/grey from
// orbit. The ocean shell and atmosphere limb still provide the distant haze.
const OCEAN_AERIAL_PERSPECTIVE_WEIGHT: f32 = 0.18;
const SKY_VIEW_MINIMUM_CAMERA_ALTITUDE_METERS: f32 = 200.0;
// Vegetation should keep its reflected green body colour in orbital views.
// A full atmospheric in-scatter term is correct for bare distant haze, but it
// overwhelms grass/forest albedo long before the land should read as blue.
const VEGETATION_AERIAL_IN_SCATTER_SCALE: f32 = 0.42;
const OCEAN_REFLECTION_SCALE: f32 = 0.35;
const OCEAN_SUN_GLINT_SCALE: f32 = 3.0;
const TWILIGHT_SHADOW_TRANSITION_METERS: f32 = 72000.0;
// Extra distance mist is driven by the sea-level-equivalent air column along
// the actual camera-to-surface segment. A vertical orbital view therefore
// crosses roughly one scale height of effective air, while a grazing view can
// cross many. There is deliberately no authored camera-altitude fade.
const TERRAIN_FOG_AIR_PATH_E_FOLD_METERS: f32 = 500000.0;
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

@group(2) @binding(8)
var atmosphere_surface_irradiance_lut: texture_2d<f32>;

@group(2) @binding(9)
var atmosphere_physical_sampler: sampler;

@group(2) @binding(10)
var atmosphere_sky_view_lut: texture_2d<f32>;

@group(2) @binding(11)
var atmosphere_sky_view_sampler: sampler;

@group(2) @binding(12)
var atmosphere_transmittance_lut: texture_2d<f32>;

struct OceanWaveSpec {
    axis: vec3<f32>,
    wavelength_meters: f32,
    // Calm and full-storm amplitudes. A storm is not a calm sea scaled up: it
    // moves the dominant band from the 1400 m swell down to a 280-430 m storm
    // sea. Same energy over a quarter of the wavelength is what makes a crest
    // tower over an eye-level camera instead of passing under it as a long
    // gentle rise. Both columns sum to the same total, so the height cap holds
    // at either end and everywhere between.
    amplitude_meters: f32,
    storm_amplitude_meters: f32,
    speed_meters_per_second: f32,
    steepness: f32,
}

struct OceanWaveContribution {
    horizontal_displacement: vec3<f32>,
    vertical_displacement: f32,
    slope: vec3<f32>,
}

struct OceanSurface {
    /// Raw crest height over what this depth can hold. Above 1 the wave is
    /// breaking. Carried out because the displacement below is already limited
    /// and so can never exceed 1 by construction -- shading off that tells you
    /// only that a crest exists, never how hard it is breaking.
    breaking_ratio: f32,
    horizontal_displacement: vec3<f32>,
    vertical_displacement: f32,
    normal: vec3<f32>,
    ripple_height: f32,
    ripple_slope: vec3<f32>,
}

// Broad displacement is only evaluated in the camera-local ocean patch. The
// rest of the planet remains the exact sea-level ownership shell; fine waves
// survive a little farther as normal-only detail, so the local patch does not
// end in a visible geometric ring.
const OCEAN_WAVES_ENABLED: bool = true;
// Diagnostic: keep only the two 1,400 m swells so the sea carries a single
// dominant octave. The 160/65/24/9 m global waves and the whole local ripple
// layer are silenced. Paired with `OCEAN_LARGE_SWELL_ONLY` in ocean.rs;
// collision must lose exactly the waves the render loses or the camera floats
// against water it cannot see.
const OCEAN_LARGE_SWELL_ONLY: bool = false;
const OCEAN_WAVE_COUNT: u32 = 17u;
// Leading entries of OCEAN_WAVE_TABLE that form the dominant swell.
const OCEAN_LARGE_SWELL_WAVE_COUNT: u32 = 2u;
// Mirrored byte-for-byte by `WAVES` in ocean.rs; the axis literals must match
// exactly, not merely to within rounding, because phase is
// wave_number * dot(direction, axis) * PLANET_RADIUS_METERS and a planet radius
// turns a 4th-decimal axis difference into tens of radians of phase.
var<private> OCEAN_WAVE_TABLE: array<OceanWaveSpec, 17> = array<OceanWaveSpec, 17>(
    OceanWaveSpec(vec3<f32>(0.9, 0.1, 0.4), 1400.0, 0.375, 0.09, 10.0, 0.45),
    OceanWaveSpec(vec3<f32>(0.86, 0.18, 0.48), 1400.0, 0.375, 0.09, 9.2, 0.4),
    OceanWaveSpec(vec3<f32>(0.1596, -0.599, 0.7847), 430.0, 0.0, 0.185, 24.0, 1.5),
    OceanWaveSpec(vec3<f32>(0.297, -0.7478, 0.5938), 350.0, 0.0, 0.205, 21.5, 1.5),
    OceanWaveSpec(vec3<f32>(0.3987, -0.8308, 0.3884), 280.0, 0.0, 0.18, 19.0, 1.5),
    OceanWaveSpec(vec3<f32>(0.576, -0.8032, 0.1519), 200.0, 0.0495, 0.0495, 6.0, 0.34),
    OceanWaveSpec(vec3<f32>(0.4646, -0.1875, 0.8654), 147.5, 0.0383, 0.0383, 6.59, 0.32),
    OceanWaveSpec(vec3<f32>(0.5761, -0.8032, 0.1515), 108.7, 0.0295, 0.0295, 7.18, 0.3),
    OceanWaveSpec(vec3<f32>(0.2007, 0.0492, 0.9784), 80.2, 0.0228, 0.0228, 7.77, 0.28),
    OceanWaveSpec(vec3<f32>(0.49, -0.8612, -0.1353), 59.1, 0.0176, 0.0176, 8.36, 0.26),
    OceanWaveSpec(vec3<f32>(0.1087, 0.131, 0.9854), 43.6, 0.0136, 0.0136, 8.95, 0.24),
    OceanWaveSpec(vec3<f32>(0.5241, -0.8493, -0.063), 32.1, 0.0105, 0.0105, 9.55, 0.22),
    OceanWaveSpec(vec3<f32>(-0.0574, 0.252, 0.966), 23.7, 0.0081, 0.0081, 10.14, 0.2),
    OceanWaveSpec(vec3<f32>(0.3157, -0.8148, -0.4862), 17.5, 0.0062, 0.0062, 10.73, 0.18),
    OceanWaveSpec(vec3<f32>(0.1407, 0.1008, 0.9849), 12.9, 0.0048, 0.0048, 11.32, 0.16),
    OceanWaveSpec(vec3<f32>(0.3542, -0.8289, -0.4329), 9.5, 0.0037, 0.0037, 11.91, 0.14),
    OceanWaveSpec(vec3<f32>(-0.2008, 0.3721, 0.9062), 7.0, 0.0029, 0.0029, 12.5, 0.12),
);
const OCEAN_GEOMETRY_FULL_DISTANCE_METERS: f32 = 4000.0;
const OCEAN_GEOMETRY_FADE_DISTANCE_METERS: f32 = 10000.0;
// OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE, OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE and
// OCEAN_STEEPNESS_SCALE are generated from `OCEAN_WAVE_SCALE` in ocean.rs and
// prepended to this file. Declaring them here as well would reintroduce exactly
// the copy that used to drift.
// Shoreline treatment. Everything at the water's edge keys off one quantity --
// how deep the water is under the fragment -- so every coast on the planet gets
// the same behaviour with nothing authored per location.
//
// Depth over which water reads as shallow. Less column means less of the light
// entering it is absorbed before it comes back out, so it lightens and takes
// the bottom's colour.
const OCEAN_SHALLOW_DEPTH_METERS: f32 = 6.0;
const OCEAN_SHALLOW_COLOUR: vec3<f32> = vec3<f32>(0.16, 0.52, 0.55);
// How far into breaking a crest must be before it starts going white. Below
// this the wave is merely feeling the bottom, not yet breaking on it.
// How far past the depth limit a crest must be before it whitens, and where it
// is fully white. Both are ratios of crest height to holdable height.
const OCEAN_BREAKING_KNEE: f32 = 4.0;
// Below this there is not enough water for foam to be made of.
const OCEAN_FOAM_MINIMUM_DEPTH_METERS: f32 = 1.1;
const OCEAN_BREAKING_FOAM_ONSET: f32 = 1.8;
const OCEAN_BREAKING_FOAM_FULL: f32 = 3.6;
// Where the wave has long since broken and the foam has dispersed.
const OCEAN_BREAKING_FOAM_SPENT: f32 = 6.0;
const OCEAN_BREAKING_FOAM_GONE: f32 = 16.0;
// Foam is spray over water, not paint: even a fully broken crest keeps some of
// the sea's colour, which stops the surf zone reading as a white sheet.
const OCEAN_BREAKING_FOAM_MAX: f32 = 0.82;
// The instantaneous column -- still depth plus the wave's own displacement --
// below which the swell is breaking and going white. Because it uses the live
// surface height rather than the sea bed alone, the surf line runs up and back
// down the beach with the water instead of sitting there as a painted ring.
const OCEAN_SURF_COLUMN_METERS: f32 = 0.25;
const OCEAN_SURF_COLOUR: vec3<f32> = vec3<f32>(0.92, 0.95, 0.96);
// Still used by the ripple layer and the raymarch path for how far a shore
// effect reaches; it no longer gates the swell, which is depth-limited instead.
const OCEAN_SHORE_FULL_DEPTH_METERS: f32 = 30.0;
const OCEAN_RIPPLE_FULL_DISTANCE_METERS: f32 = 2000.0;
const OCEAN_RIPPLE_FADE_DISTANCE_METERS: f32 = 8000.0;
const OCEAN_RIPPLE_FIRST_AMPLITUDE: f32 = 1.8;
const OCEAN_RIPPLE_SECOND_AMPLITUDE: f32 = 1.64;
const OCEAN_RIPPLE_THIRD_AMPLITUDE: f32 = 1.20;
const OCEAN_RIPPLE_FIRST_AXIS: vec3<f32> = vec3<f32>(0.72, 0.18, -0.67);
const OCEAN_RIPPLE_SECOND_AXIS: vec3<f32> = vec3<f32>(-0.31, 0.91, 0.28);
const OCEAN_RIPPLE_THIRD_AXIS: vec3<f32> = vec3<f32>(0.15, -0.58, 0.80);

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

/// Phase distance added by the shoaling bottom. Paired with
/// `shoaling_phase_offset_meters` in ocean.rs; see it for why this both turns
/// crests onto the contours and makes the swell arrive from seaward.
fn shoaling_phase_offset_meters(water_depth_meters: f32) -> f32 {
    let depth = max(water_depth_meters, 0.0);
    if depth >= OCEAN_REFRACTION_REFERENCE_DEPTH_METERS {
        return 0.0;
    }
    let remaining = OCEAN_REFRACTION_REFERENCE_DEPTH_METERS - depth;
    return -OCEAN_WAVE_PHASE_SPEED_SIGN * remaining * remaining
        / (2.0 * OCEAN_REFRACTION_REFERENCE_DEPTH_METERS
            * OCEAN_REFRACTION_NOMINAL_SHELF_SLOPE);
}

fn gerstner_wave(
    direction: vec3<f32>,
    wave_axis: vec3<f32>,
    wavelength_meters: f32,
    amplitude_meters: f32,
    speed_meters_per_second: f32,
    steepness: f32,
    time_seconds: f32,
    water_depth_meters: f32,
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
        * (dot(direction, axis) * PLANET_RADIUS_METERS
            + OCEAN_WAVE_PHASE_SPEED_SIGN * speed_meters_per_second * time_seconds
            + shoaling_phase_offset_meters(water_depth_meters));
    return OceanWaveContribution(
        tangent * (steepness * OCEAN_STEEPNESS_SCALE * amplitude_meters * cos(phase)),
        amplitude_meters * sin(phase),
        tangent * (amplitude_meters * wave_number * cos(phase)),
    );
}

fn ocean_ripple(
    direction: vec3<f32>,
    time_seconds: f32,
    camera_distance_meters: f32,
    shore_weight: f32,
    water_depth_meters: f32,
) -> OceanWaveContribution {
    let distance_weight = 1.0 - smoothstep(
        OCEAN_RIPPLE_FULL_DISTANCE_METERS,
        OCEAN_RIPPLE_FADE_DISTANCE_METERS,
        camera_distance_meters,
    );
    if distance_weight <= 0.0 || shore_weight <= 0.0 {
        return OceanWaveContribution(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    }
    // These shorter waves are part of the local geometry as well as its normal:
    // the CPU surface query mirrors their vertical displacement at the patch
    // centre, so nearby camera buoyancy cannot drift from the visible water.
    let first = gerstner_wave(direction, OCEAN_RIPPLE_FIRST_AXIS, 180.0, OCEAN_RIPPLE_FIRST_AMPLITUDE, 14.0, 0.0, time_seconds, water_depth_meters);
    let second = gerstner_wave(direction, OCEAN_RIPPLE_SECOND_AXIS, 70.0, OCEAN_RIPPLE_SECOND_AMPLITUDE, 11.0, 0.0, time_seconds, water_depth_meters);
    let third = gerstner_wave(direction, OCEAN_RIPPLE_THIRD_AXIS, 28.0, OCEAN_RIPPLE_THIRD_AMPLITUDE, 8.0, 0.0, time_seconds, water_depth_meters);
    let weight = distance_weight * shore_weight;
    return OceanWaveContribution(
        vec3<f32>(0.0),
        (first.vertical_displacement + second.vertical_displacement + third.vertical_displacement) * weight,
        (first.slope + second.slope + third.slope) * weight,
    );
}

/// Water colour at the shore, from the depth beneath it.
///
/// `still_depth_meters` is the sea bed below mean sea level; `surface_height_meters`
/// is the wave's displacement at this point. Their sum is the water actually
/// standing here at this instant, which is what decides both how shallow it
/// reads and whether it is breaking.
fn shoreline_water_albedo(
    open_water: vec3<f32>,
    still_depth_meters: f32,
    surface_height_meters: f32,
    breaking_ratio: f32,
) -> vec3<f32> {
    let shallow = 1.0 - smoothstep(0.0, OCEAN_SHALLOW_DEPTH_METERS, still_depth_meters);
    // Squared so the shallows stay tight to the beach rather than washing the
    // whole bay out.
    var albedo = mix(open_water, OCEAN_SHALLOW_COLOUR, shallow * shallow);
    // A crest that has used up what the depth can hold is breaking, and goes
    // white. Paired with `breaking_fraction` in ocean.rs. This is the shore
    // surf: it follows the wave, so the white travels in with each crest
    // instead of sitting on the beach as a painted ring.
    // How far past what the depth holds this crest would have stood if the
    // water let it. Unclamped, so it separates crests that are genuinely
    // tumbling from ones merely feeling the bottom -- which is what leaves the
    // foam in bands rather than as one sheet across the whole surf zone.
    // Surf is a band, not a field. Once a crest is many times what the depth
    // can hold it broke a long way back and the water behind it is spent, so
    // the foam has to fade out again -- otherwise the whole shelf whitens,
    // which is what raising the onset alone could never fix because the ratio
    // saturates across all of it.
    let crest_foam =
        smoothstep(OCEAN_BREAKING_FOAM_ONSET, OCEAN_BREAKING_FOAM_FULL, breaking_ratio)
            * (1.0 - smoothstep(OCEAN_BREAKING_FOAM_SPENT, OCEAN_BREAKING_FOAM_GONE, breaking_ratio))
            * OCEAN_BREAKING_FOAM_MAX;
    // And the wash right at the edge, where there is barely any water left.
    let column_meters = still_depth_meters + surface_height_meters;
    let wash = 1.0 - smoothstep(0.0, OCEAN_SURF_COLUMN_METERS, max(column_meters, 0.0));
    // Foam has to be made of water. Without this the wash keys off a depth of
    // zero and whitens ground the sea is barely covering.
    let has_water = smoothstep(0.0, OCEAN_FOAM_MINIMUM_DEPTH_METERS, still_depth_meters);
    return mix(
        albedo,
        OCEAN_SURF_COLOUR,
        max(crest_foam, wash * wash * OCEAN_BREAKING_FOAM_MAX) * has_water,
    );
}

fn ocean_surface(
    direction: vec3<f32>,
    time_seconds: f32,
    camera_distance_meters: f32,
    water_depth_meters: f32,
) -> OceanSurface {
    if !OCEAN_WAVES_ENABLED {
        return flat_ocean_surface(direction);
    }
    // Applied to the summed height further down, not here: the limit depends on
    // how tall this crest actually is, which is not known until the waves are
    // summed. See `breaking_weight` in ocean.rs.
    let shore_weight = 1.0;
    let geometry_weight = (1.0 - smoothstep(
        OCEAN_GEOMETRY_FULL_DISTANCE_METERS,
        OCEAN_GEOMETRY_FADE_DISTANCE_METERS,
        camera_distance_meters,
    )) * shore_weight;
    if geometry_weight <= 0.0 && camera_distance_meters >= OCEAN_RIPPLE_FADE_DISTANCE_METERS {
        return flat_ocean_surface(direction);
    }
    // The two dominant equal-amplitude swells are only 6.88 degrees apart.
    // Their slightly different phase speeds make the broad constructive and
    // destructive interference pattern evolve rather than lock in place.
    //
    // Crests here are small circles about each axis, not straight lines, so an
    // axis near its own pole gives curved long-period rollers while one on its
    // great circle gives dead-straight parallel bands. The swell pair is aimed
    // deliberately close to its poles at the ocean scenarios; re-aiming it onto
    // the great circles renders the sea as corduroy. The three shortest octaves
    // instead spread widely in azimuth, which is what breaks the crests up, and
    // are held at least 35 degrees off their poles at every ocean scenario so a
    // 24 m wave still renders near 24 m instead of collapsing into the 65 m
    // band.
    let storm_intensity = clamp(camera.flat_triangle_options.y, 0.0, 1.0);
    let storm_blend = smoothstep(0.15, 0.85, storm_intensity);
    var horizontal = vec3<f32>(0.0);
    var vertical = 0.0;
    var slope = vec3<f32>(0.0);
    for (var i = 0u; i < OCEAN_WAVE_COUNT; i = i + 1u) {
        let spec = OCEAN_WAVE_TABLE[i];
        var amplitude = mix(spec.amplitude_meters, spec.storm_amplitude_meters, storm_blend);
        if OCEAN_LARGE_SWELL_ONLY && i >= OCEAN_LARGE_SWELL_WAVE_COUNT {
            amplitude = 0.0;
        }
        let contribution = gerstner_wave(
            direction,
            spec.axis,
            spec.wavelength_meters,
            amplitude,
            spec.speed_meters_per_second,
            spec.steepness,
            time_seconds,
            water_depth_meters,
        );
        horizontal += contribution.horizontal_displacement;
        vertical += contribution.vertical_displacement;
        slope += contribution.slope;
    }
    var ripple = ocean_ripple(
        direction,
        time_seconds,
        camera_distance_meters,
        shore_weight,
        water_depth_meters,
    );
    if camera.flat_triangle_options.z > 0.5 {
        // Fixed water-following diagnostics compare against the broad CPU
        // sample. Remove sub-mesh ripples whose wavelengths are below the
        // coarse triangle spacing; otherwise interpolation can visibly put
        // the eye above one vertex and below its neighbouring crest.
        ripple = OceanWaveContribution(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    }
    if OCEAN_LARGE_SWELL_ONLY {
        // The ripple layer is a shorter octave by definition, so the large
        // swell diagnostic drops it whatever the camera is doing.
        ripple = OceanWaveContribution(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    }
    let horizontal_transport = select(1.0, 0.0, camera.flat_triangle_options.z > 0.5);
    let geometry_amplitude_scale = mix(
        OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE,
        OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE,
        storm_blend,
    );
    // A wave cannot stand taller than the water it is in. Squeeze the summed
    // crest toward what this depth can hold, so it flattens off as it shoals
    // instead of either cutting through the sea bed or being faded out before
    // it gets there. tanh rather than a clamp keeps the surface smooth and
    // differentiable through the break. Paired with `breaking_weight` in
    // ocean.rs, which the CPU collision query uses.
    let raw_vertical = vertical * geometry_weight * geometry_amplitude_scale;
    let breaking_limit_meters =
        0.5 * OCEAN_BREAKING_HEIGHT_TO_DEPTH_RATIO * max(water_depth_meters, 0.0);
    var breaking_weight = 0.0;
    if breaking_limit_meters > 0.0 {
        // Soft-max knee, paired with `breaking_weight` in ocean.rs: a crest
        // well under what the depth holds is left alone, and only bends as it
        // approaches. A tanh here bit everywhere and put the camera on a
        // shorter sea than the one being drawn.
        let ratio = pow(abs(raw_vertical) / breaking_limit_meters, OCEAN_BREAKING_KNEE);
        breaking_weight = pow(1.0 + ratio, -1.0 / OCEAN_BREAKING_KNEE);
    }
    let limited = geometry_weight * geometry_amplitude_scale * breaking_weight;
    // Zero depth is no water at all, not an infinitely broken wave. Calling it
    // the latter painted every flat where the bake carries no bathymetry as
    // solid foam, which is most of a gently shelving coast.
    var breaking_ratio = 0.0;
    if breaking_limit_meters > 0.0 {
        breaking_ratio = max(raw_vertical, 0.0) / breaking_limit_meters;
    }
    return OceanSurface(
        breaking_ratio,
        horizontal * limited * horizontal_transport,
        vertical * limited,
        normalize(direction - slope * limited - ripple.slope),
        ripple.vertical_displacement,
        ripple.slope,
    );
}

fn flat_ocean_surface(direction: vec3<f32>) -> OceanSurface {
    return OceanSurface(
        0.0,
        vec3<f32>(0.0),
        0.0,
        normalize(direction),
        0.0,
        vec3<f32>(0.0),
    );
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
    let upper_atmosphere_amount = smoothstep(60000.0, 240000.0, sample_altitude_meters);
    return base_air_mass * mix(1.0, 8.0, horizon_amount * upper_atmosphere_amount);
}

fn terrain_aerial_solar_air_mass(
    solar_zenith_cosine: f32,
    sample_altitude_meters: f32,
) -> f32 {
    let base_air_mass = twilight_solar_air_mass(
        solar_zenith_cosine,
        sample_altitude_meters,
    );
    // The fullscreen sky needs the stronger limb column for the visible
    // twilight gradient, but applying that same boost to distant terrain
    // facets makes a daytime horizon turn orange. Keep terrain's long view
    // rays warm without letting the upper-atmosphere multiplier dominate.
    let horizon_amount = 1.0 - smoothstep(0.08, 0.30, solar_zenith_cosine);
    let upper_atmosphere_amount = smoothstep(60000.0, 240000.0, sample_altitude_meters);
    return base_air_mass
        * mix(
            1.0,
            TERRAIN_AERIAL_UPPER_HORIZON_AIR_MASS_SCALE,
            horizon_amount * upper_atmosphere_amount,
        );
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

fn flat_horizon_sun_visibility(altitude_meters: f32, solar_zenith_cosine: f32) -> f32 {
    let radius_meters = PLANET_RADIUS_METERS + max(altitude_meters, 0.0);
    let planet_radius_ratio = PLANET_RADIUS_METERS / radius_meters;
    let horizon_cosine = -sqrt(max(1.0 - planet_radius_ratio * planet_radius_ratio, 0.0));
    let solar_angular_radius_sine = 0.004625;
    return smoothstep(
        horizon_cosine - solar_angular_radius_sine,
        horizon_cosine + solar_angular_radius_sine,
        solar_zenith_cosine,
    );
}

fn surface_direct_sun_transmittance(
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
    sun_direction: vec3<f32>,
) -> vec3<f32> {
    let optical_altitude = max(surface_altitude_meters, 0.0) / 4.5;
    let solar_zenith_cosine = dot(surface_direction, sun_direction);
    let visibility = flat_horizon_sun_visibility(
        surface_altitude_meters,
        solar_zenith_cosine,
    );
    let uv = vec2<f32>(
        clamp(max(solar_zenith_cosine, 0.0) * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(optical_altitude / 640000.0, 0.0, 1.0)),
    );
    // This is the same wavelength-dependent optical column used while
    // generating the physical sky. The LUT's below-horizon samples include
    // solid-planet occlusion, so visibility is instead evaluated against the
    // simple geometric horizon at the actual surface altitude.
    let transmittance = textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_physical_sampler,
        uv,
        0.0,
    ).rgb;
    return transmittance * visibility;
}

fn sky_diffuse_irradiance(
    normal: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
    sun_direction: vec3<f32>,
) -> vec3<f32> {
    let optical_altitude = max(surface_altitude_meters, 0.0) / 4.5;
    let uv = vec2<f32>(
        clamp(dot(surface_direction, sun_direction) * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(optical_altitude / 640000.0, 0.0, 1.0)),
    );
    let horizontal_diffuse = textureSampleLevel(
        atmosphere_surface_irradiance_lut,
        atmosphere_physical_sampler,
        uv,
        0.0,
    ).rgb;
    // The LUT integrates a horizontal Lambertian receiver. Retain most of
    // that broad sky fill on steep facets, while allowing upward-facing
    // terrain to receive the full physical E/pi value.
    let upward_facing = sqrt(max(dot(normal, surface_direction), 0.0));
    // The sky display applies one fixed, hue-preserving perceptual curve so
    // nautical twilight remains visible without auto exposure. Apply that
    // same presentation to E/pi before it lights the surface; otherwise the
    // visible blue sky would illuminate the terrain with its much smaller raw
    // radiometric value and appear disconnected from it.
    return perceptual_physical_sky_radiance(max(horizontal_diffuse, vec3<f32>(0.0)))
        * mix(0.65, 1.0, upward_facing)
        * SKY_DIFFUSE_LIGHT_SCALE;
}

fn physical_sky_sphere_horizon_cosine(camera_radius: f32, sphere_radius: f32) -> f32 {
    let radius_ratio = clamp(
        sphere_radius / max(camera_radius, sphere_radius),
        0.0,
        1.0,
    );
    return -sqrt(max(1.0 - radius_ratio * radius_ratio, 0.0));
}

fn physical_sky_view_v_from_zenith_cosine(
    zenith_cosine: f32,
    camera_radius: f32,
    camera_altitude: f32,
) -> f32 {
    let atmosphere_horizon = physical_sky_sphere_horizon_cosine(
        camera_radius,
        PLANET_RADIUS_METERS + SKY_VIEW_OPTICAL_ATMOSPHERE_HEIGHT_METERS,
    );
    let ground_horizon = physical_sky_sphere_horizon_cosine(
        camera_radius,
        PLANET_RADIUS_METERS,
    );
    let orbital_amount = smoothstep(
        SKY_VIEW_ORBITAL_BLEND_START_METERS,
        SKY_VIEW_ORBITAL_BLEND_END_METERS,
        camera_altitude,
    );
    let atmosphere_v = mix(
        (1.0 - atmosphere_horizon) * 0.5,
        SKY_VIEW_ORBITAL_ATMOSPHERE_LUT_V,
        orbital_amount,
    );
    let ground_v = mix(
        (1.0 - ground_horizon) * 0.5,
        SKY_VIEW_ORBITAL_GROUND_LUT_V,
        orbital_amount,
    );
    if zenith_cosine >= atmosphere_horizon {
        return atmosphere_v * (1.0 - zenith_cosine)
            / max(1.0 - atmosphere_horizon, 1.0e-6);
    }
    if zenith_cosine >= ground_horizon {
        return mix(
            atmosphere_v,
            ground_v,
            (atmosphere_horizon - zenith_cosine)
                / max(atmosphere_horizon - ground_horizon, 1.0e-6),
        );
    }
    return mix(
        ground_v,
        1.0,
        (ground_horizon - zenith_cosine) / max(ground_horizon + 1.0, 1.0e-6),
    );
}

fn physical_sky_view_uv(ray_view: vec3<f32>) -> vec2<f32> {
    let up = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let sun = normalize(camera.sun_direction_view.xyz);
    var toward_sun = sun - up * dot(up, sun);
    if dot(toward_sun, toward_sun) < 1.0e-6 {
        toward_sun = normalize(camera.camera_right.xyz);
    } else {
        toward_sun = normalize(toward_sun);
    }
    let side = normalize(cross(up, toward_sun));
    let view_zenith_cosine = clamp(dot(ray_view, up), -1.0, 1.0);
    let horizontal = ray_view - up * view_zenith_cosine;
    var azimuth = 0.0;
    if dot(horizontal, horizontal) > 1.0e-8 {
        let horizontal_direction = normalize(horizontal);
        azimuth = atan2(
            dot(horizontal_direction, side),
            dot(horizontal_direction, toward_sun),
        );
    }
    let camera_altitude = max(
        camera.camera_planet_direction_view_altitude.w,
        SKY_VIEW_MINIMUM_CAMERA_ALTITUDE_METERS,
    );
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude;
    return vec2<f32>(
        fract(azimuth / (2.0 * PHYSICAL_ATMOSPHERE_PI) + 0.5),
        clamp(
            physical_sky_view_v_from_zenith_cosine(
                view_zenith_cosine,
                camera_radius,
                camera_altitude,
            ),
            0.0,
            1.0,
        ),
    );
}

fn perceptual_physical_sky_radiance(radiance: vec3<f32>) -> vec3<f32> {
    let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    if luminance <= 1.0e-8 {
        return vec3<f32>(0.0);
    }
    let perceived_luminance = 0.22 * pow(luminance, 0.42);
    let gain = clamp(perceived_luminance / luminance, 0.35, 80.0);
    return radiance * gain;
}

fn physical_camera_sky_radiance(ray_view: vec3<f32>) -> vec3<f32> {
    let radiance = textureSampleLevel(
        atmosphere_sky_view_lut,
        atmosphere_sky_view_sampler,
        physical_sky_view_uv(ray_view),
        0.0,
    ).rgb;
    return perceptual_physical_sky_radiance(radiance);
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
        let sun_air_mass = terrain_aerial_solar_air_mass(
            sun_zenith_cosine,
            in_scatter_altitude,
        );
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

fn terrain_fog_air_path_meters(
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> f32 {
    let distance_meters = length(camera_relative_view_position);
    if distance_meters <= 1.0e-3 {
        return 0.0;
    }
    let view_direction = camera_relative_view_position / distance_meters;
    let camera_radius = PLANET_RADIUS_METERS
        + camera.camera_planet_direction_view_altitude.w;
    let radial_dot_view = camera_radius * dot(
        camera.camera_planet_direction_view_altitude.xyz,
        view_direction,
    );
    let view_interval = atmosphere_interval(camera_radius, radial_dot_view);
    let view_start = max(view_interval.x, 0.0);
    let view_end = min(view_interval.y, distance_meters);
    if view_end <= view_start {
        return 0.0;
    }

    let atmospheric_view_length = view_end - view_start;
    let start_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_view,
        view_start,
    );
    let end_altitude = max(surface_altitude_meters, 0.0);
    let surface_to_camera_zenith_cosine = max(
        dot(planet_to_view(surface_direction), -view_direction),
        0.0,
    );
    let air_mass = min(
        1.0 / max(surface_to_camera_zenith_cosine, 0.08),
        12.0,
    );
    let bounded_path_length = min(
        atmospheric_view_length,
        2.0 * RAYLEIGH_SCALE_HEIGHT_METERS * air_mass,
    );
    let average_density = 0.5
        * (density(start_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
            + density(end_altitude, RAYLEIGH_SCALE_HEIGHT_METERS));
    return average_density * bounded_path_length;
}

fn terrain_fog(
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> TerrainFog {
    let air_path_meters = terrain_fog_air_path_meters(
        camera_relative_view_position,
        surface_direction,
        surface_altitude_meters,
    );
    let fog_amount = 1.0 - exp(
        -air_path_meters / TERRAIN_FOG_AIR_PATH_E_FOLD_METERS,
    );
    if fog_amount <= 1.0e-4 {
        return TerrainFog(0.0, vec3<f32>(0.0));
    }
    // Match the fog endpoint to the same camera sky ray used by the
    // fullscreen physical atmosphere. The matching ray points from the
    // camera toward this terrain fragment; the opposite direction above is
    // retained only for the terrain horizon-angle test.
    let camera_to_surface_ray_view = normalize(camera_relative_view_position);
    return TerrainFog(
        fog_amount,
        physical_camera_sky_radiance(camera_to_surface_ray_view),
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
    // Water colour must never climb a positive land slope. Positive samples
    // remain land (with the beach material handling the shoreline); only
    // non-positive ocean samples can blend toward the analytic sea shell.
    return select(0.0, 1.0 - smoothstep(-80.0, 0.0, height_meters), height_meters <= 0.0);
}

// Lakes use the same shallow 200m coast transition as open ocean. Their
// positive basin floor is still terrain data, so height-based coverage avoids
// a hard categorical biome edge at the shoreline.
fn lake_coast_coverage(biome_id: u32, macro_height_meters: f32) -> f32 {
    return select(
        0.0,
        1.0 - smoothstep(-80.0, 0.0, macro_height_meters),
        biome_id == 1u && macro_height_meters <= 0.0,
    );
}

fn terrain_material_is_vegetation(biome_id: u32) -> bool {
    return biome_id == 4u || biome_id == 5u || biome_id == 6u;
}

fn terrain_material_is_snow(biome_id: u32) -> bool {
    return biome_id == 2u || biome_id == 9u;
}

fn terrain_material_transmittance(
    transmittance: vec3<f32>,
    biome_id: u32,
) -> vec3<f32> {
    var neutrality = 0.0;
    if terrain_material_is_vegetation(biome_id) {
        neutrality = 0.82;
    } else if terrain_material_is_snow(biome_id) {
        neutrality = 0.92;
    }
    let luminance = dot(transmittance, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(transmittance, vec3<f32>(luminance), neutrality);
}

fn terrain_material_in_scatter(
    in_scatter: vec3<f32>,
    biome_id: u32,
) -> vec3<f32> {
    var neutrality = 0.0;
    if terrain_material_is_vegetation(biome_id) {
        neutrality = 0.82;
    } else if terrain_material_is_snow(biome_id) {
        neutrality = 0.92;
    }
    let luminance = dot(in_scatter, vec3<f32>(0.2126, 0.7152, 0.0722));
    let material_scatter = mix(in_scatter, vec3<f32>(luminance), neutrality);
    return material_scatter * select(
        1.0,
        VEGETATION_AERIAL_IN_SCATTER_SCALE,
        terrain_material_is_vegetation(biome_id),
    );
}

fn neutralize_snow_surface_lighting(
    lighting: vec3<f32>,
    biome_id: u32,
) -> vec3<f32> {
    if !terrain_material_is_snow(biome_id) {
        return lighting;
    }
    let luminance = dot(lighting, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(lighting, vec3<f32>(luminance), 0.82);
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
        * (sky_diffuse + sun_transmittance * (0.4 * SURFACE_SUNLIGHT_SCALE));
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

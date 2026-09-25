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
// Extra amplitude below a few tens of metres. A self-similar ladder makes
// ground that rolls; the near field carried 23cm at 4m and 6cm at 1m, and
// shading that smoothness is what read as smeared wax. Mirrored in planet.rs.
const TERRAIN_DETAIL_SHORT_GAIN: f32 = 3.2;
const TERRAIN_DETAIL_SHORT_TAPER_METERS: f32 = 48.0;

fn terrain_detail_octave_tilt(wavelength_meters: f32) -> f32 {
    return 1.0
        + (TERRAIN_DETAIL_LONG_GAIN - 1.0)
            * smoothstep(
                TERRAIN_DETAIL_TILT_TAPER_METERS,
                TERRAIN_DETAIL_START_WAVELENGTH_METERS,
                wavelength_meters,
            )
        + (TERRAIN_DETAIL_SHORT_GAIN - 1.0)
            * (1.0
                - smoothstep(
                    TERRAIN_DETAIL_SHORT_TAPER_METERS * 0.25,
                    TERRAIN_DETAIL_SHORT_TAPER_METERS,
                    wavelength_meters,
                ));
}

const TERRAIN_DETAIL_OCTAVES: i32 = 13;
// What the whole ladder can reach: amplitude halves with wavelength, so the
// geometric series sums to twice its first term. Anything normalising against
// the detail field has to use this and not the retired CPU field's 111.5m,
// which is an order of magnitude larger and silently scales the result away.
// With the long-wave boost disabled this is the ordinary finite halving sum,
// 4096 * 0.058 * (1 + 1/2 + ... + 1/4096) = 475.078m, rounded upward. The ray
// path and culling shell use it as a conservative bound.
const TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS: f32 = 480.7;
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
/// Elevation between one crevasse and the next. Metres of height, not of
/// ground, so they crowd together as the ice steepens without needing a flow
/// model: on a 19-degree slope this is a 507m spacing, on a 41-degree one 252m.
///
/// It is this wide because of what the camera can resolve. At 1.5km one pixel
/// is 2.2m of ground, so a metre-scale crack is sub-pixel and renders as a
/// hairline; what reads at this range is icefall structure, tens of metres
/// across and hundreds apart.
const CREVASSE_VERTICAL_SPACING_METERS: f32 = 165.0;
/// Share of each cycle that is open slot rather than intact ice.
const CREVASSE_WIDTH_SHARE: f32 = 0.42;
/// Tangent of the wall tilt at the slot edge. This is the whole contrast
/// control: at zero the field is invisible whatever else is set.
const CREVASSE_WALL_TILT: f32 = 0.85;
/// Scale the bands are broken into separate segments over.
const CREVASSE_SEGMENT_WAVELENGTH_METERS: f32 = 1100.0;
/// How far a segment may slide along the contour, in whole cycles, so
/// neighbouring segments step rather than line up.
const CREVASSE_SEGMENT_OFFSET_CYCLES: f32 = 1.7;
/// Share of the ladder's full reach that counts as one block of relief. Small,
/// because the relief that breaks a crack is the metre-scale surface, not the
/// hundreds of metres the whole ladder can reach.
const CREVASSE_BLOCK_RELIEF_SHARE: f32 = 0.06;
/// How far the block field may slide a crack, in whole cycles.
const CREVASSE_BLOCK_OFFSET_CYCLES: f32 = 0.22;
/// Narrowest a crack is pinched by the block field, as a share of its width.
const CREVASSE_BLOCK_WIDTH_FLOOR: f32 = 0.12;
/// Share of the direct beam a slot floor loses to its own walls.
const CREVASSE_SUN_OCCLUSION: f32 = 0.92;
/// Ambient the slot interior loses to its own walls.
const CREVASSE_AMBIENT_OCCLUSION: f32 = 0.55;
/// Light that enters the ice and scatters back out of a slot wall. This is why
/// a crevasse reads blue rather than black, and it is the one place on this
/// planet where a shadow has a colour of its own.
const CREVASSE_INTERIOR_GLOW: f32 = 0.16;
const CREVASSE_FADE_NEAR_METERS: f32 = 6000.0;
const CREVASSE_FADE_FAR_METERS: f32 = 13000.0;
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
// Each material layer's own mean linear albedo -- what its 1x1 top mip holds.
// The tint below divides by these so it carries a layer's *texture variation*
// and not its brightness. Taken from `terrain_material_texel`, and pinned by
// `material_layer_means_match_the_generator` so they cannot drift.
const TERRAIN_MATERIAL_MEAN_VEGETATION: vec3<f32> = vec3<f32>(0.038619, 0.046335, 0.007556);
const TERRAIN_MATERIAL_MEAN_EARTH: vec3<f32> = vec3<f32>(0.147235, 0.071834, 0.019578);
const TERRAIN_MATERIAL_MEAN_ROCK: vec3<f32> = vec3<f32>(0.072114, 0.064923, 0.054733);
const TERRAIN_MATERIAL_MEAN_SNOW: vec3<f32> = vec3<f32>(0.551075, 0.643671, 0.689233);
const TERRAIN_MATERIAL_VEGETATION: i32 = 0;
const TERRAIN_MATERIAL_EARTH: i32 = 1;
const TERRAIN_MATERIAL_ROCK: i32 = 2;
const TERRAIN_MATERIAL_SNOW: i32 = 3;
const RENDER_DEBUG_FINAL: u32 = 0u;
const RENDER_DEBUG_RAW_ALBEDO: u32 = 1u;
const RENDER_DEBUG_SURFACE_LIGHTING: u32 = 2u;
const RENDER_DEBUG_AERIAL_CONTRIBUTION: u32 = 3u;
const RENDER_DEBUG_FLAT_TRIANGLES: u32 = 6u;
const RENDER_DEBUG_UNDERSIDE_TRANSMISSION: u32 = 7u;
const RENDER_DEBUG_UNDERSIDE_REFRACTED_SKY: u32 = 8u;
const RENDER_DEBUG_UNDERSIDE_REFLECTION_HIT: u32 = 9u;

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

// The moon's albedo markings, baked once at startup by moon_markings.wgsl.
// A 1x1 placeholder on bodies that have none.
@group(2) @binding(13)
var moon_marking_map: texture_cube<f32>;

@group(2) @binding(14)
var moon_marking_sampler: sampler;

@group(2) @binding(15)
var foam_history_map: texture_2d<f32>;

// FFT ocean field: one layer per cascade, (height, Dx, Dz, 0). Written by
// ocean_fft.wgsl; sampled here only when OCEAN_FFT_ENABLED.
@group(2) @binding(16)
var ocean_fft_map: texture_2d_array<f32>;

@group(2) @binding(17)
var ocean_fft_sampler: sampler;

struct OceanFftView {
    axis_u: vec4<f32>,
    axis_v: vec4<f32>,
    // Camera fractional tile u, v; tile length metres; unused.
    cascade: array<vec4<f32>, 3>,
    gain: vec4<f32>,
}

@group(2) @binding(18)
var<uniform> ocean_fft_view: OceanFftView;

// Camera-relative view-space position of the surface point being evaluated,
// set by the caller: it keeps wave coordinates precise where the unit
// direction alone would quantise to about 0.25m on this planet.
var<private> ocean_fft_view_position: vec3<f32>;
// Metres between adjacent mesh vertices, set only by the vertex stage. Zero in
// the fragment stage, which filters to its pixel footprint instead.
var<private> ocean_fft_vertex_spacing_meters: f32;

struct OceanWaveSpec {
    axis: vec3<f32>,
    wavelength_meters: f32,
    // Calm and full-storm amplitudes. A storm is not a calm sea scaled up: it
    // moves the dominant band from the 1400 m swell down to a 280-430 m storm
    // sea. Long swells are now restrained so the shorter wind sea remains
    // visible from a deck-height camera rather than being hidden by a wall of
    // water. The columns need not sum to the same total; the CPU height cap
    // takes the taller endpoint and checks every intermediate blend.
    amplitude_meters: f32,
    storm_amplitude_meters: f32,
    speed_meters_per_second: f32,
    steepness: f32,
}

struct OceanWaveContribution {
    horizontal_displacement: vec3<f32>,
    horizontal_derivative: mat3x3<f32>,
    vertical_displacement: f32,
    slope: vec3<f32>,
    /// How sharp this wave's crest is here: its dimensionless Gerstner
    /// steepness `k * a` times `sin(phase)`, so it peaks exactly at the crest,
    /// where the drawn profile peaks too. Being dimensionless is the whole
    /// point -- it does not scale with the wave's size, so a small sharp wave
    /// reads as thin just as a large one does.
    ///
    /// Retained as an authored sharpness proxy for the foam/lighting response.
    /// The rendered transport Jacobian below is the actual geometric curvature.
    convergence: f32,
}

fn ocean_outer_product(a: vec3<f32>, b: vec3<f32>) -> mat3x3<f32> {
    return mat3x3<f32>(a * b.x, a * b.y, a * b.z);
}

struct OceanSurface {
    /// Raw crest height over what this depth can hold. Above 1 the wave is
    /// breaking. Carried out because the displacement below is already limited
    /// and so can never exceed 1 by construction -- shading off that tells you
    /// only that a crest exists, never how hard it is breaking.
    breaking_ratio: f32,
    horizontal_displacement: vec3<f32>,
    horizontal_derivative: mat3x3<f32>,
    vertical_displacement: f32,
    slope: vec3<f32>,
    normal: vec3<f32>,
    ripple_height: f32,
    ripple_slope: vec3<f32>,
    /// Summed crest sharpness, dimensionless and scale-free: see
    /// `OceanWaveContribution::convergence`. Faded by the same geometry weight
    /// and amplitude scale as the displacement, so water drawn flat in the
    /// distance does not claim a sharpness it is not showing.
    crest_sharpness: f32,
}

// Broad displacement is only evaluated in the camera-local ocean patch. The
// rest of the planet remains the exact sea-level ownership shell; fine waves
// survive a little farther as normal-only detail, so the local patch does not
// end in a visible geometric ring.
const OCEAN_WAVES_ENABLED: bool = true;
// Diagnostic: keep only the three 1,400 m swells so the sea carries a single
// dominant octave. The 160/65/24/9 m global waves and the whole local ripple
// layer are silenced. Paired with `OCEAN_LARGE_SWELL_ONLY` in ocean.rs;
// collision must lose exactly the waves the render loses or the camera floats
// against water it cannot see.
const OCEAN_LARGE_SWELL_ONLY: bool = false;
const OCEAN_WAVE_COUNT: u32 = 18u;
// Leading entries of OCEAN_WAVE_TABLE that form the dominant swell.
const OCEAN_LARGE_SWELL_WAVE_COUNT: u32 = 3u;
// Mirrored byte-for-byte by `WAVES` in ocean.rs; the axis literals must match
// exactly, not merely to within rounding, because phase is
// wave_number * dot(direction, axis) * PLANET_RADIUS_METERS and a planet radius
// turns a 4th-decimal axis difference into tens of radians of phase.
var<private> OCEAN_WAVE_TABLE: array<OceanWaveSpec, 18> = array<OceanWaveSpec, 18>(
    OceanWaveSpec(vec3<f32>(0.9, 0.1, 0.4), 1400.0, 0.05, 0.012, 46.7449, 0.45),
    OceanWaveSpec(vec3<f32>(0.86, 0.18, 0.48), 1400.0, 0.05, 0.012, 46.7449, 0.4),
    OceanWaveSpec(vec3<f32>(0.65548185, 0.45377367, 0.60368286), 1400.0, 0.05, 0.012, 46.7449, 0.425),
    OceanWaveSpec(vec3<f32>(0.1596, -0.599, 0.7847), 430.0, 0.01, 0.037, 25.9063, 1.5),
    OceanWaveSpec(vec3<f32>(0.297, -0.7478, 0.5938), 350.0, 0.011, 0.041, 23.3725, 1.5),
    OceanWaveSpec(vec3<f32>(0.3987, -0.8308, 0.3884), 280.0, 0.0095, 0.036, 20.905, 1.5),
    OceanWaveSpec(vec3<f32>(-0.1455, 0.9255, 0.3498), 200.0, 0.0495, 0.0495, 17.6679, 0.34),
    OceanWaveSpec(vec3<f32>(0.7017, -0.5337, 0.4721), 147.5, 0.0383, 0.0383, 15.1728, 0.32),
    OceanWaveSpec(vec3<f32>(0.3314, 0.5967, -0.7308), 108.7, 0.0295, 0.0295, 13.0252, 0.3),
    OceanWaveSpec(vec3<f32>(0.0303, 0.3888, 0.9208), 80.2, 0.0228, 0.0228, 11.1881, 0.28),
    OceanWaveSpec(vec3<f32>(0.8446, -0.435, -0.312), 59.1, 0.0176, 0.0176, 9.6043, 0.26),
    OceanWaveSpec(vec3<f32>(-0.0552, 0.9878, -0.1455), 43.6, 0.0136, 0.0136, 8.2492, 0.24),
    OceanWaveSpec(vec3<f32>(0.4574, -0.2866, 0.8418), 32.1, 0.0105, 0.0105, 7.0782, 0.22),
    OceanWaveSpec(vec3<f32>(0.6013, 0.17, -0.7807), 23.7, 0.0081, 0.0081, 6.082, 0.2),
    OceanWaveSpec(vec3<f32>(-0.1235, 0.771, 0.6247), 17.5, 0.0062, 0.0062, 5.2262, 0.18),
    OceanWaveSpec(vec3<f32>(0.8015, -0.5719, 0.1745), 12.9, 0.0048, 0.0048, 4.4871, 0.16),
    OceanWaveSpec(vec3<f32>(0.1622, 0.8076, -0.567), 9.5, 0.0037, 0.0037, 3.8506, 0.14),
    OceanWaveSpec(vec3<f32>(0.1801, 0.1161, 0.9768), 7.0, 0.0029, 0.0029, 3.3054, 0.12),
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
// The body of the sea. Deliberately dark: what makes water read as deep is that
// almost nothing comes back out of it, and every bright thing on this surface --
// the sun glint, the sky reflection, the foam, the transmitted crest -- is
// added on top of this rather than mixed into it. Raising it flattens all four.
// Display-space tropical sand, shared by the dry beach and submerged bed.
const BEACH_SAND_COLOUR_SRGB: vec3<f32> = vec3<f32>(0.94, 0.89, 0.70);
// Ground material only: neither water ownership nor wave geometry uses this.
fn beach_sand_albedo(height_meters: f32) -> vec3<f32> {
    let dry_sand = srgb_to_linear(BEACH_SAND_COLOUR_SRGB);
    if height_meters <= 0.0 { return dry_sand; }
    let wet_sand = srgb_to_linear(vec3<f32>(0.62, 0.53, 0.35));
    // The submerged beach reaches sea level with the full cream palette.
    // Start the land's darker wet-sand treatment continuously from that same
    // colour, then retain the existing treatment from 4m inland height onward.
    // These are height metres, not a fixed horizontal beach width.
    let wet_band = smoothstep(0.0, 4.0, height_meters)
        * (1.0 - smoothstep(0.0, 20.0, height_meters));
    return mix(dry_sand, wet_sand, wet_band * 0.38);
}

// How fast the bed's turquoise contribution falls off with the water standing
// over it. Chosen by measuring tinted screen area on `ocean_clear_shallows`,
// not by eye. The open-water visibility e-fold (12.8m once halved for the
// two-way path) barely varies across the few metres a shoaling wave spans and
// left the whole surf zone tinted together, at 2.5% of the frame obviously
// turquoise. OCEAN_SHALLOW_DEPTH_METERS itself, 6m, was too sharp the other
// way and removed the effect almost entirely, at 0.2%. This sits between them
// and keeps the tint on the thinnest water only.
const OCEAN_SHALLOW_TINT_EFOLD_METERS: f32 = 9.0;
const OCEAN_BODY_COLOUR: vec3<f32> = vec3<f32>(0.005, 0.032, 0.170);
// Lit wave faces carry the reference's teal, while shaded troughs retain the
// dark blue body. The transition uses the existing wave normal and sun.
const OCEAN_SUNLIT_BODY_COLOUR: vec3<f32> = vec3<f32>(0.008, 0.150, 0.220);

fn ocean_body_albedo(normal: vec3<f32>) -> vec3<f32> {
    let sun_facing = max(dot(normal, normalize(camera.sun_direction.xyz)), 0.0);
    return mix(OCEAN_BODY_COLOUR, OCEAN_SUNLIT_BODY_COLOUR,
        smoothstep(0.20, 0.75, sun_facing));
}
// Where the transmitted turquoise starts and where it is full, in units of
// summed crest sharpness (`OceanSurface::crest_sharpness`) -- dimensionless
// Gerstner steepness, not metres of anything.
//
// This used to key off displacement above mean sea level, which was wrong in a
// way worth recording: it made transmission a property of how *tall* the water
// stood, so a small wave got none however sharp its tip, while a large lazy
// swell got it on its flanks. Thinness is what lets light through, and thinness
// does not scale with height. Steepness is dimensionless, so a 2m crest and a
// 30m crest of the same sharpness now read the same.
//
// Historical calibration notes (the permanent-storm statements below no longer
// describe normal interactive launches):
// These percentile measurements predate the third crossing swell. Its
// redistribution preserves the fold budget, not the phase distribution;
// recalibrate the thresholds when adding calm/storm sea-state transitions.
// Chosen against the measured distribution rather than by eye. The table's
// fold budget -- every component crest aligned at once, the theoretical
// maximum -- is 2.0507; with the calm amplitude column it is 0.7279. Sampling
// the sum over random phases puts calm p90 at 0.225 and p99 at 0.381, and
// storm p75 at 0.509 and p90 at 0.950.
//
// The first pair, 0.25 and 0.75, was picked from the calm column: 0.25 is
// "roughly the calm sea's top tenth of water". That reasoning was against a
// distribution this game never renders. `GLOBAL_OCEAN_STORM_INTENSITY` is
// pinned at 1.0, so `storm_blend` is 1.0 always and the storm column is the
// only one in play; the calm figures are unreachable. Measured against the
// storm column instead, the old pair put 36.9% of the sea under some
// turquoise, 25.3% over half strength, and 15.8% clipped flat at the top of
// the ramp -- so the claim that 0.75 "is reached only by a storm's sharpest
// crests" was wrong by a factor of about fifteen, and the tint read as a
// water colour rather than as thin water.
//
// Re-anchored on the storm percentiles that do occur: p90 = 0.950 begins the
// tint and p99 = 1.534 completes it, so turquoise starts on the sharpest tenth
// of the sea and is only full on the sharpest hundredth. That measures 8.2%
// under any tint and 2.8% over half strength.
// Current smaller-swell spectrum: 600,000 independent phase vectors, seed
// 14926. The calm p90/p99 are 0.066/0.118, the storm p90/p99 0.121/0.212.
// Wind filtering and shore steering can still change the local distribution.
const OCEAN_CREST_TRANSMISSION_CALM_ONSET: f32 = 0.066;
const OCEAN_CREST_TRANSMISSION_CALM_FULL: f32 = 0.118;
const OCEAN_CREST_TRANSMISSION_ONSET: f32 = 0.121;
const OCEAN_CREST_TRANSMISSION_FULL: f32 = 0.212;
// Subtle open-ocean crest transmission, not tropical water colour. Real crests
// are mostly foam and specular; thin-water colour should only bias the most
// strongly backlit sharp crests.
const OCEAN_CREST_TRANSMISSION_TINT: vec3<f32> = vec3<f32>(0.018, 0.045, 0.040);
// Fine ripple crests cover very little screen area, so the restrained broad-
// crest tint above disappeared through exposure and tone mapping. Give only
// that already-localised selector enough blue-green radiance to read by eye.
const OCEAN_FINE_CREST_TRANSMISSION_TINT: vec3<f32> = vec3<f32>(0.025, 0.160, 0.360);
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

// How far a submerged eye can see, in metres. The usual definition of
// visibility: the range at which contrast is down to 2%, so the extinction
// e-fold is this over ln(50) rather than this itself.
const OCEAN_UNDERWATER_VISIBILITY_METERS: f32 = 100.0;
// What the water itself looks like once everything else has been extinguished.
// Blue-green rather than the sky's blue: water absorbs red first, then green,
// which is why the far end of a flooded quarry is this colour and not navy.
const OCEAN_UNDERWATER_TINT: vec3<f32> = vec3<f32>(0.055, 0.30, 0.42);
// Real wave faces are not optically smooth. Unresolved capillary roughness and
// suspended microbubbles scatter a small amount of skylight into the nominally
// total-internal-reflection region, keeping a grazing underside from reading as
// a perfect sand-coloured mirror. This is deliberately bounded: the resolved
// Snell window and the reflected submerged scene remain the dominant terms.
const OCEAN_UNDERSIDE_SKYLIGHT_BLEND: f32 = 0.25;
// Aerated crests are a diffuse layer, not a clear water-air interface. From
// below they replace the directional sky/reflection with softer, paler light.
const OCEAN_UNDERSIDE_FOAM_NEUTRALISATION: f32 = 0.55;
const OCEAN_UNDERSIDE_FOAM_RADIANCE_SCALE: f32 = 0.80;

// Whitecaps. A crest that is steep enough spills and goes white wherever it is,
// with no shore involved -- which is the whole difference from the surf above,
// and why the open sea had no foam on it at all.
//
// The previous spectrum was tuned against measured coverage in the render,
// not against the CPU slope
// probe. `open_sea_slope_distribution` reports p90 0.703 and p99 1.041 on the
// open sea, but it reads `global_wave_slope`, which excludes the local ripple
// layer that the *rendered* normal carries -- so thresholds taken from it put
// foam on 27% of the sea. Real whitecap coverage is a few per cent even in a
// storm. Swept against `ocean_rough_horizon`:
//
//     onset 0.75 / full 1.25   27.2%
//     onset 0.95 / full 1.55    4.6%
//     onset 1.15 / full 1.85    0.4%
//
// The smaller-swell spectrum needs a lower slope band; the positive-height
// gate below still keeps whitecaps on crests rather than steep troughs.
const OCEAN_WHITECAP_SLOPE_ONSET: f32 = 0.35;
const OCEAN_WHITECAP_SLOPE_FULL: f32 = 0.80;
// Foam belongs on the upper part of a wave. A trough has faces just as steep as
// a crest does, and foam sitting in the hollows reads as scum, not as breaking.
//
// Written as a fraction of the sea's own maximum crest rather than in metres, so
// it keeps its meaning when `OCEAN_WAVE_SCALE` changes the size of the sea; a
// fixed 4m meant one thing against a 42m calm cap and another against the 53m
// storm cap. The band also starts *below* mean level. The old `smoothstep(0.0,
// 4.0, h)` left 49% of the rendered sea at exactly zero with only 6.3% of it
// anywhere inside the ramp, so the term was a step rather than a fade, and the
// edge of that step is the mean-water contour -- which drew a straight line
// across the swell. Foam thrown off a breaking crest runs down the face and
// lingers; it does not stop dead at sea level. It still has to be gone a few
// metres into the trough, which is what the negative low end is bounded by.
const OCEAN_WHITECAP_CREST_LOW_FRACTION: f32 = -0.08;
const OCEAN_WHITECAP_CREST_HIGH_FRACTION: f32 = 0.18;
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
    if BODY_IS_MOON {
        return moon_height(direction);
    }
    return placeholder_octave(direction, 8.0, 2800.0)
        + placeholder_octave(direction, 512.0, 600.0)
        + placeholder_octave(direction, 32768.0, 100.0)
        + placeholder_octave(direction, 2097152.0, 3.0);
}

/// One impact structure, mirrored from `moon.rs::profile`. `t` is angular
/// distance over rim radius: a bowl inside the crest, an ejecta blanket out to
/// twice that, and exactly zero beyond, so each crater stays local and the
/// datum does not drift with the catalogue's size.
fn moon_crater_profile(t: f32, depth_meters: f32, rim_meters: f32) -> f32 {
    if t >= MOON_EJECTA_EXTENT {
        return 0.0;
    }
    if t <= 1.0 {
        let bowl = depth_meters * (t * t - 1.0);
        let rim = rim_meters * t * t * t;
        return bowl + rim;
    }
    // Thins as roughly an inverse cube, the way a real blanket does.
    let outer = clamp((t - 1.0) / (MOON_EJECTA_EXTENT - 1.0), 0.0, 1.0);
    let remaining = 1.0 - outer;
    return rim_meters * remaining * remaining * remaining;
}

/// The moon's macro surface: its impact history rather than eroded geography.
/// The planet reads this shape from baked tiles; an airless body synthesises it,
/// which is why the moon needs no outmap.
fn moon_height(direction: vec3<f32>) -> f32 {
    // Just the craters. Ice is where the sun never reaches, which is a question
    // about shadow that only the bake can answer -- see
    // `baker::moon::classify_ice`. The placeholder shown without a bake is
    // therefore bare regolith, which is honest: it has no ice rather than ice
    // in the wrong places.
    return MOON_DATUM_METERS + moon_raw_height(direction);
}

/// The impact field, mirrored from `moon.rs`.
/// The slice of `MOON_FIELD` that can reach a sample at this latitude sine.
///
/// Latitude is 1-Lipschitz on the sphere, so a crater whose latitude differs by
/// more than its reach cannot touch the sample; the field is sorted by
/// latitude, so what survives is one contiguous run. This is the whole reason
/// a catalogue of hundreds is affordable: without it every sample pays for
/// every crater. Mirrors `moon.rs::field_index_range`.
fn moon_field_index_range(latitude_sine: f32) -> vec2<u32> {
    let sine = clamp(latitude_sine, -1.0, 1.0);
    let cosine = sqrt(max(1.0 - sine * sine, 0.0));
    // sin and cos of (latitude +- window) by the angle-sum identities, so no
    // `asin`. Past a pole the window wraps over it and the bound becomes the
    // pole, which the sign of cos(latitude +- window) detects.
    var upper = clamp(sine * MOON_FIELD_WINDOW_COS + cosine * MOON_FIELD_WINDOW_SIN, -1.0, 1.0);
    if cosine * MOON_FIELD_WINDOW_COS - sine * MOON_FIELD_WINDOW_SIN < 0.0 {
        upper = 1.0;
    }
    var lower = clamp(sine * MOON_FIELD_WINDOW_COS - cosine * MOON_FIELD_WINDOW_SIN, -1.0, 1.0);
    if cosine * MOON_FIELD_WINDOW_COS + sine * MOON_FIELD_WINDOW_SIN < 0.0 {
        lower = -1.0;
    }
    // Invert `z = 1 - 2 * (index + 0.5) / count`, which falls with the index.
    let count = f32(MOON_FIELD_COUNT);
    let first = max(floor((1.0 - upper) * count * 0.5 - 0.5), 0.0);
    let last = max(ceil((1.0 - lower) * count * 0.5 - 0.5), 0.0);
    return vec2<u32>(
        u32(min(first, count - 1.0)),
        u32(min(last, count - 1.0)),
    );
}

/// One crater's height at a direction, or zero if it does not reach.
fn moon_contribution(crater: vec4<f32>, depths: vec3<f32>, unit: vec3<f32>) -> f32 {
    let cosine = clamp(dot(crater.xyz, unit), -1.0, 1.0);
    // Beyond the blanket the profile is exactly zero, so this is an
    // optimisation and not an approximation. It catches what survives the
    // latitude window but is far away in longitude.
    if cosine <= depths.z {
        return 0.0;
    }
    return moon_crater_profile(acos(cosine) / crater.w, depths.x, depths.y);
}

fn moon_raw_height(direction: vec3<f32>) -> f32 {
    let unit = normalize(direction);
    var total = 0.0;
    for (var index = 0u; index < MOON_BASIN_COUNT; index = index + 1u) {
        total = total + moon_contribution(MOON_BASINS[index], MOON_BASINS_DEPTHS[index], unit);
    }
    let range = moon_field_index_range(unit.y);
    for (var index = range.x; index <= range.y; index = index + 1u) {
        total = total + moon_contribution(MOON_FIELD[index], MOON_FIELD_DEPTHS[index], unit);
    }
    return total;
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
/// A height field laid out by direction cannot describe a cliff: across a steep
/// face its gradient is stretched down the fall line into stripes. Fade the
/// detail out as the face steepens. `1 - cos`: 0.18 is 35 degrees, 0.43 is 55.
fn terrain_detail_steep_fade(normal: vec3<f32>, direction: vec3<f32>) -> f32 {
    let steepness = 1.0 - clamp(dot(normalize(normal), direction), 0.0, 1.0);
    return 1.0 - smoothstep(0.181, 0.426, steepness);
}

fn terrain_detail_perturbed_normal(
    normal: vec3<f32>,
    direction: vec3<f32>,
    slope: vec3<f32>,
) -> vec3<f32> {
    let tangential_slope = slope - direction * dot(slope, direction);
    return normalize(normal - tangential_slope * terrain_detail_steep_fade(normal, direction));
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

/// Depth no longer adds a scalar phase offset: that made equal-depth contours
/// into artificial circular wave sources around a coast. Depth remains in the
/// amplitude/steepness path.
fn shoaling_phase_offset_meters(water_depth_meters: f32) -> f32 {
    return 0.0;
}

// Spawn coast prototype (default on, optional opt-out). Blend fields, not phases: no depth contours,
// camera-relative anchors, tile ownership, or unbounded time-dependent slopes.
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
    if !SPAWN_COAST_ENABLED || OCEAN_WAVE_PHASE_SPEED_SIGN * sign(speed_meters_per_second) * dot(normalize(wave_axis), SPAWN_COAST_ONSHORE) <= 0.0 {
        return gerstner_wave_unsteered(direction, wave_axis, wavelength_meters,
            amplitude_meters, speed_meters_per_second, steepness, time_seconds, water_depth_meters);
    }
    let delta = direction - normalize(SPAWN_COAST_CENTER);
    let span = SPAWN_COAST_OUTER * SPAWN_COAST_OUTER - SPAWN_COAST_INNER * SPAWN_COAST_INNER;
    let t = clamp((dot(delta, delta) - SPAWN_COAST_INNER * SPAWN_COAST_INNER) / span, 0.0, 1.0);
    let weight = 1.0 - t * t * (3.0 - 2.0 * t);
    if weight <= 0.0 {
        return gerstner_wave_unsteered(direction, wave_axis, wavelength_meters,
            amplitude_meters, speed_meters_per_second, steepness, time_seconds, water_depth_meters);
    }
    let incoming = gerstner_wave_unsteered(direction, wave_axis, wavelength_meters,
        amplitude_meters, speed_meters_per_second, steepness, -time_seconds, water_depth_meters);
    if weight >= 1.0 {
        return incoming;
    }
    let original = gerstner_wave_unsteered(direction, wave_axis, wavelength_meters,
        amplitude_meters, speed_meters_per_second, steepness, time_seconds, water_depth_meters);
    let gradient = delta * (-12.0 * t * (1.0 - t) / (PLANET_RADIUS_METERS * span));
    let tangent_gradient = gradient - direction * dot(gradient, direction);
    return OceanWaveContribution(
        mix(original.horizontal_displacement, incoming.horizontal_displacement, weight),
        (original.horizontal_derivative * (1.0 - weight) + incoming.horizontal_derivative * weight)
            + ocean_outer_product(
                incoming.horizontal_displacement - original.horizontal_displacement,
                tangent_gradient,
            ),
        mix(original.vertical_displacement, incoming.vertical_displacement, weight),
        mix(original.slope, incoming.slope, weight)
            + tangent_gradient * (incoming.vertical_displacement - original.vertical_displacement),
        mix(original.convergence, incoming.convergence, weight),
    );
}

fn gerstner_wave_unsteered(
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
        return OceanWaveContribution(vec3<f32>(0.0), mat3x3<f32>(), 0.0, vec3<f32>(0.0), 0.0);
    }
    let tangent = tangent_unnormalized / tangent_length;
    let wave_number = 6.2831853 / wavelength_meters;
    let phase = wave_number
        * (dot(direction, axis) * PLANET_RADIUS_METERS
            + OCEAN_WAVE_PHASE_SPEED_SIGN * speed_meters_per_second * time_seconds
            + shoaling_phase_offset_meters(water_depth_meters));
    // Same bounded, zero-mean radial crest profile as CPU buoyancy.
    let sine = sin(phase);
    let cosine = cos(phase);
    // u^k on the wave's rise: a narrow crest over a long shallow trough, and
    // monotonic in u so it keeps exactly two extrema however sharp it gets.
    // Mirrors `wave_profile` in ocean.rs, which owns both constants.
    let rise = 0.5 * (1.0 + sine);
    let normalization = 1.0 / (1.0 - OCEAN_CREST_MEAN);
    let profile = (pow(rise, OCEAN_CREST_EXPONENT) - OCEAN_CREST_MEAN) * normalization;
    let profile_derivative = OCEAN_CREST_EXPONENT
        * pow(rise, OCEAN_CREST_EXPONENT - 1.0)
        * 0.5
        * cosine
        * normalization;
    let horizontal_scale = steepness * OCEAN_STEEPNESS_SCALE * amplitude_meters;
    var horizontal_derivative = mat3x3<f32>();
    var horizontal_displacement = vec3<f32>(0.0);
    if OCEAN_TRANSPORT_ENABLED && wavelength_meters >= 100.0 && wavelength_meters <= 200.0 {
        let tangent_derivative = ocean_outer_product(direction, tangent_unnormalized) * -1.0
            - (mat3x3<f32>(
                vec3<f32>(1.0, 0.0, 0.0) - direction * direction.x,
                vec3<f32>(0.0, 1.0, 0.0) - direction * direction.y,
                vec3<f32>(0.0, 0.0, 1.0) - direction * direction.z,
            )) * dot(direction, axis);
        horizontal_derivative =
            (tangent_derivative * cosine
                - ocean_outer_product(tangent_unnormalized, tangent_unnormalized)
                    * (wave_number * sine)) * horizontal_scale;
        horizontal_displacement = tangent_unnormalized * (horizontal_scale * cosine);
    }
    return OceanWaveContribution(
        horizontal_displacement,
        horizontal_derivative,
        amplitude_meters * profile,
        // d(dot(direction, axis) * R)/ds is the projected axis, not its
        // unit tangent. Normalizing it exaggerated slopes near an axis pole.
        tangent_unnormalized * (amplitude_meters * wave_number * profile_derivative),
        // `steepness_scaled * amplitude * wave_number` is this wave's
        // dimensionless steepness; `sin(phase)` is +1 at the crest, which is
        // also where the drawn profile peaks, so the product is largest
        // exactly on the sharp water. `tangent_length` is the same projection
        // factor the slope uses; without it a wave near its own axis pole
        // would claim a sharpness it does not have.
        steepness * OCEAN_STEEPNESS_SCALE * amplitude_meters * wave_number
            * tangent_length * sine,
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
        return OceanWaveContribution(vec3<f32>(0.0), mat3x3<f32>(), 0.0, vec3<f32>(0.0), 0.0);
    }
    // These shorter waves are part of the local geometry as well as its normal:
    // the CPU surface query mirrors their vertical displacement at the patch
    // centre, so nearby camera buoyancy cannot drift from the visible water.
    let first = gerstner_wave(direction, OCEAN_RIPPLE_FIRST_AXIS, 180.0, OCEAN_RIPPLE_FIRST_AMPLITUDE * OCEAN_WIND_RIPPLE_WEIGHTS.x, 16.7613 * OCEAN_WIND_RIPPLE_SIGNS.x, 0.0, time_seconds, water_depth_meters);
    let second = gerstner_wave(direction, OCEAN_RIPPLE_SECOND_AXIS, 70.0, OCEAN_RIPPLE_SECOND_AMPLITUDE * OCEAN_WIND_RIPPLE_WEIGHTS.y, 10.4525 * OCEAN_WIND_RIPPLE_SIGNS.y, 0.0, time_seconds, water_depth_meters);
    let third = gerstner_wave(direction, OCEAN_RIPPLE_THIRD_AXIS, 28.0, OCEAN_RIPPLE_THIRD_AMPLITUDE * OCEAN_WIND_RIPPLE_WEIGHTS.z, 6.6107 * OCEAN_WIND_RIPPLE_SIGNS.z, 0.0, time_seconds, water_depth_meters);
    let weight = distance_weight * shore_weight;
    return OceanWaveContribution(
        vec3<f32>(0.0),
        mat3x3<f32>(),
        (first.vertical_displacement + second.vertical_displacement + third.vertical_displacement) * weight,
        (first.slope + second.slope + third.slope) * weight,
        // The ripple layer is authored at zero steepness, so it pinches
        // nothing and contributes no sharpness.
        0.0,
    );
}

/// Water colour at the shore, from the depth beneath it.
///
/// `still_depth_meters` is the sea bed below mean sea level; `surface_height_meters`
/// is the wave's displacement at this point. Their sum is the water actually
/// standing here at this instant, which is what decides both how shallow it
/// reads and whether it is breaking.
/// Rise per metre of horizontal travel, from a surface normal. `tan` of the
/// tilt, so it is the same quantity `global_wave_slope` reports on the CPU and
/// the whitecap thresholds are stated in.
fn ocean_surface_slope(normal: vec3<f32>, up: vec3<f32>) -> f32 {
    let facing = clamp(dot(normalize(normal), normalize(up)), 1.0e-3, 1.0);
    return sqrt(max(1.0 - facing * facing, 0.0)) / facing;
}

fn ocean_history_coverage(up: vec3<f32>) -> f32 {
    if !OCEAN_FOAM_HISTORY_ENABLED {
        return 0.0;
    }
    let center = normalize(view_to_planet(camera.camera_planet_direction_view_altitude.xyz));
    let east = normalize(camera.camera_right.xyz
        - center * dot(camera.camera_right.xyz, center));
    let north = cross(center, east);
    let offset = (up - center) * PLANET_RADIUS_METERS;
    let uv = vec2<f32>(dot(offset, east), dot(offset, north)) / 512.0 + 0.5;
    if any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
        return 0.0;
    }
    let edge = min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y));
    return textureSampleLevel(foam_history_map, terrain_material_sampler, uv, 0.0).r
        * smoothstep(0.0, 0.04, edge);
}

/// How much of this patch of sea is white, from both causes: surf, which needs
/// a bottom to break on, and whitecaps, which do not.
fn ocean_foam_coverage(
    still_depth_meters: f32,
    surface_height_meters: f32,
    breaking_ratio: f32,
    normal: vec3<f32>,
    up: vec3<f32>,
) -> f32 {
    // Surf is a band, not a field. Once a crest is many times what the depth can
    // hold it broke a long way back and the water behind it is spent, so the
    // foam has to fade out again -- otherwise the whole shelf whitens.
    let crest_foam =
        smoothstep(OCEAN_BREAKING_FOAM_ONSET, OCEAN_BREAKING_FOAM_FULL, breaking_ratio)
            * (1.0 - smoothstep(OCEAN_BREAKING_FOAM_SPENT, OCEAN_BREAKING_FOAM_GONE, breaking_ratio));
    // And the wash right at the edge, where there is barely any water left.
    let column_meters = still_depth_meters + surface_height_meters;
    let wash = 1.0 - smoothstep(0.0, OCEAN_SURF_COLUMN_METERS, max(column_meters, 0.0));
    let surf = max(crest_foam, wash * wash);
    let surface_slope = ocean_surface_slope(normal, up);
    let whitecap = smoothstep(
        OCEAN_WHITECAP_SLOPE_ONSET,
        OCEAN_WHITECAP_SLOPE_FULL,
        surface_slope,
    ) * smoothstep(
        OCEAN_WHITECAP_CREST_LOW_FRACTION * OCEAN_MAXIMUM_WAVE_HEIGHT_METERS,
        OCEAN_WHITECAP_CREST_HIGH_FRACTION * OCEAN_MAXIMUM_WAVE_HEIGHT_METERS,
        surface_height_meters,
    );
    // Foam has to be made of water. Without this it keys off a depth of zero
    // and whitens ground the sea is barely covering.
    let has_water = smoothstep(0.0, OCEAN_FOAM_MINIMUM_DEPTH_METERS, still_depth_meters);
    let lingering_foam = ocean_history_coverage(up)
        * smoothstep(0.15, 0.40, surface_slope) * 0.7;
    return max(max(surf, whitecap), lingering_foam)
        * has_water * OCEAN_BREAKING_FOAM_MAX;
}

/// Foam lit the way the water beside it is lit, so it darkens at dusk instead
/// of staying a painted white. Same diffuse form as `ocean_lighting`, with
/// foam's albedo in place of the water body's.
fn ocean_foam_radiance(sun_transmittance: vec3<f32>, sky_diffuse: vec3<f32>) -> vec3<f32> {
    return OCEAN_SURF_COLOUR * (sky_diffuse + sun_transmittance * (0.4 * SURFACE_SUNLIGHT_SCALE));
}

fn shoreline_water_albedo(open_water: vec3<f32>, still_depth_meters: f32, foam: f32) -> vec3<f32> {
    let shallow = 1.0 - smoothstep(0.0, OCEAN_SHALLOW_DEPTH_METERS, still_depth_meters);
    // Squared so the shallows stay tight to the beach rather than washing the
    // whole bay out.
    let albedo = mix(open_water, OCEAN_SHALLOW_COLOUR, shallow * shallow);
    return mix(albedo, OCEAN_SURF_COLOUR, foam);
}

// (height, dh/du, dh/dv, div D) for one cascade at planet-plane offset `local`.
fn ocean_fft_cascade(cascade_index: u32, local: vec2<f32>, filter_width_meters: f32) -> vec4<f32> {
    let entry = ocean_fft_view.cascade[cascade_index];
    let uv = entry.xy + local / entry.z;
    let texel_meters = entry.z / 256.0;
    // Box-filtering to 2^lod texels removes waves shorter than ~2x that width,
    // which is what a mesh (or pixel) this coarse cannot represent.
    let lod = clamp(log2(max(filter_width_meters / texel_meters, 1.0)), 0.0, 8.0);
    let texel = exp2(lod) / 256.0;
    let step_meters = entry.z * texel;
    let s0 = textureSampleLevel(ocean_fft_map, ocean_fft_sampler, uv, cascade_index, lod);
    let su = textureSampleLevel(ocean_fft_map, ocean_fft_sampler, uv + vec2<f32>(texel, 0.0), cascade_index, lod);
    let sv = textureSampleLevel(ocean_fft_map, ocean_fft_sampler, uv + vec2<f32>(0.0, texel), cascade_index, lod);
    return vec4<f32>(
        s0.x,
        (su.x - s0.x) / step_meters,
        (sv.x - s0.x) / step_meters,
        ((su.y - s0.y) + (sv.z - s0.z)) / step_meters,
    );
}

fn ocean_surface_fft(
    direction: vec3<f32>,
    camera_distance_meters: f32,
    water_depth_meters: f32,
) -> OceanSurface {
    let geometry_weight = 1.0 - smoothstep(
        OCEAN_GEOMETRY_FULL_DISTANCE_METERS,
        OCEAN_GEOMETRY_FADE_DISTANCE_METERS,
        camera_distance_meters,
    );
    let view = ocean_fft_view_position;
    let planet_offset = view.x * camera.camera_right.xyz
        + view.y * camera.camera_up.xyz
        - view.z * camera.camera_forward.xyz;
    let axis_u = ocean_fft_view.axis_u.xyz;
    let axis_v = ocean_fft_view.axis_v.xyz;
    let local = vec2<f32>(dot(planet_offset, axis_u), dot(planet_offset, axis_v));
    let gain = ocean_fft_view.gain.x;
    // Cascade 0 (wavelengths above ~12m) is mesh geometry; the two finer
    // cascades only shade, fading out before they alias at range.
    // Filter width: twice the vertex spacing in the vertex stage, otherwise
    // twice the pixel footprint (distance times the angle one pixel spans).
    let pixel_footprint = camera_distance_meters * (2.0 * camera.projection.y / 720.0);
    let filter_width = 2.0 * select(
        pixel_footprint,
        max(ocean_fft_vertex_spacing_meters, 0.0),
        ocean_fft_vertex_spacing_meters > 0.0,
    );
    let broad = ocean_fft_cascade(0u, local, filter_width);
    let mid_weight = 1.0 - smoothstep(600.0, 3000.0, camera_distance_meters);
    let fine_weight = 1.0 - smoothstep(150.0, 700.0, camera_distance_meters);
    let mid = ocean_fft_cascade(1u, local, filter_width);
    let fine = ocean_fft_cascade(2u, local, filter_width);
    let tangent_slope = (axis_u * broad.y + axis_v * broad.z) * gain;
    let slope = tangent_slope - direction * dot(tangent_slope, direction);
    let ripple_tangent = (axis_u * (mid.y * mid_weight + fine.y * fine_weight)
        + axis_v * (mid.z * mid_weight + fine.z * fine_weight)) * gain;
    let ripple_slope = ripple_tangent - direction * dot(ripple_tangent, direction);
    let raw_vertical = broad.x * gain * geometry_weight;
    let breaking_limit_meters =
        0.5 * OCEAN_BREAKING_HEIGHT_TO_DEPTH_RATIO * max(water_depth_meters, 0.0);
    var breaking_weight = 0.0;
    var breaking_slope_weight = 0.0;
    var breaking_ratio = 0.0;
    if breaking_limit_meters > 0.0 {
        let ratio = pow(abs(raw_vertical) / breaking_limit_meters, OCEAN_BREAKING_KNEE);
        breaking_weight = pow(1.0 + ratio, -1.0 / OCEAN_BREAKING_KNEE);
        breaking_slope_weight = breaking_weight / (1.0 + ratio);
        breaking_ratio = max(raw_vertical, 0.0) / breaking_limit_meters;
    }
    let convergence = -(broad.w + mid.w * mid_weight) * gain * geometry_weight;
    return OceanSurface(
        breaking_ratio,
        vec3<f32>(0.0),
        mat3x3<f32>(),
        raw_vertical * breaking_weight,
        slope * (geometry_weight * breaking_slope_weight),
        normalize(direction - slope * (geometry_weight * breaking_slope_weight)),
        (mid.x * mid_weight + fine.x * fine_weight) * gain,
        ripple_slope,
        convergence,
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
    if OCEAN_FFT_ENABLED {
        return ocean_surface_fft(direction, camera_distance_meters, water_depth_meters);
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
    // Crests here are small circles about each axis, not straight lines. The
    // dominant swell stays coherent; the shorter wind-sea tail is deliberately
    // spread around the storm-ocean view direction so it breaks the surface into
    // crossing chop instead of repeating one corduroy axis across many octaves.
    let storm_intensity = clamp(camera.flat_triangle_options.y, 0.0, 1.0);
    let storm_blend = smoothstep(0.15, 0.85, storm_intensity);
    var horizontal = vec3<f32>(0.0);
    var horizontal_derivative = mat3x3<f32>();
    var vertical = 0.0;
    var slope = vec3<f32>(0.0);
    var convergence = 0.0;
    for (var i = 0u; i < OCEAN_WAVE_COUNT; i = i + 1u) {
        let spec = OCEAN_WAVE_TABLE[i];
        var amplitude = mix(spec.amplitude_meters, spec.storm_amplitude_meters, storm_blend);
        if OCEAN_WIND_ENABLED {
            amplitude *= OCEAN_WIND_WEIGHTS[i];
        }
        if OCEAN_LARGE_SWELL_ONLY && i >= OCEAN_LARGE_SWELL_WAVE_COUNT {
            amplitude = 0.0;
        }
        let contribution = gerstner_wave(
            direction,
            spec.axis,
            spec.wavelength_meters,
            amplitude,
            spec.speed_meters_per_second * OCEAN_WIND_SPEED_SIGNS[i],
            spec.steepness,
            time_seconds,
            water_depth_meters,
        );
        if OCEAN_TRANSPORT_ENABLED
            && spec.wavelength_meters >= 100.0
            && spec.wavelength_meters <= 200.0
        {
            horizontal += contribution.horizontal_displacement;
            horizontal_derivative += contribution.horizontal_derivative;
        }
        vertical += contribution.vertical_displacement;
        slope += contribution.slope;
        convergence += contribution.convergence;
    }
    var ripple = ocean_ripple(
        direction,
        time_seconds,
        camera_distance_meters,
        shore_weight,
        water_depth_meters,
    );
    if OCEAN_LARGE_SWELL_ONLY {
        // The ripple layer is a shorter octave by definition, so the large
        // swell diagnostic drops it whatever the camera is doing.
        ripple = OceanWaveContribution(vec3<f32>(0.0), mat3x3<f32>(), 0.0, vec3<f32>(0.0), 0.0);
    }
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
    var breaking_slope_weight = 0.0;
    if breaking_limit_meters > 0.0 {
        // Soft-max knee, paired with `breaking_weight` in ocean.rs: a crest
        // well under what the depth holds is left alone, and only bends as it
        // approaches. A tanh here bit everywhere and put the camera on a
        // shorter sea than the one being drawn.
        let ratio = pow(abs(raw_vertical) / breaking_limit_meters, OCEAN_BREAKING_KNEE);
        breaking_weight = pow(1.0 + ratio, -1.0 / OCEAN_BREAKING_KNEE);
        // Differentiate the limited height, not just its raw wave. Paired
        // with ocean.rs::breaking_rate_weight; reuse the existing pow.
        breaking_slope_weight = breaking_weight / (1.0 + ratio);
    }
    let limited = geometry_weight * geometry_amplitude_scale * breaking_weight;
    let limited_slope = geometry_weight * geometry_amplitude_scale * breaking_slope_weight;
    let transport_weight = select(1.0, 0.0, !OCEAN_TRANSPORT_ENABLED)
        * smoothstep(30.0, 100.0, water_depth_meters);
    let transport_limited = geometry_weight * geometry_amplitude_scale;
    let transport_jacobian = horizontal_derivative
        * (transport_limited * OCEAN_TRANSPORT_GAIN * transport_weight);
    var geometric_normal = normalize(direction - slope * limited_slope);
    if transport_weight > 0.0 {
        let reference_axis = select(
            vec3<f32>(1.0, 0.0, 0.0),
            vec3<f32>(0.0, 1.0, 0.0),
            abs(direction.x) > 0.9,
        );
        let tangent_u = normalize(cross(direction, reference_axis));
        let tangent_v = cross(direction, tangent_u);
        let jacobian_u = tangent_u * (1.0 + vertical * limited / PLANET_RADIUS_METERS)
            + direction * dot(slope * limited_slope, tangent_u)
            + transport_jacobian * tangent_u;
        let jacobian_v = tangent_v * (1.0 + vertical * limited / PLANET_RADIUS_METERS)
            + direction * dot(slope * limited_slope, tangent_v)
            + transport_jacobian * tangent_v;
        geometric_normal = normalize(cross(jacobian_u, jacobian_v));
        if dot(geometric_normal, direction) < 0.0 {
            geometric_normal = -geometric_normal;
        }
    }
    // Zero depth is no water at all, not an infinitely broken wave. Calling it
    // the latter painted every flat where the bake carries no bathymetry as
    // solid foam, which is most of a gently shelving coast.
    var breaking_ratio = 0.0;
    if breaking_limit_meters > 0.0 {
        breaking_ratio = max(raw_vertical, 0.0) / breaking_limit_meters;
    }
    return OceanSurface(
        breaking_ratio,
        horizontal * transport_limited * OCEAN_TRANSPORT_GAIN * transport_weight,
        transport_jacobian,
        vertical * limited,
        slope * limited_slope,
        // Geometry and CPU buoyancy share this broad normal. The already-paid
        // sub-mesh ripple slope stays separate for fragment lighting so it can
        // sharpen the smallest visible waves without moving the mesh or eye.
        geometric_normal,
        ripple.vertical_displacement,
        ripple.slope,
        convergence * geometry_weight * geometry_amplitude_scale,
    );
}

fn ocean_shading_normal(surface: OceanSurface) -> vec3<f32> {
    return normalize(surface.normal - surface.ripple_slope);
}

/// Convert a world radial (used by ray queries) back to the parameter direction
/// whose transported surface reaches it. Raster vertices already carry that
/// parameter direction and must continue to call `ocean_surface` directly.
fn ocean_surface_world_direction(
    world_direction: vec3<f32>,
    time_seconds: f32,
    camera_distance_meters: f32,
    water_depth_meters: f32,
) -> OceanSurface {
    let target_direction = normalize(world_direction);
    if !OCEAN_TRANSPORT_ENABLED {
        return ocean_surface(target_direction, time_seconds, camera_distance_meters, water_depth_meters);
    }
    let target_direction_axis = select(
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, 1.0, 0.0),
        abs(target_direction.x) > 0.9,
    );
    let target_direction_u = normalize(cross(target_direction, target_direction_axis));
    let target_direction_v = cross(target_direction, target_direction_u);
    var parameter = target_direction;
    for (var iteration = 0u; iteration < 8u; iteration += 1u) {
        let surface = ocean_surface(parameter, time_seconds, camera_distance_meters, water_depth_meters);
        let parameter_axis = select(
            vec3<f32>(1.0, 0.0, 0.0),
            vec3<f32>(0.0, 1.0, 0.0),
            abs(parameter.x) > 0.9,
        );
        let parameter_u = normalize(cross(parameter, parameter_axis));
        let parameter_v = cross(parameter, parameter_u);
        let gradient = surface.slope;
        let derivative_u = parameter_u * (1.0 + surface.vertical_displacement / PLANET_RADIUS_METERS)
            + parameter * dot(gradient, parameter_u)
            + surface.horizontal_derivative * parameter_u;
        let derivative_v = parameter_v * (1.0 + surface.vertical_displacement / PLANET_RADIUS_METERS)
            + parameter * dot(gradient, parameter_v)
            + surface.horizontal_derivative * parameter_v;
        let position = parameter * (PLANET_RADIUS_METERS + surface.vertical_displacement)
            + surface.horizontal_displacement;
        let radial_residual = position - target_direction * dot(position, target_direction);
        let a = dot(derivative_u, target_direction_u);
        let b = dot(derivative_v, target_direction_u);
        let c = dot(derivative_u, target_direction_v);
        let d = dot(derivative_v, target_direction_v);
        let determinant = a * d - b * c;
        if abs(determinant) < 1.0e-4 {
            break;
        }
        let rhs_u = dot(radial_residual, target_direction_u);
        let rhs_v = dot(radial_residual, target_direction_v);
        let step_u = (d * rhs_u - b * rhs_v) / determinant;
        let step_v = (a * rhs_v - c * rhs_u) / determinant;
        parameter = normalize(parameter - (target_direction_u * step_u + target_direction_v * step_v) / PLANET_RADIUS_METERS);
        if abs(step_u) + abs(step_v) < 0.0001 {
            break;
        }
    }
    return ocean_surface(parameter, time_seconds, camera_distance_meters, water_depth_meters);
}

fn ocean_fine_crest_transmission(surface: OceanSurface) -> f32 {
    // The normal-only ripple octave has no Gerstner convergence because it
    // deliberately does not transport geometry. Select its thin upper faces
    // from quantities it does carry: positive crest height and a steep local
    // slope. Squaring both ramps confines transmission to small highlights
    // instead of tinting the whole wave field.
    let upper_face = smoothstep(0.0, 1.2, surface.ripple_height);
    let steep_face = smoothstep(0.12, 0.32, length(surface.ripple_slope));
    return upper_face * upper_face * steep_face * steep_face;
}

fn flat_ocean_surface(direction: vec3<f32>) -> OceanSurface {
    return OceanSurface(
        0.0,
        vec3<f32>(0.0),
        mat3x3<f32>(),
        0.0,
        vec3<f32>(0.0),
        normalize(direction),
        0.0,
        vec3<f32>(0.0),
        0.0,
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
    // Vacuum has no optical column: sunlight arrives the same colour at every
    // angle, so an airless body has no warm terminator. The geometric horizon
    // still occludes it, which is what keeps the day/night line sharp.
    if !BODY_HAS_ATMOSPHERE {
        return vec3<f32>(1.0) * visibility;
    }
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
    // No sky, no skylight. Shadows on an airless body are lit by nothing at
    // all, which is why their contrast is so violent.
    if !BODY_HAS_ATMOSPHERE {
        return vec3<f32>(0.0);
    }
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
    // The LUT integrates a horizontal Lambertian receiver. A tilted facet sees
    // only (1 + cos tilt) / 2 of the sky dome, so a cliff gets half the fill of
    // flat ground rather than most of it.
    let sky_view = 0.5 + 0.5 * clamp(dot(normalize(normal), surface_direction), -1.0, 1.0);
    // The sky display applies one fixed, hue-preserving perceptual curve so
    // nautical twilight remains visible without auto exposure. Apply that
    // same presentation to E/pi before it lights the surface; otherwise the
    // visible blue sky would illuminate the terrain with its much smaller raw
    // radiometric value and appear disconnected from it.
    return perceptual_physical_sky_radiance(max(horizontal_diffuse, vec3<f32>(0.0)))
        * sky_view
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

// Water medium chosen by the caller: an ocean back face establishes this
// per pixel even if the CPU eye-height query still says "above water".
fn ocean_depth_extinction_weight(water_depth_meters: f32) -> f32 {
    return mix(
        0.15,
        1.0,
        smoothstep(2.0, 30.0, max(water_depth_meters, 0.0)),
    );
}

// Altitude of a camera-relative view-space point, for the bounded local
// distances the water paths work over.
//
// Expanded about the camera -- its own altitude, the point's rise along the
// radial, and the curvature drop of the tangent plane -- so every term stays at
// metre scale rather than cancelling two values near the planet radius.
//
// Do not oversell that. This radius is 4.0e6, where one f32 ulp is 0.25m, but
// measured over realistic local offsets the direct `altitude_along_ray` form is
// only 1.3mm out in strict binary32, and this GPU does better still: both forms
// pass the GPU test to under a centimetre. Since the depth this feeds is used by
// smoothstep(2, 30), anything under a decimetre is invisible. So this is a
// tidiness choice, not a fix -- it just keeps the arithmetic local. Truncation
// of the expansion is order distance^3 / radius^2, measured at 1e-6 m.
//
// `camera_planet_direction_view_altitude.xyz` is already a unit radial in view
// space -- it is built by `world_to_view` on the Rust side -- so it is dotted
// with a view-space vector directly, as `terrain_fog_air_path_meters` does.
fn local_view_altitude_meters(camera_relative_view_position: vec3<f32>) -> f32 {
    let camera_altitude_meters = camera.camera_planet_direction_view_altitude.w;
    let rise_meters = dot(
        camera.camera_planet_direction_view_altitude.xyz,
        camera_relative_view_position,
    );
    let horizontal_squared = max(
        dot(camera_relative_view_position, camera_relative_view_position)
            - rise_meters * rise_meters,
        0.0,
    );
    return camera_altitude_meters
        + rise_meters
        + horizontal_squared
            / (2.0 * (PLANET_RADIUS_METERS + camera_altitude_meters));
}

// Only the *amount* depends on depth; the colour below does not. Split out so
// the depth rule can be tested without binding the sky LUT, and so callers that
// invert an already-applied fog can reason about the one term that differs.
fn ocean_water_fog_amount(
    camera_relative_view_position: vec3<f32>,
    water_depth_meters: f32,
) -> f32 {
    let e_fold_meters = OCEAN_UNDERWATER_VISIBILITY_METERS / log(50.0);
    // A shallow shelf is continually relit by the nearby bright bed. Let its
    // long horizontal views retain that sand/turquoise contribution instead
    // of converging on the same dark blue as deep water. Below 2m the water is
    // nearly clear; by 30m it uses the full 100m visibility extinction.
    let depth_weight = ocean_depth_extinction_weight(water_depth_meters);
    return 1.0 - exp(
        -length(camera_relative_view_position) * depth_weight / e_fold_meters,
    );
}

fn ocean_water_fog_at_depth(
    camera_relative_view_position: vec3<f32>,
    water_depth_meters: f32,
) -> TerrainFog {
    let amount = ocean_water_fog_amount(
        camera_relative_view_position,
        water_depth_meters,
    );
    // Lit by the sky overhead rather than painted: the water goes dark at
    // night and at depth, because what reaches the eye is daylight that got
    // down here and then scattered off the water.
    let up_view = normalize(
        planet_to_view(camera.camera_planet_direction_view_altitude.xyz),
    );
    return TerrainFog(amount, physical_camera_sky_radiance(up_view) * OCEAN_UNDERWATER_TINT);
}

fn ocean_water_fog(camera_relative_view_position: vec3<f32>) -> TerrainFog {
    return ocean_water_fog_at_depth(
        camera_relative_view_position,
        camera.camera_forward.w,
    );
}

fn ocean_distance_fog(
    surface_color: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
) -> vec3<f32> {
    let fog = ocean_water_fog(camera_relative_view_position);
    return mix(surface_color, fog.color, fog.amount);
}

fn ocean_depth_aware_distance_fog(
    surface_color: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    water_depth_meters: f32,
) -> vec3<f32> {
    let fog = ocean_water_fog_at_depth(
        camera_relative_view_position,
        water_depth_meters,
    );
    return mix(surface_color, fog.color, fog.amount);
}

fn terrain_fog(
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> TerrainFog {
    // Distance mist is air. There is none here, and the colour it mixes toward
    // is `physical_camera_sky_radiance` -- Rayleigh blue -- so on an airless
    // body this laid a blue haze over everything far away, including the
    // unlit side, which is how a night side that computes to exactly zero
    // surface lighting still came out at (4, 6, 15) instead of black. Reported
    // as the moon not going straight to black at the edge of the light.
    //
    // The aerial-perspective terms next to this were gated when the moon was
    // built; this one was missed because it is composed later, as presentation
    // rather than as physics.
    // A submerged eye is not looking through air. Water extinguishes over
    // metres where air takes kilometres, so the atmosphere's path integral is
    // not merely the wrong amount here, it is the wrong medium: without this
    // the sea bed and the coast beyond it were drawn at full contrast through
    // any depth of water at all.
    if camera.flat_triangle_options.w > 0.5 {
        return ocean_water_fog_at_depth(
            camera_relative_view_position,
            max(-surface_altitude_meters, 0.0),
        );
    }
    if !BODY_HAS_ATMOSPHERE {
        return TerrainFog(0.0, vec3<f32>(0.0));
    }
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
        case 8u: { display_color = vec3<f32>(74.0, 70.0, 66.0) / 255.0; }
        // Medial moraine: debris riding on ice, close to mountain rock so the
        // stripe reads as the same material laid over the glacier.
        case 10u: { display_color = vec3<f32>(92.0, 86.0, 78.0) / 255.0; }
        // Crevasse field: darker and bluer than clean firn, because most of
        // what the eye catches at distance is the inside of the cracks.
        case 11u: { display_color = vec3<f32>(176.0, 198.0, 214.0) / 255.0; }
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
    // Under a snow biome's snow is mountain rock, not soil or grass. A moraine
    // is debris lying on the ice, so it is rock all the way down and never
    // takes the vegetation or earth layers.
    if biome == 8u || biome == 10u || terrain_material_is_snow(biome) {
        rock_amount = max(rock_amount, 0.78);
    }
    if biome == 10u {
        rock_amount = 1.0;
    }

    let latitude_amount = abs(surface_direction.y);
    let snowline_meters = mix(6200.0, 2200.0, latitude_amount);
    var snow_amount = smoothstep(
        snowline_meters,
        snowline_meters + 900.0,
        macro_height_meters,
    );
    if biome == 2u {
        snow_amount = 1.0;
    } else if biome == 11u {
        // Broken ice still reads mostly white from any distance; the cracks
        // darken it rather than uncovering it.
        snow_amount = 0.82;
    } else if biome == 9u {
        snow_amount = max(snow_amount, 0.88);
    } else if biome == 10u {
        // Debris cover, not snow.
        snow_amount = 0.0;
    }
    // After the biome overrides, or a snow biome paints its cliffs white.
    snow_amount *= snow_slope_hold(slope, biome);

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
    // Every water surface in both passes reads its colour here, so the body's
    // tint belongs at this one point rather than at each call site.
    return BODY_WATER_TINT * vec3<f32>(0.008, 0.055, 0.28);
}

fn is_open_ocean_surface(outmap: bool, macro_height_meters: f32, biome_id: u32) -> bool {
    // A body with no sea has nothing to hand these fragments to. The terrain
    // pass discards what it believes the analytic shell owns, so without this
    // an airless body loses every fragment at or below its datum -- which on a
    // cratered moon is all of it except the raised rims -- to a draw that is
    // switched off, and the clear colour shows through.
    if !BODY_HAS_OCEAN {
        return false;
    }
    let ice = outmap && biome_id == 2u;
    let lake = outmap && biome_id == 1u;
    return macro_height_meters <= 0.0 && !ice && !lake;
}

fn outmap_ocean_coverage(outmap: bool, height_meters: f32) -> f32 {
    // Same ownership rule as `is_open_ocean_surface`: with no sea to blend
    // toward, sub-datum ground is ordinary ground.
    if !BODY_HAS_OCEAN {
        return 0.0;
    }
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

/// Snow sheds from faces steeper than it can hold, so those show bare rock.
/// `slope` is `1 - cos(angle)`: 0.134 is 30 degrees, 0.293 is 45. Glacier ice
/// clings a little steeper. A small remainder keeps snow lodged in gullies.
fn snow_slope_hold(slope: f32, biome: u32) -> f32 {
    // Measured on the summit survey: with the old bands only 17.5% of visible
    // ground shed its snow, against 78% of raw albedo reading near-white, so
    // faces the photographs show as bare rock stayed white. The geometry was
    // never the problem -- half the frame sits past 30 degrees -- the bands
    // simply started too late. Glacier ice still clings steeper than snow.
    // `1 - cos`: 0.067 is 21 degrees, 0.087 is 24, 0.22 is 38, 0.26 is 41.
    let shed = select(
        smoothstep(0.067, 0.220, slope),
        smoothstep(0.087, 0.260, slope),
        biome == 2u,
    );
    return 1.0 - shed * 0.96;
}

fn terrain_material_is_snow(biome_id: u32) -> bool {
    // A crevasse field is ice that has pulled apart, so it still holds snow and
    // sheds it on the same slopes. A moraine is debris lying on top of the ice
    // and behaves as rock, so it is deliberately excluded here.
    return biome_id == 2u || biome_id == 9u || biome_id == 11u;
}

/// Share of a biome blend owned by one biome. Colour decisions weighted by
/// this fade across a texel border; switching on the nearest biome drew every
/// patch edge as a staircase of texels.
fn biome_blend_share(blend: BiomeBlendSample, wanted: u32) -> f32 {
    return dot(blend.weights, vec4<f32>(
        select(0.0, 1.0, blend.ids.x == wanted),
        select(0.0, 1.0, blend.ids.y == wanted),
        select(0.0, 1.0, blend.ids.z == wanted),
        select(0.0, 1.0, blend.ids.w == wanted),
    ));
}

fn biome_blend_snow_share(blend: BiomeBlendSample) -> f32 {
    // Must agree with `terrain_material_is_snow`: a crevasse field is ice that
    // pulled apart, so aerial neutrality and the ice light floor have to treat
    // it as snow. Disagreeing here is how white ground in a non-snow biome took
    // the raw orange long-path transmittance and stamped that biome's texel
    // staircase across it.
    return biome_blend_share(blend, 2u)
        + biome_blend_share(blend, 9u)
        + biome_blend_share(blend, 11u);
}

fn biome_blend_vegetation_share(blend: BiomeBlendSample) -> f32 {
    return biome_blend_share(blend, 4u) + biome_blend_share(blend, 5u) + biome_blend_share(blend, 6u);
}

/// How much a rendered (linear) albedo looks like snow: bright and nearly grey.
/// Aerial neutrality keys on this as well as the biome, because ground that
/// renders white in a non-snow biome otherwise takes the raw orange long-path
/// transmittance, stamped along that biome's texel edges.
fn albedo_snow_look(albedo: vec3<f32>) -> f32 {
    let luminance = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
    let brightest = max(max(albedo.x, albedo.y), albedo.z);
    let chroma = (brightest - min(min(albedo.x, albedo.y), albedo.z)) / max(brightest, 1.0e-4);
    return smoothstep(0.35, 0.60, luminance) * (1.0 - smoothstep(0.25, 0.45, chroma));
}

fn terrain_aerial_neutrality(blend: BiomeBlendSample, snow_look: f32) -> f32 {
    return min(
        0.82 * biome_blend_vegetation_share(blend)
            + 0.92 * max(biome_blend_snow_share(blend), snow_look),
        0.92,
    );
}

fn terrain_material_transmittance_blend(
    transmittance: vec3<f32>,
    blend: BiomeBlendSample,
    snow_look: f32,
) -> vec3<f32> {
    if !BODY_HAS_ATMOSPHERE {
        return vec3<f32>(1.0);
    }
    let neutrality = terrain_aerial_neutrality(blend, snow_look);
    let luminance = dot(transmittance, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(transmittance, vec3<f32>(luminance), neutrality);
}

fn terrain_material_in_scatter_blend(
    in_scatter: vec3<f32>,
    blend: BiomeBlendSample,
    snow_look: f32,
) -> vec3<f32> {
    if !BODY_HAS_ATMOSPHERE {
        return vec3<f32>(0.0);
    }
    let vegetation = biome_blend_vegetation_share(blend);
    let neutrality = terrain_aerial_neutrality(blend, snow_look);
    let luminance = dot(in_scatter, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(in_scatter, vec3<f32>(luminance), neutrality)
        * mix(1.0, VEGETATION_AERIAL_IN_SCATTER_SCALE, vegetation);
}

/// A crevasse field, shaded rather than painted.
///
/// The baker already says *where* these are: `crevasse_field` is 3.8% of the
/// texels on the alpine judging tile and 6.2% of the seventeen-texel window the
/// camera stands in. What it could not say is what one looks like, because its
/// texels are 3,906m apart, so the label was drawn as a slightly bluer white and
/// read as a tonal patch. This synthesises the cracks inside the labelled patch.
///
/// Two earlier attempts painted dark lines into the albedo and were both judged
/// as looking painted, which they were: a line that does not move when the sun
/// moves is a decal. This instead **tilts the normal** across each slot, so the
/// wall turned toward the sun brightens and the wall turned away darkens, and
/// the whole field inverts when the sun crosses it. The lighting already in the
/// shader does the work.
///
/// The bands follow **contours of surface height**, which is where transverse
/// crevasses actually run, and it costs nothing to know: the height is already
/// interpolated to this fragment. It also means their ground spacing falls out
/// of the slope for free -- tight where the ice steepens into an icefall, wide
/// apart on a flat basin -- instead of needing a separate flow model.
struct GlacierCrevasses {
    /// Normal tilt across the slot, tangential to the surface.
    wall_tilt: vec3<f32>,
    /// 0 on intact ice, 1 deep in a slot: drives occlusion and the blue.
    interior: f32,
    /// Share of the direct beam that still reaches the slot floor.
    ///
    /// This carries the effect, and tilting the walls alone does not. Measured:
    /// slots covered 1.6-4.9% of the frame and moved it by at most 5 levels of
    /// 255, because sunlit snow sits deep in the ACES shoulder where a 20%
    /// change in radiance is worth 0.02 of display. A crevasse is not a 20%
    /// change. Its floor sees the sun only when the sun is within a few degrees
    /// of the slot's own plane, and with sky fill at 0.5% of surface light here
    /// there is nothing else to light it -- so occluding the beam is what makes
    /// it read, and it is also what finally gives this picture a dark end.
    sun_visibility: f32,
}

fn glacier_crevasses(
    glacier_share: f32,
    surface_normal: vec3<f32>,
    surface_direction: vec3<f32>,
    macro_height_meters: f32,
    terrain_detail_meters: f32,
    anchor_direction: vec3<f32>,
    local_meters: vec3<f32>,
    camera_distance_meters: f32,
) -> GlacierCrevasses {
    let quiet = GlacierCrevasses(vec3<f32>(0.0), 0.0, 1.0);
    if glacier_share <= 0.0 {
        return quiet;
    }
    // Where on a glacier the ice is actually broken, which is a slope band.
    // Flat firn is intact, the ice cracks where it is pulled over a steepening,
    // and by the time a face is steep enough to be bare it is already shedding
    // its snow to `snow_slope_hold` and is rock, not a crevasse field. The band
    // is deliberately continuous with that rule rather than independent of it.
    // `1 - cos`: 0.018 is 10.9 degrees, 0.055 is 19, 0.120 is 28.4, 0.260 is 41.
    let slope = 1.0 - clamp(dot(normalize(surface_normal), surface_direction), 0.0, 1.0);
    let slope_gate = smoothstep(0.018, 0.055, slope)
        * (1.0 - smoothstep(0.120, 0.260, slope));
    // Below the spacing a band is thinner than a pixel and would only shimmer,
    // so it is faded out well before that rather than aliased.
    let range = 1.0 - smoothstep(
        CREVASSE_FADE_NEAR_METERS,
        CREVASSE_FADE_FAR_METERS,
        camera_distance_meters,
    );
    let strength = glacier_share * slope_gate * range;
    if strength <= 0.001 {
        return quiet;
    }
    // Contour bands of the *macro* surface. Using the drawn height instead put
    // the contours on the runtime detail ladder's own 475m wobble, which cycles
    // many times over a few metres of ground and ruled the glacier with
    // corduroy. The baked macro surface is the one the ice actually flows over.
    let cycle = macro_height_meters / CREVASSE_VERTICAL_SPACING_METERS;
    // A real field is broken into finite en-echelon segments, not one crack
    // wrapped round the mountain, so one noise sample offsets and gates the
    // bands along their own length.
    let cells = terrain_detail_domain(anchor_direction)
        * (PLANET_RADIUS_METERS / CREVASSE_SEGMENT_WAVELENGTH_METERS);
    let cell_floor = floor(cells);
    let segment = terrain_detail_value_noise(
        vec3<i32>(cell_floor),
        (cells - cell_floor)
            + terrain_detail_domain(local_meters) / CREVASSE_SEGMENT_WAVELENGTH_METERS,
    );
    // A field is a scatter of finite cracks, not a ruled contour map: most of
    // the glacier has none, and where they exist they step past one another.
    let present = smoothstep(0.24, 0.52, segment.value);
    if present <= 0.0 {
        return quiet;
    }
    // A second, finer field breaks each crack along its own length, or the
    // result is evenly spaced curved dashes -- brush strokes rather than broken
    // ice. A second noise lookup did that and cost half of this function's
    // 6.15ms, so instead it reuses the ladder's own relief, which is already
    // interpolated to this fragment and free. It is also the better field: the
    // ice surface's own bumps are what decide where it opens.
    let block_value = clamp(
        terrain_detail_meters / (TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS * CREVASSE_BLOCK_RELIEF_SHARE),
        -1.0,
        1.0,
    );
    let phase = fract(
        cycle
            + segment.value * CREVASSE_SEGMENT_OFFSET_CYCLES
            + block_value * CREVASSE_BLOCK_OFFSET_CYCLES,
    );
    // One slot per cycle: a signed ramp across its width, zero on intact ice.
    // `across` is +1 on the uphill wall and -1 on the downhill one.
    let centred = (phase - 0.5) * 2.0;
    let width = CREVASSE_WIDTH_SHARE
        * mix(CREVASSE_BLOCK_WIDTH_FLOOR, 1.0, clamp(block_value * 0.5 + 0.5, 0.0, 1.0));
    let open = 1.0 - smoothstep(0.0, width, abs(centred));
    let across = clamp(centred / max(width, 1.0e-4), -1.0, 1.0);
    // Down the fall line: the tangential part of the normal points downhill, so
    // this is the axis a contour-parallel slot is cut across.
    let tangential = surface_normal - surface_direction * dot(surface_normal, surface_direction);
    let fall = normalize_or_zero_vec3(tangential);
    // Presence is a mask, not a weight. Multiplying four sub-unit factors
    // together left the deepest slot at 15% darker than intact ice, which the
    // tone curve then rounded away; a crevasse either is there or is not, and
    // where it is, its floor is in shadow.
    let presence = smoothstep(0.10, 0.45, strength * present);
    let slot = open * open;
    let amount = presence * open;
    return GlacierCrevasses(
        fall * (across * amount * CREVASSE_WALL_TILT),
        amount,
        1.0 - presence * slot * CREVASSE_SUN_OCCLUSION,
    );
}

fn normalize_or_zero_vec3(value: vec3<f32>) -> vec3<f32> {
    let length_squared = dot(value, value);
    if length_squared <= 1.0e-12 {
        return vec3<f32>(0.0);
    }
    return value * inverseSqrt(length_squared);
}

fn neutralize_snow_surface_lighting_blend(
    lighting: vec3<f32>,
    blend: BiomeBlendSample,
) -> vec3<f32> {
    // 0.82 of the way to grey left snow shadows colourless; sky-lit snow is
    // blue. Keep enough neutrality that low sun cannot paint the icecap orange.
    let luminance = dot(lighting, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(lighting, vec3<f32>(luminance), 0.55 * biome_blend_snow_share(blend));
}

fn terrain_material_transmittance(
    transmittance: vec3<f32>,
    biome_id: u32,
) -> vec3<f32> {
    // Vacuum neither absorbs nor scatters: ground reaches the eye undimmed at
    // any range, which is why an airless body's distances are so hard to judge.
    if !BODY_HAS_ATMOSPHERE {
        return vec3<f32>(1.0);
    }
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
    // Nothing between the surface and the eye to scatter light into the path,
    // so there is no aerial perspective and no horizon haze at all.
    if !BODY_HAS_ATMOSPHERE {
        return vec3<f32>(0.0);
    }
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

/// How shadow-prone a region has to be before ice can hold there. Deliberately
/// a wide band: the baked fraction is one value per 828m cell, and a narrow
/// threshold across it is a staircase however it is filtered.
const AIRLESS_ICE_REGION_LOW: f32 = 0.25;
const AIRLESS_ICE_REGION_HIGH: f32 = 0.85;
/// How far a facet has to turn away from the sun's plane to be in permanent
/// shadow, as the cosine between its horizontal normal and its horizontal
/// position. -1 is facing straight poleward.
const AIRLESS_ICE_FACING_LIT: f32 = -0.12;
const AIRLESS_ICE_FACING_DARK: f32 = -0.55;

/// How permanently the sun misses this facet, from its own orientation.
///
/// With zero obliquity the sun stays in the equatorial plane, so it only ever
/// reaches directions `s` with no vertical component. A facet is lit at some
/// point in the rotation unless `dot(n, s) <= 0` for every such `s` that is also
/// above its horizon -- and because both tests depend only on the horizontal
/// parts of `n` and the position, that reduces to one dot product: the facet is
/// never lit exactly when its horizontal normal points *opposite* its horizontal
/// position. Which is to say, when it faces poleward.
///
/// Exact for self-shadowing, and it costs nothing, but its real value is that
/// it works at fragment resolution. The baked shadow field is computed on the
/// 828m working grid, which does not contain the relief that casts the shadows
/// you can actually see -- so ice placed from it alone lands beside the shadows
/// rather than in them, in hard 828m blocks. This follows the drawn surface.
fn airless_permanent_shadow(surface_normal: vec3<f32>, surface_direction: vec3<f32>) -> f32 {
    let position = vec2<f32>(surface_direction.x, surface_direction.z);
    let normal = vec2<f32>(surface_normal.x, surface_normal.z);
    // At the poles both go to zero and the question stops meaning anything;
    // there the regional term is already saturated.
    if length(position) < 1.0e-4 || length(normal) < 1.0e-5 {
        return 1.0;
    }
    let facing = dot(normalize(normal), normalize(position));
    return smoothstep(AIRLESS_ICE_FACING_LIT, AIRLESS_ICE_FACING_DARK, facing);
}

/// How dark the deepest basin floor goes, against unmarked regolith.
///
/// The Moon's two terrains differ by nearly a factor of two: anorthositic
/// highlands sit around 0.13 and mare basalt around 0.07. The maria are exactly
/// the basins -- impacts deep enough to crack the crust, later flooded by
/// basalt that welled up through it -- so on this body low ground *is* mare
/// ground, and the depth already in the height field says where.
const MOON_MARE_DARKEST: f32 = 0.62;
/// Depth below the datum at which that is reached, in metres.
const MOON_MARE_DEPTH_METERS: f32 = 6000.0;

/// Where the surface is bright and where it is dark, as a multiplier on the
/// regolith albedo.
///
/// The Moon is not a uniform grey ball, and none of what breaks it up is
/// texture in the wallpaper sense -- every marking is a *record of an impact*,
/// which is why this is driven by the same catalogue that dug the holes rather
/// than by a noise field laid over the top.
///
/// Three processes, in the order they matter at a distance:
///
/// * **Maria.** Basins flooded with dark basalt. Keyed to depth, because on
///   this body the deep places are the ones that cracked the crust.
/// * **Ejecta haloes.** Freshly excavated material is bright. Solar wind and
///   micrometeorite gardening darken and redden it over hundreds of millions of
///   years, so the halo fades with the crater's age rather than its size.
/// * **Rays.** The finest ejecta, thrown furthest, from the youngest craters
///   only. This is what makes Tycho visible from a garden on Earth.
///
/// The last two depend on direction alone, so they are baked into a cubemap
/// once at startup and only sampled here; the code that draws them lives in
/// `moon_markings.wgsl`. The maria depend on the height field, so they stay.
fn moon_surface_albedo_scale(direction: vec3<f32>, macro_height_meters: f32) -> f32 {
    let unit = normalize(direction);
    // Mare first, so a bright halo laid on a dark floor stays darker than the
    // same halo on highland -- which is how a real one behaves. This one stays
    // per-fragment: it is a smoothstep on a height the shader already has, it
    // costs nothing, and it tracks the height field at its own resolution
    // rather than the marking map's.
    let depth = max(MOON_DATUM_METERS - macro_height_meters, 0.0);
    let scale = mix(
        1.0,
        MOON_MARE_DARKEST,
        smoothstep(0.0, MOON_MARE_DEPTH_METERS, depth),
    );
    // The markings are baked once into a cubemap at startup, because they are
    // a function of direction alone and were costing 12.1ms of every 28.7ms
    // moon-surface frame to rederive -- 96 markings walked per fragment, each
    // an acos, an atan2 and seven cosines once it reached.
    let markings = textureSampleLevel(
        moon_marking_map,
        moon_marking_sampler,
        unit,
        0.0,
    ).r;
    return scale * (1.0 + 1.6 * markings);
}

/// How much of this ground is ice, 0 to 1.
///
/// Two terms with different jobs. The baked fraction knows about crater rims,
/// which a fragment cannot see, but only at 828m. The facing term knows nothing
/// about rims and resolves every pixel. Multiplied, the first says *where* ice
/// is possible and the second says exactly which ground it lies on -- so a
/// crater's poleward wall ices and its sunward wall does not, at the resolution
/// the surface is drawn.
///
/// Shared with the specular, which is the point. The highlight used to key off
/// the *categorical* biome instead: ice biome fell through
/// `material_specular_scale` to full strength while regolith got 0.16, so a
/// bright white glint appeared in hard 828m blocks on top of the smoothly mixed
/// pink underneath it. One mix, both uses, no disagreement.
fn airless_ice_mix(
    moisture: f32,
    surface_normal: vec3<f32>,
    surface_direction: vec3<f32>,
) -> f32 {
    let region = smoothstep(AIRLESS_ICE_REGION_LOW, AIRLESS_ICE_REGION_HIGH, moisture);
    return region * airless_permanent_shadow(surface_normal, surface_direction);
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
    // An airless body has exactly two materials, regolith and basin ice.
    // Everything below this line is a planet's material chain -- a beach blend,
    // a moisture wash, vegetation -- and none of it describes rock in vacuum.
    //
    // The mix between them is the *shadow fraction*, carried in the moisture
    // channel and sampled bilinearly, rather than the categorical biome. The
    // biome is one value per texel and sampled nearest, so using it drew the ice
    // as hard axis-aligned cells and plus-shapes -- reported, and visible from
    // orbit. A bilinear fraction with a soft threshold gives an edge that
    // follows the ground at any distance, because it is a field rather than a
    // stencil. The biome still decides identity; this decides the look.
    if !BODY_HAS_ATMOSPHERE {
        let ice = airless_ice_mix(moisture, surface_normal, surface_direction);
        // Markings apply to the rock, not to the ice: a permanently shadowed
        // floor is not weathered by the sun and holds no ejecta from craters
        // that post-date it being ice.
        let regolith =
            biome_color(8u) * moon_surface_albedo_scale(surface_direction, macro_height_meters);
        // The ice takes the body's own tint rather than a separate palette
        // entry, so the planet's glaciers keep reading the shared colour.
        return BODY_TERRAIN_TINT * mix(regolith, BODY_ICE_TINT * biome_color(2u), ice);
    }
    var color = vec3<f32>(0.32, 0.58, 0.74);
    if !outmap {
        return color;
    }

    color = base_color * mix(0.88, 1.06, moisture);
    // Use bilinear terrain height, not a nearest biome class, for the coast:
    // a continuous shallow-water/beach transition. Ice and snow get no sand,
    // judged from the blended palette so the exclusion has no texel edges.
    let beach = (1.0 - smoothstep(20.0, 220.0, macro_height_meters))
        * (1.0 - smoothstep(0.55, 0.80, dot(base_color, vec3<f32>(0.2126, 0.7152, 0.0722))));
    color = mix(color, srgb_to_linear(BEACH_SAND_COLOUR_SRGB), beach * 0.65);
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
    // Rock is darker than the grey it used to be, and it is not one tone:
    // the bounded detail field bands it so a face reads as strata rather than
    // a flat patch. The field is already computed above, so this is free.
    let rock_shade = 1.0 + detail * 0.55;
    let rock_color = srgb_to_linear(vec3<f32>(0.21, 0.19, 0.17)) * rock_shade;
    // A snow biome's palette colour is the snow itself, so where the face is
    // too steep to hold snow the rock beneath has to replace it.
    //
    // The replacement is unconditional. It used to be weighted by
    // `smoothstep(0.55, 0.80)` on the blended palette's luminance, as a proxy
    // for how much of this ground is snow -- and across an ice/rock border that
    // proxy leaves a strip half-replaced, so the ice palette's brightness
    // partly survives and the border renders as a pale ribbon between two dark
    // faces. Measured on the round 19 judging views, where all three judges
    // called it "a hard pale diagonal line": at the midpoint of the blend it
    // came out two to three times brighter than either side of it.
    //
    // Dropping the weight is also the more honest rule. Ground too steep to
    // hold snow shows the rock underneath whatever its palette says, and where
    // the palette was not snow in the first place this mixes a rock colour
    // toward a rock colour, which is what `rock_amount` does immediately below
    // in any case.
    color = mix(
        color,
        biome_color(8u) * mix(0.88, 1.06, moisture),
        1.0 - snow_slope_hold(slope, biome),
    );
    color = mix(color, rock_color, rock_amount * 0.88);
    let latitude_amount = abs(surface_direction.y);
    let snowline_meters = mix(6200.0, 2200.0, latitude_amount);
    let snow_amount = smoothstep(
        snowline_meters,
        snowline_meters + 900.0,
        macro_height_meters,
    ) * snow_slope_hold(slope, biome);
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
    // Vegetation, earth, rock and snow: the planet's four material layers,
    // blended by moisture and slope. There is no vegetation on an airless
    // body, no soil, and no snow that is not the ice already in its biome map
    // -- and now that the moon streams tiles like any other world, `outmap` no
    // longer keeps it out of here. Its two materials are the palette colours
    // `terrain_material_color` returns, unmodulated.
    if !BODY_HAS_ATMOSPHERE {
        return vec3<f32>(1.0);
    }
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
            macro_height_meters + terrain_detail_meters,
            fine_position,
            fine_weight,
        );
    }
    if base_weights.y > 1.0e-4 {
        earth = triplanar_material_sample(
            TERRAIN_MATERIAL_EARTH,
            surface_direction,
            surface_normal,
            macro_height_meters + terrain_detail_meters,
            fine_position,
            fine_weight,
        );
    }
    if base_weights.z > 1.0e-4 {
        rock = triplanar_material_sample(
            TERRAIN_MATERIAL_ROCK,
            surface_direction,
            surface_normal,
            macro_height_meters + terrain_detail_meters,
            fine_position,
            fine_weight,
        );
    }
    if base_weights.w > 1.0e-4 {
        snow = triplanar_material_sample(
            TERRAIN_MATERIAL_SNOW,
            surface_direction,
            surface_normal,
            macro_height_meters + terrain_detail_meters,
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
    // Divide by the blend's own mean albedo rather than by `base_albedo`. The
    // old ratio asked "how does this texel differ from the colour the palette
    // chose", so a dark palette entry inflated it: measured on the summit
    // survey, rock's palette colour (linear ~0.035) under rock's texture mean
    // (~0.065) pinned the ratio at the 2.4 ceiling and multiplied every shed
    // face back to pale grey -- the exposed rock existed in the material and
    // was erased here. Dividing by the layer mean makes this a pure detail
    // ratio centred on 1.0, so rock keeps both the palette's darkness and the
    // stone texture.
    let material_mean = TERRAIN_MATERIAL_MEAN_VEGETATION * weights.x
        + TERRAIN_MATERIAL_MEAN_EARTH * weights.y
        + TERRAIN_MATERIAL_MEAN_ROCK * weights.z
        + TERRAIN_MATERIAL_MEAN_SNOW * weights.w;
    let tint = clamp(
        material_albedo / max(material_mean, vec3<f32>(0.015)),
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
    surface_height_meters: f32,
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
    // Height belongs in the coordinate: without it every point up a cliff face
    // shares one direction and so one texel, and the material smears into
    // vertical streaks instead of reading as rock.
    let texture_position = surface_direction
        * ((PLANET_RADIUS_METERS + surface_height_meters) / TERRAIN_MATERIAL_TILE_METERS);
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

// Reverse interface crossing: air into water bends toward the wave normal.
fn ocean_air_to_water(view_ray: vec3<f32>, outward_normal: vec3<f32>) -> vec3<f32> {
    let eta = 1.0 / 1.333;
    let cosine = clamp(-dot(view_ray, outward_normal), 0.0, 1.0);
    let water_cosine = sqrt(1.0 - eta * eta * (1.0 - cosine * cosine));
    return eta * view_ray + (eta * cosine - water_cosine) * outward_normal;
}

fn ocean_water_transmittance(distance_meters: f32) -> f32 {
    return exp(-max(distance_meters, 0.0) * log(50.0) / OCEAN_UNDERWATER_VISIBILITY_METERS);
}

// View-ray refraction from water (n=1.333) into air (n=1). xyz is the
// refracted sky direction; w is unpolarised Fresnel transmission. A zero w
// means total internal reflection: never sample the sky with a zero vector.
fn ocean_water_to_air(view_ray: vec3<f32>, outward_normal: vec3<f32>) -> vec4<f32> {
    let eta = 1.333;
    let cos_water = clamp(dot(view_ray, outward_normal), 0.0, 1.0);
    let sin_air_squared = eta * eta * (1.0 - cos_water * cos_water);
    if sin_air_squared >= 1.0 {
        return vec4<f32>(0.0);
    }
    let cos_air = sqrt(1.0 - sin_air_squared);
    let sky_ray = eta * view_ray + (cos_air - eta * cos_water) * outward_normal;
    let rs = (eta * cos_water - cos_air) / (eta * cos_water + cos_air);
    let rp = (eta * cos_air - cos_water) / (eta * cos_air + cos_water);
    let transmission = 1.0 - 0.5 * (rs * rs + rp * rp);
    return vec4<f32>(sky_ray, transmission);
}

fn ocean_underside_reflection_with_skylight(
    reflected: vec3<f32>,
    skylight: vec3<f32>,
) -> vec3<f32> {
    return mix(reflected, skylight, OCEAN_UNDERSIDE_SKYLIGHT_BLEND);
}

fn ocean_underside_with_foam(
    clear_interface: vec3<f32>,
    skylight: vec3<f32>,
    foam: f32,
) -> vec3<f32> {
    let neutral_skylight = vec3<f32>(max(max(skylight.r, skylight.g), skylight.b));
    let scattered = mix(
        skylight,
        neutral_skylight,
        OCEAN_UNDERSIDE_FOAM_NEUTRALISATION,
    ) * OCEAN_UNDERSIDE_FOAM_RADIANCE_SCALE;
    return mix(clear_interface, scattered, foam);
}

/// The sea seen from underneath.
///
/// `ocean_lighting` cannot do this: it takes `max(dot(normal, view), 0.0)`, and
/// from below that is zero over the whole surface, so Fresnel is constant, the
/// reflection samples one texel of a 1x1-per-face cubemap, and the underside
/// comes out a single flat colour with no waves in it at all.
///
/// What actually makes waves visible from under water is Snell's window. Light
/// from the whole sky is refracted into a cone of half-angle
/// `asin(1 / 1.333) = 48.6 degrees` about the surface normal; outside that cone
/// the surface is a mirror looking back down into the dark. The cone is about
/// the *local* normal, so its edge follows every wave, and that moving boundary
/// is the shape you see.
fn ocean_underside_colour(
    surface_normal: vec3<f32>,
    ripple_slope: vec3<f32>,
    surface_direction: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    reflected_scene: vec4<f32>,
    foam: f32,
) -> vec3<f32> {
    let view_ray = normalize(camera_relative_view_position);
    // The ripple layer is folded in the same way the lit side does it, so both
    // faces of the sea agree about which way it is pointing. Measured: it does
    // not visibly move the window's edge at 1m depth, and should not -- the
    // window is only about 2.3m across there and the shortest wave in the
    // spectrum is 7m, so a smooth boundary is the correct answer. It is depth
    // that widens the window enough for waves to distort it.
    let normal_view = normalize(planet_to_view(normalize(surface_normal - ripple_slope)));
    // The old path sampled view_ray unchanged and used the normal only for a
    // soft window mask. It therefore painted the same sky through every wave.
    // Bend the sky lookup with the local wave normal, with the physical
    // critical angle and Fresnel transition at the window's moving edge.
    let refraction = ocean_water_to_air(view_ray, normal_view);
    let render_debug_mode = u32(camera.projection.w + 0.5);
    if render_debug_mode == RENDER_DEBUG_UNDERSIDE_TRANSMISSION {
        return vec3<f32>(refraction.w);
    }
    let up_view = normalize(planet_to_view(surface_direction));
    let skylight = physical_camera_sky_radiance(up_view);
    let fallback = skylight * OCEAN_UNDERWATER_TINT * 0.30;
    // Outside Snell's window the interface reflects the submerged scene,
    // rather than becoming an opaque dark ceiling. Misses retain a bounded
    // fallback; confidence fades screen edges and already-extinguished data.
    let reflected_below = mix(fallback, reflected_scene.rgb, reflected_scene.w);
    let below = ocean_underside_reflection_with_skylight(
        reflected_below,
        skylight,
    );
    if render_debug_mode == RENDER_DEBUG_UNDERSIDE_REFRACTED_SKY {
        if refraction.w <= 0.0 { return vec3<f32>(0.0); }
        return physical_camera_sky_radiance(normalize(refraction.xyz));
    }
    var clear_interface = below;
    if refraction.w > 0.0 {
        let above = physical_camera_sky_radiance(normalize(refraction.xyz));
        clear_interface = mix(below, above, refraction.w);
    }
    // Use the same breaking/whitecap coverage as the top face. Foam is air in
    // water, so it blocks the directional sky and bed reflection while
    // returning diffuse pale skylight instead of behaving like white paint.
    return ocean_underside_with_foam(clear_interface, skylight, foam);
}

// Sea of Thieves-style water shading (FFT ocean only). After Rare's SIGGRAPH
// 2018 talk: a stylised blend of a deep-water colour and a subsurface colour,
// weighted by a wave-peak mask (here the FFT convergence, i.e. where the
// displacement field bunches), the view angle and the sun direction, plus an
// area-light sun specular (Karis 2013 representative point) whose roughness
// grows with range for the wide low-sun reflection.
const OCEAN_SOT_DEEP_COLOUR: vec3<f32> = vec3<f32>(0.004, 0.030, 0.140);
// Thin water is the same water, lighter and a little more cyan, not a
// different colour: about 1.5x the sunlit body (0.008, 0.150, 0.220).
const OCEAN_SOT_SUBSURFACE_COLOUR: vec3<f32> = vec3<f32>(0.014, 0.225, 0.300);
// Convergence where the peak mask starts and is full (dimensionless |k| h).
// Wide so the change is a gradient across the wave, not a patch edge.
const OCEAN_SOT_PEAK_ONSET: f32 = 0.06;
const OCEAN_SOT_PEAK_FULL: f32 = 1.0;
// First pass (25 Sept, judged too contrasty), kept for comparison.
const OCEAN_SOT_SUBSURFACE_COLOUR_V1: vec3<f32> = vec3<f32>(0.020, 0.300, 0.330);
const OCEAN_SOT_PEAK_ONSET_V1: f32 = 0.20;
const OCEAN_SOT_PEAK_FULL_V1: f32 = 0.65;
// Artistic angular radius (tan) of the sun for the area-light lobe; the real
// sun is 0.0046.
const OCEAN_SOT_SUN_RADIUS: f32 = 0.06;
const OCEAN_SOT_ROUGHNESS_NEAR: f32 = 0.08;
const OCEAN_SOT_ROUGHNESS_FAR: f32 = 0.5;

fn ocean_sot_specular(
    normal_view: vec3<f32>,
    view_direction: vec3<f32>,
    sun_direction_view: vec3<f32>,
    roughness: f32,
) -> f32 {
    let reflection = reflect(-view_direction, normal_view);
    let centre_to_ray = dot(sun_direction_view, reflection) * reflection - sun_direction_view;
    let closest = sun_direction_view
        + centre_to_ray * clamp(OCEAN_SOT_SUN_RADIUS / max(length(centre_to_ray), 1.0e-5), 0.0, 1.0);
    let light = normalize(closest);
    let widened = clamp(roughness + OCEAN_SOT_SUN_RADIUS * 0.5, 0.0, 1.0);
    let a2 = widened * widened;
    let half_vector = normalize(light + view_direction);
    let n_dot_h = max(dot(normal_view, half_vector), 0.0);
    let n_dot_l = max(dot(normal_view, light), 0.0);
    let n_dot_v = max(dot(normal_view, view_direction), 1.0e-3);
    let denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    // Peak-normalised GGX (1 at the lobe centre) so the existing glint scale
    // keeps its meaning while the tails give the wide low-sun sheen.
    let lobe = a2 * a2 / (denominator * denominator);
    let visibility = 0.5 / max(
        n_dot_l * (n_dot_v * (1.0 - widened) + widened)
            + n_dot_v * (n_dot_l * (1.0 - widened) + widened),
        1.0e-4,
    );
    let energy = (roughness / widened) * (roughness / widened);
    // Fade the lobe out at grazing view angles: Fresnel and visibility both
    // blow up there and drew a white line along the horizon.
    return lobe * min(visibility * 2.0, 1.5) * n_dot_l * energy * smoothstep(0.0, 0.12, n_dot_v);
}

fn ocean_lighting_sot(
    normal: vec3<f32>,
    crest_sharpness: f32,
    fine_crest_transmission: f32,
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
    let daylight = max(max(sun_transmittance.x, sun_transmittance.y), sun_transmittance.z);
    let peak_linear = clamp(
        (crest_sharpness - OCEAN_SOT_PEAK_ONSET) / (OCEAN_SOT_PEAK_FULL - OCEAN_SOT_PEAK_ONSET),
        0.0,
        1.0,
    );
    let peak = peak_linear * peak_linear * (3.0 - 2.0 * peak_linear) * peak_linear;
    let backlight = pow(max(dot(-view_direction, sun_direction_view), 0.0), 3.0);
    let sun_facing = max(dot(normal_view, sun_direction_view), 0.0);
    // Light reaches the viewer through the thin wave top; more so looking
    // toward the sun, and more where the surface tilts toward it.
    let subsurface_weight = clamp(
        peak * (0.35 + 0.65 * backlight) + 0.25 * sun_facing * (1.0 - facing) + 0.25 * fine_crest_transmission,
        0.0,
        1.0,
    );
    // Deep colour warms toward teal on faces turned to the sun (the existing
    // sun-facing body ramp), then thin peaks blend on toward the subsurface colour.
    let deep = mix(OCEAN_SOT_DEEP_COLOUR, ocean_body_albedo(normal), 0.85);
    let body = mix(deep, OCEAN_SOT_SUBSURFACE_COLOUR, subsurface_weight);
    let diffuse = body * (sky_diffuse + sun_transmittance * (0.4 * SURFACE_SUNLIGHT_SCALE));
    // Glow keeps the body's hue: it lightens the water rather than tinting it.
    let glow = body * peak * (0.15 + backlight)
        * sun_transmittance * (0.35 * SURFACE_SUNLIGHT_SCALE) * (vec3<f32>(1.0) - fresnel);
    let range = length(camera_relative_view_position);
    let roughness = mix(
        OCEAN_SOT_ROUGHNESS_NEAR,
        OCEAN_SOT_ROUGHNESS_FAR,
        smoothstep(30.0, 1500.0, range),
    );
    let specular = ocean_sot_specular(normal_view, view_direction, sun_direction_view, roughness);
    return diffuse + glow
        + reflected_color * fresnel * daylight * OCEAN_REFLECTION_SCALE
        + sun_transmittance * specular * fresnel
            * (OCEAN_SUN_GLINT_SCALE * SURFACE_SUNLIGHT_SCALE);
}

fn ocean_lighting(
    normal: vec3<f32>,
    crest_sharpness: f32,
    fine_crest_transmission: f32,
    camera_relative_view_position: vec3<f32>,
    sun_transmittance: vec3<f32>,
    sky_diffuse: vec3<f32>,
) -> vec3<f32> {
    if OCEAN_FFT_ENABLED {
        return ocean_lighting_sot(normal, crest_sharpness, fine_crest_transmission,
            camera_relative_view_position, sun_transmittance, sky_diffuse);
    }
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
    // Keep the solar glitter narrower than the resolved swell facets. A broad
    // lobe turns the regular wave-normal field into large rectangular pools of
    // light in steep-down views; the narrower lobe reads as separated glints
    // without adding another normal or texture sample.
    let specular = pow(max(dot(normal_view, half_vector), 0.0), 512.0);
    let daylight = max(max(sun_transmittance.x, sun_transmittance.y), sun_transmittance.z);
    // Keep the water body a dark blue; direct sunlight and reflection still
    // provide the daylight highlights and glints.
    let diffuse = ocean_body_albedo(normal)
        * (sky_diffuse + sun_transmittance * (0.4 * SURFACE_SUNLIGHT_SCALE));
    // The Phase 6 cubemap is static. It represents daytime sky reflection, so
    // gate it by direct daylight instead of reflecting a bright blue sky from
    // the fully occluded hemisphere.
    // Cheap thin-crest transmission approximation, not alpha transparency or
    // a measured water-volume thickness. Positive wave height selects the upper
    // crest; forward scattering lights it when the sun is behind the wave.
    // Keep depth writes and reflection intact; foam is composed by the caller.
    // Keyed on how sharp the crest is, not how high it stands: a thin crest is
    // what light gets through, and a small wave's tip is as thin as a large
    // one's. Squared rather than linear. The earlier note here preferred a
    // linear ramp for not accelerating through its middle, but the middle is
    // the whole question: with the ramp spanning p90 to p99 of the sea's
    // sharpness, a linear rise hands half-strength tint to everything past
    // p95. Transmission through a thinning wedge of water is not linear in its
    // thickness either. Cube the ramp and require tight sun alignment so this
    // remains an edge accent rather than a turquoise water colour.
    let sea_blend = smoothstep(0.15, 0.85, camera.flat_triangle_options.y);
    let onset = mix(OCEAN_CREST_TRANSMISSION_CALM_ONSET,
        OCEAN_CREST_TRANSMISSION_ONSET, sea_blend);
    let full = mix(OCEAN_CREST_TRANSMISSION_CALM_FULL,
        OCEAN_CREST_TRANSMISSION_FULL, sea_blend);
    let ramp = clamp(
        (crest_sharpness - onset) / (full - onset),
        0.0,
        1.0,
    );
    let crest = ramp * ramp * ramp;
    let fine_crest = min(fine_crest_transmission * 1.75, 1.0);
    let backlight = pow(max(dot(-view_direction, sun_direction_view), 0.0), 4.0);
    let transmission_tint = OCEAN_CREST_TRANSMISSION_TINT * crest
        + OCEAN_FINE_CREST_TRANSMISSION_TINT * fine_crest;
    let transmitted = transmission_tint
        * sun_transmittance * (SURFACE_SUNLIGHT_SCALE * backlight)
        * (vec3<f32>(1.0) - fresnel);
    return diffuse + transmitted
        + reflected_color * fresnel * daylight * OCEAN_REFLECTION_SCALE
        + sun_transmittance
            * specular
            * fresnel
            * (OCEAN_SUN_GLINT_SCALE * SURFACE_SUNLIGHT_SCALE);
}

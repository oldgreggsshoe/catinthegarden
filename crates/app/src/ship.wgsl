// Flat-shaded low-poly ship. Composed after shared_planet.wgsl (camera,
// `planet_to_view` and the sea come from there), with the terrain's shared
// group(2) bound so the hull can see the FFT sea it floats in.

struct ShipUniform {
    // The hull's local origin relative to the camera, already rotated into
    // view axes on the CPU in f64. A planet-absolute position would arrive
    // here with half a metre of f32 quantisation at a 4,000km radius.
    view_position: vec4<f32>,
    // Ship-local axes expressed in planet-local axes, one per column.
    orientation_x: vec4<f32>,
    orientation_y: vec4<f32>,
    orientation_z: vec4<f32>,
    // Local up at the hull, for the sky/ground ambient split.
    up: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> ship: ShipUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec3<f32>,
    // 1 for the solid hull; the water on and in the ship varies it.
    @location(3) alpha: f32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    // Flat, because the hull is meant to read as facets. Interpolating these
    // would round the low-poly silhouette's shading back off.
    @location(0) @interpolate(flat) normal: vec3<f32>,
    @location(1) @interpolate(flat) colour: vec3<f32>,
    // Where on the hull, relative to the camera in view axes: to tell which
    // part is under the waves, and where their caustics fall on it.
    @location(2) view_position: vec3<f32>,
    @location(3) alpha: f32,
}

fn ship_to_planet(vector: vec3<f32>) -> vec3<f32> {
    return ship.orientation_x.xyz * vector.x
        + ship.orientation_y.xyz * vector.y
        + ship.orientation_z.xyz * vector.z;
}

// Water under the hull is deep enough never to break or shoal the waves.
const SHIP_WATER_DEPTH_METERS: f32 = 1000.0;
// Vertex spacing of the sea mesh near the ship (L17-L18 chunks). The waterline
// on the hull is where that mesh meets it, so the water height here is filtered
// the way the mesh is rather than to the pixel.
const SHIP_WATERLINE_SPACING_METERS: f32 = 1.0;
// Reflected sun is Fresnel-weak at most elevations (2-6% of the direct sun),
// which on a dark hull is barely visible; stylised up, as Sea of Thieves does.
// Physical is 1.
const SHIP_REFLECTED_LIGHT_GAIN: f32 = 4.0;
// Caustics play only on the hull's sides: faces steeper than this (|normal.up|
// below the first value) take them fully, flatter ones (deck, roofs, the flat
// of the bottom) not at all past the second.
// How fast daylight is absorbed on its way down to the hull, per channel, per
// metre: red is gone in a few metres (e-fold 6m), green lasts to about 17m and
// blue to about 29m, so a hull dims and goes blue-green, then black, as it sinks
// (a fifth of the light at 10m, a thousandth at 50m).
const SHIP_DEPTH_ABSORPTION: vec3<f32> = vec3<f32>(0.17, 0.06, 0.035);
const SHIP_CAUSTIC_SIDE_FULL: f32 = 0.5;
const SHIP_CAUSTIC_SIDE_NONE: f32 = 0.8;

// How much of the caustic pattern a face takes for its steepness.
fn ship_caustic_side_weight(normal: vec3<f32>, up: vec3<f32>) -> f32 {
    return 1.0 - smoothstep(SHIP_CAUSTIC_SIDE_FULL, SHIP_CAUSTIC_SIDE_NONE, abs(dot(normal, up)));
}

// How far below the drawn sea surface this point of the hull is (negative
// above it). The sea is drawn displaced sideways by its choppy D, several
// metres in a rough sea, so the water standing over a point came from a label
// up to that far away: find it by fixed-point iteration, label = point - D,
// as the CPU buoyancy's Newton inverse does. Reading the height straight
// under the point instead put caustics on hull the drawn water had left.
fn ship_water_depth(view_position: vec3<f32>) -> f32 {
    let up = normalize(ship.up.xyz);
    ocean_fft_vertex_spacing_meters = SHIP_WATERLINE_SPACING_METERS;
    var label = view_position;
    var height = 0.0;
    for (var step = 0u; step < 3u; step += 1u) {
        ocean_fft_view_position = label;
        ocean_fft_label_view_position = label;
        let surface = ocean_surface(up, camera.projection.z, length(label), SHIP_WATER_DEPTH_METERS);
        height = surface.vertical_displacement;
        label = view_position - planet_to_view(surface.horizontal_displacement);
    }
    ocean_fft_vertex_spacing_meters = 0.0;
    return height - local_view_altitude_meters(view_position);
}

// Light the sea puts on the hull, as (direct sun, sky, reflected sun)
// multipliers. Below the waterline both the sun and the sky come through the
// water, and the sun is gathered and spread by the waves overhead exactly as on
// the sea bed (`ocean_fft_caustics`). Above it, sunlight bounced off the moving
// surface dances on the sides facing it (`ocean_fft_reflected_caustics`).
fn ship_sea_light(view_position: vec3<f32>, normal: vec3<f32>, sun_direction: vec3<f32>) -> vec4<f32> {
    if !OCEAN_FFT_ENABLED {
        return vec4<f32>(1.0, 1.0, 0.0, 0.0);
    }
    // Deck and superstructure well clear of any crest: nothing to do.
    let reach = 1.5 * ocean_fft_view.gain.z + 10.0 + OCEAN_REFLECTED_CAUSTIC_REACH_METERS;
    if local_view_altitude_meters(view_position) > reach {
        return vec4<f32>(1.0, 1.0, 0.0, 0.0);
    }
    let up = normalize(ship.up.xyz);
    let side = ship_caustic_side_weight(normal, up);
    let depth = ship_water_depth(view_position);
    let pixel_meters = length(view_position) * (2.0 * camera.projection.y / 720.0);
    let planet_offset = view_to_planet(view_position);
    if depth > 0.0 {
        // Within the same 2m of the surface as the reflected shimmer, fading
        // over the next metre to the plain (unfocused) sun.
        let near_surface = 1.0 - smoothstep(
            OCEAN_REFLECTED_CAUSTIC_FULL_METERS,
            OCEAN_REFLECTED_CAUSTIC_REACH_METERS,
            depth,
        );
        let caustics = mix(
            1.0,
            ocean_fft_caustics(planet_offset, up, sun_direction, depth, pixel_meters),
            side * near_surface * sun_visible_fraction(),
        );
        // The depth rides along as w: the light that reaches this far down has
        // lost its red first and then its green, per channel (`fs_main`).
        return vec4<f32>(caustics, 1.0, 0.0, depth);
    }
    let bounce = ocean_fft_reflected_caustics(planet_offset, up, sun_direction, -depth, pixel_meters)
        * max(dot(normal, ocean_reflected_sun_direction(up, sun_direction)), 0.0)
        * side * sun_visible_fraction() * SHIP_REFLECTED_LIGHT_GAIN;
    return vec4<f32>(1.0, 1.0, bounce, 0.0);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let planet_offset = ship_to_planet(input.position);
    let view_position = ship.view_position.xyz + planet_to_view(planet_offset);
    var output: VertexOutput;
    output.position = camera.projection_matrix * vec4<f32>(view_position, 1.0);
    output.normal = normalize(ship_to_planet(input.normal));
    output.colour = input.colour;
    output.view_position = view_position;
    output.alpha = input.alpha;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(ship_shaded(input), 1.0);
}

// The water on and in the ship. It is the sea's own water, so it takes the
// sea's colour (`ocean_underwater_medium_colour`: the water albedo, or the
// swirl, lit as the sea body is) rather than a painted blue, and it is thin:
// mostly see-through (alpha from its depth, ship_model::build_water), with
// the sky reflected off its surface by Fresnel, strongest at grazing angles.
// Wherever the sea covers it -- a deck awash, a hull going down -- it is the
// sea, and fades out over its first few centimetres under the surface.
@fragment
fn fs_water(input: VertexOutput) -> @location(0) vec4<f32> {
    let view_direction = normalize(input.view_position);
    var normal = normalize(input.normal);
    if dot(normal, view_direction) > 0.0 {
        normal = -normal;
    }
    var submerged = 0.0;
    let reach = 1.5 * ocean_fft_view.gain.z + 10.0;
    if OCEAN_FFT_ENABLED && local_view_altitude_meters(input.view_position) < reach {
        submerged = smoothstep(0.0, SHIP_WATER_SUBMERGED_FADE_METERS, ship_water_depth(input.view_position));
    }
    if submerged >= 1.0 {
        discard;
    }
    let sea_colour = ocean_underwater_medium_colour();
    let sky = storm_overcast_colour(physical_camera_sky_radiance(reflect(view_direction, normal)));
    let fresnel = 0.02 + 0.98 * pow(1.0 - abs(dot(normal, view_direction)), 5.0);
    let colour = mix(sea_colour, sky, fresnel);
    let fog = terrain_fog(
        input.view_position,
        normalize(ship.up.xyz),
        local_view_altitude_meters(input.view_position),
    );
    let alpha = clamp(input.alpha + 0.6 * fresnel, 0.0, 1.0) * (1.0 - submerged);
    return vec4<f32>(mix(colour, fog.color, fog.amount), alpha);
}

// Water on or in the hull fades out over this depth under the sea surface.
const SHIP_WATER_SUBMERGED_FADE_METERS: f32 = 0.05;

fn ship_shaded(input: VertexOutput) -> vec3<f32> {
    let sun_direction = normalize(camera.sun_direction.xyz);
    let normal = normalize(input.normal);
    let sun_lambert = max(dot(normal, sun_direction), 0.0);
    // Sun strength follows its own elevation, so the hull darkens with the sea
    // around it at low sun instead of staying lit against a dusk horizon.
    let sun_elevation = clamp(dot(sun_direction, ship.up.xyz), 0.0, 1.0);
    let sunlight = vec3<f32>(1.9, 1.78, 1.6) * sun_elevation;
    // Hemispheric ambient: sky from above, a dimmer bounce off the water below.
    let sky_facing = 0.5 + 0.5 * dot(normal, ship.up.xyz);
    let sky_light = mix(
        vec3<f32>(0.06, 0.075, 0.09),
        vec3<f32>(0.26, 0.32, 0.40),
        sky_facing,
    ) * (0.25 + 0.75 * sun_elevation);
    let sea = ship_sea_light(input.view_position, normal, sun_direction);
    let lit = input.colour
        * (sunlight * (sun_lambert * sea.x + sea.z) + sky_light * sea.y)
        * exp(-sea.w * SHIP_DEPTH_ABSORPTION);
    // Through the same distance fog as the sea it floats in (or the water, from
    // under it): in a storm's closing fog the hull greys with the waves.
    let fog = terrain_fog(
        input.view_position,
        normalize(ship.up.xyz),
        local_view_altitude_meters(input.view_position),
    );
    return mix(lit, fog.color, fog.amount);
}

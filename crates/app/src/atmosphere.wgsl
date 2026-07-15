const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_HEIGHT_METERS: f32 = 720000.0;
const ATMOSPHERE_EDGE_FADE_METERS: f32 = 480000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 36000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 4800.0;
const RAYLEIGH_COEFFICIENT: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFFICIENT: vec3<f32> = vec3<f32>(0.01e-6);
const MIE_G: f32 = 0.76;
const SOLAR_RADIANCE: f32 = 1.25;
const SKY_SAMPLE_COUNT: u32 = 16u;
const SKY_DENSITY_SAMPLE_EXPONENT: f32 = 3.0;

struct Camera {
    projection_matrix: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_view_altitude: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_direction_view: vec4<f32>,
    projection: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

fn density(altitude_meters: f32, scale_height_meters: f32) -> f32 {
    let clamped_altitude_meters = max(altitude_meters, 0.0);
    // The physical exponential density remains dominant, but this final taper
    // makes the finite raymarch shell disappear continuously into space.
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

fn sphere_interval(radius_meters: f32, radial_dot_ray: f32) -> vec2<f32> {
    let discriminant = radial_dot_ray * radial_dot_ray
        + ATMOSPHERE_RADIUS_METERS * ATMOSPHERE_RADIUS_METERS
        - radius_meters * radius_meters;
    if discriminant <= 0.0 {
        return vec2<f32>(-1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-radial_dot_ray - root, -radial_dot_ray + root);
}

fn solid_planet_entry_distance(radius_meters: f32, radial_dot_ray: f32) -> f32 {
    let discriminant = radial_dot_ray * radial_dot_ray
        + PLANET_RADIUS_METERS * PLANET_RADIUS_METERS
        - radius_meters * radius_meters;
    if discriminant <= 0.0 {
        return 1.0e30;
    }
    let root = sqrt(discriminant);
    let near_distance = -radial_dot_ray - root;
    if near_distance > 0.0 {
        return near_distance;
    }
    let far_distance = -radial_dot_ray + root;
    if far_distance > 0.0 {
        return far_distance;
    }
    return 1.0e30;
}

fn altitude_along_ray(radius_meters: f32, radial_dot_ray: f32, distance_meters: f32) -> f32 {
    return sqrt(
        radius_meters * radius_meters
            + 2.0 * radial_dot_ray * distance_meters
            + distance_meters * distance_meters,
    ) - PLANET_RADIUS_METERS;
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
    return smoothstep(-transition_meters, transition_meters, clearance_meters);
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

fn local_solar_transmittance(
    sample_altitude: f32,
    sample_radius: f32,
    sample_radial_dot_sun: f32,
    sample_direction: vec3<f32>,
    sun: vec3<f32>,
    shadow_transition_meters: f32,
) -> vec3<f32> {
    // A full shell endpoint-average treats the near-vacuum upper endpoint as
    // half of a dense, near-ground solar path. At sunset that turns the entire
    // lower sky black before its Rayleigh colour can scatter toward the camera.
    // Match direct surface lighting's scale-height air-mass estimate instead:
    // dense air still reddens and attenuates the low sun, but does not erase
    // the illuminated horizon.
    let sun_zenith_cosine = max(dot(sample_direction, sun), 0.0);
    let air_mass = min(1.0 / max(sun_zenith_cosine, 0.08), 12.0);
    let rayleigh_optical_depth = RAYLEIGH_COEFFICIENT
        * density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
        * RAYLEIGH_SCALE_HEIGHT_METERS
        * air_mass;
    let mie_optical_depth = MIE_COEFFICIENT
        * density(sample_altitude, MIE_SCALE_HEIGHT_METERS)
        * MIE_SCALE_HEIGHT_METERS
        * air_mass;
    return exp(-(rayleigh_optical_depth + mie_optical_depth))
        * sun_visibility(sample_radius, sample_radial_dot_sun, shadow_transition_meters);
}

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let horizontal = ndc.x * camera.projection.x * camera.projection.y;
    let vertical = ndc.y * camera.projection.y;
    return normalize(vec3<f32>(horizontal, vertical, -1.0));
}

fn density_sample_fraction(fraction: f32, closest_fraction: f32) -> f32 {
    // Allocate the fixed sample budget around the ray's lowest atmospheric
    // point, where the exponential density changes most rapidly. This avoids
    // quantized colour rings without increasing the fullscreen raymarch cost.
    if closest_fraction <= 0.05 {
        return pow(fraction, SKY_DENSITY_SAMPLE_EXPONENT);
    }
    if closest_fraction >= 0.95 {
        return 1.0 - pow(1.0 - fraction, SKY_DENSITY_SAMPLE_EXPONENT);
    }
    if fraction <= 0.5 {
        let local_fraction = fraction * 2.0;
        return closest_fraction
            * (1.0 - pow(1.0 - local_fraction, SKY_DENSITY_SAMPLE_EXPONENT));
    }
    let local_fraction = (fraction - 0.5) * 2.0;
    return closest_fraction
        + (1.0 - closest_fraction) * pow(local_fraction, SKY_DENSITY_SAMPLE_EXPONENT);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), position);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let ray = view_direction(input.ndc);
    let camera_altitude = camera.camera_planet_direction_view_altitude.w;
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude;
    let radial_dot_ray = camera_radius
        * dot(camera.camera_planet_direction_view_altitude.xyz, ray);
    let interval = sphere_interval(camera_radius, radial_dot_ray);
    let start_distance = max(interval.x, 0.0);
    // The fullscreen pass is a background. Stop at the solid planet rather
    // than integrating the far-side shell through an opaque surface.
    let end_distance = min(
        interval.y,
        solid_planet_entry_distance(camera_radius, radial_dot_ray),
    );
    if end_distance <= start_distance {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let atmosphere_path_length = end_distance - start_distance;
    let closest_distance = clamp(-radial_dot_ray, start_distance, end_distance);
    let closest_fraction = (closest_distance - start_distance) / atmosphere_path_length;
    let atmosphere_entry_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_ray,
        start_distance,
    );
    let sun = normalize(camera.sun_direction_view.xyz);
    let cos_theta = dot(ray, sun);
    let rayleigh_phase = phase_rayleigh(cos_theta);
    let mie_phase = phase_mie(cos_theta);
    // A binary shadow test per raymarch point produces visible concentric
    // terminator bands. Keep a wider penumbra at the dense lower layers so a
    // setting sun tapers smoothly all the way to full occultation, while
    // deeply shadowed samples still receive no direct in-scattering.
    var radiance = vec3<f32>(0.0);
    for (var index = 0u; index < SKY_SAMPLE_COUNT; index += 1u) {
        let fraction_start = f32(index) / f32(SKY_SAMPLE_COUNT);
        let fraction_end = f32(index + 1u) / f32(SKY_SAMPLE_COUNT);
        let sample_start = density_sample_fraction(fraction_start, closest_fraction);
        let sample_end = density_sample_fraction(fraction_end, closest_fraction);
        let sample_length = (sample_end - sample_start) * atmosphere_path_length;
        let distance_meters = start_distance
            + 0.5 * (sample_start + sample_end) * atmosphere_path_length;
        let sample_altitude = altitude_along_ray(camera_radius, radial_dot_ray, distance_meters);
        let sample_radius = PLANET_RADIUS_METERS + sample_altitude;
        let lower_atmosphere_weight = density(
            sample_altitude,
            RAYLEIGH_SCALE_HEIGHT_METERS,
        );
        let sample_shadow_transition_meters = max(24000.0, sample_length * 0.50)
            * mix(1.0, 2.0, lower_atmosphere_weight);
        let sample_radial_dot_sun = (
            camera_radius * dot(camera.camera_planet_direction_view_altitude.xyz, sun)
                + distance_meters * dot(ray, sun)
        );
        // The camera may be in space. Only the segment from the atmosphere
        // entry point to this sample has optical depth; treating the preceding
        // vacuum as half-density incorrectly darkened the lower atmosphere.
        let view_transmittance = transmittance(
            atmosphere_entry_altitude,
            sample_altitude,
            distance_meters - start_distance,
        );
        let sun_transmittance = local_solar_transmittance(
            sample_altitude,
            sample_radius,
            sample_radial_dot_sun,
            normalize(
                camera.camera_planet_direction_view_altitude.xyz * camera_radius
                    + ray * distance_meters,
            ),
            sun,
            sample_shadow_transition_meters,
        );
        let rayleigh_scattering = RAYLEIGH_COEFFICIENT
            * density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
            * rayleigh_phase;
        let mie_scattering = MIE_COEFFICIENT * density(sample_altitude, MIE_SCALE_HEIGHT_METERS)
            * mie_phase;
        radiance += view_transmittance * sun_transmittance
            * (rayleigh_scattering + mie_scattering)
            * sample_length;
    }
    return vec4<f32>(max(radiance * SOLAR_RADIANCE, vec3<f32>(0.0)), 1.0);
}

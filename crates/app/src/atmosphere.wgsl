const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_HEIGHT_METERS: f32 = 360000.0;
const ATMOSPHERE_EDGE_FADE_METERS: f32 = 240000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 36000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 4800.0;
const RAYLEIGH_COEFFICIENT: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFFICIENT: vec3<f32> = vec3<f32>(21.0e-6);
const MIE_G: f32 = 0.76;
const SOLAR_RADIANCE: f32 = 2.0;
const SKY_SAMPLE_COUNT: u32 = 16u;

struct Camera {
    view_projection: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_altitude: vec4<f32>,
    sun_direction: vec4<f32>,
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

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let horizontal = ndc.x * camera.projection.x * camera.projection.y;
    let vertical = ndc.y * camera.projection.y;
    return normalize(camera.camera_forward.xyz + camera.camera_right.xyz * horizontal
        + camera.camera_up.xyz * vertical);
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
    let camera_altitude = camera.camera_planet_direction_altitude.w;
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude;
    let radial_dot_ray = camera_radius * dot(camera.camera_planet_direction_altitude.xyz, ray);
    let interval = sphere_interval(camera_radius, radial_dot_ray);
    let start_distance = max(interval.x, 0.0);
    let end_distance = interval.y;
    if end_distance <= start_distance {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let sample_length = (end_distance - start_distance) / f32(SKY_SAMPLE_COUNT);
    let atmosphere_entry_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_ray,
        start_distance,
    );
    let sun = normalize(camera.sun_direction.xyz);
    let cos_theta = dot(ray, sun);
    let rayleigh_phase = phase_rayleigh(cos_theta);
    let mie_phase = phase_mie(cos_theta);
    // A binary shadow test per raymarch point produces visible concentric
    // terminator bands. Sixteen samples let this tighter spacing-aware blend
    // retain a naturally tapered directional limb without discrete contours.
    let sun_shadow_transition_meters = max(12000.0, sample_length * 0.30);
    var radiance = vec3<f32>(0.0);
    for (var index = 0u; index < SKY_SAMPLE_COUNT; index += 1u) {
        let distance_meters = start_distance + (f32(index) + 0.5) * sample_length;
        let sample_altitude = altitude_along_ray(camera_radius, radial_dot_ray, distance_meters);
        let sample_radius = PLANET_RADIUS_METERS + sample_altitude;
        let sample_radial_dot_sun = (
            camera_radius * dot(camera.camera_planet_direction_altitude.xyz, sun)
                + distance_meters * dot(ray, sun)
        );
        let sun_interval = sphere_interval(sample_radius, sample_radial_dot_sun);
        let sun_distance = max(sun_interval.y, 0.0);
        // The camera may be in space. Only the segment from the atmosphere
        // entry point to this sample has optical depth; treating the preceding
        // vacuum as half-density incorrectly darkened the lower atmosphere.
        let view_transmittance = transmittance(
            atmosphere_entry_altitude,
            sample_altitude,
            distance_meters - start_distance,
        );
        let sun_transmittance = transmittance(
            sample_altitude,
            ATMOSPHERE_HEIGHT_METERS,
            sun_distance,
        ) * sun_visibility(
            sample_radius,
            sample_radial_dot_sun,
            sun_shadow_transition_meters,
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

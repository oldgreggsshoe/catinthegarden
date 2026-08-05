const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_HEIGHT_METERS: f32 = 720000.0;
const ATMOSPHERE_EDGE_FADE_METERS: f32 = 480000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 36000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 4800.0;
const RAYLEIGH_COEFFICIENT: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFFICIENT: vec3<f32> = vec3<f32>(0.5e-6);
const MIE_G: f32 = 0.76;
const SOLAR_RADIANCE: f32 = 2.0;
const SKY_SAMPLE_COUNT: u32 = 16u;
const SKY_DENSITY_SAMPLE_EXPONENT: f32 = 3.0;
const TWILIGHT_SHADOW_TRANSITION_METERS: f32 = 36000.0;
const ANTISOLAR_TWILIGHT_MIN_SCATTER: f32 = 0.48;
// Clear Earth skies are blue without the neon cyan crossover that a high
// saturation pass produces in a screenshot.  Keep the transform modest so
// camera exposure and the physical scattering coefficients remain visible.
const SKY_ATMOSPHERE_SATURATION: f32 = 1.18;
// The useful photographic blue hour is the short interval after the red
// sunset, not the whole astronomical twilight.  These sine values correspond
// roughly to -4, -7, -12, and -16 degrees solar elevation.
const BLUE_HOUR_START_SINE: f32 = 0.07;
const BLUE_HOUR_FULL_SINE: f32 = 0.12;
const BLUE_HOUR_FADE_SINE: f32 = 0.28;
const BLUE_HOUR_END_SINE: f32 = 0.36;
const BLUE_HOUR_SCATTER_GAIN: f32 = 0.26;
const BLUE_HOUR_TINT: vec3<f32> = vec3<f32>(0.55, 0.75, 1.0);
// A bounded warm bridge keeps the visible sky intensity rising through the
// last blue-hour frame into the strong red horizon band. It is deliberately
// separate from direct terrain/ocean sunlight and adds no raymarch samples.
const TWILIGHT_RED_RADIANCE: vec3<f32> = vec3<f32>(0.30, 0.012, 0.001);

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

fn saturate_sky_color(color: vec3<f32>) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    return max(
        vec3<f32>(luminance) + (color - vec3<f32>(luminance)) * SKY_ATMOSPHERE_SATURATION,
        vec3<f32>(0.0),
    );
}

fn suppress_green_dominance(color: vec3<f32>) -> vec3<f32> {
    // Direct single scattering can cross from red/yellow extinction to blue
    // Rayleigh scattering through an unphysical green-dominant interval. A
    // real atmosphere's broader indirect paths and absorption desaturate that
    // crossover. Preserve yellow, cyan, blue, and red, but do not let green
    // become the largest sky channel.
    return vec3<f32>(color.r, min(color.g, max(color.r, color.b)), color.b);
}

fn blue_hour_weight(
    camera_solar_zenith_cosine: f32,
    camera_radius_meters: f32,
) -> f32 {
    // At altitude the geometric horizon is already below the radial
    // horizontal. Key blue hour to the sun's depression below that horizon,
    // rather than radial solar elevation, so the sky cannot turn blue while
    // the visible sun is still above the limb.
    let radius_ratio = PLANET_RADIUS_METERS / max(camera_radius_meters, PLANET_RADIUS_METERS);
    let horizon_solar_zenith_cosine = -sqrt(max(1.0 - radius_ratio * radius_ratio, 0.0));
    let solar_depression_sine = max(
        horizon_solar_zenith_cosine - camera_solar_zenith_cosine,
        0.0,
    );
    let rise = smoothstep(
        BLUE_HOUR_START_SINE,
        BLUE_HOUR_FULL_SINE,
        solar_depression_sine,
    );
    let fade = 1.0 - smoothstep(
        BLUE_HOUR_FADE_SINE,
        BLUE_HOUR_END_SINE,
        solar_depression_sine,
    );
    return rise * fade;
}

fn blue_hour_rayleigh_scattering(
    camera_altitude: f32,
    view_zenith_cosine: f32,
    rayleigh_phase: f32,
) -> vec3<f32> {
    // Analytic optical column for the indirect approximation. Accumulating a
    // second source through all 16 direct-scattering samples measured slower.
    // `tau / (1 + tau)` is a bounded approximation to `1 - exp(-tau)` that
    // avoids adding another fullscreen exponential.
    let view_air_mass = min(1.0 / max(view_zenith_cosine, 0.08), 12.5);
    let optical_depth = RAYLEIGH_COEFFICIENT
        * density(camera_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
        * RAYLEIGH_SCALE_HEIGHT_METERS
        * view_air_mass;
    return optical_depth / (vec3<f32>(1.0) + optical_depth) * rayleigh_phase;
}

fn low_sun_red_transition(solar_elevation: f32) -> f32 {
    let rising = smoothstep(-0.14, -0.03, solar_elevation);
    let fading = 1.0 - smoothstep(0.0, 0.09, solar_elevation);
    return rising * fading;
}

fn twilight_directional_weight(
    cos_theta: f32,
    camera_solar_zenith_cosine: f32,
) -> f32 {
    // This renderer only models direct single scattering. Its symmetric
    // Rayleigh phase therefore leaves the anti-solar twilight almost as bright
    // as the sunset direction, missing the directional contrast produced by
    // the real atmosphere and the rising Earth shadow. Keep daytime unchanged,
    // then smoothly restrain only the back hemisphere as the sun approaches
    // the horizon. The forward Mie lobe and sunset-facing sky are untouched.
    let twilight_amount = 1.0 - smoothstep(0.0, 0.25, camera_solar_zenith_cosine);
    let antisolar_amount = smoothstep(0.0, 1.0, max(-cos_theta, 0.0));
    return mix(
        1.0,
        ANTISOLAR_TWILIGHT_MIN_SCATTER,
        twilight_amount * antisolar_amount,
    );
}

fn twilight_solar_air_mass(solar_zenith_cosine: f32, sample_altitude_meters: f32) -> f32 {
    // A 12x grazing column made the horizon almost black before sunset. Start
    // with a brighter orange 8x column at the limb, then redden smoothly toward
    // the existing 12x column over roughly seven degrees of solar depression.
    let grazing_air_mass = min(1.0 / max(solar_zenith_cosine, 0.125), 8.0);
    let twilight_depth = smoothstep(0.0, 0.12, max(-solar_zenith_cosine, 0.0));
    let base_air_mass = mix(grazing_air_mass, 12.0, twilight_depth);
    // A local scale-height column substantially underestimates extinction in
    // the thin upper atmosphere near the limb. Increase only the near-horizon
    // column there so a high-altitude sunrise/sunset still loses blue before
    // fading to night; daytime illumination remains unchanged.
    let horizon_amount = 1.0 - smoothstep(0.08, 0.30, solar_zenith_cosine);
    let upper_atmosphere_amount = smoothstep(30000.0, 120000.0, sample_altitude_meters);
    return base_air_mass * mix(1.0, 8.0, horizon_amount * upper_atmosphere_amount);
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
    // Keep geometrically unoccluded air fully lit up to the limb. The wide
    // sample-spacing penumbra exists to hide bands behind the terminator; when
    // centred on zero it incorrectly halves the sky as soon as the sun dips
    // below a sample's local tangent plane, making sunset arrive too early.
    return smoothstep(-transition_meters, 0.0, clearance_meters);
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
    let sun_zenith_cosine = dot(sample_direction, sun);
    let air_mass = twilight_solar_air_mass(sun_zenith_cosine, sample_altitude);
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
    // The red bridge is a bounded low-sun fill, not a second atmosphere
    // shell.  Weight it by the density at the ray's lowest point so an
    // orbital view gets a thin, physical red limb instead of a solid red
    // annulus across the full 720 km shell.  Near the surface the lowest
    // point is the camera itself, preserving the validated sunrise band.
    let red_twilight_atmosphere_weight = density(
        altitude_along_ray(camera_radius, radial_dot_ray, closest_distance),
        RAYLEIGH_SCALE_HEIGHT_METERS,
    );
    let atmosphere_entry_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_ray,
        start_distance,
    );
    let sun = normalize(camera.sun_direction_view.xyz);
    let cos_theta = dot(ray, sun);
    let rayleigh_phase = phase_rayleigh(cos_theta);
    let mie_phase = phase_mie(cos_theta);
    let camera_solar_zenith_cosine = dot(
        camera.camera_planet_direction_view_altitude.xyz,
        sun,
    );
    let directional_weight = twilight_directional_weight(
        cos_theta,
        camera_solar_zenith_cosine,
    );
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
        let sample_shadow_transition_meters = max(
            TWILIGHT_SHADOW_TRANSITION_METERS,
            sample_length * 0.50,
        )
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
    // The direct single-scattering term above correctly produces the warm
    // sunset, but becomes zero once every visible sample enters the planet's
    // shadow. Real twilight then retains a blue indirect/multiple-scattered
    // sky before astronomical darkness. A bounded analytic Rayleigh view
    // column adds no samples and cannot affect terrain, ocean, aerial
    // perspective, or night.
    let blue_hour_radiance = blue_hour_rayleigh_scattering(
        camera_altitude,
        max(
            dot(camera.camera_planet_direction_view_altitude.xyz, ray),
            0.0,
        ),
        rayleigh_phase,
    )
        * BLUE_HOUR_TINT
        * (SOLAR_RADIANCE * BLUE_HOUR_SCATTER_GAIN)
        * blue_hour_weight(camera_solar_zenith_cosine, camera_radius);
    let red_twilight_radiance = TWILIGHT_RED_RADIANCE
        * low_sun_red_transition(camera_solar_zenith_cosine)
        * red_twilight_atmosphere_weight
        * mix(
            0.35,
            1.0,
            smoothstep(0.0, 1.0, max(cos_theta, 0.0)),
        );
    let sky_radiance = max(
        radiance * SOLAR_RADIANCE * directional_weight
            + blue_hour_radiance
            + red_twilight_radiance,
        vec3<f32>(0.0),
    );
    return vec4<f32>(
        suppress_green_dominance(saturate_sky_color(sky_radiance)),
        1.0,
    );
}

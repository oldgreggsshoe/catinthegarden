// Shared physical atmosphere model for the LUT generation passes.
// The visible atmosphere is 2,880 km deep in world space so the exaggerated
// game terrain remains inside useful air. Sky visibility is clipped against
// that world-space shell, while LUT integration maps its density profile into
// a 640 km optical atmosphere around the actual 4,000 km planet. This keeps
// the requested shell extent without changing the established Earth-like
// extinction or relying on a timed twilight palette.
const PI: f32 = 3.141592653589793;
const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_VERTICAL_SCALE: f32 = 4.5;
const SKY_VIEW_MINIMUM_CAMERA_ALTITUDE_METERS: f32 = 200.0;
const ATMOSPHERE_HEIGHT_METERS: f32 = 2880000.0;
const OPTICAL_PLANET_RADIUS_METERS: f32 = PLANET_RADIUS_METERS;
const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 =
    ATMOSPHERE_HEIGHT_METERS / ATMOSPHERE_VERTICAL_SCALE;
const OPTICAL_ATMOSPHERE_EDGE_FADE_METERS: f32 = 426666.668;
const OPTICAL_ATMOSPHERE_RADIUS_METERS: f32 =
    OPTICAL_PLANET_RADIUS_METERS + OPTICAL_ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 8000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 1200.0;
const OZONE_CENTER_METERS: f32 = 25000.0;
const OZONE_HALF_WIDTH_METERS: f32 = 15000.0;
const RAYLEIGH_SCATTERING: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_SCATTERING: vec3<f32> = vec3<f32>(4.0e-6);
const MIE_ABSORPTION: vec3<f32> = vec3<f32>(0.4e-6);
const OZONE_ABSORPTION: vec3<f32> = vec3<f32>(0.65e-6, 1.881e-6, 0.085e-6);
// A stronger ozone column deepens the Chappuis-band absorption along grazing
// sunlight. This changes the physical optical depth rather than applying a
// sunset colour, and leaves overhead Rayleigh/Mie daylight structurally
// unchanged.
const OZONE_COLUMN_SCALE: f32 = 1.5;
const GROUND_ALBEDO: vec3<f32> = vec3<f32>(0.12);
const MIE_G: f32 = 0.76;
// Relative illuminance used by the Hillaire LUT formulation. Ten keeps the
// fixed-exposure twilight visible without an exposure- or time-keyed colour
// floor; the display tone map handles the much brighter daytime range.
const SOLAR_LUMINANCE: f32 = 10.0;

fn atmosphere_edge_fade(altitude_meters: f32) -> f32 {
    return 1.0 - smoothstep(
        OPTICAL_ATMOSPHERE_HEIGHT_METERS - OPTICAL_ATMOSPHERE_EDGE_FADE_METERS,
        OPTICAL_ATMOSPHERE_HEIGHT_METERS,
        max(altitude_meters, 0.0),
    );
}

fn rayleigh_density(altitude_meters: f32) -> f32 {
    return exp(-max(altitude_meters, 0.0) / RAYLEIGH_SCALE_HEIGHT_METERS)
        * atmosphere_edge_fade(altitude_meters);
}

fn mie_density(altitude_meters: f32) -> f32 {
    return exp(-max(altitude_meters, 0.0) / MIE_SCALE_HEIGHT_METERS)
        * atmosphere_edge_fade(altitude_meters);
}

fn ozone_density(altitude_meters: f32) -> f32 {
    return max(
        1.0 - abs(altitude_meters - OZONE_CENTER_METERS) / OZONE_HALF_WIDTH_METERS,
        0.0,
    ) * atmosphere_edge_fade(altitude_meters);
}

fn medium_scattering(altitude_meters: f32) -> vec3<f32> {
    return RAYLEIGH_SCATTERING * rayleigh_density(altitude_meters)
        + MIE_SCATTERING * mie_density(altitude_meters);
}

fn medium_extinction(altitude_meters: f32) -> vec3<f32> {
    return medium_scattering(altitude_meters)
        + MIE_ABSORPTION * mie_density(altitude_meters)
        + OZONE_ABSORPTION * ozone_density(altitude_meters) * OZONE_COLUMN_SCALE;
}

fn rayleigh_phase(cos_theta: f32) -> f32 {
    return 3.0 * (1.0 + cos_theta * cos_theta) / (16.0 * PI);
}

fn mie_phase(cos_theta: f32) -> f32 {
    let g2 = MIE_G * MIE_G;
    let denominator = max(1.0 + g2 - 2.0 * MIE_G * cos_theta, 1.0e-4);
    return 3.0 * (1.0 - g2) * (1.0 + cos_theta * cos_theta)
        / (8.0 * PI * (2.0 + g2) * pow(denominator, 1.5));
}

fn phase_scattering(altitude_meters: f32, cos_theta: f32) -> vec3<f32> {
    return RAYLEIGH_SCATTERING * rayleigh_density(altitude_meters)
            * rayleigh_phase(cos_theta)
        + MIE_SCATTERING * mie_density(altitude_meters)
            * mie_phase(cos_theta);
}

fn sphere_interval(position: vec3<f32>, direction: vec3<f32>, radius: f32) -> vec2<f32> {
    let b = dot(position, direction);
    let discriminant = b * b - dot(position, position) + radius * radius;
    if discriminant < 0.0 {
        return vec2<f32>(-1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-b - root, -b + root);
}

fn nearest_positive_sphere_distance(
    position: vec3<f32>,
    direction: vec3<f32>,
    radius: f32,
) -> f32 {
    let interval = sphere_interval(position, direction, radius);
    if interval.x > 1.0 {
        return interval.x;
    }
    if interval.y > 1.0 {
        return interval.y;
    }
    return -1.0;
}

fn optical_atmosphere_exit_distance(position: vec3<f32>, direction: vec3<f32>) -> f32 {
    return sphere_interval(position, direction, OPTICAL_ATMOSPHERE_RADIUS_METERS).y;
}

fn transmittance_lut_uv(altitude_meters: f32, zenith_cosine: f32) -> vec2<f32> {
    return vec2<f32>(
        clamp(zenith_cosine * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(altitude_meters / OPTICAL_ATMOSPHERE_HEIGHT_METERS, 0.0, 1.0)),
    );
}

fn sample_transmittance_lut(
    lut: texture_2d<f32>,
    lut_sampler: sampler,
    altitude_meters: f32,
    zenith_cosine: f32,
) -> vec3<f32> {
    return textureSampleLevel(
        lut,
        lut_sampler,
        transmittance_lut_uv(altitude_meters, zenith_cosine),
        0.0,
    ).rgb;
}

fn sample_multiple_scattering_lut(
    lut: texture_2d<f32>,
    lut_sampler: sampler,
    altitude_meters: f32,
    solar_zenith_cosine: f32,
) -> vec3<f32> {
    return textureSampleLevel(
        lut,
        lut_sampler,
        vec2<f32>(
            clamp(solar_zenith_cosine * 0.5 + 0.5, 0.0, 1.0),
            sqrt(clamp(altitude_meters / OPTICAL_ATMOSPHERE_HEIGHT_METERS, 0.0, 1.0)),
        ),
        0.0,
    ).rgb;
}

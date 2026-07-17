const PLANET_RADIUS_METERS: f32 = 4000000.0;
// Material/height tiles are intentionally denser than the fixed 32x32 chunk
// grid, so material detail and coastline transitions do not inherit mesh size.
const MATERIAL_TILE_LOGICAL_QUADS: f32 = 128.0;
const TILE_GUTTER: f32 = 1.0;
const MATERIAL_TILE_LAST_STORED_COORD: i32 = 130;
const GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS: f32 = 111.5;
const ATMOSPHERE_HEIGHT_METERS: f32 = 720000.0;
const ATMOSPHERE_EDGE_FADE_METERS: f32 = 480000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 36000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 4800.0;
const RAYLEIGH_COEFFICIENT: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFFICIENT: vec3<f32> = vec3<f32>(0.5e-6);
const MIE_G: f32 = 0.76;
const SOLAR_RADIANCE: f32 = 1.25;
// Artistic surface exposure only: this does not alter sky scattering or the
// camera-facing sun disc.
const SURFACE_SUNLIGHT_SCALE: f32 = 2.0;
const SKY_DIFFUSE_LIGHT_SCALE: f32 = 0.18;
const AERIAL_IN_SCATTER_SAMPLE_COUNT: u32 = 2u;
const AERIAL_DENSITY_SAMPLE_EXPONENT: f32 = 3.0;
// Artistic aerial-only control, applied after physically bounded integration.
// It does not alter extinction, direct terrain/ocean lighting, or the sky pass.
const AERIAL_IN_SCATTER_GAIN: f32 = 3.0;
const TWILIGHT_SHADOW_TRANSITION_METERS: f32 = 36000.0;
const TERRAIN_FOG_START_METERS: f32 = 2000.0;
const TERRAIN_FOG_END_METERS: f32 = 60000.0;
const TERRAIN_FOG_MAX_CAMERA_ALTITUDE_METERS: f32 = 100000.0;
const TERRAIN_FOG_FULL_HORIZON_COSINE: f32 = 0.05;
const TERRAIN_FOG_CLEAR_HORIZON_COSINE: f32 = 0.35;
const RENDER_DEBUG_FINAL: u32 = 0u;
const RENDER_DEBUG_RAW_ALBEDO: u32 = 1u;
const RENDER_DEBUG_SURFACE_LIGHTING: u32 = 2u;
const RENDER_DEBUG_AERIAL_CONTRIBUTION: u32 = 3u;

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

@group(1) @binding(0)
var height_map: texture_2d<f32>;

@group(1) @binding(1)
var biome_map: texture_2d<u32>;

@group(1) @binding(2)
var moisture_map: texture_2d<f32>;

@group(1) @binding(3)
var environment_map: texture_cube<f32>;

@group(1) @binding(4)
var environment_sampler: sampler;

struct TerrainSettings {
    outmap_height_scale: vec4<f32>,
}

@group(1) @binding(5)
var<uniform> terrain_settings: TerrainSettings;

struct VertexInput {
    @location(0) anchor_relative_position: vec3<f32>,
    @location(1) sphere_direction: vec3<f32>,
    @location(2) tile_uv: vec2<f32>,
    @location(3) skirt_depth_meters: f32,
    @location(4) anchor_view_position: vec3<f32>,
    @location(5) source_uv_scale: vec2<f32>,
    @location(6) source_uv_offset: vec2<f32>,
    @location(7) terrain_info: u32,
    @location(8) lod_transition: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) camera_relative_view_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) aerial_color: vec3<f32>,
    @location(3) lod_transition: vec2<f32>,
    @location(4) surface_direction: vec3<f32>,
    @location(5) ocean: f32,
    @location(6) source_uv: vec2<f32>,
    @location(7) outmap: f32,
    @location(8) surface_lighting: vec3<f32>,
    @location(9) terrain_detail_meters: f32,
}

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

fn uses_outmap(terrain_info: u32) -> bool {
    return (terrain_info & 1u) != 0u;
}

fn cube_face(terrain_info: u32) -> u32 {
    return (terrain_info >> 1u) & 0x7u;
}

fn requested_level(terrain_info: u32) -> u32 {
    return (terrain_info >> 4u) & 0x1fu;
}

fn terrain_detail_octave_distance_weight(
    camera_distance_meters: f32,
    full_distance_meters: f32,
) -> f32 {
    // Camera distance is continuous across mixed-LOD edges, unlike requested
    // chunk level. Scale it by FOV so optical zoom retains resolvable detail.
    let effective_distance = camera_distance_meters * camera.projection.y / 0.57735026;
    return 1.0 - smoothstep(
        full_distance_meters,
        full_distance_meters * 2.0,
        effective_distance,
    );
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

fn global_terrain_detail_octave(
    direction: vec3<f32>,
    frequency: f32,
    amplitude_meters: f32,
    axis: vec3<f32>,
    phase: f32,
) -> f32 {
    return amplitude_meters * sin(frequency * dot(direction, axis) + phase);
}

// Planet-local direction makes this continuous across cube-face and tile
// boundaries. The baked outmap still supplies all macro geography.
fn global_terrain_detail(direction: vec3<f32>, camera_distance_meters: f32) -> f32 {
    return global_terrain_detail_octave(
        direction, 4096.0, 80.0, vec3<f32>(0.79, 0.52, -0.32), 0.37,
    ) * terrain_detail_octave_distance_weight(
        camera_distance_meters, 150000.0,
    ) + global_terrain_detail_octave(
        direction, 32768.0, 24.0, vec3<f32>(-0.23, 0.91, 0.41), 1.11,
    ) * terrain_detail_octave_distance_weight(
        camera_distance_meters, 20000.0,
    ) + global_terrain_detail_octave(
        direction, 262144.0, 6.0, vec3<f32>(0.61, -0.17, 0.77), 2.07,
    ) * terrain_detail_octave_distance_weight(
        camera_distance_meters, 2500.0,
    ) + global_terrain_detail_octave(
        direction, 2097152.0, 1.5, vec3<f32>(-0.48, -0.66, 0.58), 2.73,
    ) * terrain_detail_octave_distance_weight(
        camera_distance_meters, 300.0,
    );
}

fn sample_height(source_uv: vec2<f32>) -> f32 {
    let gutter_uv = 1.0 / MATERIAL_TILE_LOGICAL_QUADS;
    let coordinate = vec2<f32>(TILE_GUTTER)
        + clamp(source_uv, vec2<f32>(-gutter_uv), vec2<f32>(1.0 + gutter_uv))
            * MATERIAL_TILE_LOGICAL_QUADS;
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(
        lower + vec2<i32>(1),
        vec2<i32>(MATERIAL_TILE_LAST_STORED_COORD),
    );
    let amount = fract(coordinate);
    let lower_left = textureLoad(height_map, lower, 0).x;
    let lower_right = textureLoad(height_map, vec2<i32>(upper.x, lower.y), 0).x;
    let upper_left = textureLoad(height_map, vec2<i32>(lower.x, upper.y), 0).x;
    let upper_right = textureLoad(height_map, upper, 0).x;
    return mix(
        mix(lower_left, lower_right, amount.x),
        mix(upper_left, upper_right, amount.x),
        amount.y,
    );
}

fn macro_terrain_height(outmap: bool, source_uv: vec2<f32>, direction: vec3<f32>) -> f32 {
    if outmap {
        return sample_height(source_uv);
    }
    return placeholder_height(direction);
}

fn terrain_height(
    outmap: bool,
    source_uv: vec2<f32>,
    direction: vec3<f32>,
    camera_distance_meters: f32,
) -> f32 {
    let macro_height = macro_terrain_height(outmap, source_uv, direction);
    if !outmap {
        return macro_height;
    }
    let land_detail_weight = smoothstep(100.0, 400.0, macro_height);
    return macro_height * terrain_settings.outmap_height_scale.x
        + global_terrain_detail(direction, camera_distance_meters)
            * land_detail_weight
            * terrain_settings.outmap_height_scale.y;
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
    let first = gerstner_wave(direction, vec3<f32>(0.9, 0.1, 0.4), 900.0, 1.5, 4.0, 0.45, time_seconds);
    let second = gerstner_wave(direction, vec3<f32>(-0.3, 0.4, 0.85), 420.0, 0.85, 5.0, 0.40, time_seconds);
    let third = gerstner_wave(direction, vec3<f32>(0.55, -0.75, 0.35), 160.0, 0.45, 6.5, 0.34, time_seconds);
    let fourth = gerstner_wave(direction, vec3<f32>(-0.75, -0.2, 0.63), 65.0, 0.22, 8.0, 0.28, time_seconds);
    let fifth = gerstner_wave(direction, vec3<f32>(0.2, 0.95, -0.24), 24.0, 0.11, 10.0, 0.20, time_seconds);
    let sixth = gerstner_wave(direction, vec3<f32>(-0.5, 0.7, -0.5), 9.0, 0.05, 12.0, 0.14, time_seconds);
    let horizontal = first.horizontal_displacement + second.horizontal_displacement
        + third.horizontal_displacement + fourth.horizontal_displacement
        + fifth.horizontal_displacement + sixth.horizontal_displacement;
    let vertical = first.vertical_displacement + second.vertical_displacement
        + third.vertical_displacement + fourth.vertical_displacement
        + fifth.vertical_displacement + sixth.vertical_displacement;
    let slope = first.slope + second.slope + third.slope + fourth.slope + fifth.slope + sixth.slope;
    return OceanSurface(horizontal, vertical, normalize(direction - slope));
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
    let radial_dot_sun = surface_radius * dot(surface_direction, sun_direction);
    let solar_visibility = smoothstep(
        -0.01,
        0.01,
        dot(surface_direction, sun_direction),
    );

    // The generic endpoint-average estimate spans the full 360km shell for
    // a noon surface point. That makes the near-zero density at its top count
    // as half the density of the entire path, nearly extinguishing direct
    // daylight before the existing surface-only intensity scale can matter.
    // Estimate a local scale-height air mass instead. It retains directional
    // warm attenuation near the terminator without altering sky scattering.
    let sun_zenith_cosine = max(dot(surface_direction, sun_direction), 0.0);
    let air_mass = min(1.0 / max(sun_zenith_cosine, 0.08), 12.0);
    let rayleigh_optical_depth = RAYLEIGH_COEFFICIENT
        * density(surface_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS)
        * RAYLEIGH_SCALE_HEIGHT_METERS
        * air_mass;
    let mie_optical_depth = MIE_COEFFICIENT
        * density(surface_altitude_meters, MIE_SCALE_HEIGHT_METERS)
        * MIE_SCALE_HEIGHT_METERS
        * air_mass;
    return exp(-(rayleigh_optical_depth + mie_optical_depth)) * solar_visibility;
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
    let rayleigh_scattering = RAYLEIGH_COEFFICIENT
        * density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
        * phase_rayleigh(cos_theta)
        * rayleigh_path_length;
    let mie_scattering = MIE_COEFFICIENT
        * density(sample_altitude, MIE_SCALE_HEIGHT_METERS)
        * phase_mie(cos_theta)
        * mie_path_length;
    return view_transmittance * sun_transmittance
        * (rayleigh_scattering + mie_scattering)
        * SOLAR_RADIANCE;
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
    let sky = max(local_sky, sunward_sky * 0.65);
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

fn aerial_perspective(
    lit_surface_color: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> vec3<f32> {
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
        return lit_surface_color;
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
    return lit_surface_color * view_transmittance + in_scatter;
}

fn terrain_distance_fog(
    aerial_color: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> vec3<f32> {
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
        return aerial_color;
    }
    let fog_color = sky_radiance(
        surface_to_camera_direction,
        surface_direction,
        surface_altitude_meters,
        normalize(camera.sun_direction.xyz),
    );
    return mix(aerial_color, fog_color, fog_amount);
}

fn sample_biome(source_uv: vec2<f32>) -> u32 {
    let coordinate = vec2<i32>(round(
        vec2<f32>(TILE_GUTTER)
            + clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0))
                * MATERIAL_TILE_LOGICAL_QUADS,
    ));
    return textureLoad(biome_map, coordinate, 0).x;
}

fn sample_moisture(source_uv: vec2<f32>) -> f32 {
    let coordinate = vec2<i32>(round(
        vec2<f32>(TILE_GUTTER)
            + clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0))
                * MATERIAL_TILE_LOGICAL_QUADS,
    ));
    return textureLoad(moisture_map, coordinate, 0).x;
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

fn face_component(direction: vec3<f32>, face: u32) -> f32 {
    if face <= 1u {
        return abs(direction.x);
    }
    if face <= 3u {
        return abs(direction.y);
    }
    return abs(direction.z);
}

fn displaced_surface_normal(
    direction: vec3<f32>,
    source_uv: vec2<f32>,
    source_uv_scale: vec2<f32>,
    terrain_info: u32,
    camera_distance_meters: f32,
) -> vec3<f32> {
    let face = cube_face(terrain_info);
    let tangent_u = face_tangent_u(face);
    let tangent_v = face_tangent_v(face);
    let cube_position = direction / max(face_component(direction, face), 1.0e-6);
    let cube_step = 2.0
        / (MATERIAL_TILE_LOGICAL_QUADS * exp2(f32(requested_level(terrain_info))));
    let left_direction = normalize(cube_position - tangent_u * cube_step);
    let right_direction = normalize(cube_position + tangent_u * cube_step);
    let down_direction = normalize(cube_position - tangent_v * cube_step);
    let up_direction = normalize(cube_position + tangent_v * cube_step);
    let uv_step = source_uv_scale / MATERIAL_TILE_LOGICAL_QUADS;
    let outmap = uses_outmap(terrain_info);
    let left_height = terrain_height(
        outmap,
        source_uv - vec2<f32>(uv_step.x, 0.0),
        left_direction,
        camera_distance_meters,
    );
    let right_height = terrain_height(
        outmap,
        source_uv + vec2<f32>(uv_step.x, 0.0),
        right_direction,
        camera_distance_meters,
    );
    let down_height = terrain_height(
        outmap,
        source_uv - vec2<f32>(0.0, uv_step.y),
        down_direction,
        camera_distance_meters,
    );
    let up_height = terrain_height(
        outmap,
        source_uv + vec2<f32>(0.0, uv_step.y),
        up_direction,
        camera_distance_meters,
    );
    let tangent_delta_u = (right_direction - left_direction) * PLANET_RADIUS_METERS
        + right_direction * right_height
        - left_direction * left_height;
    let tangent_delta_v = (up_direction - down_direction) * PLANET_RADIUS_METERS
        + up_direction * up_height
        - down_direction * down_height;
    return normalize(cross(tangent_delta_u, tangent_delta_v));
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

fn terrain_material_color(
    outmap: bool,
    source_uv: vec2<f32>,
    macro_height_meters: f32,
    terrain_detail_meters: f32,
) -> vec3<f32> {
    var color = vec3<f32>(0.32, 0.58, 0.74);
    if !outmap {
        return color;
    }

    let biome = sample_biome(source_uv);
    let moisture = sample_moisture(source_uv);
    color = biome_color(biome) * mix(0.88, 1.06, moisture);
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
        terrain_detail_meters / GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS,
        -1.0,
        1.0,
    );
    color *= 1.0 + detail * detail_weight * 0.22;
    return color;
}

fn debug_ocean_albedo() -> vec3<f32> {
    return vec3<f32>(0.02, 0.12, 0.50);
}

fn outmap_ocean_coverage(outmap: bool, height_meters: f32) -> f32 {
    if !outmap {
        return select(0.0, 1.0, height_meters <= 0.0);
    }
    return 1.0 - smoothstep(-80.0, 120.0, height_meters);
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
    let diffuse = vec3<f32>(0.02, 0.12, 0.50)
        * (sky_diffuse + sun_transmittance * (0.4 * SURFACE_SUNLIGHT_SCALE));
    // The Phase 6 cubemap is static. It represents daytime sky reflection, so
    // gate it by direct daylight instead of reflecting a bright blue sky from
    // the fully occluded hemisphere.
    return diffuse + reflected_color * fresnel * daylight
        + sun_transmittance * specular * fresnel * (12.0 * SURFACE_SUNLIGHT_SCALE);
}

fn ocean_with_aerial_perspective(
    direction: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    sun_direction: vec3<f32>,
) -> vec3<f32> {
    let surface = ocean_surface(direction, camera.projection.z);
    let sun_transmittance = surface_direct_sun_transmittance(
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        surface.normal,
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let water_color = ocean_lighting(
        surface.normal,
        camera_relative_view_position,
        sun_transmittance,
        sky_diffuse,
    );
    return aerial_perspective(
        water_color,
        camera_relative_view_position,
        direction,
        surface.vertical_displacement,
    );
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let direction = normalize(input.sphere_direction);
    let source_uv = input.source_uv_offset + input.tile_uv * input.source_uv_scale;
    let outmap = uses_outmap(input.terrain_info);
    let macro_height = macro_terrain_height(outmap, source_uv, direction);
    let base_camera_relative_view_position = input.anchor_view_position
        + planet_to_view(input.anchor_relative_position);
    let camera_distance_meters = length(base_camera_relative_view_position);
    var terrain_detail_meters = 0.0;
    if outmap {
        terrain_detail_meters = global_terrain_detail(direction, camera_distance_meters);
    }
    let detail_weight = smoothstep(100.0, 400.0, macro_height);
    let unscaled_height = macro_height + terrain_detail_meters * detail_weight;
    let height = select(
        unscaled_height,
        macro_height * terrain_settings.outmap_height_scale.x
            + terrain_detail_meters
                * detail_weight
                * terrain_settings.outmap_height_scale.y,
        outmap,
    );
    // Polar ice overrides ocean in the baked biome contract. Lift it just
    // above sea level so the cap remains visible rather than becoming water.
    let ice = outmap && sample_biome(source_uv) == 2u;
    let ocean = macro_height <= 0.0 && !ice;
    let wave_surface = ocean_surface(direction, camera.projection.z);
    let land_height = select(height, max(height, 5.0), ice);
    let surface_height = select(land_height, wave_surface.vertical_displacement, ocean);
    let local_planet_position = input.anchor_relative_position
        + direction * (surface_height - input.skirt_depth_meters)
        + select(vec3<f32>(0.0), wave_surface.horizontal_displacement, ocean);
    let camera_relative_view_position = input.anchor_view_position
        + planet_to_view(local_planet_position);
    var normal = displaced_surface_normal(
        direction,
        source_uv,
        input.source_uv_scale,
        input.terrain_info,
        camera_distance_meters,
    );
    if ocean {
        normal = wave_surface.normal;
    }
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_transmittance = surface_direct_sun_transmittance(
        direction,
        surface_height,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        normal,
        direction,
        surface_height,
        sun_direction,
    );
    let direct_light = max(dot(normal, sun_direction), 0.0);
    let surface_irradiance = sky_diffuse
        + sun_transmittance * direct_light * SURFACE_SUNLIGHT_SCALE;
    var lit_surface_color = terrain_material_color(
        outmap,
        source_uv,
        macro_height,
        terrain_detail_meters,
    ) * surface_irradiance;
    if ice {
        // Keep daylight snow neutral without creating an emissive floor after
        // direct and atmospheric illumination are fully occulted.
        let ice_light_floor = clamp(
            max(max(surface_irradiance.x, surface_irradiance.y), surface_irradiance.z),
            0.0,
            1.0,
        );
        lit_surface_color = max(
            lit_surface_color,
            biome_color(2u) * 0.65 * ice_light_floor,
        );
    }
    var aerial_color = aerial_perspective(
        lit_surface_color,
        camera_relative_view_position,
        direction,
        surface_height,
    );
    if !ocean {
        aerial_color = terrain_distance_fog(
            aerial_color,
            camera_relative_view_position,
            direction,
            surface_height,
        );
    }
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(camera_relative_view_position, 1.0),
        camera_relative_view_position,
        normal,
        aerial_color,
        input.lod_transition,
        direction,
        select(0.0, 1.0, ocean),
        source_uv,
        select(0.0, 1.0, outmap),
        lit_surface_color,
        terrain_detail_meters,
    );
}

fn bayer_dither(fragment_position: vec4<f32>) -> f32 {
    let pattern = array<u32, 16>(
        0u, 8u, 2u, 10u,
        12u, 4u, 14u, 6u,
        3u, 11u, 1u, 9u,
        15u, 7u, 13u, 5u,
    );
    let pixel = vec2<u32>(u32(fragment_position.x), u32(fragment_position.y));
    let index = (pixel.y & 3u) * 4u + (pixel.x & 3u);
    return (f32(pattern[index]) + 0.5) / 16.0;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let transition_progress = input.lod_transition.x;
    let incoming = input.lod_transition.y > 0.5;
    let threshold = bayer_dither(input.position);
    if (incoming && threshold >= transition_progress)
        || (!incoming && threshold < transition_progress)
    {
        discard;
    }
    let direction = normalize(input.surface_direction);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let render_debug_mode = u32(camera.projection.w + 0.5);
    if input.ocean > 0.5 {
        if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
            return vec4<f32>(debug_ocean_albedo(), 1.0);
        }
        let surface = ocean_surface(direction, camera.projection.z);
        let sun_transmittance = surface_direct_sun_transmittance(
            direction,
            surface.vertical_displacement,
            sun_direction,
        );
        let sky_diffuse = sky_diffuse_irradiance(
            surface.normal,
            direction,
            surface.vertical_displacement,
            sun_direction,
        );
        let water_surface_color = ocean_lighting(
            surface.normal,
            input.camera_relative_view_position,
            sun_transmittance,
            sky_diffuse,
        );
        if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
            return vec4<f32>(water_surface_color, 1.0);
        }
        let water_aerial_color = aerial_perspective(
            water_surface_color,
            input.camera_relative_view_position,
            direction,
            surface.vertical_displacement,
        );
        if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
            return vec4<f32>(max(water_aerial_color - water_surface_color, vec3<f32>(0.0)), 1.0);
        }
        return vec4<f32>(water_aerial_color, 1.0);
    }
    let outmap = input.outmap > 0.5;
    let macro_height_meters = macro_terrain_height(outmap, input.source_uv, direction);
    let ocean_coverage = outmap_ocean_coverage(outmap, macro_height_meters);
    let terrain_albedo = terrain_material_color(
        outmap,
        input.source_uv,
        macro_height_meters,
        input.terrain_detail_meters,
    );
    if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
        return vec4<f32>(
            mix(terrain_albedo, debug_ocean_albedo(), ocean_coverage),
            1.0,
        );
    }
    if ocean_coverage <= 0.0 {
        if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
            return vec4<f32>(input.surface_lighting, 1.0);
        }
        if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
            return vec4<f32>(
                max(input.aerial_color - input.surface_lighting, vec3<f32>(0.0)),
                1.0,
            );
        }
        return vec4<f32>(input.aerial_color, 1.0);
    }
    let surface = ocean_surface(direction, camera.projection.z);
    let sun_transmittance = surface_direct_sun_transmittance(
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        surface.normal,
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let water_surface_color = ocean_lighting(
        surface.normal,
        input.camera_relative_view_position,
        sun_transmittance,
        sky_diffuse,
    );
    let water_aerial_color = aerial_perspective(
        water_surface_color,
        input.camera_relative_view_position,
        direction,
        surface.vertical_displacement,
    );
    let surface_color = mix(input.surface_lighting, water_surface_color, ocean_coverage);
    let aerial_color = mix(input.aerial_color, water_aerial_color, ocean_coverage);
    if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
        return vec4<f32>(surface_color, 1.0);
    }
    if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
        return vec4<f32>(max(aerial_color - surface_color, vec3<f32>(0.0)), 1.0);
    }
    return vec4<f32>(
        aerial_color,
        1.0,
    );
}

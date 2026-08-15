const PLANET_RADIUS_METERS: f32 = 4000000.0;
const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 320000.0;
const ATMOSPHERE_VERTICAL_SCALE: f32 = 4.5;
const CLOUD_DIRECT_SUN_SCALE: f32 = 1.0;
const CLOUD_SKY_FILL_SCALE: f32 = 1.0;

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
@group(1) @binding(0)
var atmosphere_transmittance_lut: texture_2d<f32>;
@group(1) @binding(1)
var atmosphere_surface_irradiance_lut: texture_2d<f32>;
@group(1) @binding(2)
var atmosphere_physical_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) face_normal: vec3<f32>,
    @location(2) center_speed: vec4<f32>,
    @location(3) wind_axis: vec4<f32>,
    @location(4) radii_brightness: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) face_normal_planet: vec3<f32>,
    @location(1) @interpolate(flat) cloud_direction: vec3<f32>,
    @location(2) @interpolate(flat) altitude_brightness: vec2<f32>,
}

fn rotate_about_axis(vector: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let sine = sin(angle);
    let cosine = cos(angle);
    return vector * cosine
        + cross(axis, vector) * sine
        + axis * dot(axis, vector) * (1.0 - cosine);
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let wind_axis = normalize(input.wind_axis.xyz);
    let wind_angle = camera.flat_triangle_options.y * input.center_speed.w;
    let center = rotate_about_axis(input.center_speed.xyz, wind_axis, wind_angle);
    let radial = normalize(center);
    let base_along_wind = normalize(cross(wind_axis, radial));
    let base_across_wind = normalize(cross(radial, base_along_wind));
    let roll_sine = sin(input.wind_axis.w);
    let roll_cosine = cos(input.wind_axis.w);
    let along_wind = base_along_wind * roll_cosine + base_across_wind * roll_sine;
    let across_wind = cross(radial, along_wind);
    let radii = input.radii_brightness.xyz;
    let local_offset = along_wind * (input.position.x * radii.x)
        + across_wind * (input.position.y * radii.y)
        + radial * (input.position.z * radii.z);
    let world_position = center + local_offset;
    let camera_position_view = normalize(
        camera.camera_planet_direction_view_altitude.xyz,
    ) * (PLANET_RADIUS_METERS + camera.camera_planet_direction_view_altitude.w);
    let camera_relative_view = planet_to_view(world_position) - camera_position_view;

    // The inverse-transpose of the ellipsoid scale keeps each repeated
    // icosahedron face geometrically correct under its non-uniform puff shape.
    let scaled_normal = input.face_normal / radii;
    let face_normal_planet = normalize(
        along_wind * scaled_normal.x
            + across_wind * scaled_normal.y
            + radial * scaled_normal.z,
    );
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(camera_relative_view, 1.0),
        face_normal_planet,
        radial,
        vec2<f32>(length(center) - PLANET_RADIUS_METERS, input.radii_brightness.w),
    );
}

fn atmosphere_lut_uv(altitude_meters: f32, solar_zenith_cosine: f32) -> vec2<f32> {
    let optical_altitude = max(altitude_meters, 0.0) / ATMOSPHERE_VERTICAL_SCALE;
    return vec2<f32>(
        clamp(solar_zenith_cosine * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(optical_altitude / OPTICAL_ATMOSPHERE_HEIGHT_METERS, 0.0, 1.0)),
    );
}

fn flat_horizon_sun_visibility(altitude_meters: f32, solar_zenith_cosine: f32) -> f32 {
    let radius_meters = PLANET_RADIUS_METERS + max(altitude_meters, 0.0);
    let planet_radius_ratio = PLANET_RADIUS_METERS / radius_meters;
    let horizon_cosine = -sqrt(max(1.0 - planet_radius_ratio * planet_radius_ratio, 0.0));
    // Resolve the solar disc over a narrow interval instead of popping when
    // its centre crosses the altitude-dependent geometric horizon.
    let solar_angular_radius_sine = 0.004625;
    return smoothstep(
        horizon_cosine - solar_angular_radius_sine,
        horizon_cosine + solar_angular_radius_sine,
        solar_zenith_cosine,
    );
}

fn sample_sun_transmittance(altitude_meters: f32, solar_zenith_cosine: f32) -> vec3<f32> {
    let visibility = flat_horizon_sun_visibility(altitude_meters, solar_zenith_cosine);
    let transmittance = textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_physical_sampler,
        atmosphere_lut_uv(altitude_meters, max(solar_zenith_cosine, 0.0)),
        0.0,
    ).rgb;
    return transmittance * visibility;
}

fn sample_sky_irradiance(altitude_meters: f32, solar_zenith_cosine: f32) -> vec3<f32> {
    return textureSampleLevel(
        atmosphere_surface_irradiance_lut,
        atmosphere_physical_sampler,
        atmosphere_lut_uv(altitude_meters, solar_zenith_cosine),
        0.0,
    ).rgb;
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

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.face_normal_planet);
    let cloud_direction = normalize(input.cloud_direction);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_zenith_cosine = dot(cloud_direction, sun_direction);
    let sun_transmittance = sample_sun_transmittance(
        input.altitude_brightness.x,
        solar_zenith_cosine,
    );
    let sky_irradiance = perceptual_physical_sky_radiance(sample_sky_irradiance(
        input.altitude_brightness.x,
        solar_zenith_cosine,
    ));
    let direct_diffuse = max(dot(normal, sun_direction), 0.0);
    let sky_facing = max(dot(normal, cloud_direction), 0.0);
    // A solid droplet body transmits a bounded fraction of direct light to
    // faces turned away from the sun; without this multiple-scattering term a
    // backlit daytime cloud becomes an implausibly black opaque cut-out.
    let direct_response = 0.20 + 0.80 * direct_diffuse;
    let direct_light = sun_transmittance * direct_response * CLOUD_DIRECT_SUN_SCALE;
    // Multiple scattering through a solid water-droplet body partially
    // neutralises the strongly blue incident sky without replacing its hue.
    let sky_luminance = dot(sky_irradiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    let multiply_scattered_sky = mix(vec3<f32>(sky_luminance), sky_irradiance, 0.55);
    let fill_light = multiply_scattered_sky
        * mix(0.55, 1.0, sky_facing)
        * CLOUD_SKY_FILL_SCALE;
    let cloud_albedo = vec3<f32>(0.92) * input.altitude_brightness.y;
    return vec4<f32>(cloud_albedo * max(direct_light + fill_light, vec3<f32>(0.0)), 1.0);
}

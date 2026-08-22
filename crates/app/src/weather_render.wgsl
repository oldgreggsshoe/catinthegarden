const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_VERTICAL_SCALE: f32 = 4.5;
const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 640000.0;

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
var cloud_field_current: texture_cube<f32>;
@group(1) @binding(1)
var cloud_field_previous: texture_cube<f32>;
@group(1) @binding(2)
var cloud_field_sampler: sampler;

struct WeatherRenderUniform {
    blend: f32,
    drift_radians: f32,
    lower_shell_radius_meters: f32,
    upper_shell_radius_meters: f32,
    noise_scale: f32,
    noise_strength: f32,
    _padding: vec2<f32>,
}

@group(1) @binding(3)
var<uniform> weather: WeatherRenderUniform;

@group(2) @binding(0)
var atmosphere_transmittance_lut: texture_2d<f32>;
@group(2) @binding(1)
var atmosphere_irradiance_lut: texture_2d<f32>;
@group(2) @binding(2)
var atmosphere_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) altitude: f32,
    @location(3) @interpolate(flat) shell_index: u32,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let direction = normalize(input.position);
    let shell_radius = select(
        weather.lower_shell_radius_meters,
        weather.upper_shell_radius_meters,
        instance_index == 1u,
    );
    let world_position = direction * shell_radius;
    let camera_position_view = normalize(camera.camera_planet_direction_view_altitude.xyz)
        * (PLANET_RADIUS_METERS + camera.camera_planet_direction_view_altitude.w);
    let camera_relative_view = planet_to_view(world_position) - camera_position_view;
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(camera_relative_view, 1.0),
        direction,
        normalize(input.normal),
        shell_radius - PLANET_RADIUS_METERS,
        instance_index,
    );
}

fn direct_atmosphere_uv(altitude: f32, solar_zenith_cosine: f32) -> vec2<f32> {
    return vec2<f32>(
        clamp(max(solar_zenith_cosine, 0.0) * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(max(altitude, 0.0) / ATMOSPHERE_VERTICAL_SCALE / OPTICAL_ATMOSPHERE_HEIGHT_METERS, 0.0, 1.0)),
    );
}

fn irradiance_atmosphere_uv(altitude: f32, solar_zenith_cosine: f32) -> vec2<f32> {
    return vec2<f32>(
        clamp(solar_zenith_cosine * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(max(altitude, 0.0) / ATMOSPHERE_VERTICAL_SCALE / OPTICAL_ATMOSPHERE_HEIGHT_METERS, 0.0, 1.0)),
    );
}

fn flat_horizon_visibility(altitude: f32, solar_zenith_cosine: f32) -> f32 {
    let radius = PLANET_RADIUS_METERS + max(altitude, 0.0);
    let horizon = -sqrt(max(1.0 - (PLANET_RADIUS_METERS / radius) * (PLANET_RADIUS_METERS / radius), 0.0));
    return smoothstep(horizon - 0.004625, horizon + 0.004625, solar_zenith_cosine);
}

// The terrain depth buffer normally rejects clouds behind the ground, but it
// is not allowed to be the planet's only occluder. A near-plane or coarse-mesh
// hole must not expose the cloud shell on the other side of the world.
fn solid_planet_blocks_cloud(
    camera_position: vec3<f32>,
    cloud_position: vec3<f32>,
) -> bool {
    let segment = cloud_position - camera_position;
    let a = dot(segment, segment);
    let b = 2.0 * dot(camera_position, segment);
    let c = dot(camera_position, camera_position)
        - PLANET_RADIUS_METERS * PLANET_RADIUS_METERS;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant <= 0.0 || a <= 1.0e-6 {
        return false;
    }
    let root = sqrt(discriminant);
    let near_t = (-b - root) / (2.0 * a);
    let far_t = (-b + root) / (2.0 * a);
    return (near_t > 1.0e-5 && near_t < 0.99999)
        || (far_t > 1.0e-5 && far_t < 0.99999);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let camera_position_view = normalize(
        camera.camera_planet_direction_view_altitude.xyz,
    ) * (PLANET_RADIUS_METERS + camera.camera_planet_direction_view_altitude.w);
    let cloud_position_view = planet_to_view(
        normalize(input.direction) * (PLANET_RADIUS_METERS + input.altitude),
    );
    if solid_planet_blocks_cloud(camera_position_view, cloud_position_view) {
        discard;
    }
    let cloud = cloudSample(input.direction, f32(input.shell_index));
    let density = cloud.density;
    // Dense cloud is optically thicker than a linear alpha ramp implies. Keep
    // wisps unchanged, then push only the upper density range toward opacity.
    let alpha = max(density, smoothstep(0.25, 0.85, density));
    if alpha < 0.002 {
        discard;
    }
    let normal = normalize(input.normal);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_zenith_cosine = dot(normalize(input.direction), sun_direction);
    let transmittance = textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_sampler,
        direct_atmosphere_uv(input.altitude, solar_zenith_cosine),
        0.0,
    ).rgb * flat_horizon_visibility(input.altitude, solar_zenith_cosine);
    let irradiance = textureSampleLevel(
        atmosphere_irradiance_lut,
        atmosphere_sampler,
        irradiance_atmosphere_uv(input.altitude, solar_zenith_cosine),
        0.0,
    ).rgb;
    let direct = transmittance * (0.20 + 0.80 * max(dot(normal, sun_direction), 0.0));
    let sky = mix(vec3<f32>(dot(irradiance, vec3<f32>(0.2126, 0.7152, 0.0722))), irradiance, 0.55);
    let lighting = max(direct + sky * (0.55 + 0.45 * max(dot(normal, normalize(input.direction)), 0.0)), vec3<f32>(0.0));
    let fair_weather_albedo = mix(
        vec3<f32>(0.42, 0.46, 0.50),
        vec3<f32>(0.94, 0.96, 1.0),
        clamp(density * 2.0, 0.0, 1.0),
    );
    let storm_weight = smoothstep(0.10, 0.30, cloud.storm)
        * smoothstep(0.30, 0.78, density);
    let storm_darkening = 1.0 - 0.72 * storm_weight;
    let albedo = fair_weather_albedo
        * storm_darkening
        * select(1.0, 0.82, input.shell_index == 1u);
    return vec4<f32>(albedo * lighting, alpha);
}

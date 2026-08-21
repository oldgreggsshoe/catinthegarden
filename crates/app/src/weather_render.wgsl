const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_VERTICAL_SCALE: f32 = 4.5;
const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 320000.0;

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
var cloud_field_current: texture_2d_array<f32>;
@group(1) @binding(1)
var cloud_field_previous: texture_2d_array<f32>;
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

fn rotate_drift(direction: vec3<f32>) -> vec3<f32> {
    let sine = sin(weather.drift_radians);
    let cosine = cos(weather.drift_radians);
    return vec3<f32>(
        direction.x * cosine - direction.z * sine,
        direction.y,
        direction.x * sine + direction.z * cosine,
    );
}

fn hash_noise(point: vec3<f32>) -> f32 {
    return fract(sin(dot(point, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

fn cloud_noise(direction: vec3<f32>, shell_index: u32) -> f32 {
    let scale = weather.noise_scale * select(1.0, 1.7, shell_index == 1u);
    let point = direction * scale;
    let cell = floor(point);
    let local = smoothstep(vec3<f32>(0.0), vec3<f32>(1.0), fract(point));
    let n000 = hash_noise(cell);
    let n100 = hash_noise(cell + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash_noise(cell + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash_noise(cell + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash_noise(cell + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash_noise(cell + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash_noise(cell + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash_noise(cell + vec3<f32>(1.0, 1.0, 1.0));
    let low = mix(mix(n000, n100, local.x), mix(n010, n110, local.x), local.y);
    let high = mix(mix(n001, n101, local.x), mix(n011, n111, local.x), local.y);
    return mix(low, high, local.z) * 2.0 - 1.0;
}

fn flow_warp(direction: vec3<f32>, shell_index: u32) -> vec3<f32> {
    let flow = normalize(vec3<f32>(-direction.z, 0.18, direction.x));
    let phase = weather.drift_radians * select(0.8, 0.45, shell_index == 1u);
    let amount = sin(phase + dot(direction, flow) * 18.0) * 0.018;
    return normalize(direction + flow * amount);
}

fn cube_field_uv(direction: vec3<f32>) -> vec3<f32> {
    let absolute = abs(direction);
    var face = 0.0;
    var u = 0.0;
    var v = 0.0;
    if absolute.x >= absolute.y && absolute.x >= absolute.z {
        let denominator = max(absolute.x, 1.0e-6);
        if direction.x >= 0.0 {
            face = 0.0;
            u = -direction.z / denominator;
            v = direction.y / denominator;
        } else {
            face = 1.0;
            u = direction.z / denominator;
            v = direction.y / denominator;
        }
    } else if absolute.y >= absolute.z {
        let denominator = max(absolute.y, 1.0e-6);
        if direction.y >= 0.0 {
            face = 2.0;
            u = direction.x / denominator;
            v = -direction.z / denominator;
        } else {
            face = 3.0;
            u = direction.x / denominator;
            v = direction.z / denominator;
        }
    } else {
        let denominator = max(absolute.z, 1.0e-6);
        if direction.z >= 0.0 {
            face = 4.0;
            u = direction.x / denominator;
            v = direction.y / denominator;
        } else {
            face = 5.0;
            u = -direction.x / denominator;
            v = direction.y / denominator;
        }
    }
    return vec3<f32>(u * 0.5 + 0.5, v * 0.5 + 0.5, face);
}

fn atmosphere_uv(altitude: f32, solar_zenith_cosine: f32) -> vec2<f32> {
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

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let base_direction = normalize(input.direction);
    let field_direction = flow_warp(rotate_drift(base_direction), input.shell_index);
    let uv = cube_field_uv(field_direction);
    let current = textureSampleLevel(cloud_field_current, cloud_field_sampler, uv.xy, i32(uv.z), 0.0);
    let previous = textureSampleLevel(cloud_field_previous, cloud_field_sampler, uv.xy, i32(uv.z), 0.0);
    let field = mix(previous, current, weather.blend);
    // Humid air is rendered as a faint nascent cloud before condensation has
    // completed, so the first shell is visible immediately while cloud water
    // remains the authoritative opaque/storm contribution.
    let noise = cloud_noise(field_direction, input.shell_index) * weather.noise_strength;
    let humidity_cloud = smoothstep(0.55, 0.82, field.b);
    let shell_cloud = select(field.r, field.r * 0.72, input.shell_index == 1u);
    let density = max(shell_cloud + noise, humidity_cloud * select(0.75, 0.46, input.shell_index == 1u));
    let alpha = smoothstep(0.08, 0.26, density)
        * (0.32 + 0.58 * field.g)
        * select(1.0, 0.62, input.shell_index == 1u);
    if alpha < 0.002 {
        discard;
    }
    let normal = normalize(input.normal);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_zenith_cosine = dot(normalize(input.direction), sun_direction);
    let transmittance = textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_sampler,
        atmosphere_uv(input.altitude, solar_zenith_cosine),
        0.0,
    ).rgb * flat_horizon_visibility(input.altitude, solar_zenith_cosine);
    let irradiance = textureSampleLevel(
        atmosphere_irradiance_lut,
        atmosphere_sampler,
        atmosphere_uv(input.altitude, solar_zenith_cosine),
        0.0,
    ).rgb;
    let direct = transmittance * (0.20 + 0.80 * max(dot(normal, sun_direction), 0.0));
    let sky = mix(vec3<f32>(dot(irradiance, vec3<f32>(0.2126, 0.7152, 0.0722))), irradiance, 0.55);
    let lighting = max(direct + sky * (0.55 + 0.45 * max(dot(normal, normalize(input.direction)), 0.0)), vec3<f32>(0.0));
    let albedo = mix(
        vec3<f32>(0.42, 0.46, 0.50),
        vec3<f32>(0.94, 0.96, 1.0),
        clamp(density * 2.0, 0.0, 1.0),
    ) * select(1.0, 0.82, input.shell_index == 1u);
    return vec4<f32>(albedo * lighting, alpha);
}

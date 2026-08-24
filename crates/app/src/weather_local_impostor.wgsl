const PLANET_RADIUS_METERS: f32 = 4000000.0;

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

const ATMOSPHERE_VERTICAL_SCALE: f32 = 4.5;
const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 640000.0;

@group(2) @binding(0)
var atmosphere_transmittance_lut: texture_2d<f32>;
@group(2) @binding(1)
var atmosphere_irradiance_lut: texture_2d<f32>;
@group(2) @binding(2)
var atmosphere_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) alpha: f32,
    @location(1) colour: vec3<f32>,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

fn hash(value: f32) -> f32 {
    return fract(sin(value * 91.173 + 17.31) * 43758.5453);
}

fn cloud_atmosphere_horizon_cosine(altitude: f32) -> f32 {
    let optical_altitude = max(altitude, 0.0)
        / ATMOSPHERE_VERTICAL_SCALE;
    let radius = PLANET_RADIUS_METERS + optical_altitude;
    return -sqrt(max(
        1.0 - (PLANET_RADIUS_METERS / radius) * (PLANET_RADIUS_METERS / radius),
        0.0,
    ));
}

fn cloud_sun_visibility(altitude: f32, solar_zenith_cosine: f32) -> f32 {
    let horizon = cloud_atmosphere_horizon_cosine(altitude);
    let solar_angular_radius_sine = 0.004625;
    return smoothstep(
        horizon - solar_angular_radius_sine,
        horizon + solar_angular_radius_sine,
        solar_zenith_cosine,
    );
}

fn cloud_atmosphere_uv(altitude: f32, solar_zenith_cosine: f32) -> vec2<f32> {
    // The optical LUT compresses the deliberately thick game atmosphere.
    // Keep the signed low-sun column, but clamp below its optical horizon so
    // elevated puffs retain a warm transmitted edge instead of sampling the
    // solid-planet rows.
    let safe_cosine = max(
        solar_zenith_cosine,
        cloud_atmosphere_horizon_cosine(altitude) + 0.004625,
    );
    return vec2<f32>(
        clamp(safe_cosine * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(
            max(altitude, 0.0)
                / ATMOSPHERE_VERTICAL_SCALE
                / OPTICAL_ATMOSPHERE_HEIGHT_METERS,
            0.0,
            1.0,
        )),
    );
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let id = f32(instance_index);
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex_index % 6u];
    let camera_altitude = max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude;
    let radial = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let tangent_right = normalize(camera.camera_right.xyz);
    let tangent_up = normalize(camera.camera_up.xyz);
    let tangent_forward = normalize(camera.camera_forward.xyz);
    let drift = weather.drift_radians * 37.0;
    let lateral = (hash(id + 1.0 + drift) * 2.0 - 1.0) * 6500.0;
    let depth = (hash(id + 2.0 - drift) * 2.0 - 1.0) * 6500.0;
    let direction = normalize(
        radial + tangent_right * (lateral / camera_radius)
            + tangent_forward * (depth / camera_radius),
    );
    let puff_altitude = clamp(
        camera_altitude + 1200.0 + hash(id + 5.0 + drift) * 7000.0,
        1200.0,
        18000.0,
    );
    let centre = direction * (PLANET_RADIUS_METERS + puff_altitude);
    let size = 350.0 + hash(id + 8.0 - drift) * 850.0;
    let world_position = centre
        + tangent_right * corner.x * size
        + tangent_up * corner.y * size * 0.45;
    let camera_position_view = radial * camera_radius;
    let view_position = planet_to_view(world_position) - camera_position_view;

    let cloud = cloudSample(direction, 0.0);
    let coverage = cloud.density;
    let storm = smoothstep(0.10, 0.55, cloud.storm);
    let alpha = smoothstep(0.12, 0.38, coverage) * mix(0.06, 0.24, storm);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_zenith_cosine = dot(direction, sun_direction);
    let sun_transmittance = textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_sampler,
        cloud_atmosphere_uv(puff_altitude, solar_zenith_cosine),
        0.0,
    ).rgb * cloud_sun_visibility(puff_altitude, solar_zenith_cosine);
    let irradiance = textureSampleLevel(
        atmosphere_irradiance_lut,
        atmosphere_sampler,
        cloud_atmosphere_uv(puff_altitude, solar_zenith_cosine),
        0.0,
    ).rgb;
    let sky = mix(
        vec3<f32>(dot(irradiance, vec3<f32>(0.2126, 0.7152, 0.0722))),
        irradiance,
        0.55,
    );
    let direct = sun_transmittance
        * (0.20 + 0.80 * max(solar_zenith_cosine, 0.0))
        * 2.0;
    let low_sun_amount = 1.0 - smoothstep(0.0, 0.35, solar_zenith_cosine);
    let lighting = max(
        direct + sky * mix(0.45, 0.16, low_sun_amount),
        vec3<f32>(0.0),
    );
    let colour = mix(vec3<f32>(0.86, 0.88, 0.90), vec3<f32>(0.38, 0.42, 0.47), storm)
        * lighting;
    let visible_alpha = select(alpha, 0.0, camera_altitude > 22000.0);
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(view_position, 1.0),
        visible_alpha,
        colour,
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.alpha < 0.002 {
        discard;
    }
    return vec4<f32>(input.colour, input.alpha);
}

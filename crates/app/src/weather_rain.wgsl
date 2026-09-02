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

@group(1) @binding(4)
var weather_surface_current: texture_cube<f32>;
@group(1) @binding(5)
var weather_surface_previous: texture_cube<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) precipitation: f32,
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

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let particle = vertex_index / 2u;
    let endpoint = vertex_index % 2u;
    let id = f32(particle);
    let x = (hash(id + 1.0) * 2.0 - 1.0) * 48.0;
    let y = (hash(id + 9.0) * 2.0 - 1.0) * 48.0;
    let camera_altitude = max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let radius = PLANET_RADIUS_METERS + camera_altitude;
    let radial = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let tangent_right = normalize(camera.camera_right.xyz);
    let tangent_up = normalize(camera.camera_up.xyz);
    let direction = normalize(radial + tangent_right * (x / radius) + tangent_up * (y / radius));
    let top = camera_altitude + 35.0 + hash(id + 21.0) * 80.0;
    let length = 8.0 + hash(id + 31.0) * 8.0;
    let altitude = top - select(0.0, length, endpoint == 1u);
    let world_position = direction * (PLANET_RADIUS_METERS + altitude);
    let camera_position_view = radial * radius;
    let view_position = planet_to_view(world_position) - camera_position_view;
    let current = textureSampleLevel(cloud_field_current, cloud_field_sampler, direction, 0.0).a;
    let previous = textureSampleLevel(cloud_field_previous, cloud_field_sampler, direction, 0.0).a;
    let precipitation = mix(previous, current, weather.blend);
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(view_position, 1.0),
        precipitation,
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if camera.camera_planet_direction_view_altitude.w > 20000.0 {
        discard;
    }
    let alpha = smoothstep(0.02, 0.10, input.precipitation) * 0.32;
    if alpha < 0.002 {
        discard;
    }
    return vec4<f32>(0.62, 0.70, 0.78, alpha);
}

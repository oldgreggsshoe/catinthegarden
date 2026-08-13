const SKY_VIEW_SAMPLE_COUNT: u32 = 20u;

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
var transmittance_lut: texture_2d<f32>;
@group(1) @binding(1)
var multiple_scattering_lut: texture_2d<f32>;
@group(1) @binding(2)
var atmosphere_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), position * 0.5 + 0.5);
}

fn sky_basis(up: vec3<f32>, sun: vec3<f32>) -> mat3x3<f32> {
    var toward_sun = sun - up * dot(up, sun);
    if dot(toward_sun, toward_sun) < 1.0e-6 {
        toward_sun = normalize(camera.camera_right.xyz);
    } else {
        toward_sun = normalize(toward_sun);
    }
    let side = normalize(cross(up, toward_sun));
    return mat3x3<f32>(toward_sun, side, up);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let up = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let sun = normalize(camera.sun_direction_view.xyz);
    let basis = sky_basis(up, sun);
    let view_zenith_cosine = clamp(1.0 - 2.0 * input.uv.y, -1.0, 1.0);
    let azimuth = (input.uv.x * 2.0 - 1.0) * PI;
    let horizontal_length = sqrt(max(1.0 - view_zenith_cosine * view_zenith_cosine, 0.0));
    let ray = basis * vec3<f32>(
        cos(azimuth) * horizontal_length,
        sin(azimuth) * horizontal_length,
        view_zenith_cosine,
    );

    let optical_camera_altitude =
        camera.camera_planet_direction_view_altitude.w / ATMOSPHERE_VERTICAL_SCALE;
    let camera_radius = OPTICAL_PLANET_RADIUS_METERS + optical_camera_altitude;
    let camera_position = up * camera_radius;
    let atmosphere_interval = sphere_interval(
        camera_position,
        ray,
        OPTICAL_ATMOSPHERE_RADIUS_METERS,
    );
    let start_distance = max(atmosphere_interval.x, 0.0);
    var end_distance = atmosphere_interval.y;
    let ground_distance = nearest_positive_sphere_distance(
        camera_position,
        ray,
        OPTICAL_PLANET_RADIUS_METERS,
    );
    if ground_distance > start_distance && ground_distance < end_distance {
        end_distance = ground_distance;
    }
    if end_distance <= start_distance {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let ray_length = end_distance - start_distance;
    var view_transmittance = vec3<f32>(1.0);
    var luminance = vec3<f32>(0.0);
    for (var index = 0u; index < SKY_VIEW_SAMPLE_COUNT; index += 1u) {
        let fraction = (f32(index) + 0.5) / f32(SKY_VIEW_SAMPLE_COUNT);
        let shaped_fraction = fraction * fraction * (3.0 - 2.0 * fraction);
        let previous_fraction = f32(index) / f32(SKY_VIEW_SAMPLE_COUNT);
        let previous_shaped = previous_fraction * previous_fraction
            * (3.0 - 2.0 * previous_fraction);
        let next_fraction = f32(index + 1u) / f32(SKY_VIEW_SAMPLE_COUNT);
        let next_shaped = next_fraction * next_fraction
            * (3.0 - 2.0 * next_fraction);
        let segment_length = (next_shaped - previous_shaped) * ray_length;
        let sample_position = camera_position
            + ray * (start_distance + shaped_fraction * ray_length);
        let sample_radius = length(sample_position);
        let sample_direction = sample_position / sample_radius;
        let sample_altitude = sample_radius - OPTICAL_PLANET_RADIUS_METERS;
        let extinction = medium_extinction(sample_altitude);
        let scattering = medium_scattering(sample_altitude);
        let segment_transmittance = exp(-extinction * segment_length);
        let integrated_segment = (vec3<f32>(1.0) - segment_transmittance)
            / max(extinction, vec3<f32>(1.0e-9));
        let solar_zenith_cosine = dot(sample_direction, sun);
        let direct_sun = sample_transmittance_lut(
            transmittance_lut,
            atmosphere_sampler,
            sample_altitude,
            solar_zenith_cosine,
        );
        let multiple_scattering = sample_multiple_scattering_lut(
            multiple_scattering_lut,
            atmosphere_sampler,
            sample_altitude,
            solar_zenith_cosine,
        );
        let source = direct_sun
                * phase_scattering(sample_altitude, dot(ray, sun))
                * SOLAR_LUMINANCE
            + multiple_scattering * scattering;
        luminance += view_transmittance * source * integrated_segment;
        view_transmittance *= segment_transmittance;
    }
    return vec4<f32>(max(luminance, vec3<f32>(0.0)), 1.0);
}

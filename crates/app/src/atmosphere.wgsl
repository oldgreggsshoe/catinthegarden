const PI: f32 = 3.141592653589793;
const ORBITAL_GEOMETRY_BLEND_START_METERS: f32 = 200000.0;
const ORBITAL_GEOMETRY_BLEND_END_METERS: f32 = 400000.0;
const PLANET_RADIUS_METERS: f32 = 4000000.0;
const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 160000.0;
const ORBITAL_ATMOSPHERE_LUT_V: f32 = 0.72;
const ORBITAL_GROUND_LUT_V: f32 = 0.88;

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
var sky_view_lut: texture_2d<f32>;
@group(1) @binding(1)
var sky_view_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
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

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let view = vec3<f32>(
        ndc.x * camera.projection.x * camera.projection.y,
        ndc.y * camera.projection.y,
        -1.0,
    );
    return normalize(view);
}

fn sphere_horizon_cosine(camera_radius: f32, sphere_radius: f32) -> f32 {
    let radius_ratio = clamp(
        sphere_radius / max(camera_radius, sphere_radius),
        0.0,
        1.0,
    );
    return -sqrt(max(1.0 - radius_ratio * radius_ratio, 0.0));
}

fn sky_view_v_from_zenith_cosine(
    zenith_cosine: f32,
    camera_radius: f32,
    camera_altitude: f32,
) -> f32 {
    let atmosphere_horizon = sphere_horizon_cosine(
        camera_radius,
        PLANET_RADIUS_METERS + OPTICAL_ATMOSPHERE_HEIGHT_METERS,
    );
    let ground_horizon = sphere_horizon_cosine(camera_radius, PLANET_RADIUS_METERS);
    let orbital_amount = smoothstep(
        ORBITAL_GEOMETRY_BLEND_START_METERS,
        ORBITAL_GEOMETRY_BLEND_END_METERS,
        camera_altitude,
    );
    let atmosphere_v = mix(
        (1.0 - atmosphere_horizon) * 0.5,
        ORBITAL_ATMOSPHERE_LUT_V,
        orbital_amount,
    );
    let ground_v = mix(
        (1.0 - ground_horizon) * 0.5,
        ORBITAL_GROUND_LUT_V,
        orbital_amount,
    );
    if zenith_cosine >= atmosphere_horizon {
        return atmosphere_v * (1.0 - zenith_cosine)
            / max(1.0 - atmosphere_horizon, 1.0e-6);
    }
    if zenith_cosine >= ground_horizon {
        return mix(
            atmosphere_v,
            ground_v,
            (atmosphere_horizon - zenith_cosine)
                / max(atmosphere_horizon - ground_horizon, 1.0e-6),
        );
    }
    return mix(
        ground_v,
        1.0,
        (ground_horizon - zenith_cosine) / max(ground_horizon + 1.0, 1.0e-6),
    );
}

fn sky_view_uv(ray: vec3<f32>) -> vec2<f32> {
    let up = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let sun = normalize(camera.sun_direction_view.xyz);
    var toward_sun = sun - up * dot(up, sun);
    if dot(toward_sun, toward_sun) < 1.0e-6 {
        toward_sun = normalize(camera.camera_right.xyz);
    } else {
        toward_sun = normalize(toward_sun);
    }
    let side = normalize(cross(up, toward_sun));
    let view_zenith_cosine = clamp(dot(ray, up), -1.0, 1.0);
    let camera_altitude = camera.camera_planet_direction_view_altitude.w;
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude;
    let horizontal = ray - up * view_zenith_cosine;
    var azimuth = 0.0;
    if dot(horizontal, horizontal) > 1.0e-8 {
        let horizontal_direction = normalize(horizontal);
        azimuth = atan2(
            dot(horizontal_direction, side),
            dot(horizontal_direction, toward_sun),
        );
    }
    return vec2<f32>(
        fract(azimuth / (2.0 * PI) + 0.5),
        clamp(
            sky_view_v_from_zenith_cosine(
                view_zenith_cosine,
                camera_radius,
                camera_altitude,
            ),
            0.0,
            1.0,
        ),
    );
}

fn perceptual_sky_radiance(radiance: vec3<f32>) -> vec3<f32> {
    // A fixed display exposure cannot show both daylight and the physically
    // much dimmer nautical-twilight sky. Compress luminance monotonically while
    // preserving the LUT chromaticity exactly. This is independent of clock,
    // sun angle, and hue, so it cannot introduce a hand-authored colour stage
    // or a brightness reversal.
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
    let ray = view_direction(input.ndc);
    let radiance = textureSample(
        sky_view_lut,
        sky_view_sampler,
        sky_view_uv(ray),
    ).rgb;
    // The perceptual lift is needed to retain dim twilight for a surface
    // observer, but in space it turns extremely thin upper air into an opaque
    // blue shell. Fade only that presentation lift above all authored terrain;
    // the physical sky-view radiance and world-space shell remain unchanged.
    let orbital_blend = smoothstep(
        ORBITAL_GEOMETRY_BLEND_START_METERS,
        ORBITAL_GEOMETRY_BLEND_END_METERS,
        camera.camera_planet_direction_view_altitude.w,
    );
    return vec4<f32>(
        mix(perceptual_sky_radiance(radiance), radiance, orbital_blend),
        1.0,
    );
}

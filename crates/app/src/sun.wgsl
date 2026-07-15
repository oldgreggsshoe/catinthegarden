const PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS: f32 = 0.004625;
// Keep the visual sun and its corona at one tenth of their former diameter.
const VISUAL_SUN_SIZE_SCALE: f32 = 0.3;
const SUN_ANGULAR_RADIUS_RADIANS: f32 = PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS * VISUAL_SUN_SIZE_SCALE;
// The core tones to white under ACES, so apparent glare comes from a broad
// HDR corona rather than increasing only the already-clipped disc centre.
const SUN_HALO_RADIUS_SCALE: f32 = 8.0;
// This multiplier belongs only to the camera-facing HDR disc.  Terrain,
// ocean, and atmosphere lighting use their own physical solar radiance.
const SUN_VISUAL_RADIANCE_SCALE: f32 = 5.0;
const SUN_CORE_RADIANCE: vec3<f32> = vec3<f32>(72.0, 65.0, 52.0);
const SUN_HALO_RADIANCE: vec3<f32> = vec3<f32>(6.0, 5.5, 4.5);

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

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let horizontal = ndc.x * camera.projection.x * camera.projection.y;
    let vertical = ndc.y * camera.projection.y;
    return normalize(vec3<f32>(horizontal, vertical, -1.0));
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
    let sun = normalize(camera.sun_direction_view.xyz);
    let alignment = clamp(dot(ray, sun), -1.0, 1.0);
    let angular_distance = atan2(length(cross(ray, sun)), alignment);
    let normalized_distance = angular_distance / SUN_ANGULAR_RADIUS_RADIANS;
    if normalized_distance > SUN_HALO_RADIUS_SCALE {
        discard;
    }
    let disc_coverage = 1.0 - smoothstep(0.92, 1.0, normalized_distance);
    let limb_darkening = 1.0 - 0.25 * min(normalized_distance, 1.0);
    let halo = pow(1.0 - normalized_distance / SUN_HALO_RADIUS_SCALE, 2.0);
    let radiance = SUN_VISUAL_RADIANCE_SCALE
        * (SUN_CORE_RADIANCE * disc_coverage * limb_darkening + SUN_HALO_RADIANCE * halo);
    return vec4<f32>(radiance, 1.0);
}

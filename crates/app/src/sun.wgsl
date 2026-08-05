const PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS: f32 = 0.004625;
// Use the real solar angular diameter (~0.53 degrees) for near-surface views.
// The previous 0.159 degree presentation made the sun read like a distant
// star rather than the disk seen in an Earth-sky photograph.
const VISUAL_SUN_SIZE_SCALE: f32 = 1.0;
const SUN_ANGULAR_RADIUS_RADIANS: f32 = PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS * VISUAL_SUN_SIZE_SCALE;
// A compact, soft corona gives the camera-like glow seen around a bright sun
// without turning the whole sky into a white disk.
const SUN_HALO_RADIUS_SCALE: f32 = 6.5;
const SUN_INNER_GLARE_RADIUS_SCALE: f32 = 2.5;
// This multiplier belongs only to the camera-facing HDR disc.  Terrain,
// ocean, and atmosphere lighting use their own physical solar radiance.
const SUN_VISUAL_RADIANCE_SCALE: f32 = 5.0;
const SUN_CORE_RADIANCE: vec3<f32> = vec3<f32>(72.0, 65.0, 52.0);
const SUN_HALO_RADIANCE: vec3<f32> = vec3<f32>(10.0, 7.0, 3.5);
const SUN_GLARE_RADIANCE: vec3<f32> = vec3<f32>(8.0, 5.5, 2.5);

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

fn sun_disc_tint(solar_elevation: f32) -> vec3<f32> {
    // The atmosphere shader carries the detailed path extinction.  This
    // bounded camera-only tint keeps the visible disk itself from remaining
    // white at the horizon, matching the yellow/orange/red progression in
    // outdoor photographs without double-counting terrain lighting.
    let low_sun = 1.0 - smoothstep(-0.02, 0.30, solar_elevation);
    return mix(
        vec3<f32>(1.0, 0.96, 0.88),
        vec3<f32>(1.0, 0.48, 0.16),
        low_sun,
    );
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
    let halo = pow(max(1.0 - normalized_distance / SUN_HALO_RADIUS_SCALE, 0.0), 2.5);
    let inner_glare = pow(
        max(1.0 - normalized_distance / SUN_INNER_GLARE_RADIUS_SCALE, 0.0),
        2.0,
    );
    let solar_elevation = dot(
        normalize(camera.camera_planet_direction_view_altitude.xyz),
        normalize(camera.sun_direction_view.xyz),
    );
    let tint = sun_disc_tint(solar_elevation);
    // Keep the clipped photographic core close to white while allowing the
    // corona and inner glare to carry the visible low-sun colour.
    let core_tint = mix(vec3<f32>(1.0), tint, 0.25);
    let halo_tint = mix(vec3<f32>(1.0), tint, 0.85);
    let radiance = SUN_VISUAL_RADIANCE_SCALE
        * (core_tint * SUN_CORE_RADIANCE * disc_coverage * limb_darkening
            + halo_tint * SUN_HALO_RADIANCE * halo
            + halo_tint * SUN_GLARE_RADIANCE * inner_glare);
    return vec4<f32>(radiance, 1.0);
}

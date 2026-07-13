const SUN_ANGULAR_RADIUS_RADIANS: f32 = 0.004625;
const SUN_RADIANCE: vec3<f32> = vec3<f32>(48.0, 43.0, 35.0);

struct Camera {
    view_projection: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_altitude: vec4<f32>,
    sun_direction: vec4<f32>,
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
    return normalize(camera.camera_forward.xyz + camera.camera_right.xyz * horizontal
        + camera.camera_up.xyz * vertical);
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
    let alignment = dot(view_direction(input.ndc), normalize(camera.sun_direction.xyz));
    let edge = cos(SUN_ANGULAR_RADIUS_RADIANS);
    if alignment < edge {
        discard;
    }
    let normalized_disc_radius = clamp((1.0 - alignment) / (1.0 - edge), 0.0, 1.0);
    let limb_darkening = 1.0 - 0.25 * normalized_disc_radius;
    return vec4<f32>(SUN_RADIANCE * limb_darkening, 1.0);
}

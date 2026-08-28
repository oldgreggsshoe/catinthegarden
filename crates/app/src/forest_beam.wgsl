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

struct ForestUniform {
    camera_planet_position: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> forest: ForestUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) solar_elevation: f32,
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
    let up = normalize(input.position);
    let view_position = planet_to_view(input.position - forest.camera_planet_position.xyz);
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(view_position, 1.0),
        input.uv,
        dot(up, normalize(camera.sun_direction.xyz)),
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let edge = smoothstep(0.0, 0.24, input.uv.x)
        * (1.0 - smoothstep(0.76, 1.0, input.uv.x));
    let height_fade = 1.0 - 0.35 * smoothstep(0.15, 1.0, input.uv.y);
    let sun = 0.10 + 0.90 * smoothstep(-0.12, 0.18, input.solar_elevation);
    let alpha = edge * height_fade * 0.16 * sun;
    if alpha < 0.002 {
        discard;
    }
    let colour = vec3<f32>(1.0, 0.86, 0.62) * (0.75 + sun * 0.75);
    return vec4<f32>(colour, alpha);
}

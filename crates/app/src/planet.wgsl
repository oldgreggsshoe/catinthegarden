struct Camera {
    view_projection: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    @location(0) camera_relative_position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) camera_relative_position: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    return VertexOutput(
        camera.view_projection * vec4<f32>(input.camera_relative_position, 1.0),
        input.camera_relative_position,
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let face_normal = normalize(cross(dpdx(input.camera_relative_position), dpdy(input.camera_relative_position)));
    let light_direction = normalize(vec3<f32>(0.4, 0.7, 0.6));
    let light = max(dot(face_normal, light_direction), 0.14);
    let base_color = vec3<f32>(0.32, 0.58, 0.74);
    return vec4<f32>(base_color * light, 1.0);
}

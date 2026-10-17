// Same camera prefix and flat sunlight/hemisphere convention as village.wgsl.
// Only a camera bind group: no terrain, weather or road-mask texture lookups.
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
@group(0) @binding(0) var<uniform> camera: Camera;

struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec3<f32>,
    @location(3) origin: vec3<f32>,
    @location(4) across: vec3<f32>,
    @location(5) forward: vec3<f32>,
    @location(6) up: vec3<f32>,
}
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) normal: vec3<f32>,
    @location(1) @interpolate(flat) colour: vec3<f32>,
    @location(2) @interpolate(flat) up: vec3<f32>,
}

@vertex
fn vs_main(input: Input) -> Output {
    let relative = input.origin + input.across * input.position.x
        + input.forward * input.position.y + input.up * input.position.z;
    let view = vec3<f32>(dot(relative, camera.camera_right.xyz),
        dot(relative, camera.camera_up.xyz), -dot(relative, camera.camera_forward.xyz));
    var output: Output;
    output.position = camera.projection_matrix * vec4<f32>(view, 1.0);
    output.normal = normalize(input.across * input.normal.x
        + input.forward * input.normal.y + input.up * input.normal.z);
    output.colour = input.colour;
    output.up = input.up;
    return output;
}

@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    let sun = normalize(camera.sun_direction.xyz);
    let elevation = clamp(dot(sun, input.up), 0.0, 1.0);
    let direct = vec3<f32>(1.9, 1.78, 1.6) * elevation * max(dot(input.normal, sun), 0.0);
    let ambient = mix(vec3<f32>(0.05, 0.055, 0.06), vec3<f32>(0.24, 0.29, 0.36),
        0.5 + 0.5 * dot(input.normal, input.up)) * (0.25 + 0.75 * elevation);
    return vec4<f32>(input.colour * (direct + ambient), 1.0);
}

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
    @location(0) direction_and_base_radius: vec4<f32>,
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
    let up = normalize(input.direction_and_base_radius.xyz);
    let bottom = up * input.direction_and_base_radius.w;
    let top = up * 6880000.0;
    let bottom_view = planet_to_view(bottom - forest.camera_planet_position.xyz);
    let top_view = planet_to_view(top - forest.camera_planet_position.xyz);
    let bottom_clip = camera.projection_matrix * vec4<f32>(bottom_view, 1.0);
    let top_clip = camera.projection_matrix * vec4<f32>(top_view, 1.0);
    let bottom_w = select(bottom_clip.w, 1.0e-4, abs(bottom_clip.w) < 1.0e-4);
    let top_w = select(top_clip.w, 1.0e-4, abs(top_clip.w) < 1.0e-4);
    let bottom_ndc = bottom_clip.xy / bottom_w;
    let top_ndc = top_clip.xy / top_w;
    let screen_aspect = vec2<f32>(max(camera.projection.x, 1.0e-4), 1.0);
    var line_screen = (top_ndc - bottom_ndc) * screen_aspect;
    if dot(line_screen, line_screen) < 1.0e-8 {
        line_screen = vec2<f32>(0.0, 1.0);
    }
    let line_direction = normalize(line_screen);
    let perpendicular_screen = vec2<f32>(-line_direction.y, line_direction.x);
    let side = input.uv.x * 2.0 - 1.0;
    let offset_ndc = perpendicular_screen / screen_aspect * (side * 0.0075);
    var clip_position = mix(bottom_clip, top_clip, input.uv.y);
    clip_position.x += offset_ndc.x * clip_position.w;
    clip_position.y += offset_ndc.y * clip_position.w;
    return VertexOutput(
        clip_position,
        input.uv,
        dot(up, normalize(camera.sun_direction.xyz)),
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let edge = smoothstep(0.0, 0.24, input.uv.x)
        * (1.0 - smoothstep(0.76, 1.0, input.uv.x));
    let height_fade = 1.0 - 0.35 * smoothstep(0.15, 1.0, input.uv.y);
    let sun = smoothstep(-0.12, 0.18, input.solar_elevation);
    let alpha = edge * height_fade * 0.18;
    if alpha < 0.002 {
        discard;
    }
    let colour = vec3<f32>(1.0, 0.86, 0.62) * (0.75 + sun * 0.75);
    return vec4<f32>(colour, alpha);
}

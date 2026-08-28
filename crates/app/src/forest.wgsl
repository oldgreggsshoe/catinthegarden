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

struct ForestUniform {
    camera_planet_position: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> forest: ForestUniform;

struct VertexInput {
    @location(0) centre_and_height: vec4<f32>,
    @location(1) width_shade_kind_seed: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) colour_and_kind: vec4<f32>,
    @location(2) @interpolate(flat) seed: f32,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

@vertex
fn vs_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex_index];
    let centre = input.centre_and_height.xyz;
    let height = input.centre_and_height.w;
    let width = input.width_shade_kind_seed.x;
    let shade = input.width_shade_kind_seed.y;
    let kind = input.width_shade_kind_seed.z;
    let up = normalize(centre);
    let to_camera = forest.camera_planet_position.xyz - centre;
    var right = cross(up, to_camera);
    if dot(right, right) < 1.0e-4 {
        right = cross(up, camera.camera_forward.xyz);
    }
    right = normalize(right);
    let world_position = centre
        + right * corner.x * width * 0.5
        + up * corner.y * height;
    let view_position = planet_to_view(world_position - forest.camera_planet_position.xyz);
    let sun_amount = max(dot(up, normalize(camera.sun_direction.xyz)), 0.0);
    let lighting = 0.36 + sun_amount * 1.24;
    let broadleaf = srgb_to_linear(vec3<f32>(0.10, 0.34, 0.12));
    let conifer = srgb_to_linear(vec3<f32>(0.07, 0.25, 0.10));
    let colour = mix(broadleaf, conifer, kind) * lighting * shade;
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(view_position, 1.0),
        vec2<f32>(corner.x * 0.5 + 0.5, corner.y),
        vec4<f32>(colour, kind),
        input.width_shade_kind_seed.w,
    );
}

fn circle(point: vec2<f32>, centre: vec2<f32>, radius: vec2<f32>) -> bool {
    let local = (point - centre) / radius;
    return dot(local, local) <= 1.0;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let point = vec2<f32>(input.uv.x * 2.0 - 1.0, input.uv.y);
    let trunk_half_width = 0.075 + input.seed * 0.025;
    let trunk = abs(point.x) < trunk_half_width && point.y < 0.38;
    var canopy = false;
    if input.colour_and_kind.w < 0.5 {
        canopy = circle(point, vec2<f32>(-0.20, 0.60), vec2<f32>(0.43, 0.31))
            || circle(point, vec2<f32>(0.20, 0.61), vec2<f32>(0.45, 0.33))
            || circle(point, vec2<f32>(0.0, 0.79), vec2<f32>(0.48, 0.30));
    } else {
        let crown_width = (1.0 - point.y) * 0.82
            + 0.08 * sin(point.y * 43.0 + input.seed * 19.0);
        canopy = point.y > 0.17 && point.y < 0.98 && abs(point.x) < crown_width;
    }
    if !trunk && !canopy {
        discard;
    }
    let trunk_colour = srgb_to_linear(vec3<f32>(0.22, 0.12, 0.055));
    let colour = select(trunk_colour * 0.75, input.colour_and_kind.rgb, canopy);
    return vec4<f32>(colour, 1.0);
}

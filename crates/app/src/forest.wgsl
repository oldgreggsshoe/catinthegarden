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

@group(2) @binding(0)
var cloud_field_current: texture_cube<f32>;

@group(2) @binding(1)
var cloud_field_previous: texture_cube<f32>;

@group(2) @binding(2)
var cloud_field_sampler: sampler;

struct WeatherRenderUniform {
    blend: f32,
    drift_radians: f32,
    lower_shell_radius_meters: f32,
    upper_shell_radius_meters: f32,
    noise_scale: f32,
    noise_strength: f32,
    _padding: vec2<f32>,
}

@group(2) @binding(3)
var<uniform> weather: WeatherRenderUniform;

struct VertexInput {
    @location(0) centre_and_height: vec4<f32>,
    @location(1) width_shade_kind_seed: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) colour_and_kind: vec4<f32>,
    @location(2) @interpolate(flat) seed: f32,
    @location(3) @interpolate(flat) lighting: f32,
    @location(4) @interpolate(flat) valid: f32,
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

fn cloud_shadow_density_at_shell(
    surface_position: vec3<f32>,
    sun_direction: vec3<f32>,
    shell_radius: f32,
    shell_index: f32,
) -> f32 {
    let surface_radius_squared = dot(surface_position, surface_position);
    if surface_radius_squared >= shell_radius * shell_radius {
        return 0.0;
    }
    let ray_offset = dot(surface_position, sun_direction);
    let discriminant = ray_offset * ray_offset
        - (surface_radius_squared - shell_radius * shell_radius);
    if discriminant <= 0.0 {
        return 0.0;
    }
    let distance = -ray_offset + sqrt(discriminant);
    if distance <= 0.0 {
        return 0.0;
    }
    let shadow_position = surface_position + sun_direction * distance;
    return cloudDensityWithOctaves(normalize(shadow_position), shell_index, 3u);
}

fn cloud_shadow_visibility(
    surface_direction: vec3<f32>,
    surface_height: f32,
    sun_direction: vec3<f32>,
) -> f32 {
    let surface_position = normalize(surface_direction)
        * (PLANET_RADIUS_METERS + max(surface_height, 0.0));
    let lower_density = cloud_shadow_density_at_shell(
        surface_position,
        sun_direction,
        weather.lower_shell_radius_meters,
        0.0,
    );
    let upper_density = cloud_shadow_density_at_shell(
        surface_position,
        sun_direction,
        weather.upper_shell_radius_meters,
        1.0,
    );
    let combined_density = 1.0
        - (1.0 - clamp(lower_density, 0.0, 1.0))
            * (1.0 - clamp(upper_density, 0.0, 1.0));
    let posterized_density = floor(combined_density * 4.0 + 0.5) / 4.0;
    return 1.0 - posterized_density * 0.88;
}

fn tree_lighting(solar_elevation_cosine: f32, cloud_visibility: f32) -> f32 {
    let direct = max(solar_elevation_cosine, 0.0) * 1.24;
    let sky_ambient = smoothstep(-0.18, 0.02, solar_elevation_cosine) * 0.36;
    return direct * cloud_visibility + sky_ambient;
}

@vertex
fn vs_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let height = input.centre_and_height.w;
    if height <= 0.0 {
        return VertexOutput(
            vec4<f32>(2.0, 2.0, 0.0, 1.0),
            vec2<f32>(0.0),
            vec4<f32>(0.0),
            0.0,
            0.0,
            0.0,
        );
    }
    // One oversized triangle covers the same unit billboard rectangle as two
    // triangles. The fragment silhouettes already discard outside the tree,
    // so this halves transform and lighting work without changing its shape.
    let corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(3.0, 0.0), vec2<f32>(-1.0, 2.0),
    );
    let corner = corners[vertex_index];
    let centre = input.centre_and_height.xyz;
    let width = input.width_shade_kind_seed.x;
    let shade = input.width_shade_kind_seed.y;
    let kind = input.width_shade_kind_seed.z;
    let species_kind = kind - floor(kind * 0.5) * 2.0;
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
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_elevation_cosine = dot(up, sun_direction);
    var cloud_visibility = 1.0;
    if solar_elevation_cosine > 0.0 {
        cloud_visibility = cloud_shadow_visibility(
            up,
            length(centre) - PLANET_RADIUS_METERS,
            sun_direction,
        );
    }
    let lighting = tree_lighting(solar_elevation_cosine, cloud_visibility) * shade;
    let broadleaf = srgb_to_linear(vec3<f32>(0.10, 0.34, 0.12));
    let conifer = srgb_to_linear(vec3<f32>(0.07, 0.25, 0.10));
    let colour = mix(broadleaf, conifer, species_kind) * lighting;
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(view_position, 1.0),
        vec2<f32>(corner.x * 0.5 + 0.5, corner.y),
        vec4<f32>(colour, kind),
        input.width_shade_kind_seed.w,
        lighting,
        1.0,
    );
}

fn circle(point: vec2<f32>, centre: vec2<f32>, radius: vec2<f32>) -> bool {
    let local = (point - centre) / radius;
    return dot(local, local) <= 1.0;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.valid < 0.5 {
        discard;
    }
    let point = vec2<f32>(input.uv.x * 2.0 - 1.0, input.uv.y);
    let trunk_half_width = 0.075 + input.seed * 0.025;
    let proxy = input.colour_and_kind.w >= 2.0;
    let species_kind = input.colour_and_kind.w
        - floor(input.colour_and_kind.w * 0.5) * 2.0;
    var trunk = abs(point.x) < trunk_half_width && point.y < 0.38;
    var canopy = false;
    if proxy {
        trunk = point.y < 0.34
            && (abs(point.x + 0.58) < trunk_half_width * 0.7
                || abs(point.x + 0.18) < trunk_half_width * 0.7
                || abs(point.x - 0.24) < trunk_half_width * 0.7
                || abs(point.x - 0.61) < trunk_half_width * 0.7);
        if species_kind < 0.5 {
            canopy = circle(point, vec2<f32>(-0.58, 0.50), vec2<f32>(0.34, 0.26))
                || circle(point, vec2<f32>(-0.18, 0.67), vec2<f32>(0.40, 0.32))
                || circle(point, vec2<f32>(0.25, 0.57), vec2<f32>(0.38, 0.29))
                || circle(point, vec2<f32>(0.62, 0.72), vec2<f32>(0.32, 0.27));
        } else {
            let left = (1.0 - point.y) * 0.34;
            canopy = point.y > 0.16 && point.y < 0.96
                && (abs(point.x + 0.60) < left
                    || abs(point.x + 0.20) < left * 1.1
                    || abs(point.x - 0.24) < left
                    || abs(point.x - 0.62) < left * 0.9);
        }
    } else if species_kind < 0.5 {
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
    let colour = select(trunk_colour * 0.75 * input.lighting, input.colour_and_kind.rgb, canopy);
    return vec4<f32>(colour, 1.0);
}

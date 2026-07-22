const RAYMARCH_REFINEMENT_COUNT: u32 = 5u;

struct RayUniform {
    height_min_meters: f32,
    height_max_meters: f32,
    face_quads: u32,
    march_steps: u32,
    camera_radius_meters: f32,
    camera_radius_squared: f32,
    minimum_shell_radius_meters: f32,
    maximum_shell_radius_meters: f32,
}

@group(1) @binding(0)
var height_faces: texture_2d_array<f32>;
@group(1) @binding(1)
var biome_faces: texture_2d_array<u32>;
@group(1) @binding(2)
var moisture_faces: texture_2d_array<f32>;
@group(1) @binding(3)
var<uniform> ray_settings: RayUniform;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

struct FaceUv {
    face: u32,
    uv: vec2<f32>,
}

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let horizontal = ndc.x * camera.projection.x * camera.projection.y;
    let vertical = ndc.y * camera.projection.y;
    return normalize(vec3<f32>(horizontal, vertical, -1.0));
}

fn direction_to_face_uv(direction: vec3<f32>) -> FaceUv {
    let normalized_direction = normalize(direction);
    let absolute = abs(normalized_direction);
    if absolute.x >= absolute.y && absolute.x >= absolute.z {
        if normalized_direction.x >= 0.0 {
            return FaceUv(
                0u,
                vec2<f32>(-normalized_direction.z, normalized_direction.y)
                    / normalized_direction.x,
            );
        }
        let scale = -normalized_direction.x;
        return FaceUv(
            1u,
            vec2<f32>(normalized_direction.z, normalized_direction.y) / scale,
        );
    }
    if absolute.y >= absolute.z {
        if normalized_direction.y >= 0.0 {
            return FaceUv(
                2u,
                vec2<f32>(normalized_direction.x, -normalized_direction.z)
                    / normalized_direction.y,
            );
        }
        let scale = -normalized_direction.y;
        return FaceUv(
            3u,
            vec2<f32>(normalized_direction.x, normalized_direction.z) / scale,
        );
    }
    if normalized_direction.z >= 0.0 {
        return FaceUv(
            4u,
            vec2<f32>(normalized_direction.x, normalized_direction.y) / normalized_direction.z,
        );
    }
    let scale = -normalized_direction.z;
    return FaceUv(
        5u,
        vec2<f32>(-normalized_direction.x, normalized_direction.y) / scale,
    );
}

fn face_texel_coordinate(face_uv: FaceUv) -> vec2<f32> {
    return vec2<f32>(1.0)
        + (face_uv.uv * 0.5 + vec2<f32>(0.5)) * f32(ray_settings.face_quads);
}

fn sample_height(direction: vec3<f32>) -> f32 {
    let face_uv = direction_to_face_uv(direction);
    let coordinate = face_texel_coordinate(face_uv);
    let lower = vec2<i32>(floor(coordinate));
    let amount = fract(coordinate);
    let h00 = textureLoad(height_faces, lower, i32(face_uv.face), 0).x;
    let h10 = textureLoad(height_faces, lower + vec2<i32>(1, 0), i32(face_uv.face), 0).x;
    let h01 = textureLoad(height_faces, lower + vec2<i32>(0, 1), i32(face_uv.face), 0).x;
    let h11 = textureLoad(height_faces, lower + vec2<i32>(1, 1), i32(face_uv.face), 0).x;
    return mix(mix(h00, h10, amount.x), mix(h01, h11, amount.x), amount.y);
}

fn sample_biome(direction: vec3<f32>) -> u32 {
    let face_uv = direction_to_face_uv(direction);
    let coordinate = vec2<i32>(round(face_texel_coordinate(face_uv)));
    return textureLoad(biome_faces, coordinate, i32(face_uv.face), 0).x;
}

fn shell_interval(radial_dot_ray: f32) -> vec2<f32> {
    let discriminant = radial_dot_ray * radial_dot_ray
        - (ray_settings.camera_radius_squared
            - ray_settings.maximum_shell_radius_meters
                * ray_settings.maximum_shell_radius_meters);
    if discriminant < 0.0 {
        return vec2<f32>(-1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-radial_dot_ray - root, -radial_dot_ray + root);
}

fn radius_at(distance_meters: f32, radial_dot_ray: f32) -> f32 {
    return sqrt(max(
        ray_settings.camera_radius_squared
            + 2.0 * distance_meters * radial_dot_ray
            + distance_meters * distance_meters,
        0.0,
    ));
}

fn surface_function(
    distance_meters: f32,
    radial_dot_ray: f32,
    camera_position_view: vec3<f32>,
    ray: vec3<f32>,
) -> f32 {
    let point_view = camera_position_view + ray * distance_meters;
    let surface_direction = normalize(view_to_planet(point_view));
    let surface_radius = PLANET_RADIUS_METERS
        + sample_height(surface_direction) * terrain_macro_height_scale();
    return radius_at(distance_meters, radial_dot_ray) - surface_radius;
}

fn refine_hit(
    lower_distance: f32,
    upper_distance: f32,
    lower_value: f32,
    upper_value: f32,
    radial_dot_ray: f32,
    camera_position_view: vec3<f32>,
    ray: vec3<f32>,
) -> f32 {
    var lower = lower_distance;
    var upper = upper_distance;
    var value_lower = lower_value;
    var value_upper = upper_value;
    for (var index = 0u; index < RAYMARCH_REFINEMENT_COUNT; index += 1u) {
        let denominator = value_upper - value_lower;
        let secant = select(
            0.5 * (lower + upper),
            (lower * value_upper - upper * value_lower) / denominator,
            abs(denominator) > 1.0e-5,
        );
        let candidate = clamp(secant, lower, upper);
        let value = surface_function(
            candidate,
            radial_dot_ray,
            camera_position_view,
            ray,
        );
        if value > 0.0 {
            lower = candidate;
            value_lower = value;
        } else {
            upper = candidate;
            value_upper = value;
        }
    }
    return 0.5 * (lower + upper);
}

fn terrain_normal(surface_direction: vec3<f32>) -> vec3<f32> {
    let reference_axis = select(
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(1.0, 0.0, 0.0),
        abs(surface_direction.y) > 0.95,
    );
    let east = normalize(cross(reference_axis, surface_direction));
    let north = normalize(cross(surface_direction, east));
    let epsilon = 2.0 / f32(ray_settings.face_quads);
    let east_direction = normalize(surface_direction + east * epsilon);
    let north_direction = normalize(surface_direction + north * epsilon);
    let height_scale = terrain_macro_height_scale();
    let height = sample_height(surface_direction) * height_scale;
    let east_height = sample_height(east_direction) * height_scale;
    let north_height = sample_height(north_direction) * height_scale;
    let center = surface_direction * (PLANET_RADIUS_METERS + height);
    let east_point = east_direction * (PLANET_RADIUS_METERS + east_height);
    let north_point = north_direction * (PLANET_RADIUS_METERS + north_height);
    return normalize(cross(east_point - center, north_point - center));
}

fn sky_color(ray: vec3<f32>) -> vec3<f32> {
    let zenith = clamp(ray.y * 0.5 + 0.5, 0.0, 1.0);
    return mix(vec3<f32>(0.004, 0.008, 0.018), vec3<f32>(0.035, 0.075, 0.16), zenith);
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
fn fs_main(input: VertexOutput) -> FragmentOutput {
    let ray = view_direction(input.ndc);
    let camera_position_view = camera.camera_planet_direction_view_altitude.xyz
        * ray_settings.camera_radius_meters;
    let radial_dot_ray = dot(camera_position_view, ray);
    let interval = shell_interval(radial_dot_ray);
    let start_distance = max(interval.x, 0.0);
    let end_distance = interval.y;
    if interval.x < 0.0
        && ray_settings.camera_radius_meters > ray_settings.maximum_shell_radius_meters
    {
        return FragmentOutput(vec4<f32>(sky_color(ray), 1.0), 0.0);
    }
    if end_distance <= start_distance {
        return FragmentOutput(vec4<f32>(sky_color(ray), 1.0), 0.0);
    }

    var previous_distance = start_distance;
    var previous_value = surface_function(
        previous_distance,
        radial_dot_ray,
        camera_position_view,
        ray,
    );
    var hit_distance = -1.0;
    for (var index = 0u; index < 192u; index += 1u) {
        if index >= ray_settings.march_steps {
            break;
        }
        let amount = f32(index + 1u) / f32(ray_settings.march_steps);
        let distance = mix(start_distance, end_distance, amount);
        let value = surface_function(distance, radial_dot_ray, camera_position_view, ray);
        if value <= 0.0 && previous_value >= 0.0 {
            hit_distance = refine_hit(
                previous_distance,
                distance,
                previous_value,
                value,
                radial_dot_ray,
                camera_position_view,
                ray,
            );
            break;
        }
        previous_distance = distance;
        previous_value = value;
    }
    if hit_distance < 0.0 {
        return FragmentOutput(vec4<f32>(sky_color(ray), 1.0), 0.0);
    }

    let hit_view = camera_position_view + ray * hit_distance;
    let surface_direction = normalize(view_to_planet(hit_view));
    let normal = terrain_normal(surface_direction);
    let sunlight = max(dot(normal, normalize(camera.sun_direction.xyz)), 0.0);
    let albedo = biome_color(sample_biome(surface_direction));
    let color = albedo * sunlight;
    let clip = camera.projection_matrix * vec4<f32>(ray * hit_distance, 1.0);
    return FragmentOutput(vec4<f32>(color, 1.0), clip.z / clip.w);
}

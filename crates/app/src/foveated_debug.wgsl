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
}

struct FieldUniform {
    height_min_meters: f32,
    height_max_meters: f32,
    face_quads: u32,
    _padding: u32,
}

@group(0) @binding(0)
var<uniform> camera: Camera;
@group(1) @binding(0)
var height_faces: texture_2d_array<f32>;
@group(1) @binding(1)
var biome_faces: texture_2d_array<u32>;
@group(1) @binding(2)
var moisture_faces: texture_2d_array<f32>;
@group(1) @binding(3)
var<uniform> fields: FieldUniform;

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

fn view_to_planet(vector: vec3<f32>) -> vec3<f32> {
    return camera.camera_right.xyz * vector.x
        + camera.camera_up.xyz * vector.y
        - camera.camera_forward.xyz * vector.z;
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

fn sample_height(direction: vec3<f32>) -> f32 {
    let face_uv = direction_to_face_uv(direction);
    let coordinate = vec2<f32>(1.0)
        + (face_uv.uv * 0.5 + vec2<f32>(0.5)) * f32(fields.face_quads);
    let lower = vec2<i32>(floor(coordinate));
    let amount = fract(coordinate);
    let h00 = textureLoad(height_faces, lower, i32(face_uv.face), 0).x;
    let h10 = textureLoad(height_faces, lower + vec2<i32>(1, 0), i32(face_uv.face), 0).x;
    let h01 = textureLoad(height_faces, lower + vec2<i32>(0, 1), i32(face_uv.face), 0).x;
    let h11 = textureLoad(height_faces, lower + vec2<i32>(1, 1), i32(face_uv.face), 0).x;
    return mix(mix(h00, h10, amount.x), mix(h01, h11, amount.x), amount.y);
}

fn sphere_entry_distance(camera_radius: f32, radial_dot_ray: f32) -> f32 {
    let discriminant = radial_dot_ray * radial_dot_ray
        + PLANET_RADIUS_METERS * PLANET_RADIUS_METERS
        - camera_radius * camera_radius;
    if discriminant < 0.0 {
        return -1.0;
    }
    let root = sqrt(discriminant);
    let near_distance = -radial_dot_ray - root;
    if near_distance > 0.0 {
        return near_distance;
    }
    let far_distance = -radial_dot_ray + root;
    if far_distance > 0.0 {
        return far_distance;
    }
    return -1.0;
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
    let camera_altitude = camera.camera_planet_direction_view_altitude.w;
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude;
    let camera_position_view = camera.camera_planet_direction_view_altitude.xyz * camera_radius;
    let radial_dot_ray = dot(camera_position_view, ray);
    let hit_distance = sphere_entry_distance(camera_radius, radial_dot_ray);
    if hit_distance < 0.0 {
        discard;
    }
    let hit_view = camera_position_view + ray * hit_distance;
    let height = sample_height(view_to_planet(hit_view));
    let normalized_height = select(
        0.35 + 0.65 * clamp(height / max(fields.height_max_meters, 1.0), 0.0, 1.0),
        0.05 + 0.15 * clamp(
            (height - fields.height_min_meters) / max(-fields.height_min_meters, 1.0),
            0.0,
            1.0,
        ),
        height <= 0.0,
    );
    let clip = camera.projection_matrix * vec4<f32>(ray * hit_distance, 1.0);
    return FragmentOutput(vec4<f32>(vec3<f32>(normalized_height), 1.0), clip.z / clip.w);
}

const PLANET_RADIUS_METERS: f32 = 4000000.0;
const TILE_LOGICAL_QUADS: f32 = 32.0;
const TILE_GUTTER: f32 = 1.0;

struct Camera {
    view_projection: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var height_map: texture_2d<f32>;

@group(1) @binding(1)
var biome_map: texture_2d<u32>;

@group(1) @binding(2)
var moisture_map: texture_2d<f32>;

struct VertexInput {
    @location(0) anchor_relative_position: vec3<f32>,
    @location(1) sphere_direction: vec3<f32>,
    @location(2) tile_uv: vec2<f32>,
    @location(3) skirt_depth_meters: f32,
    @location(4) anchor_relative_to_camera: vec3<f32>,
    @location(5) source_uv_scale: vec2<f32>,
    @location(6) source_uv_offset: vec2<f32>,
    @location(7) terrain_info: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) camera_relative_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) base_color: vec3<f32>,
}

fn uses_outmap(terrain_info: u32) -> bool {
    return (terrain_info & 1u) != 0u;
}

fn cube_face(terrain_info: u32) -> u32 {
    return (terrain_info >> 1u) & 0x7u;
}

fn requested_level(terrain_info: u32) -> u32 {
    return (terrain_info >> 4u) & 0x1fu;
}

fn placeholder_octave(direction: vec3<f32>, frequency: f32, amplitude: f32) -> f32 {
    let wave = sin(frequency * direction.x) - direction.x * sin(frequency)
        + sin(1.375 * frequency * direction.y)
        + sin(1.75 * frequency * direction.z);
    return amplitude * wave / 4.0;
}

fn placeholder_height(direction: vec3<f32>) -> f32 {
    return placeholder_octave(direction, 8.0, 2800.0)
        + placeholder_octave(direction, 512.0, 600.0)
        + placeholder_octave(direction, 32768.0, 100.0)
        + placeholder_octave(direction, 2097152.0, 3.0);
}

fn sample_height(source_uv: vec2<f32>) -> f32 {
    let gutter_uv = 1.0 / TILE_LOGICAL_QUADS;
    let coordinate = vec2<f32>(TILE_GUTTER)
        + clamp(source_uv, vec2<f32>(-gutter_uv), vec2<f32>(1.0 + gutter_uv)) * TILE_LOGICAL_QUADS;
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(lower + vec2<i32>(1), vec2<i32>(34));
    let amount = fract(coordinate);
    let lower_left = textureLoad(height_map, lower, 0).x;
    let lower_right = textureLoad(height_map, vec2<i32>(upper.x, lower.y), 0).x;
    let upper_left = textureLoad(height_map, vec2<i32>(lower.x, upper.y), 0).x;
    let upper_right = textureLoad(height_map, upper, 0).x;
    return mix(
        mix(lower_left, lower_right, amount.x),
        mix(upper_left, upper_right, amount.x),
        amount.y,
    );
}

fn terrain_height(outmap: bool, source_uv: vec2<f32>, direction: vec3<f32>) -> f32 {
    if outmap {
        return sample_height(source_uv);
    }
    return placeholder_height(direction);
}

fn sample_biome(source_uv: vec2<f32>) -> u32 {
    let coordinate = vec2<i32>(round(vec2<f32>(TILE_GUTTER) + clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0)) * TILE_LOGICAL_QUADS));
    return textureLoad(biome_map, coordinate, 0).x;
}

fn sample_moisture(source_uv: vec2<f32>) -> f32 {
    let coordinate = vec2<i32>(round(vec2<f32>(TILE_GUTTER) + clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0)) * TILE_LOGICAL_QUADS));
    return textureLoad(moisture_map, coordinate, 0).x;
}

fn face_tangent_u(face: u32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(0.0, 0.0, -1.0); }
        case 1u: { return vec3<f32>(0.0, 0.0, 1.0); }
        case 2u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 3u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 4u: { return vec3<f32>(1.0, 0.0, 0.0); }
        default: { return vec3<f32>(-1.0, 0.0, 0.0); }
    }
}

fn face_tangent_v(face: u32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 1u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 2u: { return vec3<f32>(0.0, 0.0, -1.0); }
        case 3u: { return vec3<f32>(0.0, 0.0, 1.0); }
        case 4u: { return vec3<f32>(0.0, 1.0, 0.0); }
        default: { return vec3<f32>(0.0, 1.0, 0.0); }
    }
}

fn face_component(direction: vec3<f32>, face: u32) -> f32 {
    if face <= 1u {
        return abs(direction.x);
    }
    if face <= 3u {
        return abs(direction.y);
    }
    return abs(direction.z);
}

fn displaced_surface_normal(
    direction: vec3<f32>,
    source_uv: vec2<f32>,
    source_uv_scale: vec2<f32>,
    terrain_info: u32,
) -> vec3<f32> {
    let face = cube_face(terrain_info);
    let tangent_u = face_tangent_u(face);
    let tangent_v = face_tangent_v(face);
    let cube_position = direction / max(face_component(direction, face), 1.0e-6);
    let cube_step = 2.0 / (TILE_LOGICAL_QUADS * exp2(f32(requested_level(terrain_info))));
    let left_direction = normalize(cube_position - tangent_u * cube_step);
    let right_direction = normalize(cube_position + tangent_u * cube_step);
    let down_direction = normalize(cube_position - tangent_v * cube_step);
    let up_direction = normalize(cube_position + tangent_v * cube_step);
    let uv_step = source_uv_scale / TILE_LOGICAL_QUADS;
    let outmap = uses_outmap(terrain_info);
    let left_height = terrain_height(outmap, source_uv - vec2<f32>(uv_step.x, 0.0), left_direction);
    let right_height = terrain_height(outmap, source_uv + vec2<f32>(uv_step.x, 0.0), right_direction);
    let down_height = terrain_height(outmap, source_uv - vec2<f32>(0.0, uv_step.y), down_direction);
    let up_height = terrain_height(outmap, source_uv + vec2<f32>(0.0, uv_step.y), up_direction);
    let tangent_delta_u = (right_direction - left_direction) * PLANET_RADIUS_METERS
        + right_direction * right_height
        - left_direction * left_height;
    let tangent_delta_v = (up_direction - down_direction) * PLANET_RADIUS_METERS
        + up_direction * up_height
        - down_direction * down_height;
    return normalize(cross(tangent_delta_u, tangent_delta_v));
}

fn biome_color(biome: u32) -> vec3<f32> {
    switch biome {
        case 0u: { return vec3<f32>(20.0, 65.0, 150.0) / 255.0; }
        case 1u: { return vec3<f32>(45.0, 115.0, 190.0) / 255.0; }
        case 2u: { return vec3<f32>(230.0, 240.0, 245.0) / 255.0; }
        case 3u: { return vec3<f32>(130.0, 145.0, 120.0) / 255.0; }
        case 4u: { return vec3<f32>(45.0, 105.0, 55.0) / 255.0; }
        case 5u: { return vec3<f32>(105.0, 145.0, 65.0) / 255.0; }
        case 6u: { return vec3<f32>(25.0, 125.0, 55.0) / 255.0; }
        case 7u: { return vec3<f32>(205.0, 180.0, 105.0) / 255.0; }
        case 8u: { return vec3<f32>(105.0, 100.0, 95.0) / 255.0; }
        default: { return vec3<f32>(205.0, 210.0, 210.0) / 255.0; }
    }
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let direction = normalize(input.sphere_direction);
    let source_uv = input.source_uv_offset + input.tile_uv * input.source_uv_scale;
    let outmap = uses_outmap(input.terrain_info);
    let height = terrain_height(outmap, source_uv, direction);
    let camera_relative_position = input.anchor_relative_to_camera
        + input.anchor_relative_position
        + direction * (height - input.skirt_depth_meters);
    var color = vec3<f32>(0.32, 0.58, 0.74);
    if outmap {
        let moisture = sample_moisture(source_uv);
        color = biome_color(sample_biome(source_uv)) * mix(0.88, 1.06, moisture);
    }
    return VertexOutput(
        camera.view_projection * vec4<f32>(camera_relative_position, 1.0),
        camera_relative_position,
        displaced_surface_normal(
            direction,
            source_uv,
            input.source_uv_scale,
            input.terrain_info,
        ),
        color,
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let light_direction = normalize(vec3<f32>(0.4, 0.7, 0.6));
    let light = max(dot(normalize(input.world_normal), light_direction), 0.14);
    return vec4<f32>(input.base_color * light, 1.0);
}

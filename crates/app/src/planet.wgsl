const PLANET_RADIUS_METERS: f32 = 4000000.0;
const TILE_LOGICAL_QUADS: f32 = 32.0;
const TILE_GUTTER: f32 = 1.0;
const ATMOSPHERE_HEIGHT_METERS: f32 = 360000.0;
const ATMOSPHERE_EDGE_FADE_METERS: f32 = 240000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 36000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 4800.0;
const RAYLEIGH_COEFFICIENT: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFFICIENT: vec3<f32> = vec3<f32>(21.0e-6);
const MIE_G: f32 = 0.76;
const SOLAR_RADIANCE: f32 = 2.0;

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
    @location(8) lod_transition: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) camera_relative_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) aerial_color: vec3<f32>,
    @location(3) lod_transition: vec2<f32>,
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

fn density(altitude_meters: f32, scale_height_meters: f32) -> f32 {
    let clamped_altitude_meters = max(altitude_meters, 0.0);
    let edge_fade = 1.0 - smoothstep(
        ATMOSPHERE_HEIGHT_METERS - ATMOSPHERE_EDGE_FADE_METERS,
        ATMOSPHERE_HEIGHT_METERS,
        clamped_altitude_meters,
    );
    return exp(-clamped_altitude_meters / scale_height_meters) * edge_fade;
}

fn phase_rayleigh(cos_theta: f32) -> f32 {
    return 3.0 * (1.0 + cos_theta * cos_theta) / (16.0 * 3.14159265);
}

fn phase_mie(cos_theta: f32) -> f32 {
    let g_squared = MIE_G * MIE_G;
    let denominator = max(1.0 + g_squared - 2.0 * MIE_G * cos_theta, 1.0e-4);
    return 3.0 * (1.0 - g_squared) * (1.0 + cos_theta * cos_theta)
        / (8.0 * 3.14159265 * (2.0 + g_squared) * pow(denominator, 1.5));
}

fn transmittance(
    start_altitude_meters: f32,
    end_altitude_meters: f32,
    distance_meters: f32,
) -> vec3<f32> {
    let rayleigh_density = 0.5
        * (density(start_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS)
            + density(end_altitude_meters, RAYLEIGH_SCALE_HEIGHT_METERS));
    let mie_density = 0.5
        * (density(start_altitude_meters, MIE_SCALE_HEIGHT_METERS)
            + density(end_altitude_meters, MIE_SCALE_HEIGHT_METERS));
    return exp(-(RAYLEIGH_COEFFICIENT * rayleigh_density + MIE_COEFFICIENT * mie_density)
        * max(distance_meters, 0.0));
}

fn atmosphere_interval(radius_meters: f32, radial_dot_ray: f32) -> vec2<f32> {
    let discriminant = radial_dot_ray * radial_dot_ray
        + ATMOSPHERE_RADIUS_METERS * ATMOSPHERE_RADIUS_METERS
        - radius_meters * radius_meters;
    if discriminant <= 0.0 {
        return vec2<f32>(-1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-radial_dot_ray - root, -radial_dot_ray + root);
}

fn atmosphere_exit_distance(radius_meters: f32, radial_dot_ray: f32) -> f32 {
    return max(atmosphere_interval(radius_meters, radial_dot_ray).y, 0.0);
}

fn altitude_along_ray(radius_meters: f32, radial_dot_ray: f32, distance_meters: f32) -> f32 {
    return sqrt(
        radius_meters * radius_meters
            + 2.0 * radial_dot_ray * distance_meters
            + distance_meters * distance_meters,
    ) - PLANET_RADIUS_METERS;
}

fn sun_is_occluded(radius_meters: f32, radial_dot_sun: f32) -> bool {
    let discriminant = radial_dot_sun * radial_dot_sun
        - (radius_meters * radius_meters - PLANET_RADIUS_METERS * PLANET_RADIUS_METERS);
    return radial_dot_sun < 0.0 && discriminant >= 0.0;
}

fn aerial_perspective(
    lit_surface_color: vec3<f32>,
    camera_relative_position: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_altitude_meters: f32,
) -> vec3<f32> {
    let distance_meters = length(camera_relative_position);
    let camera_altitude_meters = camera.camera_planet_direction_altitude.w;
    let view_direction = normalize(camera_relative_position);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let camera_radius = PLANET_RADIUS_METERS + camera_altitude_meters;
    let radial_dot_view = camera_radius
        * dot(camera.camera_planet_direction_altitude.xyz, view_direction);
    let view_interval = atmosphere_interval(camera_radius, radial_dot_view);
    let view_start = max(view_interval.x, 0.0);
    let view_end = min(view_interval.y, distance_meters);
    if view_end <= view_start {
        return lit_surface_color;
    }
    let atmospheric_view_length = view_end - view_start;
    let atmospheric_view_start_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_view,
        view_start,
    );
    let atmospheric_view_end_altitude = altitude_along_ray(
        camera_radius,
        radial_dot_view,
        view_end,
    );
    let surface_radius = PLANET_RADIUS_METERS + surface_altitude_meters;
    let radial_dot_sun = surface_radius * dot(surface_direction, sun_direction);
    let sun_distance = atmosphere_exit_distance(surface_radius, radial_dot_sun);
    let sun_transmittance = select(
        transmittance(surface_altitude_meters, ATMOSPHERE_HEIGHT_METERS, sun_distance),
        vec3<f32>(0.0),
        sun_is_occluded(surface_radius, radial_dot_sun),
    );
    let view_transmittance = transmittance(
        atmospheric_view_start_altitude,
        atmospheric_view_end_altitude,
        atmospheric_view_length,
    );
    let average_rayleigh_density = 0.5
        * (density(
            atmospheric_view_start_altitude,
            RAYLEIGH_SCALE_HEIGHT_METERS,
        ) + density(
            atmospheric_view_end_altitude,
            RAYLEIGH_SCALE_HEIGHT_METERS,
        ));
    let average_mie_density = 0.5
        * (density(atmospheric_view_start_altitude, MIE_SCALE_HEIGHT_METERS)
            + density(atmospheric_view_end_altitude, MIE_SCALE_HEIGHT_METERS));
    let cos_theta = dot(view_direction, sun_direction);
    let in_scatter = sun_transmittance
        * (RAYLEIGH_COEFFICIENT * average_rayleigh_density * phase_rayleigh(cos_theta)
            + MIE_COEFFICIENT * average_mie_density * phase_mie(cos_theta))
        * atmospheric_view_length
        * SOLAR_RADIANCE;
    return lit_surface_color * view_transmittance + in_scatter;
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
    let normal = displaced_surface_normal(
        direction,
        source_uv,
        input.source_uv_scale,
        input.terrain_info,
    );
    let sun_direction = normalize(camera.sun_direction.xyz);
    let surface_radius = PLANET_RADIUS_METERS + height;
    let radial_dot_sun = surface_radius * dot(direction, sun_direction);
    let direct_sun_transmittance = select(
        transmittance(
            height,
            ATMOSPHERE_HEIGHT_METERS,
            atmosphere_exit_distance(surface_radius, radial_dot_sun),
        ),
        vec3<f32>(0.0),
        sun_is_occluded(surface_radius, radial_dot_sun),
    );
    let direct_light = max(dot(normal, sun_direction), 0.14);
    let lit_surface_color = color * (vec3<f32>(0.14) + direct_sun_transmittance * direct_light);
    return VertexOutput(
        camera.view_projection * vec4<f32>(camera_relative_position, 1.0),
        camera_relative_position,
        normal,
        aerial_perspective(lit_surface_color, camera_relative_position, direction, height),
        input.lod_transition,
    );
}

fn bayer_dither(fragment_position: vec4<f32>) -> f32 {
    let pattern = array<u32, 16>(
        0u, 8u, 2u, 10u,
        12u, 4u, 14u, 6u,
        3u, 11u, 1u, 9u,
        15u, 7u, 13u, 5u,
    );
    let pixel = vec2<u32>(u32(fragment_position.x), u32(fragment_position.y));
    let index = (pixel.y & 3u) * 4u + (pixel.x & 3u);
    return (f32(pattern[index]) + 0.5) / 16.0;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let transition_progress = input.lod_transition.x;
    let incoming = input.lod_transition.y > 0.5;
    let threshold = bayer_dither(input.position);
    if (incoming && threshold >= transition_progress)
        || (!incoming && threshold < transition_progress)
    {
        discard;
    }
    return vec4<f32>(input.aerial_color, 1.0);
}

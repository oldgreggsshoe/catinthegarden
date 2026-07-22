const RAYMARCH_REFINEMENT_COUNT: u32 = 5u;
const RAY_SKY_SAMPLE_COUNT: u32 = 16u;
const RAY_SKY_DENSITY_SAMPLE_EXPONENT: f32 = 3.0;
const RAY_ANTISOLAR_TWILIGHT_MIN_SCATTER: f32 = 0.48;
const RAY_SKY_ATMOSPHERE_SATURATION: f32 = 2.0;
const RAY_OCEAN_SHELL_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + 1.0;
const RENDER_DEBUG_SKY_ONLY: u32 = 4u;
const EXPERIMENT_HORIZON_DENSITY: u32 = 1u << 0u;
const EXPERIMENT_TEMPORAL_REUSE: u32 = 1u << 1u;
const EXPERIMENT_FOVEATED_SHADING: u32 = 1u << 3u;

struct RayUniform {
    height_min_meters: f32,
    height_max_meters: f32,
    face_quads: u32,
    march_steps: u32,
    camera_radius_meters: f32,
    camera_radius_squared: f32,
    minimum_shell_radius_meters: f32,
    maximum_shell_radius_meters: f32,
    max_height_mip_count: u32,
    minimum_step_meters: f32,
    fovea_ndc: vec2<f32>,
    experiment_flags: u32,
    frame_index: u32,
    _padding: vec2<u32>,
    previous_fovea_ndc: vec2<f32>,
    temporal_valid: u32,
    _temporal_padding: u32,
    previous_camera_forward: vec4<f32>,
    previous_camera_right: vec4<f32>,
    previous_camera_up: vec4<f32>,
}

@group(1) @binding(0)
var height_faces: texture_2d_array<f32>;
@group(1) @binding(1)
var biome_faces: texture_2d_array<u32>;
@group(1) @binding(2)
var moisture_faces: texture_2d_array<f32>;
@group(1) @binding(3)
var<uniform> ray_settings: RayUniform;
@group(1) @binding(4)
var max_height_faces: texture_2d_array<f32>;
@group(3) @binding(0)
var history_color: texture_2d<f32>;
@group(3) @binding(1)
var history_distance: texture_2d<f32>;
@group(3) @binding(2)
var history_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

struct WarpFragmentOutput {
    @location(0) color: vec4<f32>,
    @location(1) distance_meters: f32,
}

struct RayResult {
    color: vec4<f32>,
    distance_meters: f32,
}

struct FaceUv {
    face: u32,
    uv: vec2<f32>,
}

struct OceanHit {
    distance_meters: f32,
    coverage: f32,
}

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let horizontal = ndc.x * camera.projection.x * camera.projection.y;
    let vertical = ndc.y * camera.projection.y;
    return normalize(vec3<f32>(horizontal, vertical, -1.0));
}

fn warp_axis(coordinate: f32) -> f32 {
    const EXPONENT: f32 = 2.0;
    const LINEAR_CORE: f32 = 0.5;
    let magnitude = abs(coordinate);
    let core_power = pow(LINEAR_CORE, EXPONENT);
    let denominator = pow(1.0 + LINEAR_CORE, EXPONENT) - core_power;
    let warped = (
        pow(magnitude + LINEAR_CORE, EXPONENT) - core_power
    ) / denominator;
    return sign(coordinate) * warped;
}

fn warped_screen_axis(coordinate: f32, fovea: f32) -> f32 {
    let side_extent = select(1.0 + fovea, 1.0 - fovea, coordinate >= 0.0);
    return fovea + warp_axis(coordinate) * side_extent;
}

fn warped_screen_ndc(warp_ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        warped_screen_axis(warp_ndc.x, ray_settings.fovea_ndc.x),
        warped_screen_axis(warp_ndc.y, ray_settings.fovea_ndc.y),
    );
}

fn unwarp_axis(coordinate: f32) -> f32 {
    const EXPONENT: f32 = 2.0;
    const LINEAR_CORE: f32 = 0.5;
    let magnitude = abs(coordinate);
    let core_power = pow(LINEAR_CORE, EXPONENT);
    let denominator = pow(1.0 + LINEAR_CORE, EXPONENT) - core_power;
    let unwarped = pow(
        clamp(magnitude, 0.0, 1.0) * denominator + core_power,
        1.0 / EXPONENT,
    ) - LINEAR_CORE;
    return sign(coordinate) * unwarped;
}

fn history_coordinates(ray: vec3<f32>) -> vec3<f32> {
    let planet_ray = view_to_planet(ray);
    let previous_ray = vec3<f32>(
        dot(planet_ray, ray_settings.previous_camera_right.xyz),
        dot(planet_ray, ray_settings.previous_camera_up.xyz),
        -dot(planet_ray, ray_settings.previous_camera_forward.xyz),
    );
    if previous_ray.z >= -1.0e-5 {
        return vec3<f32>(0.0);
    }
    let screen_ndc = vec2<f32>(
        previous_ray.x / -previous_ray.z / (camera.projection.x * camera.projection.y),
        previous_ray.y / -previous_ray.z / camera.projection.y,
    );
    let offset = screen_ndc - ray_settings.previous_fovea_ndc;
    let side_extent = select(
        vec2<f32>(1.0) + ray_settings.previous_fovea_ndc,
        vec2<f32>(1.0) - ray_settings.previous_fovea_ndc,
        offset >= vec2<f32>(0.0),
    );
    let warp_ndc = vec2<f32>(
        unwarp_axis(offset.x / side_extent.x),
        unwarp_axis(offset.y / side_extent.y),
    );
    let uv = vec2<f32>(warp_ndc.x * 0.5 + 0.5, 0.5 - warp_ndc.y * 0.5);
    let valid = select(
        0.0,
        1.0,
        all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)),
    );
    return vec3<f32>(uv, valid);
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

fn sample_biome_blend(direction: vec3<f32>) -> BiomeBlendSample {
    let face_uv = direction_to_face_uv(direction);
    let coordinate = face_texel_coordinate(face_uv);
    let lower = vec2<i32>(floor(coordinate));
    let upper = lower + vec2<i32>(1);
    let amount = fract(coordinate);
    return BiomeBlendSample(
        vec4<u32>(
            textureLoad(biome_faces, lower, i32(face_uv.face), 0).x,
            textureLoad(biome_faces, vec2<i32>(upper.x, lower.y), i32(face_uv.face), 0).x,
            textureLoad(biome_faces, vec2<i32>(lower.x, upper.y), i32(face_uv.face), 0).x,
            textureLoad(biome_faces, upper, i32(face_uv.face), 0).x,
        ),
        vec4<f32>(
            (1.0 - amount.x) * (1.0 - amount.y),
            amount.x * (1.0 - amount.y),
            (1.0 - amount.x) * amount.y,
            amount.x * amount.y,
        ),
    );
}

fn sample_moisture(direction: vec3<f32>) -> f32 {
    let face_uv = direction_to_face_uv(direction);
    let coordinate = face_texel_coordinate(face_uv);
    let lower = vec2<i32>(floor(coordinate));
    let amount = fract(coordinate);
    let lower_left = textureLoad(moisture_faces, lower, i32(face_uv.face), 0).x;
    let lower_right = textureLoad(
        moisture_faces,
        lower + vec2<i32>(1, 0),
        i32(face_uv.face),
        0,
    ).x;
    let upper_left = textureLoad(
        moisture_faces,
        lower + vec2<i32>(0, 1),
        i32(face_uv.face),
        0,
    ).x;
    let upper_right = textureLoad(
        moisture_faces,
        lower + vec2<i32>(1),
        i32(face_uv.face),
        0,
    ).x;
    return mix(
        mix(lower_left, lower_right, amount.x),
        mix(upper_left, upper_right, amount.x),
        amount.y,
    );
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

fn sample_max_height(
    start_direction: vec3<f32>,
    end_direction: vec3<f32>,
    desired_step_meters: f32,
) -> f32 {
    let start_face_uv = direction_to_face_uv(start_direction);
    let end_face_uv = direction_to_face_uv(end_direction);
    if start_face_uv.face != end_face_uv.face {
        return ray_settings.height_max_meters;
    }
    let base_extent = textureDimensions(max_height_faces, 0).x;
    let base_texel_span_meters = ray_settings.minimum_step_meters * 2.0;
    let mip_level = u32(clamp(
        ceil(log2(max(desired_step_meters / base_texel_span_meters, 1.0))) + 2.0,
        0.0,
        f32(ray_settings.max_height_mip_count - 1u),
    ));
    let mip_dimensions = textureDimensions(max_height_faces, i32(mip_level));
    let start_coordinate = face_texel_coordinate(start_face_uv)
        * vec2<f32>(mip_dimensions)
        / f32(base_extent);
    let end_coordinate = face_texel_coordinate(end_face_uv)
        * vec2<f32>(mip_dimensions)
        / f32(base_extent);
    let start_texel = vec2<i32>(floor(start_coordinate));
    let end_texel = vec2<i32>(floor(end_coordinate));
    if any(start_texel != end_texel)
        || start_texel.x < 0
        || start_texel.y < 0
        || start_texel.x >= i32(mip_dimensions.x)
        || start_texel.y >= i32(mip_dimensions.y)
    {
        return ray_settings.height_max_meters;
    }
    return textureLoad(
        max_height_faces,
        start_texel,
        i32(start_face_uv.face),
        i32(mip_level),
    ).x;
}

fn adaptive_step_distance(
    iteration: u32,
    distance_meters: f32,
    baseline_step_meters: f32,
    radial_dot_ray: f32,
    camera_position_view: vec3<f32>,
    ray: vec3<f32>,
) -> f32 {
    if iteration == 0u {
        return baseline_step_meters;
    }
    let desired_step_meters = baseline_step_meters
        * exp2(f32(min(iteration, 6u)));
    let point_view = camera_position_view + ray * distance_meters;
    let end_point_view = point_view + ray * desired_step_meters;
    let maximum_height_meters = sample_max_height(
        normalize(view_to_planet(point_view)),
        normalize(view_to_planet(end_point_view)),
        desired_step_meters,
    ) * terrain_macro_height_scale();
    let maximum_radius_meters = PLANET_RADIUS_METERS + maximum_height_meters;
    let discriminant = radial_dot_ray * radial_dot_ray
        + maximum_radius_meters * maximum_radius_meters
        - ray_settings.camera_radius_squared;
    if discriminant < 0.0 {
        return desired_step_meters;
    }
    let root = sqrt(discriminant);
    let near_distance = -radial_dot_ray - root;
    let far_distance = -radial_dot_ray + root;
    if distance_meters < near_distance {
        return clamp(
            (near_distance - distance_meters) * 0.8,
            baseline_step_meters,
            desired_step_meters,
        );
    }
    if distance_meters > far_distance {
        return desired_step_meters;
    }
    return baseline_step_meters;
}

fn sphere_entry_distance(radius_meters: f32, radial_dot_ray: f32) -> f32 {
    let discriminant = radial_dot_ray * radial_dot_ray
        + radius_meters * radius_meters
        - ray_settings.camera_radius_squared;
    if discriminant < 0.0 {
        return -1.0;
    }
    let root = sqrt(discriminant);
    let near_distance = -radial_dot_ray - root;
    if near_distance > 0.0 {
        return near_distance;
    }
    let far_distance = -radial_dot_ray + root;
    return select(-1.0, far_distance, far_distance > 0.0);
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

fn ocean_hit(
    radial_dot_ray: f32,
    camera_position_view: vec3<f32>,
    ray: vec3<f32>,
    detail: f32,
) -> OceanHit {
    let shell_distance = sphere_entry_distance(
        RAY_OCEAN_SHELL_RADIUS_METERS,
        radial_dot_ray,
    );
    if shell_distance < 0.0 {
        return OceanHit(-1.0, 0.0);
    }
    let shell_direction = normalize(view_to_planet(
        camera_position_view + ray * shell_distance,
    ));
    let coverage = outmap_ocean_coverage(true, sample_height(shell_direction));
    if coverage <= 0.0 {
        return OceanHit(-1.0, 0.0);
    }

    var distance_meters = shell_distance;
    let use_waves = (ray_settings.experiment_flags & EXPERIMENT_FOVEATED_SHADING) == 0u
        || detail >= 0.45;
    if use_waves {
        for (var index = 0u; index < 2u; index += 1u) {
            let direction = normalize(view_to_planet(
                camera_position_view + ray * distance_meters,
            ));
            let surface = ocean_surface(direction, camera.projection.z);
            distance_meters = sphere_entry_distance(
                PLANET_RADIUS_METERS + surface.vertical_displacement,
                radial_dot_ray,
            );
            if distance_meters < 0.0 {
                return OceanHit(-1.0, 0.0);
            }
        }
    }
    return OceanHit(distance_meters, coverage);
}

fn solid_planet_entry_distance(radial_dot_ray: f32) -> f32 {
    let discriminant = radial_dot_ray * radial_dot_ray
        + PLANET_RADIUS_METERS * PLANET_RADIUS_METERS
        - ray_settings.camera_radius_squared;
    if discriminant <= 0.0 {
        return 1.0e30;
    }
    let root = sqrt(discriminant);
    let near_distance = -radial_dot_ray - root;
    if near_distance > 0.0 {
        return near_distance;
    }
    let far_distance = -radial_dot_ray + root;
    return select(1.0e30, far_distance, far_distance > 0.0);
}

fn ray_saturate_sky_color(color: vec3<f32>) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    return max(
        vec3<f32>(luminance)
            + (color - vec3<f32>(luminance)) * RAY_SKY_ATMOSPHERE_SATURATION,
        vec3<f32>(0.0),
    );
}

fn ray_twilight_directional_weight(
    cos_theta: f32,
    camera_solar_zenith_cosine: f32,
) -> f32 {
    let twilight_amount = 1.0 - smoothstep(0.0, 0.25, camera_solar_zenith_cosine);
    let antisolar_amount = smoothstep(0.0, 1.0, max(-cos_theta, 0.0));
    return mix(
        1.0,
        RAY_ANTISOLAR_TWILIGHT_MIN_SCATTER,
        twilight_amount * antisolar_amount,
    );
}

fn ray_density_sample_fraction(fraction: f32, closest_fraction: f32) -> f32 {
    if closest_fraction <= 0.05 {
        return pow(fraction, RAY_SKY_DENSITY_SAMPLE_EXPONENT);
    }
    if closest_fraction >= 0.95 {
        return 1.0 - pow(1.0 - fraction, RAY_SKY_DENSITY_SAMPLE_EXPONENT);
    }
    if fraction <= 0.5 {
        let local_fraction = fraction * 2.0;
        return closest_fraction
            * (1.0 - pow(1.0 - local_fraction, RAY_SKY_DENSITY_SAMPLE_EXPONENT));
    }
    let local_fraction = (fraction - 0.5) * 2.0;
    return closest_fraction
        + (1.0 - closest_fraction)
            * pow(local_fraction, RAY_SKY_DENSITY_SAMPLE_EXPONENT);
}

fn ray_local_solar_transmittance(
    sample_altitude: f32,
    sample_radius: f32,
    sample_radial_dot_sun: f32,
    sample_direction_view: vec3<f32>,
    sun_view: vec3<f32>,
    shadow_transition_meters: f32,
) -> vec3<f32> {
    let air_mass = twilight_solar_air_mass(
        dot(sample_direction_view, sun_view),
        sample_altitude,
    );
    let rayleigh_optical_depth = RAYLEIGH_COEFFICIENT
        * density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
        * RAYLEIGH_SCALE_HEIGHT_METERS
        * air_mass;
    let mie_optical_depth = MIE_COEFFICIENT
        * density(sample_altitude, MIE_SCALE_HEIGHT_METERS)
        * MIE_SCALE_HEIGHT_METERS
        * air_mass;
    return exp(-(rayleigh_optical_depth + mie_optical_depth))
        * sun_visibility(sample_radius, sample_radial_dot_sun, shadow_transition_meters);
}

fn ray_atmosphere_radiance(ray: vec3<f32>, radial_dot_ray: f32, detail: f32) -> vec3<f32> {
    let interval = atmosphere_interval(ray_settings.camera_radius_meters, radial_dot_ray);
    let start_distance = max(interval.x, 0.0);
    let end_distance = min(interval.y, solid_planet_entry_distance(radial_dot_ray));
    if end_distance <= start_distance {
        return vec3<f32>(0.0);
    }

    let path_length = end_distance - start_distance;
    let closest_distance = clamp(-radial_dot_ray, start_distance, end_distance);
    let closest_fraction = (closest_distance - start_distance) / path_length;
    let entry_altitude = altitude_along_ray(
        ray_settings.camera_radius_meters,
        radial_dot_ray,
        start_distance,
    );
    let sun_view = normalize(camera.sun_direction_view.xyz);
    let cos_theta = dot(ray, sun_view);
    let rayleigh_phase = phase_rayleigh(cos_theta);
    let mie_phase = phase_mie(cos_theta);
    let directional_weight = ray_twilight_directional_weight(
        cos_theta,
        dot(camera.camera_planet_direction_view_altitude.xyz, sun_view),
    );
    let camera_position_view = camera.camera_planet_direction_view_altitude.xyz
        * ray_settings.camera_radius_meters;
    let foveated_shading = (ray_settings.experiment_flags & EXPERIMENT_FOVEATED_SHADING) != 0u;
    let sample_count = select(16u, 6u, foveated_shading && detail < 0.45);
    var radiance = vec3<f32>(0.0);
    for (var index = 0u; index < RAY_SKY_SAMPLE_COUNT; index += 1u) {
        if index >= sample_count {
            break;
        }
        let fraction_start = f32(index) / f32(sample_count);
        let fraction_end = f32(index + 1u) / f32(sample_count);
        let sample_start = ray_density_sample_fraction(fraction_start, closest_fraction);
        let sample_end = ray_density_sample_fraction(fraction_end, closest_fraction);
        let sample_length = (sample_end - sample_start) * path_length;
        let distance_meters = start_distance
            + 0.5 * (sample_start + sample_end) * path_length;
        let sample_altitude = altitude_along_ray(
            ray_settings.camera_radius_meters,
            radial_dot_ray,
            distance_meters,
        );
        let sample_radius = PLANET_RADIUS_METERS + sample_altitude;
        let lower_atmosphere_weight = density(
            sample_altitude,
            RAYLEIGH_SCALE_HEIGHT_METERS,
        );
        let shadow_transition_meters = max(
            TWILIGHT_SHADOW_TRANSITION_METERS,
            sample_length * 0.5,
        ) * mix(1.0, 2.0, lower_atmosphere_weight);
        let sample_position_view = camera_position_view + ray * distance_meters;
        let sample_direction_view = normalize(sample_position_view);
        let sample_radial_dot_sun = dot(sample_position_view, sun_view);
        let view_transmittance = transmittance(
            entry_altitude,
            sample_altitude,
            distance_meters - start_distance,
        );
        let sun_transmittance = ray_local_solar_transmittance(
            sample_altitude,
            sample_radius,
            sample_radial_dot_sun,
            sample_direction_view,
            sun_view,
            shadow_transition_meters,
        );
        let rayleigh_scattering = RAYLEIGH_COEFFICIENT
            * density(sample_altitude, RAYLEIGH_SCALE_HEIGHT_METERS)
            * rayleigh_phase;
        let mie_scattering = MIE_COEFFICIENT
            * density(sample_altitude, MIE_SCALE_HEIGHT_METERS)
            * mie_phase;
        radiance += view_transmittance * sun_transmittance
            * (rayleigh_scattering + mie_scattering)
            * sample_length;
    }
    return ray_saturate_sky_color(max(
        radiance * SOLAR_RADIANCE * directional_weight,
        vec3<f32>(0.0),
    ));
}

fn shade_terrain(
    surface_direction: vec3<f32>,
    normal: vec3<f32>,
    hit_view_position: vec3<f32>,
) -> vec3<f32> {
    let render_debug_mode = u32(camera.projection.w + 0.5);
    let macro_height_meters = sample_height(surface_direction);
    let biome = sample_biome(surface_direction);
    let biome_blend = sample_biome_blend(surface_direction);
    let moisture = sample_moisture(surface_direction);
    let base_biome_color = blended_biome_color(biome_blend);
    let terrain_albedo = terrain_material_color(
        true,
        biome,
        moisture,
        base_biome_color,
        macro_height_meters,
        0.0,
        normal,
        surface_direction,
    );
    let detail_tint = terrain_material_tint(
        true,
        moisture,
        biome_blend,
        macro_height_meters,
        terrain_albedo,
        surface_direction,
        normal,
        hit_view_position,
    );
    let textured_albedo = terrain_albedo * detail_tint;
    if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
        return textured_albedo;
    }

    let surface_height = macro_height_meters * terrain_macro_height_scale();
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_transmittance = surface_direct_sun_transmittance(
        surface_direction,
        surface_height,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        normal,
        surface_direction,
        surface_height,
        sun_direction,
    );
    let surface_irradiance = sky_diffuse
        + sun_transmittance
            * max(dot(normal, sun_direction), 0.0)
            * SURFACE_SUNLIGHT_SCALE;
    var surface_lighting = textured_albedo * surface_irradiance;
    if biome == 2u {
        let ice_light_floor = clamp(
            max(max(surface_irradiance.x, surface_irradiance.y), surface_irradiance.z),
            0.0,
            1.0,
        );
        surface_lighting = max(
            surface_lighting,
            biome_color(2u) * 0.65 * ice_light_floor,
        );
    }
    if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
        return surface_lighting;
    }
    let aerial = terrain_distance_fog(
        aerial_perspective(
            surface_lighting,
            hit_view_position,
            surface_direction,
            surface_height,
        ),
        hit_view_position,
        surface_direction,
        surface_height,
    );
    if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
        return max(aerial - surface_lighting, vec3<f32>(0.0));
    }
    return aerial;
}

fn shade_ocean(
    surface_direction: vec3<f32>,
    hit_view_position: vec3<f32>,
    water_base_height: f32,
    detail: f32,
) -> vec3<f32> {
    let render_debug_mode = u32(camera.projection.w + 0.5);
    if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
        return debug_ocean_albedo();
    }
    var surface = OceanSurface(vec3<f32>(0.0), 0.0, surface_direction);
    if (ray_settings.experiment_flags & EXPERIMENT_FOVEATED_SHADING) == 0u
        || detail >= 0.45
    {
        surface = ocean_surface(surface_direction, camera.projection.z);
    }
    let water_surface_height = water_base_height + surface.vertical_displacement;
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_transmittance = surface_direct_sun_transmittance(
        surface_direction,
        water_surface_height,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        surface.normal,
        surface_direction,
        water_surface_height,
        sun_direction,
    );
    let surface_color = ocean_lighting(
        surface.normal,
        hit_view_position,
        sun_transmittance,
        sky_diffuse,
    );
    if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
        return surface_color;
    }
    let aerial_color = ocean_aerial_perspective(
        surface_color,
        hit_view_position,
        surface_direction,
        water_surface_height,
    );
    if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
        return max(aerial_color - surface_color, vec3<f32>(0.0));
    }
    return aerial_color;
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

fn trace_ray(ray: vec3<f32>, detail: f32) -> RayResult {
    let camera_position_view = camera.camera_planet_direction_view_altitude.xyz
        * ray_settings.camera_radius_meters;
    let radial_dot_ray = dot(camera_position_view, ray);
    let render_debug_mode = u32(camera.projection.w + 0.5);
    if render_debug_mode == RENDER_DEBUG_SKY_ONLY {
        return RayResult(
            vec4<f32>(ray_atmosphere_radiance(ray, radial_dot_ray, detail), 1.0),
            -1.0,
        );
    }
    let interval = shell_interval(radial_dot_ray);
    let start_distance = max(interval.x, 0.0);
    let end_distance = interval.y;
    if interval.x < 0.0
        && ray_settings.camera_radius_meters > ray_settings.maximum_shell_radius_meters
    {
        return RayResult(
            vec4<f32>(ray_atmosphere_radiance(ray, radial_dot_ray, detail), 1.0),
            -1.0,
        );
    }
    if end_distance <= start_distance {
        return RayResult(
            vec4<f32>(ray_atmosphere_radiance(ray, radial_dot_ray, detail), 1.0),
            -1.0,
        );
    }

    var previous_distance = start_distance;
    var previous_value = surface_function(
        previous_distance,
        radial_dot_ray,
        camera_position_view,
        ray,
    );
    var hit_distance = -1.0;
    let baseline_step_meters = (end_distance - start_distance)
        / f32(ray_settings.march_steps);
    for (var index = 0u; index < 192u; index += 1u) {
        if index >= ray_settings.march_steps {
            break;
        }
        let step_distance = adaptive_step_distance(
            index,
            previous_distance,
            baseline_step_meters,
            radial_dot_ray,
            camera_position_view,
            ray,
        );
        let distance = min(previous_distance + step_distance, end_distance);
        if distance <= previous_distance {
            break;
        }
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
    let water_hit = ocean_hit(radial_dot_ray, camera_position_view, ray, detail);
    if water_hit.distance_meters >= 0.0
        && (hit_distance < 0.0 || water_hit.distance_meters <= hit_distance)
    {
        let water_view = camera_position_view + ray * water_hit.distance_meters;
        let water_direction = normalize(view_to_planet(water_view));
        var color = shade_ocean(
            water_direction,
            ray * water_hit.distance_meters,
            0.0,
            detail,
        );
        if water_hit.coverage < 1.0 && hit_distance >= 0.0 {
            let terrain_view = camera_position_view + ray * hit_distance;
            let terrain_direction = normalize(view_to_planet(terrain_view));
            let terrain_color = shade_terrain(
                terrain_direction,
                terrain_normal(terrain_direction),
                ray * hit_distance,
            );
            color = mix(terrain_color, color, water_hit.coverage);
        }
        return RayResult(vec4<f32>(color, 1.0), water_hit.distance_meters);
    }
    if hit_distance < 0.0 {
        return RayResult(
            vec4<f32>(ray_atmosphere_radiance(ray, radial_dot_ray, detail), 1.0),
            -1.0,
        );
    }

    let hit_view = camera_position_view + ray * hit_distance;
    let surface_direction = normalize(view_to_planet(hit_view));
    let normal = terrain_normal(surface_direction);
    let macro_height = sample_height(surface_direction);
    let biome = sample_biome(surface_direction);
    let ocean_coverage = outmap_ocean_coverage(true, macro_height);
    let terrain_color = shade_terrain(surface_direction, normal, ray * hit_distance);
    var color = terrain_color;
    if biome == 1u {
        color = shade_ocean(
            surface_direction,
            ray * hit_distance,
            macro_height * terrain_macro_height_scale(),
            detail,
        );
    } else if ocean_coverage > 0.0 {
        let ocean_color = shade_ocean(
            surface_direction,
            ray * hit_distance,
            0.0,
            detail,
        );
        color = mix(terrain_color, ocean_color, ocean_coverage);
    }
    return RayResult(vec4<f32>(color, 1.0), hit_distance);
}

@fragment
fn fs_main(input: VertexOutput) -> FragmentOutput {
    let ray = view_direction(input.ndc);
    let result = trace_ray(ray, 1.0);
    if result.distance_meters < 0.0 {
        return FragmentOutput(result.color, 0.0);
    }
    let clip = camera.projection_matrix
        * vec4<f32>(ray * result.distance_meters, 1.0);
    return FragmentOutput(result.color, clip.z / clip.w);
}

@fragment
fn fs_warp(input: VertexOutput) -> WarpFragmentOutput {
    let detail = 1.0 - smoothstep(0.25, 1.0, length(input.ndc));
    let screen_ndc = warped_screen_ndc(input.ndc);
    let ray = view_direction(screen_ndc);
    let camera_position_view = camera.camera_planet_direction_view_altitude.xyz
        * ray_settings.camera_radius_meters;
    let radial_dot_ray = dot(camera_position_view, ray);
    let closest_radius = sqrt(max(
        ray_settings.camera_radius_squared - radial_dot_ray * radial_dot_ray,
        0.0,
    ));
    let near_horizon = abs(closest_radius - PLANET_RADIUS_METERS) < 50000.0;
    let checker = (
        u32(input.position.x) / 8u
        + u32(input.position.y) / 8u
        + ray_settings.frame_index
    ) & 1u;
    if (ray_settings.experiment_flags & EXPERIMENT_TEMPORAL_REUSE) != 0u
        && ray_settings.temporal_valid != 0u
        && detail < 0.45
        && !near_horizon
        && checker == 0u
    {
        let history = history_coordinates(ray);
        if history.z > 0.5 {
            let uv = history.xy;
            let color = textureSampleLevel(history_color, history_sampler, uv, 0.0);
            let dimensions = textureDimensions(history_distance);
            let texel = clamp(
                vec2<i32>(floor(uv * vec2<f32>(dimensions))),
                vec2<i32>(0),
                vec2<i32>(dimensions) - vec2<i32>(1),
            );
            let distance_meters = textureLoad(history_distance, texel, 0).x;
            return WarpFragmentOutput(color, distance_meters);
        }
    }
    var result = trace_ray(ray, detail);
    if (ray_settings.experiment_flags & EXPERIMENT_HORIZON_DENSITY) != 0u {
        if abs(closest_radius - PLANET_RADIUS_METERS) < 30000.0 {
            let neighbor_warp_ndc = input.ndc + vec2<f32>(
                dpdx(input.ndc.x),
                dpdy(input.ndc.y),
            ) * 0.35;
            let neighbor = trace_ray(
                view_direction(warped_screen_ndc(neighbor_warp_ndc)),
                detail,
            );
            result.color = mix(result.color, neighbor.color, 0.5);
        }
    }
    return WarpFragmentOutput(result.color, result.distance_meters);
}

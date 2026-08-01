const RAYMARCH_REFINEMENT_COUNT: u32 = 5u;
// How many ray footprints wide the detail filter is. The raster path's
// TERRAIN_DETAIL_FILTER_RATIO of 0.01 works out to about seven of its pixels at
// a 60 degree vertical field over 720 lines, and that oversampling is why its
// ground does not crawl. The ray path has to match it against its own footprint
// rather than inherit the constant: it renders into a smaller warped buffer, so
// a ray covers more angle than a raster pixel, and more again in the periphery.
const RAY_DETAIL_FILTER_OVERSAMPLE: f32 = 7.0;
const RAY_DETAIL_MIN_INCIDENCE: f32 = 0.06;
// Twelve, not six. The comb's resolution is its span over this count, and the
// span tracks the relief actually present. When the mountains went from 313m
// of relief within 2km to 1001m, six samples became three times coarser
// without anything saying so -- p90 on path_parity_ridge went to 20m against a
// 6m tolerance while the raster path held.
const RAY_DETAIL_HIT_STEPS: i32 = 12;
const RAY_DETAIL_HIT_REFINEMENTS: i32 = 4;
const RAY_DETAIL_FRONT_SEARCH_STEPS: i32 = 8;
/// How much further than the relief measured at the macro hit the detail walk
/// may reach, to cover the ladder standing taller further along the ray.
const RAY_DETAIL_HIT_REACH_FACTOR: f32 = 3.0;
/// Relief below which the macro hit is already the answer.
const RAY_DETAIL_HIT_MIN_RELIEF_METERS: f32 = 0.05;
const RAY_SKY_SAMPLE_COUNT: u32 = 16u;
const RAY_SKY_DENSITY_SAMPLE_EXPONENT: f32 = 3.0;
const RAY_ANTISOLAR_TWILIGHT_MIN_SCATTER: f32 = 0.48;
const RAY_SKY_ATMOSPHERE_SATURATION: f32 = 1.3;
const RAY_OCEAN_SHELL_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + 1.0;
const NEAR_FIELD_BLOCKS: u32 = 8u;
const RENDER_DEBUG_SKY_ONLY: u32 = 4u;
const RENDER_DEBUG_RAY_HIT_STATUS: u32 = 5u;
const DETAIL_HIT_STATUS_NONE: u32 = 0u;
const DETAIL_HIT_STATUS_NO_RELIEF: u32 = 1u;
const DETAIL_HIT_STATUS_BRACKETED: u32 = 2u;
const DETAIL_HIT_STATUS_FALLBACK: u32 = 3u;
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
    near_field_uv_origin: vec2<f32>,
    near_field_uv_span: f32,
    near_field_max_height_meters: f32,
    near_field_face: u32,
    near_field_enabled: u32,
    near_field_samples: u32,
    _near_field_padding: u32,
    near_field_source_levels: array<vec4<u32>, 16>,
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
@group(1) @binding(5)
var near_field_height: texture_2d<f32>;
@group(1) @binding(6)
var near_field_biome: texture_2d<u32>;
@group(1) @binding(7)
var near_field_moisture: texture_2d<f32>;
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

struct DetailHit {
    distance_meters: f32,
    status: u32,
}

struct FaceUv {
    face: u32,
    uv: vec2<f32>,
}

struct OceanHit {
    distance_meters: f32,
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

/// The camera's own surface point, used as the anchor for the synthesised
/// detail ladder. The ladder needs an exact cell index plus a short local
/// offset; the raster path takes both from its node, and the ray path has no
/// node, so the camera serves instead.
fn ray_detail_anchor() -> vec3<f32> {
    return normalize(view_to_planet(camera.camera_planet_direction_view_altitude.xyz));
}

/// Offset from that anchor's surface point to a point on the ray, in metres,
/// **projected back onto the reference sphere**.
///
/// The projection is the whole point. The detail field is a function of
/// direction alone -- the CPU evaluates `domain(direction) * PLANET_RADIUS` and
/// the raster path passes `(direction - anchor_direction) * PLANET_RADIUS`,
/// which sums to the same thing. This used to pass the true 3D offset instead,
/// which carries the terrain's own elevation, so the ray path sampled the field
/// at `domain(direction) * (PLANET_RADIUS + height)`. Over 2900m of ground that
/// is eleven whole cells at the coarsest octave: a completely different piece of
/// noise, of exactly the right amplitude and character, sliding as the elevation
/// changes. The surface probe measured its correlation with the raster path's
/// relief at 0.0 while the raster reached 0.96.
///
/// Built additively rather than by subtracting two absolute positions. The
/// camera sits 4e6 m from the planet centre, so differencing absolute points
/// loses the metre-scale offset the fine octaves live in; the altitude and the
/// camera-relative offset are both small and stay exact.
fn ray_detail_local_meters(view_offset: vec3<f32>) -> vec3<f32> {
    let anchor = ray_detail_anchor();
    let offset_meters = anchor * camera.camera_planet_direction_view_altitude.w
        + view_to_planet(view_offset);
    // Work in planet radii, where the anchor is the unit vector, so the
    // projection is a normalize of `anchor + relative`.
    let relative = offset_meters * (1.0 / PLANET_RADIUS_METERS);
    // `length(anchor + relative)^2 - 1`, formed without ever computing the
    // near-one length and subtracting: for a point a few metres away that
    // cancellation would take the entire offset with it.
    let excess = 2.0 * dot(anchor, relative) + dot(relative, relative);
    let scale = sqrt(1.0 + excess);
    let one_minus_scale = -excess / (1.0 + scale);
    return (relative + anchor * one_minus_scale) * (PLANET_RADIUS_METERS / scale);
}

/// Synthesised relief at a point on the ray, filtered to the spacing that point
/// is being sampled at. Same ladder and same filter the raster path displaces
/// with, so the two describe one planet rather than two.
/// Spacing of the baked samples this point is being drawn from, so the ladder
/// starts where the baked data stops. The window is much finer than the dense
/// faces, so this follows the same blend the height sampling does.
fn ray_baked_spacing_meters(surface_direction: vec3<f32>) -> f32 {
    let face_spacing = 2.0 * PLANET_RADIUS_METERS / f32(ray_settings.face_quads);
    if ray_settings.near_field_enabled == 0u {
        return face_spacing;
    }
    let weight = near_field_weight(direction_to_face_uv(surface_direction));
    if weight <= 0.0 {
        return face_spacing;
    }
    let face_uv = direction_to_face_uv(surface_direction);
    let source_level = near_field_sample_source_level(face_uv);
    let source_spacing = continuous_baked_sample_spacing_meters(
        face_uv.uv,
        source_level,
        true,
    );
    return mix(face_spacing, source_spacing, weight);
}

fn ray_terrain_detail(
    view_offset: vec3<f32>,
    surface_direction: vec3<f32>,
    scaled_macro_height_meters: f32,
    footprint_radians: f32,
) -> TerrainDetail {
    if scaled_macro_height_meters <= 0.0 {
        return TerrainDetail(0.0, vec3<f32>(0.0));
    }
    let distance_meters = length(view_offset);
    // Deliberately no 1/sin(incidence) widening. It is the textbook correction
    // for a grazing ray's projected footprint, and it was tried here -- but the
    // raster path does not do it, does not alias, and a side-by-side outside the
    // sparse corridor showed the widened version washing the relief out of
    // exactly the grazing views that most need it. Matching the raster filter is
    // what parity means; this is a parity feature.
    let filter_meters = max(
        distance_meters * footprint_radians * RAY_DETAIL_FILTER_OVERSAMPLE,
        TERRAIN_DETAIL_MIN_FILTER_METERS,
    );
    let detail = terrain_detail(
        ray_detail_anchor(),
        ray_detail_local_meters(view_offset),
        filter_meters,
        ray_baked_spacing_meters(surface_direction),
        scaled_macro_height_meters,
    );
    return detail;
}

/// Fraction of this point that the near-field window covers.
///
/// Zero outside the window and one well inside it, with a fade band at the
/// border so the hundred-metre step between the fine window and the coarse
/// pyramid arrives as a ramp rather than a wall. The band is a fixed fraction
/// of the window, so it scales with whatever level the window is at.
const NEAR_FIELD_FADE: f32 = 0.06;

fn near_field_weight(face_uv: FaceUv) -> f32 {
    if ray_settings.near_field_enabled == 0u
        || face_uv.face != ray_settings.near_field_face
        || ray_settings.near_field_uv_span <= 0.0 {
        return 0.0;
    }
    let window_uv = (face_uv.uv - ray_settings.near_field_uv_origin)
        / ray_settings.near_field_uv_span;
    if any(window_uv < vec2<f32>(0.0)) || any(window_uv > vec2<f32>(1.0)) {
        return 0.0;
    }
    let edge = min(
        min(window_uv.x, 1.0 - window_uv.x),
        min(window_uv.y, 1.0 - window_uv.y),
    );
    return smoothstep(0.0, NEAR_FIELD_FADE, edge);
}

fn near_field_sample_height(face_uv: FaceUv) -> f32 {
    let window_uv = (face_uv.uv - ray_settings.near_field_uv_origin)
        / ray_settings.near_field_uv_span;
    let last = f32(ray_settings.near_field_samples - 1u);
    let coordinate = clamp(window_uv, vec2<f32>(0.0), vec2<f32>(1.0)) * last;
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(lower + vec2<i32>(1), vec2<i32>(i32(last)));
    let amount = fract(coordinate);
    let h00 = textureLoad(near_field_height, lower, 0).x;
    let h10 = textureLoad(near_field_height, vec2<i32>(upper.x, lower.y), 0).x;
    let h01 = textureLoad(near_field_height, vec2<i32>(lower.x, upper.y), 0).x;
    let h11 = textureLoad(near_field_height, upper, 0).x;
    return mix(mix(h00, h10, amount.x), mix(h01, h11, amount.x), amount.y);
}

fn near_field_texel_coordinate(face_uv: FaceUv) -> vec2<f32> {
    let window_uv = (face_uv.uv - ray_settings.near_field_uv_origin)
        / ray_settings.near_field_uv_span;
    let last = f32(ray_settings.near_field_samples - 1u);
    return clamp(window_uv, vec2<f32>(0.0), vec2<f32>(1.0)) * last;
}

fn near_field_sample_source_level(face_uv: FaceUv) -> u32 {
    let window_uv = clamp(
        (face_uv.uv - ray_settings.near_field_uv_origin)
            / ray_settings.near_field_uv_span,
        vec2<f32>(0.0),
        vec2<f32>(0.999999),
    );
    let block = vec2<u32>(floor(window_uv * f32(NEAR_FIELD_BLOCKS)));
    let index = block.y * NEAR_FIELD_BLOCKS + block.x;
    return ray_settings.near_field_source_levels[index / 4u][index % 4u];
}

fn near_field_sample_biome(face_uv: FaceUv) -> u32 {
    let coordinate = vec2<i32>(round(near_field_texel_coordinate(face_uv)));
    return textureLoad(near_field_biome, coordinate, 0).x;
}

fn near_field_sample_biome_blend(face_uv: FaceUv) -> BiomeBlendSample {
    let coordinate = near_field_texel_coordinate(face_uv);
    let lower = vec2<i32>(floor(coordinate));
    let last = i32(ray_settings.near_field_samples - 1u);
    let upper = min(lower + vec2<i32>(1), vec2<i32>(last));
    let amount = fract(coordinate);
    return BiomeBlendSample(
        vec4<u32>(
            textureLoad(near_field_biome, lower, 0).x,
            textureLoad(near_field_biome, vec2<i32>(upper.x, lower.y), 0).x,
            textureLoad(near_field_biome, vec2<i32>(lower.x, upper.y), 0).x,
            textureLoad(near_field_biome, upper, 0).x,
        ),
        vec4<f32>(
            (1.0 - amount.x) * (1.0 - amount.y),
            amount.x * (1.0 - amount.y),
            (1.0 - amount.x) * amount.y,
            amount.x * amount.y,
        ),
    );
}

fn near_field_sample_moisture(face_uv: FaceUv) -> f32 {
    let coordinate = near_field_texel_coordinate(face_uv);
    let lower = vec2<i32>(floor(coordinate));
    let last = i32(ray_settings.near_field_samples - 1u);
    let upper = min(lower + vec2<i32>(1), vec2<i32>(last));
    let amount = fract(coordinate);
    let lower_left = textureLoad(near_field_moisture, lower, 0).x;
    let lower_right = textureLoad(
        near_field_moisture,
        vec2<i32>(upper.x, lower.y),
        0,
    ).x;
    let upper_left = textureLoad(
        near_field_moisture,
        vec2<i32>(lower.x, upper.y),
        0,
    ).x;
    let upper_right = textureLoad(near_field_moisture, upper, 0).x;
    return mix(
        mix(lower_left, lower_right, amount.x),
        mix(upper_left, upper_right, amount.x),
        amount.y,
    );
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
    let coarse = mix(mix(h00, h10, amount.x), mix(h01, h11, amount.x), amount.y);
    // The dense pyramid is 3068m per texel, which reads 815m at the landing
    // site where the finest baked data reads 920m. That gap was the whole of
    // this path's disagreement with the ground the camera stands on.
    let weight = near_field_weight(face_uv);
    if weight <= 0.0 {
        return coarse;
    }
    return mix(coarse, near_field_sample_height(face_uv), weight);
}

fn sample_biome(direction: vec3<f32>) -> u32 {
    let face_uv = direction_to_face_uv(direction);
    let coordinate = vec2<i32>(round(face_texel_coordinate(face_uv)));
    let coarse = textureLoad(biome_faces, coordinate, i32(face_uv.face), 0).x;
    if near_field_weight(face_uv) < 0.5 {
        return coarse;
    }
    return near_field_sample_biome(face_uv);
}

fn sample_biome_blend(direction: vec3<f32>) -> BiomeBlendSample {
    let face_uv = direction_to_face_uv(direction);
    let coordinate = face_texel_coordinate(face_uv);
    let lower = vec2<i32>(floor(coordinate));
    let upper = lower + vec2<i32>(1);
    let amount = fract(coordinate);
    let coarse = BiomeBlendSample(
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
    if near_field_weight(face_uv) < 0.5 {
        return coarse;
    }
    return near_field_sample_biome_blend(face_uv);
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
    let coarse = mix(
        mix(lower_left, lower_right, amount.x),
        mix(upper_left, upper_right, amount.x),
        amount.y,
    );
    let weight = near_field_weight(face_uv);
    if weight <= 0.0 {
        return coarse;
    }
    return mix(coarse, near_field_sample_moisture(face_uv), weight);
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
    let coarse_bound = textureLoad(
        max_height_faces,
        start_texel,
        i32(start_face_uv.face),
        i32(mip_level),
    ).x;
    // The window can raise the ground a hundred metres above what the coarse
    // pyramid's maximum promised. Empty-space skipping trusts that maximum, so
    // without this the marcher steps straight through the terrain it is meant
    // to be finding.
    return max(coarse_bound, near_field_max_height_bound(start_face_uv, end_face_uv));
}

/// The window's own conservative ceiling, or a floor when this step is nowhere
/// near it. The rectangle is widened by the fade band, so a step that clips a
/// corner is still covered.
fn near_field_max_height_bound(start_face_uv: FaceUv, end_face_uv: FaceUv) -> f32 {
    if ray_settings.near_field_enabled == 0u {
        return ray_settings.height_min_meters;
    }
    let margin = ray_settings.near_field_uv_span * NEAR_FIELD_FADE;
    let low = ray_settings.near_field_uv_origin - vec2<f32>(margin);
    let high = ray_settings.near_field_uv_origin
        + vec2<f32>(ray_settings.near_field_uv_span + margin);
    let inside_start = start_face_uv.face == ray_settings.near_field_face
        && all(start_face_uv.uv >= low)
        && all(start_face_uv.uv <= high);
    let inside_end = end_face_uv.face == ray_settings.near_field_face
        && all(end_face_uv.uv >= low)
        && all(end_face_uv.uv <= high);
    if inside_start || inside_end {
        return ray_settings.near_field_max_height_meters;
    }
    return ray_settings.height_min_meters;
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
    let maximum_height_meters = scaled_terrain_macro_height(sample_max_height(
        normalize(view_to_planet(point_view)),
        normalize(view_to_planet(end_point_view)),
        desired_step_meters,
    ));
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
        + scaled_terrain_macro_height(sample_height(surface_direction));
    return radius_at(distance_meters, radial_dot_ray) - surface_radius;
}

/// The macro surface plus synthesised detail, as a signed distance along the ray.
fn detail_surface_function(
    distance_meters: f32,
    radial_dot_ray: f32,
    camera_position_view: vec3<f32>,
    ray: vec3<f32>,
    footprint_radians: f32,
) -> f32 {
    let view_offset = ray * distance_meters;
    let point_view = camera_position_view + view_offset;
    let surface_direction = normalize(view_to_planet(point_view));
    let macro_height = scaled_terrain_macro_height(sample_height(surface_direction));
    let detail = ray_terrain_detail(
        view_offset,
        surface_direction,
        macro_height,
        footprint_radians,
    );
    return radius_at(distance_meters, radial_dot_ray)
        - (PLANET_RADIUS_METERS + macro_height + detail.height_meters);
}

/// Finds the first detailed-surface entry around the macro hit.
///
/// The old comb trusted the detail sign at the macro crossing and searched
/// only one direction. A non-monotonic relief field can rise through the ray
/// and fall out again before that point, so the sign does not identify the
/// first visible crossing. Search the same bounded interval from front to back
/// instead, then refine its first outside-to-inside sign change.
fn refine_detail_hit(
    macro_hit_meters: f32,
    radial_dot_ray: f32,
    camera_position_view: vec3<f32>,
    ray: vec3<f32>,
    footprint_radians: f32,
) -> DetailHit {
    let value = detail_surface_function(
        macro_hit_meters,
        radial_dot_ray,
        camera_position_view,
        ray,
        footprint_radians,
    );
    // No relief here means the macro hit already is the detailed surface.
    // Walking would find nothing and cost six ladder evaluations doing it.
    // Inside the sparse corridor the ladder's high cut zeroes the detail
    // outright, so this is the common case there rather than an edge case.
    if abs(value) < RAY_DETAIL_HIT_MIN_RELIEF_METERS {
        return DetailHit(macro_hit_meters, DETAIL_HIT_STATUS_NO_RELIEF);
    }
    let hit_direction = normalize(view_to_planet(camera_position_view + ray * macro_hit_meters));
    let incidence = max(
        abs(dot(normalize(view_to_planet(ray)), hit_direction)),
        RAY_DETAIL_MIN_INCIDENCE,
    );
    // Walk by the relief that is actually here, not by the most the ladder
    // could ever produce anywhere. At the macro hit `value` is minus the local
    // detail height, and a ray closes a height gap at a rate set by its
    // incidence, so |value| / incidence is where the crossing should sit. The
    // global amplitude stays on as a ceiling, because the ladder can be taller
    // further along the ray than it is at this one point.
    //
    // A previous hill-band experiment expanded this ceiling without rescaling
    // the walk, so grazing rays jumped past crossings and produced quality
    // steps locked to viewing angle. The local-relief span prevents that fault
    // if directional coarse relief is added again later.
    let span_meters = min(
        abs(value) * RAY_DETAIL_HIT_REACH_FACTOR,
        TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
    ) / incidence;
    let conservative_start = max(
        macro_hit_meters - TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS / incidence,
        0.0,
    );
    let conservative_end =
        macro_hit_meters + TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS / incidence;
    let search_start = max(macro_hit_meters - span_meters, 0.0);
    let search_end = macro_hit_meters + span_meters;
    var bracket_near = search_start;
    var value_near = detail_surface_function(
        search_start,
        radial_dot_ray,
        camera_position_view,
        ray,
        footprint_radians,
    );
    var bracket_far = search_end;
    var found = false;
    if value_near <= 0.0 {
        // Local relief can already have risen through the ray before the
        // value-sized interval. Expand toward the conservative amplitude
        // bound until the outside side of that first encountered entry is
        // found; returning the macro hit here produced the camera-locked red
        // fallback bands in the parity diagnostic.
        var front_distance = search_start;
        var front_step = max(span_meters, RAY_DETAIL_HIT_MIN_RELIEF_METERS);
        for (
            var step = 0;
            step < RAY_DETAIL_FRONT_SEARCH_STEPS && !found;
            step = step + 1
        ) {
            let candidate = max(
                front_distance - front_step,
                conservative_start,
            );
            let candidate_value = detail_surface_function(
                candidate,
                radial_dot_ray,
                camera_position_view,
                ray,
                footprint_radians,
            );
            if candidate_value > 0.0 {
                bracket_near = candidate;
                value_near = candidate_value;
                bracket_far = front_distance;
                found = true;
                break;
            }
            front_distance = candidate;
            front_step = front_step * 2.0;
            if front_distance <= conservative_start {
                break;
            }
        }
    }
    for (
        var step = 1;
        step <= RAY_DETAIL_HIT_STEPS && !found;
        step = step + 1
    ) {
        let candidate = mix(
            search_start,
            search_end,
            f32(step) / f32(RAY_DETAIL_HIT_STEPS),
        );
        let candidate_value = detail_surface_function(
            candidate,
            radial_dot_ray,
            camera_position_view,
            ray,
            footprint_radians,
        );
        if candidate_value <= 0.0 && value_near > 0.0 {
            bracket_far = candidate;
            found = true;
            break;
        }
        if value_near > 0.0 {
            bracket_near = candidate;
            value_near = candidate_value;
        }
    }
    if !found && value_near > 0.0 {
        // Negative local relief can move the entry farther behind the macro
        // root than the value-sized interval predicted. Continue in
        // front-to-back order, doubling the reach but never exceeding the
        // ladder's conservative amplitude bound.
        var back_distance = search_end;
        var back_step = max(span_meters, RAY_DETAIL_HIT_MIN_RELIEF_METERS);
        for (
            var step = 0;
            step < RAY_DETAIL_FRONT_SEARCH_STEPS && !found;
            step = step + 1
        ) {
            let candidate = min(
                back_distance + back_step,
                conservative_end,
            );
            let candidate_value = detail_surface_function(
                candidate,
                radial_dot_ray,
                camera_position_view,
                ray,
                footprint_radians,
            );
            if candidate_value <= 0.0 {
                bracket_near = back_distance;
                bracket_far = candidate;
                found = true;
                break;
            }
            back_distance = candidate;
            bracket_near = candidate;
            value_near = candidate_value;
            back_step = back_step * 2.0;
            if back_distance >= conservative_end {
                break;
            }
        }
    }
    if !found {
        // The ladder never crossed the ray inside its own amplitude. Keep the
        // macro hit rather than inventing a surface that is not there.
        return DetailHit(macro_hit_meters, DETAIL_HIT_STATUS_FALLBACK);
    }
    var lower = min(bracket_near, bracket_far);
    var upper = max(bracket_near, bracket_far);
    for (var index = 0; index < RAY_DETAIL_HIT_REFINEMENTS; index = index + 1) {
        let middle = 0.5 * (lower + upper);
        let middle_value = detail_surface_function(
            middle,
            radial_dot_ray,
            camera_position_view,
            ray,
            footprint_radians,
        );
        // Above the surface means the crossing is further along the ray.
        if middle_value > 0.0 {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    return DetailHit(
        0.5 * (lower + upper),
        DETAIL_HIT_STATUS_BRACKETED,
    );
}

fn detail_hit_status_color(status: u32) -> vec3<f32> {
    if status == DETAIL_HIT_STATUS_BRACKETED {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    if status == DETAIL_HIT_STATUS_FALLBACK {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    if status == DETAIL_HIT_STATUS_NO_RELIEF {
        return vec3<f32>(1.0, 0.75, 0.0);
    }
    return vec3<f32>(0.0);
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

fn terrain_normal(
    surface_direction: vec3<f32>,
    hit_view_position: vec3<f32>,
) -> vec3<f32> {
    let reference_axis = select(
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(1.0, 0.0, 0.0),
        abs(surface_direction.y) > 0.95,
    );
    let east = normalize(cross(reference_axis, surface_direction));
    let north = normalize(cross(surface_direction, east));
    // Match raster's continuously distance-filtered normal footprint while
    // never probing below the resolved baked sample spacing.
    let source_sample_meters = min(
        ray_baked_spacing_meters(surface_direction),
        TERRAIN_NORMAL_MAX_SAMPLE_METERS,
    );
    let normal_sample_meters = clamp(
        length(hit_view_position) * 0.01,
        max(TERRAIN_NORMAL_MIN_SAMPLE_METERS, source_sample_meters),
        TERRAIN_NORMAL_MAX_SAMPLE_METERS,
    );
    let epsilon = normal_sample_meters / PLANET_RADIUS_METERS;
    let east_direction = normalize(surface_direction + east * epsilon);
    let north_direction = normalize(surface_direction + north * epsilon);
    let height = scaled_terrain_macro_height(sample_height(surface_direction));
    let east_height = scaled_terrain_macro_height(sample_height(east_direction));
    let north_height = scaled_terrain_macro_height(sample_height(north_direction));
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
        return OceanHit(-1.0);
    }
    let shell_direction = normalize(view_to_planet(
        camera_position_view + ray * shell_distance,
    ));
    let macro_height = sample_height(shell_direction);
    let biome_id = sample_biome(shell_direction);
    if !is_open_ocean_surface(true, macro_height, biome_id) {
        return OceanHit(-1.0);
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
                return OceanHit(-1.0);
            }
        }
    }
    return OceanHit(distance_meters);
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
    // Angular width of this ray, from screen-space derivatives, so foveated
    // periphery filters harder than the fovea without being told about warping.
    footprint_radians: f32,
) -> vec3<f32> {
    let render_debug_mode = u32(camera.projection.w + 0.5);
    let macro_height_meters = sample_height(surface_direction);
    // Same synthesised ladder the raster path uses, anchored on the camera
    // rather than on a node. This is what the ray path was missing entirely:
    // its height field is the dense level only, about 3 km per texel.
    let detail = ray_terrain_detail(
        hit_view_position,
        surface_direction,
        scaled_terrain_macro_height(macro_height_meters),
        footprint_radians,
    );
    let detail_normal = terrain_detail_perturbed_normal(
        normal,
        surface_direction,
        detail.slope,
    );
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
        detail.height_meters,
        detail_normal,
        surface_direction,
    );
    let detail_tint = terrain_material_tint(
        true,
        moisture,
        biome_blend,
        macro_height_meters,
        terrain_albedo,
        surface_direction,
        detail_normal,
        hit_view_position,
        detail.height_meters,
        // The close-range tile is now addressable here too: the camera anchor
        // plus the camera-relative hit offset gives the same exact split the
        // raster path builds from its node.
        ray_detail_anchor(),
        ray_detail_local_meters(hit_view_position),
        terrain_material_fine_weight(length(hit_view_position)),
    );
    let textured_albedo = terrain_albedo * detail_tint;
    if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
        return textured_albedo;
    }

    let surface_height = scaled_terrain_macro_height(macro_height_meters);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_transmittance = surface_direct_sun_transmittance(
        surface_direction,
        surface_height,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        detail_normal,
        surface_direction,
        surface_height,
        sun_direction,
    );
    // Lit by the detail normal, not the macro one. Shading the relief is the
    // whole point of evaluating it -- the raster path does the same, per pixel.
    let surface_irradiance = sky_diffuse
        + sun_transmittance
            * max(dot(detail_normal, sun_direction), 0.0)
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

fn trace_ray(ray: vec3<f32>, detail: f32, footprint_radians: f32) -> RayResult {
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
    var detail_hit_status = DETAIL_HIT_STATUS_NONE;
    // The camera can stand above the detailed surface while sitting below the
    // macro one: the ladder subtracts as well as adds, and at the landing site
    // it is -2.3m under a camera standing 2m up. The macro march is only an
    // accelerator and it assumes it starts outside its own shell, so when that
    // is false there is no macro crossing to find and the detail walk is the
    // only thing that can answer. It reports "no bracket" by returning its own
    // starting distance, which is the case to fall through on -- a ray looking
    // out at the horizon from here leaves the shell without ever crossing it,
    // and a camera genuinely underground has no hit to find either.
    //
    // This only became reachable once the near-field window raised the macro
    // surface to where it belongs. Against the 3068m pyramid the ground sat a
    // hundred metres below any camera and the question never arose.
    var started_inside_macro_shell = false;
    if previous_value < 0.0 {
        let near_hit = refine_detail_hit(
            start_distance,
            radial_dot_ray,
            camera_position_view,
            ray,
            footprint_radians,
        );
        detail_hit_status = near_hit.status;
        if near_hit.distance_meters > start_distance {
            hit_distance = near_hit.distance_meters;
            started_inside_macro_shell = true;
        }
    }
    let baseline_step_meters = (end_distance - start_distance)
        / f32(ray_settings.march_steps);
    for (var index = 0u; index < 192u; index += 1u) {
        if index >= ray_settings.march_steps || started_inside_macro_shell {
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
    if hit_distance >= 0.0 && !started_inside_macro_shell {
        let detail_hit = refine_detail_hit(
            hit_distance,
            radial_dot_ray,
            camera_position_view,
            ray,
            footprint_radians,
        );
        hit_distance = detail_hit.distance_meters;
        detail_hit_status = detail_hit.status;
    }
    let water_hit = ocean_hit(radial_dot_ray, camera_position_view, ray, detail);
    if water_hit.distance_meters >= 0.0
        && (hit_distance < 0.0 || water_hit.distance_meters <= hit_distance)
    {
        if render_debug_mode == RENDER_DEBUG_RAY_HIT_STATUS {
            return RayResult(
                vec4<f32>(0.0, 0.5, 1.0, 1.0),
                water_hit.distance_meters,
            );
        }
        let water_view = camera_position_view + ray * water_hit.distance_meters;
        let water_direction = normalize(view_to_planet(water_view));
        let color = shade_ocean(
            water_direction,
            ray * water_hit.distance_meters,
            0.0,
            detail,
        );
        return RayResult(vec4<f32>(color, 1.0), water_hit.distance_meters);
    }
    if hit_distance < 0.0 {
        if render_debug_mode == RENDER_DEBUG_RAY_HIT_STATUS {
            return RayResult(
                vec4<f32>(detail_hit_status_color(detail_hit_status), 1.0),
                -1.0,
            );
        }
        return RayResult(
            vec4<f32>(ray_atmosphere_radiance(ray, radial_dot_ray, detail), 1.0),
            -1.0,
        );
    }

    if render_debug_mode == RENDER_DEBUG_RAY_HIT_STATUS {
        return RayResult(
            vec4<f32>(detail_hit_status_color(detail_hit_status), 1.0),
            hit_distance,
        );
    }

    let hit_view = camera_position_view + ray * hit_distance;
    let surface_direction = normalize(view_to_planet(hit_view));
    let normal = terrain_normal(surface_direction, ray * hit_distance);
    let macro_height = sample_height(surface_direction);
    let biome = sample_biome(surface_direction);
    let terrain_color = shade_terrain(
        surface_direction,
        normal,
        ray * hit_distance,
        footprint_radians,
    );
    var color = terrain_color;
    if biome == 1u {
        color = shade_ocean(
            surface_direction,
            ray * hit_distance,
            scaled_terrain_macro_height(macro_height),
            detail,
        );
    }
    return RayResult(vec4<f32>(color, 1.0), hit_distance);
}

@fragment
fn fs_main(input: VertexOutput) -> FragmentOutput {
    let ray = view_direction(input.ndc);
    // Derivatives must be taken in uniform control flow, so the footprint is
    // measured here before any of the marching branches.
    let footprint_radians = max(length(dpdx(ray)), length(dpdy(ray)));
    let result = trace_ray(ray, 1.0, footprint_radians);
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
    // Taken from the warped ray, so peripheral rays report the wider footprint
    // the warp actually gives them and filter themselves accordingly.
    let footprint_radians = max(length(dpdx(ray)), length(dpdy(ray)));
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
    var result = trace_ray(ray, detail, footprint_radians);
    if (ray_settings.experiment_flags & EXPERIMENT_HORIZON_DENSITY) != 0u {
        if abs(closest_radius - PLANET_RADIUS_METERS) < 30000.0 {
            let neighbor_warp_ndc = input.ndc + vec2<f32>(
                dpdx(input.ndc.x),
                dpdy(input.ndc.y),
            ) * 0.35;
            let neighbor = trace_ray(
                view_direction(warped_screen_ndc(neighbor_warp_ndc)),
                detail,
                footprint_radians,
            );
            result.color = mix(result.color, neighbor.color, 0.5);
        }
    }
    return WarpFragmentOutput(result.color, result.distance_meters);
}

// Immediate deterministic forest geometry. Candidate ownership is canonical
// per L12 cell; the CPU uploads only cell/source descriptors, never trees.
struct GpuForestUniform {
    camera_planet_position: vec4<f32>,
}

struct GpuForestCell {
    cell_uv_origin_span: vec4<f32>,
    source_uv_scale_offset: vec4<f32>,
    anchor_direction_source_level: vec4<f32>,
    key: vec4<u32>,
}

@group(1) @binding(3)
var<uniform> gpu_forest: GpuForestUniform;

@group(1) @binding(4)
var<storage, read> gpu_forest_cells: array<GpuForestCell>;

fn gpu_forest_hash_u32(input: u32) -> u32 {
    var value = input;
    value = value ^ (value >> 16u);
    value = value * 0x7feb352du;
    value = value ^ (value >> 15u);
    value = value * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return value;
}

fn gpu_forest_hash(value: u32) -> f32 {
    return f32(gpu_forest_hash_u32(value)) * (1.0 / 4294967295.0);
}

fn gpu_forest_unit_hash(value: u32) -> f32 {
    return f32(gpu_forest_hash_u32(value)) * (1.0 / 4294967296.0);
}

fn gpu_forest_cell_seed(cell: GpuForestCell) -> u32 {
    return gpu_forest_hash_u32(
        0x6d2b79f5u
            ^ cell.key.x * 0x9e3779b9u
            ^ 12u * 0x85ebca6bu
            ^ cell.key.y * 0xc2b2ae35u
            ^ cell.key.z * 0x27d4eb2fu,
    );
}

fn gpu_forest_biome_owns_trees(biome: u32) -> bool {
    return biome == 2u || biome == 3u || biome == 4u
        || biome == 5u || biome == 6u || biome == 9u;
}

fn gpu_forest_evergreen(biome: u32) -> bool {
    return biome == 2u || biome == 3u || biome == 9u;
}


struct GpuForestTree {
    centre_and_height: vec4<f32>,
    width_shade_kind_seed: vec4<f32>,
}

@group(1) @binding(5)
var<storage, read_write> gpu_forest_trees: array<GpuForestTree>;

fn gpu_forest_invalid_tree() -> GpuForestTree {
    return GpuForestTree(vec4<f32>(0.0), vec4<f32>(0.0));
}

fn gpu_forest_generate(
    invocation_index: u32,
    candidates_per_cell: u32,
    candidate_stride: u32,
) {
    let cell_index = invocation_index / candidates_per_cell;
    let candidate_in_cell = invocation_index % candidates_per_cell;
    let candidate_index = candidate_in_cell * candidate_stride;
    let cell = gpu_forest_cells[cell_index];
    let output_index = cell.key.w + candidate_in_cell;
    let cell_seed = gpu_forest_cell_seed(cell);
    let candidate_seed = cell_seed ^ candidate_index;
    let local_uv = vec2<f32>(
        gpu_forest_unit_hash(cell_seed ^ candidate_index ^ 0x6a09e667u),
        gpu_forest_unit_hash(cell_seed ^ candidate_index ^ 0xbb67ae85u),
    );
    let face_uv = cell.cell_uv_origin_span.xy + local_uv * cell.cell_uv_origin_span.z;
    let face = cell.key.x;
    let direction = normalize(
        face_normal(face)
            + face_tangent_u(face) * face_uv.x
            + face_tangent_v(face) * face_uv.y,
    );
    let density = forest_density_at_direction(direction);
    let placement_seed = gpu_forest_hash(candidate_seed ^ 0x165667b1u);
    if placement_seed > density {
        gpu_forest_trees[output_index] = gpu_forest_invalid_tree();
        return;
    }

    let source_uv = cell.source_uv_scale_offset.zw
        + local_uv * cell.source_uv_scale_offset.xy;
    let biome = sample_biome(source_uv);
    let moisture = sample_moisture(source_uv);
    let macro_height = macro_terrain_height(true, source_uv, direction);
    if !gpu_forest_biome_owns_trees(biome)
        || moisture < 0.38
        || macro_height <= 0.0
    {
        gpu_forest_trees[output_index] = gpu_forest_invalid_tree();
        return;
    }

    let source_level_value = u32(cell.anchor_direction_source_level.w + 0.5);
    let terrain_info = 1u
        | (face << 1u)
        | (12u << 4u)
        | (source_level_value << 9u);
    let anchor_direction = normalize(cell.anchor_direction_source_level.xyz);
    let anchor_relative_position = (direction - anchor_direction) * PLANET_RADIUS_METERS;
    let base_height = scaled_terrain_macro_height(macro_height);
    let base_planet_position = direction * (PLANET_RADIUS_METERS + base_height);
    let camera_distance_meters = length(
        base_planet_position - gpu_forest.camera_planet_position.xyz,
    );
    let detail = terrain_detail(
        anchor_direction,
        anchor_relative_position,
        terrain_detail_filter_meters(camera_distance_meters),
        continuous_baked_sample_spacing_meters(
            face_uv,
            source_level_value,
            false,
        ),
        base_height,
    );
    let surface_height = base_height + detail.height_meters;
    var surface_normal = displaced_surface_normal(
        direction,
        source_uv,
        cell.source_uv_scale_offset.xy,
        terrain_info,
        camera_distance_meters,
    );
    surface_normal = terrain_detail_perturbed_normal(
        surface_normal,
        direction,
        detail.slope,
    );
    let slope_cosine = clamp(dot(surface_normal, direction), 0.0, 1.0);
    if slope_cosine < 0.8480481 {
        gpu_forest_trees[output_index] = gpu_forest_invalid_tree();
        return;
    }

    let height = 22.0 + gpu_forest_hash(candidate_seed ^ 0xa511e9b3u) * 26.0;
    let width = height * (0.32 + gpu_forest_hash(candidate_seed ^ 0x63d83595u) * 0.18);
    let shade = 0.82 + gpu_forest_hash(candidate_seed ^ 0x9e3779b9u) * 0.34;
    var kind = select(0.0, 1.0, gpu_forest_hash(candidate_seed ^ 0x27d4eb2fu) < 0.28);
    if gpu_forest_evergreen(biome) {
        kind = 1.0;
    }
    let slope_tangent = sqrt(max(1.0 - slope_cosine * slope_cosine, 0.0))
        / max(slope_cosine, 1.0e-4);
    let sink = 0.45 + width * 0.5 * slope_tangent;
    let centre = direction * (PLANET_RADIUS_METERS + surface_height - sink);
    gpu_forest_trees[output_index] = GpuForestTree(
        vec4<f32>(centre, height),
        vec4<f32>(width, shade, kind, placement_seed),
    );
}

@compute @workgroup_size(64)
fn forest_gpu_compute_full(@builtin(global_invocation_id) id: vec3<u32>) {
    gpu_forest_generate(id.x, 12288u, 1u);
}

@compute @workgroup_size(64)
fn forest_gpu_compute_medium(@builtin(global_invocation_id) id: vec3<u32>) {
    gpu_forest_generate(id.x, 768u, 16u);
}

@compute @workgroup_size(64)
fn forest_gpu_compute_sparse(@builtin(global_invocation_id) id: vec3<u32>) {
    gpu_forest_generate(id.x, 64u, 192u);
}

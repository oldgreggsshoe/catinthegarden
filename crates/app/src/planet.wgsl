@group(1) @binding(0)
var height_map: texture_2d<f32>;

@group(1) @binding(1)
var biome_map: texture_2d<u32>;

@group(1) @binding(2)
var moisture_map: texture_2d<f32>;

@group(3) @binding(0)
var cloud_field_current: texture_cube<f32>;

@group(3) @binding(1)
var cloud_field_previous: texture_cube<f32>;

@group(3) @binding(2)
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

@group(3) @binding(3)
var<uniform> weather: WeatherRenderUniform;

@group(3) @binding(4)
var weather_surface_current: texture_cube<f32>;
@group(3) @binding(5)
var weather_surface_previous: texture_cube<f32>;

struct VertexInput {
    @location(0) anchor_relative_position: vec3<f32>,
    @location(1) sphere_direction: vec3<f32>,
    @location(2) tile_uv: vec2<f32>,
    @location(3) skirt_depth_meters: f32,
    @location(4) anchor_view_position: vec3<f32>,
    @location(5) source_uv_scale: vec2<f32>,
    @location(6) source_uv_offset: vec2<f32>,
    @location(7) terrain_info: u32,
    @location(8) lod_transition: vec2<f32>,
    @location(9) edge_stitch: u32,
    @location(10) node_uv_origin_span: vec4<f32>,
    @location(11) node_anchor_direction_cube_length: vec4<f32>,
}

struct VertexOutput {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) camera_relative_view_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) aerial_in_scatter: vec3<f32>,
    @location(3) lod_transition: vec2<f32>,
    @location(4) surface_direction: vec3<f32>,
    // Skirts close residual LOD gaps but are not terrain-data facets. Keep
    // their deliberately near-vertical filler geometry out of the low-poly
    // normal treatment.
    @location(5) skirt_depth_meters: f32,
    @location(6) source_uv: vec2<f32>,
    // Outmap flag and the scaled baked height, packed: the fragment stage
    // needs the height for each octave's headroom and the inter-stage location
    // budget is full.
    @location(7) outmap_and_macro_height: vec2<f32>,
    @location(8) aerial_transmittance: vec3<f32>,
    // Pack the presentation-mist amount into the otherwise scalar detail
    // location so the final fragment composition can converge to the sky
    // after biome-specific aerial colour correction.
    @location(9) terrain_detail_meters_and_fog_amount: vec2<f32>,
    // The fog endpoint is evaluated per vertex like the existing aerial
    // approximation, then interpolated without consuming another inter-stage
    // location (the Quadro path is already at the 16-location limit).
    @location(10) surface_height_and_fog_color: vec4<f32>,
    // Detail is evaluated anchor-locally for precision, so the pixel needs the
    // same anchor the vertex used. Flat: it is constant across the node, and
    // interpolating it would defeat the exact-integer cell it provides. In
    // flat-triangle mode this otherwise-unused slot carries the instance-flat
    // source UV offset, keeping the inter-stage location count within wgpu's
    // limit.
    @location(11) @interpolate(flat) detail_anchor_direction: vec3<f32>,
    @location(12) detail_local_meters: vec3<f32>,
    // Mesh vertex spacing for this node. Flat because it is genuinely constant
    // across the node -- it depends only on the node's level. The rest of the
    // handover cutoff is recomputed per pixel from camera distance: passing that
    // flat instead made every triangle take its provoking vertex's distance, so
    // the band boundary stepped per triangle and shaded as hard facets.
    @location(13) @interpolate(flat) detail_vertex_spacing_meters: f32,
    @location(14) tile_uv: vec2<f32>,
    // Source scale remains flat for exact tile ownership. The z component
    // carries the provoking vertex latitude for flat-triangle palette fades;
    // w carries the terrain triangle's single specular value.
    @location(15) @interpolate(flat) source_uv_scale_and_latitude: vec4<f32>,
}

struct OceanVertexOutput {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) camera_relative_view_position: vec3<f32>,
    @location(1) lod_transition: vec2<f32>,
    @location(2) surface_direction: vec3<f32>,
    @location(3) source_uv: vec2<f32>,
    @location(4) @interpolate(flat) outmap: f32,
    // The shell must not win depth on a raised land triangle when its
    // independently sampled coastline falls on the opposite side of a
    // bilinear cell. This interpolated vertex height mirrors the geometry
    // actually displaced by the terrain pass.
    @location(5) terrain_height_hint: f32,
    @location(6) tile_uv: vec2<f32>,
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

/// Pyramid level of the tile this node is actually reading. Coarser than the
/// requested level whenever the node fell back to an ancestor, which is the
/// common case away from the sparse corridor.
fn source_level(terrain_info: u32) -> u32 {
    return (terrain_info >> 9u) & 0x1fu;
}

fn source_edge_fade_enabled(terrain_info: u32) -> bool {
    return (terrain_info & (1u << 14u)) != 0u;
}

fn near_field_texture() -> bool {
    return textureDimensions(height_map, 0).x == u32(NEAR_FIELD_WINDOW_LOGICAL_QUADS + 1.0);
}

fn source_coordinate(source_uv: vec2<f32>) -> vec2<f32> {
    if near_field_texture() {
        return clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0))
            * NEAR_FIELD_WINDOW_LOGICAL_QUADS;
    }
    return vec2<f32>(TILE_GUTTER)
        + clamp(source_uv, vec2<f32>(-1.0 / MATERIAL_TILE_LOGICAL_QUADS), vec2<f32>(1.0 + 1.0 / MATERIAL_TILE_LOGICAL_QUADS))
            * MATERIAL_TILE_LOGICAL_QUADS;
}

fn sample_height(source_uv: vec2<f32>) -> f32 {
    let coordinate = source_coordinate(source_uv);
    let lower = vec2<i32>(floor(coordinate));
    let last_coordinate = select(
        MATERIAL_TILE_LAST_STORED_COORD,
        i32(NEAR_FIELD_WINDOW_LOGICAL_QUADS),
        near_field_texture(),
    );
    let upper = min(
        lower + vec2<i32>(1),
        vec2<i32>(last_coordinate),
    );
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

fn macro_terrain_height(outmap: bool, source_uv: vec2<f32>, direction: vec3<f32>) -> f32 {
    if outmap {
        return sample_height(source_uv);
    }
    return placeholder_height(direction);
}

fn terrain_height(
    outmap: bool,
    source_uv: vec2<f32>,
    direction: vec3<f32>,
    camera_distance_meters: f32,
) -> f32 {
    let macro_height = macro_terrain_height(outmap, source_uv, direction);
    if !outmap {
        return macro_height;
    }
    // Macro only. Synthesised detail is added once in vs_main with an analytic
    // slope, rather than here, so the four normal probes stay pure texture reads
    // instead of each re-running the whole octave ladder.
    return scaled_terrain_macro_height(macro_height);
}

fn sample_biome(source_uv: vec2<f32>) -> u32 {
    let coordinate = vec2<i32>(round(source_coordinate(source_uv)));
    let last_coordinate = select(
        MATERIAL_TILE_LAST_STORED_COORD,
        i32(NEAR_FIELD_WINDOW_LOGICAL_QUADS),
        near_field_texture(),
    );
    let clamped = min(coordinate, vec2<i32>(last_coordinate));
    return textureLoad(biome_map, clamped, 0).x;
}

fn sample_biome_blend(source_uv: vec2<f32>) -> BiomeBlendSample {
    // Biomes remain categorical in the baked outmap, but display-space
    // materials should not expose their texel grid. Blend the four nearest
    // baked owners exactly as the height channel is blended; the gutter keeps
    // this continuous when the resident source changes at a tile edge.
    let coordinate = source_coordinate(source_uv);
    let last_coordinate = select(
        MATERIAL_TILE_LAST_STORED_COORD,
        i32(NEAR_FIELD_WINDOW_LOGICAL_QUADS),
        near_field_texture(),
    );
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(
        lower + vec2<i32>(1),
        vec2<i32>(last_coordinate),
    );
    let amount = fract(coordinate);
    return BiomeBlendSample(
        vec4<u32>(
            textureLoad(biome_map, lower, 0).x,
            textureLoad(biome_map, vec2<i32>(upper.x, lower.y), 0).x,
            textureLoad(biome_map, vec2<i32>(lower.x, upper.y), 0).x,
            textureLoad(biome_map, upper, 0).x,
        ),
        vec4<f32>(
            (1.0 - amount.x) * (1.0 - amount.y),
            amount.x * (1.0 - amount.y),
            (1.0 - amount.x) * amount.y,
            amount.x * amount.y,
        ),
    );
}

fn sample_moisture(source_uv: vec2<f32>) -> f32 {
    let coordinate = source_coordinate(source_uv);
    let last_coordinate = select(
        MATERIAL_TILE_LAST_STORED_COORD,
        i32(NEAR_FIELD_WINDOW_LOGICAL_QUADS),
        near_field_texture(),
    );
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(
        lower + vec2<i32>(1),
        vec2<i32>(last_coordinate),
    );
    let amount = fract(coordinate);
    let lower_left = textureLoad(moisture_map, lower, 0).x;
    let lower_right = textureLoad(moisture_map, vec2<i32>(upper.x, lower.y), 0).x;
    let upper_left = textureLoad(moisture_map, vec2<i32>(lower.x, upper.y), 0).x;
    let upper_right = textureLoad(moisture_map, upper, 0).x;
    return mix(
        mix(lower_left, lower_right, amount.x),
        mix(upper_left, upper_right, amount.x),
        amount.y,
    );
}

fn displaced_surface_normal(
    direction: vec3<f32>,
    source_uv: vec2<f32>,
    source_uv_scale: vec2<f32>,
    terrain_info: u32,
    camera_distance_meters: f32,
) -> vec3<f32> {
    let face = cube_face(terrain_info);
    let tangent_u = face_tangent_u(face);
    let tangent_v = face_tangent_v(face);
    let cube_position = direction / max(face_component(direction, face), 1.0e-6);
    let requested_cube_step = 2.0
        / (MATERIAL_TILE_LOGICAL_QUADS * exp2(f32(requested_level(terrain_info))));
    // Filter normals continuously by camera distance, never by node level.
    // Shared positions therefore retain the same lighting across mixed LODs,
    // while nearby baked relief is no longer blurred through a fixed 256m
    // footprint.
    //
    // Never probe finer than one source texel. sample_height is manually
    // bilinear, so the height field is piecewise bilinear and its gradient is
    // piecewise constant within a texel: differencing below texel width returns
    // that constant and the surface shades as flat texel-sized facets. This is
    // a floor, not a filter width -- it only binds where the LOD is coarser than
    // the probe, which is most of the ground beyond a few hundred metres.
    let source_texel_meters = requested_cube_step * PLANET_RADIUS_METERS;
    let normal_sample_meters = clamp(
        camera_distance_meters * 0.01,
        max(TERRAIN_NORMAL_MIN_SAMPLE_METERS, source_texel_meters),
        TERRAIN_NORMAL_MAX_SAMPLE_METERS,
    );
    let cube_step = normal_sample_meters / PLANET_RADIUS_METERS;
    let normal_step_scale = cube_step / requested_cube_step;
    let left_direction = normalize(cube_position - tangent_u * cube_step);
    let right_direction = normalize(cube_position + tangent_u * cube_step);
    let down_direction = normalize(cube_position - tangent_v * cube_step);
    let up_direction = normalize(cube_position + tangent_v * cube_step);
    let uv_step = source_uv_scale / MATERIAL_TILE_LOGICAL_QUADS * normal_step_scale;
    let outmap = uses_outmap(terrain_info);
    let left_height = terrain_height(
        outmap,
        source_uv - vec2<f32>(uv_step.x, 0.0),
        left_direction,
        camera_distance_meters,
    );
    let right_height = terrain_height(
        outmap,
        source_uv + vec2<f32>(uv_step.x, 0.0),
        right_direction,
        camera_distance_meters,
    );
    let down_height = terrain_height(
        outmap,
        source_uv - vec2<f32>(0.0, uv_step.y),
        down_direction,
        camera_distance_meters,
    );
    let up_height = terrain_height(
        outmap,
        source_uv + vec2<f32>(0.0, uv_step.y),
        up_direction,
        camera_distance_meters,
    );
    let tangent_delta_u = (right_direction - left_direction) * PLANET_RADIUS_METERS
        + right_direction * right_height
        - left_direction * left_height;
    let tangent_delta_v = (up_direction - down_direction) * PLANET_RADIUS_METERS
        + up_direction * up_height
        - down_direction * down_height;
    return normalize(cross(tangent_delta_u, tangent_delta_v));
}

fn ocean_with_aerial_perspective(
    direction: vec3<f32>,
    camera_relative_view_position: vec3<f32>,
    sun_direction: vec3<f32>,
) -> vec3<f32> {
    let surface = ocean_surface(direction, camera.projection.z);
    let sun_transmittance = surface_direct_sun_transmittance(
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        surface.normal,
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let water_color = ocean_lighting(
        surface.normal,
        camera_relative_view_position,
        sun_transmittance,
        sky_diffuse,
    );
    return ocean_aerial_perspective(
        water_color,
        camera_relative_view_position,
        direction,
        surface.vertical_displacement,
    );
}

fn edge_stitch_level_delta(edge_stitch: u32, edge: u32) -> u32 {
    return (edge_stitch >> (edge * 5u)) & 0x1fu;
}

fn snap_edge_coordinate(coordinate: f32, level_delta: u32) -> f32 {
    if level_delta == 0u {
        return coordinate;
    }
    let grid_coordinate = u32(round(coordinate * 32.0));
    // Never collapse more than four fine edge quads into one segment. Large
    // LOD gaps use skirts; collapsing all 32 quads produces a giant fan that
    // is far more conspicuous than the residual T-junction it tries to hide.
    let stride = 1u << min(level_delta, 2u);
    return f32((grid_coordinate / stride) * stride) / 32.0;
}

fn stitched_tile_uv(tile_uv: vec2<f32>, edge_stitch: u32) -> vec2<f32> {
    var stitched = tile_uv;
    if tile_uv.y <= 1.0e-5 {
        stitched.x = snap_edge_coordinate(
            stitched.x,
            edge_stitch_level_delta(edge_stitch, 0u),
        );
    }
    if tile_uv.x >= 1.0 - 1.0e-5 {
        stitched.y = snap_edge_coordinate(
            stitched.y,
            edge_stitch_level_delta(edge_stitch, 1u),
        );
    }
    if tile_uv.y >= 1.0 - 1.0e-5 {
        stitched.x = snap_edge_coordinate(
            stitched.x,
            edge_stitch_level_delta(edge_stitch, 2u),
        );
    }
    if tile_uv.x <= 1.0e-5 {
        stitched.y = snap_edge_coordinate(
            stitched.y,
            edge_stitch_level_delta(edge_stitch, 3u),
        );
    }
    return stitched;
}

fn edge_detail_filter_meters(
    tile_uv: vec2<f32>,
    edge_stitch: u32,
    requested_level: u32,
) -> f32 {
    let node_spacing = terrain_vertex_spacing_meters(requested_level);
    var filter_meters = node_spacing;
    let edge_distances = vec4<f32>(
        tile_uv.y,
        1.0 - tile_uv.x,
        1.0 - tile_uv.y,
        tile_uv.x,
    );
    for (var edge = 0u; edge < 4u; edge += 1u) {
        let level_delta = edge_stitch_level_delta(edge_stitch, edge);
        if level_delta == 0u {
            continue;
        }
        let neighbor_spacing = terrain_vertex_spacing_meters(
            requested_level - min(requested_level, level_delta),
        );
        // Carry the coarser filter far enough into the fine patch to retire
        // frequencies that the neighbouring grid cannot represent, then hand
        // them back continuously. At a shared edge both chunks now evaluate
        // the same displacement; large budget-induced gaps blend across the
        // whole fine node instead of forming a narrow near-vertical wall.
        let fade_width = min(
            exp2(f32(level_delta)) / TERRAIN_CHUNK_QUADS,
            1.0,
        );
        let edge_weight = 1.0 - smoothstep(
            0.0,
            fade_width,
            edge_distances[edge],
        );
        filter_meters = max(
            filter_meters,
            mix(node_spacing, neighbor_spacing, edge_weight),
        );
    }
    return filter_meters;
}

fn lod_morphed_tile_uv(tile_uv: vec2<f32>, lod_transition: vec2<f32>) -> vec2<f32> {
    if lod_transition.y <= 0.5 || lod_transition.x >= 1.0 {
        return tile_uv;
    }
    // A child covers half of its parent, so the parent's vertices inside the
    // child footprint lie on a 16x16 grid. Grow the odd child vertices out of
    // that grid while the complementary parent fades away.
    let parent_grid_uv = round(tile_uv * 16.0) / 16.0;
    return mix(parent_grid_uv, tile_uv, lod_transition.x);
}

fn flat_triangle_vertex_specular(
    normal: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_height: f32,
    camera_relative_view_position: vec3<f32>,
) -> f32 {
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_transmittance = surface_direct_sun_transmittance(
        surface_direction,
        surface_height,
        sun_direction,
    );
    let sun_visibility = dot(sun_transmittance, vec3<f32>(0.2126, 0.7152, 0.0722));
    let view_direction = normalize(view_to_planet(-camera_relative_view_position));
    let half_vector = normalize(sun_direction + view_direction);
    return pow(max(dot(normal, half_vector), 0.0), 64.0)
        * sun_visibility
        * (0.08 * SURFACE_SUNLIGHT_SCALE);
}

fn stitched_surface_direction(
    original_direction: vec3<f32>,
    tile_uv: vec2<f32>,
    stitched_uv: vec2<f32>,
    terrain_info: u32,
) -> vec3<f32> {
    let uv_delta = stitched_uv - tile_uv;
    if all(abs(uv_delta) <= vec2<f32>(1.0e-7)) {
        return original_direction;
    }
    let face = cube_face(terrain_info);
    let cube_position = original_direction
        / max(face_component(original_direction, face), 1.0e-6);
    let node_span = 2.0 / exp2(f32(requested_level(terrain_info)));
    return normalize(
        cube_position
            + face_tangent_u(face) * uv_delta.x * node_span
            + face_tangent_v(face) * uv_delta.y * node_span,
    );
}

struct PatchVertex {
    direction: vec3<f32>,
    anchor_relative_position: vec3<f32>,
    tile_uv: vec2<f32>,
}

fn project_patch_vertex(input: VertexInput) -> PatchVertex {
    let face = cube_face(input.terrain_info);
    let morphed_tile_uv = lod_morphed_tile_uv(input.tile_uv, input.lod_transition);
    let tile_uv = stitched_tile_uv(morphed_tile_uv, input.edge_stitch);
    let anchor_direction = input.node_anchor_direction_cube_length.xyz;
    let anchor_cube = anchor_direction * input.node_anchor_direction_cube_length.w;
    let cube_offset = face_tangent_u(face)
            * (tile_uv.x - 0.5)
            * input.node_uv_origin_span.z
        + face_tangent_v(face)
            * (tile_uv.y - 0.5)
            * input.node_uv_origin_span.w;
    let surface_cube = anchor_cube + cube_offset;
    let parallel = dot(surface_cube, anchor_direction);
    let tangent = surface_cube - anchor_direction * parallel;
    let tangent_length_squared = dot(tangent, tangent);
    let surface_cube_length = sqrt(
        parallel * parallel + tangent_length_squared,
    );
    let radial_scale = -tangent_length_squared / max(
        surface_cube_length * (parallel + surface_cube_length),
        1.0e-8,
    );
    var direction = normalize(surface_cube);
    // Evaluate the tiny direction difference in an anchor-local form. Direct
    // subtraction of two absolute f32 directions loses most of an L18
    // triangle to cancellation near cube-face UV +/-1.
    var anchor_relative_position = (
        tangent / surface_cube_length + anchor_direction * radial_scale
    ) * PLANET_RADIUS_METERS;
    if tile_uv.x <= 1.0e-5 || tile_uv.x >= 1.0 - 1.0e-5
        || tile_uv.y <= 1.0e-5 || tile_uv.y >= 1.0 - 1.0e-5 {
        // Evaluate shared boundaries from their global dyadic face UV. Both
        // neighbours then produce identical edge positions; the stable
        // anchor-local path above retains sub-metre precision in the interior.
        let node_uv = input.node_uv_origin_span.xy
            + tile_uv * input.node_uv_origin_span.zw;
        direction = normalize(
            face_normal(face)
                + face_tangent_u(face) * node_uv.x
                + face_tangent_v(face) * node_uv.y,
        );
        anchor_relative_position =
            (direction - anchor_direction) * PLANET_RADIUS_METERS;
    }
    return PatchVertex(direction, anchor_relative_position, tile_uv);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let projected = project_patch_vertex(input);
    let direction = projected.direction;
    let anchor_relative_position = projected.anchor_relative_position;
    let tile_uv = projected.tile_uv;
    let anchor_direction = input.node_anchor_direction_cube_length.xyz;
    let source_uv = input.source_uv_offset + tile_uv * input.source_uv_scale;
    let outmap = uses_outmap(input.terrain_info);
    let flat_triangles = u32(camera.projection.w + 0.5) == RENDER_DEBUG_FLAT_TRIANGLES;
    let macro_height = macro_terrain_height(outmap, source_uv, direction);
    let base_camera_relative_view_position = input.anchor_view_position
        + planet_to_view(anchor_relative_position);
    let camera_distance_meters = length(base_camera_relative_view_position);
    let base_height = select(
        macro_height,
        scaled_terrain_macro_height(macro_height),
        outmap,
    );
    // Anchor-local metres, not an absolute direction: this is what carries the
    // in-cell fraction for metre-scale octaves without losing it to f32.
    // The mesh cannot represent relief finer than its own vertex spacing, so it
    // stops there and the fragment shader picks up the rest as a normal detail.
    let vertex_spacing_meters = terrain_vertex_spacing_meters(
        requested_level(input.terrain_info),
    );
    let vertex_filter_meters = max(
        terrain_detail_filter_meters(camera_distance_meters),
        edge_detail_filter_meters(
            tile_uv,
            input.edge_stitch,
            requested_level(input.terrain_info),
        ),
    );
    let detail = terrain_detail(
        anchor_direction,
        anchor_relative_position,
        vertex_filter_meters,
        continuous_baked_sample_spacing_meters(
            input.node_uv_origin_span.xy + tile_uv * input.node_uv_origin_span.zw,
            source_level(input.terrain_info),
            source_edge_fade_enabled(input.terrain_info),
        ),
        // Each octave asks this separately for its own headroom, so the
        // ladder no longer needs a single scalar weight on the outside.
        select(0.0, base_height, outmap),
    );
    let terrain_detail_meters = detail.height_meters;
    let height = base_height + terrain_detail_meters;
    // Polar ice overrides ocean in the baked biome contract. Lift it just
    // above sea level so the cap remains visible rather than becoming water.
    let biome_id = sample_biome(source_uv);
    let ice = outmap && biome_id == 2u;
    let lake = outmap && biome_id == 1u;
    let land_height = select(height, max(height, 5.0), ice);
    // Baked biome ownership can straddle a coarse source cell at a shoreline
    // (and ancestor fallback makes that cell cover a wider world-space area).
    // Only a non-positive sample is actually water geometry.  Flattening a
    // positive sample solely because its categorical owner says water creates
    // kilometre-scale vertical walls through otherwise continuous land.
    let water_owned = (biome_id == 0u || biome_id == 1u) && macro_height <= 0.0;
    let surface_height = select(land_height, 0.0, flat_triangles && water_owned);
    let skirt_depth_meters = select(
        0.0,
        min(
            input.node_uv_origin_span.z
                * PLANET_RADIUS_METERS
                * TERRAIN_SKIRT_DEPTH_RATIO,
            MAX_TERRAIN_SKIRT_DEPTH_METERS,
        ),
        input.skirt_depth_meters > 0.0,
    );
    let local_planet_position = anchor_relative_position
        + direction * (surface_height - skirt_depth_meters);
    let camera_relative_view_position = input.anchor_view_position
        + planet_to_view(local_planet_position);
    var normal = displaced_surface_normal(
        direction,
        source_uv,
        input.source_uv_scale,
        input.terrain_info,
        camera_distance_meters,
    );
    if outmap {
        // The slope already carries each octave's own headroom, so there is no
        // scalar weight left to apply here.
        normal = terrain_detail_perturbed_normal(normal, direction, detail.slope);
    }
    var flat_specular = 0.0;
    if flat_triangles {
        flat_specular = flat_triangle_vertex_specular(
            normal,
            direction,
            surface_height,
            camera_relative_view_position,
        );
    }
    // Flat-triangle mode keeps the categorical material and face lighting, but
    // still needs the ordinary aerial path so distant facets fade toward the
    // same atmosphere as the rest of the terrain.
    var aerial = AerialPerspectiveComponents(
        vec3<f32>(1.0),
        vec3<f32>(0.0),
    );
    // Evaluate the same continuous aerial model for flat facets at every
    // distance. A hard near/far cutoff creates a visible ring when a triangle
    // crosses the threshold, while the bounded atmospheric column already
    // fades naturally toward the camera.
    aerial = aerial_perspective_components(
        camera_relative_view_position,
        direction,
        surface_height,
    );
    if flat_triangles {
        // Keep flat-mode extinction continuous, but fade the warm forward
        // aerial in-scatter in over distance. Without this bounded blend the
        // diagnostic terrain can turn orange while the daytime sky remains
        // blue; a hard cutoff would recreate the old visible ring.
        let flat_aerial_weight = smoothstep(20000.0, 180000.0, camera_distance_meters);
        aerial = AerialPerspectiveComponents(
            aerial.transmittance,
            aerial.in_scatter * flat_aerial_weight,
        );
    }
    // Keep the extra presentation mist separate from physical aerial
    // in-scatter. It must be composed after biome-specific aerial correction;
    // folding the sky endpoint into `aerial.in_scatter` here lets vegetation's
    // 0.42 in-scatter scale darken even fully saturated fog.
    let fog = terrain_fog(
        camera_relative_view_position,
        direction,
        surface_height,
    );
    let detail_anchor_or_flat_source_offset = select(
        anchor_direction,
        vec3<f32>(input.source_uv_offset, 0.0),
        flat_triangles,
    );
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(camera_relative_view_position, 1.0),
        camera_relative_view_position,
        normal,
        aerial.in_scatter,
        input.lod_transition,
        direction,
        input.skirt_depth_meters,
        source_uv,
        vec2<f32>(select(0.0, 1.0, outmap), select(0.0, base_height, outmap)),
        aerial.transmittance,
        vec2<f32>(terrain_detail_meters, fog.amount),
        vec4<f32>(surface_height, fog.color),
        detail_anchor_or_flat_source_offset,
        anchor_relative_position,
        select(0.0, vertex_spacing_meters, outmap),
        tile_uv,
        vec4<f32>(input.source_uv_scale, direction.y, flat_specular),
    );
}

@vertex
fn vs_ocean(input: VertexInput) -> OceanVertexOutput {
    let projected = project_patch_vertex(input);
    let source_uv = input.source_uv_offset + projected.tile_uv * input.source_uv_scale;
    let outmap = uses_outmap(input.terrain_info);
    let macro_height_meters = macro_terrain_height(outmap, source_uv, projected.direction);
    let terrain_height_hint = select(
        0.0,
        scaled_terrain_macro_height(macro_height_meters),
        outmap,
    );
    var surface = ocean_surface(projected.direction, camera.projection.z);
    if u32(camera.projection.w + 0.5) == RENDER_DEBUG_FLAT_TRIANGLES {
        surface = flat_ocean_surface(projected.direction);
    }
    let local_planet_position = projected.anchor_relative_position
        + projected.direction * surface.vertical_displacement
        + surface.horizontal_displacement;
    let camera_relative_view_position = input.anchor_view_position
        + planet_to_view(local_planet_position);
    return OceanVertexOutput(
        camera.projection_matrix * vec4<f32>(camera_relative_view_position, 1.0),
        camera_relative_view_position,
        input.lod_transition,
        projected.direction,
        source_uv,
        select(0.0, 1.0, outmap),
        terrain_height_hint,
        projected.tile_uv,
    );
}

fn lod_dither_threshold(fragment_position: vec4<f32>) -> f32 {
    // Stable interleaved-gradient noise avoids the visible checker/grid of an
    // ordered matrix. Parent and child still evaluate the exact same threshold
    // at a screen pixel, so their coverage remains complementary.
    let pixel = floor(fragment_position.xy);
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

const FLAT_TRIANGLE_GRID_QUADS: f32 = 32.0;

fn flat_triangle_cell(input_tile_uv: vec2<f32>) -> vec2<f32> {
    let local = fract(input_tile_uv * FLAT_TRIANGLE_GRID_QUADS);
    let cell = floor(input_tile_uv * FLAT_TRIANGLE_GRID_QUADS);
    let upper = local.x + local.y > 1.0;
    let centre = select(vec2<f32>(1.0 / 3.0), vec2<f32>(2.0 / 3.0), upper);
    return (cell + centre) / FLAT_TRIANGLE_GRID_QUADS;
}

fn flat_triangle_edge(input_tile_uv: vec2<f32>, skirt: f32) -> f32 {
    if camera.flat_triangle_options.x < 0.5 {
        return 0.0;
    }
    if skirt > 0.0 {
        return 0.0;
    }
    let local = fract(input_tile_uv * FLAT_TRIANGLE_GRID_QUADS);
    let upper = local.x + local.y > 1.0;
    let barycentric = select(
        vec3<f32>(1.0 - local.x - local.y, local.x, local.y),
        vec3<f32>(1.0 - local.y, local.x + local.y - 1.0, 1.0 - local.x),
        upper,
    );
    let width = max(fwidth(barycentric), vec3<f32>(1.0e-4));
    return 1.0 - min(
        min(
            smoothstep(0.0, width.x * 1.5, barycentric.x),
            smoothstep(0.0, width.y * 1.5, barycentric.y),
        ),
        smoothstep(0.0, width.z * 1.5, barycentric.z),
    );
}

fn flat_triangle_normal(
    camera_relative_view_position: vec3<f32>,
    fallback_normal: vec3<f32>,
) -> vec3<f32> {
    let derivatives = cross(
        dpdx(camera_relative_view_position),
        dpdy(camera_relative_view_position),
    );
    if dot(derivatives, derivatives) < 1.0e-8 {
        return normalize(fallback_normal);
    }
    var view_normal = normalize(derivatives);
    let to_camera = normalize(-camera_relative_view_position);
    if dot(view_normal, to_camera) < 0.0 {
        view_normal = -view_normal;
    }
    return normalize(view_to_planet(view_normal));
}

fn flat_triangle_outward_normal(normal: vec3<f32>, surface_direction: vec3<f32>) -> vec3<f32> {
    let outward = normalize(surface_direction);
    return select(normal, -normal, dot(normal, outward) < 0.0);
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
    let shadow_direction = normalize(shadow_position);
    return cloudDensityWithOctaves(shadow_direction, shell_index, 3u);
}

fn weather_surface_sample(direction: vec3<f32>) -> vec4<f32> {
    let current = textureSampleLevel(
        weather_surface_current,
        cloud_field_sampler,
        normalize(direction),
        0.0,
    );
    let previous = textureSampleLevel(
        weather_surface_previous,
        cloud_field_sampler,
        normalize(direction),
        0.0,
    );
    return mix(previous, current, weather.blend);
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
    // Match the cloud presentation's deliberately small set of hard density
    // bands instead of putting a soft photographic shadow under it.
    let posterized_density = floor(combined_density * 4.0 + 0.5) / 4.0;
    return 1.0 - posterized_density * 0.88;
}

fn flat_triangle_lighting(
    albedo: vec3<f32>,
    normal: vec3<f32>,
    surface_direction: vec3<f32>,
    surface_height: f32,
    camera_relative_view_position: vec3<f32>,
    triangle_specular: f32,
    use_triangle_specular: bool,
    receive_cloud_shadow: bool,
) -> vec3<f32> {
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
    var cloud_visibility = 1.0;
    if receive_cloud_shadow
        && dot(sun_transmittance, vec3<f32>(0.2126, 0.7152, 0.0722)) > 0.001
    {
        cloud_visibility = cloud_shadow_visibility(
            surface_direction,
            surface_height,
            sun_direction,
        );
    }
    let diffuse = sky_diffuse
        + sun_transmittance * cloud_visibility
            * max(dot(normal, sun_direction), 0.0)
            * SURFACE_SUNLIGHT_SCALE;
    var specular = triangle_specular;
    if !use_triangle_specular {
        let view_direction = normalize(view_to_planet(-camera_relative_view_position));
        let half_vector = normalize(sun_direction + view_direction);
        specular = pow(max(dot(normal, half_vector), 0.0), 64.0)
            * dot(sun_transmittance, vec3<f32>(0.2126, 0.7152, 0.0722))
            * (0.08 * SURFACE_SUNLIGHT_SCALE);
    }
    return albedo * diffuse + vec3<f32>(specular * cloud_visibility);
}

fn flat_triangle_land_biome(primary: u32, first: u32, second: u32, third: u32) -> u32 {
    var selected = primary;
    if selected == 0u || selected == 1u {
        if first != 0u && first != 1u {
            selected = first;
        } else if second != 0u && second != 1u {
            selected = second;
        } else if third != 0u && third != 1u {
            selected = third;
        }
    }
    return selected;
}

// Individual tree geometry is deliberately camera-local. Once those trees
// are sub-pixel, retain the baked forest ownership with a seam-safe canopy
// density made only from planet direction. This never changes biome ownership,
// terrain height, or water; it darkens the far forest albedo in proportion to
// the same broad density field used by local evergreen placement.
fn far_forest_canopy_albedo(
    base_albedo: vec3<f32>,
    outmap: bool,
    biome_id: u32,
    moisture: f32,
    macro_height_meters: f32,
    surface_normal: vec3<f32>,
    surface_direction: vec3<f32>,
    camera_distance_meters: f32,
    snow_cover: f32,
) -> vec3<f32> {
    let forest_owned = biome_id == 2u
        || biome_id == 3u
        || biome_id == 4u
        || biome_id == 6u
        || biome_id == 9u;
    if !outmap || !forest_owned {
        return base_albedo;
    }

    // The close material stack and local billboards own the first 32km. Fade
    // this macro treatment in only after the repeating ground detail is gone.
    let distance_weight = smoothstep(32000.0, 160000.0, camera_distance_meters);
    if distance_weight <= 0.0 {
        return base_albedo;
    }

    let direction = normalize(surface_direction);
    let slope = 1.0 - clamp(dot(normalize(surface_normal), direction), 0.0, 1.0);
    let slope_weight = 1.0 - smoothstep(0.08, 0.28, slope);
    let moisture_weight = smoothstep(0.38, 0.82, moisture);
    let land_weight = select(0.0, 1.0, macro_height_meters > 0.0);
    // Snow does not suppress this species: these are evergreen trees, and the
    // cold biomes intentionally remain eligible where moisture and slope pass.
    let snow_weight = mix(1.0, 0.76, clamp(snow_cover, 0.0, 1.0));
    let canopy_weight = distance_weight
        * moisture_weight
        * land_weight
        * slope_weight
        * snow_weight;
    if canopy_weight <= 0.0 {
        return base_albedo;
    }

    // A modest cell frequency gives orbit-scale clusters rather than a
    // per-pixel sparkle. Sampling normalized 3D direction keeps the field
    // continuous across cube faces, source tiles, and LOD boundaries.
    let noise_position = direction * 192.0;
    let canopy_noise = terrain_detail_value_noise(
        vec3<i32>(floor(noise_position)),
        fract(noise_position),
    ).value * 0.5 + 0.5;
    let canopy_cluster = smoothstep(0.24, 0.76, canopy_noise);
    let tree_density = mix(0.35, 1.0, canopy_cluster);
    let canopy_tint = mix(
        vec3<f32>(0.58, 0.68, 0.60),
        vec3<f32>(0.78, 0.86, 0.72),
        canopy_cluster,
    );
    return mix(base_albedo, base_albedo * canopy_tint, canopy_weight * tree_density * 0.62);
}

fn apply_terrain_distance_fog(
    aerial_color: vec3<f32>,
    input: VertexOutput,
) -> vec3<f32> {
    return mix(
        aerial_color,
        input.surface_height_and_fog_color.yzw,
        input.terrain_detail_meters_and_fog_amount.y,
    );
}

fn flat_triangle_colour(
    input: VertexOutput,
) -> vec4<f32> {
    let centre_tile_uv = flat_triangle_cell(input.tile_uv);
    let source_uv_scale = input.source_uv_scale_and_latitude.xy;
    let source_uv_offset = input.detail_anchor_direction.xy;
    let centre_source_uv = source_uv_offset + centre_tile_uv * source_uv_scale;
    let cell = floor(input.tile_uv * FLAT_TRIANGLE_GRID_QUADS);
    let local = fract(input.tile_uv * FLAT_TRIANGLE_GRID_QUADS);
    let upper = local.x + local.y > 1.0;
    var first_tile_uv = cell / FLAT_TRIANGLE_GRID_QUADS;
    var second_tile_uv = (cell + vec2<f32>(1.0, 0.0)) / FLAT_TRIANGLE_GRID_QUADS;
    var third_tile_uv = (cell + vec2<f32>(0.0, 1.0)) / FLAT_TRIANGLE_GRID_QUADS;
    if upper {
        first_tile_uv = (cell + vec2<f32>(1.0, 0.0)) / FLAT_TRIANGLE_GRID_QUADS;
        second_tile_uv = (cell + vec2<f32>(1.0, 1.0)) / FLAT_TRIANGLE_GRID_QUADS;
        third_tile_uv = (cell + vec2<f32>(0.0, 1.0)) / FLAT_TRIANGLE_GRID_QUADS;
    }
    let first_source_uv = source_uv_offset + first_tile_uv * source_uv_scale;
    let second_source_uv = source_uv_offset + second_tile_uv * source_uv_scale;
    let third_source_uv = source_uv_offset + third_tile_uv * source_uv_scale;
    let first_biome = sample_biome(first_source_uv);
    let second_biome = sample_biome(second_source_uv);
    let third_biome = sample_biome(third_source_uv);
    let biome_id = flat_triangle_land_biome(
        sample_biome(centre_source_uv),
        first_biome,
        second_biome,
        third_biome,
    );
    let first_height = macro_terrain_height(
        input.outmap_and_macro_height.x > 0.5,
        first_source_uv,
        normalize(input.surface_direction),
    );
    let second_height = macro_terrain_height(
        input.outmap_and_macro_height.x > 0.5,
        second_source_uv,
        normalize(input.surface_direction),
    );
    let third_height = macro_terrain_height(
        input.outmap_and_macro_height.x > 0.5,
        third_source_uv,
        normalize(input.surface_direction),
    );
    let mixed_land_triangle = max(first_height, max(second_height, third_height)) > 0.0;
    let fill_biome = select(biome_id, 5u, mixed_land_triangle && (biome_id == 0u || biome_id == 1u));
    var fill = select(debug_ocean_albedo(), biome_color(fill_biome), fill_biome != 1u);
    if fill_biome == 2u {
        // Keep one final colour per triangle, but avoid making the ice prior
        // read as a mathematically perfect latitude circle. Low-latitude ice
        // from mountain height remains fully icy through the second term.
        let polar_ice = smoothstep(
            0.58,
            0.70,
            abs(input.source_uv_scale_and_latitude.z),
        );
        let high_ice = smoothstep(2200.0, 4200.0, input.outmap_and_macro_height.y);
        fill = mix(biome_color(3u), fill, max(polar_ice, high_ice));
    }
    let normal = flat_triangle_outward_normal(
        flat_triangle_normal(input.camera_relative_view_position, input.world_normal),
        input.surface_direction,
    );
    if fill_biome != 0u && fill_biome != 1u {
        let surface_field = weather_surface_sample(normalize(input.surface_direction));
        let wetness = smoothstep(0.18, 0.82, surface_field.r);
        let snow_cover = smoothstep(0.08, 0.70, surface_field.g);
        fill *= 1.0 - 0.22 * wetness;
        fill = mix(
            fill,
            mix(vec3<f32>(0.70, 0.73, 0.76), vec3<f32>(0.94, 0.96, 1.0), snow_cover),
            snow_cover,
        );
        fill = far_forest_canopy_albedo(
            fill,
            input.outmap_and_macro_height.x > 0.5,
            fill_biome,
            sample_moisture(centre_source_uv),
            input.outmap_and_macro_height.y,
            normal,
            normalize(input.surface_direction),
            length(input.camera_relative_view_position),
            snow_cover,
        );
    }
    let lit = flat_triangle_lighting(
        fill,
        normal,
        normalize(input.surface_direction),
        input.surface_height_and_fog_color.x,
        input.camera_relative_view_position,
        input.source_uv_scale_and_latitude.w,
        true,
        true,
    );
    // Keep flat fills categorical, but apply the same affine atmospheric
    // composition as smooth terrain. This lifts distant shadowed facets
    // toward the sky instead of leaving them at raw face-lighting black.
    var aerial_lit = lit
        * terrain_material_transmittance(input.aerial_transmittance, fill_biome)
        + terrain_material_in_scatter(input.aerial_in_scatter, fill_biome);
    let edge = flat_triangle_edge(input.tile_uv, input.skirt_depth_meters);
    if fill_biome == 0u || fill_biome == 1u {
        // Flat mode's terrain path still owns coarse water triangles because
        // the separate analytic ocean shell is disabled there. Keep those
        // triangles on the ocean-specific aerial blend; the generic terrain
        // in-scatter is strong and green enough at altitude to turn blue water
        // olive as the camera moves away or changes view angle.
        aerial_lit = ocean_aerial_perspective(
            lit,
            input.camera_relative_view_position,
            normalize(input.surface_direction),
            0.0,
        );
        // Water replaces the vertex-composed physical terrain atmosphere;
        // shared presentation mist is applied to both materials below.
    }
    // Outlines are unlit geometry edges, not emissive ink. Darken the
    // physically composed triangle so night-side lines fade with their
    // surroundings instead of imposing a blue-grey luminance floor. Apply
    // distance mist afterward so a fully obscured triangle cannot survive as
    // a black wireframe against the matching sky endpoint.
    let outlined_aerial_lit = mix(aerial_lit, aerial_lit * 0.68, edge);
    let misted_aerial_lit = apply_terrain_distance_fog(outlined_aerial_lit, input);
    return vec4<f32>(misted_aerial_lit, 1.0);
}

fn flat_ocean_colour(input: OceanVertexOutput) -> vec4<f32> {
    let direction = normalize(input.surface_direction);
    let surface = flat_ocean_surface(direction);
    let normal = flat_triangle_outward_normal(
        flat_triangle_normal(
            input.camera_relative_view_position,
            surface.normal,
        ),
        direction,
    );
    let lit = flat_triangle_lighting(
        debug_ocean_albedo(),
        normal,
        direction,
        surface.vertical_displacement,
        input.camera_relative_view_position,
        0.0,
        false,
        false,
    );
    let misted_ocean_lit = terrain_distance_fog(
        lit,
        input.camera_relative_view_position,
        direction,
        surface.vertical_displacement,
    );
    let edge = flat_triangle_edge(input.tile_uv, 0.0);
    return vec4<f32>(mix(misted_ocean_lit, misted_ocean_lit * 0.68, edge), 1.0);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Flat mode is a diagnostic presentation with one fixed topology. It has
    // no parent/child coverage to cross-fade; applying the transition dither
    // here exposes the other depth-writing facets as salt-and-pepper seams.
    if u32(camera.projection.w + 0.5) == RENDER_DEBUG_FLAT_TRIANGLES {
        return terrain_fragment_color(input);
    }
    let transition_progress = input.lod_transition.x;
    let incoming = input.lod_transition.y > 0.5;
    let threshold = lod_dither_threshold(input.position);
    if ((incoming && threshold >= transition_progress)
        || (!incoming && threshold < transition_progress)) {
        discard;
    }
    return terrain_fragment_color(input);
}

@fragment
fn fs_main_stable(input: VertexOutput) -> @location(0) vec4<f32> {
    return terrain_fragment_color(input);
}

@fragment
fn fs_ocean(input: OceanVertexOutput) -> @location(0) vec4<f32> {
    if u32(camera.projection.w + 0.5) == RENDER_DEBUG_FLAT_TRIANGLES {
        return ocean_fragment_color(input);
    }
    let transition_progress = input.lod_transition.x;
    let incoming = input.lod_transition.y > 0.5;
    let threshold = lod_dither_threshold(input.position);
    if ((incoming && threshold >= transition_progress)
        || (!incoming && threshold < transition_progress)) {
        discard;
    }
    return ocean_fragment_color(input);
}

@fragment
fn fs_ocean_stable(input: OceanVertexOutput) -> @location(0) vec4<f32> {
    return ocean_fragment_color(input);
}

fn ocean_fragment_color(input: OceanVertexOutput) -> vec4<f32> {
    let direction = normalize(input.surface_direction);
    let outmap = input.outmap > 0.5;
    let macro_height_meters = macro_terrain_height(outmap, input.source_uv, direction);
    let biome_id = sample_biome(input.source_uv);
    // This draw is a geometric sea shell, not another material arm on the
    // terrain mesh. Sample ownership per fragment so a coastline triangle
    // cannot lift water between a sea-level and a raised land vertex.
    if !is_open_ocean_surface(outmap, macro_height_meters, biome_id) {
        discard;
    }
    if input.terrain_height_hint > 0.0 {
        discard;
    }

    let render_debug_mode = u32(camera.projection.w + 0.5);
    if render_debug_mode == RENDER_DEBUG_FLAT_TRIANGLES {
        return flat_ocean_colour(input);
    }

    if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
        return vec4<f32>(debug_ocean_albedo(), 1.0);
    }
    let surface = ocean_surface(direction, camera.projection.z);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_transmittance = surface_direct_sun_transmittance(
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        surface.normal,
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let water_surface_color = ocean_lighting(
        surface.normal,
        input.camera_relative_view_position,
        sun_transmittance,
        sky_diffuse,
    );
    if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
        return vec4<f32>(water_surface_color, 1.0);
    }
    let water_aerial_color = ocean_aerial_perspective(
        water_surface_color,
        input.camera_relative_view_position,
        direction,
        surface.vertical_displacement,
    );
    if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
        return vec4<f32>(
            max(water_aerial_color - water_surface_color, vec3<f32>(0.0)),
            1.0,
        );
    }
    return vec4<f32>(water_aerial_color, 1.0);
}

fn terrain_fragment_color(input: VertexOutput) -> vec4<f32> {
    let direction = normalize(input.surface_direction);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let render_debug_mode = u32(camera.projection.w + 0.5);
    let outmap = input.outmap_and_macro_height.x > 0.5;
    let macro_height_meters = macro_terrain_height(outmap, input.source_uv, direction);
    let biome_id = sample_biome(input.source_uv);
    let ice = outmap && biome_id == 2u;
    let lake = outmap && biome_id == 1u;
    if render_debug_mode == RENDER_DEBUG_FLAT_TRIANGLES {
        // Flat mode is intentionally a single categorical terrain pass. Its
        // fixed L7 mesh can span a mixed L4 land/water source footprint, so
        // the analytic shell must not be allowed to compete for ownership;
        // retaining the skirt triangles here closes residual mixed-LOD edge
        // gaps. flat_triangle_edge() suppresses outlines on skirts, so they
        // fill cracks without adding a second wireframe band.
        return flat_triangle_colour(input);
    }
    // Open sea belongs exclusively to the analytic shell drawn after this
    // pass. Keep bathymetry out of the depth buffer unless this interpolated
    // triangle is visibly above sea level. A fallback source tile can sample
    // a negative texel at a fragment even though the displaced triangle was
    // built from a positive neighbouring sample; discarding that fragment
    // exposes the later shell as square holes in otherwise solid land.
    if is_open_ocean_surface(outmap, macro_height_meters, biome_id)
        && input.surface_height_and_fog_color.x <= 0.0
    {
        discard;
    }
    let lake_coverage = lake_coast_coverage(biome_id, macro_height_meters);
    if lake && lake_coverage > 0.0 {
        if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
            return vec4<f32>(mix(
                blended_biome_color(sample_biome_blend(input.source_uv)),
                debug_ocean_albedo(),
                lake_coverage,
            ), 1.0);
        }
        let surface = ocean_surface(direction, camera.projection.z);
        let water_base_height = terrain_height(
            outmap,
            input.source_uv,
            direction,
            length(input.camera_relative_view_position),
        );
        let water_surface_height = water_base_height + surface.vertical_displacement;
        let sun_transmittance = surface_direct_sun_transmittance(
            direction,
            water_surface_height,
            sun_direction,
        );
        let sky_diffuse = sky_diffuse_irradiance(
            surface.normal,
            direction,
            water_surface_height,
            sun_direction,
        );
        let water_surface_color = ocean_lighting(
            surface.normal,
            input.camera_relative_view_position,
            sun_transmittance,
            sky_diffuse,
        );
        if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
            return vec4<f32>(water_surface_color, 1.0);
        }
        let water_aerial_color = ocean_aerial_perspective(
            water_surface_color,
            input.camera_relative_view_position,
            direction,
            water_surface_height,
        );
        let lake_land_color = blended_biome_color(sample_biome_blend(input.source_uv));
        let lake_surface_color = mix(lake_land_color, water_surface_color, lake_coverage);
        let lake_aerial_color = mix(lake_land_color, water_aerial_color, lake_coverage);
        let misted_lake_aerial_color = apply_terrain_distance_fog(lake_aerial_color, input);
        if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
            return vec4<f32>(
                max(misted_lake_aerial_color - lake_surface_color, vec3<f32>(0.0)),
                1.0,
            );
        }
        return vec4<f32>(misted_lake_aerial_color, 1.0);
    }
    // Preserve the established shallow beach colour on positive terrain.
    // Actual open sea (macro height <= 0) was discarded above and is drawn by
    // the level shell, so this blend can no longer raise the ocean silhouette.
    let ocean_coverage = outmap_ocean_coverage(outmap, macro_height_meters);
    let biome_blend = sample_biome_blend(input.source_uv);
    let moisture = sample_moisture(input.source_uv);
    let base_biome_color = blended_biome_color(biome_blend);
    let terrain_normal = input.world_normal;
    let terrain_sun_transmittance = surface_direct_sun_transmittance(
        direction,
        input.surface_height_and_fog_color.x,
        sun_direction,
    );
    let terrain_sky_diffuse = sky_diffuse_irradiance(
        terrain_normal,
        direction,
        input.surface_height_and_fog_color.x,
        sun_direction,
    );
    let terrain_direct_light = max(dot(terrain_normal, sun_direction), 0.0);
    var terrain_cloud_visibility = 1.0;
    if terrain_direct_light > 0.0
        && dot(terrain_sun_transmittance, vec3<f32>(0.2126, 0.7152, 0.0722)) > 0.001
    {
        terrain_cloud_visibility = cloud_shadow_visibility(
            direction,
            input.surface_height_and_fog_color.x,
            sun_direction,
        );
    }
    let terrain_surface_irradiance = terrain_sky_diffuse
        + terrain_sun_transmittance * terrain_cloud_visibility
            * terrain_direct_light
            * SURFACE_SUNLIGHT_SCALE;
    let terrain_albedo = terrain_material_color(
        outmap,
        biome_id,
        moisture,
        base_biome_color,
        macro_height_meters,
        input.terrain_detail_meters_and_fog_amount.x,
        terrain_normal,
        direction,
    );
    let detail_tint = terrain_material_tint(
        outmap,
        moisture,
        biome_blend,
        macro_height_meters,
        terrain_albedo,
        direction,
        terrain_normal,
        input.camera_relative_view_position,
        input.terrain_detail_meters_and_fog_amount.x,
        input.detail_anchor_direction,
        input.detail_local_meters,
        terrain_material_fine_weight(
            length(input.camera_relative_view_position),
        ),
    );
    var textured_terrain_albedo = terrain_albedo * detail_tint;
    let weather_surface = weather_surface_sample(direction);
    let wetness = smoothstep(0.18, 0.82, weather_surface.r);
    let snow_cover = smoothstep(0.08, 0.70, weather_surface.g);
    // Rain darkens exposed ground and increases its broad specular response;
    // accumulated snow replaces the material only where the coupled surface
    // field says it has persisted. Ocean and lake branches returned above.
    textured_terrain_albedo *= 1.0 - 0.22 * wetness;
    textured_terrain_albedo = mix(
        textured_terrain_albedo,
        mix(vec3<f32>(0.70, 0.73, 0.76), vec3<f32>(0.94, 0.96, 1.0), snow_cover),
        snow_cover,
    );
    textured_terrain_albedo = far_forest_canopy_albedo(
        textured_terrain_albedo,
        outmap,
        biome_id,
        moisture,
        macro_height_meters,
        terrain_normal,
        direction,
        length(input.camera_relative_view_position),
        snow_cover,
    );
    if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
        return vec4<f32>(
            mix(textured_terrain_albedo, debug_ocean_albedo(), ocean_coverage),
            1.0,
        );
    }
    // The data still owns every vertex position. Only lighting changes here:
    // use one geometric normal for the whole rendered triangle instead of
    // rounding its gradient through interpolated vertex normals.
    var textured_surface_lighting = textured_terrain_albedo
        * terrain_surface_irradiance;
    let wet_specular = pow(
        max(
            dot(
                reflect(-sun_direction, terrain_normal),
                normalize(view_to_planet(-input.camera_relative_view_position)),
            ),
            0.0,
        ),
        64.0,
    ) * wetness
        * dot(terrain_sun_transmittance, vec3<f32>(0.2126, 0.7152, 0.0722))
        * 0.18;
    textured_surface_lighting += vec3<f32>(wet_specular);
    // Relief finer than the mesh can hold, shaded per pixel. The vertex ladder
    // stopped at its own spacing, so this picks up exactly the octaves it left.
    // Re-light the triangle normal by the finer-than-mesh detail ratio. The
    // offset keeps the divisor away from zero at the terminator and stops
    // grazing light exploding into white speckle.
    if input.detail_vertex_spacing_meters > 0.0 {
        // Rebuild the vertex's cutoff from this pixel's own camera distance,
        // using the same expression vs_main used. Both are continuous in
        // distance, so the handover slides smoothly instead of stepping.
        let pixel_filter_meters = terrain_detail_filter_meters(
            length(input.camera_relative_view_position),
        );
        let vertex_filter_meters = max(
            pixel_filter_meters,
            input.detail_vertex_spacing_meters,
        );
        if pixel_filter_meters < vertex_filter_meters {
            let fine_detail = terrain_detail_band(
                input.detail_anchor_direction,
                input.detail_local_meters,
                pixel_filter_meters,
                vertex_filter_meters,
                input.outmap_and_macro_height.y,
            );
            let vertex_normal = terrain_normal;
            let detail_normal = terrain_detail_perturbed_normal(
                vertex_normal,
                direction,
                fine_detail.slope,
            );
            let ambient = 0.18;
            let vertex_lambert = max(dot(vertex_normal, sun_direction), 0.0);
            let detail_lambert = max(dot(detail_normal, sun_direction), 0.0);
            // Fallback triangles can have a nearly unlit interpolated base
            // normal while their fine detail points toward the sun. Bound the
            // relight gain so that case cannot turn a whole coarse triangle
            // into a bright geometric patch.
            let detail_relight = clamp(
                (detail_lambert + ambient) / (vertex_lambert + ambient),
                0.55,
                1.75,
            );
            textured_surface_lighting *= detail_relight;
            // Close-range surface texture from the same field, rather than a
            // finer material tile: a 12m tile needs a 3e5 domain coordinate,
            // where f32 quantises the lookup to whole texels. This field is
            // already anchor-local and exact here, and costs nothing extra.
            // Normalised by the filter so the variation is scale-free.
            let surface_texture = clamp(
                fine_detail.height_meters / max(pixel_filter_meters, 0.05),
                -1.0,
                1.0,
            );
            textured_surface_lighting *= 1.0
                + surface_texture * TERRAIN_DETAIL_ALBEDO_STRENGTH;
        }
    }
    if outmap && biome_id == 2u {
        let ice_light_floor = clamp(
            max(
                max(terrain_surface_irradiance.x, terrain_surface_irradiance.y),
                terrain_surface_irradiance.z,
            ),
            0.0,
            1.0,
        );
        textured_surface_lighting = max(
            textured_surface_lighting,
            biome_color(2u) * 0.65 * ice_light_floor,
        );
    }
    textured_surface_lighting = neutralize_snow_surface_lighting(
        textured_surface_lighting,
        biome_id,
    );
    // Aerial perspective is affine: attenuate the fragment-frequency surface
    // by the interpolated view transmittance, then add in-scatter. Rebuilding
    // this from a ratio of two vertex colours used to require a hard threshold
    // near black. Low-sun shadows crossed that threshold per channel, lifting
    // their interiors by up to 16x while leaving a dark outline at the switch.
    let textured_aerial_color = textured_surface_lighting
        * terrain_material_transmittance(input.aerial_transmittance, biome_id)
        + terrain_material_in_scatter(input.aerial_in_scatter, biome_id);
    let misted_textured_aerial_color = apply_terrain_distance_fog(
        textured_aerial_color,
        input,
    );
    if ocean_coverage <= 0.0 {
        if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
            return vec4<f32>(textured_surface_lighting, 1.0);
        }
        if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
            return vec4<f32>(
                max(misted_textured_aerial_color - textured_surface_lighting, vec3<f32>(0.0)),
                1.0,
            );
        }
        return vec4<f32>(misted_textured_aerial_color, 1.0);
    }
    let surface = ocean_surface(direction, camera.projection.z);
    let sun_transmittance = surface_direct_sun_transmittance(
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        surface.normal,
        direction,
        surface.vertical_displacement,
        sun_direction,
    );
    let water_surface_color = ocean_lighting(
        surface.normal,
        input.camera_relative_view_position,
        sun_transmittance,
        sky_diffuse,
    );
    let water_aerial_color = ocean_aerial_perspective(
        water_surface_color,
        input.camera_relative_view_position,
        direction,
        surface.vertical_displacement,
    );
    let surface_color = mix(textured_surface_lighting, water_surface_color, ocean_coverage);
    let aerial_color = mix(textured_aerial_color, water_aerial_color, ocean_coverage);
    let misted_aerial_color = apply_terrain_distance_fog(aerial_color, input);
    if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
        return vec4<f32>(surface_color, 1.0);
    }
    if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
        return vec4<f32>(max(misted_aerial_color - surface_color, vec3<f32>(0.0)), 1.0);
    }
    return vec4<f32>(misted_aerial_color, 1.0);
}

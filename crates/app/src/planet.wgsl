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
    @location(2) aerial_color: vec3<f32>,
    @location(3) lod_transition: vec2<f32>,
    @location(4) surface_direction: vec3<f32>,
    @location(5) ocean: f32,
    @location(6) source_uv: vec2<f32>,
    @location(7) outmap: f32,
    @location(8) surface_lighting: vec3<f32>,
    @location(9) terrain_detail_meters: f32,
    @location(10) surface_irradiance: vec3<f32>,
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

fn sample_height(source_uv: vec2<f32>) -> f32 {
    let gutter_uv = 1.0 / MATERIAL_TILE_LOGICAL_QUADS;
    let coordinate = vec2<f32>(TILE_GUTTER)
        + clamp(source_uv, vec2<f32>(-gutter_uv), vec2<f32>(1.0 + gutter_uv))
            * MATERIAL_TILE_LOGICAL_QUADS;
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(
        lower + vec2<i32>(1),
        vec2<i32>(MATERIAL_TILE_LAST_STORED_COORD),
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
    return macro_height * terrain_macro_height_scale();
}

fn sample_biome(source_uv: vec2<f32>) -> u32 {
    let coordinate = vec2<i32>(round(
        vec2<f32>(TILE_GUTTER)
            + clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0))
                * MATERIAL_TILE_LOGICAL_QUADS,
    ));
    return textureLoad(biome_map, coordinate, 0).x;
}

fn sample_biome_blend(source_uv: vec2<f32>) -> BiomeBlendSample {
    // Biomes remain categorical in the baked outmap, but display-space
    // materials should not expose their texel grid. Blend the four nearest
    // baked owners exactly as the height channel is blended; the gutter keeps
    // this continuous when the resident source changes at a tile edge.
    let coordinate = vec2<f32>(TILE_GUTTER)
        + clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0))
            * MATERIAL_TILE_LOGICAL_QUADS;
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(
        lower + vec2<i32>(1),
        vec2<i32>(MATERIAL_TILE_LAST_STORED_COORD),
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
    let coordinate = vec2<f32>(TILE_GUTTER)
        + clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0))
            * MATERIAL_TILE_LOGICAL_QUADS;
    let lower = vec2<i32>(floor(coordinate));
    let upper = min(
        lower + vec2<i32>(1),
        vec2<i32>(MATERIAL_TILE_LAST_STORED_COORD),
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
    let normal_sample_meters = clamp(
        camera_distance_meters * 0.01,
        TERRAIN_NORMAL_MIN_SAMPLE_METERS,
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
    return (edge_stitch >> (edge * 3u)) & 0x7u;
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

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
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
    let source_uv = input.source_uv_offset + tile_uv * input.source_uv_scale;
    let outmap = uses_outmap(input.terrain_info);
    let macro_height = macro_terrain_height(outmap, source_uv, direction);
    let base_camera_relative_view_position = input.anchor_view_position
        + planet_to_view(anchor_relative_position);
    let camera_distance_meters = length(base_camera_relative_view_position);
    // Baked tiles own geometric detail. Retain this zero varying for the
    // material interface without evaluating the retired runtime noise.
    let terrain_detail_meters = 0.0;
    let height = select(
        macro_height,
        macro_height * terrain_macro_height_scale(),
        outmap,
    );
    // Polar ice overrides ocean in the baked biome contract. Lift it just
    // above sea level so the cap remains visible rather than becoming water.
    let biome_id = sample_biome(source_uv);
    let biome_blend = sample_biome_blend(source_uv);
    let moisture = sample_moisture(source_uv);
    let base_biome_color = blended_biome_color(biome_blend);
    let ice = outmap && biome_id == 2u;
    let lake = outmap && biome_id == 1u;
    let ocean = (macro_height <= 0.0 || lake) && !ice;
    let wave_surface = ocean_surface(direction, camera.projection.z);
    let land_height = select(height, max(height, 5.0), ice);
    let water_base_height = select(0.0, height, lake);
    let surface_height = select(
        land_height,
        water_base_height + wave_surface.vertical_displacement,
        ocean,
    );
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
        + direction * (surface_height - skirt_depth_meters)
        + select(vec3<f32>(0.0), wave_surface.horizontal_displacement, ocean);
    let camera_relative_view_position = input.anchor_view_position
        + planet_to_view(local_planet_position);
    var normal = displaced_surface_normal(
        direction,
        source_uv,
        input.source_uv_scale,
        input.terrain_info,
        camera_distance_meters,
    );
    if ocean {
        normal = wave_surface.normal;
    }
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun_transmittance = surface_direct_sun_transmittance(
        direction,
        surface_height,
        sun_direction,
    );
    let sky_diffuse = sky_diffuse_irradiance(
        normal,
        direction,
        surface_height,
        sun_direction,
    );
    let direct_light = max(dot(normal, sun_direction), 0.0);
    let surface_irradiance = sky_diffuse
        + sun_transmittance * direct_light * SURFACE_SUNLIGHT_SCALE;
    var lit_surface_color = terrain_material_color(
        outmap,
        biome_id,
        moisture,
        base_biome_color,
        macro_height,
        terrain_detail_meters,
        normal,
        direction,
    ) * surface_irradiance;
    if ice {
        // Keep daylight snow neutral without creating an emissive floor after
        // direct and atmospheric illumination are fully occulted.
        let ice_light_floor = clamp(
            max(max(surface_irradiance.x, surface_irradiance.y), surface_irradiance.z),
            0.0,
            1.0,
        );
        lit_surface_color = max(
            lit_surface_color,
            biome_color(2u) * 0.65 * ice_light_floor,
        );
    }
    var aerial_color = aerial_perspective(
        lit_surface_color,
        camera_relative_view_position,
        direction,
        surface_height,
    );
    if !ocean {
        aerial_color = terrain_distance_fog(
            aerial_color,
            camera_relative_view_position,
            direction,
            surface_height,
        );
    }
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(camera_relative_view_position, 1.0),
        camera_relative_view_position,
        normal,
        aerial_color,
        input.lod_transition,
        direction,
        select(0.0, 1.0, ocean),
        source_uv,
        select(0.0, 1.0, outmap),
        lit_surface_color,
        terrain_detail_meters,
        surface_irradiance,
    );
}

fn lod_dither_threshold(fragment_position: vec4<f32>) -> f32 {
    // Stable interleaved-gradient noise avoids the visible checker/grid of an
    // ordered matrix. Parent and child still evaluate the exact same threshold
    // at a screen pixel, so their coverage remains complementary.
    let pixel = floor(fragment_position.xy);
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
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

fn terrain_fragment_color(input: VertexOutput) -> vec4<f32> {
    let direction = normalize(input.surface_direction);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let render_debug_mode = u32(camera.projection.w + 0.5);
    if input.ocean > 0.5 {
        if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
            return vec4<f32>(debug_ocean_albedo(), 1.0);
        }
        let surface = ocean_surface(direction, camera.projection.z);
        let outmap = input.outmap > 0.5;
        let lake = outmap && sample_biome(input.source_uv) == 1u;
        let water_base_height = select(
            0.0,
            terrain_height(
                outmap,
                input.source_uv,
                direction,
                length(input.camera_relative_view_position),
            ),
            lake,
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
        if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
            return vec4<f32>(max(water_aerial_color - water_surface_color, vec3<f32>(0.0)), 1.0);
        }
        return vec4<f32>(water_aerial_color, 1.0);
    }
    let outmap = input.outmap > 0.5;
    let macro_height_meters = macro_terrain_height(outmap, input.source_uv, direction);
    let ocean_coverage = outmap_ocean_coverage(outmap, macro_height_meters);
    let biome_id = sample_biome(input.source_uv);
    let biome_blend = sample_biome_blend(input.source_uv);
    let moisture = sample_moisture(input.source_uv);
    let base_biome_color = blended_biome_color(biome_blend);
    let terrain_albedo = terrain_material_color(
        outmap,
        biome_id,
        moisture,
        base_biome_color,
        macro_height_meters,
        input.terrain_detail_meters,
        input.world_normal,
        direction,
    );
    let detail_tint = terrain_material_tint(
        outmap,
        moisture,
        biome_blend,
        macro_height_meters,
        terrain_albedo,
        direction,
        input.world_normal,
        input.camera_relative_view_position,
    );
    let textured_terrain_albedo = terrain_albedo * detail_tint;
    if render_debug_mode == RENDER_DEBUG_RAW_ALBEDO {
        return vec4<f32>(
            mix(textured_terrain_albedo, debug_ocean_albedo(), ocean_coverage),
            1.0,
        );
    }
    // Biome, moisture and triplanar material are fragment-frequency values.
    // Apply the interpolated illumination to the fragment albedo instead of
    // tinting a vertex-frequency biome colour; the latter made coarse leaves
    // read as large Gouraud-shaded software-rendered triangles.
    var textured_surface_lighting = textured_terrain_albedo * input.surface_irradiance;
    if outmap && biome_id == 2u {
        let ice_light_floor = clamp(
            max(
                max(input.surface_irradiance.x, input.surface_irradiance.y),
                input.surface_irradiance.z,
            ),
            0.0,
            1.0,
        );
        textured_surface_lighting = max(
            textured_surface_lighting,
            biome_color(2u) * 0.65 * ice_light_floor,
        );
    }
    let textured_aerial_color = textured_surface_lighting
        + max(input.aerial_color - input.surface_lighting, vec3<f32>(0.0));
    if ocean_coverage <= 0.0 {
        if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
            return vec4<f32>(textured_surface_lighting, 1.0);
        }
        if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
            return vec4<f32>(
                max(textured_aerial_color - textured_surface_lighting, vec3<f32>(0.0)),
                1.0,
            );
        }
        return vec4<f32>(textured_aerial_color, 1.0);
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
    if render_debug_mode == RENDER_DEBUG_SURFACE_LIGHTING {
        return vec4<f32>(surface_color, 1.0);
    }
    if render_debug_mode == RENDER_DEBUG_AERIAL_CONTRIBUTION {
        return vec4<f32>(max(aerial_color - surface_color, vec3<f32>(0.0)), 1.0);
    }
    return vec4<f32>(
        aerial_color,
        1.0,
    );
}

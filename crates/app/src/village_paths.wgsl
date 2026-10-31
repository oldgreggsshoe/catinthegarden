// Sparse, cached ground wear. The terrain owns the pixels and their lighting.
@group(2) @binding(22) var<storage, read> village_path_data: array<vec4<f32>>;
fn village_path_local_coverage(p: vec2<f32>, filter_width: f32) -> f32 {
    if village_path_data[0].w < 0.5 { return 0.0; }
    let coarse = vec2<i32>(floor(p / 128.0)) + vec2<i32>(128);
    if any(coarse < vec2<i32>(0)) || any(coarse >= vec2<i32>(256)) { return 0.0; }
    let bit = u32(coarse.y * 256 + coarse.x);
    let word = bit / 16u;
    let occupied = u32(village_path_data[4u + word / 4u][word % 4u]);
    if (occupied & (1u << (bit % 16u))) == 0u { return 0.0; }
    let cell = vec2<i32>(floor(p / 16.0));
    let hash = (u32(cell.x) * 73856093u ^ u32(cell.y) * 19349663u) & 8191u;
    for (var probe = 0u; probe < 32u; probe += 1u) {
        let bucket = village_path_data[1028u + ((hash + probe) & 8191u)];
        if bucket.w < 0.5 { return 0.0; }
        if any(vec2<i32>(bucket.xy) != cell) { continue; }
        var wear = 0.0;
        for (var j = 0u; j < min(u32(bucket.w), 128u); j += 1u) {
            let r = u32(bucket.z) + j;
            let index = u32(village_path_data[9220u + r / 4u][r % 4u]);
            let segment = village_path_data[41988u + index * 2u];
            let width = village_path_data[41989u + index * 2u].x;
            let delta = segment.zw - segment.xy;
            let t = clamp(dot(p - segment.xy, delta) / max(dot(delta, delta), 0.001), 0.0, 1.0);
            let distance = length(p - mix(segment.xy, segment.zw, t));
            let fringe = max(filter_width, 0.28);
            // Soft rounded unions, not overlapping translucent ribbons.
            wear = max(wear, 1.0 - smoothstep(width - fringe * 0.5, width + fringe, distance));
        }
        return wear;
    }
    return 0.0;
}
// Ground shaping under the paths, mirrored on the CPU by `village_paths::ground_shape`
// and `terrain.rs`. Full within a path's half-width, fading out over the bank.
const VILLAGE_PATH_BANK_METERS: f32 = 1.2;
// The ramp to the court floor: full this close to the path mouth, gone by the end.
const VILLAGE_PATH_RAMP_FULL_METERS: f32 = 3.0;
const VILLAGE_PATH_RAMP_END_METERS: f32 = 15.0;
// Relief shorter than this is mostly smoothed away under a path.
const VILLAGE_PATH_SMOOTH_METERS: f32 = 12.0;
const VILLAGE_PATH_SMOOTHING: f32 = 0.8;
// Never move the ground more than this (plus the ramp's allowance at the mouth).
const VILLAGE_PATH_MAX_CHANGE_METERS: f32 = 0.8;
const VILLAGE_PATH_RAMP_MAX_CHANGE_METERS: f32 = 3.2;
// Shaping fades out as the mesh's detail filter (1% of distance) passes the
// width of a path: the mesh cannot carry it further away, and the lookup
// costs a vertex shader search. Mirrored by `village_paths::distance_fade`.
const VILLAGE_PATH_FADE_FILTER_START_METERS: f32 = 2.0;
const VILLAGE_PATH_FADE_FILTER_END_METERS: f32 = 4.0;

// Ground-shape atlas (village_paths.rs): per-slot tile numbers, then one
// tile per occupied 16 m cell -- a header (court floor, mouth x/y, has mouth)
// and 17x17 samples 1 m apart, each two 12-bit bank distances in one float.
const VILLAGE_PATH_TILE_INDEX: u32 = 74756u;
const VILLAGE_PATH_ATLAS: u32 = 76804u;
const VILLAGE_PATH_TILE_SIDE: u32 = 17u;
const VILLAGE_PATH_TILE_FLOAT4S: u32 = 74u;

// Stored range of the bank distances (village_paths.rs BANK_STORED_*).
const VILLAGE_PATH_BANK_STORED_MIN: f32 = -3.0;
const VILLAGE_PATH_BANK_STORED_MAX: f32 = 2.0;

/// (bank distance to any path, to any path with a court mouth): 0 at the
/// path's edge, 1 at the bank's outer edge, negative on the path.
fn village_path_tile_sample(base: u32, x: u32, y: u32) -> vec2<f32> {
    let index = y * VILLAGE_PATH_TILE_SIDE + x;
    let packed = u32(village_path_data[base + 1u + index / 4u][index % 4u]);
    return vec2<f32>(f32(packed & 4095u), f32(packed >> 12u)) / 4095.0
        * (VILLAGE_PATH_BANK_STORED_MAX - VILLAGE_PATH_BANK_STORED_MIN)
        + VILLAGE_PATH_BANK_STORED_MIN;
}

/// (path weight, ramp weight, court floor height, ramp distance factor) at a
/// path-frame point: how strongly the ground here belongs to a path, and how
/// strongly it should run to its court's floor. Four baked samples, blended;
/// the CPU twin is `village_paths::ground_shape`.
fn village_path_ground_shape(p: vec2<f32>) -> vec4<f32> {
    if village_path_data[0].w < 0.5 || village_path_data[3].w < 0.5 { return vec4<f32>(0.0); }
    return village_path_ground_shape_published(p);
}
/// As `village_path_ground_shape`, for callers that have already checked the
/// paths are published with shaping on.
fn village_path_ground_shape_published(p: vec2<f32>) -> vec4<f32> {
    var shape = vec4<f32>(0.0);
    let coarse = vec2<i32>(floor(p / 128.0)) + vec2<i32>(128);
    if any(coarse < vec2<i32>(0)) || any(coarse >= vec2<i32>(256)) { return shape; }
    let bit = u32(coarse.y * 256 + coarse.x);
    let word = bit / 16u;
    let occupied = u32(village_path_data[4u + word / 4u][word % 4u]);
    if (occupied & (1u << (bit % 16u))) == 0u { return shape; }
    let cell = vec2<i32>(floor(p / 16.0));
    let hash = (u32(cell.x) * 73856093u ^ u32(cell.y) * 19349663u) & 8191u;
    for (var probe = 0u; probe < 32u; probe += 1u) {
        let slot = (hash + probe) & 8191u;
        let bucket = village_path_data[1028u + slot];
        if bucket.w < 0.5 { return shape; }
        if any(vec2<i32>(bucket.xy) != cell) { continue; }
        let tile = u32(village_path_data[VILLAGE_PATH_TILE_INDEX + slot / 4u][slot % 4u]);
        let base = VILLAGE_PATH_ATLAS + tile * VILLAGE_PATH_TILE_FLOAT4S;
        let local = clamp(p - vec2<f32>(cell) * 16.0, vec2<f32>(0.0), vec2<f32>(16.0));
        let i = min(vec2<u32>(floor(local)), vec2<u32>(15u));
        let f = local - vec2<f32>(i);
        let bank = mix(
            mix(village_path_tile_sample(base, i.x, i.y), village_path_tile_sample(base, i.x + 1u, i.y), f.x),
            mix(village_path_tile_sample(base, i.x, i.y + 1u), village_path_tile_sample(base, i.x + 1u, i.y + 1u), f.x),
            f.y,
        );
        shape.x = 1.0 - smoothstep(0.0, 1.0, bank.x);
        let header = village_path_data[base];
        let ramp_weight = 1.0 - smoothstep(0.0, 1.0, bank.y);
        if header.w > 0.5 && ramp_weight > 0.0 {
            let near_mouth = 1.0 - smoothstep(VILLAGE_PATH_RAMP_FULL_METERS,
                VILLAGE_PATH_RAMP_END_METERS, length(p - header.yz));
            if ramp_weight * near_mouth > 0.0 {
                shape = vec4<f32>(shape.x, ramp_weight * near_mouth, header.x, near_mouth);
            }
        }
        return shape;
    }
    return shape;
}
fn village_path_xy_from_offset(planet_offset: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(dot(planet_offset, village_path_data[0].xyz), dot(planet_offset, village_path_data[1].xyz))
        + village_path_data[2].xy;
}
/// (wear, path-frame x, y) for this pixel; wear 0 off the paths.
fn village_path_fragment(planet_offset: vec3<f32>, height: f32) -> vec3<f32> {
    if village_path_data[0].w < 0.5 { return vec3<f32>(0.0); }
    let p = village_path_xy_from_offset(planet_offset) * village_path_data[1].w / (village_path_data[1].w + height);
    let pixel_width = max(length(dpdx(p)), length(dpdy(p)));
    return vec3<f32>(village_path_local_coverage(p, pixel_width), p);
}
fn village_path_albedo(albedo: vec3<f32>, planet_offset: vec3<f32>, height: f32) -> vec3<f32> {
    return village_path_albedo_from(albedo, village_path_fragment(planet_offset, height), 1.0);
}
fn village_path_albedo_from(albedo: vec3<f32>, path: vec3<f32>, ground_grain: f32) -> vec3<f32> {
    let wear = path.x;
    let p = path.yz;
    if wear <= 0.0 { return albedo; }
    // Muted compacted soil, with low-contrast broad mottling; the terrain's
    // existing detail, normals, rain/snow and illumination remain in charge.
    let mottling = 0.94 + 0.06 * sin(p.x * 1.7 + sin(p.y * 0.8)) * sin(p.y * 2.1);
    // Reuse the terrain's already mip-filtered grain instead of painting over
    // it with flat brown. The flat/categorical debug path supplies unity.
    let grain = clamp(ground_grain, 0.6, 1.4);
    let earth = vec3<f32>(0.19, 0.135, 0.082) * mottling * grain;
    return mix(albedo, mix(albedo * vec3<f32>(0.88, 0.72, 0.55), earth, 0.8), wear * 0.9);
}
fn village_path_clears_tree(direction: vec3<f32>) -> bool {
    if village_path_data[0].w < 0.5 || dot(direction, village_path_data[3].xyz) < 0.99 { return false; }
    let p = vec2<f32>(dot(direction, village_path_data[0].xyz), dot(direction, village_path_data[1].xyz)) * village_path_data[1].w;
    return village_path_local_coverage(p, 1.5) > 0.05;
}

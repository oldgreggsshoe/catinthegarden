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
fn village_path_xy_from_offset(planet_offset: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(dot(planet_offset, village_path_data[0].xyz), dot(planet_offset, village_path_data[1].xyz))
        + village_path_data[2].xy;
}
fn village_path_albedo(albedo: vec3<f32>, planet_offset: vec3<f32>, height: f32) -> vec3<f32> {
    if village_path_data[0].w < 0.5 { return albedo; }
    let p = village_path_xy_from_offset(planet_offset) * village_path_data[1].w / (village_path_data[1].w + height);
    let pixel_width = max(length(dpdx(p)), length(dpdy(p)));
    let wear = village_path_local_coverage(p, pixel_width);
    if wear <= 0.0 { return albedo; }
    // Muted compacted soil, with low-contrast broad mottling; the terrain's
    // existing detail, normals, rain/snow and illumination remain in charge.
    let mottling = 0.94 + 0.06 * sin(p.x * 1.7 + sin(p.y * 0.8)) * sin(p.y * 2.1);
    let earth = vec3<f32>(0.19, 0.135, 0.082) * mottling;
    return mix(albedo, mix(albedo * vec3<f32>(0.88, 0.72, 0.55), earth, 0.8), wear * 0.9);
}
fn village_path_clears_tree(direction: vec3<f32>) -> bool {
    if village_path_data[0].w < 0.5 || dot(direction, village_path_data[3].xyz) < 0.99 { return false; }
    let p = vec2<f32>(dot(direction, village_path_data[0].xyz), dot(direction, village_path_data[1].xyz)) * village_path_data[1].w;
    return village_path_local_coverage(p, 1.5) > 0.05;
}

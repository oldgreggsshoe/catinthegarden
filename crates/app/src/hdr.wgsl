struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

struct Exposure {
    exposure: f32,
    hdr_effect_enabled: u32,
    presentation_size: vec2<f32>,
}

@group(0) @binding(0)
var source_texture: texture_2d<f32>;

@group(0) @binding(1)
var<uniform> exposure: Exposure;

@group(0) @binding(2)
var effect_texture: texture_2d<f32>;

fn luminance(color: vec3<f32>) -> f32 {
    return dot(max(color, vec3<f32>(0.0)), vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn aces_filmic(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((color * (a * color + vec3<f32>(b))) / (color * (c * color + vec3<f32>(d)) + vec3<f32>(e)), vec3<f32>(0.0), vec3<f32>(1.0));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return VertexOutput(vec4<f32>(positions[vertex_index], 0.0, 1.0));
}

@fragment
fn luminance_from_scene(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let value = luminance(textureLoad(source_texture, pixel, 0).rgb);
    // R carries the local weighted average and G its relative source-pixel
    // weight. The weight is renormalized at every mip, which keeps it in the
    // useful Rgba16Float range even for 4K+ render targets. Empty black space
    // has no photographic bearing on the visible planet, so smoothly reject
    // it while retaining dim terrain and the atmospheric limb.
    let metering_weight = smoothstep(0.002, 0.02, value);
    return vec4<f32>(value, metering_weight, 0.0, 1.0);
}

@fragment
fn luminance_downsample(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let output_pixel = vec2<i32>(position.xy);
    let source_size = vec2<i32>(textureDimensions(source_texture));
    let output_size = max(source_size / 2, vec2<i32>(1));
    let source_begin = output_pixel * source_size / output_size;
    let source_end = (output_pixel + vec2<i32>(1)) * source_size / output_size;
    var weighted_luminance_sum = 0.0;
    var texel_weight = 0.0;
    for (var y = source_begin.y; y < source_end.y; y += 1) {
        for (var x = source_begin.x; x < source_end.x; x += 1) {
            let sample_value = textureLoad(source_texture, vec2<i32>(x, y), 0).rg;
            weighted_luminance_sum += sample_value.x * sample_value.y;
            texel_weight += sample_value.y;
        }
    }
    let max_samples_per_axis = (source_size + output_size - vec2<i32>(1)) / output_size;
    let max_samples_per_output = max_samples_per_axis.x * max_samples_per_axis.y;
    return vec4<f32>(
        weighted_luminance_sum / max(texel_weight, 1.0e-8),
        texel_weight / f32(max_samples_per_output),
        0.0,
        1.0,
    );
}

@fragment
fn tone_map(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let source_size = vec2<f32>(textureDimensions(source_texture));
    let presentation_size = max(exposure.presentation_size, vec2<f32>(1.0));
    let source_position = position.xy * source_size / presentation_size;
    let pixel = min(
        vec2<i32>(source_position),
        vec2<i32>(textureDimensions(source_texture)) - vec2<i32>(1),
    );
    let hdr_color = textureLoad(source_texture, pixel, 0).rgb;
    if (exposure.hdr_effect_enabled == 0u) {
        // HDR-off is a display-curve toggle, not an exposure toggle. Preserve
        // auto-exposure so dim atmospheric haze remains visible.
        return vec4<f32>(hdr_color * exposure.exposure, 1.0);
    }
    return vec4<f32>(aces_filmic(hdr_color * exposure.exposure), 1.0);
}

// FXAA-style edge-aware anti-aliasing (replaces the old box blur). Edges are
// found on tone-compressed luma so bright HDR values cannot dominate, and the
// blend toward the neighbour across the edge is luma-weighted so a bright
// pixel does not swamp a dark one. Flat areas and fine texture pass through.
fn aa_luma(color: vec3<f32>) -> f32 {
    let luma = dot(color, vec3<f32>(0.299, 0.587, 0.114));
    return luma / (1.0 + luma);
}

fn aa_load(pixel: vec2<i32>, size: vec2<i32>) -> vec3<f32> {
    return textureLoad(source_texture, clamp(pixel, vec2<i32>(0), size - vec2<i32>(1)), 0).rgb;
}

@fragment
fn blur_scene(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let size = vec2<i32>(textureDimensions(source_texture));
    let rgb_m = aa_load(pixel, size);
    let rgb_n = aa_load(pixel + vec2<i32>(0, -1), size);
    let rgb_s = aa_load(pixel + vec2<i32>(0, 1), size);
    let rgb_w = aa_load(pixel + vec2<i32>(-1, 0), size);
    let rgb_e = aa_load(pixel + vec2<i32>(1, 0), size);
    let luma_m = aa_luma(rgb_m);
    let luma_n = aa_luma(rgb_n);
    let luma_s = aa_luma(rgb_s);
    let luma_w = aa_luma(rgb_w);
    let luma_e = aa_luma(rgb_e);
    let luma_min = min(luma_m, min(min(luma_n, luma_s), min(luma_w, luma_e)));
    let luma_max = max(luma_m, max(max(luma_n, luma_s), max(luma_w, luma_e)));
    let range = luma_max - luma_min;
    if range < max(0.0312, luma_max * 0.125) {
        return vec4<f32>(rgb_m, 1.0);
    }
    let luma_nw = aa_luma(aa_load(pixel + vec2<i32>(-1, -1), size));
    let luma_ne = aa_luma(aa_load(pixel + vec2<i32>(1, -1), size));
    let luma_sw = aa_luma(aa_load(pixel + vec2<i32>(-1, 1), size));
    let luma_se = aa_luma(aa_load(pixel + vec2<i32>(1, 1), size));
    let edge_horizontal = abs(luma_nw + luma_sw - 2.0 * luma_w)
        + 2.0 * abs(luma_n + luma_s - 2.0 * luma_m)
        + abs(luma_ne + luma_se - 2.0 * luma_e);
    let edge_vertical = abs(luma_nw + luma_ne - 2.0 * luma_n)
        + 2.0 * abs(luma_w + luma_e - 2.0 * luma_m)
        + abs(luma_sw + luma_se - 2.0 * luma_s);
    let horizontal = edge_horizontal >= edge_vertical;
    // Neighbour across the edge with the steeper gradient.
    var toward = rgb_w;
    var luma_toward = luma_w;
    var away_luma = luma_e;
    var away = rgb_e;
    if horizontal {
        toward = rgb_n;
        luma_toward = luma_n;
        away = rgb_s;
        away_luma = luma_s;
    }
    if abs(away_luma - luma_m) > abs(luma_toward - luma_m) {
        toward = away;
    }
    // Sub-pixel aliasing (isolated bright or dark pixels) gets a stronger blend.
    let luma_average = (luma_n + luma_s + luma_w + luma_e) * 0.25;
    var subpixel = clamp(abs(luma_average - luma_m) / range, 0.0, 1.0);
    subpixel = smoothstep(0.0, 1.0, subpixel);
    let weight = clamp(0.5 + 0.25 * subpixel * subpixel, 0.0, 0.75);
    let weight_m = (1.0 - weight) / (1.0 + dot(rgb_m, vec3<f32>(0.299, 0.587, 0.114)));
    let weight_t = weight / (1.0 + dot(toward, vec3<f32>(0.299, 0.587, 0.114)));
    return vec4<f32>((rgb_m * weight_m + toward * weight_t) / (weight_m + weight_t), 1.0);
}

@fragment
fn bloom_blur(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let size = vec2<i32>(textureDimensions(source_texture));
    var color = vec3<f32>(0.0);
    var weight = 0.0;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let offset = vec2<i32>(x, y);
            let sample_weight = select(1.0, 2.0, x == 0 || y == 0);
            let sample_color = textureLoad(
                source_texture,
                clamp(pixel + offset, vec2<i32>(0), size - vec2<i32>(1)),
                0,
            ).rgb;
            color += max(sample_color - vec3<f32>(1.0), vec3<f32>(0.0)) * sample_weight;
            weight += sample_weight;
        }
    }
    return vec4<f32>(color / weight, 1.0);
}

@fragment
fn bloom_composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let scene = textureLoad(source_texture, pixel, 0).rgb;
    let bloom = textureLoad(effect_texture, pixel, 0).rgb;
    return vec4<f32>(scene + bloom * 0.75, 1.0);
}

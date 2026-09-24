// Render-side FFT sea: samples the cascades written by ocean_fft.wgsl.
// Included by ocean_fft.rs::wgsl_source only when the FFT sea is enabled.
//
// The tiles are planar. Three orthographic projections, one per planet axis,
// carry them onto the sphere, blended by |direction.axis|^8 and divided by the
// root of the summed squared weights so a two-way blend keeps the sea's height
// variance. Texture coordinates come from the camera-relative offset of the
// undisplaced sea point, plus the camera's own position in each tile, wrapped
// in f64 on the CPU. A 4,000km planet-frame position cannot address a 16cm
// texel in f32; a camera-relative offset can.

@group(2) @binding(16) var ocean_fft_displacement: texture_2d_array<f32>;
@group(2) @binding(17) var ocean_fft_derivatives: texture_2d_array<f32>;
@group(2) @binding(18) var ocean_fft_sampler: sampler;

struct OceanFftRenderParams {
    // Per cascade: fract(camera.xyz / L), planet frame.
    tile_offsets: array<vec4<f32>, 3>,
    // xyz: camera position (planet frame, approximate); w: radians per pixel.
    camera_position_pixel_angle: vec4<f32>,
}
@group(2) @binding(19) var<uniform> ocean_fft_params: OceanFftRenderParams;

struct OceanFftSample {
    horizontal: vec3<f32>,
    height: f32,
    // Tangent gradient of height, corrected for the horizontal compression so
    // choppy crests shade as steep as they are drawn.
    slope: vec3<f32>,
    // 1 - Jacobian: positive where the choppy displacement squeezes water into
    // a crest. The peak mask for crest colour and foam.
    crest: f32,
}

fn ocean_fft_axis_component(value: vec3<f32>, axis: u32) -> f32 {
    return select(select(value.z, value.y, axis == 1u), value.x, axis == 0u);
}

fn ocean_fft_axis(axis: u32) -> vec3<f32> {
    return select(select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), axis == 1u),
        vec3<f32>(1.0, 0.0, 0.0), axis == 0u);
}

/// `footprint_floor_meters` is the smallest ground footprint the caller can
/// represent: a mesh's vertex spacing, or zero for a per-pixel query. The
/// pixel footprint at this distance is applied on top.
fn ocean_fft_sample(
    direction: vec3<f32>,
    camera_relative: vec3<f32>,
    camera_distance_meters: f32,
    footprint_floor_meters: f32,
    sigma: f32,
) -> OceanFftSample {
    var weights = pow(abs(direction), vec3<f32>(OCEAN_FFT_PROJECTION_EXPONENT));
    weights = weights / (weights.x + weights.y + weights.z);
    weights = select(weights, vec3<f32>(0.0), weights < vec3<f32>(OCEAN_FFT_PROJECTION_MINIMUM_WEIGHT));
    let norm = length(weights);
    let footprint = max(
        max(camera_distance_meters, 0.01) * ocean_fft_params.camera_position_pixel_angle.w,
        footprint_floor_meters,
    );
    var horizontal = vec3<f32>(0.0);
    var height = 0.0;
    var slope = vec3<f32>(0.0);
    var crest = 0.0;
    for (var projection = 0u; projection < 3u; projection = projection + 1u) {
        let weight = ocean_fft_axis_component(weights, projection);
        if weight <= 0.0 {
            continue;
        }
        // Projection p looks down axis p; (u, v) are the next two axes.
        let u_axis = (projection + 1u) % 3u;
        let v_axis = (projection + 2u) % 3u;
        let relative_uv = vec2<f32>(
            ocean_fft_axis_component(camera_relative, u_axis),
            ocean_fft_axis_component(camera_relative, v_axis),
        );
        // displacement: (Dx, h, Dz, dDx/dz); derivatives: (dh/dx, dh/dz, dDx/dx, dDz/dz)
        var displacement = vec4<f32>(0.0);
        var derivatives = vec4<f32>(0.0);
        for (var cascade = 0u; cascade < 3u; cascade = cascade + 1u) {
            let size = OCEAN_FFT_CASCADE_SIZES[cascade];
            let lod = log2(max(footprint * OCEAN_FFT_N / size, 1.0e-6));
            // Past the last mip a tile averages to its (zero) mean.
            if lod >= 7.5 {
                continue;
            }
            let offset = ocean_fft_params.tile_offsets[cascade];
            let uv = vec2<f32>(
                ocean_fft_axis_component(offset.xyz, u_axis),
                ocean_fft_axis_component(offset.xyz, v_axis),
            ) + relative_uv / size + 0.5 / OCEAN_FFT_N;
            let level = max(lod, 0.0);
            displacement += textureSampleLevel(ocean_fft_displacement, ocean_fft_sampler, uv, cascade, level);
            derivatives += textureSampleLevel(ocean_fft_derivatives, ocean_fft_sampler, uv, cascade, level);
        }
        displacement *= sigma;
        derivatives *= sigma;
        let u_world = ocean_fft_axis(u_axis);
        let v_world = ocean_fft_axis(v_axis);
        let u_tangent = u_world - direction * dot(direction, u_world);
        let v_tangent = v_world - direction * dot(direction, v_world);
        let w = weight / norm;
        let jacobian = (1.0 + derivatives.z) * (1.0 + derivatives.w) - displacement.w * displacement.w;
        horizontal += (u_tangent * displacement.x + v_tangent * displacement.z) * w;
        height += displacement.y * w;
        slope += (u_tangent * (derivatives.x / max(1.0 + derivatives.z, 0.25))
            + v_tangent * (derivatives.y / max(1.0 + derivatives.w, 0.25))) * w;
        crest += (1.0 - jacobian) * w;
    }
    return OceanFftSample(horizontal, height, slope, crest);
}

fn ocean_fft_camera_position_uniform() -> vec3<f32> {
    return ocean_fft_params.camera_position_pixel_angle.xyz;
}

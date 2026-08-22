const TRANSMITTANCE_SAMPLE_COUNT: u32 = 80u;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];
    // LUT texture coordinates start at the render target's top-left, while
    // clip-space Y points upward. Flip only generated data LUTs so v=0 keeps
    // meaning ground altitude when the LUT is sampled later.
    let uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), uv);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let altitude = input.uv.y * input.uv.y * OPTICAL_ATMOSPHERE_HEIGHT_METERS;
    let zenith_cosine = input.uv.x * 2.0 - 1.0;
    let position = vec3<f32>(
        0.0,
        OPTICAL_PLANET_RADIUS_METERS + max(altitude, 2.0),
        0.0,
    );
    let direction = vec3<f32>(
        sqrt(max(1.0 - zenith_cosine * zenith_cosine, 0.0)),
        zenith_cosine,
        0.0,
    );
    let ground_distance = nearest_positive_sphere_distance(
        position,
        direction,
        OPTICAL_PLANET_RADIUS_METERS,
    );
    let atmosphere_distance = optical_atmosphere_exit_distance(position, direction);
    if ground_distance > 0.0 && ground_distance < atmosphere_distance {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    var optical_depth = vec3<f32>(0.0);
    for (var index = 0u; index < TRANSMITTANCE_SAMPLE_COUNT; index += 1u) {
        let fraction = (f32(index) + 0.5) / f32(TRANSMITTANCE_SAMPLE_COUNT);
        let sample_position = position + direction * (fraction * atmosphere_distance);
        let sample_altitude = length(sample_position) - OPTICAL_PLANET_RADIUS_METERS;
        optical_depth += medium_extinction(sample_altitude)
            * (atmosphere_distance / f32(TRANSMITTANCE_SAMPLE_COUNT));
    }
    return vec4<f32>(exp(-optical_depth), 1.0);
}

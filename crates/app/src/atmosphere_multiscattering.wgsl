const MULTIPLE_SCATTER_DIRECTION_COUNT: u32 = 64u;
const MULTIPLE_SCATTER_SAMPLE_COUNT: u32 = 40u;

@group(0) @binding(0)
var transmittance_lut: texture_2d<f32>;
@group(0) @binding(1)
var atmosphere_sampler: sampler;

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
    // Match the transmittance LUT convention: v=0 is ground altitude.
    let uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), uv);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let altitude = input.uv.y * input.uv.y * OPTICAL_ATMOSPHERE_HEIGHT_METERS;
    let solar_zenith_cosine = input.uv.x * 2.0 - 1.0;
    let position = vec3<f32>(
        0.0,
        OPTICAL_PLANET_RADIUS_METERS + max(altitude, 2.0),
        0.0,
    );
    let sun_direction = vec3<f32>(
        sqrt(max(1.0 - solar_zenith_cosine * solar_zenith_cosine, 0.0)),
        solar_zenith_cosine,
        0.0,
    );

    var second_order_luminance = vec3<f32>(0.0);
    var scattering_feedback = vec3<f32>(0.0);
    for (var direction_index = 0u;
        direction_index < MULTIPLE_SCATTER_DIRECTION_COUNT;
        direction_index += 1u
    ) {
        let z = 1.0 - 2.0
            * (f32(direction_index) + 0.5)
            / f32(MULTIPLE_SCATTER_DIRECTION_COUNT);
        let phi = 2.0 * PI * fract(f32(direction_index) * 0.61803398875);
        let direction = vec3<f32>(
            sqrt(max(1.0 - z * z, 0.0)) * cos(phi),
            z,
            sqrt(max(1.0 - z * z, 0.0)) * sin(phi),
        );
        let atmosphere_distance = optical_atmosphere_exit_distance(position, direction);
        let ground_distance = nearest_positive_sphere_distance(
            position,
            direction,
            OPTICAL_PLANET_RADIUS_METERS,
        );
        let hits_ground = ground_distance > 0.0 && ground_distance < atmosphere_distance;
        let ray_length = select(atmosphere_distance, ground_distance, hits_ground);
        var view_transmittance = vec3<f32>(1.0);
        var ray_luminance = vec3<f32>(0.0);
        var ray_feedback = vec3<f32>(0.0);

        for (var sample_index = 0u;
            sample_index < MULTIPLE_SCATTER_SAMPLE_COUNT;
            sample_index += 1u
        ) {
            let fraction = (f32(sample_index) + 0.5) / f32(MULTIPLE_SCATTER_SAMPLE_COUNT);
            let segment_length = ray_length / f32(MULTIPLE_SCATTER_SAMPLE_COUNT);
            let sample_position = position + direction * (fraction * ray_length);
            let sample_radius = length(sample_position);
            let sample_direction = sample_position / sample_radius;
            let sample_altitude = sample_radius - OPTICAL_PLANET_RADIUS_METERS;
            let extinction = medium_extinction(sample_altitude);
            let scattering = medium_scattering(sample_altitude);
            let segment_transmittance = exp(-extinction * segment_length);
            let integrated_segment = (vec3<f32>(1.0) - segment_transmittance)
                / max(extinction, vec3<f32>(1.0e-9));
            let sun_transmittance = sample_transmittance_lut(
                transmittance_lut,
                atmosphere_sampler,
                sample_altitude,
                dot(sample_direction, sun_direction),
            );
            // The multiple-scattering approximation assumes the radiance
            // arriving from every sampled direction is redistributed by an
            // isotropic phase. Directional Rayleigh/Mie phase is applied only
            // by the final sky-view integration.
            let direct_source = sun_transmittance
                * scattering
                * (SOLAR_LUMINANCE / (4.0 * PI));
            ray_luminance += view_transmittance * direct_source * integrated_segment;
            ray_feedback += view_transmittance * scattering * integrated_segment;
            view_transmittance *= segment_transmittance;
        }

        if hits_ground {
            let ground_position = position + direction * ground_distance;
            let ground_direction = normalize(ground_position);
            let ground_sun_cosine = dot(ground_direction, sun_direction);
            let ground_sun = sample_transmittance_lut(
                transmittance_lut,
                atmosphere_sampler,
                2.0,
                ground_sun_cosine,
            ) * max(ground_sun_cosine, 0.0);
            ray_luminance += view_transmittance
                * GROUND_ALBEDO
                * ground_sun
                * (SOLAR_LUMINANCE / PI);
        }

        second_order_luminance += ray_luminance;
        scattering_feedback += ray_feedback;
    }

    let direction_count = f32(MULTIPLE_SCATTER_DIRECTION_COUNT);
    second_order_luminance /= direction_count;
    scattering_feedback /= direction_count;
    let infinite_scattering = second_order_luminance
        / max(vec3<f32>(1.0) - scattering_feedback, vec3<f32>(0.05));
    return vec4<f32>(max(infinite_scattering, vec3<f32>(0.0)), 1.0);
}

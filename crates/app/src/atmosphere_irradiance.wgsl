const IRRADIANCE_DIRECTION_COUNT: u32 = 16u;
const IRRADIANCE_SAMPLE_COUNT: u32 = 12u;

@group(0) @binding(0)
var transmittance_lut: texture_2d<f32>;
@group(0) @binding(1)
var multiple_scattering_lut: texture_2d<f32>;
@group(0) @binding(2)
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
    // This is a generated data LUT, so keep v=0 at ground altitude just like
    // the transmittance and multiple-scattering LUTs.
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

    // Integrate the upper hemisphere into Lambertian E/pi. Directions are
    // expressed in the sun/up frame, so the small fixed set cannot rotate or
    // shimmer as the planet turns beneath it.
    var diffuse_radiance = vec3<f32>(0.0);
    for (var direction_index = 0u;
        direction_index < IRRADIANCE_DIRECTION_COUNT;
        direction_index += 1u
    ) {
        let zenith_cosine = (f32(direction_index) + 0.5)
            / f32(IRRADIANCE_DIRECTION_COUNT);
        let azimuth = 2.0 * PI * fract(f32(direction_index) * 0.61803398875);
        let horizontal_length = sqrt(max(1.0 - zenith_cosine * zenith_cosine, 0.0));
        let ray = vec3<f32>(
            horizontal_length * cos(azimuth),
            zenith_cosine,
            horizontal_length * sin(azimuth),
        );
        let ray_length = optical_atmosphere_exit_distance(position, ray);
        var view_transmittance = vec3<f32>(1.0);
        var ray_luminance = vec3<f32>(0.0);

        for (var sample_index = 0u;
            sample_index < IRRADIANCE_SAMPLE_COUNT;
            sample_index += 1u
        ) {
            let fraction = (f32(sample_index) + 0.5) / f32(IRRADIANCE_SAMPLE_COUNT);
            let segment_length = ray_length / f32(IRRADIANCE_SAMPLE_COUNT);
            let sample_position = position + ray * (fraction * ray_length);
            let sample_radius = length(sample_position);
            let sample_direction = sample_position / sample_radius;
            let sample_altitude = sample_radius - OPTICAL_PLANET_RADIUS_METERS;
            let extinction = medium_extinction(sample_altitude);
            let scattering = medium_scattering(sample_altitude);
            let segment_transmittance = exp(-extinction * segment_length);
            let integrated_segment = (vec3<f32>(1.0) - segment_transmittance)
                / max(extinction, vec3<f32>(1.0e-9));
            let sample_solar_zenith_cosine = dot(sample_direction, sun_direction);
            let direct_sun = sample_transmittance_lut(
                transmittance_lut,
                atmosphere_sampler,
                sample_altitude,
                sample_solar_zenith_cosine,
            );
            let multiple_scattering = sample_multiple_scattering_lut(
                multiple_scattering_lut,
                atmosphere_sampler,
                sample_altitude,
                sample_solar_zenith_cosine,
            );
            let source = direct_sun
                    * phase_scattering(sample_altitude, dot(ray, sun_direction))
                    * SOLAR_LUMINANCE
                + multiple_scattering * scattering;
            ray_luminance += view_transmittance * source * integrated_segment;
            view_transmittance *= segment_transmittance;
        }

        // Uniform hemisphere sampling contributes 2 * L * cos(theta) / N to
        // E/pi, which is the diffuse-radiance factor expected by the surface
        // material shaders.
        diffuse_radiance += ray_luminance
            * (2.0 * zenith_cosine / f32(IRRADIANCE_DIRECTION_COUNT));
    }
    return vec4<f32>(max(diffuse_radiance, vec3<f32>(0.0)), 1.0);
}

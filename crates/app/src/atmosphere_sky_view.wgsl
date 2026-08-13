const SKY_VIEW_SAMPLE_COUNT: u32 = 20u;
// Keep the signed-off surface atmosphere byte-for-byte below the tallest game
// terrain. Above it, blend to world-space shell geometry so an orbital camera
// does not see the compressed optical atmosphere as a large solid bubble.
const ORBITAL_GEOMETRY_BLEND_START_METERS: f32 = 200000.0;
const ORBITAL_GEOMETRY_BLEND_END_METERS: f32 = 400000.0;
// Linear cosine rows collapse the complete orbital atmosphere into less than
// one LUT row at long range. Keep fixed row bands for the atmosphere and the
// solid planet once the camera is above all authored terrain.
const ORBITAL_ATMOSPHERE_LUT_V: f32 = 0.72;
const ORBITAL_GROUND_LUT_V: f32 = 0.88;

struct Camera {
    projection_matrix: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_view_altitude: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_direction_view: vec4<f32>,
    projection: vec4<f32>,
    flat_triangle_options: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;
@group(1) @binding(0)
var transmittance_lut: texture_2d<f32>;
@group(1) @binding(1)
var multiple_scattering_lut: texture_2d<f32>;
@group(1) @binding(2)
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
    // Generated LUT textures start at the render target's top-left, while
    // clip-space Y points upward. Keep v=0 at the sky zenith when sampled.
    let uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), uv);
}

fn sky_basis(up: vec3<f32>, sun: vec3<f32>) -> mat3x3<f32> {
    var toward_sun = sun - up * dot(up, sun);
    if dot(toward_sun, toward_sun) < 1.0e-6 {
        toward_sun = normalize(camera.camera_right.xyz);
    } else {
        toward_sun = normalize(toward_sun);
    }
    let side = normalize(cross(up, toward_sun));
    return mat3x3<f32>(toward_sun, side, up);
}

fn ground_horizon_cosine(camera_radius: f32) -> f32 {
    return sphere_horizon_cosine(camera_radius, PLANET_RADIUS_METERS);
}

fn sphere_horizon_cosine(camera_radius: f32, sphere_radius: f32) -> f32 {
    let radius_ratio = clamp(
        sphere_radius / max(camera_radius, sphere_radius),
        0.0,
        1.0,
    );
    return -sqrt(max(1.0 - radius_ratio * radius_ratio, 0.0));
}

fn sky_view_mapping_rows(camera_radius: f32, camera_altitude: f32) -> vec4<f32> {
    let atmosphere_horizon = sphere_horizon_cosine(
        camera_radius,
        PLANET_RADIUS_METERS + OPTICAL_ATMOSPHERE_HEIGHT_METERS,
    );
    let ground_horizon = ground_horizon_cosine(camera_radius);
    let orbital_amount = smoothstep(
        ORBITAL_GEOMETRY_BLEND_START_METERS,
        ORBITAL_GEOMETRY_BLEND_END_METERS,
        camera_altitude,
    );
    let atmosphere_v = mix(
        (1.0 - atmosphere_horizon) * 0.5,
        ORBITAL_ATMOSPHERE_LUT_V,
        orbital_amount,
    );
    let ground_v = mix(
        (1.0 - ground_horizon) * 0.5,
        ORBITAL_GROUND_LUT_V,
        orbital_amount,
    );
    return vec4<f32>(atmosphere_horizon, ground_horizon, atmosphere_v, ground_v);
}

fn sky_view_zenith_cosine_from_v(
    v: f32,
    camera_radius: f32,
    camera_altitude: f32,
) -> f32 {
    let mapping = sky_view_mapping_rows(camera_radius, camera_altitude);
    if v <= mapping.z {
        return mix(1.0, mapping.x, v / max(mapping.z, 1.0e-6));
    }
    if v <= mapping.w {
        return mix(
            mapping.x,
            mapping.y,
            (v - mapping.z) / max(mapping.w - mapping.z, 1.0e-6),
        );
    }
    return mix(
        mapping.y,
        -1.0,
        (v - mapping.w) / max(1.0 - mapping.w, 1.0e-6),
    );
}

fn optical_zenith_cosine(world_cosine: f32, world_radius: f32, optical_radius: f32) -> f32 {
    let world_horizon = ground_horizon_cosine(world_radius);
    let optical_horizon = ground_horizon_cosine(optical_radius);
    if world_cosine >= world_horizon {
        let amount = (world_cosine - world_horizon) / max(1.0 - world_horizon, 1.0e-6);
        return mix(optical_horizon, 1.0, amount);
    }
    let amount = (world_cosine + 1.0) / max(world_horizon + 1.0, 1.0e-6);
    return mix(-1.0, optical_horizon, amount);
}

fn direction_with_zenith_cosine(
    direction: vec3<f32>,
    up: vec3<f32>,
    zenith_cosine: f32,
) -> vec3<f32> {
    var horizontal = direction - up * dot(direction, up);
    if dot(horizontal, horizontal) < 1.0e-8 {
        horizontal = camera.camera_right.xyz
            - up * dot(camera.camera_right.xyz, up);
    }
    let horizontal_direction = normalize(horizontal);
    let horizontal_length = sqrt(max(1.0 - zenith_cosine * zenith_cosine, 0.0));
    return horizontal_direction * horizontal_length + up * zenith_cosine;
}

fn integrate_world_space_sky(
    camera_position: vec3<f32>,
    ray: vec3<f32>,
    sun: vec3<f32>,
) -> vec3<f32> {
    let atmosphere_interval = sphere_interval(
        camera_position,
        ray,
        PLANET_RADIUS_METERS + OPTICAL_ATMOSPHERE_HEIGHT_METERS,
    );
    let start_distance = max(atmosphere_interval.x, 0.0);
    var end_distance = atmosphere_interval.y;
    let ground_distance = nearest_positive_sphere_distance(
        camera_position,
        ray,
        PLANET_RADIUS_METERS,
    );
    if ground_distance > start_distance && ground_distance < end_distance {
        end_distance = ground_distance;
    }
    if end_distance <= start_distance {
        return vec3<f32>(0.0);
    }

    let ray_length = end_distance - start_distance;
    var view_transmittance = vec3<f32>(1.0);
    var luminance = vec3<f32>(0.0);
    for (var index = 0u; index < SKY_VIEW_SAMPLE_COUNT; index += 1u) {
        let fraction = (f32(index) + 0.5) / f32(SKY_VIEW_SAMPLE_COUNT);
        let shaped_fraction = fraction * fraction * (3.0 - 2.0 * fraction);
        let previous_fraction = f32(index) / f32(SKY_VIEW_SAMPLE_COUNT);
        let previous_shaped = previous_fraction * previous_fraction
            * (3.0 - 2.0 * previous_fraction);
        let next_fraction = f32(index + 1u) / f32(SKY_VIEW_SAMPLE_COUNT);
        let next_shaped = next_fraction * next_fraction
            * (3.0 - 2.0 * next_fraction);
        let segment_length = (next_shaped - previous_shaped) * ray_length;
        let sample_position = camera_position
            + ray * (start_distance + shaped_fraction * ray_length);
        let sample_radius = length(sample_position);
        let sample_direction = sample_position / sample_radius;
        let sample_altitude = sample_radius - PLANET_RADIUS_METERS;
        let extinction = medium_extinction(sample_altitude);
        let scattering = medium_scattering(sample_altitude);
        let segment_transmittance = exp(-extinction * segment_length);
        let integrated_segment = (vec3<f32>(1.0) - segment_transmittance)
            / max(extinction, vec3<f32>(1.0e-9));
        let solar_zenith_cosine = dot(sample_direction, sun);
        let direct_sun = sample_transmittance_lut(
            transmittance_lut,
            atmosphere_sampler,
            sample_altitude,
            solar_zenith_cosine,
        );
        let multiple_scattering = sample_multiple_scattering_lut(
            multiple_scattering_lut,
            atmosphere_sampler,
            sample_altitude,
            solar_zenith_cosine,
        );
        let source = direct_sun
                * phase_scattering(sample_altitude, dot(ray, sun))
                * SOLAR_LUMINANCE
            + multiple_scattering * scattering;
        luminance += view_transmittance * source * integrated_segment;
        view_transmittance *= segment_transmittance;
    }
    return max(luminance, vec3<f32>(0.0));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let up = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let sun = normalize(camera.sun_direction_view.xyz);
    let world_camera_radius = PLANET_RADIUS_METERS
        + camera.camera_planet_direction_view_altitude.w;
    let optical_camera_altitude =
        camera.camera_planet_direction_view_altitude.w / ATMOSPHERE_VERTICAL_SCALE;
    let camera_radius = OPTICAL_PLANET_RADIUS_METERS + optical_camera_altitude;
    let optical_sun = direction_with_zenith_cosine(
        sun,
        up,
        optical_zenith_cosine(dot(sun, up), world_camera_radius, camera_radius),
    );
    let world_basis = sky_basis(up, sun);
    let optical_basis = sky_basis(up, optical_sun);
    let world_view_zenith_cosine = sky_view_zenith_cosine_from_v(
        input.uv.y,
        world_camera_radius,
        camera.camera_planet_direction_view_altitude.w,
    );
    let optical_view_zenith_cosine = optical_zenith_cosine(
        world_view_zenith_cosine,
        world_camera_radius,
        camera_radius,
    );
    let azimuth = (input.uv.x * 2.0 - 1.0) * PI;
    let world_horizontal_length = sqrt(max(
        1.0 - world_view_zenith_cosine * world_view_zenith_cosine,
        0.0,
    ));
    let world_ray = world_basis * vec3<f32>(
        cos(azimuth) * world_horizontal_length,
        sin(azimuth) * world_horizontal_length,
        world_view_zenith_cosine,
    );
    var world_space_luminance = vec3<f32>(0.0);
    if camera.camera_planet_direction_view_altitude.w
        > ORBITAL_GEOMETRY_BLEND_START_METERS
    {
        world_space_luminance = integrate_world_space_sky(
            up * world_camera_radius,
            world_ray,
            sun,
        );
        if camera.camera_planet_direction_view_altitude.w
            >= ORBITAL_GEOMETRY_BLEND_END_METERS
        {
            return vec4<f32>(world_space_luminance, 1.0);
        }
    }
    let optical_horizontal_length = sqrt(max(
        1.0 - optical_view_zenith_cosine * optical_view_zenith_cosine,
        0.0,
    ));
    let ray = optical_basis * vec3<f32>(
        cos(azimuth) * optical_horizontal_length,
        sin(azimuth) * optical_horizontal_length,
        optical_view_zenith_cosine,
    );

    // The optical profile deliberately compresses the unusually thick game
    // atmosphere, but it must not make that shell occupy a different part of
    // the sky. Reject rays that miss the actual world-space shell first.
    let world_atmosphere_interval = sphere_interval(
        up * world_camera_radius,
        world_ray,
        PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS,
    );
    if world_atmosphere_interval.y <= max(world_atmosphere_interval.x, 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let camera_position = up * camera_radius;
    let atmosphere_interval = sphere_interval(
        camera_position,
        ray,
        OPTICAL_ATMOSPHERE_RADIUS_METERS,
    );
    let start_distance = max(atmosphere_interval.x, 0.0);
    var end_distance = atmosphere_interval.y;
    let ground_distance = nearest_positive_sphere_distance(
        camera_position,
        ray,
        OPTICAL_PLANET_RADIUS_METERS,
    );
    if ground_distance > start_distance && ground_distance < end_distance {
        end_distance = ground_distance;
    }
    if end_distance <= start_distance {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let ray_length = end_distance - start_distance;
    var view_transmittance = vec3<f32>(1.0);
    var luminance = vec3<f32>(0.0);
    for (var index = 0u; index < SKY_VIEW_SAMPLE_COUNT; index += 1u) {
        let fraction = (f32(index) + 0.5) / f32(SKY_VIEW_SAMPLE_COUNT);
        let shaped_fraction = fraction * fraction * (3.0 - 2.0 * fraction);
        let previous_fraction = f32(index) / f32(SKY_VIEW_SAMPLE_COUNT);
        let previous_shaped = previous_fraction * previous_fraction
            * (3.0 - 2.0 * previous_fraction);
        let next_fraction = f32(index + 1u) / f32(SKY_VIEW_SAMPLE_COUNT);
        let next_shaped = next_fraction * next_fraction
            * (3.0 - 2.0 * next_fraction);
        let segment_length = (next_shaped - previous_shaped) * ray_length;
        let sample_position = camera_position
            + ray * (start_distance + shaped_fraction * ray_length);
        let sample_radius = length(sample_position);
        let sample_direction = sample_position / sample_radius;
        let sample_altitude = sample_radius - OPTICAL_PLANET_RADIUS_METERS;
        let extinction = medium_extinction(sample_altitude);
        let scattering = medium_scattering(sample_altitude);
        let segment_transmittance = exp(-extinction * segment_length);
        let integrated_segment = (vec3<f32>(1.0) - segment_transmittance)
            / max(extinction, vec3<f32>(1.0e-9));
        let solar_zenith_cosine = dot(sample_direction, optical_sun);
        let direct_sun = sample_transmittance_lut(
            transmittance_lut,
            atmosphere_sampler,
            sample_altitude,
            solar_zenith_cosine,
        );
        let multiple_scattering = sample_multiple_scattering_lut(
            multiple_scattering_lut,
            atmosphere_sampler,
            sample_altitude,
            solar_zenith_cosine,
        );
        let source = direct_sun
                * phase_scattering(sample_altitude, dot(ray, optical_sun))
                * SOLAR_LUMINANCE
            + multiple_scattering * scattering;
        luminance += view_transmittance * source * integrated_segment;
        view_transmittance *= segment_transmittance;
    }
    let optical_luminance = max(luminance, vec3<f32>(0.0));
    let orbital_blend = smoothstep(
        ORBITAL_GEOMETRY_BLEND_START_METERS,
        ORBITAL_GEOMETRY_BLEND_END_METERS,
        camera.camera_planet_direction_view_altitude.w,
    );
    return vec4<f32>(mix(optical_luminance, world_space_luminance, orbital_blend), 1.0);
}

// Composite an airless body's physical radiance through the planet atmosphere.
// Both body renderers use the same camera-relative reversed-Z near plane.
@group(2) @binding(0) var moon_colour: texture_2d<f32>;
@group(2) @binding(1) var moon_depth: texture_depth_2d;
@group(2) @binding(2) var system_transmittance: texture_2d<f32>;
@group(2) @binding(3) var system_sampler: sampler;

fn system_column(altitude: f32, mu: f32) -> vec3<f32> {
    return textureSampleLevel(system_transmittance, system_sampler, vec2<f32>(
        clamp(mu * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(altitude / 2880000.0, 0.0, 1.0)),
    ), 0.0).rgb;
}
fn system_extinction(ray: vec3<f32>) -> vec3<f32> {
    let altitude = max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let radius = PLANET_RADIUS_METERS + altitude;
    let mu = dot(normalize(camera.camera_planet_direction_view_altitude.xyz), ray);
    let closest_radius = radius * sqrt(max(1.0 - mu * mu, 0.0));
    if mu >= 0.0 { return system_column(altitude, mu); }
    let tangent_column = system_column(closest_radius - PLANET_RADIUS_METERS, 0.0);
    let outgoing_column = system_column(altitude, -mu);
    return clamp(tangent_column * tangent_column / max(outgoing_column, vec3<f32>(1.0e-6)), vec3<f32>(0.0), vec3<f32>(1.0));
}
struct SystemOutput {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
@fragment
fn fs_system(input: VertexOutput) -> SystemOutput {
    let pixel = vec2<i32>(input.position.xy);
    let depth = textureLoad(moon_depth, pixel, 0);
    if depth == 0.0 { discard; }
    let ray = view_direction(input.ndc);
    let radiance = textureLoad(moon_colour, pixel, 0).rgb;
    var output: SystemOutput;
    // In-scatter behind an opaque foreground body must NOT be added over it.
    // From the moon, even a ray aimed at the planet hits nearby regolith before
    // it ever reaches the planet's atmosphere. Depth alone cannot fix this:
    // the background LUT already contains that distant atmosphere's radiance.
    let camera_radius = PLANET_RADIUS_METERS + max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let origin = normalize(camera.camera_planet_direction_view_altitude.xyz) * camera_radius;
    let outer_radius = PLANET_RADIUS_METERS + 2880000.0;
    let b = dot(origin, ray);
    let discriminant = b * b - dot(origin, origin) + outer_radius * outer_radius;
    var crosses_air = camera_radius < outer_radius;
    if discriminant > 0.0 && b < 0.0 {
        let entry_distance = max(-b - sqrt(discriminant), 0.0);
        let surface_distance = camera.projection_matrix[3][2] / (depth * max(-ray.z, 1e-6));
        crosses_air = crosses_air || surface_distance > entry_distance;
    }
    output.colour = vec4<f32>(radiance, 1.0);
    if crosses_air {
        output.colour = vec4<f32>(displayed_sky_radiance(ray) + radiance * system_extinction(ray), 1.0);
    }
    output.depth = depth;
    return output;
}

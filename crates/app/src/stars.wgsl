// Assembled after atmosphere.wgsl: reuse its camera and exact displayed-sky
// luminance, including the perceptual twilight lift and horizon mist.
@group(2) @binding(0) var<uniform> stellar_settings: vec4<f32>;
@group(2) @binding(1) var stellar_transmittance: texture_2d<f32>;
@group(2) @binding(2) var stellar_sampler: sampler;

struct WeatherRenderUniform {
    blend: f32,
    drift_radians: f32,
    lower_shell_radius_meters: f32,
    upper_shell_radius_meters: f32,
    noise_scale: f32,
    noise_strength: f32,
    _padding: vec2<f32>,
}
@group(3) @binding(0) var cloud_field_current: texture_cube<f32>;
@group(3) @binding(1) var cloud_field_previous: texture_cube<f32>;
@group(3) @binding(2) var cloud_field_sampler: sampler;
@group(3) @binding(3) var<uniform> weather: WeatherRenderUniform;

struct StarInput {
    @location(0) direction_flux: vec4<f32>,
    @location(1) colour_radius: vec4<f32>,
    @location(2) shape: vec4<f32>,
}
struct StarOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) centre_pixel: vec2<f32>,
    @location(2) @interpolate(flat) emission: vec3<f32>,
    @location(3) @interpolate(flat) sky_luminance: f32,
    @location(4) @interpolate(flat) kind: f32,
    @location(5) celestial_direction: vec3<f32>,
}

fn stellar_view(direction: vec3<f32>) -> vec3<f32> {
    // Catalogues are inertial. Counter-rotate with the planet exactly as the
    // sun does; never seed stars from camera position or scroll them in UV.
    let s = stellar_settings.z;
    let c = stellar_settings.w;
    let local = vec3<f32>(c * direction.x - s * direction.z, direction.y, s * direction.x + c * direction.z);
    return vec3<f32>(dot(local, camera.camera_right.xyz), dot(local, camera.camera_up.xyz), -dot(local, camera.camera_forward.xyz));
}

fn stellar_view_to_planet(direction: vec3<f32>) -> vec3<f32> {
    return camera.camera_right.xyz * direction.x + camera.camera_up.xyz * direction.y - camera.camera_forward.xyz * direction.z;
}

fn stellar_column(altitude: f32, mu: f32) -> vec3<f32> {
    return textureSampleLevel(stellar_transmittance, stellar_sampler, vec2<f32>(
        clamp(mu * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(altitude / 2880000.0, 0.0, 1.0)),
    ), 0.0).rgb;
}

fn stellar_extinction(ray: vec3<f32>) -> vec3<f32> {
    let altitude = max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let radius = PLANET_RADIUS_METERS + altitude;
    let mu = dot(normalize(camera.camera_planet_direction_view_altitude.xyz), ray);
    let closest_radius = radius * sqrt(max(1.0 - mu * mu, 0.0));
    if mu < 0.0 && closest_radius <= PLANET_RADIUS_METERS {
        return vec3<f32>(0.0);
    }
    if mu >= 0.0 {
        return stellar_column(altitude, mu);
    }
    // A space observer looking through the limb traverses both halves of the
    // tangent column. Do not let a thin local camera density reveal stars
    // through dense air below it, or a night-side planet become transparent.
    let tangent_column = stellar_column(closest_radius - PLANET_RADIUS_METERS, 0.0);
    let outgoing_column = stellar_column(altitude, -mu);
    return clamp(tangent_column * tangent_column / max(outgoing_column, vec3<f32>(1.0e-6)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn stellar_cloud_transmission(ray: vec3<f32>) -> f32 {
    let camera_position = normalize(camera.camera_planet_direction_view_altitude.xyz)
        * (PLANET_RADIUS_METERS + max(camera.camera_planet_direction_view_altitude.w, 0.0));
    let b = dot(camera_position, ray);
    let discriminant = b * b - dot(camera_position, camera_position)
        + weather.lower_shell_radius_meters * weather.lower_shell_radius_meters;
    if discriminant <= 0.0 { return 1.0; }
    let root = sqrt(discriminant);
    var distance = -b - root;
    if distance <= 0.0 { distance = -b + root; }
    if distance <= 0.0 { return 1.0; }
    let direction = normalize(stellar_view_to_planet(camera_position + ray * distance));
    let density = cloudDensityWithOctaves(direction, 0.0, 3u);
    // Optical obscuration, not an emissive light that shines through a dark
    // storm. The real cloud/impostor passes additionally composite afterward.
    return pow(1.0 - smoothstep(0.05, 0.60, density), 4.0);
}

@vertex
fn vs_stars(input: StarInput, @builtin(vertex_index) vertex: u32) -> StarOutput {
    var corners = array<vec2<f32>, 6>(vec2<f32>(-1.0,-1.0), vec2<f32>(1.0,-1.0), vec2<f32>(1.0,1.0), vec2<f32>(-1.0,-1.0), vec2<f32>(1.0,1.0), vec2<f32>(-1.0,1.0));
    let corner = corners[vertex];
    let direction = input.direction_flux.xyz;
    let ray = stellar_view(direction);
    var output: StarOutput;
    output.position = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    if ray.z >= -0.001 || camera.flat_triangle_options.w > 0.5 { return output; }
    let projection_scale = vec2<f32>(camera.projection.x * camera.projection.y, camera.projection.y);
    let centre = ray.xy / (-ray.z * projection_scale);
    let margin = 2.0 * input.colour_radius.w / max(-ray.z * projection_scale.y, 0.001) + 0.01;
    if any(abs(centre) > vec2<f32>(1.0 + margin)) { return output; }
    let sky = dot(displayed_sky_radiance(normalize(ray)), vec3<f32>(0.2126, 0.7152, 0.0722));
    var emission = input.colour_radius.rgb * input.direction_flux.w * stellar_extinction(normalize(ray));
    if dot(emission, vec3<f32>(0.2126, 0.7152, 0.0722)) <= sky { return output; }
    emission *= stellar_cloud_transmission(normalize(ray));
    var projected = centre + corner * 3.0 / stellar_settings.xy;
    var celestial_direction = direction;
    if input.shape.z > 0.5 {
        let reference = select(vec3<f32>(0.0,1.0,0.0), vec3<f32>(1.0,0.0,0.0), abs(direction.y) > 0.9);
        let tangent = normalize(cross(direction, reference));
        let bitangent = cross(direction, tangent);
        let c = cos(input.shape.y);
        let s = sin(input.shape.y);
        let axis = tangent * c + bitangent * s;
        let other = bitangent * c - tangent * s;
        celestial_direction = normalize(direction + input.colour_radius.w * (axis * corner.x + other * corner.y * input.shape.x));
        let extended_ray = stellar_view(celestial_direction);
        projected = extended_ray.xy / (-extended_ray.z * projection_scale);
    }
    // Reversed-Z infinity: only empty scene depth passes; never write depth.
    output.position = vec4<f32>(projected, 0.0, 1.0);
    output.uv = corner;
    output.centre_pixel = (centre * vec2<f32>(0.5, -0.5) + 0.5) * stellar_settings.xy;
    output.emission = emission;
    output.sky_luminance = sky;
    output.kind = input.shape.z;
    output.celestial_direction = celestial_direction;
    return output;
}

fn stellar_pixel_filter(distance: f32) -> f32 {
    let d = abs(distance);
    if d < 0.5 { return 0.75 - d * d; }
    return 0.5 * pow(max(1.5 - d, 0.0), 2.0);
}

@fragment
fn fs_stars(input: StarOutput) -> @location(0) vec4<f32> {
    let delta = input.position.xy - input.centre_pixel;
    var profile = stellar_pixel_filter(delta.x) * stellar_pixel_filter(delta.y);
    if input.kind > 0.5 {
        let r2 = dot(input.uv, input.uv);
        let direction = normalize(input.celestial_direction);
        // Stable celestial-space wisps, continuous across individual objects.
        let structure = 0.65 + 0.35 * sin(dot(direction, vec3<f32>(173.0, 241.0, -137.0)))
            * sin(dot(direction, vec3<f32>(-97.0, 113.0, 193.0)));
        profile = exp(-4.0 * r2) * (1.0 - smoothstep(0.55, 1.0, r2)) * structure;
    }
    let radiance = input.emission * profile;
    let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    // Add only the excess over the actual sky. For one object the resulting
    // luminance is max(sky, star), retaining the star's colour rather than
    // taking three unrelated RGB maxima. Dim stars emerge last, naturally.
    let visible = max(luminance - input.sky_luminance, 0.0) / max(luminance, 1.0e-8);
    return vec4<f32>(radiance * visible, 1.0);
}

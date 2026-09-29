
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
    lightning: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;

struct ForestUniform {
    camera_planet_position: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> forest: ForestUniform;

@group(1) @binding(1)
var sky_view_lut: texture_2d<f32>;

@group(1) @binding(2)
var sky_view_sampler: sampler;

@group(2) @binding(0)
var cloud_field_current: texture_cube<f32>;

@group(2) @binding(1)
var cloud_field_previous: texture_cube<f32>;

@group(2) @binding(2)
var cloud_field_sampler: sampler;

struct WeatherRenderUniform {
    blend: f32,
    drift_radians: f32,
    lower_shell_radius_meters: f32,
    upper_shell_radius_meters: f32,
    noise_scale: f32,
    noise_strength: f32,
    _padding: vec2<f32>,
}

@group(2) @binding(3)
var<uniform> weather: WeatherRenderUniform;

struct VertexInput {
    @location(0) centre_and_height: vec4<f32>,
    @location(1) width_shade_kind_seed: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) colour_and_kind: vec4<f32>,
    @location(2) @interpolate(flat) seed: f32,
    @location(3) @interpolate(flat) lighting: f32,
    @location(4) @interpolate(flat) valid: f32,
    @location(5) @interpolate(flat, first) fog: vec4<f32>,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

// Match the terrain fog's horizon sky lookup and 100m full-storm air path.
// This runs once per billboard vertex, not for every covered fragment.
fn tree_storm_fog(centre: vec3<f32>, view_position: vec3<f32>) -> vec4<f32> {
    let overcast = clamp(camera.sun_direction.w, 0.0, 1.0);
    if overcast <= 0.0 { return vec4<f32>(0.0); }
    let distance = length(view_position);
    if distance <= 1.0e-3 { return vec4<f32>(0.0); }
    let camera_altitude = max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let tree_altitude = max(length(centre) - PLANET_RADIUS_METERS, 0.0);
    let mean_density = 0.5 * (exp(-camera_altitude / 122000.0)
        + exp(-tree_altitude / 122000.0));
    let e_fold = exp(mix(log(500000.0), log(100.0 / 4.6051702), overcast));
    let amount = 1.0 - exp(-distance * mean_density / e_fold);
    if amount <= 1.0e-4 { return vec4<f32>(0.0); }

    let up = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let sun = normalize(camera.sun_direction_view.xyz);
    var toward_sun = sun - up * dot(up, sun);
    if dot(toward_sun, toward_sun) < 1.0e-6 {
        toward_sun = normalize(camera.camera_right.xyz);
    } else {
        toward_sun = normalize(toward_sun);
    }
    let ray = normalize(view_position);
    let horizontal = ray - up * dot(ray, up);
    var azimuth = 0.0;
    if dot(horizontal, horizontal) > 1.0e-8 {
        let direction = normalize(horizontal);
        azimuth = atan2(dot(direction, cross(up, toward_sun)), dot(direction, toward_sun));
    }
    let radius = PLANET_RADIUS_METERS + max(camera_altitude, 200.0);
    let horizon_cosine = -sqrt(max(1.0 - pow(PLANET_RADIUS_METERS / radius, 2.0), 0.0));
    let uv = vec2<f32>(fract(azimuth / (2.0 * 3.141592653589793) + 0.5),
        0.5 * (1.0 - horizon_cosine));
    let radiance = textureSampleLevel(sky_view_lut, sky_view_sampler, uv, 0.0).rgb;
    let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    let perceived = 0.22 * pow(luminance, 0.42);
    let gain = clamp(perceived / max(luminance, 1.0e-8), 0.35, 80.0);
    let sky = radiance * gain;
    let grey = dot(sky, vec3<f32>(0.2126, 0.7152, 0.0722)) * 0.45;
    var lightning_glow = vec3<f32>(0.0);
    if camera.lightning.w > 0.0 {
        lightning_glow = vec3<f32>(0.72, 0.78, 0.9) * camera.lightning.w
            * smoothstep(0.35, 0.9, dot(ray, normalize(camera.lightning.xyz)));
    }
    return vec4<f32>(mix(sky, vec3<f32>(grey), overcast) + lightning_glow, amount);
}

fn cloud_shadow_density_at_shell(
    surface_position: vec3<f32>,
    sun_direction: vec3<f32>,
    shell_radius: f32,
    shell_index: f32,
) -> f32 {
    let surface_radius_squared = dot(surface_position, surface_position);
    if surface_radius_squared >= shell_radius * shell_radius {
        return 0.0;
    }
    let ray_offset = dot(surface_position, sun_direction);
    let discriminant = ray_offset * ray_offset
        - (surface_radius_squared - shell_radius * shell_radius);
    if discriminant <= 0.0 {
        return 0.0;
    }
    let distance = -ray_offset + sqrt(discriminant);
    if distance <= 0.0 {
        return 0.0;
    }
    let shadow_position = surface_position + sun_direction * distance;
    return cloudDensityWithOctaves(normalize(shadow_position), shell_index, 3u);
}

fn cloud_shadow_visibility(
    surface_direction: vec3<f32>,
    surface_height: f32,
    sun_direction: vec3<f32>,
) -> f32 {
    let surface_position = normalize(surface_direction)
        * (PLANET_RADIUS_METERS + max(surface_height, 0.0));
    let lower_density = cloud_shadow_density_at_shell(
        surface_position,
        sun_direction,
        weather.lower_shell_radius_meters,
        0.0,
    );
    let upper_density = cloud_shadow_density_at_shell(
        surface_position,
        sun_direction,
        weather.upper_shell_radius_meters,
        1.0,
    );
    let combined_density = 1.0
        - (1.0 - clamp(lower_density, 0.0, 1.0))
            * (1.0 - clamp(upper_density, 0.0, 1.0));
    let posterized_density = floor(combined_density * 4.0 + 0.5) / 4.0;
    return 1.0 - posterized_density * 0.88;
}

fn tree_lighting(solar_elevation_cosine: f32, cloud_visibility: f32) -> f32 {
    let direct = max(solar_elevation_cosine, 0.0) * 1.24;
    let sky_ambient = smoothstep(-0.18, 0.02, solar_elevation_cosine) * 0.36;
    return direct * cloud_visibility + sky_ambient;
}

@vertex
fn vs_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let height = input.centre_and_height.w;
    if height <= 0.0 {
        return VertexOutput(
            vec4<f32>(2.0, 2.0, 0.0, 1.0),
            vec2<f32>(0.0),
            vec4<f32>(0.0),
            0.0,
            0.0,
            0.0,
            vec4<f32>(0.0),
        );
    }
    // One oversized triangle covers the same unit billboard rectangle as two
    // triangles. The fragment silhouettes already discard outside the tree,
    // so this halves transform and lighting work without changing its shape.
    let corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(3.0, 0.0), vec2<f32>(-1.0, 2.0),
    );
    let corner = corners[vertex_index];
    let centre = input.centre_and_height.xyz;
    let width = input.width_shade_kind_seed.x;
    let shade = input.width_shade_kind_seed.y;
    let kind = input.width_shade_kind_seed.z;
    let species_kind = kind - floor(kind * 0.5) * 2.0;
    let up = normalize(centre);
    let to_camera = forest.camera_planet_position.xyz - centre;
    var right = cross(up, to_camera);
    if dot(right, right) < 1.0e-4 {
        right = cross(up, camera.camera_forward.xyz);
    }
    right = normalize(right);
    let world_position = centre
        + right * corner.x * width * 0.5
        + up * corner.y * height;
    let view_position = planet_to_view(world_position - forest.camera_planet_position.xyz);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_elevation_cosine = dot(up, sun_direction);
    var cloud_visibility = 1.0;
    if TERRAIN_CLOUD_SHADOW_ENABLED && solar_elevation_cosine > 0.0 {
        cloud_visibility = cloud_shadow_visibility(
            up,
            length(centre) - PLANET_RADIUS_METERS,
            sun_direction,
        );
    }
    let lighting = tree_lighting(solar_elevation_cosine, cloud_visibility) * shade;
    let broadleaf = srgb_to_linear(vec3<f32>(0.05, 0.17, 0.06));
    let conifer = srgb_to_linear(vec3<f32>(0.035, 0.125, 0.05));
    let colour = mix(broadleaf, conifer, species_kind) * lighting;
    var fog = vec4<f32>(0.0);
    // A billboard is one triangle. Flat interpolation takes its first vertex,
    // so the other two must not repeat the sky lookup and fog calculation.
    if vertex_index == 0u {
        fog = tree_storm_fog(centre, planet_to_view(centre - forest.camera_planet_position.xyz));
    }
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(view_position, 1.0),
        vec2<f32>(corner.x * 0.5 + 0.5, corner.y),
        vec4<f32>(colour, kind),
        input.width_shade_kind_seed.w,
        lighting,
        1.0,
        fog,
    );
}

fn circle(point: vec2<f32>, centre: vec2<f32>, radius: vec2<f32>) -> bool {
    let local = (point - centre) / radius;
    return dot(local, local) <= 1.0;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.valid < 0.5 {
        discard;
    }
    let point = vec2<f32>(input.uv.x * 2.0 - 1.0, input.uv.y);
    let trunk_half_width = 0.075 + input.seed * 0.025;
    let proxy = input.colour_and_kind.w >= 2.0;
    let species_kind = input.colour_and_kind.w
        - floor(input.colour_and_kind.w * 0.5) * 2.0;
    var trunk = abs(point.x) < trunk_half_width && point.y < 0.38;
    var canopy = false;
    if proxy {
        trunk = point.y < 0.34
            && (abs(point.x + 0.58) < trunk_half_width * 0.7
                || abs(point.x + 0.18) < trunk_half_width * 0.7
                || abs(point.x - 0.24) < trunk_half_width * 0.7
                || abs(point.x - 0.61) < trunk_half_width * 0.7);
        if species_kind < 0.5 {
            canopy = circle(point, vec2<f32>(-0.58, 0.50), vec2<f32>(0.34, 0.26))
                || circle(point, vec2<f32>(-0.18, 0.67), vec2<f32>(0.40, 0.32))
                || circle(point, vec2<f32>(0.25, 0.57), vec2<f32>(0.38, 0.29))
                || circle(point, vec2<f32>(0.62, 0.72), vec2<f32>(0.32, 0.27));
        } else {
            let left = (1.0 - point.y) * 0.34;
            canopy = point.y > 0.16 && point.y < 0.96
                && (abs(point.x + 0.60) < left
                    || abs(point.x + 0.20) < left * 1.1
                    || abs(point.x - 0.24) < left
                    || abs(point.x - 0.62) < left * 0.9);
        }
    } else if species_kind < 0.5 {
        canopy = circle(point, vec2<f32>(-0.20, 0.60), vec2<f32>(0.43, 0.31))
            || circle(point, vec2<f32>(0.20, 0.61), vec2<f32>(0.45, 0.33))
            || circle(point, vec2<f32>(0.0, 0.79), vec2<f32>(0.48, 0.30));
    } else {
        let crown_width = (1.0 - point.y) * 0.82
            + 0.08 * sin(point.y * 43.0 + input.seed * 19.0);
        canopy = point.y > 0.17 && point.y < 0.98 && abs(point.x) < crown_width;
    }
    if !trunk && !canopy {
        discard;
    }
    let trunk_colour = srgb_to_linear(vec3<f32>(0.22, 0.12, 0.055));
    let colour = select(trunk_colour * 0.75 * input.lighting, input.colour_and_kind.rgb, canopy);
    return vec4<f32>(mix(colour, input.fog.rgb, input.fog.w), 1.0);
}

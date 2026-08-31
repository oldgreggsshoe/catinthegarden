const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_VERTICAL_SCALE: f32 = 4.5;
const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 640000.0;
const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 8000.0;
const MIE_SCALE_HEIGHT_METERS: f32 = 1200.0;
const RAYLEIGH_COEFFICIENT: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFFICIENT: vec3<f32> = vec3<f32>(4.4e-6);

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
var cloud_field_current: texture_cube<f32>;
@group(1) @binding(1)
var cloud_field_previous: texture_cube<f32>;
@group(1) @binding(2)
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

@group(1) @binding(3)
var<uniform> weather: WeatherRenderUniform;

@group(1) @binding(4)
var weather_surface_current: texture_cube<f32>;
@group(1) @binding(5)
var weather_surface_previous: texture_cube<f32>;

@group(2) @binding(0)
var atmosphere_transmittance_lut: texture_2d<f32>;
@group(2) @binding(1)
var atmosphere_irradiance_lut: texture_2d<f32>;
@group(2) @binding(2)
var atmosphere_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) altitude: f32,
    @location(3) @interpolate(flat) shell_index: u32,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let direction = normalize(input.position);
    let shell_radius = select(
        weather.lower_shell_radius_meters,
        weather.upper_shell_radius_meters,
        instance_index == 1u,
    );
    let world_position = direction * shell_radius;
    let camera_position_view = normalize(camera.camera_planet_direction_view_altitude.xyz)
        * (PLANET_RADIUS_METERS + camera.camera_planet_direction_view_altitude.w);
    let camera_relative_view = planet_to_view(world_position) - camera_position_view;
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(camera_relative_view, 1.0),
        direction,
        normalize(input.normal),
        shell_radius - PLANET_RADIUS_METERS,
        instance_index,
    );
}

fn direct_atmosphere_uv(altitude: f32, solar_zenith_cosine: f32) -> vec2<f32> {
    // The LUT stores the compressed optical atmosphere. Preserve its signed
    // low-sun column so clouds inherit the physical red/orange transmission,
    // but stay just above the optical horizon to avoid the solid-planet rows.
    let optical_altitude = max(altitude, 0.0) / ATMOSPHERE_VERTICAL_SCALE;
    let optical_radius = PLANET_RADIUS_METERS + optical_altitude;
    let optical_horizon = -sqrt(max(
        1.0 - (PLANET_RADIUS_METERS / optical_radius)
            * (PLANET_RADIUS_METERS / optical_radius),
        0.0,
    ));
    let safe_solar_zenith_cosine = max(
        solar_zenith_cosine,
        optical_horizon + 0.004625,
    );
    return vec2<f32>(
        clamp(safe_solar_zenith_cosine * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(max(altitude, 0.0) / ATMOSPHERE_VERTICAL_SCALE / OPTICAL_ATMOSPHERE_HEIGHT_METERS, 0.0, 1.0)),
    );
}

fn irradiance_atmosphere_uv(altitude: f32, solar_zenith_cosine: f32) -> vec2<f32> {
    return vec2<f32>(
        clamp(solar_zenith_cosine * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(max(altitude, 0.0) / ATMOSPHERE_VERTICAL_SCALE / OPTICAL_ATMOSPHERE_HEIGHT_METERS, 0.0, 1.0)),
    );
}

fn cloud_horizon_cosine(altitude: f32) -> f32 {
    let radius = PLANET_RADIUS_METERS + max(altitude, 0.0);
    return -sqrt(max(
        1.0 - (PLANET_RADIUS_METERS / radius) * (PLANET_RADIUS_METERS / radius),
        0.0,
    ));
}

fn cloud_layer_sun_visibility(altitude: f32, solar_zenith_cosine: f32) -> f32 {
    // A rendered shell is the centre of a cloud volume, not an infinitesimal
    // sheet. Its high edge sees the sun first and its low edge sees the full
    // disc last. Integrating that vertical extent removes the isolated,
    // hard-edged twilight strip produced by the two point-height horizons.
    let first_light_horizon = cloud_horizon_cosine(
        altitude + CLOUD_LAYER_HALF_DEPTH_METERS,
    );
    let fully_lit_horizon = cloud_horizon_cosine(
        altitude - CLOUD_LAYER_HALF_DEPTH_METERS,
    );
    let solar_angular_radius_sine = 0.004625;
    return smoothstep(
        first_light_horizon - solar_angular_radius_sine,
        fully_lit_horizon + solar_angular_radius_sine,
        solar_zenith_cosine,
    );
}

fn cloud_view_transmittance(
    camera_altitude: f32,
    cloud_altitude: f32,
    view_length: f32,
    view_zenith_cosine: f32,
) -> vec3<f32> {
    // Shells are composited after the terrain pass, so they must carry the
    // same scale-height-limited camera column as distant terrain. Otherwise a
    // white cloud at 90km is pasted over a red sunset with no view extinction.
    let camera_density_rayleigh = exp(-max(camera_altitude, 0.0) / RAYLEIGH_SCALE_HEIGHT_METERS);
    let cloud_density_rayleigh = exp(-max(cloud_altitude, 0.0) / RAYLEIGH_SCALE_HEIGHT_METERS);
    let camera_density_mie = exp(-max(camera_altitude, 0.0) / MIE_SCALE_HEIGHT_METERS);
    let cloud_density_mie = exp(-max(cloud_altitude, 0.0) / MIE_SCALE_HEIGHT_METERS);
    let air_mass = min(1.0 / max(view_zenith_cosine, 0.08), 12.0);
    let rayleigh_path = min(view_length, 2.0 * RAYLEIGH_SCALE_HEIGHT_METERS * air_mass);
    let mie_path = min(view_length, 2.0 * MIE_SCALE_HEIGHT_METERS * air_mass);
    return exp(-(
        RAYLEIGH_COEFFICIENT
            * 0.5 * (camera_density_rayleigh + cloud_density_rayleigh)
            * rayleigh_path
        + MIE_COEFFICIENT
            * 0.5 * (camera_density_mie + cloud_density_mie)
            * mie_path
    ));
}

// The terrain depth buffer normally rejects clouds behind the ground, but it
// is not allowed to be the planet's only occluder. A near-plane or coarse-mesh
// hole must not expose the cloud shell on the other side of the world.
fn solid_planet_blocks_cloud(
    camera_position: vec3<f32>,
    cloud_position: vec3<f32>,
) -> bool {
    let segment = cloud_position - camera_position;
    let a = dot(segment, segment);
    let b = 2.0 * dot(camera_position, segment);
    let c = dot(camera_position, camera_position)
        - PLANET_RADIUS_METERS * PLANET_RADIUS_METERS;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant <= 0.0 || a <= 1.0e-6 {
        return false;
    }
    let root = sqrt(discriminant);
    let near_t = (-b - root) / (2.0 * a);
    let far_t = (-b + root) / (2.0 * a);
    return (near_t > 1.0e-5 && near_t < 0.99999)
        || (far_t > 1.0e-5 && far_t < 0.99999);
}

// Forward Mie scattering is what gives a terrestrial cloud its bright silver
// lining when the sun is immediately behind a thin edge. Henyey-Greenstein is
// a compact approximation of that strongly forward-peaked phase function.
fn henyey_greenstein(cosine_theta: f32, asymmetry: f32) -> f32 {
    let denominator = max(
        1.0 + asymmetry * asymmetry - 2.0 * asymmetry * cosine_theta,
        1.0e-4,
    );
    return (1.0 - asymmetry * asymmetry)
        / (4.0 * 3.14159265 * pow(denominator, 1.5));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let camera_position_view = normalize(
        camera.camera_planet_direction_view_altitude.xyz,
    ) * (PLANET_RADIUS_METERS + camera.camera_planet_direction_view_altitude.w);
    let cloud_position_view = planet_to_view(
        normalize(input.direction) * (PLANET_RADIUS_METERS + input.altitude),
    );
    if solid_planet_blocks_cloud(camera_position_view, cloud_position_view) {
        discard;
    }
    let cloud = cloudSample(input.direction, f32(input.shell_index));
    let density = cloud.density;
    // Keep tenuous and mid-density weather from becoming a grey film that
    // repeatedly desaturates the physical sunset as the camera moves beneath
    // it. Dense storm cloud still reaches the established 0.78 ceiling, so
    // overcast regions retain their contrast and shadow strength.
    let alpha = density * mix(0.50, 0.78, density);
    if alpha < 0.002 {
        discard;
    }
    let normal = normalize(input.normal);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_zenith_cosine = dot(normalize(input.direction), sun_direction);
    let transmittance = textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_sampler,
        direct_atmosphere_uv(input.altitude, solar_zenith_cosine),
        0.0,
    ).rgb * cloud_layer_sun_visibility(input.altitude, solar_zenith_cosine);
    let irradiance = textureSampleLevel(
        atmosphere_irradiance_lut,
        atmosphere_sampler,
        irradiance_atmosphere_uv(input.altitude, solar_zenith_cosine),
        0.0,
    ).rgb;
    // Match the direct-light scale used by terrain/ocean. Keeping the RGB
    // transmittance intact is what makes the same physical low-sun column
    // read warm instead of letting neutral skylight wash the cloud white.
    let direct = transmittance
        * (0.20 + 0.80 * max(dot(normal, sun_direction), 0.0))
        * 2.0;
    let sky = mix(vec3<f32>(dot(irradiance, vec3<f32>(0.2126, 0.7152, 0.0722))), irradiance, 0.55);
    // Near the local horizon the direct RGB column is the sunset colour. Do
    // not let the broad neutral upper-sky fill wash that physical signal out;
    // overhead clouds retain the full ambient term.
    let low_sun_amount = 1.0 - smoothstep(0.0, 0.35, solar_zenith_cosine);
    let sky_fill = mix(0.55, 0.16, low_sun_amount)
        * (0.55 + 0.45 * max(dot(normal, normalize(input.direction)), 0.0));
    let base_lighting = max(direct + sky * sky_fill, vec3<f32>(0.0));
    let fair_weather_albedo = mix(
        vec3<f32>(0.42, 0.46, 0.50),
        vec3<f32>(0.94, 0.96, 1.0),
        clamp(density * 2.0, 0.0, 1.0),
    );
    let storm_weight = smoothstep(0.10, 0.30, cloud.storm)
        * smoothstep(0.30, 0.78, density);
    let storm_darkening = 1.0 - 0.72 * storm_weight;
    let albedo = fair_weather_albedo
        * storm_darkening
        * select(1.0, 0.82, input.shell_index == 1u);
    let camera_to_cloud = normalize(cloud_position_view - camera_position_view);
    let view_transmittance = cloud_view_transmittance(
        camera.camera_planet_direction_view_altitude.w,
        input.altitude,
        length(cloud_position_view - camera_position_view),
        dot(
            normalize(camera.camera_planet_direction_view_altitude.xyz),
            camera_to_cloud,
        ),
    );
    let lighting = base_lighting * view_transmittance;
    let sun_alignment = clamp(
        dot(camera_to_cloud, normalize(camera.sun_direction_view.xyz)),
        -1.0,
        1.0,
    );
    let forward_phase = clamp(
        henyey_greenstein(sun_alignment, 0.76)
            / henyey_greenstein(1.0, 0.76),
        0.0,
        1.0,
    );
    // Only the optically thin fringe receives the extra forward-scattered
    // sunlight. Dense storm interiors remain dark rather than emissive.
    let translucent_edge = smoothstep(0.025, 0.20, alpha)
        * (1.0 - smoothstep(0.38, 0.62, alpha));
    let silver_lining = transmittance
        * view_transmittance
        * forward_phase
        * translucent_edge
        * 2.5;
    return vec4<f32>(albedo * lighting + silver_lining, alpha);
}

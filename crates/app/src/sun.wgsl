const PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS: f32 = 0.004625;
const PLANET_RADIUS_METERS: f32 = 4000000.0;
const ATMOSPHERE_HEIGHT_METERS: f32 = 2880000.0;
const ATMOSPHERE_RADIUS_METERS: f32 = PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS;
// The game presentation deliberately uses twice the real solar angular
// diameter, while retaining the physical radius as the reference value.
const VISUAL_SUN_SIZE_SCALE: f32 = 2.0;
const SUN_ANGULAR_RADIUS_RADIANS: f32 = PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS * VISUAL_SUN_SIZE_SCALE;
// A compact, soft corona gives the camera-like glow seen around a bright sun
// without turning the whole sky into a white disk.
// Halve the radius multipliers when doubling the disc so the requested sun
// grows without also quadrupling the flare's screen area and fragment cost.
const SUN_HALO_RADIUS_SCALE: f32 = 3.25;
const SUN_INNER_GLARE_RADIUS_SCALE: f32 = 1.25;
// A camera spreads a bright solar source into a soft veil and a few aperture
// rays. Keep both deliberately separate from the doubled visual disc so they
// cannot change solar geometry or the sun's physical lighting.
const SUN_VEILING_GLARE_RADIUS_SCALE: f32 = 15.0;
const SUN_VEILING_GLARE_RADIANCE: vec3<f32> = vec3<f32>(0.20, 0.16, 0.11);
const SUN_STAR_RAY_RADIUS_SCALE: f32 = 21.0;
const SUN_STAR_RAY_RADIANCE: vec3<f32> = vec3<f32>(0.40, 0.32, 0.23);
// Keep the discard outside every presentation lobe, rather than at the veil
// radius itself. Otherwise the zero-valued tail still leaves a visible ring.
const SUN_OVERLAY_CUTOFF_RADIUS_SCALE: f32 = 32.0;
// This multiplier belongs only to the camera-facing HDR disc.  Terrain,
// ocean, and atmosphere lighting use their own physical solar radiance.
const SUN_VISUAL_RADIANCE_SCALE: f32 = 5.0;
// Keep the physically tinted core legible at the horizon. This is a camera
// presentation floor, not a lighting contribution; the terrain and sky still
// receive the unmodified atmospheric transmittance.
const SUN_CORE_VISIBILITY_FLOOR: f32 = 0.12;
// Keep the visual disc readable at the last above-horizon samples. The
// surrounding glare may collapse with transmittance; the disc itself must not
// disappear before geometric occultation by the planet.
const SUN_CORE_RADIANCE_FLOOR: f32 = 0.50;
const SUN_GLARE_VISIBILITY_FLOOR: f32 = 0.08;
// The last useful physical red column sits about 0.05 in solar-direction
// cosine above the LUT's opaque horizon row. Present that column at the
// geometric horizon and hold it below, rather than sampling into black.
const SUN_HORIZON_LUT_ELEVATION: f32 = 0.05;
const SUN_CORE_RADIANCE: vec3<f32> = vec3<f32>(72.0, 65.0, 52.0);
const SUN_HALO_RADIANCE: vec3<f32> = vec3<f32>(4.0, 2.8, 1.3);
const SUN_GLARE_RADIANCE: vec3<f32> = vec3<f32>(2.5, 1.8, 0.8);

struct Camera {
    projection_matrix: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_view_altitude: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_direction_view: vec4<f32>,
    projection: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;

struct WeatherRenderUniform {
    blend: f32,
    drift_radians: f32,
    lower_shell_radius_meters: f32,
    upper_shell_radius_meters: f32,
    noise_scale: f32,
    noise_strength: f32,
    _padding: vec2<f32>,
}

@group(1) @binding(0)
var cloud_field_current: texture_cube<f32>;
@group(1) @binding(1)
var cloud_field_previous: texture_cube<f32>;
@group(1) @binding(2)
var cloud_field_sampler: sampler;
@group(1) @binding(3)
var<uniform> weather: WeatherRenderUniform;

@group(2) @binding(0)
var atmosphere_transmittance_lut: texture_2d<f32>;
@group(2) @binding(1)
var atmosphere_physical_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let horizontal = ndc.x * camera.projection.x * camera.projection.y;
    let vertical = ndc.y * camera.projection.y;
    return normalize(vec3<f32>(horizontal, vertical, -1.0));
}

fn view_to_planet(vector: vec3<f32>) -> vec3<f32> {
    return camera.camera_right.xyz * vector.x
        + camera.camera_up.xyz * vector.y
        - camera.camera_forward.xyz * vector.z;
}

fn sun_screen_position(sun: vec3<f32>) -> vec2<f32> {
    // The sun direction uses the same camera-view convention as
    // view_direction: a visible source has negative view-space Z.
    let depth = max(-sun.z, 1.0e-4);
    return vec2<f32>(
        sun.x / (depth * camera.projection.x * camera.projection.y),
        sun.y / (depth * camera.projection.y),
    );
}

fn cloud_density_on_camera_ray(
    ray_view: vec3<f32>,
    shell_radius: f32,
    shell_index: f32,
) -> f32 {
    let camera_position = normalize(
        camera.camera_planet_direction_view_altitude.xyz,
    ) * (
        PLANET_RADIUS_METERS
            + max(camera.camera_planet_direction_view_altitude.w, 0.0)
    );
    let ray_offset = dot(camera_position, ray_view);
    let discriminant = ray_offset * ray_offset
        - dot(camera_position, camera_position)
        + shell_radius * shell_radius;
    if discriminant <= 0.0 {
        return 0.0;
    }
    let root = sqrt(discriminant);
    let near_distance = -ray_offset - root;
    let far_distance = -ray_offset + root;
    var distance = near_distance;
    if distance <= 1.0 {
        distance = far_distance;
    }
    if distance <= 1.0 {
        return 0.0;
    }
    let cloud_position_view = camera_position + ray_view * distance;
    let cloud_direction = normalize(view_to_planet(cloud_position_view));
    return cloudDensityWithOctaves(cloud_direction, shell_index, 3u);
}

fn cloud_sun_visibility(ray_view: vec3<f32>) -> f32 {
    let lower_density = cloud_density_on_camera_ray(
        ray_view,
        weather.lower_shell_radius_meters,
        0.0,
    );
    let upper_density = cloud_density_on_camera_ray(
        ray_view,
        weather.upper_shell_radius_meters,
        1.0,
    );
    // Match the visible cloud shell's opacity response. Each layer transmits
    // the light left by the other, so a genuinely opaque storm can hide both
    // the physical disc and its camera-only halo while a wisp merely dims it.
    let lower_alpha = max(
        lower_density,
        smoothstep(0.25, 0.85, lower_density),
    );
    let upper_alpha = max(
        upper_density,
        smoothstep(0.25, 0.85, upper_density),
    );
    let geometric_transmission = (1.0 - lower_alpha) * (1.0 - upper_alpha);
    // Alpha describes camera compositing, not the optical depth through a
    // many-kilometre cloud. Convert it into a steeper light transmission so
    // an HDR sun cannot remain white merely because ten percent leaks through.
    let cloud_opacity = 1.0 - geometric_transmission;
    if cloud_opacity >= 0.60 {
        return 0.0;
    }
    return pow(geometric_transmission, 4.0);
}

fn sampled_sun_transmittance(
    observer_altitude: f32,
    solar_elevation: f32,
) -> vec3<f32> {
    let optical_altitude = max(observer_altitude, 0.0) / 4.5;
    let uv = vec2<f32>(
        clamp(solar_elevation * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(optical_altitude / 640000.0, 0.0, 1.0)),
    );
    return textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_physical_sampler,
        uv,
        0.0,
    ).rgb;
}

fn relative_sun_transmittance(
    observer_altitude: f32,
    solar_elevation: f32,
) -> vec3<f32> {
    // Use the same wavelength-dependent column as direct terrain and ocean
    // light. Dividing by the local zenith result preserves the established
    // midday disc brightness while retaining the LUT's low-sun dimming and
    // red shift instead of imposing a separately timed authored tint.
    // Delay the useful red endpoint until the geometric horizon. Subtracting
    // this offset sampled the LUT's opaque rows early and was the direct cause
    // of the disc disappearing while it was still visibly above the horizon.
    let visible_solar_elevation = max(solar_elevation, 0.0)
        + SUN_HORIZON_LUT_ELEVATION;
    let transmitted = sampled_sun_transmittance(
        observer_altitude,
        visible_solar_elevation,
    );
    let zenith = sampled_sun_transmittance(observer_altitude, 1.0);
    return clamp(
        transmitted / max(zenith, vec3<f32>(1.0e-4)),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

struct SunAtmosphereSample {
    transmittance: vec3<f32>,
    presentation_elevation: f32,
}

fn sun_disc_atmosphere_sample(solar_elevation: f32) -> SunAtmosphereSample {
    let camera_altitude = max(
        camera.camera_planet_direction_view_altitude.w,
        0.0,
    );
    if camera_altitude < ATMOSPHERE_HEIGHT_METERS {
        return SunAtmosphereSample(
            relative_sun_transmittance(camera_altitude, solar_elevation),
            solar_elevation,
        );
    }

    // Outside the atmosphere, a negative local elevation is not a sunset:
    // "down" is only the direction toward the planet. Attenuate the visual
    // sun only when its actual camera ray enters the world-space air shell.
    let camera_position = normalize(
        camera.camera_planet_direction_view_altitude.xyz,
    ) * (PLANET_RADIUS_METERS + camera_altitude);
    let sun_direction = normalize(camera.sun_direction_view.xyz);
    let radial_dot_sun = dot(camera_position, sun_direction);
    let discriminant = radial_dot_sun * radial_dot_sun
        - dot(camera_position, camera_position)
        + ATMOSPHERE_RADIUS_METERS * ATMOSPHERE_RADIUS_METERS;
    if discriminant <= 0.0 {
        return SunAtmosphereSample(vec3<f32>(1.0), 1.0);
    }
    let entry_distance = -radial_dot_sun - sqrt(discriminant);
    if entry_distance <= 0.0 {
        return SunAtmosphereSample(vec3<f32>(1.0), 1.0);
    }
    let entry_position = camera_position + sun_direction * entry_distance;
    let entry_radius = length(entry_position);
    let entry_altitude = max(entry_radius - PLANET_RADIUS_METERS, 0.0);
    let entry_solar_elevation = dot(entry_position / entry_radius, sun_direction);
    return SunAtmosphereSample(
        relative_sun_transmittance(entry_altitude, entry_solar_elevation),
        entry_solar_elevation,
    );
}

fn sun_disc_is_fully_occulted() -> bool {
    let camera_radius = max(
        PLANET_RADIUS_METERS + camera.camera_planet_direction_view_altitude.w,
        PLANET_RADIUS_METERS,
    );
    let planet_center_direction = -normalize(
        camera.camera_planet_direction_view_altitude.xyz,
    );
    let sun_direction = normalize(camera.sun_direction_view.xyz);
    let center_angle = acos(clamp(
        dot(sun_direction, planet_center_direction),
        -1.0,
        1.0,
    ));
    let planet_angular_radius = asin(clamp(
        PLANET_RADIUS_METERS / camera_radius,
        0.0,
        1.0,
    ));
    // Depth still clips each fragment during partial occultation. Once the
    // complete physical disc is behind the solid planet, discard the whole
    // camera overlay so the larger halo cannot remain around the silhouette.
    return center_angle + SUN_ANGULAR_RADIUS_RADIANS <= planet_angular_radius;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), position);
}

fn sun_radiance(input: VertexOutput, draw_disc: bool) -> vec4<f32> {
    let ray = view_direction(input.ndc);
    let sun = normalize(camera.sun_direction_view.xyz);
    let sun_ndc = sun_screen_position(sun);
    let lens_offset_ndc = input.ndc - sun_ndc;
    let lens_offset_view = vec2<f32>(
        lens_offset_ndc.x * camera.projection.x * camera.projection.y,
        lens_offset_ndc.y * camera.projection.y,
    );
    let lens_offset_length = length(lens_offset_view);
    let lens_distance = lens_offset_length / SUN_ANGULAR_RADIUS_RADIANS;
    let lens_unit = lens_offset_view / max(lens_offset_length, 1.0e-4);
    let alignment = clamp(dot(ray, sun), -1.0, 1.0);
    let angular_distance = atan2(length(cross(ray, sun)), alignment);
    let normalized_distance = angular_distance / SUN_ANGULAR_RADIUS_RADIANS;

    if normalized_distance > SUN_OVERLAY_CUTOFF_RADIUS_SCALE {
        return vec4<f32>(0.0);
    }
    let disc_coverage = 1.0 - smoothstep(0.92, 1.0, normalized_distance);
    let limb_darkening = 1.0 - 0.25 * min(normalized_distance, 1.0);
    let halo = pow(max(1.0 - normalized_distance / SUN_HALO_RADIUS_SCALE, 0.0), 2.5);
    let inner_glare = pow(
        max(1.0 - normalized_distance / SUN_INNER_GLARE_RADIUS_SCALE, 0.0),
        2.0,
    );
    let veiling_glare = pow(
        max(1.0 - normalized_distance / SUN_VEILING_GLARE_RADIUS_SCALE, 0.0),
        1.7,
    );
    // A small aperture produces the narrow star rays visible in photographs.
    // They begin outside the disc, taper smoothly, and remain camera-only
    // presentation so the physical solar lighting is untouched.
    // Chebyshev angle multiples keep this presentation cheap on the older
    // mobile GPU: no additional trigonometric calls are needed per pixel.
    let cos_two = 2.0 * lens_unit.x * lens_unit.x - 1.0;
    let cos_four = 2.0 * cos_two * cos_two - 1.0;
    let cos_eight = 2.0 * cos_four * cos_four - 1.0;
    let major_star_rays = pow(max(abs(cos_four), 0.0), 28.0);
    let minor_star_rays = pow(max(abs(cos_eight), 0.0), 72.0);
    let star_ray_profile = smoothstep(1.15, 2.2, lens_distance)
        * pow(
            max(1.0 - lens_distance / SUN_STAR_RAY_RADIUS_SCALE, 0.0),
            2.2,
        )
        * step(0.0, -sun.z);
    let star_rays = star_ray_profile * (
        0.88 * major_star_rays + 0.12 * minor_star_rays
    );
    let solar_elevation = dot(
        normalize(camera.camera_planet_direction_view_altitude.xyz),
        normalize(camera.sun_direction_view.xyz),
    );
    let atmosphere_sample = sun_disc_atmosphere_sample(solar_elevation);
    let tint = atmosphere_sample.transmittance;
    let presentation_elevation = atmosphere_sample.presentation_elevation;
    // Preserve the real angular diameter while atmospheric distance dims and
    // reddens the core. Only the glare/halo receives the stronger visibility
    // rolloff, so a sunset sun does not appear to contract toward a point.
    // A second bounded camera-only optical column prevents the overbright HDR
    // core from clipping its physical red shift back to white at the limb.
    let low_sun_amount = 1.0 - smoothstep(0.0, 0.25, presentation_elevation);
    let limb_tint = mix(
        vec3<f32>(1.0),
        vec3<f32>(1.0, 0.20, 0.03),
        low_sun_amount,
    );
    let presentation_tint = tint
        * limb_tint
        * mix(vec3<f32>(1.0), tint, low_sun_amount);
    let core_radiance_scale = mix(
        SUN_CORE_RADIANCE_FLOOR,
        1.0,
        smoothstep(0.0, 0.25, presentation_elevation),
    );
    let strongest_channel = max(
        presentation_tint.r,
        max(presentation_tint.g, presentation_tint.b),
    );
    let core_visibility = max(strongest_channel, SUN_CORE_VISIBILITY_FLOOR);
    // A visibility floor cannot revive a zero vector. Preserve the physical
    // transmitted hue while it is representable, then use its limiting red
    // hue if half-float LUT precision has underflowed. This affects only the
    // camera overlay, never atmosphere or surface lighting.
    var core_hue = vec3<f32>(1.0, 0.08, 0.01);
    if strongest_channel > 1.0e-4 {
        core_hue = presentation_tint / strongest_channel;
    }
    let core_tint = core_hue * core_visibility;
    let glare_visibility = max(pow(strongest_channel, 4.0), SUN_GLARE_VISIBILITY_FLOOR);
    let atmospheric_core = core_radiance_scale * core_tint * (
        SUN_CORE_RADIANCE * disc_coverage * limb_darkening
    );
    let atmospheric_glare = presentation_tint * glare_visibility * (
        SUN_HALO_RADIANCE * halo
            + SUN_GLARE_RADIANCE * inner_glare
            + SUN_VEILING_GLARE_RADIANCE * veiling_glare
            + SUN_STAR_RAY_RADIANCE * star_rays
    );
    // The broad veiling response is scattered light around the disc, so a
    // cloud blocks it more strongly than the disc core itself. This keeps a
    // storm from leaving a bright camera bloom after it has hidden the sun.
    let cloud_visibility = cloud_sun_visibility(sun);
    let glare_cloud_visibility = pow(cloud_visibility, 4.0);
    let radiance = SUN_VISUAL_RADIANCE_SCALE * select(
        atmospheric_glare * glare_cloud_visibility,
        atmospheric_core * cloud_visibility,
        draw_disc,
    );
    return vec4<f32>(radiance, 1.0);
}

// The visual solar disc is depth-tested against terrain and the planet.
// Partial occultation therefore clips only the actual source geometry.
@fragment
fn fs_disc(input: VertexOutput) -> @location(0) vec4<f32> {
    if sun_disc_is_fully_occulted() {
        discard;
    }
    return sun_radiance(input, true);
}

// Corona, star rays, and veil are camera response, not geometry surrounding
// the sun. Draw their complete shape while any part of the visual disc remains
// visible; the analytic full-disc test removes the response as soon as the
// planet has occulted the complete source.
@fragment
fn fs_flare(input: VertexOutput) -> @location(0) vec4<f32> {
    if sun_disc_is_fully_occulted() {
        discard;
    }
    return sun_radiance(input, false);
}

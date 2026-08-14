const PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS: f32 = 0.004625;
// Use the real solar angular diameter (~0.53 degrees) for near-surface views.
// The previous 0.159 degree presentation made the sun read like a distant
// star rather than the disk seen in an Earth-sky photograph.
const VISUAL_SUN_SIZE_SCALE: f32 = 1.0;
const SUN_ANGULAR_RADIUS_RADIANS: f32 = PHYSICAL_SUN_ANGULAR_RADIUS_RADIANS * VISUAL_SUN_SIZE_SCALE;
// A compact, soft corona gives the camera-like glow seen around a bright sun
// without turning the whole sky into a white disk.
const SUN_HALO_RADIUS_SCALE: f32 = 6.5;
const SUN_INNER_GLARE_RADIUS_SCALE: f32 = 2.5;
// This multiplier belongs only to the camera-facing HDR disc.  Terrain,
// ocean, and atmosphere lighting use their own physical solar radiance.
const SUN_VISUAL_RADIANCE_SCALE: f32 = 5.0;
// Keep the physically tinted core legible at the horizon. This is a camera
// presentation floor, not a lighting contribution; the terrain and sky still
// receive the unmodified atmospheric transmittance.
const SUN_CORE_VISIBILITY_FLOOR: f32 = 0.12;
// Keep the physical disc readable at the last above-horizon samples. The
// surrounding glare may collapse with transmittance; the disc itself must not
// disappear before geometric occultation by the planet.
const SUN_CORE_RADIANCE_FLOOR: f32 = 0.50;
const SUN_GLARE_VISIBILITY_FLOOR: f32 = 0.18;
// The last visible above-horizon sample used a roughly -0.05 radian optical
// column. Hold that same physical-looking red/dim state once the centre
// reaches the horizon, rather than letting the overlay fade through black.
const SUN_HORIZON_LUT_ELEVATION_RADIANS: f32 = -0.05;
const SUN_CORE_RADIANCE: vec3<f32> = vec3<f32>(72.0, 65.0, 52.0);
const SUN_HALO_RADIANCE: vec3<f32> = vec3<f32>(10.0, 7.0, 3.5);
const SUN_GLARE_RADIANCE: vec3<f32> = vec3<f32>(8.0, 5.5, 2.5);

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
@group(1) @binding(0)
var atmosphere_transmittance_lut: texture_2d<f32>;
@group(1) @binding(1)
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

fn sampled_sun_transmittance(solar_elevation: f32) -> vec3<f32> {
    let optical_altitude = max(
        camera.camera_planet_direction_view_altitude.w,
        0.0,
    ) / 4.5;
    let uv = vec2<f32>(
        clamp(solar_elevation * 0.5 + 0.5, 0.0, 1.0),
        sqrt(clamp(optical_altitude / 320000.0, 0.0, 1.0)),
    );
    return textureSampleLevel(
        atmosphere_transmittance_lut,
        atmosphere_physical_sampler,
        uv,
        0.0,
    ).rgb;
}

fn sun_disc_atmospheric_transmittance(solar_elevation: f32) -> vec3<f32> {
    // Use the same wavelength-dependent column as direct terrain and ocean
    // light. Dividing by the local zenith result preserves the established
    // midday disc brightness while retaining the LUT's low-sun dimming and
    // red shift instead of imposing a separately timed authored tint.
    // Shift the optical column so its red/dim endpoint is reached at the
    // geometric horizon, then hold that endpoint below it. The planet/depth
    // test remains responsible for removing the disc; the overlay must not
    // disappear merely because the LUT was sampled farther down the column.
    let visible_solar_elevation = max(
        solar_elevation - 0.05,
        SUN_HORIZON_LUT_ELEVATION_RADIANS,
    );
    let transmitted = sampled_sun_transmittance(visible_solar_elevation);
    let zenith = sampled_sun_transmittance(1.0);
    let relative_transmittance = clamp(
        transmitted / max(zenith, vec3<f32>(1.0e-4)),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    // Return the physical wavelength-dependent attenuation only. The disc's
    // angular coverage must remain constant as it approaches the horizon;
    // camera-only glare can fade separately without shrinking the solar core.
    return relative_transmittance;
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

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let ray = view_direction(input.ndc);
    let sun = normalize(camera.sun_direction_view.xyz);
    let alignment = clamp(dot(ray, sun), -1.0, 1.0);
    let angular_distance = atan2(length(cross(ray, sun)), alignment);
    let normalized_distance = angular_distance / SUN_ANGULAR_RADIUS_RADIANS;
    if normalized_distance > SUN_HALO_RADIUS_SCALE {
        discard;
    }
    let disc_coverage = 1.0 - smoothstep(0.92, 1.0, normalized_distance);
    let limb_darkening = 1.0 - 0.25 * min(normalized_distance, 1.0);
    let halo = pow(max(1.0 - normalized_distance / SUN_HALO_RADIUS_SCALE, 0.0), 2.5);
    let inner_glare = pow(
        max(1.0 - normalized_distance / SUN_INNER_GLARE_RADIUS_SCALE, 0.0),
        2.0,
    );
    let solar_elevation = dot(
        normalize(camera.camera_planet_direction_view_altitude.xyz),
        normalize(camera.sun_direction_view.xyz),
    );
    let tint = sun_disc_atmospheric_transmittance(solar_elevation);
    // Preserve the real angular diameter while atmospheric distance dims and
    // reddens the core. Only the glare/halo receives the stronger visibility
    // rolloff, so a sunset sun does not appear to contract toward a point.
    // A second bounded camera-only optical column prevents the overbright HDR
    // core from clipping its physical red shift back to white at the limb.
    let low_sun_amount = 1.0 - smoothstep(0.0, 0.25, solar_elevation);
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
        smoothstep(0.0, 0.25, solar_elevation),
    );
    let strongest_channel = max(
        presentation_tint.r,
        max(presentation_tint.g, presentation_tint.b),
    );
    let core_visibility = max(strongest_channel, SUN_CORE_VISIBILITY_FLOOR);
    let core_tint = presentation_tint
        * (core_visibility / max(strongest_channel, 1.0e-4));
    let glare_visibility = max(pow(strongest_channel, 4.0), SUN_GLARE_VISIBILITY_FLOOR);
    let atmospheric_core = core_radiance_scale * core_tint * (
        SUN_CORE_RADIANCE * disc_coverage * limb_darkening
    );
    let atmospheric_glare = presentation_tint * glare_visibility * (
        SUN_HALO_RADIANCE * halo + SUN_GLARE_RADIANCE * inner_glare
    );
    let radiance = SUN_VISUAL_RADIANCE_SCALE
        * (atmospheric_core + atmospheric_glare);
    return vec4<f32>(radiance, 1.0);
}

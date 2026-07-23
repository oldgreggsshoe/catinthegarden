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

struct WarpUniform {
    fovea_ndc: vec2<f32>,
    debug_view: u32,
    experiment_flags: u32,
}

const EXPERIMENT_RADIAL_BLUR: u32 = 1u << 4u;
const EXPERIMENT_HALFTONE: u32 = 1u << 5u;
const HALFTONE_CELL_NDC: f32 = 0.06;
const HALFTONE_MIN_DOT_FRACTION: f32 = 0.12;
const HALFTONE_MAX_DOT_FRACTION: f32 = 0.92;
const HALFTONE_TONE_COMPRESSION: f32 = 0.12;
const HALFTONE_EDGE_SOFTNESS_NDC: f32 = 0.004;
const HALFTONE_BACKGROUND_WEIGHT: f32 = 0.08;

@group(0) @binding(0)
var<uniform> camera: Camera;
@group(1) @binding(0)
var warp_color: texture_2d<f32>;
@group(1) @binding(1)
var warp_distance: texture_2d<f32>;
@group(1) @binding(2)
var warp_sampler: sampler;
@group(1) @binding(3)
var<uniform> warp_settings: WarpUniform;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

fn view_direction(ndc: vec2<f32>) -> vec3<f32> {
    let horizontal = ndc.x * camera.projection.x * camera.projection.y;
    let vertical = ndc.y * camera.projection.y;
    return normalize(vec3<f32>(horizontal, vertical, -1.0));
}

fn unwarp_axis(coordinate: f32) -> f32 {
    const EXPONENT: f32 = 2.0;
    const LINEAR_CORE: f32 = 0.5;
    let magnitude = abs(coordinate);
    let core_power = pow(LINEAR_CORE, EXPONENT);
    let denominator = pow(1.0 + LINEAR_CORE, EXPONENT) - core_power;
    let unwarped = pow(
        clamp(magnitude, 0.0, 1.0) * denominator + core_power,
        1.0 / EXPONENT,
    ) - LINEAR_CORE;
    return sign(coordinate) * unwarped;
}

fn unwarped_texture_axis(screen_coordinate: f32, fovea: f32) -> f32 {
    let offset = screen_coordinate - fovea;
    let side_extent = select(1.0 + fovea, 1.0 - fovea, offset >= 0.0);
    return unwarp_axis(offset / side_extent);
}

fn texture_uv(ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

fn texture_ndc_for_screen(screen_ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        unwarped_texture_axis(screen_ndc.x, warp_settings.fovea_ndc.x),
        unwarped_texture_axis(screen_ndc.y, warp_settings.fovea_ndc.y),
    );
}

// Quantizes the screen into aspect-correct square cells and, in each cell,
// samples one color and draws it as a filled circle sized by that sample's
// tone (bright cells get large dots, dark cells get small ones) rather than
// shading every pixel independently. This is a real halftone/Ben-Day screen,
// not just a dot-shaped tint: dot *size* carries the tonal variation, the
// way it does in print, instead of every dot being the same size. It turns
// per-pixel undersampling noise (the coarse-field/material shimmer at low
// altitude) into a deliberate stipple pattern instead of raw grain.
fn halftone_cell_center_ndc(screen_ndc: vec2<f32>, aspect_ratio: f32) -> vec2<f32> {
    let corrected = vec2<f32>(screen_ndc.x * aspect_ratio, screen_ndc.y);
    let cell = floor(corrected / HALFTONE_CELL_NDC);
    let cell_center_corrected = (cell + vec2<f32>(0.5)) * HALFTONE_CELL_NDC;
    return vec2<f32>(cell_center_corrected.x / aspect_ratio, cell_center_corrected.y);
}

fn halftone_color(screen_ndc: vec2<f32>) -> vec3<f32> {
    let aspect_ratio = camera.projection.x;
    let cell_center_ndc = halftone_cell_center_ndc(screen_ndc, aspect_ratio);
    let cell_uv = texture_uv(texture_ndc_for_screen(cell_center_ndc));
    let cell_color = textureSampleLevel(warp_color, warp_sampler, cell_uv, 0.0).rgb;
    let corrected = vec2<f32>(screen_ndc.x * aspect_ratio, screen_ndc.y);
    let cell_center_corrected = vec2<f32>(
        cell_center_ndc.x * aspect_ratio,
        cell_center_ndc.y,
    );

    // Reinhard-style local response: maps any HDR radiance (this samples the
    // pre-tonemap scene color) into 0..1 without needing the real exposure
    // value, so dot size still varies sensibly however bright the frame is.
    let luminance = dot(cell_color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let tone = luminance / (luminance + HALFTONE_TONE_COMPRESSION);
    let dot_fraction = mix(HALFTONE_MIN_DOT_FRACTION, HALFTONE_MAX_DOT_FRACTION, saturate(tone));
    let radius = HALFTONE_CELL_NDC * 0.5 * dot_fraction;

    let distance_from_center = length(corrected - cell_center_corrected);
    let coverage = 1.0 - smoothstep(
        radius - HALFTONE_EDGE_SOFTNESS_NDC,
        radius + HALFTONE_EDGE_SOFTNESS_NDC,
        distance_from_center,
    );
    return mix(cell_color * HALFTONE_BACKGROUND_WEIGHT, cell_color, coverage);
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
fn fs_main(input: VertexOutput) -> FragmentOutput {
    let warped_ndc = texture_ndc_for_screen(input.ndc);
    let sample_ndc = select(warped_ndc, input.ndc, warp_settings.debug_view != 0u);
    let uv = texture_uv(sample_ndc);
    var color = textureSampleLevel(warp_color, warp_sampler, uv, 0.0);
    if warp_settings.debug_view != 0u {
        return FragmentOutput(color, 0.0);
    }
    if (warp_settings.experiment_flags & EXPERIMENT_RADIAL_BLUR) != 0u {
        let screen_offset = input.ndc - warp_settings.fovea_ndc;
        let eccentricity = length(warped_ndc);
        let blur_distance = smoothstep(0.35, 1.0, eccentricity) * 0.018;
        if blur_distance > 0.0 && length(screen_offset) > 1.0e-4 {
            let radial = normalize(screen_offset);
            let inner_screen = clamp(
                input.ndc - radial * blur_distance,
                vec2<f32>(-1.0),
                vec2<f32>(1.0),
            );
            let outer_screen = clamp(
                input.ndc + radial * blur_distance,
                vec2<f32>(-1.0),
                vec2<f32>(1.0),
            );
            let inner = textureSampleLevel(
                warp_color,
                warp_sampler,
                texture_uv(texture_ndc_for_screen(inner_screen)),
                0.0,
            );
            let outer = textureSampleLevel(
                warp_color,
                warp_sampler,
                texture_uv(texture_ndc_for_screen(outer_screen)),
                0.0,
            );
            color = inner * 0.25 + color * 0.5 + outer * 0.25;
        }
    }
    if (warp_settings.experiment_flags & EXPERIMENT_HALFTONE) != 0u {
        color = vec4<f32>(halftone_color(input.ndc), color.a);
    }

    let dimensions = textureDimensions(warp_distance);
    let texel = clamp(
        vec2<i32>(floor(uv * vec2<f32>(dimensions))),
        vec2<i32>(0),
        vec2<i32>(dimensions) - vec2<i32>(1),
    );
    let distance_meters = textureLoad(warp_distance, texel, 0).x;
    if distance_meters < 0.0 {
        return FragmentOutput(color, 0.0);
    }
    let ray = view_direction(input.ndc);
    let clip = camera.projection_matrix
        * vec4<f32>(ray * distance_meters, 1.0);
    return FragmentOutput(color, clip.z / clip.w);
}

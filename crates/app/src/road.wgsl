// Same camera prefix and flat sunlight/hemisphere convention as village.wgsl.
// The pilot only binds the camera. Streamed roads also query the resident
// excavation field to clip internal bore walls and overlapping cut banks.
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
@group(0) @binding(0) var<uniform> camera: Camera;

struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec3<f32>,
    @location(3) origin: vec4<f32>,
    @location(4) across: vec4<f32>,
    @location(5) forward: vec3<f32>,
    @location(6) up: vec3<f32>,
}
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) normal: vec3<f32>,
    @location(1) @interpolate(flat) colour: vec3<f32>,
    @location(2) @interpolate(flat) up: vec3<f32>,
    @location(3) @interpolate(linear) coverage: vec4<f32>,
    @location(4) @interpolate(flat) sheltered: f32,
    @location(5) local_position: vec3<f32>,
    @location(6) @interpolate(flat) network_kind: f32,
    @location(7) removed: f32,
}

fn project_vertex(input: Input) -> Output {
    let relative = input.origin.xyz + input.across.xyz * input.position.x
        + input.forward * input.position.y + input.up * input.position.z;
    let view = vec3<f32>(dot(relative, camera.camera_right.xyz),
        dot(relative, camera.camera_up.xyz), -dot(relative, camera.camera_forward.xyz));
    var output: Output;
    output.position = camera.projection_matrix * vec4<f32>(view, 1.0);
    output.normal = normalize(input.across.xyz * input.normal.x
        + input.forward * input.normal.y + input.up * input.normal.z);
    output.colour = abs(input.colour);
    output.local_position = input.position;
    output.network_kind = select(select(0.0, 1.0, input.colour.y<0.0 && input.colour.z<0.0),
        2.0, input.colour.y<0.0 && input.colour.z>=0.0);
    output.removed=max(0.0,(-input.colour.y-0.19)*64.0);
    if output.network_kind>1.5 {output.colour=vec3<f32>(0.24,0.19,0.13);}
    // Signed colour lanes avoid increasing the cached vertex stride:
    // negative blue = sheltered; negative red = emissive marker (near mesh).
    output.sheltered = select(select(0.0, 1.0, input.colour.z < 0.0), 2.0, input.colour.x < 0.0);
    // An outward-facing tunnel shell is lit outdoors, not an emissive marker.
    if all(input.colour<vec3<f32>(0.0)) {output.network_kind=3.0;output.sheltered=0.0;}
    output.up = input.up;
    output.coverage = vec4<f32>(0.0);
    // Road paint carries its signed half-width in the normal lane. A strip
    // thinner than ~1.6px is widened so distant lines stay continuous
    // instead of breaking into isolated pixels.
    if all(abs(output.colour - vec3<f32>(0.65, 0.65, 0.61)) < vec3<f32>(0.004))
        && output.network_kind < 0.5 && output.position.w > 0.05 {
        let half = input.across.xyz * input.normal.x
            + input.forward * input.normal.y + input.up * input.normal.z;
        let viewport = max(vec2<f32>(input.origin.w, input.across.w), vec2<f32>(1.0));
        let half_px = length(pixel_vector(half, output.position, viewport));
        output.normal = input.up;
        if half_px > 0.000001 && half_px < 0.8 {
            let widened = relative + half * (min(0.8 / half_px, 60.0) - 1.0);
            let view2 = vec3<f32>(dot(widened, camera.camera_right.xyz),
                dot(widened, camera.camera_up.xyz), -dot(widened, camera.camera_forward.xyz));
            output.position = camera.projection_matrix * vec4<f32>(view2, 1.0);
        }
    }
    return output;
}

const PAINT_GLOW = vec3<f32>(0.08, 0.60, 0.18);

fn lit_colour(input: Output) -> vec3<f32> {
    let sun = normalize(camera.sun_direction.xyz);
    let elevation = clamp(dot(sun, input.up), 0.0, 1.0);
    let direct = vec3<f32>(1.9, 1.78, 1.6) * elevation * max(dot(input.normal, sun), 0.0);
    let ambient = mix(vec3<f32>(0.05, 0.055, 0.06), vec3<f32>(0.24, 0.29, 0.36),
        0.5 + 0.5 * dot(input.normal, input.up)) * (0.25 + 0.75 * elevation);
    if input.sheltered > 1.5 { return input.colour; }
    let vertical = dot(input.normal,input.up);
    let interior = (0.10 + 0.04*(1.0-abs(vertical)) + 0.12*max(vertical,0.0))
        * vec3<f32>(1.0,0.96,0.90);
    let lit = input.colour * mix(direct + ambient, interior, input.sheltered);
    // White road paint (colour 0.65,0.65,0.61) also phosphoresces green;
    // whichever of the lit paint and the glow is brighter is what shows.
    let is_paint = all(abs(input.colour - vec3<f32>(0.65, 0.65, 0.61)) < vec3<f32>(0.004));
    if is_paint {
        let luma = vec3<f32>(0.2126, 0.7152, 0.0722);
        if dot(PAINT_GLOW, luma) > dot(lit, luma) { return PAINT_GLOW; }
    }
    return lit;
}

@vertex
fn vs_main(input: Input) -> Output {
    return project_vertex(input);
}

@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    /* NETWORK_EXCAVATION_CLIP */
    return vec4<f32>(lit_colour(input), 1.0);
}

fn view_vector(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(v, camera.camera_right.xyz),
        dot(v, camera.camera_up.xyz), -dot(v, camera.camera_forward.xyz));
}

fn pixel_vector(v: vec3<f32>, clip: vec4<f32>, viewport: vec2<f32>) -> vec2<f32> {
    let delta = camera.projection_matrix * vec4<f32>(view_vector(v), 0.0);
    return (delta.xy * clip.w - clip.xy * delta.w)
        / max(clip.w * clip.w, 0.000001) * viewport * 0.5;
}

@vertex
fn vs_far(input: Input) -> Output {
    var output = project_vertex(input);
    // The far mesh stores a half-width longitudinal tangent rather than a normal.
    let tangent = input.across.xyz * input.normal.x
        + input.forward * input.normal.y + input.up * input.normal.z;
    let across = input.across.xyz * input.normal.y - input.forward * input.normal.x;
    let viewport = max(vec2<f32>(input.origin.w, input.across.w), vec2<f32>(1.0));
    let side = sign(input.colour.x);
    if output.position.w > max(1.0, 2.0 * length(tangent)) {
        let along_px = pixel_vector(tangent, output.position, viewport);
        let across_px = pixel_vector(across, output.position, viewport);
        var n = vec2<f32>(along_px.y, -along_px.x) / max(length(along_px), 0.000001);
        n *= select(-1.0, 1.0, dot(n, across_px) >= 0.0);
        let half_width = abs(dot(n, across_px));
        let extent = half_width + 1.0;
        output.position = vec4<f32>(output.position.xy
            + side * n * extent * 2.0 / viewport * output.position.w,
            output.position.zw);
        output.coverage = vec4<f32>(side * extent, half_width, n);
    } else {
        // Perspective's derivative is singular at the eye plane. A road far
        // offscreen can otherwise expand across the entire image. Use its
        // real physical quad here and let homogeneous clipping handle it.
        var physical = input;
        physical.position += side * vec3<f32>(input.normal.y, -input.normal.x, 0.0);
        output = project_vertex(physical);
        output.coverage = vec4<f32>(0.0, 1000000.0, 1.0, 0.0);
    }
    output.normal = normalize(cross(across, tangent));
    output.colour = abs(input.colour);
    output.sheltered = 0.0;
    return output;
}

// CDF of a unit pixel square projected onto the strip normal. Integrating
// this box filter conserves coverage even for diagonal sub-pixel roads.
fn pixel_cdf(x: f32, n: vec2<f32>) -> f32 {
    let major = max(max(abs(n.x), abs(n.y)), 0.000001);
    let minor = min(abs(n.x), abs(n.y));
    if minor < 0.001 {
        return clamp(x / major + 0.5, 0.0, 1.0);
    }
    let t = x + 0.5 * (major + minor);
    if t <= 0.0 { return 0.0; }
    if t >= major + minor { return 1.0; }
    if t < minor { return t * t / (2.0 * major * minor); }
    if t < major { return (t - 0.5 * minor) / major; }
    let remaining = major + minor - t;
    return 1.0 - remaining * remaining / (2.0 * major * minor);
}

@fragment
fn fs_far(input: Output) -> @location(0) vec4<f32> {
    let distance = input.coverage.x;
    let half_width = input.coverage.y;
    let n = input.coverage.zw;
    let coverage = clamp(pixel_cdf(half_width - distance, n)
        - pixel_cdf(-half_width - distance, n), 0.0, 1.0);
    // Premultiplied coverage; never write depth for the widened fringe.
    return vec4<f32>(lit_colour(input) * coverage, coverage);
}

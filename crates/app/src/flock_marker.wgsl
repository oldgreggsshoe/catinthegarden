// A screen-space reticle over the nearest flock: a filled square with a tick
// out to each side and one above and below.
//
// Drawn with the depth test disabled on purpose, so it stays visible through
// terrain and through the birds themselves. It is a marker, not an object in
// the world: a flock behind a dune is exactly the case worth marking.
//
// A standalone pass like the ship's, redeclaring the camera struct rather than
// including shared_planet.wgsl, whose terrain and atmosphere bindings it has no
// use for.

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

struct Marker {
    // The flock's centre relative to the camera, rotated into view axes on the
    // CPU in f64. A planet-absolute position would arrive quantised.
    view_position: vec4<f32>,
    // xy: viewport size in pixels. z: 1 when there is a flock to mark at all.
    viewport: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> marker: Marker;

// Bright enough to read as a marker after tonemapping rather than as a dull
// maroon, and almost entirely in red so it cannot be mistaken for anything the
// sky or the sand is doing.
const MARKER_COLOUR: vec3<f32> = vec3<f32>(4.0, 0.03, 0.03);

struct VertexInput {
    // Offset from the marked point, in pixels. Constant on screen, so the
    // reticle does not shrink as the flock flies away.
    @location(0) offset_pixels: vec2<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> @builtin(position) vec4<f32> {
    // Nothing to mark, or the flock is behind the camera. Collapse the
    // geometry outside the clip volume instead of branching the draw call.
    if marker.viewport.z < 0.5 || marker.view_position.z >= -0.001 {
        return vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    let clip = camera.projection_matrix * vec4<f32>(marker.view_position.xyz, 1.0);
    // A flock in front of the camera is very often outside a 54 degree frame,
    // and a reticle that simply vanishes then is no use for finding birds.
    // Pin it to the edge instead, where it says which way to turn.
    let centre = clamp(clip.xy / clip.w, vec2<f32>(-0.93), vec2<f32>(0.93));
    // Pixels to normalised device coordinates. Y is negated because pixel space
    // counts downward and clip space counts up.
    let to_ndc = vec2<f32>(2.0 / marker.viewport.x, -2.0 / marker.viewport.y);
    // Depth 0.0 is the far plane under reverse-Z, which is where a marker that
    // is not depth-tested belongs: it writes no depth and occludes nothing.
    return vec4<f32>(centre + input.offset_pixels * to_ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(MARKER_COLOUR, 1.0);
}

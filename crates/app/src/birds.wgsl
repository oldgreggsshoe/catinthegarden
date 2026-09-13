// Flat-shaded low-poly birds, one instanced draw for every flock in range.
// A standalone pass like the ship's: it redeclares the camera struct rather
// than including shared_planet.wgsl, whose terrain and atmosphere bindings this
// pass has no use for.

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

const TAU: f32 = 6.2831853;
/// Half-stroke of a wingbeat, in radians about the bird's own forward axis.
const FLAP_AMPLITUDE: f32 = 0.85;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // x: wing side, -1 left, 0 body, +1 right. y: spanwise fraction, 0 at the
    // root and 1 at the tip, which is also what weights the fold.
    @location(2) flap: vec2<f32>,
    @location(3) colour: vec3<f32>,
}

struct InstanceInput {
    // The bird's centre relative to the camera, already rotated into view axes
    // on the CPU in f64. A planet-absolute position would arrive here with a
    // quarter-metre of f32 quantisation at a 4,000km radius, which is most of a
    // bird.
    @location(4) view_position: vec3<f32>,
    // Heading and local up, in planet-local axes.
    @location(5) forward: vec3<f32>,
    @location(6) up: vec3<f32>,
    // x: wingbeat phase in turns. y: how folded the wings are, 1 when walking.
    // z: body length in metres.
    @location(7) motion: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    // Flat, because the bird is meant to read as facets at close range and as
    // a silhouette at any distance. Interpolating would round both off.
    @location(0) @interpolate(flat) normal: vec3<f32>,
    @location(1) @interpolate(flat) colour: vec3<f32>,
    @location(2) @interpolate(flat) up: vec3<f32>,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

@vertex
fn vs_main(input: VertexInput, instance: InstanceInput) -> VertexOutput {
    let side = input.flap.x;
    let span = input.flap.y;
    let phase = instance.motion.x;
    let fold = clamp(instance.motion.y, 0.0, 1.0);
    let scale = instance.motion.z;

    // Folding draws the tip inboard and a little aft, so a walking bird carries
    // its wings against its body instead of holding them out mid-stroke.
    var local = input.position;
    local.x = local.x * (1.0 - 0.72 * fold * span);
    local.z = local.z - 0.10 * fold * span;

    // Hinge the wings about the bird's forward axis. Body vertices carry
    // span 0, so this is the identity for them without a branch.
    let flap_angle = sin(phase * TAU) * FLAP_AMPLITUDE * span * (1.0 - fold) * side;
    let flap_cos = cos(flap_angle);
    let flap_sin = sin(flap_angle);
    let hinged = vec3<f32>(
        local.x * flap_cos - local.y * flap_sin,
        local.x * flap_sin + local.y * flap_cos,
        local.z,
    );
    let hinged_normal = vec3<f32>(
        input.normal.x * flap_cos - input.normal.y * flap_sin,
        input.normal.x * flap_sin + input.normal.y * flap_cos,
        input.normal.z,
    );

    // Orthonormal bird frame: +Z forward, +Y up, +X to the bird's right.
    let forward = normalize(instance.forward);
    let right = normalize(cross(instance.up, forward));
    let up = cross(forward, right);

    let planet_offset = (right * hinged.x + up * hinged.y + forward * hinged.z) * scale;
    let view_position = instance.view_position + planet_to_view(planet_offset);

    var output: VertexOutput;
    output.position = camera.projection_matrix * vec4<f32>(view_position, 1.0);
    output.normal = normalize(
        right * hinged_normal.x + up * hinged_normal.y + forward * hinged_normal.z,
    );
    output.colour = input.colour;
    output.up = normalize(instance.up);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sun_direction = normalize(camera.sun_direction.xyz);
    let normal = normalize(input.normal);
    // Wings are thin and lit from both faces; without this a bird overhead is a
    // black cutout against the sky whenever the sun is behind it.
    let sun_lambert = abs(dot(normal, sun_direction));
    // Sun strength follows its own elevation, so birds darken with the ground
    // under them at low sun instead of staying lit against a dusk horizon. Same
    // treatment as the ship's hull.
    let sun_elevation = clamp(dot(sun_direction, input.up), 0.0, 1.0);
    let sunlight = vec3<f32>(1.9, 1.78, 1.6) * sun_elevation;
    // Hemispheric ambient: sky above, a dimmer bounce off the ground below.
    let sky_facing = 0.5 + 0.5 * dot(normal, input.up);
    let sky_light = mix(
        vec3<f32>(0.07, 0.075, 0.08),
        vec3<f32>(0.28, 0.33, 0.40),
        sky_facing,
    ) * (0.25 + 0.75 * sun_elevation);
    let lit = input.colour * (sunlight * sun_lambert + sky_light);
    return vec4<f32>(lit, 1.0);
}

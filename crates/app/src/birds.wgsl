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
/// Half-stroke of the arm, in radians about the bird's own forward axis.
const SHOULDER_AMPLITUDE: f32 = 0.80;
/// The hand adds its own stroke on top of the arm's.
const WRIST_AMPLITUDE: f32 = 0.70;
/// How far behind the arm the hand runs, in turns. This lag is the whole
/// difference between a bird and a dragonfly: one rigid plate pivoting at the
/// shoulder is an insect, and a wing whose hand trails its arm reads as a bird.
/// It is what produces the M at the top of the beat and the swept look at the
/// bottom.
const WRIST_LAG_TURNS: f32 = 0.16;
/// Where the hand is hinged, in the bird's own local frame. Matches
/// `WING_WRIST_FRACTION` and the wrist vertices in `birds::build_mesh`.
const WRIST_LOCAL: vec3<f32> = vec3<f32>(0.325, 0.048, -0.02);
/// How far the hand folds back against the body when a bird is walking.
const WRIST_FOLD_RADIANS: f32 = 2.0;
/// The shallow V a set wing holds, and the slight droop of the hand outboard of
/// the wrist that goes with it.
const GLIDE_DIHEDRAL_RADIANS: f32 = 0.16;
const GLIDE_WRIST_DROOP_RADIANS: f32 = 0.22;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // The wing rig: x is the side (-1 left, 0 body, +1 right), y the arm weight
    // (0 at the shoulder, 1 from the wrist out) and z the hand weight (0 on the
    // arm, 1 at the tip). See `birds::BirdVertex`.
    @location(2) flap: vec3<f32>,
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
    // z: body length in metres. w: bank, in radians, positive to the right.
    @location(7) motion: vec4<f32>,
    // How set the wings are: 0 beating, 1 fully gliding.
    @location(8) glide: f32,
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

/// Rotate about the bird's own forward axis, which is the axis both wing bones
/// hinge on.
fn hinge(vector: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(vector.x * c - vector.y * s, vector.x * s + vector.y * c, vector.z);
}

@vertex
fn vs_main(input: VertexInput, instance: InstanceInput) -> VertexOutput {
    let side = input.flap.x;
    let arm = input.flap.y;
    let hand = input.flap.z;
    let phase = instance.motion.x;
    let fold = clamp(instance.motion.y, 0.0, 1.0);
    let scale = instance.motion.z;

    // Folding draws the hand inboard and a little aft, so a walking bird carries
    // its wings against its body instead of holding them out mid-stroke.
    var local = input.position;
    local.x = local.x * (1.0 - 0.72 * fold * arm);
    local.z = local.z - 0.10 * fold * arm;

    // Gliding sets the wings: the stroke fades out and the arm settles into a
    // shallow dihedral, the shape a gull holds when it stops working. Without
    // the dihedral a glide reads as a bird frozen mid-beat rather than coasting.
    let glide = clamp(instance.glide, 0.0, 1.0);
    let beat = (1.0 - fold) * (1.0 - glide);
    let shoulder_angle = sin(phase * TAU) * SHOULDER_AMPLITUDE * beat
        + GLIDE_DIHEDRAL_RADIANS * glide * (1.0 - fold);
    let wrist_angle = sin((phase - WRIST_LAG_TURNS) * TAU) * WRIST_AMPLITUDE * beat
        - WRIST_FOLD_RADIANS * fold
        - GLIDE_WRIST_DROOP_RADIANS * glide * (1.0 - fold);

    // Bone one, hinged at the shoulder. Body vertices carry arm 0, so this is
    // the identity for them without a branch.
    let arm_rotation = shoulder_angle * side * arm;
    var hinged = hinge(local, arm_rotation);
    var hinged_normal = hinge(input.normal, arm_rotation);

    // Bone two, hinged at the wrist, which has itself been carried round by the
    // arm. Arm vertices carry hand 0, so again no branch is needed.
    let wrist_point = hinge(
        vec3<f32>(WRIST_LOCAL.x * side, WRIST_LOCAL.y, WRIST_LOCAL.z),
        shoulder_angle * side,
    );
    let hand_rotation = wrist_angle * side * hand;
    hinged = wrist_point + hinge(hinged - wrist_point, hand_rotation);
    hinged_normal = hinge(hinged_normal, hand_rotation);

    // Orthonormal bird frame: +Z forward, +Y up, +X to the bird's right.
    //
    // `instance.up` is the planetary radial, so a frame built from it alone is
    // dead level and the bird flies every turn flat, like a model on a wire.
    // Rolling it about its own forward axis is what makes a turn read as a
    // turn: the silhouette carries it, with the wings coming round into view on
    // the inside of the arc.
    let forward = normalize(instance.forward);
    let level_right = normalize(cross(instance.up, forward));
    let level_up = cross(forward, level_right);
    let bank = instance.motion.w;
    let bank_cos = cos(bank);
    let bank_sin = sin(bank);
    let right = level_right * bank_cos + level_up * bank_sin;
    let up = level_up * bank_cos - level_right * bank_sin;

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

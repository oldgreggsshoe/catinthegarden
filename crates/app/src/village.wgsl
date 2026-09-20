// Flat-shaded painted houses, drawn instanced.
//
// A standalone pass like the ship's: it redeclares the camera struct rather
// than including shared_planet.wgsl, whose terrain and atmosphere bindings this
// pass has no use for.
//
// Every house arrives as an offset from the camera, already differenced in f64
// on the CPU. A planet-absolute position would reach here with about half a
// metre of f32 quantisation at a 4,000km radius, which on a six-metre house is
// the difference between a wall and a staircase.

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

struct VertexInput {
    // House-local axes: x across the gable, y along the ridge, z up.
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // 0 = painted wall, 1 = roof. The roof keeps its slate whatever the walls
    // are painted, so a village reads as painted timber under dark roofs.
    @location(2) roof: f32,
    // Per-instance, in planet-local axes.
    @location(3) camera_relative_position: vec3<f32>,
    @location(4) up: vec3<f32>,
    @location(5) forward: vec3<f32>,
    @location(6) colour: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    // Flat: houses are meant to read as facets, and interpolating would round
    // the low-poly silhouette's shading back off.
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
fn vs_main(input: VertexInput) -> VertexOutput {
    // The house frame on the sphere: up is radial at the house, forward is its
    // horizontal facing, and across completes a right-handed set.
    let up = normalize(input.up);
    let forward = normalize(input.forward - up * dot(input.forward, up));
    let across = cross(forward, up);

    let local = across * input.position.x + forward * input.position.y + up * input.position.z;
    let planet_offset = input.camera_relative_position + local;
    let view_position = planet_to_view(planet_offset);

    let normal = across * input.normal.x + forward * input.normal.y + up * input.normal.z;

    var output: VertexOutput;
    output.position = camera.projection_matrix * vec4<f32>(view_position, 1.0);
    output.normal = normalize(normal);
    output.colour = select(input.colour, vec3<f32>(0.118, 0.110, 0.106), input.roof > 0.5);
    output.up = up;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sun_direction = normalize(camera.sun_direction.xyz);
    let normal = normalize(input.normal);
    let sun_lambert = max(dot(normal, sun_direction), 0.0);
    // Sunlight follows its own elevation at the house, so a village darkens
    // with the ground around it at low sun rather than staying lit against a
    // dusk horizon.
    let sun_elevation = clamp(dot(sun_direction, input.up), 0.0, 1.0);
    let sunlight = vec3<f32>(1.9, 1.78, 1.6) * sun_elevation;
    // Hemispheric ambient: sky above, a dimmer bounce off the ground below.
    let sky_facing = 0.5 + 0.5 * dot(normal, input.up);
    let sky_light = mix(
        vec3<f32>(0.05, 0.055, 0.06),
        vec3<f32>(0.24, 0.29, 0.36),
        sky_facing,
    ) * (0.25 + 0.75 * sun_elevation);
    let lit = input.colour * (sunlight * sun_lambert + sky_light);
    return vec4<f32>(lit, 1.0);
}

// ---------------------------------------------------------------------------
// Ground contact shadow.
//
// One blended fan per house, lying in the house's own tangent plane. It is not
// occlusion in any real sense -- nothing is traced and nothing is sampled --
// but a house meeting the ground with no darkening at all reads as a model
// resting on a texture, and this is the cheapest thing that fixes it.

struct ShadowVertexInput {
    // Across the gable and along the ridge, in metres from the ground point.
    @location(0) offset: vec2<f32>,
    // 0 at the house centre, 1 at the fan's rim.
    @location(1) rim_share: f32,
    // The same instance stream the houses use, so a shadow cannot drift from
    // the house it belongs to.
    @location(2) camera_relative_position: vec3<f32>,
    @location(3) up: vec3<f32>,
    @location(4) forward: vec3<f32>,
}

struct ShadowVertexOutput {
    @builtin(position) position: vec4<f32>,
    // Distance from the house centre as a share of the fan, for the falloff.
    @location(0) rim_share: f32,
}

@vertex
fn vs_ground_shadow(input: ShadowVertexInput) -> ShadowVertexOutput {
    let up = normalize(input.up);
    let forward = normalize(input.forward - up * dot(input.forward, up));
    let across = cross(forward, up);

    let local = across * input.offset.x + forward * input.offset.y
        + up * GROUND_SHADOW_LIFT_METERS;
    let view_position = planet_to_view(input.camera_relative_position + local);

    var output: ShadowVertexOutput;
    output.position = camera.projection_matrix * vec4<f32>(view_position, 1.0);
    output.rim_share = input.rim_share;
    return output;
}

@fragment
fn fs_ground_shadow(input: ShadowVertexOutput) -> @location(0) vec4<f32> {
    // Solid under the walls, fading across the strip that is actually visible.
    let fade = 1.0 - smoothstep(GROUND_SHADOW_FADE_START, 1.0, input.rim_share);
    let amount = clamp(GROUND_SHADOW_STRENGTH * fade, 0.0, 1.0);
    // Black at `amount` alpha: straight alpha blending multiplies the ground
    // down without tinting it, so no material can be pushed off its hue.
    return vec4<f32>(0.0, 0.0, 0.0, amount);
}

// ---------------------------------------------------------------------------
// Village locator beams.
//
// A translucent shaft standing on each village and reaching out of the
// atmosphere, drawn at a constant screen width so a settlement stays findable
// from orbit rather than shrinking to nothing. Debug presentation: it lights
// nothing and is off by default.

struct BeamVertexInput {
    // x picks the side of the ribbon, y runs from the ground to the top.
    @location(0) uv: vec2<f32>,
    @location(1) camera_relative_base: vec3<f32>,
    @location(2) up: vec3<f32>,
}

struct BeamVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) solar_elevation: f32,
}

@vertex
fn vs_beam(input: BeamVertexInput) -> BeamVertexOutput {
    let up = normalize(input.up);
    let bottom_view = planet_to_view(input.camera_relative_base);
    let top_view = planet_to_view(input.camera_relative_base + up * BEAM_LENGTH_METERS);
    let bottom_clip = camera.projection_matrix * vec4<f32>(bottom_view, 1.0);
    let top_clip = camera.projection_matrix * vec4<f32>(top_view, 1.0);
    // Behind the eye a clip w of zero sends the ribbon to infinity; clamping
    // keeps the screen-space direction finite instead.
    let bottom_w = select(bottom_clip.w, 1.0e-4, abs(bottom_clip.w) < 1.0e-4);
    let top_w = select(top_clip.w, 1.0e-4, abs(top_clip.w) < 1.0e-4);
    let screen_aspect = vec2<f32>(max(camera.projection.x, 1.0e-4), 1.0);
    var line_screen = (top_clip.xy / top_w - bottom_clip.xy / bottom_w) * screen_aspect;
    if dot(line_screen, line_screen) < 1.0e-8 {
        line_screen = vec2<f32>(0.0, 1.0);
    }
    let line_direction = normalize(line_screen);
    let perpendicular = vec2<f32>(-line_direction.y, line_direction.x);
    // Widened in NDC rather than in metres, which is what keeps the beam the
    // same thickness whether the village is 200m or 2,000km away.
    let side = input.uv.x * 2.0 - 1.0;
    let offset_ndc = perpendicular / screen_aspect * (side * BEAM_SCREEN_HALF_WIDTH);
    var clip_position = mix(bottom_clip, top_clip, input.uv.y);
    clip_position.x += offset_ndc.x * clip_position.w;
    clip_position.y += offset_ndc.y * clip_position.w;

    var output: BeamVertexOutput;
    output.position = clip_position;
    output.uv = input.uv;
    output.solar_elevation = dot(up, normalize(camera.sun_direction.xyz));
    return output;
}

@fragment
fn fs_beam(input: BeamVertexOutput) -> @location(0) vec4<f32> {
    // Soft edges across the ribbon, and a fade with height so the shaft reads
    // as rising out of the ground rather than as a hard bar.
    let edge = smoothstep(0.0, 0.24, input.uv.x) * (1.0 - smoothstep(0.76, 1.0, input.uv.x));
    let height_fade = 1.0 - 0.35 * smoothstep(0.15, 1.0, input.uv.y);
    let alpha = edge * height_fade * BEAM_ALPHA;
    if alpha < 0.002 {
        discard;
    }
    // Dimmer on the night side, so a beam does not glow out of dark ground.
    let sun = smoothstep(-0.12, 0.18, input.solar_elevation);
    return vec4<f32>(vec3<f32>(1.0, 0.86, 0.62) * (0.75 + sun * 0.75), alpha);
}

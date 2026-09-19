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

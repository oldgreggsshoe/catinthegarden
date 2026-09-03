// Flat-shaded low-poly ship. A standalone pass: it redeclares the camera
// struct rather than including shared_planet.wgsl, whose group(2) terrain and
// atmosphere bindings this pass has no use for.

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

struct ShipUniform {
    // The hull's local origin relative to the camera, already rotated into
    // view axes on the CPU in f64. A planet-absolute position would arrive
    // here with half a metre of f32 quantisation at a 4,000km radius.
    view_position: vec4<f32>,
    // Ship-local axes expressed in planet-local axes, one per column.
    orientation_x: vec4<f32>,
    orientation_y: vec4<f32>,
    orientation_z: vec4<f32>,
    // Local up at the hull, for the sky/ground ambient split.
    up: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> ship: ShipUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    // Flat, because the hull is meant to read as facets. Interpolating these
    // would round the low-poly silhouette's shading back off.
    @location(0) @interpolate(flat) normal: vec3<f32>,
    @location(1) @interpolate(flat) colour: vec3<f32>,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

fn ship_to_planet(vector: vec3<f32>) -> vec3<f32> {
    return ship.orientation_x.xyz * vector.x
        + ship.orientation_y.xyz * vector.y
        + ship.orientation_z.xyz * vector.z;
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let planet_offset = ship_to_planet(input.position);
    let view_position = ship.view_position.xyz + planet_to_view(planet_offset);
    var output: VertexOutput;
    output.position = camera.projection_matrix * vec4<f32>(view_position, 1.0);
    output.normal = normalize(ship_to_planet(input.normal));
    output.colour = input.colour;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sun_direction = normalize(camera.sun_direction.xyz);
    let normal = normalize(input.normal);
    let sun_lambert = max(dot(normal, sun_direction), 0.0);
    // Sun strength follows its own elevation, so the hull darkens with the sea
    // around it at low sun instead of staying lit against a dusk horizon.
    let sun_elevation = clamp(dot(sun_direction, ship.up.xyz), 0.0, 1.0);
    let sunlight = vec3<f32>(1.9, 1.78, 1.6) * sun_elevation;
    // Hemispheric ambient: sky from above, a dimmer bounce off the water below.
    let sky_facing = 0.5 + 0.5 * dot(normal, ship.up.xyz);
    let sky_light = mix(
        vec3<f32>(0.06, 0.075, 0.09),
        vec3<f32>(0.26, 0.32, 0.40),
        sky_facing,
    ) * (0.25 + 0.75 * sun_elevation);
    let lit = input.colour * (sunlight * sun_lambert + sky_light);
    return vec4<f32>(lit, 1.0);
}

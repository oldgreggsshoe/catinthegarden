// The cart's orb: a self-lit violet sphere with a soft halo and two glowing
// threads back to the cart. Same camera prefix as road_signs.wgsl; vertices
// arrive camera-relative in the planet frame.
struct Camera {
    projection_matrix: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec4<f32>,
}
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) relative: vec3<f32>,
    @location(2) @interpolate(flat) colour: vec4<f32>,
}
const VIOLET = vec3<f32>(0.50, 0.22, 1.0);
const CORE = vec3<f32>(0.85, 0.62, 1.0);
@vertex fn vs_main(v: Input) -> Output {
    let r = v.position;
    let view = vec3<f32>(dot(r, camera.camera_right.xyz), dot(r, camera.camera_up.xyz),
        -dot(r, camera.camera_forward.xyz));
    var out: Output;
    out.position = camera.projection_matrix * vec4<f32>(view, 1.0);
    out.normal = v.normal;
    out.relative = r;
    out.colour = v.colour;
    return out;
}
@fragment fn fs_orb(v: Output) -> @location(0) vec4<f32> {
    let n = normalize(v.normal);
    let facing = clamp(dot(n, normalize(-v.relative)), 0.0, 1.0);
    // Hot lilac heart, violet toward the limb; a faint glassy sheen remains
    // when the motor is off and the glow has sunk to an ember.
    // Scaled so a fully charged, hauling orb (glow 1.5) stays under the tone
    // curve's white: it should read violet, not as a blown-out disc.
    let light = mix(VIOLET, CORE, pow(facing, 3.0)) * v.colour.x * (0.25 + 0.55 * facing);
    let sheen = vec3<f32>(0.04, 0.035, 0.05) * (0.3 + 0.7 * (1.0 - facing) * (1.0 - facing));
    return vec4<f32>(light + sheen, 1.0);
}
@fragment fn fs_glow(v: Output) -> @location(0) vec4<f32> {
    var strength = 0.0;
    if v.colour.w < 1.5 {
        // Halo, zero at the square's edge so no outline shows.
        let r2 = dot(v.normal.xy, v.normal.xy);
        strength = max(exp(-4.0 * r2) - exp(-4.0), 0.0) * 0.35;
    } else {
        // Thread, brightest along its middle.
        strength = (1.0 - v.normal.x * v.normal.x) * 0.4;
    }
    return vec4<f32>(VIOLET * v.colour.x * strength, 0.0);
}

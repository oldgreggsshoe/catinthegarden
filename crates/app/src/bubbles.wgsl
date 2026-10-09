// Tiny bubbles in the water around an eye under the sea (bubbles.rs). Each is
// a point in a box of water around the eye, carried by the water, rising at
// its own speed and wrapped so the box never runs out; it is drawn as the
// streak it makes in one exposure, along its velocity relative to the eye, so
// they hold still while the eye drifts with the water and stream past when it
// moves through it. Composed after shared_planet.wgsl; BUBBLE_COUNT comes
// from bubbles.rs.

struct BubbleUniform {
    // View-space east, north and up of the water box at the camera.
    // w: box size (m), visibility 0-1, exposure (s).
    east: vec4<f32>,
    north: vec4<f32>,
    up: vec4<f32>,
    // xy: the box's east/north offset (m, 0 to the box size); zw: viewport (px).
    scroll: vec4<f32>,
    // Vertical offset of each of the four bubble sizes (m, 0 to the box size).
    rise_scroll: vec4<f32>,
    // Their rise speeds (m/s).
    rise_speed: vec4<f32>,
    // xyz: the water's velocity relative to the eye, view space (m/s).
    // w: pixels per unit of view-space slope.
    drift: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> bubbles: BubbleUniform;

// Diameters of the four bubble sizes (m): fine sub-millimetre ones up to the
// odd couple of millimetres.
const BUBBLE_DIAMETERS: vec4<f32> = vec4<f32>(0.0006, 0.001, 0.0015, 0.0022);
// Bubbles are never drawn thinner than this; one thinner than a pixel keeps
// its share of the pixel as opacity instead, so distant ones thin out.
const BUBBLE_MIN_WIDTH_PIXELS: f32 = 1.5;
const BUBBLE_OPACITY: f32 = 0.9;
// A bubble's surface mirrors the light coming down into the water (it is
// silvery, not the water's colour), so it reads as a pale speck against dark
// open water and bright shallows alike: this share of that light.
const BUBBLE_BRIGHTNESS: f32 = 0.9;
// Keep streak ends this far in front of the eye (view-space z).
const BUBBLE_NEAR_METERS: f32 = 0.05;

struct BubbleVertex {
    @builtin(position) position: vec4<f32>,
    // -1 to 1 across the streak.
    @location(0) across: f32,
    @location(1) @interpolate(flat) colour: vec4<f32>,
}

fn bubble_hash(value: u32) -> u32 {
    var x = value;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

fn bubble_random(seed: u32) -> f32 {
    return f32(bubble_hash(seed) & 0xffffffu) / 16777216.0;
}

fn bubble_culled() -> BubbleVertex {
    return BubbleVertex(vec4<f32>(0.0, 0.0, -1.0, 1.0), 0.0, vec4<f32>(0.0));
}

// The light coming down into the water at the eye, as the sea body is lit
// (`ocean_underwater_medium_colour` without the water's albedo).
fn bubble_light() -> vec3<f32> {
    let up = normalize(view_to_planet(camera.camera_planet_direction_view_altitude.xyz));
    let sun_direction = normalize(camera.sun_direction.xyz);
    return ocean_sot_body_light(
        surface_direct_sun_transmittance(up, 0.0, sun_direction) * sun_visible_fraction(),
        sky_diffuse_irradiance(up, up, 0.0, sun_direction),
    );
}

@vertex
fn vs_bubble(@builtin(vertex_index) vertex_index: u32) -> BubbleVertex {
    let bubble = vertex_index / 6u;
    let corner = vertex_index % 6u;
    let size = bubbles.east.w;
    let visibility = bubbles.north.w;
    if visibility <= 0.0 {
        return bubble_culled();
    }
    let seed = bubble_hash(bubble * 0x9e3779b9u + 0x2c1b3c6du);
    let random = vec4<f32>(
        bubble_random(seed ^ 0x68e31da4u),
        bubble_random(seed ^ 0xb5297a4du),
        bubble_random(seed ^ 0x1b56c4e9u),
        bubble_random(seed ^ 0x7f4a7c15u),
    );
    let size_class = bubble & 3u;
    let offset = vec3<f32>(bubbles.scroll.xy, bubbles.rise_scroll[size_class]);
    let in_box = (fract(random.xyz + offset / size) - 0.5) * size;
    let centre = bubbles.east.xyz * in_box.x + bubbles.north.xyz * in_box.y
        + bubbles.up.xyz * in_box.z;
    let distance = length(centre);
    // Out to the sphere inside the box, so its corners never show; and not
    // right against the eye.
    let fade = visibility * smoothstep(0.08, 0.3, distance)
        * (1.0 - smoothstep(0.3 * size, 0.5 * size, distance));
    if fade <= 0.0 {
        return bubble_culled();
    }
    let velocity = bubbles.drift.xyz + bubbles.up.xyz * bubbles.rise_speed[size_class];
    var head = centre;
    var tail = centre - velocity * bubbles.up.w;
    let near = -BUBBLE_NEAR_METERS;
    if head.z > near && tail.z > near {
        return bubble_culled();
    }
    if head.z > near {
        head = mix(tail, head, (near - tail.z) / (head.z - tail.z));
    } else if tail.z > near {
        tail = mix(head, tail, (near - head.z) / (tail.z - head.z));
    }
    let head_clip = camera.projection_matrix * vec4<f32>(head, 1.0);
    let tail_clip = camera.projection_matrix * vec4<f32>(tail, 1.0);
    let half_viewport = 0.5 * bubbles.scroll.zw;
    let head_pixels = head_clip.xy / head_clip.w * half_viewport;
    let tail_pixels = tail_clip.xy / tail_clip.w * half_viewport;
    let along = head_pixels - tail_pixels;
    let length_pixels = length(along);
    let axis = select(vec2<f32>(0.0, 1.0), along / length_pixels, length_pixels > 1.0e-3);
    let side = vec2<f32>(-axis.y, axis.x);
    let true_width = BUBBLE_DIAMETERS[size_class] * bubbles.drift.w
        / max(-centre.z, BUBBLE_NEAR_METERS);
    let width = max(true_width, BUBBLE_MIN_WIDTH_PIXELS);
    // A bubble narrower than the drawn speck keeps only part of its opacity,
    // but less than its share of the width, so the field stays visible.
    let opacity = fade * BUBBLE_OPACITY * sqrt(min(true_width / width, 1.0)) * (0.5 + random.w);
    let at_head = corner == 1u || corner == 2u || corner == 4u;
    let across = select(-1.0, 1.0, corner == 2u || corner == 4u || corner == 5u);
    let end_pixels = select(tail_pixels, head_pixels, at_head)
        + axis * (select(-0.5, 0.5, at_head) * width)
        + side * (0.5 * across * width);
    let clip = select(tail_clip, head_clip, at_head);
    return BubbleVertex(
        vec4<f32>(end_pixels / half_viewport * clip.w, clip.z, clip.w),
        across,
        vec4<f32>(bubble_light() * BUBBLE_BRIGHTNESS, min(opacity, 1.0)),
    );
}

@fragment
fn fs_bubble(input: BubbleVertex) -> @location(0) vec4<f32> {
    let profile = 1.0 - input.across * input.across;
    return vec4<f32>(input.colour.rgb, input.colour.a * profile);
}

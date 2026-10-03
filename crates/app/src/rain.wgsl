// Rain around the camera in a storm (rain.rs). Each drop is a point in a box
// of air around the eye, carried by the wind, falling at its own terminal
// speed and wrapped so the box never runs out; it is drawn as the streak it
// makes in one exposure, along its velocity relative to the eye, so the wind
// slants it and the camera's own motion (a pitching bridge) tilts it.
// Composed after shared_planet.wgsl; RAIN_STREAKS and RAIN_ONSET_SPREAD come
// from rain.rs.

struct RainUniform {
    // View-space east, north and up of the air box at the camera.
    // w: box size (m), rain intensity 0-1, exposure (s).
    east: vec4<f32>,
    north: vec4<f32>,
    up: vec4<f32>,
    // xy: the box's east/north offset (m, 0 to the box size); zw: viewport (px).
    scroll: vec4<f32>,
    // Vertical offset of each of the four drop sizes (m, 0 to the box size).
    fall_scroll: vec4<f32>,
    // Their terminal fall speeds (m/s).
    fall_speed: vec4<f32>,
    // xyz: the air's velocity relative to the eye, view space (m/s).
    // w: pixels per unit of view-space slope (half the viewport height over
    // the tangent of half the vertical field of view).
    drift: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> rain: RainUniform;

// Apparent width of a falling drop. Drops are 1-5mm; motion blur and the eye
// widen the streak.
const RAIN_DROP_METERS: f32 = 0.008;
// Streaks are never drawn thinner than this; a drop thinner than a pixel keeps
// its share of the pixel as opacity instead, so distant rain thins out.
const RAIN_MIN_WIDTH_PIXELS: f32 = 1.0;
const RAIN_OPACITY: f32 = 0.6;
// A drop is a tiny lens showing the sky around it: drawn a little brighter
// than the horizon sky behind it (which is what a storm's fog turns the whole
// sky to), so rain reads as pale streaks against the dark sea and all but
// vanishes against the sky, as it does.
const RAIN_SKY_BRIGHTNESS: f32 = 1.25;
// Sunlight scattered forward through a drop, toward an eye looking sunward.
const RAIN_SUN_GLINT: f32 = 0.6;
// Keep streak ends this far in front of the eye (view-space z).
const RAIN_NEAR_METERS: f32 = 0.05;

struct RainVertex {
    @builtin(position) position: vec4<f32>,
    // -1 to 1 across the streak.
    @location(0) across: f32,
    // Colour and opacity, one per streak.
    @location(1) @interpolate(flat) colour: vec4<f32>,
}

fn rain_hash(value: u32) -> u32 {
    var x = value;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

fn rain_random(seed: u32) -> f32 {
    return f32(rain_hash(seed) & 0xffffffu) / 16777216.0;
}

// Outside the clip volume: all six corners the same, so nothing is drawn.
fn rain_culled() -> RainVertex {
    return RainVertex(vec4<f32>(0.0, 0.0, -1.0, 1.0), 0.0, vec4<f32>(0.0));
}

// Light a drop sends toward the eye looking along `direction_view`.
fn rain_light(direction_view: vec3<f32>) -> vec3<f32> {
    let up_view = rain.up.xyz;
    var level = direction_view - up_view * dot(direction_view, up_view);
    if dot(level, level) < 1.0e-6 {
        level = rain.east.xyz;
    }
    let horizon_ray = normalize(normalize(level) + 0.1 * up_view);
    let sky = storm_overcast_colour(physical_camera_sky_radiance(horizon_ray))
        * RAIN_SKY_BRIGHTNESS;
    let up = normalize(view_to_planet(up_view));
    let sun_direction = normalize(camera.sun_direction.xyz);
    let sun = surface_direct_sun_transmittance(up, 0.0, sun_direction)
        * (1.0 - STORM_SUN_BLOCK * storm_overcast());
    let toward_sun = pow(
        max(dot(direction_view, normalize(camera.sun_direction_view.xyz)), 0.0),
        12.0,
    );
    let radiance = sky + sun * (RAIN_SUN_GLINT * SURFACE_SUNLIGHT_SCALE) * toward_sun;
    let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec3<f32>(luminance);
}

@vertex
fn vs_rain(@builtin(vertex_index) vertex_index: u32) -> RainVertex {
    let streak = vertex_index / 6u;
    let corner = vertex_index % 6u;
    let size = rain.east.w;
    let gate = clamp(
        (rain.north.w - f32(streak) / RAIN_STREAKS) / RAIN_ONSET_SPREAD,
        0.0,
        1.0,
    );
    if gate <= 0.0 {
        return rain_culled();
    }
    let seed = rain_hash(streak * 0x9e3779b9u + 0x632be5abu);
    let random = vec4<f32>(
        rain_random(seed ^ 0x68e31da4u),
        rain_random(seed ^ 0xb5297a4du),
        rain_random(seed ^ 0x1b56c4e9u),
        rain_random(seed ^ 0x7f4a7c15u),
    );
    let size_class = streak & 3u;
    let offset = vec3<f32>(rain.scroll.xy, rain.fall_scroll[size_class]);
    let in_box = (fract(random.xyz + offset / size) - 0.5) * size;
    let centre = rain.east.xyz * in_box.x + rain.north.xyz * in_box.y + rain.up.xyz * in_box.z;
    let distance = length(centre);
    // Out to the sphere inside the box, so its corners never show; and not
    // right against the eye, where one drop would fill the screen.
    let fade = gate * smoothstep(0.3, 1.0, distance)
        * (1.0 - smoothstep(0.3 * size, 0.5 * size, distance));
    if fade <= 0.0 {
        return rain_culled();
    }
    let velocity = rain.drift.xyz - rain.up.xyz * rain.fall_speed[size_class];
    var head = centre;
    var tail = centre - velocity * rain.up.w;
    let near = -RAIN_NEAR_METERS;
    if head.z > near && tail.z > near {
        return rain_culled();
    }
    if head.z > near {
        head = mix(tail, head, (near - tail.z) / (head.z - tail.z));
    } else if tail.z > near {
        tail = mix(head, tail, (near - head.z) / (tail.z - head.z));
    }
    let head_clip = camera.projection_matrix * vec4<f32>(head, 1.0);
    let tail_clip = camera.projection_matrix * vec4<f32>(tail, 1.0);
    let half_viewport = 0.5 * rain.scroll.zw;
    let head_pixels = head_clip.xy / head_clip.w * half_viewport;
    let tail_pixels = tail_clip.xy / tail_clip.w * half_viewport;
    let along = head_pixels - tail_pixels;
    let length_pixels = length(along);
    let axis = select(vec2<f32>(0.0, 1.0), along / length_pixels, length_pixels > 1.0e-3);
    let side = vec2<f32>(-axis.y, axis.x);
    let true_width = RAIN_DROP_METERS * rain.drift.w / max(-centre.z, RAIN_NEAR_METERS);
    let width = max(true_width, RAIN_MIN_WIDTH_PIXELS);
    let opacity = fade * RAIN_OPACITY * min(true_width / width, 1.0) * (0.5 + random.w);
    // Two triangles, tail to head, each end capped by half the width.
    let at_head = corner == 1u || corner == 2u || corner == 4u;
    let across = select(-1.0, 1.0, corner == 2u || corner == 4u || corner == 5u);
    let end_pixels = select(tail_pixels, head_pixels, at_head)
        + axis * (select(-0.5, 0.5, at_head) * width)
        + side * (0.5 * across * width);
    let clip = select(tail_clip, head_clip, at_head);
    return RainVertex(
        vec4<f32>(end_pixels / half_viewport * clip.w, clip.z, clip.w),
        across,
        vec4<f32>(rain_light(centre / max(distance, 1.0e-3)), opacity),
    );
}

@fragment
fn fs_rain(input: RainVertex) -> @location(0) vec4<f32> {
    let profile = 1.0 - input.across * input.across;
    return vec4<f32>(input.colour.rgb, input.colour.a * profile);
}

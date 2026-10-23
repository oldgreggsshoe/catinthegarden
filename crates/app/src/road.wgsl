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
@group(1) @binding(0) var<storage, read> tunnel_lights: array<vec4<f32>>;

struct Input {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec3<f32>,
    @location(3) origin: vec4<f32>,
    @location(4) across: vec4<f32>,
    @location(5) forward: vec3<f32>,
    @location(6) up: vec3<f32>,
    @location(8) lighting: u32,
    @location(10) material_origin: vec3<f32>,
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
    @location(8) @interpolate(flat) lighting: u32,
    @location(9) @interpolate(flat) normal_local: vec3<f32>,
    @location(10) material_position: vec3<f32>,
}

// Bounded, smooth irregular flame flutter, independently seeded in world
// space. Shared by the diffuser and its illumination; never an on/off strobe.
fn lantern_hash(seed: u32) -> f32 {
    var x=seed;
    x^=x>>16u; x*=0x7feb352du; x^=x>>15u; x*=0x846ca68bu; x^=x>>16u;
    return f32(x & 0x00ffffffu)/16777216.0;
}
fn lantern_noise(t: f32, seed: u32) -> f32 {
    let tick=u32(floor(t));
    let f=fract(t);
    return mix(lantern_hash(seed^tick),lantern_hash(seed^(tick+1u)),f*f*(3.0-2.0*f));
}
fn lantern_light(seed: f32) -> vec3<f32> {
    let key=u32(seed*16777216.0);
    let time=max(camera.projection.z,0.0);
    // More visible oil-flame dips/flares, plus quick irregular flutter.
    // Range0.45..1.37, expected mean0.91 as before; never switches off.
    let strength=0.45+0.62*lantern_noise(time*1.7+seed*23.0,key)
        +0.30*lantern_noise(time*7.0+seed*71.0,key^0x85ebca6bu);
    let amber=mix(vec3<f32>(1.0,0.33,0.12),vec3<f32>(1.0,0.29,0.10),seed);
    // Shared by visible flames and their cast light: two halvings of the prior radiance.
    return amber*strength*0.25;
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
    if input.colour.x < -2.5 {output.colour=input.colour.z*lantern_light(input.colour.y);}
    output.local_position = input.position;
    // CPU wraps only the frame origin in f64. Do not wrap vertices: that
    // would interpolate across a tile boundary rather than through it.
    output.material_position = input.material_origin + input.across.xyz * input.position.x
        + input.forward * input.position.y + input.up * input.position.z;
    output.lighting = input.lighting;
    output.normal_local = normalize(input.normal);
    output.network_kind = select(select(0.0, 1.0, input.colour.y<0.0 && input.colour.z<0.0),
        2.0, input.colour.y<0.0 && input.colour.z>=0.0);
    output.removed=max(0.0,(-input.colour.y-0.19)*64.0);
    if output.network_kind>1.5 {output.colour=vec3<f32>(0.24,0.19,0.13);}
    // Signed colour lanes retain the existing material tags:
    // negative blue = sheltered; negative red = emissive marker (near mesh).
    output.sheltered = select(select(0.0, 1.0, input.colour.z < 0.0), 2.0, input.colour.x < 0.0);
    // An outward-facing tunnel shell is lit outdoors, not an emissive marker.
    if all(input.colour<vec3<f32>(0.0)) {output.network_kind=3.0;output.sheltered=0.0;}
    output.up = input.up;
    output.coverage = vec4<f32>(0.0);
    return output;
}

// Ultraviolet fluorescence of the timber: a dim violet that only shows where
// the ordinary lighting is nearly dark (night, unlit bores), never by day.
const WOOD_UV_GLOW = vec3<f32>(0.20, 0.04, 0.55) * 0.09;
const DECK_PLANK_ALBEDO = vec3<f32>(0.20, 0.115, 0.055);
const BEAM_ALBEDO = vec3<f32>(0.17, 0.10, 0.05);

// Immutable nearby fixtures, never a search through the planetary network.
// The finite soft cutoff removes light smoothly, without a bright/dark step.
fn tunnel_irradiance(input: Output) -> vec3<f32> {
    let base=(input.lighting & 0xffffu)*25u;
    let count=min(u32(tunnel_lights[base].x),24u);
    var light=vec3<f32>(0.0); // illumination comes from actual lanterns only
    for (var i=0u;i<count;i+=1u) {
        let lamp=tunnel_lights[base+1u+i];
        let delta=lamp.xyz-input.local_position;
        let distance=length(delta);
        if distance>=34.0 {continue;}
        let fade=1.0-smoothstep(24.0,34.0,distance);
        let diffuse=max(dot(input.normal_local,delta/max(distance,0.05)),0.0);
        light+=lantern_light(lamp.w)*85.0*fade*fade
            *(0.12+0.88*diffuse)/(dot(delta,delta)+16.0);
    }
    return light;
}

// Periodic, three-dimensional material field: continuous around edges and
// across deck/junction batches, without UV stretching on tall piers. All
// frequencies repeat after256m; unresolved detail tends to its mean, not noise.
fn road_material_hash(cell: vec3<i32>, period: i32) -> f32 {
    let q = vec3<u32>(((cell % period) + period) % period);
    return lantern_hash(q.x * 0x9e3779b9u ^ q.y * 0x85ebca6bu ^ q.z * 0xc2b2ae35u);
}
fn road_material_grain(p: vec3<f32>, frequency: f32, footprint: f32) -> f32 {
    let weight = 1.0 - smoothstep(0.3, 1.0, footprint * frequency);
    if weight < 0.001 { return 0.0; }
    let scaled = p * frequency;
    let cell = vec3<i32>(floor(scaled));
    let f = fract(scaled);
    let t = f * f * (3.0 - 2.0 * f);
    let period = i32(256.0 * frequency);
    let a = mix(road_material_hash(cell, period), road_material_hash(cell+vec3<i32>(1,0,0),period),t.x);
    let b = mix(road_material_hash(cell+vec3<i32>(0,1,0),period),road_material_hash(cell+vec3<i32>(1,1,0),period),t.x);
    let c = mix(road_material_hash(cell+vec3<i32>(0,0,1),period),road_material_hash(cell+vec3<i32>(1,0,1),period),t.x);
    let d = mix(road_material_hash(cell+vec3<i32>(0,1,1),period),road_material_hash(cell+vec3<i32>(1,1,1),period),t.x);
    return (mix(mix(a,b,t.y),mix(c,d,t.y),t.z) - 0.5) * weight;
}
// Anisotropic trilinear value noise; unlike road_material_grain it takes its
// scales from the caller so grain can be stretched along a board or timber.
fn wood_value(q: vec3<f32>) -> f32 {
    let cell = vec3<i32>(floor(q));
    let f = fract(q);
    let t = f * f * (3.0 - 2.0 * f);
    let period = 1048576;
    let a = mix(road_material_hash(cell, period), road_material_hash(cell+vec3<i32>(1,0,0),period),t.x);
    let b = mix(road_material_hash(cell+vec3<i32>(0,1,0),period),road_material_hash(cell+vec3<i32>(1,1,0),period),t.x);
    let c = mix(road_material_hash(cell+vec3<i32>(0,0,1),period),road_material_hash(cell+vec3<i32>(1,0,1),period),t.x);
    let d = mix(road_material_hash(cell+vec3<i32>(0,1,1),period),road_material_hash(cell+vec3<i32>(1,1,1),period),t.x);
    return mix(mix(a,b,t.y),mix(c,d,t.y),t.z) - 0.5;
}
fn plank_heading(code: u32) -> vec2<f32> {
    let h = code >> 16u;
    if h == 0u { return vec2<f32>(0.0, 1.0); }
    let angle = f32(h - 1u) / 65534.0 * 6.2831853;
    return vec2<f32>(cos(angle), sin(angle));
}
fn is_road_tag(colour: vec3<f32>, tag: vec3<f32>) -> bool {
    return all(abs(colour - tag) < vec3<f32>(0.0005));
}
// Wood albedo (rgb) and a 0..1 fluorescence strength (a). Tags keep their old
// meaning: the dark "asphalt" tag is the planked deck, the two "concrete" tags
// are large timbers (parapets, deck edges, legs, bore lining, court walls).
// Anything else (iron, soil, emissive fixtures) is not wood: a = -1.
fn road_wood(input: Output, footprint: f32) -> vec4<f32> {
    let deck = is_road_tag(input.colour, vec3<f32>(0.045,0.048,0.05));
    let timber = is_road_tag(input.colour, vec3<f32>(0.32,0.31,0.29));
    let lining = is_road_tag(input.colour, vec3<f32>(0.24,0.235,0.22));
    if !deck && !timber && !lining { return vec4<f32>(input.colour, -1.0); }
    let facing_up = dot(input.normal, input.up);
    if deck && facing_up > 0.5 {
        // Boards laid across the road in short courses with staggered ends.
        let d = plank_heading(input.lighting);
        let along = dot(input.local_position.xy, d);
        let across = dot(input.local_position.xy, vec2<f32>(-d.y, d.x));
        let width = 0.32;
        let plank_length = 2.6;
        let row = floor(along / width);
        let row_f = fract(along / width);
        let shift = road_material_hash(vec3<i32>(i32(row), 7, 3), 1048576) * plank_length;
        let column = floor((across + shift) / plank_length);
        let column_f = fract((across + shift) / plank_length);
        let tone = road_material_hash(vec3<i32>(i32(row), i32(column), 11), 1048576);
        let seam = min(min(row_f * width, (1.0 - row_f) * width),
            min(column_f * plank_length, (1.0 - column_f) * plank_length) * 0.5);
        let gap = 1.0 - smoothstep(0.004, 0.016, seam);
        let board = vec3<f32>(across * 0.7, along * 26.0, tone * 40.0);
        let grain = 0.6 * wood_value(board) + 0.4 * wood_value(board * vec3<f32>(3.1, 2.3, 1.0));
        let weight = 1.0 - smoothstep(0.08, 0.30, footprint);
        let shade = (0.80 + 0.40 * tone) * (1.0 + 0.55 * grain * weight);
        let albedo = DECK_PLANK_ALBEDO * mix(1.0, shade * mix(1.0, 0.50, gap), weight);
        return vec4<f32>(albedo, clamp(0.8 + 1.2 * grain * weight - 0.7 * gap * weight, 0.1, 1.2));
    }
    // Large squared timbers. Grain runs along the post on upright faces and
    // along the beam elsewhere.
    let up = normalize(input.up);
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(up.x) > 0.8);
    let e1 = normalize(cross(up, helper));
    let e2 = cross(up, e1);
    let p = input.material_position;
    let q = vec3<f32>(dot(p, e1), dot(p, e2), dot(p, up));
    let upright = abs(facing_up) < 0.6;
    var scale = vec3<f32>(0.6, 7.0, 7.0);
    if upright { scale = vec3<f32>(5.0, 5.0, 0.45); }
    let coarse = wood_value(q * scale);
    let fine = wood_value(q * scale * 4.1 + vec3<f32>(17.0, 3.0, 9.0));
    let weight = 1.0 - smoothstep(0.05, 0.25, footprint);
    // Dark butt joints between stacked timbers every 2.4m up a leg.
    var joint = 0.0;
    if upright { let f = fract(q.z / 2.4); joint = 1.0 - smoothstep(0.01, 0.04, min(f, 1.0 - f) * 2.4); }
    var base = BEAM_ALBEDO;
    if lining { base = BEAM_ALBEDO * 0.8; }
    let shade = 1.0 + (0.9 * coarse + 0.5 * fine) * weight;
    let albedo = base * clamp(shade, 0.5, 1.6) * (1.0 - 0.6 * joint * weight);
    return vec4<f32>(albedo, clamp(0.8 + 1.6 * coarse * weight - 0.6 * joint * weight, 0.1, 1.2));
}

fn lit_colour(input: Output) -> vec3<f32> {
    // Derivatives outside material branches remain valid at primitive edges.
    let width = fwidth(input.material_position);
    let footprint = max(max(width.x,width.y),width.z);
    let sun = normalize(camera.sun_direction.xyz);
    let elevation = clamp(dot(sun, input.up), 0.0, 1.0);
    let direct = vec3<f32>(1.9, 1.78, 1.6) * elevation * max(dot(input.normal, sun), 0.0);
    let ambient = mix(vec3<f32>(0.05, 0.055, 0.06), vec3<f32>(0.24, 0.29, 0.36),
        0.5 + 0.5 * dot(input.normal, input.up)) * (0.25 + 0.75 * elevation);
    if input.sheltered > 1.5 { return input.colour; }
    let vertical = dot(input.normal,input.up);
    let interior = (0.10 + 0.04*(1.0-abs(vertical)) + 0.12*max(vertical,0.0))
        * vec3<f32>(1.0,0.96,0.90);
    var illumination=mix(direct + ambient, interior, input.sheltered);
    if (input.lighting & 0xffffu)>0u {illumination=tunnel_irradiance(input);}
    let wood = road_wood(input, footprint);
    let lit = wood.rgb * illumination;
    // Timber fluoresces violet wherever the real light has all but gone.
    if wood.a >= 0.0 && input.sheltered < 1.5 {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        return lit + WOOD_UV_GLOW * wood.a * (1.0 - smoothstep(0.02, 0.14, luma));
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
    output.normal_local = vec3<f32>(0.0,0.0,1.0);
    output.sheltered = select(0.0,1.0,input.colour.z<0.0);
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

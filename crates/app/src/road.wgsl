// Same camera prefix and flat sunlight/hemisphere convention as village.wgsl.
// Both paths bind the camera and cached timber/light material. Streamed roads query the resident
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
    lightning: vec4<f32>,
    moonlight: vec4<f32>,
    moon_direction_view: vec4<f32>,
    moonlight_transmittance: vec4<f32>,
    moon_eclipse: vec4<f32>,
    solar_eclipse_moon: vec4<f32>,
    solar_eclipse_moon_view: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> tunnel_lights: array<vec4<f32>>;
@group(1) @binding(1) var timber_texture: texture_2d<f32>;
@group(1) @binding(2) var timber_sampler: sampler;

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

// Ultraviolet fluorescence of outdoor timber after sunset; never in bores.
const WOOD_UV_GLOW = vec3<f32>(0.20, 0.04, 0.55) * 0.09;
const WOOD_GLOW_BLUE = vec3<f32>(0.07, 0.09, 0.70) * 0.09;
const WOOD_GLOW_RED = vec3<f32>(0.46, 0.03, 0.42) * 0.09;
// Slowly drifting hue: violet shading toward blue in one direction and red in
// the other. Three plane waves whose wave-vectors are integer multiples of the
// 256 m material period, so the field is seamless across the wrapped
// material_position (wavelengths 256/128/85 m, fixed to the ground).
fn wood_glow_colour(p: vec3<f32>) -> vec3<f32> {
    let k = 6.2831853 / 256.0;
    let t = clamp(0.55 * sin(k * (p.x + 2.0 * p.y)) + 0.30 * sin(k * (2.0 * p.y - 3.0 * p.z) + 1.7)
        + 0.25 * sin(k * (3.0 * p.x + p.z) + 4.1), -1.0, 1.0);
    return mix(WOOD_UV_GLOW, select(WOOD_GLOW_BLUE, WOOD_GLOW_RED, t > 0.0), abs(t));
}

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
fn plank_heading(code: u32) -> vec2<f32> {
    let h = code >> 16u;
    if h == 0u { return vec2<f32>(0.0, 1.0); }
    let angle = f32(h - 1u) / 65534.0 * 6.2831853;
    return vec2<f32>(cos(angle), sin(angle));
}
fn is_road_tag(colour: vec3<f32>, tag: vec3<f32>) -> bool {
    return all(abs(colour - tag) < vec3<f32>(0.0005));
}
// Daylight gating is independent of face orientation/material darkness.
// Sunset starts the fade; full glow by approximately seven degrees below it.
fn wood_glow_visibility(sun_elevation: f32, sheltered: f32) -> f32 {
    if sheltered > 0.5 { return 0.0; }
    return 1.0 - smoothstep(-0.12, 0.0, sun_elevation);
}
struct WoodMaterial { albedo: vec3<f32>, fluorescence: f32, height: f32 }
fn joint_coverage(distance: f32, footprint: f32) -> f32 {
    // Integrate a 3mm joint rather than widening it into a distant black line.
    let half_pixel=max(footprint*0.5,0.0001);
    return clamp((min(distance+half_pixel,0.0015)-max(distance-half_pixel,-0.0015))
        /(2.0*half_pixel),0.0,1.0);
}
fn road_wood(input: Output, dx: vec3<f32>, dy: vec3<f32>, wx: vec3<f32>, wy: vec3<f32>) -> WoodMaterial {
    let deck=is_road_tag(input.colour,vec3<f32>(0.045,0.048,0.05));
    let timber=is_road_tag(input.colour,vec3<f32>(0.32,0.31,0.29));
    let lining=is_road_tag(input.colour,vec3<f32>(0.24,0.235,0.22));
    if !deck && !timber && !lining { return WoodMaterial(input.colour,-1.0,0.0); }
    var uv=vec2<f32>(0.0);var ux=uv;var uy=uv;
    var joint=0.0;var tone=1.0;
    let facing_up=dot(input.normal,input.up);
    if deck && facing_up>0.5 {
        let d=plank_heading(input.lighting);
        let side=vec2<f32>(-d.y,d.x);
        let along=dot(input.local_position.xy,d);
        let across=dot(input.local_position.xy,side);
        let row=floor(along/0.32);
        let shift=road_material_hash(vec3<i32>(i32(row),7,3),1048576)*2.6;
        let column=floor((across+shift)/2.6);
        let seed=road_material_hash(vec3<i32>(i32(row),i32(column),11),1048576);
        let flip=select(-1.0,1.0,seed>0.5);
        uv=vec2<f32>(across*0.5*flip,along)+vec2<f32>(seed*17.0,seed*3.0);
        ux=vec2<f32>(dot(dx.xy,side)*0.5*flip,dot(dx.xy,d));
        uy=vec2<f32>(dot(dy.xy,side)*0.5*flip,dot(dy.xy,d));
        let along_width=abs(dot(dx.xy,d))+abs(dot(dy.xy,d));
        let across_width=abs(dot(dx.xy,side))+abs(dot(dy.xy,side));
        let row_distance=min(fract(along/0.32),1.0-fract(along/0.32))*0.32;
        let end_distance=min(fract((across+shift)/2.6),1.0-fract((across+shift)/2.6))*2.6;
        let detail=1.0-smoothstep(0.10,0.32,max(along_width,across_width));
        joint=max(joint_coverage(row_distance,along_width),joint_coverage(end_distance,across_width))*detail;
        tone=mix(1.0,0.91+0.18*seed,detail);
    } else {
        // Dominant-face mapping: one texture lookup, no stretched 3D noise or
        // three-sample triplanar blend. Posts retain grain up their length.
        let up=normalize(input.up);
        let helper=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(up.x)>0.8);
        let e1=normalize(cross(up,helper));let e2=cross(up,e1);
        var u=e1;var v=e2;
        if abs(facing_up)<0.6 {
            u=up;v=select(e1,e2,abs(dot(input.normal,e1))>abs(dot(input.normal,e2)));
            let height=dot(input.material_position,up);
            let distance=min(fract(height/2.4),1.0-fract(height/2.4))*2.4;
            joint=joint_coverage(distance,abs(dot(wx,up))+abs(dot(wy,up)));
        }
        uv=vec2<f32>(dot(input.material_position,u)*0.5,dot(input.material_position,v));
        ux=vec2<f32>(dot(wx,u)*0.5,dot(wx,v));
        uy=vec2<f32>(dot(wy,u)*0.5,dot(wy,v));
        tone=select(0.88,0.76,lining);
    }
    // Gradients are from continuous metre coordinates, not per-board random
    // offsets: joints cannot select blurry mip levels or sparkle in motion.
    let tex=textureSampleGrad(timber_texture,timber_sampler,uv,ux,uy);
    let albedo=tex.rgb*tone*(1.0-0.55*joint);
    let grain=dot(tex.rgb,vec3<f32>(0.2126,0.7152,0.0722));
    return WoodMaterial(albedo,clamp(0.65+grain*0.65-0.25*joint,0.25,1.05),
        tex.a*0.0012-joint*0.0008);
}
fn grain_normal(normal: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>, height_dx: f32, height_dy: f32) -> vec3<f32> {
    let r1=cross(dy,normal);let r2=cross(normal,dx);
    let determinant=dot(dx,r1);
    let gradient=(r1*height_dx+r2*height_dy)*sign(determinant)/max(abs(determinant),0.00000001);
    // Shallow relief only; no jagged silhouettes or high-contrast normal noise.
    return normalize(normal-gradient*min(1.0,0.16/max(length(gradient),0.00001)));
}
fn lit_colour(input: Output) -> vec3<f32> {
    // All derivatives are outside material/light branches.
    let dx=dpdx(input.local_position);let dy=dpdy(input.local_position);
    let wx=dpdx(input.material_position);let wy=dpdy(input.material_position);
    let wood=road_wood(input,dx,dy,wx,wy);
    let hx=dpdx(wood.height);let hy=dpdy(wood.height);
    var shaded=input;
    shaded.normal=grain_normal(input.normal,wx,wy,hx,hy);
    shaded.normal_local=grain_normal(input.normal_local,dx,dy,hx,hy);
    let sun=normalize(camera.sun_direction.xyz);
    let sun_elevation=dot(sun,normalize(input.up));
    let elevation=clamp(sun_elevation,0.0,1.0);
    // A solar eclipse, by the camera's share of the sun (roads are near it):
    // direct sun, and skylight through the sky's perceptual curve with
    // sky_moon.rs SOLAR_ECLIPSE_TOTALITY_SKY as its floor.
    let eclipse=camera.solar_eclipse_moon_view.w;
    // sky_moon.rs SOLAR_ECLIPSE_TOTALITY_LIGHT under the umbra.
    let eclipse_sun=mix(0.06,1.0,eclipse);
    let eclipse_sky=pow(mix(0.02,1.0,eclipse),0.42);
    let direct=vec3<f32>(1.9,1.78,1.6)*elevation*max(dot(shaded.normal,sun),0.0)*eclipse_sun;
    let ambient=mix(vec3<f32>(0.05,0.055,0.06),vec3<f32>(0.24,0.29,0.36),
        0.5+0.5*dot(shaded.normal,input.up))*(0.25+0.75*elevation)*eclipse_sky;
    if input.sheltered>1.5 { return input.colour; }
    let vertical=dot(shaded.normal,input.up);
    let interior=(0.10+0.04*(1.0-abs(vertical))+0.12*max(vertical,0.0))*vec3<f32>(1.0,0.96,0.90);
    var illumination=mix(direct+ambient,interior,input.sheltered);
    if (input.lighting & 0xffffu)>0u {illumination=tunnel_irradiance(shaded);}
    let lit=wood.albedo*illumination;
    if wood.fluorescence>=0.0 {
        return lit+wood_glow_colour(input.material_position)*wood.fluorescence
            *wood_glow_visibility(sun_elevation,input.sheltered);
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

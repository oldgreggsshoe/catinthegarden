// Debug centre lines: fixed pixel width, unlit, visible through terrain/bores.
// The far side of the globe must not shine through the planet from orbit.
struct Camera {
    projection_matrix: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_view_altitude: vec4<f32>,
}
struct Frame {
    origin: vec4<f32>,
    across: vec4<f32>,
    forward: vec4<f32>,
    up: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> frame: Frame;
const PLANET_RADIUS: f32 = __PLANET_RADIUS__;
struct Line {
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) generated: f32,
}
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(linear) side: f32,
    @location(1) horizon: f32,
    @location(2) @interpolate(flat) valid: f32,
    @location(3) @interpolate(flat) generated: f32,
    @location(4) @interpolate(flat) schematic: f32,
}
fn relative(p: vec3<f32>) -> vec3<f32> {
    return frame.origin.xyz + frame.across.xyz*p.x + frame.forward.xyz*p.y + frame.up.xyz*p.z;
}
fn view(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(p,camera.camera_right.xyz),dot(p,camera.camera_up.xyz),-dot(p,camera.camera_forward.xyz));
}
fn horizon(p: vec3<f32>, r: vec3<f32>) -> f32 {
    let absolute = frame.across.xyz*p.x + frame.forward.xyz*p.y + frame.up.xyz*(p.z+PLANET_RADIUS);
    return dot(normalize(absolute),-r);
}
@vertex
fn vs_main(line: Line, @builtin(vertex_index) corner: u32) -> Output {
    var a=relative(line.a);
    var b=relative(line.b);
    var va=view(a);
    var vb=view(b);
    var ha=horizon(line.a,a);
    var hb=horizon(line.b,b);
    // Clip in view space before dividing by w; an end behind the eye must
    // never expand a line into a screen-filling red triangle.
    let near=-0.1;
    let valid=va.z<near || vb.z<near;
    if va.z>=near && vb.z<near {
        let t=(near-va.z)/(vb.z-va.z);
        va=mix(va,vb,t); ha=mix(ha,hb,t);
    } else if vb.z>=near && va.z<near {
        let t=(near-vb.z)/(va.z-vb.z);
        vb=mix(vb,va,t); hb=mix(hb,ha,t);
    }
    let ca=camera.projection_matrix*vec4<f32>(va,1.0);
    let cb=camera.projection_matrix*vec4<f32>(vb,1.0);
    let viewport=max(vec2<f32>(frame.origin.w,frame.across.w),vec2<f32>(1.0));
    let pa=ca.xy/max(ca.w,0.01)*viewport*0.5;
    let pb=cb.xy/max(cb.w,0.01)*viewport*0.5;
    let delta=pb-pa;
    let axis=delta/max(length(delta),0.0001);
    let tangent=select(vec2<f32>(1.0,0.0),axis,length(delta)>0.0001);
    let normal=vec2<f32>(-tangent.y,tangent.x);
    let end=corner==1u || corner==2u || corner==4u;
    let across=select(-1.0,1.0,corner==2u || corner==4u || corner==5u);
    let clip=select(ca,cb,end);
    let schematic=frame.up.w>0.5 && line.generated<0.5;
    // A two-pixel global grid blankets entire continents at orbital scale.
    // Keep planned links thin/translucent and generated routes prominent.
    let half_width=select(1.5,0.75,schematic);
    let pixel=select(pa,pb,end)+normal*(across*half_width)+tangent*select(-0.5,0.5,end);
    var output: Output;
    output.position=vec4<f32>(pixel*2.0/viewport*clip.w,0.0,clip.w);
    output.side=across*half_width;
    output.horizon=select(ha,hb,end);
    output.valid=select(0.0,1.0,valid);
    output.generated=line.generated;
    output.schematic=select(0.0,1.0,schematic);
    return output;
}
@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    if input.valid<0.5 { discard; }
    if camera.camera_planet_direction_view_altitude.w>=50000.0 && input.horizon<0.0 { discard; }
    let alpha=select(1.0-smoothstep(0.75,1.5,abs(input.side)),0.55*(1.0-smoothstep(0.0,0.75,abs(input.side))),input.schematic>0.5);
    return vec4<f32>(select(vec3<f32>(2.0,0.005,0.005),vec3<f32>(0.005,2.0,0.005),input.generated>0.5),alpha);
}

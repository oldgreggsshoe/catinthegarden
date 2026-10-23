struct Camera {
    projection_matrix:mat4x4<f32>,camera_forward:vec4<f32>,camera_right:vec4<f32>,camera_up:vec4<f32>,
    camera_planet_direction_view_altitude:vec4<f32>,sun_direction:vec4<f32>,sun_direction_view:vec4<f32>,projection:vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
@group(1) @binding(0) var atlas:texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler:sampler;
@group(1) @binding(2) var wood_texture:texture_2d<f32>;
@group(1) @binding(3) var wood_sampler:sampler;
struct Input {
    @location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,@location(3) colour:vec3<f32>,
    @location(4) origin:vec4<f32>,@location(5) across:vec4<f32>,@location(6) forward:vec4<f32>,@location(7) up:vec4<f32>,
}
struct Output {
    @builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,@location(1) colour:vec3<f32>,
    @location(2) @interpolate(flat) light:vec3<f32>,
    @location(3) @interpolate(flat) wood:f32,
}
@vertex fn vs_main(v:Input)->Output {
    let relative=v.origin.xyz+v.across.xyz*v.position.x+v.forward.xyz*v.position.y+v.up.xyz*v.position.z;
    let view=vec3<f32>(dot(relative,camera.camera_right.xyz),dot(relative,camera.camera_up.xyz),-dot(relative,camera.camera_forward.xyz));
    let normal=normalize(v.across.xyz*v.normal.x+v.forward.xyz*v.normal.y+v.up.xyz*v.normal.z);
    var out:Output;out.position=camera.projection_matrix*vec4<f32>(view,1.0);out.uv=v.uv;out.colour=abs(v.colour);
    out.wood=select(0.0,1.0,v.uv.x < -1.5);
    if out.wood>0.5 {out.uv=vec2<f32>(-2.0-v.uv.x,v.uv.y);}
    // The road's own lighting (road.wgsl lit_colour): flat sun plus sky ambient,
    // nothing self-lit. Boards inside bores (negative tag) get the sheltered level.
    let up=normalize(v.up.xyz);let sun=normalize(camera.sun_direction.xyz);
    let elevation=clamp(dot(sun,up),0.0,1.0);
    let direct=vec3<f32>(1.9,1.78,1.6)*elevation*max(dot(normal,sun),0.0);
    let ambient=mix(vec3<f32>(0.05,0.055,0.06),vec3<f32>(0.24,0.29,0.36),0.5+0.5*dot(normal,up))*(0.25+0.75*elevation);
    out.light=direct+ambient;
    if v.colour.z<0.0 {out.light=(0.10+0.04*(1.0-abs(dot(normal,up))))*vec3<f32>(1.0,0.96,0.90);}
    return out;
}
@fragment fn fs_main(v:Output)->@location(0) vec4<f32> {
    var alpha=1.0;
    var colour=v.colour;
    var wood_grain=-1.0;
    if v.wood>0.5 {
        let grain=textureSample(wood_texture,wood_sampler,v.uv).rgb;
        colour*=grain;
        wood_grain=dot(grain,vec3<f32>(0.3333));
    }
    else if v.uv.x>=0.0 {alpha=textureSample(atlas,atlas_sampler,v.uv).a;}
    if alpha<0.02 {discard;}
    var lit=colour*v.light;
    // Same dim violet fluorescence as the road timber, only where the light has all but gone.
    if wood_grain>=0.0 {
        let luma=dot(lit,vec3<f32>(0.2126,0.7152,0.0722));
        lit+=vec3<f32>(0.20,0.04,0.55)*0.09*(0.5+1.0*wood_grain)*(1.0-smoothstep(0.02,0.14,luma));
    }
    return vec4<f32>(lit,alpha);
}

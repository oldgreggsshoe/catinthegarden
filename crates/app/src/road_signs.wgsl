struct Camera {
    projection_matrix:mat4x4<f32>,camera_forward:vec4<f32>,camera_right:vec4<f32>,camera_up:vec4<f32>,
    camera_planet_direction_view_altitude:vec4<f32>,sun_direction:vec4<f32>,sun_direction_view:vec4<f32>,projection:vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
@group(1) @binding(0) var atlas:texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler:sampler;
struct Input {
    @location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,@location(3) colour:vec3<f32>,
    @location(4) origin:vec4<f32>,@location(5) across:vec4<f32>,@location(6) forward:vec4<f32>,@location(7) up:vec4<f32>,
}
struct Output {
    @builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,@location(1) colour:vec3<f32>,
    @location(2) @interpolate(flat) light:f32,
}
@vertex fn vs_main(v:Input)->Output {
    let relative=v.origin.xyz+v.across.xyz*v.position.x+v.forward.xyz*v.position.y+v.up.xyz*v.position.z;
    let view=vec3<f32>(dot(relative,camera.camera_right.xyz),dot(relative,camera.camera_up.xyz),-dot(relative,camera.camera_forward.xyz));
    let normal=normalize(v.across.xyz*v.normal.x+v.forward.xyz*v.normal.y+v.up.xyz*v.normal.z);
    var out:Output;out.position=camera.projection_matrix*vec4<f32>(view,1.0);out.uv=v.uv;out.colour=abs(v.colour);
    out.light=0.40+1.4*max(dot(normal,normalize(camera.sun_direction.xyz)),0.0)*max(dot(v.up.xyz,normalize(camera.sun_direction.xyz)),0.0);
    if v.colour.z<0.0 {out.light=1.0;}
    return out;
}
@fragment fn fs_main(v:Output)->@location(0) vec4<f32> {
    var alpha=1.0;
    if v.uv.x>=0.0 {alpha=textureSample(atlas,atlas_sampler,v.uv).a;}
    if alpha<0.02 {discard;}
    return vec4<f32>(v.colour*v.light,alpha);
}

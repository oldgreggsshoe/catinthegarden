// Instanced timber vehicles. Same camera and real lantern list as the road.
struct Camera {
    projection_matrix:mat4x4<f32>, camera_forward:vec4<f32>, camera_right:vec4<f32>, camera_up:vec4<f32>,
    camera_planet_direction_view_altitude:vec4<f32>, sun_direction:vec4<f32>, sun_direction_view:vec4<f32>,
    projection:vec4<f32>, flat_triangle_options:vec4<f32>, lightning:vec4<f32>, moonlight:vec4<f32>,
    moon_direction_view:vec4<f32>, moonlight_transmittance:vec4<f32>, moon_eclipse:vec4<f32>,
    solar_eclipse_moon:vec4<f32>, solar_eclipse_moon_view:vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
@group(1) @binding(0) var<storage,read> lamps:array<vec4<f32>>;
@group(1) @binding(1) var timber:texture_2d<f32>;
@group(1) @binding(2) var timber_sampler:sampler;
struct Frame {across:vec4<f32>,forward:vec4<f32>,up:vec4<f32>}
@group(2) @binding(0) var<uniform> frame:Frame;
struct Input {
    @location(0) position:vec3<f32>, @location(1) normal:vec3<f32>, @location(2) colour:vec3<f32>,
    @location(3) origin_spin:vec4<f32>, @location(4) right_steer:vec4<f32>,
    @location(5) forward_tunnel:vec4<f32>, @location(6) up_light:vec4<f32>,
    @location(7) field_fade:vec4<f32>, @location(8) tint:vec4<f32>, @location(9) part:u32,
}
struct Output {
    @invariant @builtin(position) position:vec4<f32>, @location(0) normal:vec3<f32>,
    @location(1) local:vec3<f32>, @location(2) field_position:vec3<f32>, @location(3) normal_field:vec3<f32>,
    @location(4) @interpolate(flat) colour:vec3<f32>, @location(5) @interpolate(flat) up:vec3<f32>,
    @location(6) @interpolate(flat) settings:vec4<f32>, @location(7) @interpolate(flat) material:u32, @location(8) material_uv:vec2<f32>,
    @location(9) eye_ray:vec3<f32>,
}
fn rotate_wheel(v:vec3<f32>,spin:f32,steer:f32)->vec3<f32> {
    let rolling=vec3<f32>(v.x,cos(spin)*v.y+sin(spin)*v.z,-sin(spin)*v.y+cos(spin)*v.z);
    return vec3<f32>(cos(steer)*rolling.x-sin(steer)*rolling.y,sin(steer)*rolling.x+cos(steer)*rolling.y,rolling.z);
}
fn field_vector(v:vec3<f32>)->vec3<f32> {return vec3<f32>(dot(v,frame.across.xyz),dot(v,frame.forward.xyz),dot(v,frame.up.xyz));}
@vertex fn vs_main(input:Input)->Output {
    var p=input.position;var n=input.normal;
    let part=input.part&255u;
    if part>=1u && part<=4u {
        let i=part-1u;let front=i>=2u;
        let pivot=vec3<f32>(select(-0.94,0.94,(i&1u)>0u),select(-1.15,1.15,front),0.52);
        let steer=select(0.0,input.right_steer.w,front);
        p=pivot+rotate_wheel(p-pivot,input.origin_spin.w,steer);
        n=rotate_wheel(n,input.origin_spin.w,steer);
    }
    let offset=input.right_steer.xyz*p.x+input.forward_tunnel.xyz*p.y+input.up_light.xyz*p.z;
    let world=input.origin_spin.xyz+offset;
    let view=vec3<f32>(dot(world,camera.camera_right.xyz),dot(world,camera.camera_up.xyz),-dot(world,camera.camera_forward.xyz));
    let normal=normalize(input.right_steer.xyz*n.x+input.forward_tunnel.xyz*n.y+input.up_light.xyz*n.z);
    var out:Output;out.position=camera.projection_matrix*vec4<f32>(view,1.0);out.normal=normal;out.local=input.position;
    out.field_position=input.field_fade.xyz+field_vector(offset);out.normal_field=field_vector(normal);
    out.material_uv=select(input.position.xy,input.position.yz,abs(input.normal.x)>abs(input.normal.z))*vec2<f32>(0.55,0.3);
    out.colour=input.colour*input.tint.xyz;out.up=input.up_light.xyz;
    out.settings=vec4<f32>(input.forward_tunnel.w,input.up_light.w,input.field_fade.w,length(view));out.material=input.part>>8u;
    out.eye_ray=world;
    return out;
}
@fragment fn fs_main(input:Output)->@location(0) vec4<f32> {
    let distance_fade=1.0-smoothstep(1700.0,2000.0,input.settings.w);
    let fade=input.settings.z*distance_fade;
    if fade<1.0 {let pixel=vec2<u32>(input.position.xy);let noise=f32((pixel.x*1973u+pixel.y*9277u)&255u)/255.0;if noise>fade {discard;}}
    if input.material==3u {return vec4<f32>(input.colour*2.0,1.0);}
    let n=normalize(input.normal);
    let oak=textureSample(timber,timber_sampler,input.material_uv).rgb;
    var albedo=input.colour;
    if input.material==0u {albedo*=mix(vec3<f32>(0.7),oak*2.4,0.75);}
    let sun=normalize(camera.sun_direction.xyz);let elevation=max(dot(sun,input.up),0.0);
    let eclipse=camera.solar_eclipse_moon_view.w;
    let direct=vec3<f32>(1.9,1.78,1.6)*elevation*max(dot(n,sun),0.0)*mix(0.06,1.0,eclipse);
    let ambient=mix(vec3<f32>(0.05,0.055,0.06),vec3<f32>(0.24,0.29,0.36),0.5+0.5*dot(n,input.up))*(0.25+0.75*elevation)*pow(mix(0.02,1.0,eclipse),0.42);
    let moon=normalize(camera.moonlight.xyz+vec3<f32>(0.0,1e-9,0.0));
    let moon_light=vec3<f32>(0.7,0.79,1.0)*camera.moonlight.w*max(dot(n,moon),0.0)*camera.moonlight_transmittance.rgb*camera.moon_direction_view.w*camera.moon_eclipse.rgb;
    var light=direct+ambient+moon_light;
    if input.settings.x>0.5 {
        light=vec3<f32>(0.0);
        let base=u32(input.settings.y)*25u;let count=min(u32(lamps[base].x),24u);
        for(var i=0u;i<count;i+=1u){let lamp=lamps[base+1u+i];let delta=lamp.xyz-input.field_position;let distance=length(delta);
            if distance>=34.0{continue;}
            let fade=1.0-smoothstep(24.0,34.0,distance);let diffuse=max(dot(normalize(input.normal_field),delta/max(distance,0.05)),0.0);
            light+=lantern_light(lamp.w)*85.0*fade*fade*(0.12+0.88*diffuse)/(dot(delta,delta)+16.0);
        }
    }
    return vec4<f32>(albedo*light,1.0);
}
// Engine jar glass: faint and clear face-on, a pale sky-lit rim at grazing
// angles, tinted by the orb glowing inside. Alpha blended, no depth write.
@fragment fn fs_glass(input:Output)->@location(0) vec4<f32> {
    let fade=input.settings.z*(1.0-smoothstep(1700.0,2000.0,input.settings.w));
    if fade<1.0 {let pixel=vec2<u32>(input.position.xy);let noise=f32((pixel.x*1973u+pixel.y*9277u)&255u)/255.0;if noise>fade {discard;}}
    let n=normalize(input.normal);let v=normalize(-input.eye_ray);
    let rim=pow(1.0-abs(dot(n,v)),3.0);
    let day=max(dot(normalize(camera.sun_direction.xyz),input.up),0.0);
    let sky=vec3<f32>(0.55,0.62,0.75)*(0.06+0.9*day)*select(1.0,0.12,input.settings.x>0.5);
    let glow=vec3<f32>(0.35,0.12,0.7)*0.18;
    return vec4<f32>(sky*rim+glow,0.1+0.55*rim);
}

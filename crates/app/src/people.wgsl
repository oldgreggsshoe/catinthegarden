// Instanced villagers: rigid-part skeleton posed from walk phase, walk weight
// and sit weight; garment choices collapse unused superset parts.
struct Camera {
    projection_matrix:mat4x4<f32>, camera_forward:vec4<f32>, camera_right:vec4<f32>, camera_up:vec4<f32>,
    camera_planet_direction_view_altitude:vec4<f32>, sun_direction:vec4<f32>, sun_direction_view:vec4<f32>,
    projection:vec4<f32>, flat_triangle_options:vec4<f32>, lightning:vec4<f32>, moonlight:vec4<f32>,
    moon_direction_view:vec4<f32>, moonlight_transmittance:vec4<f32>, moon_eclipse:vec4<f32>,
    solar_eclipse_moon:vec4<f32>, solar_eclipse_moon_view:vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
@group(1) @binding(0) var<storage,read> lamps:array<vec4<f32>>;
struct Frame {across:vec4<f32>,forward:vec4<f32>,up:vec4<f32>}
@group(2) @binding(0) var<uniform> frame:Frame;
struct Input {
    @location(0) position:vec3<f32>, @location(1) normal:vec3<f32>, @location(2) tags:u32,
    @location(3) origin_phase:vec4<f32>, @location(4) forward_walk:vec4<f32>, @location(5) up_sit:vec4<f32>,
    @location(6) body:vec4<f32>, @location(7) field:vec4<f32>,
    @location(8) colours_a:vec4<u32>, @location(9) colours_b:vec4<u32>,
}
struct Output {
    @invariant @builtin(position) position:vec4<f32>,
    @location(0) normal:vec3<f32>,
    @location(1) @interpolate(flat) colour:vec3<f32>,
    @location(2) @interpolate(flat) up:vec3<f32>,
    @location(3) field_position:vec3<f32>,
    @location(4) normal_field:vec3<f32>,
    // Lamp group (negative out of doors), fade, eye distance, shadow strength.
    @location(5) @interpolate(flat) settings:vec4<f32>,
    @location(6) eye_ray:vec3<f32>,
    @location(7) shadow_uv:vec2<f32>,
}
const PI:f32=3.14159265;
fn pitch(v:vec3<f32>,a:f32)->vec3<f32> {let c=cos(a);let s=sin(a);return vec3<f32>(v.x,v.y*c-v.z*s,v.y*s+v.z*c);}
fn yaw(v:vec3<f32>,a:f32)->vec3<f32> {let c=cos(a);let s=sin(a);return vec3<f32>(v.x*c-v.y*s,v.x*s+v.y*c,v.z);}
fn rgb(c:u32)->vec3<f32> {return unpack4x8unorm(c).rgb;}
fn field_vector(v:vec3<f32>)->vec3<f32> {return vec3<f32>(dot(v,frame.across.xyz),dot(v,frame.forward.xyz),dot(v,frame.up.xyz));}

struct Pose {thigh:vec2<f32>,knee:vec2<f32>,arm:vec2<f32>,elbow:vec2<f32>,lean:f32,twist:f32,bob:f32,sway:f32}
// Joint angles (radians, positive swings a limb forward; knee and elbow are
// flexion) blended from standing, walking and sitting.
fn pose_at(phase:f32,walk:f32,sit:f32,skirt:bool)->Pose {
    let amplitude=select(0.42,0.3,skirt);
    let s=sin(phase);
    let swing=vec2<f32>(amplitude*s,-amplitude*s);
    // The knee bends most early in the swing, straight again at heel strike.
    let lift=vec2<f32>(pow(max(cos(phase+0.7),0.0),2.0),pow(max(cos(phase+0.7+PI),0.0),2.0));
    let walk_knee=vec2<f32>(0.08)+select(0.62,0.4,skirt)*lift;
    let walk_arm=-0.6*swing;
    let walk_elbow=vec2<f32>(0.25)+0.6*max(walk_arm,vec2<f32>(0.0));
    let breathe=sin(phase*1.3);
    let stand_arm=vec2<f32>(0.05+0.012*breathe);
    var p:Pose;
    p.thigh=mix(mix(vec2<f32>(0.0),swing,walk),vec2<f32>(1.5),sit);
    p.knee=mix(mix(vec2<f32>(0.04),walk_knee,walk),vec2<f32>(1.42),sit);
    p.arm=mix(mix(stand_arm,walk_arm,walk),vec2<f32>(0.5,0.45),sit);
    p.elbow=mix(mix(vec2<f32>(0.18),walk_elbow,walk),vec2<f32>(0.85,0.9),sit);
    p.lean=mix(mix(0.004*breathe,0.06,walk),-0.07,sit);
    let moving=walk*(1.0-sit);
    p.twist=-0.07*s*moving;
    p.bob=(0.012*cos(2.0*phase)-0.012)*moving;
    p.sway=0.012*s*moving+0.004*sin(phase*0.37)*(1.0-walk)*(1.0-sit);
    return p;
}

@vertex fn vs_main(input:Input)->Output {
    var out:Output;
    let bone=input.tags&15u;
    let slot=(input.tags>>4u)&15u;
    let group=(input.tags>>8u)&7u;
    let allowed=(input.tags>>11u)&255u;
    let selection=input.colours_b.w;
    if group>0u {
        let choice=(selection>>(3u*(group-1u)))&7u;
        if ((allowed>>choice)&1u)==0u {
            out.position=vec4<f32>(0.0,0.0,0.0,1.0);
            return out;
        }
    }
    let height=input.body.x;let girth=input.body.y;
    let skirt=(selection&7u)==1u;
    let sit=input.up_sit.w;
    let pose=pose_at(input.origin_phase.w,input.forward_walk.w,sit,skirt);
    var p=input.position;var n=input.normal;
    if bone!=2u && bone!=15u {p=vec3<f32>(p.x*girth,p.y*girth,p.z);}
    if bone==9u || bone==10u {
        let i=bone-9u;let knee=vec3<f32>(select(-0.085,0.085,i==1u)*girth,0.0,0.288);
        p=knee+pitch(p-knee,-pose.knee[i]);n=pitch(n,-pose.knee[i]);
    }
    if bone>=7u && bone<=10u {
        let i=(bone-7u)%2u;let hip=vec3<f32>(select(-0.085,0.085,i==1u)*girth,0.0,0.53);
        p=hip+pitch(p-hip,pose.thigh[i]);n=pitch(n,pose.thigh[i]);
    }
    if bone==5u || bone==6u {
        let i=bone-5u;let elbow=vec3<f32>(select(-0.188,0.188,i==1u)*girth,0.0,0.625);
        p=elbow+pitch(p-elbow,pose.elbow[i]);n=pitch(n,pose.elbow[i]);
    }
    if bone>=3u && bone<=6u {
        let i=(bone-3u)%2u;let shoulder=vec3<f32>(select(-0.172,0.172,i==1u)*girth,0.0,0.8);
        p=shoulder+pitch(p-shoulder,pose.arm[i]);n=pitch(n,pose.arm[i]);
    }
    if bone==2u {
        let neck=vec3<f32>(0.0,0.0,0.875);
        // Children: a relatively larger head on the same skeleton.
        let head=select(1.0,1.28,(selection&(1u<<24u))!=0u);
        p=neck+yaw((p-neck)*head,input.body.z);n=yaw(n,input.body.z);
    }
    if bone>=1u && bone<=6u {
        let waist=vec3<f32>(0.0,0.0,0.58);
        p=waist+yaw(pitch(p-waist,pose.lean),pose.twist);n=yaw(pitch(n,pose.lean),pose.twist);
    }
    var shadow=0.0;
    if bone==15u {
        out.shadow_uv=input.position.xy/0.2;
        shadow=(1.0-sit)*input.body.w;
    } else {
        p=yaw(p,-0.5*pose.twist)+vec3<f32>(pose.sway,0.0,pose.bob);
        n=yaw(n,-0.5*pose.twist);
    }
    p*=height;
    let up=normalize(input.up_sit.xyz);let forward=normalize(input.forward_walk.xyz);
    let right=normalize(cross(forward,up));
    let offset=right*p.x+forward*p.y+up*p.z;
    let world=input.origin_phase.xyz+offset;
    let view=vec3<f32>(dot(world,camera.camera_right.xyz),dot(world,camera.camera_up.xyz),-dot(world,camera.camera_forward.xyz));
    out.position=camera.projection_matrix*vec4<f32>(view,1.0);
    let normal=normalize(right*n.x+forward*n.y+up*n.z);
    out.normal=normal;
    out.field_position=input.field.xyz+field_vector(offset);
    out.normal_field=field_vector(normal);
    out.up=up;
    out.eye_ray=world;
    out.settings=vec4<f32>(input.field.w,input.body.w,length(view),shadow);
    var colour=vec3<f32>(0.5);
    switch slot {
        case 0u {colour=rgb(input.colours_a.x);}
        case 1u {colour=rgb(input.colours_a.y);}
        case 2u {colour=rgb(input.colours_a.z);}
        case 3u {colour=rgb(input.colours_a.w);}
        case 4u {colour=rgb(input.colours_b.x);}
        case 5u {colour=rgb(input.colours_b.y);}
        case 6u {colour=rgb(input.colours_b.z);}
        // Trousers, or dark stockings under a skirt.
        case 7u {colour=select(rgb(input.colours_a.w),rgb(input.colours_b.z)*0.9+vec3<f32>(0.015),skirt);}
        case 8u {colour=vec3<f32>(0.025,0.02,0.018);}
        case 9u {colour=vec3<f32>(0.07,0.045,0.025);}
        case 10u {colour=vec3<f32>(0.6,0.56,0.46);}
        default {}
    }
    out.colour=colour;
    return out;
}

fn dithered(position:vec4<f32>,settings:vec4<f32>) {
    let fade=settings.y*(1.0-smoothstep(380.0,450.0,settings.z));
    if fade<1.0 {let pixel=vec2<u32>(position.xy);let noise=f32((pixel.x*1973u+pixel.y*9277u)&255u)/255.0;if noise>fade {discard;}}
}

@fragment fn fs_main(input:Output)->@location(0) vec4<f32> {
    dithered(input.position,input.settings);
    var n=normalize(input.normal);
    // Double-sided cloth: light the face that is seen.
    if dot(n,input.eye_ray)>0.0 {n=-n;}
    let sun=normalize(camera.sun_direction.xyz);let elevation=max(dot(sun,input.up),0.0);
    let eclipse=camera.solar_eclipse_moon_view.w;
    let wrap=max((dot(n,sun)+0.15)/1.15,0.0);
    let direct=vec3<f32>(1.9,1.78,1.6)*elevation*wrap*mix(0.06,1.0,eclipse);
    let ambient=mix(vec3<f32>(0.05,0.055,0.06),vec3<f32>(0.26,0.31,0.38),0.5+0.5*dot(n,input.up))*(0.25+0.75*elevation)*pow(mix(0.02,1.0,eclipse),0.42);
    let moon=normalize(camera.moonlight.xyz+vec3<f32>(0.0,1e-9,0.0));
    let moon_light=vec3<f32>(0.7,0.79,1.0)*camera.moonlight.w*max(dot(n,moon),0.0)*camera.moonlight_transmittance.rgb*camera.moon_direction_view.w*camera.moon_eclipse.rgb;
    var light=direct+ambient+moon_light;
    if input.settings.x>=0.0 {
        light=vec3<f32>(0.0);
        let normal_field=normalize(input.normal_field)*select(1.0,-1.0,dot(normalize(input.normal),input.eye_ray)>0.0);
        let base=u32(input.settings.x)*25u;let count=min(u32(lamps[base].x),24u);
        for(var i=0u;i<count;i+=1u){let lamp=lamps[base+1u+i];let delta=lamp.xyz-input.field_position;let distance=length(delta);
            if distance>=34.0{continue;}
            let fade=1.0-smoothstep(24.0,34.0,distance);let diffuse=max(dot(normal_field,delta/max(distance,0.05)),0.0);
            light+=lantern_light(lamp.w)*85.0*fade*fade*(0.12+0.88*diffuse)/(dot(delta,delta)+16.0);
        }
    }
    return vec4<f32>(input.colour*light,1.0);
}

// A soft dark patch under each standing or walking villager.
@fragment fn fs_shadow(input:Output)->@location(0) vec4<f32> {
    let r=length(input.shadow_uv);
    let fade=1.0-smoothstep(380.0,450.0,input.settings.z);
    let alpha=0.42*(1.0-smoothstep(0.25,1.0,r))*input.settings.w*fade;
    return vec4<f32>(0.0,0.0,0.0,alpha);
}

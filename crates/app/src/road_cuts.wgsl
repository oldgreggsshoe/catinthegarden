// Indexed corridor field: four header vectors followed by uniformly y-spaced
// centre/profile samples. No loop over the route, and no fragment-stage search.
@group(2) @binding(21) var<storage, read> road_cut_data: array<vec4<f32>>;
struct RoadTerrainCut { height: f32, normal: vec3<f32>, lowered: bool, paved: bool }
fn road_pilot_terrain_cut(direction: vec3<f32>, uncut_height: f32) -> RoadTerrainCut {
    var result = RoadTerrainCut(uncut_height,direction,false,false);
    if !ROAD_CUTS_ENABLED { return result; }
    let settings = road_cut_data[0];
    if settings.w < 0.5 { return result; }
    let across = road_cut_data[1].xyz;
    let forward = road_cut_data[2].xyz;
    let up = road_cut_data[3].xyz;
    let cosine = dot(direction,up);
    if cosine < 0.99 { return result; }
    let tangent = vec2<f32>(dot(direction,across),dot(direction,forward));
    let last = u32(settings.z)-1u;
    let end = road_cut_data[4u+last].y;
    for (var iteration=0u; iteration<2u; iteration+=1u) {
        let xy = tangent*(settings.x+result.height);
        if abs(xy.x)>3600.0 || xy.y < -500.0 || xy.y>end+500.0 { return RoadTerrainCut(uncut_height,direction,false,false); }
        let station=clamp(xy.y/settings.y,0.0,f32(last));
        let index=min(u32(floor(station)),last-1u);
        if road_cut_data[4u+index].w > 0.5 { return RoadTerrainCut(uncut_height,direction,false,false); }
        let a=road_cut_data[4u+index].xyz;
        let b=road_cut_data[5u+index].xyz;
        let centre=mix(a,b,station-f32(index));
        let slope=(b.xz-a.xz)/(b.y-a.y);
        let normalizer=sqrt(1.0+slope.x*slope.x);
        let offset=(xy.x-centre.x)/normalizer;
        let bank=max(abs(offset)-9.0,0.0);
        let beyond=max(max(-xy.y,xy.y-end),0.0);
        let cap=centre.z-6.0+bank+beyond;
        // Subtract the datum before adding the small height, not after. This
        // avoids quarter-metre cancellation against a 4,000 km radius.
        let height=(cap-(cosine-1.0)*settings.x)/cosine;
        result.height=min(uncut_height,height);
        result.lowered=height<uncut_height;
        if !result.lowered { return result; }
        let bank_slope=select(0.0,sign(offset)/normalizer,abs(offset)>9.0);
        let end_slope=select(select(0.0,1.0,xy.y>end),-1.0,xy.y<0.0);
        result.normal=normalize(up-across*bank_slope-forward*(slope.y-bank_slope*slope.x+end_slope));
    }
    return result;
}

// The removed vegetation exposes soil/rock. Reuse the existing material's
// grain and brightness; this adds no texture lookup or corridor search.
fn road_cut_albedo(colour: vec3<f32>, removed: f32) -> vec3<f32> {
    if !ROAD_CUTS_ENABLED { return colour; }
    let luminance=dot(colour,vec3<f32>(0.2126,0.7152,0.0722));
    return mix(colour,luminance*vec3<f32>(1.15,0.91,0.65),smoothstep(0.3,2.0,removed));
}

// Positive in tunnel air, negative in rock. Only the portal-height fragments
// are removed; the hillside above the roof is retained. The half-metre bore
// allowance buries the lining and absorbs planet-radius float rounding.
fn road_pilot_tunnel_void(direction: vec3<f32>, height: f32) -> f32 {
    if !ROAD_CUTS_ENABLED { return -1000000.0; }
    let settings=road_cut_data[0];
    if settings.w < 0.5 { return -1000000.0; }
    let cosine=dot(direction,road_cut_data[3].xyz);
    if cosine < 0.99 { return -1000000.0; }
    let xy=vec2<f32>(dot(direction,road_cut_data[1].xyz),dot(direction,road_cut_data[2].xyz))*(settings.x+height);
    let last=u32(settings.z)-1u;
    if abs(xy.x)>3600.0 || xy.y<0.0 || xy.y>=road_cut_data[4u+last].y { return -1000000.0; }
    let station=xy.y/settings.y;
    let index=min(u32(floor(station)),last-1u);
    let a=road_cut_data[4u+index];
    let b=road_cut_data[5u+index];
    if max(a.w,b.w)<0.5 { return -1000000.0; }
    let centre=mix(a.xyz,b.xyz,station-f32(index));
    let slope=(b.x-a.x)/(b.y-a.y);
    let offset=abs((xy.x-centre.x)/sqrt(1.0+slope*slope));
    let z=(cosine-1.0)*settings.x+cosine*height-centre.z;
    let roof=min(9.0-0.25*offset,12.8-1.2*offset);
    return min(min(7.0-offset,z+6.5),roof+0.5-z);
}

// The liner's 0.8m shell covers the 0.5m excavation plus this tolerance.
// Ray terrain must not author a competing inner surface in that shell.
fn road_tunnel_lining_owns(direction: vec3<f32>, height: f32) -> bool {
    return road_tunnel_void(direction,height)>-0.25;
}

// Network mode: fixed-capacity open-addressed grid, built with the CPU field.
// A query reads one local cell (<=64 segments), never the planetary network.
const NETWORK_BUCKETS: u32 = 32768u;
const NETWORK_REFS: u32 = 32772u;
const NETWORK_SEGMENTS: u32 = 98308u;
// Explicit metadata distinguishes courts from roads whose short endpoints
// happen to quantize to one f32 position. Courts have coincident endpoints.
const NETWORK_PARKING_RADIUS: f32 = 20.0;
const NETWORK_PARKING_CUT_RADIUS: f32 = 22.0;
const NETWORK_PARKING_TREE_RADIUS: f32 = 40.0;
fn road_network_cell(xy: vec2<f32>) -> vec3<u32> {
    let cell = vec2<i32>(floor(xy / road_cut_data[0].y));
    let hash = (bitcast<u32>(cell.x)*73856093u) ^ (bitcast<u32>(cell.y)*19349663u);
    for (var probe=0u; probe<32u; probe+=1u) {
        let entry=road_cut_data[4u+((hash+probe)&(NETWORK_BUCKETS-1u))];
        if entry.w<0.5 { break; }
        if all(vec2<i32>(entry.xy)==cell) {
            return vec3<u32>(vec2<u32>(entry.zw),select(0u,1u,fract(entry.w)>0.1));
        }
    }
    return vec3<u32>(0u);
}
fn road_network_segment(reference: u32) -> u32 {
    let index=u32(road_cut_data[NETWORK_REFS+reference/4u][reference%4u]);
    return NETWORK_SEGMENTS+index*2u;
}
fn road_network_local(direction: vec3<f32>, height: f32) -> vec3<f32> {
    let radius=road_cut_data[0].x;
    let cosine=dot(direction,road_cut_data[3].xyz);
    return vec3<f32>(vec2<f32>(dot(direction,road_cut_data[1].xyz),
        dot(direction,road_cut_data[2].xyz))*(radius+height),
        (cosine-1.0)*radius+cosine*height);
}
fn road_network_terrain_cut(direction: vec3<f32>, uncut: f32) -> RoadTerrainCut {
    var result=RoadTerrainCut(uncut,direction,false,false);
    let up=road_cut_data[3].xyz;
    let cosine=dot(direction,up);
    if cosine<0.99 { return result; }
    for (var iteration=0u; iteration<2u; iteration+=1u) {
        let p=road_network_local(direction,result.height);
        let list=road_network_cell(p.xy);
        result=RoadTerrainCut(uncut,direction,false,false);
        for (var j=0u; j<min(list.y,64u); j+=1u) {
            let index=road_network_segment(list.x+j);
            let a=road_cut_data[index];
            let b=road_cut_data[index+1u];
            if b.w>0.5 { continue; }
            let delta=b.xyz-a.xyz;
            let length2=dot(delta.xy,delta.xy);
            let parking=b.w < -16.0;
            if !parking && length2<0.00000001 {continue;}
            let length_xy=max(sqrt(length2),0.0001);
            let along=dot(p.xy-a.xy,delta.xy)/length_xy;
            if !parking && (along < -1.0 || along>length_xy+1.0) { continue; }
            let t=clamp(dot(p.xy-a.xy,delta.xy)/max(length2,0.00000001),0.0,1.0);
            let centre=mix(a.xyz,b.xyz,t);
            let displacement=p.xy-centre.xy;
            let offset=length(displacement);
            // Return court ownership with the existing query: cached road
            // banks must not interpolate a soil triangle through the asphalt.
            if parking && offset<NETWORK_PARKING_RADIUS*0.99879546 {result.paved=true;}
            if offset>a.w { continue; }
            let floor=select(select(-6.0,b.w+1.0,b.w<0.0),b.w+32.0,parking);
            let flat_radius=select(9.0,NETWORK_PARKING_CUT_RADIUS,parking);
            let cap=centre.z+floor+max(offset-flat_radius,0.0);
            let raw_height=(cap-(cosine-1.0)*road_cut_data[0].x)/cosine;
            // A finite bank blends out before its index footprint ends. No
            // hard cut at a cell/reach boundary, even with f32 radius rounding.
            let fade=clamp((a.w-offset)/8.0,0.0,1.0);
            let weight=fade*fade*(3.0-2.0*fade);
            let height=mix(uncut,raw_height,weight);
            if height>=result.height { continue; }
            result.height=height;
            result.lowered=true;
            var gradient=vec2<f32>(0.0);
            if t>0.0 && t<1.0 { gradient=delta.xy*delta.z/dot(delta.xy,delta.xy); }
            if offset>flat_radius { gradient+=displacement/max(offset,0.00001); }
            gradient=gradient*weight-(raw_height-uncut)*cosine
                *6.0*fade*(1.0-fade)/8.0*displacement/max(offset,0.00001);
            result.normal=normalize(up-road_cut_data[1].xyz*gradient.x-road_cut_data[2].xyz*gradient.y);
        }
    }
    return result;
}
fn road_network_tunnel_void(direction: vec3<f32>, height: f32) -> f32 {
    if dot(direction,road_cut_data[3].xyz)<0.99 { return -1000000.0; }
    return road_network_void_local(road_network_local(direction,height));
}
fn road_network_void_local(p: vec3<f32>) -> f32 {
    // Uniform no-bore regions and open-only cells are the common case.
    if road_cut_data[0].z<0.5 { return -1000000.0; }
    let list=road_network_cell(p.xy);
    if list.z==0u { return -1000000.0; }
    var result=-1000000.0;
    for (var j=0u; j<min(list.y,64u); j+=1u) {
        let index=road_network_segment(list.x+j);
        let a=road_cut_data[index];
        let b=road_cut_data[index+1u];
        if b.w<0.5 { continue; }
        let delta=b.xyz-a.xyz;
        let length_xy=length(delta.xy);
        let along=dot(p.xy-a.xy,delta.xy)/length_xy;
        let offset=abs(dot(p.xy-a.xy,vec2<f32>(delta.y,-delta.x))/length_xy);
        let z=p.z-a.z-along*delta.z/length_xy;
        let roof=min(9.0-0.25*offset,12.8-1.2*offset);
        result=max(result,min(min(min(7.0-offset,z+6.5),roof+0.5-z),min(along+0.5,length_xy+0.5-along)));
    }
    return result;
}
fn road_terrain_cut(direction: vec3<f32>, uncut_height: f32) -> RoadTerrainCut {
    if !ROAD_CUTS_ENABLED { return RoadTerrainCut(uncut_height,direction,false,false); }
    if road_cut_data[0].w>1.5 { return road_network_terrain_cut(direction,uncut_height); }
    return road_pilot_terrain_cut(direction,uncut_height);
}
fn road_tunnel_void(direction: vec3<f32>, height: f32) -> f32 {
    if !ROAD_CUTS_ENABLED { return -1000000.0; }
    if road_cut_data[0].w>1.5 { return road_network_tunnel_void(direction,height); }
    return road_pilot_tunnel_void(direction,height);
}

// Clear a crown-sized margin around open decks/shoulders, plus excavated banks.
// Covered bore segments deliberately do not clear the forest above their roof.
fn road_clears_tree(direction: vec3<f32>, uncut: f32) -> bool {
    if !ROAD_CUTS_ENABLED || road_cut_data[0].w<0.5 { return false; }
    if dot(direction,road_cut_data[3].xyz)<0.99 { return false; }
    if road_cut_data[0].w>1.5 {
        let p=road_network_local(direction,uncut);
        let list=road_network_cell(p.xy);
        for (var j=0u;j<min(list.y,64u);j+=1u) {
            let index=road_network_segment(list.x+j);
            let a=road_cut_data[index]; let b=road_cut_data[index+1u];
            if b.w>0.5 { continue; }
            let delta=b.xy-a.xy;
            if b.w < -16.0 {
                if distance(p.xy,a.xy)<=NETWORK_PARKING_TREE_RADIUS {return true;}
                continue;
            }
            if dot(delta,delta)<0.00000001 {continue;}
            let along=dot(p.xy-a.xy,delta)/length(delta);
            let margin=select(1.0,25.0,b.w>=-2.0 && b.w<0.0);
            if along < -margin || along>length(delta)+margin { continue; }
            let t=clamp(dot(p.xy-a.xy,delta)/dot(delta,delta),0.0,1.0);
            if distance(p.xy,mix(a.xy,b.xy,t))<=25.0 { return true; }
        }
    } else {
        let p=road_network_local(direction,uncut);
        let last=u32(road_cut_data[0].z)-1u;
        let end=road_cut_data[4u+last].y;
        if p.y < -25.0 || p.y>end+25.0 { return false; }
        let station=clamp(p.y/road_cut_data[0].y,0.0,f32(last));
        let i=min(u32(floor(station)),last-1u);
        let a=road_cut_data[4u+i]; let b=road_cut_data[5u+i];
        if a.w>0.5 { return false; }
        let centre=mix(a.xyz,b.xyz,station-f32(i));
        let slope=(b.x-a.x)/(b.y-a.y);
        if abs(p.x-centre.x)/sqrt(1.0+slope*slope)<=25.0 { return true; }
    }
    return road_terrain_cut(direction,uncut).height<uncut-0.25;
}

fn road_network_open_cut_owns(direction: vec3<f32>, height: f32) -> bool {
    if !ROAD_CUTS_ENABLED || road_cut_data[0].w<1.5 || road_cut_data[3].w<=0.0 {return false;}
    let p=road_network_local(direction,height);
    if distance(p.xy,vec2<f32>(road_cut_data[1].w,road_cut_data[2].w))>road_cut_data[3].w {return false;}
    // Test the actual fragment against the bank planes in one indexed pass.
    // A ground-level apron has almost no excavation, but its slab must still
    // own the pixels down to its underside. A cut-depth-only test leaves
    // interpolated terrain poking through the asphalt (and f32 radial error
    // can exceed the apron excavation itself).
    let list=road_network_cell(p.xy);
    for (var j=0u; j<min(list.y,64u); j+=1u) {
        let index=road_network_segment(list.x+j);
        let a=road_cut_data[index]; let b=road_cut_data[index+1u];
        if b.w>0.5 { continue; }
        let delta=b.xy-a.xy;
        let length2=dot(delta,delta);
        let parking=b.w < -16.0;
        if !parking && length2<0.00000001 {continue;}
        let length_xy=max(sqrt(length2),0.0001);
        let along=dot(p.xy-a.xy,delta)/length_xy;
        if !parking && (along < -0.5 || along > length_xy+0.5) { continue; }
        let t=clamp(dot(p.xy-a.xy,delta)/max(length2,0.00000001),0.0,1.0);
        let centre=mix(a.xyz,b.xyz,t);
        let offset=distance(p.xy,centre.xy);
        // Inscribed radius of the 64-sided pavement avoids erasing terrain
        // beyond the actual mesh at a polygon edge.
        let deck_radius=select(5.1,NETWORK_PARKING_RADIUS*0.99879546,parking);
        if offset<=deck_radius && p.z>centre.z-0.75 { return true; }
        let floor_depth=select(select(6.0,-b.w-1.0,b.w<0.0),-b.w-32.0,parking);
        let flat_radius=select(9.0,NETWORK_PARKING_CUT_RADIUS,parking);
        let cap=centre.z-floor_depth+max(offset-flat_radius,0.0);
        let weight=smoothstep(0.0,8.0,a.w-offset);
        if (p.z-cap)*weight>0.75 { return true; }
    }
    return false;
}

// Actual raster height, including LOD skirts and triangle chord sag. Avoid
// subtracting two planet-sized f32 radii: rationalize the small curvature term.
fn road_raster_fragment_height(relative: vec3<f32>, eye_up: vec3<f32>, eye_height: f32, radius: f32) -> f32 {
    let along=dot(relative,eye_up);
    let transverse=relative-eye_up*along;
    let side2=dot(transverse,transverse);
    let radial=radius+eye_height+along;
    return eye_height+along+side2/(sqrt(radial*radial+side2)+radial);
}

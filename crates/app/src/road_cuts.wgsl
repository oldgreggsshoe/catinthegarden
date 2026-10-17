// Indexed corridor field: four header vectors followed by uniformly y-spaced
// centre/profile samples. No loop over the route, and no fragment-stage search.
@group(2) @binding(21) var<storage, read> road_cut_data: array<vec4<f32>>;
struct RoadTerrainCut { height: f32, normal: vec3<f32>, lowered: bool }
fn road_terrain_cut(direction: vec3<f32>, uncut_height: f32) -> RoadTerrainCut {
    var result = RoadTerrainCut(uncut_height, direction, false);
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
        if abs(xy.x)>3600.0 || xy.y < -500.0 || xy.y>end+500.0 { return RoadTerrainCut(uncut_height,direction,false); }
        let station=clamp(xy.y/settings.y,0.0,f32(last));
        let index=min(u32(floor(station)),last-1u);
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

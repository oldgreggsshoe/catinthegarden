// Camera-centred world-space foam history. The two atlas origins are carried
// explicitly so camera motion reprojects the old mask instead of dragging it.
@group(1) @binding(0) var previous_foam: texture_2d<f32>;
@group(1) @binding(1) var foam_sampler: sampler;
@group(1) @binding(2) var next_foam: texture_storage_2d<rgba8unorm, write>;

struct FoamFrame {
    previous_center: vec4<f32>,
    previous_east: vec4<f32>,
    previous_north: vec4<f32>,
    current_center: vec4<f32>,
    current_east: vec4<f32>,
    current_north: vec4<f32>,
    // x: elapsed seconds, y: whether the previous atlas is valid.
    timing: vec4<f32>,
}
@group(1) @binding(3) var<uniform> foam_frame: FoamFrame;

fn foam_birth_hash(cell: vec3<i32>) -> f32 {
    var value = u32(cell.x) * 0x9e3779b9u
        ^ u32(cell.y) * 0x85ebca6bu
        ^ u32(cell.z) * 0xc2b2ae35u;
    value = (value ^ (value >> 16u)) * 0x7feb352du;
    value = (value ^ (value >> 15u)) * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return f32(value & 65535u) / 65535.0;
}

@compute @workgroup_size(8, 8)
fn cs_foam(@builtin(global_invocation_id) id: vec3<u32>) {
    let dimensions = textureDimensions(next_foam);
    if id.x >= dimensions.x || id.y >= dimensions.y {
        return;
    }
    let atlas_uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(dimensions);
    let offset = (atlas_uv - 0.5) * 512.0;
    let center = foam_frame.current_center.xyz;
    let direction = normalize(center
        + (foam_frame.current_east.xyz * offset.x
            + foam_frame.current_north.xyz * offset.y) / PLANET_RADIUS_METERS);

    var retained = 0.0;
    if foam_frame.timing.y > 0.5 {
        let from_previous = (direction - foam_frame.previous_center.xyz)
            * PLANET_RADIUS_METERS;
        let old_uv = vec2<f32>(
            dot(from_previous, foam_frame.previous_east.xyz),
            dot(from_previous, foam_frame.previous_north.xyz),
        ) / 512.0 + 0.5;
        if all(old_uv >= vec2<f32>(0.0)) && all(old_uv <= vec2<f32>(1.0)) {
            retained = textureSampleLevel(previous_foam, foam_sampler, old_uv, 0.0).r
                * exp(-min(foam_frame.timing.x, 10.0) / 1.5);
        }
    }

    // Resolve the six shortest wind-sea components only. Re-running the full
    // eighteen-wave ocean at every atlas texel costs more than the foam buys.
    let storm_blend = smoothstep(0.15, 0.85,
        clamp(camera.flat_triangle_options.y, 0.0, 1.0));
    var convergence = 0.0;
    for (var i = 12u; i < OCEAN_WAVE_COUNT; i = i + 1u) {
        let spec = OCEAN_WAVE_TABLE[i];
        var amplitude = mix(spec.amplitude_meters, spec.storm_amplitude_meters,
            storm_blend);
        if OCEAN_WIND_ENABLED {
            amplitude *= OCEAN_WIND_WEIGHTS[i];
        }
        if OCEAN_LARGE_SWELL_ONLY || !OCEAN_WAVES_ENABLED {
            amplitude = 0.0;
        }
        let wave = gerstner_wave(direction, spec.axis, spec.wavelength_meters,
            amplitude, spec.speed_meters_per_second * OCEAN_WIND_SPEED_SIGNS[i],
            spec.steepness, camera.projection.z, 1000.0);
        convergence += wave.convergence;
    }
    let peak = smoothstep(0.0003, 0.0018, convergence);
    let cell = vec3<i32>(floor(direction * (PLANET_RADIUS_METERS / 7.0)));
    let fleck = smoothstep(0.72, 0.98, foam_birth_hash(cell));
    let born = peak * 0.65 * fleck;
    textureStore(next_foam, vec2<i32>(id.xy), vec4<f32>(max(retained, born), 0.0, 0.0, 1.0));
}

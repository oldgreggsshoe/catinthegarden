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
    // x: elapsed seconds, y: whether the previous atlas is valid, zw: unit
    // wind direction in the FFT (u, v) axes (FFT mode).
    timing: vec4<f32>,
    // Ship waterline origin from the atlas centre (east, north m), bow slam
    // intensity, 1 if a ship is present.
    ship: vec4<f32>,
    // Ship forward (east, north), hull half-length, half-beam.
    ship_axes: vec4<f32>,
}
@group(1) @binding(3) var<uniform> foam_frame: FoamFrame;
@group(1) @binding(4) var foam_fft_map: texture_2d_array<f32>;
@group(1) @binding(5) var foam_fft_sampler: sampler;
@group(1) @binding(6) var<uniform> foam_fft_view: OceanFftView;

// Fold-Jacobian parts (dDu/du, dDv/dv, dDu/dv, dDv/du) of one cascade at
// tangent-plane offset `local` metres from the camera, filtered to `width`.
fn foam_fft_jacobian(cascade_index: u32, local: vec2<f32>, width: f32) -> vec4<f32> {
    let entry = foam_fft_view.cascade[cascade_index];
    let texel_meters = entry.z / 256.0;
    let lod = clamp(log2(max(width / texel_meters, 1.0)), 0.0, 8.0);
    let texel = exp2(lod) / 256.0;
    let uv = entry.xy + local / entry.z;
    let s0 = textureSampleLevel(foam_fft_map, foam_fft_sampler, uv, cascade_index, lod);
    let su = textureSampleLevel(foam_fft_map, foam_fft_sampler, uv + vec2<f32>(texel, 0.0), cascade_index, lod);
    let sv = textureSampleLevel(foam_fft_map, foam_fft_sampler, uv + vec2<f32>(0.0, texel), cascade_index, lod);
    return vec4<f32>(su.y - s0.y, sv.z - s0.z, sv.y - s0.y, su.z - s0.z) / (entry.z * texel);
}

// Wind-driven surface drift of FFT fold foam: it slides downwind and smears
// along the wind into streaks.
const FOAM_FFT_DRIFT_METERS_PER_SECOND: f32 = 0.8;
// Seconds for FFT fold foam to fade to 1/e once its crest has passed.
const FOAM_FFT_DECAY_SECONDS: f32 = 2.5;
const FOAM_FFT_ATLAS_JACOBIAN_ONSET: f32 = 0.66;
const FOAM_FFT_ATLAS_JACOBIAN_FULL: f32 = 0.36;

fn foam_birth_hash(cell: vec3<i32>) -> f32 {
    var value = u32(cell.x) * 0x9e3779b9u
        ^ u32(cell.y) * 0x85ebca6bu
        ^ u32(cell.z) * 0xc2b2ae35u;
    value = (value ^ (value >> 16u)) * 0x7feb352du;
    value = (value ^ (value >> 15u)) * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return f32(value & 65535u) / 65535.0;
}

// Water churned against the hull: a band just outside the waterline outline
// (the same plan shape as ship.rs `half_beam_meters`), heaviest at the bow and
// stronger when it slams. Left in the world-fixed history, it trails as a wake.
fn ship_hull_foam(offset: vec2<f32>) -> f32 {
    let forward = foam_frame.ship_axes.xy;
    let port = vec2<f32>(-forward.y, forward.x);
    let relative = offset - foam_frame.ship.xy;
    let half_length = foam_frame.ship_axes.z;
    let t = dot(relative, forward) / half_length;
    let across = abs(dot(relative, port));
    let tc = clamp(t, -1.0, 1.0);
    let shape = select(1.0 - 0.2 * tc * tc, pow(max(1.0 - tc * tc, 0.0), 0.6), tc >= 0.0);
    let half_beam = foam_frame.ship_axes.w * shape;
    // Distance outside the hull outline, ends included.
    let beyond_ends = max(abs(t) - 1.0, 0.0) * half_length;
    let outside = max(across - half_beam, 0.0) + beyond_ends;
    let inside = across < half_beam && abs(t) <= 1.0;
    if inside {
        return 0.0;
    }
    let band = 1.0 - smoothstep(0.0, 2.5 + 2.0 * foam_frame.ship.z, outside);
    let bow = 0.35 + 0.65 * smoothstep(-0.2, 0.9, t);
    return band * bow * (0.45 + 0.55 * foam_frame.ship.z);
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
        let wind_world = foam_fft_view.axis_u.xyz * foam_frame.timing.z
            + foam_fft_view.axis_v.xyz * foam_frame.timing.w;
        let drift = select(
            vec3<f32>(0.0),
            wind_world * (FOAM_FFT_DRIFT_METERS_PER_SECOND * min(foam_frame.timing.x, 1.0)),
            OCEAN_FFT_ENABLED,
        );
        let from_previous = (direction - foam_frame.previous_center.xyz)
            * PLANET_RADIUS_METERS - drift;
        let old_uv = vec2<f32>(
            dot(from_previous, foam_frame.previous_east.xyz),
            dot(from_previous, foam_frame.previous_north.xyz),
        ) / 512.0 + 0.5;
        if all(old_uv >= vec2<f32>(0.0)) && all(old_uv <= vec2<f32>(1.0)) {
            let decay_seconds = select(1.5, FOAM_FFT_DECAY_SECONDS, OCEAN_FFT_ENABLED);
            var previous = textureSampleLevel(previous_foam, foam_sampler, old_uv, 0.0).r;
            if OCEAN_FFT_ENABLED {
                // Small feedback blur, stretched along the wind: foam spreads
                // as it ages, mostly downwind and upwind, into streaks.
                let step = 1.0 / vec2<f32>(dimensions);
                var along = vec2<f32>(
                    dot(wind_world, foam_frame.previous_east.xyz),
                    dot(wind_world, foam_frame.previous_north.xyz),
                );
                along = select(vec2<f32>(1.0, 0.0), normalize(along), dot(along, along) > 1.0e-6);
                let across = vec2<f32>(-along.y, along.x);
                let spread = 0.35 * (textureSampleLevel(previous_foam, foam_sampler, old_uv + along * step, 0.0).r
                    + textureSampleLevel(previous_foam, foam_sampler, old_uv - along * step, 0.0).r)
                    + 0.15 * (textureSampleLevel(previous_foam, foam_sampler, old_uv + across * step, 0.0).r
                    + textureSampleLevel(previous_foam, foam_sampler, old_uv - across * step, 0.0).r);
                let blur = mix(previous, spread, 1.0 - exp(-min(foam_frame.timing.x, 1.0) / 0.5));
                previous = blur;
            }
            retained = previous * exp(-min(foam_frame.timing.x, 10.0) / decay_seconds);
        }
    }

    var born = 0.0;
    if OCEAN_FFT_ENABLED {
        // The texel's offset from the camera is exact in metres; no f32
        // planet-radius subtraction.
        let world_offset = foam_frame.current_east.xyz * offset.x
            + foam_frame.current_north.xyz * offset.y;
        let local = vec2<f32>(
            dot(world_offset, foam_fft_view.axis_u.xyz),
            dot(world_offset, foam_fft_view.axis_v.xyz),
        );
        let texel_meters = 512.0 / f32(dimensions.x);
        let jacobian = foam_fft_jacobian(0u, local, texel_meters)
            + foam_fft_jacobian(1u, local, texel_meters)
            + foam_fft_jacobian(2u, local, texel_meters)
            + foam_fft_jacobian(3u, local, texel_meters) * foam_fft_view.gain.z;
        // 2m texels average away the finer cascades' sharpest folds, so the
        // atlas births foam at a gentler Jacobian than the per-pixel rule.
        let scaled = jacobian * foam_fft_view.gain.x;
        // Drawn surface is x0 - D, so it folds where I - grad D does: crests.
        let j = (1.0 - scaled.x) * (1.0 - scaled.y) - scaled.z * scaled.w;
        born = smoothstep(FOAM_FFT_ATLAS_JACOBIAN_ONSET, FOAM_FFT_ATLAS_JACOBIAN_FULL, j);
    } else {
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
        born = peak * 0.65 * fleck;
    }
    if foam_frame.ship.w > 0.5 {
        born = max(born, ship_hull_foam(offset));
    }
    textureStore(next_foam, vec2<i32>(id.xy), vec4<f32>(max(retained, born), 0.0, 0.0, 1.0));
}

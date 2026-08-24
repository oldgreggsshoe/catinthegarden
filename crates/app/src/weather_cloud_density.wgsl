// Shared cloud-density evaluation contract.
//
// The including shader supplies `weather`, `cloud_field_current`,
// `cloud_field_previous`, and `cloud_field_sampler` with the same bindings as
// the weather shell. `t` selects the shell layer (0 = lower, 1 = upper).
// Terrain shadows and cloud impostor spawn must include this source rather
// than reimplementing the field/flow/noise/posterisation path.

fn hash_noise(point: vec3<f32>) -> f32 {
    return fract(sin(dot(point, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

fn value_noise(point: vec3<f32>) -> f32 {
    let cell = floor(point);
    let local = smoothstep(vec3<f32>(0.0), vec3<f32>(1.0), fract(point));
    let n000 = hash_noise(cell);
    let n100 = hash_noise(cell + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash_noise(cell + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash_noise(cell + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash_noise(cell + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash_noise(cell + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash_noise(cell + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash_noise(cell + vec3<f32>(1.0, 1.0, 1.0));
    let low = mix(mix(n000, n100, local.x), mix(n010, n110, local.x), local.y);
    let high = mix(mix(n001, n101, local.x), mix(n011, n111, local.x), local.y);
    return mix(low, high, local.z) * 2.0 - 1.0;
}

// This is written as a column-vector multiply explicitly. WGSL matrix
// constructors are column-major; keeping the rotation explicit avoids a
// silent row/column transpose when this source is reused by another shader.
fn rotate_noise(point: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.00 * point.x + 0.80 * point.y + 0.60 * point.z,
        -0.80 * point.x + 0.36 * point.y - 0.48 * point.z,
        -0.60 * point.x - 0.48 * point.y + 0.64 * point.z,
    );
}

fn cloud_noise(direction: vec3<f32>, shell_index: u32, octave_count: u32) -> f32 {
    let scale = weather.noise_scale * select(1.0, 1.7, shell_index == 1u);
    let shell_offset = select(
        vec3<f32>(0.0, 0.0, 0.0),
        vec3<f32>(17.3, -9.1, 5.7),
        shell_index == 1u,
    );
    var point = direction * scale + shell_offset;
    var value = 0.0;
    var amplitude = 0.5;
    var amplitude_sum = 0.0;
    for (var octave = 0u; octave < min(octave_count, 5u); octave = octave + 1u) {
        value = value + amplitude * value_noise(point);
        amplitude_sum = amplitude_sum + amplitude;
        point = rotate_noise(point) * 2.02;
        amplitude = amplitude * 0.5;
    }
    return value / amplitude_sum;
}

fn flow_warp(direction: vec3<f32>, shell_index: u32) -> vec3<f32> {
    let flow = normalize(vec3<f32>(-direction.z, 0.18, direction.x));
    // This is a fixed spatial breakup, not a second transport mechanism.
    // Cloud motion comes from the interpolated, wind-advected cubemap. A
    // time-varying phase here (or a global longitude rotation) makes one
    // rendered shell orbit independently of the simulated weather field.
    let shell_phase = select(0.0, 1.618, shell_index == 1u);
    let amount = sin(shell_phase + dot(direction, flow) * 18.0) * 0.018;
    return normalize(direction + flow * amount);
}

struct CloudSample {
    density: f32,
    storm: f32,
    precipitation: f32,
}

fn cloudSampleWithOctaves(dir: vec3<f32>, t: f32, octave_count: u32) -> CloudSample {
    let shell_index = select(0u, 1u, t >= 0.5);
    let base_direction = normalize(dir);
    let field_direction = flow_warp(base_direction, shell_index);
    let current = textureSampleLevel(cloud_field_current, cloud_field_sampler, field_direction, 0.0);
    let previous = textureSampleLevel(cloud_field_previous, cloud_field_sampler, field_direction, 0.0);
    let field = mix(previous, current, weather.blend);
    let detail = cloud_noise(field_direction, shell_index, octave_count);
    let noise = detail * weather.noise_strength;
    let shell_cloud = select(field.r, field.r * 0.72, shell_index == 1u);
    let coverage = clamp(0.82 + noise, 0.0, 1.0);
    let density = shell_cloud * coverage;
    let posterized = smoothstep(0.08, 0.26, density);
    // Before condensation has built cloud water, retain sparse humid wisps
    // rather than a planet-wide constant veil. Native field resolution and
    // rotated detail keep this startup signal from reading as fog or a grid.
    let humidity_precursor = smoothstep(0.54, 0.76, field.b);
    let precursor_breakup = smoothstep(-0.16, 0.18, detail);
    let lower_precursor = humidity_precursor * precursor_breakup * 0.13;
    // Condensed water can concentrate into narrow weather fronts after many
    // simulated days, leaving an entire orbital hemisphere visually clear.
    // Use the upper layer for sparse fair-weather cirrus. A thin presentation
    // floor prevents hemisphere-scale holes in the coarse one-layer climate,
    // while local vapour raises its density. Rotated detail retains broad
    // clear gaps instead of restoring the rejected global veil.
    let upper_precursor = mix(
        0.45,
        1.0,
        smoothstep(0.06, 0.34, field.b),
    ) * smoothstep(-0.08, 0.22, detail)
        * 0.24;
    // Keep weak condensate thinner while driving genuinely stormy cells
    // toward opacity. This widens contrast without filling the clear gaps or
    // changing the sparse startup precursor that appears before condensation.
    // The live field's strongest mature cells sit around 0.3 storm intensity;
    // reach optical thickness there rather than reserving opacity for values
    // the current weather model cannot produce.
    let storm_amount = smoothstep(0.05, 0.32, field.g);
    let condensed = posterized
        * mix(0.22, 1.0, storm_amount)
        * select(1.0, 0.78, shell_index == 1u);
    let precursor_density = select(
        lower_precursor * 0.32,
        upper_precursor,
        shell_index == 1u,
    );
    return CloudSample(
        clamp(max(condensed, precursor_density), 0.0, 1.0),
        clamp(field.g, 0.0, 1.0),
        clamp(field.a, 0.0, 1.0),
    );
}

fn cloudDensityWithOctaves(dir: vec3<f32>, t: f32, octave_count: u32) -> f32 {
    return cloudSampleWithOctaves(dir, t, octave_count).density;
}

fn cloudSample(dir: vec3<f32>, t: f32) -> CloudSample {
    return cloudSampleWithOctaves(dir, t, 5u);
}

fn cloudDensity(dir: vec3<f32>, t: f32) -> f32 {
    return cloudSample(dir, t).density;
}

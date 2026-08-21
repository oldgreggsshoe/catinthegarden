// Shared cloud-density evaluation contract.
//
// The including shader supplies `weather`, `cloud_field_current`,
// `cloud_field_previous`, and `cloud_field_sampler` with the same bindings as
// the weather shell. `t` selects the shell layer (0 = lower, 1 = upper).
// Terrain shadows and cloud impostor spawn must include this source rather
// than reimplementing the field/flow/noise/posterisation path.

fn rotate_drift(direction: vec3<f32>) -> vec3<f32> {
    let sine = sin(weather.drift_radians);
    let cosine = cos(weather.drift_radians);
    return vec3<f32>(
        direction.x * cosine - direction.z * sine,
        direction.y,
        direction.x * sine + direction.z * cosine,
    );
}

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
    let phase = weather.drift_radians * select(0.8, 0.45, shell_index == 1u);
    let amount = sin(phase + dot(direction, flow) * 18.0) * 0.018;
    return normalize(direction + flow * amount);
}

fn cloudDensityWithOctaves(dir: vec3<f32>, t: f32, octave_count: u32) -> f32 {
    let shell_index = select(0u, 1u, t >= 0.5);
    let base_direction = normalize(dir);
    let field_direction = flow_warp(rotate_drift(base_direction), shell_index);
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
    let precursor = humidity_precursor
        * precursor_breakup
        * select(0.13, 0.075, shell_index == 1u);
    return max(posterized, precursor)
        * (0.32 + 0.58 * field.g)
        * select(1.0, 0.62, shell_index == 1u);
}

fn cloudDensity(dir: vec3<f32>, t: f32) -> f32 {
    return cloudDensityWithOctaves(dir, t, 5u);
}

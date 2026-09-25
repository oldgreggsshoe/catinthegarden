// Wind-blown spray from folding FFT crests: particle update and spawning.
//
// Particles live in the FFT tangent plane relative to the camera (u, v metres)
// with height above sea level, so f32 stays precise. Each frame the camera's
// own tangent-plane movement is subtracted. Dead particles try a random point
// near the camera and are born only where the displaced surface is folding
// (the same Jacobian the fold foam uses, much stricter), thrown upward and
// dragged toward the wind until gravity brings them down.

struct Particle {
    // u, v (m from camera), height above sea level (m), age (s).
    position: vec4<f32>,
    // du, dv, dh (m/s), lifetime (s); lifetime <= 0 means dead.
    velocity: vec4<f32>,
}

struct SprayFrame {
    axis_u: vec4<f32>,
    axis_v: vec4<f32>,
    // x, y: camera shift in (u, v) metres since last update; z: dt; w: frame.
    shift_dt: vec4<f32>,
    // x, y: unit wind direction in (u, v); z: wind speed (m/s).
    wind: vec4<f32>,
    // x: spawn radius (m); y: births per second per fully folded particle
    // attempt; z: choppiness; w: swell height (m).
    params: vec4<f32>,
    // Ship waterline origin relative to the camera (u, v, height), intensity.
    ship_origin: vec4<f32>,
    // Ship forward (u, v), hull half-length, half-beam.
    ship_axes: vec4<f32>,
    // Ship velocity (u, v, up).
    ship_velocity: vec4<f32>,
    // Slam intensity at hull stations t = -0.9, -0.3, 0.3, 0.9, port then
    // starboard.
    ship_port: vec4<f32>,
    ship_starboard: vec4<f32>,
}

struct OceanFftView {
    axis_u: vec4<f32>,
    axis_v: vec4<f32>,
    cascade: array<vec4<f32>, 4>,
    gain: vec4<f32>,
    second_order: vec4<f32>,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> frame: SprayFrame;
@group(0) @binding(2) var fft_map: texture_2d_array<f32>;
@group(0) @binding(3) var fft_sampler: sampler;
@group(0) @binding(4) var<uniform> fft_view: OceanFftView;

const GRAVITY: f32 = 9.81;
// The first particle slots belong to the ship's bow (ocean_spray.rs).
const SHIP_SPRAY_SLOTS: u32 = 2048u;
// Bow births per second per slot attempt at full slam.
const SHIP_SPRAY_RATE: f32 = 30.0;
// Horizontal air drag toward the wind, and vertical drag, per second.
const WIND_DRAG: f32 = 1.2;
const VERTICAL_DRAG: f32 = 0.6;
// Spray is born only where the drawn surface is close to folding over.
const SPRAY_JACOBIAN_ONSET: f32 = 0.40;
const SPRAY_JACOBIAN_FULL: f32 = 0.0;

struct SprayField {
    height: f32,
    displacement: vec2<f32>,
    // dDu/du, dDv/dv, dDu/dv, dDv/du.
    jacobian: vec4<f32>,
}

fn hash(value: u32) -> u32 {
    var x = value;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

fn unit_random(seed: u32) -> f32 {
    return f32(hash(seed) & 0xffffffu) / 16777216.0;
}

fn spray_cascade(index: u32, local: vec2<f32>, weight: f32, field: ptr<function, SprayField>) {
    let entry = fft_view.cascade[index];
    let uv = entry.xy + local / entry.z;
    let texel = 1.0 / 256.0;
    let step = entry.z * texel;
    let s0 = textureSampleLevel(fft_map, fft_sampler, uv, index, 0.0);
    let su = textureSampleLevel(fft_map, fft_sampler, uv + vec2<f32>(texel, 0.0), index, 0.0);
    let sv = textureSampleLevel(fft_map, fft_sampler, uv + vec2<f32>(0.0, texel), index, 0.0);
    (*field).height += s0.x * weight;
    (*field).displacement += s0.yz * weight;
    (*field).jacobian += vec4<f32>(su.y - s0.y, sv.z - s0.z, sv.y - s0.y, su.z - s0.z)
        * (weight / step);
}

fn spray_field(local: vec2<f32>) -> SprayField {
    var field = SprayField(0.0, vec2<f32>(0.0), vec4<f32>(0.0));
    spray_cascade(0u, local, 1.0, &field);
    spray_cascade(1u, local, 1.0, &field);
    spray_cascade(2u, local, 1.0, &field);
    spray_cascade(3u, local, frame.params.w, &field);
    return field;
}

// Slam intensity at hull position t (-1 stern, +1 stem) from the four stations
// at t = -0.9, -0.3, 0.3, 0.9 (ocean_spray.rs SHIP_STATIONS).
fn station_intensity(values: vec4<f32>, t: f32) -> f32 {
    let x = clamp((t + 0.9) / 0.6, 0.0, 3.0);
    let i = min(u32(floor(x)), 2u);
    return mix(values[i], values[i + 1u], x - f32(i));
}

// Waterline half-beam over the hull half-beam (ship.rs half_beam_meters).
fn hull_shape(t: f32) -> f32 {
    return select(1.0 - 0.2 * t * t, pow(max(1.0 - t * t, 0.0), 0.6), t >= 0.0);
}

@compute @workgroup_size(64)
fn cs_spray(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= arrayLength(&particles) {
        return;
    }
    var particle = particles[index];
    let dt = frame.shift_dt.z;
    let alive = particle.velocity.w > 0.0 && particle.position.w < particle.velocity.w;
    if alive {
        let wind_velocity = frame.wind.xy * frame.wind.z;
        var velocity = particle.velocity.xyz;
        velocity = vec3<f32>(
            velocity.xy + (wind_velocity - velocity.xy) * (1.0 - exp(-WIND_DRAG * dt)),
            velocity.z * exp(-VERTICAL_DRAG * dt) - GRAVITY * dt,
        );
        var position = particle.position.xyz + velocity * dt;
        position = vec3<f32>(position.xy - frame.shift_dt.xy, position.z);
        particle.position = vec4<f32>(position, particle.position.w + dt);
        particle.velocity = vec4<f32>(velocity, particle.velocity.w);
        // Out of the spawn disc: retire rather than draw spray nobody sees.
        if length(position.xy) > frame.params.x * 1.5 {
            particle.velocity.w = 0.0;
        }
        particles[index] = particle;
        return;
    }

    let seed = index * 747796405u + u32(frame.shift_dt.w) * 2891336453u;
    if index < SHIP_SPRAY_SLOTS {
        // Hull spray: torn off the waterline wherever the water slams against
        // the hull (a floating hull is hit all round, not just at the bow),
        // thrown outward along the local hull normal and up, carried with the
        // hull and the wind.
        let r1 = unit_random(seed ^ 0x9e3779b9u);
        let r2 = unit_random(seed ^ 0x85ebca6bu);
        let r3 = unit_random(seed ^ 0xc2b2ae35u);
        let r4 = unit_random(seed ^ 0x27d4eb2fu);
        let t = mix(-0.98, 0.98, r1);
        let side = select(-1.0, 1.0, r2 < 0.5);
        let intensity = station_intensity(
            select(frame.ship_starboard, frame.ship_port, side > 0.0),
            t,
        );
        if frame.ship_origin.w <= 0.0 || intensity <= 0.0
            || unit_random(seed ^ 0x2545f491u) >= intensity * SHIP_SPRAY_RATE * dt
        {
            particle.velocity.w = 0.0;
            particles[index] = particle;
            return;
        }
        let half_length = frame.ship_axes.z;
        let half_beam = frame.ship_axes.w * hull_shape(t);
        let forward = frame.ship_axes.xy;
        let port = vec2<f32>(-forward.y, forward.x);
        let place = frame.ship_origin.xy + forward * (t * half_length) + port * (side * half_beam);
        // Outline normal: raked forward where the bow narrows, aft toward the
        // stern, straight aft off the transom.
        let slope = frame.ship_axes.w
            * (hull_shape(t + 0.01) - hull_shape(t - 0.01)) / (0.02 * half_length);
        var outward = normalize(port * side - forward * slope);
        outward = normalize(mix(outward, -forward, smoothstep(-0.88, -0.98, t)));
        let speed = (3.0 + 8.0 * intensity * r3);
        let horizontal = outward * speed + frame.ship_velocity.xy
            + frame.wind.xy * frame.wind.z * 0.15;
        // Up to ~17m/s: storm bow spray clears the 6m freeboard and the deck.
        let lift = frame.ship_velocity.z * 0.5 + 5.0 + 12.0 * intensity * r4;
        particle.position = vec4<f32>(place, frame.ship_origin.z + 0.3, 0.0);
        particle.velocity = vec4<f32>(horizontal, lift, 0.9 + 1.2 * r3);
        particles[index] = particle;
        return;
    }

    // Dead: one spawn attempt at a random point, density weighted toward the
    // camera (radius linear in the random number), where spray is visible.
    let radius = frame.params.x * unit_random(seed);
    let angle = 6.2831853 * unit_random(seed ^ 0x68e31da4u);
    let local = radius * vec2<f32>(cos(angle), sin(angle));
    let field = spray_field(local);
    let chop = frame.params.z;
    let j = field.jacobian * fft_view.gain.x;
    let jacobian = (1.0 - chop * j.x) * (1.0 - chop * j.y) - chop * chop * j.z * j.w;
    let fold = smoothstep(SPRAY_JACOBIAN_ONSET, SPRAY_JACOBIAN_FULL, jacobian);
    let chance = fold * frame.params.y * dt;
    if frame.params.y <= 0.0 || unit_random(seed ^ 0x2545f491u) >= chance {
        particle.velocity.w = 0.0;
        particles[index] = particle;
        return;
    }
    let divergence = clamp((j.x + j.y), -0.6, 0.6);
    let height = field.height * fft_view.gain.x
        * (1.0 + fft_view.second_order.x * divergence) - fft_view.second_order.y;
    // Born at the drawn crest: the label point moved by -D.
    let drawn = local - chop * field.displacement * fft_view.gain.x;
    let r1 = unit_random(seed ^ 0x9e3779b9u);
    let r2 = unit_random(seed ^ 0x85ebca6bu);
    let r3 = unit_random(seed ^ 0xc2b2ae35u);
    let lift = 1.0 + 3.5 * fold * r1;
    let sideways = (vec2<f32>(r2, r3) - 0.5) * 2.0;
    let horizontal = frame.wind.xy * frame.wind.z * (0.2 + 0.3 * r2) + sideways;
    particle.position = vec4<f32>(drawn, height, 0.0);
    particle.velocity = vec4<f32>(horizontal, lift, 1.0 + 1.6 * r3);
    particles[index] = particle;
}

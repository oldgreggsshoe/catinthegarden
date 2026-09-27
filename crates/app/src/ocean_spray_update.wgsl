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
    // x: height above the water surface beneath it (m), for the draw fade.
    extra: vec4<f32>,
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
    // Steep or breaking waves running into the same stations.
    ship_port_impact: vec4<f32>,
    ship_starboard_impact: vec4<f32>,
    // Altitude of the water against the hull at the same stations.
    ship_port_water: vec4<f32>,
    ship_starboard_water: vec4<f32>,
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
// Bow births per second per slot attempt at full slam, on the tuned hull.
const SHIP_SPRAY_RATE: f32 = 30.0;
// Ship spray was tuned against a hull of this half-length (84m, ship.rs
// SPLASH_TUNED_HALF_LENGTH_METERS). Any other hull scales it by Froude
// similarity: lengths by the size ratio, speeds and times by its root.
const SHIP_SPRAY_TUNED_HALF_LENGTH: f32 = 42.0;
// Horizontal air drag toward the wind, and vertical drag, per second: the
// ship's sheets of water.
const WIND_DRAG: f32 = 1.2;
const VERTICAL_DRAG: f32 = 0.6;
// Crest spray, after Sea of Thieves: a thin sheet of mist torn off the crest
// line, brightest where it leaves the water, combed downwind and gone within
// a second. Fine mist follows the air: it is at the wind's speed within a
// quarter second and falls at ~3 m/s, rather than flying on ballistically
// and falling at 16 m/s as a streak for a frame before it hit the water.
const CREST_WIND_DRAG: f32 = 4.0;
const CREST_VERTICAL_DRAG: f32 = 3.0;
const CREST_LIFETIME_SECONDS: f32 = 0.35;
const CREST_LIFETIME_SPREAD_SECONDS: f32 = 0.45;
// Spray is born only where the drawn surface is close to folding over.
const SPRAY_JACOBIAN_ONSET: f32 = 0.40;
const SPRAY_JACOBIAN_FULL: f32 = 0.0;

struct SprayField {
    height: f32,
    displacement: vec2<f32>,
    // dDu/du, dDv/dv, dDu/dv, dDv/du.
    jacobian: vec4<f32>,
    // Sum over the geometry bands of (k/2)(h^2 - |D|^2), before strength and
    // gain (ocean_fft.rs `second_order`).
    stokes: f32,
    // Height of the wind sea alone (every cascade but the swell).
    wind_height: f32,
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

fn spray_cascade(index: u32, local: vec2<f32>, weight: f32, geometry: bool, field: ptr<function, SprayField>) {
    let entry = fft_view.cascade[index];
    let uv = entry.xy + local / entry.z;
    let texel = 1.0 / 256.0;
    let step = entry.z * texel;
    // Half-texel central differences, as ocean_fft_cascade takes them.
    let half = 0.5 * texel;
    let se = textureSampleLevel(fft_map, fft_sampler, uv + vec2<f32>(half, 0.0), index, 0.0);
    let sw = textureSampleLevel(fft_map, fft_sampler, uv - vec2<f32>(half, 0.0), index, 0.0);
    let sn = textureSampleLevel(fft_map, fft_sampler, uv + vec2<f32>(0.0, half), index, 0.0);
    let ss = textureSampleLevel(fft_map, fft_sampler, uv - vec2<f32>(0.0, half), index, 0.0);
    let s0 = 0.25 * (se + sw + sn + ss);
    (*field).height += s0.x * weight;
    if index != 3u {
        (*field).wind_height += s0.x * weight;
    }
    (*field).displacement += s0.yz * weight;
    (*field).jacobian += vec4<f32>(se.y - sw.y, sn.z - ss.z, sn.y - ss.y, se.z - sw.z)
        * (weight / step);
    if geometry {
        // This band's own second-order term, at its mean wavenumber (entry.w).
        let h = s0.x * weight;
        let d = s0.yz * weight;
        (*field).stokes += 0.5 * entry.w * (h * h - dot(d, d));
    }
}

fn spray_field(local: vec2<f32>) -> SprayField {
    var field = SprayField(0.0, vec2<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0);
    spray_cascade(0u, local, 1.0, true, &field);
    spray_cascade(1u, local, 1.0, true, &field);
    spray_cascade(2u, local, 1.0, false, &field);
    spray_cascade(3u, local, frame.params.w, true, &field);
    return field;
}

// Water surface height (m above sea level) at a sampled field, with the
// shader's second-order crest term.
fn spray_surface_height(field: SprayField) -> f32 {
    let gain = fft_view.gain.x;
    return field.height * gain + fft_view.second_order.x * gain * gain * field.stokes;
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
        let from_ship = index < SHIP_SPRAY_SLOTS;
        let wind_drag = select(CREST_WIND_DRAG, WIND_DRAG, from_ship);
        let vertical_drag = select(CREST_VERTICAL_DRAG, VERTICAL_DRAG, from_ship);
        var velocity = particle.velocity.xyz;
        velocity = vec3<f32>(
            velocity.xy + (wind_velocity - velocity.xy) * (1.0 - exp(-wind_drag * dt)),
            velocity.z * exp(-vertical_drag * dt) - GRAVITY * dt,
        );
        var position = particle.position.xyz + velocity * dt;
        position = vec3<f32>(position.xy - frame.shift_dt.xy, position.z);
        particle.position = vec4<f32>(position, particle.position.w + dt);
        particle.velocity = vec4<f32>(velocity, particle.velocity.w);
        // Spray that falls back into the sea is gone: retire it at the
        // surface rather than letting it fly on underwater. Only once it is
        // falling: the surface here is estimated at the undisplaced (label)
        // point, which on a steep crest can stand above the water the spray
        // was actually born from, and rising spray retired against it blinked
        // out the frame after it appeared.
        let clearance = position.z - spray_surface_height(spray_field(position.xy));
        particle.extra = vec4<f32>(clearance, 0.0, 0.0, 0.0);
        if clearance < 0.0 && velocity.z < 0.0 {
            particle.velocity.w = 0.0;
        }
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
        let slam = station_intensity(
            select(frame.ship_starboard, frame.ship_port, side > 0.0),
            t,
        );
        let impact = station_intensity(
            select(frame.ship_starboard_impact, frame.ship_port_impact, side > 0.0),
            t,
        );
        let intensity = max(slam, impact);
        let size = frame.ship_axes.z / SHIP_SPRAY_TUNED_HALF_LENGTH;
        let froude = sqrt(size);
        // A wave striking the hull throws more water than the hull slamming.
        // Shorter-lived spray off a smaller hull is born faster, so the same
        // number of particles make up its (smaller) sheet.
        if frame.ship_origin.w <= 0.0 || intensity <= 0.0
            || unit_random(seed ^ 0x2545f491u)
                >= intensity * (SHIP_SPRAY_RATE / froude) * dt * (1.0 + impact)
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
        // Slam: thrown outward off the hull. Wave impact: a sheet driven up
        // the hull side, barely outward, then carried by the wind.
        let struck = impact > slam;
        let speed = select(3.0 + 8.0 * intensity * r3, 1.0 + 2.5 * impact * r3, struck) * froude;
        let horizontal = outward * speed + frame.ship_velocity.xy
            + frame.wind.xy * frame.wind.z * select(0.15, 0.3, struck);
        // Up to ~17m/s on the tuned hull: storm bow spray clears its 6m
        // freeboard and the deck.
        let lift = frame.ship_velocity.z * 0.5
            + select(5.0 + 12.0 * intensity * r4, 8.0 + 14.0 * impact * r4, struck) * froude;
        // Where the water actually meets the hull at this station (measured
        // on the CPU at the drawn surface, so a pitched or heaving hull throws
        // from its own waterline, not from the midship height or a nearby
        // crest), just above it so it is not retired at birth.
        let water_here = station_intensity(
            select(frame.ship_starboard_water, frame.ship_port_water, side > 0.0),
            t,
        );
        particle.position = vec4<f32>(place, water_here + 0.2 * size, 0.0);
        particle.extra = vec4<f32>(1.0e3, 0.0, 0.0, 0.0);
        particle.velocity = vec4<f32>(
            horizontal,
            lift,
            select(0.9 + 1.2 * r3, 1.2 + 1.3 * r3, struck) * froude,
        );
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
    // From the tops of the wind waves, not their faces: pinched water well
    // above the wind sea's own mean, wherever it rides on the swell (a crest
    // in a swell's trough is still a crest to the wind). Scaled by the wind
    // sea's height spread, about 1.26m.
    let wind_spread = 1.26 * fft_view.gain.x;
    let crest = smoothstep(0.2 * wind_spread, 1.0 * wind_spread, field.wind_height * fft_view.gain.x);
    let chance = fold * crest * frame.params.y * dt;
    if frame.params.y <= 0.0 || unit_random(seed ^ 0x2545f491u) >= chance {
        particle.velocity.w = 0.0;
        particles[index] = particle;
        return;
    }
    // Born on the crest itself, at the drawn crest (the label point moved by
    // -D), so the crest hides the sheet's lower part and it starts from a
    // hard line at the wave's edge.
    let height = spray_surface_height(field);
    let drawn = local - chop * field.displacement * fft_view.gain.x;
    let r1 = unit_random(seed ^ 0x9e3779b9u);
    let r2 = unit_random(seed ^ 0x85ebca6bu);
    let r3 = unit_random(seed ^ 0xc2b2ae35u);
    // Lifted off the crest by the air flowing over it (rising a metre or
    // two before the drag stops it), and already moving most of the way to
    // the wind.
    let lift = 2.0 + 6.0 * fold * r1;
    let sideways = (vec2<f32>(r2, r3) - 0.5) * 0.8;
    let horizontal = frame.wind.xy * frame.wind.z * (0.5 + 0.3 * r2) + sideways;
    particle.position = vec4<f32>(drawn, height, 0.0);
    particle.extra = vec4<f32>(1.0e3, 0.0, 0.0, 0.0);
    particle.velocity = vec4<f32>(
        horizontal,
        lift,
        CREST_LIFETIME_SECONDS + CREST_LIFETIME_SPREAD_SECONDS * r3,
    );
    particles[index] = particle;
}

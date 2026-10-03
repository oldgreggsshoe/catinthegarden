// Wind-blown spray: camera-facing soft puffs, lit like the ocean's foam.
// Composed after shared_planet.wgsl (camera at group 0, the shared atmosphere
// LUTs at group 2); the particles and frame are group 1.

struct SprayParticle {
    position: vec4<f32>,
    velocity: vec4<f32>,
    // x: height above the water beneath it (m).
    extra: vec4<f32>,
}

struct SprayDrawFrame {
    axis_u: vec4<f32>,
    axis_v: vec4<f32>,
    shift_dt: vec4<f32>,
    wind: vec4<f32>,
    params: vec4<f32>,
    ship_origin: vec4<f32>,
    ship_axes: vec4<f32>,
    ship_velocity: vec4<f32>,
    ship_port: vec4<f32>,
    ship_starboard: vec4<f32>,
    ship_port_impact: vec4<f32>,
    ship_starboard_impact: vec4<f32>,
    ship_port_water: vec4<f32>,
    ship_starboard_water: vec4<f32>,
}

@group(1) @binding(0) var<storage, read> spray_particles: array<SprayParticle>;
@group(1) @binding(1) var<uniform> spray_frame: SprayDrawFrame;

struct SprayVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
    @location(2) alpha: f32,
    @location(3) seed: f32,
    // 1 for the ship's sheets of water, 0 for crest mist.
    @location(4) kind: f32,
    @location(5) age: f32,
}

// A small deterministic value-noise field gives every crest puff a distinct
// torn edge without a texture lookup. Keep the ship's splash clumps separate.
fn spray_hash(cell: vec2<i32>, seed: u32) -> f32 {
    var hash = bitcast<u32>(cell.x) * 0x8da6b343u
        ^ bitcast<u32>(cell.y) * 0xd8163841u
        ^ seed * 0xcb1ab31fu;
    hash ^= hash >> 16u;
    hash *= 0x7feb352du;
    hash ^= hash >> 15u;
    hash *= 0x846ca68bu;
    hash ^= hash >> 16u;
    return f32(hash & 0x00ffffffu) * (1.0 / 16777216.0);
}

fn spray_noise(point: vec2<f32>, seed: u32) -> f32 {
    let cell = vec2<i32>(floor(point));
    let local = fract(point);
    let eased = local * local * (3.0 - 2.0 * local);
    let a = spray_hash(cell, seed);
    let b = spray_hash(cell + vec2<i32>(1, 0), seed);
    let c = spray_hash(cell + vec2<i32>(0, 1), seed);
    let d = spray_hash(cell + vec2<i32>(1, 1), seed);
    return mix(mix(a, b, eased.x), mix(c, d, eased.x), eased.y);
}

fn crest_mist_mask(corner: vec2<f32>, age: f32, seed: f32) -> f32 {
    let noise_seed = u32(seed * 16777215.0);
    // The quad's x axis follows particle motion: slower variation along it
    // leaves wind-combed filaments; the age drift keeps their shape breathing.
    let point = corner * vec2<f32>(2.8, 6.5) + vec2<f32>(age * 1.1, age * 0.25);
    let warp_sample = spray_noise(point * 0.55 + vec2<f32>(3.1, 7.7), noise_seed) - 0.5;
    let warp = vec2<f32>(warp_sample, -0.65 * warp_sample);
    let warped = point + warp * 1.6;
    let billow = spray_noise(warped * 0.8, noise_seed);
    let breakup = spray_noise(warped * vec2<f32>(1.7, 3.2) + vec2<f32>(4.7, 2.3), noise_seed ^ 0x85ebca6bu);
    // Suppress sub-pixel breakup as these small puffs recede; otherwise the
    // noise aliases into the same hard speckles the mask is meant to remove.
    let footprint = max(fwidth(warped.x), fwidth(warped.y));
    let detail = 1.0 - smoothstep(0.12, 0.5, footprint * 3.8);
    let density = billow * 0.64 + mix(0.5, breakup, detail) * 0.36;
    let ragged_edge = length(corner + warp * 0.16);
    let envelope = 1.0 - smoothstep(0.5, 1.0, ragged_edge);
    let threshold_width = max(fwidth(density), 0.025);
    let wisps = smoothstep(0.25 - threshold_width, 0.62 + threshold_width, density);
    return envelope * mix(0.12, 0.78, wisps);
}

// Hull spray is strongest where it leaves the hull and fades as it travels:
// gone one half-beam out from the waterline outline (the plan shape of ship.rs
// half_beam_meters). Blown along the hull by the wind it otherwise drew
// splashes off the bow and stern, where there is no hull to throw them.
fn ship_spray_near_hull(position: vec2<f32>) -> f32 {
    let forward = spray_frame.ship_axes.xy;
    let port = vec2<f32>(-forward.y, forward.x);
    let relative = position - spray_frame.ship_origin.xy;
    let half_length = spray_frame.ship_axes.z;
    let t = dot(relative, forward) / half_length;
    let tc = clamp(t, -1.0, 1.0);
    let shape = select(1.0 - 0.2 * tc * tc, pow(max(1.0 - tc * tc, 0.0), 0.6), tc >= 0.0);
    let outside = length(vec2<f32>(
        max(abs(t) - 1.0, 0.0) * half_length,
        max(abs(dot(relative, port)) - spray_frame.ship_axes.w * shape, 0.0),
    ));
    return 1.0 - smoothstep(0.0, spray_frame.ship_axes.w, outside);
}

@vertex
fn vs_spray(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> SprayVertexOutput {
    let particle = spray_particles[instance_index];
    var out: SprayVertexOutput;
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex_index];
    out.corner = corner;
    let lifetime = particle.velocity.w;
    if lifetime <= 0.0 || particle.position.w >= lifetime {
        out.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        out.alpha = 0.0;
        return out;
    }
    let up = normalize(view_to_planet(camera.camera_planet_direction_view_altitude.xyz));
    let offset = spray_frame.axis_u.xyz * particle.position.x
        + spray_frame.axis_v.xyz * particle.position.y
        + up * (particle.position.z - camera.camera_planet_direction_view_altitude.w);
    var view_position = planet_to_view(offset);
    let centre_view_position = view_position;
    let age = particle.position.w / lifetime;
    let seed = fract(f32(instance_index) * 0.61803398875);
    let distance = length(view_position);
    // Fine droplets that spread a little as they disperse, capped on screen so
    // one passing the eye cannot become a cloud. The ship's slots (first
    // SHIP_SPRAY_SLOTS) throw sheets of spray metres across off the bow.
    let from_ship = instance_index < 2048u;
    // Ship sheets were sized on the 84m hull (half-length 42m); a smaller
    // hull throws proportionally smaller ones.
    let ship_size = spray_frame.ship_axes.z / 42.0;
    // Crest mist is sheet-sized from birth, so a crest's worth of it reads as
    // one torn curtain rather than separate specks.
    let grown = select(mix(0.3, 1.1, sqrt(age)), mix(0.8, 3.5, sqrt(age)) * ship_size, from_ship);
    let size = min(grown * (0.7 + 0.6 * seed), distance * select(0.03, 0.06, from_ship));
    // Stretched along the droplets' motion: spray streaks downwind.
    let velocity_view = planet_to_view(spray_frame.axis_u.xyz * particle.velocity.x
        + spray_frame.axis_v.xyz * particle.velocity.y + up * particle.velocity.z);
    let screen_velocity = velocity_view.xy;
    let along = select(
        vec2<f32>(1.0, 0.0),
        normalize(screen_velocity),
        dot(screen_velocity, screen_velocity) > 1.0e-6,
    );
    let across = vec2<f32>(-along.y, along.x);
    // A streak is velocity times a time, and time scales with the hull's root.
    let streak_seconds = 0.12 * select(1.0, sqrt(ship_size), from_ship);
    let streak = min(length(screen_velocity) * streak_seconds, distance * 0.06);
    view_position = vec3<f32>(
        view_position.xy + along * corner.x * (size + streak) + across * corner.y * size,
        view_position.z,
    );
    out.clip_position = camera.projection_matrix * vec4<f32>(view_position, 1.0);
    // Densest the instant it leaves the water and gone quickly after: the
    // source reads as a hard edge (the crest or the hull), the mist as a fast
    // fade downwind. No fade-in, so there is no soft start.
    // Bow sheets are denser water and hang longer than wind-torn crest mist.
    let fade = select(0.55 * exp(-3.4 * age), 0.9 * exp(-2.5 * age), from_ship);
    // Falling back to the sea, it thins over its last metre above the
    // surface instead of sitting on the water as a blob (it is retired at
    // the surface). Rising spray keeps its hard edge at the source.
    let settle_meters = select(1.0, ship_size, from_ship);
    let settling = select(1.0, smoothstep(0.0, settle_meters, particle.extra.x), particle.velocity.z < 0.0);
    out.alpha = fade * (1.0 - age) * smoothstep(3.0, 10.0, distance) * settling
        * select(1.0, ship_spray_near_hull(particle.position.xy), from_ship);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let height = max(particle.position.z, 0.0);
    let sun_transmittance = surface_direct_sun_transmittance(up, height, sun_direction);
    let sky_diffuse = sky_diffuse_irradiance(up, up, height, sun_direction);
    // Droplets scatter strongly forward: backlit spray glows.
    let view_direction = normalize(view_position);
    let forward = pow(max(dot(view_direction, normalize(camera.sun_direction_view.xyz)), 0.0), 6.0);
    // Through the same distance fog as the sea behind it, so storm fog swallows
    // spray and water together.
    let fog = terrain_fog(centre_view_position, up, height);
    out.colour = mix(
        ocean_foam_radiance(sun_transmittance, sky_diffuse)
            + sun_transmittance * (0.6 * forward * SURFACE_SUNLIGHT_SCALE),
        fog.color,
        fog.amount,
    );
    out.seed = seed;
    out.kind = select(0.0, 1.0, from_ship);
    out.age = age;
    return out;
}

@fragment
fn fs_spray(input: SprayVertexOutput) -> @location(0) vec4<f32> {
    let radius = length(input.corner);
    // Keep the ship's existing clumps and only pay for noise on crest puffs.
    var texture = 1.0;
    if input.kind > 0.5 {
        let s = input.seed * 40.0;
        texture = 0.55 + 0.45 * sin(input.corner.x * 5.3 + s) * sin(input.corner.y * 4.7 + s * 1.7);
    } else {
        texture = crest_mist_mask(input.corner, input.age, input.seed);
    }
    let soft_edge = 1.0 - smoothstep(0.15, 1.0, radius);
    let alpha = input.alpha * soft_edge * clamp(texture, 0.0, 1.0);
    if alpha <= 0.002 {
        discard;
    }
    return vec4<f32>(input.colour * alpha, alpha);
}

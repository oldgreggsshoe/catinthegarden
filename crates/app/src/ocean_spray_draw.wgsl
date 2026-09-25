// Wind-blown spray: camera-facing soft puffs, lit like the ocean's foam.
// Composed after shared_planet.wgsl (camera at group 0, the shared atmosphere
// LUTs at group 2); the particles and frame are group 1.

struct SprayParticle {
    position: vec4<f32>,
    velocity: vec4<f32>,
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
}

@group(1) @binding(0) var<storage, read> spray_particles: array<SprayParticle>;
@group(1) @binding(1) var<uniform> spray_frame: SprayDrawFrame;

struct SprayVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
    @location(2) alpha: f32,
    @location(3) seed: f32,
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
    let age = particle.position.w / lifetime;
    let seed = fract(f32(instance_index) * 0.61803398875);
    let distance = length(view_position);
    // Fine droplets that spread a little as they disperse, capped on screen so
    // one passing the eye cannot become a cloud. The ship's slots (first
    // SHIP_SPRAY_SLOTS) throw sheets of spray metres across off the bow.
    let from_ship = instance_index < 2048u;
    let grown = select(mix(0.12, 0.5, sqrt(age)), mix(0.8, 3.5, sqrt(age)), from_ship);
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
    let streak = min(length(screen_velocity) * 0.12, distance * 0.06);
    view_position = vec3<f32>(
        view_position.xy + along * corner.x * (size + streak) + across * corner.y * size,
        view_position.z,
    );
    out.clip_position = camera.projection_matrix * vec4<f32>(view_position, 1.0);
    // Densest the instant it leaves the water and gone quickly after: the
    // source reads as a hard edge (the crest or the hull), the mist as a fast
    // fade downwind. No fade-in, so there is no soft start.
    // Bow sheets are denser water and hang longer than wind-torn crest mist.
    let fade = select(0.8 * exp(-4.0 * age), 0.9 * exp(-2.5 * age), from_ship);
    out.alpha = fade * (1.0 - age) * smoothstep(3.0, 10.0, distance);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let height = max(particle.position.z, 0.0);
    let sun_transmittance = surface_direct_sun_transmittance(up, height, sun_direction);
    let sky_diffuse = sky_diffuse_irradiance(up, up, height, sun_direction);
    // Droplets scatter strongly forward: backlit spray glows.
    let view_direction = normalize(view_position);
    let forward = pow(max(dot(view_direction, normalize(camera.sun_direction_view.xyz)), 0.0), 6.0);
    out.colour = ocean_foam_radiance(sun_transmittance, sky_diffuse)
        + sun_transmittance * (0.6 * forward * SURFACE_SUNLIGHT_SCALE);
    out.seed = seed;
    return out;
}

@fragment
fn fs_spray(input: SprayVertexOutput) -> @location(0) vec4<f32> {
    let radius = length(input.corner);
    // A soft round puff broken into a few droplet clumps.
    let s = input.seed * 40.0;
    let clumps = 0.55 + 0.45 * sin(input.corner.x * 5.3 + s) * sin(input.corner.y * 4.7 + s * 1.7);
    let alpha = input.alpha * smoothstep(1.0, 0.15, radius) * clamp(clumps, 0.0, 1.0);
    if alpha <= 0.002 {
        discard;
    }
    return vec4<f32>(input.colour * alpha, alpha);
}

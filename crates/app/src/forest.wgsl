
struct Camera {
    projection_matrix: mat4x4<f32>,
    camera_forward: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_planet_direction_view_altitude: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_direction_view: vec4<f32>,
    projection: vec4<f32>,
    flat_triangle_options: vec4<f32>,
    lightning: vec4<f32>,
    moonlight: vec4<f32>,
    moon_direction_view: vec4<f32>,
    moonlight_transmittance: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: Camera;

struct ForestUniform {
    camera_planet_position: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> forest: ForestUniform;

@group(1) @binding(1)
var sky_view_lut: texture_2d<f32>;

@group(1) @binding(2)
var sky_view_sampler: sampler;

@group(2) @binding(0)
var cloud_field_current: texture_cube<f32>;

@group(2) @binding(1)
var cloud_field_previous: texture_cube<f32>;

@group(2) @binding(2)
var cloud_field_sampler: sampler;

struct WeatherRenderUniform {
    blend: f32,
    drift_radians: f32,
    lower_shell_radius_meters: f32,
    upper_shell_radius_meters: f32,
    noise_scale: f32,
    noise_strength: f32,
    _padding: vec2<f32>,
}

@group(2) @binding(3)
var<uniform> weather: WeatherRenderUniform;

@group(1) @binding(3) var atmosphere_transmittance_lut: texture_2d<f32>;
@group(1) @binding(4) var atmosphere_surface_irradiance_lut: texture_2d<f32>;
@group(1) @binding(5) var atmosphere_physical_sampler: sampler;

struct VertexInput {
    @location(0) centre_and_height: vec4<f32>,
    @location(1) width_shade_kind_seed: vec4<f32>,
}

struct TreeMeshVertex {
    position: vec3<f32>,
    normal: vec3<f32>,
    foliage: f32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) @interpolate(flat) up: vec3<f32>,
    @location(2) @interpolate(flat) species_kind: f32,
    @location(3) foliage: f32,
    @location(4) @interpolate(flat) shade: f32,
    @location(5) @interpolate(flat) seed: f32,
    // x: the sun's cloud visibility; y: this tree's share of the moon's
    // light (`surface_moonlight_share` times its cloud visibility). One slot.
    @location(6) @interpolate(flat, first) cloud_visibility: vec2<f32>,
    @location(7) @interpolate(flat, first) fog: vec4<f32>,
    @location(8) local_position: vec3<f32>,
    @location(9) @interpolate(flat) sunlight: vec3<f32>,
    @location(10) @interpolate(flat) skylight: vec3<f32>,
}

fn planet_to_view(vector: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vector, camera.camera_right.xyz),
        dot(vector, camera.camera_up.xyz),
        -dot(vector, camera.camera_forward.xyz),
    );
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

// Match the terrain fog's horizon sky lookup and 100m full-storm air path.
fn tree_storm_fog(centre: vec3<f32>, view_position: vec3<f32>) -> vec4<f32> {
    let overcast = clamp(camera.sun_direction.w, 0.0, 1.0);
    if overcast <= 0.0 { return vec4<f32>(0.0); }
    let distance = length(view_position);
    if distance <= 1.0e-3 { return vec4<f32>(0.0); }
    let camera_altitude = max(camera.camera_planet_direction_view_altitude.w, 0.0);
    let tree_altitude = max(length(centre) - PLANET_RADIUS_METERS, 0.0);
    let mean_density = mix(0.5 * (exp(-camera_altitude / 122000.0)
        + exp(-tree_altitude / 122000.0)), 1.0, overcast);
    let e_fold = exp(mix(log(500000.0), log(100.0 / 4.6051702), overcast));
    let amount = 1.0 - exp(-distance * mean_density / e_fold);
    if amount <= 1.0e-4 { return vec4<f32>(0.0); }

    let up = normalize(camera.camera_planet_direction_view_altitude.xyz);
    let sun = normalize(camera.sun_direction_view.xyz);
    var toward_sun = sun - up * dot(up, sun);
    if dot(toward_sun, toward_sun) < 1.0e-6 {
        toward_sun = normalize(camera.camera_right.xyz);
    } else {
        toward_sun = normalize(toward_sun);
    }
    let ray = normalize(view_position);
    let horizontal = ray - up * dot(ray, up);
    var azimuth = 0.0;
    if dot(horizontal, horizontal) > 1.0e-8 {
        let direction = normalize(horizontal);
        azimuth = atan2(dot(direction, cross(up, toward_sun)), dot(direction, toward_sun));
    }
    let radius = PLANET_RADIUS_METERS + max(camera_altitude, 200.0);
    let horizon_cosine = -sqrt(max(1.0 - pow(PLANET_RADIUS_METERS / radius, 2.0), 0.0));
    let uv = vec2<f32>(fract(azimuth / (2.0 * 3.141592653589793) + 0.5),
        0.5 * (1.0 - horizon_cosine));
    let radiance = textureSampleLevel(sky_view_lut, sky_view_sampler, uv, 0.0).rgb;
    let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    let perceived = 0.22 * pow(luminance, 0.42);
    let gain = clamp(perceived / max(luminance, 1.0e-8), 0.35, 80.0);
    let sky = radiance * gain;
    let grey = dot(sky, vec3<f32>(0.2126, 0.7152, 0.0722)) * 0.45;
    var lightning_glow = vec3<f32>(0.0);
    if camera.lightning.w > 0.0 {
        let altitude = max(camera.camera_planet_direction_view_altitude.w, 0.0);
        let outer_cosine = altitude / sqrt(altitude * altitude + 50000.0 * 50000.0);
        let inner_cosine = altitude / sqrt(altitude * altitude + 15000.0 * 15000.0);
        lightning_glow = vec3<f32>(0.72, 0.78, 0.9) * camera.lightning.w
            * mix(0.15, 1.0, smoothstep(
                max(0.35, outer_cosine),
                max(0.9, inner_cosine),
                dot(ray, normalize(camera.lightning.xyz)),
            ));
    }
    return vec4<f32>(mix(sky, vec3<f32>(grey), overcast) + lightning_glow, amount);
}

fn cloud_shadow_density_at_shell(
    surface_position: vec3<f32>,
    sun_direction: vec3<f32>,
    shell_radius: f32,
    shell_index: f32,
) -> f32 {
    let surface_radius_squared = dot(surface_position, surface_position);
    if surface_radius_squared >= shell_radius * shell_radius { return 0.0; }
    let ray_offset = dot(surface_position, sun_direction);
    let discriminant = ray_offset * ray_offset
        - (surface_radius_squared - shell_radius * shell_radius);
    if discriminant <= 0.0 { return 0.0; }
    let distance = -ray_offset + sqrt(discriminant);
    if distance <= 0.0 { return 0.0; }
    let shadow_position = surface_position + sun_direction * distance;
    return cloudDensityWithOctaves(normalize(shadow_position), shell_index, 3u);
}

fn cloud_shadow_visibility(
    surface_direction: vec3<f32>,
    surface_height: f32,
    sun_direction: vec3<f32>,
) -> f32 {
    let surface_position = normalize(surface_direction)
        * (PLANET_RADIUS_METERS + max(surface_height, 0.0));
    let lower_density = cloud_shadow_density_at_shell(
        surface_position, sun_direction, weather.lower_shell_radius_meters, 0.0,
    );
    let upper_density = cloud_shadow_density_at_shell(
        surface_position, sun_direction, weather.upper_shell_radius_meters, 1.0,
    );
    let combined_density = 1.0
        - (1.0 - clamp(lower_density, 0.0, 1.0))
            * (1.0 - clamp(upper_density, 0.0, 1.0));
    return 1.0 - floor(combined_density * 4.0 + 0.5) / 4.0 * 0.88;
}

// A compact, indexed 3D tree mesh shared by every instance. One continuous
// crown profile changes shape by species instead of stacking primitive masses.
fn tree_mesh_vertex(
    vertex_index: u32,
    height: f32,
    width: f32,
    species_kind: f32,
    seed: f32,
) -> TreeMeshVertex {
    if vertex_index < 12u {
        let side = vertex_index / 2u;
        let upper = vertex_index % 2u == 1u;
        let directions = array<vec2<f32>, 6>(
            vec2<f32>(1.0, 0.0), vec2<f32>(0.5, 0.8660254),
            vec2<f32>(-0.5, 0.8660254), vec2<f32>(-1.0, 0.0),
            vec2<f32>(-0.5, -0.8660254), vec2<f32>(0.5, -0.8660254),
        );
        let lower_radius = width * (0.040 + seed * 0.012);
        let upper_radius = width * 0.018;
        let top = select(height * 0.63, height * 0.98, species_kind >= 0.5);
        let side_a = directions[side];
        let side_b = directions[(side + 1u) % 6u];
        let direction = select(side_a, side_b, vertex_index % 2u == 0u);
        let radial = select(lower_radius, upper_radius, upper);
        let position = vec3<f32>(direction.x * radial, select(0.0, top, upper), direction.y * radial);
        return TreeMeshVertex(
            position,
            normalize(vec3<f32>(direction.x, 0.08, direction.y)),
            0.0,
        );
    }

    if vertex_index < 60u {
        let branch_vertex = vertex_index - 12u;
        let branch = branch_vertex / 12u;
        let ring_vertex = branch_vertex % 12u;
        let side = ring_vertex % 6u;
        let end_ring = ring_vertex >= 6u;
        var start_height = height * 0.40;
        var end = vec3<f32>(-width * 0.31, height * 0.62, width * 0.05);
        if branch == 1u {
            start_height = height * 0.44;
            end = vec3<f32>(width * 0.29, height * 0.64, -width * 0.08);
        } else if branch == 2u {
            start_height = height * 0.48;
            end = vec3<f32>(width * 0.06, height * 0.61, width * 0.34);
        } else if branch == 3u {
            start_height = height * 0.52;
            end = vec3<f32>(-width * 0.04, height * 0.70, -width * 0.32);
        }
        end.x += (seed - 0.5) * width * 0.035;
        let start = vec3<f32>(0.0, start_height, 0.0);
        // Only broadleaf crowns are wide enough to hide these boughs; on a
        // conifer they poked out of the tiers as dark spikes, so they collapse
        // into the trunk (zero-area triangles) there.
        if species_kind >= 0.5 {
            return TreeMeshVertex(start, vec3<f32>(0.0, 1.0, 0.0), 0.25);
        }
        end = vec3<f32>(end.x * 0.8, end.y, end.z * 0.8);
        let branch_axis = normalize(end - start);
        var reference = vec3<f32>(0.0, 0.0, 1.0);
        if abs(dot(branch_axis, reference)) > 0.92 {
            reference = vec3<f32>(0.0, 1.0, 0.0);
        }
        let tangent = normalize(cross(branch_axis, reference));
        let bitangent = cross(branch_axis, tangent);
        let angle = f32(side) * (6.2831853 / 6.0) + seed * 1.6180339 + f32(branch) * 0.7;
        let radial = tangent * cos(angle) + bitangent * sin(angle);
        let branch_radius = width * select(0.026, 0.009, end_ring);
        let centre = select(start, end, end_ring);
        return TreeMeshVertex(centre + radial * branch_radius, radial, 0.25);
    }

    let foliage_index = vertex_index - 60u;
    let cluster_index = foliage_index / 38u;
    let cluster_vertex_index = foliage_index % 38u;
    let phase = seed * 6.2831853;
    let broadleaf = species_kind < 0.5;
    var centre = vec3<f32>(0.0);
    var radius_x = 0.0;
    var radius_y = 0.0;
    var radius_z = 0.0;
    if broadleaf {
        // Written as branches rather than local arrays: a dynamically indexed
        // array is copied into per-thread memory on every vertex.
        var cluster_layout = vec3<f32>(0.0, 0.76, 0.0);
        var dimensions = vec3<f32>(0.24, 0.13, 0.24);
        if cluster_index == 9u {
            cluster_layout = vec3<f32>(0.0, 0.88, 0.0);
            dimensions = vec3<f32>(0.15, 0.09, 0.15);
        } else if cluster_index >= 5u {
            let diagonal = cluster_index - 5u;
            let sign_x = select(-1.0, 1.0, diagonal == 1u || diagonal == 2u);
            let sign_z = select(-1.0, 1.0, diagonal == 0u || diagonal == 2u);
            cluster_layout = vec3<f32>(
                0.13 * sign_x,
                select(0.58, 0.57, diagonal == 0u || diagonal == 2u),
                0.13 * sign_z,
            );
            dimensions = vec3<f32>(0.22, 0.10, 0.22);
        } else if cluster_index >= 1u {
            let axis = cluster_index - 1u;
            let sign_value = select(-1.0, 1.0, axis == 1u || axis == 2u);
            cluster_layout = vec3<f32>(
                select(0.0, 0.23 * sign_value, axis < 2u),
                0.64,
                select(0.0, 0.23 * sign_value, axis >= 2u),
            );
            dimensions = vec3<f32>(0.27, 0.11, 0.27);
        }
        let lean = vec2<f32>(seed - 0.5, fract(seed * 3.73) - 0.5) * width * 0.035;
        centre = vec3<f32>(
            cluster_layout.x * width + lean.x,
            cluster_layout.y * height,
            cluster_layout.z * width + lean.y,
        );
        radius_x = dimensions.x * width * mix(0.92, 1.08, fract(seed * 5.17));
        radius_y = dimensions.y * height;
        radius_z = dimensions.z * width * mix(0.92, 1.08, fract(seed * 2.37));
    } else {
        if cluster_index < 9u {
            let tier = f32(cluster_index / 3u);
            let spoke = f32(cluster_index % 3u);
            let tier_fraction = (tier + 0.5) / 3.0;
            let tier_radius = width * 0.56 * pow(1.0 - tier_fraction, 0.82);
            let angle = spoke * 2.0943951 + phase + tier * 0.45;
            let direction = vec2<f32>(cos(angle), sin(angle));
            centre = vec3<f32>(
                direction.x * tier_radius * 0.40,
                height * (0.12 + 0.25 * (tier + 0.5)),
                direction.y * tier_radius * 0.40,
            );
            radius_x = tier_radius * 0.66;
            radius_y = height * 0.11;
            radius_z = tier_radius * 0.66;
        } else {
            centre.y = height * 0.91;
            radius_x = width * 0.10;
            radius_y = height * 0.11;
            radius_z = width * 0.10;
        }
    }

    if cluster_vertex_index == 0u || cluster_vertex_index == 37u {
        let top = cluster_vertex_index == 0u;
        let pole = select(-1.0, 1.0, top);
        return TreeMeshVertex(
            centre + vec3<f32>(0.0, radius_y * pole, 0.0),
            vec3<f32>(0.0, pole, 0.0),
            1.0,
        );
    }

    let ring_index = (cluster_vertex_index - 1u) / 12u;
    let sector = (cluster_vertex_index - 1u) % 12u;
    let angle = f32(sector) * (6.2831853 / 12.0) + 2.399963 * seed;
    let t = (f32(ring_index) + 1.0) / 4.0;
    let radial_direction = vec2<f32>(cos(angle), sin(angle));
    let profile = sin(t * 3.14159265);
    let vertical = cos(t * 3.14159265);
    var radial_jitter = 1.0;
    if broadleaf {
        radial_jitter += 0.20 * cos(angle * 3.0 + phase + f32(cluster_index))
            + 0.12 * sin(angle * 5.0 - phase * 1.3)
            + 0.07 * cos(angle * 7.0 + t * 6.2831853 + phase)
            + 0.05 * sin(t * 12.566371 + angle * 2.0 + phase);
    } else {
        radial_jitter += 0.12 * cos(angle * 3.0 + phase + f32(cluster_index))
            + 0.06 * sin(angle * 5.0 - phase * 1.3)
            + 0.04 * cos(angle * 7.0 + t * 6.2831853 + phase)
            + 0.03 * sin(t * 12.566371 + angle * 2.0 + phase);
    }
    let cluster_wave = sin(angle * 3.0 + phase + f32(cluster_index) * 1.7);
    var position = centre + vec3<f32>(
        radial_direction.x * radius_x * profile * radial_jitter,
        radius_y * vertical,
        radial_direction.y * radius_z * profile * radial_jitter,
    );
    if broadleaf {
        position.y += height * 0.035 * cluster_wave * profile;
    } else {
        position.y += height * 0.025 * cluster_wave * profile
            * (1.0 - (f32(cluster_index) / 10.0));
    }
    let normal = normalize(vec3<f32>(
        radial_direction.x * profile / max(radius_x, 1.0e-3),
        vertical / max(radius_y, 1.0e-3),
        radial_direction.y * profile / max(radius_z, 1.0e-3),
    ));
    return TreeMeshVertex(position, normal, clamp(0.98 + (radial_jitter - 1.0) * 0.45, 0.90, 1.10));
}

@vertex
fn vs_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let centre = input.centre_and_height.xyz;
    let height = input.centre_and_height.w;
    let width = input.width_shade_kind_seed.x;
    // Cell culling is deliberately coarse. Reject a whole tree before its
    // procedural mesh and lighting when its generous bounding sphere misses
    // the view. Re-evaluate against this frame's camera, not cached generation.
    let centre_view = planet_to_view(centre - forest.camera_planet_position.xyz);
    let radius = height * 1.25 + width + 2.0;
    let tangent = vec2<f32>(camera.projection.x * camera.projection.y, camera.projection.y);
    let depth = -centre_view.z;
    let outside = depth < -radius
        || any(abs(centre_view.xy) > depth * tangent + radius * sqrt(vec2<f32>(1.0) + tangent * tangent));
    // Rejected GPU candidates have zero height. Do not shade a whole invisible
    // mesh (or normalize its zero centre) for every rejected instance.
    if height <= 0.0 || outside {
        return VertexOutput(
            vec4<f32>(2.0, 2.0, 0.0, 1.0),
            vec3<f32>(0.0), vec3<f32>(0.0), 0.0, 0.0, 0.0, 0.0,
            vec2<f32>(1.0, 0.0), vec4<f32>(0.0), vec3<f32>(0.0),
            vec3<f32>(0.0), vec3<f32>(0.0),
        );
    }
    let shade = input.width_shade_kind_seed.y;
    let kind = input.width_shade_kind_seed.z;
    let species_kind = kind - floor(kind * 0.5) * 2.0;
    let seed = input.width_shade_kind_seed.w;
    let up = normalize(centre);
    let reference = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(up.y) > 0.92);
    let base_right = normalize(cross(up, reference));
    let base_forward = normalize(cross(up, base_right));
    let rotation_angle = min(floor(seed * 8.0), 7.0) * 0.7853982;
    let rotation = vec2<f32>(cos(rotation_angle), sin(rotation_angle));
    let right = base_right * rotation.x + base_forward * rotation.y;
    let forward = base_forward * rotation.x - base_right * rotation.y;
    let mesh = tree_mesh_vertex(vertex_index, height, width, species_kind, seed);
    let world_position = centre
        + right * mesh.position.x
        + up * mesh.position.y
        + forward * mesh.position.z;
    let world_normal = normalize(
        right * mesh.normal.x + up * mesh.normal.y + forward * mesh.normal.z,
    );
    let view_position = planet_to_view(world_position - forest.camera_planet_position.xyz);
    let sun_direction = normalize(camera.sun_direction.xyz);
    let solar_elevation_cosine = dot(up, sun_direction);
    var cloud_visibility = 1.0;
    if TERRAIN_CLOUD_SHADOW_ENABLED && solar_elevation_cosine > 0.0 {
        cloud_visibility = cloud_shadow_visibility(
            up,
            length(centre) - PLANET_RADIUS_METERS,
            sun_direction,
        );
    }
    let fog = tree_storm_fog(
        centre,
        planet_to_view(centre - forest.camera_planet_position.xyz),
    );
    // Sample the same physical inputs as the ground at this tree's base.
    let altitude = length(centre) - PLANET_RADIUS_METERS;
    let sunlight = surface_direct_sun_transmittance(up, altitude, sun_direction);
    let skylight = sky_diffuse_irradiance(up, up, altitude, sun_direction);
    // The ground's rule (planet.wgsl terrain_moonlight): its own cloud shadow
    // when those are on, otherwise the camera's view of the moon.
    var moon_share = surface_moonlight_share(up, altitude, sun_direction);
    if moon_share > 0.0 {
        if TERRAIN_CLOUD_SHADOW_ENABLED {
            moon_share *= cloud_shadow_visibility(up, altitude, moon_direction());
        } else {
            moon_share *= moon_camera_cloud_visibility();
        }
    }
    return VertexOutput(
        camera.projection_matrix * vec4<f32>(view_position, 1.0),
        world_normal,
        up,
        species_kind,
        mesh.foliage,
        shade,
        seed,
        vec2<f32>(cloud_visibility, moon_share),
        fog,
        mesh.position,
        sunlight,
        skylight,
    );
}

fn tree_foliage_hash(cell: vec3<i32>, seed: f32) -> f32 {
    var value = u32(cell.x) * 0x9e3779b9u
        ^ u32(cell.y) * 0x85ebca6bu
        ^ u32(cell.z) * 0xc2b2ae35u
        ^ u32(seed * 65535.0);
    value = value ^ (value >> 16u);
    value = value * 0x7feb352du;
    value = value ^ (value >> 15u);
    value = value * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return f32(value & 0x00ffffffu) / 16777215.0;
}

fn tree_foliage_noise(position: vec3<f32>, seed: f32) -> f32 {
    let cell_floor = floor(position);
    let cell = vec3<i32>(cell_floor);
    let fraction = fract(position);
    let weight = fraction * fraction * (vec3<f32>(3.0) - 2.0 * fraction);
    let c000 = tree_foliage_hash(cell, seed);
    let c100 = tree_foliage_hash(cell + vec3<i32>(1, 0, 0), seed);
    let c010 = tree_foliage_hash(cell + vec3<i32>(0, 1, 0), seed);
    let c110 = tree_foliage_hash(cell + vec3<i32>(1, 1, 0), seed);
    let c001 = tree_foliage_hash(cell + vec3<i32>(0, 0, 1), seed);
    let c101 = tree_foliage_hash(cell + vec3<i32>(1, 0, 1), seed);
    let c011 = tree_foliage_hash(cell + vec3<i32>(0, 1, 1), seed);
    let c111 = tree_foliage_hash(cell + vec3<i32>(1, 1, 1), seed);
    let lower = mix(mix(c000, c100, weight.x), mix(c010, c110, weight.x), weight.y);
    let upper = mix(mix(c001, c101, weight.x), mix(c011, c111, weight.x), weight.y);
    return mix(lower, upper, weight.z);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sun_direction = normalize(camera.sun_direction.xyz);
    let normal = normalize(input.normal);
    let direct = max(dot(normal, sun_direction), 0.0) * SURFACE_SUNLIGHT_SCALE;
    // Leaf transmission is sunlight too, not a second sun below the planet.
    let back_scatter = max(dot(normal, -sun_direction), 0.0) * 0.16 * input.foliage;
    let sky_view = 0.5 + 0.5 * clamp(dot(normal, input.up), -1.0, 1.0);
    var moon = vec3<f32>(0.0);
    if input.cloud_visibility.y > 0.0 {
        let moon_direction = moon_direction();
        moon = moonlight_colour() * input.cloud_visibility.y
            * (max(dot(normal, moon_direction), 0.0)
                + max(dot(normal, -moon_direction), 0.0) * 0.16 * input.foliage);
    }
    let lighting = (input.sunlight * input.cloud_visibility.x * (direct + back_scatter)
        + input.skylight * sky_view + moon) * input.shade;
    let broadleaf = srgb_to_linear(vec3<f32>(0.26, 0.36, 0.20));
    let conifer = srgb_to_linear(vec3<f32>(0.21, 0.31, 0.21));
    let foliage = mix(broadleaf, conifer, input.species_kind)
        * mix(0.82, 1.12, input.seed);
    let bark = srgb_to_linear(vec3<f32>(0.32, 0.22, 0.12));
    let branch_bark = srgb_to_linear(vec3<f32>(0.25, 0.18, 0.11));
    var albedo = bark;
    var leaf_detail = 0.0;
    var leaf_noise = 0.5;
    if input.foliage > 0.5 {
        let noise_position = input.local_position * 4.5
            + vec3<f32>(input.seed * 13.7, input.seed * 7.1, input.seed * 19.3);
        let footprint = max(
            max(fwidth(noise_position.x), fwidth(noise_position.y)),
            fwidth(noise_position.z),
        );
        leaf_detail = 1.0 - smoothstep(0.12, 0.40, footprint);
        if leaf_detail > 0.01 {
            leaf_noise = tree_foliage_noise(noise_position, input.seed);
        }
        albedo = foliage
            * clamp(input.foliage, 0.90, 1.10)
            * mix(1.0, mix(0.42, 1.42, leaf_noise), leaf_detail * 0.72);
    } else if input.foliage > 0.1 {
        albedo = branch_bark;
    }
    let colour = albedo * lighting;
    return vec4<f32>(mix(colour, input.fog.rgb, input.fog.w), 1.0);
}

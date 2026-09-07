// Bakes the moon's albedo markings into a cubemap, once, at startup.
//
// These are a function of surface direction alone, so recomputing them per
// fragment was pure waste: walking 96 markings cost 12.1ms of every 28.7ms
// moon-surface frame, 42% of it, measured by removing the loop and comparing
// against the same replay. Baking is what the outmap already does for height,
// biome and moisture; this is the same idea without a fourth channel, because
// the finest marking feature is about 6km across on a 1,080km body and a
// cubemap resolves that at a fraction of the outmap's size.
//
// Rendered rather than precomputed on the CPU so the catalogue in the shader
// stays the single source of truth: the constants come from the same
// `moon::wgsl_constants()` the surface shader is built with.

/// How much brighter fresh ejecta is than the ground it fell on.
const MOON_HALO_STRENGTH: f32 = 0.55;
/// And the rays, which are brighter still but cover far less.
const MOON_RAY_STRENGTH: f32 = 0.75;
/// Freshness below which a crater has no rays at all. Rays are the first thing
/// space weathering erases -- they are thin and bright and made of the finest
/// material -- so only genuinely young craters keep them.
const MOON_RAY_FRESHNESS_FLOOR: f32 = 0.45;

struct MoonMarkingFace {
    index: u32,
    size: f32,
    pad0: u32,
    pad1: u32,
}
@group(0) @binding(0) var<uniform> moon_marking_face: MoonMarkingFace;

/// The cube face convention `textureSample` uses, so the baked map lines up
/// with the direction the surface shader looks it up with. Getting this wrong
/// mirrors or rotates every marking off its crater, which is why the fix is
/// verified by differing the baked render against the analytic one.
fn moon_marking_face_direction(face: u32, u: f32, v: f32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(1.0, -v, -u); }
        case 1u: { return vec3<f32>(-1.0, -v, u); }
        case 2u: { return vec3<f32>(u, 1.0, v); }
        case 3u: { return vec3<f32>(u, -1.0, -v); }
        case 4u: { return vec3<f32>(u, -v, 1.0); }
        default: { return vec3<f32>(-u, -v, -1.0); }
    }
}

/// What the surface shader used to work out per fragment: how much brighter
/// this direction is than bare regolith, saturating so overlapping haloes
/// brighten toward a limit instead of running away where young craters cluster.
fn moon_marking_brightening(unit: vec3<f32>) -> f32 {
    var brightening = 0.0;
    for (var index = 0u; index < MOON_MARKING_COUNT; index = index + 1u) {
        let marking = MOON_MARKINGS[index];
        let traits = MOON_MARKING_TRAITS[index];
        // Reach in rim radii, already clamped by the cost bound: the fades run
        // to this, so they land on zero at the same place the cutoff rejects.
        let extents = MOON_MARKING_EXTENTS[index];
        let cosine = clamp(dot(marking.xyz, unit), -1.0, 1.0);
        // Rays reach furthest, so their cutoff rejects everything.
        if cosine <= traits.w {
            continue;
        }
        // Past this point the marking genuinely reaches, so the transcendentals
        // are paid for rather than spent on a rejected crater.
        let freshness = traits.x;
        let t = acos(cosine) / marking.w;

        // Azimuth around the crater, from a stable basis: the component of the
        // sample direction perpendicular to the crater's axis.
        let tangent = normalize(unit - marking.xyz * cosine);
        let reference = normalize(cross(marking.xyz, vec3<f32>(0.0, 1.0, 0.0))
            + vec3<f32>(1.0e-5, 0.0, 0.0));
        let across = cross(marking.xyz, reference);
        let azimuth = atan2(dot(tangent, across), dot(tangent, reference)) + traits.y;

        // The halo: strongest at the rim, gone by the marking's reach. Squared so
        // it concentrates near the crater rather than washing the whole area.
        // Its edge is pushed in and out with azimuth, because an ejecta blanket
        // is not a circle -- the impact came in at an angle and the ground it
        // landed on was not flat.
        if cosine > traits.z {
            // Normalised to peak at 1 rather than 1.53, so the raggedest
            // azimuth's fade reaches zero exactly where the cutoff rejects
            // instead of a third of the way past it. The edge still moves in
            // and out with azimuth; it just no longer overshoots its own
            // cutoff and leaves a step there.
            let ragged = (1.0 + 0.35 * cos(3.0 * azimuth + traits.y)
                + 0.18 * cos(5.0 * azimuth - traits.y)) / (1.0 + 0.35 + 0.18);
            let fade = clamp(
                1.0 - (t - 1.0) / max((extents.x - 1.0) * ragged, 0.05),
                0.0,
                1.0,
            );
            brightening = brightening + MOON_HALO_STRENGTH * freshness * fade * fade;
        }

        if freshness <= MOON_RAY_FRESHNESS_FLOOR || t <= 1.0 {
            continue;
        }
        // Four incommensurate harmonics rather than two. With two the rays came
        // out as evenly spaced spokes of constant width -- a wheel, not a
        // splash. Real ray systems are uneven in spacing, unequal in strength,
        // and often heavily one-sided, which is what an oblique impact does.
        // Normalised by the sum of the amplitudes, so the peak is 1 whatever
        // the harmonics are. Raising an unnormalised sum to a power crushed the
        // rays to nothing, because four terms rarely align.
        let lobes = (0.46 * cos(6.0 * azimuth)
            + 0.28 * cos(11.0 * azimuth + traits.y)
            + 0.17 * cos(17.0 * azimuth - 2.0 * traits.y)
            + 0.22 * cos(2.0 * azimuth + 0.7 * traits.y))
            / 1.13;
        // A threshold rather than a power: it sets where a ray starts and how
        // hard its edge is, instead of dimming everything including the peaks.
        let streak = smoothstep(0.30, 0.78, lobes);
        // Patchy along their length as well as around: a ray is a chain of
        // bright clumps, not a painted line, because it is ballistic ejecta
        // landing in secondary craters rather than a continuous stream.
        let clumping = 0.62 + 0.38 * cos(4.0 * t + traits.y * 3.0)
            * cos(2.3 * t - traits.y);
        // Cubed, so a ray thins out with distance instead of stopping at a
        // circle.
        let reach = clamp(1.0 - (t - 1.0) / max(extents.y - 1.0, 0.05), 0.0, 1.0);
        let age = smoothstep(MOON_RAY_FRESHNESS_FLOOR, 1.0, freshness);
        brightening = brightening
            + MOON_RAY_STRENGTH * age * streak * max(clumping, 0.0) * reach * reach;
    }
    return 1.0 - exp(-brightening);
}

@vertex
fn vs_moon_markings(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    // One oversized triangle, so every texel of the face is covered without an
    // index buffer.
    let x = f32(i32(vertex) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex) & 1) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_moon_markings(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let u = 2.0 * (position.x / moon_marking_face.size) - 1.0;
    let v = 2.0 * (position.y / moon_marking_face.size) - 1.0;
    let direction = normalize(moon_marking_face_direction(moon_marking_face.index, u, v));
    return vec4<f32>(moon_marking_brightening(direction), 0.0, 0.0, 1.0);
}

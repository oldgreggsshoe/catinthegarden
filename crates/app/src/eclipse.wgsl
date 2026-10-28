// Solar eclipses: the moon's shadow on the planet (sky_moon.rs, which these
// mirror and generates them into every body's WGSL). Pure functions, so any
// shader can call them with its own camera uniform's moon.

const ECLIPSE_PI: f32 = 3.14159265;

// Share of a disc of angular radius `source` hidden by one of radius
// `occluder` centred `separation` from it (radians): the lens area of the
// two circles over the source's. sky_moon.rs `disc_overlap_share`.
fn eclipse_disc_overlap_share(separation: f32, source: f32, occluder: f32) -> f32 {
    if separation >= source + occluder {
        return 0.0;
    }
    if separation <= abs(occluder - source) {
        let inner = min(source, occluder);
        return inner * inner / (source * source);
    }
    let d2 = separation * separation;
    let s2 = source * source;
    let o2 = occluder * occluder;
    let source_arc = s2 * acos(clamp((d2 + s2 - o2) / (2.0 * separation * source), -1.0, 1.0));
    let occluder_arc = o2 * acos(clamp((d2 + o2 - s2) / (2.0 * separation * occluder), -1.0, 1.0));
    let kite = 0.5 * sqrt(max(
        (-separation + source + occluder)
            * (separation + source - occluder)
            * (separation - source + occluder)
            * (separation + source + occluder),
        0.0,
    ));
    return clamp((source_arc + occluder_arc - kite) / (ECLIPSE_PI * s2), 0.0, 1.0);
}

// Share of a light of angular radius `source` round the unit `sun` direction
// that the moon (`moon.xyz` its centre, `moon.w` its radius; 0 for no moon)
// leaves at `point`. `point` and `moon.xyz` are metres from the planet's
// centre in one frame, the sun's. sky_moon.rs `moon_shadow_sunlight`.
//
// Run per pixel and per sky sample while an eclipse is near, so it keeps
// to cheap arithmetic: every angle here is under a tenth of a radian, where
// the tangent and sine stand in for the angle itself (off by under 0.1% at
// the shadow's edge), and only the penumbra pays for the lens area.
fn moon_shadow_sunlight(point: vec3<f32>, sun: vec3<f32>, moon: vec4<f32>, source: f32) -> f32 {
    if moon.w <= 0.0 {
        return 1.0;
    }
    let to_moon = moon.xyz - point;
    let distance = length(to_moon);
    let along = dot(to_moon, sun);
    // The moon's angular radius, and how far the sun is from its centre.
    let occluder = moon.w / distance;
    let reach = source + occluder;
    let across = cross(to_moon, sun);
    let across_squared = dot(across, across);
    if along <= 0.0 || across_squared >= reach * reach * along * along {
        return 1.0;
    }
    let separation = sqrt(across_squared) / along;
    if separation <= occluder - source {
        return 0.0;
    }
    return 1.0 - eclipse_disc_overlap_share(separation, source, occluder);
}

// The direct sunlight the moon leaves at `point`.
fn moon_shadow_direct_sun(point: vec3<f32>, sun: vec3<f32>, moon: vec4<f32>) -> f32 {
    return moon_shadow_sunlight(point, sun, moon, ECLIPSE_SUN_ANGULAR_RADIUS);
}

// The sun's light on a surface at `point`: the share of the disc the moon
// leaves, and in the umbra the light of the sunlit air round the horizon
// (sky_moon.rs SOLAR_ECLIPSE_TOTALITY_LIGHT).
fn moon_shadow_surface_sun(point: vec3<f32>, sun: vec3<f32>, moon: vec4<f32>) -> f32 {
    return mix(SOLAR_ECLIPSE_TOTALITY_LIGHT, 1.0, moon_shadow_direct_sun(point, sun, moon));
}

// The light the air scatters at `point` or onto it from the sky above: shaded
// by a sun widened to the sunlit air around it, and never quite gone, since
// light scatters in from beyond the shadow.
fn moon_shadow_scattered_light(point: vec3<f32>, sun: vec3<f32>, moon: vec4<f32>) -> f32 {
    return mix(
        SOLAR_ECLIPSE_TOTALITY_SKY,
        1.0,
        moon_shadow_sunlight(point, sun, moon, SOLAR_ECLIPSE_SKYLIGHT_SOURCE_RADIANS),
    );
}

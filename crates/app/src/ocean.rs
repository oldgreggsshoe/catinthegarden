use glam::DVec3;

use crate::planet::PLANET_RADIUS_METERS;

// Keep these values byte-for-byte aligned with the raster/ray WGSL ocean
// surface. Collision and buoyancy must sample the same displaced water that
// the player sees; a stale CPU scale leaves a camera apparently above water
// while the rendered crest has already passed over it.
/// The one ocean-size knob. Everything below is derived from it.
///
/// Six constants used to have to move together by hand -- two amplitude scales
/// on the CPU, the same two in WGSL, the height cap, and the steepness that
/// keeps the waves from folding through themselves -- and missing one of them
/// fails quietly rather than loudly. Set this instead.
pub const OCEAN_WAVE_SCALE: f64 = 1.0;

const BASE_CALM_GEOMETRY_AMPLITUDE_SCALE: f64 = 44.0;
const BASE_STORM_GEOMETRY_AMPLITUDE_SCALE: f64 = 55.0;
pub const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE: f64 =
    BASE_CALM_GEOMETRY_AMPLITUDE_SCALE * OCEAN_WAVE_SCALE;
pub const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE: f64 =
    BASE_STORM_GEOMETRY_AMPLITUDE_SCALE * OCEAN_WAVE_SCALE;

/// Gerstner waves fold through themselves once the sum of
/// `steepness * amplitude * wave_number` passes 1, and a folded surface renders
/// as knots and holes at grazing angles. Steepness is therefore not free: it is
/// whatever holds that sum where it already is. Amplitude scaling by `k` means
/// steepness scaling by `1/k`, so a taller sea is not automatically a steeper
/// one. `fold_budget` measures the sum, and a test holds it invariant.
pub const OCEAN_STEEPNESS_SCALE: f64 = 1.0 / OCEAN_WAVE_SCALE;

/// Tallest crest the sea can raise, straight from the table and the scale, so
/// it can never quietly disagree with them.
pub const MAXIMUM_WAVE_HEIGHT_METERS: f64 =
    storm_amplitude_sum() * OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE;

/// The ocean constants the shader needs, generated from the knob above.
///
/// Prepended to `shared_planet.wgsl` by `planet::shared_planet_shader_source`
/// so these exist in exactly one place. They used to be typed out in the WGSL
/// as well, which meant `OCEAN_WAVE_SCALE` could silently disagree with the sea
/// actually being drawn.
pub(crate) fn wgsl_constants() -> String {
    fn wgsl_number(value: f64) -> String {
        if value.fract() == 0.0 {
            format!("{value:.1}")
        } else {
            format!("{value}")
        }
    }
    format!(
        "// Generated from ocean.rs; OCEAN_WAVE_SCALE = {}. Do not edit here.\n\
         const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE: f32 = {};\n\
         const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE: f32 = {};\n\
         const OCEAN_STEEPNESS_SCALE: f32 = {};\n\
         const OCEAN_MAXIMUM_WAVE_HEIGHT_METERS: f32 = {};\n\
         const OCEAN_BREAKING_HEIGHT_TO_DEPTH_RATIO: f32 = {};\n\
         const OCEAN_WAVE_PHASE_SPEED_SIGN: f32 = {};\n",
        wgsl_number(OCEAN_WAVE_SCALE),
        wgsl_number(OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE),
        wgsl_number(OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE),
        wgsl_number(OCEAN_STEEPNESS_SCALE),
        wgsl_number(MAXIMUM_WAVE_HEIGHT_METERS),
        wgsl_number(BREAKING_HEIGHT_TO_DEPTH_RATIO),
        wgsl_number(OCEAN_WAVE_PHASE_SPEED_SIGN),
    )
}

/// Sum of every wave's full-storm amplitude, before the geometry scale.
const fn storm_amplitude_sum() -> f64 {
    let mut total = 0.0;
    let mut index = 0;
    while index < WAVES.len() {
        total += WAVES[index].storm_amplitude_meters;
        index += 1;
    }
    total
}

/// The self-intersection budget described on `OCEAN_STEEPNESS_SCALE`. At or
/// above 1.0 the surface folds through itself.
pub fn fold_budget() -> f64 {
    fold_budget_at(OCEAN_WAVE_SCALE)
}

/// `fold_budget` for a hypothetical knob setting, so a test can show the budget
/// really is invariant rather than merely correct at today's value.
pub fn fold_budget_at(wave_scale: f64) -> f64 {
    let scale = BASE_STORM_GEOMETRY_AMPLITUDE_SCALE * wave_scale;
    let steepness_scale = 1.0 / wave_scale;
    WAVES
        .iter()
        .map(|wave| {
            let wave_number = std::f64::consts::TAU / wave.wavelength_meters;
            wave.steepness * steepness_scale * wave.storm_amplitude_meters * scale * wave_number
        })
        .sum()
}
pub const GLOBAL_OCEAN_STORM_INTENSITY: f32 = 1.0;

/// Whether the rendered sea carries Gerstner horizontal transport.
///
/// Must stay `false` while the CPU height query is radial: `wave_height_meters`
/// asks how high the water is directly below a point, and horizontal transport
/// slides the rendered surface up to 54m sideways from there. On a steep face
/// that is metres of height error, and the camera swims under the water it is
/// supposed to be floating on.
///
/// This used to be derived from `WATER_BOBBING_ENABLED`, which welded a camera
/// setting to a renderer one: turning bobbing back on silently turned transport
/// on with it. Give the CPU query a horizontal term and this can go true.
pub const OCEAN_HORIZONTAL_TRANSPORT_ENABLED: bool = false;

/// Whether the renderer displaces geometry by the short ripple octave.
///
/// It does not: `ocean_surface` hands `ripple_height` out for colour and
/// `ripple_slope` for the normal, and `vs_ocean` adds neither to
/// `local_planet_position`. The ripples are a shading detail, not a surface.
///
/// Collision must follow the water that is actually drawn, so while this is
/// false the camera's surface query leaves them out. Including them adds up to
/// 4.6m of height the renderer never draws, which puts the eye under a crest it
/// cannot see -- the fixed-height diagnostic hid this by querying the global
/// swell, so it only appeared when buoyant bobbing was restored.
pub const OCEAN_RIPPLES_ARE_GEOMETRIC: bool = false;

/// Which way crests travel along each wave's axis.
///
/// Phase is `wave_number * (dot(direction, axis) * R + sign * speed * time)`.
/// Holding a crest's phase constant as time grows requires the along-axis
/// coordinate to move against `sign`, so `+1.0` sends crests along `-axis` and
/// `-1.0` sends them along `+axis`. Nothing in the model refracts, so which of
/// those runs onshore at any given coast is an accident of how that coast
/// faces; this is the switch for testing whether the table was authored for the
/// opposite convention.
pub const OCEAN_WAVE_PHASE_SPEED_SIGN: f64 = -1.0;
/// Diagnostic: collide and render against only the two 1,400 m swells at the
/// head of `WAVES`, dropping the 160/65/24/9 m global waves and the whole
/// ripple layer. Must match `OCEAN_LARGE_SWELL_ONLY` in `shared_planet.wgsl`,
/// which `large_swell_only_is_paired_with_the_shader` enforces.
pub const OCEAN_LARGE_SWELL_ONLY: bool = false;
/// Number of leading `WAVES` entries that make up the dominant swell pair.
const LARGE_SWELL_WAVE_COUNT: usize = 2;
#[derive(Clone, Copy)]
struct GerstnerWave {
    direction: DVec3,
    wavelength_meters: f64,
    /// Calm amplitude, and the amplitude at full storm. A storm moves the
    /// dominant band from the 1400 m swell down to the 280-430 m storm sea
    /// rather than simply scaling the calm sea up. Both columns sum to the
    /// same total so the height cap is unaffected.
    amplitude_meters: f64,
    storm_amplitude_meters: f64,
    speed_meters_per_second: f64,
    /// Gerstner horizontal-displacement factor. Unused by the CPU height
    /// query, which only needs the vertical term, but carried here so the
    /// self-intersection budget can be computed and held from one place.
    steepness: f64,
}

const OCEAN_RIPPLE_WAVES: [GerstnerWave; 3] = [
    GerstnerWave {
        direction: DVec3::new(0.72, 0.18, -0.67),
        wavelength_meters: 180.0,
        amplitude_meters: 1.8,
        storm_amplitude_meters: 1.8,
        speed_meters_per_second: 14.0,
        steepness: 0.0,
    },
    GerstnerWave {
        direction: DVec3::new(-0.31, 0.91, 0.28),
        wavelength_meters: 70.0,
        amplitude_meters: 1.64,
        storm_amplitude_meters: 1.64,
        speed_meters_per_second: 11.0,
        steepness: 0.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.15, -0.58, 0.80),
        wavelength_meters: 28.0,
        amplitude_meters: 1.20,
        storm_amplitude_meters: 1.20,
        speed_meters_per_second: 8.0,
        steepness: 0.0,
    },
];

/// Two narrow-band swells plus a broad-directional wind sea. One component per
/// wavelength band renders as a single perfect plane wave, so a handful of them
/// read as "one big regular wave plus one small regular wave"; real irregularity
/// comes from the wind sea carrying many components spread widely in azimuth.
/// Mirrored byte-for-byte by `OCEAN_WAVE_TABLE` in `shared_planet.wgsl`.
const WAVES: [GerstnerWave; 17] = [
    GerstnerWave {
        direction: DVec3::new(0.9, 0.1, 0.4),
        wavelength_meters: 1400.0,
        amplitude_meters: 0.375,
        storm_amplitude_meters: 0.09,
        speed_meters_per_second: 10.0,
        steepness: 0.45,
    },
    GerstnerWave {
        direction: DVec3::new(0.86, 0.18, 0.48),
        wavelength_meters: 1400.0,
        amplitude_meters: 0.375,
        storm_amplitude_meters: 0.09,
        speed_meters_per_second: 9.2,
        steepness: 0.4,
    },
    GerstnerWave {
        direction: DVec3::new(0.1596, -0.599, 0.7847),
        wavelength_meters: 430.0,
        amplitude_meters: 0.0,
        storm_amplitude_meters: 0.185,
        speed_meters_per_second: 24.0,
        steepness: 1.5,
    },
    GerstnerWave {
        direction: DVec3::new(0.297, -0.7478, 0.5938),
        wavelength_meters: 350.0,
        amplitude_meters: 0.0,
        storm_amplitude_meters: 0.205,
        speed_meters_per_second: 21.5,
        steepness: 1.5,
    },
    GerstnerWave {
        direction: DVec3::new(0.3987, -0.8308, 0.3884),
        wavelength_meters: 280.0,
        amplitude_meters: 0.0,
        storm_amplitude_meters: 0.18,
        speed_meters_per_second: 19.0,
        steepness: 1.5,
    },
    GerstnerWave {
        direction: DVec3::new(0.576, -0.8032, 0.1519),
        wavelength_meters: 200.0,
        amplitude_meters: 0.0495,
        storm_amplitude_meters: 0.0495,
        speed_meters_per_second: 6.0,
        steepness: 0.34,
    },
    GerstnerWave {
        direction: DVec3::new(0.4646, -0.1875, 0.8654),
        wavelength_meters: 147.5,
        amplitude_meters: 0.0383,
        storm_amplitude_meters: 0.0383,
        speed_meters_per_second: 6.59,
        steepness: 0.32,
    },
    GerstnerWave {
        direction: DVec3::new(0.5761, -0.8032, 0.1515),
        wavelength_meters: 108.7,
        amplitude_meters: 0.0295,
        storm_amplitude_meters: 0.0295,
        speed_meters_per_second: 7.18,
        steepness: 0.3,
    },
    GerstnerWave {
        direction: DVec3::new(0.2007, 0.0492, 0.9784),
        wavelength_meters: 80.2,
        amplitude_meters: 0.0228,
        storm_amplitude_meters: 0.0228,
        speed_meters_per_second: 7.77,
        steepness: 0.28,
    },
    GerstnerWave {
        direction: DVec3::new(0.49, -0.8612, -0.1353),
        wavelength_meters: 59.1,
        amplitude_meters: 0.0176,
        storm_amplitude_meters: 0.0176,
        speed_meters_per_second: 8.36,
        steepness: 0.26,
    },
    GerstnerWave {
        direction: DVec3::new(0.1087, 0.131, 0.9854),
        wavelength_meters: 43.6,
        amplitude_meters: 0.0136,
        storm_amplitude_meters: 0.0136,
        speed_meters_per_second: 8.95,
        steepness: 0.24,
    },
    GerstnerWave {
        direction: DVec3::new(0.5241, -0.8493, -0.063),
        wavelength_meters: 32.1,
        amplitude_meters: 0.0105,
        storm_amplitude_meters: 0.0105,
        speed_meters_per_second: 9.55,
        steepness: 0.22,
    },
    GerstnerWave {
        direction: DVec3::new(-0.0574, 0.252, 0.966),
        wavelength_meters: 23.7,
        amplitude_meters: 0.0081,
        storm_amplitude_meters: 0.0081,
        speed_meters_per_second: 10.14,
        steepness: 0.2,
    },
    GerstnerWave {
        direction: DVec3::new(0.3157, -0.8148, -0.4862),
        wavelength_meters: 17.5,
        amplitude_meters: 0.0062,
        storm_amplitude_meters: 0.0062,
        speed_meters_per_second: 10.73,
        steepness: 0.18,
    },
    GerstnerWave {
        direction: DVec3::new(0.1407, 0.1008, 0.9849),
        wavelength_meters: 12.9,
        amplitude_meters: 0.0048,
        storm_amplitude_meters: 0.0048,
        speed_meters_per_second: 11.32,
        steepness: 0.16,
    },
    GerstnerWave {
        direction: DVec3::new(0.3542, -0.8289, -0.4329),
        wavelength_meters: 9.5,
        amplitude_meters: 0.0037,
        storm_amplitude_meters: 0.0037,
        speed_meters_per_second: 11.91,
        steepness: 0.14,
    },
    GerstnerWave {
        direction: DVec3::new(-0.2008, 0.3721, 0.9062),
        wavelength_meters: 7.0,
        amplitude_meters: 0.0029,
        storm_amplitude_meters: 0.0029,
        speed_meters_per_second: 12.5,
        steepness: 0.12,
    },
];



/// The global waves the surface actually carries under the current toggle.
fn active_waves() -> &'static [GerstnerWave] {
    if OCEAN_LARGE_SWELL_ONLY {
        &WAVES[..LARGE_SWELL_WAVE_COUNT]
    } else {
        &WAVES
    }
}

/// The local ripple octave the surface actually carries under the current
/// toggle. The renderer drops it wholesale in large-swell-only mode.
fn active_ripple_waves() -> &'static [GerstnerWave] {
    if OCEAN_LARGE_SWELL_ONLY {
        &[]
    } else {
        &OCEAN_RIPPLE_WAVES
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WaveHeightStats {
    pub minimum_meters: f32,
    pub maximum_meters: f32,
}

impl WaveHeightStats {
    pub fn range_meters(self) -> f32 {
        self.maximum_meters - self.minimum_meters
    }
}

pub fn wave_height_stats(sim_time: f64, storm_intensity: f32) -> WaveHeightStats {
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for y in -2..=2 {
        for z in -2..=2 {
            let direction =
                (DVec3::X + DVec3::Y * f64::from(y) * 0.002 + DVec3::Z * f64::from(z) * 0.002)
                    .normalize();
            let height = wave_height_meters(direction, sim_time, storm_intensity);
            minimum = minimum.min(height);
            maximum = maximum.max(height);
        }
    }
    WaveHeightStats {
        minimum_meters: minimum as f32,
        maximum_meters: maximum as f32,
    }
}

/// `smoothstep(0.15, 0.85, storm_intensity)`, mirroring the shader.
fn storm_blend(storm_intensity: f32) -> f64 {
    let storm = f64::from(storm_intensity.clamp(0.0, 1.0));
    if storm <= 0.15 {
        0.0
    } else if storm >= 0.85 {
        1.0
    } else {
        let t = (storm - 0.15) / 0.70;
        t * t * (3.0 - 2.0 * t)
    }
}

impl GerstnerWave {
    fn amplitude(self, blend: f64) -> f64 {
        self.amplitude_meters + (self.storm_amplitude_meters - self.amplitude_meters) * blend
    }
}

fn geometry_amplitude_scale(storm_intensity: f32) -> f64 {
    let blend = storm_blend(storm_intensity);
    OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE
        + (OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE - OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE) * blend
}

pub fn maximum_wave_height_meters(storm_intensity: f32) -> f64 {
    let blend = storm_blend(storm_intensity);
    active_waves()
        .iter()
        .map(|wave| wave.amplitude(blend))
        .sum::<f64>()
        * geometry_amplitude_scale(storm_intensity)
}

pub fn wave_height_meters(direction: DVec3, sim_time: f64, storm_intensity: f32) -> f64 {
    let amplitude_scale = geometry_amplitude_scale(storm_intensity);
    let blend = storm_blend(storm_intensity);
    active_waves()
        .iter()
        .map(|wave| {
            let phase = std::f64::consts::TAU / wave.wavelength_meters
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + OCEAN_WAVE_PHASE_SPEED_SIGN * wave.speed_meters_per_second * sim_time);
            wave.amplitude(blend) * amplitude_scale * phase.sin()
        })
        .sum()
}

/// A wave cannot stand taller than the water it is in: past roughly this
/// fraction of the depth, crest to trough, it breaks.
pub const BREAKING_HEIGHT_TO_DEPTH_RATIO: f64 = 0.78;

/// Sharpness of the knee where a wave meets its depth limit.
///
/// The limiter was a `tanh`, which bites everywhere -- it took 15% off a 52m
/// crest in 200m of water, where a wave that size is nowhere near breaking.
/// That put the camera on a shorter sea than the one being drawn. A soft-max
/// leaves anything well under the limit alone and only bends the crest as it
/// approaches, which is what shoaling actually does.
const BREAKING_KNEE: i32 = 4;

/// Limits a wave height to what the local depth can physically hold.
///
/// Returned as a multiplier so height, vertical velocity and slope can all be
/// scaled by the same figure. Deep water returns 1; as the water thins the
/// crest is squeezed toward `0.5 * BREAKING_HEIGHT_TO_DEPTH_RATIO * depth` and
/// flattens off, which is what a wave spilling over a bar actually does.
///
/// The squeeze is a `tanh`, not a hard clamp, so the surface stays smooth and
/// differentiable through the break. Because the result can never exceed the
/// limit, the sea can never cut through the sea bed either -- the limit reaches
/// zero exactly where the water does.
///
/// The earlier version scaled amplitude in proportion to depth. That kept waves
/// off the bed but made shoaling impossible: the crest and the limit shrank
/// together, so their ratio was identical at 1 m and 120 m of depth and nothing
/// ever broke.
pub fn breaking_weight(raw_height_meters: f64, water_depth_meters: f64) -> f64 {
    let limit = breaking_amplitude_limit_meters(water_depth_meters);
    if limit <= 0.0 {
        return 0.0;
    }
    if raw_height_meters.abs() < 1.0e-9 {
        return 1.0;
    }
    let ratio = (raw_height_meters.abs() / limit).powi(BREAKING_KNEE);
    (1.0 + ratio).powf(-1.0 / BREAKING_KNEE as f64)
}

/// The limiter's slope: `d/dx [ L*tanh(h/L) ] = sech^2(h/L) * dh/dx`.
///
/// Rates of change of a limited height -- vertical velocity and surface slope
/// -- scale by this, not by `breaking_weight`. Using the height's own factor
/// for them leaves the analytic velocity disagreeing with a finite difference
/// of the height, which is exactly what the derivative regression catches.
pub fn breaking_rate_weight(raw_height_meters: f64, water_depth_meters: f64) -> f64 {
    let limit = breaking_amplitude_limit_meters(water_depth_meters);
    if limit <= 0.0 {
        return 0.0;
    }
    let ratio = (raw_height_meters.abs() / limit).powi(BREAKING_KNEE);
    (1.0 + ratio).powf(-(BREAKING_KNEE as f64 + 1.0) / BREAKING_KNEE as f64)
}

/// The tallest crest this depth can hold before the wave breaks.
pub fn breaking_amplitude_limit_meters(water_depth_meters: f64) -> f64 {
    0.5 * BREAKING_HEIGHT_TO_DEPTH_RATIO * water_depth_meters.max(0.0)
}

/// How far past breaking the water here is: 0 in open sea, 1 where a crest has
/// reached everything the depth can hold, and pinned at 1 beyond that. This is
/// what turns crests white in the shallows.
pub fn breaking_fraction(water_depth_meters: f64, raw_height_meters: f64) -> f64 {
    let limit = breaking_amplitude_limit_meters(water_depth_meters);
    if limit <= 0.0 {
        return 1.0;
    }
    (raw_height_meters.max(0.0) / limit).min(1.0)
}


pub fn global_wave_height_meters(direction: DVec3, sim_time: f64, water_depth_meters: f64) -> f64 {
    let raw = wave_height_meters(direction, sim_time, GLOBAL_OCEAN_STORM_INTENSITY);
    raw * breaking_weight(raw, water_depth_meters)
}

/// Height at the camera-local ocean patch centre. The renderer adds these
/// three shorter ripples inside its local geometry radius; collision must use
/// the same vertical displacement or the camera will appear to ignore nearby
/// crests while only following the broad swell.
pub fn local_wave_height_meters(direction: DVec3, sim_time: f64, water_depth_meters: f64) -> f64 {
    let shore_weight = breaking_weight(
        wave_height_meters(direction, sim_time, GLOBAL_OCEAN_STORM_INTENSITY),
        water_depth_meters,
    );
    let ripple_height = active_ripple_waves()
        .iter()
        .map(|wave| {
            let phase = std::f64::consts::TAU / wave.wavelength_meters
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + OCEAN_WAVE_PHASE_SPEED_SIGN * wave.speed_meters_per_second * sim_time);
            wave.amplitude_meters * phase.sin()
        })
        .sum::<f64>();
    global_wave_height_meters(direction, sim_time, water_depth_meters)
        + if OCEAN_RIPPLES_ARE_GEOMETRIC {
            ripple_height * shore_weight
        } else {
            0.0
        }
}

/// Tangential gradient of the global wave surface: metres of rise per metre of
/// horizontal travel, as a vector in the planet frame.
///
/// This is what tilts a floating hull's buoyancy off the vertical. Buoyancy
/// normal to a sloped surface has a horizontal component, and because that
/// component differs along the hull it is also the only thing in the model
/// that can yaw it; with the force pinned to the radial a hull can heave and
/// tilt but never swings its head.
pub fn global_wave_slope(direction: DVec3, sim_time: f64, water_depth_meters: f64) -> DVec3 {
    let radial = direction.normalize();
    let amplitude_scale = geometry_amplitude_scale(GLOBAL_OCEAN_STORM_INTENSITY);
    let blend = storm_blend(GLOBAL_OCEAN_STORM_INTENSITY);
    // d(phase)/ds along a unit tangent u is wave_number * (u . axis): the
    // planet radius in the phase cancels against the 1/radius change in
    // `direction` from moving a metre tangentially.
    let gradient = active_waves()
        .iter()
        .map(|wave| {
            let axis = wave.direction.normalize();
            let wave_number = std::f64::consts::TAU / wave.wavelength_meters;
            let phase = wave_number
                * (radial.dot(axis) * PLANET_RADIUS_METERS
                    + OCEAN_WAVE_PHASE_SPEED_SIGN * wave.speed_meters_per_second * sim_time);
            axis * (wave.amplitude(blend) * amplitude_scale * wave_number * phase.cos())
        })
        .sum::<DVec3>()
        * breaking_rate_weight(
            wave_height_meters(radial, sim_time, GLOBAL_OCEAN_STORM_INTENSITY),
            water_depth_meters,
        );
    gradient - radial * gradient.dot(radial)
}

pub fn global_wave_vertical_velocity_meters_per_second(
    direction: DVec3,
    sim_time: f64,
    water_depth_meters: f64,
) -> f64 {
    let amplitude_scale = geometry_amplitude_scale(GLOBAL_OCEAN_STORM_INTENSITY);
    let blend = storm_blend(GLOBAL_OCEAN_STORM_INTENSITY);
    let vertical_velocity = active_waves()
        .iter()
        .map(|wave| {
            let wave_number = std::f64::consts::TAU / wave.wavelength_meters;
            let phase = wave_number
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + OCEAN_WAVE_PHASE_SPEED_SIGN * wave.speed_meters_per_second * sim_time);
            OCEAN_WAVE_PHASE_SPEED_SIGN
                * wave.amplitude(blend)
                * amplitude_scale
                * wave_number
                * wave.speed_meters_per_second
                * phase.cos()
        })
        .sum::<f64>();
    // Scaled by the same figure the height was, so a limited crest and its
    // velocity stay consistent: the analytic derivative regression compares
    // them directly.
    vertical_velocity
        * breaking_rate_weight(
            wave_height_meters(direction, sim_time, GLOBAL_OCEAN_STORM_INTENSITY),
            water_depth_meters,
        )
}

pub fn local_wave_vertical_velocity_meters_per_second(
    direction: DVec3,
    sim_time: f64,
    water_depth_meters: f64,
) -> f64 {
    let shore_weight = breaking_weight(
        wave_height_meters(direction, sim_time, GLOBAL_OCEAN_STORM_INTENSITY),
        water_depth_meters,
    );
    let ripple_velocity = active_ripple_waves()
        .iter()
        .map(|wave| {
            let wave_number = std::f64::consts::TAU / wave.wavelength_meters;
            let phase = wave_number
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + OCEAN_WAVE_PHASE_SPEED_SIGN * wave.speed_meters_per_second * sim_time);
            OCEAN_WAVE_PHASE_SPEED_SIGN
                * wave.amplitude_meters
                * wave_number
                * wave.speed_meters_per_second
                * phase.cos()
        })
        .sum::<f64>();
    global_wave_vertical_velocity_meters_per_second(direction, sim_time, water_depth_meters)
        + if OCEAN_RIPPLES_ARE_GEOMETRIC {
            ripple_velocity * shore_weight
        } else {
            0.0
        }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::{
        GLOBAL_OCEAN_STORM_INTENSITY, LARGE_SWELL_WAVE_COUNT, MAXIMUM_WAVE_HEIGHT_METERS,
        OCEAN_STEEPNESS_SCALE, OCEAN_WAVE_SCALE,
        OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE, OCEAN_LARGE_SWELL_ONLY, OCEAN_RIPPLE_WAVES,
        OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE, WAVES, active_ripple_waves, active_waves,
        geometry_amplitude_scale, global_wave_height_meters,
        global_wave_vertical_velocity_meters_per_second, maximum_wave_height_meters,
        breaking_weight, wave_height_stats,
    };

    /// Every wave in the table, at storm scale. Independent of the diagnostic
    /// toggle, so it still guards the table itself.
    const FULL_TABLE_MAXIMUM_METERS: f64 = 52.6625;
    /// The dominant swell pair alone, at storm scale.
    const LARGE_SWELL_ONLY_MAXIMUM_METERS: f64 = 41.25;

    #[test]
    fn gerstner_wave_height_stats_are_non_zero_and_time_varying() {
        let first = wave_height_stats(0.0, 0.0);
        let later = wave_height_stats(1.0, 0.0);
        assert!(first.range_meters() > 2.0);
        assert!(later.range_meters() > 2.0);
        assert_ne!(first.minimum_meters, later.minimum_meters);
        assert!(f64::from(first.maximum_meters) <= super::MAXIMUM_WAVE_HEIGHT_METERS);
        assert!(f64::from(later.maximum_meters) <= super::MAXIMUM_WAVE_HEIGHT_METERS);
    }

    #[test]
    fn storm_intensity_smoothly_reaches_the_giant_wave_scale() {
        assert_eq!(
            geometry_amplitude_scale(0.0),
            OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE
        );
        assert_eq!(
            geometry_amplitude_scale(1.0),
            OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE
        );
        assert!(geometry_amplitude_scale(0.5) > OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE);
        assert!(geometry_amplitude_scale(0.5) < OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE);
        let maximum_possible_height = maximum_wave_height_meters(1.0);
        assert!(maximum_possible_height <= MAXIMUM_WAVE_HEIGHT_METERS);
        assert!(maximum_possible_height >= 20.0);
    }

    #[test]
    fn dominant_wave_pair_is_equal_amplitude_and_slightly_off_parallel() {
        assert_eq!(WAVES[0].amplitude_meters, WAVES[1].amplitude_meters);
        let angle_degrees = WAVES[0]
            .direction
            .angle_between(WAVES[1].direction)
            .to_degrees();
        assert!(
            (5.0..10.0).contains(&angle_degrees),
            "angle was {angle_degrees}"
        );
    }

    #[test]
    fn global_sea_is_maximum_storm_but_waves_shoal_at_the_coast() {
        assert_eq!(GLOBAL_OCEAN_STORM_INTENSITY, 1.0);
        // A crest that fits is essentially untouched. tanh approaches 1 rather
        // than reaching it, so a 1 m crest in 1000 m of water keeps all but
        // (h/limit)^2/3 of itself -- about two parts per million.
        assert!((breaking_weight(1.0, 1000.0) - 1.0).abs() < 1.0e-5);
        assert!(breaking_weight(1.0, 1000.0) < 1.0);
        // No water, no wave: this is what keeps the surface off the sea bed.
        assert_eq!(breaking_weight(5.0, 0.0), 0.0);
        // Shallow water still has a sea in it, just a shorter one. The old
        // taper returned zero here, which is why the shallows read as static.
        assert!(breaking_weight(5.0, 2.0) > 0.0);
        // Monotonic in depth, and never taller than the depth can hold: that
        // second one is the anti-clipping guarantee, and it has to hold at
        // every depth rather than at the few a taper was tuned for.
        // The anti-clipping guarantee, at every depth and for any crest the
        // spectrum can raise: a limited crest never exceeds what the water
        // holds, so the surface cannot cut through the sea bed.
        for step in 0..=400 {
            let depth = step as f64 * 0.5;
            let holdable = super::breaking_amplitude_limit_meters(depth);
            for raw in [0.5, 2.0, 8.0, 30.0, MAXIMUM_WAVE_HEIGHT_METERS] {
                let limited = raw * breaking_weight(raw, depth);
                assert!(
                    limited <= holdable + 1.0e-9,
                    "a {raw} m crest limited to {limited} m still exceeds the \
                     {holdable} m that {depth} m of water can hold"
                );
                assert!(limited.is_finite());
            }
        }
        // Shoaling: the same crest is squeezed harder as the water thins, which
        // is what the proportional taper could not do.
        let deep = 30.0 * breaking_weight(30.0, 400.0);
        let shallow = 30.0 * breaking_weight(30.0, 12.0);
        assert!(shallow < deep, "crest did not shorten as it shoaled");
        // Breaking: a crest using everything the depth holds is fully broken,
        // and open sea is not breaking at all.
        // A crest at the limit the depth can hold is fully broken; one that
        // over-tops it stays clamped there.
        assert!(super::breaking_fraction(10.0, 3.9) > 0.999);
        assert_eq!(super::breaking_fraction(10.0, 40.0), 1.0);
        // Open sea is not breaking, and a trough never is.
        assert!(super::breaking_fraction(1000.0, 3.9) < 0.02);
        assert_eq!(super::breaking_fraction(1000.0, -8.0), 0.0);
        let direction = DVec3::new(0.3, 0.8, -0.5).normalize();
        assert_eq!(global_wave_height_meters(direction, 3.0, 0.0), 0.0);
        // Deep water leaves the swell essentially untouched -- essentially,
        // not exactly, because the limiter approaches full height rather than
        // switching off at a threshold. That smoothness is the point: there is
        // no depth at which the sea visibly changes behaviour.
        let deep = global_wave_height_meters(direction, 3.0, 1000.0);
        let unlimited = super::wave_height_meters(direction, 3.0, 1.0);
        assert!(
            (deep - unlimited).abs() / unlimited.abs() < 1.0e-3,
            "deep water gave {deep} m against an unlimited {unlimited} m"
        );
        let shader = include_str!("shared_planet.wgsl");
        assert!(shader.contains(
            "0.5 * OCEAN_BREAKING_HEIGHT_TO_DEPTH_RATIO * max(water_depth_meters, 0.0)"
        ));
    }

    #[test]
    fn cpu_wave_scale_matches_the_rendered_ocean_scale() {
        let shader = crate::planet::shared_planet_shader_source();
        assert!(shader.contains(&format!(
            "const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE: f32 = {};",
            wgsl_number(OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE)
        )));
        assert!(shader.contains(&format!(
            "const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE: f32 = {};",
            wgsl_number(OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE)
        )));
        assert!(shader.contains("let horizontal_transport = select(1.0, 0.0"));
        let calm_sum = WAVES.iter().map(|wave| wave.amplitude_meters).sum::<f64>();
        let storm_sum = WAVES
            .iter()
            .map(|wave| wave.storm_amplitude_meters)
            .sum::<f64>();
        // A storm redistributes the spectrum, it does not add energy: if these
        // two drift apart the height cap silently stops holding mid-blend.
        assert!((calm_sum - storm_sum).abs() < 1.0e-9, "{calm_sum} vs {storm_sum}");
        let full_table_maximum = storm_sum * OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE;
        assert!((full_table_maximum - FULL_TABLE_MAXIMUM_METERS).abs() < 1.0e-9);
        for intensity in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert!(maximum_wave_height_meters(intensity) <= MAXIMUM_WAVE_HEIGHT_METERS);
        }
        let expected = if OCEAN_LARGE_SWELL_ONLY {
            LARGE_SWELL_ONLY_MAXIMUM_METERS
        } else {
            FULL_TABLE_MAXIMUM_METERS
        };
        assert!((maximum_wave_height_meters(1.0) - expected).abs() < 1.0e-9);
    }

    #[test]
    fn the_analytic_wave_slope_matches_a_centred_finite_difference() {
        use super::PLANET_RADIUS_METERS;
        let direction = DVec3::new(0.836, 0.504, 0.216).normalize();
        let sim_time = 41.5;
        let depth = 4000.0;
        let slope = super::global_wave_slope(direction, sim_time, depth);
        // Two tangents, so both components of the gradient are checked.
        let first = direction.cross(DVec3::Y).normalize();
        let second = direction.cross(first).normalize();
        for tangent in [first, second] {
            // The shortest wave in the table is 7m, so a coarse step measures
            // the difference's own truncation error rather than the gradient.
            let step_meters = 0.02;
            let offset = |sign: f64| {
                (direction * PLANET_RADIUS_METERS + tangent * (sign * step_meters)).normalize()
            };
            let numeric = (global_wave_height_meters(offset(1.0), sim_time, depth)
                - global_wave_height_meters(offset(-1.0), sim_time, depth))
                / (2.0 * step_meters);
            let analytic = slope.dot(tangent);
            assert!(
                (analytic - numeric).abs() < 1.0e-4,
                "analytic {analytic} vs finite difference {numeric}"
            );
        }
        // A gradient is tangential by construction; a radial component would
        // tilt the surface normal toward the planet centre.
        assert!(slope.dot(direction).abs() < 1.0e-9);
    }

    /// WGSL wants `44.0`, Rust's `{}` prints `44`.
    fn wgsl_number(value: f64) -> String {
        if value.fract() == 0.0 {
            format!("{value:.1}")
        } else {
            format!("{value}")
        }
    }

    #[test]
    fn the_camera_only_follows_water_the_renderer_actually_displaces() {
        // The ripple octave is shading detail: ocean_surface hands it out for
        // colour and normal, and vs_ocean displaces by neither. A camera that
        // adds it to its collision surface floats on 4.6m of water nobody drew,
        // and goes under a crest it cannot see.
        let shader = crate::planet::shared_planet_shader_source()
            + include_str!("planet.wgsl");
        let displaces_ripple = shader.contains(
            "+ projected.direction * surface.vertical_displacement\n        + surface.ripple_height",
        );
        assert_eq!(
            displaces_ripple,
            super::OCEAN_RIPPLES_ARE_GEOMETRIC,
            "OCEAN_RIPPLES_ARE_GEOMETRIC says {} but vs_ocean {} the ripple \
             height into the surface position",
            super::OCEAN_RIPPLES_ARE_GEOMETRIC,
            if displaces_ripple { "does add" } else { "does not add" }
        );
        // Whichever way that goes, the two local queries must agree with the
        // global one about whether ripples exist at all.
        let direction = DVec3::new(0.836, 0.504, 0.216).normalize();
        let global = global_wave_height_meters(direction, 7.0, 4000.0);
        let local = super::local_wave_height_meters(direction, 7.0, 4000.0);
        if super::OCEAN_RIPPLES_ARE_GEOMETRIC {
            assert!((local - global).abs() > 1.0e-6);
        } else {
            assert!(
                (local - global).abs() < 1.0e-9,
                "local query is {} m off the drawn surface",
                local - global
            );
        }
    }

    #[test]
    fn horizontal_transport_stays_off_while_the_cpu_query_is_radial() {
        // wave_height_meters takes a direction and returns the height on that
        // radial. It has no horizontal term, so it can only describe the
        // rendered surface while the renderer has none either. Enabling
        // transport without adding one puts the camera under the water.
        assert!(
            !super::OCEAN_HORIZONTAL_TRANSPORT_ENABLED,
            "give wave_height_meters a horizontal displacement term before \
             enabling transport; the radial query cannot follow a surface that \
             slides up to 54m sideways"
        );
        let shader = crate::planet::shared_planet_shader_source();
        assert!(shader.contains("let horizontal_transport = select(1.0, 0.0,"));
    }

    #[test]
    fn the_wave_scale_knob_derives_every_constant_it_is_coupled_to() {
        // Raising the sea used to mean hand-editing six constants across two
        // languages, where missing one fails quietly. This holds the knob to
        // owning all of them, and prints the shader lines to paste when it
        // moves.
        // The cap is the table's own sum, so it cannot drift from it.
        let storm_sum: f64 = WAVES.iter().map(|wave| wave.storm_amplitude_meters).sum();
        assert!(
            (MAXIMUM_WAVE_HEIGHT_METERS - storm_sum * OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE).abs()
                < 1.0e-9
        );

        // The point of deriving steepness: the fold budget does not move when
        // the knob does. Without that, scaling the sea scales it straight into
        // self-intersection.
        let budget = super::fold_budget_at(1.0);
        for wave_scale in [0.5, 2.0, 5.0, 20.0] {
            assert!(
                (super::fold_budget_at(wave_scale) - budget).abs() < 1.0e-9,
                "fold budget moved to {} at scale {wave_scale}",
                super::fold_budget_at(wave_scale)
            );
        }

        // The camera's underwater floor has to stay below the deepest trough,
        // or a falling trough pins the eye there and it reports clearance it
        // does not have. This is the knob's real ceiling: about
        // OCEAN_WAVE_SCALE 1.8 at today's floor.
        assert!(
            crate::surface_camera::PLANET_CORE_CLEARANCE_METERS < -MAXIMUM_WAVE_HEIGHT_METERS,
            "a {MAXIMUM_WAVE_HEIGHT_METERS} m trough reaches past the {} m safety floor; \
             lower PLANET_CORE_CLEARANCE_METERS before raising OCEAN_WAVE_SCALE further",
            crate::surface_camera::PLANET_CORE_CLEARANCE_METERS
        );

        // The assembled source, not the raw file: these constants only exist
        // once the generator has put them there.
        let shader = crate::planet::shared_planet_shader_source();
        for line in [
            format!(
                "const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE: f32 = {};",
                wgsl_number(OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE)
            ),
            format!(
                "const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE: f32 = {};",
                wgsl_number(OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE)
            ),
            format!(
                "const OCEAN_STEEPNESS_SCALE: f32 = {};",
                wgsl_number(OCEAN_STEEPNESS_SCALE)
            ),
        ] {
            assert!(
                shader.contains(&line),
                "OCEAN_WAVE_SCALE is {OCEAN_WAVE_SCALE}, so the generated prelude must carry:\n    {line}"
            );
        }

        // And the raw file must not declare them again. A second copy would
        // shadow or clash with the generated one and put the drift straight
        // back.
        let raw = include_str!("shared_planet.wgsl");
        for name in [
            "const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE",
            "const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE",
            "const OCEAN_STEEPNESS_SCALE",
        ] {
            assert!(
                !raw.contains(name),
                "{name} is generated from OCEAN_WAVE_SCALE; declaring it in \
                 shared_planet.wgsl as well reintroduces the copy that drifts"
            );
        }
    }

    #[test]
    fn large_swell_only_is_paired_with_the_shader() {
        let shader = include_str!("shared_planet.wgsl");
        assert!(shader.contains(&format!(
            "const OCEAN_LARGE_SWELL_ONLY: bool = {OCEAN_LARGE_SWELL_ONLY};"
        )));
        assert!(shader.contains(&format!(
            "const OCEAN_LARGE_SWELL_WAVE_COUNT: u32 = {LARGE_SWELL_WAVE_COUNT}u;"
        )));
        assert!(shader.contains(&format!("const OCEAN_WAVE_COUNT: u32 = {}u;", WAVES.len())));
        assert!(shader.contains("if OCEAN_LARGE_SWELL_ONLY && i >= OCEAN_LARGE_SWELL_WAVE_COUNT {"));
        assert!(shader.contains("if OCEAN_LARGE_SWELL_ONLY {"));
        let (expected_waves, expected_ripples) = if OCEAN_LARGE_SWELL_ONLY {
            (LARGE_SWELL_WAVE_COUNT, 0)
        } else {
            (WAVES.len(), OCEAN_RIPPLE_WAVES.len())
        };
        assert_eq!(active_waves().len(), expected_waves);
        assert_eq!(active_ripple_waves().len(), expected_ripples);
    }

    #[test]
    fn cpu_wave_table_matches_the_shader_wave_table() {
        // The divergence this catches is silent: a GPU-only amplitude edit
        // leaves collision following water the renderer stopped drawing.
        let shader = include_str!("shared_planet.wgsl");
        for (index, wave) in WAVES.iter().enumerate() {
            let amplitude = if index < LARGE_SWELL_WAVE_COUNT {
                format!("{}", wave.amplitude_meters)
            } else {
                format!("{} * short_swell_scale", wave.amplitude_meters)
            };
            // Direction is part of the pair: an axis that matches on one side
            // only would leave collision following a differently aimed sea.
            let _ = amplitude;
            // Rust prints 1400.0 as "1400"; WGSL literals carry the ".0".
            let n = |value: f64| {
                if value.fract() == 0.0 {
                    format!("{value:.1}")
                } else {
                    format!("{value}")
                }
            };
            let needle = format!(
                "OceanWaveSpec(vec3<f32>({}, {}, {}), {}, {}, {}, {},",
                n(wave.direction.x),
                n(wave.direction.y),
                n(wave.direction.z),
                n(wave.wavelength_meters),
                n(wave.amplitude_meters),
                n(wave.storm_amplitude_meters),
                n(wave.speed_meters_per_second),
            );
            assert!(shader.contains(&needle), "shader is missing `{needle}`");
        }
        for wave in OCEAN_RIPPLE_WAVES.iter() {
            let needle = format!("{}", wave.amplitude_meters);
            assert!(shader.contains(&needle), "shader is missing ripple `{needle}`");
        }
    }

    #[test]
    fn analytic_wave_velocity_matches_a_small_time_difference() {
        let direction = DVec3::new(0.3, 0.8, -0.5).normalize();
        let time = 7.0;
        let epsilon = 1.0e-4;
        let finite_difference = (global_wave_height_meters(direction, time + epsilon, 1000.0)
            - global_wave_height_meters(direction, time - epsilon, 1000.0))
            / (2.0 * epsilon);
        let analytic = global_wave_vertical_velocity_meters_per_second(direction, time, 1000.0);
        assert!((analytic - finite_difference).abs() < 1.0e-5);
        assert_eq!(
            global_wave_vertical_velocity_meters_per_second(direction, time, 0.0),
            0.0
        );
    }
}

#[cfg(test)]
mod breaking_probe {
    use glam::DVec3;

    /// Instrument, not an assertion: prints how close crests come to breaking
    /// at each depth. Run with
    /// `cargo test -- --ignored --nocapture crest_to_limit`.
    #[test]
    #[ignore = "instrument, not an assertion"]
    fn crest_to_limit_ratio_in_the_shallows() {
        let direction = DVec3::new(0.836442275001636, 0.503727905284262, 0.215922481525239)
            .normalize();
        for depth in [1.0, 2.0, 4.0, 8.0, 16.0, 40.0, 120.0] {
            let mut peak: f64 = 0.0;
            let mut sum = 0.0;
            let mut n = 0.0;
            for step in 0..2000 {
                let t = step as f64 * 0.05;
                let h = super::global_wave_height_meters(direction, t, depth);
                let f = super::breaking_fraction(depth, h);
                peak = peak.max(f);
                sum += f;
                n += 1.0;
            }
            println!(
                "depth {depth:6.1} m: peak breaking fraction {peak:.3}, mean {:.3}",
                sum / n
            );
        }
    }
}

use glam::DVec3;

use crate::planet::PLANET_RADIUS_METERS;

// Keep these values byte-for-byte aligned with the raster/ray WGSL ocean
// surface. Collision and buoyancy must sample the same displaced water that
// the player sees; a stale CPU scale leaves a camera apparently above water
// while the rendered crest has already passed over it.
pub const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE: f64 = 44.0;
pub const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE: f64 = 55.0;
pub const MAXIMUM_WAVE_HEIGHT_METERS: f64 = 53.0;
pub const GLOBAL_OCEAN_STORM_INTENSITY: f32 = 1.0;
/// Diagnostic: collide and render against only the two 1,400 m swells at the
/// head of `WAVES`, dropping the 160/65/24/9 m global waves and the whole
/// ripple layer. Must match `OCEAN_LARGE_SWELL_ONLY` in `shared_planet.wgsl`,
/// which `large_swell_only_is_paired_with_the_shader` enforces.
pub const OCEAN_LARGE_SWELL_ONLY: bool = false;
/// Number of leading `WAVES` entries that make up the dominant swell pair.
const LARGE_SWELL_WAVE_COUNT: usize = 2;
const OCEAN_SHORE_WAVE_START_DEPTH_METERS: f64 = 2.0;
const OCEAN_SHORE_FULL_WAVE_DEPTH_METERS: f64 = 30.0;
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
}

const OCEAN_RIPPLE_WAVES: [GerstnerWave; 3] = [
    GerstnerWave {
        direction: DVec3::new(0.72, 0.18, -0.67),
        wavelength_meters: 180.0,
        amplitude_meters: 1.8,
        storm_amplitude_meters: 1.8,
        speed_meters_per_second: 14.0,
    },
    GerstnerWave {
        direction: DVec3::new(-0.31, 0.91, 0.28),
        wavelength_meters: 70.0,
        amplitude_meters: 1.64,
        storm_amplitude_meters: 1.64,
        speed_meters_per_second: 11.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.15, -0.58, 0.80),
        wavelength_meters: 28.0,
        amplitude_meters: 1.20,
        storm_amplitude_meters: 1.20,
        speed_meters_per_second: 8.0,
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
    },
    GerstnerWave {
        direction: DVec3::new(0.86, 0.18, 0.48),
        wavelength_meters: 1400.0,
        amplitude_meters: 0.375,
        storm_amplitude_meters: 0.09,
        speed_meters_per_second: 9.2,
    },
    GerstnerWave {
        direction: DVec3::new(0.1596, -0.599, 0.7847),
        wavelength_meters: 430.0,
        amplitude_meters: 0.0,
        storm_amplitude_meters: 0.185,
        speed_meters_per_second: 24.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.297, -0.7478, 0.5938),
        wavelength_meters: 350.0,
        amplitude_meters: 0.0,
        storm_amplitude_meters: 0.205,
        speed_meters_per_second: 21.5,
    },
    GerstnerWave {
        direction: DVec3::new(0.3987, -0.8308, 0.3884),
        wavelength_meters: 280.0,
        amplitude_meters: 0.0,
        storm_amplitude_meters: 0.18,
        speed_meters_per_second: 19.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.576, -0.8032, 0.1519),
        wavelength_meters: 200.0,
        amplitude_meters: 0.0495,
        storm_amplitude_meters: 0.0495,
        speed_meters_per_second: 6.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.4646, -0.1875, 0.8654),
        wavelength_meters: 147.5,
        amplitude_meters: 0.0383,
        storm_amplitude_meters: 0.0383,
        speed_meters_per_second: 6.59,
    },
    GerstnerWave {
        direction: DVec3::new(0.5761, -0.8032, 0.1515),
        wavelength_meters: 108.7,
        amplitude_meters: 0.0295,
        storm_amplitude_meters: 0.0295,
        speed_meters_per_second: 7.18,
    },
    GerstnerWave {
        direction: DVec3::new(0.2007, 0.0492, 0.9784),
        wavelength_meters: 80.2,
        amplitude_meters: 0.0228,
        storm_amplitude_meters: 0.0228,
        speed_meters_per_second: 7.77,
    },
    GerstnerWave {
        direction: DVec3::new(0.49, -0.8612, -0.1353),
        wavelength_meters: 59.1,
        amplitude_meters: 0.0176,
        storm_amplitude_meters: 0.0176,
        speed_meters_per_second: 8.36,
    },
    GerstnerWave {
        direction: DVec3::new(0.1087, 0.131, 0.9854),
        wavelength_meters: 43.6,
        amplitude_meters: 0.0136,
        storm_amplitude_meters: 0.0136,
        speed_meters_per_second: 8.95,
    },
    GerstnerWave {
        direction: DVec3::new(0.5241, -0.8493, -0.063),
        wavelength_meters: 32.1,
        amplitude_meters: 0.0105,
        storm_amplitude_meters: 0.0105,
        speed_meters_per_second: 9.55,
    },
    GerstnerWave {
        direction: DVec3::new(-0.0574, 0.252, 0.966),
        wavelength_meters: 23.7,
        amplitude_meters: 0.0081,
        storm_amplitude_meters: 0.0081,
        speed_meters_per_second: 10.14,
    },
    GerstnerWave {
        direction: DVec3::new(0.3157, -0.8148, -0.4862),
        wavelength_meters: 17.5,
        amplitude_meters: 0.0062,
        storm_amplitude_meters: 0.0062,
        speed_meters_per_second: 10.73,
    },
    GerstnerWave {
        direction: DVec3::new(0.1407, 0.1008, 0.9849),
        wavelength_meters: 12.9,
        amplitude_meters: 0.0048,
        storm_amplitude_meters: 0.0048,
        speed_meters_per_second: 11.32,
    },
    GerstnerWave {
        direction: DVec3::new(0.3542, -0.8289, -0.4329),
        wavelength_meters: 9.5,
        amplitude_meters: 0.0037,
        storm_amplitude_meters: 0.0037,
        speed_meters_per_second: 11.91,
    },
    GerstnerWave {
        direction: DVec3::new(-0.2008, 0.3721, 0.9062),
        wavelength_meters: 7.0,
        amplitude_meters: 0.0029,
        storm_amplitude_meters: 0.0029,
        speed_meters_per_second: 12.5,
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
                    + wave.speed_meters_per_second * sim_time);
            wave.amplitude(blend) * amplitude_scale * phase.sin()
        })
        .sum()
}

pub fn shore_wave_weight(water_depth_meters: f64) -> f64 {
    let t = ((water_depth_meters - OCEAN_SHORE_WAVE_START_DEPTH_METERS)
        / (OCEAN_SHORE_FULL_WAVE_DEPTH_METERS - OCEAN_SHORE_WAVE_START_DEPTH_METERS))
        .clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn global_wave_height_meters(direction: DVec3, sim_time: f64, water_depth_meters: f64) -> f64 {
    wave_height_meters(direction, sim_time, GLOBAL_OCEAN_STORM_INTENSITY)
        * shore_wave_weight(water_depth_meters)
}

/// Height at the camera-local ocean patch centre. The renderer adds these
/// three shorter ripples inside its local geometry radius; collision must use
/// the same vertical displacement or the camera will appear to ignore nearby
/// crests while only following the broad swell.
pub fn local_wave_height_meters(direction: DVec3, sim_time: f64, water_depth_meters: f64) -> f64 {
    let shore_weight = shore_wave_weight(water_depth_meters);
    let ripple_height = active_ripple_waves()
        .iter()
        .map(|wave| {
            let phase = std::f64::consts::TAU / wave.wavelength_meters
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + wave.speed_meters_per_second * sim_time);
            wave.amplitude_meters * phase.sin()
        })
        .sum::<f64>();
    global_wave_height_meters(direction, sim_time, water_depth_meters)
        + ripple_height * shore_weight
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
                    + wave.speed_meters_per_second * sim_time);
            wave.amplitude(blend)
                * amplitude_scale
                * wave_number
                * wave.speed_meters_per_second
                * phase.cos()
        })
        .sum::<f64>();
    vertical_velocity * shore_wave_weight(water_depth_meters)
}

pub fn local_wave_vertical_velocity_meters_per_second(
    direction: DVec3,
    sim_time: f64,
    water_depth_meters: f64,
) -> f64 {
    let shore_weight = shore_wave_weight(water_depth_meters);
    let ripple_velocity = active_ripple_waves()
        .iter()
        .map(|wave| {
            let wave_number = std::f64::consts::TAU / wave.wavelength_meters;
            let phase = wave_number
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + wave.speed_meters_per_second * sim_time);
            wave.amplitude_meters * wave_number * wave.speed_meters_per_second * phase.cos()
        })
        .sum::<f64>();
    global_wave_vertical_velocity_meters_per_second(direction, sim_time, water_depth_meters)
        + ripple_velocity * shore_weight
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::{
        GLOBAL_OCEAN_STORM_INTENSITY, LARGE_SWELL_WAVE_COUNT, MAXIMUM_WAVE_HEIGHT_METERS,
        OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE, OCEAN_LARGE_SWELL_ONLY, OCEAN_RIPPLE_WAVES,
        OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE, WAVES, active_ripple_waves, active_waves,
        geometry_amplitude_scale, global_wave_height_meters,
        global_wave_vertical_velocity_meters_per_second, maximum_wave_height_meters,
        shore_wave_weight, wave_height_stats,
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
        assert_eq!(shore_wave_weight(0.0), 0.0);
        assert_eq!(shore_wave_weight(2.0), 0.0);
        assert_eq!(shore_wave_weight(30.0), 1.0);
        assert_eq!(shore_wave_weight(1000.0), 1.0);
        let direction = DVec3::new(0.3, 0.8, -0.5).normalize();
        assert_eq!(global_wave_height_meters(direction, 3.0, 0.0), 0.0);
        assert_eq!(
            global_wave_height_meters(direction, 3.0, 1000.0),
            super::wave_height_meters(direction, 3.0, 1.0)
        );
        let shader = include_str!("shared_planet.wgsl");
        assert!(shader.contains(
            "let shore_weight = smoothstep(2.0, OCEAN_SHORE_FULL_DEPTH_METERS, water_depth_meters);"
        ));
    }

    #[test]
    fn cpu_wave_scale_matches_the_rendered_ocean_scale() {
        let shader = include_str!("shared_planet.wgsl");
        assert!(shader.contains("const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE: f32 = 44.0;"));
        assert!(shader.contains("const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE: f32 = 55.0;"));
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

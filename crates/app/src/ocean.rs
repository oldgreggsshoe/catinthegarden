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
const OCEAN_SHORE_WAVE_START_DEPTH_METERS: f64 = 2.0;
const OCEAN_SHORE_FULL_WAVE_DEPTH_METERS: f64 = 30.0;
#[derive(Clone, Copy)]
struct GerstnerWave {
    direction: DVec3,
    wavelength_meters: f64,
    amplitude_meters: f64,
    speed_meters_per_second: f64,
}

const OCEAN_RIPPLE_WAVES: [GerstnerWave; 3] = [
    GerstnerWave {
        direction: DVec3::new(0.72, 0.18, -0.67),
        wavelength_meters: 180.0,
        amplitude_meters: 1.8,
        speed_meters_per_second: 14.0,
    },
    GerstnerWave {
        direction: DVec3::new(-0.31, 0.91, 0.28),
        wavelength_meters: 70.0,
        amplitude_meters: 1.64,
        speed_meters_per_second: 11.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.15, -0.58, 0.80),
        wavelength_meters: 28.0,
        amplitude_meters: 1.20,
        speed_meters_per_second: 8.0,
    },
];

const WAVES: [GerstnerWave; 6] = [
    GerstnerWave {
        direction: DVec3::new(0.9, 0.1, 0.4),
        wavelength_meters: 1_400.0,
        amplitude_meters: 0.375,
        speed_meters_per_second: 10.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.86, 0.18, 0.48),
        wavelength_meters: 1_400.0,
        amplitude_meters: 0.375,
        speed_meters_per_second: 9.2,
    },
    GerstnerWave {
        direction: DVec3::new(0.55, -0.75, 0.35),
        wavelength_meters: 160.0,
        amplitude_meters: 0.1125,
        speed_meters_per_second: 6.5,
    },
    GerstnerWave {
        direction: DVec3::new(-0.75, -0.2, 0.63),
        wavelength_meters: 65.0,
        amplitude_meters: 0.055,
        speed_meters_per_second: 8.0,
    },
    GerstnerWave {
        direction: DVec3::new(0.2, 0.95, -0.24),
        wavelength_meters: 24.0,
        amplitude_meters: 0.0275,
        speed_meters_per_second: 10.0,
    },
    GerstnerWave {
        direction: DVec3::new(-0.5, 0.7, -0.5),
        wavelength_meters: 9.0,
        amplitude_meters: 0.0125,
        speed_meters_per_second: 12.0,
    },
];

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

fn geometry_amplitude_scale(storm_intensity: f32) -> f64 {
    let storm = f64::from(storm_intensity.clamp(0.0, 1.0));
    let blend = if storm <= 0.15 {
        0.0
    } else if storm >= 0.85 {
        1.0
    } else {
        let t = (storm - 0.15) / 0.70;
        t * t * (3.0 - 2.0 * t)
    };
    OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE
        + (OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE - OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE) * blend
}

pub fn maximum_wave_height_meters(storm_intensity: f32) -> f64 {
    WAVES.iter().map(|wave| wave.amplitude_meters).sum::<f64>()
        * geometry_amplitude_scale(storm_intensity)
}

pub fn wave_height_meters(direction: DVec3, sim_time: f64, storm_intensity: f32) -> f64 {
    let amplitude_scale = geometry_amplitude_scale(storm_intensity);
    WAVES
        .iter()
        .map(|wave| {
            let phase = std::f64::consts::TAU / wave.wavelength_meters
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + wave.speed_meters_per_second * sim_time);
            wave.amplitude_meters * amplitude_scale * phase.sin()
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
    let ripple_height = OCEAN_RIPPLE_WAVES
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
    let vertical_velocity = WAVES
        .iter()
        .map(|wave| {
            let wave_number = std::f64::consts::TAU / wave.wavelength_meters;
            let phase = wave_number
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + wave.speed_meters_per_second * sim_time);
            wave.amplitude_meters
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
    let ripple_velocity = OCEAN_RIPPLE_WAVES
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
        GLOBAL_OCEAN_STORM_INTENSITY, MAXIMUM_WAVE_HEIGHT_METERS,
        OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE, OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE, WAVES,
        geometry_amplitude_scale, global_wave_height_meters,
        global_wave_vertical_velocity_meters_per_second, maximum_wave_height_meters,
        shore_wave_weight, wave_height_stats,
    };

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
        assert!((maximum_wave_height_meters(1.0) - 52.6625).abs() < 1.0e-9);
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

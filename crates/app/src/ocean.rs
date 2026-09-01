use glam::DVec3;

use crate::planet::PLANET_RADIUS_METERS;

const OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE: f64 = 4.0;
const OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE: f64 = 25.0;
pub const MAXIMUM_WAVE_HEIGHT_METERS: f64 = 24.0;

#[derive(Clone, Copy)]
struct GerstnerWave {
    direction: DVec3,
    wavelength_meters: f64,
    amplitude_meters: f64,
    speed_meters_per_second: f64,
}

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

#[cfg(test)]
mod tests {
    use super::{
        MAXIMUM_WAVE_HEIGHT_METERS, OCEAN_CALM_GEOMETRY_AMPLITUDE_SCALE,
        OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE, WAVES, geometry_amplitude_scale,
        maximum_wave_height_meters, wave_height_stats,
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
}

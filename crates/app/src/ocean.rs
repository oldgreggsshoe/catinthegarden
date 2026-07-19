use glam::DVec3;

use crate::planet::PLANET_RADIUS_METERS;

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
        wavelength_meters: 900.0,
        amplitude_meters: 0.375,
        speed_meters_per_second: 4.0,
    },
    GerstnerWave {
        direction: DVec3::new(-0.3, 0.4, 0.85),
        wavelength_meters: 420.0,
        amplitude_meters: 0.2125,
        speed_meters_per_second: 5.0,
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

pub fn wave_height_stats(sim_time: f64) -> WaveHeightStats {
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for y in -2..=2 {
        for z in -2..=2 {
            let direction =
                (DVec3::X + DVec3::Y * f64::from(y) * 0.002 + DVec3::Z * f64::from(z) * 0.002)
                    .normalize();
            let height = wave_height_meters(direction, sim_time);
            minimum = minimum.min(height);
            maximum = maximum.max(height);
        }
    }
    WaveHeightStats {
        minimum_meters: minimum as f32,
        maximum_meters: maximum as f32,
    }
}

fn wave_height_meters(direction: DVec3, sim_time: f64) -> f64 {
    WAVES
        .iter()
        .map(|wave| {
            let phase = std::f64::consts::TAU / wave.wavelength_meters
                * (direction.dot(wave.direction.normalize()) * PLANET_RADIUS_METERS
                    + wave.speed_meters_per_second * sim_time);
            wave.amplitude_meters * phase.sin()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::wave_height_stats;

    #[test]
    fn gerstner_wave_height_stats_are_non_zero_and_time_varying() {
        let first = wave_height_stats(0.0);
        let later = wave_height_stats(1.0);
        assert!(first.range_meters() > 0.4);
        assert!(later.range_meters() > 0.4);
        assert_ne!(first.minimum_meters, later.minimum_meters);
    }
}

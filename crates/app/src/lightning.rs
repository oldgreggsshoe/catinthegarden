//! Brief light inside storm fog, followed by sound after its travel time.

use glam::DVec3;

const SOUND_SPEED_METERS_PER_SECOND: f64 = 343.0;
const CLOUD_ALTITUDE_METERS: f64 = 3_000.0;

#[derive(Clone, Copy)]
struct Strike {
    position: DVec3,
    time: f64,
}

pub struct Lightning {
    next_time: f64,
    sequence: u32,
    active: Option<Strike>,
    thunder: Vec<(f64, f32, f32)>,
}

impl Lightning {
    pub fn new() -> Self {
        Self {
            next_time: f64::NAN,
            sequence: 0,
            active: None,
            thunder: Vec::new(),
        }
    }

    /// `centre` is fixed to the planet for a forced storm; natural storms
    /// choose a nearby location each time. Returns thunder gain and pan when
    /// the delayed sound reaches the eye.
    pub fn update(
        &mut self,
        time: f64,
        strength: f32,
        centre: DVec3,
        eye: DVec3,
        right: DVec3,
    ) -> Option<(f32, f32)> {
        if !self.next_time.is_finite() || time < self.next_time - 60.0 {
            self.next_time = time + 5.0;
        }
        if strength >= 0.65 && time >= self.next_time {
            let strike = Strike {
                position: strike_position(centre, self.sequence),
                time,
            };
            let offset = strike.position - eye;
            let distance = offset.length();
            let pan = (0.5 + 0.45 * offset.normalize().dot(right)).clamp(0.05, 0.95) as f32;
            let gain = (1.0 / (1.0 + distance / 8_000.0)) as f32;
            self.thunder
                .push((time + thunder_delay_seconds(distance), gain, pan));
            self.active = Some(strike);
            self.sequence = self.sequence.wrapping_add(1);
            self.next_time = time + 7.0 + 7.0 * random01(self.sequence);
        }
        if let Some(index) = self.thunder.iter().position(|event| time >= event.0) {
            let (_, gain, pan) = self.thunder.remove(index);
            Some((gain, pan))
        } else {
            None
        }
    }

    /// Direction and brightness of the cloud flash from the current eye.
    pub fn flash(&self, time: f64, eye: DVec3) -> [f32; 4] {
        let Some(strike) = self.active else {
            return [0.0; 4];
        };
        let age = time - strike.time;
        if !(0.0..0.4).contains(&age) {
            return [0.0; 4];
        }
        let pulse = |start: f64, length: f64| {
            let t = ((age - start) / length).clamp(0.0, 1.0);
            if age < start || age > start + length {
                0.0
            } else {
                (1.0 - t) * (1.0 - t)
            }
        };
        let brightness = (pulse(0.0, 0.11) + 0.65 * pulse(0.17, 0.18)).min(1.0) as f32;
        let direction = (strike.position - eye).normalize().as_vec3();
        [direction.x, direction.y, direction.z, brightness]
    }
}

fn thunder_delay_seconds(distance_meters: f64) -> f64 {
    distance_meters / SOUND_SPEED_METERS_PER_SECOND
}

fn random01(index: u32) -> f64 {
    let mut x = index.wrapping_add(0x9e37_79b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    f64::from(x) / f64::from(u32::MAX)
}

fn strike_position(centre: DVec3, index: u32) -> DVec3 {
    let up = centre.normalize();
    let east = up
        .cross(if up.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y })
        .normalize();
    let north = up.cross(east);
    let angle = std::f64::consts::TAU * random01(index);
    let radius = 2_000.0 * random01(index.wrapping_add(17)).sqrt();
    up * (crate::planet::planet_radius_meters() + CLOUD_ALTITUDE_METERS)
        + radius * (east * angle.cos() + north * angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thunder_waits_for_sound_to_cross_the_air() {
        assert!((thunder_delay_seconds(343.0) - 1.0).abs() < 1.0e-9);
        assert!((thunder_delay_seconds(3_430.0) - 10.0).abs() < 1.0e-9);
        let mut storm = Lightning::new();
        let centre = DVec3::X * crate::planet::planet_radius_meters();
        assert!(storm.update(0.0, 1.0, centre, centre, DVec3::Y).is_none());
        assert!(storm.update(5.0, 1.0, centre, centre, DVec3::Y).is_none());
        assert!(storm.flash(5.0, centre)[3] > 0.0);
        assert!(storm.update(5.5, 1.0, centre, centre, DVec3::Y).is_none());
        assert!(storm.update(30.0, 0.0, centre, centre, DVec3::Y).is_some());
    }

    #[test]
    fn strikes_stay_in_the_local_cloud_over_any_planet_direction() {
        for centre in [DVec3::X, DVec3::Y, DVec3::new(0.2, -0.4, 0.9).normalize()] {
            for index in 0..100 {
                let strike = strike_position(centre, index);
                let up = centre.normalize();
                assert!(
                    (strike.dot(up) - crate::planet::planet_radius_meters() - 3_000.0).abs() < 0.01
                );
                assert!((strike - up * strike.dot(up)).length() <= 2_000.1);
            }
        }
    }
}

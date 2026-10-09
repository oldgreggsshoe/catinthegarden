//! Brief light inside storm fog, followed by sound after its travel time.

use glam::DVec3;

const SOUND_SPEED_METERS_PER_SECOND: f64 = 343.0;
const CLOUD_ALTITUDE_METERS: f64 = 3_000.0;
/// Strikes land between these horizontal distances from the storm centre,
/// spread evenly over the area between them. They used to stay within 2 km
/// of it, so every bolt was about 3 km away overhead and every thunder came
/// about 9-10 s after its flash -- about when the next flash (7-14 s apart)
/// went off, so the thunder seemed to come with the light. Now a close strike
/// cracks within a second or two and a far one rumbles in up to ~26 s later.
const STRIKE_MIN_RADIUS_METERS: f64 = 300.0;
const STRIKE_MAX_RADIUS_METERS: f64 = 9_000.0;
/// Thunder from this far away is heard at half the loudness of a strike
/// overhead: a close strike is far louder than a full storm's sea and wind,
/// a far roll about level with them.
const THUNDER_HALF_GAIN_METERS: f64 = 3_000.0;

#[derive(Clone, Copy)]
struct Strike {
    position: DVec3,
    time: f64,
}

pub struct Lightning {
    next_time: f64,
    sequence: u32,
    active: Option<Strike>,
    /// Arrival time, gain, pan and distance (m) of thunder still in the air.
    thunder: Vec<(f64, f32, f32, f32)>,
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
    /// choose a nearby location each time. Returns thunder gain, pan and the
    /// distance it came from when the delayed sound reaches the eye.
    pub fn update(
        &mut self,
        time: f64,
        strength: f32,
        centre: DVec3,
        eye: DVec3,
        right: DVec3,
    ) -> Option<(f32, f32, f32)> {
        if !self.next_time.is_finite() || time < self.next_time - 60.0 {
            self.next_time = time + 5.0;
        }
        if strength >= 0.65 && time >= self.next_time {
            let strike = Strike {
                position: strike_position(centre, self.sequence),
                time,
            };
            // Thunder starts with the sound from the nearest part of the bolt,
            // which runs from the cloud down to the ground below it.
            let offset = nearest_point_of_bolt(strike.position, eye) - eye;
            let distance = offset.length();
            let pan = (0.5 + 0.45 * offset.normalize_or_zero().dot(right)).clamp(0.05, 0.95) as f32;
            let gain = (1.0 / (1.0 + distance / THUNDER_HALF_GAIN_METERS)) as f32;
            self.thunder
                .push((time + thunder_delay_seconds(distance), gain, pan, distance as f32));
            self.active = Some(strike);
            self.sequence = self.sequence.wrapping_add(1);
            self.next_time = time + 7.0 + 7.0 * random01(self.sequence);
        }
        if let Some(index) = self.thunder.iter().position(|event| time >= event.0) {
            let (_, gain, pan, distance) = self.thunder.remove(index);
            Some((gain, pan, distance))
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

/// The point of a bolt (a vertical channel from sea level up to the cloud at
/// `top`) closest to `eye`.
fn nearest_point_of_bolt(top: DVec3, eye: DVec3) -> DVec3 {
    let up = top.normalize();
    let ground = up * crate::planet::planet_radius_meters();
    let along = (eye - ground).dot(up).clamp(0.0, (top - ground).length());
    ground + up * along
}

/// A full two-round integer hash (as in the spray shader). The single round
/// this replaced gave nearly evenly stepped values for consecutive indices, so
/// strike positions and intervals walked round in a pattern.
fn random01(index: u32) -> f64 {
    let mut x = index.wrapping_add(0x9e37_79b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    f64::from(x) / f64::from(u32::MAX)
}

fn strike_position(centre: DVec3, index: u32) -> DVec3 {
    let up = centre.normalize();
    let east = up
        .cross(if up.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y })
        .normalize();
    let north = up.cross(east);
    let angle = std::f64::consts::TAU * random01(index);
    let (inner, outer) = (STRIKE_MIN_RADIUS_METERS, STRIKE_MAX_RADIUS_METERS);
    let radius = (inner * inner + (outer * outer - inner * inner) * random01(index.wrapping_add(17))).sqrt();
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
        assert!(storm.update(60.0, 0.0, centre, centre, DVec3::Y).is_some());
    }

    /// Standing under the storm, some thunder comes within a couple of seconds
    /// of its flash and some long after, so it can be told from the next one.
    #[test]
    fn thunder_delays_range_from_near_to_far_strikes() {
        let eye = DVec3::X * crate::planet::planet_radius_meters();
        let delays: Vec<f64> = (0..200)
            .map(|index| {
                let bolt = nearest_point_of_bolt(strike_position(DVec3::X, index), eye);
                thunder_delay_seconds((bolt - eye).length())
            })
            .collect();
        let shortest = delays.iter().cloned().fold(f64::INFINITY, f64::min);
        let longest = delays.iter().cloned().fold(0.0, f64::max);
        assert!(shortest < 3.0, "{shortest}");
        assert!(longest > 20.0, "{longest}");
        // From the ground the nearest point is the bolt's foot, not the cloud.
        let top = strike_position(DVec3::X, 3);
        let foot = top.normalize() * crate::planet::planet_radius_meters();
        assert!((nearest_point_of_bolt(top, eye) - foot).length() < 1.0e-6);
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
                let horizontal = (strike - up * strike.dot(up)).length();
                assert!((299.9..=9_000.1).contains(&horizontal), "{horizontal}");
            }
        }
    }
}

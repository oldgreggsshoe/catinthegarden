//! Lightweight test vehicle: bicycle steering, gravity and critically damped
//! suspension. No road adhesion, walking bob, tyre simulation or GPU work.
pub const EYE_HEIGHT: f64 = 1.4;
pub const WHEELBASE: f64 = 2.8;
pub const TRACK: f64 = 1.6;
pub const STEP: f64 = 1.0 / 120.0;
const GRAVITY: f64 = 9.80665;
const MAX_LATERAL_ACCELERATION: f64 = 6.5;
/// Placeholder numbers for the magic motor: its substance is not worked out
/// yet, so it is just a charge that full throttle spends in `MAGIC_FULL_BURN_SECONDS`
/// and a parked car slowly regains over `MAGIC_RECHARGE_SECONDS`.
const MAGIC_ACCELERATION: f64 = 14.0;
const MAGIC_TOP_SPEED: f64 = 45.0;
const MAGIC_FULL_BURN_SECONDS: f64 = 90.0;
const MAGIC_RECHARGE_SECONDS: f64 = 120.0;

/// Relative mouse motion acts as a bounded virtual steering wheel. Keyboard
/// steering overrides/recentres it; free-look never changes wheel position.
#[derive(Default)]
pub struct MouseSteering {
    value: f64,
    filtered: f64,
    pub free_look: bool,
}
impl MouseSteering {
    pub fn motion(&mut self, pixels: f64) {
        if !self.free_look && pixels.is_finite() {
            self.value = (self.value + pixels / 300.0).clamp(-1.0, 1.0);
        }
    }
    /// Called at the fixed120Hz physics step, never once per render frame.
    pub fn input(&mut self, keyboard: f64) -> f64 {
        if keyboard != 0.0 {
            self.value = 0.0;
            self.filtered = 0.0;
            keyboard
        } else {
            // Fine corrections near centre without a dead zone; retain full
            // lock for larger motions. Ease discrete device counts rather
            // than snapping the tyres to each tiny input target.
            let target = self.value * (0.5 + 0.5 * self.value * self.value);
            self.filtered += (target - self.filtered) * (1.0 - (-STEP / 0.015).exp());
            self.filtered
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Car {
    pub speed: f64,
    pub vertical_speed: f64,
    pub pitch: f64,
    pub roll: f64,
    pub grounded: bool,
    steering: f64,
    /// The magic motor is switched in (otherwise the plain cart drive).
    pub magic: bool,
    /// Share of the magic charge spent, 0 full to 1 empty.
    pub spent: f64,
}
impl Car {
    /// Magic charge left, 0..=1.
    pub fn charge(&self) -> f64 {
        (1.0 - self.spent).clamp(0.0, 1.0)
    }
    fn magic_running(&self) -> bool {
        self.magic && self.spent < 1.0
    }

    fn steering_lock(&self) -> f64 {
        (MAX_LATERAL_ACCELERATION * WHEELBASE / self.speed.powi(2).max(1.0))
            .atan()
            .min(0.5)
    }

    /// Actual wheel position relative to the available lock at this speed.
    /// Positive is right, matching the mouse/keyboard steering convention.
    pub fn steering_fraction(&self) -> f64 {
        (self.steering / self.steering_lock()).clamp(-1.0, 1.0)
    }

    /// W accelerates, S brakes then reverses; release coasts, Space brakes.
    /// Steering slew and speed-dependent lock bound lateral acceleration.
    pub fn travel(&mut self, throttle: f64, steer: f64, brake: bool, scale: f64) -> (f64, f64) {
        // Auto-hold at rest: the parked test car must not roll away while
        // the player is waiting for streaming or looking around. Release on
        // deliberate throttle; a moving car still coasts normally.
        let brake = brake || (throttle.abs() < 1e-6 && self.speed.abs() < 0.05);
        // Scale the whole usable input range with speed, not just the final
        // curvature. Otherwise tiny highway inputs already hit the safety
        // clamp and all larger inputs steer identically.
        let lock = self.steering_lock();
        // Progressively soften small corrections from36 to108km/h without
        // reducing full lock/grip or delaying a correction with more filtering.
        let speed_blend = ((self.speed.abs() - 10.0) / 20.0).clamp(0.0, 1.0);
        let centre_gain = 1.0 - 0.7 * speed_blend * speed_blend * (3.0 - 2.0 * speed_blend);
        let input = steer.clamp(-1.0, 1.0);
        let target = input * (centre_gain + (1.0 - centre_gain) * input * input) * lock;
        self.steering += (target - self.steering).clamp(-8.0 * STEP, 8.0 * STEP);
        let magic = self.magic_running();
        let acceleration = if throttle * self.speed < -0.1 {
            8.0
        } else if magic {
            MAGIC_ACCELERATION
        } else {
            6.0
        };
        if self.magic {
            if throttle.abs() > 1e-6 {
                self.spent = (self.spent + throttle.abs().min(1.0) * STEP / MAGIC_FULL_BURN_SECONDS).min(1.0);
            } else if self.speed.abs() < 0.5 {
                self.spent = (self.spent - STEP / MAGIC_RECHARGE_SECONDS).max(0.0);
            }
        }
        if self.grounded {
            self.speed +=
                (throttle.clamp(-1.0, 1.0) * acceleration - GRAVITY * self.pitch.sin()) * STEP;
        }
        let drag = (if brake { 12.0 } else { 0.35 }) + 0.0015 * self.speed * self.speed;
        self.speed = self.speed.signum() * (self.speed.abs() - drag * STEP).max(0.0);
        let maximum = if magic { MAGIC_TOP_SPEED } else { 30.0 } * scale.clamp(0.1, 2.0);
        self.speed = self.speed.clamp(-8.0, maximum);
        let curvature = (self.steering.tan() / WHEELBASE).clamp(
            -MAX_LATERAL_ACCELERATION / self.speed.powi(2).max(1.0),
            MAX_LATERAL_ACCELERATION / self.speed.powi(2).max(1.0),
        );
        let distance = self.speed * STEP;
        (
            distance,
            if self.grounded {
                distance * curvature
            } else {
                0.0
            },
        )
    }

    /// Heights at four wheels: front-left, front-right, rear-left, rear-right.
    /// Outside suspension travel the vehicle falls ballistically, rather than
    /// being glued to the sampled ground. Contact has a hard compression stop.
    pub fn support(&mut self, eye: f64, heights: [f64; 4]) -> f64 {
        let ground = heights.iter().sum::<f64>() * 0.25;
        let target = ground + EYE_HEIGHT;
        let error = target - eye;
        let pitch =
            ((heights[0] + heights[1] - heights[2] - heights[3]) / (2.0 * WHEELBASE)).atan();
        let ground_speed = (self.speed * pitch.tan()).clamp(-20.0, 20.0);
        self.grounded = error > -0.6;
        let spring = if self.grounded {
            (GRAVITY + 100.0 * error - 20.0 * (self.vertical_speed - ground_speed)).max(0.0)
        } else {
            0.0
        };
        self.vertical_speed += (spring - GRAVITY) * STEP;
        let mut height = eye + self.vertical_speed * STEP;
        let stop = target - 0.35;
        if height < stop {
            height = stop;
            self.vertical_speed = self.vertical_speed.max(ground_speed);
        }
        if self.grounded {
            let roll = ((heights[0] + heights[2] - heights[1] - heights[3]) / (2.0 * TRACK)).atan();
            // Attitude filters wheel-scale roughness; never an artificial bob.
            let blend = 1.0 - (-4.0 * STEP).exp();
            self.pitch += (pitch.clamp(-0.65, 0.65) - self.pitch) * blend;
            self.roll += (roll.clamp(-0.45, 0.45) - self.roll) * blend;
        }
        height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mouse_wheel_is_bounded_frame_independent_and_keyboard_can_override() {
        let mut a = MouseSteering::default();
        let mut b = MouseSteering::default();
        a.motion(150.0);
        for _ in 0..15 {
            b.motion(10.0);
        }
        assert!((a.input(0.0) - b.input(0.0)).abs() < 1e-12);
        for _ in 0..120 {
            a.input(0.0);
        }
        assert!((a.input(0.0) - 0.3125).abs() < 1e-6);
        a.free_look = true;
        a.motion(-300.0);
        assert_eq!(a.value, 0.5);
        a.free_look = false;
        a.motion(900.0);
        for _ in 0..240 {
            a.input(0.0);
        }
        assert!((a.input(0.0) - 1.0).abs() < 1e-10);
        assert_eq!(a.input(-1.0), -1.0);
        assert_eq!(a.input(0.0), 0.0);
        a.motion(-900.0);
        for _ in 0..240 {
            a.input(0.0);
        }
        assert!((a.input(0.0) + 1.0).abs() < 1e-10);
    }
    #[test]
    fn tiny_mouse_corrections_ramp_smoothly_through_the_vehicle_controller() {
        let mut mouse = MouseSteering::default();
        let mut car = Car {
            grounded: true,
            ..Car::default()
        };
        mouse.motion(1.0);
        let mut rates = Vec::new();
        for _ in 0..120 {
            car.speed = 10.0;
            let (_, yaw) = car.travel(0.0, mouse.input(0.0), false, 1.0);
            rates.push(yaw / STEP);
        }
        let settled = rates[119];
        assert!(
            rates[0] > 0.0 && rates[0] < settled * 0.6,
            "one mouse count snaps immediately: first={} settled={settled}",
            rates[0]
        );
        assert!(
            settled < 0.002,
            "centre correction is too coarse: {settled}"
        );
        assert!(rates.windows(2).all(|v| v[1] >= v[0]));
        mouse.motion(-2.0);
        car.speed = 10.0;
        let (_, yaw) = car.travel(0.0, mouse.input(0.0), false, 1.0);
        assert!(yaw > 0.0, "opposite correction must not snap across centre");
        for _ in 0..120 {
            mouse.input(0.0);
        }
        assert!(mouse.input(0.0) < 0.0);
    }
    #[test]
    fn highway_steering_keeps_small_inputs_proportional_instead_of_saturating() {
        let turn = |input| {
            let mut car = Car {
                grounded: true,
                ..Car::default()
            };
            let mut rate = 0.0;
            for _ in 0..120 {
                car.speed = 30.0;
                rate = car.travel(0.0, input, false, 1.0).1 / STEP;
            }
            rate
        };
        let small = turn(0.05);
        let larger = turn(0.10);
        assert!(
            larger > small * 1.9 && larger < small * 2.1,
            "small highway inputs saturated: {small} / {larger}"
        );
        assert!(small < 0.012, "excessive highway sensitivity: {small}");
    }
    #[test]
    fn opposite_mouse_corrections_reverse_the_turn_within_a_tenth_of_a_second() {
        for speed in [2.0, 10.0, 30.0] {
            let mut mouse = MouseSteering::default();
            let mut car = Car {
                grounded: true,
                ..Car::default()
            };
            mouse.motion(300.0);
            for _ in 0..120 {
                car.speed = speed;
                car.travel(0.0, mouse.input(0.0), false, 1.0);
            }
            mouse.motion(-600.0);
            let mut yaw = 0.0;
            for _ in 0..12 {
                car.speed = speed;
                yaw = car.travel(0.0, mouse.input(0.0), false, 1.0).1;
            }
            assert!(yaw < 0.0, "countersteering still delayed at {speed} m/s");
        }
    }
    #[test]
    fn cornering_grip_reaches_50m_and_100m_bends_without_unlimited_highway_turns() {
        let radius_at_lock = |speed| {
            let mut car = Car {
                grounded: true,
                ..Car::default()
            };
            let mut radius = 0.0;
            for _ in 0..120 {
                car.speed = speed;
                let (distance, yaw) = car.travel(0.0, 1.0, false, 1.0);
                radius = distance / yaw;
            }
            radius
        };
        assert!(radius_at_lock(50.0 / 3.6) <= 50.0);
        assert!(
            radius_at_lock(90.0 / 3.6) <= 100.0,
            "90km/h cannot reach a100m bend: {}",
            radius_at_lock(90.0 / 3.6)
        );
        let highway = radius_at_lock(100.0 / 3.6);
        assert!(
            (118.0..120.0).contains(&highway),
            "100km/h radius {highway}"
        );
        assert!(highway > 50.0, "tight bends still need braking");
    }
    #[test]
    fn high_speed_small_inputs_are_gentler_without_losing_lock_or_fast_reversal() {
        let response = |speed, input| {
            let mut car = Car {
                grounded: true,
                ..Car::default()
            };
            for _ in 0..120 {
                car.speed = speed;
                car.travel(0.0, input, false, 1.0);
            }
            car.speed = speed;
            car.steering_fraction()
        };
        assert!((response(5.0, 0.1) - 0.1).abs() < 1e-12);
        assert!(
            response(30.0, 0.1) < response(5.0, 0.1) * 0.4,
            "high-speed near-centre input still too sensitive"
        );
        assert!((response(30.0, 1.0) - 1.0).abs() < 1e-12);
        assert!((response(30.0, -1.0) + 1.0).abs() < 1e-12);
        for speed in [10.0, 15.0, 20.0, 25.0, 30.0] {
            assert!((response(speed, -0.1) + response(speed, 0.1)).abs() < 1e-12);
        }
    }
    #[test]
    fn engine_acceleration_is_six_metres_per_second_squared_in_both_directions() {
        for throttle in [-1.0, 1.0] {
            let mut car = Car {
                grounded: true,
                ..Car::default()
            };
            car.travel(throttle, 0.0, false, 1.0);
            assert!(
                (car.speed - throttle * (6.0 - 0.35 - 0.0015 * (6.0 * STEP).powi(2)) * STEP).abs()
                    < 1e-12
            );
        }
    }
    #[test]
    fn steering_indicator_shows_the_actual_wheels_not_only_the_input_target() {
        let mut car = Car {
            speed: 10.0,
            grounded: true,
            ..Car::default()
        };
        assert_eq!(car.steering_fraction(), 0.0);
        car.travel(0.0, 1.0, false, 1.0);
        assert!(car.steering_fraction() > 0.0 && car.steering_fraction() < 1.0);
        for _ in 0..120 {
            car.travel(0.0, 1.0, false, 1.0);
        }
        assert!((0.99..=1.0).contains(&car.steering_fraction()));
        car.travel(0.0, -1.0, false, 1.0);
        assert!(
            car.steering_fraction() > 0.0,
            "show wheels while reversing, not the target"
        );
        for _ in 0..120 {
            car.travel(0.0, -1.0, false, 1.0);
        }
        assert!((-1.0..=-0.99).contains(&car.steering_fraction()));
        car.speed = 60.0;
        assert_eq!(
            car.steering_fraction(),
            -1.0,
            "speed changes cannot overflow the bar"
        );
    }
    #[test]
    fn flat_road_is_stationary_and_not_bobbing() {
        let mut car = Car::default();
        let mut h = 10.0 + EYE_HEIGHT;
        for _ in 0..1200 {
            h = car.support(h, [10.0; 4]);
        }
        assert!((h - 11.4).abs() < 1e-10);
        assert!(car.vertical_speed.abs() < 1e-10);
        assert_eq!((car.pitch, car.roll), (0.0, 0.0));
    }
    #[test]
    fn leaving_a_ledge_falls_under_gravity_without_snapping_down() {
        let mut car = Car::default();
        let mut h = 20.0;
        for _ in 0..120 {
            h = car.support(h, [0.0; 4]);
        }
        assert!((h - (20.0 - GRAVITY * 0.5)).abs() < 0.05);
        for _ in 0..1200 {
            h = car.support(h, [0.0; 4]);
        }
        assert!((h - EYE_HEIGHT).abs() < 1e-6);
    }
    #[test]
    fn suspension_attenuates_roughness_and_smooths_grade_changes() {
        let mut car = Car::default();
        let mut h = EYE_HEIGHT;
        let mut maximum = 0.0_f64;
        for i in 0..1200 {
            let bump = 0.1 * (i as f64 * STEP * 25.0).sin();
            h = car.support(h, [bump; 4]);
            maximum = maximum.max((h - EYE_HEIGHT).abs());
        }
        assert!(maximum < 0.04, "suspension motion {maximum}");
        car.support(h, [0.35, 0.35, -0.35, -0.35]);
        assert!(car.pitch > 0.0 && car.pitch < 0.02);
    }
    #[test]
    fn suspension_tracks_a_sustained_25_percent_grade_without_compression_snapping() {
        let mut car = Car {
            speed: 20.0,
            grounded: true,
            ..Car::default()
        };
        let mut h = EYE_HEIGHT;
        for i in 1..=600 {
            let ground = i as f64 * STEP * 20.0 * 0.25;
            let half_rise = WHEELBASE * 0.25 * 0.5;
            h = car.support(
                h,
                [
                    ground + half_rise,
                    ground + half_rise,
                    ground - half_rise,
                    ground - half_rise,
                ],
            );
            if i > 120 {
                assert!((h - ground - EYE_HEIGHT).abs() < 0.06);
            }
        }
    }
    #[test]
    fn steering_and_reverse_have_finite_vehicle_limits() {
        let mut car = Car {
            grounded: true,
            ..Car::default()
        };
        for _ in 0..2400 {
            let (d, yaw) = car.travel(1.0, 1.0, false, 1.0);
            assert!(yaw.abs() <= 0.02 && d >= 0.0);
            assert!(car.speed <= 30.0);
            assert!(yaw.abs() / STEP * car.speed <= MAX_LATERAL_ACCELERATION + 0.00001);
        }
        for _ in 0..2400 {
            car.travel(-1.0, 0.0, false, 1.0);
        }
        assert!(car.speed < 0.0 && car.speed >= -8.0);
        for _ in 0..240 {
            car.travel(0.0, 0.0, true, 1.0);
        }
        assert_eq!(car.speed, 0.0);
        car.pitch = 0.25_f64.atan();
        for _ in 0..1200 {
            car.travel(0.0, 0.0, false, 1.0);
        }
        assert_eq!(car.speed, 0.0, "parked auto-hold on a 25% grade");
    }

    #[test]
    fn magic_motor_is_faster_spends_charge_and_falls_back_when_empty() {
        let run = |magic: bool, seconds: f64, spent: f64| {
            let mut car = Car { grounded: true, magic, spent, ..Car::default() };
            for _ in 0..(seconds / STEP) as usize {
                car.travel(1.0, 0.0, false, 1.0);
            }
            car
        };
        let plain = run(false, 20.0, 0.0);
        let magic = run(true, 20.0, 0.0);
        assert!(magic.speed > plain.speed + 5.0, "{} vs {}", magic.speed, plain.speed);
        assert!(magic.charge() < 1.0 && magic.charge() > 0.7);
        let empty = run(true, 20.0, 1.0);
        assert!((empty.speed - plain.speed).abs() < 1e-9);
        let mut parked = Car { grounded: true, magic: true, spent: 0.5, ..Car::default() };
        for _ in 0..(10.0 / STEP) as usize {
            parked.travel(0.0, 0.0, false, 1.0);
        }
        assert!(parked.charge() > 0.5);
    }
}

//! Small, deterministic human-scale locomotion model for the surface camera.
//!
//! Horizontal terrain queries remain in `main.rs`, beside the streamed terrain
//! cache they use. This module owns only the medium-independent movement rules
//! and radial physics so they can be tested without a GPU.

pub const HUMAN_EYE_HEIGHT_METERS: f64 = 1.70;
pub const WALK_SPEED_METERS_PER_SECOND: f64 = 4.4704;
pub const SWIM_SPEED_METERS_PER_SECOND: f64 = 2.0;
pub const MAXIMUM_WALKABLE_SLOPE_DEGREES: f64 = 42.0;
pub const GRAVITY_METERS_PER_SECOND_SQUARED: f64 = 9.806_65;
pub const LAND_JUMP_SPEED_METERS_PER_SECOND: f64 = 5.2;
pub const WATER_UPWARD_IMPULSE_METERS_PER_SECOND: f64 = 2.5;
/// Diagnostic mode: follow the rendered water surface exactly, without
/// vertical inertia, buoyancy, gravity or jump impulses. Re-enable this when
/// returning to the physical swimming model.
pub const WATER_BOBBING_ENABLED: bool = true;
// A one-metre diagnostic margin leaves room for f32 phase quantisation and
// interpolation across the nearest rendered triangle while remaining close to
// the water surface.
pub const FIXED_WATER_EYE_CLEARANCE_METERS: f64 = 1.0;
/// Lowest supported eye altitude relative to sea level. Storm troughs can be
/// tens of metres below sea level; using a sea-level floor pins the camera
/// there and falsely reports enormous water clearance.
pub const PLANET_CORE_CLEARANCE_METERS: f64 = -100.0;

const EFFECTIVE_BODY_HEIGHT_METERS: f64 = HUMAN_EYE_HEIGHT_METERS;
const EFFECTIVE_BODY_DENSITY_RELATIVE_TO_WATER: f64 = 0.85;
const WATER_VERTICAL_DRAG_PER_SECOND: f64 = 3.0;
// Archimedes alone is too weak to keep a human-scale eye above a rapidly
// rising storm crest. This bounded spring draws the submerged body toward the
// same still-water equilibrium without pinning it to the animated surface.
const WATER_BUOYANCY_RESTORING_ACCELERATION_PER_METER: f64 = 6.0;
const WATER_BUOYANCY_MAX_RESTORING_ACCELERATION: f64 = 24.0;
/// The eye may ride down into a crest, but never through it.
///
/// This ocean's crests accelerate downward at close to g, so a genuinely
/// buoyant swimmer is overtaken by them: even a restoring term twenty times
/// stronger leaves the eye submerged a quarter of the time. Bobbing therefore
/// needs a floor rather than a stiffer spring. Buoyancy still drives the
/// motion, so the eye rises, falls and lags with the sea; it just cannot end a
/// substep below the water it is swimming on.
pub const MINIMUM_SWIMMING_EYE_CLEARANCE_METERS: f64 = 0.06;
const MAXIMUM_PHYSICS_STEP_SECONDS: f64 = 1.0 / 120.0;
pub const GROUND_CONTACT_EPSILON_METERS: f64 = 0.02;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfacePhysicsState {
    pub vertical_velocity_meters_per_second: f64,
    pub grounded: bool,
    pub in_water: bool,
}

pub fn movement_speed_meters_per_second(in_open_ocean: bool, speed_scale: f64) -> f64 {
    let base = if in_open_ocean {
        SWIM_SPEED_METERS_PER_SECOND
    } else {
        WALK_SPEED_METERS_PER_SECOND
    };
    base * speed_scale
}

pub fn fixed_water_eye_altitude_meters(water_height_meters: f64) -> f64 {
    water_height_meters + FIXED_WATER_EYE_CLEARANCE_METERS
}

pub fn walkable_step(
    current_ground_height_meters: f64,
    candidate_ground_height_meters: f64,
    horizontal_distance_meters: f64,
    candidate_is_open_ocean: bool,
) -> bool {
    if candidate_is_open_ocean || candidate_ground_height_meters <= current_ground_height_meters {
        return true;
    }
    if horizontal_distance_meters <= f64::EPSILON {
        return false;
    }
    let rise = candidate_ground_height_meters - current_ground_height_meters;
    rise.atan2(horizontal_distance_meters).to_degrees() <= MAXIMUM_WALKABLE_SLOPE_DEGREES
}

impl SurfacePhysicsState {
    pub fn settle_on_land(&mut self) {
        self.vertical_velocity_meters_per_second = 0.0;
        self.grounded = true;
        self.in_water = false;
    }

    pub fn settle_in_water(&mut self) {
        self.vertical_velocity_meters_per_second = 0.0;
        self.grounded = false;
        self.in_water = true;
    }

    /// Advances the eye altitude in the local radial direction.
    ///
    /// `water_surface` is absent over land. When present, buoyancy comes from
    /// the fraction of the simple human-height body below that surface, while
    /// drag follows the water's vertical velocity. The camera is therefore not
    /// pinned to a wave: large moving waves lift it, inertia lets it bob, and
    /// gravity returns it naturally.
    pub fn advance_vertical(
        &mut self,
        mut eye_altitude_meters: f64,
        terrain_height_meters: f64,
        water_surface: Option<(f64, f64)>,
        jump_requested: bool,
        delta_seconds: f64,
    ) -> f64 {
        let ground_eye_height = terrain_height_meters + HUMAN_EYE_HEIGHT_METERS;
        self.update_medium(eye_altitude_meters, water_surface);
        if jump_requested {
            if self.in_water {
                self.vertical_velocity_meters_per_second += WATER_UPWARD_IMPULSE_METERS_PER_SECOND;
                self.grounded = false;
            } else if self.grounded {
                self.vertical_velocity_meters_per_second = LAND_JUMP_SPEED_METERS_PER_SECOND;
                self.grounded = false;
            }
        }

        let mut remaining = delta_seconds.max(0.0);
        while remaining > 0.0 {
            let step = remaining.min(MAXIMUM_PHYSICS_STEP_SECONDS);
            let mut acceleration = -GRAVITY_METERS_PER_SECOND_SQUARED;
            if let Some((water_height, water_vertical_velocity)) = water_surface {
                let submerged_fraction = ((water_height
                    - (eye_altitude_meters - EFFECTIVE_BODY_HEIGHT_METERS))
                    / EFFECTIVE_BODY_HEIGHT_METERS)
                    .clamp(0.0, 1.0);
                if submerged_fraction > 0.0 {
                    acceleration += GRAVITY_METERS_PER_SECOND_SQUARED * submerged_fraction
                        / EFFECTIVE_BODY_DENSITY_RELATIVE_TO_WATER;
                    acceleration += WATER_VERTICAL_DRAG_PER_SECOND
                        * submerged_fraction
                        * (water_vertical_velocity - self.vertical_velocity_meters_per_second);
                    let resting_error = water_height + equilibrium_eye_height_above_water_meters()
                        - eye_altitude_meters;
                    acceleration +=
                        (resting_error * WATER_BUOYANCY_RESTORING_ACCELERATION_PER_METER).clamp(
                            -WATER_BUOYANCY_MAX_RESTORING_ACCELERATION,
                            WATER_BUOYANCY_MAX_RESTORING_ACCELERATION,
                        ) * submerged_fraction;
                }
            }
            self.vertical_velocity_meters_per_second += acceleration * step;
            eye_altitude_meters += self.vertical_velocity_meters_per_second * step;

            if eye_altitude_meters <= ground_eye_height + GROUND_CONTACT_EPSILON_METERS
                && self.vertical_velocity_meters_per_second <= 0.0
                && water_surface.is_none()
            {
                eye_altitude_meters = ground_eye_height;
                self.vertical_velocity_meters_per_second = 0.0;
                self.grounded = true;
            } else {
                self.grounded = false;
            }
            if eye_altitude_meters <= PLANET_CORE_CLEARANCE_METERS {
                eye_altitude_meters = PLANET_CORE_CLEARANCE_METERS;
                self.vertical_velocity_meters_per_second =
                    self.vertical_velocity_meters_per_second.max(0.0);
                self.grounded = false;
            }
            if let Some((water_height, water_vertical_velocity)) = water_surface {
                // The floor described on MINIMUM_SWIMMING_EYE_CLEARANCE_METERS.
                // A crest that overtakes the eye carries it up rather than
                // closing over it, so the eye keeps the surface's own upward
                // speed instead of being left behind by it.
                let floor = water_height + MINIMUM_SWIMMING_EYE_CLEARANCE_METERS;
                if eye_altitude_meters < floor {
                    eye_altitude_meters = floor;
                    self.vertical_velocity_meters_per_second = self
                        .vertical_velocity_meters_per_second
                        .max(water_vertical_velocity);
                }
            }
            self.update_medium(eye_altitude_meters, water_surface);
            remaining -= step;
        }

        if eye_altitude_meters < ground_eye_height && water_surface.is_none() {
            eye_altitude_meters = ground_eye_height;
            self.vertical_velocity_meters_per_second =
                self.vertical_velocity_meters_per_second.max(0.0);
            self.grounded = true;
        }
        if eye_altitude_meters <= PLANET_CORE_CLEARANCE_METERS {
            eye_altitude_meters = PLANET_CORE_CLEARANCE_METERS;
            self.vertical_velocity_meters_per_second =
                self.vertical_velocity_meters_per_second.max(0.0);
            self.grounded = false;
        }
        eye_altitude_meters
    }

    fn update_medium(&mut self, eye_altitude_meters: f64, water_surface: Option<(f64, f64)>) {
        self.in_water = water_surface.is_some_and(|(water_height, _)| {
            eye_altitude_meters - EFFECTIVE_BODY_HEIGHT_METERS < water_height
        });
        if self.grounded
            && eye_altitude_meters
                > water_surface.map_or(f64::NEG_INFINITY, |(height, _)| height)
                    + EFFECTIVE_BODY_HEIGHT_METERS
                    + GROUND_CONTACT_EPSILON_METERS
        {
            self.in_water = false;
        }
    }
}

pub fn equilibrium_eye_height_above_water_meters() -> f64 {
    EFFECTIVE_BODY_HEIGHT_METERS * (1.0 - EFFECTIVE_BODY_DENSITY_RELATIVE_TO_WATER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_rejects_steep_uphill_but_allows_descent_and_ocean_entry() {
        assert!(walkable_step(10.0, 10.5, 1.0, false));
        assert!(!walkable_step(10.0, 11.0, 1.0, false));
        assert!(walkable_step(11.0, 10.0, 1.0, false));
        assert!(walkable_step(-1.0, 2.0, 1.0, true));
    }

    #[test]
    fn walking_and_swimming_share_the_external_speed_scale() {
        assert_eq!(movement_speed_meters_per_second(false, 2.0), 8.9408);
        assert_eq!(movement_speed_meters_per_second(true, 2.0), 4.0);
    }

    #[test]
    fn land_jump_rises_then_gravity_returns_the_eye_to_the_ground() {
        let mut state = SurfacePhysicsState::default();
        state.settle_on_land();
        let ground_eye = HUMAN_EYE_HEIGHT_METERS;
        let mut eye = state.advance_vertical(ground_eye, 0.0, None, true, 1.0 / 120.0);
        assert!(eye > ground_eye);
        assert!(state.vertical_velocity_meters_per_second > 0.0);
        for _ in 0..480 {
            eye = state.advance_vertical(eye, 0.0, None, false, 1.0 / 120.0);
        }
        assert!((eye - ground_eye).abs() < 1.0e-9);
        assert!(state.grounded);
    }

    #[test]
    fn buoyancy_converges_without_pinning_the_eye_to_the_wave() {
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let equilibrium = equilibrium_eye_height_above_water_meters();
        let mut eye = equilibrium;
        eye = state.advance_vertical(eye, -100.0, Some((2.0, 1.0)), false, 1.0 / 60.0);
        assert!(
            eye < 2.0 + equilibrium,
            "the camera must not snap to the wave"
        );
        assert!(state.vertical_velocity_meters_per_second > 0.0);
        for _ in 0..1_200 {
            eye = state.advance_vertical(eye, -100.0, Some((2.0, 0.0)), false, 1.0 / 120.0);
        }
        assert!((eye - (2.0 + equilibrium)).abs() < 0.05, "eye={eye}");
    }

    #[test]
    fn buoyancy_moves_the_camera_with_a_rising_and_falling_wave() {
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let equilibrium = equilibrium_eye_height_above_water_meters();
        let mut eye = equilibrium;
        let mut minimum = eye;
        let mut maximum = eye;
        for index in 0..(20 * 120) {
            let time = f64::from(index) / 120.0;
            let phase = std::f64::consts::TAU * time / 20.0;
            let water_height = 8.0 * phase.sin();
            let water_velocity = 8.0 * std::f64::consts::TAU / 20.0 * phase.cos();
            eye = state.advance_vertical(
                eye,
                -100.0,
                Some((water_height, water_velocity)),
                false,
                1.0 / 120.0,
            );
            minimum = minimum.min(eye);
            maximum = maximum.max(eye);
        }
        assert!(
            maximum - minimum > 1.0,
            "camera range={}",
            maximum - minimum
        );
    }

    #[test]
    fn water_jump_is_one_upward_impulse_then_physics_resumes() {
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let eye = equilibrium_eye_height_above_water_meters();
        let after_impulse = state.advance_vertical(eye, -100.0, Some((0.0, 0.0)), true, 0.0);
        assert_eq!(after_impulse, eye);
        assert_eq!(
            state.vertical_velocity_meters_per_second,
            WATER_UPWARD_IMPULSE_METERS_PER_SECOND
        );
        state.advance_vertical(after_impulse, -100.0, Some((0.0, 0.0)), false, 0.25);
        assert!(state.vertical_velocity_meters_per_second < WATER_UPWARD_IMPULSE_METERS_PER_SECOND);
    }

    #[test]
    fn a_swimmer_bobs_without_being_swallowed_by_a_storm_crest() {
        // The regression this exists for: with buoyancy alone the eye spent
        // 41% of a storm underwater, because crests here accelerate down at
        // close to g and simply overtake a floating body. Twenty times the
        // restoring force only got that to 23%, so the floor is the fix.
        use crate::ocean;
        let direction = glam::DVec3::new(0.836442275001636, 0.503727905284262, 0.215922481525239)
            .normalize();
        // Swept over real sea beds, not just abyssal depth. The bug this
        // caught: at 4000m the depth limiter is inert, so a test that only ran
        // there could not see the eye and the rendered surface drifting apart
        // in the couple of hundred metres of water an actual coast has.
        for depth in [80.0, 200.0, 1000.0, 4000.0] {
            swim_at_depth(direction, depth);
        }
    }

    fn swim_at_depth(direction: glam::DVec3, depth: f64) {
        use crate::ocean;
        let mut physics = SurfacePhysicsState::default();
        physics.settle_in_water();
        let mut eye = ocean::global_wave_height_meters(direction, 0.0, depth) + 0.255;
        let mut submerged = 0usize;
        let mut clearances = Vec::new();
        let mut time_seconds = 0.0;
        for _ in 0..3600 {
            let height = ocean::local_wave_height_meters(direction, time_seconds, depth);
            let velocity =
                ocean::local_wave_vertical_velocity_meters_per_second(direction, time_seconds, depth);
            eye = physics.advance_vertical(eye, -depth, Some((height, velocity)), false, 1.0 / 60.0);
            time_seconds += 1.0 / 60.0;
            let clearance = eye - height;
            if clearance < 0.0 {
                submerged += 1;
            }
            clearances.push(clearance);
            assert!(eye.is_finite());
        }
        assert_eq!(
            submerged, 0,
            "eye went under on {submerged} frames in {depth} m of water"
        );

        // It still has to be a float, not a rail: the eye must ride above the
        // surface some of the time rather than being pinned to the floor.
        let highest = clearances.iter().cloned().fold(f64::MIN, f64::max);
        assert!(
            highest > 0.3,
            "in {depth} m of water the eye never rose past {highest:.3} m of \
             clearance, so it is welded to the surface"
        );
        // And it must actually move with the sea rather than holding one height.
        let lowest = clearances.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            highest - lowest > 0.2,
            "clearance only varied {:.3} m; that is not bobbing",
            highest - lowest
        );
    }

    #[test]
    fn shallow_water_stays_swimming_instead_of_becoming_grounded() {
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let eye = state.advance_vertical(
            equilibrium_eye_height_above_water_meters(),
            -0.5,
            Some((0.0, 0.0)),
            false,
            1.0 / 60.0,
        );
        assert!(!state.grounded);
        assert!(state.in_water);
        assert!(eye >= -0.5);
    }

    #[test]
    fn fixed_water_test_height_is_a_small_offset_above_the_surface() {
        assert_eq!(fixed_water_eye_altitude_meters(-28.0), -27.0);
    }

    #[test]
    fn swimming_cannot_fall_below_the_underwater_safety_floor() {
        let mut state = SurfacePhysicsState {
            vertical_velocity_meters_per_second: -100.0,
            grounded: false,
            in_water: true,
        };
        let eye = state.advance_vertical(0.1, -1_000.0, Some((-100.0, 0.0)), false, 1.0);
        assert!(eye >= PLANET_CORE_CLEARANCE_METERS);
        assert!(state.vertical_velocity_meters_per_second >= 0.0);
    }
}

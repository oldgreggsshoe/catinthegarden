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
/// A submerged diver is neutrally buoyant, so releasing the stroke means
/// holding this depth rather than drifting anywhere. This is what kills the
/// residual vertical motion so "stop swimming" does not mean "coast": a 0.125s
/// time constant, brisk enough to feel like stopping and slow enough not to
/// snap.
const SUBMERGED_VERTICAL_DRAG_PER_SECOND: f64 = 8.0;
/// Eye clearance above the sea bed on a dive. Small enough to inspect the
/// bottom, large enough to absorb the disagreement between the bathymetry the
/// CPU samples here and the bed the renderer actually draws.
///
/// Read it through `swimming_bed_eye_altitude_meters`, never by adding it to a
/// bed height at a call site: the post-streaming resolve in `main.rs` holds the
/// same floor, and when it had its own arithmetic it held the eye 1.70m off the
/// bed while this module was clamping to 0.5m.
pub const SWIMMING_BED_EYE_CLEARANCE_METERS: f64 = 0.5;
/// A stroke shallower than this is not a dive. The crest guard below stays on
/// through it, so ordinary level swimming cannot be swallowed by a wave on the
/// strength of a look vector that is a few float ulps off horizontal.
const DELIBERATE_DESCENT_SPEED_METERS_PER_SECOND: f64 = 0.05;
/// How far under the waterline the floating model has fully given way to a
/// neutrally buoyant diver.
///
/// This was one body height, and a body height is far too deep: the most
/// interesting thing to hold station in front of is Snell's window, which is
/// directly overhead in the first metre or two, and at -1.0m the old fade left
/// enough buoyancy to float a stopped diver back out in about ten seconds.
/// Making it short keeps that band neutral while changing nothing about
/// floating, because the crest guard holds a non-diving eye above the waterline
/// where the fade is saturated at one regardless of how steep it is.
const NEUTRAL_BUOYANCY_DEPTH_METERS: f64 = 0.3;
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
    /// Latched underwater-swimming mode, entered by a deliberate dive. A wave
    /// trough briefly uncovering the eye must not restore surface buoyancy or
    /// the crest guard. An upward stroke through the surface exits this mode;
    /// `in_water` separately tracks actual body contact with water.
    pub submerged: bool,
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
        self.submerged = false;
    }

    pub fn settle_in_water(&mut self) {
        self.vertical_velocity_meters_per_second = 0.0;
        self.grounded = false;
        self.in_water = true;
        self.submerged = false;
    }

    /// Advances the eye altitude in the local radial direction.
    ///
    /// `water_surface` is absent over land. When present, buoyancy comes from
    /// the fraction of the simple human-height body below that surface, while
    /// drag follows the water's vertical velocity. The camera is therefore not
    /// pinned to a wave: large moving waves lift it, inertia lets it bob, and
    /// gravity returns it naturally.
    ///
    /// `swim_vertical_speed_meters_per_second` is the radial part of a
    /// swimmer's stroke, signed, already scaled to metres per second. It is
    /// applied kinematically, exactly as the tangential part of the same stroke
    /// is applied by the caller, so a dive does not have to out-accelerate
    /// buoyancy -- it simply moves.
    pub fn advance_vertical(
        &mut self,
        mut eye_altitude_meters: f64,
        terrain_height_meters: f64,
        water_surface: Option<(f64, f64)>,
        jump_requested: bool,
        swim_vertical_speed_meters_per_second: f64,
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
                    // Everything about floating is a *surface* device: the
                    // restoring spring exists to keep a bobbing eye with the sea
                    // it floats on, the drag references the surface's orbital
                    // velocity, and the buoyancy itself is what makes a body
                    // float rather than hold station. A metre under, the spring
                    // saturates its own 24 m/s^2 clamp and no stroke can beat
                    // it, and the surface's orbital velocity is not the water's
                    // velocity down there anyway. So the whole floating model
                    // fades out near the surface and stays off throughout a
                    // deliberate dive, even when a moving trough uncovers the
                    // eye. Swimming back up is how you return to floating.
                    let surface_authority = if self.submerged {
                        0.0
                    } else {
                        surface_authority(eye_altitude_meters, water_height)
                    };
                    acceleration += GRAVITY_METERS_PER_SECOND_SQUARED * submerged_fraction
                        / EFFECTIVE_BODY_DENSITY_RELATIVE_TO_WATER;
                    // Gravity and buoyancy net out together, so this is the one
                    // multiply that makes a deep diver weightless.
                    acceleration *= surface_authority;
                    let drag_per_second = WATER_VERTICAL_DRAG_PER_SECOND * surface_authority
                        + SUBMERGED_VERTICAL_DRAG_PER_SECOND * (1.0 - surface_authority);
                    let reference_velocity = water_vertical_velocity * surface_authority;
                    acceleration += drag_per_second
                        * submerged_fraction
                        * (reference_velocity - self.vertical_velocity_meters_per_second);
                    let resting_error = water_height + equilibrium_eye_height_above_water_meters()
                        - eye_altitude_meters;
                    acceleration +=
                        (resting_error * WATER_BUOYANCY_RESTORING_ACCELERATION_PER_METER).clamp(
                            -WATER_BUOYANCY_MAX_RESTORING_ACCELERATION,
                            WATER_BUOYANCY_MAX_RESTORING_ACCELERATION,
                        ) * submerged_fraction
                            * surface_authority;
                }
            }
            self.vertical_velocity_meters_per_second += acceleration * step;
            eye_altitude_meters += self.vertical_velocity_meters_per_second * step;
            if water_surface.is_some() {
                eye_altitude_meters += swim_vertical_speed_meters_per_second * step;
            }

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
            // The core clearance is the backstop for a runaway with no bed to
            // stop it. Over water there is always a bed, and it is the floor at
            // any depth -- a diver can reach the bottom of the deepest ocean.
            if water_surface.is_none() && eye_altitude_meters <= PLANET_CORE_CLEARANCE_METERS {
                eye_altitude_meters = PLANET_CORE_CLEARANCE_METERS;
                self.vertical_velocity_meters_per_second =
                    self.vertical_velocity_meters_per_second.max(0.0);
                self.grounded = false;
            }
            if let Some((water_height, water_vertical_velocity)) = water_surface {
                // Over water the ground clamps above are disabled, so a dive
                // needs its own bed. Where the bed is deeper than the core
                // clearance the clamp below wins instead, which is what caps a
                // dive in open ocean.
                let bed_floor = swimming_bed_eye_altitude_meters(terrain_height_meters);
                if eye_altitude_meters < bed_floor {
                    eye_altitude_meters = bed_floor;
                    self.vertical_velocity_meters_per_second =
                        self.vertical_velocity_meters_per_second.max(0.0);
                }
                // The floor described on MINIMUM_SWIMMING_EYE_CLEARANCE_METERS.
                // A crest that overtakes the eye carries it up rather than
                // closing over it, so the eye keeps the surface's own upward
                // speed instead of being left behind by it.
                //
                // It guards a swimmer *riding* the surface, so it is off for a
                // diver: on the way down it is what a dive has to get past, and
                // on the way up it would snap the last metre of an ascent to
                // the surface instead of letting the diver rise through it.
                let diving = swim_vertical_speed_meters_per_second
                    <= -DELIBERATE_DESCENT_SPEED_METERS_PER_SECOND;
                let floor = water_height + MINIMUM_SWIMMING_EYE_CLEARANCE_METERS;
                if !self.submerged && !diving && eye_altitude_meters < floor {
                    eye_altitude_meters = floor;
                    self.vertical_velocity_meters_per_second = self
                        .vertical_velocity_meters_per_second
                        .max(water_vertical_velocity);
                }
                self.update_submerged(
                    eye_altitude_meters,
                    water_height,
                    swim_vertical_speed_meters_per_second,
                );
            } else {
                self.submerged = false;
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
        if water_surface.is_none() && eye_altitude_meters <= PLANET_CORE_CLEARANCE_METERS {
            eye_altitude_meters = PLANET_CORE_CLEARANCE_METERS;
            self.vertical_velocity_meters_per_second =
                self.vertical_velocity_meters_per_second.max(0.0);
            self.grounded = false;
        }
        eye_altitude_meters
    }

    /// Only a commanded dive submerges you.
    ///
    /// This asymmetry is the whole point. If merely being below the waterline
    /// set the flag, a crest overtaking a floating swimmer would set it, the
    /// crest guard would switch itself off, and the sea would close over an eye
    /// that never asked to go under. Only a deliberate upward stroke through
    /// the guard's clearance ends a dive, not a moving trough passing the eye.
    fn update_submerged(&mut self, eye_altitude_meters: f64, water_height: f64, stroke: f64) {
        if stroke <= -DELIBERATE_DESCENT_SPEED_METERS_PER_SECOND
            && eye_altitude_meters < water_height
        {
            self.submerged = true;
        } else if stroke >= DELIBERATE_DESCENT_SPEED_METERS_PER_SECOND
            && eye_altitude_meters >= water_height + MINIMUM_SWIMMING_EYE_CLEARANCE_METERS
        {
            self.submerged = false;
        }
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

/// How much of the surface-floating model still applies at this eye altitude.
///
/// One at and above the waterline, falling to zero `NEUTRAL_BUOYANCY_DEPTH_METERS`
/// under it. While the crest guard is holding the eye above the surface this is
/// pinned at one, which is why nothing about swimming on the surface changes.
fn surface_authority(eye_altitude_meters: f64, water_height: f64) -> f64 {
    ((eye_altitude_meters - (water_height - NEUTRAL_BUOYANCY_DEPTH_METERS))
        / NEUTRAL_BUOYANCY_DEPTH_METERS)
        .clamp(0.0, 1.0)
}

/// The lowest the eye may go over water: the sea bed, at any depth.
///
/// The single definition of that floor. `advance_vertical` holds it during
/// movement and `resolve_surface_camera_after_streaming` holds it again after a
/// tile lands, and they must agree -- when the second one had its own
/// arithmetic it used the walking eye height and quietly overrode this one.
pub fn swimming_bed_eye_altitude_meters(bed_height_meters: f64) -> f64 {
    bed_height_meters + SWIMMING_BED_EYE_CLEARANCE_METERS
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
        let mut eye = state.advance_vertical(ground_eye, 0.0, None, true, 0.0, 1.0 / 120.0);
        assert!(eye > ground_eye);
        assert!(state.vertical_velocity_meters_per_second > 0.0);
        for _ in 0..480 {
            eye = state.advance_vertical(eye, 0.0, None, false, 0.0, 1.0 / 120.0);
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
        eye = state.advance_vertical(eye, -100.0, Some((2.0, 1.0)), false, 0.0, 1.0 / 60.0);
        assert!(
            eye < 2.0 + equilibrium,
            "the camera must not snap to the wave"
        );
        assert!(state.vertical_velocity_meters_per_second > 0.0);
        for _ in 0..1_200 {
            eye = state.advance_vertical(eye, -100.0, Some((2.0, 0.0)), false, 0.0, 1.0 / 120.0);
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
                0.0,
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
        let after_impulse = state.advance_vertical(eye, -100.0, Some((0.0, 0.0)), true, 0.0, 0.0);
        assert_eq!(after_impulse, eye);
        assert_eq!(
            state.vertical_velocity_meters_per_second,
            WATER_UPWARD_IMPULSE_METERS_PER_SECOND
        );
        state.advance_vertical(after_impulse, -100.0, Some((0.0, 0.0)), false, 0.0, 0.25);
        assert!(state.vertical_velocity_meters_per_second < WATER_UPWARD_IMPULSE_METERS_PER_SECOND);
    }

    #[test]
    fn a_swimmer_bobs_without_being_swallowed_by_a_storm_crest() {
        // The regression this exists for: with buoyancy alone the eye spent
        // 41% of a storm underwater, because crests here accelerate down at
        // close to g and simply overtake a floating body. Twenty times the
        // restoring force only got that to 23%, so the floor is the fix.

        let direction =
            glam::DVec3::new(0.836442275001636, 0.503727905284262, 0.215922481525239).normalize();
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
            let velocity = ocean::local_wave_vertical_velocity_meters_per_second(
                direction,
                time_seconds,
                depth,
            );
            eye = physics.advance_vertical(
                eye,
                -depth,
                Some((height, velocity)),
                false,
                0.0,
                1.0 / 60.0,
            );
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
            0.0,
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

    /// Swims for `seconds` at a fixed stroke, over still water at sea level.
    fn dive(
        state: &mut SurfacePhysicsState,
        mut eye: f64,
        bed_meters: f64,
        swim_vertical_speed: f64,
        seconds: f64,
    ) -> f64 {
        let step = 1.0 / 60.0;
        for _ in 0..(seconds / step).round() as usize {
            eye = state.advance_vertical(
                eye,
                bed_meters,
                Some((0.0, 0.0)),
                false,
                swim_vertical_speed,
                step,
            );
        }
        eye
    }

    #[test]
    fn a_downward_stroke_carries_the_eye_under_the_surface() {
        // The feature: pitching below the horizontal descends. Before this the
        // crest guard ended every substep at least 0.06 m above the water, so
        // no stroke of any size could get the eye under at all.
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let eye = dive(
            &mut state,
            equilibrium_eye_height_above_water_meters(),
            -60.0,
            -2.0,
            2.0,
        );
        assert!(
            eye < -2.0,
            "two seconds of a 2 m/s dive only reached {eye:.3} m"
        );
        assert!(state.submerged, "the diver is under the surface");
        assert!(state.in_water);
        assert!(!state.grounded);
    }

    #[test]
    fn a_dive_goes_weightless_almost_as_soon_as_the_eye_is_under() {
        // The fade is short on purpose. It used to be a body height, which left
        // enough buoyancy at -1.0m to float a stopped diver back out in about
        // ten seconds -- and a metre under is exactly where you want to hold
        // station to look up at Snell's window.
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let start = equilibrium_eye_height_above_water_meters();
        let after_two = dive(&mut state, start, -600.0, -2.0, 2.0);
        let first_two_seconds = start - after_two;
        // Free of the surface the stroke alone would cover 4.0 m in that time,
        // and the only thing missing is the moment spent crossing the fade.
        assert!(
            (3.8..4.0).contains(&first_two_seconds),
            "the first two seconds covered {first_two_seconds:.3} m of a possible 4.0 m"
        );

        // Under the fade the diver is weightless, so the settled descent is the
        // stroke itself with nothing left to subtract from it.
        let after_four = dive(&mut state, after_two, -600.0, -2.0, 2.0);
        let settled_rate = (after_two - after_four) / 2.0;
        assert!(
            (settled_rate - 2.0).abs() < 0.01,
            "settled descent was {settled_rate:.4} m/s, expected the 2.0 m/s stroke"
        );
    }

    #[test]
    fn a_level_stroke_still_cannot_sink_the_swimmer() {
        // The other half of it: the guard has to stay on for everyone who is
        // not diving, including a look vector a few ulps off horizontal.
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        for stroke in [0.0, -1e-15, -0.04, 0.04] {
            let eye = dive(
                &mut state,
                equilibrium_eye_height_above_water_meters(),
                -60.0,
                stroke,
                2.0,
            );
            assert!(
                eye >= MINIMUM_SWIMMING_EYE_CLEARANCE_METERS,
                "a {stroke} m/s stroke sank the eye to {eye:.3} m"
            );
            assert!(!state.submerged);
        }
    }

    #[test]
    fn a_diver_who_stops_swimming_holds_the_depth_they_stopped_at() {
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let deep = dive(
            &mut state,
            equilibrium_eye_height_above_water_meters(),
            -60.0,
            -2.0,
            5.0,
        );
        assert!(state.submerged);

        // Release the stroke. A submerged diver is neutrally buoyant, so this
        // has to hold station rather than drift either way.
        let held = dive(&mut state, deep, -60.0, 0.0, 30.0);
        assert!(
            (held - deep).abs() < 0.05,
            "released at {deep:.3} m and drifted to {held:.3} m in 30 s"
        );
        assert!(
            state.vertical_velocity_meters_per_second.abs() < 0.01,
            "still moving at {:.4} m/s",
            state.vertical_velocity_meters_per_second
        );
        assert!(state.submerged, "still under");
    }

    #[test]
    fn moving_wave_trough_does_not_recapture_a_stopped_diver() {
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let mut eye = dive(&mut state, -7.0, -1_000.0, -2.0, 0.5);
        state.vertical_velocity_meters_per_second = 0.0;
        let held = eye;
        assert!(state.submerged);
        let mut highest = held;
        for frame in 0..3_600 {
            let time = frame as f64 / 60.0;
            // The trough briefly uncovers the eye, but not the body. The
            // subsequent crest must not turn a stopped diver into a floater.
            let height = held + 0.2 + 0.4 * (time * 2.0).cos();
            let velocity = -0.8 * (time * 2.0).sin();
            eye = state.advance_vertical(
                eye,
                -1_000.0,
                Some((height, velocity)),
                false,
                0.0,
                1.0 / 60.0,
            );
            highest = highest.max(eye);
        }
        assert!(
            (eye - held).abs() < 1e-6 && highest <= held + 1e-6,
            "moving water pulled the stopped diver from {held}m to {eye}m, highest {highest}m"
        );
        assert!(state.submerged);
    }

    #[test]
    fn a_stopped_diver_holds_station_against_a_frozen_wave() {
        // Reported from the app: freeze the scene under a shallow dive and the
        // camera climbs back to the surface. Freezing is not really what does
        // it -- it stops the wave field, so `water_vertical_velocity` sticks at
        // whatever it was and the drag term references a surface rising for
        // ever, while the restoring spring pulls toward a fixed equilibrium.
        // Both are surface devices and neither may reach a diver.
        for depth in [0.5, 1.0, 8.0] {
            for frozen_velocity in [0.0, 2.5, -2.5] {
                let mut state = SurfacePhysicsState::default();
                state.settle_in_water();
                state.in_water = true;
                state.submerged = true;
                let mut eye = -depth;
                for _ in 0..1_800 {
                    eye = state.advance_vertical(
                        eye,
                        -60.0,
                        Some((0.0, frozen_velocity)),
                        false,
                        0.0,
                        1.0 / 60.0,
                    );
                }
                assert!(
                    (eye + depth).abs() < 1.0e-6,
                    "held at -{depth} m against a frozen {frozen_velocity} m/s wave for 30 s \
                     and drifted to {eye:.4} m"
                );
            }
        }
    }

    #[test]
    fn a_diver_surfaces_by_swimming_up_and_the_crest_guard_comes_back() {
        // Holding depth means the way back up is the stroke, so the ascent has
        // to actually reach the surface and hand the swimmer back to the
        // floating model rather than stalling a body height under it.
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let deep = dive(
            &mut state,
            equilibrium_eye_height_above_water_meters(),
            -60.0,
            -2.0,
            5.0,
        );
        let surfaced = dive(&mut state, deep, -60.0, 2.0, 10.0);
        assert!(
            surfaced >= MINIMUM_SWIMMING_EYE_CLEARANCE_METERS,
            "swam up for 10 s from {deep:.3} m and only reached {surfaced:.3} m"
        );
        assert!(
            !state.submerged,
            "the crest guard is still off after surfacing"
        );
    }

    #[test]
    fn a_shallow_dive_stops_at_the_sea_bed() {
        // Over water both ground clamps are disabled, so without a bed of its
        // own a dive would swim straight through the bathymetry.
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let bed = -8.0;
        let eye = dive(
            &mut state,
            equilibrium_eye_height_above_water_meters(),
            bed,
            -2.0,
            30.0,
        );
        assert!(
            (eye - swimming_bed_eye_altitude_meters(bed)).abs() < 1e-6,
            "came to rest at {eye:.3} m over a {bed} m bed"
        );
    }

    #[test]
    fn a_deep_ocean_dive_reaches_the_bed_rather_than_a_depth_cap() {
        // The core clearance is a backstop for a runaway on dry land. Over
        // water the bed is the floor at any depth, so four kilometres of ocean
        // is four kilometres of dive.
        let bed = -4_000.0;
        let mut state = SurfacePhysicsState::default();
        state.settle_in_water();
        let deep = dive(
            &mut state,
            equilibrium_eye_height_above_water_meters(),
            bed,
            -2.0,
            2_400.0,
        );
        assert!(
            (deep - swimming_bed_eye_altitude_meters(bed)).abs() < 1e-6,
            "a 4 km dive stopped at {deep:.3} m, not the bed at \
             {:.3} m",
            swimming_bed_eye_altitude_meters(bed)
        );
        assert!(
            deep < PLANET_CORE_CLEARANCE_METERS,
            "the dive never got past the core clearance backstop"
        );
    }

    #[test]
    fn a_runaway_downward_velocity_is_still_caught_by_the_bed() {
        // This used to assert the core clearance. Now that a dive may pass it,
        // the bed is what has to stop a runaway over water -- and it is a
        // tighter bound than the one it replaces, not a looser one.
        let bed = -1_000.0;
        let mut state = SurfacePhysicsState {
            vertical_velocity_meters_per_second: -100.0,
            grounded: false,
            in_water: true,
            submerged: false,
        };
        let eye = state.advance_vertical(0.1, bed, Some((-100.0, 0.0)), false, 0.0, 1.0);
        assert!(
            eye >= swimming_bed_eye_altitude_meters(bed),
            "fell to {eye:.3} m, past a bed at {bed} m"
        );
        assert!(state.vertical_velocity_meters_per_second >= 0.0);
    }
}

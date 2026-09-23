//! Invertible horizontal ocean compression, not yet enabled in the renderer.
//!
//! A surface parameter is not its displaced world direction. Queries first
//! invert that mapping, then differentiate the displaced surface (including
//! horizontal motion). Keeping these steps together prevents a visually steep
//! crest from leaving the ship/camera on the old radial height field.
use glam::{DMat3, DVec3};

use super::*;

const MAX_COMPRESSION: f64 = 0.95;
const QUERY_TOLERANCE_METERS: f64 = 1.0e-5;
const MAX_QUERY_ITERATIONS: usize = 24;
const TRANSPORT_WAVELENGTH_MIN_METERS: f64 = 100.0;
const TRANSPORT_WAVELENGTH_MAX_METERS: f64 = 200.0;

fn carries_compression(wave: &GerstnerWave) -> bool {
    (TRANSPORT_WAVELENGTH_MIN_METERS..=TRANSPORT_WAVELENGTH_MAX_METERS)
        .contains(&wave.wavelength_meters)
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

#[derive(Clone, Copy)]
struct TransportSample {
    displacement: DVec3,
    /// Derivative with respect to metre-scale tangential parameter movement.
    derivative: DMat3,
    velocity: DVec3,
}

/// Bound the sum of derivative norms, including spherical curvature and the
/// coast's blend derivative, rather than clamping a Jacobian after it folds.
/// This conservative budget is deliberately independent of phase and time.
pub(super) fn compression_scale() -> f64 {
    let radius = planet_radius_meters();
    let coast_gradient_bound = if spawn_coast_waves_enabled() {
        let span = SPAWN_COAST_OUTER.powi(2) - SPAWN_COAST_INNER.powi(2);
        3.0 * SPAWN_COAST_OUTER / (radius * span)
    } else {
        0.0
    };
    let bound = active_waves()
        .iter()
        .filter(|wave| carries_compression(wave))
        .map(|wave| {
            let amplitude = wave.amplitude_meters.max(wave.storm_amplitude_meters)
                * OCEAN_STORM_GEOMETRY_AMPLITUDE_SCALE;
            let k = std::f64::consts::TAU / wave.wavelength_meters;
            amplitude
                * wave.steepness
                * OCEAN_STEEPNESS_SCALE
                * (k + 2.0 / radius + 2.0 * coast_gradient_bound)
        })
        .sum::<f64>();
    MAX_COMPRESSION / bound.max(1.0e-12)
}

fn transport_sample(q: DVec3, time: f64, state: SeaState, strength: f64) -> TransportSample {
    let radius = planet_radius_meters();
    let projection = DMat3::IDENTITY - outer(q, q);
    let mut result = TransportSample {
        displacement: DVec3::ZERO,
        derivative: DMat3::ZERO,
        velocity: DVec3::ZERO,
    };
    let gain = compression_scale() * strength.clamp(0.0, 1.0);
    for wave in active_waves() {
        if !carries_compression(wave) {
            continue;
        }
        let axis = wave.direction.normalize();
        let tangent = projection * axis;
        let k = std::f64::consts::TAU / wave.wavelength_meters;
        let wind = ocean_wind();
        let weight = wind.map_or(1.0, |wind| {
            wind.amplitude_weight(axis, wave.wavelength_meters)
        });
        let sign = OCEAN_WAVE_PHASE_SPEED_SIGN
            * wind.map_or(1.0, |wind| {
                wind.propagation_sign(axis, wave.wavelength_meters)
            });
        let phase = k * (q.dot(axis) * radius + sign * wave.speed_meters_per_second * time);
        let mut cosine = phase.cos();
        let mut cosine_gradient = tangent * (-k * phase.sin());
        let mut cosine_rate = -k * sign * wave.speed_meters_per_second * phase.sin();
        if spawn_coast_waves_enabled() && sign * axis.dot(SPAWN_COAST_ONSHORE) > 0.0 {
            let (blend, gradient) = spawn_coast_weight(q);
            let reverse = k * (q.dot(axis) * radius - sign * wave.speed_meters_per_second * time);
            cosine_gradient = cosine_gradient.lerp(tangent * (-k * reverse.sin()), blend)
                + projection * gradient * (reverse.cos() - cosine);
            cosine_rate +=
                blend * (k * sign * wave.speed_meters_per_second * reverse.sin() - cosine_rate);
            cosine += blend * (reverse.cos() - cosine);
        }
        let scale = wave.steepness * OCEAN_STEEPNESS_SCALE * gain * weight;
        let amplitude = wave.amplitude(storm_blend(state.intensity))
            * geometry_amplitude_scale(state.intensity)
            * scale;
        // Use the projected axis, not its unit vector: the latter is singular
        // at either axis pole and cannot have a global non-folding bound.
        let tangent_derivative = (-outer(q, tangent) - projection * q.dot(axis)) / radius;
        result.displacement += tangent * (amplitude * cosine);
        result.derivative +=
            (outer(tangent, cosine_gradient) + tangent_derivative * cosine) * amplitude;
        result.velocity += tangent
            * (amplitude * cosine_rate + amplitude_change_velocity(wave, state) * scale * cosine);
    }
    result
}

#[derive(Clone, Copy)]
struct ParametricSurface {
    position: DVec3,
    derivative: DMat3,
    velocity: DVec3,
}

fn forward(q: DVec3, time: f64, depth: f64, state: SeaState, strength: f64) -> ParametricSurface {
    let radius = planet_radius_meters();
    let projection = DMat3::IDENTITY - outer(q, q);
    let raw_height = wave_height_meters(q, time, state.intensity, depth);
    let height = raw_height * breaking_weight(raw_height, depth);
    let rate = breaking_rate_weight(raw_height, depth);
    let mut slope = DVec3::ZERO;
    let mut height_velocity = 0.0;
    for wave in active_waves() {
        let sample = sample_wave(q, time, wave);
        let amplitude = wave.amplitude(storm_blend(state.intensity))
            * geometry_amplitude_scale(state.intensity);
        slope += sample.slope * amplitude;
        height_velocity +=
            sample.velocity * amplitude + sample.profile * amplitude_change_velocity(wave, state);
    }
    // No sideways transport in the shallow band. Depth is a supplied local
    // constant, just as in the existing analytic height derivative contract.
    // Sampling a varying bed at displaced positions is a renderer integration
    // requirement, not something this fixed-depth query silently approximates.
    let t = ((depth - 30.0) / 70.0).clamp(0.0, 1.0);
    let depth_ramp = t * t * (3.0 - 2.0 * t);
    let transport = transport_sample(q, time, state, strength * depth_ramp);
    ParametricSurface {
        position: q * (radius + height) + transport.displacement,
        derivative: projection * (1.0 + height / radius)
            + outer(q, projection * slope * rate)
            + transport.derivative,
        velocity: q * (height_velocity * rate) + transport.velocity,
    }
}

#[derive(Debug)]
pub(super) struct SurfaceQuery {
    pub(super) parameter: DVec3,
    pub(super) height: f64,
    pub(super) normal: DVec3,
    pub(super) slope: DVec3,
    pub(super) vertical_velocity: f64,
    pub(super) residual_meters: f64,
}

/// Solve for the actual world radial, not for the undisplaced wave parameter.
/// Failed convergence is explicit; never return an apparently valid old height.
pub(super) fn query(
    direction: DVec3,
    time: f64,
    depth: f64,
    state: SeaState,
    strength: f64,
) -> Option<SurfaceQuery> {
    if !time.is_finite()
        || !depth.is_finite()
        || !strength.is_finite()
        || !state.intensity.is_finite()
        || !state.intensity_rate.is_finite()
    {
        return None;
    }
    invert(direction, |q| forward(q, time, depth, state, strength))
}

fn invert(
    direction: DVec3,
    surface_at: impl Fn(DVec3) -> ParametricSurface,
) -> Option<SurfaceQuery> {
    let target = direction.try_normalize()?;
    let radius = planet_radius_meters();
    let projection = DMat3::IDENTITY - outer(target, target);
    let mut q = target;
    for _ in 0..MAX_QUERY_ITERATIONS {
        let surface = surface_at(q);
        let residual = projection * surface.position;
        // Extend the tangent-to-tangent map with a radial column, so one 3x3
        // solve handles poles without selecting a discontinuous local basis.
        let jacobian = projection * surface.derivative + outer(target, q);
        if !jacobian.is_finite() || jacobian.determinant().abs() < 1.0e-10 {
            return None;
        }
        let inverse = jacobian.inverse();
        if residual.length() <= QUERY_TOLERANCE_METERS {
            let tangent = q.any_orthonormal_vector();
            let mut normal = (surface.derivative * tangent)
                .cross(surface.derivative * q.cross(tangent))
                .normalize();
            if normal.dot(target) < 0.0 {
                normal = -normal;
            }
            let facing = normal.dot(target);
            if facing <= 1.0e-6 {
                return None;
            }
            // At a fixed world radial the parameter moves against horizontal
            // transport. The normal removes that tangential velocity exactly.
            let radial_distance = surface.position.dot(target);
            return Some(SurfaceQuery {
                parameter: q,
                height: radial_distance - radius,
                normal,
                slope: -(normal - target * facing) / facing * radial_distance / radius,
                vertical_velocity: normal.dot(surface.velocity) / facing,
                residual_meters: residual.length(),
            });
        }
        let step = inverse * residual;
        // Backtracking prevents a near-cusp Newton step jumping to another
        // wavelength. An exhausted line search is a failed query, not a clamp.
        let mut fraction = 1.0;
        let mut accepted = false;
        for _ in 0..12 {
            let candidate = (q - step * (fraction / radius)).normalize();
            let error = projection * surface_at(candidate).position;
            if error.length_squared() < residual.length_squared() {
                q = candidate;
                accepted = true;
                break;
            }
            fraction *= 0.5;
        }
        if !accepted {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(intensity: f32) -> SeaState {
        SeaState {
            intensity,
            intensity_rate: 0.0,
        }
    }

    #[test]
    fn compressed_surface_round_trips_on_the_real_planet() {
        let mut largest_shift: f64 = 0.0;
        let mut largest_radial_error: f64 = 0.0;
        for index in 0..96 {
            let z = 1.0 - 2.0 * (index as f64 + 0.5) / 96.0;
            let phi = index as f64 * 2.399963229728653;
            let q = DVec3::new(
                (1.0 - z * z).sqrt() * phi.cos(),
                z,
                (1.0 - z * z).sqrt() * phi.sin(),
            );
            for intensity in [0.0, 0.5, 1.0] {
                for depth in [0.0, 2.0, 30.0, 65.0, 4000.0] {
                    let time = index as f64 * 1.31;
                    let surface = forward(q, time, depth, state(intensity), 1.0);
                    let result =
                        query(surface.position, time, depth, state(intensity), 1.0).unwrap();
                    assert!((result.parameter - q).length() * planet_radius_meters() < 0.001);
                    assert!(
                        (result.height - (surface.position.length() - planet_radius_meters()))
                            .abs()
                            < 0.001
                    );
                    assert!(result.residual_meters <= QUERY_TOLERANCE_METERS);
                    largest_shift = largest_shift
                        .max((surface.position.normalize() - q).length() * planet_radius_meters());
                    let radial_height =
                        wave_height_meters(surface.position.normalize(), time, intensity, depth);
                    largest_radial_error = largest_radial_error.max(
                        (radial_height * breaking_weight(radial_height, depth) - result.height)
                            .abs(),
                    );
                }
            }
        }
        assert!(
            largest_shift > 1.0,
            "the transport regression must not be inert"
        );
        assert!(
            largest_radial_error > 0.1,
            "the old radial query must fail this regression"
        );
    }

    #[test]
    fn inverse_handles_both_faces_of_a_near_cusped_crest() {
        let radius = planet_radius_meters();
        let k = std::f64::consts::TAU / 32.0;
        // Deliberately much closer to singular than the production budget:
        // a 0.995 compression has a minimum horizontal derivative of 0.005.
        let compression = 0.995;
        let surface_at = |q: DVec3| {
            let projection = DMat3::IDENTITY - outer(q, q);
            let tangent = projection * DVec3::X;
            let phase = k * radius * q.x;
            let height = 3.0 * phase.sin();
            let displacement = tangent * (compression / k * phase.cos());
            let derivative = projection * (1.0 + height / radius)
                + outer(q, tangent * (3.0 * k * phase.cos()))
                - outer(tangent, tangent) * (compression * phase.sin())
                - (outer(q, tangent) + projection * q.x)
                    * (compression / (k * radius) * phase.cos());
            ParametricSurface {
                position: q * (radius + height) + displacement,
                derivative,
                velocity: DVec3::ZERO,
            }
        };
        let mut steepest: f64 = 0.0;
        for offset in [-0.3, -0.1, -0.03, 0.0, 0.03, 0.1, 0.3] {
            let x = (std::f64::consts::FRAC_PI_2 + offset) / (k * radius);
            let q = DVec3::new(x, (1.0 - x * x).sqrt(), 0.0);
            let drawn = surface_at(q);
            let result = invert(drawn.position, surface_at).unwrap();
            assert!((result.parameter - q).length() * radius < 0.002);
            assert!((result.height - (drawn.position.length() - radius)).abs() < 0.0001);
            steepest = steepest.max(result.slope.length());
        }
        assert!(
            steepest > 5.0,
            "test a steep geometric face, not just rounded waves"
        );
    }

    #[test]
    fn compressed_surface_derivatives_match_fixed_world_queries() {
        let center = SPAWN_COAST_CENTER.normalize();
        let across = (SPAWN_COAST_ONSHORE - center * center.dot(SPAWN_COAST_ONSHORE)).normalize();
        for q in [
            DVec3::X,
            DVec3::Y,
            DVec3::Z,
            center,
            (center + across * 0.0025).normalize(),
            (center + across * 0.0035).normalize(),
        ] {
            for depth in [2.0, 65.0, 4000.0] {
                let state = SeaState {
                    intensity: 0.5,
                    intensity_rate: 0.003,
                };
                let time = 7.0;
                let h = 0.01;
                let target = forward(q, time, depth, state, 1.0).position.normalize();
                let result = query(target, time, depth, state, 1.0).unwrap();
                let tangent = target.any_orthonormal_vector();
                let plus = query(
                    (target + tangent * h / planet_radius_meters()).normalize(),
                    time,
                    depth,
                    state,
                    1.0,
                )
                .unwrap();
                let minus = query(
                    (target - tangent * h / planet_radius_meters()).normalize(),
                    time,
                    depth,
                    state,
                    1.0,
                )
                .unwrap();
                assert!(
                    ((plus.height - minus.height) / (2.0 * h) - result.slope.dot(tangent)).abs()
                        < 0.002
                );
                let dt = 0.001;
                let after = SeaState {
                    intensity: state.intensity + (state.intensity_rate * dt) as f32,
                    ..state
                };
                let before = SeaState {
                    intensity: state.intensity - (state.intensity_rate * dt) as f32,
                    ..state
                };
                let plus = query(target, time + dt, depth, after, 1.0).unwrap();
                let minus = query(target, time - dt, depth, before, 1.0).unwrap();
                assert!(
                    ((plus.height - minus.height) / (2.0 * dt) - result.vertical_velocity).abs()
                        < 0.002
                );
            }
        }
    }

    #[test]
    fn compression_derivative_is_bounded_and_matches_the_forward_map() {
        for index in 0..100 {
            let q = DVec3::new(0.4, (index as f64).sin(), (index as f64).cos()).normalize();
            let tangent = q.any_orthonormal_vector();
            let time = index as f64 * 0.31;
            let sample = transport_sample(q, time, state(1.0), 1.0);
            assert!((sample.derivative * tangent).length() <= MAX_COMPRESSION);
            let h = 0.01 / planet_radius_meters();
            let plus = transport_sample((q + tangent * h).normalize(), time, state(1.0), 1.0);
            let minus = transport_sample((q - tangent * h).normalize(), time, state(1.0), 1.0);
            assert!(
                ((plus.displacement - minus.displacement) / 0.02 - sample.derivative * tangent)
                    .length()
                    < 1.0e-5
            );
        }
    }

    #[test]
    fn zero_transport_preserves_radial_queries_and_bad_inputs_fail() {
        let direction = DVec3::new(0.836, 0.504, 0.216).normalize();
        for depth in [0.0, 2.0, 4000.0] {
            let result = query(direction, 7.0, depth, state(1.0), 0.0).unwrap();
            let raw = wave_height_meters(direction, 7.0, 1.0, depth);
            assert!((result.height - raw * breaking_weight(raw, depth)).abs() < 1.0e-8);
        }
        assert!(query(DVec3::ZERO, 0.0, 4000.0, state(1.0), 1.0).is_none());
        assert!(query(direction, f64::NAN, 4000.0, state(1.0), 1.0).is_none());
    }
}

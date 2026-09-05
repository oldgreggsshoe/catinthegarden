//! The celestial body the renderer is currently drawing.
//!
//! Everything in this crate used to read one compile-time radius, because there
//! was one world. A moon is the same machinery at a different scale, so the
//! quantities that describe *which* world are gathered here and the rest of the
//! crate asks for them rather than assuming them.
//!
//! **One body is active at a time.** That is a deliberate simplification while
//! the second world is being brought up: the planet is switched off while you
//! stand on the moon. It is what lets the radius reach the shaders as a
//! generated constant rather than a per-draw uniform, which in turn means the
//! CPU and GPU cannot disagree about it — the failure mode this codebase spends
//! most of its tests guarding against. Rendering both at once needs the radius
//! to become a uniform, and that is the next step, not this one.

use std::sync::OnceLock;

/// A world the renderer can draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub name: &'static str,
    pub radius_meters: f64,
    /// Seconds for one rotation about its axis, in simulation time.
    pub rotation_period_seconds: f64,
    /// Whether sea level exists. A body without an ocean resolves negative
    /// baked height as ordinary ground rather than as water.
    pub has_ocean: bool,
    /// Whether the atmosphere, weather, and aerial perspective run. An airless
    /// body draws stars at noon, which is correct rather than a defect.
    pub has_atmosphere: bool,
}

/// The baked planet. Its radius is the one the outmap under
/// `assets/outmaps/test-planet` was generated against, so it is not free to
/// change without a rebake — `catinthegarden_coretypes::PLANET_RADIUS_METERS`
/// remains the baker's copy and the two are asserted equal.
pub const PLANET: Body = Body {
    name: "planet",
    radius_meters: catinthegarden_coretypes::PLANET_RADIUS_METERS,
    rotation_period_seconds: 15.0,
    has_ocean: true,
    has_atmosphere: true,
};

/// A moon at roughly a quarter of the planet's radius, which is the Earth/Luna
/// ratio. Airless and dry, and its terrain is synthesised rather than baked, so
/// it needs no outmap of its own.
pub const MOON: Body = Body {
    name: "moon",
    radius_meters: 1_080_000.0,
    // Tidally locked bodies turn once per orbit. Until an orbit exists this is
    // simply slower than the planet, so a standing observer sees the sky move.
    rotation_period_seconds: 60.0,
    has_ocean: false,
    has_atmosphere: false,
};

static ACTIVE: OnceLock<Body> = OnceLock::new();

/// Selects the world for this process. Only the first call takes effect, and
/// the returned bool says whether it did: the radius is baked into generated
/// shader source at pipeline construction, so a later change would leave the
/// GPU describing a different world from the CPU.
pub fn set_active(body: Body) -> bool {
    ACTIVE.set(body).is_ok()
}

pub fn active() -> Body {
    *ACTIVE.get_or_init(|| PLANET)
}

/// Radius of the body being drawn. This is the accessor that replaced a
/// crate-wide constant; it is read on hot paths, so it stays a plain copy out
/// of a `OnceLock` rather than anything that allocates or locks.
pub fn radius_meters() -> f64 {
    active().radius_meters
}

/// Not yet consumed: rotation is still driven by
/// `planet::PLANET_ROTATION_PERIOD_SECONDS`, which several `const` expressions
/// derive from. Moving those to the active body is the next stage.
#[allow(dead_code)]
pub fn rotation_period_seconds() -> f64 {
    active().rotation_period_seconds
}

/// Not yet consumed: sea level is still unconditional. The moon's dry, airless
/// behaviour lands with its terrain.
#[allow(dead_code)]
pub fn has_ocean() -> bool {
    active().has_ocean
}

#[allow(dead_code)]
pub fn has_atmosphere() -> bool {
    active().has_atmosphere
}

/// The generated WGSL every shader that needs the body's scale prepends.
///
/// The radius used to be written out by hand as `4000000.0` in eight separate
/// shaders, with a test policing that they all matched Rust. Generating it
/// removes the thing the test was guarding instead of guarding it, and is what
/// allows the value to differ per world at all.
pub fn wgsl_constants() -> String {
    let radius = radius_meters();
    format!(
        "// Generated from body.rs for `{}`. Do not edit here.\n\
         const PLANET_RADIUS_METERS: f32 = {radius:.1};\n",
        active().name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checked at compile time: these compare constants, so the optimiser
    /// already knows the answer and a runtime assertion proves nothing.
    #[test]
    fn the_planet_matches_the_radius_its_outmap_was_baked_against() {
        const {
            assert!(
                PLANET.radius_meters == catinthegarden_coretypes::PLANET_RADIUS_METERS,
                "the planet's radius must match the one its outmap was baked against",
            );
        }
    }

    #[test]
    fn the_moon_is_a_smaller_airless_dry_body() {
        const {
            assert!(MOON.radius_meters < PLANET.radius_meters);
            assert!(!MOON.has_ocean);
            assert!(!MOON.has_atmosphere);
            assert!(PLANET.has_ocean && PLANET.has_atmosphere);
        }
    }

    #[test]
    fn the_generated_shader_constant_carries_the_active_radius() {
        let generated = wgsl_constants();
        let declared = generated
            .split("const PLANET_RADIUS_METERS: f32 = ")
            .nth(1)
            .and_then(|rest| rest.split(';').next())
            .expect("the generated source declares the radius")
            .trim()
            .parse::<f64>()
            .expect("the radius is generated as a plain literal");
        assert_eq!(declared, radius_meters());
    }
}

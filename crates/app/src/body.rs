//! The celestial body the renderer is currently drawing.
//!
//! Everything in this crate used to read one compile-time radius, because there
//! was one world. A moon is the same machinery at a different scale, so the
//! quantities that describe *which* world are gathered here and the rest of the
//! crate asks for them rather than assuming them.
//!
//! The normal renderer selects its default body once. Multi-body rendering
//! constructs persistent resources inside `with_body` scopes: each pipeline
//! retains its generated constants, while CPU updates use the matching scope.
//! This preserves CPU/GPU agreement without rebuilding shaders during travel.

use std::cell::Cell;
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
    /// Multiplies the surface albedo. The planet's materials are already the
    /// colours its biomes describe, so it uses white; the moon recolours the
    /// same shading chain rather than duplicating it.
    pub terrain_tint: [f32; 3],
    /// Multiplies the water albedo, on the same principle.
    pub water_tint: [f32; 3],
    /// Multiplies the ice albedo, so a body can have ice of its own colour
    /// without touching the shared biome palette the planet's glaciers read
    /// from. The planet uses white and its ice is exactly what `biome_color`
    /// says.
    pub ice_tint: [f32; 3],
    /// Multiplies baked positive terrain height.
    ///
    /// The planet's geography is deliberately exaggerated four times for play:
    /// its baked relief is Earth-like and Earth is very flat at this radius.
    /// The moon's craters are already the size an impact of that diameter
    /// digs, so exaggerating them would be inventing terrain rather than
    /// showing it -- and would take its 16km datum to 64km.
    pub outmap_height_scale: f64,
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
    terrain_tint: [1.0, 1.0, 1.0],
    water_tint: [1.0, 1.0, 1.0],
    ice_tint: [1.0, 1.0, 1.0],
    outmap_height_scale: 4.0,
};

/// A moon at roughly a quarter of the planet's radius, which is the Earth/Luna
/// ratio. Airless and dry, and its terrain is synthesised rather than baked, so
/// it needs no outmap of its own.
pub const MOON: Body = Body {
    name: "moon",
    radius_meters: catinthegarden_coretypes::moon::MOON_RADIUS_METERS,
    // Tidally locked bodies turn once per orbit. Until an orbit exists this is
    // simply slower than the planet, so a standing observer sees the sky move.
    rotation_period_seconds: 60.0,
    // No liquid at all. What fills the crater floors is ice, and ice is
    // terrain: it is drawn by the terrain pass, collided with by the ground
    // query, and walked on with no special case anywhere.
    has_ocean: false,
    has_atmosphere: false,
    // Grey-white regolith. The ice takes its own biome material.
    terrain_tint: [0.86, 0.86, 0.88],
    water_tint: [1.0, 1.0, 1.0],
    // Pink. Not a colour ice comes in, and asked for anyway -- it is the one
    // thing on this body that is a choice rather than a consequence.
    //
    // Stronger than the ratio that would turn the palette's pale blue pink on
    // its own, because almost no pixel is pure ice: the shader mixes ice into
    // regolith by how permanently shadowed the ground is, and a half-mixed pink
    // against grey reads as off-white. Set from the *rendered* result rather
    // than from the palette entry.
    ice_tint: [1.36, 0.22, 0.55],
    outmap_height_scale: 1.0,
};

static ACTIVE: OnceLock<Body> = OnceLock::new();

thread_local! {
    static RENDER_BODY: Cell<Option<Body>> = const { Cell::new(None) };
}

/// Evaluate one persistent body's CPU work and shader construction without
/// changing the process default or another thread's body. GPU pipelines keep
/// their generated constants; there is no shader compilation during travel.
pub(crate) fn with_body<T>(body: Body, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Body>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RENDER_BODY.set(self.0);
        }
    }
    let _restore = Restore(RENDER_BODY.replace(Some(body)));
    work()
}

/// Selects the world for this process. Only the first call takes effect, and
/// the returned bool says whether it did: the radius is baked into generated
/// shader source at pipeline construction, so a later change would leave the
/// GPU describing a different world from the CPU.
pub fn set_active(body: Body) -> bool {
    ACTIVE.set(body).is_ok()
}

pub fn active() -> Body {
    RENDER_BODY
        .get()
        .unwrap_or_else(|| *ACTIVE.get_or_init(|| PLANET))
}

/// Radius of the body being drawn. This is the accessor that replaced a
/// crate-wide constant; it is read on hot paths, so it stays a plain copy out
/// of the scoped body (or process default), without allocation or locking.
pub fn radius_meters() -> f64 {
    active().radius_meters
}

pub fn rotation_period_seconds() -> f64 {
    active().rotation_period_seconds
}

/// How much the baked height is exaggerated on this body.
pub fn outmap_height_scale() -> f64 {
    active().outmap_height_scale
}

pub fn has_ocean() -> bool {
    active().has_ocean
}

/// Whether weather exists at all. On a body without air there is nothing to
/// hold cloud, rain, light shafts, or aerial perspective.
pub fn has_atmosphere() -> bool {
    active().has_atmosphere
}

/// Whether the game begins standing on this body rather than in orbit.
///
/// The moon is somewhere you arrive on foot; the planet still opens on the
/// orbital view it was built around. Kept here as a property of the world
/// rather than a branch in the camera code, so it is testable without a window.
pub fn spawns_on_surface(body: Body) -> bool {
    body == MOON
}

/// The generated WGSL every shader that needs the body's scale prepends.
///
/// The radius used to be written out by hand as `4000000.0` in eight separate
/// shaders, with a test policing that they all matched Rust. Generating it
/// removes the thing the test was guarding instead of guarding it, and is what
/// allows the value to differ per world at all.
pub fn wgsl_constants() -> String {
    let radius = radius_meters();
    let is_moon = active().name == MOON.name;
    let has_ocean = active().has_ocean;
    let has_atmosphere = active().has_atmosphere;
    let [tt0, tt1, tt2] = active().terrain_tint;
    let [wt0, wt1, wt2] = active().water_tint;
    let [it0, it1, it2] = active().ice_tint;
    format!(
        "// Generated from body.rs for `{}`. Do not edit here.\n\
         const PLANET_RADIUS_METERS: f32 = {radius:.1};\n\
         const BODY_IS_MOON: bool = {is_moon};\n\
         const BODY_HAS_OCEAN: bool = {has_ocean};\n\
         const BODY_HAS_ATMOSPHERE: bool = {has_atmosphere};\n\
         const BODY_TERRAIN_TINT: vec3<f32> = vec3<f32>({tt0}, {tt1}, {tt2});\n\
         const BODY_WATER_TINT: vec3<f32> = vec3<f32>({wt0}, {wt1}, {wt2});\n\
         const BODY_ICE_TINT: vec3<f32> = vec3<f32>({it0}, {it1}, {it2});\n{}",
        active().name,
        crate::moon::wgsl_constants(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_scopes_nest_and_restore_after_panics() {
        let original = active();
        with_body(MOON, || {
            assert_eq!(active(), MOON);
            with_body(PLANET, || assert_eq!(active(), PLANET));
            assert_eq!(active(), MOON);
        });
        assert_eq!(active(), original);
        let _ = std::panic::catch_unwind(|| with_body(MOON, || panic!("scope test")));
        assert_eq!(active(), original);
    }

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
    fn the_moon_is_stood_on_and_the_planet_is_orbited() {
        assert!(spawns_on_surface(MOON));
        assert!(!spawns_on_surface(PLANET));
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

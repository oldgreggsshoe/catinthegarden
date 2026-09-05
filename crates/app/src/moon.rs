//! The moon's macro height field: impact craters, not eroded terrain.
//!
//! The planet's geography is baked, because erosion, hydrology and climate need
//! a pipeline. A moon needs none of that — an airless, dry body's large-scale
//! shape is overwhelmingly its impact history — so its macro height is
//! synthesised from a crater catalogue instead of streamed from tiles. That is
//! why the moon needs no bake of its own.
//!
//! **The catalogue is generated once and emitted into the shader**, rather than
//! hand-mirrored the way `OCEAN_WAVE_TABLE` is. The ocean table has a test
//! asserting every literal matches between Rust and WGSL; generating removes
//! the need for that test rather than adding a second one, and it is the same
//! reason the body's radius is generated. The CPU clearance query and the GPU
//! displacement evaluate arithmetic built from identical numbers.
//!
//! The profile is deliberately smooth trigonometry and polynomials, with no
//! hashing and no `fract`: a CPU/GPU field folded by `fract` is either
//! bit-identical or unrelated, and this one has to be evaluated in f64 on one
//! side and f32 on the other.

use glam::DVec3;

/// One impact structure, as a cap on the unit sphere.
#[derive(Clone, Copy, Debug)]
pub struct Crater {
    /// Unit direction of the crater's centre.
    pub axis: DVec3,
    /// Angular radius of the rim crest, in radians.
    pub angular_radius: f64,
    /// Floor depth below the surrounding datum, in metres.
    pub depth_meters: f64,
    /// Rim height above the surrounding datum, in metres.
    pub rim_meters: f64,
}

/// How many impacts the catalogue carries.
///
/// Every one is evaluated per vertex, so this is a frame-time cost as much as a
/// visual choice: the count is kept to what reads as a cratered body at the
/// scale the mesh can actually resolve, not what a photograph of Luna contains.
/// Small craters below the vertex spacing belong to the detail ladder, not
/// here.
pub const CRATER_COUNT: usize = 48;

/// Largest and smallest rim radius in the catalogue, in radians. At the moon's
/// 1,080km radius, 0.30 rad is a 324km basin and 0.02 rad is a 22km crater.
const MAX_ANGULAR_RADIUS: f64 = 0.30;
const MIN_ANGULAR_RADIUS: f64 = 0.02;

/// Fresh craters run about a fifth as deep as they are wide; large basins
/// relax much shallower than that, which is why depth is not simply
/// proportional to radius.
const DEPTH_TO_RADIUS: f64 = 0.20;
/// How far up a polar crater the ice reaches, as a fraction of its depth.
const POLAR_ICE_DEPTH_FRACTION: f64 = 0.5;
/// Sine of the latitude where polar ice starts appearing and where it is fully
/// established. 0.80 is about 53 degrees, 0.94 about 70.
const POLAR_ICE_LATITUDE_SINE_START: f64 = 0.80;
const POLAR_ICE_LATITUDE_SINE_FULL: f64 = 0.94;
const RIM_TO_DEPTH: f64 = 0.28;
/// How far the ejecta reaches past the rim crest, as a multiple of the rim
/// radius. A real blanket is thin and close; 2.0 made a plateau, not a skirt.
const EJECTA_EXTENT: f64 = 1.35;

/// Deterministic, evenly spread impact sites.
///
/// A golden-angle spiral distributes the centres without clumping, and a
/// sine-based mix on the index varies size. There is no randomness and no seed
/// to drift: the same catalogue is produced on every run and on both sides of
/// the CPU/GPU divide.
pub fn craters() -> Vec<Crater> {
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..CRATER_COUNT)
        .map(|index| {
            let i = index as f64;
            let z = 1.0 - 2.0 * (i + 0.5) / CRATER_COUNT as f64;
            let ring_radius = (1.0 - z * z).max(0.0).sqrt();
            let theta = golden_angle * i;
            let axis =
                DVec3::new(ring_radius * theta.cos(), z, ring_radius * theta.sin()).normalize();

            // A size mix in [0,1] that is smooth in the index but not periodic
            // with the spiral, so neighbours differ.
            let mix = 0.5 * (1.0 + (i * 2.399_963 + 1.7).sin() * (i * 0.531_7 + 0.3).cos());
            // Cube the mix so most impacts are small and a few are basins,
            // which is the shape of a real size-frequency distribution.
            let size = mix * mix * mix;
            let angular_radius =
                MIN_ANGULAR_RADIUS + (MAX_ANGULAR_RADIUS - MIN_ANGULAR_RADIUS) * size;

            // Big basins relax: depth grows with radius but sub-linearly.
            let radius_meters = angular_radius * crate::body::MOON.radius_meters;
            let relaxation = 1.0 / (1.0 + radius_meters / 60_000.0);
            let depth_meters = DEPTH_TO_RADIUS * radius_meters * relaxation;
            Crater {
                axis,
                angular_radius,
                depth_meters,
                rim_meters: depth_meters * RIM_TO_DEPTH,
            }
        })
        .collect()
}

/// The crater profile, as a function of angular distance over rim radius.
///
/// `t = 0` is the centre, `t = 1` the rim crest. Inside the rim it is a bowl;
/// outside, the ejecta blanket decays to nothing by `t = 2`, so a crater has
/// bounded influence and the field stays local. Returns metres relative to the
/// surrounding datum, negative inside.
fn profile(t: f64, depth_meters: f64, rim_meters: f64) -> f64 {
    if t >= EJECTA_EXTENT {
        return 0.0;
    }
    if t <= 1.0 {
        // A paraboloid floor lifted so it meets the rim crest at t = 1.
        let bowl = depth_meters * (t * t - 1.0);
        let rim = rim_meters * t * t * t;
        return bowl + rim;
    }
    // Ejecta thins very fast with distance -- roughly an inverse cube in the
    // real thing. It was a smoothstep out to twice the rim radius, which is not
    // a blanket but a raised plateau half the crater wide again, and it read as
    // a huge bright ring around every impact.
    let outer = ((t - 1.0) / (EJECTA_EXTENT - 1.0)).clamp(0.0, 1.0);
    let remaining = 1.0 - outer;
    rim_meters * remaining * remaining * remaining
}

/// The moon's macro height with the ice sheets in place, in metres about its
/// datum. This is the surface that is drawn and stood on.
///
/// Everything the impacts dug below the datum is filled level with it: there is
/// no liquid on this body, only ice, and ice ponds flat. Because the fill is
/// part of the *terrain* rather than an ocean surface, walking on it needs no
/// special case — the ground query and the collision surface are already this
/// height.
pub fn height_meters(direction: DVec3) -> f64 {
    let raw = raw_height_meters(direction);
    let ice = ice_surface_meters(direction);
    // Away from the poles there is no ice, and the bowl is left as the impact
    // dug it. Filling to the datum there would flatten every crater on the
    // body into a disc.
    if ice < 0.0 { raw.max(ice) } else { raw }
}

/// Latitude weight for a polar cold trap, 0 at the equator and 1 at the poles.
///
/// The rotation axis is Y, so this is the sine of the latitude. Ice survives on
/// an airless body only where the sun never reaches: crater floors near the
/// poles, permanently shadowed. Everywhere else it sublimes away, which is why
/// the rest of the craters are dry.
fn polar_weight(direction: DVec3) -> f64 {
    let latitude_sine = direction.normalize().y.abs();
    smoothstep(
        POLAR_ICE_LATITUDE_SINE_START,
        POLAR_ICE_LATITUDE_SINE_FULL,
        latitude_sine,
    )
}

fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The level an ice sheet ponds at, in metres about the datum. Zero away from
/// the poles, so a crater there is simply empty.
///
/// Within one crater this is constant, because it is half the depth of the
/// crater that dominates the point — so the ice surface is flat, as a frozen
/// pond should be, rather than following the bowl down.
fn ice_surface_meters(direction: DVec3) -> f64 {
    let polar = polar_weight(direction);
    if polar <= 0.0 {
        return 0.0;
    }
    let direction = direction.normalize();
    let mut dominant_depth = 0.0_f64;
    let mut deepest = 0.0_f64;
    for crater in craters() {
        let cosine = crater.axis.dot(direction).clamp(-1.0, 1.0);
        let t = cosine.acos() / crater.angular_radius;
        let contribution = profile(t, crater.depth_meters, crater.rim_meters);
        if contribution < deepest {
            deepest = contribution;
            dominant_depth = crater.depth_meters;
        }
    }
    -POLAR_ICE_DEPTH_FRACTION * dominant_depth * polar
}

/// True where ice actually covers the floor: the impact dug below the level
/// the ice ponds at. Away from the poles that level is the datum and nothing
/// reaches it, so those craters read as bare rock. Mirrors the shader's
/// `moon_is_ice`.
#[cfg(test)]
pub fn is_ice_at(direction: DVec3) -> bool {
    let surface = ice_surface_meters(direction);
    surface < 0.0 && raw_height_meters(direction) < surface
}

/// The impact field before the ice fills it. The shader's `moon_is_ice` uses
/// its sign to choose ice over regolith; nothing on the CPU needs that yet,
/// because the ice is terrain and the ground query only wants a height.
pub fn raw_height_meters(direction: DVec3) -> f64 {
    let direction = direction.normalize();
    craters()
        .iter()
        .map(|crater| {
            let cosine = crater.axis.dot(direction).clamp(-1.0, 1.0);
            let t = cosine.acos() / crater.angular_radius;
            profile(t, crater.depth_meters, crater.rim_meters)
        })
        .sum()
}

/// The catalogue and the profile, emitted as WGSL.
///
/// Generated rather than hand-written for the same reason the radius is: two
/// copies of forty-eight craters would be a divergence waiting to happen, and
/// the test that caught it would be policing a problem that need not exist.
pub fn wgsl_constants() -> String {
    let craters = craters();
    let mut source = String::new();
    source.push_str(&format!(
        "// Generated from moon.rs. Do not edit here.\n\
         const MOON_CRATER_COUNT: u32 = {CRATER_COUNT}u;\n\
         // axis.xyz, rim radius in radians; then depth and rim height in metres.\n\
         const MOON_CRATERS: array<vec4<f32>, {CRATER_COUNT}> = array<vec4<f32>, {CRATER_COUNT}>(\n"
    ));
    for crater in &craters {
        source.push_str(&format!(
            "    vec4<f32>({:.9}, {:.9}, {:.9}, {:.9}),\n",
            crater.axis.x, crater.axis.y, crater.axis.z, crater.angular_radius
        ));
    }
    source.push_str(");\n");
    source.push_str(&format!(
        "const MOON_CRATER_DEPTHS: array<vec2<f32>, {CRATER_COUNT}> = array<vec2<f32>, {CRATER_COUNT}>(\n"
    ));
    for crater in &craters {
        source.push_str(&format!(
            "    vec2<f32>({:.4}, {:.4}),\n",
            crater.depth_meters, crater.rim_meters
        ));
    }
    source.push_str(");\n");
    source.push_str(&format!(
        "const MOON_EJECTA_EXTENT: f32 = {EJECTA_EXTENT:.4};\n\
         const MOON_POLAR_ICE_DEPTH_FRACTION: f32 = {POLAR_ICE_DEPTH_FRACTION:.4};\n\
         const MOON_POLAR_ICE_LATITUDE_SINE_START: f32 = {POLAR_ICE_LATITUDE_SINE_START:.4};\n\
         const MOON_POLAR_ICE_LATITUDE_SINE_FULL: f32 = {POLAR_ICE_LATITUDE_SINE_FULL:.4};\n"
    ));
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_deterministic_and_well_formed() {
        let first = craters();
        let second = craters();
        assert_eq!(first.len(), CRATER_COUNT);
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.axis, b.axis);
            assert_eq!(a.angular_radius, b.angular_radius);
            assert_eq!(a.depth_meters, b.depth_meters);
        }
        for crater in &first {
            assert!((crater.axis.length() - 1.0).abs() < 1.0e-12);
            assert!((MIN_ANGULAR_RADIUS..=MAX_ANGULAR_RADIUS).contains(&crater.angular_radius));
            assert!(crater.depth_meters > 0.0);
            assert!(crater.rim_meters > 0.0 && crater.rim_meters < crater.depth_meters);
        }
    }

    #[test]
    fn a_crater_is_a_bowl_inside_a_rim() {
        let crater = craters()[0];
        let centre = profile(0.0, crater.depth_meters, crater.rim_meters);
        let crest = profile(1.0, crater.depth_meters, crater.rim_meters);
        let outside = profile(2.0, crater.depth_meters, crater.rim_meters);
        assert!(centre < 0.0, "the floor sits below the datum");
        assert!(crest > 0.0, "the rim stands above it");
        assert!(centre < crest);
        assert_eq!(outside, 0.0, "influence ends at twice the rim radius");
    }

    /// A crater has to reach zero at the edge of its influence, or every
    /// catalogue entry would offset the whole body and the datum would drift
    /// with the crater count.
    #[test]
    fn influence_is_local() {
        let crater = craters()[3];
        let antipode = -crater.axis;
        let cosine = crater.axis.dot(antipode).clamp(-1.0, 1.0);
        let t = cosine.acos() / crater.angular_radius;
        assert_eq!(profile(t, crater.depth_meters, crater.rim_meters), 0.0);
    }

    #[test]
    fn the_field_dips_inside_a_crater_and_is_finite_everywhere() {
        let crater = craters()[0];
        let floor = height_meters(crater.axis);
        let far = height_meters(-crater.axis);
        assert!(floor < far, "the crater floor is below the far side");
        for direction in [
            DVec3::X,
            DVec3::Y,
            DVec3::Z,
            DVec3::new(0.3, -0.7, 0.2),
            DVec3::new(-0.9, 0.1, 0.4),
        ] {
            assert!(height_meters(direction).is_finite());
        }
    }

    /// Ice survives only where the sun never reaches. Away from the poles a
    /// crater is an empty bowl; near them its floor is flooded to half depth.
    #[test]
    fn only_polar_craters_hold_ice() {
        let mut equatorial_bowls = 0;
        let mut polar_bowls = 0;
        for crater in craters() {
            let raw = raw_height_meters(crater.axis);
            if raw >= -1.0 {
                continue;
            }
            let filled = height_meters(crater.axis);
            if crater.axis.y.abs() < 0.5 {
                equatorial_bowls += 1;
                assert_eq!(filled, raw, "an equatorial crater is empty, not flooded",);
            } else if crater.axis.y.abs() > POLAR_ICE_LATITUDE_SINE_FULL {
                polar_bowls += 1;
                assert!(filled > raw, "a polar crater floor is under ice");
                assert!(
                    filled < 0.0,
                    "the ice sits below the datum, not level with it",
                );
                assert!(is_ice_at(crater.axis), "and reads as ice");
            }
        }
        assert!(
            equatorial_bowls > 0 && polar_bowls > 0,
            "both cases sampled"
        );
    }

    /// The ice surface must be flat across a crater rather than following the
    /// bowl, or it is a coat of paint on the floor instead of a frozen pond.
    #[test]
    fn polar_ice_ponds_level() {
        let polar = craters()
            .into_iter()
            .filter(|crater| crater.axis.y.abs() > POLAR_ICE_LATITUDE_SINE_FULL)
            .max_by(|a, b| a.depth_meters.total_cmp(&b.depth_meters))
            .expect("the catalogue has a polar crater");
        let centre = ice_surface_meters(polar.axis);
        let offset = (polar.axis + DVec3::new(0.0, 0.0, polar.angular_radius * 0.25)).normalize();
        assert!(centre < 0.0);
        assert!(
            (ice_surface_meters(offset) - centre).abs() < 1.0,
            "the pond is level across the floor",
        );
    }

    #[test]
    fn the_generated_catalogue_carries_every_crater() {
        let generated = wgsl_constants();
        assert_eq!(generated.matches("    vec4<f32>(").count(), CRATER_COUNT);
        assert_eq!(generated.matches("    vec2<f32>(").count(), CRATER_COUNT);
        assert!(generated.contains(&format!("const MOON_CRATER_COUNT: u32 = {CRATER_COUNT}u;")));
    }
}

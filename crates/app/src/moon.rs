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
const RIM_TO_DEPTH: f64 = 0.28;

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
    if t >= 2.0 {
        return 0.0;
    }
    if t <= 1.0 {
        // A paraboloid floor lifted so it meets the rim crest at t = 1.
        let bowl = depth_meters * (t * t - 1.0);
        let rim = rim_meters * t * t * t;
        return bowl + rim;
    }
    // Ejecta: from the crest down to the datum, smooth at both ends.
    let outer = (t - 1.0).clamp(0.0, 1.0);
    let fade = 1.0 - outer * outer * (3.0 - 2.0 * outer);
    rim_meters * fade
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
    raw_height_meters(direction).max(0.0)
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

    #[test]
    fn the_generated_catalogue_carries_every_crater() {
        let generated = wgsl_constants();
        assert_eq!(generated.matches("    vec4<f32>(").count(), CRATER_COUNT);
        assert_eq!(generated.matches("    vec2<f32>(").count(), CRATER_COUNT);
        assert!(generated.contains(&format!("const MOON_CRATER_COUNT: u32 = {CRATER_COUNT}u;")));
    }
}

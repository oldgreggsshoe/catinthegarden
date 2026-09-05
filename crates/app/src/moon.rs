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
    /// `cos(EJECTA_EXTENT * angular_radius)`: the dot product below which this
    /// crater cannot reach the sample. Precomputed so the per-sample loop can
    /// reject a distant crater with a comparison instead of an `acos`, which
    /// is what makes a catalogue of hundreds affordable per fragment.
    pub cosine_cutoff: f64,
}

// # Why the catalogue is split in two
//
// There is no baked height for this body, so a crater is a formula rather
// than a stored shape: every height query re-evaluates the catalogue, and
// there are millions of height queries per frame. Testing all of them per
// sample is what a few hundred craters cannot afford — measured at 104ms a
// frame against 17ms for forty-eight.
//
// A crater only reaches a tiny cap of the sphere, and the spiral below walks
// the sphere pole to pole in index order. So the field is *sorted by
// latitude*, and a sample only has to test the slice of it at its own
// latitude. That window is only narrow if every crater in it is small, which
// is why the few large basins live in their own short array that is always
// tested in full. The split is the whole reason the count can be in the
// hundreds.

/// The large impacts, tested at every sample because they reach too far to
/// window usefully. Kept to a handful: one of these costs what a dozen field
/// craters cost.
pub const BASIN_COUNT: usize = 8;

/// The small impacts, sorted by latitude and tested through a window. This is
/// where nearly all of the count lives, which is the same thing as saying most
/// craters are small.
pub const FIELD_COUNT: usize = 352;

/// What the body carries in total.
pub const CRATER_COUNT: usize = BASIN_COUNT + FIELD_COUNT;

/// Largest basin, in radians. At the moon's 1,080km radius, 0.30 rad is a
/// 324km basin.
const MAX_ANGULAR_RADIUS: f64 = 0.30;

/// Largest crater in the windowed field, in radians — 67km. This is the number
/// that sets the window width and therefore the frame cost: everything in the
/// field is at most this big, so a sample need only look this far in latitude.
const FIELD_MAX_ANGULAR_RADIUS: f64 = 0.062;

/// Exponents of the size-frequency law, as radius against rank within each
/// array: the `r`th largest has radius `max * r.powf(-exponent)`.
///
/// A real cumulative distribution runs about `N(>R) ∝ R^-2`, which is an
/// exponent of 0.5. The basins are steeper so the very largest stands alone
/// rather than having near-twins; the field is close to the real law, which
/// spreads 352 craters from 67km down to about 4.8km.
const BASIN_RANK_EXPONENT: f64 = 0.72;
const FIELD_RANK_EXPONENT: f64 = 0.45;

/// Coprime with `FIELD_COUNT`, so `index * STRIDE % FIELD_COUNT` visits every
/// rank exactly once. Without it the field would grade smoothly from large
/// craters at one pole to small at the other, because index order is latitude
/// order.
const SIZE_RANK_STRIDE: usize = 197;
const SIZE_RANK_OFFSET: usize = 89;

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
pub fn basins() -> &'static [Crater] {
    static CATALOGUE: std::sync::LazyLock<Vec<Crater>> = std::sync::LazyLock::new(|| {
        build_catalogue(BASIN_COUNT, MAX_ANGULAR_RADIUS, BASIN_RANK_EXPONENT)
    });
    &CATALOGUE
}

/// The windowed field, in descending latitude order.
///
/// The order is not cosmetic: `field_index_range` relies on `axis.y` falling
/// monotonically with the index, and a test asserts it.
pub fn field() -> &'static [Crater] {
    static CATALOGUE: std::sync::LazyLock<Vec<Crater>> = std::sync::LazyLock::new(|| {
        build_catalogue(FIELD_COUNT, FIELD_MAX_ANGULAR_RADIUS, FIELD_RANK_EXPONENT)
    });
    &CATALOGUE
}

fn build_catalogue(count: usize, max_angular_radius: f64, rank_exponent: f64) -> Vec<Crater> {
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..count)
        .map(|index| {
            let i = index as f64;
            let z = 1.0 - 2.0 * (i + 0.5) / count as f64;
            let ring_radius = (1.0 - z * z).max(0.0).sqrt();
            let theta = golden_angle * i;
            let axis =
                DVec3::new(ring_radius * theta.cos(), z, ring_radius * theta.sin()).normalize();

            // Size by rank, not by a smooth function of the index: this gives
            // the catalogue an exact size-frequency law with each rank used
            // once, rather than whatever histogram a sine mix happens to have.
            let rank = (index * SIZE_RANK_STRIDE + SIZE_RANK_OFFSET) % count + 1;
            let angular_radius = max_angular_radius * (rank as f64).powf(-rank_exponent);

            // Big basins relax: depth grows with radius but sub-linearly.
            let radius_meters = angular_radius * crate::body::MOON.radius_meters;
            let relaxation = 1.0 / (1.0 + radius_meters / 60_000.0);
            let depth_meters = DEPTH_TO_RADIUS * radius_meters * relaxation;
            Crater {
                axis,
                angular_radius,
                depth_meters,
                rim_meters: depth_meters * RIM_TO_DEPTH,
                cosine_cutoff: (EJECTA_EXTENT * angular_radius)
                    .min(std::f64::consts::PI)
                    .cos(),
            }
        })
        .collect()
}

/// Half-width of the latitude window, in radians: nothing in the field reaches
/// further than this from its own centre.
const FIELD_WINDOW_RADIANS: f64 = EJECTA_EXTENT * FIELD_MAX_ANGULAR_RADIUS;

/// The slice of `field()` that can possibly reach a sample at this latitude
/// sine, as an inclusive index range.
///
/// Latitude is 1-Lipschitz on the sphere: two directions are at least as far
/// apart as their latitudes are. So a crater whose latitude differs by more
/// than its own reach cannot touch the sample, and since the field is sorted by
/// latitude that leaves one contiguous run. Conservative, never wrong — the
/// exhaustive comparison in the tests is what proves it.
///
/// Mirrored by `moon_field_index_range` in the shader.
fn field_index_range(latitude_sine: f64) -> (usize, usize) {
    let sine = latitude_sine.clamp(-1.0, 1.0);
    let cosine = (1.0 - sine * sine).max(0.0).sqrt();
    let (window_sin, window_cos) = FIELD_WINDOW_RADIANS.sin_cos();
    // sin and cos of (latitude +- window), by the angle-sum identities, so no
    // `asin` is needed. Past a pole the window wraps over it and the bound
    // becomes the pole itself, which `cos(latitude +- window) < 0` detects.
    let upper = if cosine * window_cos - sine * window_sin < 0.0 {
        1.0
    } else {
        (sine * window_cos + cosine * window_sin).clamp(-1.0, 1.0)
    };
    let lower = if cosine * window_cos + sine * window_sin < 0.0 {
        -1.0
    } else {
        (sine * window_cos - cosine * window_sin).clamp(-1.0, 1.0)
    };
    // Invert `z = 1 - 2 * (index + 0.5) / count`, which falls with the index.
    let count = FIELD_COUNT as f64;
    let first = ((1.0 - upper) * count * 0.5 - 0.5).floor().max(0.0) as usize;
    let last = ((1.0 - lower) * count * 0.5 - 0.5).ceil().max(0.0) as usize;
    (first.min(FIELD_COUNT - 1), last.min(FIELD_COUNT - 1))
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
    let (first, last) = field_index_range(direction.y);
    let mut dominant_depth = 0.0_f64;
    let mut deepest = 0.0_f64;
    for crater in basins().iter().chain(&field()[first..=last]) {
        let here = contribution(crater, direction);
        if here < deepest {
            deepest = here;
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
    let (first, last) = field_index_range(direction.y);
    basins()
        .iter()
        .chain(&field()[first..=last])
        .map(|crater| contribution(crater, direction))
        .sum()
}

/// One crater's height at a direction, or zero if it does not reach.
fn contribution(crater: &Crater, direction: DVec3) -> f64 {
    let cosine = crater.axis.dot(direction).clamp(-1.0, 1.0);
    // Outside the ejecta blanket the profile is exactly zero, so this rejects
    // the crater without paying for the `acos`. It catches what survives the
    // latitude window but is far away in longitude. Mirrors `moon_raw_height`.
    if cosine <= crater.cosine_cutoff {
        return 0.0;
    }
    let t = cosine.acos() / crater.angular_radius;
    profile(t, crater.depth_meters, crater.rim_meters)
}

/// The catalogue and the profile, emitted as WGSL.
///
/// Generated rather than hand-written for the same reason the radius is: two
/// copies of forty-eight craters would be a divergence waiting to happen, and
/// the test that caught it would be policing a problem that need not exist.
pub fn wgsl_constants() -> String {
    let mut source = String::from("// Generated from moon.rs. Do not edit here.\n");
    source.push_str(&format!(
        "// {CRATER_COUNT} impacts: the basins are always tested, the field is windowed.\n\
         const MOON_BASIN_COUNT: u32 = {BASIN_COUNT}u;\n\
         const MOON_FIELD_COUNT: u32 = {FIELD_COUNT}u;\n"
    ));
    emit_catalogue(&mut source, "MOON_BASINS", basins());
    emit_catalogue(&mut source, "MOON_FIELD", field());
    let (window_sin, window_cos) = FIELD_WINDOW_RADIANS.sin_cos();
    source.push_str(&format!(
        "const MOON_FIELD_WINDOW_SIN: f32 = {window_sin:.9};\n\
         const MOON_FIELD_WINDOW_COS: f32 = {window_cos:.9};\n\
         const MOON_EJECTA_EXTENT: f32 = {EJECTA_EXTENT:.4};\n\
         const MOON_POLAR_ICE_DEPTH_FRACTION: f32 = {POLAR_ICE_DEPTH_FRACTION:.4};\n\
         const MOON_POLAR_ICE_LATITUDE_SINE_START: f32 = {POLAR_ICE_LATITUDE_SINE_START:.4};\n\
         const MOON_POLAR_ICE_LATITUDE_SINE_FULL: f32 = {POLAR_ICE_LATITUDE_SINE_FULL:.4};\n"
    ));
    source
}

/// One catalogue as a pair of WGSL arrays: axes with rim radius, and the
/// depths with the cutoff the loop rejects on.
fn emit_catalogue(source: &mut String, name: &str, craters: &[Crater]) {
    let count = craters.len();
    source.push_str(&format!(
        "// axis.xyz, rim radius in radians.\n\
         const {name}: array<vec4<f32>, {count}> = array<vec4<f32>, {count}>(\n"
    ));
    for crater in craters {
        source.push_str(&format!(
            "    vec4<f32>({:.9}, {:.9}, {:.9}, {:.9}),\n",
            crater.axis.x, crater.axis.y, crater.axis.z, crater.angular_radius
        ));
    }
    source.push_str(");\n");
    source.push_str(&format!(
        "// depth and rim height in metres, then the cosine cutoff.\n\
         const {name}_DEPTHS: array<vec3<f32>, {count}> = array<vec3<f32>, {count}>(\n"
    ));
    for crater in craters {
        source.push_str(&format!(
            "    vec3<f32>({:.4}, {:.4}, {:.9}),\n",
            crater.depth_meters, crater.rim_meters, crater.cosine_cutoff
        ));
    }
    source.push_str(");\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both catalogues, as one list. Every property that is true of a crater
    /// is true of a basin and a field crater alike.
    fn all_craters() -> Vec<Crater> {
        basins().iter().chain(field().iter()).copied().collect()
    }

    /// The exhaustive height: every crater tested, no window and no cutoff.
    /// This is the definition the fast path has to reproduce.
    fn exhaustive_raw_height_meters(direction: DVec3) -> f64 {
        let direction = direction.normalize();
        all_craters()
            .iter()
            .map(|crater| {
                let cosine = crater.axis.dot(direction).clamp(-1.0, 1.0);
                let t = cosine.acos() / crater.angular_radius;
                profile(t, crater.depth_meters, crater.rim_meters)
            })
            .sum()
    }

    /// A spread of directions that does not line up with the spiral, covering
    /// both poles, the equator and the latitudes in between.
    fn sample_directions() -> Vec<DVec3> {
        (0..2000)
            .map(|index| {
                let i = index as f64;
                let z = 1.0 - 2.0 * (i + 0.5) / 2000.0;
                let ring = (1.0 - z * z).max(0.0).sqrt();
                let theta = 2.399_963_23 * i + 0.37;
                DVec3::new(ring * theta.cos(), z, ring * theta.sin()).normalize()
            })
            .collect()
    }

    #[test]
    fn the_catalogue_is_deterministic_and_well_formed() {
        assert_eq!(basins().len(), BASIN_COUNT);
        assert_eq!(field().len(), FIELD_COUNT);
        assert_eq!(all_craters().len(), CRATER_COUNT);
        for (a, b) in basins().iter().zip(basins().iter()) {
            assert_eq!(a.axis, b.axis);
            assert_eq!(a.angular_radius, b.angular_radius);
        }
        for crater in all_craters() {
            assert!((crater.axis.length() - 1.0).abs() < 1.0e-12);
            assert!(crater.angular_radius > 0.0);
            assert!(crater.angular_radius <= MAX_ANGULAR_RADIUS);
            assert!(crater.depth_meters > 0.0);
            assert!(crater.rim_meters > 0.0 && crater.rim_meters < crater.depth_meters);
        }
    }

    /// `field_index_range` inverts the spiral's index-to-latitude mapping, so
    /// it is only correct while the field really is sorted by latitude.
    #[test]
    fn the_field_is_sorted_by_latitude() {
        for pair in field().windows(2) {
            assert!(
                pair[0].axis.y > pair[1].axis.y,
                "the window search needs a monotonic field",
            );
        }
    }

    /// Nothing in the windowed field may be big enough to reach outside the
    /// window, or the window would silently drop craters.
    #[test]
    fn the_field_stays_inside_the_window_it_assumes() {
        for crater in field() {
            assert!(
                EJECTA_EXTENT * crater.angular_radius <= FIELD_WINDOW_RADIANS + 1.0e-12,
                "a field crater reaches further than the window looks",
            );
        }
    }

    /// The load-bearing test for the whole optimisation: the fast path and the
    /// exhaustive definition must agree everywhere, not merely look similar.
    /// If the window or the cutoff ever drops a crater that contributes, this
    /// is what catches it.
    #[test]
    fn the_windowed_height_matches_testing_every_crater() {
        let mut worst = 0.0_f64;
        for direction in sample_directions() {
            let difference =
                (raw_height_meters(direction) - exhaustive_raw_height_meters(direction)).abs();
            worst = worst.max(difference);
        }
        assert!(
            worst < 1.0e-6,
            "the window changed the height field by {worst} metres",
        );
    }

    /// The window has to actually be narrow, or it is correct and useless.
    #[test]
    fn the_window_tests_a_small_fraction_of_the_field() {
        let mut worst = 0;
        let mut total = 0;
        let samples = sample_directions();
        for direction in &samples {
            let (first, last) = field_index_range(direction.y);
            let tested = last - first + 1;
            worst = worst.max(tested);
            total += tested;
        }
        let mean = total / samples.len();
        assert!(
            worst < FIELD_COUNT / 4,
            "the widest window tested {worst} of {FIELD_COUNT} craters",
        );
        assert!(
            mean < FIELD_COUNT / 8,
            "the average window tested {mean} of {FIELD_COUNT} craters",
        );
    }

    #[test]
    fn a_crater_is_a_bowl_inside_a_rim() {
        let crater = basins()[0];
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
        let crater = basins()[3];
        let antipode = -crater.axis;
        let cosine = crater.axis.dot(antipode).clamp(-1.0, 1.0);
        let t = cosine.acos() / crater.angular_radius;
        assert_eq!(profile(t, crater.depth_meters, crater.rim_meters), 0.0);
    }

    #[test]
    fn the_field_dips_inside_a_crater_and_is_finite_everywhere() {
        let crater = basins()[0];
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
        for crater in all_craters() {
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
        let polar = all_craters()
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

    /// The catalogue must be mostly small craters, or a few hundred of them
    /// simply merges into noise at the large end. This asserts the shape of
    /// the size law rather than any individual radius.
    #[test]
    fn most_impacts_are_small_and_the_basins_are_rare() {
        let mut radii: Vec<f64> = all_craters().iter().map(|c| c.angular_radius).collect();
        radii.sort_by(|a, b| a.partial_cmp(b).expect("radii are finite"));
        let median = radii[CRATER_COUNT / 2];
        assert!(
            median < 0.1 * MAX_ANGULAR_RADIUS,
            "the median crater is small against the largest basin, got {median}",
        );
        let large = radii
            .iter()
            .filter(|r| **r > 0.5 * MAX_ANGULAR_RADIUS)
            .count();
        assert!(
            (1..=4).contains(&large),
            "a handful of basins, not a field of them, got {large}",
        );
    }

    /// The cutoff must reject only craters whose profile is already zero, so
    /// it is an optimisation and not a change to the height field.
    #[test]
    fn the_cutoff_only_rejects_craters_that_contribute_nothing() {
        for crater in all_craters() {
            let outside = crater.cosine_cutoff.clamp(-1.0, 1.0).acos() / crater.angular_radius;
            assert!(
                outside >= EJECTA_EXTENT - 1.0e-9,
                "the cutoff sits at or beyond the end of the blanket",
            );
            // Not exactly zero: `cos` and `acos` do not round-trip, so the
            // cutoff can land a few ulp inside the end of the blanket, where
            // the cubic leaves a residue around 1e-32 metres.
            assert!(
                profile(outside, crater.depth_meters, crater.rim_meters) < 1.0e-9,
                "a crater rejected by the cutoff was contributing nothing",
            );
        }
    }

    #[test]
    fn the_generated_catalogue_carries_every_crater() {
        let generated = wgsl_constants();
        assert_eq!(generated.matches("    vec4<f32>(").count(), CRATER_COUNT);
        assert_eq!(generated.matches("    vec3<f32>(").count(), CRATER_COUNT);
        assert!(generated.contains(&format!("const MOON_BASIN_COUNT: u32 = {BASIN_COUNT}u;")));
        assert!(generated.contains(&format!("const MOON_FIELD_COUNT: u32 = {FIELD_COUNT}u;")));
    }
}

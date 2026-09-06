//! Baking the moon: the crater catalogue rendered into the working grid.
//!
//! The renderer evaluates craters *per sample*, which is why its catalogue is
//! only 360 — see `coretypes::moon`. Offline there is no such budget, so the
//! baked catalogue is 11,449, down to about 3km across. That is more craters
//! than the per-sample loop could ever afford, and it costs nothing at run
//! time because the result is tiles like any other world's.
//!
//! **The evaluation is inverted.** Asking each grid cell which craters reach it
//! would be 176 basins plus a ~345-crater window at every one of 33 million
//! cells: about 17 billion tests. Asking each *crater* which cells it covers is
//! the same answer for the work actually required — the total area of all the
//! blankets, around 9 million cell writes. A test holds the two against each
//! other, because the fast one is only worth having if it is the same field.

use catinthegarden_coretypes::{
    BiomeId,
    moon::{Catalogue, Crater, MOON_DATUM_METERS, MOON_RADIUS_METERS, profile},
};
use glam::DVec3;
use rayon::prelude::*;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::grid::SphericalGrid;

/// The moon's surface as the working grid sees it.
pub struct MoonSurface {
    /// Metres from the body's radius, already lifted by `MOON_DATUM_METERS`.
    pub height_meters: Vec<f64>,
    pub biome: Vec<BiomeId>,
    /// How much of a rotation each place spends in shadow, 0-255. Carried in
    /// the moisture channel, which an airless body has no other use for, and
    /// sampled bilinearly so the ice edge is a shape rather than a staircase.
    pub shadow_fraction: Vec<u8>,
}

/// Renders the catalogue into the grid by splatting each crater over the cells
/// it covers.
///
/// The height is the craters and nothing else. Where the ice goes is a separate
/// question, answered by `classify_ice` from shadow rather than from depth.
pub fn generate(grid: &SphericalGrid, catalogue: &Catalogue) -> MoonSurface {
    let cells = grid.len();
    let mut raw = vec![0.0_f64; cells];

    for crater in catalogue.iter() {
        splat(grid, crater, &mut raw);
    }

    let height_meters: Vec<f64> = raw.iter().map(|h| MOON_DATUM_METERS + h).collect();
    let (biome, shadow_fraction) = classify_ice(grid, &height_meters);

    MoonSurface {
        height_meters,
        biome,
        shadow_fraction,
    }
}

/// How many positions of the sun to test over one rotation.
///
/// The moon's axis is Y and its tilt is taken as zero, so the sun stays in the
/// equatorial plane and traces the same arc every rotation. Testing that arc is
/// therefore testing every moment there has ever been, which is what makes
/// "permanently shadowed" a computable property rather than a guess.
const SUN_SAMPLES: usize = 32;

/// Below this latitude sine, skip the march.
///
/// This was 0.80 -- 53 degrees -- justified by crater walls running to about 30
/// degrees, so that the sun would clear any rim below that. It drew a hard
/// arctic circle, which was reported, and the justification was wrong in the
/// direction that matters: overlapping rims and ejecta make slopes far steeper
/// than one crater's wall, and a deep basin's own rim is high enough to shadow
/// its floor well outside the polar circle.
///
/// 0.20 is 11.5 degrees, and only skips the deep tropics where the sun passes
/// within 11 degrees of vertical and nothing on this body can hide from it.
/// Where shadow is possible, the march decides -- which is the whole point.
const SHADOW_LATITUDE_SINE: f64 = 0.20;

/// How far to look for something blocking the sun, and in how many steps.
///
/// A crater 30km across can shadow its own floor from its far rim, so the reach
/// has to be a rim radius or two. Geometric spacing puts most of the samples
/// close, where the horizon is steepest and a near wall does the work.
const HORIZON_REACH_METERS: f64 = 40_000.0;
const HORIZON_STEPS: usize = 14;

/// Ice where the sun never reaches, regolith everywhere else.
///
/// This replaced a rule that filled polar crater floors to half their depth,
/// level, gated on latitude alone. That put ice in flat ponds in places chosen
/// by a formula rather than by the light. Real ice on an airless body survives
/// exactly where it is never heated: the floors and poleward walls of craters
/// near the poles, where the rim hides the sun through the whole rotation. It
/// does not have to be flat, and it is not level -- a shadowed wall is icy at
/// whatever angle the wall happens to lie at.
fn classify_ice(grid: &SphericalGrid, height_meters: &[f64]) -> (Vec<BiomeId>, Vec<u8>) {
    let suns: Vec<DVec3> = (0..SUN_SAMPLES)
        .map(|index| {
            let angle = TAU * index as f64 / SUN_SAMPLES as f64;
            // In the equatorial plane: zero tilt, so the sun never leaves it.
            DVec3::new(angle.cos(), 0.0, angle.sin())
        })
        .collect();

    let shadow: Vec<f64> = (0..grid.len())
        .into_par_iter()
        .map(|index| {
            let direction = grid.direction(index);
            if direction.y.abs() < SHADOW_LATITUDE_SINE {
                return 0.0;
            }
            let normal = surface_normal(grid, height_meters, index);
            let height = height_meters[index];
            let mut dark = 0usize;
            let mut daylight = 0usize;
            for sun in &suns {
                let sun_elevation = direction.dot(*sun).clamp(-1.0, 1.0).asin();
                if sun_elevation <= 0.0 {
                    // Night everywhere on the body at this moment, which says
                    // nothing about whether this place is ever lit.
                    continue;
                }
                daylight += 1;
                // Its own slope can hide it before any terrain does.
                if normal.dot(*sun) <= 0.0
                    || horizon_blocks(grid, height_meters, index, height, *sun, sun_elevation)
                {
                    dark += 1;
                }
            }
            if daylight == 0 {
                0.0
            } else {
                dark as f64 / daylight as f64
            }
        })
        .collect();

    // Stored as a continuous field, not a yes/no. Two reasons, both reported
    // from screenshots. The renderer samples it bilinearly, so an ice edge
    // follows the ground instead of the 828m grid -- the old boolean came back
    // as hard axis-aligned cells and plus-shapes. And near a pole the sun grazes,
    // so "is this facet lit" is a knife-edge that flips between neighbouring
    // cells: the boolean speckled, scattering ice across flat lit ground while
    // leaving genuinely shadowed crater floors bare. A fraction varies smoothly
    // across a crater and says how *nearly* permanent the shadow is.
    // Blurred before storing. The per-cell verdict is a knife-edge near a pole
    // -- neighbouring cells flip -- and a speckled field stays speckled under
    // bilinear sampling, which is what put ice in hard 828m blocks beside the
    // shadows instead of in them. The renderer uses this as a *regional* term,
    // "can ice hold anywhere near here", and pairs it with a per-fragment test
    // that resolves the actual ground. So it wants to be smooth.
    let smoothed = blur(grid, &shadow, SHADOW_BLUR_PASSES);
    let fraction: Vec<u8> = smoothed
        .iter()
        .map(|value| (value * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect();
    let biome = smoothed
        .iter()
        .map(|value| {
            if *value >= ICE_SHADOW_THRESHOLD {
                BiomeId::Ice
            } else {
                BiomeId::MountainRock
            }
        })
        .collect();
    (biome, fraction)
}

/// How many box passes smooth the shadow field. Three is enough to turn
/// per-cell speckle into a region without erasing a crater-sized feature.
const SHADOW_BLUR_PASSES: usize = 3;

/// Separable box blur over the grid, wrapping in longitude and clamping in
/// latitude, which is how the grid itself is addressed.
fn blur(grid: &SphericalGrid, values: &[f64], passes: usize) -> Vec<f64> {
    let mut current = values.to_vec();
    for _ in 0..passes {
        let next: Vec<f64> = (0..grid.len())
            .into_par_iter()
            .map(|index| {
                let mut total = current[index];
                let mut count = 1.0;
                for (dx, dy) in [(1_isize, 0_isize), (-1, 0), (0, 1), (0, -1)] {
                    if let Some(neighbor) = grid.offset_index(index, dx, dy) {
                        total += current[neighbor];
                        count += 1.0;
                    }
                }
                total / count
            })
            .collect();
        current = next;
    }
    current
}

/// How much of the rotation a place must spend in shadow to hold ice.
///
/// Not 1.0. A cell that catches the sun for one of thirty-two samples is a
/// cell the march resolved coarsely, not a place with a summer -- and the
/// renderer blends across this rather than cutting at it, so the threshold
/// decides identity for collision and material, while the look comes from the
/// stored fraction.
const ICE_SHADOW_THRESHOLD: f64 = 0.97;

/// Whether terrain between here and the horizon stands above the sun.
///
/// Marches along the great circle toward the sun's azimuth and compares each
/// sample's elevation angle, seen from this point, against the sun's own. The
/// curvature term matters: over tens of kilometres on a 1,080km body the ground
/// falls away, and ignoring it would invent blockers that are actually below
/// the horizon.
fn horizon_blocks(
    grid: &SphericalGrid,
    height_meters: &[f64],
    index: usize,
    height: f64,
    sun: DVec3,
    sun_elevation: f64,
) -> bool {
    let direction = grid.direction(index);
    // The sun's azimuth as a tangent vector: the part of the sun direction that
    // is not straight up.
    let along = (sun - direction * direction.dot(sun)).normalize_or_zero();
    if along.length_squared() < 0.5 {
        return false;
    }
    let observer_radius = MOON_RADIUS_METERS + height;
    for step in 1..=HORIZON_STEPS {
        let fraction = step as f64 / HORIZON_STEPS as f64;
        // Geometric spacing: dense near the observer, sparse far away.
        let distance = HORIZON_REACH_METERS * fraction * fraction;
        let angle = distance / MOON_RADIUS_METERS;
        let sample_direction = (direction * angle.cos() + along * angle.sin()).normalize();
        let sample_height = grid.sample_f64(height_meters, sample_direction);
        let sample_radius = MOON_RADIUS_METERS + sample_height;
        let rise = sample_radius * angle.cos() - observer_radius;
        let run = sample_radius * angle.sin();
        if run > 1.0 && (rise / run).atan() > sun_elevation {
            return true;
        }
    }
    false
}

/// Outward normal of the height field at one cell, from its neighbours.
fn surface_normal(grid: &SphericalGrid, height_meters: &[f64], index: usize) -> DVec3 {
    let direction = grid.direction(index);
    let mut gradient = DVec3::ZERO;
    for (dx, dy) in [(1_isize, 0_isize), (-1, 0), (0, 1), (0, -1)] {
        let Some(neighbor) = grid.offset_index(index, dx, dy) else {
            continue;
        };
        let separation = grid.distance_meters(index, neighbor);
        if separation <= 1.0 {
            continue;
        }
        let neighbor_direction = grid.direction(neighbor);
        let tangent = (neighbor_direction - direction * direction.dot(neighbor_direction))
            .normalize_or_zero();
        gradient += tangent * ((height_meters[neighbor] - height_meters[index]) / separation);
    }
    (direction - gradient).normalize()
}

/// Adds one crater to every grid cell inside its blanket.
///
/// The row range comes from the latitude band the crater reaches; within a row
/// the longitude half-width comes from the spherical law of cosines, so the
/// cells visited are the cap and not its bounding box. The contribution itself
/// is still the exact `profile` of the exact angular distance — this only
/// decides *which* cells to ask about.
fn splat(grid: &SphericalGrid, crater: &Crater, raw: &mut [f64]) {
    let reach = crater.reach_radians();
    let axis = crater.axis;
    let centre_latitude = axis.y.clamp(-1.0, 1.0).asin();
    let centre_longitude = axis.z.atan2(axis.x);
    let width = grid.width() as f64;
    let height = grid.height() as f64;

    // Rows: latitude(y) = ((y + 0.5) / height) * PI - PI/2.
    let first_row = (((centre_latitude - reach) + FRAC_PI_2) / PI * height - 0.5).floor();
    let last_row = (((centre_latitude + reach) + FRAC_PI_2) / PI * height - 0.5).ceil();
    let first_row = first_row.max(0.0) as usize;
    let last_row = (last_row.max(0.0) as usize).min(grid.height().saturating_sub(1));

    for y in first_row..=last_row {
        let latitude = ((y as f64 + 0.5) / height) * PI - FRAC_PI_2;
        // cos(reach) = sin(lat_c) sin(lat) + cos(lat_c) cos(lat) cos(dLon)
        let denominator = centre_latitude.cos() * latitude.cos();
        let half_width = if denominator.abs() < 1.0e-12 {
            PI
        } else {
            let cosine = (reach.cos() - centre_latitude.sin() * latitude.sin()) / denominator;
            if cosine <= -1.0 {
                PI
            } else if cosine >= 1.0 {
                continue;
            } else {
                cosine.acos()
            }
        };

        let first_column = ((centre_longitude - half_width + PI) / TAU * width - 0.5).floor();
        let last_column = ((centre_longitude + half_width + PI) / TAU * width - 0.5).ceil();
        let span = (last_column - first_column) as isize;
        if span < 0 {
            continue;
        }
        // A wide enough cap wraps the whole row; clamping the span keeps a
        // near-polar crater from walking the row more than once.
        let span = span.min(grid.width() as isize - 1);
        let base = first_column as isize;
        for step in 0..=span {
            let x = (base + step).rem_euclid(grid.width() as isize) as usize;
            let index = grid.index(x, y);
            let cosine = axis.dot(grid.direction(index)).clamp(-1.0, 1.0);
            if cosine <= crater.cosine_cutoff {
                continue;
            }
            let contribution = profile(
                cosine.acos() / crater.angular_radius,
                crater.depth_meters,
                crater.rim_meters,
            );
            raw[index] += contribution;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catinthegarden_coretypes::moon::{CatalogueSpec, MOON_RADIUS_METERS};

    fn small_catalogue() -> Catalogue {
        Catalogue::new(CatalogueSpec {
            basin_count: 5,
            field_count: 120,
            basin_max_angular_radius: 0.30,
            field_max_angular_radius: 0.30,
            basin_rank_exponent: 0.5,
            field_rank_exponent: 0.5,
            min_angular_radius: 0.01,
            small_size_spread: 0.0,
            position_jitter: 1.0,
            field_rank_offset: 5,
        })
    }

    /// The load-bearing test for the whole splat: it has to produce exactly
    /// the field the per-sample evaluation does, cell for cell. If the row
    /// range, the longitude half-width, or the wrap is wrong, the two diverge
    /// here rather than as a subtle artefact in a 372MB bake.
    #[test]
    fn splatting_matches_evaluating_every_cell() {
        let grid = SphericalGrid::new(128, 64);
        let catalogue = small_catalogue();
        let surface = generate(&grid, &catalogue);
        let mut worst = 0.0_f64;
        for index in 0..grid.len() {
            let direction = grid.direction(index);
            let expected = catalogue.surface_height_meters(direction);
            worst = worst.max((surface.height_meters[index] - expected).abs());
        }
        assert!(
            worst < 1.0e-6,
            "splatting disagreed with per-sample evaluation by {worst} metres",
        );
    }

    /// Ice is a shadow question now, so the test is about shadow: a deep pit at
    /// the pole keeps its floor dark through every sun position, and the same
    /// pit at the equator does not, because there the sun climbs overhead.
    ///
    /// Constructed rather than taken from the catalogue, so it fails for the
    /// reason it names instead of tracking whatever the crater field happens to
    /// look like this week.
    #[test]
    fn ice_survives_only_where_the_sun_never_reaches() {
        let grid = SphericalGrid::with_radius(512, 256, MOON_RADIUS_METERS);
        let pit = |centre: DVec3, index: usize| {
            let angle = centre.dot(grid.direction(index)).clamp(-1.0, 1.0).acos();
            let radius = 0.02_f64;
            if angle >= radius {
                MOON_DATUM_METERS
            } else {
                // A bowl 6km deep and 21km across: steep enough that a rim
                // hides its floor from a sun that never rises far.
                MOON_DATUM_METERS - 6_000.0 * (1.0 - (angle / radius).powi(2))
            }
        };
        let polar = DVec3::Y;
        let polar_field: Vec<f64> = (0..grid.len()).map(|i| pit(polar, i)).collect();
        let (polar_biomes, _) = classify_ice(&grid, &polar_field);
        let polar_floor = (0..grid.len())
            .min_by(|a, b| polar_field[*a].total_cmp(&polar_field[*b]))
            .expect("the grid is not empty");
        assert_eq!(
            polar_biomes[polar_floor],
            BiomeId::Ice,
            "a deep polar pit floor is never lit",
        );

        let equatorial = DVec3::X;
        let equatorial_field: Vec<f64> = (0..grid.len()).map(|i| pit(equatorial, i)).collect();
        let (equatorial_biomes, _) = classify_ice(&grid, &equatorial_field);
        let equatorial_floor = (0..grid.len())
            .min_by(|a, b| equatorial_field[*a].total_cmp(&equatorial_field[*b]))
            .expect("the grid is not empty");
        assert_eq!(
            equatorial_biomes[equatorial_floor],
            BiomeId::MountainRock,
            "the same pit at the equator gets the sun straight down it",
        );
    }

    /// Flat ground at the pole is lit: the sun grazes it, but grazing is not
    /// shadow. Only terrain makes shadow, which is the whole point of the
    /// change -- a latitude rule alone would ice this.
    #[test]
    fn a_flat_pole_is_not_ice() {
        let grid = SphericalGrid::with_radius(256, 128, MOON_RADIUS_METERS);
        let flat = vec![MOON_DATUM_METERS; grid.len()];
        let (biomes, _) = classify_ice(&grid, &flat);
        let pole = (0..grid.len())
            .max_by(|a, b| grid.direction(*a).y.total_cmp(&grid.direction(*b).y))
            .expect("the grid is not empty");
        assert_eq!(biomes[pole], BiomeId::MountainRock);
    }

    /// The datum has to keep the whole body inside what the height channel can
    /// store, or the deepest basins are clipped flat.
    #[test]
    fn the_baked_surface_stays_inside_the_stored_height_range() {
        let grid = SphericalGrid::new(512, 256);
        let surface = generate(&grid, catalogue_for_test());
        let lowest = surface
            .height_meters
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let highest = surface
            .height_meters
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            lowest > crate::terrain::MIN_HEIGHT_METERS,
            "the deepest basin floor is at {lowest}m, below the stored floor",
        );
        assert!(
            highest < crate::terrain::MAX_HEIGHT_METERS,
            "the highest rim is at {highest}m, above the stored ceiling",
        );
        // And nothing may reach zero, which every part of the renderer that
        // predates this body reads as sea level.
        assert!(
            lowest > 0.0,
            "the moon dips to {lowest}m, at or below datum"
        );
    }

    /// Not an assertion -- run with `--ignored --nocapture` to read the real
    /// extremes at the resolution the bake actually uses, which is how
    /// `MOON_DATUM_METERS` was chosen.
    #[test]
    #[ignore]
    fn report_the_baked_height_extremes() {
        let grid = SphericalGrid::with_radius(
            crate::config::MOON_WORKING_WIDTH,
            crate::config::MOON_WORKING_HEIGHT,
            MOON_RADIUS_METERS,
        );
        let started = std::time::Instant::now();
        let surface = generate(&grid, catalogue_for_test());
        let lowest = surface
            .height_meters
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let highest = surface
            .height_meters
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let icy = surface
            .biome
            .iter()
            .filter(|biome| **biome == BiomeId::Ice)
            .count();
        println!(
            "moon splat: {:.1}s, {} cells, height {lowest:.1}..{highest:.1}m, {icy} ice cells ({:.3}%)",
            started.elapsed().as_secs_f64(),
            grid.len(),
            100.0 * icy as f64 / grid.len() as f64,
        );
    }

    fn catalogue_for_test() -> &'static Catalogue {
        catinthegarden_coretypes::moon::baked()
    }

    /// A size-frequency law is only doing its job if most craters are small.
    /// A floor breaks that -- everything below it becomes one size, and the
    /// surface reads as one grade of crater. This asserts the shape survives.
    #[test]
    fn most_craters_are_below_the_grid_and_a_few_are_basins() {
        let catalogue = catalogue_for_test();
        let cell_meters = TAU * MOON_RADIUS_METERS / crate::config::MOON_WORKING_WIDTH as f64;
        let mut below = 0usize;
        let mut basins = 0usize;
        for crater in catalogue.iter() {
            let across = 2.0 * crater.angular_radius * MOON_RADIUS_METERS;
            if across < cell_meters * 2.0 {
                below += 1;
            }
            if across > 100_000.0 {
                basins += 1;
            }
        }
        assert!(
            below > catalogue.len() / 2,
            "only {below} of {} craters are finer than the grid; the law has a floor in it",
            catalogue.len(),
        );
        assert!(
            (1..=60).contains(&basins),
            "{basins} craters over 100km across is not a handful of basins",
        );
    }
}

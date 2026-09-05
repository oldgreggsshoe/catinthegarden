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
    moon::{Catalogue, Crater, MOON_DATUM_METERS, POLAR_ICE_DEPTH_FRACTION, polar_weight, profile},
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::grid::SphericalGrid;

/// The moon's surface as the working grid sees it.
pub struct MoonSurface {
    /// Metres from the body's radius, already lifted by `MOON_DATUM_METERS`.
    pub height_meters: Vec<f64>,
    pub biome: Vec<BiomeId>,
}

/// Renders the catalogue into the grid by splatting each crater over the cells
/// it covers.
///
/// Alongside the summed height it tracks, per cell, the single deepest crater
/// contribution and that crater's depth. That is what the polar ice needs: the
/// pond level is half the depth of the crater that *dominates* the point, which
/// is constant within one crater and so gives a flat pond rather than a coat of
/// paint following the bowl down.
pub fn generate(grid: &SphericalGrid, catalogue: &Catalogue) -> MoonSurface {
    let cells = grid.len();
    let mut raw = vec![0.0_f64; cells];
    let mut deepest = vec![0.0_f64; cells];
    let mut dominant_depth = vec![0.0_f64; cells];

    for crater in catalogue.iter() {
        splat(grid, crater, &mut raw, &mut deepest, &mut dominant_depth);
    }

    let mut height_meters = Vec::with_capacity(cells);
    let mut biome = Vec::with_capacity(cells);
    for index in 0..cells {
        let direction = grid.direction(index);
        let polar = polar_weight(direction);
        let ice_surface = -POLAR_ICE_DEPTH_FRACTION * dominant_depth[index] * polar;
        // Away from the poles there is no ice and the bowl is left as the
        // impact dug it; filling to the datum there would flatten every crater
        // on the body into a disc.
        let filled = if ice_surface < 0.0 {
            raw[index].max(ice_surface)
        } else {
            raw[index]
        };
        // Ice only where the impact actually dug below the level it ponds at.
        let icy = ice_surface < 0.0 && raw[index] < ice_surface;
        height_meters.push(MOON_DATUM_METERS + filled);
        biome.push(if icy {
            BiomeId::Ice
        } else {
            BiomeId::MountainRock
        });
    }

    MoonSurface {
        height_meters,
        biome,
    }
}

/// Adds one crater to every grid cell inside its blanket.
///
/// The row range comes from the latitude band the crater reaches; within a row
/// the longitude half-width comes from the spherical law of cosines, so the
/// cells visited are the cap and not its bounding box. The contribution itself
/// is still the exact `profile` of the exact angular distance — this only
/// decides *which* cells to ask about.
fn splat(
    grid: &SphericalGrid,
    crater: &Crater,
    raw: &mut [f64],
    deepest: &mut [f64],
    dominant_depth: &mut [f64],
) {
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
            if contribution < deepest[index] {
                deepest[index] = contribution;
                dominant_depth[index] = crater.depth_meters;
            }
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

    /// And the biome has to agree too, since the ice depends on which single
    /// crater dominates rather than on the summed height.
    #[test]
    fn splatting_agrees_about_which_floors_are_ice() {
        let grid = SphericalGrid::new(128, 64);
        let catalogue = small_catalogue();
        let surface = generate(&grid, &catalogue);
        for index in 0..grid.len() {
            let direction = grid.direction(index);
            let expected = if catalogue.is_ice_at(direction) {
                BiomeId::Ice
            } else {
                BiomeId::MountainRock
            };
            assert_eq!(surface.biome[index], expected, "at cell {index}");
        }
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

    /// The moon's craters have to be big enough for the grid to hold. A crater
    /// only a cell or two across is a spike, not a bowl.
    #[test]
    fn the_smallest_crater_spans_several_working_cells() {
        let catalogue = catalogue_for_test();
        let smallest = catalogue
            .iter()
            .map(|crater| crater.angular_radius)
            .fold(f64::INFINITY, f64::min);
        let cell_meters = TAU * MOON_RADIUS_METERS / crate::config::MOON_WORKING_WIDTH as f64;
        let cells_across = 2.0 * smallest * MOON_RADIUS_METERS / cell_meters;
        assert!(
            cells_across >= 4.0,
            "the smallest crater is {cells_across:.1} cells across",
        );
    }
}

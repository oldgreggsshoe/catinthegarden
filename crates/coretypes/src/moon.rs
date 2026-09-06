//! The moon's macro height field: impact craters, not eroded terrain.
//!
//! The planet's geography is baked because erosion, hydrology and climate need
//! a pipeline. A moon needs none of that — an airless, dry body's large-scale
//! shape is overwhelmingly its impact history — so the catalogue here *is* the
//! generator, and it is deliberately usable from either side of the bake:
//!
//! * the **baker** evaluates a large catalogue offline and writes tiles, which
//!   is how the moon gets thousands of craters for no per-frame cost;
//! * the **renderer** evaluates a small one per sample, as the placeholder
//!   terrain shown when no bake is present, and emits it into the shader.
//!
//! That is why this lives in `coretypes` rather than in either crate: both
//! must describe the same body, and a second copy of the profile would be a
//! divergence waiting to happen.
//!
//! The profile is smooth trigonometry and polynomials, with no hashing and no
//! `fract`: a CPU/GPU field folded by `fract` is either bit-identical or
//! unrelated, and this one is evaluated in f64 on one side and f32 on the
//! other.

use glam::DVec3;

/// A moon at roughly a quarter of the planet's radius, which is the
/// Earth/Luna ratio.
pub const MOON_RADIUS_METERS: f64 = 1_080_000.0;

/// How far the pristine surface sits above the height field's zero.
///
/// The crater field is naturally centred on zero — untouched ground is exactly
/// 0.0 and every floor is below it — but a baked outmap cannot store that. Its
/// height channel bottoms out at -5,000m, which would clip the 10km basins
/// flat, and zero means sea level to every part of the renderer that predates
/// this body. Lifting the whole surface puts the moon comfortably inside the
/// stored range and entirely above the datum, so nothing anywhere has to learn
/// that this world's mean surface is its zero. Applied by
/// `surface_height_meters`, which is what both the bake and the shader draw.
///
/// Sized from the measurement, not from one basin's depth: six hundred
/// thousand craters overlap heavily, and stacked bowls reach 19,256m below the
/// crater field's zero where a single largest basin reaches only 10,125m. So
/// this is a property of the *catalogue* and not of any one crater: raising the
/// count deepens the stack, and `report_the_baked_height_extremes` in the baker
/// is what to re-run after changing it. At 22,000m the baked body spans 2,744m
/// to 26,993m, inside the stored range at both ends and above zero everywhere.
///
/// The measurement is exact rather than a sample, because it is taken on the
/// same grid the bake writes: what it reports is what gets stored.
pub const MOON_DATUM_METERS: f64 = 22_000.0;

/// Where the planet sits in the moon's sky, as a body-fixed unit direction.
///
/// A tidally locked moon keeps one face toward what it orbits, so the planet
/// does not rise or set: it hangs motionless overhead at the sub-planet point
/// while the sun goes round. That is why this is a constant and not an orbit —
/// no orbit exists yet, and for the near side's lighting none is needed.
///
/// Chosen on the equator at the baked landing site's longitude, so the planet
/// is high in the sky from the place the game starts. On the far side it never
/// rises at all, and `earthshine_irradiance` returns nothing there, which is
/// correct rather than a limitation.
pub const MOON_PLANET_SKY_DIRECTION: [f64; 3] = [-0.254_534_6, 0.0, -0.967_063_4];

/// How much of direct sunlight reaches the surface as planetshine, at full
/// phase and with the planet overhead.
///
/// The literal figure is about 1.2e-4: seen from the moon the planet is roughly
/// ten magnitudes fainter than the sun. At that value it is invisible here —
/// the night side quantises to zero, because this renderer has a fixed exposure
/// and no eye adaptation, and a real observer's night vision is most of why
/// earthshine looks as bright as it does. So this is about ten times the
/// literal ratio, which is an exposure decision written down rather than a
/// claim about photometry.
pub const MOON_PLANETSHINE_FRACTION: f64 = 0.001_2;

/// Planetshine is the colour of the planet: ocean and Rayleigh sky, so blue.
pub const MOON_PLANETSHINE_COLOUR: [f64; 3] = [0.55, 0.70, 1.0];

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
    /// crater cannot reach a sample. Precomputed so a loop can reject a
    /// distant crater with a comparison instead of an `acos`.
    pub cosine_cutoff: f64,
}

impl Crater {
    /// How far this crater reaches from its own centre, in radians.
    pub fn reach_radians(&self) -> f64 {
        EJECTA_EXTENT * self.angular_radius
    }

    /// This crater's height at a direction, or zero if it does not reach.
    pub fn contribution_meters(&self, direction: DVec3) -> f64 {
        let cosine = self.axis.dot(direction).clamp(-1.0, 1.0);
        if cosine <= self.cosine_cutoff {
            return 0.0;
        }
        profile(
            cosine.acos() / self.angular_radius,
            self.depth_meters,
            self.rim_meters,
        )
    }
}

/// Fresh craters run about a fifth as deep as they are wide; large basins
/// relax much shallower than that, which is why depth is not simply
/// proportional to radius.
///
/// 0.25 rather than the 0.20 a fresh terrestrial crater gives, a quarter deeper
/// by choice: on a body whose only relief is impacts, shallow craters read as
/// dimples and the shadows they cast are what make the surface legible at all.
const DEPTH_TO_RADIUS: f64 = 0.25;
const RIM_TO_DEPTH: f64 = 0.28;

/// How far the ejecta reaches past the rim crest, as a multiple of the rim
/// radius. A real blanket is thin and close; 2.0 made a plateau, not a skirt.
pub const EJECTA_EXTENT: f64 = 1.35;

// Ice used to be a *fill*: crater floors near the poles ponded flat to half
// their depth, gated on latitude alone. That is not where ice is. It survives
// where sunlight never reaches -- the floors and the poleward walls of craters
// near the poles, permanently shadowed because the sun never climbs above the
// rim. Which places those are is a question about shadow, so it is answered by
// `baker::moon::permanently_shadowed`, from the terrain and the sun's actual
// track. Nothing here fills anything, and the surface is just the craters.

/// Seeds the shuffle that assigns sizes to positions.
///
/// This was a modular stride: `index * 197 + 89 % count`, which visits every
/// rank exactly once and looks like decorrelation. It is not. For any narrow
/// band of ranks -- which is to say any one size of crater -- the indices
/// carrying it form an *arithmetic progression*, and an arithmetic progression
/// on a golden-angle spiral is exactly what draws phyllotactic arms. So every
/// size class sat on its own regular lattice, and the medium craters were
/// visibly in rows on the finished body.
///
/// Position jitter did not save it. That displaces by one neighbour spacing,
/// and a size class is spaced tens of spacings apart, so its lattice survived
/// intact underneath. A shuffle has no such structure to survive.
const SIZE_RANK_SEED: u64 = 0x5EED_C0FF_EE15_600D;

/// How a catalogue is shaped: two tiers, because the renderer's latitude
/// window is only narrow if everything inside it is small.
#[derive(Clone, Copy, Debug)]
pub struct CatalogueSpec {
    /// Large impacts, tested at every sample because they reach too far to
    /// window usefully.
    pub basin_count: usize,
    /// Small impacts, sorted by latitude and tested through a window.
    pub field_count: usize,
    /// Radius of the largest basin, in radians.
    pub basin_max_angular_radius: f64,
    /// Radius the field's law is measured from, in radians. With a non-zero
    /// `field_rank_offset` this is the law's rank-1 radius rather than any
    /// crater that exists, so the largest field crater is smaller than it.
    pub field_max_angular_radius: f64,
    /// Exponents of the size-frequency law, as radius against rank: the `r`th
    /// largest has radius `max * r.powf(-exponent)`. A real cumulative
    /// distribution runs about `N(>R) ∝ R^-2`, which is an exponent of 0.5.
    pub basin_rank_exponent: f64,
    pub field_rank_exponent: f64,
    /// Smallest crater the catalogue will produce, in radians. The power law
    /// runs past it into sizes too small for the grid to hold, so the tail is
    /// resized rather than dropped.
    pub min_angular_radius: f64,
    /// How much larger than the floor a resized tail crater may be.
    ///
    /// The tail used to be *clamped*, so every one of the 582,000 craters past
    /// the law's reach came out at exactly the floor. Identical discs, on a
    /// spiral whose spacing is 4.94km, at 4.8km across: they tiled the surface,
    /// and a saturated tiling of identical circles reads as woven fabric. One
    /// spacing of position jitter cannot hide that, because the regularity is
    /// in the *sizes*, not the places.
    ///
    /// Spreading them over floor..floor*(1 + this) breaks the tiling, at the
    /// cost of a size-frequency law that is already fiction below the floor.
    pub small_size_spread: f64,
    /// How far a crater is displaced from its lattice position, as a fraction
    /// of the spacing between neighbours.
    ///
    /// A golden-angle spiral is *too* even: at these counts the phyllotactic
    /// arms are plainly visible as a grid, which is not what an impact history
    /// looks like. Impacts are independent events, so the positions want to be
    /// Poisson rather than lattice. This is not a lattice with noise on top at
    /// full strength -- at 1.0 the displacement is the whole spacing and the
    /// arms are gone.
    pub position_jitter: f64,
    /// Where the field's ranks start.
    ///
    /// Zero gives the two tiers independent laws, which is what the runtime
    /// catalogue wants: its field is deliberately steeper and finer than a
    /// continuation of its basins would be, because it has only 352 craters to
    /// spend and wants texture out of them. Setting it to `basin_count` makes
    /// the whole catalogue *one* law instead, which is what the baked
    /// catalogue wants — there the count is large enough for the real
    /// distribution to be worth following exactly.
    pub field_rank_offset: usize,
}

/// What the renderer evaluates per sample, and emits into the shader.
///
/// Small on purpose: every crater here is tested per terrain vertex and per
/// fragment. This is the placeholder terrain, shown when the moon has no bake.
pub const RUNTIME_SPEC: CatalogueSpec = CatalogueSpec {
    basin_count: 8,
    field_count: 352,
    basin_max_angular_radius: 0.30,
    field_max_angular_radius: 0.062,
    basin_rank_exponent: 0.72,
    field_rank_exponent: 0.45,
    min_angular_radius: 0.0,
    small_size_spread: 0.0,
    position_jitter: 1.6,
    field_rank_offset: 0,
};

/// What the baker evaluates offline and writes into tiles.
///
/// Large on purpose: the cost is paid once, so the count is set by what the
/// outmap can resolve rather than by the frame budget. The smallest crater
/// here is about 3.2km, a few samples across at the working grid's spacing;
/// anything finer belongs to the renderer's detail ladder, exactly as it does
/// on the planet.
///
/// The law's range follows from the resolution rather than being picked. A
/// cumulative `N(>R) ∝ R^-2` from 324km down to the 2.4km floor is a
/// hundred-and-thirty-fold in radius and so about eighteen thousand craters.
/// Past that the law would ask for sizes the grid cannot hold, and the count
/// instead goes into more craters *at* the floor — which is what a saturated
/// regolith surface is. 600,176 in total, so all but about three per cent of
/// them are floor-sized: the count buys density, not new sizes.
///
/// The split at rank 176 is where the always-tested and windowed costs
/// balance for the renderer. The bake does not use the window at all: it
/// splats, so its cost is the craters' total area and not the count.
pub const BAKED_SPEC: CatalogueSpec = CatalogueSpec {
    basin_count: 176,
    field_count: 600_000,
    basin_max_angular_radius: 0.30,
    field_max_angular_radius: 0.30,
    basin_rank_exponent: 0.5,
    field_rank_exponent: 0.5,
    // No floor. Any floor piles every crater below it into one size, and that
    // is what a size-frequency law is *for*: at 2.4km, 598,536 of the 600,176
    // ended up between 2 and 8km across, and the surface read as one grade of
    // crater everywhere. Spreading the pile only widened the band it bunched
    // in. Craters below the working cell are not resolved as craters, but they
    // are not spikes either -- they are the fine dimpling the ground should
    // have, and letting them through is what keeps the distribution honest:
    // 573,933 under 2km, 24,603 between 2 and 8, 1,640 above.
    min_angular_radius: 0.0,
    small_size_spread: 0.0,
    position_jitter: 1.6,
    // One continuous law across both tiers: the field picks up at rank 177,
    // exactly where the basins stop.
    field_rank_offset: 176,
};

/// A generated impact history: basins tested in full, field windowed.
#[derive(Clone, Debug)]
pub struct Catalogue {
    basins: Vec<Crater>,
    field: Vec<Crater>,
    field_window_radians: f64,
    /// How far the jittered field can sit from where an evenly spaced spiral
    /// would put it, in indices. `field_index_range` inverts the even spacing,
    /// so it widens its answer by this much; measured at build time rather
    /// than assumed, so no amount of jitter can silently drop a crater.
    field_index_slack: usize,
}

impl Catalogue {
    /// Builds both tiers. Deterministic: no randomness and no seed to drift,
    /// so the same catalogue is produced on every run and on both sides of
    /// the bake.
    pub fn new(spec: CatalogueSpec) -> Self {
        let basins = build_tier(
            spec,
            spec.basin_count,
            spec.basin_max_angular_radius,
            spec.basin_rank_exponent,
            0,
        );
        let mut field = build_tier(
            spec,
            spec.field_count,
            spec.field_max_angular_radius,
            spec.field_rank_exponent,
            spec.field_rank_offset,
        );
        // The window search needs the field ordered by latitude, and jitter
        // moves craters across each other. Sorting restores the invariant the
        // spiral gave for free; `total_cmp` keeps it a total order.
        field.sort_by(|a, b| b.axis.y.total_cmp(&a.axis.y));
        let count = field.len() as f64;
        let field_index_slack = field
            .iter()
            .enumerate()
            .map(|(index, crater)| {
                let predicted = ((1.0 - crater.axis.y) * count * 0.5 - 0.5).round();
                (index as f64 - predicted).abs() as usize
            })
            .max()
            .unwrap_or(0);
        // Measured from the craters rather than taken from the spec, so the
        // window cannot be narrower than what it has to cover however the tier
        // was parameterised.
        let field_window_radians = field
            .iter()
            .map(Crater::reach_radians)
            .fold(0.0_f64, f64::max);
        Self {
            field_window_radians,
            field_index_slack,
            basins,
            field,
        }
    }

    pub fn basins(&self) -> &[Crater] {
        &self.basins
    }

    /// The windowed field, in descending latitude order. The order is not
    /// cosmetic: `field_index_range` inverts it, and a test asserts it.
    pub fn field(&self) -> &[Crater] {
        &self.field
    }

    pub fn field_window_radians(&self) -> f64 {
        self.field_window_radians
    }

    pub fn len(&self) -> usize {
        self.basins.len() + self.field.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every crater, largest tier first.
    pub fn iter(&self) -> impl Iterator<Item = &Crater> {
        self.basins.iter().chain(self.field.iter())
    }

    /// The slice of `field` that can possibly reach a sample at this latitude
    /// sine, as an inclusive index range.
    ///
    /// Latitude is 1-Lipschitz on the sphere: two directions are at least as
    /// far apart as their latitudes are. So a crater whose latitude differs by
    /// more than its own reach cannot touch the sample, and since the field is
    /// sorted by latitude that leaves one contiguous run. Conservative, never
    /// wrong — the exhaustive comparison in the tests is what proves it.
    ///
    /// Mirrored by `moon_field_index_range` in the shader.
    pub fn field_index_range(&self, latitude_sine: f64) -> (usize, usize) {
        if self.field.is_empty() {
            return (0, 0);
        }
        let sine = latitude_sine.clamp(-1.0, 1.0);
        let cosine = (1.0 - sine * sine).max(0.0).sqrt();
        let (window_sin, window_cos) = self.field_window_radians.sin_cos();
        // sin and cos of (latitude +- window) by the angle-sum identities, so
        // no `asin` is needed. Past a pole the window wraps over it and the
        // bound becomes the pole itself, which the sign of the cosine detects.
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
        // Invert `z = 1 - 2 * (index + 0.5) / count`, which falls with index.
        let count = self.field.len() as f64;
        let first = ((1.0 - upper) * count * 0.5 - 0.5).floor().max(0.0) as usize;
        let last = ((1.0 - lower) * count * 0.5 - 0.5).ceil().max(0.0) as usize;
        let end = self.field.len() - 1;
        (
            first.saturating_sub(self.field_index_slack).min(end),
            last.saturating_add(self.field_index_slack).min(end),
        )
    }

    /// The impact field before the ice fills it, in metres about the datum.
    pub fn raw_height_meters(&self, direction: DVec3) -> f64 {
        let direction = direction.normalize();
        let (first, last) = self.field_index_range(direction.y);
        self.basins
            .iter()
            .chain(&self.field[first..=last])
            .map(|crater| crater.contribution_meters(direction))
            .sum()
    }

    /// The surface that is drawn and stood on, in metres from the body's
    /// radius. This is what the bake stores and the shader displaces by.
    ///
    /// Just the craters. Ice is a material laid on this surface where the sun
    /// never reaches, not a level that fills it in -- so a shadowed slope is
    /// icy at whatever angle it happens to lie at.
    pub fn surface_height_meters(&self, direction: DVec3) -> f64 {
        MOON_DATUM_METERS + self.raw_height_meters(direction)
    }
}

/// The catalogue the renderer and the shader share.
pub fn runtime() -> &'static Catalogue {
    static CATALOGUE: std::sync::LazyLock<Catalogue> =
        std::sync::LazyLock::new(|| Catalogue::new(RUNTIME_SPEC));
    &CATALOGUE
}

/// The catalogue the baker writes into tiles.
pub fn baked() -> &'static Catalogue {
    static CATALOGUE: std::sync::LazyLock<Catalogue> =
        std::sync::LazyLock::new(|| Catalogue::new(BAKED_SPEC));
    &CATALOGUE
}

/// One tier: centres on a golden-angle spiral, sizes by a power law on rank.
/// A deterministic 64-bit mix. Used only to place craters, and only on the
/// CPU: the catalogue reaches the GPU as literals and the bake as tiles, so
/// nothing here has to be reproduced in a shader. That is what makes an
/// integer hash safe to use at all — the rule against `fract` hashing applies
/// to fields both sides evaluate, and this is not one.
fn mix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn unit_from_hash(value: u64) -> f64 {
    (value >> 11) as f64 / (1_u64 << 53) as f64
}

fn build_tier(
    spec: CatalogueSpec,
    count: usize,
    max_angular_radius: f64,
    rank_exponent: f64,
    rank_offset: usize,
) -> Vec<Crater> {
    // Every rank used exactly once, in an order with no arithmetic structure.
    // Fisher-Yates, seeded rather than random, so the catalogue is still the
    // same on every run and on both sides of the bake.
    let mut ranks: Vec<usize> = (0..count).collect();
    for position in (1..count).rev() {
        let draw = mix64(SIZE_RANK_SEED ^ (position as u64).wrapping_mul(0x9E37_79B9))
            % (position as u64 + 1);
        ranks.swap(position, draw as usize);
    }
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..count)
        .map(|index| {
            let i = index as f64;
            let z = 1.0 - 2.0 * (i + 0.5) / count as f64;
            let ring_radius = (1.0 - z * z).max(0.0).sqrt();
            let theta = golden_angle * i;
            let lattice =
                DVec3::new(ring_radius * theta.cos(), z, ring_radius * theta.sin()).normalize();
            // Displace off the lattice by up to a full neighbour spacing, in a
            // uniformly random direction in the tangent plane. `sqrt` on the
            // radius keeps the offsets area-uniform inside that disc rather
            // than piling them up at the centre.
            let seed = mix64((index as u64) << 8 ^ (count as u64).wrapping_mul(0x2545_F491));
            let angle = unit_from_hash(seed) * std::f64::consts::TAU;
            let spacing = (4.0 * std::f64::consts::PI / count as f64).sqrt();
            let offset = spec.position_jitter * spacing * unit_from_hash(mix64(seed)).sqrt();
            let (tangent, bitangent) = tangent_basis(lattice);
            let axis =
                (lattice + (tangent * angle.cos() + bitangent * angle.sin()) * offset).normalize();

            // Size by rank, not by a smooth function of the index: this gives
            // an exact size-frequency law with each rank used once, rather
            // than whatever histogram a sine mix happens to have.
            let rank = rank_offset + ranks[index] + 1;
            let law = max_angular_radius * (rank as f64).powf(-rank_exponent);
            // Past the floor the law is asking for craters the grid cannot
            // hold. Resize rather than clamp: clamping made them all identical,
            // and identical craters at the spiral's own spacing tile.
            let angular_radius = if law >= spec.min_angular_radius {
                law
            } else {
                let draw = unit_from_hash(mix64(SIZE_RANK_SEED ^ (index as u64) << 17));
                spec.min_angular_radius * (1.0 + spec.small_size_spread * draw)
            };

            // Big basins relax: depth grows with radius but sub-linearly.
            let radius_meters = angular_radius * MOON_RADIUS_METERS;
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

/// The crater profile, as a function of angular distance over rim radius.
///
/// `t = 0` is the centre, `t = 1` the rim crest. Inside the rim it is a bowl;
/// outside, the ejecta blanket decays to nothing by `EJECTA_EXTENT`, so a
/// crater has bounded influence and the field stays local. Returns metres
/// relative to the surrounding datum, negative inside.
pub fn profile(t: f64, depth_meters: f64, rim_meters: f64) -> f64 {
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
    // real thing. It was a smoothstep out to twice the rim radius, which is
    // not a blanket but a raised plateau half the crater wide again, and it
    // read as a huge bright ring around every impact.
    let outer = ((t - 1.0) / (EJECTA_EXTENT - 1.0)).clamp(0.0, 1.0);
    let remaining = 1.0 - outer;
    rim_meters * remaining * remaining * remaining
}

/// Any two unit vectors perpendicular to `axis` and to each other.
fn tangent_basis(axis: DVec3) -> (DVec3, DVec3) {
    // Pick the world axis the direction leans on least, so the cross product
    // never approaches zero.
    let reference = if axis.y.abs() < 0.9 {
        DVec3::Y
    } else {
        DVec3::X
    };
    let tangent = axis.cross(reference).normalize();
    (tangent, axis.cross(tangent))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exhaustive height: every crater tested, no window and no cutoff.
    /// This is the definition the fast paths have to reproduce.
    fn exhaustive_raw_height_meters(catalogue: &Catalogue, direction: DVec3) -> f64 {
        let direction = direction.normalize();
        catalogue
            .iter()
            .map(|crater| {
                let cosine = crater.axis.dot(direction).clamp(-1.0, 1.0);
                let t = cosine.acos() / crater.angular_radius;
                profile(t, crater.depth_meters, crater.rim_meters)
            })
            .sum()
    }

    /// A spread of directions that does not line up with either spiral,
    /// covering both poles, the equator and the latitudes between.
    fn sample_directions(count: usize) -> Vec<DVec3> {
        (0..count)
            .map(|index| {
                let i = index as f64;
                let z = 1.0 - 2.0 * (i + 0.5) / count as f64;
                let ring = (1.0 - z * z).max(0.0).sqrt();
                let theta = 2.399_963_23 * i + 0.37;
                DVec3::new(ring * theta.cos(), z, ring * theta.sin()).normalize()
            })
            .collect()
    }

    #[test]
    fn both_catalogues_are_deterministic_and_well_formed() {
        for catalogue in [runtime(), baked()] {
            let repeat = Catalogue::new(if catalogue.len() == runtime().len() {
                RUNTIME_SPEC
            } else {
                BAKED_SPEC
            });
            for (a, b) in catalogue.iter().zip(repeat.iter()) {
                assert_eq!(a.axis, b.axis);
                assert_eq!(a.angular_radius, b.angular_radius);
                assert_eq!(a.depth_meters, b.depth_meters);
            }
            for crater in catalogue.iter() {
                assert!((crater.axis.length() - 1.0).abs() < 1.0e-12);
                assert!(crater.angular_radius > 0.0);
                assert!(crater.depth_meters > 0.0);
                assert!(crater.rim_meters > 0.0 && crater.rim_meters < crater.depth_meters);
            }
        }
    }

    /// `field_index_range` inverts the spiral's index-to-latitude mapping, so
    /// it is only correct while the field really is sorted by latitude.
    #[test]
    fn the_field_is_sorted_by_latitude() {
        for catalogue in [runtime(), baked()] {
            for pair in catalogue.field().windows(2) {
                assert!(
                    pair[0].axis.y > pair[1].axis.y,
                    "the window search needs a monotonic field",
                );
            }
        }
    }

    /// Size must be scattered across position, not strided through it.
    ///
    /// The stride this replaced gave every size class its own arithmetic
    /// progression of indices, and on a golden-angle spiral that is a lattice
    /// -- the medium craters came out in visible rows. A step histogram catches
    /// it: a stride has exactly one step value, a shuffle has hundreds.
    #[test]
    fn size_is_shuffled_across_position_rather_than_strided() {
        let field = runtime().field();
        let ranks: Vec<f64> = field.iter().map(|c| c.angular_radius).collect();
        let mut steps = std::collections::HashSet::new();
        for pair in ranks.windows(2) {
            // Quantised, so floating-point noise cannot manufacture variety.
            steps.insert(((pair[1] - pair[0]) * 1.0e6).round() as i64);
        }
        assert!(
            steps.len() > ranks.len() / 4,
            "only {} distinct size steps across {} craters, which is a stride \
             rather than a shuffle",
            steps.len(),
            ranks.len(),
        );
    }

    /// Jitter must break the lattice, or the spiral's arms read as a grid.
    /// Measured as the spread of nearest-neighbour distances: an even spiral
    /// has nearly identical spacing everywhere, and scattered points do not.
    #[test]
    fn the_field_is_scattered_rather_than_a_lattice() {
        let field = runtime().field();
        let mut nearest: Vec<f64> = Vec::with_capacity(field.len());
        for (index, crater) in field.iter().enumerate() {
            // The field is latitude-sorted, so neighbours in space are near
            // neighbours in index; a local window is enough to find them.
            let low = index.saturating_sub(24);
            let high = (index + 25).min(field.len());
            let closest = field[low..high]
                .iter()
                .enumerate()
                .filter(|(offset, _)| low + offset != index)
                .map(|(_, other)| crater.axis.dot(other.axis).clamp(-1.0, 1.0).acos())
                .fold(f64::INFINITY, f64::min);
            nearest.push(closest);
        }
        let mean = nearest.iter().sum::<f64>() / nearest.len() as f64;
        let variance =
            nearest.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / nearest.len() as f64;
        let spread = variance.sqrt() / mean;
        assert!(
            spread > 0.35,
            "nearest-neighbour spacing varies by only {spread:.3} of its mean, which is a lattice rather than an impact history",
        );
    }

    /// Nothing in the windowed field may reach outside the window, or the
    /// window would silently drop craters.
    #[test]
    fn the_field_stays_inside_the_window_it_assumes() {
        for catalogue in [runtime(), baked()] {
            for crater in catalogue.field() {
                assert!(
                    crater.reach_radians() <= catalogue.field_window_radians() + 1.0e-12,
                    "a field crater reaches further than the window looks",
                );
            }
        }
    }

    /// The load-bearing test for the window: the fast path and the exhaustive
    /// definition must agree everywhere, not merely look similar. If the
    /// window or the cutoff ever drops a crater that contributes, this is what
    /// catches it.
    #[test]
    fn the_windowed_height_matches_testing_every_crater() {
        for catalogue in [runtime(), baked()] {
            let mut worst = 0.0_f64;
            let samples = if catalogue.len() > 50_000 { 60 } else { 600 };
            for direction in sample_directions(samples) {
                let difference = (catalogue.raw_height_meters(direction)
                    - exhaustive_raw_height_meters(catalogue, direction))
                .abs();
                worst = worst.max(difference);
            }
            assert!(
                worst < 1.0e-6,
                "the window changed the height field by {worst} metres",
            );
        }
    }

    /// The window has to actually be narrow, or it is correct and useless.
    ///
    /// The bound is looser than the geometry alone would give, because the
    /// jitter's index slack widens every answer. At 352 craters that slack is
    /// a meaningful share of an already narrow window; on a bigger field it is
    /// noise.
    #[test]
    fn the_window_tests_a_small_fraction_of_the_field() {
        let catalogue = runtime();
        let samples = sample_directions(2000);
        let mut worst = 0;
        let mut total = 0;
        for direction in &samples {
            let (first, last) = catalogue.field_index_range(direction.y);
            let tested = last - first + 1;
            worst = worst.max(tested);
            total += tested;
        }
        let field = catalogue.field().len();
        assert!(
            worst < field / 3,
            "the widest window tested {worst} of {field}"
        );
        assert!(
            total / samples.len() < field / 5,
            "the average window tested {} of {field}",
            total / samples.len(),
        );
    }

    #[test]
    fn a_crater_is_a_bowl_inside_a_rim() {
        let crater = runtime().basins()[0];
        let centre = profile(0.0, crater.depth_meters, crater.rim_meters);
        let crest = profile(1.0, crater.depth_meters, crater.rim_meters);
        let outside = profile(2.0, crater.depth_meters, crater.rim_meters);
        assert!(centre < 0.0, "the floor sits below the datum");
        assert!(crest > 0.0, "the rim stands above it");
        assert!(centre < crest);
        assert_eq!(outside, 0.0, "influence ends inside the ejecta extent");
    }

    /// A crater has to reach zero at the edge of its influence, or every
    /// catalogue entry would offset the whole body and the datum would drift
    /// with the crater count.
    #[test]
    fn influence_is_local() {
        let crater = runtime().basins()[3];
        assert_eq!(crater.contribution_meters(-crater.axis), 0.0);
    }

    /// The cutoff must reject only craters whose profile is already zero, so
    /// it is an optimisation and not a change to the height field.
    #[test]
    fn the_cutoff_only_rejects_craters_that_contribute_nothing() {
        for catalogue in [runtime(), baked()] {
            for crater in catalogue.iter() {
                let outside = crater.cosine_cutoff.clamp(-1.0, 1.0).acos() / crater.angular_radius;
                assert!(
                    outside >= EJECTA_EXTENT - 1.0e-9,
                    "the cutoff sits at or beyond the end of the blanket",
                );
                // Not exactly zero: `cos` and `acos` do not round-trip, so the
                // cutoff can land a few ulp inside the end of the blanket,
                // where the cubic leaves a residue around 1e-32 metres.
                assert!(
                    profile(outside, crater.depth_meters, crater.rim_meters) < 1.0e-9,
                    "a crater rejected by the cutoff was contributing nothing",
                );
            }
        }
    }

    /// Most impacts are small, or a catalogue of thousands merges into noise
    /// at the large end. This asserts the shape of the size law rather than
    /// any individual radius.
    #[test]
    fn most_impacts_are_small_and_the_basins_are_rare() {
        for catalogue in [runtime(), baked()] {
            let mut radii: Vec<f64> = catalogue.iter().map(|c| c.angular_radius).collect();
            radii.sort_by(|a, b| a.partial_cmp(b).expect("radii are finite"));
            let median = radii[radii.len() / 2];
            let largest = radii[radii.len() - 1];
            assert!(
                median < 0.1 * largest,
                "the median crater is small against the largest, got {median}",
            );
            let large = radii.iter().filter(|r| **r > 0.5 * largest).count();
            assert!(
                large <= 4,
                "a handful of basins, not a field of them, got {large}",
            );
        }
    }

    /// The datum is what keeps the baked body inside the height channel and
    /// above sea level. The baker asserts the measured extremes; this asserts
    /// the constant is big enough for the deepest single basin at minimum, so
    /// a change to the catalogue that deepens it fails here too.
    #[test]
    fn the_datum_clears_the_deepest_basin() {
        let deepest = baked()
            .iter()
            .map(|crater| crater.depth_meters)
            .fold(0.0_f64, f64::max);
        assert!(
            MOON_DATUM_METERS > deepest,
            "the datum {MOON_DATUM_METERS}m does not clear a {deepest}m basin",
        );
    }
}

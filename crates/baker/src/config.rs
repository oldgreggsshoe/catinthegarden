use std::path::PathBuf;

use catinthegarden_coretypes::{
    MAX_DENSE_LEVEL, PLANET_RADIUS_METERS, QUADTREE_MAX_LEVEL, moon::MOON_RADIUS_METERS,
};

/// Working grid width for a moon bake.
///
/// Wider than the planet's 4,096 despite the smaller body: at 1,080km radius
/// this is 828m a cell, which is what lets the catalogue's 3km craters be
/// several cells across instead of spikes. A test in `moon.rs` holds that.
pub const MOON_WORKING_WIDTH: usize = 8_192;
pub const MOON_WORKING_HEIGHT: usize = 4_096;

#[derive(Clone, Debug, PartialEq)]
pub struct BakeConfig {
    pub output: PathBuf,
    /// Optional NOAA ETOPO 2022 Ice Surface GeoTIFF. When present, its real
    /// global relief replaces the authored base shape; synthetic erosion and
    /// river/glacier carving stay disabled so the imported terrain survives.
    pub etopo: Option<PathBuf>,
    /// Apply the deliberately game-like relief pass to the macro source:
    /// positive land is vertically amplified and receives dense, bounded
    /// ridge detail while coastlines, sea level, and bathymetry remain fixed.
    pub game_terrain: bool,
    /// Remap a compact observed source window across the globe before the
    /// game-relief pass. This is intentionally non-Earth-like: a small,
    /// mountain-rich "zoomed map" becomes the whole playable planet.
    pub zoomed_terrain: bool,
    /// Generate a fully procedural, globally seam-safe game landscape. This
    /// profile does not read ETOPO or the authored Earth-like ellipses; the
    /// continuous noise field is followed by the normal erosion, drainage,
    /// river, lake, and biome stages.
    pub procedural_terrain: bool,
    /// Bake an airless, dry body from the shared crater catalogue instead of
    /// running continents, erosion and hydrology. Sets `radius_meters` to the
    /// moon's, and skips every stage that needs water or air.
    pub moon: bool,
    /// Radius of the body being baked. Written into the manifest, and the
    /// renderer refuses an outmap whose radius is not the body it is drawing —
    /// streaming a 4,000km world's tiles onto a 1,080km one would drape the
    /// planet's geography over the moon at four times the relief.
    pub radius_meters: f64,
    pub seed: u32,
    pub width: usize,
    pub height: usize,
    pub dense_level: u8,
    pub max_level: u8,
    /// Constant sparse radius override. `None` uses the default physical-
    /// coverage profile, whose tile radius grows as tiles become smaller.
    pub sparse_radius: Option<u32>,
    pub erosion_iterations: usize,
}

impl Default for BakeConfig {
    fn default() -> Self {
        Self {
            output: PathBuf::from("assets/outmaps/test-planet"),
            etopo: None,
            game_terrain: false,
            zoomed_terrain: false,
            procedural_terrain: false,
            moon: false,
            radius_meters: PLANET_RADIUS_METERS,
            // Coastline and regional-detail seed for the Earth-like macro
            // layout in terrain.rs. The large continent and mountain-belt
            // placement is authored; this keeps its smaller shapes
            // deterministic without making a literal elevation copy.
            seed: 0xEA27_2026,
            // Preserve continental/hydrology data at a useful resolution, then
            // make actual L4 tiles available globally. L4 is the current
            // coarsest rendered level, so a lower dense level only makes the
            // renderer spend geometry work on ancestor-fallback textures.
            width: 4_096,
            height: 2_048,
            dense_level: 4,
            max_level: QUADTREE_MAX_LEVEL,
            sparse_radius: None,
            erosion_iterations: 2_048,
        }
    }
}

impl BakeConfig {
    pub fn quick(output: PathBuf) -> Self {
        Self {
            output,
            width: 64,
            height: 32,
            dense_level: 1,
            max_level: 4,
            sparse_radius: Some(0),
            erosion_iterations: 16,
            ..Self::default()
        }
    }

    /// An airless, dry body from the crater catalogue. Wider working grid and
    /// no erosion, because there is no water to do any.
    pub fn moon(output: PathBuf) -> Self {
        Self {
            output,
            moon: true,
            radius_meters: MOON_RADIUS_METERS,
            width: MOON_WORKING_WIDTH,
            height: MOON_WORKING_HEIGHT,
            erosion_iterations: 0,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.radius_meters <= 0.0 || !self.radius_meters.is_finite() {
            return Err("body radius must be finite and positive".to_owned());
        }
        if self.moon && (self.etopo.is_some() || self.game_terrain || self.zoomed_terrain) {
            return Err(
                "a moon bake takes its shape from the crater catalogue, not an Earth source"
                    .to_owned(),
            );
        }
        if self.width < 16 || self.height < 8 {
            return Err("working grid must be at least 16x8".to_owned());
        }
        if !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2) {
            return Err("working grid dimensions must be even".to_owned());
        }
        if self.dense_level > self.max_level || self.max_level > QUADTREE_MAX_LEVEL {
            return Err(format!(
                "levels must satisfy dense <= max <= {QUADTREE_MAX_LEVEL}"
            ));
        }
        if self.dense_level > MAX_DENSE_LEVEL {
            return Err(format!(
                "dense levels above {MAX_DENSE_LEVEL} are intentionally unsupported"
            ));
        }
        if self.sparse_radius.is_some_and(|radius| radius > 8) {
            return Err("sparse radius above 8 is intentionally unsupported".to_owned());
        }
        // An airless, dry body runs no erosion at all, so zero is the only
        // correct value there rather than a misconfiguration.
        if self.erosion_iterations == 0 && !self.moon {
            return Err("erosion iterations must be positive".to_owned());
        }
        if self.erosion_iterations != 0 && self.moon {
            return Err("a moon has no water to erode with".to_owned());
        }
        if self
            .etopo
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err("ETOPO path must not be empty".to_owned());
        }
        if self.procedural_terrain && self.etopo.is_some() {
            return Err("procedural terrain cannot be combined with ETOPO".to_owned());
        }
        Ok(())
    }
}

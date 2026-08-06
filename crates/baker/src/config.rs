use std::path::PathBuf;

use catinthegarden_coretypes::{MAX_DENSE_LEVEL, QUADTREE_MAX_LEVEL};

#[derive(Clone, Debug, Eq, PartialEq)]
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

    pub fn validate(&self) -> Result<(), String> {
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
        if self.erosion_iterations == 0 {
            return Err("erosion iterations must be positive".to_owned());
        }
        if self
            .etopo
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err("ETOPO path must not be empty".to_owned());
        }
        Ok(())
    }
}

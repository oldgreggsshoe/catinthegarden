use std::path::PathBuf;

use catinthegarden_coretypes::{MAX_DENSE_LEVEL, QUADTREE_MAX_LEVEL};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BakeConfig {
    pub output: PathBuf,
    pub seed: u32,
    pub width: usize,
    pub height: usize,
    pub dense_level: u8,
    pub max_level: u8,
    pub sparse_radius: u32,
    pub erosion_iterations: usize,
}

impl Default for BakeConfig {
    fn default() -> Self {
        Self {
            output: PathBuf::from("assets/outmaps/test-planet"),
            seed: 0x000C_471A,
            // Preserve continental/hydrology data at a useful resolution, then
            // make actual L4 tiles available globally. L4 is the current
            // coarsest rendered level, so a lower dense level only makes the
            // renderer spend geometry work on ancestor-fallback textures.
            width: 1_024,
            height: 512,
            dense_level: 4,
            max_level: QUADTREE_MAX_LEVEL,
            sparse_radius: 1,
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
            sparse_radius: 0,
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
        if self.sparse_radius > 4 {
            return Err("sparse radius above 4 is intentionally unsupported".to_owned());
        }
        if self.erosion_iterations == 0 {
            return Err("erosion iterations must be positive".to_owned());
        }
        Ok(())
    }
}

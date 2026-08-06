pub mod config;
mod etopo;
mod export;
mod grid;
pub mod terrain;

use catinthegarden_coretypes::OutmapManifest;

pub use config::BakeConfig;
pub use export::{
    available_tile_keys, refine_existing_outmap, sparse_radius_for_level, validate_output,
};
pub use terrain::{MountainVisibilityReport, Terrain};

pub type BakeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn bake(config: &BakeConfig) -> BakeResult<OutmapManifest> {
    bake_internal(config, false).map(|(manifest, _)| manifest)
}

pub fn bake_with_mountain_coverage(
    config: &BakeConfig,
) -> BakeResult<(OutmapManifest, MountainVisibilityReport)> {
    bake_internal(config, true)
}

fn bake_internal(
    config: &BakeConfig,
    report_mountain_coverage: bool,
) -> BakeResult<(OutmapManifest, MountainVisibilityReport)> {
    config
        .validate()
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    let terrain = Terrain::try_generate(config)?;
    let mountain_coverage = report_mountain_coverage
        .then(|| terrain.mountain_visibility_coverage())
        .unwrap_or_default();
    let manifest = export::export_outmap(config, &terrain)?;
    validate_output(&config.output)?;
    Ok((manifest, mountain_coverage))
}

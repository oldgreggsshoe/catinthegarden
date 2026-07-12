pub mod config;
mod export;
mod grid;
pub mod terrain;

use catinthegarden_coretypes::OutmapManifest;

pub use config::BakeConfig;
pub use export::{available_tile_keys, validate_output};
pub use terrain::Terrain;

pub type BakeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn bake(config: &BakeConfig) -> BakeResult<OutmapManifest> {
    config
        .validate()
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    let terrain = Terrain::generate(config);
    let manifest = export::export_outmap(config, &terrain)?;
    validate_output(&config.output)?;
    Ok(manifest)
}

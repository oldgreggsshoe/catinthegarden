use std::{
    collections::BTreeSet,
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use catinthegarden_coretypes::{
    BiomeId, BiomeManifestEntry, ChannelManifest, CubeFace, OUTMAP_SCHEMA_VERSION, OutmapManifest,
    PLANET_RADIUS_METERS, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey,
    face_uv_to_direction,
};
use glam::DVec3;
use image::{GrayImage, ImageBuffer, ImageReader, Luma, Rgb, RgbImage};

use crate::{
    BakeResult,
    config::BakeConfig,
    terrain::{MAX_HEIGHT_METERS, MIN_HEIGHT_METERS, Terrain},
};

const HEIGHT_FILE: &str = "height.r32f";
const BIOME_FILE: &str = "biome.r8";
const MOISTURE_FILE: &str = "moisture.r8";

pub fn export_outmap(config: &BakeConfig, terrain: &Terrain) -> BakeResult<OutmapManifest> {
    fs::create_dir_all(&config.output)?;
    write_previews(&config.output.join("previews"), terrain)?;
    let available_tiles = available_tile_keys(config);
    for &key in &available_tiles {
        write_tile(&config.output, terrain, key)?;
    }
    let manifest = build_manifest(config, available_tiles);
    manifest
        .validate()
        .map_err(|message| io::Error::new(ErrorKind::InvalidData, message))?;
    fs::write(
        config.output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

pub fn validate_output(output: &Path) -> BakeResult<OutmapManifest> {
    let manifest: OutmapManifest =
        serde_json::from_slice(&fs::read(output.join("manifest.json"))?)?;
    manifest
        .validate()
        .map_err(|message| io::Error::new(ErrorKind::InvalidData, message))?;
    validate_channels(&manifest)?;
    validate_previews(output, &manifest)?;
    for &key in &manifest.available_tiles {
        let directory = output.join(key.relative_path());
        let height = fs::read(directory.join(HEIGHT_FILE))?;
        let biome = fs::read(directory.join(BIOME_FILE))?;
        let moisture = fs::read(directory.join(MOISTURE_FILE))?;
        let samples = (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize;
        if height.len() != samples * size_of::<f32>() {
            return Err(invalid_data(format!("bad height byte count for {key:?}")));
        }
        if biome.len() != samples || moisture.len() != samples {
            return Err(invalid_data(format!("bad R8 byte count for {key:?}")));
        }
        for bytes in height.chunks_exact(size_of::<f32>()) {
            let value = f32::from_le_bytes(bytes.try_into().expect("chunk has four bytes"));
            if !value.is_finite()
                || value < MIN_HEIGHT_METERS as f32 - 0.01
                || value > MAX_HEIGHT_METERS as f32 + 0.01
            {
                return Err(invalid_data(format!("invalid height in {key:?}")));
            }
        }
        if let Some(&invalid) = biome
            .iter()
            .find(|&&value| BiomeId::try_from(value).is_err())
        {
            return Err(invalid_data(format!(
                "invalid biome id {invalid} in {key:?}"
            )));
        }
        if directory.join("normal.r8").exists() || directory.join("normal.bin").exists() {
            return Err(invalid_data(format!(
                "normal data must not be baked for {key:?}"
            )));
        }
    }
    validate_landing_height(output, &manifest)?;
    Ok(manifest)
}

fn validate_channels(manifest: &OutmapManifest) -> BakeResult<()> {
    let expected = [
        ("height", "r32float_le", HEIGHT_FILE),
        ("biome", "r8uint", BIOME_FILE),
        ("moisture", "r8unorm", MOISTURE_FILE),
    ];
    for ((name, format, file_name), actual) in expected.into_iter().zip(&manifest.channels) {
        if actual.name != name || actual.format != format || actual.file_name != file_name {
            return Err(invalid_data("unexpected terrain channel descriptor"));
        }
    }
    Ok(())
}

fn validate_previews(output: &Path, manifest: &OutmapManifest) -> BakeResult<()> {
    for name in ["height.png", "biome.png", "moisture.png"] {
        let image = ImageReader::open(output.join("previews").join(name))?
            .with_guessed_format()?
            .decode()?;
        if image.width() != manifest.working_width || image.height() != manifest.working_height {
            return Err(invalid_data(format!("bad dimensions for preview {name}")));
        }
    }
    Ok(())
}

fn validate_landing_height(output: &Path, manifest: &OutmapManifest) -> BakeResult<()> {
    let level = manifest.max_level;
    let side = 1_u32 << level;
    let key = TileKey {
        face: CubeFace::PositiveX,
        level,
        x: side / 2,
        y: side / 2,
    };
    if !manifest.has_tile(key) {
        return Err(invalid_data("missing +X landing tile at maximum level"));
    }
    let bytes = fs::read(output.join(key.relative_path()).join(HEIGHT_FILE))?;
    let sample_index = (TILE_STORED_SIZE + 1) as usize;
    let start = sample_index * size_of::<f32>();
    let height = f32::from_le_bytes(
        bytes[start..start + size_of::<f32>()]
            .try_into()
            .expect("height sample has four bytes"),
    );
    if height > 0.0 {
        return Err(invalid_data(format!(
            "+X landing height must be at or below sea level, got {height}"
        )));
    }
    Ok(())
}

fn invalid_data(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(io::Error::new(ErrorKind::InvalidData, message.into()))
}

fn build_manifest(config: &BakeConfig, available_tiles: Vec<TileKey>) -> OutmapManifest {
    OutmapManifest {
        schema_version: OUTMAP_SCHEMA_VERSION,
        generator: format!("catinthegarden-baker {}", env!("CARGO_PKG_VERSION")),
        seed: config.seed,
        planet_radius_meters: PLANET_RADIUS_METERS,
        working_width: config.width as u32,
        working_height: config.height as u32,
        dense_level: config.dense_level,
        max_level: config.max_level,
        tile_logical_size: TILE_LOGICAL_SIZE,
        tile_stored_size: TILE_STORED_SIZE,
        tile_gutter: TILE_GUTTER,
        height_min_meters: MIN_HEIGHT_METERS as f32,
        height_max_meters: MAX_HEIGHT_METERS as f32,
        sparse_landing_direction: DVec3::X.to_array(),
        channels: vec![
            ChannelManifest {
                name: "height".to_owned(),
                format: "r32float_le".to_owned(),
                file_name: HEIGHT_FILE.to_owned(),
            },
            ChannelManifest {
                name: "biome".to_owned(),
                format: "r8uint".to_owned(),
                file_name: BIOME_FILE.to_owned(),
            },
            ChannelManifest {
                name: "moisture".to_owned(),
                format: "r8unorm".to_owned(),
                file_name: MOISTURE_FILE.to_owned(),
            },
        ],
        biomes: BiomeId::ALL
            .into_iter()
            .map(|biome| BiomeManifestEntry {
                id: biome as u8,
                name: biome.name().to_owned(),
                color: biome.color(),
            })
            .collect(),
        available_tiles,
    }
}

pub fn available_tile_keys(config: &BakeConfig) -> Vec<TileKey> {
    let mut keys = BTreeSet::new();
    for level in 0..=config.dense_level {
        let side = 1_u32 << level;
        for face in CubeFace::ALL {
            for y in 0..side {
                for x in 0..side {
                    keys.insert(TileKey { face, level, x, y });
                }
            }
        }
    }
    for level in config.dense_level.saturating_add(1)..=config.max_level {
        let side = 1_u32 << level;
        let center = side / 2;
        let start_x = center.saturating_sub(config.sparse_radius);
        let start_y = center.saturating_sub(config.sparse_radius);
        let end_x = center.saturating_add(config.sparse_radius).min(side - 1);
        let end_y = center.saturating_add(config.sparse_radius).min(side - 1);
        for y in start_y..=end_y {
            for x in start_x..=end_x {
                let mut key = Some(TileKey {
                    face: CubeFace::PositiveX,
                    level,
                    x,
                    y,
                });
                while let Some(current) = key {
                    keys.insert(current);
                    key = current.parent();
                }
            }
        }
    }
    keys.into_iter().collect()
}

fn write_previews(output: &Path, terrain: &Terrain) -> BakeResult<()> {
    fs::create_dir_all(output)?;
    let width = terrain.grid.width() as u32;
    let height = terrain.grid.height() as u32;
    let mut height_image: ImageBuffer<Luma<u16>, Vec<u16>> = ImageBuffer::new(width, height);
    let mut biome_image = RgbImage::new(width, height);
    let mut moisture_image = GrayImage::new(width, height);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let index = terrain.grid.index(x, y);
            let normalized = ((terrain.height_meters[index] - MIN_HEIGHT_METERS)
                / (MAX_HEIGHT_METERS - MIN_HEIGHT_METERS))
                .clamp(0.0, 1.0);
            height_image.put_pixel(x as u32, y as u32, Luma([(normalized * 65_535.0) as u16]));
            biome_image.put_pixel(x as u32, y as u32, Rgb(terrain.biome[index].color()));
            moisture_image.put_pixel(x as u32, y as u32, Luma([terrain.moisture[index]]));
        }
    }
    height_image.save(output.join("height.png"))?;
    biome_image.save(output.join("biome.png"))?;
    moisture_image.save(output.join("moisture.png"))?;
    Ok(())
}

fn write_tile(output: &Path, terrain: &Terrain, key: TileKey) -> BakeResult<()> {
    let directory = output.join(key.relative_path());
    fs::create_dir_all(&directory)?;
    let sample_count = (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize;
    let mut height_bytes = Vec::with_capacity(sample_count * size_of::<f32>());
    let mut biome_bytes = Vec::with_capacity(sample_count);
    let mut moisture_bytes = Vec::with_capacity(sample_count);
    let biome_ids: Vec<u8> = terrain.biome.iter().map(|biome| *biome as u8).collect();
    let side = 1_u64 << key.level;
    let denominator = u64::from(TILE_LOGICAL_SIZE - 1) * side;
    for stored_y in 0..TILE_STORED_SIZE {
        let local_y = i64::from(stored_y) - i64::from(TILE_GUTTER);
        let global_y = i64::from(key.y) * i64::from(TILE_LOGICAL_SIZE - 1) + local_y;
        let v = -1.0 + 2.0 * global_y as f64 / denominator as f64;
        for stored_x in 0..TILE_STORED_SIZE {
            let local_x = i64::from(stored_x) - i64::from(TILE_GUTTER);
            let global_x = i64::from(key.x) * i64::from(TILE_LOGICAL_SIZE - 1) + local_x;
            let u = -1.0 + 2.0 * global_x as f64 / denominator as f64;
            let direction = face_uv_to_direction(key.face, u, v);
            let height = terrain
                .grid
                .sample_f64(&terrain.height_meters, direction)
                .clamp(MIN_HEIGHT_METERS, MAX_HEIGHT_METERS) as f32;
            height_bytes.extend_from_slice(&height.to_le_bytes());
            biome_bytes.push(terrain.grid.sample_u8_nearest(&biome_ids, direction));
            moisture_bytes.push(terrain.grid.sample_u8_linear(&terrain.moisture, direction));
        }
    }
    fs::write(directory.join(HEIGHT_FILE), height_bytes)?;
    fs::write(directory.join(BIOME_FILE), biome_bytes)?;
    fs::write(directory.join(MOISTURE_FILE), moisture_bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_keys_reach_level_eighteen_and_have_parents() {
        let config = BakeConfig::default();
        let keys = available_tile_keys(&config);
        assert!(
            keys.iter()
                .any(|key| { key.face == CubeFace::PositiveX && key.level == config.max_level })
        );
        for key in &keys {
            if let Some(parent) = key.parent() {
                assert!(keys.binary_search(&parent).is_ok());
            }
        }
    }
}

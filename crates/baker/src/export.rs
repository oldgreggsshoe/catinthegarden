use std::{
    collections::{BTreeMap, BTreeSet},
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
use noise::{NoiseFn, Perlin};

use crate::{
    BakeResult,
    config::BakeConfig,
    terrain::{MAX_HEIGHT_METERS, MIN_HEIGHT_METERS, Terrain, snowline_meters},
};

const HEIGHT_FILE: &str = "height.r32f";
const BIOME_FILE: &str = "biome.r8";
const MOISTURE_FILE: &str = "moisture.r8";
const MICRORELIEF_START_LEVEL: u8 = 12;
const MICRORELIEF_MAX_AMPLITUDE_METERS: f64 = 2.0;
const MICRORELIEF_FREQUENCY: f64 = 220_000.0;
const BAKED_DETAIL_START_LEVEL: u8 = 3;
const BAKED_DETAIL_MAX_AMPLITUDE_METERS: f64 = 300.0;
const BAKED_DETAIL_BASE_FREQUENCY: f64 = 40.0;
const LANDING_DETAIL_PROTECTION_METERS: f64 = 500.0;
const BAKED_BIOME_DETAIL_START_LEVEL: u8 = 3;
const BAKED_BIOME_DETAIL_FREQUENCY: f64 = 280.0;

pub fn export_outmap(config: &BakeConfig, terrain: &Terrain) -> BakeResult<OutmapManifest> {
    fs::create_dir_all(&config.output)?;
    write_previews(&config.output.join("previews"), terrain)?;
    let available_tiles = available_tile_keys(config);
    let mut generated_tiles = BTreeMap::new();
    let microrelief = Perlin::new(config.seed ^ 0x4D49_4352);
    for &key in &available_tiles {
        let mut tile = sample_tile(config, terrain, key, &microrelief);
        if let Some(parent) = key.parent() {
            let parent_tile = generated_tiles
                .get(&parent)
                .expect("available tile ordering must place parents before children");
            constrain_logical_border_to_parent(&mut tile, key, parent_tile);
        }
        write_tile(&config.output, key, &tile)?;
        generated_tiles.insert(key, tile);
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
    let mut height_tiles = BTreeMap::new();
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
        let mut decoded_height = Vec::with_capacity(samples);
        for bytes in height.chunks_exact(size_of::<f32>()) {
            let value = f32::from_le_bytes(bytes.try_into().expect("chunk has four bytes"));
            if !value.is_finite()
                || value < MIN_HEIGHT_METERS as f32 - 0.01
                || value > MAX_HEIGHT_METERS as f32 + 0.01
            {
                return Err(invalid_data(format!("invalid height in {key:?}")));
            }
            decoded_height.push(value);
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
        height_tiles.insert(key, decoded_height);
    }
    validate_fallback_edges(&manifest, &height_tiles)?;
    validate_landing_height(output, &manifest)?;
    Ok(manifest)
}

fn validate_fallback_edges(
    manifest: &OutmapManifest,
    height_tiles: &BTreeMap<TileKey, Vec<f32>>,
) -> BakeResult<()> {
    const EDGE_TOLERANCE_METERS: f32 = 0.002;
    for &key in &manifest.available_tiles {
        if key.level == 0 {
            continue;
        }
        let side = 1_u32 << key.level;
        for (dx, dy, edge) in [(-1_i32, 0_i32, 0_u8), (1, 0, 1), (0, -1, 2), (0, 1, 3)] {
            let neighbor_x = key.x as i64 + i64::from(dx);
            let neighbor_y = key.y as i64 + i64::from(dy);
            if neighbor_x < 0
                || neighbor_y < 0
                || neighbor_x >= i64::from(side)
                || neighbor_y >= i64::from(side)
            {
                continue;
            }
            let neighbor = TileKey {
                face: key.face,
                level: key.level,
                x: neighbor_x as u32,
                y: neighbor_y as u32,
            };
            let fallback = manifest
                .best_available_ancestor(neighbor)
                .expect("every face has a root fallback");
            let active_height = &height_tiles[&key];
            let fallback_height = &height_tiles[&fallback];
            for offset in 0..TILE_LOGICAL_SIZE {
                let (local_x, local_y) = match edge {
                    0 => (0, offset),
                    1 => (TILE_LOGICAL_SIZE - 1, offset),
                    2 => (offset, 0),
                    3 => (offset, TILE_LOGICAL_SIZE - 1),
                    _ => unreachable!(),
                };
                let global_x =
                    u64::from(key.x) * u64::from(TILE_LOGICAL_SIZE - 1) + u64::from(local_x);
                let global_y =
                    u64::from(key.y) * u64::from(TILE_LOGICAL_SIZE - 1) + u64::from(local_y);
                let scale = (1_u64 << fallback.level) as f64 / (1_u64 << key.level) as f64;
                let fallback_x =
                    global_x as f64 * scale - f64::from(fallback.x * (TILE_LOGICAL_SIZE - 1));
                let fallback_y =
                    global_y as f64 * scale - f64::from(fallback.y * (TILE_LOGICAL_SIZE - 1));
                let actual = active_height[logical_index(local_x, local_y)];
                let expected = sample_parent_f32(fallback_height, fallback_x, fallback_y);
                if (actual - expected).abs() > EDGE_TOLERANCE_METERS {
                    return Err(invalid_data(format!(
                        "fallback edge mismatch {key:?} -> {fallback:?}: {:.6}m",
                        (actual - expected).abs()
                    )));
                }
            }
        }
    }
    Ok(())
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

#[derive(Clone, Debug)]
struct TileData {
    height: Vec<f32>,
    biome: Vec<u8>,
    moisture: Vec<u8>,
}

fn sample_tile(
    config: &BakeConfig,
    terrain: &Terrain,
    key: TileKey,
    microrelief: &Perlin,
) -> TileData {
    let sample_count = (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize;
    let mut height = Vec::with_capacity(sample_count);
    let mut biome = Vec::with_capacity(sample_count);
    let mut moisture = Vec::with_capacity(sample_count);
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
            let sampled_moisture = terrain.grid.sample_u8_linear(&terrain.moisture, direction);
            let sampled_height = terrain.grid.sample_f64(&terrain.height_meters, direction)
                + baked_surface_detail(key, direction, microrelief)
                + sparse_microrelief(config, key, direction, microrelief);
            let sampled_height = sampled_height.clamp(MIN_HEIGHT_METERS, MAX_HEIGHT_METERS) as f32;
            height.push(sampled_height);
            let sampled_biome = terrain.grid.sample_u8_nearest(&biome_ids, direction);
            biome.push(baked_biome_detail(
                key,
                direction,
                f64::from(sampled_height),
                sampled_moisture,
                sampled_biome,
                microrelief,
            ));
            moisture.push(sampled_moisture);
        }
    }
    TileData {
        height,
        biome,
        moisture,
    }
}

fn baked_biome_detail(
    key: TileKey,
    direction: DVec3,
    height_meters: f64,
    moisture: u8,
    base_biome: u8,
    noise: &Perlin,
) -> u8 {
    let base_biome = BiomeId::try_from(base_biome).expect("terrain biome ids are valid");
    if key.level < BAKED_BIOME_DETAIL_START_LEVEL
        || matches!(base_biome, BiomeId::Ocean | BiomeId::Lake | BiomeId::Ice)
    {
        return base_biome as u8;
    }

    let latitude = direction.y.asin();
    let absolute_latitude = latitude.abs();
    if absolute_latitude > 66.0_f64.to_radians() || height_meters > snowline_meters(latitude) {
        return BiomeId::Ice as u8;
    }
    if height_meters <= 0.0 {
        return BiomeId::Ocean as u8;
    }
    if height_meters > (snowline_meters(latitude) - 700.0).max(2_800.0) {
        return BiomeId::MountainSnow as u8;
    }
    if height_meters > 2_400.0 {
        return BiomeId::MountainRock as u8;
    }

    let detail_moisture = noise.get([
        direction.x * BAKED_BIOME_DETAIL_FREQUENCY,
        direction.y * BAKED_BIOME_DETAIL_FREQUENCY,
        direction.z * BAKED_BIOME_DETAIL_FREQUENCY,
    ]);
    let wetness = (f64::from(moisture) / 255.0 + detail_moisture * 0.26).clamp(0.0, 1.0);
    let temperature = 1.0
        - absolute_latitude / std::f64::consts::FRAC_PI_2
        - height_meters / MAX_HEIGHT_METERS * 0.55;
    let biome = if temperature < 0.24 {
        BiomeId::Tundra
    } else if temperature > 0.72 && wetness > 0.62 {
        BiomeId::TropicalForest
    } else if wetness < 0.28 {
        BiomeId::Desert
    } else if wetness > 0.58 {
        BiomeId::TemperateForest
    } else {
        BiomeId::TemperateGrassland
    };
    biome as u8
}

fn baked_surface_detail(key: TileKey, direction: DVec3, noise: &Perlin) -> f64 {
    if key.level < BAKED_DETAIL_START_LEVEL {
        return 0.0;
    }

    // The equirectangular terrain atlas carries continental features and
    // erosion. These three 3D-noise bands are sampled only by the baker, then
    // stored in higher-level tiles, providing kilometre-scale terrain rather
    // than a runtime procedural fallback.
    let sample = |frequency: f64| {
        noise.get([
            direction.x * frequency,
            direction.y * frequency,
            direction.z * frequency,
        ])
    };
    let detail = sample(BAKED_DETAIL_BASE_FREQUENCY) * 0.58
        + sample(BAKED_DETAIL_BASE_FREQUENCY * 2.0) * 0.29
        + sample(BAKED_DETAIL_BASE_FREQUENCY * 4.0) * 0.13;
    let level_ramp = (f64::from(key.level - BAKED_DETAIL_START_LEVEL + 1) / 2.0).min(1.0);

    // Keep the deterministic descent target locally flat, without leaving the
    // old multi-degree landing disc as the only detail visible after zooming.
    let landing_angle = direction.dot(DVec3::X).clamp(-1.0, 1.0).acos();
    let protection_angle = LANDING_DETAIL_PROTECTION_METERS / PLANET_RADIUS_METERS;
    let landing_ramp = (landing_angle / protection_angle).clamp(0.0, 1.0);
    let landing_ramp = landing_ramp * landing_ramp * (3.0 - 2.0 * landing_ramp);

    detail * BAKED_DETAIL_MAX_AMPLITUDE_METERS * level_ramp * landing_ramp
}

fn sparse_microrelief(config: &BakeConfig, key: TileKey, direction: DVec3, noise: &Perlin) -> f64 {
    if key.face != CubeFace::PositiveX
        || key.level <= config.dense_level
        || key.level < MICRORELIEF_START_LEVEL
    {
        return 0.0;
    }
    let progress = (f64::from(key.level - MICRORELIEF_START_LEVEL)
        / f64::from(catinthegarden_coretypes::QUADTREE_MAX_LEVEL - MICRORELIEF_START_LEVEL))
    .clamp(0.0, 1.0);
    let ramp = progress * progress * (3.0 - 2.0 * progress);
    let sample = |sample_direction: DVec3| {
        noise.get([
            sample_direction.x * MICRORELIEF_FREQUENCY,
            sample_direction.y * MICRORELIEF_FREQUENCY,
            sample_direction.z * MICRORELIEF_FREQUENCY,
        ])
    };
    let centered = (sample(direction) - sample(DVec3::X)).clamp(-1.0, 1.0);
    centered * MICRORELIEF_MAX_AMPLITUDE_METERS * ramp
}

fn constrain_logical_border_to_parent(tile: &mut TileData, key: TileKey, parent: &TileData) {
    let half_quads = (TILE_LOGICAL_SIZE - 1) as f64 / 2.0;
    let quadrant_x = f64::from(key.x & 1);
    let quadrant_y = f64::from(key.y & 1);
    for logical_y in 0..TILE_LOGICAL_SIZE {
        for logical_x in 0..TILE_LOGICAL_SIZE {
            if logical_x != 0
                && logical_x != TILE_LOGICAL_SIZE - 1
                && logical_y != 0
                && logical_y != TILE_LOGICAL_SIZE - 1
            {
                continue;
            }
            let parent_x = quadrant_x * half_quads + f64::from(logical_x) * 0.5;
            let parent_y = quadrant_y * half_quads + f64::from(logical_y) * 0.5;
            let index = stored_index(logical_x + TILE_GUTTER, logical_y + TILE_GUTTER);
            tile.height[index] = sample_parent_f32(&parent.height, parent_x, parent_y);
            tile.moisture[index] = sample_parent_u8_linear(&parent.moisture, parent_x, parent_y);
            tile.biome[index] = sample_parent_u8_nearest(&parent.biome, parent_x, parent_y);
        }
    }
}

fn sample_parent_f32(values: &[f32], x: f64, y: f64) -> f32 {
    sample_parent_bilinear(values, x, y, |value| f64::from(*value)) as f32
}

fn sample_parent_u8_linear(values: &[u8], x: f64, y: f64) -> u8 {
    sample_parent_bilinear(values, x, y, |value| f64::from(*value))
        .round()
        .clamp(0.0, 255.0) as u8
}

fn sample_parent_bilinear<T>(values: &[T], x: f64, y: f64, promote: impl Fn(&T) -> f64) -> f64 {
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(TILE_LOGICAL_SIZE - 1);
    let y1 = (y0 + 1).min(TILE_LOGICAL_SIZE - 1);
    let tx = x - f64::from(x0);
    let ty = y - f64::from(y0);
    let top = promote(&values[logical_index(x0, y0)]) * (1.0 - tx)
        + promote(&values[logical_index(x1, y0)]) * tx;
    let bottom = promote(&values[logical_index(x0, y1)]) * (1.0 - tx)
        + promote(&values[logical_index(x1, y1)]) * tx;
    top * (1.0 - ty) + bottom * ty
}

fn sample_parent_u8_nearest(values: &[u8], x: f64, y: f64) -> u8 {
    let x = x.round().clamp(0.0, f64::from(TILE_LOGICAL_SIZE - 1)) as u32;
    let y = y.round().clamp(0.0, f64::from(TILE_LOGICAL_SIZE - 1)) as u32;
    values[logical_index(x, y)]
}

fn logical_index(x: u32, y: u32) -> usize {
    stored_index(x + TILE_GUTTER, y + TILE_GUTTER)
}

fn stored_index(x: u32, y: u32) -> usize {
    (y * TILE_STORED_SIZE + x) as usize
}

fn write_tile(output: &Path, key: TileKey, tile: &TileData) -> BakeResult<()> {
    let directory = output.join(key.relative_path());
    fs::create_dir_all(&directory)?;
    let mut height_bytes = Vec::with_capacity(tile.height.len() * size_of::<f32>());
    for height in &tile.height {
        height_bytes.extend_from_slice(&height.to_le_bytes());
    }
    fs::write(directory.join(HEIGHT_FILE), height_bytes)?;
    fs::write(directory.join(BIOME_FILE), &tile.biome)?;
    fs::write(directory.join(MOISTURE_FILE), &tile.moisture)?;
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

    #[test]
    fn level_four_tiles_contain_baked_detail_away_from_landing_site() {
        let key = TileKey {
            face: CubeFace::PositiveZ,
            level: 4,
            x: 8,
            y: 8,
        };
        let noise = Perlin::new(0xABCD_0123);
        let mut minimum = f64::INFINITY;
        let mut maximum = f64::NEG_INFINITY;
        for y in 0..=32 {
            for x in 0..=32 {
                let direction = face_uv_to_direction(
                    key.face,
                    -1.0 + 2.0 * (f64::from(key.x) + f64::from(x) / 32.0) / 16.0,
                    -1.0 + 2.0 * (f64::from(key.y) + f64::from(y) / 32.0) / 16.0,
                );
                let detail = baked_surface_detail(key, direction, &noise);
                minimum = minimum.min(detail);
                maximum = maximum.max(detail);
            }
        }
        assert!(maximum - minimum > 20.0);
        assert_eq!(
            baked_surface_detail(
                TileKey {
                    face: CubeFace::PositiveX,
                    level: 18,
                    x: 1 << 17,
                    y: 1 << 17,
                },
                DVec3::X,
                &noise,
            ),
            0.0
        );
    }

    #[test]
    fn level_four_biome_detail_breaks_up_land_materials() {
        let key = TileKey {
            face: CubeFace::PositiveZ,
            level: 4,
            x: 8,
            y: 8,
        };
        let noise = Perlin::new(0xABCD_0123);
        let biomes: std::collections::BTreeSet<_> = (0..=32)
            .flat_map(|y| (0..=32).map(move |x| (x, y)))
            .map(|(x, y)| {
                let direction = face_uv_to_direction(
                    key.face,
                    -1.0 + 2.0 * (f64::from(key.x) + f64::from(x) / 32.0) / 16.0,
                    -1.0 + 2.0 * (f64::from(key.y) + f64::from(y) / 32.0) / 16.0,
                );
                baked_biome_detail(
                    key,
                    direction,
                    500.0,
                    128,
                    BiomeId::TemperateGrassland as u8,
                    &noise,
                )
            })
            .collect();
        assert!(biomes.len() >= 2);
    }
}

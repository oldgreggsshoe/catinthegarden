use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use catinthegarden_coretypes::{
    BiomeId, BiomeManifestEntry, ChannelManifest, CubeFace, OUTMAP_SCHEMA_VERSION, OutmapManifest,
    PLANET_RADIUS_METERS, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey,
    direction_to_face_uv, face_uv_to_direction, tile_key_for_direction,
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
const BAKED_DETAIL_START_LEVEL: u8 = 3;
const BAKED_DETAIL_MAX_AMPLITUDE_METERS: f64 = 300.0;
const BAKED_DETAIL_BASE_FREQUENCY: f64 = 40.0;
const LANDING_DETAIL_PROTECTION_METERS: f64 = 30.0;
/// Progressively stored, band-limited detail. Each octave appears only once
/// its tile sampling can resolve it; all frequencies are planet-direction
/// based, so cube-face and tile boundaries remain deterministic.
const SPARSE_DETAIL_BANDS: [(u8, f64, f64); 7] = [
    (6, 512.0, 120.0),
    (8, 2_048.0, 45.0),
    (10, 8_192.0, 14.0),
    (12, 32_768.0, 4.0),
    (14, 131_072.0, 1.0),
    (16, 524_288.0, 0.25),
    (18, 2_097_152.0, 0.06),
];
const BAKED_BIOME_DETAIL_START_LEVEL: u8 = 3;
const BAKED_BIOME_DETAIL_FREQUENCY: f64 = 280.0;

pub fn export_outmap(config: &BakeConfig, terrain: &Terrain) -> BakeResult<OutmapManifest> {
    fs::create_dir_all(&config.output)?;
    write_previews(&config.output.join("previews"), terrain)?;
    let landing_direction = terrain.sparse_landing_direction();
    let available_tiles = available_tile_keys(config, landing_direction);
    let mut generated_tiles = BTreeMap::new();
    let microrelief = Perlin::new(config.seed ^ 0x4D49_4352);
    for &key in &available_tiles {
        let mut tile = sample_tile(terrain, key, landing_direction, &microrelief);
        if let Some(parent) = key.parent() {
            let parent_tile = generated_tiles
                .get(&parent)
                .expect("available tile ordering must place parents before children");
            constrain_logical_border_to_parent(&mut tile, key, parent_tile);
        }
        write_tile(&config.output, key, &tile)?;
        generated_tiles.insert(key, tile);
    }
    let manifest = build_manifest(config, landing_direction, available_tiles);
    manifest
        .validate()
        .map_err(|message| io::Error::new(ErrorKind::InvalidData, message))?;
    fs::write(
        config.output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

/// Expands the sparse high-resolution corridor without rerunning the global
/// erosion pipeline. Existing dense tiles remain the authoritative macro
/// terrain; new sparse tiles bilinearly refine those samples and add only the
/// deterministic, level-band-limited detail defined above.
pub fn refine_existing_outmap(output: &Path) -> BakeResult<OutmapManifest> {
    let existing: OutmapManifest =
        serde_json::from_slice(&fs::read(output.join("manifest.json"))?)?;
    existing
        .validate()
        .map_err(|message| io::Error::new(ErrorKind::InvalidData, message))?;
    let config = BakeConfig {
        output: output.to_path_buf(),
        seed: existing.seed,
        width: existing.working_width as usize,
        height: existing.working_height as usize,
        dense_level: existing.dense_level,
        max_level: existing.max_level,
        sparse_radius: None,
        // This path does not regenerate the global terrain, but retaining a
        // valid non-zero value keeps BakeConfig's invariants explicit.
        erosion_iterations: 1,
    };
    config
        .validate()
        .map_err(|message| io::Error::new(ErrorKind::InvalidInput, message))?;

    let landing_direction = DVec3::from_array(existing.sparse_landing_direction);
    let available_tiles = available_tile_keys(&config, landing_direction);
    let detail_noise = Perlin::new(config.seed ^ 0x4D49_4352);
    let mut dense_tiles = BTreeMap::new();
    let mut generated_tiles = BTreeMap::new();
    for &key in available_tiles
        .iter()
        .filter(|key| key.level > config.dense_level)
    {
        let mut tile = sample_refined_tile_from_dense(
            output,
            config.dense_level,
            key,
            landing_direction,
            &detail_noise,
            &mut dense_tiles,
        )?;
        let parent = key.parent().expect("sparse tiles have a parent");
        let parent_tile = if parent.level <= config.dense_level {
            load_tile_cached(output, parent, &mut dense_tiles)?.clone()
        } else {
            generated_tiles
                .get(&parent)
                .cloned()
                .expect("available tile ordering must place parents before children")
        };
        constrain_logical_border_to_parent(&mut tile, key, &parent_tile);
        write_tile(output, key, &tile)?;
        generated_tiles.insert(key, tile);
    }

    let manifest = build_manifest(&config, landing_direction, available_tiles);
    manifest
        .validate()
        .map_err(|message| io::Error::new(ErrorKind::InvalidData, message))?;
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    validate_output(output)?;
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
    let landing_direction = DVec3::from_array(manifest.sparse_landing_direction);
    let key = tile_key_for_direction(landing_direction, level);
    if !manifest.has_tile(key) {
        return Err(invalid_data("missing landing tile at maximum level"));
    }
    let (_, u, v) = direction_to_face_uv(landing_direction);
    let side = f64::from(1_u32 << level);
    let tile_u = ((u + 1.0) * 0.5 * side - f64::from(key.x)).clamp(0.0, 1.0);
    let tile_v = ((v + 1.0) * 0.5 * side - f64::from(key.y)).clamp(0.0, 1.0);
    let stored_x = TILE_GUTTER + (tile_u * f64::from(TILE_LOGICAL_SIZE - 1)).round() as u32;
    let stored_y = TILE_GUTTER + (tile_v * f64::from(TILE_LOGICAL_SIZE - 1)).round() as u32;
    let sample_index = (stored_y * TILE_STORED_SIZE + stored_x) as usize;
    let directory = output.join(key.relative_path());
    let bytes = fs::read(directory.join(HEIGHT_FILE))?;
    let start = sample_index * size_of::<f32>();
    let height = f32::from_le_bytes(
        bytes[start..start + size_of::<f32>()]
            .try_into()
            .expect("height sample has four bytes"),
    );
    let biome = fs::read(directory.join(BIOME_FILE))?[sample_index];
    if height <= 0.0
        || matches!(
            BiomeId::try_from(biome),
            Ok(BiomeId::Ocean | BiomeId::Lake | BiomeId::Ice)
        )
    {
        return Err(invalid_data(format!(
            "sparse landing site must be dry land, got height {height}m and biome {biome}"
        )));
    }
    Ok(())
}

fn invalid_data(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(io::Error::new(ErrorKind::InvalidData, message.into()))
}

fn build_manifest(
    config: &BakeConfig,
    landing_direction: DVec3,
    available_tiles: Vec<TileKey>,
) -> OutmapManifest {
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
        sparse_landing_direction: landing_direction.to_array(),
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

pub fn available_tile_keys(config: &BakeConfig, landing_direction: DVec3) -> Vec<TileKey> {
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
    let (landing_face, landing_u, landing_v) = direction_to_face_uv(landing_direction);
    for level in config.dense_level.saturating_add(1)..=config.max_level {
        let side = 1_u32 << level;
        let center = tile_key_for_direction(landing_direction, level);
        let sparse_radius = sparse_radius_for_level(config, level);
        debug_assert_eq!(center.face, landing_face);
        debug_assert!((-1.0..=1.0).contains(&landing_u));
        debug_assert!((-1.0..=1.0).contains(&landing_v));
        let start_x = center.x.saturating_sub(sparse_radius);
        let start_y = center.y.saturating_sub(sparse_radius);
        let end_x = center.x.saturating_add(sparse_radius).min(side - 1);
        let end_y = center.y.saturating_add(sparse_radius).min(side - 1);
        for y in start_y..=end_y {
            for x in start_x..=end_x {
                let mut key = Some(TileKey {
                    face: landing_face,
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

/// Default sparse coverage grows in tile count while individual tiles shrink,
/// then tapers at the metre-scale levels. This keeps several kilometres of
/// real source data in a low-flight view instead of a fixed 3x3 footprint that
/// collapses to only a few metres at L18.
pub fn sparse_radius_for_level(config: &BakeConfig, level: u8) -> u32 {
    if let Some(radius) = config.sparse_radius {
        return radius;
    }
    match level {
        0..=10 => 1,
        11 => 2,
        12 => 3,
        13 => 4,
        14 => 6,
        15 | 16 => 8,
        17 => 6,
        _ => 4,
    }
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

fn load_tile_cached<'a>(
    output: &Path,
    key: TileKey,
    cache: &'a mut BTreeMap<TileKey, TileData>,
) -> BakeResult<&'a TileData> {
    if !cache.contains_key(&key) {
        let directory = output.join(key.relative_path());
        let height_bytes = fs::read(directory.join(HEIGHT_FILE))?;
        let biome = fs::read(directory.join(BIOME_FILE))?;
        let moisture = fs::read(directory.join(MOISTURE_FILE))?;
        let sample_count = (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize;
        if height_bytes.len() != sample_count * size_of::<f32>()
            || biome.len() != sample_count
            || moisture.len() != sample_count
        {
            return Err(invalid_data(format!("bad source tile payload for {key:?}")));
        }
        let height = height_bytes
            .chunks_exact(size_of::<f32>())
            .map(|bytes| {
                f32::from_le_bytes(bytes.try_into().expect("height sample has four bytes"))
            })
            .collect();
        cache.insert(
            key,
            TileData {
                height,
                biome,
                moisture,
            },
        );
    }
    Ok(cache.get(&key).expect("tile was inserted into cache"))
}

fn sample_refined_tile_from_dense(
    output: &Path,
    dense_level: u8,
    key: TileKey,
    landing_direction: DVec3,
    noise: &Perlin,
    dense_tiles: &mut BTreeMap<TileKey, TileData>,
) -> BakeResult<TileData> {
    let sample_count = (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize;
    let mut height = Vec::with_capacity(sample_count);
    let mut biome = Vec::with_capacity(sample_count);
    let mut moisture = Vec::with_capacity(sample_count);
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
            let source_key = tile_key_for_direction(direction, dense_level);
            let source = load_tile_cached(output, source_key, dense_tiles)?;
            let (_, source_u, source_v) = direction_to_face_uv(direction);
            let source_side = f64::from(1_u32 << dense_level);
            let source_x = ((source_u + 1.0) * 0.5 * source_side - f64::from(source_key.x))
                * f64::from(TILE_LOGICAL_SIZE - 1);
            let source_y = ((source_v + 1.0) * 0.5 * source_side - f64::from(source_key.y))
                * f64::from(TILE_LOGICAL_SIZE - 1);
            let source_x = source_x.clamp(0.0, f64::from(TILE_LOGICAL_SIZE - 1));
            let source_y = source_y.clamp(0.0, f64::from(TILE_LOGICAL_SIZE - 1));
            let macro_height = f64::from(sample_parent_f32(&source.height, source_x, source_y));
            let old_landing_ramp =
                landing_detail_ramp_with_radius(direction, landing_direction, 500.0);
            let broad_detail_correction = unprotected_baked_surface_detail(direction, noise)
                * (landing_detail_ramp(direction, landing_direction) - old_landing_ramp);
            let detail = broad_detail_correction
                + sparse_surface_detail(key, direction, landing_direction, noise);
            let sampled_height = (macro_height + detail * smoothstep(25.0, 150.0, macro_height))
                .clamp(MIN_HEIGHT_METERS, MAX_HEIGHT_METERS)
                as f32;
            height.push(sampled_height);
            biome.push(sample_parent_u8_nearest(&source.biome, source_x, source_y));
            moisture.push(sample_parent_u8_linear(
                &source.moisture,
                source_x,
                source_y,
            ));
        }
    }
    Ok(TileData {
        height,
        biome,
        moisture,
    })
}

fn sample_tile(
    terrain: &Terrain,
    key: TileKey,
    landing_direction: DVec3,
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
            let macro_height = terrain.grid.sample_f64(&terrain.height_meters, direction);
            let detail = baked_surface_detail(key, direction, landing_direction, microrelief)
                + sparse_surface_detail(key, direction, landing_direction, microrelief);
            // Do not move the coastline: introduce relief only after the base
            // baker has established dry land, then reach full strength at the
            // selected low-flight site's modest coastal elevation.
            let land_weight = smoothstep(25.0, 150.0, macro_height);
            let sampled_height = macro_height + detail * land_weight;
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

fn baked_surface_detail(
    key: TileKey,
    direction: DVec3,
    landing_direction: DVec3,
    noise: &Perlin,
) -> f64 {
    if key.level < BAKED_DETAIL_START_LEVEL {
        return 0.0;
    }

    let level_ramp = (f64::from(key.level - BAKED_DETAIL_START_LEVEL + 1) / 2.0).min(1.0);

    // Keep the deterministic inspection centre locally safe while retaining
    // the generated coastal terrain around it.
    unprotected_baked_surface_detail(direction, noise)
        * level_ramp
        * landing_detail_ramp(direction, landing_direction)
}

fn unprotected_baked_surface_detail(direction: DVec3, noise: &Perlin) -> f64 {
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
    (sample(BAKED_DETAIL_BASE_FREQUENCY) * 0.58
        + sample(BAKED_DETAIL_BASE_FREQUENCY * 2.0) * 0.29
        + sample(BAKED_DETAIL_BASE_FREQUENCY * 4.0) * 0.13)
        * BAKED_DETAIL_MAX_AMPLITUDE_METERS
}

fn sparse_surface_detail(
    key: TileKey,
    direction: DVec3,
    landing_direction: DVec3,
    noise: &Perlin,
) -> f64 {
    let detail = SPARSE_DETAIL_BANDS
        .iter()
        .filter(|(minimum_level, _, _)| key.level >= *minimum_level)
        .map(|(_, frequency, amplitude_meters)| {
            let sample = |sample_direction: DVec3| {
                noise.get([
                    sample_direction.x * frequency,
                    sample_direction.y * frequency,
                    sample_direction.z * frequency,
                ])
            };
            (sample(direction) - sample(landing_direction)).clamp(-1.0, 1.0) * amplitude_meters
        })
        .sum::<f64>();
    detail * landing_detail_ramp(direction, landing_direction)
}

fn landing_detail_ramp(direction: DVec3, landing_direction: DVec3) -> f64 {
    landing_detail_ramp_with_radius(
        direction,
        landing_direction,
        LANDING_DETAIL_PROTECTION_METERS,
    )
}

fn landing_detail_ramp_with_radius(
    direction: DVec3,
    landing_direction: DVec3,
    protection_radius_meters: f64,
) -> f64 {
    let landing_distance_meters =
        direction.dot(landing_direction).clamp(-1.0, 1.0).acos() * PLANET_RADIUS_METERS;
    smoothstep(0.0, protection_radius_meters, landing_distance_meters)
}

fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
    let amount = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    amount * amount * (3.0 - 2.0 * amount)
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
        let landing = face_uv_to_direction(CubeFace::PositiveZ, 0.31, -0.27);
        let keys = available_tile_keys(&config, landing);
        assert!(
            keys.binary_search(&tile_key_for_direction(landing, config.max_level))
                .is_ok()
        );
        for key in &keys {
            if let Some(parent) = key.parent() {
                assert!(keys.binary_search(&parent).is_ok());
            }
        }
    }

    #[test]
    fn adaptive_sparse_radius_preserves_low_flight_source_coverage() {
        let config = BakeConfig::default();
        assert_eq!(sparse_radius_for_level(&config, 10), 1);
        assert_eq!(sparse_radius_for_level(&config, 13), 4);
        assert_eq!(sparse_radius_for_level(&config, 15), 8);
        assert_eq!(sparse_radius_for_level(&config, 18), 4);

        let mut overridden = config;
        overridden.sparse_radius = Some(2);
        assert_eq!(sparse_radius_for_level(&overridden, 15), 2);
    }

    #[test]
    fn sparse_height_bands_add_resolvable_detail_without_roughening_the_spawn_point() {
        let noise = Perlin::new(0xABCD_0123);
        let landing = DVec3::X;
        let nearby = face_uv_to_direction(CubeFace::PositiveX, 0.002, -0.001);
        let node = |level| TileKey {
            face: CubeFace::PositiveX,
            level,
            x: 1_u32 << (level - 1),
            y: 1_u32 << (level - 1),
        };

        assert_eq!(sparse_surface_detail(node(5), nearby, landing, &noise), 0.0);
        assert_ne!(sparse_surface_detail(node(6), nearby, landing, &noise), 0.0);
        assert_eq!(
            sparse_surface_detail(node(18), landing, landing, &noise),
            0.0
        );
        assert!(LANDING_DETAIL_PROTECTION_METERS < 50.0);
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
        for y in 0..TILE_LOGICAL_SIZE {
            for x in 0..TILE_LOGICAL_SIZE {
                let direction = face_uv_to_direction(
                    key.face,
                    -1.0 + 2.0
                        * (f64::from(key.x) + f64::from(x) / f64::from(TILE_LOGICAL_SIZE - 1))
                        / 16.0,
                    -1.0 + 2.0
                        * (f64::from(key.y) + f64::from(y) / f64::from(TILE_LOGICAL_SIZE - 1))
                        / 16.0,
                );
                let detail = baked_surface_detail(key, direction, DVec3::X, &noise);
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
        let biomes: std::collections::BTreeSet<_> = (0..TILE_LOGICAL_SIZE)
            .flat_map(|y| (0..TILE_LOGICAL_SIZE).map(move |x| (x, y)))
            .map(|(x, y)| {
                let direction = face_uv_to_direction(
                    key.face,
                    -1.0 + 2.0
                        * (f64::from(key.x) + f64::from(x) / f64::from(TILE_LOGICAL_SIZE - 1))
                        / 16.0,
                    -1.0 + 2.0
                        * (f64::from(key.y) + f64::from(y) / f64::from(TILE_LOGICAL_SIZE - 1))
                        / 16.0,
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

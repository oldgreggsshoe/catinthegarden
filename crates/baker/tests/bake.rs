use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use catinthegarden_baker::{BakeConfig, bake, validate_output};
use catinthegarden_coretypes::{CubeFace, TILE_STORED_SIZE, TileKey};
use image::{ColorType, ImageReader};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn temporary_output(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "catinthegarden-baker-{name}-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ))
}

fn small_config(output: PathBuf) -> BakeConfig {
    BakeConfig {
        output,
        width: 32,
        height: 16,
        dense_level: 0,
        max_level: 3,
        sparse_radius: 0,
        erosion_iterations: 4,
        ..BakeConfig::default()
    }
}

#[test]
fn complete_bake_validates_and_is_byte_deterministic() {
    let first_output = temporary_output("first");
    let second_output = temporary_output("second");
    let first = bake(&small_config(first_output.clone())).unwrap();
    let second = bake(&small_config(second_output.clone())).unwrap();
    assert_eq!(first, second);
    validate_output(&first_output).unwrap();
    validate_output(&second_output).unwrap();
    assert_trees_equal(&first_output, &second_output, Path::new(""));
}

#[test]
fn tiles_have_gutters_expected_channel_sizes_and_level_eighteen_refinement() {
    let output = temporary_output("formats");
    let config = BakeConfig {
        output: output.clone(),
        width: 32,
        height: 16,
        dense_level: 0,
        max_level: 18,
        sparse_radius: 0,
        erosion_iterations: 2,
        ..BakeConfig::default()
    };
    let manifest = bake(&config).unwrap();
    let level_eighteen = TileKey {
        face: CubeFace::PositiveX,
        level: 18,
        x: (1 << 18) / 2,
        y: (1 << 18) / 2,
    };
    assert!(manifest.has_tile(level_eighteen));
    let directory = output.join(level_eighteen.relative_path());
    let samples = (TILE_STORED_SIZE * TILE_STORED_SIZE) as u64;
    assert_eq!(
        fs::metadata(directory.join("height.r32f")).unwrap().len(),
        samples * 4
    );
    assert_eq!(
        fs::metadata(directory.join("biome.r8")).unwrap().len(),
        samples
    );
    assert_eq!(
        fs::metadata(directory.join("moisture.r8")).unwrap().len(),
        samples
    );
    assert!(!directory.join("normal.r8").exists());
    let height = fs::read(directory.join("height.r32f")).unwrap();
    assert_eq!(height_sample(&height, 1, 1), -10.0);
    let interior: Vec<f32> = (2..33)
        .flat_map(|row| (2..33).map(move |column| (row, column)))
        .map(|(row, column)| height_sample(&height, row, column))
        .collect();
    let minimum = interior.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = interior.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(maximum - minimum > 0.1, "L18 microrelief was flat");
    assert!(minimum >= -12.001 && maximum <= -7.999);
}

#[test]
fn tile_edges_gutters_parent_samples_and_previews_are_consistent() {
    let output = temporary_output("seams");
    let config = BakeConfig {
        output: output.clone(),
        width: 32,
        height: 16,
        dense_level: 2,
        max_level: 2,
        sparse_radius: 0,
        erosion_iterations: 2,
        ..BakeConfig::default()
    };
    bake(&config).unwrap();
    let left = TileKey {
        face: CubeFace::PositiveX,
        level: 1,
        x: 0,
        y: 0,
    };
    let right = TileKey { x: 1, ..left };
    let left_height = read_channel(&output, left, "height.r32f");
    let right_height = read_channel(&output, right, "height.r32f");
    let left_biome = read_channel(&output, left, "biome.r8");
    let right_biome = read_channel(&output, right, "biome.r8");
    for row in 0..TILE_STORED_SIZE as usize {
        assert_eq!(
            height_sample(&left_height, row, 33),
            height_sample(&right_height, row, 1)
        );
        if (2..=32).contains(&row) {
            assert_eq!(
                height_sample(&left_height, row, 34),
                height_sample(&right_height, row, 2)
            );
        }
        assert_eq!(
            r8_sample(&left_biome, row, 33),
            r8_sample(&right_biome, row, 1)
        );
    }

    let parent = TileKey::root(CubeFace::PositiveX);
    let parent_height = read_channel(&output, parent, "height.r32f");
    for y in 0..=16 {
        for x in 0..=16 {
            assert_eq!(
                height_sample(&parent_height, 1 + y, 1 + x),
                height_sample(&left_height, 1 + y * 2, 1 + x * 2)
            );
        }
    }

    let negative_z = TileKey::root(CubeFace::NegativeZ);
    let negative_z_height = read_channel(&output, negative_z, "height.r32f");
    for row in 1..=33 {
        assert_eq!(
            height_sample(&parent_height, row, 33),
            height_sample(&negative_z_height, row, 1)
        );
    }

    let positive_x_edge_child = TileKey {
        face: CubeFace::PositiveX,
        level: 1,
        x: 1,
        y: 0,
    };
    let negative_z_edge_child = TileKey {
        face: CubeFace::NegativeZ,
        level: 1,
        x: 0,
        y: 0,
    };
    let positive_x_child_height = read_channel(&output, positive_x_edge_child, "height.r32f");
    let negative_z_child_height = read_channel(&output, negative_z_edge_child, "height.r32f");
    for row in 1..=33 {
        assert_eq!(
            height_sample(&positive_x_child_height, row, 33),
            height_sample(&negative_z_child_height, row, 1)
        );
    }

    let across_parent_left = TileKey {
        face: CubeFace::PositiveX,
        level: 2,
        x: 1,
        y: 0,
    };
    let across_parent_right = TileKey {
        x: 2,
        ..across_parent_left
    };
    let across_parent_left_height = read_channel(&output, across_parent_left, "height.r32f");
    let across_parent_right_height = read_channel(&output, across_parent_right, "height.r32f");
    for row in 1..=33 {
        assert_eq!(
            height_sample(&across_parent_left_height, row, 33),
            height_sample(&across_parent_right_height, row, 1)
        );
    }

    let height_preview = ImageReader::open(output.join("previews/height.png"))
        .unwrap()
        .decode()
        .unwrap();
    let biome_preview = ImageReader::open(output.join("previews/biome.png"))
        .unwrap()
        .decode()
        .unwrap();
    let moisture_preview = ImageReader::open(output.join("previews/moisture.png"))
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!(height_preview.width(), config.width as u32);
    assert_eq!(height_preview.height(), config.height as u32);
    assert_eq!(height_preview.color(), ColorType::L16);
    assert_eq!(biome_preview.color(), ColorType::Rgb8);
    assert_eq!(moisture_preview.color(), ColorType::L8);
    let height_values = height_preview.to_luma16().into_raw();
    let biome_values = biome_preview.to_rgb8().into_raw();
    let moisture_values = moisture_preview.to_luma8().into_raw();
    assert_ne!(height_values.iter().min(), height_values.iter().max());
    assert!(
        biome_values
            .chunks_exact(3)
            .any(|color| color != &biome_values[0..3])
    );
    assert_ne!(moisture_values.iter().min(), moisture_values.iter().max());
}

#[test]
fn deep_sparse_edge_matches_root_fallback_interpolation() {
    let output = temporary_output("ancestor-seam");
    let config = BakeConfig {
        output: output.clone(),
        width: 32,
        height: 16,
        dense_level: 0,
        max_level: 8,
        sparse_radius: 0,
        erosion_iterations: 2,
        ..BakeConfig::default()
    };
    let manifest = bake(&config).unwrap();
    let side = 1_u32 << config.max_level;
    let deep = TileKey {
        face: CubeFace::PositiveX,
        level: config.max_level,
        x: side / 2,
        y: side / 2,
    };
    let missing_neighbor = TileKey {
        x: deep.x - 1,
        ..deep
    };
    let fallback = manifest
        .best_available_ancestor(missing_neighbor)
        .expect("root fallback exists");
    assert_eq!(fallback, TileKey::root(CubeFace::PositiveX));

    let deep_height = read_channel(&output, deep, "height.r32f");
    let root_height = read_channel(&output, fallback, "height.r32f");
    for logical_y in 0..TILE_STORED_SIZE - 2 {
        let global_y = u64::from(deep.y) * 32 + u64::from(logical_y);
        let root_y = global_y as f64 / side as f64;
        let expected = bilinear_height(&root_height, 16.0, root_y);
        let actual = height_sample(&deep_height, (logical_y + 1) as usize, 1);
        assert!(
            (actual - expected).abs() <= 0.001,
            "deep edge row {logical_y}: {actual} != root fallback {expected}"
        );
    }
}

fn read_channel(output: &Path, key: TileKey, file: &str) -> Vec<u8> {
    fs::read(output.join(key.relative_path()).join(file)).unwrap()
}

fn height_sample(bytes: &[u8], row: usize, column: usize) -> f32 {
    let index = (row * TILE_STORED_SIZE as usize + column) * 4;
    f32::from_le_bytes(bytes[index..index + 4].try_into().unwrap())
}

fn bilinear_height(bytes: &[u8], x: f64, y: f64) -> f32 {
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(32);
    let y1 = (y0 + 1).min(32);
    let tx = (x - x0 as f64) as f32;
    let ty = (y - y0 as f64) as f32;
    let top = height_sample(bytes, y0 + 1, x0 + 1) * (1.0 - tx)
        + height_sample(bytes, y0 + 1, x1 + 1) * tx;
    let bottom = height_sample(bytes, y1 + 1, x0 + 1) * (1.0 - tx)
        + height_sample(bytes, y1 + 1, x1 + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

fn r8_sample(bytes: &[u8], row: usize, column: usize) -> u8 {
    bytes[row * TILE_STORED_SIZE as usize + column]
}

fn assert_trees_equal(first: &Path, second: &Path, relative: &Path) {
    let mut first_entries: Vec<_> = fs::read_dir(first.join(relative))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let mut second_entries: Vec<_> = fs::read_dir(second.join(relative))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    first_entries.sort();
    second_entries.sort();
    assert_eq!(
        first_entries, second_entries,
        "different entries at {relative:?}"
    );
    for name in first_entries {
        let child = relative.join(name);
        if first.join(&child).is_dir() {
            assert_trees_equal(first, second, &child);
        } else {
            assert_eq!(
                fs::read(first.join(&child)).unwrap(),
                fs::read(second.join(&child)).unwrap(),
                "different file {child:?}"
            );
        }
    }
}

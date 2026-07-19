use std::{
    error::Error,
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

use catinthegarden_coretypes::{BiomeId, OutmapManifest, TILE_STORED_SIZE, TileKey};

const TILE_SAMPLE_COUNT: usize = TILE_STORED_SIZE as usize * TILE_STORED_SIZE as usize;
const HEIGHT_BYTES_PER_SAMPLE: usize = size_of::<f32>();

#[derive(Clone, Debug, PartialEq)]
pub struct TileData {
    pub requested_key: TileKey,
    pub source_key: TileKey,
    pub heights_meters: Vec<f32>,
    pub biome_ids: Vec<u8>,
    pub moisture: Vec<u8>,
}

#[allow(dead_code)]
impl TileData {
    pub fn used_fallback(&self) -> bool {
        self.requested_key != self.source_key
    }
}

#[derive(Clone, Debug)]
pub struct Outmap {
    root: PathBuf,
    manifest: OutmapManifest,
}

#[allow(dead_code)]
impl Outmap {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, OutmapError> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join("manifest.json");
        let manifest_bytes =
            fs::read(&manifest_path).map_err(|source| OutmapError::ReadManifest {
                path: manifest_path.clone(),
                source,
            })?;
        let manifest: OutmapManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|source| {
                OutmapError::ParseManifest {
                    path: manifest_path,
                    source,
                }
            })?;
        manifest
            .validate()
            .and_then(|()| validate_reader_manifest(&manifest))
            .map_err(OutmapError::InvalidManifest)?;
        Ok(Self { root, manifest })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &OutmapManifest {
        &self.manifest
    }

    pub fn resolve_tile(&self, requested_key: TileKey) -> Result<TileKey, OutmapError> {
        if !requested_key.is_valid() {
            return Err(OutmapError::InvalidTileKey(requested_key));
        }
        self.manifest
            .best_available_ancestor(requested_key)
            .ok_or(OutmapError::MissingTile(requested_key))
    }

    pub fn load_tile(&self, requested_key: TileKey) -> Result<TileData, OutmapError> {
        let source_key = self.resolve_tile(requested_key)?;
        let height_bytes = self.read_channel(source_key, "height")?;
        let biome_ids = self.read_channel(source_key, "biome")?;
        let moisture = self.read_channel(source_key, "moisture")?;

        validate_length(
            source_key,
            "height",
            TILE_SAMPLE_COUNT * HEIGHT_BYTES_PER_SAMPLE,
            height_bytes.len(),
        )?;
        validate_length(source_key, "biome", TILE_SAMPLE_COUNT, biome_ids.len())?;
        validate_length(source_key, "moisture", TILE_SAMPLE_COUNT, moisture.len())?;

        let mut heights_meters = Vec::with_capacity(TILE_SAMPLE_COUNT);
        for (index, bytes) in height_bytes
            .chunks_exact(HEIGHT_BYTES_PER_SAMPLE)
            .enumerate()
        {
            let height = f32::from_le_bytes(bytes.try_into().expect("height chunk is four bytes"));
            if !height.is_finite() {
                return Err(OutmapError::NonFiniteHeight {
                    key: source_key,
                    index,
                    value: height,
                });
            }
            if height < self.manifest.height_min_meters || height > self.manifest.height_max_meters
            {
                return Err(OutmapError::HeightOutOfRange {
                    key: source_key,
                    index,
                    value: height,
                    minimum: self.manifest.height_min_meters,
                    maximum: self.manifest.height_max_meters,
                });
            }
            heights_meters.push(height);
        }
        for (index, &biome_id) in biome_ids.iter().enumerate() {
            if BiomeId::try_from(biome_id).is_err() {
                return Err(OutmapError::InvalidBiomeId {
                    key: source_key,
                    index,
                    value: biome_id,
                });
            }
        }

        Ok(TileData {
            requested_key,
            source_key,
            heights_meters,
            biome_ids,
            moisture,
        })
    }

    fn read_channel(&self, key: TileKey, name: &'static str) -> Result<Vec<u8>, OutmapError> {
        let channel = self
            .manifest
            .channels
            .iter()
            .find(|channel| channel.name == name)
            .expect("reader manifest validation guarantees each channel");
        let path = self.root.join(key.relative_path()).join(&channel.file_name);
        fs::read(&path).map_err(|source| OutmapError::ReadTileChannel {
            key,
            channel: name,
            path,
            source,
        })
    }
}

#[derive(Debug)]
pub enum OutmapError {
    ReadManifest {
        path: PathBuf,
        source: io::Error,
    },
    ParseManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidManifest(String),
    InvalidTileKey(TileKey),
    MissingTile(TileKey),
    ReadTileChannel {
        key: TileKey,
        channel: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidChannelLength {
        key: TileKey,
        channel: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFiniteHeight {
        key: TileKey,
        index: usize,
        value: f32,
    },
    HeightOutOfRange {
        key: TileKey,
        index: usize,
        value: f32,
        minimum: f32,
        maximum: f32,
    },
    InvalidBiomeId {
        key: TileKey,
        index: usize,
        value: u8,
    },
}

impl fmt::Display for OutmapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadManifest { path, source } => {
                write!(
                    formatter,
                    "could not read manifest {}: {source}",
                    path.display()
                )
            }
            Self::ParseManifest { path, source } => {
                write!(
                    formatter,
                    "could not parse manifest {}: {source}",
                    path.display()
                )
            }
            Self::InvalidManifest(reason) => write!(formatter, "invalid outmap manifest: {reason}"),
            Self::InvalidTileKey(key) => write!(formatter, "invalid outmap tile key {key:?}"),
            Self::MissingTile(key) => {
                write!(
                    formatter,
                    "no outmap tile or ancestor is available for {key:?}"
                )
            }
            Self::ReadTileChannel {
                key,
                channel,
                path,
                source,
            } => write!(
                formatter,
                "could not read {channel} channel for {key:?} at {}: {source}",
                path.display()
            ),
            Self::InvalidChannelLength {
                key,
                channel,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid {channel} channel length for {key:?}: expected {expected} bytes, found {actual}"
            ),
            Self::NonFiniteHeight { key, index, value } => write!(
                formatter,
                "non-finite height {value} at sample {index} in {key:?}"
            ),
            Self::HeightOutOfRange {
                key,
                index,
                value,
                minimum,
                maximum,
            } => write!(
                formatter,
                "height {value} at sample {index} in {key:?} is outside [{minimum}, {maximum}]"
            ),
            Self::InvalidBiomeId { key, index, value } => write!(
                formatter,
                "invalid biome id {value} at sample {index} in {key:?}"
            ),
        }
    }
}

impl Error for OutmapError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ReadManifest { source, .. } | Self::ReadTileChannel { source, .. } => {
                Some(source)
            }
            Self::ParseManifest { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn validate_reader_manifest(manifest: &OutmapManifest) -> Result<(), String> {
    for (name, format) in [
        ("height", "r32float_le"),
        ("biome", "r8uint"),
        ("moisture", "r8unorm"),
    ] {
        let matching: Vec<_> = manifest
            .channels
            .iter()
            .filter(|channel| channel.name == name)
            .collect();
        if matching.len() != 1 {
            return Err(format!("manifest must contain exactly one {name} channel"));
        }
        let channel = matching[0];
        if !channel.format.eq_ignore_ascii_case(format) {
            return Err(format!(
                "unsupported {} format {}, expected {format}",
                channel.name, channel.format
            ));
        }
        if !is_safe_file_name(&channel.file_name) {
            return Err(format!(
                "{} channel has unsafe file name {}",
                channel.name, channel.file_name
            ));
        }
    }
    let mut file_names: Vec<_> = manifest
        .channels
        .iter()
        .map(|channel| &channel.file_name)
        .collect();
    file_names.sort_unstable();
    if file_names.windows(2).any(|names| names[0] == names[1]) {
        return Err("terrain channels must use distinct file names".to_owned());
    }

    for biome in BiomeId::ALL {
        let entry = manifest
            .biomes
            .iter()
            .find(|entry| entry.id == biome as u8)
            .ok_or_else(|| format!("manifest is missing biome id {}", biome as u8))?;
        if entry.name != biome.name() || entry.color != biome.color() {
            return Err(format!(
                "manifest biome {} does not match schema",
                biome as u8
            ));
        }
    }
    if manifest
        .sparse_landing_direction
        .iter()
        .any(|component| !component.is_finite())
    {
        return Err("sparse landing direction must be finite".to_owned());
    }
    Ok(())
}

fn is_safe_file_name(file_name: &str) -> bool {
    let mut components = Path::new(file_name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn validate_length(
    key: TileKey,
    channel: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), OutmapError> {
    if actual == expected {
        Ok(())
    } else {
        Err(OutmapError::InvalidChannelLength {
            key,
            channel,
            expected,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use catinthegarden_coretypes::{
        BiomeId, BiomeManifestEntry, ChannelManifest, CubeFace, OUTMAP_SCHEMA_VERSION,
        OutmapManifest, PLANET_RADIUS_METERS, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE,
        TileKey,
    };

    use super::{Outmap, OutmapError, TILE_SAMPLE_COUNT};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "catinthegarden-outmap-{name}-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn root_key() -> TileKey {
        TileKey::root(CubeFace::PositiveX)
    }

    fn valid_manifest() -> OutmapManifest {
        let available_tiles = CubeFace::ALL.into_iter().map(TileKey::root).collect();
        OutmapManifest {
            schema_version: OUTMAP_SCHEMA_VERSION,
            generator: "outmap-reader-test".to_owned(),
            seed: 1,
            planet_radius_meters: PLANET_RADIUS_METERS,
            working_width: 16,
            working_height: 8,
            dense_level: 0,
            max_level: 0,
            tile_logical_size: TILE_LOGICAL_SIZE,
            tile_stored_size: TILE_STORED_SIZE,
            tile_gutter: TILE_GUTTER,
            height_min_meters: -5_000.0,
            height_max_meters: 9_000.0,
            sparse_landing_direction: [1.0, 0.0, 0.0],
            channels: vec![
                ChannelManifest {
                    name: "height".to_owned(),
                    format: "r32float_le".to_owned(),
                    file_name: "height.r32f".to_owned(),
                },
                ChannelManifest {
                    name: "biome".to_owned(),
                    format: "r8uint".to_owned(),
                    file_name: "biome.r8".to_owned(),
                },
                ChannelManifest {
                    name: "moisture".to_owned(),
                    format: "r8unorm".to_owned(),
                    file_name: "moisture.r8".to_owned(),
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

    fn write_manifest(root: &Path, manifest: &OutmapManifest) {
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
    }

    fn write_tile(root: &Path, key: TileKey, heights: &[f32], biome_ids: &[u8]) {
        let directory = root.join(key.relative_path());
        fs::create_dir_all(&directory).expect("create tile directory");
        let height_bytes: Vec<_> = heights
            .iter()
            .flat_map(|height| height.to_le_bytes())
            .collect();
        fs::write(directory.join("height.r32f"), height_bytes).expect("write height");
        fs::write(directory.join("biome.r8"), biome_ids).expect("write biome");
        fs::write(directory.join("moisture.r8"), vec![128; TILE_SAMPLE_COUNT])
            .expect("write moisture");
    }

    fn valid_fixture(name: &str) -> TestDirectory {
        let directory = TestDirectory::new(name);
        write_manifest(directory.path(), &valid_manifest());
        write_tile(
            directory.path(),
            root_key(),
            &vec![123.5; TILE_SAMPLE_COUNT],
            &vec![BiomeId::TemperateGrassland as u8; TILE_SAMPLE_COUNT],
        );
        directory
    }

    #[test]
    fn opens_and_loads_exact_raw_channels() {
        let fixture = valid_fixture("valid");
        let outmap = Outmap::open(fixture.path()).expect("open valid outmap");
        let tile = outmap.load_tile(root_key()).expect("load valid tile");

        assert_eq!(outmap.root(), fixture.path());
        assert_eq!(tile.requested_key, root_key());
        assert_eq!(tile.source_key, root_key());
        assert!(!tile.used_fallback());
        assert_eq!(tile.heights_meters, vec![123.5; TILE_SAMPLE_COUNT]);
        assert_eq!(
            tile.biome_ids,
            vec![BiomeId::TemperateGrassland as u8; TILE_SAMPLE_COUNT]
        );
        assert_eq!(tile.moisture, vec![128; TILE_SAMPLE_COUNT]);
    }

    #[test]
    fn resolves_missing_detail_to_best_available_ancestor() {
        let fixture = valid_fixture("fallback");
        let outmap = Outmap::open(fixture.path()).expect("open valid outmap");
        let requested = TileKey {
            face: CubeFace::PositiveX,
            level: 3,
            x: 5,
            y: 2,
        };

        assert_eq!(
            outmap.resolve_tile(requested).expect("resolve fallback"),
            root_key()
        );
        let tile = outmap.load_tile(requested).expect("load root fallback");
        assert_eq!(tile.requested_key, requested);
        assert_eq!(tile.source_key, root_key());
        assert!(tile.used_fallback());
    }

    #[test]
    fn rejects_missing_tile_payload() {
        let fixture = TestDirectory::new("missing");
        write_manifest(fixture.path(), &valid_manifest());
        let outmap = Outmap::open(fixture.path()).expect("manifest remains valid");

        assert!(matches!(
            outmap.load_tile(root_key()),
            Err(OutmapError::ReadTileChannel {
                channel: "height",
                ..
            })
        ));
    }

    #[test]
    fn rejects_corrupt_or_invalid_manifest() {
        let fixture = TestDirectory::new("corrupt-manifest");
        fs::write(fixture.path().join("manifest.json"), b"{not-json")
            .expect("write corrupt manifest");
        assert!(matches!(
            Outmap::open(fixture.path()),
            Err(OutmapError::ParseManifest { .. })
        ));

        let mut invalid = valid_manifest();
        invalid.schema_version += 1;
        write_manifest(fixture.path(), &invalid);
        assert!(matches!(
            Outmap::open(fixture.path()),
            Err(OutmapError::InvalidManifest(_))
        ));
    }

    #[test]
    fn rejects_truncated_raw_channel() {
        let fixture = valid_fixture("truncated");
        let path = fixture
            .path()
            .join(root_key().relative_path())
            .join("height.r32f");
        fs::write(path, vec![0; TILE_SAMPLE_COUNT * size_of::<f32>() - 1])
            .expect("truncate height");
        let outmap = Outmap::open(fixture.path()).expect("open manifest");

        assert!(matches!(
            outmap.load_tile(root_key()),
            Err(OutmapError::InvalidChannelLength {
                channel: "height",
                ..
            })
        ));
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_height() {
        let fixture = valid_fixture("bad-height");
        write_tile(
            fixture.path(),
            root_key(),
            &vec![f32::NAN; TILE_SAMPLE_COUNT],
            &vec![BiomeId::Ocean as u8; TILE_SAMPLE_COUNT],
        );
        let outmap = Outmap::open(fixture.path()).expect("open manifest");
        assert!(matches!(
            outmap.load_tile(root_key()),
            Err(OutmapError::NonFiniteHeight { index: 0, .. })
        ));

        write_tile(
            fixture.path(),
            root_key(),
            &vec![9_001.0; TILE_SAMPLE_COUNT],
            &vec![BiomeId::Ocean as u8; TILE_SAMPLE_COUNT],
        );
        assert!(matches!(
            outmap.load_tile(root_key()),
            Err(OutmapError::HeightOutOfRange { index: 0, .. })
        ));
    }

    #[test]
    fn rejects_unknown_biome_id() {
        let fixture = valid_fixture("bad-biome");
        write_tile(
            fixture.path(),
            root_key(),
            &vec![0.0; TILE_SAMPLE_COUNT],
            &vec![u8::MAX; TILE_SAMPLE_COUNT],
        );
        let outmap = Outmap::open(fixture.path()).expect("open manifest");

        assert!(matches!(
            outmap.load_tile(root_key()),
            Err(OutmapError::InvalidBiomeId {
                index: 0,
                value: u8::MAX,
                ..
            })
        ));
    }
}

use std::path::PathBuf;

use glam::DVec3;
use serde::{Deserialize, Serialize};

pub const OUTMAP_SCHEMA_VERSION: u32 = 2;
pub const PLANET_RADIUS_METERS: f64 = 4_000_000.0;
pub const QUADTREE_MAX_LEVEL: u8 = 18;
pub const MAX_DENSE_LEVEL: u8 = 5;
// Outmap samples are deliberately denser than the fixed 33x33 terrain mesh.
// Height, normals, biomes, and shoreline material can therefore gain detail
// without changing the Phase 2 chunk topology.
pub const TILE_LOGICAL_SIZE: u32 = 129;
pub const TILE_GUTTER: u32 = 1;
pub const TILE_STORED_SIZE: u32 = TILE_LOGICAL_SIZE + TILE_GUTTER * 2;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CubeFace {
    PositiveX = 0,
    NegativeX = 1,
    PositiveY = 2,
    NegativeY = 3,
    PositiveZ = 4,
    NegativeZ = 5,
}

impl CubeFace {
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    pub const fn index(self) -> u8 {
        self as u8
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::PositiveX => "px",
            Self::NegativeX => "nx",
            Self::PositiveY => "py",
            Self::NegativeY => "ny",
            Self::PositiveZ => "pz",
            Self::NegativeZ => "nz",
        }
    }

    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::PositiveX),
            1 => Some(Self::NegativeX),
            2 => Some(Self::PositiveY),
            3 => Some(Self::NegativeY),
            4 => Some(Self::PositiveZ),
            5 => Some(Self::NegativeZ),
            _ => None,
        }
    }
}

pub fn face_uv_to_direction(face: CubeFace, u: f64, v: f64) -> DVec3 {
    let (normal, tangent_u, tangent_v) = match face {
        CubeFace::PositiveX => (DVec3::X, -DVec3::Z, DVec3::Y),
        CubeFace::NegativeX => (-DVec3::X, DVec3::Z, DVec3::Y),
        CubeFace::PositiveY => (DVec3::Y, DVec3::X, -DVec3::Z),
        CubeFace::NegativeY => (-DVec3::Y, DVec3::X, DVec3::Z),
        CubeFace::PositiveZ => (DVec3::Z, DVec3::X, DVec3::Y),
        CubeFace::NegativeZ => (-DVec3::Z, -DVec3::X, DVec3::Y),
    };
    (normal + tangent_u * u + tangent_v * v).normalize()
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct TileKey {
    pub face: CubeFace,
    pub level: u8,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    pub const fn root(face: CubeFace) -> Self {
        Self {
            face,
            level: 0,
            x: 0,
            y: 0,
        }
    }

    pub fn is_valid(self) -> bool {
        if self.level > QUADTREE_MAX_LEVEL {
            return false;
        }
        let side = 1_u32 << self.level;
        self.x < side && self.y < side
    }

    pub const fn parent(self) -> Option<Self> {
        if self.level == 0 {
            None
        } else {
            Some(Self {
                face: self.face,
                level: self.level - 1,
                x: self.x / 2,
                y: self.y / 2,
            })
        }
    }

    pub fn children(self) -> Option<[Self; 4]> {
        if self.level >= QUADTREE_MAX_LEVEL {
            return None;
        }
        let level = self.level + 1;
        let x = self.x * 2;
        let y = self.y * 2;
        Some([
            Self {
                face: self.face,
                level,
                x,
                y,
            },
            Self {
                face: self.face,
                level,
                x: x + 1,
                y,
            },
            Self {
                face: self.face,
                level,
                x,
                y: y + 1,
            },
            Self {
                face: self.face,
                level,
                x: x + 1,
                y: y + 1,
            },
        ])
    }

    pub fn relative_path(self) -> PathBuf {
        PathBuf::from(format!(
            "tiles/{}/l{:02}/x{:06}_y{:06}",
            self.face.name(),
            self.level,
            self.x,
            self.y
        ))
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BiomeId {
    Ocean = 0,
    Lake = 1,
    Ice = 2,
    Tundra = 3,
    TemperateForest = 4,
    TemperateGrassland = 5,
    TropicalForest = 6,
    Desert = 7,
    MountainRock = 8,
    MountainSnow = 9,
}

impl BiomeId {
    pub const ALL: [Self; 10] = [
        Self::Ocean,
        Self::Lake,
        Self::Ice,
        Self::Tundra,
        Self::TemperateForest,
        Self::TemperateGrassland,
        Self::TropicalForest,
        Self::Desert,
        Self::MountainRock,
        Self::MountainSnow,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Ocean => "ocean",
            Self::Lake => "lake",
            Self::Ice => "ice",
            Self::Tundra => "tundra",
            Self::TemperateForest => "temperate_forest",
            Self::TemperateGrassland => "temperate_grassland",
            Self::TropicalForest => "tropical_forest",
            Self::Desert => "desert",
            Self::MountainRock => "mountain_rock",
            Self::MountainSnow => "mountain_snow",
        }
    }

    pub const fn color(self) -> [u8; 3] {
        match self {
            Self::Ocean => [20, 65, 150],
            Self::Lake => [45, 115, 190],
            Self::Ice => [230, 240, 245],
            Self::Tundra => [130, 145, 120],
            Self::TemperateForest => [45, 105, 55],
            Self::TemperateGrassland => [105, 145, 65],
            Self::TropicalForest => [25, 125, 55],
            Self::Desert => [205, 180, 105],
            Self::MountainRock => [105, 100, 95],
            Self::MountainSnow => [205, 210, 210],
        }
    }
}

impl TryFrom<u8> for BiomeId {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::ALL.get(value as usize).copied().ok_or(value)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChannelManifest {
    pub name: String,
    pub format: String,
    pub file_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BiomeManifestEntry {
    pub id: u8,
    pub name: String,
    pub color: [u8; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OutmapManifest {
    pub schema_version: u32,
    pub generator: String,
    pub seed: u32,
    pub planet_radius_meters: f64,
    pub working_width: u32,
    pub working_height: u32,
    pub dense_level: u8,
    pub max_level: u8,
    pub tile_logical_size: u32,
    pub tile_stored_size: u32,
    pub tile_gutter: u32,
    pub height_min_meters: f32,
    pub height_max_meters: f32,
    pub sparse_landing_direction: [f64; 3],
    pub channels: Vec<ChannelManifest>,
    pub biomes: Vec<BiomeManifestEntry>,
    pub available_tiles: Vec<TileKey>,
}

impl OutmapManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != OUTMAP_SCHEMA_VERSION {
            return Err(format!(
                "unsupported outmap schema {}, expected {}",
                self.schema_version, OUTMAP_SCHEMA_VERSION
            ));
        }
        if self.planet_radius_meters <= 0.0 || !self.planet_radius_meters.is_finite() {
            return Err("planet radius must be finite and positive".to_owned());
        }
        if self.working_width < 4 || self.working_height < 2 {
            return Err("working grid is too small".to_owned());
        }
        if self.dense_level > self.max_level
            || self.dense_level > MAX_DENSE_LEVEL
            || self.max_level > QUADTREE_MAX_LEVEL
        {
            return Err("invalid dense/max level range".to_owned());
        }
        if self.tile_logical_size != TILE_LOGICAL_SIZE
            || self.tile_stored_size != TILE_STORED_SIZE
            || self.tile_gutter != TILE_GUTTER
        {
            return Err("unsupported tile dimensions".to_owned());
        }
        if !self.height_min_meters.is_finite()
            || !self.height_max_meters.is_finite()
            || self.height_min_meters >= self.height_max_meters
        {
            return Err("invalid height range".to_owned());
        }
        if self.channels.len() != 3 {
            return Err("manifest must describe exactly three terrain channels".to_owned());
        }
        if self.biomes.len() != BiomeId::ALL.len() {
            return Err("manifest biome table is incomplete".to_owned());
        }
        for (entry, biome) in self.biomes.iter().zip(BiomeId::ALL) {
            if entry.id != biome as u8 || entry.name != biome.name() || entry.color != biome.color()
            {
                return Err("manifest biome table does not match BiomeId".to_owned());
            }
        }
        let landing = DVec3::from_array(self.sparse_landing_direction);
        if !landing.is_finite() || (landing.length() - 1.0).abs() > 1.0e-9 {
            return Err("sparse landing direction must be normalized".to_owned());
        }
        if self.available_tiles.is_empty() {
            return Err("manifest contains no tiles".to_owned());
        }
        if self
            .available_tiles
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err("available tile keys must be sorted and unique".to_owned());
        }
        for &key in &self.available_tiles {
            if !key.is_valid() || key.level > self.max_level {
                return Err(format!("invalid tile key {key:?}"));
            }
            if let Some(parent) = key.parent()
                && self.available_tiles.binary_search(&parent).is_err()
            {
                return Err(format!("tile {key:?} has no available parent"));
            }
        }
        for face in CubeFace::ALL {
            if self
                .available_tiles
                .binary_search(&TileKey::root(face))
                .is_err()
            {
                return Err(format!("missing root tile for {}", face.name()));
            }
        }
        for level in 0..=self.dense_level {
            let side = 1_u32 << level;
            for face in CubeFace::ALL {
                for y in 0..side {
                    for x in 0..side {
                        let key = TileKey { face, level, x, y };
                        if self.available_tiles.binary_search(&key).is_err() {
                            return Err(format!("missing dense tile {key:?}"));
                        }
                    }
                }
            }
        }
        if self.max_level > self.dense_level
            && !self
                .available_tiles
                .iter()
                .any(|key| key.face == CubeFace::PositiveX && key.level == self.max_level)
        {
            return Err("sparse refinement does not reach max level on +X".to_owned());
        }
        Ok(())
    }

    pub fn has_tile(&self, key: TileKey) -> bool {
        self.available_tiles.binary_search(&key).is_ok()
    }

    pub fn best_available_ancestor(&self, mut key: TileKey) -> Option<TileKey> {
        loop {
            if self.has_tile(key) {
                return Some(key);
            }
            key = key.parent()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_face_indices_and_names_are_stable() {
        for face in CubeFace::ALL {
            assert_eq!(CubeFace::from_index(face.index()), Some(face));
            assert_eq!(face.name().len(), 2);
        }
        assert_eq!(CubeFace::from_index(6), None);
    }

    #[test]
    fn face_centers_and_edges_are_normalized() {
        assert_eq!(
            face_uv_to_direction(CubeFace::PositiveX, 0.0, 0.0),
            DVec3::X
        );
        for face in CubeFace::ALL {
            let direction = face_uv_to_direction(face, 1.0, -1.0);
            assert!((direction.length() - 1.0).abs() < 1.0e-12);
        }
    }

    #[test]
    fn tile_parent_and_path_are_stable() {
        let key = TileKey {
            face: CubeFace::PositiveX,
            level: 3,
            x: 5,
            y: 2,
        };
        assert!(key.is_valid());
        assert_eq!(key.parent().unwrap().x, 2);
        assert_eq!(
            key.relative_path(),
            PathBuf::from("tiles/px/l03/x000005_y000002")
        );
    }

    #[test]
    fn biome_ids_are_dense_and_round_trip() {
        for biome in BiomeId::ALL {
            assert_eq!(BiomeId::try_from(biome as u8), Ok(biome));
            assert!(!biome.name().is_empty());
        }
        assert_eq!(BiomeId::try_from(10), Err(10));
    }

    #[test]
    fn manifest_falls_back_to_nearest_available_parent() {
        let child = TileKey {
            face: CubeFace::PositiveX,
            level: 1,
            x: 1,
            y: 1,
        };
        let mut available_tiles: Vec<_> = CubeFace::ALL
            .into_iter()
            .map(TileKey::root)
            .chain(std::iter::once(child))
            .collect();
        available_tiles.sort();
        let manifest = OutmapManifest {
            schema_version: OUTMAP_SCHEMA_VERSION,
            generator: "test".to_owned(),
            seed: 1,
            planet_radius_meters: PLANET_RADIUS_METERS,
            working_width: 16,
            working_height: 8,
            dense_level: 0,
            max_level: 1,
            tile_logical_size: TILE_LOGICAL_SIZE,
            tile_stored_size: TILE_STORED_SIZE,
            tile_gutter: TILE_GUTTER,
            height_min_meters: -5_000.0,
            height_max_meters: 9_000.0,
            sparse_landing_direction: DVec3::X.to_array(),
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
        };
        manifest.validate().unwrap();
        let missing_grandchild = TileKey {
            face: CubeFace::PositiveX,
            level: 2,
            x: 3,
            y: 3,
        };
        assert_eq!(
            manifest.best_available_ancestor(missing_grandchild),
            Some(child)
        );
    }
}

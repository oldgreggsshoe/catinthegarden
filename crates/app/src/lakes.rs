//! Inland lakes from the outmap's lake sidecar (baker `--derive-lakes`).
//!
//! Per cube face, the water level of the lake a point belongs to (stored,
//! unscaled metres; `NO_LAKE_LEVEL` where there is none), and the land biome
//! to paint where the bake's biome map says `Lake`. The terrain draws water
//! wherever the presented level is above the drawn ground, so the shoreline is
//! where the detailed ground meets a flat level. `lake_level_meters` in
//! planet.wgsl samples the same faces the same way.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use glam::DVec3;
use planet_coretypes::{
    CubeFace, direction_to_face_uv,
    lakes::{
        DIRECTORY, LAND_BIOME_CHANNEL, LEVEL_CHANNEL, NO_LAKE_LEVEL, SCHEMA_VERSION,
        face_file_name, fnv1a,
    },
};

/// Levels below this are "no lake" (the stored sentinel blended in).
pub const NO_LAKE_THRESHOLD_METERS: f64 = NO_LAKE_LEVEL as f64 * 0.5;

pub struct LakeField {
    /// Samples along a face edge, gutter included.
    extent: usize,
    levels: Vec<f32>,
    land_biomes: Vec<u8>,
    /// Per face, which blocks of `BLOCKS[i]` texels hold any lake level,
    /// finest first: chunks whose footprint touches none skip the vertex
    /// lookup.
    blocks: [Vec<bool>; 3],
}

/// Texels along a side of an occupancy block, per pyramid level.
const BLOCKS: [usize; 3] = [4, 16, 64];

impl LakeField {
    /// One shared copy per outmap: the terrain and the road worker's village
    /// siting read the same field. `PLANET_LAKES=0` draws no lakes (A/B).
    pub fn shared(root: &Path) -> Option<Arc<Self>> {
        if matches!(std::env::var("PLANET_LAKES").as_deref(), Ok("0" | "off")) {
            return None;
        }
        static FIELDS: OnceLock<Mutex<HashMap<PathBuf, Option<Arc<LakeField>>>>> = OnceLock::new();
        let mut fields = FIELDS.get_or_init(Default::default).lock().expect("lake field cache");
        fields
            .entry(root.to_path_buf())
            .or_insert_with(|| Self::open(root).map(Arc::new))
            .clone()
    }

    /// The sidecar manifest's bytes, which change whenever the lakes do.
    pub fn manifest_bytes(root: &Path) -> Vec<u8> {
        fs::read(root.join(DIRECTORY).join("manifest.json")).unwrap_or_default()
    }

    /// The sidecar beside the outmap at `root`, if there is one derived from
    /// this outmap. A missing or stale sidecar draws no lakes.
    fn open(root: &Path) -> Option<Self> {
        match Self::read(root) {
            Ok(field) => field,
            Err(error) => {
                tracing::warn!(target: "planet::lakes", %error, "lake sidecar not loaded");
                None
            }
        }
    }

    fn read(root: &Path) -> Result<Option<Self>, String> {
        let directory = root.join(DIRECTORY);
        let Ok(sidecar) = fs::read(directory.join("manifest.json")) else {
            return Ok(None);
        };
        let sidecar: serde_json::Value =
            serde_json::from_slice(&sidecar).map_err(|e| e.to_string())?;
        let manifest = fs::read(root.join("manifest.json")).map_err(|e| e.to_string())?;
        if sidecar["schema_version"].as_u64() != Some(u64::from(SCHEMA_VERSION)) {
            return Err("lake sidecar schema".into());
        }
        if sidecar["source_manifest_fnv1a"].as_str() != Some(fnv1a(&manifest).as_str()) {
            return Err("lake sidecar was derived from a different outmap; rerun --derive-lakes".into());
        }
        let samples = sidecar["face_samples"].as_u64().ok_or("lake sidecar face_samples")? as usize;
        let extent = samples + 2;
        let mut levels = Vec::with_capacity(6 * extent * extent);
        let mut land_biomes = Vec::with_capacity(6 * extent * extent);
        for face in CubeFace::ALL {
            let bytes = fs::read(directory.join(face_file_name(face, LEVEL_CHANNEL)))
                .map_err(|e| e.to_string())?;
            let biomes = fs::read(directory.join(face_file_name(face, LAND_BIOME_CHANNEL)))
                .map_err(|e| e.to_string())?;
            if bytes.len() != extent * extent * 4 || biomes.len() != extent * extent {
                return Err(format!("lake sidecar face {face:?} size"));
            }
            levels.extend(bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())));
            land_biomes.extend(biomes);
        }
        tracing::info!(target: "planet::lakes", lakes = sidecar["lakes"].as_u64(), samples, "lake sidecar loaded");
        Ok(Some(Self::new(extent, levels, land_biomes)))
    }

    fn new(extent: usize, levels: Vec<f32>, land_biomes: Vec<u8>) -> Self {
        let blocks = BLOCKS.map(|block| {
            let side = extent.div_ceil(block);
            let mut blocks = vec![false; 6 * side * side];
            for face in 0..6 {
                for y in 0..extent {
                    for x in 0..extent {
                        if f64::from(levels[(face * extent + y) * extent + x]) > NO_LAKE_THRESHOLD_METERS {
                            blocks[(face * side + y / block) * side + x / block] = true;
                        }
                    }
                }
            }
            blocks
        });
        Self { extent, levels, land_biomes, blocks }
    }

    /// Quads along a face edge, as the shader's settings carry it.
    pub fn face_quads(&self) -> u32 {
        (self.extent - 3) as u32
    }

    /// (face, fractional texel coordinate including the gutter) of a direction.
    fn texel(&self, direction: DVec3) -> (usize, f64, f64) {
        let (face, u, v) = direction_to_face_uv(direction);
        let quads = (self.extent - 3) as f64;
        (face.index() as usize, (u + 1.0) * 0.5 * quads + 1.0, (v + 1.0) * 0.5 * quads + 1.0)
    }

    /// Bilinear water level at a direction, in stored (unscaled) metres; at or
    /// below `NO_LAKE_THRESHOLD_METERS` where there is no lake.
    pub fn level_meters(&self, direction: DVec3) -> f64 {
        let (face, x, y) = self.texel(direction);
        let max = (self.extent - 2) as f64;
        let (x, y) = (x.clamp(0.0, max), y.clamp(0.0, max));
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |dx: usize, dy: usize| {
            let index = (face * self.extent + y0 as usize + dy) * self.extent + x0 as usize + dx;
            f64::from(self.levels[index])
        };
        let bottom = at(0, 0) + (at(1, 0) - at(0, 0)) * fx;
        let top = at(0, 1) + (at(1, 1) - at(0, 1)) * fx;
        bottom + (top - bottom) * fy
    }

    /// The level in the terrain's presented (scaled) metres, if a lake owns
    /// this point -- the height its water is drawn at.
    pub fn presented_level_meters(&self, direction: DVec3) -> Option<f64> {
        let level = self.level_meters(direction);
        (level > NO_LAKE_THRESHOLD_METERS).then(|| {
            if level > 0.0 {
                crate::planet::scaled_outmap_macro_height_meters(level, 0.0)
            } else {
                level
            }
        })
    }

    /// Whether any lake level reaches the face rectangle `[u0,u1]x[v0,v1]`
    /// (a terrain chunk's footprint), counting the texels its bilinear
    /// samples read. Conservative: a block holding any level counts.
    pub fn touches(&self, face: CubeFace, uv_min: [f64; 2], uv_max: [f64; 2]) -> bool {
        let quads = (self.extent - 3) as f64;
        let to_texel = |c: f64| (c + 1.0) * 0.5 * quads + 1.0;
        let lo = |c: f64| ((to_texel(c).floor() as i64 - 1).max(0) as usize).min(self.extent - 1);
        let hi = |c: f64| ((to_texel(c).ceil() as i64 + 1).max(0) as usize).min(self.extent - 1);
        let (tx0, tx1, ty0, ty1) = (lo(uv_min[0]), hi(uv_max[0]), lo(uv_min[1]), hi(uv_max[1]));
        let face = face.index() as usize;
        // The finest level that keeps the test to at most 8x8 blocks.
        let level = (0..BLOCKS.len())
            .find(|&l| (tx1 - tx0).max(ty1 - ty0) / BLOCKS[l] < 8)
            .unwrap_or(BLOCKS.len() - 1);
        let block = BLOCKS[level];
        let side = self.extent.div_ceil(block);
        let blocks = &self.blocks[level];
        (ty0 / block..=ty1 / block)
            .any(|y| (tx0 / block..=tx1 / block).any(|x| blocks[(face * side + y) * side + x]))
    }

    /// The land biome painted where the bake's biome map says `Lake`.
    pub fn land_biome(&self, direction: DVec3) -> u8 {
        let (face, x, y) = self.texel(direction);
        let index = (face * self.extent + y.round() as usize) * self.extent + x.round() as usize;
        self.land_biomes[index]
    }

    pub fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> LakeTextures {
        LakeTextures::new(device, queue, Some(self))
    }
}

/// The two face arrays bound at shared bindings 23 (levels, R32Float) and
/// 24 (land biomes, R8Uint); 1x1 placeholders when there are no lakes.
pub struct LakeTextures {
    pub levels: wgpu::TextureView,
    pub land_biomes: wgpu::TextureView,
}

impl LakeTextures {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, field: Option<&LakeField>) -> Self {
        let extent = field.map_or(1, |f| f.extent as u32);
        let make = |label: &str, format: wgpu::TextureFormat, texel_bytes: u32, data: &[u8]| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: extent, height: extent, depth_or_array_layers: 6 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let row = extent * texel_bytes;
            let aligned = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let layer = (row * extent) as usize;
            for face in 0..6_u32 {
                let source = &data[face as usize * layer..(face as usize + 1) * layer];
                let mut padded = vec![0_u8; (aligned * extent) as usize];
                for y in 0..extent as usize {
                    padded[y * aligned as usize..y * aligned as usize + row as usize]
                        .copy_from_slice(&source[y * row as usize..(y + 1) * row as usize]);
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: face },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &padded,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(aligned),
                        rows_per_image: Some(extent),
                    },
                    wgpu::Extent3d { width: extent, height: extent, depth_or_array_layers: 1 },
                );
            }
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let (levels, biomes): (Vec<u8>, Vec<u8>) = match field {
            Some(f) => (bytemuck::cast_slice(&f.levels).to_vec(), f.land_biomes.clone()),
            None => (bytemuck::cast_slice(&[NO_LAKE_LEVEL; 6]).to_vec(), vec![0; 6]),
        };
        Self {
            levels: make("lake level faces", wgpu::TextureFormat::R32Float, 4, &levels),
            land_biomes: make("lake land biome faces", wgpu::TextureFormat::R8Uint, 1, &biomes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(level: impl Fn(usize, usize, usize) -> f32) -> LakeField {
        let extent = 11;
        let mut levels = Vec::new();
        for face in 0..6 {
            for y in 0..extent {
                for x in 0..extent {
                    levels.push(level(face, x, y));
                }
            }
        }
        LakeField::new(extent, levels, vec![5; 6 * extent * extent])
    }

    /// Texel centres sit at the face's uv lattice (gutter included), and the
    /// blend between them is bilinear.
    #[test]
    fn level_is_bilinear_over_the_face_lattice() {
        let f = field(|face, x, y| if face == 0 { (x * 10 + y) as f32 } else { NO_LAKE_LEVEL });
        // +X face, u = v = 0 is the face centre: texel (5, 5) with 8 quads + gutter.
        assert!((f.level_meters(DVec3::X) - 55.0).abs() < 1e-9);
        let quads = 8.0;
        let u = 2.0 * 0.25 / quads; // a quarter texel along +u
        let direction = planet_coretypes::face_uv_to_direction(CubeFace::PositiveX, u, 0.0);
        assert!((f.level_meters(direction) - 57.5).abs() < 1e-6);
        assert_eq!(f.face_quads(), 8);
        assert_eq!(f.land_biome(DVec3::X), 5);
    }

    /// A chunk is flagged exactly when its bilinear reads can reach a level.
    #[test]
    fn chunks_touch_lakes_only_where_a_level_is_stored() {
        // One level, at +X texel (8, 8): u = v = 0.75.
        let f = field(|face, x, y| if face == 0 && x == 8 && y == 8 { 10.0 } else { NO_LAKE_LEVEL });
        assert!(f.touches(CubeFace::PositiveX, [0.5, 0.5], [1.0, 1.0]));
        assert!(f.touches(CubeFace::PositiveX, [-1.0, -1.0], [1.0, 1.0]));
        assert!(!f.touches(CubeFace::NegativeX, [-1.0, -1.0], [1.0, 1.0]));
        let none = field(|_, _, _| NO_LAKE_LEVEL);
        assert!(!none.touches(CubeFace::PositiveX, [-1.0, -1.0], [1.0, 1.0]));
    }
}

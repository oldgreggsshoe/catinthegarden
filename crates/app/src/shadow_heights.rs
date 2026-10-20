//! The coarse planet-wide height faces that raster terrain samples for cast
//! shadows: the globally dense outmap level stitched into six guttered
//! `R32Float` cube-face layers. Heights only; nothing else reads these.

use std::{error::Error, fmt};

use planet_coretypes::{CubeFace, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey};

use crate::{
    outmap::{Outmap, OutmapError},
    terrain::TerrainSource,
};

const FIELD_LEVEL: u8 = 4;
const FACE_COUNT: u32 = 6;

pub struct ShadowHeights {
    texture: wgpu::Texture,
    face_quads: u32,
}

impl ShadowHeights {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain_source: TerrainSource,
    ) -> Result<Self, ShadowHeightsError> {
        let source = FieldSource::new(terrain_source)?;
        crate::startup_profile::mark("  shadow heights: open source");
        let face_quads = source.face_quads();
        let extent = face_quads + TILE_GUTTER * 2;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow height faces"),
            size: wgpu::Extent3d {
                width: extent,
                height: extent,
                depth_or_array_layers: FACE_COUNT,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // The six faces are stitched on threads of their own; only the
        // texture writes stay on this one.
        let timer = std::time::Instant::now();
        let built: Vec<Result<(CubeFace, Vec<f32>), ShadowHeightsError>> =
            std::thread::scope(|scope| {
                let source = &source;
                CubeFace::ALL
                    .map(|face| scope.spawn(move || Ok((face, source.build_face(face)?))))
                    .into_iter()
                    .map(|handle| handle.join().expect("face build thread panicked"))
                    .collect()
            });
        crate::startup_profile::note(
            "  shadow heights: stitch, six faces in parallel",
            timer.elapsed().as_secs_f64() * 1e3,
        );
        let timer = std::time::Instant::now();
        for built_face in built {
            let (face, heights_meters) = built_face?;
            upload_face_layer(
                queue,
                &texture,
                face.index() as u32,
                extent,
                bytemuck::cast_slice(&heights_meters),
                size_of::<f32>() as u32,
            );
        }
        crate::startup_profile::note(
            "  shadow heights: queue.write_texture x6",
            timer.elapsed().as_secs_f64() * 1e3,
        );

        Ok(Self {
            texture,
            face_quads,
        })
    }

    /// The height faces as a six-layer array view, and the quads per face edge.
    pub fn faces(&self) -> (wgpu::TextureView, u32) {
        (
            self.texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            self.face_quads,
        )
    }
}

fn upload_face_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    extent: u32,
    bytes: &[u8],
    bytes_per_texel: u32,
) {
    let padded = padded_texture_rows(bytes, extent, extent, bytes_per_texel);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned_texture_row_bytes(extent * bytes_per_texel)),
            rows_per_image: Some(extent),
        },
        wgpu::Extent3d {
            width: extent,
            height: extent,
            depth_or_array_layers: 1,
        },
    );
}

fn aligned_texture_row_bytes(row_bytes: u32) -> u32 {
    row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

fn padded_texture_rows(bytes: &[u8], width: u32, height: u32, bytes_per_texel: u32) -> Vec<u8> {
    let row_bytes = width * bytes_per_texel;
    assert_eq!(bytes.len(), (row_bytes * height) as usize);
    let aligned_row_bytes = aligned_texture_row_bytes(row_bytes);
    if aligned_row_bytes == row_bytes {
        return bytes.to_vec();
    }
    let mut padded = vec![0; (aligned_row_bytes * height) as usize];
    for row in 0..height as usize {
        let source_start = row * row_bytes as usize;
        let target_start = row * aligned_row_bytes as usize;
        padded[target_start..target_start + row_bytes as usize]
            .copy_from_slice(&bytes[source_start..source_start + row_bytes as usize]);
    }
    padded
}

enum FieldSource {
    Placeholder,
    Outmap { outmap: Outmap, level: u8 },
}

impl FieldSource {
    fn new(source: TerrainSource) -> Result<Self, ShadowHeightsError> {
        match source {
            TerrainSource::Placeholder => Ok(Self::Placeholder),
            TerrainSource::Outmap(path) => {
                let outmap = Outmap::open(path)?;
                let level = outmap.manifest().dense_level.min(FIELD_LEVEL);
                Ok(Self::Outmap { outmap, level })
            }
        }
    }

    fn face_quads(&self) -> u32 {
        TILE_LOGICAL_SIZE.saturating_sub(1) * (1_u32 << self.level())
    }

    fn level(&self) -> u8 {
        match self {
            Self::Placeholder => 0,
            Self::Outmap { level, .. } => *level,
        }
    }

    fn build_face(&self, face: CubeFace) -> Result<Vec<f32>, ShadowHeightsError> {
        match self {
            Self::Placeholder => {
                let extent = self.face_quads() + TILE_GUTTER * 2;
                Ok(vec![0.0; (extent * extent) as usize])
            }
            Self::Outmap { outmap, level } => build_outmap_face(outmap, *level, face),
        }
    }
}

fn build_outmap_face(
    outmap: &Outmap,
    level: u8,
    face: CubeFace,
) -> Result<Vec<f32>, ShadowHeightsError> {
    let side = 1_u32 << level;
    let mut tiles = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        for x in 0..side {
            let requested_key = TileKey { face, level, x, y };
            let tile = outmap.load_tile(requested_key)?;
            if tile.source_key != requested_key {
                return Err(ShadowHeightsError::IncompleteDenseLevel {
                    requested: requested_key,
                    source: tile.source_key,
                });
            }
            tiles.push(tile);
        }
    }

    let face_quads = (TILE_LOGICAL_SIZE - 1) * side;
    let extent = face_quads + TILE_GUTTER * 2;
    let axis_sources: Vec<_> = (0..extent)
        .map(|coordinate| face_sample_source(coordinate, side))
        .collect();
    let mut heights_meters = Vec::with_capacity((extent * extent) as usize);
    for &(tile_y, sample_y) in &axis_sources {
        for &(tile_x, sample_x) in &axis_sources {
            let tile = &tiles[(tile_y * side + tile_x) as usize];
            let source_index = (sample_y * TILE_STORED_SIZE + sample_x) as usize;
            heights_meters.push(tile.heights_meters[source_index]);
        }
    }
    Ok(heights_meters)
}

fn face_sample_source(coordinate: u32, side: u32) -> (u32, u32) {
    let face_quads = (TILE_LOGICAL_SIZE - 1) * side;
    if coordinate == 0 {
        return (0, 0);
    }
    if coordinate == face_quads + 1 {
        return (side - 1, TILE_STORED_SIZE - 1);
    }
    let logical_coordinate = coordinate - 1;
    let tile = (logical_coordinate / (TILE_LOGICAL_SIZE - 1)).min(side - 1);
    let sample = logical_coordinate - tile * (TILE_LOGICAL_SIZE - 1) + TILE_GUTTER;
    (tile, sample)
}

#[derive(Debug)]
pub enum ShadowHeightsError {
    Outmap(OutmapError),
    IncompleteDenseLevel { requested: TileKey, source: TileKey },
}

impl fmt::Display for ShadowHeightsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outmap(source) => {
                write!(formatter, "could not build shadow height faces: {source}")
            }
            Self::IncompleteDenseLevel { requested, source } => write!(
                formatter,
                "shadow height faces require dense tile {requested:?}, but it resolved to {source:?}"
            ),
        }
    }
}

impl Error for ShadowHeightsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Outmap(source) => Some(source),
            Self::IncompleteDenseLevel { .. } => None,
        }
    }
}

impl From<OutmapError> for ShadowHeightsError {
    fn from(source: OutmapError) -> Self {
        Self::Outmap(source)
    }
}

#[cfg(test)]
mod tests {
    use super::{FIELD_LEVEL, face_sample_source};
    use planet_coretypes::{TILE_LOGICAL_SIZE, TILE_STORED_SIZE};

    #[test]
    fn stitched_face_coordinates_keep_one_gutter_and_shared_tile_edges() {
        let side = 1_u32 << FIELD_LEVEL;
        let face_quads = (TILE_LOGICAL_SIZE - 1) * side;
        assert_eq!(face_sample_source(0, side), (0, 0));
        assert_eq!(face_sample_source(1, side), (0, 1));
        assert_eq!(face_sample_source(TILE_LOGICAL_SIZE - 1, side), (0, 128));
        assert_eq!(face_sample_source(TILE_LOGICAL_SIZE, side), (1, 1));
        assert_eq!(
            face_sample_source(face_quads + 1, side),
            (side - 1, TILE_STORED_SIZE - 1)
        );
    }
}

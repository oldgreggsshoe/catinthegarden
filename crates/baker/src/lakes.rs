//! Lakes derived from an existing outmap's dense heights, without changing them.
//!
//! The biome channel's `Lake` tag cannot be drawn as water: it covers about a
//! fifth of the planet and many tagged regions rise far above their own rim
//! (later relief passes moved the ground under an earlier flood). This pass
//! floods the dense cube faces from the ocean, keeps basins that hold water
//! below a bounded depth, and writes each face's water level beside the tiles.
//! The renderer draws water wherever that level is above the drawn ground, so
//! shorelines follow the detailed terrain rather than 3 km cells.

use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    fs,
    path::Path,
};

use glam::DVec3;
use planet_coretypes::{
    BiomeId, CubeFace, OutmapManifest, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey,
    direction_to_face_uv, face_uv_to_direction,
};

use crate::BakeResult;

use planet_coretypes::lakes::{
    DIRECTORY, LAND_BIOME_CHANNEL, LEVEL_CHANNEL, NO_LAKE_LEVEL, SCHEMA_VERSION, face_file_name,
    fnv1a,
};
/// Deepest water a basin may hold, in stored (unscaled) metres. Highland basins
/// at this grid scale would otherwise fill to distant passes kilometres up.
pub const MAX_LAKE_DEPTH_METERS: f32 = 40.0;
/// Shallowest basin worth drawing, and the fewest flooded cells (~3 km each).
pub const MIN_LAKE_DEPTH_METERS: f32 = 2.0;
pub const MIN_LAKE_CELLS: usize = 4;
/// Dry rim cells this many cells out keep the lake's level, so the blended
/// level stays flat across the shore and the ground decides the shoreline.
const RIM_RINGS: usize = 2;

#[derive(Debug, Default)]
pub struct LakeReport {
    pub lakes: usize,
    pub water_cells: usize,
    pub land_cells: usize,
    pub old_lake_cells: usize,
    pub water_in_old_lake_cells: usize,
}

/// Six stitched faces of the dense level, logical samples only.
struct Faces {
    side: usize,
    heights: Vec<f32>,
    biomes: Vec<u8>,
}

impl Faces {
    fn index(&self, face: usize, x: usize, y: usize) -> usize {
        (face * self.side + y) * self.side + x
    }

    fn direction(&self, face: usize, x: f64, y: f64) -> DVec3 {
        let span = (self.side - 1) as f64;
        face_uv_to_direction(
            CubeFace::from_index(face as u8).expect("face index"),
            -1.0 + 2.0 * x / span,
            -1.0 + 2.0 * y / span,
        )
    }

    fn nearest(&self, direction: DVec3) -> usize {
        let (face, u, v) = direction_to_face_uv(direction);
        let span = (self.side - 1) as f64;
        let x = (((u + 1.0) * 0.5 * span).round() as usize).min(self.side - 1);
        let y = (((v + 1.0) * 0.5 * span).round() as usize).min(self.side - 1);
        self.index(face.index() as usize, x, y)
    }

    /// The eight neighbours, reaching across cube edges by direction.
    fn neighbours(&self, index: usize, out: &mut Vec<usize>) {
        out.clear();
        let face = index / (self.side * self.side);
        let x = (index % (self.side * self.side)) % self.side;
        let y = (index % (self.side * self.side)) / self.side;
        for dy in -1_i64..=1 {
            for dx in -1_i64..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                let n = if (0..self.side as i64).contains(&nx) && (0..self.side as i64).contains(&ny)
                {
                    self.index(face, nx as usize, ny as usize)
                } else {
                    self.nearest(self.direction(face, nx as f64, ny as f64))
                };
                if n != index {
                    out.push(n);
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Flood {
    level: f32,
    index: usize,
}
impl Eq for Flood {}
impl Ord for Flood {
    fn cmp(&self, other: &Self) -> Ordering {
        other.level.total_cmp(&self.level).then(other.index.cmp(&self.index))
    }
}
impl PartialOrd for Flood {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Per-cell lake level (`NO_LAKE_LEVEL` where none) from heights and biomes.
/// Ocean cells seed a priority flood; each connected flooded basin keeps
/// water to `min(spill, floor + MAX_LAKE_DEPTH_METERS)`.
fn lake_levels(faces: &Faces, report: &mut LakeReport) -> Vec<f32> {
    let count = faces.heights.len();
    let mut filled = faces.heights.clone();
    let mut visited = vec![false; count];
    let mut queue = BinaryHeap::new();
    for index in 0..count {
        if faces.biomes[index] == BiomeId::Ocean as u8 {
            visited[index] = true;
            queue.push(Flood { level: faces.heights[index], index });
        }
    }
    let mut neighbours = Vec::with_capacity(8);
    while let Some(cell) = queue.pop() {
        faces.neighbours(cell.index, &mut neighbours);
        for &n in &neighbours {
            if visited[n] {
                continue;
            }
            visited[n] = true;
            filled[n] = faces.heights[n].max(cell.level);
            queue.push(Flood { level: filled[n], index: n });
        }
    }
    // Basins: connected cells the flood raised. A basin with no ocean at all
    // (every cell unvisited) has no spill and is left dry.
    let wet = |i: usize| visited[i] && filled[i] - faces.heights[i] > 0.5;
    let mut basin = vec![usize::MAX; count];
    let mut levels = vec![NO_LAKE_LEVEL; count];
    let mut stack = Vec::new();
    let mut members = Vec::new();
    for start in 0..count {
        if basin[start] != usize::MAX || !wet(start) {
            continue;
        }
        basin[start] = start;
        stack.push(start);
        members.clear();
        let (mut floor, mut spill) = (f32::INFINITY, f32::NEG_INFINITY);
        while let Some(i) = stack.pop() {
            members.push(i);
            floor = floor.min(faces.heights[i]);
            spill = spill.max(filled[i]);
            faces.neighbours(i, &mut neighbours);
            for &n in &neighbours {
                if basin[n] == usize::MAX && wet(n) {
                    basin[n] = start;
                    stack.push(n);
                }
            }
        }
        let level = spill.min(floor + MAX_LAKE_DEPTH_METERS);
        if level - floor < MIN_LAKE_DEPTH_METERS {
            continue;
        }
        let water = members.iter().filter(|&&i| faces.heights[i] < level).count();
        if water < MIN_LAKE_CELLS {
            continue;
        }
        report.lakes += 1;
        for &i in &members {
            if faces.heights[i] < level {
                levels[i] = level;
                report.water_cells += 1;
            }
        }
    }
    // Carry each level out over its dry rim (ground at or above it), so the
    // blended field stays flat to the shore. Cells below a level they do not
    // belong to stay without one.
    for _ in 0..RIM_RINGS {
        let mut next = levels.clone();
        for index in 0..count {
            if levels[index] != NO_LAKE_LEVEL || faces.biomes[index] == BiomeId::Ocean as u8 {
                continue;
            }
            faces.neighbours(index, &mut neighbours);
            let level = neighbours
                .iter()
                .map(|&n| levels[n])
                .fold(NO_LAKE_LEVEL, f32::max);
            if level != NO_LAKE_LEVEL && faces.heights[index] >= level {
                next[index] = level;
            }
        }
        levels = next;
    }
    levels
}

/// For cells the bake tagged `Lake`, the nearest other land biome, so the
/// renderer can paint them as the land they are; other cells keep their own.
fn land_biomes(faces: &Faces) -> Vec<u8> {
    let lake = BiomeId::Lake as u8;
    let mut biomes = faces.biomes.clone();
    let mut frontier: Vec<usize> = (0..biomes.len()).filter(|&i| biomes[i] != lake).collect();
    let mut neighbours = Vec::with_capacity(8);
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for &i in &frontier {
            faces.neighbours(i, &mut neighbours);
            for &n in &neighbours {
                if biomes[n] == lake {
                    // Ocean next to a tagged lake would paint it as sea floor;
                    // grassland is the neutral stand-in there.
                    biomes[n] = if biomes[i] == BiomeId::Ocean as u8 {
                        BiomeId::TemperateGrassland as u8
                    } else {
                        biomes[i]
                    };
                    next.push(n);
                }
            }
        }
        frontier = next;
    }
    biomes
}

fn load_faces(output: &Path, manifest: &OutmapManifest) -> BakeResult<Faces> {
    let level = manifest.dense_level;
    let tiles_per_side = 1_usize << level;
    let quads = (TILE_LOGICAL_SIZE as usize - 1) * tiles_per_side;
    let side = quads + 1;
    let mut heights = vec![0.0_f32; 6 * side * side];
    let mut biomes = vec![0_u8; 6 * side * side];
    for face in CubeFace::ALL {
        for ty in 0..tiles_per_side {
            for tx in 0..tiles_per_side {
                let key = TileKey { face, level, x: tx as u32, y: ty as u32 };
                let directory = output.join(key.relative_path());
                let height = fs::read(directory.join("height.r32f"))?;
                let biome = fs::read(directory.join("biome.r8"))?;
                let stored = TILE_STORED_SIZE as usize;
                if height.len() != stored * stored * 4 || biome.len() != stored * stored {
                    return Err(format!("bad dense tile payload for {key:?}").into());
                }
                for ly in 0..TILE_LOGICAL_SIZE as usize {
                    for lx in 0..TILE_LOGICAL_SIZE as usize {
                        let s = (ly + TILE_GUTTER as usize) * stored + lx + TILE_GUTTER as usize;
                        let x = tx * (TILE_LOGICAL_SIZE as usize - 1) + lx;
                        let y = ty * (TILE_LOGICAL_SIZE as usize - 1) + ly;
                        let d = (face.index() as usize * side + y) * side + x;
                        heights[d] = f32::from_le_bytes(height[s * 4..s * 4 + 4].try_into()?);
                        biomes[d] = biome[s];
                    }
                }
            }
        }
    }
    Ok(Faces { side, heights, biomes })
}

/// One face of a per-cell field with a one-sample gutter taken from the
/// neighbouring faces, as the renderer's face arrays are laid out.
fn guttered_face<T: Copy>(faces: &Faces, field: &[T], face: usize) -> Vec<T> {
    let extent = faces.side + 2;
    let mut out = Vec::with_capacity(extent * extent);
    for gy in 0..extent {
        for gx in 0..extent {
            let (x, y) = (gx as i64 - 1, gy as i64 - 1);
            let index = if (0..faces.side as i64).contains(&x) && (0..faces.side as i64).contains(&y) {
                faces.index(face, x as usize, y as usize)
            } else {
                faces.nearest(faces.direction(face, x as f64, y as f64))
            };
            out.push(field[index]);
        }
    }
    out
}

/// Derive the lakes of the outmap at `output` and write `output/lakes/`.
pub fn derive_lakes(output: &Path) -> BakeResult<LakeReport> {
    let manifest_bytes = fs::read(output.join("manifest.json"))?;
    let manifest: OutmapManifest = serde_json::from_slice(&manifest_bytes)?;
    let faces = load_faces(output, &manifest)?;
    let mut report = LakeReport::default();
    let levels = lake_levels(&faces, &mut report);
    let biomes = land_biomes(&faces);
    report.land_cells = faces.heights.len()
        - faces.biomes.iter().filter(|&&b| b == BiomeId::Ocean as u8).count();
    report.old_lake_cells = faces.biomes.iter().filter(|&&b| b == BiomeId::Lake as u8).count();
    report.water_in_old_lake_cells = (0..levels.len())
        .filter(|&i| levels[i] > faces.heights[i] && faces.biomes[i] == BiomeId::Lake as u8)
        .count();

    let directory = output.join(DIRECTORY);
    fs::create_dir_all(&directory)?;
    for face in CubeFace::ALL {
        let level = guttered_face(&faces, &levels, face.index() as usize);
        let bytes: Vec<u8> = level.iter().flat_map(|v| v.to_le_bytes()).collect();
        fs::write(directory.join(face_file_name(face, LEVEL_CHANNEL)), bytes)?;
        let biome = guttered_face(&faces, &biomes, face.index() as usize);
        fs::write(directory.join(face_file_name(face, LAND_BIOME_CHANNEL)), biome)?;
    }
    let sidecar = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "level": manifest.dense_level,
        "face_samples": faces.side,
        "gutter": 1,
        "no_lake_level": NO_LAKE_LEVEL,
        "max_lake_depth_meters": MAX_LAKE_DEPTH_METERS,
        "min_lake_depth_meters": MIN_LAKE_DEPTH_METERS,
        "min_lake_cells": MIN_LAKE_CELLS,
        "source_manifest_fnv1a": fnv1a(&manifest_bytes),
        "lakes": report.lakes,
        "water_cells": report.water_cells,
        "land_cells": report.land_cells,
    });
    fs::write(directory.join("manifest.json"), serde_json::to_vec_pretty(&sidecar)?)?;
    Ok(report)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn faces_with(side: usize, height: impl Fn(usize, usize, usize) -> f32, biome: impl Fn(usize, usize, usize) -> u8) -> Faces {
        let mut heights = vec![0.0; 6 * side * side];
        let mut biomes = vec![0; 6 * side * side];
        for f in 0..6 {
            for y in 0..side {
                for x in 0..side {
                    heights[(f * side + y) * side + x] = height(f, x, y);
                    biomes[(f * side + y) * side + x] = biome(f, x, y);
                }
            }
        }
        Faces { side, heights, biomes }
    }

    /// A walled basin on land fills to its rim; a deep one stops at the cap;
    /// the rim keeps the level, the slope beyond does not.
    #[test]
    fn basins_fill_to_their_rim_up_to_the_depth_cap() {
        let side = 33;
        let land = BiomeId::TemperateGrassland as u8;
        // Face 0 is land: a 9x9 basin floor at 100 inside a wall at `wall`,
        // the ground outside falling away to 50. Other faces are ocean.
        let make = |wall: f32| {
            faces_with(
                side,
                move |f, x, y| {
                    if f != 0 {
                        return -100.0;
                    }
                    let d = (x as i64 - 16).abs().max((y as i64 - 16).abs());
                    match d {
                        0..=4 => 100.0,
                        5 => wall,
                        _ => 50.0,
                    }
                },
                move |f, _, _| if f == 0 { land } else { BiomeId::Ocean as u8 },
            )
        };
        let faces = make(120.0);
        let mut report = LakeReport::default();
        let levels = lake_levels(&faces, &mut report);
        assert_eq!(report.lakes, 1);
        assert_eq!(levels[faces.index(0, 16, 16)], 120.0);
        assert_eq!(levels[faces.index(0, 21, 16)], 120.0, "the rim keeps the level");
        assert_eq!(levels[faces.index(0, 23, 16)], NO_LAKE_LEVEL, "the slope beyond does not");
        let faces = make(1000.0);
        let levels = lake_levels(&faces, &mut LakeReport::default());
        assert_eq!(levels[faces.index(0, 16, 16)], 100.0 + MAX_LAKE_DEPTH_METERS);
        // A wall of 101 holds a one-metre pond: too shallow to draw.
        let levels = lake_levels(&make(101.0), &mut LakeReport::default());
        assert!(levels.iter().all(|&l| l == NO_LAKE_LEVEL));
    }

    #[test]
    fn old_lake_tags_take_the_nearest_land_biome() {
        let faces = faces_with(
            9,
            |_, _, _| 10.0,
            |f, x, _| match (f, x) {
                (0, 0..=3) => BiomeId::Desert as u8,
                (0, _) => BiomeId::Lake as u8,
                _ => BiomeId::Ocean as u8,
            },
        );
        let biomes = land_biomes(&faces);
        assert_eq!(biomes[faces.index(0, 5, 4)], BiomeId::Desert as u8);
        assert!(biomes.iter().all(|&b| b != BiomeId::Lake as u8));
    }
}

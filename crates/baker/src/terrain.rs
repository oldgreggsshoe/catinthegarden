use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
};

use catinthegarden_coretypes::{BiomeId, direction_to_face_uv};
use noise::{NoiseFn, Perlin};
use rayon::prelude::*;

use crate::{BakeResult, config::BakeConfig, etopo::load_etopo, grid::SphericalGrid};

pub const MIN_HEIGHT_METERS: f64 = -5_000.0;
pub const MAX_HEIGHT_METERS: f64 = 9_000.0;
const FLOW_REFRESH_INTERVAL: usize = 32;
const THERMAL_INTERVAL: usize = 8;
const EROSION_PARALLEL_TILE_CELLS: usize = 4_096;
/// The optional presentation bake intentionally exaggerates positive relief
/// while leaving the planet radius, coastline, sea level, and bathymetry
/// unchanged. The runtime's existing terrain scale then applies on top.
const GAME_TERRAIN_LAND_SCALE: f64 = 1.6;
const GAME_TERRAIN_RIDGE_AMPLITUDE_METERS: f64 = 2_400.0;
const GAME_TERRAIN_RIDGE_BASE_FREQUENCY: f64 = 150.0;
const GAME_TERRAIN_RIDGE_DETAIL_FREQUENCY: f64 = 520.0;
// The zoomed profile samples this compact observed window repeatedly. Periodic
// domain-warped coordinates return to the same source coordinate at every
// repeat boundary, while cross-coupled harmonics prevent every repeat from
// becoming an obvious mirrored copy.
const ZOOMED_TERRAIN_CENTER_LATITUDE_DEGREES: f64 = 35.0;
const ZOOMED_TERRAIN_CENTER_LONGITUDE_DEGREES: f64 = 77.0;
const ZOOMED_TERRAIN_SOURCE_HALF_LATITUDE_DEGREES: f64 = 18.0;
const ZOOMED_TERRAIN_SOURCE_HALF_LONGITUDE_DEGREES: f64 = 24.0;
const ZOOMED_TERRAIN_LONGITUDE_REPEATS: f64 = 4.0;
const ZOOMED_TERRAIN_LATITUDE_REPEATS: f64 = 4.0;
const ZOOMED_TERRAIN_LAND_SCALE: f64 = 2.6;
const ZOOMED_TERRAIN_RIDGE_AMPLITUDE_METERS: f64 = 7_000.0;
// The atlas stores regional averages. Erosion models unresolved local relief
// instead of treating an entire coarse atlas cell as one planar slope.
const MAX_EROSION_CELL_METERS: f64 = 8_000.0;

#[derive(Clone, Debug)]
pub struct Terrain {
    pub grid: SphericalGrid,
    pub height_meters: Vec<f64>,
    pub flow_to: Vec<Option<usize>>,
    pub flow_accumulation: Vec<f64>,
    pub river: Vec<bool>,
    pub lake: Vec<bool>,
    pub glacial_valley: Vec<bool>,
    pub moisture: Vec<u8>,
    pub biome: Vec<BiomeId>,
}

/// Area-weighted result of the deterministic mountain visibility survey.
/// A land position passes when any of the eight azimuths reaches at least
/// thirty degrees above its local tangent at one of the 2-10km profile ranges.
#[derive(Clone, Copy, Debug, Default)]
pub struct MountainVisibilityReport {
    pub land_positions: usize,
    pub passing_positions: usize,
    pub qualifying_rays: usize,
    pub land_area_weight: f64,
    pub passing_area_weight: f64,
}

impl MountainVisibilityReport {
    pub fn coverage_percent(self) -> f64 {
        if self.land_area_weight > 0.0 {
            self.passing_area_weight / self.land_area_weight * 100.0
        } else {
            0.0
        }
    }
}

impl Terrain {
    /// Preserves the original infallible API for authored/test configurations.
    /// File-backed sources should normally use `try_generate` so I/O errors can
    /// be reported rather than converted into a panic.
    pub fn generate(config: &BakeConfig) -> Self {
        Self::try_generate(config).expect("terrain generation failed")
    }

    pub fn try_generate(config: &BakeConfig) -> BakeResult<Self> {
        let grid = SphericalGrid::new(config.width, config.height);
        let imported = config.etopo.is_some() && !config.procedural_terrain;
        let mut height_meters = if config.procedural_terrain {
            generate_procedural_game_shape(&grid, config.seed)
        } else if let Some(path) = &config.etopo {
            load_etopo(path, config.width, config.height)?
                .into_par_iter()
                .map(|height| height.clamp(MIN_HEIGHT_METERS, MAX_HEIGHT_METERS))
                .collect()
        } else {
            generate_base_shape(&grid, config.seed)
        };
        if config.zoomed_terrain && !config.procedural_terrain {
            height_meters = zoom_source_window(&grid, &height_meters);
        }
        if (config.game_terrain || config.zoomed_terrain) && !config.procedural_terrain {
            apply_game_terrain_relief(
                &grid,
                &mut height_meters,
                config.seed,
                config.zoomed_terrain,
            );
        }
        let len = grid.len();
        let mut terrain = Self {
            grid,
            height_meters,
            flow_to: vec![None; len],
            flow_accumulation: vec![1.0; len],
            river: vec![false; len],
            lake: vec![false; len],
            glacial_valley: vec![false; len],
            moisture: vec![0; len],
            biome: vec![BiomeId::Ocean; len],
        };
        if imported {
            // ETOPO is observed, naturally eroded terrain. Reapplying the
            // stylised hydraulic/thermal and valley carving stages rounds off
            // real ranges and moves their heights by kilometres. Retain its
            // surface while still deriving the downstream hydrology fields.
            // ETOPO has no lake-bed channel: priority-flooding positive cells
            // would turn every shallow basin into a raised, square lake in the
            // renderer. Keep only disconnected below-sea components as lakes.
            terrain.recompute_flow();
            terrain.mark_rivers();
            terrain.mark_inland_negative_lakes();
        } else {
            terrain.erode(config.erosion_iterations);
            terrain.recompute_flow();
            terrain.carve_rivers();
            terrain.fill_lakes();
            terrain.carve_glacial_valleys();
        }
        terrain.compute_moisture();
        terrain.classify_biomes(imported);
        Ok(terrain)
    }

    #[cfg(test)]
    fn from_heights(width: usize, height: usize, height_meters: Vec<f64>) -> Self {
        let grid = SphericalGrid::new(width, height);
        assert_eq!(grid.len(), height_meters.len());
        let len = grid.len();
        Self {
            grid,
            height_meters,
            flow_to: vec![None; len],
            flow_accumulation: vec![1.0; len],
            river: vec![false; len],
            lake: vec![false; len],
            glacial_valley: vec![false; len],
            moisture: vec![0; len],
            biome: vec![BiomeId::Ocean; len],
        }
    }

    fn erode(&mut self, iterations: usize) {
        for iteration in 0..iterations {
            if iteration % FLOW_REFRESH_INTERVAL == 0 {
                self.recompute_flow();
            }
            let progress = iteration as f64 / iterations.max(1) as f64;
            let step = 1.0 - progress * 0.95;
            let heights = &self.height_meters;
            let flow_to = &self.flow_to;
            let accumulation = &self.flow_accumulation;
            let grid = &self.grid;
            let mut erosion = vec![0.0; heights.len()];
            erosion
                .par_chunks_mut(EROSION_PARALLEL_TILE_CELLS)
                .enumerate()
                .for_each(|(tile_index, tile)| {
                    let tile_start = tile_index * EROSION_PARALLEL_TILE_CELLS;
                    for (local_index, amount) in tile.iter_mut().enumerate() {
                        let index = tile_start + local_index;
                        let Some(downstream) = flow_to[index] else {
                            continue;
                        };
                        let drop = (heights[index] - heights[downstream]).max(0.0);
                        let slope = drop
                            / grid
                                .distance_meters(index, downstream)
                                .min(MAX_EROSION_CELL_METERS);
                        let stream_power = accumulation[index].powf(0.5) * slope;
                        *amount = (stream_power * 15.0 * step)
                            .min(2.0 * step)
                            .min((heights[index] - MIN_HEIGHT_METERS).max(0.0));
                    }
                });
            self.height_meters
                .par_iter_mut()
                .zip(erosion)
                .for_each(|(height, amount)| *height -= amount);

            if iteration % THERMAL_INTERVAL == 0 {
                self.thermal_step(step);
            }
        }
        self.height_meters
            .par_iter_mut()
            .for_each(|height| *height = height.clamp(MIN_HEIGHT_METERS, MAX_HEIGHT_METERS));
    }

    fn thermal_step(&mut self, step: f64) {
        let heights = &self.height_meters;
        let grid = &self.grid;
        let outgoing: Vec<(Option<usize>, f64)> = (0..heights.len())
            .into_par_iter()
            .map(|index| {
                let lowest = (0..8)
                    .filter_map(|neighbor| grid.neighbor(index, neighbor))
                    .min_by(|&a, &b| heights[a].total_cmp(&heights[b]));
                let Some(lowest) = lowest else {
                    return (None, 0.0);
                };
                let drop = heights[index] - heights[lowest];
                if drop <= 0.0 {
                    return (None, 0.0);
                }
                let talus_degrees: f64 = if heights[index] > 3_500.0 { 45.0 } else { 35.0 };
                let stable_drop = talus_degrees.to_radians().tan()
                    * grid
                        .distance_meters(index, lowest)
                        .min(MAX_EROSION_CELL_METERS);
                let excess = (drop - stable_drop).max(0.0);
                (Some(lowest), (excess * 0.05 * step).min(10.0 * step))
            })
            .collect();
        let old = self.height_meters.clone();
        self.height_meters
            .par_iter_mut()
            .enumerate()
            .for_each(|(index, height)| {
                let incoming: f64 = (0..8)
                    .filter_map(|neighbor| grid.neighbor(index, neighbor))
                    .filter(|&source| outgoing[source].0 == Some(index))
                    .map(|source| outgoing[source].1)
                    .sum();
                *height = old[index] - outgoing[index].1 + incoming;
            });
    }

    fn recompute_flow(&mut self) {
        self.flow_to = compute_flow_directions(&self.grid, &self.height_meters);
        self.flow_accumulation = accumulate_flow(&self.height_meters, &self.flow_to);
    }

    fn carve_rivers(&mut self) {
        self.mark_rivers();
        let threshold = (self.grid.len() as f64 / 1_024.0).max(8.0);
        let original = self.height_meters.clone();
        for center in 0..self.grid.len() {
            if !self.river[center] {
                continue;
            }
            let ratio = (self.flow_accumulation[center] / threshold).max(1.0);
            let depth = (12.0 + ratio.ln_1p() * 18.0).min(140.0);
            let radius = ratio.log2().floor().clamp(0.0, 2.0) as isize;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let Some(index) = self.grid.offset_index(center, dx, dy) else {
                        continue;
                    };
                    let distance = ((dx * dx + dy * dy) as f64).sqrt();
                    if distance > radius as f64 + 0.25 {
                        continue;
                    }
                    let weight = 1.0 - distance / (radius as f64 + 1.0);
                    let target = original[index] - depth * weight;
                    self.height_meters[index] = self.height_meters[index].min(target);
                    self.river[index] = true;
                }
            }
        }
    }

    fn mark_rivers(&mut self) {
        let threshold = (self.grid.len() as f64 / 1_024.0).max(8.0);
        self.river = self
            .flow_accumulation
            .iter()
            .zip(&self.height_meters)
            .map(|(&flow, &height)| flow >= threshold && height > 0.0)
            .collect();
    }

    fn fill_lakes(&mut self) {
        let mut filled = self.height_meters.clone();
        let mut visited = vec![false; self.grid.len()];
        let mut queue = BinaryHeap::new();
        for (index, &height) in self.height_meters.iter().enumerate() {
            if height <= 0.0 {
                visited[index] = true;
                queue.push(FloodCell { index, height });
            }
        }
        if queue.is_empty() {
            let index = self
                .height_meters
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(b.1))
                .map(|(index, _)| index)
                .expect("terrain is non-empty");
            visited[index] = true;
            queue.push(FloodCell {
                index,
                height: self.height_meters[index],
            });
        }
        while let Some(cell) = queue.pop() {
            for neighbor in (0..8).filter_map(|slot| self.grid.neighbor(cell.index, slot)) {
                if visited[neighbor] {
                    continue;
                }
                visited[neighbor] = true;
                let spill_height = self.height_meters[neighbor].max(cell.height);
                filled[neighbor] = spill_height;
                if self.height_meters[neighbor] > 0.0
                    && spill_height - self.height_meters[neighbor] > 0.5
                {
                    self.lake[neighbor] = true;
                }
                queue.push(FloodCell {
                    index: neighbor,
                    height: spill_height,
                });
            }
        }
    }

    /// ETOPO contains a few below-sea inland basins. The priority flood above
    /// intentionally seeds every negative sample as open ocean, so those
    /// basins otherwise receive ocean ownership and become square water holes
    /// when the sparse raster is rendered. On the sphere the largest connected
    /// negative component is the real ocean; every other negative component is
    /// a major landlocked lake. Tiny components are source noise rather than
    /// useful water at the renderer's global macro resolution.
    fn mark_inland_negative_lakes(&mut self) {
        const CARDINAL_NEIGHBORS: [usize; 4] = [1, 3, 4, 6];
        let mut visited = vec![false; self.grid.len()];
        let mut largest_start = None;
        let mut largest_size = 0usize;
        // ETOPO has no lake mask and its sub-grid negative noise produces
        // implausible square ponds after categorical L5 sampling. Keep only
        // major inland bodies; the small-grid test floor preserves coverage
        // of the hydrology rule without making production speckle.
        let minimum_lake_cells = (self.grid.len() * 512 / (4_096 * 2_048)).max(8);
        for start in 0..self.grid.len() {
            if visited[start] || self.height_meters[start] > 0.0 {
                continue;
            }
            let mut queue = VecDeque::from([start]);
            visited[start] = true;
            let mut size = 0usize;
            while let Some(index) = queue.pop_front() {
                size += 1;
                for neighbor in CARDINAL_NEIGHBORS
                    .into_iter()
                    .filter_map(|slot| self.grid.neighbor(index, slot))
                {
                    if !visited[neighbor] && self.height_meters[neighbor] <= 0.0 {
                        visited[neighbor] = true;
                        queue.push_back(neighbor);
                    }
                }
            }
            if size > largest_size {
                largest_size = size;
                largest_start = Some(start);
            }
        }
        let Some(ocean_start) = largest_start else {
            return;
        };
        visited.fill(false);
        for start in 0..self.grid.len() {
            if visited[start] || self.height_meters[start] > 0.0 {
                continue;
            }
            let is_ocean = start == ocean_start;
            let mut queue = VecDeque::from([start]);
            visited[start] = true;
            let mut component = Vec::new();
            while let Some(index) = queue.pop_front() {
                component.push(index);
                for neighbor in CARDINAL_NEIGHBORS
                    .into_iter()
                    .filter_map(|slot| self.grid.neighbor(index, slot))
                {
                    if !visited[neighbor] && self.height_meters[neighbor] <= 0.0 {
                        visited[neighbor] = true;
                        queue.push_back(neighbor);
                    }
                }
            }
            if !is_ocean && component.len() >= minimum_lake_cells {
                for index in component {
                    self.lake[index] = true;
                }
            }
        }
    }

    fn carve_glacial_valleys(&mut self) {
        let before = self.height_meters.clone();
        for (index, &before_height) in before.iter().enumerate() {
            if !self.river[index] {
                continue;
            }
            let latitude = self.grid.latitude(index);
            let snowline = snowline_meters(latitude);
            if latitude.abs() < 50.0_f64.to_radians() && before_height < snowline * 0.75 {
                continue;
            }
            let Some(downstream) = self.flow_to[index] else {
                continue;
            };
            let (x, y) = self.grid.coordinates(index);
            let (down_x, down_y) = self.grid.coordinates(downstream);
            let mut dx = down_x as isize - x as isize;
            if dx.abs() > 1 {
                dx = -dx.signum();
            }
            let dy = (down_y as isize - y as isize).clamp(-1, 1);
            let (perpendicular_x, perpendicular_y) = (-dy, dx);
            if perpendicular_x == 0 && perpendicular_y == 0 {
                continue;
            }
            let width = 3_isize;
            let center_height = before_height - 35.0;
            for offset in -width..=width {
                let Some(cross_index) = self.grid.offset_index(
                    index,
                    perpendicular_x * offset,
                    perpendicular_y * offset,
                ) else {
                    continue;
                };
                let normalized = offset as f64 / width as f64;
                let target = center_height + 95.0 * normalized * normalized;
                self.height_meters[cross_index] = self.height_meters[cross_index].min(target);
                self.glacial_valley[cross_index] = true;
            }
        }
    }

    fn compute_moisture(&mut self) {
        let mut distance = vec![u16::MAX; self.grid.len()];
        let mut queue = VecDeque::new();
        for (index, cell_distance) in distance.iter_mut().enumerate() {
            if self.height_meters[index] <= 0.0 || self.river[index] || self.lake[index] {
                *cell_distance = 0;
                queue.push_back(index);
            }
        }
        while let Some(index) = queue.pop_front() {
            let next_distance = distance[index].saturating_add(1);
            for neighbor in (0..8).filter_map(|slot| self.grid.neighbor(index, slot)) {
                if next_distance < distance[neighbor] {
                    distance[neighbor] = next_distance;
                    queue.push_back(neighbor);
                }
            }
        }
        self.moisture = distance
            .iter()
            .map(|&distance| {
                if distance == u16::MAX {
                    0
                } else {
                    (255.0 / (1.0 + f64::from(distance) / 8.0)).round() as u8
                }
            })
            .collect();
        for _ in 0..3 {
            let previous = self.moisture.clone();
            self.moisture
                .par_iter_mut()
                .enumerate()
                .for_each(|(index, moisture)| {
                    let mut sum = u32::from(previous[index]);
                    let mut count = 1_u32;
                    for neighbor in (0..8).filter_map(|slot| self.grid.neighbor(index, slot)) {
                        sum += u32::from(previous[neighbor]);
                        count += 1;
                    }
                    *moisture = (sum / count) as u8;
                });
        }
    }

    /// Chooses a deterministic, dry coastal site for sparse high-resolution
    /// refinement. The cube-face margin keeps every sparse-radius tile on one
    /// face, while the minimum elevation leaves room for baked relief without
    /// allowing the inspection point to become water.
    pub fn sparse_landing_direction(&self) -> glam::DVec3 {
        for minimum_height in [450.0, 100.0, 0.0] {
            let mut best: Option<(f64, usize)> = None;
            for index in 0..self.grid.len() {
                let biome = self.biome[index];
                let height = self.height_meters[index];
                if matches!(biome, BiomeId::Ocean | BiomeId::Lake | BiomeId::Ice)
                    || height <= minimum_height
                {
                    continue;
                }
                let direction = self.grid.direction(index);
                if direction.y.abs() > 0.88 {
                    continue;
                }
                let (_, u, v) = direction_to_face_uv(direction);
                if u.abs() > 0.8 || v.abs() > 0.8 {
                    continue;
                }

                let mut touches_water = false;
                let mut touches_ocean = false;
                let mut minimum_dry_height = height;
                let mut maximum_dry_height = height;
                for neighbor in (0..8).filter_map(|slot| self.grid.neighbor(index, slot)) {
                    let neighbor_biome = self.biome[neighbor];
                    if matches!(neighbor_biome, BiomeId::Ocean | BiomeId::Lake) {
                        touches_water = true;
                        touches_ocean |= neighbor_biome == BiomeId::Ocean;
                    } else if neighbor_biome != BiomeId::Ice {
                        let neighbor_height = self.height_meters[neighbor];
                        minimum_dry_height = minimum_dry_height.min(neighbor_height);
                        maximum_dry_height = maximum_dry_height.max(neighbor_height);
                    }
                }
                if !touches_water {
                    continue;
                }
                let relief = maximum_dry_height - minimum_dry_height;
                let biome_bonus = match biome {
                    BiomeId::TemperateForest | BiomeId::TropicalForest => 400.0,
                    BiomeId::TemperateGrassland => 300.0,
                    BiomeId::Tundra | BiomeId::Desert => 100.0,
                    BiomeId::MountainRock | BiomeId::MountainSnow => -200.0,
                    BiomeId::Ocean | BiomeId::Lake | BiomeId::Ice => unreachable!(),
                };
                let score = biome_bonus
                    + if touches_ocean { 500.0 } else { 0.0 }
                    + relief.min(2_000.0) * 0.25
                    - (height - 800.0).abs() * 0.08
                    - direction.y.abs() * 150.0;
                if !matches!(best, Some((best_score, _)) if score <= best_score) {
                    best = Some((score, index));
                }
            }
            if let Some((_, index)) = best {
                return self.grid.direction(index);
            }
        }

        self.biome
            .iter()
            .enumerate()
            .find_map(|(index, &biome)| {
                if matches!(biome, BiomeId::Ocean | BiomeId::Lake | BiomeId::Ice) {
                    return None;
                }
                let direction = self.grid.direction(index);
                let (_, u, v) = direction_to_face_uv(direction);
                (u.abs() <= 0.8 && v.abs() <= 0.8).then_some(direction)
            })
            .unwrap_or(glam::DVec3::X)
    }

    /// Measures the generated macro surface at every source-grid position.
    /// This is deliberately area-weighted because the equirectangular source
    /// has smaller cells near the poles. It is the fast generation-time gate;
    /// a 400m exhaustive scan would repeatedly interpolate the same source
    /// cells because the current source grid is much coarser than 400m.
    pub fn mountain_visibility_coverage(&self) -> MountainVisibilityReport {
        const DIRECTIONS: usize = 8;
        const MIN_ELEVATION_DEGREES: f64 = 30.0;
        const DISTANCES_METERS: [f64; 5] = [2_000.0, 4_000.0, 6_000.0, 8_000.0, 10_000.0];

        (0..self.grid.len())
            .into_par_iter()
            .filter_map(|index| {
                let base = self.grid.direction(index);
                let camera_height = self.height_meters[index];
                if camera_height <= 0.0 {
                    return None;
                }
                let east = glam::DVec3::new(-base.z, 0.0, base.x).normalize();
                let north = east.cross(base).normalize();
                let mut passing = false;
                let mut qualifying_rays = 0_usize;
                for direction_index in 0..DIRECTIONS {
                    let azimuth =
                        direction_index as f64 * std::f64::consts::TAU / DIRECTIONS as f64;
                    let tangent = east * azimuth.cos() + north * azimuth.sin();
                    let mut direction_passes = false;
                    for distance in DISTANCES_METERS {
                        let target = (base
                            + tangent
                                * (distance / catinthegarden_coretypes::PLANET_RADIUS_METERS))
                            .normalize();
                        let target_height = self.grid.sample_f64(&self.height_meters, target);
                        let elevation =
                            (target_height - camera_height).atan2(distance).to_degrees();
                        if elevation >= MIN_ELEVATION_DEGREES {
                            qualifying_rays += 1;
                            direction_passes = true;
                        }
                    }
                    passing |= direction_passes;
                }
                let area_weight = base.x.hypot(base.z);
                Some(MountainVisibilityReport {
                    land_positions: 1,
                    passing_positions: passing as usize,
                    qualifying_rays,
                    land_area_weight: area_weight,
                    passing_area_weight: if passing { area_weight } else { 0.0 },
                })
            })
            .reduce(MountainVisibilityReport::default, |left, right| {
                MountainVisibilityReport {
                    land_positions: left.land_positions + right.land_positions,
                    passing_positions: left.passing_positions + right.passing_positions,
                    qualifying_rays: left.qualifying_rays + right.qualifying_rays,
                    land_area_weight: left.land_area_weight + right.land_area_weight,
                    passing_area_weight: left.passing_area_weight + right.passing_area_weight,
                }
            })
    }

    fn classify_biomes(&mut self, imported_etopo: bool) {
        self.biome
            .par_iter_mut()
            .enumerate()
            .for_each(|(index, biome)| {
                let latitude = self.grid.latitude(index);
                let height = self.height_meters[index];
                let absolute_latitude = latitude.abs();
                let snowline = snowline_meters(latitude);
                let direction = self.grid.direction(index);
                let land_ice = if imported_etopo {
                    authored_land_ice_mask(direction, height)
                } else {
                    absolute_latitude > 66.0_f64.to_radians()
                };
                // Water ownership must win before the polar land-ice rule:
                // otherwise the old latitude test turns the Arctic Ocean into
                // a solid circular ice-coloured land cap.
                *biome = if self.lake[index] {
                    BiomeId::Lake
                } else if height <= 0.0 {
                    BiomeId::Ocean
                } else if land_ice || height > snowline {
                    BiomeId::Ice
                } else if height > (snowline - 700.0).max(2_800.0) {
                    BiomeId::MountainSnow
                } else if height > 2_400.0 {
                    BiomeId::MountainRock
                } else {
                    let latitude_temperature =
                        1.0 - absolute_latitude / std::f64::consts::FRAC_PI_2;
                    let temperature =
                        latitude_temperature - height.max(0.0) / MAX_HEIGHT_METERS * 0.55;
                    let wetness = f64::from(self.moisture[index]) / 255.0;
                    // ETOPO source columns are reversed into the renderer's
                    // geographic-east = -Z convention. Keep the authored arid
                    // regions aligned with their real-world source longitudes.
                    let longitude_degrees = biome_longitude_degrees(direction, imported_etopo);
                    let aridity = earthlike_aridity_field(latitude.to_degrees(), longitude_degrees);
                    if temperature < 0.24 {
                        BiomeId::Tundra
                    } else if temperature > 0.34 && aridity > 0.42 {
                        BiomeId::Desert
                    } else if temperature > 0.72 && wetness > 0.62 {
                        BiomeId::TropicalForest
                    } else if wetness < 0.28 {
                        BiomeId::Desert
                    } else if wetness > 0.58 {
                        BiomeId::TemperateForest
                    } else {
                        BiomeId::TemperateGrassland
                    }
                };
            });
    }
}

/// Turns the observed Earth-like macro surface into a denser game landscape.
///
/// This is deliberately a bake-time operation rather than runtime noise: all
/// consumers (CPU clearance, raster displacement, ray height fields, and LOD
/// bounds) see the same surface. Positive land is amplified, then two
/// direction-domain ridged bands add narrow mountain shoulders and peaks. The
/// relief gate is based on the incoming elevation, so oceans, lakes, and
/// coastlines are not raised or reclassified by this pass.
fn apply_game_terrain_relief(grid: &SphericalGrid, heights: &mut [f64], seed: u32, zoomed: bool) {
    let ridges = Perlin::new(seed ^ 0x4752_4944);
    let detail = Perlin::new(seed ^ 0x4D54_4E31);
    heights
        .par_iter_mut()
        .enumerate()
        .for_each(|(index, height)| {
            if *height <= 0.0 {
                return;
            }
            let source_height = *height;
            let gate_start = if zoomed { 300.0 } else { 800.0 };
            let gate_span = if zoomed { 1_800.0 } else { 2_400.0 };
            let mountain_gate =
                smoother_step(((source_height - gate_start) / gate_span).clamp(0.0, 1.0));
            let direction = grid.direction(index).to_array();
            let broad_ridge = ridged_fbm(&ridges, direction, GAME_TERRAIN_RIDGE_BASE_FREQUENCY, 3);
            let fine_ridge = ridged_fbm(&detail, direction, GAME_TERRAIN_RIDGE_DETAIL_FREQUENCY, 2);
            let ridge = (broad_ridge * 0.72 + fine_ridge * 0.28 - 0.52).max(0.0) / 0.48;
            let ridge_amplitude = if zoomed {
                ZOOMED_TERRAIN_RIDGE_AMPLITUDE_METERS
            } else {
                GAME_TERRAIN_RIDGE_AMPLITUDE_METERS
            };
            let land_scale = if zoomed {
                ZOOMED_TERRAIN_LAND_SCALE
            } else {
                GAME_TERRAIN_LAND_SCALE
            };
            let added_ridge = mountain_gate * ridge.min(1.0) * ridge_amplitude;
            *height = (source_height * land_scale + added_ridge).clamp(1.0, MAX_HEIGHT_METERS);
        });
}

/// Samples the Himalaya/India/Tibet-sized portion of the source repeatedly
/// across the planet. The sinusoidal coordinate map is periodic in both
/// dimensions and keeps the source window continuous at each repeat boundary.
fn zoom_source_window(grid: &SphericalGrid, source_heights: &[f64]) -> Vec<f64> {
    (0..grid.len())
        .into_par_iter()
        .map(|index| {
            let source_direction = zoomed_source_direction(grid.direction(index));
            grid.sample_f64(source_heights, source_direction)
        })
        .collect()
}

fn zoomed_source_direction(direction: glam::DVec3) -> glam::DVec3 {
    let centre_latitude = ZOOMED_TERRAIN_CENTER_LATITUDE_DEGREES.to_radians();
    let centre_longitude = ZOOMED_TERRAIN_CENTER_LONGITUDE_DEGREES.to_radians();
    let half_latitude = ZOOMED_TERRAIN_SOURCE_HALF_LATITUDE_DEGREES.to_radians();
    let half_longitude = ZOOMED_TERRAIN_SOURCE_HALF_LONGITUDE_DEGREES.to_radians();
    let latitude = direction.y.asin();
    let longitude = direction.z.atan2(direction.x);
    let global_latitude = latitude - centre_latitude;
    let global_longitude = longitude - centre_longitude;
    // These low-frequency global terms deliberately vary the compact source
    // orientation from repeat to repeat. They are themselves 2pi-periodic,
    // so the dateline still joins exactly while neighbouring copies differ.
    let longitude_phase = global_longitude * ZOOMED_TERRAIN_LONGITUDE_REPEATS
        + 0.48 * global_longitude.sin()
        + 0.18 * (2.0 * global_latitude).sin();
    let latitude_phase = global_latitude * ZOOMED_TERRAIN_LATITUDE_REPEATS
        + 0.37 * global_longitude.cos()
        + 0.16 * global_latitude.sin();
    let longitude_wave = (longitude_phase.sin()
        + 0.24 * (longitude_phase + 0.70 * latitude_phase.sin()).sin()
        + 0.10 * (2.0 * longitude_phase - 0.45 * latitude_phase.cos()).sin())
        / 1.34;
    let latitude_wave = (latitude_phase.sin()
        + 0.22 * (latitude_phase - 0.60 * longitude_phase.sin()).sin()
        + 0.12 * (2.0 * latitude_phase + 0.35 * longitude_phase.cos()).sin())
        / 1.34;
    let source_latitude = centre_latitude + half_latitude * latitude_wave;
    let source_longitude = centre_longitude + half_longitude * longitude_wave;
    glam::DVec3::new(
        source_latitude.cos() * source_longitude.cos(),
        source_latitude.sin(),
        source_latitude.cos() * source_longitude.sin(),
    )
}

/// Authored land-ice footprint used when ETOPO supplies observed elevations
/// but no separate ice-mask band. Positive ETOPO samples provide the observed
/// land mask; this small geographic prior captures Greenland and Antarctica,
/// while high terrain still falls through to the latitude/elevation snowline.
fn authored_land_ice_mask(direction: glam::DVec3, height_meters: f64) -> bool {
    if height_meters <= 0.0 {
        return false;
    }
    let latitude = direction.y.asin().to_degrees();
    let longitude = biome_longitude_degrees(direction, true);
    let greenland = ellipse_field(
        latitude,
        longitude,
        GeoEllipse::new(-42.0, 72.0, 9.0, 14.0, -8.0),
    ) > 0.0;
    let antarctica = latitude < -70.0;
    greenland || antarctica
}

fn biome_longitude_degrees(direction: glam::DVec3, imported_etopo: bool) -> f64 {
    let longitude_sign = if imported_etopo { -1.0 } else { 1.0 };
    longitude_sign * direction.z.atan2(direction.x).to_degrees()
}

/// Builds a complete game-sized planet from a continuous direction-domain
/// field. Unlike the authored Earth-like profile, this has no latitude or
/// longitude anchors and no repeated source window: continents, highlands,
/// basins, and mountain regions all come from 3D noise sampled on the unit
/// sphere. The result remains deterministic and exactly seam-safe at cube-face
/// boundaries, then follows the ordinary erosion/hydrology pipeline.
fn generate_procedural_game_shape(grid: &SphericalGrid, seed: u32) -> Vec<f64> {
    let continents = Perlin::new(seed ^ 0xC01A_11A5);
    let warp = Perlin::new(seed ^ 0xD04A_1A7E);
    let regions = Perlin::new(seed ^ 0xA07E_7A1B);
    let ridges = Perlin::new(seed ^ 0xB1D6_E5B1);
    let narrow_ridges = Perlin::new(seed ^ 0xC11F_7EAD);
    let coverage_ridges = Perlin::new(seed ^ 0xC0DE_A7E5);
    let detail = Perlin::new(seed ^ 0xD37A_11A4);
    (0..grid.len())
        .into_par_iter()
        .map(|index| {
            let direction = grid.direction(index).to_array();
            // Three differently permuted warp samples keep the continental
            // edges organic without introducing a seam at longitude +/-pi.
            let warp_point = [
                direction[0] * 1.15,
                direction[1] * 1.15,
                direction[2] * 1.15,
            ];
            let warped = [
                direction[0] + fbm(&warp, warp_point, 1.15, 3) * 0.22,
                direction[1]
                    + fbm(
                        &warp,
                        [warp_point[2], warp_point[0], warp_point[1]],
                        1.15,
                        3,
                    ) * 0.22,
                direction[2]
                    + fbm(
                        &warp,
                        [warp_point[1], warp_point[2], warp_point[0]],
                        1.15,
                        3,
                    ) * 0.22,
            ];
            let continental = fbm(&continents, warped, 1.35, 5);
            let coast_detail = fbm(&continents, warped, 5.2, 3) * 0.14;
            let signed_land = continental + coast_detail - 0.015;
            let broad_relief = fbm(&continents, warped, 2.25, 4);
            if signed_land <= 0.0 {
                let basin = smoother_step((-signed_land / 0.62).clamp(0.0, 1.0));
                return (-90.0 - basin * 4_300.0 + broad_relief * 220.0)
                    .clamp(MIN_HEIGHT_METERS, -1.0);
            }

            let interior = smoother_step((signed_land / 0.42).clamp(0.0, 1.0));
            let mountain_region = smoother_step(
                ((fbm(&regions, warped, 1.75, 4) + 0.12 * broad_relief + 0.08) / 0.34)
                    .clamp(0.0, 1.0),
            );
            let ridge = ridged_fbm(&ridges, warped, 3.35, 5);
            let sharp_ridge = ((ridge - 0.52) / 0.48).clamp(0.0, 1.0).powi(2);
            // Keep the mountain region broad enough to read as a range, but
            // put its strongest elevation on a much narrower spine. The
            // high-frequency folds are deliberately thresholded and cubed so
            // they create isolated, steep summits rather than another smooth
            // blanket of hills. The second fold is sampled well below the
            // continental scale: it is the local peak spacing that makes a
            // valley feel enclosed by nearby high ground instead of sitting
            // on a kilometre-wide plateau.
            let narrow_ridge = ridged_fbm(&narrow_ridges, warped, 11.0, 4);
            let narrow_peak = ((narrow_ridge - 0.68) / 0.32).clamp(0.0, 1.0).powi(3);
            let local_ridge = ridged_fbm(&narrow_ridges, warped, 360.0, 3);
            let local_peak = ((local_ridge - 0.70) / 0.30).clamp(0.0, 1.0).powi(3);
            // A separate, continuous ridge family gives the generator a
            // measurable mountain-coverage target rather than concentrating
            // all relief in one rare global massif. Its 1500x wavelength is
            // narrow enough for a 2km look-ahead to see a steep wall while
            // remaining seam-safe in the direction domain.
            let coverage_ridge = ridged_fbm(&coverage_ridges, warped, 1_500.0, 3);
            let coverage_peak = ((coverage_ridge - 0.55) / 0.20).clamp(0.0, 1.0);
            // The previous field left most positive land in a nearly planar
            // 50-850m band. Add a resolvable foothill field across land, then
            // let the thresholded mountain belts rise out of it. This keeps
            // the peaks narrow without requiring source frequencies finer
            // than the dense L4 bake can represent.
            let foothills = interior * ridged_fbm(&regions, warped, 8.0, 3).powi(2) * 900.0;
            let highland = (0.5 + broad_relief * 0.5).max(0.0);
            let fine_breakup = fbm(&detail, warped, 12.0, 3);
            let lowland =
                80.0 + interior * (360.0 + highland * 620.0) + foothills + fine_breakup * 70.0;
            let mountain_height = mountain_region
                * (220.0 + sharp_ridge * 4_500.0 + highland * 700.0)
                + mountain_region * narrow_peak * 5_000.0
                + mountain_region * local_peak * 2_200.0
                + interior * coverage_peak * 8_500.0;
            // Keep the coverage ridge below the export ceiling instead of
            // producing a population of clipped, identical 9km summits.
            let mountain_headroom = (MAX_HEIGHT_METERS - lowland - 100.0).max(0.0);
            (lowland + mountain_height.min(mountain_headroom)).clamp(1.0, MAX_HEIGHT_METERS)
        })
        .collect()
}

fn generate_base_shape(grid: &SphericalGrid, seed: u32) -> Vec<f64> {
    let base = Perlin::new(seed);
    let warp = Perlin::new(seed ^ 0x00D0_A11A);
    let mountains = Perlin::new(seed ^ 0xBEEF_9000);
    (0..grid.len())
        .into_par_iter()
        .map(|index| {
            earthlike_base_height(grid.direction(index).to_array(), &base, &warp, &mountains)
        })
        .collect()
}

#[derive(Clone, Copy)]
struct GeoEllipse {
    longitude_degrees: f64,
    latitude_degrees: f64,
    radius_x_degrees: f64,
    radius_y_degrees: f64,
    rotation_degrees: f64,
}

const EARTHLIKE_CONTINENTS: &[GeoEllipse] = &[
    // North America, including Alaska, Mexico and the eastern seaboard.
    GeoEllipse::new(-108.0, 49.0, 30.0, 23.0, -12.0),
    GeoEllipse::new(-88.0, 39.0, 24.0, 18.0, -18.0),
    GeoEllipse::new(-150.0, 62.0, 20.0, 12.0, -8.0),
    GeoEllipse::new(-101.0, 25.0, 12.0, 17.0, 20.0),
    GeoEllipse::new(-83.0, 18.0, 8.0, 13.0, 38.0),
    // South America: broad tropical north tapering into Patagonia.
    GeoEllipse::new(-61.0, -8.0, 20.0, 25.0, -10.0),
    GeoEllipse::new(-65.0, -25.0, 15.0, 24.0, -6.0),
    GeoEllipse::new(-70.0, -45.0, 8.0, 17.0, -3.0),
    // Europe, Africa and their connecting peninsulas.
    GeoEllipse::new(13.0, 50.0, 21.0, 12.0, 4.0),
    GeoEllipse::new(19.0, 61.0, 8.0, 15.0, -8.0),
    GeoEllipse::new(17.0, 8.0, 22.0, 29.0, 2.0),
    GeoEllipse::new(22.0, -22.0, 17.0, 24.0, -4.0),
    GeoEllipse::new(42.0, 8.0, 10.0, 9.0, -20.0),
    GeoEllipse::new(45.0, 23.0, 15.0, 10.0, -8.0),
    // Eurasia, India and southeast Asia.
    GeoEllipse::new(58.0, 50.0, 38.0, 18.0, 2.0),
    GeoEllipse::new(97.0, 49.0, 42.0, 20.0, -2.0),
    GeoEllipse::new(126.0, 38.0, 25.0, 18.0, -14.0),
    GeoEllipse::new(78.0, 21.0, 12.0, 15.0, 5.0),
    GeoEllipse::new(104.0, 16.0, 15.0, 14.0, -18.0),
    GeoEllipse::new(120.0, 2.0, 20.0, 7.0, -8.0),
    // Australia, New Guinea, Greenland and the major North Atlantic islands.
    GeoEllipse::new(135.0, -25.0, 20.0, 14.0, -6.0),
    GeoEllipse::new(145.0, -6.0, 12.0, 5.0, -8.0),
    GeoEllipse::new(-42.0, 72.0, 9.0, 14.0, -8.0),
    GeoEllipse::new(-18.0, 65.0, 5.0, 4.0, -15.0),
];

const EARTHLIKE_MOUNTAIN_BELTS: &[GeoEllipse] = &[
    GeoEllipse::new(-72.0, -18.0, 3.2, 33.0, -3.0), // Andes
    GeoEllipse::new(-116.0, 44.0, 5.5, 27.0, -10.0), // Rockies
    GeoEllipse::new(-151.0, 62.0, 6.0, 14.0, -18.0), // Alaska
    GeoEllipse::new(84.0, 31.0, 25.0, 4.2, 3.0),    // Himalaya
    GeoEllipse::new(66.0, 40.0, 19.0, 7.0, 8.0),    // Hindu Kush / Central Asia
    GeoEllipse::new(12.0, 46.0, 10.0, 3.0, -4.0),   // Alps
    GeoEllipse::new(-5.0, 32.0, 9.0, 3.0, 8.0),     // Atlas
    GeoEllipse::new(36.0, 1.0, 4.5, 18.0, -4.0),    // East African rift
    GeoEllipse::new(147.0, -27.0, 3.0, 17.0, -3.0), // Great Dividing Range
    GeoEllipse::new(145.0, -6.0, 10.0, 3.0, -8.0),  // New Guinea
];

const EARTHLIKE_ARID_REGIONS: &[GeoEllipse] = &[
    GeoEllipse::new(15.0, 24.0, 27.0, 10.0, 0.0),   // Sahara
    GeoEllipse::new(45.0, 24.0, 13.0, 7.0, -5.0),   // Arabia
    GeoEllipse::new(96.0, 42.0, 21.0, 8.0, 2.0),    // Central Asia / Gobi
    GeoEllipse::new(134.0, -25.0, 16.0, 10.0, 0.0), // Australian interior
    GeoEllipse::new(-112.0, 32.0, 9.0, 7.0, -8.0),  // North American southwest
    GeoEllipse::new(-70.0, -23.0, 3.5, 12.0, -2.0), // Atacama
    GeoEllipse::new(22.0, -24.0, 10.0, 8.0, 0.0),   // Kalahari
];

impl GeoEllipse {
    const fn new(
        longitude_degrees: f64,
        latitude_degrees: f64,
        radius_x_degrees: f64,
        radius_y_degrees: f64,
        rotation_degrees: f64,
    ) -> Self {
        Self {
            longitude_degrees,
            latitude_degrees,
            radius_x_degrees,
            radius_y_degrees,
            rotation_degrees,
        }
    }
}

fn earthlike_base_height(
    direction: [f64; 3],
    base: &Perlin,
    warp: &Perlin,
    mountains: &Perlin,
) -> f64 {
    let latitude_degrees = direction[1].asin().to_degrees();
    let longitude_degrees = direction[2].atan2(direction[0]).to_degrees();
    let warped = [
        direction[0]
            + warp.get([direction[0] * 0.9, direction[1] * 0.9, direction[2] * 0.9]) * 0.12,
        direction[1]
            + warp.get([direction[2] * 0.9, direction[0] * 0.9, direction[1] * 0.9]) * 0.12,
        direction[2]
            + warp.get([direction[1] * 0.9, direction[2] * 0.9, direction[0] * 0.9]) * 0.12,
    ];
    let continent = earthlike_continent_field(latitude_degrees, longitude_degrees);
    let coast_detail = fbm(base, warped, 1.7, 6) * 0.23 + fbm(base, warped, 6.5, 3) * 0.045;
    let signed_land = continent + coast_detail;
    let broad_relief = fbm(base, warped, 2.8, 5);
    if signed_land <= 0.0 {
        let ocean_depth = smoother_step((-signed_land / 0.72).clamp(0.0, 1.0));
        return (-120.0 - ocean_depth * 4_650.0 + broad_relief * 180.0)
            .clamp(MIN_HEIGHT_METERS, -1.0);
    }

    let interior = smoother_step((signed_land / 0.38).clamp(0.0, 1.0));
    let mountain_field = earthlike_mountain_field(latitude_degrees, longitude_degrees);
    let mountain_profile = smoother_step((mountain_field / 0.82).clamp(0.0, 1.0));
    let ridge = ridged_fbm(mountains, warped, 4.2, 5).powi(2);
    let lowland = 35.0 + interior * 720.0 + broad_relief * 360.0;
    let mountain_height = mountain_profile * (3_400.0 + ridge * 6_200.0);
    (lowland + mountain_height).clamp(1.0, MAX_HEIGHT_METERS)
}

fn earthlike_continent_field(latitude_degrees: f64, longitude_degrees: f64) -> f64 {
    let continents = EARTHLIKE_CONTINENTS
        .iter()
        .map(|ellipse| ellipse_field(latitude_degrees, longitude_degrees, *ellipse))
        .fold(f64::NEG_INFINITY, f64::max);
    let antarctica = (-latitude_degrees - 67.0) / 13.0;
    continents.max(antarctica)
}

fn earthlike_mountain_field(latitude_degrees: f64, longitude_degrees: f64) -> f64 {
    EARTHLIKE_MOUNTAIN_BELTS
        .iter()
        .map(|ellipse| ellipse_field(latitude_degrees, longitude_degrees, *ellipse))
        .fold(0.0, f64::max)
}

fn earthlike_aridity_field(latitude_degrees: f64, longitude_degrees: f64) -> f64 {
    EARTHLIKE_ARID_REGIONS
        .iter()
        .map(|ellipse| ellipse_field(latitude_degrees, longitude_degrees, *ellipse))
        .fold(0.0, f64::max)
}

fn ellipse_field(latitude_degrees: f64, longitude_degrees: f64, ellipse: GeoEllipse) -> f64 {
    let longitude_delta =
        (longitude_degrees - ellipse.longitude_degrees + 180.0).rem_euclid(360.0) - 180.0;
    let x = longitude_delta * ellipse.latitude_degrees.to_radians().cos();
    let y = latitude_degrees - ellipse.latitude_degrees;
    let rotation = ellipse.rotation_degrees.to_radians();
    let rotated_x = x * rotation.cos() + y * rotation.sin();
    let rotated_y = -x * rotation.sin() + y * rotation.cos();
    1.0 - ((rotated_x / ellipse.radius_x_degrees).powi(2)
        + (rotated_y / ellipse.radius_y_degrees).powi(2))
    .sqrt()
}

fn smoother_step(value: f64) -> f64 {
    value * value * (3.0 - 2.0 * value)
}

fn fbm(noise: &Perlin, point: [f64; 3], frequency: f64, octaves: u32) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut total_amplitude = 0.0;
    let mut frequency = frequency;
    for _ in 0..octaves {
        value += noise.get([
            point[0] * frequency,
            point[1] * frequency,
            point[2] * frequency,
        ]) * amplitude;
        total_amplitude += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    value / total_amplitude
}

fn ridged_fbm(noise: &Perlin, point: [f64; 3], frequency: f64, octaves: u32) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut total_amplitude = 0.0;
    let mut frequency = frequency;
    for _ in 0..octaves {
        let ridge = 1.0
            - noise
                .get([
                    point[0] * frequency,
                    point[1] * frequency,
                    point[2] * frequency,
                ])
                .abs();
        value += ridge * ridge * amplitude;
        total_amplitude += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    value / total_amplitude
}

fn compute_flow_directions(grid: &SphericalGrid, heights: &[f64]) -> Vec<Option<usize>> {
    (0..grid.len())
        .into_par_iter()
        .map(|index| {
            (0..8)
                .filter_map(|slot| grid.neighbor(index, slot))
                .filter(|&neighbor| heights[neighbor] < heights[index])
                .max_by(|&a, &b| {
                    let slope_a = (heights[index] - heights[a]) / grid.distance_meters(index, a);
                    let slope_b = (heights[index] - heights[b]) / grid.distance_meters(index, b);
                    slope_a.total_cmp(&slope_b).then_with(|| b.cmp(&a))
                })
        })
        .collect()
}

fn accumulate_flow(heights: &[f64], flow_to: &[Option<usize>]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..heights.len()).collect();
    order.sort_unstable_by(|&a, &b| heights[b].total_cmp(&heights[a]).then_with(|| a.cmp(&b)));
    let mut accumulation = vec![1.0; heights.len()];
    for index in order {
        if let Some(downstream) = flow_to[index] {
            accumulation[downstream] += accumulation[index];
        }
    }
    accumulation
}

pub fn snowline_meters(latitude: f64) -> f64 {
    5_000.0 * (1.0 - latitude.abs() / std::f64::consts::FRAC_PI_2).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug)]
struct FloodCell {
    index: usize,
    height: f64,
}

impl Eq for FloodCell {}

impl PartialEq for FloodCell {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.height.to_bits() == other.height.to_bits()
    }
}

impl Ord for FloodCell {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .height
            .total_cmp(&self.height)
            .then_with(|| other.index.cmp(&self.index))
    }
}

impl PartialOrd for FloodCell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "instrument: cargo test -p catinthegarden-baker --lib terrain::tests::mountain_visibility_throughput -- --ignored --nocapture"]
    fn mountain_visibility_throughput() {
        use std::{hint::black_box, time::Instant};

        use glam::DVec3;

        const CANDIDATE_POSITIONS: usize = 200_000;
        const DIRECTIONS: usize = 8;
        const DISTANCES_METERS: [f64; 5] = [2_000.0, 4_000.0, 6_000.0, 8_000.0, 10_000.0];
        const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;

        // Use a generated in-memory height field so this measures the
        // directional/profile work rather than filesystem tile I/O. The
        // operation is the same bilinear spherical lookup used by the bake
        // data and keeps the 8-direction, five-range proposal intact.
        let grid = SphericalGrid::new(512, 256);
        let heights = generate_procedural_game_shape(&grid, 0xEA27_2026);
        let start = Instant::now();
        let mut checksum = 0.0_f64;

        for index in 0..CANDIDATE_POSITIONS {
            let fraction = (index as f64 + 0.5) / CANDIDATE_POSITIONS as f64;
            let latitude = (1.0 - 2.0 * fraction).asin();
            let longitude = index as f64 * GOLDEN_ANGLE;
            let base = DVec3::new(
                latitude.cos() * longitude.cos(),
                latitude.sin(),
                latitude.cos() * longitude.sin(),
            );
            let east = DVec3::new(-base.z, 0.0, base.x).normalize();
            let north = east.cross(base).normalize();
            let camera_height = grid.sample_f64(&heights, base);

            for direction_index in 0..DIRECTIONS {
                let azimuth = direction_index as f64 * std::f64::consts::TAU / DIRECTIONS as f64;
                let tangent = east * azimuth.cos() + north * azimuth.sin();
                for distance in DISTANCES_METERS {
                    let target = (base
                        + tangent * (distance / catinthegarden_coretypes::PLANET_RADIUS_METERS))
                        .normalize();
                    let target_height = grid.sample_f64(&heights, target);
                    checksum += (target_height - camera_height).atan2(distance).to_degrees();
                }
            }
        }

        let elapsed = start.elapsed().as_secs_f64();
        let rays = CANDIDATE_POSITIONS * DIRECTIONS;
        let samples = rays * DISTANCES_METERS.len();
        println!(
            "mountain visibility throughput: {rays} rays / {samples} profile samples in {elapsed:.3}s = {:.0} rays/s, {:.0} samples/s (checksum {:.3})",
            rays as f64 / elapsed,
            samples as f64 / elapsed,
            black_box(checksum),
        );
    }

    #[test]
    #[ignore = "instrument: cargo test -p catinthegarden-baker --lib terrain::tests::procedural_mountain_coverage_snapshot -- --ignored --nocapture"]
    fn procedural_mountain_coverage_snapshot() {
        let config = BakeConfig {
            width: 1024,
            height: 512,
            erosion_iterations: 256,
            procedural_terrain: true,
            game_terrain: true,
            ..BakeConfig::quick(std::path::PathBuf::new())
        };
        let terrain = Terrain::generate(&config);
        let report = terrain.mountain_visibility_coverage();
        println!(
            "procedural mountain coverage: {:.2}% of area-weighted land; {}/{} positions, {} qualifying rays",
            report.coverage_percent(),
            report.passing_positions,
            report.land_positions,
            report.qualifying_rays,
        );
    }

    #[test]
    fn generated_base_is_deterministic_signed_and_bounded() {
        let grid = SphericalGrid::new(64, 32);
        let first = generate_base_shape(&grid, 1234);
        let second = generate_base_shape(&grid, 1234);
        assert_eq!(first, second);
        assert!(first.iter().all(|height| height.is_finite()));
        assert!(first.iter().any(|&height| height < 0.0));
        assert!(first.iter().any(|&height| height > 2_000.0));
        assert!(first.iter().all(|&height| height <= MAX_HEIGHT_METERS));
    }

    #[test]
    fn procedural_game_shape_is_deterministic_asymmetric_and_varied() {
        let grid = SphericalGrid::new(128, 64);
        let first = generate_procedural_game_shape(&grid, 0xEA27_2026);
        let second = generate_procedural_game_shape(&grid, 0xEA27_2026);
        assert_eq!(first, second);
        assert!(first.iter().all(|height| height.is_finite()));
        assert!(first.iter().any(|&height| height < -1_000.0));
        assert!(first.iter().any(|&height| height > 3_000.0));
        let maximum = first.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            maximum < 8_950.0,
            "procedural ridges clipped at {maximum:.1}m"
        );
        assert!(
            first
                .iter()
                .all(|&height| { (MIN_HEIGHT_METERS..=MAX_HEIGHT_METERS).contains(&height) })
        );
        let positive = first.iter().filter(|&&height| height > 0.0).count();
        assert!(positive > first.len() / 8 && positive < first.len() * 7 / 8);
        // Opposite directions must not collapse to a mirrored or antipodal
        // copy of the same macro surface.
        let asymmetry: f64 = first
            .iter()
            .zip(first.iter().rev())
            .map(|(left, right)| (left - right).abs())
            .sum::<f64>()
            / first.len() as f64;
        assert!(
            asymmetry > 50.0,
            "procedural field was too symmetric: {asymmetry:.1}m"
        );

        let summit = first
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .expect("procedural field has samples");
        let neighbour_max = (0..8)
            .filter_map(|slot| grid.neighbor(summit, slot))
            .map(|index| first[index])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            first[summit] - neighbour_max > 250.0,
            "highest summit is too broad: {:.1}m local prominence",
            first[summit] - neighbour_max
        );
    }

    #[test]
    fn game_terrain_relief_keeps_sea_level_and_adds_dense_positive_detail() {
        let grid = SphericalGrid::new(128, 64);
        let mut heights: Vec<_> = (0..grid.len())
            .map(|index| if index % 5 == 0 { -250.0 } else { 3_000.0 })
            .collect();
        let original = heights.clone();
        apply_game_terrain_relief(&grid, &mut heights, 0xEA27_2026, false);

        assert!(
            heights
                .iter()
                .zip(original)
                .all(|(&actual, expected)| expected <= 0.0 && actual == expected
                    || expected > 0.0 && actual >= expected * GAME_TERRAIN_LAND_SCALE)
        );
        assert!(heights.iter().any(|&height| height > 5_000.0));
        assert!(
            heights
                .iter()
                .filter(|&&height| height > 0.0)
                .map(|height| height.to_bits())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 8
        );
    }

    #[test]
    fn zoomed_source_window_repeats_without_nonfinite_or_sea_level_drift() {
        let grid = SphericalGrid::new(128, 64);
        let source: Vec<_> = (0..grid.len())
            .map(|index| grid.direction(index).y * 6_000.0 - 3_000.0)
            .collect();
        let zoomed = zoom_source_window(&grid, &source);
        assert_eq!(zoomed.len(), source.len());
        assert!(zoomed.iter().all(|height| height.is_finite()));
        assert!(zoomed.iter().any(|&height| height > 500.0));
        assert!(zoomed.iter().any(|&height| height < -500.0));
    }

    #[test]
    fn zoomed_source_mapping_is_periodic_but_not_mirrored() {
        let latitude = 12.0_f64.to_radians();
        let longitude = 33.0_f64.to_radians();
        let direction = glam::DVec3::new(
            latitude.cos() * longitude.cos(),
            latitude.sin(),
            latitude.cos() * longitude.sin(),
        );
        let repeat_period = std::f64::consts::TAU / ZOOMED_TERRAIN_LONGITUDE_REPEATS;
        let half_period = repeat_period * 0.5;
        let epsilon = 1.0e-7;
        let before = glam::DVec3::new(
            latitude.cos() * (longitude + repeat_period - epsilon).cos(),
            latitude.sin(),
            latitude.cos() * (longitude + repeat_period - epsilon).sin(),
        );
        let after = glam::DVec3::new(
            latitude.cos() * (longitude + repeat_period + epsilon).cos(),
            latitude.sin(),
            latitude.cos() * (longitude + repeat_period + epsilon).sin(),
        );
        let before_source = zoomed_source_direction(before);
        let after_source = zoomed_source_direction(after);
        assert!(before_source.distance(after_source) < 1.0e-6);
        let first_source = zoomed_source_direction(direction);
        let full_repeat = glam::DVec3::new(
            latitude.cos() * (longitude + repeat_period).cos(),
            latitude.sin(),
            latitude.cos() * (longitude + repeat_period).sin(),
        );
        let full_repeat_source = zoomed_source_direction(full_repeat);
        assert!(first_source.distance(full_repeat_source) > 1.0e-3);
        let half_repeat = glam::DVec3::new(
            latitude.cos() * (longitude + half_period).cos(),
            latitude.sin(),
            latitude.cos() * (longitude + half_period).sin(),
        );
        let second_source = zoomed_source_direction(half_repeat);
        assert!(first_source.distance(second_source) > 1.0e-3);
    }

    #[test]
    fn earthlike_profile_places_the_major_continents_and_oceans() {
        let seed = BakeConfig::default().seed;
        for (name, latitude, longitude) in [
            ("North America", 40.0, -100.0),
            ("South America", -15.0, -60.0),
            ("Europe", 50.0, 12.0),
            ("Africa", 5.0, 20.0),
            ("Asia", 45.0, 95.0),
            ("India", 20.0, 78.0),
            ("Australia", -25.0, 135.0),
            ("Greenland", 72.0, -42.0),
            ("Antarctica", -80.0, 0.0),
        ] {
            let height = earthlike_height_at(latitude, longitude, seed);
            assert!(height > 0.0, "{name} anchor was ocean at {height:.1}m");
        }
        for (name, latitude, longitude) in [
            ("central Pacific", 0.0, -150.0),
            ("central Atlantic", 0.0, -35.0),
            ("southern Pacific", -35.0, -120.0),
            ("southern Indian", -42.0, 75.0),
        ] {
            let height = earthlike_height_at(latitude, longitude, seed);
            assert!(height < 0.0, "{name} anchor was land at {height:.1}m");
        }
    }

    #[test]
    fn earthlike_profile_has_high_andean_rocky_and_himalayan_belts() {
        let seed = BakeConfig::default().seed;
        for (name, latitude, longitude) in [
            ("Andes", -18.0, -72.0),
            ("Rockies", 44.0, -116.0),
            ("Himalaya", 31.0, 84.0),
        ] {
            let height = earthlike_height_at(latitude, longitude, seed);
            assert!(
                height >= 3_000.0,
                "{name} anchor was only {height:.1}m high"
            );
        }
    }

    #[test]
    fn earthlike_profile_pins_major_arid_regions_without_drying_rainforests() {
        for (name, latitude, longitude) in [
            ("Sahara", 24.0, 15.0),
            ("Arabia", 24.0, 45.0),
            ("Gobi", 42.0, 96.0),
            ("Australian interior", -25.0, 134.0),
        ] {
            assert!(
                earthlike_aridity_field(latitude, longitude) > 0.8,
                "{name} lost its arid-region mask"
            );
        }
        for (name, latitude, longitude) in [
            ("Amazon", -5.0, -62.0),
            ("Congo", 0.0, 22.0),
            ("western Europe", 50.0, 0.0),
        ] {
            assert!(
                earthlike_aridity_field(latitude, longitude) <= 0.0,
                "{name} was incorrectly masked as desert"
            );
        }
    }

    fn earthlike_height_at(latitude_degrees: f64, longitude_degrees: f64, seed: u32) -> f64 {
        let latitude = latitude_degrees.to_radians();
        let longitude = longitude_degrees.to_radians();
        let direction = [
            latitude.cos() * longitude.cos(),
            latitude.sin(),
            latitude.cos() * longitude.sin(),
        ];
        earthlike_base_height(
            direction,
            &Perlin::new(seed),
            &Perlin::new(seed ^ 0x00D0_A11A),
            &Perlin::new(seed ^ 0xBEEF_9000),
        )
    }

    #[test]
    fn d8_accumulation_merges_tributaries() {
        let grid = SphericalGrid::new(16, 8);
        let mut heights = vec![100.0; grid.len()];
        let center = grid.index(8, 4);
        heights[center] = 0.0;
        heights[grid.index(7, 4)] = 10.0;
        heights[grid.index(9, 4)] = 10.0;
        let flow = compute_flow_directions(&grid, &heights);
        let accumulation = accumulate_flow(&heights, &flow);
        assert!(accumulation[center] >= 3.0);
    }

    #[test]
    fn priority_flood_marks_a_landlocked_depression() {
        let width = 16;
        let height = 8;
        let mut heights = vec![100.0; width * height];
        heights
            .iter_mut()
            .take(width)
            .for_each(|height| *height = -10.0);
        let basin = 4 * width + 8;
        heights[basin] = 10.0;
        let mut terrain = Terrain::from_heights(width, height, heights);
        terrain.fill_lakes();
        assert!(terrain.lake[basin]);
    }

    #[test]
    fn disconnected_negative_inland_component_is_a_lake() {
        let width = 32;
        let height = 16;
        let mut heights = vec![100.0; width * height];
        heights[4 * width..5 * width].fill(-10.0);
        for y in 6..9 {
            for x in 14..18 {
                heights[y * width + x] = -20.0;
            }
        }
        let mut terrain = Terrain::from_heights(width, height, heights);
        terrain.mark_inland_negative_lakes();
        terrain.classify_biomes(false);
        assert_eq!(terrain.biome[7 * width + 16], BiomeId::Lake);
        assert_eq!(terrain.biome[4 * width], BiomeId::Ocean);
    }

    #[test]
    fn tiny_inland_negative_components_are_not_rendered_as_lakes() {
        let width = 32;
        let height = 16;
        let mut heights = vec![100.0; width * height];
        heights[4 * width..5 * width].fill(-10.0);
        heights[7 * width + 7] = -20.0;
        let mut terrain = Terrain::from_heights(width, height, heights);
        terrain.mark_inland_negative_lakes();
        assert!(!terrain.lake[7 * width + 7]);
    }

    #[test]
    fn moisture_falls_with_distance_from_water() {
        let width = 32;
        let height = 16;
        let mut heights = vec![100.0; width * height];
        for y in 0..height {
            heights[y * width] = -1.0;
        }
        let mut terrain = Terrain::from_heights(width, height, heights);
        terrain.compute_moisture();
        assert!(
            terrain.moisture[terrain.grid.index(1, 8)]
                > terrain.moisture[terrain.grid.index(12, 8)]
        );
    }

    #[test]
    fn snowline_reaches_zero_at_pole() {
        assert_eq!(snowline_meters(0.0), 5_000.0);
        assert!(snowline_meters(std::f64::consts::FRAC_PI_2).abs() < f64::EPSILON);
    }

    #[test]
    fn imported_land_ice_mask_keeps_ocean_open_and_targets_real_ice_regions() {
        assert!(!authored_land_ice_mask(glam::DVec3::Y, -10.0));
        assert!(authored_land_ice_mask(
            glam::DVec3::new(0.229, 0.951, 0.207).normalize(),
            100.0
        ));
        assert!(authored_land_ice_mask(
            glam::DVec3::new(0.1, -0.98, -0.1).normalize(),
            100.0
        ));
        assert!(!authored_land_ice_mask(glam::DVec3::X, 100.0));
    }

    #[test]
    fn sparse_landing_selection_returns_dry_terrain() {
        let config = BakeConfig {
            width: 64,
            height: 32,
            erosion_iterations: 1,
            ..BakeConfig::quick(std::path::PathBuf::new())
        };
        let terrain = Terrain::generate(&config);
        let direction = terrain.sparse_landing_direction();
        let height = terrain.grid.sample_f64(&terrain.height_meters, direction);
        let biome = terrain.grid.sample_u8_nearest(
            &terrain
                .biome
                .iter()
                .map(|biome| *biome as u8)
                .collect::<Vec<_>>(),
            direction,
        );
        assert!(height > 0.0, "landing height was {height}");
        assert!(!matches!(
            BiomeId::try_from(biome),
            Ok(BiomeId::Ocean | BiomeId::Lake | BiomeId::Ice)
        ));
    }

    #[test]
    fn hydraulic_erosion_removes_material_with_a_diminishing_step() {
        let width = 16;
        let height = 8;
        let heights: Vec<f64> = (0..width * height)
            .map(|index| 2_000.0 - (index % width) as f64 * 80.0)
            .collect();
        let mut terrain = Terrain::from_heights(width, height, heights);
        let before: f64 = terrain.height_meters.iter().sum();
        terrain.erode(16);
        let after: f64 = terrain.height_meters.iter().sum();
        assert!(after < before);
    }

    #[test]
    fn thermal_erosion_moves_an_over_talus_spike_downhill() {
        let width = 16;
        let height = 8;
        let center = 4 * width + 8;
        let mut heights = vec![0.0; width * height];
        heights[center] = 9_000.0;
        let mut terrain = Terrain::from_heights(width, height, heights);
        terrain.thermal_step(1.0);
        assert!(terrain.height_meters[center] < 9_000.0);
        assert!(
            terrain
                .height_meters
                .iter()
                .enumerate()
                .any(|(index, &height)| { index != center && height > 0.0 })
        );
    }

    #[test]
    fn river_width_and_depth_grow_from_accumulation() {
        let width = 16;
        let height = 8;
        let center = 4 * width + 8;
        let heights = vec![1_000.0; width * height];
        let mut terrain = Terrain::from_heights(width, height, heights);
        terrain.flow_accumulation[center] = 256.0;
        terrain.carve_rivers();
        assert!(terrain.river[center]);
        assert!(terrain.height_meters[center] < 1_000.0);
        assert!(terrain.river[center + 1]);
    }

    #[test]
    fn glacial_river_gets_a_wide_parabolic_cross_section() {
        let width = 16;
        let height = 8;
        let center = width + 8;
        let downstream = center + width;
        let heights = vec![4_000.0; width * height];
        let mut terrain = Terrain::from_heights(width, height, heights);
        terrain.river[center] = true;
        terrain.flow_to[center] = Some(downstream);
        terrain.carve_glacial_valleys();
        assert!(terrain.glacial_valley[center]);
        assert!(terrain.glacial_valley[center + 1]);
        assert!(terrain.height_meters[center] < terrain.height_meters[center + 3]);
    }

    #[test]
    fn biome_rules_include_lakes_ocean_and_ice_override() {
        let width = 16;
        let height = 8;
        let mut terrain = Terrain::from_heights(width, height, vec![100.0; width * height]);
        terrain.moisture.fill(128);
        let polar_land = terrain.grid.index(4, 0);
        let polar_ocean = terrain.grid.index(4, 1);
        let ocean = terrain.grid.index(4, 4);
        let lake = terrain.grid.index(6, 4);
        let high = terrain.grid.index(8, 4);
        terrain.height_meters[polar_land] = 100.0;
        terrain.height_meters[polar_ocean] = -100.0;
        terrain.height_meters[ocean] = -100.0;
        terrain.lake[lake] = true;
        terrain.height_meters[high] = 6_000.0;
        terrain.classify_biomes(false);
        assert_eq!(terrain.biome[polar_land], BiomeId::Ice);
        assert_eq!(terrain.biome[polar_ocean], BiomeId::Ocean);
        assert_eq!(terrain.biome[ocean], BiomeId::Ocean);
        assert_eq!(terrain.biome[lake], BiomeId::Lake);
        assert_eq!(terrain.biome[high], BiomeId::Ice);
    }

    #[test]
    fn etopo_biome_longitude_follows_the_imported_visual_orientation() {
        assert_eq!(biome_longitude_degrees(glam::DVec3::Z, false), 90.0);
        assert_eq!(biome_longitude_degrees(glam::DVec3::Z, true), -90.0);
        assert_eq!(biome_longitude_degrees(-glam::DVec3::Z, true), 90.0);
    }
}

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
};

use catinthegarden_coretypes::BiomeId;
use noise::{NoiseFn, Perlin};
use rayon::prelude::*;

use crate::{config::BakeConfig, grid::SphericalGrid};

pub const MIN_HEIGHT_METERS: f64 = -5_000.0;
pub const MAX_HEIGHT_METERS: f64 = 9_000.0;
const FLOW_REFRESH_INTERVAL: usize = 32;
const THERMAL_INTERVAL: usize = 8;
const EROSION_PARALLEL_TILE_CELLS: usize = 4_096;
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

impl Terrain {
    pub fn generate(config: &BakeConfig) -> Self {
        let grid = SphericalGrid::new(config.width, config.height);
        let height_meters = generate_base_shape(&grid, config.seed);
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
        terrain.erode(config.erosion_iterations);
        terrain.recompute_flow();
        terrain.carve_rivers();
        terrain.fill_lakes();
        terrain.carve_glacial_valleys();
        terrain.apply_landing_patch();
        terrain.compute_moisture();
        terrain.classify_biomes();
        terrain
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
        let threshold = (self.grid.len() as f64 / 1_024.0).max(8.0);
        self.river = self
            .flow_accumulation
            .iter()
            .zip(&self.height_meters)
            .map(|(&flow, &height)| flow >= threshold && height > 0.0)
            .collect();
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

    fn apply_landing_patch(&mut self) {
        let center = glam::DVec3::X;
        // Cover at least the four working-grid samples surrounding +X so
        // bilinear tile export cannot interpolate the landing point upward.
        let inner_radius = 2.0_f64
            .to_radians()
            .max(std::f64::consts::PI / self.grid.height() as f64);
        let outer_radius = inner_radius + 4.0_f64.to_radians();
        self.height_meters
            .par_iter_mut()
            .enumerate()
            .for_each(|(index, height)| {
                let angle = self
                    .grid
                    .direction(index)
                    .dot(center)
                    .clamp(-1.0, 1.0)
                    .acos();
                if angle >= outer_radius {
                    return;
                }
                let normalized =
                    ((angle - inner_radius) / (outer_radius - inner_radius)).clamp(0.0, 1.0);
                let smooth = normalized * normalized * (3.0 - 2.0 * normalized);
                *height = -10.0 * (1.0 - smooth) + *height * smooth;
            });
    }

    fn classify_biomes(&mut self) {
        self.biome
            .par_iter_mut()
            .enumerate()
            .for_each(|(index, biome)| {
                let latitude = self.grid.latitude(index);
                let height = self.height_meters[index];
                let absolute_latitude = latitude.abs();
                let snowline = snowline_meters(latitude);
                *biome = if absolute_latitude > 66.0_f64.to_radians() || height > snowline {
                    BiomeId::Ice
                } else if height <= 0.0 {
                    BiomeId::Ocean
                } else if self.lake[index] {
                    BiomeId::Lake
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
                    if temperature < 0.24 {
                        BiomeId::Tundra
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

fn generate_base_shape(grid: &SphericalGrid, seed: u32) -> Vec<f64> {
    let base = Perlin::new(seed);
    let warp = Perlin::new(seed ^ 0x00D0_A11A);
    let mountains = Perlin::new(seed ^ 0xBEEF_9000);
    let tectonics = Perlin::new(seed ^ 0x7EC7_011C);
    (0..grid.len())
        .into_par_iter()
        .map(|index| {
            let direction = grid.direction(index).to_array();
            let warped = [
                direction[0]
                    + warp.get([
                        direction[0] * 0.65,
                        direction[1] * 0.65,
                        direction[2] * 0.65,
                    ]) * 0.18,
                direction[1]
                    + warp.get([
                        direction[2] * 0.65,
                        direction[0] * 0.65,
                        direction[1] * 0.65,
                    ]) * 0.18,
                direction[2]
                    + warp.get([
                        direction[1] * 0.65,
                        direction[2] * 0.65,
                        direction[0] * 0.65,
                    ]) * 0.18,
            ];
            let continent = fbm(&base, warped, 0.9, 6);
            let ridge = ridged_fbm(&mountains, warped, 3.4, 5).powi(3);
            let tectonic_belt = (1.0 - fbm(&tectonics, direction, 0.55, 2).abs())
                .clamp(0.0, 1.0)
                .powi(4);
            (continent * 3_800.0 - 450.0 + ridge * tectonic_belt * 8_000.0)
                .clamp(MIN_HEIGHT_METERS, MAX_HEIGHT_METERS)
        })
        .collect()
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
    fn landing_patch_keeps_nominal_ten_meter_descent_above_ground() {
        let config = BakeConfig {
            width: 64,
            height: 32,
            erosion_iterations: 1,
            ..BakeConfig::quick(std::path::PathBuf::new())
        };
        let terrain = Terrain::generate(&config);
        let height = terrain
            .grid
            .sample_f64(&terrain.height_meters, glam::DVec3::X);
        assert!(height <= 0.0, "landing height was {height}");
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
        let polar = terrain.grid.index(4, 0);
        let ocean = terrain.grid.index(4, 4);
        let lake = terrain.grid.index(6, 4);
        let high = terrain.grid.index(8, 4);
        terrain.height_meters[polar] = -100.0;
        terrain.height_meters[ocean] = -100.0;
        terrain.lake[lake] = true;
        terrain.height_meters[high] = 6_000.0;
        terrain.classify_biomes();
        assert_eq!(terrain.biome[polar], BiomeId::Ice);
        assert_eq!(terrain.biome[ocean], BiomeId::Ocean);
        assert_eq!(terrain.biome[lake], BiomeId::Lake);
        assert_eq!(terrain.biome[high], BiomeId::Ice);
    }
}

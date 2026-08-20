use glam::DVec3;

use crate::planet::{PLANET_RADIUS_METERS, cube_face_basis, cube_face_direction};

pub const WEATHER_GRID_SIDE: usize = 64;
const WEATHER_FACE_COUNT: usize = 6;
const OVERLAY_BINS: usize = 16;
const NEIGHBOUR_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeatherNeighbour {
    West = 0,
    East = 1,
    South = 2,
    North = 3,
}

#[derive(Clone, Copy, Debug)]
pub struct WeatherCell {
    pub direction: DVec3,
    pub east: DVec3,
    pub north: DVec3,
    pub area_square_meters: f64,
    neighbours: [u32; NEIGHBOUR_COUNT],
}

impl WeatherCell {
    pub fn neighbour(self, side: WeatherNeighbour) -> u32 {
        self.neighbours[side as usize]
    }
}

#[derive(Debug)]
pub struct WeatherGrid {
    cells: Vec<WeatherCell>,
}

impl WeatherGrid {
    pub fn new() -> Self {
        let mut cells = Vec::with_capacity(WEATHER_FACE_COUNT * WEATHER_GRID_SIDE.pow(2));
        for face in 0..WEATHER_FACE_COUNT {
            for j in 0..WEATHER_GRID_SIDE {
                for i in 0..WEATHER_GRID_SIDE {
                    let direction = cell_direction(face as u8, i as isize, j as isize);
                    let (east, north) = tangent_basis(direction);
                    let area_square_meters = cell_area_square_meters(face as u8, i, j);
                    cells.push(WeatherCell {
                        direction,
                        east,
                        north,
                        area_square_meters,
                        neighbours: [0; NEIGHBOUR_COUNT],
                    });
                }
            }
        }

        for face in 0..WEATHER_FACE_COUNT {
            for j in 0..WEATHER_GRID_SIDE {
                for i in 0..WEATHER_GRID_SIDE {
                    let index = cell_index(face as u8, i, j);
                    cells[index].neighbours = [
                        adjacent_cell_index(face as u8, i as isize - 1, j as isize),
                        adjacent_cell_index(face as u8, i as isize + 1, j as isize),
                        adjacent_cell_index(face as u8, i as isize, j as isize - 1),
                        adjacent_cell_index(face as u8, i as isize, j as isize + 1),
                    ];
                }
            }
        }

        Self { cells }
    }

    pub fn cells(&self) -> &[WeatherCell] {
        &self.cells
    }

    pub fn cell(&self, index: u32) -> WeatherCell {
        self.cells[index as usize]
    }

    /// Stable topology fingerprint shown in diagnostics until the simulation
    /// starts using neighbour links for transport.
    pub fn neighbour_checksum(&self) -> u64 {
        (0..self.cells.len())
            .map(|index| {
                let cell = self.cell(index as u32);
                let west = u64::from(cell.neighbour(WeatherNeighbour::West));
                let east = u64::from(cell.neighbour(WeatherNeighbour::East));
                let south = u64::from(cell.neighbour(WeatherNeighbour::South));
                let north = u64::from(cell.neighbour(WeatherNeighbour::North));
                (index as u64 + 1).wrapping_mul(
                    west ^ east.rotate_left(11) ^ south.rotate_left(23) ^ north.rotate_left(37),
                )
            })
            .fold(0, u64::wrapping_add)
    }

    pub fn total_area_square_meters(&self) -> f64 {
        self.cells.iter().map(|cell| cell.area_square_meters).sum()
    }

    pub fn area_range_square_meters(&self) -> (f64, f64) {
        self.cells
            .iter()
            .map(|cell| cell.area_square_meters)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), area| {
                (min.min(area), max.max(area))
            })
    }

    pub fn max_tangent_error(&self) -> f64 {
        self.cells
            .iter()
            .map(|cell| {
                cell.direction
                    .dot(cell.east)
                    .abs()
                    .max(cell.direction.dot(cell.north).abs())
                    .max(cell.east.dot(cell.north).abs())
                    .max((cell.direction.length_squared() - 1.0).abs())
                    .max((cell.east.length_squared() - 1.0).abs())
                    .max((cell.north.length_squared() - 1.0).abs())
            })
            .fold(0.0, f64::max)
    }

    pub fn overlay_latitude_bins(
        &self,
    ) -> [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT] {
        let mut bins = [[0.0; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT];
        for face in 0..WEATHER_FACE_COUNT {
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let u = (x as f64 + 0.5) / OVERLAY_BINS as f64 * 2.0 - 1.0;
                    let v = (y as f64 + 0.5) / OVERLAY_BINS as f64 * 2.0 - 1.0;
                    bins[face][y * OVERLAY_BINS + x] =
                        cube_face_direction(face as u8, u, v).y as f32 * 0.5 + 0.5;
                }
            }
        }
        bins
    }
}

impl Default for WeatherGrid {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WeatherDebugSnapshot {
    pub overlay_enabled: bool,
    pub total_cells: usize,
    pub total_area_square_meters: f64,
    pub minimum_cell_area_square_meters: f64,
    pub maximum_cell_area_square_meters: f64,
    pub maximum_tangent_error: f64,
    pub neighbour_checksum: u64,
    pub latitude_bins: [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
}

impl WeatherDebugSnapshot {
    pub fn paint_overlay(&self, ui: &mut egui::Ui) {
        if !self.overlay_enabled {
            return;
        }

        ui.separator();
        ui.label("Weather field overlay: latitude diagnostic (physics not started)");
        let panel_size = egui::vec2(256.0, 176.0);
        let (rect, _) = ui.allocate_exact_size(panel_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        for face in 0..WEATHER_FACE_COUNT {
            let origin = rect.min + egui::vec2((face % 3) as f32 * 86.0, (face / 3) as f32 * 86.0);
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let value = self.latitude_bins[face][y * OVERLAY_BINS + x];
                    let colour = egui::Color32::from_rgb(
                        (30.0 + 220.0 * value) as u8,
                        (50.0 + 150.0 * (1.0 - (2.0 * value - 1.0).abs())) as u8,
                        (220.0 - 170.0 * value) as u8,
                    );
                    let cell = egui::Rect::from_min_size(
                        origin + egui::vec2(x as f32 * 4.0, y as f32 * 4.0),
                        egui::vec2(4.1, 4.1),
                    );
                    painter.rect_filled(cell, 0.0, colour);
                }
            }
            painter.text(
                origin + egui::vec2(3.0, 3.0),
                egui::Align2::LEFT_TOP,
                format!("F{face}"),
                egui::FontId::monospace(9.0),
                egui::Color32::WHITE,
            );
        }
        ui.label("Wind arrows: idle until the momentum step is implemented");
    }
}

pub struct WeatherState {
    grid: WeatherGrid,
    overlay_enabled: bool,
}

impl WeatherState {
    pub fn new() -> Self {
        Self {
            grid: WeatherGrid::new(),
            overlay_enabled: false,
        }
    }

    pub fn toggle_overlay(&mut self) {
        self.overlay_enabled = !self.overlay_enabled;
    }

    pub fn debug_snapshot(&self) -> WeatherDebugSnapshot {
        let (minimum_cell_area_square_meters, maximum_cell_area_square_meters) =
            self.grid.area_range_square_meters();
        WeatherDebugSnapshot {
            overlay_enabled: self.overlay_enabled,
            total_cells: self.grid.cells().len(),
            total_area_square_meters: self.grid.total_area_square_meters(),
            minimum_cell_area_square_meters,
            maximum_cell_area_square_meters,
            maximum_tangent_error: self.grid.max_tangent_error(),
            neighbour_checksum: self.grid.neighbour_checksum(),
            latitude_bins: self.grid.overlay_latitude_bins(),
        }
    }
}

impl Default for WeatherState {
    fn default() -> Self {
        Self::new()
    }
}

fn cell_index(face: u8, i: usize, j: usize) -> usize {
    (face as usize * WEATHER_GRID_SIDE + j) * WEATHER_GRID_SIDE + i
}

fn cell_direction(face: u8, i: isize, j: isize) -> DVec3 {
    let u = (2.0 * (i as f64 + 0.5) / WEATHER_GRID_SIDE as f64) - 1.0;
    let v = (2.0 * (j as f64 + 0.5) / WEATHER_GRID_SIDE as f64) - 1.0;
    cube_face_direction(face, u, v)
}

fn adjacent_cell_index(face: u8, i: isize, j: isize) -> u32 {
    let edge_epsilon = 1.0e-7;
    let u = if i < 0 {
        -1.0 - edge_epsilon
    } else if i >= WEATHER_GRID_SIDE as isize {
        1.0 + edge_epsilon
    } else {
        2.0 * (i as f64 + 0.5) / WEATHER_GRID_SIDE as f64 - 1.0
    };
    let v = if j < 0 {
        -1.0 - edge_epsilon
    } else if j >= WEATHER_GRID_SIDE as isize {
        1.0 + edge_epsilon
    } else {
        2.0 * (j as f64 + 0.5) / WEATHER_GRID_SIDE as f64 - 1.0
    };
    let (normal, tangent_u, tangent_v) = cube_face_basis(face);
    let direction = (normal + tangent_u * u + tangent_v * v).normalize();
    let (mapped_face, mapped_i, mapped_j) = direction_to_cell(direction);
    cell_index(mapped_face, mapped_i, mapped_j) as u32
}

fn direction_to_cell(direction: DVec3) -> (u8, usize, usize) {
    let mut best_face = 0;
    let mut best_normal_dot = f64::NEG_INFINITY;
    for face in 0..WEATHER_FACE_COUNT as u8 {
        let (normal, _, _) = cube_face_basis(face);
        let normal_dot = direction.dot(normal);
        if normal_dot > best_normal_dot {
            best_face = face;
            best_normal_dot = normal_dot;
        }
    }
    let (_normal, tangent_u, tangent_v) = cube_face_basis(best_face);
    let u = direction.dot(tangent_u) / best_normal_dot;
    let v = direction.dot(tangent_v) / best_normal_dot;
    let i = (((u + 1.0) * 0.5 * WEATHER_GRID_SIDE as f64 - 0.5).round() as isize)
        .clamp(0, WEATHER_GRID_SIDE as isize - 1) as usize;
    let j = (((v + 1.0) * 0.5 * WEATHER_GRID_SIDE as f64 - 0.5).round() as isize)
        .clamp(0, WEATHER_GRID_SIDE as isize - 1) as usize;
    (best_face, i, j)
}

fn tangent_basis(direction: DVec3) -> (DVec3, DVec3) {
    let reference = if direction.y.abs() > 0.95 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let east = reference.cross(direction).normalize();
    let north = direction.cross(east).normalize();
    (east, north)
}

fn spherical_triangle_area(a: DVec3, b: DVec3, c: DVec3) -> f64 {
    let numerator = a.dot(b.cross(c)).abs();
    let denominator = 1.0 + a.dot(b) + b.dot(c) + c.dot(a);
    2.0 * numerator.atan2(denominator)
}

fn cell_area_square_meters(face: u8, i: usize, j: usize) -> f64 {
    let u0 = 2.0 * i as f64 / WEATHER_GRID_SIDE as f64 - 1.0;
    let u1 = 2.0 * (i + 1) as f64 / WEATHER_GRID_SIDE as f64 - 1.0;
    let v0 = 2.0 * j as f64 / WEATHER_GRID_SIDE as f64 - 1.0;
    let v1 = 2.0 * (j + 1) as f64 / WEATHER_GRID_SIDE as f64 - 1.0;
    let a = cube_face_direction(face, u0, v0);
    let b = cube_face_direction(face, u1, v0);
    let c = cube_face_direction(face, u1, v1);
    let d = cube_face_direction(face, u0, v1);
    (spherical_triangle_area(a, b, c) + spherical_triangle_area(a, c, d))
        * PLANET_RADIUS_METERS
        * PLANET_RADIUS_METERS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_six_faces_and_expected_cell_count() {
        let grid = WeatherGrid::new();
        assert_eq!(
            grid.cells().len(),
            6 * WEATHER_GRID_SIDE * WEATHER_GRID_SIDE
        );
    }

    #[test]
    fn cell_areas_cover_the_planet_without_a_face_seam_gap() {
        let grid = WeatherGrid::new();
        let expected = 4.0 * std::f64::consts::PI * PLANET_RADIUS_METERS.powi(2);
        assert!((grid.total_area_square_meters() - expected).abs() / expected < 1.0e-12);
    }

    #[test]
    fn tangent_bases_are_orthonormal_everywhere() {
        assert!(WeatherGrid::new().max_tangent_error() < 1.0e-12);
    }

    #[test]
    fn seam_neighbours_are_reciprocal() {
        let grid = WeatherGrid::new();
        for (index, cell) in grid.cells().iter().copied().enumerate() {
            for side in [
                WeatherNeighbour::West,
                WeatherNeighbour::East,
                WeatherNeighbour::South,
                WeatherNeighbour::North,
            ] {
                let neighbour = grid.cell(cell.neighbour(side));
                assert!(
                    neighbour.neighbours.contains(&(index as u32)),
                    "index {index}, side {side:?}, neighbour {}",
                    cell.neighbour(side)
                );
            }
        }
    }

    #[test]
    fn latitude_overlay_is_deterministic_and_bounded() {
        let first = WeatherGrid::new().overlay_latitude_bins();
        let second = WeatherGrid::new().overlay_latitude_bins();
        assert_eq!(first, second);
        assert!(
            first
                .into_iter()
                .flatten()
                .all(|value| (0.0..=1.0).contains(&value))
        );
    }
}

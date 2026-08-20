use glam::DVec3;

use crate::planet::{PLANET_RADIUS_METERS, cube_face_basis, cube_face_direction};

pub const WEATHER_GRID_SIDE: usize = 64;
pub const WEATHER_TIMESTEP_SECONDS: f64 = 600.0;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherCellState {
    pub temperature_kelvin: f32,
    pub surface_pressure_pascals: f32,
    pub specific_humidity: f32,
    pub east_wind_meters_per_second: f32,
    pub north_wind_meters_per_second: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherOverlayWind {
    pub east_meters_per_second: f32,
    pub north_meters_per_second: f32,
    pub speed_meters_per_second: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherConservationBaseline {
    pub pressure_area_integral: f64,
    pub humidity_area_integral: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherFieldDiagnostics {
    pub minimum_temperature_kelvin: f32,
    pub maximum_temperature_kelvin: f32,
    pub mean_temperature_kelvin: f32,
    pub minimum_pressure_pascals: f32,
    pub maximum_pressure_pascals: f32,
    pub mean_pressure_pascals: f32,
    pub minimum_humidity: f32,
    pub maximum_humidity: f32,
    pub mean_humidity: f32,
    pub maximum_wind_meters_per_second: f32,
    pub maximum_cfl: f64,
    pub relaxation_weight_at_1800_seconds: f64,
    pub pressure_conservation_error: f64,
    pub humidity_conservation_error: f64,
}

#[derive(Debug)]
pub struct WeatherFields {
    cells: Vec<WeatherCellState>,
    conservation_baseline: WeatherConservationBaseline,
}

impl WeatherFields {
    pub fn initial(grid: &WeatherGrid) -> Self {
        let cells = grid
            .cells()
            .iter()
            .map(|cell| initial_cell_state(cell.direction))
            .collect::<Vec<_>>();
        let conservation_baseline = conservation_baseline(grid, &cells);
        Self {
            cells,
            conservation_baseline,
        }
    }

    pub fn cells(&self) -> &[WeatherCellState] {
        &self.cells
    }

    pub fn diagnostics(&self, grid: &WeatherGrid) -> WeatherFieldDiagnostics {
        let mut min_temperature = f32::INFINITY;
        let mut max_temperature = f32::NEG_INFINITY;
        let mut min_pressure = f32::INFINITY;
        let mut max_pressure = f32::NEG_INFINITY;
        let mut min_humidity = f32::INFINITY;
        let mut max_humidity = f32::NEG_INFINITY;
        let mut mean_temperature = 0.0_f64;
        let mut mean_pressure = 0.0_f64;
        let mut mean_humidity = 0.0_f64;
        let mut maximum_wind = 0.0_f32;
        let mut maximum_cfl = 0.0_f64;
        let mut pressure_integral = 0.0_f64;
        let mut humidity_integral = 0.0_f64;

        for (cell, state) in grid.cells().iter().zip(&self.cells) {
            let area = cell.area_square_meters;
            let wind_speed = state
                .east_wind_meters_per_second
                .hypot(state.north_wind_meters_per_second);
            let cfl = WEATHER_TIMESTEP_SECONDS * f64::from(wind_speed) / area.sqrt();
            min_temperature = min_temperature.min(state.temperature_kelvin);
            max_temperature = max_temperature.max(state.temperature_kelvin);
            min_pressure = min_pressure.min(state.surface_pressure_pascals);
            max_pressure = max_pressure.max(state.surface_pressure_pascals);
            min_humidity = min_humidity.min(state.specific_humidity);
            max_humidity = max_humidity.max(state.specific_humidity);
            maximum_wind = maximum_wind.max(wind_speed);
            maximum_cfl = maximum_cfl.max(cfl);
            mean_temperature += f64::from(state.temperature_kelvin) * area;
            mean_pressure += f64::from(state.surface_pressure_pascals) * area;
            mean_humidity += f64::from(state.specific_humidity) * area;
            pressure_integral += f64::from(state.surface_pressure_pascals) * area;
            humidity_integral += f64::from(state.specific_humidity) * area;
        }

        let total_area = grid.total_area_square_meters();
        WeatherFieldDiagnostics {
            minimum_temperature_kelvin: min_temperature,
            maximum_temperature_kelvin: max_temperature,
            mean_temperature_kelvin: (mean_temperature / total_area) as f32,
            minimum_pressure_pascals: min_pressure,
            maximum_pressure_pascals: max_pressure,
            mean_pressure_pascals: (mean_pressure / total_area) as f32,
            minimum_humidity: min_humidity,
            maximum_humidity: max_humidity,
            mean_humidity: (mean_humidity / total_area) as f32,
            maximum_wind_meters_per_second: maximum_wind,
            maximum_cfl,
            relaxation_weight_at_1800_seconds: exponential_relaxation_weight(
                WEATHER_TIMESTEP_SECONDS,
                1_800.0,
            ),
            pressure_conservation_error: relative_error(
                pressure_integral,
                self.conservation_baseline.pressure_area_integral,
            ),
            humidity_conservation_error: relative_error(
                humidity_integral,
                self.conservation_baseline.humidity_area_integral,
            ),
        }
    }

    pub fn overlay_wind_bins(
        &self,
        grid: &WeatherGrid,
    ) -> [[WeatherOverlayWind; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT] {
        let mut bins = [[WeatherOverlayWind {
            east_meters_per_second: 0.0,
            north_meters_per_second: 0.0,
            speed_meters_per_second: 0.0,
        }; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT];
        let cells_per_bin = WEATHER_GRID_SIDE / OVERLAY_BINS;
        for face in 0..WEATHER_FACE_COUNT {
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let mut east = 0.0_f64;
                    let mut north = 0.0_f64;
                    let mut area = 0.0_f64;
                    for local_y in 0..cells_per_bin {
                        for local_x in 0..cells_per_bin {
                            let index = cell_index(
                                face as u8,
                                x * cells_per_bin + local_x,
                                y * cells_per_bin + local_y,
                            );
                            let cell_area = grid.cells()[index].area_square_meters;
                            let state = self.cells()[index];
                            east += f64::from(state.east_wind_meters_per_second) * cell_area;
                            north += f64::from(state.north_wind_meters_per_second) * cell_area;
                            area += cell_area;
                        }
                    }
                    let east = (east / area) as f32;
                    let north = (north / area) as f32;
                    bins[face][y * OVERLAY_BINS + x] = WeatherOverlayWind {
                        east_meters_per_second: east,
                        north_meters_per_second: north,
                        speed_meters_per_second: east.hypot(north),
                    };
                }
            }
        }
        bins
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
    pub field_diagnostics: WeatherFieldDiagnostics,
    pub latitude_bins: [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
    pub wind_bins: [[WeatherOverlayWind; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
}

impl WeatherDebugSnapshot {
    pub fn paint_overlay(&self, ui: &mut egui::Ui) {
        if !self.overlay_enabled {
            return;
        }

        ui.separator();
        ui.label("Weather field overlay: latitude / initial wind diagnostic");
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
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let wind = self.wind_bins[face][y * OVERLAY_BINS + x];
                    let center = origin + egui::vec2(x as f32 * 4.0 + 2.0, y as f32 * 4.0 + 2.0);
                    let maximum_arrow_speed = 24.0_f32;
                    let arrow =
                        egui::vec2(wind.east_meters_per_second, -wind.north_meters_per_second)
                            * (10.0 / maximum_arrow_speed);
                    if arrow.length_sq() > 0.25 {
                        let tip = center + arrow;
                        let direction = arrow.normalized();
                        let side = egui::vec2(-direction.y, direction.x);
                        let head = 2.0;
                        painter.line_segment(
                            [center - arrow * 0.35, tip],
                            egui::Stroke::new(0.8, egui::Color32::WHITE),
                        );
                        painter.line_segment(
                            [tip, tip - direction * head + side * head * 0.6],
                            egui::Stroke::new(0.8, egui::Color32::WHITE),
                        );
                        painter.line_segment(
                            [tip, tip - direction * head - side * head * 0.6],
                            egui::Stroke::new(0.8, egui::Color32::WHITE),
                        );
                    }
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
        ui.label("Arrows: initial tangent wind (transport not started)");
    }
}

pub struct WeatherState {
    grid: WeatherGrid,
    fields: WeatherFields,
    overlay_enabled: bool,
}

impl WeatherState {
    pub fn new() -> Self {
        let grid = WeatherGrid::new();
        let fields = WeatherFields::initial(&grid);
        Self {
            grid,
            fields,
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
            field_diagnostics: self.fields.diagnostics(&self.grid),
            latitude_bins: self.grid.overlay_latitude_bins(),
            wind_bins: self.fields.overlay_wind_bins(&self.grid),
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

fn initial_cell_state(direction: DVec3) -> WeatherCellState {
    let latitude = direction.y.clamp(-1.0, 1.0).asin();
    let longitude = direction.z.atan2(direction.x);
    let latitude_sine = latitude.sin().abs();
    let temperature_kelvin = 288.0 - 42.0 * latitude_sine;
    let surface_pressure_pascals =
        101_325.0 * (1.0 - 0.04 * latitude_sine + 0.01 * (longitude * 2.0).cos());
    let specific_humidity =
        (0.76 - 0.34 * latitude_sine + 0.04 * longitude.sin()).clamp(0.08, 0.82);
    let east_wind_meters_per_second =
        18.0 * (2.0 * latitude).sin() - 8.0 * (3.0 * latitude).sin() * longitude.cos();
    let north_wind_meters_per_second = 4.0 * latitude.cos() * (2.0 * longitude).sin();
    WeatherCellState {
        temperature_kelvin: temperature_kelvin as f32,
        surface_pressure_pascals: surface_pressure_pascals as f32,
        specific_humidity: specific_humidity as f32,
        east_wind_meters_per_second: east_wind_meters_per_second as f32,
        north_wind_meters_per_second: north_wind_meters_per_second as f32,
    }
}

fn conservation_baseline(
    grid: &WeatherGrid,
    cells: &[WeatherCellState],
) -> WeatherConservationBaseline {
    let (pressure_area_integral, humidity_area_integral) =
        grid.cells()
            .iter()
            .zip(cells)
            .fold((0.0, 0.0), |(pressure, humidity), (cell, state)| {
                let area = cell.area_square_meters;
                (
                    pressure + f64::from(state.surface_pressure_pascals) * area,
                    humidity + f64::from(state.specific_humidity) * area,
                )
            });
    WeatherConservationBaseline {
        pressure_area_integral,
        humidity_area_integral,
    }
}

fn relative_error(value: f64, reference: f64) -> f64 {
    if reference == 0.0 {
        (value - reference).abs()
    } else {
        (value - reference).abs() / reference.abs()
    }
}

/// Returns the bounded fraction of a reservoir transfer completed in one
/// fixed step. Unlike an explicit `dt / tau` factor, this cannot overshoot.
pub fn exponential_relaxation_weight(step_seconds: f64, time_constant_seconds: f64) -> f64 {
    if step_seconds <= 0.0 {
        0.0
    } else if time_constant_seconds <= 0.0 {
        1.0
    } else {
        1.0 - (-step_seconds / time_constant_seconds).exp()
    }
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

    #[test]
    fn initial_fields_are_deterministic_finite_and_physically_bounded() {
        let grid = WeatherGrid::new();
        let first = WeatherFields::initial(&grid);
        let second = WeatherFields::initial(&grid);
        assert_eq!(first.cells(), second.cells());
        assert!(first.cells().iter().all(|state| {
            state.temperature_kelvin.is_finite()
                && (246.0..=289.0).contains(&state.temperature_kelvin)
                && state.surface_pressure_pascals.is_finite()
                && (96_000.0..=103_000.0).contains(&state.surface_pressure_pascals)
                && (0.0..=1.0).contains(&state.specific_humidity)
                && state.east_wind_meters_per_second.is_finite()
                && state.north_wind_meters_per_second.is_finite()
        }));
    }

    #[test]
    fn diagnostics_have_bounded_cfl_and_zero_initial_conservation_error() {
        let grid = WeatherGrid::new();
        let fields = WeatherFields::initial(&grid);
        let diagnostics = fields.diagnostics(&grid);
        assert!((250.0..=290.0).contains(&diagnostics.mean_temperature_kelvin));
        assert!((99_000.0..=102_000.0).contains(&diagnostics.mean_pressure_pascals));
        assert!((0.2..=0.8).contains(&diagnostics.mean_humidity));
        assert!(diagnostics.maximum_cfl < 1.0);
        assert!(diagnostics.pressure_conservation_error < f64::EPSILON);
        assert!(diagnostics.humidity_conservation_error < f64::EPSILON);
    }

    #[test]
    fn exponential_relaxation_never_overshoots() {
        let weight = exponential_relaxation_weight(WEATHER_TIMESTEP_SECONDS, 1_800.0);
        assert!((0.0..1.0).contains(&weight));
        let start = 280.0;
        let target = 320.0;
        let next = start + (target - start) * weight;
        assert!((start..=target).contains(&next));
        assert_eq!(exponential_relaxation_weight(0.0, 1_800.0), 0.0);
        assert_eq!(exponential_relaxation_weight(600.0, 0.0), 1.0);
    }

    #[test]
    fn wind_overlay_is_deterministic_and_non_zero() {
        let grid = WeatherGrid::new();
        let fields = WeatherFields::initial(&grid);
        let first = fields.overlay_wind_bins(&grid);
        assert_eq!(first, fields.overlay_wind_bins(&grid));
        assert!(first.into_iter().flatten().any(|wind| {
            wind.speed_meters_per_second > 1.0
                && wind.east_meters_per_second.is_finite()
                && wind.north_meters_per_second.is_finite()
        }));
    }
}

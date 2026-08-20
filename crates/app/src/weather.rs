use glam::DVec3;

use crate::planet::{PLANET_RADIUS_METERS, cube_face_basis, cube_face_direction};

pub const WEATHER_GRID_SIDE: usize = 64;
pub const WEATHER_TIMESTEP_SECONDS: f64 = 600.0;
const WEATHER_REFERENCE_AIR_DENSITY_KG_PER_CUBIC_METER: f64 = 1.225;
const WEATHER_MOMENTUM_DAMPING_SECONDS: f64 = 7_200.0;
const WEATHER_MAX_WIND_SPEED_METERS_PER_SECOND: f64 = 60.0;
const WEATHER_PLANET_ANGULAR_VELOCITY_RADIANS_PER_SECOND: f64 = 7.292_115_9e-5;
const WEATHER_SOLAR_CONSTANT_WATTS_PER_SQUARE_METER: f64 = 1_361.0;
const WEATHER_STEFAN_BOLTZMANN: f64 = 5.670_374_419e-8;
const WEATHER_LAND_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN: f64 = 2.4e6;
const WEATHER_OCEAN_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN: f64 = 1.2e7;
const WEATHER_LAND_ALBEDO: f64 = 0.28;
const WEATHER_OCEAN_ALBEDO: f64 = 0.08;
const WEATHER_GREENHOUSE_FACTOR: f64 = 0.12;
const WEATHER_PRESSURE_PER_KELVIN_PASCALS: f64 = 75.0;
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

    #[allow(dead_code)] // consumed when the render-loop clock is wired in
    fn directional_neighbour_index(&self, index: usize, direction: DVec3) -> u32 {
        let cell = self.cells[index];
        let mut best_index = cell.neighbour(WeatherNeighbour::East);
        let mut best_dot = f64::NEG_INFINITY;
        for side in [
            WeatherNeighbour::West,
            WeatherNeighbour::East,
            WeatherNeighbour::South,
            WeatherNeighbour::North,
        ] {
            let neighbour_index = cell.neighbour(side);
            let neighbour = self.cell(neighbour_index);
            let tangent = (neighbour.direction
                - cell.direction * neighbour.direction.dot(cell.direction))
            .normalize();
            let alignment = tangent.dot(direction);
            if alignment > best_dot {
                best_dot = alignment;
                best_index = neighbour_index;
            }
        }
        best_index
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
    pub surface_albedo: f32,
    pub heat_capacity_joules_per_square_meter_kelvin: f32,
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

    /// Applies one fixed-step radiative energy balance. The ocean/land split
    /// is a deterministic proxy until the terrain sampler is wired into the
    /// weather field; ocean cells deliberately carry five times the thermal
    /// inertia of land so their temperature responds more slowly.
    pub fn apply_insolation_and_radiative_cooling(
        &mut self,
        grid: &WeatherGrid,
        sun_direction: DVec3,
        step_seconds: f64,
    ) {
        if step_seconds <= 0.0 {
            return;
        }
        let sun_direction = sun_direction.normalize_or_zero();
        for (cell, state) in grid.cells().iter().zip(&mut self.cells) {
            let insolation = cell.direction.dot(sun_direction).max(0.0);
            let absorbed = WEATHER_SOLAR_CONSTANT_WATTS_PER_SQUARE_METER
                * insolation
                * (1.0 - f64::from(state.surface_albedo));
            let humidity = f64::from(state.specific_humidity);
            let emissivity = (1.0 - WEATHER_GREENHOUSE_FACTOR * humidity).clamp(0.75, 1.0);
            let temperature = f64::from(state.temperature_kelvin).clamp(180.0, 340.0);
            let emitted = WEATHER_STEFAN_BOLTZMANN * temperature.powi(4) * emissivity;
            let net_flux = absorbed - emitted;
            let heat_capacity = f64::from(state.heat_capacity_joules_per_square_meter_kelvin);
            state.temperature_kelvin =
                (temperature + net_flux * step_seconds / heat_capacity).clamp(180.0, 340.0) as f32;
        }
    }

    /// Diagnoses surface pressure from the current temperature field. Pressure
    /// is intentionally not conserved: it is a diagnostic response to thermal
    /// gradients and is refreshed before the momentum pass.
    pub fn diagnose_pressure_from_temperature(&mut self, grid: &WeatherGrid) {
        let total_area = grid.total_area_square_meters();
        let mean_temperature = grid
            .cells()
            .iter()
            .zip(&self.cells)
            .map(|(cell, state)| f64::from(state.temperature_kelvin) * cell.area_square_meters)
            .sum::<f64>()
            / total_area;
        for state in &mut self.cells {
            state.surface_pressure_pascals = (101_325.0
                - WEATHER_PRESSURE_PER_KELVIN_PASCALS
                    * (f64::from(state.temperature_kelvin) - mean_temperature))
                as f32;
        }
    }

    /// Moves humidity along the local tangent wind field using conservative
    /// mass fluxes. Each source emits to one seam-safe neighbouring cell; the
    /// area-weighted humidity integral is unchanged by construction.
    #[allow(dead_code)] // consumed when the render-loop clock is wired in
    pub fn advect_humidity(&mut self, grid: &WeatherGrid, step_seconds: f64) {
        if step_seconds <= 0.0 {
            return;
        }
        let old_humidity = self
            .cells
            .iter()
            .map(|state| f64::from(state.specific_humidity))
            .collect::<Vec<_>>();
        let mut transfers = vec![(0_usize, 0.0_f64); self.cells.len()];
        let mut incoming = vec![0.0_f64; self.cells.len()];
        for (index, cell) in grid.cells().iter().enumerate() {
            let state = self.cells[index];
            let wind = cell.east * f64::from(state.east_wind_meters_per_second)
                + cell.north * f64::from(state.north_wind_meters_per_second);
            let speed = wind.length();
            if speed <= f64::EPSILON {
                continue;
            }
            let fraction = (step_seconds * speed / cell.area_square_meters.sqrt()).min(0.95);
            let target = grid.directional_neighbour_index(index, wind / speed) as usize;
            let mass = f64::from(state.specific_humidity) * cell.area_square_meters;
            let transfer = mass * fraction;
            transfers[index] = (target, transfer);
            incoming[target] += transfer;
        }

        let mut mass_delta = vec![0.0_f64; self.cells.len()];
        for (index, (target, transfer)) in transfers.into_iter().enumerate() {
            if transfer == 0.0 {
                continue;
            }
            let target_capacity =
                grid.cells()[target].area_square_meters * (1.0 - old_humidity[target]);
            let target_scale = if incoming[target] > target_capacity {
                (target_capacity / incoming[target]).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let actual_transfer = transfer * target_scale;
            mass_delta[index] -= actual_transfer;
            mass_delta[target] += actual_transfer;
        }
        for (index, (cell, state)) in grid.cells().iter().zip(&mut self.cells).enumerate() {
            let mass = old_humidity[index] * cell.area_square_meters + mass_delta[index];
            state.specific_humidity = (mass / cell.area_square_meters).clamp(0.0, 1.0) as f32;
        }
    }

    /// Applies one pressure-gradient and Coriolis momentum update in each
    /// cell's tangent frame. Pressure is a diagnosed field; damping and the
    /// explicit speed cap keep this coarse first closure stable.
    pub fn update_wind_from_pressure(&mut self, grid: &WeatherGrid, step_seconds: f64) {
        if step_seconds <= 0.0 {
            return;
        }
        let damping = (-step_seconds / WEATHER_MOMENTUM_DAMPING_SECONDS).exp();
        let spin = DVec3::Y * WEATHER_PLANET_ANGULAR_VELOCITY_RADIANS_PER_SECOND;
        let mut next_wind = vec![(0.0_f32, 0.0_f32); self.cells.len()];
        for (index, cell) in grid.cells().iter().enumerate() {
            let east_index = grid.directional_neighbour_index(index, cell.east) as usize;
            let west_index = grid.directional_neighbour_index(index, -cell.east) as usize;
            let north_index = grid.directional_neighbour_index(index, cell.north) as usize;
            let south_index = grid.directional_neighbour_index(index, -cell.north) as usize;
            let width = cell.area_square_meters.sqrt();
            let east_gradient = (f64::from(self.cells[east_index].surface_pressure_pascals)
                - f64::from(self.cells[west_index].surface_pressure_pascals))
                / (2.0 * width);
            let north_gradient = (f64::from(self.cells[north_index].surface_pressure_pascals)
                - f64::from(self.cells[south_index].surface_pressure_pascals))
                / (2.0 * width);
            let east_acceleration =
                -east_gradient / WEATHER_REFERENCE_AIR_DENSITY_KG_PER_CUBIC_METER;
            let north_acceleration =
                -north_gradient / WEATHER_REFERENCE_AIR_DENSITY_KG_PER_CUBIC_METER;
            let velocity = cell.east * f64::from(self.cells[index].east_wind_meters_per_second)
                + cell.north * f64::from(self.cells[index].north_wind_meters_per_second);
            let coriolis = -2.0 * spin.cross(velocity);
            let coriolis_tangent = coriolis - cell.direction * coriolis.dot(cell.direction);
            let east_acceleration = east_acceleration + coriolis_tangent.dot(cell.east);
            let north_acceleration = north_acceleration + coriolis_tangent.dot(cell.north);
            let mut east = (f64::from(self.cells[index].east_wind_meters_per_second)
                + east_acceleration * step_seconds)
                * damping;
            let mut north = (f64::from(self.cells[index].north_wind_meters_per_second)
                + north_acceleration * step_seconds)
                * damping;
            let speed = east.hypot(north);
            if speed > WEATHER_MAX_WIND_SPEED_METERS_PER_SECOND {
                let scale = WEATHER_MAX_WIND_SPEED_METERS_PER_SECOND / speed;
                east *= scale;
                north *= scale;
            }
            next_wind[index] = (east as f32, north as f32);
        }
        for (state, (east, north)) in self.cells.iter_mut().zip(next_wind) {
            state.east_wind_meters_per_second = east;
            state.north_wind_meters_per_second = north;
        }
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

    pub fn overlay_humidity_bins(
        &self,
        grid: &WeatherGrid,
    ) -> [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT] {
        let mut bins = [[0.0; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT];
        let cells_per_bin = WEATHER_GRID_SIDE / OVERLAY_BINS;
        for face in 0..WEATHER_FACE_COUNT {
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let mut humidity = 0.0_f64;
                    let mut area = 0.0_f64;
                    for local_y in 0..cells_per_bin {
                        for local_x in 0..cells_per_bin {
                            let index = cell_index(
                                face as u8,
                                x * cells_per_bin + local_x,
                                y * cells_per_bin + local_y,
                            );
                            let cell_area = grid.cells()[index].area_square_meters;
                            humidity +=
                                f64::from(self.cells()[index].specific_humidity) * cell_area;
                            area += cell_area;
                        }
                    }
                    bins[face][y * OVERLAY_BINS + x] = (humidity / area) as f32;
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
    pub simulation_time_seconds: f64,
    pub completed_steps: u64,
    pub field_diagnostics: WeatherFieldDiagnostics,
    pub humidity_bins: [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
    pub wind_bins: [[WeatherOverlayWind; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
}

impl WeatherDebugSnapshot {
    pub fn paint_overlay(&self, ui: &mut egui::Ui) {
        if !self.overlay_enabled {
            return;
        }

        ui.separator();
        ui.label("Weather field overlay: humidity (dry brown -> saturated blue) / wind");
        let panel_size = egui::vec2(256.0, 176.0);
        let (rect, _) = ui.allocate_exact_size(panel_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        for face in 0..WEATHER_FACE_COUNT {
            let origin = rect.min + egui::vec2((face % 3) as f32 * 86.0, (face / 3) as f32 * 86.0);
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let value = self.humidity_bins[face][y * OVERLAY_BINS + x].clamp(0.0, 1.0);
                    let colour = egui::Color32::from_rgb(
                        (185.0 - 145.0 * value) as u8,
                        (85.0 + 115.0 * value) as u8,
                        (45.0 + 190.0 * value) as u8,
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
        ui.label("Colour: 0 dry -> 1 saturated humidity | arrows: tangent wind");
    }
}

#[allow(dead_code)] // the clock is intentionally staged before render-loop integration
pub struct WeatherState {
    grid: WeatherGrid,
    fields: WeatherFields,
    overlay_enabled: bool,
    last_input_time_seconds: f64,
    accumulator_seconds: f64,
    simulation_time_seconds: f64,
    completed_steps: u64,
}

impl WeatherState {
    pub fn new() -> Self {
        let grid = WeatherGrid::new();
        let fields = WeatherFields::initial(&grid);
        Self {
            grid,
            fields,
            overlay_enabled: false,
            last_input_time_seconds: 0.0,
            accumulator_seconds: 0.0,
            simulation_time_seconds: 0.0,
            completed_steps: 0,
        }
    }

    pub fn toggle_overlay(&mut self) {
        self.overlay_enabled = !self.overlay_enabled;
    }

    /// Consumes scene time through a fixed 600-second weather clock. The
    /// caller may provide render/scenario time at any cadence; no partial
    /// transfer is applied between fixed steps.
    #[allow(dead_code)] // consumed when the render-loop clock is wired in
    pub fn advance_to(&mut self, scene_time_seconds: f64) -> u64 {
        let sun_direction = weather_sun_direction(scene_time_seconds);
        self.advance_to_with_sun(scene_time_seconds, sun_direction)
    }

    /// Advances the fixed weather clock using a sun direction expressed in the
    /// planet-local frame. The renderer uses this entry point so scenario sun
    /// waypoints and the weather terminator share the same lighting direction.
    pub fn advance_to_with_sun(&mut self, scene_time_seconds: f64, sun_direction: DVec3) -> u64 {
        if !scene_time_seconds.is_finite() || scene_time_seconds <= self.last_input_time_seconds {
            return 0;
        }
        self.accumulator_seconds += scene_time_seconds - self.last_input_time_seconds;
        self.last_input_time_seconds = scene_time_seconds;
        let mut completed = 0;
        while self.accumulator_seconds >= WEATHER_TIMESTEP_SECONDS {
            self.fields.apply_insolation_and_radiative_cooling(
                &self.grid,
                sun_direction,
                WEATHER_TIMESTEP_SECONDS,
            );
            self.fields.diagnose_pressure_from_temperature(&self.grid);
            self.fields
                .update_wind_from_pressure(&self.grid, WEATHER_TIMESTEP_SECONDS);
            self.fields
                .advect_humidity(&self.grid, WEATHER_TIMESTEP_SECONDS);
            self.accumulator_seconds -= WEATHER_TIMESTEP_SECONDS;
            self.simulation_time_seconds += WEATHER_TIMESTEP_SECONDS;
            self.completed_steps += 1;
            completed += 1;
        }
        completed
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
            simulation_time_seconds: self.simulation_time_seconds,
            completed_steps: self.completed_steps,
            field_diagnostics: self.fields.diagnostics(&self.grid),
            humidity_bins: self.fields.overlay_humidity_bins(&self.grid),
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
    let land_fraction = proxy_land_fraction(direction, latitude, longitude);
    let surface_albedo =
        WEATHER_OCEAN_ALBEDO + (WEATHER_LAND_ALBEDO - WEATHER_OCEAN_ALBEDO) * land_fraction;
    let heat_capacity = WEATHER_OCEAN_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN
        + (WEATHER_LAND_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN
            - WEATHER_OCEAN_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN)
            * land_fraction;
    WeatherCellState {
        temperature_kelvin: temperature_kelvin as f32,
        surface_pressure_pascals: surface_pressure_pascals as f32,
        specific_humidity: specific_humidity as f32,
        east_wind_meters_per_second: east_wind_meters_per_second as f32,
        north_wind_meters_per_second: north_wind_meters_per_second as f32,
        surface_albedo: surface_albedo as f32,
        heat_capacity_joules_per_square_meter_kelvin: heat_capacity as f32,
    }
}

/// Temporary climate-surface proxy. It provides broad land/ocean thermal
/// inertia without importing renderer tile state into the weather module; the
/// terrain sampler will replace this with baked ocean coverage in a later
/// coupling stage.
fn proxy_land_fraction(direction: DVec3, latitude: f64, longitude: f64) -> f64 {
    let continental_signal = 0.5
        + 0.22 * (2.0 * longitude + 0.7).sin() * latitude.cos()
        + 0.16 * (3.0 * longitude - 0.4).cos() * (2.0 * latitude).cos()
        + 0.10 * direction.x * direction.y
        + 0.07 * (5.0 * longitude + 1.2 * latitude).sin();
    ((continental_signal - 0.40) / 0.20).clamp(0.0, 1.0)
}

fn weather_sun_direction(scene_time_seconds: f64) -> DVec3 {
    let day_phase = scene_time_seconds.rem_euclid(86_400.0) / 86_400.0 * std::f64::consts::TAU;
    DVec3::new(day_phase.cos(), 0.25, day_phase.sin()).normalize()
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

    #[test]
    fn humidity_overlay_is_bounded_and_changes_after_transport() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        let before = fields.overlay_humidity_bins(&grid);
        assert!(
            before
                .iter()
                .flatten()
                .all(|value| (0.0..=1.0).contains(value))
        );
        fields.advect_humidity(&grid, WEATHER_TIMESTEP_SECONDS);
        let after = fields.overlay_humidity_bins(&grid);
        assert!(before != after);
        assert!(
            after
                .iter()
                .flatten()
                .all(|value| (0.0..=1.0).contains(value))
        );
    }

    #[test]
    fn humidity_transport_preserves_area_integral_and_bounds() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        let before = fields.diagnostics(&grid);
        fields.advect_humidity(&grid, WEATHER_TIMESTEP_SECONDS);
        let after = fields.diagnostics(&grid);
        assert!(after.maximum_cfl < 1.0);
        assert!(after.minimum_humidity >= 0.0);
        assert!(after.maximum_humidity <= 1.0);
        assert!(
            after.humidity_conservation_error < 1.0e-6,
            "humidity conservation drift: {:#?}",
            after
        );
        assert!((before.mean_humidity - after.mean_humidity).abs() < 1.0e-6);
    }

    #[test]
    fn pressure_momentum_is_deterministic_bounded_and_cfl_safe() {
        let grid = WeatherGrid::new();
        let mut first = WeatherFields::initial(&grid);
        let mut second = WeatherFields::initial(&grid);
        for _ in 0..10 {
            first.update_wind_from_pressure(&grid, WEATHER_TIMESTEP_SECONDS);
            first.advect_humidity(&grid, WEATHER_TIMESTEP_SECONDS);
            second.update_wind_from_pressure(&grid, WEATHER_TIMESTEP_SECONDS);
            second.advect_humidity(&grid, WEATHER_TIMESTEP_SECONDS);
        }
        assert_eq!(first.cells(), second.cells());
        let diagnostics = first.diagnostics(&grid);
        assert!(diagnostics.maximum_wind_meters_per_second <= 60.0);
        assert!(diagnostics.maximum_cfl < 1.0);
        assert!(
            diagnostics.humidity_conservation_error < 1.0e-5,
            "momentum humidity drift: {:#?}",
            diagnostics
        );
        assert!(first.cells().iter().all(|state| {
            state.east_wind_meters_per_second.is_finite()
                && state.north_wind_meters_per_second.is_finite()
        }));
    }

    #[test]
    fn radiative_balance_is_deterministic_day_night_and_heat_capacity_bounded() {
        let grid = WeatherGrid::new();
        let mut day = WeatherFields::initial(&grid);
        let mut night = WeatherFields::initial(&grid);
        let initial = day.diagnostics(&grid);
        let initial_cells = day.cells().to_vec();
        let sun = DVec3::X;
        day.apply_insolation_and_radiative_cooling(&grid, sun, WEATHER_TIMESTEP_SECONDS);
        night.apply_insolation_and_radiative_cooling(&grid, -sun, WEATHER_TIMESTEP_SECONDS);
        day.diagnose_pressure_from_temperature(&grid);
        night.diagnose_pressure_from_temperature(&grid);
        let day_diagnostics = day.diagnostics(&grid);
        let night_diagnostics = night.diagnostics(&grid);
        assert!(
            day_diagnostics.mean_temperature_kelvin > night_diagnostics.mean_temperature_kelvin
        );
        assert!(day.cells().iter().all(|state| {
            state.temperature_kelvin.is_finite()
                && (180.0..=340.0).contains(&state.temperature_kelvin)
                && state.surface_pressure_pascals.is_finite()
        }));
        assert!(
            day.cells()
                .iter()
                .zip(&initial_cells)
                .any(|(after, before)| after.temperature_kelvin != before.temperature_kelvin)
        );
        let minimum_heat_capacity = day
            .cells()
            .iter()
            .map(|state| f64::from(state.heat_capacity_joules_per_square_meter_kelvin))
            .fold(f64::INFINITY, f64::min);
        let maximum_heat_capacity = day
            .cells()
            .iter()
            .map(|state| f64::from(state.heat_capacity_joules_per_square_meter_kelvin))
            .fold(0.0, f64::max);
        assert!(maximum_heat_capacity / minimum_heat_capacity > 3.0);
        assert!(day_diagnostics.mean_temperature_kelvin.is_finite());
        assert!(initial.mean_temperature_kelvin.is_finite());
    }

    #[test]
    fn coriolis_is_tangent_and_reverses_across_the_equator() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        for state in &mut fields.cells {
            state.surface_pressure_pascals = 101_325.0;
            state.east_wind_meters_per_second = 20.0;
            state.north_wind_meters_per_second = 0.0;
        }
        let north_target = DVec3::new(1.0, 1.0, 0.0).normalize();
        let south_target = DVec3::new(1.0, -1.0, 0.0).normalize();
        let equator_target = DVec3::X;
        let nearest = |target: DVec3| {
            grid.cells()
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| {
                    left.direction
                        .dot(target)
                        .partial_cmp(&right.direction.dot(target))
                        .unwrap()
                })
                .map(|(index, _)| index)
                .unwrap()
        };
        let north_index = nearest(north_target);
        let south_index = nearest(south_target);
        let equator_index = nearest(equator_target);
        fields.update_wind_from_pressure(&grid, WEATHER_TIMESTEP_SECONDS);
        let north_wind = fields.cells[north_index].north_wind_meters_per_second;
        let south_wind = fields.cells[south_index].north_wind_meters_per_second;
        let equator_wind = fields.cells[equator_index].north_wind_meters_per_second;
        assert!(north_wind.abs() > 0.1);
        assert!(south_wind.abs() > 0.1);
        assert!(north_wind * south_wind < 0.0);
        assert!(equator_wind.abs() < 0.1);
        assert!(fields.cells.iter().all(|state| {
            state.east_wind_meters_per_second.is_finite()
                && state.north_wind_meters_per_second.is_finite()
                && state
                    .east_wind_meters_per_second
                    .hypot(state.north_wind_meters_per_second)
                    <= WEATHER_MAX_WIND_SPEED_METERS_PER_SECOND as f32
        }));
    }

    #[test]
    fn fixed_clock_only_runs_complete_weather_steps() {
        let mut state = WeatherState::new();
        assert_eq!(state.advance_to(WEATHER_TIMESTEP_SECONDS - 0.1), 0);
        assert_eq!(state.debug_snapshot().completed_steps, 0);
        assert_eq!(state.advance_to(WEATHER_TIMESTEP_SECONDS), 1);
        assert_eq!(state.debug_snapshot().completed_steps, 1);
        assert_eq!(state.advance_to(WEATHER_TIMESTEP_SECONDS), 0);
        assert_eq!(state.advance_to(1.0), 0);
        assert_eq!(state.advance_to(WEATHER_TIMESTEP_SECONDS * 2.0 + 0.1), 1);
        assert_eq!(state.debug_snapshot().completed_steps, 2);
    }

    #[test]
    fn transport_can_cross_a_cube_face_seam() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        let source_index = cell_index(0, 0, WEATHER_GRID_SIDE / 2);
        let source = grid.cells()[source_index];
        let target_index = grid.directional_neighbour_index(source_index, source.east) as usize;
        assert_ne!(source_index, target_index);
        for state in &mut fields.cells {
            state.east_wind_meters_per_second = 0.0;
            state.north_wind_meters_per_second = 0.0;
        }
        fields.cells[source_index].east_wind_meters_per_second = 20.0;
        fields.cells[source_index].specific_humidity = 0.8;
        fields.cells[target_index].specific_humidity = 0.2;
        fields.conservation_baseline = conservation_baseline(&grid, &fields.cells);
        let before = fields.diagnostics(&grid);
        fields.advect_humidity(&grid, WEATHER_TIMESTEP_SECONDS);
        let after = fields.diagnostics(&grid);
        assert!(
            after.humidity_conservation_error < 1.0e-6,
            "humidity conservation drift: {:#?}",
            after
        );
        assert!(fields.cells[source_index].specific_humidity < 0.8);
        assert!(fields.cells[target_index].specific_humidity > 0.2);
        assert!((before.mean_humidity - after.mean_humidity).abs() < 1.0e-6);
    }
}

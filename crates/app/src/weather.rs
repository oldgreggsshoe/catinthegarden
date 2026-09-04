use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
};

use glam::DVec3;

use crate::planet::{
    EARTH_AXIAL_TILT_RADIANS, PLANET_RADIUS_METERS, cube_face_basis, cube_face_direction,
};
use crate::terrain::TerrainClimateSample;

pub const WEATHER_GRID_SIDE: usize = 64;
pub const WEATHER_TIMESTEP_SECONDS: f64 = 600.0;
pub const WEATHER_ORBITAL_PERIOD_SECONDS: f64 = 365.2422 * 86_400.0;
/// Weather days per planet rotation. One, because a day is a rotation.
///
/// The two clocks used to be tuned apart: weather ran at 3600x real time while
/// the planet turned once per 300 real seconds, so a rotation took 12.5 weather
/// days and every cell sat in darkness for over six of them. That was set to
/// make the weather visibly move back when it was running down to a dead state;
/// with the advection leak fixed it only skewed the day.
pub const WEATHER_DAYS_PER_PLANET_ROTATION: f64 = 1.0;
/// Real seconds for one rotation, at the default rotation speed.
const PLANET_ROTATION_REAL_SECONDS: f64 = crate::planet::PLANET_ROTATION_PERIOD_SECONDS
    / crate::INTERACTIVE_PLANET_ROTATION_TIME_SCALE;
/// Derived, so the day cannot drift away from the rotation again.
pub const INTERACTIVE_WEATHER_TIME_SCALE: f64 =
    86_400.0 * WEATHER_DAYS_PER_PLANET_ROTATION / PLANET_ROTATION_REAL_SECONDS;
// Keep wind and thermal transport visibly fast without aging cloud phase
// changes and precipitation by the same hour-per-real-second clock.
const INTERACTIVE_CLOUD_MICROPHYSICS_TIME_SCALE: f64 = 60.0;
const WEATHER_MICROPHYSICS_TIMESTEP_SECONDS: f64 = WEATHER_TIMESTEP_SECONDS
    * INTERACTIVE_CLOUD_MICROPHYSICS_TIME_SCALE
    / INTERACTIVE_WEATHER_TIME_SCALE;
const WEATHER_MAX_STEPS_PER_ADVANCE: u64 = 12;
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
/// Largest share of its difference from the downwind cell that one cell trades
/// per step. Under a half so several donors cannot overshoot between them.
const WEATHER_ADVECTION_MAX_FRACTION: f64 = 0.3;
const WEATHER_PRESSURE_PER_KELVIN_PASCALS: f64 = 75.0;
const WEATHER_EVAPORATION_TIME_CONSTANT_SECONDS: f64 = 1_800.0;
const WEATHER_LATENT_COOLING_KELVIN_PER_UNIT: f64 = 2.5;
const WEATHER_CONDENSATION_TIME_CONSTANT_SECONDS: f64 = 900.0;
const WEATHER_LAPSE_RATE_KELVIN_PER_METER: f64 = 0.0065;
const WEATHER_OROGRAPHIC_RESPONSE_FRACTION: f64 = 0.2;
const WEATHER_MAX_OROGRAPHIC_UPLIFT_METERS_PER_SECOND: f64 = 8.0;
const WEATHER_MAX_LAPSE_DISPLACEMENT_METERS_PER_STEP: f64 = 750.0;
const WEATHER_PRECIPITATION_TIME_CONSTANT_SECONDS: f64 = 3_600.0;
const WEATHER_CLOUD_PRECIPITATION_THRESHOLD: f64 = 0.18;
const WEATHER_CLOUD_WATER_DEPTH_MILLIMETERS: f64 = 2.0;
const WEATHER_SNOW_MELT_TIME_CONSTANT_SECONDS: f64 = 7_200.0;
const WEATHER_SNOW_ACCUMULATION_FRACTION: f64 = 0.45;
const WEATHER_GPU_PRECIPITATION_SCALE_MILLIMETERS_PER_HOUR: f64 = 20.0;
const WEATHER_LATENT_HEATING_KELVIN_PER_UNIT: f64 = 2.5;
const WEATHER_STORM_LATENT_HEAT_SCALE_KELVIN: f64 = 2.5;
const WEATHER_FACE_COUNT: usize = 6;
const OVERLAY_BINS: usize = 16;
const WEATHER_ISOBAR_INTERVAL_PASCALS: f32 = 400.0;
const NEIGHBOUR_COUNT: usize = 4;

const WEATHER_GPU_FIELD_SIDE: usize = WEATHER_GRID_SIDE;

pub fn interactive_weather_time_seconds(presentation_time_seconds: f64) -> f64 {
    presentation_time_seconds.max(0.0) * INTERACTIVE_WEATHER_TIME_SCALE
}
pub const WEATHER_FIELD_TEXTURE_SIDE: u32 = WEATHER_GPU_FIELD_SIDE as u32;

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
    pub ground_moisture: f32,
    pub cloud_water: f32,
    pub surface_elevation_meters: f32,
    pub orographic_uplift_meters_per_second: f32,
    pub precipitation_millimeters_per_hour: f32,
    pub snow_cover: f32,
    pub latent_temperature_tendency_kelvin: f32,
    pub storm_intensity: f32,
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
    pub minimum_cloud_water: f32,
    pub maximum_cloud_water: f32,
    pub mean_cloud_water: f32,
    pub maximum_surface_elevation_meters: f32,
    pub maximum_orographic_uplift_meters_per_second: f32,
    pub minimum_ground_moisture: f32,
    pub maximum_ground_moisture: f32,
    pub mean_ground_moisture: f32,
    pub minimum_snow_cover: f32,
    pub maximum_snow_cover: f32,
    pub mean_snow_cover: f32,
    pub maximum_precipitation_millimeters_per_hour: f32,
    pub mean_precipitation_millimeters_per_hour: f32,
    pub maximum_latent_temperature_tendency_kelvin: f32,
    pub maximum_storm_intensity: f32,
    pub mean_storm_intensity: f32,
    pub maximum_wind_meters_per_second: f32,
    pub maximum_cfl: f64,
    pub relaxation_weight_at_1800_seconds: f64,
    pub pressure_conservation_error: f64,
    pub humidity_conservation_error: f64,
}

#[derive(Clone, Debug)]
pub struct WeatherFields {
    cells: Vec<WeatherCellState>,
    conservation_baseline: WeatherConservationBaseline,
}

enum WeatherPredictionCommand {
    Predict {
        fields: WeatherFields,
        sun_direction: DVec3,
    },
    Shutdown,
}

struct WeatherPredictionWorker {
    command_sender: Sender<WeatherPredictionCommand>,
    result_receiver: Receiver<WeatherFields>,
    thread: Option<JoinHandle<()>>,
    in_flight: bool,
}

impl WeatherPredictionWorker {
    fn new(grid: Arc<WeatherGrid>) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("weather-prediction".to_owned())
            .spawn(move || {
                while let Ok(command) = command_receiver.recv() {
                    match command {
                        WeatherPredictionCommand::Predict {
                            mut fields,
                            sun_direction,
                        } => {
                            WeatherState::simulate_fields_step(&grid, &mut fields, sun_direction);
                            if result_sender.send(fields).is_err() {
                                break;
                            }
                        }
                        WeatherPredictionCommand::Shutdown => break,
                    }
                }
            })
            .expect("weather prediction worker must start");
        Self {
            command_sender,
            result_receiver,
            thread: Some(thread),
            in_flight: false,
        }
    }

    fn request(&mut self, fields: WeatherFields, sun_direction: DVec3) {
        assert!(
            !self.in_flight,
            "only one weather prediction may be in flight"
        );
        self.command_sender
            .send(WeatherPredictionCommand::Predict {
                fields,
                sun_direction,
            })
            .expect("weather prediction worker must remain connected");
        self.in_flight = true;
    }

    fn poll(&mut self) -> Option<WeatherFields> {
        if !self.in_flight {
            return None;
        }
        match self.result_receiver.try_recv() {
            Ok(fields) => {
                self.in_flight = false;
                Some(fields)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                panic!("weather prediction worker disconnected")
            }
        }
    }

    fn wait(&mut self) -> WeatherFields {
        let fields = self
            .result_receiver
            .recv()
            .expect("weather prediction worker must return its in-flight state");
        self.in_flight = false;
        fields
    }
}

impl Drop for WeatherPredictionWorker {
    fn drop(&mut self) {
        let _ = self.command_sender.send(WeatherPredictionCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl WeatherFields {
    pub fn initial(grid: &WeatherGrid) -> Self {
        Self::initial_with_terrain(grid, None)
    }

    pub fn initial_with_terrain(
        grid: &WeatherGrid,
        terrain_samples: Option<&[TerrainClimateSample]>,
    ) -> Self {
        assert!(terrain_samples.is_none_or(|samples| samples.len() == grid.cells().len()));
        let cells = grid
            .cells()
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                initial_cell_state(
                    cell.direction,
                    terrain_samples.map(|samples| samples[index]),
                )
            })
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

    /// Applies one fixed-step radiative energy balance. Baked terrain supplies
    /// the ocean/land split and thermal inertia; the fallback used by unit
    /// tests retains the original deterministic proxy.
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

    /// Adds bounded surface evaporation before moisture advection. Ocean-like
    /// cells provide an effectively unlimited source; land cells draw from
    /// their local ground-moisture reservoir. The normalized humidity field
    /// relaxes toward saturation without an explicit `dt / tau` overshoot.
    pub fn evaporate_moisture(&mut self, step_seconds: f64) {
        if step_seconds <= 0.0 {
            return;
        }
        let relaxation =
            exponential_relaxation_weight(step_seconds, WEATHER_EVAPORATION_TIME_CONSTANT_SECONDS);
        for state in &mut self.cells {
            let speed = f64::from(state.east_wind_meters_per_second)
                .hypot(f64::from(state.north_wind_meters_per_second));
            let ocean_fraction = ocean_fraction_from_albedo(f64::from(state.surface_albedo));
            let land_fraction = 1.0 - ocean_fraction;
            let source_wetness = (ocean_fraction
                + land_fraction * f64::from(state.ground_moisture) * 0.4)
                .clamp(0.0, 1.0);
            let wind_factor = (1.0 + speed / 30.0).clamp(1.0, 3.0);
            let thermal_factor =
                ((f64::from(state.temperature_kelvin) - 245.0) / 35.0).clamp(0.0, 1.0);
            let deficit = (1.0 - f64::from(state.specific_humidity)).clamp(0.0, 1.0);
            let evaporated =
                (deficit * relaxation * source_wetness * wind_factor * thermal_factor * 0.35)
                    .min(deficit);
            state.specific_humidity =
                (f64::from(state.specific_humidity) + evaporated).clamp(0.0, 1.0) as f32;
            state.ground_moisture = (f64::from(state.ground_moisture)
                - evaporated * land_fraction * 0.35)
                .clamp(0.0, 1.0) as f32;
            state.temperature_kelvin = (f64::from(state.temperature_kelvin)
                - evaporated * WEATHER_LATENT_COOLING_KELVIN_PER_UNIT)
                .clamp(180.0, 340.0) as f32;
        }
    }

    /// Relaxes normalized water vapour toward a temperature-dependent
    /// saturation value. Supersaturated vapour becomes cloud water; when the
    /// cell is undersaturated, cloud water re-evaporates. The phase change is
    /// local and conservative, so this stage creates a diagnostic cloud field
    /// without pretending to model precipitation or terrain water yet.
    pub fn condense_cloud_water(&mut self, step_seconds: f64) {
        if step_seconds <= 0.0 {
            return;
        }
        let relaxation =
            exponential_relaxation_weight(step_seconds, WEATHER_CONDENSATION_TIME_CONSTANT_SECONDS);
        for state in &mut self.cells {
            let saturation = saturation_specific_humidity(f64::from(state.temperature_kelvin));
            let humidity = f64::from(state.specific_humidity);
            let cloud_water = f64::from(state.cloud_water);
            let mut latent_tendency = 0.0;
            if humidity > saturation {
                let condensed = ((humidity - saturation) * relaxation).min(humidity);
                state.specific_humidity = (humidity - condensed).clamp(0.0, 1.0) as f32;
                state.cloud_water = (cloud_water + condensed).clamp(0.0, 1.0) as f32;
                latent_tendency = condensed * WEATHER_LATENT_HEATING_KELVIN_PER_UNIT;
            } else if cloud_water > 0.0 {
                let evaporated = ((saturation - humidity) * relaxation).min(cloud_water);
                state.specific_humidity = (humidity + evaporated).clamp(0.0, 1.0) as f32;
                state.cloud_water = (cloud_water - evaporated).clamp(0.0, 1.0) as f32;
                latent_tendency = -evaporated * WEATHER_LATENT_HEATING_KELVIN_PER_UNIT;
            }
            state.temperature_kelvin =
                (f64::from(state.temperature_kelvin) + latent_tendency).clamp(180.0, 340.0) as f32;
            state.latent_temperature_tendency_kelvin = latent_tendency as f32;
            let uplift_factor = (f64::from(state.orographic_uplift_meters_per_second).max(0.0)
                / WEATHER_MAX_OROGRAPHIC_UPLIFT_METERS_PER_SECOND)
                .clamp(0.0, 1.0);
            let latent_factor =
                (latent_tendency.abs() / WEATHER_STORM_LATENT_HEAT_SCALE_KELVIN).clamp(0.0, 1.0);
            state.storm_intensity =
                (0.55 * f64::from(state.cloud_water) + 0.30 * uplift_factor + 0.15 * latent_factor)
                    .clamp(0.0, 1.0) as f32;
        }
    }

    /// Removes cloud water above the precipitation threshold and routes it to
    /// liquid ground moisture or a bounded snow reservoir. Snow melts when
    /// the cell warms, returning water to the same ground reservoir. Ocean
    /// precipitation remains an outlet. The emitted rate is a diagnostic
    /// rain-equivalent in millimetres per hour.
    pub fn precipitate_and_update_ground_moisture(&mut self, step_seconds: f64) {
        if step_seconds <= 0.0 {
            return;
        }
        let relaxation = exponential_relaxation_weight(
            step_seconds,
            WEATHER_PRECIPITATION_TIME_CONSTANT_SECONDS,
        );
        for state in &mut self.cells {
            let cloud_water = f64::from(state.cloud_water);
            let cloud_excess = ((cloud_water - WEATHER_CLOUD_PRECIPITATION_THRESHOLD)
                / (1.0 - WEATHER_CLOUD_PRECIPITATION_THRESHOLD))
                .clamp(0.0, 1.0);
            let removed = (cloud_excess * relaxation).min(cloud_water);
            state.cloud_water = (cloud_water - removed).clamp(0.0, 1.0) as f32;
            let ocean_fraction = ocean_fraction_from_albedo(f64::from(state.surface_albedo));
            let land_fraction = 1.0 - ocean_fraction;
            let freezing = ((273.15 - f64::from(state.temperature_kelvin)) / 6.0).clamp(0.0, 1.0);
            let snow_input =
                removed * land_fraction * freezing * WEATHER_SNOW_ACCUMULATION_FRACTION;
            let melt_relaxation = exponential_relaxation_weight(
                step_seconds,
                WEATHER_SNOW_MELT_TIME_CONSTANT_SECONDS,
            );
            let melt_fraction =
                ((f64::from(state.temperature_kelvin) - 268.0) / 12.0).clamp(0.0, 1.0);
            let melted = f64::from(state.snow_cover) * melt_fraction * melt_relaxation;
            state.snow_cover =
                (f64::from(state.snow_cover) + snow_input - melted).clamp(0.0, 1.0) as f32;
            let liquid_input = removed * (1.0 - freezing) + melted;
            let ground_input = liquid_input
                * land_fraction
                * (0.75 + 0.25 * (1.0 - f64::from(state.ground_moisture)));
            state.ground_moisture =
                (f64::from(state.ground_moisture) + ground_input).clamp(0.0, 1.0) as f32;
            state.precipitation_millimeters_per_hour =
                (removed * WEATHER_CLOUD_WATER_DEPTH_MILLIMETERS * 3_600.0 / step_seconds) as f32;
        }
    }

    /// Estimates vertical motion from the tangent wind crossing the local
    /// surface-elevation gradient. A bounded moist-adiabatic lapse tendency
    /// cools rising air and warms descending air. The response fraction keeps
    /// this one-layer proxy stable until real terrain and vertical columns are
    /// wired into the weather field.
    pub fn apply_lapse_rate_and_orographic_uplift(
        &mut self,
        grid: &WeatherGrid,
        step_seconds: f64,
    ) {
        if step_seconds <= 0.0 {
            return;
        }
        let mut next_uplift = vec![0.0_f32; self.cells.len()];
        let mut next_temperature = vec![0.0_f32; self.cells.len()];
        for (index, cell) in grid.cells().iter().enumerate() {
            let east_index = grid.directional_neighbour_index(index, cell.east) as usize;
            let west_index = grid.directional_neighbour_index(index, -cell.east) as usize;
            let north_index = grid.directional_neighbour_index(index, cell.north) as usize;
            let south_index = grid.directional_neighbour_index(index, -cell.north) as usize;
            let width = cell.area_square_meters.sqrt();
            let east_gradient = (f64::from(self.cells[east_index].surface_elevation_meters)
                - f64::from(self.cells[west_index].surface_elevation_meters))
                / (2.0 * width);
            let north_gradient = (f64::from(self.cells[north_index].surface_elevation_meters)
                - f64::from(self.cells[south_index].surface_elevation_meters))
                / (2.0 * width);
            let uplift = (f64::from(self.cells[index].east_wind_meters_per_second) * east_gradient
                + f64::from(self.cells[index].north_wind_meters_per_second) * north_gradient)
                .clamp(
                    -WEATHER_MAX_OROGRAPHIC_UPLIFT_METERS_PER_SECOND,
                    WEATHER_MAX_OROGRAPHIC_UPLIFT_METERS_PER_SECOND,
                );
            let displacement = (uplift * step_seconds * WEATHER_OROGRAPHIC_RESPONSE_FRACTION)
                .clamp(
                    -WEATHER_MAX_LAPSE_DISPLACEMENT_METERS_PER_STEP,
                    WEATHER_MAX_LAPSE_DISPLACEMENT_METERS_PER_STEP,
                );
            let lapse_delta = -WEATHER_LAPSE_RATE_KELVIN_PER_METER * displacement;
            next_uplift[index] = uplift as f32;
            next_temperature[index] = (f64::from(self.cells[index].temperature_kelvin)
                + lapse_delta)
                .clamp(180.0, 340.0) as f32;
        }
        for (index, state) in self.cells.iter_mut().enumerate() {
            state.orographic_uplift_meters_per_second = next_uplift[index];
            state.temperature_kelvin = next_temperature[index];
        }
    }

    /// MacCormack temperature advection. A backward predictor and a forward
    /// correction retain fronts better than one-pass semi-Lagrangian transport;
    /// the local source stencil bounds the correction so a coarse cube seam or
    /// sharp thermal front cannot create a new extremum.
    /// Moves temperature downwind as a paired exchange, so the area-weighted
    /// mean is unchanged by construction.
    ///
    /// Temperature is intensive, so it cannot ride the donor-cell mass
    /// transport humidity uses: that moves absolute `T * area`, and a cell
    /// which happens to receive from no upwind neighbour sheds a twentieth of
    /// 250K every step until the clamp catches it -- and the clamp is where
    /// conservation dies. Measured that way it bled 105K in four weather-days.
    /// Each cell instead trades a fraction of its *difference* from the cell
    /// downwind of it, and what leaves one enters the other exactly.
    ///
    /// The scheme before both of those was a semi-Lagrangian upstream sample
    /// with a clamped corrector, which conserves nothing: it bled 24K in four
    /// weather-days while radiation put back less than one. The field then fell
    /// past the 245K evaporation needs, moisture stopped being replenished, and
    /// the weather ran down to a cold, cloudless, motionless state with no way
    /// back.
    pub fn advect_temperature(&mut self, grid: &WeatherGrid, step_seconds: f64) {
        if step_seconds <= 0.0 {
            return;
        }
        let old_temperature = self
            .cells
            .iter()
            .map(|state| f64::from(state.temperature_kelvin))
            .collect::<Vec<_>>();
        // In kelvin-square-metres, so a transfer out of one cell is exactly the
        // transfer into the other whatever their areas.
        let mut heat_delta = vec![0.0_f64; old_temperature.len()];
        for (index, cell) in grid.cells().iter().enumerate() {
            let state = self.cells[index];
            let wind = cell.east * f64::from(state.east_wind_meters_per_second)
                + cell.north * f64::from(state.north_wind_meters_per_second);
            let speed = wind.length();
            if speed <= f64::EPSILON {
                continue;
            }
            let width = cell.area_square_meters.sqrt();
            // Bounded well under a half so several upwind donors cannot
            // between them push a cell past the values they came from.
            let fraction = (step_seconds * speed / width).min(WEATHER_ADVECTION_MAX_FRACTION);
            let target = grid.directional_neighbour_index(index, wind / speed) as usize;
            if target == index {
                continue;
            }
            let exchange = fraction
                * cell.area_square_meters
                * (old_temperature[index] - old_temperature[target]);
            heat_delta[index] -= exchange;
            heat_delta[target] += exchange;
        }
        for (index, cell) in grid.cells().iter().enumerate() {
            let temperature =
                old_temperature[index] + heat_delta[index] / cell.area_square_meters;
            self.cells[index].temperature_kelvin = temperature.clamp(180.0, 340.0) as f32;
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
        let next_humidity = advect_scalar_mass(grid, &old_humidity, &self.cells, step_seconds);
        for (state, humidity) in self.cells.iter_mut().zip(next_humidity) {
            state.specific_humidity = humidity as f32;
        }
    }

    /// Moves condensed cloud water with the same conservative tangent-wind
    /// transport as vapour. Without this pass, condensation can create cloud
    /// over a source region while the cloud reservoir itself remains pinned to
    /// the original cells even when the wind field is moving.
    pub fn advect_cloud_water(&mut self, grid: &WeatherGrid, step_seconds: f64) {
        if step_seconds <= 0.0 {
            return;
        }
        let old_cloud_water = self
            .cells
            .iter()
            .map(|state| f64::from(state.cloud_water))
            .collect::<Vec<_>>();
        let next_cloud_water =
            advect_scalar_mass(grid, &old_cloud_water, &self.cells, step_seconds);
        for (state, cloud_water) in self.cells.iter_mut().zip(next_cloud_water) {
            state.cloud_water = cloud_water as f32;
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
        let mut min_cloud_water = f32::INFINITY;
        let mut max_cloud_water = f32::NEG_INFINITY;
        let mut maximum_surface_elevation = 0.0_f32;
        let mut maximum_orographic_uplift = 0.0_f32;
        let mut min_ground_moisture = f32::INFINITY;
        let mut max_ground_moisture = f32::NEG_INFINITY;
        let mut min_snow_cover = f32::INFINITY;
        let mut max_snow_cover = f32::NEG_INFINITY;
        let mut maximum_precipitation = 0.0_f32;
        let mut maximum_latent_tendency = 0.0_f32;
        let mut maximum_storm_intensity = 0.0_f32;
        let mut mean_temperature = 0.0_f64;
        let mut mean_pressure = 0.0_f64;
        let mut mean_humidity = 0.0_f64;
        let mut mean_cloud_water = 0.0_f64;
        let mut mean_ground_moisture = 0.0_f64;
        let mut mean_snow_cover = 0.0_f64;
        let mut mean_precipitation = 0.0_f64;
        let mut mean_storm_intensity = 0.0_f64;
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
            min_cloud_water = min_cloud_water.min(state.cloud_water);
            max_cloud_water = max_cloud_water.max(state.cloud_water);
            maximum_surface_elevation =
                maximum_surface_elevation.max(state.surface_elevation_meters);
            maximum_orographic_uplift =
                maximum_orographic_uplift.max(state.orographic_uplift_meters_per_second.abs());
            min_ground_moisture = min_ground_moisture.min(state.ground_moisture);
            max_ground_moisture = max_ground_moisture.max(state.ground_moisture);
            min_snow_cover = min_snow_cover.min(state.snow_cover);
            max_snow_cover = max_snow_cover.max(state.snow_cover);
            maximum_precipitation =
                maximum_precipitation.max(state.precipitation_millimeters_per_hour);
            maximum_latent_tendency =
                maximum_latent_tendency.max(state.latent_temperature_tendency_kelvin.abs());
            maximum_storm_intensity = maximum_storm_intensity.max(state.storm_intensity);
            maximum_wind = maximum_wind.max(wind_speed);
            maximum_cfl = maximum_cfl.max(cfl);
            mean_temperature += f64::from(state.temperature_kelvin) * area;
            mean_pressure += f64::from(state.surface_pressure_pascals) * area;
            mean_humidity += f64::from(state.specific_humidity) * area;
            mean_cloud_water += f64::from(state.cloud_water) * area;
            mean_ground_moisture += f64::from(state.ground_moisture) * area;
            mean_snow_cover += f64::from(state.snow_cover) * area;
            mean_precipitation += f64::from(state.precipitation_millimeters_per_hour) * area;
            mean_storm_intensity += f64::from(state.storm_intensity) * area;
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
            minimum_cloud_water: min_cloud_water,
            maximum_cloud_water: max_cloud_water,
            mean_cloud_water: (mean_cloud_water / total_area) as f32,
            maximum_surface_elevation_meters: maximum_surface_elevation,
            maximum_orographic_uplift_meters_per_second: maximum_orographic_uplift,
            minimum_ground_moisture: min_ground_moisture,
            maximum_ground_moisture: max_ground_moisture,
            mean_ground_moisture: (mean_ground_moisture / total_area) as f32,
            minimum_snow_cover: min_snow_cover,
            maximum_snow_cover: max_snow_cover,
            mean_snow_cover: (mean_snow_cover / total_area) as f32,
            maximum_precipitation_millimeters_per_hour: maximum_precipitation,
            mean_precipitation_millimeters_per_hour: (mean_precipitation / total_area) as f32,
            maximum_latent_temperature_tendency_kelvin: maximum_latent_tendency,
            maximum_storm_intensity,
            mean_storm_intensity: (mean_storm_intensity / total_area) as f32,
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

    pub fn overlay_cloud_water_bins(
        &self,
        grid: &WeatherGrid,
    ) -> [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT] {
        let mut bins = [[0.0; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT];
        let cells_per_bin = WEATHER_GRID_SIDE / OVERLAY_BINS;
        for face in 0..WEATHER_FACE_COUNT {
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let mut cloud_water = 0.0_f64;
                    let mut area = 0.0_f64;
                    for local_y in 0..cells_per_bin {
                        for local_x in 0..cells_per_bin {
                            let index = cell_index(
                                face as u8,
                                x * cells_per_bin + local_x,
                                y * cells_per_bin + local_y,
                            );
                            let cell_area = grid.cells()[index].area_square_meters;
                            cloud_water += f64::from(self.cells()[index].cloud_water) * cell_area;
                            area += cell_area;
                        }
                    }
                    bins[face][y * OVERLAY_BINS + x] = (cloud_water / area) as f32;
                }
            }
        }
        bins
    }

    pub fn overlay_pressure_bins(
        &self,
        grid: &WeatherGrid,
    ) -> [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT] {
        let mut bins = [[0.0; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT];
        let cells_per_bin = WEATHER_GRID_SIDE / OVERLAY_BINS;
        for face in 0..WEATHER_FACE_COUNT {
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let mut pressure = 0.0_f64;
                    let mut area = 0.0_f64;
                    for local_y in 0..cells_per_bin {
                        for local_x in 0..cells_per_bin {
                            let index = cell_index(
                                face as u8,
                                x * cells_per_bin + local_x,
                                y * cells_per_bin + local_y,
                            );
                            let cell_area = grid.cells()[index].area_square_meters;
                            pressure +=
                                f64::from(self.cells()[index].surface_pressure_pascals) * cell_area;
                            area += cell_area;
                        }
                    }
                    bins[face][y * OVERLAY_BINS + x] = (pressure / area) as f32;
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
    pub cloud_water_bins: [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
    pub pressure_bins: [[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
    pub wind_bins: [[WeatherOverlayWind; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
}

impl WeatherDebugSnapshot {
    pub fn paint_overlay(&self, ui: &mut egui::Ui) {
        if !self.overlay_enabled {
            return;
        }

        ui.separator();
        ui.label("Weather field overlay: humidity / cloud / isobars / wind");
        let panel_size = egui::vec2(256.0, 176.0);
        let (rect, _) = ui.allocate_exact_size(panel_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let pressure_centres = pressure_centres(
            &self.pressure_bins,
            self.field_diagnostics.mean_pressure_pascals,
        );
        for face in 0..WEATHER_FACE_COUNT {
            let origin = rect.min + egui::vec2((face % 3) as f32 * 86.0, (face / 3) as f32 * 86.0);
            for y in 0..OVERLAY_BINS {
                for x in 0..OVERLAY_BINS {
                    let value = self.humidity_bins[face][y * OVERLAY_BINS + x].clamp(0.0, 1.0);
                    let cloud_water = self.cloud_water_bins[face][y * OVERLAY_BINS + x]
                        .clamp(0.0, 1.0)
                        .sqrt();
                    let base = [
                        185.0 - 145.0 * value,
                        85.0 + 115.0 * value,
                        45.0 + 190.0 * value,
                    ];
                    let colour = egui::Color32::from_rgb(
                        (base[0] + (235.0 - base[0]) * cloud_water) as u8,
                        (base[1] + (235.0 - base[1]) * cloud_water) as u8,
                        (base[2] + (235.0 - base[2]) * cloud_water) as u8,
                    );
                    let cell = egui::Rect::from_min_size(
                        origin + egui::vec2(x as f32 * 4.0, y as f32 * 4.0),
                        egui::vec2(4.1, 4.1),
                    );
                    painter.rect_filled(cell, 0.0, colour);
                }
            }
            paint_isobars(
                &painter,
                origin,
                &self.pressure_bins[face],
                self.field_diagnostics.minimum_pressure_pascals,
                self.field_diagnostics.maximum_pressure_pascals,
            );
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
            for centre in pressure_centres.iter().filter(|centre| centre.face == face) {
                let position =
                    origin + egui::vec2(centre.x as f32 * 4.0 + 2.0, centre.y as f32 * 4.0 + 2.0);
                let (label, colour) = if centre.high {
                    ("H", egui::Color32::from_rgb(255, 100, 100))
                } else {
                    ("L", egui::Color32::from_rgb(100, 210, 255))
                };
                painter.text(
                    position + egui::vec2(0.7, 0.7),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::monospace(8.0),
                    egui::Color32::BLACK,
                );
                painter.text(
                    position,
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::monospace(8.0),
                    colour,
                );
            }
            painter.text(
                origin + egui::vec2(3.0, 3.0),
                egui::Align2::LEFT_TOP,
                format!("F{face}"),
                egui::FontId::monospace(9.0),
                egui::Color32::WHITE,
            );
        }
        ui.label(format!(
            "Isobars: {:.0} hPa spacing, H/L centres | arrows: tangent wind",
            WEATHER_ISOBAR_INTERVAL_PASCALS / 100.0
        ));
        ui.label("Colour: humidity (brown -> blue), cloud water whitens");
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WeatherPressureCentre {
    face: usize,
    x: usize,
    y: usize,
    high: bool,
}

fn pressure_centres(
    pressure_bins: &[[f32; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT],
    mean_pressure_pascals: f32,
) -> Vec<WeatherPressureCentre> {
    let mut candidates = Vec::new();
    for (face, bins) in pressure_bins.iter().enumerate() {
        for y in 1..OVERLAY_BINS - 1 {
            for x in 1..OVERLAY_BINS - 1 {
                let pressure = bins[y * OVERLAY_BINS + x];
                if (pressure - mean_pressure_pascals).abs() < WEATHER_ISOBAR_INTERVAL_PASCALS * 0.5
                {
                    continue;
                }
                let mut higher_than_all = true;
                let mut lower_than_all = true;
                for neighbour_y in y - 1..=y + 1 {
                    for neighbour_x in x - 1..=x + 1 {
                        if neighbour_x == x && neighbour_y == y {
                            continue;
                        }
                        let neighbour = bins[neighbour_y * OVERLAY_BINS + neighbour_x];
                        higher_than_all &= pressure > neighbour;
                        lower_than_all &= pressure < neighbour;
                    }
                }
                if higher_than_all || lower_than_all {
                    candidates.push((
                        (pressure - mean_pressure_pascals).abs(),
                        WeatherPressureCentre {
                            face,
                            x,
                            y,
                            high: higher_than_all,
                        },
                    ));
                }
            }
        }
    }

    let mut guaranteed = Vec::new();
    for high in [false, true] {
        let extremum = pressure_bins
            .iter()
            .enumerate()
            .flat_map(|(face, bins)| {
                bins.iter()
                    .copied()
                    .enumerate()
                    .map(move |(index, pressure)| {
                        (
                            pressure,
                            WeatherPressureCentre {
                                face,
                                x: index % OVERLAY_BINS,
                                y: index / OVERLAY_BINS,
                                high,
                            },
                        )
                    })
            })
            .reduce(|a, b| {
                if (high && b.0 > a.0) || (!high && b.0 < a.0) {
                    b
                } else {
                    a
                }
            });
        if let Some((pressure, centre)) = extremum {
            if (pressure - mean_pressure_pascals).abs() >= WEATHER_ISOBAR_INTERVAL_PASCALS * 0.5 {
                candidates.push(((pressure - mean_pressure_pascals).abs(), centre));
                guaranteed.push(centre);
            }
        }
    }

    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut centres = guaranteed;
    for (_, candidate) in candidates {
        if centres.iter().any(|existing: &WeatherPressureCentre| {
            existing.face == candidate.face
                && existing.high == candidate.high
                && existing.x.abs_diff(candidate.x).pow(2) + existing.y.abs_diff(candidate.y).pow(2)
                    < 16
        }) {
            continue;
        }
        centres.push(candidate);
        if centres.len() == 12 {
            break;
        }
    }
    centres
}

fn paint_isobars(
    painter: &egui::Painter,
    origin: egui::Pos2,
    pressure_bins: &[f32; OVERLAY_BINS * OVERLAY_BINS],
    minimum_pressure_pascals: f32,
    maximum_pressure_pascals: f32,
) {
    for level in isobar_levels(minimum_pressure_pascals, maximum_pressure_pascals) {
        for y in 0..OVERLAY_BINS - 1 {
            for x in 0..OVERLAY_BINS - 1 {
                let top_left = (
                    origin + egui::vec2(x as f32 * 4.0 + 2.0, y as f32 * 4.0 + 2.0),
                    pressure_bins[y * OVERLAY_BINS + x],
                );
                let top_right = (
                    origin + egui::vec2((x + 1) as f32 * 4.0 + 2.0, y as f32 * 4.0 + 2.0),
                    pressure_bins[y * OVERLAY_BINS + x + 1],
                );
                let bottom_right = (
                    origin + egui::vec2((x + 1) as f32 * 4.0 + 2.0, (y + 1) as f32 * 4.0 + 2.0),
                    pressure_bins[(y + 1) * OVERLAY_BINS + x + 1],
                );
                let bottom_left = (
                    origin + egui::vec2(x as f32 * 4.0 + 2.0, (y + 1) as f32 * 4.0 + 2.0),
                    pressure_bins[(y + 1) * OVERLAY_BINS + x],
                );
                paint_isobar_triangle(painter, [top_left, top_right, bottom_right], level);
                paint_isobar_triangle(painter, [top_left, bottom_right, bottom_left], level);
            }
        }
    }
}

fn isobar_levels(minimum_pressure_pascals: f32, maximum_pressure_pascals: f32) -> Vec<f32> {
    if !minimum_pressure_pascals.is_finite()
        || !maximum_pressure_pascals.is_finite()
        || minimum_pressure_pascals > maximum_pressure_pascals
    {
        return Vec::new();
    }
    let first_level = (minimum_pressure_pascals / WEATHER_ISOBAR_INTERVAL_PASCALS).ceil()
        * WEATHER_ISOBAR_INTERVAL_PASCALS;
    let last_level = (maximum_pressure_pascals / WEATHER_ISOBAR_INTERVAL_PASCALS).floor()
        * WEATHER_ISOBAR_INTERVAL_PASCALS;
    let level_count =
        ((last_level - first_level) / WEATHER_ISOBAR_INTERVAL_PASCALS).floor() as isize + 1;
    (0..level_count.max(0))
        .map(|index| first_level + index as f32 * WEATHER_ISOBAR_INTERVAL_PASCALS)
        .collect()
}

fn paint_isobar_triangle(painter: &egui::Painter, vertices: [(egui::Pos2, f32); 3], level: f32) {
    let mut crossings = Vec::with_capacity(2);
    for edge in 0..3 {
        let (start_position, start_value) = vertices[edge];
        let (end_position, end_value) = vertices[(edge + 1) % 3];
        if (start_value < level) == (end_value < level) {
            continue;
        }
        let fraction = ((level - start_value) / (end_value - start_value)).clamp(0.0, 1.0);
        crossings.push(start_position.lerp(end_position, fraction));
    }
    if let [start, end] = crossings.as_slice() {
        painter.line_segment(
            [*start, *end],
            egui::Stroke::new(1.4, egui::Color32::from_black_alpha(190)),
        );
        painter.line_segment(
            [*start, *end],
            egui::Stroke::new(0.6, egui::Color32::from_rgb(255, 220, 80)),
        );
    }
}

#[allow(dead_code)] // the clock is intentionally staged before render-loop integration
pub struct WeatherState {
    grid: Arc<WeatherGrid>,
    fields: WeatherFields,
    next_fields: Option<WeatherFields>,
    following_fields: Option<WeatherFields>,
    prediction_worker: Option<WeatherPredictionWorker>,
    overlay_enabled: bool,
    last_input_time_seconds: f64,
    accumulator_seconds: f64,
    simulation_time_seconds: f64,
    completed_steps: u64,
}

impl WeatherState {
    pub fn new() -> Self {
        Self::new_with_terrain_samples(None)
    }

    pub fn new_with_terrain_samples(terrain_samples: Option<&[TerrainClimateSample]>) -> Self {
        let grid = Arc::new(WeatherGrid::new());
        let fields = WeatherFields::initial_with_terrain(&grid, terrain_samples);
        Self {
            grid,
            fields,
            next_fields: None,
            following_fields: None,
            prediction_worker: None,
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

    /// Packs the cloud field into six native-resolution RGBA8 cube-face layers.
    /// The channels are cloud water, storm intensity, humidity, and normalized
    /// precipitation. A renderer can upload this after fixed weather ticks
    /// without reducing the 64x64 simulation grid to coarse bins.
    pub fn cloud_field_texture_data(&self) -> Vec<u8> {
        Self::cloud_field_texture_data_for(&self.fields)
    }

    /// Packs the already-simulated next fixed weather state. The renderer
    /// blends toward this field throughout the whole 600-second interval.
    pub fn next_cloud_field_texture_data(&self) -> Option<Vec<u8>> {
        self.next_fields
            .as_ref()
            .map(Self::cloud_field_texture_data_for)
    }

    fn cloud_field_texture_data_for(fields: &WeatherFields) -> Vec<u8> {
        let mut bytes =
            vec![0_u8; WEATHER_FACE_COUNT * WEATHER_GPU_FIELD_SIDE * WEATHER_GPU_FIELD_SIDE * 4];
        for face in 0..WEATHER_FACE_COUNT {
            for y in 0..WEATHER_GPU_FIELD_SIDE {
                for x in 0..WEATHER_GPU_FIELD_SIDE {
                    let index = cell_index(face as u8, x, y);
                    let state = fields.cells()[index];
                    let offset =
                        ((face * WEATHER_GPU_FIELD_SIDE + y) * WEATHER_GPU_FIELD_SIDE + x) * 4;
                    bytes[offset] =
                        (f64::from(state.cloud_water).clamp(0.0, 1.0) * 255.0).round() as u8;
                    bytes[offset + 1] =
                        (f64::from(state.storm_intensity).clamp(0.0, 1.0) * 255.0).round() as u8;
                    bytes[offset + 2] =
                        (f64::from(state.specific_humidity).clamp(0.0, 1.0) * 255.0).round() as u8;
                    bytes[offset + 3] = ((f64::from(state.precipitation_millimeters_per_hour)
                        / WEATHER_GPU_PRECIPITATION_SCALE_MILLIMETERS_PER_HOUR)
                        .clamp(0.0, 1.0)
                        * 255.0)
                        .round() as u8;
                }
            }
        }
        bytes
    }

    /// Packs surface-coupling values into a second native-resolution cubemap:
    /// ground moisture, snow cover, normalized temperature, and an opaque
    /// marker. It follows the same temporal current/previous pairing as clouds.
    pub fn surface_field_texture_data(&self) -> Vec<u8> {
        Self::surface_field_texture_data_for(&self.fields)
    }

    pub fn next_surface_field_texture_data(&self) -> Option<Vec<u8>> {
        self.next_fields
            .as_ref()
            .map(Self::surface_field_texture_data_for)
    }

    fn surface_field_texture_data_for(fields: &WeatherFields) -> Vec<u8> {
        let mut bytes =
            vec![0_u8; WEATHER_FACE_COUNT * WEATHER_GPU_FIELD_SIDE * WEATHER_GPU_FIELD_SIDE * 4];
        for face in 0..WEATHER_FACE_COUNT {
            for y in 0..WEATHER_GPU_FIELD_SIDE {
                for x in 0..WEATHER_GPU_FIELD_SIDE {
                    let index = cell_index(face as u8, x, y);
                    let state = fields.cells()[index];
                    let offset =
                        ((face * WEATHER_GPU_FIELD_SIDE + y) * WEATHER_GPU_FIELD_SIDE + x) * 4;
                    bytes[offset] =
                        (f64::from(state.ground_moisture).clamp(0.0, 1.0) * 255.0).round() as u8;
                    bytes[offset + 1] =
                        (f64::from(state.snow_cover).clamp(0.0, 1.0) * 255.0).round() as u8;
                    bytes[offset + 2] = (((f64::from(state.temperature_kelvin) - 180.0) / 160.0)
                        .clamp(0.0, 1.0)
                        * 255.0)
                        .round() as u8;
                    bytes[offset + 3] = 255;
                }
            }
        }
        bytes
    }

    /// Simulates the next authoritative fixed-step state once, ready for the
    /// renderer to approach continuously. Returns true only when a new target
    /// was created and therefore needs uploading.
    pub fn prepare_next(&mut self, sun_direction: DVec3) -> bool {
        if self.next_fields.is_some() {
            return false;
        }
        let mut next_fields = self.fields.clone();
        Self::simulate_fields_step(&self.grid, &mut next_fields, sun_direction);
        self.next_fields = Some(next_fields);
        true
    }

    /// Prepares the first visible target before rendering starts, then keeps
    /// one additional fixed weather state in flight on a dedicated worker.
    /// The 600-second authoritative states remain identical to the synchronous
    /// path; only where the already-required prediction is calculated changes.
    pub fn enable_background_prediction(&mut self, sun_direction: DVec3) {
        assert!(
            self.prediction_worker.is_none(),
            "background weather prediction may only be enabled once"
        );
        self.prepare_next(sun_direction);
        let mut worker = WeatherPredictionWorker::new(Arc::clone(&self.grid));
        worker.request(
            self.next_fields
                .as_ref()
                .expect("the first weather target must be prepared")
                .clone(),
            sun_direction,
        );
        self.prediction_worker = Some(worker);
    }

    fn collect_background_prediction(&mut self) {
        if self.following_fields.is_some() {
            return;
        }
        self.following_fields = self
            .prediction_worker
            .as_mut()
            .and_then(WeatherPredictionWorker::poll);
    }

    fn request_background_prediction(&mut self, sun_direction: DVec3) {
        let fields = self
            .next_fields
            .as_ref()
            .expect("the next weather target must exist before predicting beyond it")
            .clone();
        self.prediction_worker
            .as_mut()
            .expect("background weather prediction must be enabled")
            .request(fields, sun_direction);
    }

    pub fn interpolation_fraction(&self) -> f32 {
        (self.accumulator_seconds / WEATHER_TIMESTEP_SECONDS).clamp(0.0, 1.0) as f32
    }

    /// Smooth local storm strength for presentation systems such as ocean
    /// swell. Spatial bilinear filtering avoids a wave-height jump at weather
    /// cell boundaries; temporal interpolation matches the cloud renderer.
    pub fn storm_intensity_at(&self, direction: DVec3) -> f32 {
        let current = sample_cell_property_bilinear(&self.fields, direction, |cell| {
            f64::from(cell.storm_intensity)
        });
        let next = self
            .next_fields
            .as_ref()
            .map(|fields| {
                sample_cell_property_bilinear(fields, direction, |cell| {
                    f64::from(cell.storm_intensity)
                })
            })
            .unwrap_or(current);
        (current + (next - current) * f64::from(self.interpolation_fraction())) as f32
    }

    pub fn visual_time_seconds(&self) -> f64 {
        self.simulation_time_seconds + self.accumulator_seconds
    }

    /// Consumes scene time through a fixed 600-second weather clock. The
    /// authoritative physics still changes only on complete steps; the
    /// renderer uses `interpolation_fraction` between those states.
    #[allow(dead_code)] // consumed when the render-loop clock is wired in
    pub fn advance_to(&mut self, scene_time_seconds: f64) -> u64 {
        let sun_direction = weather_sun_direction(scene_time_seconds);
        self.advance_to_with_sun(scene_time_seconds, sun_direction)
    }

    /// Advances the fixed weather clock using a sun direction expressed in the
    /// planet-local frame. The renderer uses this entry point so scenario sun
    /// waypoints and the weather terminator share the same lighting direction.
    pub fn advance_to_with_sun(&mut self, scene_time_seconds: f64, sun_direction: DVec3) -> u64 {
        assert!(
            self.prediction_worker.is_none(),
            "interactive weather must use advance_interactive_to_with_sun"
        );
        if !scene_time_seconds.is_finite() || scene_time_seconds <= self.last_input_time_seconds {
            return 0;
        }
        self.prepare_next(sun_direction);
        self.accumulator_seconds += scene_time_seconds - self.last_input_time_seconds;
        self.last_input_time_seconds = scene_time_seconds;
        let mut completed = 0;
        while self.accumulator_seconds >= WEATHER_TIMESTEP_SECONDS
            && completed < WEATHER_MAX_STEPS_PER_ADVANCE
        {
            self.fields = self
                .next_fields
                .take()
                .expect("next weather state must be prepared before a fixed-step boundary");
            self.accumulator_seconds -= WEATHER_TIMESTEP_SECONDS;
            self.simulation_time_seconds += WEATHER_TIMESTEP_SECONDS;
            self.completed_steps += 1;
            completed += 1;
            self.prepare_next(sun_direction);
        }
        if self.accumulator_seconds >= WEATHER_TIMESTEP_SECONDS {
            self.accumulator_seconds = self
                .accumulator_seconds
                .rem_euclid(WEATHER_TIMESTEP_SECONDS);
        }
        completed
    }

    /// Advances the interactive clock without ever calculating a weather state
    /// on the render thread. Normally the one-state-ahead worker has finished
    /// long before the 600-second boundary. If it has not, presentation holds
    /// on the exact target at blend 1 until the result arrives rather than
    /// stalling or handing off to a partial state.
    pub fn advance_interactive_to_with_sun(
        &mut self,
        scene_time_seconds: f64,
        sun_direction: DVec3,
    ) -> u64 {
        assert!(
            self.prediction_worker.is_some(),
            "background weather prediction must be enabled"
        );
        self.collect_background_prediction();
        if scene_time_seconds.is_finite() && scene_time_seconds > self.last_input_time_seconds {
            self.accumulator_seconds += scene_time_seconds - self.last_input_time_seconds;
            self.last_input_time_seconds = scene_time_seconds;
        }

        let mut completed = 0;
        while self.accumulator_seconds >= WEATHER_TIMESTEP_SECONDS
            && completed < WEATHER_MAX_STEPS_PER_ADVANCE
        {
            let Some(following_fields) = self.following_fields.take() else {
                break;
            };
            self.fields = self
                .next_fields
                .take()
                .expect("the visible weather target must exist at a fixed-step boundary");
            self.next_fields = Some(following_fields);
            self.accumulator_seconds -= WEATHER_TIMESTEP_SECONDS;
            self.simulation_time_seconds += WEATHER_TIMESTEP_SECONDS;
            self.completed_steps += 1;
            completed += 1;
            self.request_background_prediction(sun_direction);
            self.collect_background_prediction();
        }

        // A resume from suspend must not build an arbitrarily long queue of
        // obsolete weather states. Keep one pending boundary plus the current
        // fractional phase; the worker will hand that boundary off smoothly.
        if self.accumulator_seconds >= WEATHER_TIMESTEP_SECONDS * 2.0 {
            self.accumulator_seconds = WEATHER_TIMESTEP_SECONDS
                + self
                    .accumulator_seconds
                    .rem_euclid(WEATHER_TIMESTEP_SECONDS);
        }
        completed
    }

    /// Executes exactly one fixed weather step without consuming render-clock
    /// time. This is a diagnostic control for inspecting field changes while
    /// the interactive world is paused with F10.
    pub fn step_once(&mut self, sun_direction: DVec3) {
        if self.prediction_worker.is_some() {
            self.collect_background_prediction();
            let following_fields = self.following_fields.take().unwrap_or_else(|| {
                self.prediction_worker
                    .as_mut()
                    .expect("background weather prediction must be enabled")
                    .wait()
            });
            self.fields = self
                .next_fields
                .take()
                .expect("manual weather step must have a prepared target");
            self.next_fields = Some(following_fields);
            self.accumulator_seconds = 0.0;
            self.simulation_time_seconds += WEATHER_TIMESTEP_SECONDS;
            self.completed_steps += 1;
            self.request_background_prediction(sun_direction);
            return;
        }
        self.prepare_next(sun_direction);
        self.fields = self
            .next_fields
            .take()
            .expect("manual weather step must have a prepared target");
        self.accumulator_seconds = 0.0;
        self.simulation_time_seconds += WEATHER_TIMESTEP_SECONDS;
        self.completed_steps += 1;
        self.prepare_next(sun_direction);
    }

    fn simulate_fields_step(grid: &WeatherGrid, fields: &mut WeatherFields, sun_direction: DVec3) {
        fields.apply_insolation_and_radiative_cooling(
            grid,
            sun_direction,
            WEATHER_TIMESTEP_SECONDS,
        );
        fields.diagnose_pressure_from_temperature(grid);
        fields.update_wind_from_pressure(grid, WEATHER_TIMESTEP_SECONDS);
        fields.apply_lapse_rate_and_orographic_uplift(grid, WEATHER_TIMESTEP_SECONDS);
        fields.evaporate_moisture(WEATHER_MICROPHYSICS_TIMESTEP_SECONDS);
        fields.advect_temperature(grid, WEATHER_TIMESTEP_SECONDS);
        fields.advect_humidity(grid, WEATHER_TIMESTEP_SECONDS);
        fields.advect_cloud_water(grid, WEATHER_TIMESTEP_SECONDS);
        fields.condense_cloud_water(WEATHER_MICROPHYSICS_TIMESTEP_SECONDS);
        fields.precipitate_and_update_ground_moisture(WEATHER_MICROPHYSICS_TIMESTEP_SECONDS);
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
            cloud_water_bins: self.fields.overlay_cloud_water_bins(&self.grid),
            pressure_bins: self.fields.overlay_pressure_bins(&self.grid),
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

fn advect_scalar_mass(
    grid: &WeatherGrid,
    old_values: &[f64],
    states: &[WeatherCellState],
    step_seconds: f64,
) -> Vec<f64> {
    // Humidity and cloud water are fractions, so a receiving cell can only
    // hold up to 1.
    advect_scalar_mass_bounded(grid, old_values, states, step_seconds, 1.0)
}

/// The same donor-cell transport with an explicit ceiling on what a receiving
/// cell can hold. Temperature has no such ceiling in transport -- it is bounded
/// afterwards by its own clamp -- and passing one meant for a 0-to-1 fraction
/// would throttle nearly every transfer.
fn advect_scalar_mass_bounded(
    grid: &WeatherGrid,
    old_values: &[f64],
    states: &[WeatherCellState],
    step_seconds: f64,
    maximum_value: f64,
) -> Vec<f64> {
    debug_assert_eq!(old_values.len(), grid.cells().len());
    debug_assert_eq!(states.len(), grid.cells().len());
    let mut transfers = vec![(0_usize, 0.0_f64); old_values.len()];
    let mut incoming = vec![0.0_f64; old_values.len()];
    for (index, cell) in grid.cells().iter().enumerate() {
        let state = states[index];
        let wind = cell.east * f64::from(state.east_wind_meters_per_second)
            + cell.north * f64::from(state.north_wind_meters_per_second);
        let speed = wind.length();
        if speed <= f64::EPSILON {
            continue;
        }
        let fraction = (step_seconds * speed / cell.area_square_meters.sqrt()).min(0.95);
        let target = grid.directional_neighbour_index(index, wind / speed) as usize;
        let mass = old_values[index] * cell.area_square_meters;
        let transfer = mass * fraction;
        transfers[index] = (target, transfer);
        incoming[target] += transfer;
    }

    let mut mass_delta = vec![0.0_f64; old_values.len()];
    for (index, (target, transfer)) in transfers.into_iter().enumerate() {
        if transfer == 0.0 {
            continue;
        }
        let target_capacity =
            grid.cells()[target].area_square_meters * (maximum_value - old_values[target]);
        let target_scale = if incoming[target] > target_capacity {
            (target_capacity / incoming[target]).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let actual_transfer = transfer * target_scale;
        mass_delta[index] -= actual_transfer;
        mass_delta[target] += actual_transfer;
    }

    grid.cells()
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let mass = old_values[index] * cell.area_square_meters + mass_delta[index];
            (mass / cell.area_square_meters).clamp(0.0, 1.0)
        })
        .collect()
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

fn sample_scalar_bilinear(values: &[f64], direction: DVec3) -> f64 {
    sample_scalar_bilinear_with_bounds(values, direction).0
}

fn sample_scalar_bilinear_with_bounds(values: &[f64], direction: DVec3) -> (f64, f64, f64) {
    let (face, fractional_i, fractional_j) = direction_to_fractional_cell(direction);
    let i0 = fractional_i.floor() as isize;
    let j0 = fractional_j.floor() as isize;
    let tx = fractional_i - i0 as f64;
    let ty = fractional_j - j0 as f64;
    let sample = |i: isize, j: isize| values[adjacent_cell_index(face, i, j) as usize];
    let west_south = sample(i0, j0);
    let east_south = sample(i0 + 1, j0);
    let west_north = sample(i0, j0 + 1);
    let east_north = sample(i0 + 1, j0 + 1);
    let south = west_south + (east_south - west_south) * tx;
    let north = west_north + (east_north - west_north) * tx;
    (
        south + (north - south) * ty,
        west_south.min(east_south).min(west_north).min(east_north),
        west_south.max(east_south).max(west_north).max(east_north),
    )
}

fn sample_cell_property_bilinear(
    fields: &WeatherFields,
    direction: DVec3,
    property: impl Fn(WeatherCellState) -> f64,
) -> f64 {
    let (face, fractional_i, fractional_j) = direction_to_fractional_cell(direction);
    let i0 = fractional_i.floor() as isize;
    let j0 = fractional_j.floor() as isize;
    let tx = fractional_i - i0 as f64;
    let ty = fractional_j - j0 as f64;
    let sample =
        |i: isize, j: isize| property(fields.cells()[adjacent_cell_index(face, i, j) as usize]);
    let west_south = sample(i0, j0);
    let east_south = sample(i0 + 1, j0);
    let west_north = sample(i0, j0 + 1);
    let east_north = sample(i0 + 1, j0 + 1);
    let south = west_south + (east_south - west_south) * tx;
    let north = west_north + (east_north - west_north) * tx;
    south + (north - south) * ty
}

fn direction_to_cell(direction: DVec3) -> (u8, usize, usize) {
    let (best_face, fractional_i, fractional_j) = direction_to_fractional_cell(direction);
    (
        best_face,
        fractional_i
            .round()
            .clamp(0.0, (WEATHER_GRID_SIDE - 1) as f64) as usize,
        fractional_j
            .round()
            .clamp(0.0, (WEATHER_GRID_SIDE - 1) as f64) as usize,
    )
}

fn direction_to_fractional_cell(direction: DVec3) -> (u8, f64, f64) {
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
    let fractional_i = (u + 1.0) * 0.5 * WEATHER_GRID_SIDE as f64 - 0.5;
    let fractional_j = (v + 1.0) * 0.5 * WEATHER_GRID_SIDE as f64 - 0.5;
    (best_face, fractional_i, fractional_j)
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

fn initial_cell_state(
    direction: DVec3,
    terrain_sample: Option<TerrainClimateSample>,
) -> WeatherCellState {
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
    let (surface_elevation, surface_albedo, heat_capacity, ground_moisture) = terrain_sample
        .map(|sample| {
            (
                sample.surface_elevation_meters,
                sample.surface_albedo,
                sample.heat_capacity_joules_per_square_meter_kelvin,
                sample.ground_moisture,
            )
        })
        .unwrap_or_else(|| {
            let land_fraction = proxy_land_fraction(direction, latitude, longitude);
            (
                proxy_surface_elevation_meters(direction, latitude, longitude),
                WEATHER_OCEAN_ALBEDO + (WEATHER_LAND_ALBEDO - WEATHER_OCEAN_ALBEDO) * land_fraction,
                WEATHER_OCEAN_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN
                    + (WEATHER_LAND_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN
                        - WEATHER_OCEAN_HEAT_CAPACITY_JOULES_PER_SQUARE_METER_KELVIN)
                        * land_fraction,
                0.65 * land_fraction,
            )
        });
    WeatherCellState {
        temperature_kelvin: temperature_kelvin as f32,
        surface_pressure_pascals: surface_pressure_pascals as f32,
        specific_humidity: specific_humidity as f32,
        east_wind_meters_per_second: east_wind_meters_per_second as f32,
        north_wind_meters_per_second: north_wind_meters_per_second as f32,
        surface_albedo: surface_albedo as f32,
        heat_capacity_joules_per_square_meter_kelvin: heat_capacity as f32,
        ground_moisture: ground_moisture as f32,
        cloud_water: 0.0,
        surface_elevation_meters: surface_elevation as f32,
        orographic_uplift_meters_per_second: 0.0,
        precipitation_millimeters_per_hour: 0.0,
        snow_cover: 0.0,
        latent_temperature_tendency_kelvin: 0.0,
        storm_intensity: 0.0,
    }
}

/// Saturation in the normalized humidity units used by this diagnostic
/// model. It follows the expected Clausius-Clapeyron direction (warmer air
/// holds more vapour) while staying inside the field's `[0,1]` contract.
fn saturation_specific_humidity(temperature_kelvin: f64) -> f64 {
    (0.55 * (0.045 * (temperature_kelvin - 273.15)).exp()).clamp(0.05, 0.98)
}

/// Seam-safe low-resolution relief fallback used by unit tests and placeholder
/// launches without an active baked outmap.
fn proxy_surface_elevation_meters(direction: DVec3, latitude: f64, longitude: f64) -> f64 {
    let land_fraction = proxy_land_fraction(direction, latitude, longitude);
    let broad = (0.5
        + 0.5 * (2.0 * longitude + 0.3 * latitude).sin() * latitude.cos()
        + 0.25 * (3.0 * longitude - latitude).cos())
    .clamp(0.0, 1.0);
    let ridge = (0.5
        + 0.5 * (5.0 * longitude - 1.7 * latitude).sin()
        + 0.25 * (7.0 * longitude + 0.8 * latitude).cos())
    .clamp(0.0, 1.0);
    (land_fraction * (250.0 + 2_000.0 * broad + 1_200.0 * ridge)).clamp(0.0, 4_500.0)
}

/// Deterministic land/ocean fallback for placeholder launches without baked
/// terrain. Production weather initialization supplies the outmap samples.
fn proxy_land_fraction(direction: DVec3, latitude: f64, longitude: f64) -> f64 {
    let continental_signal = 0.5
        + 0.22 * (2.0 * longitude + 0.7).sin() * latitude.cos()
        + 0.16 * (3.0 * longitude - 0.4).cos() * (2.0 * latitude).cos()
        + 0.10 * direction.x * direction.y
        + 0.07 * (5.0 * longitude + 1.2 * latitude).sin();
    ((continental_signal - 0.40) / 0.20).clamp(0.0, 1.0)
}

fn ocean_fraction_from_albedo(albedo: f64) -> f64 {
    ((WEATHER_LAND_ALBEDO - albedo) / (WEATHER_LAND_ALBEDO - WEATHER_OCEAN_ALBEDO)).clamp(0.0, 1.0)
}

fn weather_sun_direction(scene_time_seconds: f64) -> DVec3 {
    let day_phase = scene_time_seconds.rem_euclid(86_400.0) / 86_400.0 * std::f64::consts::TAU;
    DVec3::new(day_phase.cos(), 0.25, day_phase.sin()).normalize()
}

/// Applies an Earth-like annual declination cycle to a planet-local sun ray.
/// The daily azimuth is supplied by the caller; only the orbital tilt changes
/// over the year. At orbital phase zero this matches the established default
/// solstice orientation, so existing startup/scenario lighting is unchanged.
pub fn seasonal_sun_direction(base_direction: DVec3, orbital_time_seconds: f64) -> DVec3 {
    let base_direction = base_direction.normalize_or_zero();
    let horizontal = DVec3::new(base_direction.x, 0.0, base_direction.z).normalize_or_zero();
    if horizontal.length_squared() <= f64::EPSILON {
        return DVec3::Y;
    }
    let phase = orbital_time_seconds.rem_euclid(WEATHER_ORBITAL_PERIOD_SECONDS)
        / WEATHER_ORBITAL_PERIOD_SECONDS
        * std::f64::consts::TAU;
    let declination = EARTH_AXIAL_TILT_RADIANS * phase.cos();
    (horizontal * declination.cos() + DVec3::Y * declination.sin()).normalize()
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
    fn local_storm_presentation_is_spatially_filtered_and_temporally_interpolated() {
        let mut weather = WeatherState::new();
        for cell in &mut weather.fields.cells {
            cell.storm_intensity = 0.2;
        }
        let mut next = weather.fields.clone();
        for cell in &mut next.cells {
            cell.storm_intensity = 0.8;
        }
        weather.next_fields = Some(next);
        weather.accumulator_seconds = WEATHER_TIMESTEP_SECONDS * 0.5;

        let sampled = weather.storm_intensity_at(DVec3::new(0.3, 0.8, -0.5).normalize());
        assert!((sampled - 0.5).abs() < 1.0e-6, "sampled {sampled}");
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
    fn seasonal_sun_direction_cycles_declination_without_changing_azimuth() {
        let base = DVec3::new(0.4, EARTH_AXIAL_TILT_RADIANS.sin(), 0.6).normalize();
        let start = seasonal_sun_direction(base, 0.0);
        let equinox = seasonal_sun_direction(base, WEATHER_ORBITAL_PERIOD_SECONDS * 0.25);
        let opposite = seasonal_sun_direction(base, WEATHER_ORBITAL_PERIOD_SECONDS * 0.5);
        let expected_horizontal = DVec3::new(base.x, 0.0, base.z).normalize();
        for direction in [start, equinox, opposite] {
            assert!((direction.length() - 1.0).abs() < 1.0e-12);
            let horizontal = DVec3::new(direction.x, 0.0, direction.z).normalize();
            assert!(horizontal.dot(expected_horizontal) > 1.0 - 1.0e-12);
        }
        assert!((start.y - EARTH_AXIAL_TILT_RADIANS.sin()).abs() < 1.0e-12);
        assert!(equinox.y.abs() < 1.0e-12);
        assert!((opposite.y + EARTH_AXIAL_TILT_RADIANS.sin()).abs() < 1.0e-12);
    }

    #[test]
    fn baked_terrain_samples_override_the_climate_fallback() {
        let grid = WeatherGrid::new();
        let sample = TerrainClimateSample {
            land_fraction: 1.0,
            surface_elevation_meters: 3_210.0,
            surface_albedo: 0.42,
            heat_capacity_joules_per_square_meter_kelvin: 3.4e6,
            ground_moisture: 0.17,
        };
        let samples = vec![sample; grid.cells().len()];
        let fields = WeatherFields::initial_with_terrain(&grid, Some(&samples));
        assert!(fields.cells().iter().all(|state| {
            (f64::from(state.surface_elevation_meters) - sample.surface_elevation_meters).abs()
                < 1.0e-3
                && (f64::from(state.surface_albedo) - sample.surface_albedo).abs() < 1.0e-6
                && (f64::from(state.ground_moisture) - sample.ground_moisture).abs() < 1.0e-6
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
    fn pressure_overlay_is_area_weighted_deterministic_and_bounded() {
        let grid = WeatherGrid::new();
        let fields = WeatherFields::initial(&grid);
        let diagnostics = fields.diagnostics(&grid);
        let first = fields.overlay_pressure_bins(&grid);
        assert_eq!(first, fields.overlay_pressure_bins(&grid));
        assert!(first.iter().flatten().all(|pressure| {
            pressure.is_finite()
                && *pressure >= diagnostics.minimum_pressure_pascals
                && *pressure <= diagnostics.maximum_pressure_pascals
        }));
        assert!(
            isobar_levels(
                diagnostics.minimum_pressure_pascals,
                diagnostics.maximum_pressure_pascals,
            )
            .len()
                >= 2
        );
        let centres = pressure_centres(&first, diagnostics.mean_pressure_pascals);
        assert!(centres.iter().any(|centre| centre.high));
        assert!(centres.iter().any(|centre| !centre.high));
    }

    #[test]
    fn isobars_use_fixed_four_hectopascal_levels() {
        assert_eq!(
            isobar_levels(99_550.0, 101_750.0),
            vec![
                99_600.0, 100_000.0, 100_400.0, 100_800.0, 101_200.0, 101_600.0
            ]
        );
        assert!(isobar_levels(f32::NAN, 101_000.0).is_empty());
        assert!(isobar_levels(102_000.0, 101_000.0).is_empty());
    }

    #[test]
    fn pressure_centres_find_distinct_highs_and_lows() {
        let mean = 101_000.0;
        let mut bins = [[mean; OVERLAY_BINS * OVERLAY_BINS]; WEATHER_FACE_COUNT];
        bins[0][5 * OVERLAY_BINS + 6] = mean + 800.0;
        bins[4][10 * OVERLAY_BINS + 9] = mean - 800.0;
        let centres = pressure_centres(&bins, mean);
        assert!(centres.contains(&WeatherPressureCentre {
            face: 0,
            x: 6,
            y: 5,
            high: true,
        }));
        assert!(centres.contains(&WeatherPressureCentre {
            face: 4,
            x: 9,
            y: 10,
            high: false,
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
    fn cloud_water_transport_moves_with_wind_and_preserves_area_integral() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        for state in &mut fields.cells {
            state.east_wind_meters_per_second = 0.0;
            state.north_wind_meters_per_second = 0.0;
            state.cloud_water = 0.0;
        }
        let source_index = cell_index(0, 0, WEATHER_GRID_SIDE / 2);
        let source = grid.cells()[source_index];
        let target_index = grid.directional_neighbour_index(source_index, source.east) as usize;
        fields.cells[source_index].east_wind_meters_per_second = 20.0;
        fields.cells[source_index].cloud_water = 0.8;
        let before_mass = grid
            .cells()
            .iter()
            .zip(&fields.cells)
            .map(|(cell, state)| f64::from(state.cloud_water) * cell.area_square_meters)
            .sum::<f64>();

        fields.advect_cloud_water(&grid, WEATHER_TIMESTEP_SECONDS);

        assert!(fields.cells[target_index].cloud_water > 0.0);
        let after_mass = grid
            .cells()
            .iter()
            .zip(&fields.cells)
            .map(|(cell, state)| f64::from(state.cloud_water) * cell.area_square_meters)
            .sum::<f64>();
        assert!((before_mass - after_mass).abs() / before_mass < 1.0e-6);
        assert!(
            fields
                .cells
                .iter()
                .all(|state| (0.0..=1.0).contains(&state.cloud_water))
        );
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
    fn temperature_advection_conserves_the_planet_it_is_stirring() {
        // The one that let the weather die. Advection only moves heat about,
        // so the area-weighted mean must come out where it went in. The old
        // semi-Lagrangian scheme bled 24K of it in four weather-days, and
        // rewriting it as donor-cell mass transport bled 105K; both left the
        // field under the 245K that evaporation needs, and with evaporation the
        // only source of moisture in the model there is no way back from that.
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        let mean = |fields: &WeatherFields| {
            let mut area = 0.0;
            let mut total = 0.0;
            for (cell, state) in grid.cells().iter().zip(fields.cells()) {
                area += cell.area_square_meters;
                total += f64::from(state.temperature_kelvin) * cell.area_square_meters;
            }
            total / area
        };
        // Stir it first, so the field has gradients and a wind to move them.
        fields.apply_insolation_and_radiative_cooling(&grid, DVec3::X, WEATHER_TIMESTEP_SECONDS);
        fields.diagnose_pressure_from_temperature(&grid);
        fields.update_wind_from_pressure(&grid, WEATHER_TIMESTEP_SECONDS);
        let before = mean(&fields);
        let before_cells = fields.cells().to_vec();
        for _ in 0..50 {
            fields.advect_temperature(&grid, WEATHER_TIMESTEP_SECONDS);
        }
        let after = mean(&fields);
        // Cell temperatures are stored as f32, whose ULP at 267K is about
        // 3e-5, so fifty steps cannot land closer than storage rounding. Still
        // four orders of magnitude tighter than the 24K this is guarding.
        assert!(
            (after - before).abs() < 1.0e-3,
            "fifty steps of advection moved the mean from {before} K to {after} K"
        );
        // And it has to actually stir: a no-op conserves perfectly too.
        let moved = fields
            .cells()
            .iter()
            .zip(&before_cells)
            .filter(|(now, was)| {
                (f64::from(now.temperature_kelvin) - f64::from(was.temperature_kelvin)).abs() > 0.05
            })
            .count();
        assert!(
            moved > grid.cells().len() / 20,
            "only {moved} cells changed, so nothing was advected"
        );
        // Bounded: no cell may be pushed outside the range it started in.
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for state in &before_cells {
            low = low.min(f64::from(state.temperature_kelvin));
            high = high.max(f64::from(state.temperature_kelvin));
        }
        for state in fields.cells() {
            let t = f64::from(state.temperature_kelvin);
            assert!(t >= low - 1.0e-3 && t <= high + 1.0e-3, "cell reached {t} K");
        }
    }

    #[test]
    fn temperature_advection_is_deterministic_bounded_and_changes_the_field() {
        let grid = WeatherGrid::new();
        let mut first = WeatherFields::initial(&grid);
        let mut second = WeatherFields::initial(&grid);
        let before = first.cells().to_vec();
        for fields in [&mut first, &mut second] {
            fields.update_wind_from_pressure(&grid, WEATHER_TIMESTEP_SECONDS);
            fields.advect_temperature(&grid, WEATHER_TIMESTEP_SECONDS);
        }
        assert_eq!(first.cells(), second.cells());
        assert!(first.cells().iter().all(|state| {
            state.temperature_kelvin.is_finite()
                && (180.0..=340.0).contains(&state.temperature_kelvin)
        }));
        assert!(
            first
                .cells()
                .iter()
                .zip(before)
                .any(|(after, before)| after.temperature_kelvin != before.temperature_kelvin)
        );
    }

    #[test]
    fn maccormack_temperature_advection_does_not_create_new_extrema() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        for state in &mut fields.cells {
            state.temperature_kelvin = 280.0;
            state.east_wind_meters_per_second = 60.0;
            state.north_wind_meters_per_second = 0.0;
        }
        fields.cells[cell_index(0, WEATHER_GRID_SIDE / 2, WEATHER_GRID_SIDE / 2)]
            .temperature_kelvin = 320.0;
        fields.advect_temperature(&grid, WEATHER_TIMESTEP_SECONDS);
        assert!(
            fields
                .cells()
                .iter()
                .all(|state| (280.0..=320.0).contains(&state.temperature_kelvin))
        );
    }

    #[test]
    fn evaporation_adds_bounded_humidity_and_cools_source_cells() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        let before = fields.cells().to_vec();
        fields.evaporate_moisture(WEATHER_TIMESTEP_SECONDS);
        assert!(fields.cells().iter().all(|state| {
            (0.0..=1.0).contains(&state.specific_humidity)
                && (0.0..=1.0).contains(&state.ground_moisture)
                && (180.0..=340.0).contains(&state.temperature_kelvin)
        }));
        assert!(
            fields
                .cells()
                .iter()
                .zip(&before)
                .any(|(after, before)| after.specific_humidity > before.specific_humidity)
        );
        assert!(
            fields
                .cells()
                .iter()
                .zip(&before)
                .any(|(after, before)| after.temperature_kelvin < before.temperature_kelvin)
        );
        assert!(
            fields
                .cells()
                .iter()
                .zip(&before)
                .all(|(after, before)| after.ground_moisture <= before.ground_moisture)
        );
    }

    #[test]
    fn condensation_creates_bounded_cloud_water_and_conserves_local_phase_mass() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        let before = fields.cells().to_vec();
        fields.condense_cloud_water(WEATHER_TIMESTEP_SECONDS);
        assert!(fields.cells().iter().all(|state| {
            (0.0..=1.0).contains(&state.specific_humidity)
                && (0.0..=1.0).contains(&state.cloud_water)
        }));
        assert!(fields.cells().iter().any(|state| state.cloud_water > 0.0));
        assert!(
            fields
                .cells()
                .iter()
                .zip(&before)
                .any(|(after, before)| after.cloud_water > before.cloud_water)
        );
        assert!(fields.cells().iter().zip(&before).all(|(after, before)| {
            let before_total = f64::from(before.specific_humidity) + f64::from(before.cloud_water);
            let after_total = f64::from(after.specific_humidity) + f64::from(after.cloud_water);
            (after_total - before_total).abs() < 1.0e-6
        }));
        let bins = fields.overlay_cloud_water_bins(&grid);
        assert!(
            bins.iter()
                .flatten()
                .all(|value| (0.0..=1.0).contains(value))
        );
    }

    #[test]
    fn orographic_lapse_is_deterministic_bounded_and_has_expected_sign() {
        let grid = WeatherGrid::new();
        let mut first = WeatherFields::initial(&grid);
        let mut second = WeatherFields::initial(&grid);
        for fields in [&mut first, &mut second] {
            fields.apply_lapse_rate_and_orographic_uplift(&grid, WEATHER_TIMESTEP_SECONDS);
        }
        assert_eq!(first.cells(), second.cells());
        assert!(first.cells().iter().all(|state| {
            state.surface_elevation_meters.is_finite()
                && (0.0..=4_500.0).contains(&state.surface_elevation_meters)
                && state.orographic_uplift_meters_per_second.is_finite()
                && state.orographic_uplift_meters_per_second.abs()
                    <= WEATHER_MAX_OROGRAPHIC_UPLIFT_METERS_PER_SECOND as f32
                && (180.0..=340.0).contains(&state.temperature_kelvin)
        }));
        assert!(
            first
                .cells()
                .iter()
                .any(|state| state.orographic_uplift_meters_per_second.abs() > 0.01)
        );

        let target_index = cell_index(0, WEATHER_GRID_SIDE / 2, WEATHER_GRID_SIDE / 2);
        let target = grid.cell(target_index as u32);
        let east_index = grid.directional_neighbour_index(target_index, target.east) as usize;
        let west_index = grid.directional_neighbour_index(target_index, -target.east) as usize;
        assert_ne!(east_index, west_index);
        let mut directional = WeatherFields::initial(&grid);
        for state in &mut directional.cells {
            state.surface_elevation_meters = 0.0;
            state.temperature_kelvin = 280.0;
            state.east_wind_meters_per_second = 0.0;
            state.north_wind_meters_per_second = 0.0;
        }
        directional.cells[east_index].surface_elevation_meters = 1_000.0;
        directional.cells[target_index].east_wind_meters_per_second = 20.0;
        directional.apply_lapse_rate_and_orographic_uplift(&grid, WEATHER_TIMESTEP_SECONDS);
        assert!(directional.cells[target_index].orographic_uplift_meters_per_second > 0.0);
        assert!(directional.cells[target_index].temperature_kelvin < 280.0);

        directional.cells[target_index].temperature_kelvin = 280.0;
        directional.cells[target_index].east_wind_meters_per_second = -20.0;
        directional.apply_lapse_rate_and_orographic_uplift(&grid, WEATHER_TIMESTEP_SECONDS);
        assert!(directional.cells[target_index].orographic_uplift_meters_per_second < 0.0);
        assert!(directional.cells[target_index].temperature_kelvin > 280.0);
    }

    #[test]
    fn precipitation_wets_land_and_is_bounded_with_ocean_outlet() {
        let grid = WeatherGrid::new();
        let initial = WeatherFields::initial(&grid);
        let land_index = grid
            .cells()
            .iter()
            .enumerate()
            .find(|(index, _)| {
                1.0 - ocean_fraction_from_albedo(f64::from(initial.cells()[*index].surface_albedo))
                    > 0.9
            })
            .map(|(index, _)| index)
            .expect("proxy should contain land");
        let ocean_index = grid
            .cells()
            .iter()
            .enumerate()
            .find(|(index, _)| {
                ocean_fraction_from_albedo(f64::from(initial.cells()[*index].surface_albedo)) > 0.9
            })
            .map(|(index, _)| index)
            .expect("proxy should contain ocean");
        let mut first = WeatherFields::initial(&grid);
        let mut second = WeatherFields::initial(&grid);
        for fields in [&mut first, &mut second] {
            fields.cells[land_index].cloud_water = 0.9;
            fields.cells[land_index].ground_moisture = 0.0;
            fields.cells[land_index].surface_albedo = WEATHER_LAND_ALBEDO as f32;
            fields.cells[land_index].temperature_kelvin = 280.0;
            fields.cells[ocean_index].cloud_water = 0.9;
            fields.cells[ocean_index].ground_moisture = 0.3;
            fields.cells[ocean_index].surface_albedo = WEATHER_OCEAN_ALBEDO as f32;
            fields.precipitate_and_update_ground_moisture(WEATHER_TIMESTEP_SECONDS);
        }
        assert_eq!(first.cells(), second.cells());
        let land = first.cells()[land_index];
        let ocean = first.cells()[ocean_index];
        assert!(land.cloud_water < 0.9);
        assert!(land.ground_moisture > 0.0);
        assert!(land.precipitation_millimeters_per_hour > 0.0);
        assert!(ocean.cloud_water < 0.9);
        assert!((ocean.ground_moisture - 0.3).abs() < 1.0e-6);
        assert!(first.cells().iter().all(|state| {
            (0.0..=1.0).contains(&state.cloud_water)
                && (0.0..=1.0).contains(&state.ground_moisture)
                && (0.0..=1.0).contains(&state.snow_cover)
                && state.precipitation_millimeters_per_hour.is_finite()
                && state.precipitation_millimeters_per_hour >= 0.0
        }));
        assert!(land.snow_cover <= 1.0);
    }

    #[test]
    fn latent_heat_feedback_warms_condensation_and_bounds_storm_signal() {
        let grid = WeatherGrid::new();
        let target_index = cell_index(0, WEATHER_GRID_SIDE / 2, WEATHER_GRID_SIDE / 2);
        let mut first = WeatherFields::initial(&grid);
        let mut second = WeatherFields::initial(&grid);
        for fields in [&mut first, &mut second] {
            fields.cells[target_index].temperature_kelvin = 250.0;
            fields.cells[target_index].specific_humidity = 0.95;
            fields.cells[target_index].cloud_water = 0.0;
            fields.cells[target_index].orographic_uplift_meters_per_second = 4.0;
            fields.condense_cloud_water(WEATHER_TIMESTEP_SECONDS);
        }
        assert_eq!(first.cells(), second.cells());
        let state = first.cells()[target_index];
        assert!(state.cloud_water > 0.0);
        assert!(state.specific_humidity < 0.95);
        assert!(state.latent_temperature_tendency_kelvin > 0.0);
        assert!(state.temperature_kelvin > 250.0);
        assert!((0.0..=1.0).contains(&state.storm_intensity));
        assert!(first.cells().iter().all(|state| {
            state.latent_temperature_tendency_kelvin.is_finite()
                && state.storm_intensity.is_finite()
                && (0.0..=1.0).contains(&state.storm_intensity)
        }));
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
    fn manual_step_advances_without_consuming_render_clock() {
        let mut state = WeatherState::new();
        state.step_once(DVec3::X);
        let after_one = state.debug_snapshot();
        assert_eq!(after_one.completed_steps, 1);
        assert_eq!(after_one.simulation_time_seconds, WEATHER_TIMESTEP_SECONDS);
        assert_eq!(state.advance_to(0.0), 0);
        state.step_once(DVec3::NEG_X);
        let after_two = state.debug_snapshot();
        assert_eq!(after_two.completed_steps, 2);
        assert_eq!(
            after_two.simulation_time_seconds,
            WEATHER_TIMESTEP_SECONDS * 2.0
        );
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
    fn one_planet_rotation_is_one_weather_day() {
        // A day is a rotation. These were tuned apart once -- weather at 3600x
        // against a 300 real-second rotation -- which made a day 12.5 weather
        // days and left every cell in darkness for over six of them.
        let rotation_real_seconds = crate::planet::PLANET_ROTATION_PERIOD_SECONDS
            / crate::INTERACTIVE_PLANET_ROTATION_TIME_SCALE;
        let weather_seconds = interactive_weather_time_seconds(rotation_real_seconds);
        assert!(
            (weather_seconds - 86_400.0 * WEATHER_DAYS_PER_PLANET_ROTATION).abs() < 1.0e-6,
            "one rotation advances the weather {weather_seconds} s, not a day"
        );
        assert_eq!(interactive_weather_time_seconds(0.0), 0.0);
        // Linear, so the sun and the sim cannot drift apart over a session.
        assert!(
            (interactive_weather_time_seconds(2.0)
                - 2.0 * interactive_weather_time_seconds(1.0))
                .abs()
                < 1.0e-9
        );
    }

    #[test]
    fn a_weather_day_is_a_whole_number_of_steps_and_they_all_run() {
        // 86_400 / 600: the day has to divide into steps exactly, or the sun
        // and the field drift apart by a fraction of a step every rotation.
        let steps_per_day = 86_400.0 / WEATHER_TIMESTEP_SECONDS;
        assert_eq!(steps_per_day, steps_per_day.round());
        let mut state = WeatherState::new();
        let mut completed = 0;
        let day_real_seconds = crate::planet::PLANET_ROTATION_PERIOD_SECONDS
            / crate::INTERACTIVE_PLANET_ROTATION_TIME_SCALE;
        for frame in 1..=120 {
            let presentation_time = day_real_seconds * f64::from(frame) / 120.0;
            completed += state.advance_to_with_sun(
                interactive_weather_time_seconds(presentation_time),
                DVec3::X,
            );
        }
        assert_eq!(completed as f64, steps_per_day);
        assert!(state.interpolation_fraction() <= f32::EPSILON);
    }

    #[test]
    fn interactive_cloud_microphysics_persists_across_fast_transport_steps() {
        // Microphysics ages on its own clock, not the transport one, so cloud
        // phase changes do not race when transport is fast. Asserted as that
        // rate rather than as a step length, which moves with the weather
        // scale: sixty weather-seconds of ageing per real second.
        let microphysics_per_real_second = WEATHER_MICROPHYSICS_TIMESTEP_SECONDS
            / (WEATHER_TIMESTEP_SECONDS / INTERACTIVE_WEATHER_TIME_SCALE);
        assert!(
            (microphysics_per_real_second - INTERACTIVE_CLOUD_MICROPHYSICS_TIME_SCALE).abs()
                < 1.0e-9
        );
        assert_eq!(
            WEATHER_CONDENSATION_TIME_CONSTANT_SECONDS / INTERACTIVE_CLOUD_MICROPHYSICS_TIME_SCALE,
            15.0
        );
        assert_eq!(
            WEATHER_PRECIPITATION_TIME_CONSTANT_SECONDS / INTERACTIVE_CLOUD_MICROPHYSICS_TIME_SCALE,
            60.0
        );

        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        let target = &mut fields.cells[0];
        target.temperature_kelvin = 273.15;
        target.specific_humidity = 0.45;
        target.cloud_water = 0.2;
        // One real second of ageing, expressed as the rate rather than as a
        // count of transport steps: how many of those fit in a second moves
        // with the weather scale, and this is about the microphysics clock.
        fields.condense_cloud_water(INTERACTIVE_CLOUD_MICROPHYSICS_TIME_SCALE);
        assert!(
            fields.cells[0].cloud_water > 0.19,
            "one real second of interactive phase change should retain cloud water"
        );
    }

    #[test]
    fn background_prediction_hands_off_the_exact_synchronous_states() {
        let mut reference = WeatherState::new();
        assert!(reference.prepare_next(DVec3::X));
        let expected_first = reference
            .next_cloud_field_texture_data()
            .expect("synchronous first target");
        assert_eq!(
            reference.advance_to_with_sun(WEATHER_TIMESTEP_SECONDS, DVec3::X),
            1
        );
        let expected_second = reference
            .next_cloud_field_texture_data()
            .expect("synchronous second target");

        let mut state = WeatherState::new();
        state.enable_background_prediction(DVec3::X);
        assert_eq!(
            state.next_cloud_field_texture_data().as_deref(),
            Some(expected_first.as_slice())
        );
        assert_eq!(
            state.advance_interactive_to_with_sun(WEATHER_TIMESTEP_SECONDS * 0.5, DVec3::X),
            0
        );
        assert!((state.interpolation_fraction() - 0.5).abs() <= f32::EPSILON);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut completed =
            state.advance_interactive_to_with_sun(WEATHER_TIMESTEP_SECONDS, DVec3::X);
        while completed == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
            completed = state.advance_interactive_to_with_sun(WEATHER_TIMESTEP_SECONDS, DVec3::X);
        }
        assert_eq!(
            completed, 1,
            "background prediction missed its two-second deadline"
        );
        assert_eq!(state.cloud_field_texture_data(), expected_first);
        assert_eq!(
            state.next_cloud_field_texture_data().as_deref(),
            Some(expected_second.as_slice())
        );
        assert!(state.interpolation_fraction() <= f32::EPSILON);
    }

    #[test]
    fn a_long_frame_cannot_trigger_an_unbounded_weather_catch_up() {
        let mut state = WeatherState::new();
        let completed = state.advance_to_with_sun(WEATHER_TIMESTEP_SECONDS * 100.0, DVec3::X);
        assert_eq!(completed, 12);
        assert!(state.interpolation_fraction() < 1.0);
    }

    #[test]
    fn predicted_field_becomes_the_exact_current_field_at_the_boundary() {
        let mut state = WeatherState::new();
        assert!(state.prepare_next(DVec3::X));
        let predicted = state
            .next_cloud_field_texture_data()
            .expect("prepared weather target");
        assert!(!state.prepare_next(DVec3::NEG_X));

        assert_eq!(state.advance_to_with_sun(300.0, DVec3::X), 0);
        assert!((state.interpolation_fraction() - 0.5).abs() <= f32::EPSILON);
        assert_eq!(state.advance_to_with_sun(600.0, DVec3::X), 1);
        assert_eq!(state.interpolation_fraction(), 0.0);
        assert_eq!(state.cloud_field_texture_data(), predicted);
        assert!(state.next_cloud_field_texture_data().is_some());
    }

    #[test]
    fn manual_step_restarts_continuous_weather_interval_at_the_boundary() {
        let mut state = WeatherState::new();
        state.advance_to_with_sun(300.0, DVec3::X);
        let predicted = state
            .next_cloud_field_texture_data()
            .expect("prepared weather target");

        state.step_once(DVec3::X);

        assert_eq!(state.cloud_field_texture_data(), predicted);
        assert_eq!(state.interpolation_fraction(), 0.0);
        assert_eq!(state.debug_snapshot().completed_steps, 1);
    }

    #[test]
    fn cloud_field_upload_is_deterministic_and_rgba8_bounded() {
        let state = WeatherState::new();
        let first = state.cloud_field_texture_data();
        assert_eq!(
            first.len(),
            WEATHER_FACE_COUNT * WEATHER_GPU_FIELD_SIDE * WEATHER_GPU_FIELD_SIDE * 4
        );
        assert!(first.chunks_exact(4).all(|texel| texel[3] == 0));
        assert_eq!(first, state.cloud_field_texture_data());
        let surface = state.surface_field_texture_data();
        assert!(surface.chunks_exact(4).all(|texel| texel[3] == 255));
        assert!(surface.chunks_exact(4).all(|texel| texel[1] == 0));
    }

    #[test]
    fn cold_precipitation_accumulates_snow_and_warmth_melts_it() {
        let grid = WeatherGrid::new();
        let index = grid
            .cells()
            .iter()
            .enumerate()
            .find(|(index, _)| {
                1.0 - ocean_fraction_from_albedo(f64::from(
                    WeatherFields::initial(&grid).cells()[*index].surface_albedo,
                )) > 0.9
            })
            .map(|(index, _)| index)
            .expect("fallback should contain land");
        let mut fields = WeatherFields::initial(&grid);
        fields.cells[index].surface_albedo = WEATHER_LAND_ALBEDO as f32;
        fields.cells[index].temperature_kelvin = 260.0;
        fields.cells[index].cloud_water = 0.95;
        fields.precipitate_and_update_ground_moisture(WEATHER_TIMESTEP_SECONDS);
        let snow = fields.cells[index].snow_cover;
        assert!(snow > 0.0);
        fields.cells[index].temperature_kelvin = 290.0;
        fields.cells[index].cloud_water = 0.0;
        fields.precipitate_and_update_ground_moisture(WEATHER_TIMESTEP_SECONDS);
        assert!(fields.cells[index].snow_cover < snow);
        assert!(fields.cells[index].ground_moisture > 0.0);
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

#[cfg(test)]
mod rundown_probe {
    use super::*;
    use glam::DVec3;

    fn totals(grid: &WeatherGrid, fields: &WeatherFields) -> (f64, f64, f64) {
        let mut area = 0.0;
        let mut temperature = 0.0;
        let mut moisture = 0.0;
        for (cell, state) in grid.cells().iter().zip(fields.cells()) {
            let a = cell.area_square_meters;
            area += a;
            temperature += f64::from(state.temperature_kelvin) * a;
            moisture += (f64::from(state.specific_humidity) + f64::from(state.cloud_water)) * a;
        }
        (temperature / area, moisture / area, area)
    }

    /// Instrument, not an assertion. Attributes the run-down to a stage.
    /// `cargo test -p catinthegarden-app -- --ignored --nocapture weather_rundown`
    #[test]
    #[ignore = "instrument, not an assertion"]
    fn weather_rundown_by_stage() {
        let grid = WeatherGrid::new();
        let mut fields = WeatherFields::initial(&grid);
        // One rotation of this planet is 12.5 weather-days, so the sun barely
        // moves per step: 600s of weather against a 1_080_000s day.
        let day_seconds = 1_080_000.0;
        let mut names: Vec<&str> = Vec::new();
        let mut d_temp: Vec<f64> = Vec::new();
        let mut d_moist: Vec<f64> = Vec::new();
        let steps = 600;
        for step in 0..steps {
            let angle = std::f64::consts::TAU
                * (step as f64 * WEATHER_TIMESTEP_SECONDS / day_seconds);
            let sun = DVec3::new(angle.cos(), 0.2, angle.sin()).normalize();
            let mut record = |label: &'static str,
                              fields: &WeatherFields,
                              before: (f64, f64, f64),
                              names: &mut Vec<&'static str>,
                              dt: &mut Vec<f64>,
                              dm: &mut Vec<f64>| {
                let after = totals(&grid, fields);
                if let Some(i) = names.iter().position(|n| *n == label) {
                    dt[i] += after.0 - before.0;
                    dm[i] += after.1 - before.1;
                } else {
                    names.push(label);
                    dt.push(after.0 - before.0);
                    dm.push(after.1 - before.1);
                }
                after
            };
            let mut b = totals(&grid, &fields);
            fields.apply_insolation_and_radiative_cooling(&grid, sun, WEATHER_TIMESTEP_SECONDS);
            b = record("radiation", &fields, b, &mut names, &mut d_temp, &mut d_moist);
            fields.diagnose_pressure_from_temperature(&grid);
            fields.update_wind_from_pressure(&grid, WEATHER_TIMESTEP_SECONDS);
            fields.apply_lapse_rate_and_orographic_uplift(&grid, WEATHER_TIMESTEP_SECONDS);
            b = record("lapse/uplift", &fields, b, &mut names, &mut d_temp, &mut d_moist);
            fields.evaporate_moisture(WEATHER_MICROPHYSICS_TIMESTEP_SECONDS);
            b = record("evaporation", &fields, b, &mut names, &mut d_temp, &mut d_moist);
            fields.advect_temperature(&grid, WEATHER_TIMESTEP_SECONDS);
            b = record("advect T", &fields, b, &mut names, &mut d_temp, &mut d_moist);
            fields.advect_humidity(&grid, WEATHER_TIMESTEP_SECONDS);
            b = record("advect q", &fields, b, &mut names, &mut d_temp, &mut d_moist);
            fields.advect_cloud_water(&grid, WEATHER_TIMESTEP_SECONDS);
            b = record("advect cloud", &fields, b, &mut names, &mut d_temp, &mut d_moist);
            fields.condense_cloud_water(WEATHER_MICROPHYSICS_TIMESTEP_SECONDS);
            b = record("condense", &fields, b, &mut names, &mut d_temp, &mut d_moist);
            fields.precipitate_and_update_ground_moisture(WEATHER_MICROPHYSICS_TIMESTEP_SECONDS);
            let _ = record("precipitate", &fields, b, &mut names, &mut d_temp, &mut d_moist);

            if step % 400 == 0 || step == steps - 1 {
                let (t, m, _) = totals(&grid, &fields);
                println!("step {step:5}  mean T {t:7.2} K   total moisture {m:.4}");
            }
        }
        println!("\ncumulative over {steps} steps ({:.1} weather-days):",
            steps as f64 * WEATHER_TIMESTEP_SECONDS / 86400.0);
        println!("  {:<14} {:>12} {:>12}", "stage", "dT (K)", "dq");
        for i in 0..names.len() {
            println!("  {:<14} {:>12.2} {:>12.4}", names[i], d_temp[i], d_moist[i]);
        }
    }
}

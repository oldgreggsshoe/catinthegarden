use serde::{Deserialize, Serialize};

use crate::planet::{MAX_VERTICAL_FOV_DEGREES, MIN_VERTICAL_FOV_DEGREES};

pub const MAX_TERRAIN_LOD_LEVEL: u8 = 18;
const DEFAULT_SUN_DIRECTION: [f64; 3] = [0.4, 0.7, 0.6];

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ScenarioAssertions {
    pub require_finite_metrics: bool,
    pub required_peak_lod_level: Option<u8>,
    pub required_lod_level_sequence: Option<Vec<u8>>,
    pub require_monotonic_lod_progression: bool,
    pub require_unlimited_lod_budget: bool,
    pub min_resident_chunks: Option<u32>,
    pub max_resident_chunks: Option<u32>,
    pub max_lod_thrash_events: Option<u32>,
    pub max_seam_delta_m: Option<f64>,
    pub max_fallback_chunks: Option<u32>,
    pub expected_screenshots: Option<usize>,
    pub sky_sample_uv: Option<[f32; 2]>,
    pub min_sunset_red_blue_growth: Option<f32>,
    pub min_final_sunset_red_blue_ratio: Option<f32>,
    pub max_adjacent_sky_luminance_delta: Option<f32>,
    pub max_sky_luminance: Option<f32>,
    pub day_surface_sample_uv: Option<[f32; 2]>,
    pub night_surface_sample_uv: Option<[f32; 2]>,
    pub min_day_night_surface_luminance_ratio: Option<f32>,
    pub min_exposure: Option<f32>,
    pub max_exposure: Option<f32>,
    pub max_exposure_delta_per_frame: Option<f32>,
    pub max_exposure_oscillation_events: Option<u32>,
    pub min_ocean_wave_height_range_meters: Option<f32>,
}

impl Default for ScenarioAssertions {
    fn default() -> Self {
        Self {
            require_finite_metrics: true,
            required_peak_lod_level: None,
            required_lod_level_sequence: None,
            require_monotonic_lod_progression: false,
            require_unlimited_lod_budget: false,
            min_resident_chunks: None,
            max_resident_chunks: None,
            max_lod_thrash_events: None,
            max_seam_delta_m: None,
            max_fallback_chunks: None,
            expected_screenshots: None,
            sky_sample_uv: None,
            min_sunset_red_blue_growth: None,
            min_final_sunset_red_blue_ratio: None,
            max_adjacent_sky_luminance_delta: None,
            max_sky_luminance: None,
            day_surface_sample_uv: None,
            night_surface_sample_uv: None,
            min_day_night_surface_luminance_ratio: None,
            min_exposure: None,
            max_exposure: None,
            max_exposure_delta_per_frame: None,
            max_exposure_oscillation_events: None,
            min_ocean_wave_height_range_meters: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ScenarioAssertionResult {
    pub name: String,
    pub passed: bool,
    pub details: String,
}

#[derive(Debug, Deserialize)]
pub struct ScenarioDefinition {
    pub name: String,
    pub fixed_timestep_seconds: f64,
    pub duration_seconds: f64,
    pub solid_color_screen: bool,
    #[serde(default)]
    pub hide_overlay: bool,
    #[serde(default)]
    pub seam_gap_check: bool,
    /// Test scenarios default to a static planet so terrain/LOD regressions
    /// remain focused; atmosphere scenarios opt in explicitly.
    #[serde(default)]
    pub planet_rotation_time_scale: f64,
    pub orbit_radius_meters: Option<f64>,
    pub orbit_elevation_degrees: Option<f64>,
    pub orbit_turns: Option<f64>,
    pub screenshot_times_seconds: Vec<f64>,
    pub waypoints: Vec<Waypoint>,
    #[serde(default)]
    pub sun_waypoints: Vec<SunWaypoint>,
    #[serde(default)]
    pub vertical_fov_waypoints: Vec<VerticalFovWaypoint>,
    #[serde(default)]
    pub assertions: ScenarioAssertions,
}

#[derive(Debug, Deserialize)]
pub struct Waypoint {
    pub time_s: f64,
    pub position: [f64; 3],
    pub look_at: [f64; 3],
}

#[derive(Debug, Deserialize)]
pub struct SunWaypoint {
    pub time_s: f64,
    pub direction: [f64; 3],
}

#[derive(Debug, Deserialize)]
pub struct VerticalFovWaypoint {
    pub time_s: f64,
    pub vertical_fov_degrees: f64,
}

pub struct FramePlan {
    pub sim_time: f64,
    pub write_log: bool,
    pub capture_screenshot: bool,
    pub complete: bool,
    pub orbit_azimuth_radians: Option<f64>,
    pub camera_world_position: [f64; 3],
    pub camera_look_at: [f64; 3],
    pub vertical_fov_degrees: Option<f64>,
    pub sun_direction: [f64; 3],
    pub planet_rotation_time_scale: f64,
}

pub struct ScenarioRunner {
    definition: ScenarioDefinition,
    sim_time: f64,
    next_screenshot: usize,
    next_log_time: f64,
}

impl ScenarioRunner {
    pub fn load(name: &str) -> Result<Self, String> {
        let source = match name {
            "still_5s" => include_str!("../scenarios/still_5s.json"),
            "orbit_once" => include_str!("../scenarios/orbit_once.json"),
            "descent_to_10m" => include_str!("../scenarios/descent_to_10m.json"),
            "sunset_sweep" => include_str!("../scenarios/sunset_sweep.json"),
            "night_side_atmosphere" => include_str!("../scenarios/night_side_atmosphere.json"),
            "limb_atmosphere" => include_str!("../scenarios/limb_atmosphere.json"),
            "ground_to_orbit" => include_str!("../scenarios/ground_to_orbit.json"),
            "stare_at_sun" => include_str!("../scenarios/stare_at_sun.json"),
            "ocean_flyover" => include_str!("../scenarios/ocean_flyover.json"),
            "orbital_zoom_lod" => include_str!("../scenarios/orbital_zoom_lod.json"),
            _ => return Err(format!("unknown scenario '{name}'")),
        };
        Self::from_source(source)
    }

    fn from_source(source: &str) -> Result<Self, String> {
        let mut definition: ScenarioDefinition =
            serde_json::from_str(source).map_err(|error| error.to_string())?;
        if !definition.fixed_timestep_seconds.is_finite()
            || definition.fixed_timestep_seconds <= 0.0
            || !definition.duration_seconds.is_finite()
            || definition.duration_seconds <= 0.0
        {
            return Err("scenario timings must be positive".to_owned());
        }
        if definition
            .screenshot_times_seconds
            .iter()
            .any(|time| !time.is_finite() || *time <= 0.0 || *time > definition.duration_seconds)
            || definition
                .screenshot_times_seconds
                .windows(2)
                .any(|times| times[0] >= times[1])
        {
            return Err(
                "screenshot times must be finite, sorted, unique, and within the scenario duration"
                    .to_owned(),
            );
        }
        if definition.waypoints.is_empty()
            || definition.waypoints.iter().any(|waypoint| {
                !waypoint.time_s.is_finite()
                    || waypoint.time_s < 0.0
                    || waypoint.time_s > definition.duration_seconds
                    || waypoint.position.iter().any(|value| !value.is_finite())
                    || waypoint.look_at.iter().any(|value| !value.is_finite())
                    || waypoint.position == waypoint.look_at
            })
            || definition.waypoints[0].time_s != 0.0
            || definition
                .waypoints
                .windows(2)
                .any(|waypoints| waypoints[0].time_s >= waypoints[1].time_s)
        {
            return Err(
                "scenario waypoints must start at zero, be finite, sorted, unique, in range, and look away from the camera"
                    .to_owned(),
            );
        }
        if !definition.planet_rotation_time_scale.is_finite()
            || definition.planet_rotation_time_scale < 0.0
        {
            return Err("planet rotation time scale must be finite and non-negative".to_owned());
        }
        if !definition.sun_waypoints.is_empty()
            && (definition.sun_waypoints.iter().any(|waypoint| {
                !waypoint.time_s.is_finite()
                    || waypoint.time_s < 0.0
                    || waypoint.time_s > definition.duration_seconds
                    || waypoint.direction.iter().any(|value| !value.is_finite())
                    || squared_length(waypoint.direction) <= f64::EPSILON
            }) || definition.sun_waypoints[0].time_s != 0.0
                || definition
                    .sun_waypoints
                    .windows(2)
                    .any(|waypoints| waypoints[0].time_s >= waypoints[1].time_s))
        {
            return Err(
                "sun waypoints must start at zero, have finite non-zero directions, and be sorted within the scenario duration"
                    .to_owned(),
            );
        }
        if !definition.vertical_fov_waypoints.is_empty()
            && (definition.vertical_fov_waypoints.iter().any(|waypoint| {
                !waypoint.time_s.is_finite()
                    || waypoint.time_s < 0.0
                    || waypoint.time_s > definition.duration_seconds
                    || !waypoint.vertical_fov_degrees.is_finite()
                    || !(MIN_VERTICAL_FOV_DEGREES..=MAX_VERTICAL_FOV_DEGREES)
                        .contains(&waypoint.vertical_fov_degrees)
            }) || definition.vertical_fov_waypoints[0].time_s != 0.0
                || definition
                    .vertical_fov_waypoints
                    .windows(2)
                    .any(|waypoints| waypoints[0].time_s >= waypoints[1].time_s))
        {
            return Err(format!(
                "vertical FOV waypoints must start at zero, stay within {MIN_VERTICAL_FOV_DEGREES}..={MAX_VERTICAL_FOV_DEGREES} degrees, and be sorted within the scenario duration"
            ));
        }
        let orbit_fields_present = [
            definition.orbit_radius_meters.is_some(),
            definition.orbit_elevation_degrees.is_some(),
            definition.orbit_turns.is_some(),
        ];
        if orbit_fields_present.iter().any(|present| *present)
            && (!orbit_fields_present.iter().all(|present| *present)
                || definition
                    .orbit_radius_meters
                    .is_some_and(|radius| !radius.is_finite() || radius <= 0.0)
                || definition
                    .orbit_elevation_degrees
                    .is_some_and(|elevation| !elevation.is_finite())
                || definition
                    .orbit_turns
                    .is_some_and(|turns| !turns.is_finite()))
        {
            return Err(
                "orbit scenarios require finite radius, elevation, and turn count".to_owned(),
            );
        }

        validate_assertions(
            &definition.assertions,
            definition.screenshot_times_seconds.len(),
        )?;
        definition
            .assertions
            .expected_screenshots
            .get_or_insert(definition.screenshot_times_seconds.len());

        Ok(Self {
            definition,
            sim_time: 0.0,
            next_screenshot: 0,
            next_log_time: 0.0,
        })
    }

    pub fn name(&self) -> &str {
        &self.definition.name
    }

    pub fn renders_solid_color(&self) -> bool {
        self.definition.solid_color_screen
    }

    pub fn expected_screenshots(&self) -> usize {
        self.definition.screenshot_times_seconds.len()
    }

    pub fn expected_log_samples(&self) -> usize {
        (self.definition.duration_seconds / 0.5).floor() as usize + 1
    }

    pub fn assertions(&self) -> &ScenarioAssertions {
        &self.definition.assertions
    }

    pub fn hides_overlay(&self) -> bool {
        self.definition.hide_overlay
    }

    pub fn needs_seam_gap_check(&self) -> bool {
        self.definition.seam_gap_check
    }

    pub fn orbit_settings(&self) -> Option<(f64, f64)> {
        Some((
            self.definition.orbit_radius_meters?,
            self.definition.orbit_elevation_degrees?.to_radians(),
        ))
    }

    pub fn advance(&mut self) -> FramePlan {
        self.sim_time = (self.sim_time + self.definition.fixed_timestep_seconds)
            .min(self.definition.duration_seconds);

        let write_log = self.sim_time + f64::EPSILON >= self.next_log_time;
        if write_log {
            self.next_log_time += 0.5;
        }

        let capture_screenshot = self
            .definition
            .screenshot_times_seconds
            .get(self.next_screenshot)
            .is_some_and(|time| self.sim_time + f64::EPSILON >= *time);
        if capture_screenshot {
            self.next_screenshot += 1;
        }

        let orbit_azimuth_radians = self.definition.orbit_turns.map(|turns| {
            std::f64::consts::TAU * turns * self.sim_time / self.definition.duration_seconds
        });
        let (mut camera_world_position, camera_look_at) =
            interpolated_waypoint(&self.definition.waypoints, self.sim_time);
        let sun_direction = if self.definition.sun_waypoints.is_empty() {
            normalize_array(DEFAULT_SUN_DIRECTION)
        } else {
            interpolated_sun_direction(&self.definition.sun_waypoints, self.sim_time)
        };
        let vertical_fov_degrees =
            (!self.definition.vertical_fov_waypoints.is_empty()).then(|| {
                interpolated_vertical_fov(&self.definition.vertical_fov_waypoints, self.sim_time)
            });
        if let (Some(radius), Some(elevation), Some(azimuth)) = (
            self.definition.orbit_radius_meters,
            self.definition.orbit_elevation_degrees,
            orbit_azimuth_radians,
        ) {
            let elevation = elevation.to_radians();
            let horizontal_radius = radius * elevation.cos();
            camera_world_position = [
                horizontal_radius * azimuth.cos(),
                radius * elevation.sin(),
                horizontal_radius * azimuth.sin(),
            ];
        }

        FramePlan {
            sim_time: self.sim_time,
            write_log,
            capture_screenshot,
            complete: self.sim_time + f64::EPSILON >= self.definition.duration_seconds
                && self.next_screenshot == self.definition.screenshot_times_seconds.len(),
            orbit_azimuth_radians,
            camera_world_position,
            camera_look_at,
            vertical_fov_degrees,
            sun_direction,
            planet_rotation_time_scale: self.definition.planet_rotation_time_scale,
        }
    }
}

fn validate_assertions(
    assertions: &ScenarioAssertions,
    screenshot_count: usize,
) -> Result<(), String> {
    if assertions
        .required_peak_lod_level
        .is_some_and(|level| level > MAX_TERRAIN_LOD_LEVEL)
    {
        return Err(format!(
            "required peak LOD level cannot exceed {MAX_TERRAIN_LOD_LEVEL}"
        ));
    }
    if assertions
        .required_lod_level_sequence
        .as_ref()
        .is_some_and(|levels| {
            levels.is_empty() || levels.iter().any(|level| *level > MAX_TERRAIN_LOD_LEVEL)
        })
    {
        return Err(format!(
            "required LOD level sequence must be non-empty and cannot exceed {MAX_TERRAIN_LOD_LEVEL}"
        ));
    }
    if matches!(
        (assertions.min_resident_chunks, assertions.max_resident_chunks),
        (Some(minimum), Some(maximum)) if minimum > maximum
    ) {
        return Err("minimum resident chunks cannot exceed the maximum".to_owned());
    }
    if matches!(
        (assertions.min_exposure, assertions.max_exposure),
        (Some(minimum), Some(maximum)) if minimum > maximum
    ) {
        return Err("minimum exposure cannot exceed the maximum".to_owned());
    }
    if assertions
        .max_seam_delta_m
        .is_some_and(|tolerance| !tolerance.is_finite() || tolerance < 0.0)
    {
        return Err("maximum seam delta must be finite and non-negative".to_owned());
    }
    if assertions
        .expected_screenshots
        .is_some_and(|expected| expected != screenshot_count)
    {
        return Err("expected screenshot count must match screenshot times".to_owned());
    }
    let needs_sky_sample = assertions.min_sunset_red_blue_growth.is_some()
        || assertions.min_final_sunset_red_blue_ratio.is_some()
        || assertions.max_adjacent_sky_luminance_delta.is_some()
        || assertions.max_sky_luminance.is_some();
    if needs_sky_sample && assertions.sky_sample_uv.is_none() {
        return Err("sky image assertions require sky_sample_uv".to_owned());
    }
    if assertions.sky_sample_uv.is_some_and(|uv| {
        uv.iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    }) {
        return Err("sky_sample_uv must be finite normalized coordinates".to_owned());
    }
    let needs_surface_samples = assertions.min_day_night_surface_luminance_ratio.is_some();
    if needs_surface_samples
        && (assertions.day_surface_sample_uv.is_none()
            || assertions.night_surface_sample_uv.is_none())
    {
        return Err(
            "day/night surface luminance assertions require both surface sample coordinates"
                .to_owned(),
        );
    }
    for (name, sample) in [
        ("day_surface_sample_uv", assertions.day_surface_sample_uv),
        (
            "night_surface_sample_uv",
            assertions.night_surface_sample_uv,
        ),
    ] {
        if sample.is_some_and(|uv| {
            uv.iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        }) {
            return Err(format!("{name} must be finite normalized coordinates"));
        }
    }
    for (name, value) in [
        (
            "minimum sunset red/blue growth",
            assertions.min_sunset_red_blue_growth,
        ),
        (
            "minimum final sunset red/blue ratio",
            assertions.min_final_sunset_red_blue_ratio,
        ),
        (
            "maximum adjacent sky luminance delta",
            assertions.max_adjacent_sky_luminance_delta,
        ),
        ("maximum sky luminance", assertions.max_sky_luminance),
        (
            "minimum day/night surface luminance ratio",
            assertions.min_day_night_surface_luminance_ratio,
        ),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(format!("{name} must be finite and non-negative"));
        }
    }
    for (name, value) in [
        ("minimum exposure", assertions.min_exposure),
        ("maximum exposure", assertions.max_exposure),
        (
            "maximum exposure delta per frame",
            assertions.max_exposure_delta_per_frame,
        ),
        (
            "minimum ocean wave height range",
            assertions.min_ocean_wave_height_range_meters,
        ),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(format!("{name} must be finite and non-negative"));
        }
    }
    Ok(())
}

fn interpolated_waypoint(waypoints: &[Waypoint], time_s: f64) -> ([f64; 3], [f64; 3]) {
    let first = &waypoints[0];
    if time_s <= first.time_s {
        return (first.position, first.look_at);
    }
    for pair in waypoints.windows(2) {
        let start = &pair[0];
        let end = &pair[1];
        if time_s <= end.time_s {
            let amount = (time_s - start.time_s) / (end.time_s - start.time_s);
            return (
                lerp_array(start.position, end.position, amount),
                lerp_array(start.look_at, end.look_at, amount),
            );
        }
    }
    let last = &waypoints[waypoints.len() - 1];
    (last.position, last.look_at)
}

fn lerp_array(start: [f64; 3], end: [f64; 3], amount: f64) -> [f64; 3] {
    std::array::from_fn(|index| start[index] + (end[index] - start[index]) * amount)
}

fn interpolated_sun_direction(waypoints: &[SunWaypoint], time_s: f64) -> [f64; 3] {
    let first = &waypoints[0];
    if time_s <= first.time_s {
        return normalize_array(first.direction);
    }
    for pair in waypoints.windows(2) {
        let start = &pair[0];
        let end = &pair[1];
        if time_s <= end.time_s {
            return normalize_array(lerp_array(
                start.direction,
                end.direction,
                (time_s - start.time_s) / (end.time_s - start.time_s),
            ));
        }
    }
    normalize_array(waypoints[waypoints.len() - 1].direction)
}

fn interpolated_vertical_fov(waypoints: &[VerticalFovWaypoint], time_s: f64) -> f64 {
    let first = &waypoints[0];
    if time_s <= first.time_s {
        return first.vertical_fov_degrees;
    }
    for pair in waypoints.windows(2) {
        let start = &pair[0];
        let end = &pair[1];
        if time_s <= end.time_s {
            let amount = (time_s - start.time_s) / (end.time_s - start.time_s);
            if amount >= 1.0 {
                return end.vertical_fov_degrees;
            }
            return (start.vertical_fov_degrees.ln()
                + (end.vertical_fov_degrees.ln() - start.vertical_fov_degrees.ln()) * amount)
                .exp();
        }
    }
    waypoints[waypoints.len() - 1].vertical_fov_degrees
}

fn normalize_array(direction: [f64; 3]) -> [f64; 3] {
    let inverse_length = squared_length(direction).sqrt().recip();
    std::array::from_fn(|index| direction[index] * inverse_length)
}

fn squared_length(direction: [f64; 3]) -> f64 {
    direction
        .iter()
        .map(|component| component * component)
        .sum()
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use crate::planet::{OrbitCamera, PlanetLod};

    use super::{
        MAX_TERRAIN_LOD_LEVEL, ScenarioRunner, interpolated_vertical_fov, interpolated_waypoint,
    };

    #[test]
    fn still_scenario_has_three_deterministic_captures() {
        let mut scenario = ScenarioRunner::load("still_5s").expect("scenario parses");
        let mut captures = 0;
        let completion_time = loop {
            let frame = scenario.advance();
            captures += usize::from(frame.capture_screenshot);
            if frame.complete {
                break frame.sim_time;
            }
        };

        assert_eq!(captures, 3);
        assert_eq!(completion_time, 5.0);
    }

    #[test]
    fn orbit_scenario_completes_one_turn_with_four_captures() {
        let mut scenario = ScenarioRunner::load("orbit_once").expect("scenario parses");
        let mut captures = 0;
        let (final_azimuth, final_position) = loop {
            let frame = scenario.advance();
            captures += usize::from(frame.capture_screenshot);
            if frame.complete {
                break (
                    frame.orbit_azimuth_radians.expect("orbit angle"),
                    frame.camera_world_position,
                );
            }
        };

        assert_eq!(captures, 4);
        assert!((final_azimuth - std::f64::consts::TAU).abs() < f64::EPSILON);
        assert!((final_position[0] - 9_396_926.207_859_084).abs() < 0.001);
        assert!((final_position[1] - 3_420_201.433_256_687).abs() < 0.001);
        assert!(final_position[2].abs() < 0.001);
    }

    #[test]
    fn descent_interpolates_sorted_f64_waypoints_and_reaches_ten_meters() {
        let mut scenario = ScenarioRunner::load("descent_to_10m").expect("scenario parses");
        let (position, look_at) = interpolated_waypoint(&scenario.definition.waypoints, 7.0);

        assert_eq!(position, [4_011_000.0, 0.0, 0.0]);
        assert_eq!(look_at, [0.0; 3]);

        let mut frame = scenario.advance();
        while !frame.complete {
            frame = scenario.advance();
        }
        assert_eq!(frame.camera_world_position, [4_000_010.0, 0.0, 0.0]);
        assert_eq!(scenario.expected_screenshots(), 7);
        assert_eq!(
            scenario.assertions().required_peak_lod_level,
            Some(MAX_TERRAIN_LOD_LEVEL)
        );
    }

    #[test]
    fn orbital_zoom_scenario_interpolates_fov_logarithmically_and_returns_wide() {
        let mut scenario = ScenarioRunner::load("orbital_zoom_lod").expect("scenario parses");
        let waypoints = &scenario.definition.vertical_fov_waypoints;
        let midpoint = interpolated_vertical_fov(waypoints, 3.25);
        assert!((midpoint - (75.0_f64 * 0.000_05).sqrt()).abs() < 1.0e-9);

        let mut frame = scenario.advance();
        while !frame.complete {
            frame = scenario.advance();
        }
        assert_eq!(frame.vertical_fov_degrees, Some(75.0));
        assert_eq!(scenario.assertions().required_peak_lod_level, Some(18));
        assert_eq!(
            scenario.assertions().required_lod_level_sequence,
            Some((2_u8..=18).chain((2_u8..18).rev()).collect::<Vec<_>>())
        );
        assert!(scenario.assertions().require_unlimited_lod_budget);
    }

    #[test]
    fn orbital_zoom_scenario_keeps_the_full_ladder_in_a_short_viewport() {
        let viewport_height = 240;
        let mut scenario = ScenarioRunner::load("orbital_zoom_lod").expect("scenario parses");
        let mut camera = OrbitCamera::default();
        let mut lod = PlanetLod::default();
        let mut observed_levels = Vec::new();

        loop {
            let frame = scenario.advance();
            camera.set_world_pose(
                DVec3::from_array(frame.camera_world_position),
                DVec3::from_array(frame.camera_look_at),
            );
            camera.set_reference_vertical_fov_degrees_for_viewport(
                frame.vertical_fov_degrees.expect("zoom scenario FOV"),
                viewport_height,
            );
            let update = lod.update_for_view(
                camera.world_position(),
                camera.direction_dvec3(),
                1.5,
                viewport_height,
                camera.vertical_fov_radians(),
            );
            if observed_levels.last() != Some(&update.metrics.max_level) {
                observed_levels.push(update.metrics.max_level);
            }
            if frame.complete {
                break;
            }
        }

        assert_eq!(
            observed_levels,
            (2_u8..=18).chain((2_u8..18).rev()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn atmosphere_scenarios_have_deterministic_sun_and_ascent_coverage() {
        let mut sunset = ScenarioRunner::load("sunset_sweep").expect("sunset scenario parses");
        let first_sun = sunset.advance().sun_direction;
        let mut last_sun = first_sun;
        while sunset.sim_time < 8.0 {
            let frame = sunset.advance();
            last_sun = frame.sun_direction;
            if frame.complete {
                break;
            }
        }
        assert!(last_sun[0] < first_sun[0]);
        assert_eq!(sunset.expected_screenshots(), 4);
        assert!(
            sunset
                .assertions()
                .min_sunset_red_blue_growth
                .is_some_and(|growth| growth > 1.0)
        );
        assert_eq!(sunset.definition.planet_rotation_time_scale, 1.0);

        let night_side = ScenarioRunner::load("night_side_atmosphere")
            .expect("night-side atmosphere scenario parses");
        assert_eq!(night_side.expected_screenshots(), 1);
        assert_eq!(night_side.assertions().max_sky_luminance, Some(0.02));

        let limb =
            ScenarioRunner::load("limb_atmosphere").expect("limb atmosphere scenario parses");
        assert_eq!(limb.expected_screenshots(), 1);
        assert_eq!(limb.definition.planet_rotation_time_scale, 0.0);

        let ascent = ScenarioRunner::load("ground_to_orbit").expect("ascent scenario parses");
        assert_eq!(ascent.expected_screenshots(), 7);
        assert_eq!(ascent.definition.planet_rotation_time_scale, 1.0);
        assert!(
            ascent
                .assertions()
                .max_adjacent_sky_luminance_delta
                .is_some()
        );
        assert_eq!(ascent.assertions().min_exposure, Some(0.05));

        let stare_at_sun = ScenarioRunner::load("stare_at_sun").expect("sun scenario parses");
        assert_eq!(stare_at_sun.expected_screenshots(), 3);
        assert_eq!(
            stare_at_sun.assertions().max_exposure_delta_per_frame,
            Some(0.5)
        );

        let ocean_flyover = ScenarioRunner::load("ocean_flyover").expect("ocean scenario parses");
        assert_eq!(ocean_flyover.expected_screenshots(), 5);
        assert_eq!(
            ocean_flyover
                .assertions()
                .min_ocean_wave_height_range_meters,
            Some(0.5)
        );
    }

    #[test]
    fn unsorted_waypoints_are_rejected() {
        let source = r#"{
            "name": "bad",
            "fixed_timestep_seconds": 1.0,
            "duration_seconds": 2.0,
            "solid_color_screen": false,
            "screenshot_times_seconds": [],
            "waypoints": [
                {"time_s": 1.0, "position": [2.0, 0.0, 0.0], "look_at": [0.0, 0.0, 0.0]},
                {"time_s": 0.0, "position": [1.0, 0.0, 0.0], "look_at": [0.0, 0.0, 0.0]}
            ]
        }"#;
        let error = ScenarioRunner::from_source(source)
            .err()
            .expect("must fail");
        assert!(error.contains("sorted"));
    }

    #[test]
    fn out_of_camera_range_fov_waypoints_are_rejected() {
        let source = r#"{
            "name": "bad-fov",
            "fixed_timestep_seconds": 1.0,
            "duration_seconds": 2.0,
            "solid_color_screen": false,
            "screenshot_times_seconds": [],
            "waypoints": [
                {"time_s": 0.0, "position": [10000000.0, 0.0, 0.0], "look_at": [0.0, 0.0, 0.0]}
            ],
            "vertical_fov_waypoints": [
                {"time_s": 0.0, "vertical_fov_degrees": 0.000001}
            ]
        }"#;
        let error = ScenarioRunner::from_source(source)
            .err()
            .expect("must fail");
        assert!(error.contains("0.00005..=75"));
    }
}

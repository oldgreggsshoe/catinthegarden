use serde::{Deserialize, Serialize};

pub const MAX_TERRAIN_LOD_LEVEL: u8 = 18;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ScenarioAssertions {
    pub require_finite_metrics: bool,
    pub required_peak_lod_level: Option<u8>,
    pub require_monotonic_lod_progression: bool,
    pub min_resident_chunks: Option<u32>,
    pub max_resident_chunks: Option<u32>,
    pub max_lod_thrash_events: Option<u32>,
    pub max_seam_delta_m: Option<f64>,
    pub max_fallback_chunks: Option<u32>,
    pub expected_screenshots: Option<usize>,
}

impl Default for ScenarioAssertions {
    fn default() -> Self {
        Self {
            require_finite_metrics: true,
            required_peak_lod_level: None,
            require_monotonic_lod_progression: false,
            min_resident_chunks: None,
            max_resident_chunks: None,
            max_lod_thrash_events: None,
            max_seam_delta_m: None,
            max_fallback_chunks: None,
            expected_screenshots: None,
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
    pub orbit_radius_meters: Option<f64>,
    pub orbit_elevation_degrees: Option<f64>,
    pub orbit_turns: Option<f64>,
    pub screenshot_times_seconds: Vec<f64>,
    pub waypoints: Vec<Waypoint>,
    #[serde(default)]
    pub assertions: ScenarioAssertions,
}

#[derive(Debug, Deserialize)]
pub struct Waypoint {
    pub time_s: f64,
    pub position: [f64; 3],
    pub look_at: [f64; 3],
}

pub struct FramePlan {
    pub sim_time: f64,
    pub write_log: bool,
    pub capture_screenshot: bool,
    pub complete: bool,
    pub orbit_azimuth_radians: Option<f64>,
    pub camera_world_position: [f64; 3],
    pub camera_look_at: [f64; 3],
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
    if matches!(
        (assertions.min_resident_chunks, assertions.max_resident_chunks),
        (Some(minimum), Some(maximum)) if minimum > maximum
    ) {
        return Err("minimum resident chunks cannot exceed the maximum".to_owned());
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

#[cfg(test)]
mod tests {
    use super::{MAX_TERRAIN_LOD_LEVEL, ScenarioRunner, interpolated_waypoint};

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
}

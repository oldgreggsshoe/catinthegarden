use serde::{Deserialize, Serialize};

use crate::planet::{MAX_VERTICAL_FOV_DEGREES, MIN_VERTICAL_FOV_DEGREES};

pub const MAX_TERRAIN_LOD_LEVEL: u8 = 18;

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
    pub max_sky_green_dominance: Option<f32>,
    pub min_blue_hour_blue_red_ratio: Option<f32>,
    pub min_blue_hour_luminance: Option<f32>,
    pub max_final_blue_hour_luminance_ratio: Option<f32>,
    pub min_solar_antisolar_sky_luminance_ratio: Option<f32>,
    pub max_adjacent_sky_luminance_delta: Option<f32>,
    pub max_adjacent_sky_luminance_increase: Option<f32>,
    pub max_sky_luminance: Option<f32>,
    pub sun_background_sample_uv: Option<[f32; 2]>,
    pub min_visible_sun_contrast: Option<f32>,
    pub max_occluded_sun_contrast: Option<f32>,
    pub day_surface_sample_uv: Option<[f32; 2]>,
    pub night_surface_sample_uv: Option<[f32; 2]>,
    pub min_day_night_surface_luminance_ratio: Option<f32>,
    pub min_exposure: Option<f32>,
    pub max_exposure: Option<f32>,
    pub max_exposure_delta_per_frame: Option<f32>,
    pub max_exposure_oscillation_events: Option<u32>,
    pub min_ocean_wave_height_range_meters: Option<f32>,
    pub ice_sample_uv: Option<[f32; 2]>,
    pub min_ice_sample_luminance: Option<f32>,
    pub max_ice_sample_channel_spread: Option<f32>,
    /// Largest tolerated gap between the surface the renderer drew and the
    /// surface the CPU would collide with, over the probe's sample grid. This
    /// is an outlier guard and should be set loosely; horizon-grazing samples
    /// dominate it. `max_surface_probe_p90_delta_m` is the one that means
    /// something.
    pub max_surface_probe_delta_m: Option<f64>,
    pub max_surface_probe_p90_delta_m: Option<f64>,
    /// Bounds on how far the camera sits above the CPU's terrain. Setting both
    /// is what pins "standing on the ground" rather than sunk or floating.
    pub min_camera_clearance_m: Option<f64>,
    pub max_camera_clearance_m: Option<f64>,
    /// Floor on how many probe points were actually compared. Without it a
    /// scenario that happened to see only sky would pass the delta assertion
    /// on no evidence at all.
    pub min_surface_probe_points: Option<usize>,
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
            max_sky_green_dominance: None,
            min_blue_hour_blue_red_ratio: None,
            min_blue_hour_luminance: None,
            max_final_blue_hour_luminance_ratio: None,
            min_solar_antisolar_sky_luminance_ratio: None,
            max_adjacent_sky_luminance_delta: None,
            max_adjacent_sky_luminance_increase: None,
            max_sky_luminance: None,
            sun_background_sample_uv: None,
            min_visible_sun_contrast: None,
            max_occluded_sun_contrast: None,
            day_surface_sample_uv: None,
            night_surface_sample_uv: None,
            min_day_night_surface_luminance_ratio: None,
            min_exposure: None,
            max_exposure: None,
            max_exposure_delta_per_frame: None,
            max_exposure_oscillation_events: None,
            min_ocean_wave_height_range_meters: None,
            ice_sample_uv: None,
            min_ice_sample_luminance: None,
            max_ice_sample_channel_spread: None,
            max_surface_probe_delta_m: None,
            max_surface_probe_p90_delta_m: None,
            min_camera_clearance_m: None,
            max_camera_clearance_m: None,
            min_surface_probe_points: None,
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
    /// Keep the camera roll aligned with the planet surface instead of the
    /// global Y axis. Low-flight parity captures need the same local horizon
    /// frame as the interactive F4 camera they were recorded from.
    #[serde(default)]
    pub planet_relative_up: bool,
    /// Per-scenario override for the depth probe's comparison range. Ordinary
    /// ground-contact checks intentionally stay near the camera; path-parity
    /// diagnosis opts into the longer range it is trying to compare.
    #[serde(default)]
    pub surface_probe_max_distance_meters: Option<f64>,
    /// Test scenarios default to a static planet so terrain/LOD regressions
    /// remain focused; atmosphere scenarios opt in explicitly.
    #[serde(default)]
    pub planet_rotation_time_scale: f64,
    /// Hold presented exposure at 1.0 while the meter continues adapting.
    /// Colour-order scenarios use this to test scene radiance rather than the
    /// exposure response to a frame becoming dark.
    #[serde(default)]
    pub fixed_exposure: bool,
    /// Enables the real low-flight camera at the first waypoint and holds W
    /// from this simulation time onward. This is deliberately not waypoint
    /// interpolation: terrain-follow regressions must exercise the same
    /// acceleration, geodesic movement, and clearance path as keyboard input.
    #[serde(default)]
    pub forward_flight_start_time_seconds: Option<f64>,
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
    #[allow(dead_code)]
    pub orbit_azimuth_radians: Option<f64>,
    pub camera_world_position: [f64; 3],
    pub camera_look_at: [f64; 3],
    pub vertical_fov_degrees: Option<f64>,
    pub sun_direction: [f64; 3],
    pub planet_rotation_time_scale: f64,
    pub forward_flight_held: Option<bool>,
}

pub struct ScenarioRunner {
    definition: ScenarioDefinition,
    sim_time: f64,
    next_screenshot: usize,
    next_log_time: f64,
}

#[allow(dead_code)]
impl ScenarioRunner {
    pub fn load(name: &str) -> Result<Self, String> {
        let source = match name {
            "still_5s" => include_str!("../scenarios/still_5s.json"),
            "orbit_once" => include_str!("../scenarios/orbit_once.json"),
            "descent_to_10m" => include_str!("../scenarios/descent_to_10m.json"),
            "sunset_sweep" => include_str!("../scenarios/sunset_sweep.json"),
            "sunset_blue_hour" => include_str!("../scenarios/sunset_blue_hour.json"),
            "sunrise_midday_surface" => {
                include_str!("../scenarios/sunrise_midday_surface.json")
            }
            "twilight_directionality" => {
                include_str!("../scenarios/twilight_directionality.json")
            }
            "night_side_atmosphere" => include_str!("../scenarios/night_side_atmosphere.json"),
            "limb_atmosphere" => include_str!("../scenarios/limb_atmosphere.json"),
            "orbital_atmosphere_profile" => {
                include_str!("../scenarios/orbital_atmosphere_profile.json")
            }
            "orbital_atmosphere_continuity" => {
                include_str!("../scenarios/orbital_atmosphere_continuity.json")
            }
            "atmospheric_mist_paths" => {
                include_str!("../scenarios/atmospheric_mist_paths.json")
            }
            "ground_to_orbit" => include_str!("../scenarios/ground_to_orbit.json"),
            "stare_at_sun" => include_str!("../scenarios/stare_at_sun.json"),
            "orbital_sun_visibility" => {
                include_str!("../scenarios/orbital_sun_visibility.json")
            }
            "weather_contrast" => include_str!("../scenarios/weather_contrast.json"),
            "weather_sun_occlusion" => {
                include_str!("../scenarios/weather_sun_occlusion.json")
            }
            "sun_horizon_visibility" => {
                include_str!("../scenarios/sun_horizon_visibility.json")
            }
            "partial_sun_occultation" => {
                include_str!("../scenarios/partial_sun_occultation.json")
            }
            "ocean_flyover" => include_str!("../scenarios/ocean_flyover.json"),
            "ocean_coastline" => include_str!("../scenarios/ocean_coastline.json"),
            "orbital_zoom_lod" => include_str!("../scenarios/orbital_zoom_lod.json"),
            "polar_ice_cap" => include_str!("../scenarios/polar_ice_cap.json"),
            "terrain_material_preview" => {
                include_str!("../scenarios/terrain_material_preview.json")
            }
            "low_flight_performance" => {
                include_str!("../scenarios/low_flight_performance.json")
            }
            "landing_site_ground_detail" => {
                include_str!("../scenarios/landing_site_ground_detail.json")
            }
            "landing_site_eye_level" => {
                include_str!("../scenarios/landing_site_eye_level.json")
            }
            "forest_startup" => include_str!("../scenarios/forest_startup.json"),
            "forest_performance" => include_str!("../scenarios/forest_performance.json"),
            "forest_vast_distance" => {
                include_str!("../scenarios/forest_vast_distance.json")
            }
            "forest_night" => include_str!("../scenarios/forest_night.json"),
            "forest_boundary_transition" => {
                include_str!("../scenarios/forest_boundary_transition.json")
            }
            "forest_travel" => include_str!("../scenarios/forest_travel.json"),
            "highest_prominence_peak" => {
                include_str!("../scenarios/highest_prominence_peak.json")
            }
            "manual_forward_clearance" => {
                include_str!("../scenarios/manual_forward_clearance.json")
            }
            "manual_high_speed_clearance" => {
                include_str!("../scenarios/manual_high_speed_clearance.json")
            }
            "manual_near_terrain_culling" => {
                include_str!("../scenarios/manual_near_terrain_culling.json")
            }
            "manual_lod_approach_replay" => {
                include_str!("../scenarios/manual_lod_approach_replay.json")
            }
            "manual_sky_ocean_replay" => {
                include_str!("../scenarios/manual_sky_ocean_replay.json")
            }
            "outlined_shadows" => include_str!("../scenarios/outlined_shadows.json"),
            "stand_on_ground" => include_str!("../scenarios/stand_on_ground.json"),
            "terrain_detail_altitude_ladder" => {
                include_str!("../scenarios/terrain_detail_altitude_ladder.json")
            }
            "path_parity_ridge" => include_str!("../scenarios/path_parity_ridge.json"),
            "render_path_parity" => include_str!("../scenarios/render_path_parity.json"),
            "manual_render_faults" => include_str!("../scenarios/manual_render_faults.json"),
            "mountain_render_faults" => include_str!("../scenarios/mountain_render_faults.json"),
            "low_pass_bands" => include_str!("../scenarios/low_pass_bands.json"),
            "tour_mountains" => include_str!("../scenarios/tour_mountains.json"),
            "tour_desert" => include_str!("../scenarios/tour_desert.json"),
            "tour_coast" => include_str!("../scenarios/tour_coast.json"),
            "tour_grassland" => include_str!("../scenarios/tour_grassland.json"),
            "tour_tundra" => include_str!("../scenarios/tour_tundra.json"),
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
            .surface_probe_max_distance_meters
            .is_some_and(|distance| !distance.is_finite() || distance <= 0.0)
        {
            return Err("surface probe maximum distance must be finite and positive".to_owned());
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
        if definition
            .forward_flight_start_time_seconds
            .is_some_and(|time| {
                !time.is_finite() || time < 0.0 || time > definition.duration_seconds
            })
        {
            return Err(
                "forward flight start time must be finite and within the scenario duration"
                    .to_owned(),
            );
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
        let cadence_samples = (self.definition.duration_seconds / 0.5).floor() as usize + 1;
        let rendered_frames = (self.definition.duration_seconds
            / self.definition.fixed_timestep_seconds)
            .ceil() as usize;
        cadence_samples.min(rendered_frames)
    }

    pub fn assertions(&self) -> &ScenarioAssertions {
        &self.definition.assertions
    }

    /// Keep the LOD zoom regression centred on the same sparse tile chain
    /// selected by the baker. Other scenarios retain their authored poses.
    pub fn retarget_sparse_landing_direction(&mut self, landing_direction: glam::DVec3) {
        if !matches!(
            self.definition.name.as_str(),
            "orbital_zoom_lod"
                | "low_flight_performance"
                | "landing_site_ground_detail"
                | "landing_site_eye_level"
                | "stand_on_ground"
                | "terrain_detail_altitude_ladder"
        ) {
            return;
        }
        let Some(landing_direction) = landing_direction.try_normalize() else {
            return;
        };
        let rotation = glam::DQuat::from_rotation_arc(glam::DVec3::X, landing_direction);
        for waypoint in &mut self.definition.waypoints {
            waypoint.position = rotation
                .mul_vec3(glam::DVec3::from_array(waypoint.position))
                .to_array();
            waypoint.look_at = rotation
                .mul_vec3(glam::DVec3::from_array(waypoint.look_at))
                .to_array();
        }
        // These scenarios author their sun in the same landing-relative frame
        // as their camera, so the light has to rotate with the pose or the
        // grazing angle chosen to reveal relief is lost.
        if matches!(
            self.definition.name.as_str(),
            "low_flight_performance"
                | "landing_site_ground_detail"
                | "landing_site_eye_level"
                | "stand_on_ground"
        ) {
            for waypoint in &mut self.definition.sun_waypoints {
                waypoint.direction = rotation
                    .mul_vec3(glam::DVec3::from_array(waypoint.direction))
                    .to_array();
            }
        }
    }

    pub fn hides_overlay(&self) -> bool {
        self.definition.hide_overlay
    }

    pub fn needs_seam_gap_check(&self) -> bool {
        self.definition.seam_gap_check
    }

    pub fn uses_planet_relative_up(&self) -> bool {
        self.definition.planet_relative_up
            || matches!(
                self.definition.name.as_str(),
                "low_flight_performance"
                    | "landing_site_ground_detail"
                    | "landing_site_eye_level"
                    | "terrain_detail_altitude_ladder"
            )
    }

    pub fn surface_probe_max_distance_meters(&self) -> f64 {
        self.definition
            .surface_probe_max_distance_meters
            .unwrap_or(crate::probe::MAX_COMPARISON_DISTANCE_METERS)
    }

    pub fn replays_forward_flight(&self) -> bool {
        self.definition.forward_flight_start_time_seconds.is_some()
    }

    pub fn uses_fixed_exposure(&self) -> bool {
        self.definition.fixed_exposure
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
            crate::planet::default_sun_direction().to_array()
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
            forward_flight_held: self
                .definition
                .forward_flight_start_time_seconds
                .map(|start| self.sim_time + f64::EPSILON >= start),
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
        .max_surface_probe_delta_m
        .is_some_and(|tolerance| !tolerance.is_finite() || tolerance < 0.0)
    {
        return Err("maximum surface probe delta must be finite and non-negative".to_owned());
    }
    if matches!(
        (
            assertions.min_camera_clearance_m,
            assertions.max_camera_clearance_m
        ),
        (Some(minimum), Some(maximum)) if minimum > maximum
    ) {
        return Err("minimum camera clearance cannot exceed the maximum".to_owned());
    }
    for (name, value) in [
        (
            "minimum camera clearance",
            assertions.min_camera_clearance_m,
        ),
        (
            "maximum camera clearance",
            assertions.max_camera_clearance_m,
        ),
    ] {
        if value.is_some_and(|value| !value.is_finite()) {
            return Err(format!("{name} must be finite"));
        }
    }
    // A delta tolerance with no evidence requirement is the failure mode this
    // whole probe exists to avoid, so the two are wired together here.
    if assertions.max_surface_probe_delta_m.is_some()
        && assertions.min_surface_probe_points.is_none()
    {
        return Err(
            "a surface probe delta tolerance requires min_surface_probe_points as well".to_owned(),
        );
    }
    if assertions
        .expected_screenshots
        .is_some_and(|expected| expected != screenshot_count)
    {
        return Err("expected screenshot count must match screenshot times".to_owned());
    }
    let needs_sky_sample = assertions.min_sunset_red_blue_growth.is_some()
        || assertions.min_final_sunset_red_blue_ratio.is_some()
        || assertions.max_sky_green_dominance.is_some()
        || assertions.min_blue_hour_blue_red_ratio.is_some()
        || assertions.min_blue_hour_luminance.is_some()
        || assertions.max_final_blue_hour_luminance_ratio.is_some()
        || assertions.min_solar_antisolar_sky_luminance_ratio.is_some()
        || assertions.max_adjacent_sky_luminance_delta.is_some()
        || assertions.max_adjacent_sky_luminance_increase.is_some()
        || assertions.max_sky_luminance.is_some()
        || assertions.min_visible_sun_contrast.is_some()
        || assertions.max_occluded_sun_contrast.is_some();
    if needs_sky_sample && assertions.sky_sample_uv.is_none() {
        return Err("sky image assertions require sky_sample_uv".to_owned());
    }
    if assertions.sky_sample_uv.is_some_and(|uv| {
        uv.iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    }) {
        return Err("sky_sample_uv must be finite normalized coordinates".to_owned());
    }
    if (assertions.min_visible_sun_contrast.is_some()
        || assertions.max_occluded_sun_contrast.is_some())
        && assertions.sun_background_sample_uv.is_none()
    {
        return Err("sun contrast assertions require sun_background_sample_uv".to_owned());
    }
    if assertions.sun_background_sample_uv.is_some_and(|uv| {
        uv.iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    }) {
        return Err("sun_background_sample_uv must be finite normalized coordinates".to_owned());
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
            "maximum sky green dominance",
            assertions.max_sky_green_dominance,
        ),
        (
            "minimum blue-hour blue/red ratio",
            assertions.min_blue_hour_blue_red_ratio,
        ),
        (
            "minimum blue-hour luminance",
            assertions.min_blue_hour_luminance,
        ),
        (
            "maximum final/peak blue-hour luminance ratio",
            assertions.max_final_blue_hour_luminance_ratio,
        ),
        (
            "minimum solar/anti-solar sky luminance ratio",
            assertions.min_solar_antisolar_sky_luminance_ratio,
        ),
        (
            "maximum adjacent sky luminance delta",
            assertions.max_adjacent_sky_luminance_delta,
        ),
        (
            "maximum adjacent sky luminance increase",
            assertions.max_adjacent_sky_luminance_increase,
        ),
        ("maximum sky luminance", assertions.max_sky_luminance),
        (
            "minimum visible sun contrast",
            assertions.min_visible_sun_contrast,
        ),
        (
            "maximum occluded sun contrast",
            assertions.max_occluded_sun_contrast,
        ),
        (
            "minimum day/night surface luminance ratio",
            assertions.min_day_night_surface_luminance_ratio,
        ),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(format!("{name} must be finite and non-negative"));
        }
    }
    if (assertions.min_blue_hour_luminance.is_some()
        || assertions.max_final_blue_hour_luminance_ratio.is_some())
        && assertions.min_blue_hour_blue_red_ratio.is_none()
    {
        return Err(
            "blue-hour luminance assertions require min_blue_hour_blue_red_ratio".to_owned(),
        );
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
    fn forest_startup_holds_the_authored_interactive_view() {
        let scenario = ScenarioRunner::load("forest_startup").expect("forest scenario parses");
        assert_eq!(scenario.expected_screenshots(), 2);
        assert!(scenario.definition.hide_overlay);
        assert!(scenario.definition.planet_relative_up);
    }

    #[test]
    fn forest_performance_is_a_capture_free_fixed_pose() {
        let scenario = ScenarioRunner::load("forest_performance").expect("forest scenario parses");
        assert_eq!(scenario.expected_screenshots(), 0);
        assert!(scenario.definition.hide_overlay);
        assert_eq!(scenario.definition.duration_seconds, 8.0);
    }

    #[test]
    fn forest_vast_distance_has_no_individual_tree_geometry() {
        let scenario =
            ScenarioRunner::load("forest_vast_distance").expect("forest scenario parses");
        assert_eq!(scenario.expected_screenshots(), 1);
        assert!(scenario.definition.hide_overlay);
        assert!(
            DVec3::from_array(scenario.definition.waypoints[0].position).length()
                - crate::planet::PLANET_RADIUS_METERS
                > 50_000.0
        );
    }

    #[test]
    fn forest_night_holds_fixed_exposure_for_the_lighting_regression() {
        let scenario = ScenarioRunner::load("forest_night").expect("forest scenario parses");
        assert_eq!(scenario.expected_screenshots(), 1);
        assert!(scenario.uses_fixed_exposure());
    }

    #[test]
    fn forest_boundary_transition_replays_the_seven_metre_pop_regression() {
        let scenario =
            ScenarioRunner::load("forest_boundary_transition").expect("forest scenario parses");
        assert_eq!(scenario.expected_screenshots(), 4);
        assert!(scenario.definition.hide_overlay);
        assert!(scenario.definition.planet_relative_up);
    }

    #[test]
    fn forest_travel_exercises_the_real_low_flight_camera_across_patch_cells() {
        let scenario = ScenarioRunner::load("forest_travel").expect("forest scenario parses");
        assert_eq!(scenario.expected_screenshots(), 6);
        assert_eq!(
            scenario.definition.forward_flight_start_time_seconds,
            Some(0.5)
        );
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
    fn highest_prominence_scenario_replays_the_f4_start_pose() {
        let scenario =
            ScenarioRunner::load("highest_prominence_peak").expect("peak scenario parses");
        let waypoint = &scenario.definition.waypoints[0];
        let position = glam::DVec3::from_array(waypoint.position);
        let direction = position.normalize();

        assert!((direction.y.asin().to_degrees() - (-20.349_651_274_351)).abs() < 1.0e-6);
        assert!(
            (crate::planet::geographic_longitude_degrees(direction) + 51.995_567_522_201).abs()
                < 1.0e-6
        );
        assert!((position.length() - 4_181_087.114_995_877).abs() < 1.0e-6);
        assert_eq!(scenario.expected_screenshots(), 2);
        assert_eq!(scenario.assertions().min_camera_clearance_m, Some(150.0));
        assert_eq!(scenario.assertions().max_camera_clearance_m, Some(155.0));
    }

    #[test]
    fn manual_forward_scenario_replays_the_captured_pose_and_holds_w() {
        let mut scenario =
            ScenarioRunner::load("manual_forward_clearance").expect("manual replay parses");
        assert!(scenario.replays_forward_flight());
        assert_eq!(scenario.expected_screenshots(), 7);
        assert_eq!(scenario.assertions().min_camera_clearance_m, Some(10.0));
        assert_eq!(scenario.assertions().max_camera_clearance_m, Some(50.0));
        assert_eq!(scenario.assertions().max_surface_probe_p90_delta_m, None);

        let waypoint = &scenario.definition.waypoints[0];
        assert!(
            (DVec3::from_array(waypoint.position)
                - DVec3::new(
                    963_666.587_339_783_7,
                    2_669_549.155_721_813_4,
                    2_856_170.063_578_058,
                ))
            .length()
                < 1.0e-9
        );

        let mut frame = scenario.advance();
        while frame.sim_time + f64::EPSILON < 3.0 {
            assert_eq!(frame.forward_flight_held, Some(false));
            frame = scenario.advance();
        }
        assert!((3.0..=3.0 + 1.0 / 60.0).contains(&frame.sim_time));
        assert_eq!(frame.forward_flight_held, Some(true));
    }

    #[test]
    fn manual_near_terrain_culling_sweeps_the_peak_foreground() {
        let scenario = ScenarioRunner::load("manual_near_terrain_culling")
            .expect("near-terrain replay parses");

        assert_eq!(scenario.expected_screenshots(), 9);
        assert_eq!(scenario.assertions().min_camera_clearance_m, Some(150.0));
        assert_eq!(scenario.assertions().max_camera_clearance_m, Some(155.0));
    }

    #[test]
    fn manual_lod_approach_replays_the_reported_outline_off_descent() {
        let scenario = ScenarioRunner::load("manual_lod_approach_replay")
            .expect("manual LOD approach replay parses");

        assert_eq!(scenario.expected_screenshots(), 12);
        assert_eq!(scenario.definition.waypoints.len(), 24);
        assert_eq!(scenario.assertions().min_surface_probe_points, Some(100));
        assert_eq!(
            scenario.assertions().max_surface_probe_p90_delta_m,
            Some(40.0)
        );
    }

    #[test]
    fn manual_sky_ocean_replay_preserves_the_logged_frozen_sun_flight() {
        let mut scenario = ScenarioRunner::load("manual_sky_ocean_replay")
            .expect("manual sky/ocean replay parses");
        let first = scenario.advance();

        assert_eq!(scenario.expected_screenshots(), 9);
        assert!(scenario.uses_planet_relative_up());
        assert!(
            (DVec3::from_array(first.sun_direction)
                - DVec3::new(
                    0.508_927_518_800_963_9,
                    0.397_776_994_021_848,
                    0.763_391_278_201_445_7,
                ))
            .length()
                < 1.0e-12
        );
        assert_eq!(first.planet_rotation_time_scale, 0.0);
        assert_eq!(first.vertical_fov_degrees, Some(75.0));
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
        assert!(!scenario.assertions().require_unlimited_lod_budget);
        assert_eq!(scenario.assertions().max_fallback_chunks, Some(256));
    }

    #[test]
    fn orbital_zoom_scenario_retargets_to_the_baked_sparse_direction() {
        let mut scenario = ScenarioRunner::load("orbital_zoom_lod").expect("scenario parses");
        let original_radius = DVec3::from_array(scenario.definition.waypoints[0].position).length();

        scenario.retarget_sparse_landing_direction(DVec3::Z);
        let frame = scenario.advance();
        let position = DVec3::from_array(frame.camera_world_position);
        let look_at = DVec3::from_array(frame.camera_look_at);

        assert!((position.length() - original_radius).abs() < 1.0e-6);
        assert!((look_at.normalize().dot(DVec3::Z) - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn low_flight_performance_retargets_camera_and_daylight_to_the_sparse_site() {
        let mut scenario = ScenarioRunner::load("low_flight_performance").expect("scenario parses");
        scenario.retarget_sparse_landing_direction(DVec3::Z);
        let frame = scenario.advance();
        let position = DVec3::from_array(frame.camera_world_position).normalize();
        let sun = DVec3::from_array(frame.sun_direction).normalize();

        assert!(position.dot(DVec3::Z) > 1.0 - 1.0e-12);
        assert!(sun.dot(DVec3::Z) > 0.7);
        assert_eq!(scenario.expected_screenshots(), 2);
        assert!(scenario.assertions().require_unlimited_lod_budget);
    }

    #[test]
    fn render_path_parity_replays_the_logged_altitude_ladder() {
        let mut scenario = ScenarioRunner::load("render_path_parity").expect("scenario parses");
        let mut capture_altitudes = Vec::new();
        loop {
            let frame = scenario.advance();
            if frame.capture_screenshot {
                capture_altitudes.push(
                    DVec3::from_array(frame.camera_world_position).length()
                        - crate::planet::PLANET_RADIUS_METERS,
                );
            }
            if frame.complete {
                break;
            }
        }

        assert_eq!(scenario.expected_screenshots(), 4);
        assert!(scenario.uses_planet_relative_up());
        assert_eq!(scenario.surface_probe_max_distance_meters(), 200_000.0);
        for (observed, expected) in capture_altitudes
            .iter()
            .zip([70_792.6, 29_873.5, 13_993.6, 737.9])
        {
            assert!(
                (observed - expected).abs() < 0.1,
                "expected {expected}m, observed {observed}m"
            );
        }
    }

    #[test]
    fn manual_render_faults_replays_the_two_logged_camera_poses() {
        let mut scenario = ScenarioRunner::load("manual_render_faults").expect("scenario parses");
        let mut captures = Vec::new();
        loop {
            let frame = scenario.advance();
            if frame.capture_screenshot {
                let position = DVec3::from_array(frame.camera_world_position);
                captures.push((
                    position,
                    (DVec3::from_array(frame.camera_look_at) - position).normalize(),
                ));
            }
            if frame.complete {
                break;
            }
        }

        assert_eq!(captures.len(), 2);
        assert!(scenario.uses_planet_relative_up());
        for ((position, direction), (expected_position, expected_direction)) in
            captures.iter().zip([
                (
                    DVec3::new(
                        -3_960_082.052234005,
                        313_838.9311170833,
                        -495_914.6300803465,
                    ),
                    DVec3::new(
                        -0.06730167798646366,
                        -0.6710929528113925,
                        0.7383120836252732,
                    ),
                ),
                (
                    DVec3::new(
                        -3_959_697.9372234177,
                        314_983.9975962875,
                        -495_015.9466270908,
                    ),
                    DVec3::new(0.3953746540707302, 0.8970116612273945, 0.19760810342827706),
                ),
            ])
        {
            assert!(position.distance(expected_position) < 1.0e-6);
            assert!(direction.distance(expected_direction) < 1.0e-12);
        }
    }

    #[test]
    fn eye_level_scenario_descends_toward_the_landing_site_without_burying_the_camera() {
        let mut scenario = ScenarioRunner::load("landing_site_eye_level").expect("scenario parses");
        scenario.retarget_sparse_landing_direction(DVec3::Z);

        let mut frame = scenario.advance();
        let start_radius = DVec3::from_array(frame.camera_world_position).length();
        let mut end_radius = start_radius;
        while !frame.complete {
            frame = scenario.advance();
            end_radius = DVec3::from_array(frame.camera_world_position).length();
        }

        // What this can check without a GPU is that the descent actually
        // arrives where it was authored to. Whether that endpoint is *above
        // the ground* is a different question, it depends on the detail
        // ladder, and pinning a literal for it here made this test go stale
        // every time the ladder changed. The surface probe answers it directly
        // now: `min_camera_clearance_m` on this scenario holds the camera
        // above the drawn ground in both render paths.
        assert!(start_radius > end_radius);
        let authored: serde_json::Value =
            serde_json::from_str(include_str!("../scenarios/landing_site_eye_level.json"))
                .expect("scenario parses");
        let final_waypoint = authored["waypoints"]
            .as_array()
            .and_then(|waypoints| waypoints.last())
            .expect("the descent has waypoints");
        let authored_radius = DVec3::new(
            final_waypoint["position"][0].as_f64().expect("x"),
            final_waypoint["position"][1].as_f64().expect("y"),
            final_waypoint["position"][2].as_f64().expect("z"),
        )
        .length();
        // Waypoint interpolation lands a rounding step short of the authored
        // endpoint, so compare with a tolerance far below the metre this is
        // actually about.
        assert!(
            end_radius >= authored_radius - 1.0e-3,
            "ended at {end_radius} m, authored {authored_radius} m"
        );
        assert_eq!(scenario.expected_screenshots(), 5);
    }

    #[test]
    fn terrain_detail_altitude_ladder_covers_the_four_requested_heights() {
        let mut scenario =
            ScenarioRunner::load("terrain_detail_altitude_ladder").expect("scenario parses");
        scenario.retarget_sparse_landing_direction(DVec3::Z);
        let start = DVec3::from_array(scenario.advance().camera_world_position).length();
        let mut frame = scenario.advance();
        while !frame.complete {
            frame = scenario.advance();
        }
        let end = DVec3::from_array(frame.camera_world_position).length();
        assert!(end > start);
        assert_eq!(scenario.expected_screenshots(), 5);
        assert_eq!(scenario.expected_log_samples(), 17);
    }

    #[test]
    fn long_weather_steps_expect_at_most_one_spatial_log_per_frame() {
        let scenario = ScenarioRunner::load("weather_contrast").expect("scenario parses");
        assert_eq!(scenario.expected_log_samples(), 24);
    }

    #[test]
    fn atmospheric_mist_paths_capture_grazing_and_radial_air_columns() {
        let scenario = ScenarioRunner::load("atmospheric_mist_paths").expect("scenario parses");
        assert_eq!(scenario.expected_screenshots(), 4);
        assert_eq!(scenario.definition.waypoints.len(), 8);
        assert_eq!(scenario.definition.waypoints[2].position[0], 5_440_000.0);
        assert_eq!(scenario.definition.waypoints[4].look_at, [0.0; 3]);
        assert_eq!(scenario.definition.waypoints[6].position[0], 8_000_000.0);
    }

    #[test]
    fn weather_sun_occlusion_replays_a_mature_sunlit_ocean_storm() {
        let scenario = ScenarioRunner::load("weather_sun_occlusion").expect("scenario parses");
        assert_eq!(scenario.expected_screenshots(), 1);
        assert_eq!(scenario.expected_log_samples(), 24);
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
    fn manual_high_speed_scenario_extends_the_captured_w_flight() {
        let scenario = ScenarioRunner::load("manual_high_speed_clearance")
            .expect("high-speed manual replay parses");
        assert!(scenario.replays_forward_flight());
        assert_eq!(scenario.expected_screenshots(), 21);
        assert_eq!(scenario.assertions().min_camera_clearance_m, Some(4.5));
        assert_eq!(scenario.assertions().max_camera_clearance_m, None);
        assert_eq!(
            scenario.definition.forward_flight_start_time_seconds,
            Some(3.0)
        );
        assert_eq!(scenario.definition.duration_seconds, 18.0);
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

        let directionality = ScenarioRunner::load("twilight_directionality")
            .expect("twilight directionality scenario parses");
        assert_eq!(directionality.expected_screenshots(), 2);
        assert_eq!(directionality.definition.planet_rotation_time_scale, 0.0);
        assert_eq!(
            directionality
                .assertions()
                .min_solar_antisolar_sky_luminance_ratio,
            Some(1.1)
        );

        let night_side = ScenarioRunner::load("night_side_atmosphere")
            .expect("night-side atmosphere scenario parses");
        assert_eq!(night_side.expected_screenshots(), 1);
        assert_eq!(night_side.assertions().max_sky_luminance, Some(0.02));

        let limb =
            ScenarioRunner::load("limb_atmosphere").expect("limb atmosphere scenario parses");
        assert_eq!(limb.expected_screenshots(), 1);
        assert_eq!(limb.definition.planet_rotation_time_scale, 0.0);

        let orbital_profile = ScenarioRunner::load("orbital_atmosphere_profile")
            .expect("orbital atmosphere profile scenario parses");
        assert_eq!(orbital_profile.expected_screenshots(), 1);
        assert_eq!(orbital_profile.assertions().max_sky_luminance, Some(0.08));

        let orbital_continuity = ScenarioRunner::load("orbital_atmosphere_continuity")
            .expect("orbital atmosphere continuity scenario parses");
        assert_eq!(orbital_continuity.expected_screenshots(), 12);
        assert!(orbital_continuity.uses_fixed_exposure());

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

        let horizon_sun =
            ScenarioRunner::load("sun_horizon_visibility").expect("horizon sun scenario parses");
        assert_eq!(horizon_sun.expected_screenshots(), 8);
        assert!(horizon_sun.uses_fixed_exposure());
        assert_eq!(
            horizon_sun.assertions().sun_background_sample_uv,
            Some([0.56, 0.5])
        );
        assert_eq!(
            horizon_sun.assertions().min_visible_sun_contrast,
            Some(0.05)
        );

        let partial_sun =
            ScenarioRunner::load("partial_sun_occultation").expect("partial sun scenario parses");
        assert_eq!(partial_sun.expected_screenshots(), 2);
        let camera = DVec3::from_array(partial_sun.definition.waypoints[0].position);
        let sun = DVec3::from_array(partial_sun.definition.sun_waypoints[0].direction).normalize();
        let planet_direction = -camera.normalize();
        let center_angle = sun.dot(planet_direction).clamp(-1.0, 1.0).acos();
        let planet_angle = (crate::planet::PLANET_RADIUS_METERS / camera.length()).asin();
        let limb_offset = center_angle - planet_angle;
        assert!(
            limb_offset.abs() < 0.00925,
            "scenario must put the horizon through the visual solar disc"
        );
        let final_sun = DVec3::from_array(
            partial_sun
                .definition
                .sun_waypoints
                .last()
                .expect("final occulted sun")
                .direction,
        )
        .normalize();
        let final_center_angle = final_sun.dot(planet_direction).clamp(-1.0, 1.0).acos();
        assert!(
            final_center_angle + 0.00925 < planet_angle,
            "scenario must finish with the complete visual disc behind the planet"
        );

        let stare_at_sun = ScenarioRunner::load("stare_at_sun").expect("sun scenario parses");
        assert_eq!(stare_at_sun.expected_screenshots(), 3);
        assert_eq!(
            stare_at_sun.assertions().max_exposure_delta_per_frame,
            Some(0.5)
        );

        let sunrise_midday = ScenarioRunner::load("sunrise_midday_surface")
            .expect("surface sun comparison scenario parses");
        assert_eq!(sunrise_midday.expected_screenshots(), 4);
        assert!(sunrise_midday.uses_fixed_exposure());
        assert_eq!(sunrise_midday.definition.planet_rotation_time_scale, 0.0);

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
    fn blue_hour_scenario_sweeps_from_daylight_into_night() {
        let scenario = ScenarioRunner::load("sunset_blue_hour").expect("blue-hour scenario parses");
        assert_eq!(scenario.expected_screenshots(), 6);
        assert!(scenario.uses_fixed_exposure());
        assert_eq!(scenario.assertions().sky_sample_uv, Some([0.5, 0.1]));
        assert_eq!(scenario.assertions().max_sky_green_dominance, Some(0.0));
        assert_eq!(
            scenario.assertions().min_blue_hour_blue_red_ratio,
            Some(1.2)
        );
        assert_eq!(scenario.assertions().min_blue_hour_luminance, Some(0.03));
        assert_eq!(
            scenario.assertions().max_final_blue_hour_luminance_ratio,
            Some(0.02)
        );
        assert_eq!(
            scenario.assertions().max_adjacent_sky_luminance_increase,
            Some(0.005)
        );
        assert_eq!(scenario.definition.sun_waypoints.len(), 7);
        assert_eq!(scenario.definition.sun_waypoints[1].time_s, 1.0);
        assert_eq!(scenario.definition.sun_waypoints[6].time_s, 11.0);
        let first_elevation = scenario.definition.sun_waypoints[1].direction[0]
            .asin()
            .to_degrees();
        let last_elevation = scenario.definition.sun_waypoints[6].direction[0]
            .asin()
            .to_degrees();
        assert!((first_elevation - 15.0).abs() < 1.0e-9);
        assert!((last_elevation + 20.0).abs() < 1.0e-9);
    }

    #[test]
    fn outlined_shadow_scenario_replays_the_reported_low_sun_location() {
        let scenario = ScenarioRunner::load("outlined_shadows").expect("scenario parses");
        assert_eq!(scenario.expected_screenshots(), 1);
        assert_eq!(scenario.definition.waypoints.len(), 1);
        assert_eq!(scenario.definition.sun_waypoints.len(), 1);
        let mut camera = OrbitCamera::default();
        camera.set_reference_vertical_fov_degrees_for_viewport(
            scenario.definition.vertical_fov_waypoints[0].vertical_fov_degrees,
            720,
        );
        assert!(
            (camera.vertical_fov_radians().to_degrees() - 37.221_771_740_090_07).abs() < 1.0e-12
        );
        let position = DVec3::from_array(scenario.definition.waypoints[0].position);
        assert!((position.length() - 4_015_998.546_948_375_6).abs() < 1.0e-6);
        let solar_elevation = position
            .normalize()
            .dot(DVec3::from_array(
                scenario.definition.sun_waypoints[0].direction,
            ))
            .asin()
            .to_degrees();
        assert!((solar_elevation - 18.303_833_434_177_43).abs() < 1.0e-9);
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

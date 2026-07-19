use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tracing_subscriber::fmt::MakeWriter;

use crate::scenario::{MAX_TERRAIN_LOD_LEVEL, ScenarioAssertionResult, ScenarioAssertions};

pub const LOD_LEVEL_COUNT: usize = MAX_TERRAIN_LOD_LEVEL as usize + 1;

#[derive(Clone, Debug)]
pub struct SpatialLogSample {
    pub sim_time: f64,
    pub camera_world_position: [f64; 3],
    pub latitude_degrees: f64,
    pub longitude_degrees: f64,
    pub altitude_meters: f64,
    pub velocity_meters_per_second: f64,
    pub orientation: String,
    pub orientation_azimuth_radians: f64,
    pub orientation_elevation_radians: f64,
    pub vertical_fov_degrees: f64,
    pub sun_direction: [f64; 3],
    pub planet_rotation_radians: f64,
    pub lod_level_histogram: [u32; LOD_LEVEL_COUNT],
    pub chunks_loaded: u32,
    pub chunks_unloaded: u32,
    pub frame_time_ms: f32,
    pub draw_calls: u32,
    pub max_seam_delta_m: f64,
    pub resident_chunks: u32,
    pub drawn_chunks: u32,
    pub terrain_triangles: u64,
    pub fallback_chunks: u32,
    pub source_level_delta_histogram: [u32; LOD_LEVEL_COUNT],
    pub resident_tiles: u32,
    pub tiles_loaded: u32,
    pub tiles_unloaded: u32,
    pub lod_thrash_events: u32,
    pub budget_limited: bool,
    pub exposure: f32,
    pub ocean_wave_min_meters: f32,
    pub ocean_wave_max_meters: f32,
}

#[derive(Clone)]
pub(crate) struct SharedFile(Arc<Mutex<File>>);

pub(crate) struct LockedFile<'a>(MutexGuard<'a, File>);

impl Write for LockedFile<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for SharedFile {
    type Writer = LockedFile<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LockedFile(self.0.lock().expect("log file lock poisoned"))
    }
}

#[derive(Serialize)]
struct RunManifest {
    scenario: String,
    git_commit: String,
    timestamp_unix_seconds: u64,
    passed: Option<bool>,
    assertion_results: Vec<ScenarioAssertionResult>,
    failure_reasons: Vec<String>,
}

#[derive(Serialize)]
struct ScreenshotManifest {
    screenshots: Vec<ScreenshotEntry>,
}

#[derive(Clone, Serialize)]
struct ScreenshotEntry {
    filename: String,
    log_entry_sim_time: f64,
    solid_color_verified: bool,
    seam_gap_verified: Option<bool>,
    sky_sample_rgb: Option<[u8; 3]>,
    day_night_surface_luminance_ratio: Option<f32>,
    ice_sample_rgb: Option<[u8; 3]>,
}

pub struct RunArtifacts {
    root: PathBuf,
    screenshots_dir: PathBuf,
    manifest: RunManifest,
    screenshots: Vec<ScreenshotEntry>,
    spatial_log_count: usize,
    assertion_tracker: AssertionTracker,
}

struct AssertionTracker {
    config: ScenarioAssertions,
    sample_count: usize,
    non_finite_samples: usize,
    peak_lod_level: Option<u8>,
    observed_lod_level_sequence: Vec<u8>,
    previous_lod_level: Option<u8>,
    lod_regressions: usize,
    resident_chunk_violations: usize,
    maximum_resident_chunks: u32,
    lod_frame_count: usize,
    per_frame_resident_chunk_violations: usize,
    per_frame_maximum_resident_chunks: u32,
    lod_thrash_events: u64,
    per_frame_lod_thrash_events: u64,
    lod_budget_limited_frames: u64,
    seam_violations: usize,
    maximum_seam_delta_m: f64,
    fallback_violations: usize,
    maximum_fallback_chunks: u32,
    sky_samples: Vec<[u8; 3]>,
    day_night_surface_luminance_ratios: Vec<f32>,
    ice_samples: Vec<[u8; 3]>,
    exposure_sample_count: usize,
    exposure_bound_violations: usize,
    maximum_exposure_frame_delta: f32,
    exposure_oscillation_events: u32,
    previous_exposure: Option<f32>,
    previous_exposure_delta: Option<f32>,
    previous_target_exposure: Option<f32>,
    maximum_ocean_wave_range_meters: f32,
}

impl AssertionTracker {
    fn new(config: ScenarioAssertions) -> Self {
        Self {
            config,
            sample_count: 0,
            non_finite_samples: 0,
            peak_lod_level: None,
            observed_lod_level_sequence: Vec::new(),
            previous_lod_level: None,
            lod_regressions: 0,
            resident_chunk_violations: 0,
            maximum_resident_chunks: 0,
            lod_frame_count: 0,
            per_frame_resident_chunk_violations: 0,
            per_frame_maximum_resident_chunks: 0,
            lod_thrash_events: 0,
            per_frame_lod_thrash_events: 0,
            lod_budget_limited_frames: 0,
            seam_violations: 0,
            maximum_seam_delta_m: 0.0,
            fallback_violations: 0,
            maximum_fallback_chunks: 0,
            sky_samples: Vec::new(),
            day_night_surface_luminance_ratios: Vec::new(),
            ice_samples: Vec::new(),
            exposure_sample_count: 0,
            exposure_bound_violations: 0,
            maximum_exposure_frame_delta: 0.0,
            exposure_oscillation_events: 0,
            previous_exposure: None,
            previous_exposure_delta: None,
            previous_target_exposure: None,
            maximum_ocean_wave_range_meters: 0.0,
        }
    }

    fn observe(&mut self, sample: &SpatialLogSample) {
        self.sample_count += 1;
        if !sample_metrics_are_finite(sample) {
            self.non_finite_samples += 1;
        }

        let current_lod_level = sample
            .lod_level_histogram
            .iter()
            .rposition(|count| *count > 0)
            .map(|level| level as u8);
        if let Some(level) = current_lod_level {
            self.peak_lod_level = Some(self.peak_lod_level.map_or(level, |peak| peak.max(level)));
            if self.config.require_monotonic_lod_progression
                && self
                    .previous_lod_level
                    .is_some_and(|previous| level < previous)
            {
                self.lod_regressions += 1;
            }
            self.previous_lod_level = Some(level);
        }

        self.maximum_resident_chunks = self.maximum_resident_chunks.max(sample.resident_chunks);
        if self
            .config
            .min_resident_chunks
            .is_some_and(|minimum| sample.resident_chunks < minimum)
            || self
                .config
                .max_resident_chunks
                .is_some_and(|maximum| sample.resident_chunks > maximum)
        {
            self.resident_chunk_violations += 1;
        }

        self.lod_thrash_events += u64::from(sample.lod_thrash_events);
        self.maximum_ocean_wave_range_meters = self
            .maximum_ocean_wave_range_meters
            .max(sample.ocean_wave_max_meters - sample.ocean_wave_min_meters);
        if sample.max_seam_delta_m.is_finite() {
            self.maximum_seam_delta_m = self.maximum_seam_delta_m.max(sample.max_seam_delta_m);
            if self
                .config
                .max_seam_delta_m
                .is_some_and(|maximum| sample.max_seam_delta_m > maximum)
            {
                self.seam_violations += 1;
            }
        }

        self.maximum_fallback_chunks = self.maximum_fallback_chunks.max(sample.fallback_chunks);
        if self
            .config
            .max_fallback_chunks
            .is_some_and(|maximum| sample.fallback_chunks > maximum)
        {
            self.fallback_violations += 1;
        }
    }

    fn observe_lod_frame(
        &mut self,
        level_histogram: &[u32; LOD_LEVEL_COUNT],
        resident_chunks: u32,
        lod_thrash_events: u32,
        budget_limited: bool,
    ) {
        self.lod_frame_count += 1;
        if let Some(level) = level_histogram
            .iter()
            .rposition(|count| *count > 0)
            .map(|level| level as u8)
        {
            if self.observed_lod_level_sequence.last() != Some(&level) {
                self.observed_lod_level_sequence.push(level);
            }
        }
        self.per_frame_maximum_resident_chunks =
            self.per_frame_maximum_resident_chunks.max(resident_chunks);
        if self
            .config
            .min_resident_chunks
            .is_some_and(|minimum| resident_chunks < minimum)
            || self
                .config
                .max_resident_chunks
                .is_some_and(|maximum| resident_chunks > maximum)
        {
            self.per_frame_resident_chunk_violations += 1;
        }
        self.per_frame_lod_thrash_events += u64::from(lod_thrash_events);
        self.lod_budget_limited_frames += u64::from(budget_limited);
    }

    fn observe_sky_sample(&mut self, sample: [u8; 3]) {
        self.sky_samples.push(sample);
    }

    fn observe_day_night_surface_luminance_ratio(&mut self, ratio: f32) {
        self.day_night_surface_luminance_ratios.push(ratio);
    }

    fn observe_exposure(&mut self, exposure: f32, target_exposure: f32, average_luminance: f32) {
        self.exposure_sample_count += 1;
        if !exposure.is_finite() || !target_exposure.is_finite() || !average_luminance.is_finite() {
            self.exposure_bound_violations += 1;
            return;
        }
        if self
            .config
            .min_exposure
            .is_some_and(|minimum| exposure < minimum)
            || self
                .config
                .max_exposure
                .is_some_and(|maximum| exposure > maximum)
        {
            self.exposure_bound_violations += 1;
        }
        if let Some(previous_exposure) = self.previous_exposure {
            let delta = exposure - previous_exposure;
            self.maximum_exposure_frame_delta = self.maximum_exposure_frame_delta.max(delta.abs());
            let target_is_stable = self
                .previous_target_exposure
                .is_some_and(|previous_target| {
                    (target_exposure - previous_target).abs()
                        <= previous_target.abs().max(0.01) * 0.01
                });
            if target_is_stable
                && self.previous_exposure_delta.is_some_and(|previous_delta| {
                    previous_delta.abs() > 0.001
                        && delta.abs() > 0.001
                        && previous_delta.signum() != delta.signum()
                })
            {
                self.exposure_oscillation_events += 1;
            }
            self.previous_exposure_delta = Some(delta);
        }
        self.previous_exposure = Some(exposure);
        self.previous_target_exposure = Some(target_exposure);
    }

    fn results(&self, screenshot_count: usize) -> Vec<ScenarioAssertionResult> {
        let mut results = Vec::new();
        if self.config.require_finite_metrics {
            results.push(assertion_result(
                "finite_metrics",
                self.non_finite_samples == 0,
                format!(
                    "{} of {} spatial samples contained non-finite metrics",
                    self.non_finite_samples, self.sample_count
                ),
            ));
        }
        if let Some(required) = self.config.required_peak_lod_level {
            let observed = self.peak_lod_level;
            results.push(assertion_result(
                "lod_reaches_required_level",
                observed.is_some_and(|level| level >= required),
                format!("required level {required}, observed peak {observed:?}"),
            ));
        }
        if let Some(required) = &self.config.required_lod_level_sequence {
            results.push(assertion_result(
                "lod_traverses_required_sequence",
                self.observed_lod_level_sequence == *required,
                format!(
                    "required {required:?}, observed {:?}",
                    self.observed_lod_level_sequence
                ),
            ));
        }
        if self.config.require_unlimited_lod_budget {
            results.push(assertion_result(
                "lod_stays_within_chunk_budget",
                self.lod_budget_limited_frames == 0,
                format!(
                    "observed {} budget-limited frames",
                    self.lod_budget_limited_frames
                ),
            ));
        }
        if self.config.require_monotonic_lod_progression {
            results.push(assertion_result(
                "lod_progression_is_monotonic",
                self.lod_regressions == 0,
                format!("observed {} LOD regressions", self.lod_regressions),
            ));
        }
        if self.config.min_resident_chunks.is_some() || self.config.max_resident_chunks.is_some() {
            let (maximum_observed, violations) = if self.lod_frame_count > 0 {
                (
                    self.per_frame_maximum_resident_chunks,
                    self.per_frame_resident_chunk_violations,
                )
            } else {
                (self.maximum_resident_chunks, self.resident_chunk_violations)
            };
            results.push(assertion_result(
                "resident_chunk_count_is_bounded",
                violations == 0,
                format!(
                    "bounds {:?}..={:?}, maximum observed {}, violations {}",
                    self.config.min_resident_chunks,
                    self.config.max_resident_chunks,
                    maximum_observed,
                    violations
                ),
            ));
        }
        if let Some(maximum) = self.config.max_lod_thrash_events {
            let observed = self.lod_thrash_events.max(self.per_frame_lod_thrash_events);
            results.push(assertion_result(
                "lod_does_not_thrash",
                observed <= u64::from(maximum),
                format!("allowed {maximum} LOD thrash events, observed {}", observed),
            ));
        }
        if let Some(maximum) = self.config.max_seam_delta_m {
            results.push(assertion_result(
                "seam_delta_is_within_tolerance",
                self.seam_violations == 0,
                format!(
                    "tolerance {maximum}m, maximum observed {}m, violations {}",
                    self.maximum_seam_delta_m, self.seam_violations
                ),
            ));
        }
        if let Some(maximum) = self.config.max_fallback_chunks {
            results.push(assertion_result(
                "fallback_chunk_count_is_bounded",
                self.fallback_violations == 0,
                format!(
                    "allowed {maximum}, maximum observed {}, violations {}",
                    self.maximum_fallback_chunks, self.fallback_violations
                ),
            ));
        }
        if let Some(expected) = self.config.expected_screenshots {
            results.push(assertion_result(
                "expected_screenshots_were_captured",
                screenshot_count == expected,
                format!("expected {expected}, captured {screenshot_count}"),
            ));
        }
        if let Some(minimum_growth) = self.config.min_sunset_red_blue_growth {
            let first = self.sky_samples.first().copied();
            let last = self.sky_samples.last().copied();
            let growth = first.zip(last).map(|(first, last)| {
                red_blue_ratio(last) / red_blue_ratio(first).max(f32::EPSILON)
            });
            results.push(assertion_result(
                "sunset_red_over_blue_grows",
                growth.is_some_and(|growth| growth >= minimum_growth),
                format!(
                    "required growth {minimum_growth:.3}, observed {} from {} samples",
                    growth.map_or_else(|| "none".to_owned(), |growth| format!("{growth:.3}")),
                    self.sky_samples.len(),
                ),
            ));
        }
        if let Some(minimum_ratio) = self.config.min_final_sunset_red_blue_ratio {
            let ratio = self.sky_samples.last().copied().map(red_blue_ratio);
            results.push(assertion_result(
                "sunset_final_red_over_blue_is_warm",
                ratio.is_some_and(|ratio| ratio >= minimum_ratio),
                format!(
                    "required final ratio {minimum_ratio:.3}, observed {}",
                    ratio.map_or_else(|| "none".to_owned(), |ratio| format!("{ratio:.3}")),
                ),
            ));
        }
        if let Some(minimum_ratio) = self.config.min_solar_antisolar_sky_luminance_ratio {
            let ratio = self
                .sky_samples
                .first()
                .copied()
                .zip(self.sky_samples.last().copied())
                .map(|(solar, antisolar)| {
                    sky_luminance(solar) / sky_luminance(antisolar).max(0.001)
                });
            results.push(assertion_result(
                "solar_sky_is_brighter_than_antisolar_sky",
                self.sky_samples.len() >= 2 && ratio.is_some_and(|ratio| ratio >= minimum_ratio),
                format!(
                    "required ratio {minimum_ratio:.3}, observed {} from {} samples",
                    ratio.map_or_else(|| "none".to_owned(), |ratio| format!("{ratio:.3}")),
                    self.sky_samples.len(),
                ),
            ));
        }
        if let Some(maximum_delta) = self.config.max_adjacent_sky_luminance_delta {
            let maximum_observed = self
                .sky_samples
                .windows(2)
                .map(|pair| (sky_luminance(pair[1]) - sky_luminance(pair[0])).abs())
                .fold(0.0_f32, f32::max);
            results.push(assertion_result(
                "sky_luminance_transition_is_continuous",
                self.sky_samples.len() >= 2 && maximum_observed <= maximum_delta,
                format!(
                    "allowed adjacent delta {maximum_delta:.3}, observed {maximum_observed:.3} from {} samples",
                    self.sky_samples.len(),
                ),
            ));
        }
        if let Some(maximum_luminance) = self.config.max_sky_luminance {
            let maximum_observed = self
                .sky_samples
                .iter()
                .copied()
                .map(sky_luminance)
                .fold(0.0_f32, f32::max);
            results.push(assertion_result(
                "sky_samples_are_dark",
                !self.sky_samples.is_empty() && maximum_observed <= maximum_luminance,
                format!(
                    "allowed luminance {maximum_luminance:.3}, observed {maximum_observed:.3} from {} samples",
                    self.sky_samples.len(),
                ),
            ));
        }
        if let Some(minimum_ratio) = self.config.min_day_night_surface_luminance_ratio {
            let minimum_observed = self
                .day_night_surface_luminance_ratios
                .iter()
                .copied()
                .fold(f32::INFINITY, f32::min);
            results.push(assertion_result(
                "day_side_surface_is_brighter_than_night_side",
                minimum_observed.is_finite() && minimum_observed >= minimum_ratio,
                format!(
                    "required ratio {minimum_ratio:.3}, observed {} from {} screenshots",
                    if minimum_observed.is_finite() {
                        format!("{minimum_observed:.3}")
                    } else {
                        "none".to_owned()
                    },
                    self.day_night_surface_luminance_ratios.len(),
                ),
            ));
        }
        if self.config.min_exposure.is_some() || self.config.max_exposure.is_some() {
            results.push(assertion_result(
                "exposure_is_bounded",
                self.exposure_sample_count > 0 && self.exposure_bound_violations == 0,
                format!(
                    "bounds {:?}..={:?}, samples {}, violations {}",
                    self.config.min_exposure,
                    self.config.max_exposure,
                    self.exposure_sample_count,
                    self.exposure_bound_violations,
                ),
            ));
        }
        if let Some(maximum_delta) = self.config.max_exposure_delta_per_frame {
            results.push(assertion_result(
                "exposure_adapts_smoothly",
                self.exposure_sample_count >= 2
                    && self.maximum_exposure_frame_delta <= maximum_delta,
                format!(
                    "allowed per-frame delta {maximum_delta:.4}, observed {:.4} from {} samples",
                    self.maximum_exposure_frame_delta, self.exposure_sample_count,
                ),
            ));
        }
        if let Some(maximum_events) = self.config.max_exposure_oscillation_events {
            results.push(assertion_result(
                "exposure_does_not_oscillate",
                self.exposure_oscillation_events <= maximum_events,
                format!(
                    "allowed {maximum_events} oscillation events, observed {}",
                    self.exposure_oscillation_events,
                ),
            ));
        }
        if let Some(minimum_range) = self.config.min_ocean_wave_height_range_meters {
            results.push(assertion_result(
                "ocean_waves_have_required_height_range",
                self.maximum_ocean_wave_range_meters >= minimum_range,
                format!(
                    "required range {minimum_range:.3}m, observed {:.3}m",
                    self.maximum_ocean_wave_range_meters,
                ),
            ));
        }
        if self.config.min_ice_sample_luminance.is_some()
            || self.config.max_ice_sample_channel_spread.is_some()
        {
            let sample = self.ice_samples.last().copied();
            let luminance = sample.map(sky_luminance).unwrap_or(-1.0);
            let spread = sample
                .map(|rgb| {
                    let minimum = *rgb.iter().min().expect("RGB has channels") as f32 / 255.0;
                    let maximum = *rgb.iter().max().expect("RGB has channels") as f32 / 255.0;
                    maximum - minimum
                })
                .unwrap_or(f32::INFINITY);
            let passed = sample.is_some()
                && self
                    .config
                    .min_ice_sample_luminance
                    .is_none_or(|v| luminance >= v)
                && self
                    .config
                    .max_ice_sample_channel_spread
                    .is_none_or(|v| spread <= v);
            results.push(assertion_result(
                "polar_ice_is_bright_and_neutral",
                passed,
                format!("sample {sample:?}, luminance {luminance:.3}, channel spread {spread:.3}"),
            ));
        }
        results
    }
}

fn red_blue_ratio(sample: [u8; 3]) -> f32 {
    f32::from(sample[0]) / f32::from(sample[2]).max(1.0)
}

fn sky_luminance(sample: [u8; 3]) -> f32 {
    (0.2126 * f32::from(sample[0]) + 0.7152 * f32::from(sample[1]) + 0.0722 * f32::from(sample[2]))
        / 255.0
}

fn sample_metrics_are_finite(sample: &SpatialLogSample) -> bool {
    sample.sim_time.is_finite()
        && sample
            .camera_world_position
            .iter()
            .all(|value| value.is_finite())
        && sample.latitude_degrees.is_finite()
        && sample.longitude_degrees.is_finite()
        && sample.altitude_meters.is_finite()
        && sample.velocity_meters_per_second.is_finite()
        && sample.orientation_azimuth_radians.is_finite()
        && sample.orientation_elevation_radians.is_finite()
        && sample.vertical_fov_degrees.is_finite()
        && sample.sun_direction.iter().all(|value| value.is_finite())
        && sample.planet_rotation_radians.is_finite()
        && sample.frame_time_ms.is_finite()
        && sample.max_seam_delta_m.is_finite()
        && sample.exposure.is_finite()
        && sample.ocean_wave_min_meters.is_finite()
        && sample.ocean_wave_max_meters.is_finite()
}

fn assertion_result(name: &str, passed: bool, details: String) -> ScenarioAssertionResult {
    ScenarioAssertionResult {
        name: name.to_owned(),
        passed,
        details,
    }
}

pub struct PendingCapture {
    buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    filename: String,
}

#[allow(dead_code)]
impl RunArtifacts {
    pub fn create(scenario: &str) -> Result<(Self, SharedFile), String> {
        Self::create_with_assertions(scenario, ScenarioAssertions::default())
    }

    pub fn create_with_assertions(
        scenario: &str,
        assertions: ScenarioAssertions,
    ) -> Result<(Self, SharedFile), String> {
        let run_id = format!(
            "{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs(),
            std::process::id()
        );
        let root = Path::new("test-runs").join(scenario).join(run_id);
        let screenshots_dir = root.join("screenshots");
        fs::create_dir_all(&screenshots_dir).map_err(|error| error.to_string())?;
        let manifest = RunManifest {
            scenario: scenario.to_owned(),
            git_commit: git_commit(),
            timestamp_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs(),
            passed: None,
            assertion_results: Vec::new(),
            failure_reasons: Vec::new(),
        };
        let artifacts = Self {
            root,
            screenshots_dir,
            manifest,
            screenshots: Vec::new(),
            spatial_log_count: 0,
            assertion_tracker: AssertionTracker::new(assertions),
        };
        artifacts.write_manifests()?;
        let log_file =
            File::create(artifacts.root.join("log.jsonl")).map_err(|error| error.to_string())?;

        Ok((artifacts, SharedFile(Arc::new(Mutex::new(log_file)))))
    }

    pub fn configure_assertions(&mut self, assertions: ScenarioAssertions) {
        assert_eq!(
            self.spatial_log_count, 0,
            "scenario assertions must be configured before spatial samples are recorded"
        );
        self.assertion_tracker = AssertionTracker::new(assertions);
    }

    pub fn record_spatial_log(
        &mut self,
        sim_time: f64,
        camera_world_position: [f64; 3],
        altitude_meters: f64,
        orientation_azimuth_radians: f64,
        orientation_elevation_radians: f64,
        frame_time_ms: f32,
        draw_calls: u32,
    ) {
        self.record_spatial_sample(SpatialLogSample {
            sim_time,
            camera_world_position,
            latitude_degrees: 0.0,
            longitude_degrees: 0.0,
            altitude_meters,
            velocity_meters_per_second: 0.0,
            orientation: "orbit".to_owned(),
            orientation_azimuth_radians,
            orientation_elevation_radians,
            vertical_fov_degrees: 45.0,
            sun_direction: [0.4, 0.7, 0.6],
            planet_rotation_radians: 0.0,
            lod_level_histogram: [0; LOD_LEVEL_COUNT],
            chunks_loaded: 0,
            chunks_unloaded: 0,
            frame_time_ms,
            draw_calls,
            max_seam_delta_m: 0.0,
            resident_chunks: 0,
            drawn_chunks: 0,
            terrain_triangles: 0,
            fallback_chunks: 0,
            source_level_delta_histogram: [0; LOD_LEVEL_COUNT],
            resident_tiles: 0,
            tiles_loaded: 0,
            tiles_unloaded: 0,
            lod_thrash_events: 0,
            budget_limited: false,
            exposure: 1.0,
            ocean_wave_min_meters: 0.0,
            ocean_wave_max_meters: 0.0,
        });
    }

    pub fn record_spatial_sample(&mut self, sample: SpatialLogSample) {
        self.spatial_log_count += 1;
        self.assertion_tracker.observe(&sample);
        tracing::info!(
            target: "catinthegarden::spatial",
            sim_time = sample.sim_time,
            camera_world_x = sample.camera_world_position[0],
            camera_world_y = sample.camera_world_position[1],
            camera_world_z = sample.camera_world_position[2],
            latitude_degrees = sample.latitude_degrees,
            longitude_degrees = sample.longitude_degrees,
            altitude_meters = sample.altitude_meters,
            velocity_meters_per_second = sample.velocity_meters_per_second,
            orientation = sample.orientation,
            orientation_azimuth_radians = sample.orientation_azimuth_radians,
            orientation_elevation_radians = sample.orientation_elevation_radians,
            vertical_fov_degrees = sample.vertical_fov_degrees,
            sun_direction = ?sample.sun_direction,
            planet_rotation_radians = sample.planet_rotation_radians,
            lod_level_histogram = ?sample.lod_level_histogram,
            chunks_loaded = sample.chunks_loaded,
            chunks_unloaded = sample.chunks_unloaded,
            frame_time_ms = sample.frame_time_ms,
            draw_calls = sample.draw_calls,
            max_seam_delta_m = sample.max_seam_delta_m,
            resident_chunks = sample.resident_chunks,
            drawn_chunks = sample.drawn_chunks,
            terrain_triangles = sample.terrain_triangles,
            fallback_chunks = sample.fallback_chunks,
            source_level_delta_histogram = ?sample.source_level_delta_histogram,
            resident_tiles = sample.resident_tiles,
            tiles_loaded = sample.tiles_loaded,
            tiles_unloaded = sample.tiles_unloaded,
            lod_thrash_events = sample.lod_thrash_events,
            budget_limited = sample.budget_limited,
            exposure = sample.exposure,
            ocean_wave_min_meters = sample.ocean_wave_min_meters,
            ocean_wave_max_meters = sample.ocean_wave_max_meters,
            "spatial frame"
        );
    }

    pub fn observe_lod_frame(
        &mut self,
        level_histogram: &[u32; LOD_LEVEL_COUNT],
        resident_chunks: u32,
        lod_thrash_events: u32,
        budget_limited: bool,
    ) {
        self.assertion_tracker.observe_lod_frame(
            level_histogram,
            resident_chunks,
            lod_thrash_events,
            budget_limited,
        );
    }

    pub fn record_exposure_sample(
        &mut self,
        sim_time: f64,
        exposure: f32,
        target_exposure: f32,
        average_luminance: f32,
    ) {
        self.assertion_tracker
            .observe_exposure(exposure, target_exposure, average_luminance);
        tracing::info!(
            target: "catinthegarden::exposure",
            sim_time,
            exposure,
            target_exposure,
            average_luminance,
            "auto exposure frame"
        );
    }

    pub fn screenshot_count(&self) -> usize {
        self.screenshots.len()
    }

    pub fn spatial_log_count(&self) -> usize {
        self.spatial_log_count
    }

    pub fn record_render_profile(
        &self,
        sim_time: f64,
        simulation_ms: f32,
        egui_ms: f32,
        surface_acquire_ms: f32,
        egui_upload_ms: f32,
        vertex_rebase_ms: f32,
        vertex_upload_ms: f32,
        encode_ms: f32,
        submit_ms: f32,
        present_ms: f32,
        capture_readback_ms: f32,
        gpu_render_ms: f64,
        gpu_timestamp_readback_ms: f32,
        total_render_ms: f32,
    ) {
        tracing::info!(
            target: "catinthegarden::render_profile",
            sim_time,
            simulation_ms,
            egui_ms,
            surface_acquire_ms,
            egui_upload_ms,
            vertex_rebase_ms,
            vertex_upload_ms,
            encode_ms,
            submit_ms,
            present_ms,
            capture_readback_ms,
            gpu_render_ms,
            gpu_timestamp_readback_ms,
            total_render_ms,
            "render timing sample"
        );
    }

    pub fn record_gpu_timestamps(&self, sim_time: f64, timings: crate::GpuStageTimings) {
        tracing::info!(
            target: "catinthegarden::gpu_profile",
            sim_time,
            gpu_scene_ms = timings.scene_ms,
            gpu_luminance_ms = timings.luminance_ms,
            gpu_sun_ms = timings.sun_ms,
            gpu_blur_ms = timings.blur_ms,
            gpu_bloom_ms = timings.bloom_ms,
            gpu_tone_map_ms = timings.tone_map_ms,
            gpu_egui_ms = timings.egui_ms,
            gpu_render_ms = timings.total_ms(),
            "asynchronous GPU stage timing sample"
        );
    }

    pub fn assertion_results(&self) -> Vec<ScenarioAssertionResult> {
        self.assertion_tracker.results(self.screenshot_count())
    }

    pub fn assertion_failure_reasons(&self) -> Vec<String> {
        self.assertion_results()
            .into_iter()
            .filter_map(|result| {
                (!result.passed).then(|| format!("{}: {}", result.name, result.details))
            })
            .collect()
    }

    pub fn final_passed(&self, harness_passed: bool) -> bool {
        harness_passed && self.assertion_failure_reasons().is_empty()
    }

    pub fn finish(&mut self, harness_passed: bool) -> Result<(), String> {
        self.manifest.assertion_results = self.assertion_results();
        self.manifest.failure_reasons = self.assertion_failure_reasons();
        if !harness_passed {
            self.manifest
                .failure_reasons
                .push("scenario harness checks failed".to_owned());
        }
        self.manifest.passed = Some(harness_passed && self.manifest.failure_reasons.is_empty());
        self.write_manifests()
    }

    fn record_screenshot(
        &mut self,
        filename: String,
        log_entry_sim_time: f64,
        solid_color_verified: bool,
        seam_gap_verified: Option<bool>,
        sky_sample_rgb: Option<[u8; 3]>,
        day_night_surface_luminance_ratio: Option<f32>,
        ice_sample_rgb: Option<[u8; 3]>,
    ) -> Result<(), String> {
        self.screenshots.push(ScreenshotEntry {
            filename,
            log_entry_sim_time,
            solid_color_verified,
            seam_gap_verified,
            sky_sample_rgb,
            day_night_surface_luminance_ratio,
            ice_sample_rgb,
        });
        self.write_manifests()
    }

    fn record_sky_sample(&mut self, pixels: &[u8], width: u32, height: u32) -> Option<[u8; 3]> {
        let [u, v] = self.assertion_tracker.config.sky_sample_uv?;
        let x = (u * (width.saturating_sub(1)) as f32).round() as usize;
        let y = (v * (height.saturating_sub(1)) as f32).round() as usize;
        let pixel = pixels[(y * width as usize + x) * 4..][..3]
            .try_into()
            .expect("sample coordinate is inside the screenshot");
        self.assertion_tracker.observe_sky_sample(pixel);
        Some(pixel)
    }

    fn record_day_night_surface_luminance_ratio(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Option<f32> {
        let [day_u, day_v] = self.assertion_tracker.config.day_surface_sample_uv?;
        let [night_u, night_v] = self.assertion_tracker.config.night_surface_sample_uv?;
        let sample = |u: f32, v: f32| {
            let x = (u * (width.saturating_sub(1)) as f32).round() as usize;
            let y = (v * (height.saturating_sub(1)) as f32).round() as usize;
            pixels[(y * width as usize + x) * 4..][..3]
                .try_into()
                .expect("sample coordinate is inside the screenshot")
        };
        let day_luminance = sky_luminance(sample(day_u, day_v));
        let night_luminance = sky_luminance(sample(night_u, night_v));
        let ratio = day_luminance / night_luminance.max(0.001);
        self.assertion_tracker
            .observe_day_night_surface_luminance_ratio(ratio);
        Some(ratio)
    }

    fn record_ice_sample(&mut self, pixels: &[u8], width: u32, height: u32) -> Option<[u8; 3]> {
        let [u, v] = self.assertion_tracker.config.ice_sample_uv?;
        let x = (u.clamp(0.0, 1.0) * (width - 1) as f32).round() as usize;
        let y = (v.clamp(0.0, 1.0) * (height - 1) as f32).round() as usize;
        let sample: [u8; 3] = pixels[(y * width as usize + x) * 4..][..3]
            .try_into()
            .expect("sample coordinate is inside the screenshot");
        self.assertion_tracker.ice_samples.push(sample);
        Some(sample)
    }

    fn write_manifests(&self) -> Result<(), String> {
        write_json(self.root.join("manifest.json"), &self.manifest)?;
        write_json(
            self.screenshots_dir.join("manifest.json"),
            &ScreenshotManifest {
                screenshots: self.screenshots.clone(),
            },
        )
    }
}

pub fn init_tracing(log_writer: SharedFile) {
    tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_writer(log_writer)
        .init();
}

pub fn schedule_capture(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    capture_number: usize,
) -> PendingCapture {
    let unpadded_bytes_per_row = width * 4;
    let padded_bytes_per_row =
        unpadded_bytes_per_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot readback buffer"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    PendingCapture {
        buffer,
        padded_bytes_per_row,
        width,
        height,
        format,
        filename: format!("capture-{capture_number:03}.png"),
    }
}

pub fn finish_capture(
    device: &wgpu::Device,
    pending: PendingCapture,
    artifacts: &mut RunArtifacts,
    sim_time: f64,
    verify_solid_color: bool,
    verify_no_background_gaps: bool,
) -> Result<bool, String> {
    let (sender, receiver) = mpsc::channel();
    pending
        .buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())??;

    let mapped = pending.buffer.slice(..).get_mapped_range();
    let mut pixels = Vec::with_capacity((pending.width * pending.height * 4) as usize);
    for row in mapped.chunks_exact(pending.padded_bytes_per_row as usize) {
        pixels.extend_from_slice(&row[..(pending.width * 4) as usize]);
    }
    drop(mapped);
    pending.buffer.unmap();

    match pending.format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => {}
        format => return Err(format!("unsupported screenshot format {format:?}")),
    }

    let solid_color_verified = pixels.chunks_exact(4).all(|pixel| pixel == &pixels[..4]);
    if verify_solid_color && !solid_color_verified {
        return Err("solid-color scenario screenshot contains more than one color".to_owned());
    }
    let seam_gap_verified = verify_no_background_gaps
        .then(|| no_background_gaps(&pixels, pending.width, pending.height));
    if seam_gap_verified == Some(false) {
        return Err("planet screenshot contains a background gap inside its silhouette".to_owned());
    }
    image::save_buffer(
        artifacts.screenshots_dir.join(&pending.filename),
        &pixels,
        pending.width,
        pending.height,
        image::ColorType::Rgba8,
    )
    .map_err(|error| error.to_string())?;
    let sky_sample_rgb = artifacts.record_sky_sample(&pixels, pending.width, pending.height);
    let day_night_surface_luminance_ratio =
        artifacts.record_day_night_surface_luminance_ratio(&pixels, pending.width, pending.height);
    let ice_sample_rgb = artifacts.record_ice_sample(&pixels, pending.width, pending.height);
    artifacts.record_screenshot(
        pending.filename,
        sim_time,
        solid_color_verified,
        seam_gap_verified,
        sky_sample_rgb,
        day_night_surface_luminance_ratio,
        ice_sample_rgb,
    )?;
    Ok(solid_color_verified)
}

fn no_background_gaps(pixels: &[u8], width: u32, height: u32) -> bool {
    let background = &pixels[..4];
    let mut rows_with_planet = 0;
    for row in pixels
        .chunks_exact((width * 4) as usize)
        .take(height as usize)
    {
        let non_background = row
            .chunks_exact(4)
            .enumerate()
            .filter_map(|(index, pixel)| (pixel != background).then_some(index));
        let Some(first) = non_background.clone().next() else {
            continue;
        };
        let last = non_background
            .last()
            .expect("first non-background pixel exists");
        rows_with_planet += 1;
        if row
            .chunks_exact(4)
            .skip(first)
            .take(last - first + 1)
            .any(|pixel| pixel == background)
        {
            return false;
        }
    }
    rows_with_planet > height as usize / 8
}

fn write_json(path: PathBuf, value: &impl Serialize) -> Result<(), String> {
    let contents = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, contents).map_err(|error| error.to_string())
}

fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|commit| commit.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{AssertionTracker, LOD_LEVEL_COUNT, SpatialLogSample};
    use crate::scenario::ScenarioAssertions;

    fn assertions() -> ScenarioAssertions {
        ScenarioAssertions {
            require_finite_metrics: true,
            required_peak_lod_level: Some(18),
            required_lod_level_sequence: None,
            require_monotonic_lod_progression: true,
            require_unlimited_lod_budget: false,
            min_resident_chunks: Some(6),
            max_resident_chunks: Some(64),
            max_lod_thrash_events: Some(0),
            max_seam_delta_m: Some(0.1),
            max_fallback_chunks: Some(4),
            expected_screenshots: Some(2),
            sky_sample_uv: None,
            min_sunset_red_blue_growth: None,
            min_final_sunset_red_blue_ratio: None,
            min_solar_antisolar_sky_luminance_ratio: None,
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
            ice_sample_uv: None,
            min_ice_sample_luminance: None,
            max_ice_sample_channel_spread: None,
        }
    }

    fn sample(level: usize, resident_chunks: u32) -> SpatialLogSample {
        let mut lod_level_histogram = [0; LOD_LEVEL_COUNT];
        lod_level_histogram[level] = resident_chunks;
        SpatialLogSample {
            sim_time: level as f64,
            camera_world_position: [4_000_010.0, 0.0, 0.0],
            latitude_degrees: 0.0,
            longitude_degrees: 0.0,
            altitude_meters: 10.0,
            velocity_meters_per_second: 0.0,
            orientation: "waypoint".to_owned(),
            orientation_azimuth_radians: 0.0,
            orientation_elevation_radians: 0.0,
            vertical_fov_degrees: 45.0,
            sun_direction: [0.4, 0.7, 0.6],
            planet_rotation_radians: 0.0,
            lod_level_histogram,
            chunks_loaded: 1,
            chunks_unloaded: 0,
            frame_time_ms: 16.0,
            draw_calls: resident_chunks,
            max_seam_delta_m: 0.01,
            resident_chunks,
            drawn_chunks: resident_chunks,
            terrain_triangles: u64::from(resident_chunks) * 2_304,
            fallback_chunks: 0,
            source_level_delta_histogram: [0; LOD_LEVEL_COUNT],
            resident_tiles: 0,
            tiles_loaded: 0,
            tiles_unloaded: 0,
            lod_thrash_events: 0,
            budget_limited: false,
            exposure: 1.0,
            ocean_wave_min_meters: 0.0,
            ocean_wave_max_meters: 0.0,
        }
    }

    fn result<'a>(
        results: &'a [crate::scenario::ScenarioAssertionResult],
        name: &str,
    ) -> &'a crate::scenario::ScenarioAssertionResult {
        results
            .iter()
            .find(|result| result.name == name)
            .expect("assertion result exists")
    }

    #[test]
    fn valid_descent_metrics_pass_all_tier_one_assertions() {
        let mut tracker = AssertionTracker::new(assertions());
        tracker.observe(&sample(0, 6));
        tracker.observe(&sample(9, 24));
        tracker.observe(&sample(18, 48));

        let results = tracker.results(2);
        assert!(results.iter().all(|result| result.passed));
    }

    #[test]
    fn invalid_metrics_report_each_failure_reason() {
        let mut tracker = AssertionTracker::new(assertions());
        tracker.observe(&sample(10, 12));
        let mut invalid = sample(8, 100);
        invalid.altitude_meters = f64::NAN;
        invalid.lod_thrash_events = 1;
        invalid.max_seam_delta_m = 0.25;
        invalid.fallback_chunks = 5;
        tracker.observe(&invalid);

        let results = tracker.results(1);
        for name in [
            "finite_metrics",
            "lod_reaches_required_level",
            "lod_progression_is_monotonic",
            "resident_chunk_count_is_bounded",
            "lod_does_not_thrash",
            "seam_delta_is_within_tolerance",
            "fallback_chunk_count_is_bounded",
            "expected_screenshots_were_captured",
        ] {
            assert!(!result(&results, name).passed, "{name} should fail");
        }
    }

    #[test]
    fn per_frame_lod_assertions_require_the_full_round_trip_without_budget_pressure() {
        let mut config = ScenarioAssertions::default();
        config.required_lod_level_sequence = Some(vec![2, 3, 4, 3, 2]);
        config.require_unlimited_lod_budget = true;
        config.max_resident_chunks = Some(1);
        let mut tracker = AssertionTracker::new(config);

        for level in [2_usize, 3, 4, 3, 2] {
            let mut histogram = [0; LOD_LEVEL_COUNT];
            histogram[level] = 1;
            tracker.observe_lod_frame(&histogram, 1, 0, false);
        }
        let results = tracker.results(0);
        assert!(result(&results, "lod_traverses_required_sequence").passed);
        assert!(result(&results, "lod_stays_within_chunk_budget").passed);
        assert!(result(&results, "resident_chunk_count_is_bounded").passed);

        let mut histogram = [0; LOD_LEVEL_COUNT];
        histogram[2] = 1;
        tracker.observe_lod_frame(&histogram, 2, 0, true);
        assert!(!result(&tracker.results(0), "lod_stays_within_chunk_budget").passed);
        assert!(!result(&tracker.results(0), "resident_chunk_count_is_bounded").passed);
    }

    #[test]
    fn exposure_assertions_reject_snaps_and_oscillation() {
        let mut config = ScenarioAssertions::default();
        config.min_exposure = Some(0.05);
        config.max_exposure = Some(2.0);
        config.max_exposure_delta_per_frame = Some(0.2);
        config.max_exposure_oscillation_events = Some(0);
        let mut tracker = AssertionTracker::new(config);

        tracker.observe_exposure(1.0, 1.0, 0.18);
        tracker.observe_exposure(1.1, 1.0, 0.18);
        tracker.observe_exposure(1.4, 1.0, 0.18);
        tracker.observe_exposure(1.1, 1.0, 0.18);

        let results = tracker.results(0);
        assert!(result(&results, "exposure_is_bounded").passed);
        assert!(!result(&results, "exposure_adapts_smoothly").passed);
        assert!(!result(&results, "exposure_does_not_oscillate").passed);
    }

    #[test]
    fn image_assertions_measure_sunset_warming_and_smooth_sky_transition() {
        let mut config = assertions();
        config.sky_sample_uv = Some([0.5, 0.25]);
        config.min_sunset_red_blue_growth = Some(1.5);
        config.min_final_sunset_red_blue_ratio = Some(1.0);
        config.max_adjacent_sky_luminance_delta = Some(0.2);
        let mut tracker = AssertionTracker::new(config);
        tracker.observe_sky_sample([40, 80, 180]);
        tracker.observe_sky_sample([70, 80, 130]);
        tracker.observe_sky_sample([100, 75, 80]);

        let results = tracker.results(2);
        for name in [
            "sunset_red_over_blue_grows",
            "sunset_final_red_over_blue_is_warm",
            "sky_luminance_transition_is_continuous",
        ] {
            assert!(result(&results, name).passed, "{name} should pass");
        }
    }

    #[test]
    fn image_assertions_require_directional_twilight() {
        let mut config = assertions();
        config.sky_sample_uv = Some([0.5, 0.25]);
        config.min_solar_antisolar_sky_luminance_ratio = Some(1.5);
        let mut tracker = AssertionTracker::new(config);
        tracker.observe_sky_sample([180, 120, 40]);
        tracker.observe_sky_sample([60, 50, 40]);

        assert!(
            result(
                &tracker.results(2),
                "solar_sky_is_brighter_than_antisolar_sky"
            )
            .passed
        );
    }

    #[test]
    fn image_assertions_reject_a_lit_night_side_sky_sample() {
        let mut config = assertions();
        config.sky_sample_uv = Some([0.5, 0.25]);
        config.max_sky_luminance = Some(0.02);
        let mut tracker = AssertionTracker::new(config);
        tracker.observe_sky_sample([4, 4, 4]);
        assert!(result(&tracker.results(1), "sky_samples_are_dark").passed);

        tracker.observe_sky_sample([80, 80, 80]);
        assert!(!result(&tracker.results(2), "sky_samples_are_dark").passed);
    }

    #[test]
    fn image_assertions_require_a_bright_day_side_and_dark_night_side() {
        let mut config = assertions();
        config.min_day_night_surface_luminance_ratio = Some(5.0);

        let mut dim_contrast = AssertionTracker::new(config.clone());
        dim_contrast.observe_day_night_surface_luminance_ratio(4.9);
        assert!(
            !result(
                &dim_contrast.results(1),
                "day_side_surface_is_brighter_than_night_side"
            )
            .passed
        );

        let mut sufficient_contrast = AssertionTracker::new(config);
        sufficient_contrast.observe_day_night_surface_luminance_ratio(5.1);
        assert!(
            result(
                &sufficient_contrast.results(1),
                "day_side_surface_is_brighter_than_night_side"
            )
            .passed
        );
    }
}

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
    pub fallback_chunks: u32,
    pub resident_tiles: u32,
    pub tiles_loaded: u32,
    pub tiles_unloaded: u32,
    pub lod_thrash_events: u32,
    pub exposure: f32,
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
    previous_lod_level: Option<u8>,
    lod_regressions: usize,
    resident_chunk_violations: usize,
    maximum_resident_chunks: u32,
    lod_thrash_events: u64,
    seam_violations: usize,
    maximum_seam_delta_m: f64,
    fallback_violations: usize,
    maximum_fallback_chunks: u32,
    sky_samples: Vec<[u8; 3]>,
    exposure_sample_count: usize,
    exposure_bound_violations: usize,
    maximum_exposure_frame_delta: f32,
    exposure_oscillation_events: u32,
    previous_exposure: Option<f32>,
    previous_exposure_delta: Option<f32>,
    previous_target_exposure: Option<f32>,
}

impl AssertionTracker {
    fn new(config: ScenarioAssertions) -> Self {
        Self {
            config,
            sample_count: 0,
            non_finite_samples: 0,
            peak_lod_level: None,
            previous_lod_level: None,
            lod_regressions: 0,
            resident_chunk_violations: 0,
            maximum_resident_chunks: 0,
            lod_thrash_events: 0,
            seam_violations: 0,
            maximum_seam_delta_m: 0.0,
            fallback_violations: 0,
            maximum_fallback_chunks: 0,
            sky_samples: Vec::new(),
            exposure_sample_count: 0,
            exposure_bound_violations: 0,
            maximum_exposure_frame_delta: 0.0,
            exposure_oscillation_events: 0,
            previous_exposure: None,
            previous_exposure_delta: None,
            previous_target_exposure: None,
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

    fn observe_sky_sample(&mut self, sample: [u8; 3]) {
        self.sky_samples.push(sample);
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
        if self.config.require_monotonic_lod_progression {
            results.push(assertion_result(
                "lod_progression_is_monotonic",
                self.lod_regressions == 0,
                format!("observed {} LOD regressions", self.lod_regressions),
            ));
        }
        if self.config.min_resident_chunks.is_some() || self.config.max_resident_chunks.is_some() {
            results.push(assertion_result(
                "resident_chunk_count_is_bounded",
                self.resident_chunk_violations == 0,
                format!(
                    "bounds {:?}..={:?}, maximum observed {}, violations {}",
                    self.config.min_resident_chunks,
                    self.config.max_resident_chunks,
                    self.maximum_resident_chunks,
                    self.resident_chunk_violations
                ),
            ));
        }
        if let Some(maximum) = self.config.max_lod_thrash_events {
            results.push(assertion_result(
                "lod_does_not_thrash",
                self.lod_thrash_events <= u64::from(maximum),
                format!(
                    "allowed {maximum} LOD thrash events, observed {}",
                    self.lod_thrash_events
                ),
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
            fallback_chunks: 0,
            resident_tiles: 0,
            tiles_loaded: 0,
            tiles_unloaded: 0,
            lod_thrash_events: 0,
            exposure: 1.0,
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
            fallback_chunks = sample.fallback_chunks,
            resident_tiles = sample.resident_tiles,
            tiles_loaded = sample.tiles_loaded,
            tiles_unloaded = sample.tiles_unloaded,
            lod_thrash_events = sample.lod_thrash_events,
            exposure = sample.exposure,
            "spatial frame"
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

    pub fn record_gpu_timestamp(&self, sim_time: f64, gpu_render_ms: f64) {
        tracing::info!(
            target: "catinthegarden::gpu_profile",
            sim_time,
            gpu_render_ms,
            "asynchronous GPU timing sample"
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
    ) -> Result<(), String> {
        self.screenshots.push(ScreenshotEntry {
            filename,
            log_entry_sim_time,
            solid_color_verified,
            seam_gap_verified,
            sky_sample_rgb,
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
    artifacts.record_screenshot(
        pending.filename,
        sim_time,
        solid_color_verified,
        seam_gap_verified,
        sky_sample_rgb,
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
            require_monotonic_lod_progression: true,
            min_resident_chunks: Some(6),
            max_resident_chunks: Some(64),
            max_lod_thrash_events: Some(0),
            max_seam_delta_m: Some(0.1),
            max_fallback_chunks: Some(4),
            expected_screenshots: Some(2),
            sky_sample_uv: None,
            min_sunset_red_blue_growth: None,
            min_final_sunset_red_blue_ratio: None,
            max_adjacent_sky_luminance_delta: None,
            min_exposure: None,
            max_exposure: None,
            max_exposure_delta_per_frame: None,
            max_exposure_oscillation_events: None,
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
            fallback_chunks: 0,
            resident_tiles: 0,
            tiles_loaded: 0,
            tiles_unloaded: 0,
            lod_thrash_events: 0,
            exposure: 1.0,
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
}

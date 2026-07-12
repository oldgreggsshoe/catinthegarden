use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tracing_subscriber::fmt::MakeWriter;

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
}

pub struct RunArtifacts {
    root: PathBuf,
    screenshots_dir: PathBuf,
    manifest: RunManifest,
    screenshots: Vec<ScreenshotEntry>,
    spatial_log_count: usize,
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
        };
        let artifacts = Self {
            root,
            screenshots_dir,
            manifest,
            screenshots: Vec::new(),
            spatial_log_count: 0,
        };
        artifacts.write_manifests()?;
        let log_file =
            File::create(artifacts.root.join("log.jsonl")).map_err(|error| error.to_string())?;

        Ok((artifacts, SharedFile(Arc::new(Mutex::new(log_file)))))
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
        self.spatial_log_count += 1;
        tracing::info!(
            target: "catinthegarden::spatial",
            sim_time,
            camera_world_x = camera_world_position[0],
            camera_world_y = camera_world_position[1],
            camera_world_z = camera_world_position[2],
            latitude_degrees = 0.0_f64,
            longitude_degrees = 0.0_f64,
            altitude_meters,
            velocity_meters_per_second = 0.0_f64,
            orientation = "orbit",
            orientation_azimuth_radians,
            orientation_elevation_radians,
            lod_level_histogram = "0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0",
            chunks_loaded = 0_u32,
            chunks_unloaded = 0_u32,
            frame_time_ms,
            draw_calls,
            "spatial frame"
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

    pub fn finish(&mut self, passed: bool) -> Result<(), String> {
        self.manifest.passed = Some(passed);
        self.write_manifests()
    }

    fn record_screenshot(
        &mut self,
        filename: String,
        log_entry_sim_time: f64,
        solid_color_verified: bool,
        seam_gap_verified: Option<bool>,
    ) -> Result<(), String> {
        self.screenshots.push(ScreenshotEntry {
            filename,
            log_entry_sim_time,
            solid_color_verified,
            seam_gap_verified,
        });
        self.write_manifests()
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
    artifacts.record_screenshot(
        pending.filename,
        sim_time,
        solid_color_verified,
        seam_gap_verified,
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

mod atmosphere;
mod debug;
mod outmap;
mod planet;
mod scenario;
mod terrain;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use winit::{
    event::{DeviceEvent, Event, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowAttributes},
};

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.08,
    g: 0.08,
    b: 0.09,
    a: 1.0,
};
const HUD_REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const HIDDEN_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const GPU_PROFILE_RING_SIZE: usize = 3;
const DEFAULT_OUTMAP_PATH: &str = "assets/outmaps/test-planet";
const DEFAULT_CAMERA_ORBIT_RADIANS_PER_SECOND: f64 = 0.08;
const MOUSE_LOOK_RADIANS_PER_PIXEL: f64 = 0.0006;

struct PendingGpuTimestamp {
    sim_time: f64,
    receiver: mpsc::Receiver<bool>,
}

struct GpuProfileSlot {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    pending: Option<PendingGpuTimestamp>,
}

struct GpuProfiler {
    slots: Vec<GpuProfileSlot>,
    next_slot: usize,
    timestamp_period_ns: f32,
}

impl GpuProfiler {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let slots = (0..GPU_PROFILE_RING_SIZE)
            .map(|_| GpuProfileSlot {
                query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("render timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                }),
                resolve_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("render timestamp resolve"),
                    size: 16,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                readback_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("render timestamp readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                pending: None,
            })
            .collect();
        Self {
            slots,
            next_slot: 0,
            timestamp_period_ns: queue.get_timestamp_period(),
        }
    }

    fn acquire_slot(&mut self) -> Option<usize> {
        for offset in 0..self.slots.len() {
            let index = (self.next_slot + offset) % self.slots.len();
            if self.slots[index].pending.is_none() {
                self.next_slot = (index + 1) % self.slots.len();
                return Some(index);
            }
        }
        None
    }

    fn begin_readback(&mut self, index: usize, sim_time: f64) {
        let (sender, receiver) = mpsc::channel();
        self.slots[index]
            .readback_buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result.is_ok());
            });
        self.slots[index].pending = Some(PendingGpuTimestamp { sim_time, receiver });
    }

    fn collect_completed(&mut self, device: &wgpu::Device) -> Vec<(f64, f64)> {
        let _ = device.poll(wgpu::PollType::Poll);
        let mut completed = Vec::new();
        for slot in &mut self.slots {
            let Some(pending) = slot.pending.as_ref() else {
                continue;
            };
            let Ok(mapped_ok) = pending.receiver.try_recv() else {
                continue;
            };
            let sim_time = pending.sim_time;
            slot.pending = None;
            if !mapped_ok {
                continue;
            }
            let timestamps = slot.readback_buffer.slice(..).get_mapped_range();
            let values: &[u64] = bytemuck::cast_slice(&timestamps);
            let elapsed_ms = values[1].saturating_sub(values[0]) as f64
                * f64::from(self.timestamp_period_ns)
                / 1_000_000.0;
            drop(timestamps);
            slot.readback_buffer.unmap();
            completed.push((sim_time, elapsed_ms));
        }
        completed
    }
}

struct State {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    depth_view: wgpu::TextureView,
    atmosphere: atmosphere::AtmosphereRenderer,
    terrain: terrain::TerrainRenderer,
    terrain_stats: terrain::TerrainStats,
    camera: planet::OrbitCamera,
    sun_direction: glam::DVec3,
    previous_camera_world_position: glam::DVec3,
    previous_sim_time: f64,
    last_auto_orbit_sim_time: f64,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    started_at: Instant,
    egui_context: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    last_frame: Instant,
    fps: f32,
    debug_overlay_visible: bool,
    manual_screenshot_requested: bool,
    next_log_time: f64,
    capture_number: usize,
    scenario: Option<scenario::ScenarioRunner>,
    artifacts: debug::RunArtifacts,
    scenario_capture_failed: bool,
    mouse_captured: bool,
    profile_render: bool,
    gpu_profiler: Option<GpuProfiler>,
    cached_paint_jobs: Vec<egui::ClippedPrimitive>,
    egui_buffers_dirty: bool,
    next_hud_update: Instant,
    hud_dirty: bool,
}

impl State {
    async fn new(
        window: Arc<Window>,
        scenario_name: Option<String>,
        profile_render: bool,
        terrain_source: terrain::TerrainSource,
    ) -> Self {
        let scenario = scenario_name
            .as_deref()
            .map(scenario::ScenarioRunner::load)
            .transpose()
            .expect("scenario must be valid");
        let artifact_name = scenario
            .as_ref()
            .map_or("manual", scenario::ScenarioRunner::name);
        let assertions = scenario
            .as_ref()
            .map(|scenario| scenario.assertions().clone())
            .unwrap_or_default();
        let (artifacts, log_writer) =
            debug::RunArtifacts::create_with_assertions(artifact_name, assertions)
                .expect("test-run storage must be writable");
        debug::init_tracing(log_writer);
        tracing::info!(scenario = artifact_name, ?terrain_source, "run started");

        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("the window must provide a compatible surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("no suitable GPU adapter found");
        let timestamp_features =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        let requested_features =
            if profile_render && adapter.features().contains(timestamp_features) {
                timestamp_features
            } else {
                wgpu::Features::empty()
            };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("render device"),
                required_features: requested_features,
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("failed to create render device");
        let gpu_profiler = requested_features
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| GpuProfiler::new(&device, &queue));

        let surface_capabilities = surface.get_capabilities(&adapter);
        let surface_format = surface_capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_capabilities.formats[0]);
        assert!(
            surface_capabilities
                .usages
                .contains(wgpu::TextureUsages::COPY_SRC),
            "the selected surface does not support screenshot readback"
        );
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let depth_view = create_depth_view(&device, size);

        let camera = planet::OrbitCamera::default();
        let initial_camera_world_position = camera.world_position();
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera projection"),
            size: size_of::<planet::CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let terrain = terrain::TerrainRenderer::new(
            &device,
            &queue,
            config.format,
            &camera_bind_group_layout,
            terrain_source,
        )
        .expect("terrain renderer must initialize");
        let atmosphere =
            atmosphere::AtmosphereRenderer::new(&device, config.format, &camera_bind_group_layout);

        let egui_context = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth_view,
            atmosphere,
            terrain,
            terrain_stats: terrain::TerrainStats::default(),
            camera,
            sun_direction: glam::DVec3::new(0.4, 0.7, 0.6).normalize(),
            previous_camera_world_position: initial_camera_world_position,
            previous_sim_time: 0.0,
            last_auto_orbit_sim_time: 0.0,
            camera_buffer,
            camera_bind_group,
            started_at: Instant::now(),
            egui_context,
            egui_state,
            egui_renderer,
            last_frame: Instant::now(),
            fps: 0.0,
            debug_overlay_visible: true,
            manual_screenshot_requested: false,
            next_log_time: 0.0,
            capture_number: 0,
            scenario,
            artifacts,
            scenario_capture_failed: false,
            mouse_captured: false,
            profile_render,
            gpu_profiler,
            cached_paint_jobs: Vec::new(),
            egui_buffers_dirty: true,
            next_hud_update: Instant::now(),
            hud_dirty: true,
        }
    }

    fn resize(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, size);
        self.egui_buffers_dirty = true;
        self.mark_hud_dirty();
    }

    fn rotate_camera(&mut self, azimuth_delta: f64, elevation_delta: f64) {
        self.camera.orbit(azimuth_delta, elevation_delta);
    }

    fn look_camera(&mut self, yaw_delta: f64, pitch_delta: f64) {
        self.camera.look(yaw_delta, pitch_delta);
    }

    fn zoom_camera(&mut self, wheel_delta: f64) {
        self.camera.zoom(wheel_delta);
    }

    fn set_mouse_capture(&mut self, window: &Window, captured: bool) {
        if captured {
            let result = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            self.mouse_captured = result.is_ok();
            window.set_cursor_visible(!self.mouse_captured);
            if let Err(error) = result {
                tracing::warn!(%error, "cursor capture is unavailable");
            }
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.mouse_captured = false;
        }
    }

    fn mark_hud_dirty(&mut self) {
        self.hud_dirty = true;
    }

    fn toggle_debug_overlay(&mut self) {
        self.debug_overlay_visible = !self.debug_overlay_visible;
        self.cached_paint_jobs.clear();
        self.egui_buffers_dirty = self.debug_overlay_visible;
        self.hud_dirty = self.debug_overlay_visible;
        self.next_hud_update = Instant::now()
            + if self.debug_overlay_visible {
                Duration::ZERO
            } else {
                HIDDEN_REFRESH_INTERVAL
            };
    }

    fn flush_gpu_profile(&mut self) {
        if self.gpu_profiler.is_none() {
            return;
        }
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        });
        let completed = self
            .gpu_profiler
            .as_mut()
            .expect("GPU profiler exists")
            .collect_completed(&self.device);
        for (sample_time, gpu_render_ms) in completed {
            self.artifacts
                .record_gpu_timestamp(sample_time, gpu_render_ms);
        }
    }

    fn render(&mut self, window: &Window) -> Option<bool> {
        let profile_started = Instant::now();
        let now = Instant::now();
        let completed_gpu_samples = self
            .gpu_profiler
            .as_mut()
            .map(|profiler| profiler.collect_completed(&self.device))
            .unwrap_or_default();
        for (sample_time, gpu_render_ms) in completed_gpu_samples {
            self.artifacts
                .record_gpu_timestamp(sample_time, gpu_render_ms);
        }
        let frame_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if frame_time > 0.0 {
            self.fps = 1.0 / frame_time;
        }

        let (
            sim_time,
            write_log,
            scenario_capture,
            scenario_complete,
            solid_color_screen,
            hide_overlay,
            seam_gap_check,
            scenario_pose,
            scenario_sun_direction,
            scenario_planet_rotation_time_scale,
        ) = if let Some(scenario) = self.scenario.as_mut() {
            let frame = scenario.advance();
            let solid_color_screen = scenario.renders_solid_color();
            let scenario_pose = (!solid_color_screen).then(|| {
                (
                    glam::DVec3::from_array(frame.camera_world_position),
                    glam::DVec3::from_array(frame.camera_look_at),
                )
            });
            (
                frame.sim_time,
                frame.write_log,
                frame.capture_screenshot,
                frame.complete,
                solid_color_screen,
                scenario.hides_overlay(),
                scenario.needs_seam_gap_check(),
                scenario_pose,
                Some(glam::DVec3::from_array(frame.sun_direction)),
                frame.planet_rotation_time_scale,
            )
        } else {
            let sim_time = self.started_at.elapsed().as_secs_f64();
            let write_log = sim_time >= self.next_log_time;
            if write_log {
                self.next_log_time = sim_time + 0.5;
            }
            (
                sim_time, write_log, false, false, false, false, false, None, None, 1.0,
            )
        };
        if let Some((position, look_at)) = scenario_pose {
            self.camera.set_world_pose(position, look_at);
        }
        if let Some(sun_direction) = scenario_sun_direction {
            self.sun_direction = sun_direction.normalize();
        }
        if self.scenario.is_none() {
            let orbit_delta_seconds = (sim_time - self.last_auto_orbit_sim_time).max(0.0);
            self.camera.orbit(
                DEFAULT_CAMERA_ORBIT_RADIANS_PER_SECOND * orbit_delta_seconds,
                0.0,
            );
        }
        self.last_auto_orbit_sim_time = sim_time;
        let camera_world_position = self.camera.world_position();
        let planet_rotation_radians =
            planet::planet_rotation_radians(sim_time * scenario_planet_rotation_time_scale);
        let camera_planet_frame_position = self
            .camera
            .planet_frame_world_position(planet_rotation_radians);
        let camera_radius = camera_world_position.length();
        let camera_altitude = camera_radius - planet::PLANET_RADIUS_METERS;
        let delta_sim_time = (sim_time - self.previous_sim_time).max(f64::EPSILON);
        let velocity_meters_per_second =
            camera_world_position.distance(self.previous_camera_world_position) / delta_sim_time;
        self.previous_camera_world_position = camera_world_position;
        self.previous_sim_time = sim_time;
        self.terrain_stats = if solid_color_screen {
            terrain::TerrainStats::default()
        } else {
            self.terrain
                .update(
                    camera_planet_frame_position,
                    [self.size.width, self.size.height],
                    self.camera.vertical_fov_radians(),
                )
                .unwrap_or_else(|error| panic!("terrain update failed: {error}"))
        };
        let draw_calls = if solid_color_screen {
            0
        } else {
            self.terrain_stats.draw_calls
        };
        if write_log {
            let latitude_degrees = (camera_world_position.y / camera_radius)
                .clamp(-1.0, 1.0)
                .asin()
                .to_degrees();
            let longitude_degrees = camera_world_position
                .z
                .atan2(camera_world_position.x)
                .to_degrees();
            self.artifacts
                .record_spatial_sample(debug::SpatialLogSample {
                    sim_time,
                    camera_world_position: camera_world_position.to_array(),
                    latitude_degrees,
                    longitude_degrees,
                    altitude_meters: camera_altitude,
                    velocity_meters_per_second,
                    orientation: if self.scenario.is_some() {
                        "waypoint".to_owned()
                    } else {
                        "free_look".to_owned()
                    },
                    orientation_azimuth_radians: self.camera.azimuth_radians,
                    orientation_elevation_radians: self.camera.elevation_radians,
                    sun_direction: self.sun_direction.to_array(),
                    planet_rotation_radians,
                    lod_level_histogram: self.terrain_stats.level_histogram,
                    chunks_loaded: self.terrain_stats.chunks_loaded,
                    chunks_unloaded: self.terrain_stats.chunks_unloaded,
                    frame_time_ms: frame_time * 1000.0,
                    draw_calls,
                    max_seam_delta_m: self.terrain_stats.max_seam_delta_meters,
                    resident_chunks: self.terrain_stats.resident_chunks,
                    fallback_chunks: self.terrain_stats.fallback_chunks,
                    resident_tiles: self.terrain_stats.resident_tiles,
                    tiles_loaded: self.terrain_stats.tiles_loaded,
                    tiles_unloaded: self.terrain_stats.tiles_unloaded,
                    lod_thrash_events: self.terrain_stats.lod_thrash_events,
                });
        }
        let simulation_ms = profile_started.elapsed().as_secs_f32() * 1_000.0;

        let mut textures_to_free = Vec::new();
        let render_egui = !solid_color_screen && !hide_overlay && self.debug_overlay_visible;
        let refresh_egui = render_egui && (self.hud_dirty || now >= self.next_hud_update);
        if refresh_egui {
            let raw_input = self.egui_state.take_egui_input(window);
            let show_debug_overlay = self.debug_overlay_visible;
            let fps = self.fps;
            let camera_position = camera_world_position;
            let camera_direction = self.camera.direction();
            let vertical_fov_degrees = self.camera.vertical_fov_radians().to_degrees();
            let terrain_stats = self.terrain_stats.clone();
            let full_output = self.egui_context.run_ui(raw_input, |ui| {
                if show_debug_overlay {
                    let context = ui.ctx().clone();
                    egui::Window::new("Cat in the Garden")
                        .default_pos([12.0, 12.0])
                        .show(&context, |ui| {
                            ui.label("Quadtree terrain renderer");
                            ui.label(format!("Render FPS: {fps:.0}"));
                            ui.label(format!(
                                "Camera: [{:.0}, {:.0}, {:.0}] m",
                                camera_position.x, camera_position.y, camera_position.z
                            ));
                            ui.label(format!(
                                "Direction: [{:.3}, {:.3}, {:.3}]",
                                camera_direction.x, camera_direction.y, camera_direction.z
                            ));
                            ui.label(format!(
                                "Altitude: {camera_altitude:.0} m  |  LOD: {}  |  Chunks: {}",
                                terrain_stats.max_level, terrain_stats.resident_chunks
                            ));
                            ui.label(format!(
                                "Optical zoom: {:.1}\u{00b0} vertical FOV",
                                vertical_fov_degrees
                            ));
                            ui.label(format!(
                                "Tiles: {}  |  Fallback chunks: {}  |  Seam: {:.4} m",
                                terrain_stats.resident_tiles,
                                terrain_stats.fallback_chunks,
                                terrain_stats.max_seam_delta_meters
                            ));
                            ui.label("F3: toggle overlay  |  F12: capture PNG");
                            ui.label("Default: auto-orbit  |  Mouse: free look  |  Wheel: optical zoom  |  Esc/Q: quit");
                        });
                }
            });
            self.egui_state
                .handle_platform_output(window, full_output.platform_output);
            for (texture_id, image_delta) in &full_output.textures_delta.set {
                self.egui_renderer.update_texture(
                    &self.device,
                    &self.queue,
                    *texture_id,
                    image_delta,
                );
            }
            textures_to_free = full_output.textures_delta.free;
            self.cached_paint_jobs = self
                .egui_context
                .tessellate(full_output.shapes, self.egui_context.pixels_per_point());
            self.egui_buffers_dirty = true;
            self.next_hud_update = now + HUD_REFRESH_INTERVAL;
            self.hud_dirty = false;
        }
        let paint_jobs = render_egui.then_some(&self.cached_paint_jobs);
        if !self.debug_overlay_visible {
            self.hud_dirty = false;
            self.next_hud_update = now + HIDDEN_REFRESH_INTERVAL;
        }
        let egui_ms = profile_started.elapsed().as_secs_f32() * 1_000.0 - simulation_ms;
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.size.width, self.size.height],
            pixels_per_point: window.scale_factor() as f32,
        };

        let mut reconfigure_surface = false;
        let surface_acquire_started = Instant::now();
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) => output,
            wgpu::CurrentSurfaceTexture::Suboptimal(output) => {
                reconfigure_surface = true;
                output
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.resize(self.size);
                return None;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return None,
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let surface_acquire_ms = surface_acquire_started.elapsed().as_secs_f32() * 1_000.0;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render encoder"),
            });
        let gpu_slot_index = if self.profile_render && write_log {
            self.gpu_profiler
                .as_mut()
                .and_then(GpuProfiler::acquire_slot)
        } else {
            None
        };
        let egui_upload_started = Instant::now();
        if self.egui_buffers_dirty
            && let Some(paint_jobs) = &paint_jobs
        {
            self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                paint_jobs,
                &screen_descriptor,
            );
            self.egui_buffers_dirty = false;
        }
        let egui_upload_ms = egui_upload_started.elapsed().as_secs_f32() * 1_000.0;

        let upload_started = Instant::now();
        let camera_uniform = planet::CameraUniform::from_camera(
            &self.camera,
            self.size.width as f32 / self.size.height as f32,
            self.sun_direction,
            planet_rotation_radians,
        );
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        let vertex_rebase_ms = 0.0;
        let vertex_upload_ms = upload_started.elapsed().as_secs_f32() * 1_000.0;
        let encode_started = Instant::now();
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cube-sphere pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: gpu_slot_index.map(|slot_index| {
                    let profiler = self.gpu_profiler.as_ref().expect("GPU profiler exists");
                    wgpu::RenderPassTimestampWrites {
                        query_set: &profiler.slots[slot_index].query_set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }
                }),
                multiview_mask: None,
            });
            if !solid_color_screen {
                self.atmosphere
                    .draw(&mut render_pass, &self.camera_bind_group);
                self.terrain.draw(&mut render_pass, &self.camera_bind_group);
            }
        }
        if let Some(paint_jobs) = &paint_jobs {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.egui_renderer.render(
                &mut render_pass.forget_lifetime(),
                paint_jobs,
                &screen_descriptor,
            );
        }

        let capture_requested = self.manual_screenshot_requested || scenario_capture;
        self.manual_screenshot_requested = false;
        let pending_capture = capture_requested.then(|| {
            self.capture_number += 1;
            debug::schedule_capture(
                &self.device,
                &mut encoder,
                &output.texture,
                self.size.width,
                self.size.height,
                self.config.format,
                self.capture_number,
            )
        });

        let encode_ms = encode_started.elapsed().as_secs_f32() * 1_000.0;

        if let Some(slot_index) = gpu_slot_index {
            let profiler = self.gpu_profiler.as_ref().expect("GPU profiler exists");
            let slot = &profiler.slots[slot_index];
            encoder.resolve_query_set(&slot.query_set, 0..2, &slot.resolve_buffer, 0);
            encoder.copy_buffer_to_buffer(&slot.resolve_buffer, 0, &slot.readback_buffer, 0, 16);
        }

        let submit_started = Instant::now();
        self.queue.submit(Some(encoder.finish()));
        let submit_ms = submit_started.elapsed().as_secs_f32() * 1_000.0;
        let gpu_readback_started = Instant::now();
        if let Some(slot_index) = gpu_slot_index {
            self.gpu_profiler
                .as_mut()
                .expect("GPU profiler exists")
                .begin_readback(slot_index, sim_time);
        }
        let gpu_timestamp_readback_ms = gpu_readback_started.elapsed().as_secs_f32() * 1_000.0;
        let present_started = Instant::now();
        output.present();
        let present_ms = present_started.elapsed().as_secs_f32() * 1_000.0;
        let capture_started = Instant::now();
        if let Some(pending_capture) = pending_capture {
            if let Err(error) = debug::finish_capture(
                &self.device,
                pending_capture,
                &mut self.artifacts,
                sim_time,
                solid_color_screen,
                seam_gap_check,
            ) {
                self.scenario_capture_failed = true;
                tracing::error!(%error, "screenshot capture failed");
            }
        }
        let capture_readback_ms = capture_started.elapsed().as_secs_f32() * 1_000.0;
        if self.profile_render && write_log {
            self.artifacts.record_render_profile(
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
                -1.0,
                gpu_timestamp_readback_ms,
                profile_started.elapsed().as_secs_f32() * 1_000.0,
            );
        }

        for texture_id in &textures_to_free {
            self.egui_renderer.free_texture(texture_id);
        }

        if reconfigure_surface {
            self.resize(self.size);
        }

        if scenario_complete {
            self.flush_gpu_profile();
            let expected_screenshots = self
                .scenario
                .as_ref()
                .map_or(0, scenario::ScenarioRunner::expected_screenshots);
            let harness_passed = !self.scenario_capture_failed
                && self.artifacts.screenshot_count() == expected_screenshots
                && self.artifacts.spatial_log_count()
                    >= self
                        .scenario
                        .as_ref()
                        .map_or(0, scenario::ScenarioRunner::expected_log_samples);
            let passed = self.artifacts.final_passed(harness_passed);
            self.artifacts.finish(harness_passed).unwrap_or_else(
                |error| tracing::error!(%error, "could not finalize test-run manifest"),
            );
            tracing::info!(passed, "scenario completed");
            return Some(passed);
        }

        None
    }
}

fn main() {
    let launch_options = launch_options().unwrap_or_else(|error| panic!("{error}"));
    let scenario_failed = Arc::new(AtomicBool::new(false));
    let scenario_failed_in_loop = scenario_failed.clone();
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let window = Arc::new(
        event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("Cat in the Garden")
                    .with_inner_size(winit::dpi::LogicalSize::new(960.0, 640.0)),
            )
            .expect("failed to create window"),
    );
    let mut state = pollster::block_on(State::new(
        window.clone(),
        launch_options.scenario_name,
        launch_options.profile_render,
        launch_options.terrain_source,
    ));
    state.set_mouse_capture(&window, true);

    event_loop
        .run(move |event, event_loop| match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => {
                if matches!(
                    &event,
                    WindowEvent::KeyboardInput { event, .. }
                        if event.state.is_pressed()
                            && matches!(
                                event.physical_key,
                                PhysicalKey::Code(KeyCode::Escape | KeyCode::KeyQ)
                            )
                ) {
                    event_loop.exit();
                    return;
                }
                let egui_response = state.egui_state.on_window_event(&window, &event);
                if egui_response.repaint && !matches!(&event, WindowEvent::RedrawRequested) {
                    window.request_redraw();
                }

                if !egui_response.consumed {
                    match event {
                        WindowEvent::CloseRequested => event_loop.exit(),
                        WindowEvent::Focused(focused) => state.set_mouse_capture(&window, focused),
                        WindowEvent::Resized(size) => state.resize(size),
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::F3) =>
                        {
                            state.toggle_debug_overlay();
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::F12) =>
                        {
                            state.manual_screenshot_requested = true;
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowLeft) =>
                        {
                            state.rotate_camera(-0.08, 0.0);
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowRight) =>
                        {
                            state.rotate_camera(0.08, 0.0);
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowUp) =>
                        {
                            state.rotate_camera(0.0, 0.05);
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowDown) =>
                        {
                            state.rotate_camera(0.0, -0.05);
                            window.request_redraw();
                        }
                        WindowEvent::MouseWheel { delta, .. } => {
                            let wheel_delta = match delta {
                                MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                                MouseScrollDelta::PixelDelta(position) => position.y / 80.0,
                            };
                            state.zoom_camera(wheel_delta);
                            window.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            if let Some(passed) = state.render(&window) {
                                scenario_failed_in_loop.store(!passed, Ordering::Relaxed);
                                event_loop.exit();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::DeviceEvent {
                event: DeviceEvent::MouseMotion { delta },
                ..
            } if state.mouse_captured => {
                state.look_camera(
                    delta.0 * MOUSE_LOOK_RADIANS_PER_PIXEL,
                    -delta.1 * MOUSE_LOOK_RADIANS_PER_PIXEL,
                );
                window.request_redraw();
            }
            Event::AboutToWait => {
                // Present mode is FIFO, so this remains display-paced while the cached HUD
                // refreshes independently at its lower rate. Waiting for HUD refreshes here
                // incorrectly made an otherwise idle scene render at 10 FPS.
                event_loop.set_control_flow(ControlFlow::Poll);
                window.request_redraw();
            }
            _ => {}
        })
        .expect("event loop failed");

    if scenario_failed.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}

struct LaunchOptions {
    scenario_name: Option<String>,
    profile_render: bool,
    terrain_source: terrain::TerrainSource,
}

fn launch_options() -> Result<LaunchOptions, String> {
    let default_outmap = PathBuf::from(DEFAULT_OUTMAP_PATH);
    let mut options = LaunchOptions {
        scenario_name: None,
        profile_render: false,
        terrain_source: if default_outmap.join("manifest.json").is_file() {
            terrain::TerrainSource::Outmap(default_outmap.clone())
        } else {
            terrain::TerrainSource::Placeholder
        },
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--scenario" => {
                options.scenario_name = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--scenario requires a scenario name".to_owned())?,
                )
            }
            "--profile-render" => options.profile_render = true,
            "--terrain" => {
                options.terrain_source = match arguments
                    .next()
                    .ok_or_else(|| "--terrain requires 'placeholder' or 'outmap'".to_owned())?
                    .as_str()
                {
                    "placeholder" => terrain::TerrainSource::Placeholder,
                    "outmap" => terrain::TerrainSource::Outmap(default_outmap.clone()),
                    value => return Err(format!("unsupported terrain source '{value}'")),
                };
            }
            "--outmap" => {
                options.terrain_source = terrain::TerrainSource::Outmap(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--outmap requires a path".to_owned())?,
                ));
            }
            _ => return Err(format!("unrecognized argument '{flag}'")),
        }
    }
    Ok(options)
}

fn create_depth_view(
    device: &wgpu::Device,
    size: winit::dpi::PhysicalSize<u32>,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("reversed-z depth texture"),
            size: wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

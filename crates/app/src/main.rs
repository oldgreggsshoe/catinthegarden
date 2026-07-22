mod atmosphere;
mod debug;
mod foveated;
mod hdr;
mod ocean;
mod outmap;
mod planet;
mod scenario;
mod sun;
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
    application::ApplicationHandler,
    event::{DeviceEvent, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Fullscreen, Window, WindowAttributes, WindowId},
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
const GPU_TIMESTAMP_COUNT: u32 = 14;
const DEFAULT_OUTMAP_PATH: &str = "assets/outmaps/test-planet";

fn should_enter_fullscreen(currently_fullscreen: bool) -> bool {
    !currently_fullscreen
}

fn render_size_for_surface_resize(
    surface_size: winit::dpi::PhysicalSize<u32>,
    fullscreen_render_size: Option<winit::dpi::PhysicalSize<u32>>,
) -> winit::dpi::PhysicalSize<u32> {
    fullscreen_render_size.unwrap_or(surface_size)
}
const DEFAULT_VIEWPORT_WIDTH: u32 = 1280;
const DEFAULT_VIEWPORT_HEIGHT: u32 = 720;
const DEFAULT_CAMERA_ORBIT_RADIANS_PER_SECOND: f64 = 0.4;
const DEFAULT_CAMERA_ORBIT_INCLINATION_RADIANS: f64 = 28.5_f64.to_radians();
const INTERACTIVE_PLANET_ROTATION_TIME_SCALE: f64 = 0.3;
const MOUSE_LOOK_RADIANS_PER_PIXEL: f64 = 0.0006;
const LOW_FLIGHT_ALTITUDE_METERS: f64 = 5_000.0 * 0.3048;
/// Flight begins gently enough for surface inspection, then acceleration
/// doubles while a movement key remains held so the same controls can leave
/// the planet. Shift accelerates the ramp without changing its shape.
const LOW_FLIGHT_BASE_ACCELERATION_METERS_PER_SECOND_SQUARED: f64 = 50.0;
const LOW_FLIGHT_ACCELERATION_DOUBLING_SECONDS: f64 = 0.75;
const LOW_FLIGHT_BOOST_ACCELERATION_MULTIPLIER: f64 = 4.0;
const LOW_FLIGHT_MAX_ACCELERATION_METERS_PER_SECOND_SQUARED: f64 = 4_000_000.0;
const LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND: f64 = 8_000_000.0;
/// Releasing all movement keys halves speed every 80ms. This gives short taps
/// precise stopping while still allowing a brief, readable coast at speed.
const LOW_FLIGHT_RELEASE_BRAKE_HALF_LIFE_SECONDS: f64 = 0.08;
const LOW_FLIGHT_VERTICAL_FOV_DEGREES: f64 = 60.0;
/// Start with the landing site visibly below the horizon. A tangent view at
/// 5,000 ft spent most of the frame on atmosphere and made the finest sparse
/// terrain patch effectively invisible even though the camera was above it.
const LOW_FLIGHT_INITIAL_PITCH_RADIANS: f64 = -18.0_f64.to_radians();
/// Prevent a slow render frame from turning into a much larger terrain jump on
/// the next frame. This is a visual navigation mode rather than a physics
/// integrator, so bounded slowdown is preferable to a performance feedback
/// loop while boosted across streamed terrain.
const MAX_LOW_FLIGHT_FRAME_DELTA_SECONDS: f64 = 1.0 / 30.0;

fn adapter_preference(info: &wgpu::AdapterInfo) -> (u8, bool, bool) {
    let device_rank = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => 4,
        wgpu::DeviceType::IntegratedGpu => 3,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 1,
        wgpu::DeviceType::Cpu => 0,
    };
    (
        device_rank,
        info.vendor == 0x10de || info.name.to_ascii_lowercase().contains("nvidia"),
        info.backend == wgpu::Backend::Vulkan,
    )
}

async fn select_render_adapter(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> wgpu::Adapter {
    let mut adapters: Vec<_> = instance
        .enumerate_adapters(wgpu::Backends::all())
        .await
        .into_iter()
        .filter(|adapter| adapter.is_surface_supported(surface))
        .collect();
    if let Ok(requested_name) = std::env::var("WGPU_ADAPTER_NAME") {
        let requested_name = requested_name.to_ascii_lowercase();
        if let Some(index) = adapters.iter().position(|adapter| {
            adapter
                .get_info()
                .name
                .to_ascii_lowercase()
                .contains(&requested_name)
        }) {
            return adapters.swap_remove(index);
        }
        tracing::warn!(
            target: "catinthegarden::adapter",
            requested_name,
            "requested WGPU adapter is unavailable; using the best compatible adapter"
        );
    }
    adapters
        .into_iter()
        .max_by_key(|adapter| adapter_preference(&adapter.get_info()))
        .unwrap_or_else(|| {
            panic!("no surface-compatible GPU adapter found");
        })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FlightMovementInput {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    boost: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FlightSpeedState {
    speed_meters_per_second: f64,
    acceleration_time_seconds: f64,
}

fn advance_flight_speed(
    state: FlightSpeedState,
    movement_held: bool,
    boost: bool,
    delta_seconds: f64,
) -> FlightSpeedState {
    if delta_seconds <= 0.0 {
        return state;
    }
    if movement_held {
        let acceleration_time_seconds = state.acceleration_time_seconds + delta_seconds;
        let boost_multiplier = if boost {
            LOW_FLIGHT_BOOST_ACCELERATION_MULTIPLIER
        } else {
            1.0
        };
        let acceleration = (LOW_FLIGHT_BASE_ACCELERATION_METERS_PER_SECOND_SQUARED
            * 2.0_f64.powf(acceleration_time_seconds / LOW_FLIGHT_ACCELERATION_DOUBLING_SECONDS)
            * boost_multiplier)
            .min(LOW_FLIGHT_MAX_ACCELERATION_METERS_PER_SECOND_SQUARED);
        FlightSpeedState {
            speed_meters_per_second: (state.speed_meters_per_second + acceleration * delta_seconds)
                .min(LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND),
            acceleration_time_seconds,
        }
    } else {
        let speed_meters_per_second = state.speed_meters_per_second
            * 0.5_f64.powf(delta_seconds / LOW_FLIGHT_RELEASE_BRAKE_HALF_LIFE_SECONDS);
        FlightSpeedState {
            speed_meters_per_second: if speed_meters_per_second < 0.01 {
                0.0
            } else {
                speed_meters_per_second
            },
            acceleration_time_seconds: 0.0,
        }
    }
}

fn flight_movement_direction(
    input: FlightMovementInput,
    camera_forward: glam::DVec3,
    camera_right: glam::DVec3,
) -> Option<glam::DVec3> {
    let forward_amount = f64::from(i8::from(input.forward) - i8::from(input.backward));
    let right_amount = f64::from(i8::from(input.right) - i8::from(input.left));
    let movement = camera_forward * forward_amount + camera_right * right_amount;
    (movement.length_squared() > 0.0).then(|| movement.normalize())
}

/// Returns the initial tangent used by a planet-relative flight camera.
///
/// This longitude-derived value is safe only for initialization: longitude is
/// undefined at the poles, so an active flight camera transports this tangent
/// with its radial direction instead of rebuilding it each frame.
fn initial_flight_tangent(local_radial: glam::DVec3) -> glam::DVec3 {
    let surface_azimuth_radians = local_radial.z.atan2(local_radial.x);
    glam::DVec3::new(
        -surface_azimuth_radians.sin(),
        0.0,
        surface_azimuth_radians.cos(),
    )
}

/// Parallel-transports a local tangent over the sphere as the camera moves.
///
/// Unlike recomputing a tangent from longitude, this keeps the camera frame
/// continuous while crossing either pole.
fn transport_flight_tangent(
    local_tangent: glam::DVec3,
    previous_radial: glam::DVec3,
    next_radial: glam::DVec3,
) -> glam::DVec3 {
    let rotation_axis = previous_radial.cross(next_radial);
    let transported = if rotation_axis.length_squared() > f64::EPSILON {
        let angle = rotation_axis
            .length()
            .atan2(previous_radial.dot(next_radial));
        glam::DQuat::from_axis_angle(rotation_axis.normalize(), angle).mul_vec3(local_tangent)
    } else {
        local_tangent
    };
    let tangent = transported - next_radial * transported.dot(next_radial);
    if tangent.length_squared() > f64::EPSILON {
        tangent.normalize()
    } else {
        initial_flight_tangent(next_radial)
    }
}

fn transport_flight_direction(
    direction: glam::DVec3,
    previous_radial: glam::DVec3,
    next_radial: glam::DVec3,
) -> glam::DVec3 {
    let rotation_axis = previous_radial.cross(next_radial);
    if rotation_axis.length_squared() <= f64::EPSILON {
        return direction;
    }
    let angle = rotation_axis
        .length()
        .atan2(previous_radial.dot(next_radial));
    glam::DQuat::from_axis_angle(rotation_axis.normalize(), angle)
        .mul_vec3(direction)
        .normalize()
}

fn flight_view_direction(
    local_radial: glam::DVec3,
    local_tangent: glam::DVec3,
    yaw_radians: f64,
    pitch_radians: f64,
) -> glam::DVec3 {
    let local_right = local_tangent.cross(local_radial).normalize();
    let horizontal = pitch_radians.cos();
    (local_tangent * (yaw_radians.cos() * horizontal)
        + local_right * (yaw_radians.sin() * horizontal)
        + local_radial * pitch_radians.sin())
    .normalize()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CameraMode {
    Orbit,
    LowFlight,
}

impl CameraMode {
    fn label(self) -> &'static str {
        match self {
            Self::Orbit => "orbit",
            Self::LowFlight => "accelerating WASD flight (Shift: 4x acceleration)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum RenderPath {
    #[default]
    Raster,
    FoveatedRay,
}

impl RenderPath {
    fn toggled(self) -> Self {
        match self {
            Self::Raster => Self::FoveatedRay,
            Self::FoveatedRay => Self::Raster,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Raster => "raster",
            Self::FoveatedRay => "foveated ray",
        }
    }
}

/// Scene time intentionally stops under F10, but low-flight navigation remains
/// responsive so frozen composition diagnostics can be framed in place.
fn interactive_camera_delta_seconds(
    camera_mode: CameraMode,
    scene_delta_seconds: f64,
    frame_delta_seconds: f64,
) -> f64 {
    match camera_mode {
        CameraMode::Orbit => scene_delta_seconds,
        CameraMode::LowFlight => frame_delta_seconds.min(MAX_LOW_FLIGHT_FRAME_DELTA_SECONDS),
    }
}

fn advance_flight_position_on_sphere(
    position: glam::DVec3,
    movement_direction: glam::DVec3,
    distance_meters: f64,
) -> glam::DVec3 {
    let radial = position.normalize();
    let radial_distance = movement_direction.dot(radial) * distance_meters;
    let tangent = movement_direction - radial * movement_direction.dot(radial);
    let next_radius = (position.length() + radial_distance).max(planet::PLANET_RADIUS_METERS);
    if tangent.length_squared() <= f64::EPSILON || distance_meters <= 0.0 {
        return radial * next_radius;
    }
    let tangent_direction = tangent.normalize();
    let rotation_axis = radial.cross(tangent_direction).normalize();
    let angular_distance = tangent.length() * distance_meters / next_radius;
    glam::DQuat::from_axis_angle(rotation_axis, angular_distance).mul_vec3(radial) * next_radius
}

fn format_vertical_fov(vertical_fov_degrees: f64) -> String {
    if vertical_fov_degrees >= 10.0 {
        format!("{vertical_fov_degrees:.1}")
    } else if vertical_fov_degrees >= 1.0 {
        format!("{vertical_fov_degrees:.2}")
    } else if vertical_fov_degrees >= 0.01 {
        format!("{vertical_fov_degrees:.3}")
    } else {
        format!("{vertical_fov_degrees:.6}")
    }
}

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

#[derive(Clone, Copy, Debug)]
struct GpuStageTimings {
    scene_ms: f64,
    luminance_ms: f64,
    sun_ms: f64,
    blur_ms: f64,
    bloom_ms: f64,
    tone_map_ms: f64,
    egui_ms: f64,
}

impl GpuStageTimings {
    fn total_ms(self) -> f64 {
        self.scene_ms
            + self.luminance_ms
            + self.sun_ms
            + self.blur_ms
            + self.bloom_ms
            + self.tone_map_ms
            + self.egui_ms
    }
}

impl GpuProfiler {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let slots = (0..GPU_PROFILE_RING_SIZE)
            .map(|_| GpuProfileSlot {
                query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("render timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: GPU_TIMESTAMP_COUNT,
                }),
                resolve_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("render timestamp resolve"),
                    size: u64::from(GPU_TIMESTAMP_COUNT) * 8,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                readback_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("render timestamp readback"),
                    size: u64::from(GPU_TIMESTAMP_COUNT) * 8,
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

    fn collect_completed(&mut self, device: &wgpu::Device) -> Vec<(f64, GpuStageTimings)> {
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
            let elapsed = |begin: usize, end: usize| {
                values[end].saturating_sub(values[begin]) as f64
                    * f64::from(self.timestamp_period_ns)
                    / 1_000_000.0
            };
            let timings = GpuStageTimings {
                scene_ms: elapsed(0, 1),
                luminance_ms: elapsed(2, 3),
                sun_ms: elapsed(4, 5),
                blur_ms: elapsed(6, 7),
                bloom_ms: elapsed(8, 9),
                tone_map_ms: elapsed(10, 11),
                egui_ms: elapsed(12, 13),
            };
            drop(timestamps);
            slot.readback_buffer.unmap();
            completed.push((sim_time, timings));
        }
        completed
    }
}

struct State {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// Internal HDR/depth/LOD resolution. This remains fixed while fullscreen.
    size: winit::dpi::PhysicalSize<u32>,
    /// Native swapchain size, which can be larger than the internal scene.
    surface_size: winit::dpi::PhysicalSize<u32>,
    fullscreen_render_size: Option<winit::dpi::PhysicalSize<u32>>,
    depth_view: wgpu::TextureView,
    hdr: hdr::HdrRenderer,
    atmosphere: atmosphere::AtmosphereRenderer,
    sun: sun::SunRenderer,
    foveated: foveated::FoveatedRenderer,
    terrain: terrain::TerrainRenderer,
    terrain_stats: terrain::TerrainStats,
    adapter_label: String,
    camera: planet::OrbitCamera,
    sun_direction: glam::DVec3,
    previous_camera_world_position: glam::DVec3,
    previous_sim_time: f64,
    last_auto_orbit_sim_time: f64,
    camera_mode: CameraMode,
    flight_local_position: glam::DVec3,
    flight_local_tangent: glam::DVec3,
    flight_surface_height_meters: f64,
    flight_look_yaw_radians: f64,
    flight_look_pitch_radians: f64,
    flight_movement: FlightMovementInput,
    flight_speed: FlightSpeedState,
    flight_travel_direction: glam::DVec3,
    saved_orbit_camera_pose: Option<(glam::DVec3, glam::DVec3, f64)>,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    started_at: Instant,
    egui_context: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    last_frame: Instant,
    fps: f32,
    debug_overlay_visible: bool,
    render_path: RenderPath,
    render_debug_mode: planet::RenderDebugMode,
    animation_frozen: bool,
    frozen_sim_time: f64,
    interactive_scene_time_offset_seconds: f64,
    manual_screenshot_requested: bool,
    next_spatial_log_presentation_time: f64,
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
        vertical_fov_degrees: Option<f64>,
        terrain_source: terrain::TerrainSource,
    ) -> Self {
        let mut scenario = scenario_name
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
        let adapter = select_render_adapter(&instance, &surface).await;
        let adapter_info = adapter.get_info();
        let adapter_label = format!(
            "{} ({:?}, {:?})",
            adapter_info.name, adapter_info.device_type, adapter_info.backend
        );
        tracing::info!(
            target: "catinthegarden::adapter",
            name = adapter_info.name,
            vendor = adapter_info.vendor,
            device = adapter_info.device,
            device_type = ?adapter_info.device_type,
            backend = ?adapter_info.backend,
            driver = adapter_info.driver,
            driver_info = adapter_info.driver_info,
            "selected render adapter"
        );
        if adapter_info.device_type != wgpu::DeviceType::DiscreteGpu {
            tracing::warn!(
                target: "catinthegarden::adapter",
                name = adapter_info.name,
                "no compatible discrete GPU is available; rendering on a non-discrete adapter"
            );
        }
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
        let hdr = hdr::HdrRenderer::new(&device, size, config.format);

        let mut camera = planet::OrbitCamera::default();
        if let Some(vertical_fov_degrees) = vertical_fov_degrees {
            camera.set_vertical_fov_degrees_for_viewport(vertical_fov_degrees, size.height);
        }
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
        let foveated = foveated::FoveatedRenderer::new(
            &device,
            &queue,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            terrain_source.clone(),
        )
        .expect("foveated renderer must initialize");
        let terrain = terrain::TerrainRenderer::new(
            &device,
            &queue,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            terrain_source,
        )
        .expect("terrain renderer must initialize");
        if let (Some(scenario), Some(landing_direction)) =
            (&mut scenario, terrain.preferred_landing_direction())
        {
            scenario.retarget_sparse_landing_direction(landing_direction);
        }
        let atmosphere = atmosphere::AtmosphereRenderer::new(
            &device,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
        );
        let sun = sun::SunRenderer::new(
            &device,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
        );

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
            surface_size: size,
            fullscreen_render_size: None,
            depth_view,
            hdr,
            atmosphere,
            sun,
            foveated,
            terrain,
            terrain_stats: terrain::TerrainStats::default(),
            adapter_label,
            camera,
            sun_direction: planet::default_sun_direction(),
            previous_camera_world_position: initial_camera_world_position,
            previous_sim_time: 0.0,
            last_auto_orbit_sim_time: 0.0,
            camera_mode: CameraMode::Orbit,
            flight_local_position: glam::DVec3::X
                * (planet::PLANET_RADIUS_METERS + LOW_FLIGHT_ALTITUDE_METERS),
            flight_local_tangent: glam::DVec3::Z,
            flight_surface_height_meters: 0.0,
            flight_look_yaw_radians: 0.0,
            flight_look_pitch_radians: 0.0,
            flight_movement: FlightMovementInput::default(),
            flight_speed: FlightSpeedState::default(),
            flight_travel_direction: glam::DVec3::ZERO,
            saved_orbit_camera_pose: None,
            camera_buffer,
            camera_bind_group,
            started_at: Instant::now(),
            egui_context,
            egui_state,
            egui_renderer,
            last_frame: Instant::now(),
            fps: 0.0,
            debug_overlay_visible: true,
            render_path: RenderPath::default(),
            render_debug_mode: planet::RenderDebugMode::Final,
            animation_frozen: false,
            frozen_sim_time: 0.0,
            interactive_scene_time_offset_seconds: 0.0,
            manual_screenshot_requested: false,
            next_spatial_log_presentation_time: 0.0,
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

        self.surface_size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.hdr.set_presentation_size(&self.queue, size);
        let render_size = render_size_for_surface_resize(size, self.fullscreen_render_size);
        if render_size != self.size {
            self.resize_render_targets(render_size);
        }
        self.egui_buffers_dirty = true;
        self.mark_hud_dirty();
    }

    fn resize_render_targets(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        self.size = size;
        self.camera
            .clamp_vertical_fov_for_viewport(self.size.height);
        self.depth_view = create_depth_view(&self.device, size);
        self.hdr.resize(&self.device, size);
    }

    fn toggle_fullscreen(&mut self, window: &Window) {
        let entering = should_enter_fullscreen(window.fullscreen().is_some());
        self.fullscreen_render_size = entering.then_some(self.size);
        window.set_fullscreen(entering.then_some(Fullscreen::Borderless(window.current_monitor())));
        self.mark_hud_dirty();
    }

    fn rotate_camera(&mut self, azimuth_delta: f64, elevation_delta: f64) {
        self.camera.orbit(azimuth_delta, elevation_delta);
    }

    fn look_camera(&mut self, yaw_delta: f64, pitch_delta: f64) {
        if self.camera_mode == CameraMode::LowFlight {
            let sensitivity = self.camera.look_sensitivity_scale();
            self.flight_look_yaw_radians += yaw_delta * sensitivity;
            self.flight_look_pitch_radians =
                (self.flight_look_pitch_radians + pitch_delta * sensitivity).clamp(-1.5, 1.5);
        } else {
            self.camera
                .look_with_optical_sensitivity(yaw_delta, pitch_delta);
        }
    }

    fn zoom_camera(&mut self, wheel_delta: f64) {
        self.camera.zoom_for_viewport(wheel_delta, self.size.height);
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
            self.flight_movement = FlightMovementInput::default();
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

    fn toggle_blur(&mut self) {
        self.hdr.set_effects(
            &self.device,
            !self.hdr.blur_enabled(),
            self.hdr.bloom_enabled(),
        );
        self.mark_hud_dirty();
    }

    fn toggle_bloom(&mut self) {
        self.hdr.set_effects(
            &self.device,
            self.hdr.blur_enabled(),
            !self.hdr.bloom_enabled(),
        );
        self.mark_hud_dirty();
    }

    fn toggle_hdr_effect(&mut self) {
        self.hdr
            .set_hdr_effect_enabled(&self.queue, !self.hdr.hdr_effect_enabled());
        self.mark_hud_dirty();
    }

    fn toggle_render_path(&mut self) {
        self.render_path = self.render_path.toggled();
        self.mark_hud_dirty();
    }

    fn cycle_render_debug_mode(&mut self) {
        self.render_debug_mode = self.render_debug_mode.next();
        self.mark_hud_dirty();
    }

    fn interactive_sim_time(&self) -> f64 {
        let elapsed_sim_time = self.started_at.elapsed().as_secs_f64();
        if self.animation_frozen {
            self.frozen_sim_time
        } else {
            elapsed_sim_time - self.interactive_scene_time_offset_seconds
        }
    }

    fn low_flight_view_direction(&self, local_radial: glam::DVec3) -> glam::DVec3 {
        flight_view_direction(
            local_radial,
            self.flight_local_tangent,
            self.flight_look_yaw_radians,
            self.flight_look_pitch_radians,
        )
    }

    fn set_flight_movement_key(&mut self, key_code: KeyCode, pressed: bool) -> bool {
        let movement_key = match key_code {
            KeyCode::KeyW => &mut self.flight_movement.forward,
            KeyCode::KeyS => &mut self.flight_movement.backward,
            KeyCode::KeyA => &mut self.flight_movement.left,
            KeyCode::KeyD => &mut self.flight_movement.right,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => &mut self.flight_movement.boost,
            _ => return false,
        };
        *movement_key = pressed;
        true
    }

    fn advance_low_flight_camera(&mut self, delta_seconds: f64, planet_rotation_radians: f64) {
        let local_radial = self.flight_local_position.normalize();
        let local_forward = self.low_flight_view_direction(local_radial);
        let local_right = local_forward.cross(local_radial).normalize();
        let movement_direction =
            flight_movement_direction(self.flight_movement, local_forward, local_right);
        self.flight_speed = advance_flight_speed(
            self.flight_speed,
            movement_direction.is_some(),
            self.flight_movement.boost,
            delta_seconds,
        );
        if let Some(movement_direction) = movement_direction {
            self.flight_travel_direction = movement_direction;
        }
        if self.flight_speed.speed_meters_per_second > 0.0
            && self.flight_travel_direction.length_squared() > 0.0
        {
            self.flight_local_position = advance_flight_position_on_sphere(
                self.flight_local_position,
                self.flight_travel_direction,
                self.flight_speed.speed_meters_per_second * delta_seconds,
            );
            let moved_radial = self.flight_local_position.normalize();
            self.flight_local_tangent =
                transport_flight_tangent(self.flight_local_tangent, local_radial, moved_radial);
            self.flight_travel_direction = transport_flight_direction(
                self.flight_travel_direction,
                local_radial,
                moved_radial,
            );
        }
        self.update_low_flight_camera(planet_rotation_radians);
    }

    fn update_low_flight_camera(&mut self, planet_rotation_radians: f64) {
        let local_radial = self.flight_local_position.normalize();
        let camera_altitude_meters =
            (self.flight_local_position.length() - planet::PLANET_RADIUS_METERS).max(0.0);
        if let Some(surface_height_meters) = self
            .terrain
            .surface_height_meters_at(local_radial, camera_altitude_meters)
        {
            self.flight_surface_height_meters = surface_height_meters;
        }
        // Terrain tiles can become resident while the camera is idle. Enforce
        // clearance every frame so a newly resolved higher surface cannot
        // leave the camera underground until the next movement key is pressed.
        let minimum_radius = planet::PLANET_RADIUS_METERS
            + self.flight_surface_height_meters
            + LOW_FLIGHT_ALTITUDE_METERS;
        if self.flight_local_position.length() < minimum_radius {
            self.flight_local_position = local_radial * minimum_radius;
        }
        let local_view_direction = self.low_flight_view_direction(local_radial);
        let planet_to_world = glam::DQuat::from_rotation_y(planet_rotation_radians);
        let world_position = planet_to_world.mul_vec3(self.flight_local_position);
        let world_direction = planet_to_world.mul_vec3(local_view_direction);
        let world_up = planet_to_world.mul_vec3(local_radial);
        self.camera.set_world_pose_with_up(
            world_position,
            world_position + world_direction,
            world_up,
        );
    }

    fn toggle_camera_mode(&mut self) {
        if self.scenario.is_some() {
            return;
        }

        let sim_time = self.interactive_sim_time();
        match self.camera_mode {
            CameraMode::Orbit => {
                let planet_rotation_radians = planet::planet_rotation_radians(
                    sim_time * INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
                );
                let local_position = planet::planet_local_vector(
                    self.camera.world_position(),
                    planet_rotation_radians,
                );
                self.saved_orbit_camera_pose = Some((
                    self.camera.world_position(),
                    self.camera.direction_dvec3(),
                    self.camera.vertical_fov_radians().to_degrees(),
                ));
                // Enter inspection mode at the baker-selected dry landing
                // site backed by the sparse high-resolution tile chain.
                let local_radial = self
                    .terrain
                    .preferred_landing_direction()
                    .unwrap_or_else(|| local_position.normalize());
                self.flight_surface_height_meters = self
                    .terrain
                    .surface_height_meters_at(local_radial, LOW_FLIGHT_ALTITUDE_METERS)
                    .unwrap_or(0.0);
                self.flight_local_position = local_radial
                    * (planet::PLANET_RADIUS_METERS
                        + self.flight_surface_height_meters
                        + LOW_FLIGHT_ALTITUDE_METERS);
                self.flight_local_tangent = initial_flight_tangent(local_radial);
                self.flight_look_yaw_radians = 0.0;
                self.flight_look_pitch_radians = LOW_FLIGHT_INITIAL_PITCH_RADIANS;
                self.flight_movement = FlightMovementInput::default();
                self.flight_speed = FlightSpeedState::default();
                self.flight_travel_direction = glam::DVec3::ZERO;
                self.camera_mode = CameraMode::LowFlight;
                self.camera.set_vertical_fov_degrees_for_viewport(
                    LOW_FLIGHT_VERTICAL_FOV_DEGREES,
                    self.size.height,
                );
                self.update_low_flight_camera(planet_rotation_radians);
            }
            CameraMode::LowFlight => {
                if let Some((position, direction, vertical_fov_degrees)) =
                    self.saved_orbit_camera_pose.take()
                {
                    self.camera.set_world_pose(position, position + direction);
                    self.camera.set_vertical_fov_degrees_for_viewport(
                        vertical_fov_degrees,
                        self.size.height,
                    );
                }
                self.flight_movement = FlightMovementInput::default();
                self.flight_speed = FlightSpeedState::default();
                self.flight_travel_direction = glam::DVec3::ZERO;
                self.camera_mode = CameraMode::Orbit;
            }
        }
        self.last_auto_orbit_sim_time = sim_time;
        self.mark_hud_dirty();
    }

    fn toggle_animation_freeze(&mut self) {
        if self.scenario.is_some() {
            return;
        }

        if self.animation_frozen {
            let elapsed_sim_time = self.started_at.elapsed().as_secs_f64();
            self.animation_frozen = false;
            // Keep all scene-time users continuous after a diagnostic pause.
            // In particular, neither the orbit nor planet rotation should jump
            // by the time spent taking screenshots.
            self.interactive_scene_time_offset_seconds = elapsed_sim_time - self.frozen_sim_time;
            self.last_auto_orbit_sim_time = self.frozen_sim_time;
        } else {
            self.frozen_sim_time = self.started_at.elapsed().as_secs_f64()
                - self.interactive_scene_time_offset_seconds;
            self.animation_frozen = true;
        }
        self.mark_hud_dirty();
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
        for (sample_time, timings) in completed {
            self.artifacts.record_gpu_timestamps(sample_time, timings);
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
        for (sample_time, timings) in completed_gpu_samples {
            self.artifacts.record_gpu_timestamps(sample_time, timings);
        }
        let frame_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if frame_time > 0.0 {
            self.fps = 1.0 / frame_time;
        }

        let (
            sim_time,
            presentation_time,
            write_log,
            scenario_capture,
            scenario_complete,
            solid_color_screen,
            hide_overlay,
            seam_gap_check,
            scenario_pose,
            scenario_vertical_fov_degrees,
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
                frame.sim_time,
                frame.write_log,
                frame.capture_screenshot,
                frame.complete,
                solid_color_screen,
                scenario.hides_overlay(),
                scenario.needs_seam_gap_check(),
                scenario_pose,
                frame.vertical_fov_degrees,
                Some(glam::DVec3::from_array(frame.sun_direction)),
                frame.planet_rotation_time_scale,
            )
        } else {
            let sim_time = self.interactive_sim_time();
            let presentation_time = self.started_at.elapsed().as_secs_f64();
            let write_log = presentation_time >= self.next_spatial_log_presentation_time;
            if write_log {
                self.next_spatial_log_presentation_time = presentation_time + 0.5;
            }
            (
                sim_time,
                presentation_time,
                write_log,
                false,
                false,
                false,
                false,
                false,
                None,
                None,
                None,
                INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
            )
        };
        if let Some((position, look_at)) = scenario_pose {
            if self
                .scenario
                .as_ref()
                .is_some_and(|scenario| scenario.name() == "low_flight_performance")
            {
                self.camera
                    .set_world_pose_with_up(position, look_at, position.normalize());
            } else {
                self.camera.set_world_pose(position, look_at);
            }
        }
        if let Some(vertical_fov_degrees) = scenario_vertical_fov_degrees {
            self.camera.set_reference_vertical_fov_degrees_for_viewport(
                vertical_fov_degrees,
                self.size.height,
            );
        }
        if let Some(sun_direction) = scenario_sun_direction {
            self.sun_direction = sun_direction.normalize();
        }
        let planet_rotation_radians =
            planet::planet_rotation_radians(sim_time * scenario_planet_rotation_time_scale);
        let scene_delta_seconds = (sim_time - self.last_auto_orbit_sim_time).max(0.0);
        if self.scenario.is_none() {
            let camera_delta_seconds = interactive_camera_delta_seconds(
                self.camera_mode,
                scene_delta_seconds,
                f64::from(frame_time),
            );
            match self.camera_mode {
                CameraMode::Orbit => self.camera.advance_inclined_orbit(
                    DEFAULT_CAMERA_ORBIT_RADIANS_PER_SECOND * camera_delta_seconds,
                    DEFAULT_CAMERA_ORBIT_INCLINATION_RADIANS,
                ),
                CameraMode::LowFlight => {
                    self.advance_low_flight_camera(camera_delta_seconds, planet_rotation_radians)
                }
            }
        }
        self.last_auto_orbit_sim_time = sim_time;
        let camera_world_position = self.camera.world_position();
        let camera_planet_frame_position = self
            .camera
            .planet_frame_world_position(planet_rotation_radians);
        let camera_planet_frame_direction = self
            .camera
            .planet_frame_direction_dvec3(planet_rotation_radians);
        let camera_planet_frame_up = self.camera.planet_frame_view_up(planet_rotation_radians);
        let camera_radius = camera_world_position.length();
        let camera_altitude =
            if self.scenario.is_none() && self.camera_mode == CameraMode::LowFlight {
                camera_radius - planet::PLANET_RADIUS_METERS - self.flight_surface_height_meters
            } else {
                camera_radius - planet::PLANET_RADIUS_METERS
            };
        let delta_sim_time = (sim_time - self.previous_sim_time).max(f64::EPSILON);
        let delta_camera_motion_seconds = if self.scenario.is_none() {
            f64::from(frame_time).max(f64::EPSILON)
        } else {
            delta_sim_time
        };
        let velocity_meters_per_second = camera_world_position
            .distance(self.previous_camera_world_position)
            / delta_camera_motion_seconds;
        self.previous_camera_world_position = camera_world_position;
        self.previous_sim_time = sim_time;
        self.hdr.collect_completed_luminance(&self.device);
        // Eye adaptation is a presentation effect, not simulation state. It
        // must continue to converge while F10 freezes planet animation.
        self.hdr.update_exposure(&self.queue, f64::from(frame_time));
        let exposure_state = self.hdr.exposure_state();
        self.artifacts.record_exposure_sample(
            sim_time,
            exposure_state.exposure,
            exposure_state.target_exposure,
            exposure_state.average_luminance,
        );
        self.terrain_stats = if solid_color_screen {
            terrain::TerrainStats::default()
        } else {
            self.terrain
                .update(
                    camera_planet_frame_position,
                    camera_planet_frame_direction,
                    camera_planet_frame_up,
                    presentation_time,
                    [self.size.width, self.size.height],
                    self.camera.vertical_fov_radians(),
                )
                .unwrap_or_else(|error| panic!("terrain update failed: {error}"))
        };
        self.artifacts.observe_lod_frame(
            &self.terrain_stats.level_histogram,
            self.terrain_stats.resident_chunks,
            self.terrain_stats.lod_thrash_events,
            self.terrain_stats.budget_limited,
        );
        let draw_calls = if solid_color_screen {
            0
        } else {
            self.terrain_stats.draw_calls
        };
        let ocean_wave_stats = ocean::wave_height_stats(sim_time);
        let ocean_wave_range = ocean_wave_stats.range_meters();
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
                    vertical_fov_degrees: self.camera.vertical_fov_radians().to_degrees(),
                    sun_direction: self.sun_direction.to_array(),
                    planet_rotation_radians,
                    lod_level_histogram: self.terrain_stats.level_histogram,
                    chunks_loaded: self.terrain_stats.chunks_loaded,
                    chunks_unloaded: self.terrain_stats.chunks_unloaded,
                    frame_time_ms: frame_time * 1000.0,
                    draw_calls,
                    max_seam_delta_m: self.terrain_stats.max_seam_delta_meters,
                    resident_chunks: self.terrain_stats.resident_chunks,
                    drawn_chunks: self.terrain_stats.drawn_chunks,
                    terrain_triangles: self.terrain_stats.terrain_triangles,
                    fallback_chunks: self.terrain_stats.fallback_chunks,
                    source_level_delta_histogram: self.terrain_stats.source_level_delta_histogram,
                    resident_tiles: self.terrain_stats.resident_tiles,
                    tiles_loaded: self.terrain_stats.tiles_loaded,
                    tiles_unloaded: self.terrain_stats.tiles_unloaded,
                    lod_thrash_events: self.terrain_stats.lod_thrash_events,
                    budget_limited: self.terrain_stats.budget_limited,
                    exposure: exposure_state.exposure,
                    ocean_wave_min_meters: ocean_wave_stats.minimum_meters,
                    ocean_wave_max_meters: ocean_wave_stats.maximum_meters,
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
            let exposure = exposure_state.exposure;
            let average_luminance = exposure_state.average_luminance;
            let ocean_wave_range = ocean_wave_range;
            let blur_enabled = self.hdr.blur_enabled();
            let bloom_enabled = self.hdr.bloom_enabled();
            let render_path = self.render_path;
            let render_debug_mode = self.render_debug_mode;
            let animation_frozen = self.animation_frozen;
            let camera_mode = self.camera_mode;
            let flight_speed_meters_per_second = self.flight_speed.speed_meters_per_second;
            let adapter_label = self.adapter_label.clone();
            let terrain_stats = self.terrain_stats.clone();
            let minimum_lod_level = terrain_stats
                .level_histogram
                .iter()
                .position(|count| *count > 0)
                .unwrap_or(0);
            let lod_range = if minimum_lod_level == usize::from(terrain_stats.max_level) {
                format!("L{}", terrain_stats.max_level)
            } else {
                format!("L{minimum_lod_level}-L{}", terrain_stats.max_level)
            };
            let vertical_fov_label = format_vertical_fov(vertical_fov_degrees);
            let full_output = self.egui_context.run_ui(raw_input, |ui| {
                if show_debug_overlay {
                    let context = ui.ctx().clone();
                    egui::Window::new("Cat in the Garden")
                        .default_pos([12.0, 12.0])
                        .show(&context, |ui| {
                            ui.label("Quadtree terrain renderer");
                            ui.label(format!("GPU: {adapter_label}"));
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
                                "Altitude: {camera_altitude:.0} m  |  LOD: {lod_range}"
                            ));
                            ui.label(format!(
                                "Terrain: {} active  |  {} drawn  |  {} triangles  |  {} draws",
                                terrain_stats.resident_chunks,
                                terrain_stats.drawn_chunks,
                                terrain_stats.terrain_triangles,
                                terrain_stats.draw_calls,
                            ));
                            ui.label(format!("Camera mode: {}", camera_mode.label()));
                            if camera_mode == CameraMode::LowFlight {
                                ui.label(format!(
                                    "Flight speed: {flight_speed_meters_per_second:.0} m/s"
                                ));
                            }
                            ui.label(format!(
                                "Optical zoom: {vertical_fov_label}\u{00b0} vertical FOV"
                            ));
                            ui.label(format!(
                                "Tiles: {}  |  Fallback chunks: {}  |  Seam: {:.4} m",
                                terrain_stats.resident_tiles,
                                terrain_stats.fallback_chunks,
                                terrain_stats.max_seam_delta_meters
                            ));
                            ui.label(format!(
                                "LOD work: {} splits  |  {} merges  |  {} culled",
                                terrain_stats.splits, terrain_stats.merges, terrain_stats.culled_nodes
                            ));
                            ui.label(format!(
                                "Exposure: {exposure:.3}  |  Average luminance: {average_luminance:.3}"
                            ));
                            ui.label(format!(
                                "Post: blur {}  |  bloom {}",
                                if blur_enabled { "on" } else { "off" },
                                if bloom_enabled { "on" } else { "off" },
                            ));
                            ui.label(format!(
                                "Composition debug: {}",
                                render_debug_mode.label(),
                            ));
                            ui.label(format!("Render path: {} (F5)", render_path.label()));
                            ui.label(format!(
                                "Animation: {}",
                                if animation_frozen { "frozen" } else { "running" },
                            ));
                            ui.label(format!("Ocean Gerstner range: {ocean_wave_range:.2} m"));
                            ui.label(
                                "F: fullscreen  |  F3: overlay  |  F4: camera mode  |  F5: render path  |  WASD: fly  |  F6: blur  |  F7: bloom  |  F8: HDR  |  F9: composition  |  F10: freeze  |  F12: capture PNG",
                            );
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
            size_in_pixels: [self.surface_size.width, self.surface_size.height],
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
                self.resize(self.surface_size);
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
            sim_time,
            self.render_debug_mode,
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
                    view: self.hdr.scene_view(),
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
                        store: wgpu::StoreOp::Store,
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
            if !solid_color_screen && self.render_path == RenderPath::Raster {
                self.atmosphere
                    .draw(&mut render_pass, &self.camera_bind_group);
                if self.render_debug_mode != planet::RenderDebugMode::SkyOnly {
                    self.terrain.draw(&mut render_pass, &self.camera_bind_group);
                }
            } else if !solid_color_screen && self.render_path == RenderPath::FoveatedRay {
                self.foveated
                    .draw_debug(&mut render_pass, &self.camera_bind_group);
            }
        }
        let timestamp_query_set = gpu_slot_index.map(|slot_index| {
            &self
                .gpu_profiler
                .as_ref()
                .expect("GPU profiler exists")
                .slots[slot_index]
                .query_set
        });
        self.hdr.encode_luminance(
            &mut encoder,
            timestamp_query_set.map(|query_set| (query_set, 2, 3)),
        );
        let hdr_luminance_readback_slot = self.hdr.encode_luminance_readback(&mut encoder);
        // The disc and corona are a camera-only visual aid. Composite them
        // after the meter has sampled the physical atmosphere/terrain scene so
        // their terrain occlusion cannot drive a false exposure rebound at
        // sunset. They remain HDR input for bloom and tone mapping below.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("visual sun overlay pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.hdr.scene_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: timestamp_query_set.map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(4),
                        end_of_pass_write_index: Some(5),
                    }
                }),
                multiview_mask: None,
            });
            if !solid_color_screen && self.render_debug_mode != planet::RenderDebugMode::SkyOnly {
                self.sun.draw(&mut render_pass, &self.camera_bind_group);
            }
        }
        self.hdr.encode_blur(
            &mut encoder,
            timestamp_query_set.map(|query_set| (query_set, 6, 7)),
        );
        self.hdr.encode_bloom(
            &mut encoder,
            timestamp_query_set.map(|query_set| (query_set, 8, 9)),
        );
        self.hdr.encode_tone_map(
            &mut encoder,
            &view,
            timestamp_query_set.map(|query_set| (query_set, 10, 11)),
        );
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
                timestamp_writes: timestamp_query_set.map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(12),
                        end_of_pass_write_index: Some(13),
                    }
                }),
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
                self.surface_size.width,
                self.surface_size.height,
                self.config.format,
                self.capture_number,
            )
        });

        let encode_ms = encode_started.elapsed().as_secs_f32() * 1_000.0;

        if let Some(slot_index) = gpu_slot_index {
            let profiler = self.gpu_profiler.as_ref().expect("GPU profiler exists");
            let slot = &profiler.slots[slot_index];
            let byte_size = u64::from(GPU_TIMESTAMP_COUNT) * 8;
            encoder.resolve_query_set(
                &slot.query_set,
                0..GPU_TIMESTAMP_COUNT,
                &slot.resolve_buffer,
                0,
            );
            encoder.copy_buffer_to_buffer(
                &slot.resolve_buffer,
                0,
                &slot.readback_buffer,
                0,
                byte_size,
            );
        }

        let submit_started = Instant::now();
        self.queue.submit(Some(encoder.finish()));
        let submit_ms = submit_started.elapsed().as_secs_f32() * 1_000.0;
        if let Some(slot_index) = hdr_luminance_readback_slot {
            self.hdr.begin_luminance_readback(slot_index);
        }
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
            self.resize(self.surface_size);
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
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App::new(launch_options);
    event_loop.run_app(&mut app).expect("event loop failed");
    if app.scenario_failed.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}

struct App {
    launch_options: LaunchOptions,
    scenario_failed: Arc<AtomicBool>,
    window: Option<Arc<Window>>,
    state: Option<State>,
}

impl App {
    fn new(launch_options: LaunchOptions) -> Self {
        Self {
            launch_options,
            scenario_failed: Arc::new(AtomicBool::new(false)),
            window: None,
            state: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("Cat in the Garden")
                        .with_inner_size(winit::dpi::PhysicalSize::new(
                            DEFAULT_VIEWPORT_WIDTH,
                            DEFAULT_VIEWPORT_HEIGHT,
                        )),
                )
                .expect("failed to create window"),
        );
        let mut state = pollster::block_on(State::new(
            window.clone(),
            self.launch_options.scenario_name.clone(),
            self.launch_options.profile_render,
            self.launch_options.vertical_fov_degrees,
            self.launch_options.terrain_source.clone(),
        ));
        state.set_mouse_capture(&window, true);
        self.state = Some(state);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window_id != window.id() {
            return;
        }
        let state = self.state.as_mut().expect("state initialized with window");
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
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(key_code) = event.physical_key
            && state.set_flight_movement_key(key_code, event.state.is_pressed())
        {
            window.request_redraw();
        }
        let egui_response = state.egui_state.on_window_event(window, &event);
        if egui_response.repaint && !matches!(&event, WindowEvent::RedrawRequested) {
            window.request_redraw();
        }

        if let WindowEvent::MouseWheel { delta, .. } = &event {
            let wheel_delta = match delta {
                winit::event::MouseScrollDelta::LineDelta(_, y) => f64::from(*y),
                winit::event::MouseScrollDelta::PixelDelta(position) => position.y / 80.0,
            };
            state.zoom_camera(wheel_delta);
            window.request_redraw();
            return;
        }

        if !egui_response.consumed {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Focused(focused) => state.set_mouse_capture(window, focused),
                WindowEvent::Resized(size) => state.resize(size),
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::KeyF) =>
                {
                    state.toggle_fullscreen(window);
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F3) =>
                {
                    state.toggle_debug_overlay();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F4) =>
                {
                    state.toggle_camera_mode();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F5) =>
                {
                    state.toggle_render_path();
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
                        && event.physical_key == PhysicalKey::Code(KeyCode::F6) =>
                {
                    state.toggle_blur();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F7) =>
                {
                    state.toggle_bloom();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F8) =>
                {
                    state.toggle_hdr_effect();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F9) =>
                {
                    state.cycle_render_debug_mode();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F10) =>
                {
                    state.toggle_animation_freeze();
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
                WindowEvent::RedrawRequested => {
                    if let Some(passed) = state.render(window) {
                        self.scenario_failed.store(!passed, Ordering::Relaxed);
                        event_loop.exit();
                    }
                }
                _ => {}
            }
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        let (Some(window), Some(state)) = (self.window.as_ref(), self.state.as_mut()) else {
            return;
        };
        if let DeviceEvent::MouseMotion { delta } = event
            && state.mouse_captured
        {
            state.look_camera(
                delta.0 * MOUSE_LOOK_RADIANS_PER_PIXEL,
                -delta.1 * MOUSE_LOOK_RADIANS_PER_PIXEL,
            );
            window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
            window.request_redraw();
        }
    }
}

struct LaunchOptions {
    scenario_name: Option<String>,
    profile_render: bool,
    vertical_fov_degrees: Option<f64>,
    terrain_source: terrain::TerrainSource,
}

fn launch_options() -> Result<LaunchOptions, String> {
    let default_outmap = PathBuf::from(DEFAULT_OUTMAP_PATH);
    let mut options = LaunchOptions {
        scenario_name: None,
        profile_render: false,
        vertical_fov_degrees: None,
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
            "--vertical-fov-degrees" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--vertical-fov-degrees requires a number".to_owned())?;
                let degrees = value.parse::<f64>().map_err(|_| {
                    "--vertical-fov-degrees must be a finite positive number".to_owned()
                })?;
                if !degrees.is_finite() || degrees <= 0.0 {
                    return Err(
                        "--vertical-fov-degrees must be a finite positive number".to_owned()
                    );
                }
                options.vertical_fov_degrees = Some(degrees);
            }
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

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::{
        CameraMode, FlightMovementInput, FlightSpeedState, INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
        LOW_FLIGHT_INITIAL_PITCH_RADIANS, LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND,
        MAX_LOW_FLIGHT_FRAME_DELTA_SECONDS, RenderPath, advance_flight_position_on_sphere,
        advance_flight_speed, flight_movement_direction, flight_view_direction,
        initial_flight_tangent, interactive_camera_delta_seconds, render_size_for_surface_resize,
        should_enter_fullscreen, transport_flight_tangent,
    };
    use crate::planet::PLANET_ROTATION_PERIOD_SECONDS;

    #[test]
    fn idle_flight_has_no_movement_direction() {
        assert_eq!(
            flight_movement_direction(FlightMovementInput::default(), DVec3::Z, DVec3::X),
            None
        );
    }

    #[test]
    fn fullscreen_key_toggles_windowed_state() {
        assert!(should_enter_fullscreen(false));
        assert!(!should_enter_fullscreen(true));
    }

    #[test]
    fn fullscreen_resize_preserves_the_prior_internal_render_resolution() {
        let windowed = winit::dpi::PhysicalSize::new(320, 200);
        let fullscreen = winit::dpi::PhysicalSize::new(1920, 1080);
        assert_eq!(
            render_size_for_surface_resize(fullscreen, Some(windowed)),
            windowed
        );
        assert_eq!(render_size_for_surface_resize(windowed, None), windowed);
    }

    #[test]
    fn render_path_defaults_to_raster_and_toggles_both_ways() {
        let path = RenderPath::default();
        assert_eq!(path, RenderPath::Raster);
        assert_eq!(path.toggled(), RenderPath::FoveatedRay);
        assert_eq!(path.toggled().toggled(), RenderPath::Raster);
    }

    #[test]
    fn flight_forward_and_backward_follow_the_camera_vector() {
        let camera_forward = DVec3::new(0.2, 0.7, -0.4).normalize();
        let forward = flight_movement_direction(
            FlightMovementInput {
                forward: true,
                ..FlightMovementInput::default()
            },
            camera_forward,
            DVec3::X,
        )
        .unwrap();
        let backward = flight_movement_direction(
            FlightMovementInput {
                backward: true,
                ..FlightMovementInput::default()
            },
            camera_forward,
            DVec3::X,
        )
        .unwrap();

        assert!(forward.distance(camera_forward) < 1.0e-12);
        assert!(backward.distance(-camera_forward) < 1.0e-12);
    }

    #[test]
    fn held_flight_input_increases_acceleration_over_time() {
        let first = advance_flight_speed(FlightSpeedState::default(), true, false, 0.5);
        let second = advance_flight_speed(first, true, false, 0.5);
        let third = advance_flight_speed(second, true, false, 0.5);

        let first_gain = first.speed_meters_per_second;
        let second_gain = second.speed_meters_per_second - first.speed_meters_per_second;
        let third_gain = third.speed_meters_per_second - second.speed_meters_per_second;
        assert!(second_gain > first_gain);
        assert!(third_gain > second_gain);
    }

    #[test]
    fn releasing_flight_input_brakes_quickly_and_resets_the_ramp() {
        let mut held = FlightSpeedState::default();
        for _ in 0..180 {
            held = advance_flight_speed(held, true, false, 1.0 / 60.0);
        }
        let released = advance_flight_speed(held, false, false, 0.4);

        assert!(released.speed_meters_per_second < held.speed_meters_per_second / 30.0);
        assert_eq!(released.acceleration_time_seconds, 0.0);
    }

    #[test]
    fn accelerated_flight_has_a_finite_interplanetary_speed_cap() {
        let mut state = FlightSpeedState::default();
        for _ in 0..1_800 {
            state = advance_flight_speed(state, true, true, 1.0 / 60.0);
        }

        assert_eq!(
            state.speed_meters_per_second,
            LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND
        );
    }

    #[test]
    fn diagonal_flight_is_normalized_and_strafes_camera_right() {
        let direction = flight_movement_direction(
            FlightMovementInput {
                forward: true,
                right: true,
                ..FlightMovementInput::default()
            },
            DVec3::Z,
            DVec3::X,
        )
        .unwrap();

        assert!((direction.length() - 1.0).abs() < 1.0e-12);
        assert!(direction.dot(DVec3::Z) > 0.0);
        assert!(direction.dot(DVec3::X) > 0.0);
    }

    #[test]
    fn flight_tangent_stays_continuous_across_a_pole() {
        let before_pole = DVec3::new(0.0, 1.0, 0.001).normalize();
        let after_pole = DVec3::new(0.0, 1.0, -0.001).normalize();
        let tangent_before = initial_flight_tangent(before_pole);
        let longitude_tangent_after = initial_flight_tangent(after_pole);
        let transported_tangent = transport_flight_tangent(tangent_before, before_pole, after_pole);

        assert!(tangent_before.dot(longitude_tangent_after) < -0.999);
        assert!(tangent_before.dot(transported_tangent) > 0.999);
        assert!(transported_tangent.dot(after_pole).abs() < 1.0e-12);
    }

    #[test]
    fn tangent_flight_follows_the_sphere_without_gaining_altitude() {
        let altitude = 1_524.0;
        let position = DVec3::X * (crate::planet::PLANET_RADIUS_METERS + altitude);
        let moved = advance_flight_position_on_sphere(position, DVec3::Z, 25_000.0);

        assert!((moved.length() - position.length()).abs() < 1.0e-9);
        assert!(moved.z > 0.0);
    }

    #[test]
    fn low_flight_starts_looking_down_at_the_landing_site() {
        let radial = DVec3::X;
        let direction =
            flight_view_direction(radial, DVec3::Z, 0.0, LOW_FLIGHT_INITIAL_PITCH_RADIANS);

        assert!(direction.dot(radial) < -0.25);
        assert!(direction.dot(DVec3::Z) > 0.9);
    }

    #[test]
    fn frozen_scene_keeps_low_flight_navigation_on_frame_time() {
        let frame_delta_seconds = 1.0 / 60.0;

        assert_eq!(
            interactive_camera_delta_seconds(CameraMode::LowFlight, 0.0, frame_delta_seconds),
            frame_delta_seconds
        );
        assert_eq!(
            interactive_camera_delta_seconds(CameraMode::Orbit, 0.0, frame_delta_seconds),
            0.0
        );
    }

    #[test]
    fn slow_frames_cannot_amplify_low_flight_terrain_churn() {
        assert_eq!(
            interactive_camera_delta_seconds(CameraMode::LowFlight, 0.0, 0.25),
            MAX_LOW_FLIGHT_FRAME_DELTA_SECONDS,
        );
    }

    #[test]
    fn interactive_world_space_sun_moves_relative_to_planet() {
        let rotation =
            crate::planet::planet_rotation_radians(15.0 * INTERACTIVE_PLANET_ROTATION_TIME_SCALE);
        let initial_sun = crate::planet::planet_local_vector(DVec3::X, 0.0);
        let later_sun = crate::planet::planet_local_vector(DVec3::X, rotation);
        let relative_motion_degrees = initial_sun.angle_between(later_sun).to_degrees();
        let unwrapped_motion_degrees =
            360.0 * 15.0 * INTERACTIVE_PLANET_ROTATION_TIME_SCALE / PLANET_ROTATION_PERIOD_SECONDS;
        let expected_motion_degrees = unwrapped_motion_degrees
            .rem_euclid(360.0)
            .min((-unwrapped_motion_degrees).rem_euclid(360.0));

        assert!((relative_motion_degrees - expected_motion_degrees).abs() < 1.0e-12);
    }
}

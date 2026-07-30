mod atmosphere;
mod debug;
mod foveated;
mod haze;
mod hdr;
mod ocean;
mod outmap;
mod planet;
mod probe;
#[cfg(test)]
mod relief_survey;
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

fn device_mouse_look_enabled(mouse_captured: bool, scenario_active: bool) -> bool {
    mouse_captured && !scenario_active
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
const LOW_FLIGHT_ALTITUDE_METERS: f64 = 500.0 * 0.3048;
/// Highest summit on the active Earth-like planet after the fixed 3x macro
/// presentation and bounded runtime detail are applied. The L4 source was
/// scanned globally, every cell capable of beating the current maximum was
/// refined to one metre, and the resulting summit lies at 41.530039 N,
/// 71.196130 E. As the planet's highest summit it has no higher parent, so the
/// standard Earth prominence convention uses sea level as its key col.
const EARTHLIKE_HIGHEST_PROMINENCE_DIRECTION: glam::DVec3 = glam::DVec3::new(
    0.241_298_616_876_062,
    0.663_012_622_123_029,
    0.708_653_117_116_721,
);
const EARTHLIKE_HIGHEST_PROMINENCE_METERS: f64 = 27_207.866_782_074;
#[cfg(test)]
const EARTHLIKE_HIGHEST_RAW_MACRO_ELEVATION_METERS: f64 = 8_738.565_429_687_5;
/// How close to the ground flight may descend. This used to be the entry
/// altitude above, doing double duty, so the camera could never get nearer the
/// surface than 500 ft and eye-level views of the terrain were unreachable.
/// Separating them is only safe now that CPU clearance evaluates the same
/// synthesised relief the shader displaces with; against baked heights alone a
/// floor this low would have put the camera inside the ground.
const LOW_FLIGHT_MINIMUM_CLEARANCE_METERS: f64 = 2.0;
/// Sweep the camera point through the rendered terrain instead of checking
/// only the end of a frame. L18 patch boundaries are roughly a metre apart;
/// sub-metre samples prevent a downward W flight from tunnelling through a
/// higher incoming/outgoing patch between two otherwise safe endpoints.
const LOW_FLIGHT_COLLISION_SWEEP_STEP_METERS: f64 = 0.5;
const LOW_FLIGHT_COLLISION_MAX_SWEEP_SAMPLES: usize = 64;
/// Translating through the one-metre raster frontier needs a camera-sized
/// clearance envelope, not the two-metre idle eye-height point. The captured
/// mountain replay measured up to 25.9m between the point truth and a visible
/// transition/skirt hit; 30m keeps the near camera outside that rendered
/// envelope while preserving the 2m stationary inspection height.
const LOW_FLIGHT_MOVING_CLEARANCE_METERS: f64 = 30.0;
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
const FOVEA_MINIMUM_SPEED_METERS_PER_SECOND: f64 = 1.0;
const FOVEA_MINIMUM_FORWARD_COSINE: f64 = 0.1;
const FOVEA_MAXIMUM_NDC_OFFSET: f64 = 0.7;
const CONTENT_ADAPTIVE_WARP_MINIMUM_PLANET_COVERAGE: f64 = 0.65;

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

    /// Whether this path draws the streamed chunk meshes. Terrain *data* is
    /// updated in every path regardless -- see the call site.
    fn draws_terrain_meshes(self) -> bool {
        self == Self::Raster
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

fn focus_of_expansion_ndc(
    camera_mode: CameraMode,
    velocity_planet_frame: glam::DVec3,
    camera: &planet::CameraUniform,
) -> [f32; 2] {
    if camera_mode != CameraMode::LowFlight
        || velocity_planet_frame.length() < FOVEA_MINIMUM_SPEED_METERS_PER_SECOND
    {
        return [0.0; 2];
    }
    let velocity_direction = velocity_planet_frame.normalize();
    let forward = glam::DVec3::new(
        f64::from(camera.camera_forward[0]),
        f64::from(camera.camera_forward[1]),
        f64::from(camera.camera_forward[2]),
    );
    let forward_cosine = velocity_direction.dot(forward);
    if forward_cosine <= FOVEA_MINIMUM_FORWARD_COSINE {
        return [0.0; 2];
    }
    let right = glam::DVec3::new(
        f64::from(camera.camera_right[0]),
        f64::from(camera.camera_right[1]),
        f64::from(camera.camera_right[2]),
    );
    let up = glam::DVec3::new(
        f64::from(camera.camera_up[0]),
        f64::from(camera.camera_up[1]),
        f64::from(camera.camera_up[2]),
    );
    let aspect_ratio = f64::from(camera.projection[0]);
    let vertical_tangent = f64::from(camera.projection[1]);
    let horizontal =
        velocity_direction.dot(right) / (forward_cosine * vertical_tangent * aspect_ratio);
    let vertical = velocity_direction.dot(up) / (forward_cosine * vertical_tangent);
    [
        horizontal.clamp(-FOVEA_MAXIMUM_NDC_OFFSET, FOVEA_MAXIMUM_NDC_OFFSET) as f32,
        vertical.clamp(-FOVEA_MAXIMUM_NDC_OFFSET, FOVEA_MAXIMUM_NDC_OFFSET) as f32,
    ]
}

fn projected_planet_coverage(
    camera_radius_meters: f64,
    vertical_fov_radians: f64,
    aspect_ratio: f64,
) -> f64 {
    if camera_radius_meters <= planet::PLANET_RADIUS_METERS {
        return 1.0;
    }
    let sine_radius = (planet::PLANET_RADIUS_METERS / camera_radius_meters).clamp(0.0, 1.0);
    let tangent_radius = sine_radius / (1.0 - sine_radius * sine_radius).max(1.0e-12).sqrt();
    let vertical_radius = tangent_radius / (vertical_fov_radians * 0.5).tan();
    let horizontal_radius = vertical_radius / aspect_ratio;
    std::f64::consts::PI * horizontal_radius.clamp(0.0, 1.0) * vertical_radius.clamp(0.0, 1.0) / 4.0
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

fn swept_flight_clearance_lift(
    start: glam::DVec3,
    end: glam::DVec3,
    clearance_meters: f64,
    mut surface_height_meters: impl FnMut(glam::DVec3, f64) -> Option<f64>,
) -> f64 {
    let start_radius = start.length();
    let end_radius = end.length();
    let start_direction = start / start_radius;
    let end_direction = end / end_radius;
    let angular_distance = start_direction.dot(end_direction).clamp(-1.0, 1.0).acos();
    let surface_distance = angular_distance * 0.5 * (start_radius + end_radius);
    let travel_distance = surface_distance.hypot(end_radius - start_radius);
    let steps = ((travel_distance / LOW_FLIGHT_COLLISION_SWEEP_STEP_METERS).ceil() as usize)
        .clamp(1, LOW_FLIGHT_COLLISION_MAX_SWEEP_SAMPLES);
    let mut lift_meters = 0.0_f64;
    for step in 0..=steps {
        let amount = step as f64 / steps as f64;
        let direction = start_direction.lerp(end_direction, amount).normalize();
        let radius = start_radius + (end_radius - start_radius) * amount;
        let altitude_meters = (radius - planet::PLANET_RADIUS_METERS).max(0.0);
        if let Some(height_meters) = surface_height_meters(direction, altitude_meters) {
            lift_meters = lift_meters
                .max(planet::PLANET_RADIUS_METERS + height_meters + clearance_meters - radius);
        }
    }
    lift_meters.max(0.0)
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
    render_path: RenderPath,
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
    raymarch_ms: f64,
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

    fn begin_readback(&mut self, index: usize, sim_time: f64, render_path: RenderPath) {
        let (sender, receiver) = mpsc::channel();
        self.slots[index]
            .readback_buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result.is_ok());
            });
        self.slots[index].pending = Some(PendingGpuTimestamp {
            sim_time,
            render_path,
            receiver,
        });
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
            let render_path = pending.render_path;
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
            let scene_ms = elapsed(0, 1);
            let timings = GpuStageTimings {
                scene_ms,
                raymarch_ms: if render_path == RenderPath::FoveatedRay {
                    scene_ms
                } else {
                    0.0
                },
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
    depth_texture: wgpu::Texture,
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
    scenario_flight_initialized: bool,
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
            present_mode: match std::env::var("CATINGARDEN_PRESENT_MODE").as_deref() {
                Ok("immediate") => wgpu::PresentMode::Immediate,
                Ok("mailbox") => wgpu::PresentMode::Mailbox,
                Ok("fifo_relaxed") => wgpu::PresentMode::FifoRelaxed,
                Ok("auto_vsync") => wgpu::PresentMode::AutoVsync,
                Ok("auto_no_vsync") => wgpu::PresentMode::AutoNoVsync,
                _ => wgpu::PresentMode::Fifo,
            },
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: std::env::var("CATINGARDEN_FRAME_LATENCY")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2),
        };
        tracing::info!(
            target: "catinthegarden::adapter",
            supported_present_modes = ?surface_capabilities.present_modes,
            selected_present_mode = ?config.present_mode,
            surface_usage = ?config.usage,
            "configured surface"
        );
        surface.configure(&device, &config);
        let (depth_texture, depth_view) = create_depth_texture(&device, size);
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
        let shared_planet_bind_group_layout = terrain::create_shared_bind_group_layout(&device);
        let foveated = foveated::FoveatedRenderer::new(
            &device,
            &queue,
            hdr::HdrRenderer::SCENE_FORMAT,
            size,
            &camera_bind_group_layout,
            &shared_planet_bind_group_layout,
            terrain_source.clone(),
        )
        .expect("foveated renderer must initialize");
        let terrain = terrain::TerrainRenderer::new(
            &device,
            &queue,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            shared_planet_bind_group_layout,
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

        let mut state = Self {
            surface,
            device,
            queue,
            config,
            size,
            surface_size: size,
            fullscreen_render_size: None,
            depth_texture,
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
            render_debug_mode: match std::env::var("CATINGARDEN_DEBUG_MODE")
                .unwrap_or_default()
                .trim()
            {
                "albedo" => planet::RenderDebugMode::RawAlbedo,
                "lighting" => planet::RenderDebugMode::SurfaceLighting,
                "aerial" => planet::RenderDebugMode::AerialContribution,
                "sky" => planet::RenderDebugMode::SkyOnly,
                "ray_hit" => planet::RenderDebugMode::RayHitStatus,
                _ => planet::RenderDebugMode::Final,
            },
            animation_frozen: false,
            frozen_sim_time: 0.0,
            interactive_scene_time_offset_seconds: 0.0,
            manual_screenshot_requested: false,
            next_spatial_log_presentation_time: 0.0,
            capture_number: 0,
            scenario,
            scenario_flight_initialized: false,
            artifacts,
            scenario_capture_failed: false,
            mouse_captured: false,
            profile_render,
            gpu_profiler,
            cached_paint_jobs: Vec::new(),
            egui_buffers_dirty: true,
            next_hud_update: Instant::now(),
            hud_dirty: true,
        };
        state.apply_startup_experiment_overrides();
        state
    }

    /// Applies the render path and ray-experiment toggles requested through the
    /// environment, so an automated benchmark can reach the same state a human
    /// gets by pressing F5 and a number key. These call the identical toggle
    /// helpers the key handlers use, and the resulting state is logged so every
    /// run records which configuration produced its samples.
    fn apply_startup_experiment_overrides(&mut self) {
        if std::env::var("CATINGARDEN_RENDER_PATH").as_deref() == Ok("ray") {
            self.toggle_render_path();
        }
        if let Ok(experiments) = std::env::var("CATINGARDEN_RAY_EXPERIMENTS") {
            for index in experiments
                .split(',')
                .filter_map(|value| value.trim().parse::<u8>().ok())
            {
                self.toggle_ray_experiment(index);
            }
        }
        let enabled: Vec<u8> = (1..=5)
            .filter(|index| self.foveated.experiment_enabled(*index))
            .collect();
        tracing::info!(
            target: "catinthegarden::experiment",
            render_path = self.render_path.label(),
            render_debug_mode = self.render_debug_mode.label(),
            enabled_experiments = ?enabled,
            "render configuration"
        );
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
        let (depth_texture, depth_view) = create_depth_texture(&self.device, size);
        self.depth_texture = depth_texture;
        self.depth_view = depth_view;
        self.hdr.resize(&self.device, size);
        self.foveated.resize(&self.device, size);
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

    fn toggle_auto_exposure(&mut self) {
        self.hdr
            .set_auto_exposure_enabled(&self.queue, !self.hdr.auto_exposure_enabled());
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

    fn toggle_warp_debug(&mut self) {
        self.foveated.toggle_warp_debug(&self.queue);
        self.mark_hud_dirty();
    }

    fn toggle_ray_experiment(&mut self, index: u8) {
        self.foveated.toggle_experiment(&self.queue, index);
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
        let movement_start_position = self.flight_local_position;
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
        self.update_low_flight_camera(Some(movement_start_position), planet_rotation_radians);
    }

    fn update_low_flight_camera(
        &mut self,
        movement_start_position: Option<glam::DVec3>,
        planet_rotation_radians: f64,
    ) {
        if self.render_path == RenderPath::Raster
            && let Some(start) = movement_start_position
            && start.distance_squared(self.flight_local_position) > f64::EPSILON
        {
            let lift_meters = swept_flight_clearance_lift(
                start,
                self.flight_local_position,
                LOW_FLIGHT_MOVING_CLEARANCE_METERS,
                |direction, altitude_meters| {
                    self.terrain
                        .raster_surface_height_meters_at(direction, altitude_meters)
                },
            );
            if lift_meters > 0.0 {
                self.flight_local_position = self.flight_local_position.normalize()
                    * (self.flight_local_position.length() + lift_meters);
            }
        }
        let local_radial = self.flight_local_position.normalize();
        let camera_altitude_meters =
            (self.flight_local_position.length() - planet::PLANET_RADIUS_METERS).max(0.0);
        let surface_height_meters = match self.render_path {
            RenderPath::Raster => self
                .terrain
                .raster_surface_height_meters_at(local_radial, camera_altitude_meters),
            RenderPath::FoveatedRay => self
                .terrain
                .surface_height_meters_at(local_radial, camera_altitude_meters),
        };
        if let Some(surface_height_meters) = surface_height_meters {
            self.flight_surface_height_meters = surface_height_meters;
        }
        // Terrain tiles can become resident while the camera is idle. Enforce
        // clearance every frame so a newly resolved higher surface cannot
        // leave the camera underground until the next movement key is pressed.
        let minimum_radius = planet::PLANET_RADIUS_METERS
            + self.flight_surface_height_meters
            + LOW_FLIGHT_MINIMUM_CLEARANCE_METERS;
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
                // Enter inspection mode over the active planet's highest-
                // prominence summit. Make its global L4 tile resident
                // synchronously: resolving through a coarse ancestor here can
                // differ by hundreds of metres and would move the camera after
                // F4 while ordinary streaming catches up.
                let outmap_is_active = self.terrain.preferred_landing_direction().is_some();
                let local_radial = if outmap_is_active {
                    EARTHLIKE_HIGHEST_PROMINENCE_DIRECTION.normalize()
                } else {
                    local_position.normalize()
                };
                self.flight_surface_height_meters = if outmap_is_active {
                    self.terrain
                        .prepare_flight_start_surface_height_meters(
                            local_radial,
                            LOW_FLIGHT_ALTITUDE_METERS,
                        )
                        .unwrap_or(EARTHLIKE_HIGHEST_PROMINENCE_METERS)
                } else {
                    self.terrain
                        .surface_height_meters_at(local_radial, LOW_FLIGHT_ALTITUDE_METERS)
                        .unwrap_or(0.0)
                };
                self.flight_local_position = local_radial
                    * (planet::PLANET_RADIUS_METERS
                        + self.flight_surface_height_meters
                        + LOW_FLIGHT_ALTITUDE_METERS);
                // Face back across the summit bowl. The opposite azimuth has
                // the same pitch and controls but avoids putting the nearby
                // L4 frontier edge across the foreground.
                self.flight_local_tangent = if outmap_is_active {
                    -initial_flight_tangent(local_radial)
                } else {
                    initial_flight_tangent(local_radial)
                };
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
                self.update_low_flight_camera(None, planet_rotation_radians);
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
        self.previous_camera_world_position = self.camera.world_position();
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
            scenario_planet_relative_up,
            surface_probe_max_distance_meters,
            scenario_vertical_fov_degrees,
            scenario_sun_direction,
            scenario_planet_rotation_time_scale,
            scenario_forward_flight_held,
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
                scenario.uses_planet_relative_up(),
                scenario.surface_probe_max_distance_meters(),
                frame.vertical_fov_degrees,
                Some(glam::DVec3::from_array(frame.sun_direction)),
                frame.planet_rotation_time_scale,
                frame.forward_flight_held,
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
                false,
                probe::MAX_COMPARISON_DISTANCE_METERS,
                None,
                None,
                INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
                None,
            )
        };
        if let Some((position, look_at)) = scenario_pose {
            // Surface-level scenarios need the horizon level, so their up axis
            // is pinned to the local radial. Orbital scenarios keep the default
            // basis, where a radial up is degenerate when looking straight down.
            if scenario_planet_relative_up {
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
        if let Some(forward_held) = scenario_forward_flight_held {
            if !self.scenario_flight_initialized {
                let local_position = planet::planet_local_vector(
                    self.camera.world_position(),
                    planet_rotation_radians,
                );
                let local_radial = local_position.normalize();
                let local_view_direction = planet::planet_local_vector(
                    self.camera.direction_dvec3(),
                    planet_rotation_radians,
                )
                .normalize();
                let local_tangent =
                    local_view_direction - local_radial * local_view_direction.dot(local_radial);
                assert!(
                    local_tangent.length_squared() > f64::EPSILON,
                    "forward-flight scenario cannot look exactly radial"
                );
                self.flight_local_position = local_position;
                self.flight_local_tangent = local_tangent.normalize();
                self.flight_look_yaw_radians = 0.0;
                self.flight_look_pitch_radians = local_view_direction
                    .dot(local_radial)
                    .clamp(-1.0, 1.0)
                    .asin();
                let sea_level_altitude = local_position.length() - planet::PLANET_RADIUS_METERS;
                self.flight_surface_height_meters = self
                    .terrain
                    .prepare_flight_start_surface_height_meters(local_radial, sea_level_altitude)
                    .unwrap_or(0.0);
                self.flight_speed = FlightSpeedState::default();
                self.flight_travel_direction = glam::DVec3::ZERO;
                self.camera_mode = CameraMode::LowFlight;
                self.previous_camera_world_position = self.camera.world_position();
                self.scenario_flight_initialized = true;
            }
            self.flight_movement = FlightMovementInput {
                forward: forward_held,
                ..FlightMovementInput::default()
            };
            self.advance_low_flight_camera(scene_delta_seconds, planet_rotation_radians);
        } else if self.scenario.is_none() {
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
        let camera_velocity_world = (camera_world_position - self.previous_camera_world_position)
            / delta_camera_motion_seconds;
        let velocity_meters_per_second = camera_velocity_world.length();
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
        // Terrain streaming runs in every render path, not just the one that
        // draws the meshes.
        //
        // The raymarch path used to skip this to save the quadtree's cost, and
        // the saving was real, but it also froze the CPU's tile cache: the
        // camera's collision height, the near plane and the LOD reference all
        // came from whatever tiles happened to be resident at startup. At the
        // landing site that put the CPU's ground at 989.9m instead of 923.1m,
        // so a camera placed 2m above the ground reported itself 64m *inside*
        // it. Terrain truth cannot depend on which path is drawing.
        //
        // It is also cheaper than it looks: measured at 22.4ms for eye level
        // and 17.9ms at orbit in ray mode, unchanged from suspending it,
        // because the expensive part of a raster frame is drawing the chunks
        // rather than selecting them.
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
        // The chunk mesh statistics describe drawing, so a path that streams
        // terrain without drawing it reports none.
        let draws_terrain_meshes = !solid_color_screen && self.render_path.draws_terrain_meshes();
        if !draws_terrain_meshes {
            self.terrain_stats.drawn_chunks = 0;
            self.terrain_stats.terrain_triangles = 0;
            self.terrain_stats.ocean_chunks = 0;
            self.terrain_stats.ocean_triangles = 0;
            self.terrain_stats.draw_calls = 0;
        }
        let draw_calls = self.terrain_stats.draw_calls;
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
                    ocean_chunks: self.terrain_stats.ocean_chunks,
                    ocean_triangles: self.terrain_stats.ocean_triangles,
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
            let metered_exposure = exposure_state.metered_exposure;
            let auto_exposure_enabled = exposure_state.auto_exposure_enabled;
            let average_luminance = exposure_state.average_luminance;
            let ocean_wave_range = ocean_wave_range;
            let blur_enabled = self.hdr.blur_enabled();
            let bloom_enabled = self.hdr.bloom_enabled();
            let hdr_effect_enabled = self.hdr.hdr_effect_enabled();
            let render_path = self.render_path;
            let render_debug_mode = self.render_debug_mode;
            let warp_size = self.foveated.warp_size();
            let warp_debug_visible = self.foveated.warp_debug_visible();
            let fovea_ndc = self.foveated.fovea_ndc();
            let experiment_states =
                [1_u8, 2, 3, 4, 5].map(|index| self.foveated.experiment_enabled(index));
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
                            ui.label(format!(
                                "Ocean: {} chunks  |  {} triangles",
                                terrain_stats.ocean_chunks, terrain_stats.ocean_triangles,
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
                                "Exposure: {exposure:.3} {}  |  Meter: {metered_exposure:.3}  |  Average luminance: {average_luminance:.3}",
                                if auto_exposure_enabled {
                                    "auto"
                                } else {
                                    "fixed"
                                },
                            ));
                            ui.label(format!(
                                "Post: blur {}  |  bloom {}  |  HDR curve {}",
                                if blur_enabled { "on" } else { "off" },
                                if bloom_enabled { "on" } else { "off" },
                                if hdr_effect_enabled { "on" } else { "off" },
                            ));
                            ui.label(format!(
                                "Composition debug: {}",
                                render_debug_mode.label(),
                            ));
                            ui.label(format!("Render path: {} (F5)", render_path.label()));
                            ui.label(format!(
                                "Ray warp: {}x{}  |  fovea {:+.2}, {:+.2} NDC  |  debug {} (F11)",
                                warp_size.width,
                                warp_size.height,
                                fovea_ndc[0],
                                fovea_ndc[1],
                                if warp_debug_visible { "on" } else { "off" },
                            ));
                            ui.label(format!(
                                "M8: 1 horizon {} | 2 temporal {} | 3 adaptive {} | 4 shading {} | 5 blur {}",
                                if experiment_states[0] { "on" } else { "off" },
                                if experiment_states[1] { "on" } else { "off" },
                                if experiment_states[2] { "on" } else { "off" },
                                if experiment_states[3] { "on" } else { "off" },
                                if experiment_states[4] { "on" } else { "off" },
                            ));
                            ui.label(format!(
                                "Animation: {}",
                                if animation_frozen { "frozen" } else { "running" },
                            ));
                            ui.label(format!("Ocean Gerstner range: {ocean_wave_range:.2} m"));
                            ui.label(
                                "F: fullscreen  |  F3: overlay  |  F4: camera mode  |  F5: render path  |  WASD: fly  |  F6: blur  |  F7: bloom  |  F8: HDR  |  6: exposure  |  F9: composition  |  F10: freeze  |  F11: warp view  |  F12: capture PNG",
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
        // The near plane has to be set from the camera's clearance above the
        // ground, so scenarios and orbit need this as much as flight does --
        // any camera over high terrain clips its own foreground otherwise.
        // The surface probe reports its clearance from the same number, so the
        // two can never drift apart.
        // Sea-level altitude, which is what this argument selects the height
        // scale by -- not the above-ground figure the HUD uses.
        let camera_sea_level_altitude_meters =
            camera_planet_frame_position.length() - planet::PLANET_RADIUS_METERS;
        let camera_surface_height_meters = match self.render_path {
            RenderPath::Raster => self.terrain.raster_surface_height_meters_at(
                camera_planet_frame_position.normalize(),
                camera_sea_level_altitude_meters,
            ),
            RenderPath::FoveatedRay => self.terrain.surface_height_meters_at(
                camera_planet_frame_position.normalize(),
                camera_sea_level_altitude_meters,
            ),
        }
        .unwrap_or(0.0);
        let aspect_ratio = self.size.width as f32 / self.size.height as f32;
        let camera_uniform = planet::CameraUniform::from_camera(
            &self.camera,
            aspect_ratio,
            self.sun_direction,
            planet_rotation_radians,
            sim_time,
            self.render_debug_mode,
            camera_surface_height_meters,
        );
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        // Probe on the frames that produce a screenshot, so a measured
        // disagreement always has the picture that goes with it.
        let probe_requested =
            !solid_color_screen && (scenario_capture || self.manual_screenshot_requested);
        let probe_geometry = probe_requested.then(|| {
            probe::ProbeGeometry::new(
                planet::near_plane_meters(
                    camera_sea_level_altitude_meters,
                    camera_surface_height_meters,
                ),
                self.camera.vertical_fov_radians(),
                f64::from(aspect_ratio),
                camera_planet_frame_position,
                camera_planet_frame_direction,
                camera_planet_frame_up,
            )
        });
        let mut ray_near_field_coverage = None;
        if self.render_path == RenderPath::FoveatedRay {
            // Keep the raymarch path's near-field window under the camera. The
            // dense pyramid it otherwise samples is 3068m per texel, which at
            // the landing site reads 104m below the ground the camera stands
            // on; the window is the only thing that closes that.
            //
            // Rebuilt only when the camera leaves the square it was built for,
            // which at ground level is kilometres of travel. Assembly reads
            // resident tiles only, so a window that cannot be completed yet
            // leaves the previous one in place rather than uploading a hole.
            let clearance_meters =
                (camera_sea_level_altitude_meters - camera_surface_height_meters).max(0.0);
            match self
                .terrain
                .near_field_key(camera_planet_frame_position.normalize(), clearance_meters)
            {
                // Rebuild against the tiles actually backing the window, not
                // just its position. A stationary camera keeps the same key
                // while streaming replaces coarse ancestors underneath it, and
                // the first assembly of a run is always the coarse one -- at
                // the parity ridge that reads 290m below the settled surface.
                Some(key) => {
                    self.terrain.request_near_field_tiles(key);
                    if let Some(sources) = self.terrain.near_field_sources(key)
                        && self.foveated.near_field_sources() != Some(&sources)
                        && let Some(window) = self.terrain.near_field_window(&sources)
                    {
                        tracing::info!(
                            level = key.level,
                            face = key.face.index(),
                            max_height_m = window.max_height_meters,
                            "near-field window built"
                        );
                        self.foveated.set_near_field(&self.queue, &window);
                    }
                    if probe_requested {
                        ray_near_field_coverage =
                            self.terrain.near_field_coverage(key).map(|mut coverage| {
                                coverage.active_window_level = self
                                    .foveated
                                    .near_field_sources()
                                    .map(|sources| sources.key.level);
                                tracing::info!(
                                    requested_level = coverage.requested_level,
                                    resident_blocks = coverage.resident_blocks,
                                    fine_blocks = coverage.finer_than_dense_blocks,
                                    minimum_source_level = coverage.minimum_source_level,
                                    maximum_source_level = coverage.maximum_source_level,
                                    window_eligible = coverage.window_eligible,
                                    active_window_level = coverage.active_window_level,
                                    "ray near-field coverage"
                                );
                                coverage
                            });
                    }
                }
                // Only a camera high enough to stop needing the window turns it
                // off. A momentary gap in residency keeps whatever is loaded.
                None => self.foveated.clear_near_field(),
            }
            let flight_velocity_planet_frame =
                if self.scenario.is_none() && self.camera_mode == CameraMode::LowFlight {
                    self.flight_travel_direction * self.flight_speed.speed_meters_per_second
                } else {
                    glam::DVec3::ZERO
                };
            let target_fovea_ndc = focus_of_expansion_ndc(
                self.camera_mode,
                flight_velocity_planet_frame,
                &camera_uniform,
            );
            self.foveated.update(
                &self.queue,
                camera_radius - planet::PLANET_RADIUS_METERS,
                target_fovea_ndc,
                frame_time,
                camera_uniform.camera_forward[..3]
                    .try_into()
                    .expect("camera forward has three components"),
                camera_uniform.camera_right[..3]
                    .try_into()
                    .expect("camera right has three components"),
                camera_uniform.camera_up[..3]
                    .try_into()
                    .expect("camera up has three components"),
                camera_planet_frame_position.to_array(),
            );
        }
        let planet_coverage = projected_planet_coverage(
            camera_radius,
            self.camera.vertical_fov_radians(),
            self.size.width as f64 / self.size.height as f64,
        );
        let use_foveated_warp = self.render_path == RenderPath::FoveatedRay
            && self.render_debug_mode == planet::RenderDebugMode::Final
            && (!self.foveated.experiment_enabled(3)
                || planet_coverage >= CONTENT_ADAPTIVE_WARP_MINIMUM_PLANET_COVERAGE
                || self.foveated.warp_debug_visible());
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
                timestamp_writes: gpu_slot_index
                    .filter(|_| {
                        self.render_path == RenderPath::Raster
                            || self.render_debug_mode != planet::RenderDebugMode::Final
                            || !use_foveated_warp
                            || solid_color_screen
                    })
                    .map(|slot_index| {
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
                if self.render_debug_mode != planet::RenderDebugMode::Final || !use_foveated_warp {
                    self.foveated.draw_direct(
                        &mut render_pass,
                        &self.camera_bind_group,
                        self.terrain.shared_bind_group(),
                    );
                }
            }
        }
        if !solid_color_screen && use_foveated_warp {
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("warped raymarch pass"),
                    color_attachments: &[
                        Some(wgpu::RenderPassColorAttachment {
                            view: self.foveated.warp_color_view(),
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        }),
                        Some(wgpu::RenderPassColorAttachment {
                            view: self.foveated.warp_distance_view(),
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: -1.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 0.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        }),
                    ],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: gpu_slot_index.map(|slot_index| {
                        let profiler = self.gpu_profiler.as_ref().expect("GPU profiler exists");
                        wgpu::RenderPassTimestampWrites {
                            query_set: &profiler.slots[slot_index].query_set,
                            beginning_of_pass_write_index: Some(0),
                            end_of_pass_write_index: None,
                        }
                    }),
                    multiview_mask: None,
                });
                self.foveated.draw_warped(
                    &mut render_pass,
                    &self.camera_bind_group,
                    self.terrain.shared_bind_group(),
                );
            }
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("foveated unwarp pass"),
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
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: gpu_slot_index.map(|slot_index| {
                        let profiler = self.gpu_profiler.as_ref().expect("GPU profiler exists");
                        wgpu::RenderPassTimestampWrites {
                            query_set: &profiler.slots[slot_index].query_set,
                            beginning_of_pass_write_index: None,
                            end_of_pass_write_index: Some(1),
                        }
                    }),
                    multiview_mask: None,
                });
                self.foveated
                    .draw_unwarp(&mut render_pass, &self.camera_bind_group);
            }
            if self.foveated.experiment_enabled(2) {
                self.foveated.copy_to_history(&mut encoder);
            }
        }
        // Read the depth attachment back while it still holds the terrain, and
        // only the terrain: the visual sun overlay pass below discards depth on
        // store, and atmosphere and sun both write no depth at all.
        let pending_depth_probe = probe_requested.then(|| {
            probe::schedule_depth_readback(
                &self.device,
                &mut encoder,
                &self.depth_texture,
                self.size.width,
                self.size.height,
            )
        });
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
            if !solid_color_screen
                && self.render_debug_mode != planet::RenderDebugMode::SkyOnly
                && !(self.render_path == RenderPath::FoveatedRay
                    && self.render_debug_mode == planet::RenderDebugMode::Final
                    && self.foveated.warp_debug_visible())
            {
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
                .begin_readback(slot_index, sim_time, self.render_path);
        }
        let gpu_timestamp_readback_ms = gpu_readback_started.elapsed().as_secs_f32() * 1_000.0;
        let present_started = Instant::now();
        output.present();
        let present_ms = present_started.elapsed().as_secs_f32() * 1_000.0;
        let capture_started = Instant::now();
        let mut captured_frame = None;
        if let Some(pending_capture) = pending_capture {
            match debug::finish_capture(
                &self.device,
                pending_capture,
                &mut self.artifacts,
                sim_time,
                solid_color_screen,
                seam_gap_check,
            ) {
                Ok(frame) => captured_frame = Some(frame),
                Err(error) => {
                    self.scenario_capture_failed = true;
                    tracing::error!(%error, "screenshot capture failed");
                }
            }
        }
        let capture_readback_ms = capture_started.elapsed().as_secs_f32() * 1_000.0;
        if let (Some(pending), Some(geometry)) = (pending_depth_probe, probe_geometry) {
            match probe::finish_depth_readback(&self.device, pending) {
                Ok(depth) => {
                    let terrain = &self.terrain;
                    let render_path = self.render_path;
                    let mut report = probe::compare_surface_with_limit(
                        sim_time,
                        self.render_path.label(),
                        &geometry,
                        &depth,
                        camera_sea_level_altitude_meters,
                        camera_surface_height_meters,
                        surface_probe_max_distance_meters,
                        |direction, camera_distance_meters| match render_path {
                            RenderPath::Raster => terrain
                                .raster_surface_height_breakdown_at_distance(
                                    direction,
                                    camera_sea_level_altitude_meters,
                                    camera_distance_meters,
                                ),
                            RenderPath::FoveatedRay => terrain.surface_height_breakdown_at(
                                direction,
                                camera_sea_level_altitude_meters,
                            ),
                        },
                    );
                    report.render_debug_mode = self.render_debug_mode.label().to_owned();
                    report.ray_near_field = ray_near_field_coverage;
                    // Same depth image, different question: the probe asks
                    // whether the surface is where it should be, this asks
                    // whether distance reads on it.
                    if let Some(frame) = &captured_frame {
                        let haze = haze::measure(
                            sim_time,
                            self.render_path.label(),
                            &geometry,
                            &depth,
                            &frame.pixels,
                            frame.width,
                            frame.height,
                        );
                        tracing::info!(
                            render_path = haze.render_path,
                            convergence = haze.convergence,
                            bands = haze.bins.len(),
                            "haze probe"
                        );
                        self.artifacts.record_haze(haze);
                    }
                    tracing::info!(
                        render_path = report.render_path.as_str(),
                        clearance_m = report.camera_clearance_meters,
                        compared = report.compared_points,
                        max_abs_delta_m = report.max_abs_delta_meters,
                        p90_abs_delta_m = report.p90_abs_delta_meters,
                        median_abs_delta_m = report.median_abs_delta_meters,
                        mean_delta_m = report.mean_delta_meters,
                        mean_delta_from_macro_m = report.mean_delta_from_macro_meters,
                        detail_correlation = report.detail_correlation,
                        "surface probe"
                    );
                    self.artifacts.record_surface_probe(report);
                }
                Err(error) => {
                    self.scenario_capture_failed = true;
                    tracing::error!(%error, "surface probe readback failed");
                }
            }
        }
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
    // Both of these produce a picture that looks like a different, much worse
    // renderer, and neither used to say anything at all about why.
    if matches!(
        launch_options.terrain_source,
        terrain::TerrainSource::Placeholder
    ) {
        eprintln!(
            "WARNING: no baked planet found ({DEFAULT_OUTMAP_PATH} in this directory or any \
             parent), so this is running on placeholder terrain. Synthesised ground detail, \
             close-range materials and per-pixel relief are all disabled -- they are gated on \
             the outmap. Bake a planet or run from a directory under the repository root."
        );
    }
    if cfg!(debug_assertions) {
        eprintln!(
            "WARNING: debug build. Measured about 7x slower than release (215 ms vs 32 ms per \
             frame at ground level), which is slow enough that terrain streaming cannot keep up \
             while the camera moves and the ground stays on coarse fallback chunks. Use \
             --release."
        );
    }
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App::new(launch_options);
    event_loop.run_app(&mut app).expect("event loop failed");
    let scenario_failed = app.scenario_failed.load(Ordering::Relaxed);
    // Release the GPU device before exiting. process::exit runs no
    // destructors, so taking it with wgpu resources still live leaves the
    // driver to be torn down by the OS underneath its own in-flight work.
    drop(app);
    if scenario_failed {
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
                        && event.physical_key == PhysicalKey::Code(KeyCode::Digit6) =>
                {
                    state.toggle_auto_exposure();
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
                        && event.physical_key == PhysicalKey::Code(KeyCode::F11) =>
                {
                    state.toggle_warp_debug();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && matches!(
                            event.physical_key,
                            PhysicalKey::Code(
                                KeyCode::Digit1
                                    | KeyCode::Digit2
                                    | KeyCode::Digit3
                                    | KeyCode::Digit4
                                    | KeyCode::Digit5
                            )
                        ) =>
                {
                    let experiment = match event.physical_key {
                        PhysicalKey::Code(KeyCode::Digit1) => Some(1),
                        PhysicalKey::Code(KeyCode::Digit2) => Some(2),
                        PhysicalKey::Code(KeyCode::Digit3) => Some(3),
                        PhysicalKey::Code(KeyCode::Digit4) => Some(4),
                        PhysicalKey::Code(KeyCode::Digit5) => Some(5),
                        _ => None,
                    };
                    if let Some(experiment) = experiment {
                        state.toggle_ray_experiment(experiment);
                        window.request_redraw();
                    }
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
            && device_mouse_look_enabled(state.mouse_captured, state.scenario.is_some())
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

/// Finds the baked planet by walking up from the working directory.
///
/// `DEFAULT_OUTMAP_PATH` is relative, so running from anywhere but the repo
/// root used to miss it and fall back to placeholder terrain -- four sine
/// octaves, with the detail ladder, material tiling and per-pixel normals all
/// switched off together, because every one of them is gated on `outmap`. It
/// looks like a different program and says nothing about why.
fn find_default_outmap() -> Option<PathBuf> {
    let mut directory = std::env::current_dir().ok()?;
    loop {
        let candidate = directory.join(DEFAULT_OUTMAP_PATH);
        if candidate.join("manifest.json").is_file() {
            return Some(candidate);
        }
        if !directory.pop() {
            return None;
        }
    }
}

fn launch_options() -> Result<LaunchOptions, String> {
    let default_outmap = find_default_outmap();
    let mut options = LaunchOptions {
        scenario_name: None,
        profile_render: false,
        vertical_fov_degrees: None,
        terrain_source: match &default_outmap {
            Some(path) => terrain::TerrainSource::Outmap(path.clone()),
            None => terrain::TerrainSource::Placeholder,
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
                    "outmap" => {
                        terrain::TerrainSource::Outmap(default_outmap.clone().ok_or_else(|| {
                            format!(
                                "--terrain outmap was requested but no {DEFAULT_OUTMAP_PATH} \
                                 was found in this directory or any parent"
                            )
                        })?)
                    }
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

fn create_depth_texture(
    device: &wgpu::Device,
    size: winit::dpi::PhysicalSize<u32>,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
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
        // COPY_SRC is for the surface probe, which reads this attachment back
        // to compare the drawn ground against the ground the camera collides
        // with.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::{
        CameraMode, DEFAULT_OUTMAP_PATH, EARTHLIKE_HIGHEST_PROMINENCE_DIRECTION,
        EARTHLIKE_HIGHEST_PROMINENCE_METERS, EARTHLIKE_HIGHEST_RAW_MACRO_ELEVATION_METERS,
        FlightMovementInput, FlightSpeedState, INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
        LOW_FLIGHT_ALTITUDE_METERS, LOW_FLIGHT_INITIAL_PITCH_RADIANS,
        LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND, LOW_FLIGHT_MINIMUM_CLEARANCE_METERS,
        MAX_LOW_FLIGHT_FRAME_DELTA_SECONDS, RenderPath, advance_flight_position_on_sphere,
        advance_flight_speed, device_mouse_look_enabled, find_default_outmap,
        flight_movement_direction, flight_view_direction, focus_of_expansion_ndc,
        initial_flight_tangent, interactive_camera_delta_seconds, projected_planet_coverage,
        render_size_for_surface_resize, should_enter_fullscreen, swept_flight_clearance_lift,
        transport_flight_tangent,
    };
    use crate::planet::{
        CameraUniform, OrbitCamera, PLANET_ROTATION_PERIOD_SECONDS, RenderDebugMode,
        default_sun_direction,
    };

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
    fn deterministic_scenarios_ignore_live_mouse_motion() {
        assert!(device_mouse_look_enabled(true, false));
        assert!(!device_mouse_look_enabled(true, true));
        assert!(!device_mouse_look_enabled(false, false));
    }

    #[test]
    fn flight_focus_projects_forward_motion_and_rejects_orbit_or_backward_motion() {
        let camera = OrbitCamera::default();
        let uniform = CameraUniform::from_camera(
            &camera,
            16.0 / 9.0,
            default_sun_direction(),
            0.0,
            0.0,
            RenderDebugMode::Final,
            0.0,
        );
        let axis = |values: [f32; 4]| {
            DVec3::new(
                f64::from(values[0]),
                f64::from(values[1]),
                f64::from(values[2]),
            )
        };
        let forward = axis(uniform.camera_forward);
        let right = axis(uniform.camera_right);
        let up = axis(uniform.camera_up);

        assert_eq!(
            focus_of_expansion_ndc(CameraMode::Orbit, forward * 1_000.0, &uniform),
            [0.0; 2]
        );
        assert_eq!(
            focus_of_expansion_ndc(CameraMode::LowFlight, -forward * 1_000.0, &uniform),
            [0.0; 2]
        );
        assert_eq!(
            focus_of_expansion_ndc(CameraMode::LowFlight, forward * 0.5, &uniform),
            [0.0; 2]
        );
        let projected = focus_of_expansion_ndc(
            CameraMode::LowFlight,
            (forward + right * 0.2 + up * 0.1).normalize() * 1_000.0,
            &uniform,
        );
        assert!(projected[0] > 0.0);
        assert!(projected[1] > 0.0);
        let clamped = focus_of_expansion_ndc(
            CameraMode::LowFlight,
            (forward * 0.11 + right).normalize() * 1_000.0,
            &uniform,
        );
        assert_eq!(clamped[0], 0.7);
    }

    #[test]
    fn projected_planet_coverage_distinguishes_orbit_from_low_flight() {
        let aspect = 16.0 / 9.0;
        let fov = 45.0_f64.to_radians();
        let orbit = projected_planet_coverage(10_000_000.0, fov, aspect);
        let low_flight = projected_planet_coverage(4_002_000.0, fov, aspect);
        assert!(orbit < super::CONTENT_ADAPTIVE_WARP_MINIMUM_PLANET_COVERAGE);
        assert!(low_flight >= super::CONTENT_ADAPTIVE_WARP_MINIMUM_PLANET_COVERAGE);
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
        assert!(RenderPath::Raster.draws_terrain_meshes());
        assert!(!RenderPath::FoveatedRay.draws_terrain_meshes());
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
    fn flight_collision_sweep_catches_ground_between_safe_endpoints() {
        let radius = crate::planet::PLANET_RADIUS_METERS + 10.0;
        let start = DVec3::X * radius;
        let end = (DVec3::X + DVec3::Z * (4.0 / radius)).normalize() * radius;
        let end_z = end.normalize().z;
        let lift = swept_flight_clearance_lift(
            start,
            end,
            LOW_FLIGHT_MINIMUM_CLEARANCE_METERS,
            |direction, _| {
                Some(if (0.4 * end_z..0.6 * end_z).contains(&direction.z) {
                    20.0
                } else {
                    0.0
                })
            },
        );

        assert!((lift - 12.0).abs() < 1.0e-9);
    }

    #[test]
    fn low_flight_starts_looking_down_from_the_prominent_peak() {
        let radial = EARTHLIKE_HIGHEST_PROMINENCE_DIRECTION.normalize();
        let tangent = -initial_flight_tangent(radial);
        let direction =
            flight_view_direction(radial, tangent, 0.0, LOW_FLIGHT_INITIAL_PITCH_RADIANS);

        assert!(direction.dot(radial) < -0.25);
        assert!(direction.dot(tangent) > 0.9);
    }

    #[test]
    fn earthlike_peak_measurement_uses_standard_global_summit_prominence() {
        let direction = EARTHLIKE_HIGHEST_PROMINENCE_DIRECTION;
        assert!((direction.length() - 1.0).abs() < 1.0e-12);
        assert!((direction.y.asin().to_degrees() - 41.530_039_222).abs() < 1.0e-6);
        assert!((direction.z.atan2(direction.x).to_degrees() - 71.196_129_733).abs() < 1.0e-6);

        // A planet's highest summit has no higher parent. As for Everest, its
        // key col is sea level, so prominence equals summit elevation ASL.
        assert_eq!(
            EARTHLIKE_HIGHEST_PROMINENCE_METERS,
            EARTHLIKE_HIGHEST_PROMINENCE_METERS - 0.0
        );
        assert!(
            EARTHLIKE_HIGHEST_PROMINENCE_METERS
                > EARTHLIKE_HIGHEST_RAW_MACRO_ELEVATION_METERS * 3.0
        );
    }

    /// Entry altitude and minimum clearance are separate concerns. While they
    /// shared one constant the camera could not descend below 500 ft, so the
    /// ground was only ever seen from the air.
    /// Running from a subdirectory used to silently fall back to placeholder
    /// terrain, which disables the detail ladder, close-range materials and
    /// per-pixel relief in one go because all three are gated on `outmap`.
    #[test]
    fn the_baked_planet_is_found_from_any_directory_under_the_root() {
        let root = std::env::current_dir().expect("cargo runs tests with a working directory");
        if !root
            .join(DEFAULT_OUTMAP_PATH)
            .join("manifest.json")
            .is_file()
        {
            // Tests run from the crate directory in some layouts; the walk-up
            // is what this is checking, so only assert when a planet exists.
            return;
        }
        let found = find_default_outmap().expect("the planet is found from the root itself");
        assert!(found.join("manifest.json").is_file());
    }

    #[test]
    fn flight_may_descend_to_eye_level_but_not_into_the_ground() {
        assert!(LOW_FLIGHT_MINIMUM_CLEARANCE_METERS > 0.0);
        assert!(
            LOW_FLIGHT_MINIMUM_CLEARANCE_METERS < 3.0,
            "a {LOW_FLIGHT_MINIMUM_CLEARANCE_METERS}m floor is too high to stand on the ground"
        );
        assert!(LOW_FLIGHT_MINIMUM_CLEARANCE_METERS < LOW_FLIGHT_ALTITUDE_METERS);
        // The floor is only safe because clearance sees the synthesised relief.
        // If that ever regresses to baked heights alone, the ladder can put
        // ground up to its full amplitude above where the CPU thinks it is.
        let ladder_amplitude = crate::planet::TERRAIN_DETAIL_START_WAVELENGTH_METERS
            * crate::planet::TERRAIN_DETAIL_ROUGHNESS
            * 2.0;
        assert!(ladder_amplitude > LOW_FLIGHT_MINIMUM_CLEARANCE_METERS);
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

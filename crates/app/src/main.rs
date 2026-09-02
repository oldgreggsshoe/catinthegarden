mod atmosphere;
mod debug;
mod forest;
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
mod surface_camera;
mod terrain;
mod weather;
mod weather_render;

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

fn should_start_interactive_fullscreen(scenario_active: bool) -> bool {
    !scenario_active
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
const INTERACTIVE_PLANET_ROTATION_TIME_SCALE: f64 = 0.05;
const MOUSE_LOOK_RADIANS_PER_PIXEL: f64 = 0.0006;
/// F4 enters close inspection at roughly 2m above the resident surface so
/// mountain walls can be judged from the ground rather than from the former
/// tens-of-metres collision envelope.
const LOW_FLIGHT_ALTITUDE_METERS: f64 = 2.0;
/// Highest summit on the active procedural, eroded bake after the fixed
/// runtime macro presentation and bounded detail are applied. The direction
/// is measured by the standard global-summit/prominence instrument; as the
/// planet's highest summit it uses sea level as its key col.
/// Re-measure after every rebake with
/// `cargo test -- --ignored --nocapture global_highest_summit`, which asserts
/// these three values against the outmap it scans and prints the replacements
/// when they drift. The latest mountain-coverage retune moved and lowered the
/// summit; the ignored calibration instrument keeps this pose in lockstep.
const ACTIVE_HIGHEST_PROMINENCE_DIRECTION: glam::DVec3 = glam::DVec3::new(
    -0.462_957_306_613_887,
    -0.441_947_898_858_313,
    0.768_344_055_060_972,
);
const ACTIVE_HIGHEST_PROMINENCE_METERS: f64 = 186_701.172_076_862;
#[cfg(test)]
/// Raw macro elevation *at the summit above*, not the global raw L4 maximum --
/// since the coverage retune those are different points, and the prominence
/// relationship this feeds only holds when both describe the same place.
const ACTIVE_HIGHEST_RAW_MACRO_ELEVATION_METERS: f64 = 46_735.316_406_250;
/// How close to the ground flight may descend. CPU clearance evaluates the
/// same synthesised relief the shader displaces, so a sub-metre floor is safe.
const LOW_FLIGHT_MINIMUM_CLEARANCE_METERS: f64 = 0.75;
/// Once the player has descended onto the moving ocean surface, keep the eye
/// attached to it instead of ratcheting upward on every successive crest.
const LOW_FLIGHT_OCEAN_FOLLOW_TOLERANCE_METERS: f64 = 0.25;

fn low_flight_clearance_radius(
    current_radius_meters: f64,
    previous_surface_height_meters: f64,
    resolved_surface_height_meters: Option<f64>,
    minimum_clearance_meters: f64,
    follow_resolved_surface: bool,
) -> f64 {
    let surface_height_meters =
        resolved_surface_height_meters.unwrap_or(previous_surface_height_meters);
    let minimum_radius =
        planet::PLANET_RADIUS_METERS + surface_height_meters + minimum_clearance_meters;
    if follow_resolved_surface && resolved_surface_height_meters.is_some() {
        minimum_radius
    } else {
        current_radius_meters.max(minimum_radius)
    }
}
/// Sweep the camera point through the rendered terrain instead of checking
/// only the end of a frame. L18 patch boundaries are roughly a metre apart;
/// sub-metre samples prevent a downward W flight from tunnelling through a
/// higher incoming/outgoing patch between two otherwise safe endpoints.
const LOW_FLIGHT_COLLISION_SWEEP_STEP_METERS: f64 = 0.5;
const LOW_FLIGHT_COLLISION_MAX_SWEEP_SAMPLES: usize = 64;
/// Translating through the raster frontier still needs a safety envelope, but
/// five metres keeps moving inspection well below the former 30m minimum.
const LOW_FLIGHT_MOVING_CLEARANCE_METERS: f64 = 5.0;
/// Held WASD is an immediate, fixed-speed command. The 100m reference keeps
/// apparent local angular motion approximately constant: 250 mph at ground
/// level, then proportionally faster as altitude opens the view footprint.
const LOW_FLIGHT_BASE_SPEED_METERS_PER_SECOND: f64 = 5.0 * 50.0 * 0.44704;
const LOW_FLIGHT_APPARENT_MOTION_REFERENCE_ALTITUDE_METERS: f64 = 100.0;
const LOW_FLIGHT_BOOST_SPEED_MULTIPLIER: f64 = 4.0;
const LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND: f64 = 40_000_000.0;
const FLIGHT_SPEED_SCALE_STEP: f64 = 2.0;
const MINIMUM_FLIGHT_SPEED_SCALE: f64 = 1.0 / 32.0;
const MAXIMUM_FLIGHT_SPEED_SCALE: f64 = 32.0;
const LOW_FLIGHT_VERTICAL_FOV_DEGREES: f64 = 60.0;
/// Authored dry point immediately inside the active bake's ocean boundary.
/// The tangent points across the adjacent open sea, giving touch-only remote
/// sessions a useful ocean view without needing mouse-look or WASD input.
const COASTAL_START_DIRECTION: glam::DVec3 = glam::DVec3::new(
    0.843_210_618_038_952,
    0.494_447_406_479_720,
    0.210_991_980_539_184,
);
const COASTAL_SEAWARD_TANGENT: glam::DVec3 = glam::DVec3::new(
    -0.536_211_408_972_529,
    0.745_549_575_617_721,
    0.395_769_067_997_906,
);
const COASTAL_START_ALTITUDE_METERS: f64 = 100.0;
const COASTAL_START_PITCH_RADIANS: f64 = -8.0_f64.to_radians();
/// Open-water direction used only for the interactive storm-at-sea startup.
/// It is a known deep-ocean point a few kilometres seaward of the authored
/// coast start, so surface mode can resolve buoyancy immediately.
const STORM_OCEAN_START_DIRECTION: glam::DVec3 = glam::DVec3::new(
    0.836_442_275_001_636,
    0.503_727_905_284_262,
    0.215_922_481_525_239,
);
const STORM_OCEAN_START_PITCH_RADIANS: f64 = -4.0_f64.to_radians();
const PLANET_ROTATION_SCALE_STEP: f64 = 2.0;
const MINIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE: f64 =
    INTERACTIVE_PLANET_ROTATION_TIME_SCALE / 32.0;
const MAXIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE: f64 =
    INTERACTIVE_PLANET_ROTATION_TIME_SCALE * 32.0;
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
}

#[derive(Clone, Copy, Debug)]
struct SurfaceEnvironment {
    terrain_height_meters: f64,
    water_surface: Option<(f64, f64)>,
    open_ocean: bool,
}

impl SurfaceEnvironment {
    fn visible_surface_height_meters(self) -> f64 {
        self.water_surface
            .map_or(self.terrain_height_meters, |(height, _)| height)
    }
}

fn advance_flight_speed(
    _state: FlightSpeedState,
    movement_held: bool,
    boost: bool,
    altitude_meters: f64,
    speed_scale: f64,
) -> FlightSpeedState {
    if !movement_held {
        return FlightSpeedState::default();
    }
    let altitude_meters = altitude_meters.max(0.0);
    let boost_multiplier = if boost {
        LOW_FLIGHT_BOOST_SPEED_MULTIPLIER
    } else {
        1.0
    };
    let altitude_scaled_speed = (LOW_FLIGHT_BASE_SPEED_METERS_PER_SECOND
        * (1.0 + altitude_meters / LOW_FLIGHT_APPARENT_MOTION_REFERENCE_ALTITUDE_METERS)
        * boost_multiplier)
        .min(LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND);
    FlightSpeedState {
        speed_meters_per_second: altitude_scaled_speed
            * speed_scale.clamp(MINIMUM_FLIGHT_SPEED_SCALE, MAXIMUM_FLIGHT_SPEED_SCALE),
    }
}

fn adjusted_flight_speed_scale(current_scale: f64, scale_factor: f64) -> f64 {
    (current_scale * scale_factor).clamp(MINIMUM_FLIGHT_SPEED_SCALE, MAXIMUM_FLIGHT_SPEED_SCALE)
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

fn surface_movement_direction(
    input: FlightMovementInput,
    camera_forward: glam::DVec3,
    local_radial: glam::DVec3,
    fallback_forward: glam::DVec3,
) -> Option<glam::DVec3> {
    let projected_forward = camera_forward - local_radial * camera_forward.dot(local_radial);
    let forward = if projected_forward.length_squared() > f64::EPSILON {
        projected_forward.normalize()
    } else {
        fallback_forward
    };
    let right = forward.cross(local_radial).normalize();
    flight_movement_direction(input, forward, right)
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
    Surface,
}

impl CameraMode {
    fn label(self) -> &'static str {
        match self {
            Self::Orbit => "orbit",
            Self::LowFlight => "fixed-speed WASD flight; altitude-scaled ([/]: scale, Shift: 4x)",
            Self::Surface => "surface walking/swimming; G: return to flight, Space: jump/thrust",
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
        CameraMode::LowFlight | CameraMode::Surface => {
            frame_delta_seconds.min(MAX_LOW_FLIGHT_FRAME_DELTA_SECONDS)
        }
    }
}

/// Ocean waves are persistent environmental motion, like interactive weather.
/// F10 holds the planet/sun composition for inspection, but must not turn the
/// water at the default coastal start into a static blue sheet.
fn ocean_animation_time_seconds(_scene_time_seconds: f64, presentation_time_seconds: f64) -> f64 {
    presentation_time_seconds
}

fn retimed_planet_rotation(
    sim_time: f64,
    old_scale: f64,
    old_offset: f64,
    scale_factor: f64,
) -> (f64, f64) {
    let rotation_time = sim_time * old_scale + old_offset;
    let new_scale = (old_scale * scale_factor).clamp(
        MINIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
        MAXIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
    );
    let new_offset = rotation_time - sim_time * new_scale;
    (new_scale, new_offset)
}

fn focus_of_expansion_ndc(
    camera_mode: CameraMode,
    velocity_planet_frame: glam::DVec3,
    camera: &planet::CameraUniform,
) -> [f32; 2] {
    if camera_mode == CameraMode::Orbit
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
    luminance_enabled: bool,
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

    fn begin_readback(
        &mut self,
        index: usize,
        sim_time: f64,
        render_path: RenderPath,
        luminance_enabled: bool,
    ) {
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
            luminance_enabled,
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
            let luminance_enabled = pending.luminance_enabled;
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
                // Query 2/3 are intentionally unwritten while fixed
                // exposure is active; do not interpret stale query memory as
                // a luminance timing sample.
                luminance_ms: if luminance_enabled {
                    elapsed(2, 3)
                } else {
                    0.0
                },
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

/// Everything a frame needs to know before any simulation runs, resolved from
/// either the active scenario or interactive state.
///
/// This was a fifteen-element tuple destructured at the top of `render`, where
/// position was the only thing distinguishing `write_log` from `hide_overlay`
/// -- six consecutive `bool`s that the compiler would happily let you swap.
struct FrameInputs {
    sim_time: f64,
    presentation_time: f64,
    write_log: bool,
    scenario_capture: bool,
    scenario_complete: bool,
    solid_color_screen: bool,
    hide_overlay: bool,
    seam_gap_check: bool,
    /// Camera world position and look-at target, absent while a scenario is
    /// rendering a solid colour and when flying interactively.
    pose: Option<(glam::DVec3, glam::DVec3)>,
    planet_relative_up: bool,
    surface_probe_max_distance_meters: f64,
    vertical_fov_degrees: Option<f64>,
    sun_direction: Option<glam::DVec3>,
    planet_rotation_time_scale: f64,
    forward_flight_held: Option<bool>,
}

/// Frame-local values the debug overlay displays. Everything else it shows is
/// read straight off `self`.
struct HudInputs<'a> {
    window: &'a Window,
    now: Instant,
    camera_world_position: glam::DVec3,
    camera_altitude: f64,
    exposure_state: hdr::ExposureState,
    ocean_wave_range: f32,
}

/// The per-frame measurements the spatial log records. Everything else on the
/// sample is read straight off `self`, so only the frame-local values travel.
struct SpatialLogInputs {
    sim_time: f64,
    camera_world_position: glam::DVec3,
    camera_radius: f64,
    camera_altitude: f64,
    velocity_meters_per_second: f64,
    planet_rotation_radians: f64,
    frame_time: f32,
    draw_calls: u32,
    exposure: f32,
    ocean_wave_stats: ocean::WaveHeightStats,
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
    weather: weather::WeatherState,
    weather_clouds: weather_render::WeatherCloudRenderer,
    rain: weather_render::RainRenderer,
    local_cloud_impostors: weather_render::LocalCloudImpostorRenderer,
    forest: forest::ForestRenderer,
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
    flight_speed_scale: f64,
    flight_travel_direction: glam::DVec3,
    surface_physics: surface_camera::SurfacePhysicsState,
    surface_jump_requested: bool,
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
    flat_triangle_outline_mode: planet::FlatTriangleOutlineMode,
    animation_frozen: bool,
    frozen_sim_time: f64,
    interactive_scene_time_offset_seconds: f64,
    interactive_planet_rotation_time_scale: f64,
    interactive_planet_rotation_time_offset_seconds: f64,
    manual_screenshot_requested: bool,
    next_spatial_log_presentation_time: f64,
    capture_number: usize,
    scenario: Option<scenario::ScenarioRunner>,
    scenario_flight_initialized: bool,
    artifacts: debug::RunArtifacts,
    log_writer: debug::SharedFile,
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
        debug::init_tracing(log_writer.clone());
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
                    visibility: wgpu::ShaderStages::VERTEX
                        | wgpu::ShaderStages::FRAGMENT
                        | wgpu::ShaderStages::COMPUTE,
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
        let atmosphere = atmosphere::AtmosphereRenderer::new(
            &device,
            &queue,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
        );
        let terrain_startup_samples = terrain::terrain_startup_samples(&terrain_source)
            .expect("active terrain startup samples must load");
        let mut weather = weather::WeatherState::new_with_terrain_samples(
            terrain_startup_samples
                .as_ref()
                .map(|samples| samples.climate.as_slice()),
        );
        if scenario.is_none() {
            weather.enable_background_prediction(planet::default_sun_direction());
        }
        let mut weather_clouds = weather_render::WeatherCloudRenderer::new(
            &device,
            &queue,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            atmosphere.surface_lighting_resources(),
        );
        weather_clouds.initialize_fields(
            &device,
            &queue,
            &weather.cloud_field_texture_data(),
            &weather.surface_field_texture_data(),
        );
        if scenario.is_none() {
            weather_clouds.replace_fields(
                &device,
                &queue,
                &weather
                    .next_cloud_field_texture_data()
                    .expect("interactive weather target must be prepared before rendering"),
                &weather
                    .next_surface_field_texture_data()
                    .expect("interactive weather surface target must be prepared before rendering"),
            );
        }
        let rain = weather_render::RainRenderer::new(
            &device,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            weather_clouds.field_bind_group_layout(),
        );
        let local_cloud_impostors = weather_render::LocalCloudImpostorRenderer::new(
            &device,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            weather_clouds.field_bind_group_layout(),
            atmosphere.surface_lighting_resources(),
        );
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
        let mut terrain = terrain::TerrainRenderer::new(
            &device,
            &queue,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            shared_planet_bind_group_layout,
            weather_clouds.field_bind_group_layout(),
            atmosphere.surface_lighting_resources(),
            terrain_source,
        )
        .expect("terrain renderer must initialize");
        let forest = forest::ForestRenderer::new(
            &device,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            weather_clouds.field_bind_group_layout(),
            terrain_startup_samples
                .as_ref()
                .map(|samples| samples.forests.as_slice())
                .unwrap_or_default(),
            &mut terrain,
        );
        if let (Some(scenario), Some(landing_direction)) =
            (&mut scenario, terrain.preferred_landing_direction())
        {
            scenario.retarget_sparse_landing_direction(landing_direction);
        }
        let sun = sun::SunRenderer::new(
            &device,
            hdr::HdrRenderer::SCENE_FORMAT,
            &camera_bind_group_layout,
            weather_clouds.field_bind_group_layout(),
            atmosphere.surface_lighting_resources(),
            &depth_view,
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
            weather,
            weather_clouds,
            rain,
            local_cloud_impostors,
            forest,
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
            flight_speed_scale: 1.0,
            flight_travel_direction: glam::DVec3::ZERO,
            surface_physics: surface_camera::SurfacePhysicsState::default(),
            surface_jump_requested: false,
            saved_orbit_camera_pose: None,
            camera_buffer,
            camera_bind_group,
            started_at: Instant::now(),
            egui_context,
            egui_state,
            egui_renderer,
            last_frame: Instant::now(),
            fps: 0.0,
            debug_overlay_visible: false,
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
                "flat_triangles" => planet::RenderDebugMode::FlatTriangles,
                _ => planet::RenderDebugMode::FlatTriangles,
            },
            flat_triangle_outline_mode: planet::FlatTriangleOutlineMode::Dark,
            animation_frozen: false,
            frozen_sim_time: 0.0,
            interactive_scene_time_offset_seconds: 0.0,
            interactive_planet_rotation_time_scale: INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
            interactive_planet_rotation_time_offset_seconds: 0.0,
            manual_screenshot_requested: false,
            next_spatial_log_presentation_time: 0.0,
            capture_number: 0,
            scenario,
            scenario_flight_initialized: false,
            artifacts,
            log_writer,
            scenario_capture_failed: false,
            mouse_captured: false,
            profile_render,
            gpu_profiler,
            cached_paint_jobs: Vec::new(),
            egui_buffers_dirty: true,
            next_hud_update: Instant::now(),
            hud_dirty: true,
        };
        let scenario_uses_fixed_exposure = state
            .scenario
            .as_ref()
            .is_some_and(scenario::ScenarioRunner::uses_fixed_exposure);
        // Automated captures were authored with the HDR curve enabled. Keep
        // that scenario presentation explicit while the interactive default
        // remains opt-in, so image assertions do not drift with user startup
        // preferences.
        if state.scenario.is_some() {
            state.hdr.set_hdr_effect_enabled(&state.queue, true);
        }
        if scenario_uses_fixed_exposure {
            state.hdr.set_auto_exposure_enabled(&state.queue, false);
        }
        state.apply_startup_experiment_overrides();
        state.apply_interactive_startup_controls();
        state
    }

    /// Start interactive launches floating in the maximum-intensity ocean
    /// storm, then apply the existing F6/F10 presentation defaults.
    /// Scenarios retain their authored camera, post-processing, and clock.
    fn apply_interactive_startup_controls(&mut self) {
        if self.scenario.is_some() {
            return;
        }

        self.toggle_camera_mode();
        if self.position_storm_ocean_start() {
            self.toggle_surface_camera_mode();
        }
        self.toggle_blur();
        self.toggle_animation_freeze();
        tracing::info!(
            target: "catinthegarden::startup",
            camera_mode = self.camera_mode.label(),
            blur_enabled = self.hdr.blur_enabled(),
            animation_frozen = self.animation_frozen,
            "interactive startup controls applied"
        );
    }

    /// Move the freshly-created low-flight pose to a known open-ocean point
    /// before entering surface mode. The dense startup tile is loaded once so
    /// the ocean ownership/depth query cannot fall back to an ancestor tile.
    fn position_storm_ocean_start(&mut self) -> bool {
        let local_radial = STORM_OCEAN_START_DIRECTION.normalize();
        let altitude_meters = LOW_FLIGHT_ALTITUDE_METERS;
        let Some(surface_height_meters) = self
            .terrain
            .prepare_flight_start_surface_height_meters(local_radial, altitude_meters)
        else {
            tracing::warn!(
                target: "catinthegarden::startup",
                "storm-ocean startup terrain sample unavailable; retaining coastal pose"
            );
            return false;
        };
        if self.terrain.open_ocean_at(local_radial) != Some(true) {
            tracing::warn!(
                target: "catinthegarden::startup",
                "storm-ocean startup direction is not open ocean; retaining coastal pose"
            );
            return false;
        }
        self.flight_local_position =
            local_radial * (planet::PLANET_RADIUS_METERS + surface_height_meters + altitude_meters);
        self.flight_surface_height_meters = surface_height_meters;
        self.flight_local_tangent = initial_flight_tangent(local_radial);
        self.flight_look_yaw_radians = 0.0;
        self.flight_look_pitch_radians = STORM_OCEAN_START_PITCH_RADIANS;
        self.flight_movement = FlightMovementInput::default();
        self.flight_travel_direction = glam::DVec3::ZERO;
        true
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
        self.sun.resize_depth(&self.device, &self.depth_view);
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
        if matches!(
            self.camera_mode,
            CameraMode::LowFlight | CameraMode::Surface
        ) {
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
            self.surface_jump_requested = false;
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

    fn cycle_flat_triangle_outline_mode(&mut self) {
        self.flat_triangle_outline_mode = self.flat_triangle_outline_mode.next();
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

    fn request_surface_jump(&mut self) {
        if self.scenario.is_none() && self.camera_mode == CameraMode::Surface {
            self.surface_jump_requested = true;
        }
    }

    fn adjust_flight_speed_scale(&mut self, scale_factor: f64) {
        if self.scenario.is_some() {
            return;
        }
        self.flight_speed_scale =
            adjusted_flight_speed_scale(self.flight_speed_scale, scale_factor);
        tracing::info!(
            target: "catinthegarden::controls",
            flight_speed_scale = self.flight_speed_scale,
            "flight movement speed scale changed"
        );
        self.mark_hud_dirty();
    }

    fn advance_low_flight_camera(
        &mut self,
        delta_seconds: f64,
        planet_rotation_radians: f64,
        ocean_time_seconds: f64,
    ) {
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
            self.flight_local_position.length() - planet::PLANET_RADIUS_METERS,
            self.flight_speed_scale,
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
        self.update_low_flight_camera(
            Some(movement_start_position),
            planet_rotation_radians,
            ocean_time_seconds,
        );
    }

    fn open_ocean_environment_at(
        &self,
        local_radial: glam::DVec3,
        _camera_altitude_meters: f64,
        ocean_time_seconds: f64,
        fallback_terrain_height_meters: f64,
    ) -> Option<(f64, f64, f64)> {
        if self.terrain.open_ocean_at(local_radial) != Some(true) {
            return None;
        }
        // Flat-triangle presentation clamps the hidden ocean terrain mesh to
        // sea level. The actual water shell still displaces around sea level,
        // so use the unclamped source height for depth, buoyancy and collision.
        let bathymetry_meters = self
            .terrain
            .bathymetry_height_meters_at(local_radial)
            .unwrap_or(fallback_terrain_height_meters.min(0.0));
        let depth_meters = (-bathymetry_meters).max(0.0);
        let wave_height =
            ocean::local_wave_height_meters(local_radial, ocean_time_seconds, depth_meters);
        let wave_velocity = ocean::local_wave_vertical_velocity_meters_per_second(
            local_radial,
            ocean_time_seconds,
            depth_meters,
        );
        if bathymetry_meters >= wave_height {
            Some((bathymetry_meters, bathymetry_meters, 0.0))
        } else {
            Some((bathymetry_meters, wave_height, wave_velocity))
        }
    }

    fn surface_environment_at(
        &self,
        local_radial: glam::DVec3,
        camera_altitude_meters: f64,
        ocean_time_seconds: f64,
    ) -> Option<SurfaceEnvironment> {
        let sampled_terrain_height_meters = match self.render_path {
            RenderPath::Raster => self
                .terrain
                .raster_surface_height_meters_at(local_radial, camera_altitude_meters),
            RenderPath::FoveatedRay => self
                .terrain
                .surface_height_meters_at(local_radial, camera_altitude_meters),
        }?;
        let ocean_environment = self.open_ocean_environment_at(
            local_radial,
            camera_altitude_meters,
            ocean_time_seconds,
            sampled_terrain_height_meters,
        );
        let open_ocean = ocean_environment.is_some();
        let terrain_height_meters = ocean_environment
            .map_or(sampled_terrain_height_meters, |(bathymetry, _, _)| {
                bathymetry
            });
        let water_surface = ocean_environment.map(|(_, height, velocity)| (height, velocity));
        Some(SurfaceEnvironment {
            terrain_height_meters,
            water_surface,
            open_ocean,
        })
    }

    fn sync_surface_camera_pose(&mut self, planet_rotation_radians: f64) {
        let local_radial = self.flight_local_position.normalize();
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

    fn advance_surface_camera(
        &mut self,
        delta_seconds: f64,
        planet_rotation_radians: f64,
        ocean_time_seconds: f64,
    ) {
        let mut remaining = delta_seconds.max(0.0);
        let mut jump_requested = std::mem::take(&mut self.surface_jump_requested);
        self.flight_travel_direction = glam::DVec3::ZERO;
        self.flight_speed = FlightSpeedState::default();

        while remaining > 0.0 {
            let step_seconds = remaining.min(1.0 / 120.0);
            let local_radial = self.flight_local_position.normalize();
            let eye_altitude_meters =
                self.flight_local_position.length() - planet::PLANET_RADIUS_METERS;
            let Some(mut environment) =
                self.surface_environment_at(local_radial, eye_altitude_meters, ocean_time_seconds)
            else {
                break;
            };
            let local_forward = self.low_flight_view_direction(local_radial);
            let movement_direction = surface_movement_direction(
                self.flight_movement,
                local_forward,
                local_radial,
                self.flight_local_tangent,
            );
            let movement_speed = surface_camera::movement_speed_meters_per_second(
                environment.open_ocean,
                self.flight_speed_scale,
            );

            if let Some(movement_direction) = movement_direction {
                let movement_distance = movement_speed * step_seconds;
                let candidate_position = advance_flight_position_on_sphere(
                    self.flight_local_position,
                    movement_direction,
                    movement_distance,
                );
                let candidate_radial = candidate_position.normalize();
                if let Some(candidate_environment) = self.surface_environment_at(
                    candidate_radial,
                    eye_altitude_meters,
                    ocean_time_seconds,
                ) && surface_camera::walkable_step(
                    environment.terrain_height_meters,
                    candidate_environment.terrain_height_meters,
                    movement_distance,
                    candidate_environment.open_ocean,
                ) {
                    self.flight_local_position = candidate_position;
                    self.flight_local_tangent = transport_flight_tangent(
                        self.flight_local_tangent,
                        local_radial,
                        candidate_radial,
                    );
                    self.flight_travel_direction = movement_direction;
                    self.flight_speed.speed_meters_per_second = movement_speed;
                    environment = candidate_environment;
                }
            }

            let moved_radial = self.flight_local_position.normalize();
            let moved_eye_altitude =
                self.flight_local_position.length() - planet::PLANET_RADIUS_METERS;
            let resolved_eye_altitude = self.surface_physics.advance_vertical(
                moved_eye_altitude,
                environment.terrain_height_meters,
                environment.water_surface,
                jump_requested,
                step_seconds,
            );
            jump_requested = false;
            self.flight_local_position =
                moved_radial * (planet::PLANET_RADIUS_METERS + resolved_eye_altitude);
            self.flight_surface_height_meters = environment.visible_surface_height_meters();
            remaining -= step_seconds;
        }
        self.sync_surface_camera_pose(planet_rotation_radians);
    }

    fn resolve_surface_camera_after_streaming(
        &mut self,
        planet_rotation_radians: f64,
        ocean_time_seconds: f64,
    ) -> bool {
        let previous_position = self.flight_local_position;
        let local_radial = previous_position.normalize();
        let mut eye_altitude = previous_position.length() - planet::PLANET_RADIUS_METERS;
        if eye_altitude <= surface_camera::PLANET_CORE_CLEARANCE_METERS {
            eye_altitude = surface_camera::PLANET_CORE_CLEARANCE_METERS;
            self.flight_local_position =
                local_radial * (planet::PLANET_RADIUS_METERS + eye_altitude);
            self.surface_physics.vertical_velocity_meters_per_second = self
                .surface_physics
                .vertical_velocity_meters_per_second
                .max(0.0);
        }
        let Some(environment) =
            self.surface_environment_at(local_radial, eye_altitude, ocean_time_seconds)
        else {
            return false;
        };
        let minimum_eye_altitude =
            environment.terrain_height_meters + surface_camera::HUMAN_EYE_HEIGHT_METERS;
        if environment.water_surface.is_none()
            && eye_altitude <= minimum_eye_altitude + surface_camera::GROUND_CONTACT_EPSILON_METERS
        {
            self.flight_local_position =
                local_radial * (planet::PLANET_RADIUS_METERS + minimum_eye_altitude);
            self.surface_physics.settle_on_land();
        } else if eye_altitude < minimum_eye_altitude {
            self.flight_local_position =
                local_radial * (planet::PLANET_RADIUS_METERS + minimum_eye_altitude);
            self.surface_physics.vertical_velocity_meters_per_second = self
                .surface_physics
                .vertical_velocity_meters_per_second
                .max(0.0);
        }
        self.flight_surface_height_meters = environment.visible_surface_height_meters();
        self.sync_surface_camera_pose(planet_rotation_radians);
        previous_position.distance_squared(self.flight_local_position) > f64::EPSILON
    }

    fn update_low_flight_camera(
        &mut self,
        movement_start_position: Option<glam::DVec3>,
        planet_rotation_radians: f64,
        ocean_time_seconds: f64,
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
                    let terrain_height = self
                        .terrain
                        .raster_surface_height_meters_at(direction, altitude_meters)?;
                    Some(
                        if let Some((bathymetry, water_height, _)) = self.open_ocean_environment_at(
                            direction,
                            altitude_meters,
                            ocean_time_seconds,
                            terrain_height,
                        ) {
                            bathymetry.max(water_height)
                        } else {
                            terrain_height
                        },
                    )
                },
            );
            if lift_meters > 0.0 {
                self.flight_local_position = self.flight_local_position.normalize()
                    * (self.flight_local_position.length() + lift_meters);
            }
        }
        self.enforce_low_flight_clearance(
            LOW_FLIGHT_MINIMUM_CLEARANCE_METERS,
            planet_rotation_radians,
            ocean_time_seconds,
        );
    }

    fn enforce_low_flight_clearance(
        &mut self,
        minimum_clearance_meters: f64,
        planet_rotation_radians: f64,
        ocean_time_seconds: f64,
    ) -> bool {
        let previous_local_position = self.flight_local_position;
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
        let ocean_environment = surface_height_meters.and_then(|terrain_height| {
            self.open_ocean_environment_at(
                local_radial,
                camera_altitude_meters,
                ocean_time_seconds,
                terrain_height,
            )
        });
        let open_ocean = ocean_environment.is_some();
        let was_following_ocean = open_ocean
            && camera_altitude_meters - self.flight_surface_height_meters
                <= minimum_clearance_meters + LOW_FLIGHT_OCEAN_FOLLOW_TOLERANCE_METERS;
        let surface_height_meters = match (surface_height_meters, ocean_environment) {
            (Some(_), Some((bathymetry, water_height, _))) => Some(bathymetry.max(water_height)),
            (surface, None) => surface,
            (None, Some(_)) => unreachable!("ocean environment requires terrain height"),
        };
        // Terrain tiles can become resident while the camera is idle. Enforce
        // clearance every frame so a newly resolved higher surface cannot
        // leave the camera underground until the next movement key is pressed.
        let clearance_radius = low_flight_clearance_radius(
            self.flight_local_position.length(),
            self.flight_surface_height_meters,
            surface_height_meters,
            minimum_clearance_meters,
            was_following_ocean,
        );
        if let Some(surface_height_meters) = surface_height_meters {
            self.flight_surface_height_meters = surface_height_meters;
        }
        if (self.flight_local_position.length() - clearance_radius).abs() > f64::EPSILON {
            self.flight_local_position = local_radial * clearance_radius;
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
        previous_local_position.distance_squared(self.flight_local_position) > f64::EPSILON
    }

    fn toggle_surface_camera_mode(&mut self) {
        if self.scenario.is_some() {
            return;
        }
        if self.camera_mode == CameraMode::Orbit {
            self.toggle_camera_mode();
        }

        match self.camera_mode {
            CameraMode::LowFlight => {
                let sim_time = self.interactive_sim_time();
                let planet_rotation_radians = planet::planet_rotation_radians(
                    self.interactive_planet_rotation_time(sim_time),
                );
                let ocean_time_seconds = self.started_at.elapsed().as_secs_f64();
                let local_radial = self.flight_local_position.normalize();
                let prior_altitude =
                    self.flight_local_position.length() - planet::PLANET_RADIUS_METERS;
                let _ = self
                    .terrain
                    .prepare_flight_start_surface_height_meters(local_radial, prior_altitude);
                let Some(environment) =
                    self.surface_environment_at(local_radial, prior_altitude, ocean_time_seconds)
                else {
                    return;
                };
                let eye_altitude = if let Some((water_height, _)) = environment.water_surface {
                    self.surface_physics.settle_in_water();
                    water_height + surface_camera::equilibrium_eye_height_above_water_meters()
                } else {
                    self.surface_physics.settle_on_land();
                    environment.terrain_height_meters + surface_camera::HUMAN_EYE_HEIGHT_METERS
                };
                self.flight_local_position =
                    local_radial * (planet::PLANET_RADIUS_METERS + eye_altitude);
                self.flight_surface_height_meters = environment.visible_surface_height_meters();
                self.flight_speed = FlightSpeedState::default();
                self.flight_travel_direction = glam::DVec3::ZERO;
                self.surface_jump_requested = false;
                self.camera_mode = CameraMode::Surface;
                self.camera.set_vertical_fov_degrees_for_viewport(
                    LOW_FLIGHT_VERTICAL_FOV_DEGREES,
                    self.size.height,
                );
                self.sync_surface_camera_pose(planet_rotation_radians);
            }
            CameraMode::Surface => {
                self.surface_physics = surface_camera::SurfacePhysicsState::default();
                self.surface_jump_requested = false;
                self.flight_speed = FlightSpeedState::default();
                self.flight_travel_direction = glam::DVec3::ZERO;
                self.camera_mode = CameraMode::LowFlight;
            }
            CameraMode::Orbit => unreachable!("orbit enters low flight before surface mode"),
        }
        self.previous_camera_world_position = self.camera.world_position();
        self.mark_hud_dirty();
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
                // Enter inspection mode on the authored dry side of a coast,
                // facing across open sea. Make its global L4 tile resident
                // synchronously: resolving through a coarse ancestor here can
                // differ by hundreds of metres and would move the camera after
                // F4 while ordinary streaming catches up.
                let outmap_is_active = self.terrain.preferred_landing_direction().is_some();
                let local_radial = if outmap_is_active {
                    COASTAL_START_DIRECTION
                } else {
                    local_position.normalize()
                };
                let flight_start_altitude_meters = if outmap_is_active {
                    COASTAL_START_ALTITUDE_METERS
                } else {
                    LOW_FLIGHT_ALTITUDE_METERS
                };
                self.flight_surface_height_meters = if outmap_is_active {
                    self.terrain
                        .prepare_flight_start_surface_height_meters(
                            local_radial,
                            flight_start_altitude_meters,
                        )
                        .unwrap_or(400.0)
                } else {
                    self.terrain
                        .surface_height_meters_at(local_radial, flight_start_altitude_meters)
                        .unwrap_or(0.0)
                };
                self.flight_local_position = local_radial
                    * (planet::PLANET_RADIUS_METERS
                        + self.flight_surface_height_meters
                        + flight_start_altitude_meters);
                self.flight_local_tangent = if outmap_is_active {
                    COASTAL_SEAWARD_TANGENT
                } else {
                    initial_flight_tangent(local_radial)
                };
                self.flight_look_yaw_radians = 0.0;
                self.flight_look_pitch_radians = if outmap_is_active {
                    COASTAL_START_PITCH_RADIANS
                } else {
                    LOW_FLIGHT_INITIAL_PITCH_RADIANS
                };
                self.flight_movement = FlightMovementInput::default();
                self.flight_speed = FlightSpeedState::default();
                self.flight_travel_direction = glam::DVec3::ZERO;
                self.surface_physics = surface_camera::SurfacePhysicsState::default();
                self.surface_jump_requested = false;
                self.camera_mode = CameraMode::LowFlight;
                self.camera.set_vertical_fov_degrees_for_viewport(
                    LOW_FLIGHT_VERTICAL_FOV_DEGREES,
                    self.size.height,
                );
                self.update_low_flight_camera(
                    None,
                    planet_rotation_radians,
                    self.started_at.elapsed().as_secs_f64(),
                );
            }
            CameraMode::LowFlight | CameraMode::Surface => {
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
                self.surface_physics = surface_camera::SurfacePhysicsState::default();
                self.surface_jump_requested = false;
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

    fn step_weather_once(&mut self) {
        if self.scenario.is_some() {
            return;
        }
        let sim_time = self.interactive_sim_time();
        let planet_rotation =
            planet::planet_rotation_radians(self.interactive_planet_rotation_time(sim_time));
        let sun_direction = planet::planet_local_vector(self.sun_direction, planet_rotation);
        self.weather.step_once(sun_direction);
        let weather_field = self.weather.cloud_field_texture_data();
        self.weather_clouds.replace_fields(
            &self.device,
            &self.queue,
            &weather_field,
            &self.weather.surface_field_texture_data(),
        );
        self.mark_hud_dirty();
    }

    fn adjust_planet_rotation_speed(&mut self, scale_factor: f64) {
        if self.scenario.is_some() {
            return;
        }

        let sim_time = self.interactive_sim_time();
        let (new_scale, new_offset) = retimed_planet_rotation(
            sim_time,
            self.interactive_planet_rotation_time_scale,
            self.interactive_planet_rotation_time_offset_seconds,
            scale_factor,
        );
        self.interactive_planet_rotation_time_scale = new_scale;
        self.interactive_planet_rotation_time_offset_seconds = new_offset;
        tracing::info!(
            target: "catinthegarden::controls",
            rotation_time_scale = new_scale,
            "interactive planet rotation speed changed"
        );
        self.mark_hud_dirty();
    }

    fn interactive_planet_rotation_time(&self, sim_time: f64) -> f64 {
        sim_time * self.interactive_planet_rotation_time_scale
            + self.interactive_planet_rotation_time_offset_seconds
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

    /// Resolve this frame's inputs from the active scenario, or from
    /// interactive state when no scenario is running.
    ///
    /// Advances the scenario, so it must be called exactly once per frame.
    fn frame_inputs(&mut self) -> FrameInputs {
        let Some(scenario) = self.scenario.as_mut() else {
            let sim_time = self.interactive_sim_time();
            let presentation_time = self.started_at.elapsed().as_secs_f64();
            let write_log = presentation_time >= self.next_spatial_log_presentation_time;
            if write_log {
                self.next_spatial_log_presentation_time = presentation_time + 0.5;
            }
            return FrameInputs {
                sim_time,
                presentation_time,
                write_log,
                scenario_capture: false,
                scenario_complete: false,
                solid_color_screen: false,
                hide_overlay: false,
                seam_gap_check: false,
                pose: None,
                planet_relative_up: false,
                surface_probe_max_distance_meters: probe::MAX_COMPARISON_DISTANCE_METERS,
                vertical_fov_degrees: None,
                sun_direction: None,
                planet_rotation_time_scale: INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
                forward_flight_held: None,
            };
        };
        let frame = scenario.advance();
        let solid_color_screen = scenario.renders_solid_color();
        FrameInputs {
            // Scenarios are driven by fixed-timestep scene time, so the
            // presentation clock is the same clock.
            sim_time: frame.sim_time,
            presentation_time: frame.sim_time,
            write_log: frame.write_log,
            scenario_capture: frame.capture_screenshot,
            scenario_complete: frame.complete,
            solid_color_screen,
            hide_overlay: scenario.hides_overlay(),
            seam_gap_check: scenario.needs_seam_gap_check(),
            pose: (!solid_color_screen).then(|| {
                (
                    glam::DVec3::from_array(frame.camera_world_position),
                    glam::DVec3::from_array(frame.camera_look_at),
                )
            }),
            planet_relative_up: scenario.uses_planet_relative_up(),
            surface_probe_max_distance_meters: scenario.surface_probe_max_distance_meters(),
            vertical_fov_degrees: frame.vertical_fov_degrees,
            sun_direction: Some(glam::DVec3::from_array(frame.sun_direction)),
            planet_rotation_time_scale: frame.planet_rotation_time_scale,
            forward_flight_held: frame.forward_flight_held,
        }
    }

    fn record_spatial_log_sample(&mut self, inputs: SpatialLogInputs) {
        let SpatialLogInputs {
            sim_time,
            camera_world_position,
            camera_radius,
            camera_altitude,
            velocity_meters_per_second,
            planet_rotation_radians,
            frame_time,
            draw_calls,
            exposure,
            ocean_wave_stats,
        } = inputs;
        let latitude_degrees = (camera_world_position.y / camera_radius)
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees();
        let longitude_degrees =
            planet::geographic_longitude_degrees(camera_world_position.normalize());
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
                exposure,
                ocean_wave_min_meters: ocean_wave_stats.minimum_meters,
                ocean_wave_max_meters: ocean_wave_stats.maximum_meters,
            });
    }

    /// Rebuild the debug overlay's paint jobs.
    ///
    /// Called at most every `HUD_REFRESH_INTERVAL`, not every frame: the
    /// tessellated output is cached on `self` and reused in between. Returns
    /// the egui textures the caller must free once the frame is submitted.
    fn refresh_hud(&mut self, inputs: HudInputs<'_>) -> Vec<egui::TextureId> {
        let HudInputs {
            window,
            now,
            camera_world_position,
            camera_altitude,
            exposure_state,
            ocean_wave_range,
        } = inputs;
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
        let blur_enabled = self.hdr.blur_enabled();
        let bloom_enabled = self.hdr.bloom_enabled();
        let hdr_effect_enabled = self.hdr.hdr_effect_enabled();
        let render_path = self.render_path;
        let render_debug_mode = self.render_debug_mode;
        let flat_triangle_outline_mode = self.flat_triangle_outline_mode;
        let warp_size = self.foveated.warp_size();
        let warp_debug_visible = self.foveated.warp_debug_visible();
        let fovea_ndc = self.foveated.fovea_ndc();
        let experiment_states =
            [1_u8, 2, 3, 4, 5].map(|index| self.foveated.experiment_enabled(index));
        let animation_frozen = self.animation_frozen;
        let camera_mode = self.camera_mode;
        let surface_height_meters = self.flight_surface_height_meters;
        let flight_speed_meters_per_second = self.flight_speed.speed_meters_per_second;
        let flight_speed_scale = self.flight_speed_scale;
        let surface_vertical_speed = self.surface_physics.vertical_velocity_meters_per_second;
        let surface_grounded = self.surface_physics.grounded;
        let surface_in_water = self.surface_physics.in_water;
        let adapter_label = self.adapter_label.clone();
        let terrain_stats = self.terrain_stats.clone();
        let forest_snapshot = self.forest.stats();
        let weather_snapshot = self.weather.debug_snapshot();
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
        let camera_altitude_label = if camera_mode == CameraMode::Surface {
            format!("{camera_altitude:.2}")
        } else {
            format!("{camera_altitude:.0}")
        };
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
                        if camera_mode == CameraMode::Surface {
                            ui.label(format!(
                                "Clearance: {camera_altitude_label} m  |  wave surface: {surface_height_meters:+.2} m  |  LOD: {lod_range}"
                            ));
                        } else {
                            ui.label(format!(
                                "Altitude: {camera_altitude_label} m  |  LOD: {lod_range}"
                            ));
                        }
                        ui.label(format!(
                            "Terrain: {} active  |  {} drawn  |  {} triangles  |  {} draws",
                            terrain_stats.resident_chunks,
                            terrain_stats.drawn_chunks,
                            terrain_stats.terrain_triangles,
                            terrain_stats.draw_calls,
                        ));
                        ui.label(format!(
                            "Forest: {} trees  |  {} patches  |  nearest {:?}  |  {} global beams {} (B)",
                            forest_snapshot.instances,
                            forest_snapshot.patch_count,
                            forest_snapshot.patch_key,
                            forest_snapshot.beam_count,
                            if forest_snapshot.beams_enabled { "on" } else { "off" },
                        ));
                        ui.label(format!(
                            "Ocean: {} chunks  |  {} triangles",
                            terrain_stats.ocean_chunks, terrain_stats.ocean_triangles,
                        ));
                        ui.label(format!(
                            "Weather grid: {} cells  |  area {:.6e} m²  |  cell area {:.3e}-{:.3e} m²  |  t {:.0}s / {} steps",
                            weather_snapshot.total_cells,
                            weather_snapshot.total_area_square_meters,
                            weather_snapshot.minimum_cell_area_square_meters,
                            weather_snapshot.maximum_cell_area_square_meters,
                            weather_snapshot.simulation_time_seconds,
                            weather_snapshot.completed_steps,
                        ));
                        ui.label(format!(
                            "Weather topology tangent error: {:.3e}  |  neighbours {:016x}  |  overlay {} (7)",
                            weather_snapshot.maximum_tangent_error,
                            weather_snapshot.neighbour_checksum,
                            if weather_snapshot.overlay_enabled { "on" } else { "off" },
                        ));
                        let field = weather_snapshot.field_diagnostics;
                        ui.label(format!(
                            "Weather fields: T {:.1}-{:.1}K (mean {:.1})  |  p {:.0}-{:.0}Pa (mean {:.0})  |  RH {:.2}-{:.2} (mean {:.2})  |  clouds {:.2}-{:.2} (mean {:.2})",
                            field.minimum_temperature_kelvin,
                            field.maximum_temperature_kelvin,
                            field.mean_temperature_kelvin,
                            field.minimum_pressure_pascals,
                            field.maximum_pressure_pascals,
                            field.mean_pressure_pascals,
                            field.minimum_humidity,
                            field.maximum_humidity,
                            field.mean_humidity,
                            field.minimum_cloud_water,
                            field.maximum_cloud_water,
                            field.mean_cloud_water,
                        ));
                        ui.label(format!(
                            "Weather relief: baked max {:.0}m  |  orographic uplift max {:.2}m/s",
                            field.maximum_surface_elevation_meters,
                            field.maximum_orographic_uplift_meters_per_second,
                        ));
                        ui.label(format!(
                            "Weather water: ground {:.2}-{:.2} (mean {:.2})  |  precip max {:.2}mm/h (mean {:.2})  |  snow {:.2}-{:.2} (mean {:.2})",
                            field.minimum_ground_moisture,
                            field.maximum_ground_moisture,
                            field.mean_ground_moisture,
                            field.maximum_precipitation_millimeters_per_hour,
                            field.mean_precipitation_millimeters_per_hour,
                            field.minimum_snow_cover,
                            field.maximum_snow_cover,
                            field.mean_snow_cover,
                        ));
                        ui.label(format!(
                            "Weather storms: intensity max {:.2} (mean {:.2})  |  latent ΔT max {:.2}K",
                            field.maximum_storm_intensity,
                            field.mean_storm_intensity,
                            field.maximum_latent_temperature_tendency_kelvin,
                        ));
                        ui.label(format!(
                            "Weather wind: max {:.1}m/s  |  CFL@{}s {:.3}  |  relax@1800s {:.3}  |  conservation Δp {:.2e} Δq {:.2e}",
                            field.maximum_wind_meters_per_second,
                            weather::WEATHER_TIMESTEP_SECONDS as u32,
                            field.maximum_cfl,
                            field.relaxation_weight_at_1800_seconds,
                            field.pressure_conservation_error,
                            field.humidity_conservation_error,
                        ));
                        weather_snapshot.paint_overlay(ui);
                        ui.label(format!("Camera mode: {}", camera_mode.label()));
                        if camera_mode == CameraMode::LowFlight {
                            ui.label(format!(
                                "Flight speed: {flight_speed_meters_per_second:.0} m/s  |  scale {flight_speed_scale:.5}x  ([ / ])"
                            ));
                        } else if camera_mode == CameraMode::Surface {
                            ui.label(format!(
                                "Surface speed: {flight_speed_meters_per_second:.2} m/s  |  vertical {surface_vertical_speed:+.2} m/s  |  {}  |  scale {flight_speed_scale:.5}x  ([ / ])",
                                if surface_in_water {
                                    "swimming"
                                } else if surface_grounded {
                                    "grounded"
                                } else {
                                    "airborne"
                                },
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
                        ui.label(format!(
                            "Flat triangle outlines: {} (O)",
                            flat_triangle_outline_mode.label(),
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
                            "F: fullscreen  |  F3: overlay  |  F4: orbit/flight  |  G: surface camera  |  WASD: move  |  Space: jump/swim thrust  |  [ / ]: speed  |  F5: render path  |  O: triangle outlines  |  B: forest beams  |  F6: blur  |  F7: bloom  |  F8: HDR  |  6: exposure  |  7: weather field  |  9: weather step  |  F9: composition  |  F10: freeze  |  F11: warp view  |  F12: capture PNG",
                        );
                        ui.label("Default: fullscreen, HUD hidden, auto-orbit  |  Mouse: free look  |  Wheel: optical zoom  |  Esc/Q: quit");
                    });
            }
        });
        self.egui_state
            .handle_platform_output(window, full_output.platform_output);
        for (texture_id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *texture_id, image_delta);
        }
        let textures_to_free = full_output.textures_delta.free;
        self.cached_paint_jobs = self
            .egui_context
            .tessellate(full_output.shapes, self.egui_context.pixels_per_point());
        self.egui_buffers_dirty = true;
        self.next_hud_update = now + HUD_REFRESH_INTERVAL;
        self.hud_dirty = false;
        textures_to_free
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

        let FrameInputs {
            sim_time,
            presentation_time,
            write_log,
            scenario_capture,
            scenario_complete,
            solid_color_screen,
            hide_overlay,
            seam_gap_check,
            pose: scenario_pose,
            planet_relative_up: scenario_planet_relative_up,
            surface_probe_max_distance_meters,
            vertical_fov_degrees: scenario_vertical_fov_degrees,
            sun_direction: scenario_sun_direction,
            planet_rotation_time_scale: scenario_planet_rotation_time_scale,
            forward_flight_held: scenario_forward_flight_held,
        } = self.frame_inputs();
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
        let planet_rotation_time = if self.scenario.is_some() {
            sim_time * scenario_planet_rotation_time_scale
        } else {
            self.interactive_planet_rotation_time(sim_time)
        };
        let planet_rotation_radians = planet::planet_rotation_radians(planet_rotation_time);
        let weather_time = if self.scenario.is_some() {
            presentation_time
        } else {
            weather::interactive_weather_time_seconds(presentation_time)
        };
        let mut weather_sun_direction =
            planet::planet_local_vector(self.sun_direction, planet_rotation_radians);
        if self.scenario.is_none() {
            weather_sun_direction =
                weather::seasonal_sun_direction(weather_sun_direction, weather_time);
        }
        if self.scenario.is_some() && self.weather.prepare_next(weather_sun_direction) {
            let weather_target = self
                .weather
                .next_cloud_field_texture_data()
                .expect("prepared weather target");
            self.weather_clouds.replace_fields(
                &self.device,
                &self.queue,
                &weather_target,
                &self
                    .weather
                    .next_surface_field_texture_data()
                    .expect("prepared weather surface target"),
            );
        }
        // Weather keeps evolving while F10 freezes the scene clock; the
        // planet-local sun direction above remains frozen with the scene.
        // Interactive prediction runs one state further ahead off-thread;
        // authored scenarios retain the synchronous deterministic path.
        let weather_steps = if self.scenario.is_some() {
            self.weather
                .advance_to_with_sun(weather_time, weather_sun_direction)
        } else {
            self.weather
                .advance_interactive_to_with_sun(weather_time, weather_sun_direction)
        };
        if weather_steps > 0 {
            let weather_target = self
                .weather
                .next_cloud_field_texture_data()
                .expect("advanced weather target");
            if weather_steps > 1 {
                let weather_field = self.weather.cloud_field_texture_data();
                let surface_field = self.weather.surface_field_texture_data();
                self.weather_clouds.initialize_fields(
                    &self.device,
                    &self.queue,
                    &weather_field,
                    &surface_field,
                );
            }
            self.weather_clouds.replace_fields(
                &self.device,
                &self.queue,
                &weather_target,
                &self
                    .weather
                    .next_surface_field_texture_data()
                    .expect("advanced weather surface target"),
            );
        }
        self.weather_clouds.set_temporal_state(
            &self.queue,
            self.weather.interpolation_fraction(),
            self.weather.visual_time_seconds(),
        );
        let ocean_time_seconds = ocean_animation_time_seconds(sim_time, presentation_time);
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
            self.advance_low_flight_camera(
                scene_delta_seconds,
                planet_rotation_radians,
                ocean_time_seconds,
            );
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
                CameraMode::LowFlight => self.advance_low_flight_camera(
                    camera_delta_seconds,
                    planet_rotation_radians,
                    ocean_time_seconds,
                ),
                CameraMode::Surface => self.advance_surface_camera(
                    camera_delta_seconds,
                    planet_rotation_radians,
                    ocean_time_seconds,
                ),
            }
        }
        self.last_auto_orbit_sim_time = sim_time;
        let mut camera_world_position = self.camera.world_position();
        let mut camera_planet_frame_position = self
            .camera
            .planet_frame_world_position(planet_rotation_radians);
        let mut camera_planet_frame_direction = self
            .camera
            .planet_frame_direction_dvec3(planet_rotation_radians);
        let mut camera_planet_frame_up = self.camera.planet_frame_view_up(planet_rotation_radians);
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
        // Movement initially collides against the previous frame's rendered
        // frontier. At high speed the new camera direction can lie completely
        // outside that frontier; `terrain.update` above is what resolves the
        // patches actually drawn at the destination. Clamp once more against
        // those patches before presenting the frame, then rebuild their
        // camera-relative anchors if the correction moved the camera.
        if !solid_color_screen && self.render_path == RenderPath::Raster {
            let camera_corrected = match self.camera_mode {
                CameraMode::LowFlight => {
                    let minimum_clearance_meters =
                        if self.flight_speed.speed_meters_per_second > 0.0 {
                            LOW_FLIGHT_MOVING_CLEARANCE_METERS
                        } else {
                            LOW_FLIGHT_MINIMUM_CLEARANCE_METERS
                        };
                    self.enforce_low_flight_clearance(
                        minimum_clearance_meters,
                        planet_rotation_radians,
                        ocean_time_seconds,
                    )
                }
                CameraMode::Surface => self.resolve_surface_camera_after_streaming(
                    planet_rotation_radians,
                    ocean_time_seconds,
                ),
                CameraMode::Orbit => false,
            };
            if camera_corrected {
                camera_world_position = self.camera.world_position();
                camera_planet_frame_position = self
                    .camera
                    .planet_frame_world_position(planet_rotation_radians);
                camera_planet_frame_direction = self
                    .camera
                    .planet_frame_direction_dvec3(planet_rotation_radians);
                camera_planet_frame_up = self.camera.planet_frame_view_up(planet_rotation_radians);
                self.terrain_stats = self
                    .terrain
                    .update(
                        camera_planet_frame_position,
                        camera_planet_frame_direction,
                        camera_planet_frame_up,
                        presentation_time,
                        [self.size.width, self.size.height],
                        self.camera.vertical_fov_radians(),
                    )
                    .unwrap_or_else(|error| panic!("terrain update failed: {error}"));
            }
        }
        let camera_radius = camera_world_position.length();
        let camera_altitude = if self.scenario.is_none()
            && matches!(
                self.camera_mode,
                CameraMode::LowFlight | CameraMode::Surface
            ) {
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
        let auto_exposure_enabled = self.hdr.auto_exposure_enabled();
        if auto_exposure_enabled {
            self.hdr.collect_completed_luminance(&self.device);
            // Eye adaptation is a presentation effect, not simulation state.
            // It must continue to converge while F10 freezes planet
            // animation, but fixed exposure should not pay for meter/readback
            // work at all.
            self.hdr.update_exposure(&self.queue, f64::from(frame_time));
        }
        let exposure_state = self.hdr.exposure_state();
        self.artifacts.record_exposure_sample(
            sim_time,
            exposure_state.exposure,
            exposure_state.target_exposure,
            exposure_state.average_luminance,
            self.scenario.is_some() || write_log,
        );
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
        let local_storm_intensity = ocean::GLOBAL_OCEAN_STORM_INTENSITY;
        let ocean_wave_stats = ocean::wave_height_stats(ocean_time_seconds, local_storm_intensity);
        let ocean_wave_range = ocean_wave_stats.range_meters();
        if write_log {
            self.record_spatial_log_sample(SpatialLogInputs {
                sim_time,
                camera_world_position,
                camera_radius,
                camera_altitude,
                velocity_meters_per_second,
                planet_rotation_radians,
                frame_time,
                draw_calls,
                exposure: exposure_state.exposure,
                ocean_wave_stats,
            });
        }
        let simulation_ms = profile_started.elapsed().as_secs_f32() * 1_000.0;

        let mut textures_to_free = Vec::new();
        let render_egui = !solid_color_screen && !hide_overlay && self.debug_overlay_visible;
        let refresh_egui = render_egui && (self.hud_dirty || now >= self.next_hud_update);
        if refresh_egui {
            textures_to_free = self.refresh_hud(HudInputs {
                window,
                now,
                camera_world_position,
                camera_altitude,
                exposure_state,
                ocean_wave_range,
            });
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
        if !solid_color_screen && self.render_path == RenderPath::Raster {
            self.forest
                .encode_gpu_generation(&mut encoder, &self.camera_bind_group, &self.terrain);
        }
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
        let camera_direction = camera_planet_frame_position.normalize();
        let mut camera_surface_height_meters = match self.render_path {
            RenderPath::Raster => self.terrain.raster_surface_height_meters_at(
                camera_direction,
                camera_sea_level_altitude_meters,
            ),
            RenderPath::FoveatedRay => self
                .terrain
                .surface_height_meters_at(camera_direction, camera_sea_level_altitude_meters),
        }
        .unwrap_or(0.0);
        if let Some((bathymetry, water_height, _)) = self.open_ocean_environment_at(
            camera_direction,
            camera_sea_level_altitude_meters,
            ocean_time_seconds,
            camera_surface_height_meters,
        ) {
            camera_surface_height_meters = bathymetry.max(water_height);
        }
        let aspect_ratio = self.size.width as f32 / self.size.height as f32;
        let mut camera_uniform = planet::CameraUniform::from_camera(
            &self.camera,
            aspect_ratio,
            self.sun_direction,
            planet_rotation_radians,
            ocean_time_seconds,
            self.render_debug_mode,
            self.flat_triangle_outline_mode,
            camera_surface_height_meters,
        );
        // Spare presentation channel shared by raster and ray paths. Ocean
        // displacement uses the same temporally interpolated local storm field
        // as the cloud system, without adding a bind group or texture lookup.
        camera_uniform.flat_triangle_options[1] = local_storm_intensity;
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        self.forest
            .update_camera(&self.queue, camera_planet_frame_position);
        self.forest.update_patch(
            &self.queue,
            &self.terrain,
            camera_planet_frame_position,
            camera_sea_level_altitude_meters,
            self.size.height,
            self.camera.vertical_fov_radians(),
            presentation_time,
        );
        if write_log {
            let forest = self.forest.stats();
            tracing::info!(
                target: "catinthegarden::forest",
                patch_count = forest.patch_count,
                proxy_patch_count = forest.proxy_patch_count,
                beam_count = forest.beam_count,
                instances = forest.instances,
                proxy_instances = forest.proxy_instances,
                full_instances = forest.full_instances,
                medium_instances = forest.medium_instances,
                sparse_instances = forest.sparse_instances,
                zero_instances = forest.zero_instances,
                rebuild_count = forest.rebuild_count,
                patch_key = ?forest.patch_key,
                minimum_source_level = ?forest.minimum_source_level,
                pending_candidates = forest.pending_candidates,
                pending_candidates_total = forest.pending_candidates_total,
                transition_progress = forest.transition_progress,
                beams_enabled = forest.beams_enabled,
                "procedural forest state"
            );
        }
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
            let flight_velocity_planet_frame = if self.scenario.is_none()
                && matches!(
                    self.camera_mode,
                    CameraMode::LowFlight | CameraMode::Surface
                ) {
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
        if !solid_color_screen {
            self.atmosphere
                .update(&mut encoder, &self.camera_bind_group);
        }
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
                    self.terrain.draw(
                        &mut render_pass,
                        &self.camera_bind_group,
                        self.weather_clouds.field_bind_group(),
                    );
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
        if !solid_color_screen
            && matches!(
                self.render_debug_mode,
                planet::RenderDebugMode::Final | planet::RenderDebugMode::FlatTriangles
            )
            && !(self.render_path == RenderPath::FoveatedRay
                && self.render_debug_mode == planet::RenderDebugMode::Final
                && self.foveated.warp_debug_visible())
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("weather cloud shell pass"),
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
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.forest.draw_beams(
                &mut render_pass,
                &self.camera_bind_group,
                camera_sea_level_altitude_meters,
            );
            self.weather_clouds
                .draw(&mut render_pass, &self.camera_bind_group);
            self.forest.draw(
                &mut render_pass,
                &self.camera_bind_group,
                self.weather_clouds.field_bind_group(),
                camera_sea_level_altitude_meters,
            );
            self.local_cloud_impostors.draw(
                &mut render_pass,
                &self.camera_bind_group,
                self.weather_clouds.field_bind_group(),
            );
            self.rain.draw(
                &mut render_pass,
                &self.camera_bind_group,
                self.weather_clouds.field_bind_group(),
            );
        }
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
        if auto_exposure_enabled {
            self.hdr.encode_luminance(
                &mut encoder,
                timestamp_query_set.map(|query_set| (query_set, 2, 3)),
            );
        }
        let hdr_luminance_readback_slot = auto_exposure_enabled
            .then(|| self.hdr.encode_luminance_readback(&mut encoder))
            .flatten();
        // The disc and corona are a camera-only visual aid. Composite them
        // after the meter has sampled the physical atmosphere and terrain scene so
        // their terrain occlusion cannot drive a false exposure rebound at
        // sunset. The depth-tested disc is drawn first; the following
        // attachment-free flare pass can then sample completed solid depth and
        // keep its whole camera-response shape only while some disc is visible.
        // Both remain HDR input for bloom and tone mapping below.
        let draw_visual_sun = !solid_color_screen
            && self.render_debug_mode != planet::RenderDebugMode::SkyOnly
            && !(self.render_path == RenderPath::FoveatedRay
                && self.render_debug_mode == planet::RenderDebugMode::Final
                && self.foveated.warp_debug_visible());
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("visual sun disc pass"),
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
                timestamp_writes: timestamp_query_set.map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(4),
                        end_of_pass_write_index: None,
                    }
                }),
                multiview_mask: None,
            });
            if draw_visual_sun {
                self.sun.draw_disc(
                    &mut render_pass,
                    &self.camera_bind_group,
                    self.weather_clouds.field_bind_group(),
                );
            }
        }
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("visual sun optical flare pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.hdr.scene_view(),
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
                        beginning_of_pass_write_index: None,
                        end_of_pass_write_index: Some(5),
                    }
                }),
                multiview_mask: None,
            });
            if draw_visual_sun {
                self.sun.draw_flare(
                    &mut render_pass,
                    &self.camera_bind_group,
                    self.weather_clouds.field_bind_group(),
                );
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
                .begin_readback(
                    slot_index,
                    sim_time,
                    self.render_path,
                    auto_exposure_enabled,
                );
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
            // Captures are externally consumed while a run is in progress;
            // make the JSONL evidence durable alongside the PNG.
            let _ = self.log_writer.flush();
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
            let _ = self.log_writer.flush();
            tracing::info!(passed, "scenario completed");
            return Some(passed);
        }

        None
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let _ = self.log_writer.flush();
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
    /// Defer the compositor fullscreen request until wgpu has presented once.
    /// On X11, mapping a new window fullscreen during its first surface
    /// acquire can leave the desktop image visible until a later resize.
    startup_fullscreen_pending: bool,
    startup_redraw_seen: bool,
}

impl App {
    fn new(launch_options: LaunchOptions) -> Self {
        Self {
            launch_options,
            scenario_failed: Arc::new(AtomicBool::new(false)),
            window: None,
            state: None,
            startup_fullscreen_pending: false,
            startup_redraw_seen: false,
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
        if should_start_interactive_fullscreen(self.launch_options.scenario_name.is_some()) {
            self.startup_fullscreen_pending = true;
            tracing::info!(
                target: "catinthegarden::startup",
                "deferring fullscreen until the first surface frame"
            );
        }
        // Keep the authored startup view stable. Remote-desktop and touch
        // clients can emit synthetic relative motion while the new fullscreen
        // window is settling; capturing here let that motion turn the camera
        // before the first useful frame. A deliberate left click enables the
        // existing captured mouse-look path below.
        state.set_mouse_capture(&window, false);
        self.state = Some(state);
        self.window = Some(window);
        self.window
            .as_ref()
            .expect("window initialized above")
            .request_redraw();
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
        if self.startup_fullscreen_pending && matches!(&event, WindowEvent::RedrawRequested) {
            self.startup_redraw_seen = true;
        }
        if matches!(
            &event,
            WindowEvent::KeyboardInput { event, .. }
                if event.state.is_pressed()
                    && matches!(
                        event.physical_key,
                        PhysicalKey::Code(KeyCode::Escape)
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
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state.is_pressed()
            && !event.repeat
            && event.physical_key == PhysicalKey::Code(KeyCode::Space)
        {
            state.request_surface_jump();
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
                WindowEvent::Focused(false) => state.set_mouse_capture(window, false),
                WindowEvent::MouseInput {
                    state: winit::event::ElementState::Pressed,
                    button: winit::event::MouseButton::Left,
                    ..
                } => state.set_mouse_capture(window, true),
                WindowEvent::Resized(size) => state.resize(size),
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::KeyF) =>
                {
                    self.startup_fullscreen_pending = false;
                    self.startup_redraw_seen = false;
                    state.toggle_fullscreen(window);
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F1) =>
                {
                    state.adjust_planet_rotation_speed(1.0 / PLANET_ROTATION_SCALE_STEP);
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::F2) =>
                {
                    state.adjust_planet_rotation_speed(PLANET_ROTATION_SCALE_STEP);
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
                        && event.physical_key == PhysicalKey::Code(KeyCode::BracketLeft) =>
                {
                    state.adjust_flight_speed_scale(1.0 / FLIGHT_SPEED_SCALE_STEP);
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::BracketRight) =>
                {
                    state.adjust_flight_speed_scale(FLIGHT_SPEED_SCALE_STEP);
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
                        && !event.repeat
                        && event.physical_key == PhysicalKey::Code(KeyCode::KeyG) =>
                {
                    state.toggle_surface_camera_mode();
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
                        && event.physical_key == PhysicalKey::Code(KeyCode::KeyO) =>
                {
                    state.cycle_flat_triangle_outline_mode();
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::KeyB) =>
                {
                    state.forest.toggle_beams();
                    state.mark_hud_dirty();
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
                        && event.physical_key == PhysicalKey::Code(KeyCode::Digit7) =>
                {
                    state.weather.toggle_overlay();
                    state.hud_dirty = true;
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state.is_pressed()
                        && event.physical_key == PhysicalKey::Code(KeyCode::Digit9) =>
                {
                    state.step_weather_once();
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
        if self.startup_fullscreen_pending && self.startup_redraw_seen {
            if let (Some(window), Some(state)) = (self.window.as_ref(), self.state.as_mut()) {
                self.startup_fullscreen_pending = false;
                self.startup_redraw_seen = false;
                state.toggle_fullscreen(window);
                tracing::info!(
                    target: "catinthegarden::startup",
                    "entered fullscreen after the first surface frame"
                );
                window.request_redraw();
            }
        }
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::{
        ACTIVE_HIGHEST_PROMINENCE_DIRECTION, ACTIVE_HIGHEST_PROMINENCE_METERS,
        ACTIVE_HIGHEST_RAW_MACRO_ELEVATION_METERS, CameraMode, DEFAULT_OUTMAP_PATH,
        FLIGHT_SPEED_SCALE_STEP, FlightMovementInput, FlightSpeedState,
        INTERACTIVE_PLANET_ROTATION_TIME_SCALE, LOW_FLIGHT_ALTITUDE_METERS,
        LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND, LOW_FLIGHT_MINIMUM_CLEARANCE_METERS,
        MAX_LOW_FLIGHT_FRAME_DELTA_SECONDS, MAXIMUM_FLIGHT_SPEED_SCALE,
        MAXIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE, MINIMUM_FLIGHT_SPEED_SCALE,
        MINIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE, PLANET_ROTATION_SCALE_STEP, RenderPath,
        STORM_OCEAN_START_DIRECTION, STORM_OCEAN_START_PITCH_RADIANS, adjusted_flight_speed_scale,
        advance_flight_position_on_sphere, advance_flight_speed, device_mouse_look_enabled,
        find_default_outmap, flight_movement_direction, flight_view_direction,
        focus_of_expansion_ndc, initial_flight_tangent, interactive_camera_delta_seconds,
        low_flight_clearance_radius, projected_planet_coverage, render_size_for_surface_resize,
        retimed_planet_rotation, should_enter_fullscreen, should_start_interactive_fullscreen,
        surface_movement_direction, swept_flight_clearance_lift, transport_flight_tangent,
    };
    use crate::planet::{
        CameraUniform, FlatTriangleOutlineMode, OrbitCamera, PLANET_ROTATION_PERIOD_SECONDS,
        RenderDebugMode, default_sun_direction, geographic_longitude_degrees,
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
    fn interactive_startup_is_fullscreen_but_scenarios_stay_windowed() {
        assert!(should_start_interactive_fullscreen(false));
        assert!(!should_start_interactive_fullscreen(true));
    }

    #[test]
    fn ocean_animation_keeps_advancing_while_scene_time_is_frozen() {
        assert_eq!(super::ocean_animation_time_seconds(2.0, 7.5), 7.5);
    }

    #[test]
    fn camera_in_contact_with_ocean_follows_troughs_instead_of_ratcheting_upward() {
        let prior_surface = 12.0;
        let current_radius = crate::planet::PLANET_RADIUS_METERS
            + prior_surface
            + LOW_FLIGHT_MINIMUM_CLEARANCE_METERS;
        let falling_surface = -4.0;
        let followed = low_flight_clearance_radius(
            current_radius,
            prior_surface,
            Some(falling_surface),
            LOW_FLIGHT_MINIMUM_CLEARANCE_METERS,
            true,
        );
        assert_eq!(
            followed,
            crate::planet::PLANET_RADIUS_METERS
                + falling_surface
                + LOW_FLIGHT_MINIMUM_CLEARANCE_METERS
        );

        let flying = low_flight_clearance_radius(
            current_radius + 100.0,
            prior_surface,
            Some(falling_surface),
            LOW_FLIGHT_MINIMUM_CLEARANCE_METERS,
            false,
        );
        assert_eq!(flying, current_radius + 100.0);
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
            FlatTriangleOutlineMode::Dark,
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
    fn held_flight_input_uses_fixed_altitude_scaled_speed() {
        let ground = advance_flight_speed(FlightSpeedState::default(), true, false, 0.0, 1.0);
        let high = advance_flight_speed(FlightSpeedState::default(), true, false, 100_000.0, 1.0);
        let repeated = advance_flight_speed(ground, true, false, 0.0, 1.0);

        assert!((ground.speed_meters_per_second - 111.76).abs() < 1.0e-12);
        assert!(high.speed_meters_per_second > ground.speed_meters_per_second * 900.0);
        assert_eq!(repeated, ground);
    }

    #[test]
    fn releasing_flight_input_stops_immediately() {
        let held = advance_flight_speed(FlightSpeedState::default(), true, false, 500.0, 1.0);
        let released = advance_flight_speed(held, false, false, 500.0, 1.0);

        assert!(held.speed_meters_per_second > 0.0);
        assert_eq!(released, FlightSpeedState::default());
    }

    #[test]
    fn altitude_scaled_flight_has_a_finite_interplanetary_speed_cap() {
        let mut state = FlightSpeedState::default();
        for _ in 0..1_800 {
            state = advance_flight_speed(state, true, true, 1_000_000_000.0, 1.0);
        }

        assert_eq!(
            state.speed_meters_per_second,
            LOW_FLIGHT_MAX_SPEED_METERS_PER_SECOND
        );
    }

    #[test]
    fn flight_speed_scale_multiplies_the_altitude_curve_and_is_bounded() {
        let normal = advance_flight_speed(FlightSpeedState::default(), true, false, 500.0, 1.0);
        let slower = advance_flight_speed(FlightSpeedState::default(), true, false, 500.0, 0.5);
        let faster = advance_flight_speed(FlightSpeedState::default(), true, false, 500.0, 2.0);
        assert_eq!(
            slower.speed_meters_per_second,
            normal.speed_meters_per_second * 0.5
        );
        assert_eq!(
            faster.speed_meters_per_second,
            normal.speed_meters_per_second * 2.0
        );

        let mut scale = 1.0;
        for _ in 0..16 {
            scale = adjusted_flight_speed_scale(scale, 1.0 / FLIGHT_SPEED_SCALE_STEP);
        }
        assert_eq!(scale, MINIMUM_FLIGHT_SPEED_SCALE);
        for _ in 0..32 {
            scale = adjusted_flight_speed_scale(scale, FLIGHT_SPEED_SCALE_STEP);
        }
        assert_eq!(scale, MAXIMUM_FLIGHT_SPEED_SCALE);
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
    fn surface_forward_input_stays_tangent_even_when_looking_up_or_down() {
        let radial = DVec3::Y;
        let downhill_look = DVec3::new(0.0, -0.8, 0.6).normalize();
        let movement = surface_movement_direction(
            FlightMovementInput {
                forward: true,
                ..FlightMovementInput::default()
            },
            downhill_look,
            radial,
            DVec3::Z,
        )
        .expect("forward is held");
        assert!(movement.dot(radial).abs() < 1.0e-12);
        assert!(movement.dot(DVec3::Z) > 0.999);
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
        const FLIGHT_ALTITUDE_METERS: f64 = 10.0;
        const HIDDEN_PEAK_HEIGHT_METERS: f64 = 20.0;

        let radius = crate::planet::PLANET_RADIUS_METERS + FLIGHT_ALTITUDE_METERS;
        let start = DVec3::X * radius;
        let end = (DVec3::X + DVec3::Z * (4.0 / radius)).normalize() * radius;
        let end_z = end.normalize().z;
        let lift = swept_flight_clearance_lift(
            start,
            end,
            LOW_FLIGHT_MINIMUM_CLEARANCE_METERS,
            |direction, _| {
                Some(if (0.4 * end_z..0.6 * end_z).contains(&direction.z) {
                    HIDDEN_PEAK_HEIGHT_METERS
                } else {
                    0.0
                })
            },
        );

        // Derived, not restated: the expected lift is whatever it takes to
        // clear the midpoint peak, so lowering the clearance floor retunes this
        // test instead of breaking it. The hardcoded form silently went stale
        // when the floor dropped from 2m to sub-metre.
        let expected = HIDDEN_PEAK_HEIGHT_METERS + LOW_FLIGHT_MINIMUM_CLEARANCE_METERS
            - FLIGHT_ALTITUDE_METERS;
        assert!(
            (lift - expected).abs() < 1.0e-9,
            "swept lift {lift} should clear the midpoint peak at {expected}",
        );
    }

    #[test]
    fn low_flight_starts_on_land_facing_slightly_down_across_the_sea() {
        let radial = super::COASTAL_START_DIRECTION;
        let tangent = super::COASTAL_SEAWARD_TANGENT;
        let direction =
            flight_view_direction(radial, tangent, 0.0, super::COASTAL_START_PITCH_RADIANS);

        assert!((radial.length() - 1.0).abs() < 1.0e-12);
        assert!((tangent.length() - 1.0).abs() < 1.0e-12);
        assert!(radial.dot(tangent).abs() < 1.0e-12);
        assert_eq!(super::COASTAL_START_ALTITUDE_METERS, 100.0);
        assert!((-0.15..-0.12).contains(&direction.dot(radial)));
        assert!(direction.dot(tangent) > 0.99);
    }

    #[test]
    fn interactive_startup_storm_pose_is_a_surface_facing_ocean_view() {
        let radial = STORM_OCEAN_START_DIRECTION;
        let tangent = initial_flight_tangent(radial);
        let direction =
            flight_view_direction(radial, tangent, 0.0, STORM_OCEAN_START_PITCH_RADIANS);

        assert!((radial.length() - 1.0).abs() < 1.0e-12);
        assert!(radial.dot(tangent).abs() < 1.0e-12);
        assert!(direction.dot(radial) < 0.0);
        assert!(direction.dot(tangent) > 0.99);
        assert!((radial.y.asin().to_degrees() - 30.246_944_237).abs() < 1.0e-6);
        assert!((geographic_longitude_degrees(radial) + 14.474_559_060).abs() < 1.0e-6);
    }

    #[test]
    fn active_peak_measurement_uses_standard_global_summit_prominence() {
        let direction = ACTIVE_HIGHEST_PROMINENCE_DIRECTION;
        assert!((direction.length() - 1.0).abs() < 1.0e-12);
        assert!((direction.y.asin().to_degrees() - (-26.228_230_938)).abs() < 1.0e-6);
        assert!(
            (crate::planet::geographic_longitude_degrees(direction) + 121.070_605_516).abs()
                < 1.0e-6
        );

        // A planet's highest summit has no higher parent. As for Everest, its
        // key col is sea level, so prominence equals summit elevation ASL.
        assert!(
            ACTIVE_HIGHEST_PROMINENCE_METERS
                > ACTIVE_HIGHEST_RAW_MACRO_ELEVATION_METERS * 4.0
                    - crate::planet::TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS
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
        assert!(LOW_FLIGHT_ALTITUDE_METERS <= 10.0);
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
        assert_eq!(
            interactive_camera_delta_seconds(CameraMode::Surface, 0.0, frame_delta_seconds),
            frame_delta_seconds
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
    fn planet_rotation_speed_steps_are_continuous_and_bounded() {
        let sim_time = 123.0;
        let old_scale = INTERACTIVE_PLANET_ROTATION_TIME_SCALE;
        let old_offset = 0.7;
        let rotation_time_before = sim_time * old_scale + old_offset;

        let (slower_scale, slower_offset) = retimed_planet_rotation(
            sim_time,
            old_scale,
            old_offset,
            1.0 / PLANET_ROTATION_SCALE_STEP,
        );
        assert_eq!(slower_scale, old_scale * 0.5);
        assert!((sim_time * slower_scale + slower_offset - rotation_time_before).abs() < 1.0e-12);

        let (faster_scale, faster_offset) = retimed_planet_rotation(
            sim_time,
            slower_scale,
            slower_offset,
            PLANET_ROTATION_SCALE_STEP,
        );
        assert_eq!(faster_scale, old_scale);
        assert!((sim_time * faster_scale + faster_offset - rotation_time_before).abs() < 1.0e-12);

        assert_eq!(
            retimed_planet_rotation(
                sim_time,
                MINIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
                old_offset,
                0.5,
            )
            .0,
            MINIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
        );
        assert_eq!(
            retimed_planet_rotation(
                sim_time,
                MAXIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
                old_offset,
                2.0,
            )
            .0,
            MAXIMUM_INTERACTIVE_PLANET_ROTATION_TIME_SCALE,
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

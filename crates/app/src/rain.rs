//! Rain around the camera in a storm.
//!
//! Drops fill a box of air around the eye, carried by the wind and falling at
//! the terminal speeds of four drop sizes. The box wraps around the eye, so it
//! is always in the middle of rain that stays put in the air as the eye moves
//! through it. Each drop is drawn as the streak it makes in one exposure,
//! along its velocity relative to the eye: the wind slants it, gusts slant it
//! further, and the camera's own motion (a pitching bridge) tilts it the way
//! it would for a real camera.
//!
//! How hard it rains follows the storm overcast (`intensity_for`), the same
//! eased storm strength that closes in the fog and greys the sea.

use glam::DVec3;

use crate::planet::CameraViewBasis;

/// Streaks at full rain. Mirrored into the shader as `RAIN_STREAKS`.
pub const STREAKS: u32 = 32768;
/// Streaks switch on in index order as the rain thickens, each over this much
/// of the intensity range; their places are random, so any count is an even
/// spread. Mirrored into the shader as `RAIN_ONSET_SPREAD`.
const ONSET_SPREAD: f32 = 0.05;
/// Side of the box of air carried around the eye (m). Streaks fade out to the
/// sphere inside it; past about ten metres a drop is thinner than a pixel and
/// the storm fog carries the rain instead.
pub const BOX_METERS: f64 = 24.0;
/// Terminal fall speeds (m/s) of the four drop sizes: 1.5-5mm drops fall at
/// 5.5-9 m/s.
pub const FALL_SPEEDS: [f64; 4] = [5.5, 6.8, 8.0, 9.0];
/// How long the eye sees each drop move for: the streak length.
pub const EXPOSURE_SECONDS: f64 = 1.0 / 40.0;
/// Storm overcast over which the rain sets in, light to full.
const OVERCAST_ONSET: f32 = 0.3;
const OVERCAST_FULL: f32 = 0.8;
/// How much heavier the rain comes in a full gust (and lighter in a lull): a
/// squall.
const GUST_BURST: f32 = 0.3;
/// Rain falls below the storm's cloud; above this there is none.
const CEILING_METERS: f64 = 20_000.0;
/// The eye's velocity is smoothed over this long, so one uneven frame does not
/// swing every streak.
const CAMERA_VELOCITY_SECONDS: f64 = 0.15;
/// Frames longer than this (pauses, hitches) are clamped so the rain does not
/// jump.
const MAX_STEP_SECONDS: f64 = 0.1;
/// Faster than this relative to the air (fast flight), streaks stay this long
/// rather than becoming lines across the screen.
const MAX_RELATIVE_AIR_SPEED: f64 = 40.0;
/// A camera jump further than this is a teleport, not a velocity.
const TELEPORT_METERS: f64 = 100.0;

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How hard it rains, 0-1, under this storm overcast and with this gust at the
/// camera (`gust::Gust::value`).
pub fn intensity_for(storm_overcast: f32, gust: f32) -> f32 {
    (smoothstep(OVERCAST_ONSET, OVERCAST_FULL, storm_overcast) * (1.0 + GUST_BURST * gust))
        .clamp(0.0, 1.0)
}

/// Whether rain can reach an eye at this altitude with the water surface at
/// `water_altitude_meters`: none above the cloud, none under the sea.
pub fn reaches_the_eye(altitude_meters: f64, water_altitude_meters: f64) -> bool {
    altitude_meters < CEILING_METERS && altitude_meters > water_altitude_meters
}

pub fn shader_source() -> String {
    format!(
        "{}\nconst RAIN_STREAKS: f32 = {STREAKS}.0;\nconst RAIN_ONSET_SPREAD: f32 = {ONSET_SPREAD:.3};\n{}",
        crate::planet::shared_planet_shader_source(),
        include_str!("rain.wgsl")
    )
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RainUniform {
    east: [f32; 4],
    north: [f32; 4],
    up: [f32; 4],
    scroll: [f32; 4],
    fall_scroll: [f32; 4],
    fall_speed: [f32; 4],
    drift: [f32; 4],
}

/// What the rain needs to know about this frame.
pub struct RainFrame {
    /// Planet-frame camera position (m).
    pub camera_position: DVec3,
    /// Planet-frame camera axes.
    pub basis: CameraViewBasis,
    pub time_seconds: f64,
    /// 0-1 (`intensity_for`), already zero where rain cannot reach the eye.
    pub intensity: f32,
    /// Planet-frame wind (m/s), gusts included.
    pub wind: DVec3,
    pub viewport: [u32; 2],
    pub vertical_fov_radians: f64,
}

/// Where the air box is, carried from frame to frame.
#[derive(Clone, Debug, Default)]
struct Drift {
    /// East and north offset of the box (m, 0 to `BOX_METERS`).
    scroll: [f64; 2],
    /// Vertical offset of each drop size.
    fall: [f64; 4],
    previous: Option<(DVec3, f64)>,
    camera_velocity: DVec3,
}

/// Local east, north and up at `position`, east taken from the FFT sea's own
/// plane so the frame turns smoothly (and is defined at the poles).
fn local_axes(position: DVec3) -> (DVec3, DVec3, DVec3) {
    let up = position.normalize();
    let (u, _) = crate::ocean_fft::anchor_axes(up.to_array());
    let u = DVec3::from_array(u);
    let east = (u - up * u.dot(up)).normalize();
    (east, up.cross(east), up)
}

impl Drift {
    /// Advances the box: the wind carries it, the drops fall through it, and
    /// the camera's own movement is taken out so the rain stays in the air.
    fn advance(&mut self, position: DVec3, time: f64, wind: DVec3) {
        let (east, north, up) = local_axes(position);
        let (moved, dt) = match self.previous {
            Some((previous, previous_time)) => {
                let moved = position - previous;
                let dt = (time - previous_time).clamp(0.0, MAX_STEP_SECONDS);
                if moved.length() > TELEPORT_METERS {
                    (DVec3::ZERO, dt)
                } else {
                    if dt > 1.0e-4 {
                        let weight = 1.0 - (-dt / CAMERA_VELOCITY_SECONDS).exp();
                        self.camera_velocity += (moved / dt - self.camera_velocity) * weight;
                    }
                    (moved, dt)
                }
            }
            None => (DVec3::ZERO, 0.0),
        };
        self.previous = Some((position, time));
        self.scroll[0] =
            (self.scroll[0] + wind.dot(east) * dt - moved.dot(east)).rem_euclid(BOX_METERS);
        self.scroll[1] =
            (self.scroll[1] + wind.dot(north) * dt - moved.dot(north)).rem_euclid(BOX_METERS);
        for (fall, speed) in self.fall.iter_mut().zip(FALL_SPEEDS) {
            *fall = (*fall - speed * dt - moved.dot(up)).rem_euclid(BOX_METERS);
        }
    }

    /// The air's velocity relative to the eye (m/s), capped.
    fn relative_air_velocity(&self, wind: DVec3) -> DVec3 {
        let relative = wind - self.camera_velocity;
        let speed = relative.length();
        if speed > MAX_RELATIVE_AIR_SPEED {
            relative * (MAX_RELATIVE_AIR_SPEED / speed)
        } else {
            relative
        }
    }
}

pub struct Rain {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    drift: Drift,
    intensity: f32,
}

impl Rain {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        shared_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rain uniform"),
            size: size_of::<RainUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rain bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rain bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rain pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&bind_group_layout),
                Some(shared_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rain shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rain pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_rain"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_rain"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // Behind the hull, the sea and the land; never hiding them.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniform_buffer,
            bind_group,
            drift: Drift::default(),
            intensity: 0.0,
        }
    }

    /// How hard it is raining at the camera this frame, 0-1.
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// The camera's own velocity (planet frame, m/s), smoothed: what the air
    /// rushes past it with, besides the wind.
    pub fn camera_velocity(&self) -> DVec3 {
        self.drift.camera_velocity
    }

    /// Call once per frame, after the camera is settled: the box is uploaded
    /// in view axes.
    pub fn update(&mut self, queue: &wgpu::Queue, frame: RainFrame) {
        self.drift
            .advance(frame.camera_position, frame.time_seconds, frame.wind);
        self.intensity = frame.intensity.clamp(0.0, 1.0);
        if self.intensity <= 0.0 {
            return;
        }
        let (east, north, up) = local_axes(frame.camera_position);
        let view = |v: DVec3| frame.basis.world_to_view(v).as_vec3();
        let [east, north, up] = [east, north, up].map(view);
        let drift = view(self.drift.relative_air_velocity(frame.wind));
        let height = f64::from(frame.viewport[1].max(1));
        let uniform = RainUniform {
            east: east.extend(BOX_METERS as f32).to_array(),
            north: north.extend(self.intensity).to_array(),
            up: up.extend(EXPOSURE_SECONDS as f32).to_array(),
            scroll: [
                self.drift.scroll[0] as f32,
                self.drift.scroll[1] as f32,
                frame.viewport[0].max(1) as f32,
                height as f32,
            ],
            fall_scroll: self.drift.fall.map(|fall| fall as f32),
            fall_speed: FALL_SPEEDS.map(|speed| speed as f32),
            drift: drift
                .extend((0.5 * height / (0.5 * frame.vertical_fov_radians).tan()) as f32)
                .to_array(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Streaks to draw: every one the shader might switch on.
    fn streak_count(&self) -> u32 {
        let share = (self.intensity + ONSET_SPREAD).min(1.0);
        ((f64::from(STREAKS) * f64::from(share)).ceil() as u32).min(STREAKS)
    }

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
        shared_bind_group: &wgpu::BindGroup,
    ) {
        if self.intensity <= 0.0 {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_bind_group(2, shared_bind_group, &[]);
        render_pass.draw(0..self.streak_count() * 6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_sets_in_with_the_storm_and_comes_in_bursts_with_the_gusts() {
        assert_eq!(intensity_for(0.0, 0.0), 0.0);
        assert_eq!(intensity_for(OVERCAST_ONSET, 1.0), 0.0);
        assert_eq!(intensity_for(1.0, 0.0), 1.0);
        let steady = intensity_for(0.55, 0.0);
        assert!(steady > 0.2 && steady < 0.8, "{steady}");
        assert!(intensity_for(0.55, 1.0) > steady);
        assert!(intensity_for(0.55, -1.0) < steady);
        assert!(intensity_for(1.0, -1.0) < 1.0);
    }

    #[test]
    fn no_rain_above_the_cloud_or_under_the_sea() {
        assert!(reaches_the_eye(2.5, 0.4));
        assert!(!reaches_the_eye(-1.0, 0.4));
        assert!(!reaches_the_eye(CEILING_METERS + 1.0, 0.0));
    }

    #[test]
    fn the_rain_stays_in_the_air_as_the_camera_moves_through_it() {
        // In still air, a camera moving 3m east sees every drop 3m further
        // west: the box scrolls back by exactly what the camera moved.
        let position = DVec3::new(0.0, 0.0, crate::planet::planet_radius_meters() + 2.0);
        let (east, _, _) = local_axes(position);
        let mut drift = Drift::default();
        drift.advance(position, 10.0, DVec3::ZERO);
        let before = drift.scroll;
        drift.advance(position + east * 3.0, 10.0, DVec3::ZERO);
        let moved = (before[0] - drift.scroll[0]).rem_euclid(BOX_METERS);
        assert!((moved - 3.0).abs() < 1.0e-6, "{moved}");
        assert!((drift.scroll[1] - before[1]).abs() < 1.0e-6);
    }

    #[test]
    fn drops_fall_at_their_own_speeds_and_ride_the_wind() {
        let position = DVec3::new(0.0, 0.0, crate::planet::planet_radius_meters() + 2.0);
        let (east, north, _) = local_axes(position);
        let wind = east * 12.0 + north * 5.0;
        let mut drift = Drift::default();
        drift.advance(position, 0.0, wind);
        let (scroll, fall) = (drift.scroll, drift.fall);
        drift.advance(position, 0.05, wind);
        let step = |a: f64, b: f64| (b - a).rem_euclid(BOX_METERS);
        assert!((step(scroll[0], drift.scroll[0]) - 0.6).abs() < 1.0e-9);
        assert!((step(scroll[1], drift.scroll[1]) - 0.25).abs() < 1.0e-9);
        for (index, speed) in FALL_SPEEDS.iter().enumerate() {
            let dropped = step(drift.fall[index], fall[index]);
            assert!((dropped - speed * 0.05).abs() < 1.0e-9, "{dropped}");
        }
    }

    #[test]
    fn a_moving_camera_tilts_the_streaks_but_a_teleport_does_not() {
        let position = DVec3::new(0.0, 0.0, crate::planet::planet_radius_meters() + 2.0);
        let (_, _, up) = local_axes(position);
        let mut drift = Drift::default();
        let mut time = 0.0;
        for step in 0..60 {
            drift.advance(position + up * (0.1 * f64::from(step)), time, DVec3::ZERO);
            time += 1.0 / 60.0;
        }
        // Rising at 6 m/s: the air (and every drop) comes down at the camera
        // 6 m/s faster.
        let relative = drift.relative_air_velocity(DVec3::ZERO);
        assert!((relative.dot(up) + 6.0).abs() < 0.1, "{relative}");
        drift.advance(position + up * 5_000.0, time, DVec3::ZERO);
        assert!((drift.relative_air_velocity(DVec3::ZERO).dot(up) + 6.0).abs() < 0.1);
        // Fast flight: streaks are capped rather than spanning the screen.
        let gale = drift.relative_air_velocity(up * 500.0);
        assert!((gale.length() - MAX_RELATIVE_AIR_SPEED).abs() < 1.0e-9);
    }

    #[test]
    fn the_rain_shader_parses_and_validates() {
        let shader = shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&shader).expect("rain shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("rain shader must validate");
        assert_eq!(std::mem::size_of::<RainUniform>(), 7 * 16);
    }

    #[test]
    fn rain_lighting_is_luminance_matched_greyscale() {
        let shader = include_str!("rain.wgsl");
        assert!(shader.contains(
            "let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));"
        ));
        assert!(shader.contains("return vec3<f32>(luminance);"));
    }
}

//! Tiny bubbles suspended in the water around an eye under the sea.
//!
//! Under water there is nothing near the eye to show that it is moving: the
//! water is one colour in every direction. A box of tiny bubbles is carried
//! around the eye, as `rain` carries its drops: they ride the water (its
//! orbital velocity at the eye, the same current that carries the swimmer),
//! rise slowly at the speeds of four bubble sizes, and wrap so the box never
//! runs out. They stay put in the water, so they hold still while the eye
//! drifts with it and stream past when it swims, sinks or is driven through
//! it. Each is drawn as the short streak it makes in one exposure, relative
//! to the eye. They fade in over the first half metre under the surface.

use glam::DVec3;

use crate::planet::CameraViewBasis;

/// Bubbles in the box. Mirrored into the shader as `BUBBLE_COUNT`.
pub const COUNT: u32 = 6144;
/// Side of the box of water carried around the eye (m). Bubbles fade out to
/// the sphere inside it; past about three metres a millimetre bubble is well
/// under a pixel.
pub const BOX_METERS: f64 = 6.0;
/// Rise speeds (m/s) of the four bubble sizes: sub-millimetre bubbles rise
/// at a few centimetres a second, millimetre ones at 10-20.
pub const RISE_SPEEDS: [f64; 4] = [0.03, 0.06, 0.10, 0.16];
/// How long the eye sees each bubble move for: the streak length. Short, so
/// they read as specks with a hint of motion rather than as rain.
pub const EXPOSURE_SECONDS: f64 = 1.0 / 150.0;
/// Depth under the drawn surface over which the bubbles fade in.
const FADE_IN_START_METERS: f64 = 0.1;
const FADE_IN_FULL_METERS: f64 = 0.6;
/// The eye's velocity is smoothed over this long, so one uneven frame does not
/// swing every streak.
const CAMERA_VELOCITY_SECONDS: f64 = 0.15;
/// Frames longer than this (pauses, hitches) are clamped so the box does not
/// jump.
const MAX_STEP_SECONDS: f64 = 0.1;
/// Faster than this through the water, streaks stay this long rather than
/// becoming lines across the screen.
const MAX_RELATIVE_WATER_SPEED: f64 = 20.0;
/// A camera jump further than this is a teleport, not a velocity.
const TELEPORT_METERS: f64 = 100.0;

/// How much of the bubble field shows with the eye `depth_meters` under the
/// drawn surface (negative above it), 0-1.
pub fn visibility_at_depth(depth_meters: f64) -> f32 {
    let t = ((depth_meters - FADE_IN_START_METERS) / (FADE_IN_FULL_METERS - FADE_IN_START_METERS))
        .clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

pub fn shader_source() -> String {
    format!(
        "{}\nconst BUBBLE_COUNT: f32 = {COUNT}.0;\n{}",
        crate::planet::shared_planet_shader_source(),
        include_str!("bubbles.wgsl")
    )
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BubbleUniform {
    east: [f32; 4],
    north: [f32; 4],
    up: [f32; 4],
    scroll: [f32; 4],
    rise_scroll: [f32; 4],
    rise_speed: [f32; 4],
    drift: [f32; 4],
}

/// What the bubbles need to know about this frame.
pub struct BubbleFrame {
    /// Planet-frame camera position (m).
    pub camera_position: DVec3,
    /// Planet-frame camera axes.
    pub basis: CameraViewBasis,
    pub time_seconds: f64,
    /// How far under the drawn sea surface the eye is (m, negative above it).
    pub depth_meters: f64,
    /// Planet-frame velocity of the water at the eye (m/s), heave included.
    pub water_velocity: DVec3,
    pub viewport: [u32; 2],
    pub vertical_fov_radians: f64,
}

/// Where the box of water is, carried from frame to frame.
#[derive(Clone, Debug, Default)]
struct Drift {
    /// East and north offset of the box (m, 0 to `BOX_METERS`).
    scroll: [f64; 2],
    /// Vertical offset of each bubble size.
    rise: [f64; 4],
    previous: Option<(DVec3, f64)>,
    camera_velocity: DVec3,
}

/// Local east, north and up at `position`, as `rain` takes them.
fn local_axes(position: DVec3) -> (DVec3, DVec3, DVec3) {
    let up = position.normalize();
    let (u, _) = crate::ocean_fft::anchor_axes(up.to_array());
    let u = DVec3::from_array(u);
    let east = (u - up * u.dot(up)).normalize();
    (east, up.cross(east), up)
}

impl Drift {
    /// Advances the box: the water carries it, the bubbles rise through it,
    /// and the camera's own movement is taken out so they stay in the water.
    fn advance(&mut self, position: DVec3, time: f64, water: DVec3) {
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
        let carried = water * dt - moved;
        self.scroll[0] = (self.scroll[0] + carried.dot(east)).rem_euclid(BOX_METERS);
        self.scroll[1] = (self.scroll[1] + carried.dot(north)).rem_euclid(BOX_METERS);
        for (rise, speed) in self.rise.iter_mut().zip(RISE_SPEEDS) {
            *rise = (*rise + speed * dt + carried.dot(up)).rem_euclid(BOX_METERS);
        }
    }

    /// The water's velocity relative to the eye (m/s), capped.
    fn relative_water_velocity(&self, water: DVec3) -> DVec3 {
        let relative = water - self.camera_velocity;
        let speed = relative.length();
        if speed > MAX_RELATIVE_WATER_SPEED {
            relative * (MAX_RELATIVE_WATER_SPEED / speed)
        } else {
            relative
        }
    }
}

pub struct Bubbles {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    drift: Drift,
    visibility: f32,
}

impl Bubbles {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        shared_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bubble uniform"),
            size: size_of::<BubbleUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bubble bind group layout"),
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
            label: Some("bubble bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bubble pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&bind_group_layout),
                Some(shared_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bubble shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bubble pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_bubble"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bubble"),
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
            // Behind the hull and the seabed; never hiding them.
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
            visibility: 0.0,
        }
    }

    /// Call once per frame, after the camera is settled. The box is advanced
    /// even above water, so it is already moving with the water when the eye
    /// goes under.
    pub fn update(&mut self, queue: &wgpu::Queue, frame: BubbleFrame) {
        self.drift
            .advance(frame.camera_position, frame.time_seconds, frame.water_velocity);
        self.visibility = visibility_at_depth(frame.depth_meters);
        if self.visibility <= 0.0 {
            return;
        }
        let (east, north, up) = local_axes(frame.camera_position);
        let view = |v: DVec3| frame.basis.world_to_view(v).as_vec3();
        let [east, north, up] = [east, north, up].map(view);
        let drift = view(self.drift.relative_water_velocity(frame.water_velocity));
        let height = f64::from(frame.viewport[1].max(1));
        let uniform = BubbleUniform {
            east: east.extend(BOX_METERS as f32).to_array(),
            north: north.extend(self.visibility).to_array(),
            up: up.extend(EXPOSURE_SECONDS as f32).to_array(),
            scroll: [
                self.drift.scroll[0] as f32,
                self.drift.scroll[1] as f32,
                frame.viewport[0].max(1) as f32,
                height as f32,
            ],
            rise_scroll: self.drift.rise.map(|rise| rise as f32),
            rise_speed: RISE_SPEEDS.map(|speed| speed as f32),
            drift: drift
                .extend((0.5 * height / (0.5 * frame.vertical_fov_radians).tan()) as f32)
                .to_array(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
        shared_bind_group: &wgpu::BindGroup,
    ) {
        if self.visibility <= 0.0 {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_bind_group(2, shared_bind_group, &[]);
        render_pass.draw(0..COUNT * 6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubbles_show_only_under_the_water() {
        assert_eq!(visibility_at_depth(-2.0), 0.0);
        assert_eq!(visibility_at_depth(0.0), 0.0);
        assert!(visibility_at_depth(0.35) > 0.0 && visibility_at_depth(0.35) < 1.0);
        assert_eq!(visibility_at_depth(5.0), 1.0);
    }

    /// The box stays put in the water: an eye drifting with the current sees
    /// the bubbles hold still (only rising); an eye swimming through still
    /// water sees them go by by exactly what it moved.
    #[test]
    fn bubbles_ride_the_water_and_stream_past_a_swimmer() {
        let position = DVec3::new(0.0, 0.0, crate::planet::planet_radius_meters() - 3.0);
        let (east, _, _) = local_axes(position);
        let current = east * 2.0;
        let mut drift = Drift::default();
        drift.advance(position, 10.0, current);
        let before = drift.scroll;
        // Carried 0.1 m by the current in 0.05 s: the box does not move
        // relative to the eye.
        drift.advance(position + current * 0.05, 10.05, current);
        assert!((drift.scroll[0] - before[0]).abs() < 1.0e-9, "{:?}", drift.scroll);
        // Swimming 0.3 m east through still water: the box goes 0.3 m west.
        let mut still = Drift::default();
        still.advance(position, 10.0, DVec3::ZERO);
        let before = still.scroll;
        still.advance(position + east * 0.3, 10.05, DVec3::ZERO);
        let moved = (before[0] - still.scroll[0]).rem_euclid(BOX_METERS);
        assert!((moved - 0.3).abs() < 1.0e-9, "{moved}");
    }

    #[test]
    fn bubbles_rise_at_their_own_speeds() {
        let position = DVec3::new(0.0, 0.0, crate::planet::planet_radius_meters() - 3.0);
        let mut drift = Drift::default();
        drift.advance(position, 0.0, DVec3::ZERO);
        let before = drift.rise;
        drift.advance(position, 0.05, DVec3::ZERO);
        for (index, speed) in RISE_SPEEDS.iter().enumerate() {
            let risen = (drift.rise[index] - before[index]).rem_euclid(BOX_METERS);
            assert!((risen - speed * 0.05).abs() < 1.0e-9);
        }
    }
}

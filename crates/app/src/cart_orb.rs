//! The cart's engine: a 12 cm glowing orb in the lower bulb of an
//! hourglass-shaped glass jar, inside the timber engine box at the back of
//! the cart bed (box and glass are drawn with the cart, traffic_render). It is
//! the magic motor made visible; the substance that drives it is not worked
//! out yet. The orb sloshes a little in its bulb as the cart speeds up, brakes
//! and turns, and glows with the motor. Rebuilt on the CPU each frame (a few
//! hundred vertices, camera-relative), drawn as one solid and one additive
//! batch.
use crate::{driving, traffic_render};
use glam::DVec3;

pub const RADIUS: f64 = traffic_render::ORB_RADIUS;
/// Furthest the orb strays from the bulb's centre: it must stay inside the
/// glass (traffic_render tests the jar against this).
pub const SLOSH: f64 = 0.035;
/// Spring holding the orb to the bulb's centre (1/s²), a little under
/// critical damping so it rocks and settles with one small overshoot.
const STIFFNESS: f64 = 60.0;
const DAMPING_RATIO: f64 = 0.6;
/// Glow with the motor off or the charge spent: a dim violet lamp, not a dark
/// ball (0.04 read as a black disc in daylight, as if the orb were broken).
const EMBER_GLOW: f64 = 0.15;
const SEGMENTS: usize = 16;
const RINGS: usize = 10;
const MAX_VERTICES: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    position: [f32; 3],
    /// Orb: surface normal. Halo: quad coordinate in x, y.
    normal: [f32; 3],
    /// x glow, w kind (0 orb, 1 halo).
    colour: [f32; 4],
}

/// Where the orb sits relative to the bulb's centre (ahead, right, up),
/// sprung so it lags back when the cart speeds up, runs forward under
/// braking, swings out on bends and bobs a little while it waits.
#[derive(Clone, Copy, Debug)]
pub struct Orb {
    offset: DVec3,
    velocity: DVec3,
    time: f64,
    last_speed: f64,
    /// 0 idle .. 1 driving (eased throttle while the motor runs).
    pull: f64,
    /// Emissive strength, eased: dim ember when the motor is off.
    glow: f64,
}
impl Default for Orb {
    fn default() -> Self {
        Self {
            offset: DVec3::ZERO,
            velocity: DVec3::ZERO,
            time: 0.0,
            last_speed: 0.0,
            pull: 0.0,
            glow: 0.0,
        }
    }
}
impl Orb {
    /// `steering` is the car's wheel position, -1 left .. 1 right.
    pub fn advance(
        &mut self,
        seconds: f64,
        throttle: f64,
        steering: f64,
        speed: f64,
        running: bool,
        charge: f64,
    ) {
        let seconds = seconds.clamp(0.0, 0.1);
        if seconds <= 0.0 {
            return;
        }
        let acceleration = (speed - self.last_speed) / seconds;
        self.last_speed = speed;
        let ease = |value: &mut f64, target: f64, rate: f64| {
            *value += (target - *value) * (1.0 - (-rate * seconds).exp());
        };
        ease(&mut self.pull, if running { throttle.max(0.0) } else { 0.0 }, 4.0);
        let glow = if running { (0.25 + 0.75 * charge) * (1.0 + 0.5 * self.pull) } else { EMBER_GLOW };
        ease(&mut self.glow, glow, 3.0);
        let tau = std::f64::consts::TAU;
        // Speeding up leaves it behind, braking throws it forward; on a bend
        // it rolls to the outside of the bulb.
        let target = DVec3::new(
            -(acceleration * 0.006).clamp(-0.025, 0.025),
            -steering.clamp(-1.0, 1.0) * 0.02 * (speed.abs() / 10.0).min(1.0),
            0.008 * (self.time * tau * 0.55).sin() + 0.004 * (self.time * tau * 1.3).sin(),
        );
        let damping = 2.0 * DAMPING_RATIO * STIFFNESS.sqrt();
        let steps = (seconds * 240.0).ceil().max(1.0);
        let step = seconds / steps;
        for _ in 0..steps as usize {
            self.velocity += (STIFFNESS * (target - self.offset) - damping * self.velocity) * step;
            self.offset += self.velocity * step;
            let reach = self.offset.length();
            if reach > SLOSH {
                // The glass stops it: no further out, and no speed outward.
                let out = self.offset / reach;
                self.offset = out * SLOSH;
                self.velocity -= out * self.velocity.dot(out).max(0.0);
            }
        }
        self.time += seconds;
    }

    /// Solid orb, then the additive halo, relative to `camera`.
    /// `eye` is the car's eye point and `forward`/`up` its body axes.
    pub fn geometry(
        &self,
        eye: DVec3,
        forward: DVec3,
        up: DVec3,
        camera: DVec3,
    ) -> (Vec<Vertex>, Vec<Vertex>) {
        let right = forward.cross(up).normalize();
        let ground = eye - up * driving::EYE_HEIGHT;
        let centre = ground
            + forward * (traffic_render::JAR_Y + self.offset.x)
            + right * self.offset.y
            + up * (traffic_render::ORB_Z + self.offset.z);
        let glow = self.glow as f32;
        let v = |p: DVec3, n: [f32; 3], kind: f32| Vertex {
            position: (p - camera).as_vec3().into(),
            normal: n,
            colour: [glow, 0.0, 0.0, kind],
        };
        let mut solid = Vec::with_capacity(SEGMENTS * RINGS * 6);
        let direction = |ring: usize, segment: usize| {
            let polar = std::f64::consts::PI * ring as f64 / RINGS as f64;
            let azimuth = std::f64::consts::TAU * segment as f64 / SEGMENTS as f64;
            forward * (polar.sin() * azimuth.cos())
                + right * (polar.sin() * azimuth.sin())
                + up * polar.cos()
        };
        for ring in 0..RINGS {
            for segment in 0..SEGMENTS {
                let quad = [
                    direction(ring, segment),
                    direction(ring + 1, segment),
                    direction(ring + 1, segment + 1),
                    direction(ring, segment + 1),
                ];
                for i in [0, 1, 2, 0, 2, 3] {
                    let n = quad[i];
                    solid.push(v(centre + n * RADIUS, n.as_vec3().into(), 0.0));
                }
            }
        }
        // Halo: a camera-facing square around the orb, faded in the shader;
        // about the size of the bulb, so the glass looks lit from inside.
        let mut additive = Vec::with_capacity(6);
        let view = (centre - camera).normalize_or_zero();
        let across = view.cross(up).try_normalize().unwrap_or(right);
        let over = across.cross(view);
        let size = RADIUS * 2.5;
        let corner = |x: f64, y: f64| {
            v(centre + (across * x + over * y) * size, [x as f32, y as f32, 0.0], 1.0)
        };
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            additive.push(corner(x, y));
        }
        (solid, additive)
    }
}

pub struct Renderer {
    solid: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    solid_count: u32,
    additive_count: u32,
}
impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cart orb"),
            bind_group_layouts: &[Some(camera)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("cart_orb.wgsl"));
        let pipeline = |label: &str, entry: &str, blend: wgpu::BlendState, write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: crate::pipeline_cache::get(),
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        Self {
            solid: pipeline("cart orb", "fs_orb", wgpu::BlendState::REPLACE, true),
            additive: pipeline("cart orb glow", "fs_glow", add, false),
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cart orb vertices"),
                size: (MAX_VERTICES * size_of::<Vertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            solid_count: 0,
            additive_count: 0,
        }
    }
    pub fn clear(&mut self) {
        self.solid_count = 0;
        self.additive_count = 0;
    }
    pub fn upload(&mut self, queue: &wgpu::Queue, solid: &[Vertex], additive: &[Vertex]) {
        let all: Vec<Vertex> = solid.iter().chain(additive).copied().take(MAX_VERTICES).collect();
        self.solid_count = solid.len().min(all.len()) as u32;
        self.additive_count = all.len() as u32 - self.solid_count;
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&all));
    }
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: &'a wgpu::BindGroup) {
        if self.solid_count == 0 {
            return;
        }
        pass.set_bind_group(0, camera, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_pipeline(&self.solid);
        pass.draw(0..self.solid_count, 0..1);
        pass.set_pipeline(&self.additive);
        pass.draw(self.solid_count..self.solid_count + self.additive_count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orb_sits_in_the_jar_at_the_back_of_the_bed_and_sloshes_inside_it() {
        let mut orb = Orb::default();
        // Hard acceleration then a right-hand bend: it rolls back, then out
        // to the left, but never through the glass.
        for i in 0..240 {
            let speed = (i as f64 * 0.2).min(12.0);
            orb.advance(1.0 / 60.0, 1.0, if i > 120 { 1.0 } else { 0.0 }, speed, true, 1.0);
            assert!(orb.offset.length() <= SLOSH + 1e-9, "{:?}", orb.offset);
            if i == 30 {
                assert!(orb.offset.x < -0.01, "{:?}", orb.offset);
            }
        }
        assert!(orb.offset.y < -0.01, "{:?}", orb.offset);
        let (eye, forward, up) = (DVec3::new(0.0, 0.0, 4.0e6 + 1.4), DVec3::X, DVec3::Z);
        let (solid, additive) = orb.geometry(eye, forward, up, eye);
        assert!(solid.len() + additive.len() <= MAX_VERTICES);
        // Every orb vertex is RADIUS from a centre at the back of the bed,
        // about 1.1 m above the ground (the physics eye is 1.4 m up).
        let centre = solid.iter().fold(glam::Vec3::ZERO, |s, v| s + glam::Vec3::from(v.position))
            / solid.len() as f32;
        assert!(
            (centre.x as f64 - traffic_render::JAR_Y).abs() < 0.05
                && (centre.z as f64 + 1.4 - traffic_render::ORB_Z).abs() < 0.05,
            "{centre:?}"
        );
        for v in &solid {
            let r = (glam::Vec3::from(v.position) - centre).length();
            assert!((r - RADIUS as f32).abs() < 0.01, "{r}");
        }
    }

    #[test]
    fn driving_lights_it_up_and_a_sleeping_orb_is_an_ember() {
        let mut orb = Orb::default();
        for _ in 0..120 {
            orb.advance(1.0 / 60.0, 1.0, 0.0, 5.0, true, 1.0);
        }
        assert!(orb.glow > 1.2 && orb.pull > 0.9);
        for _ in 0..300 {
            orb.advance(1.0 / 60.0, 1.0, 0.0, 5.0, false, 1.0);
        }
        assert!((orb.glow - EMBER_GLOW).abs() < 0.02 && orb.pull < 0.05);
    }

    #[test]
    fn orb_shader_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("cart_orb.wgsl")).expect("parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
            .validate(&module)
            .expect("valid");
    }
}

//! The cart's puller: a 15 cm orb hanging in the air ahead of the car, tied
//! back to the front of the cart by two glowing threads, as a horse is by its
//! traces. It is the magic motor made visible; the substance that drives it is
//! not worked out yet. Rebuilt on the CPU each frame (about a thousand
//! vertices, camera-relative), drawn as one solid and one additive batch.
use crate::driving;
use glam::DVec3;

pub const RADIUS: f64 = 0.15;
/// Orb centre ahead of the cart's front edge, and above the ground.
const LEAD: f64 = 1.5;
const HEIGHT: f64 = 0.95;
/// Where the threads tie on: the front corners of the cart, this high.
const HITCH_HEIGHT: f64 = 0.7;
const FRONT: f64 = driving::WHEELBASE * 0.5 + 0.3;
/// Spring holding the orb to its place (1/s²), a little under critical
/// damping so it swings out on a bend and settles with one small overshoot.
const STIFFNESS: f64 = 30.0;
const DAMPING_RATIO: f64 = 0.7;
const SEGMENTS: usize = 16;
const RINGS: usize = 10;
const THREAD_SEGMENTS: usize = 12;
const THREAD_WIDTH: f64 = 0.012;
const MAX_VERTICES: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    position: [f32; 3],
    /// Orb: surface normal. Halo: quad coordinate in x, y. Thread: across in x.
    normal: [f32; 3],
    /// x glow, w kind (0 orb, 1 halo, 2 thread).
    colour: [f32; 4],
}

/// Where the orb hangs relative to the cart (ahead, right, up), sprung so it
/// swings out on bends, strains ahead when pulling and bobs while it waits.
#[derive(Clone, Copy, Debug)]
pub struct Orb {
    offset: DVec3,
    velocity: DVec3,
    time: f64,
    last_speed: f64,
    /// 0 slack .. 1 hauling (eased throttle while the motor runs).
    pull: f64,
    /// Emissive strength, eased: dim ember when the motor is off.
    glow: f64,
}
impl Default for Orb {
    fn default() -> Self {
        Self {
            offset: DVec3::new(FRONT + LEAD, 0.0, HEIGHT),
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
        let glow = if running { (0.25 + 0.75 * charge) * (1.0 + 0.5 * self.pull) } else { 0.04 };
        ease(&mut self.glow, glow, 3.0);
        let tau = std::f64::consts::TAU;
        let target = DVec3::new(
            FRONT + LEAD + 0.25 * self.pull + (acceleration * 0.02).clamp(-0.2, 0.2),
            steering.clamp(-1.0, 1.0) * 0.6,
            HEIGHT
                + 0.05 * (self.time * tau * 0.55).sin()
                + 0.02 * (self.time * tau * 1.3).sin(),
        );
        let damping = 2.0 * DAMPING_RATIO * STIFFNESS.sqrt();
        let steps = (seconds * 240.0).ceil().max(1.0);
        let step = seconds / steps;
        for _ in 0..steps as usize {
            self.velocity += (STIFFNESS * (target - self.offset) - damping * self.velocity) * step;
            self.offset += self.velocity * step;
        }
        self.time += seconds;
    }

    /// Solid orb, then the additive halo and threads, relative to `camera`.
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
        let centre = ground + forward * self.offset.x + right * self.offset.y + up * self.offset.z;
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
        let mut additive = Vec::with_capacity(6 + 2 * THREAD_SEGMENTS * 6);
        // Halo: a camera-facing square around the orb, faded in the shader.
        let view = (centre - camera).normalize_or_zero();
        let across = view.cross(up).try_normalize().unwrap_or(right);
        let over = across.cross(view);
        let size = RADIUS * 4.0;
        let corner = |x: f64, y: f64| {
            v(centre + (across * x + over * y) * size, [x as f32, y as f32, 0.0], 1.0)
        };
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            additive.push(corner(x, y));
        }
        // Threads back to the cart, slack and sagging until it hauls.
        let sag = 0.12 * (1.0 - self.pull) + 0.01;
        for side in [-1.0, 1.0] {
            let hitch = ground + forward * FRONT + right * (side * driving::TRACK * 0.5) + up * HITCH_HEIGHT;
            let tie = centre + (hitch - centre).normalize_or_zero() * (RADIUS * 0.9);
            let point = |t: f64| tie.lerp(hitch, t) - up * (sag * 4.0 * t * (1.0 - t));
            for i in 0..THREAD_SEGMENTS {
                let (a, b) = (
                    point(i as f64 / THREAD_SEGMENTS as f64),
                    point((i + 1) as f64 / THREAD_SEGMENTS as f64),
                );
                let width = (b - a).cross(a - camera).normalize_or_zero() * (THREAD_WIDTH * 0.5);
                let quad = [(a - width, -1.0), (a + width, 1.0), (b + width, 1.0), (b - width, -1.0)];
                for k in [0, 1, 2, 0, 2, 3] {
                    let (p, x) = quad[k];
                    additive.push(v(p, [x, 0.0, 0.0], 2.0));
                }
            }
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
    fn orb_hangs_ahead_swings_into_bends_and_fits_the_buffer() {
        let mut orb = Orb::default();
        for _ in 0..240 {
            orb.advance(1.0 / 60.0, 0.0, 1.0, 10.0, true, 1.0);
        }
        // Right-hand bend: the orb leads out to the right, still ahead and aloft.
        assert!(orb.offset.y > 0.4 && orb.offset.x > FRONT + 1.0, "{:?}", orb.offset);
        assert!((orb.offset.z - HEIGHT).abs() < 0.1);
        let (eye, forward, up) = (DVec3::new(0.0, 0.0, 4.0e6 + 1.4), DVec3::X, DVec3::Z);
        let (solid, additive) = orb.geometry(eye, forward, up, eye);
        assert!(solid.len() + additive.len() <= MAX_VERTICES);
        // Every orb vertex is 15 cm from a centre ahead of and below the eye.
        let centre = solid.iter().fold(glam::Vec3::ZERO, |s, v| s + glam::Vec3::from(v.position))
            / solid.len() as f32;
        assert!(centre.x > 2.5 && centre.z < 0.0 && centre.y < 0.0, "{centre:?}");
        for v in &solid {
            let r = (glam::Vec3::from(v.position) - centre).length();
            assert!((r - RADIUS as f32).abs() < 0.01, "{r}");
        }
    }

    #[test]
    fn hauling_lights_it_up_and_a_sleeping_orb_is_an_ember() {
        let mut orb = Orb::default();
        for _ in 0..120 {
            orb.advance(1.0 / 60.0, 1.0, 0.0, 5.0, true, 1.0);
        }
        assert!(orb.glow > 1.2 && orb.pull > 0.9);
        for _ in 0..300 {
            orb.advance(1.0 / 60.0, 1.0, 0.0, 5.0, false, 1.0);
        }
        assert!(orb.glow < 0.06 && orb.pull < 0.05);
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

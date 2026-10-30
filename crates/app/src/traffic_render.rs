//! Cached NPC cart/carriage meshes plus the player's own empty cart, each
//! with the magic engine (an orb in an hourglass jar, in a timber box at the
//! back of the bed); one compact instance upload, up to three opaque draws and
//! one translucent glass draw.
use crate::{
    road::{self, Vertex},
    road_network_field::Field,
    traffic,
};
use glam::DVec3;
use wgpu::util::DeviceExt;

pub fn shader_source() -> String {
    let road = include_str!("road.wgsl");
    let start = road.find("fn lantern_hash(").expect("road lamp helpers");
    let end = road
        .find("fn project_vertex(")
        .expect("end of road lamp helpers");
    format!("{}\n{}", include_str!("traffic.wgsl"), &road[start..end])
}

/// The player's cart for this frame, planet frame.
#[derive(Clone, Copy, Debug)]
pub struct PlayerCart {
    /// Ground point under the body centre.
    pub ground: DVec3,
    pub forward: DVec3,
    pub up: DVec3,
    /// Front wheel angle, radians, positive right.
    pub steer: f64,
    /// Signed metres rolled.
    pub travelled: f64,
    /// Tunnel lamp group, None out of doors.
    pub light: Option<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    origin_spin: [f32; 4],
    right_steer: [f32; 4],
    forward_tunnel: [f32; 4],
    up_light: [f32; 4],
    field_fade: [f32; 4],
    tint: [f32; 4],
}
fn material(mesh: &mut [Vertex], part: u32, tag: u32) {
    for v in mesh {
        v.lighting = part | (tag << 8);
    }
}
fn box_part(mesh: &mut Vec<Vertex>, lo: DVec3, hi: DVec3, colour: [f32; 3], part: u32, tag: u32) {
    let start = mesh.len();
    road::box_mesh(mesh, lo, hi, colour);
    material(&mut mesh[start..], part, tag);
}
fn sphere(mesh: &mut Vec<Vertex>, centre: DVec3, r: f64, colour: [f32; 3], tag: u32) {
    for ring in 0..5 {
        for segment in 0..10 {
            let p = |r0: usize, s: usize| {
                let a = r0 as f64 * std::f64::consts::PI / 5.0;
                let b = s as f64 * std::f64::consts::TAU / 10.0;
                centre + DVec3::new(a.sin() * b.cos(), a.sin() * b.sin(), a.cos()) * r
            };
            let q = [
                p(ring, segment),
                p(ring, segment + 1),
                p(ring + 1, segment + 1),
                p(ring + 1, segment),
            ];
            for [a, b, c] in [[q[0], q[2], q[1]], [q[0], q[3], q[2]]] {
                let n = (b - a).cross(c - a);
                if n.length_squared() < 1e-16 {
                    continue;
                }
                let n = n.normalize().as_vec3().to_array();
                for p in [a, b, c] {
                    mesh.push(Vertex {
                        position: p.as_vec3().to_array(),
                        normal: n,
                        colour,
                        lighting: tag << 8,
                    });
                }
            }
        }
    }
}

fn wheel(mesh: &mut Vec<Vertex>, pivot: DVec3, part: u32) {
    let outer = 0.52;
    let inner = 0.43;
    let depth = 0.085;
    let colour = [0.27, 0.18, 0.09];
    for i in 0..16 {
        let point = |r: f64, side: f64, k: usize| {
            let angle = k as f64 * std::f64::consts::TAU / 16.0;
            pivot + DVec3::new(side, angle.sin() * r, angle.cos() * r)
        };
        for q in [
            [
                point(outer, -depth, i),
                point(outer, -depth, i + 1),
                point(outer, depth, i + 1),
                point(outer, depth, i),
            ],
            [
                point(inner, depth, i),
                point(inner, depth, i + 1),
                point(inner, -depth, i + 1),
                point(inner, -depth, i),
            ],
            [
                point(inner, -depth, i),
                point(inner, -depth, i + 1),
                point(outer, -depth, i + 1),
                point(outer, -depth, i),
            ],
            [
                point(outer, depth, i),
                point(outer, depth, i + 1),
                point(inner, depth, i + 1),
                point(inner, depth, i),
            ],
        ] {
            let start = mesh.len();
            road::quad(mesh, [q[0], q[3], q[2], q[1]], colour);
            material(&mut mesh[start..], part, 0);
        }
    }
    for i in 0..6 {
        let angle = i as f64 * std::f64::consts::PI / 6.0;
        let start = mesh.len();
        box_part(
            mesh,
            DVec3::new(-0.055, -0.04, -inner),
            DVec3::new(0.055, 0.04, inner),
            colour,
            part,
            0,
        );
        for v in &mut mesh[start..] {
            let p = DVec3::from(v.position.map(f64::from));
            let n = DVec3::from(v.normal.map(f64::from));
            let rotate = |p: DVec3| {
                DVec3::new(
                    p.x,
                    p.y * angle.cos() - p.z * angle.sin(),
                    p.y * angle.sin() + p.z * angle.cos(),
                )
            };
            v.position = (pivot + rotate(p)).as_vec3().to_array();
            v.normal = rotate(n).as_vec3().to_array();
        }
    }
    box_part(
        mesh,
        pivot - DVec3::splat(0.13),
        pivot + DVec3::splat(0.13),
        [0.035, 0.03, 0.025],
        part,
        1,
    );
}
pub const ORB_RADIUS: f64 = 0.12;
/// The engine box stands at the back of the bed (the bed's rear edge is at
/// y = -1.75, its top at z = 0.88): base board, four corner posts and a lid,
/// open between them so the jar shows.
const BOX_HALF_WIDTH: f64 = 0.3;
const BOX_REAR: f64 = -1.62;
const BOX_FRONT: f64 = -1.02;
const BED_TOP: f64 = 0.88;
const BOARD: f64 = 0.06;
/// The jar stands between base board and lid on the box's axis.
pub const JAR_Y: f64 = (BOX_REAR + BOX_FRONT) * 0.5;
const JAR_BOTTOM: f64 = BED_TOP + BOARD;
const JAR_HEIGHT: f64 = 0.72;
/// The orb rests at the middle of the lower bulb.
pub const ORB_Z: f64 = JAR_BOTTOM + JAR_HEIGHT * 0.25;
/// Hourglass profile: radius of the glass at height `z` above the ground,
/// two round bulbs with a narrow neck between them and at each end.
pub fn jar_radius(z: f64) -> f64 {
    let t = ((z - JAR_BOTTOM) / JAR_HEIGHT).clamp(0.0, 1.0);
    let u = if t >= 1.0 { 1.0 } else { (2.0 * t).fract() };
    0.04 + 0.15 * (std::f64::consts::PI * u).sin().max(0.0).sqrt()
}
/// The jar's glass, drawn translucent for every vehicle (smooth normals).
fn glass() -> Vec<Vertex> {
    const SIDES: usize = 20;
    const STACKS: usize = 36;
    let point = |i: usize, k: usize| {
        let z = JAR_BOTTOM + JAR_HEIGHT * k as f64 / STACKS as f64;
        let a = std::f64::consts::TAU * i as f64 / SIDES as f64;
        let r = jar_radius(z);
        // Normal of the surface of revolution from the profile's slope.
        let dz = 1e-3;
        let slope = (jar_radius(z + dz) - jar_radius(z - dz)) / (2.0 * dz);
        let n = DVec3::new(a.cos(), a.sin(), -slope).normalize();
        (DVec3::new(r * a.cos(), JAR_Y + r * a.sin(), z), n)
    };
    let mut mesh = Vec::with_capacity(SIDES * STACKS * 6);
    for k in 0..STACKS {
        for i in 0..SIDES {
            let q = [point(i, k), point(i + 1, k), point(i + 1, k + 1), point(i, k + 1)];
            for j in [0, 1, 2, 0, 2, 3] {
                mesh.push(Vertex {
                    position: q[j].0.as_vec3().to_array(),
                    normal: q[j].1.as_vec3().to_array(),
                    colour: [1.0; 3],
                    lighting: 6 << 8,
                });
            }
        }
    }
    mesh
}
fn engine_box(mesh: &mut Vec<Vertex>) {
    let oak = [0.30, 0.19, 0.09];
    let (w, b) = (BOX_HALF_WIDTH, BOARD);
    let top = JAR_BOTTOM + JAR_HEIGHT;
    for (z0, z1) in [(BED_TOP, BED_TOP + b), (top, top + b)] {
        box_part(mesh, DVec3::new(-w, BOX_REAR, z0), DVec3::new(w, BOX_FRONT, z1), oak, 0, 0);
    }
    for x in [-w, w - b] {
        for y in [BOX_REAR, BOX_FRONT - b] {
            box_part(mesh, DVec3::new(x, y, BED_TOP + b), DVec3::new(x + b, y + b, top), oak, 0, 0);
        }
    }
    // Iron collars where the jar's necks meet the boards.
    for (z0, z1) in [(JAR_BOTTOM, JAR_BOTTOM + 0.03), (top - 0.03, top)] {
        box_part(
            mesh,
            DVec3::new(-0.06, JAR_Y - 0.06, z0),
            DVec3::new(0.06, JAR_Y + 0.06, z1),
            [0.035, 0.032, 0.028],
            0,
            1,
        );
    }
}
/// An NPC cart or carriage (`npc`), or the player's own: an empty open cart
/// with no tailboard, driver or fixed orb, so the view from its seat is clear
/// and the player's sprung orb (cart_orb) shows behind it.
fn mesh(carriage: bool, npc: bool) -> Vec<Vertex> {
    let mut mesh = Vec::new();
    let oak = [0.40, 0.27, 0.14];
    let iron = [0.035, 0.032, 0.028];
    box_part(
        &mut mesh,
        DVec3::new(-0.78, -1.75, 0.72),
        DVec3::new(0.78, 1.75, 0.88),
        oak,
        0,
        0,
    );
    for x in [-0.58, 0.58] {
        box_part(
            &mut mesh,
            DVec3::new(x - 0.06, -1.8, 0.5),
            DVec3::new(x + 0.06, 1.8, 0.73),
            iron,
            0,
            1,
        );
    }
    for (i, (x, y)) in [(-0.94, -1.15), (0.94, -1.15), (-0.94, 1.15), (0.94, 1.15)]
        .into_iter()
        .enumerate()
    {
        wheel(&mut mesh, DVec3::new(x, y, 0.52), i as u32 + 1);
    }
    for x in [-0.76, 0.76] {
        box_part(
            &mut mesh,
            DVec3::new(x - 0.045, -1.7, 0.85),
            DVec3::new(x + 0.045, 0.85, 1.45),
            oak,
            0,
            0,
        );
        for y in [-1.65, -0.4, 0.8] {
            box_part(
                &mut mesh,
                DVec3::new(x - 0.07, y - 0.045, 0.82),
                DVec3::new(x + 0.07, y + 0.045, 1.6),
                [0.17, 0.09, 0.035],
                0,
                0,
            );
        }
    }
    engine_box(&mut mesh);
    if npc {
        box_part(
            &mut mesh,
            DVec3::new(-0.76, -1.75, 0.86),
            DVec3::new(0.76, -1.65, 1.45),
            oak,
            0,
            0,
        );
    }
    box_part(
        &mut mesh,
        DVec3::new(-0.7, 0.75, 1.17),
        DVec3::new(0.7, 1.05, 1.32),
        oak,
        0,
        0,
    );
    if carriage {
        // Weathered canvas over a timber barrel frame, open at the driver's seat.
        for i in 0..12 {
            let a = i as f64 * std::f64::consts::PI / 12.0;
            let b = (i + 1) as f64 * std::f64::consts::PI / 12.0;
            let p =
                |angle: f64, y: f64| DVec3::new(-0.79 * angle.cos(), y, 1.45 + angle.sin() * 0.95);
            let start = mesh.len();
            road::quad(
                &mut mesh,
                [p(a, -1.72), p(b, -1.72), p(b, 0.65), p(a, 0.65)],
                [0.53, 0.44, 0.29],
            );
            material(&mut mesh[start..], 0, 2);
        }
    } else if npc {
        // Cargo forward of the engine box.
        for (x, y, h) in [(-0.35, -0.45, 0.48), (0.32, -0.55, 0.38), (0.0, 0.38, 0.30)] {
            box_part(
                &mut mesh,
                DVec3::new(x - 0.29, y - 0.35, 0.88),
                DVec3::new(x + 0.29, y + 0.35, 0.88 + h),
                [0.28, 0.19, 0.09],
                0,
                0,
            );
        }
    }
    if !npc {
        return mesh;
    }
    // Seated driver: coat, hood, boots and hands (no per-frame skeletal work).
    box_part(
        &mut mesh,
        DVec3::new(-0.22, 0.82, 1.28),
        DVec3::new(0.22, 1.12, 1.86),
        [0.065, 0.095, 0.06],
        0,
        5,
    );
    sphere(
        &mut mesh,
        DVec3::new(0.0, 1.02, 2.02),
        0.16,
        [0.30, 0.17, 0.09],
        4,
    );
    for x in [-0.16, 0.16] {
        box_part(
            &mut mesh,
            DVec3::new(x - 0.06, 1.05, 1.0),
            DVec3::new(x + 0.06, 1.46, 1.25),
            [0.055, 0.04, 0.025],
            0,
            1,
        );
        box_part(
            &mut mesh,
            DVec3::new(x - 0.045, 1.08, 1.58),
            DVec3::new(x + 0.045, 1.47, 1.7),
            [0.075, 0.10, 0.055],
            0,
            5,
        );
    }
    // The same magic engine as the player, not an invisible horse.
    sphere(
        &mut mesh,
        DVec3::new(0.0, JAR_Y, ORB_Z),
        ORB_RADIUS,
        [0.28, 0.025, 0.65],
        3,
    );
    mesh
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    meshes: [std::ops::Range<u32>; 3],
    glass: std::ops::Range<u32>,
    instances: wgpu::Buffer,
    frame: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    counts: [u32; 3],
    upload: Vec<Instance>,
}
impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
    ) -> Self {
        let mut vertices = mesh(false, true);
        let boundary = vertices.len() as u32;
        vertices.extend(mesh(true, true));
        let player = vertices.len() as u32;
        vertices.extend(mesh(false, false));
        let count = vertices.len() as u32;
        vertices.extend(glass());
        let glass_end = vertices.len() as u32;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cached traffic cart and carriage"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("traffic instances"),
            size: ((traffic::MAX_AGENTS + 1) * size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("traffic lighting frame"),
            contents: bytemuck::cast_slice(&[[0.0_f32; 4]; 3]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("traffic frame layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("traffic frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            }],
        });
        let lighting = crate::road_lighting::layout(device);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("traffic pipeline layout"),
            bind_group_layouts: &[Some(camera), Some(&lighting), Some(&frame_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("traffic shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        const V: [wgpu::VertexAttribute; 4] =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,9=>Uint32];
        const I: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Float32x4];
        let make = |label: &str, entry: &str, blend: wgpu::BlendState, write: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &V,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &I,
                    },
                ],
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
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(),
        })
        };
        let pipeline = make("instanced traffic", "fs_main", wgpu::BlendState::REPLACE, true);
        // The engine jars: translucent, after the solid vehicles, no depth write.
        let glass_pipeline =
            make("instanced traffic glass", "fs_glass", wgpu::BlendState::ALPHA_BLENDING, false);
        let _ = queue;
        Self {
            pipeline,
            glass_pipeline,
            vertices: buffer,
            meshes: [0..boundary, boundary..player, player..count],
            glass: count..glass_end,
            instances,
            frame,
            frame_group,
            counts: [0; 3],
            upload: Vec::with_capacity(traffic::MAX_AGENTS + 1),
        }
    }
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        traffic: &traffic::Controller,
        field: &std::sync::Arc<Field>,
        eye: DVec3,
        view: DVec3,
        player: Option<PlayerCart>,
    ) {
        self.upload.clear();
        self.counts = [0; 3];
        let f = field.frame;
        let vec4 = |p: DVec3, w: f64| [p.x as f32, p.y as f32, p.z as f32, w as f32];
        queue.write_buffer(
            &self.frame,
            0,
            bytemuck::cast_slice(&[vec4(f.across, 0.0), vec4(f.forward, 0.0), vec4(f.up, 0.0)]),
        );
        let agents = traffic.simulation.as_ref().map_or(&[][..], |sim| &sim.agents[..]);
        let alpha = traffic.simulation.as_ref().map_or(0.0, |sim| sim.alpha());
        for kind in 0..2 {
            for a in agents.iter().filter(|a| a.kind == kind) {
                let pose = a.pose(alpha);
                let relative = pose.position - eye;
                if relative.length_squared() > 2000.0 * 2000.0 || relative.dot(view) < -5.0 {
                    continue;
                }
                let right = pose.forward.cross(pose.position.normalize()).normalize();
                let up = right.cross(pose.forward).normalize();
                let ahead = a.future_pose(3.0);
                let yaw = pose
                    .forward
                    .cross(ahead.forward)
                    .dot(up)
                    .atan2(pose.forward.dot(ahead.forward));
                let steer = (2.3 * yaw / 3.0).atan().clamp(-0.7, 0.7);
                let matching = traffic.matches_field(field);
                let light = if matching && !a.stranded {
                    pose.light
                } else {
                    0
                };
                let tint = if a.id % 4 == 0 {
                    [0.65, 0.42, 0.28, 0.0]
                } else {
                    [1.0, 0.85, 0.66, 0.0]
                };
                self.upload.push(Instance {
                    origin_spin: vec4(relative, a.wheel_angle()),
                    right_steer: vec4(right, steer),
                    forward_tunnel: vec4(pose.forward, if pose.tunnel { 1.0 } else { 0.0 }),
                    up_light: vec4(up, light as f64),
                    field_fade: vec4(f.local(pose.position), a.fade),
                    tint,
                });
                self.counts[kind as usize] += 1;
            }
        }
        if let Some(cart) = player {
            let right = cart.forward.cross(cart.up).normalize();
            self.upload.push(Instance {
                origin_spin: vec4(
                    cart.ground - eye,
                    (cart.travelled / 0.52).rem_euclid(std::f64::consts::TAU),
                ),
                // The shader's steer turns the wheels left for positive angles.
                right_steer: vec4(right, (-cart.steer).clamp(-0.7, 0.7)),
                forward_tunnel: vec4(cart.forward, if cart.light.is_some() { 1.0 } else { 0.0 }),
                up_light: vec4(cart.up, cart.light.unwrap_or(0) as f64),
                field_fade: vec4(f.local(cart.ground), 1.0),
                tint: [1.0, 0.85, 0.66, 0.0],
            });
            self.counts[2] = 1;
        }
        if !self.upload.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.upload));
        }
    }
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera: &'a wgpu::BindGroup,
        lights: &'a wgpu::BindGroup,
    ) {
        if self.upload.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, lights, &[]);
        pass.set_bind_group(2, &self.frame_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        let mut first = 0;
        for i in 0..3 {
            if self.counts[i] > 0 {
                pass.draw(self.meshes[i].clone(), first..first + self.counts[i]);
                first += self.counts[i];
            }
        }
        pass.set_pipeline(&self.glass_pipeline);
        pass.draw(self.glass.clone(), 0..first);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traffic_meshes_are_finite_human_scale_and_have_moving_wheels() {
        for (enclosed, npc) in [(false, true), (true, true), (false, false)] {
            let m = mesh(enclosed, npc);
            assert_eq!(m.len() % 3, 0);
            for v in &m {
                assert!(glam::Vec3::from(v.position).is_finite());
                assert!(glam::Vec3::from(v.normal).is_finite());
                assert!(glam::Vec3::from(v.position).length() < 4.0);
            }
            for i in 1..=4 {
                assert!(m.iter().any(|v| v.lighting & 255 == i));
            }
            // The NPC orb rests in the jar's lower bulb, at the back of the bed.
            let magic: Vec<_> = m.iter().filter(|v| v.lighting >> 8 == 3).collect();
            for v in &magic {
                let p = DVec3::from(v.position.map(f64::from));
                assert!((p - DVec3::new(0.0, JAR_Y, ORB_Z)).length() < ORB_RADIUS + 1e-3, "{p:?}");
            }
            // The player's cart has no fixed orb or driver: its own sloshing
            // orb (cart_orb) draws in the jar and the view is from the seat.
            assert_eq!(magic.is_empty(), !npc);
            assert_eq!(m.iter().any(|v| v.lighting >> 8 == 4), npc);
        }
    }
    #[test]
    fn engine_jar_is_an_hourglass_that_holds_the_sloshing_orb_inside_the_box() {
        // Two bulbs with a neck between them and at each end.
        let r = |t: f64| jar_radius(JAR_BOTTOM + JAR_HEIGHT * t);
        assert!(r(0.25) > 0.18 && r(0.75) > 0.18);
        assert!(r(0.0) < 0.05 && r(0.5) < 0.05 && r(1.0) < 0.05);
        // The orb, strayed as far as it may in any direction, stays inside the
        // glass with a centimetre to spare.
        let reach = crate::cart_orb::SLOSH;
        for k in 0..=200 {
            let z = ORB_Z - ORB_RADIUS - reach + (ORB_RADIUS + reach) * 2.0 * k as f64 / 200.0;
            let half = (ORB_RADIUS + reach).powi(2) - (z - ORB_Z).powi(2);
            if half > 0.0 {
                assert!(half.sqrt() + 0.01 <= jar_radius(z), "z {z}");
            }
        }
        // Glass inside the box's posts, box on the bed ahead of its rear edge
        // and clear of the moved cargo.
        for v in glass() {
            let p = DVec3::from(v.position.map(f64::from));
            assert!(p.x.abs() < BOX_HALF_WIDTH - BOARD && p.y > BOX_REAR + BOARD && p.y < BOX_FRONT - BOARD);
        }
        assert!(BOX_REAR > -1.65 && BOX_FRONT < -0.9 - 0.1);
    }
    #[test]
    fn driver_and_magic_orb_spheres_are_closed_and_face_outward() {
        let mut mesh = Vec::new();
        sphere(&mut mesh, DVec3::ZERO, 1.0, [1.0; 3], 0);
        assert_eq!(mesh.len(), 240); // 80 non-degenerate triangles, both pole caps.
        for v in &mesh {
            assert!(glam::Vec3::from(v.normal).dot(glam::Vec3::from(v.position)) > 0.85);
        }
    }
    #[test]
    fn traffic_shader_and_instance_layout_validate() {
        assert_eq!(size_of::<Instance>(), 96);
        let m = wgpu::naga::front::wgsl::parse_str(&shader_source()).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .unwrap();
    }
}

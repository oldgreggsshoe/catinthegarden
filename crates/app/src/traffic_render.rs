//! Two cached cart/carriage meshes; one compact instance upload, two draws.
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
fn mesh(carriage: bool) -> Vec<Vertex> {
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
    box_part(
        &mut mesh,
        DVec3::new(-0.76, -1.75, 0.86),
        DVec3::new(0.76, -1.65, 1.45),
        oak,
        0,
        0,
    );
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
    } else {
        for (x, y, h) in [(-0.35, -0.8, 0.48), (0.32, -0.9, 0.38), (0.0, 0.05, 0.30)] {
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
    // The same magic-drive language as the player, not an invisible horse.
    sphere(
        &mut mesh,
        DVec3::new(0.0, 2.55, 1.1),
        0.12,
        [0.28, 0.025, 0.65],
        3,
    );
    for x in [-0.5, 0.5] {
        let start = mesh.len();
        road::quad(
            &mut mesh,
            [
                DVec3::new(x - 0.008, 1.7, 0.85),
                DVec3::new(x + 0.008, 1.7, 0.85),
                DVec3::new(0.008, 2.55, 1.1),
                DVec3::new(-0.008, 2.55, 1.1),
            ],
            [0.12, 0.015, 0.3],
        );
        material(&mut mesh[start..], 0, 3);
    }
    mesh
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    meshes: [std::ops::Range<u32>; 2],
    instances: wgpu::Buffer,
    frame: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    counts: [u32; 2],
    upload: Vec<Instance>,
}
impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
    ) -> Self {
        let mut vertices = mesh(false);
        let boundary = vertices.len() as u32;
        vertices.extend(mesh(true));
        let count = vertices.len() as u32;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cached traffic cart and carriage"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("traffic instances"),
            size: (traffic::MAX_AGENTS * size_of::<Instance>()) as u64,
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("instanced traffic"),
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
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(),
        });
        let _ = queue;
        Self {
            pipeline,
            vertices: buffer,
            meshes: [0..boundary, boundary..count],
            instances,
            frame,
            frame_group,
            counts: [0; 2],
            upload: Vec::with_capacity(traffic::MAX_AGENTS),
        }
    }
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        traffic: &traffic::Controller,
        field: &std::sync::Arc<Field>,
        eye: DVec3,
        view: DVec3,
    ) {
        self.upload.clear();
        self.counts = [0; 2];
        let Some(sim) = &traffic.simulation else {
            return;
        };
        let f = field.frame;
        let vec4 = |p: DVec3, w: f64| [p.x as f32, p.y as f32, p.z as f32, w as f32];
        queue.write_buffer(
            &self.frame,
            0,
            bytemuck::cast_slice(&[vec4(f.across, 0.0), vec4(f.forward, 0.0), vec4(f.up, 0.0)]),
        );
        for kind in 0..2 {
            for a in sim.agents.iter().filter(|a| a.kind == kind) {
                let pose = a.pose(sim.alpha());
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
        for i in 0..2 {
            if self.counts[i] > 0 {
                pass.draw(self.meshes[i].clone(), first..first + self.counts[i]);
                first += self.counts[i];
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traffic_meshes_are_finite_human_scale_and_have_moving_wheels() {
        for enclosed in [false, true] {
            let m = mesh(enclosed);
            assert_eq!(m.len() % 3, 0);
            for v in &m {
                assert!(glam::Vec3::from(v.position).is_finite());
                assert!(glam::Vec3::from(v.normal).is_finite());
                assert!(glam::Vec3::from(v.position).length() < 4.0);
            }
            for i in 1..=4 {
                assert!(m.iter().any(|v| v.lighting & 255 == i));
            }
        }
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

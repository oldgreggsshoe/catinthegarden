//! GPU side of the flock: one static vertex buffer for the shared bird, and an
//! instance buffer rewritten each frame with every visible bird's pose.
//!
//! Split from `birds` the way `ship_render` is split from `ship`, so the
//! flocking stays testable without a device.

use glam::DVec3;

use crate::birds::{self, Bird, BirdActivity, BirdVertex};

pub fn birds_shader_source() -> String {
    include_str!("birds.wgsl").to_string()
}

/// Body length of a rendered bird. The mesh is modelled about one unit long.
const BIRD_BODY_LENGTH_METERS: f32 = 0.42;
/// Beyond this a bird is well under a pixel and is not worth an instance.
const BIRD_DRAW_DISTANCE_METERS: f64 = 620.0;
/// Exactly the worst case the simulation can present -- every flock at the
/// merge ceiling -- rather than a round number chosen to look safe. Derived, so
/// raising the flock cap or the merge ceiling resizes the buffer with it.
const MAX_BIRD_INSTANCES: usize = birds::worst_case_bird_count();

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct BirdInstance {
    view_position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    motion: [f32; 4],
}

impl BirdVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x3];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

impl BirdInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![4 => Float32x3, 5 => Float32x3, 6 => Float32x3, 7 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct BirdRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    scratch: Vec<BirdInstance>,
}

impl BirdRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let mesh = birds::build_mesh();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bird vertices"),
            size: (mesh.len() * size_of::<BirdVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&mesh));

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bird instances"),
            size: (MAX_BIRD_INSTANCES * size_of::<BirdInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bird pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bird shader"),
            source: wgpu::ShaderSource::Wgsl(birds_shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bird pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[BirdVertex::layout(), BirdInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // The wing quads are emitted with both windings, so a bird
                // overhead keeps its wings; culling still halves the body.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // Reversed-Z, matching every other pass.
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
            vertex_buffer,
            vertex_count: mesh.len() as u32,
            instance_buffer,
            instance_count: 0,
            scratch: Vec::with_capacity(MAX_BIRD_INSTANCES),
        }
    }

    pub fn triangle_count(&self) -> u32 {
        self.vertex_count / 3 * self.instance_count
    }

    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }

    /// `camera_local` and the birds are both in the planet frame; the
    /// difference is taken here in f64 and only then narrows to f32, which is
    /// the same contract the ship's hull origin has.
    /// `alpha` is `BirdFlocks::interpolation_alpha`: how far this frame sits
    /// between the last completed 30Hz step and the next. Drawing the raw
    /// stepped pose repeats a position for several frames and then jumps, which
    /// reads as a stutter and, at wingbeat rate, as the bird hopping in place.
    pub fn update<'a>(
        &mut self,
        queue: &wgpu::Queue,
        birds: impl Iterator<Item = &'a Bird>,
        camera_local: DVec3,
        alpha: f64,
        world_to_view: impl Fn(DVec3) -> DVec3,
    ) {
        self.scratch.clear();
        for bird in birds {
            if self.scratch.len() >= MAX_BIRD_INSTANCES {
                break;
            }
            let position = bird.position_at(alpha);
            let offset = position - camera_local;
            if offset.length() > BIRD_DRAW_DISTANCE_METERS {
                continue;
            }
            let up = position.normalize();
            // A bird that has just touched down may have no tangential velocity
            // at all for a step; hold it pointing along local east rather than
            // letting the basis collapse.
            let fallback = fallback_heading(up);
            let forward = bird.heading(fallback);
            let fold = if bird.activity == BirdActivity::Walking {
                1.0
            } else {
                0.0
            };
            self.scratch.push(BirdInstance {
                view_position: world_to_view(offset).as_vec3().to_array(),
                forward: forward.as_vec3().to_array(),
                up: up.as_vec3().to_array(),
                motion: [
                    bird.wing_phase_at(alpha),
                    fold,
                    BIRD_BODY_LENGTH_METERS,
                    bird.bank_at(alpha),
                ],
            });
        }
        self.instance_count = self.scratch.len() as u32;
        if self.instance_count > 0 {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.scratch),
            );
        }
    }

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
    ) {
        if self.instance_count == 0 || self.vertex_count == 0 {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..self.instance_count);
    }
}

fn fallback_heading(up: DVec3) -> DVec3 {
    let reference = if up.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    up.cross(reference).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn bird_shader_parses_and_validates() {
        let shader = birds_shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&shader).expect("bird shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("bird shader must validate");
    }

    /// The bank has to reach the frame, not merely reach the GPU.
    ///
    /// The simulation can compute a perfect roll and the instance buffer can
    /// carry it, and the bird still flies flat if the vertex shader builds its
    /// frame straight from the planetary radial -- which is exactly what it did
    /// before. So this checks the value is consumed where it matters: the
    /// frame's right and up must both be built from the bank, not from the
    /// level axes.
    #[test]
    fn the_vertex_shader_rolls_the_bird_frame_by_its_bank() {
        let shader = birds_shader_source();
        let body = shader
            .split_once("fn vs_main")
            .expect("the vertex entry point exists")
            .1;
        assert!(
            body.contains("instance.motion.w"),
            "the vertex shader never reads the bank"
        );
        // Both axes, or the frame is sheared rather than rolled.
        for axis in ["let right =", "let up ="] {
            let line = body
                .lines()
                .find(|line| line.trim_start().starts_with(axis))
                .unwrap_or_else(|| panic!("the frame defines `{axis}`"));
            assert!(
                line.contains("bank_cos") && line.contains("bank_sin"),
                "`{}` is built without the bank, so the bird stays level",
                line.trim()
            );
        }
    }

    #[test]
    fn the_mesh_is_whole_triangles_with_wings_that_hinge_at_the_root() {
        let mesh = birds::build_mesh();
        assert_eq!(mesh.len() % 3, 0, "mesh must be whole triangles");
        assert!(!mesh.is_empty());
        // Body vertices never hinge; wing vertices carry a side and two bone
        // weights. A hand weight without an arm weight would hinge the hand off
        // a shoulder that had not moved, which is the dragonfly stroke again.
        for vertex in &mesh {
            let [side, arm, hand] = vertex.flap;
            assert!(side == -1.0 || side == 0.0 || side == 1.0, "side {side}");
            assert!((0.0..=1.0).contains(&arm), "arm {arm}");
            assert!((0.0..=1.0).contains(&hand), "hand {hand}");
            if side == 0.0 {
                assert_eq!(arm, 0.0, "a body vertex must not hinge");
                assert_eq!(hand, 0.0, "a body vertex has no hand");
            }
            assert!(hand <= arm, "hand {hand} hinges off an unmoved arm {arm}");
        }
        // The wing really is in two pieces: some vertices are out at the wrist
        // with the arm fully bent but the hand not yet, which is the joint.
        assert!(
            mesh.iter().any(|v| v.flap[1] == 1.0 && v.flap[2] == 0.0),
            "no wrist: the wing is still one rigid bone"
        );
        assert!(
            mesh.iter().any(|v| v.flap[2] == 1.0),
            "no wingtip on the hand"
        );
        // Both wings are present and mirrored.
        let left = mesh.iter().filter(|v| v.flap[0] < 0.0).count();
        let right = mesh.iter().filter(|v| v.flap[0] > 0.0).count();
        assert!(left > 0 && left == right, "left {left} right {right}");
        // A wing tip is further out than any body vertex.
        let widest_body = mesh
            .iter()
            .filter(|v| v.flap[0] == 0.0)
            .fold(0.0_f32, |widest, v| widest.max(v.position[0].abs()));
        let widest_wing = mesh
            .iter()
            .filter(|v| v.flap[2] > 0.5)
            .fold(0.0_f32, |widest, v| widest.max(v.position[0].abs()));
        assert!(widest_wing > widest_body * 3.0, "wings must span the body");
    }

    #[test]
    fn the_shader_hinges_the_hand_at_the_mesh_wrist() {
        // The shader rotates the hand about a point it declares itself. If that
        // point is not where the mesh actually has its joint, the wing tears
        // open mid-beat -- the hand pivots about thin air and separates from the
        // arm it is supposed to be attached to.
        let shader = birds_shader_source();
        let declared = shader
            .split("const WRIST_LOCAL: vec3<f32> = vec3<f32>(")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .expect("the shader declares a wrist");
        let numbers: Vec<f32> = declared
            .split(',')
            .map(|part| part.trim().parse::<f32>().expect("a wrist coordinate"))
            .collect();
        assert_eq!(numbers.len(), 3);
        for (index, value) in numbers.iter().enumerate() {
            assert!(
                (value - birds::WING_WRIST_LOCAL[index]).abs() < 1.0e-6,
                "shader wrist {numbers:?} is not the mesh wrist {:?}",
                birds::WING_WRIST_LOCAL
            );
        }

        // And the mesh really does have vertices there, on both wings.
        for side in [-1.0_f32, 1.0] {
            let wrist_x = birds::WING_WRIST_LOCAL[0] * side;
            assert!(
                birds::build_mesh().iter().any(|vertex| {
                    vertex.flap[1] == 1.0
                        && vertex.flap[2] == 0.0
                        && (vertex.position[0] - wrist_x).abs() < 0.02
                }),
                "no wrist vertices on the {side} wing near x {wrist_x}"
            );
        }
    }

    #[test]
    fn every_vertex_carries_a_unit_normal() {
        for vertex in birds::build_mesh() {
            let normal = Vec3::from(vertex.normal);
            assert!(
                (normal.length() - 1.0).abs() < 1.0e-5,
                "normal {normal:?} is not unit length"
            );
        }
    }

    #[test]
    fn the_instance_buffer_has_room_for_every_bird_a_flock_set_can_hold() {
        let mut flocks = birds::BirdFlocks::new(1);
        let radius = 4_000_000.0;
        let camera = DVec3::new(0.0, 0.0, radius + 2.0);
        let ground = |_direction: DVec3| {
            Some(birds::GroundSample {
                surface_radius_meters: radius,
                slope_radians: 0.0,
                walkable: true,
            })
        };
        flocks.advance(1.0, camera, &ground);
        assert!(
            flocks.bird_count() <= MAX_BIRD_INSTANCES,
            "{} birds exceeds the {MAX_BIRD_INSTANCES} instance buffer",
            flocks.bird_count()
        );
        // The live count is not the bound that matters: flocks merge, so the
        // worst case the buffer must hold is every flock at the merge ceiling.
        assert!(
            birds::worst_case_bird_count() <= MAX_BIRD_INSTANCES,
            "{} birds in the worst case exceeds the {MAX_BIRD_INSTANCES} instance buffer",
            birds::worst_case_bird_count()
        );
    }
}

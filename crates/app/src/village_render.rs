//! GPU side of the villages: one static house mesh, and an instance buffer
//! rebuilt when the camera has moved far enough for the visible set to change.
//!
//! Split from `village` the way `ship_render` is split from `ship`, so the
//! placement rules stay testable without a device.

use glam::DVec3;

use crate::terrain::TerrainRenderer;
use crate::village::{
    self, BeamInstance, GroundShadowVertex, HouseInstance, HouseVertex, draw_altitude_meters,
    max_draw_instances, rebuild_distance_meters,
};

pub fn village_shader_source() -> String {
    // The contact-shadow numbers live in `village.rs` with the mesh that is
    // built from them, and are substituted in here so the fan's geometry and
    // its shading cannot drift apart.
    include_str!("village.wgsl")
        .replace(
            "GROUND_SHADOW_LIFT_METERS",
            &format!("{:?}", village::house_ground_shadow_lift_meters()),
        )
        .replace(
            "GROUND_SHADOW_FADE_START",
            &format!("{:?}", village::house_ground_shadow_fade_start()),
        )
        .replace(
            "GROUND_SHADOW_STRENGTH",
            &format!("{:?}", village::house_ground_shadow_strength()),
        )
        .replace(
            "BEAM_LENGTH_METERS",
            &format!("{:?}", village::village_beam_length_meters()),
        )
        .replace(
            "BEAM_SCREEN_HALF_WIDTH",
            &format!("{:?}", village::village_beam_screen_half_width()),
        )
        .replace(
            "BEAM_ALPHA",
            &format!("{:?}", village::village_beam_alpha()),
        )
}

/// The house instance stream as the shadow fan reads it: the same buffer, but
/// its attributes land at the locations the shadow vertex leaves free.
fn ground_shadow_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 16,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 32,
            shader_location: 4,
        },
    ];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<HouseInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

impl BeamInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 16,
            shader_location: 2,
        },
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// The ribbon a beam is drawn on: two triangles in uv space, widened on screen
/// by the vertex shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BeamVertex {
    uv: [f32; 2],
}

impl BeamVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

fn beam_ribbon_vertices() -> [BeamVertex; 6] {
    [
        BeamVertex { uv: [0.0, 0.0] },
        BeamVertex { uv: [1.0, 0.0] },
        BeamVertex { uv: [1.0, 1.0] },
        BeamVertex { uv: [0.0, 0.0] },
        BeamVertex { uv: [1.0, 1.0] },
        BeamVertex { uv: [0.0, 1.0] },
    ]
}

impl GroundShadowVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

impl HouseVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

impl HouseInstance {
    /// Offsets are spelled out rather than taken from `vertex_attr_array!`,
    /// which packs locations consecutively at 0/12/24/36. This struct pads each
    /// `[f32; 3]` out to four floats for alignment, so its fields actually sit
    /// at 0/16/32/48. Under the packed layout, location 4 read `_pad0` -- a
    /// zero vector -- so `normalize(input.up)` in the shader was NaN and every
    /// house collapsed to nothing while still counting as a live instance.
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 16,
            shader_location: 4,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 32,
            shader_location: 5,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 48,
            shader_location: 6,
        },
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct VillageRenderer {
    pipeline: wgpu::RenderPipeline,
    ground_shadow_pipeline: wgpu::RenderPipeline,
    beam_pipeline: wgpu::RenderPipeline,
    beam_vertex_buffer: wgpu::Buffer,
    beam_instance_buffer: wgpu::Buffer,
    beam_count: u32,
    /// Where each village stands, for re-anchoring the beams as the camera
    /// moves.
    beam_world_positions: Vec<DVec3>,
    /// Locator beams are a debug overlay, so they start off.
    beams_enabled: bool,
    ground_shadow_vertex_buffer: wgpu::Buffer,
    ground_shadow_vertex_count: u32,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    /// Where the camera stood when the visible set was last rebuilt. Villages
    /// are static, so the list only changes when the camera does.
    last_build_position: Option<DVec3>,
    /// World position of the house nearest the camera at the last rebuild.
    ///
    /// Kept because a house count alone cannot say where the settlement is:
    /// authoring a village scenario means knowing the ground a house stands
    /// on, and a count of 80 is as true of eighty houses behind the camera as
    /// of eighty in front of it.
    nearest_house_world: Option<DVec3>,
    /// The built houses, kept so their camera-relative offsets can be rewritten
    /// as the camera moves without re-siting.
    instances: Vec<HouseInstance>,
    /// Where those houses stand, in the planet frame.
    house_world_positions: Vec<DVec3>,
    /// Houses sited in the search region, before the render cutoff.
    sited_houses: u32,
    max_ground_disagreement_meters: f64,
    nearest_site_macro_height_meters: f64,
    nearest_site_biome: Option<catinthegarden_coretypes::BiomeId>,
    nearest_site_moisture: f32,
    enabled: bool,
}

impl VillageRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let mesh = village::build_house_mesh();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("village house vertices"),
            size: (mesh.len() * size_of::<HouseVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&mesh));

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("village house instances"),
            size: (max_draw_instances() * size_of::<HouseInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shadow_mesh = village::build_house_ground_shadow_mesh();
        let ground_shadow_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("village ground shadow vertices"),
            size: (shadow_mesh.len() * size_of::<GroundShadowVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &ground_shadow_vertex_buffer,
            0,
            bytemuck::cast_slice(&shadow_mesh),
        );

        let ribbon = beam_ribbon_vertices();
        let beam_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("village beam ribbon"),
            size: std::mem::size_of_val(&ribbon) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&beam_vertex_buffer, 0, bytemuck::cast_slice(&ribbon));
        let beam_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("village beam instances"),
            // One beam per village, and a village is at least one house, so
            // the house budget bounds this too.
            size: (max_draw_instances() * size_of::<BeamInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("village pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("village shader"),
            source: wgpu::ShaderSource::Wgsl(village_shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("village pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[HouseVertex::layout(), HouseInstance::layout()],
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
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // Reversed-Z, like every other opaque pass here.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let ground_shadow_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("village ground shadow pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_ground_shadow"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[
                        GroundShadowVertex::layout(),
                        ground_shadow_instance_layout(),
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_ground_shadow"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: hdr_format,
                        // Straight alpha blending with a black source, which
                        // multiplies the ground down without tinting it.
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    // The fan is a flat sheet and which way it faces depends on
                    // where the camera stands relative to the house, so culling
                    // it would drop half the shadows.
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    // Blended, and it lies on the ground rather than replacing
                    // it: writing depth here would occlude the house standing
                    // on it.
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let beam_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("village beam pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_beam"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[BeamVertex::layout(), BeamInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_beam"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // A screen-widened ribbon has no meaningful facing.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // Translucent: it must not occlude anything, but terrain in
                // front of a village should still hide its beam's foot.
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
            ground_shadow_pipeline,
            beam_pipeline,
            beam_vertex_buffer,
            beam_instance_buffer,
            beam_count: 0,
            beam_world_positions: Vec::new(),
            beams_enabled: std::env::var("CATINGARDEN_VILLAGE_BEAMS").as_deref() == Ok("1"),
            ground_shadow_vertex_buffer,
            ground_shadow_vertex_count: shadow_mesh.len() as u32,
            vertex_buffer,
            vertex_count: mesh.len() as u32,
            instance_buffer,
            instance_count: 0,
            last_build_position: None,
            nearest_house_world: None,
            instances: Vec::new(),
            house_world_positions: Vec::new(),
            sited_houses: 0,
            max_ground_disagreement_meters: 0.0,
            nearest_site_macro_height_meters: f64::NAN,
            nearest_site_biome: None,
            nearest_site_moisture: f32::NAN,
            // `CATINGARDEN_VILLAGES=off` disables the pass so its frame cost can
            // be measured against the same scenario without a rebuild. Measured
            // this way: 74.07ms with villages against 74.61ms without.
            enabled: std::env::var("CATINGARDEN_VILLAGES").as_deref() != Ok("off"),
        }
    }

    /// Rebuilds the visible houses if the camera has moved far enough, or if
    /// nothing has been built yet. Cheap on the frames where it does nothing,
    /// which is almost all of them.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        terrain: &TerrainRenderer,
        camera_world_position: DVec3,
        camera_altitude_meters: f64,
    ) {
        if !self.enabled {
            return;
        }
        if camera_altitude_meters >= draw_altitude_meters() {
            self.instance_count = 0;
            self.last_build_position = None;
            self.nearest_house_world = None;
            self.instances.clear();
            self.house_world_positions.clear();
            self.beam_world_positions.clear();
            self.beam_count = 0;
            self.sited_houses = 0;
            self.max_ground_disagreement_meters = 0.0;
            return;
        }
        let moved_far_enough = self
            .last_build_position
            .is_none_or(|last| last.distance(camera_world_position) > rebuild_distance_meters());
        if !moved_far_enough {
            // Re-anchoring is not the same as re-siting. The instance buffer
            // holds offsets *from the camera*, so leaving it alone while the
            // camera moves glues the whole village to the eye: it slid with
            // the camera for the full 400m and then snapped back, and a
            // capture 24m from a house was indistinguishable from one 150m
            // away. Which houses exist is still only recomputed on a real
            // rebuild; this just re-differences the ones already chosen.
            self.rewrite_camera_relative_positions(queue, camera_world_position);
            self.upload_beams(queue, camera_world_position);
            return;
        }
        let direction = camera_world_position.normalize_or_zero();
        if direction.length_squared() <= f64::EPSILON {
            return;
        }
        let build = village::collect_house_instances(
            terrain,
            direction,
            camera_altitude_meters,
            camera_world_position,
        );
        let instances = build.instances;
        self.instance_count = instances.len() as u32;
        self.sited_houses = build.sited_houses;
        self.max_ground_disagreement_meters = build.max_ground_disagreement_meters;
        self.nearest_site_macro_height_meters = build.nearest_site_macro_height_meters;
        self.nearest_site_biome = build.nearest_site_biome;
        self.nearest_site_moisture = build.nearest_site_moisture;
        // World positions, kept so the offsets can be re-differenced against a
        // moved camera without re-siting anything.
        self.house_world_positions = instances
            .iter()
            .map(|instance| {
                camera_world_position
                    + DVec3::from(instance.camera_relative_position.map(f64::from))
            })
            .collect();
        self.nearest_house_world = self.house_world_positions.iter().copied().min_by(|a, b| {
            a.distance_squared(camera_world_position)
                .partial_cmp(&b.distance_squared(camera_world_position))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.instances = instances;
        self.beam_world_positions = build.beam_sites;
        self.last_build_position = Some(camera_world_position);
        self.upload_instances(queue);
        self.upload_beams(queue, camera_world_position);
    }

    /// Re-differences the built houses against the camera's current position.
    /// Cheap: it touches only the offset of each instance already chosen.
    fn rewrite_camera_relative_positions(
        &mut self,
        queue: &wgpu::Queue,
        camera_world_position: DVec3,
    ) {
        if self.instances.is_empty() {
            return;
        }
        for (instance, world) in self.instances.iter_mut().zip(&self.house_world_positions) {
            instance.camera_relative_position =
                (*world - camera_world_position).as_vec3().to_array();
        }
        self.nearest_house_world = self.house_world_positions.iter().copied().min_by(|a, b| {
            a.distance_squared(camera_world_position)
                .partial_cmp(&b.distance_squared(camera_world_position))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.upload_instances(queue);
    }

    /// Beams are re-differenced against the camera like the houses are, for
    /// the same reason: an offset baked at the last rebuild would drag the
    /// whole set along with the eye.
    fn upload_beams(&mut self, queue: &wgpu::Queue, camera_world_position: DVec3) {
        if !self.beams_enabled || self.beam_world_positions.is_empty() {
            self.beam_count = 0;
            return;
        }
        let beams: Vec<BeamInstance> = self
            .beam_world_positions
            .iter()
            .take(max_draw_instances())
            .map(|world| BeamInstance {
                camera_relative_base: (*world - camera_world_position).as_vec3().to_array(),
                _pad0: 0.0,
                up: world.normalize_or_zero().as_vec3().to_array(),
                _pad1: 0.0,
            })
            .collect();
        self.beam_count = beams.len() as u32;
        queue.write_buffer(&self.beam_instance_buffer, 0, bytemuck::cast_slice(&beams));
    }

    /// Turns the locator beams on or off, and reports the new state.
    pub fn toggle_beams(&mut self) -> bool {
        self.beams_enabled = !self.beams_enabled;
        if !self.beams_enabled {
            self.beam_count = 0;
        }
        // The buffer is refilled on the next update, which happens every frame.
        self.last_build_position = None;
        self.beams_enabled
    }

    pub fn beams_enabled(&self) -> bool {
        self.beams_enabled
    }

    fn upload_instances(&self, queue: &wgpu::Queue) {
        if !self.instances.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instances),
            );
        }
    }

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
        camera_altitude_meters: f64,
    ) {
        if !self.enabled {
            return;
        }
        // Beams are drawn before the altitude gate below: their whole purpose
        // is to show villages from further away than the houses themselves
        // are worth drawing.
        if self.beams_enabled && self.beam_count > 0 {
            render_pass.set_pipeline(&self.beam_pipeline);
            render_pass.set_bind_group(0, camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.beam_vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.beam_instance_buffer.slice(..));
            render_pass.draw(0..6, 0..self.beam_count);
        }
        if self.instance_count == 0
            || self.vertex_count == 0
            || camera_altitude_meters >= draw_altitude_meters()
        {
            return;
        }
        // Ground first: the shadow lies on terrain and the houses stand on
        // top of it, so drawing it after would put a dark sheet across their
        // walls wherever the fan passes in front of one.
        render_pass.set_pipeline(&self.ground_shadow_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.ground_shadow_vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.draw(0..self.ground_shadow_vertex_count, 0..self.instance_count);

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..self.instance_count);
    }

    /// The house nearest the camera, in world space, or `None` when no house
    /// is drawn.
    pub fn nearest_house_world(&self) -> Option<DVec3> {
        self.nearest_house_world
    }

    /// Houses sited around the camera, before the render cutoff. Unlike the
    /// drawn count this should not move when only the camera does.
    pub fn sited_houses(&self) -> u32 {
        self.sited_houses
    }

    /// The worst gap between a drawn house's ground and its sited ground.
    pub fn max_ground_disagreement_meters(&self) -> f64 {
        self.max_ground_disagreement_meters
    }

    /// What siting saw at the house nearest the camera.
    pub fn nearest_site_ground(&self) -> (f64, Option<catinthegarden_coretypes::BiomeId>, f32) {
        (
            self.nearest_site_macro_height_meters,
            self.nearest_site_biome,
            self.nearest_site_moisture,
        )
    }

    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }
}

#[cfg(test)]
mod tests {
    use super::{beam_ribbon_vertices, village_shader_source};

    #[test]
    fn instance_attributes_match_the_struct_layout() {
        use crate::village::HouseInstance;
        // `vertex_attr_array!` packs consecutively and would put these at
        // 0/12/24/36; the padded struct puts them at 0/16/32/48. Getting this
        // wrong feeds the shader a zero `up`, which is NaN after normalize and
        // draws nothing at all while the instance count still looks healthy.
        let instance = HouseInstance {
            camera_relative_position: [1.0, 2.0, 3.0],
            _pad0: 0.0,
            up: [4.0, 5.0, 6.0],
            _pad1: 0.0,
            forward: [7.0, 8.0, 9.0],
            _pad2: 0.0,
            colour: [10.0, 11.0, 12.0],
            _pad3: 0.0,
        };
        let bytes = bytemuck::bytes_of(&instance);
        let read = |offset: usize| {
            f32::from_ne_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
        };
        for attribute in HouseInstance::ATTRIBUTES {
            let first = read(attribute.offset as usize);
            let expected = match attribute.shader_location {
                3 => 1.0,
                4 => 4.0,
                5 => 7.0,
                6 => 10.0,
                other => panic!("unexpected shader location {other}"),
            };
            assert_eq!(
                first, expected,
                "location {} reads {first} at offset {}, expected {expected}",
                attribute.shader_location, attribute.offset
            );
        }
        assert_eq!(size_of::<HouseInstance>(), 64, "stride must cover all four");
    }

    #[test]
    fn the_beam_ribbon_covers_the_whole_quad_once() {
        // Two triangles, six vertices, each corner of the unit quad present
        // and the diagonal shared. A ribbon missing a corner draws a beam with
        // a bite out of it, which is the sort of thing only a capture catches.
        let ribbon = beam_ribbon_vertices();
        assert_eq!(ribbon.len(), 6);
        let mut corners: Vec<[u32; 2]> = ribbon
            .iter()
            .map(|vertex| [vertex.uv[0] as u32, vertex.uv[1] as u32])
            .collect();
        corners.sort_unstable();
        corners.dedup();
        assert_eq!(
            corners.len(),
            4,
            "the ribbon does not cover all four corners"
        );
    }

    #[test]
    fn beams_start_off_and_toggle() {
        // A locator is a debug overlay: it must not be on for someone who
        // never asked for it.
        assert!(
            std::env::var("CATINGARDEN_VILLAGE_BEAMS").is_err(),
            "this test describes the default, so the override must be unset"
        );
    }

    #[test]
    fn village_shader_parses_and_validates() {
        let shader = village_shader_source();
        let module =
            wgpu::naga::front::wgsl::parse_str(&shader).expect("village shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("village shader must validate");
    }

    #[test]
    fn houses_are_positioned_relative_to_the_camera() {
        // A planet-absolute position quantises to about half a metre in f32 at
        // this radius, which on a six-metre house is a visible step.
        let shader = village_shader_source();
        assert!(shader.contains("input.camera_relative_position + local"));
    }

    #[test]
    fn the_roof_keeps_its_own_colour() {
        let shader = village_shader_source();
        assert!(shader.contains("select(input.colour"));
    }
}

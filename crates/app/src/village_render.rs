//! GPU side of the villages: one static house mesh, and an instance buffer
//! rebuilt when the camera has moved far enough for the visible set to change.
//!
//! Split from `village` the way `ship_render` is split from `ship`, so the
//! placement rules stay testable without a device.

use glam::DVec3;

use crate::terrain::TerrainRenderer;
use crate::village::{
    self, HouseInstance, HouseVertex, draw_altitude_meters, max_draw_instances,
    rebuild_distance_meters,
};

pub fn village_shader_source() -> String {
    include_str!("village.wgsl").to_string()
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

        Self {
            pipeline,
            vertex_buffer,
            vertex_count: mesh.len() as u32,
            instance_buffer,
            instance_count: 0,
            last_build_position: None,
            nearest_house_world: None,
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
            self.sited_houses = 0;
            self.max_ground_disagreement_meters = 0.0;
            return;
        }
        let moved_far_enough = self
            .last_build_position
            .is_none_or(|last| last.distance(camera_world_position) > rebuild_distance_meters());
        if !moved_far_enough {
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
        self.nearest_house_world = instances
            .iter()
            .map(|instance| DVec3::from(instance.camera_relative_position.map(f64::from)))
            .min_by(|a, b| {
                a.length_squared()
                    .partial_cmp(&b.length_squared())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|offset| camera_world_position + offset);
        self.last_build_position = Some(camera_world_position);
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
    }

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
        camera_altitude_meters: f64,
    ) {
        if !self.enabled
            || self.instance_count == 0
            || self.vertex_count == 0
            || camera_altitude_meters >= draw_altitude_meters()
        {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
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
    use super::village_shader_source;

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

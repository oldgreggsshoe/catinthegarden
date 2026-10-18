//! Cached local viaduct batches with a distant deck-only LOD. Ground survey
//! and route planning run once; camera updates never rebuild the geometry.

use glam::DVec3;
use wgpu::util::DeviceExt;

use crate::{road, terrain::TerrainRenderer};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    origin: [f32; 4],
    across: [f32; 4],
    forward: [f32; 4],
    up: [f32; 4],
}

struct DrawChunk {
    range: std::ops::Range<u32>,
    far_range: std::ops::Range<u32>,
    structure_range: std::ops::Range<u32>,
    distant: bool,
    centre: DVec3,
    radius: f64,
    visible: bool,
}

/// Adjacent visible chunks of the same LOD share a contiguous vertex range.
/// Coalesce them without allocating or submitting one tiny draw per span.
fn draw_ranges(
    chunks: &[DrawChunk],
    distant: bool,
) -> impl Iterator<Item = std::ops::Range<u32>> + '_ {
    let mut chunks = chunks.iter().peekable();
    let range_of = move |chunk: &DrawChunk| {
        if distant {
            chunk.far_range.clone()
        } else {
            chunk.range.clone()
        }
    };
    std::iter::from_fn(move || {
        let first = chunks.find(|chunk| chunk.visible && chunk.distant == distant)?;
        let mut range = range_of(first);
        while let Some(next) = chunks.peek() {
            if !next.visible || next.distant != distant || range_of(next).start != range.end {
                break;
            }
            range.end = range_of(next).end;
            chunks.next();
        }
        Some(range)
    })
}

pub struct RoadRenderer {
    pub plan: road::Viaduct,
    pipeline: wgpu::RenderPipeline,
    far_pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    instance: wgpu::Buffer,
    chunks: Vec<DrawChunk>,
}

impl RoadRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        terrain: &mut TerrainRenderer,
    ) -> Self {
        let started = std::time::Instant::now();
        let plan = road::Viaduct::survey(|direction| {
            terrain.prepare_flight_start_surface_height_meters(direction, 0.0)
        })
        .expect("viaduct trial needs complete dry terrain at its surveyed site");
        for tunnel in &plan.tunnels {
            tracing::info!(
                entry_meters = plan.route.distances[tunnel.start.saturating_sub(1)],
                exit_meters = plan.route.distances[tunnel.end],
                exit_local_x = plan.route.points[tunnel.end].x,
                exit_local_y = plan.route.points[tunnel.end].y,
                exit_local_z = plan.route.points[tunnel.end].z,
                "road tunnel portals"
            );
        }
        terrain.set_road_cuts(plan.cuts.clone());
        let mut mesh = Vec::new();
        let mut far_mesh = Vec::new();
        let mut structures = Vec::new();
        let mut chunks = Vec::new();
        for chunk in plan.mesh() {
            let start = mesh.len() as u32;
            mesh.extend(chunk.vertices);
            let near_end = mesh.len() as u32;
            let far_start = far_mesh.len() as u32;
            far_mesh.extend(chunk.far_vertices);
            let structure_start = structures.len() as u32;
            structures.extend(chunk.structure_vertices);
            let structure_radius = structures[structure_start as usize..]
                .iter()
                .map(|v| DVec3::from_array(v.position.map(f64::from)).distance(chunk.centre))
                .fold(chunk.radius, f64::max);
            chunks.push(DrawChunk {
                structure_range: structure_start..structures.len() as u32,
                range: start..near_end,
                far_range: far_start..far_mesh.len() as u32,
                distant: false,
                centre: road::world(chunk.centre),
                radius: structure_radius,
                visible: false,
            });
        }
        let far_base = mesh.len() as u32;
        for chunk in &mut chunks {
            chunk.far_range.start += far_base;
            chunk.far_range.end += far_base;
        }
        mesh.extend(far_mesh);
        let structure_base = mesh.len() as u32;
        for chunk in &mut chunks {
            chunk.structure_range.start += structure_base;
            chunk.structure_range.end += structure_base;
        }
        mesh.extend(structures);
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viaduct static vertices"),
            contents: bytemuck::cast_slice(&mesh),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instance = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viaduct camera-relative frame"),
            size: size_of::<Instance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline = create_pipeline(device, format, camera_layout, false);
        let far_pipeline = create_pipeline(device, format, camera_layout, true);
        tracing::info!(
            route_length_meters = plan.route.length(),
            tunnel_count = plan.tunnels.len(),
            start_cover_meters = plan.route.ground[0] - plan.route.points[0].z,
            end_cover_meters =
                plan.route.ground.last().unwrap() - plan.route.points.last().unwrap().z,
            tunnel_length_meters = plan
                .tunnels
                .iter()
                .map(|t| plan.route.distances[t.end] - plan.route.distances[t.start])
                .sum::<f64>(),
            open_cut_only_volume_proxy_m3 = plan.route.cut_volume,
            straight_cut_volume_m3 = plan.route.straight_cut_volume,
            maximum_cut_meters = plan
                .route
                .ground
                .iter()
                .zip(&plan.route.points)
                .map(|(g, p)| (g - p.z + road::CUT_FLOOR_DEPTH).max(0.0))
                .fold(0.0_f64, f64::max),
            lateral_range_meters = plan
                .route
                .points
                .iter()
                .map(|p| p.x.abs())
                .fold(0.0_f64, f64::max),
            support_count = plan.piers.len(),
            maximum_support_meters = plan
                .piers
                .iter()
                .map(|p| p.top - p.local_foot.z)
                .fold(0.0_f64, f64::max),
            triangles = mesh.len() / 3,
            setup_ms = started.elapsed().as_secs_f64() * 1000.0,
            "graded viaduct route built"
        );
        Self {
            plan,
            pipeline,
            far_pipeline,
            vertices,
            instance,
            chunks,
        }
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera_planet_position: DVec3,
        camera_forward: DVec3,
        viewport: [u32; 2],
    ) {
        for chunk in &mut self.chunks {
            let relative = chunk.centre - camera_planet_position;
            chunk.distant = relative.length() > 4000.0;
            chunk.visible = relative.length() < 25000.0 + chunk.radius
                && relative.dot(camera_forward) > -chunk.radius;
        }
        if self.chunks.iter().any(|chunk| chunk.visible) {
            let vector = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32, 0.0];
            let mut frame = Instance {
                origin: vector(road::origin() - camera_planet_position),
                across: vector(road::across()),
                forward: vector(road::forward()),
                up: vector(road::up()),
            };
            frame.origin[3] = viewport[0] as f32;
            frame.across[3] = viewport[1] as f32;
            queue.write_buffer(&self.instance, 0, bytemuck::bytes_of(&frame));
        }
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: &'a wgpu::BindGroup) {
        self.draw_lod(pass, camera, false);
        // Lining/portals remain opaque at the far LOD as well. Otherwise the
        // clipped portal would expose sky when the deck switches to a ribbon.
        if self
            .chunks
            .iter()
            .any(|c| c.visible && !c.structure_range.is_empty())
        {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, camera, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instance.slice(..));
            for chunk in self
                .chunks
                .iter()
                .filter(|c| c.visible && !c.structure_range.is_empty())
            {
                pass.draw(chunk.structure_range.clone(), 0..1);
            }
        }
    }

    /// Coverage-blended decks draw after opaque objects, before local weather.
    pub fn draw_far<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: &'a wgpu::BindGroup) {
        self.draw_lod(pass, camera, true);
    }

    fn draw_lod<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera: &'a wgpu::BindGroup,
        distant: bool,
    ) {
        if self
            .chunks
            .iter()
            .any(|chunk| chunk.visible && chunk.distant == distant)
        {
            pass.set_pipeline(if distant {
                &self.far_pipeline
            } else {
                &self.pipeline
            });
            pass.set_bind_group(0, camera, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instance.slice(..));
            for range in draw_ranges(&self.chunks, distant) {
                pass.draw(range, 0..1);
            }
        }
    }
}

pub(crate) fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera_layout: &wgpu::BindGroupLayout,
    far: bool,
) -> wgpu::RenderPipeline {
    create_pipeline_inner(device, format, camera_layout, far, None)
}

pub(crate) fn create_network_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera_layout: &wgpu::BindGroupLayout,
    shared: &wgpu::BindGroupLayout,
    far: bool,
) -> wgpu::RenderPipeline {
    create_pipeline_inner(device, format, camera_layout, far, Some(shared))
}
fn create_pipeline_inner(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera_layout: &wgpu::BindGroupLayout,
    far: bool,
    shared: Option<&wgpu::BindGroupLayout>,
) -> wgpu::RenderPipeline {
    let source = if shared.is_some() {
        let clip = r#"
            // Close the union, not each individual tube at its junction.
            if ((input.network_kind > 0.5 && input.network_kind < 1.5)
                || (input.network_kind > 2.5 && input.network_kind < 3.5))
                && road_network_void_local(input.local_position) > 0.55 { discard; }
            if input.network_kind > 1.5 && input.network_kind < 2.5 {
                if input.removed < 0.25 { discard; }
                let p = input.local_position;
                let r = road_cut_data[0].x + p.z;
                let h = p.z + dot(p.xy,p.xy) / (sqrt(r*r + dot(p.xy,p.xy)) + r);
                let d = normalize(road_cut_data[3].xyz*r
                    + road_cut_data[1].xyz*p.x + road_cut_data[2].xyz*p.y);
                if road_terrain_cut(d,h).height < h - 0.75 { discard; }
            }
        "#;
        format!(
            "{}\n{}",
            crate::road::cut_shader_source(),
            include_str!("road.wgsl").replace("/* NETWORK_EXCAVATION_CLIP */", clip)
        )
    } else {
        include_str!("road.wgsl").to_owned()
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cached road shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("road unused group"),
        entries: &[],
    });
    let layouts = if let Some(shared) = shared {
        vec![Some(camera_layout), Some(&empty), Some(shared)]
    } else {
        vec![Some(camera_layout)]
    };
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viaduct pipeline layout"),
        bind_group_layouts: &layouts,
        immediate_size: 0,
    });
    const VERTEX: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
    const FRAME: [wgpu::VertexAttribute; 4] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 0,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
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
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("viaduct single batch"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(if far { "vs_far" } else { "vs_main" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: size_of::<road::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX,
                },
                wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &FRAME,
                },
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(if far { "fs_far" } else { "fs_main" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: far.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(!far),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: crate::pipeline_cache::get(),
    });
    pipeline
}

#[cfg(test)]
mod tests {
    #[test]
    fn merged_ranges_do_not_draw_hidden_or_wrong_lod_chunks() {
        let chunks = [
            (true, false),
            (true, false),
            (false, false),
            (true, true),
            (true, true),
            (true, false),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (visible, distant))| super::DrawChunk {
            structure_range: 0..0,
            range: (i as u32) * 6..(i as u32 + 1) * 6,
            far_range: 100 + (i as u32) * 6..100 + (i as u32 + 1) * 6,
            centre: glam::DVec3::ZERO,
            radius: 1.0,
            visible,
            distant,
        })
        .collect::<Vec<_>>();
        assert_eq!(
            super::draw_ranges(&chunks, false).collect::<Vec<_>>(),
            [0..12, 30..36]
        );
        assert_eq!(
            super::draw_ranges(&chunks, true).collect::<Vec<_>>(),
            [118..130]
        );
    }

    #[test]
    fn road_shader_and_camera_relative_layout_validate() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("road.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let cuts = wgpu::naga::front::wgsl::parse_str(&crate::road::cut_shader_source()).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&cuts)
        .unwrap();
        assert_eq!(size_of::<super::Instance>(), 64);
        assert_eq!(std::mem::offset_of!(super::Instance, up), 48);
        assert_eq!(size_of::<crate::road::Vertex>(), 36);
    }
}

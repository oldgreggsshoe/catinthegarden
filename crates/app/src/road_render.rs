//! One cached, locally culled draw for the opt-in viaduct. No terrain shader
//! evaluation or per-frame ground survey is needed after construction.

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

pub struct RoadRenderer {
    pub plan: road::Viaduct,
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    instance: wgpu::Buffer,
    vertex_count: u32,
    visible: bool,
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
        let mesh = plan.mesh();
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viaduct shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("road.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viaduct pipeline layout"),
            bind_group_layouts: &[Some(camera_layout)],
            immediate_size: 0,
        });
        const VERTEX: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
        const FRAME: [wgpu::VertexAttribute; 4] = [
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viaduct single batch"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
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
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
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
            cache: None,
        });
        tracing::info!(
            deck_height_meters = plan.deck_height,
            support_count = plan.piers.len(),
            maximum_support_meters = plan
                .piers
                .iter()
                .map(|p| plan.deck_height - 1.0 - p.local_foot.z)
                .fold(0.0_f64, f64::max),
            triangles = mesh.len() / 3,
            setup_ms = started.elapsed().as_secs_f64() * 1000.0,
            "level viaduct built"
        );
        Self {
            plan,
            pipeline,
            vertices,
            instance,
            vertex_count: mesh.len() as u32,
            visible: false,
        }
    }

    pub fn update(&mut self, queue: &wgpu::Queue, camera_planet_position: DVec3) {
        let centre = road::world(DVec3::new(
            road::centre_x(325.0),
            325.0,
            self.plan.deck_height,
        ));
        self.visible = camera_planet_position.distance(centre) < 5_000.0;
        if self.visible {
            let vector = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32, 0.0];
            let frame = Instance {
                origin: vector(road::origin() - camera_planet_position),
                across: vector(road::across()),
                forward: vector(road::forward()),
                up: vector(road::up()),
            };
            queue.write_buffer(&self.instance, 0, bytemuck::bytes_of(&frame));
        }
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: &'a wgpu::BindGroup) {
        if self.visible {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, camera, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instance.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn road_shader_and_camera_relative_layout_validate() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("road.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<super::Instance>(), 64);
        assert_eq!(std::mem::offset_of!(super::Instance, up), 48);
        assert_eq!(size_of::<crate::road::Vertex>(), 36);
    }
}

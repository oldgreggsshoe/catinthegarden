//! GPU side of the ship: one static vertex buffer for the hull, and a uniform
//! carrying the hull's pose relative to the camera.
//!
//! Split from `ship` the way `weather_render` is split from `weather`, so the
//! float itself stays testable without a device.

use glam::{DMat3, DVec3};

use crate::ship::{self, ShipVertex};

pub fn ship_shader_source() -> String {
    include_str!("ship.wgsl").to_string()
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShipUniform {
    view_position: [f32; 4],
    orientation_x: [f32; 4],
    orientation_y: [f32; 4],
    orientation_z: [f32; 4],
    up: [f32; 4],
}

impl ShipVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct ShipRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    visible: bool,
}

impl ShipRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let mesh = ship::build_mesh();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ship hull vertices"),
            size: (mesh.len() * size_of::<ShipVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&mesh));

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ship uniform"),
            size: size_of::<ShipUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ship bind group layout"),
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
            label: Some("ship bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ship pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ship shader"),
            source: wgpu::ShaderSource::Wgsl(ship_shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ship pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[ShipVertex::layout()],
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
            uniform_buffer,
            bind_group,
            visible: false,
        }
    }

    pub fn triangle_count(&self) -> u32 {
        self.vertex_count / 3
    }

    /// `hull_origin_view_position` is the hull's waterline origin relative to
    /// the camera, rotated into view axes. Both differences are taken in f64 by
    /// the caller before they narrow to f32.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        hull_origin_view_position: DVec3,
        orientation: DMat3,
        up: DVec3,
        visible: bool,
    ) {
        self.visible = visible;
        if !visible {
            return;
        }
        let uniform = ShipUniform {
            view_position: hull_origin_view_position.as_vec3().extend(0.0).to_array(),
            orientation_x: orientation.x_axis.as_vec3().extend(0.0).to_array(),
            orientation_y: orientation.y_axis.as_vec3().extend(0.0).to_array(),
            orientation_z: orientation.z_axis.as_vec3().extend(0.0).to_array(),
            up: up.as_vec3().extend(0.0).to_array(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>, camera_bind_group: &wgpu::BindGroup) {
        if !self.visible || self.vertex_count == 0 {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::ship_shader_source;

    #[test]
    fn ship_shader_parses_and_renders_camera_relative_flat_facets() {
        let shader = ship_shader_source();
        let module =
            wgpu::naga::front::wgsl::parse_str(&shader).expect("ship shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("ship shader must validate");
        // The hull is positioned relative to the camera, never in planet
        // coordinates: an f32 planet position quantises to half a metre here.
        assert!(shader.contains("ship.view_position.xyz + planet_to_view(planet_offset)"));
        // Facets, not smooth shading.
        assert!(shader.contains("@location(0) @interpolate(flat) normal: vec3<f32>"));
        assert!(shader.contains("@location(1) @interpolate(flat) colour: vec3<f32>"));
    }
}

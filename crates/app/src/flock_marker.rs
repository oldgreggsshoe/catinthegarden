//! The on-screen reticle over the nearest flock: a filled square with a tick
//! out to each side and one above and below.
//!
//! Split from `birds` the way `ship_render` is split from `ship`. The shape
//! itself is built here in pixels and placed by the shader, so it stays the
//! same size on screen however far away the flock is.

use glam::DVec3;

pub fn flock_marker_shader_source() -> String {
    include_str!("flock_marker.wgsl").to_string()
}

/// Half-width of the centre square, in pixels.
const SQUARE_HALF_PIXELS: f32 = 5.0;
/// Where each tick starts and stops, measured from the marked point. The gap
/// between the square and the ticks is what makes the shape read as a reticle
/// rather than a plus sign.
const TICK_INNER_PIXELS: f32 = 9.0;
const TICK_OUTER_PIXELS: f32 = 18.0;
const TICK_HALF_PIXELS: f32 = 1.5;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerVertex {
    offset_pixels: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerUniform {
    view_position: [f32; 4],
    viewport: [f32; 4],
}

impl MarkerVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

fn quad(left: f32, top: f32, right: f32, bottom: f32, out: &mut Vec<MarkerVertex>) {
    let corner = |x: f32, y: f32| MarkerVertex {
        offset_pixels: [x, y],
    };
    out.extend_from_slice(&[
        corner(left, top),
        corner(right, top),
        corner(right, bottom),
        corner(left, top),
        corner(right, bottom),
        corner(left, bottom),
    ]);
}

/// The reticle: a centre square, then a tick to the left and right and one
/// above and below. Pixel offsets from the marked point, y counting downward.
fn build_marker() -> Vec<MarkerVertex> {
    let mut out = Vec::new();
    let half = SQUARE_HALF_PIXELS;
    quad(-half, -half, half, half, &mut out);
    // Side ticks: the two dashes.
    quad(
        -TICK_OUTER_PIXELS,
        -TICK_HALF_PIXELS,
        -TICK_INNER_PIXELS,
        TICK_HALF_PIXELS,
        &mut out,
    );
    quad(
        TICK_INNER_PIXELS,
        -TICK_HALF_PIXELS,
        TICK_OUTER_PIXELS,
        TICK_HALF_PIXELS,
        &mut out,
    );
    // Above and below: the two bars.
    quad(
        -TICK_HALF_PIXELS,
        -TICK_OUTER_PIXELS,
        TICK_HALF_PIXELS,
        -TICK_INNER_PIXELS,
        &mut out,
    );
    quad(
        -TICK_HALF_PIXELS,
        TICK_INNER_PIXELS,
        TICK_HALF_PIXELS,
        TICK_OUTER_PIXELS,
        &mut out,
    );
    out
}

pub struct FlockMarkerRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    visible: bool,
    last_view_z: f32,
}

impl FlockMarkerRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let mesh = build_marker();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flock marker vertices"),
            size: (mesh.len() * size_of::<MarkerVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&mesh));

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flock marker uniform"),
            size: size_of::<MarkerUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flock marker bind group layout"),
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
            label: Some("flock marker bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flock marker pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flock marker shader"),
            source: wgpu::ShaderSource::Wgsl(flock_marker_shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flock marker pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[MarkerVertex::layout()],
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
                // Screen-space geometry with no meaningful facing.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // The whole point: visible through terrain, and writing no
                // depth of its own so it occludes nothing behind it.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
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
            last_view_z: 0.0,
        }
    }

    /// `centre_view_position` is the flock's centre relative to the camera,
    /// rotated into view axes. The caller takes that difference in f64, as the
    /// ship's hull origin does.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        centre_view_position: Option<DVec3>,
        viewport: [u32; 2],
    ) {
        self.visible = centre_view_position.is_some();
        let position = centre_view_position.unwrap_or(DVec3::ZERO);
        self.last_view_z = position.z as f32;
        let uniform = MarkerUniform {
            view_position: position.as_vec3().extend(0.0).to_array(),
            viewport: [
                viewport[0].max(1) as f32,
                viewport[1].max(1) as f32,
                f32::from(u8::from(self.visible)),
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Where the reticle landed, for the frame log: view-space z (negative is in
    /// front of the camera) and whether there was anything to mark at all.
    pub fn debug_state(&self) -> (bool, f32) {
        (self.visible, self.last_view_z)
    }

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
    ) {
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
    use super::*;

    #[test]
    fn the_marker_shader_parses_and_validates() {
        let shader = flock_marker_shader_source();
        let module =
            wgpu::naga::front::wgsl::parse_str(&shader).expect("flock marker shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("flock marker shader must validate");
    }

    #[test]
    fn the_marker_is_a_square_with_a_tick_on_each_side() {
        let mesh = build_marker();
        // Five quads: the square and four ticks.
        assert_eq!(mesh.len(), 5 * 6);

        let has = |predicate: &dyn Fn(f32, f32) -> bool| {
            mesh.iter()
                .any(|v| predicate(v.offset_pixels[0], v.offset_pixels[1]))
        };
        // A tick out each side, and one above and below, all beyond the square.
        assert!(has(&|x, _| x <= -TICK_OUTER_PIXELS), "no tick to the left");
        assert!(has(&|x, _| x >= TICK_OUTER_PIXELS), "no tick to the right");
        assert!(has(&|_, y| y <= -TICK_OUTER_PIXELS), "no tick above");
        assert!(has(&|_, y| y >= TICK_OUTER_PIXELS), "no tick below");

        // The square is centred and separate: nothing occupies the gap between
        // it and the ticks, which is what stops the shape reading as a plus.
        let gap = mesh.iter().any(|v| {
            let [x, y] = v.offset_pixels;
            let outside_square = x.abs() > SQUARE_HALF_PIXELS || y.abs() > SQUARE_HALF_PIXELS;
            let inside_ticks = x.abs() < TICK_INNER_PIXELS && y.abs() < TICK_INNER_PIXELS;
            outside_square && inside_ticks
        });
        assert!(
            !gap,
            "a vertex sits in the gap between the square and a tick"
        );

        // Nothing is further out than the ticks reach.
        for vertex in &mesh {
            let [x, y] = vertex.offset_pixels;
            assert!(x.abs() <= TICK_OUTER_PIXELS && y.abs() <= TICK_OUTER_PIXELS);
        }
    }

    #[test]
    fn the_marker_never_depth_tests_or_writes() {
        // The reticle exists to be visible through terrain. Both of these are
        // easy to "tidy" into the defaults every other pass uses, which would
        // silently hide it behind the first dune.
        //
        // Scoped to the production half of the file: asserting against the
        // whole of it would match these very strings and pass regardless.
        let production = include_str!("flock_marker.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("the file has a production half");
        assert!(production.contains("depth_write_enabled: Some(false)"));
        assert!(production.contains("depth_compare: Some(wgpu::CompareFunction::Always)"));
    }
}

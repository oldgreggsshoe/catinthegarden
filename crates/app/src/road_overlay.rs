//! Cached, constant-pixel debug lines; no road generation or layout per frame.
use wgpu::util::DeviceExt;

/// Successful regional plans and their retained gateway links, independent of residency.
pub type Generated =
    std::collections::HashMap<planet_coretypes::TileKey, std::collections::HashSet<usize>>;
pub fn record(generated: &mut Generated, region: &crate::road_network::Region) {
    generated.insert(
        region.key,
        region
            .terminals
            .iter()
            .filter_map(|t| match t.kind {
                crate::road_network::TerminalKind::Gateway { link } => Some(link),
                _ => None,
            })
            .collect(),
    );
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Line {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub generated: f32,
}
/// Schematic global trunks, split at shared gateways so each half has the
/// generation state of its own region. Exact engineered curves replace this
/// overview at ground scale. Built on the road worker, never in a movement frame.
pub fn overview(
    backbone: &crate::road_network::Backbone,
    generated: &Generated,
    radius: f64,
) -> std::sync::Arc<Vec<Line>> {
    let point = |d: glam::DVec3| (d * radius - glam::DVec3::Z * radius).as_vec3().to_array();
    std::sync::Arc::new(
        backbone
            .links
            .iter()
            .enumerate()
            // Exact occupancy can prune a coarse false-positive branch. Do
            // not advertise a green road (or a dangling red half) there.
            .filter(|(id, link)| {
                [link.a, link.b].iter().all(|&index| {
                    generated
                        .get(&crate::road_network::key(index, backbone.level))
                        .is_none_or(|links| links.contains(id))
                })
            })
            .flat_map(|(_, link)| {
                [link.a, link.b].map(|index| {
                    let key = crate::road_network::key(index, backbone.level);
                    Line {
                        a: point(crate::road_network::direction(key, 0.5, 0.5)),
                        b: point(glam::DVec3::from_array(link.gateway)),
                        generated: if generated.contains_key(&key) {
                            1.0
                        } else {
                            0.0
                        },
                    }
                })
            })
            .collect(),
    )
}
fn shader_source() -> String {
    include_str!("road_overlay.wgsl").replace(
        "__PLANET_RADIUS__",
        &format!("{:.1}", crate::planet::planet_radius_meters()),
    )
}
struct Lines {
    vertices: wgpu::Buffer,
    count: u32,
}
pub struct Renderer {
    pub enabled: bool,
    pipeline: wgpu::RenderPipeline,
    frame: wgpu::BindGroup,
    resident: Option<Lines>,
    generation: Option<u64>,
    overview: Option<Lines>,
    overview_source: Option<std::sync::Arc<Vec<Line>>>,
    overview_frame: wgpu::Buffer,
    overview_group: wgpu::BindGroup,
    orbital: bool,
}
impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
        frame: &wgpu::Buffer,
    ) -> Self {
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("road line frame layout"),
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
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("road line frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            }],
        });
        let overview_frame = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("planetary road overview frame"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let overview_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("planetary road overview group"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: overview_frame.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("road lines pipeline layout"),
            bind_group_layouts: &[Some(camera), Some(&frame_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("road overlay shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("road centre line overlay"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<Line>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(),
        });
        Self {
            enabled: std::env::var("PLANET_ROAD_LINES")
                .is_ok_and(|v| matches!(v.trim(), "1" | "true" | "on")),
            pipeline,
            frame: group,
            resident: None,
            generation: None,
            overview: None,
            overview_source: None,
            overview_frame,
            overview_group,
            orbital: false,
        }
    }
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        generation: u64,
        lines: &[Line],
        overview: &std::sync::Arc<Vec<Line>>,
    ) {
        if !self.enabled || self.generation == Some(generation) {
            return;
        }
        self.resident = (!lines.is_empty()).then(|| Lines {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("exact resident road traces"),
                contents: bytemuck::cast_slice(lines),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            count: lines.len() as u32,
        });
        if !self
            .overview_source
            .as_ref()
            .is_some_and(|old| std::sync::Arc::ptr_eq(old, overview))
        {
            self.overview = (!overview.is_empty()).then(|| Lines {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("planned and generated planetary road trunks"),
                    contents: bytemuck::cast_slice(overview),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                count: overview.len() as u32,
            });
            self.overview_source = Some(overview.clone());
        }
        self.generation = Some(generation);
    }
    pub fn update_camera(&mut self, queue: &wgpu::Queue, eye: glam::DVec3, viewport: [u32; 2]) {
        if !self.enabled {
            return;
        }
        let radius = crate::planet::planet_radius_meters();
        self.orbital = eye.length() - radius >= 50_000.0;
        if !self.orbital {
            return;
        }
        let origin = glam::DVec3::Z * radius - eye;
        let frame = [
            [
                origin.x as f32,
                origin.y as f32,
                origin.z as f32,
                viewport[0] as f32,
            ],
            [1.0, 0.0, 0.0, viewport[1] as f32],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 1.0],
        ];
        queue.write_buffer(&self.overview_frame, 0, bytemuck::cast_slice(&frame));
    }
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: &'a wgpu::BindGroup) {
        if !self.enabled {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        if self.orbital {
            if let Some(lines) = &self.overview {
                pass.set_bind_group(1, &self.overview_group, &[]);
                pass.set_vertex_buffer(0, lines.vertices.slice(..));
                pass.draw(0..6, 0..lines.count);
            }
        }
        if let Some(lines) = &self.resident {
            pass.set_bind_group(1, &self.frame, &[]);
            pass.set_vertex_buffer(0, lines.vertices.slice(..));
            pass.draw(0..6, 0..lines.count);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn road_overlay_marks_only_successfully_generated_region_halves_green() {
        use crate::road_network::{Backbone, Ground, Survey};
        struct Flat;
        impl Survey for Flat {
            fn ground(&self, _: glam::DVec3) -> Result<Ground, String> {
                Ok(Ground {
                    height: 900.0,
                    ocean: false,
                })
            }
            fn villages(&self, _: planet_coretypes::TileKey) -> Result<Vec<glam::DVec3>, String> {
                Ok(Vec::new())
            }
        }
        let backbone = Backbone::survey(1, 4e6, &Flat).unwrap();
        let mut generated = Generated::new();
        let before = overview(&backbone, &generated, 4e6);
        assert_eq!(before.len(), backbone.links.len() * 2);
        assert!(!before.is_empty());
        assert!(before.iter().all(|l| l.generated == 0.0));
        let key = crate::road_network::key(backbone.links[0].a, backbone.level);
        generated.insert(key, (0..backbone.links.len()).collect());
        let after = overview(&backbone, &generated, 4e6);
        for (pair, link) in after.chunks_exact(2).zip(&backbone.links) {
            assert_eq!(pair[0].b, pair[1].b, "shared gateway must not crack");
            for (line, index) in pair.iter().zip([link.a, link.b]) {
                assert_eq!(
                    line.generated == 1.0,
                    crate::road_network::key(index, backbone.level) == key
                );
                for p in [line.a, line.b] {
                    let world = glam::DVec3::from_array(p.map(f64::from)) + glam::DVec3::Z * 4e6;
                    assert!((world.length() - 4e6).abs() < 0.5);
                }
            }
        }
        assert!(after.iter().any(|l| l.generated == 1.0));
        assert!(after.iter().any(|l| l.generated == 0.0));
        generated.get_mut(&key).unwrap().clear();
        let pruned = overview(&backbone, &generated, 4e6);
        assert!(pruned.len() < after.len());
        assert!(pruned.iter().all(|l| l.generated == 0.0));
    }
    #[test]
    fn road_overlay_shader_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(&shader_source()).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<Line>(), 28);
    }
}

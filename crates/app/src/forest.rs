use std::mem::size_of;

use glam::DVec3;
use wgpu::util::DeviceExt;

use crate::{planet::PLANET_RADIUS_METERS, terrain::TerrainRenderer};

pub const FOREST_CENTRE_DIRECTION: DVec3 =
    DVec3::new(0.374_871_986_443, 0.737_334_908_710, 0.561_968_171_854);
pub const FOREST_START_PITCH_RADIANS: f64 = 4.0_f64.to_radians();

const TREE_COUNT: usize = 12_288;
const FOREST_RADIUS_METERS: f64 = 800.0;
const CLEARING_RADIUS_METERS: f64 = 14.0;
const TREE_BASE_SINK_METERS: f64 = 0.45;
const TREE_HEIGHT_MIN_METERS: f32 = 11.0;
const TREE_HEIGHT_RANGE_METERS: f32 = 13.0;
const FOREST_DRAW_ALTITUDE_METERS: f64 = 50_000.0;
const GOLDEN_ANGLE_RADIANS: f64 = 2.399_963_229_728_653;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TreeInstance {
    centre_and_height: [f32; 4],
    width_shade_kind_seed: [f32; 4],
}

impl TreeInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ForestUniform {
    camera_planet_position: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct TreeLayout {
    tangent_offset_meters: [f64; 2],
    height_meters: f32,
    width_meters: f32,
    shade: f32,
    kind: f32,
    seed: f32,
}

pub struct ForestRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
}

impl ForestRenderer {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        terrain: &mut TerrainRenderer,
    ) -> Self {
        let instances = grounded_tree_instances(terrain);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("forest camera uniform"),
            size: size_of::<ForestUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forest bind group layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forest bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forest pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forest billboard shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("forest.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("forest billboard pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[TreeInstance::layout()],
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
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest tree instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            instance_count: instances.len() as u32,
        }
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, camera_planet_position: DVec3) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&ForestUniform {
                camera_planet_position: [
                    camera_planet_position.x as f32,
                    camera_planet_position.y as f32,
                    camera_planet_position.z as f32,
                    0.0,
                ],
            }),
        );
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        camera_altitude_meters: f64,
    ) {
        if camera_altitude_meters >= FOREST_DRAW_ALTITUDE_METERS || self.instance_count == 0 {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        render_pass.draw(0..6, 0..self.instance_count);
    }
}

fn grounded_tree_instances(terrain: &mut TerrainRenderer) -> Vec<TreeInstance> {
    let centre = FOREST_CENTRE_DIRECTION.normalize();
    terrain.prepare_flight_start_surface_height_meters(centre, 2.0);
    let tangent_x = centre.cross(DVec3::Y).normalize();
    let tangent_y = tangent_x.cross(centre).normalize();
    tree_layouts()
        .into_iter()
        .filter_map(|tree| {
            let direction = (centre
                + tangent_x * (tree.tangent_offset_meters[0] / PLANET_RADIUS_METERS)
                + tangent_y * (tree.tangent_offset_meters[1] / PLANET_RADIUS_METERS))
                .normalize();
            let surface_height = terrain.surface_height_meters_at(direction, 2.0)?;
            (surface_height > 0.0).then(|| {
                let centre =
                    direction * (PLANET_RADIUS_METERS + surface_height - TREE_BASE_SINK_METERS);
                TreeInstance {
                    centre_and_height: [
                        centre.x as f32,
                        centre.y as f32,
                        centre.z as f32,
                        tree.height_meters,
                    ],
                    width_shade_kind_seed: [tree.width_meters, tree.shade, tree.kind, tree.seed],
                }
            })
        })
        .collect()
}

fn tree_layouts() -> Vec<TreeLayout> {
    (0..TREE_COUNT)
        .map(|index| {
            let radial_fraction = (index as f64 + 0.5) / TREE_COUNT as f64;
            let radius = (CLEARING_RADIUS_METERS * CLEARING_RADIUS_METERS
                + radial_fraction
                    * (FOREST_RADIUS_METERS * FOREST_RADIUS_METERS
                        - CLEARING_RADIUS_METERS * CLEARING_RADIUS_METERS))
                .sqrt();
            let angle = index as f64 * GOLDEN_ANGLE_RADIANS
                + (f64::from(hash(index as u32 ^ 0x68bc_21eb)) - 0.5) * 0.55;
            let height = TREE_HEIGHT_MIN_METERS
                + hash(index as u32 ^ 0xa511_e9b3) * TREE_HEIGHT_RANGE_METERS;
            let width = height * (0.32 + hash(index as u32 ^ 0x63d8_3595) * 0.18);
            TreeLayout {
                tangent_offset_meters: [radius * angle.cos(), radius * angle.sin()],
                height_meters: height,
                width_meters: width,
                shade: 0.82 + hash(index as u32 ^ 0x9e37_79b9) * 0.34,
                kind: if hash(index as u32 ^ 0x27d4_eb2f) < 0.28 {
                    1.0
                } else {
                    0.0
                },
                seed: hash(index as u32 ^ 0x1656_67b1),
            }
        })
        .collect()
}

fn hash(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forest_layout_is_dense_bounded_and_leaves_a_start_clearing() {
        let trees = tree_layouts();
        assert_eq!(trees.len(), TREE_COUNT);
        let radii = trees.iter().map(|tree| {
            DVec3::new(
                tree.tangent_offset_meters[0],
                tree.tangent_offset_meters[1],
                0.0,
            )
            .length()
        });
        let minimum = radii.clone().reduce(f64::min).expect("trees");
        let maximum = radii.reduce(f64::max).expect("trees");
        assert!(minimum >= CLEARING_RADIUS_METERS);
        assert!(maximum <= FOREST_RADIUS_METERS);
        assert!(trees.iter().all(|tree| tree.height_meters >= 11.0));
        assert!(trees.iter().any(|tree| tree.kind == 0.0));
        assert!(trees.iter().any(|tree| tree.kind == 1.0));
    }

    #[test]
    fn forest_shader_is_a_depth_writing_procedural_billboard() {
        let shader = include_str!("forest.wgsl");
        wgpu::naga::front::wgsl::parse_str(shader).expect("forest shader parses");
        assert!(shader.contains("centre_and_height: vec4<f32>"));
        assert!(shader.contains("if !trunk && !canopy"));
        assert!(!shader.contains("textureSample"));
    }

    #[test]
    fn authored_forest_centre_is_normalized() {
        assert!((FOREST_CENTRE_DIRECTION.length() - 1.0).abs() < 1.0e-9);
        assert!(FOREST_CENTRE_DIRECTION.y > 0.6);
    }
}

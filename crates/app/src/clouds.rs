use std::mem::size_of;

use glam::Vec3;
use wgpu::util::DeviceExt;

const LOWER_CLUSTER_COUNT: u32 = 42;
const UPPER_CLUSTER_COUNT: u32 = 24;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CloudVertex {
    position: [f32; 3],
    face_normal: [f32; 3],
}

impl CloudVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CloudInstance {
    center_speed: [f32; 4],
    wind_axis: [f32; 4],
    radii_brightness: [f32; 4],
}

impl CloudInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct CloudRenderer {
    pipeline: wgpu::RenderPipeline,
    atmosphere_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    vertex_count: u32,
    instance_count: u32,
}

impl CloudRenderer {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        atmosphere: crate::atmosphere::SurfaceLightingResources<'_>,
    ) -> Self {
        let atmosphere_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("solid cloud atmosphere lighting layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let atmosphere_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("solid cloud atmosphere lighting bind group"),
            layout: &atmosphere_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atmosphere.transmittance),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atmosphere.irradiance),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(atmosphere.physical_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("solid cloud pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&atmosphere_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("clouds.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("solid low-poly cloud pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[CloudVertex::layout(), CloudInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
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

        let vertices = low_poly_sphere_vertices();
        let instances = generate_cloud_instances();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("solid low-poly cloud vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("solid low-poly cloud instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            pipeline,
            atmosphere_bind_group,
            vertex_buffer,
            instance_buffer,
            vertex_count: vertices.len() as u32,
            instance_count: instances.len() as u32,
        }
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.atmosphere_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..self.instance_count);
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn low_poly_sphere_vertices() -> Vec<CloudVertex> {
    let phi = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let points = [
        Vec3::new(-1.0, phi, 0.0),
        Vec3::new(1.0, phi, 0.0),
        Vec3::new(-1.0, -phi, 0.0),
        Vec3::new(1.0, -phi, 0.0),
        Vec3::new(0.0, -1.0, phi),
        Vec3::new(0.0, 1.0, phi),
        Vec3::new(0.0, -1.0, -phi),
        Vec3::new(0.0, 1.0, -phi),
        Vec3::new(phi, 0.0, -1.0),
        Vec3::new(phi, 0.0, 1.0),
        Vec3::new(-phi, 0.0, -1.0),
        Vec3::new(-phi, 0.0, 1.0),
    ]
    .map(Vec3::normalize);
    let faces = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];

    let mut vertices = Vec::with_capacity(faces.len() * 3);
    for face in faces {
        let mut triangle = [points[face[0]], points[face[1]], points[face[2]]];
        let mut normal = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize();
        if normal.dot(triangle[0] + triangle[1] + triangle[2]) < 0.0 {
            triangle.swap(1, 2);
            normal = -normal;
        }
        for position in triangle {
            vertices.push(CloudVertex {
                position: position.to_array(),
                face_normal: normal.to_array(),
            });
        }
    }
    vertices
}

fn generate_cloud_instances() -> Vec<CloudInstance> {
    let mut instances = Vec::new();
    append_cloud_layer(&mut instances, 0, LOWER_CLUSTER_COUNT, false);
    append_cloud_layer(
        &mut instances,
        LOWER_CLUSTER_COUNT,
        UPPER_CLUSTER_COUNT,
        true,
    );
    instances
}

fn append_cloud_layer(
    instances: &mut Vec<CloudInstance>,
    first_cluster: u32,
    cluster_count: u32,
    upper: bool,
) {
    const PLANET_RADIUS_METERS: f32 = 4_000_000.0;
    for local_cluster in 0..cluster_count {
        let cluster = first_cluster + local_cluster;
        let direction = if cluster == 0 {
            // Keep one lower system in the deterministic surface-lighting
            // scenarios' forward sky so daylight/sunset/night captures
            // continuously exercise real cloud geometry and illumination.
            Vec3::new(1.0, 0.0, 0.30).normalize()
        } else {
            let latitude = (hash01(cluster, 0) * 2.0 - 1.0) * 0.94;
            let longitude = std::f32::consts::TAU * hash01(cluster, 1);
            let horizontal = (1.0 - latitude * latitude).sqrt();
            Vec3::new(
                horizontal * longitude.cos(),
                latitude,
                horizontal * longitude.sin(),
            )
        };
        let reference = if direction.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let tangent_a = reference.cross(direction).normalize();
        let tangent_b = direction.cross(tangent_a).normalize();
        let wind_angle = std::f32::consts::TAU * hash01(cluster, 2);
        let wind_direction = tangent_a * wind_angle.cos() + tangent_b * wind_angle.sin();
        let wind_axis = direction.cross(wind_direction).normalize();
        let wind_speed_meters_per_second = if upper {
            55.0 + 55.0 * hash01(cluster, 3)
        } else {
            24.0 + 36.0 * hash01(cluster, 3)
        };
        let lobe_count = if upper {
            4 + (hash_u32(cluster, 4) % 4)
        } else {
            6 + (hash_u32(cluster, 4) % 4)
        };

        for lobe in 0..lobe_count {
            let key = cluster.wrapping_mul(11).wrapping_add(lobe);
            let offset_angle = std::f32::consts::TAU * hash01(key, 5);
            let offset_radius = if lobe == 0 {
                0.0
            } else if upper {
                30_000.0 + 150_000.0 * hash01(key, 6).sqrt()
            } else {
                20_000.0 + 105_000.0 * hash01(key, 6).sqrt()
            };
            let offset = tangent_a * (offset_angle.cos() * offset_radius)
                + tangent_b * (offset_angle.sin() * offset_radius);
            let lobe_direction = (direction + offset / PLANET_RADIUS_METERS).normalize();
            let altitude = if upper {
                220_000.0 + 50_000.0 * hash01(key, 7)
            } else {
                110_000.0 + 40_000.0 * hash01(key, 7)
            };
            let center = lobe_direction * (PLANET_RADIUS_METERS + altitude);
            let central_scale = if lobe == 0 { 1.22 } else { 1.0 };
            let radii = if upper {
                Vec3::new(
                    (100_000.0 + 130_000.0 * hash01(key, 8)) * central_scale,
                    (55_000.0 + 75_000.0 * hash01(key, 9)) * central_scale,
                    (25_000.0 + 20_000.0 * hash01(key, 10)) * central_scale,
                )
            } else {
                Vec3::new(
                    (70_000.0 + 115_000.0 * hash01(key, 8)) * central_scale,
                    (58_000.0 + 105_000.0 * hash01(key, 9)) * central_scale,
                    (45_000.0 + 35_000.0 * hash01(key, 10)) * central_scale,
                )
            };
            instances.push(CloudInstance {
                center_speed: [
                    center.x,
                    center.y,
                    center.z,
                    wind_speed_meters_per_second / center.length(),
                ],
                wind_axis: [
                    wind_axis.x,
                    wind_axis.y,
                    wind_axis.z,
                    std::f32::consts::TAU * hash01(key, 12),
                ],
                radii_brightness: [radii.x, radii.y, radii.z, 0.86 + 0.12 * hash01(key, 11)],
            });
        }
    }
}

fn hash_u32(value: u32, stream: u32) -> u32 {
    let mut hash = value
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(stream.wrapping_mul(0x85eb_ca6b));
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^ (hash >> 16)
}

fn hash01(value: u32, stream: u32) -> f32 {
    hash_u32(value, stream) as f32 / u32::MAX as f32
}

#[cfg(test)]
mod tests {
    use super::{
        LOWER_CLUSTER_COUNT, UPPER_CLUSTER_COUNT, generate_cloud_instances,
        low_poly_sphere_vertices,
    };

    #[test]
    fn cloud_shader_parses_and_validates() {
        let shader = include_str!("clouds.wgsl");
        let module = wgpu::naga::front::wgsl::parse_str(shader)
            .expect("solid low-poly cloud shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("solid low-poly cloud shader must validate");
    }

    #[test]
    fn cloud_mesh_has_twenty_outward_flat_faces() {
        let vertices = low_poly_sphere_vertices();
        assert_eq!(vertices.len(), 20 * 3);
        for triangle in vertices.chunks_exact(3) {
            let normal = glam::Vec3::from_array(triangle[0].face_normal);
            assert!(
                triangle
                    .iter()
                    .all(|vertex| vertex.face_normal == triangle[0].face_normal)
            );
            assert!(normal.dot(glam::Vec3::from_array(triangle[0].position)) > 0.0);
        }
    }

    #[test]
    fn both_cloud_bands_have_deterministic_non_zero_wind() {
        let first = generate_cloud_instances();
        let second = generate_cloud_instances();
        assert_eq!(first.len(), second.len());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&first),
            bytemuck::cast_slice::<_, u8>(&second)
        );
        assert!(first.len() > (LOWER_CLUSTER_COUNT + UPPER_CLUSTER_COUNT) as usize);
        assert!(first.iter().all(|instance| instance.center_speed[3] > 0.0));
        assert!(
            first
                .iter()
                .all(|instance| instance.radii_brightness[2] > 0.0)
        );
    }

    #[test]
    fn clouds_use_physical_rgb_atmosphere_lighting_without_a_sunset_palette() {
        let shader = include_str!("clouds.wgsl");
        assert!(shader.contains("atmosphere_transmittance_lut"));
        assert!(shader.contains("atmosphere_surface_irradiance_lut"));
        assert!(shader.contains("sample_sun_transmittance"));
        assert!(shader.contains("sample_sky_irradiance"));
        assert!(shader.contains("camera.projection.z * input.center_speed.w"));
        assert!(!shader.contains("SUNSET_CLOUD_COLOR"));
    }
}

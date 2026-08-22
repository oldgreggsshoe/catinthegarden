use std::mem::size_of;

use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::{atmosphere::SurfaceLightingResources, planet::PLANET_RADIUS_METERS, weather};

const CLOUD_SHELL_ALTITUDE_METERS: f32 = 90_000.0;
const UPPER_CLOUD_SHELL_ALTITUDE_METERS: f32 = 166_000.0;
const CLOUD_LAYER_HALF_DEPTH_METERS: f32 =
    (UPPER_CLOUD_SHELL_ALTITUDE_METERS - CLOUD_SHELL_ALTITUDE_METERS) * 0.5;
const CLOUD_SHELL_LONGITUDE_SEGMENTS: usize = 96;
const CLOUD_SHELL_LATITUDE_SEGMENTS: usize = 48;
const CLOUD_TEXTURE_BYTES_PER_ROW: u32 = 256;
const CLOUD_DRIFT_RADIANS_PER_SIMULATED_SECOND: f64 = 0.000002;

fn cloud_drift_radians(weather_time_seconds: f64) -> f32 {
    (weather_time_seconds.max(0.0) * CLOUD_DRIFT_RADIANS_PER_SIMULATED_SECOND)
        .rem_euclid(std::f64::consts::TAU) as f32
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WeatherRenderUniform {
    blend: f32,
    drift_radians: f32,
    lower_shell_radius_meters: f32,
    upper_shell_radius_meters: f32,
    noise_scale: f32,
    noise_strength: f32,
    _padding: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CloudVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

fn weather_cloud_shader_source() -> String {
    format!(
        "const CLOUD_LAYER_HALF_DEPTH_METERS: f32 = {:.1};\n{}\n{}",
        CLOUD_LAYER_HALF_DEPTH_METERS,
        include_str!("weather_render.wgsl"),
        include_str!("weather_cloud_density.wgsl"),
    )
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

pub struct WeatherCloudRenderer {
    pipeline: wgpu::RenderPipeline,
    field_bind_group_layout: wgpu::BindGroupLayout,
    field_bind_group: Option<wgpu::BindGroup>,
    atmosphere_bind_group: wgpu::BindGroup,
    field_textures: [wgpu::Texture; 2],
    field_views: [wgpu::TextureView; 2],
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    current_texture: usize,
    blend: f32,
    drift_radians: f32,
}

impl WeatherCloudRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        atmosphere: SurfaceLightingResources<'_>,
    ) -> Self {
        let field_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("weather cloud field layout"),
                entries: &[
                    texture_array_entry(0),
                    texture_array_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let atmosphere_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("weather cloud atmosphere layout"),
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
            label: Some("weather cloud atmosphere bind group"),
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
            label: Some("weather cloud pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&field_bind_group_layout),
                Some(&atmosphere_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader_source = weather_cloud_shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("weather cloud shell shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("weather cloud shell pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[CloudVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
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
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let texture_desc = wgpu::TextureDescriptor {
            label: Some("weather cloud field"),
            size: wgpu::Extent3d {
                width: weather::WEATHER_FIELD_TEXTURE_SIDE,
                height: weather::WEATHER_FIELD_TEXTURE_SIDE,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let field_textures = [
            device.create_texture(&texture_desc),
            device.create_texture(&texture_desc),
        ];
        let field_views = field_textures.each_ref().map(|texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("weather cloud field cube view"),
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            })
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("weather cloud field sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("weather cloud temporal uniform"),
            size: size_of::<WeatherRenderUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut renderer = Self {
            pipeline,
            field_bind_group_layout,
            field_bind_group: None,
            atmosphere_bind_group,
            field_textures,
            field_views,
            sampler,
            uniform_buffer,
            vertex_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("weather cloud shell vertices"),
                size: 4,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            }),
            vertex_count: 0,
            current_texture: 0,
            blend: 1.0,
            drift_radians: 0.0,
        };
        renderer.field_bind_group = Some(renderer.create_field_bind_group(device));
        let vertices = shell_vertices();
        renderer.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("weather cloud shell vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        renderer.vertex_count = vertices.len() as u32;
        renderer.write_uniform(queue);
        let zeroes = vec![0_u8; padded_field_size()];
        for texture in &renderer.field_textures {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &zeroes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(CLOUD_TEXTURE_BYTES_PER_ROW),
                    rows_per_image: Some(weather::WEATHER_FIELD_TEXTURE_SIDE),
                },
                wgpu::Extent3d {
                    width: weather::WEATHER_FIELD_TEXTURE_SIDE,
                    height: weather::WEATHER_FIELD_TEXTURE_SIDE,
                    depth_or_array_layers: 6,
                },
            );
        }
        renderer
    }

    pub fn replace_field(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        assert_eq!(
            data.len(),
            weather::WEATHER_FIELD_TEXTURE_SIDE as usize
                * weather::WEATHER_FIELD_TEXTURE_SIDE as usize
                * 6
                * 4
        );
        let next = 1 - self.current_texture;
        let padded = pad_field_rows(data);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.field_textures[next],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(CLOUD_TEXTURE_BYTES_PER_ROW),
                rows_per_image: Some(weather::WEATHER_FIELD_TEXTURE_SIDE),
            },
            wgpu::Extent3d {
                width: weather::WEATHER_FIELD_TEXTURE_SIDE,
                height: weather::WEATHER_FIELD_TEXTURE_SIDE,
                depth_or_array_layers: 6,
            },
        );
        self.current_texture = next;
        self.blend = 0.0;
        self.field_bind_group = Some(self.create_field_bind_group(device));
        self.write_uniform(queue);
    }

    /// Installs the first field into both temporal textures so the weather
    /// shell is visible on the first rendered frame. Subsequent updates use
    /// `replace_field` and retain the normal cross-fade.
    pub fn initialize_field(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        assert_eq!(
            data.len(),
            weather::WEATHER_FIELD_TEXTURE_SIDE as usize
                * weather::WEATHER_FIELD_TEXTURE_SIDE as usize
                * 6
                * 4
        );
        let padded = pad_field_rows(data);
        for texture in &self.field_textures {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &padded,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(CLOUD_TEXTURE_BYTES_PER_ROW),
                    rows_per_image: Some(weather::WEATHER_FIELD_TEXTURE_SIDE),
                },
                wgpu::Extent3d {
                    width: weather::WEATHER_FIELD_TEXTURE_SIDE,
                    height: weather::WEATHER_FIELD_TEXTURE_SIDE,
                    depth_or_array_layers: 6,
                },
            );
        }
        self.current_texture = 0;
        self.blend = 1.0;
        self.field_bind_group = Some(self.create_field_bind_group(device));
        self.write_uniform(queue);
    }

    pub fn set_temporal_state(
        &mut self,
        queue: &wgpu::Queue,
        interpolation_fraction: f32,
        weather_time_seconds: f64,
    ) {
        self.blend = interpolation_fraction.clamp(0.0, 1.0);
        self.drift_radians = cloud_drift_radians(weather_time_seconds);
        self.write_uniform(queue);
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(
            1,
            self.field_bind_group
                .as_ref()
                .expect("weather field bind group"),
            &[],
        );
        render_pass.set_bind_group(2, &self.atmosphere_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..2);
    }

    pub fn field_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.field_bind_group_layout
    }

    pub fn field_bind_group(&self) -> &wgpu::BindGroup {
        self.field_bind_group
            .as_ref()
            .expect("weather field bind group")
    }

    fn create_field_bind_group(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("weather cloud field bind group"),
            layout: &self.field_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &self.field_views[self.current_texture],
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &self.field_views[1 - self.current_texture],
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn write_uniform(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&WeatherRenderUniform {
                blend: self.blend,
                drift_radians: self.drift_radians,
                lower_shell_radius_meters: PLANET_RADIUS_METERS as f32
                    + CLOUD_SHELL_ALTITUDE_METERS,
                upper_shell_radius_meters: PLANET_RADIUS_METERS as f32
                    + UPPER_CLOUD_SHELL_ALTITUDE_METERS,
                noise_scale: 32.0,
                noise_strength: 0.18,
                _padding: [0.0; 2],
            }),
        );
    }
}

fn texture_array_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::Cube,
            multisampled: false,
        },
        count: None,
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

fn padded_field_size() -> usize {
    CLOUD_TEXTURE_BYTES_PER_ROW as usize * weather::WEATHER_FIELD_TEXTURE_SIDE as usize * 6
}

fn pad_field_rows(data: &[u8]) -> Vec<u8> {
    let side = weather::WEATHER_FIELD_TEXTURE_SIDE as usize;
    let mut padded = vec![0_u8; padded_field_size()];
    for layer in 0..6 {
        for row in 0..side {
            // WebGPU cubemaps use the conventional top-down face V axis,
            // opposite the weather grid's bottom-up tangent V coordinate.
            let source_row = side - 1 - row;
            let source = (layer * side + source_row) * side * 4;
            let target = (layer * side + row) * CLOUD_TEXTURE_BYTES_PER_ROW as usize;
            padded[target..target + side * 4].copy_from_slice(&data[source..source + side * 4]);
        }
    }
    padded
}

fn shell_vertices() -> Vec<CloudVertex> {
    let mut vertices = Vec::with_capacity(
        CLOUD_SHELL_LONGITUDE_SEGMENTS * CLOUD_SHELL_LATITUDE_SEGMENTS * 6,
    );
    for y in 0..CLOUD_SHELL_LATITUDE_SEGMENTS {
        let v0 = y as f32 / CLOUD_SHELL_LATITUDE_SEGMENTS as f32;
        let v1 = (y + 1) as f32 / CLOUD_SHELL_LATITUDE_SEGMENTS as f32;
        for x in 0..CLOUD_SHELL_LONGITUDE_SEGMENTS {
            let u0 = x as f32 / CLOUD_SHELL_LONGITUDE_SEGMENTS as f32;
            let u1 = (x + 1) as f32 / CLOUD_SHELL_LONGITUDE_SEGMENTS as f32;
            let p = |u: f32, v: f32| {
                let latitude = (v - 0.5) * std::f32::consts::PI;
                let longitude = u * std::f32::consts::TAU;
                Vec3::new(
                    latitude.cos() * longitude.cos(),
                    latitude.sin(),
                    latitude.cos() * longitude.sin(),
                )
            };
            let a = p(u0, v0);
            let b = p(u1, v0);
            let c = p(u1, v1);
            let d = p(u0, v1);
            for point in [a, b, c, a, c, d] {
                vertices.push(CloudVertex {
                    position: point.to_array(),
                    normal: point.to_array(),
                });
            }
        }
    }
    vertices
}

#[cfg(test)]
mod tests {
    #[test]
    fn weather_cloud_shader_parses_and_uses_temporal_field_pair() {
        let shader = super::weather_cloud_shader_source();
        let module =
            wgpu::naga::front::wgsl::parse_str(&shader).expect("weather cloud shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("weather cloud shader must validate");
        assert!(shader.contains("cloud_field_current"));
        assert!(shader.contains("cloud_field_previous"));
        assert!(shader.contains("mix(previous, current, weather.blend)"));
        assert!(shader.contains("@builtin(instance_index) instance_index"));
        assert!(shader.contains("fn flow_warp"));
        assert!(shader.contains("fn cloud_noise"));
        assert!(shader.contains("fn cloudSample("));
        assert!(shader.contains("fn cloudDensity"));
        assert!(shader.contains("let posterized = smoothstep"));
        assert!(shader.contains("smoothstep(0.05, 0.32, field.g)"));
        assert!(shader.contains("max(density, smoothstep(0.25, 0.85, density))"));
        assert!(shader.contains("smoothstep(0.10, 0.30, cloud.storm)"));
        assert!(shader.contains("let storm_darkening = 1.0 - 0.72 * storm_weight;"));
        assert!(shader.contains("fn henyey_greenstein"));
        assert!(shader.contains("let translucent_edge"));
        assert!(shader.contains("let silver_lining"));
        assert!(shader.contains("fn direct_atmosphere_uv"));
        assert!(shader.contains("fn cloud_layer_sun_visibility"));
        assert!(shader.contains("CLOUD_LAYER_HALF_DEPTH_METERS: f32 = 38000.0"));
        assert!(shader.contains("fn solid_planet_blocks_cloud"));
        assert!(shader.contains("if solid_planet_blocks_cloud("));
        assert!(shader.contains("max(solar_zenith_cosine, 0.0)"));
        assert!(shader.contains("let humidity_precursor"));
        assert!(!shader.contains("let precursor = select(0.08"));
        assert!(shader.contains("texture_cube<f32>"));
        assert!(!shader.contains("fn cube_field_uv"));
    }

    #[test]
    fn shell_mesh_is_non_empty_and_has_expected_density() {
        assert_eq!(super::shell_vertices().len(), 96 * 48 * 6);
        assert_eq!(super::shell_vertices().len() * 2, 55_296);
    }

    #[test]
    fn cloud_detail_drift_uses_simulated_weather_time() {
        let one_real_second_of_weather = super::cloud_drift_radians(
            crate::weather::INTERACTIVE_WEATHER_TIME_SCALE,
        );
        assert!((one_real_second_of_weather - 0.0072).abs() < 1.0e-6);
    }

    #[test]
    fn lower_cloud_shell_faces_stay_above_ocean_depth() {
        let minimum_unit_face_radius = super::shell_vertices()
            .chunks_exact(3)
            .filter_map(|triangle| {
                let a = glam::Vec3::from_array(triangle[0].position);
                let b = glam::Vec3::from_array(triangle[1].position);
                let c = glam::Vec3::from_array(triangle[2].position);
                let normal = (b - a).cross(c - a);
                (normal.length_squared() > 1.0e-12)
                    .then(|| normal.normalize().dot(a).abs())
            })
            .fold(f32::INFINITY, f32::min);
        let minimum_clearance_meters =
            (crate::planet::PLANET_RADIUS_METERS as f32 + super::CLOUD_SHELL_ALTITUDE_METERS)
                * minimum_unit_face_radius
                - crate::planet::PLANET_RADIUS_METERS as f32;

        assert!(
            minimum_clearance_meters >= 85_000.0,
            "lower cloud triangles sag to {minimum_clearance_meters:.3}m above sea level"
        );
    }

    #[test]
    fn cloud_shells_shift_up_one_existing_altitude_slot() {
        assert_eq!(super::CLOUD_SHELL_ALTITUDE_METERS, 90_000.0);
        assert_eq!(super::UPPER_CLOUD_SHELL_ALTITUDE_METERS, 166_000.0);
        assert_eq!(
            super::UPPER_CLOUD_SHELL_ALTITUDE_METERS - super::CLOUD_SHELL_ALTITUDE_METERS,
            76_000.0,
        );
    }

    #[test]
    fn elevated_cloud_shells_keep_sun_after_ground_sunset() {
        let horizon_cosine = |altitude_meters: f32| {
            let radius = crate::planet::PLANET_RADIUS_METERS as f32 + altitude_meters;
            -(1.0 - (crate::planet::PLANET_RADIUS_METERS as f32 / radius).powi(2)).sqrt()
        };

        let early_twilight_sun_cosine = -0.06_f32;
        assert!(early_twilight_sun_cosine < horizon_cosine(0.0));
        assert!(early_twilight_sun_cosine > horizon_cosine(super::CLOUD_SHELL_ALTITUDE_METERS));

        let deeper_twilight_sun_cosine = -0.24_f32;
        assert!(deeper_twilight_sun_cosine < horizon_cosine(super::CLOUD_SHELL_ALTITUDE_METERS));
        assert!(
            deeper_twilight_sun_cosine > horizon_cosine(super::UPPER_CLOUD_SHELL_ALTITUDE_METERS)
        );
    }

    #[test]
    fn cloud_layer_depths_overlap_at_the_twilight_handover() {
        let horizon_cosine = |altitude_meters: f32| {
            let radius = crate::planet::PLANET_RADIUS_METERS as f32 + altitude_meters;
            -(1.0 - (crate::planet::PLANET_RADIUS_METERS as f32 / radius).powi(2)).sqrt()
        };
        let solar_angular_radius_sine = 0.004625_f32;
        let transition = |altitude_meters: f32| {
            (
                horizon_cosine(altitude_meters + super::CLOUD_LAYER_HALF_DEPTH_METERS)
                    - solar_angular_radius_sine,
                horizon_cosine((altitude_meters - super::CLOUD_LAYER_HALF_DEPTH_METERS).max(0.0))
                    + solar_angular_radius_sine,
            )
        };
        let lower = transition(super::CLOUD_SHELL_ALTITUDE_METERS);
        let upper = transition(super::UPPER_CLOUD_SHELL_ALTITUDE_METERS);

        assert!(lower.0 < lower.1, "lower layer must fade rather than step");
        assert!(upper.0 < upper.1, "upper layer must fade rather than step");
        assert!(
            lower.0 < upper.1,
            "upper twilight fade must overlap the lower layer handover"
        );
    }

    #[test]
    fn cubemap_upload_flips_source_rows_to_webgpu_orientation() {
        let side = crate::weather::WEATHER_FIELD_TEXTURE_SIDE as usize;
        let mut data = vec![0_u8; side * side * 6 * 4];
        for layer in 0..6 {
            for row in 0..side {
                for column in 0..side {
                    data[((layer * side + row) * side + column) * 4] = row as u8;
                }
            }
        }

        let padded = super::pad_field_rows(&data);
        let row_stride = super::CLOUD_TEXTURE_BYTES_PER_ROW as usize;
        for layer in 0..6 {
            let layer_start = layer * side * row_stride;
            assert_eq!(padded[layer_start], (side - 1) as u8);
            assert_eq!(padded[layer_start + (side - 1) * row_stride], 0);
        }
    }
}

use std::{error::Error, fmt};

use catinthegarden_coretypes::{
    CubeFace, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey,
};

use crate::{
    outmap::{Outmap, OutmapError, TileData},
    planet::{PLANET_RADIUS_METERS, outmap_terrain_height_scale},
    terrain::TerrainSource,
};

const FIELD_LEVEL: u8 = 4;
const FACE_COUNT: u32 = 6;
const WARP_SCALE_NUMERATOR: u32 = 3;
const WARP_SCALE_DENOMINATOR: u32 = 4;
const FOVEA_FOLLOW_RATE_PER_SECOND: f32 = 5.0;
const EXPERIMENT_HORIZON_DENSITY: u32 = 1 << 0;
const EXPERIMENT_TEMPORAL_REUSE: u32 = 1 << 1;
const EXPERIMENT_CONTENT_ADAPTIVE: u32 = 1 << 2;
const EXPERIMENT_FOVEATED_SHADING: u32 = 1 << 3;
const EXPERIMENT_RADIAL_BLUR: u32 = 1 << 4;
const EXPERIMENT_HALFTONE: u32 = 1 << 5;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RayUniform {
    height_min_meters: f32,
    height_max_meters: f32,
    face_quads: u32,
    march_steps: u32,
    camera_radius_meters: f32,
    camera_radius_squared: f32,
    minimum_shell_radius_meters: f32,
    maximum_shell_radius_meters: f32,
    max_height_mip_count: u32,
    minimum_step_meters: f32,
    fovea_ndc: [f32; 2],
    experiment_flags: u32,
    frame_index: u32,
    _padding: [u32; 2],
    previous_fovea_ndc: [f32; 2],
    temporal_valid: u32,
    _temporal_padding: u32,
    previous_camera_forward: [f32; 4],
    previous_camera_right: [f32; 4],
    previous_camera_up: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WarpUniform {
    fovea_ndc: [f32; 2],
    debug_view: u32,
    experiment_flags: u32,
}

pub struct FoveatedRenderer {
    direct_pipeline: wgpu::RenderPipeline,
    warp_pipeline: wgpu::RenderPipeline,
    unwarp_pipeline: wgpu::RenderPipeline,
    fields_bind_group: wgpu::BindGroup,
    temporal_bind_group_layout: wgpu::BindGroupLayout,
    temporal_bind_group: wgpu::BindGroup,
    unwarp_bind_group_layout: wgpu::BindGroupLayout,
    unwarp_bind_group: wgpu::BindGroup,
    ray_uniform_buffer: wgpu::Buffer,
    warp_uniform_buffer: wgpu::Buffer,
    warp_sampler: wgpu::Sampler,
    warp_color_format: wgpu::TextureFormat,
    warp_size: winit::dpi::PhysicalSize<u32>,
    warp_debug_visible: bool,
    fovea_ndc: [f32; 2],
    experiment_flags: u32,
    frame_index: u32,
    previous_camera_basis: Option<[[f32; 3]; 3]>,
    previous_camera_position: Option<[f64; 3]>,
    history_valid: bool,
    height_min_meters: f32,
    height_max_meters: f32,
    face_quads: u32,
    max_height_mip_count: u32,
    _height_texture: wgpu::Texture,
    _max_height_texture: wgpu::Texture,
    _biome_texture: wgpu::Texture,
    _moisture_texture: wgpu::Texture,
    warp_color_texture: wgpu::Texture,
    warp_color_view: wgpu::TextureView,
    warp_distance_texture: wgpu::Texture,
    warp_distance_view: wgpu::TextureView,
    history_color_texture: wgpu::Texture,
    history_color_view: wgpu::TextureView,
    history_distance_texture: wgpu::Texture,
    history_distance_view: wgpu::TextureView,
}

impl FoveatedRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        size: winit::dpi::PhysicalSize<u32>,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        shared_bind_group_layout: &wgpu::BindGroupLayout,
        terrain_source: TerrainSource,
    ) -> Result<Self, FoveatedError> {
        let source = FieldSource::new(terrain_source)?;
        let face_quads = source.face_quads();
        let extent = face_quads + TILE_GUTTER * 2;
        let texture_extent = wgpu::Extent3d {
            width: extent,
            height: extent,
            depth_or_array_layers: FACE_COUNT,
        };
        let max_height_mip_count = extent.ilog2() + 1;
        let height_texture = create_face_texture(
            device,
            "ray height faces",
            texture_extent,
            wgpu::TextureFormat::R32Float,
            1,
        );
        let max_height_texture = create_face_texture(
            device,
            "ray max-height faces",
            texture_extent,
            wgpu::TextureFormat::R32Float,
            max_height_mip_count,
        );
        let biome_texture = create_face_texture(
            device,
            "ray biome faces",
            texture_extent,
            wgpu::TextureFormat::R8Uint,
            1,
        );
        let moisture_texture = create_face_texture(
            device,
            "ray moisture faces",
            texture_extent,
            wgpu::TextureFormat::R8Unorm,
            1,
        );

        for face in CubeFace::ALL {
            let fields = source.build_face(face)?;
            upload_face_layer(
                queue,
                &height_texture,
                face.index() as u32,
                0,
                extent,
                bytemuck::cast_slice(&fields.heights_meters),
                size_of::<f32>() as u32,
            );
            for (mip_level, mip) in max_height_mips(&fields.heights_meters, extent)
                .into_iter()
                .enumerate()
            {
                let mip_extent = (extent >> mip_level).max(1);
                upload_face_layer(
                    queue,
                    &max_height_texture,
                    face.index() as u32,
                    mip_level as u32,
                    mip_extent,
                    bytemuck::cast_slice(&mip),
                    size_of::<f32>() as u32,
                );
            }
            upload_face_layer(
                queue,
                &biome_texture,
                face.index() as u32,
                0,
                extent,
                &fields.biome_ids,
                size_of::<u8>() as u32,
            );
            upload_face_layer(
                queue,
                &moisture_texture,
                face.index() as u32,
                0,
                extent,
                &fields.moisture,
                size_of::<u8>() as u32,
            );
        }

        let fields_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ray face fields layout"),
                entries: &[
                    texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                    texture_layout_entry(1, wgpu::TextureSampleType::Uint),
                    texture_layout_entry(2, wgpu::TextureSampleType::Float { filterable: true }),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    texture_layout_entry(4, wgpu::TextureSampleType::Float { filterable: false }),
                ],
            });
        let height_min_meters = source.height_min_meters();
        let height_max_meters = source.height_max_meters();
        let initial_uniform = RayUniform::for_camera(
            height_min_meters,
            height_max_meters,
            face_quads,
            max_height_mip_count,
            0.0,
        );
        let ray_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raymarch uniform"),
            size: size_of::<RayUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ray_uniform_buffer, 0, bytemuck::bytes_of(&initial_uniform));
        let fields_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ray face fields bind group"),
            layout: &fields_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&face_array_view(&height_texture)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&face_array_view(&biome_texture)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&face_array_view(
                        &moisture_texture,
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ray_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&face_array_view(
                        &max_height_texture,
                    )),
                },
            ],
        });
        let temporal_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("foveated temporal history layout"),
                entries: &[
                    texture_2d_layout_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                    texture_2d_layout_entry(
                        1,
                        wgpu::TextureSampleType::Float { filterable: false },
                    ),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ray field debug pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&fields_bind_group_layout),
                Some(shared_bind_group_layout),
                Some(&temporal_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader_source = raymarch_shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("full-resolution terrain raymarch shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let direct_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("full-resolution terrain raymarch pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let warp_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("warped terrain raymarch pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_warp"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::R32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let unwarp_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("foveated unwarp layout"),
                entries: &[
                    texture_2d_layout_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                    texture_2d_layout_entry(
                        1,
                        wgpu::TextureSampleType::Float { filterable: false },
                    ),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let unwarp_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("foveated unwarp pipeline layout"),
                bind_group_layouts: &[
                    Some(camera_bind_group_layout),
                    Some(&unwarp_bind_group_layout),
                ],
                immediate_size: 0,
            });
        let unwarp_shader =
            device.create_shader_module(wgpu::include_wgsl!("foveated_unwarp.wgsl"));
        let unwarp_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("foveated unwarp pipeline"),
            layout: Some(&unwarp_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &unwarp_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &unwarp_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let warp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("foveated warp sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let warp_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("foveated warp uniform"),
            size: size_of::<WarpUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &warp_uniform_buffer,
            0,
            bytemuck::bytes_of(&WarpUniform {
                fovea_ndc: [0.0; 2],
                debug_view: 0,
                experiment_flags: 0,
            }),
        );
        let warp_size = warp_size_for(size);
        let (warp_color_texture, warp_color_view) =
            create_warp_texture(device, "foveated warp color", warp_size, surface_format);
        let (warp_distance_texture, warp_distance_view) = create_warp_texture(
            device,
            "foveated warp distance",
            warp_size,
            wgpu::TextureFormat::R32Float,
        );
        let (history_color_texture, history_color_view) =
            create_warp_texture(device, "foveated history color", warp_size, surface_format);
        let (history_distance_texture, history_distance_view) = create_warp_texture(
            device,
            "foveated history distance",
            warp_size,
            wgpu::TextureFormat::R32Float,
        );
        let temporal_bind_group = create_temporal_bind_group(
            device,
            &temporal_bind_group_layout,
            &history_color_view,
            &history_distance_view,
            &warp_sampler,
        );
        let unwarp_bind_group = create_unwarp_bind_group(
            device,
            &unwarp_bind_group_layout,
            &warp_color_view,
            &warp_distance_view,
            &warp_sampler,
            &warp_uniform_buffer,
        );

        Ok(Self {
            direct_pipeline,
            warp_pipeline,
            unwarp_pipeline,
            fields_bind_group,
            temporal_bind_group_layout,
            temporal_bind_group,
            unwarp_bind_group_layout,
            unwarp_bind_group,
            ray_uniform_buffer,
            warp_uniform_buffer,
            warp_sampler,
            warp_color_format: surface_format,
            warp_size,
            warp_debug_visible: false,
            fovea_ndc: [0.0; 2],
            experiment_flags: 0,
            frame_index: 0,
            previous_camera_basis: None,
            previous_camera_position: None,
            history_valid: false,
            height_min_meters,
            height_max_meters,
            face_quads,
            max_height_mip_count,
            _height_texture: height_texture,
            _max_height_texture: max_height_texture,
            _biome_texture: biome_texture,
            _moisture_texture: moisture_texture,
            warp_color_texture,
            warp_color_view,
            warp_distance_texture,
            warp_distance_view,
            history_color_texture,
            history_color_view,
            history_distance_texture,
            history_distance_view,
        })
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera_altitude_meters: f64,
        target_fovea_ndc: [f32; 2],
        delta_seconds: f32,
        camera_forward: [f32; 3],
        camera_right: [f32; 3],
        camera_up: [f32; 3],
        camera_position: [f64; 3],
    ) {
        let previous_fovea_ndc = self.fovea_ndc;
        let camera_displacement_meters = self.previous_camera_position.map_or(f64::INFINITY, |p| {
            ((camera_position[0] - p[0]).powi(2)
                + (camera_position[1] - p[1]).powi(2)
                + (camera_position[2] - p[2]).powi(2))
            .sqrt()
        });
        let temporal_valid = self.history_valid
            && self.previous_camera_basis.is_some_and(|previous| {
                dot3(previous[0], camera_forward) > 0.999_85
                    && dot3(previous[1], camera_right) > 0.999_85
                    && dot3(previous[2], camera_up) > 0.999_85
                    && camera_displacement_meters <= 10.0
            });
        self.fovea_ndc = eased_fovea_ndc(
            self.fovea_ndc,
            target_fovea_ndc,
            delta_seconds,
            FOVEA_FOLLOW_RATE_PER_SECOND,
        );
        self.frame_index = self.frame_index.wrapping_add(1);
        let mut uniform = RayUniform::for_camera(
            self.height_min_meters,
            self.height_max_meters,
            self.face_quads,
            self.max_height_mip_count,
            camera_altitude_meters,
        );
        uniform.fovea_ndc = self.fovea_ndc;
        uniform.experiment_flags = self.experiment_flags;
        uniform.frame_index = self.frame_index;
        uniform.previous_fovea_ndc = previous_fovea_ndc;
        uniform.temporal_valid = u32::from(temporal_valid);
        if let Some(previous) = self.previous_camera_basis {
            uniform.previous_camera_forward[..3].copy_from_slice(&previous[0]);
            uniform.previous_camera_right[..3].copy_from_slice(&previous[1]);
            uniform.previous_camera_up[..3].copy_from_slice(&previous[2]);
        }
        queue.write_buffer(&self.ray_uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        self.write_warp_uniform(queue);
        self.previous_camera_basis = Some([camera_forward, camera_right, camera_up]);
        self.previous_camera_position = Some(camera_position);
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: winit::dpi::PhysicalSize<u32>) {
        let warp_size = warp_size_for(size);
        if warp_size == self.warp_size {
            return;
        }
        let (warp_color_texture, warp_color_view) = create_warp_texture(
            device,
            "foveated warp color",
            warp_size,
            self.warp_color_format,
        );
        let (warp_distance_texture, warp_distance_view) = create_warp_texture(
            device,
            "foveated warp distance",
            warp_size,
            wgpu::TextureFormat::R32Float,
        );
        let (history_color_texture, history_color_view) = create_warp_texture(
            device,
            "foveated history color",
            warp_size,
            self.warp_color_format,
        );
        let (history_distance_texture, history_distance_view) = create_warp_texture(
            device,
            "foveated history distance",
            warp_size,
            wgpu::TextureFormat::R32Float,
        );
        self.temporal_bind_group = create_temporal_bind_group(
            device,
            &self.temporal_bind_group_layout,
            &history_color_view,
            &history_distance_view,
            &self.warp_sampler,
        );
        self.unwarp_bind_group = create_unwarp_bind_group(
            device,
            &self.unwarp_bind_group_layout,
            &warp_color_view,
            &warp_distance_view,
            &self.warp_sampler,
            &self.warp_uniform_buffer,
        );
        self.warp_size = warp_size;
        self.warp_color_texture = warp_color_texture;
        self.warp_color_view = warp_color_view;
        self.warp_distance_texture = warp_distance_texture;
        self.warp_distance_view = warp_distance_view;
        self.history_color_texture = history_color_texture;
        self.history_color_view = history_color_view;
        self.history_distance_texture = history_distance_texture;
        self.history_distance_view = history_distance_view;
        self.history_valid = false;
    }

    pub fn toggle_warp_debug(&mut self, queue: &wgpu::Queue) {
        self.warp_debug_visible = !self.warp_debug_visible;
        self.write_warp_uniform(queue);
    }

    pub fn toggle_experiment(&mut self, queue: &wgpu::Queue, index: u8) {
        let flag = experiment_flag(index);
        self.experiment_flags ^= flag;
        if index == 2 {
            self.history_valid = false;
        }
        self.write_warp_uniform(queue);
    }

    pub fn experiment_enabled(&self, index: u8) -> bool {
        self.experiment_flags & experiment_flag(index) != 0
    }

    fn write_warp_uniform(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.warp_uniform_buffer,
            0,
            bytemuck::bytes_of(&WarpUniform {
                fovea_ndc: self.fovea_ndc,
                debug_view: u32::from(self.warp_debug_visible),
                experiment_flags: self.experiment_flags,
            }),
        );
    }

    pub const fn warp_debug_visible(&self) -> bool {
        self.warp_debug_visible
    }

    pub const fn warp_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.warp_size
    }

    pub const fn fovea_ndc(&self) -> [f32; 2] {
        self.fovea_ndc
    }

    pub fn warp_color_view(&self) -> &wgpu::TextureView {
        &self.warp_color_view
    }

    pub fn warp_distance_view(&self) -> &wgpu::TextureView {
        &self.warp_distance_view
    }

    pub fn copy_to_history(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let extent = wgpu::Extent3d {
            width: self.warp_size.width,
            height: self.warp_size.height,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_texture(
            self.warp_color_texture.as_image_copy(),
            self.history_color_texture.as_image_copy(),
            extent,
        );
        encoder.copy_texture_to_texture(
            self.warp_distance_texture.as_image_copy(),
            self.history_distance_texture.as_image_copy(),
            extent,
        );
        self.history_valid = true;
    }

    pub fn draw_direct<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        shared_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.direct_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.fields_bind_group, &[]);
        render_pass.set_bind_group(2, shared_bind_group, &[]);
        render_pass.set_bind_group(3, &self.temporal_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    pub fn draw_warped<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        shared_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.warp_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.fields_bind_group, &[]);
        render_pass.set_bind_group(2, shared_bind_group, &[]);
        render_pass.set_bind_group(3, &self.temporal_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    pub fn draw_unwarp<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.unwarp_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.unwarp_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

impl RayUniform {
    const MARCH_STEPS: u32 = 192;

    fn for_camera(
        height_min_meters: f32,
        height_max_meters: f32,
        face_quads: u32,
        max_height_mip_count: u32,
        camera_altitude_meters: f64,
    ) -> Self {
        let height_scale = outmap_terrain_height_scale(camera_altitude_meters);
        let camera_radius_meters = PLANET_RADIUS_METERS + camera_altitude_meters;
        Self {
            height_min_meters,
            height_max_meters,
            face_quads,
            march_steps: Self::MARCH_STEPS,
            camera_radius_meters: camera_radius_meters as f32,
            camera_radius_squared: camera_radius_meters.powi(2) as f32,
            minimum_shell_radius_meters: (PLANET_RADIUS_METERS
                + f64::from(height_min_meters) * height_scale)
                as f32,
            maximum_shell_radius_meters: (PLANET_RADIUS_METERS
                + f64::from(height_max_meters) * height_scale)
                as f32,
            max_height_mip_count,
            minimum_step_meters: (PLANET_RADIUS_METERS * 2.0 / f64::from(face_quads)) as f32 * 0.5,
            fovea_ndc: [0.0; 2],
            experiment_flags: 0,
            frame_index: 0,
            _padding: [0; 2],
            previous_fovea_ndc: [0.0; 2],
            temporal_valid: 0,
            _temporal_padding: 0,
            previous_camera_forward: [0.0; 4],
            previous_camera_right: [0.0; 4],
            previous_camera_up: [0.0; 4],
        }
    }
}

fn experiment_flag(index: u8) -> u32 {
    match index {
        1 => EXPERIMENT_HORIZON_DENSITY,
        2 => EXPERIMENT_TEMPORAL_REUSE,
        3 => EXPERIMENT_CONTENT_ADAPTIVE,
        4 => EXPERIMENT_FOVEATED_SHADING,
        5 => EXPERIMENT_RADIAL_BLUR,
        6 => EXPERIMENT_HALFTONE,
        _ => panic!("M8 experiment index must be in 1..=6"),
    }
}

fn eased_fovea_ndc(
    current: [f32; 2],
    target: [f32; 2],
    delta_seconds: f32,
    follow_rate_per_second: f32,
) -> [f32; 2] {
    let blend = 1.0 - (-follow_rate_per_second * delta_seconds.max(0.0)).exp();
    [
        current[0] + (target[0] - current[0]) * blend,
        current[1] + (target[1] - current[1]) * blend,
    ]
}

fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn raymarch_shader_source() -> String {
    [
        include_str!("shared_planet.wgsl"),
        include_str!("foveated_debug.wgsl"),
    ]
    .join("\n")
}

fn texture_layout_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn texture_2d_layout_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn warp_size_for(size: winit::dpi::PhysicalSize<u32>) -> winit::dpi::PhysicalSize<u32> {
    winit::dpi::PhysicalSize::new(
        (size.width * WARP_SCALE_NUMERATOR / WARP_SCALE_DENOMINATOR).max(1),
        (size.height * WARP_SCALE_NUMERATOR / WARP_SCALE_DENOMINATOR).max(1),
    )
}

fn create_warp_texture(
    device: &wgpu::Device,
    label: &str,
    size: winit::dpi::PhysicalSize<u32>,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_temporal_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    color_view: &wgpu::TextureView,
    distance_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("foveated temporal history bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(distance_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn create_unwarp_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    color_view: &wgpu::TextureView,
    distance_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("foveated unwarp bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(distance_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: uniform_buffer.as_entire_binding(),
            },
        ],
    })
}

fn create_face_texture(
    device: &wgpu::Device,
    label: &str,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
    mip_level_count: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn face_array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

fn upload_face_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    mip_level: u32,
    extent: u32,
    bytes: &[u8],
    bytes_per_texel: u32,
) {
    let padded = padded_texture_rows(bytes, extent, extent, bytes_per_texel);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned_texture_row_bytes(extent * bytes_per_texel)),
            rows_per_image: Some(extent),
        },
        wgpu::Extent3d {
            width: extent,
            height: extent,
            depth_or_array_layers: 1,
        },
    );
}

fn aligned_texture_row_bytes(row_bytes: u32) -> u32 {
    row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

fn padded_texture_rows(bytes: &[u8], width: u32, height: u32, bytes_per_texel: u32) -> Vec<u8> {
    let row_bytes = width * bytes_per_texel;
    assert_eq!(bytes.len(), (row_bytes * height) as usize);
    let aligned_row_bytes = aligned_texture_row_bytes(row_bytes);
    if aligned_row_bytes == row_bytes {
        return bytes.to_vec();
    }
    let mut padded = vec![0; (aligned_row_bytes * height) as usize];
    for row in 0..height as usize {
        let source_start = row * row_bytes as usize;
        let target_start = row * aligned_row_bytes as usize;
        padded[target_start..target_start + row_bytes as usize]
            .copy_from_slice(&bytes[source_start..source_start + row_bytes as usize]);
    }
    padded
}

fn max_height_mips(base: &[f32], extent: u32) -> Vec<Vec<f32>> {
    assert_eq!(base.len(), (extent * extent) as usize);
    let mut mips = vec![base.to_vec()];
    let mut source_extent = extent;
    while source_extent > 1 {
        let target_extent = (source_extent / 2).max(1);
        let source = mips.last().expect("base mip exists");
        let mut target = Vec::with_capacity((target_extent * target_extent) as usize);
        for target_y in 0..target_extent {
            let source_y_start = target_y * source_extent / target_extent;
            let source_y_end = (target_y + 1) * source_extent / target_extent;
            for target_x in 0..target_extent {
                let source_x_start = target_x * source_extent / target_extent;
                let source_x_end = (target_x + 1) * source_extent / target_extent;
                let mut maximum = f32::NEG_INFINITY;
                for source_y in source_y_start..source_y_end {
                    for source_x in source_x_start..source_x_end {
                        maximum =
                            maximum.max(source[(source_y * source_extent + source_x) as usize]);
                    }
                }
                target.push(maximum);
            }
        }
        mips.push(target);
        source_extent = target_extent;
    }
    mips
}

struct FaceFields {
    heights_meters: Vec<f32>,
    biome_ids: Vec<u8>,
    moisture: Vec<u8>,
}

enum FieldSource {
    Placeholder,
    Outmap { outmap: Outmap, level: u8 },
}

impl FieldSource {
    fn new(source: TerrainSource) -> Result<Self, FoveatedError> {
        match source {
            TerrainSource::Placeholder => Ok(Self::Placeholder),
            TerrainSource::Outmap(path) => {
                let outmap = Outmap::open(path)?;
                let level = outmap.manifest().dense_level.min(FIELD_LEVEL);
                Ok(Self::Outmap { outmap, level })
            }
        }
    }

    fn face_quads(&self) -> u32 {
        TILE_LOGICAL_SIZE.saturating_sub(1) * (1_u32 << self.level())
    }

    fn level(&self) -> u8 {
        match self {
            Self::Placeholder => 0,
            Self::Outmap { level, .. } => *level,
        }
    }

    fn height_min_meters(&self) -> f32 {
        match self {
            Self::Placeholder => -1.0,
            Self::Outmap { outmap, .. } => outmap.manifest().height_min_meters,
        }
    }

    fn height_max_meters(&self) -> f32 {
        match self {
            Self::Placeholder => 1.0,
            Self::Outmap { outmap, .. } => outmap.manifest().height_max_meters,
        }
    }

    fn build_face(&self, face: CubeFace) -> Result<FaceFields, FoveatedError> {
        match self {
            Self::Placeholder => {
                let extent = self.face_quads() + TILE_GUTTER * 2;
                let sample_count = (extent * extent) as usize;
                Ok(FaceFields {
                    heights_meters: vec![0.0; sample_count],
                    biome_ids: vec![0; sample_count],
                    moisture: vec![128; sample_count],
                })
            }
            Self::Outmap { outmap, level } => build_outmap_face(outmap, *level, face),
        }
    }
}

fn build_outmap_face(
    outmap: &Outmap,
    level: u8,
    face: CubeFace,
) -> Result<FaceFields, FoveatedError> {
    let side = 1_u32 << level;
    let mut tiles = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        for x in 0..side {
            let requested_key = TileKey { face, level, x, y };
            let tile = outmap.load_tile(requested_key)?;
            if tile.source_key != requested_key {
                return Err(FoveatedError::IncompleteDenseLevel {
                    requested: requested_key,
                    source: tile.source_key,
                });
            }
            tiles.push(tile);
        }
    }

    Ok(stitch_face_tiles(side, &tiles))
}

fn stitch_face_tiles(side: u32, tiles: &[TileData]) -> FaceFields {
    assert_eq!(tiles.len(), (side * side) as usize);
    let face_quads = (TILE_LOGICAL_SIZE - 1) * side;
    let extent = face_quads + TILE_GUTTER * 2;
    let axis_sources: Vec<_> = (0..extent)
        .map(|coordinate| face_sample_source(coordinate, side))
        .collect();
    let sample_count = (extent * extent) as usize;
    let mut fields = FaceFields {
        heights_meters: Vec::with_capacity(sample_count),
        biome_ids: Vec::with_capacity(sample_count),
        moisture: Vec::with_capacity(sample_count),
    };
    for &(tile_y, sample_y) in &axis_sources {
        for &(tile_x, sample_x) in &axis_sources {
            let tile = &tiles[(tile_y * side + tile_x) as usize];
            let source_index = (sample_y * TILE_STORED_SIZE + sample_x) as usize;
            fields
                .heights_meters
                .push(tile.heights_meters[source_index]);
            fields.biome_ids.push(tile.biome_ids[source_index]);
            fields.moisture.push(tile.moisture[source_index]);
        }
    }
    fields
}

fn face_sample_source(coordinate: u32, side: u32) -> (u32, u32) {
    let face_quads = (TILE_LOGICAL_SIZE - 1) * side;
    if coordinate == 0 {
        return (0, 0);
    }
    if coordinate == face_quads + 1 {
        return (side - 1, TILE_STORED_SIZE - 1);
    }
    let logical_coordinate = coordinate - 1;
    let tile = (logical_coordinate / (TILE_LOGICAL_SIZE - 1)).min(side - 1);
    let sample = logical_coordinate - tile * (TILE_LOGICAL_SIZE - 1) + TILE_GUTTER;
    (tile, sample)
}

#[derive(Debug)]
pub enum FoveatedError {
    Outmap(OutmapError),
    IncompleteDenseLevel { requested: TileKey, source: TileKey },
}

impl fmt::Display for FoveatedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outmap(source) => write!(formatter, "could not build ray face fields: {source}"),
            Self::IncompleteDenseLevel { requested, source } => write!(
                formatter,
                "ray face field requires dense tile {requested:?}, but it resolved to {source:?}"
            ),
        }
    }
}

impl Error for FoveatedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Outmap(source) => Some(source),
            Self::IncompleteDenseLevel { .. } => None,
        }
    }
}

impl From<OutmapError> for FoveatedError {
    fn from(source: OutmapError) -> Self {
        Self::Outmap(source)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EXPERIMENT_CONTENT_ADAPTIVE, EXPERIMENT_FOVEATED_SHADING, EXPERIMENT_HALFTONE,
        EXPERIMENT_HORIZON_DENSITY, EXPERIMENT_RADIAL_BLUR, EXPERIMENT_TEMPORAL_REUSE, FIELD_LEVEL,
        RayUniform, WarpUniform, eased_fovea_ndc, experiment_flag, face_sample_source,
        max_height_mips, raymarch_shader_source, warp_size_for,
    };
    use catinthegarden_coretypes::{TILE_LOGICAL_SIZE, TILE_STORED_SIZE};

    const PLANET_RADIUS_METERS: f64 = 4_000_000.0;

    fn warp_axis(value: f32) -> f32 {
        let core = 0.5_f32;
        let denominator = (1.0 + core).powi(2) - core.powi(2);
        value.signum() * (((value.abs() + core).powi(2) - core.powi(2)) / denominator)
    }

    fn unwarp_axis(value: f32) -> f32 {
        let core = 0.5_f32;
        let denominator = (1.0 + core).powi(2) - core.powi(2);
        value.signum() * ((value.abs() * denominator + core.powi(2)).sqrt() - core)
    }

    fn warped_screen_axis(value: f32, fovea: f32) -> f32 {
        let side_extent = if value >= 0.0 {
            1.0 - fovea
        } else {
            1.0 + fovea
        };
        fovea + warp_axis(value) * side_extent
    }

    fn unwarped_texture_axis(value: f32, fovea: f32) -> f32 {
        let offset = value - fovea;
        let side_extent = if offset >= 0.0 {
            1.0 - fovea
        } else {
            1.0 + fovea
        };
        unwarp_axis(offset / side_extent)
    }

    fn sphere_entry_distance_f64(camera_radius: f64, ray_inward_cosine: f64) -> Option<f64> {
        let radial_dot_ray = -camera_radius * ray_inward_cosine;
        let discriminant = radial_dot_ray * radial_dot_ray
            - (camera_radius * camera_radius - PLANET_RADIUS_METERS * PLANET_RADIUS_METERS);
        (discriminant >= 0.0).then(|| -radial_dot_ray - discriminant.sqrt())
    }

    fn shader_radius_at_surface(
        camera_radius: f64,
        ray_inward_cosine: f64,
        distance_meters: f64,
    ) -> f32 {
        let camera_radius_squared = camera_radius.powi(2) as f32;
        let radial_dot_ray = (camera_radius as f32) * (-(ray_inward_cosine as f32));
        let distance_meters = distance_meters as f32;
        (camera_radius_squared
            + 2.0 * distance_meters * radial_dot_ray
            + distance_meters * distance_meters)
            .max(0.0)
            .sqrt()
    }

    #[test]
    fn stitched_face_coordinates_keep_one_gutter_and_shared_tile_edges() {
        let side = 1_u32 << FIELD_LEVEL;
        let face_quads = (TILE_LOGICAL_SIZE - 1) * side;
        assert_eq!(face_sample_source(0, side), (0, 0));
        assert_eq!(face_sample_source(1, side), (0, 1));
        assert_eq!(face_sample_source(TILE_LOGICAL_SIZE - 1, side), (0, 128));
        assert_eq!(face_sample_source(TILE_LOGICAL_SIZE, side), (1, 1));
        assert_eq!(
            face_sample_source(face_quads + 1, side),
            (side - 1, TILE_STORED_SIZE - 1)
        );
    }

    #[test]
    fn max_height_mips_cover_odd_edges_conservatively() {
        let mut base = vec![0.0; 25];
        base[4] = 7.0;
        base[24] = 11.0;
        let mips = max_height_mips(&base, 5);
        assert_eq!(mips.iter().map(Vec::len).collect::<Vec<_>>(), [25, 4, 1]);
        assert_eq!(mips[1], [0.0, 7.0, 0.0, 11.0]);
        assert_eq!(mips[2], [11.0]);
    }

    #[test]
    fn adaptive_growth_crosses_a_fixed_march_in_nine_steps() {
        let mut crossed_intervals = 0_u32;
        let mut iterations = 0_u32;
        while crossed_intervals < RayUniform::MARCH_STEPS {
            crossed_intervals += 1_u32 << iterations.min(6);
            iterations += 1;
        }
        assert_eq!(iterations, 9);
        assert!(iterations <= RayUniform::MARCH_STEPS / 20);
    }

    #[test]
    fn separable_warp_round_trips_around_an_off_center_fovea() {
        for fovea in [-0.7_f32, -0.25, 0.0, 0.4, 0.7] {
            for value in [-1.0_f32, -0.75, -0.25, -0.01, 0.0, 0.01, 0.25, 0.75, 1.0] {
                let screen = warped_screen_axis(value, fovea);
                let round_trip = unwarped_texture_axis(screen, fovea);
                assert!((round_trip - value).abs() <= 2.0e-6);
            }
            assert!((warped_screen_axis(0.0, fovea) - fovea).abs() <= f32::EPSILON);
        }
        assert!(warp_axis(0.01).abs() > 0.0);
    }

    // Must track HALFTONE_CELL_NDC in foveated_unwarp.wgsl.
    const HALFTONE_CELL_NDC: f32 = 0.06;

    fn halftone_cell_center_ndc(screen_ndc: [f32; 2], aspect_ratio: f32) -> [f32; 2] {
        let corrected = [screen_ndc[0] * aspect_ratio, screen_ndc[1]];
        let cell = [
            (corrected[0] / HALFTONE_CELL_NDC).floor(),
            (corrected[1] / HALFTONE_CELL_NDC).floor(),
        ];
        let center_corrected = [
            (cell[0] + 0.5) * HALFTONE_CELL_NDC,
            (cell[1] + 0.5) * HALFTONE_CELL_NDC,
        ];
        [center_corrected[0] / aspect_ratio, center_corrected[1]]
    }

    #[test]
    fn halftone_cells_share_a_center_and_correct_for_aspect_ratio() {
        let aspect_ratio = 1.6_f32;
        let base = halftone_cell_center_ndc([0.10, 0.10], aspect_ratio);
        let nearby = halftone_cell_center_ndc([0.105, 0.105], aspect_ratio);
        assert_eq!(
            base, nearby,
            "points inside the same cell must share a center"
        );

        // Cells are square in aspect-corrected space, so stepping one cell
        // width along screen-space x (which the shader scales by the aspect
        // ratio before quantizing) must move the center by less than a
        // step along y, exactly in proportion to the aspect ratio.
        let center_origin = halftone_cell_center_ndc([0.0, 0.0], aspect_ratio);
        let center_x_step = halftone_cell_center_ndc([HALFTONE_CELL_NDC, 0.0], aspect_ratio);
        let center_y_step = halftone_cell_center_ndc([0.0, HALFTONE_CELL_NDC], aspect_ratio);
        let x_step_ndc = center_x_step[0] - center_origin[0];
        let y_step_ndc = center_y_step[1] - center_origin[1];
        assert!((x_step_ndc - HALFTONE_CELL_NDC / aspect_ratio).abs() <= 1.0e-6);
        assert!((y_step_ndc - HALFTONE_CELL_NDC).abs() <= 1.0e-6);
    }

    #[test]
    fn fovea_easing_is_frame_rate_independent_and_returns_toward_center() {
        let one_step = eased_fovea_ndc([0.0; 2], [0.7, -0.4], 1.0, 5.0);
        let mut ten_steps = [0.0; 2];
        for _ in 0..10 {
            ten_steps = eased_fovea_ndc(ten_steps, [0.7, -0.4], 0.1, 5.0);
        }
        assert!((one_step[0] - ten_steps[0]).abs() <= 1.0e-6);
        assert!((one_step[1] - ten_steps[1]).abs() <= 1.0e-6);
        let returning = eased_fovea_ndc(one_step, [0.0; 2], 0.1, 5.0);
        assert!(returning[0].abs() < one_step[0].abs());
        assert!(returning[1].abs() < one_step[1].abs());
    }

    #[test]
    fn warp_targets_track_three_quarters_of_internal_size() {
        assert_eq!(
            warp_size_for(winit::dpi::PhysicalSize::new(640, 427)),
            winit::dpi::PhysicalSize::new(480, 320),
        );
        assert_eq!(
            warp_size_for(winit::dpi::PhysicalSize::new(1, 1)),
            winit::dpi::PhysicalSize::new(1, 1),
        );
        assert_eq!(size_of::<WarpUniform>(), 16);
        assert_eq!(size_of::<RayUniform>(), 128);
    }

    #[test]
    fn experiment_indices_map_to_independent_bits() {
        assert_eq!(experiment_flag(1), EXPERIMENT_HORIZON_DENSITY);
        assert_eq!(experiment_flag(2), EXPERIMENT_TEMPORAL_REUSE);
        assert_eq!(experiment_flag(3), EXPERIMENT_CONTENT_ADAPTIVE);
        assert_eq!(experiment_flag(4), EXPERIMENT_FOVEATED_SHADING);
        assert_eq!(experiment_flag(5), EXPERIMENT_RADIAL_BLUR);
        assert_eq!(experiment_flag(6), EXPERIMENT_HALFTONE);
        assert_eq!(
            (1..=6)
                .map(experiment_flag)
                .fold(0, |flags, flag| flags | flag),
            0b11_1111
        );
    }

    #[test]
    fn unwarp_shader_is_valid_and_writes_webgpu_reversed_depth() {
        let shader = include_str!("foveated_unwarp.wgsl");
        let module =
            wgpu::naga::front::wgsl::parse_str(shader).expect("foveated unwarp shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("foveated unwarp shader must validate");
        assert!(shader.contains("clip.z / clip.w"));
        assert!(!shader.contains("clip.z / clip.w * 0.5"));
        assert!(shader.contains("return FragmentOutput(color, 0.0)"));
    }

    #[test]
    fn full_resolution_raymarch_shader_is_valid_wgsl() {
        let shader = raymarch_shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&shader)
            .expect("full-resolution raymarch shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("full-resolution raymarch shader must validate");
        assert!(shader.contains("for (var index = 0u; index < 192u; index += 1u)"));
        assert!(shader.contains("refine_hit("));
        assert!(shader.contains("terrain_normal("));
        assert!(shader.contains("fn sample_max_height("));
        assert!(shader.contains("fn adaptive_step_distance("));
        assert!(shader.contains("exp2(f32(min(iteration, 6u)))"));
        assert!(shader.contains("fn fs_warp("));
        assert!(shader.contains(
            "+ 2.0 * distance_meters * radial_dot_ray\n            + distance_meters * distance_meters"
        ));
        assert!(shader.contains("fn shade_terrain("));
        assert!(shader.contains("let terrain_color = shade_terrain("));
        assert!(shader.contains("fn ocean_hit("));
        assert!(shader.contains("let water_hit = ocean_hit("));
        assert!(shader.contains("ray_atmosphere_radiance("));
        assert!(shader.contains("RENDER_DEBUG_SKY_ONLY"));
        assert!(shader.contains("@builtin(frag_depth) depth: f32"));
    }

    #[test]
    fn ray_shell_bounds_use_the_shared_altitude_height_scale() {
        let near = RayUniform::for_camera(-5_000.0, 9_000.0, 2_048, 12, 10_000.0);
        let far = RayUniform::for_camera(-5_000.0, 9_000.0, 2_048, 12, 2_000_000.0);
        assert_eq!(near.march_steps, 192);
        assert_eq!(near.max_height_mip_count, 12);
        assert_eq!(near.minimum_step_meters, 1_953.125);
        assert_eq!(near.minimum_shell_radius_meters, 3_995_000.0);
        assert_eq!(near.maximum_shell_radius_meters, 4_009_000.0);
        assert_eq!(far.minimum_shell_radius_meters, 3_980_000.0);
        assert_eq!(far.maximum_shell_radius_meters, 4_036_000.0);
        assert_eq!(near.camera_radius_meters, 4_010_000.0);
        assert_eq!(near.camera_radius_squared, 4_010_000.0_f32.powi(2));
    }

    #[test]
    fn quadratic_radius_stays_sub_meter_at_low_altitude() {
        for altitude_meters in [0.1, 1.0, 10.0, 1_700.0, 10_000.0] {
            let camera_radius = PLANET_RADIUS_METERS + altitude_meters;
            for angle_degrees in [0.0_f64, 30.0, 60.0, 80.0, 88.0, 89.0] {
                let ray_inward_cosine = angle_degrees.to_radians().cos();
                let Some(distance_meters) =
                    sphere_entry_distance_f64(camera_radius, ray_inward_cosine)
                else {
                    continue;
                };
                let radius =
                    shader_radius_at_surface(camera_radius, ray_inward_cosine, distance_meters);
                let error_meters = (f64::from(radius) - PLANET_RADIUS_METERS).abs();
                assert!(
                    error_meters <= 0.5,
                    "altitude={altitude_meters}m angle={angle_degrees}deg error={error_meters}m"
                );
            }
        }
    }

    #[test]
    fn camera_radius_squared_is_rounded_only_after_f64_multiplication() {
        let uniform = RayUniform::for_camera(-5_000.0, 9_000.0, 2_048, 12, 0.1);
        let camera_radius = PLANET_RADIUS_METERS + 0.1;
        assert_eq!(uniform.camera_radius_squared, camera_radius.powi(2) as f32);
        assert_ne!(
            uniform.camera_radius_squared,
            (camera_radius as f32).powi(2),
            "squaring after the f32 cast discards the low-altitude contribution"
        );
    }
}

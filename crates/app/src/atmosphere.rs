use std::borrow::Cow;

const TRANSMITTANCE_LUT_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 256,
    height: 64,
    depth_or_array_layers: 1,
};
const MULTIPLE_SCATTERING_LUT_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 32,
    height: 32,
    depth_or_array_layers: 1,
};
const SKY_VIEW_LUT_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 256,
    height: 128,
    depth_or_array_layers: 1,
};
const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct AtmosphereRenderer {
    pipeline: wgpu::RenderPipeline,
    sky_view_pipeline: wgpu::RenderPipeline,
    sky_view: wgpu::TextureView,
    physical_luts_bind_group: wgpu::BindGroup,
    sky_view_bind_group: wgpu::BindGroup,
}

impl AtmosphereRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let physical_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("physical atmosphere LUT sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let sky_view_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("physical atmosphere sky-view sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let transmittance = create_lut(
            device,
            "physical atmosphere transmittance LUT",
            TRANSMITTANCE_LUT_SIZE,
        );
        let multiple_scattering = create_lut(
            device,
            "physical atmosphere multiple-scattering LUT",
            MULTIPLE_SCATTERING_LUT_SIZE,
        );
        let sky_view_texture = create_lut(
            device,
            "physical atmosphere sky-view LUT",
            SKY_VIEW_LUT_SIZE,
        );
        let transmittance_view = transmittance.create_view(&wgpu::TextureViewDescriptor::default());
        let multiple_scattering_view =
            multiple_scattering.create_view(&wgpu::TextureViewDescriptor::default());
        let sky_view = sky_view_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let physical_luts_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("physical atmosphere LUT layout"),
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
        let physical_luts_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical atmosphere LUT bind group"),
            layout: &physical_luts_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&multiple_scattering_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&physical_sampler),
                },
            ],
        });
        let multiple_scattering_input_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("physical atmosphere multiple-scattering input layout"),
                entries: &[
                    texture_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let multiple_scattering_input = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical atmosphere multiple-scattering input"),
            layout: &multiple_scattering_input_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&physical_sampler),
                },
            ],
        });

        let sky_view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("physical atmosphere sky-view layout"),
            entries: &[
                texture_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sky_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical atmosphere sky-view bind group"),
            layout: &sky_view_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sky_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sky_view_sampler),
                },
            ],
        });

        let common = include_str!("atmosphere_lut_common.wgsl");
        let transmittance_shader = shader_module(
            device,
            "physical atmosphere transmittance shader",
            common,
            include_str!("atmosphere_transmittance.wgsl"),
        );
        let multiple_scattering_shader = shader_module(
            device,
            "physical atmosphere multiple-scattering shader",
            common,
            include_str!("atmosphere_multiscattering.wgsl"),
        );
        let sky_view_shader = shader_module(
            device,
            "physical atmosphere sky-view shader",
            common,
            include_str!("atmosphere_sky_view.wgsl"),
        );
        let display_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("physical atmosphere display shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("atmosphere.wgsl"))),
        });

        let transmittance_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("physical atmosphere transmittance pipeline layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });
        let transmittance_pipeline = lut_pipeline(
            device,
            "physical atmosphere transmittance pipeline",
            &transmittance_pipeline_layout,
            &transmittance_shader,
        );
        let multiple_scattering_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("physical atmosphere multiple-scattering pipeline layout"),
                bind_group_layouts: &[Some(&multiple_scattering_input_layout)],
                immediate_size: 0,
            });
        let multiple_scattering_pipeline = lut_pipeline(
            device,
            "physical atmosphere multiple-scattering pipeline",
            &multiple_scattering_pipeline_layout,
            &multiple_scattering_shader,
        );
        let sky_view_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("physical atmosphere sky-view pipeline layout"),
                bind_group_layouts: &[Some(camera_bind_group_layout), Some(&physical_luts_layout)],
                immediate_size: 0,
            });
        let sky_view_pipeline = lut_pipeline(
            device,
            "physical atmosphere sky-view pipeline",
            &sky_view_pipeline_layout,
            &sky_view_shader,
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("physical atmosphere display pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout), Some(&sky_view_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("physical LUT atmosphere pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &display_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &display_shader,
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("physical atmosphere static LUT encoder"),
        });
        draw_lut(
            &mut encoder,
            "physical atmosphere transmittance LUT pass",
            &transmittance_view,
            &transmittance_pipeline,
            None,
        );
        draw_lut(
            &mut encoder,
            "physical atmosphere multiple-scattering LUT pass",
            &multiple_scattering_view,
            &multiple_scattering_pipeline,
            Some(&multiple_scattering_input),
        );
        queue.submit(Some(encoder.finish()));

        Self {
            pipeline,
            sky_view_pipeline,
            sky_view,
            physical_luts_bind_group,
            sky_view_bind_group,
        }
    }

    pub fn update(&self, encoder: &mut wgpu::CommandEncoder, camera_bind_group: &wgpu::BindGroup) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("physical atmosphere sky-view LUT pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.sky_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        render_pass.set_pipeline(&self.sky_view_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.physical_luts_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.sky_view_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

fn create_lut(device: &wgpu::Device, label: &'static str, size: wgpu::Extent3d) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: LUT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
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

fn shader_module(
    device: &wgpu::Device,
    label: &'static str,
    common: &str,
    body: &str,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{common}\n{body}"))),
    })
}

fn lut_pipeline(
    device: &wgpu::Device,
    label: &'static str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: LUT_FORMAT,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn draw_lut(
    encoder: &mut wgpu::CommandEncoder,
    label: &'static str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    bind_group: Option<&wgpu::BindGroup>,
) {
    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    });
    render_pass.set_pipeline(pipeline);
    if let Some(bind_group) = bind_group {
        render_pass.set_bind_group(0, bind_group, &[]);
    }
    render_pass.draw(0..3, 0..1);
}

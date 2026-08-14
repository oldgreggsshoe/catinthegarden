pub struct SunRenderer {
    pipeline: wgpu::RenderPipeline,
    atmosphere_bind_group: wgpu::BindGroup,
}

impl SunRenderer {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        atmosphere: crate::atmosphere::SurfaceLightingResources<'_>,
    ) -> Self {
        let atmosphere_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sun atmosphere transmittance layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let atmosphere_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun atmosphere transmittance bind group"),
            layout: &atmosphere_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atmosphere.transmittance),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(atmosphere.physical_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sun disc pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&atmosphere_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("sun.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sun disc pipeline"),
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
                    format: hdr_format,
                    // Keep the warm disc and halo overbright in the HDR scene,
                    // instead of replacing the sky color beneath the halo.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            // Draw after the physical scene and its luminance meter. Equal
            // matches only the untouched reversed-Z clear depth (0.0), so the
            // planet still occludes the visual-only disc without letting it
            // influence exposure.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Equal),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            atmosphere_bind_group,
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
        render_pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn visible_sun_uses_the_surface_transmittance_lut() {
        let shader = include_str!("sun.wgsl");
        assert!(shader.contains("var atmosphere_transmittance_lut: texture_2d<f32>;"));
        assert!(shader.contains("textureSampleLevel("));
        assert!(shader.contains("sun_disc_atmospheric_transmittance(solar_elevation)"));
        assert!(shader.contains("const SUN_HORIZON_LUT_ELEVATION_RADIANS: f32 = -0.05;"));
        assert!(shader.contains("solar_elevation - 0.05"));
        assert!(!shader.contains("vec3<f32>(1.0, 0.48, 0.16)"));
    }

    #[test]
    fn visible_sun_keeps_angular_disc_size_when_glare_dims_at_low_sun() {
        let shader = include_str!("sun.wgsl");
        let compact: String = shader.split_whitespace().collect();
        assert!(shader.contains("const SUN_CORE_VISIBILITY_FLOOR: f32 = 0.12;"));
        assert!(shader.contains("const SUN_CORE_RADIANCE_FLOOR: f32 = 0.50;"));
        assert!(shader.contains("const SUN_GLARE_VISIBILITY_FLOOR: f32 = 0.18;"));
        assert!(shader.contains("let presentation_tint = tint"));
        assert!(shader.contains("let limb_tint = mix("));
        assert!(shader.contains("let core_radiance_scale = mix("));
        assert!(shader.contains("SUN_CORE_RADIANCE_FLOOR"));
        assert!(shader.contains("let atmospheric_core = core_radiance_scale * core_tint * ("));
        assert!(
            shader.contains("let atmospheric_glare = presentation_tint * glare_visibility * (")
        );
        assert!(compact.contains(
            "letradiance=SUN_VISUAL_RADIANCE_SCALE*(atmospheric_core+atmospheric_glare);"
        ));
        assert!(!compact.contains("letradiance=SUN_VISUAL_RADIANCE_SCALE*tint*(SUN_CORE_RADIANCE"));
    }
}

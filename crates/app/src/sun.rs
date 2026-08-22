pub struct SunRenderer {
    pipeline: wgpu::RenderPipeline,
    atmosphere_bind_group: wgpu::BindGroup,
}

fn sun_shader_source() -> String {
    format!(
        "{}\n{}",
        include_str!("sun.wgsl"),
        include_str!("weather_cloud_density.wgsl"),
    )
}

impl SunRenderer {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        weather_field_bind_group_layout: &wgpu::BindGroupLayout,
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
                Some(weather_field_bind_group_layout),
                Some(&atmosphere_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun disc shader"),
            source: wgpu::ShaderSource::Wgsl(sun_shader_source().into()),
        });
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
            // Draw after the physical scene and its luminance meter, only
            // where the depth buffer still contains the reversed-Z far value.
            // Terrain and the solid planet therefore continue to occlude it.
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
        weather_field_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, weather_field_bind_group, &[]);
        render_pass.set_bind_group(2, &self.atmosphere_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn orbital_sun_misses_atmosphere_and_uses_full_brightness() {
        let camera = glam::DVec3::new(
            -812_390.069_260_471_5,
            -10_188_038.341_751_29,
            -15_094_498.977_686_338,
        );
        let sun = glam::DVec3::new(
            0.508_927_518_800_963_9,
            0.397_776_994_021_848,
            0.763_391_278_201_445_7,
        );
        let closest_approach = camera.cross(sun).length();
        let atmosphere_radius = 4_000_000.0 + 2_880_000.0;
        assert!(
            closest_approach > atmosphere_radius,
            "captured orbital sun ray unexpectedly enters the atmosphere"
        );

        let shader = super::sun_shader_source();
        assert!(shader.contains("fn sun_disc_atmosphere_sample("));
        assert!(shader.contains("SunAtmosphereSample(vec3<f32>(1.0), 1.0)"));
    }

    #[test]
    fn visible_sun_uses_the_surface_transmittance_lut() {
        let shader = include_str!("sun.wgsl");
        assert!(shader.contains("var atmosphere_transmittance_lut: texture_2d<f32>;"));
        assert!(shader.contains("textureSampleLevel("));
        assert!(shader.contains("sun_disc_atmosphere_sample(solar_elevation)"));
        assert!(shader.contains("relative_sun_transmittance(camera_altitude, solar_elevation)"));
        assert!(shader.contains("const SUN_HORIZON_LUT_ELEVATION: f32 = 0.05;"));
        assert!(shader.contains("max(solar_elevation, 0.0)"));
        assert!(shader.contains("+ SUN_HORIZON_LUT_ELEVATION"));
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
        assert!(shader.contains("var core_hue = vec3<f32>(1.0, 0.08, 0.01);"));
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

    #[test]
    fn visible_sun_disc_controls_the_halo_after_planet_occultation() {
        let shader = super::sun_shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&shader)
            .expect("sun shader must parse before WGPU creates the pipeline");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("sun shader must validate before WGPU creates the pipeline");
        assert!(shader.contains("fn sun_disc_is_fully_occulted() -> bool"));
        assert!(
            shader.contains("center_angle + SUN_ANGULAR_RADIUS_RADIANS <= planet_angular_radius")
        );
        assert!(shader.contains("if sun_disc_is_fully_occulted()"));
        assert!(shader.contains("discard;"));
    }

    #[test]
    fn visible_sun_samples_shared_cloud_density_along_the_camera_ray() {
        let shader = include_str!("sun.wgsl");
        let renderer = include_str!("sun.rs");
        assert!(shader.contains("var cloud_field_current: texture_cube<f32>;"));
        assert!(shader.contains("fn cloud_sun_visibility("));
        assert!(shader.contains("cloudDensityWithOctaves(cloud_direction, shell_index, 3u)"));
        assert!(shader.contains("if cloud_opacity >= 0.60"));
        assert!(shader.contains("pow(geometric_transmission, 4.0)"));
        assert!(shader.contains("cloud_sun_visibility(sun)"));
        assert!(shader.contains("radiance * cloud_visibility"));
        assert!(renderer.contains("weather_field_bind_group_layout"));
        assert!(renderer.contains("weather_field_bind_group"));
    }
}

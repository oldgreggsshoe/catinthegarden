pub struct SunRenderer {
    disc_pipeline: wgpu::RenderPipeline,
    flare_pipeline: wgpu::RenderPipeline,
    atmosphere_bind_group: wgpu::BindGroup,
    depth_bind_group_layout: wgpu::BindGroupLayout,
    depth_bind_group: wgpu::BindGroup,
}

pub(crate) fn sun_shader_source() -> String {
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
        depth_view: &wgpu::TextureView,
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
        let depth_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sun terrain-occlusion depth layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
        let depth_bind_group =
            create_depth_bind_group(device, &depth_bind_group_layout, depth_view);
        let disc_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sun disc pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(weather_field_bind_group_layout),
                Some(&atmosphere_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let flare_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sun optical flare pipeline layout"),
                bind_group_layouts: &[
                    Some(camera_bind_group_layout),
                    Some(weather_field_bind_group_layout),
                    Some(&atmosphere_bind_group_layout),
                    Some(&depth_bind_group_layout),
                ],
                immediate_size: 0,
            });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun disc shader"),
            source: wgpu::ShaderSource::Wgsl(sun_shader_source().into()),
        });
        let create_pipeline = |label, layout, fragment_entry, depth_stencil| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: hdr_format,
                        // Both contributions are camera-only HDR additions;
                        // neither replaces the physical sky beneath it.
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
                depth_stencil,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The visual disc keeps the original terrain/planet depth test.
        let disc_pipeline = create_pipeline(
            "sun disc pipeline",
            &disc_pipeline_layout,
            "fs_disc",
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Equal),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
        );
        // Optical flare is a full-frame camera response and therefore has no
        // per-fragment depth test. Its shader samples the completed scene depth
        // to retain the whole flare if any part of the disc is visible, or
        // remove it when terrain covers the complete source.
        let flare_pipeline = create_pipeline(
            "sun optical flare pipeline",
            &flare_pipeline_layout,
            "fs_flare",
            None,
        );
        Self {
            disc_pipeline,
            flare_pipeline,
            atmosphere_bind_group,
            depth_bind_group_layout,
            depth_bind_group,
        }
    }

    pub fn resize_depth(&mut self, device: &wgpu::Device, depth_view: &wgpu::TextureView) {
        self.depth_bind_group =
            create_depth_bind_group(device, &self.depth_bind_group_layout, depth_view);
    }

    fn bind_shared<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        weather_field_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, weather_field_bind_group, &[]);
        render_pass.set_bind_group(2, &self.atmosphere_bind_group, &[]);
    }

    pub fn draw_disc<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        weather_field_bind_group: &'pass wgpu::BindGroup,
    ) {
        self.bind_shared(render_pass, camera_bind_group, weather_field_bind_group);
        render_pass.set_pipeline(&self.disc_pipeline);
        render_pass.draw(0..3, 0..1);
    }

    pub fn draw_flare<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        weather_field_bind_group: &'pass wgpu::BindGroup,
    ) {
        self.bind_shared(render_pass, camera_bind_group, weather_field_bind_group);
        render_pass.set_bind_group(3, &self.depth_bind_group, &[]);
        render_pass.set_pipeline(&self.flare_pipeline);
        render_pass.draw(0..3, 0..1);
    }
}

fn create_depth_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    depth_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sun terrain-occlusion depth bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(depth_view),
        }],
    })
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
        assert!(shader.contains("const SUN_GLARE_VISIBILITY_FLOOR: f32 = 0.08;"));
        assert!(shader.contains("const VISUAL_SUN_SIZE_SCALE: f32 = 2.0;"));
        assert!(shader.contains("const SUN_HALO_RADIUS_SCALE: f32 = 3.25;"));
        assert!(shader.contains("const SUN_VEILING_GLARE_RADIUS_SCALE: f32 = 15.0;"));
        assert!(shader.contains("const SUN_STAR_RAY_RADIUS_SCALE: f32 = 21.0;"));
        assert!(shader.contains("const SUN_OVERLAY_CUTOFF_RADIUS_SCALE: f32 = 32.0;"));
        assert!(shader.contains("let veiling_glare = pow("));
        assert!(shader.contains("let major_star_rays = pow("));
        assert!(shader.contains("let minor_star_rays = pow("));
        assert!(shader.contains("SUN_STAR_RAY_RADIANCE * star_rays"));
        assert!(shader.contains("if normalized_distance > SUN_OVERLAY_CUTOFF_RADIUS_SCALE"));
        assert!(shader.contains("SUN_VEILING_GLARE_RADIANCE * veiling_glare"));
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
            "letradiance=SUN_VISUAL_RADIANCE_SCALE*select(atmospheric_glare*glare_cloud_visibility,atmospheric_core*cloud_visibility,draw_disc,);"
        ));
        assert!(!compact.contains("letradiance=SUN_VISUAL_RADIANCE_SCALE*tint*(SUN_CORE_RADIANCE"));
    }

    #[test]
    fn camera_flare_is_centered_and_has_no_coloured_axis_ghosts() {
        let shader = include_str!("sun.wgsl");
        assert!(!shader.contains("lens_ghost"));
        assert!(!shader.contains("purple_ghost"));
        assert!(!shader.contains("cyan_ghost"));
        assert!(shader.contains("SUN_HALO_RADIANCE * halo"));
        assert!(shader.contains("SUN_STAR_RAY_RADIANCE * star_rays"));
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
    fn partial_occultation_clips_only_the_physical_disc() {
        let renderer = include_str!("sun.rs");
        let shader = include_str!("sun.wgsl");
        assert!(renderer.contains("disc_pipeline: wgpu::RenderPipeline"));
        assert!(renderer.contains("flare_pipeline: wgpu::RenderPipeline"));
        assert!(renderer.contains(
            "\"sun disc pipeline\",\n            &disc_pipeline_layout,\n            \"fs_disc\","
        ));
        assert!(renderer.contains(
            "\"sun optical flare pipeline\",\n            &flare_pipeline_layout,\n            \"fs_flare\",\n            None,"
        ));
        assert!(shader.contains("fn sun_radiance(input: VertexOutput, draw_disc: bool)"));
        assert!(shader.contains("fn fs_disc(input: VertexOutput)"));
        assert!(shader.contains("fn fs_flare(input: VertexOutput)"));
    }

    #[test]
    fn local_terrain_occlusion_gates_the_complete_optical_flare() {
        let renderer = include_str!("sun.rs");
        let shader = include_str!("sun.wgsl");
        assert!(shader.contains("var scene_depth: texture_depth_2d;"));
        assert!(shader.contains("fn sun_disc_has_visible_depth() -> bool"));
        assert!(shader.contains("if !sun_disc_has_visible_depth()"));
        let flare = shader
            .split("fn fs_flare(input: VertexOutput)")
            .nth(1)
            .expect("flare entry exists");
        assert!(
            flare.find("SUN_OVERLAY_CUTOFF_RADIUS_SCALE").unwrap()
                < flare.find("sun_disc_has_visible_depth()").unwrap(),
            "off-flare pixels must discard before the multi-sample depth probe",
        );
        assert!(renderer.contains("pub fn draw_disc<'pass>("));
        assert!(renderer.contains("pub fn draw_flare<'pass>("));
        assert!(renderer.contains("depth_stencil: None"));
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
        assert!(shader.contains("atmospheric_core * cloud_visibility"));
        assert!(shader.contains("let glare_cloud_visibility = cloud_visibility;"));
        assert!(!shader.contains("pow(cloud_visibility, 4.0)"));
        assert!(shader.contains("atmospheric_glare * glare_cloud_visibility"));
        assert!(renderer.contains("weather_field_bind_group_layout"));
        assert!(renderer.contains("weather_field_bind_group"));
    }
}

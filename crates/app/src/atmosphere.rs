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
const IRRADIANCE_LUT_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 64,
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
    transmittance: wgpu::TextureView,
    sky_view: wgpu::TextureView,
    surface_irradiance: wgpu::TextureView,
    physical_sampler: wgpu::Sampler,
    sky_view_sampler: wgpu::Sampler,
    physical_luts_bind_group: wgpu::BindGroup,
    sky_view_bind_group: wgpu::BindGroup,
}

pub struct SurfaceLightingResources<'a> {
    pub transmittance: &'a wgpu::TextureView,
    pub irradiance: &'a wgpu::TextureView,
    pub physical_sampler: &'a wgpu::Sampler,
    pub sky_view: &'a wgpu::TextureView,
    pub sky_view_sampler: &'a wgpu::Sampler,
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
        let surface_irradiance_texture = create_lut(
            device,
            "physical atmosphere surface-irradiance LUT",
            IRRADIANCE_LUT_SIZE,
        );
        let sky_view_texture = create_lut(
            device,
            "physical atmosphere sky-view LUT",
            SKY_VIEW_LUT_SIZE,
        );
        let transmittance_view = transmittance.create_view(&wgpu::TextureViewDescriptor::default());
        let multiple_scattering_view =
            multiple_scattering.create_view(&wgpu::TextureViewDescriptor::default());
        let surface_irradiance =
            surface_irradiance_texture.create_view(&wgpu::TextureViewDescriptor::default());
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
        let irradiance_shader = shader_module(
            device,
            "physical atmosphere surface-irradiance shader",
            common,
            include_str!("atmosphere_irradiance.wgsl"),
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
        let irradiance_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("physical atmosphere surface-irradiance pipeline layout"),
                bind_group_layouts: &[Some(&physical_luts_layout)],
                immediate_size: 0,
            });
        let irradiance_pipeline = lut_pipeline(
            device,
            "physical atmosphere surface-irradiance pipeline",
            &irradiance_pipeline_layout,
            &irradiance_shader,
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
        draw_lut(
            &mut encoder,
            "physical atmosphere surface-irradiance LUT pass",
            &surface_irradiance,
            &irradiance_pipeline,
            Some(&physical_luts_bind_group),
        );
        queue.submit(Some(encoder.finish()));

        Self {
            pipeline,
            sky_view_pipeline,
            transmittance: transmittance_view,
            sky_view,
            surface_irradiance,
            physical_sampler,
            sky_view_sampler,
            physical_luts_bind_group,
            sky_view_bind_group,
        }
    }

    pub fn surface_lighting_resources(&self) -> SurfaceLightingResources<'_> {
        SurfaceLightingResources {
            transmittance: &self.transmittance,
            irradiance: &self.surface_irradiance,
            physical_sampler: &self.physical_sampler,
            sky_view: &self.sky_view,
            sky_view_sampler: &self.sky_view_sampler,
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

#[cfg(test)]
mod tests {
    #[test]
    fn sky_view_generation_and_display_share_the_vertical_convention() {
        for (label, shader) in [
            (
                "transmittance",
                include_str!("atmosphere_transmittance.wgsl"),
            ),
            (
                "multiple scattering",
                include_str!("atmosphere_multiscattering.wgsl"),
            ),
            (
                "surface irradiance",
                include_str!("atmosphere_irradiance.wgsl"),
            ),
            ("sky view", include_str!("atmosphere_sky_view.wgsl")),
        ] {
            assert!(
                shader.contains("0.5 - position.y * 0.5"),
                "{label} LUT would be stored upside-down relative to texture sampling",
            );
        }

        let display = include_str!("atmosphere.wgsl");
        assert!(display.contains("ndc.y * camera.projection.y"));
        assert!(!display.contains("-ndc.y * camera.projection.y"));

        let sky_view = include_str!("atmosphere_sky_view.wgsl");
        assert!(sky_view.contains("PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS"));
        assert!(sky_view.contains("world_atmosphere_interval.y"));
        assert!(sky_view.contains("optical_camera_altitude"));
        assert!(sky_view.contains("fn optical_zenith_cosine("));
        assert!(sky_view.contains("optical_zenith_cosine(dot(sun, up)"));
        assert!(sky_view.contains("world_ray"));
        for declaration in [
            "const ORBITAL_GEOMETRY_BLEND_START_METERS: f32 = 200000.0;",
            "const ORBITAL_GEOMETRY_BLEND_END_METERS: f32 = 400000.0;",
        ] {
            assert!(sky_view.contains(declaration));
            assert!(display.contains(declaration));
        }
        assert!(sky_view.contains("fn integrate_world_space_sky("));
        assert!(sky_view.contains("PLANET_RADIUS_METERS + OPTICAL_ATMOSPHERE_HEIGHT_METERS"));
        assert!(
            display.contains("mix(perceptual_sky_radiance(radiance), radiance, orbital_blend)")
        );
    }

    #[test]
    fn visible_sky_gain_is_presentation_only() {
        let display = include_str!("atmosphere.wgsl");
        assert!(display.contains("const VISIBLE_SKY_RADIANCE_SCALE: f32 = 2.0;"));
        assert!(display.contains(
            "let visible_radiance = VISIBLE_SKY_RADIANCE_SCALE\n        * mix(perceptual_sky_radiance(radiance), radiance, orbital_blend);"
        ));
        for physical_stage in [
            include_str!("atmosphere_lut_common.wgsl"),
            include_str!("atmosphere_transmittance.wgsl"),
            include_str!("atmosphere_multiscattering.wgsl"),
            include_str!("atmosphere_sky_view.wgsl"),
            include_str!("atmosphere_irradiance.wgsl"),
            include_str!("shared_planet.wgsl"),
        ] {
            assert!(!physical_stage.contains("VISIBLE_SKY_RADIANCE_SCALE"));
        }
    }

    #[test]
    fn visible_sky_mist_uses_the_terrain_air_column_and_physical_horizon_colour() {
        let display = include_str!("atmosphere.wgsl");
        let terrain = include_str!("shared_planet.wgsl");
        for declaration in [
            "const RAYLEIGH_SCALE_HEIGHT_METERS: f32 = 72000.0;",
            "const TERRAIN_FOG_AIR_PATH_E_FOLD_METERS: f32 = 500000.0;",
        ] {
            assert!(display.contains(declaration));
            assert!(terrain.contains(declaration));
        }
        assert!(display.contains("fn sky_fog_air_path_meters(ray: vec3<f32>) -> f32"));
        assert!(
            display.contains("let closest_altitude = max(closest_radius - PLANET_RADIUS_METERS")
        );
        assert!(display.contains("let descent_amount = 1.0 - exp("));
        assert!(display.contains("let horizon_radiance = textureSample("));
        assert!(display.contains("let fog_amount = 1.0 - exp("));
        assert!(display.contains("mix(visible_radiance, horizon_fog_radiance, fog_amount)"));
    }

    #[test]
    fn sky_mist_air_path_is_grazing_weighted_without_an_altitude_switch() {
        const PLANET_RADIUS_METERS: f64 = 4_000_000.0;
        const SCALE_HEIGHT_METERS: f64 = 72_000.0;
        const E_FOLD_METERS: f64 = 500_000.0;
        // The reported capture was 144.6km clear of its unusually tall local
        // terrain, but the atmosphere uniform correctly carries 177.7km datum
        // altitude.
        let camera_altitude_meters: f64 = 177_661.059_391_079;
        let camera_radius = PLANET_RADIUS_METERS + camera_altitude_meters;
        let camera_density = (-camera_altitude_meters / SCALE_HEIGHT_METERS).exp();
        let fog_amount = |air_path_meters: f64| 1.0 - (-air_path_meters / E_FOLD_METERS).exp();

        // A radial sky ray leaves the atmosphere through the locally thin air.
        let radial_air_path = camera_density * SCALE_HEIGHT_METERS;
        // The ground-horizon ray descends through dense air before leaving the
        // atmosphere, so it reaches the same bounded 12-air-mass column used
        // by terrain mist. There is no camera-altitude multiplier to disable it.
        let ground_horizon_cosine = -(1.0 - (PLANET_RADIUS_METERS / camera_radius).powi(2)).sqrt();
        let closest_radius =
            camera_radius * (1.0 - ground_horizon_cosine * ground_horizon_cosine).sqrt();
        let closest_density =
            (-(closest_radius - PLANET_RADIUS_METERS) / SCALE_HEIGHT_METERS).exp();
        let descent_amount = 1.0
            - (-(camera_altitude_meters - (closest_radius - PLANET_RADIUS_METERS))
                / SCALE_HEIGHT_METERS)
                .exp();
        let grazing_density = 0.5 * camera_density * (1.0 - descent_amount)
            + 0.5 * (camera_density + closest_density) * descent_amount;
        let grazing_air_path = grazing_density * 2.0 * SCALE_HEIGHT_METERS * 12.0;
        let radial_fog = fog_amount(radial_air_path);
        let grazing_fog = fog_amount(grazing_air_path);

        assert!(radial_fog < 0.03, "radial sky fog {radial_fog}");
        assert!(
            (0.75..0.90).contains(&grazing_fog),
            "grazing sky fog {grazing_fog}"
        );
        assert!(grazing_fog > radial_fog * 20.0);

        // Crossing the local horizontal must not introduce another visible
        // atmosphere band. The closest-density contribution fades in with
        // actual descent rather than switching on for every negative cosine.
        let horizontal_air_path = camera_density * SCALE_HEIGHT_METERS * 12.0;
        let just_down_cosine = -1.0e-4_f64;
        let just_down_radius = camera_radius * (1.0 - just_down_cosine * just_down_cosine).sqrt();
        let just_down_altitude = just_down_radius - PLANET_RADIUS_METERS;
        let just_down_density = (-just_down_altitude / SCALE_HEIGHT_METERS).exp();
        let just_down_amount =
            1.0 - (-(camera_altitude_meters - just_down_altitude) / SCALE_HEIGHT_METERS).exp();
        let just_down_average = 0.5 * camera_density * (1.0 - just_down_amount)
            + 0.5 * (camera_density + just_down_density) * just_down_amount;
        let just_down_air_path = just_down_average * 2.0 * SCALE_HEIGHT_METERS * 12.0;
        assert!((just_down_air_path - horizontal_air_path).abs() / horizontal_air_path < 1.0e-5);
    }

    #[test]
    fn orbital_sky_view_reserves_rows_for_the_atmosphere_band() {
        let generation = include_str!("atmosphere_sky_view.wgsl");
        let display = include_str!("atmosphere.wgsl");
        let surface = include_str!("shared_planet.wgsl");
        for declaration in [
            "const ORBITAL_ATMOSPHERE_LUT_V: f32 = 0.72;",
            "const ORBITAL_GROUND_LUT_V: f32 = 0.88;",
        ] {
            assert!(generation.contains(declaration));
            assert!(display.contains(declaration));
        }
        assert!(generation.contains("fn sky_view_zenith_cosine_from_v("));
        assert!(display.contains("fn sky_view_v_from_zenith_cosine("));
        assert!(
            generation.contains("let world_view_zenith_cosine = sky_view_zenith_cosine_from_v(")
        );
        assert!(display.contains("sky_view_v_from_zenith_cosine("));
        assert!(display.contains("view_zenith_cosine,"));
        assert!(surface.contains("fn physical_sky_view_v_from_zenith_cosine("));
        assert!(surface.contains("physical_sky_view_v_from_zenith_cosine("));

        // At the furthest reported altitude, linear cosine mapping allocates
        // much less than one texel to the whole visible atmosphere. The
        // horizon-focused orbital mapping must retain a stable multi-row band.
        let camera_radius = 4_000_000.0_f64 + 15_000_000.0;
        let ground_cosine = -(1.0 - (4_000_000.0 / camera_radius).powi(2)).sqrt();
        let atmosphere_cosine = -(1.0 - (4_640_000.0 / camera_radius).powi(2)).sqrt();
        let linear_rows = (atmosphere_cosine - ground_cosine) * 0.5 * 128.0;
        assert!(
            linear_rows < 1.0,
            "repro requires sub-texel linear coverage"
        );
        let orbital_rows = (0.88 - 0.72) * 128.0;
        assert!(orbital_rows >= 16.0);
    }

    #[test]
    fn orbital_density_profile_keeps_the_lower_cloud_shell_inside_visible_air() {
        let sky_view = include_str!("atmosphere_sky_view.wgsl");
        assert!(sky_view.contains(
            "let optical_sample_altitude = sample_altitude / ATMOSPHERE_VERTICAL_SCALE;"
        ));
        assert!(sky_view.contains(
            "let optical_segment_length = segment_length / ATMOSPHERE_VERTICAL_SCALE;"
        ));
        assert!(sky_view.contains("PLANET_RADIUS_METERS + ATMOSPHERE_HEIGHT_METERS"));
        assert!(sky_view.contains("medium_extinction(optical_sample_altitude)"));
        assert!(sky_view.contains("medium_scattering(optical_sample_altitude)"));

        // The game cloud shell is vertically exaggerated with the atmosphere.
        // In an orbital view its 90 km world altitude therefore maps to the
        // same 20 km optical column used by the signed-off surface path.
        let optical_cloud_altitude = 90_000.0_f64 / 4.5;
        let rayleigh_density = (-optical_cloud_altitude / 8_000.0).exp();
        let unscaled_density = (-90_000.0_f64 / 8_000.0).exp();
        assert!(rayleigh_density > 0.08);
        assert!(rayleigh_density > unscaled_density * 1_000.0);
    }

    #[test]
    fn doubled_optical_atmosphere_covers_the_presented_mountain_summit() {
        let common = include_str!("atmosphere_lut_common.wgsl");
        let display = include_str!("atmosphere.wgsl");
        let surface = include_str!("shared_planet.wgsl");
        let sun = include_str!("sun.wgsl");
        assert!(common.contains("const ATMOSPHERE_VERTICAL_SCALE: f32 = 4.5;"));
        assert!(common.contains("const OPTICAL_ATMOSPHERE_EDGE_FADE_METERS: f32 = 426666.668;"));
        assert!(display.contains("const OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 640000.0;"));
        assert!(
            surface.contains("const SKY_VIEW_OPTICAL_ATMOSPHERE_HEIGHT_METERS: f32 = 640000.0;")
        );
        assert!(sun.contains("/ 4.5;"));
        assert!(sun.contains("optical_altitude / 640000.0"));
        assert!(640_000.0_f32 > 180_943.3_f32);
        for (stage, sample_count) in [
            (include_str!("atmosphere_transmittance.wgsl"), "80u"),
            (include_str!("atmosphere_multiscattering.wgsl"), "40u"),
            (include_str!("atmosphere_sky_view.wgsl"), "40u"),
            (include_str!("atmosphere_irradiance.wgsl"), "24u"),
        ] {
            assert!(
                stage.contains(sample_count),
                "doubled integration range lost its proportional sample budget"
            );
        }
    }

    #[test]
    fn sunset_redness_comes_from_physical_ozone_absorption() {
        let common = include_str!("atmosphere_lut_common.wgsl");
        assert!(common.contains("const OZONE_COLUMN_SCALE: f32 = 1.5;"));
        assert!(
            common
                .contains("OZONE_ABSORPTION * ozone_density(altitude_meters) * OZONE_COLUMN_SCALE")
        );
        for authored_tint in ["sunset_tint", "red_tint", "horizon_tint"] {
            assert!(!common.contains(authored_tint));
        }
    }
}

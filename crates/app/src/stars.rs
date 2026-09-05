//! An immutable, observer-centred celestial catalogue, not a sky texture.
//! Galactic positions are projected from inside an off-centre spiral disc.
//! Directions stay inertial; only the camera/planet transform changes each frame.
use glam::Vec3;
use wgpu::util::DeviceExt;

const FIELD_STARS: usize = 6_000;
const DISC_STARS: usize = 9_000;
const GALAXY_PATCHES: usize = 192;
const NEBULA_PATCHES: usize = 48;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyObject {
    direction_flux: [f32; 4],
    colour_radius: [f32; 4],
    // Angular ellipse aspect, position angle, kind (0 point / 1 diffuse), seed.
    shape: [f32; 4],
}

struct Random(u32);
impl Random {
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        ((self.0 >> 8) as f32 + 0.5) / 16_777_216.0
    }
    fn direction(&mut self) -> Vec3 {
        let z = self.unit() * 2.0 - 1.0;
        let angle = self.unit() * std::f32::consts::TAU;
        let r = (1.0 - z * z).sqrt();
        Vec3::new(r * angle.cos(), z, r * angle.sin())
    }
}

fn galactic_basis() -> glam::Mat3 {
    let x = -Vec3::new(0.65, 0.4, -0.65).normalize();
    let north = Vec3::new(0.05, 0.86, 0.51);
    let z = (north - x * north.dot(x)).normalize();
    glam::Mat3::from_cols(x, z.cross(x), z)
}

fn star_colour(t: f32) -> Vec3 {
    // Slightly enriched stellar colours, normalized to equal photometric Y.
    let cool = Vec3::new(0.48, 0.68, 1.0);
    let warm = Vec3::new(1.0, 0.43, 0.18);
    let colour = if t < 0.5 {
        cool.lerp(Vec3::ONE, t * 2.0)
    } else {
        Vec3::ONE.lerp(warm, (t - 0.5) * 2.0)
    };
    colour / colour.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

fn catalogue() -> Vec<SkyObject> {
    let mut rng = Random(0x57a2_2026);
    let basis = galactic_basis();
    let observer = Vec3::new(8.0, 0.0, 0.025);
    let mut objects =
        Vec::with_capacity(FIELD_STARS + DISC_STARS + GALAXY_PATCHES + NEBULA_PATCHES);
    for i in 0..FIELD_STARS + DISC_STARS + GALAXY_PATCHES + NEBULA_PATCHES {
        let diffuse = i >= FIELD_STARS + DISC_STARS;
        let nebula = i >= FIELD_STARS + DISC_STARS + GALAXY_PATCHES;
        let direction = if i < FIELD_STARS {
            rng.direction()
        } else {
            // Exponential disc plus four loose logarithmic arms. The observer
            // is inside it, so it wraps the whole sky with a brighter core,
            // not an external spiral-galaxy picture plastered on a sphere.
            let radius = (-(rng.unit() * rng.unit()).ln() * 2.6).clamp(0.4, 18.0);
            let arm = (rng.unit() * 4.0).floor() * std::f32::consts::FRAC_PI_2;
            let angle = arm + radius.ln() * 3.4 + (rng.unit() - 0.5) * 0.8;
            let height = (rng.unit() + rng.unit() + rng.unit() - 1.5) * 1.4;
            let position = Vec3::new(radius * angle.cos(), radius * angle.sin(), height);
            (basis * (position - observer)).normalize()
        };
        let colour = if nebula {
            Vec3::new(0.8, 0.22, 0.42).lerp(Vec3::new(0.18, 0.48, 0.95), rng.unit())
        } else if diffuse {
            Vec3::new(0.65, 0.68, 0.82).lerp(Vec3::new(0.9, 0.65, 0.4), rng.unit())
        } else {
            star_colour(rng.unit())
        };
        let flux = if diffuse {
            if nebula { 0.035 } else { 0.018 }
        } else {
            // Many faint stars, a sparse bright tail; no size/brightness LOD.
            0.008 + 0.85 * rng.unit().powi(9)
        };
        let radius = if diffuse {
            0.035 + rng.unit() * 0.12
        } else {
            0.0
        };
        objects.push(SkyObject {
            direction_flux: [direction.x, direction.y, direction.z, flux],
            colour_radius: [colour.x, colour.y, colour.z, radius],
            shape: [
                0.35 + rng.unit() * 0.5,
                rng.unit() * std::f32::consts::TAU,
                f32::from(diffuse),
                rng.unit() * 100.0,
            ],
        });
    }
    objects
}

fn shader_source() -> String {
    format!(
        "{}\n{}\n{}\n{}",
        crate::body::wgsl_constants(),
        include_str!("atmosphere.wgsl"),
        include_str!("stars.wgsl"),
        include_str!("weather_cloud_density.wgsl"),
    )
}

pub struct StarRenderer {
    pipeline: wgpu::RenderPipeline,
    objects: wgpu::Buffer,
    count: u32,
    sky_bind_group: wgpu::BindGroup,
    settings: wgpu::Buffer,
    settings_bind_group: wgpu::BindGroup,
    enabled: bool,
}

impl StarRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        weather_layout: &wgpu::BindGroupLayout,
        atmosphere: crate::atmosphere::SurfaceLightingResources<'_>,
    ) -> Self {
        let entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stellar sky photometry layout"),
            entries: &entries,
        });
        let sky_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stellar sky photometry"),
            layout: &sky_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atmosphere.sky_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(atmosphere.sky_view_sampler),
                },
            ],
        });
        let settings_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stellar settings and extinction layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    ..entries[0]
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    ..entries[1]
                },
            ],
        });
        let settings = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stellar viewport and inertial rotation"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let settings_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stellar settings and extinction"),
            layout: &settings_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atmosphere.transmittance),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(atmosphere.physical_sampler),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stellar catalogue pipeline layout"),
            bind_group_layouts: &[
                Some(camera_layout),
                Some(&sky_layout),
                Some(&settings_layout),
                Some(weather_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("instanced celestial objects"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("instanced celestial objects"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_stars"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<SkyObject>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_stars"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Equal),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let catalogue = catalogue();
        let objects = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable celestial catalogue"),
            contents: bytemuck::cast_slice(&catalogue),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let enabled = !matches!(
            std::env::var("CATINGARDEN_STARS").as_deref(),
            Ok("0" | "false" | "off")
        );
        tracing::info!(target: "catinthegarden::stars", enabled, points = FIELD_STARS + DISC_STARS, diffuse_objects = GALAXY_PATCHES + NEBULA_PATCHES, "configured celestial catalogue");
        Self {
            pipeline,
            objects,
            count: catalogue.len() as u32,
            sky_bind_group,
            settings,
            settings_bind_group,
            enabled,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, size: [u32; 2], rotation: f64) {
        if self.enabled {
            let (sin, cos) = rotation.sin_cos();
            queue.write_buffer(
                &self.settings,
                0,
                bytemuck::cast_slice(&[size[0] as f32, size[1] as f32, sin as f32, cos as f32]),
            );
        }
    }

    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera: &'a wgpu::BindGroup,
        weather: &'a wgpu::BindGroup,
    ) {
        if !self.enabled {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, &self.sky_bind_group, &[]);
        pass.set_bind_group(2, &self.settings_bind_group, &[]);
        pass.set_bind_group(3, weather, &[]);
        pass.set_vertex_buffer(0, self.objects.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_bounded_deterministic_and_covers_the_sphere() {
        let objects = catalogue();
        assert_eq!(objects, catalogue());
        assert_eq!(objects.len(), 15_240);
        assert!(objects.len() * size_of::<SkyObject>() < 1024 * 1024);
        let mut octants = [0; 8];
        for object in &objects {
            let d = Vec3::from_slice(&object.direction_flux);
            assert!((d.length() - 1.0).abs() < 1.0e-5);
            assert!(object.direction_flux[3] > 0.0);
            assert!(
                object
                    .colour_radius
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0)
            );
            octants[usize::from(d.x > 0.0)
                | usize::from(d.y > 0.0) << 1
                | usize::from(d.z > 0.0) << 2] += 1;
        }
        assert!(octants.iter().all(|count| *count > 600));
        let north = galactic_basis().z_axis;
        assert!(
            objects[FIELD_STARS..FIELD_STARS + DISC_STARS]
                .iter()
                .filter(|o| Vec3::from_slice(&o.direction_flux).dot(north).abs() < 0.2)
                .count()
                > DISC_STARS * 8 / 10
        );
    }

    #[test]
    fn unresolved_pixel_filter_conserves_flux_at_every_subpixel_phase() {
        // The quadratic B-spline is a pixel-integrated tent PSF. Its samples
        // partition unity: moving a sub-retina source cannot blink it off.
        let filter = |x: f32| {
            let d = x.abs();
            if d < 0.5 {
                0.75 - d * d
            } else {
                0.5 * (1.5 - d).max(0.0).powi(2)
            }
        };
        for phase in 0..100 {
            let sum: f32 = (-3..=3)
                .map(|x| filter(x as f32 - phase as f32 / 100.0))
                .sum();
            assert!((sum - 1.0).abs() < 1.0e-6);
        }
        let shader = include_str!("stars.wgsl");
        assert!(shader.contains("0.75 - d * d"));
        assert!(shader.contains("0.5 * pow(max(1.5 - d, 0.0), 2.0)"));
    }

    #[test]
    fn brighter_stars_emerge_first_without_a_time_gate() {
        let visible = |flux: f32, sky: f32| (flux - sky).max(0.0);
        assert_eq!(visible(0.01, 0.1), 0.0);
        assert!(visible(0.2, 0.1) > 0.0);
        assert!(visible(0.01, 0.001) > 0.0);
        assert!(include_str!("stars.wgsl").contains("max(luminance - input.sky_luminance, 0.0)"));
    }

    #[test]
    fn assembled_star_shader_validates() {
        let source = shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("star shader validates");
    }
}

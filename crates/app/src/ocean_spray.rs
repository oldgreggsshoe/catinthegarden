//! Wind-blown spray from folding FFT crests (FFT ocean only).
//!
//! A fixed pool of GPU particles in the FFT tangent plane, relative to the
//! camera. A compute pass integrates them (wind drag, gravity) and respawns
//! dead ones where the displaced surface is folding; a draw pass renders them
//! as soft camera-facing puffs lit like the foam. Nothing reads back.

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

pub(super) const SPRAY_PARTICLES: u32 = 16384;
/// The first slots belong to the ship's bow; the rest to breaking crests.
/// Mirrored in ocean_spray_update.wgsl, pinned by a test.
#[allow(dead_code)]
pub(super) const SHIP_SPRAY_SLOTS: u32 = 2048;
/// Spray is drawn within this distance of the camera; beyond it a puff is a
/// pixel or two and the fold foam carries the look.
const SPAWN_RADIUS_METERS: f32 = 300.0;
/// Births per second per spawn attempt on fully folded water.
const SPAWN_RATE: f32 = 40.0;
/// Frames longer than this (pauses, hitches) are clamped so particles do not
/// teleport.
const MAX_STEP_SECONDS: f32 = 0.1;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SprayFrame {
    axis_u: [f32; 4],
    axis_v: [f32; 4],
    shift_dt: [f32; 4],
    wind: [f32; 4],
    params: [f32; 4],
    /// Ship waterline origin relative to the camera (u, v m, height above sea
    /// level m), spray intensity 0-1.
    ship_origin: [f32; 4],
    /// Ship forward (u, v), hull half-length, hull half-beam.
    ship_axes: [f32; 4],
    /// Ship velocity (u, v, up m/s), unused.
    ship_velocity: [f32; 4],
}

/// Where and how hard the ship's bow is throwing spray, in the planet-local
/// frame. Intensity comes from the bow slamming down into the water.
#[derive(Clone, Copy, Debug)]
pub struct ShipSprayEmitter {
    pub waterline_origin: glam::DVec3,
    pub forward: glam::DVec3,
    pub velocity: glam::DVec3,
    pub intensity: f32,
}

/// Unit wind direction in the FFT (u, v) axes; the spectrum's own wind.
pub(super) fn wind_direction_uv() -> [f32; 2] {
    let [x, y] = crate::ocean_fft::WIND_DIRECTION;
    let length = (x * x + y * y).sqrt();
    [x / length, y / length]
}

/// Spray strength multiplier, `CATINGARDEN_OCEAN_FFT_SPRAY` (default 1, 0 off,
/// up to 5); scales the birth rate. FFT ocean only.
pub(super) fn spray_strength() -> f32 {
    static VALUE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("CATINGARDEN_OCEAN_FFT_SPRAY")
            .ok()
            .and_then(|value| value.trim().parse::<f32>().ok())
            .map_or(1.0, |value| value.clamp(0.0, 5.0))
    })
}

pub(super) fn spray_enabled() -> bool {
    crate::planet::ocean_fft_enabled() && spray_strength() > 0.0
}

pub(super) struct OceanSpray {
    _particles: wgpu::Buffer,
    uniform: wgpu::Buffer,
    update_bind_group: wgpu::BindGroup,
    draw_bind_group: wgpu::BindGroup,
    update_pipeline: wgpu::ComputePipeline,
    draw_pipeline: wgpu::RenderPipeline,
    previous_camera_uv: Option<[f64; 2]>,
    previous_time: Option<f32>,
    frame: u32,
}

impl OceanSpray {
    pub(super) fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        shared_layout: &wgpu::BindGroupLayout,
        ocean_fft: &crate::ocean_fft::OceanFft,
        hdr_format: wgpu::TextureFormat,
    ) -> Self {
        let particles = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean spray particles"),
            contents: &vec![0u8; SPRAY_PARTICLES as usize * 32],
            usage: wgpu::BufferUsages::STORAGE,
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean spray frame"),
            contents: bytemuck::bytes_of(&SprayFrame::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage_entry = |binding, visibility, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let update_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean spray update layout"),
            entries: &[
                storage_entry(0, compute, false),
                uniform_entry(1, compute),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: compute,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(4, compute),
            ],
        });
        let update_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean spray update"),
            layout: &update_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: particles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&ocean_fft.field_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&ocean_fft.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ocean_fft.view_params.as_entire_binding(),
                },
            ],
        });
        let update_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean spray update"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ocean_spray_update.wgsl").into()),
        });
        let update_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean spray update pipeline layout"),
            bind_group_layouts: &[Some(&update_layout)],
            immediate_size: 0,
        });
        let update_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ocean spray update"),
            layout: Some(&update_pipeline_layout),
            module: &update_shader,
            entry_point: Some("cs_spray"),
            compilation_options: Default::default(),
            cache: None,
        });

        let vertex_fragment = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean spray draw layout"),
            entries: &[storage_entry(0, vertex_fragment, true), uniform_entry(1, vertex_fragment)],
        });
        let draw_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean spray draw"),
            layout: &draw_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: particles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
            ],
        });
        let draw_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean spray draw"),
            source: wgpu::ShaderSource::Wgsl(draw_shader_source().into()),
        });
        let draw_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean spray draw pipeline layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&draw_layout), Some(shared_layout)],
            immediate_size: 0,
        });
        let draw_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ocean spray draw"),
            layout: Some(&draw_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &draw_shader,
                entry_point: Some("vs_spray"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: Some("fs_spray"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // Depth-tested against the scene (reversed-Z) so crests and the
            // hull hide spray behind them, but writes no depth of its own.
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
        Self {
            _particles: particles,
            uniform,
            update_bind_group,
            draw_bind_group,
            update_pipeline,
            draw_pipeline,
            previous_camera_uv: None,
            previous_time: None,
            frame: 0,
        }
    }

    /// Advances the particles to `ocean_time_seconds`; call after the FFT
    /// field update so spawning reads the current surface.
    pub(super) fn update(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera_direction: glam::DVec3,
        ocean_time_seconds: f32,
        storm_intensity: f32,
        ship: Option<&ShipSprayEmitter>,
    ) {
        let (u, v) = crate::ocean_fft::anchor_axes(camera_direction.normalize().to_array());
        let radius = crate::planet::planet_radius_meters();
        let direction = camera_direction.normalize().to_array();
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let camera_uv = [radius * dot(u, direction), radius * dot(v, direction)];
        let shift = self
            .previous_camera_uv
            .map_or([0.0, 0.0], |previous| [camera_uv[0] - previous[0], camera_uv[1] - previous[1]]);
        // A jump (teleport, first frame) moves the camera past the whole pool.
        let shift = if (shift[0] * shift[0] + shift[1] * shift[1]).sqrt() > 500.0 {
            [0.0, 0.0]
        } else {
            shift
        };
        let dt = self
            .previous_time
            .map_or(0.0, |previous| (ocean_time_seconds - previous).clamp(0.0, MAX_STEP_SECONDS));
        self.previous_camera_uv = Some(camera_uv);
        self.previous_time = Some(ocean_time_seconds);
        self.frame = self.frame.wrapping_add(1);
        let wind = wind_direction_uv();
        let xyz = |a: [f64; 3]| [a[0] as f32, a[1] as f32, a[2] as f32, 0.0];
        let (ship_origin, ship_axes, ship_velocity) = match ship {
            Some(ship) => {
                let at = |p: glam::DVec3| {
                    let d = p.normalize().to_array();
                    [radius * dot(u, d), radius * dot(v, d)]
                };
                let o = at(ship.waterline_origin);
                let relative = [o[0] - camera_uv[0], o[1] - camera_uv[1]];
                let tangent = |w: glam::DVec3| {
                    let w = w.to_array();
                    [dot(u, w), dot(v, w)]
                };
                let f = tangent(ship.forward);
                let f_len = (f[0] * f[0] + f[1] * f[1]).sqrt().max(1e-9);
                let vel = tangent(ship.velocity);
                let up_speed = ship.velocity.dot(ship.waterline_origin.normalize());
                let far = relative[0].hypot(relative[1]) > 2_000.0;
                (
                    [
                        relative[0] as f32,
                        relative[1] as f32,
                        (ship.waterline_origin.length() - radius) as f32,
                        if far { 0.0 } else { ship.intensity },
                    ],
                    [
                        (f[0] / f_len) as f32,
                        (f[1] / f_len) as f32,
                        (0.5 * crate::ship::HULL_LENGTH_METERS) as f32,
                        (0.5 * crate::ship::HULL_BEAM_METERS) as f32,
                    ],
                    [vel[0] as f32, vel[1] as f32, up_speed as f32, 0.0],
                )
            }
            None => ([0.0; 4], [1.0, 0.0, 0.0, 0.0], [0.0; 4]),
        };
        let frame = SprayFrame {
            axis_u: xyz(u),
            axis_v: xyz(v),
            shift_dt: [shift[0] as f32, shift[1] as f32, dt, (self.frame % (1 << 24)) as f32],
            wind: [wind[0], wind[1], crate::ocean_fft::wind_speed_from_environment(), 0.0],
            params: [
                SPAWN_RADIUS_METERS,
                SPAWN_RATE * spray_strength(),
                crate::ocean_fft::choppiness(),
                crate::ocean_fft::swell_height_meters(storm_intensity),
            ],
            ship_origin,
            ship_axes,
            ship_velocity,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&frame));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ocean spray update"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.update_pipeline);
        pass.set_bind_group(0, &self.update_bind_group, &[]);
        pass.dispatch_workgroups(SPRAY_PARTICLES.div_ceil(64), 1, 1);
    }

    pub(super) fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
        shared_bind_group: &wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.draw_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.draw_bind_group, &[]);
        render_pass.set_bind_group(2, shared_bind_group, &[]);
        render_pass.draw(0..6, 0..SPRAY_PARTICLES);
    }
}

fn draw_shader_source() -> String {
    format!(
        "{}\n{}",
        crate::planet::shared_planet_shader_source(),
        include_str!("ocean_spray_draw.wgsl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(source: &str) {
        let module = wgpu::naga::front::wgsl::parse_str(source).expect("spray shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("spray shader must validate");
    }

    #[test]
    fn spray_shaders_parse_and_validate() {
        validate(include_str!("ocean_spray_update.wgsl"));
        validate(&draw_shader_source());
    }

    #[test]
    fn the_shader_reserves_the_same_ship_slots() {
        let source = include_str!("ocean_spray_update.wgsl");
        assert!(source.contains(&format!("const SHIP_SPRAY_SLOTS: u32 = {SHIP_SPRAY_SLOTS}u;")));
        let draw = include_str!("ocean_spray_draw.wgsl");
        assert!(draw.contains(&format!("instance_index < {SHIP_SPRAY_SLOTS}u")));
        assert!(SHIP_SPRAY_SLOTS < SPRAY_PARTICLES);
    }

    #[test]
    fn particle_layout_matches_the_shader() {
        // Two vec4<f32> per particle.
        assert_eq!(std::mem::size_of::<[[f32; 4]; 2]>(), 32);
        assert_eq!(std::mem::size_of::<SprayFrame>(), 128);
    }
}

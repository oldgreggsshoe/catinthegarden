//! Small world-reprojected GPU foam atlas, separate from the buoyant wave field.

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

/// Atlas side in texels over 512m: 4m texels for the Gerstner trial, 2m for
/// FFT fold foam, whose folds are a few metres across.
fn atlas_side() -> u32 {
    if crate::planet::ocean_fft_enabled() { 256 } else { 128 }
}

fn previous_atlas_is_valid(previous_time: Option<f32>, current_time: f32) -> bool {
    previous_time.is_some_and(|previous| current_time >= previous && current_time - previous < 10.0)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FoamFrame {
    previous_center: [f32; 4],
    previous_east: [f32; 4],
    previous_north: [f32; 4],
    current_center: [f32; 4],
    current_east: [f32; 4],
    current_north: [f32; 4],
    timing: [f32; 4],
    /// Ship waterline origin relative to the current atlas centre (east,
    /// north m), foam intensity 0-1, 1 if a ship is present.
    ship: [f32; 4],
    /// Ship forward (east, north), hull half-length, half-beam.
    ship_axes: [f32; 4],
}

pub(super) struct OceanFoamHistory {
    _textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    _sampler: wgpu::Sampler,
    _uniform: wgpu::Buffer,
    bind_groups: [wgpu::BindGroup; 2],
    pipeline: wgpu::ComputePipeline,
    current: usize,
    previous_basis: Option<([f32; 3], [f32; 3], [f32; 3])>,
    previous_time: Option<f32>,
    side: u32,
}

impl OceanFoamHistory {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_layout: &wgpu::BindGroupLayout,
        ocean_fft: &crate::ocean_fft::OceanFft,
    ) -> Self {
        let side = atlas_side();
        let textures = std::array::from_fn(|index| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(if index == 0 {
                    "foam history A"
                } else {
                    "foam history B"
                }),
                size: wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let zero = vec![0_u8; (side * side * 4) as usize];
        for texture in &textures {
            queue.write_texture(
                texture.as_image_copy(),
                &zero,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 4),
                    rows_per_image: Some(side),
                },
                texture.size(),
            );
        }
        let views = std::array::from_fn(|index| textures[index].create_view(&Default::default()));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("foam history bilinear sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("foam history frame"),
            contents: bytemuck::bytes_of(&FoamFrame::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("foam history compute layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // FFT field, its repeat sampler and view parameters: the fold
                // foam source. Bound always; read only in FFT mode.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_groups = std::array::from_fn(|index| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("foam history ping-pong"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[index]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&views[1 - index]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&ocean_fft.field_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&ocean_fft.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: ocean_fft.view_params.as_entire_binding(),
                    },
                ],
            })
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world-space ocean foam history"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    crate::planet::shared_planet_shader_source(),
                    include_str!("ocean_foam.wgsl")
                )
                .into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("foam history pipeline layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("foam history update"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_foam"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        Self {
            _textures: textures,
            views,
            _sampler: sampler,
            _uniform: uniform,
            bind_groups,
            pipeline,
            current: 0,
            previous_basis: None,
            previous_time: None,
            side,
        }
    }

    pub(super) fn view(&self, index: usize) -> &wgpu::TextureView {
        &self.views[index]
    }

    pub(super) fn current(&self) -> usize {
        self.current
    }

    pub(super) fn update(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera_bind_group: &wgpu::BindGroup,
        center: glam::DVec3,
        right: glam::DVec3,
        ocean_time_seconds: f32,
        ship: Option<&super::ocean_spray::ShipSprayEmitter>,
    ) {
        let center = center.normalize();
        let east = (right - center * right.dot(center)).normalize();
        let north = center.cross(east);
        let xyz = |value: glam::DVec3| value.to_array().map(|component| component as f32);
        let basis = (xyz(center), xyz(east), xyz(north));
        let (previous_center, previous_east, previous_north) = self.previous_basis.unwrap_or(basis);
        let valid_previous = previous_atlas_is_valid(self.previous_time, ocean_time_seconds);
        let elapsed = self.previous_time.map_or(0.0, |previous_time| {
            (ocean_time_seconds - previous_time).max(0.0)
        });
        let frame = FoamFrame {
            previous_center: [
                previous_center[0],
                previous_center[1],
                previous_center[2],
                0.0,
            ],
            previous_east: [previous_east[0], previous_east[1], previous_east[2], 0.0],
            previous_north: [previous_north[0], previous_north[1], previous_north[2], 0.0],
            current_center: [basis.0[0], basis.0[1], basis.0[2], 0.0],
            current_east: [basis.1[0], basis.1[1], basis.1[2], 0.0],
            current_north: [basis.2[0], basis.2[1], basis.2[2], 0.0],
            timing: {
                let wind = super::ocean_spray::wind_direction_uv();
                [elapsed, f32::from(valid_previous), wind[0], wind[1]]
            },
            ship: ship.map_or([0.0; 4], |ship| {
                let offset = ship.waterline_origin
                    - center * crate::planet::planet_radius_meters();
                [
                    offset.dot(east) as f32,
                    offset.dot(north) as f32,
                    ship.intensity,
                    1.0,
                ]
            }),
            ship_axes: ship.map_or([1.0, 0.0, 0.0, 0.0], |ship| {
                let f = glam::DVec2::new(ship.forward.dot(east), ship.forward.dot(north))
                    .normalize_or(glam::DVec2::X);
                [
                    f.x as f32,
                    f.y as f32,
                    (0.5 * crate::ship::HULL_LENGTH_METERS) as f32,
                    (0.5 * crate::ship::HULL_BEAM_METERS) as f32,
                ]
            }),
        };
        queue.write_buffer(&self._uniform, 0, bytemuck::bytes_of(&frame));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ocean foam history"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, &self.bind_groups[self.current], &[]);
        pass.dispatch_workgroups(self.side / 8, self.side / 8, 1);
        drop(pass);
        self.current = 1 - self.current;
        self.previous_basis = Some(basis);
        self.previous_time = Some(ocean_time_seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foam_history_rejects_time_rewinds_and_long_gaps() {
        assert!(!previous_atlas_is_valid(None, 1.0));
        assert!(!previous_atlas_is_valid(Some(2.0), 1.0));
        assert!(!previous_atlas_is_valid(Some(0.0), 10.0));
        assert!(previous_atlas_is_valid(Some(1.0), 1.016));
    }

    #[test]
    fn moving_atlas_reprojects_the_same_world_point() {
        let radius = crate::planet::planet_radius_meters();
        let first_center = glam::DVec3::new(0.7, 0.5, 0.3).normalize();
        let first_east = glam::DVec3::Y.cross(first_center).normalize();
        let first_north = first_center.cross(first_east);
        let point = (first_center * radius + first_east * 80.0 + first_north * 40.0).normalize();
        let moved_center = (first_center * radius + first_east * 30.0).normalize();
        let moved_east = (first_east - moved_center * moved_center.dot(first_east)).normalize();
        let moved_north = moved_center.cross(moved_east);
        let old_uv = glam::DVec2::new(
            (point - first_center).dot(first_east),
            (point - first_center).dot(first_north),
        ) * (radius / 512.0)
            + glam::DVec2::splat(0.5);
        let new_uv = glam::DVec2::new(
            (point - moved_center).dot(moved_east),
            (point - moved_center).dot(moved_north),
        ) * (radius / 512.0)
            + glam::DVec2::splat(0.5);
        assert!((old_uv.x - 0.5 - 80.0 / 512.0).abs() < 1.0e-5);
        assert!((old_uv.y - 0.5 - 40.0 / 512.0).abs() < 1.0e-5);
        assert!((new_uv.x - 0.5 - 50.0 / 512.0).abs() < 1.0e-5);
        assert!((new_uv.y - old_uv.y).abs() < 1.0e-5);
    }
}

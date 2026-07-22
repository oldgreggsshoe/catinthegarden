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
    _padding: [u32; 2],
}

pub struct FoveatedRenderer {
    pipeline: wgpu::RenderPipeline,
    fields_bind_group: wgpu::BindGroup,
    ray_uniform_buffer: wgpu::Buffer,
    height_min_meters: f32,
    height_max_meters: f32,
    face_quads: u32,
    max_height_mip_count: u32,
    _height_texture: wgpu::Texture,
    _max_height_texture: wgpu::Texture,
    _biome_texture: wgpu::Texture,
    _moisture_texture: wgpu::Texture,
}

impl FoveatedRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ray field debug pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&fields_bind_group_layout),
                Some(shared_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader_source = raymarch_shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("full-resolution terrain raymarch shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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

        Ok(Self {
            pipeline,
            fields_bind_group,
            ray_uniform_buffer,
            height_min_meters,
            height_max_meters,
            face_quads,
            max_height_mip_count,
            _height_texture: height_texture,
            _max_height_texture: max_height_texture,
            _biome_texture: biome_texture,
            _moisture_texture: moisture_texture,
        })
    }

    pub fn update(&self, queue: &wgpu::Queue, camera_altitude_meters: f64) {
        let uniform = RayUniform::for_camera(
            self.height_min_meters,
            self.height_max_meters,
            self.face_quads,
            self.max_height_mip_count,
            camera_altitude_meters,
        );
        queue.write_buffer(&self.ray_uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        shared_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.fields_bind_group, &[]);
        render_pass.set_bind_group(2, shared_bind_group, &[]);
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
            _padding: [0; 2],
        }
    }
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
        FIELD_LEVEL, RayUniform, face_sample_source, max_height_mips, raymarch_shader_source,
    };
    use catinthegarden_coretypes::{TILE_LOGICAL_SIZE, TILE_STORED_SIZE};

    const PLANET_RADIUS_METERS: f64 = 4_000_000.0;

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

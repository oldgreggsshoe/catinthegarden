use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    num::NonZeroU64,
    time::{Duration, Instant},
};

use glam::DVec3;
use planet_coretypes::{TileKey, face_uv_to_direction, tile_key_for_direction};
use wgpu::util::DeviceExt;

use crate::{
    atmosphere::SurfaceLightingResources,
    planet::planet_radius_meters,
    terrain::{TerrainRenderer, forest_biome_requires_evergreen, forest_surface_is_eligible},
};

pub const FOREST_CENTRE_DIRECTION: DVec3 =
    DVec3::new(0.374_871_986_443, 0.737_334_908_710, 0.561_968_171_854);

const TREE_COUNT: usize = 12_288;
#[cfg(test)]
const TREE_MESH_VERTEX_COUNT: u32 = 440;
const TREE_MESH_INDEX_COUNT: u32 = 1260;
const TREE_MESH_MEDIUM_INDEX_COUNT: u32 = 420;
const TREE_MESH_SPARSE_INDEX_COUNT: u32 = 300;
const TREE_BASE_SINK_METERS: f64 = 0.45;
const TREE_HEIGHT_MIN_METERS: f32 = 22.0;
const TREE_HEIGHT_RANGE_METERS: f32 = 26.0;
const FOREST_DRAW_ALTITUDE_METERS: f64 = 50_000.0;
const FOREST_TREE_RENDER_DISTANCE_METERS: f64 = 8_000.0;
const FOREST_PREFETCH_DISTANCE_METERS: f64 = 12_000.0;
const FOREST_CELL_LEVEL: u8 = 12;
const FOREST_MINIMUM_MOISTURE: f32 = 0.38;
const FOREST_MAXIMUM_SLOPE_RADIANS: f64 = 32.0_f64.to_radians();
// This compact directional mask refines the kilometre-scale L4 biome source
// without adding another baked channel or tile stream. It is mirrored by the
// terrain shader so tree placement and far canopy darkening share one field.
const FOREST_DENSITY_FREQUENCY: f64 = 8_192.0;
const FOREST_PATCH_TRANSITION_SECONDS: f64 = 1.5;
const FOREST_PATCH_CANDIDATES_PER_FRAME: usize = 128;
const FOREST_PRIMARY_PATCH_CANDIDATES_PER_FRAME: usize = 256;
const FOREST_INITIAL_PATCH_CANDIDATES_PER_FRAME: usize = 512;
const FOREST_STARTUP_PATCH_COUNT: u64 = 3;
const FOREST_MAX_RENDERABLE_PATCHES: usize = 128;
const FOREST_MAX_CACHED_PATCHES: usize = 256;
const FOREST_MAX_DRAW_INSTANCES: usize = 262_144;
const FOREST_GPU_FULL_CANDIDATES: u32 = 4_096;
const FOREST_GPU_MEDIUM_CANDIDATES: u32 = 512;
const FOREST_GPU_SPARSE_CANDIDATES: u32 = 64;
/// The camera may move this far before trees are regenerated: only their
/// distance-filtered ground detail depends on it.
const FOREST_GPU_REGENERATE_DISTANCE_METERS: f64 = 25.0;
const FOREST_GPU_FULL_DISTANCE_METERS: f64 = 400.0;
/// The nearest cell is drawn in full detail when no cell is within the full
/// distance, but only if the camera is practically inside it: a cell is 1.5km
/// across, and 8,192 full-mesh trees spread over it cost ~10ms.
const FOREST_GPU_FORCED_FULL_DISTANCE_METERS: f64 = 1_400.0;
/// ...and only while the camera is this close to the ground. Looking down from
/// higher up, every tree in the cell is small and the medium mesh does.
const FOREST_GPU_FORCED_FULL_HEIGHT_METERS: f64 = 300.0;
const FOREST_GPU_MEDIUM_DISTANCE_METERS: f64 = 4_000.0;
// Terrain-grounded canopy cards make a cell read as forest before its complete
// individual-tree population has finished resolving. They are built and
// published atomically, never exposed one candidate at a time.
const FOREST_PROXY_CANDIDATES_PER_PATCH: usize = 128;
const FOREST_PROXY_PATCHES_PER_FRAME: usize = 2;
const FOREST_PROXY_CARDS_PER_SAMPLE: usize = 3;
const FOREST_PROXY_HEIGHT_SCALE: f32 = 1.25;
const FOREST_PROXY_WIDTH_SCALE: f32 = 6.0;
const TREE_LOD_FULL_PIXELS: f64 = 12.0;
const TREE_LOD_MEDIUM_PIXELS: f64 = 3.0;
const TREE_LOD_SPARSE_PIXELS: f64 = 1.0;
// Keep a small, deterministic subset of trees alive below the normal sparse
// threshold. These are rendered as tiny trees rather than being removed
// outright, so entering/leaving a forest does not reveal a hard population
// cutoff.  The subset remains bounded by the existing draw-instance budget.
const TREE_LOD_PLACEHOLDER_DENSITY: f32 = 0.12;
const TREE_LOD_PLACEHOLDER_SCALE: f32 = 0.10;
const FOREST_PLANET_SEED: u32 = 0x6d2b_79f5;

fn forest_rendering_from_env() -> bool {
    match std::env::var("PLANET_FOREST") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        ),
        Err(_) => true,
    }
}

fn gpu_resident_forests_from_env() -> bool {
    match std::env::var("PLANET_GPU_FOREST") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        ),
        Err(_) => true,
    }
}

fn forest_shader_source() -> String {
    // The same switch terrain reads, so a cloud cannot shadow trees standing
    // on unshadowed ground.
    let features = crate::planet::render_feature_constants();
    [
        crate::body::wgsl_constants().as_str(),
        features.as_str(),
        include_str!("forest.wgsl"),
        include_str!("weather_cloud_density.wgsl"),
    ]
    .join("\n")
}

fn gpu_forest_shader_source() -> String {
    [
        crate::terrain::planet_shader_source(),
        include_str!("forest_gpu.wgsl").to_owned(),
    ]
    .join("\n")
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct TreeInstance {
    centre_and_height: [f32; 4],
    width_shade_kind_seed: [f32; 4],
}

impl TreeInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ForestUniform {
    camera_planet_position: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuForestCell {
    cell_uv_origin_span: [f32; 4],
    source_uv_scale_offset: [f32; 4],
    anchor_direction_source_level: [f32; 4],
    key: [u32; 4],
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum GpuForestTier {
    Full,
    Medium,
    Sparse,
}

impl GpuForestTier {
    fn candidates_per_cell(self) -> u32 {
        match self {
            Self::Full => FOREST_GPU_FULL_CANDIDATES,
            Self::Medium => FOREST_GPU_MEDIUM_CANDIDATES,
            Self::Sparse => FOREST_GPU_SPARSE_CANDIDATES,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct GpuForestBatch {
    tier: GpuForestTier,
    source_key: TileKey,
    dynamic_offset: u32,
    cell_count: u32,
}

#[derive(Clone, Copy, Debug)]
struct TreeLayout {
    height_meters: f32,
    width_meters: f32,
    shade: f32,
    kind: f32,
    seed: f32,
}

struct ForestPatch {
    key: TileKey,
    centre_direction: DVec3,
    trees: Vec<TreeInstance>,
    minimum_source_level: Option<u8>,
    visible_since: Instant,
}

struct ForestProxyPatch {
    trees: Vec<TreeInstance>,
}

struct PendingForestPatch {
    key: TileKey,
    centre_direction: DVec3,
    candidates: Vec<(DVec3, TreeLayout)>,
    next_candidate: usize,
    trees: Vec<TreeInstance>,
    minimum_source_level: u8,
}

impl PendingForestPatch {
    fn new(key: TileKey) -> Self {
        Self {
            key,
            centre_direction: forest_cell_centre_direction(key),
            candidates: forest_patch_tree_layouts(key),
            next_candidate: 0,
            trees: Vec::with_capacity(TREE_COUNT),
            minimum_source_level: u8::MAX,
        }
    }

    fn advance(
        &mut self,
        terrain: &TerrainRenderer,
        camera_altitude_meters: f64,
        candidate_budget: usize,
    ) -> bool {
        let batch_end =
            pending_batch_end(self.next_candidate, self.candidates.len(), candidate_budget);
        while self.next_candidate < batch_end {
            let (direction, layout) = self.candidates[self.next_candidate];
            let Some(sample) = terrain.forest_surface_sample_at(direction, camera_altitude_meters)
            else {
                break;
            };
            debug_assert_eq!(sample.source_key.level, sample.source_level);
            self.minimum_source_level = self.minimum_source_level.min(sample.source_level);
            if forest_surface_is_eligible(
                sample,
                FOREST_MINIMUM_MOISTURE,
                FOREST_MAXIMUM_SLOPE_RADIANS,
            ) && !terrain.road_clears_tree(direction, sample.height_meters)
                && f64::from(layout.seed) <= forest_placement_density_at(direction)
            {
                let centre = direction
                    * (planet_radius_meters() + sample.height_meters
                        - tree_base_sink_meters(layout.width_meters, sample.slope_radians));
                self.trees.push(TreeInstance {
                    centre_and_height: [
                        centre.x as f32,
                        centre.y as f32,
                        centre.z as f32,
                        layout.height_meters,
                    ],
                    width_shade_kind_seed: [
                        layout.width_meters,
                        layout.shade,
                        if forest_biome_requires_evergreen(sample.biome) {
                            1.0
                        } else {
                            layout.kind
                        },
                        layout.seed,
                    ],
                });
            }
            self.next_candidate += 1;
        }
        self.next_candidate == self.candidates.len()
    }

    fn finish(self) -> ForestPatch {
        ForestPatch {
            key: self.key,
            centre_direction: self.centre_direction,
            trees: self.trees,
            minimum_source_level: (self.minimum_source_level != u8::MAX)
                .then_some(self.minimum_source_level),
            visible_since: Instant::now(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TreeLod {
    Full,
    Medium,
    Sparse,
    Zero,
}

#[derive(Clone, Copy, Debug, Default)]
struct TreeLodCounts {
    full: u32,
    medium: u32,
    sparse: u32,
    zero: u32,
}

impl TreeLodCounts {
    fn add(&mut self, lod: TreeLod) {
        match lod {
            TreeLod::Full => self.full += 1,
            TreeLod::Medium => self.medium += 1,
            TreeLod::Sparse => self.sparse += 1,
            TreeLod::Zero => self.zero += 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ForestStats {
    pub patch_count: u16,
    pub proxy_patch_count: u16,
    pub instances: u32,
    pub proxy_instances: u32,
    pub full_instances: u32,
    pub medium_instances: u32,
    pub sparse_instances: u32,
    pub zero_instances: u32,
    pub rebuild_count: u64,
    pub patch_key: Option<TileKey>,
    pub minimum_source_level: Option<u8>,
    pub pending_candidates: u32,
    pub pending_candidates_total: u32,
    pub transition_progress: f32,
}

pub struct ForestRenderer {
    pipeline: wgpu::RenderPipeline,
    gpu_compute_pipelines: [wgpu::ComputePipeline; 3],
    bind_group: wgpu::BindGroup,
    gpu_source_bind_group_layout: wgpu::BindGroupLayout,
    gpu_source_bind_groups: BTreeMap<TileKey, wgpu::BindGroup>,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    medium_index_buffer: wgpu::Buffer,
    sparse_index_buffer: wgpu::Buffer,
    gpu_cell_buffer: wgpu::Buffer,
    instance_count: u32,
    proxy_instance_count: u32,
    draw_lod_counts: TreeLodCounts,
    patches: BTreeMap<TileKey, ForestPatch>,
    proxy_patches: BTreeMap<TileKey, ForestProxyPatch>,
    pending_patch: Option<PendingForestPatch>,
    draw_instances: Vec<TreeInstance>,
    lod_counts: TreeLodCounts,
    empty_patch_keys: BTreeSet<TileKey>,
    primary_patch_key: Option<TileKey>,
    gpu_batches: Vec<GpuForestBatch>,
    /// What the instance buffer was last generated from. Trees are fixed to
    /// the planet, so the compute pass is only rerun when the cells, their
    /// tiers or the camera (the detail filter) have changed enough.
    gpu_generated_signature: Option<u64>,
    road_revision: u64,
    gpu_generated_camera: DVec3,
    gpu_generation_needed: bool,
    gpu_cell_count: u32,
    gpu_candidate_count: u32,
    gpu_minimum_source_level: Option<u8>,
    gpu_cell_alignment: usize,
    gpu_cell_capacity: usize,
    rebuild_count: u64,
    enabled: bool,
    gpu_resident: bool,
}

impl ForestRenderer {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        weather_field_bind_group_layout: &wgpu::BindGroupLayout,
        terrain: &TerrainRenderer,
        atmosphere: SurfaceLightingResources<'_>,
    ) -> Self {
        let initial_key = forest_cell_key(FOREST_CENTRE_DIRECTION);
        let enabled = forest_rendering_from_env();
        tracing::info!(
            target: "planet::forest",
            enabled,
            maximum_renderable_patches = FOREST_MAX_RENDERABLE_PATCHES,
            maximum_cached_patches = FOREST_MAX_CACHED_PATCHES,
            maximum_draw_instances = FOREST_MAX_DRAW_INSTANCES,
            "configured instanced 3d forest"
        );
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("forest camera uniform"),
            size: size_of::<ForestUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forest bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forest bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atmosphere.sky_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(atmosphere.sky_view_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forest pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&bind_group_layout),
                Some(weather_field_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader_source = forest_shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forest 3d tree shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("forest 3d tree pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[TreeInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(),
        });
        let gpu_cell_alignment = (device.limits().min_storage_buffer_offset_alignment as usize)
            .div_ceil(size_of::<GpuForestCell>());
        let gpu_cell_capacity = FOREST_MAX_RENDERABLE_PATCHES * (gpu_cell_alignment + 2);
        let gpu_cell_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("immediate GPU forest cells"),
            size: (gpu_cell_capacity * size_of::<GpuForestCell>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let gpu_source_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("immediate GPU forest source bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(size_of::<ForestUniform>() as u64),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: true,
                            min_binding_size: NonZeroU64::new(size_of::<GpuForestCell>() as u64),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(size_of::<TreeInstance>() as u64),
                        },
                        count: None,
                    },
                ],
            });
        let gpu_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("immediate GPU forest pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&gpu_source_bind_group_layout),
                Some(terrain.shared_bind_group_layout()),
            ],
            immediate_size: 0,
        });
        let gpu_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("immediate GPU forest shader"),
            source: wgpu::ShaderSource::Wgsl(gpu_forest_shader_source().into()),
        });
        let create_compute_pipeline = |entry_point, label| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&gpu_pipeline_layout),
                module: &gpu_shader,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: crate::pipeline_cache::get(),
            })
        };
        let gpu_compute_pipelines = [
            create_compute_pipeline("forest_gpu_compute_full", "GPU forest full compute"),
            create_compute_pipeline("forest_gpu_compute_medium", "GPU forest medium compute"),
            create_compute_pipeline("forest_gpu_compute_sparse", "GPU forest sparse compute"),
        ];
        let buffer_instances =
            vec![<TreeInstance as bytemuck::Zeroable>::zeroed(); FOREST_MAX_DRAW_INSTANCES];
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest tree instances"),
            contents: bytemuck::cast_slice(&buffer_instances),
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
        });
        let indices = tree_mesh_indices();
        debug_assert_eq!(indices.len(), TREE_MESH_INDEX_COUNT as usize);
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest shared 3d tree mesh indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let medium_indices = tree_mesh_medium_indices();
        debug_assert_eq!(medium_indices.len(), TREE_MESH_MEDIUM_INDEX_COUNT as usize);
        let medium_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest shared medium 3d tree mesh indices"),
            contents: bytemuck::cast_slice(&medium_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let sparse_indices = tree_mesh_sparse_indices();
        debug_assert_eq!(sparse_indices.len(), TREE_MESH_SPARSE_INDEX_COUNT as usize);
        let sparse_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest shared sparse 3d tree mesh indices"),
            contents: bytemuck::cast_slice(&sparse_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            pipeline,
            gpu_compute_pipelines,
            bind_group,
            gpu_source_bind_group_layout,
            gpu_source_bind_groups: BTreeMap::new(),
            uniform_buffer,
            instance_buffer,
            index_buffer,
            medium_index_buffer,
            sparse_index_buffer,
            gpu_cell_buffer,
            instance_count: 0,
            proxy_instance_count: 0,
            draw_lod_counts: TreeLodCounts::default(),
            patches: BTreeMap::new(),
            proxy_patches: BTreeMap::new(),
            pending_patch: Some(PendingForestPatch::new(initial_key)),
            draw_instances: Vec::new(),
            lod_counts: TreeLodCounts::default(),
            empty_patch_keys: BTreeSet::new(),
            primary_patch_key: Some(initial_key),
            gpu_batches: Vec::new(),
            gpu_generated_signature: None,
            road_revision: terrain.road_revision(),
            gpu_generated_camera: DVec3::ZERO,
            gpu_generation_needed: false,
            gpu_cell_count: 0,
            gpu_candidate_count: 0,
            gpu_minimum_source_level: None,
            gpu_cell_alignment,
            gpu_cell_capacity,
            rebuild_count: 0,
            enabled,
            gpu_resident: gpu_resident_forests_from_env(),
        }
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, camera_planet_position: DVec3) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&ForestUniform {
                camera_planet_position: [
                    camera_planet_position.x as f32,
                    camera_planet_position.y as f32,
                    camera_planet_position.z as f32,
                    0.0,
                ],
            }),
        );
    }

    /// Updates the bounded set of camera-local cells whose trees can still be
    /// resolved. Only the resident terrain cache is probed; one missing source
    /// stalls the current cell without adding disk I/O to the frame.
    #[allow(clippy::too_many_arguments)]
    pub fn update_patch(
        &mut self,
        queue: &wgpu::Queue,
        terrain: &TerrainRenderer,
        camera_planet_position: DVec3,
        camera_forward: DVec3,
        camera_up_hint: DVec3,
        camera_altitude_meters: f64,
        viewport_size: [u32; 2],
        vertical_fov_radians: f64,
        simulation_time_seconds: f64,
    ) {
        if self.road_revision != terrain.road_revision() {
            self.road_revision = terrain.road_revision();
            self.gpu_generated_signature = None;
            self.patches.clear();
            self.proxy_patches.clear();
            self.empty_patch_keys.clear();
            self.pending_patch = None;
        }
        let camera_direction = camera_planet_position.normalize_or_zero();
        if camera_direction.length_squared() <= f64::EPSILON
            || !camera_altitude_meters.is_finite()
            || !simulation_time_seconds.is_finite()
        {
            return;
        }
        if !self.enabled {
            self.pending_patch = None;
            self.instance_count = 0;
            self.proxy_instance_count = 0;
            self.draw_lod_counts = TreeLodCounts::default();
            self.gpu_batches.clear();
            self.gpu_cell_count = 0;
            self.gpu_candidate_count = 0;
            return;
        }
        if camera_altitude_meters >= FOREST_DRAW_ALTITUDE_METERS {
            self.pending_patch = None;
            self.instance_count = 0;
            self.proxy_instance_count = 0;
            self.draw_lod_counts = TreeLodCounts::default();
            self.gpu_batches.clear();
            self.gpu_cell_count = 0;
            self.gpu_candidate_count = 0;
            return;
        }
        let local_surface_height_meters = terrain
            .surface_height_meters_at(camera_direction, camera_altitude_meters)
            .unwrap_or(0.0)
            .max(0.0);
        let camera_forward = camera_forward.normalize_or_zero();
        let camera_right = camera_forward.cross(camera_up_hint).normalize_or_zero();
        let camera_up = camera_right.cross(camera_forward).normalize_or_zero();
        let renderable_keys = forest_renderable_cell_keys_in_view(
            camera_planet_position,
            camera_forward,
            camera_right,
            camera_up,
            viewport_size,
            vertical_fov_radians,
            local_surface_height_meters,
        );
        let renderable_key_set = renderable_keys.iter().copied().collect::<BTreeSet<_>>();
        if self.gpu_resident {
            self.update_gpu_cells(queue, terrain, camera_planet_position, renderable_keys);
            return;
        }
        let cached_keys = forest_cell_keys_within_distance(
            camera_planet_position,
            FOREST_PREFETCH_DISTANCE_METERS,
            FOREST_MAX_CACHED_PATCHES,
            local_surface_height_meters,
        );
        if cached_keys.is_empty() {
            self.pending_patch = None;
            self.update_tree_lod(
                queue,
                camera_planet_position,
                viewport_size[1],
                vertical_fov_radians,
                &renderable_key_set,
            );
            return;
        }
        self.primary_patch_key = renderable_keys
            .first()
            .or_else(|| cached_keys.first())
            .copied();
        let cached_key_set = cached_keys.iter().copied().collect::<BTreeSet<_>>();
        self.patches.retain(|key, _| cached_key_set.contains(key));
        self.proxy_patches
            .retain(|key, _| cached_key_set.contains(key));
        self.empty_patch_keys
            .retain(|key| cached_key_set.contains(key));

        // A tiny, complete canopy proxy is much cheaper to resolve than the
        // 12,288-candidate individual population. Build nearest missing cells
        // first and publish each proxy only after every sample is available.
        // The permanent terrain canopy treatment remains underneath it.
        let missing_proxy_keys = cached_keys
            .iter()
            .copied()
            .filter(|key| {
                !self.proxy_patches.contains_key(key)
                    && !self.patches.contains_key(key)
                    && !self.empty_patch_keys.contains(key)
            })
            .take(FOREST_PROXY_PATCHES_PER_FRAME)
            .collect::<Vec<_>>();
        for key in missing_proxy_keys {
            if let Some(proxy) = build_forest_proxy_patch(key, terrain, camera_altitude_meters) {
                self.proxy_patches.insert(key, proxy);
            }
        }
        if self
            .pending_patch
            .as_ref()
            .is_some_and(|pending| !cached_key_set.contains(&pending.key))
        {
            self.pending_patch = None;
        }
        let missing_renderable_key = renderable_keys
            .iter()
            .copied()
            .find(|key| !self.patches.contains_key(key) && !self.empty_patch_keys.contains(key));
        if missing_renderable_key.is_some()
            && self
                .pending_patch
                .as_ref()
                .is_some_and(|pending| !renderable_key_set.contains(&pending.key))
        {
            self.pending_patch = None;
        }
        if self.pending_patch.is_none()
            && let Some(key) = missing_renderable_key.or_else(|| {
                cached_keys.iter().copied().find(|key| {
                    !self.patches.contains_key(key) && !self.empty_patch_keys.contains(key)
                })
            })
        {
            self.pending_patch = Some(PendingForestPatch::new(key));
        }
        let candidate_budget = if self.rebuild_count < FOREST_STARTUP_PATCH_COUNT {
            FOREST_INITIAL_PATCH_CANDIDATES_PER_FRAME
        } else if self
            .pending_patch
            .as_ref()
            .is_some_and(|pending| renderable_key_set.contains(&pending.key))
        {
            FOREST_PRIMARY_PATCH_CANDIDATES_PER_FRAME
        } else {
            FOREST_PATCH_CANDIDATES_PER_FRAME
        };
        let completed = self.pending_patch.as_mut().is_some_and(|pending| {
            pending.advance(terrain, camera_altitude_meters, candidate_budget)
        });
        if completed {
            let mut patch = self
                .pending_patch
                .take()
                .expect("completed forest patch is pending")
                .finish();
            let patch_key = patch.key;
            let tree_instances = patch.trees.len();
            let minimum_source_level = patch.minimum_source_level;
            if patch.trees.is_empty() {
                self.empty_patch_keys.insert(patch_key);
                self.proxy_patches.remove(&patch_key);
            } else {
                if self.rebuild_count < FOREST_STARTUP_PATCH_COUNT {
                    patch.visible_since = Instant::now()
                        .checked_sub(Duration::from_secs_f64(FOREST_PATCH_TRANSITION_SECONDS))
                        .unwrap_or_else(Instant::now);
                }
                self.patches.insert(patch_key, patch);
                self.rebuild_count += 1;
            }
            tracing::info!(
                target: "planet::forest",
                face = ?patch_key.face,
                level = patch_key.level,
                x = patch_key.x,
                y = patch_key.y,
                tree_instances,
                minimum_source_level = ?minimum_source_level,
                active_patches = self.patches.len(),
                "completed procedural forest cell"
            );
        }
        self.update_tree_lod(
            queue,
            camera_planet_position,
            viewport_size[1],
            vertical_fov_radians,
            &renderable_key_set,
        );
    }

    fn update_gpu_cells(
        &mut self,
        queue: &wgpu::Queue,
        terrain: &TerrainRenderer,
        camera_planet_position: DVec3,
        renderable_keys: Vec<TileKey>,
    ) {
        self.pending_patch = None;
        self.patches.clear();
        self.proxy_patches.clear();
        self.empty_patch_keys.clear();
        self.primary_patch_key = renderable_keys.first().copied();

        let mut grouped = BTreeMap::<(GpuForestTier, TileKey), Vec<GpuForestCell>>::new();
        let camera_altitude = camera_planet_position.length() - planet_radius_meters();
        let camera_height_above_ground = terrain
            .forest_surface_sample_at(camera_planet_position.normalize_or_zero(), camera_altitude)
            .map_or(0.0, |ground| camera_altitude - ground.height_meters);
        let force_nearest_full = camera_height_above_ground <= FOREST_GPU_FORCED_FULL_HEIGHT_METERS;
        let mut nearest_resident_cell = true;
        for key in renderable_keys {
            let Some((source_key, source_uv_scale, source_uv_offset)) =
                terrain.resident_forest_source(key)
            else {
                continue;
            };
            let centre_direction = forest_cell_centre_direction(key);
            let centre_distance =
                (centre_direction * planet_radius_meters() - camera_planet_position).length();
            let tier = if (nearest_resident_cell
                && force_nearest_full
                && centre_distance <= FOREST_GPU_FORCED_FULL_DISTANCE_METERS)
                || centre_distance <= FOREST_GPU_FULL_DISTANCE_METERS
            {
                GpuForestTier::Full
            } else if centre_distance <= FOREST_GPU_MEDIUM_DISTANCE_METERS {
                GpuForestTier::Medium
            } else {
                GpuForestTier::Sparse
            };
            nearest_resident_cell = false;
            let cells_per_axis = f64::from(1_u32 << key.level);
            let cell_span = 2.0 / cells_per_axis;
            let u_min = -1.0 + f64::from(key.x) * cell_span;
            let v_min = -1.0 + f64::from(key.y) * cell_span;
            grouped
                .entry((tier, source_key))
                .or_default()
                .push(GpuForestCell {
                    cell_uv_origin_span: [u_min as f32, v_min as f32, cell_span as f32, 0.0],
                    source_uv_scale_offset: [
                        source_uv_scale[0],
                        source_uv_scale[1],
                        source_uv_offset[0],
                        source_uv_offset[1],
                    ],
                    anchor_direction_source_level: [
                        centre_direction.x as f32,
                        centre_direction.y as f32,
                        centre_direction.z as f32,
                        f32::from(source_key.level),
                    ],
                    key: [key.face.index() as u32, key.x, key.y, 0],
                });
        }

        let mut cells = Vec::<GpuForestCell>::new();
        let mut batches = Vec::with_capacity(grouped.len());
        let mut candidate_count = 0_u32;
        for ((tier, source_key), batch_cells) in grouped {
            while !cells.len().is_multiple_of(self.gpu_cell_alignment) {
                cells.push(<GpuForestCell as bytemuck::Zeroable>::zeroed());
            }
            let first_cell = cells.len();
            let cell_count = batch_cells.len() as u32;
            let candidates_per_cell = tier.candidates_per_cell();
            cells.extend(
                batch_cells
                    .into_iter()
                    .enumerate()
                    .map(|(index, mut cell)| {
                        cell.key[3] =
                            candidate_count.saturating_add(index as u32 * candidates_per_cell);
                        cell
                    }),
            );
            candidate_count =
                candidate_count.saturating_add(cell_count.saturating_mul(candidates_per_cell));
            batches.push(GpuForestBatch {
                tier,
                source_key,
                dynamic_offset: (first_cell * size_of::<GpuForestCell>()) as u32,
                cell_count,
            });
        }
        debug_assert!(cells.len() <= self.gpu_cell_capacity);
        if !cells.is_empty() {
            queue.write_buffer(&self.gpu_cell_buffer, 0, bytemuck::cast_slice(&cells));
        }
        let signature = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytemuck::cast_slice::<_, u8>(&cells).hash(&mut hasher);
            for batch in &batches {
                (
                    batch.tier as u8,
                    batch.source_key,
                    batch.dynamic_offset,
                    batch.cell_count,
                )
                    .hash(&mut hasher);
            }
            hasher.finish()
        };
        if self.gpu_generated_signature != Some(signature)
            || (camera_planet_position - self.gpu_generated_camera).length()
                > FOREST_GPU_REGENERATE_DISTANCE_METERS
        {
            self.gpu_generated_signature = Some(signature);
            self.gpu_generated_camera = camera_planet_position;
            self.gpu_generation_needed = true;
        }
        self.gpu_minimum_source_level = batches.iter().map(|batch| batch.source_key.level).min();
        let active_sources = batches
            .iter()
            .map(|batch| batch.source_key)
            .collect::<BTreeSet<_>>();
        self.gpu_source_bind_groups
            .retain(|key, _| active_sources.contains(key));
        let forest_cell_binding_size =
            NonZeroU64::new((FOREST_MAX_RENDERABLE_PATCHES * size_of::<GpuForestCell>()) as u64)
                .expect("forest cell binding is non-empty");
        for source_key in active_sources {
            if self.gpu_source_bind_groups.contains_key(&source_key) {
                continue;
            }
            if let Some(bind_group) = terrain.create_forest_source_bind_group(
                &self.gpu_source_bind_group_layout,
                source_key,
                &self.uniform_buffer,
                &self.gpu_cell_buffer,
                forest_cell_binding_size,
                &self.instance_buffer,
            ) {
                self.gpu_source_bind_groups.insert(source_key, bind_group);
            }
        }
        self.gpu_cell_count = batches.iter().map(|batch| batch.cell_count).sum();
        self.gpu_candidate_count = candidate_count;
        self.instance_count = candidate_count;
        self.proxy_instance_count = 0;
        self.lod_counts = TreeLodCounts {
            full: batches
                .iter()
                .filter(|batch| batch.tier == GpuForestTier::Full)
                .map(|batch| batch.cell_count * batch.tier.candidates_per_cell())
                .sum(),
            medium: batches
                .iter()
                .filter(|batch| batch.tier == GpuForestTier::Medium)
                .map(|batch| batch.cell_count * batch.tier.candidates_per_cell())
                .sum(),
            sparse: batches
                .iter()
                .filter(|batch| batch.tier == GpuForestTier::Sparse)
                .map(|batch| batch.cell_count * batch.tier.candidates_per_cell())
                .sum(),
            zero: 0,
        };
        self.draw_lod_counts = self.lod_counts;
        self.gpu_batches = batches;
    }

    fn patch_transition_progress(&self) -> f64 {
        self.patches
            .values()
            .map(|patch| patch_transition_progress(patch.visible_since.elapsed().as_secs_f64()))
            .reduce(f64::min)
            .unwrap_or(1.0)
    }

    fn update_tree_lod(
        &mut self,
        queue: &wgpu::Queue,
        camera_planet_position: DVec3,
        viewport_height: u32,
        vertical_fov_radians: f64,
        renderable_keys: &BTreeSet<TileKey>,
    ) {
        let mut patches = self
            .patches
            .values()
            .filter(|patch| renderable_keys.contains(&patch.key))
            .collect::<Vec<_>>();
        patches.sort_by(|left, right| {
            right
                .centre_direction
                .dot(camera_planet_position)
                .total_cmp(&left.centre_direction.dot(camera_planet_position))
        });
        let mut full_instances = Vec::new();
        let mut medium_instances = Vec::new();
        let mut sparse_instances = Vec::new();
        let mut lod_counts = TreeLodCounts::default();
        let mut draw_lod_counts = TreeLodCounts::default();
        let mut proxy_instance_count = 0_u32;
        for (key, proxy) in &self.proxy_patches {
            if !renderable_keys.contains(key) {
                continue;
            }
            let finished_population_is_visible = self.patches.get(key).is_some_and(|patch| {
                patch_transition_progress(patch.visible_since.elapsed().as_secs_f64()) >= 1.0
            });
            if finished_population_is_visible {
                continue;
            }
            let proxy_count = append_forest_proxy_instances(
                &mut medium_instances,
                &proxy.trees,
                camera_planet_position,
            );
            proxy_instance_count += proxy_count;
            draw_lod_counts.medium += proxy_count;
            if medium_instances.len() >= FOREST_MAX_DRAW_INSTANCES {
                break;
            }
        }
        'patches: for patch in patches {
            let transition_progress =
                patch_transition_progress(patch.visible_since.elapsed().as_secs_f64()) as f32;
            append_lodded_tree_instances(
                &mut full_instances,
                &mut medium_instances,
                &mut sparse_instances,
                &mut lod_counts,
                &mut draw_lod_counts,
                &patch.trees,
                camera_planet_position,
                viewport_height,
                vertical_fov_radians,
                transition_progress,
            );
            if full_instances.len() + medium_instances.len() + sparse_instances.len()
                >= FOREST_MAX_DRAW_INSTANCES
            {
                break 'patches;
            }
        }
        let full_count = full_instances.len() as u32;
        let medium_count = medium_instances.len() as u32;
        let mut draw_instances = full_instances;
        draw_instances.append(&mut medium_instances);
        draw_instances.append(&mut sparse_instances);
        if self.draw_instances != draw_instances {
            debug_assert!(draw_instances.len() <= FOREST_MAX_DRAW_INSTANCES);
            if !draw_instances.is_empty() {
                queue.write_buffer(
                    &self.instance_buffer,
                    0,
                    bytemuck::cast_slice(&draw_instances),
                );
            }
            self.draw_instances = draw_instances;
        }
        self.instance_count = self.draw_instances.len() as u32;
        self.proxy_instance_count = proxy_instance_count.min(self.instance_count);
        self.lod_counts = lod_counts;
        draw_lod_counts.full = full_count;
        draw_lod_counts.medium = medium_count;
        self.draw_lod_counts = draw_lod_counts;
    }

    pub fn stats(&self) -> ForestStats {
        if self.gpu_resident {
            return ForestStats {
                patch_count: self.gpu_cell_count.min(u32::from(u16::MAX)) as u16,
                proxy_patch_count: 0,
                instances: self.gpu_candidate_count,
                proxy_instances: 0,
                full_instances: self.lod_counts.full,
                medium_instances: self.lod_counts.medium,
                sparse_instances: self.lod_counts.sparse,
                zero_instances: 0,
                rebuild_count: self.rebuild_count,
                patch_key: self.primary_patch_key,
                minimum_source_level: self.gpu_minimum_source_level,
                pending_candidates: 0,
                pending_candidates_total: 0,
                transition_progress: 1.0,
            };
        }
        ForestStats {
            patch_count: self.patches.len().min(usize::from(u16::MAX)) as u16,
            proxy_patch_count: self.proxy_patches.len().min(usize::from(u16::MAX)) as u16,
            instances: self.instance_count,
            proxy_instances: self.proxy_instance_count,
            full_instances: self.lod_counts.full,
            medium_instances: self.lod_counts.medium,
            sparse_instances: self.lod_counts.sparse,
            zero_instances: self.lod_counts.zero,
            rebuild_count: self.rebuild_count,
            patch_key: self.primary_patch_key,
            minimum_source_level: self
                .patches
                .values()
                .filter_map(|patch| patch.minimum_source_level)
                .min(),
            pending_candidates: self
                .pending_patch
                .as_ref()
                .map(|pending| pending.next_candidate as u32)
                .unwrap_or(0),
            pending_candidates_total: self
                .pending_patch
                .as_ref()
                .map(|pending| pending.candidates.len() as u32)
                .unwrap_or(0),
            transition_progress: self.patch_transition_progress() as f32,
        }
    }

    pub fn encode_gpu_generation(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera_bind_group: &wgpu::BindGroup,
        terrain: &TerrainRenderer,
    ) {
        if !self.enabled
            || !self.gpu_resident
            || self.gpu_batches.is_empty()
            || !self.gpu_generation_needed
        {
            return;
        }
        self.gpu_generation_needed = false;
        let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("immediate GPU forest generation"),
            timestamp_writes: None,
        });
        compute_pass.set_bind_group(0, camera_bind_group, &[]);
        compute_pass.set_bind_group(2, terrain.shared_bind_group(), &[]);
        for batch in &self.gpu_batches {
            let Some(source_bind_group) = self.gpu_source_bind_groups.get(&batch.source_key) else {
                continue;
            };
            let pipeline_index = match batch.tier {
                GpuForestTier::Full => 0,
                GpuForestTier::Medium => 1,
                GpuForestTier::Sparse => 2,
            };
            let invocation_count = batch.cell_count * batch.tier.candidates_per_cell();
            debug_assert_eq!(invocation_count % 64, 0);
            compute_pass.set_pipeline(&self.gpu_compute_pipelines[pipeline_index]);
            compute_pass.set_bind_group(1, source_bind_group, &[batch.dynamic_offset]);
            compute_pass.dispatch_workgroups(invocation_count / 64, 1, 1);
        }
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        weather_field_bind_group: &'pass wgpu::BindGroup,
        camera_altitude_meters: f64,
    ) {
        if !self.enabled || camera_altitude_meters >= FOREST_DRAW_ALTITUDE_METERS {
            return;
        }
        if self.gpu_resident {
            let full_end = self.draw_lod_counts.full.min(self.gpu_candidate_count);
            let medium_end = full_end
                .saturating_add(self.draw_lod_counts.medium)
                .min(self.gpu_candidate_count);
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, camera_bind_group, &[]);
            render_pass.set_bind_group(1, &self.bind_group, &[]);
            render_pass.set_bind_group(2, weather_field_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
            if full_end > 0 {
                render_pass
                    .set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..TREE_MESH_INDEX_COUNT, 0, 0..full_end);
            }
            if medium_end > full_end {
                render_pass.set_index_buffer(
                    self.medium_index_buffer.slice(..),
                    wgpu::IndexFormat::Uint16,
                );
                render_pass.draw_indexed(0..TREE_MESH_MEDIUM_INDEX_COUNT, 0, full_end..medium_end);
            }
            if self.gpu_candidate_count > medium_end {
                render_pass.set_index_buffer(
                    self.sparse_index_buffer.slice(..),
                    wgpu::IndexFormat::Uint16,
                );
                render_pass.draw_indexed(
                    0..TREE_MESH_SPARSE_INDEX_COUNT,
                    0,
                    medium_end..self.gpu_candidate_count,
                );
            }
            return;
        }
        if self.instance_count == 0 {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_bind_group(2, weather_field_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        let full_end = self.draw_lod_counts.full.min(self.instance_count);
        let medium_end = full_end
            .saturating_add(self.draw_lod_counts.medium)
            .min(self.instance_count);
        if full_end > 0 {
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..TREE_MESH_INDEX_COUNT, 0, 0..full_end);
        }
        if medium_end > full_end {
            render_pass.set_index_buffer(
                self.medium_index_buffer.slice(..),
                wgpu::IndexFormat::Uint16,
            );
            render_pass.draw_indexed(0..TREE_MESH_MEDIUM_INDEX_COUNT, 0, full_end..medium_end);
        }
        if self.instance_count > medium_end {
            render_pass.set_index_buffer(
                self.sparse_index_buffer.slice(..),
                wgpu::IndexFormat::Uint16,
            );
            render_pass.draw_indexed(
                0..TREE_MESH_SPARSE_INDEX_COUNT,
                0,
                medium_end..self.instance_count,
            );
        }
    }
}

fn tree_mesh_indices() -> Vec<u16> {
    let mut indices = Vec::with_capacity(TREE_MESH_INDEX_COUNT as usize);
    append_tree_wood_indices(&mut indices, 1);
    append_tree_crown_indices(&mut indices, 2, 1);
    indices
}

fn tree_mesh_medium_indices() -> Vec<u16> {
    let mut indices = Vec::with_capacity(TREE_MESH_MEDIUM_INDEX_COUNT as usize);
    append_tree_wood_indices(&mut indices, 3);
    append_tree_crown_indices(&mut indices, 4, 2);
    indices
}

fn tree_mesh_sparse_indices() -> Vec<u16> {
    let mut indices = Vec::with_capacity(TREE_MESH_SPARSE_INDEX_COUNT as usize);
    append_tree_wood_indices(&mut indices, 3);
    append_tree_crown_indices(&mut indices, 6, 2);
    indices
}

fn append_tree_wood_indices(indices: &mut Vec<u16>, side_step: u16) {
    for side in (0_u16..6).step_by(usize::from(side_step)) {
        let bottom = side * 2;
        let top = bottom + 1;
        let next_bottom = ((side + side_step) % 6) * 2;
        let next_top = next_bottom + 1;
        indices.extend_from_slice(&[bottom, next_bottom, next_top, bottom, next_top, top]);
    }
    for branch in 0_u16..4 {
        let base = 12 + branch * 12;
        for side in (0_u16..6).step_by(usize::from(side_step)) {
            let start = base + side;
            let next_start = base + (side + side_step) % 6;
            let end = base + 6 + side;
            let next_end = base + 6 + (side + side_step) % 6;
            indices.extend_from_slice(&[start, next_start, next_end, start, next_end, end]);
        }
    }
}

fn append_tree_crown_indices(indices: &mut Vec<u16>, side_step: u16, ring_step: u16) {
    for cluster in 0_u16..10 {
        let crown_top = 60 + cluster * 38;
        let first_ring = crown_top + 1;
        let crown_bottom = crown_top + 37;
        for side in (0_u16..12).step_by(usize::from(side_step)) {
            let next_side = (side + side_step) % 12;
            indices.extend_from_slice(&[crown_top, first_ring + side, first_ring + next_side]);
            for ring in (0_u16..2).step_by(usize::from(ring_step)) {
                let upper_ring_index = (ring + ring_step).min(2);
                let lower_ring = first_ring + ring * 12;
                let upper_ring = first_ring + upper_ring_index * 12;
                let lower = lower_ring + side;
                let next_lower = lower_ring + next_side;
                let upper = upper_ring + side;
                let next_upper = upper_ring + next_side;
                indices
                    .extend_from_slice(&[lower, next_lower, next_upper, lower, next_upper, upper]);
            }
            let last_ring = first_ring + 2 * 12;
            indices.extend_from_slice(&[crown_bottom, last_ring + next_side, last_ring + side]);
        }
    }
}

fn forest_cell_key(direction: DVec3) -> TileKey {
    tile_key_for_direction(direction.normalize(), FOREST_CELL_LEVEL)
}

fn forest_cell_centre_direction(key: TileKey) -> DVec3 {
    let cells_per_axis = f64::from(1_u32 << key.level);
    let cell_span = 2.0 / cells_per_axis;
    let u = -1.0 + (f64::from(key.x) + 0.5) * cell_span;
    let v = -1.0 + (f64::from(key.y) + 0.5) * cell_span;
    face_uv_to_direction(key.face, u, v)
}

fn forest_renderable_cell_keys(
    camera_planet_position: DVec3,
    viewport_height: u32,
    vertical_fov_radians: f64,
    local_surface_height_meters: f64,
) -> Vec<TileKey> {
    let visibility_distance =
        maximum_tree_visibility_distance_meters(viewport_height, vertical_fov_radians);
    forest_cell_keys_within_distance(
        camera_planet_position,
        visibility_distance,
        FOREST_MAX_RENDERABLE_PATCHES,
        local_surface_height_meters,
    )
}

fn forest_renderable_cell_keys_in_view(
    camera_planet_position: DVec3,
    camera_forward: DVec3,
    camera_right: DVec3,
    camera_up: DVec3,
    viewport_size: [u32; 2],
    vertical_fov_radians: f64,
    local_surface_height_meters: f64,
) -> Vec<TileKey> {
    let keys = forest_renderable_cell_keys(
        camera_planet_position,
        viewport_size[1],
        vertical_fov_radians,
        local_surface_height_meters,
    );
    if viewport_size[0] == 0
        || viewport_size[1] == 0
        || !vertical_fov_radians.is_finite()
        || vertical_fov_radians <= 0.0
        || vertical_fov_radians >= std::f64::consts::PI
        || camera_forward.length_squared() <= f64::EPSILON
        || camera_right.length_squared() <= f64::EPSILON
        || camera_up.length_squared() <= f64::EPSILON
    {
        return keys;
    }
    let half_vertical_tangent = (vertical_fov_radians * 0.5).tan();
    let half_horizontal_tangent =
        half_vertical_tangent * f64::from(viewport_size[0]) / f64::from(viewport_size[1]);
    keys.into_iter()
        .filter(|key| {
            forest_cell_intersects_camera_frustum(
                *key,
                camera_planet_position,
                camera_forward,
                camera_right,
                camera_up,
                half_horizontal_tangent,
                half_vertical_tangent,
                local_surface_height_meters,
            )
        })
        .collect()
}

fn forest_cell_intersects_camera_frustum(
    key: TileKey,
    camera_planet_position: DVec3,
    camera_forward: DVec3,
    camera_right: DVec3,
    camera_up: DVec3,
    half_horizontal_tangent: f64,
    half_vertical_tangent: f64,
    local_surface_height_meters: f64,
) -> bool {
    let centre = forest_cell_centre_direction(key)
        * (planet_radius_meters() + local_surface_height_meters.max(0.0));
    let relative = centre - camera_planet_position;
    let cell_radius = planet_radius_meters() * forest_cell_angular_radius(key)
        + f64::from(TREE_HEIGHT_MIN_METERS + TREE_HEIGHT_RANGE_METERS)
        + 100.0;
    let forward_distance = relative.dot(camera_forward);
    if forward_distance + cell_radius < 0.0 {
        return false;
    }
    let visible_depth = forward_distance.max(0.0);
    let horizontal_extent = relative.dot(camera_right).abs();
    if horizontal_extent > visible_depth * half_horizontal_tangent + cell_radius {
        return false;
    }
    let vertical_extent = relative.dot(camera_up).abs();
    vertical_extent <= visible_depth * half_vertical_tangent + cell_radius
}

fn forest_cell_keys_within_distance(
    camera_planet_position: DVec3,
    visibility_distance_meters: f64,
    maximum_keys: usize,
    local_surface_height_meters: f64,
) -> Vec<TileKey> {
    let camera_direction = camera_planet_position.normalize_or_zero();
    let camera_radius = camera_planet_position.length();
    if camera_direction.length_squared() <= f64::EPSILON || maximum_keys == 0 {
        return Vec::new();
    }
    let Some(angular_radius) = forest_surface_angular_radius(
        camera_radius,
        visibility_distance_meters,
        local_surface_height_meters,
    ) else {
        return Vec::new();
    };
    let cell_radius_bound = std::f64::consts::SQRT_2 / f64::from(1_u32 << FOREST_CELL_LEVEL);
    let search_radius = angular_radius + cell_radius_bound;
    let sample_spacing = 0.4 * 2.0 / (f64::from(1_u32 << FOREST_CELL_LEVEL) * 3.0_f64.sqrt());
    let sample_extent = (search_radius / sample_spacing).ceil() as i32 + 1;
    let reference = if camera_direction.y.abs() < 0.9 {
        DVec3::Y
    } else {
        DVec3::X
    };
    let tangent_u = camera_direction.cross(reference).normalize();
    let tangent_v = camera_direction.cross(tangent_u).normalize();
    let mut sampled_keys = BTreeSet::new();
    sampled_keys.insert(forest_cell_key(camera_direction));
    for y in -sample_extent..=sample_extent {
        for x in -sample_extent..=sample_extent {
            let offset_u = f64::from(x) * sample_spacing;
            let offset_v = f64::from(y) * sample_spacing;
            if offset_u.hypot(offset_v) > search_radius + sample_spacing {
                continue;
            }
            sampled_keys.insert(forest_cell_key(
                (camera_direction + tangent_u * offset_u + tangent_v * offset_v).normalize(),
            ));
        }
    }
    let mut keys = sampled_keys
        .into_iter()
        .filter_map(|key| {
            let distance = camera_direction
                .angle_between(forest_cell_centre_direction(key))
                .abs();
            (distance <= angular_radius + forest_cell_angular_radius(key))
                .then_some((distance, key))
        })
        .collect::<Vec<_>>();
    keys.sort_by(|(left_distance, left_key), (right_distance, right_key)| {
        left_distance
            .total_cmp(right_distance)
            .then_with(|| left_key.cmp(right_key))
    });
    keys.truncate(maximum_keys);
    keys.into_iter().map(|(_, key)| key).collect()
}

fn maximum_tree_visibility_distance_meters(viewport_height: u32, vertical_fov_radians: f64) -> f64 {
    if viewport_height == 0
        || !vertical_fov_radians.is_finite()
        || vertical_fov_radians <= 0.0
        || vertical_fov_radians >= std::f64::consts::PI
    {
        return 0.0;
    }
    let maximum_tree_height = f64::from(TREE_HEIGHT_MIN_METERS + TREE_HEIGHT_RANGE_METERS);
    (maximum_tree_height * f64::from(viewport_height)
        / (2.0 * (vertical_fov_radians * 0.5).tan() * TREE_LOD_SPARSE_PIXELS))
        .clamp(0.0, FOREST_TREE_RENDER_DISTANCE_METERS)
}

fn forest_surface_angular_radius(
    camera_radius_meters: f64,
    visibility_distance_meters: f64,
    local_surface_height_meters: f64,
) -> Option<f64> {
    if !camera_radius_meters.is_finite()
        || !visibility_distance_meters.is_finite()
        || camera_radius_meters <= 0.0
        || visibility_distance_meters <= 0.0
    {
        return None;
    }
    let tree_radius = planet_radius_meters()
        + local_surface_height_meters.max(0.0)
        + f64::from(TREE_HEIGHT_MIN_METERS + TREE_HEIGHT_RANGE_METERS);
    if visibility_distance_meters < (camera_radius_meters - tree_radius).abs() {
        return None;
    }
    let cosine = (camera_radius_meters * camera_radius_meters + tree_radius * tree_radius
        - visibility_distance_meters * visibility_distance_meters)
        / (2.0 * camera_radius_meters * tree_radius);
    Some(cosine.clamp(-1.0, 1.0).acos())
}

fn forest_cell_angular_radius(key: TileKey) -> f64 {
    let cells_per_axis = f64::from(1_u32 << key.level);
    let cell_span = 2.0 / cells_per_axis;
    let u_min = -1.0 + f64::from(key.x) * cell_span;
    let v_min = -1.0 + f64::from(key.y) * cell_span;
    let centre = forest_cell_centre_direction(key);
    [
        (u_min, v_min),
        (u_min + cell_span, v_min),
        (u_min, v_min + cell_span),
        (u_min + cell_span, v_min + cell_span),
    ]
    .into_iter()
    .map(|(u, v)| {
        centre
            .angle_between(face_uv_to_direction(key.face, u, v))
            .abs()
    })
    .fold(0.0, f64::max)
}

fn forest_placement_density_at(direction: DVec3) -> f64 {
    forest_density_at(direction)
}

fn forest_patch_tree_layouts(key: TileKey) -> Vec<(DVec3, TreeLayout)> {
    (0..TREE_COUNT)
        .map(|index| forest_patch_tree_layout(key, index))
        .collect()
}

fn forest_patch_tree_layout(key: TileKey, index: usize) -> (DVec3, TreeLayout) {
    let cells_per_axis = f64::from(1_u32 << key.level);
    let cell_span = 2.0 / cells_per_axis;
    let u_min = -1.0 + f64::from(key.x) * cell_span;
    let v_min = -1.0 + f64::from(key.y) * cell_span;
    let cell_seed = canonical_cell_seed(key);
    let index = index as u32;
    // unit_hash is half-open, so a candidate belongs to exactly this cell even
    // at cube-face and cell boundaries.
    let u = u_min + unit_hash(cell_seed ^ index ^ 0x6a09_e667) * cell_span;
    let v = v_min + unit_hash(cell_seed ^ index ^ 0xbb67_ae85) * cell_span;
    (
        face_uv_to_direction(key.face, u, v),
        tree_layout_from_seed(cell_seed ^ index),
    )
}

fn build_forest_proxy_patch(
    key: TileKey,
    terrain: &TerrainRenderer,
    camera_altitude_meters: f64,
) -> Option<ForestProxyPatch> {
    let mut trees =
        Vec::with_capacity(FOREST_PROXY_CANDIDATES_PER_PATCH * FOREST_PROXY_CARDS_PER_SAMPLE);
    for proxy_index in 0..FOREST_PROXY_CANDIDATES_PER_PATCH {
        // Spread the proxy samples over the complete deterministic population;
        // taking its first N entries would produce a spatially biased card.
        let candidate_index = proxy_index * TREE_COUNT / FOREST_PROXY_CANDIDATES_PER_PATCH;
        let (direction, layout) = forest_patch_tree_layout(key, candidate_index);
        let sample = terrain.forest_surface_sample_at(direction, camera_altitude_meters)?;
        let density = forest_placement_density_at(direction);
        // A single proxy card represents several final trees, so select by
        // sqrt(density): card count times card footprint then follows the full
        // population's approximately linear density without leaving holes.
        if !forest_surface_is_eligible(
            sample,
            FOREST_MINIMUM_MOISTURE,
            FOREST_MAXIMUM_SLOPE_RADIANS,
        ) || terrain.road_clears_tree(direction, sample.height_meters)
            || f64::from(layout.seed) > density.sqrt()
        {
            continue;
        }
        let width_meters = layout.width_meters * FOREST_PROXY_WIDTH_SCALE;
        let up = direction.normalize();
        let reference = if up.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
        let tangent = up.cross(reference).normalize();
        let bitangent = up.cross(tangent).normalize();
        let offsets = [(0.0, 0.0), (-0.42, 0.24), (0.38, -0.30)];
        for (card_index, (tangent_offset, bitangent_offset)) in offsets.into_iter().enumerate() {
            let card_direction = (up
                + tangent * (tangent_offset * f64::from(width_meters) / planet_radius_meters())
                + bitangent
                    * (bitangent_offset * f64::from(width_meters) / planet_radius_meters()))
            .normalize();
            let extra_radius = (f64::from(width_meters) * 0.7 - 16.0).max(0.0);
            let blocked = terrain.road_clears_tree(card_direction, sample.height_meters)
                || (extra_radius > 0.0
                    && [tangent, -tangent, bitangent, -bitangent].into_iter().any(|axis| {
                        terrain.road_clears_tree(
                            (card_direction + axis * (extra_radius / planet_radius_meters()))
                                .normalize(),
                            sample.height_meters,
                        )
                    }));
            if blocked {
                continue;
            }
            let centre = card_direction
                * (planet_radius_meters() + sample.height_meters
                    - tree_base_sink_meters(width_meters, sample.slope_radians));
            trees.push(TreeInstance {
                centre_and_height: [
                    centre.x as f32,
                    centre.y as f32,
                    centre.z as f32,
                    layout.height_meters * FOREST_PROXY_HEIGHT_SCALE,
                ],
                width_shade_kind_seed: [
                    width_meters,
                    layout.shade,
                    2.0 + if forest_biome_requires_evergreen(sample.biome) {
                        1.0
                    } else {
                        layout.kind
                    },
                    (layout.seed + card_index as f32 * 0.173).fract(),
                ],
            });
        }
    }
    Some(ForestProxyPatch { trees })
}

fn canonical_cell_seed(key: TileKey) -> u32 {
    hash_u32(
        FOREST_PLANET_SEED
            ^ u32::from(key.face.index()).wrapping_mul(0x9e37_79b9)
            ^ u32::from(key.level).wrapping_mul(0x85eb_ca6b)
            ^ key.x.wrapping_mul(0xc2b2_ae35)
            ^ key.y.wrapping_mul(0x27d4_eb2f),
    )
}

fn tree_layout_from_seed(seed: u32) -> TreeLayout {
    let height = TREE_HEIGHT_MIN_METERS + hash(seed ^ 0xa511_e9b3) * TREE_HEIGHT_RANGE_METERS;
    let width = height * (0.32 + hash(seed ^ 0x63d8_3595) * 0.18);
    TreeLayout {
        height_meters: height,
        width_meters: width,
        shade: 0.82 + hash(seed ^ 0x9e37_79b9) * 0.34,
        // Temperate/tropical woodland chooses deterministically between
        // broadleaf and evergreen; cold biomes override this to evergreen when
        // the sampled terrain biome is known.
        kind: if hash(seed ^ 0x27d4_eb2f) < 0.28 {
            1.0
        } else {
            0.0
        },
        seed: hash(seed ^ 0x1656_67b1),
    }
}

fn tree_base_sink_meters(width_meters: f32, slope_radians: f64) -> f64 {
    TREE_BASE_SINK_METERS
        + f64::from(width_meters)
            * 0.5
            * slope_radians.clamp(0.0, FOREST_MAXIMUM_SLOPE_RADIANS).tan()
}

/// High-resolution, seam-safe density field mirrored by the terrain canopy
/// material. The floor keeps every eligible cold/forest cell capable of
/// producing a sparse stand; the field creates local clearings and denser
/// woodland inside the coarse baked biome footprint.
fn forest_density_at(direction: DVec3) -> f64 {
    let value = forest_noise_at(direction, FOREST_DENSITY_FREQUENCY) * 0.5 + 0.5;
    let cluster = smoothstep01((value - 0.28) / (0.72 - 0.28));
    0.04 + cluster * 0.96
}

fn forest_noise_at(direction: DVec3, frequency: f64) -> f64 {
    let position = direction.normalize() * frequency;
    let cell = [
        position.x.floor() as i32,
        position.y.floor() as i32,
        position.z.floor() as i32,
    ];
    let fraction = [
        position.x - f64::from(cell[0]),
        position.y - f64::from(cell[1]),
        position.z - f64::from(cell[2]),
    ];
    forest_value_noise(cell, fraction)
}

fn forest_value_noise(cell: [i32; 3], fraction: [f64; 3]) -> f64 {
    let fade = fraction.map(|amount| amount * amount * (3.0 - amount * 2.0));
    let hash_axis = |coordinate: i32, salt: u32| {
        (
            detail_mix((coordinate as u32) ^ salt),
            detail_mix((coordinate.wrapping_add(1) as u32) ^ salt),
        )
    };
    let x = hash_axis(cell[0], 0x27d4_eb2f);
    let y = hash_axis(cell[1], 0x9e37_79b9);
    let z = hash_axis(cell[2], 0x85eb_ca6b);
    let corner = |x: u32, y: u32, z: u32| {
        let combined = detail_avalanche(x ^ y.rotate_left(11) ^ z.rotate_left(22));
        f64::from(combined >> 8) * (2.0 / 16_777_216.0) - 1.0
    };
    let a = corner(x.0, y.0, z.0);
    let b = corner(x.1, y.0, z.0);
    let c = corner(x.0, y.1, z.0);
    let d = corner(x.1, y.1, z.0);
    let e = corner(x.0, y.0, z.1);
    let f = corner(x.1, y.0, z.1);
    let g = corner(x.0, y.1, z.1);
    let h = corner(x.1, y.1, z.1);
    let k1 = b - a;
    let k2 = c - a;
    let k3 = e - a;
    let k4 = a - b - c + d;
    let k5 = a - c - e + g;
    let k6 = a - b - e + f;
    let k7 = -a + b + c - d + e - f - g + h;
    a + k1 * fade[0]
        + k2 * fade[1]
        + k3 * fade[2]
        + k4 * fade[0] * fade[1]
        + k5 * fade[1] * fade[2]
        + k6 * fade[2] * fade[0]
        + k7 * fade[0] * fade[1] * fade[2]
}

fn detail_mix(value: u32) -> u32 {
    let mut value = value.wrapping_mul(0x9e37_79b1);
    value ^= value >> 15;
    value
}

fn detail_avalanche(value: u32) -> u32 {
    let mut value = value.wrapping_mul(0x85eb_ca6b);
    value ^= value >> 16;
    value
}

#[cfg(test)]
fn projected_tree_height_pixels(
    tree: TreeInstance,
    camera_planet_position: DVec3,
    viewport_height: u32,
    vertical_fov_radians: f64,
) -> f64 {
    projected_tree_height_pixels_at_distance(
        tree,
        tree_distance_meters(tree, camera_planet_position),
        viewport_height,
        vertical_fov_radians,
    )
}

fn tree_distance_meters(tree: TreeInstance, camera_planet_position: DVec3) -> f64 {
    camera_planet_position
        .distance(DVec3::from_array([
            f64::from(tree.centre_and_height[0]),
            f64::from(tree.centre_and_height[1]),
            f64::from(tree.centre_and_height[2]),
        ]))
        .max(1.0)
}

fn projected_tree_height_pixels_at_distance(
    tree: TreeInstance,
    distance_meters: f64,
    viewport_height: u32,
    vertical_fov_radians: f64,
) -> f64 {
    if viewport_height == 0
        || !vertical_fov_radians.is_finite()
        || vertical_fov_radians <= 0.0
        || vertical_fov_radians >= std::f64::consts::PI
    {
        return 0.0;
    }
    f64::from(tree.centre_and_height[3].max(0.0)) * f64::from(viewport_height)
        / (2.0 * (vertical_fov_radians * 0.5).tan() * distance_meters)
}

fn tree_lod(projected_pixels: f64) -> TreeLod {
    if projected_pixels >= TREE_LOD_FULL_PIXELS {
        TreeLod::Full
    } else if projected_pixels >= TREE_LOD_MEDIUM_PIXELS {
        TreeLod::Medium
    } else if projected_pixels >= TREE_LOD_SPARSE_PIXELS {
        TreeLod::Sparse
    } else {
        TreeLod::Zero
    }
}

#[cfg(test)]
fn lodded_tree_instance(tree: TreeInstance, projected_pixels: f64) -> Option<TreeInstance> {
    lodded_tree_instance_with_scale(tree, projected_pixels, 1.0)
}

fn lodded_tree_instance_with_scale(
    tree: TreeInstance,
    projected_pixels: f64,
    population_scale: f32,
) -> Option<TreeInstance> {
    let population_scale = population_scale.clamp(0.0, 1.0);
    let density = tree_lod_density(projected_pixels) * population_scale;
    if tree.width_shade_kind_seed[3] >= density {
        return None;
    }
    let scale = tree_lod_scale(projected_pixels);
    let mut instance = tree;
    instance.centre_and_height[3] *= scale;
    instance.width_shade_kind_seed[0] *= scale;
    Some(instance)
}

fn tree_lod_density(projected_pixels: f64) -> f32 {
    match tree_lod(projected_pixels) {
        TreeLod::Full => 1.0,
        TreeLod::Medium => {
            let progress = smoothstep01(
                (projected_pixels - TREE_LOD_MEDIUM_PIXELS)
                    / (TREE_LOD_FULL_PIXELS - TREE_LOD_MEDIUM_PIXELS),
            );
            0.5 + 0.5 * progress as f32
        }
        TreeLod::Sparse => {
            let progress = smoothstep01(
                (projected_pixels - TREE_LOD_SPARSE_PIXELS)
                    / (TREE_LOD_MEDIUM_PIXELS - TREE_LOD_SPARSE_PIXELS),
            );
            TREE_LOD_PLACEHOLDER_DENSITY + (0.5 - TREE_LOD_PLACEHOLDER_DENSITY) * progress as f32
        }
        // Keep a small deterministic population as tiny placeholders.  The
        // density meets the sparse branch at one projected pixel, so moving
        // across the threshold cannot expose a hard edge in the forest.
        TreeLod::Zero => TREE_LOD_PLACEHOLDER_DENSITY,
    }
}

fn tree_lod_scale(projected_pixels: f64) -> f32 {
    match tree_lod(projected_pixels) {
        TreeLod::Full => 1.0,
        TreeLod::Medium => {
            let progress = smoothstep01(
                (projected_pixels - TREE_LOD_MEDIUM_PIXELS)
                    / (TREE_LOD_FULL_PIXELS - TREE_LOD_MEDIUM_PIXELS),
            );
            0.35 + 0.65 * progress as f32
        }
        TreeLod::Sparse => {
            let progress = smoothstep01(
                (projected_pixels - TREE_LOD_SPARSE_PIXELS)
                    / (TREE_LOD_MEDIUM_PIXELS - TREE_LOD_SPARSE_PIXELS),
            );
            TREE_LOD_PLACEHOLDER_SCALE + (0.35 - TREE_LOD_PLACEHOLDER_SCALE) * progress as f32
        }
        TreeLod::Zero => TREE_LOD_PLACEHOLDER_SCALE,
    }
}

fn append_lodded_tree_instances(
    full_instances: &mut Vec<TreeInstance>,
    medium_instances: &mut Vec<TreeInstance>,
    sparse_instances: &mut Vec<TreeInstance>,
    lod_counts: &mut TreeLodCounts,
    draw_lod_counts: &mut TreeLodCounts,
    trees: &[TreeInstance],
    camera_planet_position: DVec3,
    viewport_height: u32,
    vertical_fov_radians: f64,
    population_scale: f32,
) {
    for tree in trees {
        if full_instances.len() + medium_instances.len() + sparse_instances.len()
            == FOREST_MAX_DRAW_INSTANCES
        {
            break;
        }
        let distance_meters = tree_distance_meters(*tree, camera_planet_position);
        let projected_pixels = if distance_meters <= FOREST_TREE_RENDER_DISTANCE_METERS {
            projected_tree_height_pixels_at_distance(
                *tree,
                distance_meters,
                viewport_height,
                vertical_fov_radians,
            )
        } else {
            0.0
        };
        let lod = tree_lod(projected_pixels);
        lod_counts.add(lod);
        if let Some(tree) =
            lodded_tree_instance_with_scale(*tree, projected_pixels, population_scale)
        {
            match lod {
                TreeLod::Full => {
                    full_instances.push(tree);
                    draw_lod_counts.full += 1;
                }
                TreeLod::Medium => {
                    medium_instances.push(tree);
                    draw_lod_counts.medium += 1;
                }
                TreeLod::Sparse | TreeLod::Zero => {
                    sparse_instances.push(tree);
                    draw_lod_counts.sparse += 1;
                }
            }
        }
    }
}

fn append_forest_proxy_instances(
    draw_instances: &mut Vec<TreeInstance>,
    trees: &[TreeInstance],
    camera_planet_position: DVec3,
) -> u32 {
    let start = draw_instances.len();
    for tree in trees {
        if draw_instances.len() == FOREST_MAX_DRAW_INSTANCES {
            break;
        }
        if tree_distance_meters(*tree, camera_planet_position) <= FOREST_TREE_RENDER_DISTANCE_METERS
        {
            draw_instances.push(*tree);
        }
    }
    (draw_instances.len() - start) as u32
}

fn patch_transition_progress(elapsed_seconds: f64) -> f64 {
    smoothstep01(elapsed_seconds / FOREST_PATCH_TRANSITION_SECONDS)
}

fn pending_batch_end(
    next_candidate: usize,
    candidate_count: usize,
    candidate_budget: usize,
) -> usize {
    next_candidate
        .saturating_add(candidate_budget)
        .min(candidate_count)
}

fn smoothstep01(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn hash_u32(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value
}

fn hash(value: u32) -> f32 {
    hash_u32(value) as f32 / u32::MAX as f32
}

fn unit_hash(value: u32) -> f64 {
    f64::from(hash_u32(value)) / (f64::from(u32::MAX) + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forest_layout_is_deterministic_cell_owned_and_mixed() {
        let key = forest_cell_key(FOREST_CENTRE_DIRECTION);
        let trees = forest_patch_tree_layouts(key);
        assert_eq!(trees.len(), TREE_COUNT);
        assert!(trees.iter().all(|(direction, layout)| {
            forest_cell_key(*direction) == key
                && (layout.kind == 0.0 || layout.kind == 1.0)
                && layout.height_meters >= TREE_HEIGHT_MIN_METERS
        }));
        assert!(trees.iter().any(|(_, layout)| layout.kind == 0.0));
        assert!(trees.iter().any(|(_, layout)| layout.kind == 1.0));
        let density = forest_density_at(FOREST_CENTRE_DIRECTION);
        assert!((0.0..=1.0).contains(&density));
    }

    #[test]
    fn proxy_samples_are_bounded_spatially_distributed_and_atomic() {
        assert_eq!(FOREST_PROXY_CANDIDATES_PER_PATCH, 128);
        assert_eq!(FOREST_PROXY_PATCHES_PER_FRAME, 2);
        let key = forest_cell_key(FOREST_CENTRE_DIRECTION);
        let samples = (0..FOREST_PROXY_CANDIDATES_PER_PATCH)
            .map(|proxy_index| {
                forest_patch_tree_layout(
                    key,
                    proxy_index * TREE_COUNT / FOREST_PROXY_CANDIDATES_PER_PATCH,
                )
                .0
            })
            .collect::<Vec<_>>();
        assert!(
            samples
                .iter()
                .all(|direction| forest_cell_key(*direction) == key)
        );
        assert!(samples.windows(2).all(|pair| pair[0] != pair[1]));

        let source = include_str!("forest.rs");
        let update_lod = source
            .split("fn update_tree_lod(")
            .nth(1)
            .and_then(|source| source.split("pub fn stats(").next())
            .expect("forest LOD update is present");
        assert!(update_lod.contains("append_forest_proxy_instances"));
        assert!(!update_lod.contains("pending_patch"));
    }

    #[test]
    fn forest_placement_has_no_radial_cell_mask() {
        let key = forest_cell_key(FOREST_CENTRE_DIRECTION);
        let cells_per_axis = f64::from(1_u32 << key.level);
        let cell_span = 2.0 / cells_per_axis;
        let u_min = -1.0 + f64::from(key.x) * cell_span;
        let v_min = -1.0 + f64::from(key.y) * cell_span;
        let centre = forest_cell_centre_direction(key);
        let near_corner =
            face_uv_to_direction(key.face, u_min + cell_span * 0.99, v_min + cell_span * 0.99);
        assert_eq!(
            forest_placement_density_at(centre),
            forest_density_at(centre)
        );
        assert_eq!(
            forest_placement_density_at(near_corner),
            forest_density_at(near_corner)
        );
    }

    #[test]
    fn tree_mesh_instances_keep_human_scale_variation() {
        let layouts = (0..256).map(tree_layout_from_seed).collect::<Vec<_>>();
        assert_eq!(TREE_HEIGHT_MIN_METERS, 22.0);
        assert_eq!(TREE_HEIGHT_MIN_METERS + TREE_HEIGHT_RANGE_METERS, 48.0);
        assert!(layouts.iter().all(|layout| {
            layout.height_meters >= 22.0
                && layout.height_meters < 48.0
                && layout.width_meters >= layout.height_meters * 0.32
                && layout.width_meters < layout.height_meters * 0.50
        }));
    }

    #[test]
    fn slope_sink_covers_the_complete_tree_base() {
        let width_meters = 24.0;
        let slope_radians = FOREST_MAXIMUM_SLOPE_RADIANS;
        let required_sink =
            TREE_BASE_SINK_METERS + f64::from(width_meters) * 0.5 * slope_radians.tan();
        assert!(
            tree_base_sink_meters(width_meters, slope_radians) >= required_sink,
            "the downhill tree edge must not hover"
        );
        assert_eq!(
            tree_base_sink_meters(width_meters, 0.0),
            TREE_BASE_SINK_METERS
        );
    }

    #[test]
    fn terrain_darkening_filters_subpixel_tree_footprints_at_distance() {
        let shader = include_str!("planet.wgsl");
        assert!(shader.contains("const FOREST_DENSITY_FREQUENCY: f32 = 8192.0;"));
        assert!(shader.contains("fn forest_density_at_direction(direction: vec3<f32>)"));
        assert!(shader.contains("fn forest_ground_darkening(direction: vec3<f32>, density: f32)"));
        let canopy = shader
            .split("fn forest_canopy_albedo(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("forest canopy ground treatment is present");
        assert!(canopy.contains("camera_distance_meters"));
        assert!(canopy.contains("let forest_density = forest_density_at_direction(direction);"));
        assert!(canopy.contains("if !outmap || !forest_surface_owns_trees("));
        assert!(canopy.contains("let density_weight = canopy_weight * forest_density;"));
        assert!(canopy.contains("let distant_ground_darkening = min("));
        assert!(canopy.contains("let visible_population = forest_visible_population("));
        assert!(canopy.contains("forest_density * visible_population"));
        assert!(canopy.contains("* visible_population,"));
        assert!(canopy.contains("let point_field_weight = 1.0 - smoothstep("));
        assert!(canopy.contains("7000.0"));
        assert!(canopy.contains("FOREST_GROUND_DARKENING_MAX"));
        let flat = shader
            .split("fn flat_triangle_colour(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("flat triangle material path is present");
        assert!(flat.contains("let material_source_uv = input.source_uv;"));
        assert!(flat.contains("sample_biome(flat_outmap, material_source_uv, direction)"));
        assert!(flat.contains("sample_moisture(material_source_uv)"));
        assert!(!flat.contains("sample_biome(centre_source_uv)"));

        let gpu = include_str!("forest_gpu.wgsl");
        assert!(gpu.contains("if !forest_surface_owns_trees("));
        assert!(!gpu.contains("fn gpu_forest_biome_owns_trees("));
    }

    #[test]
    fn specular_is_reserved_for_ice_and_water_materials() {
        let shader = include_str!("planet.wgsl");
        assert!(shader.contains("fn material_allows_specular(biome_id: u32) -> bool"));
        let helper = shader
            .split("fn material_allows_specular(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("specular material gate is present");
        assert!(helper.contains("biome_id == 0u"));
        assert!(helper.contains("biome_id == 1u"));
        assert!(helper.contains("biome_id == 2u"));
        let flat = shader
            .split("fn flat_triangle_colour(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("flat terrain path is present");
        assert!(flat.contains("let triangle_specular = select("));
        assert!(flat.contains("material_allows_specular(fill_biome)"));
        let terrain = shader
            .split("fn terrain_fragment_color(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("smooth terrain path is present");
        assert!(terrain.contains("if material_allows_specular(biome_id)"));

        let shared = include_str!("shared_planet.wgsl");
        let ocean = shared
            .split("fn ocean_lighting(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("shared water lighting path is present");
        assert!(ocean.contains("let specular = pow("));
        assert!(ocean.contains("OCEAN_SUN_GLINT_SCALE"));
    }

    #[test]
    fn procedural_patch_cells_have_canonical_half_open_tree_ownership() {
        let key = TileKey {
            face: planet_coretypes::CubeFace::PositiveX,
            level: FOREST_CELL_LEVEL,
            x: 1_234,
            y: 2_345,
        };
        let first = forest_patch_tree_layouts(key);
        let second = forest_patch_tree_layouts(key);
        assert_eq!(first.len(), TREE_COUNT);
        assert_eq!(
            first
                .iter()
                .map(|(direction, _)| direction.to_array())
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|(direction, _)| direction.to_array())
                .collect::<Vec<_>>(),
        );
        assert!(
            first
                .iter()
                .all(|(direction, _)| forest_cell_key(*direction) == key)
        );

        let neighbouring_key = TileKey {
            x: key.x + 1,
            ..key
        };
        let neighbour = forest_patch_tree_layouts(neighbouring_key);
        assert_ne!(first[0].0.to_array(), neighbour[0].0.to_array());
    }

    #[test]
    fn projected_tree_lod_uses_stable_hash_thinning_and_rejects_far_trees() {
        let tree = TreeInstance {
            centre_and_height: [0.0, 0.0, 0.0, 20.0],
            width_shade_kind_seed: [8.0, 1.0, 0.0, 0.10],
        };
        let projected_pixels = |distance_meters| {
            projected_tree_height_pixels(
                tree,
                DVec3::new(0.0, 0.0, distance_meters),
                600,
                60.0_f64.to_radians(),
            )
        };
        assert_eq!(tree_lod(projected_pixels(100.0)), TreeLod::Full);
        assert_eq!(tree_lod(projected_pixels(1_000.0)), TreeLod::Medium);
        assert_eq!(tree_lod(projected_pixels(4_000.0)), TreeLod::Sparse);
        assert_eq!(tree_lod(projected_pixels(12_000.0)), TreeLod::Zero);
        let medium = lodded_tree_instance(tree, 5.0).expect("medium tree remains represented");
        assert!(medium.centre_and_height[3] < tree.centre_and_height[3]);
        assert!(medium.centre_and_height[3] > 0.0);
        let sparse = lodded_tree_instance(tree, 2.0).expect("sparse tree remains represented");
        assert!(sparse.centre_and_height[3] < medium.centre_and_height[3]);
        let placeholder =
            lodded_tree_instance(tree, 0.5).expect("far tree keeps a tiny placeholder");
        assert_eq!(
            placeholder.centre_and_height[3],
            tree.centre_and_height[3] * TREE_LOD_PLACEHOLDER_SCALE
        );

        let rejected = TreeInstance {
            width_shade_kind_seed: [8.0, 1.0, 0.0, 0.75],
            ..tree
        };
        assert_eq!(lodded_tree_instance(rejected, 5.0), None);
        assert_eq!(lodded_tree_instance(rejected, 2.0), None);
        assert_eq!(lodded_tree_instance(rejected, 0.5), None);
    }

    #[test]
    fn far_tree_density_enters_continuously_instead_of_revealing_a_quarter_patch() {
        assert_eq!(
            tree_lod_density(TREE_LOD_SPARSE_PIXELS - 0.01),
            TREE_LOD_PLACEHOLDER_DENSITY
        );
        assert_eq!(
            tree_lod_density(TREE_LOD_SPARSE_PIXELS),
            TREE_LOD_PLACEHOLDER_DENSITY
        );
        let just_visible = tree_lod_density(TREE_LOD_SPARSE_PIXELS + 0.01);
        assert!(just_visible > TREE_LOD_PLACEHOLDER_DENSITY);
        assert!(tree_lod_density(2.0) < tree_lod_density(2.5));
        assert_eq!(tree_lod_density(TREE_LOD_MEDIUM_PIXELS), 0.5);
    }

    #[test]
    fn patch_candidate_work_is_bounded_per_frame() {
        let mut processed = 0;
        let mut frames = 0;
        while processed < TREE_COUNT {
            let next = pending_batch_end(processed, TREE_COUNT, FOREST_PATCH_CANDIDATES_PER_FRAME);
            assert!(next - processed <= FOREST_PATCH_CANDIDATES_PER_FRAME);
            processed = next;
            frames += 1;
        }
        assert_eq!(
            frames,
            TREE_COUNT.div_ceil(FOREST_PATCH_CANDIDATES_PER_FRAME)
        );
        assert_eq!(
            pending_batch_end(0, TREE_COUNT, FOREST_PRIMARY_PATCH_CANDIDATES_PER_FRAME),
            FOREST_PRIMARY_PATCH_CANDIDATES_PER_FRAME
        );
        assert_eq!(
            pending_batch_end(0, TREE_COUNT, FOREST_INITIAL_PATCH_CANDIDATES_PER_FRAME),
            FOREST_INITIAL_PATCH_CANDIDATES_PER_FRAME
        );
    }

    #[test]
    fn patch_transition_adds_each_cell_gradually_with_stable_tree_identities() {
        let trees = (0..128)
            .map(|index| TreeInstance {
                centre_and_height: [0.0, 0.0, 0.0, 20.0],
                width_shade_kind_seed: [8.0, 1.0, 0.0, index as f32 / 128.0],
            })
            .collect::<Vec<_>>();
        let select = |progress| {
            trees
                .iter()
                .copied()
                .filter(|tree| lodded_tree_instance_with_scale(*tree, 20.0, progress).is_some())
                .collect::<Vec<_>>()
        };
        assert!(select(0.0).is_empty());
        let halfway = select(0.5);
        assert!(!halfway.is_empty() && halfway.len() < trees.len());
        assert_eq!(select(1.0), trees);
        assert_eq!(patch_transition_progress(0.0), 0.0);
        assert_eq!(
            patch_transition_progress(FOREST_PATCH_TRANSITION_SECONDS),
            1.0
        );
    }

    #[test]
    fn renderable_range_selects_multiple_nearest_cells_with_a_hard_bound() {
        let camera_position = FOREST_CENTRE_DIRECTION * (planet_radius_meters() + 2.0);
        let keys = forest_renderable_cell_keys(camera_position, 427, 60.0_f64.to_radians(), 0.0);
        assert!(keys.len() > 8);
        assert!(keys.len() <= FOREST_MAX_RENDERABLE_PATCHES);
        assert_eq!(keys[0], forest_cell_key(FOREST_CENTRE_DIRECTION));
        let camera_direction = camera_position.normalize();
        let angular_radius = forest_surface_angular_radius(
            camera_position.length(),
            maximum_tree_visibility_distance_meters(427, 60.0_f64.to_radians()),
            0.0,
        )
        .expect("ground camera can resolve trees");
        assert!(keys.iter().all(|key| {
            camera_direction
                .angle_between(forest_cell_centre_direction(*key))
                .abs()
                <= angular_radius + forest_cell_angular_radius(*key) + 1.0e-12
        }));
    }

    #[test]
    fn forest_renderable_view_culls_behind_camera_cells_conservatively() {
        let direction = FOREST_CENTRE_DIRECTION;
        let camera_position = direction * (planet_radius_meters() + 600.0);
        let reference = if direction.y.abs() < 0.9 {
            DVec3::Y
        } else {
            DVec3::X
        };
        let tangent = direction.cross(reference).normalize();
        let forward = (tangent * 0.866 - direction * 0.5).normalize();
        let right = forward.cross(direction).normalize();
        let up = right.cross(forward).normalize();
        let viewport = [1280, 720];
        let fov = 60.0_f64.to_radians();
        let all = forest_renderable_cell_keys(camera_position, viewport[1], fov, 0.0);
        let visible = forest_renderable_cell_keys_in_view(
            camera_position,
            forward,
            right,
            up,
            viewport,
            fov,
            0.0,
        );
        let all_set = all.iter().copied().collect::<BTreeSet<_>>();
        assert!(!visible.is_empty());
        assert!(visible.len() < all.len());
        assert!(visible.iter().all(|key| all_set.contains(key)));
        assert!(visible.contains(&forest_cell_key(direction)));

        let behind_direction =
            (direction - tangent * (6_000.0 / planet_radius_meters())).normalize();
        let behind_key = forest_cell_key(behind_direction);
        assert!(!forest_cell_intersects_camera_frustum(
            behind_key,
            camera_position,
            forward,
            right,
            up,
            (fov * 0.5).tan() * f64::from(viewport[0]) / f64::from(viewport[1]),
            (fov * 0.5).tan(),
            0.0,
        ));
    }

    #[test]
    fn prefetch_range_contains_the_entire_draw_range_before_approach() {
        let camera_position = FOREST_CENTRE_DIRECTION * (planet_radius_meters() + 2.0);
        let renderable =
            forest_renderable_cell_keys(camera_position, 427, 60.0_f64.to_radians(), 0.0);
        let prefetched = forest_cell_keys_within_distance(
            camera_position,
            FOREST_PREFETCH_DISTANCE_METERS,
            FOREST_MAX_CACHED_PATCHES,
            0.0,
        );
        let prefetched = prefetched.into_iter().collect::<BTreeSet<_>>();
        assert!(!renderable.is_empty());
        assert!(prefetched.len() > renderable.len());
        assert!(renderable.iter().all(|key| prefetched.contains(key)));
    }

    #[test]
    fn renderable_cell_selection_crosses_cube_face_seams() {
        let direction = DVec3::new(1.0, 0.0, 1.0).normalize();
        let keys = forest_renderable_cell_keys(
            direction * (planet_radius_meters() + 2.0),
            427,
            60.0_f64.to_radians(),
            0.0,
        );
        let faces = keys.iter().map(|key| key.face).collect::<BTreeSet<_>>();
        assert!(
            faces.len() >= 2,
            "nearby forest cells must cross cube seams"
        );
    }

    #[test]
    fn tree_render_range_is_bounded_and_empty_above_it() {
        assert_eq!(
            maximum_tree_visibility_distance_meters(10_000, 10.0_f64.to_radians()),
            FOREST_TREE_RENDER_DISTANCE_METERS
        );
        let camera_position = FOREST_CENTRE_DIRECTION
            * (planet_radius_meters() + FOREST_TREE_RENDER_DISTANCE_METERS * 2.0);
        assert!(
            forest_renderable_cell_keys(camera_position, 427, 60.0_f64.to_radians(), 0.0)
                .is_empty()
        );
    }

    #[test]
    fn forest_search_follows_high_presented_terrain_instead_of_sea_level() {
        let local_surface_height_meters = 42_000.0;
        let camera_position =
            FOREST_CENTRE_DIRECTION * (planet_radius_meters() + local_surface_height_meters + 2.0);
        let visibility_distance =
            maximum_tree_visibility_distance_meters(427, 60.0_f64.to_radians());
        assert!(
            forest_surface_angular_radius(camera_position.length(), visibility_distance, 0.0)
                .is_none(),
            "the old sea-level shell cannot reach a camera on 42km terrain"
        );
        assert!(
            forest_surface_angular_radius(
                camera_position.length(),
                visibility_distance,
                local_surface_height_meters,
            )
            .is_some()
        );
        assert!(
            !forest_renderable_cell_keys(
                camera_position,
                427,
                60.0_f64.to_radians(),
                local_surface_height_meters,
            )
            .is_empty()
        );
    }

    #[test]
    fn draw_instance_budget_is_independent_of_renderable_patch_count() {
        assert_eq!(FOREST_STARTUP_PATCH_COUNT, 3);
        const {
            assert!(FOREST_MAX_CACHED_PATCHES > FOREST_MAX_RENDERABLE_PATCHES);
        }
        const {
            assert!(FOREST_MAX_DRAW_INSTANCES > TREE_COUNT);
        }
        const {
            assert!(FOREST_MAX_DRAW_INSTANCES < TREE_COUNT * FOREST_MAX_RENDERABLE_PATCHES);
        }
    }

    #[test]
    fn tree_distance_helper_matches_projected_height_path() {
        let tree = TreeInstance {
            centre_and_height: [0.0, 0.0, 0.0, 20.0],
            width_shade_kind_seed: [8.0, 1.0, 0.0, 0.5],
        };
        let camera = DVec3::new(0.0, 0.0, 2_000.0);
        assert_eq!(
            projected_tree_height_pixels(tree, camera, 600, 60.0_f64.to_radians()),
            projected_tree_height_pixels_at_distance(
                tree,
                tree_distance_meters(tree, camera),
                600,
                60.0_f64.to_radians(),
            )
        );
    }

    #[test]
    fn forest_shader_is_a_depth_writing_three_dimensional_mesh() {
        let shader = forest_shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&shader).expect("forest shader parses");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("forest shader validates");
        assert!(shader.contains("centre_and_height: vec4<f32>"));
        assert!(shader.contains("fn tree_mesh_vertex("));
        assert!(shader.contains("let foliage_index = vertex_index - 60u;"));
        assert!(shader.contains("if vertex_index < 60u"));
        assert!(shader.contains("let cluster_vertex_index = foliage_index % 38u;"));
        assert!(shader.contains("let ring_index = (cluster_vertex_index - 1u) / 12u;"));
        assert!(shader.contains("let tier_radius = width * 0.56"));
        assert!(shader.contains("fn tree_foliage_noise("));
        assert!(shader.contains("input.local_position * 4.5"));
        assert!(!shader.contains("discard;"));
        assert!(!shader.contains("fn circle("));
        assert!(!shader.contains("let to_camera = forest.camera_planet_position.xyz - centre;"));
        assert!(shader.contains("var sky_view_lut: texture_2d<f32>;"));
        assert!(shader.contains("tree_storm_fog("));
        assert!(shader.contains("@interpolate(flat, first) fog"));
        assert!(!shader.contains("if vertex_index % 3u == 0u"));
        assert!(shader.contains("mix(colour, input.fog.rgb, input.fog.w)"));
        assert!(include_str!("forest.rs").contains("draw_indexed(0..TREE_MESH_INDEX_COUNT"));
    }

    #[test]
    fn indexed_tree_mesh_references_its_compact_shared_vertex_set() {
        let indices = tree_mesh_indices();
        assert_eq!(indices.len(), TREE_MESH_INDEX_COUNT as usize);
        let referenced = indices.into_iter().collect::<BTreeSet<_>>();
        // The full mesh draws every second crown sector (hexagonal crowns), so
        // the odd-sector vertices are left unreferenced and never shaded.
        assert!(referenced.len() < TREE_MESH_VERTEX_COUNT as usize);
        assert!(
            referenced
                .iter()
                .all(|index| u32::from(*index) < TREE_MESH_VERTEX_COUNT)
        );
        assert!(referenced.contains(&0));
    }

    #[test]
    fn tree_lod_meshes_are_valid_and_reduce_triangle_work() {
        let full = tree_mesh_indices();
        let medium = tree_mesh_medium_indices();
        let sparse = tree_mesh_sparse_indices();
        assert_eq!(medium.len(), TREE_MESH_MEDIUM_INDEX_COUNT as usize);
        assert_eq!(sparse.len(), TREE_MESH_SPARSE_INDEX_COUNT as usize);
        for indices in [&medium, &sparse] {
            assert!(
                indices
                    .iter()
                    .all(|index| u32::from(*index) < TREE_MESH_VERTEX_COUNT)
            );
            assert!(indices.len() < full.len());
        }
        assert!(medium.len() * 4 < full.len() * 3);
        assert!(sparse.len() < medium.len());
    }

    #[test]
    fn immediate_gpu_forest_shader_validates_and_uses_canonical_cells() {
        let shader = gpu_forest_shader_source();
        assert!(shader.contains("road_clears_tree(direction, surface_height)"));
        let module = wgpu::naga::front::wgsl::parse_str(&shader)
            .expect("immediate GPU forest shader parses");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("immediate GPU forest shader validates");
        assert!(shader.contains("var<storage, read> gpu_forest_cells"));
        assert!(shader.contains("forest_gpu_compute_full"));
        assert!(shader.contains("forest_gpu_compute_medium"));
        assert!(shader.contains("forest_gpu_compute_sparse"));
        assert!(shader.contains("candidate_index = candidate_in_cell * candidate_stride"));
    }

    #[test]
    fn forest_shader_has_no_unconditional_night_light() {
        let shader = include_str!("forest.wgsl");
        assert!(
            shader.contains(
                "let sky_ambient = smoothstep(-0.18, 0.02, solar_elevation_cosine) * 0.36;"
            )
        );
        assert!(shader.contains("* 1.24 * input.cloud_visibility;"));
        assert!(
            shader.contains("let lighting = (direct + sky_ambient + back_scatter) * input.shade;")
        );
        assert!(!shader.contains("0.36 + sun_amount"));
    }

    #[test]
    fn forest_direct_light_receives_the_shared_cloud_shadow() {
        let shader = forest_shader_source();
        let renderer = include_str!("forest.rs");
        assert!(shader.contains("var cloud_field_current: texture_cube<f32>;"));
        assert!(shader.contains("surface_position + sun_direction * distance"));
        assert!(
            shader.contains("cloudDensityWithOctaves(normalize(shadow_position), shell_index, 3u)")
        );
        assert!(shader.contains("floor(combined_density * 4.0 + 0.5) / 4.0"));
        assert!(shader.contains("cloud_shadow_visibility("));
        assert!(shader.contains("* 1.24 * input.cloud_visibility;"));
        assert!(renderer.contains("weather_field_bind_group_layout"));
        assert!(renderer.contains("weather_field_bind_group"));
    }

    #[test]
    fn authored_forest_centre_is_normalized() {
        assert!((FOREST_CENTRE_DIRECTION.length() - 1.0).abs() < 1.0e-9);
        const {
            assert!(FOREST_CENTRE_DIRECTION.y > 0.6);
        }
    }

    #[test]
    fn forest_has_a_render_only_performance_switch() {
        let source = include_str!("forest.rs");
        assert!(source.contains("PLANET_FOREST"));
        assert!(source.contains("if !self.enabled"));
    }
}

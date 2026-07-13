use std::{
    collections::{BTreeMap, HashMap, HashSet},
    error::Error,
    fmt,
    path::PathBuf,
};

use catinthegarden_coretypes::{
    CubeFace, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey,
};
use glam::DVec3;
use wgpu::util::DeviceExt;

use crate::{
    outmap::{Outmap, OutmapError, TileData},
    planet::{
        CHUNK_GRID_QUADS, CameraViewBasis, ChunkVertex, DEFAULT_MAX_ACTIVE_CHUNKS, MAX_LOD_LEVEL,
        PlanetLod, QuadtreeNode, build_chunk_mesh, cube_face_direction,
    },
};

// Material tiles are 131x131 stored samples, independent of the 33x33 mesh.
// Retain enough nearby L4 tiles to avoid camera-motion uploads while keeping
// the three per-tile GPU textures and CPU height cache bounded.
const MAX_RESIDENT_TERRAIN_TILES: usize = 384;
const LOD_TRANSITION_DURATION_SECONDS: f64 = 0.5;

#[derive(Clone, Debug)]
pub enum TerrainSource {
    Placeholder,
    Outmap(PathBuf),
}

#[derive(Clone, Debug, Default)]
pub struct TerrainStats {
    pub level_histogram: [u32; MAX_LOD_LEVEL as usize + 1],
    pub resident_chunks: u32,
    pub chunks_loaded: u32,
    pub chunks_unloaded: u32,
    pub splits: u32,
    pub merges: u32,
    pub culled_nodes: u32,
    pub max_level: u8,
    pub max_seam_delta_meters: f64,
    pub budget_limited: bool,
    pub resident_tiles: u32,
    pub tiles_loaded: u32,
    pub tiles_unloaded: u32,
    pub fallback_chunks: u32,
    pub lod_thrash_events: u32,
    pub draw_calls: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainInstance {
    anchor_view_position: [f32; 3],
    source_uv_scale: [f32; 2],
    source_uv_offset: [f32; 2],
    terrain_info: u32,
    lod_transition: [f32; 2],
}

impl TerrainInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        4 => Float32x3,
        5 => Float32x2,
        6 => Float32x2,
        7 => Uint32,
        8 => Float32x2
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

struct GpuChunk {
    vertex_buffer: wgpu::Buffer,
    anchor_world: DVec3,
}

struct FadingChunk {
    chunk: GpuChunk,
    started_at_sim_time: f64,
}

struct GpuTile {
    _height_texture: wgpu::Texture,
    _biome_texture: wgpu::Texture,
    _moisture_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    heights_meters: Vec<f32>,
}

#[derive(Clone, Copy)]
struct DrawItem {
    node: QuadtreeNode,
    instance_index: usize,
    tile_key: Option<TileKey>,
    fading_out: bool,
}

#[derive(Clone, Copy)]
struct RenderNode {
    node: QuadtreeNode,
    active: bool,
    fading_out: bool,
    transition_progress: f32,
    transition_incoming: bool,
}

enum TerrainDataSource {
    Placeholder,
    Outmap(Outmap),
}

pub struct TerrainRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    terrain_bind_group_layout: wgpu::BindGroupLayout,
    _environment_cubemap: wgpu::Texture,
    environment_view: wgpu::TextureView,
    environment_sampler: wgpu::Sampler,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    lod: PlanetLod,
    source: TerrainDataSource,
    placeholder_tile: GpuTile,
    tile_cache: HashMap<TileKey, GpuTile>,
    tile_last_used: HashMap<TileKey, u64>,
    tile_cache_tick: u64,
    chunks: BTreeMap<QuadtreeNode, GpuChunk>,
    fading_out_chunks: BTreeMap<QuadtreeNode, FadingChunk>,
    fade_in_started_at: HashMap<QuadtreeNode, f64>,
    draw_items: Vec<DrawItem>,
    max_outmap_seam_delta_meters: f64,
}

impl TerrainRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        source: TerrainSource,
    ) -> Result<Self, TerrainError> {
        let source = match source {
            TerrainSource::Placeholder => TerrainDataSource::Placeholder,
            TerrainSource::Outmap(root) => TerrainDataSource::Outmap(Outmap::open(root)?),
        };
        let terrain_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terrain tile bind group layout"),
                entries: &[
                    texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                    texture_layout_entry(1, wgpu::TextureSampleType::Uint),
                    texture_layout_entry(2, wgpu::TextureSampleType::Float { filterable: false }),
                    cube_texture_layout_entry(3),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&terrain_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("planet.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("LOD terrain pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[ChunkVertex::layout(), TerrainInstance::layout()],
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
                cull_mode: Some(wgpu::Face::Back),
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
            cache: None,
        });

        let topology = build_chunk_mesh(QuadtreeNode::root(0));
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared terrain chunk indices"),
            contents: bytemuck::cast_slice(&topology.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_capacity = DEFAULT_MAX_ACTIVE_CHUNKS;
        let instance_buffer = create_instance_buffer(device, instance_capacity);
        let (environment_cubemap, environment_view, environment_sampler) =
            create_environment_cubemap(device, queue);
        let placeholder_tile = create_gpu_tile(
            device,
            queue,
            &terrain_bind_group_layout,
            "placeholder terrain tile",
            &vec![0.0; tile_sample_count()],
            &vec![0; tile_sample_count()],
            &vec![128; tile_sample_count()],
            &environment_view,
            &environment_sampler,
        );

        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            terrain_bind_group_layout,
            _environment_cubemap: environment_cubemap,
            environment_view,
            environment_sampler,
            index_buffer,
            index_count: topology.indices.len() as u32,
            instance_buffer,
            instance_capacity,
            lod: PlanetLod::default(),
            source,
            placeholder_tile,
            tile_cache: HashMap::new(),
            tile_last_used: HashMap::new(),
            tile_cache_tick: 0,
            chunks: BTreeMap::new(),
            fading_out_chunks: BTreeMap::new(),
            fade_in_started_at: HashMap::new(),
            draw_items: Vec::new(),
            max_outmap_seam_delta_meters: 0.0,
        })
    }

    pub fn update(
        &mut self,
        camera_world: DVec3,
        camera_forward: DVec3,
        sim_time: f64,
        viewport: [u32; 2],
        vertical_fov_radians: f64,
    ) -> Result<TerrainStats, TerrainError> {
        assert!(sim_time.is_finite() && sim_time >= 0.0);
        self.tile_cache_tick = self.tile_cache_tick.wrapping_add(1);
        self.purge_expired_lod_transitions(sim_time);
        let lod_update = self.lod.update_for_view(
            camera_world,
            camera_forward,
            f64::from(viewport[0].max(1)) / f64::from(viewport[1].max(1)),
            viewport[1].max(1),
            vertical_fov_radians,
        );
        let topology_changed =
            !lod_update.loaded_nodes.is_empty() || !lod_update.unloaded_nodes.is_empty();
        let transition_loaded_nodes: HashSet<_> = lod_update
            .loaded_nodes
            .iter()
            .copied()
            .filter(|loaded| {
                lod_update
                    .unloaded_nodes
                    .iter()
                    .copied()
                    .any(|unloaded| nodes_share_lod_transition(*loaded, unloaded))
            })
            .collect();
        let transition_unloaded_nodes: HashSet<_> = lod_update
            .unloaded_nodes
            .iter()
            .copied()
            .filter(|unloaded| {
                lod_update
                    .loaded_nodes
                    .iter()
                    .copied()
                    .any(|loaded| nodes_share_lod_transition(loaded, *unloaded))
            })
            .collect();

        for node in &lod_update.unloaded_nodes {
            let chunk = self
                .chunks
                .remove(node)
                .expect("unloaded LOD leaf has a GPU chunk");
            self.fade_in_started_at.remove(node);
            if transition_unloaded_nodes.contains(node) {
                self.fading_out_chunks.insert(
                    *node,
                    FadingChunk {
                        chunk,
                        started_at_sim_time: sim_time,
                    },
                );
            }
        }
        for &node in &lod_update.loaded_nodes {
            let chunk = self
                .fading_out_chunks
                .remove(&node)
                .map(|fading| fading.chunk)
                .unwrap_or_else(|| self.create_gpu_chunk(node));
            self.chunks.insert(node, chunk);
            if transition_loaded_nodes.contains(&node) {
                self.fade_in_started_at.insert(node, sim_time);
            }
        }

        self.fade_in_started_at.retain(|node, started_at_sim_time| {
            self.chunks.contains_key(node)
                && sim_time - *started_at_sim_time < LOD_TRANSITION_DURATION_SECONDS
        });

        let mut render_nodes =
            Vec::with_capacity(self.fading_out_chunks.len() + lod_update.active_nodes.len());
        for (&node, fading) in &self.fading_out_chunks {
            render_nodes.push(RenderNode {
                node,
                active: false,
                fading_out: true,
                transition_progress: lod_transition_progress(sim_time, fading.started_at_sim_time),
                transition_incoming: false,
            });
        }
        for &node in &lod_update.active_nodes {
            let transition_progress = self
                .fade_in_started_at
                .get(&node)
                .map_or(1.0, |started_at_sim_time| {
                    lod_transition_progress(sim_time, *started_at_sim_time)
                });
            render_nodes.push(RenderNode {
                node,
                active: true,
                fading_out: false,
                transition_progress,
                transition_incoming: true,
            });
        }

        let mut resolved_tiles = Vec::with_capacity(render_nodes.len());
        let mut pending_tiles = Vec::new();
        if let TerrainDataSource::Outmap(outmap) = &self.source {
            for render_node in &render_nodes {
                let requested_key = tile_key(render_node.node)?;
                let source_key = outmap.resolve_tile(requested_key)?;
                if !self.tile_cache.contains_key(&source_key)
                    && !pending_tiles
                        .iter()
                        .any(|(pending_key, _): &(TileKey, TileData)| *pending_key == source_key)
                {
                    pending_tiles.push((source_key, outmap.load_tile(source_key)?));
                }
                resolved_tiles.push(Some((requested_key, source_key)));
            }
        } else {
            resolved_tiles.resize(render_nodes.len(), None);
        }

        let tiles_loaded = pending_tiles.len() as u32;
        for (key, tile) in pending_tiles {
            let label = format!("terrain tile {key:?}");
            let gpu_tile = create_gpu_tile(
                &self.device,
                &self.queue,
                &self.terrain_bind_group_layout,
                &label,
                &tile.heights_meters,
                &tile.biome_ids,
                &tile.moisture,
                &self.environment_view,
                &self.environment_sampler,
            );
            self.tile_cache.insert(key, gpu_tile);
        }

        for source_key in resolved_tiles
            .iter()
            .filter_map(|resolved| resolved.map(|(_, source)| source))
        {
            self.tile_last_used.insert(source_key, self.tile_cache_tick);
        }
        let before_eviction = self.tile_cache.len();
        if self.tile_cache.len() > MAX_RESIDENT_TERRAIN_TILES {
            let mut eviction_candidates: Vec<_> = self
                .tile_cache
                .keys()
                .map(|key| (self.tile_last_used.get(key).copied().unwrap_or(0), *key))
                .collect();
            eviction_candidates.sort_unstable();
            for (_, key) in eviction_candidates
                .into_iter()
                .take(self.tile_cache.len() - MAX_RESIDENT_TERRAIN_TILES)
            {
                self.tile_cache.remove(&key);
                self.tile_last_used.remove(&key);
            }
        }
        let tiles_unloaded = (before_eviction - self.tile_cache.len()) as u32;

        let mut instances = Vec::with_capacity(render_nodes.len());
        self.draw_items.clear();
        let mut fallback_chunks = 0_u32;
        let camera_view_basis = CameraViewBasis::from_forward(camera_forward);
        for (instance_index, (render_node, resolved)) in
            render_nodes.iter().zip(resolved_tiles.iter()).enumerate()
        {
            let chunk = if render_node.fading_out {
                &self
                    .fading_out_chunks
                    .get(&render_node.node)
                    .expect("fading LOD leaf has a GPU chunk")
                    .chunk
            } else {
                self.chunks
                    .get(&render_node.node)
                    .expect("active LOD leaf has a GPU chunk")
            };
            let (source_uv_scale, source_uv_offset, source_level, tile_key, outmap_mode) =
                if let Some((requested_key, source_key)) = *resolved {
                    let (scale, offset) = fallback_uv_transform(requested_key, source_key);
                    if render_node.active {
                        fallback_chunks += u32::from(requested_key != source_key);
                    }
                    (scale, offset, source_key.level, Some(source_key), true)
                } else {
                    ([1.0, 1.0], [0.0, 0.0], render_node.node.level, None, false)
                };
            instances.push(TerrainInstance {
                anchor_view_position: camera_view_basis
                    .world_to_view(chunk.anchor_world - camera_world)
                    .as_vec3()
                    .to_array(),
                source_uv_scale,
                source_uv_offset,
                terrain_info: pack_terrain_info(
                    outmap_mode,
                    render_node.node.face,
                    render_node.node.level,
                    source_level,
                ),
                lod_transition: [
                    render_node.transition_progress,
                    if render_node.transition_incoming {
                        1.0
                    } else {
                        0.0
                    },
                ],
            });
            self.draw_items.push(DrawItem {
                node: render_node.node,
                instance_index,
                tile_key,
                fading_out: render_node.fading_out,
            });
        }
        self.ensure_instance_capacity(instances.len());
        if !instances.is_empty() {
            self.queue
                .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }

        let metrics = lod_update.metrics;
        let active_resolved_tiles: Vec<_> = render_nodes
            .iter()
            .zip(resolved_tiles.iter())
            .filter_map(|(render_node, resolved)| render_node.active.then_some(*resolved))
            .collect();
        let max_seam_delta_meters = if matches!(&self.source, TerrainDataSource::Outmap(_)) {
            if topology_changed {
                self.max_outmap_seam_delta_meters = max_outmap_seam_delta(
                    &lod_update.active_nodes,
                    &active_resolved_tiles,
                    &self.tile_cache,
                );
            }
            self.max_outmap_seam_delta_meters
        } else {
            metrics.max_seam_delta_meters
        };
        Ok(TerrainStats {
            level_histogram: metrics.level_histogram,
            resident_chunks: metrics.active_chunks,
            chunks_loaded: metrics.chunks_loaded,
            chunks_unloaded: metrics.chunks_unloaded,
            splits: metrics.splits,
            merges: metrics.merges,
            culled_nodes: metrics.culled_nodes,
            max_level: metrics.max_level,
            max_seam_delta_meters,
            budget_limited: metrics.budget_limited,
            resident_tiles: self.tile_cache.len() as u32,
            tiles_loaded,
            tiles_unloaded,
            fallback_chunks,
            lod_thrash_events: metrics.lod_thrash_events,
            draw_calls: self.draw_items.len() as u32,
        })
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for draw in &self.draw_items {
            let chunk = if draw.fading_out {
                &self
                    .fading_out_chunks
                    .get(&draw.node)
                    .expect("draw item has a fading GPU chunk")
                    .chunk
            } else {
                self.chunks
                    .get(&draw.node)
                    .expect("draw item has a GPU chunk")
            };
            let tile = draw.tile_key.map_or(&self.placeholder_tile, |key| {
                self.tile_cache
                    .get(&key)
                    .expect("draw item has a resident terrain tile")
            });
            let instance_start = (draw.instance_index * size_of::<TerrainInstance>()) as u64;
            let instance_end = instance_start + size_of::<TerrainInstance>() as u64;
            render_pass.set_bind_group(1, &tile.bind_group, &[]);
            render_pass.set_vertex_buffer(0, chunk.vertex_buffer.slice(..));
            render_pass
                .set_vertex_buffer(1, self.instance_buffer.slice(instance_start..instance_end));
            render_pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }

    fn ensure_instance_capacity(&mut self, required: usize) {
        if required <= self.instance_capacity {
            return;
        }
        self.instance_capacity = required.next_power_of_two();
        self.instance_buffer = create_instance_buffer(&self.device, self.instance_capacity);
    }

    fn create_gpu_chunk(&self, node: QuadtreeNode) -> GpuChunk {
        let mesh = build_chunk_mesh(node);
        debug_assert_eq!(mesh.indices.len() as u32, self.index_count);
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("static anchor-local terrain vertices"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        GpuChunk {
            vertex_buffer,
            anchor_world: mesh.anchor_world,
        }
    }

    fn purge_expired_lod_transitions(&mut self, sim_time: f64) {
        self.fading_out_chunks.retain(|_, fading| {
            sim_time - fading.started_at_sim_time < LOD_TRANSITION_DURATION_SECONDS
        });
        self.fade_in_started_at.retain(|node, started_at_sim_time| {
            self.chunks.contains_key(node)
                && sim_time - *started_at_sim_time < LOD_TRANSITION_DURATION_SECONDS
        });
    }
}

fn lod_transition_progress(sim_time: f64, started_at_sim_time: f64) -> f32 {
    ((sim_time - started_at_sim_time) / LOD_TRANSITION_DURATION_SECONDS).clamp(0.0, 1.0) as f32
}

fn nodes_share_lod_transition(first: QuadtreeNode, second: QuadtreeNode) -> bool {
    node_is_descendant_of(first, second) || node_is_descendant_of(second, first)
}

fn node_is_descendant_of(mut node: QuadtreeNode, ancestor: QuadtreeNode) -> bool {
    while let Some(parent) = node.parent() {
        if parent == ancestor {
            return true;
        }
        node = parent;
    }
    false
}

#[derive(Debug)]
pub enum TerrainError {
    Outmap(OutmapError),
    InvalidCubeFace(u8),
}

impl fmt::Display for TerrainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outmap(error) => write!(formatter, "outmap error: {error}"),
            Self::InvalidCubeFace(face) => write!(formatter, "invalid cube face {face}"),
        }
    }
}

impl Error for TerrainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Outmap(error) => Some(error),
            Self::InvalidCubeFace(_) => None,
        }
    }
}

impl From<OutmapError> for TerrainError {
    fn from(error: OutmapError) -> Self {
        Self::Outmap(error)
    }
}

fn texture_layout_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn cube_texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::Cube,
            multisampled: false,
        },
        count: None,
    }
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("camera-relative terrain instances"),
        size: (capacity.max(1) * size_of::<TerrainInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_gpu_tile(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bind_group_layout: &wgpu::BindGroupLayout,
    label: &str,
    heights_meters: &[f32],
    biome_ids: &[u8],
    moisture: &[u8],
    environment_view: &wgpu::TextureView,
    environment_sampler: &wgpu::Sampler,
) -> GpuTile {
    debug_assert_eq!(heights_meters.len(), tile_sample_count());
    debug_assert_eq!(biome_ids.len(), tile_sample_count());
    debug_assert_eq!(moisture.len(), tile_sample_count());
    let height_texture = create_and_upload_texture(
        device,
        queue,
        &format!("{label} height"),
        wgpu::TextureFormat::R32Float,
        bytemuck::cast_slice(heights_meters),
        size_of::<f32>() as u32,
    );
    let biome_texture = create_and_upload_texture(
        device,
        queue,
        &format!("{label} biome"),
        wgpu::TextureFormat::R8Uint,
        biome_ids,
        1,
    );
    let moisture_texture = create_and_upload_texture(
        device,
        queue,
        &format!("{label} moisture"),
        wgpu::TextureFormat::R8Unorm,
        moisture,
        1,
    );
    let height_view = height_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let biome_view = biome_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let moisture_view = moisture_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&height_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&biome_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&moisture_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(environment_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(environment_sampler),
            },
        ],
    });
    GpuTile {
        _height_texture: height_texture,
        _biome_texture: biome_texture,
        _moisture_texture: moisture_texture,
        bind_group,
        heights_meters: heights_meters.to_vec(),
    }
}

fn create_environment_cubemap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    // A compact sky/ground cube is deliberately static for Phase 6: it proves
    // cubemap reflection without introducing SSR or a dynamic environment pass.
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ocean reflection cubemap"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let faces: [u8; 24] = [
        114, 158, 201, 255, // +X sky
        93, 135, 184, 255, // -X sky
        145, 181, 216, 255, // +Y zenith
        25, 41, 48, 255, // -Y ground
        104, 151, 195, 255, // +Z sky
        83, 124, 171, 255, // -Z sky
    ];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &faces,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("ocean reflection cubemap sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    (texture, view, sampler)
}

fn create_and_upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    bytes: &[u8],
    bytes_per_texel: u32,
) -> wgpu::Texture {
    let extent = wgpu::Extent3d {
        width: TILE_STORED_SIZE,
        height: TILE_STORED_SIZE,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(TILE_STORED_SIZE * bytes_per_texel),
            rows_per_image: Some(TILE_STORED_SIZE),
        },
        extent,
    );
    texture
}

fn tile_sample_count() -> usize {
    (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize
}

fn tile_key(node: QuadtreeNode) -> Result<TileKey, TerrainError> {
    let face = CubeFace::from_index(node.face).ok_or(TerrainError::InvalidCubeFace(node.face))?;
    Ok(TileKey {
        face,
        level: node.level,
        x: node.x,
        y: node.y,
    })
}

fn fallback_uv_transform(requested: TileKey, source: TileKey) -> ([f32; 2], [f32; 2]) {
    debug_assert_eq!(requested.face, source.face);
    debug_assert!(source.level <= requested.level);
    let level_delta = requested.level - source.level;
    let subdivision = 1_u32 << level_delta;
    debug_assert_eq!(requested.x / subdivision, source.x);
    debug_assert_eq!(requested.y / subdivision, source.y);
    let scale = 1.0 / subdivision as f32;
    let relative_x = requested.x - source.x * subdivision;
    let relative_y = requested.y - source.y * subdivision;
    (
        [scale, scale],
        [relative_x as f32 * scale, relative_y as f32 * scale],
    )
}

fn pack_terrain_info(outmap: bool, face: u8, requested_level: u8, source_level: u8) -> u32 {
    u32::from(outmap)
        | (u32::from(face) << 1)
        | (u32::from(requested_level) << 4)
        | (u32::from(source_level) << 9)
}

fn max_outmap_seam_delta(
    active_nodes: &[QuadtreeNode],
    resolved_tiles: &[Option<(TileKey, TileKey)>],
    tile_cache: &HashMap<TileKey, GpuTile>,
) -> f64 {
    let mut samples: HashMap<[i64; 3], f32> = HashMap::new();
    let mut maximum = 0.0_f64;
    for (&node, resolved) in active_nodes.iter().zip(resolved_tiles) {
        let Some((requested, source)) = resolved else {
            continue;
        };
        let tile = tile_cache
            .get(source)
            .expect("resolved outmap tile is resident");
        let (scale, offset) = fallback_uv_transform(*requested, *source);
        let [u_min, v_min, u_max, v_max] = node.uv_bounds();
        for step in 0..=CHUNK_GRID_QUADS {
            let fraction = step as f64 / CHUNK_GRID_QUADS as f64;
            for (u, v, local_uv) in [
                (
                    u_min + (u_max - u_min) * fraction,
                    v_min,
                    [fraction as f32, 0.0],
                ),
                (
                    u_max,
                    v_min + (v_max - v_min) * fraction,
                    [1.0, fraction as f32],
                ),
                (
                    u_max - (u_max - u_min) * fraction,
                    v_max,
                    [1.0 - fraction as f32, 1.0],
                ),
                (
                    u_min,
                    v_max - (v_max - v_min) * fraction,
                    [0.0, 1.0 - fraction as f32],
                ),
            ] {
                let source_uv = [
                    offset[0] + local_uv[0] * scale[0],
                    offset[1] + local_uv[1] * scale[1],
                ];
                let height = sample_height_cpu(&tile.heights_meters, source_uv);
                let direction = cube_face_direction(node.face, u, v);
                let key = [
                    (direction.x * 1.0e10).round() as i64,
                    (direction.y * 1.0e10).round() as i64,
                    (direction.z * 1.0e10).round() as i64,
                ];
                if let Some(previous) = samples.insert(key, height) {
                    maximum = maximum.max(f64::from((previous - height).abs()));
                }
            }
        }
    }
    maximum
}

fn sample_height_cpu(heights: &[f32], uv: [f32; 2]) -> f32 {
    let coordinate = [
        TILE_GUTTER as f32 + uv[0].clamp(0.0, 1.0) * (TILE_LOGICAL_SIZE - 1) as f32,
        TILE_GUTTER as f32 + uv[1].clamp(0.0, 1.0) * (TILE_LOGICAL_SIZE - 1) as f32,
    ];
    let lower = [
        coordinate[0].floor() as usize,
        coordinate[1].floor() as usize,
    ];
    let upper = [
        (lower[0] + 1).min(TILE_STORED_SIZE as usize - 1),
        (lower[1] + 1).min(TILE_STORED_SIZE as usize - 1),
    ];
    let amount = [
        coordinate[0] - lower[0] as f32,
        coordinate[1] - lower[1] as f32,
    ];
    let index = |x: usize, y: usize| y * TILE_STORED_SIZE as usize + x;
    let lower_height = heights[index(lower[0], lower[1])]
        + (heights[index(upper[0], lower[1])] - heights[index(lower[0], lower[1])]) * amount[0];
    let upper_height = heights[index(lower[0], upper[1])]
        + (heights[index(upper[0], upper[1])] - heights[index(lower[0], upper[1])]) * amount[0];
    lower_height + (upper_height - lower_height) * amount[1]
}

#[cfg(test)]
mod tests {
    use catinthegarden_coretypes::{
        CubeFace, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey,
    };

    use super::{
        fallback_uv_transform, lod_transition_progress, nodes_share_lod_transition,
        pack_terrain_info, sample_height_cpu,
    };
    use crate::planet::{QuadtreeNode, build_chunk_mesh};

    #[test]
    fn child_uv_maps_into_ancestor_quadrant() {
        let source = TileKey {
            face: CubeFace::PositiveX,
            level: 1,
            x: 0,
            y: 1,
        };
        let requested = TileKey {
            face: CubeFace::PositiveX,
            level: 3,
            x: 2,
            y: 7,
        };
        let (scale, offset) = fallback_uv_transform(requested, source);
        assert_eq!(scale, [0.25, 0.25]);
        assert_eq!(offset, [0.5, 0.75]);
    }

    #[test]
    fn terrain_info_packs_mode_face_and_levels() {
        let packed = pack_terrain_info(true, 5, 18, 7);
        assert_eq!(packed & 1, 1);
        assert_eq!((packed >> 1) & 0x7, 5);
        assert_eq!((packed >> 4) & 0x1f, 18);
        assert_eq!((packed >> 9) & 0x1f, 7);
    }

    #[test]
    fn cpu_seam_sampling_matches_shader_bilinear_coordinates() {
        let heights: Vec<_> = (0..TILE_STORED_SIZE)
            .flat_map(|y| (0..TILE_STORED_SIZE).map(move |x| (x + y * TILE_STORED_SIZE) as f32))
            .collect();
        let sampled_center = sample_height_cpu(&heights, [0.5, 0.5]);
        let center_coordinate = TILE_GUTTER + (TILE_LOGICAL_SIZE - 1) / 2;
        let expected_index = center_coordinate + center_coordinate * TILE_STORED_SIZE;
        assert_eq!(sampled_center, expected_index as f32);
    }

    #[test]
    fn every_chunk_uses_the_same_index_topology() {
        let first = build_chunk_mesh(QuadtreeNode::root(0));
        let second = build_chunk_mesh(QuadtreeNode {
            face: 5,
            level: 4,
            x: 7,
            y: 9,
        });
        assert_eq!(first.indices, second.indices);
    }

    #[test]
    fn parent_child_replacements_are_lod_transitions() {
        let parent = QuadtreeNode {
            face: 2,
            level: 3,
            x: 5,
            y: 2,
        };
        let child = parent.children()[3];
        let unrelated = QuadtreeNode {
            face: 2,
            level: 3,
            x: 6,
            y: 2,
        };

        assert!(nodes_share_lod_transition(parent, child));
        assert!(nodes_share_lod_transition(child, parent));
        assert!(!nodes_share_lod_transition(parent, unrelated));
    }

    #[test]
    fn lod_transition_progress_reaches_full_coverage_after_half_a_second() {
        assert_eq!(lod_transition_progress(10.0, 10.0), 0.0);
        assert!((lod_transition_progress(10.25, 10.0) - 0.5).abs() < f32::EPSILON);
        assert_eq!(lod_transition_progress(10.5, 10.0), 1.0);
        assert_eq!(lod_transition_progress(11.0, 10.0), 1.0);
    }
}

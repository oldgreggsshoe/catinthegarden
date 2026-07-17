use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
        CHUNK_GRID_QUADS, CameraViewBasis, ChunkVertex, DEFAULT_MAX_ACTIVE_CHUNKS,
        GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS, GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE, MAX_LOD_LEVEL,
        OUTMAP_TERRAIN_FAR_HEIGHT_SCALE, OUTMAP_TERRAIN_HEIGHT_BLEND_END_METERS,
        OUTMAP_TERRAIN_HEIGHT_BLEND_START_METERS, OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE,
        PLANET_RADIUS_METERS, PlanetLod, QuadtreeNode, TerrainHeightRange, build_chunk_mesh,
        cube_face_basis, cube_face_direction, outmap_surface_height_meters,
        placeholder_height_meters,
    },
};

// Material tiles are 131x131 stored samples, independent of the 33x33 mesh.
// Retain enough nearby L4 tiles to avoid camera-motion uploads while keeping
// the three per-tile GPU textures and CPU height cache bounded.
const MAX_RESIDENT_TERRAIN_TILES: usize = 384;
const LOD_TRANSITION_DURATION_SECONDS: f64 = 0.5;
/// Cross-fades deliberately duplicate terrain draws. Retain them for small LOD
/// adjustments, but snap a large camera/zoom change to the complete active
/// topology rather than carrying hundreds of obsolete chunks for half a
/// second.
#[cfg(test)]
const MAX_ANIMATED_LOD_TOPOLOGY_CHANGES: usize = 64;
/// Mesh creation allocates and uploads a 33x33 chunk vertex buffer. Bound that
/// synchronous work so flight remains responsive while finer leaves stream in.
const MAX_CHUNK_BUILDS_PER_FRAME: usize = 8;
/// Retain recently used GPU chunks so a moving camera can reuse nearby detail,
/// while keeping the one-buffer-per-chunk implementation bounded.
const MAX_RESIDENT_GPU_CHUNKS: usize = 512;

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

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainSettings {
    outmap_height_scale: [f32; 4],
    outmap_height_blend: [f32; 4],
}

impl TerrainSettings {
    fn from_planet_constants() -> Self {
        Self {
            outmap_height_scale: [
                OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE as f32,
                OUTMAP_TERRAIN_FAR_HEIGHT_SCALE as f32,
                GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE as f32,
                0.0,
            ],
            outmap_height_blend: [
                OUTMAP_TERRAIN_HEIGHT_BLEND_START_METERS as f32,
                OUTMAP_TERRAIN_HEIGHT_BLEND_END_METERS as f32,
                0.0,
                0.0,
            ],
        }
    }
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
    _terrain_settings_buffer: wgpu::Buffer,
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
    chunk_last_used: HashMap<QuadtreeNode, u64>,
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
        let outmap_height_bounds = match &source {
            TerrainDataSource::Placeholder => None,
            TerrainDataSource::Outmap(outmap) => Some((
                f64::from(outmap.manifest().height_min_meters),
                f64::from(outmap.manifest().height_max_meters),
            )),
        };
        let terrain_height_range = match outmap_height_bounds {
            Some((height_min_meters, height_max_meters)) => TerrainHeightRange::new(
                height_min_meters - GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS,
                height_max_meters * OUTMAP_TERRAIN_FAR_HEIGHT_SCALE
                    + GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS * GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE,
            ),
            None => TerrainHeightRange::default(),
        };
        let terrain_settings_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain settings"),
                contents: bytemuck::bytes_of(&TerrainSettings::from_planet_constants()),
                usage: wgpu::BufferUsages::UNIFORM,
            });
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
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
            &terrain_settings_buffer,
        );

        let mut lod = PlanetLod::default();
        lod.set_terrain_height_range(terrain_height_range);
        let mut renderer = Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            terrain_bind_group_layout,
            _terrain_settings_buffer: terrain_settings_buffer,
            _environment_cubemap: environment_cubemap,
            environment_view,
            environment_sampler,
            index_buffer,
            index_count: topology.indices.len() as u32,
            instance_buffer,
            instance_capacity,
            lod,
            source,
            placeholder_tile,
            tile_cache: HashMap::new(),
            tile_last_used: HashMap::new(),
            tile_cache_tick: 0,
            chunks: BTreeMap::new(),
            chunk_last_used: HashMap::new(),
            fading_out_chunks: BTreeMap::new(),
            fade_in_started_at: HashMap::new(),
            draw_items: Vec::new(),
            max_outmap_seam_delta_meters: 0.0,
        };
        // Roots are always available as geometry fallbacks. Fine chunks can
        // therefore be built over several frames without leaving holes when
        // the camera enters a previously unseen part of the planet.
        for face in CubeFace::ALL {
            let node = QuadtreeNode::root(face.index());
            renderer
                .chunks
                .insert(node, renderer.create_gpu_chunk(node));
            renderer.chunk_last_used.insert(node, 0);
        }
        Ok(renderer)
    }

    /// Returns the streamed terrain height under a planet-local radial
    /// direction. Outmap sampling deliberately uses only resident CPU tile
    /// data, so following terrain never adds disk I/O or GPU uploads to a
    /// flight frame.
    pub fn surface_height_meters_at(
        &self,
        local_surface_direction: DVec3,
        camera_altitude_meters: f64,
    ) -> Option<f64> {
        match &self.source {
            TerrainDataSource::Placeholder => {
                Some(placeholder_height_meters(local_surface_direction))
            }
            TerrainDataSource::Outmap(_) => {
                let (face, face_uv) = cube_face_uv(local_surface_direction)?;
                self.tile_cache
                    .iter()
                    .filter_map(|(key, tile)| {
                        source_tile_uv(*key, face, face_uv)
                            .map(|uv| (key.level, sample_height_cpu(&tile.heights_meters, uv)))
                    })
                    .max_by_key(|(level, _)| *level)
                    .map(|(_, height)| {
                        outmap_surface_height_meters(
                            f64::from(height),
                            local_surface_direction,
                            camera_altitude_meters,
                        )
                    })
            }
        }
    }

    pub fn update(
        &mut self,
        camera_world: DVec3,
        camera_forward: DVec3,
        camera_up: DVec3,
        sim_time: f64,
        viewport: [u32; 2],
        vertical_fov_radians: f64,
    ) -> Result<TerrainStats, TerrainError> {
        assert!(sim_time.is_finite() && sim_time >= 0.0);
        self.tile_cache_tick = self.tile_cache_tick.wrapping_add(1);
        self.purge_expired_lod_transitions(sim_time);
        let lod_update = self.lod.update_for_view_with_up(
            camera_world,
            camera_forward,
            camera_up,
            f64::from(viewport[0].max(1)) / f64::from(viewport[1].max(1)),
            viewport[1].max(1),
            vertical_fov_radians,
        );
        let topology_changed =
            !lod_update.loaded_nodes.is_empty() || !lod_update.unloaded_nodes.is_empty();
        let mut resident_nodes: BTreeSet<_> = self.chunks.keys().copied().collect();
        // Selecting a leaf is cheap, but constructing its vertex buffer is
        // not. Build only the highest-priority next descendants this frame;
        // an already resident ancestor covers each remaining request.
        let missing_nodes = prioritized_missing_chunks(
            &lod_update.active_nodes,
            &resident_nodes,
            camera_world,
            MAX_CHUNK_BUILDS_PER_FRAME,
        );
        for node in &missing_nodes {
            let chunk = self.create_gpu_chunk(*node);
            self.chunks.insert(*node, chunk);
            self.chunk_last_used.insert(*node, self.tile_cache_tick);
            resident_nodes.insert(*node);
        }
        let mut active_render_nodes = BTreeSet::new();
        for &node in &lod_update.active_nodes {
            active_render_nodes.insert(
                resident_ancestor(node, &resident_nodes)
                    .expect("root terrain chunk covers every active node"),
            );
        }
        for &node in &active_render_nodes {
            self.chunk_last_used.insert(node, self.tile_cache_tick);
        }
        let chunks_unloaded = self.evict_unused_chunks(&active_render_nodes);
        let active_render_nodes: Vec<_> = active_render_nodes.into_iter().collect();

        self.fade_in_started_at.retain(|node, started_at_sim_time| {
            self.chunks.contains_key(node)
                && sim_time - *started_at_sim_time < LOD_TRANSITION_DURATION_SECONDS
        });

        let mut render_nodes =
            Vec::with_capacity(self.fading_out_chunks.len() + active_render_nodes.len());
        for (&node, fading) in &self.fading_out_chunks {
            render_nodes.push(RenderNode {
                node,
                active: false,
                fading_out: true,
                transition_progress: lod_transition_progress(sim_time, fading.started_at_sim_time),
                transition_incoming: false,
            });
        }
        for &node in &active_render_nodes {
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
                &self._terrain_settings_buffer,
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
        let camera_view_basis = CameraViewBasis::from_forward_and_up(camera_forward, camera_up);
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
            if topology_changed || !missing_nodes.is_empty() {
                self.max_outmap_seam_delta_meters = max_outmap_seam_delta(
                    &active_render_nodes,
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
            chunks_loaded: missing_nodes.len() as u32,
            chunks_unloaded: chunks_unloaded as u32,
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

    fn evict_unused_chunks(&mut self, protected_nodes: &BTreeSet<QuadtreeNode>) -> usize {
        if self.chunks.len() <= MAX_RESIDENT_GPU_CHUNKS {
            return 0;
        }
        let mut candidates: Vec<_> = self
            .chunks
            .keys()
            .copied()
            .filter(|node| node.level > 0 && !protected_nodes.contains(node))
            .map(|node| (self.chunk_last_used.get(&node).copied().unwrap_or(0), node))
            .collect();
        candidates.sort_unstable();
        let eviction_count = (self.chunks.len() - MAX_RESIDENT_GPU_CHUNKS).min(candidates.len());
        for (_, node) in candidates.into_iter().take(eviction_count) {
            self.chunks.remove(&node);
            self.chunk_last_used.remove(&node);
            self.fade_in_started_at.remove(&node);
        }
        eviction_count
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

fn prioritized_missing_chunks(
    active_nodes: &[QuadtreeNode],
    resident_nodes: &BTreeSet<QuadtreeNode>,
    camera_world: DVec3,
    maximum_builds: usize,
) -> Vec<QuadtreeNode> {
    let mut missing: Vec<_> = active_nodes
        .iter()
        .copied()
        .filter_map(|node| next_missing_descendant(node, resident_nodes))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    missing.sort_unstable_by(|left, right| {
        let left_distance = (left.center_direction() * PLANET_RADIUS_METERS).distance(camera_world);
        let right_distance =
            (right.center_direction() * PLANET_RADIUS_METERS).distance(camera_world);
        left_distance
            .total_cmp(&right_distance)
            .then_with(|| right.level.cmp(&left.level))
            .then_with(|| left.cmp(right))
    });
    missing.truncate(maximum_builds);
    missing
}

fn resident_ancestor(
    mut node: QuadtreeNode,
    resident_nodes: &BTreeSet<QuadtreeNode>,
) -> Option<QuadtreeNode> {
    loop {
        if resident_nodes.contains(&node) {
            return Some(node);
        }
        node = node.parent()?;
    }
}

fn next_missing_descendant(
    target: QuadtreeNode,
    resident_nodes: &BTreeSet<QuadtreeNode>,
) -> Option<QuadtreeNode> {
    let ancestor = resident_ancestor(target, resident_nodes)?;
    if ancestor == target {
        return None;
    }
    let level = ancestor.level + 1;
    let shift = target.level - level;
    Some(QuadtreeNode {
        face: target.face,
        level,
        x: target.x >> shift,
        y: target.y >> shift,
    })
}

fn lod_transition_progress(sim_time: f64, started_at_sim_time: f64) -> f32 {
    ((sim_time - started_at_sim_time) / LOD_TRANSITION_DURATION_SECONDS).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
fn should_animate_lod_transition(
    fading_nodes: usize,
    loaded_nodes: usize,
    unloaded_nodes: usize,
) -> bool {
    loaded_nodes.saturating_add(unloaded_nodes) <= MAX_ANIMATED_LOD_TOPOLOGY_CHANGES
        && fading_nodes.saturating_add(unloaded_nodes) <= MAX_ANIMATED_LOD_TOPOLOGY_CHANGES
}

#[cfg(test)]
fn nodes_share_lod_transition(first: QuadtreeNode, second: QuadtreeNode) -> bool {
    node_is_descendant_of(first, second) || node_is_descendant_of(second, first)
}

#[cfg(test)]
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
    terrain_settings_buffer: &wgpu::Buffer,
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
            wgpu::BindGroupEntry {
                binding: 5,
                resource: terrain_settings_buffer.as_entire_binding(),
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

#[cfg(test)]
fn source_tile_uv_at_direction(key: TileKey, direction: DVec3) -> Option<[f32; 2]> {
    let (face, face_uv) = cube_face_uv(direction)?;
    source_tile_uv(key, face, face_uv)
}

fn source_tile_uv(key: TileKey, face: CubeFace, face_uv: [f64; 2]) -> Option<[f32; 2]> {
    (key.face == face).then_some(())?;

    let tiles_per_side = 1_u32 << key.level;
    let coordinates = face_uv.map(|coordinate| {
        ((coordinate + 1.0) * 0.5 * f64::from(tiles_per_side)).clamp(0.0, f64::from(tiles_per_side))
    });
    let local_uv = [
        coordinates[0] - f64::from(key.x),
        coordinates[1] - f64::from(key.y),
    ];
    let contains = |coordinate: f64, index: u32| {
        coordinate >= 0.0
            && (coordinate < 1.0 || (index + 1 == tiles_per_side && coordinate <= 1.0))
    };
    (contains(local_uv[0], key.x) && contains(local_uv[1], key.y))
        .then(|| [local_uv[0] as f32, local_uv[1] as f32])
}

fn cube_face_uv(direction: DVec3) -> Option<(CubeFace, [f64; 2])> {
    if !direction.is_finite() || direction.length_squared() == 0.0 {
        return None;
    }
    let direction = direction.normalize();
    let mut selected_face = CubeFace::PositiveX;
    let mut selected_normal = DVec3::X;
    let mut selected_tangent_u = DVec3::NEG_Z;
    let mut selected_tangent_v = DVec3::Y;
    let mut largest_normal_dot = f64::NEG_INFINITY;
    for face in CubeFace::ALL {
        let (normal, tangent_u, tangent_v) = cube_face_basis(face.index());
        let normal_dot = direction.dot(normal);
        if normal_dot > largest_normal_dot {
            selected_face = face;
            selected_normal = normal;
            selected_tangent_u = tangent_u;
            selected_tangent_v = tangent_v;
            largest_normal_dot = normal_dot;
        }
    }
    (largest_normal_dot > 0.0).then(|| {
        (
            selected_face,
            [
                direction.dot(selected_tangent_u) / direction.dot(selected_normal),
                direction.dot(selected_tangent_v) / direction.dot(selected_normal),
            ],
        )
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
    use std::collections::BTreeSet;

    use super::{
        TerrainSettings, cube_face_uv, fallback_uv_transform, lod_transition_progress,
        next_missing_descendant, nodes_share_lod_transition, pack_terrain_info,
        prioritized_missing_chunks, resident_ancestor, sample_height_cpu,
        should_animate_lod_transition, source_tile_uv_at_direction,
    };
    use crate::planet::{
        GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE, OUTMAP_TERRAIN_FAR_HEIGHT_SCALE,
        OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE, PLANET_RADIUS_METERS, QuadtreeNode, build_chunk_mesh,
        cube_face_direction,
    };
    use catinthegarden_coretypes::{
        CubeFace, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey,
    };

    #[test]
    fn cube_face_uv_inverts_cube_face_direction() {
        for face in CubeFace::ALL {
            let direction = cube_face_direction(face.index(), 0.37, -0.61);
            let (sampled_face, [u, v]) = cube_face_uv(direction).expect("valid cube direction");
            assert_eq!(sampled_face, face);
            assert!((u - 0.37).abs() < 1.0e-12);
            assert!((v + 0.61).abs() < 1.0e-12);
        }
    }

    #[test]
    fn direction_maps_to_its_resident_source_tile_uv() {
        let key = TileKey {
            face: CubeFace::PositiveX,
            level: 3,
            x: 5,
            y: 1,
        };
        let direction = cube_face_direction(key.face.index(), 0.375, -0.625);
        let uv = source_tile_uv_at_direction(key, direction).expect("direction is in tile");
        assert!((uv[0] - 0.5).abs() < f32::EPSILON);
        assert!((uv[1] - 0.5).abs() < f32::EPSILON);
    }

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
    fn shader_reads_outmap_height_scale_from_terrain_settings() {
        let settings = TerrainSettings::from_planet_constants();
        let shader = include_str!("planet.wgsl");
        assert_eq!(
            settings.outmap_height_scale[0],
            OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE as f32
        );
        assert_eq!(
            settings.outmap_height_scale[1],
            OUTMAP_TERRAIN_FAR_HEIGHT_SCALE as f32
        );
        assert_eq!(
            settings.outmap_height_scale[2],
            GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE as f32
        );
        assert_eq!(settings.outmap_height_blend[0], 100_000.0);
        assert_eq!(settings.outmap_height_blend[1], 1_000_000.0);
        assert!(shader.matches("terrain_macro_height_scale()").count() >= 2);
    }

    #[test]
    fn shader_filters_detail_by_continuous_camera_distance_and_real_light() {
        let shader = include_str!("planet.wgsl");
        assert_eq!(
            shader
                .matches("terrain_detail_octave_distance_weight(")
                .count(),
            5
        );
        for full_distance in [
            "camera_distance_meters, 150000.0",
            "camera_distance_meters, 20000.0",
            "camera_distance_meters, 2500.0",
            "camera_distance_meters, 300.0",
        ] {
            assert!(shader.contains(full_distance));
        }
        assert!(!shader.contains("requested_lod_level: f32"));
        assert!(shader.contains("biome_color(2u) * 0.65 * ice_light_floor"));
        assert!(!shader.contains("max(lit_surface_color, biome_color(2u) * 0.65)"));
    }

    #[test]
    fn shaders_use_the_same_altitude_aware_twilight_column() {
        for shader in [include_str!("planet.wgsl"), include_str!("atmosphere.wgsl")] {
            assert!(shader.contains(
                "fn twilight_solar_air_mass(solar_zenith_cosine: f32, sample_altitude_meters: f32)"
            ));
            assert!(shader.contains("upper_atmosphere_amount"));
            assert!(shader.contains("horizon_amount"));
        }
    }

    #[test]
    fn direct_surface_sunlight_fades_before_geometric_sunset() {
        let shader = include_str!("planet.wgsl");
        assert!(shader.contains("let solar_elevation = dot(surface_direction, sun_direction);"));
        assert!(
            shader.contains("smoothstep(\n        -0.01,\n        0.08,\n        solar_elevation,")
        );
        assert!(shader.contains("sun_transmittance * specular"));
        assert!(shader.contains("sun_transmittance * direct_light"));
    }

    #[test]
    fn direct_surface_sunlight_progresses_from_orange_to_red_before_darkness() {
        let shader = include_str!("planet.wgsl");
        assert!(shader.contains("let orange_tint = vec3<f32>(1.20, 0.55, 0.16);"));
        assert!(shader.contains("let red_tint = vec3<f32>(1.35, 0.12, 0.03);"));
        assert!(shader.contains("return transmitted_sunlight * low_sun_tint * solar_visibility;"));
        assert!(shader.contains("sun_transmittance * specular"));
        assert!(shader.contains("sun_transmittance * direct_light"));
    }

    #[test]
    fn ocean_aerial_perspective_preserves_the_dark_water_body() {
        let shader = include_str!("planet.wgsl");
        assert!(shader.contains("const OCEAN_AERIAL_PERSPECTIVE_WEIGHT: f32 = 0.35;"));
        assert_eq!(shader.matches("ocean_aerial_perspective(").count(), 4);
        assert!(shader.contains("water_surface_color,\n        aerial_color,"));
    }

    #[test]
    fn terrain_material_pass_uses_displaced_slope_and_latitude_snowline() {
        let shader = include_str!("planet.wgsl");
        assert!(shader.contains("let rock_amount = smoothstep(0.10, 0.42, slope);"));
        assert!(shader.contains("let snowline_meters = mix(6200.0, 2200.0, latitude_amount);"));
        assert!(shader.contains("normal,\n        direction,\n    ) * surface_irradiance;"));
        assert!(shader.contains("input.world_normal,\n        direction,\n    );"));
    }

    #[test]
    fn fullscreen_sky_applies_the_requested_double_saturation() {
        let shader = include_str!("atmosphere.wgsl");
        assert!(shader.contains("const SKY_ATMOSPHERE_SATURATION: f32 = 2.0;"));
        assert!(shader.contains("fn saturate_sky_color(color: vec3<f32>)"));
        assert!(shader.contains("saturate_sky_color(sky_radiance)"));
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
    fn every_cube_face_chunk_winds_outward() {
        for face in 0..6 {
            let chunk = build_chunk_mesh(QuadtreeNode::root(face));
            let [first, second, third] = [
                chunk.indices[0] as usize,
                chunk.indices[1] as usize,
                chunk.indices[2] as usize,
            ];
            let first_position = chunk.vertex_world_position(first, false);
            let second_position = chunk.vertex_world_position(second, false);
            let third_position = chunk.vertex_world_position(third, false);
            let normal = (second_position - first_position).cross(third_position - first_position);
            assert!(
                normal.dot(first_position) > 0.0,
                "cube face {face} has inward-facing terrain triangles"
            );
        }
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

    #[test]
    fn large_lod_changes_snap_instead_of_duplicating_draws() {
        assert!(should_animate_lod_transition(0, 32, 32));
        assert!(!should_animate_lod_transition(0, 33, 32));
        assert!(!should_animate_lod_transition(40, 16, 25));
        assert!(!should_animate_lod_transition(usize::MAX, 0, 1));
    }

    #[test]
    fn missing_chunks_stream_nearest_first_with_a_hard_per_frame_cap() {
        let near_root = QuadtreeNode::root(0);
        let far_root = QuadtreeNode::root(1);
        let near = near_root.children()[0].children()[3];
        let far = far_root.children()[2].children()[1];
        let camera = near.center_direction() * (PLANET_RADIUS_METERS + 1_524.0);
        let resident_nodes = BTreeSet::from([near_root, far_root]);

        assert_eq!(
            prioritized_missing_chunks(&[far, near], &resident_nodes, camera, 1),
            vec![near_root.children()[0]]
        );
    }

    #[test]
    fn resident_parent_covers_an_unbuilt_child() {
        let root = QuadtreeNode::root(3);
        let parent = root.children()[1];
        let child = parent.children()[2];
        let resident_nodes = BTreeSet::from([root, parent]);

        assert_eq!(resident_ancestor(child, &resident_nodes), Some(parent));
    }

    #[test]
    fn streaming_refines_one_resident_level_at_a_time() {
        let root = QuadtreeNode::root(2);
        let target = root.children()[3].children()[1];
        let resident_nodes = BTreeSet::from([root]);

        assert_eq!(
            next_missing_descendant(target, &resident_nodes),
            Some(root.children()[3])
        );
    }
}

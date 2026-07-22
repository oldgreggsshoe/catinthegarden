use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    error::Error,
    fmt,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
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
/// Bound main-thread texture creation even if the I/O worker completed a burst
/// while rendering was paused or slow.
const MAX_TILE_UPLOADS_PER_FRAME: usize = 4;

fn planet_shader_source() -> String {
    [
        include_str!("shared_planet.wgsl"),
        include_str!("planet.wgsl"),
    ]
    .join("\n")
}
const MAX_PENDING_TILE_LOADS: usize = 32;
/// Half a second gives a newly resident grid time to replace its parent
/// without leaving the opaque dither visible long enough to sparkle during
/// normal flight. The higher-detail request itself begins early in `LodPolicy`.
const LOD_TRANSITION_DURATION_SECONDS: f64 = 0.5;
/// Cross-fades deliberately duplicate terrain draws. Retain them for small LOD
/// adjustments, but snap a large camera/zoom change to the complete active
/// topology rather than carrying hundreds of obsolete chunks for half a
/// second.
const MAX_ANIMATED_LOD_TOPOLOGY_CHANGES: usize = 64;
/// Four compact, repeatable material layers add close-range surface variation
/// without pretending to add missing baked height data to ancestor tiles.
/// A full mip chain keeps the triplanar samples stable as the camera climbs.
const TERRAIN_MATERIAL_TEXTURE_SIZE: u32 = 256;
const TERRAIN_MATERIAL_LAYER_COUNT: u32 = 4;
/// A 129-sample outmap tile contains 128 logical quads while the shared chunk
/// grid contains 32. Two extra quadtree levels split one source tile into a
/// 4x4 set of chunks and therefore consume every available height sample.
/// Refining farther only repeats the same bilinear source data.
const OUTMAP_TILE_GRID_SUBDIVISION_LEVELS: u8 = 2;
/// Conservative unresolved-height error relative to one geometry cell. Unlike
/// the former near-flight rings, this is projected from each visible node's
/// actual camera distance. The source-level cap below prevents spending this
/// error budget on repeated samples from a coarse ancestor tile.
const OUTMAP_GEOMETRIC_ERROR_RATIO: f64 = 0.15;
/// Below this altitude the camera is close enough that geometry density matters
/// more than source texel uniqueness. Ancestor tiles may feed finer grids while
/// the worker streams better sources; otherwise low flight stalls at L6 and
/// exposes huge terrain facets.
const LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS: f64 = 250_000.0;

#[derive(Clone, Debug)]
pub enum TerrainSource {
    Placeholder,
    Outmap(PathBuf),
}

#[derive(Clone, Debug, Default)]
pub struct TerrainStats {
    pub level_histogram: [u32; MAX_LOD_LEVEL as usize + 1],
    pub resident_chunks: u32,
    pub drawn_chunks: u32,
    pub terrain_triangles: u64,
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
    pub source_level_delta_histogram: [u32; MAX_LOD_LEVEL as usize + 1],
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
    edge_stitch: u32,
    node_uv_origin_span: [f32; 4],
    node_anchor_direction_cube_length: [f32; 4],
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
    const ATTRIBUTES: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
        4 => Float32x3,
        5 => Float32x2,
        6 => Float32x2,
        7 => Uint32,
        8 => Float32x2,
        9 => Uint32,
        10 => Float32x4,
        11 => Float32x4
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

struct FadingChunk {
    started_at_presentation_time: f64,
}

struct GpuTile {
    _height_texture: wgpu::Texture,
    _biome_texture: wgpu::Texture,
    _moisture_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    heights_meters: Vec<f32>,
}

#[derive(Clone, Copy)]
struct DrawBatch {
    first_instance: u32,
    instance_count: u32,
    tile_key: Option<TileKey>,
}

#[derive(Clone, Copy)]
struct RenderNode {
    node: QuadtreeNode,
    active: bool,
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
    transition_pipeline: wgpu::RenderPipeline,
    stable_pipeline: wgpu::RenderPipeline,
    terrain_bind_group_layout: wgpu::BindGroupLayout,
    _terrain_settings_buffer: wgpu::Buffer,
    _environment_cubemap: wgpu::Texture,
    environment_view: wgpu::TextureView,
    environment_sampler: wgpu::Sampler,
    _terrain_material_texture: wgpu::Texture,
    terrain_material_view: wgpu::TextureView,
    terrain_material_sampler: wgpu::Sampler,
    chunk_vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    lod: PlanetLod,
    source: TerrainDataSource,
    placeholder_tile: GpuTile,
    tile_cache: HashMap<TileKey, GpuTile>,
    tile_last_used: HashMap<TileKey, u64>,
    tile_load_requests: Option<Sender<TileKey>>,
    tile_load_results: Option<Receiver<(TileKey, Result<TileData, OutmapError>)>>,
    pending_tile_loads: BTreeSet<TileKey>,
    tile_cache_tick: u64,
    fading_out_chunks: BTreeMap<QuadtreeNode, FadingChunk>,
    fade_in_started_at: HashMap<QuadtreeNode, f64>,
    active_render_nodes: BTreeSet<QuadtreeNode>,
    draw_batches: Vec<DrawBatch>,
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
        let (tile_load_requests, tile_load_results) = match &source {
            TerrainDataSource::Placeholder => (None, None),
            TerrainDataSource::Outmap(outmap) => {
                let loader_outmap = outmap.clone();
                let (request_sender, request_receiver) = mpsc::channel();
                let (result_sender, result_receiver) = mpsc::channel();
                let _ = thread::spawn(move || {
                    while let Ok(source_key) = request_receiver.recv() {
                        let result = loader_outmap.load_tile(source_key);
                        if result_sender.send((source_key, result)).is_err() {
                            break;
                        }
                    }
                });
                (Some(request_sender), Some(result_receiver))
            }
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
                        // Terrain displacement reads these scales in the
                        // vertex stage; fragment lake shading recomputes the
                        // same surface height for atmosphere and water light.
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    texture_array_layout_entry(
                        6,
                        wgpu::TextureSampleType::Float { filterable: true },
                    ),
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
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
        let shader_source = planet_shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("planet raster shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let create_pipeline = |label, fragment_entry_point| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[ChunkVertex::layout(), TerrainInstance::layout()],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry_point),
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
            })
        };
        let transition_pipeline = create_pipeline("LOD terrain transition pipeline", "fs_main");
        let stable_pipeline = create_pipeline("LOD terrain stable pipeline", "fs_main_stable");

        let topology = build_chunk_mesh(QuadtreeNode::root(0));
        // Every quadtree leaf has the same 33x33 topology. Node bounds now
        // arrive through the instance stream and the vertex shader projects
        // that canonical grid onto the cube sphere. This removes all
        // camera-motion-dependent mesh allocation and GPU uploads.
        let chunk_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared canonical terrain grid"),
            contents: bytemuck::cast_slice(&topology.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared terrain chunk indices"),
            contents: bytemuck::cast_slice(&topology.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_capacity = DEFAULT_MAX_ACTIVE_CHUNKS;
        let instance_buffer = create_instance_buffer(device, instance_capacity);
        let (environment_cubemap, environment_view, environment_sampler) =
            create_environment_cubemap(device, queue);
        let (terrain_material_texture, terrain_material_view, terrain_material_sampler) =
            create_terrain_material_texture(device, queue);
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
            &terrain_material_view,
            &terrain_material_sampler,
        );
        // Keep one complete coarse surface resident before the first frame.
        // Background streaming can then refine from real baked geography
        // instead of flashing the analytic placeholder while its first I/O
        // requests are still in flight.
        let mut initial_tile_cache = HashMap::new();
        if let TerrainDataSource::Outmap(outmap) = &source {
            for face in CubeFace::ALL {
                let key = TileKey::root(face);
                let tile = outmap.load_tile(key)?;
                let label = format!("terrain root tile {key:?}");
                initial_tile_cache.insert(
                    key,
                    create_gpu_tile(
                        device,
                        queue,
                        &terrain_bind_group_layout,
                        &label,
                        &tile.heights_meters,
                        &tile.biome_ids,
                        &tile.moisture,
                        &environment_view,
                        &environment_sampler,
                        &terrain_settings_buffer,
                        &terrain_material_view,
                        &terrain_material_sampler,
                    ),
                );
            }
        }
        let initial_tile_last_used = initial_tile_cache.keys().map(|key| (*key, 0)).collect();

        let mut lod = PlanetLod::default();
        lod.set_terrain_height_range(terrain_height_range);
        let renderer = Self {
            device: device.clone(),
            queue: queue.clone(),
            transition_pipeline,
            stable_pipeline,
            terrain_bind_group_layout,
            _terrain_settings_buffer: terrain_settings_buffer,
            _environment_cubemap: environment_cubemap,
            environment_view,
            environment_sampler,
            _terrain_material_texture: terrain_material_texture,
            terrain_material_view,
            terrain_material_sampler,
            chunk_vertex_buffer,
            index_buffer,
            index_count: topology.indices.len() as u32,
            instance_buffer,
            instance_capacity,
            lod,
            source,
            placeholder_tile,
            tile_cache: initial_tile_cache,
            tile_last_used: initial_tile_last_used,
            tile_load_requests,
            tile_load_results,
            pending_tile_loads: BTreeSet::new(),
            tile_cache_tick: 0,
            fading_out_chunks: BTreeMap::new(),
            fade_in_started_at: HashMap::new(),
            active_render_nodes: BTreeSet::new(),
            draw_batches: Vec::new(),
            max_outmap_seam_delta_meters: 0.0,
        };
        Ok(renderer)
    }

    /// Returns the dry coastal centre selected for sparse high-resolution
    /// refinement by the baker.
    pub fn preferred_landing_direction(&self) -> Option<DVec3> {
        match &self.source {
            TerrainDataSource::Placeholder => None,
            TerrainDataSource::Outmap(outmap) => Some(DVec3::from_array(
                outmap.manifest().sparse_landing_direction,
            )),
        }
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
        presentation_time: f64,
        viewport: [u32; 2],
        vertical_fov_radians: f64,
    ) -> Result<TerrainStats, TerrainError> {
        assert!(presentation_time.is_finite() && presentation_time >= 0.0);
        self.tile_cache_tick = self.tile_cache_tick.wrapping_add(1);
        self.purge_expired_lod_transitions(presentation_time);
        let camera_altitude_meters = camera_world.length() - PLANET_RADIUS_METERS;
        let distance_reference_height_meters = self
            .surface_height_meters_at(camera_world.normalize(), camera_altitude_meters)
            .unwrap_or(0.0);
        self.lod
            .set_distance_reference_height(distance_reference_height_meters);
        let aspect_ratio = f64::from(viewport[0].max(1)) / f64::from(viewport[1].max(1));
        let lod_update = match &self.source {
            TerrainDataSource::Placeholder => self.lod.update_for_view_with_up(
                camera_world,
                camera_forward,
                camera_up,
                aspect_ratio,
                viewport[1].max(1),
                vertical_fov_radians,
            ),
            TerrainDataSource::Outmap(outmap) => {
                if camera_altitude_meters < LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS {
                    self.lod.update_for_view_with_constraints(
                        camera_world,
                        camera_forward,
                        camera_up,
                        aspect_ratio,
                        viewport[1].max(1),
                        vertical_fov_radians,
                        OUTMAP_GEOMETRIC_ERROR_RATIO,
                        &|_| MAX_LOD_LEVEL,
                    )
                } else {
                    self.lod.update_for_view_with_constraints(
                        camera_world,
                        camera_forward,
                        camera_up,
                        aspect_ratio,
                        viewport[1].max(1),
                        vertical_fov_radians,
                        OUTMAP_GEOMETRIC_ERROR_RATIO,
                        &|node| outmap_node_level_limit(outmap, node),
                    )
                }
            }
        };
        let topology_changed =
            !lod_update.loaded_nodes.is_empty() || !lod_update.unloaded_nodes.is_empty();
        // Geometry is a shared canonical grid, so every selected leaf is
        // immediately drawable. There is no resident-parent fallback and no
        // delayed whole-region promotion from one giant triangle to a fine
        // patch.
        let active_render_nodes: BTreeSet<_> = lod_update.active_nodes.iter().copied().collect();
        self.update_lod_transitions(&active_render_nodes, presentation_time);
        let active_render_nodes: Vec<_> = active_render_nodes.into_iter().collect();

        self.fade_in_started_at
            .retain(|node, started_at_presentation_time| {
                active_render_nodes.contains(node)
                    && presentation_time - *started_at_presentation_time
                        < LOD_TRANSITION_DURATION_SECONDS
            });

        let mut render_nodes =
            Vec::with_capacity(self.fading_out_chunks.len() + active_render_nodes.len());
        for (&node, fading) in &self.fading_out_chunks {
            render_nodes.push(RenderNode {
                node,
                active: false,
                transition_progress: lod_transition_progress(
                    presentation_time,
                    fading.started_at_presentation_time,
                ),
                transition_incoming: false,
            });
        }
        for &node in &active_render_nodes {
            let transition_progress =
                self.fade_in_started_at
                    .get(&node)
                    .map_or(1.0, |started_at_presentation_time| {
                        lod_transition_progress(presentation_time, *started_at_presentation_time)
                    });
            render_nodes.push(RenderNode {
                node,
                active: true,
                transition_progress,
                transition_incoming: true,
            });
        }

        let mut completed_tiles = Vec::new();
        if let Some(results) = &self.tile_load_results {
            for _ in 0..MAX_TILE_UPLOADS_PER_FRAME {
                let Ok((source_key, result)) = results.try_recv() else {
                    break;
                };
                self.pending_tile_loads.remove(&source_key);
                completed_tiles.push((source_key, result?));
            }
        }
        let tiles_loaded = completed_tiles.len() as u32;
        for (key, tile) in completed_tiles {
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
                &self.terrain_material_view,
                &self.terrain_material_sampler,
            );
            self.tile_cache.insert(key, gpu_tile);
        }

        let mut resolved_tiles = Vec::with_capacity(render_nodes.len());
        if let TerrainDataSource::Outmap(outmap) = &self.source {
            let mut load_candidates = Vec::new();
            for render_node in &render_nodes {
                let requested_key = tile_key(render_node.node)?;
                let preferred_source_key = outmap.resolve_tile(requested_key)?;
                if !self.tile_cache.contains_key(&preferred_source_key)
                    && !self.pending_tile_loads.contains(&preferred_source_key)
                {
                    load_candidates.push((
                        (render_node.node.center_direction() * PLANET_RADIUS_METERS)
                            .distance(camera_world),
                        preferred_source_key,
                    ));
                }
                resolved_tiles.push(
                    cached_tile_ancestor(requested_key, preferred_source_key, &self.tile_cache)
                        .map(|source_key| (requested_key, source_key)),
                );
            }
            load_candidates.sort_unstable_by(|left, right| {
                left.0
                    .total_cmp(&right.0)
                    .then_with(|| left.1.cmp(&right.1))
            });
            let mut queued_this_frame = BTreeSet::new();
            for (_, source_key) in load_candidates {
                if self.pending_tile_loads.len() >= MAX_PENDING_TILE_LOADS {
                    break;
                }
                if queued_this_frame.insert(source_key)
                    && self
                        .tile_load_requests
                        .as_ref()
                        .is_some_and(|requests| requests.send(source_key).is_ok())
                {
                    self.pending_tile_loads.insert(source_key);
                }
            }
        } else {
            resolved_tiles.resize(render_nodes.len(), None);
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
                .filter(|key| key.level > 0)
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

        let active_resolved_tiles: Vec<_> = render_nodes
            .iter()
            .zip(resolved_tiles.iter())
            .filter_map(|(render_node, resolved)| render_node.active.then_some(*resolved))
            .collect();
        let mut prepared_instances = Vec::with_capacity(render_nodes.len());
        self.draw_batches.clear();
        let mut fallback_chunks = 0_u32;
        let mut source_level_delta_histogram = [0_u32; MAX_LOD_LEVEL as usize + 1];
        let camera_view_basis = CameraViewBasis::from_forward_and_up(camera_forward, camera_up);
        for (render_node, resolved) in render_nodes.iter().zip(resolved_tiles.iter()) {
            let (source_uv_scale, source_uv_offset, source_level, tile_key, outmap_mode) =
                if let Some((requested_key, source_key)) = *resolved {
                    let (scale, offset) = fallback_uv_transform(requested_key, source_key);
                    if render_node.active {
                        fallback_chunks += u32::from(requested_key != source_key);
                        source_level_delta_histogram
                            [(requested_key.level - source_key.level) as usize] += 1;
                    }
                    (scale, offset, source_key.level, Some(source_key), true)
                } else {
                    ([1.0, 1.0], [0.0, 0.0], render_node.node.level, None, false)
                };
            let [u_min, v_min, u_max, v_max] = render_node.node.uv_bounds();
            let anchor_direction = render_node.node.center_direction().as_vec3().normalize();
            let anchor_world = DVec3::new(
                f64::from(anchor_direction.x),
                f64::from(anchor_direction.y),
                f64::from(anchor_direction.z),
            ) * PLANET_RADIUS_METERS;
            let anchor_u = (u_min + u_max) * 0.5;
            let anchor_v = (v_min + v_max) * 0.5;
            prepared_instances.push((
                tile_key,
                TerrainInstance {
                    anchor_view_position: camera_view_basis
                        .world_to_view(anchor_world - camera_world)
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
                    edge_stitch: if render_node.active {
                        edge_stitch_info(render_node.node, &active_render_nodes)
                    } else {
                        0
                    },
                    node_uv_origin_span: [
                        u_min as f32,
                        v_min as f32,
                        (u_max - u_min) as f32,
                        (v_max - v_min) as f32,
                    ],
                    node_anchor_direction_cube_length: [
                        anchor_direction.x,
                        anchor_direction.y,
                        anchor_direction.z,
                        (1.0 + anchor_u * anchor_u + anchor_v * anchor_v).sqrt() as f32,
                    ],
                },
            ));
        }
        // A single canonical vertex buffer makes leaves with the same source
        // tile genuinely instanced. Global L4 fallback therefore costs a few
        // draw calls rather than one call and one vertex buffer per leaf.
        prepared_instances.sort_unstable_by_key(|(tile_key, _)| *tile_key);
        let mut instances = Vec::with_capacity(prepared_instances.len());
        for (tile_key, instance) in prepared_instances {
            let instance_index = instances.len() as u32;
            if let Some(batch) = self.draw_batches.last_mut()
                && batch.tile_key == tile_key
            {
                batch.instance_count += 1;
            } else {
                self.draw_batches.push(DrawBatch {
                    first_instance: instance_index,
                    instance_count: 1,
                    tile_key,
                });
            }
            instances.push(instance);
        }
        self.ensure_instance_capacity(instances.len());
        if !instances.is_empty() {
            self.queue
                .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }

        let metrics = lod_update.metrics;
        let max_seam_delta_meters = if matches!(&self.source, TerrainDataSource::Outmap(_)) {
            if topology_changed || tiles_loaded > 0 {
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
            drawn_chunks: render_nodes.len() as u32,
            terrain_triangles: render_nodes.len() as u64 * u64::from(self.index_count / 3),
            chunks_loaded: 0,
            chunks_unloaded: 0,
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
            source_level_delta_histogram,
            lod_thrash_events: metrics.lod_thrash_events,
            draw_calls: self.draw_batches.len() as u32,
        })
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
    ) {
        let pipeline = if self.fading_out_chunks.is_empty() && self.fade_in_started_at.is_empty() {
            &self.stable_pipeline
        } else {
            &self.transition_pipeline
        };
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.set_vertex_buffer(0, self.chunk_vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        for batch in &self.draw_batches {
            let tile = batch.tile_key.map_or(&self.placeholder_tile, |key| {
                self.tile_cache
                    .get(&key)
                    .expect("draw batch has a resident terrain tile")
            });
            render_pass.set_bind_group(1, &tile.bind_group, &[]);
            render_pass.draw_indexed(
                0..self.index_count,
                0,
                batch.first_instance..batch.first_instance + batch.instance_count,
            );
        }
    }

    fn ensure_instance_capacity(&mut self, required: usize) {
        if required <= self.instance_capacity {
            return;
        }
        self.instance_capacity = required.next_power_of_two();
        self.instance_buffer = create_instance_buffer(&self.device, self.instance_capacity);
    }

    fn update_lod_transitions(
        &mut self,
        active_render_nodes: &BTreeSet<QuadtreeNode>,
        presentation_time: f64,
    ) {
        if self.active_render_nodes.is_empty() {
            self.active_render_nodes = active_render_nodes.clone();
            return;
        }

        // A node which becomes active again must stop fading out. This can
        // happen when the camera reverses inside the LOD hysteresis band.
        self.fading_out_chunks
            .retain(|node, _| !active_render_nodes.contains(node));

        let (outgoing, incoming) =
            lod_transition_nodes(&self.active_render_nodes, active_render_nodes);
        if should_animate_lod_transition(
            self.fading_out_chunks.len(),
            incoming.len(),
            outgoing.len(),
        ) {
            for node in outgoing {
                self.fading_out_chunks.insert(
                    node,
                    FadingChunk {
                        started_at_presentation_time: presentation_time,
                    },
                );
            }
            for node in incoming {
                self.fade_in_started_at.insert(node, presentation_time);
            }
        }
        self.active_render_nodes = active_render_nodes.clone();
    }

    fn purge_expired_lod_transitions(&mut self, presentation_time: f64) {
        purge_expired_lod_transitions(
            &mut self.fading_out_chunks,
            &mut self.fade_in_started_at,
            &self.active_render_nodes,
            presentation_time,
        );
    }
}

fn purge_expired_lod_transitions(
    fading_out_chunks: &mut BTreeMap<QuadtreeNode, FadingChunk>,
    fade_in_started_at: &mut HashMap<QuadtreeNode, f64>,
    active_render_nodes: &BTreeSet<QuadtreeNode>,
    presentation_time: f64,
) {
    fading_out_chunks.retain(|_, fading| {
        presentation_time - fading.started_at_presentation_time < LOD_TRANSITION_DURATION_SECONDS
    });
    fade_in_started_at.retain(|node, started_at_presentation_time| {
        active_render_nodes.contains(node)
            && presentation_time - *started_at_presentation_time < LOD_TRANSITION_DURATION_SECONDS
    });
}

fn edge_stitch_info(node: QuadtreeNode, active_nodes: &[QuadtreeNode]) -> u32 {
    let [u_min, v_min, u_max, v_max] = node.uv_bounds();
    let edge_span = u_max - u_min;
    let outside = edge_span * 1.0e-5;
    let mut packed = 0_u32;
    for edge in 0..4_u32 {
        let mut maximum_delta = 0_u8;
        for sample in 0..8 {
            let amount = (f64::from(sample) + 0.5) / 8.0;
            let u = u_min + (u_max - u_min) * amount;
            let v = v_min + (v_max - v_min) * amount;
            let (outside_u, outside_v) = match edge {
                0 => (u, v_min - outside),
                1 => (u_max + outside, v),
                2 => (u, v_max + outside),
                _ => (u_min - outside, v),
            };
            let direction = cube_face_direction(node.face, outside_u, outside_v);
            if let Some(neighbor) = active_node_at_direction(active_nodes, direction)
                && neighbor.level < node.level
            {
                // Collapsing a full 32-quad edge across a 5-level gap creates
                // one enormous visible fan triangle. Two levels still match
                // the established stitched-grid contract; larger topology
                // gaps fall back to the shallow skirts instead of destroying
                // the fine chunk's silhouette.
                maximum_delta = maximum_delta.max((node.level - neighbor.level).min(2));
            }
        }
        packed |= u32::from(maximum_delta) << (edge * 3);
    }
    packed
}

fn active_node_at_direction(
    active_nodes: &[QuadtreeNode],
    direction: DVec3,
) -> Option<QuadtreeNode> {
    let (face, face_uv) = cube_face_uv(direction)?;
    active_nodes.iter().copied().find(|node| {
        let Some(node_face) = CubeFace::from_index(node.face) else {
            return false;
        };
        let key = TileKey {
            face: node_face,
            level: node.level,
            x: node.x,
            y: node.y,
        };
        source_tile_uv(key, face, face_uv).is_some()
    })
}

#[cfg(test)]
fn edge_stitch_level_delta(packed: u32, edge: u32) -> u8 {
    ((packed >> (edge * 3)) & 0x7) as u8
}

fn lod_transition_progress(sim_time: f64, started_at_sim_time: f64) -> f32 {
    let linear =
        ((sim_time - started_at_sim_time) / LOD_TRANSITION_DURATION_SECONDS).clamp(0.0, 1.0);
    (linear * linear * (3.0 - 2.0 * linear)) as f32
}

fn should_animate_lod_transition(
    fading_nodes: usize,
    loaded_nodes: usize,
    unloaded_nodes: usize,
) -> bool {
    loaded_nodes.saturating_add(unloaded_nodes) <= MAX_ANIMATED_LOD_TOPOLOGY_CHANGES
        && fading_nodes.saturating_add(unloaded_nodes) <= MAX_ANIMATED_LOD_TOPOLOGY_CHANGES
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

fn lod_transition_nodes(
    previous: &BTreeSet<QuadtreeNode>,
    current: &BTreeSet<QuadtreeNode>,
) -> (Vec<QuadtreeNode>, Vec<QuadtreeNode>) {
    let incoming: Vec<_> = current
        .difference(previous)
        .copied()
        .filter(|node| {
            previous
                .iter()
                .any(|previous| nodes_share_lod_transition(*node, *previous))
        })
        .collect();
    let outgoing = previous
        .difference(current)
        .copied()
        .filter(|node| {
            incoming
                .iter()
                .any(|incoming| nodes_share_lod_transition(*node, *incoming))
        })
        .collect();
    (outgoing, incoming)
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

fn texture_array_layout_entry(
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
    terrain_material_view: &wgpu::TextureView,
    terrain_material_sampler: &wgpu::Sampler,
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
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(terrain_material_view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Sampler(terrain_material_sampler),
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

fn create_terrain_material_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let mip_level_count = TERRAIN_MATERIAL_TEXTURE_SIZE.ilog2() + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mipmapped terrain material array"),
        size: wgpu::Extent3d {
            width: TERRAIN_MATERIAL_TEXTURE_SIZE,
            height: TERRAIN_MATERIAL_TEXTURE_SIZE,
            depth_or_array_layers: TERRAIN_MATERIAL_LAYER_COUNT,
        },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // The generated palettes are authored in display space. Sampling an
        // sRGB texture gives the lighting shader linear albedo values.
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for layer in 0..TERRAIN_MATERIAL_LAYER_COUNT {
        let mut mip_size = TERRAIN_MATERIAL_TEXTURE_SIZE;
        let mut texels = terrain_material_layer_texels(layer, mip_size as usize);
        for mip_level in 0..mip_level_count {
            let padded_texels = padded_texture_rows(&texels, mip_size, mip_size, 4);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &padded_texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned_texture_row_bytes(mip_size * 4)),
                    rows_per_image: Some(mip_size),
                },
                wgpu::Extent3d {
                    width: mip_size,
                    height: mip_size,
                    depth_or_array_layers: 1,
                },
            );
            if mip_size == 1 {
                break;
            }
            texels = downsample_srgb_rgba8(&texels, mip_size as usize);
            mip_size /= 2;
        }
    }

    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("terrain material array view"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("mipmapped terrain material sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: 8,
        ..Default::default()
    });
    (texture, view, sampler)
}

fn terrain_material_layer_texels(layer: u32, texture_size: usize) -> Vec<u8> {
    let mut texels = Vec::with_capacity(texture_size * texture_size * 4);
    for y in 0..texture_size {
        for x in 0..texture_size {
            texels.extend_from_slice(&terrain_material_texel(layer, x, y, texture_size));
        }
    }
    texels
}

fn terrain_material_texel(layer: u32, x: usize, y: usize, texture_size: usize) -> [u8; 4] {
    debug_assert!(layer < TERRAIN_MATERIAL_LAYER_COUNT);
    let seed = 0x51f1_5e5d_u32.wrapping_add(layer.wrapping_mul(0x9e37_79b9));
    let broad = tileable_value_noise_seeded(x, y, 64, texture_size, seed);
    let medium = tileable_value_noise_seeded(x, y, 16, texture_size, seed ^ 0xa511_e9b3);
    let fine = tileable_value_noise_seeded(x, y, 4, texture_size, seed ^ 0x63d8_3595);
    let grain = tileable_detail_hash(
        (x % texture_size) as u32,
        (y % texture_size) as u32,
        seed ^ 0xc2b2_ae35,
    );

    let (low, high, color_amount, height) = match layer {
        // Vegetation: dark organic ground with drier broad patches.
        0 => (
            [0.055, 0.12, 0.035],
            [0.34, 0.33, 0.12],
            (broad * 0.64 + medium * 0.28 + grain * 0.08).clamp(0.0, 1.0),
            (0.24 + medium * 0.50 + fine * 0.20 + grain * 0.06).clamp(0.0, 1.0),
        ),
        // Earth: soil, sand, and exposed dry ground.
        1 => (
            [0.19, 0.105, 0.045],
            [0.64, 0.48, 0.25],
            (broad * 0.52 + medium * 0.36 + fine * 0.12).clamp(0.0, 1.0),
            (0.18 + broad * 0.24 + medium * 0.42 + fine * 0.16).clamp(0.0, 1.0),
        ),
        // Rock: broad mineral variation with fine fracture-like contrast.
        2 => {
            let fracture = (2.0 * (medium - 0.5).abs()).powf(3.0);
            (
                [0.15, 0.145, 0.14],
                [0.52, 0.49, 0.44],
                (broad * 0.44 + fine * 0.28 + fracture * 0.28).clamp(0.0, 1.0),
                (0.22 + broad * 0.30 + medium * 0.34 + fine * 0.14).clamp(0.0, 1.0),
            )
        }
        // Snow: cool compacted hollows with warmer wind-polished ridges.
        _ => (
            [0.59, 0.69, 0.76],
            [0.97, 0.975, 0.95],
            (broad * 0.56 + medium * 0.30 + grain * 0.14).clamp(0.0, 1.0),
            (0.38 + broad * 0.34 + medium * 0.20 + fine * 0.08).clamp(0.0, 1.0),
        ),
    };
    let color = [
        low[0] + (high[0] - low[0]) * color_amount,
        low[1] + (high[1] - low[1]) * color_amount,
        low[2] + (high[2] - low[2]) * color_amount,
    ];
    [
        normalized_u8(color[0]),
        normalized_u8(color[1]),
        normalized_u8(color[2]),
        normalized_u8(height),
    ]
}

fn normalized_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn downsample_srgb_rgba8(texels: &[u8], texture_size: usize) -> Vec<u8> {
    debug_assert!(texture_size.is_power_of_two());
    debug_assert_eq!(texels.len(), texture_size * texture_size * 4);
    let next_size = (texture_size / 2).max(1);
    let mut downsampled = Vec::with_capacity(next_size * next_size * 4);
    for y in 0..next_size {
        for x in 0..next_size {
            let mut linear_rgb = [0.0_f32; 3];
            let mut alpha = 0.0_f32;
            for offset_y in 0..2.min(texture_size) {
                for offset_x in 0..2.min(texture_size) {
                    let source_x = (x * 2 + offset_x).min(texture_size - 1);
                    let source_y = (y * 2 + offset_y).min(texture_size - 1);
                    let index = (source_x + source_y * texture_size) * 4;
                    for channel in 0..3 {
                        linear_rgb[channel] +=
                            srgb_to_linear_channel(f32::from(texels[index + channel]) / 255.0);
                    }
                    alpha += f32::from(texels[index + 3]) / 255.0;
                }
            }
            let sample_count = if texture_size == 1 { 1.0 } else { 4.0 };
            for value in linear_rgb {
                downsampled.push(normalized_u8(linear_to_srgb_channel(value / sample_count)));
            }
            downsampled.push(normalized_u8(alpha / sample_count));
        }
    }
    downsampled
}

fn srgb_to_linear_channel(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_channel(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
fn tileable_value_noise(x: usize, y: usize, cell_size: usize, texture_size: usize) -> f32 {
    tileable_value_noise_seeded(x, y, cell_size, texture_size, 0)
}

fn tileable_value_noise_seeded(
    x: usize,
    y: usize,
    cell_size: usize,
    texture_size: usize,
    seed: u32,
) -> f32 {
    let cells = texture_size / cell_size;
    let cell_x = x / cell_size;
    let cell_y = y / cell_size;
    let amount_x = (x % cell_size) as f32 / cell_size as f32;
    let amount_y = (y % cell_size) as f32 / cell_size as f32;
    let fade_x = amount_x * amount_x * (3.0 - 2.0 * amount_x);
    let fade_y = amount_y * amount_y * (3.0 - 2.0 * amount_y);
    let sample = |offset_x, offset_y| {
        let hash_x = (cell_x + offset_x) % cells;
        let hash_y = (cell_y + offset_y) % cells;
        tileable_detail_hash(hash_x as u32, hash_y as u32, seed)
    };
    let lower = sample(0, 0) + (sample(1, 0) - sample(0, 0)) * fade_x;
    let upper = sample(0, 1) + (sample(1, 1) - sample(0, 1)) * fade_x;
    lower + (upper - lower) * fade_y
}

fn tileable_detail_hash(x: u32, y: u32, seed: u32) -> f32 {
    let mut value = x
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(y.wrapping_mul(0x85eb_ca6b))
        .wrapping_add(seed);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    (value & 0xffff) as f32 / 65_535.0
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
    let padded_bytes =
        padded_texture_rows(bytes, TILE_STORED_SIZE, TILE_STORED_SIZE, bytes_per_texel);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &padded_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned_texture_row_bytes(
                TILE_STORED_SIZE * bytes_per_texel,
            )),
            rows_per_image: Some(TILE_STORED_SIZE),
        },
        extent,
    );
    texture
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

fn outmap_node_level_limit(outmap: &Outmap, node: QuadtreeNode) -> u8 {
    debug_assert_eq!(
        (TILE_LOGICAL_SIZE - 1) / CHUNK_GRID_QUADS as u32,
        1_u32 << OUTMAP_TILE_GRID_SUBDIVISION_LEVELS
    );
    let requested_key = tile_key(node).expect("quadtree nodes always use valid cube faces");
    let source_key = outmap
        .resolve_tile(requested_key)
        .expect("a validated outmap contains a root tile for every cube face");
    source_key
        .level
        .saturating_add(OUTMAP_TILE_GRID_SUBDIVISION_LEVELS)
        .min(MAX_LOD_LEVEL)
}

fn cached_tile_ancestor(
    requested_key: TileKey,
    mut source_key: TileKey,
    tile_cache: &HashMap<TileKey, GpuTile>,
) -> Option<TileKey> {
    debug_assert!(source_key.level <= requested_key.level);
    loop {
        if tile_cache.contains_key(&source_key) {
            return Some(source_key);
        }
        source_key = source_key.parent()?;
    }
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
    use std::collections::{BTreeMap, BTreeSet};

    use glam::DVec3;

    use super::{
        FadingChunk, LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS,
        OUTMAP_TILE_GRID_SUBDIVISION_LEVELS, TERRAIN_MATERIAL_LAYER_COUNT,
        TERRAIN_MATERIAL_TEXTURE_SIZE, TerrainSettings, aligned_texture_row_bytes, cube_face_uv,
        downsample_srgb_rgba8, edge_stitch_info, edge_stitch_level_delta, fallback_uv_transform,
        lod_transition_nodes, lod_transition_progress, nodes_share_lod_transition,
        pack_terrain_info, padded_texture_rows, planet_shader_source,
        purge_expired_lod_transitions, sample_height_cpu, should_animate_lod_transition,
        source_tile_uv_at_direction, terrain_material_layer_texels, terrain_material_texel,
        tileable_value_noise,
    };
    use crate::planet::{
        GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE, MAX_LOD_LEVEL, OUTMAP_TERRAIN_FAR_HEIGHT_SCALE,
        OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE, PLANET_RADIUS_METERS, PlanetLod, QuadtreeNode,
        build_chunk_mesh, cube_face_direction,
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
    fn source_tile_samples_are_consumed_after_two_grid_subdivisions() {
        assert_eq!(
            (TILE_LOGICAL_SIZE - 1) / crate::planet::CHUNK_GRID_QUADS as u32,
            1_u32 << OUTMAP_TILE_GRID_SUBDIVISION_LEVELS
        );
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
        let shader = planet_shader_source();
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
    fn shader_uses_baked_displacement_and_real_light() {
        let shader = planet_shader_source();
        let terrain_height = shader
            .split("fn terrain_height(")
            .nth(1)
            .and_then(|source| source.split("fn gerstner_wave(").next())
            .expect("terrain height function is present");
        assert!(!terrain_height.contains("global_terrain_detail("));
        assert!(terrain_height.contains("macro_height * terrain_macro_height_scale()"));
        assert!(!shader.contains("requested_lod_level: f32"));
        assert!(shader.contains("biome_color(2u) * 0.65 * ice_light_floor"));
        assert!(!shader.contains("max(lit_surface_color, biome_color(2u) * 0.65)"));
    }

    #[test]
    fn planet_shader_validates_without_runtime_detail_noise() {
        let shader = planet_shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&shader)
            .expect("planet shader must parse before WGPU creates the pipeline");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("planet shader must validate before WGPU creates the pipeline");
        assert!(shader.contains("fn fs_main_stable("));
        assert!(!shader.contains("fn terrain_detail_value_noise("));
        assert!(!shader.contains("fn global_terrain_detail("));
    }

    #[test]
    fn terrain_material_layers_are_tileable_mipmapped_and_bound_in_the_shader() {
        for cell_size in [32, 8, 2] {
            let edge = tileable_value_noise(0, 47, cell_size, 128);
            assert!((0.0..=1.0).contains(&edge));
            assert_eq!(edge, tileable_value_noise(128, 47, cell_size, 128));
        }
        let layer_samples: Vec<_> = (0..TERRAIN_MATERIAL_LAYER_COUNT)
            .map(|layer| {
                let first =
                    terrain_material_texel(layer, 0, 47, TERRAIN_MATERIAL_TEXTURE_SIZE as usize);
                assert_eq!(
                    first,
                    terrain_material_texel(
                        layer,
                        TERRAIN_MATERIAL_TEXTURE_SIZE as usize,
                        47,
                        TERRAIN_MATERIAL_TEXTURE_SIZE as usize,
                    )
                );
                assert!(first[3] > 0);
                first
            })
            .collect();
        assert!(layer_samples.windows(2).all(|pair| pair[0] != pair[1]));

        let mut mip_size = TERRAIN_MATERIAL_TEXTURE_SIZE as usize;
        let mut mip = terrain_material_layer_texels(0, mip_size);
        let mut mip_count = 1;
        while mip_size > 1 {
            mip = downsample_srgb_rgba8(&mip, mip_size);
            mip_size /= 2;
            mip_count += 1;
            assert_eq!(mip.len(), mip_size * mip_size * 4);
        }
        assert_eq!(mip_count, TERRAIN_MATERIAL_TEXTURE_SIZE.ilog2() + 1);

        let shader = planet_shader_source();
        assert!(shader.contains("@group(1) @binding(6)"));
        assert!(shader.contains("var terrain_material_map: texture_2d_array<f32>"));
        assert!(shader.contains("fn triplanar_material_sample_at_position("));
        assert!(shader.contains("fn triplanar_material_sample("));
        assert!(!shader.contains("TERRAIN_MATERIAL_WARP_FREQUENCY"));
        assert!(!shader.contains("TERRAIN_MATERIAL_FINE_SCALE"));
        assert!(!shader.contains("texture_warp"));
        assert!(shader.contains("fn sample_biome_blend("));
        assert!(shader.contains("fn blended_biome_color("));
        assert!(shader.contains("fn terrain_material_weights_for_biome("));
        assert!(shader.contains("fn height_blend_material_weights("));
        assert!(shader.contains("fn terrain_material_tint("));
    }

    #[test]
    fn texture_upload_rows_are_padded_without_changing_texels() {
        let source: Vec<_> = (0..(3 * 2)).collect();
        let padded = padded_texture_rows(&source, 3, 2, 1);
        assert_eq!(aligned_texture_row_bytes(3), 256);
        assert_eq!(padded.len(), 512);
        assert_eq!(&padded[..3], &[0, 1, 2]);
        assert_eq!(&padded[256..259], &[3, 4, 5]);
    }

    #[test]
    fn shaders_use_the_same_altitude_aware_twilight_column() {
        let planet_shader = planet_shader_source();
        for shader in [planet_shader.as_str(), include_str!("atmosphere.wgsl")] {
            assert!(shader.contains(
                "fn twilight_solar_air_mass(solar_zenith_cosine: f32, sample_altitude_meters: f32)"
            ));
            assert!(shader.contains("upper_atmosphere_amount"));
            assert!(shader.contains("horizon_amount"));
        }
    }

    #[test]
    fn direct_surface_sunlight_fades_before_geometric_sunset() {
        let shader = planet_shader_source();
        let normalized_shader = shader.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(shader.contains("let solar_elevation = dot(surface_direction, sun_direction);"));
        assert!(
            shader.contains("smoothstep(\n        -0.01,\n        0.08,\n        solar_elevation,")
        );
        assert!(normalized_shader.contains("sun_transmittance * specular"));
        assert!(normalized_shader.contains("sun_transmittance * direct_light"));
    }

    #[test]
    fn direct_surface_sunlight_progresses_from_orange_to_red_before_darkness() {
        let shader = planet_shader_source();
        let normalized_shader = shader.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(shader.contains("let orange_tint = vec3<f32>(1.20, 0.55, 0.16);"));
        assert!(shader.contains("let red_tint = vec3<f32>(1.35, 0.12, 0.03);"));
        assert!(shader.contains("return transmitted_sunlight * low_sun_tint * solar_visibility;"));
        assert!(normalized_shader.contains("sun_transmittance * specular"));
        assert!(normalized_shader.contains("sun_transmittance * direct_light"));
    }

    #[test]
    fn ocean_aerial_perspective_preserves_the_dark_water_body() {
        let shader = planet_shader_source();
        assert!(shader.contains("const OCEAN_AERIAL_PERSPECTIVE_WEIGHT: f32 = 0.35;"));
        assert_eq!(shader.matches("ocean_aerial_perspective(").count(), 4);
        assert!(shader.contains("water_surface_color,\n        aerial_color,"));
    }

    #[test]
    fn terrain_material_pass_uses_displaced_slope_and_latitude_snowline() {
        let shader = planet_shader_source();
        assert!(shader.contains("let rock_amount = smoothstep(0.10, 0.42, slope);"));
        assert!(shader.contains("let snowline_meters = mix(6200.0, 2200.0, latitude_amount);"));
        assert!(shader.contains("camera_distance_meters * 0.01"));
        assert!(shader.contains("TERRAIN_NORMAL_MIN_SAMPLE_METERS"));
        assert!(shader.contains("let normal_step_scale = cube_step / requested_cube_step;"));
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
    fn fine_edges_stitch_to_the_coarser_resident_grid() {
        let coarse = QuadtreeNode {
            face: 0,
            level: 1,
            x: 0,
            y: 0,
        };
        let fine = QuadtreeNode {
            face: 0,
            level: 3,
            x: 4,
            y: 0,
        };
        let active = [coarse, fine];
        let stitch = edge_stitch_info(fine, &active);

        assert_eq!(edge_stitch_level_delta(stitch, 0), 0);
        assert_eq!(edge_stitch_level_delta(stitch, 1), 0);
        assert_eq!(edge_stitch_level_delta(stitch, 2), 0);
        assert_eq!(edge_stitch_level_delta(stitch, 3), 2);
        assert_eq!(edge_stitch_info(coarse, &active), 0);

        let face_edge_fine = QuadtreeNode {
            face: CubeFace::PositiveX.index(),
            level: 3,
            x: 7,
            y: 2,
        };
        let adjacent_face_coarse = QuadtreeNode {
            face: CubeFace::NegativeZ.index(),
            level: 1,
            x: 0,
            y: 0,
        };
        let face_edge_stitch =
            edge_stitch_info(face_edge_fine, &[face_edge_fine, adjacent_face_coarse]);
        assert_eq!(edge_stitch_level_delta(face_edge_stitch, 1), 2);

        let extreme_fine = QuadtreeNode {
            face: 0,
            level: 8,
            x: 128,
            y: 0,
        };
        let extreme_stitch = edge_stitch_info(extreme_fine, &[coarse, extreme_fine]);
        assert_eq!(edge_stitch_level_delta(extreme_stitch, 3), 2);

        let shader = planet_shader_source();
        assert!(shader.contains("fn stitched_tile_uv("));
        assert!(shader.contains("fn stitched_surface_direction("));
        assert!(shader.contains("fn lod_morphed_tile_uv("));
        assert!(shader.contains("@location(10) node_uv_origin_span: vec4<f32>"));
        assert!(shader.contains("@location(11) node_anchor_direction_cube_length: vec4<f32>"));
        assert!(shader.contains("let stride = 1u << min(level_delta, 2u);"));
    }

    #[test]
    fn low_flight_lod_is_not_capped_by_sparse_source_tiles() {
        let mut lod = PlanetLod::default();
        let camera = DVec3::X * (PLANET_RADIUS_METERS + 16_000.0);
        let update = lod.update_for_view_with_constraints(
            camera,
            -DVec3::X,
            DVec3::Y,
            16.0 / 9.0,
            1_080,
            60.0_f64.to_radians(),
            super::OUTMAP_GEOMETRIC_ERROR_RATIO,
            &|_| MAX_LOD_LEVEL,
        );

        assert!(16_000.0 < LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS);
        assert!(
            update.metrics.max_level > 6,
            "low flight must refine geometry beyond sparse L6 source tiles"
        );
        assert!(!update.metrics.budget_limited);
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
    fn lod_transition_progress_eases_to_full_coverage_after_half_a_second() {
        assert_eq!(lod_transition_progress(10.0, 10.0), 0.0);
        assert!((lod_transition_progress(10.125, 10.0) - 0.15625).abs() < f32::EPSILON);
        assert!((lod_transition_progress(10.25, 10.0) - 0.5).abs() < f32::EPSILON);
        assert!((lod_transition_progress(10.375, 10.0) - 0.84375).abs() < f32::EPSILON);
        assert_eq!(lod_transition_progress(10.5, 10.0), 1.0);
        assert_eq!(lod_transition_progress(12.0, 10.0), 1.0);

        let shader = planet_shader_source();
        assert!(shader.contains("fn lod_dither_threshold("));
        assert!(shader.contains("52.9829189 * fract(dot(pixel"));
        assert!(shader.contains("incoming && threshold >= transition_progress"));
        assert!(shader.contains("!incoming && threshold < transition_progress"));
    }

    #[test]
    fn presentation_time_expires_lod_fades_while_scene_time_is_frozen() {
        let parent = QuadtreeNode {
            face: 0,
            level: 4,
            x: 3,
            y: 5,
        };
        let child = parent.children()[0];
        let frozen_scene_time = 12.0;
        let mut fading_out = BTreeMap::from([(
            parent,
            FadingChunk {
                started_at_presentation_time: 20.0,
            },
        )]);
        let mut fading_in = std::collections::HashMap::from([(child, 20.0)]);
        let active = BTreeSet::from([child]);

        purge_expired_lod_transitions(&mut fading_out, &mut fading_in, &active, 20.49);
        assert_eq!(frozen_scene_time, 12.0);
        assert_eq!(fading_out.len(), 1);
        assert_eq!(fading_in.len(), 1);

        purge_expired_lod_transitions(&mut fading_out, &mut fading_in, &active, 20.5);
        assert_eq!(frozen_scene_time, 12.0);
        assert!(fading_out.is_empty());
        assert!(fading_in.is_empty());
    }

    #[test]
    fn parent_child_replacement_cross_fades_but_unrelated_motion_does_not() {
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
        let (outgoing, incoming) =
            lod_transition_nodes(&BTreeSet::from([parent]), &BTreeSet::from([child]));
        assert_eq!(outgoing, vec![parent]);
        assert_eq!(incoming, vec![child]);

        let (outgoing, incoming) =
            lod_transition_nodes(&BTreeSet::from([parent]), &BTreeSet::from([unrelated]));
        assert!(outgoing.is_empty());
        assert!(incoming.is_empty());
    }

    #[test]
    fn large_lod_changes_snap_instead_of_duplicating_draws() {
        assert!(should_animate_lod_transition(0, 32, 32));
        assert!(!should_animate_lod_transition(0, 33, 32));
        assert!(!should_animate_lod_transition(40, 16, 25));
        assert!(!should_animate_lod_transition(usize::MAX, 0, 1));
    }
}

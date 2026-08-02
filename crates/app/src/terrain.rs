use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    error::Error,
    fmt,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use catinthegarden_coretypes::{
    CubeFace, TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey, tile_key_for_direction,
};
use glam::DVec3;
use wgpu::util::DeviceExt;

use crate::{
    outmap::{Outmap, OutmapError, TileData},
    planet::{
        CHUNK_GRID_QUADS, CameraViewBasis, ChunkVertex, GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS,
        GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE, GeometricErrorRatio, LodPolicy, MAX_LOD_LEVEL,
        MINIMUM_LOD_LEVEL, NEAR_FIELD_GRID_QUADS, OUTMAP_TERRAIN_FAR_HEIGHT_SCALE,
        OUTMAP_TERRAIN_HEIGHT_BLEND_END_METERS, OUTMAP_TERRAIN_HEIGHT_BLEND_START_METERS,
        OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE, PLANET_RADIUS_METERS, PlanetLod, QuadtreeNode,
        TERRAIN_DETAIL_MIN_FILTER_METERS, TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
        TerrainHeightRange, build_chunk_mesh, build_chunk_mesh_with_quads,
        continuous_baked_sample_spacing_meters, cube_face_basis, cube_face_direction,
        max_active_chunks_from_env, outmap_surface_height_meters,
        outmap_surface_height_meters_with_filter, placeholder_height_meters,
        scaled_outmap_macro_height_meters,
    },
};

// Material tiles are 131x131 stored samples, independent of the 33x33 mesh.
// Retain enough nearby L4 tiles to avoid camera-motion uploads while keeping
// the three per-tile GPU textures and CPU height cache bounded.
const MAX_RESIDENT_TERRAIN_TILES: usize = 384;
/// Bound main-thread texture creation even if the I/O worker completed a burst
/// while rendering was paused or slow.
const MAX_TILE_UPLOADS_PER_FRAME: usize = 4;
const FLAT_TRIANGLE_EXPERIMENT_DEFAULT: bool = true;

fn flat_triangle_experiment_from_env() -> bool {
    match std::env::var("CATINGARDEN_FLAT_TRIANGLES") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        ),
        Err(_) => FLAT_TRIANGLE_EXPERIMENT_DEFAULT,
    }
}

fn planet_shader_source() -> String {
    [
        include_str!("shared_planet.wgsl"),
        include_str!("planet.wgsl"),
    ]
    .join("\n")
}
const MAX_PENDING_TILE_LOADS: usize = 32;
/// Near-field prefetch is opportunistic for raster: visible geometry must keep
/// the first claim on the pending-load queue, otherwise a ray-oriented window
/// can starve the L18 tiles the raster frontier actually draws.
const MAX_RASTER_NEAR_FIELD_PREFETCH_PER_FRAME: usize = 4;
/// Dense near-field geometry is useful while a chunk still spans several
/// source texels. At finer LODs the canonical grid already samples the source
/// window at roughly one vertex per texel, so avoid paying for extra triangles.
const NEAR_FIELD_DENSE_MAX_LEVEL: u8 = 10;
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
/// Unresolved-height error relative to one geometry cell, projected from each
/// visible node's actual camera distance. The source-level cap below prevents
/// spending this error budget on repeated samples from a coarse ancestor tile.
///
/// Two contributions, and the second is the one that used to be missing.
///
/// The baked macro surface contributes curvature and resampling error. The
/// *synthesised ladder* contributes everything the mesh filtered out of its own
/// displacement: the vertex ladder fades octaves shorter than twice the vertex
/// spacing, and what it drops has RMS `ROUGHNESS * 2 * sqrt(4/3)` of that
/// spacing. Converted into this ratio's units (error is `pi/4 * ratio * vertex
/// spacing`) that is `ROUGHNESS * 2.9395`.
///
/// It has to be derived, because a constant here silently caps how steep the
/// terrain is allowed to be. At ROUGHNESS 0.0328 the ladder needs 0.0964 and
/// the flat 0.15 this replaces covered it -- by luck, since nothing connected
/// the two. At 0.06 it needs 0.1764, the selector went on believing 0.15, and
/// the result was the stair-stepped silhouettes the roughness experiment hit
/// and blamed on mesh density. The density was available; the selector was
/// simply not asking for it.
const LADDER_GEOMETRIC_ERROR_PER_ROUGHNESS: f64 = 2.939_5;
/// Back-calculated so the total reproduces the 0.15 that was in use and known
/// good at the current roughness: this change is meant to remove a ceiling, not
/// to move the tessellation everyone has been looking at.
const OUTMAP_BAKED_GEOMETRIC_ERROR_RATIO: f64 = 0.053_6;
const OUTMAP_LADDER_GEOMETRIC_ERROR_RATIO: f64 =
    crate::planet::TERRAIN_DETAIL_ROUGHNESS * LADDER_GEOMETRIC_ERROR_PER_ROUGHNESS;
/// What a node is charged while its children still have source texels to read.
/// Only the tests name it now; the selector takes the two terms apart.
#[cfg(test)]
const OUTMAP_GEOMETRIC_ERROR_RATIO: f64 =
    OUTMAP_BAKED_GEOMETRIC_ERROR_RATIO + OUTMAP_LADDER_GEOMETRIC_ERROR_RATIO;
/// The same two numbers, kept apart so the selector can stop charging for the
/// baked term once a node's children have read every source texel under them.
///
/// Charging it past that point is what made the mountains ask for sub-pixel
/// geometry: out there 151 of 256 chunks sit at L14 against L4 baked data, a
/// source-level delta of 10, and no split at that depth can resolve one more
/// texel of the macro surface. The ladder term is the part that stays honest
/// all the way down, because the ladder really does have another octave.
const OUTMAP_GEOMETRIC_ERROR: GeometricErrorRatio = GeometricErrorRatio {
    baked: OUTMAP_BAKED_GEOMETRIC_ERROR_RATIO,
    ladder: OUTMAP_LADDER_GEOMETRIC_ERROR_RATIO,
};
/// Below this altitude the camera is close enough that geometry density matters
/// more than source texel uniqueness. Ancestor tiles may feed finer grids while
/// the worker streams better sources; otherwise low flight stalls at L6 and
/// exposes huge terrain facets.
const LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS: f64 = 250_000.0;
const TERRAIN_INFO_SOURCE_EDGE_FADE_BIT: u32 = 1 << 14;
const TERRAIN_INFO_NEAR_FIELD_BIT: u32 = 1 << 15;
const TERRAIN_DETAIL_FILTER_RATIO: f64 = 0.01;

fn conservative_outmap_height_bounds(height_min_meters: f64, height_max_meters: f64) -> [f64; 2] {
    // Culling must enclose both displacement fields. The retired global field
    // remains in the uniform at a zero scale, while the live authored ladder
    // can add its full positive amplitude above the highest baked macro
    // sample. Omitting the latter made near-camera mountain patches disappear
    // when their real vertices rose outside the frustum's radial shell.
    [
        height_min_meters - GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS,
        height_max_meters * OUTMAP_TERRAIN_FAR_HEIGHT_SCALE
            + GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS * GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE
            + TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
    ]
}

/// Near-field window: a square of baked macro height around the camera, at a
/// level far finer than the six whole-face arrays the raymarch path holds.
///
/// The raymarch path samples the dense L0-L4 pyramid, which is 3068m per texel.
/// Measured at the landing site, that reads 815.3m where the finest baked data
/// reads 919.8m -- a 104m error, and the whole of the raymarch path's
/// disagreement with the ground the camera stands on. It is not a crater or a
/// missing feature: it is a coarse level averaging a local high away.
///
/// The fix does not need the finest data. The same measurement shows the
/// pyramid converging fast: L12 gives 919.34m, within half a metre of L18, and
/// an L12 tile spans 1.5km so eight of them cover 12.3km around the camera in a
/// single 1025-square texture. Everything below that is the analytic detail
/// ladder's job, and both paths already share it.
pub const NEAR_FIELD_WINDOW_TILES: u32 = 8;
pub const NEAR_FIELD_WINDOW_SAMPLES: u32 = NEAR_FIELD_WINDOW_TILES * (TILE_LOGICAL_SIZE - 1) + 1;
/// Metres of face arc, i.e. a quarter of the great circle.
const CUBE_FACE_ARC_METERS: f64 = std::f64::consts::PI * PLANET_RADIUS_METERS / 2.0;
/// The window never shrinks below this, so a camera on the ground always has
/// fine data out past its own horizon (4km at 2m of eye height).
const NEAR_FIELD_MIN_EXTENT_METERS: f64 = 12_000.0;
/// ...and it grows with height, so a climbing camera keeps fine data across
/// what it can actually see rather than a shrinking island under it.
const NEAR_FIELD_EXTENT_PER_CLEARANCE: f64 = 30.0;

/// Which square of which face the near-field window should cover.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NearFieldKey {
    pub face: CubeFace,
    pub level: u8,
    pub tile_x: u32,
    pub tile_y: u32,
}

/// The tiles a window would be assembled from. Cheap to compute (one resolve
/// per block) and the thing that decides whether a rebuild is needed: the key
/// alone is not enough, because a stationary camera keeps the same key while
/// streaming replaces coarse ancestors with the real thing underneath it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NearFieldSources {
    pub key: NearFieldKey,
    /// Row-major, `NEAR_FIELD_WINDOW_TILES` squared.
    pub(crate) source_keys: Vec<TileKey>,
}

pub struct NearFieldWindow {
    pub sources: NearFieldSources,
    /// `NEAR_FIELD_WINDOW_SAMPLES` square, row-major.
    pub heights_meters: Vec<f32>,
    /// Categorical owner resampled at the same coordinates as height.
    pub biome_ids: Vec<u8>,
    /// Bilinearly resampled material moisture, in the baked unorm encoding.
    pub moisture: Vec<u8>,
    /// Conservative bound for the ray marcher's empty-space skipping. Without
    /// it the marcher keeps the coarse pyramid's maximum and steps straight
    /// through ground the window has raised.
    /// A single ceiling for the whole window. Per-block ceilings were built and
    /// measured: no faster. The marcher's cost here is resolving ground a few
    /// metres away, not the empty space above it.
    pub max_height_meters: f32,
}

/// Residency behind the ray path's requested near-field window. Kept in the
/// capture manifest so an all-or-nothing window rejection can be distinguished
/// from a hit-refinement failure after a window was actually active.
#[derive(Clone, Debug, serde::Serialize)]
pub struct NearFieldCoverage {
    pub requested_face: u8,
    pub requested_level: u8,
    pub total_blocks: u32,
    pub resident_blocks: u32,
    pub finer_than_dense_blocks: u32,
    pub minimum_source_level: Option<u8>,
    pub maximum_source_level: Option<u8>,
    pub window_eligible: bool,
    pub active_window_level: Option<u8>,
}

/// Finest window level whose extent still covers what the camera can see.
///
/// Returns `None` when that is no finer than the dense pyramid the raymarch
/// path already holds, which is the case from orbit -- there the window would
/// be a slower copy of data already bound.
pub fn near_field_window_level(
    clearance_meters: f64,
    dense_level: u8,
    max_level: u8,
) -> Option<u8> {
    let required_extent_meters = NEAR_FIELD_MIN_EXTENT_METERS
        .max(clearance_meters.max(0.0) * NEAR_FIELD_EXTENT_PER_CLEARANCE);
    let tiles_per_side =
        f64::from(NEAR_FIELD_WINDOW_TILES) * CUBE_FACE_ARC_METERS / required_extent_meters;
    if !tiles_per_side.is_finite() || tiles_per_side < 1.0 {
        return None;
    }
    let level = tiles_per_side.log2().floor();
    if !level.is_finite() || level < 0.0 {
        return None;
    }
    let level = (level as u32).min(u32::from(max_level)) as u8;
    (level > dense_level).then_some(level)
}

/// A terrain height alongside the parts it was made of.

#[derive(Clone, Copy, Debug)]
pub struct SurfaceHeightBreakdown {
    pub height_meters: f64,
    /// Baked data only, with the altitude height scale applied and ocean
    /// resolved to sea level -- what the surface would be with no synthesised
    /// detail at all.
    pub macro_height_meters: f64,
    /// Pyramid level of the tile this came from. A coarse level here is the
    /// usual reason two sides of a comparison disagree about the macro shape.
    pub source_level: u8,
}

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
    pub ocean_chunks: u32,
    pub ocean_triangles: u64,
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
    outmap_detail: [f32; 4],
}

impl TerrainSettings {
    fn from_planet_constants(dense_level: u8) -> Self {
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
            outmap_detail: [f32::from(dense_level), 0.0, 0.0, 0.0],
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
    biome_ids: Vec<u8>,
    moisture: Vec<u8>,
    complete_logical_footprint_is_land: bool,
}

#[derive(Clone, Copy)]
struct DrawBatch {
    first_instance: u32,
    instance_count: u32,
    tile_key: Option<TileKey>,
    near_field: bool,
    dense_near_field: bool,
}

fn push_draw_batch_instance(
    batches: &mut Vec<DrawBatch>,
    tile_key: Option<TileKey>,
    near_field: bool,
    dense_near_field: bool,
    instance_index: u32,
) {
    if let Some(batch) = batches.last_mut()
        && batch.tile_key == tile_key
        && batch.near_field == near_field
        && batch.dense_near_field == dense_near_field
        && batch.first_instance + batch.instance_count == instance_index
    {
        batch.instance_count += 1;
    } else {
        batches.push(DrawBatch {
            first_instance: instance_index,
            instance_count: 1,
            tile_key,
            near_field,
            dense_near_field,
        });
    }
}

#[derive(Clone, Copy)]
struct RenderNode {
    node: QuadtreeNode,
    active: bool,
    transition_progress: f32,
    transition_incoming: bool,
}

#[derive(Clone, Copy)]
struct SurfaceDetailNode {
    node: QuadtreeNode,
    edge_stitch: u32,
    source_key: Option<TileKey>,
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
    ocean_transition_pipeline: wgpu::RenderPipeline,
    ocean_stable_pipeline: wgpu::RenderPipeline,
    terrain_tile_bind_group_layout: wgpu::BindGroupLayout,
    raster_near_field_bind_group: wgpu::BindGroup,
    shared_bind_group: wgpu::BindGroup,
    _terrain_settings_buffer: wgpu::Buffer,
    _environment_cubemap: wgpu::Texture,
    _terrain_material_texture: wgpu::Texture,
    _raster_near_field_height_texture: wgpu::Texture,
    _raster_near_field_biome_texture: wgpu::Texture,
    _raster_near_field_moisture_texture: wgpu::Texture,
    chunk_vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    near_field_vertex_buffer: wgpu::Buffer,
    near_field_index_buffer: wgpu::Buffer,
    near_field_index_count: u32,
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
    surface_detail_nodes: Vec<SurfaceDetailNode>,
    draw_batches: Vec<DrawBatch>,
    ocean_draw_batches: Vec<DrawBatch>,
    max_outmap_seam_delta_meters: f64,
    raster_near_field: Option<NearFieldSources>,
    flat_triangle_experiment: bool,
}

#[derive(Clone, Copy)]
struct RasterNearFieldBounds {
    face: u8,
    uv_min: [f64; 2],
    uv_span: f64,
}

impl TerrainRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        shared_bind_group_layout: wgpu::BindGroupLayout,
        source: TerrainSource,
    ) -> Result<Self, TerrainError> {
        let source = match source {
            TerrainSource::Placeholder => TerrainDataSource::Placeholder,
            TerrainSource::Outmap(root) => TerrainDataSource::Outmap(Outmap::open(root)?),
        };
        let flat_triangle_experiment = flat_triangle_experiment_from_env();
        let outmap_height_bounds = match &source {
            TerrainDataSource::Placeholder => None,
            TerrainDataSource::Outmap(outmap) => Some((
                f64::from(outmap.manifest().height_min_meters),
                f64::from(outmap.manifest().height_max_meters),
            )),
        };
        let terrain_height_range = match outmap_height_bounds {
            Some((height_min_meters, height_max_meters)) => {
                let [minimum, maximum] =
                    conservative_outmap_height_bounds(height_min_meters, height_max_meters);
                TerrainHeightRange::new(minimum, maximum)
            }
            None => TerrainHeightRange::default(),
        };
        let outmap_dense_level = match &source {
            TerrainDataSource::Placeholder => 0,
            TerrainDataSource::Outmap(outmap) => outmap.manifest().dense_level,
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
                contents: bytemuck::bytes_of(&TerrainSettings::from_planet_constants(
                    outmap_dense_level,
                )),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let terrain_tile_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terrain tile bind group layout"),
                entries: &[
                    texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                    texture_layout_entry(1, wgpu::TextureSampleType::Uint),
                    texture_layout_entry(2, wgpu::TextureSampleType::Float { filterable: false }),
                ],
            });
        let raster_near_field_height_texture = create_near_field_texture(
            device,
            "raster near-field height window",
            wgpu::TextureFormat::R32Float,
        );
        let raster_near_field_biome_texture = create_near_field_texture(
            device,
            "raster near-field biome window",
            wgpu::TextureFormat::R8Uint,
        );
        let raster_near_field_moisture_texture = create_near_field_texture(
            device,
            "raster near-field moisture window",
            wgpu::TextureFormat::R8Unorm,
        );
        let raster_near_field_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster near-field terrain bind group"),
            layout: &terrain_tile_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &raster_near_field_height_texture
                            .create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &raster_near_field_biome_texture
                            .create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &raster_near_field_moisture_texture
                            .create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&terrain_tile_bind_group_layout),
                Some(&shared_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader_source = planet_shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("planet raster shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let create_pipeline = |label, vertex_entry_point, fragment_entry_point| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex_entry_point),
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
        let transition_pipeline =
            create_pipeline("LOD terrain transition pipeline", "vs_main", "fs_main");
        let stable_pipeline =
            create_pipeline("LOD terrain stable pipeline", "vs_main", "fs_main_stable");
        let ocean_transition_pipeline =
            create_pipeline("LOD ocean transition pipeline", "vs_ocean", "fs_ocean");
        let ocean_stable_pipeline =
            create_pipeline("LOD ocean stable pipeline", "vs_ocean", "fs_ocean_stable");

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
        let near_field_topology =
            build_chunk_mesh_with_quads(QuadtreeNode::root(0), NEAR_FIELD_GRID_QUADS);
        let near_field_vertex_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shared near-field terrain grid"),
                contents: bytemuck::cast_slice(&near_field_topology.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let near_field_index_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shared near-field terrain indices"),
                contents: bytemuck::cast_slice(&near_field_topology.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        // The selector's budget and the instance buffer have to be the same
        // number, or a lifted budget silently draws only the first 256 chunks.
        let instance_capacity = max_active_chunks_from_env();
        let instance_buffer = create_instance_buffer(device, instance_capacity);
        let (environment_cubemap, environment_view, environment_sampler) =
            create_environment_cubemap(device, queue);
        let (terrain_material_texture, terrain_material_view, terrain_material_sampler) =
            create_terrain_material_texture(device, queue);
        let shared_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shared planet bind group"),
            layout: &shared_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&environment_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&environment_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: terrain_settings_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&terrain_material_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&terrain_material_sampler),
                },
            ],
        });
        let placeholder_tile = create_gpu_tile(
            device,
            queue,
            &terrain_tile_bind_group_layout,
            "placeholder terrain tile",
            &vec![0.0; tile_sample_count()],
            &vec![0; tile_sample_count()],
            &vec![128; tile_sample_count()],
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
                        &terrain_tile_bind_group_layout,
                        &label,
                        &tile.heights_meters,
                        &tile.biome_ids,
                        &tile.moisture,
                    ),
                );
            }
        }
        let initial_tile_last_used = initial_tile_cache.keys().map(|key| (*key, 0)).collect();

        let mut lod = if flat_triangle_experiment {
            PlanetLod::new(
                LodPolicy::fixed(MINIMUM_LOD_LEVEL),
                max_active_chunks_from_env(),
            )
        } else {
            PlanetLod::default()
        };
        lod.set_terrain_height_range(terrain_height_range);
        let renderer = Self {
            device: device.clone(),
            queue: queue.clone(),
            transition_pipeline,
            stable_pipeline,
            ocean_transition_pipeline,
            ocean_stable_pipeline,
            terrain_tile_bind_group_layout,
            raster_near_field_bind_group,
            shared_bind_group,
            _terrain_settings_buffer: terrain_settings_buffer,
            _environment_cubemap: environment_cubemap,
            _terrain_material_texture: terrain_material_texture,
            _raster_near_field_height_texture: raster_near_field_height_texture,
            _raster_near_field_biome_texture: raster_near_field_biome_texture,
            _raster_near_field_moisture_texture: raster_near_field_moisture_texture,
            chunk_vertex_buffer,
            index_buffer,
            index_count: topology.indices.len() as u32,
            near_field_vertex_buffer,
            near_field_index_buffer,
            near_field_index_count: near_field_topology.indices.len() as u32,
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
            surface_detail_nodes: Vec::new(),
            draw_batches: Vec::new(),
            ocean_draw_batches: Vec::new(),
            max_outmap_seam_delta_meters: 0.0,
            raster_near_field: None,
            flat_triangle_experiment,
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

    /// Makes the globally dense tile under an interactive flight start
    /// resident before placing the camera.
    ///
    /// Ordinary flight following remains resident-cache-only so it never adds
    /// disk I/O or GPU uploads to a movement frame. F4 is a one-time input,
    /// though, and starting above the highest summit from a coarse ancestor
    /// would initially put the camera hundreds of metres below the final L4
    /// surface before streaming caught up.
    pub fn prepare_flight_start_surface_height_meters(
        &mut self,
        local_surface_direction: DVec3,
        camera_altitude_meters: f64,
    ) -> Option<f64> {
        let source_key = match &self.source {
            TerrainDataSource::Placeholder => {
                return self
                    .surface_height_meters_at(local_surface_direction, camera_altitude_meters);
            }
            TerrainDataSource::Outmap(outmap) => {
                tile_key_for_direction(local_surface_direction, outmap.manifest().dense_level)
            }
        };
        if !self.tile_cache.contains_key(&source_key) {
            let TerrainDataSource::Outmap(outmap) = &self.source else {
                unreachable!("placeholder returned before loading a flight-start tile");
            };
            let tile = outmap.load_tile(source_key).ok()?;
            let label = format!("F4 flight-start terrain tile {source_key:?}");
            self.tile_cache.insert(
                source_key,
                create_gpu_tile(
                    &self.device,
                    &self.queue,
                    &self.terrain_tile_bind_group_layout,
                    &label,
                    &tile.heights_meters,
                    &tile.biome_ids,
                    &tile.moisture,
                ),
            );
        }
        self.tile_last_used.insert(source_key, self.tile_cache_tick);
        self.surface_height_meters_at(local_surface_direction, camera_altitude_meters)
    }

    pub fn shared_bind_group(&self) -> &wgpu::BindGroup {
        &self.shared_bind_group
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
        self.surface_height_breakdown_at(local_surface_direction, camera_altitude_meters)
            .map(|breakdown| breakdown.height_meters)
    }

    /// The same height, split into the part that came from baked data and the
    /// part the detail ladder synthesised. The surface probe needs the split to
    /// attribute a disagreement: a renderer drawing the macro surface alone
    /// looks exactly like one whose detail has the wrong amplitude, until the
    /// two terms are reported separately.
    pub fn surface_height_breakdown_at(
        &self,
        local_surface_direction: DVec3,
        camera_altitude_meters: f64,
    ) -> Option<SurfaceHeightBreakdown> {
        match &self.source {
            TerrainDataSource::Placeholder => Some(SurfaceHeightBreakdown {
                height_meters: placeholder_height_meters(local_surface_direction),
                macro_height_meters: placeholder_height_meters(local_surface_direction),
                source_level: 0,
            }),
            TerrainDataSource::Outmap(outmap) => {
                let (face, face_uv) = cube_face_uv(local_surface_direction)?;
                let dense_level = outmap.manifest().dense_level;
                self.tile_cache
                    .iter()
                    .filter_map(|(key, tile)| {
                        source_tile_uv(*key, face, face_uv)
                            .map(|uv| (key.level, sample_height_cpu(&tile.heights_meters, uv)))
                    })
                    .max_by_key(|(level, _)| *level)
                    .map(|(level, height)| {
                        let baked_meters = f64::from(height);
                        SurfaceHeightBreakdown {
                            height_meters: outmap_surface_height_meters(
                                baked_meters,
                                local_surface_direction,
                                camera_altitude_meters,
                                continuous_baked_sample_spacing_meters(face_uv, level, dense_level),
                            ),
                            macro_height_meters: if baked_meters <= 0.0 {
                                0.0
                            } else {
                                scaled_outmap_macro_height_meters(
                                    baked_meters,
                                    camera_altitude_meters,
                                )
                            },
                            source_level: level,
                        }
                    })
            }
        }
    }

    /// Samples the highest filtered raster surface drawn at this direction.
    /// Incoming and outgoing transition patches coexist and both write depth;
    /// runtime detail is signed, so neither the first patch nor the finest
    /// filter is a conservative collision surface.
    pub fn raster_surface_height_breakdown_at_distance(
        &self,
        local_surface_direction: DVec3,
        camera_altitude_meters: f64,
        camera_distance_meters: f64,
    ) -> Option<SurfaceHeightBreakdown> {
        assert!(camera_distance_meters.is_finite() && camera_distance_meters >= 0.0);
        match &self.source {
            TerrainDataSource::Placeholder => Some(SurfaceHeightBreakdown {
                height_meters: placeholder_height_meters(local_surface_direction),
                macro_height_meters: placeholder_height_meters(local_surface_direction),
                source_level: 0,
            }),
            TerrainDataSource::Outmap(outmap) => {
                let (face, face_uv) = cube_face_uv(local_surface_direction)?;
                let dense_level = outmap.manifest().dense_level;
                let rendered_surface = self
                    .surface_detail_nodes
                    .iter()
                    .filter(|surface| node_contains_face_uv(surface.node, face, face_uv))
                    .filter_map(|surface| {
                        let key = surface.source_key?;
                        let tile = self.tile_cache.get(&key)?;
                        let uv = source_tile_uv(key, face, face_uv)?;
                        let baked_meters = f64::from(sample_height_cpu(&tile.heights_meters, uv));
                        Some(SurfaceHeightBreakdown {
                            height_meters: outmap_surface_height_meters_with_filter(
                                baked_meters,
                                local_surface_direction,
                                camera_altitude_meters,
                                continuous_baked_sample_spacing_meters(
                                    face_uv,
                                    key.level,
                                    dense_level,
                                ),
                                surface_detail_filter_meters(
                                    *surface,
                                    face_uv,
                                    camera_distance_meters,
                                ),
                            ),
                            macro_height_meters: if baked_meters <= 0.0 {
                                0.0
                            } else {
                                scaled_outmap_macro_height_meters(
                                    baked_meters,
                                    camera_altitude_meters,
                                )
                            },
                            source_level: key.level,
                        })
                    })
                    .max_by(|left, right| left.height_meters.total_cmp(&right.height_meters));
                rendered_surface.or_else(|| {
                    self.tile_cache
                        .iter()
                        .filter_map(|(key, tile)| {
                            source_tile_uv(*key, face, face_uv)
                                .map(|uv| (key.level, sample_height_cpu(&tile.heights_meters, uv)))
                        })
                        .max_by_key(|(level, _)| *level)
                        .map(|(level, height)| {
                            let baked_meters = f64::from(height);
                            SurfaceHeightBreakdown {
                                height_meters: outmap_surface_height_meters(
                                    baked_meters,
                                    local_surface_direction,
                                    camera_altitude_meters,
                                    continuous_baked_sample_spacing_meters(
                                        face_uv,
                                        level,
                                        dense_level,
                                    ),
                                ),
                                macro_height_meters: if baked_meters <= 0.0 {
                                    0.0
                                } else {
                                    scaled_outmap_macro_height_meters(
                                        baked_meters,
                                        camera_altitude_meters,
                                    )
                                },
                                source_level: level,
                            }
                        })
                })
            }
        }
    }

    pub fn raster_surface_height_meters_at(
        &self,
        local_surface_direction: DVec3,
        camera_altitude_meters: f64,
    ) -> Option<f64> {
        self.raster_surface_height_breakdown_at_distance(
            local_surface_direction,
            camera_altitude_meters,
            0.0,
        )
        .map(|surface| surface.height_meters)
    }

    /// Where the near-field window should sit for this camera, or `None` when
    /// the dense pyramid already covers what it can see.
    pub fn near_field_key(
        &self,
        camera_local_direction: DVec3,
        clearance_meters: f64,
    ) -> Option<NearFieldKey> {
        let TerrainDataSource::Outmap(outmap) = &self.source else {
            return None;
        };
        let manifest = outmap.manifest();
        let level =
            near_field_window_level(clearance_meters, manifest.dense_level, manifest.max_level)?;
        let (face, face_uv) = cube_face_uv(camera_local_direction)?;
        let tiles_per_side = 1_u32 << level;
        // Centre the window on the camera, then pull it inside the face. A
        // window that hangs over an edge would need the neighbouring face's
        // tiles, and cube faces do not share a tile grid.
        let last_origin = tiles_per_side.saturating_sub(NEAR_FIELD_WINDOW_TILES);
        let origin = |coordinate: f64| -> u32 {
            let centre = (coordinate + 1.0) * 0.5 * f64::from(tiles_per_side);
            let low = centre - f64::from(NEAR_FIELD_WINDOW_TILES) * 0.5;
            (low.max(0.0) as u32).min(last_origin)
        };
        Some(NearFieldKey {
            face,
            level,
            tile_x: origin(face_uv[0]),
            tile_y: origin(face_uv[1]),
        })
    }

    /// Assembles the window from resident tiles.
    ///
    /// Each of the tile blocks is resolved once to the finest resident source
    /// covering it, then sampled per texel. Outside the sparse corridor that
    /// source is an L4 ancestor and the window is simply a resampled copy of
    /// what the raymarch path already had -- no better, but no worse, and the
    /// shader needs no coverage mask because the window is always complete.
    ///
    /// Returns `None` when any block has no resident source at all, in which
    /// case the caller keeps whatever window it already had rather than
    /// uploading a hole.
    /// Streams the tiles the near-field window needs, and keeps them resident.
    ///
    /// Nothing else asks for them. The quadtree streams what it draws, which at
    /// ground level is the L17/L18 corridor and, further out, L4 -- so the
    /// window's own level is skipped entirely and every block resolves to a
    /// coarse ancestor. Left to that, the window at the landing site was filled
    /// from the L0 tile and read 986m where the ground is 920m, which is worse
    /// than the pyramid it was meant to replace.
    pub fn request_near_field_tiles(&mut self, key: NearFieldKey) {
        self.request_near_field_tiles_budget(key, usize::MAX);
    }

    fn request_near_field_tiles_budget(&mut self, key: NearFieldKey, request_budget: usize) {
        let TerrainDataSource::Outmap(outmap) = &self.source else {
            return;
        };
        let mut wanted =
            Vec::with_capacity((NEAR_FIELD_WINDOW_TILES * NEAR_FIELD_WINDOW_TILES) as usize);
        for block_y in 0..NEAR_FIELD_WINDOW_TILES {
            for block_x in 0..NEAR_FIELD_WINDOW_TILES {
                let requested = TileKey {
                    face: key.face,
                    level: key.level,
                    x: key.tile_x + block_x,
                    y: key.tile_y + block_y,
                };
                if let Ok(preferred) = outmap.resolve_tile(requested) {
                    wanted.push(preferred);
                }
            }
        }
        let mut requested_count = 0;
        for source_key in wanted {
            if requested_count >= request_budget {
                break;
            }
            // Touch it either way: the eviction sweep only sees tiles the
            // render nodes used, and would drop the window's out from under it.
            if self.tile_cache.contains_key(&source_key) {
                self.tile_last_used.insert(source_key, self.tile_cache_tick);
                continue;
            }
            if self.pending_tile_loads.len() >= MAX_PENDING_TILE_LOADS {
                break;
            }
            if !self.pending_tile_loads.contains(&source_key)
                && self
                    .tile_load_requests
                    .as_ref()
                    .is_some_and(|requests| requests.send(source_key).is_ok())
            {
                self.pending_tile_loads.insert(source_key);
                requested_count += 1;
            }
        }
    }

    /// Which resident tile currently backs each block of the window.
    ///
    /// Returns `None` if any block has no resident source at all, in which case
    /// there is nothing to build yet and the caller keeps what it has.
    pub fn near_field_sources(&self, key: NearFieldKey) -> Option<NearFieldSources> {
        let TerrainDataSource::Outmap(outmap) = &self.source else {
            return None;
        };
        let mut source_keys =
            Vec::with_capacity((NEAR_FIELD_WINDOW_TILES * NEAR_FIELD_WINDOW_TILES) as usize);
        for block_y in 0..NEAR_FIELD_WINDOW_TILES {
            for block_x in 0..NEAR_FIELD_WINDOW_TILES {
                let requested = TileKey {
                    face: key.face,
                    level: key.level,
                    x: key.tile_x + block_x,
                    y: key.tile_y + block_y,
                };
                let preferred = outmap.resolve_tile(requested).ok()?;
                let source_key = cached_tile_ancestor(requested, preferred, &self.tile_cache)?;
                // Keep the block even when it currently resolves to the dense
                // pyramid. Other blocks in the same view may already have
                // sparse detail, and the source-level channel prevents this
                // resampled ancestor from pretending to be requested-level
                // data in the runtime relief filter.
                source_keys.push(source_key);
            }
        }
        Some(NearFieldSources { key, source_keys })
    }

    pub fn near_field_coverage(&self, key: NearFieldKey) -> Option<NearFieldCoverage> {
        let TerrainDataSource::Outmap(outmap) = &self.source else {
            return None;
        };
        let dense_level = outmap.manifest().dense_level;
        let mut resident_blocks = 0;
        let mut finer_than_dense_blocks = 0;
        let mut minimum_source_level: Option<u8> = None;
        let mut maximum_source_level: Option<u8> = None;
        for block_y in 0..NEAR_FIELD_WINDOW_TILES {
            for block_x in 0..NEAR_FIELD_WINDOW_TILES {
                let requested = TileKey {
                    face: key.face,
                    level: key.level,
                    x: key.tile_x + block_x,
                    y: key.tile_y + block_y,
                };
                let Ok(preferred) = outmap.resolve_tile(requested) else {
                    continue;
                };
                let Some(source_key) = cached_tile_ancestor(requested, preferred, &self.tile_cache)
                else {
                    continue;
                };
                resident_blocks += 1;
                finer_than_dense_blocks += u32::from(source_key.level > dense_level);
                minimum_source_level = Some(
                    minimum_source_level
                        .map_or(source_key.level, |level| level.min(source_key.level)),
                );
                maximum_source_level = Some(
                    maximum_source_level
                        .map_or(source_key.level, |level| level.max(source_key.level)),
                );
            }
        }
        let total_blocks = NEAR_FIELD_WINDOW_TILES * NEAR_FIELD_WINDOW_TILES;
        Some(NearFieldCoverage {
            requested_face: key.face.index() as u8,
            requested_level: key.level,
            total_blocks,
            resident_blocks,
            finer_than_dense_blocks,
            minimum_source_level,
            maximum_source_level,
            window_eligible: resident_blocks == total_blocks,
            active_window_level: None,
        })
    }

    pub fn near_field_window(&self, sources: &NearFieldSources) -> Option<NearFieldWindow> {
        let key = sources.key;
        let samples = NEAR_FIELD_WINDOW_SAMPLES as usize;
        let logical = TILE_LOGICAL_SIZE as usize;
        let quads = logical - 1;
        let tiles_per_side = f64::from(1_u32 << key.level);
        let mut heights_meters = vec![0.0_f32; samples * samples];
        let mut biome_ids = vec![0_u8; samples * samples];
        let mut moisture = vec![0_u8; samples * samples];
        let mut max_height_meters = f32::NEG_INFINITY;
        for block_y in 0..NEAR_FIELD_WINDOW_TILES {
            for block_x in 0..NEAR_FIELD_WINDOW_TILES {
                let source_key =
                    sources.source_keys[(block_y * NEAR_FIELD_WINDOW_TILES + block_x) as usize];
                let tile = self.tile_cache.get(&source_key)?;
                for sample_y in 0..logical {
                    let face_v = ((f64::from(key.tile_y + block_y)
                        + sample_y as f64 / quads as f64)
                        / tiles_per_side)
                        * 2.0
                        - 1.0;
                    let row = (block_y as usize * quads + sample_y) * samples;
                    for sample_x in 0..logical {
                        let face_u = ((f64::from(key.tile_x + block_x)
                            + sample_x as f64 / quads as f64)
                            / tiles_per_side)
                            * 2.0
                            - 1.0;
                        // Not `source_tile_uv`: that rejects a coordinate
                        // sitting exactly on a tile's far edge, which is where
                        // every block boundary in this window lands whenever
                        // the source resolves at the requested level.
                        let uv = source_tile_local_uv(source_key, [face_u, face_v]);
                        let height = sample_height_cpu(&tile.heights_meters, uv);
                        let index = row + block_x as usize * quads + sample_x;
                        heights_meters[index] = height;
                        biome_ids[index] = sample_biome_cpu(&tile.biome_ids, uv);
                        moisture[index] = sample_moisture_cpu(&tile.moisture, uv);
                        max_height_meters = max_height_meters.max(height);
                    }
                }
            }
        }
        Some(NearFieldWindow {
            sources: sources.clone(),
            heights_meters,
            biome_ids,
            moisture,
            max_height_meters: max_height_meters.max(0.0),
        })
    }

    fn update_raster_near_field(&mut self, key: Option<NearFieldKey>) {
        let Some(key) = key else {
            self.raster_near_field = None;
            return;
        };
        let Some(sources) = self.near_field_sources(key) else {
            return;
        };
        if self.raster_near_field.as_ref() == Some(&sources) {
            return;
        }
        let Some(window) = self.near_field_window(&sources) else {
            return;
        };
        upload_near_field_texture(
            &self.queue,
            &self._raster_near_field_height_texture,
            bytemuck::cast_slice(&window.heights_meters),
            size_of::<f32>() as u32,
        );
        upload_near_field_texture(
            &self.queue,
            &self._raster_near_field_biome_texture,
            &window.biome_ids,
            size_of::<u8>() as u32,
        );
        upload_near_field_texture(
            &self.queue,
            &self._raster_near_field_moisture_texture,
            &window.moisture,
            size_of::<u8>() as u32,
        );
        self.raster_near_field = Some(sources);
    }

    fn raster_near_field_bounds(&self) -> Option<RasterNearFieldBounds> {
        let sources = self.raster_near_field.as_ref()?;
        let key = sources.key;
        let tiles_per_side = f64::from(1_u32 << key.level);
        Some(RasterNearFieldBounds {
            face: key.face.index() as u8,
            uv_min: [
                f64::from(key.tile_x) / tiles_per_side * 2.0 - 1.0,
                f64::from(key.tile_y) / tiles_per_side * 2.0 - 1.0,
            ],
            uv_span: f64::from(NEAR_FIELD_WINDOW_TILES) / tiles_per_side * 2.0,
        })
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
        // Retain the camera-centred key so raster can opportunistically prefetch
        // its sources after visible geometry has claimed the load queue. Outside
        // the sparse high-resolution corridor this resolves to the existing
        // dense ancestor and costs nothing.
        let raster_near_field_key =
            if camera_altitude_meters < LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS {
                let clearance_meters =
                    (camera_altitude_meters - distance_reference_height_meters).max(0.0);
                self.near_field_key(camera_world.normalize(), clearance_meters)
            } else {
                None
            };
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
                // The baked error limit is asked in both branches, and it is
                // not the same question as the split cap. Low flight
                // deliberately lets a node refine past its source so the
                // camera does not sit on huge facets while better tiles
                // stream; that bypass says the mesh may keep splitting, not
                // that the baked surface acquired detail it does not have.
                let baked_error_limit = BakedErrorLimit::new(outmap);
                if camera_altitude_meters < LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS {
                    self.lod.update_for_view_with_constraints(
                        camera_world,
                        camera_forward,
                        camera_up,
                        aspect_ratio,
                        viewport[1].max(1),
                        vertical_fov_radians,
                        OUTMAP_GEOMETRIC_ERROR,
                        &|_| MAX_LOD_LEVEL,
                        Some(&|node| baked_error_limit.level_limit(node)),
                    )
                } else {
                    self.lod.update_for_view_with_constraints(
                        camera_world,
                        camera_forward,
                        camera_up,
                        aspect_ratio,
                        viewport[1].max(1),
                        vertical_fov_radians,
                        OUTMAP_GEOMETRIC_ERROR,
                        &|node| outmap_node_level_limit(outmap, node),
                        Some(&|node| baked_error_limit.level_limit(node)),
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
                &self.terrain_tile_bind_group_layout,
                &label,
                &tile.heights_meters,
                &tile.biome_ids,
                &tile.moisture,
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
            if let Some(key) = raster_near_field_key {
                self.request_near_field_tiles_budget(key, MAX_RASTER_NEAR_FIELD_PREFETCH_PER_FRAME);
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
        self.update_raster_near_field(raster_near_field_key);

        let active_resolved_tiles: Vec<_> = render_nodes
            .iter()
            .zip(resolved_tiles.iter())
            .filter_map(|(render_node, resolved)| render_node.active.then_some(*resolved))
            .collect();
        let mut prepared_instances = Vec::with_capacity(render_nodes.len());
        self.draw_batches.clear();
        self.ocean_draw_batches.clear();
        self.surface_detail_nodes.clear();
        let mut fallback_chunks = 0_u32;
        let mut source_level_delta_histogram = [0_u32; MAX_LOD_LEVEL as usize + 1];
        let camera_view_basis = CameraViewBasis::from_forward_and_up(camera_forward, camera_up);
        let outmap_dense_level = match &self.source {
            TerrainDataSource::Placeholder => 0,
            TerrainDataSource::Outmap(outmap) => outmap.manifest().dense_level,
        };
        let raster_near_field_bounds = self.raster_near_field_bounds();
        for (render_node, resolved) in render_nodes.iter().zip(resolved_tiles.iter()) {
            let [u_min, v_min, u_max, v_max] = render_node.node.uv_bounds();
            let near_field = raster_near_field_bounds.is_some_and(|bounds| {
                render_node.node.face == bounds.face
                    && u_min >= bounds.uv_min[0]
                    && v_min >= bounds.uv_min[1]
                    && u_max <= bounds.uv_min[0] + bounds.uv_span
                    && v_max <= bounds.uv_min[1] + bounds.uv_span
            });
            let (source_uv_scale, source_uv_offset, source_level, tile_key, outmap_mode) =
                if let Some((requested_key, source_key)) = *resolved {
                    let (scale, offset) = fallback_uv_transform(requested_key, source_key);
                    if render_node.active {
                        fallback_chunks += u32::from(requested_key != source_key);
                        source_level_delta_histogram
                            [(requested_key.level - source_key.level) as usize] += 1;
                    }
                    if near_field {
                        let bounds = raster_near_field_bounds.expect("near-field bounds");
                        let window = self.raster_near_field.as_ref().expect("near-field sources");
                        (
                            [
                                ((u_max - u_min) / bounds.uv_span) as f32,
                                ((v_max - v_min) / bounds.uv_span) as f32,
                            ],
                            [
                                ((u_min - bounds.uv_min[0]) / bounds.uv_span) as f32,
                                ((v_min - bounds.uv_min[1]) / bounds.uv_span) as f32,
                            ],
                            window.key.level,
                            None,
                            true,
                        )
                    } else {
                        (scale, offset, source_key.level, Some(source_key), true)
                    }
                } else {
                    ([1.0, 1.0], [0.0, 0.0], render_node.node.level, None, false)
                };
            let anchor_direction = render_node.node.center_direction().as_vec3().normalize();
            let anchor_world = DVec3::new(
                f64::from(anchor_direction.x),
                f64::from(anchor_direction.y),
                f64::from(anchor_direction.z),
            ) * PLANET_RADIUS_METERS;
            let anchor_u = (u_min + u_max) * 0.5;
            let anchor_v = (v_min + v_max) * 0.5;
            let edge_stitch = if render_node.active {
                edge_stitch_info(render_node.node, &active_render_nodes)
            } else {
                0
            };
            self.surface_detail_nodes.push(SurfaceDetailNode {
                node: render_node.node,
                edge_stitch,
                source_key: tile_key,
            });
            let may_contain_ocean = match tile_key {
                Some(key) => {
                    let tile = self
                        .tile_cache
                        .get(&key)
                        .expect("resolved terrain tile is resident");
                    let footprint_is_land =
                        if source_uv_scale == [1.0, 1.0] && source_uv_offset == [0.0, 0.0] {
                            tile.complete_logical_footprint_is_land
                        } else {
                            height_footprint_is_strictly_land(
                                &tile.heights_meters,
                                source_uv_scale,
                                source_uv_offset,
                            )
                        };
                    !footprint_is_land
                }
                // Placeholder height is procedural rather than represented by
                // the zero-filled texture, so it cannot be culled from tile
                // data.
                None => true,
            };
            prepared_instances.push((
                if near_field { None } else { tile_key },
                near_field,
                !self.flat_triangle_experiment
                    && near_field
                    && render_node.node.level <= NEAR_FIELD_DENSE_MAX_LEVEL,
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
                        outmap_mode
                            && node_intersects_source_edge_fade(
                                render_node.node,
                                source_level,
                                outmap_dense_level,
                            ),
                        near_field,
                    ),
                    lod_transition: [
                        render_node.transition_progress,
                        if render_node.transition_incoming {
                            1.0
                        } else {
                            0.0
                        },
                    ],
                    edge_stitch,
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
                may_contain_ocean,
            ));
        }
        // A single canonical vertex buffer makes leaves with the same source
        // tile genuinely instanced. Global L4 fallback therefore costs a few
        // draw calls rather than one call and one vertex buffer per leaf.
        // Within a resolved tile, put possible-ocean chunks first. Their
        // ranges are then contiguous subsets of the terrain instance stream,
        // so culling does not duplicate instance uploads.
        prepared_instances.sort_unstable_by_key(
            |(tile_key, near_field, dense_near_field, _, may_contain_ocean)| {
                (
                    *near_field,
                    *dense_near_field,
                    *tile_key,
                    !*may_contain_ocean,
                )
            },
        );
        let mut instances = Vec::with_capacity(prepared_instances.len());
        for (tile_key, near_field, dense_near_field, instance, may_contain_ocean) in
            prepared_instances
        {
            let instance_index = instances.len() as u32;
            push_draw_batch_instance(
                &mut self.draw_batches,
                tile_key,
                near_field,
                dense_near_field,
                instance_index,
            );
            if may_contain_ocean {
                push_draw_batch_instance(
                    &mut self.ocean_draw_batches,
                    tile_key,
                    near_field,
                    false,
                    instance_index,
                );
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
        let ocean_chunks = self
            .ocean_draw_batches
            .iter()
            .map(|batch| batch.instance_count)
            .sum();
        let terrain_triangles = self
            .draw_batches
            .iter()
            .map(|batch| {
                let index_count = if batch.dense_near_field {
                    self.near_field_index_count
                } else {
                    self.index_count
                };
                u64::from(batch.instance_count) * u64::from(index_count / 3)
            })
            .sum();
        let ocean_triangles = self
            .ocean_draw_batches
            .iter()
            .map(|batch| {
                let index_count = if batch.dense_near_field {
                    self.near_field_index_count
                } else {
                    self.index_count
                };
                u64::from(batch.instance_count) * u64::from(index_count / 3)
            })
            .sum();
        Ok(TerrainStats {
            level_histogram: metrics.level_histogram,
            resident_chunks: metrics.active_chunks,
            drawn_chunks: render_nodes.len() as u32,
            terrain_triangles,
            ocean_chunks,
            ocean_triangles,
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
            draw_calls: (self.draw_batches.len() + self.ocean_draw_batches.len()) as u32,
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
        let ocean_pipeline =
            if self.fading_out_chunks.is_empty() && self.fade_in_started_at.is_empty() {
                &self.ocean_stable_pipeline
            } else {
                &self.ocean_transition_pipeline
            };
        // Draw the analytic shell first. With reversed-Z, raised terrain then
        // writes a strictly greater depth and wins even when a mixed
        // coastline triangle has nearly identical far-plane depth.
        render_pass.set_pipeline(ocean_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(2, &self.shared_bind_group, &[]);
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        for batch in &self.ocean_draw_batches {
            let (vertex_buffer, index_buffer, index_count) = if batch.dense_near_field {
                (
                    &self.near_field_vertex_buffer,
                    &self.near_field_index_buffer,
                    self.near_field_index_count,
                )
            } else {
                (
                    &self.chunk_vertex_buffer,
                    &self.index_buffer,
                    self.index_count,
                )
            };
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            let bind_group = if batch.near_field {
                &self.raster_near_field_bind_group
            } else {
                let tile = batch.tile_key.map_or(&self.placeholder_tile, |key| {
                    self.tile_cache
                        .get(&key)
                        .expect("draw batch has a resident terrain tile")
                });
                &tile.bind_group
            };
            render_pass.set_bind_group(1, bind_group, &[]);
            render_pass.draw_indexed(
                0..index_count,
                0,
                batch.first_instance..batch.first_instance + batch.instance_count,
            );
        }
        render_pass.set_pipeline(pipeline);
        for batch in &self.draw_batches {
            let (vertex_buffer, index_buffer, index_count) = if batch.dense_near_field {
                (
                    &self.near_field_vertex_buffer,
                    &self.near_field_index_buffer,
                    self.near_field_index_count,
                )
            } else {
                (
                    &self.chunk_vertex_buffer,
                    &self.index_buffer,
                    self.index_count,
                )
            };
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            let bind_group = if batch.near_field {
                &self.raster_near_field_bind_group
            } else {
                let tile = batch.tile_key.map_or(&self.placeholder_tile, |key| {
                    self.tile_cache
                        .get(&key)
                        .expect("draw batch has a resident terrain tile")
                });
                &tile.bind_group
            };
            render_pass.set_bind_group(1, bind_group, &[]);
            render_pass.draw_indexed(
                0..index_count,
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

pub fn create_shared_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shared planet bind group layout"),
        entries: &[
            cube_texture_layout_entry(3),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                // Terrain displacement reads these scales in the vertex
                // stage; raymarching and lake shading use the same values.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture_array_layout_entry(6, wgpu::TextureSampleType::Float { filterable: true }),
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
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
                // Keep the full delta for the displacement filter even though
                // the position stitcher below caps its grid collapse at two
                // levels. Mountain-scale runtime relief otherwise evaluates
                // at two different filters on the same shared edge and opens
                // holes hundreds of metres high under a budget-limited LOD
                // frontier. Five bits cover the complete L0-L18 range.
                maximum_delta = maximum_delta.max(node.level - neighbor.level);
            }
        }
        packed |= u32::from(maximum_delta) << (edge * 5);
    }
    packed
}

fn active_node_at_direction(
    active_nodes: &[QuadtreeNode],
    direction: DVec3,
) -> Option<QuadtreeNode> {
    let (face, face_uv) = cube_face_uv(direction)?;
    active_nodes
        .iter()
        .copied()
        .find(|node| node_contains_face_uv(*node, face, face_uv))
}

fn edge_stitch_level_delta(packed: u32, edge: u32) -> u8 {
    ((packed >> (edge * 5)) & 0x1f) as u8
}

fn node_contains_face_uv(node: QuadtreeNode, face: CubeFace, face_uv: [f64; 2]) -> bool {
    let Some(node_face) = CubeFace::from_index(node.face) else {
        return false;
    };
    source_tile_uv(
        TileKey {
            face: node_face,
            level: node.level,
            x: node.x,
            y: node.y,
        },
        face,
        face_uv,
    )
    .is_some()
}

fn surface_detail_filter_meters(
    surface: SurfaceDetailNode,
    face_uv: [f64; 2],
    camera_distance_meters: f64,
) -> f64 {
    let node_spacing = 2.0 * PLANET_RADIUS_METERS
        / (f64::from(1_u32 << surface.node.level) * f64::from(CHUNK_GRID_QUADS as u32));
    let [u_min, v_min, u_max, v_max] = surface.node.uv_bounds();
    let tile_uv = [
        ((face_uv[0] - u_min) / (u_max - u_min)).clamp(0.0, 1.0),
        ((face_uv[1] - v_min) / (v_max - v_min)).clamp(0.0, 1.0),
    ];
    let edge_distances = [tile_uv[1], 1.0 - tile_uv[0], 1.0 - tile_uv[1], tile_uv[0]];
    let mut filter_meters = node_spacing;
    for (edge, edge_distance) in edge_distances.into_iter().enumerate() {
        let level_delta = edge_stitch_level_delta(surface.edge_stitch, edge as u32);
        if level_delta == 0 {
            continue;
        }
        let neighbor_level = surface.node.level.saturating_sub(level_delta);
        let neighbor_spacing = 2.0 * PLANET_RADIUS_METERS
            / (f64::from(1_u32 << neighbor_level) * f64::from(CHUNK_GRID_QUADS as u32));
        let fade_width =
            (f64::from(1_u32 << level_delta) / f64::from(CHUNK_GRID_QUADS as u32)).min(1.0);
        let edge_weight = 1.0 - smoothstep_f64(0.0, fade_width, edge_distance);
        filter_meters =
            filter_meters.max(node_spacing + (neighbor_spacing - node_spacing) * edge_weight);
    }
    filter_meters.max(
        (camera_distance_meters * TERRAIN_DETAIL_FILTER_RATIO)
            .max(TERRAIN_DETAIL_MIN_FILTER_METERS),
    )
}

fn smoothstep_f64(edge0: f64, edge1: f64, value: f64) -> f64 {
    let amount = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    amount * amount * (3.0 - 2.0 * amount)
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
    let complete_logical_footprint_is_land =
        height_footprint_is_strictly_land(heights_meters, [1.0, 1.0], [0.0, 0.0]);
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
        ],
    });
    GpuTile {
        _height_texture: height_texture,
        _biome_texture: biome_texture,
        _moisture_texture: moisture_texture,
        bind_group,
        heights_meters: heights_meters.to_vec(),
        biome_ids: biome_ids.to_vec(),
        moisture: moisture.to_vec(),
        complete_logical_footprint_is_land,
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

fn create_near_field_texture(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: NEAR_FIELD_WINDOW_SAMPLES,
            height: NEAR_FIELD_WINDOW_SAMPLES,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload_near_field_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    bytes: &[u8],
    bytes_per_texel: u32,
) {
    let extent = NEAR_FIELD_WINDOW_SAMPLES;
    let padded = padded_texture_rows(bytes, extent, extent, bytes_per_texel);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
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

/// Answers "how deep can baked data still resolve anything here" for every
/// node the selector evaluates, which is thousands per update.
///
/// `resolve_tile` walks from the requested key to the best available ancestor
/// and each step is a binary search over the manifest's tile list, so a node
/// ten levels below its source pays ten of them. Recording every key the walk
/// passes through collapses that: a sibling checks itself, hits the shared
/// parent in the map, and stops. Nothing is assumed about the tile pyramid --
/// the walk still tests each key itself before consulting an ancestor's entry,
/// so a lone tile with no parent would still be found.
struct BakedErrorLimit<'a> {
    outmap: &'a Outmap,
    source_levels: RefCell<HashMap<TileKey, u8>>,
}

impl<'a> BakedErrorLimit<'a> {
    fn new(outmap: &'a Outmap) -> Self {
        Self {
            outmap,
            source_levels: RefCell::new(HashMap::new()),
        }
    }

    fn source_level(&self, requested: TileKey) -> u8 {
        let mut source_levels = self.source_levels.borrow_mut();
        let mut walked = Vec::new();
        let mut key = requested;
        let source_level = loop {
            if let Some(&level) = source_levels.get(&key) {
                break level;
            }
            if self.outmap.manifest().has_tile(key) {
                walked.push(key);
                break key.level;
            }
            walked.push(key);
            match key.parent() {
                Some(parent) => key = parent,
                // A validated outmap has a root tile for every face, so this is
                // unreachable; level 0 is the honest answer if it ever is not.
                None => break 0,
            }
        };
        for key in walked {
            source_levels.insert(key, source_level);
        }
        source_level
    }

    /// The level past which splitting reads no new source texels. Two extra
    /// quadtree levels consume one tile's samples, which is the same bound
    /// `outmap_node_level_limit` enforces when it is enforced at all.
    fn level_limit(&self, node: QuadtreeNode) -> u8 {
        let Ok(requested) = tile_key(node) else {
            return MAX_LOD_LEVEL;
        };
        self.source_level(requested)
            .saturating_add(OUTMAP_TILE_GRID_SUBDIVISION_LEVELS)
            .min(MAX_LOD_LEVEL)
    }
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

/// Where a face coordinate falls inside a tile, clamped rather than rejected.
///
/// The near-field window walks tile blocks and needs the shared edge sample at
/// each boundary; `source_tile_uv` treats that exact coordinate as outside.
fn source_tile_local_uv(key: TileKey, face_uv: [f64; 2]) -> [f32; 2] {
    let tiles_per_side = f64::from(1_u64.wrapping_shl(u32::from(key.level)) as u32);
    [
        (((face_uv[0] + 1.0) * 0.5 * tiles_per_side) - f64::from(key.x)).clamp(0.0, 1.0) as f32,
        (((face_uv[1] + 1.0) * 0.5 * tiles_per_side) - f64::from(key.y)).clamp(0.0, 1.0) as f32,
    ]
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

#[cfg(test)]
pub fn cube_face_uv_for_survey(direction: DVec3) -> Option<(CubeFace, [f64; 2])> {
    cube_face_uv(direction)
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

fn node_intersects_source_edge_fade(node: QuadtreeNode, source_level: u8, dense_level: u8) -> bool {
    if source_level <= dense_level {
        return false;
    }
    let [u_min, v_min, u_max, v_max] = node.uv_bounds();
    let fade =
        crate::planet::TERRAIN_DETAIL_SOURCE_EDGE_FADE_TEXELS / f64::from(TILE_LOGICAL_SIZE - 1);
    for level in dense_level.saturating_add(1)..=source_level {
        let scale = f64::from(1_u32 << level) * 0.5;
        let overlaps_edge = |minimum: f64, maximum: f64| {
            let tile_minimum = (minimum + 1.0) * scale;
            let tile_maximum = (maximum + 1.0) * scale;
            (tile_minimum - fade).ceil() <= tile_maximum + fade
        };
        if overlaps_edge(u_min, u_max) || overlaps_edge(v_min, v_max) {
            return true;
        }
    }
    false
}

fn pack_terrain_info(
    outmap: bool,
    face: u8,
    requested_level: u8,
    source_level: u8,
    source_edge_fade: bool,
    near_field: bool,
) -> u32 {
    u32::from(outmap)
        | (u32::from(face) << 1)
        | (u32::from(requested_level) << 4)
        | (u32::from(source_level) << 9)
        | u32::from(source_edge_fade) * TERRAIN_INFO_SOURCE_EDGE_FADE_BIT
        | u32::from(near_field) * TERRAIN_INFO_NEAR_FIELD_BIT
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
    let coordinate = tile_sample_coordinate(uv);
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

fn tile_sample_coordinate(uv: [f32; 2]) -> [f32; 2] {
    [
        TILE_GUTTER as f32 + uv[0].clamp(0.0, 1.0) * (TILE_LOGICAL_SIZE - 1) as f32,
        TILE_GUTTER as f32 + uv[1].clamp(0.0, 1.0) * (TILE_LOGICAL_SIZE - 1) as f32,
    ]
}

fn sample_biome_cpu(biome_ids: &[u8], uv: [f32; 2]) -> u8 {
    let coordinate = tile_sample_coordinate(uv);
    let x = coordinate[0].round() as usize;
    let y = coordinate[1].round() as usize;
    biome_ids[y * TILE_STORED_SIZE as usize + x]
}

fn sample_moisture_cpu(moisture: &[u8], uv: [f32; 2]) -> u8 {
    let coordinate = tile_sample_coordinate(uv);
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
    let lower_value = f32::from(moisture[index(lower[0], lower[1])])
        + (f32::from(moisture[index(upper[0], lower[1])])
            - f32::from(moisture[index(lower[0], lower[1])]))
            * amount[0];
    let upper_value = f32::from(moisture[index(lower[0], upper[1])])
        + (f32::from(moisture[index(upper[0], upper[1])])
            - f32::from(moisture[index(lower[0], upper[1])]))
            * amount[0];
    (lower_value + (upper_value - lower_value) * amount[1])
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Proves that every height texel which can contribute to bilinear sampling
/// over a resolved source rectangle lies strictly above sea level.
///
/// Returning false is deliberately conservative: zero, negative, malformed,
/// or invalid bounds retain the ocean shell and let the fragment ownership
/// test make the final decision.
fn height_footprint_is_strictly_land(
    heights: &[f32],
    source_uv_scale: [f32; 2],
    source_uv_offset: [f32; 2],
) -> bool {
    if heights.len() != tile_sample_count() {
        return false;
    }
    let mut bounds = [[0_usize; 2]; 2];
    for axis in 0..2 {
        let minimum_uv = source_uv_offset[axis];
        let maximum_uv = minimum_uv + source_uv_scale[axis];
        if !minimum_uv.is_finite()
            || !maximum_uv.is_finite()
            || minimum_uv < 0.0
            || maximum_uv > 1.0
            || minimum_uv > maximum_uv
        {
            return false;
        }
        let minimum_coordinate = TILE_GUTTER as f32 + minimum_uv * (TILE_LOGICAL_SIZE - 1) as f32;
        let maximum_coordinate = TILE_GUTTER as f32 + maximum_uv * (TILE_LOGICAL_SIZE - 1) as f32;
        bounds[axis] = [
            minimum_coordinate.floor() as usize,
            maximum_coordinate.ceil() as usize,
        ];
    }
    let width = TILE_STORED_SIZE as usize;
    (bounds[1][0]..=bounds[1][1]).all(|y| {
        (bounds[0][0]..=bounds[0][1]).all(|x| {
            heights
                .get(y * width + x)
                .is_some_and(|height| height.is_finite() && *height > 0.0)
        })
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use glam::DVec3;

    use super::{
        FadingChunk, LOW_FLIGHT_SOURCE_LIMIT_BYPASS_ALTITUDE_METERS,
        OUTMAP_TILE_GRID_SUBDIVISION_LEVELS, SurfaceDetailNode, TERRAIN_INFO_NEAR_FIELD_BIT,
        TERRAIN_INFO_SOURCE_EDGE_FADE_BIT, TERRAIN_MATERIAL_LAYER_COUNT,
        TERRAIN_MATERIAL_TEXTURE_SIZE, TerrainSettings, aligned_texture_row_bytes,
        conservative_outmap_height_bounds, cube_face_uv, downsample_srgb_rgba8, edge_stitch_info,
        edge_stitch_level_delta, fallback_uv_transform, height_footprint_is_strictly_land,
        lod_transition_nodes, lod_transition_progress, node_intersects_source_edge_fade,
        nodes_share_lod_transition, pack_terrain_info, padded_texture_rows, planet_shader_source,
        purge_expired_lod_transitions, sample_biome_cpu, sample_height_cpu, sample_moisture_cpu,
        should_animate_lod_transition, source_tile_uv_at_direction, surface_detail_filter_meters,
        terrain_material_layer_texels, terrain_material_texel, tileable_value_noise,
    };
    use crate::planet::{
        GLOBAL_TERRAIN_DETAIL_HEIGHT_SCALE, MAX_LOD_LEVEL, OUTMAP_TERRAIN_FAR_HEIGHT_SCALE,
        OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE, PLANET_RADIUS_METERS, PlanetLod, QuadtreeNode,
        TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS, build_chunk_mesh, cube_face_direction,
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
        let packed = pack_terrain_info(true, 5, 18, 7, true, false);
        assert_eq!(packed & 1, 1);
        assert_eq!((packed >> 1) & 0x7, 5);
        assert_eq!((packed >> 4) & 0x1f, 18);
        assert_eq!((packed >> 9) & 0x1f, 7);
        assert_ne!(packed & TERRAIN_INFO_SOURCE_EDGE_FADE_BIT, 0);
        assert_eq!(packed & TERRAIN_INFO_NEAR_FIELD_BIT, 0);
        let near_field = pack_terrain_info(true, 5, 18, 18, false, true);
        assert_ne!(near_field & TERRAIN_INFO_NEAR_FIELD_BIT, 0);
    }

    #[test]
    fn only_chunks_near_sparse_source_borders_enable_the_vertex_fade() {
        let border = QuadtreeNode {
            face: 0,
            level: 14,
            x: 8_064,
            y: 7_000,
        };
        let interior = QuadtreeNode { x: 8_070, ..border };
        assert!(node_intersects_source_edge_fade(border, 7, 4));
        assert!(!node_intersects_source_edge_fade(interior, 7, 4));
        assert!(!node_intersects_source_edge_fade(border, 4, 4));
    }

    #[test]
    fn shader_reads_outmap_height_scale_from_terrain_settings() {
        let settings = TerrainSettings::from_planet_constants(4);
        let shader = planet_shader_source();
        assert_eq!(OUTMAP_TERRAIN_NEAR_HEIGHT_SCALE, 4.0);
        assert_eq!(OUTMAP_TERRAIN_FAR_HEIGHT_SCALE, 4.0);
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
        assert_eq!(settings.outmap_detail[0], 4.0);
        assert!(shader.contains("fn scaled_terrain_macro_height("));
        assert!(shader.matches("scaled_terrain_macro_height(").count() >= 3);
        let normal = shader
            .split("fn displaced_surface_normal(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("raster terrain normal is present");
        assert_eq!(
            normal.matches("terrain_height(").count(),
            4,
            "every raster normal probe must pass through scaled macro height"
        );
    }

    #[test]
    fn shader_uses_baked_displacement_and_real_light() {
        let shader = planet_shader_source();
        // Bound this at the next top-level item. Splitting on a named function
        // is unreliable here: the shared shader is concatenated ahead of this
        // one, so anything named in it has already been passed and the slice
        // silently ran to end of file.
        let terrain_height = shader
            .split("fn terrain_height(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("terrain height function is present");
        assert!(terrain_height.contains("scaled_terrain_macro_height(macro_height)"));
        // terrain_height stays macro-only on purpose: detail is added once in
        // vs_main with an analytic slope, so the four normal probes remain pure
        // texture reads rather than each re-running the octave ladder.
        assert!(!terrain_height.contains("terrain_detail("));
        assert!(!shader.contains("requested_lod_level: f32"));
        assert!(shader.contains("biome_color(2u) * 0.65 * ice_light_floor"));
        assert!(!shader.contains("max(lit_surface_color, biome_color(2u) * 0.65)"));
    }

    #[test]
    fn raster_land_uses_smoothed_displaced_normals_for_close_snow() {
        let shader = planet_shader_source();
        assert!(!shader.contains("fn flat_terrain_normal("));

        let fragment = shader
            .split("fn terrain_fragment_color(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("raster terrain fragment path is present");
        assert!(fragment.contains("let terrain_normal = input.world_normal;"));
        assert!(fragment.contains("terrain_normal,\n        direction,"));
        assert!(fragment.contains("let terrain_surface_irradiance = terrain_sky_diffuse"));
    }

    #[test]
    fn terrain_fragment_keeps_positive_interpolated_land_over_mixed_ocean_samples() {
        let shader = planet_shader_source();
        let fragment = shader
            .split("fn terrain_fragment_color(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("raster terrain fragment path is present");
        assert!(fragment.contains(
            "if is_open_ocean_surface(outmap, macro_height_meters, biome_id)\n        && input.surface_height <= 0.0"
        ));
        let ocean = shader
            .split("fn ocean_fragment_color(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("analytic ocean fragment path is present");
        assert!(ocean.contains("if input.terrain_height_hint > 0.0"));
    }

    #[test]
    fn flat_triangle_experiment_uses_categorical_fill_and_edges() {
        let shader = planet_shader_source();
        assert!(shader.contains("const RENDER_DEBUG_FLAT_TRIANGLES: u32 = 6u;"));
        assert!(shader.contains("fn flat_triangle_edge("));
        assert!(shader.contains("fn flat_triangle_colour("));
        assert!(shader.contains("return flat_triangle_colour(input);"));
        assert!(shader.contains("return flat_ocean_colour(input);"));
        // The experiment intentionally bypasses the material texture stack;
        // biome palette ownership remains the only fill colour source.
        let flat = shader
            .split("fn flat_triangle_colour(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("flat triangle colour path is present");
        assert!(flat.contains("biome_color(biome_id)"));
        assert!(!flat.contains("terrain_material_color("));
    }

    #[test]
    fn planet_shader_validates_with_filtered_runtime_detail_noise() {
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
        assert!(shader.contains("fn terrain_detail_value_noise("));
        assert!(shader.contains("fn continuous_baked_sample_spacing_meters("));
        // Every octave has to be faded against the sampling spacing. Without
        // this the detail aliases into crawling noise as the camera moves.
        let detail = shader
            .split("fn terrain_detail_band(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("detail function is present");
        assert!(detail.contains("smoothstep(filter_meters"));
    }

    #[test]
    fn shader_skips_surface_work_that_cannot_affect_the_image() {
        let shader = planet_shader_source();
        let detail = shader
            .split("fn terrain_detail_band(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("detail function is present");
        let ocean_bailout = detail
            .find("if scaled_macro_height_meters <= 0.0")
            .expect("non-positive terrain exits before detail noise");
        let noise_domain = detail
            .find("let anchor_domain = terrain_detail_domain(anchor_direction);")
            .expect("detail noise domain is present");
        assert!(ocean_bailout < noise_domain);

        let material = shader
            .split("fn terrain_material_tint(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("material tint function is present");
        let distance_bailout = material
            .find("if fade <= 0.0")
            .expect("distant material exits before texture work");
        let fine_coordinate = material
            .find("fine_position = terrain_material_fine_position(")
            .expect("close material coordinate is present");
        assert!(distance_bailout < fine_coordinate);
        assert!(material.contains("if fine_weight > 0.0"));
    }

    #[test]
    fn raster_ocean_uses_a_separate_analytic_shell() {
        let shader = planet_shader_source();
        let terrain_vertex = shader
            .split("fn vs_main(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("terrain vertex function is present");
        assert!(
            !terrain_vertex.contains("ocean_surface("),
            "terrain vertices must stay on land/bathymetry geometry"
        );
        assert!(
            shader.contains("fn vs_ocean("),
            "the raster ocean needs independent sea-shell geometry"
        );
        assert!(
            shader.contains("fn fs_ocean_stable("),
            "the raster ocean needs an independently depth-tested fragment stage"
        );
        let terrain_fragment = shader
            .split("fn terrain_fragment_color(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("terrain fragment function is present");
        let ocean_fragment = shader
            .split("fn ocean_fragment_color(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("ocean fragment function is present");
        assert!(terrain_fragment.contains(
            "if is_open_ocean_surface(outmap, macro_height_meters, biome_id)\n        && input.surface_height <= 0.0"
        ));
        assert!(ocean_fragment.contains(
            "if !is_open_ocean_surface(outmap, macro_height_meters, biome_id) {\n        discard;"
        ));
    }

    /// The LOD selector's error budget has to know about the synthesised
    /// ladder, or it caps how steep the terrain may be without anyone saying
    /// so. This is the arithmetic that connects them.
    #[test]
    fn geometric_error_budget_tracks_the_detail_ladder() {
        use super::{
            LADDER_GEOMETRIC_ERROR_PER_ROUGHNESS, OUTMAP_BAKED_GEOMETRIC_ERROR_RATIO,
            OUTMAP_GEOMETRIC_ERROR_RATIO,
        };
        // Error is `pi/4 * ratio` of a vertex spacing, and the ladder drops
        // octaves shorter than twice that spacing, whose RMS is
        // `ROUGHNESS * 2 * sqrt(4/3)` of it.
        let expected = 2.0 * (4.0_f64 / 3.0).sqrt() / (std::f64::consts::PI / 4.0);
        assert!(
            (LADDER_GEOMETRIC_ERROR_PER_ROUGHNESS - expected).abs() < 1.0e-3,
            "ladder error factor {LADDER_GEOMETRIC_ERROR_PER_ROUGHNESS} should be {expected}"
        );

        let at = |roughness: f64| {
            OUTMAP_BAKED_GEOMETRIC_ERROR_RATIO + roughness * LADDER_GEOMETRIC_ERROR_PER_ROUGHNESS
        };
        // The budget must cover what the ladder needs at *whatever* roughness
        // is set, which is the whole point of deriving it. Anything less and
        // the selector under-tessellates without saying so, which is what the
        // stair-stepped silhouettes at 0.06 were.
        for roughness in [0.0328_f64, 0.06, 0.10, 0.15] {
            let needed = roughness * expected;
            assert!(
                at(roughness) >= needed,
                "at roughness {roughness} the ladder needs {needed} and the budget gives {}",
                at(roughness)
            );
        }
        assert!(at(0.06) > at(0.0328));
        // 0.15 at roughness 0.0328 is the calibration point: that budget was in
        // use and measured good, and the baked term is back-calculated from it.
        assert!(
            (at(0.0328) - 0.15).abs() < 0.002,
            "calibration point moved to {}",
            at(0.0328)
        );
        // And it has to be the budget the selector is actually using, which it
        // now carries in two parts.
        assert_eq!(
            OUTMAP_GEOMETRIC_ERROR_RATIO,
            at(crate::planet::TERRAIN_DETAIL_ROUGHNESS)
        );
        assert_eq!(
            super::OUTMAP_GEOMETRIC_ERROR.total(),
            OUTMAP_GEOMETRIC_ERROR_RATIO
        );
        assert_eq!(
            super::OUTMAP_GEOMETRIC_ERROR.baked,
            OUTMAP_BAKED_GEOMETRIC_ERROR_RATIO
        );
        // The ladder alone must still drive refinement, because past the baked
        // limit it is the entire budget. A zero there stops the mesh dead at
        // the source level and gives back the facets the ladder exists to hide.
        assert!(super::OUTMAP_GEOMETRIC_ERROR.ladder > 0.0);
    }

    /// The baked term is charged only where a split can still read a source
    /// texel it has not read. Past that the macro surface is fully resolved and
    /// further refinement returns the same bilinear patch, so charging for it
    /// buys geometry that provably cannot differ from its parent's.
    #[test]
    fn the_baked_error_term_stops_at_the_source_limit() {
        use crate::planet::GeometricErrorRatio;

        let error = super::OUTMAP_GEOMETRIC_ERROR;
        assert_eq!(error.for_node_for_test(true), error.total());
        assert_eq!(error.for_node_for_test(false), error.ladder);
        assert!(error.for_node_for_test(false) < error.for_node_for_test(true));

        // The saving is a property of the two terms, not a tuned number: split
        // distance scales with the ratio and chunk count with its square.
        let demand_scale = (error.ladder / error.total()).powi(2);
        assert!(
            (0.55..0.62).contains(&demand_scale),
            "dropping the baked term should leave ~0.59 of the chunk demand, got {demand_scale}"
        );

        // The placeholder terrain has no baked source to exhaust, so it must
        // never lose its budget: it passes no limit and stays uniform.
        let placeholder =
            GeometricErrorRatio::uniform(crate::planet::PLACEHOLDER_GEOMETRIC_ERROR_RATIO);
        assert_eq!(
            placeholder.for_node_for_test(true),
            crate::planet::PLACEHOLDER_GEOMETRIC_ERROR_RATIO
        );
    }

    /// The window has to be finer than the pyramid the raymarch path already
    /// holds, or it is a slower copy of data that is already bound; and it has
    /// to stay wide enough to cover what the camera can see.
    #[test]
    fn near_field_window_level_tracks_what_the_camera_can_see() {
        use super::{
            CUBE_FACE_ARC_METERS, NEAR_FIELD_MIN_EXTENT_METERS, NEAR_FIELD_WINDOW_TILES,
            near_field_window_level,
        };
        let extent = |level: u8| {
            f64::from(NEAR_FIELD_WINDOW_TILES) / f64::from(1_u32 << level) * CUBE_FACE_ARC_METERS
        };

        // Standing on the ground, the window must still reach past the horizon,
        // which is sqrt(2 * R * h) -- about 4km at 2m of eye height.
        let ground = near_field_window_level(2.0, 4, 18).expect("ground gets a window");
        assert!(
            extent(ground) >= NEAR_FIELD_MIN_EXTENT_METERS,
            "level {ground} covers only {}m",
            extent(ground)
        );
        assert!(extent(ground) < NEAR_FIELD_MIN_EXTENT_METERS * 2.0);

        // Climbing widens it, and never narrows it.
        let mut previous = ground;
        for clearance in [100.0, 1_000.0, 10_000.0, 100_000.0] {
            let Some(level) = near_field_window_level(clearance, 4, 18) else {
                continue;
            };
            assert!(
                level <= previous,
                "window narrowed climbing to {clearance}m"
            );
            assert!(
                extent(level) >= clearance * 20.0,
                "at {clearance}m the window covers only {}m",
                extent(level)
            );
            previous = level;
        }

        // From orbit it would be no finer than the dense pyramid, so it is not
        // worth building at all.
        assert_eq!(near_field_window_level(4_000_000.0, 4, 18), None);
        // And a dense pyramid that already reached this fine leaves nothing to add.
        assert_eq!(near_field_window_level(2.0, 18, 18), None);
    }

    /// The window is addressed by whole tiles, so it must never hang over a
    /// face edge: cube faces do not share a tile grid, and the missing blocks
    /// would have no source.
    #[test]
    fn near_field_window_stays_inside_its_cube_face() {
        use super::{NEAR_FIELD_WINDOW_TILES, near_field_window_level};
        let level = near_field_window_level(2.0, 4, 18).expect("ground gets a window");
        let tiles_per_side = 1_u32 << level;
        assert!(tiles_per_side >= NEAR_FIELD_WINDOW_TILES);
        let last_origin = tiles_per_side - NEAR_FIELD_WINDOW_TILES;
        for coordinate in [-1.0_f64, -0.999, 0.0, 0.999, 1.0] {
            let centre = (coordinate + 1.0) * 0.5 * f64::from(tiles_per_side);
            let low = centre - f64::from(NEAR_FIELD_WINDOW_TILES) * 0.5;
            let origin = (low.max(0.0) as u32).min(last_origin);
            assert!(
                origin + NEAR_FIELD_WINDOW_TILES <= tiles_per_side,
                "window at uv {coordinate} runs off the face"
            );
        }
    }

    #[test]
    fn shader_detail_ladder_matches_the_cpu_clearance_ladder() {
        let shader = planet_shader_source();
        // Resolves `const NAME: f32 = <literal>` and the one derived form the
        // ladder uses, `<OTHER> * <literal>`. Constants that are derived rather
        // than restated cannot drift, which is worth more than the parser is
        // worth avoiding.
        fn declared_in(shader: &str, name: &str, depth: u32) -> f32 {
            assert!(depth < 4, "{name} is defined circularly");
            let text = shader
                .split(&format!("const {name}: f32 ="))
                .nth(1)
                .and_then(|source| source.split(';').next())
                .unwrap_or_else(|| panic!("{name} is declared in the shader"));
            text.split('*')
                .map(|part| {
                    let part = part.trim();
                    part.parse::<f32>()
                        .unwrap_or_else(|_| declared_in(shader, part, depth + 1))
                })
                .product()
        }
        let declared = |name: &str| -> f32 { declared_in(&shader, name, 0) };
        assert_eq!(
            declared("TERRAIN_DETAIL_ROUGHNESS"),
            crate::planet::TERRAIN_DETAIL_ROUGHNESS as f32,
        );
        assert_eq!(
            declared("TERRAIN_DETAIL_START_WAVELENGTH_METERS"),
            crate::planet::TERRAIN_DETAIL_START_WAVELENGTH_METERS as f32,
        );
        assert_eq!(
            declared("TERRAIN_NORMAL_MIN_SAMPLE_METERS"),
            crate::planet::TERRAIN_DETAIL_MIN_FILTER_METERS as f32,
            "the shader floors its detail filter at the normal probe spacing",
        );
        // The erosion shaping is two knobs plus two constants derived from the
        // noise's own distribution. A drift in any of them makes the CPU's
        // clearance surface a different surface again, which is exactly the
        // failure M2a existed to remove.
        for (name, value) in [
            (
                "TERRAIN_DETAIL_RIDGE_SOFTNESS",
                crate::planet::TERRAIN_DETAIL_RIDGE_SOFTNESS,
            ),
            (
                "TERRAIN_DETAIL_RIDGE_CENTRE",
                crate::planet::TERRAIN_DETAIL_RIDGE_CENTRE,
            ),
            (
                "TERRAIN_DETAIL_RIDGE_SCALE",
                crate::planet::TERRAIN_DETAIL_RIDGE_SCALE,
            ),
            (
                "TERRAIN_DETAIL_RIDGE_STRENGTH",
                crate::planet::TERRAIN_DETAIL_RIDGE_STRENGTH,
            ),
            (
                "TERRAIN_DETAIL_RIDGE_NORMALISATION",
                crate::planet::TERRAIN_DETAIL_RIDGE_NORMALISATION,
            ),
            (
                "TERRAIN_DETAIL_ATTENUATION_SLOPE",
                crate::planet::TERRAIN_DETAIL_ATTENUATION_SLOPE,
            ),
            (
                "TERRAIN_DETAIL_HEADROOM_FACTOR",
                crate::planet::TERRAIN_DETAIL_HEADROOM_FACTOR,
            ),
            // The spectral tilt. Both halves matter: the gain sets how much
            // taller a massif is than the plain around it, and the taper sets
            // where that extra amplitude stops so the fine band -- which the
            // LOD budget is charged for -- is left alone.
            (
                "TERRAIN_DETAIL_LONG_GAIN",
                crate::planet::TERRAIN_DETAIL_LONG_GAIN,
            ),
            (
                "TERRAIN_DETAIL_TILT_TAPER_METERS",
                crate::planet::TERRAIN_DETAIL_TILT_TAPER_METERS,
            ),
            (
                "TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS",
                crate::planet::TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
            ),
        ] {
            assert_eq!(declared(name), value as f32, "{name} drifted");
        }
        let octaves = shader
            .split("const TERRAIN_DETAIL_OCTAVES: i32 = ")
            .nth(1)
            .and_then(|source| source.split(';').next())
            .and_then(|value| value.trim().parse::<u32>().ok())
            .expect("octave count is declared in the shader");
        assert_eq!(octaves, crate::planet::TERRAIN_DETAIL_OCTAVES);

        // The hash has to be reproducible on the CPU, not merely similar. A
        // float hash folded by fract cannot be: the shader evaluates it in f32
        // and the clearance ladder in f64, and at the finest octave those are
        // unrelated numbers. Pin the integer form and its salts here, because
        // the failure is invisible -- both sides keep producing plausible
        // terrain, just not the same terrain.
        let mix_body = shader
            .split("fn detail_mix(value: u32) -> u32 {")
            .nth(1)
            .and_then(|source| source.split('}').next())
            .expect("the shader hashes with detail_mix");
        assert!(
            !mix_body.contains("sin(") && !mix_body.contains("fract("),
            "the detail hash must stay integer-only: {mix_body}"
        );
        for step in ["value * 0x9e3779b1u", "h ^ (h >> 15u)"] {
            assert!(mix_body.contains(step), "detail_mix lost `{step}`");
        }
        for salt in ["0x27d4eb2fu", "0x9e3779b9u"] {
            assert!(
                shader.contains(salt),
                "the per-axis salt {salt} must match planet.rs"
            );
        }
    }

    /// The close-range material tile is the only thing that gives the ground
    /// texture underfoot, and it only works because it is never expressed as an
    /// absolute planet coordinate: 4e6/6 needs 7e5 tiles, where f32 quantises
    /// the lookup to whole texels and the texture stops varying per pixel.
    #[test]
    fn close_range_material_tile_is_built_anchor_locally_and_stays_fine() {
        let shader = planet_shader_source();
        let tile_meters = shader
            .split("const TERRAIN_MATERIAL_DETAIL_TILE_METERS: f32 = ")
            .nth(1)
            .and_then(|source| source.split(';').next())
            .and_then(|value| value.trim().parse::<f32>().ok())
            .expect("close-range material tile is declared");
        assert!(
            tile_meters <= 16.0,
            "a {tile_meters}m material tile is too coarse to read as ground texture"
        );

        let fine_position = shader
            .split("fn terrain_material_fine_position(")
            .nth(1)
            .and_then(|source| source.split("\nfn ").next())
            .expect("close-range tile coordinate is built by its own function");
        // Only the fraction of the anchor's tile coordinate may survive; the
        // per-pixel variation has to come from the short local offset.
        assert!(fine_position.contains("fract(anchor_tiles)"));
        assert!(fine_position.contains("local_meters / TERRAIN_MATERIAL_DETAIL_TILE_METERS"));
        // Without the warp the repeat lands on a regular lattice, which reads
        // as wallpaper however fine the tile is.
        assert!(fine_position.contains("warp"));
    }

    /// Normals are central-differenced over this spacing, so it is a hard limit
    /// on the finest relief the planet can display. It was 8m, which erased the
    /// 0.375m baked tiles entirely.
    #[test]
    fn normal_probe_spacing_resolves_metre_scale_relief() {
        let shader = planet_shader_source();
        let minimum = shader
            .split("const TERRAIN_NORMAL_MIN_SAMPLE_METERS: f32 = ")
            .nth(1)
            .and_then(|source| source.split(';').next())
            .and_then(|value| value.trim().parse::<f32>().ok())
            .expect("normal probe floor is declared");
        assert!(
            minimum <= 1.0,
            "normal probe floor {minimum}m cannot resolve metre-scale ground detail"
        );
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
        assert!(shader.contains("@group(2) @binding(6)"));
        assert!(shader.contains("@group(1) @binding(0)\nvar height_map"));
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
        assert!(normalized_shader.contains("terrain_sun_transmittance * terrain_direct_light"));
    }

    #[test]
    fn direct_surface_sunlight_progresses_from_orange_to_red_before_darkness() {
        let shader = planet_shader_source();
        let normalized_shader = shader.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(shader.contains("let orange_tint = vec3<f32>(1.20, 0.55, 0.16);"));
        assert!(shader.contains("let red_tint = vec3<f32>(1.35, 0.12, 0.03);"));
        assert!(shader.contains("return transmitted_sunlight * low_sun_tint * solar_visibility;"));
        assert!(normalized_shader.contains("sun_transmittance * specular"));
        assert!(normalized_shader.contains("terrain_sun_transmittance * terrain_direct_light"));
    }

    #[test]
    fn ocean_aerial_perspective_preserves_the_dark_water_body() {
        let shader = planet_shader_source();
        assert!(shader.contains("const OCEAN_AERIAL_PERSPECTIVE_WEIGHT: f32 = 0.35;"));
        assert_eq!(shader.matches("ocean_aerial_perspective(").count(), 5);
        assert!(shader.contains("water_surface_color,\n        aerial_color,"));
        assert!(shader.contains("sky_diffuse * daylight"));
    }

    #[test]
    fn close_terrain_detail_relight_is_bounded() {
        let shader = planet_shader_source();
        assert!(shader.contains("let detail_relight = clamp("));
        assert!(shader.contains("0.55,\n                1.75,"));
    }

    #[test]
    fn terrain_fog_targets_the_camera_sky_ray() {
        let shader = planet_shader_source();
        assert!(shader.contains("let camera_fog_color = sky_radiance("));
        assert!(shader.contains("mix(local_fog_color, camera_fog_color, 0.75)"));
    }

    #[test]
    fn terrain_material_pass_uses_faceted_slope_and_latitude_snowline() {
        let shader = planet_shader_source();
        assert!(shader.contains("let rock_amount = smoothstep(0.10, 0.42, slope);"));
        assert!(shader.contains("let snowline_meters = mix(6200.0, 2200.0, latitude_amount);"));
        assert!(shader.contains("camera_distance_meters * 0.01"));
        assert!(shader.contains("TERRAIN_NORMAL_MIN_SAMPLE_METERS"));
        assert!(shader.contains("let normal_step_scale = cube_step / requested_cube_step;"));
        assert_eq!(shader.matches("terrain_material_color(").count(), 2);
        assert!(shader.contains("terrain_normal,\n        direction,\n    );"));
    }

    #[test]
    fn raster_aerial_retexturing_uses_continuous_affine_components() {
        let shader = planet_shader_source();
        let normalized_shader = shader.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(shader.contains("@location(2) aerial_in_scatter: vec3<f32>"));
        assert!(shader.contains("@location(8) aerial_transmittance: vec3<f32>"));
        assert!(normalized_shader.contains(
            "let textured_aerial_color = textured_surface_lighting * input.aerial_transmittance + input.aerial_in_scatter;"
        ));
        assert!(!shader.contains("let aerial_ratio ="));
        assert!(!shader.contains("input.surface_lighting > vec3<f32>(1.0e-3)"));
    }

    #[test]
    fn fullscreen_sky_applies_saturation_without_green_and_retains_blue_hour() {
        let shader = include_str!("atmosphere.wgsl");
        let module = wgpu::naga::front::wgsl::parse_str(shader)
            .expect("atmosphere shader must parse before WGPU creates the pipeline");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("atmosphere shader must validate before WGPU creates the pipeline");
        assert!(shader.contains("const SKY_ATMOSPHERE_SATURATION: f32 = 1.3;"));
        assert!(shader.contains("fn saturate_sky_color(color: vec3<f32>)"));
        assert!(shader.contains("fn suppress_green_dominance(color: vec3<f32>)"));
        assert!(shader.contains("fn blue_hour_weight(camera_solar_zenith_cosine: f32)"));
        assert!(shader.contains("fn blue_hour_rayleigh_scattering("));
        assert!(shader.contains("optical_depth / (vec3<f32>(1.0) + optical_depth)"));
        assert!(shader.contains("suppress_green_dominance(saturate_sky_color(sky_radiance))"));
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
    fn near_field_material_channels_use_the_raster_sampling_contract() {
        let index = |x: u32, y: u32| (y * TILE_STORED_SIZE + x) as usize;
        let mut biomes = vec![0_u8; (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize];
        let mut moisture = vec![0_u8; biomes.len()];
        let coordinate = TILE_GUTTER + (TILE_LOGICAL_SIZE - 1) / 2;
        biomes[index(coordinate, coordinate)] = 7;
        moisture[index(coordinate, coordinate)] = 64;
        moisture[index(coordinate + 1, coordinate)] = 128;
        moisture[index(coordinate, coordinate + 1)] = 192;
        moisture[index(coordinate + 1, coordinate + 1)] = 255;

        assert_eq!(sample_biome_cpu(&biomes, [0.5, 0.5]), 7);
        assert_eq!(sample_moisture_cpu(&moisture, [0.5, 0.5]), 64);
        assert_eq!(
            sample_moisture_cpu(
                &moisture,
                [
                    0.5 + 0.5 / (TILE_LOGICAL_SIZE - 1) as f32,
                    0.5 + 0.5 / (TILE_LOGICAL_SIZE - 1) as f32,
                ],
            ),
            160,
        );
    }

    #[test]
    fn ocean_culling_requires_the_complete_sampled_height_footprint_to_be_land() {
        let index = |x: u32, y: u32| (y * TILE_STORED_SIZE + x) as usize;
        let mut heights = vec![100.0; (TILE_STORED_SIZE * TILE_STORED_SIZE) as usize];

        // Ocean in an unrelated part of the resolved source tile cannot
        // affect this fallback sub-rectangle.
        heights[index(110, 110)] = -1.0;
        assert!(height_footprint_is_strictly_land(
            &heights,
            [0.25, 0.25],
            [0.25, 0.25],
        ));

        // Zero, negative, or invalid data in a texel touched by bilinear
        // sampling must keep the ocean draw.
        let sampled = index(40, 40);
        for height in [0.0, -1.0, f32::NAN] {
            heights[sampled] = height;
            assert!(!height_footprint_is_strictly_land(
                &heights,
                [0.25, 0.25],
                [0.25, 0.25],
            ));
        }
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
        assert_eq!(edge_stitch_level_delta(extreme_stitch, 3), 7);

        let shader = planet_shader_source();
        assert!(shader.contains("fn stitched_tile_uv("));
        assert!(shader.contains("fn edge_detail_filter_meters("));
        assert!(shader.contains("fn stitched_surface_direction("));
        assert!(shader.contains("fn lod_morphed_tile_uv("));
        assert!(shader.contains("@location(10) node_uv_origin_span: vec4<f32>"));
        assert!(shader.contains("@location(11) node_anchor_direction_cube_length: vec4<f32>"));
        assert!(shader.contains("let stride = 1u << min(level_delta, 2u);"));
        assert!(shader.contains("requested_level - min(requested_level, level_delta)"));
    }

    #[test]
    fn cpu_raster_detail_filter_matches_node_edge_and_distance_spacing() {
        let node = QuadtreeNode {
            face: CubeFace::PositiveX.index(),
            level: 18,
            x: 100_000,
            y: 120_000,
        };
        let [u_min, v_min, u_max, v_max] = node.uv_bounds();
        let node_spacing = 2.0 * PLANET_RADIUS_METERS
            / (f64::from(1_u32 << node.level) * crate::planet::CHUNK_GRID_QUADS as f64);
        let surface = SurfaceDetailNode {
            node,
            edge_stitch: 0,
            source_key: None,
        };

        assert!(
            (surface_detail_filter_meters(
                surface,
                [(u_min + u_max) * 0.5, (v_min + v_max) * 0.5],
                0.0,
            ) - node_spacing)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (surface_detail_filter_meters(
                surface,
                [(u_min + u_max) * 0.5, (v_min + v_max) * 0.5],
                2_000.0,
            ) - 20.0)
                .abs()
                < f64::EPSILON
        );

        let left_edge_delta = 2_u32 << (3 * 5);
        let stitched = SurfaceDetailNode {
            edge_stitch: left_edge_delta,
            ..surface
        };
        assert!(
            (surface_detail_filter_meters(stitched, [u_min, (v_min + v_max) * 0.5], 0.0,)
                - node_spacing * 4.0)
                .abs()
                < 1.0e-12
        );
    }

    #[test]
    fn outmap_culling_shell_contains_the_runtime_detail_ladder() {
        let height_max_meters = 8_846.0;
        let [_, maximum] = conservative_outmap_height_bounds(-5_000.0, height_max_meters);

        assert!(
            maximum
                >= height_max_meters * OUTMAP_TERRAIN_FAR_HEIGHT_SCALE
                    + TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
            "culling shell {maximum}m omits the live runtime ladder"
        );
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
            super::OUTMAP_GEOMETRIC_ERROR,
            &|_| MAX_LOD_LEVEL,
            None,
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

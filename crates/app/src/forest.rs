use std::{mem::size_of, time::Instant};

use catinthegarden_coretypes::{
    TileKey, direction_to_face_uv, face_uv_to_direction, tile_key_for_direction,
};
use glam::DVec3;
use wgpu::util::DeviceExt;

use crate::{
    planet::PLANET_RADIUS_METERS,
    terrain::{TerrainRenderer, forest_biome_requires_evergreen, forest_surface_is_eligible},
};

pub const FOREST_CENTRE_DIRECTION: DVec3 =
    DVec3::new(0.374_871_986_443, 0.737_334_908_710, 0.561_968_171_854);
pub const FOREST_START_PITCH_RADIANS: f64 = 4.0_f64.to_radians();

const TREE_COUNT: usize = 12_288;
const TREE_BASE_SINK_METERS: f64 = 0.45;
const TREE_HEIGHT_MIN_METERS: f32 = 11.0;
const TREE_HEIGHT_RANGE_METERS: f32 = 13.0;
const FOREST_DRAW_ALTITUDE_METERS: f64 = 50_000.0;
const FOREST_CELL_LEVEL: u8 = 12;
const FOREST_MINIMUM_MOISTURE: f32 = 0.38;
const FOREST_MAXIMUM_SLOPE_RADIANS: f64 = 32.0_f64.to_radians();
const FOREST_PATCH_INNER_RADIUS_METERS: f64 = 700.0;
const FOREST_PATCH_OUTER_RADIUS_METERS: f64 = 1_000.0;
const FOREST_PATCH_MINIMUM_REBUILD_SECONDS: f64 = 0.5;
const FOREST_PATCH_TRANSITION_SECONDS: f64 = 1.5;
const FOREST_PATCH_CANDIDATES_PER_FRAME: usize = 128;
const TREE_LOD_FULL_PIXELS: f64 = 12.0;
const TREE_LOD_MEDIUM_PIXELS: f64 = 3.0;
const TREE_LOD_SPARSE_PIXELS: f64 = 1.0;
const FOREST_PLANET_SEED: u32 = 0x6d2b_79f5;

fn forest_rendering_from_env() -> bool {
    match std::env::var("CATINGARDEN_FOREST") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        ),
        Err(_) => true,
    }
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

    fn advance(&mut self, terrain: &TerrainRenderer, camera_altitude_meters: f64) -> bool {
        let batch_end = pending_batch_end(self.next_candidate, self.candidates.len());
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
            ) && f64::from(layout.seed)
                <= forest_density_at(direction) * forest_cell_edge_falloff(self.key, direction)
            {
                let centre = direction
                    * (PLANET_RADIUS_METERS + sample.height_meters - TREE_BASE_SINK_METERS);
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
    pub patch_count: u8,
    pub instances: u32,
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
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    patch: Option<ForestPatch>,
    pending_patch: Option<PendingForestPatch>,
    retiring_trees: Vec<TreeInstance>,
    patch_transition_started: Option<Instant>,
    draw_instances: Vec<TreeInstance>,
    lod_counts: TreeLodCounts,
    last_patch_rebuild_at: Option<Instant>,
    last_empty_patch_key: Option<TileKey>,
    rebuild_count: u64,
    enabled: bool,
}

impl ForestRenderer {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        _terrain: &mut TerrainRenderer,
    ) -> Self {
        let initial_key = forest_cell_key(FOREST_CENTRE_DIRECTION);
        let initial_patch = ForestPatch {
            key: initial_key,
            centre_direction: forest_cell_centre_direction(initial_key),
            trees: Vec::new(),
            minimum_source_level: None,
        };
        let instances = initial_patch.trees.clone();
        let initial_instance_count = instances.len() as u32;
        let enabled = forest_rendering_from_env();
        tracing::info!(
            target: "catinthegarden::forest",
            enabled,
            tree_instances = instances.len(),
            "configured billboard forest"
        );
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("forest camera uniform"),
            size: size_of::<ForestUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forest bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forest bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forest pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forest billboard shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("forest.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("forest billboard pipeline"),
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
            cache: None,
        });
        let mut buffer_instances = vec![<TreeInstance as bytemuck::Zeroable>::zeroed(); TREE_COUNT];
        buffer_instances[..instances.len()].copy_from_slice(&instances);
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest tree instances"),
            contents: bytemuck::cast_slice(&buffer_instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            instance_count: initial_instance_count,
            patch: Some(initial_patch),
            pending_patch: Some(PendingForestPatch::new(initial_key)),
            retiring_trees: Vec::new(),
            patch_transition_started: None,
            draw_instances: instances,
            lod_counts: TreeLodCounts {
                full: initial_instance_count,
                ..Default::default()
            },
            last_patch_rebuild_at: None,
            last_empty_patch_key: None,
            rebuild_count: 0,
            enabled,
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

    /// Updates the one camera-local patch after terrain streaming has made
    /// newly requested source tiles resident. This only probes the resident
    /// terrain cache; missing samples retain the previous patch.
    #[allow(clippy::too_many_arguments)]
    pub fn update_patch(
        &mut self,
        queue: &wgpu::Queue,
        terrain: &TerrainRenderer,
        camera_planet_position: DVec3,
        camera_altitude_meters: f64,
        viewport_height: u32,
        vertical_fov_radians: f64,
        simulation_time_seconds: f64,
    ) {
        let camera_direction = camera_planet_position.normalize_or_zero();
        if camera_direction.length_squared() <= f64::EPSILON
            || !camera_altitude_meters.is_finite()
            || !simulation_time_seconds.is_finite()
        {
            return;
        }
        if camera_altitude_meters >= FOREST_DRAW_ALTITUDE_METERS {
            self.pending_patch = None;
            return;
        }
        let desired_key = forest_cell_key(camera_direction);
        if self
            .last_empty_patch_key
            .is_some_and(|key| key != desired_key)
        {
            self.last_empty_patch_key = None;
        }
        let transition_in_progress = self.patch_transition_progress() < 1.0;
        let rebuild_due = patch_rebuild_due(
            self.patch.as_ref(),
            desired_key,
            camera_direction,
            self.last_patch_rebuild_at
                .map(|last| last.elapsed().as_secs_f64()),
        );
        if self.last_empty_patch_key == Some(desired_key) {
            // A fully evaluated cell with no eligible trees is a valid empty
            // result. Keep the last populated patch available while the
            // camera is in this cell so distance-based LOD can fade it back
            // in when the player returns, rather than requiring a second
            // cell-boundary crossing.
            self.pending_patch = None;
        } else if self
            .patch
            .as_ref()
            .is_some_and(|patch| patch.key == desired_key && !patch.trees.is_empty())
        {
            self.pending_patch = None;
        } else if !transition_in_progress && rebuild_due {
            if self
                .pending_patch
                .as_ref()
                .is_none_or(|pending| pending.key != desired_key)
            {
                self.pending_patch = Some(PendingForestPatch::new(desired_key));
            }
            let completed = self
                .pending_patch
                .as_mut()
                .is_some_and(|pending| pending.advance(terrain, camera_altitude_meters));
            if completed {
                let patch = self
                    .pending_patch
                    .take()
                    .expect("completed forest patch is pending")
                    .finish();
                let now = Instant::now();
                if retain_populated_patch_for_empty_cell(self.patch.as_ref(), &patch) {
                    // Do not replace a visible forest with an empty
                    // neighbouring cell. The existing patch remains the
                    // source for LOD, so it can reappear continuously during
                    // a retreat/return instead of popping back only after a
                    // cell boundary is crossed again.
                    self.last_empty_patch_key = Some(patch.key);
                    self.last_patch_rebuild_at = Some(now);
                    self.pending_patch = None;
                    tracing::info!(
                        target: "catinthegarden::forest",
                        face = ?patch.key.face,
                        level = patch.key.level,
                        x = patch.key.x,
                        y = patch.key.y,
                        "retained procedural forest patch for empty neighbouring cell"
                    );
                } else {
                    let patch_key = patch.key;
                    let tree_instances = patch.trees.len();
                    let minimum_source_level = patch.minimum_source_level;
                    self.last_empty_patch_key = None;
                    self.retiring_trees = self
                        .patch
                        .replace(patch)
                        .map(|patch| patch.trees)
                        .unwrap_or_default();
                    self.patch_transition_started = Some(now);
                    self.last_patch_rebuild_at = Some(now);
                    self.rebuild_count += 1;
                    tracing::info!(
                        target: "catinthegarden::forest",
                        face = ?patch_key.face,
                        level = patch_key.level,
                        x = patch_key.x,
                        y = patch_key.y,
                        tree_instances,
                        minimum_source_level = ?minimum_source_level,
                        "replaced procedural forest patch"
                    );
                }
            }
        }
        self.update_tree_lod(
            queue,
            camera_planet_position,
            viewport_height,
            vertical_fov_radians,
        );
    }

    fn patch_transition_progress(&self) -> f64 {
        self.patch_transition_started
            .map(|started| patch_transition_progress(started.elapsed().as_secs_f64()))
            .unwrap_or(1.0)
    }

    fn update_tree_lod(
        &mut self,
        queue: &wgpu::Queue,
        camera_planet_position: DVec3,
        viewport_height: u32,
        vertical_fov_radians: f64,
    ) {
        let transition_progress = self.patch_transition_progress();
        if transition_progress >= 1.0 {
            self.retiring_trees.clear();
            self.patch_transition_started = None;
        }
        let Some(patch) = &self.patch else {
            return;
        };
        let candidate_count = patch.trees.len().max(self.retiring_trees.len());
        let mut draw_instances = Vec::with_capacity(candidate_count);
        let mut lod_counts = TreeLodCounts::default();
        for index in 0..candidate_count {
            let tree = transition_tree_for_slot(
                index,
                &self.retiring_trees,
                &patch.trees,
                transition_progress,
            );
            let Some(tree) = tree else {
                continue;
            };
            let projected_pixels = projected_tree_height_pixels(
                tree,
                camera_planet_position,
                viewport_height,
                vertical_fov_radians,
            );
            let lod = tree_lod(projected_pixels);
            lod_counts.add(lod);
            if let Some(tree) = lodded_tree_instance(tree, projected_pixels) {
                draw_instances.push(tree);
            }
        }
        if self.draw_instances != draw_instances {
            debug_assert!(draw_instances.len() <= TREE_COUNT);
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
        self.lod_counts = lod_counts;
    }

    pub fn stats(&self) -> ForestStats {
        ForestStats {
            patch_count: u8::from(self.patch.is_some()),
            instances: self.instance_count,
            full_instances: self.lod_counts.full,
            medium_instances: self.lod_counts.medium,
            sparse_instances: self.lod_counts.sparse,
            zero_instances: self.lod_counts.zero,
            rebuild_count: self.rebuild_count,
            patch_key: self.patch.as_ref().map(|patch| patch.key),
            minimum_source_level: self
                .patch
                .as_ref()
                .and_then(|patch| patch.minimum_source_level),
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

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        camera_altitude_meters: f64,
    ) {
        if !self.enabled
            || camera_altitude_meters >= FOREST_DRAW_ALTITUDE_METERS
            || self.instance_count == 0
        {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        render_pass.draw(0..6, 0..self.instance_count);
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

/// Soft, slightly warped footprint inside a canonical cell. Candidates still
/// belong to exactly one cell, but the visible stand tapers before the cell
/// edge so a square ownership boundary cannot become a square forest.
fn forest_cell_edge_falloff(key: TileKey, direction: DVec3) -> f64 {
    let (face, u, v) = direction_to_face_uv(direction);
    if face != key.face {
        return 0.0;
    }
    let cells_per_axis = f64::from(1_u32 << key.level);
    let cell_span = 2.0 / cells_per_axis;
    let u_min = -1.0 + f64::from(key.x) * cell_span;
    let v_min = -1.0 + f64::from(key.y) * cell_span;
    let local_u = (2.0 * (u - (u_min + cell_span * 0.5)) / cell_span).clamp(-1.0, 1.0);
    let local_v = (2.0 * (v - (v_min + cell_span * 0.5)) / cell_span).clamp(-1.0, 1.0);
    let radius = (local_u * local_u + local_v * local_v).sqrt();
    let warp = forest_noise_at(direction, 768.0) * 0.10;
    let boundary = (0.88 + warp).clamp(0.74, 0.98);
    let fade_start = boundary - 0.24;
    1.0 - smoothstep01((radius - fade_start) / (boundary - fade_start))
}

fn retain_populated_patch_for_empty_cell(
    current: Option<&ForestPatch>,
    incoming: &ForestPatch,
) -> bool {
    incoming.trees.is_empty() && current.is_some_and(|patch| !patch.trees.is_empty())
}

fn patch_rebuild_due(
    patch: Option<&ForestPatch>,
    desired_key: TileKey,
    camera_direction: DVec3,
    seconds_since_last_rebuild: Option<f64>,
) -> bool {
    let Some(patch) = patch else {
        return true;
    };
    if patch.trees.is_empty() {
        return true;
    }
    if patch.key == desired_key && !patch.trees.is_empty() {
        return false;
    }
    let distance_meters =
        camera_direction.angle_between(patch.centre_direction).abs() * PLANET_RADIUS_METERS;
    if distance_meters <= FOREST_PATCH_INNER_RADIUS_METERS {
        return false;
    }
    if distance_meters < FOREST_PATCH_OUTER_RADIUS_METERS {
        return false;
    }
    seconds_since_last_rebuild
        .map(|elapsed| elapsed >= FOREST_PATCH_MINIMUM_REBUILD_SECONDS)
        .unwrap_or(true)
}

fn forest_patch_tree_layouts(key: TileKey) -> Vec<(DVec3, TreeLayout)> {
    let cells_per_axis = f64::from(1_u32 << key.level);
    let cell_span = 2.0 / cells_per_axis;
    let u_min = -1.0 + f64::from(key.x) * cell_span;
    let v_min = -1.0 + f64::from(key.y) * cell_span;
    let cell_seed = canonical_cell_seed(key);
    (0..TREE_COUNT)
        .map(|index| {
            let index = index as u32;
            // unit_hash is half-open, so a candidate belongs to exactly this
            // cell even at cube-face and cell boundaries.
            let u = u_min + unit_hash(cell_seed ^ index ^ 0x6a09_e667) * cell_span;
            let v = v_min + unit_hash(cell_seed ^ index ^ 0xbb67_ae85) * cell_span;
            (
                face_uv_to_direction(key.face, u, v),
                tree_layout_from_seed(cell_seed ^ index),
            )
        })
        .collect()
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

/// Low-frequency, seam-safe density field shared conceptually with the far
/// terrain canopy material. The floor keeps every eligible cold/forest cell
/// capable of producing trees; the field only creates natural clearings and
/// denser stands instead of a hard-edged disk.
fn forest_density_at(direction: DVec3) -> f64 {
    let value = forest_noise_at(direction, 192.0) * 0.5 + 0.5;
    let cluster = smoothstep01((value - 0.24) / (0.76 - 0.24));
    0.35 + cluster * 0.65
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

fn projected_tree_height_pixels(
    tree: TreeInstance,
    camera_planet_position: DVec3,
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
    let distance_meters = camera_planet_position
        .distance(DVec3::from_array([
            f64::from(tree.centre_and_height[0]),
            f64::from(tree.centre_and_height[1]),
            f64::from(tree.centre_and_height[2]),
        ]))
        .max(1.0);
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

fn lodded_tree_instance(tree: TreeInstance, projected_pixels: f64) -> Option<TreeInstance> {
    let density = tree_lod_density(projected_pixels);
    (tree.width_shade_kind_seed[3] < density).then_some(tree)
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
            0.5 * progress as f32
        }
        TreeLod::Zero => 0.0,
    }
}

fn patch_transition_progress(elapsed_seconds: f64) -> f64 {
    smoothstep01(elapsed_seconds / FOREST_PATCH_TRANSITION_SECONDS)
}

fn pending_batch_end(next_candidate: usize, candidate_count: usize) -> usize {
    next_candidate
        .saturating_add(FOREST_PATCH_CANDIDATES_PER_FRAME)
        .min(candidate_count)
}

fn transition_tree_for_slot(
    index: usize,
    retiring_trees: &[TreeInstance],
    incoming_trees: &[TreeInstance],
    transition_progress: f64,
) -> Option<TreeInstance> {
    if transition_progress <= 0.0 && !retiring_trees.is_empty() {
        return retiring_trees.get(index).copied();
    }
    if retiring_trees.is_empty() || transition_progress >= 1.0 {
        return incoming_trees.get(index).copied();
    }
    let threshold = unit_hash((index as u32) ^ 0x3c6e_f372);
    if transition_progress >= threshold {
        incoming_trees.get(index).copied()
    } else {
        retiring_trees.get(index).copied()
    }
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
        assert!((0.35..=1.0).contains(&density));
    }

    #[test]
    fn forest_cell_footprint_softens_before_the_square_cell_edge() {
        let key = forest_cell_key(FOREST_CENTRE_DIRECTION);
        let cells_per_axis = f64::from(1_u32 << key.level);
        let cell_span = 2.0 / cells_per_axis;
        let u_min = -1.0 + f64::from(key.x) * cell_span;
        let v_min = -1.0 + f64::from(key.y) * cell_span;
        let centre = forest_cell_centre_direction(key);
        let near_corner =
            face_uv_to_direction(key.face, u_min + cell_span * 0.99, v_min + cell_span * 0.99);
        assert!(forest_cell_edge_falloff(key, centre) > 0.99);
        assert!(forest_cell_edge_falloff(key, near_corner) < 0.1);
    }

    #[test]
    fn procedural_patch_cells_have_canonical_half_open_tree_ownership() {
        let key = TileKey {
            face: catinthegarden_coretypes::CubeFace::PositiveX,
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
        assert_eq!(lodded_tree_instance(tree, 5.0), Some(tree));
        assert_eq!(lodded_tree_instance(tree, 2.0), Some(tree));
        assert_eq!(lodded_tree_instance(tree, 0.5), None);

        let rejected = TreeInstance {
            width_shade_kind_seed: [8.0, 1.0, 0.0, 0.75],
            ..tree
        };
        assert_eq!(lodded_tree_instance(rejected, 5.0), None);
        assert_eq!(lodded_tree_instance(rejected, 2.0), None);
    }

    #[test]
    fn far_tree_density_enters_continuously_instead_of_revealing_a_quarter_patch() {
        assert_eq!(tree_lod_density(TREE_LOD_SPARSE_PIXELS - 0.01), 0.0);
        assert_eq!(tree_lod_density(TREE_LOD_SPARSE_PIXELS), 0.0);
        let just_visible = tree_lod_density(TREE_LOD_SPARSE_PIXELS + 0.01);
        assert!(just_visible > 0.0 && just_visible < 0.001);
        assert!(tree_lod_density(2.0) < tree_lod_density(2.5));
        assert_eq!(tree_lod_density(TREE_LOD_MEDIUM_PIXELS), 0.5);
    }

    #[test]
    fn patch_candidate_work_is_bounded_per_frame() {
        let mut processed = 0;
        let mut frames = 0;
        while processed < TREE_COUNT {
            let next = pending_batch_end(processed, TREE_COUNT);
            assert!(next - processed <= FOREST_PATCH_CANDIDATES_PER_FRAME);
            processed = next;
            frames += 1;
        }
        assert_eq!(
            frames,
            TREE_COUNT.div_ceil(FOREST_PATCH_CANDIDATES_PER_FRAME)
        );
    }

    #[test]
    fn patch_transition_keeps_one_bounded_population_and_replaces_it_gradually() {
        let tree = |marker: f32, seed: f32| TreeInstance {
            centre_and_height: [marker, 0.0, 0.0, 20.0],
            width_shade_kind_seed: [8.0, 1.0, 0.0, seed],
        };
        let retiring = (0..128)
            .map(|index| tree(-1.0, index as f32 / 128.0))
            .collect::<Vec<_>>();
        let incoming = (0..128)
            .map(|index| tree(1.0, index as f32 / 128.0))
            .collect::<Vec<_>>();
        let select = |progress| {
            (0..retiring.len().max(incoming.len()))
                .filter_map(|index| transition_tree_for_slot(index, &retiring, &incoming, progress))
                .collect::<Vec<_>>()
        };
        assert!(
            select(0.0)
                .iter()
                .all(|tree| tree.centre_and_height[0] < 0.0)
        );
        let halfway = select(0.5);
        assert!(halfway.len() <= TREE_COUNT);
        assert!(halfway.iter().any(|tree| tree.centre_and_height[0] < 0.0));
        assert!(halfway.iter().any(|tree| tree.centre_and_height[0] > 0.0));
        assert!(
            select(1.0)
                .iter()
                .all(|tree| tree.centre_and_height[0] > 0.0)
        );
        assert_eq!(patch_transition_progress(0.0), 0.0);
        assert_eq!(
            patch_transition_progress(FOREST_PATCH_TRANSITION_SECONDS),
            1.0
        );
    }

    #[test]
    fn patch_key_hysteresis_and_rebuild_interval_retain_the_old_patch() {
        let key = TileKey::root(catinthegarden_coretypes::CubeFace::PositiveX);
        let patch = ForestPatch {
            key,
            centre_direction: DVec3::X,
            trees: vec![TreeInstance {
                centre_and_height: [0.0; 4],
                width_shade_kind_seed: [0.0; 4],
            }],
            minimum_source_level: None,
        };
        let desired_key = TileKey::root(catinthegarden_coretypes::CubeFace::NegativeX);
        let direction_at_distance = |distance_meters: f64| {
            let angle = distance_meters / PLANET_RADIUS_METERS;
            DVec3::new(angle.cos(), angle.sin(), 0.0)
        };
        assert!(!patch_rebuild_due(
            Some(&patch),
            desired_key,
            direction_at_distance(FOREST_PATCH_INNER_RADIUS_METERS * 0.9),
            Some(0.0),
        ));
        assert!(!patch_rebuild_due(
            Some(&patch),
            desired_key,
            direction_at_distance(FOREST_PATCH_OUTER_RADIUS_METERS * 1.1),
            Some(FOREST_PATCH_MINIMUM_REBUILD_SECONDS * 0.4),
        ));
        assert!(patch_rebuild_due(
            Some(&patch),
            desired_key,
            direction_at_distance(FOREST_PATCH_OUTER_RADIUS_METERS * 1.1),
            Some(FOREST_PATCH_MINIMUM_REBUILD_SECONDS),
        ));
    }

    #[test]
    fn empty_neighbouring_cell_does_not_replace_a_populated_patch() {
        let key = TileKey::root(catinthegarden_coretypes::CubeFace::PositiveX);
        let populated = ForestPatch {
            key,
            centre_direction: DVec3::X,
            trees: vec![TreeInstance {
                centre_and_height: [0.0; 4],
                width_shade_kind_seed: [0.0; 4],
            }],
            minimum_source_level: None,
        };
        let empty = ForestPatch {
            key: TileKey::root(catinthegarden_coretypes::CubeFace::NegativeX),
            centre_direction: -DVec3::X,
            trees: Vec::new(),
            minimum_source_level: None,
        };
        assert!(retain_populated_patch_for_empty_cell(
            Some(&populated),
            &empty
        ));
        assert!(!retain_populated_patch_for_empty_cell(None, &empty));
        assert!(!retain_populated_patch_for_empty_cell(
            Some(&populated),
            &populated
        ));
    }

    #[test]
    fn forest_shader_is_a_depth_writing_procedural_billboard() {
        let shader = include_str!("forest.wgsl");
        wgpu::naga::front::wgsl::parse_str(shader).expect("forest shader parses");
        assert!(shader.contains("centre_and_height: vec4<f32>"));
        assert!(shader.contains("if !trunk && !canopy"));
        assert!(!shader.contains("textureSample"));
    }

    #[test]
    fn forest_shader_has_no_unconditional_night_light() {
        let shader = include_str!("forest.wgsl");
        assert!(shader.contains("fn tree_lighting(solar_elevation_cosine: f32) -> f32"));
        assert!(shader.contains("smoothstep(-0.18, 0.02, solar_elevation_cosine) * 0.36"));
        assert!(shader.contains("return direct + sky_ambient;"));
        assert!(shader.contains("trunk_colour * 0.75 * input.lighting"));
        assert!(!shader.contains("0.36 + sun_amount"));
    }

    #[test]
    fn authored_forest_centre_is_normalized() {
        assert!((FOREST_CENTRE_DIRECTION.length() - 1.0).abs() < 1.0e-9);
        assert!(FOREST_CENTRE_DIRECTION.y > 0.6);
    }

    #[test]
    fn forest_has_a_render_only_performance_switch() {
        let source = include_str!("forest.rs");
        assert!(source.contains("CATINGARDEN_FOREST"));
        assert!(source.contains("if !self.enabled"));
    }
}

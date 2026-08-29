use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    time::{Duration, Instant},
};

use catinthegarden_coretypes::{
    TileKey, direction_to_face_uv, face_uv_to_direction, tile_key_for_direction,
};
use glam::DVec3;
use wgpu::util::DeviceExt;

use crate::{
    planet::PLANET_RADIUS_METERS,
    terrain::{
        TerrainForestSample, TerrainRenderer, forest_biome_requires_evergreen,
        forest_surface_is_eligible,
    },
};

pub const FOREST_CENTRE_DIRECTION: DVec3 =
    DVec3::new(0.374_871_986_443, 0.737_334_908_710, 0.561_968_171_854);
pub const FOREST_START_PITCH_RADIANS: f64 = 4.0_f64.to_radians();

const TREE_COUNT: usize = 12_288;
const TREE_BASE_SINK_METERS: f64 = 0.45;
const TREE_HEIGHT_MIN_METERS: f32 = 11.0;
const TREE_HEIGHT_RANGE_METERS: f32 = 13.0;
const FOREST_DRAW_ALTITUDE_METERS: f64 = 50_000.0;
const FOREST_TREE_RENDER_DISTANCE_METERS: f64 = 8_000.0;
const FOREST_PREFETCH_DISTANCE_METERS: f64 = 12_000.0;
const FOREST_CELL_LEVEL: u8 = 12;
const FOREST_MINIMUM_MOISTURE: f32 = 0.38;
const FOREST_MAXIMUM_SLOPE_RADIANS: f64 = 32.0_f64.to_radians();
const FOREST_PATCH_TRANSITION_SECONDS: f64 = 1.5;
const FOREST_PATCH_CANDIDATES_PER_FRAME: usize = 128;
const FOREST_PRIMARY_PATCH_CANDIDATES_PER_FRAME: usize = 256;
const FOREST_INITIAL_PATCH_CANDIDATES_PER_FRAME: usize = 512;
const FOREST_STARTUP_PATCH_COUNT: u64 = 3;
const FOREST_MAX_RENDERABLE_PATCHES: usize = 128;
const FOREST_MAX_CACHED_PATCHES: usize = 256;
const FOREST_MAX_DRAW_INSTANCES: usize = 262_144;
const TREE_LOD_FULL_PIXELS: f64 = 12.0;
const TREE_LOD_MEDIUM_PIXELS: f64 = 3.0;
const TREE_LOD_SPARSE_PIXELS: f64 = 1.0;
const FOREST_PLANET_SEED: u32 = 0x6d2b_79f5;
const FOREST_BEAM_ATMOSPHERE_HEIGHT_METERS: f64 = 2_880_000.0;
const FOREST_BEAM_TOP_RADIUS_METERS: f64 =
    PLANET_RADIUS_METERS + FOREST_BEAM_ATMOSPHERE_HEIGHT_METERS;
const FOREST_BEAM_LOCATOR_SPACING_METERS: f64 = 1_000_000.0;
const FOREST_BEAM_REFINEMENT_CANDIDATES: usize = 512;

fn forest_rendering_from_env() -> bool {
    match std::env::var("CATINGARDEN_FOREST") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        ),
        Err(_) => true,
    }
}

fn forest_beams_from_env() -> bool {
    matches!(
        std::env::var("CATINGARDEN_FOREST_BEAMS")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "on")
    )
}

fn forest_shader_source() -> String {
    [
        include_str!("forest.wgsl"),
        include_str!("weather_cloud_density.wgsl"),
    ]
    .join("\n")
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct TreeInstance {
    centre_and_height: [f32; 4],
    width_shade_kind_seed: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ForestBeamVertex {
    direction_and_base_radius: [f32; 4],
    uv: [f32; 2],
}

impl ForestBeamVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ForestBeamAnchor {
    direction: DVec3,
    base_radius_meters: f64,
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
    pub beam_count: u16,
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
    pub beams_enabled: bool,
}

pub struct ForestRenderer {
    pipeline: wgpu::RenderPipeline,
    beam_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    beam_vertex_buffer: wgpu::Buffer,
    instance_count: u32,
    beam_vertex_count: u32,
    patches: BTreeMap<TileKey, ForestPatch>,
    pending_patch: Option<PendingForestPatch>,
    draw_instances: Vec<TreeInstance>,
    lod_counts: TreeLodCounts,
    empty_patch_keys: BTreeSet<TileKey>,
    primary_patch_key: Option<TileKey>,
    rebuild_count: u64,
    enabled: bool,
    beams_enabled: bool,
}

impl ForestRenderer {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        weather_field_bind_group_layout: &wgpu::BindGroupLayout,
        global_forest_samples: &[TerrainForestSample],
        terrain: &mut TerrainRenderer,
    ) -> Self {
        let initial_key = forest_cell_key(FOREST_CENTRE_DIRECTION);
        let enabled = forest_rendering_from_env();
        let beams_enabled = forest_beams_from_env();
        let coarse_beam_anchors = global_forest_beam_anchors(global_forest_samples);
        let beam_anchors = coarse_beam_anchors
            .iter()
            .copied()
            .filter_map(|anchor| {
                refine_global_forest_beam_anchor(anchor, |direction| {
                    terrain
                        .prepare_global_forest_locator_sample(direction)
                        .filter(|sample| {
                            forest_surface_is_eligible(
                                *sample,
                                FOREST_MINIMUM_MOISTURE,
                                FOREST_MAXIMUM_SLOPE_RADIANS,
                            )
                        })
                        .map(|sample| sample.height_meters)
                })
            })
            .collect::<Vec<_>>();
        let beam_vertices = forest_beam_vertices_for_anchors(&beam_anchors);
        tracing::info!(
            target: "catinthegarden::forest",
            enabled,
            beams_enabled,
            maximum_renderable_patches = FOREST_MAX_RENDERABLE_PATCHES,
            maximum_cached_patches = FOREST_MAX_CACHED_PATCHES,
            maximum_draw_instances = FOREST_MAX_DRAW_INSTANCES,
            coarse_global_beam_locators = coarse_beam_anchors.len(),
            global_beam_locators = beam_anchors.len(),
            beam_top_radius_meters = FOREST_BEAM_TOP_RADIUS_METERS,
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
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(&bind_group_layout),
                Some(weather_field_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader_source = forest_shader_source();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forest billboard shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
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
        let beam_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forest light beam shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("forest_beam.wgsl").into()),
        });
        let beam_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forest light beam pipeline layout"),
            bind_group_layouts: &[Some(camera_bind_group_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let beam_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("forest light beam pipeline"),
            layout: Some(&beam_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &beam_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[ForestBeamVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &beam_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffer_instances =
            vec![<TreeInstance as bytemuck::Zeroable>::zeroed(); FOREST_MAX_DRAW_INSTANCES];
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest tree instances"),
            contents: bytemuck::cast_slice(&buffer_instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let beam_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forest light beam vertices"),
            contents: bytemuck::cast_slice(&beam_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Self {
            pipeline,
            beam_pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            beam_vertex_buffer,
            instance_count: 0,
            beam_vertex_count: beam_vertices.len() as u32,
            patches: BTreeMap::new(),
            pending_patch: Some(PendingForestPatch::new(initial_key)),
            draw_instances: Vec::new(),
            lod_counts: TreeLodCounts::default(),
            empty_patch_keys: BTreeSet::new(),
            primary_patch_key: Some(initial_key),
            rebuild_count: 0,
            enabled,
            beams_enabled,
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
        let local_surface_height_meters = terrain
            .surface_height_meters_at(camera_direction, camera_altitude_meters)
            .unwrap_or(0.0)
            .max(0.0);
        let renderable_keys = forest_renderable_cell_keys(
            camera_planet_position,
            viewport_height,
            vertical_fov_radians,
            local_surface_height_meters,
        );
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
                viewport_height,
                vertical_fov_radians,
            );
            return;
        }
        self.primary_patch_key = renderable_keys
            .first()
            .or_else(|| cached_keys.first())
            .copied();
        let renderable_key_set = renderable_keys.iter().copied().collect::<BTreeSet<_>>();
        let cached_key_set = cached_keys.iter().copied().collect::<BTreeSet<_>>();
        self.patches.retain(|key, _| cached_key_set.contains(key));
        self.empty_patch_keys
            .retain(|key| cached_key_set.contains(key));
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
        if self.pending_patch.is_none() {
            if let Some(key) = missing_renderable_key.or_else(|| {
                cached_keys.iter().copied().find(|key| {
                    !self.patches.contains_key(key) && !self.empty_patch_keys.contains(key)
                })
            }) {
                self.pending_patch = Some(PendingForestPatch::new(key));
            }
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
                target: "catinthegarden::forest",
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
            viewport_height,
            vertical_fov_radians,
        );
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
    ) {
        let mut patches = self.patches.values().collect::<Vec<_>>();
        patches.sort_by(|left, right| {
            right
                .centre_direction
                .dot(camera_planet_position)
                .total_cmp(&left.centre_direction.dot(camera_planet_position))
        });
        let mut draw_instances = Vec::with_capacity(
            self.draw_instances
                .len()
                .max(TREE_COUNT)
                .min(FOREST_MAX_DRAW_INSTANCES),
        );
        let mut lod_counts = TreeLodCounts::default();
        'patches: for patch in patches {
            let transition_progress =
                patch_transition_progress(patch.visible_since.elapsed().as_secs_f64()) as f32;
            for tree in &patch.trees {
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
                    lodded_tree_instance_with_scale(*tree, projected_pixels, transition_progress)
                {
                    draw_instances.push(tree);
                    if draw_instances.len() == FOREST_MAX_DRAW_INSTANCES {
                        break 'patches;
                    }
                }
            }
        }
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
        self.lod_counts = lod_counts;
    }

    pub fn stats(&self) -> ForestStats {
        ForestStats {
            patch_count: self.patches.len().min(usize::from(u16::MAX)) as u16,
            beam_count: (self.beam_vertex_count / 6).min(u32::from(u16::MAX)) as u16,
            instances: self.instance_count,
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
            beams_enabled: self.beams_enabled,
        }
    }

    pub fn toggle_beams(&mut self) {
        self.beams_enabled = !self.beams_enabled;
        tracing::info!(
            target: "catinthegarden::forest",
            enabled = self.beams_enabled,
            "forest light beams toggled"
        );
    }

    pub fn draw_beams<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        camera_altitude_meters: f64,
    ) {
        if !self.enabled
            || !self.beams_enabled
            || !camera_altitude_meters.is_finite()
            || self.beam_vertex_count == 0
        {
            return;
        }
        render_pass.set_pipeline(&self.beam_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.beam_vertex_buffer.slice(..));
        render_pass.draw(0..self.beam_vertex_count, 0..1);
    }

    pub fn draw<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        weather_field_bind_group: &'pass wgpu::BindGroup,
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
        render_pass.set_bind_group(2, weather_field_bind_group, &[]);
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
    let tree_radius = PLANET_RADIUS_METERS
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
    let density = tree_lod_density(projected_pixels) * population_scale.clamp(0.0, 1.0);
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

fn global_forest_beam_anchors(samples: &[TerrainForestSample]) -> Vec<ForestBeamAnchor> {
    let start_base_radius = samples
        .iter()
        .filter(|sample| sample.direction.is_finite())
        .max_by(|left, right| {
            left.direction
                .dot(FOREST_CENTRE_DIRECTION)
                .total_cmp(&right.direction.dot(FOREST_CENTRE_DIRECTION))
        })
        .map(|sample| PLANET_RADIUS_METERS + sample.surface_elevation_meters.max(0.0))
        .unwrap_or(PLANET_RADIUS_METERS);
    let mut anchors = vec![ForestBeamAnchor {
        direction: FOREST_CENTRE_DIRECTION,
        base_radius_meters: start_base_radius,
    }];
    let mut candidates = samples
        .iter()
        .copied()
        .filter(|sample| {
            sample.direction.is_finite()
                && sample.surface_elevation_meters.is_finite()
                && sample.surface_elevation_meters > 0.0
                && sample.moisture.is_finite()
                && sample.moisture >= FOREST_MINIMUM_MOISTURE
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        forest_density_at(right.direction)
            .total_cmp(&forest_density_at(left.direction))
            .then_with(|| left.direction.x.total_cmp(&right.direction.x))
            .then_with(|| left.direction.y.total_cmp(&right.direction.y))
            .then_with(|| left.direction.z.total_cmp(&right.direction.z))
    });
    let minimum_angular_spacing = FOREST_BEAM_LOCATOR_SPACING_METERS / PLANET_RADIUS_METERS;
    for sample in candidates {
        let direction = sample.direction.normalize();
        if anchors
            .iter()
            .any(|anchor| anchor.direction.angle_between(direction).abs() < minimum_angular_spacing)
        {
            continue;
        }
        anchors.push(ForestBeamAnchor {
            direction,
            base_radius_meters: PLANET_RADIUS_METERS + sample.surface_elevation_meters,
        });
    }
    anchors
}

fn refine_global_forest_beam_anchor(
    coarse_anchor: ForestBeamAnchor,
    mut eligible_height_at: impl FnMut(DVec3) -> Option<f64>,
) -> Option<ForestBeamAnchor> {
    let key = forest_cell_key(coarse_anchor.direction);
    forest_patch_tree_layouts(key)
        .into_iter()
        .take(FOREST_BEAM_REFINEMENT_CANDIDATES)
        .find_map(|(direction, layout)| {
            let placement_density =
                forest_density_at(direction) * forest_cell_edge_falloff(key, direction);
            if f64::from(layout.seed) > placement_density {
                return None;
            }
            eligible_height_at(direction).map(|height_meters| ForestBeamAnchor {
                direction,
                base_radius_meters: PLANET_RADIUS_METERS + height_meters,
            })
        })
}

fn forest_beam_vertices(anchor: ForestBeamAnchor) -> [ForestBeamVertex; 6] {
    let direction = anchor.direction.normalize().as_vec3().to_array();
    let direction_and_base_radius = [
        direction[0],
        direction[1],
        direction[2],
        anchor.base_radius_meters as f32,
    ];
    [
        ForestBeamVertex {
            direction_and_base_radius,
            uv: [0.0, 0.0],
        },
        ForestBeamVertex {
            direction_and_base_radius,
            uv: [1.0, 0.0],
        },
        ForestBeamVertex {
            direction_and_base_radius,
            uv: [1.0, 1.0],
        },
        ForestBeamVertex {
            direction_and_base_radius,
            uv: [0.0, 0.0],
        },
        ForestBeamVertex {
            direction_and_base_radius,
            uv: [1.0, 1.0],
        },
        ForestBeamVertex {
            direction_and_base_radius,
            uv: [0.0, 1.0],
        },
    ]
}

fn forest_beam_vertices_for_anchors(anchors: &[ForestBeamAnchor]) -> Vec<ForestBeamVertex> {
    anchors
        .iter()
        .copied()
        .flat_map(forest_beam_vertices)
        .collect()
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
        let camera_position = FOREST_CENTRE_DIRECTION * (PLANET_RADIUS_METERS + 2.0);
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
    fn prefetch_range_contains_the_entire_draw_range_before_approach() {
        let camera_position = FOREST_CENTRE_DIRECTION * (PLANET_RADIUS_METERS + 2.0);
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
            direction * (PLANET_RADIUS_METERS + 2.0),
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
            * (PLANET_RADIUS_METERS + FOREST_TREE_RENDER_DISTANCE_METERS * 2.0);
        assert!(
            forest_renderable_cell_keys(camera_position, 427, 60.0_f64.to_radians(), 0.0)
                .is_empty()
        );
    }

    #[test]
    fn forest_search_follows_high_presented_terrain_instead_of_sea_level() {
        let local_surface_height_meters = 42_000.0;
        let camera_position =
            FOREST_CENTRE_DIRECTION * (PLANET_RADIUS_METERS + local_surface_height_meters + 2.0);
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
        assert!(FOREST_MAX_CACHED_PATCHES > FOREST_MAX_RENDERABLE_PATCHES);
        assert!(FOREST_MAX_DRAW_INSTANCES > TREE_COUNT);
        assert!(FOREST_MAX_DRAW_INSTANCES < TREE_COUNT * FOREST_MAX_RENDERABLE_PATCHES);
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
    fn forest_shader_is_a_depth_writing_procedural_billboard() {
        let shader = forest_shader_source();
        let module = wgpu::naga::front::wgsl::parse_str(&shader).expect("forest shader parses");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("forest shader validates");
        assert!(shader.contains("centre_and_height: vec4<f32>"));
        assert!(shader.contains("if !trunk && !canopy"));
        assert!(!shader.contains("texture_2d"));
    }

    #[test]
    fn forest_beam_shader_parses_and_is_depth_tested() {
        let shader = include_str!("forest_beam.wgsl");
        let module =
            wgpu::naga::front::wgsl::parse_str(shader).expect("forest beam shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("forest beam shader must validate");
        assert!(shader.contains("fn vs_main"));
        assert!(shader.contains("fn fs_main"));
        assert!(shader.contains("smoothstep(0.0, 0.24"));
        assert!(shader.contains("side * 0.0075"));
        assert!(shader.contains("discard;"));
    }

    #[test]
    fn forest_beams_span_from_the_forest_surface_to_atmosphere_top() {
        let anchor = ForestBeamAnchor {
            direction: DVec3::X,
            base_radius_meters: PLANET_RADIUS_METERS + 250.0,
        };
        let vertices = forest_beam_vertices(anchor);
        assert_eq!(vertices.len(), 6);
        assert!(vertices.iter().all(|vertex| {
            (f64::from(vertex.direction_and_base_radius[3]) - anchor.base_radius_meters).abs() < 0.5
        }));
        assert!(include_str!("forest_beam.wgsl").contains("let top = up * 6880000.0"));
        assert_eq!(FOREST_BEAM_TOP_RADIUS_METERS, 6_880_000.0);
    }

    #[test]
    fn global_locator_beams_are_deterministic_spaced_and_not_camera_local() {
        let samples = [
            TerrainForestSample {
                direction: DVec3::X,
                surface_elevation_meters: 250.0,
                moisture: 0.8,
            },
            TerrainForestSample {
                direction: DVec3::Z,
                surface_elevation_meters: 500.0,
                moisture: 0.8,
            },
            TerrainForestSample {
                direction: DVec3::NEG_X,
                surface_elevation_meters: 750.0,
                moisture: 0.8,
            },
        ];
        let anchors = global_forest_beam_anchors(&samples);
        assert_eq!(anchors, global_forest_beam_anchors(&samples));
        assert!(anchors.len() >= samples.len());
        assert_eq!(
            forest_beam_vertices_for_anchors(&anchors).len(),
            anchors.len() * 6
        );
        let minimum_angle = FOREST_BEAM_LOCATOR_SPACING_METERS / PLANET_RADIUS_METERS;
        for (index, anchor) in anchors.iter().enumerate() {
            assert!(anchors[index + 1..].iter().all(|other| {
                anchor.direction.angle_between(other.direction).abs() >= minimum_angle
            }));
        }
    }

    #[test]
    fn global_locator_refinement_requires_a_real_tree_eligible_point() {
        let coarse = ForestBeamAnchor {
            direction: DVec3::X,
            base_radius_meters: PLANET_RADIUS_METERS + 12_000.0,
        };
        let key = forest_cell_key(coarse.direction);
        let expected_direction = forest_patch_tree_layouts(key)
            .into_iter()
            .filter(|(direction, layout)| {
                f64::from(layout.seed)
                    <= forest_density_at(*direction) * forest_cell_edge_falloff(key, *direction)
            })
            .nth(4)
            .expect("the deterministic cell has qualifying density candidates")
            .0;
        let refined = refine_global_forest_beam_anchor(coarse, |direction| {
            (direction == expected_direction).then_some(630.0)
        })
        .expect("an eligible generated tree point becomes the locator");
        assert_eq!(refined.direction, expected_direction);
        assert_eq!(refined.base_radius_meters, PLANET_RADIUS_METERS + 630.0);
        assert!(refine_global_forest_beam_anchor(coarse, |_| None).is_none());
    }

    #[test]
    fn forest_shader_has_no_unconditional_night_light() {
        let shader = include_str!("forest.wgsl");
        assert!(shader.contains(
            "fn tree_lighting(solar_elevation_cosine: f32, cloud_visibility: f32) -> f32"
        ));
        assert!(shader.contains("smoothstep(-0.18, 0.02, solar_elevation_cosine) * 0.36"));
        assert!(shader.contains("return direct * cloud_visibility + sky_ambient;"));
        assert!(shader.contains("trunk_colour * 0.75 * input.lighting"));
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
        assert!(shader.contains("direct * cloud_visibility"));
        assert!(renderer.contains("weather_field_bind_group_layout"));
        assert!(renderer.contains("weather_field_bind_group"));
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

    #[test]
    fn forest_beams_are_off_by_default_and_toggleable_with_b() {
        let source = include_str!("forest.rs");
        let main = include_str!("main.rs");
        assert!(source.contains("beams_enabled: false"));
        assert!(source.contains("pub fn toggle_beams"));
        assert!(main.contains("KeyCode::KeyB"));
        assert!(main.contains("state.forest.toggle_beams()"));
    }
}

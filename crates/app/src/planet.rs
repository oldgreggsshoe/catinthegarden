use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

use glam::{DQuat, DVec3, Mat4, Vec3, Vec4};

pub const PLANET_RADIUS_METERS: f64 = 4_000_000.0;
pub const CHUNK_GRID_QUADS: usize = 32;
pub const CHUNK_GRID_VERTICES: usize = CHUNK_GRID_QUADS + 1;
pub const MAX_LOD_LEVEL: u8 = 18;
/// The coarsest rendered quadtree leaf. Screen-space error raises the LOD into
/// the globally available L3/L4 data only when that detail can affect pixels.
pub const MINIMUM_LOD_LEVEL: u8 = 2;
/// Deliberately game-time-scaled so axial rotation is visible during normal play.
pub const PLANET_ROTATION_PERIOD_SECONDS: f64 = 600.0;
pub const DEFAULT_MAX_ACTIVE_CHUNKS: usize = 1_024;
pub const SKIRT_DEPTH_RATIO: f64 = 0.075;
pub const PLACEHOLDER_HEIGHT_OCTAVES: [(f64, f64); 4] = [
    (8.0, 2_800.0),
    (512.0, 600.0),
    (32_768.0, 100.0),
    (2_097_152.0, 3.0),
];
pub const PLACEHOLDER_HEIGHT_AMPLITUDE_METERS: f64 = 3_503.0;
const DEFAULT_VERTICAL_FOV_RADIANS: f64 = 45.0_f64.to_radians();
const MIN_VERTICAL_FOV_RADIANS: f64 = 2.0_f64.to_radians();
const MAX_VERTICAL_FOV_RADIANS: f64 = 75.0_f64.to_radians();
const HIGH_DETAIL_ZOOM_VERTICAL_FOV_RADIANS: f64 = 8.0_f64.to_radians();
const HIGH_DETAIL_ZOOM_MINIMUM_LOD_LEVEL: u8 = 4;
pub const PLACEHOLDER_GEOMETRIC_ERROR_RATIO: f64 = 0.02;
pub const LOD_THRASH_WINDOW_UPDATES: u64 = 4;

const FACE_COUNT: u8 = 6;
const NODE_BOUNDS_SAMPLE_STEPS: usize = 4;
const FACE_BASES: [(DVec3, DVec3, DVec3); FACE_COUNT as usize] = [
    (DVec3::X, DVec3::NEG_Z, DVec3::Y),
    (DVec3::NEG_X, DVec3::Z, DVec3::Y),
    (DVec3::Y, DVec3::X, DVec3::NEG_Z),
    (DVec3::NEG_Y, DVec3::X, DVec3::Z),
    (DVec3::Z, DVec3::X, DVec3::Y),
    (DVec3::NEG_Z, DVec3::NEG_X, DVec3::Y),
];

/// A leaf in one of the six face-local quadtrees. Coordinates address a node
/// at `level`, so children are `(x * 2 + dx, y * 2 + dy)` at `level + 1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct QuadtreeNode {
    pub face: u8,
    pub level: u8,
    pub x: u32,
    pub y: u32,
}

impl QuadtreeNode {
    pub const fn root(face: u8) -> Self {
        Self {
            face,
            level: 0,
            x: 0,
            y: 0,
        }
    }

    pub fn children(self) -> [Self; 4] {
        assert!(
            self.level < MAX_LOD_LEVEL,
            "maximum LOD node has no children"
        );
        let level = self.level + 1;
        let x = self.x * 2;
        let y = self.y * 2;
        [
            Self {
                face: self.face,
                level,
                x,
                y,
            },
            Self {
                face: self.face,
                level,
                x: x + 1,
                y,
            },
            Self {
                face: self.face,
                level,
                x,
                y: y + 1,
            },
            Self {
                face: self.face,
                level,
                x: x + 1,
                y: y + 1,
            },
        ]
    }

    pub fn parent(self) -> Option<Self> {
        (self.level > 0).then(|| Self {
            face: self.face,
            level: self.level - 1,
            x: self.x / 2,
            y: self.y / 2,
        })
    }

    pub fn is_valid(self) -> bool {
        if self.face >= FACE_COUNT || self.level > MAX_LOD_LEVEL {
            return false;
        }
        let nodes_per_axis = 1_u32 << self.level;
        self.x < nodes_per_axis && self.y < nodes_per_axis
    }

    pub fn uv_bounds(self) -> [f64; 4] {
        assert!(self.is_valid(), "invalid quadtree node {self:?}");
        let nodes_per_axis = (1_u64 << self.level) as f64;
        let size = 2.0 / nodes_per_axis;
        let u_min = -1.0 + f64::from(self.x) * size;
        let v_min = -1.0 + f64::from(self.y) * size;
        [u_min, v_min, u_min + size, v_min + size]
    }

    pub fn center_direction(self) -> DVec3 {
        let [u_min, v_min, u_max, v_max] = self.uv_bounds();
        cube_face_direction(self.face, (u_min + u_max) * 0.5, (v_min + v_max) * 0.5)
    }

    pub fn geometric_error_meters(self) -> f64 {
        let root_triangle_spacing =
            PLANET_RADIUS_METERS * std::f64::consts::FRAC_PI_2 / CHUNK_GRID_QUADS as f64;
        // Triangle spacing is resolution, not approximation error. For the smooth analytic
        // placeholder, 2% of the edge is a conservative combined curvature and unresolved-sine
        // bound. Phase 4 replaces this with the baker's per-tile measured geometric error.
        root_triangle_spacing * PLACEHOLDER_GEOMETRIC_ERROR_RATIO / f64::from(1_u32 << self.level)
    }
}

pub fn cube_face_basis(face: u8) -> (DVec3, DVec3, DVec3) {
    FACE_BASES
        .get(face as usize)
        .copied()
        .unwrap_or_else(|| panic!("invalid cube face {face}"))
}

pub fn cube_face_direction(face: u8, u: f64, v: f64) -> DVec3 {
    let (normal, tangent_u, tangent_v) = cube_face_basis(face);
    (normal + tangent_u * u + tangent_v * v).normalize()
}

pub fn planet_rotation_radians(sim_time_seconds: f64) -> f64 {
    (sim_time_seconds * std::f64::consts::TAU / PLANET_ROTATION_PERIOD_SECONDS)
        .rem_euclid(std::f64::consts::TAU)
}

/// Expresses a world-space vector in the rotating planet's local frame. The
/// renderer keeps all terrain/outmap data in this local frame and transforms
/// the f64 camera and sun inputs into it each frame.
pub fn planet_local_vector(world_vector: DVec3, planet_rotation_radians: f64) -> DVec3 {
    DQuat::from_rotation_y(-planet_rotation_radians).mul_vec3(world_vector)
}

pub fn placeholder_height_meters(direction: DVec3) -> f64 {
    PLACEHOLDER_HEIGHT_OCTAVES
        .iter()
        .map(|(frequency, amplitude_meters)| {
            let wave = (frequency * direction.x).sin() - direction.x * frequency.sin()
                + (1.375 * frequency * direction.y).sin()
                + (1.75 * frequency * direction.z).sin();
            amplitude_meters * wave * 0.25
        })
        .sum()
}

/// Screen-space LOD policy shared by all face trees. The hysteresis band
/// prevents a node from split/merge thrashing at the split boundary.
pub struct LodPolicy {
    pub split_pixels: f64,
    pub merge_pixels: f64,
    pub max_level: u8,
}

impl Default for LodPolicy {
    fn default() -> Self {
        Self {
            split_pixels: 2.0,
            merge_pixels: 1.25,
            max_level: MAX_LOD_LEVEL,
        }
    }
}

impl LodPolicy {
    fn minimum_level(&self) -> u8 {
        MINIMUM_LOD_LEVEL.min(self.max_level)
    }

    fn minimum_level_for_view(&self, vertical_fov_radians: f64) -> u8 {
        let normal_minimum = self.minimum_level();
        if vertical_fov_radians <= HIGH_DETAIL_ZOOM_VERTICAL_FOV_RADIANS {
            normal_minimum.max(HIGH_DETAIL_ZOOM_MINIMUM_LOD_LEVEL.min(self.max_level))
        } else {
            normal_minimum
        }
    }

    pub fn should_split(&self, projected_error_pixels: f64, level: u8) -> bool {
        level < self.max_level && projected_error_pixels > self.split_pixels
    }

    pub fn should_merge(&self, projected_error_pixels: f64, level: u8) -> bool {
        level >= self.minimum_level() && projected_error_pixels < self.merge_pixels
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NodeBounds {
    pub center_world: DVec3,
    pub radius_meters: f64,
}

pub fn node_bounds(node: QuadtreeNode) -> NodeBounds {
    let [u_min, v_min, u_max, v_max] = node.uv_bounds();
    let center_direction = node.center_direction();
    let center_world =
        center_direction * (PLANET_RADIUS_METERS + placeholder_height_meters(center_direction));
    let mut radius_meters: f64 = 0.0;
    for y in 0..=NODE_BOUNDS_SAMPLE_STEPS {
        let v_fraction = y as f64 / NODE_BOUNDS_SAMPLE_STEPS as f64;
        let v = v_min + (v_max - v_min) * v_fraction;
        for x in 0..=NODE_BOUNDS_SAMPLE_STEPS {
            let u_fraction = x as f64 / NODE_BOUNDS_SAMPLE_STEPS as f64;
            let u = u_min + (u_max - u_min) * u_fraction;
            let direction = cube_face_direction(node.face, u, v);
            let world = direction * (PLANET_RADIUS_METERS + placeholder_height_meters(direction));
            radius_meters = radius_meters.max(world.distance(center_world));
        }
    }
    NodeBounds {
        center_world,
        // The sampled displaced surface captures local height variation without inflating every
        // tiny node by the global height range. The unresolved geometric-error margin remains
        // conservative between samples and is replaced by measured tile bounds in Phase 4.
        radius_meters: radius_meters + node.geometric_error_meters(),
    }
}

pub fn node_is_above_horizon(node: QuadtreeNode, camera_world: DVec3) -> bool {
    let bounds = node_bounds(node);
    camera_world.dot(bounds.center_world) + camera_world.length() * bounds.radius_meters
        >= PLANET_RADIUS_METERS * PLANET_RADIUS_METERS
}

fn node_is_in_view_frustum(
    node: QuadtreeNode,
    camera_world: DVec3,
    camera_forward: DVec3,
    aspect_ratio: f64,
    vertical_fov_radians: f64,
) -> bool {
    let bounds = node_bounds(node);
    let camera_to_center = bounds.center_world - camera_world;
    let forward = camera_forward.normalize();
    let forward_distance = camera_to_center.dot(forward);
    if forward_distance < -bounds.radius_meters {
        return false;
    }

    // Match the renderer's look-to basis, but retain a stable fallback for a
    // camera that is almost parallel to its nominal up vector.
    let right = forward.cross(DVec3::Y).normalize_or_zero();
    let right = if right.length_squared() > 0.0 {
        right
    } else {
        forward.cross(DVec3::X).normalize()
    };
    let up = right.cross(forward).normalize();
    let vertical_tangent = (vertical_fov_radians * 0.5).tan();
    let horizontal_tangent = vertical_tangent * aspect_ratio;

    // Test the node's conservative bounding sphere against all four side
    // planes. This intentionally retains boundary chunks so a narrow optical
    // zoom cannot create a visible hole at the viewport edge.
    for (axis, tangent) in [(right, horizontal_tangent), (up, vertical_tangent)] {
        let normal_length = (1.0 + tangent * tangent).sqrt();
        let plane_distance = forward_distance * tangent;
        let side_distance = camera_to_center.dot(axis);
        if plane_distance + side_distance < -bounds.radius_meters * normal_length
            || plane_distance - side_distance < -bounds.radius_meters * normal_length
        {
            return false;
        }
    }
    true
}

pub fn projected_error_pixels(
    node: QuadtreeNode,
    camera_world: DVec3,
    viewport_height: u32,
    vertical_fov_radians: f64,
) -> f64 {
    let bounds = node_bounds(node);
    let distance = (camera_world.distance(bounds.center_world) - bounds.radius_meters).max(1.0);
    let projection_scale = f64::from(viewport_height.max(1))
        / (2.0 * (vertical_fov_radians.clamp(0.01, std::f64::consts::PI - 0.01) * 0.5).tan());
    node.geometric_error_meters() * projection_scale / distance
}

#[derive(Clone, Debug)]
pub struct LodMetrics {
    pub level_histogram: [u32; MAX_LOD_LEVEL as usize + 1],
    pub active_chunks: u32,
    pub chunks_loaded: u32,
    pub chunks_unloaded: u32,
    pub splits: u32,
    pub merges: u32,
    pub lod_thrash_events: u32,
    pub culled_nodes: u32,
    pub max_level: u8,
    pub max_seam_delta_meters: f64,
    pub budget_limited: bool,
}

impl Default for LodMetrics {
    fn default() -> Self {
        Self {
            level_histogram: [0; MAX_LOD_LEVEL as usize + 1],
            active_chunks: 0,
            chunks_loaded: 0,
            chunks_unloaded: 0,
            splits: 0,
            merges: 0,
            lod_thrash_events: 0,
            culled_nodes: 0,
            max_level: 0,
            max_seam_delta_meters: 0.0,
            budget_limited: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LodUpdate {
    pub active_nodes: Vec<QuadtreeNode>,
    pub loaded_nodes: Vec<QuadtreeNode>,
    pub unloaded_nodes: Vec<QuadtreeNode>,
    pub metrics: LodMetrics,
}

#[derive(Clone, Copy)]
struct NodeEvaluation {
    visible: bool,
    projected_error_pixels: f64,
}

struct SplitCandidate {
    node: QuadtreeNode,
    priority: f64,
    visible_children: Vec<QuadtreeNode>,
}

struct SeamSample {
    position: DVec3,
    references: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LodTransitionKind {
    Split,
    Merge,
}

struct RecentLodTransition {
    kind: LodTransitionKind,
    update_index: u64,
}

#[derive(Clone, Copy, PartialEq)]
struct SelectionInput {
    camera_world: DVec3,
    camera_forward: Option<DVec3>,
    aspect_ratio: f64,
    viewport_height: u32,
    vertical_fov_radians: f64,
}

impl PartialEq for SplitCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node && self.priority.total_cmp(&other.priority) == Ordering::Equal
    }
}

impl Eq for SplitCandidate {}

impl PartialOrd for SplitCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SplitCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .total_cmp(&other.priority)
            .then_with(|| self.node.cmp(&other.node))
    }
}

pub struct PlanetLod {
    policy: LodPolicy,
    max_active_chunks: usize,
    split_nodes: HashSet<QuadtreeNode>,
    active_nodes: Vec<QuadtreeNode>,
    seam_samples: HashMap<[i64; 3], SeamSample>,
    max_seam_delta_meters: f64,
    last_selection_input: Option<SelectionInput>,
    last_metrics: LodMetrics,
    update_index: u64,
    recent_lod_transitions: HashMap<QuadtreeNode, RecentLodTransition>,
}

impl Default for PlanetLod {
    fn default() -> Self {
        Self::new(LodPolicy::default(), DEFAULT_MAX_ACTIVE_CHUNKS)
    }
}

impl PlanetLod {
    pub fn new(mut policy: LodPolicy, max_active_chunks: usize) -> Self {
        policy.max_level = policy.max_level.min(MAX_LOD_LEVEL);
        assert!(policy.merge_pixels < policy.split_pixels);
        assert!(max_active_chunks >= FACE_COUNT as usize);
        Self {
            policy,
            max_active_chunks,
            split_nodes: HashSet::new(),
            active_nodes: Vec::new(),
            seam_samples: HashMap::new(),
            max_seam_delta_meters: 0.0,
            last_selection_input: None,
            last_metrics: LodMetrics::default(),
            update_index: 0,
            recent_lod_transitions: HashMap::new(),
        }
    }

    pub fn active_nodes(&self) -> &[QuadtreeNode] {
        &self.active_nodes
    }

    pub fn is_split(&self, node: QuadtreeNode) -> bool {
        self.split_nodes.contains(&node)
    }

    pub fn update(
        &mut self,
        camera_world: DVec3,
        viewport_height: u32,
        vertical_fov_radians: f64,
    ) -> LodUpdate {
        self.update_internal(
            camera_world,
            None,
            1.0,
            viewport_height,
            vertical_fov_radians,
        )
    }

    pub fn update_for_view(
        &mut self,
        camera_world: DVec3,
        camera_forward: DVec3,
        aspect_ratio: f64,
        viewport_height: u32,
        vertical_fov_radians: f64,
    ) -> LodUpdate {
        assert!(camera_world.is_finite());
        assert!(camera_world.length() > PLANET_RADIUS_METERS);
        assert!(camera_forward.is_finite() && camera_forward.length_squared() > 0.0);
        assert!(aspect_ratio.is_finite() && aspect_ratio > 0.0);
        self.update_internal(
            camera_world,
            Some(camera_forward.normalize()),
            aspect_ratio,
            viewport_height,
            vertical_fov_radians,
        )
    }

    fn update_internal(
        &mut self,
        camera_world: DVec3,
        camera_forward: Option<DVec3>,
        aspect_ratio: f64,
        viewport_height: u32,
        vertical_fov_radians: f64,
    ) -> LodUpdate {
        assert!(camera_world.is_finite());
        assert!(camera_world.length() > PLANET_RADIUS_METERS);
        assert!(aspect_ratio.is_finite() && aspect_ratio > 0.0);
        assert!(vertical_fov_radians.is_finite() && vertical_fov_radians > 0.0);
        self.update_index += 1;
        self.recent_lod_transitions.retain(|_, transition| {
            self.update_index.saturating_sub(transition.update_index) <= LOD_THRASH_WINDOW_UPDATES
        });
        let selection_input = SelectionInput {
            camera_world,
            camera_forward,
            aspect_ratio,
            viewport_height,
            vertical_fov_radians,
        };
        if self.last_selection_input == Some(selection_input) {
            let mut metrics = self.last_metrics.clone();
            metrics.chunks_loaded = 0;
            metrics.chunks_unloaded = 0;
            metrics.splits = 0;
            metrics.merges = 0;
            metrics.lod_thrash_events = 0;
            metrics.culled_nodes = 0;
            return LodUpdate {
                active_nodes: self.active_nodes.clone(),
                loaded_nodes: Vec::new(),
                unloaded_nodes: Vec::new(),
                metrics,
            };
        }

        let previous_active: HashSet<_> = self.active_nodes.iter().copied().collect();
        let previous_split = self.split_nodes.clone();
        let mut evaluations = HashMap::new();
        let mut culled_nodes = 0_u32;
        let mut leaves = HashSet::with_capacity(self.max_active_chunks);
        for face in 0..FACE_COUNT {
            let root = QuadtreeNode::root(face);
            if Self::evaluate(
                root,
                camera_world,
                camera_forward,
                aspect_ratio,
                viewport_height,
                vertical_fov_radians,
                &mut evaluations,
                &mut culled_nodes,
            )
            .visible
            {
                leaves.insert(root);
            }
        }

        let mut next_split = HashSet::new();
        let mut budget_limited = false;
        let mut candidates = BinaryHeap::new();
        for root in leaves.iter().copied() {
            if let Some(candidate) = Self::split_candidate(
                &self.policy,
                root,
                &previous_split,
                camera_world,
                camera_forward,
                aspect_ratio,
                viewport_height,
                vertical_fov_radians,
                &mut evaluations,
                &mut culled_nodes,
            ) {
                candidates.push(candidate);
            }
        }
        while let Some(candidate) = candidates.pop() {
            if !leaves.contains(&candidate.node) {
                continue;
            }
            let next_len = leaves.len() - 1 + candidate.visible_children.len();
            if next_len > self.max_active_chunks {
                budget_limited = true;
                break;
            }
            leaves.remove(&candidate.node);
            next_split.insert(candidate.node);
            for child in candidate.visible_children {
                leaves.insert(child);
                if let Some(child_candidate) = Self::split_candidate(
                    &self.policy,
                    child,
                    &previous_split,
                    camera_world,
                    camera_forward,
                    aspect_ratio,
                    viewport_height,
                    vertical_fov_radians,
                    &mut evaluations,
                    &mut culled_nodes,
                ) {
                    candidates.push(child_candidate);
                }
            }
        }

        if camera_forward.is_some() {
            // A large parent sphere can graze the viewport even when every
            // tighter child bound is outside it. Such a parent must not remain
            // as a coarse rendered leaf: it contributes no pixels, defeats the
            // minimum LOD policy, and becomes especially expensive at a narrow
            // optical zoom.
            leaves.retain(|node| {
                node.level >= self.policy.minimum_level_for_view(vertical_fov_radians)
                    || node.children().into_iter().any(|child| {
                        Self::evaluate(
                            child,
                            camera_world,
                            camera_forward,
                            aspect_ratio,
                            viewport_height,
                            vertical_fov_radians,
                            &mut evaluations,
                            &mut culled_nodes,
                        )
                        .visible
                    })
            });
        }

        let mut leaves: Vec<_> = leaves.into_iter().collect();
        leaves.sort_unstable();
        let next_active: HashSet<_> = leaves.iter().copied().collect();
        let mut loaded_nodes: Vec<_> = next_active.difference(&previous_active).copied().collect();
        let mut unloaded_nodes: Vec<_> =
            previous_active.difference(&next_active).copied().collect();
        loaded_nodes.sort_unstable();
        unloaded_nodes.sort_unstable();
        self.update_seam_metrics(&loaded_nodes, &unloaded_nodes);

        let split_transitions: Vec<_> = next_split.difference(&previous_split).copied().collect();
        let merge_transitions: Vec<_> = previous_split
            .difference(&next_split)
            .copied()
            .filter(|node| {
                let evaluation = Self::evaluate(
                    *node,
                    camera_world,
                    camera_forward,
                    aspect_ratio,
                    viewport_height,
                    vertical_fov_radians,
                    &mut evaluations,
                    &mut culled_nodes,
                );
                evaluation.visible
                    && node.level >= self.policy.minimum_level_for_view(vertical_fov_radians)
                    && self
                        .policy
                        .should_merge(evaluation.projected_error_pixels, node.level)
            })
            .collect();
        let lod_thrash_events = self.record_lod_transitions(&split_transitions, &merge_transitions);

        let mut metrics = LodMetrics {
            active_chunks: leaves.len() as u32,
            chunks_loaded: loaded_nodes.len() as u32,
            chunks_unloaded: unloaded_nodes.len() as u32,
            splits: split_transitions.len() as u32,
            merges: merge_transitions.len() as u32,
            lod_thrash_events,
            culled_nodes,
            max_seam_delta_meters: self.max_seam_delta_meters,
            budget_limited,
            ..LodMetrics::default()
        };
        for node in &leaves {
            metrics.level_histogram[node.level as usize] += 1;
            metrics.max_level = metrics.max_level.max(node.level);
        }

        self.split_nodes = next_split;
        self.active_nodes = leaves.clone();
        self.last_selection_input = Some(selection_input);
        self.last_metrics = metrics.clone();
        LodUpdate {
            active_nodes: leaves,
            loaded_nodes,
            unloaded_nodes,
            metrics,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn split_candidate(
        policy: &LodPolicy,
        node: QuadtreeNode,
        previous_split: &HashSet<QuadtreeNode>,
        camera_world: DVec3,
        camera_forward: Option<DVec3>,
        aspect_ratio: f64,
        viewport_height: u32,
        vertical_fov_radians: f64,
        evaluations: &mut HashMap<QuadtreeNode, NodeEvaluation>,
        culled_nodes: &mut u32,
    ) -> Option<SplitCandidate> {
        if node.level >= policy.max_level {
            return None;
        }
        let evaluation = Self::evaluate(
            node,
            camera_world,
            camera_forward,
            aspect_ratio,
            viewport_height,
            vertical_fov_radians,
            evaluations,
            culled_nodes,
        );
        let refine = if node.level < policy.minimum_level_for_view(vertical_fov_radians) {
            true
        } else if previous_split.contains(&node) {
            !policy.should_merge(evaluation.projected_error_pixels, node.level)
        } else {
            policy.should_split(evaluation.projected_error_pixels, node.level)
        };
        if !refine {
            return None;
        }
        let visible_children: Vec<_> = node
            .children()
            .into_iter()
            .filter(|child| {
                Self::evaluate(
                    *child,
                    camera_world,
                    camera_forward,
                    aspect_ratio,
                    viewport_height,
                    vertical_fov_radians,
                    evaluations,
                    culled_nodes,
                )
                .visible
            })
            .collect();
        if visible_children.is_empty() {
            return None;
        }
        // Once the global leaf budget is approached, favour the nearest/deepest demand
        // instead of breadth-refining the entire horizon at a lower level. Multiplying by
        // 2^level removes the nominal level-halving from geometric error, leaving camera
        // distance as the dominant priority signal.
        let priority = evaluation.projected_error_pixels * f64::from(1_u32 << node.level);
        Some(SplitCandidate {
            node,
            priority,
            visible_children,
        })
    }

    fn evaluate(
        node: QuadtreeNode,
        camera_world: DVec3,
        camera_forward: Option<DVec3>,
        aspect_ratio: f64,
        viewport_height: u32,
        vertical_fov_radians: f64,
        evaluations: &mut HashMap<QuadtreeNode, NodeEvaluation>,
        culled_nodes: &mut u32,
    ) -> NodeEvaluation {
        if let Some(evaluation) = evaluations.get(&node) {
            return *evaluation;
        }
        let visible = node_is_above_horizon(node, camera_world)
            && camera_forward.map_or(true, |camera_forward| {
                node_is_in_view_frustum(
                    node,
                    camera_world,
                    camera_forward,
                    aspect_ratio,
                    vertical_fov_radians,
                )
            });
        if !visible {
            *culled_nodes += 1;
        }
        let evaluation = NodeEvaluation {
            visible,
            projected_error_pixels: projected_error_pixels(
                node,
                camera_world,
                viewport_height,
                vertical_fov_radians,
            ),
        };
        evaluations.insert(node, evaluation);
        evaluation
    }

    fn record_lod_transitions(
        &mut self,
        split_nodes: &[QuadtreeNode],
        merge_nodes: &[QuadtreeNode],
    ) -> u32 {
        let mut thrash_events = 0;
        for (nodes, kind) in [
            (split_nodes, LodTransitionKind::Split),
            (merge_nodes, LodTransitionKind::Merge),
        ] {
            for node in nodes {
                if self
                    .recent_lod_transitions
                    .get(node)
                    .is_some_and(|previous| {
                        previous.kind != kind
                            && self.update_index.saturating_sub(previous.update_index)
                                <= LOD_THRASH_WINDOW_UPDATES
                    })
                {
                    thrash_events += 1;
                }
                self.recent_lod_transitions.insert(
                    *node,
                    RecentLodTransition {
                        kind,
                        update_index: self.update_index,
                    },
                );
            }
        }
        thrash_events
    }

    fn update_seam_metrics(
        &mut self,
        loaded_nodes: &[QuadtreeNode],
        unloaded_nodes: &[QuadtreeNode],
    ) {
        for node in unloaded_nodes {
            for (key, _) in node_boundary_samples(*node) {
                if let std::collections::hash_map::Entry::Occupied(mut entry) =
                    self.seam_samples.entry(key)
                {
                    if entry.get().references > 1 {
                        entry.get_mut().references -= 1;
                    } else {
                        entry.remove();
                    }
                }
            }
        }
        for node in loaded_nodes {
            for (key, position) in node_boundary_samples(*node) {
                if let Some(existing) = self.seam_samples.get_mut(&key) {
                    self.max_seam_delta_meters = self
                        .max_seam_delta_meters
                        .max(existing.position.distance(position));
                    existing.references += 1;
                } else {
                    self.seam_samples.insert(
                        key,
                        SeamSample {
                            position,
                            references: 1,
                        },
                    );
                }
            }
        }
    }
}

fn node_boundary_samples(node: QuadtreeNode) -> Vec<([i64; 3], DVec3)> {
    let [u_min, v_min, u_max, v_max] = node.uv_bounds();
    let mut samples = Vec::with_capacity(4 * CHUNK_GRID_QUADS);
    let mut push_sample = |u: f64, v: f64| {
        let direction = cube_face_direction(node.face, u, v);
        let position = direction * (PLANET_RADIUS_METERS + placeholder_height_meters(direction));
        let key = [
            (direction.x * 1.0e10).round() as i64,
            (direction.y * 1.0e10).round() as i64,
            (direction.z * 1.0e10).round() as i64,
        ];
        samples.push((key, position));
    };
    for step in 0..=CHUNK_GRID_QUADS {
        let fraction = step as f64 / CHUNK_GRID_QUADS as f64;
        push_sample(u_min + (u_max - u_min) * fraction, v_min);
    }
    for step in 1..=CHUNK_GRID_QUADS {
        let fraction = step as f64 / CHUNK_GRID_QUADS as f64;
        push_sample(u_max, v_min + (v_max - v_min) * fraction);
    }
    for step in (0..CHUNK_GRID_QUADS).rev() {
        let fraction = step as f64 / CHUNK_GRID_QUADS as f64;
        push_sample(u_min + (u_max - u_min) * fraction, v_max);
    }
    for step in (1..CHUNK_GRID_QUADS).rev() {
        let fraction = step as f64 / CHUNK_GRID_QUADS as f64;
        push_sample(u_min, v_min + (v_max - v_min) * fraction);
    }
    samples
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChunkVertex {
    pub anchor_relative_position: [f32; 3],
    pub sphere_direction: [f32; 3],
    pub tile_uv: [f32; 2],
    pub skirt_depth_meters: f32,
}

impl ChunkVertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x2,
        3 => Float32
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct ChunkMesh {
    pub node: QuadtreeNode,
    pub anchor_world: DVec3,
    pub vertices: Vec<ChunkVertex>,
    pub indices: Vec<u32>,
    pub edge_length_meters: f64,
    pub skirt_depth_meters: f64,
}

impl ChunkMesh {
    pub fn anchor_relative_to_camera(&self, camera_world: DVec3) -> [f32; 3] {
        (self.anchor_world - camera_world).as_vec3().to_array()
    }

    pub fn vertex_world_position(&self, vertex_index: usize, displaced: bool) -> DVec3 {
        let vertex = self.vertices[vertex_index];
        let direction = DVec3::from_array(vertex.sphere_direction.map(f64::from));
        let base =
            self.anchor_world + DVec3::from_array(vertex.anchor_relative_position.map(f64::from));
        let height = if displaced {
            placeholder_height_meters(direction)
        } else {
            0.0
        };
        base + direction * (height - f64::from(vertex.skirt_depth_meters))
    }
}

pub fn build_chunk_mesh(node: QuadtreeNode) -> ChunkMesh {
    assert!(node.is_valid(), "invalid quadtree node {node:?}");
    let [u_min, v_min, u_max, v_max] = node.uv_bounds();
    let anchor_world = node.center_direction() * PLANET_RADIUS_METERS;
    let corners = [
        cube_face_direction(node.face, u_min, v_min) * PLANET_RADIUS_METERS,
        cube_face_direction(node.face, u_max, v_min) * PLANET_RADIUS_METERS,
        cube_face_direction(node.face, u_max, v_max) * PLANET_RADIUS_METERS,
        cube_face_direction(node.face, u_min, v_max) * PLANET_RADIUS_METERS,
    ];
    let edge_length_meters = corners[0]
        .distance(corners[1])
        .max(corners[1].distance(corners[2]))
        .max(corners[2].distance(corners[3]))
        .max(corners[3].distance(corners[0]));
    let skirt_depth_meters = edge_length_meters * SKIRT_DEPTH_RATIO;
    let top_vertex_count = CHUNK_GRID_VERTICES * CHUNK_GRID_VERTICES;
    let skirt_vertex_count = 4 * CHUNK_GRID_VERTICES;
    let mut vertices = Vec::with_capacity(top_vertex_count + skirt_vertex_count);
    let mut indices =
        Vec::with_capacity(CHUNK_GRID_QUADS * CHUNK_GRID_QUADS * 6 + 4 * CHUNK_GRID_QUADS * 6);

    for y in 0..CHUNK_GRID_VERTICES {
        let v_fraction = y as f64 / CHUNK_GRID_QUADS as f64;
        let v = v_min + (v_max - v_min) * v_fraction;
        for x in 0..CHUNK_GRID_VERTICES {
            let u_fraction = x as f64 / CHUNK_GRID_QUADS as f64;
            let u = u_min + (u_max - u_min) * u_fraction;
            let direction = cube_face_direction(node.face, u, v);
            let world = direction * PLANET_RADIUS_METERS;
            vertices.push(ChunkVertex {
                anchor_relative_position: (world - anchor_world).as_vec3().to_array(),
                sphere_direction: direction.as_vec3().to_array(),
                tile_uv: [u_fraction as f32, v_fraction as f32],
                skirt_depth_meters: 0.0,
            });
        }
    }
    for y in 0..CHUNK_GRID_QUADS {
        for x in 0..CHUNK_GRID_QUADS {
            let lower_left = (y * CHUNK_GRID_VERTICES + x) as u32;
            let lower_right = lower_left + 1;
            let upper_left = lower_left + CHUNK_GRID_VERTICES as u32;
            let upper_right = upper_left + 1;
            indices.extend_from_slice(&[
                lower_left,
                lower_right,
                upper_left,
                lower_right,
                upper_right,
                upper_left,
            ]);
        }
    }

    let bottom: Vec<_> = (0..CHUNK_GRID_VERTICES).map(|x| (x, 0)).collect();
    let right: Vec<_> = (0..CHUNK_GRID_VERTICES)
        .map(|y| (CHUNK_GRID_QUADS, y))
        .collect();
    let top: Vec<_> = (0..CHUNK_GRID_VERTICES)
        .rev()
        .map(|x| (x, CHUNK_GRID_QUADS))
        .collect();
    let left: Vec<_> = (0..CHUNK_GRID_VERTICES).rev().map(|y| (0, y)).collect();
    for edge in [bottom, right, top, left] {
        let skirt_start = vertices.len() as u32;
        for &(x, y) in &edge {
            let top_index = y * CHUNK_GRID_VERTICES + x;
            let mut skirt_vertex = vertices[top_index];
            skirt_vertex.skirt_depth_meters = skirt_depth_meters as f32;
            vertices.push(skirt_vertex);
        }
        for segment in 0..CHUNK_GRID_QUADS {
            let top_start = (edge[segment].1 * CHUNK_GRID_VERTICES + edge[segment].0) as u32;
            let top_end = (edge[segment + 1].1 * CHUNK_GRID_VERTICES + edge[segment + 1].0) as u32;
            let skirt_start_vertex = skirt_start + segment as u32;
            let skirt_end_vertex = skirt_start_vertex + 1;
            indices.extend_from_slice(&[
                top_start,
                skirt_start_vertex,
                top_end,
                top_end,
                skirt_start_vertex,
                skirt_end_vertex,
            ]);
        }
    }

    ChunkMesh {
        node,
        anchor_world,
        vertices,
        indices,
        edge_length_meters,
        skirt_depth_meters,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RebasedVertex {
    pub camera_relative_position: [f32; 3],
}

impl RebasedVertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct CubeSphereMesh {
    world_positions: Vec<DVec3>,
    indices: Vec<u32>,
}

impl CubeSphereMesh {
    pub fn new() -> Self {
        let faces = FACE_BASES;
        let vertices_per_face = CHUNK_GRID_VERTICES * CHUNK_GRID_VERTICES;
        let mut world_positions = Vec::with_capacity(faces.len() * vertices_per_face);
        let mut indices = Vec::with_capacity(faces.len() * CHUNK_GRID_QUADS * CHUNK_GRID_QUADS * 6);

        for (face_index, (normal, tangent_u, tangent_v)) in faces.into_iter().enumerate() {
            let face_start = (face_index * vertices_per_face) as u32;
            for y in 0..CHUNK_GRID_VERTICES {
                let v = y as f64 / CHUNK_GRID_QUADS as f64 * 2.0 - 1.0;
                for x in 0..CHUNK_GRID_VERTICES {
                    let u = x as f64 / CHUNK_GRID_QUADS as f64 * 2.0 - 1.0;
                    world_positions.push(
                        (normal + tangent_u * u + tangent_v * v).normalize() * PLANET_RADIUS_METERS,
                    );
                }
            }
            for y in 0..CHUNK_GRID_QUADS {
                for x in 0..CHUNK_GRID_QUADS {
                    let lower_left = face_start + (y * CHUNK_GRID_VERTICES + x) as u32;
                    let lower_right = lower_left + 1;
                    let upper_left = lower_left + CHUNK_GRID_VERTICES as u32;
                    let upper_right = upper_left + 1;
                    indices.extend_from_slice(&[
                        lower_left,
                        lower_right,
                        upper_left,
                        lower_right,
                        upper_right,
                        upper_left,
                    ]);
                }
            }
        }

        Self {
            world_positions,
            indices,
        }
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn rebased_vertices(&self, camera_world_position: DVec3) -> Vec<RebasedVertex> {
        let mut vertices = Vec::with_capacity(self.world_positions.len());
        self.rebase_into(camera_world_position, &mut vertices);
        vertices
    }

    pub fn rebase_into(&self, camera_world_position: DVec3, vertices: &mut Vec<RebasedVertex>) {
        vertices.clear();
        vertices.extend(self.world_positions.iter().map(|world_position| {
            RebasedVertex {
                camera_relative_position: (*world_position - camera_world_position)
                    .as_vec3()
                    .to_array(),
            }
        }));
    }

    #[cfg(test)]
    fn world_positions(&self) -> &[DVec3] {
        &self.world_positions
    }
}

pub struct OrbitCamera {
    pub azimuth_radians: f64,
    pub elevation_radians: f64,
    pub orbit_radius_meters: f64,
    vertical_fov_radians: f64,
    look_yaw_radians: f64,
    look_pitch_radians: f64,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        let mut camera = Self {
            azimuth_radians: 0.0,
            elevation_radians: 20.0_f64.to_radians(),
            orbit_radius_meters: 10_000_000.0,
            vertical_fov_radians: DEFAULT_VERTICAL_FOV_RADIANS,
            look_yaw_radians: 0.0,
            look_pitch_radians: 0.0,
        };
        camera.look_at_origin();
        camera
    }
}

impl OrbitCamera {
    pub fn world_position(&self) -> DVec3 {
        let horizontal_radius = self.orbit_radius_meters * self.elevation_radians.cos();
        DVec3::new(
            horizontal_radius * self.azimuth_radians.cos(),
            self.orbit_radius_meters * self.elevation_radians.sin(),
            horizontal_radius * self.azimuth_radians.sin(),
        )
    }

    pub fn view_projection(&self, aspect_ratio: f32) -> Mat4 {
        view_projection_for(
            self.direction(),
            self.orbit_radius_meters - PLANET_RADIUS_METERS,
            self.vertical_fov_radians,
            aspect_ratio,
        )
    }

    pub fn view_projection_in_planet_frame(
        &self,
        aspect_ratio: f32,
        planet_rotation_radians: f64,
    ) -> Mat4 {
        view_projection_for(
            self.planet_frame_direction(planet_rotation_radians),
            self.orbit_radius_meters - PLANET_RADIUS_METERS,
            self.vertical_fov_radians,
            aspect_ratio,
        )
    }

    pub fn set_world_pose(&mut self, position: DVec3, look_at: DVec3) {
        assert!(position.is_finite() && look_at.is_finite());
        let radius = position.length();
        assert!(
            radius > 0.0,
            "camera position must not be the planet origin"
        );
        let forward = look_at - position;
        assert!(
            forward.length_squared() > 0.0,
            "camera look target must differ from its position"
        );

        self.orbit_radius_meters = radius;
        self.azimuth_radians = position.z.atan2(position.x);
        self.elevation_radians = (position.y / radius).clamp(-1.0, 1.0).asin();

        self.set_look_direction_relative(forward.normalize().as_vec3());
    }

    pub fn orbit(&mut self, azimuth_delta: f64, elevation_delta: f64) {
        self.azimuth_radians += azimuth_delta;
        self.elevation_radians = (self.elevation_radians + elevation_delta).clamp(-1.45, 1.45);
    }

    pub fn look(&mut self, yaw_delta: f64, pitch_delta: f64) {
        self.look_yaw_radians += yaw_delta;
        self.look_pitch_radians = (self.look_pitch_radians + pitch_delta).clamp(-1.5, 1.5);
    }

    pub fn look_at_origin(&mut self) {
        self.look_yaw_radians = 0.0;
        self.look_pitch_radians = 0.0;
    }

    pub fn direction(&self) -> Vec3 {
        let (down, right, up) = self.orbit_look_frame();
        let horizontal = self.look_pitch_radians.cos() as f32;
        (down * (self.look_yaw_radians.cos() as f32 * horizontal)
            + right * (self.look_yaw_radians.sin() as f32 * horizontal)
            + up * self.look_pitch_radians.sin() as f32)
            .normalize()
    }

    pub fn planet_frame_world_position(&self, planet_rotation_radians: f64) -> DVec3 {
        planet_local_vector(self.world_position(), planet_rotation_radians)
    }

    pub fn planet_frame_direction(&self, planet_rotation_radians: f64) -> Vec3 {
        planet_local_vector(self.direction().as_dvec3(), planet_rotation_radians).as_vec3()
    }

    pub fn vertical_fov_radians(&self) -> f64 {
        self.vertical_fov_radians
    }

    pub fn set_vertical_fov_degrees(&mut self, vertical_fov_degrees: f64) {
        assert!(vertical_fov_degrees.is_finite() && vertical_fov_degrees > 0.0);
        self.vertical_fov_radians = vertical_fov_degrees
            .to_radians()
            .clamp(MIN_VERTICAL_FOV_RADIANS, MAX_VERTICAL_FOV_RADIANS);
    }

    pub fn zoom(&mut self, wheel_delta: f64) {
        self.vertical_fov_radians = (self.vertical_fov_radians * (-wheel_delta * 0.12).exp())
            .clamp(MIN_VERTICAL_FOV_RADIANS, MAX_VERTICAL_FOV_RADIANS);
    }

    fn orbit_look_frame(&self) -> (Vec3, Vec3, Vec3) {
        let down = -self.world_position().normalize().as_vec3();
        let right = down.cross(Vec3::Y).normalize();
        let up = right.cross(down).normalize();
        (down, right, up)
    }

    fn set_look_direction_relative(&mut self, direction: Vec3) {
        let (down, right, up) = self.orbit_look_frame();
        self.look_yaw_radians = f64::from(direction.dot(right).atan2(direction.dot(down)));
        self.look_pitch_radians = f64::from(direction.dot(up).clamp(-1.0, 1.0).asin());
    }
}

fn view_projection_for(
    forward: Vec3,
    altitude_meters: f64,
    vertical_fov_radians: f64,
    aspect_ratio: f32,
) -> Mat4 {
    let view = Mat4::look_to_rh(Vec3::ZERO, forward, DVec3::Y.as_vec3());
    let near = (altitude_meters * 0.01).clamp(0.05, 10.0) as f32;
    reversed_z_infinite_perspective(vertical_fov_radians as f32, aspect_ratio, near) * view
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub view_projection: [[f32; 4]; 4],
    pub camera_forward: [f32; 4],
    pub camera_right: [f32; 4],
    pub camera_up: [f32; 4],
    pub camera_planet_direction_altitude: [f32; 4],
    pub sun_direction: [f32; 4],
    pub projection: [f32; 4],
}

impl CameraUniform {
    pub fn from_camera(
        camera: &OrbitCamera,
        aspect_ratio: f32,
        sun_direction: DVec3,
        planet_rotation_radians: f64,
    ) -> Self {
        let forward = camera.planet_frame_direction(planet_rotation_radians);
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward).normalize();
        let camera_world_position = camera.planet_frame_world_position(planet_rotation_radians);
        let camera_radius = camera_world_position.length();
        let planet_direction = (camera_world_position / camera_radius).as_vec3();
        let sun_direction = planet_local_vector(sun_direction, planet_rotation_radians)
            .normalize()
            .as_vec3();
        Self {
            view_projection: camera
                .view_projection_in_planet_frame(aspect_ratio, planet_rotation_radians)
                .to_cols_array_2d(),
            camera_forward: [forward.x, forward.y, forward.z, 0.0],
            camera_right: [right.x, right.y, right.z, 0.0],
            camera_up: [up.x, up.y, up.z, 0.0],
            camera_planet_direction_altitude: [
                planet_direction.x,
                planet_direction.y,
                planet_direction.z,
                (camera_radius - PLANET_RADIUS_METERS) as f32,
            ],
            sun_direction: [sun_direction.x, sun_direction.y, sun_direction.z, 0.0],
            projection: [
                aspect_ratio,
                (camera.vertical_fov_radians as f32 * 0.5).tan(),
                0.0,
                0.0,
            ],
        }
    }
}

fn reversed_z_infinite_perspective(
    vertical_fov_radians: f32,
    aspect_ratio: f32,
    near: f32,
) -> Mat4 {
    let focal_length = 1.0 / (vertical_fov_radians * 0.5).tan();
    Mat4::from_cols(
        Vec4::new(focal_length / aspect_ratio, 0.0, 0.0, 0.0),
        Vec4::new(0.0, focal_length, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0),
    )
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::{
        CHUNK_GRID_QUADS, CHUNK_GRID_VERTICES, CubeSphereMesh, HIGH_DETAIL_ZOOM_MINIMUM_LOD_LEVEL,
        LodPolicy, MAX_LOD_LEVEL, MINIMUM_LOD_LEVEL, OrbitCamera, PLANET_RADIUS_METERS, PlanetLod,
        QuadtreeNode, SKIRT_DEPTH_RATIO, build_chunk_mesh, cube_face_direction,
        placeholder_height_meters, planet_local_vector, planet_rotation_radians,
        projected_error_pixels,
    };

    #[test]
    fn quadtree_children_tile_the_parent_node() {
        let children = QuadtreeNode::root(3).children();
        assert_eq!(
            children[0],
            QuadtreeNode {
                face: 3,
                level: 1,
                x: 0,
                y: 0
            }
        );
        assert_eq!(
            children[3],
            QuadtreeNode {
                face: 3,
                level: 1,
                x: 1,
                y: 1
            }
        );
        let policy = LodPolicy::default();
        assert!(policy.should_split(2.1, 0));
        assert!(!policy.should_merge(1.0, MINIMUM_LOD_LEVEL - 1));
        assert!(policy.should_merge(1.0, MINIMUM_LOD_LEVEL));
        assert_eq!(policy.max_level, MAX_LOD_LEVEL);
        assert_eq!(children[0].parent(), Some(QuadtreeNode::root(3)));
        assert!(children.iter().all(|child| child.is_valid()));
    }

    #[test]
    fn projected_error_decreases_with_distance_and_level() {
        let node = QuadtreeNode::root(0);
        let near = DVec3::X * (PLANET_RADIUS_METERS * 3.0);
        let far = DVec3::X * (PLANET_RADIUS_METERS * 30.0);
        let near_error = projected_error_pixels(node, near, 1_080, 45.0_f64.to_radians());
        let far_error = projected_error_pixels(node, far, 1_080, 45.0_f64.to_radians());
        assert!(near_error > far_error);
        assert_eq!(
            node.children()[0].geometric_error_meters(),
            node.geometric_error_meters() * 0.5
        );
    }

    #[test]
    fn orbit_selection_stays_coarse_and_bounded() {
        let camera = OrbitCamera::default();
        let mut lod = PlanetLod::default();
        let update = lod.update_for_view(
            camera.world_position(),
            camera.direction().as_dvec3(),
            1.5,
            1_080,
            45.0_f64.to_radians(),
        );
        assert!(update.metrics.active_chunks <= super::DEFAULT_MAX_ACTIVE_CHUNKS as u32);
        assert_eq!(update.metrics.max_level, MINIMUM_LOD_LEVEL);
        assert!(
            update
                .active_nodes
                .iter()
                .all(|node| node.level >= MINIMUM_LOD_LEVEL)
        );
        assert!(!update.metrics.budget_limited);
    }

    #[test]
    fn frustum_culling_refines_only_the_visible_zoomed_patch() {
        let camera = OrbitCamera::default();
        let position = camera.world_position();
        let forward = -position.normalize();
        let mut wide_lod = PlanetLod::default();
        let wide = wide_lod.update_for_view(position, forward, 1.5, 1_080, 45.0_f64.to_radians());
        let mut zoomed_lod = PlanetLod::default();
        let zoomed =
            zoomed_lod.update_for_view(position, forward, 1.5, 1_080, 2.0_f64.to_radians());

        assert!(
            zoomed.metrics.active_chunks < wide.metrics.active_chunks,
            "wide chunks: {}, zoomed chunks: {}",
            wide.metrics.active_chunks,
            zoomed.metrics.active_chunks
        );
        assert!(zoomed.metrics.max_level >= wide.metrics.max_level);
        assert!(zoomed.metrics.max_level >= HIGH_DETAIL_ZOOM_MINIMUM_LOD_LEVEL);
        assert!(
            zoomed
                .active_nodes
                .iter()
                .all(|node| node.level >= MINIMUM_LOD_LEVEL)
        );
    }

    #[test]
    fn persistent_selector_respects_split_merge_hysteresis() {
        let node = QuadtreeNode {
            face: 0,
            level: MINIMUM_LOD_LEVEL,
            x: 1 << (MINIMUM_LOD_LEVEL - 1),
            y: 1 << (MINIMUM_LOD_LEVEL - 1),
        };
        let mut lod = PlanetLod::new(
            LodPolicy {
                split_pixels: 2.0,
                merge_pixels: 1.25,
                max_level: MINIMUM_LOD_LEVEL + 1,
            },
            1_024,
        );
        let near = camera_for_error(node, 2.5);
        lod.update(near, 1_080, 45.0_f64.to_radians());
        assert!(lod.is_split(node));

        let hysteresis_band = camera_for_error(node, 1.6);
        lod.update(hysteresis_band, 1_080, 45.0_f64.to_radians());
        assert!(lod.is_split(node));

        let far = camera_for_error(node, 0.1);
        let merged = lod.update(far, 1_080, 45.0_f64.to_radians());
        assert!(!lod.is_split(node));
        assert!(merged.metrics.merges > 0);
        assert!(merged.metrics.lod_thrash_events > 0);
    }

    #[test]
    fn monotonic_descent_does_not_report_lod_thrash() {
        let root = QuadtreeNode::root(0);
        let mut lod = PlanetLod::new(
            LodPolicy {
                split_pixels: 2.0,
                merge_pixels: 1.25,
                max_level: 4,
            },
            256,
        );
        for target_error in [0.8, 1.1, 1.6, 2.1, 3.0, 5.0, 9.0] {
            let update = lod.update(
                camera_for_error(root, target_error),
                1_080,
                45.0_f64.to_radians(),
            );
            assert_eq!(update.metrics.lod_thrash_events, 0);
        }
    }

    #[test]
    fn two_kilometer_selection_stays_below_finest_lod_and_budget() {
        let mut lod = PlanetLod::default();
        let camera = DVec3::X * (PLANET_RADIUS_METERS + 2_000.0);
        let update = lod.update(camera, 1_080, 45.0_f64.to_radians());
        assert!(update.metrics.max_level >= 9);
        assert!(update.metrics.max_level <= 13);
        assert!(!update.metrics.budget_limited);
        assert!(update.metrics.active_chunks < super::DEFAULT_MAX_ACTIVE_CHUNKS as u32);
    }

    #[test]
    fn near_surface_selection_reaches_level_eighteen_without_dense_refinement() {
        let mut lod = PlanetLod::default();
        let camera = DVec3::X * (PLANET_RADIUS_METERS + 10.0);
        let update = lod.update(camera, 1_080, 45.0_f64.to_radians());
        assert_eq!(update.metrics.max_level, MAX_LOD_LEVEL);
        assert!(update.active_nodes.len() <= super::DEFAULT_MAX_ACTIVE_CHUNKS);
        assert_eq!(
            update.metrics.level_histogram.iter().copied().sum::<u32>(),
            update.metrics.active_chunks
        );
        assert_eq!(update.metrics.chunks_loaded, update.metrics.active_chunks);
        assert!(update.metrics.culled_nodes > 0);
        assert!(update.metrics.max_seam_delta_meters < 0.01);

        let stable = lod.update(camera, 1_080, 45.0_f64.to_radians());
        assert_eq!(stable.metrics.chunks_loaded, 0);
        assert_eq!(stable.metrics.chunks_unloaded, 0);
    }

    #[test]
    fn chunk_mesh_has_fixed_grid_and_proportional_skirts() {
        let chunk = build_chunk_mesh(QuadtreeNode {
            face: 0,
            level: 4,
            x: 7,
            y: 9,
        });
        let top_vertex_count = CHUNK_GRID_VERTICES * CHUNK_GRID_VERTICES;
        assert_eq!(
            chunk.vertices.len(),
            top_vertex_count + 4 * CHUNK_GRID_VERTICES
        );
        assert_eq!(
            chunk.indices.len(),
            CHUNK_GRID_QUADS * CHUNK_GRID_QUADS * 6 + 4 * CHUNK_GRID_QUADS * 6
        );
        assert!(
            chunk.vertices[..top_vertex_count]
                .iter()
                .all(|vertex| vertex.skirt_depth_meters == 0.0)
        );
        assert!(
            chunk.vertices[top_vertex_count..]
                .iter()
                .all(|vertex| vertex.skirt_depth_meters > 0.0)
        );
        assert!(
            (chunk.skirt_depth_meters / chunk.edge_length_meters - SKIRT_DEPTH_RATIO).abs()
                < f64::EPSILON
        );
        let top_world = chunk.vertex_world_position(0, false);
        let skirt_world = chunk.vertex_world_position(top_vertex_count, false);
        assert!(
            (top_world.distance(skirt_world) - chunk.skirt_depth_meters).abs()
                < chunk.skirt_depth_meters * 0.001
        );
    }

    #[test]
    fn placeholder_height_and_cube_face_edges_are_seam_continuous() {
        assert!(placeholder_height_meters(DVec3::X).abs() < 1.0e-12);
        assert!(super::PLACEHOLDER_HEIGHT_AMPLITUDE_METERS < 4_000.0);
        for face in 0..6 {
            for y in 0..=8 {
                for x in 0..=8 {
                    let direction =
                        cube_face_direction(face, -1.0 + x as f64 * 0.25, -1.0 + y as f64 * 0.25);
                    assert!(
                        placeholder_height_meters(direction).abs()
                            <= super::PLACEHOLDER_HEIGHT_AMPLITUDE_METERS
                    );
                }
            }
        }
        for step in 0..=CHUNK_GRID_QUADS {
            let v = -1.0 + 2.0 * step as f64 / CHUNK_GRID_QUADS as f64;
            let positive_x_right = cube_face_direction(0, 1.0, v);
            let negative_z_left = cube_face_direction(5, -1.0, v);
            assert!(positive_x_right.distance(negative_z_left) < 1.0e-12);
            assert!(
                (placeholder_height_meters(positive_x_right)
                    - placeholder_height_meters(negative_z_left))
                .abs()
                    < 1.0e-9
            );
        }

        let left = build_chunk_mesh(QuadtreeNode {
            face: 0,
            level: 1,
            x: 0,
            y: 0,
        });
        let right = build_chunk_mesh(QuadtreeNode {
            face: 0,
            level: 1,
            x: 1,
            y: 0,
        });
        for y in 0..CHUNK_GRID_VERTICES {
            let left_index = y * CHUNK_GRID_VERTICES + CHUNK_GRID_QUADS;
            let right_index = y * CHUNK_GRID_VERTICES;
            assert!(
                left.vertex_world_position(left_index, true)
                    .distance(right.vertex_world_position(right_index, true))
                    < 1.0
            );
        }
    }

    #[test]
    fn cube_sphere_vertices_are_on_the_planet_radius() {
        let mesh = CubeSphereMesh::new();
        assert_eq!(mesh.world_positions().len(), 6 * 33 * 33);
        assert_eq!(mesh.indices().len(), 6 * 32 * 32 * 6);
        assert!(
            mesh.world_positions()
                .iter()
                .all(|position| (position.length() - PLANET_RADIUS_METERS).abs() < 0.001)
        );
    }

    #[test]
    fn rebasing_uploads_relative_f32_offsets() {
        let mesh = CubeSphereMesh::new();
        let camera = OrbitCamera::default();
        let camera_position = camera.world_position();
        let vertices = mesh.rebased_vertices(camera_position);
        assert!(vertices.iter().all(|vertex| {
            vertex
                .camera_relative_position
                .iter()
                .all(|value| value.is_finite())
        }));
        assert!(
            vertices
                .iter()
                .any(|vertex| vertex.camera_relative_position[0] < -1_000_000.0)
        );
    }

    #[test]
    fn wheel_zoom_changes_fov_without_moving_the_camera_and_increases_screen_error() {
        let mut camera = OrbitCamera::default();
        let position = camera.world_position();
        let error_before = projected_error_pixels(
            QuadtreeNode::root(0),
            position,
            1_080,
            camera.vertical_fov_radians(),
        );
        camera.zoom(1_000.0);
        assert_eq!(camera.world_position(), position);
        assert!((camera.vertical_fov_radians().to_degrees() - 2.0).abs() < 1.0e-9);
        let error_after = projected_error_pixels(
            QuadtreeNode::root(0),
            position,
            1_080,
            camera.vertical_fov_radians(),
        );
        assert!(error_after > error_before);

        let mut lod = PlanetLod::default();
        let detailed = lod.update(position, 1_080, camera.vertical_fov_radians());
        assert!(detailed.metrics.max_level >= HIGH_DETAIL_ZOOM_MINIMUM_LOD_LEVEL);

        camera.zoom(-1_000.0);
        assert!((camera.vertical_fov_radians().to_degrees() - 75.0).abs() < 1.0e-9);
    }

    #[test]
    fn planet_rotation_transforms_camera_and_sun_into_a_stable_local_frame() {
        let camera = OrbitCamera::default();
        let rotation = planet_rotation_radians(150.0);
        let world_position = camera.world_position();
        let local_position = camera.planet_frame_world_position(rotation);
        assert!((local_position.length() - world_position.length()).abs() < 1.0e-8);
        assert!(local_position.distance(world_position) > 1_000_000.0);

        let world_sun = DVec3::new(0.4, 0.7, 0.6).normalize();
        let local_sun = planet_local_vector(world_sun, rotation);
        let world_camera_direction = world_position.normalize();
        assert!(
            (local_position.normalize().dot(local_sun) - world_camera_direction.dot(world_sun))
                .abs()
                < 1.0e-12
        );
    }

    #[test]
    fn free_look_changes_orientation_without_moving_the_camera() {
        let mut camera = OrbitCamera::default();
        let position = camera.world_position();
        let before = camera.view_projection(1.0);
        camera.look(0.25, -0.1);
        assert_eq!(camera.world_position(), position);
        assert_ne!(camera.view_projection(1.0), before);
    }

    #[test]
    fn default_look_tracks_orbital_down_with_a_persistent_mouse_offset() {
        let mut camera = OrbitCamera::default();
        assert!(
            camera
                .direction()
                .as_dvec3()
                .distance(-camera.world_position().normalize())
                < 1.0e-6
        );

        camera.look(0.25, -0.1);
        let (down_before, right_before, up_before) = camera.orbit_look_frame();
        let relative_before = [
            camera.direction().dot(down_before),
            camera.direction().dot(right_before),
            camera.direction().dot(up_before),
        ];
        camera.orbit(0.4, 0.0);
        let (down_after, right_after, up_after) = camera.orbit_look_frame();
        let relative_after = [
            camera.direction().dot(down_after),
            camera.direction().dot(right_after),
            camera.direction().dot(up_after),
        ];
        for (before, after) in relative_before.into_iter().zip(relative_after) {
            assert!((before - after).abs() < 1.0e-6);
        }
    }

    #[test]
    fn waypoint_pose_preserves_f64_position_and_arbitrary_look_direction() {
        let mut camera = OrbitCamera::default();
        let position =
            DVec3::new(1.0, 2.0, -3.0).normalize() * (PLANET_RADIUS_METERS + 1_234.567_890_123);
        let look_at = DVec3::new(-81_234.5, 456_789.25, 12_345.75);
        camera.set_world_pose(position, look_at);

        assert!(camera.world_position().distance(position) < 1.0e-8);
        let expected_direction = (look_at - position).normalize();
        assert!(camera.direction().as_dvec3().distance(expected_direction) < 1.0e-6);
        assert!(camera.view_projection(1.5).is_finite());
    }

    fn camera_for_error(node: QuadtreeNode, target_error_pixels: f64) -> DVec3 {
        let mut near_radius = PLANET_RADIUS_METERS + 10.0;
        let mut far_radius = PLANET_RADIUS_METERS * 10_000.0;
        for _ in 0..100 {
            let radius = (near_radius + far_radius) * 0.5;
            let error =
                projected_error_pixels(node, DVec3::X * radius, 1_080, 45.0_f64.to_radians());
            if error > target_error_pixels {
                near_radius = radius;
            } else {
                far_radius = radius;
            }
        }
        DVec3::X * ((near_radius + far_radius) * 0.5)
    }
}

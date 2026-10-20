//! Deterministic, planet-wide road topology, independent of render residency.
//!
//! A terrain-weighted spanning tree of cube cells supplies shared gateways.
//! Each cell connects those gateways and its villages with a shared local tree.
//! Thus neighbouring plans join exactly, including across cube faces, without
//! constructing a complete village-to-village graph or building it per frame.
//!
//! These are survey alignments, NOT engineered deck profiles. In particular a
//! sea crossing is a REQUIRED UNDERSEA TUNNEL, never a bridge over the water.
//! Profile fitting, cover, grade and excavation must be checked before meshing.
use glam::DVec3;
use planet_coretypes::{CubeFace, TileKey, face_uv_to_direction};
use serde::Serialize;
use std::{
    cell::RefCell,
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap},
};

pub const REGION_LEVEL: u8 = 8;
pub const LOCAL_QUADS: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct Ground {
    /// Land surface or actual seabed, never the visible ocean shell.
    pub height: f64,
    pub ocean: bool,
}
impl Ground {
    fn valid(self) -> bool {
        self.height.is_finite()
    }
}

pub trait Survey {
    fn cancelled(&self) -> bool {
        false
    }
    fn settlement_core_at(&self, _direction: DVec3) -> Result<bool, String> {
        Ok(false)
    }
    /// Runtime approaches meet the ground; diagnostic/legacy plans retain 8m.
    fn village_access_height(&self, ground: Ground) -> f64 {
        ground.height + 8.0
    }
    /// Conservative occupancy: false must imply no accepted village in this
    /// region. False-positive land cells cost roads; false negatives isolate
    /// real settlements. Unknown sources therefore default to retained.
    fn may_have_villages(&self, _key: TileKey) -> Result<bool, String> {
        Ok(true)
    }
    fn ground(&self, direction: DVec3) -> Result<Ground, String>;
    fn villages(&self, key: TileKey) -> Result<Vec<DVec3>, String>;
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    pub gateway: [f64; 3],
    pub length_meters: f64,
    pub construction_cost_proxy: f64,
    /// Coarse route has water on it: the detailed planner must go below bed.
    pub requires_undersea_tunnel: bool,
}

pub struct Backbone {
    pub level: u8,
    pub potentially_inhabited_regions: usize,
    pub links: Vec<Link>,
    incident: Vec<Vec<usize>>,
    occupancy: RefCell<Occupancy>,
}

#[derive(Default)]
struct Occupancy {
    regions: HashMap<usize, bool>,
    /// Does the component rooted at this region, excluding this link, contain
    /// an accepted village? Independent of camera residency and cached lazily.
    branches: HashMap<(usize, usize), bool>,
}

fn count(level: u8) -> usize {
    6 * (1_usize << (2 * level))
}
fn id(key: TileKey) -> usize {
    let side = 1_usize << key.level;
    key.face.index() as usize * side * side + key.y as usize * side + key.x as usize
}
pub(crate) fn key(index: usize, level: u8) -> TileKey {
    let side = 1_usize << level;
    TileKey {
        face: CubeFace::ALL[index / (side * side)],
        level,
        x: (index % side) as u32,
        y: (index / side % side) as u32,
    }
}
pub(crate) fn direction(key: TileKey, x: f64, y: f64) -> DVec3 {
    let side = f64::from(1_u32 << key.level);
    face_uv_to_direction(
        key.face,
        -1.0 + 2.0 * (f64::from(key.x) + x) / side,
        -1.0 + 2.0 * (f64::from(key.y) + y) / side,
    )
}

/// Canonical shared-edge point. Do not average the two unit centre vectors:
/// on a cube projection that average is not generally on the cell boundary.
fn neighbour(key: TileKey, edge: usize) -> (TileKey, DVec3) {
    let (x, y, dx, dy) = match edge {
        0 => (0.0, 0.5, -0.01, 0.0),
        1 => (1.0, 0.5, 0.01, 0.0),
        2 => (0.5, 0.0, 0.0, -0.01),
        _ => (0.5, 1.0, 0.0, 0.01),
    };
    let d = direction(key, x + dx, y + dy);
    (
        planet_coretypes::tile_key_for_direction(d, key.level),
        direction(key, x, y),
    )
}

impl Backbone {
    pub fn survey(level: u8, radius: f64, terrain: &impl Survey) -> Result<Self, String> {
        if !(1..=REGION_LEVEL).contains(&level) || !radius.is_finite() || radius <= 0.0 {
            return Err("invalid network level or planet radius".into());
        }
        let total = count(level);
        let centres: Vec<_> = (0..total)
            .map(|i| direction(key(i, level), 0.5, 0.5))
            .collect();
        let ground = centres
            .iter()
            .map(|&d| checked_ground(terrain, d))
            .collect::<Result<Vec<_>, _>>()?;
        let occupied = (0..total)
            .map(|i| terrain.may_have_villages(key(i, level)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut candidates = Vec::with_capacity(total * 2);
        for a in 0..total {
            for side in 0..4 {
                let (other, gateway) = neighbour(key(a, level), side);
                let b = id(other);
                if b <= a {
                    continue;
                }
                let points = [
                    centres[a],
                    centres[a].lerp(gateway, 0.5).normalize(),
                    gateway,
                    gateway.lerp(centres[b], 0.5).normalize(),
                    centres[b],
                ];
                let mut samples = [ground[a]; 5];
                samples[4] = ground[b];
                for j in 1..4 {
                    samples[j] = checked_ground(terrain, points[j])?;
                }
                let mut cost = 0.0;
                let mut length = 0.0;
                for j in 0..4 {
                    let distance = arc(points[j], points[j + 1], radius);
                    length += distance;
                    cost += edge_cost(distance, samples[j], samples[j + 1]);
                }
                candidates.push(Link {
                    a,
                    b,
                    gateway: gateway.to_array(),
                    length_meters: length,
                    construction_cost_proxy: cost,
                    requires_undersea_tunnel: samples.iter().any(|g| g.ocean),
                });
            }
        }
        candidates.sort_by(|a, b| {
            a.construction_cost_proxy
                .total_cmp(&b.construction_cost_proxy)
                .then_with(|| (a.a, a.b).cmp(&(b.a, b.b)))
        });
        let mut sets = Sets::new(total);
        let mut links = Vec::with_capacity(total - 1);
        let mut incident = vec![Vec::new(); total];
        for link in candidates {
            if sets.join(link.a, link.b) {
                incident[link.a].push(links.len());
                incident[link.b].push(links.len());
                links.push(link);
                if links.len() == total - 1 {
                    break;
                }
            }
        }
        if links.len() != total - 1 {
            return Err("cube network is disconnected".into());
        }
        // Prune transit-only leaves recursively. Empty sea/desert cells may
        // carry a shared connection, but must not grow dead-end motorways.
        let mut degree: Vec<_> = incident.iter().map(Vec::len).collect();
        let mut leaves: Vec<_> = (0..total)
            .filter(|&i| !occupied[i] && degree[i] <= 1)
            .collect();
        let mut keep = vec![true; links.len()];
        while let Some(i) = leaves.pop() {
            if degree[i] != 1 {
                continue;
            }
            let edge = *incident[i].iter().find(|&&e| keep[e]).unwrap();
            keep[edge] = false;
            let link = &links[edge];
            let j = if link.a == i { link.b } else { link.a };
            degree[i] -= 1;
            degree[j] -= 1;
            if !occupied[j] && degree[j] == 1 {
                leaves.push(j);
            }
        }
        links = links
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| keep[i].then_some(e))
            .collect();
        incident.iter_mut().for_each(Vec::clear);
        for (i, link) in links.iter().enumerate() {
            incident[link.a].push(i);
            incident[link.b].push(i);
        }
        Ok(Self {
            level,
            potentially_inhabited_regions: occupied.iter().filter(|&&v| v).count(),
            links,
            incident,
            occupancy: RefCell::new(Occupancy::default()),
        })
    }

    pub fn retained_region_count(&self) -> usize {
        if self.links.is_empty() {
            self.potentially_inhabited_regions.min(1)
        } else {
            self.links.len() + 1
        }
    }
    pub fn region_count(&self) -> usize {
        self.incident.len()
    }
    pub fn region_key(&self, direction: DVec3) -> TileKey {
        planet_coretypes::tile_key_for_direction(direction, self.level)
    }
    pub fn gateways(&self, region: TileKey) -> Result<Vec<(usize, DVec3)>, String> {
        if !region.is_valid() || region.level != self.level {
            return Err("region key does not belong to this backbone".into());
        }
        Ok(self.incident[id(region)]
            .iter()
            .map(|&i| (i, DVec3::from_array(self.links[i].gateway)))
            .collect())
    }

    fn component_has_villages(
        &self,
        root: usize,
        excluded: usize,
        terrain: &impl Survey,
    ) -> Result<bool, String> {
        let mut occupancy = self.occupancy.borrow_mut();
        // Iterative DFS: long empty mountain/sea chains must not consume the
        // call stack. Stop at the first real village; never survey the world
        // eagerly, run engineering, or depend on which neighbours are loaded.
        let mut stack = vec![(root, excluded, 0)];
        while let Some(&(node, parent, next)) = stack.last() {
            if terrain.cancelled() {
                return Err("road generation superseded".into());
            }
            let known = occupancy.branches.get(&(node, parent)).copied();
            let inhabited = match known {
                Some(value) => value,
                None => match occupancy.regions.get(&node) {
                    Some(&value) => value,
                    None => {
                        let region = key(node, self.level);
                        let subdivision =
                            1_u32 << (crate::village::VILLAGE_CELL_LEVEL - self.level);
                        let mut found = false;
                        'cells: for y in 0..subdivision {
                            for x in 0..subdivision {
                                if terrain.cancelled() {
                                    return Err("road generation superseded".into());
                                }
                                let cell = TileKey {
                                    level: crate::village::VILLAGE_CELL_LEVEL,
                                    x: region.x * subdivision + x,
                                    y: region.y * subdivision + y,
                                    ..region
                                };
                                if !terrain.villages(cell)?.is_empty() {
                                    found = true;
                                    break 'cells;
                                }
                            }
                        }
                        occupancy.regions.insert(node, found);
                        found
                    }
                },
            };
            if inhabited {
                for &(ancestor, incoming, _) in &stack {
                    occupancy.branches.insert((ancestor, incoming), true);
                }
                return Ok(true);
            }
            if known == Some(false) || next == self.incident[node].len() {
                occupancy.branches.insert((node, parent), false);
                stack.pop();
            } else {
                stack.last_mut().unwrap().2 += 1;
                let edge = self.incident[node][next];
                if edge != parent {
                    let link = &self.links[edge];
                    let other = if link.a == node { link.b } else { link.a };
                    stack.push((other, edge, 0));
                }
            }
        }
        Ok(false)
    }

    /// All accepted village sites, not the camera's distance-culled house list.
    pub fn local(
        &self,
        region: TileKey,
        radius: f64,
        terrain: &impl Survey,
    ) -> Result<Region, String> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err("invalid local survey radius".into());
        }
        let gateways = self.gateways(region)?;
        let subdivision = 1_u32 << (crate::village::VILLAGE_CELL_LEVEL - region.level);
        let mut terminals = Vec::new();
        for y in 0..subdivision {
            for x in 0..subdivision {
                let cell = TileKey {
                    level: crate::village::VILLAGE_CELL_LEVEL,
                    x: region.x * subdivision + x,
                    y: region.y * subdivision + y,
                    ..region
                };
                for centre in terrain.villages(cell)? {
                    if !centre.is_finite() || centre.length_squared() == 0.0 {
                        return Err("invalid village centre".into());
                    }
                    if self.region_key(centre) != region {
                        return Err("village does not belong to its surveyed region".into());
                    }
                    terminals.push(Terminal {
                        direction: centre.normalize().to_array(),
                        kind: TerminalKind::Village,
                    });
                }
            }
        }
        let village_count = terminals.len();
        self.occupancy
            .borrow_mut()
            .regions
            .insert(id(region), village_count != 0);
        for (link, d) in gateways {
            let edge = &self.links[link];
            let other = if edge.a == id(region) { edge.b } else { edge.a };
            // Coarse occupancy deliberately retains false positives. A gateway
            // is a connection, not a destination in an otherwise empty leaf.
            if self.component_has_villages(other, link, terrain)? {
                terminals.push(Terminal {
                    direction: d.to_array(),
                    kind: TerminalKind::Gateway { link },
                });
            }
        }
        local_tree(region, radius, terrain, terminals, village_count)
    }
}

fn checked_ground(terrain: &impl Survey, direction: DVec3) -> Result<Ground, String> {
    let g = terrain.ground(direction)?;
    if g.valid() {
        Ok(g)
    } else {
        Err("non-finite terrain in road survey".into())
    }
}
fn arc(a: DVec3, b: DVec3, radius: f64) -> f64 {
    // atan2 remains accurate at the ~100m local lattice spacing.
    a.cross(b).length().atan2(a.dot(b)) * radius
}
fn edge_cost(distance: f64, a: Ground, b: Ground) -> f64 {
    crate::road_construction::survey_edge(distance, a, b)
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalKind {
    Village,
    Gateway { link: usize },
}
#[derive(Clone, Debug, Serialize)]
pub struct Terminal {
    pub direction: [f64; 3],
    pub kind: TerminalKind,
}
#[derive(Clone, Debug, Serialize)]
pub struct Node {
    pub direction: [f64; 3],
    pub ground_meters: f64,
    pub ocean: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct RoadEdge {
    pub a: usize,
    pub b: usize,
    pub requires_undersea_tunnel: bool,
}
#[derive(Clone, Serialize)]
pub struct Region {
    pub key: TileKey,
    pub village_count: usize,
    pub terminals: Vec<Terminal>,
    /// Index of the exact terminal, not an approximate lattice-snapped centre.
    pub terminal_nodes: Vec<usize>,
    pub nodes: Vec<Node>,
    pub edges: Vec<RoadEdge>,
    pub junction_count: usize,
    pub length_meters: f64,
    pub construction_cost_proxy: f64,
    pub lattice_survey_samples: usize,
}

#[derive(Clone, Copy, Debug)]
struct Queue {
    cost: f64,
    index: usize,
}
impl PartialEq for Queue {
    fn eq(&self, b: &Self) -> bool {
        self.cost.total_cmp(&b.cost).is_eq() && self.index == b.index
    }
}
impl Eq for Queue {}
impl PartialOrd for Queue {
    fn partial_cmp(&self, b: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for Queue {
    fn cmp(&self, b: &Self) -> std::cmp::Ordering {
        self.cost.total_cmp(&b.cost).then(self.index.cmp(&b.index))
    }
}

fn local_tree(
    region: TileKey,
    radius: f64,
    terrain: &impl Survey,
    terminals: Vec<Terminal>,
    village_count: usize,
) -> Result<Region, String> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err("invalid local survey radius".into());
    }
    if terminals.len() <= 1 {
        let nodes = terminals
            .iter()
            .map(|t| {
                let ground = checked_ground(terrain, DVec3::from_array(t.direction))?;
                Ok(Node {
                    direction: t.direction,
                    ground_meters: ground.height,
                    ocean: ground.ocean,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let terminal_nodes = (0..nodes.len()).collect();
        return Ok(Region {
            key: region,
            village_count,
            terminals,
            terminal_nodes,
            nodes,
            edges: Vec::new(),
            junction_count: 0,
            length_meters: 0.0,
            construction_cost_proxy: 0.0,
            lattice_survey_samples: 0,
        });
    }
    let side = LOCAL_QUADS + 1;
    let lattice = side * side;
    let mut dirs = Vec::with_capacity(lattice + terminals.len());
    for y in 0..side {
        for x in 0..side {
            dirs.push(direction(
                region,
                x as f64 / LOCAL_QUADS as f64,
                y as f64 / LOCAL_QUADS as f64,
            ));
        }
    }
    dirs.extend(terminals.iter().map(|t| DVec3::from_array(t.direction)));
    let ground = dirs
        .iter()
        .map(|&d| checked_ground(terrain, d))
        .collect::<Result<Vec<_>, _>>()?;
    let mut extra = vec![Vec::<usize>::new(); dirs.len()];
    // Attach exact terminals to all four corners of their containing square.
    // Distinct nearby villages remain distinct terminals even in one square.
    for (n, t) in terminals.iter().enumerate() {
        let d = DVec3::from_array(t.direction);
        // At a cube seam the canonical dominant face can differ from this
        // region's face. Project onto THIS face rather than changing owners.
        let (normal, u, v) = crate::planet::cube_face_basis(region.face.index());
        let scale = d.dot(normal);
        let uv = [d.dot(u) / scale, d.dot(v) / scale];
        let cells = f64::from(1_u32 << region.level);
        let xy = [
            (uv[0] + 1.0) * 0.5 * cells - f64::from(region.x),
            (uv[1] + 1.0) * 0.5 * cells - f64::from(region.y),
        ];
        let x = (xy[0] * LOCAL_QUADS as f64).clamp(0.0, (LOCAL_QUADS - 1) as f64) as usize;
        let y = (xy[1] * LOCAL_QUADS as f64).clamp(0.0, (LOCAL_QUADS - 1) as f64) as usize;
        for corner in [
            y * side + x,
            y * side + x + 1,
            (y + 1) * side + x,
            (y + 1) * side + x + 1,
        ] {
            extra[lattice + n].push(corner);
            extra[corner].push(lattice + n);
        }
    }
    let neighbours = |i: usize| {
        let mut adjacent = [usize::MAX; 8];
        if i < lattice {
            let (x, y) = ((i % side) as isize, (i / side) as isize);
            let mut at = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if dx != 0 && dy != 0 {
                        let even = (x.min(nx) + y.min(ny)) % 2 == 0;
                        if (dx == dy) != even {
                            continue;
                        }
                    }
                    if (dx != 0 || dy != 0)
                        && nx >= 0
                        && ny >= 0
                        && nx < side as isize
                        && ny < side as isize
                    {
                        adjacent[at] = ny as usize * side + nx as usize;
                        at += 1;
                    }
                }
            }
        }
        adjacent
            .into_iter()
            .filter(|&j| j != usize::MAX)
            .chain(extra[i].iter().copied())
    };
    let cost = |a: usize, b: usize| edge_cost(arc(dirs[a], dirs[b], radius), ground[a], ground[b]);
    let mut distance = vec![f64::INFINITY; dirs.len()];
    let mut owner = vec![usize::MAX; dirs.len()];
    let mut parent = vec![usize::MAX; dirs.len()];
    let mut heap = BinaryHeap::new();
    for n in 0..terminals.len() {
        let index = lattice + n;
        distance[index] = 0.0;
        owner[index] = n;
        heap.push(Reverse(Queue { cost: 0.0, index }));
    }
    // One multi-source Dijkstra, not one search for every village pair.
    while let Some(Reverse(Queue { cost: at, index: i })) = heap.pop() {
        if at > distance[i] {
            continue;
        }
        for j in neighbours(i) {
            let next = at + cost(i, j);
            if next < distance[j] {
                distance[j] = next;
                owner[j] = owner[i];
                parent[j] = i;
                heap.push(Reverse(Queue {
                    cost: next,
                    index: j,
                }));
            }
        }
    }
    let mut candidates = BTreeMap::<(usize, usize), (f64, usize, usize)>::new();
    for a in 0..dirs.len() {
        for b in neighbours(a) {
            if a >= b || owner[a] == owner[b] || owner[a] == usize::MAX || owner[b] == usize::MAX {
                continue;
            }
            let pair = (owner[a].min(owner[b]), owner[a].max(owner[b]));
            let candidate = (distance[a] + cost(a, b) + distance[b], a, b);
            let old = candidates.entry(pair).or_insert(candidate);
            if candidate.0.total_cmp(&old.0).is_lt() {
                *old = candidate;
            }
        }
    }
    let mut candidates: Vec<_> = candidates.into_values().collect();
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| (a.1, a.2).cmp(&(b.1, b.2)))
    });
    let mut sets = Sets::new(terminals.len());
    let mut selected = BTreeSet::new();
    for (_, a, b) in candidates {
        if !sets.join(owner[a], owner[b]) {
            continue;
        }
        selected.insert((a.min(b), a.max(b)));
        for mut i in [a, b] {
            while parent[i] != usize::MAX {
                let j = parent[i];
                selected.insert((i.min(j), i.max(j)));
                i = j;
            }
        }
    }
    // Remove cycles in the union of shared paths, then non-terminal spurs.
    let mut selected: Vec<_> = selected.into_iter().collect();
    selected.sort_by(|&(a, b), &(c, d)| {
        cost(a, b)
            .total_cmp(&cost(c, d))
            .then_with(|| (a, b).cmp(&(c, d)))
    });
    let mut sets = Sets::new(dirs.len());
    selected.retain(|&(a, b)| sets.join(a, b));
    let mut adjacency = vec![BTreeSet::new(); dirs.len()];
    for &(a, b) in &selected {
        adjacency[a].insert(b);
        adjacency[b].insert(a);
    }
    let mut leaves: Vec<_> = (0..lattice).filter(|&i| adjacency[i].len() == 1).collect();
    while let Some(i) = leaves.pop() {
        if adjacency[i].len() != 1 {
            continue;
        }
        let j = *adjacency[i].first().unwrap();
        adjacency[i].clear();
        adjacency[j].remove(&i);
        if j < lattice && adjacency[j].len() == 1 {
            leaves.push(j);
        }
    }
    selected.retain(|&(a, b)| adjacency[a].contains(&b));
    let mut indices = vec![usize::MAX; dirs.len()];
    let mut nodes = Vec::new();
    for i in 0..dirs.len() {
        if !adjacency[i].is_empty() || i >= lattice {
            indices[i] = nodes.len();
            nodes.push(Node {
                direction: dirs[i].to_array(),
                ground_meters: ground[i].height,
                ocean: ground[i].ocean,
            });
        }
    }
    let edges: Vec<_> = selected
        .iter()
        .map(|&(a, b)| RoadEdge {
            a: indices[a],
            b: indices[b],
            requires_undersea_tunnel: ground[a].ocean || ground[b].ocean,
        })
        .collect();
    if !terminals.is_empty() && edges.len() + 1 != nodes.len() {
        return Err("local village/gateway network is disconnected".into());
    }
    Ok(Region {
        key: region,
        village_count,
        terminals,
        terminal_nodes: (lattice..dirs.len()).map(|i| indices[i]).collect(),
        nodes,
        edges,
        junction_count: adjacency.iter().filter(|a| a.len() >= 3).count(),
        length_meters: selected
            .iter()
            .map(|&(a, b)| arc(dirs[a], dirs[b], radius))
            .sum(),
        construction_cost_proxy: selected.iter().map(|&(a, b)| cost(a, b)).sum(),
        lattice_survey_samples: lattice,
    })
}

struct Sets {
    parent: Vec<usize>,
    rank: Vec<u8>,
}
impl Sets {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }
    fn root(&mut self, mut a: usize) -> usize {
        while self.parent[a] != a {
            self.parent[a] = self.parent[self.parent[a]];
            a = self.parent[a];
        }
        a
    }
    fn join(&mut self, a: usize, b: usize) -> bool {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return false;
        }
        if self.rank[a] < self.rank[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        if self.rank[a] == self.rank[b] {
            self.rank[a] += 1;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Flat;
    impl Survey for Flat {
        fn ground(&self, _: DVec3) -> Result<Ground, String> {
            Ok(Ground {
                height: 900.0,
                ocean: false,
            })
        }
        fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
            Ok(Vec::new())
        }
    }
    fn test_key() -> TileKey {
        TileKey {
            face: CubeFace::PositiveX,
            level: REGION_LEVEL,
            x: 128,
            y: 128,
        }
    }
    fn terminals(points: &[(f64, f64)]) -> Vec<Terminal> {
        points
            .iter()
            .map(|&(x, y)| Terminal {
                direction: direction(test_key(), x, y).to_array(),
                kind: TerminalKind::Village,
            })
            .collect()
    }
    fn connected(region: &Region) {
        if region.nodes.is_empty() {
            return;
        }
        assert_eq!(region.edges.len() + 1, region.nodes.len());
        let mut sets = Sets::new(region.nodes.len());
        for e in &region.edges {
            assert!(sets.join(e.a, e.b), "cycle/duplicate");
        }
        for i in 1..region.nodes.len() {
            assert_eq!(sets.root(0), sets.root(i));
        }
        assert_eq!(region.terminal_nodes.len(), region.terminals.len());
        for (&i, t) in region.terminal_nodes.iter().zip(&region.terminals) {
            assert_eq!(region.nodes[i].direction, t.direction, "snapped terminal");
        }
    }

    #[test]
    fn road_network_prunes_empty_leaves_but_keeps_needed_undersea_transit() {
        struct Sparse {
            none: bool,
        }
        impl Survey for Sparse {
            fn may_have_villages(&self, k: TileKey) -> Result<bool, String> {
                Ok(!self.none && (id(k) == 0 || id(k) == count(k.level) - 1))
            }
            fn ground(&self, _: DVec3) -> Result<Ground, String> {
                Ok(Ground {
                    height: -100.0,
                    ocean: true,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
        }
        let r = Backbone::survey(2, 4e6, &Sparse { none: false }).unwrap();
        assert!(r.links.len() < r.region_count() - 1);
        assert!(r.links.iter().all(|e| e.requires_undersea_tunnel));
        let mut sets = Sets::new(r.region_count());
        for link in &r.links {
            assert!(sets.join(link.a, link.b));
        }
        assert_eq!(sets.root(0), sets.root(count(2) - 1));
        for i in 1..count(2) - 1 {
            assert_ne!(r.incident[i].len(), 1, "unnecessary transit leaf");
        }
        assert_eq!(r.retained_region_count(), r.links.len() + 1);
        let empty = Backbone::survey(2, 4e6, &Sparse { none: true }).unwrap();
        assert!(empty.links.is_empty());
        assert_eq!(empty.retained_region_count(), 0);
    }

    #[test]
    fn road_network_short_ridge_bore_competes_with_a_long_surface_detour() {
        struct Ridge;
        impl Survey for Ridge {
            fn ground(&self, d: DVec3) -> Result<Ground, String> {
                let k = test_key();
                let (normal, u, v) = crate::planet::cube_face_basis(k.face.index());
                let x = (d.dot(u) / d.dot(normal) + 1.0) * 128.0 - k.x as f64;
                let y = (d.dot(v) / d.dot(normal) + 1.0) * 128.0 - k.y as f64;
                let ridge = (1.0 - (x - 0.5).abs() / 0.08).max(0.0);
                let gap = ((0.9 - y) / 0.1).clamp(0.0, 1.0);
                Ok(Ground {
                    height: 900.0 + 1400.0 * ridge * gap,
                    ocean: false,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
        }
        let r = local_tree(
            test_key(),
            4e6,
            &Ridge,
            terminals(&[(0.25, 0.5), (0.75, 0.5)]),
            2,
        )
        .unwrap();
        let direct = arc(
            direction(test_key(), 0.25, 0.5),
            direction(test_key(), 0.75, 0.5),
            4e6,
        );
        assert!(
            r.length_meters < direct * 1.25,
            "a short viable bore should beat the long detour: {} vs {}",
            r.length_meters,
            direct
        );
        let p = crate::road_network_profile::Profile::fit(&r, 4e6, &Ridge)
            .unwrap()
            .unwrap();
        assert!(p.maximum_grade <= 0.25);
        assert!(
            p.maximum_grade_change_per_meter
                <= crate::road_vertical::MAX_GRADE_CHANGE_PER_METER + 1e-8
        );
        eprintln!(
            "ridge: direct {direct:.1}m, selected {:.1}m, max grade {:.4}",
            r.length_meters, p.maximum_grade
        );
        assert!(
            p.directions
                .iter()
                .zip(&p.heights_meters)
                .any(|(d, h)| { Ridge.ground(DVec3::from_array(*d)).unwrap().height - h > 100.0 }),
            "the alternative must actually pass below the ridge"
        );
    }

    #[test]
    fn road_network_exact_occupancy_removes_empty_spurs_but_keeps_transit() {
        struct Sites(Vec<TileKey>);
        impl Survey for Sites {
            fn ground(&self, _: DVec3) -> Result<Ground, String> {
                Flat.ground(DVec3::Z)
            }
            fn villages(&self, cell: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(self
                    .0
                    .iter()
                    .filter_map(|&region| {
                        let site = direction(region, 0.4, 0.4);
                        (planet_coretypes::tile_key_for_direction(site, cell.level) == cell)
                            .then_some(site)
                    })
                    .collect())
            }
        }
        let keys: Vec<_> = (0..5)
            .map(|x| TileKey {
                x: 128 + x,
                ..test_key()
            })
            .collect();
        let mut backbone = Backbone::survey(REGION_LEVEL, 4e6, &Flat).unwrap();
        // A--B--C, with a conservatively retained but actually empty B--D--E spur.
        backbone.links.clear();
        backbone.incident.iter_mut().for_each(Vec::clear);
        for (a, b) in [(0, 1), (1, 2), (1, 3), (3, 4)] {
            let link = backbone.links.len();
            backbone.links.push(Link {
                a: id(keys[a]),
                b: id(keys[b]),
                gateway: direction(keys[a], 1.0, 0.5).to_array(),
                length_meters: 1.0,
                construction_cost_proxy: 1.0,
                requires_undersea_tunnel: false,
            });
            backbone.incident[id(keys[a])].push(link);
            backbone.incident[id(keys[b])].push(link);
        }
        let sites = Sites(vec![keys[0], keys[2]]);
        let transit = backbone.local(keys[1], 4e6, &sites).unwrap();
        assert_eq!(transit.village_count, 0);
        assert_eq!(
            transit.terminals.len(),
            2,
            "empty spur is not a destination"
        );
        assert!(
            !transit.edges.is_empty(),
            "uninhabited transit must remain connected"
        );
        for &k in &[keys[0], keys[2]] {
            let region = backbone.local(k, 4e6, &sites).unwrap();
            assert_eq!(region.village_count, 1);
            assert_eq!(region.terminals.len(), 2);
            connected(&region);
        }
        let empty = backbone.local(keys[3], 4e6, &sites).unwrap();
        assert!(empty.edges.is_empty(), "do not build down the empty branch");
    }

    #[test]
    fn road_network_cube_adjacency_is_reciprocal_and_gateways_match_at_every_seam() {
        for level in [1, 2, 3, 8] {
            let side = 1_u32 << level;
            for face in CubeFace::ALL {
                let cells: Vec<_> = if level < 4 {
                    (0..side)
                        .flat_map(|y| (0..side).map(move |x| (x, y)))
                        .collect()
                } else {
                    (0..side)
                        .flat_map(|i| [(0, i), (side - 1, i), (i, 0), (i, side - 1)])
                        .collect()
                };
                for (x, y) in cells {
                    let a = TileKey { face, level, x, y };
                    for edge in 0..4 {
                        let (b, g) = neighbour(a, edge);
                        assert_ne!(a, b);
                        let back = (0..4)
                            .map(|s| neighbour(b, s))
                            .find(|(k, _)| *k == a)
                            .expect("reciprocal neighbour");
                        assert!(
                            g.distance(back.1) < 1e-12,
                            "{a:?} -> {b:?}: mismatched seam"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn road_network_backbone_is_a_deterministic_connected_tree_not_all_pairs() {
        let a = Backbone::survey(3, 4e6, &Flat).unwrap();
        let b = Backbone::survey(3, 4e6, &Flat).unwrap();
        assert_eq!(a.links.len(), count(3) - 1);
        assert_eq!(
            serde_json::to_string(&a.links).unwrap(),
            serde_json::to_string(&b.links).unwrap()
        );
        let mut sets = Sets::new(a.region_count());
        for link in &a.links {
            assert!(sets.join(link.a, link.b));
            assert!(!link.requires_undersea_tunnel);
            for index in [link.a, link.b] {
                let gates = a.gateways(key(index, 3)).unwrap();
                assert!(gates.iter().any(|(_, d)| d.to_array() == link.gateway));
            }
        }
        for i in 1..a.region_count() {
            assert_eq!(sets.root(0), sets.root(i));
        }
    }
    #[test]
    fn road_network_local_tree_reuses_trunks_and_preserves_every_village() {
        let points = [
            (0.1, 0.5),
            (0.3, 0.5),
            (0.5, 0.5),
            (0.7, 0.5),
            (0.9, 0.5),
            (0.5, 0.8),
        ];
        let r = local_tree(test_key(), 4e6, &Flat, terminals(&points), points.len()).unwrap();
        connected(&r);
        assert_eq!(r.village_count, points.len());
        assert!(r.junction_count > 0);
        let independent: f64 = points[1..]
            .iter()
            .map(|&(x, y)| {
                arc(
                    direction(test_key(), points[0].0, points[0].1),
                    direction(test_key(), x, y),
                    4e6,
                )
            })
            .sum();
        assert!(
            r.length_meters < independent * 0.65,
            "shared {} vs independent {}",
            r.length_meters,
            independent
        );
        let second = local_tree(test_key(), 4e6, &Flat, terminals(&points), points.len()).unwrap();
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
    }
    #[test]
    fn road_network_empty_single_and_coincident_sites_do_not_disappear() {
        for points in [
            vec![],
            vec![(0.5, 0.5)],
            vec![(0.5, 0.5), (0.5, 0.5)],
            vec![(0.501, 0.501), (0.502, 0.502)],
        ] {
            let r = local_tree(test_key(), 4e6, &Flat, terminals(&points), points.len()).unwrap();
            connected(&r);
            assert_eq!(r.terminal_nodes.len(), points.len());
        }
    }
    struct Islands;
    impl Survey for Islands {
        fn ground(&self, d: DVec3) -> Result<Ground, String> {
            let (_, u, _) = planet_coretypes::direction_to_face_uv(d);
            let ocean = (u - 0.00390625).abs() < 0.001;
            Ok(Ground {
                height: if ocean { -200.0 } else { 900.0 },
                ocean,
            })
        }
        fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
            Ok(Vec::new())
        }
    }
    #[test]
    fn road_network_islands_are_connected_by_required_tunnels_not_surface_bridges() {
        let points = [(0.1, 0.4), (0.2, 0.6), (0.8, 0.4), (0.9, 0.6)];
        let r = local_tree(test_key(), 4e6, &Islands, terminals(&points), points.len()).unwrap();
        connected(&r);
        assert!(r.edges.iter().any(|e| e.requires_undersea_tunnel));
        for e in &r.edges {
            assert_eq!(
                e.requires_undersea_tunnel,
                r.nodes[e.a].ocean || r.nodes[e.b].ocean
            );
            if r.nodes[e.a].ocean {
                assert!(r.nodes[e.a].ground_meters < 0.0);
            }
        }
    }
    #[test]
    fn road_network_construction_cost_prefers_land_but_allows_a_shared_tunnel() {
        let land = Ground {
            height: 900.0,
            ocean: false,
        };
        let sea = Ground {
            height: -200.0,
            ocean: true,
        };
        assert!(edge_cost(1000.0, land, land) < edge_cost(1000.0, sea, sea));
        assert!(edge_cost(1000.0, sea, sea) < edge_cost(100000.0, land, land));
        assert!(
            edge_cost(
                1000.0,
                land,
                Ground {
                    height: 1600.0,
                    ocean: false
                }
            ) > edge_cost(1000.0, land, land)
        );
    }
    #[test]
    fn road_network_missing_or_invalid_ground_fails_instead_of_omitting_villages() {
        struct Missing;
        impl Survey for Missing {
            fn ground(&self, _: DVec3) -> Result<Ground, String> {
                Err("missing tile".into())
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
        }
        assert!(Backbone::survey(2, 4e6, &Missing).is_err());
        assert!(local_tree(test_key(), 4e6, &Missing, terminals(&[(0.1, 0.2)]), 1).is_err());
        assert!(Backbone::survey(9, 4e6, &Flat).is_err());
        assert!(Backbone::survey(1, f64::NAN, &Flat).is_err());
    }
}

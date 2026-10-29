//! Worker-only navigation on engineered roads, including non-resident regions.
//! The global and regional trees are expanded on demand; no distance horizon is
//! used. Geometry generation and navigation share profiles and bore flags.
use crate::{
    road_network::{self, Backbone, Survey, TerminalKind},
    road_network_mesh::Prepared,
};
use glam::DVec3;
use planet_coretypes::{TileKey, tile_key_for_direction};
use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BinaryHeap, HashMap, HashSet},
    sync::Arc,
};

type Id = (TileKey, usize);
type Point = [i64; 3];
fn point(p: DVec3) -> Point {
    p.to_array().map(|x| (x * 1000.0).round() as i64)
}

/// Names depend on the original site, not its approach, array order or cache.
/// Repeated place names are allowed, as in real geography; identity is the site.
pub fn village_name(site: DVec3) -> String {
    const FIRST: [&str; 32] = [
        "ALDER", "ASH", "BIRCH", "BRACKEN", "BRIAR", "BROOK", "CEDAR", "CLEAR", "COLD", "CROW",
        "DEEP", "ELDER", "ELM", "FAIR", "FERN", "FOX", "GLEN", "GREEN", "HAZEL", "HIGH", "HOLLY",
        "IVY", "LOW", "MAPLE", "MILL", "MOSS", "OAK", "PINE", "RAVEN", "RED", "ROWAN", "WILLOW",
    ];
    const LAST: [&str; 32] = [
        "BANK", "BECK", "BOROUGH", "BOURNE", "BRIDGE", "BROOK", "BURY", "CLIFF", "COMBE", "CROFT",
        "DALE", "DEN", "FIELD", "FORD", "GLEN", "GROVE", "HAM", "HAVEN", "HEATH", "HILL", "HOLLOW",
        "HOLT", "LEIGH", "MERE", "MOOR", "RIDGE", "STEAD", "STONE", "VALE", "WATER", "WELL",
        "WICK",
    ];
    let mut hash = 0xcbf29ce484222325_u64;
    for n in point(site.normalize() * 4_000_000.0) {
        for byte in n.to_le_bytes() {
            hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
        }
    }
    let base = format!(
        "{}{}",
        FIRST[(hash & 31) as usize],
        LAST[((hash >> 11) & 31) as usize]
    );
    const AREA: [&str; 16] = [
        "", "NORTH ", "SOUTH ", "EAST ", "WEST ", "UPPER ", "LOWER ", "LITTLE ", "GREAT ", "OLD ",
        "NEW ", "HIGH ", "LOW ", "NETHER ", "OVER ", "ST. ",
    ];
    format!("{}{base}", AREA[((hash >> 23) & 15) as usize])
}
#[derive(Clone)]
pub struct Destination {
    pub name: String,
    pub metres: f64,
    pub branched: bool,
    pub exit_position: Option<DVec3>,
}
#[derive(Clone)]
pub struct Edge {
    pub to: Id,
    pub path: Arc<Vec<DVec3>>,
    pub length: f64,
    pub tunnel: bool,
}
impl Edge {
    pub fn at(&self, distance: f64) -> (DVec3, DVec3) {
        let mut remaining = distance.clamp(0.0, self.length);
        for p in self.path.windows(2) {
            let length = p[0].distance(p[1]);
            if remaining <= length || p[1] == *self.path.last().unwrap() {
                return (
                    p[0].lerp(p[1], (remaining / length).clamp(0.0, 1.0)),
                    (p[1] - p[0]).normalize(),
                );
            }
            remaining -= length;
        }
        unreachable!("nonzero navigation chain")
    }
}
#[derive(Clone)]
struct Node {
    position: DVec3,
    village: Option<String>,
    gateways: Vec<TileKey>,
    edges: Vec<Edge>,
}
struct Region {
    nodes: Vec<Node>,
}
impl Region {
    fn new(plan: &Prepared, radius: f64) -> Self {
        let p = &plan.profile;
        let mut map = BTreeMap::new();
        let mut positions = Vec::new();
        let groups: Vec<_> = p
            .directions
            .iter()
            .zip(&p.heights_meters)
            .map(|(d, h)| {
                let position = DVec3::from_array(*d) * (radius + h);
                *map.entry(point(position)).or_insert_with(|| {
                    positions.push(position);
                    positions.len() - 1
                })
            })
            .collect();
        let mut adjacent = vec![Vec::new(); positions.len()];
        for (index, &[a, b]) in p.segments.iter().enumerate() {
            let (a, b) = (groups[a], groups[b]);
            if a != b && !adjacent[a].iter().any(|&(other, _)| other == b) {
                adjacent[a].push((b, plan.tunnels[index]));
                adjacent[b].push((a, plan.tunnels[index]));
            }
        }
        let mut villages = HashMap::new();
        let mut gateways = HashMap::<usize, Vec<TileKey>>::new();
        let mut centres = plan.village_centres.iter();
        let centre = road_network::direction(plan.region.key, 0.5, 0.5);
        for (t, &i) in plan
            .region
            .terminals
            .iter()
            .zip(&plan.region.terminal_nodes)
        {
            let group = groups[i];
            match t.kind {
                TerminalKind::Village => {
                    villages.insert(group, village_name(*centres.next().unwrap()));
                }
                TerminalKind::Gateway { .. } => {
                    let d = DVec3::from_array(t.direction);
                    gateways
                        .entry(group)
                        .or_default()
                        .push(tile_key_for_direction(
                            d.lerp(centre, -0.01).normalize(),
                            plan.region.key.level,
                        ));
                }
            }
        }
        let keep: BTreeMap<_, _> = adjacent
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                a.len() != 2
                    || villages.contains_key(i)
                    || gateways.contains_key(i)
                    || a[0].1 != a[1].1
            })
            .enumerate()
            .map(|(new, (old, _))| (old, new))
            .collect();
        let mut nodes = Vec::new();
        for (&old, &new) in &keep {
            let mut edges = Vec::new();
            for &(first, tunnel) in &adjacent[old] {
                let mut path = vec![positions[old]];
                let (mut previous, mut at) = (old, first);
                while !keep.contains_key(&at) {
                    path.push(positions[at]);
                    let next = adjacent[at].iter().find(|(i, _)| *i != previous).unwrap().0;
                    previous = at;
                    at = next;
                }
                path.push(positions[at]);
                let length = path.windows(2).map(|p| p[0].distance(p[1])).sum();
                edges.push(Edge {
                    to: (plan.region.key, keep[&at]),
                    path: Arc::new(path),
                    length,
                    tunnel,
                });
            }
            assert_eq!(nodes.len(), new);
            nodes.push(Node {
                position: positions[old],
                village: villages.remove(&old),
                gateways: gateways.remove(&old).unwrap_or_default(),
                edges,
            });
        }
        Self { nodes }
    }
}
#[derive(Clone, Copy)]
struct Queue {
    cost: f64,
    id: Id,
}
impl PartialEq for Queue {
    fn eq(&self, b: &Self) -> bool {
        self.cmp(b) == Ordering::Equal
    }
}
impl Eq for Queue {}
impl PartialOrd for Queue {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for Queue {
    fn cmp(&self, b: &Self) -> Ordering {
        self.cost.total_cmp(&b.cost).then(self.id.cmp(&b.id))
    }
}

#[derive(Default)]
pub struct Navigator {
    regions: HashMap<TileKey, Region>,
    rejected: HashMap<TileKey, String>,
}
impl Navigator {
    pub fn seed(&mut self, plans: &[Arc<Prepared>], radius: f64) {
        if self.regions.len() > 768 || self.rejected.len() > 768 {
            self.regions.clear();
            self.rejected.clear();
        }
        for p in plans {
            self.regions
                .entry(p.region.key)
                .or_insert_with(|| Region::new(p, radius));
        }
    }
    pub fn ids(&self, plans: &[Arc<Prepared>]) -> Vec<Id> {
        plans
            .iter()
            .flat_map(|p| {
                (0..self.regions[&p.region.key].nodes.len()).map(move |i| (p.region.key, i))
            })
            .collect()
    }
    pub fn position(&self, id: Id) -> DVec3 {
        self.regions[&id.0].nodes[id.1].position
    }
    pub fn name(&self, id: Id) -> Option<&str> {
        self.regions[&id.0].nodes[id.1].village.as_deref()
    }
    fn load(
        &mut self,
        key: TileKey,
        backbone: &Backbone,
        radius: f64,
        terrain: &impl Survey,
    ) -> Result<(), String> {
        if self.regions.contains_key(&key) {
            return Ok(());
        }
        if terrain.cancelled() {
            return Err("road generation superseded".into());
        }
        // Bounded metadata cache, not a radius cutoff. Old regions can be
        // reconstructed deterministically; no full meshes are retained here.
        if self.regions.len() >= 1024 {
            return Err("navigation metadata budget exhausted".into());
        }
        if let Some(error) = self.rejected.get(&key) {
            return Err(error.clone());
        }
        let result = crate::road_network_mesh::prepare_local(backbone, key, radius, terrain);
        let p = match result {
            Ok(p) => p,
            Err(error) => {
                if !terrain.cancelled() {
                    self.rejected.insert(key, error.clone());
                }
                return Err(error);
            }
        };
        self.regions.insert(key, Region::new(&p, radius));
        Ok(())
    }
    fn linked(
        &mut self,
        id: Id,
        backbone: &Backbone,
        radius: f64,
        terrain: &impl Survey,
    ) -> Result<Vec<Id>, String> {
        let node = self.regions[&id.0].nodes[id.1].clone();
        let mut result = Vec::new();
        for key in node.gateways {
            self.load(key, backbone, radius, terrain)?;
            let other = self.regions[&key]
                .nodes
                .iter()
                .enumerate()
                .find(|(_, n)| n.position.distance(node.position) < 0.01)
                .map(|(i, _)| (key, i))
                .ok_or("navigation gateway does not match neighbouring profile")?;
            result.push(other);
        }
        Ok(result)
    }
    /// Merge coincident regional gateway aliases before counting junction arms.
    pub fn arms(
        &mut self,
        id: Id,
        backbone: &Backbone,
        radius: f64,
        terrain: &impl Survey,
    ) -> Result<Vec<Edge>, String> {
        let mut pending = vec![id];
        let mut seen = HashSet::new();
        let mut arms = Vec::new();
        while let Some(at) = pending.pop() {
            if !seen.insert(at) {
                continue;
            }
            arms.extend(self.regions[&at.0].nodes[at.1].edges.clone());
            pending.extend(self.linked(at, backbone, radius, terrain)?);
        }
        Ok(arms)
    }
    /// Exact shortest engineered distance in the requested outgoing branch.
    /// Bore mode stops at the nearest REAL exit, never a resident-set boundary.
    pub fn next(
        &mut self,
        start: Id,
        blocked: DVec3,
        bore: bool,
        backbone: &Backbone,
        radius: f64,
        terrain: &impl Survey,
    ) -> Result<Option<Destination>, String> {
        let mut heap = BinaryHeap::from([Reverse(Queue {
            cost: 0.0,
            id: start,
        })]);
        let mut distances = HashMap::from([(start, 0.0)]);
        let mut branched = false;
        while let Some(Reverse(q)) = heap.pop() {
            if terrain.cancelled() {
                return Err("road generation superseded".into());
            }
            if distances.len() > 100_000 {
                return Err("navigation search budget exhausted".into());
            }
            if q.cost > distances[&q.id] {
                continue;
            }
            let node = self.regions[&q.id.0].nodes[q.id.1].clone();
            if bore {
                if node.edges.iter().any(|e| !e.tunnel) {
                    return Ok(Some(Destination {
                        name: "TUNNEL".into(),
                        metres: q.cost,
                        branched,
                        exit_position: Some(node.position),
                    }));
                }
                branched |= node.edges.iter().filter(|e| e.tunnel).count() > 2;
            } else if let Some(name) = node.village {
                return Ok(Some(Destination {
                    name,
                    metres: q.cost + crate::road_parking::OFFSET,
                    branched: false,
                    exit_position: None,
                }));
            }
            let mut next: Vec<_> = node
                .edges
                .iter()
                .filter(|e| !bore || e.tunnel)
                .map(|e| (e.to, e.length))
                .collect();
            next.extend(
                self.linked(q.id, backbone, radius, terrain)?
                    .into_iter()
                    .map(|id| (id, 0.0)),
            );
            for (id, length) in next {
                if self.position(id).distance(blocked) < 0.01 {
                    continue;
                }
                let cost = q.cost + length;
                if distances.get(&id).is_none_or(|old| cost < *old) {
                    distances.insert(id, cost);
                    heap.push(Reverse(Queue { cost, id }));
                }
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Junction,
    Distance,
    Village,
    Tunnel,
    Branch,
    /// The timber gateway over a village footpath where it leaves its court.
    Footpath,
}
#[derive(Clone)]
pub struct Board {
    pub kind: Kind,
    pub centre: DVec3,
    pub right: DVec3,
    pub up: DVec3,
    pub lines: Vec<String>,
    pub width: f64,
    pub village: bool,
    pub support: Option<DVec3>,
}
pub fn distance(metres: f64) -> String {
    if metres < 1000.0 {
        format!("{} M", ((metres / 10.0).round() * 10.0).max(10.0) as u64)
    } else if metres < 10_000.0 {
        format!("{:.1} KM", metres / 1000.0)
    } else {
        format!("{:.0} KM", metres / 1000.0)
    }
}
/// Both faces share one station; distances follow the engineered bore to real
/// exits, including unloaded regions. Anchor stations to an exit's 500m marks,
/// rather than restarting the spacing at every regional gateway.
fn tunnel_boards(edge: &Edge, ahead: &Destination, behind: &Destination) -> Vec<Board> {
    let total_ahead = edge.length + ahead.metres;
    // A canonical real exit keeps the phase stable even when regional
    // edge order reverses. A branched bore uses its two directed next exits.
    let anchor_ahead = match (ahead.exit_position, behind.exit_position) {
        (Some(a), Some(b)) => point(a) >= point(b),
        _ => true,
    };
    let mut station = if anchor_ahead {
        total_ahead
    } else {
        -behind.metres
    }
    .rem_euclid(500.0);
    if station < 0.01 || station > 499.99 {
        station = 0.0;
    }
    let mut boards = Vec::new();
    while station <= edge.length + 0.01 {
        let forward_left = total_ahead - station;
        let backward_left = behind.metres + station;
        // Portal headers already cover the first/last few metres.
        if forward_left >= 75.0 && backward_left >= 75.0 {
            let (at, forward) = edge.at(station);
            let up = at.normalize();
            for (direction, left) in [(forward, forward_left), (-forward, backward_left)] {
                boards.push(Board {
                    kind: Kind::Tunnel,
                    centre: at - direction * 0.08 + up * 6.7,
                    right: direction.cross(up).normalize(),
                    up,
                    lines: vec![format!("EXIT {}", tunnel_distance(left))],
                    width: 5.5,
                    village: false,
                    support: None,
                });
            }
        }
        station += 500.0;
    }
    boards
}
fn tunnel_distance(metres: f64) -> String {
    if metres < 500.0 {
        return distance(metres);
    }
    let rounded = (metres / 500.0).round().max(1.0) * 500.0;
    if rounded < 1000.0 {
        "500 M".into()
    } else {
        format!("{:.1} KM", rounded / 1000.0)
    }
}

fn roadside(p: DVec3, forward: DVec3, lines: Vec<String>, inside: bool, village: bool) -> Board {
    let up = p.normalize();
    let right = forward.cross(up).normalize();
    let width = if inside { 5.0 } else { 6.0 };
    let verge = if village {
        crate::road_parking::RADIUS + 0.75
    } else {
        4.8
    };
    Board {
        kind: if village {
            Kind::Village
        } else if lines.len() > 1 {
            Kind::Junction
        } else {
            Kind::Distance
        },
        centre: p + up * if inside { 6.7 } else { 3.4 }
            - right
                * if inside {
                    0.0
                } else {
                    verge + 0.8 + width * 0.5
                },
        right,
        up,
        lines,
        width,
        village,
        support: (!inside).then_some(p - right * verge - up * 0.6),
    }
}
fn village_sign(access: DVec3, toward_road: DVec3, name: &str) -> Board {
    // Match the court beyond the terminal, not the narrow approach road.
    let up = access.normalize();
    let toward = (toward_road - up * toward_road.dot(up)).normalize();
    let court = access - toward * crate::road_parking::OFFSET;
    roadside(court, -toward, vec![name.into()], false, true)
}

/// Clear height under a footpath gateway's board, and half the gap between
/// its posts (wider than the widest shared lane, 1.4m each side).
pub const FOOTPATH_GATE_CLEARANCE: f64 = 2.25;
pub const FOOTPATH_GATE_HALF_SPAN: f64 = 1.9;

/// A gateway over the footpath where it leaves the parking court: two posts
/// either side of the path, a crossbeam, and a hanging board naming the
/// village, facing back into the court. `entry` is on the court floor at the
/// path mouth; `outward` points from the court centre along the path.
pub fn footpath_gate(entry: DVec3, outward: DVec3, name: &str) -> Board {
    let up = entry.normalize();
    let outward = (outward - up * outward.dot(up)).normalize();
    let lines = vec![name.to_owned(), "FOOTPATH".to_owned()];
    let height = 0.30 + 0.62 * lines.len() as f64;
    Board {
        kind: Kind::Footpath,
        centre: entry + up * (FOOTPATH_GATE_CLEARANCE + height / 2.0),
        right: outward.cross(up),
        up,
        lines,
        width: 2.0 * FOOTPATH_GATE_HALF_SPAN - 0.4,
        village: true,
        support: None,
    }
}

/// A finger board on the departure branch, readable from either main-road
/// approach. Reversing both the text plane and arrow keeps both faces pointing
/// physically towards the same village. Only the first face owns the post.
fn branch_signs(edge: &Edge, destination: &Destination) -> [Board; 2] {
    let station = 8.0_f64.min(edge.length * 0.25);
    let (at, forward) = edge.at(station);
    let up = at.normalize();
    let along = (forward - up * forward.dot(up)).normalize();
    let side = along.cross(up);
    let centre = at - side * if edge.tunnel { 5.8 } else { 6.8 } + up * 3.4;
    let metres = distance(destination.metres + edge.length - station);
    [false, true].map(|back| Board {
        kind: Kind::Branch,
        centre: centre + side * if back { -0.05 } else { 0.05 },
        right: if back { -along } else { along },
        up,
        lines: vec![if back {
            format!("< {} {metres}", destination.name)
        } else {
            format!("{} {metres} >", destination.name)
        }],
        width: if edge.tunnel { 4.5 } else { 6.0 },
        village: false,
        support: (!back).then_some(at - side * 4.8 - up * 0.6),
    })
}

pub fn boards(
    nav: &mut Navigator,
    plans: &[Arc<Prepared>],
    eye: DVec3,
    radius: f64,
    backbone: &Backbone,
    terrain: &impl Survey,
) -> Result<Vec<Board>, String> {
    nav.seed(plans, radius);
    let ids = nav.ids(plans);
    let mut seen = HashSet::new();
    let mut boards = Vec::new();
    let mut failures = 0;
    let mut tunnel_stations = HashSet::new();
    for id in ids {
        if terrain.cancelled() {
            return Err("road generation superseded".into());
        }
        let p = nav.position(id);
        // Only visible-neighbourhood signs are built, but their route searches
        // have no distance horizon and can cross arbitrarily unloaded regions.
        let near = p.distance(eye) < 1800.0;
        if !near
            && !nav.regions[&id.0].nodes[id.1]
                .edges
                .iter()
                .any(|e| e.path.iter().any(|p| p.distance(eye) < 1800.0))
        {
            continue;
        }
        if !seen.insert(point(p)) {
            continue;
        }
        let arms = match nav.arms(id, backbone, radius, terrain) {
            Ok(a) => a,
            Err(e) => {
                failures += 1;
                tracing::debug!(%e,"road sign navigation unresolved");
                continue;
            }
        };
        if let Some(name) = nav.name(id).filter(|_| near) {
            if let Some(e) = arms.first() {
                let (_, f) = e.at(0.0);
                boards.push(village_sign(p, f, name));
            }
        }
        if near && arms.len() > 2 {
            for incoming in &arms {
                let approach = 40.0_f64.min(incoming.length * 0.4);
                let (at, tangent) = incoming.at(approach);
                let forward = -tangent;
                let right = forward.cross(at.normalize()).normalize();
                let mut lines = Vec::new();
                for outgoing in &arms {
                    if outgoing.to == incoming.to {
                        continue;
                    }
                    match nav.next(outgoing.to, p, false, backbone, radius, terrain) {
                        Ok(Some(d)) => {
                            let (_, toward) = outgoing.at(0.0);
                            let arrow = if toward.dot(forward) > 0.82 {
                                "^"
                            } else if toward.dot(right) > 0.0 {
                                ">"
                            } else {
                                "<"
                            };
                            lines.push(format!(
                                "{arrow} {} {}",
                                d.name,
                                distance(d.metres + outgoing.length + approach)
                            ));
                        }
                        Ok(None) => {}
                        Err(_) => {
                            failures += 1;
                        }
                    }
                }
                if !lines.is_empty() {
                    boards.push(roadside(at, forward, lines, incoming.tunnel, false));
                }
            }
        }
        if near && arms.len() > 2 {
            for outgoing in &arms {
                match nav.next(outgoing.to, p, false, backbone, radius, terrain) {
                    Ok(Some(destination)) => boards.extend(branch_signs(outgoing, &destination)),
                    Ok(None) => {}
                    Err(_) => failures += 1,
                }
            }
        }
        if near && arms.iter().any(|e| !e.tunnel) {
            for e in arms.iter().filter(|e| e.tunnel) {
                match nav.next(e.to, p, true, backbone, radius, terrain) {
                    Ok(Some(d)) => {
                        let (_, forward) = e.at(0.0);
                        let up = p.normalize();
                        boards.push(Board {
                            kind: Kind::Tunnel,
                            centre: p - forward * 0.4 + up * 7.5,
                            right: forward.cross(up).normalize(),
                            up,
                            lines: vec![format!(
                                "{} {}",
                                if d.branched { "NEXT EXIT" } else { "" },
                                distance(d.metres + e.length)
                            )],
                            width: 5.5,
                            village: false,
                            support: None,
                        });
                    }
                    Ok(None) => {}
                    Err(_) => {
                        failures += 1;
                    }
                }
            }
        }
        for e in &arms {
            if point(p) >= point(nav.position(e.to)) {
                continue;
            }
            if e.tunnel {
                let ahead = nav.next(e.to, p, true, backbone, radius, terrain);
                let behind = nav.next(id, nav.position(e.to), true, backbone, radius, terrain);
                match (ahead, behind) {
                    (Ok(Some(a)), Ok(Some(b))) => {
                        let paired = tunnel_boards(e, &a, &b);
                        for pair in paired.chunks_exact(2) {
                            let centre = (pair[0].centre + pair[1].centre) * 0.5;
                            if centre.distance(eye) < 1800.0
                                && tunnel_stations.insert(point(centre))
                            {
                                boards.extend(pair.iter().cloned());
                            }
                        }
                    }
                    (Err(_), _) | (_, Err(_)) => failures += 1,
                    _ => {}
                }
                continue;
            }
            let mut station = 1000.0;
            while station < e.length - 150.0 {
                let (at, forward) = e.at(station);
                if at.distance(eye) < 1800.0 {
                    for reverse in [false, true] {
                        let (start, block, remaining) = if reverse {
                            (id, nav.position(e.to), station)
                        } else {
                            (e.to, p, e.length - station)
                        };
                        match nav.next(start, block, false, backbone, radius, terrain) {
                            Ok(Some(d)) => boards.push(roadside(
                                at,
                                if reverse { -forward } else { forward },
                                vec![format!("{} {}", d.name, distance(d.metres + remaining))],
                                e.tunnel,
                                false,
                            )),
                            Ok(None) => {}
                            Err(_) => {
                                failures += 1;
                            }
                        }
                    }
                }
                station += 1000.0;
            }
        }
        if boards.len() >= 128 {
            break;
        }
    }
    tracing::info!(
        boards = boards.len(),
        unresolved = failures,
        regions = nav.regions.len(),
        "road signs planned on engineered network"
    );
    Ok(boards)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "asset-backed undersea distance boards; no window/GPU"]
    fn road_network_undersea_distance_boards_show_both_real_exits() {
        struct Coast(crate::road_network_survey::Surface);
        impl Survey for Coast {
            fn ground(&self, d: DVec3) -> Result<road_network::Ground, String> {
                self.0.engineering_ground(d)
            }
            fn villages(&self, key: TileKey) -> Result<Vec<DVec3>, String> {
                self.0.villages(key)
            }
            fn village_access_height(&self, g: road_network::Ground) -> f64 {
                g.height + 0.25
            }
            fn settlement_core_at(&self, d: DVec3) -> Result<bool, String> {
                self.0.settlement_core_at(d)
            }
        }
        let coast = Coast(
            crate::road_network_survey::Surface::load(&crate::find_default_outmap().unwrap())
                .unwrap(),
        );
        let backbone = coast.0.backbone().unwrap();
        let eye = crate::road_network_render::longest_undersea_start_position();
        let radius = crate::planet::planet_radius_meters();
        let plan = Arc::new(
            crate::road_network_mesh::prepare_local(
                &backbone,
                backbone.region_key(eye),
                radius,
                &coast,
            )
            .unwrap(),
        );
        let boards = boards(
            &mut Navigator::default(),
            &[plan],
            eye,
            radius,
            &backbone,
            &coast,
        )
        .unwrap();
        let internal: Vec<_> = boards
            .iter()
            .filter(|b| b.kind == Kind::Tunnel && b.lines[0].starts_with("EXIT "))
            .collect();
        assert!(
            internal.len() >= 4,
            "must include paired internal boards, not just portal headers"
        );
        for pair in internal.chunks_exact(2) {
            assert!(pair[0].centre.distance(pair[1].centre) < 0.2);
            assert!(pair[0].right.dot(pair[1].right) < -0.999);
            for b in pair {
                let forward = b.up.cross(b.right).normalize();
                let eye = b.centre - forward * 18.0 - b.up * 5.3;
                let target = b.centre;
                eprintln!("tunnel board {:?}, eye={eye:?}, target={target:?}", b.lines);
            }
        }
    }
    #[test]
    fn tunnel_boards_are_paired_and_keep_exit_anchored_spacing_across_gateways() {
        let radius = 4e6;
        let edge = Edge {
            to: (
                TileKey {
                    face: planet_coretypes::CubeFace::PositiveX,
                    level: 0,
                    x: 0,
                    y: 0,
                },
                0,
            ),
            path: Arc::new(vec![
                DVec3::new(radius, 0.0, 0.0),
                DVec3::new(radius, 1600.0, 0.0),
            ]),
            length: 1600.0,
            tunnel: true,
        };
        let d = |metres| Destination {
            name: "TUNNEL".into(),
            metres,
            branched: false,
            exit_position: None,
        };
        let boards = tunnel_boards(&edge, &d(900.0), &d(1250.0));
        assert_eq!(boards.len(), 8);
        for (pair, station) in boards.chunks_exact(2).zip([0.0, 500.0, 1000.0, 1500.0]) {
            assert!((pair[0].right.dot(pair[1].right) + 1.0).abs() < 1e-10);
            assert_eq!(
                pair[0].lines,
                vec![format!("EXIT {}", tunnel_distance(2500.0 - station))]
            );
            assert_eq!(
                pair[1].lines,
                vec![format!("EXIT {}", tunnel_distance(1250.0 + station))]
            );
            assert!(((pair[0].centre + pair[1].centre) * 0.5).y > station - 0.1);
        }
        // Reversing regional edge order must not shift the 500m stations.
        let mut ahead = d(900.0);
        ahead.exit_position = Some(DVec3::new(radius, 5000.0, 0.0));
        let mut behind = d(1250.0);
        behind.exit_position = Some(DVec3::new(radius, -5000.0, 0.0));
        let reverse = Edge {
            path: Arc::new(edge.path.iter().copied().rev().collect()),
            ..edge.clone()
        };
        let centres = |boards: Vec<Board>| -> std::collections::BTreeSet<Point> {
            boards
                .chunks_exact(2)
                .map(|p| point((p[0].centre + p[1].centre) * 0.5))
                .collect()
        };
        let original = centres(tunnel_boards(&edge, &ahead, &behind));
        assert_eq!(original, centres(tunnel_boards(&reverse, &behind, &ahead)));
        // A station exactly on a gateway is retained (the caller de-duplicates).
        let split_a = Edge {
            path: Arc::new(vec![edge.path[0], DVec3::new(radius, 1000.0, 0.0)]),
            length: 1000.0,
            ..edge.clone()
        };
        let split_b = Edge {
            path: Arc::new(vec![DVec3::new(radius, 1000.0, 0.0), edge.path[1]]),
            length: 600.0,
            ..edge.clone()
        };
        let mut rest_a = ahead.clone();
        rest_a.metres += 600.0;
        let mut rest_b = behind.clone();
        rest_b.metres += 1000.0;
        let mut split = centres(tunnel_boards(&split_a, &rest_a, &behind));
        split.extend(centres(tunnel_boards(&split_b, &ahead, &rest_b)));
        assert_eq!(original, split);
        assert_eq!(tunnel_distance(19_500.0), "19.5 KM");
        assert_eq!(tunnel_distance(250.0), "250 M");
        assert_eq!(tunnel_distance(500.0), "500 M");
    }

    #[test]
    fn village_nameplate_is_outside_the_parking_circle_and_approach() {
        let access = DVec3::Z * 4e6;
        let toward_road = DVec3::Y;
        let court = access - toward_road * crate::road_parking::OFFSET;
        let board = village_sign(access, toward_road, "NORTH HIGHFORD");
        let post = board.support.unwrap();
        let horizontal = |p: DVec3| (p - court).with_z(0.0).length();
        assert!(
            horizontal(post) >= crate::road_parking::RADIUS + 0.5,
            "nameplate post occupies the parking/turning circle: {}m",
            horizontal(post)
        );
        assert!(horizontal(post) <= crate::road_parking::RADIUS + 1.0);
        for edge in [-0.5, 0.5] {
            assert!(
                horizontal(board.centre + board.right * (edge * board.width))
                    > crate::road_parking::RADIUS
            );
        }
        assert!(board.right.cross(board.up).dot(toward_road) > 0.99);
    }

    #[test]
    fn footpath_gate_faces_the_court_and_leaves_headroom_over_the_path() {
        let entry = DVec3::Z * 4e6;
        let outward = DVec3::X;
        let gate = footpath_gate(entry, outward, "OAK HOLLOW");
        let normal = gate.right.cross(gate.up);
        assert!(normal.dot(-outward) > 0.999, "board must face back into the court");
        let height = 0.30 + 0.62 * 2.0;
        let bottom = (gate.centre - entry).dot(gate.up) - height / 2.0;
        assert!((bottom - FOOTPATH_GATE_CLEARANCE).abs() < 1e-9);
        assert!(gate.width / 2.0 < FOOTPATH_GATE_HALF_SPAN);
        assert!(FOOTPATH_GATE_HALF_SPAN > 1.4 + 0.3, "posts must clear the widest lane");
        assert_eq!(gate.lines, ["OAK HOLLOW", "FOOTPATH"]);
    }

    #[test]
    fn branch_fingers_name_the_destination_and_point_the_same_way_on_both_faces() {
        let a = DVec3::Z * 4e6;
        let b = a + DVec3::Y * 100.0;
        let edge = Edge {
            to: (
                TileKey {
                    face: planet_coretypes::CubeFace::PositiveZ,
                    level: 8,
                    x: 1,
                    y: 1,
                },
                0,
            ),
            path: Arc::new(vec![a, b]),
            length: 100.0,
            tunnel: false,
        };
        let d = Destination {
            name: "OAKFORD".into(),
            exit_position: None,
            metres: 15.0,
            branched: false,
        };
        let [front, back] = branch_signs(&edge, &d);
        assert!(front.lines[0].contains("OAKFORD") && back.lines[0].contains("OAKFORD"));
        assert!(front.lines[0].ends_with('>') && back.lines[0].starts_with('<'));
        assert!(front.right.dot(DVec3::Y) > 0.99 && back.right.dot(DVec3::Y) < -0.99);
        assert!(front.centre.x < -5.0 && front.support.is_some() && back.support.is_none());
    }
    #[test]
    fn nearest_destinations_and_bore_lengths_cross_unloaded_geometry_boundaries() {
        struct Flat;
        impl Survey for Flat {
            fn ground(&self, _: DVec3) -> Result<road_network::Ground, String> {
                Ok(road_network::Ground {
                    height: 900.0,
                    ocean: false,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
        }
        let a = TileKey {
            face: planet_coretypes::CubeFace::PositiveX,
            level: 1,
            x: 0,
            y: 0,
        };
        let b = TileKey { x: 1, ..a };
        let p = DVec3::X * 4e6;
        let positions = [
            p,
            p + DVec3::Y * 100.0,
            p + DVec3::Y * 200_100.0,
            p + DVec3::Y * 270_100.0,
        ];
        let edge = |from: usize, to: Id, end: usize, tunnel: bool| Edge {
            to,
            path: Arc::new(vec![positions[from], positions[end]]),
            length: positions[from].distance(positions[end]),
            tunnel,
        };
        let mut nav = Navigator::default();
        nav.regions.insert(
            a,
            Region {
                nodes: vec![
                    Node {
                        position: positions[0],
                        village: Some("NEAR BUT BEHIND".into()),
                        gateways: vec![],
                        edges: vec![edge(0, (a, 1), 1, true)],
                    },
                    Node {
                        position: positions[1],
                        village: None,
                        gateways: vec![b],
                        edges: vec![edge(1, (a, 0), 0, true)],
                    },
                ],
            },
        );
        // This region has navigation metadata, but need not have ANY resident
        // terrain/road meshes. Its real exit is two hundred kilometres away.
        nav.regions.insert(
            b,
            Region {
                nodes: vec![
                    Node {
                        position: positions[1],
                        village: None,
                        gateways: vec![a],
                        edges: vec![edge(1, (b, 1), 2, true)],
                    },
                    Node {
                        position: positions[2],
                        village: None,
                        gateways: vec![],
                        edges: vec![edge(2, (b, 0), 1, true), edge(2, (b, 2), 3, false)],
                    },
                    Node {
                        position: positions[3],
                        village: Some("FAR FORWARD".into()),
                        gateways: vec![],
                        edges: vec![edge(3, (b, 1), 2, false)],
                    },
                ],
            },
        );
        let backbone = Backbone::survey(1, 4e6, &Flat).unwrap();
        let bore = nav
            .next((a, 1), p, true, &backbone, 4e6, &Flat)
            .unwrap()
            .unwrap();
        assert_eq!(bore.metres, 200_000.0, "a gateway is not a tunnel exit");
        let village = nav
            .next((a, 1), p, false, &backbone, 4e6, &Flat)
            .unwrap()
            .unwrap();
        assert_eq!(
            village.name, "FAR FORWARD",
            "do not point backwards to a nearer village"
        );
        assert_eq!(village.metres, 270_000.0 + crate::road_parking::OFFSET);
    }
    #[test]
    fn names_are_site_stable_and_distances_are_not_capped() {
        let p = DVec3::new(1.0, 2.0, 3.0);
        assert_eq!(village_name(p), village_name(p * 2.0));
        assert_ne!(village_name(p), village_name(DVec3::new(2.0, 3.0, 1.0)));
        assert_eq!(distance(123_456_000.0), "123456 KM");
    }
    #[test]
    fn signs_are_on_the_left_and_face_approaching_traffic() {
        let b = roadside(
            DVec3::Z * 4e6,
            DVec3::Y,
            vec!["OAKFORD 12 KM".into()],
            false,
            false,
        );
        assert!(b.centre.x < -7.0);
        assert!(b.right.cross(b.up).dot(-DVec3::Y) > 0.99);
    }
}

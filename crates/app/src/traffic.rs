//! Bounded resident-road traffic. Topology, A* and supported lane trajectories
//! are built on one worker; the render thread advances cached journeys at 30Hz.
use crate::road_network_field::Field;
use glam::DVec3;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BinaryHeap, HashMap},
    sync::{Arc, Condvar, Mutex},
};

pub const STEP: f64 = 1.0 / 30.0;
pub const MAX_AGENTS: usize = 64;
// Simulation seconds, not wall seconds. Covers 40x even at 5 FPS, while
// bounding catch-up to ~240 safe fixed steps at extreme time multipliers.
const MAX_ADVANCE_SECONDS: f64 = 8.0;
const MAX_ROUTES: usize = 16;
const LANE: f64 = 2.05;
// Includes the outward sweep of a steered front wheel.
const HALF_WIDTH: f64 = 1.35;
const BRAKE: f64 = 3.0;
type Key = [i64; 3];
fn key(p: DVec3, scale: f64) -> Key {
    p.to_array().map(|x| (x * scale).round() as i64)
}
fn hash(mut n: u64) -> u64 {
    n ^= n >> 30;
    n = n.wrapping_mul(0xbf58476d1ce4e5b9);
    n ^= n >> 27;
    n = n.wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}
fn unit(n: u64) -> f64 {
    (hash(n) >> 11) as f64 / (1u64 << 53) as f64
}
pub fn enabled_for(scenario: Option<&str>) -> bool {
    crate::subsystem_enabled("traffic")
        && match std::env::var("PLANET_TRAFFIC").as_deref() {
            Ok("0" | "off") => false,
            Ok("1" | "on") => true,
            _ => {
                scenario.is_none()
                    || matches!(scenario, Some("road_traffic" | "road_traffic_tunnel" | "cart_drivers" | "cart_driver_stop"))
            }
        }
}

fn population() -> usize {
    std::env::var("PLANET_TRAFFIC_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(32usize)
        .min(MAX_AGENTS)
}

#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub position: DVec3,
    pub forward: DVec3,
    pub tunnel: bool,
    pub light: u32,
    pub limit: f64,
}
#[derive(Clone)]
struct Sample {
    pose: Pose,
    metres: f64,
}
#[derive(Clone)]
struct Zone {
    id: Key,
    enter: f64,
    exit: f64,
    through: bool,
}
pub struct Route {
    samples: Vec<Sample>,
    zones: Vec<Zone>,
    stops: Vec<f64>,
    pub length: f64,
    closed: bool,
    source: String,
    destination: String,
    /// The village whose residents drive this route, if it touches one.
    home: Option<String>,
}
impl Route {
    pub fn at(&self, metres: f64) -> Pose {
        let s = if self.closed {
            metres.rem_euclid(self.length)
        } else {
            metres.clamp(0.0, self.length)
        };
        let i = self
            .samples
            .partition_point(|p| p.metres <= s)
            .saturating_sub(1)
            .min(self.samples.len() - 2);
        let a = &self.samples[i];
        let b = &self.samples[i + 1];
        let t = ((s - a.metres) / (b.metres - a.metres)).clamp(0.0, 1.0);
        Pose {
            position: a.pose.position.lerp(b.pose.position, t),
            forward: a.pose.forward.lerp(b.pose.forward, t).normalize_or_zero(),
            tunnel: a.pose.tunnel,
            light: a.pose.light,
            limit: a.pose.limit.min(b.pose.limit),
        }
    }
    fn next_zone(&self, d: f64) -> Option<(usize, f64, f64)> {
        let lap = (d / self.length).floor();
        self.zones
            .iter()
            .enumerate()
            .flat_map(|(i, z)| {
                [lap - 1.0, lap, lap + 1.0]
                    .into_iter()
                    .filter(move |&l| self.closed || l == 0.0)
                    .map(move |l| (i, z.enter + l * self.length, z.exit + l * self.length))
            })
            .filter(|&(_, _, exit)| exit > d - 0.01)
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
    fn next_stop(&self, d: f64, served: i64) -> Option<(i64, f64)> {
        let lap = (d / self.length).floor() as i64;
        self.stops
            .iter()
            .enumerate()
            .flat_map(|(i, &s)| {
                [lap, lap + 1]
                    .into_iter()
                    .filter(move |&l| self.closed || l == 0)
                    .map(move |l| {
                        (
                            l * self.stops.len() as i64 + i as i64,
                            s + l as f64 * self.length,
                        )
                    })
            })
            .filter(|&(id, s)| id != served && s >= d - 0.05)
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}
#[derive(Clone)]
struct Node {
    position: DVec3,
    links: Vec<(usize, f64, bool)>,
}
struct Graph {
    nodes: Vec<Node>,
    ends: Vec<End>,
}
#[derive(Clone)]
struct End {
    node: usize,
    court: Option<DVec3>,
    name: String,
}
#[derive(Clone, Copy)]
struct Queue {
    cost: f64,
    node: usize,
}
impl PartialEq for Queue {
    fn eq(&self, b: &Self) -> bool {
        self.cost == b.cost && self.node == b.node
    }
}
impl Eq for Queue {}
impl Ord for Queue {
    fn cmp(&self, b: &Self) -> Ordering {
        b.cost.total_cmp(&self.cost).then(b.node.cmp(&self.node))
    }
}
impl PartialOrd for Queue {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Graph {
    fn build(field: &Field) -> Self {
        let mut nodes: Vec<Node> = Vec::new();
        let mut buckets = HashMap::<Key, Vec<usize>>::new();
        let mut node = |p: DVec3| {
            let cell = p.to_array().map(|x| (x / 0.05).floor() as i64);
            for z in -1..=1 {
                for y in -1..=1 {
                    for x in -1..=1 {
                        let q = [cell[0] + x, cell[1] + y, cell[2] + z];
                        if let Some(list) = buckets.get(&q) {
                            for &i in list {
                                if nodes[i].position.distance_squared(p) < 0.02 * 0.02 {
                                    return i;
                                }
                            }
                        }
                    }
                }
            }
            let i = nodes.len();
            nodes.push(Node {
                position: p,
                links: Vec::new(),
            });
            buckets.entry(cell).or_default().push(i);
            i
        };
        let mut edges = Vec::new();
        for s in &field.segments {
            if !s.is_parking() {
                let a = node(field.frame.world(s.a));
                let b = node(field.frame.world(s.b));
                if a != b {
                    edges.push((a, b, s.tunnel));
                }
            }
        }
        for (a, b, tunnel) in edges {
            if !nodes[a].links.iter().any(|&(i, _, _)| i == b) {
                let length = nodes[a].position.distance(nodes[b].position);
                nodes[a].links.push((b, length, tunnel));
                nodes[b].links.push((a, length, tunnel));
            }
        }
        let mut ends = Vec::new();
        for &(site, court) in &field.village_parking {
            if let Some((i, _)) = nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.links.len() == 1)
                .map(|(i, n)| (i, n.position.distance_squared(court)))
                .filter(|&(_, d)| d < 24.0 * 24.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
            {
                if !ends.iter().any(|e: &End| e.node == i) {
                    ends.push(End {
                        node: i,
                        court: Some(court),
                        name: crate::road_navigation::village_name(site),
                    });
                }
            }
        }
        for (i, n) in nodes.iter().enumerate() {
            if n.links.len() == 1 && !ends.iter().any(|e| e.node == i) {
                ends.push(End {
                    node: i,
                    court: None,
                    name: "ONWARD ROAD".into(),
                });
            }
        }
        Self { nodes, ends }
    }
    /// Euclidean chord distance is admissible for the sum of engineered 3D lengths.
    fn shortest(
        &self,
        start: usize,
        goal: usize,
        cancelled: &impl Fn() -> bool,
    ) -> Option<Vec<usize>> {
        let mut distance = vec![f64::INFINITY; self.nodes.len()];
        let mut parent = vec![usize::MAX; self.nodes.len()];
        let mut queue = BinaryHeap::new();
        distance[start] = 0.0;
        queue.push(Queue {
            cost: 0.0,
            node: start,
        });
        let mut visits = 0;
        while let Some(Queue { cost, node }) = queue.pop() {
            visits += 1;
            if visits % 256 == 1 && cancelled() {
                return None;
            }
            let h = self.nodes[node]
                .position
                .distance(self.nodes[goal].position);
            if node != start && cost > distance[node] + h + 1e-6 {
                continue;
            }
            if node == goal {
                let mut path = vec![goal];
                while *path.last().unwrap() != start {
                    path.push(parent[*path.last().unwrap()]);
                }
                path.reverse();
                return Some(path);
            }
            for &(to, length, _) in &self.nodes[node].links {
                let d = distance[node] + length;
                if d < distance[to] {
                    distance[to] = d;
                    parent[to] = node;
                    queue.push(Queue {
                        cost: d + self.nodes[to].position.distance(self.nodes[goal].position),
                        node: to,
                    });
                }
            }
        }
        None
    }
}

pub struct Lamps {
    cells: HashMap<Key, Vec<(DVec3, u32)>>,
}
impl Lamps {
    pub fn new(data: &[[f32; 4]]) -> Self {
        let mut unique = BTreeMap::<Key, (DVec3, u32)>::new();
        for (i, group) in data.chunks_exact(25).enumerate() {
            for p in group.iter().skip(1).take((group[0][0] as usize).min(24)) {
                let position = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64);
                unique
                    .entry(key(position, 100.0))
                    .or_insert((position, i as u32));
            }
        }
        let mut cells = HashMap::<Key, Vec<_>>::new();
        for (_, entry) in unique {
            cells
                .entry(entry.0.to_array().map(|x| (x / 32.0).floor() as i64))
                .or_default()
                .push(entry);
        }
        Self { cells }
    }
    fn group(&self, p: DVec3) -> u32 {
        let c = p.to_array().map(|x| (x / 32.0).floor() as i64);
        let mut best = (f64::INFINITY, 0);
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    if let Some(list) = self.cells.get(&[c[0] + x, c[1] + y, c[2] + z]) {
                        for &(q, id) in list {
                            let d = p.distance_squared(q);
                            if d < best.0 {
                                best = (d, id);
                            }
                        }
                    }
                }
            }
        }
        best.1
    }
}
fn supported(field: &Field, p: DVec3) -> Option<DVec3> {
    let d = p.normalize();
    let h = p.length() - field.frame.radius;
    let actual = field.deck_height(d, h + 1.5)?;
    // A separate deck above this road must leave room for the covered coach.
    if field
        .deck_height(d, actual + 2.7)
        .is_some_and(|ceiling| ceiling > actual + 0.25 && ceiling < actual + 2.65)
    {
        return None;
    }
    ((actual - h).abs() < 1.0).then_some(d * (field.frame.radius + actual + 0.025))
}
fn quadratic(a: DVec3, b: DVec3, c: DVec3, t: f64) -> DVec3 {
    a * (1.0 - t).powi(2) + b * (2.0 * t * (1.0 - t)) + c * t * t
}
fn cubic(a: DVec3, b: DVec3, c: DVec3, d: DVec3, t: f64) -> DVec3 {
    a * (1.0 - t).powi(3)
        + b * (3.0 * t * (1.0 - t).powi(2))
        + c * (3.0 * t * t * (1.0 - t))
        + d * t.powi(3)
}
struct Draft {
    points: Vec<DVec3>,
    zones: Vec<(Key, DVec3, bool)>,
    stops: Vec<DVec3>,
}
fn lane(graph: &Graph, path: &[usize]) -> Option<Draft> {
    let mut raw = Vec::new();
    let mut zones = Vec::new();
    for (i, &id) in path.iter().enumerate() {
        let n = &graph.nodes[id];
        let a = graph.nodes[path[i.saturating_sub(1)]].position;
        let b = graph.nodes[path[(i + 1).min(path.len() - 1)]].position;
        let incoming = if i == 0 {
            (b - n.position).normalize()
        } else {
            (n.position - a).normalize()
        };
        let outgoing = if i + 1 == path.len() {
            incoming
        } else {
            (b - n.position).normalize()
        };
        if incoming.dot(outgoing) < -0.65 {
            return None;
        }
        let up = n.position.normalize();
        let left_a = up.cross(incoming).normalize();
        let left_b = up.cross(outgoing).normalize();
        let miter = (left_a + left_b).normalize_or_zero();
        let offset = (LANE / miter.dot(left_a).max(0.65)).min(3.1);
        raw.push(n.position + miter * offset);
        if n.links.len() > 2 {
            zones.push((
                key(n.position, 10.0),
                n.position,
                incoming.dot(outgoing) > 0.75,
            ));
        }
    }
    let mut points = vec![raw[0]];
    for i in 1..raw.len() - 1 {
        let a = raw[i] - raw[i - 1];
        let b = raw[i + 1] - raw[i];
        let trim = 5.0_f64.min(a.length() * 0.28).min(b.length() * 0.28);
        let first = raw[i] - a.normalize() * trim;
        let last = raw[i] + b.normalize() * trim;
        points.push(first);
        for step in 1..=4 {
            points.push(quadratic(first, raw[i], last, step as f64 / 4.0));
        }
    }
    points.push(*raw.last().unwrap());
    Some(Draft {
        points,
        zones,
        stops: Vec::new(),
    })
}
fn turn(field: &Field, court: DVec3, start: DVec3, end: DVec3, forward: DVec3) -> Draft {
    let up = court.normalize();
    let right = forward.cross(up).normalize();
    // Approach widens only inside the 40m court: a genuine 6m-radius U turn.
    let centre = court;
    let a = centre - right * 6.0;
    let b = centre + right * 6.0;
    let mut points = Vec::new();
    for i in 0..=8 {
        points.push(cubic(
            start,
            start + forward * 5.0,
            a - forward * 5.0,
            a,
            i as f64 / 8.0,
        ));
    }
    for i in 1..=20 {
        let angle = std::f64::consts::PI * (1.0 - i as f64 / 20.0);
        points.push(centre + right * (angle.cos() * 6.0) + forward * (angle.sin() * 6.0));
    }
    for i in 1..=8 {
        points.push(cubic(
            b,
            b - forward * 5.0,
            end + forward * 5.0,
            end,
            i as f64 / 8.0,
        ));
    }
    let floor = field.frame.local(court).z;
    for p in &mut points {
        let local = field.frame.local(*p);
        *p = field.frame.world(DVec3::new(local.x, local.y, floor));
    }
    Draft {
        points,
        zones: vec![(key(court, 10.0), court, false)],
        stops: vec![centre + forward * 6.0],
    }
}
/// A vehicle at `p` (planet frame) is on a tunnel deck.
pub fn in_tunnel(field: &Field, p: DVec3) -> bool {
    let local = field.frame.local(p);
    field.candidates(local.truncate()).any(|s| {
        s.tunnel
            && s.nearest(local.truncate()).1 < 4.0
            && (s.nearest(local.truncate()).0.z - local.z).abs() < 2.0
    })
}
/// The tunnel lamp group lighting a vehicle at `p`, or None out of doors.
pub fn tunnel_light(field: &Field, lamps: &Lamps, p: DVec3) -> Option<u32> {
    in_tunnel(field, p).then(|| lamps.group(field.frame.local(p)))
}
fn append(route: &mut Route, draft: Draft, field: &Field, lamps: &Lamps) -> Option<()> {
    let first = route.samples.len();
    for pair in draft.points.windows(2) {
        let length = pair[0].distance(pair[1]);
        let count = (length / 1.5).ceil().max(1.0) as usize;
        for i in 0..count {
            let p = supported(field, pair[0].lerp(pair[1], i as f64 / count as f64))?;
            if route
                .samples
                .last()
                .is_some_and(|last| last.pose.position.distance_squared(p) < 0.0001)
            {
                continue;
            }
            let metres = route
                .samples
                .last()
                .map_or(0.0, |last| last.metres + last.pose.position.distance(p));
            let light = tunnel_light(field, lamps, p);
            route.samples.push(Sample {
                metres,
                pose: Pose {
                    position: p,
                    forward: DVec3::ZERO,
                    tunnel: light.is_some(),
                    light: light.unwrap_or(0),
                    limit: 10.0,
                },
            });
            if route.samples.len() > 40_000 {
                return None;
            }
        }
    }
    let p = supported(field, *draft.points.last()?)?;
    let metres = route
        .samples
        .last()
        .map_or(0.0, |last| last.metres + last.pose.position.distance(p));
    if route
        .samples
        .last()
        .is_none_or(|last| last.pose.position.distance_squared(p) > 0.0001)
    {
        route.samples.push(Sample {
            metres,
            pose: Pose {
                position: p,
                forward: DVec3::ZERO,
                tunnel: false,
                light: 0,
                limit: 10.0,
            },
        });
    }
    let nearest = |p: DVec3| {
        route.samples[first..]
            .iter()
            .min_by(|a, b| {
                a.pose
                    .position
                    .distance_squared(p)
                    .total_cmp(&b.pose.position.distance_squared(p))
            })
            .map(|s| s.metres)
    };
    for (id, p, through) in draft.zones {
        if let Some(at) = nearest(p) {
            route.zones.push(Zone {
                id,
                enter: at - 8.0,
                exit: at + 8.0,
                through,
            });
        }
    }
    for p in draft.stops {
        if let Some(at) = nearest(p) {
            route.stops.push(at);
        }
    }
    Some(())
}
fn itinerary(
    graph: &Graph,
    path: &[usize],
    source: &End,
    destination: &End,
    field: &Field,
    lamps: &Lamps,
) -> Option<Route> {
    let mut forward = lane(graph, path)?;
    let mut back = path.to_vec();
    back.reverse();
    let reverse = lane(graph, &back)?;
    // A frontier is an off-screen continuation, never a U-turn on the carriageway.
    if source.court.is_none() {
        let shift = (forward.points[1] - forward.points[0]).normalize() * 4.0;
        forward.points[0] += shift;
    }
    if destination.court.is_none() {
        let n = forward.points.len();
        let shift = (forward.points[n - 1] - forward.points[n - 2]).normalize() * 4.0;
        forward.points[n - 1] -= shift;
    }
    let mut route = Route {
        samples: Vec::new(),
        zones: Vec::new(),
        stops: Vec::new(),
        length: 0.0,
        closed: source.court.is_some() && destination.court.is_some(),
        source: source.name.clone(),
        destination: destination.name.clone(),
        home: source
            .court
            .map(|_| source.name.clone())
            .or_else(|| destination.court.map(|_| destination.name.clone())),
    };
    let last_forward = *forward.points.last()?;
    let first_back = reverse.points[0];
    append(&mut route, forward, field, lamps)?;
    if let Some(court) = destination.court {
        let direction = (graph.nodes[*path.last()?].position
            - graph.nodes[path[path.len() - 2]].position)
            .normalize();
        let mut turning = turn(field, court, last_forward, first_back, direction);
        let turn_first = route.samples.last()?.metres;
        let id = key(court, 10.0);
        turning.zones.clear();
        append(&mut route, turning, field, lamps)?;
        route.zones.push(Zone {
            id,
            enter: turn_first - 3.0,
            exit: route.samples.last()?.metres + 3.0,
            through: false,
        });
        let last_back = *reverse.points.last()?;
        let first_forward = route.samples[0].pose.position;
        append(&mut route, reverse, field, lamps)?;
        if let Some(court) = source.court {
            let direction =
                (graph.nodes[path[0]].position - graph.nodes[path[1]].position).normalize();
            let start = route.samples.last()?.metres;
            let mut turning = turn(field, court, last_back, first_forward, direction);
            turning.zones.clear();
            append(&mut route, turning, field, lamps)?;
            route.zones.push(Zone {
                id: key(court, 10.0),
                enter: start - 3.0,
                exit: route.samples.last()?.metres + 3.0,
                through: false,
            });
        }
    }
    if route.samples.len() < 3 {
        return None;
    }
    route.length = route.samples.last()?.metres;
    if route.length < 50.0 || route.samples.len() > 40_000 {
        return None;
    }
    for i in 0..route.samples.len() {
        let a = route.samples[i.saturating_sub(2)].pose.position;
        let b = route.samples[(i + 2).min(route.samples.len() - 1)]
            .pose
            .position;
        let f = (b - a).normalize_or_zero();
        if f == DVec3::ZERO {
            return None;
        }
        let right = f
            .cross(route.samples[i].pose.position.normalize())
            .normalize();
        // Validate the full body envelope against the deck and actual level.
        for x in [-HALF_WIDTH, HALF_WIDTH] {
            for y in [-1.85, 1.85] {
                let p = route.samples[i].pose.position + right * x + f * y;
                let h = p.length() - field.frame.radius;
                if field
                    .deck_height(p.normalize(), h + 1.5)
                    .is_none_or(|actual| (actual - h).abs() > 1.1)
                {
                    return None;
                }
            }
        }
        route.samples[i].pose.forward = f;
        let middle = route.samples[i].pose.position;
        let u = (middle - a).normalize_or_zero();
        let v = (b - middle).normalize_or_zero();
        let curvature = u.cross(v).length() / ((b - a).length() * 0.5).max(0.5);
        let grade = f.dot(middle.normalize()).abs();
        route.samples[i].pose.limit = (1.6 / curvature.max(0.001))
            .sqrt()
            .min(10.0 / (1.0 + grade * 2.0));
    }
    // Anticipatory braking, including across the loop seam.
    for _ in 0..2 {
        for i in (0..route.samples.len() - 1).rev() {
            let distance = route.samples[i + 1].metres - route.samples[i].metres;
            route.samples[i].pose.limit = route.samples[i]
                .pose
                .limit
                .min((route.samples[i + 1].pose.limit.powi(2) + 2.0 * BRAKE * distance).sqrt());
        }
        if route.closed {
            let cap = route.samples[0].pose.limit;
            route.samples.last_mut()?.pose.limit = cap;
        }
    }
    route.zones.sort_by(|a, b| a.enter.total_cmp(&b.enter));
    Some(route)
}

pub struct Network {
    pub field: Arc<Field>,
    pub routes: Vec<Arc<Route>>,
    index: HashMap<Key, Vec<(usize, usize)>>,
}
impl Network {
    fn build(
        field: Arc<Field>,
        lights: &[[f32; 4]],
        eye: DVec3,
        cancelled: &impl Fn() -> bool,
    ) -> Option<Self> {
        let started = std::time::Instant::now();
        let graph = Graph::build(&field);
        let lamps = Lamps::new(lights);
        let mut ends = graph.ends.clone();
        ends.sort_by(|a, b| {
            graph.nodes[a.node]
                .position
                .distance_squared(eye)
                .total_cmp(&graph.nodes[b.node].position.distance_squared(eye))
        });
        // Mix village traffic with through journeys, so a long sea tunnel is
        // not starved by a dozen nearer village-to-village pairs.
        let courts: Vec<_> = ends.iter().filter(|e| e.court.is_some()).cloned().collect();
        let frontiers: Vec<_> = ends.iter().filter(|e| e.court.is_none()).cloned().collect();
        ends.clear();
        for i in 0..courts.len().max(frontiers.len()) {
            if let Some(e) = courts.get(i) {
                ends.push(e.clone());
            }
            if let Some(e) = frontiers.get(i) {
                ends.push(e.clone());
            }
        }
        ends.truncate(96);
        let mut routes = Vec::new();
        let mut samples = 0;
        for round in 1..=4 {
            for a in 0..ends.len().min(16) {
                if cancelled() {
                    return None;
                }
                let b = (a + round) % ends.len();
                if a == b {
                    continue;
                }
                let Some(path) = graph.shortest(ends[a].node, ends[b].node, cancelled) else {
                    continue;
                };
                if path.len() < 3 {
                    continue;
                }
                if let Some(route) = itinerary(&graph, &path, &ends[a], &ends[b], &field, &lamps) {
                    if samples + route.samples.len() > 160_000 {
                        continue;
                    }
                    samples += route.samples.len();
                    routes.push(Arc::new(route));
                }
                if routes.len() >= MAX_ROUTES {
                    break;
                }
            }
            if routes.len() >= MAX_ROUTES {
                break;
            }
        }
        let mut index = HashMap::<Key, Vec<_>>::new();
        for (r, route) in routes.iter().enumerate() {
            for (i, p) in route.samples.iter().enumerate() {
                index
                    .entry(
                        field
                            .frame
                            .local(p.pose.position)
                            .to_array()
                            .map(|x| (x / 24.0).floor() as i64),
                    )
                    .or_default()
                    .push((r, i));
            }
        }
        tracing::info!(target:"planet::traffic",nodes=graph.nodes.len(),destinations=ends.len(),routes=routes.len(),samples,build_ms=started.elapsed().as_secs_f64()*1000.0,"traffic routes ready");
        Some(Self {
            field,
            routes,
            index,
        })
    }
    fn remap(&self, pose: Pose) -> Option<(usize, f64)> {
        let c = self
            .field
            .frame
            .local(pose.position)
            .to_array()
            .map(|x| (x / 24.0).floor() as i64);
        let mut best = (3.0 * 3.0, 0, 0.0);
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    if let Some(list) = self.index.get(&[c[0] + x, c[1] + y, c[2] + z]) {
                        for &(r, i) in list {
                            let route = &self.routes[r];
                            if i + 1 >= route.samples.len() {
                                continue;
                            }
                            let a = &route.samples[i];
                            let b = &route.samples[i + 1];
                            if a.pose.forward.dot(pose.forward) < 0.75 {
                                continue;
                            }
                            let d = b.pose.position - a.pose.position;
                            let t = ((pose.position - a.pose.position).dot(d) / d.length_squared())
                                .clamp(0.0, 1.0);
                            let error = pose.position.distance_squared(a.pose.position + d * t);
                            if error < best.0 {
                                best = (error, r, a.metres + (b.metres - a.metres) * t);
                            }
                        }
                    }
                }
            }
        }
        (best.0 < 9.0).then_some((best.1, best.2))
    }
}

pub struct Agent {
    pub id: u64,
    pub kind: u32,
    route: Arc<Route>,
    distance: f64,
    previous: f64,
    pub speed: f64,
    wait: f64,
    dwell: f64,
    served: i64,
    held: Option<(Key, f64)>,
    pub fade: f64,
    pub travelled: f64,
    pub stranded: bool,
    /// Who drives: a resident of the route's home village (people::Person key).
    pub driver: u64,
    dwell_total: f64,
}
impl Agent {
    /// Seconds into and total length of the current village stop, if stopped.
    pub fn stop(&self, alpha: f64) -> Option<(f64, f64)> {
        (self.dwell > 0.0 && self.dwell_total > 0.0).then(|| {
            let elapsed = self.dwell_total - self.dwell + alpha * STEP;
            (elapsed.min(self.dwell_total), self.dwell_total)
        })
    }
    /// Whether the driver is a village resident (else a passing traveller).
    pub fn home(&self) -> Option<&str> {
        self.route.home.as_deref()
    }
    pub fn pose(&self, alpha: f64) -> Pose {
        self.route
            .at(self.previous + (self.distance - self.previous) * alpha)
    }
    pub fn future_pose(&self, metres: f64) -> Pose {
        self.route.at(self.distance + metres)
    }
    pub fn wheel_angle(&self) -> f64 {
        (self.travelled / 0.52).rem_euclid(std::f64::consts::TAU)
    }
}
pub struct Simulation {
    network: Arc<Network>,
    pub agents: Vec<Agent>,
    clock: f64,
    accumulator: f64,
    next_id: u64,
    pub trips: u64,
    pub waits: u64,
    initial: bool,
    target_population: usize,
}
fn ahead(p: Pose, q: DVec3) -> Option<f64> {
    let d = q - p.position;
    let up = p.position.normalize();
    let along = d.dot(p.forward);
    let side = d.dot(p.forward.cross(up).normalize());
    (along > 0.0 && side.abs() < 1.9 && d.dot(up).abs() < 3.0).then_some(along)
}
impl Simulation {
    fn new(network: Arc<Network>) -> Self {
        Self {
            network,
            agents: Vec::new(),
            clock: 0.0,
            accumulator: 0.0,
            next_id: 1,
            trips: 0,
            waits: 0,
            initial: true,
            target_population: population(),
        }
    }
    fn adopt(&mut self, network: Arc<Network>, eye: DVec3) {
        self.agents.retain_mut(|a| {
            let pose = a.pose(1.0);
            if let Some((r, s)) = network.remap(pose) {
                a.route = network.routes[r].clone();
                a.distance = s;
                a.previous = s;
                a.held = None;
                a.stranded = false;
                a.served = -1;
                true
            } else if pose.position.distance(eye) < 120.0 {
                a.speed = 0.0;
                a.stranded = true;
                true
            } else {
                false
            }
        });
        self.network = network;
    }
    pub fn alpha(&self) -> f64 {
        self.accumulator / STEP
    }
    fn spawn(&mut self, eye: DVec3, forward: DVec3) {
        if self.network.routes.is_empty() {
            return;
        }
        self.agents
            .retain(|a| a.pose(1.0).position.distance_squared(eye) < 3500.0 * 3500.0);
        for _ in 0..128 {
            if self.agents.len() >= self.target_population {
                break;
            }
            let seed = self.next_id;
            self.next_id += 1;
            let r = (hash(seed) % self.network.routes.len() as u64) as usize;
            let route = &self.network.routes[r];
            let s = unit(seed ^ 7919) * route.length;
            let pose = route.at(s);
            let relative = pose.position - eye;
            let distance = relative.length();
            if !(35.0..2200.0).contains(&distance)
                || (!self.initial && distance < 1000.0 && relative.dot(forward) > -10.0)
            {
                continue;
            }
            if route
                .next_zone(s)
                .is_some_and(|(_, enter, exit)| s > enter - 12.0 && s < exit + 12.0)
                || route.stops.iter().any(|&stop| (stop - s).abs() < 10.0)
            {
                continue;
            }
            if self
                .agents
                .iter()
                .any(|a| a.pose(1.0).position.distance_squared(pose.position) < 10.0 * 10.0)
            {
                continue;
            }
            // A resident of the home village not already driving another cart.
            let driver = match route.home.as_deref() {
                Some(home) => (0..crate::people::RESIDENTS)
                    .map(|k| {
                        crate::people::resident_key(
                            home,
                            ((hash(seed ^ 0xd21e) + k as u64) % crate::people::RESIDENTS as u64) as u32,
                        )
                    })
                    .find(|key| !self.agents.iter().any(|a| a.driver == *key))
                    .unwrap_or(hash(seed ^ 0x7ae1)),
                None => hash(seed ^ 0x7ae1),
            };
            let route = route.clone();
            self.agents.push(Agent {
                id: seed,
                kind: (seed % 2) as u32,
                route,
                distance: s,
                previous: s,
                speed: 0.0,
                wait: 0.0,
                dwell: 0.0,
                served: -1,
                held: None,
                fade: 0.0,
                travelled: 0.0,
                stranded: false,
                driver,
                dwell_total: 0.0,
            });
        }
        if self.agents.len() >= self.target_population.min(4) {
            self.initial = false;
        }
    }
    pub fn advance(&mut self, dt: f64, eye: DVec3, forward: DVec3, player: Option<DVec3>) {
        if self.clock == 0.0 && self.agents.is_empty() {
            self.spawn(eye, forward);
        }
        self.accumulator += (if dt.is_finite() { dt } else { 0.0 }).clamp(0.0, MAX_ADVANCE_SECONDS);
        while self.accumulator >= STEP {
            self.accumulator -= STEP;
            self.spawn(eye, forward);
            self.tick(player, eye);
        }
    }
    fn tick(&mut self, player: Option<DVec3>, eye: DVec3) {
        self.clock += STEP;
        for a in &mut self.agents {
            a.previous = a.distance;
            a.fade = (a.fade + STEP * 2.0).min(1.0);
            if a.held.is_some_and(|(_, end)| a.distance > end) {
                a.held = None;
            }
        }
        let poses: Vec<_> = self.agents.iter().map(|a| a.pose(1.0)).collect();
        let speeds: Vec<_> = self.agents.iter().map(|a| a.speed).collect();
        let mut owners: HashMap<Key, u64> = self
            .agents
            .iter()
            .filter_map(|a| a.held.map(|(id, _)| (id, a.id)))
            .collect();
        let mut requests = BTreeMap::<Key, Vec<(usize, f64, f64, usize, f64)>>::new();
        for (i, a) in self.agents.iter_mut().enumerate() {
            if a.held.is_some() || a.stranded {
                continue;
            }
            if let Some((z, enter, exit)) = a.route.next_zone(a.distance) {
                if enter - a.distance < 12.0 + a.speed * a.speed / (2.0 * BRAKE) {
                    a.wait += STEP;
                    let zone = &a.route.zones[z];
                    let exit_pose = a.route.at(exit + 3.0);
                    let clear = !poses.iter().enumerate().any(|(j, p)| {
                        j != i
                            && (p.position - exit_pose.position)
                                .dot(exit_pose.position.normalize())
                                .abs()
                                < 2.0
                            && (p.position - exit_pose.position)
                                .dot(exit_pose.forward)
                                .abs()
                                < 6.0
                            && (p.position - exit_pose.position)
                                .dot(exit_pose.forward.cross(exit_pose.position.normalize()))
                                .abs()
                                < 1.8
                            && speeds[j] < 1.0
                    });
                    let clear = clear
                        && player.is_none_or(|p| {
                            let delta = p - exit_pose.position;
                            delta.dot(exit_pose.position.normalize()).abs() > 3.0
                                || delta.dot(exit_pose.forward).abs() > 7.0
                                || delta
                                    .dot(exit_pose.forward.cross(exit_pose.position.normalize()))
                                    .abs()
                                    > 2.0
                        });
                    if clear {
                        requests.entry(zone.id).or_default().push((
                            i,
                            (a.wait / 8.0).floor(),
                            if zone.through { 0.0 } else { 1.0 },
                            z,
                            exit,
                        ));
                    }
                } else {
                    a.wait = 0.0;
                }
            }
        }
        for (id, mut list) in requests {
            if owners.contains_key(&id) {
                continue;
            }
            list.sort_by(|a, b| {
                b.1.total_cmp(&a.1)
                    .then(a.2.total_cmp(&b.2))
                    .then(self.agents[a.0].id.cmp(&self.agents[b.0].id))
            });
            if let Some(&(i, _, _, _, exit)) = list.first() {
                owners.insert(id, self.agents[i].id);
                self.agents[i].held = Some((id, exit));
                self.agents[i].wait = 0.0;
            }
        }
        for i in 0..self.agents.len() {
            let a = &mut self.agents[i];
            if a.stranded {
                a.speed = 0.0;
                continue;
            }
            if a.dwell > 0.0 {
                a.dwell = (a.dwell - STEP).max(0.0);
                a.speed = 0.0;
                continue;
            }
            let mut target = poses[i].limit.min(if a.kind == 0 { 6.0 } else { 8.0 });
            let mut available = f64::INFINITY;
            for (j, p) in poses.iter().enumerate() {
                if i != j {
                    if let Some(gap) = ahead(poses[i], p.position) {
                        available = available.min((gap - 4.4).max(0.0));
                    }
                }
            }
            if let Some(player) = player {
                if let Some(gap) = ahead(poses[i], player) {
                    available = available.min((gap - 4.2).max(0.0));
                }
            }
            if let Some((z, enter, _)) = a.route.next_zone(a.distance) {
                let zone = &a.route.zones[z];
                if owners.get(&zone.id).is_none_or(|&id| id != a.id) {
                    available = available.min((enter - a.distance).max(0.0));
                    if enter - a.distance < 3.0 {
                        self.waits += 1;
                    }
                }
            }
            let stop = a.route.next_stop(a.distance, a.served);
            if let Some((_, s)) = stop {
                available = available.min((s - a.distance).max(0.0));
            }
            if !a.route.closed {
                available = available.min((a.route.length - a.distance).max(0.0));
            }
            target = target.min((2.0 * BRAKE * available).sqrt());
            a.speed += (target - a.speed).clamp(-BRAKE * STEP, 1.2 * STEP);
            let travel = (a.speed * STEP).min(available);
            if travel + 1e-8 < a.speed * STEP {
                a.speed = travel / STEP;
            }
            a.distance += travel;
            a.travelled += travel;
            if let Some((id, s)) = stop {
                if s - a.distance < 0.03 && a.speed < 0.5 {
                    a.distance = s;
                    a.speed = 0.0;
                    a.dwell = 8.0 + unit(a.id ^ id as u64) * 12.0;
                    a.dwell_total = a.dwell;
                    a.served = id;
                    self.trips += 1;
                    tracing::debug!(target:"planet::traffic",id=a.id,source=%a.route.source,destination=%a.route.destination,"traffic arrived at village court");
                }
            }
            if !a.route.closed && a.distance >= a.route.length - 0.03 {
                a.speed = 0.0;
            }
        }
        // Recycle completed journeys only beyond the visible neighbourhood.
        self.agents.retain(|a| {
            a.route.closed
                || a.distance < a.route.length - 0.03
                || a.pose(1.0).position.distance(eye) < 1200.0
        });
    }
    pub fn blocks_motion(&self, from: DVec3, to: DVec3, heading: DVec3) -> bool {
        self.agents.iter().any(|a| {
            let p = a.pose(self.alpha());
            let up = p.position.normalize();
            if (from - p.position).dot(up).abs() > 3.5 {
                return false;
            }
            let right = p.forward.cross(up).normalize();
            // Project the player's oriented rectangle onto the NPC's axes.
            let player_right = heading.cross(up).normalize_or_zero();
            let half_x =
                0.92 + 1.8 * heading.dot(right).abs() + 0.95 * player_right.dot(right).abs();
            let half_y = 1.85
                + 1.8 * heading.dot(p.forward).abs()
                + 0.95 * player_right.dot(p.forward).abs();
            let start = from - p.position;
            let delta = to - from;
            let mut low = 0.0_f64;
            let mut high = 1.0_f64;
            for (axis, half) in [(right, half_x), (p.forward, half_y)] {
                let x = start.dot(axis);
                let d = delta.dot(axis);
                if d.abs() < 1e-10 {
                    if x.abs() > half {
                        return false;
                    }
                } else {
                    let a = (-half - x) / d;
                    let b = (half - x) / d;
                    low = low.max(a.min(b));
                    high = high.min(a.max(b));
                    if low > high {
                        return false;
                    }
                }
            }
            // Permit moving out of an overlap, never farther into it.
            let old = start.dot(right).powi(2) / half_x.powi(2)
                + start.dot(p.forward).powi(2) / half_y.powi(2);
            let new = (to - p.position).dot(right).powi(2) / half_x.powi(2)
                + (to - p.position).dot(p.forward).powi(2) / half_y.powi(2);
            low <= high && !(old < 1.0 && new > old)
        })
    }
}

struct Request {
    field: Arc<Field>,
    lights: Vec<[f32; 4]>,
    eye: DVec3,
    generation: u64,
}
#[derive(Default)]
struct Pending {
    request: Option<Request>,
    result: Option<(u64, Arc<Network>)>,
    stop: bool,
}
struct Worker {
    shared: Arc<(Mutex<Pending>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
    generation: u64,
}
impl Worker {
    fn new() -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let state = shared.clone();
        let body = crate::body::active();
        let thread = std::thread::Builder::new()
            .name("road-traffic".into())
            .spawn(move || {
                crate::body::with_body(body, || loop {
                    let request = {
                        let mut pending = state.0.lock().unwrap();
                        while pending.request.is_none() && !pending.stop {
                            pending = state.1.wait(pending).unwrap();
                        }
                        if pending.stop {
                            break;
                        }
                        pending.request.take().unwrap()
                    };
                    let result =
                        Network::build(request.field, &request.lights, request.eye, &|| {
                            let p = state.0.lock().unwrap();
                            p.stop
                                || p.request
                                    .as_ref()
                                    .is_some_and(|r| r.generation > request.generation)
                        });
                    if let Some(network) = result {
                        state.0.lock().unwrap().result =
                            Some((request.generation, Arc::new(network)));
                    }
                })
            })
            .expect("traffic route worker");
        Self {
            shared,
            thread: Some(thread),
            generation: 0,
        }
    }
    fn request(&mut self, field: Arc<Field>, lights: &[[f32; 4]], eye: DVec3) {
        self.generation += 1;
        self.shared.0.lock().unwrap().request = Some(Request {
            field,
            lights: lights.to_vec(),
            eye,
            generation: self.generation,
        });
        self.shared.1.notify_one();
    }
    fn take(&self) -> Option<Arc<Network>> {
        self.shared
            .0
            .lock()
            .unwrap()
            .result
            .take()
            .and_then(|(g, n)| (g == self.generation).then_some(n))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.0.lock().unwrap().stop = true;
        self.shared.1.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub struct Controller {
    worker: Option<Worker>,
    requested: Option<Arc<Field>>,
    pub simulation: Option<Simulation>,
    log_time: std::time::Instant,
    request_eye: DVec3,
    /// The cart an inspection camera is following, once chosen.
    followed: std::cell::Cell<Option<u64>>,
}
impl Controller {
    pub fn new(active: bool) -> Self {
        Self {
            worker: active.then(Worker::new),
            requested: None,
            simulation: None,
            log_time: std::time::Instant::now(),
            request_eye: DVec3::ZERO,
            followed: Default::default(),
        }
    }
    /// A close camera on a cart's driver: alongside ahead of a moving cart,
    /// or (`stop`) the cart nearest a village stop, from its back corner,
    /// where the driver climbs down and comes round to the engine jar.
    pub fn driver_pose(&self, stop: bool) -> Option<(DVec3, DVec3)> {
        let sim = self.simulation.as_ref()?;
        let chosen = self.followed.get().and_then(|id| sim.agents.iter().find(|a| a.id == id));
        let a = match chosen {
            Some(a) => a,
            None => {
                let a = if stop {
                    sim.agents
                        .iter()
                        .filter(|a| !a.stranded)
                        .min_by(|x, y| {
                            let next = |a: &Agent| {
                                if a.dwell > 0.0 {
                                    return -1.0;
                                }
                                a.route.next_stop(a.distance, a.served).map_or(f64::INFINITY, |(_, s)| s - a.distance)
                            };
                            next(x).total_cmp(&next(y))
                        })?
                } else {
                    sim.agents.iter().find(|a| !a.stranded && a.route.closed)?
                };
                self.followed.set(Some(a.id));
                a
            }
        };
        let p = a.pose(sim.alpha());
        let up = p.position.normalize();
        let right = p.forward.cross(up).normalize();
        Some(if stop {
            (
                p.position - p.forward * 5.5 - right * 4.6 + up * 2.1,
                p.position - p.forward * 0.9 - right * 0.7 + up * 1.0,
            )
        } else {
            (
                p.position + p.forward * 6.5 - right * 3.4 + up * 2.3,
                p.position + p.forward * 0.9 + up * 1.65,
            )
        })
    }
    pub fn update(
        &mut self,
        field: &Arc<Field>,
        lights: &[[f32; 4]],
        dt: f64,
        eye: DVec3,
        forward: DVec3,
        player: Option<DVec3>,
    ) {
        let Some(worker) = &mut self.worker else {
            return;
        };
        if self
            .requested
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, field))
            || self.request_eye.distance_squared(eye) > 1000.0 * 1000.0
        {
            worker.request(field.clone(), lights, eye);
            self.requested = Some(field.clone());
            self.request_eye = eye;
        }
        if let Some(network) = worker.take() {
            if Arc::ptr_eq(&network.field, field) {
                if let Some(sim) = &mut self.simulation {
                    sim.adopt(network, eye);
                } else {
                    self.simulation = Some(Simulation::new(network));
                }
            }
        }
        if let Some(sim) = &mut self.simulation {
            sim.advance(dt, eye, forward, player);
            // Fast-forward the traffic, not its diagnostic I/O cadence.
            if self.log_time.elapsed() >= std::time::Duration::from_secs(2) {
                self.log_time = std::time::Instant::now();
                tracing::info!(target:"planet::traffic",agents=sim.agents.len(),moving=sim.agents.iter().filter(|a|a.speed>0.1).count(),trips=sim.trips,junction_wait_ticks=sim.waits,travelled=sim.agents.iter().map(|a|a.travelled).sum::<f64>(),"traffic simulation");
            }
        }
    }
    pub fn matches_field(&self, field: &Arc<Field>) -> bool {
        self.simulation
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(&s.network.field, field))
    }
    pub fn inspection_pose(&self, tunnel: bool) -> Option<(DVec3, DVec3)> {
        let sim = self.simulation.as_ref()?;
        let a = if tunnel {
            sim.agents.iter().find(|a| a.pose(1.0).tunnel)?
        } else {
            sim.agents.first()?
        };
        let p = a.pose(1.0);
        let behind = a.route.at((a.distance - 12.0).max(0.0));
        let up = behind.position.normalize();
        let right = behind.forward.cross(up).normalize();
        let side = if p.tunnel { 0.4 } else { 4.1 };
        let candidate = behind.position + right * side;
        let h = candidate.length() - sim.network.field.frame.radius;
        let base = if sim
            .network
            .field
            .deck_height(candidate.normalize(), h + 1.0)
            .is_some()
        {
            candidate
        } else {
            behind.position
        };
        Some((base + up * 2.6, p.position + p.position.normalize() * 1.1))
    }
    pub fn stats(&self) -> (usize, f64) {
        self.simulation.as_ref().map_or((0, 0.0), |s| {
            (s.agents.len(), s.agents.iter().map(|a| a.travelled).sum())
        })
    }
    pub fn ready(&self) -> bool {
        self.worker.is_none() || self.simulation.is_some()
    }
    pub fn blocks_motion(&self, from: DVec3, to: DVec3, heading: DVec3) -> bool {
        self.simulation
            .as_ref()
            .is_some_and(|s| s.blocks_motion(from, to, heading))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_network_field::{Frame, Segment};
    fn flat() -> Arc<Field> {
        let frame = Frame::new(DVec3::X, 4_000_000.0).unwrap();
        let mut segments = Vec::new();
        for i in -30..30 {
            segments.push(Segment {
                a: DVec3::new(0.0, i as f64 * 20.0, 200.0),
                b: DVec3::new(0.0, (i + 1) as f64 * 20.0, 200.0),
                tunnel: false,
                reach: 20.0,
                floor_depth: 0.0,
                junction_a: false,
                junction_b: false,
            });
        }
        for y in [-615.0, 615.0] {
            segments.push(crate::road_parking::segment(DVec3::new(0.0, y, 200.0)));
        }
        let mut field = Field::build(frame, segments).unwrap();
        for y in [-615.0, 615.0] {
            field.village_parking.push((
                frame.world(DVec3::new(0.0, y * 1.2, 200.0)).normalize(),
                frame.world(DVec3::new(0.0, y, 200.0)),
            ));
        }
        Arc::new(field)
    }
    fn route_network() -> Arc<Network> {
        let f = flat();
        let eye = f.frame.world(DVec3::new(0.0, 0.0, 203.0));
        let n = Network::build(f, &[], eye, &|| false).unwrap();
        assert!(
            !n.routes.is_empty(),
            "real court manoeuvres must fit the road and parking deck"
        );
        Arc::new(n)
    }
    fn agent(route: Arc<Route>, id: u64, distance: f64, speed: f64) -> Agent {
        Agent {
            id,
            kind: 1,
            route,
            distance,
            previous: distance,
            speed,
            wait: 0.0,
            dwell: 0.0,
            served: -1,
            held: None,
            fade: 1.0,
            travelled: 0.0,
            stranded: false,
            driver: id,
            dwell_total: 0.0,
        }
    }
    #[test]
    #[ignore = "asset-backed resident road routing and traffic endurance"]
    fn real_resident_roads_support_traffic_without_deck_departures() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/outmaps/test-planet");
        let eye = crate::road_network_render::initial_position(false);
        let (_worker, snapshot) = crate::road_network_stream::Worker::start(root, eye).unwrap();
        let network = Arc::new(
            Network::build(
                snapshot.field.clone(),
                &snapshot.lighting,
                snapshot.build_centre,
                &|| false,
            )
            .unwrap(),
        );
        assert!(!network.routes.is_empty(), "no supported real routes");
        let mut sim = Simulation::new(network);
        let eye = snapshot.build_centre;
        for _ in 0..1800 {
            sim.advance(STEP, eye, DVec3::Y, None);
            for (i, a) in sim.agents.iter().enumerate() {
                let p = a.pose(1.0);
                for b in sim.agents.iter().skip(i + 1) {
                    let q = b.pose(1.0);
                    let delta = q.position - p.position;
                    if delta.length_squared() > 64.0
                        || delta.dot(p.position.normalize()).abs() > 3.0
                    {
                        continue;
                    }
                    let right_p = p.forward.cross(p.position.normalize()).normalize();
                    let right_q = q.forward.cross(q.position.normalize()).normalize();
                    let separate =
                        [p.forward, right_p, q.forward, right_q]
                            .into_iter()
                            .any(|axis| {
                                let extent = 1.85 * p.forward.dot(axis).abs()
                                    + 1.15 * right_p.dot(axis).abs()
                                    + 1.85 * q.forward.dot(axis).abs()
                                    + 1.15 * right_q.dot(axis).abs();
                                delta.dot(axis).abs() >= extent
                            });
                    assert!(separate, "NPC bodies overlap: {} and {}", a.id, b.id);
                }
            }
        }
        assert!(sim.agents.len() >= 4, "no local NPC population");
        assert!(sim.agents.iter().map(|a| a.travelled).sum::<f64>() > 100.0);
        for a in &sim.agents {
            let p = a.pose(1.0);
            let h = p.position.length() - snapshot.field.frame.radius;
            assert!(
                snapshot
                    .field
                    .deck_height(p.position.normalize(), h + 1.5)
                    .is_some_and(|deck| (deck - h).abs() < 0.2),
                "NPC left real deck"
            );
        }
        println!(
            "{} real routes, {} NPCs, {:.1}m travelled",
            sim.network.routes.len(),
            sim.agents.len(),
            sim.agents.iter().map(|a| a.travelled).sum::<f64>()
        );
    }

    #[test]
    fn a_star_uses_shortest_engineered_distance_and_does_not_connect_overpasses() {
        let mut g = Graph {
            nodes: (0..4)
                .map(|i| Node {
                    position: DVec3::new(i as f64, 0.0, 0.0),
                    links: Vec::new(),
                })
                .collect(),
            ends: Vec::new(),
        };
        for (a, b, d) in [(0, 1, 5.0), (1, 3, 5.0), (0, 2, 2.0), (2, 3, 2.0)] {
            g.nodes[a].links.push((b, d, false));
            g.nodes[b].links.push((a, d, false));
        }
        assert_eq!(g.shortest(0, 3, &|| false).unwrap(), [0, 2, 3]);
        assert!(g.shortest(0, 3, &|| true).is_none());
        let mut f = flat();
        let f = Arc::get_mut(&mut f).unwrap();
        // Two crossing decks have no shared 3D endpoint, hence no junction.
        f.segments = vec![
            Segment {
                a: DVec3::new(-30.0, 0.0, 100.0),
                b: DVec3::new(30.0, 0.0, 100.0),
                tunnel: false,
                reach: 20.0,
                floor_depth: 0.0,
                junction_a: false,
                junction_b: false,
            },
            Segment {
                a: DVec3::new(0.0, -30.0, 110.0),
                b: DVec3::new(0.0, 30.0, 110.0),
                tunnel: false,
                reach: 20.0,
                floor_depth: 0.0,
                junction_a: false,
                junction_b: false,
            },
        ];
        f.village_parking.clear();
        let graph = Graph::build(f);
        assert_eq!(graph.nodes.len(), 4);
        assert!(graph.shortest(0, 3, &|| false).is_none());
    }
    #[test]
    fn left_hand_lanes_and_six_metre_parking_turns_are_supported_and_continuous() {
        let network = route_network();
        let route = &network.routes[0];
        assert!(route.closed);
        assert_eq!(route.stops.len(), 2);
        let f = network.field.frame;
        let forward = route
            .samples
            .iter()
            .find(|s| f.local(s.pose.position).y.abs() < 2.0 && s.pose.forward.dot(f.forward) > 0.9)
            .unwrap();
        let reverse = route
            .samples
            .iter()
            .find(|s| {
                f.local(s.pose.position).y.abs() < 2.0 && s.pose.forward.dot(f.forward) < -0.9
            })
            .unwrap();
        assert!((f.local(forward.pose.position).x + LANE).abs() < 0.03);
        assert!((f.local(reverse.pose.position).x - LANE).abs() < 0.03);
        assert!(
            route
                .at(0.0)
                .position
                .distance(route.at(route.length - 0.0001).position)
                < 0.01
        );
        assert!(
            route
                .at(0.0)
                .forward
                .dot(route.at(route.length - 0.0001).forward)
                > 0.98
        );
        for s in &route.samples {
            assert!(s.pose.position.is_finite());
            assert!(s.pose.limit > 0.0);
        }
    }
    #[test]
    fn queued_carts_brake_without_overlapping_and_yield_to_the_player() {
        let network = route_network();
        let route = network.routes[0].clone();
        let eye = route.at(50.0).position;
        let mut sim = Simulation::new(network);
        sim.agents = vec![
            agent(route.clone(), 1, 30.0, 7.0),
            agent(route.clone(), 2, 44.0, 0.0),
        ];
        for _ in 0..300 {
            sim.tick(None, eye);
            let a = sim.agents[0].pose(1.0);
            let b = sim.agents[1].pose(1.0);
            assert!(a.position.distance(b.position) > 4.3);
        }
        sim.agents = vec![agent(route.clone(), 1, 30.0, 5.0)];
        let player = route.at(55.0).position + eye.normalize() * 1.4;
        for _ in 0..300 {
            sim.tick(Some(player), eye);
        }
        assert!(sim.agents[0].distance < 51.0);
        assert!(sim.agents[0].speed < 0.1);
    }
    #[test]
    fn arrivals_park_then_resume_and_render_interpolation_stays_monotonic() {
        let network = route_network();
        let route = network.routes[0].clone();
        let stop = route.stops[0];
        let eye = route.at(stop).position;
        let mut sim = Simulation::new(network);
        sim.agents = vec![agent(route, 1, stop - 1.0, 1.0)];
        let mut old = sim.agents[0].distance;
        let mut parked = false;
        for _ in 0..1200 {
            sim.tick(None, eye);
            let a = &sim.agents[0];
            assert!(a.distance >= old);
            old = a.distance;
            parked |= a.dwell > 0.0;
            let mid = a.pose(0.5);
            assert!(mid.position.is_finite());
        }
        assert!(parked);
        assert!(sim.trips > 0);
        assert!(sim.agents[0].distance > stop + 5.0);
    }
    fn crossing_route(frame: crate::road_network_field::Frame, east: bool) -> Arc<Route> {
        let direction = if east { frame.across } else { frame.forward };
        let samples = [0.0, 125.0, 250.0]
            .into_iter()
            .map(|metres| Sample {
                metres,
                pose: Pose {
                    position: frame.world(DVec3::new(0.0, 0.0, 200.0))
                        + direction * (metres - 125.0),
                    forward: direction,
                    tunnel: false,
                    light: 0,
                    limit: 8.0,
                },
            })
            .collect();
        Arc::new(Route {
            samples,
            zones: vec![Zone {
                id: [91, 12, 3],
                enter: 113.0,
                exit: 137.0,
                through: east,
            }],
            stops: Vec::new(),
            length: 250.0,
            closed: false,
            source: "START".into(),
            destination: "DESTINATION".into(),
            home: None,
        })
    }
    #[test]
    fn crossing_traffic_reserves_the_box_and_both_approaches_make_progress() {
        let network = route_network();
        let eye = network.field.frame.world(DVec3::new(0.0, 0.0, 205.0));
        let a = crossing_route(network.field.frame, true);
        let b = crossing_route(network.field.frame, false);
        let mut sim = Simulation::new(network);
        sim.agents = vec![agent(a, 1, 80.0, 5.0), agent(b, 2, 80.0, 5.0)];
        for _ in 0..1000 {
            sim.tick(None, eye);
            assert!(sim.agents.iter().filter(|a| a.held.is_some()).count() <= 1);
            assert!(
                sim.agents[0]
                    .pose(1.0)
                    .position
                    .distance(sim.agents[1].pose(1.0).position)
                    > 4.0
            );
        }
        assert!(
            sim.agents.iter().all(|a| a.distance > 150.0),
            "junction starvation"
        );
    }
    #[test]
    fn a_blocked_exit_keeps_the_cart_out_of_the_junction() {
        let network = route_network();
        let route = crossing_route(network.field.frame, true);
        let eye = route.at(125.0).position;
        let player = route.at(140.0).position + eye.normalize() * 1.4;
        let mut sim = Simulation::new(network);
        sim.agents = vec![agent(route, 1, 105.0, 3.0)];
        for _ in 0..180 {
            sim.tick(Some(player), eye);
            assert!(sim.agents[0].held.is_none());
        }
        assert!(sim.agents[0].distance <= 113.001);
    }
    #[test]
    fn traffic_scene_time_scales_motion_and_dwell_without_a_small_frame_delta_cap() {
        let network = route_network();
        let route = network.routes[0].clone();
        let eye = route.at(50.0).position;
        for scale in [0.1, 0.25, 0.5, 1.0, 2.0, 10.0, 40.0] {
            let mut scaled = Simulation::new(network.clone());
            let mut reference = Simulation::new(network.clone());
            for sim in [&mut scaled, &mut reference] {
                sim.target_population = 0;
                sim.agents = vec![agent(route.clone(), 1, 30.0, 0.0)];
            }
            // Eight real seconds at 15 FPS, including slow-rendering 40x play.
            for _ in 0..120 {
                scaled.advance(scale / 15.0, eye, DVec3::Y, None);
            }
            for _ in 0..(8.0 * scale / STEP).round() as usize {
                reference.advance(STEP, eye, DVec3::Y, None);
            }
            assert!(
                (scaled.clock - reference.clock).abs() < STEP * 1.01,
                "{scale}x clock"
            );
            assert!(
                scaled.agents[0]
                    .pose(1.0)
                    .position
                    .distance(reference.agents[0].pose(1.0).position)
                    < 0.3,
                "{scale}x position"
            );
            assert_eq!(scaled.trips, reference.trips, "{scale}x arrivals");
            assert!(
                (scaled.agents[0].dwell - reference.agents[0].dwell).abs() < STEP * 1.01,
                "{scale}x dwell"
            );
        }
    }

    #[test]
    fn traffic_pause_does_not_advance_or_retime_the_visible_vehicle() {
        let network = route_network();
        let route = network.routes[0].clone();
        let eye = route.at(50.0).position;
        let mut sim = Simulation::new(network);
        sim.target_population = 0;
        sim.agents = vec![agent(route, 1, 30.0, 6.0)];
        sim.advance(STEP * 1.5, eye, DVec3::Y, None);
        let before = sim.agents[0].pose(sim.alpha()).position;
        let clock = sim.clock;
        for _ in 0..120 {
            sim.advance(0.0, eye, DVec3::Y, None);
        }
        assert_eq!(sim.clock, clock);
        assert_eq!(sim.agents[0].pose(sim.alpha()).position, before);
        sim.advance(STEP * 0.5, eye, DVec3::Y, None);
        assert!(sim.clock > clock);
    }

    #[test]
    fn extreme_time_speed_is_work_limited_without_leaving_a_catch_up_debt() {
        let network = route_network();
        let route = network.routes[0].clone();
        let eye = route.at(50.0).position;
        let mut sim = Simulation::new(network);
        sim.target_population = 0;
        sim.agents = vec![agent(route, 1, 30.0, 6.0)];
        sim.advance(163_840.0, eye, DVec3::Y, None);
        assert!(sim.clock > MAX_ADVANCE_SECONDS - STEP * 1.01);
        assert!(sim.clock <= MAX_ADVANCE_SECONDS + 1e-9);
        assert!(sim.alpha() < 1.0);
        let clock = sim.clock;
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            sim.advance(dt, eye, DVec3::Y, None);
            assert_eq!(sim.clock, clock);
        }
        sim.advance(STEP, eye, DVec3::Y, None);
        assert!((sim.clock - clock - STEP).abs() < 1e-9);
    }

    #[test]
    fn traffic_is_wired_to_scene_time_not_the_unscaled_camera_delta() {
        let frame = include_str!("main.rs");
        let call = frame
            .split("roads.update_traffic(")
            .nth(1)
            .unwrap()
            .split(");")
            .next()
            .unwrap();
        assert!(call.contains("delta_scene_time"));
        assert!(!call.contains("delta_camera_motion_seconds"));
    }

    #[test]
    fn fixed_steps_do_not_depend_on_the_render_frame_rate() {
        let network = route_network();
        let route = network.routes[0].clone();
        let eye = route.at(30.0).position;
        let mut a = Simulation::new(network.clone());
        let mut b = Simulation::new(network);
        a.target_population = 0;
        b.target_population = 0;
        a.agents = vec![agent(route.clone(), 1, 30.0, 0.0)];
        b.agents = vec![agent(route, 1, 30.0, 0.0)];
        for _ in 0..300 {
            a.advance(1.0 / 30.0, eye, DVec3::Y, None);
        }
        for _ in 0..1200 {
            b.advance(1.0 / 120.0, eye, DVec3::Y, None);
        }
        assert!((a.agents[0].distance - b.agents[0].distance).abs() < 1e-8);
        assert!((a.agents[0].speed - b.agents[0].speed).abs() < 1e-8);
    }

    #[test]
    fn swept_player_collision_blocks_the_cart_but_not_the_other_lane_or_bridge() {
        let network = route_network();
        let route = network.routes[0].clone();
        let p = route.at(60.0);
        let up = p.position.normalize();
        let right = p.forward.cross(up).normalize();
        let mut sim = Simulation::new(network);
        sim.agents = vec![agent(route, 1, 60.0, 0.0)];
        let a = p.position - p.forward * 10.0 + up * 1.4;
        let b = p.position + p.forward * 10.0 + up * 1.4;
        assert!(sim.blocks_motion(a, b, p.forward));
        assert!(!sim.blocks_motion(a + right * 4.1, b + right * 4.1, p.forward));
        assert!(!sim.blocks_motion(a + up * 8.0, b + up * 8.0, p.forward));
    }
    #[test]
    fn snapshot_remapping_preserves_world_position_and_identity() {
        let network = route_network();
        let route = network.routes[0].clone();
        let p = route.at(63.7);
        let mut sim = Simulation::new(network.clone());
        sim.agents = vec![agent(route, 941, 63.7, 3.0)];
        sim.adopt(network, p.position);
        assert_eq!(sim.agents[0].id, 941);
        assert!(sim.agents[0].pose(1.0).position.distance(p.position) < 0.01);
    }
}

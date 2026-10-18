//! Bounded height-aware rerouting for networks rejected by profile fitting.
//!
//! A* searches (lattice point, 10m height, incoming heading), not just xy.
//! Every span must satisfy physical grade, pier and sampled seabed-cover caps.
//! New branches join existing states at THEIR height, producing shared trunks
//! and junctions instead of independently fitted overlapping roads.
use crate::{
    road_network::{self, Node, Region, RoadEdge, Survey},
    road_network_profile::{self as profile, Profile, Sample},
};
use glam::DVec3;
use serde::Serialize;
use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap},
    time::Instant,
};

const HEIGHT_STEP: f64 = 10.0;
// Leave room for the verifier's conservative global radius and later curves.
const SEARCH_GRADE: f64 = crate::road_route::MAX_GRADE - 0.005;
const MAX_STATES: usize = 400_000;
// Bounded weighted A*: engineering feasibility matters more than proving an
// exact minimum of the construction proxy. Never relax any geometric bound.
const HEURISTIC_WEIGHT: f64 = 2.0;
const DIRECTIONS: [(isize, isize); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

#[derive(Default, Serialize)]
pub struct Stats {
    pub searched_terminals: usize,
    pub expanded_states: usize,
    pub surveyed_spans: usize,
    pub seconds: f64,
}
#[derive(Debug, Serialize)]
pub struct Rejected {
    pub terminal: usize,
    pub connected_terminals: usize,
    pub reason: &'static str,
    pub expanded_states: usize,
}
pub struct Plan {
    pub region: Region,
    pub profile: Profile,
    pub stats: Stats,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
struct Position {
    cell: usize,
    level: i32,
}
impl Position {
    fn height(self) -> f64 {
        self.level as f64 * HEIGHT_STEP
    }
}
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
struct State {
    position: Position,
    heading: u8,
}
#[derive(Clone, Copy)]
struct Record {
    cost: f64,
    parent: Option<State>,
}
#[derive(Clone, Copy)]
struct Queue {
    estimate: f64,
    cost: f64,
    state: State,
}
impl PartialEq for Queue {
    fn eq(&self, b: &Self) -> bool {
        self.cmp(b).is_eq()
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
        self.estimate
            .total_cmp(&b.estimate)
            .then(self.cost.total_cmp(&b.cost))
            .then(
                (
                    self.state.position.cell,
                    self.state.position.level,
                    self.state.heading,
                )
                    .cmp(&(
                        b.state.position.cell,
                        b.state.position.level,
                        b.state.heading,
                    )),
            )
    }
}

struct Span {
    length: f64,
    samples: Vec<(f64, Sample)>,
}
impl Span {
    fn survey(a: DVec3, b: DVec3, radius: f64, terrain: &impl Survey) -> Result<Self, String> {
        let length = distance(a, b, radius);
        let count = (length / profile::STEP).ceil().max(1.0) as usize;
        let samples = (0..=count)
            .map(|i| {
                let t = i as f64 / count as f64;
                Ok((
                    t,
                    profile::sample_at(a.lerp(b, t).normalize(), radius, terrain)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { length, samples })
    }
    fn cost(&self, a: f64, b: f64, radius: f64) -> Option<f64> {
        if (a - b).abs() > SEARCH_GRADE * self.length * (1.0 + a.min(b) / radius) + 1e-7 {
            return None;
        }
        let mut effort = 0.0;
        for &(t, s) in &self.samples {
            let h = a + (b - a) * t;
            if h > s.cap + 1e-7 {
                return None;
            }
            let cover = s.ground.height - h;
            // Open cuts are expensive; sufficiently buried rock is a tunnel,
            // whose cost grows more gently with overburden. Sea is ALWAYS bore.
            let construction = if s.ground.ocean {
                6.0 + cover.max(0.0) / 2000.0
            } else if cover > 19.0 {
                3.0 + cover / 2000.0
            } else {
                1.0 + 0.002 * (cover + crate::road::CUT_FLOOR_DEPTH).max(0.0).powi(2)
                    + 0.01 * (h - s.ground.height - 8.0).max(0.0)
            };
            effort += construction;
        }
        Some(self.length * effort / self.samples.len() as f64 + 0.2 * (a - b).abs())
    }
}
fn distance(a: DVec3, b: DVec3, r: f64) -> f64 {
    a.cross(b).length().atan2(a.dot(b)) * r
}

#[derive(Clone, Copy)]
struct Neighbour {
    cell: usize,
    heading: u8,
    length: f64,
}
struct Grid {
    dirs: Vec<DVec3>,
    samples: Vec<Sample>,
    neighbours: Vec<Vec<Neighbour>>,
    spans: HashMap<(usize, usize), Span>,
    radius: f64,
    minimum_level: i32,
    maximum_level: i32,
}
impl Grid {
    fn survey(region: &Region, radius: f64, terrain: &impl Survey) -> Result<Self, String> {
        let side = road_network::LOCAL_QUADS + 1;
        let mut dirs = Vec::new();
        let mut samples = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let d = road_network::direction(
                    region.key,
                    x as f64 / (side - 1) as f64,
                    y as f64 / (side - 1) as f64,
                );
                dirs.push(d);
                samples.push(profile::sample_at(d, radius, terrain)?);
            }
        }
        let mut lo = samples
            .iter()
            .map(|s| s.cap - 40.0)
            .fold(f64::INFINITY, f64::min);
        let mut hi = samples
            .iter()
            .map(|s| s.cap)
            .fold(f64::NEG_INFINITY, f64::max);
        for terminal in &region.terminals {
            let h =
                profile::sample_at(DVec3::from_array(terminal.direction), radius, terrain)?.anchor;
            lo = lo.min(h - 40.0);
            hi = hi.max(h + 10.0);
        }
        if lo <= -radius || !lo.is_finite() || !hi.is_finite() {
            return Err("invalid rerouting height bounds".into());
        }
        let mut neighbours = vec![Vec::new(); dirs.len()];
        for (i, adjacent) in neighbours.iter_mut().enumerate() {
            let (x, y) = ((i % side) as isize, (i / side) as isize);
            for (heading, &(dx, dy)) in DIRECTIONS.iter().enumerate() {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= side as isize || ny >= side as isize {
                    continue;
                }
                // One diagonal per square, matching the planar topology grid.
                if dx != 0 && dy != 0 && ((dx == dy) != ((x.min(nx) + y.min(ny)) % 2 == 0)) {
                    continue;
                }
                let cell = ny as usize * side + nx as usize;
                adjacent.push(Neighbour {
                    cell,
                    heading: heading as u8,
                    length: distance(dirs[i], dirs[cell], radius),
                });
            }
        }
        Ok(Self {
            dirs,
            samples,
            neighbours,
            spans: HashMap::new(),
            radius,
            minimum_level: (lo / HEIGHT_STEP).floor() as i32,
            maximum_level: (hi / HEIGHT_STEP).ceil() as i32,
        })
    }
    fn span(&mut self, a: usize, b: usize, terrain: &impl Survey) -> Result<&Span, String> {
        let pair = (a.min(b), a.max(b));
        if !self.spans.contains_key(&pair) {
            self.spans.insert(
                pair,
                Span::survey(self.dirs[pair.0], self.dirs[pair.1], self.radius, terrain)?,
            );
        }
        Ok(&self.spans[&pair])
    }
    fn cost(
        &mut self,
        a: Position,
        b: Position,
        terrain: &impl Survey,
    ) -> Result<Option<f64>, String> {
        let radius = self.radius;
        let span = self.span(a.cell, b.cell, terrain)?;
        Ok(if a.cell <= b.cell {
            span.cost(a.height(), b.height(), radius)
        } else {
            span.cost(b.height(), a.height(), radius)
        })
    }
    fn approaches(
        &self,
        region: &Region,
        d: DVec3,
        height: f64,
        terrain: &impl Survey,
    ) -> Result<Vec<(Position, f64)>, String> {
        let (normal, u, v) = crate::planet::cube_face_basis(region.key.face.index());
        let scale = d.dot(normal);
        let cells = f64::from(1_u32 << region.key.level);
        let quad = road_network::LOCAL_QUADS;
        let xy = [
            ((d.dot(u) / scale + 1.0) * 0.5 * cells - region.key.x as f64) * quad as f64,
            ((d.dot(v) / scale + 1.0) * 0.5 * cells - region.key.y as f64) * quad as f64,
        ];
        let x = xy[0].clamp(0.0, (quad - 1) as f64) as usize;
        let y = xy[1].clamp(0.0, (quad - 1) as f64) as usize;
        let side = quad + 1;
        let mut result = Vec::new();
        for cell in [
            y * side + x,
            y * side + x + 1,
            (y + 1) * side + x,
            (y + 1) * side + x + 1,
        ] {
            let span = Span::survey(d, self.dirs[cell], self.radius, terrain)?;
            let allowance = SEARCH_GRADE * span.length * (1.0 + height.abs() / self.radius);
            let lo = ((height - allowance) / HEIGHT_STEP).ceil() as i32;
            let hi =
                ((height + allowance).min(self.samples[cell].cap) / HEIGHT_STEP).floor() as i32;
            for level in lo.max(self.minimum_level)..=hi.min(self.maximum_level) {
                let position = Position { cell, level };
                if let Some(cost) = span.cost(height, position.height(), self.radius) {
                    result.push((position, cost));
                }
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy)]
struct Goal {
    node: usize,
    tail_cost: f64,
}
struct Path {
    positions: Vec<Position>,
    goal: Goal,
    cost: f64,
    expanded: usize,
}

/// Lower-bound distance to the growing trunk. A planar multi-source Dijkstra
/// supplies an admissible A* heuristic without scanning every target per state.
fn heuristic(
    grid: &mut Grid,
    goals: &HashMap<Position, Goal>,
    terrain: &impl Survey,
) -> Result<(Vec<f64>, f64, f64), String> {
    let mut distances = vec![f64::INFINITY; grid.dirs.len()];
    let mut heap = BinaryHeap::new();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    // Queue's deterministic total ordering is also suitable for a planar state.
    let has_water = grid.samples.iter().any(|s| s.ground.ocean);
    for (&p, goal) in goals {
        lo = lo.min(p.height());
        hi = hi.max(p.height());
        if distances[p.cell] <= goal.tail_cost {
            continue;
        }
        distances[p.cell] = goal.tail_cost;
        heap.push(Reverse(Queue {
            estimate: goal.tail_cost,
            cost: goal.tail_cost,
            state: State {
                position: p,
                heading: 8,
            },
        }));
    }
    while let Some(Reverse(item)) = heap.pop() {
        let i = item.state.position.cell;
        if item.cost > distances[i] {
            continue;
        }
        for n in grid.neighbours[i].clone() {
            // Water construction costs at least six times a land span. Use
            // the same cached samples as feasibility checks; a distance-only
            // guide floods the land with states before attempting a sea bore.
            let lower = if has_water {
                let span = grid.span(i, n.cell, terrain)?;
                span.length
                    * span
                        .samples
                        .iter()
                        .map(|(_, s)| if s.ground.ocean { 6.0 } else { 1.0 })
                        .sum::<f64>()
                    / span.samples.len() as f64
            } else {
                n.length
            };
            let cost = item.cost + lower;
            if cost < distances[n.cell] {
                distances[n.cell] = cost;
                heap.push(Reverse(Queue {
                    estimate: cost,
                    cost,
                    state: State {
                        position: Position {
                            cell: n.cell,
                            level: 0,
                        },
                        heading: 8,
                    },
                }));
            }
        }
    }
    Ok((distances, lo, hi))
}

fn search(
    grid: &mut Grid,
    starts: &[(Position, f64)],
    goals: &HashMap<Position, Goal>,
    terrain: &impl Survey,
    limit: usize,
) -> Result<Result<Path, (&'static str, usize)>, String> {
    if starts.is_empty() {
        return Ok(Err(("no graded approach to terminal", 0)));
    }
    if goals.is_empty() {
        return Ok(Err(("no graded approach to existing trunk", 0)));
    }
    let (planar, goal_lo, goal_hi) = heuristic(grid, goals, terrain)?;
    let max_radius_ratio = 1.0 + (grid.maximum_level as f64 * HEIGHT_STEP).max(0.0) / grid.radius;
    let estimate = |p: Position| {
        planar[p.cell].max(
            (goal_lo - p.height()).max(p.height() - goal_hi).max(0.0)
                / (SEARCH_GRADE * max_radius_ratio),
        )
    };
    let mut records = HashMap::<State, Record>::new();
    let mut heap = BinaryHeap::new();
    for &(position, cost) in starts {
        let state = State {
            position,
            heading: 8,
        };
        if records.get(&state).is_none_or(|r| cost < r.cost) {
            records.insert(state, Record { cost, parent: None });
            heap.push(Reverse(Queue {
                estimate: cost + HEURISTIC_WEIGHT * estimate(position),
                cost,
                state,
            }));
        }
    }
    let mut best: Option<(f64, State, Goal)> = None;
    let mut expanded = 0;
    while let Some(Reverse(item)) = heap.pop() {
        if best.is_some_and(|b| item.estimate >= b.0) {
            break;
        }
        if item.cost > records[&item.state].cost {
            continue;
        }
        expanded += 1;
        if expanded > limit {
            return Ok(Err(("height-state search budget exceeded", expanded)));
        }
        let p = item.state.position;
        if let Some(&goal) = goals.get(&p) {
            let cost = item.cost + goal.tail_cost;
            if best.is_none_or(|b| cost < b.0) {
                best = Some((cost, item.state, goal));
            }
            if goal.tail_cost == 0.0 {
                break;
            }
        }
        for n in grid.neighbours[p.cell].clone() {
            if item.state.heading < 8 {
                let turn = (n.heading as i32 - item.state.heading as i32).abs();
                if turn.min(8 - turn) > 2 {
                    continue;
                }
            }
            let allowance = SEARCH_GRADE * n.length * (1.0 + p.height().abs() / grid.radius);
            let lo = ((p.height() - allowance) / HEIGHT_STEP).ceil() as i32;
            let hi = ((p.height() + allowance).min(grid.samples[n.cell].cap) / HEIGHT_STEP).floor()
                as i32;
            for level in lo.max(grid.minimum_level)..=hi.min(grid.maximum_level) {
                let next = Position {
                    cell: n.cell,
                    level,
                };
                let Some(span_cost) = grid.cost(p, next, terrain)? else {
                    continue;
                };
                let turn = if item.state.heading < 8 {
                    let t = (n.heading as i32 - item.state.heading as i32).abs();
                    t.min(8 - t) as f64
                } else {
                    0.0
                };
                let cost = item.cost + span_cost + turn * 8.0;
                let state = State {
                    position: next,
                    heading: n.heading,
                };
                if records.get(&state).is_none_or(|r| cost < r.cost) {
                    if (!records.contains_key(&state) && records.len() >= limit)
                        || heap.len() >= limit.saturating_mul(4)
                    {
                        return Ok(Err(("height-state memory budget exceeded", expanded)));
                    }
                    records.insert(
                        state,
                        Record {
                            cost,
                            parent: Some(item.state),
                        },
                    );
                    heap.push(Reverse(Queue {
                        estimate: cost + HEURISTIC_WEIGHT * estimate(next),
                        cost,
                        state,
                    }));
                }
            }
        }
    }
    let Some((cost, mut state, goal)) = best else {
        return Ok(Err((
            "no graded route within region/height bounds",
            expanded,
        )));
    };
    let mut positions = Vec::new();
    loop {
        positions.push(state.position);
        match records[&state].parent {
            Some(p) => state = p,
            None => break,
        }
    }
    positions.reverse();
    // A heading-change loop can revisit the same physical height-state. Erase
    // that positive-cost loop before constructing the tree (never a cycle).
    let mut unique = Vec::new();
    let mut at = HashMap::new();
    for p in positions {
        if let Some(&index) = at.get(&p) {
            for old in unique.drain(index + 1..) {
                at.remove(&old);
            }
        } else {
            at.insert(p, unique.len());
            unique.push(p);
        }
    }
    Ok(Ok(Path {
        positions: unique,
        goal,
        cost,
        expanded,
    }))
}

pub fn reroute(
    region: &Region,
    radius: f64,
    terrain: &impl Survey,
) -> Result<Result<Plan, Rejected>, String> {
    reroute_with_limit(region, radius, terrain, MAX_STATES)
}
fn reroute_with_limit(
    region: &Region,
    radius: f64,
    terrain: &impl Survey,
    limit: usize,
) -> Result<Result<Plan, Rejected>, String> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err("invalid rerouting radius".into());
    }
    if region.terminals.is_empty() {
        let p = Profile::fit(region, radius, terrain)?
            .map_err(|e| format!("empty network rejected: {e:?}"))?;
        return Ok(Ok(Plan {
            region: Region {
                key: region.key,
                village_count: 0,
                terminals: Vec::new(),
                terminal_nodes: Vec::new(),
                nodes: Vec::new(),
                edges: Vec::new(),
                junction_count: 0,
                length_meters: 0.0,
                construction_cost_proxy: 0.0,
                lattice_survey_samples: 0,
            },
            profile: p,
            stats: Stats::default(),
        }));
    }
    let started = Instant::now();
    let mut stats = Stats::default();
    let dirs: Vec<_> = region
        .terminals
        .iter()
        .map(|t| DVec3::from_array(t.direction))
        .collect();
    let samples = dirs
        .iter()
        .map(|&d| profile::sample_at(d, radius, terrain))
        .collect::<Result<Vec<_>, _>>()?;
    let mut grid = Grid::survey(region, radius, terrain)?;
    let root = (0..dirs.len())
        .min_by(|&a, &b| {
            samples[a]
                .anchor
                .total_cmp(&samples[b].anchor)
                .then(a.cmp(&b))
        })
        .unwrap();
    let mut nodes = vec![Node {
        direction: dirs[root].to_array(),
        ground_meters: samples[root].ground.height,
        ocean: samples[root].ground.ocean,
    }];
    let mut heights = vec![samples[root].anchor];
    let mut edges = Vec::new();
    let mut terminal_nodes = vec![usize::MAX; dirs.len()];
    terminal_nodes[root] = 0;
    let mut connected = vec![root];
    let mut goals = HashMap::new();
    for (p, cost) in grid.approaches(region, dirs[root], heights[0], terrain)? {
        goals.insert(
            p,
            Goal {
                node: 0,
                tail_cost: cost,
            },
        );
    }
    let mut total_cost = 0.0;
    while connected.len() < dirs.len() {
        let terminal = (0..dirs.len())
            .filter(|&i| terminal_nodes[i] == usize::MAX)
            .min_by(|&a, &b| {
                let effort = |i: usize| {
                    connected
                        .iter()
                        .map(|&j| {
                            distance(dirs[i], dirs[j], radius)
                                .max((samples[i].anchor - samples[j].anchor).abs() / SEARCH_GRADE)
                        })
                        .fold(f64::INFINITY, f64::min)
                };
                effort(a).total_cmp(&effort(b)).then(a.cmp(&b))
            })
            .unwrap();
        // Coincident settlements keep distinct metadata but share one node.
        if let Some(i) = nodes.iter().enumerate().find_map(|(i, n)| {
            (DVec3::from_array(n.direction).distance(dirs[terminal]) < 1e-12
                && (heights[i] - samples[terminal].anchor).abs() < 1e-6)
                .then_some(i)
        }) {
            terminal_nodes[terminal] = i;
            connected.push(terminal);
            continue;
        }
        let starts = grid.approaches(region, dirs[terminal], samples[terminal].anchor, terrain)?;
        stats.searched_terminals += 1;
        let path = match search(&mut grid, &starts, &goals, terrain, limit)? {
            Ok(p) => p,
            Err((reason, expanded)) => {
                return Ok(Err(Rejected {
                    terminal,
                    connected_terminals: connected.len(),
                    reason,
                    expanded_states: stats.expanded_states + expanded,
                }));
            }
        };
        stats.expanded_states += path.expanded;
        total_cost += path.cost;
        let mut previous = path.goal.node;
        for &p in path.positions.iter().rev() {
            let node = if let Some(goal) = goals.get(&p).filter(|g| g.tail_cost == 0.0) {
                goal.node
            } else if DVec3::from_array(nodes[previous].direction).distance(grid.dirs[p.cell])
                < 1e-12
                && (heights[previous] - p.height()).abs() < 1e-6
            {
                goals.insert(
                    p,
                    Goal {
                        node: previous,
                        tail_cost: 0.0,
                    },
                );
                previous
            } else {
                let node = nodes.len();
                let s = grid.samples[p.cell];
                nodes.push(Node {
                    direction: grid.dirs[p.cell].to_array(),
                    ground_meters: s.ground.height,
                    ocean: s.ground.ocean,
                });
                heights.push(p.height());
                goals.insert(
                    p,
                    Goal {
                        node,
                        tail_cost: 0.0,
                    },
                );
                node
            };
            if node != previous {
                edges.push(RoadEdge {
                    a: previous,
                    b: node,
                    requires_undersea_tunnel: nodes[previous].ocean || nodes[node].ocean,
                });
            }
            previous = node;
        }
        if DVec3::from_array(nodes[previous].direction).distance(dirs[terminal]) < 1e-12
            && (heights[previous] - samples[terminal].anchor).abs() < 1e-6
        {
            terminal_nodes[terminal] = previous;
            connected.push(terminal);
            continue;
        }
        let node = nodes.len();
        nodes.push(Node {
            direction: dirs[terminal].to_array(),
            ground_meters: samples[terminal].ground.height,
            ocean: samples[terminal].ground.ocean,
        });
        heights.push(samples[terminal].anchor);
        terminal_nodes[terminal] = node;
        edges.push(RoadEdge {
            a: previous,
            b: node,
            requires_undersea_tunnel: nodes[previous].ocean || nodes[node].ocean,
        });
        connected.push(terminal);
    }
    let mut degree = vec![0; nodes.len()];
    for e in &edges {
        degree[e.a] += 1;
        degree[e.b] += 1;
    }
    let length_meters = edges
        .iter()
        .map(|e| {
            distance(
                DVec3::from_array(nodes[e.a].direction),
                DVec3::from_array(nodes[e.b].direction),
                radius,
            )
        })
        .sum();
    let result = Region {
        key: region.key,
        village_count: region.village_count,
        terminals: region.terminals.clone(),
        terminal_nodes,
        nodes,
        edges,
        junction_count: degree.iter().filter(|&&d| d >= 3).count(),
        length_meters,
        construction_cost_proxy: total_cost,
        lattice_survey_samples: grid.dirs.len(),
    };
    if result.edges.len() + 1 != result.nodes.len() {
        return Err("height-aware rerouting produced a non-tree".into());
    }
    let fixed: Vec<_> = heights.into_iter().enumerate().collect();
    let p = match Profile::fit_fixed(&result, radius, terrain, &fixed)? {
        Ok(p) => p,
        Err(_) => {
            return Ok(Err(Rejected {
                terminal: root,
                connected_terminals: connected.len(),
                reason: "dense verification rejected searched alignment",
                expanded_states: stats.expanded_states,
            }));
        }
    };
    stats.surveyed_spans = grid.spans.len();
    stats.seconds = started.elapsed().as_secs_f64();
    Ok(Ok(Plan {
        region: result,
        profile: p,
        stats,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_network::{Ground, Terminal, TerminalKind};
    use planet_coretypes::{CubeFace, TileKey};
    fn key() -> TileKey {
        TileKey {
            face: CubeFace::PositiveX,
            level: 8,
            x: 128,
            y: 128,
        }
    }
    fn region(key: TileKey, points: &[(f64, f64)]) -> Region {
        let dirs: Vec<_> = points
            .iter()
            .map(|&(x, y)| road_network::direction(key, x, y).to_array())
            .collect();
        Region {
            key,
            village_count: dirs.len(),
            terminals: dirs
                .iter()
                .map(|&direction| Terminal {
                    direction,
                    kind: TerminalKind::Village,
                })
                .collect(),
            terminal_nodes: (0..dirs.len()).collect(),
            nodes: dirs
                .iter()
                .map(|&direction| Node {
                    direction,
                    ground_meters: 900.0,
                    ocean: false,
                })
                .collect(),
            edges: (1..dirs.len())
                .map(|b| RoadEdge {
                    a: b - 1,
                    b,
                    requires_undersea_tunnel: false,
                })
                .collect(),
            junction_count: 0,
            length_meters: 0.0,
            construction_cost_proxy: 0.0,
            lattice_survey_samples: 0,
        }
    }
    enum Land {
        Flat(f64),
        Ramp,
        Islands,
    }
    impl Survey for Land {
        fn ground(&self, d: DVec3) -> Result<Ground, String> {
            let u = -d.z / d.x;
            let x = u * 4e6;
            Ok(match self {
                Land::Flat(h) => Ground {
                    height: *h,
                    ocean: false,
                },
                Land::Ramp => Ground {
                    height: 900.0 + 0.5 * (x - 15000.0).clamp(0.0, 1000.0),
                    ocean: false,
                },
                Land::Islands => {
                    let ocean = x > 15000.0 && x < 16250.0;
                    Ground {
                        height: if ocean { -100.0 } else { 200.0 },
                        ocean,
                    }
                }
            })
        }
        fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
            Ok(Vec::new())
        }
    }
    fn validate(plan: &Plan, original: &Region, terrain: &impl Survey) {
        assert_eq!(plan.region.terminals.len(), original.terminals.len());
        assert_eq!(plan.region.village_count, original.village_count);
        assert_eq!(plan.region.edges.len() + 1, plan.region.nodes.len());
        let mut adjacency = vec![Vec::new(); plan.region.nodes.len()];
        for e in &plan.region.edges {
            adjacency[e.a].push(e.b);
            adjacency[e.b].push(e.a);
        }
        let mut seen = vec![false; adjacency.len()];
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            stack.extend(&adjacency[i]);
        }
        assert!(seen.iter().all(|&b| b));
        for (i, t) in original.terminals.iter().enumerate() {
            let node = plan.region.terminal_nodes[i];
            assert_eq!(
                plan.region.nodes[node].direction, t.direction,
                "moved a terminal"
            );
            let expected = profile::sample_at(DVec3::from_array(t.direction), 4e6, terrain)
                .unwrap()
                .anchor;
            assert!(
                (plan.profile.heights_meters[node] - expected).abs() < 1e-6,
                "moved an anchor height"
            );
        }
        for (d, &h) in plan
            .profile
            .directions
            .iter()
            .zip(&plan.profile.heights_meters)
        {
            let g = terrain.ground(DVec3::from_array(*d)).unwrap();
            assert!(
                h <= if g.ocean {
                    g.height - 14.0 + 1e-6
                } else {
                    g.height + 32.0 + 1e-6
                }
            );
        }
        for &[a, b] in &plan.profile.segments {
            let world = |i: usize| {
                DVec3::from_array(plan.profile.directions[i])
                    * (4e6 + plan.profile.heights_meters[i])
            };
            let (a, b) = (world(a), world(b));
            let angle = a
                .normalize()
                .cross(b.normalize())
                .length()
                .atan2(a.normalize().dot(b.normalize()));
            let grade =
                (a.length() - b.length()).abs() / (angle * a.length().min(b.length())).max(1e-9);
            assert!(
                grade <= crate::road_route::MAX_GRADE + 1e-6,
                "physical grade {grade}"
            );
        }
    }
    #[test]
    fn road_network_height_routing_takes_a_longer_route_without_relaxing_grade() {
        let r = region(key(), &[(0.49, 0.5), (0.51, 0.5)]);
        let terrain = Land::Ramp;
        assert!(Profile::fit(&r, 4e6, &terrain).unwrap().is_err());
        let p = reroute(&r, 4e6, &terrain).unwrap().unwrap();
        validate(&p, &r, &terrain);
        let direct = distance(
            DVec3::from_array(r.nodes[0].direction),
            DVec3::from_array(r.nodes[1].direction),
            4e6,
        );
        assert!(p.region.length_meters > direct * 1.5);
    }
    #[test]
    fn road_network_height_routing_joins_existing_trunks_at_shared_heights() {
        let r = region(key(), &[(0.25, 0.5), (0.5, 0.5), (0.75, 0.5), (0.5, 0.75)]);
        let p = reroute(&r, 4e6, &Land::Flat(900.0)).unwrap().unwrap();
        validate(&p, &r, &Land::Flat(900.0));
        assert!(p.region.junction_count > 0);
        let again = reroute(&r, 4e6, &Land::Flat(900.0)).unwrap().unwrap();
        assert_eq!(
            serde_json::to_string(&p.region).unwrap(),
            serde_json::to_string(&again.region).unwrap()
        );
        assert_eq!(p.profile.heights_meters, again.profile.heights_meters);
    }
    #[test]
    fn road_network_height_routing_does_not_create_zero_length_lattice_spurs() {
        let r = region(key(), &[(0.25, 0.5), (0.5, 0.5), (0.5, 0.5)]);
        let p = reroute(&r, 4e6, &Land::Flat(902.0)).unwrap().unwrap();
        validate(&p, &r, &Land::Flat(902.0));
        assert_eq!(p.region.terminal_nodes[1], p.region.terminal_nodes[2]);
        for e in &p.region.edges {
            assert!(
                distance(
                    DVec3::from_array(p.region.nodes[e.a].direction),
                    DVec3::from_array(p.region.nodes[e.b].direction),
                    4e6
                ) > 1e-4
            );
        }
    }
    #[test]
    fn road_network_height_routing_islands_dive_beneath_the_bed_with_longer_approaches() {
        let r = region(key(), &[(0.455, 0.5), (0.545, 0.5)]);
        let terrain = Land::Islands;
        assert!(Profile::fit(&r, 4e6, &terrain).unwrap().is_err());
        let p = reroute(&r, 4e6, &terrain).unwrap().unwrap();
        validate(&p, &r, &terrain);
        assert!(p.profile.undersea_segments > 0);
        assert!(
            p.profile
                .minimum_sampled_undersea_roof_cover_meters
                .unwrap()
                >= 5.0 - 1e-6
        );
    }
    #[test]
    fn road_network_height_routing_budget_exhaustion_is_explicit_not_a_partial_network() {
        let r = region(key(), &[(0.49, 0.5), (0.51, 0.5)]);
        let p = reroute_with_limit(&r, 4e6, &Land::Ramp, 0).unwrap();
        let Err(failure) = p else {
            panic!("zero budget accepted a plan");
        };
        assert_eq!(failure.connected_terminals, 1);
        assert!(failure.reason.contains("budget"));
    }
    #[test]
    fn road_network_height_routing_cube_seam_gateways_keep_identical_elevations() {
        struct Ocean;
        impl Survey for Ocean {
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
        let a = TileKey {
            x: 255,
            y: 128,
            ..key()
        };
        let gate = road_network::direction(a, 1.0, 0.5).to_array();
        let b = planet_coretypes::tile_key_for_direction(road_network::direction(a, 1.01, 0.5), 8);
        let mut elevations = Vec::new();
        for k in [a, b] {
            let mut r = region(k, &[(0.5, 0.5), (0.5, 0.7)]);
            r.terminals[0] = Terminal {
                direction: gate,
                kind: TerminalKind::Gateway { link: 77 },
            };
            let p = reroute(&r, 4e6, &Ocean).unwrap().unwrap();
            validate(&p, &r, &Ocean);
            elevations.push(p.profile.heights_meters[p.region.terminal_nodes[0]]);
        }
        assert_eq!(elevations[0].to_bits(), elevations[1].to_bits());
    }
}

//! Shared-height engineering check for a surveyed network tree.
//!
//! Profiles must be solved together: independently fitted roads can meet at
//! different elevations. Village access points and cross-region gateways are
//! fixed; interior nodes get a Lipschitz extension bounded by terrain/pier and
//! seabed-cover limits. An infeasible alignment is rejected for rerouting, not
//! silently steepened, disconnected, or given a fake underwater portal.
use crate::road_network::{Region, Survey};
use glam::DVec3;
use serde::Serialize;
use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
};

pub(crate) const STEP: f64 = 20.0;
// Cover the 5m junction pad (and any acute-angle mouth overlap below), rather
// than flattening a whole 20m survey station regardless of the junction shape.
const APRON_LENGTH: f64 = crate::road::HALF_WIDTH;
// Radial heights (unlike the pilot's tangent z) already include curvature.
// Retain 0.1 percentage point for tessellation below the user's 25% limit.
const GRADE: f64 = crate::road_route::MAX_GRADE - 0.001;
const CLEARANCE: f64 = 8.0;
const PIER_CAP: f64 = 32.0;
const UNDERSEA_CROWN: f64 = 9.0;
const UNDERSEA_COVER: f64 = 5.0;
const UNDERSEA_DECK_DEPTH: f64 = 20.0;

#[derive(Serialize)]
pub struct Profile {
    pub heights_meters: Vec<f64>,
    pub directions: Vec<[f64; 3]>,
    pub segments: Vec<[usize; 2]>,
    pub maximum_grade: f64,
    pub maximum_grade_change_per_meter: f64,
    pub maximum_deck_above_ground_meters: f64,
    pub minimum_sampled_undersea_roof_cover_meters: Option<f64>,
    pub undersea_segments: usize,
}
#[derive(Serialize, Debug)]
pub struct Rejected {
    pub reason: &'static str,
    pub node: usize,
    pub required_minimum_meters: f64,
    pub allowed_maximum_meters: f64,
}
#[derive(Clone, Copy)]
struct Queue {
    height: f64,
    index: usize,
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
        self.height
            .total_cmp(&b.height)
            .then(self.index.cmp(&b.index))
    }
}

fn distance(a: DVec3, b: DVec3, r: f64) -> f64 {
    a.cross(b).length().atan2(a.dot(b)) * r
}

/// Intersection of two complete carriageway quads, including curved-station
/// miters. A centreline capsule overestimates acute mouths and can make dense
/// village profiles infeasible by flattening road that is already clear.
fn overlap_polygon(mut points: Vec<glam::DVec2>, clip: &[glam::DVec2; 4]) -> Vec<glam::DVec2> {
    let cross = |a: glam::DVec2, b: glam::DVec2| a.x * b.y - a.y * b.x;
    for i in 0..4 {
        if points.is_empty() {
            break;
        }
        let origin = clip[i];
        let edge = clip[(i + 1) % 4] - origin;
        let input = std::mem::take(&mut points);
        let mut previous = *input.last().unwrap();
        let mut before = cross(edge, previous - origin);
        for p in input {
            let after = cross(edge, p - origin);
            if (before >= 0.0) != (after >= 0.0) {
                points.push(previous.lerp(p, (before / (before - after)).clamp(0.0, 1.0)));
            }
            if after >= 0.0 {
                points.push(p);
            }
            previous = p;
            before = after;
        }
    }
    points
}

/// Trace only connected overlap at the mouth, not later unrelated crossings.
/// Each arm gets its own extent; the incoming approach need not be as flat as
/// a long outgoing fork. All of this runs on the existing engineering worker.
fn fork_aprons(
    i: usize,
    dirs: &[DVec3],
    adjacent: &[Vec<usize>],
    terminals: &[usize],
    radius: f64,
    terrain: &impl Survey,
) -> Result<Vec<f64>, String> {
    use std::collections::{HashSet, VecDeque};
    let up = dirs[i];
    let reference = if up.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y };
    let x = up.cross(reference).normalize();
    let y = up.cross(x);
    let point =
        |j: usize| glam::DVec2::new(dirs[j].dot(x), dirs[j].dot(y)) * (radius / dirs[j].dot(up));
    let arms: Vec<Vec<_>> = adjacent[i]
        .iter()
        .map(|&j| {
            let mut path = vec![(glam::DVec2::ZERO, 0.0)];
            let (mut previous, mut current) = (i, j);
            loop {
                let station =
                    path.last().unwrap().1 + distance(dirs[previous], dirs[current], radius);
                path.push((point(current), station));
                if current == i
                    || adjacent[current].len() != 2
                    || terminals.contains(&current)
                    || path.len() > dirs.len()
                {
                    break;
                }
                let Some(&next) = adjacent[current].iter().find(|&&n| n != previous) else {
                    break;
                };
                previous = current;
                current = next;
            }
            path
        })
        .collect();
    let quads: Vec<Vec<_>> = arms
        .iter()
        .map(|path| {
            let sides: Vec<_> = path
                .windows(2)
                .map(|p| {
                    let d = (p[1].0 - p[0].0).normalize_or_zero();
                    glam::DVec2::new(d.y, -d.x)
                })
                .collect();
            let mut sections = vec![sides[0]];
            for pair in sides.windows(2) {
                let middle = (pair[0] + pair[1]).normalize_or_zero();
                sections.push(middle / middle.dot(pair[0]).max(0.1));
            }
            sections.push(*sides.last().unwrap());
            path.windows(2)
                .enumerate()
                .map(|(j, p)| {
                    let a = sections[j] * crate::road::HALF_WIDTH;
                    let b = sections[j + 1] * crate::road::HALF_WIDTH;
                    [p[0].0 - a, p[0].0 + a, p[1].0 + b, p[1].0 - b]
                })
                .collect()
        })
        .collect();
    let mut extents: Vec<_> = arms.iter().map(|p| APRON_LENGTH.min(p[1].1)).collect();
    for (arm, first) in arms.iter().enumerate() {
        for (other, second) in arms.iter().enumerate().skip(arm + 1) {
            if first[1].0.dot(second[1].0) <= 0.0 {
                continue;
            }
            let cosine = first[1]
                .0
                .normalize()
                .dot(second[1].0.normalize())
                .clamp(-1.0, 1.0 - 1e-9);
            if cosine < crate::road_network_align::MAX_ANGLE.cos() {
                // Independent surveyed junctions retain their established
                // bounded apron. Full overlap tracing is for the shallow
                // mouths produced by merging tangent fillet prefixes; forcing
                // every acute survey fork flat can make unrelated hillsides
                // infeasible. General shared junction surfaces remain separate.
                let legacy = (APRON_LENGTH * ((1.0 + cosine) / (1.0 - cosine)).sqrt()).min(STEP);
                for (length, path) in extents.iter_mut().zip(&arms) {
                    *length = length.max(legacy.min(path[1].1));
                }
                continue;
            }
            let mut queue = VecDeque::from([(0, 0)]);
            let mut seen = HashSet::new();
            while let Some((a, b)) = queue.pop_front() {
                if terrain.cancelled() {
                    return Err("road generation superseded".into());
                }
                if a + 1 >= first.len() || b + 1 >= second.len() || !seen.insert((a, b)) {
                    continue;
                }
                let overlap = overlap_polygon(quads[arm][a].to_vec(), &quads[other][b]);
                if overlap.is_empty() {
                    continue;
                }
                for p in overlap {
                    for (index, path, k) in [(arm, first, a), (other, second, b)] {
                        let d = path[k + 1].0 - path[k].0;
                        let t = if d.length_squared() > 1e-12 {
                            ((p - path[k].0).dot(d) / d.length_squared()).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        extents[index] =
                            extents[index].max(path[k].1 + t * (path[k + 1].1 - path[k].1));
                    }
                }
                queue.push_back((a + 1, b));
                queue.push_back((a, b + 1));
            }
        }
    }
    Ok(extents)
}

/// Minimum of source heights plus grade * network distance. Negative source
/// heights are fine: all graph edge lengths are non-negative. Running this on
/// negated anchors also constructs their required lower envelope.
fn envelope(mut heights: Vec<f64>, adjacency: &[Vec<(usize, f64)>]) -> Vec<f64> {
    let mut queue = BinaryHeap::new();
    for (index, &height) in heights.iter().enumerate() {
        if height.is_finite() {
            queue.push(Reverse(Queue { height, index }));
        }
    }
    while let Some(Reverse(Queue { height, index })) = queue.pop() {
        if height > heights[index] {
            continue;
        }
        for &(next, length) in &adjacency[index] {
            let candidate = height + GRADE * length;
            if candidate < heights[next] {
                heights[next] = candidate;
                queue.push(Reverse(Queue {
                    height: candidate,
                    index: next,
                }));
            }
        }
    }
    heights
}

#[derive(Clone, Copy)]
pub(crate) struct Sample {
    pub ground: crate::road_network::Ground,
    pub cap: f64,
    pub anchor: f64,
}

fn anchor_height(g: crate::road_network::Ground, cap: f64) -> f64 {
    if g.ocean || cap < g.height {
        cap - (UNDERSEA_DECK_DEPTH - UNDERSEA_CROWN - UNDERSEA_COVER)
    } else {
        g.height + CLEARANCE
    }
}

pub(crate) fn sample_at(d: DVec3, radius: f64, terrain: &impl Survey) -> Result<Sample, String> {
    let ground = terrain.ground(d)?;
    if !ground.height.is_finite() {
        return Err("non-finite profile terrain".into());
    }
    let protected = terrain.settlement_core_at(d)?;
    let cap = if ground.ocean || protected {
        // Canonical radial stencil: shared gateway height on both regions/faces.
        let reference = if d.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y };
        let u = d.cross(reference).normalize();
        let v = d.cross(u);
        let mut bed = ground.height;
        for offset in [u, -u, v, -v] {
            let g = terrain.ground((d + offset * (8.0 / radius)).normalize())?;
            if !g.height.is_finite() {
                return Err("non-finite tunnel cover".into());
            }
            bed = bed.min(g.height);
        }
        bed - if protected {
            20.0
        } else {
            UNDERSEA_CROWN + UNDERSEA_COVER
        }
    } else {
        ground.height + PIER_CAP
    };
    Ok(Sample {
        ground,
        cap,
        anchor: anchor_height(ground, cap),
    })
}

pub(crate) fn terminal_sample(
    terminal: &crate::road_network::Terminal,
    radius: f64,
    terrain: &impl Survey,
) -> Result<Sample, String> {
    let mut sample = sample_at(DVec3::from_array(terminal.direction), radius, terrain)?;
    if matches!(terminal.kind, crate::road_network::TerminalKind::Village) && !sample.ground.ocean {
        sample.anchor = terrain.village_access_height(sample.ground);
    }
    Ok(sample)
}

impl Profile {
    pub fn fit(
        region: &Region,
        radius: f64,
        terrain: &impl Survey,
    ) -> Result<Result<Self, Rejected>, String> {
        Self::fit_seeded(region, radius, terrain, &[])
    }

    /// Searched lattice heights seed a feasible piecewise-linear profile.
    /// Vertical curves may adjust interior heights, but never terminal anchors;
    /// every shared junction still has one height for all its incident roads.
    pub(crate) fn fit_seeded(
        region: &Region,
        radius: f64,
        terrain: &impl Survey,
        fixed: &[(usize, f64)],
    ) -> Result<Result<Self, Rejected>, String> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err("invalid network profile radius".into());
        }
        let mut dirs: Vec<_> = region
            .nodes
            .iter()
            .map(|n| DVec3::from_array(n.direction))
            .collect();
        let mut original_adj = vec![Vec::new(); region.nodes.len()];
        for e in &region.edges {
            original_adj[e.a].push(e.b);
            original_adj[e.b].push(e.a);
        }
        let mut flat_nodes = vec![false; region.nodes.len()];
        for i in 0..region.nodes.len() {
            let neighbours: Vec<_> = original_adj[i]
                .iter()
                .copied()
                .filter(|&j| dirs[j].distance(dirs[i]) > 1e-10)
                .collect();
            let gateway = region
                .terminals
                .iter()
                .any(|t| DVec3::from_array(t.direction).distance(dirs[i]) < 1e-10);
            let flat = gateway
                || neighbours.len() > 2
                || (neighbours.len() == 2 && {
                    let up = dirs[i];
                    let a = (dirs[neighbours[0]] - up * dirs[neighbours[0]].dot(up))
                        .normalize_or_zero();
                    let b = (dirs[neighbours[1]] - up * dirs[neighbours[1]].dot(up))
                        .normalize_or_zero();
                    a.dot(b) > -20.0_f64.to_radians().cos()
                });
            flat_nodes[i] = flat;
        }
        let mut apron_spans = std::collections::HashMap::<(usize, usize), f64>::new();
        for (i, &flat) in flat_nodes.iter().enumerate() {
            if !flat {
                continue;
            }
            let lengths = fork_aprons(
                i,
                &dirs,
                &original_adj,
                &region.terminal_nodes,
                radius,
                terrain,
            )?;
            for (&next, mut remaining) in original_adj[i].iter().zip(lengths) {
                let (mut previous, mut current) = (i, next);
                loop {
                    let length = distance(dirs[previous], dirs[current], radius);
                    let span = apron_spans.entry((previous, current)).or_default();
                    *span = span.max(remaining.min(length));
                    remaining -= length;
                    if remaining <= 1e-6 || flat_nodes[current] || original_adj[current].len() != 2
                    {
                        break;
                    }
                    let Some(&next) = original_adj[current].iter().find(|&&j| j != previous) else {
                        break;
                    };
                    previous = current;
                    current = next;
                }
            }
        }
        let mut segments = Vec::new();
        let mut flat_segments = Vec::new();
        for edge in &region.edges {
            let a = dirs[edge.a];
            let b = dirs[edge.b];
            let length = distance(a, b, radius);
            let start = apron_spans.get(&(edge.a, edge.b)).copied().unwrap_or(0.0);
            let end = apron_spans.get(&(edge.b, edge.a)).copied().unwrap_or(0.0);
            let mut stations = Vec::new();
            if length > start + end {
                let run = length - start - end;
                let count = (run / STEP).ceil() as usize;
                for i in 0..=count {
                    if (i == 0 && start == 0.0) || (i == count && end == 0.0) {
                        continue;
                    }
                    let t = (start + run * i as f64 / count as f64) / length;
                    if t > 0.0 && t < 1.0 {
                        stations.push(t);
                    }
                }
            }
            stations.push(1.0);
            let mut last = edge.a;
            let mut last_t = 0.0;
            for t in stations {
                let next = if t == 1.0 {
                    edge.b
                } else {
                    let j = dirs.len();
                    dirs.push(a.lerp(b, t).normalize());
                    j
                };
                segments.push([last, next]);
                flat_segments.push(
                    length <= start + end + 1e-6
                        || t * length <= start + 1e-6
                        || last_t * length >= length - end - 1e-6,
                );
                last = next;
                last_t = t;
            }
        }
        let mut ground = Vec::with_capacity(dirs.len());
        let mut caps = Vec::with_capacity(dirs.len());
        for &d in &dirs {
            let sample = sample_at(d, radius, terrain)?;
            ground.push(sample.ground);
            caps.push(sample.cap);
        }
        // Envelope edge lengths use a conservative lower radius. Heights
        // cannot fall below the lowest cap minus this anchor allowance, so
        // even a deep undersea tunnel respects the PHYSICAL grade limit.
        let cap_to_target =
            (PIER_CAP - CLEARANCE).max(UNDERSEA_DECK_DEPTH - UNDERSEA_CROWN - UNDERSEA_COVER);
        let minimum_height = (caps.iter().copied().reduce(f64::min).unwrap_or(0.0) - cap_to_target)
            .min(fixed.iter().map(|(_, h)| *h).fold(f64::INFINITY, f64::min));
        let minimum_radius = radius + minimum_height;
        if minimum_radius <= 0.0 {
            return Err("profile reaches the planet centre".into());
        }
        let mut adjacency = vec![Vec::new(); dirs.len()];
        for &[a, b] in &segments {
            let length = distance(dirs[a], dirs[b], minimum_radius);
            adjacency[a].push((b, length));
            adjacency[b].push((a, length));
        }
        // Flat junction/turn aprons stop intersecting deck planes producing
        // steps across a mouth. Mouth-length first stations share the hub height;
        // the global envelope still enforces grade and all terminal anchors.
        for (&[a, b], &flat) in segments.iter().zip(&flat_segments) {
            if flat {
                for (other, length) in &mut adjacency[a] {
                    if *other == b {
                        *length = 0.0;
                    }
                }
                for (other, length) in &mut adjacency[b] {
                    if *other == a {
                        *length = 0.0;
                    }
                }
            }
        }
        let undersea_roofs: Vec<_> = ground
            .iter()
            .zip(&caps)
            .map(|(g, cap)| g.ocean.then_some(cap + UNDERSEA_COVER))
            .collect();
        let smooth_caps = caps.clone();
        let mut anchors = Vec::new();
        let mut low_sources = vec![f64::INFINITY; dirs.len()];
        let mut upper_sources = caps;
        for (terminal, &i) in region.terminals.iter().zip(&region.terminal_nodes) {
            let anchor = terminal_sample(terminal, radius, terrain)?.anchor;
            anchors.push((i, anchor));
            low_sources[i] = -anchor;
            upper_sources[i] = anchor;
        }
        let mut seeded = vec![f64::NAN; dirs.len()];
        for &(i, height) in fixed {
            if i >= region.nodes.len() || !height.is_finite() {
                return Err("invalid fixed road-profile state".into());
            }
            // A fixed state may coincide with a terminal, but cannot change it.
            if low_sources[i].is_finite() && (height + low_sources[i]).abs() > 1e-6 {
                return Err("fixed state moved a village/gateway anchor".into());
            }
            seeded[i] = height;
        }
        let lower: Vec<_> = envelope(low_sources, &adjacency)
            .into_iter()
            .map(|h| -h)
            .collect();
        let upper = envelope(upper_sources, &adjacency);
        for i in 0..dirs.len() {
            if lower[i] > upper[i] + 1e-6 {
                return Ok(Err(Rejected {
                    reason: "alignment needs a longer/alternative graded route",
                    node: i,
                    required_minimum_meters: lower[i],
                    allowed_maximum_meters: upper[i],
                }));
            }
        }
        let desired = ground
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let h = if seeded[i].is_finite() {
                    seeded[i]
                } else if g.ocean {
                    g.height - UNDERSEA_DECK_DEPTH
                } else {
                    g.height + CLEARANCE
                };
                h.max(lower[i]).min(upper[i])
            })
            .collect();
        let mut heights = envelope(desired, &adjacency);
        let links: Vec<_> = segments
            .iter()
            .map(|&[a, b]| {
                let flat = adjacency[a]
                    .iter()
                    .any(|&(j, length)| j == b && length == 0.0);
                (
                    a,
                    b,
                    distance(dirs[a], dirs[b], minimum_radius),
                    if flat { 0.0 } else { GRADE },
                )
            })
            .collect();
        if !crate::road_vertical::smooth(&mut heights, &smooth_caps, &anchors, &links, || {
            terrain.cancelled()
        }) {
            if terrain.cancelled() {
                return Err("road generation superseded".into());
            }
            return Ok(Err(Rejected {
                reason: "vertical-curve constraints rejected alignment or exhausted solver budget",
                node: 0,
                required_minimum_meters: 0.0,
                allowed_maximum_meters: 0.0,
            }));
        }
        let maximum_grade = segments
            .iter()
            .map(|&[a, b]| {
                (heights[a] - heights[b]).abs()
                    / distance(dirs[a], dirs[b], radius + heights[a].min(heights[b])).max(1e-9)
            })
            .fold(0.0, f64::max);
        let mut physical_adjacency = vec![Vec::new(); dirs.len()];
        for &[a, b] in &segments {
            let length = distance(dirs[a], dirs[b], radius + heights[a].min(heights[b]));
            if length > 0.001 {
                physical_adjacency[a].push((b, length));
                physical_adjacency[b].push((a, length));
            }
        }
        let maximum_grade_change_per_meter = physical_adjacency
            .iter()
            .enumerate()
            .filter_map(|(i, adjacent)| {
                let &[(a, la), (b, lb)] = adjacent.as_slice() else {
                    return None;
                };
                Some(
                    ((heights[b] - heights[i]) / lb - (heights[i] - heights[a]) / la).abs()
                        / ((la + lb) * 0.5),
                )
            })
            .fold(0.0, f64::max);
        if maximum_grade > crate::road_route::MAX_GRADE
            || maximum_grade_change_per_meter > crate::road_vertical::MAX_GRADE_CHANGE_PER_METER
        {
            return Ok(Err(Rejected {
                reason: "physical grade/vertical curve verification failed",
                node: 0,
                required_minimum_meters: 0.0,
                allowed_maximum_meters: 0.0,
            }));
        }
        let maximum_deck_above_ground_meters = ground
            .iter()
            .zip(&heights)
            .map(|(g, h)| h - g.height)
            .fold(0.0, f64::max);
        let min_cover = undersea_roofs
            .iter()
            .zip(&heights)
            .filter_map(|(roof, h)| roof.map(|r| r - h))
            .reduce(f64::min);
        let undersea_segments = segments
            .iter()
            .filter(|&&[a, b]| ground[a].ocean || ground[b].ocean)
            .count();
        Ok(Ok(Self {
            heights_meters: heights,
            directions: dirs.into_iter().map(|d| d.to_array()).collect(),
            segments,
            maximum_grade,
            maximum_grade_change_per_meter,
            maximum_deck_above_ground_meters,
            minimum_sampled_undersea_roof_cover_meters: min_cover,
            undersea_segments,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_network::{Ground, Node, RoadEdge, Terminal, TerminalKind};
    use planet_coretypes::{CubeFace, TileKey};

    #[test]
    fn road_network_fork_aprons_follow_curves_but_not_the_incoming_approach() {
        let dirs: Vec<_> = [
            (0.0, 0.0),
            (-100.0, 0.0),
            (20.0, 0.0),
            (100.0, 0.0),
            (300.0, 0.0),
            (20.0, 1.4),
            (39.0, 5.5),
            (57.0, 12.4),
            (200.0, 100.0),
        ]
        .into_iter()
        .map(|(x, y)| direction(x, y))
        .collect();
        let mut adjacent = vec![Vec::new(); dirs.len()];
        for (a, b) in [
            (0, 1),
            (0, 2),
            (2, 3),
            (3, 4),
            (0, 5),
            (5, 6),
            (6, 7),
            (7, 8),
        ] {
            adjacent[a].push(b);
            adjacent[b].push(a);
        }
        let extents = fork_aprons(
            0,
            &dirs,
            &adjacent,
            &[1, 4, 8],
            4e6,
            &Surface {
                steep: false,
                water: false,
            },
        )
        .unwrap();
        assert_eq!(extents[0], APRON_LENGTH);
        for &length in &extents[1..] {
            assert!(
                (40.0..80.0).contains(&length),
                "curved overlap: {extents:?}"
            );
        }
    }
    #[test]
    fn road_network_vertical_curves_keep_acute_junction_overlaps_flat() {
        let mut region = tree();
        let branch = direction(853.553390593, 353.553390593).to_array();
        region.nodes[3].direction = branch;
        region.terminals[2].direction = branch;
        let profile = Profile::fit(
            &region,
            4e6,
            &Surface {
                steep: false,
                water: true,
            },
        )
        .unwrap()
        .unwrap();
        let hub = DVec3::from_array(profile.directions[1]);
        let expected = APRON_LENGTH * (std::f64::consts::PI / 8.0).tan().recip();
        for &[a, b] in &profile.segments {
            if a == 1 || b == 1 {
                let other = if a == 1 { b } else { a };
                let length = distance(hub, DVec3::from_array(profile.directions[other]), 4e6);
                assert!(
                    (length - expected).abs() < 0.01,
                    "apron {length} expected {expected}"
                );
                assert_eq!(profile.heights_meters[1], profile.heights_meters[other]);
            }
        }
    }
    #[test]
    fn road_network_profile_allows_twenty_percent_but_rejects_twenty_six_percent() {
        struct Ramp(f64);
        impl Survey for Ramp {
            fn ground(&self, d: DVec3) -> Result<Ground, String> {
                Ok(Ground {
                    height: 1000.0 + self.0 * d.y / d.x * 4e6,
                    ocean: false,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
        }
        for (grade, accepted) in [(0.20, true), (0.26, false)] {
            let result = Profile::fit(&tree(), 4e6, &Ramp(grade)).unwrap();
            assert_eq!(result.is_ok(), accepted, "physical grade {grade}");
            if let Ok(profile) = result {
                assert!(profile.maximum_grade <= 0.25);
            }
        }
    }

    fn direction(x: f64, y: f64) -> DVec3 {
        DVec3::new(1.0, x / 4e6, y / 4e6).normalize()
    }
    struct Surface {
        steep: bool,
        water: bool,
    }
    impl Survey for Surface {
        fn ground(&self, d: DVec3) -> Result<Ground, String> {
            let x = d.y / d.x * 4e6;
            Ok(Ground {
                height: if self.water {
                    -100.0
                } else if self.steep {
                    x * 0.5
                } else {
                    900.0 + 30.0 * (-((x - 500.0) / 80.0).powi(2)).exp()
                },
                ocean: self.water,
            })
        }
        fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
            Ok(Vec::new())
        }
    }
    fn tree() -> Region {
        let dirs = [
            direction(0.0, 0.0),
            direction(500.0, 0.0),
            direction(1000.0, 0.0),
            direction(500.0, 500.0),
        ];
        Region {
            key: TileKey {
                face: CubeFace::PositiveX,
                level: 8,
                x: 128,
                y: 128,
            },
            village_count: 3,
            terminals: [0, 2, 3]
                .into_iter()
                .map(|i| Terminal {
                    direction: dirs[i].to_array(),
                    kind: TerminalKind::Village,
                })
                .collect(),
            terminal_nodes: vec![0, 2, 3],
            nodes: dirs
                .into_iter()
                .map(|d| Node {
                    direction: d.to_array(),
                    ground_meters: 900.0,
                    ocean: false,
                })
                .collect(),
            edges: [(0, 1), (1, 2), (1, 3)]
                .into_iter()
                .map(|(a, b)| RoadEdge {
                    a,
                    b,
                    requires_undersea_tunnel: false,
                })
                .collect(),
            junction_count: 1,
            length_meters: 1500.0,
            construction_cost_proxy: 1500.0,
            lattice_survey_samples: 0,
        }
    }
    #[test]
    fn road_network_profile_junction_is_shared_and_all_segments_meet_grade() {
        let r = tree();
        let surface = Surface {
            steep: false,
            water: false,
        };
        let p = Profile::fit(&r, 4e6, &surface).unwrap().unwrap();
        assert!(p.maximum_grade <= GRADE + 1e-8);
        assert!(
            p.maximum_grade_change_per_meter <= crate::road_vertical::MAX_GRADE_CHANGE_PER_METER
        );
        assert!(p.maximum_deck_above_ground_meters <= PIER_CAP + 1e-6);
        assert_eq!(p.segments.iter().filter(|s| s.contains(&1)).count(), 3);
        for &i in &r.terminal_nodes {
            assert!(
                (p.heights_meters[i]
                    - surface
                        .ground(direction(
                            if i == 0 {
                                0.0
                            } else if i == 2 {
                                1000.0
                            } else {
                                500.0
                            },
                            0.0
                        ))
                        .unwrap()
                        .height
                    - CLEARANCE)
                    .abs()
                    < 1e-6
            );
        }
    }
    #[test]
    fn road_network_protected_gateway_keeps_the_houses_and_shared_bore_height() {
        struct Town;
        impl Survey for Town {
            fn ground(&self, _: DVec3) -> Result<Ground, String> {
                Ok(Ground {
                    height: 900.0,
                    ocean: false,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(vec![])
            }
            fn settlement_core_at(&self, _: DVec3) -> Result<bool, String> {
                Ok(true)
            }
        }
        let t = Terminal {
            direction: direction(0.0, 0.0).to_array(),
            kind: TerminalKind::Gateway { link: 0 },
        };
        let sample = terminal_sample(&t, 4e6, &Town).unwrap();
        assert_eq!(sample.cap, 880.0);
        assert_eq!(sample.anchor, 874.0);
        assert!(sample.anchor + UNDERSEA_CROWN + UNDERSEA_COVER <= 900.0);
        assert_eq!(
            terminal_sample(&t, 4e6, &Town).unwrap().anchor,
            sample.anchor
        );
    }

    #[test]
    fn road_network_profile_undersea_roof_is_below_the_bed() {
        let p = Profile::fit(
            &tree(),
            4e6,
            &Surface {
                steep: false,
                water: true,
            },
        )
        .unwrap()
        .unwrap();
        assert!(p.minimum_sampled_undersea_roof_cover_meters.unwrap() >= UNDERSEA_COVER);
        assert_eq!(p.undersea_segments, p.segments.len());
        assert!(p.heights_meters.iter().all(|h| *h < -100.0));
    }
    #[test]
    fn road_network_profile_rejects_impossible_grades_instead_of_moving_terminal_heights() {
        let p = Profile::fit(
            &tree(),
            4e6,
            &Surface {
                steep: true,
                water: false,
            },
        )
        .unwrap();
        assert!(p.is_err());
    }
}

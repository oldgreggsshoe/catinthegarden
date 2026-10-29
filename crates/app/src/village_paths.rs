//! Cached desire lines, from real doorways to the resident parking courts.
//! Routing is bounded to one small grid per settlement, never a frame-time search.
use crate::{road_network_field::Frame, village::HouseInstance};
use glam::{DVec2, DVec3};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};

const STEP: f64 = 2.0;
const SIDE: usize = 385;
const HALF: f64 = (SIDE / 2) as f64 * STEP;
const CELL: f64 = 16.0;
const BUCKETS: usize = 8192;
const PROBES: usize = 32;
const MAX_SEGMENTS: usize = 16384;
const MAX_REFS: usize = 131072;
const HEADER: usize = 4 + 1024; // 256-square, 128m coarse occupancy; 16 bits per float.
const REFS: usize = HEADER + BUCKETS;
const SEGMENTS: usize = REFS + MAX_REFS / 4;
pub const FLOAT4S: usize = SEGMENTS + MAX_SEGMENTS * 2;

#[derive(Clone, Copy, Debug)]
pub struct Trail {
    pub a: DVec2,
    pub b: DVec2,
    pub width: f64,
}

fn grid_point(i: usize) -> DVec2 {
    DVec2::new(
        (i % SIDE) as f64 * STEP - HALF,
        (i / SIDE) as f64 * STEP - HALF,
    )
}
fn grid_index(p: DVec2) -> Option<usize> {
    let p = ((p + DVec2::splat(HALF)) / STEP).round();
    (p.min_element() >= 0.0 && p.max_element() < SIDE as f64)
        .then_some(p.y as usize * SIDE + p.x as usize)
}
#[derive(Clone, Copy)]
pub struct Footprint {
    pub centre: DVec2,
    pub forward: DVec2,
}
impl Footprint {
    fn blocks(self, p: DVec2) -> bool {
        let d = p - self.centre;
        d.dot(self.forward).abs() < 4.65 && d.perp_dot(self.forward).abs() < 3.85
    }
    fn door(self) -> DVec2 {
        self.centre - self.forward * 5.8
    }
}

/// Existing footfall is cheaper, but never enough to justify a large detour.
/// The eight-neighbour search cannot cut diagonally through building corners.
#[cfg(test)]
fn route(
    houses: &[Footprint],
    destination: DVec2,
    height: impl Fn(DVec2) -> Option<f64>,
) -> Vec<Trail> {
    route_cancellable(houses, destination, height, &|| false)
}
fn route_cancellable(
    houses: &[Footprint],
    destination: DVec2,
    height: impl Fn(DVec2) -> Option<f64>,
    cancelled: &impl Fn() -> bool,
) -> Vec<Trail> {
    let Some(goal) = grid_index(destination) else {
        return Vec::new();
    };
    let mut blocked = vec![false; SIDE * SIDE];
    for h in houses {
        let lo = ((h.centre - DVec2::splat(7.0) + DVec2::splat(HALF)) / STEP)
            .floor()
            .max(DVec2::ZERO);
        let hi = ((h.centre + DVec2::splat(7.0) + DVec2::splat(HALF)) / STEP)
            .ceil()
            .min(DVec2::splat((SIDE - 1) as f64));
        for y in lo.y as usize..=hi.y as usize {
            for x in lo.x as usize..=hi.x as usize {
                let i = y * SIDE + x;
                blocked[i] |= h.blocks(grid_point(i));
            }
        }
    }
    let mut heights = vec![f64::NAN; SIDE * SIDE];
    let mut uses = vec![0u16; SIDE * SIDE];
    let mut edges = BTreeMap::<(usize, usize), u16>::new();
    let mut connectors = Vec::new();
    // Outlying houses first establish the common lanes; nearer doors join them.
    let mut order: Vec<_> = houses.iter().collect();
    order.sort_by(|a, b| {
        b.centre
            .distance_squared(destination)
            .total_cmp(&a.centre.distance_squared(destination))
    });
    for house in order {
        let door = house.door();
        let Some(rounded_start) = grid_index(door) else {
            continue;
        };
        // A rotated doorway must not be lost because the nearest grid centre
        // happens to round into its own wall or porch. Find a clear short step.
        let start = (-2isize..=2)
            .flat_map(|dy| (-2isize..=2).map(move |dx| (dx, dy)))
            .filter_map(|(dx, dy)| {
                let x = (rounded_start % SIDE) as isize + dx;
                let y = (rounded_start / SIDE) as isize + dy;
                if x < 0 || y < 0 || x >= SIDE as isize || y >= SIDE as isize {
                    return None;
                }
                let j = y as usize * SIDE + x as usize;
                let p = grid_point(j);
                (!blocked[j]
                    && !(0..=8).any(|k| {
                        houses
                            .iter()
                            .any(|h| h.blocks(door.lerp(p, k as f64 / 8.0)))
                    }))
                .then_some(j)
            })
            .min_by(|&a, &b| {
                grid_point(a)
                    .distance_squared(door)
                    .total_cmp(&grid_point(b).distance_squared(door))
            });
        let Some(start) = start else {
            continue;
        };
        if blocked[goal] {
            continue;
        }
        let mut costs = vec![u32::MAX; SIDE * SIDE];
        let mut parents = vec![usize::MAX; SIDE * SIDE];
        let mut queue = BinaryHeap::new();
        costs[start] = 0;
        queue.push(Reverse((0u32, start)));
        let mut visits = 0;
        while let Some(Reverse((priority, i))) = queue.pop() {
            visits += 1;
            if visits % 256 == 1 && cancelled() {
                return Vec::new();
            }
            let heuristic = |j| (grid_point(j).distance(destination) * 500.0) as u32;
            if priority != costs[i].saturating_add(heuristic(i)) && i != start {
                continue;
            }
            if i == goal {
                break;
            }
            if heights[i].is_nan() {
                heights[i] = height(grid_point(i)).unwrap_or(f64::INFINITY);
            }
            if !heights[i].is_finite() {
                continue;
            }
            let x = i % SIDE;
            let y = i / SIDE;
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x as isize + dx;
                    let ny = y as isize + dy;
                    if nx < 0 || ny < 0 || nx >= SIDE as isize || ny >= SIDE as isize {
                        continue;
                    }
                    let j = ny as usize * SIDE + nx as usize;
                    if blocked[j]
                        || blocked[y * SIDE + nx as usize]
                        || blocked[ny as usize * SIDE + x]
                    {
                        continue;
                    }
                    if heights[j].is_nan() {
                        heights[j] = height(grid_point(j)).unwrap_or(f64::INFINITY);
                    }
                    let length = grid_point(i).distance(grid_point(j));
                    let grade = (heights[j] - heights[i]).abs() / length;
                    if !grade.is_finite() || grade > 1.25 {
                        continue;
                    }
                    let worn = if uses[j] > 0 { 0.55 } else { 1.0 };
                    // Very low-amplitude fixed ground variation breaks grid ties,
                    // without arbitrary decorative wiggles in a pedestrian route.
                    let rough = 1.0 + 0.06 * ((nx as f64 * 0.37).sin() * (ny as f64 * 0.29).cos());
                    let cost = costs[i].saturating_add(
                        (length * 1000.0 * worn * rough * (1.0 + 10.0 * grade * grade)) as u32,
                    );
                    if cost < costs[j] {
                        costs[j] = cost;
                        parents[j] = i;
                        queue.push(Reverse((cost.saturating_add(heuristic(j)), j)));
                    }
                }
            }
        }
        if start != goal && parents[goal] == usize::MAX {
            continue;
        }
        let mut i = goal;
        while i != start {
            uses[i] = uses[i].saturating_add(1);
            let j = parents[i];
            *edges.entry((i.min(j), i.max(j))).or_default() += 1;
            i = j;
        }
        uses[start] = uses[start].saturating_add(1);
        connectors.push(Trail {
            a: house.centre - house.forward * 4.15,
            b: grid_point(start),
            width: 0.65,
        });
    }
    if uses[goal] > 0 && grid_point(goal).distance_squared(destination) > 0.0001 {
        // Finish on the actual court edge, not the nearest two-metre node.
        connectors.push(Trail {
            a: grid_point(goal),
            b: destination,
            width: (0.6 + f64::from(uses[goal]).sqrt() * 0.16).min(1.4),
        });
    }
    // Coalesce collinear grid edges before GPU indexing: a shared lane is a
    // handful of capsules, not hundreds of two-metre distance tests.
    let mut graph = BTreeMap::<usize, Vec<(usize, u16)>>::new();
    for (&(a, b), &traffic) in &edges {
        graph.entry(a).or_default().push((b, traffic));
        graph.entry(b).or_default().push((a, traffic));
    }
    let mut visited = std::collections::BTreeSet::new();
    for (&a, neighbours) in &graph {
        for &(b, traffic) in neighbours {
            if visited.contains(&(a.min(b), a.max(b))) {
                continue;
            }
            let mut previous = a;
            let mut end = b;
            let delta = (grid_point(b) - grid_point(a)).normalize();
            visited.insert((a.min(b), a.max(b)));
            while let Some(nexts) = graph.get(&end) {
                if nexts.len() != 2 {
                    break;
                }
                let &(next, next_traffic) = nexts.iter().find(|&&(n, _)| n != previous).unwrap();
                let edge = (end.min(next), end.max(next));
                if next_traffic != traffic
                    || visited.contains(&edge)
                    || (grid_point(next) - grid_point(end))
                        .normalize()
                        .distance(delta)
                        > 0.001
                {
                    break;
                }
                visited.insert(edge);
                previous = end;
                end = next;
            }
            connectors.push(Trail {
                a: grid_point(a),
                b: grid_point(end),
                width: (0.6 + f64::from(traffic).sqrt() * 0.16).min(1.4),
            });
        }
    }
    connectors
}

pub fn footprints(instances: &[HouseInstance], worlds: &[DVec3], frame: &Frame) -> Vec<Footprint> {
    instances
        .iter()
        .zip(worlds)
        .map(|(h, &p)| {
            let f = DVec3::from(h.forward.map(f64::from));
            Footprint {
                centre: frame.local(p.normalize() * frame.radius).truncate(),
                forward: DVec2::new(f.dot(frame.across), f.dot(frame.forward)).normalize(),
            }
        })
        .collect()
}
fn hash(x: i32, y: i32) -> usize {
    ((x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663)) as usize & (BUCKETS - 1)
}
/// One bounded sparse index. Failure publishes no partial path network.
pub fn pack(frame: &Frame, trails: &[Trail]) -> Result<Vec<[f32; 4]>, String> {
    if trails.len() > MAX_SEGMENTS {
        return Err("village path segment budget".into());
    }
    if trails.iter().any(|t| {
        !t.a.is_finite()
            || !t.b.is_finite()
            || !t.width.is_finite()
            || !(0.2..=3.0).contains(&t.width)
    }) {
        return Err("invalid village path capsule".into());
    }
    let mut cells = BTreeMap::<(i32, i32), Vec<usize>>::new();
    for (i, t) in trails.iter().enumerate() {
        let lo = ((t.a.min(t.b) - DVec2::splat(t.width + 0.5)) / CELL).floor();
        let hi = ((t.a.max(t.b) + DVec2::splat(t.width + 0.5)) / CELL).floor();
        for y in lo.y as i32..=hi.y as i32 {
            for x in lo.x as i32..=hi.x as i32 {
                cells.entry((x, y)).or_default().push(i);
            }
        }
    }
    if cells.len() > BUCKETS / 2
        || cells.values().any(|v| v.len() > 128)
        || cells.values().map(Vec::len).sum::<usize>() > MAX_REFS
    {
        return Err("village path index budget".into());
    }
    let mut data = vec![[0.0; 4]; FLOAT4S];
    data[0] = [
        frame.across.x as f32,
        frame.across.y as f32,
        frame.across.z as f32,
        trails.len() as f32,
    ];
    data[1] = [
        frame.forward.x as f32,
        frame.forward.y as f32,
        frame.forward.z as f32,
        frame.radius as f32,
    ];
    data[3] = [frame.up.x as f32, frame.up.y as f32, frame.up.z as f32, 0.0];
    let mut count = 0;
    for ((x, y), refs) in cells {
        let coarse_x = x.div_euclid(8) + 128;
        let coarse_y = y.div_euclid(8) + 128;
        if !(0..256).contains(&coarse_x) || !(0..256).contains(&coarse_y) {
            return Err("village path field bounds".into());
        }
        let bit = (coarse_y * 256 + coarse_x) as usize;
        let word = bit / 16;
        data[4 + word / 4][word % 4] =
            ((data[4 + word / 4][word % 4] as u32) | (1u32 << (bit % 16))) as f32;
        let slot = (0..PROBES)
            .map(|p| HEADER + ((hash(x, y) + p) & (BUCKETS - 1)))
            .find(|&p| data[p][3] == 0.0)
            .ok_or("village path hash budget")?;
        data[slot] = [x as f32, y as f32, count as f32, refs.len() as f32];
        for r in refs {
            data[REFS + count / 4][count % 4] = r as f32;
            count += 1;
        }
    }
    for (i, t) in trails.iter().enumerate() {
        data[SEGMENTS + 2 * i] = [t.a.x as f32, t.a.y as f32, t.b.x as f32, t.b.y as f32];
        data[SEGMENTS + 2 * i + 1] = [t.width as f32, 0.0, 0.0, 0.0];
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desire_lines_reach_parking_share_lanes_and_avoid_houses() {
        let houses = [
            Footprint {
                centre: DVec2::new(-25.0, 0.0),
                forward: DVec2::Y,
            },
            Footprint {
                centre: DVec2::new(0.0, 0.0),
                forward: DVec2::Y,
            },
            Footprint {
                centre: DVec2::new(25.0, 0.0),
                forward: DVec2::Y,
            },
        ];
        let goal = DVec2::new(0.0, 80.0);
        let trails = route(&houses, goal, |_| Some(0.0));
        assert!(
            trails
                .iter()
                .any(|t| t.a.distance(goal) < STEP || t.b.distance(goal) < STEP)
        );
        for h in houses {
            assert!(
                trails
                    .iter()
                    .any(|t| t.a.distance(h.centre - h.forward * 4.15) < 0.01)
            );
        }
        for t in &trails {
            for k in 0..=8 {
                let p = t.a.lerp(t.b, k as f64 / 8.0);
                assert!(
                    !houses.iter().any(|h| {
                        let d = p - h.centre;
                        d.dot(h.forward).abs() < 4.05 && d.perp_dot(h.forward).abs() < 3.25
                    }),
                    "{p:?}"
                );
            }
        }
        assert!(
            trails.iter().any(|t| t.width > 0.85),
            "shared traffic widens a lane"
        );
        assert_eq!(trails.len(), route(&houses, goal, |_| Some(0.0)).len());
    }
    #[test]
    fn paths_do_not_cross_water_or_impassable_slopes() {
        let houses = [Footprint {
            centre: DVec2::ZERO,
            forward: DVec2::Y,
        }];
        assert!(
            route(&houses, DVec2::new(0.0, 40.0), |p| if p.y > 10.0 {
                None
            } else {
                Some(0.0)
            })
            .is_empty()
        );
    }
    #[test]
    fn rotated_doorways_do_not_disappear_when_the_grid_rounds_into_a_wall() {
        for step in 0..32 {
            let angle = step as f64 * std::f64::consts::TAU / 32.0;
            let h = Footprint {
                centre: DVec2::new(0.7, 0.9),
                forward: DVec2::new(angle.cos(), angle.sin()),
            };
            let paths = route(&[h], DVec2::new(35.0, 50.0), |_| Some(0.0));
            assert!(
                paths
                    .iter()
                    .any(|t| t.a.distance(h.centre - h.forward * 4.15) < 0.01),
                "rotation {angle}"
            );
        }
    }

    #[test]
    fn superseded_routes_stop_without_publishing_a_partial_village() {
        let houses = [Footprint {
            centre: DVec2::ZERO,
            forward: DVec2::Y,
        }];
        assert!(route_cancellable(&houses, DVec2::splat(80.0), |_| Some(0.0), &|| true).is_empty());
    }

    #[test]
    fn parking_access_goes_round_the_steep_bank_instead_of_up_its_face() {
        let court = DVec2::new(200.0, 0.0);
        let entry = court_entry(court, |p| {
            Some(if p.x < 185.0 && p.y.abs() < 25.0 {
                12.0
            } else {
                0.0
            })
        });
        assert!(entry.y.abs() > 10.0);
        assert!((entry.distance(court) - (crate::road_parking::RADIUS - 0.5)).abs() < 0.001);
    }

    #[test]
    fn sparse_index_is_bounded_and_carries_only_local_coordinates() {
        let f = Frame::new(DVec3::X, 4_000_000.0).unwrap();
        let data = pack(
            &f,
            &[Trail {
                a: DVec2::ZERO,
                b: DVec2::splat(10.0),
                width: 1.0,
            }],
        )
        .unwrap();
        assert_eq!(data.len(), FLOAT4S);
        assert_eq!(data[0][3], 1.0);
        assert!(
            pack(
                &f,
                &vec![
                    Trail {
                        a: DVec2::ZERO,
                        b: DVec2::ONE,
                        width: 1.0
                    };
                    MAX_SEGMENTS + 1
                ]
            )
            .is_err()
        );
    }
}

/// Prefer a level way onto the village-facing half of a court, rather than
/// painting the shortest line straight up its steep excavated bank.
fn court_entry(court: DVec2, height: impl Fn(DVec2) -> Option<f64>) -> DVec2 {
    let toward_village = -court.normalize_or_zero();
    let fallback = court + toward_village * (crate::road_parking::RADIUS - 0.5);
    let Some(centre_height) = height(court) else {
        return fallback;
    };
    (0..=16)
        .filter_map(|i| {
            let angle = (i as f64 / 16.0 - 0.5) * std::f64::consts::PI;
            let outward = toward_village * angle.cos()
                + DVec2::new(-toward_village.y, toward_village.x) * angle.sin();
            let end = court + outward * (crate::road_parking::RADIUS - 0.5);
            let near = height(court + outward * 30.0)?;
            let far = height(court + outward * 46.0)?;
            let relief = (near - centre_height)
                .abs()
                .max((far - centre_height).abs());
            Some((end.length() * 0.2 + relief * relief * 0.7, end))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(fallback, |(_, end)| end)
}

/// Plan in a stable per-village frame, then index in one nearby field frame.
/// Drawn height and camera motion do not change the horizontal desire lines.
pub fn build(
    tiles: &std::collections::HashMap<planet_coretypes::TileKey, crate::outmap::TileData>,
    dense_level: u8,
    courts: &[(DVec3, DVec3)],
    instances: &[HouseInstance],
    worlds: &[DVec3],
    sites: &[DVec3],
    camera: DVec3,
    cancelled: &impl Fn() -> bool,
) -> Result<(Frame, Vec<[f32; 4]>, crate::road_signs::Mesh), String> {
    let frame = Frame::new(camera.normalize(), crate::planet::planet_radius_meters())?;
    let mut groups = BTreeMap::<[u64; 3], Vec<usize>>::new();
    for (i, site) in sites.iter().enumerate() {
        groups
            .entry(site.to_array().map(f64::to_bits))
            .or_default()
            .push(i);
    }
    let mut trails = Vec::new();
    let mut gates = Vec::new();
    let mut connected = 0;
    let mut doors = 0;
    for indices in groups.values() {
        if cancelled() {
            return Err("village paths superseded".into());
        }
        let site = sites[indices[0]];
        let local = Frame::new(site, frame.radius)?;
        let hs: Vec<_> = indices.iter().map(|&i| instances[i]).collect();
        let ws: Vec<_> = indices.iter().map(|&i| worlds[i]).collect();
        let houses = footprints(&hs, &ws, &local);
        let court_entry_of = courts
            .iter()
            .find(|(village, _)| village.distance(site) * frame.radius < 0.5);
        let court = court_entry_of.map(|(_, court)| court);
        let mut goal = court.map_or(DVec2::ZERO, |&c| {
            let p = local.local(c.normalize() * frame.radius).truncate();
            court_entry(p, |p| {
                crate::terrain::village_path_ground(
                    local.world(p.extend(0.0)).normalize(),
                    dense_level,
                    tiles,
                )
            })
        });
        // Mark where the path leaves the court: a gateway over the mouth,
        // standing on the court floor and facing back across it.
        if let Some((village, court)) = court_entry_of {
            let mouth = local.world(goal.extend(0.0)).normalize() * court.length();
            let outward = mouth - *court;
            if outward.length() > 1.0 && !houses.is_empty() {
                gates.push(crate::road_navigation::footpath_gate(
                    mouth,
                    outward,
                    &crate::road_navigation::village_name(*village),
                ));
            }
        }
        if court.is_none() && houses.iter().any(|h| h.blocks(goal)) {
            goal = (0..64)
                .map(|i| {
                    let angle = i as f64 * 2.399963;
                    DVec2::new(angle.cos(), angle.sin()) * (8.0 + (i / 8) as f64 * 4.0)
                })
                .find(|&p| !houses.iter().any(|h| h.blocks(p)))
                .unwrap_or(goal);
        }
        let routed = route_cancellable(
            &houses,
            goal,
            |p| {
                let direction = local.world(p.extend(0.0)).normalize();
                crate::terrain::village_path_ground(direction, dense_level, tiles)
            },
            cancelled,
        );
        doors += houses.len();
        connected += houses
            .iter()
            .filter(|h| {
                routed
                    .iter()
                    .any(|t| t.a.distance(h.centre - h.forward * 4.15) < 0.01)
            })
            .count();
        let project = |p: DVec2| {
            frame
                .local(local.world(p.extend(0.0)).normalize() * frame.radius)
                .truncate()
        };
        trails.extend(routed.into_iter().map(|t| Trail {
            a: project(t.a),
            b: project(t.b),
            width: t.width,
        }));
    }
    if cancelled() {
        return Err("village paths superseded".into());
    }
    let data = pack(&frame, &trails)?;
    // A failed sign bake keeps the paths; it only loses the gateways.
    let gate_mesh = crate::road_signs::bake(&gates, frame).unwrap_or_else(|error| {
        tracing::warn!(target:"planet::villages", %error, "footpath gateways rejected");
        crate::road_signs::Mesh::empty()
    });
    tracing::info!(target:"planet::villages", villages=groups.len(), segments=trails.len(), connected_doors=connected, doors=doors, gateways=gates.len(), "village desire lines built");
    Ok((frame, data, gate_mesh))
}

struct Request {
    generation: u64,
    tiles: std::collections::HashMap<planet_coretypes::TileKey, crate::outmap::TileData>,
    dense_level: u8,
    courts: Vec<(DVec3, DVec3)>,
    houses: Vec<HouseInstance>,
    worlds: Vec<DVec3>,
    sites: Vec<DVec3>,
    camera: DVec3,
}
type ResultData = Result<(Frame, Vec<[f32; 4]>, crate::road_signs::Mesh), String>;
#[derive(Default)]
struct Pending {
    request: Option<Request>,
    result: Option<(u64, ResultData)>,
    stop: bool,
}
pub struct Worker {
    shared: std::sync::Arc<(std::sync::Mutex<Pending>, std::sync::Condvar)>,
    generation: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        let shared = std::sync::Arc::new((
            std::sync::Mutex::new(Pending::default()),
            std::sync::Condvar::new(),
        ));
        let state = shared.clone();
        let body = crate::body::active();
        let thread = std::thread::Builder::new()
            .name("village-paths".into())
            .spawn(move || {
                crate::body::with_body(body, || {
                    loop {
                        let request = {
                            let (lock, wake) = &*state;
                            let mut pending = lock.lock().unwrap();
                            while pending.request.is_none() && !pending.stop {
                                pending = wake.wait(pending).unwrap();
                            }
                            if pending.stop {
                                break;
                            }
                            pending.request.take().unwrap()
                        };
                        let result = build(
                            &request.tiles,
                            request.dense_level,
                            &request.courts,
                            &request.houses,
                            &request.worlds,
                            &request.sites,
                            request.camera,
                            &|| {
                                let pending = state.0.lock().unwrap();
                                pending.stop
                                    || pending
                                        .request
                                        .as_ref()
                                        .is_some_and(|r| r.generation > request.generation)
                            },
                        );
                        state.0.lock().unwrap().result = Some((request.generation, result));
                    }
                })
            })
            .expect("village path worker");
        Self {
            shared,
            generation: 0,
            thread: Some(thread),
        }
    }
    pub fn request(
        &mut self,
        terrain: &crate::terrain::TerrainRenderer,
        houses: &[HouseInstance],
        worlds: &[DVec3],
        sites: &[DVec3],
        camera: DVec3,
    ) {
        // Diagnostic A/B override; normal villages always retain their paths.
        let disabled = matches!(
            std::env::var("PLANET_VILLAGE_PATHS").as_deref(),
            Ok("0" | "off")
        );
        let (houses, worlds, sites) = if disabled {
            (&[][..], &[][..], &[][..])
        } else {
            (houses, worlds, sites)
        };
        let (dense_level, tiles) = terrain.village_path_source();
        self.generation += 1;
        self.shared.0.lock().unwrap().request = Some(Request {
            generation: self.generation,
            tiles,
            dense_level,
            courts: terrain.village_parking_areas().to_vec(),
            houses: houses.to_vec(),
            worlds: worlds.to_vec(),
            sites: sites.to_vec(),
            camera,
        });
        self.shared.1.notify_one();
    }
    pub fn take(&self) -> Option<ResultData> {
        self.shared
            .0
            .lock()
            .unwrap()
            .result
            .take()
            .and_then(|(g, r)| (g == self.generation).then_some(r))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.0.lock().unwrap().stop = true;
        self.shared.1.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Match the already engineered access (not the nearest village) to its
/// existing court. Kept outside fingerprinted engineering code: cosmetic
/// footpaths must not discard a planetary precompute or alter its geometry.
pub fn attach_courts(
    snapshot: &mut crate::road_network_mesh::Snapshot,
    plans: &[std::sync::Arc<crate::road_network_mesh::Prepared>],
) {
    let centre = snapshot.build_centre;
    let field = std::sync::Arc::get_mut(&mut snapshot.field)
        .expect("unpublished road field is uniquely owned");
    let eye = field.frame.local(centre);
    let mut matched = 0;
    for plan in plans {
        let point = |i: usize| {
            field.frame.local(
                DVec3::from_array(plan.profile.directions[i])
                    * (field.frame.radius + plan.profile.heights_meters[i]),
            )
        };
        for ((_, &node), &village) in plan
            .region
            .terminals
            .iter()
            .zip(&plan.region.terminal_nodes)
            .filter(|(t, _)| matches!(t.kind, crate::road_network::TerminalKind::Village))
            .zip(&plan.village_centres)
        {
            let access = point(node);
            // Same first non-coincident profile arm used by the court mesh.
            let neighbour = plan.profile.segments.iter().find_map(|&[a, b]| {
                let j = if a == node {
                    b
                } else if b == node {
                    a
                } else {
                    return None;
                };
                ((point(j) - access).truncate().length() > 0.001).then_some(j)
            });
            let Some(neighbour) = neighbour else {
                continue;
            };
            let outward = (access - point(neighbour))
                .truncate()
                .normalize()
                .extend(0.0);
            let court = access + outward * crate::road_parking::OFFSET;
            if (court - eye).truncate().length() >= crate::road_network_mesh::FAR_RADIUS {
                continue;
            }
            if field
                .segments
                .iter()
                .any(|s| s.is_parking() && s.a.distance(court) < 0.01)
            {
                matched += 1;
            }
            // Far courts are drawn too, even outside the near excavation index.
            field
                .village_parking
                .push((village, field.frame.world(court)));
        }
    }
    let courts = field.segments.iter().filter(|s| s.is_parking()).count();
    tracing::info!(target:"planet::villages",indexed_courts=courts,matched_indexed_courts=matched,destinations=field.village_parking.len(),"village parking destinations attached");
}

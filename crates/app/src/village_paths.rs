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
// Ground-shape atlas: one baked tile per occupied cell, so the vertex shader
// reads four samples instead of searching the cell's segments. The tile index
// holds each hash slot's tile number.
const MAX_TILES: usize = BUCKETS / 2;
const TILE_INDEX: usize = SEGMENTS + MAX_SEGMENTS * 2;
const ATLAS: usize = TILE_INDEX + BUCKETS / 4;
/// Samples along a tile side: 1 m apart, both cell edges included.
const TILE_SIDE: usize = 17;
const TILE_SAMPLE_SPACING: f64 = CELL / (TILE_SIDE - 1) as f64;
/// A header (court floor, mouth x/y, has mouth) then one packed float per sample.
const TILE_FLOAT4S: usize = 1 + (TILE_SIDE * TILE_SIDE).div_ceil(4);
/// Bank distances are quantised to 12 bits each, two to a float (exact below
/// 2^24), over this range. Kept signed inside the path and past the bank so
/// the clamps fall where the weight is flat and blending stays accurate.
const BANK_LEVELS: f64 = 4095.0;
const BANK_STORED_MIN: f64 = -3.0;
const BANK_STORED_MAX: f64 = 2.0;
pub const FLOAT4S: usize = ATLAS + MAX_TILES * TILE_FLOAT4S;

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

fn civic_clearing(h: Footprint) -> [Trail; 2] {
    // Overlapping normal-width lanes clear a broad forecourt without changing
    // the atlas's established maximum capsule width or bank-distance range.
    [-1.1, 1.1].map(|side| {
        let p = h.centre + h.forward.perp() * side;
        Trail { a: p - h.forward * 2.5, b: p + h.forward * 2.5, width: 2.8 }
    })
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
/// Ground shaping under the paths; the same numbers as village_paths.wgsl.
pub const BANK_METERS: f64 = 1.2;
pub const RAMP_FULL_METERS: f64 = 3.0;
pub const RAMP_END_METERS: f64 = 15.0;
pub const SMOOTH_METERS: f64 = 12.0;
pub const SMOOTHING: f64 = 0.8;
pub const MAX_CHANGE_METERS: f64 = 0.8;
pub const RAMP_MAX_CHANGE_METERS: f64 = 3.2;
pub const FADE_FILTER_START_METERS: f64 = 2.0;
pub const FADE_FILTER_END_METERS: f64 = 4.0;

/// Shaping strength by the vertex's detail filter (1% of its distance): full
/// close by, gone once the mesh is too coarse to carry a path.
pub fn distance_fade(detail_filter_meters: f64) -> f64 {
    1.0 - smoothstep(FADE_FILTER_START_METERS, FADE_FILTER_END_METERS, detail_filter_meters)
}

/// The shape scaled by `distance_fade`, as the vertex shader applies it.
pub fn faded(shape: GroundShape, fade: f64) -> GroundShape {
    GroundShape {
        weight: shape.weight * fade,
        ramp: shape.ramp * fade,
        floor_meters: shape.floor_meters,
        near_mouth: shape.near_mouth * fade,
    }
}

/// Mouth coordinate for a path with no court: far enough that no ramp reaches.
const NO_MOUTH: f32 = 1.0e9;

/// How strongly the ground at a path-frame point belongs to a path (`weight`)
/// and runs to its court's floor (`ramp`, toward `floor_meters`); `near_mouth`
/// is the ramp's distance factor, which also widens the allowed change.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroundShape {
    pub weight: f64,
    pub ramp: f64,
    pub floor_meters: f64,
    pub near_mouth: f64,
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn shaping_enabled() -> bool {
    !matches!(
        std::env::var("PLANET_VILLAGE_PATH_SHAPING").as_deref(),
        Ok("0" | "off")
    )
}

/// How far into its bank a point is from a capsule: 0 at the path's edge,
/// 1 at the bank's outer edge, negative on the path. The shaping weight is
/// `1 - smoothstep(0, 1, ...)` of it.
fn bank_distance(p: DVec2, a: DVec2, b: DVec2, width: f64) -> f64 {
    let delta = b - a;
    let t = ((p - a).dot(delta) / delta.length_squared().max(0.001)).clamp(0.0, 1.0);
    ((p.distance(a.lerp(b, t)) - width) / BANK_METERS).clamp(BANK_STORED_MIN, BANK_STORED_MAX)
}

fn quantise_bank(distance: f64) -> f64 {
    ((distance - BANK_STORED_MIN) / (BANK_STORED_MAX - BANK_STORED_MIN) * BANK_LEVELS).round()
}

/// The tile slot and tile number of the cell holding `p`, if it has one.
fn tile_of(data: &[[f32; 4]], p: DVec2) -> Option<(DVec2, usize)> {
    let coarse = (p / 128.0).floor() + DVec2::splat(128.0);
    if coarse.min_element() < 0.0 || coarse.max_element() >= 256.0 {
        return None;
    }
    let bit = (coarse.y as usize) * 256 + coarse.x as usize;
    let word = bit / 16;
    if (data[4 + word / 4][word % 4] as u32) & (1 << (bit % 16)) == 0 {
        return None;
    }
    let cell = (p / CELL).floor();
    let (x, y) = (cell.x as i32, cell.y as i32);
    for probe in 0..PROBES {
        let slot = (hash(x, y) + probe) & (BUCKETS - 1);
        let bucket = data[HEADER + slot];
        if bucket[3] < 0.5 {
            return None;
        }
        if bucket[0] as i32 == x && bucket[1] as i32 == y {
            return Some((cell * CELL, data[TILE_INDEX + slot / 4][slot % 4] as usize));
        }
    }
    None
}

/// The 16 m cells that have a shape tile, empty when shaping is off. The
/// terrain flags chunks touching one (`touches_cells`) so the rest skip the
/// shaping lookup in the vertex stage.
pub fn shaped_cells(data: &[[f32; 4]]) -> std::collections::HashSet<(i32, i32)> {
    if data.len() < FLOAT4S || data[0][3] < 0.5 || data[3][3] < 0.5 {
        return Default::default();
    }
    data[HEADER..HEADER + BUCKETS]
        .iter()
        .filter(|bucket| bucket[3] >= 0.5)
        .map(|bucket| (bucket[0] as i32, bucket[1] as i32))
        .collect()
}

/// Whether a path-frame box overlaps any of `cells` (closed cells: a box
/// touching a cell's edge counts).
pub fn touches_cells(cells: &std::collections::HashSet<(i32, i32)>, min: DVec2, max: DVec2) -> bool {
    if cells.is_empty() {
        return false;
    }
    let lo = (min / CELL).floor();
    let hi = (max / CELL).floor();
    let span = (hi - lo + DVec2::ONE).max(DVec2::ZERO);
    if span.x * span.y <= cells.len() as f64 {
        (lo.y as i32..=hi.y as i32).any(|y| (lo.x as i32..=hi.x as i32).any(|x| cells.contains(&(x, y))))
    } else {
        cells.iter().any(|&(x, y)| {
            (lo.x as i32..=hi.x as i32).contains(&x) && (lo.y as i32..=hi.y as i32).contains(&y)
        })
    }
}

/// CPU twin of `village_path_ground_shape`: same index walk, same tile
/// samples, same bilinear blend.
pub fn ground_shape(data: &[[f32; 4]], p: DVec2) -> GroundShape {
    let mut shape = GroundShape::default();
    if data.len() < FLOAT4S || data[0][3] < 0.5 || data[3][3] < 0.5 {
        return shape;
    }
    let Some((origin, tile)) = tile_of(data, p) else {
        return shape;
    };
    let base = ATLAS + tile * TILE_FLOAT4S;
    let local = ((p - origin) / TILE_SAMPLE_SPACING).clamp(DVec2::ZERO, DVec2::splat((TILE_SIDE - 1) as f64));
    let i = local.floor().min(DVec2::splat((TILE_SIDE - 2) as f64));
    let f = local - i;
    let sample = |x: usize, y: usize| {
        let index = y * TILE_SIDE + x;
        let packed = data[base + 1 + index / 4][index % 4] as u32;
        DVec2::new((packed & 4095) as f64, (packed >> 12) as f64) / BANK_LEVELS
            * (BANK_STORED_MAX - BANK_STORED_MIN)
            + DVec2::splat(BANK_STORED_MIN)
    };
    let (x, y) = (i.x as usize, i.y as usize);
    let bank = sample(x, y)
        .lerp(sample(x + 1, y), f.x)
        .lerp(sample(x, y + 1).lerp(sample(x + 1, y + 1), f.x), f.y);
    shape.weight = 1.0 - smoothstep(0.0, 1.0, bank.x);
    let header = data[base].map(f64::from);
    let ramp_weight = 1.0 - smoothstep(0.0, 1.0, bank.y);
    if header[3] > 0.5 && ramp_weight > 0.0 {
        let near_mouth = 1.0
            - smoothstep(RAMP_FULL_METERS, RAMP_END_METERS, p.distance(DVec2::new(header[1], header[2])));
        shape.ramp = ramp_weight * near_mouth;
        if shape.ramp > 0.0 {
            shape.floor_meters = header[0];
            shape.near_mouth = near_mouth;
        }
    }
    shape
}

/// One cell's tile: header then packed (bank distance to any path, to any
/// path with a court mouth) at each sample. A cell takes the first court
/// mouth among its segments; cells near two courts do not occur (courts belong
/// to separate villages).
fn bake_tile(
    origin: DVec2,
    refs: &[usize],
    trails: &[Trail],
    mouths: &[Option<(DVec2, f64)>],
) -> [[f32; 4]; TILE_FLOAT4S] {
    let mut tile = [[0.0; 4]; TILE_FLOAT4S];
    let mouth = refs.iter().find_map(|&r| mouths.get(r).copied().flatten());
    tile[0] = match mouth {
        Some((m, floor)) => [floor as f32, m.x as f32, m.y as f32, 1.0],
        None => [0.0, NO_MOUTH, NO_MOUTH, 0.0],
    };
    for y in 0..TILE_SIDE {
        for x in 0..TILE_SIDE {
            let q = origin + DVec2::new(x as f64, y as f64) * TILE_SAMPLE_SPACING;
            let (mut any, mut ramp) = (BANK_STORED_MAX, BANK_STORED_MAX);
            for &r in refs {
                let t = trails[r];
                let d = bank_distance(q, t.a, t.b, t.width);
                any = any.min(d);
                if mouths.get(r).copied().flatten().is_some() {
                    ramp = ramp.min(d);
                }
            }
            let index = y * TILE_SIDE + x;
            tile[1 + index / 4][index % 4] =
                (quantise_bank(any) + quantise_bank(ramp) * 4096.0) as f32;
        }
    }
    tile
}

/// The shaped ground height from the full drawn height and the same height
/// with relief under `SMOOTH_METERS` filtered out (both before road cuts).
pub fn shaped_height(full: f64, coarse: f64, shape: GroundShape) -> f64 {
    if shape.weight <= 0.0 {
        return full;
    }
    let keep = 1.0 - SMOOTHING * shape.weight;
    let smoothed = coarse + keep * (full - coarse);
    let ramped = smoothed + (shape.floor_meters - smoothed) * shape.ramp;
    let limit = MAX_CHANGE_METERS + RAMP_MAX_CHANGE_METERS * shape.near_mouth;
    full + (ramped - full).clamp(-limit, limit)
}

/// One bounded sparse index. Failure publishes no partial path network.
#[cfg(test)]
pub fn pack(frame: &Frame, trails: &[Trail]) -> Result<Vec<[f32; 4]>, String> {
    pack_with_mouths(frame, trails, &[])
}

/// As `pack`, with each trail's court mouth (path-frame xy, court floor height
/// above the datum) for the ramp; missing entries have none.
pub fn pack_with_mouths(
    frame: &Frame,
    trails: &[Trail],
    mouths: &[Option<(DVec2, f64)>],
) -> Result<Vec<[f32; 4]>, String> {
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
        // The bank of the ground shaping reaches past the painted wear.
        let reach = t.width + BANK_METERS + 0.5;
        let lo = ((t.a.min(t.b) - DVec2::splat(reach)) / CELL).floor();
        let hi = ((t.a.max(t.b) + DVec2::splat(reach)) / CELL).floor();
        for y in lo.y as i32..=hi.y as i32 {
            for x in lo.x as i32..=hi.x as i32 {
                cells.entry((x, y)).or_default().push(i);
            }
        }
    }
    if cells.len() > MAX_TILES
        || cells.values().any(|v| v.len() > 128)
        || cells.values().map(Vec::len).sum::<usize>() > MAX_REFS
    {
        return Err("village path index budget".into());
    }
    let started = std::time::Instant::now();
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
    // w: ground shaping on (1) or off (0, `PLANET_VILLAGE_PATH_SHAPING=0`, the A/B override).
    data[3] = [frame.up.x as f32, frame.up.y as f32, frame.up.z as f32, f32::from(u8::from(shaping_enabled()))];
    let mut count = 0;
    let mut tiles = 0;
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
        let slot = slot - HEADER;
        data[TILE_INDEX + slot / 4][slot % 4] = tiles as f32;
        let tile = bake_tile(DVec2::new(x as f64, y as f64) * CELL, &refs, trails, mouths);
        data[ATLAS + tiles * TILE_FLOAT4S..ATLAS + (tiles + 1) * TILE_FLOAT4S].copy_from_slice(&tile);
        tiles += 1;
        for r in refs {
            data[REFS + count / 4][count % 4] = r as f32;
            count += 1;
        }
    }
    for (i, t) in trails.iter().enumerate() {
        data[SEGMENTS + 2 * i] = [t.a.x as f32, t.a.y as f32, t.b.x as f32, t.b.y as f32];
        data[SEGMENTS + 2 * i + 1] = match mouths.get(i).copied().flatten() {
            Some((mouth, floor)) => [t.width as f32, mouth.x as f32, mouth.y as f32, floor as f32],
            None => [t.width as f32, NO_MOUTH, NO_MOUTH, 0.0],
        };
    }
    tracing::info!(target:"planet::villages", tiles, pack_ms = started.elapsed().as_secs_f64() * 1e3, "village path index and shape atlas packed");
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    const BAKED_SHAPE_TOLERANCE: f64 = 0.1;
    #[test]
    fn optional_forecourts_cannot_evict_access_paths_on_a_full_budget() {
        let f = Frame::new(DVec3::X, 4_000_000.0).unwrap();
        let mut routes = vec![Trail { a:DVec2::ZERO,b:DVec2::Y*10.0,width:1.0 }];
        let mut mouths = vec![None];
        let extras = vec![routes[0]; MAX_SEGMENTS];
        let data = pack_public_paths(&f, &mut routes, &mut mouths, &extras).unwrap();
        assert_eq!(routes.len(),1);
        assert_eq!(mouths.len(),1);
        assert_eq!(data[0][3],1.0);
    }

    #[test]
    fn civic_forecourts_fit_the_existing_path_atlas() {
        let f = Frame::new(DVec3::X, 4_000_000.0).unwrap();
        let trails = civic_clearing(Footprint { centre: DVec2::ZERO, forward: DVec2::Y });
        assert!(pack(&f, &trails).is_ok());
        assert!(trails.iter().all(|t| t.width <= 3.0));
    }

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

    /// The CPU twin of the shader's shaping lookup finds every capsule whose
    /// bank reaches a point, including across cell edges, and the ramp
    /// follows the right court mouth.
    #[test]
    fn ground_shape_matches_a_direct_capsule_scan_and_ramps_to_the_mouth() {
        let frame = Frame::new(DVec3::X, 4e6).unwrap();
        let trails = [
            Trail { a: DVec2::new(-43.0, -31.0), b: DVec2::new(43.0, 31.0), width: 1.2 },
            Trail { a: DVec2::new(-20.0, 30.0), b: DVec2::new(20.0, 30.0), width: 0.8 },
        ];
        let mouth = DVec2::new(43.0, 31.0);
        let data =
            pack_with_mouths(&frame, &trails, &[Some((mouth, 512.0)), None]).unwrap();
        let mut seen = [false; 3];
        let mut worst = [0.0_f64; 2];
        for y in -70..=70 {
            for x in -100..=100 {
                let p = DVec2::new(x as f64 * 0.5, y as f64 * 0.5);
                let weights: Vec<f64> = trails
                    .iter()
                    .map(|t| {
                        let d = t.b - t.a;
                        let a = ((p - t.a).dot(d) / d.length_squared()).clamp(0.0, 1.0);
                        1.0 - smoothstep(t.width, t.width + BANK_METERS, p.distance(t.a.lerp(t.b, a)))
                    })
                    .collect();
                let shape = ground_shape(&data, p);
                let expected = weights.iter().copied().fold(0.0, f64::max);
                worst[0] = worst[0].max((shape.weight - expected).abs());
                let near = 1.0 - smoothstep(RAMP_FULL_METERS, RAMP_END_METERS, p.distance(mouth));
                worst[1] = worst[1].max((shape.ramp - weights[0] * near).abs());
                if shape.ramp > 0.0 {
                    assert_eq!(shape.floor_meters, 512.0);
                }
                seen[0] |= shape.weight == 1.0;
                seen[1] |= shape.weight > 0.0 && shape.weight < 1.0;
                seen[2] |= shape.ramp > 0.9;
            }
        }
        assert_eq!(seen, [true; 3]);
        // Baked 1 m samples of the bank distance, blended: close to exact
        // along a path (under 0.05); the worst (0.094) is at a path's open
        // round end, where the distance curves round the endpoint.
        assert!(worst[0] < BAKED_SHAPE_TOLERANCE && worst[1] < BAKED_SHAPE_TOLERANCE, "{worst:?}");
    }

    /// The baked samples are blended, not searched, so the shape is continuous
    /// across cell edges where the segment search had to agree exactly.
    #[test]
    fn baked_shape_is_continuous_across_cell_edges() {
        let frame = Frame::new(DVec3::X, 4e6).unwrap();
        let trails = [Trail { a: DVec2::new(-40.0, 3.0), b: DVec2::new(40.0, -5.0), width: 1.0 }];
        let data = pack(&frame, &trails).unwrap();
        for x in -3..=3 {
            let edge = x as f64 * CELL;
            for k in 0..40 {
                let y = -8.0 + k as f64 * 0.37;
                let a = ground_shape(&data, DVec2::new(edge - 1e-6, y)).weight;
                let b = ground_shape(&data, DVec2::new(edge + 1e-6, y)).weight;
                assert!((a - b).abs() < 1e-4, "edge {edge} y {y}: {a} vs {b}");
            }
        }
    }

    /// Chunks are flagged for shaping exactly when their box meets a shaped
    /// cell; every point with any shape lies in one.
    #[test]
    fn shaped_cells_cover_every_shaped_point_and_boxes_find_them() {
        let frame = Frame::new(DVec3::X, 4e6).unwrap();
        let trails = [Trail { a: DVec2::new(-40.0, 3.0), b: DVec2::new(40.0, -5.0), width: 1.0 }];
        let data = pack(&frame, &trails).unwrap();
        let cells = shaped_cells(&data);
        assert!(!cells.is_empty());
        for y in -40..=40 {
            for x in -120..=120 {
                let p = DVec2::new(x as f64 * 0.5, y as f64 * 0.5);
                if ground_shape(&data, p).weight > 0.0 {
                    assert!(touches_cells(&cells, p, p), "{p}");
                }
            }
        }
        // Both search orders: a small box, and one bigger than the set.
        assert!(touches_cells(&cells, DVec2::new(-1.0, -1.0), DVec2::new(1.0, 1.0)));
        assert!(!touches_cells(&cells, DVec2::new(500.0, 500.0), DVec2::new(510.0, 510.0)));
        assert!(touches_cells(&cells, DVec2::splat(-5000.0), DVec2::splat(5000.0)));
        assert!(!touches_cells(&cells, DVec2::new(1000.0, -5000.0), DVec2::new(5000.0, 5000.0)));
        let mut off = data.clone();
        off[3][3] = 0.0;
        assert!(shaped_cells(&off).is_empty());
    }

    /// planet.wgsl reads the atlas at fixed offsets; they must be this layout's.
    #[test]
    fn shader_atlas_offsets_match_the_packed_layout() {
        let shader = include_str!("village_paths.wgsl");
        for (name, value) in [
            ("VILLAGE_PATH_TILE_INDEX", TILE_INDEX),
            ("VILLAGE_PATH_ATLAS", ATLAS),
            ("VILLAGE_PATH_TILE_SIDE", TILE_SIDE),
            ("VILLAGE_PATH_TILE_FLOAT4S", TILE_FLOAT4S),
        ] {
            assert!(shader.contains(&format!("const {name}: u32 = {value}u;")), "{name} = {value}");
        }
        assert!(shader.contains("let bucket = village_path_data[1028u + slot];") && HEADER == 1028);
        assert!(shader.contains(&format!("village_path_data[{}u + r / 4u]", REFS)));
        assert!(shader.contains(&format!("village_path_data[{}u + index * 2u]", SEGMENTS)));
    }

    /// Off a path nothing moves; on one, short relief is mostly smoothed
    /// away; and the ground never moves more than the bound, except that the
    /// ramp may lift or lower it further right at the court mouth.
    #[test]
    fn shaped_height_smooths_short_relief_and_stays_bounded() {
        let off = GroundShape::default();
        assert_eq!(shaped_height(103.0, 100.0, off), 103.0);
        let on = GroundShape { weight: 1.0, ..GroundShape::default() };
        let smoothed = shaped_height(100.5, 100.0, on);
        assert!((smoothed - (100.0 + 0.5 * (1.0 - SMOOTHING))).abs() < 1e-12);
        // A tall bump is cut by the bound, not flattened to the coarse level.
        assert_eq!(shaped_height(105.0, 100.0, on), 105.0 - MAX_CHANGE_METERS);
        let mouth = GroundShape { weight: 1.0, ramp: 1.0, floor_meters: 110.0, near_mouth: 1.0 };
        assert_eq!(
            shaped_height(100.0, 100.0, mouth),
            100.0 + MAX_CHANGE_METERS + RAMP_MAX_CHANGE_METERS
        );
        let half = GroundShape { weight: 1.0, ramp: 0.5, floor_meters: 101.0, near_mouth: 0.5 };
        assert!((shaped_height(100.0, 100.0, half) - 100.5).abs() < 1e-12);
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
fn pack_public_paths(
    frame: &Frame,
    trails: &mut Vec<Trail>,
    mouths: &mut Vec<Option<(DVec2,f64)>>,
    clearings: &[Trail],
) -> Result<Vec<[f32;4]>,String> {
    let base = trails.len();
    trails.extend_from_slice(clearings);
    mouths.extend(std::iter::repeat_n(None,clearings.len()));
    match pack_with_mouths(frame,trails,mouths) {
        Err(error) if !clearings.is_empty() && error.contains("budget") => {
            // Optional broad forecourts must not evict real door-to-court
            // access or labels in a dense settlement. Keep the old GPU budget.
            tracing::warn!(target:"planet::villages", %error, "forecourt coverage omitted to retain access paths");
            trails.truncate(base);
            mouths.truncate(base);
            pack_with_mouths(frame,trails,mouths)
        }
        result => result,
    }
}

pub fn build(
    tiles: &std::collections::HashMap<planet_coretypes::TileKey, crate::outmap::TileData>,
    dense_level: u8,
    courts: &[(DVec3, DVec3)],
    instances: &[HouseInstance],
    worlds: &[DVec3],
    sites: &[DVec3],
    camera: DVec3,
    cancelled: &impl Fn() -> bool,
) -> Result<Built, String> {
    let frame = Frame::new(camera.normalize(), crate::planet::planet_radius_meters())?;
    let mut walkways = Vec::new();
    let mut groups = BTreeMap::<[u64; 3], Vec<usize>>::new();
    for (i, site) in sites.iter().enumerate() {
        groups
            .entry(site.to_array().map(f64::to_bits))
            .or_default()
            .push(i);
    }
    let mut trails = Vec::new();
    let mut mouths = Vec::new();
    let mut clearings = Vec::new();
    let mut gates: Vec<_> = instances.iter().zip(worlds)
        .filter_map(|(h, &p)| crate::village::places::board(h,p)).collect();
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
        let harbour = hs.iter().position(|h| crate::village::places::Kind::of(h) == crate::village::places::Kind::Harbour);
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
        if court.is_none() {
            if let Some(i) = harbour { goal = houses[i].door(); }
        }
        // Mark where the path leaves the court: a gateway over the mouth,
        // standing on the court floor and facing back across it. The same
        // point and floor height anchor the path's ramp onto the court.
        let mut mouth = None;
        if let Some((village, court)) = court_entry_of {
            mouth = Some((
                frame
                    .local(local.world(goal.extend(0.0)).normalize() * frame.radius)
                    .truncate(),
                court.length() - frame.radius,
            ));
            let mouth_world = local.world(goal.extend(0.0)).normalize() * court.length();
            let outward = mouth_world - *court;
            if outward.length() > 1.0 && !houses.is_empty() {
                gates.push(crate::road_navigation::footpath_gate(
                    mouth_world,
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
        // The same lanes as a walk graph for the villagers who wore them.
        let name = court_entry_of.map_or_else(
            || crate::road_navigation::village_name(site),
            |(village, _)| crate::road_navigation::village_name(*village),
        );
        let segments: Vec<_> = routed.iter().map(|t| (t.a, t.b)).collect();
        let door_points: Vec<_> = houses
            .iter()
            .zip(&hs)
            .map(|(f, h)| {
                (
                    f.centre - f.forward * 4.15,
                    crate::village::places::Kind::of(h) == crate::village::places::Kind::House,
                )
            })
            .collect();
        walkways.push(crate::crowd::Walkway::from_trails(
            name,
            &segments,
            &door_points,
            court.map(|_| goal),
            |p| local.world(p.extend(0.0)).normalize(),
        ));
        mouths.extend(std::iter::repeat_n(mouth, routed.len()));
        trails.extend(routed.into_iter().map(|t| Trail {
            a: project(t.a),
            b: project(t.b),
            width: t.width,
        }));
        for (h, footprint) in hs.iter().zip(&houses) {
            if crate::village::places::Kind::of(h) != crate::village::places::Kind::House {
                // Compact the public forecourt and clear trees beneath the
                // new buildings using the existing terrain-owned dirt mask.
                for clearing in civic_clearing(*footprint) {
                    clearings.push(Trail { a: project(clearing.a), b: project(clearing.b), width: clearing.width });
                }
            }
        }

    }
    if cancelled() {
        return Err("village paths superseded".into());
    }
    let data = pack_public_paths(&frame, &mut trails, &mut mouths, &clearings)?;
    // A failed sign bake keeps the paths; it only loses the gateways.
    let gate_mesh = crate::road_signs::bake(&gates, frame).unwrap_or_else(|error| {
        tracing::warn!(target:"planet::villages", %error, "footpath gateways rejected");
        crate::road_signs::Mesh::empty()
    });
    tracing::info!(target:"planet::villages", villages=groups.len(), segments=trails.len(), connected_doors=connected, doors=doors, gateways=gates.len(), "village desire lines built");
    Ok((frame, data, gate_mesh, walkways))
}
/// Packed path atlas, gateway signs and the villagers' walk graphs.
pub type Built = (Frame, Vec<[f32; 4]>, crate::road_signs::Mesh, Vec<crate::crowd::Walkway>);

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
type ResultData = Result<Built, String>;
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

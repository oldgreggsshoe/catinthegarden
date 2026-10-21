//! Worker-side engineering and cached geometry for streamed village roads.
use crate::{
    road::{self, Vertex},
    road_network::{self, Region, Survey, TerminalKind},
    road_network_field::{Field, Frame, Segment},
    road_network_profile::Profile,
};
use glam::{DVec2, DVec3};
use planet_coretypes::TileKey;
use std::sync::Arc;

pub const FIELD_RADIUS: f64 = 10_000.0;
pub const NEAR_RADIUS: f64 = 8_000.0;
pub const FAR_RADIUS: f64 = 25_000.0;
pub const MAX_VERTICES: usize = 1_500_000;

pub struct Prepared {
    pub region: Region,
    pub profile: Profile,
    pub tunnels: Vec<bool>,
    /// Original accepted centres, even though the road ends outside houses.
    pub village_centres: Vec<DVec3>,
}

impl Prepared {
    pub fn inspection_pose(&self, near: DVec3, radius: f64) -> Option<(DVec3, DVec3)> {
        let p = &self.profile;
        let tunnel_header = std::env::var("PLANET_ROAD_SIGN_VIEW").as_deref() == Ok("tunnel");
        {
            let mut status = vec![(false, false, None); p.directions.len()];
            for (index, &[a, b]) in p.segments.iter().enumerate() {
                for (i, j) in [(a, b), (b, a)] {
                    if self.tunnels[index] {
                        status[i].0 = true;
                        status[i].2 = Some(j);
                    } else {
                        status[i].1 = true;
                    }
                }
            }
            if let Some((i, (_, _, Some(j)))) = status
                .iter()
                .enumerate()
                .filter(|(_, s)| s.0 && s.1)
                .min_by(|(a, _), (b, _)| {
                    DVec3::from_array(p.directions[*a])
                        .distance_squared(near.normalize())
                        .total_cmp(
                            &DVec3::from_array(p.directions[*b]).distance_squared(near.normalize()),
                        )
                })
            {
                let at = DVec3::from_array(p.directions[i]) * (radius + p.heights_meters[i]);
                let next = DVec3::from_array(p.directions[*j]) * (radius + p.heights_meters[*j]);
                if tunnel_header {
                    return Some((
                        at - (next - at).normalize() * 18.0 + at.normalize() * 2.0,
                        at + at.normalize() * 7.5,
                    ));
                }
                // A surveyed entrance seed must not be replaced by the nearest
                // village court. Use the real approach for dry outdoor mouths;
                // a submerged mouth starts 3m inside its dry bore so normal sea
                // clearance cannot relocate the eye onto the water. This does
                // not repair the exposed approach. Legacy distant starts stay unchanged.
                if near.distance(at) < 32.0 {
                    let outside = p.segments.iter().enumerate().find_map(|(edge, &[a, b])| {
                        if self.tunnels[edge] {
                            return None;
                        }
                        let other = if a == i {
                            b
                        } else if b == i {
                            a
                        } else {
                            return None;
                        };
                        let point = DVec3::from_array(p.directions[other])
                            * (radius + p.heights_meters[other]);
                        (point.distance(at) > 0.001).then_some(point)
                    });
                    if let Some(outside) = outside {
                        let floor = if at.length() < radius {
                            at.lerp(next, (3.0 / at.distance(next)).min(1.0))
                        } else {
                            at.lerp(outside, (18.0 / at.distance(outside)).min(1.0))
                        };
                        return Some((floor + at.normalize() * 2.0, next + next.normalize() * 2.0));
                    }
                }
            }
        }
        let node = self
            .region
            .terminals
            .iter()
            .zip(&self.region.terminal_nodes)
            .filter(|(t, _)| matches!(t.kind, TerminalKind::Village))
            .map(|(_, &i)| i)
            .min_by(|&a, &b| {
                let distance = |i: usize| {
                    (DVec3::from_array(p.directions[i]) * (radius + p.heights_meters[i]))
                        .distance_squared(near)
                };
                distance(a).total_cmp(&distance(b))
            })
            .or_else(|| {
                p.segments
                    .iter()
                    .filter(|&&[a, b]| {
                        DVec3::from_array(p.directions[a])
                            .distance(DVec3::from_array(p.directions[b]))
                            > 1e-10
                    })
                    .min_by(|&&[a, _], &&[b, _]| {
                        let distance = |i: usize| {
                            (DVec3::from_array(p.directions[i]) * (radius + p.heights_meters[i]))
                                .distance_squared(near)
                        };
                        distance(a).total_cmp(&distance(b))
                    })
                    .map(|e| e[0])
            })?;
        let other = p.segments.iter().find_map(|&[a, b]| {
            let other = if a == node {
                Some(b)
            } else if b == node {
                Some(a)
            } else {
                None
            }?;
            (DVec3::from_array(p.directions[other]).distance(DVec3::from_array(p.directions[node]))
                > 1e-10)
                .then_some(other)
        })?;
        let point = |i: usize| DVec3::from_array(p.directions[i]) * (radius + p.heights_meters[i]);
        let a = point(node);
        let b = point(other);
        let length = a.distance(b);
        let start = a.lerp(b, (10.0 / length).min(0.5));
        let eye = start + start.normalize() * 2.0;
        Some((eye, eye + (b - a).normalize() * 45.0))
    }
}

// A preference must not turn a previously connected village into a rejected
// region. Bound speculative work deterministically, then retain the established
// conservative survey/profile if the cheaper-bore alignment cannot be built.
const PREFERRED_SURVEY_SAMPLES: usize = 100_000;
// Stop a speculative solver/search that keeps polling without advancing the
// terrain survey; the conservative pass retains its existing full budgets.
const PREFERRED_QUIET_CHECKS: usize = 256;
struct TunnelPreference<'a, T> {
    terrain: &'a T,
    samples: std::cell::Cell<usize>,
    last_samples: std::cell::Cell<usize>,
    quiet_checks: std::cell::Cell<usize>,
}
impl<T: Survey> Survey for TunnelPreference<'_, T> {
    fn cancelled(&self) -> bool {
        if self.terrain.cancelled() {
            return true;
        }
        let samples = self.samples.get();
        if samples == self.last_samples.get() {
            self.quiet_checks
                .set(self.quiet_checks.get().saturating_add(1));
        } else {
            self.last_samples.set(samples);
            self.quiet_checks.set(0);
        }
        samples >= PREFERRED_SURVEY_SAMPLES || self.quiet_checks.get() >= PREFERRED_QUIET_CHECKS
    }
    fn ground(&self, d: DVec3) -> Result<road_network::Ground, String> {
        if self.cancelled() {
            return Err("tunnel preference probe budget exhausted".into());
        }
        self.samples.set(self.samples.get() + 1);
        self.terrain.ground(d)
    }
    fn villages(&self, key: TileKey) -> Result<Vec<DVec3>, String> {
        self.terrain.villages(key)
    }
    fn settlement_core_at(&self, d: DVec3) -> Result<bool, String> {
        self.terrain.settlement_core_at(d)
    }
    fn village_access_height(&self, g: road_network::Ground) -> f64 {
        self.terrain.village_access_height(g)
    }
    fn may_have_villages(&self, key: TileKey) -> Result<bool, String> {
        self.terrain.may_have_villages(key)
    }
}
pub fn prepare_local(
    backbone: &road_network::Backbone,
    key: TileKey,
    radius: f64,
    terrain: &impl Survey,
) -> Result<Prepared, String> {
    if terrain.cancelled() {
        return Err("road generation superseded".into());
    }
    let preferred = TunnelPreference {
        terrain,
        samples: std::cell::Cell::new(0),
        last_samples: std::cell::Cell::new(0),
        quiet_checks: std::cell::Cell::new(0),
    };
    match backbone
        .local(key, radius, &preferred)
        .and_then(|r| prepare(r, radius, &preferred))
    {
        Ok(plan) => {
            tracing::debug!(
                ?key,
                samples = preferred.samples.get(),
                "tunnel-preferred alignment accepted"
            );
            return Ok(plan);
        }
        Err(error) => {
            if terrain.cancelled() {
                return Err(error);
            }
            tracing::debug!(?key,%error,"tunnel-preferred alignment rejected; keeping conservative route");
        }
    }
    prepare(
        backbone.local_conservative(key, radius, terrain)?,
        radius,
        terrain,
    )
}

pub fn prepare(region: Region, radius: f64, terrain: &impl Survey) -> Result<Prepared, String> {
    let mut error = String::new();
    // Siting a wider court changes the fixed approach height. A dry first
    // choice is not necessarily grade/curvature-feasible: retry farther out
    // before rejecting the whole resident network. All engineering caps stay.
    for setback in [0.0, 32.0, 64.0] {
        if terrain.cancelled() {
            return Err("road generation superseded".into());
        }
        match prepare_at(region.clone(), radius, terrain, setback) {
            Ok(plan) => return Ok(plan),
            Err(e) => error = e,
        }
    }
    Err(error)
}
fn prepare_at(
    mut region: Region,
    radius: f64,
    terrain: &impl Survey,
    setback: f64,
) -> Result<Prepared, String> {
    let mut centres = Vec::new();
    for (index, terminal) in region.terminals.iter_mut().enumerate() {
        if !matches!(terminal.kind, TerminalKind::Village) {
            continue;
        }
        let centre = DVec3::from_array(terminal.direction);
        centres.push(centre);
        let node = region.terminal_nodes[index];
        let neighbour = region.edges.iter().find_map(|e| {
            if e.a == node {
                Some(e.b)
            } else if e.b == node {
                Some(e.a)
            } else {
                None
            }
        });
        if let Some(next) = neighbour {
            let toward = DVec3::from_array(region.nodes[next].direction);
            let tangent = (toward - centre * toward.dot(centre)).normalize_or_zero();
            let mut approach = None;
            // Prefer the existing route direction, but never put an apron in
            // another village's houses or push a coastal approach into water.
            for extra in [0.0, 32.0, 64.0, 96.0] {
                for step in 0..16 {
                    let turn = if step % 2 == 0 {
                        step as f64 / 2.0
                    } else {
                        -((step + 1) as f64) / 2.0
                    };
                    let along =
                        glam::DQuat::from_axis_angle(centre, turn * std::f64::consts::TAU / 16.0)
                            * tangent;
                    let d = (centre * radius
                        + along
                            * (crate::village::VILLAGE_RADIUS_METERS
                                + crate::road_parking::REACH
                                + crate::road_parking::OFFSET
                                + setback
                                + 4.0
                                + extra))
                        .normalize();
                    if planet_coretypes::tile_key_for_direction(d, region.key.level) == region.key
                        && !terrain.ground(d)?.ocean
                        && !terrain.settlement_core_at(d)?
                    {
                        let mut clear = true;
                        for q in crate::road_parking::siting_samples(d, radius) {
                            if terrain.ground(q)?.ocean || terrain.settlement_core_at(q)? {
                                clear = false;
                                break;
                            }
                        }
                        if clear {
                            for q in crate::road_parking::bank_samples(d, radius) {
                                if terrain.settlement_core_at(q)? {
                                    clear = false;
                                    break;
                                }
                            }
                        }
                        if !clear {
                            continue;
                        }
                        approach = Some(d);
                        break;
                    }
                }
                if approach.is_some() {
                    break;
                }
            }
            let approach =
                approach.ok_or("no dry village access apron outside protected houses")?;
            terminal.direction = approach.to_array();
            region.nodes[node].direction = approach.to_array();
            region.nodes[node].ground_meters = terrain.ground(approach)?.height;
        }
    }
    crate::road_network_align::separate_accesses(&mut region, radius, terrain, None)?;
    let mut curved = None;
    for target in [
        crate::road_network_align::TARGET_RADIUS,
        60.0,
        30.0,
        crate::road_network_align::MIN_RADIUS,
    ] {
        let Ok(aligned) = crate::road_network_align::align(&region, radius, terrain, None, target)
        else {
            continue;
        };
        if let Ok(profile) = Profile::fit(&aligned.region, radius, terrain)? {
            tracing::debug!(minimum_radius=?aligned.minimum_radius, "network horizontal alignment fitted");
            curved = Some((aligned.region, profile));
            break;
        }
    }
    let profile = match curved {
        Some((aligned, profile)) => {
            region = aligned;
            profile
        }
        None => {
            let plan = crate::road_network_route::reroute(&region, radius, terrain)?
                .map_err(|e| format!("region {:?} engineering failed: {e:?}", region.key))?;
            region = plan.region;
            plan.profile
        }
    };
    let tunnels = classify_tunnels(&profile, region.key, radius, terrain)?;
    Ok(Prepared {
        tunnels,
        region,
        profile,
        village_centres: centres,
    })
}

/// Stable worker metadata shared by geometry and navigation, never cropped by
/// the camera's resident field. Keeping one classification prevents signs from
/// measuring an open cut as a tunnel (or treating a cache edge as an exit).
fn classify_tunnels(
    p: &Profile,
    key: TileKey,
    radius: f64,
    terrain: &impl Survey,
) -> Result<Vec<bool>, String> {
    let frame = Frame::new(road_network::direction(key, 0.5, 0.5), radius)?;
    let points: Vec<_> = p
        .directions
        .iter()
        .zip(&p.heights_meters)
        .map(|(d, h)| frame.local(DVec3::from_array(*d) * (radius + h)))
        .collect();
    let mut adjacent = vec![Vec::new(); points.len()];
    for &[a, b] in &p.segments {
        adjacent[a].push(b);
        adjacent[b].push(a);
    }
    let mut covered = Vec::new();
    let mut ocean = Vec::new();
    let mut ground = Vec::new();
    for (i, &point) in points.iter().enumerate() {
        let d = DVec3::from_array(p.directions[i]);
        let g = terrain.ground(d)?;
        let toward = adjacent[i]
            .iter()
            .find_map(|&j| {
                ((points[j] - point).truncate().length() > 0.001).then_some(points[j] - point)
            })
            .unwrap_or(DVec3::Y);
        let side = DVec3::new(toward.y, -toward.x, 0.0).normalize_or_zero();
        let mut cover = true;
        for offset in [-8.0, 0.0, 8.0] {
            let q = frame.world(point + side * offset).normalize();
            cover &= terrain.ground(q)?.height - p.heights_meters[i] >= 14.0;
        }
        if g.ocean && !cover {
            return Err("undersea bore has insufficient side/roof cover".into());
        }
        covered.push(cover);
        ocean.push(g.ocean);
        ground.push(g.height);
    }
    // Covered connected components extend deep bores toward real portals.
    // Shared sea gateways remain covered on BOTH neighbouring regions.
    let mut tunnel = vec![false; p.segments.len()];
    let mut edge_adjacency = vec![Vec::new(); points.len()];
    for (j, &[a, b]) in p.segments.iter().enumerate() {
        if covered[a] && covered[b] {
            edge_adjacency[a].push(j);
            edge_adjacency[b].push(j);
        }
    }
    let mut visited = vec![false; points.len()];
    for root in 0..points.len() {
        if visited[root] || edge_adjacency[root].is_empty() {
            continue;
        }
        let mut stack = vec![root];
        let mut edges = std::collections::BTreeSet::new();
        let mut deep_length = 0.0;
        let mut sea = false;
        while let Some(i) = stack.pop() {
            if visited[i] {
                continue;
            }
            visited[i] = true;
            sea = sea
                || ocean[i]
                || (ground[i] < crate::road_network_profile::COAST_SURVEY_HEIGHT
                    && crate::road_network_profile::near_coast(
                        DVec3::from_array(p.directions[i]),
                        radius,
                        terrain,
                    )?)
                || terrain.settlement_core_at(DVec3::from_array(p.directions[i]))?;
            for &e in &edge_adjacency[i] {
                if edges.insert(e) {
                    let [a, b] = p.segments[e];
                    if (ground[a] - p.heights_meters[a]).min(ground[b] - p.heights_meters[b])
                        >= 19.0
                    {
                        deep_length += points[a].distance(points[b]);
                    }
                    stack.push(if a == i { b } else { a });
                }
            }
        }
        if sea || deep_length >= crate::road_tunnel::MIN_LENGTH {
            for e in edges {
                tunnel[e] = true;
            }
        }
    }
    // A cap/route which cannot provide a dry inland mouth is infeasible, not
    // permission to open a submerged excavation or extend a fake tunnel void.
    let mut mouths = vec![(false, false); points.len()];
    for (e, &[a, b]) in p.segments.iter().enumerate() {
        for i in [a, b] {
            if tunnel[e] {
                mouths[i].0 = true;
            } else {
                mouths[i].1 = true;
            }
        }
    }
    for (i, &(covered, open)) in mouths.iter().enumerate() {
        if covered && open {
            if p.heights_meters[i] < crate::road_network_profile::COAST_MIN_DECK
                || ocean[i]
                || (p.heights_meters[i] < crate::road_network_profile::COAST_HIGH_DECK
                    && crate::road_network_profile::near_coast(
                        DVec3::from_array(p.directions[i]),
                        radius,
                        terrain,
                    )?)
            {
                return Err(format!(
                    "tunnel mouth needs a dry inland approach clear of the shoreline: {key:?} node {i}, deck {:.3}m, ground {:.3}m",
                    p.heights_meters[i], ground[i]
                ));
            }
        }
    }
    Ok(tunnel)
}

struct Chunk {
    pub near: Vec<Vertex>,
    pub far: Vec<Vertex>,
    pub structures: Vec<Vertex>,
    /// Planet-frame bounds, including piers and lining.
    pub centre: DVec3,
    pub radius: f64,
}
pub struct DrawChunk {
    pub near: std::ops::Range<u32>,
    pub far: std::ops::Range<u32>,
    pub structures: std::ops::Range<u32>,
    pub centre: DVec3,
    pub radius: f64,
}
pub struct Snapshot {
    pub vertices: Vec<Vertex>,
    pub lighting: Vec<[f32; 4]>,
    pub visibility: crate::tunnel_visibility::Visibility,
    pub lines: Vec<crate::road_overlay::Line>,
    pub overview: Arc<Vec<crate::road_overlay::Line>>,
    pub field: Arc<Field>,
    pub chunks: Vec<DrawChunk>,
    pub keys: Vec<TileKey>,
    pub villages: usize,
    pub segments: usize,
    pub tunnels: usize,
    pub inspection: Option<(DVec3, DVec3)>,
    pub build_centre: DVec3,
}

struct Road {
    light_station: [f64; 2],
    segment: Segment,
    piers: bool,
    near: bool,
    portal_a: bool,
    portal_b: bool,
    ribbon: Ribbon,
}

/// Retain whole regions in the CPU cache; crop only mesh/index residency.
/// Every accepted village was enumerated independently of camera visibility.
pub fn snapshot(
    plans: &[Arc<Prepared>],
    centre: DVec3,
    radius: f64,
    terrain: &impl Survey,
) -> Result<Snapshot, String> {
    let key = planet_coretypes::tile_key_for_direction(centre, road_network::REGION_LEVEL);
    let frame = Frame::new(road_network::direction(key, 0.5, 0.5), radius)?;
    let eye = frame.local(centre);
    let mut roads = Vec::new();
    let mut parking = Vec::new();
    let mut degrees = std::collections::BTreeMap::<_, usize>::new();
    let mut gateway_branches = std::collections::BTreeSet::new();

    let mut inspection = None;
    for plan in plans {
        let p = &plan.profile;
        let points: Vec<_> = p
            .directions
            .iter()
            .zip(&p.heights_meters)
            .map(|(d, h)| frame.local(DVec3::from_array(*d) * (radius + h)))
            .collect();
        let lamp_stations = crate::road_lighting::stations(&points, &p.segments);
        let mut adjacent = vec![Vec::new(); points.len()];
        for &[a, b] in &p.segments {
            adjacent[a].push(b);
            adjacent[b].push(a);
        }
        // An internal two-arm gateway also has a third arm in the next
        // region. It remains a junction even when that neighbour is unloaded.
        let mut local_degree = std::collections::BTreeMap::<_, usize>::new();
        for &[a, b] in &p.segments {
            if (points[b] - points[a]).truncate().length() < 0.001 {
                continue;
            }
            for point in [points[a], points[b]] {
                *local_degree.entry(node_key(point)).or_default() += 1;
            }
        }
        for (terminal, &i) in plan
            .region
            .terminals
            .iter()
            .zip(&plan.region.terminal_nodes)
        {
            if matches!(terminal.kind, TerminalKind::Gateway { .. })
                && local_degree.get(&node_key(points[i])).copied().unwrap_or(0) > 1
            {
                gateway_branches.insert(node_key(points[i]));
            }
        }
        let tunnel = &plan.tunnels;
        let mut portal_status = vec![(false, false); points.len()];
        for (e, &[a, b]) in p.segments.iter().enumerate() {
            if (points[b] - points[a]).truncate().length() < 0.001 {
                continue;
            }
            for i in [a, b] {
                if tunnel[e] {
                    portal_status[i].0 = true;
                } else {
                    portal_status[i].1 = true;
                }
            }
        }
        let accesses: Vec<_> = plan
            .region
            .terminals
            .iter()
            .zip(&plan.region.terminal_nodes)
            .filter(|(t, _)| matches!(t.kind, TerminalKind::Village))
            .map(|(_, &i)| points[i])
            .collect();
        for (terminal, &i) in plan
            .region
            .terminals
            .iter()
            .zip(&plan.region.terminal_nodes)
        {
            if !matches!(terminal.kind, TerminalKind::Village) {
                continue;
            }
            let access = points[i];
            let neighbour = adjacent[i]
                .iter()
                .find(|&&j| (points[j] - access).truncate().length() > 0.001)
                .copied()
                .ok_or("village court has no physical approach")?;
            let outward = (access - points[neighbour])
                .truncate()
                .normalize()
                .extend(0.0);
            let court = access + outward * crate::road_parking::OFFSET;
            if (court - eye).truncate().length() < FAR_RADIUS {
                parking.push(crate::road_parking::segment(court));
            }
        }
        let joint = |i: usize| {
            let neighbours: Vec<_> = adjacent[i]
                .iter()
                .copied()
                .filter(|&j| (points[j] - points[i]).truncate().length() > 0.001)
                .collect();
            let gateway = plan.region.terminals.iter().any(|t| {
                matches!(t.kind, TerminalKind::Gateway { .. })
                    && DVec3::from_array(t.direction).distance(DVec3::from_array(p.directions[i]))
                        < 1e-10
            });
            gateway
                || neighbours.len() > 2
                || (neighbours.len() == 2 && {
                    let a = (points[neighbours[0]] - points[i]).truncate().normalize();
                    let b = (points[neighbours[1]] - points[i]).truncate().normalize();
                    a.dot(b) > -0.995
                })
        };
        for (index, &[a, b]) in p.segments.iter().enumerate() {
            // Gateway aliases are distinct graph terminals at one position.
            // Preserve that topology, but never make a zero-area mesh/index span.
            if (points[b] - points[a]).truncate().length() < 0.001 {
                continue;
            }
            // Preserve physical topology before cropping resident spans.
            for point in [points[a], points[b]] {
                *degrees.entry(node_key(point)).or_default() += 1;
            }
            let middle = (points[a] + points[b]) * 0.5;
            let distance = (middle - eye).truncate().length();
            if distance > FAR_RADIUS + 100.0 {
                continue;
            }
            let access_distance = accesses
                .iter()
                .map(|v| {
                    v.truncate()
                        .distance(points[a].truncate())
                        .min(v.truncate().distance(points[b].truncate()))
                })
                .fold(f64::INFINITY, f64::min);
            let floor_depth = if tunnel[index] {
                6.0
            } else {
                0.15 + 5.85 * (access_distance / 60.0).min(1.0)
            };
            let segment = Segment {
                a: points[a],
                b: points[b],
                tunnel: tunnel[index],
                reach: 64.0,
                floor_depth,
                junction_a: joint(a),
                junction_b: joint(b),
            };
            if inspection.is_none() {
                let p = frame.world(segment.a);
                let q = frame.world(segment.b);
                inspection = Some((p + p.normalize() * 2.0, q + q.normalize() * 2.0));
            }
            roads.push(Road {
                light_station: lamp_stations[index],
                segment,
                piers: index % 2 == 0,
                near: distance < NEAR_RADIUS,
                portal_a: portal_status[a] == (true, true),
                portal_b: portal_status[b] == (true, true),
                ribbon: Ribbon::straight(segment),
            });
        }
    }
    for r in &mut roads {
        r.ribbon.branches = [r.segment.a, r.segment.b]
            .map(|p| degrees[&node_key(p)] > 2 || gateway_branches.contains(&node_key(p)));
    }
    join_ribbons(&mut roads)?;
    let local_roads: Vec<_> = roads
        .iter()
        .filter(|r| {
            ((r.segment.a + r.segment.b) * 0.5 - eye)
                .truncate()
                .length()
                < FIELD_RADIUS
        })
        .collect();
    let local_parking: Vec<_> = parking
        .iter()
        .copied()
        .filter(|s| (s.a - eye).truncate().length() < FIELD_RADIUS)
        .collect();
    let mut field = Field::build(
        frame,
        local_roads
            .iter()
            .map(|r| r.segment)
            .chain(local_parking.iter().copied())
            .collect(),
    )?;
    field.set_deck_sections(
        local_roads
            .iter()
            .map(|r| r.ribbon.sides)
            .chain(local_parking.iter().map(|_| [DVec3::ZERO; 2]))
            .collect(),
    )?;
    field.set_geometry_window(eye.truncate(), NEAR_RADIUS - 500.0);
    let visibility = crate::tunnel_visibility::Visibility::build(
        frame,
        &roads
            .iter()
            // Liners are drawn for every bore, independently of deck LOD.
            .map(|r| (r.segment, [r.portal_a, r.portal_b], true))
            .collect::<Vec<_>>(),
    );
    let spans: Vec<_> = roads
        .iter()
        .filter(|r| r.near && r.segment.tunnel)
        .map(|r| crate::road_lighting::Span {
            segment: r.segment,
            sides: r.ribbon.sides,
            station: r.light_station,
        })
        .collect();
    let mut lighting = crate::road_lighting::Lighting::build(&spans, &field);
    let mut light_group = 0;
    let mut chunks = Vec::new();
    let mut vertices = 0;
    // Small cached chunks cull cheaply, without one pipeline/object per road.
    let mut first = 0;
    while first < roads.len() {
        let end = chunk_end(&roads, first);
        let group = &roads[first..end];
        first = end;
        let mut near = Vec::new();
        let mut far = Vec::new();
        let mut structures = Vec::new();
        for road in group {
            let s = road.segment;
            let grade = (s.b.z - s.a.z) / (s.b - s.a).truncate().length();
            let tangents = road
                .ribbon
                .sides
                .map(|side| DVec3::new(-side.y, side.x, grade) * road::HALF_WIDTH);
            let far_start = far.len();
            road::far_segment(&mut far, s.a, s.b, tangents[0], tangents[1]);
            if s.tunnel {
                for v in &mut far[far_start..] {
                    v.colour[2] = -v.colour[2];
                }
            }
            let near_start = near.len();
            let structure_start = structures.len();
            if road.near {
                deck(&mut near, s, road.ribbon, &field, terrain, road.piers)?;
            }
            if s.tunnel {
                liner(
                    &mut structures,
                    s,
                    road.ribbon.sides,
                    road.ribbon.branches,
                    road.portal_a,
                    road.portal_b,
                );
            }
            if road.near && s.tunnel {
                structures.append(&mut lighting.fixtures[light_group]);
                light_group += 1;
                for v in &mut far[far_start..] {
                    v.lighting = light_group as u32;
                }
                for v in near[near_start..]
                    .iter_mut()
                    .chain(&mut structures[structure_start..])
                {
                    // Only the interior shell/deck, never the outdoor shell.
                    if !v.colour.iter().all(|&c| c < 0.0) && v.colour[0] >= 0.0 {
                        v.lighting = light_group as u32;
                    }
                }
            }
            // The displaced terrain is the sole open-bank surface. Wide
            // offset strips fold over tight bends and compete with that
            // terrain, producing moving slivers and depth interference.
        }
        vertices += near.len() + far.len() + structures.len();
        if vertices > MAX_VERTICES {
            return Err("resident road mesh vertex budget exceeded".into());
        }
        let mut low = DVec3::splat(f64::INFINITY);
        let mut high = DVec3::splat(f64::NEG_INFINITY);
        for v in near.iter().chain(&far).chain(&structures) {
            let p = DVec3::from_array(v.position.map(f64::from));
            low = low.min(p);
            high = high.max(p);
        }
        let centre = (low + high) * 0.5;
        let radius = low.distance(high) * 0.5;
        chunks.push(Chunk {
            near,
            far,
            structures,
            centre: frame.world(centre),
            radius,
        });
    }
    for s in parking {
        let structures = crate::road_parking::mesh(
            s,
            &field,
            terrain,
            (s.a - eye).truncate().length() < NEAR_RADIUS,
        )?;
        vertices += structures.len();
        if vertices > MAX_VERTICES {
            return Err("resident road mesh vertex budget exceeded".into());
        }
        let mut low = DVec3::splat(f64::INFINITY);
        let mut high = -low;
        for v in &structures {
            let p = DVec3::from_array(v.position.map(f64::from));
            low = low.min(p);
            high = high.max(p);
        }
        chunks.push(Chunk {
            near: Vec::new(),
            far: Vec::new(),
            structures,
            centre: frame.world((low + high) * 0.5),
            radius: low.distance(high) * 0.5,
        });
    }
    let mut mesh = Vec::new();
    let mut far_mesh = Vec::new();
    let mut structure_mesh = Vec::new();
    let mut draws = Vec::new();
    for chunk in chunks {
        let near = mesh.len() as u32;
        mesh.extend(chunk.near);
        let far = far_mesh.len() as u32;
        far_mesh.extend(chunk.far);
        let structures = structure_mesh.len() as u32;
        structure_mesh.extend(chunk.structures);
        draws.push(DrawChunk {
            near: near..mesh.len() as u32,
            far: far..far_mesh.len() as u32,
            structures: structures..structure_mesh.len() as u32,
            centre: chunk.centre,
            radius: chunk.radius,
        });
    }
    let far_base = mesh.len() as u32;
    mesh.extend(far_mesh);
    let structure_base = mesh.len() as u32;
    mesh.extend(structure_mesh);
    for draw in &mut draws {
        draw.far.start += far_base;
        draw.far.end += far_base;
        draw.structures.start += structure_base;
        draw.structures.end += structure_base;
    }
    Ok(Snapshot {
        lighting: lighting.data,
        visibility,
        overview: Arc::new(Vec::new()),
        lines: roads
            .iter()
            .map(|r| crate::road_overlay::Line {
                a: r.segment.a.as_vec3().to_array(),
                b: r.segment.b.as_vec3().to_array(),
                generated: 1.0,
            })
            .collect(),
        vertices: mesh,
        field: Arc::new(field),
        chunks: draws,
        keys: plans.iter().map(|p| p.region.key).collect(),
        villages: plans.iter().map(|p| p.village_centres.len()).sum(),
        segments: roads.len(),
        tunnels: roads.iter().filter(|r| r.segment.tunnel).count(),
        inspection,
        build_centre: centre,
    })
}

/// A graph's next edge can start kilometres away. Never let one LOD/culling
/// batch mix that far road with a nearby deck or mix cached near/far geometry.
fn chunk_end(roads: &[Road], first: usize) -> usize {
    let mut end = first + 1;
    while end < roads.len().min(first + 25)
        && roads[end].near == roads[first].near
        && roads[end - 1].segment.b.distance(roads[end].segment.a) < 1.0
    {
        end += 1;
    }
    end
}

/// Shared cross-sections close both the deck and parapets at sampled bends.
/// Resolve across regions as well: gateway endpoints share one physical join.
fn node_key(p: DVec3) -> [i64; 3] {
    p.to_array().map(|v| (v * 100.0).round() as i64)
}
fn join_ribbons(roads: &mut [Road]) -> Result<(), String> {
    use std::collections::BTreeMap;
    let mut nodes = BTreeMap::<_, Vec<(usize, usize)>>::new();
    for (i, road) in roads.iter().enumerate() {
        for (end, p) in [road.segment.a, road.segment.b].into_iter().enumerate() {
            nodes.entry(node_key(p)).or_default().push((i, end));
        }
    }
    for ends in nodes.values() {
        let branch = ends.len() > 2 || ends.iter().any(|&(i, end)| roads[i].ribbon.branches[end]);
        for &(i, end) in ends {
            roads[i].ribbon.branches[end] = branch;
            if end == 0 {
                roads[i].segment.junction_a = ends.len() > 1;
            } else {
                roads[i].segment.junction_b = ends.len() > 1;
            }
        }
        let mut pairs = Vec::new();
        if branch {
            // Keep the smooth through carriageway watertight even where a side
            // road joins it. Only pair mutually unambiguous opposite arms; an
            // acute fork or a cropped branch is not an ordinary sharp bend.
            let outward: Vec<_> = ends
                .iter()
                .map(|&(i, end)| {
                    let s = roads[i].segment;
                    let d = if end == 0 { s.b - s.a } else { s.a - s.b };
                    d.truncate().normalize_or_zero()
                })
                .collect();
            let limit = -(crate::road_network_align::MAX_ANGLE + 0.001).cos();
            let partners: Vec<_> = outward
                .iter()
                .map(|a| {
                    let mut matches = outward
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| a.dot(**b) < limit);
                    let first = matches.next().map(|(i, _)| i);
                    if matches.next().is_none() {
                        first
                    } else {
                        None
                    }
                })
                .collect();
            for (a, partner) in partners.iter().enumerate() {
                if let Some(b) = *partner {
                    if a < b && partners[b] == Some(a) {
                        pairs.push([ends[a], ends[b]]);
                    }
                }
            }
        } else if let &[a, b] = ends.as_slice() {
            pairs.push([a, b]);
        }
        for [(a, ae), (b, be)] in pairs {
            let sa = roads[a].ribbon.sides[ae];
            let sb = roads[b].ribbon.sides[be];
            let sign = if sa.dot(sb) >= 0.0 { 1.0 } else { -1.0 };
            let middle = (sa + sb * sign).normalize_or_zero();
            let cosine = middle.dot(sa);
            if cosine < ((crate::road_network_align::MAX_ANGLE + 0.001) * 0.5).cos() {
                return Err(format!(
                    "unrounded network join at {:?}: {:?} {:?}",
                    if ae == 0 {
                        roads[a].segment.a
                    } else {
                        roads[a].segment.b
                    },
                    roads[a].ribbon.sides[ae],
                    roads[b].ribbon.sides[be]
                ));
            }
            let miter = middle / cosine;
            roads[a].ribbon.sides[ae] = miter;
            roads[b].ribbon.sides[be] = miter * sign;
        }
    }
    // Station-based paint and supports, not one restart/pier per arc sample.
    let mut visited = vec![false; roads.len()];
    let roots: Vec<_> = nodes
        .values()
        .filter(|ends| ends.len() != 2 || ends.iter().any(|&(i, e)| roads[i].ribbon.branches[e]))
        .collect();
    for ends in roots {
        for &(root, end) in ends {
            let mut next = Some((root, end));
            let mut station = 0.0;
            while let Some((i, from)) = next {
                if visited[i] {
                    break;
                }
                visited[i] = true;
                let s = roads[i].segment;
                let length = s.a.distance(s.b);
                let finish = station + length;
                roads[i].ribbon.station = if from == 0 {
                    [station, finish]
                } else {
                    [finish, station]
                };
                roads[i].piers =
                    ((finish + 20.0) / 40.0).floor() > ((station + 20.0) / 40.0).floor();
                station = finish;
                let p = if from == 0 { s.b } else { s.a };
                let ends = &nodes[&node_key(p)];
                next = if ends.len() == 2 && !roads[i].ribbon.branches[1 - from] {
                    ends.iter().copied().find(|&(j, _)| j != i)
                } else {
                    None
                };
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Ribbon {
    sides: [DVec3; 2],
    branches: [bool; 2],
    station: [f64; 2],
}
impl Ribbon {
    fn straight(s: Segment) -> Self {
        let delta = s.b - s.a;
        let side = DVec3::new(delta.y, -delta.x, 0.0).normalize();
        Self {
            sides: [side; 2],
            branches: [s.junction_a, s.junction_b],
            station: [0.0, delta.length()],
        }
    }
    fn point(&self, s: Segment, t: f64, offset: f64) -> DVec3 {
        s.a.lerp(s.b, t) + self.sides[0].lerp(self.sides[1], t) * offset
    }
}

/// Subtract only actual connected road mouths, never a whole approach span.
/// All work is cached on the worker; the fragment candidate bound is unchanged.
fn exposed(ribbon: Ribbon, s: Segment, field: &Field, offset: f64, width: f64) -> Vec<(f64, f64)> {
    let a = ribbon.point(s, 0.0, offset);
    let b = ribbon.point(s, 1.0, offset);
    let mut visible: Vec<(f64, f64)> = vec![(0.0, 1.0)];
    for other in field.candidates(((s.a + s.b) * 0.5).truncate()) {
        if (other.a == s.a && other.b == s.b) || (other.a == s.b && other.b == s.a) {
            continue;
        }
        if other.is_parking() {
            if let Some((lo, hi)) = crate::road_parking::mouth(a, b, other.a) {
                visible = visible
                    .into_iter()
                    .flat_map(|(start, end)| {
                        [(start, end.min(lo)), (start.max(hi), end)]
                            .into_iter()
                            .filter(|&(a, b)| b - a > 1e-6)
                    })
                    .collect();
            }
            continue;
        }
        let shared_a = other.a.distance(s.a).min(other.b.distance(s.a)) < 0.02;
        let shared_b = other.a.distance(s.b).min(other.b.distance(s.b)) < 0.02;
        if (shared_a || shared_b)
            && (s.b - s.a)
                .truncate()
                .normalize()
                .dot((other.b - other.a).truncate().normalize())
                .abs()
                > 0.999999
        {
            continue;
        }
        if (shared_a && !ribbon.branches[0]) || (shared_b && !ribbon.branches[1]) {
            continue;
        }
        let delta = (other.b - other.a).truncate();
        let length = delta.length();
        let u = delta / length;
        let v = DVec2::new(u.y, -u.x);
        let qa = (a - other.a).truncate();
        let qb = (b - other.a).truncate();
        let za = a.z - other.a.z - qa.dot(u) * (other.b.z - other.a.z) / length;
        let zb = b.z - other.a.z - qb.dot(u) * (other.b.z - other.a.z) / length;
        let mut lo = 0.0_f64;
        let mut hi = 1.0_f64;
        for (start, finish, minimum, maximum) in [
            (qa.dot(u), qb.dot(u), -0.02, length + 0.02),
            (qa.dot(v), qb.dot(v), -width, width),
            (za, zb, -0.25, 0.25),
        ] {
            let d = finish - start;
            if d.abs() < 1e-9 {
                if start < minimum || start > maximum {
                    hi = -1.0;
                    break;
                }
            } else {
                let x = (minimum - start) / d;
                let y = (maximum - start) / d;
                lo = lo.max(x.min(y));
                hi = hi.min(x.max(y));
            }
        }
        if hi <= lo {
            continue;
        }
        visible = visible
            .into_iter()
            .flat_map(|(start, end)| {
                [(start, end.min(lo)), (start.max(hi), end)]
                    .into_iter()
                    .filter(|&(a, b)| b - a > 1e-6)
            })
            .collect();
    }
    visible
}

fn deck(
    mesh: &mut Vec<Vertex>,
    s: Segment,
    ribbon: Ribbon,
    field: &Field,
    terrain: &impl Survey,
    piers: bool,
) -> Result<(), String> {
    let delta = s.b - s.a;
    let side = DVec3::new(delta.y, -delta.x, 0.0).normalize();
    let colour = if s.tunnel {
        [0.045, 0.048, -0.05]
    } else {
        [0.045, 0.048, 0.05]
    };
    let slab = |mesh: &mut Vec<Vertex>,
                width: f64,
                low: f64,
                high: f64,
                offset: f64,
                colour,
                start: f64,
                end: f64| {
        let a = ribbon.point(s, start, offset) + DVec3::Z * low;
        let b = ribbon.point(s, end, offset) + DVec3::Z * low;
        let la = ribbon.sides[0].lerp(ribbon.sides[1], start) * width;
        let lb = ribbon.sides[0].lerp(ribbon.sides[1], end) * width;
        let z = DVec3::Z * (high - low);
        road::quad(
            mesh,
            [a - la + z, a + la + z, b + lb + z, b - lb + z],
            colour,
        );
        road::quad(mesh, [a + la, a - la, b - lb, b + lb], [0.32, 0.31, 0.29]);
        road::quad(
            mesh,
            [a - la, a - la + z, b - lb + z, b - lb],
            [0.32, 0.31, 0.29],
        );
        road::quad(
            mesh,
            [a + la + z, a + la, b + lb, b + lb + z],
            [0.32, 0.31, 0.29],
        );
        if offset != 0.0 {
            if start > 1e-6 || !s.junction_a {
                road::quad(
                    mesh,
                    [a - la, a + la, a + la + z, a - la + z],
                    [0.32, 0.31, 0.29],
                );
            }
            if end < 1.0 - 1e-6 || !s.junction_b {
                road::quad(
                    mesh,
                    [b + lb, b - lb, b - lb + z, b + lb + z],
                    [0.32, 0.31, 0.29],
                );
            }
        }
    };
    slab(mesh, road::HALF_WIDTH, -1.0, 0.0, 0.0, colour, 0.0, 1.0);
    // Low flush edge markings, not a parapet across junction mouths.
    let marking = if s.tunnel {
        [0.65, 0.65, -0.61]
    } else {
        [0.65, 0.65, 0.61]
    };
    let paint =
        |mesh: &mut Vec<Vertex>, start: f64, end: f64, offset: f64, width: f64, height: f64| {
            let a = ribbon.point(s, start, offset) + DVec3::Z * height;
            let b = ribbon.point(s, end, offset) + DVec3::Z * height;
            let la = ribbon.sides[0].lerp(ribbon.sides[1], start) * width;
            let lb = ribbon.sides[0].lerp(ribbon.sides[1], end) * width;
            road::quad(mesh, [a - la, a + la, b + lb, b - lb], marking);
            // Paint is flat, so its normal lane carries each corner's signed
            // half-width instead; road.wgsl widens sub-pixel strips with it.
            let n = mesh.len() - 6;
            for (k, half) in [(0, -la), (1, la), (2, lb), (3, -lb)] {
                let half = half.as_vec3().to_array();
                for slot in match k {
                    0 => vec![0, 3],
                    1 => vec![1],
                    2 => vec![2, 4],
                    _ => vec![5],
                } {
                    mesh[n + slot].normal = half;
                }
            }
        };
    for offset in [-4.7, 4.7] {
        for (start, end) in exposed(ribbon, s, field, offset, 5.1) {
            paint(mesh, start, end, offset, 0.10, 0.025);
        }
    }
    // Retain edge protection right up to each real opening, also at village
    // approaches. Excavation floor depth is not a measure of the fall below.
    if !s.tunnel {
        for offset in [-4.82, 4.82] {
            for (start, end) in exposed(ribbon, s, field, offset, 4.82) {
                slab(mesh, 0.18, 0.0, 0.9, offset, [0.32, 0.31, 0.29], start, end);
            }
        }
    }
    let [sa, sb] = ribbon.station;
    let length = delta.length();
    let from = if ribbon.branches[0] {
        (7.0 / length).min(1.0)
    } else {
        0.0
    };
    let to = if ribbon.branches[1] {
        (1.0 - 7.0 / length).max(0.0)
    } else {
        1.0
    };
    for dash in (sa.min(sb) / 9.0).floor() as i64..=(sa.max(sb) / 9.0).ceil() as i64 {
        let a = (dash as f64 * 9.0 - sa) / (sb - sa);
        let b = (dash as f64 * 9.0 + 3.0 - sa) / (sb - sa);
        let start = a.min(b).max(from);
        let end = a.max(b).min(to);
        if end > start {
            for (lo, hi) in exposed(ribbon, s, field, 0.0, 5.1) {
                let a = start.max(lo);
                let b = end.min(hi);
                if b > a {
                    paint(mesh, a, b, 0.0, 0.08, 0.035);
                }
            }
        }
    }
    // The last metres at villages meet the ground; longer spans retain legs.
    let p = (s.a + s.b) * 0.5;
    if piers && s.floor_depth > 1.0 {
        for offset in [-3.0, 3.0] {
            let p = p + side * offset;
            let foot = if s.tunnel {
                p.z - 8.0
            } else {
                let direction = field.frame.world(p).normalize();
                let ground = terrain.ground(direction)?.height;
                field
                    .frame
                    .local(direction * (field.frame.radius + field.height(direction, ground)))
                    .z
                    - 1.0
            };
            road::box_mesh(
                mesh,
                DVec3::new(p.x - 0.45, p.y - 0.45, foot),
                p + DVec3::new(0.45, 0.45, -1.0),
                [0.32, 0.31, 0.29],
            );
        }
    }
    Ok(())
}

fn liner(
    mesh: &mut Vec<Vertex>,
    mut s: Segment,
    sides: [DVec3; 2],
    junctions: [bool; 2],
    portal_a: bool,
    portal_b: bool,
) {
    let d = s.b - s.a;
    let clip = junctions.iter().any(|&b| b);
    if clip {
        // Closed convex bore ends are needed at turns: merely intersecting
        // open tubes leaves a triangular sky leak on the outside corner.
        // Extend past our own 0.5m CSG end, so the union shader can remove
        // caps inside the continuation without discarding this bore's cap.
        let extension = d.normalize() * 0.55;
        if junctions[0] && !portal_a {
            s.a -= extension;
        }
        if junctions[1] && !portal_b {
            s.b += extension;
        }
    }
    let side = DVec3::new(d.y, -d.x, 0.0).normalize();
    let colour = [0.24, if clip { -0.235 } else { 0.235 }, -0.22];
    for arch in crate::road_tunnel::ARCH.windows(2) {
        let p = |base: DVec3, v: DVec2| {
            base + (if base == s.a { sides[0] } else { sides[1] }) * v.x + DVec3::Z * v.y
        };
        road::quad(
            mesh,
            [
                p(s.a, arch[1]),
                p(s.a, arch[0]),
                p(s.b, arch[0]),
                p(s.b, arch[1]),
            ],
            colour,
        );
        let outer = |v: DVec2| {
            DVec2::new(
                v.x + v.x.signum() * 0.8,
                v.y + if v.y < 0.0 { -0.8 } else { 0.8 },
            )
        };
        road::quad(
            mesh,
            [
                p(s.a, outer(arch[0])),
                p(s.a, outer(arch[1])),
                p(s.b, outer(arch[1])),
                p(s.b, outer(arch[0])),
            ],
            [-0.32, -0.31, -0.29],
        );
        for (base, portal) in [(s.a, portal_a), (s.b, portal_b)] {
            if portal {
                let q = [
                    p(base, arch[0]),
                    p(base, arch[1]),
                    p(base, outer(arch[1])),
                    p(base, outer(arch[0])),
                ];
                road::quad(mesh, q, [0.32, 0.31, 0.29]);
                road::quad(mesh, [q[3], q[2], q[1], q[0]], [0.32, 0.31, 0.29]);
            }
        }
    }
    if clip {
        for (end, (base, portal)) in [(s.a, portal_a), (s.b, portal_b)].into_iter().enumerate() {
            if portal || !junctions[end] {
                continue;
            }
            let arch = crate::road_tunnel::ARCH;
            let centre = base + DVec3::Z * 1.5;
            for i in 0..arch.len() {
                let a = base + side * arch[i].x + DVec3::Z * arch[i].y;
                let q = arch[(i + 1) % arch.len()];
                let b = base + side * q.x + DVec3::Z * q.y;
                for points in [[centre, a, b], [centre, b, a]] {
                    let normal = (points[1] - points[0])
                        .cross(points[2] - points[0])
                        .normalize();
                    for p in points {
                        mesh.push(Vertex {
                            lighting: 0,
                            position: p.as_vec3().to_array(),
                            normal: normal.as_vec3().to_array(),
                            colour,
                        });
                    }
                }
            }
        }
    }
    road::quad(
        mesh,
        [
            s.a - side * 6.5 - DVec3::Z * 6.0,
            s.a + side * 6.5 - DVec3::Z * 6.0,
            s.b + side * 6.5 - DVec3::Z * 6.0,
            s.b - side * 6.5 - DVec3::Z * 6.0,
        ],
        colour,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn road_network_coastal_caps_remain_bores_without_local_ocean_nodes() {
        struct Beach;
        impl Survey for Beach {
            fn ground(&self, d: DVec3) -> Result<Ground, String> {
                let ocean = d.z * 4e6 > 100.0;
                Ok(Ground {
                    height: if ocean { -10.0 } else { 10.0 },
                    ocean,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(vec![])
            }
        }
        let p = Profile {
            directions: [-100.0, 0.0, 100.0]
                .map(|x| DVec3::new(4e6, x, 0.0).normalize().to_array())
                .to_vec(),
            heights_meters: vec![-4.5; 3],
            segments: vec![[0, 1], [1, 2]],
            maximum_grade: 0.0,
            maximum_grade_change_per_meter: 0.0,
            maximum_deck_above_ground_meters: 0.0,
            minimum_sampled_undersea_roof_cover_meters: None,
            undersea_segments: 0,
        };
        let key = TileKey {
            face: CubeFace::PositiveX,
            level: 8,
            x: 128,
            y: 128,
        };
        assert_eq!(
            classify_tunnels(&p, key, 4e6, &Beach).unwrap(),
            [true, true]
        );
    }

    #[test]
    fn road_network_rejects_a_flooded_tunnel_mouth_instead_of_faking_cover() {
        struct Shore;
        impl Survey for Shore {
            fn ground(&self, d: DVec3) -> Result<Ground, String> {
                Ok(Ground {
                    height: if d.y < 0.0 { -10.0 } else { 10.0 },
                    ocean: d.y < 0.0,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(vec![])
            }
        }
        let p = Profile {
            directions: [-400.0, -200.0, 0.0, 200.0]
                .map(|x| DVec3::new(4e6, x, 0.0).normalize().to_array())
                .to_vec(),
            heights_meters: vec![-40.0, -40.0, -40.0, 0.0],
            segments: vec![[0, 1], [1, 2], [2, 3]],
            maximum_grade: 0.2,
            maximum_grade_change_per_meter: 0.001,
            maximum_deck_above_ground_meters: 0.0,
            minimum_sampled_undersea_roof_cover_meters: Some(21.0),
            undersea_segments: 2,
        };
        let key = TileKey {
            face: CubeFace::PositiveX,
            level: 8,
            x: 128,
            y: 128,
        };
        assert!(
            classify_tunnels(&p, key, 4e6, &Shore)
                .unwrap_err()
                .contains("dry inland approach")
        );
    }

    #[test]
    fn road_network_high_cliff_mouth_is_not_a_submerged_beach_approach() {
        struct Cliff;
        impl Survey for Cliff {
            fn ground(&self, d: DVec3) -> Result<Ground, String> {
                let ocean = d.z * 4e6 > 50.0;
                Ok(Ground {
                    height: if ocean {
                        -10.0
                    } else if d.y * 4e6 < -100.0 {
                        100.0
                    } else if d.y * 4e6 < 100.0 {
                        66.422
                    } else {
                        60.0
                    },
                    ocean,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(vec![])
            }
        }
        let p = Profile {
            directions: [-400.0, -200.0, 0.0, 200.0]
                .map(|x| DVec3::new(4e6, x, 0.0).normalize().to_array())
                .to_vec(),
            heights_meters: vec![48.0; 4],
            segments: vec![[0, 1], [1, 2], [2, 3]],
            maximum_grade: 0.0,
            maximum_grade_change_per_meter: 0.0,
            maximum_deck_above_ground_meters: 8.0,
            minimum_sampled_undersea_roof_cover_meters: None,
            undersea_segments: 0,
        };
        let key = TileKey {
            face: CubeFace::PositiveX,
            level: 8,
            x: 128,
            y: 128,
        };
        assert!(crate::road_network_profile::near_coast(DVec3::X, 4e6, &Cliff).unwrap());
        assert_eq!(
            classify_tunnels(&p, key, 4e6, &Cliff).unwrap(),
            [true, true, false]
        );
    }

    #[test]
    #[ignore = "asset-backed coastal portals at both ends of the long sea bore; no GPU"]
    fn road_network_coastal_portals_are_inland_and_above_water() {
        struct Fine(crate::road_network_survey::Surface);
        impl Survey for Fine {
            fn ground(&self, d: DVec3) -> Result<Ground, String> {
                self.0.engineering_ground(d)
            }
            fn villages(&self, k: TileKey) -> Result<Vec<DVec3>, String> {
                self.0.villages(k)
            }
            fn settlement_core_at(&self, d: DVec3) -> Result<bool, String> {
                self.0.settlement_core_at(d)
            }
            fn village_access_height(&self, g: Ground) -> f64 {
                g.height + 0.25
            }
        }
        let terrain = Fine(
            crate::road_network_survey::Surface::load(&crate::find_default_outmap().unwrap())
                .unwrap(),
        );
        let backbone = terrain.0.backbone().unwrap();
        let mut portals = 0;
        let mut gateways: Vec<(DVec3, f64, bool)> = Vec::new();
        for y in [152, 151] {
            let key = TileKey {
                face: CubeFace::PositiveX,
                level: 8,
                x: 152,
                y,
            };
            let plan = prepare_local(&backbone, key, 4e6, &terrain).unwrap();
            let p = &plan.profile;
            let mut status = vec![(false, false); p.directions.len()];
            for (e, &[a, b]) in p.segments.iter().enumerate() {
                for i in [a, b] {
                    if plan.tunnels[e] {
                        status[i].0 = true;
                    } else {
                        status[i].1 = true;
                    }
                }
            }
            for (t, &i) in plan
                .region
                .terminals
                .iter()
                .zip(&plan.region.terminal_nodes)
            {
                if matches!(t.kind, TerminalKind::Gateway { .. }) {
                    let d = DVec3::from_array(p.directions[i]);
                    for &(other, h, covered) in &gateways {
                        if d.distance_squared(other) < 1e-20 {
                            assert!(
                                (p.heights_meters[i] - h).abs() < 1e-6,
                                "shared sea gateway height"
                            );
                            assert!(
                                status[i].0 && !status[i].1 && covered,
                                "bore must continue through the gateway"
                            );
                        }
                    }
                    gateways.push((d, p.heights_meters[i], status[i].0 && !status[i].1));
                }
            }
            for (i, &(covered, open)) in status.iter().enumerate() {
                if !covered || !open {
                    continue;
                }
                let d = DVec3::from_array(p.directions[i]);
                let h = p.heights_meters[i];
                eprintln!(
                    "coastal portal {key:?}/{i}: deck={h:.3} ground={:.3} direction={d:?}",
                    terrain.ground(d).unwrap().height
                );
                // Keep the entire mouth and approach clear of the shoreline,
                // not merely the centreline point on the landward side.
                let u = d.cross(DVec3::Z).normalize();
                let v = d.cross(u);
                let mut wet = false;
                for r in [25.0, 50.0, 100.0, 150.0] {
                    for a in 0..72 {
                        let angle = a as f64 * std::f64::consts::TAU / 72.0;
                        let q = (d + (u * angle.cos() + v * angle.sin()) * (r / 4e6)).normalize();
                        wet |= terrain.ground(q).unwrap().ocean;
                    }
                }
                assert!(
                    h >= 5.0 && !wet,
                    "coastal portal is flooded or too close to water"
                );
                portals += 1;
            }
            assert!(p.maximum_grade <= 0.25);
            assert!(
                p.maximum_grade_change_per_meter
                    <= crate::road_vertical::MAX_GRADE_CHANGE_PER_METER
            );
            assert!(plan.tunnels.iter().any(|&t| t));
        }
        assert!(portals >= 2);
    }

    #[test]
    fn road_tunnel_preference_budget_preserves_survey_contract_and_stops_speculation() {
        struct Flat;
        impl Survey for Flat {
            fn ground(&self, _: DVec3) -> Result<road_network::Ground, String> {
                Ok(road_network::Ground {
                    height: 1000.0,
                    ocean: false,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
            fn village_access_height(&self, g: road_network::Ground) -> f64 {
                g.height + 0.25
            }
            fn settlement_core_at(&self, _: DVec3) -> Result<bool, String> {
                Ok(true)
            }
        }
        let trial = TunnelPreference {
            terrain: &Flat,
            samples: std::cell::Cell::new(0),
            last_samples: std::cell::Cell::new(0),
            quiet_checks: std::cell::Cell::new(0),
        };
        let g = trial.ground(DVec3::Z).unwrap();
        assert_eq!(trial.village_access_height(g), 1000.25);
        assert!(trial.settlement_core_at(DVec3::Z).unwrap());
        assert!(!trial.cancelled());
        for _ in 0..PREFERRED_QUIET_CHECKS {
            trial.cancelled();
        }
        assert!(trial.cancelled());
        assert!(trial.ground(DVec3::Z).is_err());
        trial.quiet_checks.set(0);
        trial.samples.set(PREFERRED_SURVEY_SAMPLES);
        assert!(trial.cancelled());
    }

    use crate::road_network::{Backbone, Ground};
    use planet_coretypes::CubeFace;
    struct Flat;
    impl Survey for Flat {
        fn village_access_height(&self, g: Ground) -> f64 {
            g.height + 0.25
        }
        fn ground(&self, _: DVec3) -> Result<Ground, String> {
            Ok(Ground {
                height: 1000.0,
                ocean: false,
            })
        }
        fn villages(&self, k: TileKey) -> Result<Vec<DVec3>, String> {
            Ok(vec![
                road_network::direction(k, 0.4, 0.4),
                road_network::direction(k, 0.6, 0.6),
            ])
        }
    }
    fn test_road(a: DVec3, b: DVec3) -> Road {
        let segment = Segment {
            a,
            b,
            tunnel: false,
            reach: 64.0,
            floor_depth: 6.0,
            junction_a: false,
            junction_b: false,
        };
        Road {
            light_station: [0.0, (b - a).length()],
            segment,
            piers: false,
            near: true,
            portal_a: false,
            portal_b: false,
            ribbon: Ribbon::straight(segment),
        }
    }
    #[test]
    fn curved_ribbons_share_wall_and_deck_vertices_and_exact_walking_floor() {
        let a = DVec3::new(0.0, 0.0, 1000.0);
        let b = DVec3::new(20.0, 0.0, 1004.0);
        let c = b + DVec3::new(
            20.0 * 8.0_f64.to_radians().cos(),
            20.0 * 8.0_f64.to_radians().sin(),
            4.0,
        );
        let mut roads = [test_road(a, b), test_road(b, c)];
        join_ribbons(&mut roads).unwrap();
        for offset in [-5.0, -4.82, 4.82, 5.0] {
            assert!(
                roads[0]
                    .ribbon
                    .point(roads[0].segment, 1.0, offset)
                    .distance(roads[1].ribbon.point(roads[1].segment, 0.0, offset))
                    < 1e-9
            );
        }
        let mut field = Field::build(
            Frame::new(DVec3::Z, 4e6).unwrap(),
            roads.iter().map(|r| r.segment).collect(),
        )
        .unwrap();
        field
            .set_deck_sections(roads.iter().map(|r| r.ribbon.sides).collect())
            .unwrap();
        for road in roads {
            assert!(
                !road.ribbon.branches.iter().any(|&b| b),
                "ordinary smooth joins need no intersecting bore caps"
            );
            let s = road.segment;
            let q =
                [-5.0, 5.0].map(|o| [road.ribbon.point(s, 0.0, o), road.ribbon.point(s, 1.0, o)]);
            for [a, b, c] in [[q[0][0], q[1][0], q[1][1]], [q[0][0], q[1][1], q[0][1]]] {
                for i in 1..10 {
                    for j in 1..10 - i {
                        let p = a + (b - a) * (i as f64 / 10.0) + (c - a) * (j as f64 / 10.0);
                        let world = field.frame.world(p);
                        let h = world.length() - field.frame.radius;
                        assert!(
                            (field.deck_height(world.normalize(), h + 2.0).unwrap() - h).abs()
                                < 1e-6
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn photographed_branch_shares_its_through_deck_wall_and_walking_floor() {
        let o = DVec3::Z * 1000.0;
        // Local arm vectors from manual/1792415387-1209990: a 2.14 degree
        // through bend at a fork left a 19 cm wedge in the outer deck edge.
        let mut roads = [
            test_road(o + DVec3::new(-16.101957, 1.020671, -0.027389), o),
            test_road(o, o + DVec3::new(16.052621, -1.621168, 0.026859)),
            test_road(o, o + DVec3::new(12.310280, -10.429397, 0.014792)),
        ];
        join_ribbons(&mut roads).unwrap();
        assert!(roads.iter().all(|r| r.ribbon.branches.contains(&true)));
        for offset in [-5.0, -4.82, -4.7, 4.7, 4.82, 5.0] {
            let a = roads[0].ribbon.point(roads[0].segment, 1.0, offset);
            let b = roads[1].ribbon.point(roads[1].segment, 0.0, offset);
            assert!(
                a.distance(b) < 1e-9,
                "fork seam at offset {offset}: {} m",
                a.distance(b)
            );
        }
        let mut field = Field::build(
            Frame::new(DVec3::Z, 4e6).unwrap(),
            roads.iter().map(|r| r.segment).collect(),
        )
        .unwrap();
        field
            .set_deck_sections(roads.iter().map(|r| r.ribbon.sides).collect())
            .unwrap();
        for i in [0, 1] {
            let r = &roads[i];
            let wall = exposed(r.ribbon, r.segment, &field, -4.82, 4.82);
            assert_eq!(
                wall,
                vec![(0.0, 1.0)],
                "outside wall must reach the shared seam"
            );
            let q = [-5.0, 5.0].map(|o| {
                [
                    r.ribbon.point(r.segment, 0.0, o),
                    r.ribbon.point(r.segment, 1.0, o),
                ]
            });
            // Sample the outer triangle beside the photographed crack, away
            // from overlapping side-road triangles (whose higher floor wins).
            let [a, b, c] = [q[0][0], q[1][1], q[0][1]];
            for weights in [[0.001, 0.001], [0.001, 0.5], [0.001, 0.998]] {
                let p = a + (b - a) * weights[0] + (c - a) * weights[1];
                let world = field.frame.world(p);
                let h = world.length() - field.frame.radius;
                assert!((field.deck_height(world.normalize(), h + 2.0).unwrap() - h).abs() < 1e-6);
            }
        }
        let side = &roads[2];
        assert!(
            exposed(side.ribbon, side.segment, &field, 4.82, 4.82)[0].0 > 0.0,
            "the side road mouth must remain open"
        );
    }

    #[test]
    fn residency_crop_does_not_reclassify_a_fork_as_a_sharp_bend() {
        let o = DVec3::Z * 1000.0;
        let mut roads = [
            test_road(o, o + DVec3::Y * 20.0),
            test_road(o, o + DVec3::X * 20.0),
        ];
        // The third arm exists in the plan but lies beyond the residency crop.
        roads[0].ribbon.branches[0] = true;
        join_ribbons(&mut roads).unwrap();
        assert!(roads.iter().all(|r| r.ribbon.branches[0]));
        let mut invalid = [
            test_road(o, o + DVec3::Y * 20.0),
            test_road(o, o + DVec3::X * 20.0),
        ];
        assert!(join_ribbons(&mut invalid).is_err());
    }
    #[test]
    fn junction_walls_stop_at_the_opening_not_at_the_previous_station() {
        let origin = DVec3::Z * 1000.0;
        let mut roads = [
            test_road(origin, origin + DVec3::Y * 20.0),
            test_road(origin, origin - DVec3::Y * 20.0),
            test_road(origin, origin + DVec3::X * 20.0),
        ];
        join_ribbons(&mut roads).unwrap();
        let field = Field::build(
            Frame::new(DVec3::Z, 4e6).unwrap(),
            roads.iter().map(|r| r.segment).collect(),
        )
        .unwrap();
        let r = &roads[0];
        let outer = exposed(r.ribbon, r.segment, &field, -4.82, 4.82);
        assert_eq!(
            outer,
            vec![(0.0, 1.0)],
            "uninterrupted back of T needs a complete wall"
        );
        let mouth = exposed(r.ribbon, r.segment, &field, 4.82, 4.82);
        assert_eq!(mouth.len(), 1);
        assert!((mouth[0].0 - 4.82 / 20.0).abs() < 1e-8);
        assert_eq!(mouth[0].1, 1.0);
    }
    #[test]
    fn curve_sampling_does_not_multiply_supports_or_restart_the_dash_phase() {
        let mut roads: Vec<_> = (0..80)
            .map(|i| {
                test_road(
                    DVec3::new(i as f64 * 2.0, 0.0, 1000.0),
                    DVec3::new((i + 1) as f64 * 2.0, 0.0, 1000.0),
                )
            })
            .collect();
        join_ribbons(&mut roads).unwrap();
        assert_eq!(roads.iter().filter(|r| r.piers).count(), 4);
        for pair in roads.windows(2) {
            assert_eq!(pair[0].ribbon.station[1], pair[1].ribbon.station[0]);
        }
    }
    #[test]
    fn network_junction_does_not_remove_the_entire_approach_parapet() {
        let s = Segment {
            a: DVec3::new(0.0, 0.0, 1000.0),
            b: DVec3::new(0.0, 20.0, 1000.0),
            tunnel: false,
            reach: 64.0,
            floor_depth: 6.0,
            junction_a: true,
            junction_b: false,
        };
        let field = Field::build(Frame::new(DVec3::X, 4e6).unwrap(), vec![s]).unwrap();
        let mut mesh = Vec::new();
        deck(&mut mesh, s, Ribbon::straight(s), &field, &Flat, false).unwrap();
        assert!(
            mesh.iter().any(|v| v.position[2] > 1000.5),
            "a junction at one end must not remove both outside walls for a whole span"
        );
    }

    #[test]
    fn network_preparation_rounds_a_right_angle_before_height_fitting() {
        use crate::road_network::{Node, RoadEdge, Terminal, TerminalKind};
        let r = 4e6;
        let dirs = [
            DVec3::new(1.0, -500.0 / r, 0.0).normalize(),
            DVec3::X,
            DVec3::new(1.0, 0.0, 500.0 / r).normalize(),
        ];
        let region = Region {
            key: TileKey {
                face: CubeFace::PositiveX,
                level: 8,
                x: 128,
                y: 128,
            },
            village_count: 0,
            terminals: [0, 2]
                .into_iter()
                .map(|i| Terminal {
                    direction: dirs[i].to_array(),
                    kind: TerminalKind::Gateway { link: i },
                })
                .collect(),
            terminal_nodes: vec![0, 2],
            nodes: dirs
                .iter()
                .map(|d| Node {
                    direction: d.to_array(),
                    ground_meters: 1000.0,
                    ocean: false,
                })
                .collect(),
            edges: [(0, 1), (1, 2)]
                .into_iter()
                .map(|(a, b)| RoadEdge {
                    a,
                    b,
                    requires_undersea_tunnel: false,
                })
                .collect(),
            junction_count: 0,
            length_meters: 1000.0,
            construction_cost_proxy: 1000.0,
            lattice_survey_samples: 0,
        };
        let plan = prepare(region, r, &Flat).unwrap();
        let p = &plan.profile;
        let points: Vec<_> = p
            .directions
            .iter()
            .map(|d| DVec3::from_array(*d) * r)
            .collect();
        let mut adjacency = vec![Vec::new(); points.len()];
        for &[a, b] in &p.segments {
            adjacency[a].push(b);
            adjacency[b].push(a);
        }
        for (i, adjacent) in adjacency.iter().enumerate() {
            if let &[a, b] = adjacent.as_slice() {
                let incoming = (points[i] - points[a]).normalize();
                let outgoing = (points[b] - points[i]).normalize();
                assert!(
                    incoming.dot(outgoing).clamp(-1.0, 1.0).acos() < 0.142,
                    "a sampled bend must not retain an instantaneous right angle"
                );
            }
        }
    }
    #[test]
    fn network_batches_stop_at_spatial_jumps_and_near_cache_boundaries() {
        let road = |x: f64, near| Road {
            light_station: [0.0, 10.0],
            segment: Segment {
                a: DVec3::new(x, 0.0, 1000.0),
                b: DVec3::new(x + 20.0, 0.0, 1000.0),
                tunnel: false,
                reach: 64.0,
                floor_depth: 6.0,
                junction_a: false,
                junction_b: false,
            },
            piers: false,
            near,
            portal_a: false,
            portal_b: false,
            ribbon: Ribbon::straight(Segment {
                a: DVec3::new(x, 0.0, 1000.0),
                b: DVec3::new(x + 20.0, 0.0, 1000.0),
                tunnel: false,
                reach: 64.0,
                floor_depth: 6.0,
                junction_a: false,
                junction_b: false,
            }),
        };
        let roads = [
            road(0.0, true),
            road(20.0, true),
            road(10000.0, true),
            road(10020.0, false),
        ];
        assert_eq!(chunk_end(&roads, 0), 2);
        assert_eq!(chunk_end(&roads, 2), 3);
        let roads: Vec<_> = (0..31).map(|i| road(i as f64 * 20.0, true)).collect();
        assert_eq!(chunk_end(&roads, 0), 25);
        assert_eq!(chunk_end(&roads, 25), 31);
    }

    #[test]
    fn a_junction_liner_does_not_pull_its_other_smooth_join_apart() {
        let mut s = test_road(DVec3::Z * 1000.0, DVec3::Y * 20.0 + DVec3::Z * 1000.0).segment;
        s.tunnel = true;
        let sides = [DVec3::X, DVec3::new(1.0, 4.0_f64.to_radians().tan(), 0.0)];
        let mut mesh = Vec::new();
        liner(&mut mesh, s, sides, [true, false], false, false);
        let caps: Vec<_> = mesh.iter().filter(|v| v.normal[1].abs() > 0.99).collect();
        assert_eq!(caps.len(), 42, "only the real junction needs a cap");
        assert!(caps.iter().all(|v| v.position[1] < 0.0));
        let last_y = 20.0 + 7.3 * 4.0_f64.to_radians().tan();
        assert!(
            mesh.iter()
                .all(|v| f64::from(v.position[1]) <= last_y + 1e-5)
        );
        for p in crate::road_tunnel::ARCH.map(|v| s.b + sides[1] * v.x + DVec3::Z * v.y) {
            assert!(
                mesh.iter()
                    .any(|v| DVec3::from_array(v.position.map(f64::from)).distance(p) < 1e-4)
            );
        }
    }

    #[test]
    fn network_bore_junction_closes_outer_corners_but_leaves_real_portals_open() {
        let s = Segment {
            a: DVec3::new(0.0, 0.0, 1000.0),
            b: DVec3::new(0.0, 20.0, 1000.0),
            tunnel: true,
            reach: 64.0,
            floor_depth: 6.0,
            junction_a: true,
            junction_b: true,
        };
        let mut mesh = Vec::new();
        liner(
            &mut mesh,
            s,
            Ribbon::straight(s).sides,
            [true, true],
            false,
            false,
        );
        let caps: Vec<_> = mesh.iter().filter(|v| v.normal[1].abs() > 0.99).collect();
        assert_eq!(caps.len(), 84);
        assert!(caps.iter().all(|v| v.colour[1] < 0.0 && v.colour[2] < 0.0));
        assert!(
            caps.iter()
                .all(|v| v.position[1] < 0.0 || v.position[1] > 20.0)
        );
        mesh.clear();
        liner(
            &mut mesh,
            s,
            Ribbon::straight(s).sides,
            [true, true],
            true,
            true,
        );
        assert!(
            mesh.iter()
                .all(|v| v.position[1] >= 0.0 && v.position[1] <= 20.0)
        );
    }

    #[test]
    fn network_mesh_preserves_every_village_and_ground_level_access() {
        let terrain = Flat;
        let r = 4e6;
        let backbone = Backbone::survey(8, r, &terrain).unwrap();
        let key = TileKey {
            face: CubeFace::PositiveX,
            level: 8,
            x: 128,
            y: 128,
        };
        let p = prepare(backbone.local(key, r, &terrain).unwrap(), r, &terrain).unwrap();
        assert_eq!(p.village_centres.len(), p.region.village_count);
        for (t, &i) in p.region.terminals.iter().zip(&p.region.terminal_nodes) {
            if matches!(t.kind, TerminalKind::Village) {
                assert!((p.profile.heights_meters[i] - 1000.25).abs() < 1e-6);
            }
        }
    }
}

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
    /// Original accepted centres, even though the road ends outside houses.
    pub village_centres: Vec<DVec3>,
}

impl Prepared {
    pub fn inspection_pose(&self, near: DVec3, radius: f64) -> Option<(DVec3, DVec3)> {
        let p = &self.profile;
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

pub fn prepare(mut region: Region, radius: f64, terrain: &impl Survey) -> Result<Prepared, String> {
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
                        + along * (crate::village::VILLAGE_RADIUS_METERS + 24.0 + extra))
                        .normalize();
                    if planet_coretypes::tile_key_for_direction(d, region.key.level) == region.key
                        && !terrain.ground(d)?.ocean
                        && !terrain.settlement_core_at(d)?
                    {
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
    let profile = match Profile::fit(&region, radius, terrain)? {
        Ok(profile) => profile,
        Err(_) => {
            let plan = crate::road_network_route::reroute(&region, radius, terrain)?
                .map_err(|e| format!("region {:?} engineering failed: {e:?}", region.key))?;
            region = plan.region;
            plan.profile
        }
    };
    Ok(Prepared {
        region,
        profile,
        village_centres: centres,
    })
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
    segment: Segment,
    piers: bool,
    near: bool,
    clip_liner: bool,
    portal_a: bool,
    portal_b: bool,
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
    let mut field_segments = Vec::new();
    let mut inspection = None;
    for plan in plans {
        let p = &plan.profile;
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
                sea |=
                    ocean[i] || terrain.settlement_core_at(DVec3::from_array(p.directions[i]))?;
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
            if distance < FIELD_RADIUS {
                field_segments.push(segment);
            }
            roads.push(Road {
                segment,
                piers: index % 2 == 0,
                near: distance < NEAR_RADIUS,
                clip_liner: joint(a) || joint(b),
                portal_a: portal_status[a] == (true, true),
                portal_b: portal_status[b] == (true, true),
            });
        }
    }
    let mut field = Field::build(frame, field_segments)?;
    field.set_geometry_window(eye.truncate(), NEAR_RADIUS - 500.0);
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
            let tangent = (s.b - s.a).normalize();
            road::far_segment(
                &mut far,
                s.a,
                s.b,
                tangent * road::HALF_WIDTH,
                tangent * road::HALF_WIDTH,
            );
            if road.near {
                deck(&mut near, s, &field, terrain, road.piers)?;
            }
            if s.tunnel {
                liner(
                    &mut structures,
                    s,
                    road.clip_liner,
                    road.portal_a,
                    road.portal_b,
                );
            }
            if road.near && !s.tunnel {
                banks(&mut structures, s, &field, terrain)?;
            }
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

fn deck(
    mesh: &mut Vec<Vertex>,
    s: Segment,
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
    let slab = |mesh: &mut Vec<Vertex>, width: f64, low: f64, high: f64, offset: f64, colour| {
        let [a, b] = [s.a, s.b].map(|p| p + side * offset + DVec3::Z * low);
        let l = side * width;
        let z = DVec3::Z * (high - low);
        road::quad(mesh, [a - l + z, a + l + z, b + l + z, b - l + z], colour);
        road::quad(mesh, [a + l, a - l, b - l, b + l], [0.32, 0.31, 0.29]);
        road::quad(
            mesh,
            [a - l, a - l + z, b - l + z, b - l],
            [0.32, 0.31, 0.29],
        );
        road::quad(
            mesh,
            [a + l + z, a + l, b + l, b + l + z],
            [0.32, 0.31, 0.29],
        );
    };
    slab(mesh, road::HALF_WIDTH, -1.0, 0.0, 0.0, colour);
    // Low flush edge markings, not a parapet across junction mouths.
    let marking = if s.tunnel {
        [0.65, 0.65, -0.61]
    } else {
        [0.65, 0.65, 0.61]
    };
    for offset in [-4.7, 4.7] {
        let a = s.a + side * offset + DVec3::Z * 0.025;
        let b = s.b + side * offset + DVec3::Z * 0.025;
        road::quad(
            mesh,
            [
                a - side * 0.10,
                a + side * 0.10,
                b + side * 0.10,
                b - side * 0.10,
            ],
            marking,
        );
    }
    if s.floor_depth >= 5.99 && !s.junction_a && !s.junction_b {
        for offset in [-4.82, 4.82] {
            slab(mesh, 0.18, 0.0, 0.9, offset, [0.32, 0.31, 0.29]);
        }
    }
    let length = delta.length();
    for start in (0..length.ceil() as usize).step_by(9) {
        let a = s.a + delta * (start as f64 / length);
        let b = s.a + delta * ((start as f64 + 3.0).min(length) / length);
        road::quad(
            mesh,
            [
                a - side * 0.08 + DVec3::Z * 0.035,
                a + side * 0.08 + DVec3::Z * 0.035,
                b + side * 0.08 + DVec3::Z * 0.035,
                b - side * 0.08 + DVec3::Z * 0.035,
            ],
            marking,
        );
    }
    // Rounded end pads close bent and branching deck mouths without barriers.
    for (p, junction) in [(s.a, s.junction_a), (s.b, s.junction_b)] {
        if junction {
            for i in 0..8 {
                let a = i as f64 * std::f64::consts::TAU / 8.0;
                let b = (i + 1) as f64 * std::f64::consts::TAU / 8.0;
                let normal = DVec3::Z;
                for point in [
                    p,
                    p + DVec3::new(a.cos(), a.sin(), 0.0) * 5.0,
                    p + DVec3::new(b.cos(), b.sin(), 0.0) * 5.0,
                ] {
                    mesh.push(Vertex {
                        position: point.as_vec3().to_array(),
                        normal: normal.as_vec3().to_array(),
                        colour,
                    });
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

fn liner(mesh: &mut Vec<Vertex>, mut s: Segment, clip: bool, portal_a: bool, portal_b: bool) {
    let d = s.b - s.a;
    if clip {
        // Closed convex bore ends are needed at turns: merely intersecting
        // open tubes leaves a triangular sky leak on the outside corner.
        // Extend past our own 0.5m CSG end, so the union shader can remove
        // caps inside the continuation without discarding this bore's cap.
        let extension = d.normalize() * 0.55;
        if !portal_a {
            s.a -= extension;
        }
        if !portal_b {
            s.b += extension;
        }
    }
    let side = DVec3::new(d.y, -d.x, 0.0).normalize();
    let colour = [0.24, if clip { -0.235 } else { 0.235 }, -0.22];
    for arch in crate::road_tunnel::ARCH.windows(2) {
        let p = |base: DVec3, v: DVec2| base + side * v.x + DVec3::Z * v.y;
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
        for (base, portal) in [(s.a, portal_a), (s.b, portal_b)] {
            if portal {
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

fn banks(
    mesh: &mut Vec<Vertex>,
    s: Segment,
    field: &Field,
    terrain: &impl Survey,
) -> Result<(), String> {
    let d = s.b - s.a;
    let side = DVec3::new(d.y, -d.x, 0.0).normalize();
    let offsets = [
        -s.reach, -48.0, -32.0, -24.0, -16.0, -9.0, 0.0, 9.0, 16.0, 24.0, 32.0, 48.0, s.reach,
    ];
    let mut strips = Vec::new();
    let mut any_cut = false;
    for offset in offsets {
        let mut pair = [(DVec3::ZERO, 0.0_f64); 2];
        for (i, base) in [s.a, s.b].into_iter().enumerate() {
            let p = base + side * offset;
            let direction = field.frame.world(p).normalize();
            let h = terrain.ground(direction)?.height;
            let cut = field.height(direction, h);
            any_cut |= h - cut > 0.25;
            pair[i] = (
                field
                    .frame
                    .local(direction * (field.frame.radius + cut + s.floor_depth.min(0.15))),
                (h - cut).max(0.0),
            );
        }
        strips.push(pair);
    }
    if any_cut {
        for p in strips.windows(2) {
            let points = [p[0][0], p[1][0], p[1][1], p[0][1]];
            if points.iter().all(|p| p.1 <= 0.25) {
                continue;
            }
            let normal = (points[1].0 - points[0].0)
                .cross(points[2].0 - points[0].0)
                .normalize()
                .as_vec3()
                .to_array();
            for i in [0, 1, 2, 0, 2, 3] {
                // Green magnitude carries interpolated excavation, without a
                // larger vertex stride or painting untouched ground brown.
                let (point, removed) = points[i];
                mesh.push(Vertex {
                    position: point.as_vec3().to_array(),
                    normal,
                    colour: [0.24, -(0.19 + removed.min(64.0) as f32 / 64.0), 0.13],
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn network_batches_stop_at_spatial_jumps_and_near_cache_boundaries() {
        let road = |x: f64, near| Road {
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
            clip_liner: false,
            portal_a: false,
            portal_b: false,
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
        liner(&mut mesh, s, true, false, false);
        let caps: Vec<_> = mesh.iter().filter(|v| v.normal[1].abs() > 0.99).collect();
        assert_eq!(caps.len(), 84);
        assert!(caps.iter().all(|v| v.colour[1] < 0.0 && v.colour[2] < 0.0));
        assert!(
            caps.iter()
                .all(|v| v.position[1] < 0.0 || v.position[1] > 20.0)
        );
        mesh.clear();
        liner(&mut mesh, s, true, true, true);
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

//! Worker-side tangent-continuous horizontal fillets, before height fitting.
//! The same sampled alignment drives decks, excavation, forests and collision.
use crate::{
    road_network::{self, Node, Region, RoadEdge, Survey, TerminalKind},
    road_network_field::Frame,
};
use glam::{DVec2, DVec3};

pub const TARGET_RADIUS: f64 = 100.0;
pub const MIN_RADIUS: f64 = 15.0;
// Keep the 6.5m bore miters within the existing 0.5m longitudinal air overlap.
pub(crate) const MAX_ANGLE: f64 = 8.0 * std::f64::consts::PI / 180.0;

pub struct Aligned {
    pub region: Region,
    pub seeds: Vec<(usize, f64)>,
    pub minimum_radius: Option<f64>,
}

fn cross(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}
fn left(a: DVec2) -> DVec2 {
    DVec2::new(-a.y, a.x)
}

fn fillet(points: &[DVec2], radius: f64) -> Result<(Vec<DVec2>, Option<f64>), String> {
    let mut p: Vec<DVec2> = Vec::new();
    for &point in points {
        if p.last().is_some_and(|last| last.distance(point) < 0.001) {
            continue;
        }
        while p.len() >= 2 {
            let a = (p[p.len() - 1] - p[p.len() - 2]).normalize();
            let b = (point - p[p.len() - 1]).normalize();
            if cross(a, b).abs() > 1e-7 || a.dot(b) < 0.999999 {
                break;
            }
            p.pop();
        }
        p.push(point);
    }
    if p.len() < 3 {
        return Ok((p, None));
    }
    let mut trims = vec![0.0; p.len()];
    let mut angles = vec![0.0; p.len()];
    for i in 1..p.len() - 1 {
        let u = (p[i] - p[i - 1]).normalize();
        let v = (p[i + 1] - p[i]).normalize();
        let angle = u.dot(v).clamp(-1.0, 1.0).acos();
        if angle > 170.0_f64.to_radians() {
            return Err(format!(
                "alignment has an un-drivable reversal at {:?} {:?} {:?}",
                p[i - 1],
                p[i],
                p[i + 1]
            ));
        }
        angles[i] = angle;
        trims[i] = radius * (angle * 0.5).tan();
    }
    // Share each straight run between its two curves, rather than letting
    // independently rounded corners overlap or reverse their station order.
    for i in 0..p.len() - 1 {
        let available = p[i].distance(p[i + 1]) * 0.999;
        let sum = trims[i] + trims[i + 1];
        if sum > available {
            let scale = available / sum;
            trims[i] *= scale;
            trims[i + 1] *= scale;
        }
    }
    let mut result = vec![p[0]];
    let mut minimum: Option<f64> = None;
    let straight = |end: DVec2, out: &mut Vec<DVec2>| {
        let start = *out.last().unwrap();
        let d = end - start;
        let length2 = d.length_squared();
        if length2 < 1e-12 {
            return;
        }
        // Retain straight surveyed stations for the router's height seeds.
        let mut interior: Vec<_> = points
            .iter()
            .filter_map(|&q| {
                let t = (q - start).dot(d) / length2;
                (t > 1e-6 && t < 1.0 - 1e-6 && q.distance(start + d * t) < 0.001).then_some((t, q))
            })
            .collect();
        interior.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, q) in interior {
            if out.last().unwrap().distance(q) > 0.001 {
                out.push(q);
            }
        }
        out.push(end);
    };
    for i in 1..p.len() - 1 {
        let angle = angles[i];
        if angle < 1e-6 {
            straight(p[i], &mut result);
            continue;
        }
        let r = trims[i] / (angle * 0.5).tan();
        if r < MIN_RADIUS - 0.001 {
            return Err(format!("bend radius {r:.2}m is below {MIN_RADIUS}m"));
        }
        minimum = Some(minimum.map_or(r, |old| old.min(r)));
        let u = (p[i] - p[i - 1]).normalize();
        let v = (p[i + 1] - p[i]).normalize();
        let sign = cross(u, v).signum();
        let a = p[i] - u * trims[i];
        let b = p[i] + v * trims[i];
        let centre = a + left(u) * (sign * r);
        let start_angle = (a.y - centre.y).atan2(a.x - centre.x);
        straight(a, &mut result);
        let count = (angle / MAX_ANGLE)
            .ceil()
            .max((angle * r / 20.0).ceil())
            .max(1.0) as usize;
        for j in 1..=count {
            let t = j as f64 / count as f64;
            let theta = start_angle + sign * angle * t;
            result.push(if j == count {
                b
            } else {
                centre + DVec2::new(theta.cos(), theta.sin()) * r
            });
        }
    }
    straight(*p.last().unwrap(), &mut result);
    Ok((result, minimum))
}

/// Gateways share a tangent as well as a point. Region-local smoothing must
/// not leave a new hard corner at a cube/region seam.
fn gateway_inward(region: &Region, node: usize) -> Option<DVec3> {
    if !region
        .terminals
        .iter()
        .zip(&region.terminal_nodes)
        .any(|(t, &i)| i == node && matches!(t.kind, TerminalKind::Gateway { .. }))
    {
        return None;
    }
    let d = DVec3::from_array(region.nodes[node].direction);
    let here = road_network::direction(region.key, 0.5, 0.5);
    let neighbour =
        planet_coretypes::tile_key_for_direction(d.lerp(here, -0.01).normalize(), region.key.level);
    if neighbour == region.key {
        return None;
    } // Interior synthetic terminals.
    let other = road_network::direction(neighbour, 0.5, 0.5);
    let delta = here - other;
    Some((delta - d * delta.dot(d)).normalize())
}

/// A village access is a side road, not a mandatory kink in the through road.
/// Move the shared hub a short way up its first approach, retaining the exact
/// access point/height as a leaf. Recheck terrain and fit the whole tree after.
pub fn separate_accesses(
    region: &mut Region,
    radius: f64,
    terrain: &impl Survey,
    mut heights: Option<&mut Vec<f64>>,
) -> Result<(), String> {
    for (terminal, &node) in region.terminals.iter().zip(&region.terminal_nodes) {
        if !matches!(terminal.kind, TerminalKind::Village) {
            continue;
        }
        let adjacent: Vec<_> = region
            .edges
            .iter()
            .filter_map(|e| {
                if e.a == node {
                    Some(e.b)
                } else if e.b == node {
                    Some(e.a)
                } else {
                    None
                }
            })
            .collect();
        if adjacent.len() < 2 {
            continue;
        }
        let d = DVec3::from_array(region.nodes[node].direction);
        let Some(toward) = adjacent
            .iter()
            .map(|&j| DVec3::from_array(region.nodes[j].direction))
            .find(|&q| q.distance(d) * radius > 0.001)
        else {
            continue;
        };
        let length = d.cross(toward).length().atan2(d.dot(toward)) * radius;
        let direction = d.lerp(toward, (80.0 / length).min(0.5)).normalize();
        let ground = terrain.ground(direction)?;
        let hub = region.nodes.len();
        region.nodes.push(Node {
            direction: direction.to_array(),
            ground_meters: ground.height,
            ocean: ground.ocean,
        });
        if let Some(h) = heights.as_mut() {
            let value = h[node];
            h.push(value);
        }
        for edge in &mut region.edges {
            if edge.a == node {
                edge.a = hub;
            }
            if edge.b == node {
                edge.b = hub;
            }
        }
        region.edges.push(RoadEdge {
            a: node,
            b: hub,
            requires_undersea_tunnel: ground.ocean || region.nodes[node].ocean,
        });
    }
    let mut degree = vec![0; region.nodes.len()];
    for e in &region.edges {
        degree[e.a] += 1;
        degree[e.b] += 1;
    }
    region.junction_count = degree.iter().filter(|&&d| d >= 3).count();
    Ok(())
}

pub fn align(
    region: &Region,
    radius: f64,
    terrain: &impl Survey,
    seeds: Option<&[f64]>,
    target: f64,
) -> Result<Aligned, String> {
    let frame = Frame::new(road_network::direction(region.key, 0.5, 0.5), radius)?;
    let project = |d: DVec3| {
        DVec2::new(d.dot(frame.across), d.dot(frame.forward)) * (radius / d.dot(frame.up))
    };
    let direction =
        |p: DVec2| (frame.up * radius + frame.across * p.x + frame.forward * p.y).normalize();
    let mut adjacent = vec![Vec::new(); region.nodes.len()];
    for (i, e) in region.edges.iter().enumerate() {
        adjacent[e.a].push((e.b, i));
        adjacent[e.b].push((e.a, i));
    }
    let mut anchor: Vec<_> = adjacent.iter().map(|a| a.len() != 2).collect();
    for &i in &region.terminal_nodes {
        anchor[i] = true;
    }
    let mut map = vec![usize::MAX; anchor.len()];
    let mut nodes = Vec::new();
    let mut fitted_seeds = Vec::new();
    for (i, &is_anchor) in anchor.iter().enumerate() {
        if is_anchor {
            map[i] = nodes.len();
            nodes.push(region.nodes[i].clone());
            if let Some(s) = seeds {
                fitted_seeds.push((map[i], s[i]));
            }
        }
    }
    let mut visited = vec![false; region.edges.len()];
    let mut edges = Vec::new();
    let mut minimum: Option<f64> = None;
    let mut length = 0.0;
    for a in 0..anchor.len() {
        if !anchor[a] {
            continue;
        }
        for &(b, edge) in &adjacent[a] {
            if visited[edge] {
                continue;
            }
            let mut chain = vec![a];
            let mut previous = a;
            let mut current = b;
            let mut id = edge;
            loop {
                visited[id] = true;
                chain.push(current);
                if anchor[current] {
                    break;
                }
                let &(next, e) = adjacent[current]
                    .iter()
                    .find(|&&(j, _)| j != previous)
                    .ok_or("broken alignment chain")?;
                previous = current;
                current = next;
                id = e;
                if chain.len() > region.nodes.len() {
                    return Err("cyclic alignment chain".into());
                }
            }
            if terrain.cancelled() {
                return Err("road generation superseded".into());
            }
            let end = *chain.last().unwrap();
            let original: Vec<_> = chain
                .iter()
                .map(|&i| project(DVec3::from_array(region.nodes[i].direction)))
                .collect();
            let mut path = original.clone();
            let chain_length: f64 = path.windows(2).map(|p| p[0].distance(p[1])).sum();
            // Reserve a straight seam mouth, then round its transition into
            // the surveyed chain. Short connections retain a bounded stub.
            for reverse in [false, true] {
                let node = if reverse { end } else { a };
                if let Some(tangent) = gateway_inward(region, node).filter(|_| {
                    // A terminal may reach its physical road through zero-length
                    // gateway aliases. Count arms of that whole component, not
                    // just the terminal's (possibly coincident) first neighbour.
                    let mut seen = vec![false; region.nodes.len()];
                    let mut stack = vec![node];
                    let mut arms = 0;
                    while let Some(i) = stack.pop() {
                        if seen[i] {
                            continue;
                        }
                        seen[i] = true;
                        for &(j, _) in &adjacent[i] {
                            if DVec3::from_array(region.nodes[j].direction)
                                .distance(DVec3::from_array(region.nodes[i].direction))
                                * radius
                                < 0.001
                            {
                                stack.push(j);
                            } else {
                                arms += 1;
                            }
                        }
                    }
                    arms == 1
                }) {
                    if reverse {
                        path.reverse();
                    }
                    let stub = (chain_length * 0.2)
                        .clamp(30.0, 150.0)
                        .min(chain_length / 3.0);
                    if stub > 0.001 && path.len() > 1 {
                        let d = DVec3::from_array(region.nodes[node].direction);
                        let q = project((d * radius + tangent * stub).normalize());
                        let start = path[0];
                        while path.len() > 2 && path[1].distance(start) < stub * 1.5 {
                            path.remove(1);
                        }
                        path.insert(1, q);
                    }
                    if reverse {
                        path.reverse();
                    }
                }
            }
            let (curve, r) = fillet(&path, target)?;
            if let Some(r) = r {
                minimum = Some(minimum.map_or(r, |old| old.min(r)));
            }
            let mut last = map[a];
            for (j, &point) in curve.iter().enumerate().skip(1) {
                let next = if j + 1 == curve.len() {
                    map[end]
                } else {
                    let d = direction(point);
                    let g = terrain.ground(d)?;
                    if !g.height.is_finite() {
                        return Err("non-finite curve terrain".into());
                    }
                    let index = nodes.len();
                    nodes.push(Node {
                        direction: d.to_array(),
                        ground_meters: g.height,
                        ocean: g.ocean,
                    });
                    if let Some(s) = seeds {
                        let mut best = (f64::INFINITY, 0.0);
                        for (k, p) in original.windows(2).enumerate() {
                            let delta = p[1] - p[0];
                            let t = if delta.length_squared() > 1e-12 {
                                (point - p[0]).dot(delta) / delta.length_squared()
                            } else {
                                0.0
                            };
                            let t = t.clamp(0.0, 1.0);
                            let distance = point.distance_squared(p[0] + delta * t);
                            if distance < best.0 {
                                best = (distance, s[chain[k]] * (1.0 - t) + s[chain[k + 1]] * t);
                            }
                        }
                        fitted_seeds.push((index, best.1));
                    }
                    index
                };
                let da = DVec3::from_array(nodes[last].direction);
                let db = DVec3::from_array(nodes[next].direction);
                length += da.cross(db).length().atan2(da.dot(db)) * radius;
                edges.push(RoadEdge {
                    a: last,
                    b: next,
                    requires_undersea_tunnel: nodes[last].ocean || nodes[next].ocean,
                });
                last = next;
            }
            // Coincident gateway aliases still carry graph connectivity.
            if curve.len() < 2 && map[a] != map[end] {
                edges.push(RoadEdge {
                    a: map[a],
                    b: map[end],
                    requires_undersea_tunnel: nodes[map[a]].ocean,
                });
            }
        }
    }
    Ok(Aligned {
        region: Region {
            key: region.key,
            village_count: region.village_count,
            terminals: region.terminals.clone(),
            terminal_nodes: region.terminal_nodes.iter().map(|&i| map[i]).collect(),
            nodes,
            edges,
            junction_count: region.junction_count,
            length_meters: length,
            construction_cost_proxy: region.construction_cost_proxy,
            lattice_survey_samples: region.lattice_survey_samples,
        },
        seeds: fitted_seeds,
        minimum_radius: minimum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn right_angle_is_a_tangent_circle_not_a_cut_corner() {
        let (p, r) = fillet(
            &[DVec2::new(-500.0, 0.0), DVec2::ZERO, DVec2::new(0.0, 500.0)],
            100.0,
        )
        .unwrap();
        assert!((r.unwrap() - 100.0).abs() < 1e-8);
        assert_eq!(p[0], DVec2::new(-500.0, 0.0));
        assert_eq!(*p.last().unwrap(), DVec2::new(0.0, 500.0));
        for w in p.windows(3) {
            assert!(
                (w[1] - w[0])
                    .normalize()
                    .dot((w[2] - w[1]).normalize())
                    .clamp(-1.0, 1.0)
                    .acos()
                    <= MAX_ANGLE + 1e-8
            );
        }
    }
    #[test]
    fn adjacent_curves_share_space_and_never_reverse_the_path() {
        let (p, r) = fillet(
            &[
                DVec2::new(-200.0, 0.0),
                DVec2::ZERO,
                DVec2::new(0.0, 100.0),
                DVec2::new(200.0, 100.0),
            ],
            100.0,
        )
        .unwrap();
        assert!(r.unwrap() >= MIN_RADIUS);
        assert!(
            p.windows(2)
                .all(|w| w[1].x >= w[0].x - 1e-8 && w[1].y >= w[0].y - 1e-8)
        );
    }
    #[test]
    fn regional_gateway_tangent_survives_a_coincident_terminal_alias() {
        use crate::road_network::Ground;
        use planet_coretypes::{CubeFace, TileKey};
        struct Flat;
        impl Survey for Flat {
            fn ground(&self, _: DVec3) -> Result<Ground, String> {
                Ok(Ground {
                    height: 1000.0,
                    ocean: false,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
        }
        let key = TileKey {
            face: CubeFace::PositiveZ,
            level: 8,
            x: 209,
            y: 223,
        };
        let d = road_network::direction(key, 0.5, 1.0);
        let inside = road_network::direction(key, 0.8, 0.5);
        let region = Region {
            key,
            village_count: 0,
            junction_count: 0,
            length_meters: 0.0,
            construction_cost_proxy: 0.0,
            lattice_survey_samples: 0,
            nodes: [inside, d, d]
                .into_iter()
                .map(|v| Node {
                    direction: v.to_array(),
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
            terminals: vec![road_network::Terminal {
                direction: d.to_array(),
                kind: TerminalKind::Gateway { link: 0 },
            }],
            terminal_nodes: vec![2],
        };
        let expected = gateway_inward(&region, 2).unwrap();
        let aligned = align(&region, 4e6, &Flat, None, 100.0).unwrap();
        let i = aligned.region.terminal_nodes[0];
        let e = aligned
            .region
            .edges
            .iter()
            .find(|e| e.a == i || e.b == i)
            .unwrap();
        let j = if e.a == i { e.b } else { e.a };
        let q = DVec3::from_array(aligned.region.nodes[j].direction);
        let tangent = (q - d * q.dot(d)).normalize();
        assert!(
            tangent.dot(expected) > 0.999999,
            "{tangent:?}, expected {expected:?}"
        );
    }
    #[test]
    fn too_short_a_bend_is_rejected_not_published_as_a_hard_corner() {
        assert!(
            fillet(
                &[DVec2::new(-5.0, 0.0), DVec2::ZERO, DVec2::new(0.0, 5.0)],
                100.0
            )
            .is_err()
        );
    }
}

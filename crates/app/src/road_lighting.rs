//! Worker-baked wall fixtures and bounded, immutable local-light lists.
use crate::{
    road::{self, Vertex},
    road_network_field::{Field, Segment},
};
use glam::DVec3;
use std::collections::{HashMap, VecDeque};
use wgpu::util::DeviceExt;
pub const SPACING: f64 = 20.0;
pub const RADIUS: f64 = 34.0;
pub const MAX_LIGHTS: usize = 24;
pub struct Span {
    pub segment: Segment,
    pub sides: [DVec3; 2],
    pub station: [f64; 2],
}
pub struct Lighting {
    pub data: Vec<[f32; 4]>,
    pub fixtures: Vec<Vec<Vertex>>,
}
struct Lamp {
    position: DVec3,
    owner: usize,
}
impl Lighting {
    pub fn build(spans: &[Span], field: &Field) -> Self {
        let mut lamps = Vec::new();
        let mut fixtures = vec![Vec::new(); spans.len()];
        let mut endpoints = HashMap::<_, Vec<usize>>::new();
        let key = |p: DVec3| p.to_array().map(|v| (v * 1000.0).round() as i64);
        for (i, s) in spans.iter().enumerate() {
            for p in [s.segment.a, s.segment.b] {
                endpoints.entry(key(p)).or_default().push(i);
            }
            let [a, b] = s.station;
            let lo = a.min(b);
            let hi = a.max(b);
            for n in (lo / SPACING).ceil() as i64..(hi / SPACING).ceil() as i64 {
                let station = n as f64 * SPACING;
                let t = ((station - a) / (b - a)).clamp(0.0, 1.0);
                let side = s.sides[0].lerp(s.sides[1], t).normalize();
                let forward = DVec3::Z.cross(side).normalize();
                let direction = (if n.rem_euclid(2) == 0 { -1.0 } else { 1.0 })
                    * (if b > a { 1.0 } else { -1.0 });
                let position =
                    s.segment.a.lerp(s.segment.b, t) + side * (direction * 6.28) + DVec3::Z * 4.5;
                let world = field.frame.world(position);
                // A union removes walls at forks; do not hang lamps in the
                // resulting vehicle opening when their mounting wall is gone.
                // The CSG query already includes a0.5m margin; our mounting
                // inset is0.22m. Only deeper union openings remove the mount.
                if field.tunnel_void(world.normalize(), world.length() - field.frame.radius) > 0.85
                {
                    continue;
                }
                lamps.push(Lamp { position, owner: i });
                fixture(&mut fixtures[i], position, side, forward);
            }
        }
        let mut by_owner = vec![Vec::new(); spans.len()];
        for (i, l) in lamps.iter().enumerate() {
            by_owner[l.owner].push(i);
        }
        let mut data = vec![[0.0; 4]; MAX_LIGHTS + 1]; // group zero: outdoor/pilot
        for (i, s) in spans.iter().enumerate() {
            let mut distance = HashMap::from([(i, 0.0)]);
            let mut pending = VecDeque::from([i]);
            let centre = (s.segment.a + s.segment.b) * 0.5;
            // Topological locality prevents lamps bleeding through rock into
            // nearby, disconnected parallel bores. This is worker-only.
            while let Some(at) = pending.pop_front() {
                if distance.len() > 256 {
                    break;
                }
                for p in [spans[at].segment.a, spans[at].segment.b] {
                    for &next in &endpoints[&key(p)] {
                        let cost = distance[&at]
                            + (spans[at].segment.a.distance(spans[at].segment.b)
                                + spans[next].segment.a.distance(spans[next].segment.b))
                                * 0.5;
                        let limit = RADIUS
                            + s.segment.a.distance(s.segment.b)
                            + spans[next].segment.a.distance(spans[next].segment.b);
                        if cost <= limit && distance.get(&next).is_none_or(|&old| cost < old) {
                            distance.insert(next, cost);
                            pending.push_back(next);
                        }
                    }
                }
            }
            let mut candidates: Vec<_> = distance
                .keys()
                .flat_map(|&j| by_owner[j].iter().copied())
                .filter(|&l| {
                    let p = lamps[l].position;
                    let d = s.segment.b - s.segment.a;
                    let t = ((p - s.segment.a).dot(d) / d.length_squared()).clamp(0.0, 1.0);
                    p.distance(s.segment.a + d * t) < RADIUS + 8.0
                })
                .collect();
            candidates.sort_by(|&a, &b| {
                lamps[a]
                    .position
                    .distance_squared(centre)
                    .total_cmp(&lamps[b].position.distance_squared(centre))
                    .then(a.cmp(&b))
            });
            candidates.truncate(MAX_LIGHTS);
            data.push([candidates.len() as f32, 0.0, 0.0, 0.0]);
            for slot in 0..MAX_LIGHTS {
                data.push(candidates.get(slot).map_or([0.0; 4], |&l| {
                    let p = lamps[l].position;
                    [p.x as f32, p.y as f32, p.z as f32, 95.0]
                }));
            }
        }
        Self { data, fixtures }
    }
}
/// Station each complete regional alignment before residency cropping. The
/// transient joined draw mesh must not move the lamps when neighbours arrive.
pub fn stations(points: &[DVec3], edges: &[[usize; 2]]) -> Vec<[f64; 2]> {
    let mut groups = HashMap::new();
    let ids: Vec<_> = points
        .iter()
        .map(|p| {
            let next = groups.len();
            *groups.entry(p.to_array().map(f64::to_bits)).or_insert(next)
        })
        .collect();
    let mut adjacent = vec![Vec::new(); groups.len()];
    for (i, &[a, b]) in edges.iter().enumerate() {
        if ids[a] != ids[b] {
            adjacent[ids[a]].push((i, ids[b]));
            adjacent[ids[b]].push((i, ids[a]));
        }
    }
    let mut seen = vec![false; edges.len()];
    let mut result = vec![[0.; 2]; edges.len()];
    let roots = (0..adjacent.len())
        .filter(|&i| adjacent[i].len() != 2)
        .chain(0..adjacent.len());
    for root in roots {
        for &(first, _) in &adjacent[root] {
            let mut next = Some((root, first));
            let mut station = 0.0;
            while let Some((from, index)) = next {
                if seen[index] {
                    break;
                }
                seen[index] = true;
                let [a, b] = edges[index];
                let end = station + points[a].distance(points[b]);
                result[index] = if ids[a] == from {
                    [station, end]
                } else {
                    [end, station]
                };
                station = end;
                let to = if ids[a] == from { ids[b] } else { ids[a] };
                next = if adjacent[to].len() == 2 {
                    adjacent[to]
                        .iter()
                        .find(|(i, _)| *i != index)
                        .map(|&(i, _)| (to, i))
                } else {
                    None
                };
            }
        }
    }
    result
}

fn fixture(mesh: &mut Vec<Vertex>, centre: DVec3, side: DVec3, forward: DVec3) {
    let p = |x: f64, y: f64, z: f64| centre + side * x + forward * y + DVec3::Z * z;
    // Housing, then a slightly inset warm luminous diffuser on both sides.
    for (x, reverse) in [(0.09, false), (-0.09, true)] {
        let mut q = [
            p(x, -0.52, -0.14),
            p(x, 0.52, -0.14),
            p(x, 0.52, 0.14),
            p(x, -0.52, 0.14),
        ];
        if reverse {
            q.reverse();
        }
        road::quad(mesh, q, [0.08, 0.085, -0.09]);
        let x = x * 1.02;
        let mut q = [
            p(x, -0.40, -0.07),
            p(x, 0.40, -0.07),
            p(x, 0.40, 0.07),
            p(x, -0.40, 0.07),
        ];
        if reverse {
            q.reverse();
        }
        road::quad(mesh, q, [-3.0, 2.6, 2.1]);
    }
}
pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cached tunnel light lists"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}
pub fn binding(device: &wgpu::Device, data: &[[f32; 4]]) -> wgpu::BindGroup {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("immutable tunnel light lists"),
        contents: bytemuck::cast_slice(data),
        usage: wgpu::BufferUsages::STORAGE,
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tunnel lights"),
        layout: &layout(device),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lamp_stations_use_complete_source_alignment_not_resident_mesh_order() {
        let points = vec![DVec3::ZERO, DVec3::Y * 33.0, DVec3::Y * 85.0];
        let a = stations(&points, &[[0, 1], [1, 2]]);
        let b = stations(&points, &[[2, 1], [1, 0]]);
        assert_eq!(a, vec![[0., 33.], [33., 85.]]);
        assert_eq!(b, vec![[85., 33.], [33., 0.]]);
        let shifted: Vec<_> = points
            .iter()
            .map(|p| *p + DVec3::new(7000., -4321., 987.))
            .collect();
        assert_eq!(a, stations(&shifted, &[[0, 1], [1, 2]]));
    }
    #[test]
    fn wall_lights_alternate_and_lists_are_bounded_and_finite() {
        let frame = crate::road_network_field::Frame::new(DVec3::Z, 4e6).unwrap();
        let segment = Segment {
            a: DVec3::ZERO,
            b: DVec3::Y * 100.0,
            tunnel: true,
            reach: 64.0,
            floor_depth: 6.0,
            junction_a: false,
            junction_b: false,
        };
        let field = Field::build(frame, vec![segment]).unwrap();
        let baked = Lighting::build(
            &[Span {
                segment,
                sides: [DVec3::X; 2],
                station: [0., 100.],
            }],
            &field,
        );
        assert_eq!(baked.fixtures[0].len(), 5 * 24);
        for (i, light) in baked.fixtures[0].chunks_exact(24).enumerate() {
            let x = light.iter().map(|v| v.position[0]).sum::<f32>() / 24.0;
            assert_eq!(x.signum(), if i % 2 == 0 { -1.0 } else { 1.0 });
        }
        assert_eq!(baked.data.len(), 2 * (MAX_LIGHTS + 1));
        assert!(baked.data.iter().flatten().all(|v| v.is_finite()));
        assert!(baked.data[MAX_LIGHTS + 1][0] <= MAX_LIGHTS as f32);

        let other = Segment {
            a: segment.a + DVec3::X * 20.0,
            b: segment.b + DVec3::X * 20.0,
            ..segment
        };
        let field = Field::build(frame, vec![segment, other]).unwrap();
        let spans = [segment, other].map(|segment| Span {
            segment,
            sides: [DVec3::X; 2],
            station: [0., 100.],
        });
        let separated = Lighting::build(&spans, &field);
        // Nearby but disconnected bores must not light one another through rock.
        for group in separated.data.chunks_exact(MAX_LIGHTS + 1).skip(1) {
            assert_eq!(group[0][0], 5.0);
        }
    }
}

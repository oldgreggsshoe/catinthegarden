//! Bounded excavation index for arbitrary roads, including shared junctions.
//! Built off-thread with the meshes; queries never visit the planetary graph.
//! A full index is rejected atomically, never published with missing segments.
use glam::{DVec2, DVec3};
use std::collections::BTreeMap;

const CELL: f64 = 64.0;
const BUCKETS: usize = 32768;
const PROBES: usize = 32;
const MAX_PER_CELL: usize = 64;
const MAX_REFS: usize = 262144;
const MAX_SEGMENTS: usize = 32768;
const REFS: usize = 4 + BUCKETS;
const SEGMENTS: usize = REFS + MAX_REFS / 4;
pub const BUFFER_FLOAT4S: usize = SEGMENTS + 2 * MAX_SEGMENTS;

#[derive(Clone, Copy)]
pub struct Frame {
    pub up: DVec3,
    pub across: DVec3,
    pub forward: DVec3,
    pub radius: f64,
}
impl Frame {
    pub fn new(up: DVec3, radius: f64) -> Result<Self, String> {
        if !up.is_finite() || up.length_squared() < 0.5 || !radius.is_finite() || radius <= 0.0 {
            return Err("invalid network field frame".into());
        }
        let up = up.normalize();
        let reference = if up.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y };
        let forward = (reference - up * reference.dot(up)).normalize();
        Ok(Self {
            up,
            forward,
            across: forward.cross(up),
            radius,
        })
    }
    pub fn local(&self, p: DVec3) -> DVec3 {
        DVec3::new(
            p.dot(self.across),
            p.dot(self.forward),
            p.dot(self.up) - self.radius,
        )
    }
    pub fn world(&self, p: DVec3) -> DVec3 {
        self.up * (self.radius + p.z) + self.across * p.x + self.forward * p.y
    }
    fn radial_local(&self, direction: DVec3, height: f64) -> DVec3 {
        DVec3::new(
            direction.dot(self.across) * (self.radius + height),
            direction.dot(self.forward) * (self.radius + height),
            (direction.dot(self.up) - 1.0) * self.radius + direction.dot(self.up) * height,
        )
    }
}

#[derive(Clone, Copy)]
pub struct Segment {
    /// Metres in the index frame, not radius-sized float world coordinates.
    pub a: DVec3,
    pub b: DVec3,
    pub tunnel: bool,
    /// Explicit bounded excavation footprint; engineering must survey its banks.
    pub reach: f64,
}
impl Segment {
    fn nearest(self, p: DVec2) -> (DVec3, f64) {
        let delta = (self.b - self.a).truncate();
        let t = ((p - self.a.truncate()).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
        let centre = self.a.lerp(self.b, t);
        (centre, p.distance(centre.truncate()))
    }
}

pub struct Field {
    pub frame: Frame,
    pub segments: Vec<Segment>,
    cells: BTreeMap<(i32, i32), Vec<usize>>,
    data: Vec<[f32; 4]>,
}
fn hash(x: i32, y: i32) -> usize {
    ((x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663)) as usize & (BUCKETS - 1)
}
impl Field {
    pub fn build(frame: Frame, segments: Vec<Segment>) -> Result<Self, String> {
        if segments.len() > MAX_SEGMENTS {
            return Err("network excavation segment budget exceeded".into());
        }
        let mut cells = BTreeMap::<(i32, i32), Vec<usize>>::new();
        let mut references = 0;
        for (i, s) in segments.iter().enumerate() {
            if !s.a.is_finite()
                || !s.b.is_finite()
                || !s.reach.is_finite()
                || s.reach < 10.0
                || s.reach > 512.0
                || (s.b - s.a).truncate().length_squared() < 1e-6
                || s.a.abs().max_element() > 1e6
                || s.b.abs().max_element() > 1e6
            {
                return Err("invalid network excavation segment".into());
            }
            let low = (s.a.min(s.b).truncate() - DVec2::splat(s.reach)) / CELL;
            let high = (s.a.max(s.b).truncate() + DVec2::splat(s.reach)) / CELL;
            for y in low.y.floor() as i32..=high.y.floor() as i32 {
                for x in low.x.floor() as i32..=high.x.floor() as i32 {
                    let list = cells.entry((x, y)).or_default();
                    list.push(i);
                    references += 1;
                    if list.len() > MAX_PER_CELL
                        || references > MAX_REFS
                        || cells.len() > BUCKETS / 2
                    {
                        return Err("network excavation cell/reference budget exceeded".into());
                    }
                }
            }
        }
        let mut data = vec![[0.0; 4]; BUFFER_FLOAT4S];
        let v = |p: DVec3| [p.x as f32, p.y as f32, p.z as f32, 0.0];
        data[0] = [frame.radius as f32, CELL as f32, segments.len() as f32, 2.0];
        data[1] = v(frame.across);
        data[2] = v(frame.forward);
        data[3] = v(frame.up);
        let mut reference = 0;
        for (&(x, y), list) in &cells {
            let bucket = (0..PROBES)
                .map(|p| 4 + ((hash(x, y) + p) & (BUCKETS - 1)))
                .find(|&b| data[b][3] == 0.0)
                .ok_or("network excavation hash probe budget exceeded")?;
            data[bucket] = [x as f32, y as f32, reference as f32, list.len() as f32];
            for &i in list {
                data[REFS + reference / 4][reference % 4] = i as f32;
                reference += 1;
            }
        }
        for (i, s) in segments.iter().enumerate() {
            data[SEGMENTS + i * 2] = [s.a.x as f32, s.a.y as f32, s.a.z as f32, s.reach as f32];
            data[SEGMENTS + i * 2 + 1] = [
                s.b.x as f32,
                s.b.y as f32,
                s.b.z as f32,
                f32::from(s.tunnel),
            ];
        }
        Ok(Self {
            frame,
            segments,
            cells,
            data,
        })
    }
    pub fn gpu_data(&self) -> &[[f32; 4]] {
        &self.data
    }
    fn candidates(&self, xy: DVec2) -> impl Iterator<Item = Segment> + '_ {
        self.cells
            .get(&((xy.x / CELL).floor() as i32, (xy.y / CELL).floor() as i32))
            .into_iter()
            .flatten()
            .map(|&i| self.segments[i])
    }
    pub fn height(&self, d: DVec3, uncut: f64) -> f64 {
        let cosine = d.dot(self.frame.up);
        if cosine < 0.99 {
            return uncut;
        }
        let mut height = uncut;
        for _ in 0..2 {
            let p = self.frame.radial_local(d, height);
            let mut next = uncut;
            for s in self.candidates(p.truncate()).filter(|s| !s.tunnel) {
                let (centre, offset) = s.nearest(p.truncate());
                if offset > s.reach {
                    continue;
                }
                let cap = centre.z - crate::road::CUT_FLOOR_DEPTH
                    + (offset - crate::road::CUT_HALF_WIDTH).max(0.0);
                let t = ((s.reach - offset) / 8.0).clamp(0.0, 1.0);
                let weight = t * t * (3.0 - 2.0 * t);
                let cut = (cap - (cosine - 1.0) * self.frame.radius) / cosine;
                next = next.min(uncut + (cut - uncut) * weight);
            }
            height = next;
        }
        height
    }
    /// Union of local bores. A branching tunnel is one connected air volume,
    /// not several mutually blocking closed liners. Terrain above stays solid.
    pub fn tunnel_void(&self, d: DVec3, height: f64) -> f64 {
        if d.dot(self.frame.up) < 0.99 {
            return -1e6;
        }
        let p = self.frame.radial_local(d, height);
        self.candidates(p.truncate())
            .filter(|s| s.tunnel)
            .map(|s| {
                let (centre, offset) = s.nearest(p.truncate());
                let z = p.z - centre.z;
                (7.0 - offset)
                    .min(z + 6.5)
                    .min(crate::road_tunnel::roof(offset) + 0.5 - z)
            })
            .fold(-1e6, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(a: [f64; 3], b: [f64; 3], tunnel: bool) -> Segment {
        Segment {
            a: a.into(),
            b: b.into(),
            tunnel,
            reach: 48.0,
        }
    }
    #[test]
    fn network_field_cuts_arbitrary_directions_and_retains_tunnel_roofs() {
        let f = Frame::new(DVec3::new(1.0, 2.0, 3.0), 4e6).unwrap();
        let field = Field::build(
            f,
            vec![
                segment([-200.0, -100.0, 1200.0], [200.0, 100.0, 1220.0], false),
                segment([-200.0, 100.0, 1200.0], [200.0, -100.0, 1220.0], true),
            ],
        )
        .unwrap();
        let p = f.world(DVec3::new(0.0, 0.0, 1212.0));
        assert!(field.tunnel_void(p.normalize(), p.length() - f.radius) > 0.0);
        let roof = f.world(DVec3::new(0.0, 0.0, 1230.0));
        assert!(field.tunnel_void(roof.normalize(), roof.length() - f.radius) < 0.0);
        let d = p.normalize();
        assert!((field.height(d, 1250.0) - 1204.0).abs() < 0.1);
        assert_eq!(field.height(-d, 1250.0), 1250.0);
        assert!(field.candidates(DVec2::ZERO).count() <= MAX_PER_CELL);
    }
    #[test]
    fn network_field_branch_is_a_union_not_a_blocked_crossing() {
        let f = Frame::new(DVec3::X, 4e6).unwrap();
        let field = Field::build(
            f,
            vec![
                segment([-100.0, 0.0, -100.0], [100.0, 0.0, -100.0], true),
                segment([0.0, 0.0, -100.0], [0.0, 100.0, -100.0], true),
            ],
        )
        .unwrap();
        for i in -95..=95 {
            for p in [
                DVec3::new(i as f64, 0.0, -98.0),
                DVec3::new(0.0, i.max(0) as f64, -98.0),
            ] {
                let p = f.world(p);
                assert!(field.tunnel_void(p.normalize(), p.length() - f.radius) > 1.0);
            }
        }
    }
    #[test]
    fn network_field_rejects_overflow_instead_of_dropping_roads() {
        let f = Frame::new(DVec3::X, 4e6).unwrap();
        let s = segment([-10.0, 0.0, 100.0], [10.0, 0.0, 100.0], false);
        assert!(Field::build(f, vec![s; MAX_PER_CELL + 1]).is_err());
        assert!(Field::build(f, vec![Segment { b: s.a, ..s }]).is_err());
    }
    #[test]
    fn network_field_finite_banks_fade_without_a_reach_boundary_step() {
        let f = Frame::new(DVec3::X, 4e6).unwrap();
        let field = Field::build(
            f,
            vec![segment([-100.0, 0.0, 100.0], [100.0, 0.0, 100.0], false)],
        )
        .unwrap();
        let height = |offset| {
            let p = f.world(DVec3::new(0.0, offset, 150.0));
            let d = p.normalize();
            field.height(d, p.length() - f.radius)
        };
        assert!((height(47.999) - height(48.001)).abs() < 0.001);
        assert!(height(40.0) < 149.0);
    }
    #[test]
    fn network_field_gpu_layout_constants_match_the_production_shader() {
        let shader = include_str!("road_cuts.wgsl");
        for (name, value) in [
            ("NETWORK_BUCKETS", BUCKETS),
            ("NETWORK_REFS", REFS),
            ("NETWORK_SEGMENTS", SEGMENTS),
        ] {
            assert!(shader.contains(&format!("const {name}: u32 = {value}u;")));
        }
        assert!(shader.contains(&format!("probe<{}u", PROBES)));
        assert!(shader.contains(&format!("min(list.y,{}u)", MAX_PER_CELL)));
    }
    #[test]
    fn network_field_gpu_layout_hash_and_reference_lists_match_cpu() {
        let f = Frame::new(DVec3::Z, 4e6).unwrap();
        let field = Field::build(
            f,
            (-20..=20)
                .map(|i| {
                    segment(
                        [i as f64 * 100.0, -800.0, 100.0],
                        [i as f64 * 100.0, 800.0, 120.0],
                        i % 2 == 0,
                    )
                })
                .collect(),
        )
        .unwrap();
        for (&(x, y), expected) in &field.cells {
            let entry = (0..PROBES)
                .map(|p| field.data[4 + ((hash(x, y) + p) & (BUCKETS - 1))])
                .find(|b| b[3] > 0.0 && b[0] == x as f32 && b[1] == y as f32)
                .unwrap();
            let actual: Vec<_> = (entry[2] as usize..entry[2] as usize + entry[3] as usize)
                .map(|r| field.data[REFS + r / 4][r % 4] as usize)
                .collect();
            assert_eq!(&actual, expected);
        }
    }
}

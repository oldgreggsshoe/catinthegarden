//! Bounded excavation index for arbitrary roads, including shared junctions.
//! Built off-thread with the meshes; queries never visit the planetary graph.
//! A full index is rejected atomically, never published with missing segments.
use glam::{DVec2, DVec3, DVec4};
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
    pub floor_depth: f64,
    pub junction_a: bool,
    pub junction_b: bool,
}
impl Segment {
    pub(crate) fn is_parking(self) -> bool {
        self.a == self.b
    }
    pub(crate) fn nearest(self, p: DVec2) -> (DVec3, f64) {
        if self.is_parking() {
            return (self.a, p.distance(self.a.truncate()));
        }
        let delta = (self.b - self.a).truncate();
        let t = ((p - self.a.truncate()).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
        let centre = self.a.lerp(self.b, t);
        (centre, p.distance(centre.truncate()))
    }
    pub(crate) fn planes(self, margin: f64) -> [DVec4; 9] {
        let d = (self.b - self.a).truncate();
        let length = d.length();
        let u = d.extend(0.0) / length;
        let v = DVec3::new(u.y, -u.x, 0.0);
        let z = DVec3::Z - u * ((self.b.z - self.a.z) / length);
        let plane = |n: DVec3, c: f64| n.extend(c - n.dot(self.a));
        [
            plane(v, 6.5 + margin),
            plane(-v, 6.5 + margin),
            plane(z, 6.0 + margin),
            plane(-z - v * 0.25, 9.0 + margin),
            plane(-z + v * 0.25, 9.0 + margin),
            plane(-z - v * 1.2, 12.8 + margin),
            plane(-z + v * 1.2, 12.8 + margin),
            // Shrink walls/roof for the eye, never the longitudinal joins.
            // Adjacent stations must keep overlapping air, not collision caps.
            plane(u, 0.5),
            plane(-u, length + 0.5),
        ]
    }
    fn void(self, p: DVec3, margin: f64) -> f64 {
        self.planes(margin)
            .iter()
            .map(|plane| plane.dot(p.extend(1.0)))
            .fold(f64::INFINITY, f64::min)
    }
}

pub struct Field {
    pub frame: Frame,
    pub segments: Vec<Segment>,
    cells: BTreeMap<(i32, i32), Vec<usize>>,
    data: Vec<[f32; 4]>,
    /// Exact shared ribbon cross-sections for walking the rendered triangles.
    deck_sections: Option<Vec<[DVec3; 2]>>,
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
                || !s.floor_depth.is_finite()
                || !(0.0..=6.0).contains(&s.floor_depth)
                || s.reach < 10.0
                || s.reach > 512.0
                || ((s.b - s.a).truncate().length_squared() < 1e-6
                    && !(s.is_parking()
                        && !s.tunnel
                        && s.reach == crate::road_parking::REACH
                        && s.floor_depth == crate::road_parking::FLOOR_DEPTH))
                || s.a.abs().max_element() > 1e6
                || s.b.abs().max_element() > 1e6
            {
                return Err(format!(
                    "invalid network excavation segment {i}: a={:?} b={:?} reach={} floor={}",
                    s.a, s.b, s.reach, s.floor_depth
                ));
            }
            let low = (s.a.min(s.b).truncate()
                - DVec2::splat(s.reach.max(crate::road::TREE_CLEARANCE_METERS)))
                / CELL;
            let high = (s.a.max(s.b).truncate()
                + DVec2::splat(s.reach.max(crate::road::TREE_CLEARANCE_METERS)))
                / CELL;
            for y in low.y.floor() as i32..=high.y.floor() as i32 {
                for x in low.x.floor() as i32..=high.x.floor() as i32 {
                    let list = cells.entry((x, y)).or_default();
                    list.push(i);
                    references += 1;
                    let count = list.len();
                    if count > MAX_PER_CELL || references > MAX_REFS || cells.len() > BUCKETS / 2 {
                        return Err(format!(
                            "network excavation cell/reference budget exceeded: local={}, refs={references}, cells={}",
                            count,
                            cells.len()
                        ));
                    }
                }
            }
        }
        let mut data = vec![[0.0; 4]; BUFFER_FLOAT4S];
        let v = |p: DVec3| [p.x as f32, p.y as f32, p.z as f32, 0.0];
        data[0] = [
            frame.radius as f32,
            CELL as f32,
            segments.iter().filter(|s| s.tunnel).count() as f32,
            2.0,
        ];
        data[1] = v(frame.across);
        data[2] = v(frame.forward);
        data[3] = v(frame.up);
        let mut reference = 0;
        for (&(x, y), list) in &cells {
            let bucket = (0..PROBES)
                .map(|p| 4 + ((hash(x, y) + p) & (BUCKETS - 1)))
                .find(|&b| data[b][3] == 0.0)
                .ok_or("network excavation hash probe budget exceeded")?;
            // Two fractional bits preserve the integer reference count and
            // buffer size: bit0 has bores, bit1 has open roads/courts. Both
            // vertex cuts and fragment masks can skip irrelevant cell lists.
            let bore_flag = if list.iter().any(|&i| segments[i].tunnel) {
                0.25
            } else {
                0.0
            };
            let open_flag = if list.iter().any(|&i| !segments[i].tunnel) {
                0.5
            } else {
                0.0
            };
            data[bucket] = [
                x as f32,
                y as f32,
                reference as f32,
                list.len() as f32 + bore_flag + open_flag,
            ];
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
                if s.is_parking() {
                    crate::road_parking::GPU_TAG - s.floor_depth as f32
                } else if s.tunnel {
                    1.0
                } else {
                    -(s.floor_depth as f32 + 1.0)
                },
            ];
        }
        Ok(Self {
            frame,
            segments,
            cells,
            data,
            deck_sections: None,
        })
    }
    pub fn set_geometry_window(&mut self, centre: DVec2, radius: f64) {
        self.data[1][3] = centre.x as f32;
        self.data[2][3] = centre.y as f32;
        self.data[3][3] = radius as f32;
    }
    pub fn gpu_data(&self) -> &[[f32; 4]] {
        &self.data
    }
    fn candidate_indices(&self, xy: DVec2) -> impl Iterator<Item = usize> + '_ {
        self.cells
            .get(&((xy.x / CELL).floor() as i32, (xy.y / CELL).floor() as i32))
            .into_iter()
            .flatten()
            .copied()
    }
    pub(crate) fn candidates(&self, xy: DVec2) -> impl Iterator<Item = Segment> + '_ {
        self.candidate_indices(xy).map(|i| self.segments[i])
    }
    pub(crate) fn set_deck_sections(&mut self, sections: Vec<[DVec3; 2]>) -> Result<(), String> {
        if sections.len() != self.segments.len()
            || sections
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.length() > 1.03)
        {
            return Err("invalid network deck cross-sections".into());
        }
        self.deck_sections = Some(sections);
        Ok(())
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
                let delta = (s.b - s.a).truncate();
                let along = (p - s.a).truncate().dot(delta) / delta.length().max(1e-6);
                if !s.is_parking() && (along < -1.0 || along > delta.length() + 1.0) {
                    continue;
                }
                let (centre, offset) = s.nearest(p.truncate());
                if offset > s.reach {
                    continue;
                }
                let cap = centre.z - s.floor_depth
                    + (offset
                        - if s.is_parking() {
                            crate::road_parking::CUT_RADIUS
                        } else {
                            crate::road::CUT_HALF_WIDTH
                        })
                    .max(0.0);
                let t = ((s.reach - offset) / 8.0).clamp(0.0, 1.0);
                let weight = t * t * (3.0 - 2.0 * t);
                let cut = (cap - (cosine - 1.0) * self.frame.radius) / cosine;
                next = next.min(uncut + (cut - uncut) * weight);
            }
            height = next;
        }
        height
    }
    pub fn clears_tree(&self, d: DVec3, uncut: f64) -> bool {
        if d.dot(self.frame.up) < 0.99 {
            return false;
        }
        let p = self.frame.radial_local(d, uncut);
        self.candidates(p.truncate())
            .filter(|s| !s.tunnel)
            .any(|s| {
                if s.is_parking() {
                    return s.nearest(p.truncate()).1 <= crate::road_parking::TREE_RADIUS;
                }
                let delta = (s.b - s.a).truncate();
                let along = (p - s.a).truncate().dot(delta) / delta.length();
                along
                    >= -if s.floor_depth <= 1.0 {
                        crate::road::TREE_CLEARANCE_METERS
                    } else {
                        1.0
                    }
                    && along
                        <= delta.length()
                            + if s.floor_depth <= 1.0 {
                                crate::road::TREE_CLEARANCE_METERS
                            } else {
                                1.0
                            }
                    && s.nearest(p.truncate()).1 <= crate::road::TREE_CLEARANCE_METERS
            })
            || self.height(d, uncut) < uncut - 0.25
    }
    pub fn deck_height(&self, d: DVec3, eye: f64) -> Option<f64> {
        if d.dot(self.frame.up) < 0.99 {
            return None;
        }
        let local = self.frame.radial_local(d, eye);
        let ray = DVec3::new(
            d.dot(self.frame.across),
            d.dot(self.frame.forward),
            d.dot(self.frame.up),
        );
        self.candidate_indices(local.truncate())
            .filter_map(|i| {
                let s = self.segments[i];
                if s.is_parking() {
                    let radial = (s.a.z + self.frame.radius) / ray.z;
                    let p = ray * radial - DVec3::Z * self.frame.radius;
                    let height = radial - self.frame.radius;
                    return (crate::road_parking::contains((p - s.a).truncate())
                        && eye >= height - 0.25)
                        .then_some(height);
                }
                if let Some(sections) = &self.deck_sections {
                    let [a, b] = sections[i].map(|v| v * crate::road::HALF_WIDTH);
                    let q = [s.a - a, s.a + a, s.b + b, s.b - b];
                    // Same two triangles as road::quad, including sloped miters.
                    return [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]
                        .into_iter()
                        .filter_map(|[a, b, c]| {
                            let n = (b - a).cross(c - a);
                            let denominator = n.dot(ray);
                            if denominator.abs() < 1e-10 {
                                return None;
                            }
                            let radial = (n.dot(a) + n.z * self.frame.radius) / denominator;
                            let p = (ray * radial - DVec3::Z * self.frame.radius - a).truncate();
                            let u = (b - a).truncate();
                            let v = (c - a).truncate();
                            let cross = |a: DVec2, b: DVec2| a.x * b.y - a.y * b.x;
                            let determinant = cross(u, v);
                            if determinant.abs() < 1e-10 {
                                return None;
                            }
                            let x = cross(p, v) / determinant;
                            let y = cross(u, p) / determinant;
                            let h = radial - self.frame.radius;
                            (x >= -1e-7 && y >= -1e-7 && x + y <= 1.0 + 1e-7 && eye >= h - 0.25)
                                .then_some(h)
                        })
                        .reduce(f64::max);
                }
                let delta = (s.b - s.a).truncate();
                let normal =
                    DVec3::Z - delta.extend(0.0) * (s.b.z - s.a.z) / delta.length_squared();
                let radial = (normal.dot(s.a) + normal.z * self.frame.radius) / normal.dot(ray);
                let p = ray * radial - DVec3::Z * self.frame.radius;
                let along = (p - s.a).truncate().dot(delta) / delta.length_squared();
                if (along < -1e-8 && !s.junction_a) || (along > 1.0 + 1e-8 && !s.junction_b) {
                    return None;
                }
                (s.nearest(p.truncate()).1 <= crate::road::HALF_WIDTH + 0.01
                    && eye >= radial - self.frame.radius - 0.25)
                    .then_some(radial - self.frame.radius)
            })
            .reduce(f64::max)
    }
    pub fn camera_surface_height(&self, d: DVec3, eye: f64, terrain: f64) -> f64 {
        if self.tunnel_void(d, eye) > 0.0 {
            if let Some(deck) = self.deck_height(d, eye) {
                return deck;
            }
            let p = self.frame.radial_local(d, eye);
            return self
                .candidates(p.truncate())
                .filter(|s| s.tunnel && s.void(p, 0.5) > 0.0)
                .map(|s| {
                    let z = s.nearest(p.truncate()).0.z - 6.0;
                    self.frame.world(DVec3::new(p.x, p.y, z)).length() - self.frame.radius
                })
                .reduce(f64::max)
                .unwrap_or(terrain);
        }
        self.deck_height(d, eye)
            .map_or(terrain, |deck| terrain.max(deck))
    }
    pub fn constrain_motion(&self, from: DVec3, to: DVec3) -> DVec3 {
        if (from + to).normalize_or_zero().dot(self.frame.up) < 0.99 {
            return to;
        }
        let a = self.frame.local(from);
        let b = self.frame.local(to);
        let air = |p: DVec3| {
            self.candidates(p.truncate())
                .filter(|s| s.tunnel)
                .any(|s| s.void(p, -0.2) > 0.0)
        };
        let column = |p: DVec3| {
            self.candidates(p.truncate()).filter(|s| s.tunnel).any(|s| {
                let delta = (s.b - s.a).truncate();
                let t = (p - s.a).truncate().dot(delta) / delta.length_squared();
                (0.0..=1.0).contains(&t) && s.nearest(p.truncate()).1 < 9.0
            })
        };
        let steps = ((a.distance(b) / 0.5).ceil() as usize).clamp(1, 1024);
        let mut previous = a;
        let mut prior_air = air(a);
        for i in 1..=steps {
            let p = a.lerp(b, i as f64 / steps as f64);
            let now_air = air(p);
            if prior_air != now_air && column(previous) && column(p) {
                let (mut lo, mut hi) = (previous, p);
                for _ in 0..20 {
                    let mid = (lo + hi) * 0.5;
                    if air(mid) == prior_air {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return self.frame.world(lo);
            }
            previous = p;
            prior_air = now_air;
        }
        to
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
            .map(|s| s.void(p, 0.5))
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
            floor_depth: 6.0,
            junction_a: false,
            junction_b: false,
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
    fn network_field_clears_crowns_and_cuts_but_keeps_trees_above_tunnels() {
        let frame = Frame::new(DVec3::X, 4e6).unwrap();
        let field = Field::build(
            frame,
            vec![
                segment([-100.0, 0.0, 100.0], [100.0, 0.0, 100.0], false),
                segment([-100.0, 200.0, 100.0], [100.0, 200.0, 100.0], true),
            ],
        )
        .unwrap();
        for (y, height, cleared) in [
            (0.0, 90.0, true),
            (24.0, 90.0, true),
            (26.0, 90.0, false),
            (35.0, 130.0, true),
            (80.0, 130.0, false),
            (200.0, 130.0, false),
        ] {
            let p = frame.world(DVec3::new(0.0, y, height));
            assert_eq!(
                field.clears_tree(p.normalize(), p.length() - frame.radius),
                cleared,
                "y={y}"
            );
        }
    }
    #[test]
    fn road_network_motion_crosses_bore_stations_but_stops_at_walls_and_roof() {
        let frame = Frame::new(DVec3::X, 4e6).unwrap();
        let field = Field::build(
            frame,
            vec![
                segment([-40.0, 0.0, -100.0], [0.0, 0.0, -100.0], true),
                segment([0.0, 0.0, -100.0], [40.0, 0.0, -100.0], true),
            ],
        )
        .unwrap();
        let from = frame.world(DVec3::new(-10.0, 0.0, -98.0));
        let to = frame.world(DVec3::new(10.0, 0.0, -98.0));
        assert!(
            field.constrain_motion(from, to).distance(to) < 1e-4,
            "station joins must not be collision caps"
        );
        let wall = frame.world(DVec3::new(-10.0, 20.0, -98.0));
        assert!(frame.local(field.constrain_motion(from, wall)).y < 6.4);
        let roof = frame.world(DVec3::new(-10.0, 0.0, -80.0));
        assert!(frame.local(field.constrain_motion(from, roof)).z < -91.0);
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
    fn network_cell_flags_preserve_full_capacity_counts_for_every_kind() {
        let frame = Frame::new(DVec3::Z, 4e6).unwrap();
        let open = segment([0.0, 0.0, 100.0], [40.0, 0.0, 100.0], false);
        let bore = Segment {
            tunnel: true,
            ..open
        };
        let court = crate::road_parking::segment(DVec3::new(20.0, 0.0, 100.0));
        for (segments, flags) in [
            (vec![open; MAX_PER_CELL], 2),
            (vec![bore; MAX_PER_CELL], 1),
            (vec![court; MAX_PER_CELL], 2),
            (
                (0..MAX_PER_CELL)
                    .map(|i| if i % 2 == 0 { open } else { bore })
                    .collect(),
                3,
            ),
        ] {
            let field = Field::build(frame, segments).unwrap();
            let entry = (0..PROBES)
                .map(|p| field.data[4 + ((hash(0, 0) + p) & (BUCKETS - 1))])
                .find(|b| b[3] > 0.0 && b[0] == 0.0 && b[1] == 0.0)
                .unwrap();
            assert_eq!(entry[3] as usize, MAX_PER_CELL);
            assert_eq!((entry[3].fract() * 4.0) as u32, flags);
        }
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
            let flags = (entry[3].fract() * 4.0) as u32;
            assert_eq!(
                flags & 1 != 0,
                expected.iter().any(|&i| field.segments[i].tunnel)
            );
            assert_eq!(
                flags & 2 != 0,
                expected.iter().any(|&i| !field.segments[i].tunnel)
            );
        }
    }
}

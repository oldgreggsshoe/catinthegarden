//! Conservative local portal visibility, built with each road snapshot.
//! Inflated bore boxes overestimate air. A real sightline must cross every
//! overlap box on its path to a mouth. Screen-rectangle beam propagation may
//! retain hidden outdoor work, but never substitutes a few sampled rays for
//! proof that the entire view is enclosed. Unknown ends/budgets fail open.
use crate::road_network_field::{Field, Frame, Segment};
use glam::{DVec2, DVec3};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Copy)]
struct Bounds {
    lo: DVec3,
    hi: DVec3,
}
impl Bounds {
    fn around(a: DVec3, b: DVec3) -> Self {
        Self {
            lo: a.min(b) - DVec3::splat(10.0),
            hi: a.max(b) + DVec3::splat(10.0),
        }
    }
    fn contains(self, p: DVec3) -> bool {
        p.cmpge(self.lo).all() && p.cmple(self.hi).all()
    }
    fn overlap(self, b: Self) -> Option<Self> {
        let out = Self {
            lo: self.lo.max(b.lo),
            hi: self.hi.min(b.hi),
        };
        out.lo.cmple(out.hi).all().then_some(out)
    }
    fn corners(self) -> impl Iterator<Item = DVec3> {
        (0..8).map(move |i| {
            DVec3::new(
                if i & 1 == 0 { self.lo.x } else { self.hi.x },
                if i & 2 == 0 { self.lo.y } else { self.hi.y },
                if i & 4 == 0 { self.lo.z } else { self.hi.z },
            )
        })
    }
}
#[derive(Clone, Copy, PartialEq)]
struct Rect {
    lo: DVec2,
    hi: DVec2,
}
impl Rect {
    const FULL: Self = Self {
        lo: DVec2::NEG_ONE,
        hi: DVec2::ONE,
    };
    fn intersect(self, b: Self) -> Option<Self> {
        let r = Self {
            lo: self.lo.max(b.lo),
            hi: self.hi.min(b.hi),
        };
        r.lo.cmple(r.hi).all().then_some(r)
    }
    fn union(self, b: Self) -> Self {
        Self {
            lo: self.lo.min(b.lo),
            hi: self.hi.max(b.hi),
        }
    }
}
struct Cell {
    bounds: Bounds,
    core: [glam::DVec4; 9],
    escape: bool,
    mouths: Vec<Bounds>,
    adjacent: Vec<(usize, Bounds)>,
}
pub struct Visibility {
    frame: Frame,
    cells: Vec<Cell>,
    valid: bool,
}
impl Visibility {
    /// The final flag means opaque lining exists, independently of deck LOD.
    pub fn build(frame: Frame, roads: &[(Segment, [bool; 2], bool)]) -> Self {
        let mut result = Self {
            frame,
            cells: Vec::new(),
            valid: true,
        };
        let key = |p: DVec3| p.to_array().map(|v| (v * 1000.0).round() as i64);
        let mut ends = HashMap::<_, usize>::new();
        for (s, _, _) in roads.iter().filter(|r| r.0.tunnel) {
            for p in [s.a, s.b] {
                *ends.entry(key(p)).or_default() += 1;
            }
        }
        let mut grid = HashMap::<[i32; 3], Vec<usize>>::new();
        let mut pairs = HashSet::new();
        for (s, portals, opaque) in roads.iter().filter(|r| r.0.tunnel) {
            let bounds = Bounds::around(s.a, s.b);
            let mouths = [s.a, s.b]
                .into_iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    (portals[i] || ends[&key(p)] < 2).then(|| Bounds::around(p, p))
                })
                .collect();
            let i = result.cells.len();
            if i >= 8192 {
                result.valid = false;
                break;
            }
            result.cells.push(Cell {
                bounds,
                core: s.planes(-0.5),
                escape: !opaque,
                mouths,
                adjacent: Vec::new(),
            });
            let lo = (bounds.lo / 64.0).floor().as_ivec3();
            let hi = (bounds.hi / 64.0).floor().as_ivec3();
            if (hi - lo + glam::IVec3::ONE).as_dvec3().element_product() > 4096.0 {
                result.valid = false;
                break;
            }
            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        let bucket = grid.entry([x, y, z]).or_default();
                        for &j in bucket.iter() {
                            if pairs.insert((j, i))
                                && let Some(portal) = bounds.overlap(result.cells[j].bounds)
                            {
                                result.cells[i].adjacent.push((j, portal));
                                result.cells[j].adjacent.push((i, portal));
                            }
                        }
                        bucket.push(i);
                    }
                }
            }
            if pairs.len() > 262144 {
                result.valid = false;
                break;
            }
        }
        result
    }

    pub fn enclosed(
        &self,
        field: &Field,
        eye: DVec3,
        forward: DVec3,
        up: DVec3,
        fov: f64,
        aspect: f64,
    ) -> bool {
        if !self.valid || field.tunnel_void(eye.normalize(), eye.length() - self.frame.radius) < 0.4
        {
            tracing::debug!(target:"planet::tunnel_visibility",valid=self.valid,"unproven interior");
            return false;
        }
        let eye = self.frame.local(eye);
        // The cut-field query includes a wall margin. Require actual roof/
        // wall clearance too, so a near-clipped wall cannot expose the void.
        // Segment::planes deliberately preserves longitudinal join overlap.
        if !self
            .cells
            .iter()
            .any(|c| c.bounds.contains(eye) && c.core.iter().all(|p| p.dot(eye.extend(1.0)) > 0.0))
        {
            return false;
        }
        let local = |v: DVec3| {
            DVec3::new(
                v.dot(self.frame.across),
                v.dot(self.frame.forward),
                v.dot(self.frame.up),
            )
        };
        let forward = local(forward).normalize();
        let right = forward.cross(local(up)).normalize();
        let up = right.cross(forward);
        if !forward.is_finite()
            || !right.is_finite()
            || !(0.0..std::f64::consts::PI).contains(&fov)
            || !aspect.is_finite()
            || aspect <= 0.0
        {
            return false;
        }
        let tan = (fov * 0.5).tan();
        let project = |bounds: Bounds| -> Option<Rect> {
            // Reject boxes outside any side plane before handling a near-plane
            // crossing. A box at our side is not automatically a visible mouth.
            for plane in [
                forward * tan * aspect + right,
                forward * tan * aspect - right,
                forward * tan + up,
                forward * tan - up,
            ] {
                if bounds.corners().all(|p| (p - eye).dot(plane) < 0.0) {
                    return None;
                }
            }
            let mut lo = DVec2::splat(f64::INFINITY);
            let mut hi = DVec2::splat(f64::NEG_INFINITY);
            let mut near = f64::INFINITY;
            let mut far = f64::NEG_INFINITY;
            for p in bounds.corners() {
                let p = p - eye;
                let depth = p.dot(forward);
                near = near.min(depth);
                far = far.max(depth);
                let xy = DVec2::new(
                    p.dot(right) / (depth * tan * aspect),
                    p.dot(up) / (depth * tan),
                );
                lo = lo.min(xy);
                hi = hi.max(xy);
            }
            if far < 0.0 {
                None
            } else if near < 0.05 {
                Some(Rect::FULL)
            } else {
                Rect::FULL.intersect(Rect { lo, hi })
            }
        };
        let mut beams = vec![None::<Rect>; self.cells.len()];
        let mut pending = VecDeque::new();
        for (i, c) in self.cells.iter().enumerate() {
            if c.bounds.contains(eye) {
                beams[i] = Some(Rect::FULL);
                pending.push_back(i);
            }
        }
        if pending.is_empty() {
            return false;
        }
        let mut work = 0;
        while let Some(i) = pending.pop_front() {
            work += 1;
            if work > 4096 {
                return false;
            }
            let cell = &self.cells[i];
            let beam = beams[i].unwrap();
            // Liners (including their floors) remain opaque at the far deck
            // LOD. Fail open before the renderer's25km chunk-distance cutoff.
            if cell.escape
                || cell
                    .bounds
                    .corners()
                    .any(|p| p.distance_squared(eye) > 24000.0_f64.powi(2))
            {
                tracing::debug!(target:"planet::tunnel_visibility",cell=i,escape=cell.escape,"geometry limit reached");
                return false;
            }
            if cell
                .mouths
                .iter()
                .any(|&b| project(b).and_then(|r| r.intersect(beam)).is_some())
            {
                tracing::debug!(target:"planet::tunnel_visibility",cell=i,"mouth may be visible");
                return false;
            }
            for &(j, portal) in &cell.adjacent {
                if let Some(next) = project(portal).and_then(|r| r.intersect(beam)) {
                    let expanded = beams[j].map_or(next, |old| old.union(next));
                    if beams[j] != Some(expanded) {
                        beams[j] = Some(expanded);
                        pending.push_back(j);
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(points: &[[f64; 2]]) -> (Field, Visibility) {
        let frame = Frame::new(DVec3::Z, 4e6).unwrap();
        let segments: Vec<_> = points
            .windows(2)
            .map(|p| Segment {
                a: DVec3::new(p[0][0], p[0][1], 0.0),
                b: DVec3::new(p[1][0], p[1][1], 0.0),
                tunnel: true,
                reach: 64.0,
                floor_depth: 6.0,
                junction_a: false,
                junction_b: false,
            })
            .collect();
        let roads = segments
            .iter()
            .copied()
            .map(|s| (s, [false; 2], true))
            .collect::<Vec<_>>();
        (
            Field::build(frame, segments).unwrap(),
            Visibility::build(frame, &roads),
        )
    }
    #[test]
    fn straight_visible_mouths_keep_outdoors_but_wall_views_do_not() {
        let (f, v) = fixture(&[[0., -200.], [0., 0.], [0., 200.]]);
        let eye = f.frame.world(DVec3::new(0., 0., 1.4));
        assert!(!v.enclosed(&f, eye, DVec3::Y, DVec3::Z, 1.0, 1.6));
        assert!(!v.enclosed(&f, eye, -DVec3::Y, DVec3::Z, 1.0, 1.6));
        assert!(v.enclosed(&f, eye, DVec3::X, DVec3::Z, 1.0, 1.6));
        assert!(!v.enclosed(
            &f,
            f.frame.world(DVec3::new(30., 0., 1.4)),
            DVec3::X,
            DVec3::Z,
            1.0,
            1.6
        ));
        for x in [6.4, 6.6] {
            assert!(!v.enclosed(
                &f,
                f.frame.world(DVec3::new(x, -20., 1.4)),
                DVec3::X,
                DVec3::Z,
                1.0,
                1.6,
            ));
        }
    }
    #[test]
    fn a_bend_blocks_the_beam_to_a_distant_exit() {
        let (f, v) = fixture(&[
            [0., -200.],
            [0., 0.],
            [0., 100.],
            [100., 100.],
            [200., 100.],
            [200., 200.],
        ]);
        let eye = f.frame.world(DVec3::new(0., 0., 1.4));
        assert!(v.enclosed(&f, eye, DVec3::Y, DVec3::Z, 1.0, 1.6));
    }
    #[test]
    fn uncertain_or_nonopaque_geometry_never_culls_outdoors() {
        let (f, mut v) = fixture(&[[0., -200.], [0., 0.], [0., 200.]]);
        let eye = f.frame.world(DVec3::new(0., 0., 1.4));
        v.valid = false;
        assert!(!v.enclosed(&f, eye, DVec3::X, DVec3::Z, 1.0, 1.6));
        v.valid = true;
        v.cells[0].escape = true;
        assert!(!v.enclosed(&f, eye, DVec3::X, DVec3::Z, 1.0, 1.6));
    }
    #[test]
    fn junction_side_exit_and_near_plane_mouth_keep_outdoors() {
        let (f, v) = fixture(&[[0., -200.], [0., 0.], [0., 200.], [0., 0.], [200., 0.]]);
        let eye = f.frame.world(DVec3::new(0., 0., 1.4));
        assert!(!v.enclosed(&f, eye, DVec3::X, DVec3::Z, 1.0, 1.6));
        assert!(v.enclosed(&f, eye, -DVec3::X, DVec3::Z, 1.0, 1.6));
        let near = f.frame.world(DVec3::new(195., 0., 1.4));
        assert!(!v.enclosed(&f, near, DVec3::X, DVec3::Z, 1.0, 1.6));
        assert!(!v.enclosed(&f, eye, DVec3::X, DVec3::Z, 1.0, f64::NAN));
    }
}

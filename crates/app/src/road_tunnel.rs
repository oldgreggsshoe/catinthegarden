//! One-time tunnel selection and cached lining geometry for the road trial.
use crate::{
    road::{self, Vertex},
    road_route::Route,
};
use glam::{DVec2, DVec3};

pub const CUT_THRESHOLD: f64 = 25.0;
pub const MIN_LENGTH: f64 = 100.0;
pub const MIN_COVER: f64 = 5.0;
pub const HALF_WIDTH: f64 = 6.5;
pub const ROOF: f64 = 9.0;
pub const FLOOR: f64 = -6.0;
// Convex, faceted arch. The same planes are used by the excavation shader.
pub const ARCH: [DVec2; 7] = [
    DVec2::new(-6.5, -6.0),
    DVec2::new(-6.5, 5.0),
    DVec2::new(-4.0, 8.0),
    DVec2::new(0.0, 9.0),
    DVec2::new(4.0, 8.0),
    DVec2::new(6.5, 5.0),
    DVec2::new(6.5, -6.0),
];

#[derive(Clone, Debug)]
pub struct Tunnel {
    /// Inclusive portal station indices; segments start..end are enclosed.
    pub start: usize,
    pub end: usize,
}

pub fn roof(offset: f64) -> f64 {
    (9.0 - 0.25 * offset.abs()).min(12.8 - 1.2 * offset.abs())
}

/// Expand a sustained deep-cut core toward safe portal cover, but never join
/// across a low saddle or a side with insufficient cover. Unknown ground fails
/// closed: keep an open cut rather than inventing a rock roof.
pub fn select(route: &Route, mut ground: impl FnMut(DVec2) -> Option<f64>) -> Vec<Tunnel> {
    let mut covered = Vec::with_capacity(route.points.len());
    for (i, &point) in route.points.iter().enumerate() {
        let side = road::lateral(&route.points, i);
        covered.push([-7.5, -4.0, 0.0, 4.0, 7.5].into_iter().all(|offset| {
            ground((point + side * offset).truncate())
                .is_some_and(|h| h.is_finite() && h - point.z >= ROOF + MIN_COVER)
        }));
    }
    let mut tunnels: Vec<Tunnel> = Vec::new();
    let mut i = 1;
    while i + 1 < route.points.len() {
        if !covered[i]
            || route.ground[i] - route.points[i].z + road::CUT_FLOOR_DEPTH < CUT_THRESHOLD
        {
            i += 1;
            continue;
        }
        let core_start = i;
        while i + 1 < route.points.len()
            && covered[i]
            && route.ground[i] - route.points[i].z + road::CUT_FLOOR_DEPTH >= CUT_THRESHOLD
        {
            i += 1;
        }
        let core_end = i - 1;
        if route.distances[core_end] - route.distances[core_start] < MIN_LENGTH {
            continue;
        }
        let mut start = core_start;
        let mut end = core_end;
        while start > 1 && covered[start - 1] {
            start -= 1;
        }
        while end + 2 < covered.len() && covered[end + 1] {
            end += 1;
        }
        // Never invent a portal merely because the test road ends while it is
        // still buried. Such a corridor needs a different site/alignment.
        if (start == 1 && covered[0]) || (end + 2 == covered.len() && covered[end + 1]) {
            i = end + 1;
            continue;
        }
        if let Some(previous) = tunnels.last_mut().filter(|t| start <= t.end) {
            previous.end = previous.end.max(end);
        } else {
            tunnels.push(Tunnel { start, end });
        }
        i = end + 1;
    }
    tunnels
}

/// Lining has inward faces and a thin outward shell, so both portal views and
/// interiors are closed. Excavation extends 0.5m beyond this inner surface.
pub fn mesh(route: &Route, tunnels: &[Tunnel], start: usize, end: usize) -> Vec<Vertex> {
    let mut vertices = Vec::new();
    let inner = [0.24, 0.235, -0.22]; // negative blue marks sheltered material
    let outer = [0.32, 0.31, 0.29];
    for tunnel in tunnels {
        for i in tunnel.start.saturating_sub(1).max(start)..tunnel.end.min(end) {
            let section = |index: usize, p: DVec2, shell: bool| {
                let offset = if shell { p.x.signum() * 0.8 } else { 0.0 };
                route.points[index]
                    + road::lateral(&route.points, index) * (p.x + offset)
                    + DVec3::Z
                        * (p.y
                            + if shell {
                                if p.y < 0.0 { -0.8 } else { 0.8 }
                            } else {
                                0.0
                            })
            };
            // Small, unshadowed marker strips add depth cues without point
            // lights, textures, animation or a per-frame lighting loop.
            if i % 5 == 0 {
                let side = road::lateral(&route.points, i);
                let along = (route.points[i + 1] - route.points[i]).normalize() * 0.6;
                for sign in [-1.0, 1.0] {
                    let a = route.points[i] + side * (sign * 6.4) + DVec3::Z * 3.0;
                    let b = a + along;
                    let c = b + DVec3::Z * 0.12;
                    let d = a + DVec3::Z * 0.12;
                    road::quad(
                        &mut vertices,
                        if sign < 0.0 {
                            [a, b, c, d]
                        } else {
                            [d, c, b, a]
                        },
                        [-1.8, 1.2, 0.6],
                    );
                }
            }
            for edge in ARCH
                .windows(2)
                .map(|e| [e[0], e[1]])
                .chain(std::iter::once([ARCH[6], ARCH[0]]))
            {
                for shell in [false, true] {
                    let a = section(i, edge[0], shell);
                    let b = section(i, edge[1], shell);
                    let c = section(i + 1, edge[1], shell);
                    let d = section(i + 1, edge[0], shell);
                    road::quad(
                        &mut vertices,
                        if shell { [a, b, c, d] } else { [d, c, b, a] },
                        if shell { outer } else { inner },
                    );
                }
            }
        }
        // Portal collar rings project one metre into the approach and two
        // metres into rock, covering the cut/roof transition without a cap.
        for (index, facing) in [(tunnel.start.saturating_sub(1), -1.0), (tunnel.end, 1.0)] {
            if index < start || index >= end {
                continue;
            }
            let tangent = (route.points[(index + 1).min(route.points.len() - 1)]
                - route.points[index.saturating_sub(1)])
            .normalize();
            let side = road::lateral(&route.points, index);
            for edge in ARCH.windows(2) {
                let point = |p: DVec2, outside: bool, depth: f64| {
                    route.points[index]
                        + tangent * depth
                        + side * (p.x + if outside { p.x.signum() * 1.4 } else { 0.0 })
                        + DVec3::Z * (p.y + if outside { 1.4 } else { 0.0 })
                };
                let a = point(edge[0], false, facing);
                let b = point(edge[1], false, facing);
                let c = point(edge[1], true, facing);
                let d = point(edge[0], true, facing);
                road::quad(
                    &mut vertices,
                    if facing < 0.0 {
                        [a, b, c, d]
                    } else {
                        [d, c, b, a]
                    },
                    outer,
                );
                // Inner collar surface, no end cap blocking the carriageway.
                let c = point(edge[1], false, -facing * 2.0);
                let d = point(edge[0], false, -facing * 2.0);
                road::quad(
                    &mut vertices,
                    if facing < 0.0 {
                        [d, c, b, a]
                    } else {
                        [a, b, c, d]
                    },
                    inner,
                );
            }
        }
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route() -> Route {
        Route {
            points: (0..101)
                .map(|i| DVec3::new(0.0, i as f64 * 10.0, 100.0))
                .collect(),
            ground: vec![130.0; 101],
            distances: (0..101).map(|i| i as f64 * 10.0).collect(),
            straight_cut_volume: 0.0,
            cut_volume: 0.0,
        }
    }
    #[test]
    fn tunnel_selection_requires_length_and_full_width_cover() {
        let mut r = route();
        let t = select(&r, |p| {
            Some(if (100.0..=900.0).contains(&p.y) {
                130.0
            } else {
                110.0
            })
        });
        assert_eq!((t[0].start, t[0].end), (10, 90));
        assert!(
            select(&r, |_| Some(130.0)).is_empty(),
            "no underground endpoint portals"
        );
        assert!(
            select(&r, |p| Some(
                if !(100.0..=900.0).contains(&p.y) || p.x > 5.0 {
                    110.0
                } else {
                    130.0
                }
            ))
            .is_empty()
        );
        assert!(select(&r, |_| None).is_empty());
        r.ground.fill(110.0);
        r.ground[30..35].fill(130.0);
        assert!(
            select(&r, |p| Some(if (100.0..=900.0).contains(&p.y) {
                130.0
            } else {
                110.0
            }))
            .is_empty()
        );
        assert_eq!(roof(0.0), 9.0);
        assert!((roof(6.5) - 5.0).abs() < 1e-10);
    }
    #[test]
    fn tunnel_sections_merge_only_across_continuous_cover() {
        let r = route();
        let t = select(&r, |p| {
            Some(
                if p.y < 100.0 || p.y > 900.0 || (450.0..550.0).contains(&p.y) {
                    110.0
                } else {
                    130.0
                },
            )
        });
        assert_eq!(t.len(), 2);
        assert!(t[0].end < t[1].start);
        let vertices = mesh(&r, &t, 0, 100);
        assert!(!vertices.is_empty());
        assert!(
            vertices
                .iter()
                .all(|v| v.position.iter().chain(&v.normal).all(|x| x.is_finite()))
        );
    }
}

//! Shared-height engineering check for a surveyed network tree.
//!
//! Profiles must be solved together: independently fitted roads can meet at
//! different elevations. Village access points and cross-region gateways are
//! fixed; interior nodes get a Lipschitz extension bounded by terrain/pier and
//! seabed-cover limits. An infeasible alignment is rejected for rerouting, not
//! silently steepened, disconnected, or given a fake underwater portal.
use crate::road_network::{Region, Survey};
use glam::DVec3;
use serde::Serialize;
use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
};

pub(crate) const STEP: f64 = 20.0;
// Radial heights (unlike the pilot's tangent z) already include curvature.
// Retain 0.1 percentage point for tessellation below the user's 25% limit.
const GRADE: f64 = crate::road_route::MAX_GRADE - 0.001;
const CLEARANCE: f64 = 8.0;
const PIER_CAP: f64 = 32.0;
const UNDERSEA_CROWN: f64 = 9.0;
const UNDERSEA_COVER: f64 = 5.0;
const UNDERSEA_DECK_DEPTH: f64 = 20.0;

#[derive(Serialize)]
pub struct Profile {
    pub heights_meters: Vec<f64>,
    pub directions: Vec<[f64; 3]>,
    pub segments: Vec<[usize; 2]>,
    pub maximum_grade: f64,
    pub maximum_deck_above_ground_meters: f64,
    pub minimum_sampled_undersea_roof_cover_meters: Option<f64>,
    pub undersea_segments: usize,
}
#[derive(Serialize, Debug)]
pub struct Rejected {
    pub reason: &'static str,
    pub node: usize,
    pub required_minimum_meters: f64,
    pub allowed_maximum_meters: f64,
}
#[derive(Clone, Copy)]
struct Queue {
    height: f64,
    index: usize,
}
impl PartialEq for Queue {
    fn eq(&self, b: &Self) -> bool {
        self.cmp(b).is_eq()
    }
}
impl Eq for Queue {}
impl PartialOrd for Queue {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for Queue {
    fn cmp(&self, b: &Self) -> Ordering {
        self.height
            .total_cmp(&b.height)
            .then(self.index.cmp(&b.index))
    }
}

fn distance(a: DVec3, b: DVec3, r: f64) -> f64 {
    a.cross(b).length().atan2(a.dot(b)) * r
}

/// Minimum of source heights plus grade * network distance. Negative source
/// heights are fine: all graph edge lengths are non-negative. Running this on
/// negated anchors also constructs their required lower envelope.
fn envelope(mut heights: Vec<f64>, adjacency: &[Vec<(usize, f64)>]) -> Vec<f64> {
    let mut queue = BinaryHeap::new();
    for (index, &height) in heights.iter().enumerate() {
        if height.is_finite() {
            queue.push(Reverse(Queue { height, index }));
        }
    }
    while let Some(Reverse(Queue { height, index })) = queue.pop() {
        if height > heights[index] {
            continue;
        }
        for &(next, length) in &adjacency[index] {
            let candidate = height + GRADE * length;
            if candidate < heights[next] {
                heights[next] = candidate;
                queue.push(Reverse(Queue {
                    height: candidate,
                    index: next,
                }));
            }
        }
    }
    heights
}

#[derive(Clone, Copy)]
pub(crate) struct Sample {
    pub ground: crate::road_network::Ground,
    pub cap: f64,
    pub anchor: f64,
}

fn anchor_height(g: crate::road_network::Ground, cap: f64) -> f64 {
    if g.ocean {
        cap - (UNDERSEA_DECK_DEPTH - UNDERSEA_CROWN - UNDERSEA_COVER)
    } else {
        g.height + CLEARANCE
    }
}

pub(crate) fn sample_at(d: DVec3, radius: f64, terrain: &impl Survey) -> Result<Sample, String> {
    let ground = terrain.ground(d)?;
    if !ground.height.is_finite() {
        return Err("non-finite profile terrain".into());
    }
    let cap = if ground.ocean {
        // Canonical radial stencil: shared gateway height on both regions/faces.
        let reference = if d.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y };
        let u = d.cross(reference).normalize();
        let v = d.cross(u);
        let mut bed = ground.height;
        for offset in [u, -u, v, -v] {
            let g = terrain.ground((d + offset * (8.0 / radius)).normalize())?;
            if !g.height.is_finite() {
                return Err("non-finite tunnel cover".into());
            }
            bed = bed.min(g.height);
        }
        bed - UNDERSEA_CROWN - UNDERSEA_COVER
    } else {
        ground.height + PIER_CAP
    };
    Ok(Sample {
        ground,
        cap,
        anchor: anchor_height(ground, cap),
    })
}

impl Profile {
    pub fn fit(
        region: &Region,
        radius: f64,
        terrain: &impl Survey,
    ) -> Result<Result<Self, Rejected>, String> {
        Self::fit_fixed(region, radius, terrain, &[])
    }

    /// The height-aware router fixes shared lattice states as well as terminal
    /// anchors; do not subsequently slide a junction onto a different level.
    pub(crate) fn fit_fixed(
        region: &Region,
        radius: f64,
        terrain: &impl Survey,
        fixed: &[(usize, f64)],
    ) -> Result<Result<Self, Rejected>, String> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err("invalid network profile radius".into());
        }
        let mut dirs: Vec<_> = region
            .nodes
            .iter()
            .map(|n| DVec3::from_array(n.direction))
            .collect();
        let mut segments = Vec::new();
        for edge in &region.edges {
            let a = dirs[edge.a];
            let b = dirs[edge.b];
            let count = (distance(a, b, radius) / STEP).ceil().max(1.0) as usize;
            let mut last = edge.a;
            for i in 1..=count {
                let next = if i == count {
                    edge.b
                } else {
                    let j = dirs.len();
                    dirs.push(a.lerp(b, i as f64 / count as f64).normalize());
                    j
                };
                segments.push([last, next]);
                last = next;
            }
        }
        let mut ground = Vec::with_capacity(dirs.len());
        let mut caps = Vec::with_capacity(dirs.len());
        for &d in &dirs {
            let sample = sample_at(d, radius, terrain)?;
            ground.push(sample.ground);
            caps.push(sample.cap);
        }
        // Envelope edge lengths use a conservative lower radius. Heights
        // cannot fall below the lowest cap minus this anchor allowance, so
        // even a deep undersea tunnel respects the PHYSICAL grade limit.
        let cap_to_target =
            (PIER_CAP - CLEARANCE).max(UNDERSEA_DECK_DEPTH - UNDERSEA_CROWN - UNDERSEA_COVER);
        let minimum_height = (caps.iter().copied().reduce(f64::min).unwrap_or(0.0) - cap_to_target)
            .min(fixed.iter().map(|(_, h)| *h).fold(f64::INFINITY, f64::min));
        let minimum_radius = radius + minimum_height;
        if minimum_radius <= 0.0 {
            return Err("profile reaches the planet centre".into());
        }
        let mut adjacency = vec![Vec::new(); dirs.len()];
        for &[a, b] in &segments {
            let length = distance(dirs[a], dirs[b], minimum_radius);
            adjacency[a].push((b, length));
            adjacency[b].push((a, length));
        }
        let undersea_roofs: Vec<_> = ground
            .iter()
            .zip(&caps)
            .map(|(g, cap)| g.ocean.then_some(cap + UNDERSEA_COVER))
            .collect();
        let mut low_sources = vec![f64::INFINITY; dirs.len()];
        let mut upper_sources = caps;
        for &i in &region.terminal_nodes {
            let g = ground[i];
            let anchor = anchor_height(g, upper_sources[i]);
            low_sources[i] = -anchor;
            upper_sources[i] = anchor;
        }
        for &(i, height) in fixed {
            if i >= region.nodes.len() || !height.is_finite() {
                return Err("invalid fixed road-profile state".into());
            }
            if height > upper_sources[i] + 1e-6 {
                return Ok(Err(Rejected {
                    reason: "fixed state exceeds clearance/cover cap",
                    node: i,
                    required_minimum_meters: height,
                    allowed_maximum_meters: upper_sources[i],
                }));
            }
            // A fixed state may coincide with a terminal, but cannot change it.
            if low_sources[i].is_finite() && (height + low_sources[i]).abs() > 1e-6 {
                return Err("fixed state moved a village/gateway anchor".into());
            }
            low_sources[i] = -height;
            upper_sources[i] = height;
        }
        let lower: Vec<_> = envelope(low_sources, &adjacency)
            .into_iter()
            .map(|h| -h)
            .collect();
        let upper = envelope(upper_sources, &adjacency);
        for i in 0..dirs.len() {
            if lower[i] > upper[i] + 1e-6 {
                return Ok(Err(Rejected {
                    reason: "alignment needs a longer/alternative graded route",
                    node: i,
                    required_minimum_meters: lower[i],
                    allowed_maximum_meters: upper[i],
                }));
            }
        }
        let desired = ground
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let h = if g.ocean {
                    g.height - UNDERSEA_DECK_DEPTH
                } else {
                    g.height + CLEARANCE
                };
                h.max(lower[i]).min(upper[i])
            })
            .collect();
        let heights = envelope(desired, &adjacency);
        let maximum_grade = segments
            .iter()
            .map(|&[a, b]| {
                (heights[a] - heights[b]).abs()
                    / distance(dirs[a], dirs[b], radius + heights[a].min(heights[b])).max(1e-9)
            })
            .fold(0.0, f64::max);
        let maximum_deck_above_ground_meters = ground
            .iter()
            .zip(&heights)
            .map(|(g, h)| h - g.height)
            .fold(0.0, f64::max);
        let min_cover = undersea_roofs
            .iter()
            .zip(&heights)
            .filter_map(|(roof, h)| roof.map(|r| r - h))
            .reduce(f64::min);
        let undersea_segments = segments
            .iter()
            .filter(|&&[a, b]| ground[a].ocean || ground[b].ocean)
            .count();
        Ok(Ok(Self {
            heights_meters: heights,
            directions: dirs.into_iter().map(|d| d.to_array()).collect(),
            segments,
            maximum_grade,
            maximum_deck_above_ground_meters,
            minimum_sampled_undersea_roof_cover_meters: min_cover,
            undersea_segments,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_network::{Ground, Node, RoadEdge, Terminal, TerminalKind};
    use planet_coretypes::{CubeFace, TileKey};
    #[test]
    fn road_network_profile_allows_twenty_percent_but_rejects_twenty_six_percent() {
        struct Ramp(f64);
        impl Survey for Ramp {
            fn ground(&self, d: DVec3) -> Result<Ground, String> {
                Ok(Ground {
                    height: 1000.0 + self.0 * d.y / d.x * 4e6,
                    ocean: false,
                })
            }
            fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
                Ok(Vec::new())
            }
        }
        for (grade, accepted) in [(0.20, true), (0.26, false)] {
            let result = Profile::fit(&tree(), 4e6, &Ramp(grade)).unwrap();
            assert_eq!(result.is_ok(), accepted, "physical grade {grade}");
            if let Ok(profile) = result {
                assert!(profile.maximum_grade <= 0.25);
            }
        }
    }

    fn direction(x: f64, y: f64) -> DVec3 {
        DVec3::new(1.0, x / 4e6, y / 4e6).normalize()
    }
    struct Surface {
        steep: bool,
        water: bool,
    }
    impl Survey for Surface {
        fn ground(&self, d: DVec3) -> Result<Ground, String> {
            let x = d.y / d.x * 4e6;
            Ok(Ground {
                height: if self.water {
                    -100.0
                } else if self.steep {
                    x * 0.5
                } else {
                    900.0 + 30.0 * (-((x - 500.0) / 80.0).powi(2)).exp()
                },
                ocean: self.water,
            })
        }
        fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
            Ok(Vec::new())
        }
    }
    fn tree() -> Region {
        let dirs = [
            direction(0.0, 0.0),
            direction(500.0, 0.0),
            direction(1000.0, 0.0),
            direction(500.0, 500.0),
        ];
        Region {
            key: TileKey {
                face: CubeFace::PositiveX,
                level: 8,
                x: 128,
                y: 128,
            },
            village_count: 3,
            terminals: [0, 2, 3]
                .into_iter()
                .map(|i| Terminal {
                    direction: dirs[i].to_array(),
                    kind: TerminalKind::Village,
                })
                .collect(),
            terminal_nodes: vec![0, 2, 3],
            nodes: dirs
                .into_iter()
                .map(|d| Node {
                    direction: d.to_array(),
                    ground_meters: 900.0,
                    ocean: false,
                })
                .collect(),
            edges: [(0, 1), (1, 2), (1, 3)]
                .into_iter()
                .map(|(a, b)| RoadEdge {
                    a,
                    b,
                    requires_undersea_tunnel: false,
                })
                .collect(),
            junction_count: 1,
            length_meters: 1500.0,
            construction_cost_proxy: 1500.0,
            lattice_survey_samples: 0,
        }
    }
    #[test]
    fn road_network_profile_junction_is_shared_and_all_segments_meet_grade() {
        let r = tree();
        let surface = Surface {
            steep: false,
            water: false,
        };
        let p = Profile::fit(&r, 4e6, &surface).unwrap().unwrap();
        assert!(p.maximum_grade <= GRADE + 1e-8);
        assert!(p.maximum_deck_above_ground_meters <= PIER_CAP + 1e-6);
        assert_eq!(p.segments.iter().filter(|s| s.contains(&1)).count(), 3);
        for &i in &r.terminal_nodes {
            assert!(
                (p.heights_meters[i]
                    - surface
                        .ground(direction(
                            if i == 0 {
                                0.0
                            } else if i == 2 {
                                1000.0
                            } else {
                                500.0
                            },
                            0.0
                        ))
                        .unwrap()
                        .height
                    - CLEARANCE)
                    .abs()
                    < 1e-6
            );
        }
    }
    #[test]
    fn road_network_profile_undersea_roof_is_below_the_bed() {
        let p = Profile::fit(
            &tree(),
            4e6,
            &Surface {
                steep: false,
                water: true,
            },
        )
        .unwrap()
        .unwrap();
        assert!(p.minimum_sampled_undersea_roof_cover_meters.unwrap() >= UNDERSEA_COVER);
        assert_eq!(p.undersea_segments, p.segments.len());
        assert!(p.heights_meters.iter().all(|h| *h < -100.0));
    }
    #[test]
    fn road_network_profile_rejects_impossible_grades_instead_of_moving_terminal_heights() {
        let p = Profile::fit(
            &tree(),
            4e6,
            &Surface {
                steep: true,
                water: false,
            },
        )
        .unwrap();
        assert!(p.is_err());
    }
}

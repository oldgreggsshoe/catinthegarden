//! Cached route, graded viaduct mesh and a matching local terrain-cut field.
//! Positions are metres in a planet-fixed tangent frame (x across, y along,
//! z up). The route search is one-time; queries touch only nearby segments.
use crate::road_route::{Route, SAMPLE_STEP};
use glam::{DVec2, DVec3};
use std::sync::Arc;

pub const HALF_WIDTH: f64 = 5.0;
pub const CUT_FLOOR_DEPTH: f64 = 6.0;
pub const CUT_HALF_WIDTH: f64 = 9.0;
pub const CUT_BUFFER_FLOAT4S: usize = 4 + 3001;
const DECK_THICKNESS: f64 = 1.0;
const CHUNK_SEGMENTS: usize = 50;

pub fn enabled() -> bool {
    matches!(
        std::env::var("PLANET_ROAD_EXPERIMENT")
            .as_deref()
            .map(str::trim),
        Ok("1" | "true" | "on")
    )
}
pub fn cut_shader_source() -> String {
    format!(
        "const ROAD_CUTS_ENABLED: bool = {};\n{}",
        enabled() && crate::subsystem_enabled("road_cuts"),
        include_str!("road_cuts.wgsl")
    )
}
pub fn is_replay(name: &str) -> bool {
    matches!(
        name,
        "road_surface_trial"
            | "road_viaduct_overview"
            | "road_viaduct_performance"
            | "road_route_survey"
    )
}
pub fn up() -> DVec3 {
    DVec3::new(0.94919006, 0.28950297, 0.12339471).normalize()
}
pub fn forward() -> DVec3 {
    let f = DVec3::new(-0.11802703, -0.03599824, 0.99235767);
    (f - up() * f.dot(up())).normalize()
}
pub fn across() -> DVec3 {
    forward().cross(up())
}
pub fn origin() -> DVec3 {
    up() * crate::planet::planet_radius_meters()
}
pub fn world(local: DVec3) -> DVec3 {
    origin() + across() * local.x + forward() * local.y + up() * local.z
}
pub fn local(position: DVec3) -> DVec3 {
    let p = position - origin();
    DVec3::new(p.dot(across()), p.dot(forward()), p.dot(up()))
}

#[derive(Clone)]
pub struct CutField {
    pub points: Vec<DVec3>,
}
impl CutField {
    /// Uniform y indexing makes this O(1), independent of road length.
    pub fn height(&self, direction: DVec3, uncut_height: f64) -> f64 {
        let radius = crate::planet::planet_radius_meters();
        let cosine = direction.dot(up());
        if cosine < 0.99 {
            return uncut_height;
        }
        let mut height = uncut_height;
        for _ in 0..2 {
            let p = local(direction * (radius + height));
            let last = self.points.last().unwrap().y;
            if p.x.abs() > 3600.0 || p.y < -500.0 || p.y > last + 500.0 {
                return uncut_height;
            }
            let at = (p.y / SAMPLE_STEP).clamp(0.0, (self.points.len() - 1) as f64 - 1.0e-9);
            let i = at.floor() as usize;
            let a = self.points[i];
            let b = self.points[i + 1];
            let centre = a.lerp(b, at.fract());
            let dx = (b.x - a.x) / (b.y - a.y);
            let distance = (p.x - centre.x).abs() / (1.0 + dx * dx).sqrt();
            let end_distance = (-p.y).max(p.y - last).max(0.0);
            let cap =
                centre.z - CUT_FLOOR_DEPTH + (distance - CUT_HALF_WIDTH).max(0.0) + end_distance;
            height = uncut_height.min((radius + cap) / cosine - radius);
        }
        height
    }
    pub fn gpu_data(&self) -> Vec<[f32; 4]> {
        let v = |p: DVec3| [p.x as f32, p.y as f32, p.z as f32, 0.0];
        let mut data = vec![
            [
                crate::planet::planet_radius_meters() as f32,
                SAMPLE_STEP as f32,
                self.points.len() as f32,
                1.0,
            ],
            v(across()),
            v(forward()),
            v(up()),
        ];
        data.extend(self.points.iter().copied().map(v));
        data.resize(CUT_BUFFER_FLOAT4S, [0.0; 4]);
        data
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Pier {
    pub local_foot: DVec3,
    pub top: f64,
}
pub struct MeshChunk {
    pub vertices: Vec<Vertex>,
    pub far_vertices: Vec<Vertex>,
    pub centre: DVec3,
    pub radius: f64,
}
pub struct Viaduct {
    pub route: Route,
    pub cuts: Arc<CutField>,
    pub piers: Vec<Pier>,
    deepest_cut: usize,
    tallest_pier: usize,
}
impl Viaduct {
    pub fn survey(mut height_at: impl FnMut(DVec3) -> Option<f64>) -> Option<Self> {
        let mut ground = |p: DVec2| -> Option<f64> {
            let mut z = 0.0;
            for _ in 0..2 {
                let d = world(p.extend(z)).normalize();
                let h = height_at(d)?;
                if !h.is_finite() || h <= 0.0 {
                    return None;
                }
                z = local(d * (crate::planet::planet_radius_meters() + h)).z;
            }
            Some(z)
        };
        let route = Route::plan(&mut ground)?;
        let maximum_grade = route
            .points
            .windows(2)
            .map(|p| {
                let a = world(p[0]);
                let b = world(p[1]);
                (b.length() - a.length()).abs()
                    / (a.normalize().angle_between(b.normalize())
                        * crate::planet::planet_radius_meters())
            })
            .fold(0.0_f64, f64::max);
        if maximum_grade > crate::road_route::MAX_GRADE {
            return None;
        }
        tracing::info!(maximum_grade, "road route grade checked on planet");
        let cuts = Arc::new(CutField {
            points: route.points.clone(),
        });
        let mut piers = Vec::new();
        for i in (0..route.points.len()).step_by(3) {
            let p = route.points[i];
            let side = lateral(&route.points, i);
            for offset in [-3.0, 3.0] {
                let mut foot = p + side * offset;
                let uncut = ground(foot.truncate())?;
                // The floor is at least 6m below the deck; the footing is
                // buried in the carved ground, not the old hillside.
                foot.z = uncut.min(p.z - CUT_FLOOR_DEPTH) - 2.0;
                piers.push(Pier {
                    local_foot: foot,
                    top: p.z - DECK_THICKNESS,
                });
            }
        }
        let deepest_cut = route
            .ground
            .iter()
            .zip(&route.points)
            .enumerate()
            .max_by(|(_, (a, p)), (_, (b, q))| (**a - p.z).total_cmp(&(**b - q.z)))?
            .0;
        let tallest_pier = route
            .ground
            .iter()
            .zip(&route.points)
            .enumerate()
            .max_by(|(_, (a, p)), (_, (b, q))| (p.z - **a).total_cmp(&(q.z - **b)))?
            .0;
        Some(Self {
            route,
            cuts,
            piers,
            deepest_cut,
            tallest_pier,
        })
    }
    /// Profile intersection for diagnostics, extended at the route ends.
    pub fn plane_height_at(&self, direction: DVec3) -> f64 {
        let r = crate::planet::planet_radius_meters();
        let mut height = self.route.points[0].z;
        for _ in 0..2 {
            let p = local(direction * (r + height));
            height = (r + self.route.at_y(p.y).z) / direction.dot(up()) - r;
        }
        height
    }
    pub fn deck_height_at(&self, direction: DVec3) -> Option<f64> {
        if direction.dot(up()) < 0.99 {
            return None;
        }
        let p = local(
            direction * (crate::planet::planet_radius_meters() + self.plane_height_at(direction)),
        );
        if p.y < -5.0 || p.y > self.route.points.last()?.y + 5.0 {
            return None;
        }
        if (p.x - self.route.at_y(p.y).x).abs() > HALF_WIDTH * 2.0 {
            return None;
        }
        let i = (p.y / SAMPLE_STEP).floor() as i32;
        for j in (i - 2).max(0)..=(i + 2).min(self.route.points.len() as i32 - 2) {
            let j = j as usize;
            let a = self.route.points[j];
            let b = self.route.points[j + 1];
            let l = lateral(&self.route.points, j) * HALF_WIDTH;
            let m = lateral(&self.route.points, j + 1) * HALF_WIDTH;
            for triangle in [[a - l, a + l, b + m], [a - l, b + m, b - m]] {
                if let Some(h) = radial_triangle_height(direction, triangle) {
                    return Some(h);
                }
            }
        }
        None
    }
    pub fn walkable_height(&self, direction: DVec3, eye_altitude: f64) -> Option<f64> {
        self.deck_height_at(direction)
            .filter(|h| eye_altitude >= *h - 0.25)
    }
    pub fn inspection_pose(&self) -> (DVec3, DVec3) {
        self.drive_pose(40.0, 2.0)
    }
    pub fn drive_pose(&self, distance: f64, eye_height: f64) -> (DVec3, DVec3) {
        let p = world(self.route.at_distance(distance));
        let q = world(
            self.route
                .at_distance((distance + 45.0).min(self.route.length())),
        );
        let eye = p + p.normalize() * eye_height;
        (eye, q + q.normalize() * eye_height)
    }
    pub fn scenario_pose(&self, name: &str, time: f64) -> Option<(DVec3, DVec3)> {
        match name {
            "road_surface_trial" => Some(self.drive_pose(14.0 * time, 2.0)),
            "road_viaduct_performance" => Some(self.drive_pose(140.0, 2.0)),
            "road_route_survey" => {
                let index = (time / 3.0).floor().min(5.0);
                let distance = if index == 1.0 {
                    self.route.distances[self.deepest_cut]
                } else {
                    100.0 + index * (self.route.length() - 300.0) / 5.0
                };
                Some(self.drive_pose(distance.min(self.route.length() - 50.0), 2.0))
            }
            "road_viaduct_overview" => {
                let index = if time < 2.5 {
                    0
                } else if time < 4.5 {
                    1
                } else {
                    2
                };
                let i = match index {
                    0 => self.deepest_cut,
                    1 => self.tallest_pier,
                    _ => self.route.points.len() / 2,
                };
                let target = self.route.points[i];
                let offset = if index == 2 {
                    DVec3::new(-9000.0, 0.0, 11000.0)
                } else if index == 0 {
                    lateral(&self.route.points, i) * -100.0 + DVec3::Z * 160.0
                } else {
                    lateral(&self.route.points, i) * -180.0 + DVec3::Z * 55.0
                };
                Some((world(target + offset), world(target)))
            }
            _ => None,
        }
    }
    pub fn mesh(&self) -> Vec<MeshChunk> {
        let mut chunks = Vec::new();
        let concrete = [0.32, 0.31, 0.29];
        for start in (0..self.route.points.len() - 1).step_by(CHUNK_SEGMENTS) {
            let end = (start + CHUNK_SEGMENTS).min(self.route.points.len() - 1);
            let mut vertices = Vec::new();
            let mut slab = |width: f64, bottom: f64, top: f64, offset: f64, colour: [f32; 3]| {
                for i in start..end {
                    let a = self.route.points[i] + DVec3::Z * bottom;
                    let b = self.route.points[i + 1] + DVec3::Z * bottom;
                    let s = lateral(&self.route.points, i);
                    let t = lateral(&self.route.points, i + 1);
                    let l0 = a + s * (offset - width);
                    let r0 = a + s * (offset + width);
                    let l1 = b + t * (offset - width);
                    let r1 = b + t * (offset + width);
                    let rise = DVec3::Z * (top - bottom);
                    quad(
                        &mut vertices,
                        [l0 + rise, r0 + rise, r1 + rise, l1 + rise],
                        colour,
                    );
                    quad(&mut vertices, [r0, l0, l1, r1], concrete);
                    quad(&mut vertices, [l0, l0 + rise, l1 + rise, l1], concrete);
                    quad(&mut vertices, [r0 + rise, r0, r1, r1 + rise], concrete);
                    if i == 0 {
                        quad(&mut vertices, [r0, r0 + rise, l0 + rise, l0], concrete);
                    }
                    if i + 1 == self.route.points.len() - 1 {
                        quad(&mut vertices, [l1, l1 + rise, r1 + rise, r1], concrete);
                    }
                }
            };
            slab(HALF_WIDTH, -DECK_THICKNESS, 0.0, 0.0, [0.045, 0.048, 0.05]);
            slab(0.18, 0.0, 0.9, -HALF_WIDTH + 0.18, concrete);
            slab(0.18, 0.0, 0.9, HALF_WIDTH - 0.18, concrete);
            for i in start..end {
                let a = self.route.points[i] + DVec3::Z * 0.015;
                let b = a.lerp(self.route.points[i + 1] + DVec3::Z * 0.015, 0.4);
                let s = lateral(&self.route.points, i) * 0.07;
                quad(
                    &mut vertices,
                    [a - s, a + s, b + s, b - s],
                    [0.72, 0.68, 0.49],
                );
            }
            let y0 = self.route.points[start].y;
            let y1 = self.route.points[end].y;
            for pair in self
                .piers
                .chunks_exact(2)
                .filter(|p| p[0].local_foot.y >= y0 - 2.0 && p[0].local_foot.y < y1 - 2.0)
            {
                for pier in pair {
                    let height = pier.top - pier.local_foot.z;
                    let half = (height * 0.012).clamp(0.7, 2.0);
                    box_mesh(
                        &mut vertices,
                        pier.local_foot - DVec3::new(half, half, 0.0),
                        pier.local_foot + DVec3::new(half, half, height),
                        concrete,
                    );
                }
                let a = pair[0].local_foot;
                let b = pair[1].local_foot;
                let top = pair[0].top.min(pair[1].top);
                box_mesh(
                    &mut vertices,
                    DVec3::new(a.x.min(b.x) - 1.0, a.y.min(b.y) - 1.0, top - 1.3),
                    DVec3::new(a.x.max(b.x) + 1.0, a.y.max(b.y) + 1.0, top),
                    concrete,
                );
            }
            let centre = self.route.points[(start + end) / 2];
            let radius = vertices
                .iter()
                .map(|v| DVec3::from_array(v.position.map(f64::from)).distance(centre))
                .fold(0.0, f64::max);
            let mut far_vertices = Vec::new();
            for i in (start..end).step_by(4) {
                let j = (i + 4).min(end);
                let a = self.route.points[i];
                let b = self.route.points[j];
                let l = lateral(&self.route.points, i) * HALF_WIDTH;
                let m = lateral(&self.route.points, j) * HALF_WIDTH;
                quad(
                    &mut far_vertices,
                    [a - l, a + l, b + m, b - m],
                    [0.045, 0.048, 0.05],
                );
            }
            chunks.push(MeshChunk {
                vertices,
                far_vertices,
                centre,
                radius,
            });
        }
        chunks
    }
}
fn lateral(points: &[DVec3], i: usize) -> DVec3 {
    let d = points[(i + 1).min(points.len() - 1)] - points[i.saturating_sub(1)];
    DVec3::new(d.y, -d.x, 0.0).normalize()
}
fn radial_triangle_height(direction: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let r = crate::planet::planet_radius_meters();
    let d = DVec3::new(
        direction.dot(across()),
        direction.dot(forward()),
        direction.dot(up()),
    );
    let normal = (b - a).cross(c - a);
    let radius = (normal.dot(a) + normal.z * r) / normal.dot(d);
    let p = d * radius - DVec3::Z * r;
    if [(a, b), (b, c), (c, a)]
        .iter()
        .all(|(a, b)| (b - a).cross(p - a).dot(normal) >= -1.0e-5 * normal.length())
    {
        Some(radius - r)
    } else {
        None
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub colour: [f32; 3],
}

fn quad(mesh: &mut Vec<Vertex>, points: [DVec3; 4], colour: [f32; 3]) {
    let normal = (points[1] - points[0])
        .cross(points[2] - points[0])
        .normalize()
        .as_vec3();
    for i in [0, 1, 2, 0, 2, 3] {
        mesh.push(Vertex {
            position: points[i].as_vec3().to_array(),
            normal: normal.to_array(),
            colour,
        });
    }
}

fn box_mesh(mesh: &mut Vec<Vertex>, lo: DVec3, hi: DVec3, colour: [f32; 3]) {
    let p = |x: bool, y: bool, z: bool| {
        DVec3::new(
            if x { hi.x } else { lo.x },
            if y { hi.y } else { lo.y },
            if z { hi.z } else { lo.z },
        )
    };
    for corners in [
        [
            (false, false, true),
            (true, false, true),
            (true, true, true),
            (false, true, true),
        ],
        [
            (false, true, false),
            (true, true, false),
            (true, false, false),
            (false, false, false),
        ],
        [
            (false, false, false),
            (true, false, false),
            (true, false, true),
            (false, false, true),
        ],
        [
            (true, true, false),
            (false, true, false),
            (false, true, true),
            (true, true, true),
        ],
        [
            (false, true, false),
            (false, false, false),
            (false, false, true),
            (false, true, true),
        ],
        [
            (true, false, false),
            (true, true, false),
            (true, true, true),
            (true, false, true),
        ],
    ] {
        quad(mesh, corners.map(|(x, y, z)| p(x, y, z)), colour);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> Viaduct {
        Viaduct::survey(|d| Some(1900.0 + 35.0 * (d.dot(forward()) * 10000.0).sin())).unwrap()
    }
    #[test]
    fn graded_road_stays_within_ten_percent_on_the_curved_planet() {
        let p = plan();
        for pair in p.route.points.windows(2) {
            let a = world(pair[0]);
            let b = world(pair[1]);
            let rise = (b.length() - a.length()).abs();
            let run =
                a.normalize().angle_between(b.normalize()) * crate::planet::planet_radius_meters();
            assert!(
                rise / run <= crate::road_route::MAX_GRADE,
                "grade {}",
                rise / run
            );
        }
    }
    #[test]
    fn road_inspection_starts_above_the_deck_and_underpass_stays_open() {
        let p = plan();
        let (eye, _) = p.inspection_pose();
        let h = eye.length() - crate::planet::planet_radius_meters();
        assert!((h - p.deck_height_at(eye.normalize()).unwrap() - 2.0).abs() < 0.01);
        assert!(p.walkable_height(eye.normalize(), h - 15.0).is_none());
        let off = world(p.route.at_distance(100.0) + DVec3::X * 40.0);
        assert!(p.deck_height_at(off.normalize()).is_none());
        assert!(Viaduct::survey(|_| None).is_none());
    }
    #[test]
    fn terrain_cut_only_removes_material_and_leaves_far_ground_untouched() {
        let p = plan();
        for distance in [100.0, 5000.0, 15000.0, 29000.0] {
            let deck = world(p.route.at_distance(distance));
            let d = deck.normalize();
            let h = deck.length() - crate::planet::planet_radius_meters();
            assert!((p.cuts.height(d, h + 30.0) - (h - CUT_FLOOR_DEPTH)).abs() < 0.05);
            assert_eq!(p.cuts.height(d, h - 20.0), h - 20.0);
            let outside = (deck + across() * 1000.0).normalize();
            assert_eq!(p.cuts.height(outside, h + 30.0), h + 30.0);
        }
        assert_eq!(p.cuts.gpu_data().len(), CUT_BUFFER_FLOAT4S);
    }
    #[test]
    fn road_mesh_and_collider_match_with_bounded_local_batches() {
        let p = plan();
        let chunks = p.mesh();
        assert!(chunks.len() <= 61);
        for chunk in chunks {
            assert!(chunk.vertices.len() / 3 < 3000);
            for triangle in chunk
                .vertices
                .chunks_exact(3)
                .filter(|t| t[0].colour == [0.045, 0.048, 0.05])
            {
                let local = triangle
                    .iter()
                    .map(|v| DVec3::from_array(v.position.map(f64::from)))
                    .sum::<DVec3>()
                    / 3.0;
                let w = world(local);
                assert!(
                    (p.deck_height_at(w.normalize()).unwrap()
                        - (w.length() - crate::planet::planet_radius_meters()))
                    .abs()
                        < 0.01
                );
            }
            assert!(
                chunk
                    .vertices
                    .iter()
                    .all(|v| glam::Vec3::from_array(v.normal).is_finite())
            );
        }
    }
}

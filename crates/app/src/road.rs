//! Cached route, graded viaduct mesh and a matching local terrain-cut field.
//! Positions are metres in a planet-fixed tangent frame (x across, y along,
//! z up). The route search is one-time; queries touch only nearby segments.
use crate::road_route::{Route, SAMPLE_STEP};
use glam::{DVec2, DVec3};
use std::sync::Arc;

pub const HALF_WIDTH: f64 = 5.0;
pub const CUT_FLOOR_DEPTH: f64 = 6.0;
pub const CUT_HALF_WIDTH: f64 = 9.0;
/// Shoulder plus the largest full-tree crown radius, so boughs cannot overhang.
pub const TREE_CLEARANCE_METERS: f64 = 25.0;
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
        (enabled() || crate::road_network_render::enabled())
            && crate::subsystem_enabled("road_cuts"),
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
            | "road_tunnel_trial"
            | "road_tunnel_walk"
            | "road_tunnel_performance"
    )
}
pub fn up() -> DVec3 {
    // Alpine ridge country: the westbound trial crosses steep natural
    // relief and covered ridge tunnels; the allowed road grade is 25%.
    DVec3::new(0.45242405006232295, 0.5402686479569877, 0.7095225626855942).normalize()
}
pub fn forward() -> DVec3 {
    static HEADING: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    let heading = *HEADING.get_or_init(|| {
        std::env::var("PLANET_ROAD_HEADING")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(270.0)
            .to_radians()
    });
    let north = (DVec3::Z - up() * up().z).normalize();
    glam::DQuat::from_axis_angle(up(), heading) * north
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
    pub tunnel_segments: Vec<bool>,
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
            if self.tunnel_segments[i] {
                return uncut_height;
            }
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
    pub fn clears_tree(&self, direction: DVec3, uncut_height: f64) -> bool {
        if direction.dot(up()) < 0.99 {
            return false;
        }
        let p = local(direction * (crate::planet::planet_radius_meters() + uncut_height));
        let end = self.points.last().unwrap().y;
        if p.y < -TREE_CLEARANCE_METERS || p.y > end + TREE_CLEARANCE_METERS {
            return false;
        }
        let at = (p.y / SAMPLE_STEP).clamp(0.0, (self.points.len() - 1) as f64 - 1e-9);
        let i = at.floor() as usize;
        if self.tunnel_segments[i] {
            return false;
        }
        let a = self.points[i];
        let b = self.points[i + 1];
        let centre = a.lerp(b, at.fract());
        let slope = (b.x - a.x) / (b.y - a.y);
        (p.x - centre.x).abs() / (1.0 + slope * slope).sqrt() <= TREE_CLEARANCE_METERS
            || self.height(direction, uncut_height) < uncut_height - 0.25
    }
    /// Local bore frame, including the one-station portal approach. The
    /// outside hillside remains the ordinary heightfield above the roof.
    pub fn tunnel_frame(&self, p: DVec3) -> Option<(DVec3, f64)> {
        if p.x.abs() > 3600.0 || p.y < 0.0 || p.y >= self.points.last()?.y {
            return None;
        }
        let station = p.y / SAMPLE_STEP;
        let i = station.floor() as usize;
        if !self.tunnel_segments.get(i).copied().unwrap_or(false)
            && !self.tunnel_segments.get(i + 1).copied().unwrap_or(false)
        {
            return None;
        }
        let a = self.points[i];
        let b = self.points[i + 1];
        let centre = a.lerp(b, station.fract());
        let slope = (b.x - a.x) / (b.y - a.y);
        Some((centre, (p.x - centre.x) / (1.0 + slope * slope).sqrt()))
    }
    /// Positive inside the excavated air volume; matching WGSL CSG operation.
    pub fn tunnel_void(&self, direction: DVec3, height: f64) -> f64 {
        self.local_tunnel_void(
            local(direction * (crate::planet::planet_radius_meters() + height)),
            0.5,
        )
    }
    fn local_tunnel_void(&self, p: DVec3, margin: f64) -> f64 {
        let Some((centre, offset)) = self.tunnel_frame(p) else {
            return -1.0e6;
        };
        (crate::road_tunnel::HALF_WIDTH + margin - offset.abs())
            .min(p.z - centre.z - crate::road_tunnel::FLOOR + margin)
            .min(crate::road_tunnel::roof(offset) + margin - (p.z - centre.z))
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
        data.extend(self.points.iter().copied().enumerate().map(|(i, p)| {
            let mut sample = v(p);
            sample[3] = f32::from(self.tunnel_segments.get(i).copied().unwrap_or(false));
            sample
        }));
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
    pub structure_vertices: Vec<Vertex>,
    pub centre: DVec3,
    pub radius: f64,
}
pub struct Viaduct {
    pub route: Route,
    pub cuts: Arc<CutField>,
    pub piers: Vec<Pier>,
    pub tunnels: Vec<crate::road_tunnel::Tunnel>,
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
        let tunnels = if std::env::var("PLANET_ROAD_TUNNELS").as_deref() == Ok("0") {
            Vec::new()
        } else {
            crate::road_tunnel::select(&route, &mut ground)
        };
        let mut tunnel_segments = vec![false; route.points.len() - 1];
        for tunnel in &tunnels {
            tunnel_segments[tunnel.start..tunnel.end].fill(true);
        }
        let cuts = Arc::new(CutField {
            points: route.points.clone(),
            tunnel_segments,
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
            tunnels,
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
    /// Camera floor inside a bore is the deck, never the hillside roof.
    pub fn camera_surface_height(&self, direction: DVec3, eye: f64, terrain: f64) -> f64 {
        if self.cuts.tunnel_void(direction, eye) > 0.0 {
            if let Some(deck) = self.walkable_height(direction, eye) {
                return deck;
            }
            let p = local(direction * (crate::planet::planet_radius_meters() + eye));
            if let Some((centre, _)) = self.cuts.tunnel_frame(p) {
                let floor = world(DVec3::new(p.x, p.y, centre.z + crate::road_tunnel::FLOOR));
                return floor.length() - crate::planet::planet_radius_meters();
            }
        }
        self.walkable_height(direction, eye)
            .map_or(terrain, |deck| terrain.max(deck))
    }
    /// Sweep both entering and leaving cameras against the physical lining.
    /// Only longitudinal portal crossings are open; retain a 20cm eye margin.
    pub fn constrain_tunnel_motion(&self, from: DVec3, to: DVec3) -> DVec3 {
        if self.tunnels.is_empty() {
            return to;
        }
        let a = local(from);
        let b = local(to);
        if a.x.min(b.x) > 3600.0
            || a.x.max(b.x) < -3600.0
            || a.y.max(b.y) < 0.0
            || a.y.min(b.y) > self.route.points.last().unwrap().y
        {
            return to;
        }
        let steps = ((a.distance(b) / 0.5).ceil() as usize).clamp(1, 512);
        let mut previous = a;
        let mut previous_air = self.cuts.local_tunnel_void(previous, 0.0) > 0.0;
        for step in 1..=steps {
            let candidate = a.lerp(b, step as f64 / steps as f64);
            let air = self.cuts.local_tunnel_void(candidate, 0.0) > 0.0;
            if air != previous_air
                && self.cuts.tunnel_frame(previous).is_some()
                && self.cuts.tunnel_frame(candidate).is_some()
            {
                let mut low = previous;
                let mut high = candidate;
                for _ in 0..20 {
                    let mid = (low + high) * 0.5;
                    if (self.cuts.local_tunnel_void(mid, 0.0) > 0.0) == previous_air {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                let retreat = 0.2_f64.min(low.distance(a));
                return world(low - (b - a).normalize_or_zero() * retreat);
            }
            previous = candidate;
            previous_air = air;
        }
        to
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
            "road_tunnel_trial" => {
                let tunnel = self
                    .tunnels
                    .first()
                    .expect("tunnel replay needs a covered deep-cut section");
                let entry = self.route.distances[tunnel.start.saturating_sub(1)];
                let exit = self.route.distances[tunnel.end];
                if time <= 10.0 {
                    // Separate short, normal-speed entry/interior/exit shots;
                    // do not race through a many-kilometre bore in ten seconds.
                    let distance = if time < 3.0 {
                        entry - 35.0 + 14.0 * time
                    } else if time < 6.0 {
                        (entry + exit) * 0.5 - 21.0 + 14.0 * (time - 3.0)
                    } else {
                        exit - 35.0 + 14.0 * (time - 6.0)
                    };
                    Some(self.drive_pose(distance, 2.0))
                } else {
                    let i = if time < 13.0 {
                        tunnel.start.saturating_sub(1)
                    } else {
                        (tunnel.start + tunnel.end) / 2
                    };
                    let target = self.route.points[i] + DVec3::Z * 4.0;
                    let offset = if time < 13.0 {
                        let d = self.route.points[(i + 1).min(self.route.points.len() - 1)]
                            - self.route.points[i];
                        -d.normalize() * 60.0 + DVec3::Z * 12.0
                    } else {
                        lateral(&self.route.points, i) * -130.0 + DVec3::Z * 190.0
                    };
                    Some((world(target + offset), world(target)))
                }
            }
            "road_tunnel_walk" => {
                let t = self
                    .tunnels
                    .first()
                    .expect("tunnel walk replay needs a covered deep-cut section");
                Some(self.drive_pose(
                    (self.route.distances[t.start.saturating_sub(1)] - 15.0).max(0.0),
                    1.7,
                ))
            }
            // This pose depends on the route, not tunnel classification, so
            // PLANET_ROAD_TUNNELS=0 keeps the before/after camera identical.
            "road_tunnel_performance" => {
                Some(self.drive_pose(self.route.distances[self.deepest_cut], 2.0))
            }
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
                let tangent = |index: usize| {
                    let d = self.route.points[(index + 1).min(self.route.points.len() - 1)]
                        - self.route.points[index.saturating_sub(1)];
                    d / d.truncate().length() * HALF_WIDTH
                };
                far_segment(&mut far_vertices, a, b, tangent(i), tangent(j));
            }
            let structure_vertices =
                crate::road_tunnel::mesh(&self.route, &self.tunnels, start, end);
            for triangle in vertices.chunks_exact_mut(3) {
                let y = triangle
                    .iter()
                    .map(|v| f64::from(v.position[1]))
                    .sum::<f64>()
                    / 3.0;
                let i = (y / SAMPLE_STEP).floor().max(0.0) as usize;
                if self.cuts.tunnel_segments.get(i).copied().unwrap_or(false) {
                    for v in triangle {
                        v.colour[2] = -v.colour[2].abs();
                    }
                }
            }
            chunks.push(MeshChunk {
                structure_vertices,
                vertices,
                far_vertices,
                centre,
                radius,
            });
        }
        chunks
    }
}
pub(crate) fn lateral(points: &[DVec3], i: usize) -> DVec3 {
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

/// Far ribbon: centreline, half-width tangent, and signed red encode its sides.
/// Shared endpoint tangents keep expanded neighbouring segments joined.
pub(crate) fn far_segment(
    mesh: &mut Vec<Vertex>,
    a: DVec3,
    b: DVec3,
    tangent_a: DVec3,
    tangent_b: DVec3,
) {
    for (position, tangent, side) in [
        (a, tangent_a, -1.0),
        (a, tangent_a, 1.0),
        (b, tangent_b, 1.0),
        (a, tangent_a, -1.0),
        (b, tangent_b, 1.0),
        (b, tangent_b, -1.0),
    ] {
        mesh.push(Vertex {
            lighting: 0,
            position: position.as_vec3().to_array(),
            normal: tangent.as_vec3().to_array(),
            colour: [side * 0.045, 0.048, 0.05],
        });
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub colour: [f32; 3],
    pub lighting: u32,
}

pub(crate) fn quad(mesh: &mut Vec<Vertex>, points: [DVec3; 4], colour: [f32; 3]) {
    let normal = (points[1] - points[0])
        .cross(points[2] - points[0])
        .normalize()
        .as_vec3();
    for i in [0, 1, 2, 0, 2, 3] {
        mesh.push(Vertex {
            lighting: 0,
            position: points[i].as_vec3().to_array(),
            normal: normal.to_array(),
            colour,
        });
    }
}

/// Like `quad`, but records which way the road runs across this face in the
/// high 16 bits of the otherwise per-tunnel `lighting` lane, so the shader can
/// lay planks square to the road even on bends. 0 means "unknown".
pub(crate) fn quad_with_heading(
    mesh: &mut Vec<Vertex>,
    points: [DVec3; 4],
    colour: [f32; 3],
    along: DVec3,
) {
    let start = mesh.len();
    quad(mesh, points, colour);
    let turn = along.y.atan2(along.x).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
    let code = 1 + (turn * 65534.0) as u32;
    for v in &mut mesh[start..] {
        v.lighting |= code << 16;
    }
}

pub(crate) fn box_mesh(mesh: &mut Vec<Vertex>, lo: DVec3, hi: DVec3, colour: [f32; 3]) {
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
    fn graded_road_stays_within_twenty_five_percent_on_the_curved_planet() {
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
    fn tunnel_retains_roof_and_camera_stays_on_deck_without_escaping_walls() {
        let points: Vec<_> = (0..101)
            .map(|i| DVec3::new(0.0, i as f64 * 10.0, 100.0))
            .collect();
        let mut flags = vec![false; 100];
        flags[20..80].fill(true);
        let plan = Viaduct {
            cuts: Arc::new(CutField {
                points: points.clone(),
                tunnel_segments: flags,
            }),
            route: Route {
                points,
                ground: vec![140.0; 101],
                distances: (0..101).map(|i| i as f64 * 10.0).collect(),
                cut_volume: 0.0,
                straight_cut_volume: 0.0,
            },
            piers: Vec::new(),
            tunnels: vec![crate::road_tunnel::Tunnel { start: 20, end: 80 }],
            deepest_cut: 50,
            tallest_pier: 0,
        };
        let eye = world(DVec3::new(0.0, 500.0, 102.0));
        let direction = eye.normalize();
        let h = eye.length() - crate::planet::planet_radius_meters();
        let deck = plan.deck_height_at(direction).unwrap();
        assert!((plan.camera_surface_height(direction, h, h + 40.0) - deck).abs() < 0.001);
        assert_eq!(plan.cuts.height(direction, h + 40.0), h + 40.0);
        assert_eq!(
            plan.camera_surface_height(direction, h + 40.0, h + 38.0),
            h + 38.0
        );
        let wall = plan.constrain_tunnel_motion(eye, world(DVec3::new(20.0, 500.0, 102.0)));
        assert!((local(wall).x - 6.3).abs() < 0.001);
        let outside = world(DVec3::new(20.0, 500.0, 102.0));
        let entry = plan.constrain_tunnel_motion(outside, eye);
        assert!((local(entry).x - 6.7).abs() < 0.001);

        let ceiling = plan.constrain_tunnel_motion(eye, world(DVec3::new(0.0, 500.0, 130.0)));
        assert!((local(ceiling).z - 108.8).abs() < 0.001);
        let exit = world(DVec3::new(0.0, 820.0, 102.0));
        assert!(plan.constrain_tunnel_motion(eye, exit).distance(exit) < 1e-6);
        assert!(plan.cuts.tunnel_void(direction, h) > 0.0);
        assert!(plan.cuts.tunnel_void(direction, h + 20.0) < 0.0);
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

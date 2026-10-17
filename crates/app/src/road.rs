//! Bounded, opt-in viaduct trial. A single horizontal deck, not terrain paint.
//! All positions are metres in one planet-fixed tangent frame (x across,
//! y along, z up), so the deck cannot acquire the terrain's bumps or LOD.

use glam::DVec3;

pub const LENGTH: f64 = 650.0;
pub const HALF_WIDTH: f64 = 5.0;
const CLEARANCE: f64 = 12.0;
const DECK_THICKNESS: f64 = 1.0;
const PIER_SPACING: f64 = 25.0;
const SEGMENT_LENGTH: f64 = 10.0;

pub fn enabled() -> bool {
    matches!(
        std::env::var("PLANET_ROAD_EXPERIMENT")
            .as_deref()
            .map(str::trim),
        Ok("1" | "true" | "on")
    )
}

pub fn is_replay(name: &str) -> bool {
    matches!(
        name,
        "road_surface_trial" | "road_viaduct_overview" | "road_viaduct_performance"
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

fn smoothstep(value: f64) -> f64 {
    let t = value.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn centre_x(along: f64) -> f64 {
    -30.0 * smoothstep(along / 220.0) + 50.0 * smoothstep((along - 260.0) / 360.0)
}

fn lateral(along: f64) -> DVec3 {
    let derivative = |t: f64, length: f64| {
        let t = t.clamp(0.0, 1.0);
        6.0 * t * (1.0 - t) / length
    };
    let dx = -30.0 * derivative(along / 220.0, 220.0)
        + 50.0 * derivative((along - 260.0) / 360.0, 360.0);
    DVec3::new(1.0, -dx, 0.0).normalize()
}

fn point(along: f64, offset: f64, height: f64) -> DVec3 {
    DVec3::new(centre_x(along), along, height) + lateral(along) * offset
}

pub fn world(local: DVec3) -> DVec3 {
    origin() + across() * local.x + forward() * local.y + up() * local.z
}

#[derive(Clone, Copy, Debug)]
pub struct Pier {
    pub local_foot: DVec3,
}

pub struct Viaduct {
    pub deck_height: f64,
    pub piers: Vec<Pier>,
}

impl Viaduct {
    /// Survey once from resident dense terrain, never from per-frame LOD.
    /// This is a sampled clearance contract for this pilot, not an arbitrary
    /// terrain/network solver. Missing terrain rejects the whole plan.
    pub fn survey(mut height_at: impl FnMut(DVec3) -> Option<f64>) -> Option<Self> {
        let mut ground = |p: DVec3| -> Option<f64> {
            let mut z = 0.0;
            // Refine the radial query to the vertical through this local x/y.
            for _ in 0..3 {
                let direction = world(DVec3::new(p.x, p.y, z)).normalize();
                let height = height_at(direction)?;
                if !height.is_finite() || height <= 0.0 {
                    return None;
                }
                z = (direction * (crate::planet::planet_radius_meters() + height) - origin())
                    .dot(up());
            }
            Some(z)
        };
        let mut highest = f64::NEG_INFINITY;
        for station in 0..=130 {
            for offset in [-HALF_WIDTH, 0.0, HALF_WIDTH] {
                highest = highest.max(ground(point(station as f64 * 5.0, offset, 0.0))?);
            }
        }
        let mut piers = Vec::new();
        for station in 0..=(LENGTH / PIER_SPACING) as usize {
            for offset in [-3.0, 3.0] {
                let mut p = point(station as f64 * PIER_SPACING, offset, 0.0);
                p.z = ground(p)? - 2.0; // Bury the footing instead of floating on fine relief.
                highest = highest.max(p.z + 2.0);
                piers.push(Pier { local_foot: p });
            }
        }
        Some(Self {
            deck_height: highest + CLEARANCE + DECK_THICKNESS,
            piers,
        })
    }

    /// Radial intersection with the level plane, independent of the ground.
    pub fn plane_height_at(&self, direction: DVec3) -> f64 {
        (crate::planet::planet_radius_meters() + self.deck_height) / direction.dot(up())
            - crate::planet::planet_radius_meters()
    }

    pub fn deck_height_at(&self, direction: DVec3) -> Option<f64> {
        if direction.dot(up()) <= 0.0 {
            return None;
        }
        let height = self.plane_height_at(direction);
        let p = direction * (crate::planet::planet_radius_meters() + height) - origin();
        let along = p.dot(forward());
        let local = DVec3::new(p.dot(across()), along, 0.0);
        if !(-1.0e-6..=LENGTH + 1.0e-6).contains(&along)
            || (local.x - centre_x(along)).abs() > HALF_WIDTH * 1.1
        {
            return None;
        }
        // Match the actual mesh strip, not just distance from the smooth
        // centreline. The tiny edge tolerance covers radial roundoff only.
        let segment = (along / SEGMENT_LENGTH).floor() as i32;
        for i in (segment - 1).max(0)..=(segment + 1).min((LENGTH / SEGMENT_LENGTH) as i32 - 1) {
            let a = f64::from(i) * SEGMENT_LENGTH;
            let b = a + SEGMENT_LENGTH;
            let corners = [
                point(a, -HALF_WIDTH, 0.0),
                point(a, HALF_WIDTH, 0.0),
                point(b, HALF_WIDTH, 0.0),
                point(b, -HALF_WIDTH, 0.0),
            ];
            if (0..4).all(|j| {
                (corners[(j + 1) % 4] - corners[j])
                    .cross(local - corners[j])
                    .z
                    >= -1.0e-6
            }) {
                return Some(height);
            }
        }
        None
    }

    /// A person below the bridge must not be teleported onto it.
    pub fn walkable_height(&self, direction: DVec3, eye_altitude: f64) -> Option<f64> {
        self.deck_height_at(direction)
            .filter(|height| eye_altitude >= *height - 0.25)
    }

    pub fn mesh(&self) -> Vec<Vertex> {
        let mut mesh = Vec::new();
        let concrete = [0.32, 0.31, 0.29];
        let asphalt = [0.045, 0.048, 0.05];
        let z = self.deck_height;
        let mut slab = |width: f64, bottom: f64, top: f64, offset: f64, colour: [f32; 3]| {
            for segment in 0..(LENGTH / SEGMENT_LENGTH) as usize {
                let a = segment as f64 * SEGMENT_LENGTH;
                let b = a + SEGMENT_LENGTH;
                let l0 = point(a, offset - width, bottom);
                let r0 = point(a, offset + width, bottom);
                let l1 = point(b, offset - width, bottom);
                let r1 = point(b, offset + width, bottom);
                let rise = DVec3::Z * (top - bottom);
                quad(
                    &mut mesh,
                    [l0 + rise, r0 + rise, r1 + rise, l1 + rise],
                    colour,
                );
                quad(&mut mesh, [r0, l0, l1, r1], concrete);
                quad(&mut mesh, [l0, l0 + rise, l1 + rise, l1], concrete);
                quad(&mut mesh, [r0 + rise, r0, r1, r1 + rise], concrete);
                if segment == 0 {
                    quad(&mut mesh, [r0, r0 + rise, l0 + rise, l0], concrete);
                }
                if b >= LENGTH {
                    quad(&mut mesh, [l1, l1 + rise, r1 + rise, r1], concrete);
                }
            }
        };
        slab(HALF_WIDTH, z - DECK_THICKNESS, z, 0.0, asphalt);
        // Solid edge parapets read clearly without dozens of tiny rail posts.
        slab(0.18, z, z + 0.9, -HALF_WIDTH + 0.18, concrete);
        slab(0.18, z, z + 0.9, HALF_WIDTH - 0.18, concrete);
        for along in (0..LENGTH as usize).step_by(10) {
            quad(
                &mut mesh,
                [
                    point(along as f64, -0.07, z + 0.015),
                    point(along as f64, 0.07, z + 0.015),
                    point(along as f64 + 4.0, 0.07, z + 0.015),
                    point(along as f64 + 4.0, -0.07, z + 0.015),
                ],
                [0.72, 0.68, 0.49],
            );
        }
        for pair in self.piers.chunks_exact(2) {
            for pier in pair {
                let height = z - DECK_THICKNESS - pier.local_foot.z;
                let half = (height * 0.012).clamp(0.7, 2.0);
                box_mesh(
                    &mut mesh,
                    pier.local_foot - DVec3::new(half, half, 0.0),
                    pier.local_foot + DVec3::new(half, half, height),
                    concrete,
                );
            }
            let a = pair[0].local_foot;
            let b = pair[1].local_foot;
            box_mesh(
                &mut mesh,
                DVec3::new(a.x.min(b.x) - 1.0, a.y.min(b.y) - 1.0, z - 2.3),
                DVec3::new(a.x.max(b.x) + 1.0, a.y.max(b.y) + 1.0, z - 1.0),
                concrete,
            );
        }
        mesh
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
    use glam::Vec3;

    #[test]
    fn viaduct_is_level_and_clears_an_uneven_survey() {
        let plan =
            Viaduct::survey(|d| Some(1900.0 + 25.0 * (d.dot(forward()) * 20000.0).sin())).unwrap();
        for along in 0..=650 {
            let p = world(point(along as f64, 0.0, plan.deck_height));
            assert!(((p - origin()).dot(up()) - plan.deck_height).abs() < 1.0e-8);
            let direction = p.normalize();
            assert!(
                (plan.deck_height_at(direction).unwrap()
                    - (p.length() - crate::planet::planet_radius_meters()))
                .abs()
                    < 1.0e-8
            );
        }
        assert!(
            plan.piers
                .iter()
                .map(|p| p.local_foot.z)
                .fold(f64::NEG_INFINITY, f64::max)
                <= plan.deck_height - CLEARANCE
        );
        assert!(
            plan.piers
                .windows(2)
                .any(|p| (p[0].local_foot.z - p[1].local_foot.z).abs() > 0.01)
        );
    }

    #[test]
    fn viaduct_missing_ground_rejects_plan_and_underpass_stays_open() {
        assert!(Viaduct::survey(|_| None).is_none());
        assert!(Viaduct::survey(|_| Some(f64::NAN)).is_none());
        let plan = Viaduct::survey(|_| Some(1900.0)).unwrap();
        let direction = world(point(100.0, 0.0, plan.deck_height)).normalize();
        assert!(plan.walkable_height(direction, 1900.0).is_none());
        assert!(
            plan.walkable_height(direction, plan.deck_height + 2.0)
                .is_some()
        );
        let outside = world(point(100.0, 8.0, plan.deck_height)).normalize();
        assert!(plan.deck_height_at(outside).is_none());
    }

    #[test]
    fn viaduct_mesh_is_bounded_finite_and_road_top_is_flat() {
        let plan = Viaduct::survey(|_| Some(1900.0)).unwrap();
        let mesh = plan.mesh();
        assert!(mesh.len() / 3 < 4000);
        assert_eq!(plan.piers.len(), 54);
        for v in &mesh {
            assert!(Vec3::from_array(v.position).is_finite());
            assert!((Vec3::from_array(v.normal).length() - 1.0).abs() < 1.0e-5);
            if v.colour == [0.045, 0.048, 0.05] {
                assert!((f64::from(v.position[2]) - plan.deck_height).abs() < 0.001);
                assert_eq!(v.normal, [0.0, 0.0, 1.0]);
                let direction = world(DVec3::from_array(v.position.map(f64::from))).normalize();
                assert!(
                    (plan.plane_height_at(direction)
                        - (world(DVec3::from_array(v.position.map(f64::from))).length()
                            - crate::planet::planet_radius_meters()))
                    .abs()
                        < 0.001
                );
            }
        }
        for triangle in mesh
            .chunks_exact(3)
            .filter(|t| t[0].colour == [0.045, 0.048, 0.05])
        {
            let local = triangle
                .iter()
                .map(|v| DVec3::from_array(v.position.map(f64::from)))
                .sum::<DVec3>()
                / 3.0;
            let p = world(local);
            assert!(
                (plan.deck_height_at(p.normalize()).unwrap()
                    - (p.length() - crate::planet::planet_radius_meters()))
                .abs()
                    < 0.001
            );
        }
    }
}

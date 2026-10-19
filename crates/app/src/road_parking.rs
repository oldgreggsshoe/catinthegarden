//! Cached village entrance courts, sharing the road cut/forest/walking index.
use crate::{
    road::{self, Vertex},
    road_network::Survey,
    road_network_field::{Field, Segment},
};
use glam::{DVec2, DVec3};

pub const RADIUS: f64 = 20.0;
// Put the court beyond the road end. Only its last 5m overlap the existing
// flat arrival apron; don't force a new 40m flat onto every fitted route.
pub const OFFSET: f64 = RADIUS - 5.0;
pub const CUT_RADIUS: f64 = 22.0;
pub const REACH: f64 = 48.0;
pub const FLOOR_DEPTH: f64 = 0.35;
// Open roads use -1..-7; bores use +1. Reserve a distinct point-shape tag.
pub const GPU_TAG: f32 = -32.0;
// Retain the road system's existing crown margin, not just trunk clearance.
pub const TREE_RADIUS: f64 = RADIUS + crate::road::TREE_CLEARANCE_METERS - road::HALF_WIDTH;
const SECTORS: usize = 64;

/// A point primitive in the existing bounded index, never a zero-length road.
/// GPU metadata tags the point explicitly; no new buffer or larger search.
pub fn segment(centre: DVec3) -> Segment {
    Segment {
        a: centre,
        b: centre,
        tunnel: false,
        reach: REACH,
        floor_depth: FLOOR_DEPTH,
        junction_a: false,
        junction_b: false,
    }
}

/// Survey the whole possible court footprint before accepting an entrance.
pub fn siting_samples(d: DVec3, radius: f64) -> impl Iterator<Item = DVec3> {
    samples(d, radius, OFFSET + CUT_RADIUS)
}
pub fn bank_samples(d: DVec3, radius: f64) -> impl Iterator<Item = DVec3> {
    samples(d, radius, OFFSET + REACH)
}
fn samples(d: DVec3, radius: f64, extent: f64) -> impl Iterator<Item = DVec3> {
    let reference = if d.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y };
    let u = d.cross(reference).normalize();
    let v = d.cross(u);
    [0.5, 1.0].into_iter().flat_map(move |r| {
        (0..16).map(move |i| {
            let angle = i as f64 * std::f64::consts::TAU / 16.0;
            (d * radius + (u * angle.cos() + v * angle.sin()) * (r * extent)).normalize()
        })
    })
}

pub fn mesh(
    s: Segment,
    field: &Field,
    terrain: &impl Survey,
    near: bool,
) -> Result<Vec<Vertex>, String> {
    let mut mesh = Vec::new();
    let p = |i: usize, r: f64| {
        let angle = i as f64 * std::f64::consts::TAU / SECTORS as f64;
        s.a + DVec3::new(angle.cos(), angle.sin(), 0.0) * r
    };
    let mut feet = Vec::new();
    for i in 0..=SECTORS {
        let d = field.frame.world(p(i, RADIUS)).normalize();
        let ground = terrain.ground(d)?.height;
        let z = field
            .frame
            .local(d * (field.frame.radius + field.height(d, ground)))
            .z;
        feet.push(p(i, RADIUS).with_z(z.min(s.a.z - 0.3) - 0.05));
    }
    for i in 0..SECTORS {
        for point in [s.a, p(i, RADIUS), p(i + 1, RADIUS)] {
            mesh.push(Vertex {
                position: point.as_vec3().to_array(),
                normal: [0.0, 0.0, 1.0],
                colour: [0.045, 0.048, 0.05],
            });
        }
        road::quad(
            &mut mesh,
            [p(i, RADIUS), feet[i], feet[i + 1], p(i + 1, RADIUS)],
            [0.32, 0.31, 0.29],
        );
    }
    if near {
        let radii = [RADIUS, CUT_RADIUS, 28.0, 36.0, 44.0, REACH];
        let mut rings = Vec::new();
        for radius in radii {
            let mut ring = Vec::new();
            for i in 0..=SECTORS {
                let d = field.frame.world(p(i, radius)).normalize();
                let ground = terrain.ground(d)?.height;
                let cut = field.height(d, ground);
                ring.push((
                    field.frame.local(d * (field.frame.radius + cut + 0.1)),
                    (ground - cut).max(0.0),
                ));
            }
            rings.push(ring);
        }
        for rings in rings.windows(2) {
            for i in 0..SECTORS {
                let q = [rings[0][i], rings[1][i], rings[1][i + 1], rings[0][i + 1]];
                if q.iter().all(|p| p.1 <= 0.25) {
                    continue;
                }
                let normal = (q[1].0 - q[0].0)
                    .cross(q[2].0 - q[0].0)
                    .normalize()
                    .as_vec3()
                    .to_array();
                for j in [0, 1, 2, 0, 2, 3] {
                    mesh.push(Vertex {
                        position: q[j].0.as_vec3().to_array(),
                        normal,
                        colour: [0.24, -(0.19 + q[j].1.min(64.0) as f32 / 64.0), 0.13],
                    });
                }
            }
        }
    }
    Ok(mesh)
}

/// Interval of a line inside the court, also clipped to its deck elevation.
pub fn mouth(a: DVec3, b: DVec3, centre: DVec3) -> Option<(f64, f64)> {
    let d = (b - a).truncate();
    let q = (a - centre).truncate();
    let aa = d.length_squared();
    let bb = q.dot(d);
    let discriminant = bb * bb - aa * (q.length_squared() - RADIUS * RADIUS);
    if aa < 1e-12 || discriminant < 0.0 {
        return None;
    }
    let mut lo = ((-bb - discriminant.sqrt()) / aa).max(0.0);
    let mut hi = ((-bb + discriminant.sqrt()) / aa).min(1.0);
    let dz = b.z - a.z;
    if dz.abs() < 1e-9 {
        if (a.z - centre.z).abs() > 0.25 {
            return None;
        }
    } else {
        let x = (centre.z - 0.25 - a.z) / dz;
        let y = (centre.z + 0.25 - a.z) / dz;
        lo = lo.max(x.min(y));
        hi = hi.min(x.max(y));
    }
    (hi > lo).then_some((lo, hi))
}

/// The pavement is a regular polygon, not a collision-only analytic circle.
pub fn contains(offset: DVec2) -> bool {
    let angle = offset
        .y
        .atan2(offset.x)
        .rem_euclid(std::f64::consts::TAU / SECTORS as f64);
    let half = std::f64::consts::PI / SECTORS as f64;
    offset.length() <= RADIUS * half.cos() / (angle - half).cos() + 1e-7
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_network::{Ground, Survey};
    use crate::road_network_field::Frame;
    use planet_coretypes::TileKey;
    struct Flat;
    impl Survey for Flat {
        fn ground(&self, _: DVec3) -> Result<Ground, String> {
            Ok(Ground {
                height: 1010.0,
                ocean: false,
            })
        }
        fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
            Ok(Vec::new())
        }
    }
    #[test]
    fn court_is_level_walkable_excavated_and_clear_in_every_direction() {
        let f = Frame::new(DVec3::Z, 4e6).unwrap();
        let s = segment(DVec3::Z * 1000.0);
        let mut field = Field::build(f, vec![s]).unwrap();
        field.set_deck_sections(vec![[DVec3::ZERO; 2]]).unwrap();
        let vertices = mesh(s, &field, &Flat, true).unwrap();
        assert!(
            vertices
                .iter()
                .all(|v| v.position.iter().chain(&v.normal).all(|n| n.is_finite()))
        );
        for i in 0..128 {
            let a = i as f64 * std::f64::consts::TAU / 128.0;
            for radius in [0.0, 5.0, 15.0, 19.9] {
                let p = s.a + DVec3::new(a.cos(), a.sin(), 0.0) * radius;
                let world = f.world(p);
                let d = world.normalize();
                let h = world.length() - f.radius;
                assert!((field.deck_height(d, h + 2.0).unwrap() - h).abs() < 1e-6);
                assert!(field.height(d, h + 10.0) < h - 0.3);
                assert!(field.clears_tree(d, h - 1.0));
            }
        }
        assert!(
            mouth(
                DVec3::new(0.0, 0.0, 1000.0),
                DVec3::new(40.0, 0.0, 1000.0),
                s.a
            )
            .unwrap()
            .1 == 0.5
        );
        assert!(
            mouth(
                DVec3::new(0.0, 0.0, 1010.0),
                DVec3::new(40.0, 0.0, 1010.0),
                s.a
            )
            .is_none()
        );
    }
    #[test]
    fn court_shape_and_tree_constants_match_wgsl() {
        let wgsl = include_str!("road_cuts.wgsl");
        for (name, value) in [
            ("RADIUS", RADIUS),
            ("CUT_RADIUS", CUT_RADIUS),
            ("TREE_RADIUS", TREE_RADIUS),
        ] {
            assert!(wgsl.contains(&format!("const NETWORK_PARKING_{name}: f32 = {value:.1};")));
        }
    }
}

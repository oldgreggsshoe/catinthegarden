//! Civic timber buildings share the established village footprints, materials
//! and path worker. Meshes are cached once; nothing is generated per draw.
use super::{push_quad, push_triangle, timber_box, HouseInstance, HouseVertex};
use glam::DVec3;
use std::ops::Range;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    House,
    Inn,
    Workshop,
    Market,
    Harbour,
    Lighthouse,
}
impl Kind {
    pub const ALL: [Self; 6] = [
        Self::House,
        Self::Inn,
        Self::Workshop,
        Self::Market,
        Self::Harbour,
        Self::Lighthouse,
    ];
    pub fn of(h: &HouseInstance) -> Self {
        Self::ALL
            .get(h._pad3 as usize)
            .copied()
            .unwrap_or(Self::House)
    }
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::House => None,
            Self::Inn => Some("WAYFARER INN"),
            Self::Workshop => Some("CARPENTER"),
            Self::Market => Some("MARKET"),
            Self::Harbour => Some("HARBOUR"),
            Self::Lighthouse => Some("LIGHTHOUSE"),
        }
    }
}
pub fn enabled() -> bool {
    !matches!(
        std::env::var("PLANET_CIVIC_BUILDINGS").as_deref(),
        Ok("0" | "off")
    )
}
/// Rank the immutable layout, not whichever houses happen to be in view.
/// Changing camera/residency therefore cannot turn a home into a different shop.
pub fn village_roles(layout: &[(f64, f64, f64)]) -> Vec<Kind> {
    let mut roles = vec![Kind::House; layout.len()];
    if !enabled() {
        return roles;
    }
    let mut rank: Vec<_> = (0..layout.len()).collect();
    rank.sort_by(|&a, &b| {
        (layout[a].0.powi(2) + layout[a].1.powi(2))
            .total_cmp(&(layout[b].0.powi(2) + layout[b].1.powi(2)))
            .then(a.cmp(&b))
    });
    for (i, k) in rank
        .into_iter()
        .zip([Kind::Market, Kind::Inn, Kind::Workshop])
    {
        roles[i] = k;
    }
    roles
}
fn beam(v: &mut Vec<HouseVertex>, a: DVec3, b: DVec3, width: f64, tag: f32) {
    let z = (b - a).normalize();
    let r = if z.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y };
    let x = z.cross(r).normalize();
    let y = z.cross(x);
    let start = v.len();
    timber_box(
        v,
        DVec3::new(-width / 2., -width / 2., 0.),
        DVec3::new(width / 2., width / 2., a.distance(b)),
        tag,
    );
    for p in &mut v[start..] {
        let q = DVec3::from(p.position.map(f64::from));
        let n = DVec3::from(p.normal.map(f64::from));
        p.position = (a + x * q.x + y * q.y + z * q.z).as_vec3().to_array();
        p.normal = (x * n.x + y * n.y + z * n.z).as_vec3().to_array();
    }
}
fn cylinder(v: &mut Vec<HouseVertex>, a: DVec3, b: DVec3, r: f64, tag: f32) {
    let z = (b - a).normalize();
    let x = z
        .cross(if z.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y })
        .normalize();
    let y = z.cross(x);
    let ring = |c: DVec3, i: usize| {
        let t = i as f64 * std::f64::consts::TAU / 8.;
        c + (x * t.cos() + y * t.sin()) * r
    };
    for i in 0..8 {
        push_quad(
            v,
            ring(a, i),
            ring(a, i + 1),
            ring(b, i + 1),
            ring(b, i),
            tag,
        );
        push_triangle(v, a, ring(a, i + 1), ring(a, i), tag);
        push_triangle(v, b, ring(b, i), ring(b, i + 1), tag);
    }
}
fn canopy(v: &mut Vec<HouseVertex>, z: f64, tag: f32) {
    for side in [-1., 1.] {
        let q = [
            DVec3::new(side * 3.6, -4.5, z),
            DVec3::new(0., -4.5, z + 1.6),
            DVec3::new(0., 4.5, z + 1.6),
            DVec3::new(side * 3.6, 4.5, z),
        ];
        if side < 0. {
            push_quad(v, q[0], q[1], q[2], q[3], tag);
        } else {
            push_quad(v, q[3], q[2], q[1], q[0], tag);
        }
    }
}
pub fn mesh(kind: Kind, detail: bool) -> Vec<HouseVertex> {
    let mut v = Vec::new();
    match kind {
        Kind::House | Kind::Inn | Kind::Harbour => {
            v = super::build_house_mesh();
            if !detail {
                v.truncate(42);
            }
            if kind == Kind::Inn {
                for p in &mut v {
                    if p.position[2] >= 2.9 {
                        p.position[2] += 2.8;
                    }
                }
                if detail {
                    timber_box(
                        &mut v,
                        DVec3::new(-3.25, -4.18, 2.9),
                        DVec3::new(3.25, 4.18, 3.15),
                        5.,
                    );
                    for x in [-1.8, 1.8] {
                        timber_box(
                            &mut v,
                            DVec3::new(x - 0.55, -4.14, 3.8),
                            DVec3::new(x + 0.55, -4.105, 5.),
                            4.,
                        );
                        for dx in [-0.62, 0.62] {
                            timber_box(
                                &mut v,
                                DVec3::new(x + dx - 0.06, -4.23, 3.75),
                                DVec3::new(x + dx + 0.06, -4.10, 5.08),
                                2.,
                            );
                        }
                    }
                    timber_box(
                        &mut v,
                        DVec3::new(1.8, 1.4, 6.),
                        DVec3::new(2.5, 2.1, 9.5),
                        6.,
                    );
                    for x in [-2.3, 2.3] {
                        timber_box(
                            &mut v,
                            DVec3::new(x - 0.45, -5.1, 0.05),
                            DVec3::new(x + 0.45, -4.6, 0.55),
                            5.,
                        );
                    }
                }
            }
            if kind == Kind::Harbour {
                // Warehouse front/door is inland (-Y); the pier leaves its rear.
                timber_box(
                    &mut v,
                    DVec3::new(-3., 2., -0.15),
                    DVec3::new(3., 62., 0.15),
                    9.,
                );
                timber_box(
                    &mut v,
                    DVec3::new(-9., 58., -0.15),
                    DVec3::new(9., 66., 0.15),
                    9.,
                );
                for y in [8., 20., 32., 44., 60.] {
                    for x in [-2.5, 2.5] {
                        timber_box(
                            &mut v,
                            DVec3::new(x - 0.2, y - 0.2, -32.),
                            DVec3::new(x + 0.2, y + 0.2, 1.2),
                            5.,
                        );
                    }
                }
                if detail {
                    for side in [-1., 1.] {
                        for y in [10., 22., 34., 46., 58.] {
                            beam(
                                &mut v,
                                DVec3::new(side * 2.8, y, 0.15),
                                DVec3::new(side * 2.8, y, 1.2),
                                0.14,
                                5.,
                            );
                        }
                        beam(
                            &mut v,
                            DVec3::new(side * 2.8, 8., 1.2),
                            DVec3::new(side * 2.8, 58., 1.2),
                            0.12,
                            5.,
                        );
                    }
                    for y in [12., 17., 22.] {
                        timber_box(
                            &mut v,
                            DVec3::new(-2.6, y, 0.15),
                            DVec3::new(-1.2, y + 1.2, 1.4),
                            5.,
                        );
                    }
                    beam(
                        &mut v,
                        DVec3::new(2., 55., 0.15),
                        DVec3::new(2., 55., 6.),
                        0.32,
                        5.,
                    );
                    beam(
                        &mut v,
                        DVec3::new(2., 55., 5.),
                        DVec3::new(6., 55., 5.),
                        0.25,
                        5.,
                    );
                    beam(
                        &mut v,
                        DVec3::new(2., 55., 3.5),
                        DVec3::new(5., 55., 5.),
                        0.18,
                        5.,
                    );
                }
            }
        }
        Kind::Workshop => {
            v = super::build_house_mesh()[..42].to_vec();
            v.drain(18..24); // Open working bay, not a painted door.
            for x in [-3., 3.] {
                timber_box(
                    &mut v,
                    DVec3::new(x - 0.13, -4.1, -0.35),
                    DVec3::new(x + 0.13, -3.84, 3.1),
                    5.,
                );
            }
            timber_box(
                &mut v,
                DVec3::new(-3.2, -4.2, 2.9),
                DVec3::new(3.2, -3.9, 3.15),
                5.,
            );
            if detail {
                timber_box(
                    &mut v,
                    DVec3::new(-2.5, 0.4, 0.9),
                    DVec3::new(2.5, 1.5, 1.08),
                    5.,
                );
                for x in [-2.1, 2.1] {
                    for y in [0.55, 1.35] {
                        timber_box(
                            &mut v,
                            DVec3::new(x - 0.08, y - 0.08, 0.),
                            DVec3::new(x + 0.08, y + 0.08, 0.9),
                            5.,
                        );
                    }
                }
                for i in 0..4 {
                    let z = 0.25 + i as f64 * 0.2;
                    timber_box(
                        &mut v,
                        DVec3::new(-2.9, 2., z),
                        DVec3::new(2.9, 2.4, z + 0.12),
                        5.,
                    );
                }
                for i in 0..3 {
                    cylinder(
                        &mut v,
                        DVec3::new(-2.6 + i as f64 * 0.55, -2.6, 0.28),
                        DVec3::new(-2.6 + i as f64 * 0.55, -0.2, 0.28),
                        0.24,
                        5.,
                    );
                }
                for x in [-1.3, 1.3] {
                    for side in [-1., 1.] {
                        beam(
                            &mut v,
                            DVec3::new(x + side * 0.45, -1.9, 0.),
                            DVec3::new(x, -1.9, 0.8),
                            0.1,
                            5.,
                        );
                    }
                    beam(
                        &mut v,
                        DVec3::new(x, -2.3, 0.8),
                        DVec3::new(x, -1.5, 0.8),
                        0.12,
                        5.,
                    );
                }
            }
        }
        Kind::Market => {
            canopy(&mut v, 3.1, 8.);
            for x in [-3., 3.] {
                for y in [-3.8, 0., 3.8] {
                    timber_box(
                        &mut v,
                        DVec3::new(x - 0.12, y - 0.12, -0.35),
                        DVec3::new(x + 0.12, y + 0.12, 3.2),
                        5.,
                    );
                }
            }
            beam(
                &mut v,
                DVec3::new(-3.2, -3.8, 3.1),
                DVec3::new(3.2, -3.8, 3.1),
                0.18,
                5.,
            );
            if detail {
                for x in [-2.1, 2.1] {
                    for y in [-1.8, 1.8] {
                        timber_box(
                            &mut v,
                            DVec3::new(x - 0.75, y - 0.8, 0.3),
                            DVec3::new(x + 0.75, y + 0.8, 1.05),
                            5.,
                        );
                        for k in 0..3 {
                            timber_box(
                                &mut v,
                                DVec3::new(x - 0.65 + k as f64 * 0.45, y - 0.65, 1.05),
                                DVec3::new(x - 0.3 + k as f64 * 0.45, y + 0.65, 1.38),
                                2.,
                            );
                        }
                    }
                }
            }
        }
        Kind::Lighthouse => {
            let point = |r: f64, z: f64, i: usize| {
                let a = i as f64 * std::f64::consts::TAU / 8.;
                DVec3::new(r * a.cos(), r * a.sin(), z)
            };
            timber_box(
                &mut v,
                DVec3::new(-3., -3., -4.),
                DVec3::new(3., 3., 0.25),
                6.,
            );
            for i in 0..8 {
                push_quad(
                    &mut v,
                    point(2.7, 0.2, i),
                    point(2.7, 0.2, i + 1),
                    point(1.8, 14., i + 1),
                    point(1.8, 14., i),
                    0.,
                );
                push_quad(
                    &mut v,
                    point(1.5, 14.5, i),
                    point(1.5, 14.5, i + 1),
                    point(1.5, 17., i + 1),
                    point(1.5, 17., i),
                    7.,
                );
                push_triangle(
                    &mut v,
                    point(2.3, 17.1, i),
                    point(2.3, 17.1, i + 1),
                    DVec3::new(0., 0., 19.),
                    1.,
                );
            }
            timber_box(
                &mut v,
                DVec3::new(-3.2, -3.2, 14.),
                DVec3::new(3.2, 3.2, 14.25),
                9.,
            );
            if detail {
                for i in 0..8 {
                    beam(&mut v, point(2.8, 0., i), point(1.85, 14., i), 0.22, 5.);
                    beam(&mut v, point(2.75, 0.5, i), point(2.2, 8., i + 1), 0.12, 5.);
                    beam(&mut v, point(3., 14.2, i), point(3., 15.3, i), 0.13, 5.);
                    beam(&mut v, point(3., 15.3, i), point(3., 15.3, i + 1), 0.11, 5.);
                }
                timber_box(
                    &mut v,
                    DVec3::new(-0.6, -2.77, 0.15),
                    DVec3::new(0.6, -2.65, 2.3),
                    3.,
                );
            }
        }
    }
    v
}
pub struct Meshes {
    pub vertices: Vec<HouseVertex>,
    pub ranges: Vec<[Range<u32>; 2]>,
}
pub fn meshes() -> Meshes {
    let mut vertices = Vec::new();
    let mut ranges = Vec::new();
    for kind in Kind::ALL {
        let mut r = [0..0, 0..0];
        for (i, detail) in [false, true].into_iter().enumerate() {
            let start = vertices.len() as u32;
            vertices.extend(mesh(kind, detail));
            r[i] = start..vertices.len() as u32;
        }
        ranges.push(r);
    }
    Meshes { vertices, ranges }
}
/// Worker-built labels reuse the existing cached wood/sign atlas and draw.
pub fn board(instance: &HouseInstance, world: DVec3) -> Option<crate::road_navigation::Board> {
    let kind = Kind::of(instance);
    let label = kind.label()?;
    let up = DVec3::from(instance.up.map(f64::from));
    let forward = DVec3::from(instance.forward.map(f64::from));
    Some(crate::road_navigation::Board {
        kind: crate::road_navigation::Kind::Village,
        centre: world - forward * if kind == Kind::Lighthouse { 2.85 } else { 4.34 }
            + up * if kind == Kind::Inn { 3.5 } else { 2.7 },
        right: forward.cross(up),
        up,
        lines: vec![label.into()],
        width: if kind == Kind::Lighthouse { 2.3 } else { 3.4 },
        village: true,
        support: None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn civic_roles_are_stable_and_use_three_distinct_central_footprints() {
        let p = [
            (80., 0., 0.),
            (0., 0., 0.),
            (15., 0., 0.),
            (30., 0., 0.),
            (50., 0., 0.),
        ];
        let r = village_roles(&p);
        assert_eq!(
            r,
            [
                Kind::House,
                Kind::Market,
                Kind::Inn,
                Kind::Workshop,
                Kind::House
            ]
        );
        assert_eq!(r, village_roles(&p));
    }
    #[test]
    fn cached_buildings_have_bounded_finite_geometry_and_cheaper_far_meshes() {
        for kind in Kind::ALL {
            let near = mesh(kind, true);
            let far = mesh(kind, false);
            assert!(near.len() >= far.len());
            assert!(near.len() < 20_000);
            assert_eq!(near.len() % 3, 0);
            for p in near {
                assert!(glam::Vec3::from(p.position).is_finite());
                assert!(glam::Vec3::from(p.normal).is_normalized());
                assert!((0.0..=9.0).contains(&p.roof));
            }
        }
        assert!(
            mesh(Kind::Inn, false)[30..42]
                .iter()
                .all(|v| v.position[2] > 5.7),
            "the entire inn roof, including its eaves, moves above the upper storey"
        );
        assert!(mesh(Kind::Inn, false).iter().any(|v| v.position[2] > 8.));
        assert!(mesh(Kind::Lighthouse, false).iter().any(|v| v.roof == 7.));
        assert!(mesh(Kind::Market, false).iter().any(|v| v.roof == 8.));
    }
}

// Sparse coastal outposts: one candidate per deterministic L10 cell, only a
// quarter of cells considered. Cache the dense-data survey across 400m rebuilds.
use super::VillageSurface;
use crate::{planet::planet_radius_meters, terrain::TerrainRenderer};
use planet_coretypes::{face_uv_to_direction, tile_key_for_direction, BiomeId, TileKey};
use std::collections::{BTreeSet, HashMap};
const COAST_LEVEL: u8 = 10;
#[derive(Clone, Copy, Debug)]
pub struct CoastSite {
    pub terminal: DVec3,
    pub seaward: DVec3,
    pub tower: Option<DVec3>,
}
pub type CoastCache = HashMap<TileKey, Option<CoastSite>>;
fn offset(d: DVec3, v: DVec3) -> DVec3 {
    (d * planet_radius_meters() + v).normalize()
}
pub fn coast_keys(d: DVec3, ring: i32) -> Vec<TileKey> {
    let k = tile_key_for_direction(d, COAST_LEVEL);
    let step = 2. / (1u32 << COAST_LEVEL) as f64;
    let mut keys = BTreeSet::new();
    for y in -ring..=ring {
        for x in -ring..=ring {
            let u = -1. + (k.x as f64 + x as f64 + 0.5) * step;
            let v = -1. + (k.y as f64 + y as f64 + 0.5) * step;
            keys.insert(tile_key_for_direction(
                face_uv_to_direction(k.face, u, v),
                COAST_LEVEL,
            ));
        }
    }
    keys.into_iter().collect()
}
fn dry(terrain: &impl VillageSurface, d: DVec3) -> Option<f64> {
    let s = terrain.village_sample(d)?;
    if terrain.village_open_ocean(d)?
        || !s.height_meters.is_finite()
        || s.height_meters < 1.0
        || matches!(
            s.biome,
            BiomeId::Ocean
                | BiomeId::Lake
                | BiomeId::Ice
                | BiomeId::MountainSnow
                | BiomeId::GlacialMoraine
                | BiomeId::CrevasseField
        )
        || terrain
            .village_lake_level(d)
            .is_some_and(|level| level >= s.height_meters - 1.)
    {
        return None;
    }
    Some(s.height_meters)
}
fn land_footprint(terrain: &impl VillageSurface, d: DVec3, forward: DVec3) -> Option<f64> {
    let up = d;
    let f = (forward - up * forward.dot(up)).normalize();
    let right = f.cross(up);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for x in [-4., 0., 4.] {
        for y in [-5., 0., 5.] {
            let h = dry(terrain, offset(d, right * x + f * y))?;
            lo = lo.min(h);
            hi = hi.max(h);
        }
    }
    (hi - lo <= 3.).then_some((hi + lo) * 0.5)
}
fn uv_bounds(key: TileKey) -> [f64; 4] {
    let step = 2.0 / (1_u32 << key.level) as f64;
    let u = -1.0 + key.x as f64 * step;
    let v = -1.0 + key.y as f64 * step;
    [u, v, u + step, v + step]
}
pub fn coast_site(terrain: &impl VillageSurface, key: TileKey) -> Option<CoastSite> {
    if key.level != COAST_LEVEL || super::cell_seed(key) % 4 != 0 {
        return None;
    }
    let [u0, v0, u1, v1] = uv_bounds(key);
    let point = |x: usize, y: usize| {
        face_uv_to_direction(
            key.face,
            u0 + (u1 - u0) * x as f64 / 4.,
            v0 + (v1 - v0) * y as f64 / 4.,
        )
    };
    for y in 0..5 {
        for x in 0..5 {
            let a = point(x, y);
            let ocean = terrain.village_open_ocean(a)?;
            for (xx, yy) in [(x + 1, y), (x, y + 1)] {
                if xx > 4 || yy > 4 {
                    continue;
                }
                let b = point(xx, yy);
                if terrain.village_open_ocean(b)? == ocean {
                    continue;
                }
                let (mut water, mut land) = if ocean { (a, b) } else { (b, a) };
                for _ in 0..14 {
                    let mid = (water + land).normalize();
                    if terrain.village_open_ocean(mid)? {
                        water = mid;
                    } else {
                        land = mid;
                    }
                }
                let shore = (water + land).normalize();
                let inland = (land - water - shore * (land - water).dot(shore)).normalize();
                // The zero-height ocean boundary, not the edge of a lake or a
                // categorical biome halfway up a cliff.
                if terrain.village_sample(shore)?.height_meters.abs() > 2. {
                    continue;
                }
                for back in [16., 24., 32., 40.] {
                    let terminal = offset(shore, inland * back);
                    let Some(height) = land_footprint(terrain, terminal, inland) else {
                        continue;
                    };
                    if !(4.0..=12.0).contains(&height) {
                        continue;
                    }
                    let seaward = -inland;
                    let mut water_ok = true;
                    // Check the width of the T-landing as well as its centre.
                    let across = seaward.cross(terminal);
                    for side in [-9., 0., 9.] {
                        let p = offset(terminal, seaward * 64. + across * side);
                        let Some(s) = terrain.village_sample(p) else {
                            water_ok = false;
                            break;
                        };
                        if terrain.village_open_ocean(p) != Some(true)
                            || s.height_meters > -1.0
                            || s.height_meters < -24.0
                            || terrain.village_lake_level(p).is_some()
                        {
                            water_ok = false;
                            break;
                        }
                    }
                    if !water_ok {
                        continue;
                    }
                    let mut tower = None;
                    let mut best = 0.;
                    for distance in [100., 180., 260.] {
                        for side in [-120., 0., 120.] {
                            let d = offset(terminal, inland * distance + across * side);
                            if let Some(h) = land_footprint(terrain, d, inland) {
                                if (12.0..=160.0).contains(&h) && h > best {
                                    best = h;
                                    tower = Some(d);
                                }
                            }
                        }
                    }
                    return Some(CoastSite {
                        terminal,
                        seaward,
                        tower,
                    });
                }
            }
        }
    }
    None
}
fn drawn_footprint(
    terrain: &TerrainRenderer,
    d: DVec3,
    forward: DVec3,
    camera: DVec3,
) -> Option<f64> {
    let f = (forward - d * forward.dot(d)).normalize();
    let right = f.cross(d);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for x in [-3., 0., 3.] {
        for y in [-4., 0., 4.] {
            let p = offset(d, right * x + f * y);
            let h = terrain.village_raster_height_at(p, camera)?;
            if h < 1.
                || terrain.dense_level_open_ocean_at(p) != Some(false)
                || terrain
                    .lake_level_meters(p)
                    .is_some_and(|level| level >= h - 1.)
            {
                return None;
            }
            lo = lo.min(h);
            hi = hi.max(h);
        }
    }
    (hi - lo <= 4.).then_some((hi + lo) * 0.5)
}
pub fn coastal_instances(
    terrain: &TerrainRenderer,
    camera: DVec3,
    cache: &mut CoastCache,
) -> Vec<(HouseInstance, DVec3)> {
    if !enabled() {
        cache.clear();
        return Vec::new();
    }
    let keys = coast_keys(camera.normalize(), 2);
    cache.retain(|key, _| keys.contains(key));
    let mut result = Vec::new();
    for key in keys {
        // Missing siting tiles are not cached as a permanent rejection.
        if terrain
            .dense_level_surface_sample_at(face_uv_to_direction(
                key.face,
                -1. + (key.x as f64 + 0.5) * 2. / (1u32 << COAST_LEVEL) as f64,
                -1. + (key.y as f64 + 0.5) * 2. / (1u32 << COAST_LEVEL) as f64,
            ))
            .is_none()
        {
            continue;
        }
        let site = *cache.entry(key).or_insert_with(|| coast_site(terrain, key));
        let Some(site) = site else {
            continue;
        };
        if site.terminal.distance(camera.normalize()) * planet_radius_meters() > 8000. {
            continue;
        }
        let Some(base) = drawn_footprint(terrain, site.terminal, site.seaward, camera) else {
            continue;
        };
        if !(3.0..=16.0).contains(&base) {
            continue;
        }
        let make = |d: DVec3, h: f64, kind: Kind| {
            let world = d * (planet_radius_meters() + h);
            let bearing = if matches!(kind, Kind::Inn | Kind::Workshop | Kind::Market) {
                -site.seaward
            } else {
                site.seaward
            };
            let f = (bearing - d * bearing.dot(d)).normalize();
            (
                HouseInstance {
                    camera_relative_position: (world - camera).as_vec3().to_array(),
                    _pad0: 0.,
                    up: d.as_vec3().to_array(),
                    _pad1: 0.,
                    forward: f.as_vec3().to_array(),
                    _pad2: 0.,
                    colour: if kind == Kind::Harbour {
                        [0.42, 0.28, 0.13]
                    } else {
                        [0.65, 0.58, 0.44]
                    },
                    _pad3: kind as u8 as f32,
                },
                site.terminal,
            )
        };
        result.push(make(site.terminal, base, Kind::Harbour));
        // A compact waterfront trading post, not just a pier in empty country.
        // Every extra plot independently passes the actual dry raster footprint.
        let across = site.seaward.cross(site.terminal);
        for (kind, distance, side) in [
            (Kind::Inn, 45.0, 25.0),
            (Kind::Workshop, 45.0, -25.0),
            (Kind::Market, 75.0, 0.0),
        ] {
            let d = offset(site.terminal, -site.seaward * distance + across * side);
            if let Some(h) = drawn_footprint(terrain, d, site.seaward, camera) {
                result.push(make(d, h, kind));
            }
        }

        if let Some(tower) = site.tower {
            if let Some(h) = drawn_footprint(terrain, tower, site.seaward, camera) {
                result.push(make(tower, h, Kind::Lighthouse));
            }
        }
        if result.len() >= 12 {
            break;
        }
    }
    result
}
/// Exact tangent-plane top of the drawn pier. No clamp up from underneath it.
pub fn pier_height(h: &HouseInstance, world: DVec3, d: DVec3, eye: f64) -> Option<f64> {
    if Kind::of(h) != Kind::Harbour {
        return None;
    }
    let up = DVec3::from(h.up.map(f64::from)).normalize();
    let f = DVec3::from(h.forward.map(f64::from)).normalize();
    let across = f.cross(up);
    if d.distance_squared(up) > (90. / planet_radius_meters()).powi(2) {
        return None;
    }
    let point = d * ((world.dot(up) + 0.15) / d.dot(up));
    let offset = point - world;
    let x = offset.dot(across);
    let y = offset.dot(f);
    let on = (x.abs() <= 3. && (-4.1..=62.).contains(&y))
        || (x.abs() <= 9. && (58.0..=66.0).contains(&y));
    let height = point.length() - planet_radius_meters();
    (on && height <= eye + 1.5).then_some(height)
}

#[cfg(test)]
mod coast_tests {
    use super::*;
    struct Coast {
        origin: DVec3,
        inland: DVec3,
        lake: bool,
        cliff: bool,
    }
    impl VillageSurface for Coast {
        fn village_sample(&self, d: DVec3) -> Option<crate::terrain::ForestSurfaceSample> {
            let h = (d - self.origin).dot(self.inland) * planet_radius_meters() * 0.25;
            Some(crate::terrain::ForestSurfaceSample {
                height_meters: if self.cliff && h > 0. { h + 200. } else { h },
                macro_height_meters: h,
                biome: if h < 0. {
                    BiomeId::Ocean
                } else {
                    BiomeId::TemperateGrassland
                },
                moisture: 0.6,
                slope_radians: 0.1,
                source_key: tile_key_for_direction(d, 4),
                source_level: 4,
            })
        }
        fn village_open_ocean(&self, d: DVec3) -> Option<bool> {
            Some((d - self.origin).dot(self.inland) < 0.)
        }
        fn village_lake_level(&self, _: DVec3) -> Option<f64> {
            self.lake.then_some(10.)
        }
    }
    #[test]
    fn harbours_require_a_dry_approach_real_sea_and_shallow_landing() {
        let k = coast_keys(DVec3::X, 5)
            .into_iter()
            .find(|k| super::super::cell_seed(*k) % 4 == 0)
            .unwrap();
        let [a, b, c, d] = uv_bounds(k);
        let centre = face_uv_to_direction(k.face, (a + c) * 0.5, (b + d) * 0.5);
        let f = DVec3::Z - centre * centre.z;
        let coast = Coast {
            origin: centre,
            inland: f.normalize(),
            lake: false,
            cliff: false,
        };
        assert!(coast_site(&coast, k).is_some());
        assert!(coast_site(
            &Coast {
                lake: true,
                ..coast
            },
            k
        )
        .is_none());
        assert!(coast_site(
            &Coast {
                cliff: true,
                ..coast
            },
            k
        )
        .is_none());
    }
    #[test]
    fn pier_floor_agrees_with_the_mesh_includes_the_t_and_respects_an_eye_below_it() {
        let r = planet_radius_meters();
        let p = DVec3::X * (r + 6.);
        let h = HouseInstance {
            camera_relative_position: [0.; 3],
            _pad0: 0.,
            up: [1., 0., 0.],
            _pad1: 0.,
            forward: [0., 0., 1.],
            _pad2: 0.,
            colour: [1.; 3],
            _pad3: Kind::Harbour as u8 as f32,
        };
        let d = (p + DVec3::Z * 40.).normalize();
        assert!((pier_height(&h, p, d, 8.).unwrap() - 6.15).abs() < 0.002);
        assert!(pier_height(&h, p, d, 0.).is_none());
        assert!(pier_height(&h, p, (p + DVec3::Z * 63. + DVec3::Y * 8.).normalize(), 8.).is_some());
        assert!(pier_height(&h, p, (p + DVec3::Z * 40. + DVec3::Y * 8.).normalize(), 8.).is_none());
    }
    #[test]
    #[ignore = "asset-backed coast survey and capture coordinates"]
    fn real_coasts_have_sited_ports_and_headland_lighthouses() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/outmaps/test-planet");
        let surface = crate::road_network_survey::Surface::load(&root).unwrap();
        let d = DVec3::new(3372422., 1978479., 844259.).normalize();
        let mut found = 0;
        let mut keys = coast_keys(d, 24);
        keys.sort_by(|a, b| {
            let p = |k: &TileKey| {
                let [u, v, x, y] = uv_bounds(*k);
                face_uv_to_direction(k.face, (u + x) * 0.5, (v + y) * 0.5).distance_squared(d)
            };
            p(a).total_cmp(&p(b))
        });
        for key in keys {
            if let Some(p) = coast_site(&surface, key) {
                println!(
                    "PORT {:?} shore {:?} forward {:?} tower {:?}",
                    key, p.terminal, p.seaward, p.tower
                );
                found += 1;
                if found >= 3 {
                    break;
                }
            }
        }
        assert!(found > 0, "no suitable real harbour coast");
    }
}

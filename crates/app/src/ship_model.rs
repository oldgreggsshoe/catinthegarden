//! The ship's drawn model: a small 17th-century galleon -- tarred hull with a
//! gilded wale, gunports with cannon, a high stern castle with a windowed
//! transom, a forecastle and beakhead, three masts with tops, yards, belled
//! square sails and a lateen mizzen, shrouds with ratlines, stays, pennants and
//! an ensign.
//!
//! It is drawn, nothing more: the float, foam and spray still come from the
//! unchanged hull form in `ship.rs` (`half_beam_meters`, `keel_depth_meters`,
//! `sheer_height_meters`), and every hull-body vertex here stays inside that
//! envelope so they agree. Fittings (masts, sails, bowsprit, rudder, guns) may
//! stand outside it.
//!
//! Dimensions in the design below are written in metres at the 42m design
//! length and turned into ship-local metres by `SHIP_SCALE` (`p`). Axes: +x
//! toward the bow, +y to one beam, +z up, origin on the design waterline.

use glam::DVec3;

use crate::ship::{
    HULL_LENGTH_METERS, SHIP_SCALE, ShipVertex, half_beam_meters, keel_depth_meters, push_box,
    push_quad, push_triangle, sheer_height_meters,
};

const S: f64 = SHIP_SCALE;

/// Mesh stations along the hull: section edges, -1 at the transom to +1 at the
/// stem. Twenty segments of about 2m (at the 42m design length).
const SEGMENTS: usize = 20;

const TAR: [f32; 3] = [0.05, 0.04, 0.035];
const TOPSIDE: [f32; 3] = [0.27, 0.14, 0.07];
const TOPSIDE_DARK: [f32; 3] = [0.20, 0.10, 0.05];
const WALE: [f32; 3] = [0.62, 0.44, 0.12];
const RAIL_OUTER: [f32; 3] = [0.34, 0.18, 0.08];
const RAIL_CAP: [f32; 3] = [0.55, 0.40, 0.22];
const BULWARK_INNER: [f32; 3] = [0.42, 0.25, 0.12];
const DECK_A: [f32; 3] = [0.50, 0.38, 0.22];
const DECK_B: [f32; 3] = [0.43, 0.32, 0.18];
const WOOD: [f32; 3] = [0.30, 0.20, 0.11];
const SAIL_A: [f32; 3] = [0.82, 0.76, 0.62];
const SAIL_B: [f32; 3] = [0.76, 0.70, 0.56];
const ROPE: [f32; 3] = [0.12, 0.09, 0.06];
const GOLD: [f32; 3] = [0.75, 0.58, 0.15];
const RED: [f32; 3] = [0.65, 0.08, 0.06];
const WHITE: [f32; 3] = [0.85, 0.85, 0.82];
const DARK_OPENING: [f32; 3] = [0.04, 0.03, 0.03];
const GUN: [f32; 3] = [0.08, 0.08, 0.09];

/// Height of the bulwark/rail above whatever deck it stands on, at design size.
const RAIL_HEIGHT: f64 = 0.9;
/// Hull-side plane the bulwarks and raised decks stand on, and the plane of
/// their inner face, as fractions of the half-beam.
const SIDE_OUTER: f64 = 0.84;
const SIDE_INNER: f64 = 0.78;

/// Where the bridge camera stands: on the poop, the highest deck, aft of the
/// mizzen and off the centreline, an eye height above the deck. From the
/// quarterdeck the eye is level with the main course and looks into canvas;
/// from here it looks over the courses and under the topsails. Design size, scaled by `SHIP_SCALE`.
pub const BRIDGE_EYE_LOCAL: DVec3 = DVec3::new(
    BRIDGE_EYE_X * S,
    BRIDGE_EYE_Y * S,
    (POOP_RAISE + BRIDGE_EYE_DECK_Z + BRIDGE_EYE_HEIGHT) * S,
);
pub const BRIDGE_EYE_X: f64 = -14.0;
const BRIDGE_EYE_Y: f64 = 2.0;
/// Design-size deck height at the eye's station: `3.0 * (1 + 0.45 t^2)` at
/// t = -0.667, written out because the const above cannot call the hull form.
const BRIDGE_EYE_DECK_Z: f64 = 3.6;
/// An eye height above the deck, the same 1m (at ship scale) the old bridge had.
const BRIDGE_EYE_HEIGHT: f64 = 4.0;

const QUARTERDECK_RAISE: f64 = 2.4;
const POOP_RAISE: f64 = 4.6;
const FORECASTLE_RAISE: f64 = 2.4;

/// Raised deck height (design metres) over hull segment `i`: the poop and
/// quarterdeck aft, the forecastle forward, the waist level between.
fn raise(segment: usize) -> f64 {
    match segment {
        0..=3 => POOP_RAISE,
        4..=6 => QUARTERDECK_RAISE,
        15..=17 => FORECASTLE_RAISE,
        _ => 0.0,
    }
}

/// Hull body (inside the float's envelope) and fittings (free of it).
pub struct ShipModel {
    pub hull: Vec<ShipVertex>,
    pub fittings: Vec<ShipVertex>,
}

pub fn build() -> ShipModel {
    let mut hull = Vec::new();
    build_hull(&mut hull);
    let mut fittings = Vec::new();
    build_fittings(&mut fittings);
    ShipModel { hull, fittings }
}

/// Design-size metres to ship-local.
fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z) * S
}

fn station_t(index: usize) -> f64 {
    (-1.0 + 2.0 * index as f64 / SEGMENTS as f64).clamp(-1.0, 1.0)
}

fn station_x(t: f64) -> f64 {
    0.5 * HULL_LENGTH_METERS * t
}

/// Deck (sheer line) height at ship-local x.
fn deck_z(x: f64) -> f64 {
    sheer_height_meters((x / (0.5 * HULL_LENGTH_METERS)).clamp(-1.0, 1.0))
}

const RING_COUNT: usize = 7;

/// One point on the hull section at station `t`: ring 0 is the keel, 2 the
/// waterline (greatest beam), 3-4 the gilded wale, 6 the sheer.
fn ring_point(t: f64, ring: usize, side: f64) -> DVec3 {
    let half_beam = half_beam_meters(t);
    let keel = keel_depth_meters(t);
    let sheer = sheer_height_meters(t);
    let (fraction, z) = match ring {
        0 => (0.0, -keel),
        1 => (0.9, -0.5 * keel),
        2 => (1.0, 0.0),
        3 => (1.0, 0.35 * sheer),
        4 => (0.98, 0.52 * sheer),
        5 => (0.92, 0.78 * sheer),
        _ => (SIDE_OUTER, sheer),
    };
    DVec3::new(station_x(t), side * half_beam * fraction, z)
}

const RING_COLOURS: [[f32; 3]; RING_COUNT - 1] = [TAR, TAR, TOPSIDE, WALE, TOPSIDE, TOPSIDE_DARK];

/// Quad whose front is whichever side `hint` points to.
fn quad_facing(
    vertices: &mut Vec<ShipVertex>,
    a: DVec3,
    b: DVec3,
    c: DVec3,
    d: DVec3,
    colour: [f32; 3],
    hint: DVec3,
) {
    let mut normal = (b - a).cross(c - a);
    if normal.length_squared() <= 0.0 {
        normal = (c - a).cross(d - a);
    }
    if normal.dot(hint) >= 0.0 {
        push_quad(vertices, a, b, c, d, colour);
    } else {
        push_quad(vertices, a, d, c, b, colour);
    }
}

fn triangle_facing(
    vertices: &mut Vec<ShipVertex>,
    a: DVec3,
    b: DVec3,
    c: DVec3,
    colour: [f32; 3],
    hint: DVec3,
) {
    if (b - a).cross(c - a).dot(hint) >= 0.0 {
        push_triangle(vertices, a, b, c, colour);
    } else {
        push_triangle(vertices, a, c, b, colour);
    }
}

/// Seen from both sides: sails, flags.
fn quad_both_sides(
    vertices: &mut Vec<ShipVertex>,
    a: DVec3,
    b: DVec3,
    c: DVec3,
    d: DVec3,
    colour: [f32; 3],
    axis: DVec3,
) {
    quad_facing(vertices, a, b, c, d, colour, axis);
    quad_facing(vertices, a, b, c, d, colour, -axis);
}

fn triangle_both_sides(
    vertices: &mut Vec<ShipVertex>,
    a: DVec3,
    b: DVec3,
    c: DVec3,
    colour: [f32; 3],
    axis: DVec3,
) {
    triangle_facing(vertices, a, b, c, colour, axis);
    triangle_facing(vertices, a, b, c, colour, -axis);
}

/// A (tapered) prism between two ship-local points.
fn prism(
    vertices: &mut Vec<ShipVertex>,
    from: DVec3,
    to: DVec3,
    radius_from: f64,
    radius_to: f64,
    sides: usize,
    colour: [f32; 3],
    caps: bool,
) {
    let axis = (to - from).normalize();
    let helper = if axis.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let u = axis.cross(helper).normalize();
    let w = axis.cross(u);
    let ring = |centre: DVec3, radius: f64, index: usize| {
        let angle = std::f64::consts::TAU * index as f64 / sides as f64;
        centre + (u * angle.cos() + w * angle.sin()) * radius
    };
    for index in 0..sides {
        let next = index + 1;
        let middle = std::f64::consts::TAU * (index as f64 + 0.5) / sides as f64;
        let outward = u * middle.cos() + w * middle.sin();
        quad_facing(
            vertices,
            ring(from, radius_from, index),
            ring(from, radius_from, next),
            ring(to, radius_to, next),
            ring(to, radius_to, index),
            colour,
            outward,
        );
        if caps {
            triangle_facing(vertices, from, ring(from, radius_from, index), ring(from, radius_from, next), colour, -axis);
            triangle_facing(vertices, to, ring(to, radius_to, index), ring(to, radius_to, next), colour, axis);
        }
    }
}

fn line(vertices: &mut Vec<ShipVertex>, from: DVec3, to: DVec3) {
    prism(vertices, from, to, 0.07 * S, 0.07 * S, 3, ROPE, false);
}

fn build_hull(v: &mut Vec<ShipVertex>) {
    // Planked sides, bottom to sheer, in bands: tar, topsides, gilded wale.
    for segment in 0..SEGMENTS {
        let (t0, t1) = (station_t(segment), station_t(segment + 1));
        for side in [1.0, -1.0] {
            for ring in 0..RING_COUNT - 1 {
                let down = if ring < 2 { -0.7 } else { 0.0 };
                quad_facing(
                    v,
                    ring_point(t0, ring, side),
                    ring_point(t1, ring, side),
                    ring_point(t1, ring + 1, side),
                    ring_point(t0, ring + 1, side),
                    RING_COLOURS[ring],
                    DVec3::new(0.0, side, down),
                );
            }
        }
    }

    // Deck planking, bulwarks and raised decks, segment by segment.
    for segment in 0..SEGMENTS {
        let (t0, t1) = (station_t(segment), station_t(segment + 1));
        let (x0, x1) = (station_x(t0), station_x(t1));
        let (b0, b1) = (half_beam_meters(t0), half_beam_meters(t1));
        let (s0, s1) = (sheer_height_meters(t0), sheer_height_meters(t1));
        let r = raise(segment) * S;
        let rail = RAIL_HEIGHT * S;
        for side in [1.0, -1.0] {
            let outer = |x: f64, b: f64, z: f64| DVec3::new(x, side * SIDE_OUTER * b, z);
            let inner = |x: f64, b: f64, z: f64| DVec3::new(x, side * SIDE_INNER * b, z);
            let hint = DVec3::new(0.0, side, 0.0);
            if r > 0.0 {
                quad_facing(v, outer(x0, b0, s0), outer(x1, b1, s1), outer(x1, b1, s1 + r), outer(x0, b0, s0 + r), TOPSIDE, hint);
            }
            quad_facing(
                v,
                outer(x0, b0, s0 + r),
                outer(x1, b1, s1 + r),
                outer(x1, b1, s1 + r + rail),
                outer(x0, b0, s0 + r + rail),
                RAIL_OUTER,
                hint,
            );
            quad_facing(
                v,
                outer(x0, b0, s0 + r + rail),
                outer(x1, b1, s1 + r + rail),
                inner(x1, b1, s1 + r + rail),
                inner(x0, b0, s0 + r + rail),
                RAIL_CAP,
                DVec3::Z,
            );
            quad_facing(
                v,
                inner(x0, b0, s0 + r + rail),
                inner(x1, b1, s1 + r + rail),
                inner(x1, b1, s1 + r),
                inner(x0, b0, s0 + r),
                BULWARK_INNER,
                -hint,
            );
        }
        const PLANKS: usize = 8;
        for plank in 0..PLANKS {
            let (f0, f1) = (
                -SIDE_INNER + 2.0 * SIDE_INNER * plank as f64 / PLANKS as f64,
                -SIDE_INNER + 2.0 * SIDE_INNER * (plank + 1) as f64 / PLANKS as f64,
            );
            quad_facing(
                v,
                DVec3::new(x0, f0 * b0, s0 + r),
                DVec3::new(x1, f0 * b1, s1 + r),
                DVec3::new(x1, f1 * b1, s1 + r),
                DVec3::new(x0, f1 * b0, s0 + r),
                if plank % 2 == 0 { DECK_A } else { DECK_B },
                DVec3::Z,
            );
        }
    }

    // Walls where the raised decks step: the quarterdeck's face to the waist
    // (with its door), the poop's step down, the forecastle's two ends.
    for station in 1..SEGMENTS {
        let (behind, ahead) = (raise(station - 1), raise(station));
        if (behind - ahead).abs() < 1.0e-9 {
            continue;
        }
        let t = station_t(station);
        let (x, b, sheer) = (station_x(t), half_beam_meters(t), sheer_height_meters(t));
        let (low, high) = (behind.min(ahead) * S, behind.max(ahead) * S);
        let facing = if ahead < behind { DVec3::X } else { -DVec3::X };
        let w = SIDE_INNER * b;
        quad_facing(
            v,
            DVec3::new(x, -w, sheer + low),
            DVec3::new(x, w, sheer + low),
            DVec3::new(x, w, sheer + high),
            DVec3::new(x, -w, sheer + high),
            TOPSIDE,
            facing,
        );
        // A door at deck level on each of these walls that reach it.
        if low == 0.0 {
            let nudge = facing * 0.004;
            quad_facing(
                v,
                DVec3::new(x, -0.9 * S, sheer) + nudge,
                DVec3::new(x, 0.9 * S, sheer) + nudge,
                DVec3::new(x, 0.9 * S, sheer + 1.8 * S) + nudge,
                DVec3::new(x, -0.9 * S, sheer + 1.8 * S) + nudge,
                DARK_DOOR,
                facing,
            );
        }
    }

    // The transom: hull sections closed across, then two galleries of windows
    // and a gilded rail up to the top of the poop.
    let t = -1.0;
    let x = station_x(t);
    let sheer = sheer_height_meters(t);
    let half = SIDE_OUTER * half_beam_meters(t);
    for ring in 0..RING_COUNT - 1 {
        let down = if ring < 2 { -0.7 } else { 0.0 };
        quad_facing(
            v,
            ring_point(t, ring, 1.0),
            ring_point(t, ring, -1.0),
            ring_point(t, ring + 1, -1.0),
            ring_point(t, ring + 1, 1.0),
            RING_COLOURS[ring],
            DVec3::new(-1.0, 0.0, down),
        );
    }
    let levels = [
        (sheer, sheer + 2.4 * S, TOPSIDE),
        (sheer + 2.4 * S, sheer + POOP_RAISE * S, TOPSIDE),
        (sheer + POOP_RAISE * S, sheer + (POOP_RAISE + RAIL_HEIGHT) * S, GOLD),
    ];
    for (z0, z1, colour) in levels {
        quad_facing(
            v,
            DVec3::new(x, half, z0),
            DVec3::new(x, -half, z0),
            DVec3::new(x, -half, z1),
            DVec3::new(x, half, z1),
            colour,
            -DVec3::X,
        );
    }
    for (row, z_centre) in [(0, sheer + 1.2 * S), (1, sheer + 3.5 * S)] {
        let _ = row;
        for y_centre in [-2.4, 0.0, 2.4] {
            let (w, h) = (0.75 * S, 0.55 * S);
            let (xw, yc) = (x - 0.004, y_centre * S);
            quad_facing(
                v,
                DVec3::new(xw, yc + w, z_centre - h),
                DVec3::new(xw, yc - w, z_centre - h),
                DVec3::new(xw, yc - w, z_centre + h),
                DVec3::new(xw, yc + w, z_centre + h),
                DARK_OPENING,
                -DVec3::X,
            );
        }
    }
}

const DARK_DOOR: [f32; 3] = [0.06, 0.04, 0.03];

/// A point on the hull side, `u` of the way from station `segment` to the
/// next and `w` of the way from ring `ring` to the next, pushed a hair out.
fn side_point(segment: usize, ring: usize, u: f64, w: f64, side: f64) -> DVec3 {
    let (t0, t1) = (station_t(segment), station_t(segment + 1));
    let along = |ring: usize| {
        ring_point(t0, ring, side).lerp(ring_point(t1, ring, side), u)
    };
    let point = along(ring).lerp(along(ring + 1), w);
    point + DVec3::new(0.0, side * 0.004, 0.0)
}

fn build_fittings(v: &mut Vec<ShipVertex>) {
    gunports_and_guns(v);
    stem_beakhead_and_rudder(v);
    deck_furniture(v);
    stern_details(v);
    for mast in MASTS {
        mast_and_rigging(v, mast);
    }
    sails(v);
    stays_and_pennants(v);
}

/// Eight guns a side along the waist, in the dark band above the gilded wale.
fn gunports_and_guns(v: &mut Vec<ShipVertex>) {
    for segment in 7..15 {
        for side in [1.0, -1.0] {
            let corner = |u: f64, w: f64| side_point(segment, 4, u, w, side);
            quad_facing(
                v,
                corner(0.2, 0.15),
                corner(0.8, 0.15),
                corner(0.8, 0.85),
                corner(0.2, 0.85),
                DARK_OPENING,
                DVec3::new(0.0, side, 0.0),
            );
            let centre = side_point(segment, 4, 0.5, 0.5, side);
            let outward = DVec3::new(0.0, side, 0.0);
            prism(
                v,
                centre - outward * 0.3 * S,
                centre + outward * 1.3 * S,
                0.28 * S,
                0.22 * S,
                6,
                GUN,
                true,
            );
        }
    }
}

fn stem_beakhead_and_rudder(v: &mut Vec<ShipVertex>) {
    let bow_top = sheer_height_meters(1.0);
    // Stem post.
    prism(v, p(21.0, 0.0, -0.75), DVec3::new(station_x(1.0) + 0.3 * S, 0.0, bow_top), 0.3 * S, 0.3 * S, 6, WOOD, true);
    // Bowsprit, steeving up from the forecastle.
    prism(v, p(15.0, 0.0, 5.6), p(31.0, 0.0, 10.4), 0.5 * S, 0.22 * S, 8, WOOD, true);
    // Beakhead platform and a gilded lion figurehead.
    push_box(v, p(22.4, 0.0, 4.5), p(1.6, 0.9, 0.12), DECK_A);
    push_box(v, p(23.9, 0.0, 5.1), p(0.6, 0.35, 0.5), GOLD);
    push_box(v, p(24.4, 0.0, 5.45), p(0.35, 0.3, 0.3), GOLD);
    // Rudder hung on the sternpost.
    push_box(v, p(-21.6, 0.0, -0.6), p(0.6, 0.25, 2.4), WOOD);
}

fn deck_furniture(v: &mut Vec<ShipVertex>) {
    let waist = deck_z(5.0 * S);
    // Main hatch with its grating, a capstan, a longboat chocked on the waist.
    push_box(v, p(-1.5, 0.0, 0.0) + DVec3::new(0.0, 0.0, waist + 0.2 * S), p(1.6, 1.4, 0.2), DARK_OPENING);
    prism(v, DVec3::new(6.0 * S, 0.0, waist), DVec3::new(6.0 * S, 0.0, waist + 1.4 * S), 0.8 * S, 0.7 * S, 8, WOOD, true);
    let boat_z = deck_z(7.5 * S);
    push_box(v, p(7.5, 0.0, 0.0) + DVec3::new(0.0, 0.0, boat_z + 0.7 * S), p(3.2, 1.3, 0.7), TOPSIDE);
    push_box(v, p(7.5, 0.0, 0.0) + DVec3::new(0.0, 0.0, boat_z + 1.42 * S), p(3.0, 1.1, 0.05), DECK_B);
    // Binnacle and the whipstaff on the poop, by the helm.
    let poop = deck_z(-15.5 * S) + POOP_RAISE * S;
    push_box(v, p(-15.5, -1.6, 0.0) + DVec3::new(0.0, 0.0, poop + 0.6 * S), p(0.6, 0.6, 0.6), WOOD);
    push_box(v, p(-15.5, -1.6, 0.0) + DVec3::new(0.0, 0.0, poop + 1.35 * S), p(0.35, 0.35, 0.15), GOLD);
}

fn stern_details(v: &mut Vec<ShipVertex>) {
    let top = sheer_height_meters(-1.0) + (POOP_RAISE + RAIL_HEIGHT) * S;
    let x = station_x(-1.0);
    // Stern lantern.
    prism(v, DVec3::new(x + 0.3 * S, 0.0, top), DVec3::new(x + 0.3 * S, 0.0, top + 1.6 * S), 0.12 * S, 0.12 * S, 4, WOOD, false);
    push_box(v, DVec3::new(x + 0.3 * S, 0.0, top + 2.3 * S), p(0.5, 0.5, 0.8), GOLD);
    push_box(v, DVec3::new(x + 0.3 * S, 0.0, top + 3.3 * S), p(0.35, 0.35, 0.2), GOLD);
    // Ensign: a red cross on white, flying aft from a leaning staff.
    let staff_top = DVec3::new(x - 0.9 * S, 0.0, top + 6.2 * S);
    prism(v, DVec3::new(x + 0.6 * S, 0.0, top), staff_top, 0.1 * S, 0.07 * S, 4, WOOD, true);
    let (length, height) = (4.6 * S, 2.8 * S);
    let corner = |along: f64, down: f64, y: f64| {
        DVec3::new(staff_top.x - along * length, y, staff_top.z - 0.1 * S - down * height)
    };
    quad_both_sides(v, corner(0.0, 0.0, 0.0), corner(1.0, 0.0, 0.0), corner(1.0, 1.0, 0.0), corner(0.0, 1.0, 0.0), WHITE, DVec3::Y);
    for y in [0.01 * S, -0.01 * S] {
        let hint = DVec3::new(0.0, y.signum(), 0.0);
        quad_facing(v, corner(0.0, 0.4, y), corner(1.0, 0.4, y), corner(1.0, 0.6, y), corner(0.0, 0.6, y), RED, hint);
        quad_facing(v, corner(0.32, 0.0, y), corner(0.48, 0.0, y), corner(0.48, 1.0, y), corner(0.32, 1.0, y), RED, hint);
    }
}

#[derive(Clone, Copy)]
struct Mast {
    /// Design-size x of the mast, and the design-size height of the deck
    /// raise it stands on.
    x: f64,
    raise: f64,
    /// Heights of the lower mast's head and the topmast's truck.
    lower_top: f64,
    top: f64,
    /// Main and fore carry shrouds with ratlines to a top; the mizzen less.
    has_top: bool,
}

const MASTS: [Mast; 3] = [
    Mast { x: 12.5, raise: 0.0, lower_top: 19.0, top: 32.0, has_top: true },
    Mast { x: 2.5, raise: 0.0, lower_top: 23.0, top: 38.0, has_top: true },
    Mast { x: -9.0, raise: QUARTERDECK_RAISE, lower_top: 17.0, top: 27.0, has_top: false },
];

fn mast_base(mast: Mast) -> f64 {
    deck_z(mast.x * S) / S + mast.raise
}

fn mast_and_rigging(v: &mut Vec<ShipVertex>, mast: Mast) {
    let base = mast_base(mast);
    let x = mast.x;
    prism(v, p(x, 0.0, base - 1.0), p(x, 0.0, mast.lower_top), 0.55 * S, 0.38 * S, 8, WOOD, true);
    prism(v, p(x, 0.0, mast.lower_top - 2.0), p(x, 0.0, mast.top), 0.3 * S, 0.12 * S, 8, WOOD, true);
    if mast.has_top {
        push_box(v, p(x, 0.0, mast.lower_top), p(1.8, 1.5, 0.22), WOOD);
    }
    // Shrouds from the masthead to the rail, with ratlines between them.
    let shroud_count = if mast.has_top { 3 } else { 2 };
    for side in [1.0, -1.0] {
        let half_beam = half_beam_meters((x * S / (0.5 * HULL_LENGTH_METERS)).clamp(-1.0, 1.0)) / S;
        let rail_y = side * half_beam * SIDE_INNER;
        let rail_z = base + RAIL_HEIGHT;
        let head = |k: usize| {
            let spread = (k as f64 - 0.5 * (shroud_count as f64 - 1.0)) * 0.5;
            p(x + spread, side * 0.55, mast.lower_top - 0.4)
        };
        let foot = |k: usize| {
            let along = (k as f64 - 0.5 * (shroud_count as f64 - 1.0)) * 1.5;
            p(x + along, rail_y, rail_z)
        };
        for k in 0..shroud_count {
            line(v, head(k), foot(k));
        }
        if mast.has_top {
            for k in 0..shroud_count - 1 {
                for rung in 1..=6 {
                    let f = rung as f64 / 7.0;
                    line(v, foot(k).lerp(head(k), f), foot(k + 1).lerp(head(k + 1), f));
                }
            }
        }
    }
}

/// Square sails billow forward, on the main and fore masts, plus the
/// spritsail; the mizzen carries a lateen.
fn sails(v: &mut Vec<ShipVertex>) {
    // (mast x, yard height, yard length, sail depth)
    let square: [(f64, f64, f64, f64); 6] = [
        (2.5, 12.0, 18.0, 10.0),
        (2.5, 26.0, 13.0, 9.0),
        (2.5, 35.0, 8.0, 6.0),
        (12.5, 11.0, 15.0, 9.0),
        (12.5, 22.0, 11.0, 8.0),
        (12.5, 29.0, 7.0, 5.0),
    ];
    for (x, z, length, depth) in square {
        prism(v, p(x, -0.5 * length, z), p(x, 0.5 * length, z), 0.2 * S, 0.2 * S, 6, WOOD, true);
        square_sail(v, x, z - 0.3, depth, 0.46 * length, 0.4 * length, 1.5);
    }
    // Spritsail under the bowsprit's end.
    prism(v, p(27.2, -4.0, 8.9), p(27.2, 4.0, 8.9), 0.15 * S, 0.15 * S, 6, WOOD, true);
    square_sail(v, 27.2, 8.7, 5.0, 3.6, 3.2, 0.9);

    // Lateen mizzen: a long yard slung from the mast, the sail a triangle.
    let (head, tail) = (p(-4.0, 0.0, 10.5), p(-18.0, 0.0, 26.0));
    prism(v, head, tail, 0.2 * S, 0.12 * S, 6, WOOD, true);
    let clew = p(-19.0, 0.0, 8.5);
    let (columns, rows) = (6usize, 4usize);
    let point = |column: usize, row: usize| {
        let u = column as f64 / columns as f64;
        let w = row as f64 / rows as f64;
        let on_yard = head.lerp(tail, u);
        let base = on_yard.lerp(clew, w);
        base + DVec3::Y * (1.4 * S * (std::f64::consts::PI * u).sin() * (std::f64::consts::PI * w).sin() * (1.0 - w))
    };
    for column in 0..columns {
        for row in 0..rows {
            quad_both_sides(
                v,
                point(column, row),
                point(column + 1, row),
                point(column + 1, row + 1),
                point(column, row + 1),
                if column % 2 == 0 { SAIL_A } else { SAIL_B },
                DVec3::Y,
            );
        }
    }
}

fn square_sail(
    v: &mut Vec<ShipVertex>,
    x: f64,
    z_top: f64,
    depth: f64,
    half_top: f64,
    half_foot: f64,
    billow: f64,
) {
    let (columns, rows) = (6usize, 4usize);
    let point = |column: usize, row: usize| {
        let u = column as f64 / columns as f64;
        let w = row as f64 / rows as f64;
        let half = half_top + (half_foot - half_top) * w;
        let forward = billow * (std::f64::consts::PI * u).sin() * (std::f64::consts::PI * w).sin();
        p(x + forward, -half + 2.0 * half * u, z_top - depth * w)
    };
    for column in 0..columns {
        for row in 0..rows {
            quad_both_sides(
                v,
                point(column, row),
                point(column + 1, row),
                point(column + 1, row + 1),
                point(column, row + 1),
                if column % 2 == 0 { SAIL_A } else { SAIL_B },
                DVec3::X,
            );
        }
    }
}

/// Stays between the masts and the bowsprit, backstays to the rail, and a
/// pennant at each truck.
fn stays_and_pennants(v: &mut Vec<ShipVertex>) {
    line(v, p(12.5, 0.0, 32.0), p(31.0, 0.0, 10.4));
    line(v, p(2.5, 0.0, 38.0), p(12.5, 0.0, 19.5));
    line(v, p(-9.0, 0.0, 27.0), p(2.5, 0.0, 16.0));
    for side in [1.0, -1.0] {
        line(v, p(2.5, 0.0, 38.0), p(-2.0, side * 4.2, 4.2));
        line(v, p(12.5, 0.0, 32.0), p(8.0, side * 4.0, 4.2));
    }
    for mast in MASTS {
        let (x, z) = (mast.x, mast.top);
        triangle_both_sides(v, p(x, 0.0, z), p(x, 0.0, z - 1.3), p(x - 6.0, 0.0, z - 0.65), RED, DVec3::Y);
    }
}

//! Villagers: procedurally dressed people with a rigid-part walking skeleton.
//!
//! One cached "superset" body mesh holds every garment, hair style, beard and
//! hat; each instance's selection bits collapse the parts it does not wear, so
//! every person in view is one instanced draw. Bones are rigid limb segments
//! posed in the vertex shader from three numbers per person (walk phase, walk
//! weight and sit weight), so walking, standing and sitting on a cart seat
//! cost no CPU skinning.
//!
//! A person's look is a pure function of their key, so a villager walking a
//! footpath and the same villager driving a cart are visibly the same person.
use glam::DVec3;
use wgpu::util::DeviceExt;

/// Residents per village. Traffic drivers and footpath walkers both draw from
/// this pool, so a cart's driver is someone who lives in its home village.
pub const RESIDENTS: u32 = 24;

fn mix64(mut n: u64) -> u64 {
    n ^= n >> 30;
    n = n.wrapping_mul(0xbf58476d1ce4e5b9);
    n ^= n >> 27;
    n = n.wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}

/// Set in `Person::selection` for children (the shader enlarges the head).
pub const CHILD_BIT: u32 = 1 << 24;

/// Stable identity of child `index` of the household in home `home` of the
/// named village.
pub fn child_key(village: &str, home: u32, index: u32) -> u64 {
    resident_key(village, 1000 + home * 8 + index) ^ 0xc41d
}
/// How many children live in home `home` (0, 1 or 2; about one in two homes).
pub fn children_in(village: &str, home: u32) -> u32 {
    match mix64(resident_key(village, 5000 + home)) % 20 {
        0..=8 => 0,
        9..=15 => 1,
        _ => 2,
    }
}

/// Stable identity of resident `index` of the named village.
pub fn resident_key(village: &str, index: u32) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in village.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    mix64(hash ^ (u64::from(index) << 40) ^ 0x5eed_0f_9e09_1e)
}

/// Garment groups, each choosing one of up to eight values per person.
pub mod group {
    pub const LOWER: u32 = 1; // 0 trousers, 1 skirt
    pub const COAT: u32 = 2; // 0 none, 1 long coat, 2 cloak
    pub const APRON: u32 = 3; // 0 none, 1 over trousers, 2 over a skirt
    pub const HAIR: u32 = 4; // 0 cropped, 1 short, 2 long, 3 bun, 4 ponytail, 5 curls
    pub const BEARD: u32 = 5; // 0 none, 1 full, 2 moustache
    pub const HAT: u32 = 6; // 0 none, 1 flat cap, 2 straw, 3 hood, 4 headscarf, 5 felt
    pub const SLEEVES: u32 = 7; // 0 sleeved, 1 rolled to the elbow
}

/// Colour slots a vertex can take.
mod slot {
    pub const SKIN: u32 = 0;
    pub const HAIR: u32 = 1;
    pub const TOP: u32 = 2;
    pub const BOTTOM: u32 = 3;
    pub const ACCENT: u32 = 4;
    pub const HAT: u32 = 5;
    pub const SHOES: u32 = 6;
    /// Trousers, or stockings under a skirt.
    pub const LEGS: u32 = 7;
    pub const EYE: u32 = 8;
    pub const BELT: u32 = 9;
    pub const APRON: u32 = 10;
}

/// Bones. Rest pivots are in `PIVOTS` (unit-height body, feet at z = 0,
/// facing +y, x to the right).
mod bone {
    pub const ROOT: u32 = 0;
    pub const TORSO: u32 = 1;
    pub const HEAD: u32 = 2;
    pub const ARM_L: u32 = 3;
    pub const ARM_R: u32 = 4;
    pub const FORE_L: u32 = 5;
    pub const FORE_R: u32 = 6;
    pub const THIGH_L: u32 = 7;
    pub const THIGH_R: u32 = 8;
    pub const SHIN_L: u32 = 9;
    pub const SHIN_R: u32 = 10;
    pub const SHADOW: u32 = 15;
}
/// Hip joint height on a unit-height body (the shader's pivot).
pub const HIP_HEIGHT: f64 = 0.53;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Person {
    pub key: u64,
    pub height: f32,
    pub girth: f32,
    /// skin, hair, top, bottom, accent, hat, shoes (RGBA8 linear albedo).
    pub colours: [u32; 7],
    /// Three bits per garment group (group g at bit 3(g-1)).
    pub selection: u32,
    /// Walking speed, m/s.
    pub pace: f32,
}

fn pack(c: [f32; 3]) -> u32 {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    b(c[0]) | b(c[1]) << 8 | b(c[2]) << 16 | 255 << 24
}

const SKIN: [[f32; 3]; 8] = [
    [0.80, 0.60, 0.50],
    [0.74, 0.53, 0.42],
    [0.68, 0.48, 0.36],
    [0.60, 0.41, 0.29],
    [0.50, 0.33, 0.22],
    [0.40, 0.26, 0.17],
    [0.29, 0.18, 0.12],
    [0.20, 0.13, 0.09],
];
const HAIR: [[f32; 3]; 9] = [
    [0.025, 0.02, 0.018],
    [0.07, 0.045, 0.03],
    [0.14, 0.085, 0.045],
    [0.24, 0.12, 0.05],
    [0.34, 0.12, 0.04],
    [0.55, 0.42, 0.22],
    [0.42, 0.36, 0.27],
    [0.38, 0.37, 0.36],
    [0.66, 0.65, 0.62],
];
/// Natural dyes and undyed cloth.
const CLOTH: [[f32; 3]; 15] = [
    [0.58, 0.53, 0.43], // undyed linen
    [0.46, 0.41, 0.32], // oatmeal
    [0.42, 0.11, 0.08], // madder red
    [0.40, 0.19, 0.08], // rust
    [0.13, 0.20, 0.36], // woad blue
    [0.07, 0.09, 0.20], // indigo
    [0.54, 0.44, 0.13], // weld yellow
    [0.21, 0.27, 0.12], // moss
    [0.08, 0.16, 0.09], // forest green
    [0.25, 0.16, 0.09], // walnut brown
    [0.29, 0.29, 0.28], // grey wool
    [0.09, 0.09, 0.10], // charcoal
    [0.26, 0.11, 0.21], // plum
    [0.46, 0.30, 0.10], // ochre
    [0.16, 0.25, 0.25], // teal
];
const LEATHER: [[f32; 3]; 4] = [
    [0.12, 0.07, 0.04],
    [0.06, 0.045, 0.035],
    [0.20, 0.12, 0.06],
    [0.03, 0.03, 0.03],
];

impl Person {
    pub fn from_key(key: u64) -> Self {
        Self::make(key, false)
    }
    /// A child: smaller, slimmer, quicker, with a relatively larger head (the
    /// shader's child bit), no beard, apron or long coat.
    pub fn child_from_key(key: u64) -> Self {
        Self::make(key, true)
    }
    pub fn is_child(&self) -> bool {
        self.selection & CHILD_BIT != 0
    }
    fn make(key: u64, child: bool) -> Self {
        let state = std::cell::Cell::new(mix64(key ^ 0x9e3779b97f4a7c15));
        let next = || {
            state.set(mix64(state.get().wrapping_add(0x9e3779b97f4a7c15)));
            (state.get() >> 11) as f64 / (1u64 << 53) as f64
        };
        let pick = |n: usize| ((next() * n as f64) as usize).min(n - 1);
        let feminine = pick(2) == 0;
        let old = pick(6) == 0;
        let mut sel = [0u32; 8];
        sel[group::LOWER as usize] = if feminine { u32::from(pick(5) != 0) } else { u32::from(pick(14) == 0) };
        sel[group::COAT as usize] = [0, 0, 0, 1, 1, 2][pick(6)];
        let apron = pick(4) == 0;
        sel[group::APRON as usize] = if apron { 1 + sel[group::LOWER as usize] } else { 0 };
        sel[group::HAIR as usize] = if feminine {
            [1, 2, 2, 3, 3, 4, 5][pick(7)]
        } else {
            [0, 1, 1, 1, 2, 4, 5][pick(7)]
        };
        sel[group::BEARD as usize] = if feminine { 0 } else { [0, 0, 1, 1, 2][pick(5)] };
        sel[group::HAT as usize] = if feminine {
            [0, 0, 2, 3, 4, 4][pick(6)]
        } else {
            [0, 0, 1, 1, 2, 3, 5][pick(7)]
        };
        sel[group::SLEEVES as usize] = u32::from(pick(3) == 0 && sel[group::COAT as usize] != 1);
        let old = old && !child;
        if child {
            sel[group::COAT as usize] = if pick(10) == 0 { 2 } else { 0 };
            sel[group::APRON as usize] = 0;
            sel[group::BEARD as usize] = 0;
            sel[group::HAT as usize] = [0, 0, 0, 0, 1, 2, 4][pick(7)];
            sel[group::HAIR as usize] = if feminine { [2, 3, 4, 4, 5][pick(5)] } else { [0, 1, 1, 5][pick(4)] };
            sel[group::SLEEVES as usize] = u32::from(pick(2) == 0);
        }
        let selection = (1..8).fold(0, |s, g| s | sel[g] << (3 * (g - 1))) | if child { CHILD_BIT } else { 0 };
        let height = if child {
            0.95 + 0.5 * next()
        } else if feminine {
            1.56 + 0.18 * next()
        } else {
            1.66 + 0.22 * next()
        };
        let girth = if child { 0.84 + 0.14 * next() } else { 0.9 + 0.28 * next() + if old { 0.05 } else { 0.0 } };
        let hair = if old { HAIR[7 + pick(2)] } else { HAIR[pick(7)] };
        let top = CLOTH[pick(CLOTH.len())];
        let mut bottom = CLOTH[pick(CLOTH.len())];
        if bottom == top {
            bottom = CLOTH[(pick(CLOTH.len()) + 3) % CLOTH.len()];
        }
        let accent = CLOTH[pick(CLOTH.len())];
        let hat = match sel[group::HAT as usize] {
            2 => [0.62, 0.50, 0.27], // straw
            1 | 5 => [[0.12, 0.10, 0.08], [0.22, 0.18, 0.13], [0.07, 0.07, 0.08]][pick(3)], // felt
            _ => CLOTH[pick(CLOTH.len())],
        };
        let shoes = LEATHER[pick(LEATHER.len())];
        Self {
            key,
            height: height as f32,
            girth: girth as f32,
            colours: [
                pack(SKIN[pick(SKIN.len())]),
                pack(hair),
                pack(top),
                pack(bottom),
                pack(accent),
                pack(hat),
                pack(shoes),
            ],
            selection,
            pace: if child { 1.25 + 0.65 * next() } else { (if old { 0.95 } else { 1.15 }) + 0.35 * next() } as f32,
        }
    }
    #[cfg(test)]
    pub fn choice(&self, g: u32) -> u32 {
        (self.selection >> (3 * (g - 1))) & 7
    }
}

/// Where and how to draw one person this frame, planet frame.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub person: Person,
    /// Ground point between the feet.
    pub feet: DVec3,
    pub forward: DVec3,
    pub up: DVec3,
    /// Walk cycle angle (radians), or a slow clock while standing.
    pub phase: f32,
    pub walk: f32,
    pub sit: f32,
    /// Head turn, radians, positive to the left.
    pub look: f32,
    pub fade: f32,
    /// Tunnel lamp group, and the feet in the road field frame.
    pub light: Option<(DVec3, u32)>,
}

/// Walk phase advanced over `metres` for this person (one cycle is two steps).
pub fn stride_phase(person: &Person, metres: f64) -> f64 {
    metres / (0.82 * f64::from(person.height)) * std::f64::consts::PI
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    tags: u32,
}
fn tags(bone: u32, slot: u32, group: u32, mask: u32) -> u32 {
    bone | slot << 4 | group << 8 | mask << 11
}
fn mask(values: &[u32]) -> u32 {
    values.iter().fold(0, |m, v| m | 1 << v)
}

struct Builder {
    mesh: Vec<Vertex>,
    tag: u32,
    /// Full detail for close people; the distant body has fewer facets and
    /// no face details or curls.
    fine: bool,
}
impl Builder {
    fn set(&mut self, bone: u32, slot: u32) -> &mut Self {
        self.tag = tags(bone, slot, 0, 0);
        self
    }
    fn only(&mut self, bone: u32, slot: u32, group: u32, values: &[u32]) -> &mut Self {
        self.tag = tags(bone, slot, group, mask(values));
        self
    }
    fn triangle(&mut self, p: [DVec3; 3], n: [DVec3; 3]) {
        for i in 0..3 {
            self.mesh.push(Vertex {
                position: p[i].as_vec3().to_array(),
                normal: n[i].normalize_or_zero().as_vec3().to_array(),
                tags: self.tag,
            });
        }
    }
    /// Tapered elliptical tube from `a` to `b`, optionally only over the
    /// angular range `keep` (angle from +y toward +x, radians), capped.
    fn tube(
        &mut self,
        a: DVec3,
        b: DVec3,
        ra: [f64; 2],
        rb: [f64; 2],
        caps: [bool; 2],
        keep: impl Fn(f64) -> bool,
    ) -> &mut Self {
        let sides = if self.fine { 10 } else { 6 };
        let axis = (b - a).normalize();
        let u = (DVec3::X - axis * axis.x).normalize();
        // Angle 0 faces the body's front whichever way the tube runs.
        let mut v = axis.cross(u);
        if v.y < 0.0 {
            v = -v;
        }
        let ring = |c: DVec3, r: [f64; 2], i: usize| {
            let t = std::f64::consts::TAU * i as f64 / sides as f64;
            // Angle 0 is +y (the front of the body).
            let (s, co) = t.sin_cos();
            let p = c + u * (s * r[0]) + v * (co * r[1]);
            let n = u * (s / r[0].max(1e-4)) + v * (co / r[1].max(1e-4));
            (p, n, t)
        };
        for i in 0..sides {
            let mid = std::f64::consts::TAU * (i as f64 + 0.5) / sides as f64;
            if !keep(mid) {
                continue;
            }
            let (p0, n0, _) = ring(a, ra, i);
            let (p1, n1, _) = ring(a, ra, i + 1);
            let (p2, n2, _) = ring(b, rb, i + 1);
            let (p3, n3, _) = ring(b, rb, i);
            self.triangle([p0, p1, p2], [n0, n1, n2]);
            self.triangle([p0, p2, p3], [n0, n2, n3]);
            if caps[0] {
                self.triangle([a, p1, p0], [-axis; 3]);
            }
            if caps[1] {
                self.triangle([b, p3, p2], [axis; 3]);
            }
        }
        self
    }
    fn cylinder(&mut self, a: DVec3, b: DVec3, ra: f64, rb: f64) -> &mut Self {
        self.tube(a, b, [ra, ra], [rb, rb], [true, true], |_| true)
    }
    /// Ellipsoid, keeping the triangles whose centre direction passes `keep`.
    fn ellipsoid(&mut self, c: DVec3, r: DVec3, keep: impl Fn(DVec3) -> bool) -> &mut Self {
        let (rings, segments) = if self.fine { (8, 12) } else { (4, 6) };
        let dir = |i: usize, j: usize| {
            let a = i as f64 * std::f64::consts::PI / rings as f64;
            let b = j as f64 * std::f64::consts::TAU / segments as f64;
            DVec3::new(a.sin() * b.sin(), a.sin() * b.cos(), a.cos())
        };
        for i in 0..rings {
            for j in 0..segments {
                let d = [dir(i, j), dir(i, j + 1), dir(i + 1, j + 1), dir(i + 1, j)];
                if !keep((d[0] + d[1] + d[2] + d[3]).normalize_or_zero()) {
                    continue;
                }
                let p = d.map(|d| c + d * r);
                let n = d.map(|d| d / r);
                for [x, y, z] in [[0, 2, 1], [0, 3, 2]] {
                    if (p[y] - p[x]).cross(p[z] - p[x]).length_squared() > 1e-14 {
                        self.triangle([p[x], p[y], p[z]], [n[x], n[y], n[z]]);
                    }
                }
            }
        }
        self
    }
    fn sphere(&mut self, c: DVec3, r: DVec3) -> &mut Self {
        self.ellipsoid(c, r, |_| true)
    }
    fn cuboid(&mut self, lo: DVec3, hi: DVec3) -> &mut Self {
        let c = |x: usize, y: usize, z: usize| {
            DVec3::new([lo.x, hi.x][x], [lo.y, hi.y][y], [lo.z, hi.z][z])
        };
        let faces = [
            ([c(1, 0, 0), c(1, 1, 0), c(1, 1, 1), c(1, 0, 1)], DVec3::X),
            ([c(0, 1, 0), c(0, 0, 0), c(0, 0, 1), c(0, 1, 1)], -DVec3::X),
            ([c(1, 1, 0), c(0, 1, 0), c(0, 1, 1), c(1, 1, 1)], DVec3::Y),
            ([c(0, 0, 0), c(1, 0, 0), c(1, 0, 1), c(0, 0, 1)], -DVec3::Y),
            ([c(0, 0, 1), c(1, 0, 1), c(1, 1, 1), c(0, 1, 1)], DVec3::Z),
            ([c(0, 1, 0), c(1, 1, 0), c(1, 0, 0), c(0, 0, 0)], -DVec3::Z),
        ];
        for (q, n) in faces {
            self.triangle([q[0], q[1], q[2]], [n; 3]);
            self.triangle([q[0], q[2], q[3]], [n; 3]);
        }
        self
    }
}

const HEAD_CENTRE: DVec3 = DVec3::new(0.0, 0.005, 0.93);
const HEAD_R: DVec3 = DVec3::new(0.058, 0.066, 0.072);
fn v(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// The superset body: every garment, style and hat, tagged by bone, colour
/// slot and the garment choices that show it. Unit height, feet at z = 0.
pub fn body(fine: bool) -> Vec<Vertex> {
    use bone::*;
    use group::*;
    let mut b = Builder {
        mesh: Vec::with_capacity(16_000),
        tag: 0,
        fine,
    };
    let all = |_: f64| true;
    // Head, face and neck.
    b.set(HEAD, slot::SKIN).ellipsoid(HEAD_CENTRE, HEAD_R, |_| true);
    if fine {
        b.cuboid(v(-0.009, 0.062, 0.912), v(0.009, 0.08, 0.938));
        for x in [-0.009, 0.009] {
            b.ellipsoid(v(x * 7.0, 0.0, 0.93), v(0.012, 0.02, 0.022), |_| true);
        }
        b.set(HEAD, slot::EYE);
        for x in [-0.022, 0.022] {
            b.cuboid(v(x - 0.007, 0.058, 0.942), v(x + 0.007, 0.066, 0.951));
        }
    }
    b.set(TORSO, slot::SKIN).cylinder(v(0.0, 0.0, 0.82), v(0.0, 0.003, 0.89), 0.029, 0.027);
    // Torso, shoulders and pelvis.
    b.set(TORSO, slot::TOP)
        .tube(v(0.0, 0.0, 0.58), v(0.0, 0.004, 0.74), [0.128, 0.08], [0.158, 0.094], [false, false], all)
        .tube(v(0.0, 0.004, 0.74), v(0.0, 0.0, 0.835), [0.158, 0.094], [0.115, 0.07], [false, true], all);
    for x in [-0.152, 0.152] {
        b.sphere(v(x, 0.0, 0.8), v(0.048, 0.05, 0.045));
    }
    b.set(ROOT, slot::LEGS).tube(v(0.0, 0.0, 0.47), v(0.0, 0.0, 0.6), [0.125, 0.085], [0.128, 0.08], [true, false], all);
    b.set(ROOT, slot::BELT).tube(v(0.0, 0.0, 0.578), v(0.0, 0.0, 0.602), [0.134, 0.088], [0.134, 0.088], [false, false], all);
    b.cuboid(v(-0.02, 0.083, 0.576), v(0.02, 0.092, 0.604));
    // Legs and boots.
    for (x, thigh, shin) in [(-0.085, THIGH_L, SHIN_L), (0.085, THIGH_R, SHIN_R)] {
        b.set(thigh, slot::LEGS).cylinder(v(x, 0.0, 0.535), v(x, 0.0, 0.285), 0.062, 0.046);
        b.set(shin, slot::LEGS).cylinder(v(x, 0.0, 0.29), v(x, 0.0, 0.13), 0.045, 0.036);
        b.set(shin, slot::SHOES)
            .cylinder(v(x, 0.0, 0.15), v(x, 0.0, 0.03), 0.04, 0.039)
            .cuboid(v(x - 0.036, -0.045, 0.0), v(x + 0.036, 0.115, 0.055));
    }
    // Arms: sleeved or rolled forearms, and hands.
    for (x, arm, fore) in [(-1.0, ARM_L, FORE_L), (1.0, ARM_R, FORE_R)] {
        b.set(arm, slot::TOP).cylinder(v(x * 0.172, 0.0, 0.81), v(x * 0.188, 0.0, 0.62), 0.042, 0.036);
        let (e, w) = (v(x * 0.188, 0.0, 0.625), v(x * 0.198, 0.004, 0.465));
        b.only(fore, slot::TOP, SLEEVES, &[0]).cylinder(e, w, 0.036, 0.029);
        b.only(fore, slot::SKIN, SLEEVES, &[1]).cylinder(e, w, 0.031, 0.025);
        b.only(fore, slot::TOP, SLEEVES, &[1]).cylinder(e + v(0.0, 0.0, 0.012), e - v(0.0, 0.0, 0.03), 0.04, 0.038);
        b.set(fore, slot::SKIN).ellipsoid(v(x * 0.2, 0.008, 0.425), v(0.021, 0.03, 0.042), |_| true);
    }
    // Skirt (a bell from the waist), apron, long coat and cloak.
    b.only(ROOT, slot::BOTTOM, LOWER, &[1]).tube(
        v(0.0, 0.0, 0.6),
        v(0.0, 0.012, 0.13),
        [0.138, 0.092],
        [0.25, 0.225],
        [false, false],
        all,
    );
    let front = |a: f64| a < 0.85 || a > std::f64::consts::TAU - 0.85;
    b.only(ROOT, slot::APRON, APRON, &[1])
        .tube(v(0.0, 0.0, 0.6), v(0.0, 0.03, 0.3), [0.142, 0.098], [0.15, 0.15], [false, false], front);
    b.only(ROOT, slot::APRON, APRON, &[2])
        .tube(v(0.0, 0.0, 0.6), v(0.0, 0.03, 0.2), [0.15, 0.1], [0.26, 0.245], [false, false], front);
    let open = |a: f64| a > 0.42 && a < std::f64::consts::TAU - 0.42;
    b.only(TORSO, slot::ACCENT, COAT, &[1])
        .tube(v(0.0, 0.0, 0.575), v(0.0, 0.004, 0.74), [0.137, 0.088], [0.167, 0.102], [false, false], open)
        .tube(v(0.0, 0.004, 0.74), v(0.0, 0.0, 0.84), [0.167, 0.102], [0.124, 0.078], [false, false], open);
    b.only(ROOT, slot::ACCENT, COAT, &[1])
        .tube(v(0.0, 0.0, 0.61), v(0.0, -0.01, 0.31), [0.142, 0.094], [0.2, 0.16], [false, false], open);
    for (x, arm, fore) in [(-1.0, ARM_L, FORE_L), (1.0, ARM_R, FORE_R)] {
        b.only(arm, slot::ACCENT, COAT, &[1]).cylinder(v(x * 0.172, 0.0, 0.825), v(x * 0.188, 0.0, 0.615), 0.05, 0.043);
        b.only(fore, slot::ACCENT, COAT, &[1]).cylinder(v(x * 0.188, 0.0, 0.63), v(x * 0.198, 0.004, 0.48), 0.042, 0.036);
    }
    let back = |a: f64| a > 1.2 && a < std::f64::consts::TAU - 1.2;
    b.only(TORSO, slot::ACCENT, COAT, &[2])
        .tube(v(0.0, 0.0, 0.85), v(0.0, -0.03, 0.3), [0.2, 0.12], [0.27, 0.2], [false, false], back)
        .tube(v(0.0, 0.0, 0.85), v(0.0, 0.0, 0.875), [0.2, 0.12], [0.06, 0.05], [false, false], |_| true);
    // Hair.
    let cap = |d: DVec3| d.z > 0.42 || (d.y < 0.25 && d.z > -0.3);
    let hair_r = HEAD_R + DVec3::splat(0.008);
    b.only(HEAD, slot::HAIR, HAIR, &[1, 2, 3, 4, 5]).ellipsoid(HEAD_CENTRE + v(0.0, -0.002, 0.004), hair_r, cap);
    b.only(HEAD, slot::HAIR, HAIR, &[2]).tube(
        v(0.0, -0.012, 0.95),
        v(0.0, -0.035, 0.79),
        [0.068, 0.064],
        [0.08, 0.05],
        [false, false],
        |a| a > 1.25 && a < std::f64::consts::TAU - 1.25,
    );
    b.only(HEAD, slot::HAIR, HAIR, &[3]).sphere(v(0.0, -0.068, 0.975), v(0.032, 0.028, 0.03));
    b.only(HEAD, slot::HAIR, HAIR, &[4])
        .sphere(v(0.0, -0.07, 0.96), v(0.018, 0.016, 0.018))
        .cylinder(v(0.0, -0.075, 0.955), v(0.0, -0.09, 0.81), 0.017, 0.009);
    b.only(HEAD, slot::HAIR, HAIR, &[5]);
    for i in 0..if fine { 14 } else { 0 } {
        let a = i as f64 * 2.399963;
        let z = 0.25 + 0.75 * (i as f64 / 14.0);
        let d = v(a.sin() * (1.0 - z * z).sqrt(), (a.cos() * (1.0 - z * z).sqrt()).min(0.3), z).normalize();
        b.sphere(HEAD_CENTRE + d * hair_r, DVec3::splat(0.024));
    }
    // Beards.
    b.only(HEAD, slot::HAIR, BEARD, &[1]).ellipsoid(
        v(0.0, 0.03, 0.892),
        v(0.05, 0.042, 0.042),
        |d| d.y > -0.2 && d.z < 0.6,
    );
    b.only(HEAD, slot::HAIR, BEARD, &[1, 2]).cuboid(v(-0.024, 0.064, 0.904), v(0.024, 0.074, 0.913));
    // Hats.
    b.only(HEAD, slot::HAT, HAT, &[1])
        .ellipsoid(v(0.0, 0.006, 0.985), v(0.07, 0.077, 0.034), |d| d.z > -0.05)
        .cuboid(v(-0.05, 0.055, 0.972), v(0.05, 0.11, 0.98));
    b.only(HEAD, slot::HAT, HAT, &[2])
        .cylinder(v(0.0, 0.0, 0.982), v(0.0, 0.0, 0.99), 0.15, 0.15)
        .cylinder(v(0.0, 0.0, 0.99), v(0.0, 0.0, 1.05), 0.072, 0.063);
    b.only(HEAD, slot::HAT, HAT, &[5])
        .cylinder(v(0.0, 0.0, 0.982), v(0.0, 0.0, 0.992), 0.105, 0.105)
        .cylinder(v(0.0, 0.0, 0.99), v(0.0, 0.0, 1.11), 0.066, 0.06);
    b.only(HEAD, slot::HAT, HAT, &[3]).ellipsoid(
        HEAD_CENTRE + v(0.0, -0.012, 0.006),
        HEAD_R + v(0.025, 0.026, 0.024),
        |d| !(d.y > 0.25 && d.z < 0.8 && d.z > -0.75),
    );
    b.only(TORSO, slot::HAT, HAT, &[3]).tube(
        v(0.0, -0.01, 0.88),
        v(0.0, 0.0, 0.79),
        [0.085, 0.085],
        [0.17, 0.11],
        [false, false],
        |a| a > 0.5 && a < std::f64::consts::TAU - 0.5,
    );
    b.only(HEAD, slot::HAT, HAT, &[4])
        .ellipsoid(HEAD_CENTRE + v(0.0, -0.004, 0.004), HEAD_R + DVec3::splat(0.014), |d| {
            d.z > 0.2 || (d.y < 0.1 && d.z > -0.35)
        })
        .sphere(v(0.0, -0.08, 0.89), v(0.024, 0.02, 0.022));
    // Soft contact shadow, drawn by its own blended pipeline.
    if !fine {
        return b.mesh;
    }
    b.set(SHADOW, 0);
    for i in 0..16 {
        let a = |k: usize| std::f64::consts::TAU * k as f64 / 16.0;
        let p = |k: usize| v(a(k).cos() * 0.2, a(k).sin() * 0.2, 0.012);
        b.triangle([v(0.0, 0.0, 0.012), p(i), p(i + 1)], [DVec3::Z; 3]);
    }
    b.mesh
}

pub fn shader_source() -> String {
    let road = include_str!("road.wgsl");
    let start = road.find("fn lantern_hash(").expect("road lamp helpers");
    let end = road.find("fn project_vertex(").expect("end of road lamp helpers");
    format!("{}\n{}", include_str!("people.wgsl"), &road[start..end])
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    origin_phase: [f32; 4],
    forward_walk: [f32; 4],
    up_sit: [f32; 4],
    body: [f32; 4],
    field: [f32; 4],
    colours_a: [u32; 4],
    colours_b: [u32; 4],
}
const MAX_PEOPLE: usize = 512;
const DRAW_DISTANCE: f64 = 450.0;
/// Closer than this, people get the full-detail body.
const FINE_DISTANCE: f64 = 40.0;

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    /// Close and distant bodies, and the contact shadow.
    bodies: [std::ops::Range<u32>; 2],
    shadow: std::ops::Range<u32>,
    instances: wgpu::Buffer,
    frame: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    outdoor_lights: wgpu::BindGroup,
    /// Instances in four runs: lit by the road's tunnel lamps (close, far),
    /// then outdoors (close, far).
    runs: [u32; 4],
    upload: Vec<Instance>,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
    ) -> Self {
        let mut mesh = body(true);
        let split = mesh.iter().position(|v| v.tags & 15 == bone::SHADOW).unwrap_or(mesh.len());
        let shadow_mesh = mesh.split_off(split);
        let fine_end = mesh.len() as u32;
        mesh.extend(body(false));
        let coarse_end = mesh.len() as u32;
        mesh.extend(shadow_mesh);
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("villager body"),
            contents: bytemuck::cast_slice(&mesh),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("villager instances"),
            size: (MAX_PEOPLE * size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("villager road frame"),
            contents: bytemuck::cast_slice(&[[0.0_f32; 4]; 3]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("villager frame layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("villager frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            }],
        });
        // Out of doors nobody is lit by a tunnel lamp: an empty list group.
        let lighting = crate::road_lighting::layout(device);
        let empty = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("villager outdoor lamp list"),
            contents: bytemuck::cast_slice(&[[0.0_f32; 4]; crate::road_lighting::MAX_LIGHTS + 1]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("villager placeholder texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255; 4],
        );
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let outdoor_lights = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("villager outdoor lights"),
            layout: &lighting,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: empty.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("villager pipeline layout"),
            bind_group_layouts: &[Some(camera), Some(&lighting), Some(&frame_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("villager shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        const V: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Uint32];
        const I: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
            3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Uint32x4,9=>Uint32x4
        ];
        let make = |label: &str, fragment: &str, blend: wgpu::BlendState, write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        wgpu::VertexBufferLayout {
                            array_stride: size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &V,
                        },
                        wgpu::VertexBufferLayout {
                            array_stride: size_of::<Instance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &I,
                        },
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: crate::pipeline_cache::get(),
            })
        };
        let pipeline = make("instanced villagers", "fs_main", wgpu::BlendState::REPLACE, true);
        let shadow_pipeline = make(
            "villager contact shadows",
            "fs_shadow",
            wgpu::BlendState::ALPHA_BLENDING,
            false,
        );
        Self {
            pipeline,
            shadow_pipeline,
            vertices,
            bodies: [0..fine_end, fine_end..coarse_end],
            shadow: coarse_end..mesh.len() as u32,
            instances,
            frame,
            frame_group,
            outdoor_lights,
            runs: [0; 4],
            upload: Vec::with_capacity(MAX_PEOPLE),
        }
    }

    /// Uploads this frame's people. `lit` may stand in a road tunnel and are
    /// lit by its lamps (`frame` is the road field's frame); `outdoor` never.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        eye: DVec3,
        view: DVec3,
        lit: &[Placement],
        outdoor: &[Placement],
        frame: Option<&crate::road_network_field::Frame>,
    ) {
        self.upload.clear();
        let vec4 = |p: DVec3, w: f64| [p.x as f32, p.y as f32, p.z as f32, w as f32];
        if let Some(f) = frame {
            queue.write_buffer(
                &self.frame,
                0,
                bytemuck::cast_slice(&[vec4(f.across, 0.0), vec4(f.forward, 0.0), vec4(f.up, 0.0)]),
            );
        }
        let push = |p: &Placement, upload: &mut Vec<Instance>, close: bool| {
            let relative = p.feet - eye;
            let distance = relative.length();
            if upload.len() >= MAX_PEOPLE
                || distance > DRAW_DISTANCE
                || relative.dot(view) < -3.0
                || p.fade <= 0.0
                || (distance < FINE_DISTANCE) != close
            {
                return;
            }
            let (field, light) = p.light.map_or((DVec3::ZERO, -1.0), |(f, l)| (f, f64::from(l)));
            let c = p.person.colours;
            upload.push(Instance {
                origin_phase: vec4(relative, f64::from(p.phase)),
                forward_walk: vec4(p.forward, f64::from(p.walk)),
                up_sit: vec4(p.up, f64::from(p.sit)),
                body: [p.person.height, p.person.girth, p.look, p.fade.min(1.0)],
                field: vec4(field, light),
                colours_a: [c[0], c[1], c[2], c[3]],
                colours_b: [c[4], c[5], c[6], p.person.selection],
            });
        };
        for (run, (people, close)) in [(lit, true), (lit, false), (outdoor, true), (outdoor, false)]
            .into_iter()
            .enumerate()
        {
            let start = self.upload.len();
            for p in people {
                push(p, &mut self.upload, close);
            }
            self.runs[run] = (self.upload.len() - start) as u32;
        }
        if !self.upload.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.upload));
        }
    }
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera: &'a wgpu::BindGroup,
        road_lights: Option<&'a wgpu::BindGroup>,
        outdoors_visible: bool,
    ) {
        if self.upload.is_empty() {
            return;
        }
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(2, &self.frame_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        pass.set_pipeline(&self.pipeline);
        let r = self.runs;
        let lit = r[0] + r[1];
        let outdoor = lit..lit + if outdoors_visible { r[2] + r[3] } else { 0 };
        if let Some(lights) = road_lights.filter(|_| lit > 0) {
            pass.set_bind_group(1, lights, &[]);
            pass.draw(self.bodies[0].clone(), 0..r[0]);
            pass.draw(self.bodies[1].clone(), r[0]..lit);
        }
        if !outdoor.is_empty() {
            pass.set_bind_group(1, &self.outdoor_lights, &[]);
            pass.draw(self.bodies[0].clone(), outdoor.start..outdoor.start + r[2]);
            pass.draw(self.bodies[1].clone(), outdoor.start + r[2]..outdoor.end);
            pass.set_pipeline(&self.shadow_pipeline);
            pass.draw(self.shadow.clone(), outdoor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_is_finite_human_scale_and_every_choice_has_geometry() {
        let coarse = body(false);
        let mesh = body(true);
        assert_eq!(mesh.len() % 3, 0);
        assert!(mesh.len() < 30_000, "{} vertices", mesh.len());
        assert!(coarse.len() * 3 < mesh.len(), "{} far vertices", coarse.len());
        assert!(!coarse.iter().any(|v| v.tags & 15 == bone::SHADOW));
        for t in mesh.chunks(3) {
            // Triangles never mix tags: hidden choices collapse whole faces.
            assert!(t.iter().all(|v| v.tags == t[0].tags));
        }
        for v in &mesh {
            let p = glam::Vec3::from(v.position);
            assert!(p.is_finite() && glam::Vec3::from(v.normal).is_finite());
            assert!(p.z >= -0.001 && p.z < 1.12 && p.x.abs() < 0.3 && p.y.abs() < 0.3, "{p:?}");
        }
        let has = |g: u32, value: u32| {
            mesh.iter()
                .any(|v| (v.tags >> 8) & 7 == g && (v.tags >> 11) & (1 << value) != 0)
        };
        for (g, values) in [
            (group::LOWER, 1..2),
            (group::COAT, 1..3),
            (group::APRON, 1..3),
            (group::HAIR, 1..6),
            (group::BEARD, 1..3),
            (group::HAT, 1..6),
            (group::SLEEVES, 0..2),
        ] {
            for value in values {
                assert!(has(g, value), "group {g} value {value}");
            }
        }
        // Every bone the shader poses carries geometry.
        for b in 0..=10 {
            assert!(mesh.iter().any(|v| v.tags & 15 == b), "bone {b}");
        }
    }
    #[test]
    fn people_are_varied_stable_and_plausible() {
        assert_eq!(Person::from_key(42), Person::from_key(42));
        let crowd: Vec<_> = (0..400).map(|i| Person::from_key(resident_key("OAKVALE", i))).collect();
        let distinct = |f: &dyn Fn(&Person) -> u32| {
            let mut v: Vec<_> = crowd.iter().map(|p| f(p)).collect();
            v.sort();
            v.dedup();
            v.len()
        };
        assert!(distinct(&|p| p.choice(group::HAT)) == 6);
        assert!(distinct(&|p| p.choice(group::HAIR)) == 6);
        assert!(distinct(&|p| p.colours[2]) >= 12);
        assert!(distinct(&|p| p.selection) > 150);
        for p in &crowd {
            assert!((1.5..1.9).contains(&p.height) && (0.85..1.25).contains(&p.girth));
            // An apron matches the garment under it; a long coat has sleeves.
            let apron = p.choice(group::APRON);
            assert!(apron == 0 || apron == 1 + p.choice(group::LOWER));
            assert!(!(p.choice(group::COAT) == 1 && p.choice(group::SLEEVES) == 1));
        }
        let children: Vec<_> = (0..200).map(|i| Person::child_from_key(child_key("OAKVALE", i, 0))).collect();
        for c in &children {
            assert!(c.is_child() && (0.9..1.5).contains(&c.height) && c.pace > 1.2);
            assert_eq!(c.choice(group::BEARD), 0);
            assert_eq!(c.choice(group::APRON), 0);
            assert_ne!(c.choice(group::COAT), 1);
        }
        assert!(!crowd.iter().any(|p| p.is_child()));
        let homes: Vec<_> = (0..400).map(|h| children_in("OAKVALE", h)).collect();
        assert!(homes.contains(&0) && homes.contains(&1) && homes.contains(&2));
        // Different villages, different people.
        assert_ne!(resident_key("OAKVALE", 3), resident_key("ASHFORD", 3));
        assert_ne!(resident_key("OAKVALE", 3), resident_key("OAKVALE", 4));
    }
    #[test]
    fn villager_shader_and_instance_layout_validate() {
        assert_eq!(size_of::<Instance>(), 112);
        let m = wgpu::naga::front::wgsl::parse_str(&shader_source()).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .unwrap();
    }
}

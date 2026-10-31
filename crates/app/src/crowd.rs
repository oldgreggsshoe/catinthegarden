//! Villagers on foot: households live in the village's houses, walk the worn
//! desire paths between doors and the court, stand about, and go indoors for
//! a while. Children play about the lanes.
//!
//! Each adult resident has a home, and may own a cart: when the traffic says
//! their cart is due to leave they walk down to the court, climb on and drive
//! off (they are then away from the village until it parks again). Carts
//! arriving from elsewhere bring visitors, who walk up to the inn, market or
//! a friend's door and come back to their cart when it is time to go.
//!
//! The walk graph is the footpath network the village path worker already
//! routes (door connectors, shared lanes, the court mouth), so people follow
//! the dirt they wore; across the paved court they walk straight to the cart.
use crate::people::{self, Person, Placement};
use crate::traffic::ParkedCart;
use glam::{DVec2, DVec3};
use std::collections::{BinaryHeap, HashMap, HashSet};

/// One village's footpaths as a graph on the planet (unit directions).
#[derive(Clone, Debug, Default)]
pub struct Walkway {
    /// The village's name, which keys its residents (shared with traffic).
    pub village: String,
    pub nodes: Vec<DVec3>,
    pub links: Vec<Vec<u32>>,
    /// Door nodes, one per connected building.
    pub doors: Vec<u32>,
    /// Parallel to `doors`: a dwelling (else an inn, workshop, market...).
    pub homes: Vec<bool>,
    /// Where the path meets the parking court, if it has one.
    pub mouth: Option<u32>,
}

impl Walkway {
    /// Builds the graph from trail segments in a local frame (2D metres),
    /// projected to unit directions by `world`. Doors carry whether they are
    /// a dwelling.
    pub fn from_trails(
        village: String,
        trails: &[(DVec2, DVec2)],
        doors: &[(DVec2, bool)],
        mouth: Option<DVec2>,
        world: impl Fn(DVec2) -> DVec3,
    ) -> Self {
        let key = |p: DVec2| [(p.x * 100.0).round() as i64, (p.y * 100.0).round() as i64];
        let mut index = HashMap::new();
        let mut w = Walkway {
            village,
            ..Default::default()
        };
        let mut node = |p: DVec2, w: &mut Walkway| {
            *index.entry(key(p)).or_insert_with(|| {
                w.nodes.push(world(p));
                w.links.push(Vec::new());
                (w.nodes.len() - 1) as u32
            })
        };
        for &(a, b) in trails {
            if a.distance_squared(b) < 1e-6 {
                continue;
            }
            let (i, j) = (node(a, &mut w), node(b, &mut w));
            if !w.links[i as usize].contains(&j) {
                w.links[i as usize].push(j);
                w.links[j as usize].push(i);
            }
        }
        let find = |p: DVec2| index.get(&key(p)).copied();
        for &(d, home) in doors {
            if let Some(n) = find(d) {
                w.doors.push(n);
                w.homes.push(home);
            }
        }
        w.mouth = mouth.and_then(find);
        w
    }
    fn length(&self, a: u32, b: u32) -> f64 {
        (self.nodes[a as usize] - self.nodes[b as usize]).length() * crate::planet::planet_radius_meters()
    }
    fn dwellings(&self) -> Vec<u32> {
        self.doors.iter().zip(&self.homes).filter(|(_, h)| **h).map(|(d, _)| *d).collect()
    }
    fn public_doors(&self) -> Vec<u32> {
        self.doors.iter().zip(&self.homes).filter(|(_, h)| !**h).map(|(d, _)| *d).collect()
    }
    fn nearest(&self, p: DVec3) -> u32 {
        (0..self.nodes.len() as u32)
            .min_by(|&a, &b| {
                self.nodes[a as usize]
                    .distance_squared(p)
                    .total_cmp(&self.nodes[b as usize].distance_squared(p))
            })
            .unwrap_or(0)
    }
    /// Shortest node path from `from` to `to`, inclusive.
    fn path(&self, from: u32, to: u32) -> Option<Vec<u32>> {
        #[derive(PartialEq)]
        struct Q(f64, u32);
        impl Eq for Q {}
        impl PartialOrd for Q {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Q {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.0.total_cmp(&self.0)
            }
        }
        let n = self.nodes.len();
        let mut cost = vec![f64::INFINITY; n];
        let mut parent = vec![u32::MAX; n];
        let mut queue = BinaryHeap::new();
        cost[from as usize] = 0.0;
        queue.push(Q(0.0, from));
        while let Some(Q(c, i)) = queue.pop() {
            if i == to {
                break;
            }
            if c > cost[i as usize] {
                continue;
            }
            for &j in &self.links[i as usize] {
                let next = c + self.length(i, j);
                if next < cost[j as usize] {
                    cost[j as usize] = next;
                    parent[j as usize] = i;
                    queue.push(Q(next, j));
                }
            }
        }
        if !cost[to as usize].is_finite() {
            return None;
        }
        let mut path = vec![to];
        while *path.last()? != from {
            path.push(parent[*path.last()? as usize]);
        }
        path.reverse();
        Some(path)
    }
    fn centre(&self) -> DVec3 {
        self.nodes.iter().sum::<DVec3>().normalize_or_zero()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Doing {
    /// Standing about for this many more seconds.
    Standing(f64),
    /// Walking the plan.
    Walking,
    /// Indoors for this many more seconds (faded out at the door).
    Indoors(f64),
    /// Out driving, or visiting another village: not here at all.
    Away,
}

/// A point on a walk: a footpath node, or a spot on the court floor.
#[derive(Clone, Copy, Debug)]
struct Step {
    dir: DVec3,
    node: Option<u32>,
    floor: Option<f64>,
}

struct Walker {
    person: Person,
    village: String,
    /// Their own front door (None for a visitor).
    home: Option<u32>,
    visitor: bool,
    plan: Vec<Step>,
    leg: usize,
    along: f64,
    /// Footpath node they stand at (None on the court, by their cart).
    at: Option<u32>,
    position: DVec3,
    /// Ground height while off the footpaths (the court floor).
    floor: Option<f64>,
    doing: Doing,
    /// Their cart is due to leave: its door point and floor height.
    cart: Option<(DVec3, f64)>,
    /// Just off their cart at home: head indoors.
    homeward: bool,
    heading: DVec3,
    phase: f64,
    walk: f64,
    look: f64,
    fade: f64,
    ground: Option<f64>,
    seed: u64,
    /// Close behind someone going the same way: slow to keep a gap.
    following: bool,
    /// A visitor who has climbed back on their cart.
    gone: bool,
}
fn unit(n: u64) -> f64 {
    (n >> 11) as f64 / (1u64 << 53) as f64
}
fn rand(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    unit(seed.wrapping_mul(0x9e3779b97f4a7c15) ^ (*seed >> 29))
}

impl Walker {
    fn new(person: Person, village: &str, home: Option<u32>, at: u32, doing: Doing, way: &Walkway) -> Self {
        let mut seed = person.key ^ 0x7a1c;
        Walker {
            person,
            village: village.to_string(),
            home,
            visitor: home.is_none(),
            plan: Vec::new(),
            leg: 0,
            along: 0.0,
            at: Some(at),
            position: way.nodes[at as usize],
            floor: None,
            fade: if matches!(doing, Doing::Indoors(_) | Doing::Away) { 0.0 } else { 1.0 },
            doing,
            cart: None,
            homeward: false,
            heading: DVec3::ZERO,
            phase: rand(&mut seed) * 20.0,
            walk: 0.0,
            look: 0.0,
            ground: None,
            seed,
            following: false,
            gone: false,
        }
    }
    /// Standing by a cart's door on the court.
    fn put_at_cart(&mut self, door: DVec3) {
        self.at = None;
        self.position = door.normalize();
        self.floor = Some(door.length() - crate::planet::planet_radius_meters());
        self.plan.clear();
        self.doing = Doing::Standing(0.3);
        self.fade = 1.0;
    }
}

/// Walkers in view. Households are simulated for the nearest few villages;
/// everything is rebuilt from the walk graphs when they change.
#[derive(Default)]
pub struct Crowd {
    walkways: Vec<Walkway>,
    walkers: Vec<Walker>,
    /// The resident an inspection camera follows, once chosen.
    followed: Option<u64>,
    /// Drivers who reached their carts this frame (for the traffic).
    boarded: Vec<u64>,
    /// Drivers with a parked cart here whom this crowd is walking about.
    managed: HashSet<u64>,
    /// Drivers who reached their carts and are waiting to be driven off
    /// (the traffic may not have stepped yet, e.g. while paused).
    climbing_on: HashSet<u64>,
}

const VILLAGES: usize = 6;
const VILLAGE_RANGE: f64 = 2500.0;
const GROUND_RANGE: f64 = 450.0;
/// Keep left on a footpath, as on the roads, so people pass each other.
const KEEP_LEFT: f64 = 0.35;
/// A visitor stays in until their cart's driver is due back.
const UNTIL_RECALLED: f64 = 1.0e9;

impl Crowd {
    pub fn clear(&mut self) {
        self.walkways.clear();
        self.walkers.clear();
    }
    #[cfg(test)]
    pub fn walkers(&self) -> usize {
        self.walkers.len()
    }
    /// Drivers who reached their carts since the last call.
    pub fn take_boarded(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.boarded)
    }
    pub fn managed(&self) -> HashSet<u64> {
        self.managed.clone()
    }
    /// New footpaths: rehouse every household of the nearest villages, keeping
    /// people already out and about where they stand.
    pub fn set_walkways(&mut self, walkways: Vec<Walkway>, eye: DVec3) {
        let up = eye.normalize_or_zero();
        let mut order: Vec<_> = walkways
            .iter()
            .enumerate()
            .filter(|(_, w)| w.homes.iter().any(|&h| h) && w.nodes.len() >= 3)
            .map(|(i, w)| (w.centre().distance(up) * crate::planet::planet_radius_meters(), i))
            .filter(|&(d, _)| d < VILLAGE_RANGE)
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        order.truncate(VILLAGES);
        let mut old: HashMap<u64, Walker> = self.walkers.drain(..).map(|w| (w.person.key, w)).collect();
        let keep = |mut w: Walker, way: &Walkway, home: Option<u32>| {
            w.home = home;
            if w.at.is_some() || w.floor.is_none() {
                let at = way.nearest(w.position);
                w.at = Some(at);
                w.position = way.nodes[at as usize];
                w.floor = None;
            }
            w.plan.clear();
            w.leg = 0;
            w.along = 0.0;
            if w.doing == Doing::Walking {
                w.doing = Doing::Standing(1.0);
            }
            w
        };
        for &(_, i) in &order {
            let w = &walkways[i];
            let homes = w.dwellings();
            for r in 0..people::RESIDENTS {
                let key = people::resident_key(&w.village, r);
                let home = homes[r as usize % homes.len()];
                let walker = match old.remove(&key) {
                    Some(kept) => keep(kept, w, Some(home)),
                    None => {
                        let person = Person::from_key(key);
                        let mut seed = key ^ 0x51ab;
                        let (at, doing) = if rand(&mut seed) < 0.55 {
                            (home, Doing::Indoors(5.0 + rand(&mut seed) * 85.0))
                        } else {
                            let at = (rand(&mut seed) * w.nodes.len() as f64) as u32 % w.nodes.len() as u32;
                            (at, Doing::Standing(rand(&mut seed) * 6.0))
                        };
                        Walker::new(person, &w.village, Some(home), at, doing, w)
                    }
                };
                self.walkers.push(walker);
            }
            for (h, &home) in homes.iter().enumerate() {
                for c in 0..people::children_in(&w.village, h as u32) {
                    let key = people::child_key(&w.village, h as u32, c);
                    let walker = match old.remove(&key) {
                        Some(kept) => keep(kept, w, Some(home)),
                        None => {
                            let person = Person::child_from_key(key);
                            let mut seed = key ^ 0x51ab;
                            let (at, doing) = if rand(&mut seed) < 0.3 {
                                (home, Doing::Indoors(5.0 + rand(&mut seed) * 35.0))
                            } else {
                                let near = w.links[home as usize].first().copied().unwrap_or(home);
                                (near, Doing::Standing(rand(&mut seed) * 4.0))
                            };
                            Walker::new(person, &w.village, Some(home), at, doing, w)
                        }
                    };
                    self.walkers.push(walker);
                }
            }
            // Visitors already here stay (their cart is still in the court).
            let visitors: Vec<u64> = old
                .iter()
                .filter(|(_, v)| v.visitor && v.village == w.village)
                .map(|(&k, _)| k)
                .collect();
            for k in visitors {
                let v = old.remove(&k).unwrap();
                self.walkers.push(keep(v, w, None));
            }
        }
        tracing::info!(target:"planet::villages", walkways=walkways.len(), nearby=order.len(), walkers=self.walkers.len(), children=self.walkers.iter().filter(|w| w.person.is_child()).count(), "villagers placed in their homes");
        self.walkways = walkways;
    }

    /// Matches the crowd to the carts: who is away driving, who has just got
    /// down from a cart in this court, whose cart is due to leave, and which
    /// visitors have arrived.
    pub fn sync(&mut self, parked: &[ParkedCart], away: &HashSet<u64>) {
        let radius = crate::planet::planet_radius_meters();
        let by_driver: HashMap<u64, &ParkedCart> = parked.iter().map(|c| (c.driver, c)).collect();
        let ways: HashMap<&str, usize> =
            self.walkways.iter().enumerate().map(|(i, w)| (w.village.as_str(), i)).collect();
        // Only villages with households here.
        let living: HashSet<String> = self.walkers.iter().map(|w| w.village.clone()).collect();
        self.managed.clear();
        self.climbing_on.retain(|d| by_driver.contains_key(d));
        for w in &mut self.walkers {
            let Some(&wi) = ways.get(w.village.as_str()) else {
                continue;
            };
            let way = &self.walkways[wi];
            let key = w.person.key;
            let cart = by_driver.get(&key).filter(|c| c.court == w.village);
            if w.visitor {
                // Gone once their cart is no longer standing here.
                if cart.is_none() {
                    w.gone = true;
                    continue;
                }
            } else {
                let elsewhere = by_driver.get(&key).is_some_and(|c| c.court != w.village);
                if away.contains(&key) || elsewhere {
                    if w.doing != Doing::Away {
                        w.doing = Doing::Away;
                        w.plan.clear();
                        w.cart = None;
                        w.fade = 0.0;
                    }
                    continue;
                }
                if self.climbing_on.contains(&key) {
                    continue;
                }
                if w.doing == Doing::Away {
                    // Back home: off the cart that just parked, or (if it
                    // came in unseen) already indoors.
                    match cart {
                        Some(c) if c.since < 3.0 && way.mouth.is_some() => {
                            w.put_at_cart(c.door);
                            w.homeward = true;
                        }
                        _ => {
                            let home = w.home.unwrap_or(0);
                            w.at = Some(home);
                            w.position = way.nodes[home as usize];
                            w.floor = None;
                            w.doing = Doing::Indoors(10.0 + rand(&mut w.seed) * 50.0);
                        }
                    }
                }
            }
            match cart {
                Some(c) if way.mouth.is_some() => {
                    self.managed.insert(key);
                    if c.recalled && w.cart.is_none() {
                        w.cart = Some((c.door.normalize(), c.door.length() - radius));
                        if let Doing::Indoors(_) = w.doing {
                            w.doing = Doing::Standing(0.6);
                        }
                    }
                }
                _ => w.cart = None,
            }
        }
        self.walkers.retain(|w| !w.gone);
        // Visitors who have just driven in, or were here before we were.
        let present: HashSet<u64> = self.walkers.iter().map(|w| w.person.key).collect();
        for c in parked {
            if c.home.as_deref() == Some(c.court.as_str())
                || present.contains(&c.driver)
                || self.climbing_on.contains(&c.driver)
                || !living.contains(&c.court)
            {
                continue;
            }
            let Some(&wi) = ways.get(c.court.as_str()) else {
                continue;
            };
            let way = &self.walkways[wi];
            let Some(mouth) = way.mouth else {
                continue;
            };
            let person = Person::from_key(c.driver);
            let mut v = if c.since < 3.0 {
                let mut v = Walker::new(person, &c.court, None, mouth, Doing::Standing(0.3), way);
                v.put_at_cart(c.door);
                v
            } else {
                let mut seed = c.driver;
                let doors = way.public_doors();
                let doors = if doors.is_empty() { way.doors.clone() } else { doors };
                let at = doors[(rand(&mut seed) * doors.len() as f64) as usize % doors.len()];
                Walker::new(person, &c.court, None, at, Doing::Indoors(UNTIL_RECALLED), way)
            };
            if c.recalled {
                v.cart = Some((c.door.normalize(), c.door.length() - radius));
                if let Doing::Indoors(_) = v.doing {
                    v.doing = Doing::Standing(0.6);
                }
            }
            self.managed.insert(c.driver);
            self.walkers.push(v);
        }
    }

    /// Camera for watching one villager: ahead of them and to one side, a
    /// little above head height, so their walk, clothes and face all show.
    pub fn inspection_pose(&mut self, eye: DVec3, children: bool) -> Option<(DVec3, DVec3)> {
        let radius = crate::planet::planet_radius_meters();
        // Keep following them while they are on the move (or just stopping).
        let known = self.followed.filter(|k| {
            self.walkers
                .iter()
                .any(|w| w.person.key == *k && (w.doing == Doing::Walking || w.walk > 0.02))
        });
        let key = known.or_else(|| {
            self.walkers
                .iter()
                .filter(|w| w.person.is_child() == children)
                .filter(|w| w.ground.is_some() && w.fade > 0.5 && !matches!(w.doing, Doing::Indoors(_) | Doing::Away))
                .min_by(|a, b| {
                    // Someone already on the move, then the nearest.
                    let d = |w: &Walker| {
                        w.position.distance(eye.normalize()) + if w.doing == Doing::Walking { 0.0 } else { 1.0 }
                    };
                    d(a).total_cmp(&d(b))
                })
                .map(|w| (w.person.key, w.doing == Doing::Walking))
                .map(|(key, walking)| {
                    // Commit to someone once they set off.
                    if walking {
                        self.followed = Some(key);
                    }
                    key
                })
        })?;
        let w = self.walkers.iter().find(|w| w.person.key == key)?;
        let way = self.walkways.iter().find(|v| v.village == w.village)?;
        let up = w.position.normalize();
        let feet = up * (radius + w.ground?);
        let forward = facing(w, way);
        let left = up.cross(forward);
        // Ahead of a walker; behind someone standing (who may face a door).
        let ahead = if w.doing == Doing::Walking { 3.6 } else { -3.6 };
        // Children from a little higher, so bumps in the ground between do
        // not hide them.
        let (rise, aim) = if children { (3.2, 0.6) } else { (1.9, 1.0) };
        Some((feet + forward * ahead + left * 2.4 + up * rise, feet + up * aim))
    }

    /// Advances everyone by `dt` scene seconds and writes those near `eye` to
    /// `out`, standing on the ground `height` reports (drawn terrain) or on
    /// the court floor.
    pub fn update(
        &mut self,
        dt: f64,
        eye: DVec3,
        height: impl Fn(DVec3) -> Option<f64>,
        out: &mut Vec<Placement>,
    ) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 2.0) } else { 0.0 };
        let radius = crate::planet::planet_radius_meters();
        let by_name: HashMap<&str, usize> =
            self.walkways.iter().enumerate().map(|(i, w)| (w.village.as_str(), i)).collect();
        // Fall in behind anyone walking the same way just ahead, rather than
        // walking through them (people coming the other way pass on the left).
        let mut walking: HashMap<&str, Vec<(DVec3, DVec3)>> = HashMap::new();
        for w in self.walkers.iter().filter(|w| w.doing == Doing::Walking) {
            walking.entry(w.village.as_str()).or_default().push((w.position, w.heading));
        }
        let behind: Vec<bool> = self
            .walkers
            .iter()
            .map(|w| {
                w.doing == Doing::Walking
                    && walking.get(w.village.as_str()).is_some_and(|others| {
                        others.iter().any(|&(p, h)| {
                            let d = (p - w.position) * radius;
                            let along = d.dot(w.heading);
                            along > 0.05 && along < 1.5 && (d - w.heading * along).length() < 0.7 && h.dot(w.heading) > 0.3
                        })
                    })
            })
            .collect();
        for (walker, &behind) in self.walkers.iter_mut().zip(&behind) {
            walker.following = behind;
            let Some(&wi) = by_name.get(walker.village.as_str()) else {
                continue;
            };
            let way = &self.walkways[wi];
            if walker.doing == Doing::Away {
                continue;
            }
            if step(walker, way, dt) {
                // Reached their cart: on to the bench (the traffic draws that).
                self.boarded.push(walker.person.key);
                self.climbing_on.insert(walker.person.key);
                walker.cart = None;
                if walker.visitor {
                    walker.gone = true;
                } else {
                    walker.doing = Doing::Away;
                    walker.fade = 0.0;
                }
                continue;
            }
            let up = walker.position.normalize();
            // Ground distance: villages stand well above sea level.
            let near = up.distance(eye.normalize()) * radius < GROUND_RANGE;
            if let Some(floor) = walker.floor {
                walker.ground = Some(floor);
            } else if near || walker.ground.is_none() {
                if let Some(h) = height(up) {
                    walker.ground = Some(h);
                }
            }
            let Some(ground) = walker.ground else {
                continue;
            };
            if walker.fade <= 0.0 || !near {
                continue;
            }
            let forward = facing(walker, way);
            let left = up.cross(forward);
            let side = KEEP_LEFT * walker.walk * if walker.floor.is_some() { 0.0 } else { 1.0 };
            // Children at a run lengthen their stride.
            let run = if walker.person.is_child() && walker.person.pace > 1.55 { 1.2 } else { 1.0 };
            out.push(Placement {
                person: walker.person,
                feet: up * (radius + ground) + left * side,
                forward,
                up,
                phase: walker.phase as f32,
                walk: (walker.walk * run) as f32,
                sit: 0.0,
                look: walker.look as f32,
                fade: walker.fade as f32,
                light: None,
            });
        }
        self.walkers.retain(|w| !w.gone);
    }
}

/// Which way a walker faces: their heading, or out along the path when they
/// have not yet moved.
fn facing(w: &Walker, way: &Walkway) -> DVec3 {
    let up = w.position.normalize();
    let mut forward = w.heading - up * w.heading.dot(up);
    if forward.length_squared() < 1e-12 {
        forward = w
            .at
            .and_then(|at| way.links[at as usize].first())
            .map_or(DVec3::ZERO, |&j| way.nodes[j as usize] - w.position);
        forward -= up * forward.dot(up);
    }
    forward.try_normalize().unwrap_or_else(|| up.any_orthonormal_vector())
}

/// Rotates `from` toward `to` about the local vertical by at most `limit`
/// radians (either way round, including a full about-turn).
fn turn_towards(from: DVec3, to: DVec3, up: DVec3, limit: f64) -> DVec3 {
    let up = up.normalize();
    let flat = |v: DVec3| (v - up * v.dot(up)).normalize_or_zero();
    let (f, t) = (flat(from), flat(to));
    if f == DVec3::ZERO {
        return t;
    }
    let angle = f.cross(t).dot(up).atan2(f.dot(t));
    let a = angle.clamp(-limit, limit);
    (f * a.cos() + up.cross(f) * a.sin()).normalize()
}

enum Target {
    Node(u32),
    /// A cart's door point on the court floor.
    Cart(DVec3, f64),
}
/// The walk from where they stand to `target`: footpath nodes, with the
/// court leg (from or to a cart) walked straight across the floor.
fn plan(w: &Walker, way: &Walkway, target: Target) -> Option<Vec<Step>> {
    let node = |n: u32| Step {
        dir: way.nodes[n as usize],
        node: Some(n),
        floor: None,
    };
    let mut steps = Vec::new();
    let start = match w.at {
        Some(n) => n,
        None => {
            steps.push(Step {
                dir: w.position,
                node: None,
                floor: w.floor,
            });
            way.mouth?
        }
    };
    let goal = match target {
        Target::Node(n) => n,
        Target::Cart(..) => way.mouth?,
    };
    steps.extend(way.path(start, goal)?.into_iter().map(node));
    if let Target::Cart(dir, floor) = target {
        steps.push(Step {
            dir,
            node: None,
            floor: Some(floor),
        });
    }
    steps.dedup_by(|b, a| a.dir.distance_squared(b.dir) < 1e-16);
    (steps.len() >= 2).then_some(steps)
}

/// Where to wander next.
fn destination(w: &mut Walker, way: &Walkway) -> u32 {
    let pick = |seed: &mut u64, list: &[u32]| list[(rand(seed) * list.len() as f64) as usize % list.len()];
    let any = |seed: &mut u64| (rand(seed) * way.nodes.len() as f64) as u32 % way.nodes.len() as u32;
    let public = way.public_doors();
    let home = w.home.unwrap_or(0);
    if w.visitor {
        return if public.is_empty() { pick(&mut w.seed, &way.doors) } else { pick(&mut w.seed, &public) };
    }
    if std::mem::take(&mut w.homeward) {
        return home;
    }
    let roll = rand(&mut w.seed);
    if w.person.is_child() {
        // Children play about the lanes, never far from home for long.
        return if roll < 0.3 {
            home
        } else if roll < 0.85 {
            any(&mut w.seed)
        } else {
            pick(&mut w.seed, &way.doors)
        };
    }
    if roll < 0.35 {
        home
    } else if roll < 0.5 && way.mouth.is_some() {
        way.mouth.unwrap()
    } else if roll < 0.75 && !public.is_empty() {
        pick(&mut w.seed, &public)
    } else if roll < 0.9 {
        pick(&mut w.seed, &way.doors)
    } else {
        any(&mut w.seed)
    }
}

/// One walker's step. Returns true when they reach their cart's door.
fn step(w: &mut Walker, way: &Walkway, dt: f64) -> bool {
    let target_walk = if w.doing == Doing::Walking { 1.0 } else { 0.0 };
    w.walk += (target_walk - w.walk).clamp(-dt * 3.0, dt * 3.0);
    match w.doing {
        Doing::Away => {}
        Doing::Indoors(left) => {
            w.fade = (w.fade - dt * 1.5).max(0.0);
            w.doing = if left - dt <= 0.0 {
                Doing::Standing(1.5 + rand(&mut w.seed) * 3.0)
            } else {
                Doing::Indoors(left - dt)
            };
        }
        Doing::Standing(left) => {
            w.fade = (w.fade + dt * 1.5).min(1.0);
            w.phase += dt;
            // Now and then glance about.
            let target = ((w.phase * 0.21 + unit(w.person.key) * 9.0).sin() * 0.7) * 0.8;
            w.look += (target - w.look) * (dt * 1.5).min(1.0);
            if left - dt > 0.0 {
                w.doing = Doing::Standing(left - dt);
                return false;
            }
            let target = match w.cart {
                Some((dir, floor)) => Target::Cart(dir, floor),
                None => Target::Node(destination(w, way)),
            };
            match plan(w, way, target) {
                Some(plan) => {
                    w.plan = plan;
                    w.leg = 0;
                    w.along = 0.0;
                    w.doing = Doing::Walking;
                }
                None => w.doing = Doing::Standing(3.0 + rand(&mut w.seed) * 6.0),
            }
        }
        Doing::Walking => {
            w.fade = (w.fade + dt * 1.5).min(1.0);
            w.look *= (1.0 - dt * 2.0).max(0.0);
            let radius = crate::planet::planet_radius_meters();
            let speed = f64::from(w.person.pace) * if w.following { 0.4 } else { 1.0 };
            let mut remaining = speed * dt;
            w.phase += people::stride_phase(&w.person, remaining);
            let leg_length = |a: &Step, b: &Step| (a.dir.distance(b.dir) * radius).max(1e-3);
            while remaining > 0.0 && w.leg + 1 < w.plan.len() {
                let length = leg_length(&w.plan[w.leg], &w.plan[w.leg + 1]);
                let step = remaining.min(length - w.along);
                w.along += step;
                remaining -= step;
                if w.along >= length - 1e-9 {
                    w.leg += 1;
                    w.along = 0.0;
                }
            }
            if w.leg + 1 >= w.plan.len() {
                let last = *w.plan.last().unwrap();
                w.at = last.node;
                w.position = last.dir;
                w.floor = last.floor;
                if w.at.is_none() && w.cart.is_some() {
                    return true;
                }
                let at = w.at.unwrap_or(0);
                let is_door = way.doors.contains(&at);
                let child = w.person.is_child();
                let r = rand(&mut w.seed);
                w.doing = if w.visitor && is_door {
                    Doing::Indoors(UNTIL_RECALLED)
                } else if w.home == Some(at) && r < if child { 0.35 } else { 0.75 } {
                    Doing::Indoors(if child { 10.0 + rand(&mut w.seed) * 30.0 } else { 40.0 + rand(&mut w.seed) * 160.0 })
                } else if is_door && r < if child { 0.15 } else { 0.45 } {
                    Doing::Indoors(10.0 + rand(&mut w.seed) * 50.0)
                } else if child {
                    Doing::Standing(1.0 + rand(&mut w.seed) * 5.0)
                } else {
                    Doing::Standing(3.0 + rand(&mut w.seed) * 15.0)
                };
                return false;
            }
            let (a, b) = (w.plan[w.leg], w.plan[w.leg + 1]);
            let t = w.along / leg_length(&a, &b);
            w.position = a.dir.lerp(b.dir, t).normalize();
            // On the court floor for the leg to or from a cart.
            w.floor = a.floor.or(b.floor);
            if w.floor.is_some() {
                w.at = None;
            }
            // Turn onto each new leg at a walker's pace rather than snapping;
            // a fresh start (barely moving yet) just faces the way.
            let target = (b.dir - a.dir).normalize();
            w.heading = if w.heading == DVec3::ZERO || w.walk < 0.3 {
                target
            } else {
                turn_towards(w.heading, target, w.position, dt * 5.0)
            };
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> crate::road_network_field::Frame {
        crate::road_network_field::Frame::new(DVec3::Y, crate::planet::planet_radius_meters()).unwrap()
    }
    fn village() -> Walkway {
        // A lane from the court mouth with three doors off it (two homes, an inn).
        let p = |x: f64, y: f64| DVec2::new(x, y);
        let trails = [
            (p(0.0, 0.0), p(20.0, 0.0)),
            (p(20.0, 0.0), p(40.0, 0.0)),
            (p(20.0, 0.0), p(20.0, 8.0)),
            (p(40.0, 0.0), p(40.0, 6.0)),
            (p(0.0, 0.0), p(-3.0, 5.0)),
        ];
        let f = frame();
        Walkway::from_trails(
            "TESTHAM".into(),
            &trails,
            &[(p(20.0, 8.0), true), (p(40.0, 6.0), false), (p(-3.0, 5.0), true)],
            Some(p(40.0, 0.0)),
            |q| f.world(q.extend(0.0)).normalize(),
        )
    }
    fn eye(w: &Walkway) -> DVec3 {
        w.centre() * (crate::planet::planet_radius_meters() + 2.0)
    }
    #[test]
    fn walk_graph_joins_shared_points_and_routes_door_to_door() {
        let w = village();
        assert_eq!(w.nodes.len(), 6);
        assert_eq!(w.doors.len(), 3);
        assert_eq!(w.dwellings().len(), 2);
        assert_eq!(w.public_doors().len(), 1);
        let path = w.path(w.doors[0], w.doors[2]).unwrap();
        assert_eq!(path.len(), 4); // door, junction, start, door
        let length: f64 = path.windows(2).map(|p| w.length(p[0], p[1])).sum();
        assert!((length - (8.0 + 20.0 + 34f64.sqrt())).abs() < 0.01, "{length}");
    }
    #[test]
    fn households_fill_the_homes_children_play_and_nobody_jumps() {
        let w = village();
        let eye = eye(&w);
        let mut crowd = Crowd::default();
        crowd.set_walkways(vec![w.clone()], eye);
        let children: usize = (0..2).map(|h| people::children_in("TESTHAM", h) as usize).sum();
        assert_eq!(crowd.walkers(), people::RESIDENTS as usize + children);
        // Every resident lives in a dwelling, not the inn.
        let homes = w.dwellings();
        assert!(crowd.walkers.iter().all(|v| v.home.is_some_and(|h| homes.contains(&h))));
        let (mut walked, mut indoors, mut seen) = (0.0, false, 0);
        let mut last: HashMap<u64, DVec3> = HashMap::new();
        for _ in 0..6000 {
            let mut out = Vec::new();
            crowd.sync(&[], &HashSet::new());
            crowd.update(0.05, eye, |_| Some(10.0), &mut out);
            seen += out.len();
            for p in &out {
                assert!(p.feet.is_finite() && p.forward.is_finite());
                assert!((p.feet.length() - crate::planet::planet_radius_meters() - 10.0).abs() < 0.01);
                assert!(p.forward.dot(p.up).abs() < 1e-6);
                if let Some(q) = last.insert(p.person.key, p.feet) {
                    let moved = q.distance(p.feet);
                    // Never faster than a child's run, plus the keep-left
                    // sidestep swinging round a corner.
                    assert!(moved < 0.05 * 1.95 + 0.12, "{moved}");
                    walked += moved;
                }
            }
            indoors |= crowd.walkers.iter().any(|v| matches!(v.doing, Doing::Indoors(_)));
        }
        assert!(walked > 300.0, "{walked}");
        assert!(indoors && seen > 0);
    }
    #[test]
    fn owners_leave_on_their_carts_and_visitors_come_and_go() {
        let w = village();
        let eye = eye(&w);
        let f = frame();
        let radius = crate::planet::planet_radius_meters();
        // A cart door on the court floor a few metres past the mouth.
        let door = f.world(DVec3::new(46.0, -3.0, 0.0)).normalize() * (radius + 10.0);
        let mut crowd = Crowd::default();
        crowd.set_walkways(vec![w.clone()], eye);
        let owner = people::resident_key("TESTHAM", 3);
        let guest = people::resident_key("ELSEWHERE", 1);
        let cart = |driver: u64, home: &str, since: f64, recalled: bool| ParkedCart {
            driver,
            court: "TESTHAM".into(),
            home: Some(home.into()),
            door,
            since,
            recalled,
        };
        // The owner is away driving; then their cart parks and they get down.
        let mut away: HashSet<u64> = [owner].into();
        crowd.sync(&[], &away);
        assert!(crowd.walkers.iter().any(|v| v.person.key == owner && v.doing == Doing::Away));
        away.clear();
        let parked = [cart(owner, "TESTHAM", 0.1, false), cart(guest, "ELSEWHERE", 0.1, false)];
        crowd.sync(&parked, &away);
        let at_cart = |c: &Crowd, k: u64| {
            c.walkers.iter().find(|v| v.person.key == k).is_some_and(|v| v.at.is_none() && v.floor.is_some())
        };
        assert!(at_cart(&crowd, owner) && at_cart(&crowd, guest));
        assert!(crowd.managed().contains(&owner) && crowd.managed().contains(&guest));
        // They walk off (owner home, guest to the inn), then both are recalled.
        for _ in 0..1200 {
            crowd.sync(&parked, &away);
            crowd.update(0.05, eye, |_| Some(10.0), &mut Vec::new());
        }
        let guest_walker = crowd.walkers.iter().find(|v| v.person.key == guest).unwrap();
        assert_eq!(guest_walker.at, w.public_doors().first().copied());
        let recalled = [cart(owner, "TESTHAM", 60.0, true), cart(guest, "ELSEWHERE", 60.0, true)];
        let mut boarded = Vec::new();
        for _ in 0..2400 {
            let still: Vec<_> = recalled.iter().filter(|c| !boarded.contains(&c.driver)).cloned().collect();
            crowd.sync(&still, &away);
            crowd.update(0.05, eye, |_| Some(10.0), &mut Vec::new());
            boarded.extend(crowd.take_boarded());
            if boarded.len() == 2 {
                break;
            }
        }
        assert!(boarded.contains(&owner) && boarded.contains(&guest), "{boarded:?}");
        // The guest has gone; the owner is away (on their cart).
        assert!(!crowd.walkers.iter().any(|v| v.person.key == guest));
        crowd.sync(&[], &[owner].into());
        assert!(crowd.walkers.iter().any(|v| v.person.key == owner && v.doing == Doing::Away));
    }
}

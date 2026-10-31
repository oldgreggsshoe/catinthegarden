//! Villagers on foot: residents walk the worn desire paths between their
//! doors and the court, stop to stand about, and go indoors for a while.
//!
//! The walk graph is the footpath network the village path worker already
//! routes (door connectors, shared lanes, the court mouth), so people follow
//! the dirt they wore. Heights come from the drawn terrain each frame for the
//! few people near the camera. A resident who is away driving a cart is not
//! also on the footpaths.
use crate::people::{self, Person, Placement};
use glam::DVec3;
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
    /// Where the path meets the parking court, if it has one.
    pub mouth: Option<u32>,
}

impl Walkway {
    /// Builds the graph from trail segments in a local frame (2D metres),
    /// projected to unit directions by `world`.
    pub fn from_trails(
        village: String,
        trails: &[(glam::DVec2, glam::DVec2)],
        doors: &[glam::DVec2],
        mouth: Option<glam::DVec2>,
        world: impl Fn(glam::DVec2) -> DVec3,
    ) -> Self {
        let key = |p: glam::DVec2| [(p.x * 100.0).round() as i64, (p.y * 100.0).round() as i64];
        let mut index = HashMap::new();
        let mut w = Walkway {
            village,
            ..Default::default()
        };
        let mut node = |p: glam::DVec2, w: &mut Walkway| {
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
        let find = |p: glam::DVec2| index.get(&key(p)).copied();
        w.doors = doors.iter().filter_map(|&d| find(d)).collect();
        w.mouth = mouth.and_then(find);
        w
    }
    fn length(&self, a: u32, b: u32) -> f64 {
        (self.nodes[a as usize] - self.nodes[b as usize]).length() * crate::planet::planet_radius_meters()
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

#[derive(Clone, Copy, PartialEq)]
enum Doing {
    /// Standing about for this many more seconds.
    Standing(f64),
    /// Walking the plan.
    Walking,
    /// Indoors for this many more seconds (faded out at the door).
    Indoors(f64),
}

struct Walker {
    person: Person,
    village: String,
    /// Node index of where the plan starts, and the plan itself.
    plan: Vec<u32>,
    leg: usize,
    along: f64,
    at: u32,
    doing: Doing,
    position: DVec3,
    heading: DVec3,
    phase: f64,
    walk: f64,
    look: f64,
    fade: f64,
    ground: Option<f64>,
    seed: u64,
    /// Close behind someone going the same way: slow to keep a gap.
    following: bool,
}
fn unit(n: u64) -> f64 {
    (n >> 11) as f64 / (1u64 << 53) as f64
}
fn rand(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    unit(seed.wrapping_mul(0x9e3779b97f4a7c15) ^ (*seed >> 29))
}

/// Walkers in view. Village populations are kept only for the nearest few
/// villages; everything is rebuilt from the walk graphs when they change.
#[derive(Default)]
pub struct Crowd {
    walkways: Vec<Walkway>,
    walkers: Vec<Walker>,
    /// The resident an inspection camera follows, once chosen.
    followed: Option<u64>,
}

const VILLAGES: usize = 8;
const VILLAGE_RANGE: f64 = 2500.0;
const GROUND_RANGE: f64 = 450.0;
/// Keep left on a footpath, as on the roads, so people pass each other.
const KEEP_LEFT: f64 = 0.35;

impl Crowd {
    pub fn clear(&mut self) {
        self.walkways.clear();
        self.walkers.clear();
    }
    /// Camera for watching one villager: ahead of them and to one side, a
    /// little above head height, so their walk, clothes and face all show.
    pub fn inspection_pose(&mut self, eye: DVec3) -> Option<(DVec3, DVec3)> {
        let radius = crate::planet::planet_radius_meters();
        // Keep following them unless they have gone indoors.
        let known = self
            .followed
            .filter(|k| self.walkers.iter().any(|w| w.person.key == *k && !matches!(w.doing, Doing::Indoors(_))));
        let key = known.or_else(|| {
            self.walkers
                .iter()
                .filter(|w| w.ground.is_some() && w.fade > 0.5 && !matches!(w.doing, Doing::Indoors(_)))
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
        Some((feet + forward * 3.6 + left * 2.4 + up * 1.9, feet + up * 1.0))
    }
    #[cfg(test)]
    pub fn walkers(&self) -> usize {
        self.walkers.len()
    }
    /// New footpaths: repopulate, keeping residents already out and about
    /// where they stand (snapped to the nearest node of the new graph).
    pub fn set_walkways(&mut self, walkways: Vec<Walkway>, eye: DVec3, away: &HashSet<u64>) {
        let up = eye.normalize_or_zero();
        let mut order: Vec<_> = walkways
            .iter()
            .enumerate()
            .filter(|(_, w)| w.doors.len() >= 2 && w.nodes.len() >= 3)
            .map(|(i, w)| (w.centre().distance(up) * crate::planet::planet_radius_meters(), i))
            .filter(|&(d, _)| d < VILLAGE_RANGE)
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        order.truncate(VILLAGES);
        let mut old: HashMap<u64, Walker> = self.walkers.drain(..).map(|w| (w.person.key, w)).collect();
        for &(_, i) in &order {
            let w = &walkways[i];
            let count = (w.doors.len() / 2).clamp(2, 9);
            let mut taken = 0;
            for r in 0..people::RESIDENTS {
                if taken >= count {
                    break;
                }
                let key = people::resident_key(&w.village, r);
                if away.contains(&key) {
                    continue;
                }
                taken += 1;
                let mut seed = key ^ 0x7a1c;
                let walker = if let Some(mut kept) = old.remove(&key) {
                    let p = kept.position;
                    let at = (0..w.nodes.len() as u32)
                        .min_by(|&a, &b| w.nodes[a as usize].distance_squared(p).total_cmp(&w.nodes[b as usize].distance_squared(p)))
                        .unwrap_or(0);
                    kept.at = at;
                    kept.plan = vec![at];
                    kept.leg = 0;
                    kept.along = 0.0;
                    kept.position = w.nodes[at as usize];
                    if kept.doing == Doing::Walking {
                        kept.doing = Doing::Standing(1.0);
                    }
                    kept
                } else {
                    let home = w.doors[r as usize % w.doors.len()];
                    let at = if rand(&mut seed) < 0.5 {
                        home
                    } else {
                        (rand(&mut seed) * w.nodes.len() as f64) as u32 % w.nodes.len() as u32
                    };
                    let indoors = at == home && rand(&mut seed) < 0.5;
                    Walker {
                        person: Person::from_key(key),
                        village: w.village.clone(),
                        plan: vec![at],
                        leg: 0,
                        along: 0.0,
                        at,
                        doing: if indoors {
                            Doing::Indoors(rand(&mut seed) * 30.0)
                        } else {
                            Doing::Standing(rand(&mut seed) * 6.0)
                        },
                        position: w.nodes[at as usize],
                        heading: DVec3::ZERO,
                        phase: rand(&mut seed) * 20.0,
                        walk: 0.0,
                        look: 0.0,
                        fade: if indoors { 0.0 } else { 1.0 },
                        ground: None,
                        seed,
                        following: false,
                    }
                };
                self.walkers.push(walker);
            }
        }
        tracing::info!(target:"planet::villages", walkways=walkways.len(), with_doors=walkways.iter().filter(|w| w.doors.len() >= 2).count(), nearby=order.len(), walkers=self.walkers.len(), "villagers placed on the footpaths");
        self.walkways = walkways;
    }

    /// Advances everyone by `dt` scene seconds and writes those near `eye` to
    /// `out`, standing on the ground `height` reports (drawn terrain).
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
        let ahead: Vec<bool> = self
            .walkers
            .iter()
            .map(|w| {
                w.doing == Doing::Walking
                    && self.walkers.iter().any(|o| {
                        if std::ptr::eq(o, w) || o.doing != Doing::Walking || o.village != w.village {
                            return false;
                        }
                        let d = (o.position - w.position) * radius;
                        let along = d.dot(w.heading);
                        along > 0.0
                            && along < 1.5
                            && (d - w.heading * along).length() < 0.7
                            && o.heading.dot(w.heading) > 0.3
                    })
            })
            .collect();
        for (walker, &behind) in self.walkers.iter_mut().zip(&ahead) {
            walker.following = behind;
            let Some(&wi) = by_name.get(walker.village.as_str()) else {
                continue;
            };
            let way = &self.walkways[wi];
            step(walker, way, dt);
            let up = walker.position.normalize();
            // Ground distance: villages stand well above sea level.
            let near = up.distance(eye.normalize()) * radius < GROUND_RANGE;
            if near || walker.ground.is_none() {
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
            // Walking keeps to the left of the path's centre line.
            let left = up.cross(forward);
            let side = KEEP_LEFT * walker.walk;
            out.push(Placement {
                person: walker.person,
                feet: up * (radius + ground) + left * side,
                forward,
                up,
                phase: walker.phase as f32,
                walk: walker.walk as f32,
                sit: 0.0,
                look: walker.look as f32,
                fade: walker.fade as f32,
                light: None,
            });
        }
    }
}

/// Which way a walker faces: their heading, or out along the path when they
/// have not yet moved.
fn facing(w: &Walker, way: &Walkway) -> DVec3 {
    let up = w.position.normalize();
    let mut forward = w.heading - up * w.heading.dot(up);
    if forward.length_squared() < 1e-12 {
        forward = way.links[w.at as usize]
            .first()
            .map_or(DVec3::ZERO, |&j| way.nodes[j as usize] - w.position);
        forward -= up * forward.dot(up);
    }
    forward.try_normalize().unwrap_or_else(|| up.any_orthonormal_vector())
}

fn step(w: &mut Walker, way: &Walkway, dt: f64) {
    let target_walk = if w.doing == Doing::Walking { 1.0 } else { 0.0 };
    w.walk += (target_walk - w.walk).clamp(-dt * 3.0, dt * 3.0);
    match w.doing {
        Doing::Indoors(left) => {
            w.fade = (w.fade - dt * 1.5).max(0.0);
            if left - dt <= 0.0 {
                w.doing = Doing::Standing(1.5 + rand(&mut w.seed) * 3.0);
            } else {
                w.doing = Doing::Indoors(left - dt);
            }
            return;
        }
        Doing::Standing(left) => {
            w.fade = (w.fade + dt * 1.5).min(1.0);
            w.phase += dt;
            // Now and then glance about.
            let target = ((w.phase * 0.21 + unit(w.person.key) * 9.0).sin() * 0.7) * 0.8;
            w.look += (target - w.look) * (dt * 1.5).min(1.0);
            if left - dt > 0.0 {
                w.doing = Doing::Standing(left - dt);
                return;
            }
            // Choose where to go next: home, the court, a neighbour, anywhere.
            let home = way.doors[(w.person.key % people::RESIDENTS as u64) as usize % way.doors.len()];
            let roll = rand(&mut w.seed);
            let goal = if roll < 0.3 {
                home
            } else if roll < 0.55 && way.mouth.is_some() {
                way.mouth.unwrap()
            } else if roll < 0.85 {
                way.doors[(rand(&mut w.seed) * way.doors.len() as f64) as usize % way.doors.len()]
            } else {
                (rand(&mut w.seed) * way.nodes.len() as f64) as u32 % way.nodes.len() as u32
            };
            match way.path(w.at, goal).filter(|p| p.len() >= 2) {
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
            let speed = f64::from(w.person.pace) * if w.following { 0.4 } else { 1.0 };
            let mut remaining = speed * dt;
            w.phase += people::stride_phase(&w.person, remaining);
            while remaining > 0.0 && w.leg + 1 < w.plan.len() {
                let (a, b) = (w.plan[w.leg], w.plan[w.leg + 1]);
                let length = way.length(a, b).max(1e-3);
                let step = remaining.min(length - w.along);
                w.along += step;
                remaining -= step;
                if w.along >= length - 1e-9 {
                    w.leg += 1;
                    w.along = 0.0;
                    w.at = b;
                }
            }
            if w.leg + 1 >= w.plan.len() {
                w.at = *w.plan.last().unwrap();
                w.position = way.nodes[w.at as usize];
                let is_door = way.doors.contains(&w.at);
                w.doing = if is_door && rand(&mut w.seed) < 0.6 {
                    Doing::Indoors(10.0 + rand(&mut w.seed) * 40.0)
                } else {
                    Doing::Standing(3.0 + rand(&mut w.seed) * 15.0)
                };
                return;
            }
            let (a, b) = (way.nodes[w.plan[w.leg] as usize], way.nodes[w.plan[w.leg + 1] as usize]);
            let t = w.along / way.length(w.plan[w.leg], w.plan[w.leg + 1]).max(1e-3);
            w.position = a.lerp(b, t).normalize();
            let heading = b - a;
            // Turn onto each new leg at a walker's pace rather than snapping;
            // a fresh start (barely moving yet) just faces the way.
            let target = heading.normalize();
            w.heading = if w.heading == DVec3::ZERO || w.walk < 0.3 {
                target
            } else {
                turn_towards(w.heading, target, w.position, dt * 5.0)
            };
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    fn village() -> Walkway {
        // A lane from the court mouth with three doors off it.
        let p = |x: f64, y: f64| DVec2::new(x, y);
        let trails = [
            (p(0.0, 0.0), p(20.0, 0.0)),
            (p(20.0, 0.0), p(40.0, 0.0)),
            (p(20.0, 0.0), p(20.0, 8.0)),
            (p(40.0, 0.0), p(40.0, 6.0)),
            (p(0.0, 0.0), p(-3.0, 5.0)),
        ];
        let frame = crate::road_network_field::Frame::new(DVec3::Y, crate::planet::planet_radius_meters()).unwrap();
        Walkway::from_trails(
            "TESTHAM".into(),
            &trails,
            &[p(20.0, 8.0), p(40.0, 6.0), p(-3.0, 5.0)],
            Some(p(40.0, 0.0)),
            |q| frame.world(q.extend(0.0)).normalize(),
        )
    }
    #[test]
    fn walk_graph_joins_shared_points_and_routes_door_to_door() {
        let w = village();
        assert_eq!(w.nodes.len(), 6);
        assert_eq!(w.doors.len(), 3);
        let path = w.path(w.doors[0], w.doors[2]).unwrap();
        assert_eq!(path.len(), 4); // door, junction, start, door
        let length: f64 = path.windows(2).map(|p| w.length(p[0], p[1])).sum();
        assert!((length - (8.0 + 20.0 + 34f64.sqrt())).abs() < 0.01, "{length}");
    }
    #[test]
    fn residents_walk_the_paths_go_indoors_and_skip_those_away_driving() {
        let w = village();
        let up = w.centre();
        let eye = up * (crate::planet::planet_radius_meters() + 2.0);
        let away: HashSet<_> = [people::resident_key("TESTHAM", 0)].into();
        let mut crowd = Crowd::default();
        crowd.set_walkways(vec![w.clone()], eye, &away);
        assert_eq!(crowd.walkers(), 2);
        assert!(crowd.walkers.iter().all(|v| !away.contains(&v.person.key)));
        let (mut walked, mut indoors, mut seen) = (0.0, false, 0);
        let mut last: HashMap<u64, DVec3> = HashMap::new();
        for _ in 0..6000 {
            let mut out = Vec::new();
            crowd.update(0.05, eye, |_| Some(10.0), &mut out);
            seen += out.len();
            for p in &out {
                assert!(p.feet.is_finite() && p.forward.is_finite());
                assert!((p.feet.length() - crate::planet::planet_radius_meters() - 10.0).abs() < 0.01);
                assert!(p.forward.dot(p.up).abs() < 1e-6);
                if let Some(q) = last.insert(p.person.key, p.feet) {
                    let moved = q.distance(p.feet);
                    // Never faster than a brisk walk, plus the keep-left
                    // sidestep swinging round a corner.
                    assert!(moved < 0.05 * 1.6 + 0.12, "{moved}");
                    walked += moved;
                }
            }
            indoors |= crowd.walkers.iter().any(|v| matches!(v.doing, Doing::Indoors(_)));
        }
        assert!(walked > 100.0, "{walked}");
        assert!(indoors && seen > 0);
    }
}

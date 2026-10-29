//! One bounded worker owns source I/O, planetary topology and regional plans.
//! The render thread only replaces a latest-request slot and takes ready meshes.
use crate::{
    road_network::{self, Ground, Survey},
    road_network_mesh::{self, Prepared, Snapshot},
    road_network_survey::Surface,
};
use glam::DVec3;
use planet_coretypes::{TileKey, tile_key_for_direction};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

const CACHE_REGIONS: usize = 32;
#[derive(Clone, Copy)]
struct Request {
    centre: DVec3,
    generation: u64,
}
struct Shared {
    request: Mutex<Option<Request>>,
    wake: Condvar,
    result: Mutex<Option<(u64, Result<Snapshot, String>)>>,
    signs: Mutex<Option<(u64, crate::road_signs::Mesh)>>,
    generation: AtomicU64,
    stop: AtomicBool,
}
pub struct Worker {
    shared: Arc<Shared>,
    last_centre: DVec3,
    pub source_version: u64,
}
struct Engineering<'a> {
    source: &'a Surface,
    shared: &'a Shared,
    generation: u64,
}
impl Survey for Engineering<'_> {
    fn cancelled(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
            || self.shared.generation.load(Ordering::Relaxed) != self.generation
    }
    fn settlement_core_at(&self, d: DVec3) -> Result<bool, String> {
        if self.cancelled() {
            return Err("road generation superseded".into());
        }
        self.source.settlement_core_at(d)
    }
    fn village_access_height(&self, g: Ground) -> f64 {
        g.height + 0.25
    }
    fn ground(&self, d: DVec3) -> Result<Ground, String> {
        if self.cancelled() {
            return Err("road generation superseded".into());
        }
        self.source.engineering_ground(d)
    }
    fn villages(&self, key: TileKey) -> Result<Vec<DVec3>, String> {
        if self.cancelled() {
            return Err("road generation superseded".into());
        }
        self.source.villages(key)
    }
}

/// Projection samples include diagonals and cross cube edges/corners without
/// clamping to one face. Only these nearby regions become resident on the GPU.
pub fn nearby_keys(centre: DVec3) -> Vec<TileKey> {
    let key = tile_key_for_direction(centre, road_network::REGION_LEVEL);
    let mut keys = std::collections::BTreeSet::new();
    for y in -1..=1 {
        for x in -1..=1 {
            keys.insert(tile_key_for_direction(
                road_network::direction(key, 0.5 + x as f64, 0.5 + y as f64),
                key.level,
            ));
        }
    }
    let mut keys: Vec<_> = keys.into_iter().collect();
    keys.sort_by(|a, b| {
        road_network::direction(*b, 0.5, 0.5)
            .dot(centre.normalize())
            .total_cmp(&road_network::direction(*a, 0.5, 0.5).dot(centre.normalize()))
            .then(a.cmp(b))
    });
    keys
}

impl Worker {
    pub fn start(root: PathBuf, initial: DVec3) -> Result<(Self, Snapshot), String> {
        let shared = Arc::new(Shared {
            request: Mutex::new(None),
            wake: Condvar::new(),
            result: Mutex::new(None),
            signs: Mutex::new(None),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let (sender, receiver) = mpsc::sync_channel(1);
        let thread_shared = shared.clone();
        let body = crate::body::active();
        thread::Builder::new()
            .name("village road builder".into())
            .spawn(move || {
                crate::body::with_body(body, || {
                    let work = || -> Result<(), String> {
                        let source = Surface::load(&root)?;
                        let version = source.version();
                        let backbone = source.backbone()?;
                        let radius = crate::planet::planet_radius_meters();
                        let key = backbone.region_key(initial);
                        let survey = Engineering {
                            source: &source,
                            shared: &thread_shared,
                            generation: 0,
                        };
                        let plan = Arc::new(road_network_mesh::prepare_local(&backbone,key,radius,&survey)?);
                        let pose = plan.inspection_pose(initial, radius);
                        let centre = pose.map_or(initial, |p| p.0);
                        let mut initial_plans = vec![plan.clone()];
                        if centre.length() < radius {
                            // Do not spawn a swimmer beside an unloaded sea
                            // gateway, exposing sky at the end of a dry bore.
                            // Load the immediate continuation before showing it;
                            // the rest of the neighbourhood remains background.
                            let cell_centre = road_network::direction(key, 0.5, 0.5);
                            for terminal in &plan.region.terminals {
                                if !matches!(
                                    terminal.kind,
                                    road_network::TerminalKind::Gateway { .. }
                                ) {
                                    continue;
                                }
                                let d = DVec3::from_array(terminal.direction);
                                if d.distance(centre.normalize()) * radius > 2000.0 {
                                    continue;
                                }
                                let neighbour = tile_key_for_direction(
                                    d.lerp(cell_centre, -0.01).normalize(),
                                    key.level,
                                );
                                if neighbour != key
                                    && !initial_plans.iter().any(|p| p.region.key == neighbour)
                                {
                                    initial_plans.push(Arc::new(road_network_mesh::prepare_local(&backbone,neighbour,radius,&survey)?));
                                }
                            }
                        }
                        let mut snapshot =
                            road_network_mesh::snapshot(&initial_plans, centre, radius, &survey)?;
                        crate::village_paths::attach_courts(&mut snapshot, &initial_plans);
                        snapshot.inspection = pose.or(snapshot.inspection);
                        let mut generated = crate::road_overlay::Generated::new();
                        for plan in &initial_plans {
                            crate::road_overlay::record(&mut generated, &plan.region);
                        }
                        let mut overview = crate::road_overlay::overview(&backbone, &generated, radius);
                        snapshot.overview = overview.clone();
                        let mut overview_regions = generated.len();
                        sender
                            .send(Ok((version, snapshot)))
                            .map_err(|_| "road startup receiver closed")?;
                        let mut cache = HashMap::<TileKey, (Arc<Prepared>, u64)>::new();
                        for plan in initial_plans {
                            cache.insert(plan.region.key, (plan, 0));
                        }
                        let mut tick = 0_u64;
                        let mut navigation = crate::road_navigation::Navigator::default();
                        loop {
                            let request = {
                                let mut slot = thread_shared.request.lock().unwrap();
                                while slot.is_none() && !thread_shared.stop.load(Ordering::Relaxed)
                                {
                                    slot = thread_shared.wake.wait(slot).unwrap();
                                }
                                if thread_shared.stop.load(Ordering::Relaxed) {
                                    return Ok(());
                                }
                                slot.take().unwrap()
                            };
                            let survey = Engineering {
                                source: &source,
                                shared: &thread_shared,
                                generation: request.generation,
                            };
                            let mut nav_plans = Vec::new();
                            let result = (|| {
                                let mut plans = Vec::new();
                                tick += 1;
                                for key in nearby_keys(request.centre) {
                                    if survey.cancelled() {
                                        return Err("road generation superseded".into());
                                    }
                                    if !cache.contains_key(&key) {
                                        let plan = road_network_mesh::prepare_local(&backbone,key,radius,&survey)?;
                                        if cache.len() >= CACHE_REGIONS {
                                            let oldest = *cache
                                                .iter()
                                                .min_by_key(|(_, (_, age))| *age)
                                                .unwrap()
                                                .0;
                                            cache.remove(&oldest);
                                        }
                                        crate::road_overlay::record(&mut generated, &plan.region);
                                        cache.insert(key, (Arc::new(plan), tick));
                                    }
                                    let (plan, age) = cache.get_mut(&key).unwrap();
                                    *age = tick;
                                    plans.push(plan.clone());
                                }
                                nav_plans = plans.clone();
                                let mut snapshot = road_network_mesh::snapshot(&plans, request.centre, radius, &survey)?;
                                crate::village_paths::attach_courts(&mut snapshot, &plans);
                                if overview_regions != generated.len() {
                                    overview = crate::road_overlay::overview(&backbone, &generated, radius);
                                    overview_regions = generated.len();
                                }
                                snapshot.overview = overview.clone();
                                Ok(snapshot)
                            })();
                            let frame = result.as_ref().ok().map(|s| s.field.frame);
                            if !survey.cancelled() {
                                *thread_shared.result.lock().unwrap() =
                                    Some((request.generation, result));
                            }
                            // Publish roads first. Navigation can engineer distant regions;
                            // never hold back a valid resident road set for sign searches.
                            if let Some(frame) = frame.filter(|_| !survey.cancelled()) {
                                let result = crate::road_navigation::boards(&mut navigation,
                                    &nav_plans, request.centre, radius, &backbone, &survey)
                                    .and_then(|boards| crate::road_signs::bake(&boards, frame));
                                if !survey.cancelled() {
                                    match result {
                                        Ok(mesh) => *thread_shared.signs.lock().unwrap() = Some((request.generation, mesh)),
                                        Err(error) => tracing::warn!(%error,"road sign planning rejected; roads retained"),
                                    }
                                }
                            }
                        }
                    };
                    if let Err(error) = work() {
                        // Startup failures have no partially connected substitute.
                        let _ = sender.send(Err(error));
                    }
                })
            })
            .map_err(|e| e.to_string())?;
        let (version, snapshot) = receiver.recv().map_err(|e| e.to_string())??;
        Ok((
            Self {
                shared,
                last_centre: initial,
                source_version: version,
            },
            snapshot,
        ))
    }
    pub fn request(&mut self, centre: DVec3, force: bool) {
        if !force
            && self.last_centre.distance(centre) < 1000.0
            && tile_key_for_direction(self.last_centre, road_network::REGION_LEVEL)
                == tile_key_for_direction(centre, road_network::REGION_LEVEL)
        {
            return;
        }
        self.last_centre = centre;
        let generation = self.shared.generation.fetch_add(1, Ordering::Relaxed) + 1;
        *self.shared.request.lock().unwrap() = Some(Request { centre, generation });
        self.shared.wake.notify_one();
    }
    pub fn take(&self) -> Option<(u64, Result<Snapshot, String>)> {
        self.shared.result.lock().unwrap().take()
    }
    pub fn take_signs(&self) -> Option<(u64, crate::road_signs::Mesh)> {
        self.shared.signs.lock().unwrap().take()
    }
    pub fn current(&self, generation: u64) -> bool {
        self.shared.generation.load(Ordering::Relaxed) == generation
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Pair the predicate change with the condvar mutex: otherwise notify
        // can race between the worker's predicate check and entering wait.
        let _slot = self.shared.request.lock().unwrap();
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "asset-backed verified longest-undersea entrance, no display/GPU"]
    fn road_network_live_longest_undersea_start_is_at_the_mouth_facing_inside() {
        let initial = crate::road_network_render::longest_undersea_start_position();
        let (_worker, snapshot) = Worker::start(crate::find_default_outmap().unwrap(), initial).unwrap();
        let (eye, target) = snapshot.inspection.expect("entrance pose");
        assert!(eye.distance(initial) < 32.0, "must use the inland mouth, not the village: {eye:?}");
        let field = &snapshot.field;
        let local = field.frame.local(eye);
        let target = field.frame.local(target);
        let approach = field.segments.iter().filter(|s| !s.tunnel)
            .min_by(|a,b| a.nearest(local.truncate()).1.total_cmp(&b.nearest(local.truncate()).1)).unwrap();
        assert!(eye.length() - field.frame.radius >= 7.0,
            "start on the dry landward approach above sea level");
        assert!(!crate::road_network_survey::Surface::load(&crate::find_default_outmap().unwrap())
            .unwrap().engineering_ground(eye.normalize()).unwrap().ocean);
        let (floor, across) = approach.nearest(local.truncate());
        assert!(across < 0.01 && (local.z - floor.z - 2.0).abs() < 0.05, "eye must be on the open approach at the mouth");
        let covered = field.segments.iter().filter(|s| s.tunnel)
            .min_by(|a,b| a.nearest(target.truncate()).1.total_cmp(&b.nearest(target.truncate()).1)).unwrap();
        let (floor, across) = covered.nearest(target.truncate());
        assert!(across < 0.01 && (target.z - floor.z - 2.0).abs() < 0.05, "look into the covered carriageway");
        let inside = field.frame.world(floor + DVec3::Z * 2.0);
        assert!(field.tunnel_void(inside.normalize(), inside.length() - field.frame.radius) > 0.0);
        eprintln!("longest-undersea entrance eye={eye:?}, target={:?}, {} tunnel spans", field.frame.world(target), snapshot.tunnels);
    }

    #[test]
    #[ignore = "asset-backed photographed shared fork and support clearance; no GPU"]
    fn road_network_live_photographed_fork_shares_trunk_and_clearance() {
        let eye = DVec3::new(1818071.767832849, 2162375.6548068463, 2843752.6942250505);
        let (mut worker, _) = Worker::start(crate::find_default_outmap().unwrap(), eye).unwrap();
        worker.request(eye, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        let snapshot = loop {
            if let Some((_, s)) = worker.take() {
                break s.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        // At this photographed fork, ten pairs of spans shared a horizontal
        // prefix with up to 11.7m height separation. It must be one trunk.
        let field = &snapshot.field;
        let local = field.frame.local(eye);
        let roads: Vec<_> = field
            .segments
            .iter()
            .filter(|s| !s.is_parking() && s.nearest(local.truncate()).1 < 200.0)
            .collect();
        for (i, s) in roads.iter().enumerate() {
            let delta = (s.b - s.a).truncate();
            let length = delta.length();
            let u = delta / length;
            for other in &roads[..i] {
                let d = (other.b - other.a).truncate();
                if u.dot(d.normalize()).abs() < 0.99999 {
                    continue;
                }
                let a = (other.a - s.a).truncate().dot(u);
                let b = (other.b - s.a).truncate().dot(u);
                let lo = a.min(b).max(0.0);
                let hi = a.max(b).min(length);
                if hi - lo < 0.2 {
                    continue;
                }
                let p = s.a.lerp(s.b, (lo + hi) * 0.5 / length);
                assert!(
                    other.nearest(p.truncate()).1 >= 0.05,
                    "photographed fork still has duplicate horizontal trunks: {:?} {:?}",
                    s.a,
                    other.a
                );
            }
        }
        // Inspect real generated axis-aligned 0.9m pier boxes, rather than
        // hiding supports or accepting two road surfaces at different levels.
        let mut piers = std::collections::BTreeSet::new();
        for vertices in snapshot.vertices.windows(36).step_by(3) {
            if vertices.iter().any(|v| {
                v.colour
                    .iter()
                    .zip([0.32, 0.31, 0.29])
                    .any(|(&a, b)| (a - b).abs() > 1e-5)
            }) {
                continue;
            }
            let mut low = DVec3::splat(f64::INFINITY);
            let mut high = DVec3::splat(f64::NEG_INFINITY);
            for v in vertices {
                let p = DVec3::from_array(v.position.map(f64::from));
                low = low.min(p);
                high = high.max(p);
            }
            let size = high - low;
            if (size.x - 0.9).abs() > 0.003
                || (size.y - 0.9).abs() > 0.003
                || size.z < 2.0
                || ((low + high) * 0.5 - local).truncate().length() > 200.0
            {
                continue;
            }
            if vertices.iter().any(|v| {
                let p = DVec3::from_array(v.position.map(f64::from));
                (p - low).abs().min((p - high).abs()).max_element() > 0.003
            }) {
                continue;
            }
            if !piers.insert((
                (low.x * 100.0).round() as i64,
                (low.y * 100.0).round() as i64,
                (low.z * 100.0).round() as i64,
            )) {
                continue;
            }
            for x in [0.05, 0.5, 0.95] {
                for y in [0.05, 0.5, 0.95] {
                    let p = low + size * DVec3::new(x, y, 0.0);
                    for s in &roads {
                        let d = (s.b - s.a).truncate();
                        let t = (p - s.a).truncate().dot(d) / d.length_squared();
                        if !(0.0..=1.0).contains(&t) {
                            continue;
                        }
                        let deck = s.a.lerp(s.b, t);
                        let inside = (p - deck).truncate().length() < 4.8;
                        assert!(
                            !(inside && low.z < deck.z && high.z > deck.z + 0.25),
                            "support blocks the photographed carriageway: {low:?} {high:?} at {deck:?}"
                        );
                    }
                }
            }
        }
        assert!(piers.len() >= 2, "the test must examine actual supports");
        eprintln!(
            "photographed fork: {} spans and {} unobstructing piers",
            roads.len(),
            piers.len()
        );
    }

    #[test]
    #[ignore = "asset-backed asynchronous road sign/atlas publication; no GPU"]
    fn road_network_live_signs_follow_published_field() {
        let initial = DVec3::new(2574139.618674488, -2554011.900402223, 1699659.6951081678)
            .normalize()
            * 4_010_000.0;
        let (mut worker, start) =
            Worker::start(crate::find_default_outmap().unwrap(), initial).unwrap();
        worker.request(start.build_centre, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        let mut published = None;
        loop {
            if let Some((generation, result)) = worker.take() {
                let snapshot = result.unwrap();
                assert!(snapshot.villages >= 39);
                published = Some(generation);
            }
            if let Some((generation, signs)) = worker.take_signs() {
                assert_eq!(
                    Some(generation),
                    published,
                    "signs must follow the matching road field"
                );
                assert!(!signs.vertices.is_empty());
                assert!(signs.inspection.is_some());
                eprintln!(
                    "signs: {} vertices, {}x{} atlas; kinds {:?}",
                    signs.vertices.len(),
                    signs.size[0],
                    signs.size[1],
                    signs.counts
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "sign publication deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    #[test]
    #[ignore = "asset-backed photographed empty regional branch; no GPU"]
    fn road_network_live_snow_road_has_no_unserved_dead_end() {
        let eye = DVec3::new(1804537.8505246886, 2191767.3184277415, 2834020.239721973);
        let (mut worker, _) = Worker::start(crate::find_default_outmap().unwrap(), eye).unwrap();
        worker.request(eye, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        let snapshot = loop {
            if let Some((_, result)) = worker.take() {
                break result.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(snapshot.keys.len(), nearby_keys(eye).len());
        assert_eq!(snapshot.villages, 10, "do not remove actual settlements");
        // Endpoint projected onto the abrupt stop in manual/1792411566-1161582.
        // It is the gateway into an empty one-gateway region, not a portal.
        let endpoint = DVec3::new(7056.787246791568, -6034.3787442639, 11264.672527371906);
        assert!(
            snapshot
                .field
                .segments
                .iter()
                .all(|s| { s.nearest(endpoint.truncate()).1 > 30.0 }),
            "a road must not be built just to an uninhabited regional boundary"
        );
        eprintln!(
            "snow site: {} regions, {} villages, {} field segments",
            snapshot.keys.len(),
            snapshot.villages,
            snapshot.field.segments.len()
        );
    }
    #[test]
    #[ignore = "manual flight resident updates, asset-backed without GPU"]
    fn road_network_live_manual_flight_publishes_roads_at_unserved_villages() {
        for position in [
            [1820798.7659331304, 2146703.3691791724, 2857276.798427007],
            [1826056.4364370368, 2125286.357207378, 2868723.9804182025],
        ] {
            let initial = DVec3::from_array(position);
            let (mut worker, _) =
                Worker::start(crate::find_default_outmap().unwrap(), initial).unwrap();
            worker.request(initial, true);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
            let snapshot = loop {
                if let Some((_, result)) = worker.take() {
                    break result.unwrap();
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            assert_eq!(snapshot.keys.len(), nearby_keys(initial).len());
            eprintln!(
                "manual flight: {:?}, {} villages, {} spans",
                initial, snapshot.villages, snapshot.segments
            );
        }
    }
    #[test]
    #[ignore = "asset-backed network startup, no display/GPU"]
    fn road_network_live_village_startup_mesh_is_complete_and_bounded() {
        let root = crate::find_default_outmap().expect("active bake");
        let initial = DVec3::new(2574139.618674488, -2554011.900402223, 1699659.6951081678)
            .normalize()
            * 4_010_000.0;
        let (worker, snapshot) = Worker::start(root, initial).unwrap();
        assert_eq!(snapshot.villages, 39);
        assert!(
            snapshot.field.segments.iter().any(|s| s.is_parking()),
            "village entrance courts must share the road field"
        );
        assert_eq!(snapshot.keys.len(), 1);
        assert!(!snapshot.vertices.is_empty());
        assert!(snapshot.vertices.len() <= crate::road_network_mesh::MAX_VERTICES);
        assert!(
            snapshot
                .vertices
                .iter()
                .all(|v| v.position.iter().all(|c| c.is_finite()))
        );
        eprintln!(
            "network startup: {} villages, {} segments, {} vertices",
            snapshot.villages,
            snapshot.segments,
            snapshot.vertices.len()
        );
        drop(worker);
    }
    #[test]
    #[ignore = "asset-backed streamed neighbour engineering, no display/GPU"]
    fn road_network_live_village_neighbours_publish_one_complete_set() {
        let root = crate::find_default_outmap().expect("active bake");
        let initial = DVec3::new(2574139.618674488, -2554011.900402223, 1699659.6951081678)
            .normalize()
            * 4_010_000.0;
        let (mut worker, snapshot) = Worker::start(root, initial).unwrap();
        worker.request(snapshot.build_centre, true);
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_secs(600);
        let ready = loop {
            if let Some((_, ready)) = worker.take() {
                break ready.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "neighbour generation deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        eprintln!("neighbour generation took {:.1}s", started.elapsed().as_secs_f64());
        assert_eq!(ready.keys.len(), nearby_keys(snapshot.build_centre).len());
        assert!(ready.villages >= snapshot.villages);
        assert!(ready.vertices.len() <= crate::road_network_mesh::MAX_VERTICES);
        eprintln!(
            "complete resident set: {} regions, {} villages, {} spans, {} vertices",
            ready.keys.len(),
            ready.villages,
            ready.segments,
            ready.vertices.len()
        );
    }
    #[test]
    #[ignore = "asset-backed undersea/cube-seam network, no display/GPU"]
    fn road_network_live_undersea_neighbours_keep_a_dry_covered_route() {
        let root = crate::find_default_outmap().expect("active bake");
        let key = TileKey {
            face: planet_coretypes::CubeFace::NegativeX,
            level: road_network::REGION_LEVEL,
            x: 0,
            y: 184,
        };
        let initial = crate::road_network_render::undersea_start_position();
        let (mut worker, snapshot) = Worker::start(root, initial).unwrap();
        let (eye, _) = snapshot.inspection.unwrap();
        let altitude = eye.length() - 4e6;
        assert!(altitude < 0.0);
        assert!(
            snapshot.keys.iter().any(|k| k.face != key.face),
            "a dry seam start needs its immediate continuation before the first frame"
        );
        let gateway = DVec3::new(-0.674991988920733, 0.2979456826095423, -0.674991988920733);
        assert!(snapshot.field.tunnel_void(gateway, altitude) > 0.0);
        assert!(snapshot.field.tunnel_void(eye.normalize(), altitude) > 1.0);
        assert!(
            snapshot
                .field
                .camera_surface_height(eye.normalize(), altitude, 0.0)
                < altitude - 1.0
        );
        worker.request(snapshot.build_centre, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        let ready = loop {
            if let Some((_, ready)) = worker.take() {
                break ready.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "undersea generation deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(ready.keys.iter().any(|k| k.face != key.face));
        assert!(ready.field.tunnel_void(eye.normalize(), altitude) > 1.0);
        eprintln!(
            "undersea resident set: {} regions, {} villages, {} tunnel spans, {} vertices",
            ready.keys.len(),
            ready.villages,
            ready.tunnels,
            ready.vertices.len()
        );
    }
    #[test]
    #[ignore = "asset-backed mountainous resident network, no display/GPU"]
    fn road_network_live_mountain_neighbours_keep_every_access() {
        let root = crate::find_default_outmap().expect("active bake");
        let initial = crate::road::up() * 4_010_000.0;
        let (mut worker, snapshot) = Worker::start(root, initial).unwrap();
        worker.request(snapshot.build_centre, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        let ready = loop {
            if let Some((_, ready)) = worker.take() {
                break ready.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "mountain generation deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(ready.keys.len(), nearby_keys(snapshot.build_centre).len());
        assert!(ready.villages >= snapshot.villages);
        eprintln!(
            "mountain resident set: {} regions, {} villages, {} spans, {} vertices",
            ready.keys.len(),
            ready.villages,
            ready.segments,
            ready.vertices.len()
        );
    }
    #[test]
    fn road_stream_keeps_only_the_latest_request_and_cancels_old_generations() {
        let shared = Arc::new(Shared {
            request: Mutex::new(None),
            wake: Condvar::new(),
            result: Mutex::new(None),
            signs: Mutex::new(None),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let mut worker = Worker {
            shared: shared.clone(),
            last_centre: DVec3::X * 4e6,
            source_version: 1,
        };
        worker.request(DVec3::Y * 4e6, false);
        let first = shared.request.lock().unwrap().unwrap().generation;
        worker.request(DVec3::Z * 4e6, false);
        let latest = shared.request.lock().unwrap().unwrap();
        assert!(worker.current(latest.generation));
        assert!(!worker.current(first));
        assert_eq!(latest.centre, DVec3::Z * 4e6);
        worker.request(latest.centre, false);
        assert_eq!(shared.generation.load(Ordering::Relaxed), latest.generation);
        drop(worker);
        assert!(shared.stop.load(Ordering::Relaxed));
    }

    #[test]
    fn road_stream_neighbours_span_cube_seams_and_remain_bounded() {
        for d in [
            DVec3::X,
            DVec3::new(1.0, 1.0, 0.0).normalize(),
            DVec3::ONE.normalize(),
        ] {
            let keys = nearby_keys(d);
            assert!((4..=9).contains(&keys.len()));
            assert!(keys.contains(&tile_key_for_direction(d, road_network::REGION_LEVEL)));
            if d.y > 0.0 {
                assert!(keys.iter().any(|k| k.face != keys[0].face));
            }
        }
    }
}

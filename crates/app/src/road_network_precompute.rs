//! Offline, resumable precompute of the village road network. Each retained
//! region is engineered with the same `prepare_local` the live worker uses and
//! written as `<face>_<x>_<y>.json`; a finished file is never recomputed.
//! Headless: no GPU, no frame loop. Tunnel runs are measured per region, so a
//! bore crossing a region gateway is reported as two runs (see `summary.json`).
use crate::{
    road_network::{self, Ground, Survey},
    road_network_mesh,
    road_network_survey::Surface,
};
use glam::DVec3;
use planet_coretypes::TileKey;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
pub struct Options {
    pub output: PathBuf,
    pub threads: usize,
    pub limit: Option<usize>,
    /// min_lat, max_lat, min_lon, max_lon in degrees (north = +Y, east = -Z).
    pub area: Option<[f64; 4]>,
    pub write_plans: bool,
}
impl Options {
    pub fn new(output: PathBuf) -> Self {
        Self {
            output,
            threads: 1,
            limit: None,
            area: None,
            write_plans: false,
        }
    }
    pub fn set(&mut self, flag: &str, value: &str) -> Result<(), String> {
        match flag {
            "--road-precompute-threads" => {
                self.threads = value
                    .parse::<usize>()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or("--road-precompute-threads must be a positive integer")?
            }
            "--road-precompute-limit" => {
                self.limit = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| "--road-precompute-limit must be an integer")?,
                )
            }
            _ => {
                let v: Vec<f64> = value
                    .split(',')
                    .filter_map(|p| p.trim().parse().ok())
                    .collect();
                if v.len() != 4 || v[0] > v[1] || v[2] > v[3] {
                    return Err(
                        "--road-precompute-box needs min_lat,max_lat,min_lon,max_lon".into(),
                    );
                }
                self.area = Some([v[0], v[1], v[2], v[3]]);
            }
        }
        Ok(())
    }
}

/// The live worker's engineering view of the surface, without cancellation.
struct Offline<'a>(&'a Surface);
impl Survey for Offline<'_> {
    fn settlement_core_at(&self, d: DVec3) -> Result<bool, String> {
        self.0.settlement_core_at(d)
    }
    fn village_access_height(&self, g: Ground) -> f64 {
        g.height + 0.25
    }
    fn ground(&self, d: DVec3) -> Result<Ground, String> {
        self.0.engineering_ground(d)
    }
    fn villages(&self, key: TileKey) -> Result<Vec<DVec3>, String> {
        self.0.villages(key)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Bore {
    pub length_meters: f64,
    pub undersea_length_meters: f64,
    pub segments: usize,
}
#[derive(Serialize, Deserialize)]
struct RegionFile {
    key: String,
    status: String,
    error: Option<String>,
    source_version: u64,
    villages: usize,
    junctions: usize,
    road_length_meters: f64,
    tunnel_length_meters: f64,
    undersea_tunnel_length_meters: f64,
    bores: Vec<Bore>,
    max_grade: Option<f64>,
    seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<serde_json::Value>,
}

fn name(key: TileKey) -> String {
    format!("{:?}_{}_{}_{}", key.face, key.level, key.x, key.y)
}
fn in_area(area: &Option<[f64; 4]>, d: DVec3) -> bool {
    let Some([a, b, c, e]) = area else {
        return true;
    };
    let d = d.normalize();
    let lat = d.y.asin().to_degrees();
    let lon = (-d.z).atan2(d.x).to_degrees();
    (*a..=*b).contains(&lat) && (*c..=*e).contains(&lon)
}

/// Connected covered components (including branches), with metres over open sea.
fn bores(
    plan: &road_network_mesh::Prepared,
    radius: f64,
    survey: &impl Survey,
) -> Result<Vec<Bore>, String> {
    let p = &plan.profile;
    let point = |i: usize| DVec3::from_array(p.directions[i]) * (radius + p.heights_meters[i]);
    let mut parent: Vec<usize> = (0..p.directions.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (index, &[a, b]) in p.segments.iter().enumerate() {
        if plan.tunnels[index] {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[ra] = rb;
        }
    }
    let mut runs = std::collections::BTreeMap::<usize, Bore>::new();
    for (index, &[a, b]) in p.segments.iter().enumerate() {
        if !plan.tunnels[index] {
            continue;
        }
        let length = point(a).distance(point(b));
        let mid = ((point(a) + point(b)) * 0.5).normalize();
        let sea = survey.ground(mid)?.ocean;
        let root = find(&mut parent, a);
        let run = runs.entry(root).or_insert(Bore {
            length_meters: 0.0,
            undersea_length_meters: 0.0,
            segments: 0,
        });
        run.length_meters += length;
        run.segments += 1;
        if sea {
            run.undersea_length_meters += length;
        }
    }
    Ok(runs.into_values().collect())
}

fn process(
    backbone: &road_network::Backbone,
    surface: &Surface,
    version: u64,
    key: TileKey,
    write_plans: bool,
) -> RegionFile {
    let started = std::time::Instant::now();
    let radius = crate::planet::planet_radius_meters();
    let survey = Offline(surface);
    let mut file = RegionFile {
        key: name(key),
        status: "ok".into(),
        error: None,
        source_version: version,
        villages: 0,
        junctions: 0,
        road_length_meters: 0.0,
        tunnel_length_meters: 0.0,
        undersea_tunnel_length_meters: 0.0,
        bores: Vec::new(),
        max_grade: None,
        seconds: 0.0,
        plan: None,
    };
    let result =
        road_network_mesh::prepare_local(backbone, key, radius, &survey).and_then(|plan| {
            let bores = bores(&plan, radius, &survey)?;
            Ok((plan, bores))
        });
    match result {
        Ok((plan, bores)) => {
            file.villages = plan.region.village_count;
            file.junctions = plan.region.junction_count;
            file.road_length_meters = plan.region.length_meters;
            file.tunnel_length_meters = bores.iter().fold(0.0, |t, b| t + b.length_meters);
            file.undersea_tunnel_length_meters =
                bores.iter().fold(0.0, |t, b| t + b.undersea_length_meters);
            file.max_grade = Some(plan.profile.maximum_grade);
            file.bores = bores;
            if write_plans {
                file.plan = serde_json::to_value((&plan.region, &plan.profile, &plan.tunnels)).ok();
            }
        }
        Err(error) => {
            file.status = "failed".into();
            file.error = Some(error);
        }
    }
    file.seconds = started.elapsed().as_secs_f64();
    file
}

// The writer publishes complete JSON atomically, but an existing file may
// still represent a failed attempt, different terrain or a stats-only run.
// Keep successful compatible results; retry everything else without deleting
// the old result before its replacement is ready.
fn reusable(bytes: &[u8], key: &str, version: u64, require_plan: bool) -> bool {
    let Ok(file) = serde_json::from_slice::<RegionFile>(bytes) else {
        return false;
    };
    file.status == "ok"
        && file.key == key
        && file.source_version == version
        && (!require_plan
            || file.plan.as_ref().is_some_and(|plan| {
                plan.as_array().is_some_and(|parts| {
                    parts.len() == 3 && parts.iter().all(|part| !part.is_null())
                })
            }))
}

pub fn run(root: &Path, options: &Options) -> Result<(), String> {
    if crate::body::active() != crate::body::PLANET {
        return Err("village road networks require the planet".into());
    }
    std::fs::create_dir_all(&options.output).map_err(|e| e.to_string())?;
    eprintln!("Road precompute: loading terrain and backbone");
    let (keys, version) = {
        let surface = Surface::load(root)?;
        let backbone = surface.backbone()?;
        let mut keys: Vec<_> = backbone
            .retained_keys()
            .into_iter()
            .filter(|&k| in_area(&options.area, road_network::direction(k, 0.5, 0.5)))
            .collect();
        if let Some(limit) = options.limit {
            keys.truncate(limit);
        }
        (keys, surface.version())
    };
    let todo: Vec<_> = keys
        .iter()
        .copied()
        .filter(|&k| {
            let key = name(k);
            std::fs::read(options.output.join(format!("{key}.json"))).map_or(true, |bytes| {
                !reusable(&bytes, &key, version, options.write_plans)
            })
        })
        .collect();
    eprintln!(
        "Road precompute: {} regions selected, {} already done, {} threads",
        keys.len(),
        keys.len() - todo.len(),
        options.threads
    );
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let body = crate::body::active();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..options.threads.min(todo.len().max(1)) {
            handles.push(scope.spawn(|| {
                crate::body::with_body(body, || -> Result<(), String> {
                    // One surface per thread: its caches are not shared state.
                    let surface = Surface::load(root)?;
                    let backbone = surface.backbone()?;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&key) = todo.get(i) else {
                            return Ok(());
                        };
                        let file = process(&backbone, &surface, version, key, options.write_plans);
                        let path = options.output.join(format!("{}.json", name(key)));
                        let temporary = path.with_extension("tmp");
                        std::fs::write(
                            &temporary,
                            serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?,
                        )
                        .and_then(|_| std::fs::rename(&temporary, &path))
                        .map_err(|e| e.to_string())?;
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        eprintln!(
                            "[{n}/{}] {} {} {:.1}s tunnels {:.0}m sea {:.0}m",
                            todo.len(),
                            file.key,
                            file.status,
                            file.seconds,
                            file.tunnel_length_meters,
                            file.undersea_tunnel_length_meters
                        );
                    }
                })
            }));
        }
        handles.into_iter().try_for_each(|h| {
            h.join()
                .map_err(|_| "precompute thread panicked".to_owned())?
        })
    })?;
    summarise(&options.output, &keys)
}

#[derive(Serialize)]
struct Longest {
    region: String,
    length_meters: f64,
}
#[derive(Serialize)]
struct Summary {
    regions: usize,
    failed: Vec<String>,
    total_tunnel_meters: f64,
    longest_tunnel: Option<Longest>,
    longest_undersea_tunnel: Option<Longest>,
    note: &'static str,
}
fn summarise(dir: &Path, keys: &[TileKey]) -> Result<(), String> {
    let mut summary = Summary {
        regions: 0,
        failed: Vec::new(),
        total_tunnel_meters: 0.0,
        longest_tunnel: None,
        longest_undersea_tunnel: None,
        note: concat!(
            "Lengths sum connected covered components, including branches, in the final engineered profile. ",
            "Region gateways split components; these are not necessarily entrance-to-exit tunnel lengths. ",
            "Undersea length counts covered segments whose midpoint is over open sea."
        ),
    };
    for &key in keys {
        let path = dir.join(format!("{}.json", name(key)));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        summary.regions += 1;
        if v["status"] != "ok" {
            summary.failed.push(name(key));
            continue;
        }
        summary.total_tunnel_meters += v["tunnel_length_meters"].as_f64().unwrap_or(0.0);
        for bore in v["bores"].as_array().into_iter().flatten() {
            for (field, slot) in [
                ("length_meters", &mut summary.longest_tunnel),
                (
                    "undersea_length_meters",
                    &mut summary.longest_undersea_tunnel,
                ),
            ] {
                let length = bore[field].as_f64().unwrap_or(0.0);
                if length > slot.as_ref().map_or(0.0, |l| l.length_meters) {
                    *slot = Some(Longest {
                        region: name(key),
                        length_meters: length,
                    });
                }
            }
        }
    }
    std::fs::write(
        dir.join("summary.json"),
        serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    eprintln!(
        "Road precompute: {} regions, {} failed, longest tunnel {:?} m, longest undersea {:?} m",
        summary.regions,
        summary.failed.len(),
        summary.longest_tunnel.as_ref().map(|l| l.length_meters),
        summary
            .longest_undersea_tunnel
            .as_ref()
            .map(|l| l.length_meters)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> serde_json::Value {
        serde_json::json!({
            "key": "PositiveX_8_42_1", "status": "ok", "error": null,
            "source_version": 11553600317892695948_u64,
            "villages": 14, "junctions": 13, "road_length_meters": 1000.0,
            "tunnel_length_meters": 200.0, "undersea_tunnel_length_meters": 0.0,
            "bores": [{"length_meters": 200.0, "undersea_length_meters": 0.0, "segments": 10}],
            "max_grade": 0.24, "seconds": 8.0
        })
    }
    fn accepts(record: &serde_json::Value, plans: bool) -> bool {
        reusable(
            &serde_json::to_vec(record).unwrap(),
            "PositiveX_8_42_1",
            11553600317892695948,
            plans,
        )
    }

    #[test]
    fn road_precompute_resume_keeps_complete_compatible_successes() {
        let file = record();
        assert!(accepts(&file, false), "legacy stats remain reusable");
        assert!(!reusable(
            &serde_json::to_vec(&file).unwrap(),
            "PositiveX_8_43_1",
            11553600317892695948,
            false
        ));
        assert!(!reusable(
            &serde_json::to_vec(&file).unwrap(),
            "PositiveX_8_42_1",
            1,
            false
        ));
    }

    #[test]
    fn road_precompute_resume_retries_failures_and_incomplete_files() {
        let mut file = record();
        file["status"] = "failed".into();
        assert!(!accepts(&file, false));
        file["status"] = "unknown".into();
        assert!(!accepts(&file, false));
        assert!(!reusable(
            b"{truncated",
            "PositiveX_8_42_1",
            11553600317892695948,
            false
        ));
        let mut file = record();
        file.as_object_mut().unwrap().remove("bores");
        assert!(!accepts(&file, false));
    }

    #[test]
    fn road_precompute_resume_requires_the_requested_plan_payload() {
        let mut file = record();
        assert!(!accepts(&file, true));
        for plan in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!([null, null, null]),
        ] {
            file["plan"] = plan;
            assert!(!accepts(&file, true));
        }
        file["plan"] = serde_json::json!([{"nodes": [], "edges": []}, {"directions": [], "heights_meters": [], "segments": []}, []]);
        assert!(accepts(&file, true));
    }
}

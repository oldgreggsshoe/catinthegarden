//! Headless road-network survey. No window, GPU, audio or per-frame work.
//! Uses the same village eligibility/footprint sampler as the live renderer.
use crate::{
    outmap::{Outmap, TileData},
    road_network::{self, Backbone, Ground, Survey},
    terrain,
    village::{self, VillageSurface},
};
use glam::DVec3;
use planet_coretypes::{BiomeId, CubeFace, TileKey, direction_to_face_uv, tile_key_for_direction};
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};

pub struct Surface {
    outmap: Outmap,
    tiles: HashMap<TileKey, TileData>,
    fine_tiles: RefCell<HashMap<TileKey, Arc<TileData>>>,
    village_sites: RefCell<HashMap<TileKey, Vec<DVec3>>>,
}
impl Surface {
    pub fn load(root: &Path) -> Result<Self, String> {
        let outmap = Outmap::open(root).map_err(|e| e.to_string())?;
        let level = outmap.manifest().dense_level;
        let side = 1_u32 << level;
        let mut tiles = HashMap::new();
        for face in CubeFace::ALL {
            for y in 0..side {
                for x in 0..side {
                    let key = TileKey { face, level, x, y };
                    let tile = outmap
                        .load_tile(key)
                        .map_err(|e| format!("road survey {key:?}: {e}"))?;
                    if tile.source_key != key {
                        return Err(format!("road survey missing dense tile {key:?}"));
                    }
                    tiles.insert(key, tile);
                }
            }
        }
        Ok(Self {
            outmap,
            tiles,
            fine_tiles: RefCell::new(HashMap::new()),
            village_sites: RefCell::new(HashMap::new()),
        })
    }
    pub fn backbone(&self) -> Result<Backbone, String> {
        Backbone::survey(
            road_network::REGION_LEVEL,
            crate::planet::planet_radius_meters(),
            &Macro(self),
        )
    }
    pub fn version(&self) -> u64 {
        let bytes = serde_json::to_vec(self.outmap.manifest()).expect("serializable manifest");
        let mut hash = 0xcbf29ce484222325_u64;
        for b in bytes
            .iter()
            .copied()
            .chain(include_bytes!("road_network_profile.rs").iter().copied())
            .chain(include_bytes!("road_network_align.rs").iter().copied())
            .chain(include_bytes!("road_vertical.rs").iter().copied())
            .chain(include_bytes!("road_network_route.rs").iter().copied())
        {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
        hash
    }
    /// Worker-only, finest available baked source. Never called by a frame.
    pub fn engineering_ground(&self, d: DVec3) -> Result<Ground, String> {
        let (face, u, v) = direction_to_face_uv(d);
        let requested = tile_key_for_direction(d, self.outmap.manifest().max_level);
        let source = self
            .outmap
            .resolve_tile(requested)
            .map_err(|e| e.to_string())?;
        let owned;
        let tile = if let Some(tile) = self.tiles.get(&source) {
            tile
        } else {
            let mut cache = self.fine_tiles.borrow_mut();
            if !cache.contains_key(&source) {
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(
                    source,
                    Arc::new(self.outmap.load_tile(source).map_err(|e| e.to_string())?),
                );
            }
            owned = cache[&source].clone();
            &owned
        };
        let uv = terrain::source_tile_uv(source, face, [u, v]).ok_or("invalid engineering UV")?;
        let raw = f64::from(terrain::sample_height_cpu(&tile.heights_meters, uv));
        let biome = BiomeId::try_from(terrain::sample_biome_cpu(&tile.biome_ids, uv))
            .map_err(|_| "invalid engineering biome")?;
        let height = if raw <= 0.0 {
            raw
        } else {
            crate::planet::outmap_surface_height_meters(
                raw,
                d,
                0.0,
                crate::planet::continuous_baked_sample_spacing_meters(
                    [u, v],
                    source.level,
                    self.outmap.manifest().dense_level,
                ),
            )
        };
        Ok(Ground {
            height,
            ocean: terrain::is_open_ocean_sample(raw as f32, biome),
        })
    }
    pub fn settlement_core_at(&self, d: DVec3) -> Result<bool, String> {
        let key = tile_key_for_direction(d, village::VILLAGE_CELL_LEVEL);
        let radius =
            (village::VILLAGE_RADIUS_METERS + 15.0) / crate::planet::planet_radius_meters();
        Ok(self
            .villages(key)?
            .iter()
            .any(|p| p.normalize().distance_squared(d) < radius * radius))
    }
    fn raw(&self, d: DVec3) -> Result<(f64, BiomeId, [f64; 2]), String> {
        let (face, u, v) = direction_to_face_uv(d);
        let key = tile_key_for_direction(d, self.outmap.manifest().dense_level);
        let uv =
            terrain::source_tile_uv(key, face, [u, v]).ok_or("road survey invalid source UV")?;
        let tile = self
            .tiles
            .get(&key)
            .ok_or("road survey missing source tile")?;
        let height = f64::from(terrain::sample_height_cpu(&tile.heights_meters, uv));
        let biome = BiomeId::try_from(terrain::sample_biome_cpu(&tile.biome_ids, uv))
            .map_err(|_| "road survey invalid biome")?;
        Ok((height, biome, [u, v]))
    }
    fn ground_at(&self, d: DVec3, detailed: bool) -> Result<Ground, String> {
        if !d.is_finite() || d.length_squared() == 0.0 {
            return Err("invalid survey direction".into());
        }
        let (height, biome, uv) = self.raw(d)?;
        let ocean = terrain::is_open_ocean_sample(height as f32, biome);
        // Negative baked values describe the bathymetry, not the sea surface.
        let height = if height <= 0.0 {
            height
        } else if detailed {
            crate::planet::outmap_surface_height_meters(
                height,
                d,
                0.0,
                crate::planet::continuous_baked_sample_spacing_meters(
                    uv,
                    self.outmap.manifest().dense_level,
                    self.outmap.manifest().dense_level,
                ),
            )
        } else {
            crate::planet::scaled_outmap_macro_height_meters(height, 0.0)
        };
        Ok(Ground { height, ocean })
    }
}
impl VillageSurface for Surface {
    fn village_sample(&self, d: DVec3) -> Option<terrain::ForestSurfaceSample> {
        terrain::dense_siting_surface_sample(d, self.outmap.manifest().dense_level, &self.tiles)
    }
    fn village_open_ocean(&self, d: DVec3) -> Option<bool> {
        self.raw(d)
            .ok()
            .map(|(height, biome, _)| terrain::is_open_ocean_sample(height as f32, biome))
    }
}
impl Survey for Surface {
    fn ground(&self, d: DVec3) -> Result<Ground, String> {
        self.ground_at(d, true)
    }
    fn villages(&self, key: TileKey) -> Result<Vec<DVec3>, String> {
        if key.level != village::VILLAGE_CELL_LEVEL || !key.is_valid() {
            return Err("invalid village cell".into());
        }
        if let Some(sites) = self.village_sites.borrow().get(&key) {
            return Ok(sites.clone());
        }
        let sites = village::village_beam_sites_in_cell(self, key);
        let mut cache = self.village_sites.borrow_mut();
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(key, sites.clone());
        Ok(sites)
    }
}
struct Macro<'a>(&'a Surface);
impl Survey for Macro<'_> {
    fn may_have_villages(&self, region: TileKey) -> Result<bool, String> {
        let subdivision = 1_u32 << (village::VILLAGE_CELL_LEVEL - region.level);
        for y in 0..subdivision {
            for x in 0..subdivision {
                let key = TileKey {
                    level: village::VILLAGE_CELL_LEVEL,
                    x: region.x * subdivision + x,
                    y: region.y * subdivision + y,
                    ..region
                };
                for d in village::village_candidates_in_cell(key) {
                    let (raw, biome, _) = self.0.raw(d)?;
                    let height = crate::planet::scaled_outmap_macro_height_meters(raw, 0.0);
                    if village::village_macro_site_is_possible(biome, height) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    fn ground(&self, d: DVec3) -> Result<Ground, String> {
        self.0.ground_at(d, false)
    }
    fn villages(&self, _: TileKey) -> Result<Vec<DVec3>, String> {
        Err("macro pass must not site villages".into())
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Engineering {
    Feasible {
        profile: crate::road_network_profile::Profile,
    },
    NeedsReroute {
        rejection: crate::road_network_profile::Rejected,
        rerouting: crate::road_network_route::Rejected,
    },
}

#[derive(Serialize)]
struct RegionReport {
    #[serde(flatten)]
    topology: road_network::Region,
    engineering: Engineering,
    rerouting: Option<crate::road_network_route::Stats>,
}

#[derive(Serialize)]
struct Report {
    status: &'static str,
    limitations: &'static str,
    source_seed: u32,
    source_generator: String,
    source_dense_level: u8,
    planet_radius_meters: f64,
    region_level: u8,
    regions: usize,
    retained_regions: usize,
    potentially_inhabited_regions: usize,
    backbone_links: usize,
    undersea_links: usize,
    backbone_length_meters: f64,
    load_seconds: f64,
    backbone_seconds: f64,
    local_seconds: f64,
    samples: Vec<RegionReport>,
}

pub fn run(root: &Path, output: &Path) -> Result<(), String> {
    if output.try_exists().map_err(|e| e.to_string())? {
        return Err(format!("refusing to overwrite {}", output.display()));
    }
    if crate::body::active() != crate::body::PLANET {
        return Err("village road networks require the planet".into());
    }
    let started = Instant::now();
    eprintln!(
        "Road network: loading fixed dense terrain (headless survey, not live road geometry)"
    );
    let terrain = Surface::load(root)?;
    let load_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    eprintln!(
        "Road network: surveying all six faces at level {}",
        road_network::REGION_LEVEL
    );
    let network = Backbone::survey(
        road_network::REGION_LEVEL,
        crate::planet::planet_radius_meters(),
        &Macro(&terrain),
    )?;
    let backbone_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let mut keys = vec![
        network.region_key(DVec3::new(
            2574139.618674488,
            -2554011.900402223,
            1699659.6951081678,
        )),
        network.region_key(crate::road::up()),
    ];
    // Survey both sides of a real land/sea backbone link, not a fabricated
    // island fixture. Exact shared gateway coordinates are part of the output.
    let mut sea_link = None;
    for link in network.links.iter().filter(|l| l.requires_undersea_tunnel) {
        if terrain.ground(DVec3::from_array(link.gateway))?.ocean {
            sea_link = Some(link);
            break;
        }
    }
    if let Some(link) = sea_link {
        for index in [link.a, link.b] {
            let side = 1_usize << network.level;
            keys.push(TileKey {
                face: CubeFace::ALL[index / (side * side)],
                level: network.level,
                x: (index % side) as u32,
                y: (index / side % side) as u32,
            });
        }
    }
    keys.sort();
    keys.dedup();
    let mut samples = Vec::new();
    for key in keys {
        eprintln!("Road network: detailed local survey {key:?}");
        let mut region = network.local(key, crate::planet::planet_radius_meters(), &terrain)?;
        let mut rerouting = None;
        eprintln!(
            "  {} villages, {} junctions, {:.2} km shared alignment",
            region.village_count,
            region.junction_count,
            region.length_meters / 1000.0
        );
        let engineering = match crate::road_network_profile::Profile::fit(
            &region,
            crate::planet::planet_radius_meters(),
            &terrain,
        )? {
            Ok(profile) => {
                eprintln!(
                    "  feasible shared profile: {:.3}% grade, {:.1}m maximum deck clearance",
                    profile.maximum_grade * 100.0,
                    profile.maximum_deck_above_ground_meters
                );
                Engineering::Feasible { profile }
            }
            Err(rejection) => {
                eprintln!("  rerouting required: {:?}", rejection);
                match crate::road_network_route::reroute(
                    &region,
                    crate::planet::planet_radius_meters(),
                    &terrain,
                )? {
                    Ok(plan) => {
                        eprintln!(
                            "  rerouted: {} terminals, {} junctions, {:.2} km, {:.3}% grade; {} states / {:.3}s",
                            plan.region.terminals.len(),
                            plan.region.junction_count,
                            plan.region.length_meters / 1000.0,
                            plan.profile.maximum_grade * 100.0,
                            plan.stats.expanded_states,
                            plan.stats.seconds
                        );
                        region = plan.region;
                        rerouting = Some(plan.stats);
                        Engineering::Feasible {
                            profile: plan.profile,
                        }
                    }
                    Err(failure) => {
                        eprintln!("  rerouting rejected: {:?}", failure);
                        Engineering::NeedsReroute {
                            rejection,
                            rerouting: failure,
                        }
                    }
                }
            }
        };
        samples.push(RegionReport {
            topology: region,
            engineering,
            rerouting,
        });
    }
    let report = Report {
        status: "survey_and_profile_checks_only",
        limitations: "Not rendered; profile checks use pinned dense-level terrain, not final sparse-source/render-LOD grounding. Shared profiles are checked at <=24.9% physical grade (user limit 25%) with a 32m deck-clearance cap and sampled seabed roof cover; infeasible alignments are explicitly rejected for height-aware rerouting. Continuous cover, portals, excavation, approach ramps and collision remain unvalidated. Coarse water links REQUIRE tunnels below the bed. Backbone MST optimises a sampled cost proxy on cell adjacency, not global road construction cost; transit-only leaves are conservatively pruned (coarse occupancy may retain unused land roads).",
        source_seed: terrain.outmap.manifest().seed,
        source_generator: terrain.outmap.manifest().generator.clone(),
        source_dense_level: terrain.outmap.manifest().dense_level,
        planet_radius_meters: crate::planet::planet_radius_meters(),
        region_level: network.level,
        regions: network.region_count(),
        retained_regions: network.retained_region_count(),
        potentially_inhabited_regions: network.potentially_inhabited_regions,
        backbone_links: network.links.len(),
        undersea_links: network
            .links
            .iter()
            .filter(|l| l.requires_undersea_tunnel)
            .count(),
        backbone_length_meters: network.links.iter().map(|l| l.length_meters).sum(),
        load_seconds,
        backbone_seconds,
        local_seconds: started.elapsed().as_secs_f64(),
        samples,
    };
    // create_new refuses to truncate previous evidence or a user file.
    let file =
        File::create_new(output).map_err(|e| format!("creating {}: {e}", output.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &report).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    eprintln!(
        "Road network survey: {} regions / {} links / {} undersea; {:.2}s total; {}",
        report.regions,
        report.backbone_links,
        report.undersea_links,
        report.load_seconds + report.backbone_seconds + report.local_seconds,
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "asset-backed mountain pilot, physical grades and tunnel coverage"]
    fn road_mountain_pilot_profile_report() {
        let root = crate::find_default_outmap().expect("active baked planet");
        let surface = Surface::load(&root).unwrap();
        let plan =
            crate::road::Viaduct::survey(|d| surface.ground(d).ok().map(|g| g.height)).unwrap();
        let min = plan
            .route
            .ground
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let max = plan
            .route
            .ground
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let tunnel_length: f64 = plan
            .tunnels
            .iter()
            .map(|t| plan.route.distances[t.end] - plan.route.distances[t.start])
            .sum();
        eprintln!(
            "mountain pilot: relief={}m length={}m tunnels={} bore_length={}m cut_proxy={}m3 straight_proxy={}m3",
            max - min,
            plan.route.length(),
            plan.tunnels.len(),
            tunnel_length,
            plan.route.cut_volume,
            plan.route.straight_cut_volume
        );
        assert!(max - min > 2000.0, "test site must have substantial relief");
        assert!((29980.0..=30000.0).contains(&plan.route.length()));
        assert!(
            !plan.tunnels.is_empty(),
            "mountain test must exercise covered tunnels"
        );
        for p in &plan.route.points {
            let (height, biome, _) = surface.raw(crate::road::world(*p).normalize()).unwrap();
            assert!(
                height > 0.0 && !matches!(biome, BiomeId::Ocean | BiomeId::Lake),
                "pilot must stay on dry mountain terrain"
            );
        }
    }

    #[test]
    #[ignore = "instrument: measures dry mountainous road corridors on the active bake"]
    fn road_test_mountain_site_survey() {
        let root = crate::find_default_outmap().expect("active baked planet");
        let surface = Surface::load(&root).unwrap();
        let old = crate::road::up();
        let radius = crate::planet::planet_radius_meters();
        let north = (DVec3::Z - old * old.z).normalize();
        let across = north.cross(old);
        let mut candidates = Vec::new();
        for y in -10..=10 {
            for x in -10..=10 {
                let up =
                    (old * radius + across * (x as f64 * 8000.0) + north * (y as f64 * 8000.0))
                        .normalize();
                for heading in [0.0_f64, 90.0, 180.0, 270.0] {
                    let forward = glam::DQuat::from_axis_angle(up, heading.to_radians())
                        * (DVec3::Z - up * up.z).normalize();
                    let mut heights = Vec::new();
                    let mut dry = 0;
                    for row in 0..=60 {
                        let d = (up * radius + forward * (row as f64 * 500.0)).normalize();
                        let (raw, biome, _) = surface.raw(d).unwrap();
                        dry += usize::from(
                            raw > 0.0 && !matches!(biome, BiomeId::Ocean | BiomeId::Lake),
                        );
                        heights.push(surface.ground_at(d, false).unwrap().height);
                    }
                    let min = heights.iter().copied().fold(f64::INFINITY, f64::min);
                    let max = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    if dry >= 55 && min > 0.0 && max < 25_000.0 {
                        candidates.push((max - min, up, forward, heading));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut found = 0;
        for (_, up, forward, heading) in candidates.into_iter().take(80) {
            let side = forward.cross(up);
            let mut ground = |p: glam::DVec2| -> Option<f64> {
                let mut z = 0.0;
                for _ in 0..2 {
                    let d = (up * (radius + z) + forward * p.y + side * p.x).normalize();
                    let (raw, biome, _) = surface.raw(d).ok()?;
                    if raw <= 0.0 || matches!(biome, BiomeId::Ocean | BiomeId::Lake) {
                        return None;
                    }
                    let h = surface.ground(d).ok()?.height;
                    z = d.dot(up) * (radius + h) - radius;
                }
                Some(z)
            };
            if let Some(route) = crate::road_route::Route::plan(&mut ground) {
                let tunnels = crate::road_tunnel::select(&route, &mut ground);
                let min = route.ground.iter().copied().fold(f64::INFINITY, f64::min);
                let max = route
                    .ground
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max);
                if max - min > 2000.0 && !tunnels.is_empty() {
                    found += 1;
                    eprintln!(
                        "mountain candidate {}",
                        serde_json::to_string(&(max - min, tunnels.len(), up.to_array(), heading))
                            .unwrap()
                    );
                    if found == 5 {
                        break;
                    }
                }
            }
        }
        assert!(
            found > 0,
            "no dry mountain route with a covered tunnel found"
        );
    }

    #[test]
    #[ignore = "instrument: requires the active baked planet; no GPU needed"]
    fn road_network_conservative_occupancy_never_prunes_an_accepted_village() {
        let root = crate::find_default_outmap().expect("active baked planet");
        let surface = Surface::load(&root).unwrap();
        let coarse = Macro(&surface);
        let mut accepted = 0;
        let mut empty = 0;
        for face in CubeFace::ALL {
            for n in 0..24_u32 {
                let region = TileKey {
                    face,
                    level: road_network::REGION_LEVEL,
                    x: (n * 73 + 43) % 256,
                    y: (n * 37 + 1) % 256,
                };
                let possible = coarse.may_have_villages(region).unwrap();
                if !possible {
                    empty += 1;
                }
                for y in 0..4 {
                    for x in 0..4 {
                        let cell = TileKey {
                            face,
                            level: village::VILLAGE_CELL_LEVEL,
                            x: region.x * 4 + x,
                            y: region.y * 4 + y,
                        };
                        let sites = surface.villages(cell).unwrap();
                        assert!(
                            sites.is_empty() || possible,
                            "accepted village would be pruned in {region:?}"
                        );
                        accepted += sites.len();
                    }
                }
            }
        }
        assert!(
            accepted > 0 && empty > 0,
            "survey must exercise both settlements and rejected cells"
        );
        eprintln!(
            "conservative occupancy: {accepted} accepted villages retained; {empty} empty regions checked"
        );
    }
}

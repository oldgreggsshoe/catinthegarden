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
    collections::HashMap,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

pub struct Surface {
    outmap: Outmap,
    tiles: HashMap<TileKey, TileData>,
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
        Ok(Self { outmap, tiles })
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
        Ok(village::village_beam_sites_in_cell(self, key))
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
    },
}

#[derive(Serialize)]
struct RegionReport {
    #[serde(flatten)]
    topology: road_network::Region,
    engineering: Engineering,
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
        let region = network.local(key, crate::planet::planet_radius_meters(), &terrain)?;
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
                Engineering::NeedsReroute { rejection }
            }
        };
        samples.push(RegionReport {
            topology: region,
            engineering,
        });
    }
    let report = Report {
        status: "survey_and_profile_checks_only",
        limitations: "Not rendered; profile checks use pinned dense-level terrain, not final sparse-source/render-LOD grounding. Shared profiles are checked at <=9.9% physical grade with a 32m deck-clearance cap and sampled seabed roof cover; infeasible alignments are explicitly rejected for rerouting. Continuous cover, portals, excavation, approach ramps and collision remain unvalidated. Coarse water links REQUIRE tunnels below the bed. Backbone MST optimises a sampled cost proxy on cell adjacency, not global road construction cost; transit-only leaves are conservatively pruned (coarse occupancy may retain unused land roads).",
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

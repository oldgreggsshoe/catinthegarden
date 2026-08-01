//! Temporary instrument: what the detail ladder actually builds at the
//! mountains, in the terms a hill is described in.
//!
//! Delete once the mountain character question is settled.

#[cfg(test)]
mod tests {
    use crate::planet::{
        PLANET_RADIUS_METERS, TERRAIN_DETAIL_START_WAVELENGTH_METERS,
        TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS, baked_sample_spacing_meters,
        scaled_outmap_macro_height_meters, terrain_detail_meters,
    };
    use glam::DVec3;

    /// The `tour_mountains` look-at, normalised. Everything the camera sees
    /// there is the runtime ladder: the baked data is L4, ~1953m per texel.
    fn mountain_direction() -> DVec3 {
        DVec3::new(2_384_910.906, 19_981.783, 3_217_300.842).normalize()
    }

    fn percentile(sorted: &[f64], q: f64) -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        let index = ((sorted.len() - 1) as f64 * q).round() as usize;
        sorted[index]
    }

    /// Walks a transect and reports it the way a hill is described: how much
    /// relief there is inside a given horizontal distance, and how steep the
    /// ground is.
    fn survey(label: &str, spacing_meters: f64, samples: usize) {
        let centre = mountain_direction();
        // A tangent basis, so the walk is along the surface rather than through
        // it, and each step really is `spacing_meters` of ground.
        let east = centre.cross(DVec3::Y).normalize();
        let baked_spacing = crate::planet::baked_sample_spacing_meters(4);
        // Well above every octave's headroom knee, which is where the
        // mountains are: 4721m of macro surface saturates all thirteen.
        let macro_height = 4_721.0;

        let heights: Vec<f64> = (0..samples)
            .map(|index| {
                let offset = index as f64 * spacing_meters;
                let direction = (centre + east * (offset / PLANET_RADIUS_METERS)).normalize();
                terrain_detail_meters(direction, baked_spacing, macro_height)
            })
            .collect();

        let mut slopes_deg: Vec<f64> = heights
            .windows(2)
            .map(|pair| {
                ((pair[1] - pair[0]) / spacing_meters)
                    .abs()
                    .atan()
                    .to_degrees()
            })
            .collect();
        slopes_deg.sort_by(f64::total_cmp);

        let span = heights.iter().cloned().fold(f64::MIN, f64::max)
            - heights.iter().cloned().fold(f64::MAX, f64::min);

        // Local relief: the biggest rise inside a fixed horizontal window, which
        // is what separates a plateau from a peak.
        let mut window_reliefs = Vec::new();
        for window_meters in [500.0_f64, 1_000.0, 2_000.0, 3_000.0] {
            let width = (window_meters / spacing_meters) as usize;
            if width < 2 || width >= heights.len() {
                continue;
            }
            let relief = heights
                .windows(width)
                .map(|window| {
                    window.iter().cloned().fold(f64::MIN, f64::max)
                        - window.iter().cloned().fold(f64::MAX, f64::min)
                })
                .fold(f64::MIN, f64::max);
            window_reliefs.push((window_meters, relief));
        }

        println!(
            "\n== {label}  ({samples} samples at {spacing_meters}m = {:.1}km transect)",
            samples as f64 * spacing_meters / 1000.0
        );
        println!("   total relief over the transect : {span:8.1} m");
        for (window, relief) in window_reliefs {
            println!("   max relief within {window:6.0} m     : {relief:8.1} m");
        }
        println!(
            "   slope deg  p50 {:5.2}  p90 {:5.2}  p99 {:5.2}  max {:5.2}",
            percentile(&slopes_deg, 0.50),
            percentile(&slopes_deg, 0.90),
            percentile(&slopes_deg, 0.99),
            percentile(&slopes_deg, 1.00),
        );
        let over_25 = slopes_deg.iter().filter(|angle| **angle > 25.0).count() as f64
            / slopes_deg.len() as f64;
        let over_35 = slopes_deg.iter().filter(|angle| **angle > 35.0).count() as f64
            / slopes_deg.len() as f64;
        println!(
            "   ground steeper than 25 deg     : {:6.3}%",
            over_25 * 100.0
        );
        println!(
            "   ground steeper than 35 deg     : {:6.3}%",
            over_35 * 100.0
        );
    }

    /// Does the rock material fire now?
    ///
    /// `rock_amount = smoothstep(0.10, 0.42, slope)` where slope is
    /// `1 - dot(normal, radial)`. HANDOFF 6.2 measured that at p50 0.0041,
    /// p99 0.0217, max 0.159 -- crossing 0.10 over 0.004% of the surface, so
    /// the path was wired in and had essentially never run. The mountain work
    /// moved the slope distribution a long way, so that premise needs
    /// re-measuring before anything is built on it either way.
    #[test]
    #[ignore = "instrument: cargo test -- --ignored --nocapture rock_fires"]
    fn does_the_rock_material_fire() {
        let centre = mountain_direction();
        let east = centre.cross(DVec3::Y).normalize();
        let north = centre.cross(east).normalize();
        let baked_spacing = crate::planet::baked_sample_spacing_meters(4);

        // The shader central-differences its normal over camera_distance *
        // TERRAIN_DETAIL_FILTER_RATIO, clamped to [0.5, 256]m, and filters the
        // octaves to the same spacing. So the slope the *materials* see is a
        // function of how far away the ground is, not of the height field.
        for (label, macro_height, step) in [
            ("mountain @ 4m probe (50m away)", 4_721.0_f64, 4.0_f64),
            ("mountain @ 10m probe (1km away)", 4_721.0, 10.0),
            ("mountain @ 30m probe (3km away)", 4_721.0, 30.0),
            ("mountain @ 100m probe (10km away)", 4_721.0, 100.0),
            ("mountain @ 256m probe (26km+, the cap)", 4_721.0, 256.0),
            ("plain @ 4m probe", 300.0, 4.0),
        ] {
            let mut slopes = Vec::new();
            for iy in 0..200 {
                for ix in 0..200 {
                    let at = |dx: f64, dy: f64| {
                        let offset = east * ((ix as f64 * step + dx) / PLANET_RADIUS_METERS)
                            + north * ((iy as f64 * step + dy) / PLANET_RADIUS_METERS);
                        terrain_detail_meters(
                            (centre + offset).normalize(),
                            baked_spacing,
                            macro_height,
                        )
                    };
                    let centre_h = at(0.0, 0.0);
                    let dhdx = (at(step, 0.0) - centre_h) / step;
                    let dhdy = (at(0.0, step) - centre_h) / step;
                    // 1 - cos(angle between the surface normal and the radial)
                    let gradient = (dhdx * dhdx + dhdy * dhdy).sqrt();
                    slopes.push(1.0 - 1.0 / (1.0 + gradient * gradient).sqrt());
                }
            }
            slopes.sort_by(f64::total_cmp);
            let rock = |slope: f64| {
                let t = ((slope - 0.10) / 0.32).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let firing =
                slopes.iter().filter(|s| **s > 0.10).count() as f64 / slopes.len() as f64 * 100.0;
            println!("\n== {label} ({} samples at {step}m)", slopes.len());
            println!(
                "   slope (1 - N.radial)  p50 {:.4}  p90 {:.4}  p99 {:.4}  max {:.4}",
                percentile(&slopes, 0.50),
                percentile(&slopes, 0.90),
                percentile(&slopes, 0.99),
                percentile(&slopes, 1.00),
            );
            println!("   ground past the 0.10 rock threshold : {firing:6.3}%");
            println!(
                "   rock_amount           p50 {:.3}  p90 {:.3}  p99 {:.3}  max {:.3}",
                rock(percentile(&slopes, 0.50)),
                rock(percentile(&slopes, 0.90)),
                rock(percentile(&slopes, 0.99)),
                rock(percentile(&slopes, 1.00)),
            );
        }
        println!("\n   HANDOFF 6.2 measured, before the mountain work:");
        println!("   p50 0.0041, p99 0.0217, max 0.159, crossing 0.10 over 0.004% of surface");
    }

    /// What the mesh drops, measured rather than assumed.
    ///
    /// `OUTMAP_GEOMETRIC_ERROR_RATIO` is `error = pi/4 * ratio * vertex
    /// spacing`, and the ladder's share of it was derived analytically for a
    /// plain fBm. Ridging, attenuation and the spectral tilt all change the
    /// per-octave RMS, so the constant has to be re-measured or the selector
    /// under-tessellates and the silhouettes stair-step. The chord residual
    /// between vertices *is* the geometric error, so measure that.
    #[test]
    #[ignore = "instrument: cargo test -- --ignored --nocapture mesh_drops"]
    fn what_the_mesh_drops() {
        let centre = mountain_direction();
        let east = centre.cross(DVec3::Y).normalize();
        let baked_spacing = crate::planet::baked_sample_spacing_meters(4);

        println!("\n  vertex spacing | chord-residual RMS | implied ratio");
        println!("  ----------------------------------------------------");
        let mut worst: f64 = 0.0;
        for macro_height in [4_721.0_f64, 300.0] {
            println!("  -- macro height {macro_height:.0} m");
            for spacing in [12.0_f64, 48.0, 192.0, 768.0] {
                let fine = spacing / 16.0;
                let samples = 8_192;
                let heights: Vec<f64> = (0..samples)
                    .map(|index| {
                        let offset = index as f64 * fine;
                        let direction =
                            (centre + east * (offset / PLANET_RADIUS_METERS)).normalize();
                        terrain_detail_meters(direction, baked_spacing, macro_height)
                    })
                    .collect();
                let step = 16_usize;
                let mut sum_sq = 0.0;
                let mut count = 0_usize;
                let mut vertex = 0_usize;
                while vertex + step < heights.len() {
                    let a = heights[vertex];
                    let b = heights[vertex + step];
                    for inner in 1..step {
                        let t = inner as f64 / step as f64;
                        let chord = a + (b - a) * t;
                        let residual = heights[vertex + inner] - chord;
                        sum_sq += residual * residual;
                        count += 1;
                    }
                    vertex += step;
                }
                let rms = (sum_sq / count as f64).sqrt();
                let ratio = rms / spacing / (std::f64::consts::PI / 4.0);
                worst = worst.max(ratio);
                println!("  {spacing:9.0} m    |      {rms:9.4} m     |     {ratio:.4}");
            }
        }
        println!("\n  worst implied ratio: {worst:.4}");
        println!(
            "  what the selector currently charges: {:.4}",
            0.0536 + crate::planet::TERRAIN_DETAIL_ROUGHNESS * 2.9395
        );
    }

    /// The same transect against the *baked* macro surface, which decides
    /// whether the mountains are a gentle upland in the bake or a good massif
    /// the ladder is flattening. Reads the outmap directly at the finest level
    /// available out here.
    #[test]
    #[ignore = "instrument: cargo test -- --ignored --nocapture baked_macro_relief"]
    fn baked_macro_relief_at_the_mountains() {
        use crate::outmap::Outmap;
        use catinthegarden_coretypes::TileKey;

        let outmap = Outmap::open(std::path::Path::new("../../assets/outmaps/test-planet"))
            .or_else(|_| Outmap::open(std::path::Path::new("assets/outmaps/test-planet")))
            .expect("test planet outmap");
        let centre = mountain_direction();
        let east = centre.cross(DVec3::Y).normalize();

        for transect_km in [60.0_f64, 20.0] {
            let samples = 600_usize;
            let spacing = transect_km * 1000.0 / samples as f64;
            let mut heights = Vec::new();
            let mut biomes: Vec<u8> = Vec::new();
            let mut source_level = 0_u8;
            for index in 0..samples {
                let offset = (index as f64 - samples as f64 * 0.5) * spacing;
                let direction = (centre + east * (offset / PLANET_RADIUS_METERS)).normalize();
                let Some((face, face_uv)) = crate::terrain::cube_face_uv_for_survey(direction)
                else {
                    continue;
                };
                // Walk down to the finest tile that actually exists here.
                let mut level = 18_u8;
                let height = loop {
                    let tiles_per_side = 1_u32 << level;
                    let x = ((face_uv[0] * 0.5 + 0.5) * f64::from(tiles_per_side)) as u32;
                    let y = ((face_uv[1] * 0.5 + 0.5) * f64::from(tiles_per_side)) as u32;
                    let key = TileKey {
                        face,
                        level,
                        x: x.min(tiles_per_side - 1),
                        y: y.min(tiles_per_side - 1),
                    };
                    if let Ok(resolved) = outmap.resolve_tile(key) {
                        if resolved.level == level {
                            if let Ok(data) = outmap.load_tile(key) {
                                source_level = level;
                                let tiles_per_side = f64::from(1_u32 << key.level);
                                let u =
                                    (face_uv[0] * 0.5 + 0.5) * tiles_per_side - f64::from(key.x);
                                let v =
                                    (face_uv[1] * 0.5 + 0.5) * tiles_per_side - f64::from(key.y);
                                let logical =
                                    catinthegarden_coretypes::TILE_LOGICAL_SIZE as f64 - 1.0;
                                let sx = (u.clamp(0.0, 1.0) * logical).round() as usize;
                                let sy = (v.clamp(0.0, 1.0) * logical).round() as usize;
                                let stored = catinthegarden_coretypes::TILE_STORED_SIZE as usize;
                                let gutter = catinthegarden_coretypes::TILE_GUTTER as usize;
                                biomes.push(data.biome_ids[(sy + gutter) * stored + sx + gutter]);
                                break sample_tile_height(&data, key, face_uv);
                            }
                        }
                    }
                    if level == 0 {
                        break 0.0;
                    }
                    level -= 1;
                };
                heights.push(height);
            }

            let span = heights.iter().cloned().fold(f64::MIN, f64::max)
                - heights.iter().cloned().fold(f64::MAX, f64::min);
            let mut grades: Vec<f64> = heights
                .windows(2)
                .map(|pair| ((pair[1] - pair[0]) / spacing).abs().atan().to_degrees())
                .collect();
            grades.sort_by(f64::total_cmp);
            println!(
                "\n== baked macro, {transect_km:.0}km transect at {spacing:.0}m, source L{source_level}"
            );
            println!(
                "   height {:.0}..{:.0} m, relief {span:.0} m",
                heights.iter().cloned().fold(f64::MAX, f64::min),
                heights.iter().cloned().fold(f64::MIN, f64::max),
            );
            let mut histogram = std::collections::BTreeMap::new();
            for biome in &biomes {
                *histogram.entry(*biome).or_insert(0_usize) += 1;
            }
            println!("   biome histogram (id: count) {histogram:?}");
            println!(
                "   slope deg  p50 {:5.2}  p90 {:5.2}  max {:5.2}",
                percentile(&grades, 0.50),
                percentile(&grades, 0.90),
                percentile(&grades, 1.00),
            );
        }
    }

    fn sample_tile_height(
        data: &crate::outmap::TileData,
        key: catinthegarden_coretypes::TileKey,
        face_uv: [f64; 2],
    ) -> f64 {
        let tiles_per_side = f64::from(1_u32 << key.level);
        let u = (face_uv[0] * 0.5 + 0.5) * tiles_per_side - f64::from(key.x);
        let v = (face_uv[1] * 0.5 + 0.5) * tiles_per_side - f64::from(key.y);
        let logical = catinthegarden_coretypes::TILE_LOGICAL_SIZE as f64 - 1.0;
        let sx = (u.clamp(0.0, 1.0) * logical).round() as usize;
        let sy = (v.clamp(0.0, 1.0) * logical).round() as usize;
        let stored = catinthegarden_coretypes::TILE_STORED_SIZE as usize;
        let gutter = catinthegarden_coretypes::TILE_GUTTER as usize;
        f64::from(data.heights_meters[(sy + gutter) * stored + sx + gutter])
    }

    /// Re-runs the global-summit measurement whenever the active macro source
    /// changes. The standard prominence of the planet's highest summit is its
    /// elevation above the sea-level key col.
    #[test]
    #[ignore = "instrument: cargo test -- --ignored --nocapture global_highest_summit"]
    fn global_highest_summit() {
        use catinthegarden_coretypes::{
            TILE_GUTTER, TILE_LOGICAL_SIZE, TILE_STORED_SIZE, TileKey, face_uv_to_direction,
        };

        #[derive(Clone, Copy)]
        struct Cell {
            key: TileKey,
            sample_x: usize,
            sample_y: usize,
            heights: [f64; 4],
            upper_bound: f64,
        }

        fn sample_direction(cell: Cell, u: f64, v: f64) -> DVec3 {
            let side = f64::from(1_u32 << cell.key.level);
            let logical_quads = f64::from(TILE_LOGICAL_SIZE - 1);
            let face_u =
                ((f64::from(cell.key.x) + (cell.sample_x as f64 + u) / logical_quads) / side) * 2.0
                    - 1.0;
            let face_v =
                ((f64::from(cell.key.y) + (cell.sample_y as f64 + v) / logical_quads) / side) * 2.0
                    - 1.0;
            face_uv_to_direction(cell.key.face, face_u, face_v)
        }

        fn sample_surface(cell: Cell, u: f64, v: f64, spacing: f64) -> (f64, f64, DVec3) {
            let lower = cell.heights[0] + (cell.heights[1] - cell.heights[0]) * u;
            let upper = cell.heights[2] + (cell.heights[3] - cell.heights[2]) * u;
            let raw = lower + (upper - lower) * v;
            let direction = sample_direction(cell, u, v);
            let macro_height = scaled_outmap_macro_height_meters(raw, 152.4);
            let height = macro_height + terrain_detail_meters(direction, spacing, macro_height);
            (height, raw, direction)
        }

        fn refine(cell: Cell, spacing: f64) -> (f64, f64, DVec3) {
            let divisions = 16_usize;
            let mut seeds = Vec::with_capacity((divisions + 1) * (divisions + 1));
            for y in 0..=divisions {
                for x in 0..=divisions {
                    let u = x as f64 / divisions as f64;
                    let v = y as f64 / divisions as f64;
                    let sample = sample_surface(cell, u, v, spacing);
                    seeds.push((sample.0, u, v, sample.1, sample.2));
                }
            }
            seeds.sort_by(|a, b| b.0.total_cmp(&a.0));
            seeds.truncate(8);

            let mut best = (f64::NEG_INFINITY, 0.0, DVec3::X);
            for (_, mut u, mut v, _, _) in seeds {
                let mut step = 1.0 / divisions as f64;
                while step * spacing > 0.5 {
                    let mut local = sample_surface(cell, u, v, spacing);
                    let mut local_uv = (u, v);
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let candidate_u = (u + f64::from(dx) * step).clamp(0.0, 1.0);
                            let candidate_v = (v + f64::from(dy) * step).clamp(0.0, 1.0);
                            let candidate = sample_surface(cell, candidate_u, candidate_v, spacing);
                            if candidate.0 > local.0 {
                                local = candidate;
                                local_uv = (candidate_u, candidate_v);
                            }
                        }
                    }
                    (u, v) = local_uv;
                    step *= 0.5;
                }
                let candidate = sample_surface(cell, u, v, spacing);
                if candidate.0 > best.0 {
                    best = candidate;
                }
            }
            best
        }

        let outmap = crate::outmap::Outmap::open("../../assets/outmaps/test-planet")
            .or_else(|_| crate::outmap::Outmap::open("assets/outmaps/test-planet"))
            .expect("active test planet outmap");
        let dense_level = outmap.manifest().dense_level;
        let keys: Vec<_> = outmap
            .manifest()
            .available_tiles
            .iter()
            .copied()
            .filter(|key| key.level == dense_level)
            .collect();
        let stored = TILE_STORED_SIZE as usize;
        let gutter = TILE_GUTTER as usize;
        let logical = TILE_LOGICAL_SIZE as usize;
        let spacing = baked_sample_spacing_meters(dense_level);

        let mut raw_maximum = (f64::NEG_INFINITY, DVec3::X);
        for &key in &keys {
            let tile = outmap.load_tile(key).expect("dense tile");
            for y in 0..logical {
                for x in 0..logical {
                    let raw = f64::from(tile.heights_meters[(y + gutter) * stored + x + gutter]);
                    if raw > raw_maximum.0 {
                        let cell = Cell {
                            key,
                            sample_x: x.min(logical - 2),
                            sample_y: y.min(logical - 2),
                            heights: [raw; 4],
                            upper_bound: 0.0,
                        };
                        raw_maximum = (
                            raw,
                            sample_direction(
                                cell,
                                if x == logical - 1 { 1.0 } else { 0.0 },
                                if y == logical - 1 { 1.0 } else { 0.0 },
                            ),
                        );
                    }
                }
            }
        }

        let raw_macro = scaled_outmap_macro_height_meters(raw_maximum.0, 152.4);
        let mut best = (
            raw_macro + terrain_detail_meters(raw_maximum.1, spacing, raw_macro),
            raw_maximum.0,
            raw_maximum.1,
        );
        let mut candidates = Vec::new();
        for &key in &keys {
            let tile = outmap.load_tile(key).expect("dense tile");
            let height = |x: usize, y: usize| {
                f64::from(tile.heights_meters[(y + gutter) * stored + x + gutter])
            };
            for y in 0..logical - 1 {
                for x in 0..logical - 1 {
                    let heights = [
                        height(x, y),
                        height(x + 1, y),
                        height(x, y + 1),
                        height(x + 1, y + 1),
                    ];
                    let maximum = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let upper_bound = scaled_outmap_macro_height_meters(maximum, 152.4)
                        + TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS;
                    if upper_bound > best.0 {
                        candidates.push(Cell {
                            key,
                            sample_x: x,
                            sample_y: y,
                            heights,
                            upper_bound,
                        });
                    }
                }
            }
        }
        candidates.sort_by(|a, b| b.upper_bound.total_cmp(&a.upper_bound));
        let candidate_count = candidates.len();
        let mut refined_count = 0_usize;
        for candidate in candidates {
            if candidate.upper_bound <= best.0 {
                break;
            }
            let refined = refine(candidate, spacing);
            refined_count += 1;
            if refined.0 > best.0 {
                best = refined;
            }
        }

        println!("\n== global highest summit");
        println!("   dense tiles scanned: {} at L{dense_level}", keys.len());
        println!("   candidate cells: {candidate_count}; refined: {refined_count}");
        println!("   highest raw L4 macro sample: {:.9}m", raw_maximum.0);
        println!("   summit / prominence ASL: {:.9}m", best.0);
        println!("   latitude: {:.12} deg", best.2.y.asin().to_degrees());
        println!(
            "   longitude: {:.12} deg",
            best.2.z.atan2(best.2.x).to_degrees()
        );
        println!(
            "   direction: [{:.15}, {:.15}, {:.15}]",
            best.2.x, best.2.y, best.2.z
        );
        println!("   raw macro at summit: {:.9}m", best.1);
    }

    #[test]
    #[ignore = "instrument, not an assertion: cargo test -- --ignored --nocapture mountain_relief"]
    fn mountain_relief_as_it_stands() {
        println!(
            "\nladder start wavelength {TERRAIN_DETAIL_START_WAVELENGTH_METERS} m, roughness {}",
            crate::planet::TERRAIN_DETAIL_ROUGHNESS
        );
        // Massif scale: does a Ben Nevis exist in here at all?
        survey("massif scale", 20.0, 1_500);
        // Hillwalking scale: what the ground under the camera does.
        survey("near scale", 2.0, 4_000);

        println!("\nreal hills, for scale:");
        println!("   Ben Nevis   1345m summit, ~1200m of relief within 2km of the summit");
        println!("   Yr Wyddfa   1085m summit, ~700m of relief within 1.5km (Clogwyn face)");
        println!("   Cairn Gorm  1245m summit, ~300m of relief within 2km on the plateau side");
    }
}

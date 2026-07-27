//! Temporary instrument: what the detail ladder actually builds at the
//! mountains, in the terms a hill is described in.
//!
//! Delete once the mountain character question is settled.

#[cfg(test)]
mod tests {
    use crate::planet::{
        PLANET_RADIUS_METERS, TERRAIN_DETAIL_START_WAVELENGTH_METERS, terrain_detail_meters,
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

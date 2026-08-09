use std::{env, path::PathBuf, process::ExitCode};

use catinthegarden_baker::{
    BakeConfig, BakeProgress, bake_with_progress, refine_existing_outmap_with_progress,
    sparse_radius_for_level, validate_output_with_progress,
};
use catinthegarden_coretypes::{PLANET_RADIUS_METERS, TILE_LOGICAL_SIZE};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("baker error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        print_help();
        return Ok(());
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--validate")
    {
        let output = arguments
            .get(index + 1)
            .ok_or("--validate requires an output directory")?;
        let mut progress = BakeProgress::new();
        let manifest =
            validate_output_with_progress(PathBuf::from(output).as_path(), &mut progress)?;
        println!(
            "validated {} tiles in schema {}",
            manifest.available_tiles.len(),
            manifest.schema_version
        );
        return Ok(());
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--refine-existing")
    {
        let output = arguments
            .get(index + 1)
            .ok_or("--refine-existing requires an outmap directory")?;
        let mut progress = BakeProgress::new();
        let manifest =
            refine_existing_outmap_with_progress(PathBuf::from(output).as_path(), &mut progress)?;
        println!(
            "refined and validated {} tiles at {}",
            manifest.available_tiles.len(),
            output
        );
        return Ok(());
    }

    let config = parse_config(&arguments)?;
    let report_mountain_coverage = arguments
        .iter()
        .any(|argument| argument == "--mountain-coverage");
    if let Some(path) = &config.etopo {
        println!(
            "baking {}x{} ETOPO grid, dense L{} + sparse L{}",
            config.width, config.height, config.dense_level, config.max_level
        );
        println!(
            "macro source: NOAA ETOPO 2022 Ice Surface ({})",
            path.display()
        );
    } else if !config.procedural_terrain {
        println!(
            "baking {}x{} grid, {} erosion iterations, dense L{} + sparse L{}",
            config.width,
            config.height,
            config.erosion_iterations,
            config.dense_level,
            config.max_level
        );
        println!("macro source: authored Earth-like generator");
    }
    if config.game_terrain {
        println!("relief profile: game terrain (amplified land + dense mountain ridges)");
    }
    if config.zoomed_terrain {
        println!("relief profile: zoomed game terrain (compact Himalaya window repeated globally)");
    }
    if config.procedural_terrain {
        println!("macro source: procedural continents + mountain regions + erosion");
    }
    print_sparse_coverage(&config);
    let mut progress = BakeProgress::new();
    let (manifest, mountain_coverage) =
        bake_with_progress(&config, report_mountain_coverage, &mut progress)?;
    let [landing_x, landing_y, landing_z] = manifest.sparse_landing_direction;
    println!("selected dry coastal sparse centre [{landing_x:.6}, {landing_y:.6}, {landing_z:.6}]");
    println!(
        "wrote and validated {} tiles plus previews at {}",
        manifest.available_tiles.len(),
        config.output.display()
    );
    if report_mountain_coverage {
        println!(
            "mountain coverage: {:.2}% of area-weighted land positions pass; {}/{} positions, {} qualifying rays",
            mountain_coverage.coverage_percent(),
            mountain_coverage.passing_positions,
            mountain_coverage.land_positions,
            mountain_coverage.qualifying_rays,
        );
    }
    Ok(())
}

fn print_sparse_coverage(config: &BakeConfig) {
    println!("sparse source coverage (approximate face-centre widths):");
    for level in config.dense_level.saturating_add(1)..=config.max_level {
        let radius = sparse_radius_for_level(config, level);
        let tile_width_meters =
            PLANET_RADIUS_METERS * std::f64::consts::FRAC_PI_2 / f64::from(1_u32 << level);
        let coverage_width_meters = tile_width_meters * f64::from(radius * 2 + 1);
        let sample_spacing_meters =
            tile_width_meters / f64::from(TILE_LOGICAL_SIZE.saturating_sub(1));
        println!(
            "  L{level:02}: radius {radius:>2}, coverage {coverage_width_meters:>9.1}m, sample spacing {sample_spacing_meters:>7.3}m"
        );
    }
}

fn parse_config(arguments: &[String]) -> Result<BakeConfig, String> {
    let mut config = BakeConfig::default();
    let mut index = 0;
    let mut positional_output_seen = false;
    while index < arguments.len() {
        let argument = &arguments[index];
        if !argument.starts_with('-') {
            if positional_output_seen {
                return Err(format!("unexpected positional argument '{argument}'"));
            }
            config.output = PathBuf::from(argument);
            positional_output_seen = true;
            index += 1;
            continue;
        }
        match argument.as_str() {
            "--quick" => {
                let output = config.output.clone();
                config = BakeConfig::quick(output);
                index += 1;
            }
            "--output" => {
                config.output = PathBuf::from(value(arguments, index, argument)?);
                index += 2;
            }
            "--etopo" => {
                config.etopo = Some(PathBuf::from(value(arguments, index, argument)?));
                index += 2;
            }
            "--game-terrain" => {
                config.game_terrain = true;
                index += 1;
            }
            "--zoomed-terrain" => {
                config.zoomed_terrain = true;
                config.game_terrain = true;
                index += 1;
            }
            "--procedural-terrain" => {
                config.procedural_terrain = true;
                config.game_terrain = true;
                index += 1;
            }
            "--mountain-coverage" => {
                index += 1;
            }
            "--seed" => {
                config.seed = parse_u32(value(arguments, index, argument)?)?;
                index += 2;
            }
            "--width" => {
                config.width = parse(value(arguments, index, argument)?, argument)?;
                index += 2;
            }
            "--height" => {
                config.height = parse(value(arguments, index, argument)?, argument)?;
                index += 2;
            }
            "--dense-level" => {
                config.dense_level = parse(value(arguments, index, argument)?, argument)?;
                index += 2;
            }
            "--max-level" => {
                config.max_level = parse(value(arguments, index, argument)?, argument)?;
                index += 2;
            }
            "--sparse-radius" => {
                config.sparse_radius = Some(parse(value(arguments, index, argument)?, argument)?);
                index += 2;
            }
            "--erosion-iterations" => {
                config.erosion_iterations = parse(value(arguments, index, argument)?, argument)?;
                index += 2;
            }
            _ => return Err(format!("unrecognized argument '{argument}'")),
        }
    }
    config.validate()?;
    Ok(config)
}

fn value<'a>(arguments: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    arguments
        .get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse<T: std::str::FromStr>(value: &str, flag: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value '{value}' for {flag}"))
}

fn parse_u32(value: &str) -> Result<u32, String> {
    if let Some(hex) = value.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).map_err(|_| format!("invalid seed '{value}'"))
    } else {
        parse(value, "--seed")
    }
}

fn print_help() {
    println!(
        "catinthegarden-baker [OUTPUT] [OPTIONS]\n\
         \n\
         Options:\n\
           --output PATH              Output root (default assets/outmaps/test-planet)\n\
           --etopo PATH               NOAA ETOPO 2022 Ice Surface GeoTIFF macro source\n\
           --game-terrain              Amplify land and add dense baked mountain ridges\n\
           --zoomed-terrain             Repeat a compact mountain-rich source window globally\n\
           --procedural-terrain         Generate continents/mountains then erode them\n\
           --mountain-coverage          Report area-weighted 8-direction mountain coverage\n\
           --seed N                   Decimal or 0x-prefixed deterministic seed\n\
           --width N                  Working equirectangular grid width\n\
           --height N                 Working grid height\n\
           --dense-level N            Highest globally dense quadtree level\n\
           --max-level N              Sparse coastal refinement depth (maximum 18)\n\
           --sparse-radius N          Constant tile radius (default: adaptive coverage)\n\
           --erosion-iterations N     Authored-source hydraulic iteration count\n\
           --quick                    Small deterministic development bake\n\
           --validate PATH            Validate an existing outmap and exit\n\
           --refine-existing PATH     Expand sparse detail from existing dense macro tiles\n\
           -h, --help                 Show this help"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_small_configuration() {
        let arguments = [
            "--quick".to_owned(),
            "--output".to_owned(),
            "/tmp/outmap".to_owned(),
            "--max-level".to_owned(),
            "6".to_owned(),
        ];
        let config = parse_config(&arguments).unwrap();
        assert_eq!(config.output, PathBuf::from("/tmp/outmap"));
        assert_eq!(config.width, 64);
        assert_eq!(config.max_level, 6);
    }

    #[test]
    fn parses_etopo_source() {
        let arguments = ["--etopo".to_owned(), "/tmp/etopo.tif".to_owned()];
        let config = parse_config(&arguments).unwrap();
        assert_eq!(config.etopo, Some(PathBuf::from("/tmp/etopo.tif")));
    }

    #[test]
    fn parses_game_terrain_profile() {
        let arguments = ["--game-terrain".to_owned()];
        let config = parse_config(&arguments).unwrap();
        assert!(config.game_terrain);
    }

    #[test]
    fn parses_zoomed_terrain_profile_and_enables_game_relief() {
        let arguments = ["--zoomed-terrain".to_owned()];
        let config = parse_config(&arguments).unwrap();
        assert!(config.zoomed_terrain);
        assert!(config.game_terrain);
    }

    #[test]
    fn parses_procedural_terrain_profile_and_enables_game_relief() {
        let arguments = ["--procedural-terrain".to_owned()];
        let config = parse_config(&arguments).unwrap();
        assert!(config.procedural_terrain);
        assert!(config.game_terrain);
    }
}

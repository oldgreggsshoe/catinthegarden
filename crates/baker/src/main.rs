use std::{env, path::PathBuf, process::ExitCode};

use catinthegarden_baker::{BakeConfig, bake, validate_output};

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
        let manifest = validate_output(PathBuf::from(output).as_path())?;
        println!(
            "validated {} tiles in schema {}",
            manifest.available_tiles.len(),
            manifest.schema_version
        );
        return Ok(());
    }

    let config = parse_config(&arguments)?;
    println!(
        "baking {}x{} grid, {} erosion iterations, dense L{} + sparse L{}",
        config.width,
        config.height,
        config.erosion_iterations,
        config.dense_level,
        config.max_level
    );
    let manifest = bake(&config)?;
    println!(
        "wrote and validated {} tiles plus previews at {}",
        manifest.available_tiles.len(),
        config.output.display()
    );
    Ok(())
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
                config.sparse_radius = parse(value(arguments, index, argument)?, argument)?;
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
           --seed N                   Decimal or 0x-prefixed deterministic seed\n\
           --width N                  Working equirectangular grid width\n\
           --height N                 Working grid height\n\
           --dense-level N            Highest globally dense quadtree level\n\
           --max-level N              Sparse +X refinement depth (maximum 18)\n\
           --sparse-radius N          Tile radius around the +X landing area\n\
           --erosion-iterations N     Hydraulic iteration count\n\
           --quick                    Small deterministic development bake\n\
           --validate PATH            Validate an existing outmap and exit\n\
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
}

//! Offline terrain baker. Runtime code never generates terrain.

use std::{env, fs, path::PathBuf};

use image::{Rgb, RgbImage};
use noise::{NoiseFn, Perlin};
use rayon::prelude::*;

const PREVIEW_WIDTH: u32 = 1024;
const PREVIEW_HEIGHT: u32 = 512;
const MAX_MOUNTAIN_HEIGHT_METERS: f64 = 9_000.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/outmaps/test-planet/previews"));
    fs::create_dir_all(&output)?;

    let base = Perlin::new(0xC47_1A);
    let warp = Perlin::new(0xD0A1_1A);
    let mountain = Perlin::new(0xBEEF_9000);
    let rows: Vec<Vec<[u8; 3]>> = (0..PREVIEW_HEIGHT)
        .into_par_iter()
        .map(|y| {
            (0..PREVIEW_WIDTH)
                .map(|x| preview_pixel(x, y, &base, &warp, &mountain))
                .collect()
        })
        .collect();

    let mut height_preview = RgbImage::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
    let mut biome_preview = RgbImage::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
    let mut moisture_preview = RgbImage::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
    for (y, row) in rows.iter().enumerate() {
        for (x, pixel) in row.iter().enumerate() {
            height_preview.put_pixel(x as u32, y as u32, Rgb(*pixel));
            let elevation = f64::from(pixel[0]) / 255.0 * 2.0 - 1.0;
            let latitude = y as f64 / (PREVIEW_HEIGHT - 1) as f64 * std::f64::consts::PI
                - std::f64::consts::FRAC_PI_2;
            biome_preview.put_pixel(x as u32, y as u32, Rgb(biome_color(elevation, latitude)));
            let moisture = ((1.0 - elevation.abs()) * 255.0) as u8;
            moisture_preview.put_pixel(x as u32, y as u32, Rgb([moisture; 3]));
        }
    }
    height_preview.save(output.join("height.png"))?;
    biome_preview.save(output.join("biome.png"))?;
    moisture_preview.save(output.join("moisture.png"))?;
    println!("wrote previews to {}", output.display());
    Ok(())
}

fn preview_pixel(x: u32, y: u32, base: &Perlin, warp: &Perlin, mountain: &Perlin) -> [u8; 3] {
    let longitude = x as f64 / PREVIEW_WIDTH as f64 * std::f64::consts::TAU - std::f64::consts::PI;
    let latitude =
        y as f64 / (PREVIEW_HEIGHT - 1) as f64 * std::f64::consts::PI - std::f64::consts::FRAC_PI_2;
    let direction = [
        latitude.cos() * longitude.cos(),
        latitude.sin(),
        latitude.cos() * longitude.sin(),
    ];
    let warped = [
        direction[0]
            + warp.get([direction[0] * 0.7, direction[1] * 0.7, direction[2] * 0.7]) * 0.18,
        direction[1]
            + warp.get([direction[2] * 0.7, direction[0] * 0.7, direction[1] * 0.7]) * 0.18,
        direction[2]
            + warp.get([direction[1] * 0.7, direction[2] * 0.7, direction[0] * 0.7]) * 0.18,
    ];
    let continent = fbm(base, warped, 0.9, 6);
    let ridged = 1.0
        - mountain
            .get([warped[0] * 4.0, warped[1] * 4.0, warped[2] * 4.0])
            .abs();
    let elevation_meters = (continent * 2_200.0 + ridged.powi(3) * 7_000.0)
        .clamp(-5_000.0, MAX_MOUNTAIN_HEIGHT_METERS);
    let normalized = ((elevation_meters + 5_000.0) / 14_000.0 * 255.0) as u8;
    [normalized; 3]
}

fn fbm(noise: &Perlin, point: [f64; 3], frequency: f64, octaves: u32) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut total_amplitude = 0.0;
    let mut frequency = frequency;
    for _ in 0..octaves {
        value += noise.get([
            point[0] * frequency,
            point[1] * frequency,
            point[2] * frequency,
        ]) * amplitude;
        total_amplitude += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    value / total_amplitude
}

fn biome_color(elevation: f64, latitude: f64) -> [u8; 3] {
    if latitude.abs() > 66.0_f64.to_radians() || elevation > 0.8 {
        [230, 240, 245]
    } else if elevation < 0.36 {
        [20, 65, 150]
    } else {
        [50, 130, 65]
    }
}

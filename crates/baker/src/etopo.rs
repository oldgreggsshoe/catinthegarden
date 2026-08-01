use std::{
    fs::File,
    io::{BufReader, Error, ErrorKind},
    path::Path,
};

use rayon::prelude::*;
use tiff::{
    ColorType,
    decoder::{Decoder, DecodingResult, Limits},
};

use crate::BakeResult;

const MAX_SOURCE_PIXELS: usize = 300_000_000;
const PEAK_RETENTION_START_METERS: f64 = 4_000.0;
const PEAK_RETENTION_END_METERS: f64 = 6_000.0;

enum ElevationSamples {
    I16(Vec<i16>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl ElevationSamples {
    fn len(&self) -> usize {
        match self {
            Self::I16(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    fn get(&self, index: usize) -> f64 {
        match self {
            Self::I16(values) => f64::from(values[index]),
            Self::I32(values) => f64::from(values[index]),
            Self::F32(values) => f64::from(values[index]),
            Self::F64(values) => values[index],
        }
    }
}

/// Loads a north-up, whole-world NOAA ETOPO 2022 Ice Surface GeoTIFF and
/// bilinearly resamples its pixel-centred geographic grid onto the baker's
/// south-up pixel-centred working grid. Longitude wraps at the dateline;
/// latitude clamps at the poles.
pub fn load_etopo(path: &Path, target_width: usize, target_height: usize) -> BakeResult<Vec<f64>> {
    let file = File::open(path)?;
    let mut decoder = Decoder::new(BufReader::new(file))?;
    let (width, height) = decoder.dimensions()?;
    let source_width = width as usize;
    let source_height = height as usize;
    let source_pixels = source_width
        .checked_mul(source_height)
        .ok_or_else(|| invalid_data("ETOPO dimensions overflow"))?;
    if source_width < 8 || source_height < 4 || source_width != source_height * 2 {
        return Err(invalid_data(format!(
            "ETOPO must be a whole-world 2:1 grid, got {width}x{height}"
        )));
    }
    if source_pixels > MAX_SOURCE_PIXELS {
        return Err(invalid_data(format!(
            "ETOPO grid has {source_pixels} pixels, limit is {MAX_SOURCE_PIXELS}"
        )));
    }
    if !matches!(decoder.colortype()?, ColorType::Gray(_)) {
        return Err(invalid_data(
            "ETOPO must contain one grayscale elevation band",
        ));
    }
    let mut limits = Limits::default();
    limits.decoding_buffer_size = source_pixels
        .checked_mul(size_of::<f64>())
        .ok_or_else(|| invalid_data("ETOPO decoding limit overflow"))?;
    let samples = match decoder.with_limits(limits).read_image()? {
        DecodingResult::I16(values) => ElevationSamples::I16(values),
        DecodingResult::I32(values) => ElevationSamples::I32(values),
        DecodingResult::F32(values) => ElevationSamples::F32(values),
        DecodingResult::F64(values) => ElevationSamples::F64(values),
        _ => {
            return Err(invalid_data(
                "ETOPO elevation samples must be signed integer or float",
            ));
        }
    };
    if samples.len() != source_pixels {
        return Err(invalid_data(format!(
            "ETOPO decoded {} samples, expected {source_pixels}",
            samples.len()
        )));
    }
    let (minimum, maximum, all_finite) = (0..samples.len())
        .into_par_iter()
        .map(|index| samples.get(index))
        .fold(
            || (f64::INFINITY, f64::NEG_INFINITY, true),
            |(minimum, maximum, all_finite), value| {
                (
                    minimum.min(value),
                    maximum.max(value),
                    all_finite && value.is_finite(),
                )
            },
        )
        .reduce(
            || (f64::INFINITY, f64::NEG_INFINITY, true),
            |a, b| (a.0.min(b.0), a.1.max(b.1), a.2 && b.2),
        );
    if !all_finite || minimum >= 0.0 || maximum <= 0.0 {
        return Err(invalid_data(format!(
            "ETOPO must contain finite ocean and land elevations, got {minimum}..{maximum}m"
        )));
    }

    let target_pixels = target_width
        .checked_mul(target_height)
        .ok_or_else(|| invalid_data("target grid dimensions overflow"))?;
    let heights = (0..target_pixels)
        .into_par_iter()
        .map(|index| {
            let x = index % target_width;
            let y = index / target_width;
            let source_x = (x as f64 + 0.5) / target_width as f64 * source_width as f64 - 0.5;
            let northward = 1.0 - (y as f64 + 0.5) / target_height as f64;
            let source_y = northward * source_height as f64 - 0.5;
            let centre = bilinear(&samples, source_width, source_height, source_x, source_y);
            if centre > 0.0 && source_width > target_width {
                // A point sample can miss an entire summit when the 60 arc-second
                // source is reduced to the working grid (about five source cells
                // per axis at 4096x2048). Keep ordinary hills and the coastline
                // tied to the bilinear centre sample: max-filtering all positive
                // land creates kilometre-wide terraces. Only the highest ranges
                // fade toward the observed footprint maximum, retaining narrow
                // summits without turning low terrain into plateaus.
                let peak = land_peak_in_target_cell(
                    &samples,
                    source_width,
                    source_height,
                    target_width,
                    target_height,
                    x,
                    y,
                );
                let weight =
                    smoothstep(PEAK_RETENTION_START_METERS, PEAK_RETENTION_END_METERS, peak);
                centre + (peak - centre).max(0.0) * weight
            } else {
                centre
            }
        })
        .collect();
    Ok(heights)
}

#[allow(clippy::too_many_arguments)]
fn land_peak_in_target_cell(
    samples: &ElevationSamples,
    source_width: usize,
    source_height: usize,
    target_width: usize,
    target_height: usize,
    target_x: usize,
    target_y: usize,
) -> f64 {
    let x0 = source_cell_start(target_x, target_width, source_width);
    let x1 = source_cell_end(target_x, target_width, source_width);
    // Source rows run north to south, while target rows run south to north.
    let source_north_cell = target_height - 1 - target_y;
    let y0 = source_cell_start(source_north_cell, target_height, source_height);
    let y1 = source_cell_end(source_north_cell, target_height, source_height);
    let mut peak = f64::NEG_INFINITY;
    for source_y in y0..=y1 {
        for source_x in x0..=x1 {
            peak = peak.max(samples.get(source_y * source_width + source_x));
        }
    }
    peak
}

fn source_cell_start(target: usize, target_size: usize, source_size: usize) -> usize {
    ((target as f64 / target_size as f64 * source_size as f64 - 0.5).ceil() as isize)
        .clamp(0, source_size as isize - 1) as usize
}

fn source_cell_end(target: usize, target_size: usize, source_size: usize) -> usize {
    ((((target + 1) as f64 / target_size as f64 * source_size as f64 - 0.5).floor()) as isize)
        .clamp(0, source_size as isize - 1) as usize
}

fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
    let amount = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    amount * amount * (3.0 - 2.0 * amount)
}

fn bilinear(samples: &ElevationSamples, width: usize, height: usize, x: f64, y: f64) -> f64 {
    let floor_x = x.floor();
    let x0 = (floor_x as isize).rem_euclid(width as isize) as usize;
    let x1 = (x0 + 1) % width;
    let clamped_y = y.clamp(0.0, height.saturating_sub(1) as f64);
    let y0 = clamped_y.floor() as usize;
    let y1 = (y0 + 1).min(height - 1);
    let tx = x - floor_x;
    let ty = clamped_y - y0 as f64;
    let top = samples.get(y0 * width + x0) * (1.0 - tx) + samples.get(y0 * width + x1) * tx;
    let bottom = samples.get(y1 * width + x0) * (1.0 - tx) + samples.get(y1 * width + x1) * tx;
    top * (1.0 - ty) + bottom * ty
}

fn invalid_data(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(Error::new(ErrorKind::InvalidData, message.into()))
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tiff::encoder::{TiffEncoder, colortype::GrayI16};

    use super::*;

    #[test]
    fn imports_north_up_signed_relief_into_south_up_grid() {
        let path = std::env::temp_dir().join(format!(
            "catinthegarden-etopo-{}-orientation.tif",
            std::process::id()
        ));
        let width = 8_u32;
        let height = 4_u32;
        let values: Vec<i16> = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    let latitude_band = 1_000 - y as i16 * 700;
                    latitude_band + x as i16 * 10
                })
            })
            .collect();
        let file = File::create(&path).unwrap();
        TiffEncoder::new(file)
            .unwrap()
            .write_image::<GrayI16>(width, height, &values)
            .unwrap();

        let imported = load_etopo(&path, width as usize, height as usize).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(imported[0], f64::from(values[3 * width as usize]));
        assert_eq!(imported[3 * width as usize], f64::from(values[0]));
        assert_eq!(
            imported[width as usize - 1],
            f64::from(values[4 * width as usize - 1])
        );
    }

    #[test]
    fn preserves_observed_land_peak_when_downsampling() {
        let path = std::env::temp_dir().join(format!(
            "catinthegarden-etopo-{}-peak.tif",
            std::process::id()
        ));
        let width = 16_u32;
        let height = 8_u32;
        let mut values = vec![-1_000_i16; width as usize * height as usize];
        for y in 2..=3 {
            for x in 4..=5 {
                values[y * width as usize + x] = 1_000;
            }
        }
        values[2 * width as usize + 4] = 8_000;
        for y in 2..=3 {
            for x in 8..=9 {
                values[y * width as usize + x] = 1_000;
            }
        }
        values[2 * width as usize + 8] = 3_000;
        let file = File::create(&path).unwrap();
        TiffEncoder::new(file)
            .unwrap()
            .write_image::<GrayI16>(width, height, &values)
            .unwrap();

        let imported = load_etopo(&path, 8, 4).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            imported.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            8_000.0
        );
        assert_eq!(imported[2 * 8 + 4], 1_500.0);
    }
}

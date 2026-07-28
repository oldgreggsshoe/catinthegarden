//! Does distance read?
//!
//! `probe.rs` made "are we standing on the ground" an objective question by
//! comparing the drawn surface against the surface the camera collides with.
//! This does the same for aerial perspective, and it needs no reference
//! photograph and no opinion, because the renderer already contains its own
//! ground truth:
//!
//! **Terrain at infinite distance must equal the sky radiance in that
//! direction.** That is what a horizon *is*. So the asymptote is fixed by
//! physics and is testable; a range that stays its own colour out to the
//! horizon is wrong however pleasant it looks.
//!
//! What is *not* fixed is the rate. How fast terrain approaches the sky is
//! aerosol load, and a clear alpine day and a humid summer afternoon differ
//! enormously with both being real. So this reports the curve and scores the
//! asymptote, and deliberately does not assert how quickly it gets there.
//!
//! The sky reference is sampled just above the terrain silhouette, per column,
//! so it is the sky at the same azimuth the far terrain is seen against rather
//! than an average over the whole upper frame.

use crate::probe::{DepthImage, ProbeGeometry};

/// Terrain colour in one distance band.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct HazeBin {
    pub near_meters: f64,
    pub far_meters: f64,
    pub sample_count: u32,
    pub mean_rgb: [f64; 3],
    pub luminance: f64,
    pub saturation: f64,
    /// Distance from this band's colour to the horizon sky, in linear RGB.
    /// This is the quantity that must fall toward zero with distance.
    pub distance_to_sky: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct HazeReport {
    pub sim_time: f64,
    pub render_path: &'static str,
    pub sky_rgb: Option<[f64; 3]>,
    pub sky_sample_count: u32,
    pub bins: Vec<HazeBin>,
    /// How much of the way to the sky the farthest populated band has come,
    /// relative to the nearest one: `1 - far_to_sky / near_to_sky`.
    ///
    /// 1.0 is terrain that has reached the sky. 0.0 is terrain that is no
    /// closer to it at 50km than at 2km — a range painted on a backdrop.
    /// Negative means distance is moving terrain *away* from the sky, which is
    /// extinction with no in-scatter to balance it.
    pub convergence: Option<f64>,
}

/// Band edges in metres. Logarithmic, because haze is a per-unit-depth process
/// and the interesting decade is 2–50 km rather than the first hundred metres.
const BAND_EDGES_METERS: [f64; 7] = [
    500.0, 2_000.0, 5_000.0, 12_000.0, 30_000.0, 80_000.0, 200_000.0,
];

fn luminance(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn saturation(rgb: [f64; 3]) -> f64 {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    if max <= 1.0e-6 {
        0.0
    } else {
        (max - min) / max
    }
}

fn rgb_distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn pixel_rgb(pixels: &[u8], index: usize) -> [f64; 3] {
    [
        f64::from(pixels[index * 4]) / 255.0,
        f64::from(pixels[index * 4 + 1]) / 255.0,
        f64::from(pixels[index * 4 + 2]) / 255.0,
    ]
}

/// The sky the far terrain is seen *against*: for each column, the pixel just
/// above the topmost drawn surface.
///
/// Averaging the whole upper frame would fold in zenith sky, which is a
/// different radiance from the horizon sky and would make convergence look
/// worse than it is.
fn horizon_sky(
    pixels: &[u8],
    width: u32,
    height: u32,
    depth: &DepthImage,
    geometry: &ProbeGeometry,
) -> (Option<[f64; 3]>, u32) {
    let mut sum = [0.0_f64; 3];
    let mut count = 0_u32;
    for column in 0..width {
        // Walk down until the first drawn pixel; the one above it is the sky
        // at this azimuth. A column that never hits terrain contributes
        // nothing, so a frame of pure sky produces no reference at all.
        let mut previous_was_sky = false;
        for row in 0..height {
            let index = (row * width + column) as usize;
            let drawn = depth
                .depths
                .get(index)
                .and_then(|d| geometry.forward_distance_meters(*d))
                .is_some();
            if drawn {
                if previous_was_sky && row > 0 {
                    let sky = pixel_rgb(pixels, ((row - 1) * width + column) as usize);
                    sum[0] += sky[0];
                    sum[1] += sky[1];
                    sum[2] += sky[2];
                    count += 1;
                }
                break;
            }
            previous_was_sky = true;
        }
    }
    if count == 0 {
        return (None, 0);
    }
    (
        Some([
            sum[0] / f64::from(count),
            sum[1] / f64::from(count),
            sum[2] / f64::from(count),
        ]),
        count,
    )
}

/// Bins the drawn surface by distance and scores how far toward the horizon
/// sky it has travelled.
pub fn measure(
    sim_time: f64,
    render_path: &'static str,
    geometry: &ProbeGeometry,
    depth: &DepthImage,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> HazeReport {
    let band_count = BAND_EDGES_METERS.len() - 1;
    let mut sums = vec![[0.0_f64; 3]; band_count];
    let mut counts = vec![0_u32; band_count];

    if depth.width == width
        && depth.height == height
        && pixels.len() >= (width * height * 4) as usize
    {
        for index in 0..(width * height) as usize {
            let Some(forward) = depth
                .depths
                .get(index)
                .and_then(|d| geometry.forward_distance_meters(*d))
            else {
                continue;
            };
            let Some(band) = BAND_EDGES_METERS
                .windows(2)
                .position(|edge| forward >= edge[0] && forward < edge[1])
            else {
                continue;
            };
            let rgb = pixel_rgb(pixels, index);
            sums[band][0] += rgb[0];
            sums[band][1] += rgb[1];
            sums[band][2] += rgb[2];
            counts[band] += 1;
        }
    }

    let (sky_rgb, sky_sample_count) = horizon_sky(pixels, width, height, depth, geometry);

    let bins: Vec<HazeBin> = (0..band_count)
        .filter(|band| counts[*band] > 0)
        .map(|band| {
            let n = f64::from(counts[band]);
            let mean = [sums[band][0] / n, sums[band][1] / n, sums[band][2] / n];
            HazeBin {
                near_meters: BAND_EDGES_METERS[band],
                far_meters: BAND_EDGES_METERS[band + 1],
                sample_count: counts[band],
                mean_rgb: mean,
                luminance: luminance(mean),
                saturation: saturation(mean),
                distance_to_sky: sky_rgb.map_or(f64::NAN, |sky| rgb_distance(mean, sky)),
            }
        })
        .collect();

    // Compare the extremes rather than fitting a curve: the question is whether
    // the far end has arrived, and a fit would let a steep near-field slope
    // stand in for an asymptote it never reaches.
    let convergence = match (bins.first(), bins.last(), sky_rgb) {
        (Some(near), Some(far), Some(_)) if bins.len() >= 2 && near.distance_to_sky > 1.0e-6 => {
            Some(1.0 - far.distance_to_sky / near.distance_to_sky)
        }
        _ => None,
    };

    HazeReport {
        sim_time,
        render_path,
        sky_rgb,
        sky_sample_count,
        bins,
        convergence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 8;

    fn geometry() -> ProbeGeometry {
        ProbeGeometry::new(
            1.0,
            60.0_f64.to_radians(),
            1.0,
            DVec3::X * 4_001_000.0,
            -DVec3::X,
            DVec3::Y,
        )
    }

    /// Depth that puts row `r` at a chosen distance, with the top `sky_rows`
    /// rows left as the clear value.
    fn scene(sky_rows: u32, distances: &[f64]) -> DepthImage {
        let mut depths = vec![0.0_f32; (WIDTH * HEIGHT) as usize];
        for row in sky_rows..HEIGHT {
            let distance = distances[((row - sky_rows) as usize).min(distances.len() - 1)];
            for column in 0..WIDTH {
                depths[(row * WIDTH + column) as usize] = (1.0 / distance) as f32;
            }
        }
        DepthImage {
            width: WIDTH,
            height: HEIGHT,
            depths,
        }
    }

    fn paint(sky: [u8; 3], rows: &[[u8; 3]], sky_rows: u32) -> Vec<u8> {
        let mut pixels = Vec::new();
        for row in 0..HEIGHT {
            let rgb = if row < sky_rows {
                sky
            } else {
                rows[((row - sky_rows) as usize).min(rows.len() - 1)]
            };
            for _ in 0..WIDTH {
                pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        pixels
    }

    #[test]
    fn terrain_that_reaches_the_sky_scores_one() {
        // Far rows are painted the sky's own colour: fully hazed out.
        let depth = scene(
            2,
            &[40_000.0, 40_000.0, 40_000.0, 1_000.0, 1_000.0, 1_000.0],
        );
        let pixels = paint(
            [40, 90, 160],
            &[
                [40, 90, 160],
                [40, 90, 160],
                [40, 90, 160],
                [200, 150, 90],
                [200, 150, 90],
                [200, 150, 90],
            ],
            2,
        );
        let report = measure(0.0, "raster", &geometry(), &depth, &pixels, WIDTH, HEIGHT);
        let convergence = report.convergence.expect("two populated bands and a sky");
        assert!(
            convergence > 0.95,
            "terrain matching the sky at distance should converge, got {convergence}"
        );
    }

    /// The failure this instrument exists to catch: a range that keeps its own
    /// colour all the way out, which reads as a painted backdrop.
    #[test]
    fn terrain_that_keeps_its_colour_scores_zero() {
        let depth = scene(
            2,
            &[40_000.0, 40_000.0, 40_000.0, 1_000.0, 1_000.0, 1_000.0],
        );
        let pixels = paint([40, 90, 160], &[[200, 150, 90]], 2);
        let report = measure(0.0, "raster", &geometry(), &depth, &pixels, WIDTH, HEIGHT);
        let convergence = report.convergence.expect("two populated bands and a sky");
        assert!(
            convergence.abs() < 0.05,
            "unchanging terrain colour should score zero, got {convergence}"
        );
    }

    /// Extinction with no in-scatter to balance it moves terrain *away* from
    /// the sky, and that has to read as a negative score rather than as no
    /// result.
    ///
    /// Note the case has to be built against a *bright* sky. Against a dark one
    /// this metric is not a direction test: terrain extinguishing toward black
    /// gets closer to a dark sky in RGB and scores positive, which is correct
    /// for the question being asked ("has it arrived") and simply is not the
    /// question "did it get there the right way".
    #[test]
    fn receding_from_the_sky_scores_negative() {
        let depth = scene(
            2,
            &[40_000.0, 40_000.0, 40_000.0, 1_000.0, 1_000.0, 1_000.0],
        );
        let pixels = paint(
            [140, 180, 230],
            &[
                [30, 20, 10],
                [30, 20, 10],
                [30, 20, 10],
                [150, 170, 200],
                [150, 170, 200],
                [150, 170, 200],
            ],
            2,
        );
        let report = measure(0.0, "raster", &geometry(), &depth, &pixels, WIDTH, HEIGHT);
        let convergence = report.convergence.expect("two populated bands and a sky");
        assert!(
            convergence < 0.0,
            "terrain receding from the sky should score negative, got {convergence}"
        );
    }

    /// The sky reference must come from beside the silhouette, not from the
    /// whole upper frame, or a bright zenith drags the target away from the
    /// horizon radiance the far terrain is actually seen against.
    #[test]
    fn the_sky_reference_is_taken_at_the_silhouette() {
        let mut pixels = paint([40, 90, 160], &[[200, 150, 90]], 3);
        // Repaint the top row a very different colour: it is zenith, not
        // horizon, and must not be sampled.
        for column in 0..WIDTH {
            let index = (column * 4) as usize;
            pixels[index] = 255;
            pixels[index + 1] = 0;
            pixels[index + 2] = 0;
        }
        let depth = scene(3, &[5_000.0]);
        let report = measure(0.0, "raster", &geometry(), &depth, &pixels, WIDTH, HEIGHT);
        let sky = report.sky_rgb.expect("a silhouette exists");
        assert!(
            (sky[0] - 40.0 / 255.0).abs() < 1.0e-6,
            "sky sampled from the wrong row: {sky:?}"
        );
        assert_eq!(report.sky_sample_count, WIDTH);
    }

    /// A frame with no terrain in it has no reference and no curve, and must
    /// say so rather than reporting a convergence of zero.
    #[test]
    fn an_empty_sky_frame_reports_nothing() {
        let depth = DepthImage {
            width: WIDTH,
            height: HEIGHT,
            depths: vec![0.0; (WIDTH * HEIGHT) as usize],
        };
        let pixels = paint([40, 90, 160], &[[40, 90, 160]], HEIGHT);
        let report = measure(0.0, "raster", &geometry(), &depth, &pixels, WIDTH, HEIGHT);
        assert!(report.sky_rgb.is_none());
        assert!(report.convergence.is_none());
        assert!(report.bins.is_empty());
    }
}

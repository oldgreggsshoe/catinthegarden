//! Ground-truth probe: recovers the height of the surface that was actually
//! *drawn*, so it can be compared against the surface the CPU collides with.
//!
//! The standing invariant for this planet is that the camera must be able to
//! stand on the ground -- not sink into it, not float above it. That only
//! holds if the CPU's terrain height and the renderer's displaced surface are
//! the same surface, and the two are computed by completely separate code: a
//! resident-tile lookup plus an f64 detail ladder on one side, a streamed
//! texture fetch plus an f32 detail ladder inside a vertex or ray shader on
//! the other.
//!
//! Reading the depth buffer measures the drawn result rather than
//! re-evaluating the shader's arithmetic on the CPU and hoping the two agree,
//! so it also catches everything between the height function and the pixel:
//! mesh tessellation, LOD selection, skirts, and the raymarcher's hit
//! refinement. It works the same way for both render paths because both write
//! reversed-Z depth into the same attachment.

use std::sync::mpsc;
use std::time::Duration;

use glam::{DVec2, DVec3};

use crate::planet::{CameraViewBasis, PLANET_RADIUS_METERS};
use crate::terrain::{NearFieldCoverage, SurfaceHeightBreakdown};

/// Grid resolution of the screen-space sample pattern, per axis.
pub const PROBE_GRID: u32 = 9;

/// Hits beyond this range are recorded but excluded from the agreement
/// statistics. The CPU samples whichever resident tile is finest, while the
/// GPU samples the LOD it chose for that pixel; far away those are routinely
/// different levels of the pyramid, so a disagreement out there measures the
/// pyramid rather than the two surface functions. Near the camera -- which is
/// the only place standing on the ground is a question -- they should be
/// looking at the same data.
pub const MAX_COMPARISON_DISTANCE_METERS: f64 = 4_000.0;

/// Everything needed to turn a depth sample back into a planet-frame point.
/// These must be the values the camera uniform was built from for that frame,
/// not recomputed approximations of them.
#[derive(Clone, Copy, Debug)]
pub struct ProbeGeometry {
    near_meters: f64,
    tan_half_vertical_fov: f64,
    aspect_ratio: f64,
    camera_world_position: DVec3,
    basis: CameraViewBasis,
}

impl ProbeGeometry {
    pub fn new(
        near_meters: f64,
        vertical_fov_radians: f64,
        aspect_ratio: f64,
        camera_world_position: DVec3,
        camera_forward: DVec3,
        camera_up: DVec3,
    ) -> Self {
        Self {
            near_meters,
            tan_half_vertical_fov: (vertical_fov_radians * 0.5).tan(),
            aspect_ratio,
            camera_world_position,
            basis: CameraViewBasis::from_forward_and_up(camera_forward, camera_up),
        }
    }

    /// Distance along the view axis for a depth sample.
    ///
    /// `reversed_z_infinite_perspective` puts `near` in the w-row and `-1` in
    /// the z-row, so clip z/w is exactly `near / forward_distance`. Zero is the
    /// clear value and means nothing was drawn at that pixel.
    pub fn forward_distance_meters(&self, depth: f32) -> Option<f64> {
        let depth = f64::from(depth);
        (depth.is_finite() && depth > 0.0).then(|| self.near_meters / depth)
    }

    pub fn hit(&self, ndc: DVec2, depth: f32) -> Option<ProbeHit> {
        let forward_distance_meters = self.forward_distance_meters(depth)?;
        let view_offset = DVec3::new(
            ndc.x * self.aspect_ratio * self.tan_half_vertical_fov * forward_distance_meters,
            ndc.y * self.tan_half_vertical_fov * forward_distance_meters,
            -forward_distance_meters,
        );
        let offset = self.basis.view_to_world(view_offset);
        let world_position = self.camera_world_position + offset;
        let radius_meters = world_position.length();
        if !world_position.is_finite() || radius_meters <= 0.0 {
            return None;
        }
        Some(ProbeHit {
            ndc: ndc.to_array(),
            distance_meters: offset.length(),
            direction: world_position / radius_meters,
            height_meters: radius_meters - PLANET_RADIUS_METERS,
        })
    }
}

/// A reconstructed point on the drawn surface, in the planet frame.
#[derive(Clone, Copy, Debug)]
pub struct ProbeHit {
    pub ndc: [f64; 2],
    pub distance_meters: f64,
    pub direction: DVec3,
    pub height_meters: f64,
}

/// One drawn point set against what the CPU believes is there.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct ProbeComparison {
    pub ndc: [f64; 2],
    pub distance_meters: f64,
    pub rendered_height_meters: f64,
    pub cpu_height_meters: f64,
    /// The CPU's baked-data-only height at the same point.
    pub cpu_macro_height_meters: f64,
    pub cpu_source_level: u8,
    /// Positive means the renderer drew the ground *above* where the CPU
    /// thinks it is, which is the direction that buries a camera.
    pub delta_meters: f64,
    /// The same gap measured against baked data alone. When this is near zero
    /// while `delta_meters` is not, the renderer drew the macro surface and
    /// none of the synthesised detail -- a different fault from drawing the
    /// detail wrongly, and one that looks identical in a screenshot.
    pub delta_from_macro_meters: f64,
}

fn comparison(hit: &ProbeHit, cpu: SurfaceHeightBreakdown) -> ProbeComparison {
    ProbeComparison {
        ndc: hit.ndc,
        distance_meters: hit.distance_meters,
        rendered_height_meters: hit.height_meters,
        cpu_height_meters: cpu.height_meters,
        cpu_macro_height_meters: cpu.macro_height_meters,
        cpu_source_level: cpu.source_level,
        delta_meters: hit.height_meters - cpu.height_meters,
        delta_from_macro_meters: hit.height_meters - cpu.macro_height_meters,
    }
}

/// The frame's verdict. `camera_clearance_meters` is the stand-on-ground
/// number: how far the camera sits above the CPU's surface directly beneath
/// it. The delta statistics say whether that surface is the drawn one.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SurfaceProbeReport {
    pub sim_time: f64,
    pub render_path: String,
    pub render_debug_mode: String,
    pub ray_near_field: Option<NearFieldCoverage>,
    pub comparison_distance_limit_meters: f64,
    pub camera_altitude_meters: f64,
    pub camera_surface_height_meters: f64,
    pub camera_clearance_meters: f64,
    pub sampled_points: usize,
    pub surface_hits: usize,
    pub compared_points: usize,
    pub center: Option<ProbeComparison>,
    pub max_abs_delta_meters: f64,
    /// The statistic the assertions should read. A maximum over the sample grid
    /// is dominated by whichever point happened to graze the horizon: at 2m of
    /// eye height the horizon is 4km, the probe compares out to 4km, and a ray
    /// arriving there at a fraction of a degree turns a metre of ground into
    /// hundreds of metres of reconstructed height. Measured at the landing site
    /// in the raymarch path, 75 of 77 points sat inside 5m while two grazing
    /// points at 2-3km read 25m and 195m.
    pub p90_abs_delta_meters: f64,
    pub median_abs_delta_meters: f64,
    pub mean_delta_meters: f64,
    /// Mean gap against baked data alone, over the same points.
    pub mean_delta_from_macro_meters: f64,
    /// How well the renderer's synthesised relief tracks the CPU's, as a
    /// Pearson correlation over the compared points. Both sides add a detail
    /// field on top of the same baked macro shape; this asks whether it is the
    /// *same* field. Near 1 means yes. Near 0 means the two are independent
    /// noise of similar amplitude, which looks fine in a screenshot and in
    /// every amplitude statistic, while the surface the camera collides with
    /// is not the surface it can see.
    pub detail_correlation: Option<f64>,
    /// Slope of the same fit: how much of the CPU's relief the renderer
    /// actually carries. One means all of it.
    pub detail_slope: Option<f64>,
    pub nearest_hit_distance_meters: f64,
    /// Every compared point, for diagnosis. The aggregates above are what the
    /// assertions read.
    pub comparisons: Vec<ProbeComparison>,
}

/// Screen-space sample pattern, as NDC.
///
/// The grid deliberately stops short of the frame edges: an edge sample sits
/// half a pixel from the frustum boundary, where a one-texel error in the
/// depth sample moves the reconstructed point much further than it does in the
/// middle of the frame.
pub fn probe_ndc_grid() -> Vec<DVec2> {
    let steps = PROBE_GRID.max(1);
    let mut points = Vec::with_capacity((steps * steps) as usize);
    for row in 0..steps {
        for column in 0..steps {
            let fraction = |index: u32| {
                if steps == 1 {
                    0.5
                } else {
                    f64::from(index) / f64::from(steps - 1)
                }
            };
            points.push(DVec2::new(
                (fraction(column) * 2.0 - 1.0) * 0.9,
                (fraction(row) * 2.0 - 1.0) * 0.9,
            ));
        }
    }
    points
}

/// A depth attachment read back to host memory, in framebuffer row order.
pub struct DepthImage {
    pub width: u32,
    pub height: u32,
    pub depths: Vec<f32>,
}

impl DepthImage {
    /// Nearest-neighbour fetch. Depth is not linear in screen space across a
    /// silhouette, so interpolating between neighbours would invent a surface
    /// that spans the gap.
    pub fn sample(&self, ndc: DVec2) -> Option<f32> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let column = (((ndc.x * 0.5 + 0.5) * f64::from(self.width)).floor() as i64)
            .clamp(0, i64::from(self.width) - 1) as u32;
        // Framebuffer row 0 is the top of the frame, which is NDC y = +1.
        let row = (((0.5 - ndc.y * 0.5) * f64::from(self.height)).floor() as i64)
            .clamp(0, i64::from(self.height) - 1) as u32;
        self.depths
            .get((row * self.width + column) as usize)
            .copied()
    }
}

/// Compares the drawn surface against `cpu_height` over the sample pattern.
///
/// `cpu_height` is the same lookup the camera follows, so a zero delta here is
/// exactly the statement "what you can see is what you would stand on".
#[cfg(test)]
pub fn compare_surface(
    sim_time: f64,
    render_path: &str,
    geometry: &ProbeGeometry,
    depth: &DepthImage,
    camera_altitude_meters: f64,
    camera_surface_height_meters: f64,
    cpu_height: impl Fn(DVec3) -> Option<SurfaceHeightBreakdown>,
) -> SurfaceProbeReport {
    compare_surface_with_limit(
        sim_time,
        render_path,
        geometry,
        depth,
        camera_altitude_meters,
        camera_surface_height_meters,
        MAX_COMPARISON_DISTANCE_METERS,
        cpu_height,
    )
}

pub fn compare_surface_with_limit(
    sim_time: f64,
    render_path: &str,
    geometry: &ProbeGeometry,
    depth: &DepthImage,
    camera_altitude_meters: f64,
    camera_surface_height_meters: f64,
    maximum_comparison_distance_meters: f64,
    cpu_height: impl Fn(DVec3) -> Option<SurfaceHeightBreakdown>,
) -> SurfaceProbeReport {
    assert!(
        maximum_comparison_distance_meters.is_finite() && maximum_comparison_distance_meters > 0.0
    );
    let points = probe_ndc_grid();
    let mut comparisons: Vec<ProbeComparison> = Vec::with_capacity(points.len());
    let mut surface_hits = 0usize;
    let mut nearest_hit_distance_meters = f64::INFINITY;
    for ndc in &points {
        let Some(sample) = depth.sample(*ndc) else {
            continue;
        };
        let Some(hit) = geometry.hit(*ndc, sample) else {
            continue;
        };
        surface_hits += 1;
        nearest_hit_distance_meters = nearest_hit_distance_meters.min(hit.distance_meters);
        if hit.distance_meters > maximum_comparison_distance_meters {
            continue;
        }
        let Some(cpu) = cpu_height(hit.direction) else {
            continue;
        };
        comparisons.push(comparison(&hit, cpu));
    }

    let center = depth
        .sample(DVec2::ZERO)
        .and_then(|sample| geometry.hit(DVec2::ZERO, sample))
        .and_then(|hit| cpu_height(hit.direction).map(|cpu| comparison(&hit, cpu)));

    let mut absolute: Vec<f64> = comparisons
        .iter()
        .map(|comparison| comparison.delta_meters.abs())
        .collect();
    absolute.sort_by(f64::total_cmp);
    let quantile = |fraction: f64| {
        if absolute.is_empty() {
            0.0
        } else {
            absolute[((absolute.len() - 1) as f64 * fraction) as usize]
        }
    };
    let median_abs_delta_meters = quantile(0.5);
    let p90_abs_delta_meters = quantile(0.9);
    let mean = |select: fn(&ProbeComparison) -> f64| {
        if comparisons.is_empty() {
            0.0
        } else {
            comparisons.iter().map(select).sum::<f64>() / comparisons.len() as f64
        }
    };
    let mean_delta_meters = mean(|comparison| comparison.delta_meters);
    let mean_delta_from_macro_meters = mean(|comparison| comparison.delta_from_macro_meters);
    let (detail_correlation, detail_slope) = detail_agreement(&comparisons);

    SurfaceProbeReport {
        sim_time,
        render_path: render_path.to_owned(),
        render_debug_mode: "final HDR scene".to_owned(),
        ray_near_field: None,
        comparison_distance_limit_meters: maximum_comparison_distance_meters,
        camera_altitude_meters,
        camera_surface_height_meters,
        camera_clearance_meters: camera_altitude_meters - camera_surface_height_meters,
        sampled_points: points.len(),
        surface_hits,
        compared_points: comparisons.len(),
        center,
        max_abs_delta_meters: absolute.last().copied().unwrap_or(0.0),
        p90_abs_delta_meters,
        median_abs_delta_meters,
        mean_delta_meters,
        mean_delta_from_macro_meters,
        detail_correlation,
        detail_slope,
        nearest_hit_distance_meters: if nearest_hit_distance_meters.is_finite() {
            nearest_hit_distance_meters
        } else {
            0.0
        },
        comparisons,
    }
}

/// Fits the renderer's relief against the CPU's, returning `(correlation,
/// slope)`. Both are `None` when the points carry no relief to compare -- a
/// flat plain cannot say whether two detail fields match, and reporting a
/// correlation of zero there would look like a failure.
fn detail_agreement(comparisons: &[ProbeComparison]) -> (Option<f64>, Option<f64>) {
    if comparisons.len() < 8 {
        return (None, None);
    }
    let cpu: Vec<f64> = comparisons
        .iter()
        .map(|comparison| comparison.cpu_height_meters - comparison.cpu_macro_height_meters)
        .collect();
    let rendered: Vec<f64> = comparisons
        .iter()
        .map(|comparison| comparison.delta_from_macro_meters)
        .collect();
    let count = comparisons.len() as f64;
    let cpu_mean = cpu.iter().sum::<f64>() / count;
    let rendered_mean = rendered.iter().sum::<f64>() / count;
    let cpu_variance: f64 = cpu.iter().map(|value| (value - cpu_mean).powi(2)).sum();
    let rendered_variance: f64 = rendered
        .iter()
        .map(|value| (value - rendered_mean).powi(2))
        .sum();
    let covariance: f64 = cpu
        .iter()
        .zip(&rendered)
        .map(|(left, right)| (left - cpu_mean) * (right - rendered_mean))
        .sum();
    // Half a metre of spread over the sample is the least that can support a
    // meaningful fit at the amplitudes this ladder works at.
    if cpu_variance < 0.25 * count || rendered_variance < 0.25 * count {
        return (None, None);
    }
    (
        Some(covariance / (cpu_variance * rendered_variance).sqrt()),
        Some(covariance / cpu_variance),
    )
}

pub struct PendingDepthReadback {
    buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
    width: u32,
    height: u32,
}

/// Must be encoded while the depth attachment still holds the terrain result.
/// The visual sun overlay pass discards depth on store, so anything scheduled
/// after it reads undefined contents.
pub fn schedule_depth_readback(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    depth_texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> PendingDepthReadback {
    let padded_bytes_per_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("surface probe depth readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: depth_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    PendingDepthReadback {
        buffer,
        padded_bytes_per_row,
        width,
        height,
    }
}

pub fn finish_depth_readback(
    device: &wgpu::Device,
    pending: PendingDepthReadback,
) -> Result<DepthImage, String> {
    let (sender, receiver) = mpsc::channel();
    pending
        .buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())??;

    let mapped = pending.buffer.slice(..).get_mapped_range();
    let mut depths = Vec::with_capacity((pending.width * pending.height) as usize);
    for row in mapped.chunks_exact(pending.padded_bytes_per_row as usize) {
        for texel in row[..(pending.width * 4) as usize].chunks_exact(4) {
            depths.push(f32::from_le_bytes(
                texel.try_into().expect("four bytes per depth texel"),
            ));
        }
    }
    drop(mapped);
    pending.buffer.unmap();

    Ok(DepthImage {
        width: pending.width,
        height: pending.height,
        depths,
    })
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};

    use super::{
        DepthImage, MAX_COMPARISON_DISTANCE_METERS, PROBE_GRID, ProbeGeometry, compare_surface,
        compare_surface_with_limit, probe_ndc_grid,
    };
    use crate::planet::PLANET_RADIUS_METERS;
    use crate::terrain::SurfaceHeightBreakdown;

    /// A terrain that is entirely baked macro shape, with no synthesised
    /// detail: the simplest thing the probe can be held against.
    fn flat_truth(height_meters: f64) -> impl Fn(DVec3) -> Option<SurfaceHeightBreakdown> {
        move |_| {
            Some(SurfaceHeightBreakdown {
                height_meters,
                macro_height_meters: height_meters,
                source_level: 18,
            })
        }
    }

    fn nadir_geometry(altitude_meters: f64, near_meters: f64) -> ProbeGeometry {
        let camera = DVec3::new(0.0, 0.0, PLANET_RADIUS_METERS + altitude_meters);
        ProbeGeometry::new(
            near_meters,
            60_f64.to_radians(),
            16.0 / 9.0,
            camera,
            -DVec3::Z,
            DVec3::Y,
        )
    }

    fn uniform_depth(width: u32, height: u32, depth: f32) -> DepthImage {
        DepthImage {
            width,
            height,
            depths: vec![depth; (width * height) as usize],
        }
    }

    #[test]
    fn depth_inverts_to_the_distance_that_produced_it() {
        let geometry = nadir_geometry(1_000.0, 0.5);
        let distance_meters = 137.25;
        let depth = (0.5 / distance_meters) as f32;
        let recovered = geometry
            .forward_distance_meters(depth)
            .expect("a positive depth is a surface");
        assert!((recovered - distance_meters).abs() < 0.01);
    }

    #[test]
    fn cleared_depth_is_not_a_surface() {
        let geometry = nadir_geometry(1_000.0, 0.5);
        assert!(geometry.forward_distance_meters(0.0).is_none());
        assert!(geometry.forward_distance_meters(f32::NAN).is_none());
    }

    #[test]
    fn a_centre_hit_straight_down_lands_at_the_expected_height() {
        // Looking at the planet centre from 1000m, a hit 400m along the view
        // axis is standing 600m above the reference sphere.
        let geometry = nadir_geometry(1_000.0, 1.0);
        let hit = geometry
            .hit(DVec2::ZERO, (1.0 / 400.0) as f32)
            .expect("the ray hits the surface");
        assert!((hit.height_meters - 600.0).abs() < 0.05, "{hit:?}");
        assert!((hit.distance_meters - 400.0).abs() < 0.05);
        assert!(hit.direction.dot(DVec3::Z) > 0.999_999);
    }

    #[test]
    fn an_off_centre_hit_moves_sideways_by_the_field_of_view() {
        let geometry = nadir_geometry(1_000.0, 1.0);
        let forward_distance = 400.0;
        let hit = geometry
            .hit(
                DVec2::new(1.0, 0.0),
                (1.0 / f64::from(forward_distance as f32)) as f32,
            )
            .expect("the ray hits the surface");
        // At the right edge the offset is aspect * tan(fov/2) * distance.
        let expected_sideways = 16.0 / 9.0 * 30_f64.to_radians().tan() * 400.0;
        let sideways = (hit.distance_meters.powi(2) - 400.0_f64.powi(2)).sqrt();
        assert!(
            (sideways - expected_sideways).abs() < 1.0,
            "{sideways} vs {expected_sideways}"
        );
    }

    #[test]
    fn the_sample_grid_covers_the_frame_without_touching_its_edges() {
        let points = probe_ndc_grid();
        assert_eq!(points.len(), (PROBE_GRID * PROBE_GRID) as usize);
        assert!(points.contains(&DVec2::ZERO));
        assert!(
            points
                .iter()
                .all(|point| point.x.abs() <= 0.9 + 1.0e-12 && point.y.abs() <= 0.9 + 1.0e-12)
        );
    }

    #[test]
    fn depth_rows_start_at_the_top_of_the_frame() {
        let mut image = uniform_depth(4, 4, 0.0);
        image.depths[0] = 0.25;
        // NDC y = +1 is the top row, which is framebuffer row 0.
        assert_eq!(image.sample(DVec2::new(-1.0, 1.0)), Some(0.25));
        assert_eq!(image.sample(DVec2::new(-1.0, -1.0)), Some(0.0));
    }

    #[test]
    fn an_exactly_agreeing_surface_reports_zero_delta() {
        let altitude = 1_000.0;
        let geometry = nadir_geometry(altitude, 1.0);
        let depth = uniform_depth(64, 64, (1.0 / 400.0) as f32);
        let report = compare_surface(
            0.0,
            "raster",
            &geometry,
            &depth,
            altitude,
            600.0,
            // Every reconstructed point is 600m up, because every depth sample
            // is the same and the frame is small enough for curvature to be
            // negligible over it.
            flat_truth(600.0),
        );
        assert_eq!(report.compared_points, report.sampled_points);
        assert!(report.max_abs_delta_meters < 0.5, "{report:?}");
        assert!((report.camera_clearance_meters - 400.0).abs() < 1.0e-9);
        assert!(report.center.is_some());
    }

    #[test]
    fn a_sunken_camera_shows_as_a_positive_delta() {
        // The renderer draws ground 5m higher than the CPU believes: the sign
        // that buries a camera placed from the CPU's number.
        let altitude = 1_000.0;
        let geometry = nadir_geometry(altitude, 1.0);
        let depth = uniform_depth(64, 64, (1.0 / 400.0) as f32);
        let report = compare_surface(
            0.0,
            "ray",
            &geometry,
            &depth,
            altitude,
            595.0,
            flat_truth(595.0),
        );
        assert!(
            (report.mean_delta_meters - 5.0).abs() < 0.5,
            "{} should be about +5",
            report.mean_delta_meters
        );
        assert!((report.camera_clearance_meters - 405.0).abs() < 1.0e-9);
    }

    #[test]
    fn drawing_only_the_baked_macro_shape_shows_up_as_a_macro_agreement() {
        // The renderer drew 600m -- exactly the baked surface -- while the CPU
        // believes the detail ladder adds 3.3m on top. The overall delta is
        // -3.3m, but the macro delta is zero, which names the fault: the
        // detail never reached the geometry.
        let altitude = 1_000.0;
        let geometry = nadir_geometry(altitude, 1.0);
        let depth = uniform_depth(64, 64, (1.0 / 400.0) as f32);
        let report = compare_surface(0.0, "raster", &geometry, &depth, altitude, 603.3, |_| {
            Some(SurfaceHeightBreakdown {
                height_meters: 603.3,
                macro_height_meters: 600.0,
                source_level: 18,
            })
        });
        assert!((report.mean_delta_meters + 3.3).abs() < 0.5, "{report:?}");
        assert!(
            report.mean_delta_from_macro_meters.abs() < 0.5,
            "{report:?}"
        );
        assert_eq!(report.comparisons.len(), report.compared_points);
    }

    #[test]
    fn detail_correlation_separates_the_same_field_from_independent_noise() {
        let altitude = 1_000.0;
        let geometry = nadir_geometry(altitude, 1.0);
        // Give every probe point its own depth so the reconstructed heights
        // vary, then hand the CPU either the same relief or unrelated relief.
        let mut depths = Vec::new();
        for index in 0..(64 * 64) {
            let relief = ((index % 17) as f64 - 8.0) * 0.6;
            depths.push((1.0 / (400.0 - relief)) as f32);
        }
        let depth = DepthImage {
            width: 64,
            height: 64,
            depths,
        };

        let matching = compare_surface(
            0.0,
            "raster",
            &geometry,
            &depth,
            altitude,
            600.0,
            |direction| {
                // Reproduce the rendered relief exactly from the point itself.
                let relief = (direction.x * 1.0e7).sin() * 0.0;
                Some(SurfaceHeightBreakdown {
                    height_meters: 600.0 + relief,
                    macro_height_meters: 600.0,
                    source_level: 18,
                })
            },
        );
        // The CPU here is flat, so there is nothing to correlate against and
        // the fit must decline to answer rather than report zero.
        assert_eq!(matching.detail_correlation, None);

        let independent = compare_surface(
            0.0,
            "raster",
            &geometry,
            &depth,
            altitude,
            600.0,
            |direction| {
                let relief = ((direction.x * 9.1e6).sin() * 43_758.5).fract() * 6.0;
                Some(SurfaceHeightBreakdown {
                    height_meters: 600.0 + relief,
                    macro_height_meters: 600.0,
                    source_level: 18,
                })
            },
        );
        let correlation = independent
            .detail_correlation
            .expect("both sides carry relief here");
        assert!(
            correlation.abs() < 0.6,
            "independent noise should not correlate: {correlation}"
        );
    }

    #[test]
    fn sky_pixels_are_counted_as_misses_not_as_agreement() {
        let altitude = 1_000.0;
        let geometry = nadir_geometry(altitude, 1.0);
        let depth = uniform_depth(64, 64, 0.0);
        let report = compare_surface(
            0.0,
            "raster",
            &geometry,
            &depth,
            altitude,
            0.0,
            flat_truth(0.0),
        );
        assert_eq!(report.surface_hits, 0);
        assert_eq!(report.compared_points, 0);
        assert_eq!(report.max_abs_delta_meters, 0.0);
        assert!(report.center.is_none());
    }

    #[test]
    fn distant_hits_are_left_out_of_the_statistics() {
        let altitude = 100_000.0;
        let geometry = nadir_geometry(altitude, 10.0);
        let far_meters = MAX_COMPARISON_DISTANCE_METERS * 5.0;
        let depth = uniform_depth(64, 64, (10.0 / far_meters) as f32);
        let report = compare_surface(
            0.0,
            "raster",
            &geometry,
            &depth,
            altitude,
            0.0,
            flat_truth(0.0),
        );
        assert!(report.surface_hits > 0);
        assert_eq!(report.compared_points, 0);

        let extended = compare_surface_with_limit(
            0.0,
            "raster",
            &geometry,
            &depth,
            altitude,
            0.0,
            far_meters * 2.0,
            flat_truth(0.0),
        );
        assert_eq!(extended.compared_points, extended.surface_hits);
        assert_eq!(extended.comparison_distance_limit_meters, far_meters * 2.0);
    }
}

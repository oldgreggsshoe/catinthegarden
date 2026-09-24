//! Tessendorf FFT wind sea: the short part of the ocean spectrum.
//!
//! Sea of Thieves's ocean is an FFT ocean (Rare, SIGGRAPH 2018 talk "The
//! Technical Art of Sea of Thieves"). A hand-placed Gerstner set cannot reach
//! that look. With one plane wave per band the eye finds the lattice they form
//! (`test-runs/ocean_sot_detail_2026-09-23`), and re-aiming them only rotates
//! it. Here the part of the sea shorter than `FFT_LONGEST_WAVELENGTH_METERS` is
//! instead tens of thousands of modes drawn from a directional power spectrum
//! and summed each frame by an inverse FFT on the GPU.
//!
//! The long swells and the storm sea stay Gerstner. They are what the weather
//! drives, they are coherent across the whole planet, and a 1.4km swell would
//! need a tile so large that its repeat would show.
//!
//! **Layout.** There are three cascades of `N` x `N`, with tile sizes chosen
//! not to be multiples of one another. Each cascade owns one annulus of
//! wavelengths, so the three add up without counting any mode twice. The
//! planar tiles reach the sphere through three orthographic projections (one
//! per planet axis), blended by `|direction.axis|^8`. The blend preserves
//! variance, so the sea keeps its height where two projections meet.
//!
//! **CPU parity.** Buoyancy and collision need the same water. The CPU keeps
//! every mode of the two long cascades and sums them directly at the query
//! point, then inverts the horizontal (choppy) displacement by fixed-point
//! iteration. The short cascade is under 1% of the variance and is shading
//! detail. `ocean_fft_cpu_matches_cpu_fft_grid` pins the DFT against the
//! grid, and `gpu_fft_matches_cpu_fft` (ignored, needs a GPU) pins the grid
//! against the compute shader.

use std::sync::OnceLock;

use glam::{DVec2, DVec3};
use wgpu::util::DeviceExt;

use crate::planet::planet_radius_meters;

/// Grid side of each cascade.
pub const N: usize = 256;
const LOG2_N: u32 = 8;
pub const CASCADE_COUNT: usize = 3;
/// Tile sizes in metres. The ratios (6.13, 3.98) are deliberately irrational
/// looking so no two cascades repeat in step.
pub const CASCADE_SIZES_METERS: [f64; CASCADE_COUNT] = [1000.0, 163.0, 41.0];
/// Wavelength annulus each cascade owns: [shortest, longest).
const CASCADE_BANDS_METERS: [(f64, f64); CASCADE_COUNT] = [
    (60.0, FFT_LONGEST_WAVELENGTH_METERS),
    (15.0, 60.0),
    (0.0, 15.0),
];
/// The FFT carries every wave shorter than this. The Gerstner table keeps the
/// rest; `ocean::FFT_REPLACES_FROM` names the first Gerstner row it replaces.
pub const FFT_LONGEST_WAVELENGTH_METERS: f64 = 250.0;
/// Capillary cutoff: modes much shorter than this are damped away.
const DAMPING_LENGTH_METERS: f64 = 0.25;
/// Longuet-Higgins spreading exponent, cos^(2s)(theta/2). Low values give the
/// broad crossing chop of an actively blowing sea rather than a combed swell.
const SPREADING_S: f64 = 3.0;
/// Downwind direction in each projection plane (u, v).
const WIND_DIRECTION: DVec2 = DVec2::new(0.8944271909999159, 0.4472135954999579);
/// The dispersion relation is quantised to multiples of 2pi/`REPEAT_SECONDS`,
/// so the whole field is exactly periodic and the GPU never sees a large time.
pub const REPEAT_SECONDS: f64 = 1000.0;
/// Horizontal (choppy) displacement factor. Chosen so the calm sea never
/// folds and the storm sea only just does at its highest crests; the foam
/// test measures both.
pub const CHOPPINESS: f64 = 1.1;
/// Standard deviation of the FFT sea height (metres) at calm and at full
/// storm. This matches the variance of the Gerstner rows it replaced
/// (rows 6..18 at 44x and 55x geometry scale).
pub const CALM_HEIGHT_SIGMA_METERS: f64 = 2.2;
pub const STORM_HEIGHT_SIGMA_METERS: f64 = 2.75;
/// Cascades the CPU sums. The third holds under 1% of the variance.
const CPU_CASCADES: usize = 2;

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        matches!(
            std::env::var("CATINGARDEN_OCEAN_FFT")
                .ok()
                .as_deref()
                .map(str::trim),
            Some("1" | "true" | "on")
        )
    })
}

/// Height standard deviation for a storm intensity, following the same
/// smoothstep blend the Gerstner storm columns use.
pub fn height_sigma_meters(storm_intensity: f32) -> f64 {
    let blend = crate::ocean::storm_blend_value(storm_intensity);
    CALM_HEIGHT_SIGMA_METERS + (STORM_HEIGHT_SIGMA_METERS - CALM_HEIGHT_SIGMA_METERS) * blend
}

fn gravity() -> f64 {
    crate::surface_camera::GRAVITY_METERS_PER_SECOND_SQUARED
}

fn quantised_omega(wave_number: f64) -> f64 {
    let base = std::f64::consts::TAU / REPEAT_SECONDS;
    ((gravity() * wave_number).sqrt() / base).round() * base
}

/// Splitmix64 of a mode's identity: deterministic, and independent of the
/// order the modes are visited in.
fn mode_hash(cascade: usize, n: i32, m: i32, salt: u64) -> u64 {
    let mut value = 0x0cea_7fd2_2026_0924_u64
        ^ (cascade as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (n as i64 as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ (m as i64 as u64).wrapping_mul(0x94d0_49bb_1331_11eb)
        ^ salt.wrapping_mul(0x2545_f491_4f6c_dd1d);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit_open(bits: u64) -> f64 {
    ((bits >> 11) as f64 + 0.5) / (1_u64 << 53) as f64
}

/// Two independent standard normals for a mode (Box-Muller).
fn gaussian_pair(cascade: usize, n: i32, m: i32) -> DVec2 {
    let u1 = unit_open(mode_hash(cascade, n, m, 1));
    let u2 = unit_open(mode_hash(cascade, n, m, 2));
    let radius = (-2.0 * u1.ln()).sqrt();
    let angle = std::f64::consts::TAU * u2;
    DVec2::new(radius * angle.cos(), radius * angle.sin())
}

fn wave_vector(cascade: usize, n: i32, m: i32) -> DVec2 {
    DVec2::new(n as f64, m as f64) * (std::f64::consts::TAU / CASCADE_SIZES_METERS[cascade])
}

/// Unnormalised spectral density at a wave vector, restricted to this
/// cascade's band.
fn spectrum(cascade: usize, k: DVec2) -> f64 {
    let wave_number = k.length();
    if wave_number <= 0.0 {
        return 0.0;
    }
    let wavelength = std::f64::consts::TAU / wave_number;
    let (shortest, longest) = CASCADE_BANDS_METERS[cascade];
    if wavelength < shortest || wavelength >= longest {
        return 0.0;
    }
    let cos_theta = (k / wave_number).dot(WIND_DIRECTION);
    // cos^(2s)(theta/2) = ((1 + cos theta)/2)^s
    let spreading = (0.5 * (1.0 + cos_theta)).powf(SPREADING_S);
    wave_number.powi(-4) * spreading * (-(wave_number * DAMPING_LENGTH_METERS).powi(2)).exp()
}

/// Initial complex amplitude h0(k) at unit spectral scale.
fn raw_h0(cascade: usize, n: i32, m: i32) -> DVec2 {
    let half = (N / 2) as i32;
    if n <= -half || m <= -half || n >= half || m >= half {
        return DVec2::ZERO;
    }
    let dk = std::f64::consts::TAU / CASCADE_SIZES_METERS[cascade];
    let k = wave_vector(cascade, n, m);
    gaussian_pair(cascade, n, m) * (spectrum(cascade, k) * dk * dk * 0.5).sqrt()
}

#[derive(Clone, Copy, Debug)]
struct Mode {
    cascade: usize,
    /// Integer wave-vector index within the cascade tile.
    index: (i32, i32),
    k: DVec2,
    omega: f64,
    /// h0(k) and conj(h0(-k)), already scaled to unit total height sigma.
    h0: DVec2,
    h0_minus_conj: DVec2,
}

struct Spectrum {
    /// Per cascade, row-major (m, n) texels of (h0, conj(h0(-k))).
    texels: Vec<[f32; 4]>,
    omegas: Vec<f32>,
    /// Modes the CPU sums, all non-zero modes of the first `CPU_CASCADES`.
    cpu_modes: Vec<Mode>,
    /// Multiplier from `raw_h0` to unit total sigma.
    #[cfg_attr(not(test), allow(dead_code))]
    scale: f64,
}

fn complex_mul(a: DVec2, b: DVec2) -> DVec2 {
    DVec2::new(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x)
}

fn conj(a: DVec2) -> DVec2 {
    DVec2::new(a.x, -a.y)
}

/// The time-evolved amplitude h(k, t), for waves travelling along +k.
#[cfg(test)]
fn evolve(h0: DVec2, h0_minus_conj: DVec2, omega: f64, time: f64) -> DVec2 {
    let phase = omega * time;
    let rotation = DVec2::new(phase.cos(), phase.sin());
    complex_mul(h0, conj(rotation)) + complex_mul(h0_minus_conj, rotation)
}

fn spectrum_data() -> &'static Spectrum {
    static SPECTRUM: OnceLock<Spectrum> = OnceLock::new();
    SPECTRUM.get_or_init(|| {
        let half = (N / 2) as i32;
        // Total variance of a real field f = sum h(k) e^{ikx} with Hermitian
        // h(k) = h0(k) + conj(h0(-k)) at t = 0: sum |h(k)|^2 in expectation
        // is 2 sum |h0|^2. Measure the realised sum rather than trusting the
        // expectation, so the drawn sea has exactly the stated sigma.
        let mut raw = vec![DVec2::ZERO; CASCADE_COUNT * N * N];
        for cascade in 0..CASCADE_COUNT {
            for m in -half..half {
                for n in -half..half {
                    let index = cascade * N * N + (m + half) as usize * N + (n + half) as usize;
                    raw[index] = raw_h0(cascade, n, m);
                }
            }
        }
        let at = |cascade: usize, n: i32, m: i32| -> DVec2 {
            if n <= -half || m <= -half || n >= half || m >= half {
                DVec2::ZERO
            } else {
                raw[cascade * N * N + (m + half) as usize * N + (n + half) as usize]
            }
        };
        let mut variance = 0.0;
        for cascade in 0..CASCADE_COUNT {
            for m in -half..half {
                for n in -half..half {
                    let h = at(cascade, n, m) + conj(at(cascade, -n, -m));
                    variance += h.length_squared();
                }
            }
        }
        let scale = 1.0 / variance.sqrt();
        let mut texels = Vec::with_capacity(CASCADE_COUNT * N * N);
        let mut omegas = Vec::with_capacity(CASCADE_COUNT * N * N);
        let mut cpu_modes = Vec::new();
        for cascade in 0..CASCADE_COUNT {
            for m in -half..half {
                for n in -half..half {
                    let h0 = at(cascade, n, m) * scale;
                    let h0_minus_conj = conj(at(cascade, -n, -m)) * scale;
                    let k = wave_vector(cascade, n, m);
                    let omega = quantised_omega(k.length());
                    texels.push([
                        h0.x as f32,
                        h0.y as f32,
                        h0_minus_conj.x as f32,
                        h0_minus_conj.y as f32,
                    ]);
                    omegas.push(omega as f32);
                    if cascade < CPU_CASCADES
                        && (h0.length_squared() + h0_minus_conj.length_squared()) > 0.0
                    {
                        cpu_modes.push(Mode {
                            cascade,
                            index: (n, m),
                            k,
                            omega,
                            h0,
                            h0_minus_conj,
                        });
                    }
                }
            }
        }
        Spectrum {
            texels,
            omegas,
            cpu_modes,
            scale,
        }
    })
}

/// A planar sample of one projection, in that plane's (u, v) coordinates,
/// at unit sigma.
#[derive(Clone, Copy, Debug, Default)]
struct PlanarSample {
    height: f64,
    displacement: DVec2,
    gradient: DVec2,
    vertical_velocity: f64,
}

/// A mode advanced to one instant: its amplitude and that amplitude's rate.
#[derive(Clone, Copy)]
struct EvolvedMode {
    k: DVec2,
    unit_k: DVec2,
    h: DVec2,
    rate: DVec2,
}

thread_local! {
    /// Buoyancy asks for many points at one instant. Advancing every mode in
    /// time once per instant, not once per point, halves the trigonometry.
    static EVOLVED: std::cell::RefCell<(u64, Vec<EvolvedMode>)> =
        const { std::cell::RefCell::new((u64::MAX, Vec::new())) };
}

fn evolved_modes<R>(time: f64, use_modes: impl FnOnce(&[EvolvedMode]) -> R) -> R {
    EVOLVED.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.0 != time.to_bits() || cache.1.is_empty() {
            cache.1.clear();
            for mode in &spectrum_data().cpu_modes {
                let phase = mode.omega * time;
                let rotation = DVec2::new(phase.cos(), phase.sin());
                let forward = complex_mul(mode.h0, conj(rotation));
                let backward = complex_mul(mode.h0_minus_conj, rotation);
                // d/dt: -i w forward + i w backward
                let rate = complex_mul(backward - forward, DVec2::new(0.0, mode.omega));
                cache.1.push(EvolvedMode {
                    k: mode.k,
                    unit_k: mode.k / mode.k.length(),
                    h: forward + backward,
                    rate,
                });
            }
            cache.0 = time.to_bits();
        }
        use_modes(&cache.1)
    })
}

fn planar_sample(point: DVec2, time: f64, choppiness: f64) -> PlanarSample {
    let time = time.rem_euclid(REPEAT_SECONDS);
    evolved_modes(time, |modes| {
        let mut sample = PlanarSample::default();
        for mode in modes {
            let (sin, cos) = mode.k.dot(point).sin_cos();
            // Real part of h e^{ikx} and of i h e^{ikx}.
            let real = mode.h.x * cos - mode.h.y * sin;
            let imaginary = mode.h.x * sin + mode.h.y * cos;
            sample.height += real;
            // d/dx of Re(h e^{ikx}) = Re(i k h e^{ikx}) = -k Im(h e^{ikx}).
            sample.gradient -= mode.k * imaginary;
            // D = Re(-i k/|k| h e^{ikx}) = (k/|k|) Im(h e^{ikx}).
            sample.displacement += mode.unit_k * (imaginary * choppiness);
            sample.vertical_velocity += mode.rate.x * cos - mode.rate.y * sin;
        }
        sample
    })
}

/// The three projection planes: normal axis, then (u, v) axes.
const PROJECTIONS: [(DVec3, DVec3, DVec3); 3] = [
    (DVec3::X, DVec3::Y, DVec3::Z),
    (DVec3::Y, DVec3::Z, DVec3::X),
    (DVec3::Z, DVec3::X, DVec3::Y),
];
pub const PROJECTION_BLEND_EXPONENT: i32 = 8;
/// Projections weighted below this are skipped, on both CPU and GPU.
pub const PROJECTION_MINIMUM_WEIGHT: f64 = 0.01;

/// Normalised projection weights and the variance-preserving divisor.
fn projection_weights(direction: DVec3) -> ([f64; 3], f64) {
    let mut weights = [0.0; 3];
    for (index, (axis, _, _)) in PROJECTIONS.iter().enumerate() {
        weights[index] = direction.dot(*axis).abs().powi(PROJECTION_BLEND_EXPONENT);
    }
    let sum: f64 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= sum;
        if *weight < PROJECTION_MINIMUM_WEIGHT {
            *weight = 0.0;
        }
    }
    let norm = weights.iter().map(|w| w * w).sum::<f64>().sqrt();
    (weights, norm)
}

/// FFT sea at a parameter direction (before its own horizontal displacement),
/// in the planet frame, at unit sigma.
#[derive(Clone, Copy, Debug, Default)]
pub struct FftSample {
    /// The undisplaced point whose water lands on the query radial.
    pub parameter: DVec3,
    pub height: f64,
    /// Tangent horizontal displacement, metres.
    pub horizontal: DVec3,
    /// Tangent gradient of height with respect to the parameter point.
    pub slope: DVec3,
    pub vertical_velocity: f64,
}

fn sample_parameter(direction: DVec3, planar_at: &dyn Fn(DVec2) -> PlanarSample) -> FftSample {
    let direction = direction.normalize();
    let point = direction * planet_radius_meters();
    let (weights, norm) = projection_weights(direction);
    let mut result = FftSample::default();
    for (index, (_, u_axis, v_axis)) in PROJECTIONS.iter().enumerate() {
        let weight = weights[index];
        if weight == 0.0 {
            continue;
        }
        let planar = planar_at(DVec2::new(point.dot(*u_axis), point.dot(*v_axis)));
        let u_tangent = *u_axis - direction * direction.dot(*u_axis);
        let v_tangent = *v_axis - direction * direction.dot(*v_axis);
        let w = weight / norm;
        result.height += w * planar.height;
        result.vertical_velocity += w * planar.vertical_velocity;
        result.horizontal +=
            (u_tangent * planar.displacement.x + v_tangent * planar.displacement.y) * w;
        result.slope += (u_tangent * planar.gradient.x + v_tangent * planar.gradient.y) * w;
    }
    result
}

/// FFT sea under a *world* radial: finds the parameter point whose choppy
/// displacement lands here, then reports the sea there. Heights are scaled
/// to `sigma`; the sideways displacement additionally by `horizontal_weight`
/// (it fades out as the water shoals).
///
/// Reads the CPU grids (`cpu_grid`), not the mode sum: birds alone make
/// thousands of queries a frame, and summing ~1,200 modes for each cost 155ms
/// of simulation. `sample_world_exact` is the mode sum the grids are tested
/// against.
pub fn sample_world(direction: DVec3, time: f64, sigma: f64, horizontal_weight: f64) -> FftSample {
    let snapshots = cpu_grid::snapshots(time);
    world_query(direction, sigma, horizontal_weight, &|point| {
        cpu_grid::sample(&snapshots, point)
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn sample_world_exact(
    direction: DVec3,
    time: f64,
    sigma: f64,
    horizontal_weight: f64,
) -> FftSample {
    world_query(direction, sigma, horizontal_weight, &|point| {
        planar_sample(point, time, CHOPPINESS)
    })
}

fn world_query(
    direction: DVec3,
    sigma: f64,
    horizontal_weight: f64,
    planar_at: &dyn Fn(DVec2) -> PlanarSample,
) -> FftSample {
    let target = direction.normalize();
    let radius = planet_radius_meters();
    let horizontal_scale = sigma * horizontal_weight;
    let mut parameter = target;
    let mut sample = sample_parameter(parameter, planar_at);
    if horizontal_scale > 0.0 {
        for _ in 0..4 {
            // x0 = x - D(x0): a contraction wherever the sea does not fold.
            parameter = (target - sample.horizontal * (horizontal_scale / radius)).normalize();
            sample = sample_parameter(parameter, planar_at);
        }
    }
    FftSample {
        parameter,
        height: sample.height * sigma,
        horizontal: sample.horizontal * horizontal_scale,
        slope: sample.slope * sigma,
        vertical_velocity: sample.vertical_velocity * sigma,
    }
}

/// The CPU cascades as periodic grids, inverse-FFT'd from the same modes the
/// exact sum uses, at instants on a fixed `STEP_SECONDS` lattice. A query
/// interpolates bicubically in space and linearly between the two snapshots
/// around its time. Birds look up to 4s ahead, so a snapshot is built
/// once and reused for its whole look-ahead:
/// ten new snapshots a second, whoever is asking.
mod cpu_grid {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    use glam::DVec2;

    use super::{
        CASCADE_SIZES_METERS, CHOPPINESS, CPU_CASCADES, PlanarSample, REPEAT_SECONDS, complex_mul,
        conj, spectrum_data,
    };

    /// Grid side. Every CPU mode's index is under 17, so 64 would hold them
    /// all, but at 64 the 1000m cascade's texels are 15.6m against its 60m
    /// shortest wave and interpolation lost 0.47m of height. 128 with a cubic
    /// filter is the pair measured by `ocean_fft_cpu_grids_match_the_mode_sum`.
    pub(super) const SIDE: usize = 128;
    /// A 15m wave turns 0.2 rad in this step; linear interpolation in time
    /// loses under 0.5% of its amplitude.
    pub(super) const STEP_SECONDS: f64 = 0.1;
    /// Snapshots kept: 4s of bird look-ahead plus the steps around it.
    const CAPACITY: usize = 56;
    const LATTICE: u64 = (REPEAT_SECONDS / STEP_SECONDS) as u64;

    /// Per cascade, per texel: height, vertical velocity, Dx, Dz, dh/dx, dh/dz
    /// at unit sigma.
    pub(super) struct Snapshot {
        cascades: Vec<Vec<[f32; 6]>>,
    }

    pub(super) struct Snapshots {
        first: Arc<Snapshot>,
        second: Option<(Arc<Snapshot>, f64)>,
    }

    struct Cache {
        clock: u64,
        entries: HashMap<u64, (u64, Arc<Snapshot>)>,
    }

    fn cache() -> &'static Mutex<Cache> {
        static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
        CACHE.get_or_init(|| {
            Mutex::new(Cache {
                clock: 0,
                entries: HashMap::new(),
            })
        })
    }

    fn snapshot(index: u64) -> Arc<Snapshot> {
        let mut cache = cache().lock().expect("ocean grid cache");
        cache.clock += 1;
        let clock = cache.clock;
        if let Some(entry) = cache.entries.get_mut(&index) {
            entry.0 = clock;
            return entry.1.clone();
        }
        if cache.entries.len() >= CAPACITY {
            let oldest = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.0)
                .map(|(key, _)| *key)
                .expect("cache is full");
            cache.entries.remove(&oldest);
        }
        let built = Arc::new(build(index as f64 * STEP_SECONDS));
        cache.entries.insert(index, (clock, built.clone()));
        built
    }

    pub(super) fn snapshots(time: f64) -> Snapshots {
        let steps = time.rem_euclid(REPEAT_SECONDS) / STEP_SECONDS;
        let rounded = steps.round();
        // Bird and ship steps land on the lattice up to rounding: one grid.
        if (steps - rounded).abs() < 1.0e-6 {
            return Snapshots {
                first: snapshot(rounded as u64 % LATTICE),
                second: None,
            };
        }
        let floor = steps.floor();
        let index = floor as u64 % LATTICE;
        Snapshots {
            first: snapshot(index),
            second: Some((snapshot((index + 1) % LATTICE), steps - floor)),
        }
    }

    fn inverse_fft(data: &mut [DVec2]) {
        let n = data.len();
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                data.swap(i, j);
            }
        }
        let mut half = 1;
        while half < n {
            for pos in 0..half {
                let angle = std::f64::consts::PI * pos as f64 / half as f64;
                let w = DVec2::new(angle.cos(), angle.sin());
                for start in (0..n).step_by(half * 2) {
                    let a = data[start + pos];
                    let b = complex_mul(data[start + pos + half], w);
                    data[start + pos] = a + b;
                    data[start + pos + half] = a - b;
                }
            }
            half *= 2;
        }
    }

    fn inverse_fft_2d(grid: &mut [DVec2]) {
        for row in grid.chunks_exact_mut(SIDE) {
            inverse_fft(row);
        }
        let mut column = vec![DVec2::ZERO; SIDE];
        for x in 0..SIDE {
            for y in 0..SIDE {
                column[y] = grid[y * SIDE + x];
            }
            inverse_fft(&mut column);
            for y in 0..SIDE {
                grid[y * SIDE + x] = column[y];
            }
        }
    }

    fn times_i(z: DVec2) -> DVec2 {
        DVec2::new(-z.y, z.x)
    }

    fn build(time: f64) -> Snapshot {
        let side = SIDE as i32;
        let mut cascades = Vec::with_capacity(CPU_CASCADES);
        for cascade in 0..CPU_CASCADES {
            // Two real fields per complex grid: f1 + i f2.
            let mut height_velocity = vec![DVec2::ZERO; SIDE * SIDE];
            let mut displacement = vec![DVec2::ZERO; SIDE * SIDE];
            let mut gradient = vec![DVec2::ZERO; SIDE * SIDE];
            for mode in spectrum_data()
                .cpu_modes
                .iter()
                .filter(|mode| mode.cascade == cascade)
            {
                let phase = mode.omega * time;
                let rotation = DVec2::new(phase.cos(), phase.sin());
                let h = complex_mul(mode.h0, conj(rotation))
                    + complex_mul(mode.h0_minus_conj, rotation);
                let rate = complex_mul(
                    complex_mul(mode.h0_minus_conj, rotation)
                        - complex_mul(mode.h0, conj(rotation)),
                    DVec2::new(0.0, mode.omega),
                );
                let unit = mode.k / mode.k.length();
                let (n, m) = mode.index;
                let texel = m.rem_euclid(side) as usize * SIDE + n.rem_euclid(side) as usize;
                height_velocity[texel] += h + times_i(rate);
                // D = -i (k/|k|) h, scaled by the choppiness.
                let minus_i_h = DVec2::new(h.y, -h.x) * CHOPPINESS;
                displacement[texel] += minus_i_h * unit.x + times_i(minus_i_h * unit.y);
                let i_h = times_i(h);
                gradient[texel] += i_h * mode.k.x + times_i(i_h * mode.k.y);
            }
            inverse_fft_2d(&mut height_velocity);
            inverse_fft_2d(&mut displacement);
            inverse_fft_2d(&mut gradient);
            cascades.push(
                (0..SIDE * SIDE)
                    .map(|texel| {
                        [
                            height_velocity[texel].x as f32,
                            height_velocity[texel].y as f32,
                            displacement[texel].x as f32,
                            displacement[texel].y as f32,
                            gradient[texel].x as f32,
                            gradient[texel].y as f32,
                        ]
                    })
                    .collect(),
            );
        }
        Snapshot { cascades }
    }

    /// Catmull-Rom weights for the four taps around a fraction.
    fn cubic_weights(t: f64) -> [f64; 4] {
        let t2 = t * t;
        let t3 = t2 * t;
        [
            0.5 * (-t3 + 2.0 * t2 - t),
            0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
            0.5 * (-3.0 * t3 + 4.0 * t2 + t),
            0.5 * (t3 - t2),
        ]
    }

    fn interpolate(snapshot: &Snapshot, point: DVec2) -> [f64; 6] {
        let mut out = [0.0; 6];
        let side = SIDE as i64;
        for (cascade, grid) in snapshot.cascades.iter().enumerate() {
            let scaled = point * (SIDE as f64 / CASCADE_SIZES_METERS[cascade]);
            let base = scaled.floor();
            let fraction = scaled - base;
            let (bx, by) = (base.x as i64, base.y as i64);
            let wx = cubic_weights(fraction.x);
            let wy = cubic_weights(fraction.y);
            for (row, weight_y) in wy.iter().enumerate() {
                let y = (by + row as i64 - 1).rem_euclid(side) as usize;
                for (column, weight_x) in wx.iter().enumerate() {
                    let x = (bx + column as i64 - 1).rem_euclid(side) as usize;
                    let weight = weight_x * weight_y;
                    let texel = &grid[y * SIDE + x];
                    for (value, field) in out.iter_mut().zip(texel) {
                        *value += weight * f64::from(*field);
                    }
                }
            }
        }
        out
    }

    pub(super) fn sample(snapshots: &Snapshots, point: DVec2) -> PlanarSample {
        let mut fields = interpolate(&snapshots.first, point);
        if let Some((second, fraction)) = &snapshots.second {
            let later = interpolate(second, point);
            for (value, later) in fields.iter_mut().zip(later) {
                *value += (later - *value) * fraction;
            }
        }
        PlanarSample {
            height: fields[0],
            vertical_velocity: fields[1],
            displacement: DVec2::new(fields[2], fields[3]),
            gradient: DVec2::new(fields[4], fields[5]),
        }
    }
}

/// Generated WGSL: constants plus either the sampling function or a stub.
pub(crate) fn wgsl_source() -> String {
    let mut out = format!(
        "const OCEAN_FFT_ENABLED: bool = {};\n\
         const OCEAN_FFT_N: f32 = {:.1};\n\
         const OCEAN_FFT_CASCADE_SIZES: vec3<f32> = vec3<f32>({:.4}, {:.4}, {:.4});\n\
         const OCEAN_FFT_CALM_SIGMA: f32 = {:.6};\n\
         const OCEAN_FFT_STORM_SIGMA: f32 = {:.6};\n\
         const OCEAN_FFT_PROJECTION_EXPONENT: f32 = {:.1};\n\
         const OCEAN_FFT_PROJECTION_MINIMUM_WEIGHT: f32 = {:.6};\n\
         const OCEAN_FFT_REPLACES_FROM: u32 = {}u;\n",
        enabled(),
        N as f64,
        CASCADE_SIZES_METERS[0],
        CASCADE_SIZES_METERS[1],
        CASCADE_SIZES_METERS[2],
        CALM_HEIGHT_SIGMA_METERS,
        STORM_HEIGHT_SIGMA_METERS,
        PROJECTION_BLEND_EXPONENT as f64,
        PROJECTION_MINIMUM_WEIGHT,
        crate::ocean::FFT_REPLACES_FROM,
    );
    if enabled() {
        out.push_str(include_str!("ocean_fft_sample.wgsl"));
    } else {
        out.push_str(
            "struct OceanFftSample { horizontal: vec3<f32>, height: f32, slope: vec3<f32>, crest: f32, }\n\
             fn ocean_fft_sample(direction: vec3<f32>, camera_relative: vec3<f32>, camera_distance_meters: f32, footprint_floor_meters: f32, sigma: f32) -> OceanFftSample {\n\
                 return OceanFftSample(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 0.0);\n\
             }\n\
             fn ocean_fft_camera_position_uniform() -> vec3<f32> { return vec3<f32>(0.0); }\n",
        );
    }
    out
}

// ---------------------------------------------------------------------------
// GPU

const MIP_LEVELS: u32 = LOG2_N + 1;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ComputeParams {
    time: f32,
    choppiness: f32,
    _pad: [f32; 2],
}

/// Per-frame values the render side needs: where the camera sits inside each
/// cascade tile (wrapped in f64 on the CPU), and the pixel footprint.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RenderParams {
    /// Per cascade: fract(camera.xyz / L).
    camera_tile_offsets: [[f32; 4]; CASCADE_COUNT],
    /// xyz: camera planet-frame position (approximate, for callers without a
    /// camera-relative point); w: radians per pixel.
    camera_position_pixel_angle: [f32; 4],
}

pub(crate) struct OceanFftGpu {
    params: wgpu::Buffer,
    render_params: wgpu::Buffer,
    spectrum_pipeline: wgpu::ComputePipeline,
    rows_pipeline: wgpu::ComputePipeline,
    columns_pipeline: wgpu::ComputePipeline,
    resolve_pipeline: wgpu::ComputePipeline,
    mip_pipeline: wgpu::ComputePipeline,
    spectrum_bind_group: wgpu::BindGroup,
    rows_bind_group: wgpu::BindGroup,
    columns_bind_group: wgpu::BindGroup,
    resolve_bind_group: wgpu::BindGroup,
    mip_bind_groups: Vec<wgpu::BindGroup>,
    #[cfg_attr(not(test), allow(dead_code))]
    displacement: wgpu::Texture,
    #[cfg_attr(not(test), allow(dead_code))]
    derivatives: wgpu::Texture,
    displacement_view: wgpu::TextureView,
    derivatives_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    pub(crate) last_time: Option<f64>,
}

fn storage_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2Array,
        },
        count: None,
    }
}

fn read_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn array_texture(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    mips: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: CASCADE_COUNT as u32,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn array_view(texture: &wgpu::Texture, mip: Option<u32>) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        base_mip_level: mip.unwrap_or(0),
        mip_level_count: mip.map(|_| 1),
        ..Default::default()
    })
}

impl OceanFftGpu {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let spectrum = spectrum_data();
        let rgba32 = wgpu::TextureFormat::Rgba32Float;
        let rgba16 = wgpu::TextureFormat::Rgba16Float;
        let storage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let h0 = array_texture(
            device,
            "ocean fft h0",
            rgba32,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let omega = array_texture(
            device,
            "ocean fft omega",
            wgpu::TextureFormat::R32Float,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            h0.as_image_copy(),
            bytemuck::cast_slice(&spectrum.texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((N * 16) as u32),
                rows_per_image: Some(N as u32),
            },
            h0.size(),
        );
        queue.write_texture(
            omega.as_image_copy(),
            bytemuck::cast_slice(&spectrum.omegas),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((N * 4) as u32),
                rows_per_image: Some(N as u32),
            },
            omega.size(),
        );
        let frequency_a = array_texture(device, "ocean fft frequency a", rgba32, 1, storage);
        let frequency_b = array_texture(device, "ocean fft frequency b", rgba32, 1, storage);
        let temporary_a = array_texture(device, "ocean fft row pass a", rgba32, 1, storage);
        let temporary_b = array_texture(device, "ocean fft row pass b", rgba32, 1, storage);
        let displacement = array_texture(
            device,
            "ocean fft displacement",
            rgba16,
            MIP_LEVELS,
            storage | wgpu::TextureUsages::COPY_SRC,
        );
        let derivatives = array_texture(
            device,
            "ocean fft derivatives",
            rgba16,
            MIP_LEVELS,
            storage | wgpu::TextureUsages::COPY_SRC,
        );
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean fft params"),
            contents: bytemuck::bytes_of(&ComputeParams {
                time: 0.0,
                choppiness: CHOPPINESS as f32,
                _pad: [0.0; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let render_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean fft render params"),
            contents: bytemuck::bytes_of(&<RenderParams as bytemuck::Zeroable>::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean fft compute"),
            source: wgpu::ShaderSource::Wgsl(compute_shader_source().into()),
        });
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let spectrum_layout = layout(
            "ocean fft spectrum layout",
            &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                read_entry(1),
                read_entry(2),
                storage_entry(3, rgba32),
                storage_entry(4, rgba32),
            ],
        );
        let fft_layout = layout(
            "ocean fft butterfly layout",
            &[
                read_entry(5),
                read_entry(6),
                storage_entry(7, rgba32),
                storage_entry(8, rgba32),
            ],
        );
        let resolve_layout = layout(
            "ocean fft resolve layout",
            &[
                read_entry(9),
                read_entry(10),
                storage_entry(11, rgba16),
                storage_entry(12, rgba16),
            ],
        );
        let mip_layout = layout(
            "ocean fft mip layout",
            &[
                read_entry(13),
                read_entry(14),
                storage_entry(15, rgba16),
                storage_entry(16, rgba16),
            ],
        );
        let pipeline = |label: &str, bind_layout: &wgpu::BindGroupLayout, entry: &str| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bind_layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let spectrum_pipeline = pipeline("ocean fft spectrum", &spectrum_layout, "cs_spectrum");
        let rows_pipeline = pipeline("ocean fft rows", &fft_layout, "cs_fft_rows");
        let columns_pipeline = pipeline("ocean fft columns", &fft_layout, "cs_fft_columns");
        let resolve_pipeline = pipeline("ocean fft resolve", &resolve_layout, "cs_resolve");
        let mip_pipeline = pipeline("ocean fft mips", &mip_layout, "cs_mip");
        let view = |texture: &wgpu::Texture| array_view(texture, None);
        let bind =
            |label: &str, bind_layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: bind_layout,
                    entries,
                })
            };
        fn texture_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            }
        }
        let (h0_view, omega_view) = (view(&h0), view(&omega));
        let (frequency_a_view, frequency_b_view) = (view(&frequency_a), view(&frequency_b));
        let (temporary_a_view, temporary_b_view) = (view(&temporary_a), view(&temporary_b));
        let spectrum_bind_group = bind(
            "ocean fft spectrum",
            &spectrum_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                texture_entry(1, &h0_view),
                texture_entry(2, &omega_view),
                texture_entry(3, &frequency_a_view),
                texture_entry(4, &frequency_b_view),
            ],
        );
        let rows_bind_group = bind(
            "ocean fft rows",
            &fft_layout,
            &[
                texture_entry(5, &frequency_a_view),
                texture_entry(6, &frequency_b_view),
                texture_entry(7, &temporary_a_view),
                texture_entry(8, &temporary_b_view),
            ],
        );
        let columns_bind_group = bind(
            "ocean fft columns",
            &fft_layout,
            &[
                texture_entry(5, &temporary_a_view),
                texture_entry(6, &temporary_b_view),
                texture_entry(7, &frequency_a_view),
                texture_entry(8, &frequency_b_view),
            ],
        );
        let displacement_mips: Vec<_> = (0..MIP_LEVELS)
            .map(|mip| array_view(&displacement, Some(mip)))
            .collect();
        let derivative_mips: Vec<_> = (0..MIP_LEVELS)
            .map(|mip| array_view(&derivatives, Some(mip)))
            .collect();
        let resolve_bind_group = bind(
            "ocean fft resolve",
            &resolve_layout,
            &[
                texture_entry(9, &frequency_a_view),
                texture_entry(10, &frequency_b_view),
                texture_entry(11, &displacement_mips[0]),
                texture_entry(12, &derivative_mips[0]),
            ],
        );
        let mip_bind_groups = (1..MIP_LEVELS as usize)
            .map(|mip| {
                bind(
                    "ocean fft mip",
                    &mip_layout,
                    &[
                        texture_entry(13, &displacement_mips[mip - 1]),
                        texture_entry(14, &derivative_mips[mip - 1]),
                        texture_entry(15, &displacement_mips[mip]),
                        texture_entry(16, &derivative_mips[mip]),
                    ],
                )
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ocean fft repeat sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let displacement_view = view(&displacement);
        let derivatives_view = view(&derivatives);
        Self {
            params,
            render_params,
            spectrum_pipeline,
            rows_pipeline,
            columns_pipeline,
            resolve_pipeline,
            mip_pipeline,
            spectrum_bind_group,
            rows_bind_group,
            columns_bind_group,
            resolve_bind_group,
            mip_bind_groups,
            displacement,
            derivatives,
            displacement_view,
            derivatives_view,
            sampler,
            last_time: None,
        }
    }

    pub(crate) fn displacement_view(&self) -> &wgpu::TextureView {
        &self.displacement_view
    }

    pub(crate) fn derivatives_view(&self) -> &wgpu::TextureView {
        &self.derivatives_view
    }

    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub(crate) fn render_params(&self) -> &wgpu::Buffer {
        &self.render_params
    }

    #[cfg(test)]
    pub(crate) fn textures(&self) -> (&wgpu::Texture, &wgpu::Texture) {
        (&self.displacement, &self.derivatives)
    }

    /// Records the FFT for `time` (seconds, the ocean clock) and the render
    /// parameters for a camera at `camera_position` (planet frame, metres).
    pub(crate) fn update(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        time: f64,
        camera_position: DVec3,
        radians_per_pixel: f64,
    ) {
        let mut offsets = [[0.0_f32; 4]; CASCADE_COUNT];
        for (cascade, offset) in offsets.iter_mut().enumerate() {
            let size = CASCADE_SIZES_METERS[cascade];
            for axis in 0..3 {
                offset[axis] = (camera_position[axis] / size).rem_euclid(1.0) as f32;
            }
        }
        queue.write_buffer(
            &self.render_params,
            0,
            bytemuck::bytes_of(&RenderParams {
                camera_tile_offsets: offsets,
                camera_position_pixel_angle: [
                    camera_position.x as f32,
                    camera_position.y as f32,
                    camera_position.z as f32,
                    radians_per_pixel as f32,
                ],
            }),
        );
        if self.last_time == Some(time) {
            return;
        }
        self.last_time = Some(time);
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&ComputeParams {
                time: time.rem_euclid(REPEAT_SECONDS) as f32,
                choppiness: CHOPPINESS as f32,
                _pad: [0.0; 2],
            }),
        );
        self.encode(encoder);
    }

    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ocean fft"),
            timestamp_writes: None,
        });
        let layers = CASCADE_COUNT as u32;
        let side = N as u32;
        pass.set_pipeline(&self.spectrum_pipeline);
        pass.set_bind_group(0, &self.spectrum_bind_group, &[]);
        pass.dispatch_workgroups(side / 8, side / 8, layers);
        pass.set_pipeline(&self.rows_pipeline);
        pass.set_bind_group(0, &self.rows_bind_group, &[]);
        pass.dispatch_workgroups(side, 1, layers);
        pass.set_pipeline(&self.columns_pipeline);
        pass.set_bind_group(0, &self.columns_bind_group, &[]);
        pass.dispatch_workgroups(side, 1, layers);
        pass.set_pipeline(&self.resolve_pipeline);
        pass.set_bind_group(0, &self.resolve_bind_group, &[]);
        pass.dispatch_workgroups(side / 8, side / 8, layers);
        pass.set_pipeline(&self.mip_pipeline);
        for (index, bind_group) in self.mip_bind_groups.iter().enumerate() {
            let mip_side = (side >> (index + 1)).max(1);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(mip_side.div_ceil(8), mip_side.div_ceil(8), layers);
        }
    }
}

fn compute_shader_source() -> String {
    format!(
        "const FFT_N: u32 = {}u;\nconst FFT_LOG2_N: u32 = {}u;\nconst FFT_CASCADE_SIZES: vec3<f32> = vec3<f32>({:.4}, {:.4}, {:.4});\n{}",
        N,
        LOG2_N,
        CASCADE_SIZES_METERS[0],
        CASCADE_SIZES_METERS[1],
        CASCADE_SIZES_METERS[2],
        include_str!("ocean_fft.wgsl")
    )
}

// ---------------------------------------------------------------------------
// CPU reference FFT, for tests. The same radix-2 inverse transform and index
// convention as the compute shader.

#[cfg(test)]
pub(crate) mod reference {
    use super::*;

    fn inverse_fft_in_place(data: &mut [DVec2]) {
        let n = data.len();
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                data.swap(i, j);
            }
        }
        let mut half = 1;
        while half < n {
            for start in (0..n).step_by(half * 2) {
                for pos in 0..half {
                    let angle = std::f64::consts::PI * pos as f64 / half as f64;
                    let w = DVec2::new(angle.cos(), angle.sin());
                    let a = data[start + pos];
                    let b = complex_mul(data[start + pos + half], w);
                    data[start + pos] = a + b;
                    data[start + pos + half] = a - b;
                }
            }
            half *= 2;
        }
    }

    /// Height grid of one cascade at unit sigma: value at texel (x, y) is the
    /// field at planar point (x, y) * L / N.
    pub(crate) fn height_grid(cascade: usize, time: f64) -> Vec<f64> {
        let spectrum = spectrum_data();
        let time = time.rem_euclid(REPEAT_SECONDS);
        let half = (N / 2) as i32;
        let mut grid = vec![DVec2::ZERO; N * N];
        for index in 0..N * N {
            let n = (index % N) as i32 - half;
            let m = (index / N) as i32 - half;
            let h0 = raw_h0(cascade, n, m) * spectrum.scale;
            let h0_minus_conj = conj(raw_h0(cascade, -n, -m)) * spectrum.scale;
            let omega = quantised_omega(wave_vector(cascade, n, m).length());
            grid[index] = evolve(h0, h0_minus_conj, omega, time);
        }
        for row in 0..N {
            inverse_fft_in_place(&mut grid[row * N..(row + 1) * N]);
        }
        let mut column = vec![DVec2::ZERO; N];
        for x in 0..N {
            for y in 0..N {
                column[y] = grid[y * N + x];
            }
            inverse_fft_in_place(&mut column);
            for y in 0..N {
                grid[y * N + x] = column[y];
            }
        }
        (0..N * N)
            .map(|index| {
                let (x, y) = (index % N, index / N);
                let sign = if (x + y) % 2 == 0 { 1.0 } else { -1.0 };
                grid[index].x * sign
            })
            .collect()
    }

    pub(crate) fn planar_height(point: DVec2, time: f64) -> f64 {
        super::planar_sample(point, time, CHOPPINESS).height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocean_fft_spectrum_has_unit_sigma_and_no_duplicate_modes() {
        let spectrum = spectrum_data();
        let mut variance = 0.0;
        for cascade in 0..CASCADE_COUNT {
            let grid = reference::height_grid(cascade, 0.0);
            let mean = grid.iter().sum::<f64>() / grid.len() as f64;
            assert!(mean.abs() < 1.0e-9, "cascade {cascade} mean {mean}");
            variance += grid.iter().map(|h| h * h).sum::<f64>() / grid.len() as f64;
        }
        assert!((variance - 1.0).abs() < 1.0e-6, "total variance {variance}");
        assert!(
            spectrum.cpu_modes.len() > 500,
            "{}",
            spectrum.cpu_modes.len()
        );
    }

    #[test]
    fn ocean_fft_cpu_matches_cpu_fft_grid() {
        for cascade in 0..CPU_CASCADES {
            let time = 37.25;
            let grid = reference::height_grid(cascade, time);
            let spacing = CASCADE_SIZES_METERS[cascade] / N as f64;
            // The CPU DFT sums the first CPU_CASCADES together, so compare the
            // sum of their grids where both have texels (every texel of the
            // coarse cascade that also lies on the finer one's lattice is
            // rare); instead evaluate each cascade's grid alone by DFT.
            let modes: Vec<_> = spectrum_data()
                .cpu_modes
                .iter()
                .filter(|mode| {
                    let per_meter = std::f64::consts::TAU / CASCADE_SIZES_METERS[cascade];
                    let n = (mode.k / per_meter).round();
                    (mode.k - n * per_meter).length() < 1.0e-9
                        && spectrum_for_cascade(cascade, mode.k)
                })
                .copied()
                .collect();
            for (x, y) in [(0, 0), (17, 3), (128, 200), (255, 255), (64, 31)] {
                let point = DVec2::new(x as f64, y as f64) * spacing;
                let mut height = 0.0;
                for mode in &modes {
                    let h = evolve(mode.h0, mode.h0_minus_conj, mode.omega, time);
                    let phase = mode.k.dot(point);
                    height += h.x * phase.cos() - h.y * phase.sin();
                }
                let expected = grid[y * N + x];
                assert!(
                    (height - expected).abs() < 1.0e-9,
                    "cascade {cascade} texel ({x},{y}): dft {height} fft {expected}"
                );
            }
        }
    }

    fn spectrum_for_cascade(cascade: usize, k: DVec2) -> bool {
        spectrum(cascade, k) > 0.0
    }

    #[test]
    fn ocean_fft_is_periodic_and_travels() {
        let point = DVec2::new(123.0, -45.0);
        let a = reference::planar_height(point, 12.0);
        let b = reference::planar_height(point, 12.0 + REPEAT_SECONDS);
        assert!((a - b).abs() < 1.0e-9);
        let c = reference::planar_height(point, 14.0);
        assert!((a - c).abs() > 1.0e-3, "the sea must move");
    }

    #[test]
    fn ocean_fft_gradient_matches_finite_difference() {
        let point = DVec2::new(311.0, 77.0);
        let time = 5.5;
        let sample = planar_sample(point, time, CHOPPINESS);
        let step = 0.01;
        let dx = (planar_sample(point + DVec2::X * step, time, CHOPPINESS).height
            - planar_sample(point - DVec2::X * step, time, CHOPPINESS).height)
            / (2.0 * step);
        let dy = (planar_sample(point + DVec2::Y * step, time, CHOPPINESS).height
            - planar_sample(point - DVec2::Y * step, time, CHOPPINESS).height)
            / (2.0 * step);
        assert!(
            (sample.gradient.x - dx).abs() < 1.0e-5,
            "{} {}",
            sample.gradient.x,
            dx
        );
        assert!(
            (sample.gradient.y - dy).abs() < 1.0e-5,
            "{} {}",
            sample.gradient.y,
            dy
        );
        let dt = 0.001;
        let rate = (planar_sample(point, time + dt, CHOPPINESS).height
            - planar_sample(point, time - dt, CHOPPINESS).height)
            / (2.0 * dt);
        assert!(
            (sample.vertical_velocity - rate).abs() < 1.0e-5,
            "{} {}",
            sample.vertical_velocity,
            rate
        );
    }

    #[test]
    fn ocean_fft_world_query_inverts_choppy_displacement() {
        let direction = DVec3::new(0.836, 0.504, 0.216).normalize();
        let time = 9.0;
        let sigma = STORM_HEIGHT_SIGMA_METERS;
        let world = sample_world_exact(direction, time, sigma, 1.0);
        // Check the parameter point's displacement lands on the query radial.
        let radius = planet_radius_meters();
        let parameter = world.parameter;
        let forward = sample_parameter(parameter, &|point| planar_sample(point, time, CHOPPINESS));
        let landed = (parameter * radius + forward.horizontal * sigma).normalize();
        let miss_meters = (landed - direction).length() * radius;
        assert!(miss_meters < 0.05, "inversion missed by {miss_meters}m");
    }

    /// The runtime reads grids, not the mode sum; this is what it gives up.
    /// Times off the lattice exercise the time interpolation too.
    #[test]
    fn ocean_fft_cpu_grids_match_the_mode_sum() {
        let base = DVec3::new(0.3, 0.5, -0.81).normalize();
        let east = base.cross(DVec3::Y).normalize();
        let north = base.cross(east);
        let radius = planet_radius_meters();
        let sigma = STORM_HEIGHT_SIGMA_METERS;
        let (mut height, mut velocity, mut slope, mut horizontal) =
            (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
        for (step, time) in [123.0, 123.0 + 1.0 / 120.0, 456.789]
            .into_iter()
            .enumerate()
        {
            for i in 0..12 {
                for j in 0..12 {
                    let offset = east * (i as f64 * 37.3 + step as f64) + north * (j as f64 * 29.1);
                    let direction = (base * radius + offset).normalize();
                    let grid = sample_world(direction, time, sigma, 1.0);
                    let exact = sample_world_exact(direction, time, sigma, 1.0);
                    height = height.max((grid.height - exact.height).abs());
                    velocity =
                        velocity.max((grid.vertical_velocity - exact.vertical_velocity).abs());
                    slope = slope.max((grid.slope - exact.slope).length());
                    horizontal = horizontal.max((grid.horizontal - exact.horizontal).length());
                }
            }
        }
        eprintln!(
            "worst grid error: height {height:.4}m, velocity {velocity:.4}m/s, slope {slope:.4}, horizontal {horizontal:.4}m"
        );
        // Measured 0.010m, 0.008m/s, 0.0007 and 0.006m; bilinear at 64 texels
        // was 0.47m, 0.50m/s, 0.043 and 0.48m.
        assert!(height < 0.03, "height {height}");
        assert!(velocity < 0.03, "velocity {velocity}");
        assert!(slope < 0.003, "slope {slope}");
        assert!(horizontal < 0.03, "horizontal {horizontal}");
    }

    fn f16_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let mantissa = f32::from(bits & 0x3ff);
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            31 => f32::NAN,
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    #[test]
    #[ignore = "requires a Vulkan GPU; run explicitly for FFT shader changes"]
    fn gpu_fft_matches_cpu_fft() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("Vulkan adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("GPU device");
        let mut gpu = OceanFftGpu::new(&device, &queue);
        let time = 1234.5;
        let mut encoder = device.create_command_encoder(&Default::default());
        gpu.update(&queue, &mut encoder, time, DVec3::ZERO, 0.0);
        let row_bytes = (N * 8) as u32;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fft readback"),
            size: u64::from(row_bytes) * (N * CASCADE_COUNT) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let (displacement, _) = gpu.textures();
        encoder.copy_texture_to_buffer(
            displacement.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(N as u32),
                },
            },
            wgpu::Extent3d {
                width: N as u32,
                height: N as u32,
                depth_or_array_layers: CASCADE_COUNT as u32,
            },
        );
        queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = readback.slice(..).get_mapped_range();
        let halves: &[u16] = bytemuck::cast_slice(&data);
        for cascade in 0..CASCADE_COUNT {
            let expected = reference::height_grid(cascade, time);
            let sigma =
                (expected.iter().map(|h| h * h).sum::<f64>() / expected.len() as f64).sqrt();
            let mut worst = 0.0_f64;
            for index in 0..N * N {
                let gpu_height = f64::from(f16_to_f32(halves[(cascade * N * N + index) * 4 + 1]));
                worst = worst.max((gpu_height - expected[index]).abs());
            }
            eprintln!("cascade {cascade}: sigma {sigma:.5}, worst |gpu - cpu| {worst:.6}");
            assert!(
                worst < sigma * 0.01 + 1.0e-4,
                "cascade {cascade}: worst {worst} against sigma {sigma}"
            );
        }
    }
}

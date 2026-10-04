//! GPU FFT ocean wave field (phase B of the Sea of Thieves plan). Standalone:
//! not yet wired into rendering or buoyancy.
#![allow(dead_code)]

use wgpu::util::DeviceExt;

pub const GRID: usize = 256;
pub const CASCADES: usize = 4;
/// Cascades 0-2 partition the wind sea by wavenumber; the last is swell.
pub const WIND_CASCADES: usize = 3;
pub const SWELL_CASCADE: usize = 3;
/// 256 down to 1 texel.
pub const MIP_LEVELS: u32 = 9;
/// Tile edge lengths in metres, chosen so the repeats do not line up.
pub const TILE_METERS: [f32; CASCADES] = [1000.0, 237.0, 53.0, 2170.0];

/// Global wavelength modifier: every wave (wind sea and swell) is this many
/// times longer, at the same height. A spatial stretch of the whole field:
/// the spectra are built on `TILE_METERS` and the tiles are laid out
/// `OCEAN_WAVELENGTH_SCALE` times larger, so longer waves travel slower (deep-
/// water dispersion from the stretched wavenumbers) and are gentler in the
/// same proportion. `PLANET_OCEAN_FFT_WAVELENGTH` overrides it (0.25-4).
pub const OCEAN_WAVELENGTH_SCALE: f32 = 1.0;

pub fn wavelength_scale() -> f32 {
    static VALUE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("PLANET_OCEAN_FFT_WAVELENGTH")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(OCEAN_WAVELENGTH_SCALE, |v| v.clamp(0.25, 4.0))
    })
}

/// Laid-out tile length of cascade `c` in metres (`TILE_METERS` stretched by
/// the wavelength modifier). Everything that places, samples or evolves the
/// field uses this; only the spectrum construction uses `TILE_METERS`.
pub fn tile_meters(c: usize) -> f32 {
    TILE_METERS[c] * wavelength_scale()
}
/// Wavenumber band owned by each wind cascade (rad/m); bands abut exactly.
pub const BAND_EDGES: [f32; WIND_CASCADES + 1] = [0.0, 0.5, 2.0, 1.0e9];
/// Swell: a narrow-band long-crested sea from distant weather, crossing the
/// local wind sea so their crests collide. Normalised to 1m significant height
/// and scaled at run time by `swell_height_meters`.
const SWELL_PEAK_WAVELENGTH_METERS: f32 = 170.0;
/// Bigger swell is longer swell: the peak wavelength is at least this many
/// times the (unstormed) significant height, so crests keep a sea's
/// steepness instead of pinching into spikes. 15m -> 180m, 20m -> 240m,
/// 30m -> 360m; the default 8m keeps 170m.
const SWELL_WAVELENGTH_PER_HEIGHT: f32 = 12.0;

/// Swell peak wavelength for the configured swell height.
pub fn swell_peak_wavelength_meters() -> f32 {
    swell_wavelength_for_height(swell_base_height_meters())
}

fn swell_wavelength_for_height(height_meters: f32) -> f32 {
    SWELL_PEAK_WAVELENGTH_METERS.max(SWELL_WAVELENGTH_PER_HEIGHT * height_meters)
}
/// Relative spread of angular frequency around the peak (Gaussian sigma).
const SWELL_FREQUENCY_SPREAD: f32 = 0.10;
/// cos^n directional spreading; large n is long-crested.
const SWELL_SPREAD_POWER: i32 = 16;
const SWELL_ANGLE_FROM_WIND_RADIANS: f32 = -0.52;
const GRAVITY: f32 = 9.81;

const WORKGROUPS_PER_LINE_PASS: u32 = GRID as u32;
const FFT_ARRAYS: u32 = (CASCADES * 2) as u32;

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn gaussian_pair(seed: u32) -> (f32, f32) {
    let a = (hash(seed) as f32 + 0.5) / 4_294_967_296.0;
    let b = (hash(seed ^ 0x9e37_79b9) as f32 + 0.5) / 4_294_967_296.0;
    let r = (-2.0 * a.ln()).sqrt();
    let t = std::f32::consts::TAU * b;
    (r * t.cos(), r * t.sin())
}

/// JONSWAP omnidirectional spectrum S(omega).
fn jonswap(omega: f32, wind_speed: f32, fetch_meters: f32) -> f32 {
    // The peak frequency divides by the wind speed: no wind, no wind sea.
    if wind_speed < MIN_WIND_SEA_METERS_PER_SECOND {
        return 0.0;
    }
    let alpha = 0.076 * (wind_speed * wind_speed / (fetch_meters * GRAVITY)).powf(0.22);
    let omega_peak = 22.0 * (GRAVITY * GRAVITY / (wind_speed * fetch_meters)).powf(1.0 / 3.0);
    let sigma = if omega <= omega_peak { 0.07 } else { 0.09 };
    let r = (-(omega - omega_peak).powi(2) / (2.0 * sigma * sigma * omega_peak * omega_peak)).exp();
    alpha * GRAVITY * GRAVITY / omega.powi(5)
        * (-1.25 * (omega_peak / omega).powi(4)).exp()
        * 3.3f32.powf(r)
}

/// Directional spreading, normalised over [-pi/2, pi/2] cos^2 lobe.
fn spread(theta: f32) -> f32 {
    let c = theta.cos();
    if c <= 0.0 {
        0.0
    } else {
        2.0 / std::f32::consts::PI * c * c
    }
}

/// Builds h0 texels as (h0(k).re, h0(k).im, h0(-k).re, h0(-k).im) for every
/// cascade, k indices centred (texel x <-> n = x - GRID/2).
pub fn generate_h0(seed: u32, wind_speed: f32, wind_dir: [f32; 2], fetch_meters: f32) -> Vec<[f32; 4]> {
    let mut out = vec![[0.0f32; 4]; CASCADES * GRID * GRID];
    let wind_angle = wind_dir[1].atan2(wind_dir[0]);
    for c in 0..CASCADES {
        let dk = std::f32::consts::TAU / TILE_METERS[c];
        if c == SWELL_CASCADE {
            let layer = swell_h0(seed, dk, wind_angle + SWELL_ANGLE_FROM_WIND_RADIANS);
            out[c * GRID * GRID..(c + 1) * GRID * GRID].copy_from_slice(&layer);
            continue;
        }
        let mut table = vec![[0.0f32; 2]; GRID * GRID];
        for y in 0..GRID {
            for x in 0..GRID {
                let kx = (x as f32 - GRID as f32 / 2.0) * dk;
                let kz = (y as f32 - GRID as f32 / 2.0) * dk;
                let k = (kx * kx + kz * kz).sqrt();
                if k < BAND_EDGES[c].max(1e-4) || k >= BAND_EDGES[c + 1] {
                    continue;
                }
                let omega = (GRAVITY * k).sqrt();
                let d_omega_dk = 0.5 * (GRAVITY / k).sqrt();
                let theta = kz.atan2(kx) - wind_angle;
                // Waves travelling against the wind are heavily damped.
                let directional = spread(theta) + 0.05 * spread(theta + std::f32::consts::PI);
                let power = 2.0 * jonswap(omega, wind_speed, fetch_meters) * directional
                    * d_omega_dk / k * dk * dk;
                let (gr, gi) = gaussian_pair(seed ^ hash((c * GRID * GRID + y * GRID + x) as u32));
                let a = (0.5 * power).sqrt();
                table[y * GRID + x] = [gr * a, gi * a];
            }
        }
        for y in 0..GRID {
            for x in 0..GRID {
                let h = table[y * GRID + x];
                let m = if x >= 1 && y >= 1 {
                    table[(GRID - y) * GRID + (GRID - x)]
                } else {
                    [0.0, 0.0]
                };
                out[c * GRID * GRID + y * GRID + x] = [h[0], h[1], m[0], m[1]];
            }
        }
    }
    out
}

/// Swell h0 layer, normalised to 1m significant wave height (Hs = 4 sigma).
fn swell_h0(seed: u32, dk: f32, swell_angle: f32) -> Vec<[f32; 4]> {
    let k_peak = std::f32::consts::TAU / swell_peak_wavelength_meters();
    let omega_peak = (GRAVITY * k_peak).sqrt();
    let mut table = vec![[0.0f32; 2]; GRID * GRID];
    for y in 0..GRID {
        for x in 0..GRID {
            let kx = (x as f32 - GRID as f32 / 2.0) * dk;
            let kz = (y as f32 - GRID as f32 / 2.0) * dk;
            let k = (kx * kx + kz * kz).sqrt();
            if k < 1e-6 {
                continue;
            }
            let omega = (GRAVITY * k).sqrt();
            let offset = (omega - omega_peak) / (SWELL_FREQUENCY_SPREAD * omega_peak);
            if offset.abs() > 3.5 {
                continue;
            }
            let cosine = (kz.atan2(kx) - swell_angle).cos();
            if cosine <= 0.0 {
                continue;
            }
            let power = (-0.5 * offset * offset).exp() * cosine.powi(SWELL_SPREAD_POWER)
                * 0.5 * (GRAVITY / k).sqrt() / k * dk * dk;
            let (gr, gi) = gaussian_pair(seed ^ 0x5eed_5e11 ^ hash((y * GRID + x) as u32));
            let a = (0.5 * power).sqrt();
            table[y * GRID + x] = [gr * a, gi * a];
        }
    }
    let mut layer = vec![[0.0f32; 4]; GRID * GRID];
    let mut variance = 0.0f64;
    for y in 0..GRID {
        for x in 0..GRID {
            let h = table[y * GRID + x];
            let m = if x >= 1 && y >= 1 { table[(GRID - y) * GRID + (GRID - x)] } else { [0.0, 0.0] };
            layer[y * GRID + x] = [h[0], h[1], m[0], m[1]];
            // Parseval on the unnormalised inverse transform: the spatial
            // variance at t = 0 is the sum of |h0(k) + conj(h0(-k))|^2.
            let (re, im) = ((h[0] + m[0]) as f64, (h[1] - m[1]) as f64);
            variance += re * re + im * im;
        }
    }
    let scale = if variance > 0.0 { (0.25 / variance.sqrt()) as f32 } else { 0.0 };
    for texel in &mut layer {
        for value in texel.iter_mut() {
            *value *= scale;
        }
    }
    layer
}

/// Wind speed (m/s) for the FFT sea; shared by the GPU spectrum and the CPU model.
/// Below this there is no wind sea at all (a glassy swell): the JONSWAP peak
/// divides by the wind speed, and at zero every wave came out NaN.
pub const MIN_WIND_SEA_METERS_PER_SECOND: f32 = 0.5;

pub fn wind_speed_from_environment() -> f32 {
    std::env::var("PLANET_OCEAN_FFT_WIND")
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map_or(14.0, |value| value.clamp(0.0, 40.0))
}

/// Wind direction of the FFT spectrum in the tangent-plane (u, v) axes.
pub const WIND_DIRECTION: [f32; 2] = [1.0, 0.3];

pub fn default_h0() -> Vec<[f32; 4]> {
    generate_h0(1, wind_speed_from_environment(), WIND_DIRECTION, 80_000.0)
}

static ANCHOR: std::sync::OnceLock<([f64; 3], [f64; 3])> = std::sync::OnceLock::new();

/// Tangent-plane axes shared by the shader and the CPU surface model. Fixed at
/// the first camera direction seen so the pattern never slides afterwards.
pub fn anchor_axes(camera_direction: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    *ANCHOR.get_or_init(|| {
        let d = camera_direction;
        let helper = if d[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
        };
        let c = cross(helper, d);
        let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        let u = [c[0] / l, c[1] / l, c[2] / l];
        (u, cross(d, u))
    })
}

/// CPU mirror of the geometry cascades: wind sea (cascade 0) and swell. The
/// same spectra the GPU transforms are inverse-FFT'd on the CPU into height,
/// vertical velocity and horizontal displacement grids that are bilinearly
/// interpolated exactly like the GPU sampler. Finer cascades only shade.
///
/// Grids are cached on a fixed time lattice (`LATTICE_SECONDS`) and advanced
/// by velocity to the query time, so any number of callers asking about any
/// times (camera, ship substeps, every bird's look-ahead) each cost at most
/// one transform per lattice step. The previous per-query refresh window made
/// the transform count grow with query count and time speed.
pub struct CpuSurface {
    shared: std::sync::Arc<SurfaceShared>,
    /// `band_wavenumbers` of the CPU cascades (wind, mid, swell).
    wavenumbers: [f64; 3],
}

struct SurfaceShared {
    cascades: Vec<CpuCascade>,
    /// Highest lattice key any caller has asked for; the worker builds the
    /// steps just beyond it so callers normally find them ready.
    frontier: std::sync::atomic::AtomicI64,
    wake: (std::sync::Mutex<bool>, std::sync::Condvar),
}

const LATTICE_SECONDS: f64 = 0.1;
/// Birds look ahead 3s; this keeps every lattice step they revisit.
const LATTICE_SLOTS: usize = 40;
const INVERSE_ITERATIONS: usize = 6;
/// Denominator floor for the frozen-scale normal/inverse approximation.
/// Displacement is limited by CHOP_STEEPNESS_BUDGET, not a pointwise Jacobian.
pub const MIN_JACOBIAN: f64 = 0.1;
/// Phase-envelope compression budget; includes headroom for envelope gradients.
/// Chosen against drawn GPU triangles, not the frozen-scale determinant.
pub const CHOP_STEEPNESS_BUDGET: f64 = 0.85;
/// Lattice steps the background worker builds ahead of the frontier.
const PREFETCH_STEPS: i64 = 3;

/// Horizontal (choppy) displacement strength; 1.0 is the Tessendorf field the
/// fold Jacobian is computed from, 0 disables it. `PLANET_OCEAN_FFT_CHOP`.
pub fn choppiness() -> f32 {
    static VALUE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("PLANET_OCEAN_FFT_CHOP")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .map_or(1.0, |v| v.clamp(0.0, 2.0))
    })
}

/// Significant wave height (metres) of the swell cascade. Swell is generated
/// by distant storms, so it is present in calm local weather; the local storm
/// raises it by up to 80%, at most 8m. Base from `PLANET_OCEAN_FFT_SWELL` (default 16, max 30).
pub fn swell_height_meters(storm_intensity: f32) -> f32 {
    stormed_swell_height(swell_base_height_meters(), storm_intensity)
}

/// The most a local storm adds to the swell, however big the swell is: a
/// swell from distant weather does not grow by 80% because of local weather
/// (30m would have become 54m, past any real sea and at breaking steepness).
const SWELL_STORM_BOOST_MAX_METERS: f32 = 8.0;
const DEFAULT_SWELL_HEIGHT_METERS: f32 = 16.0;

/// Swell height with the local storm's boost: up to 80% more, capped at
/// `SWELL_STORM_BOOST_MAX_METERS` (the default 16m swell reaches 24m).
fn stormed_swell_height(base: f32, storm_intensity: f32) -> f32 {
    let t = ((storm_intensity - 0.15) / 0.70).clamp(0.0, 1.0);
    let blend = t * t * (3.0 - 2.0 * t);
    base + (0.8 * base).min(SWELL_STORM_BOOST_MAX_METERS) * blend
}

/// `PLANET_OCEAN_FFT_SWELL` (default 16m, up to 30m): the swell's
/// significant height before the local storm raises it.
pub fn swell_base_height_meters() -> f32 {
    static BASE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *BASE.get_or_init(|| {
        std::env::var("PLANET_OCEAN_FFT_SWELL")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(DEFAULT_SWELL_HEIGHT_METERS, |v| v.clamp(0.0, 30.0))
    })
}

/// `PLANET_OCEAN_SWIRL=<seed>`: swirling dark sea colours from that seed
/// (shared_planet.wgsl `ocean_swirl_albedo`); unset, the ordinary water colour.
pub fn swirl_seed() -> Option<u32> {
    static SEED: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *SEED.get_or_init(|| {
        std::env::var("PLANET_OCEAN_SWIRL")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
    })
}

/// Strength of the second-order (Stokes) crest term, `PLANET_OCEAN_FFT_PEAKS`
/// (default 1, 0-3). 1 is second-order Stokes for a single wave: crests rise
/// and troughs flatten by k a^2 / 2. Where crests of one band cross it adds
/// more than that, which is what piles colliding crests into higher peaks.
pub fn second_order_strength() -> f32 {
    static VALUE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("PLANET_OCEAN_FFT_PEAKS")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .map_or(1.0, |v| v.clamp(0.0, 3.0))
    })
}

/// Mean wavenumber of each cascade's band, sum |k| |h~|^2 / sum |h~|^2 (0 for
/// an empty band). The second-order term of a band is taken at this one
/// wavenumber: the narrow-band (Tayfun) form of Stokes' correction.
pub fn band_wavenumbers(h0: &[[f32; 4]]) -> [f64; CASCADES] {
    let mut out = [0.0; CASCADES];
    for (c, mean) in out.iter_mut().enumerate() {
        let dk = std::f64::consts::TAU / tile_meters(c) as f64;
        let (mut weighted, mut total) = (0.0, 0.0);
        for y in 0..GRID {
            for x in 0..GRID {
                let t = h0[c * GRID * GRID + y * GRID + x];
                let (re, im) = ((t[0] + t[2]) as f64, (t[1] - t[3]) as f64);
                let n = x as f64 - GRID as f64 / 2.0;
                let m = y as f64 - GRID as f64 / 2.0;
                let power = re * re + im * im;
                weighted += dk * (n * n + m * m).sqrt() * power;
                total += power;
            }
        }
        *mean = if total > 0.0 { weighted / total } else { 0.0 };
    }
    out
}

/// Second-order (Stokes) height, its (u, v) gradient and its rate of change,
/// band by band: each band `(k, field)` adds (k/2)(h^2 - |D|^2) at its own
/// mean wavenumber k.
///
/// For one wave, h = a cos, D = a sin, this is Stokes' (k a^2 / 2) cos 2theta,
/// and crossing crests of a band (large h, small D) pile higher. It averages to
/// zero, since a linear field's h and D carry equal power, so sea level stays
/// put. It is *not* taken across bands: the old h(total) * div D(total) form
/// multiplied every short wave by (1 + k_short * swell height), so on a big
/// swell crest the chop grew into tall spikes and in the trough turned upside
/// down. A short wave riding a swell is carried by it, not amplified by it.
///
/// The rate uses D.dD/dt ~ -h dh/dt, exact for one wave, since the CPU grids
/// carry no displacement velocity.
fn second_order(bands: &[(f64, FieldSample)]) -> (f64, [f64; 2], f64) {
    let strength = second_order_strength() as f64;
    let (mut height, mut gradient, mut rate) = (0.0, [0.0; 2], 0.0);
    for (k, f) in bands {
        let [du, dv] = f.displacement;
        let j = f.jacobian;
        height += 0.5 * k * (f.height * f.height - du * du - dv * dv);
        gradient[0] += k * (f.height * f.slope[0] - du * j[0] - dv * j[3]);
        gradient[1] += k * (f.height * f.slope[1] - du * j[2] - dv * j[1]);
        rate += 2.0 * k * f.height * f.velocity;
    }
    (strength * height, gradient.map(|g| strength * g), strength * rate)
}

struct CpuCascade {
    tile_meters: f64,
    modes: Vec<Mode>,
    occupied_rows: Vec<bool>,
    cache: std::sync::Mutex<SlotCache>,
}

struct Slot {
    key: i64,
    last_used: u64,
    data: std::sync::Arc<SlotData>,
}

struct SlotData {
    height: Vec<f32>,
    velocity: Vec<f32>,
    dx: Vec<f32>,
    dz: Vec<f32>,
}

struct SlotCache {
    slots: Vec<Slot>,
    clock: u64,
}

struct Mode {
    x: usize,
    y: usize,
    h0: [f64; 2],
    h0m: [f64; 2],
    omega: f64,
    /// Unit wave direction (kx/k, kz/k).
    unit: [f64; 2],
}

/// Height, tangent-plane slope (du, dv) in metres per metre, vertical velocity.
#[derive(Clone, Copy, Debug)]
pub struct CpuSample {
    pub height: f64,
    pub slope_uv: [f64; 2],
    pub velocity: f64,
    /// Determinant of the drawn surface's horizontal Jacobian: 1 on flat
    /// water, falling toward `MIN_JACOBIAN` as a crest pinches and breaks.
    pub fold: f64,
    pub axis_u: [f64; 3],
    pub axis_v: [f64; 3],
}

/// One cascade's field and its forward differences at a tangent-plane point.
#[derive(Clone, Copy, Default)]
struct FieldSample {
    height: f64,
    slope: [f64; 2],
    velocity: f64,
    displacement: [f64; 2],
    /// dDu/du, dDv/dv, dDu/dv, dDv/du, as the shader orders them.
    jacobian: [f64; 4],
}

impl FieldSample {
    fn add_scaled(&mut self, other: &FieldSample, scale: f64) {
        self.height += other.height * scale;
        self.velocity += other.velocity * scale;
        for i in 0..2 {
            self.slope[i] += other.slope[i] * scale;
            self.displacement[i] += other.displacement[i] * scale;
        }
        for i in 0..4 {
            self.jacobian[i] += other.jacobian[i] * scale;
        }
    }
}

fn cmul(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}

fn twiddles() -> &'static [[f64; 2]] {
    static TABLE: std::sync::OnceLock<Vec<[f64; 2]>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        (0..GRID / 2)
            .map(|j| {
                let a = std::f64::consts::TAU * j as f64 / GRID as f64;
                [a.cos(), a.sin()]
            })
            .collect()
    })
}

/// In-place unnormalised inverse FFT (e^{+i}) of one 256-point line.
fn fft_line(line: &mut [[f64; 2]]) {
    let twiddle = twiddles();
    for i in 0..GRID {
        let j = (i as u32).reverse_bits() as usize >> (32 - 8);
        if j > i {
            line.swap(i, j);
        }
    }
    let mut half = 1;
    while half < GRID {
        let stride = GRID / (half * 2);
        for start in (0..GRID).step_by(half * 2) {
            for j in 0..half {
                let w = twiddle[j * stride];
                let t = cmul(w, line[start + j + half]);
                let u = line[start + j];
                line[start + j] = [u[0] + t[0], u[1] + t[1]];
                line[start + j + half] = [u[0] - t[0], u[1] - t[1]];
            }
        }
        half *= 2;
    }
}

/// 2D inverse FFT of a centred spectrum, returning [x][y]-transposed output
/// (index x * GRID + y) with the centring sign already applied.
fn inverse_fft_2d(mut grid: Vec<[f64; 2]>, occupied_rows: &[bool]) -> Vec<[f64; 2]> {
    for (y, row) in grid.chunks_mut(GRID).enumerate() {
        if occupied_rows[y] {
            fft_line(row);
        }
    }
    let mut transposed = vec![[0.0f64; 2]; GRID * GRID];
    for y in 0..GRID {
        for x in 0..GRID {
            transposed[x * GRID + y] = grid[y * GRID + x];
        }
    }
    for (x, column) in transposed.chunks_mut(GRID).enumerate() {
        fft_line(column);
        for (y, value) in column.iter_mut().enumerate() {
            if (x + y) & 1 == 1 {
                *value = [-value[0], -value[1]];
            }
        }
    }
    transposed
}

impl CpuCascade {
    fn new(h0: &[[f32; 4]], cascade: usize) -> Self {
        let half = GRID as i32 / 2;
        let tile_meters = tile_meters(cascade) as f64;
        let layer = &h0[cascade * GRID * GRID..(cascade + 1) * GRID * GRID];
        let mut modes = Vec::new();
        let mut occupied_rows = vec![false; GRID];
        for y in 0..GRID {
            for x in 0..GRID {
                let t = layer[y * GRID + x];
                if t == [0.0; 4] {
                    continue;
                }
                let n = (x as i32 - half) as f64;
                let m = (y as i32 - half) as f64;
                let length = (n * n + m * m).sqrt();
                let k = length * std::f64::consts::TAU / tile_meters;
                occupied_rows[y] = true;
                modes.push(Mode {
                    x,
                    y,
                    h0: [t[0] as f64, t[1] as f64],
                    h0m: [t[2] as f64, t[3] as f64],
                    omega: (GRAVITY as f64 * k).sqrt(),
                    unit: if length > 0.0 { [n / length, m / length] } else { [0.0, 0.0] },
                });
            }
        }
        Self {
            tile_meters,
            modes,
            occupied_rows,
            cache: std::sync::Mutex::new(SlotCache { slots: Vec::new(), clock: 0 }),
        }
    }

    /// Height, velocity and displacement grids at `time`, all stored [y][x].
    fn transform(&self, time: f64) -> [Vec<f32>; 4] {
        // Two packed spectra: h + i v and Dx + i Dz; every field is Hermitian,
        // so each complex inverse FFT returns two real grids.
        let mut vertical = vec![[0.0f64; 2]; GRID * GRID];
        let mut horizontal = vec![[0.0f64; 2]; GRID * GRID];
        for mode in &self.modes {
            let (s, c) = (mode.omega * time).sin_cos();
            let plus = cmul(mode.h0, [c, s]);
            let minus = cmul([mode.h0m[0], -mode.h0m[1]], [c, -s]);
            let h = [plus[0] + minus[0], plus[1] + minus[1]];
            // d/dt: i*omega*plus - i*omega*minus.
            let v = [-mode.omega * (plus[1] - minus[1]), mode.omega * (plus[0] - minus[0])];
            // D = -i (k/|k|) h, as the GPU evolve pass builds it.
            let dx = [mode.unit[0] * h[1], -mode.unit[0] * h[0]];
            let dz = [mode.unit[1] * h[1], -mode.unit[1] * h[0]];
            let cell = mode.y * GRID + mode.x;
            vertical[cell] = [h[0] - v[1], h[1] + v[0]];
            horizontal[cell] = [dx[0] - dz[1], dx[1] + dz[0]];
        }
        let rows = &self.occupied_rows;
        let (vertical, horizontal) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| inverse_fft_2d(horizontal, rows));
            let vertical = inverse_fft_2d(vertical, rows);
            (vertical, worker.join().expect("ocean CPU FFT worker"))
        });
        let mut out = [
            vec![0.0f32; GRID * GRID],
            vec![0.0f32; GRID * GRID],
            vec![0.0f32; GRID * GRID],
            vec![0.0f32; GRID * GRID],
        ];
        for x in 0..GRID {
            for y in 0..GRID {
                let (a, b) = (vertical[x * GRID + y], horizontal[x * GRID + y]);
                let cell = y * GRID + x;
                out[0][cell] = a[0] as f32;
                out[1][cell] = a[1] as f32;
                out[2][cell] = b[0] as f32;
                out[3][cell] = b[1] as f32;
            }
        }
        out
    }

    fn cached(&self, key: i64) -> Option<std::sync::Arc<SlotData>> {
        let mut cache = self.cache.lock().unwrap();
        cache.clock += 1;
        let clock = cache.clock;
        let slot = cache.slots.iter_mut().find(|slot| slot.key == key)?;
        slot.last_used = clock;
        Some(slot.data.clone())
    }

    fn insert(&self, key: i64, data: std::sync::Arc<SlotData>) -> std::sync::Arc<SlotData> {
        let mut cache = self.cache.lock().unwrap();
        cache.clock += 1;
        let clock = cache.clock;
        if let Some(slot) = cache.slots.iter_mut().find(|slot| slot.key == key) {
            slot.last_used = clock;
            return slot.data.clone();
        }
        let slot = Slot { key, last_used: clock, data: data.clone() };
        if cache.slots.len() < LATTICE_SLOTS {
            cache.slots.push(slot);
        } else {
            let oldest = (0..cache.slots.len())
                .min_by_key(|&i| cache.slots[i].last_used)
                .expect("slots");
            cache.slots[oldest] = slot;
        }
        data
    }

    fn build(&self, key: i64) -> std::sync::Arc<SlotData> {
        let [height, velocity, dx, dz] = self.transform(key as f64 * LATTICE_SECONDS);
        std::sync::Arc::new(SlotData { height, velocity, dx, dz })
    }

    /// The lattice slot for `key`, built on this thread if the worker has not
    /// got to it. The transform runs outside the cache lock.
    fn slot(&self, key: i64) -> std::sync::Arc<SlotData> {
        match self.cached(key) {
            Some(data) => data,
            None => self.insert(key, self.build(key)),
        }
    }
}

/// Sample of a slot at tangent-plane metres (u, v) exactly as the shader
/// takes it (`ocean_fft_cascade`): four bilinear samples half a texel either
/// side, the value their average and the derivatives their differences, so
/// slopes are continuous instead of constant over each texel.
fn sample_slot(slot: &SlotData, tile_meters: f64, position: [f64; 2], delta_seconds: f64) -> FieldSample {
    const MASK: i64 = GRID as i64 - 1;
    let step = tile_meters / GRID as f64;
    let half = 0.5 * step;
    // Per tap: the four texel indices and bilinear weights, computed once and
    // shared by every field (GRID is a power of two, so wrapping is a mask).
    let tap = |u: f64, v: f64| {
        let tx = (u / tile_meters).rem_euclid(1.0) * GRID as f64 - 0.5;
        let ty = (v / tile_meters).rem_euclid(1.0) * GRID as f64 - 0.5;
        let (fi, fj) = (tx.floor(), ty.floor());
        let (fx, fy) = (tx - fi, ty - fj);
        let (i0, j0) = (fi as i64, fj as i64);
        let (i1, j1) = ((i0 + 1) & MASK, (j0 + 1) & MASK);
        let (i0, j0) = (i0 & MASK, j0 & MASK);
        let row = |j: i64| j as usize * GRID;
        (
            [
                row(j0) + i0 as usize,
                row(j0) + i1 as usize,
                row(j1) + i0 as usize,
                row(j1) + i1 as usize,
            ],
            [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy],
        )
    };
    let taps = [
        tap(position[0] + half, position[1]),
        tap(position[0] - half, position[1]),
        tap(position[0], position[1] + half),
        tap(position[0], position[1] - half),
    ];
    let read = |field: &[f32]| {
        taps.map(|(index, weight)| {
            field[index[0]] as f64 * weight[0]
                + field[index[1]] as f64 * weight[1]
                + field[index[2]] as f64 * weight[2]
                + field[index[3]] as f64 * weight[3]
        })
    };
    let velocity = read(&slot.velocity);
    let raw = read(&slot.height);
    let height: [f64; 4] = std::array::from_fn(|i| raw[i] + delta_seconds * velocity[i]);
    let dx = read(&slot.dx);
    let dz = read(&slot.dz);
    let mean = |v: [f64; 4]| 0.25 * (v[0] + v[1] + v[2] + v[3]);
    FieldSample {
        height: mean(height),
        slope: [(height[0] - height[1]) / step, (height[2] - height[3]) / step],
        velocity: mean(velocity),
        displacement: [mean(dx), mean(dz)],
        jacobian: [
            (dx[0] - dx[1]) / step,
            (dz[2] - dz[3]) / step,
            (dx[2] - dx[3]) / step,
            (dz[0] - dz[1]) / step,
        ],
    }
}

/// Builds the lattice steps just past the highest one requested, so a clock
/// moving forward finds its grids ready. Exits when the surface is dropped.
fn prefetch_worker(shared: std::sync::Weak<SurfaceShared>) {
    loop {
        let Some(surface) = shared.upgrade() else { return };
        let frontier = surface.frontier.load(std::sync::atomic::Ordering::Relaxed);
        if frontier != i64::MIN {
            for key in frontier + 1..=frontier + PREFETCH_STEPS {
                for cascade in &surface.cascades {
                    if cascade.cached(key).is_none() {
                        cascade.insert(key, cascade.build(key));
                    }
                }
            }
        }
        let (flag, condvar) = &surface.wake;
        let mut woken = flag.lock().unwrap();
        if !*woken {
            woken = condvar
                .wait_timeout(woken, std::time::Duration::from_millis(50))
                .unwrap()
                .0;
        }
        *woken = false;
    }
}

/// Horizontal displacement and its frozen-scale Jacobian. Allocate a
/// phase-insensitive steepness envelope from swell to broad to mid, so the
/// short waves cannot jerk the entire swell sideways by changing its limiter.
/// For one wave, sqrt(|J|^2 + |grad(h)|^2) = k a throughout its phase. The
/// envelope still varies across wave packets: test the drawn mesh, not just
/// this Jacobian (which, like the old inverse, neglects scale gradients).
/// Mirrored by ocean_fft_chop_envelope / ocean_fft_chop_scales in WGSL.
fn limited_chop(bands: &[FieldSample; 3], chop: f64, budget: f64) -> FieldSample {
    let mut remaining = budget;
    let mut result = FieldSample::default();
    for i in [2, 0, 1] {
        let band = &bands[i];
        let j2 = band.jacobian.iter().map(|x| x * x).sum::<f64>();
        let slope2 = band.slope.iter().map(|x| x * x).sum::<f64>();
        let envelope = chop * (j2 + slope2).sqrt();
        let scale = (remaining / envelope.max(1.0e-6)).min(1.0);
        remaining = (remaining - envelope * scale).max(0.0);
        for axis in 0..2 {
            result.displacement[axis] += band.displacement[axis] * (chop * scale);
        }
        for axis in 0..4 {
            result.jacobian[axis] += band.jacobian[axis] * (chop * scale);
        }
    }
    result
}

// The old pointwise limiter remains only as a regression reference: its
// per-point determinant is NOT the determinant of the drawn mesh.
#[cfg(test)]
/// Largest scale f in [0, 1] on a choppy offset with Jacobian `j` (dDu/du,
/// dDv/dv, dDu/dv, dDv/du of the drawn offset) keeping det(I + f J) at or above
/// `MIN_JACOBIAN`. Historical shader formula, retained for regression tests.
fn fold_scale(j: [f64; 4]) -> f64 {
    fold_scale_to(j, MIN_JACOBIAN)
}

/// `fold_scale` held at an arbitrary `floor` (for measuring alternatives).
#[cfg(test)]
fn fold_scale_to(j: [f64; 4], floor: f64) -> f64 {
    let trace = j[0] + j[1];
    let det = j[0] * j[1] - j[2] * j[3];
    let margin = 1.0 - floor;
    if 1.0 + trace + det >= floor {
        return 1.0;
    }
    if det.abs() < 1.0e-6 {
        return (-margin / trace).clamp(0.0, 1.0);
    }
    let root = (trace * trace - 4.0 * det * margin).max(0.0).sqrt();
    let mut f: f64 = 1.0;
    for candidate in [(-trace - root) / (2.0 * det), (-trace + root) / (2.0 * det)] {
        if candidate > 0.0 {
            f = f.min(candidate);
        }
    }
    f.clamp(0.0, 1.0)
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl CpuSurface {
    pub fn new(h0: &[[f32; 4]]) -> Self {
        let shared = std::sync::Arc::new(SurfaceShared {
            cascades: vec![
                CpuCascade::new(h0, 0),
                CpuCascade::new(h0, 1),
                CpuCascade::new(h0, SWELL_CASCADE),
            ],
            frontier: std::sync::atomic::AtomicI64::new(i64::MIN),
            wake: (std::sync::Mutex::new(false), std::sync::Condvar::new()),
        });
        let weak = std::sync::Arc::downgrade(&shared);
        std::thread::Builder::new()
            .name("ocean-cpu-fft".into())
            .spawn(move || prefetch_worker(weak))
            .expect("spawn ocean CPU FFT worker");
        let k = band_wavenumbers(h0);
        Self { shared, wavenumbers: [k[0], k[1], k[SWELL_CASCADE]] }
    }

    fn cascades(&self) -> &[CpuCascade] {
        &self.shared.cascades
    }

    pub fn mode_count(&self) -> usize {
        self.cascades().iter().map(|c| c.modes.len()).sum()
    }

    /// The water surface drawn at planet direction `direction`: the label
    /// point x0 whose displaced position x0 - D(x0) lands here is found by
    /// Newton iteration, then height, slope (through the displacement
    /// Jacobian, as the shader shades it) and vertical velocity are read there.
    pub fn sample(
        &self,
        direction: [f64; 3],
        radius_meters: f64,
        time: f64,
        storm_intensity: f32,
    ) -> CpuSample {
        let (u, v) = anchor_axes(direction);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let target = [radius_meters * dot(u, direction), radius_meters * dot(v, direction)];
        let (sample, _) = self.sample_at(target, time, storm_intensity);
        CpuSample { axis_u: u, axis_v: v, ..sample }
    }

    /// Horizontal velocity (planet frame, m/s) of the water drawn at
    /// `direction`. The drawn surface is x0 - cD(x0, t), so the water there
    /// is its label x0 moving at -c dD/dt: the orbital motion that carries
    /// anything floating on it back and forth under each crest. dD/dt is
    /// taken across the 0.1s time lattice at the label.
    pub fn horizontal_velocity(
        &self,
        direction: [f64; 3],
        radius_meters: f64,
        time: f64,
        storm_intensity: f32,
    ) -> [f64; 3] {
        let (u, v) = anchor_axes(direction);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let target = [radius_meters * dot(u, direction), radius_meters * dot(v, direction)];
        let (_, label) = self.sample_at(target, time, storm_intensity);
        let key = (time / LATTICE_SECONDS).floor() as i64;
        let scales = [1.0, 1.0, swell_height_meters(storm_intensity) as f64];
        let field = |key: i64| {
            let bands = std::array::from_fn(|i| {
                let cascade = &self.cascades()[i];
                let mut band = FieldSample::default();
                band.add_scaled(
                    &sample_slot(&cascade.slot(key), cascade.tile_meters, label, 0.0),
                    scales[i],
                );
                band
            });
            limited_chop(&bands, choppiness() as f64, CHOP_STEEPNESS_BUDGET)
        };
        let (now, next) = (field(key), field(key + 1));
        let rate = [
            -(next.displacement[0] - now.displacement[0]) / LATTICE_SECONDS,
            -(next.displacement[1] - now.displacement[1]) / LATTICE_SECONDS,
        ];
        std::array::from_fn(|i| u[i] * rate[0] + v[i] * rate[1])
    }

    /// The rest position (label), in tangent-plane metres along `anchor_axes`,
    /// of the water drawn at `direction`: the coordinate the foam atlas is
    /// indexed by, so a float that holds this fixed rides with the foam.
    pub fn label_meters(
        &self,
        direction: [f64; 3],
        radius_meters: f64,
        time: f64,
        storm_intensity: f32,
    ) -> [f64; 2] {
        let (u, v) = anchor_axes(direction);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let target = [radius_meters * dot(u, direction), radius_meters * dot(v, direction)];
        self.sample_at(target, time, storm_intensity).1
    }

    /// A float held to one piece of the sea: the water whose rest position
    /// (label) is `label`, in the tangent-plane metres of `label_meters`.
    /// Returns that water's velocity (planet frame, m/s) and the vector from
    /// `direction` (at `radius_meters`) to where that water is drawn, so a
    /// float that follows the first and closes the second stays with the
    /// water -- and with the foam, which is indexed by the same label --
    /// instead of slowly sliding off it.
    pub fn follow_label(
        &self,
        label: [f64; 2],
        direction: [f64; 3],
        radius_meters: f64,
        time: f64,
        storm_intensity: f32,
    ) -> ([f64; 3], [f64; 3]) {
        let (u, v) = anchor_axes(direction);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let key = (time / LATTICE_SECONDS).round() as i64;
        let delta = time - key as f64 * LATTICE_SECONDS;
        let previous = self.shared.frontier.fetch_max(key, std::sync::atomic::Ordering::Relaxed);
        if key > previous {
            let (flag, condvar) = &self.shared.wake;
            *flag.lock().unwrap() = true;
            condvar.notify_one();
        }
        let scales = [1.0, 1.0, swell_height_meters(storm_intensity) as f64];
        let cascades = self.cascades();
        let slots_at = |key: i64| {
            cascades.iter().map(|cascade| cascade.slot(key)).collect::<Vec<_>>()
        };
        // The sea's horizontal displacement D at this label, from the grids of
        // one lattice step, `delta` seconds past it.
        let displacement = |slots: &[std::sync::Arc<SlotData>], delta: f64| {
            let bands: [FieldSample; 3] = std::array::from_fn(|i| {
                let mut band = FieldSample::default();
                band.add_scaled(
                    &sample_slot(&slots[i], cascades[i].tile_meters, label, delta),
                    scales[i],
                );
                band
            });
            limited_chop(&bands, choppiness() as f64, CHOP_STEEPNESS_BUDGET).displacement
        };
        let now_slots = slots_at(key);
        let now = displacement(&now_slots, delta);
        // Its rate across the lattice step, as `horizontal_velocity` takes it
        // (the grids only advance the height by velocity within a step, not D).
        let (here_key, next_key) = (displacement(&now_slots, 0.0), displacement(&slots_at(key + 1), 0.0));
        let rate = [
            -(next_key[0] - here_key[0]) / LATTICE_SECONDS,
            -(next_key[1] - here_key[1]) / LATTICE_SECONDS,
        ];
        // The drawn surface is label - D.
        let drawn = [label[0] - now[0], label[1] - now[1]];
        let here = [radius_meters * dot(u, direction), radius_meters * dot(v, direction)];
        let gap = [drawn[0] - here[0], drawn[1] - here[1]];
        (
            std::array::from_fn(|i| u[i] * rate[0] + v[i] * rate[1]),
            std::array::from_fn(|i| u[i] * gap[0] + v[i] * gap[1]),
        )
    }

    /// `sample` at tangent-plane metres `target`; also returns the label point.
    fn sample_at(&self, target: [f64; 2], time: f64, storm_intensity: f32) -> (CpuSample, [f64; 2]) {
        let key = (time / LATTICE_SECONDS).round() as i64;
        let delta = time - key as f64 * LATTICE_SECONDS;
        let swell_height = swell_height_meters(storm_intensity) as f64;
        // Wind (0) and mid (1) cascades at unit gain; the mid cascade's
        // distance fade is 1 within 600m of the camera, where CPU queries are.
        let scales = [1.0, 1.0, swell_height];
        let chop = choppiness() as f64;
        let cascades = self.cascades();
        let previous = self.shared.frontier.fetch_max(key, std::sync::atomic::Ordering::Relaxed);
        if key > previous {
            let (flag, condvar) = &self.shared.wake;
            *flag.lock().unwrap() = true;
            condvar.notify_one();
        }
        let slots: Vec<_> = cascades.iter().map(|cascade| cascade.slot(key)).collect();
        {
            {
                // The sum, and each cascade scaled (for the per-band term).
                let field = |position: [f64; 2]| {
                    let bands: [FieldSample; 3] = std::array::from_fn(|i| {
                        let mut band = FieldSample::default();
                        let sample = sample_slot(&slots[i], cascades[i].tile_meters, position, delta);
                        band.add_scaled(&sample, scales[i]);
                        band
                    });
                    let mut total = FieldSample::default();
                    for band in &bands {
                        total.add_scaled(band, 1.0);
                    }
                    (total, bands)
                };
                let limited =
                    |bands: &[FieldSample; 3]| limited_chop(bands, chop, CHOP_STEEPNESS_BUDGET);
                let mut label = target;
                let (mut sample, mut bands) = field(label);
                for _ in 0..INVERSE_ITERATIONS {
                    let horizontal = limited(&bands);
                    let residual = [
                        label[0] - horizontal.displacement[0] - target[0],
                        label[1] - horizontal.displacement[1] - target[1],
                    ];
                    let j = horizontal.jacobian;
                    // Frozen-scale inverse approximation, rows u and v.
                    let (a, b, cc, d) = (1.0 - j[0], -j[2], -j[3], 1.0 - j[1]);
                    // Within a millimetre: the sample in hand is the answer.
                    if residual[0].abs() + residual[1].abs() < 1.0e-3 {
                        break;
                    }
                    let det = (a * d - b * cc).max(0.2);
                    label[0] -= (d * residual[0] - b * residual[1]) / det;
                    label[1] -= (-cc * residual[0] + a * residual[1]) / det;
                    (sample, bands) = field(label);
                }
                let horizontal = limited(&bands);
                let j = horizontal.jacobian;
                let (a, b, cc, d) = (1.0 - j[0], -j[2], -j[3], 1.0 - j[1]);
                let det = (a * d - b * cc).max(MIN_JACOBIAN);
                // Second-order crest term, band by band, as the shader adds it.
                let k = self.wavenumbers;
                let (lift, lift_slope, lift_rate) =
                    second_order(&[(k[0], bands[0]), (k[1], bands[1]), (k[2], bands[2])]);
                let height = sample.height + lift;
                let [hu, hv] = [sample.slope[0] + lift_slope[0], sample.slope[1] + lift_slope[1]];
                (
                    CpuSample {
                        height,
                        slope_uv: [(d * hu - cc * hv) / det, (-b * hu + a * hv) / det],
                        velocity: sample.velocity + lift_rate,
                        fold: det,
                        axis_u: [0.0; 3],
                        axis_v: [0.0; 3],
                    },
                    label,
                )
            }
        }
    }

    /// Height texel of CPU cascade `index` (0 wind, 1 mid, 2 swell) at exact `time`.
    #[cfg(test)]
    fn texel(&self, index: usize, i: usize, j: usize, time: f64) -> f64 {
        self.cascades()[index].transform(time)[0][j * GRID + i] as f64
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    time: f32,
    pad: [f32; 3],
    tile: [[f32; 4]; CASCADES],
}

/// Uniform read by the ocean shaders: fixed tangent-plane axes plus, per
/// cascade, the camera's fractional tile coordinates (u, v), tile length and
/// the band's mean wavenumber (`band_wavenumbers`, for the second-order term).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewParams {
    pub axis_u: [f32; 4],
    pub axis_v: [f32; 4],
    pub cascade: [[f32; 4]; CASCADES],
    /// x: overall gain, y: choppiness, z: swell height (m), w: unused.
    pub gain: [f32; 4],
    /// x: second-order strength; y unused; z, w: the camera's absolute
    /// tangent-plane position (u, v metres), for patterns fixed to the sea
    /// rather than to the camera (`ocean_swirl_albedo`).
    pub second_order: [f32; 4],
    /// xyz: the camera's planet direction rounded to f32; w unused. Chunk
    /// edge vertices measure their offset from the camera through this one
    /// shared point, so neighbouring chunks sample the waves at bit-identical
    /// coordinates and land on bit-identical positions (see
    /// `ocean_edge_planet_offset` in planet.wgsl).
    pub edge_reference_direction: [f32; 4],
    /// xyz: that direction at planet radius minus the camera position,
    /// planet frame, metres, formed in f64; w unused.
    pub edge_reference_offset: [f32; 4],
    /// Storm gusts (`gust::Gust::uniform`): the camera's gust-field
    /// coordinate, gustiness, mean wind speed.
    pub gust: [f32; 4],
}

pub struct OceanFft {
    params: wgpu::Buffer,
    h0: wgpu::Buffer,
    pub field: wgpu::Texture,
    pub field_view: wgpu::TextureView,
    /// Height Laplacian of cascade c in channel c, one mip, for caustics.
    pub curvature_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub view_params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    mip_pipeline: wgpu::ComputePipeline,
    mip_groups: Vec<wgpu::BindGroup>,
    evolve: wgpu::ComputePipeline,
    rows: wgpu::ComputePipeline,
    cols: wgpu::ComputePipeline,
    assemble: wgpu::ComputePipeline,
    curvature: wgpu::ComputePipeline,
    wavenumbers: [f64; CASCADES],
}

impl OceanFft {
    pub fn new(device: &wgpu::Device, h0: &[[f32; 4]]) -> Self {
        assert_eq!(h0.len(), CASCADES * GRID * GRID);
        let wavenumbers = band_wavenumbers(h0);
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean fft layout"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(1, storage(true)),
                entry(2, storage(false)),
                entry(
                    3,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                ),
                entry(
                    4,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean fft pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean fft"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ocean_fft.wgsl").into()),
        });
        let pipeline = |entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let mut tile = [[0.0; 4]; CASCADES];
        for (c, t) in tile.iter_mut().enumerate() {
            t[0] = tile_meters(c);
        }
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean fft params"),
            contents: bytemuck::bytes_of(&Params { time: 0.0, pad: [0.0; 3], tile }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let h0 = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean fft h0"),
            contents: bytemuck::cast_slice(h0),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let cells = (GRID * GRID) as u64;
        let spec = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean fft spectrum"),
            size: FFT_ARRAYS as u64 * cells * 8,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let field = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ocean fft field"),
            size: wgpu::Extent3d {
                width: GRID as u32,
                height: GRID as u32,
                depth_or_array_layers: CASCADES as u32,
            },
            mip_level_count: MIP_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let field_view = field.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let curvature = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ocean fft curvature"),
            size: wgpu::Extent3d {
                width: GRID as u32,
                height: GRID as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let curvature_view = curvature.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ocean fft sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let view_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean fft view params"),
            size: size_of::<ViewParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mip_view = |texture: &wgpu::Texture, level: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean fft mip layout"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                ),
            ],
        });
        let mip_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean fft mips"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ocean_fft_mips.wgsl").into()),
        });
        let mip_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean fft mip pipeline layout"),
            bind_group_layouts: &[Some(&mip_layout)],
            immediate_size: 0,
        });
        let mip_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ocean fft downsample"),
            layout: Some(&mip_pipeline_layout),
            module: &mip_shader,
            entry_point: Some("downsample"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mip_groups = (1..MIP_LEVELS)
            .map(|level| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("ocean fft mip bind group"),
                    layout: &mip_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&mip_view(&field, level - 1)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&mip_view(&field, level)),
                        },
                    ],
                })
            })
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean fft bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: h0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: spec.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&mip_view(&field, 0)) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&curvature_view) },
            ],
        });
        Self {
            params,
            h0,
            field,
            field_view,
            curvature_view,
            sampler,
            view_params,
            bind_group,
            mip_pipeline,
            mip_groups,
            evolve: pipeline("evolve"),
            rows: pipeline("fft_rows"),
            cols: pipeline("fft_cols"),
            assemble: pipeline("assemble"),
            curvature: pipeline("curvature_bands"),
            wavenumbers,
        }
    }

    /// Anchors the tangent plane at the first camera direction, then keeps it
    /// fixed so the wave pattern never slides; only the camera's fractional
    /// tile coordinates change per frame.
    pub fn update_view(
        &self,
        queue: &wgpu::Queue,
        camera_position: [f64; 3],
        radius_meters: f64,
        gain: f32,
        storm_intensity: f32,
        gust: [f32; 4],
    ) {
        let camera_distance = (camera_position[0] * camera_position[0]
            + camera_position[1] * camera_position[1]
            + camera_position[2] * camera_position[2])
            .sqrt();
        let camera_direction = camera_position.map(|axis| axis / camera_distance);
        let reference_direction = camera_direction.map(|axis| axis as f32);
        let reference_offset: [f32; 4] = [
            (f64::from(reference_direction[0]) * radius_meters - camera_position[0]) as f32,
            (f64::from(reference_direction[1]) * radius_meters - camera_position[1]) as f32,
            (f64::from(reference_direction[2]) * radius_meters - camera_position[2]) as f32,
            0.0,
        ];
        let (u, v) = anchor_axes(camera_direction);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let (cu, cv) = (radius_meters * dot(u, camera_direction), radius_meters * dot(v, camera_direction));
        let mut cascade = [[0.0f32; 4]; CASCADES];
        for (c, entry) in cascade.iter_mut().enumerate() {
            let length = tile_meters(c) as f64;
            *entry = [
                (cu / length).rem_euclid(1.0) as f32,
                (cv / length).rem_euclid(1.0) as f32,
                tile_meters(c),
                self.wavenumbers[c] as f32,
            ];
        }
        let params = ViewParams {
            axis_u: [u[0] as f32, u[1] as f32, u[2] as f32, 0.0],
            axis_v: [v[0] as f32, v[1] as f32, v[2] as f32, 0.0],
            cascade,
            gain: [gain, choppiness(), swell_height_meters(storm_intensity), 0.0],
            second_order: [second_order_strength(), 0.0, cu as f32, cv as f32],
            edge_reference_direction: [
                reference_direction[0],
                reference_direction[1],
                reference_direction[2],
                0.0,
            ],
            edge_reference_offset: reference_offset,
            gust,
        };
        queue.write_buffer(&self.view_params, 0, bytemuck::bytes_of(&params));
    }

    pub fn set_time(&self, queue: &wgpu::Queue, time: f32) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&time));
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        self.encode_mask(encoder, 0b11111);
    }

    pub fn encode_mask(&self, encoder: &mut wgpu::CommandEncoder, mask: u32) {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_bind_group(0, &self.bind_group, &[]);
        let groups = (GRID / 8) as u32;
        if mask & 1 != 0 {
        pass.set_pipeline(&self.evolve);
        pass.dispatch_workgroups(groups, groups, CASCADES as u32);
        }
        if mask & 2 != 0 {
        pass.set_pipeline(&self.rows);
        pass.dispatch_workgroups(WORKGROUPS_PER_LINE_PASS, FFT_ARRAYS, 1);
        }
        if mask & 4 != 0 {
        pass.set_pipeline(&self.cols);
        pass.dispatch_workgroups(WORKGROUPS_PER_LINE_PASS, FFT_ARRAYS, 1);
        }
        if mask & 8 != 0 {
        pass.set_pipeline(&self.assemble);
        pass.dispatch_workgroups(groups, groups, CASCADES as u32);
        pass.set_pipeline(&self.curvature);
        pass.dispatch_workgroups(groups, groups, 1);
        }
        drop(pass);
        if mask & 16 != 0 {
            for (index, group) in self.mip_groups.iter().enumerate() {
                let size = (GRID as u32 >> (index + 1)).max(1);
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.mip_pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), CASCADES as u32);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    pub(crate) use super::tests::{device, read_field};
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("Vulkan adapter");
        eprintln!("ocean fft adapter: {}", adapter.get_info().name);
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("GPU device")
    }

    fn wait(device: &wgpu::Device) {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
    }

    fn f16_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = ((bits >> 10) & 0x1f) as i32;
        let frac = (bits & 0x3ff) as f32;
        match exp {
            0 => sign * frac * 2f32.powi(-24),
            31 => sign * f32::INFINITY,
            _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
        }
    }

    pub(crate) fn read_field(device: &wgpu::Device, queue: &wgpu::Queue, fft: &OceanFft) -> Vec<[f32; 4]> {
        let row = (GRID * 8) as u32;
        let bytes = row as u64 * GRID as u64 * CASCADES as u64;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fft readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            fft.field.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(GRID as u32),
                },
            },
            fft.field.size(),
        );
        queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        wait(device);
        rx.recv().unwrap().unwrap();
        let data = readback.slice(..).get_mapped_range();
        bytemuck::cast_slice::<u8, u16>(&data)
            .chunks(4)
            .map(|t| [f16_to_f32(t[0]), f16_to_f32(t[1]), f16_to_f32(t[2]), f16_to_f32(t[3])])
            .collect()
    }

    #[test]
    #[ignore = "requires a Vulkan GPU"]
    fn single_mode_produces_the_analytic_standing_wave() {
        let (n_idx, m_idx) = (3i32, 5i32);
        let mut h0 = vec![[0.0f32; 4]; CASCADES * GRID * GRID];
        let amp = 0.25f32;
        let (x, y) = ((GRID as i32 / 2 + n_idx) as usize, (GRID as i32 / 2 + m_idx) as usize);
        let (mx, my) = ((GRID as i32 / 2 - n_idx) as usize, (GRID as i32 / 2 - m_idx) as usize);
        // Cascade 0: both +k and -k carry amplitude `amp` (real).
        h0[y * GRID + x] = [amp, 0.0, amp, 0.0];
        h0[my * GRID + mx] = [amp, 0.0, amp, 0.0];
        let (device, queue) = device();
        let fft = OceanFft::new(&device, &h0);
        fft.set_time(&queue, 0.0);
        let mut encoder = device.create_command_encoder(&Default::default());
        fft.encode(&mut encoder);
        queue.submit(Some(encoder.finish()));
        let field = read_field(&device, &queue, &fft);
        let dk = std::f32::consts::TAU / tile_meters(0);
        let (kx, kz) = (n_idx as f32 * dk, m_idx as f32 * dk);
        let kl = (kx * kx + kz * kz).sqrt();
        let mut max_err = 0.0f32;
        for &(px, py) in &[(0usize, 0usize), (17, 40), (101, 7), (200, 250), (255, 255), (128, 129)] {
            let phase = std::f32::consts::TAU * (n_idx as f32 * px as f32 + m_idx as f32 * py as f32)
                / GRID as f32;
            // h = 2*(2A) cos ; Dx = 4A (kx/kl) sin ; Dz likewise ; S = -4A k sin.
            let expected = [
                4.0 * amp * phase.cos(),
                4.0 * amp * (kx / kl) * phase.sin(),
                4.0 * amp * (kz / kl) * phase.sin(),
                0.0,
            ];
            let got = field[py * GRID + px];
            for i in 0..3 {
                max_err = max_err.max((got[i] - expected[i]).abs());
            }
        }
        eprintln!("single-mode max error {max_err}");
        assert!(max_err < 2e-3, "max error {max_err}");
    }

    #[test]
    #[ignore = "requires a Vulkan GPU; benchmark"]
    fn full_field_cost_per_frame() {
        let h0 = generate_h0(1, 14.0, [1.0, 0.3], 80_000.0);
        let (device, queue) = device();
        let fft = OceanFft::new(&device, &h0);
        let run_mask = |mask: u32| {
            let start = std::time::Instant::now();
            for i in 0..100 {
                fft.set_time(&queue, i as f32 * 0.016);
                let mut encoder = device.create_command_encoder(&Default::default());
                fft.encode_mask(&mut encoder, mask);
                queue.submit(Some(encoder.finish()));
            }
            wait(&device);
            start.elapsed().as_secs_f64() * 10.0
        };
        for (n, m) in [("none", 0), ("evolve", 1), ("rows", 2), ("cols", 4), ("assemble", 8)] {
            run_mask(m);
            eprintln!("pass {n}: {:.3} ms", run_mask(m));
        }
        let run = |frames: u32| {
            let start = std::time::Instant::now();
            for i in 0..frames {
                fft.set_time(&queue, i as f32 * 0.016);
                let mut encoder = device.create_command_encoder(&Default::default());
                fft.encode(&mut encoder);
                queue.submit(Some(encoder.finish()));
            }
            wait(&device);
            start.elapsed().as_secs_f64() * 1000.0 / frames as f64
        };
        run(20);
        let samples: Vec<f64> = (0..5).map(|_| run(100)).collect();
        eprintln!("ocean FFT ({CASCADES} x 256^2, {FFT_ARRAYS} FFTs) ms/frame: {samples:.3?}");
        let field = read_field(&device, &queue, &fft);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for texel in &field[..GRID * GRID] {
            lo = lo.min(texel[0]);
            hi = hi.max(texel[0]);
        }
        eprintln!("cascade 0 height range {lo:.3}..{hi:.3} m");
        assert!(lo.is_finite() && hi.is_finite() && hi > lo);
    }

    #[test]
    #[ignore = "requires a Vulkan GPU"]
    fn cpu_model_matches_the_gpu_texture() {
        let h0 = default_h0();
        let cpu = CpuSurface::new(&h0);
        let (device, queue) = device();
        let fft = OceanFft::new(&device, &h0);
        let time = 37.25;
        fft.set_time(&queue, time as f32);
        let mut encoder = device.create_command_encoder(&Default::default());
        fft.encode(&mut encoder);
        queue.submit(Some(encoder.finish()));
        let field = read_field(&device, &queue, &fft);
        for (index, layer) in [(0usize, 0usize), (1, 1), (2, SWELL_CASCADE)] {
            let (mut max_err, mut max_h) = (0.0f64, 0.0f64);
            for &(i, j) in &[(0usize, 0usize), (5, 9), (100, 200), (255, 255), (128, 64), (17, 240), (77, 3)] {
                let gpu = field[layer * GRID * GRID + j * GRID + i][0] as f64;
                max_err = max_err.max((gpu - cpu.texel(index, i, j, time)).abs());
                max_h = max_h.max(gpu.abs());
            }
            eprintln!("layer {layer}: cpu vs gpu texel max error {max_err:.5} m (max height {max_h:.3})");
            assert!(max_err < 0.02, "layer {layer} max error {max_err}");
        }
    }

    #[test]
    fn swell_layer_is_normalised_to_one_metre_significant_height() {
        let cpu = CpuSurface::new(&default_h0());
        let grid = cpu.cascades()[2].transform(0.0);
        let n = grid[0].len() as f64;
        let mean = grid[0].iter().map(|&h| h as f64).sum::<f64>() / n;
        let sigma = (grid[0].iter().map(|&h| (h as f64 - mean).powi(2)).sum::<f64>() / n).sqrt();
        eprintln!("swell layer Hs {:.4} m, modes {}", 4.0 * sigma, cpu.cascades()[2].modes.len());
        assert!((4.0 * sigma - 1.0).abs() < 0.01, "Hs {}", 4.0 * sigma);
        assert!(swell_height_meters(0.0) > 0.0);
    }

    #[test]
    fn lattice_extrapolation_tracks_an_exact_transform() {
        let cpu = CpuSurface::new(&default_h0());
        let cascade = &cpu.cascades()[0];
        // Worst case: halfway between lattice points.
        let time = 12.0 + 0.5 * LATTICE_SECONDS - 1e-9;
        let key = (time / LATTICE_SECONDS).round() as i64;
        let delta = time - key as f64 * LATTICE_SECONDS;
        let [height, velocity, dx, dz] = cascade.transform(time);
        let exact = SlotData { height, velocity, dx, dz };
        let mut worst = 0.0f64;
        let step = cascade.tile_meters / GRID as f64;
        for &(i, j) in &[(3usize, 7usize), (90, 12), (200, 150), (255, 0)] {
            let position = [(i as f64 + 0.3) * step, (j as f64 + 0.7) * step];
            let held = sample_slot(&cascade.slot(key), cascade.tile_meters, position, delta);
            let fresh = sample_slot(&exact, cascade.tile_meters, position, 0.0);
            worst = worst.max((held.height - fresh.height).abs());
        }
        eprintln!("lattice extrapolation worst height error {worst:.5} m");
        assert!(worst < 0.005, "{worst}");
    }

    #[test]
    fn inverse_displacement_finds_the_label_that_lands_on_the_target() {
        let cpu = CpuSurface::new(&default_h0());
        let (time, storm) = (21.3, 0.0);
        let key = (time / LATTICE_SECONDS).round() as i64;
        let delta = time - key as f64 * LATTICE_SECONDS;
        let scale = swell_height_meters(storm) as f64;
        let chop = choppiness() as f64;
        for label in [[10.0, 20.0], [333.3, -71.0], [1234.5, 876.5], [-40.0, 512.25]] {
            // Forward-map a label exactly as the shader displaces a vertex.
            let at = |p: [f64; 2]| {
                let mut total = FieldSample::default();
                for (cascade, weight) in cpu.cascades().iter().zip([1.0, 1.0, scale]) {
                    total.add_scaled(&sample_slot(&cascade.slot(key), cascade.tile_meters, p, delta), weight);
                }
                total
            };
            let field = at(label);
            let forward_bands = std::array::from_fn(|i| {
                let cascade = &cpu.cascades()[i];
                let mut band = FieldSample::default();
                band.add_scaled(
                    &sample_slot(&cascade.slot(key), cascade.tile_meters, label, delta),
                    [1.0, 1.0, scale][i],
                );
                band
            });
            let horizontal = limited_chop(&forward_bands, chop, CHOP_STEEPNESS_BUDGET);
            let target = [
                label[0] - horizontal.displacement[0],
                label[1] - horizontal.displacement[1],
            ];
            let (sample, found) = cpu.sample_at(target, time, storm);
            let miss = ((found[0] - label[0]).powi(2) + (found[1] - label[1]).powi(2)).sqrt();
            eprintln!(
                "label {label:?}: displaced {:.2} m, recovered within {miss:.4} m, height {:.3} vs {:.3}",
                (horizontal.displacement[0].powi(2) + horizontal.displacement[1].powi(2)).sqrt(),
                sample.height,
                field.height
            );
            let bands: Vec<(f64, FieldSample)> = cpu
                .cascades()
                .iter()
                .zip([1.0, 1.0, scale])
                .zip(cpu.wavenumbers)
                .map(|((cascade, weight), k)| {
                    let mut band = FieldSample::default();
                    band.add_scaled(&sample_slot(&cascade.slot(key), cascade.tile_meters, label, delta), weight);
                    (k, band)
                })
                .collect();
            let expected = field.height + second_order(&bands).0;
            assert!(miss < 0.02, "{miss}");
            assert!((sample.height - expected).abs() < 0.005, "{} vs {expected}", sample.height);
        }
    }

    /// One wave along u, h = a cos(kx), D = a sin(kx) (the field's D = +k^ a sin),
    /// with its slope, velocity and displacement Jacobian.
    fn one_wave(k: f64, a: f64, x: f64) -> FieldSample {
        let omega = (GRAVITY as f64 * k).sqrt();
        FieldSample {
            height: a * (k * x).cos(),
            slope: [-k * a * (k * x).sin(), 0.0],
            velocity: a * omega * (k * x).sin(),
            displacement: [a * (k * x).sin(), 0.0],
            jacobian: [k * a * (k * x).cos(), 0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn second_order_term_is_stokes_for_one_wave_and_keeps_mean_level() {
        let strength = second_order_strength() as f64;
        let (k, a) = (0.05, 3.0);
        let mut mean = 0.0;
        let steps = 64;
        for i in 0..steps {
            let x = std::f64::consts::TAU / k * i as f64 / steps as f64;
            let (lift, gradient, _) = second_order(&[(k, one_wave(k, a, x))]);
            // (k a^2 / 2) cos(2kx) and its derivative.
            let stokes = strength * 0.5 * k * a * a * (2.0 * k * x).cos();
            let slope = -strength * k * k * a * a * (2.0 * k * x).sin();
            assert!((lift - stokes).abs() < 1e-9, "{lift} vs {stokes}");
            assert!((gradient[0] - slope).abs() < 1e-9, "{} vs {slope}", gradient[0]);
            mean += lift / steps as f64;
        }
        assert!(mean.abs() < 1e-9, "sea level moved {mean}");
        // The band wavenumber of a one-mode spectrum is that mode's.
        let mut h0 = vec![[0.0f32; 4]; CASCADES * GRID * GRID];
        let n = 8usize;
        h0[(GRID / 2) * GRID + GRID / 2 + n] = [0.5, 0.0, 0.5, 0.0];
        h0[(GRID / 2) * GRID + GRID / 2 - n] = [0.5, 0.0, 0.5, 0.0];
        let expected = std::f64::consts::TAU * n as f64 / tile_meters(0) as f64;
        assert!((band_wavenumbers(&h0)[0] - expected).abs() < 1e-9 * expected);
        assert_eq!(band_wavenumbers(&h0)[1], 0.0, "an empty band has no term");
    }

    #[test]
    fn a_swell_carries_the_chop_without_scaling_it() {
        // Ian: with any wind the sea went spiky. The old term, h(total) *
        // div D(total), multiplied a short wave by (1 + k_short * swell
        // height): on a 10m swell crest 8m chop grew several times over and in
        // the trough turned upside down. Per band, the chop's second-order
        // shape is the same on the crest as in the trough.
        let (k_swell, a_swell) = (std::f64::consts::TAU / 300.0, 10.0);
        let (k_chop, a_chop) = (std::f64::consts::TAU / 8.0, 0.4);
        let chop_range = |centre: f64| {
            let (mut low, mut high) = (f64::MAX, f64::MIN);
            for i in 0..=80 {
                let x = centre + 8.0 * (i as f64 / 80.0 - 0.5);
                let swell = one_wave(k_swell, a_swell, x);
                let both = second_order(&[(k_swell, swell), (k_chop, one_wave(k_chop, a_chop, x))]).0;
                let chop_part = both - second_order(&[(k_swell, swell)]).0;
                low = low.min(chop_part);
                high = high.max(chop_part);
            }
            high - low
        };
        let (crest, trough) = (chop_range(0.0), chop_range(150.0));
        let own = second_order_strength() as f64 * k_chop * a_chop * a_chop;
        eprintln!("chop second-order range: crest {crest:.4} m, trough {trough:.4} m, own Stokes {own:.4} m");
        assert!((crest - trough).abs() < 1e-3, "crest {crest} vs trough {trough}");
        assert!(crest <= own + 1e-3, "{crest} is more than the chop's own Stokes term {own}");
    }

    #[test]
    #[ignore = "benchmark: CPU cost of one water sample"]
    fn cpu_sample_cost() {
        let cpu = CpuSurface::new(&default_h0());
        let r = 4_000_000.0;
        let time = 30.0;
        cpu.sample([0.6, 0.8, 0.0], r, time, 1.0); // warm the slots
        let n = 20_000;
        let start = std::time::Instant::now();
        for i in 0..n {
            let a = i as f64 * 1.0e-6;
            std::hint::black_box(cpu.sample([0.6 + a, 0.8, a], r, time + (i % 3) as f64 * 0.01, 1.0));
        }
        eprintln!("CPU water sample: {:.2} us each", start.elapsed().as_secs_f64() * 1e6 / n as f64);
    }

    /// The drawn mesh, as the vertex shader places it: label points on a
    /// `side` x `side` grid `spacing` apart moved to x0 - cD, with the fold
    /// envelope budget held at `budget` (as a function of the label slope), plus
    /// their heights.
    fn drawn_mesh(
        cpu: &CpuSurface,
        time: f64,
        side: usize,
        spacing: f64,
        budget: &dyn Fn([f64; 2]) -> f64,
    ) -> Vec<[f64; 3]> {
        let chop = choppiness() as f64;
        let scale = swell_height_meters(0.22) as f64;
        let key = (time / LATTICE_SECONDS).round() as i64;
        let delta = time - key as f64 * LATTICE_SECONDS;
        (0..side * side)
            .map(|n| {
                let p = [(n % side) as f64 * spacing, (n / side) as f64 * spacing];
                let bands = std::array::from_fn(|i| {
                    let cascade = &cpu.cascades()[i];
                    let mut band = FieldSample::default();
                    band.add_scaled(
                        &sample_slot(&cascade.slot(key), cascade.tile_meters, p, delta),
                        [1.0, 1.0, scale][i],
                    );
                    band
                });
                let mut f = FieldSample::default();
                for band in &bands {
                    f.add_scaled(band, 1.0);
                }
                let horizontal = limited_chop(&bands, chop, budget(f.slope));
                [
                    p[0] - horizontal.displacement[0],
                    p[1] - horizontal.displacement[1],
                    f.height,
                ]
            })
            .collect()
    }

    /// Per cell of a drawn mesh: None if it turned inside out, otherwise the
    /// slope of the plane through its three corners.
    fn drawn_cells(mesh: &[[f64; 3]], side: usize) -> Vec<Option<f64>> {
        let mut cells = Vec::with_capacity((side - 1) * (side - 1));
        for y in 0..side - 1 {
            for x in 0..side - 1 {
                let a = mesh[y * side + x];
                let b = mesh[y * side + x + 1];
                let c = mesh[(y + 1) * side + x];
                let (du, dv) = ([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]);
                let det = du[0] * dv[1] - du[1] * dv[0];
                if det <= 0.0 {
                    cells.push(None);
                    continue;
                }
                let (eb, ec) = (b[2] - a[2], c[2] - a[2]);
                let su = (eb * dv[1] - ec * du[1]) / det;
                let sv = (du[0] * ec - dv[0] * eb) / det;
                cells.push(Some((su * su + sv * sv).sqrt()));
            }
        }
        cells
    }

    #[test]
    fn the_drawn_sea_almost_never_turns_inside_out() {
        // Measure the mesh itself, including changes in the limiter. The
        // former slope-dependent floor failed this default-chop census;
        // the subsequent fixed floor passed but still made shards at chop 2.
        // The new GPU regression covers that high-chop, filtered field too.
        let cpu = CpuSurface::new(&default_h0());
        let (mut cells, mut inverted) = (0usize, 0usize);
        for time in [12.3, 47.9, 96.1] {
            for cell in drawn_cells(
                &drawn_mesh(&cpu, time, 300, 0.5, &|_| CHOP_STEEPNESS_BUDGET),
                300,
            ) {
                cells += 1;
                inverted += usize::from(cell.is_none());
            }
        }
        let percent = 100.0 * inverted as f64 / cells as f64;
        eprintln!("{inverted} of {cells} drawn cells inside out ({percent:.4}%)");
        assert!(percent < 0.005, "{percent}% of the drawn sea turned inside out");
    }

    #[test]
    #[ignore = "instrument: drawn-mesh fold-overs and steepness at several envelope budgets"]
    fn drawn_fold_census() {
        let cpu = CpuSurface::new(&default_h0());
        for budget in [0.90, 0.85, 0.80, 0.70] {
            let (mut total, mut inverted) = (0usize, 0usize);
            let mut slopes = Vec::new();
            for step in 0..20 {
                let time = 5.0 + step as f64 * 7.3;
                for cell in drawn_cells(&drawn_mesh(&cpu, time, 400, 0.5, &|_| budget), 400) {
                    total += 1;
                    match cell {
                        None => inverted += 1,
                        Some(slope) => slopes.push(slope),
                    }
                }
            }
            slopes.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |p: f64| slopes[((slopes.len() - 1) as f64 * p) as usize].atan().to_degrees();
            let over = |degrees: f64| {
                let t = degrees.to_radians().tan();
                100.0 * slopes.iter().filter(|&&s| s > t).count() as f64 / slopes.len() as f64
            };
            eprintln!(
                "budget {budget:.2}: inverted {:.4}% | slope p50 {:.1} p99 {:.1} p99.99 {:.1} deg, over 45 {:.4}%, over 60 {:.5}%",
                100.0 * inverted as f64 / total as f64, q(0.5), q(0.99), q(0.9999), over(45.0), over(60.0)
            );
        }
    }

    #[test]
    fn legacy_fold_scale_bounds_the_frozen_scale_determinant() {
        let det = |j: [f64; 4], f: f64| (1.0 + f * j[0]) * (1.0 + f * j[1]) - f * f * j[2] * j[3];
        // Gentle, one-axis cusp, crossing crests compressing both ways, shear.
        for j in [[-0.3, -0.1, 0.0, 0.0], [-1.6, 0.2, 0.1, 0.1], [-1.2, -1.2, 0.3, 0.3], [-0.9, -0.9, -0.8, 0.8]] {
            let f = fold_scale(j);
            assert!((0.0..=1.0).contains(&f));
            assert!(det(j, f) >= MIN_JACOBIAN - 1.0e-9, "{j:?} f {f} det {}", det(j, f));
            if det(j, 1.0) >= MIN_JACOBIAN {
                assert_eq!(f, 1.0, "left alone when it does not fold");
            } else {
                assert!((det(j, f) - MIN_JACOBIAN).abs() < 1.0e-9, "held exactly at the floor");
            }
        }
    }

    #[test]
    fn chop_limiter_is_phase_independent_for_a_single_wave() {
        // A 20m amplitude, 200m long wave at chop 2 exceeds the budget.
        // Its limiter must not change between its tip and flanks: doing so
        // introduces the D * grad(scale) term that made the sideways blades.
        let (amplitude, k, chop) = (20.0, std::f64::consts::TAU / 200.0, 2.0);
        let scale = CHOP_STEEPNESS_BUDGET / (chop * k * amplitude);
        for step in 0..256 {
            let phase = std::f64::consts::TAU * step as f64 / 256.0;
            let swell = FieldSample {
                slope: [-k * amplitude * phase.sin(), 0.0],
                displacement: [amplitude * phase.sin(), 0.0],
                jacobian: [k * amplitude * phase.cos(), 0.0, 0.0, 0.0],
                ..Default::default()
            };
            let bands = [FieldSample::default(), FieldSample::default(), swell];
            let drawn = limited_chop(&bands, chop, CHOP_STEEPNESS_BUDGET);
            assert!((drawn.displacement[0] - chop * scale * swell.displacement[0]).abs() < 1.0e-12);
        }
    }

    #[test]
    fn the_shader_holds_crests_at_the_same_pinch() {
        let shader = include_str!("shared_planet.wgsl");
        assert!(shader.contains(&format!("const OCEAN_FFT_MIN_JACOBIAN: f32 = {MIN_JACOBIAN:?};")));
        let budget = format!("const OCEAN_FFT_CHOP_BUDGET: f32 = {CHOP_STEEPNESS_BUDGET:?};");
        assert!(shader.contains(&budget));
        assert!(include_str!("ocean_spray_update.wgsl").contains(&budget));
    }

    #[test]
    fn a_storm_adds_at_most_eight_metres_of_swell() {
        assert_eq!(DEFAULT_SWELL_HEIGHT_METERS, 16.0);
        assert_eq!(stormed_swell_height(DEFAULT_SWELL_HEIGHT_METERS, 0.0), 16.0);
        assert_eq!(stormed_swell_height(DEFAULT_SWELL_HEIGHT_METERS, 1.0), 24.0);
        assert_eq!(stormed_swell_height(8.0, 0.0), 8.0);
        assert!((stormed_swell_height(8.0, 1.0) - 14.4).abs() < 1e-4);
        assert!((stormed_swell_height(5.0, 1.0) - 9.0).abs() < 1e-4);
        assert!((stormed_swell_height(30.0, 1.0) - 38.0).abs() < 1e-4, "not 54m");
        assert!(stormed_swell_height(30.0, 0.5) < 38.0);
    }

    #[test]
    fn bigger_swell_is_longer_swell() {
        assert_eq!(swell_wavelength_for_height(8.0), 170.0, "default unchanged");
        assert_eq!(swell_wavelength_for_height(20.0), 240.0);
        assert_eq!(swell_wavelength_for_height(30.0), 360.0);
        // Several waves per swell tile even at the largest.
        assert!(TILE_METERS[SWELL_CASCADE] / swell_wavelength_for_height(30.0) > 5.0);
    }

    #[test]
    fn no_wind_leaves_a_finite_glassy_swell() {
        let h0 = generate_h0(1, 0.0, WIND_DIRECTION, 80_000.0);
        assert!(h0.iter().flatten().all(|value| value.is_finite()));
        let wind_layers = &h0[..WIND_CASCADES * GRID * GRID];
        assert!(wind_layers.iter().flatten().all(|&value| value == 0.0), "no wind sea");
        let swell = &h0[SWELL_CASCADE * GRID * GRID..];
        assert!(swell.iter().flatten().any(|&value| value != 0.0), "swell remains");
        let cpu = CpuSurface::new(&h0);
        let sample = cpu.sample([0.6, 0.8, 0.0], 4_000_000.0, 12.0, 0.0);
        assert!(sample.height.is_finite() && sample.velocity.is_finite() && sample.fold.is_finite());
        assert!(sample.slope_uv.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn a_clock_moving_forward_finds_its_grids_prefetched() {
        let cpu = CpuSurface::new(&default_h0());
        let time = 50.0;
        cpu.sample([0.6_f64, 0.8, 0.0], 4_000_000.0, time, 0.0);
        let key = (time / LATTICE_SECONDS).round() as i64;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        // The render thread never asked for these; the worker must build them.
        for ahead in 1..=PREFETCH_STEPS {
            for cascade in cpu.cascades() {
                while cascade.cached(key + ahead).is_none() {
                    assert!(std::time::Instant::now() < deadline, "step +{ahead} never prefetched");
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn interleaved_query_times_cost_one_transform_per_lattice_step() {
        let cpu = CpuSurface::new(&default_h0());
        let d = [0.6_f64, 0.8, 0.0];
        let r = 4_000_000.0;
        let start = std::time::Instant::now();
        for cascade in cpu.cascades() {
            std::hint::black_box(cascade.transform(5.0));
        }
        let one_step = start.elapsed().as_secs_f64();
        // A bird's look-ahead pattern: several times, stepped at 30Hz for 1s.
        let start = std::time::Instant::now();
        for step in 0..30 {
            let now = 10.0 + step as f64 / 30.0;
            for ahead in [0.0, 0.5, 1.0, 1.5, 2.0, 3.0] {
                std::hint::black_box(cpu.sample(d, r, now + ahead, 0.0));
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        // 1s of steps spans ~10 lattice steps per look-ahead offset, but the
        // offsets revisit each other's steps: at most ~40 distinct keys.
        eprintln!("transform (all CPU cascades) {:.2} ms; 180 look-ahead samples {:.1} ms", one_step * 1e3, elapsed * 1e3);
        assert!(elapsed < one_step * 45.0, "{elapsed} vs {one_step}");
    }
}

#[cfg(test)]
mod jacobian_study {
    use super::tests_support::*;
    use super::*;

    #[test]
    #[ignore = "requires a Vulkan GPU; prints the fold Jacobian distribution"]
    fn jacobian_distribution() {
        let h0 = default_h0();
        let (device, queue) = device();
        let fft = OceanFft::new(&device, &h0);
        fft.set_time(&queue, 37.25);
        let mut encoder = device.create_command_encoder(&Default::default());
        fft.encode(&mut encoder);
        queue.submit(Some(encoder.finish()));
        let field = read_field(&device, &queue, &fft);
        for c in 0..CASCADES {
            let at = |x: usize, y: usize| field[c * GRID * GRID + (y % GRID) * GRID + (x % GRID)];
            let step = tile_meters(c) / GRID as f32;
            let mut j = Vec::new();
            for y in 0..GRID {
                for x in 0..GRID {
                    let (l, r) = (at(x + GRID - 1, y), at(x + 1, y));
                    let (d, u) = (at(x, y + GRID - 1), at(x, y + 1));
                    let dxdx = (r[1] - l[1]) / (2.0 * step);
                    let dzdz = (u[2] - d[2]) / (2.0 * step);
                    let dxdz = (u[1] - d[1]) / (2.0 * step);
                    let dzdx = (r[2] - l[2]) / (2.0 * step);
                    j.push((1.0 - dxdx) * (1.0 - dzdz) - dxdz * dzdx);
                }
            }
            j.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p = |q: f32| j[((j.len() - 1) as f32 * q) as usize];
            let hs: Vec<f32> = (0..GRID * GRID).map(|i| field[c * GRID * GRID + i][0]).collect();
            let mean = hs.iter().sum::<f32>() / hs.len() as f32;
            let std = (hs.iter().map(|h| (h - mean) * (h - mean)).sum::<f32>() / hs.len() as f32).sqrt();
            eprintln!("cascade {c}: height std {std:.4} m");
            eprintln!(
                "cascade {c}: J min {:.3} p0.1 {:.3} p1 {:.3} p5 {:.3} p50 {:.3}",
                j[0], p(0.001), p(0.01), p(0.05), p(0.5)
            );
        }
    }
}

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
/// Wavenumber band owned by each wind cascade (rad/m); bands abut exactly.
pub const BAND_EDGES: [f32; WIND_CASCADES + 1] = [0.0, 0.5, 2.0, 1.0e9];
/// Swell: a narrow-band long-crested sea from distant weather, crossing the
/// local wind sea so their crests collide. Normalised to 1m significant height
/// and scaled at run time by `swell_height_meters`.
const SWELL_PEAK_WAVELENGTH_METERS: f32 = 170.0;
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
    let k_peak = std::f32::consts::TAU / SWELL_PEAK_WAVELENGTH_METERS;
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
pub fn wind_speed_from_environment() -> f32 {
    std::env::var("CATINGARDEN_OCEAN_FFT_WIND")
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .unwrap_or(14.0)
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
    second_order_means: [f64; CASCADES],
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
/// Lattice steps the background worker builds ahead of the frontier.
const PREFETCH_STEPS: i64 = 3;

/// Horizontal (choppy) displacement strength; 1.0 is the Tessendorf field the
/// fold Jacobian is computed from, 0 disables it. `CATINGARDEN_OCEAN_FFT_CHOP`.
pub fn choppiness() -> f32 {
    static VALUE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("CATINGARDEN_OCEAN_FFT_CHOP")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .map_or(1.0, |v| v.clamp(0.0, 2.0))
    })
}

/// Significant wave height (metres) of the swell cascade. Swell is generated
/// by distant storms, so it is present in calm local weather; the local storm
/// raises it by up to 1.8x. Base from `CATINGARDEN_OCEAN_FFT_SWELL` (default 8).
pub fn swell_height_meters(storm_intensity: f32) -> f32 {
    static BASE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    let base = *BASE.get_or_init(|| {
        std::env::var("CATINGARDEN_OCEAN_FFT_SWELL")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .map_or(8.0, |v| v.clamp(0.0, 15.0))
    });
    let t = ((storm_intensity - 0.15) / 0.70).clamp(0.0, 1.0);
    base * (1.0 + 0.8 * t * t * (3.0 - 2.0 * t))
}

/// Strength of the second-order (Stokes) crest term, `CATINGARDEN_OCEAN_FFT_PEAKS`
/// (default 1, 0-3). 1 is second-order Stokes for a single wave: crests rise
/// and troughs flatten by k a^2 / 2. Where crests cross it adds several times
/// that, which is what piles colliding crests into higher peaks.
pub fn second_order_strength() -> f32 {
    static VALUE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("CATINGARDEN_OCEAN_FFT_PEAKS")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .map_or(1.0, |v| v.clamp(0.0, 3.0))
    })
}

/// Divergence of D is clamped to this before the second-order product, so a
/// near-fold cannot throw a spike.
pub const SECOND_ORDER_DIVERGENCE_LIMIT: f64 = 0.6;

/// Spatial mean of h * div(D) per cascade (Parseval: sum of |k| |h~(k)|^2),
/// subtracted so the second-order term does not raise mean sea level.
pub fn second_order_means(h0: &[[f32; 4]]) -> [f64; CASCADES] {
    let mut out = [0.0; CASCADES];
    for (c, mean) in out.iter_mut().enumerate() {
        let dk = std::f64::consts::TAU / TILE_METERS[c] as f64;
        for y in 0..GRID {
            for x in 0..GRID {
                let t = h0[c * GRID * GRID + y * GRID + x];
                let (re, im) = ((t[0] + t[2]) as f64, (t[1] - t[3]) as f64);
                let n = x as f64 - GRID as f64 / 2.0;
                let m = y as f64 - GRID as f64 / 2.0;
                *mean += dk * (n * n + m * m).sqrt() * (re * re + im * im);
            }
        }
    }
    out
}

/// Mean second-order lift of the geometry cascades (wind 0, mid 1, swell
/// scaled by its height), times the strength.
fn second_order_offset(means: &[f64; CASCADES], swell_height: f64) -> f64 {
    second_order_strength() as f64
        * (means[0] + means[1] + swell_height * swell_height * means[SWELL_CASCADE])
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
        let tile_meters = TILE_METERS[cascade] as f64;
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
        Self { shared, second_order_means: second_order_means(h0) }
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
                let field = |position: [f64; 2]| {
                    let mut total = FieldSample::default();
                    for ((cascade, slot), scale) in cascades.iter().zip(&slots).zip(scales) {
                        total.add_scaled(&sample_slot(slot, cascade.tile_meters, position, delta), scale);
                    }
                    total
                };
                // Mirrors the shader's fold limiter: displacement eases off
                // where the surface would otherwise turn inside out.
                let limited = |sample: &FieldSample| {
                    let j = sample.jacobian;
                    let raw = (1.0 - chop * j[0]) * (1.0 - chop * j[1]) - chop * chop * j[2] * j[3];
                    chop * (0.25 + 0.75 * smoothstep(0.1, 0.6, raw))
                };
                let mut label = target;
                let mut sample = field(label);
                for _ in 0..INVERSE_ITERATIONS {
                    let c = limited(&sample);
                    let residual = [
                        label[0] - c * sample.displacement[0] - target[0],
                        label[1] - c * sample.displacement[1] - target[1],
                    ];
                    let j = sample.jacobian;
                    // d(label - cD)/d(label), rows u and v.
                    let (a, b, cc, d) = (1.0 - c * j[0], -c * j[2], -c * j[3], 1.0 - c * j[1]);
                    // Within a millimetre: the sample in hand is the answer.
                    if residual[0].abs() + residual[1].abs() < 1.0e-3 {
                        break;
                    }
                    let det = (a * d - b * cc).max(0.2);
                    label[0] -= (d * residual[0] - b * residual[1]) / det;
                    label[1] -= (-cc * residual[0] + a * residual[1]) / det;
                    sample = field(label);
                }
                let c = limited(&sample);
                let j = sample.jacobian;
                let (a, b, cc, d) = (1.0 - c * j[0], -c * j[2], -c * j[3], 1.0 - c * j[1]);
                let det = (a * d - b * cc).max(0.35);
                // Second-order crest term, as the shader adds it.
                let strength = second_order_strength() as f64;
                let divergence = (j[0] + j[1])
                    .clamp(-SECOND_ORDER_DIVERGENCE_LIMIT, SECOND_ORDER_DIVERGENCE_LIMIT);
                let lift = 1.0 + 2.0 * strength * divergence;
                let height = sample.height
                    + strength * sample.height * divergence
                    - second_order_offset(&self.second_order_means, swell_height);
                let [hu, hv] = [sample.slope[0] * lift, sample.slope[1] * lift];
                (
                    CpuSample {
                        height,
                        slope_uv: [(d * hu - cc * hv) / det, (-b * hu + a * hv) / det],
                        velocity: sample.velocity * lift,
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
/// cascade, the camera's fractional tile coordinates (u, v), tile length and 0.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewParams {
    pub axis_u: [f32; 4],
    pub axis_v: [f32; 4],
    pub cascade: [[f32; 4]; CASCADES],
    /// x: overall gain, y: choppiness, z: swell height (m), w: unused.
    pub gain: [f32; 4],
    /// x: second-order strength, y: its mean (m) to subtract.
    pub second_order: [f32; 4],
}

pub struct OceanFft {
    params: wgpu::Buffer,
    h0: wgpu::Buffer,
    pub field: wgpu::Texture,
    pub field_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub view_params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    mip_pipeline: wgpu::ComputePipeline,
    mip_groups: Vec<wgpu::BindGroup>,
    evolve: wgpu::ComputePipeline,
    rows: wgpu::ComputePipeline,
    cols: wgpu::ComputePipeline,
    assemble: wgpu::ComputePipeline,
    second_order_means: [f64; CASCADES],
}

impl OceanFft {
    pub fn new(device: &wgpu::Device, h0: &[[f32; 4]]) -> Self {
        assert_eq!(h0.len(), CASCADES * GRID * GRID);
        let means = second_order_means(h0);
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
        for (t, l) in tile.iter_mut().zip(TILE_METERS) {
            t[0] = l;
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
            ],
        });
        Self {
            params,
            h0,
            field,
            field_view,
            sampler,
            view_params,
            bind_group,
            mip_pipeline,
            mip_groups,
            evolve: pipeline("evolve"),
            rows: pipeline("fft_rows"),
            cols: pipeline("fft_cols"),
            assemble: pipeline("assemble"),
            second_order_means: means,
        }
    }

    /// Anchors the tangent plane at the first camera direction, then keeps it
    /// fixed so the wave pattern never slides; only the camera's fractional
    /// tile coordinates change per frame.
    pub fn update_view(
        &self,
        queue: &wgpu::Queue,
        camera_direction: [f64; 3],
        radius_meters: f64,
        gain: f32,
        storm_intensity: f32,
    ) {
        let (u, v) = anchor_axes(camera_direction);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let (cu, cv) = (radius_meters * dot(u, camera_direction), radius_meters * dot(v, camera_direction));
        let mut cascade = [[0.0f32; 4]; CASCADES];
        for (c, entry) in cascade.iter_mut().enumerate() {
            let length = TILE_METERS[c] as f64;
            *entry = [(cu / length).rem_euclid(1.0) as f32, (cv / length).rem_euclid(1.0) as f32, TILE_METERS[c], 0.0];
        }
        let params = ViewParams {
            axis_u: [u[0] as f32, u[1] as f32, u[2] as f32, 0.0],
            axis_v: [v[0] as f32, v[1] as f32, v[2] as f32, 0.0],
            cascade,
            gain: [gain, choppiness(), swell_height_meters(storm_intensity), 0.0],
            second_order: [
                second_order_strength(),
                second_order_offset(&self.second_order_means, swell_height_meters(storm_intensity) as f64)
                    as f32,
                0.0,
                0.0,
            ],
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
        let dk = std::f32::consts::TAU / TILE_METERS[0];
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
            let j = field.jacobian;
            let raw = (1.0 - chop * j[0]) * (1.0 - chop * j[1]) - chop * chop * j[2] * j[3];
            let c = chop * (0.25 + 0.75 * smoothstep(0.1, 0.6, raw));
            let target = [label[0] - c * field.displacement[0], label[1] - c * field.displacement[1]];
            let (sample, found) = cpu.sample_at(target, time, storm);
            let miss = ((found[0] - label[0]).powi(2) + (found[1] - label[1]).powi(2)).sqrt();
            eprintln!(
                "label {label:?}: displaced {:.2} m, recovered within {miss:.4} m, height {:.3} vs {:.3}",
                (c * c * (field.displacement[0].powi(2) + field.displacement[1].powi(2))).sqrt(),
                sample.height,
                field.height
            );
            let divergence = (j[0] + j[1])
                .clamp(-SECOND_ORDER_DIVERGENCE_LIMIT, SECOND_ORDER_DIVERGENCE_LIMIT);
            let strength = second_order_strength() as f64;
            let expected = field.height + strength * field.height * divergence
                - second_order_offset(&cpu.second_order_means, scale);
            assert!(miss < 0.02, "{miss}");
            assert!((sample.height - expected).abs() < 0.005, "{} vs {expected}", sample.height);
        }
    }

    #[test]
    fn second_order_term_is_stokes_for_one_wave_and_keeps_mean_level() {
        // One mode: h = a cos(kx); second order must add (k a^2 / 2) cos(2kx).
        let mut h0 = vec![[0.0f32; 4]; CASCADES * GRID * GRID];
        let (n, amp) = (8usize, 0.5f32);
        h0[(GRID / 2) * GRID + GRID / 2 + n] = [amp, 0.0, amp, 0.0];
        h0[(GRID / 2) * GRID + GRID / 2 - n] = [amp, 0.0, amp, 0.0];
        let means = second_order_means(&h0);
        let k = std::f64::consts::TAU * n as f64 / TILE_METERS[0] as f64;
        let a = 4.0 * amp as f64; // both +k and -k, each doubled by h0(-k)
        assert!((means[0] - 0.5 * k * a * a).abs() < 1e-6 * means[0].max(1.0), "{} vs {}", means[0], 0.5 * k * a * a);
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
            let step = TILE_METERS[c] / GRID as f32;
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

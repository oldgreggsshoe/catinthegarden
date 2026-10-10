//! Storm gusts: patches of stronger, veered wind carried downwind over the sea.
//!
//! A gust is a lump of faster air brought down from above and swept along by
//! the mean wind. Over water it shows as a rougher, whitecapped patch running
//! downwind; standing in it, as a surge that drives the rain flatter and
//! throws more spray. Modelled as frozen turbulence (Taylor's hypothesis): one
//! fixed 2D field, slid downwind at the mean wind speed.
//!
//! The field is evaluated in two places: here on the CPU at the camera, for
//! the rain, and in `gust.wgsl` on the GPU, for the sea and the spray. They
//! agree because the camera's place in the field is formed here in f64 and
//! handed over reduced to one period of it, where f32 resolves millimetres.

use glam::DVec3;

/// Gust patches are longer along the wind than across it.
pub const ALONG_METERS: f64 = 240.0;
pub const ACROSS_METERS: f64 = 140.0;
/// The field repeats after this many cells each way (61km along the wind,
/// about an hour for the wind to carry it past): what lets the camera's place
/// in it be handed over as a small number.
pub const PERIOD_CELLS: u32 = 256;
/// Normalises the two noise octaves to a standard deviation of 0.5, so a
/// one-in-twenty gust reaches 0.82 of the full spread. Pinned by a test.
pub const FIELD_GAIN: f64 = 1.0;
/// Strongest gust over the mean wind at full storm, as a fraction of it; the
/// deepest lull falls as far below. Storm gusts run 1.3-1.5x the mean.
pub const SPEED_SPREAD: f64 = 0.45;
/// How far a full gust veers the wind (12 degrees): the faster air comes down
/// from aloft, where the wind blows at a different angle.
pub const VEER_RADIANS: f64 = 0.21;
/// Storm overcast (main.rs `update_storm_overcast`) over which the wind turns
/// from steady to fully gusty.
const GUSTINESS_ONSET: f32 = 0.2;
const GUSTINESS_FULL: f32 = 0.9;

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How gusty the wind is under this storm overcast: 0 steady, 1 full storm.
pub fn gustiness(storm_overcast: f32) -> f32 {
    smoothstep(GUSTINESS_ONSET, GUSTINESS_FULL, storm_overcast)
}

/// `PLANET_GUSTS=0` keeps the storm wind steady, for comparisons.
pub fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        !matches!(
            std::env::var("PLANET_GUSTS")
                .ok()
                .as_deref()
                .map(str::trim),
            Some("0" | "off" | "false")
        )
    })
}

/// Unit mean wind direction in the FFT plane (u, v): the sea's own wind.
pub fn mean_direction_uv() -> [f64; 2] {
    let [x, y] = crate::ocean_fft::WIND_DIRECTION.map(f64::from);
    let length = x.hypot(y);
    [x / length, y / length]
}

/// Mean wind speed (m/s): the sea's own wind.
pub fn mean_speed() -> f64 {
    f64::from(crate::ocean_fft::wind_speed_from_environment())
}

fn hash(x: u32, y: u32, salt: u32) -> f64 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ salt.wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    f64::from(h >> 8) / 16_777_216.0 * 2.0 - 1.0
}

fn value_noise(p: [f64; 2], period: i64, salt: u32) -> f64 {
    let cell = [p[0].floor(), p[1].floor()];
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (fx, fy) = (fade(p[0] - cell[0]), fade(p[1] - cell[1]));
    let wrap = |c: f64, step: i64| (c as i64 + step).rem_euclid(period) as u32;
    let (x0, x1) = (wrap(cell[0], 0), wrap(cell[0], 1));
    let (y0, y1) = (wrap(cell[1], 0), wrap(cell[1], 1));
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let bottom = lerp(hash(x0, y0, salt), hash(x1, y0, salt), fx);
    let top = lerp(hash(x0, y1, salt), hash(x1, y1, salt), fx);
    lerp(bottom, top, fy)
}

/// The gust at field coordinate `xi` (cells): -1 deepest lull, 1 strongest
/// gust, before the storm's gustiness scales it. Mirrors `gust_field`.
pub fn field(xi: [f64; 2]) -> f64 {
    let period = i64::from(PERIOD_CELLS);
    let broad = value_noise(xi, period, 1);
    let fine = value_noise([2.0 * xi[0] + 0.37, 2.0 * xi[1] + 0.61], 2 * period, 2);
    ((broad + 0.5 * fine) * FIELD_GAIN).clamp(-1.0, 1.0)
}

/// Field coordinate of the FFT-plane point `plane_uv` (metres) at `time`, one
/// period's worth: the field slides downwind at the mean wind speed.
pub fn origin(plane_uv: [f64; 2], time_seconds: f64) -> [f64; 2] {
    let along = mean_direction_uv();
    let across = [-along[1], along[0]];
    let period = f64::from(PERIOD_CELLS);
    // Travel, not speed x time: the live keys change the wind, and speed x
    // time would jump the whole field by the change x the time so far.
    let a = plane_uv[0] * along[0] + plane_uv[1] * along[1]
        - crate::ocean_fft::wind_travel_meters(time_seconds);
    let c = plane_uv[0] * across[0] + plane_uv[1] * across[1];
    [
        (a / ALONG_METERS).rem_euclid(period),
        (c / ACROSS_METERS).rem_euclid(period),
    ]
}

/// The wind a gust of `gust` (already scaled by gustiness) makes of the mean
/// wind, in the FFT plane (m/s). Mirrors `gust_wind`.
pub fn wind_uv(gust: f64) -> [f64; 2] {
    let [x, y] = mean_direction_uv();
    let (s, c) = (VEER_RADIANS * gust).sin_cos();
    let speed = mean_speed() * (1.0 + SPEED_SPREAD * gust);
    [(c * x - s * y) * speed, (s * x + c * y) * speed]
}

/// The gust where the camera is this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gust {
    /// The camera's field coordinate (`origin`).
    pub origin: [f64; 2],
    /// Gustiness, 0-1 (`gustiness`).
    pub strength: f32,
    /// The gust at the camera, scaled by `strength`: -1 to 1.
    pub value: f32,
}

impl Gust {
    pub const CALM: Self = Self {
        origin: [0.0, 0.0],
        strength: 0.0,
        value: 0.0,
    };

    /// `camera_planet_position` is planet-frame (the FFT plane's frame).
    pub fn at_camera(
        camera_planet_position: DVec3,
        time_seconds: f64,
        storm_overcast: f32,
    ) -> Self {
        let direction = camera_planet_position.normalize().to_array();
        let (u, v) = crate::ocean_fft::anchor_axes(direction);
        let radius = crate::planet::planet_radius_meters();
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let origin = origin(
            [radius * dot(u, direction), radius * dot(v, direction)],
            time_seconds,
        );
        let strength = if enabled() {
            gustiness(storm_overcast)
        } else {
            0.0
        };
        Self {
            origin,
            strength,
            value: strength * field(origin) as f32,
        }
    }

    /// Wind at the camera in the FFT plane (m/s).
    pub fn wind_uv(&self) -> [f64; 2] {
        wind_uv(f64::from(self.value))
    }

    /// For the GPU: field origin, gustiness, mean wind speed.
    pub fn uniform(&self) -> [f32; 4] {
        [
            self.origin[0] as f32,
            self.origin[1] as f32,
            self.strength,
            mean_speed() as f32,
        ]
    }
}

/// `gust.wgsl` with its constants, to prepend to a shader that uses it.
pub(crate) fn wgsl_source() -> String {
    let [x, y] = mean_direction_uv();
    format!(
        "const GUST_ALONG_METERS: f32 = {ALONG_METERS:.1};\n\
         const GUST_ACROSS_METERS: f32 = {ACROSS_METERS:.1};\n\
         const GUST_PERIOD_CELLS: u32 = {PERIOD_CELLS}u;\n\
         const GUST_FIELD_GAIN: f32 = {FIELD_GAIN:.3};\n\
         const GUST_SPEED_SPREAD: f32 = {SPEED_SPREAD:.3};\n\
         const GUST_VEER_RADIANS: f32 = {VEER_RADIANS:.3};\n\
         const GUST_WIND_DIRECTION: vec2<f32> = vec2<f32>({x:.7}, {y:.7});\n{}",
        include_str!("gust.wgsl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_points(count: usize) -> impl Iterator<Item = [f64; 2]> {
        // A fixed low-discrepancy walk over one period: deterministic, and
        // spread evenly enough for the statistics below to be stable.
        let period = f64::from(PERIOD_CELLS);
        (0..count).map(move |i| {
            let i = i as f64;
            [
                (i * 0.618_033_988_7 * period).rem_euclid(period),
                (i * 0.754_877_666_2 * period).rem_euclid(period),
            ]
        })
    }

    #[test]
    fn the_field_repeats_after_one_period_so_the_origin_can_be_reduced() {
        let period = f64::from(PERIOD_CELLS);
        for xi in sample_points(500) {
            let value = field(xi);
            assert!((field([xi[0] + period, xi[1]]) - value).abs() < 1.0e-12);
            assert!((field([xi[0], xi[1] - period]) - value).abs() < 1.0e-12);
        }
    }

    #[test]
    fn the_field_is_normalised_to_half_its_range_and_rarely_clips() {
        let values: Vec<f64> = sample_points(100_000).map(field).collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let deviation = (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>()
            / values.len() as f64)
            .sqrt();
        let clipped = values.iter().filter(|v| v.abs() >= 1.0).count() as f64 / values.len() as f64;
        assert!(mean.abs() < 0.02, "mean {mean}");
        assert!(
            (deviation - 0.5).abs() < 0.03,
            "standard deviation {deviation}"
        );
        assert!(clipped < 0.05, "clipped {clipped}");
    }

    #[test]
    fn a_gust_is_carried_downwind_at_the_mean_wind() {
        // Frozen turbulence: whatever gust is at a point now is, ten seconds
        // later, the mean wind's ten seconds further downwind.
        let [x, y] = mean_direction_uv();
        let travel = mean_speed() * 10.0;
        for (i, start) in sample_points(50).enumerate() {
            let point = [start[0] * 170.0 + i as f64, start[1] * 90.0];
            let now = field(origin(point, 3.0));
            let later = field(origin([point[0] + x * travel, point[1] + y * travel], 13.0));
            assert!((now - later).abs() < 1.0e-9, "{now} vs {later}");
        }
    }

    #[test]
    fn gusts_are_faster_and_veered_and_lulls_slower_and_backed() {
        let mean = wind_uv(0.0);
        let gust = wind_uv(1.0);
        let lull = wind_uv(-1.0);
        let speed = |w: [f64; 2]| w[0].hypot(w[1]);
        assert!((speed(mean) - mean_speed()).abs() < 1.0e-9);
        assert!((speed(gust) / speed(mean) - (1.0 + SPEED_SPREAD)).abs() < 1.0e-9);
        assert!((speed(lull) / speed(mean) - (1.0 - SPEED_SPREAD)).abs() < 1.0e-9);
        let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
        assert!(cross(mean, gust) > 0.0 && cross(mean, lull) < 0.0);
    }

    #[test]
    fn calm_weather_has_no_gusts() {
        assert_eq!(gustiness(0.0), 0.0);
        assert_eq!(gustiness(GUSTINESS_ONSET), 0.0);
        assert_eq!(gustiness(1.0), 1.0);
        let calm = Gust::at_camera(DVec3::new(0.0, 0.0, 4.0e6), 12.0, 0.0);
        assert_eq!(calm.value, 0.0);
        assert_eq!(calm.wind_uv(), wind_uv(0.0));
    }

    #[test]
    fn the_gust_shader_parses_with_its_constants() {
        let source = format!(
            "{}\n@compute @workgroup_size(1) fn main() {{ let w = gust_wind(GUST_WIND_DIRECTION, 14.0, gust_field(gust_coordinate(vec2<f32>(3.0, 4.0), vec2<f32>(10.0, -2.0)))); }}",
            wgsl_source()
        );
        let module = wgpu::naga::front::wgsl::parse_str(&source).expect("gust shader must parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("gust shader must validate");
    }
}

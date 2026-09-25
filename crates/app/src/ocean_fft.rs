//! GPU FFT ocean wave field (phase B of the Sea of Thieves plan). Standalone:
//! not yet wired into rendering or buoyancy.
#![allow(dead_code)]

use wgpu::util::DeviceExt;

pub const GRID: usize = 256;
pub const CASCADES: usize = 3;
/// 256 down to 1 texel.
pub const MIP_LEVELS: u32 = 9;
/// Tile edge lengths in metres, chosen so the repeats do not line up.
pub const TILE_METERS: [f32; CASCADES] = [1000.0, 237.0, 53.0];
/// Wavenumber band owned by each cascade (rad/m); bands abut exactly.
pub const BAND_EDGES: [f32; CASCADES + 1] = [0.0, 0.5, 2.0, 1.0e9];
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


/// Wind speed (m/s) for the FFT sea; shared by the GPU spectrum and the CPU model.
pub fn wind_speed_from_environment() -> f32 {
    std::env::var("CATINGARDEN_OCEAN_FFT_WIND")
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .unwrap_or(14.0)
}

pub fn default_h0() -> Vec<[f32; 4]> {
    generate_h0(1, wind_speed_from_environment(), [1.0, 0.3], 80_000.0)
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

/// CPU mirror of cascade 0: the same spectrum the GPU transforms, inverse-FFT'd
/// on the CPU into a height and vertical-velocity grid that is bilinearly
/// interpolated exactly like the GPU sampler. The grid is refreshed only when
/// the ocean clock moves more than `REFRESH_SECONDS`; in between, height is
/// advanced by its velocity (error below ~5mm for the cascade's periods).
pub struct CpuSurface {
    modes: Vec<Mode>,
    twiddle: Vec<[f64; 2]>,
    state: std::sync::Mutex<GridState>,
}

const REFRESH_SECONDS: f64 = 0.03;

struct GridState {
    time: f64,
    valid: bool,
    height: Vec<f32>,
    velocity: Vec<f32>,
}

struct Mode {
    x: usize,
    y: usize,
    h0: [f64; 2],
    h0m: [f64; 2],
    omega: f64,
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

fn cmul(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}

impl CpuSurface {
    pub fn new(h0: &[[f32; 4]]) -> Self {
        let half = GRID as i32 / 2;
        let mut modes = Vec::new();
        for y in 0..GRID {
            for x in 0..GRID {
                let t = h0[y * GRID + x];
                if t == [0.0; 4] {
                    continue;
                }
                let n = (x as i32 - half) as f64;
                let m = (y as i32 - half) as f64;
                let k = (n * n + m * m).sqrt() * std::f64::consts::TAU / TILE_METERS[0] as f64;
                modes.push(Mode {
                    x,
                    y,
                    h0: [t[0] as f64, t[1] as f64],
                    h0m: [t[2] as f64, t[3] as f64],
                    omega: (GRAVITY as f64 * k).sqrt(),
                });
            }
        }
        let twiddle = (0..GRID / 2)
            .map(|j| {
                let a = std::f64::consts::TAU * j as f64 / GRID as f64;
                [a.cos(), a.sin()]
            })
            .collect();
        Self {
            modes,
            twiddle,
            state: std::sync::Mutex::new(GridState {
                time: 0.0,
                valid: false,
                height: vec![0.0; GRID * GRID],
                velocity: vec![0.0; GRID * GRID],
            }),
        }
    }

    pub fn mode_count(&self) -> usize {
        self.modes.len()
    }

    /// In-place unnormalised inverse FFT (e^{+i}) of one 256-point line.
    fn fft_line(&self, line: &mut [[f64; 2]; GRID]) {
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
                    let w = self.twiddle[j * stride];
                    let t = cmul(w, line[start + j + half]);
                    let u = line[start + j];
                    line[start + j] = [u[0] + t[0], u[1] + t[1]];
                    line[start + j + half] = [u[0] - t[0], u[1] - t[1]];
                }
            }
            half *= 2;
        }
    }

    fn refresh(&self, state: &mut GridState, time: f64) {
        // Packed spectrum: h_hat + i * v_hat, both Hermitian, so one complex
        // inverse FFT returns height (real) and velocity (imaginary).
        let mut grid = vec![[0.0f64; 2]; GRID * GRID];
        for mode in &self.modes {
            let phase = mode.omega * time;
            let (s, c) = phase.sin_cos();
            let plus = cmul(mode.h0, [c, s]);
            let minus = cmul([mode.h0m[0], -mode.h0m[1]], [c, -s]);
            let h = [plus[0] + minus[0], plus[1] + minus[1]];
            // d/dt: i*omega*plus - i*omega*minus.
            let v = [-mode.omega * (plus[1] - minus[1]), mode.omega * (plus[0] - minus[0])];
            // h + i v
            grid[mode.y * GRID + mode.x] = [h[0] - v[1], h[1] + v[0]];
        }
        let mut line = [[0.0f64; 2]; GRID];
        let mut occupied = [false; GRID];
        for mode in &self.modes {
            occupied[mode.y] = true;
        }
        for y in 0..GRID {
            if !occupied[y] {
                continue;
            }
            line.copy_from_slice(&grid[y * GRID..(y + 1) * GRID]);
            self.fft_line(&mut line);
            grid[y * GRID..(y + 1) * GRID].copy_from_slice(&line);
        }
        for x in 0..GRID {
            for y in 0..GRID {
                line[y] = grid[y * GRID + x];
            }
            self.fft_line(&mut line);
            for y in 0..GRID {
                let sign = if (x + y) & 1 == 1 { -1.0 } else { 1.0 };
                let value = line[y];
                state.height[y * GRID + x] = (sign * value[0]) as f32;
                state.velocity[y * GRID + x] = (sign * value[1]) as f32;
            }
        }
        state.time = time;
        state.valid = true;
    }

    pub fn sample(&self, direction: [f64; 3], radius_meters: f64, time: f64) -> CpuSample {
        let (u, v) = anchor_axes(direction);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let length = TILE_METERS[0] as f64;
        let tx = ((radius_meters * dot(u, direction) / length).rem_euclid(1.0)) * GRID as f64 - 0.5;
        let ty = ((radius_meters * dot(v, direction) / length).rem_euclid(1.0)) * GRID as f64 - 0.5;
        let (i0, j0) = (tx.floor() as i64, ty.floor() as i64);
        let (fx, fy) = (tx - i0 as f64, ty - j0 as f64);
        let mut state = self.state.lock().unwrap();
        if !state.valid || (time - state.time).abs() > REFRESH_SECONDS {
            self.refresh(&mut state, time);
        }
        let delta = time - state.time;
        let at = |i: i64, j: i64| {
            let index = (j.rem_euclid(GRID as i64) as usize) * GRID + i.rem_euclid(GRID as i64) as usize;
            (
                state.height[index] as f64 + delta * state.velocity[index] as f64,
                state.velocity[index] as f64,
            )
        };
        let bilinear = |ox: i64, oy: i64, pick: fn((f64, f64)) -> f64| {
            let c = |a: i64, b: i64| pick(at(i0 + a + ox, j0 + b + oy));
            (c(0, 0) * (1.0 - fx) + c(1, 0) * fx) * (1.0 - fy)
                + (c(0, 1) * (1.0 - fx) + c(1, 1) * fx) * fy
        };
        let base = bilinear(0, 0, |c| c.0);
        let step_meters = length / GRID as f64;
        CpuSample {
            height: base,
            slope_uv: [
                (bilinear(1, 0, |c| c.0) - base) / step_meters,
                (bilinear(0, 1, |c| c.0) - base) / step_meters,
            ],
            velocity: bilinear(0, 0, |c| c.1),
            axis_u: u,
            axis_v: v,
        }
    }

    #[cfg(test)]
    fn texel(&self, i: usize, j: usize, time: f64) -> f64 {
        let mut state = self.state.lock().unwrap();
        self.refresh(&mut state, time);
        state.height[j * GRID + i] as f64
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
    pub gain: [f32; 4],
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
}

impl OceanFft {
    pub fn new(device: &wgpu::Device, h0: &[[f32; 4]]) -> Self {
        assert_eq!(h0.len(), CASCADES * GRID * GRID);
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
        }
    }

    /// Anchors the tangent plane at the first camera direction, then keeps it
    /// fixed so the wave pattern never slides; only the camera's fractional
    /// tile coordinates change per frame.
    pub fn update_view(&self, queue: &wgpu::Queue, camera_direction: [f64; 3], radius_meters: f64, gain: f32) {
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
            gain: [gain, 0.0, 0.0, 0.0],
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
mod tests {
    use super::*;

    fn device() -> (wgpu::Device, wgpu::Queue) {
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

    fn read_field(device: &wgpu::Device, queue: &wgpu::Queue, fft: &OceanFft) -> Vec<[f32; 4]> {
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
        eprintln!("ocean FFT (3 x 256^2, 6 FFTs) ms/frame: {samples:.3?}");
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
        let (mut max_err, mut max_h) = (0.0f64, 0.0f64);
        for &(i, j) in &[(0usize, 0usize), (5, 9), (100, 200), (255, 255), (128, 64), (17, 240), (77, 3)] {
            let gpu = field[j * GRID + i][0] as f64;
            max_err = max_err.max((gpu - cpu.texel(i, j, time)).abs());
            max_h = max_h.max(gpu.abs());
        }
        eprintln!("cpu vs gpu texel max error {max_err:.5} m (max height {max_h:.3}), modes {}", cpu.mode_count());
        assert!(max_err < 0.02, "max error {max_err}");
    }

    #[test]
    fn cpu_velocity_matches_numeric_derivative_and_refresh_is_cheap() {
        let cpu = CpuSurface::new(&default_h0());
        let d = [0.836_f64, 0.504, 0.216];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let d = [d[0] / l, d[1] / l, d[2] / l];
        let r = 4_000_000.0;
        let a = cpu.sample(d, r, 10.0);
        let b = cpu.sample(d, r, 10.001);
        let numeric = (b.height - a.height) / 0.001;
        assert!((numeric - a.velocity).abs() < 5e-3, "numeric {numeric} analytic {}", a.velocity);
        // Height between refreshes tracks a fresh transform to a few mm.
        let near = cpu.sample(d, r, 10.025).height;
        let fresh = {
            let other = CpuSurface::new(&default_h0());
            other.sample(d, r, 10.025).height
        };
        eprintln!("held-grid error {:.5} m", (near - fresh).abs());
        assert!((near - fresh).abs() < 0.01);
        let start = std::time::Instant::now();
        for k in 0..20 {
            std::hint::black_box(CpuSurface::sample(&cpu, d, r, 100.0 + k as f64));
        }
        eprintln!("refresh+sample: {:.3} ms each", start.elapsed().as_secs_f64() * 50.0);
    }
}

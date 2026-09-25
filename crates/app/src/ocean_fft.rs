//! GPU FFT ocean wave field (phase B of the Sea of Thieves plan). Standalone:
//! not yet wired into rendering or buoyancy.
#![allow(dead_code)]

use wgpu::util::DeviceExt;

pub const GRID: usize = 256;
pub const CASCADES: usize = 3;
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
    axes: std::cell::Cell<Option<([f64; 3], [f64; 3])>>,
    bind_group: wgpu::BindGroup,
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
            mip_level_count: 1,
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean fft bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: h0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: spec.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&field.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })) },
            ],
        });
        Self {
            params,
            h0,
            field,
            field_view,
            sampler,
            view_params,
            axes: std::cell::Cell::new(None),
            bind_group,
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
        let (u, v) = self.axes.get().unwrap_or_else(|| {
            let d = camera_direction;
            let helper = if d[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
            let cross = |a: [f64; 3], b: [f64; 3]| {
                [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
            };
            let norm = |a: [f64; 3]| {
                let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
                [a[0] / l, a[1] / l, a[2] / l]
            };
            let u = norm(cross(helper, d));
            let v = cross(d, u);
            self.axes.set(Some((u, v)));
            (u, v)
        });
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
        self.encode_mask(encoder, 0b1111);
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
}

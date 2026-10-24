//! One mipmapped timber material, shared by every resident road snapshot.
use wgpu::util::DeviceExt;

const SIZE: u32 = 1024;
const LEVELS: u32 = 11;
pub struct Material {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

fn linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}
fn pixels() -> Vec<u8> {
    let image = image::load_from_memory(include_bytes!(
        "../../../assets/road_materials/weathered_oak.png"
    ))
    .expect("embedded timber texture must decode")
    .to_rgb8();
    // Filter albedo in linear light. Alpha stores a very shallow grain-height
    // proxy, not transparency; its derivatives add relief without more samples.
    let mut level = image::Rgba32FImage::from_fn(image.width(), image.height(), |x, y| {
        let p = image
            .get_pixel(x, y)
            .0
            .map(|v| linear(f32::from(v) / 255.0));
        let height = 0.15 + 0.7 * (0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]);
        image::Rgba([p[0], p[1], p[2], height])
    });
    level = image::imageops::resize(&level, SIZE, SIZE, image::imageops::FilterType::Triangle);
    let mut result = Vec::new();
    loop {
        for p in level.pixels() {
            result.extend(
                p.0[..3]
                    .iter()
                    .map(|&c| (srgb(c).clamp(0.0, 1.0) * 255.0).round() as u8),
            );
            result.push((p[3] * 255.0).round() as u8);
        }
        if level.width() == 1 {
            break;
        }
        level = image::imageops::resize(
            &level,
            level.width() / 2,
            level.height() / 2,
            image::imageops::FilterType::Triangle,
        );
    }
    result
}
impl Material {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("cached timber albedo and grain height"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: LEVELS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &pixels(),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("anisotropic repeating timber"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        Self {
            view: texture.create_view(&Default::default()),
            sampler,
        }
    }
}
#[cfg(test)]
pub(crate) fn mean_red_albedo() -> f64 {
    let p = pixels();
    f64::from(linear(f32::from(p[p.len() - 4]) / 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timber_has_complete_mips_grain_and_bounded_height() {
        let p = pixels();
        assert_eq!(
            p.len(),
            (0..LEVELS)
                .map(|l| ((SIZE >> l).pow(2) * 4) as usize)
                .sum::<usize>()
        );
        let base = &p[..(SIZE * SIZE * 4) as usize];
        let lo = base.chunks_exact(4).map(|p| p[3]).min().unwrap();
        let hi = base.chunks_exact(4).map(|p| p[3]).max().unwrap();
        assert!(hi - lo > 32, "grain height must have useful detail");
        assert!(
            base.chunks_exact(4).all(|p| p[3] < 255),
            "no clipped grain heights"
        );
        assert!(p[p.len() - 4] > p[p.len() - 2], "natural warm timber mean");
    }
}

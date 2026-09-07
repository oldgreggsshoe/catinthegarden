//! The moon's albedo markings, baked once into a cubemap at startup.
//!
//! Haloes and ray systems are a function of surface direction alone, so
//! rederiving them per fragment was work the frame did not need to do: the
//! 96-marking loop cost 12.1ms of every 28.7ms moon-surface frame, 42% of it.
//! Baking is what the outmap already does for height, biome and moisture, and
//! this is the same idea without adding a fourth channel to it.
//!
//! A cubemap rather than an outmap channel because the markings are coarse:
//! the finest feature is the ray azimuth harmonic near a small marking's rim,
//! about 6km across, on a body of radius 1,080km. One face at [`FACE_SIZE`]
//! resolves that several times over for a few megabytes, and needs neither a
//! schema bump nor a rebake of the height field.
//!
//! Rendered on the GPU rather than computed on the CPU so the catalogue cannot
//! drift: the bake shader is built from the same `moon::wgsl_constants()` as
//! the surface shader, and evaluates the same code the surface shader used to.

use crate::body;

/// Texels along one cube face edge. Six faces at one byte each is 6.3MB.
pub const FACE_SIZE: u32 = 1024;

/// R8 is a ~1% step in the marking factor. The fade it quantises is a smooth
/// gradient over a large area, so this is the value to revisit first if the
/// haloes ever show banding; R16 doubles the memory and nothing else.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Builds the marking cubemap for the active body.
///
/// Off the moon this is a 1x1 placeholder: the markings would never be sampled,
/// and baking a full one would spend six million fragments' worth of a
/// catalogue the body does not have.
pub fn create(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let bake = body::active() == body::MOON;
    let size = if bake { FACE_SIZE } else { 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("moon marking cubemap"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if bake {
        bake_faces(device, queue, &texture);
    } else {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0_u8; 6],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("moon marking cubemap sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    (texture, view, sampler)
}

/// The shader the bake runs, catalogue included.
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}",
        body::wgsl_constants(),
        include_str!("moon_markings.wgsl")
    )
}

fn bake_faces(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("moon marking bake"),
        source: wgpu::ShaderSource::Wgsl(shader_source().into()),
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("moon marking bake layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("moon marking bake pipeline layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("moon marking bake pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_moon_markings"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_moon_markings"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("moon marking bake encoder"),
    });
    // One buffer per face rather than one rewritten six times: the passes are
    // recorded into a single encoder, so a shared buffer would hand all six the
    // last face written.
    let mut faces = Vec::with_capacity(6);
    for face in 0..6_u32 {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("moon marking bake face"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut contents = [0_u8; 16];
        contents[0..4].copy_from_slice(&face.to_le_bytes());
        contents[4..8].copy_from_slice(&(FACE_SIZE as f32).to_le_bytes());
        queue.write_buffer(&uniform, 0, &contents);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("moon marking bake face bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("moon marking bake face view"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: face,
            array_layer_count: Some(1),
            ..Default::default()
        });
        faces.push((bind_group, view));
    }
    for (bind_group, view) in &faces {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("moon marking bake pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.submit(Some(encoder.finish()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bake has to carry the same catalogue the surface shader is built
    /// with, or the haloes sit on craters that are not there.
    #[test]
    fn the_bake_shader_carries_the_marking_catalogue() {
        let source = body::with_body(body::MOON, shader_source);
        for required in [
            "const MOON_MARKING_COUNT:",
            "const MOON_MARKINGS:",
            "const MOON_MARKING_TRAITS:",
            "const MOON_MARKING_EXTENTS:",
            "fn moon_marking_brightening(",
        ] {
            assert!(
                source.contains(required),
                "bake shader is missing {required}"
            );
        }
    }

    /// The surface shader must no longer walk the catalogue: that loop is the
    /// 12.1ms this exists to remove, and leaving a second copy behind would put
    /// it straight back.
    #[test]
    fn the_surface_shader_samples_the_map_instead_of_walking_the_catalogue() {
        let surface = include_str!("shared_planet.wgsl");
        assert!(surface.contains("textureSampleLevel(\n        moon_marking_map,"));
        assert!(
            !surface.contains("MOON_MARKING_COUNT"),
            "the surface shader is walking the marking catalogue again",
        );
    }
}

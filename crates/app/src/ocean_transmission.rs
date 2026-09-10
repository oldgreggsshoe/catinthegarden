//! Immutable pre-water scene snapshots: never sample a live render attachment.
pub(crate) struct OceanTransmission {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    color: wgpu::Texture,
    depth: wgpu::Texture,
}

impl OceanTransmission {
    pub fn new(device: &wgpu::Device, size: wgpu::Extent3d) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean transmission layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pre-water snapshot"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let color = texture(crate::hdr::HdrRenderer::SCENE_FORMAT);
        let depth = texture(wgpu::TextureFormat::Depth32Float);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pre-water scene"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(
                        &color.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(
                        &depth.create_view(&Default::default()),
                    ),
                },
            ],
        });
        Self {
            layout,
            bind_group,
            color,
            depth,
        }
    }

    pub fn snapshot(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::Texture,
        depth: &wgpu::Texture,
    ) {
        if self.color.size() != color.size() {
            *self = Self::new(device, color.size());
        }
        encoder.copy_texture_to_texture(
            color.as_image_copy(),
            self.color.as_image_copy(),
            color.size(),
        );
        let mut source = depth.as_image_copy();
        source.aspect = wgpu::TextureAspect::DepthOnly;
        let mut destination = self.depth.as_image_copy();
        destination.aspect = wgpu::TextureAspect::DepthOnly;
        encoder.copy_texture_to_texture(source, destination, depth.size());
    }
}

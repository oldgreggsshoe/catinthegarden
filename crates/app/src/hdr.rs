use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};

use crate::planet::{BLOOM_ENABLED, BLUR_ENABLED, HDR_EFFECT_ENABLED};

const EXPOSURE_KEY: f32 = 0.18;
const EXPOSURE_EPSILON: f32 = 1.0e-4;
const EXPOSURE_ADAPT_SPEED: f32 = 1.5;
const MINIMUM_EXPOSURE: f32 = 0.05;
const MAXIMUM_EXPOSURE: f32 = 4.0;
const READBACK_RING_SIZE: usize = 3;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ExposureUniform {
    exposure: f32,
    hdr_effect_enabled: u32,
    presentation_size: [f32; 2],
}

struct PendingLuminanceReadback {
    receiver: mpsc::Receiver<bool>,
}

struct LuminanceReadbackSlot {
    buffer: wgpu::Buffer,
    pending: Option<PendingLuminanceReadback>,
}

#[derive(Clone, Copy, Debug)]
pub struct ExposureState {
    pub exposure: f32,
    pub target_exposure: f32,
    pub average_luminance: f32,
}

pub struct HdrRenderer {
    size: winit::dpi::PhysicalSize<u32>,
    presentation_size: winit::dpi::PhysicalSize<u32>,
    _scene_texture: wgpu::Texture,
    scene_view: wgpu::TextureView,
    blur_texture: wgpu::Texture,
    blur_view: wgpu::TextureView,
    bloom_blur_texture: wgpu::Texture,
    bloom_blur_view: wgpu::TextureView,
    bloom_texture: wgpu::Texture,
    bloom_view: wgpu::TextureView,
    luminance_texture: wgpu::Texture,
    luminance_mip_views: Vec<wgpu::TextureView>,
    luminance_from_scene_bind_group: wgpu::BindGroup,
    luminance_downsample_bind_groups: Vec<wgpu::BindGroup>,
    tone_bind_group: wgpu::BindGroup,
    blur_bind_group: wgpu::BindGroup,
    bloom_bind_group: wgpu::BindGroup,
    luminance_bind_group_layout: wgpu::BindGroupLayout,
    tone_bind_group_layout: wgpu::BindGroupLayout,
    bloom_bind_group_layout: wgpu::BindGroupLayout,
    luminance_from_scene_pipeline: wgpu::RenderPipeline,
    luminance_downsample_pipeline: wgpu::RenderPipeline,
    tone_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    bloom_blur_pipeline: wgpu::RenderPipeline,
    bloom_pipeline: wgpu::RenderPipeline,
    exposure_buffer: wgpu::Buffer,
    readback_slots: Vec<LuminanceReadbackSlot>,
    next_readback_slot: usize,
    average_luminance: f32,
    target_exposure: f32,
    exposure: f32,
    blur_enabled: bool,
    bloom_enabled: bool,
    hdr_effect_enabled: bool,
}

impl HdrRenderer {
    pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

    pub fn new(
        device: &wgpu::Device,
        size: winit::dpi::PhysicalSize<u32>,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let luminance_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("HDR luminance source layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
        let tone_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("HDR tone map layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let bloom_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("HDR bloom composite layout"),
                entries: &[texture_layout_entry(0), texture_layout_entry(2)],
            });
        let exposure_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto exposure uniform"),
            size: size_of::<ExposureUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("hdr.wgsl"));
        let luminance_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("HDR luminance pipeline layout"),
                bind_group_layouts: &[Some(&luminance_bind_group_layout)],
                immediate_size: 0,
            });
        let tone_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HDR tone map pipeline layout"),
            bind_group_layouts: &[Some(&tone_bind_group_layout)],
            immediate_size: 0,
        });
        let bloom_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("HDR bloom pipeline layout"),
                bind_group_layouts: &[Some(&bloom_bind_group_layout)],
                immediate_size: 0,
            });
        let luminance_from_scene_pipeline = fullscreen_pipeline(
            device,
            &shader,
            &luminance_pipeline_layout,
            "luminance_from_scene",
            Self::SCENE_FORMAT,
            "HDR source luminance pipeline",
        );
        let luminance_downsample_pipeline = fullscreen_pipeline(
            device,
            &shader,
            &luminance_pipeline_layout,
            "luminance_downsample",
            Self::SCENE_FORMAT,
            "HDR luminance downsample pipeline",
        );
        let tone_pipeline = fullscreen_pipeline(
            device,
            &shader,
            &tone_pipeline_layout,
            "tone_map",
            surface_format,
            "ACES tone map pipeline",
        );
        let blur_pipeline = fullscreen_pipeline(
            device,
            &shader,
            &luminance_pipeline_layout,
            "blur_scene",
            Self::SCENE_FORMAT,
            "HDR blur pipeline",
        );
        let bloom_pipeline = fullscreen_pipeline(
            device,
            &shader,
            &bloom_pipeline_layout,
            "bloom_composite",
            Self::SCENE_FORMAT,
            "HDR bloom composite pipeline",
        );
        let bloom_blur_pipeline = fullscreen_pipeline(
            device,
            &shader,
            &luminance_pipeline_layout,
            "bloom_blur",
            Self::SCENE_FORMAT,
            "HDR bloom blur pipeline",
        );

        let readback_slots = (0..READBACK_RING_SIZE)
            .map(|_| LuminanceReadbackSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("1x1 luminance readback"),
                    size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                pending: None,
            })
            .collect();

        let placeholder_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("initial HDR scene target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let placeholder_view =
            placeholder_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let placeholder_blur = create_scene_texture(device, winit::dpi::PhysicalSize::new(1, 1));
        let placeholder_blur_view = placeholder_blur.create_view(&Default::default());
        let placeholder_bloom_blur =
            create_scene_texture(device, winit::dpi::PhysicalSize::new(1, 1));
        let placeholder_bloom_blur_view = placeholder_bloom_blur.create_view(&Default::default());
        let placeholder_bloom = create_scene_texture(device, winit::dpi::PhysicalSize::new(1, 1));
        let placeholder_bloom_view = placeholder_bloom.create_view(&Default::default());
        let placeholder_luminance = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("initial luminance target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let placeholder_luminance_view =
            placeholder_luminance.create_view(&wgpu::TextureViewDescriptor::default());
        let placeholder_luminance_bind_group =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("initial luminance source"),
                layout: &luminance_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&placeholder_view),
                }],
            });
        let placeholder_tone_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("initial tone map source"),
            layout: &tone_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&placeholder_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: exposure_buffer.as_entire_binding(),
                },
            ],
        });
        let placeholder_blur_bind_group = texture_bind_group(
            device,
            &luminance_bind_group_layout,
            "initial blur source",
            &placeholder_view,
        );
        let placeholder_bloom_bind_group = bloom_bind_group(
            device,
            &bloom_bind_group_layout,
            &placeholder_view,
            &placeholder_bloom_blur_view,
        );
        let mut renderer = Self {
            size: winit::dpi::PhysicalSize::new(1, 1),
            presentation_size: size,
            _scene_texture: placeholder_texture,
            scene_view: placeholder_view,
            blur_texture: placeholder_blur,
            blur_view: placeholder_blur_view,
            bloom_blur_texture: placeholder_bloom_blur,
            bloom_blur_view: placeholder_bloom_blur_view,
            bloom_texture: placeholder_bloom,
            bloom_view: placeholder_bloom_view,
            luminance_texture: placeholder_luminance,
            luminance_mip_views: vec![placeholder_luminance_view],
            luminance_from_scene_bind_group: placeholder_luminance_bind_group,
            luminance_downsample_bind_groups: Vec::new(),
            tone_bind_group: placeholder_tone_bind_group,
            blur_bind_group: placeholder_blur_bind_group,
            bloom_bind_group: placeholder_bloom_bind_group,
            luminance_bind_group_layout,
            tone_bind_group_layout,
            bloom_bind_group_layout,
            luminance_from_scene_pipeline,
            luminance_downsample_pipeline,
            tone_pipeline,
            blur_pipeline,
            bloom_blur_pipeline,
            bloom_pipeline,
            exposure_buffer,
            readback_slots,
            next_readback_slot: 0,
            average_luminance: EXPOSURE_KEY,
            target_exposure: 1.0,
            exposure: 1.0,
            blur_enabled: BLUR_ENABLED,
            bloom_enabled: BLOOM_ENABLED,
            hdr_effect_enabled: HDR_EFFECT_ENABLED,
        };
        renderer.resize(device, size);
        renderer
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: winit::dpi::PhysicalSize<u32>) {
        let size = winit::dpi::PhysicalSize::new(size.width.max(1), size.height.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        self._scene_texture = create_scene_texture(device, size);
        self.scene_view = self
            ._scene_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.blur_texture = create_scene_texture(device, size);
        self.blur_view = self.blur_texture.create_view(&Default::default());
        self.bloom_blur_texture = create_scene_texture(device, size);
        self.bloom_blur_view = self.bloom_blur_texture.create_view(&Default::default());
        self.bloom_texture = create_scene_texture(device, size);
        self.bloom_view = self.bloom_texture.create_view(&Default::default());
        let mip_count = luminance_mip_count(size);
        self.luminance_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("manual luminance mip chain"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.luminance_mip_views = (0..mip_count)
            .map(|mip_level| {
                self.luminance_texture
                    .create_view(&wgpu::TextureViewDescriptor {
                        base_mip_level: mip_level,
                        mip_level_count: Some(1),
                        ..Default::default()
                    })
            })
            .collect();
        self.luminance_from_scene_bind_group =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("HDR scene luminance source"),
                layout: &self.luminance_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.scene_view),
                }],
            });
        self.luminance_downsample_bind_groups = self
            .luminance_mip_views
            .windows(2)
            .enumerate()
            .map(|(index, views)| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("luminance mip {index} source")),
                    layout: &self.luminance_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[0]),
                    }],
                })
            })
            .collect();
        self.blur_bind_group = texture_bind_group(
            device,
            &self.luminance_bind_group_layout,
            "HDR blur source",
            &self.scene_view,
        );
        self.bloom_bind_group = bloom_bind_group(
            device,
            &self.bloom_bind_group_layout,
            if self.blur_enabled {
                &self.blur_view
            } else {
                &self.scene_view
            },
            &self.bloom_blur_view,
        );
        let tone_source = if self.bloom_enabled {
            &self.bloom_view
        } else if self.blur_enabled {
            &self.blur_view
        } else {
            &self.scene_view
        };
        self.tone_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HDR scene tone map source"),
            layout: &self.tone_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(tone_source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.exposure_buffer.as_entire_binding(),
                },
            ],
        });
    }

    /// The tone-map target can be larger than the HDR scene when fullscreen
    /// preserves a deliberately low internal render resolution.
    pub fn set_presentation_size(
        &mut self,
        queue: &wgpu::Queue,
        presentation_size: winit::dpi::PhysicalSize<u32>,
    ) {
        self.presentation_size = winit::dpi::PhysicalSize::new(
            presentation_size.width.max(1),
            presentation_size.height.max(1),
        );
        self.write_exposure_uniform(queue);
    }

    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    pub fn blur_enabled(&self) -> bool {
        self.blur_enabled
    }

    pub fn bloom_enabled(&self) -> bool {
        self.bloom_enabled
    }

    pub fn hdr_effect_enabled(&self) -> bool {
        self.hdr_effect_enabled
    }

    pub fn set_effects(&mut self, device: &wgpu::Device, blur_enabled: bool, bloom_enabled: bool) {
        self.blur_enabled = blur_enabled;
        self.bloom_enabled = bloom_enabled;
        self.bloom_bind_group = bloom_bind_group(
            device,
            &self.bloom_bind_group_layout,
            if blur_enabled {
                &self.blur_view
            } else {
                &self.scene_view
            },
            &self.bloom_blur_view,
        );
        let tone_source = if bloom_enabled {
            &self.bloom_view
        } else if blur_enabled {
            &self.blur_view
        } else {
            &self.scene_view
        };
        self.tone_bind_group = tone_bind_group(
            device,
            &self.tone_bind_group_layout,
            tone_source,
            &self.exposure_buffer,
        );
    }

    pub fn set_hdr_effect_enabled(&mut self, queue: &wgpu::Queue, hdr_effect_enabled: bool) {
        self.hdr_effect_enabled = hdr_effect_enabled;
        self.write_exposure_uniform(queue);
    }

    pub fn collect_completed_luminance(&mut self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut self.readback_slots {
            let Some(pending) = slot.pending.as_ref() else {
                continue;
            };
            let Ok(mapped) = pending.receiver.try_recv() else {
                continue;
            };
            slot.pending = None;
            if !mapped {
                continue;
            }
            let mapped = slot.buffer.slice(..).get_mapped_range();
            let luminance = f16_to_f32(u16::from_le_bytes([mapped[0], mapped[1]]));
            drop(mapped);
            slot.buffer.unmap();
            if luminance.is_finite() && luminance >= 0.0 {
                self.average_luminance = luminance;
            }
        }
    }

    pub fn update_exposure(&mut self, queue: &wgpu::Queue, delta_seconds: f64) {
        let delta_seconds = delta_seconds.clamp(0.0, 1.0) as f32;
        self.target_exposure = target_exposure(self.average_luminance);
        let interpolation = 1.0 - (-delta_seconds * EXPOSURE_ADAPT_SPEED).exp();
        self.exposure = (self.exposure + (self.target_exposure - self.exposure) * interpolation)
            .clamp(MINIMUM_EXPOSURE, MAXIMUM_EXPOSURE);
        self.write_exposure_uniform(queue);
    }

    fn write_exposure_uniform(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.exposure_buffer,
            0,
            bytemuck::bytes_of(&ExposureUniform {
                exposure: self.exposure,
                hdr_effect_enabled: u32::from(self.hdr_effect_enabled),
                presentation_size: [
                    self.presentation_size.width as f32,
                    self.presentation_size.height as f32,
                ],
            }),
        );
    }

    pub fn exposure_state(&self) -> ExposureState {
        ExposureState {
            exposure: self.exposure,
            target_exposure: self.target_exposure,
            average_luminance: self.average_luminance,
        }
    }

    pub fn encode_luminance(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<(&wgpu::QuerySet, u32, u32)>,
    ) {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HDR luminance extraction"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.luminance_mip_views[0],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: timestamps.map(|(query_set, begin, _)| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(begin),
                        end_of_pass_write_index: None,
                    }
                }),
                multiview_mask: None,
            });
            pass.set_pipeline(&self.luminance_from_scene_pipeline);
            pass.set_bind_group(0, &self.luminance_from_scene_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        for (index, bind_group) in self.luminance_downsample_bind_groups.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HDR luminance downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.luminance_mip_views[index + 1],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: timestamps.and_then(|(query_set, _, end)| {
                    (index + 1 == self.luminance_downsample_bind_groups.len()).then_some(
                        wgpu::RenderPassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: None,
                            end_of_pass_write_index: Some(end),
                        },
                    )
                }),
                multiview_mask: None,
            });
            pass.set_pipeline(&self.luminance_downsample_pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub fn encode_blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<(&wgpu::QuerySet, u32, u32)>,
    ) {
        if !self.blur_enabled {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HDR blur pass"),
            color_attachments: &[Some(post_attachment(&self.blur_view))],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: timestamp_writes(timestamps),
            multiview_mask: None,
        });
        pass.set_pipeline(&self.blur_pipeline);
        pass.set_bind_group(0, &self.blur_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn encode_bloom(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<(&wgpu::QuerySet, u32, u32)>,
    ) {
        if !self.bloom_enabled {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HDR bloom blur pass"),
            color_attachments: &[Some(post_attachment(&self.bloom_blur_view))],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: timestamps.map(|(query_set, begin, _)| {
                wgpu::RenderPassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(begin),
                    end_of_pass_write_index: None,
                }
            }),
            multiview_mask: None,
        });
        pass.set_pipeline(&self.bloom_blur_pipeline);
        pass.set_bind_group(0, &self.blur_bind_group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HDR bloom composite pass"),
            color_attachments: &[Some(post_attachment(&self.bloom_view))],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: timestamps.map(|(query_set, _, end)| {
                wgpu::RenderPassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(end),
                }
            }),
            multiview_mask: None,
        });
        pass.set_pipeline(&self.bloom_pipeline);
        pass.set_bind_group(0, &self.bloom_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn encode_tone_map(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        timestamps: Option<(&wgpu::QuerySet, u32, u32)>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ACES tone map pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: timestamp_writes(timestamps),
            multiview_mask: None,
        });
        pass.set_pipeline(&self.tone_pipeline);
        pass.set_bind_group(0, &self.tone_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn encode_luminance_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<usize> {
        for offset in 0..self.readback_slots.len() {
            let index = (self.next_readback_slot + offset) % self.readback_slots.len();
            if self.readback_slots[index].pending.is_none() {
                self.next_readback_slot = (index + 1) % self.readback_slots.len();
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.luminance_texture,
                        mip_level: self.luminance_mip_views.len() as u32 - 1,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &self.readback_slots[index].buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                            rows_per_image: Some(1),
                        },
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                return Some(index);
            }
        }
        None
    }

    pub fn begin_luminance_readback(&mut self, index: usize) {
        let (sender, receiver) = mpsc::channel();
        self.readback_slots[index]
            .buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result.is_ok());
            });
        self.readback_slots[index].pending = Some(PendingLuminanceReadback { receiver });
    }
}

fn target_exposure(average_luminance: f32) -> f32 {
    let luminance = average_luminance.clamp(EXPOSURE_EPSILON, 10_000.0);
    (EXPOSURE_KEY / (luminance + EXPOSURE_EPSILON)).clamp(MINIMUM_EXPOSURE, MAXIMUM_EXPOSURE)
}

fn texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(view),
        }],
    })
}

fn bloom_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene: &wgpu::TextureView,
    effect: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("HDR bloom composite sources"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(effect),
            },
        ],
    })
}

fn tone_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    source: &wgpu::TextureView,
    exposure_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("HDR tone map source"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: exposure_buffer.as_entire_binding(),
            },
        ],
    })
}

fn post_attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
        },
    }
}

fn timestamp_writes(
    timestamps: Option<(&wgpu::QuerySet, u32, u32)>,
) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
    timestamps.map(|(query_set, begin, end)| wgpu::RenderPassTimestampWrites {
        query_set,
        beginning_of_pass_write_index: Some(begin),
        end_of_pass_write_index: Some(end),
    })
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fragment_entry_point: &str,
    target_format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_scene_texture(
    device: &wgpu::Device,
    size: winit::dpi::PhysicalSize<u32>,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("HDR scene target"),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HdrRenderer::SCENE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn luminance_mip_count(size: winit::dpi::PhysicalSize<u32>) -> u32 {
    size.width.max(size.height).ilog2() + 1
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    match exponent {
        0 => sign * f32::from(fraction) * 2.0_f32.powi(-24),
        31 => {
            if fraction == 0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + f32::from(fraction) / 1024.0) * 2.0_f32.powi(i32::from(exponent) - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::{f16_to_f32, target_exposure};

    #[test]
    fn decodes_half_float_luminance_values() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0x4000), 2.0);
        assert!((f16_to_f32(0x2e66) - 0.1).abs() < 0.001);
    }

    #[test]
    fn black_space_cannot_overexpose_a_visible_planet() {
        assert_eq!(target_exposure(0.0), 4.0);
        assert!((target_exposure(0.18) - 1.0).abs() < 0.001);
    }

    #[test]
    fn tone_map_shader_supports_a_larger_presentation_target() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("hdr.wgsl"))
            .expect("HDR shader must parse before WGPU creates the pipeline");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("HDR shader must validate before WGPU creates the pipeline");
    }
}

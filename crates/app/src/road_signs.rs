//! Cached 3D sign boards and atlas text. One bounded batch, no per-frame layout.
use crate::{
    road_navigation::{Board, Kind},
    road_network_field::Frame,
};
use glam::DVec3;
use wgpu::util::DeviceExt;
const WOOD_EDGE: [f32; 3] = [0.65; 3];
const INK: [f32; 3] = [0.006; 3];
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    colour: [f32; 3],
}
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub size: [u32; 2],
    pub pixels: Vec<u8>,
    pub inspection: Option<(DVec3, DVec3)>,
    pub counts: [usize; 5],
}
impl Mesh {
    pub fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            size: [1, 1],
            pixels: vec![255; 4],
            inspection: None,
            counts: [0; 5],
        }
    }
}
pub fn bake(boards: &[Board], frame: Frame) -> Result<Mesh, String> {
    if boards.is_empty() {
        return Ok(Mesh::empty());
    }
    let ctx = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "road-sign-serif".into(),
        egui::FontData::from_static(include_bytes!(
            "../../../assets/road_signs/EBGaramond12-Bold.ttf"
        ))
        .into(),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(0, "road-sign-serif".into());
    ctx.set_fonts(fonts);
    let mut galleys = Vec::new();
    ctx.begin_pass(egui::RawInput::default());
    {
        for board in boards {
            for line in &board.lines {
                galleys.push(ctx.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(
                        line.clone(),
                        egui::FontId::proportional(64.0),
                        egui::Color32::WHITE,
                    )
                }));
            }
        }
    }
    let output = ctx.end_pass();
    let image = output
        .textures_delta
        .set
        .iter()
        .find(|(id, _)| *id == egui::TextureId::Managed(0))
        .ok_or("road sign font atlas missing")?;
    if image.1.pos.is_some() {
        return Err("road sign atlas must be complete".into());
    }
    let egui::ImageData::Color(image) = &image.1.image;
    let size = image.size.map(|v| v as u32);
    let pixels = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    let mut mesh = Mesh {
        vertices: Vec::new(),
        size,
        pixels,
        inspection: None,
        counts: [0; 5],
    };
    let mut text = galleys.iter();
    let view = std::env::var("PLANET_ROAD_SIGN_VIEW").ok();
    for board in boards {
        let first_vertex = mesh.vertices.len();
        mesh.counts[board.kind as usize] += 1;
        let normal = board.right.cross(board.up).normalize();
        let height = 0.30 + 0.62 * board.lines.len() as f64;
        let front = board.centre + normal * 0.03;
        let quad = |centre: DVec3, width: f64, height: f64| {
            [
                centre - board.right * width / 2.0 - board.up * height / 2.0,
                centre + board.right * width / 2.0 - board.up * height / 2.0,
                centre + board.right * width / 2.0 + board.up * height / 2.0,
                centre - board.right * width / 2.0 + board.up * height / 2.0,
            ]
        };
        let panel = quad(board.centre, board.width, height);
        let back = quad(board.centre - normal * 0.06, board.width, height);
        wood(
            &mut mesh.vertices,
            frame,
            panel,
            WOOD_EDGE,
            [board.width, height],
        );
        let mut back_face = back;
        back_face.reverse();
        wood(
            &mut mesh.vertices,
            frame,
            back_face,
            WOOD_EDGE,
            [board.width, height],
        );
        for i in 0..4 {
            let j = (i + 1) % 4;
            wood(
                &mut mesh.vertices,
                frame,
                [panel[i], back[i], back[j], panel[j]],
                [0.55; 3],
                [0.06, panel[i].distance(panel[j])],
            );
        }
        wood(
            &mut mesh.vertices,
            frame,
            quad(front, board.width - 0.08, height - 0.08),
            [1.0; 3],
            [board.width - 0.08, height - 0.08],
        );
        if let Some(anchor) = board.support {
            let foot = board.centre - board.up * 4.0;
            beam(&mut mesh.vertices, frame, anchor, foot, 0.11);
            beam(&mut mesh.vertices, frame, foot, board.centre, 0.08);
        } else if board.kind != Kind::Branch {
            // Ceiling-mounted boards keep the vehicle envelope free.
            for side in [-1.0, 1.0] {
                let at = board.centre + board.right * (side * board.width * 0.35);
                beam(&mut mesh.vertices, frame, at, at + board.up * 1.4, 0.035);
            }
        }
        for (line, _) in board.lines.iter().enumerate() {
            let galley = text.next().unwrap();
            let scale = ((board.width - 0.40) / galley.rect.width() as f64)
                .min(0.47 / galley.rect.height() as f64);
            let origin = front + normal * 0.015
                - board.right * (galley.rect.width() as f64 * scale / 2.0)
                + board.up * (height / 2.0 - 0.13 - line as f64 * 0.62);
            for row in &galley.rows {
                for triangle in row.row.visuals.mesh.indices.chunks_exact(3) {
                    for index in [triangle[0], triangle[2], triangle[1]] {
                        let v = &row.row.visuals.mesh.vertices[index as usize];
                        let p = origin + board.right * ((v.pos.x + row.pos.x) as f64 * scale)
                            - board.up * ((v.pos.y + row.pos.y) as f64 * scale);
                        mesh.vertices.push(vertex(
                            frame,
                            p,
                            normal,
                            [v.uv.x / size[0] as f32, v.uv.y / size[1] as f32],
                            INK,
                        ));
                    }
                }
            }
        }
        if board.kind == Kind::Tunnel {
            // Backlit tunnel information is independent of sun and clouds.
            for v in &mut mesh.vertices[first_vertex..] {
                v.colour[2] = -v.colour[2].abs();
            }
        }
        let wanted = match view.as_deref() {
            Some("tunnel") => board.kind == Kind::Tunnel,
            Some("village") => board.kind == Kind::Village,
            Some("distance") => board.kind == Kind::Distance,
            Some("junction") => board.kind == Kind::Junction,
            Some("branch") => board.kind == Kind::Branch,
            _ => true,
        };
        if wanted {
            mesh.inspection
                .get_or_insert((board.centre + normal * 18.0 - board.up * 1.5, board.centre));
        }
        if mesh.vertices.len() > 100_000 {
            return Err("road sign vertex budget exceeded".into());
        }
    }
    tracing::info!(kinds=?mesh.counts,vertices=mesh.vertices.len(),"road sign meshes built (junction/distance/village/tunnel/branch)");
    Ok(mesh)
}
fn vertex(frame: Frame, p: DVec3, n: DVec3, uv: [f32; 2], colour: [f32; 3]) -> Vertex {
    Vertex {
        position: frame.local(p).to_array().map(|v| v as f32),
        normal: [
            n.dot(frame.across) as f32,
            n.dot(frame.forward) as f32,
            n.dot(frame.up) as f32,
        ],
        uv,
        colour,
    }
}
fn solid(out: &mut Vec<Vertex>, f: Frame, p: [DVec3; 4], c: [f32; 3]) {
    let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(vertex(f, p[i], n, [-1.0; 2], c));
    }
}
// Negative U distinguishes wood from both glyph UVs and solid hardware.
// Local board coordinates keep horizontal grain attached to each sign.
fn wood(out: &mut Vec<Vertex>, f: Frame, p: [DVec3; 4], c: [f32; 3], size: [f64; 2]) {
    let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
    let u = size[0] as f32 / 2.5;
    let v = size[1] as f32 / 1.25;
    let uv = [[-2.0, v], [-2.0 - u, v], [-2.0 - u, 0.0], [-2.0, 0.0]];
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(vertex(f, p[i], n, uv[i], c));
    }
}

fn wood_mips() -> Vec<u8> {
    let decoded = image::load_from_memory(include_bytes!("../../../assets/road_signs/oak.png"))
        .expect("embedded oak texture must decode")
        .to_rgba8();
    let mut level =
        image::imageops::resize(&decoded, 512, 512, image::imageops::FilterType::Triangle);
    let mut pixels = Vec::new();
    loop {
        pixels.extend_from_slice(level.as_raw());
        let size = level.width();
        if size == 1 {
            break;
        }
        level = image::imageops::resize(
            &level,
            size / 2,
            size / 2,
            image::imageops::FilterType::Triangle,
        );
    }
    pixels
}

fn wood_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("cached oak sign texture"),
            size: wgpu::Extent3d {
                width: 512,
                height: 512,
                depth_or_array_layers: 1,
            },
            mip_level_count: 10,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &wood_mips(),
    );
    texture.create_view(&Default::default())
}

fn beam(out: &mut Vec<Vertex>, f: Frame, a: DVec3, b: DVec3, r: f64) {
    let axis = (b - a).normalize();
    let reference = if axis.z.abs() < 0.9 {
        DVec3::Z
    } else {
        DVec3::X
    };
    let x = axis.cross(reference).normalize() * r;
    let y = axis.cross(x).normalize() * r;
    let ends = [a, b].map(|p| [p - x - y, p + x - y, p + x + y, p - x + y]);
    for i in 0..4 {
        let j = (i + 1) % 4;
        solid(
            out,
            f,
            [ends[0][i], ends[0][j], ends[1][j], ends[1][i]],
            [0.20, 0.22, 0.23],
        );
    }
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    texture: wgpu::BindGroup,
    wood: wgpu::TextureView,
    vertices: wgpu::Buffer,
    count: u32,
    pub inspection: Option<(DVec3, DVec3)>,
}
impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("road sign atlas layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("road signs"),
            bind_group_layouts: &[Some(camera), Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("road_signs.wgsl"));
        let pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
            label:Some("cached road signs"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState{module:&shader,entry_point:Some("vs_main"),compilation_options:Default::default(),buffers:&[
                wgpu::VertexBufferLayout{array_stride:size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x3]},
                wgpu::VertexBufferLayout{array_stride:64,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4]}]},
            fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("fs_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format,blend:Some(wgpu::BlendState::ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),
            primitive:wgpu::PrimitiveState{cull_mode:Some(wgpu::Face::Back),..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState{format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::GreaterEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:Default::default(),multiview_mask:None,cache:crate::pipeline_cache::get()});
        let wood = wood_texture(device, queue);
        let texture = create_atlas(device, &layout, [1, 1], &wood);
        Self {
            pipeline,
            layout,
            texture: texture.1,
            wood,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("empty signs"),
                size: 44,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            }),
            count: 0,
            inspection: None,
        }
    }
    pub fn clear(&mut self) {
        self.count = 0;
        self.inspection = None;
    }
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: Mesh) {
        self.count = mesh.vertices.len() as u32;
        self.inspection = mesh.inspection;
        if mesh.vertices.is_empty() {
            return;
        }
        let (texture, bind) = create_atlas(device, &self.layout, mesh.size, &self.wood);
        self.texture = bind;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &mesh.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mesh.size[0] * 4),
                rows_per_image: Some(mesh.size[1]),
            },
            wgpu::Extent3d {
                width: mesh.size[0],
                height: mesh.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cached signs and glyphs"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    }
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera: &'a wgpu::BindGroup,
        instance: &'a wgpu::Buffer,
    ) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, &self.texture, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, instance.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}
fn create_atlas(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    size: [u32; 2],
    wood: &wgpu::TextureView,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("road sign glyph atlas"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let wood_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("mipmapped repeating wood"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("road sign atlas"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(wood),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&wood_sampler),
            },
        ],
    });
    (texture, bind)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "asset-backed photographed junction approach and branch signs, no GPU"]
    fn photographed_junction_has_named_branch_signs() {
        let eye = DVec3::new(1835167.0957529722, 2115693.4193919445, 2869280.8813291406);
        let (mut worker, _) =
            crate::road_network_stream::Worker::start(crate::find_default_outmap().unwrap(), eye)
                .unwrap();
        worker.request(eye, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        let mut frame = None;
        loop {
            if let Some((_, snapshot)) = worker.take() {
                frame = Some(snapshot.unwrap().field.frame);
            }
            if let Some((_, mesh)) = worker.take_signs() {
                let frame = frame.unwrap();
                let mut boards = Vec::new();
                for quad in mesh.vertices.chunks_exact(6) {
                    if quad[0].colour == WOOD_EDGE && quad[0].normal[2].is_finite() {
                        let local = quad
                            .iter()
                            .map(|v| DVec3::from_array(v.position.map(f64::from)))
                            .sum::<DVec3>()
                            / 6.0;
                        boards.push(frame.world(local));
                    }
                }
                assert_eq!(mesh.counts[0], 3, "one approach sign per incoming arm");
                assert_eq!(
                    mesh.counts[4], 6,
                    "double-sided name/distance finger on each departure branch"
                );
                assert!(boards.iter().filter(|p| p.distance(eye) < 70.0).count() >= 9);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    #[test]
    fn sign_shader_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("road_signs.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
    #[test]
    fn atlas_text_has_finite_world_geometry_and_real_glyph_alpha() {
        let p = DVec3::Z * 4_001_000.0;
        let f = Frame::new(p.normalize(), 4e6).unwrap();
        let board = Board {
            kind: Kind::Distance,
            centre: p,
            right: DVec3::X,
            up: DVec3::Z,
            lines: vec!["OAKFORD 1234 KM".into()],
            width: 6.0,
            village: false,
            support: None,
        };
        let m = bake(&[board], f).unwrap();
        assert!(m.vertices.len() > 100);
        for triangle in m.vertices.chunks_exact(3).filter(|t| t[0].uv[0] >= 0.0) {
            let [a, b, c] = triangle.try_into().unwrap();
            let a = DVec3::from_array(a.position.map(f64::from));
            let b = DVec3::from_array(b.position.map(f64::from));
            let c = DVec3::from_array(c.position.map(f64::from));
            assert!(
                (b - a).cross(c - a).dot(-DVec3::Y) > 0.0,
                "glyphs must face the driver, not the back of the board"
            );
        }
        assert!(
            m.vertices
                .iter()
                .all(|v| v.position.iter().all(|v| v.is_finite()))
        );
        assert!(m.pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
    }
}

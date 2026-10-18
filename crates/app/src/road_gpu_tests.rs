//! Execute the production terrain-cut lookup, not a Rust copy of its shader.
use glam::DVec3;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires Vulkan; run explicitly when the road cut field changes"]
fn gpu_road_cuts_match_cpu_at_bends_banks_ends_and_underpasses() {
    let mut plan = crate::road::Viaduct::survey(|d| {
        let x = d.dot(crate::road::across()) * crate::planet::planet_radius_meters();
        let y = d.dot(crate::road::forward()) * crate::planet::planet_radius_meters();
        Some(1900.0 + 200.0 * (-(x / 350.0).powi(2) - ((y - 8000.0) / 400.0).powi(2)).exp())
    })
    .unwrap();
    assert!(plan.route.points.iter().any(|p| p.x.abs() > 300.0));
    // Force a known bore into the fixture so parity does not depend on the
    // optimiser choosing a tunnel in this otherwise gently rolling survey.
    std::sync::Arc::get_mut(&mut plan.cuts)
        .unwrap()
        .tunnel_segments[1000..1020]
        .fill(true);

    let mut cases = Vec::new();
    for distance in [0.0, 100.0, 8000.0, 10000.0, 29000.0] {
        for offset in [0.0, 4.0, 12.0, 30.0, 1000.0] {
            let p = crate::road::world(plan.route.at_distance(distance))
                + crate::road::across() * offset;
            let direction = p.normalize().as_vec3();
            let height = (p.length() - crate::planet::planet_radius_meters()) as f32;
            for extra in [-20.0, 40.0] {
                cases.push([direction.x, direction.y, direction.z, height + extra]);
            }
        }
    }
    for p in [
        DVec3::new(0.0, -150.0, 1900.0),
        DVec3::new(0.0, 30200.0, 1900.0),
    ] {
        let p = crate::road::world(p);
        let direction = p.normalize().as_vec3();
        cases.push([
            direction.x,
            direction.y,
            direction.z,
            (p.length() - crate::planet::planet_radius_meters()) as f32 + 40.0,
        ]);
    }
    for y in [9985.0, 10005.0, 10100.0, 10195.0, 10205.0] {
        let centre = plan.route.at_y(y);
        for (offset, z) in [(0.0, 2.0), (0.0, 20.0), (6.0, 2.0), (8.0, 2.0), (0.0, -8.0)] {
            let p = crate::road::world(centre + DVec3::new(offset, 0.0, z));
            let d = p.normalize().as_vec3();
            cases.push([
                d.x,
                d.y,
                d.z,
                (p.length() - crate::planet::planet_radius_meters()) as f32,
            ]);
        }
    }
    let heights = execute_cut_samples(&plan.cuts.gpu_data(), &cases);
    let mut maximum = 0.0_f64;
    for (c, h) in cases.iter().zip(&heights) {
        let expected = plan.cuts.height(
            DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64),
            c[3] as f64,
        );
        let error = (h[0] as f64 - expected).abs();
        maximum = maximum.max(error);
        assert!(
            error < 0.75,
            "CPU {expected}, GPU {h:?}, error {error}, case {c:?}"
        );
        assert!(h[0] <= c[3] + 0.001);
        let void = plan.cuts.tunnel_void(
            DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64),
            c[3] as f64,
        );
        assert_eq!(
            h[2] > 0.5,
            void > -0.25,
            "lining ownership mismatch at {c:?}"
        );
        assert!(
            (f64::from(h[1]) - void).abs() < 0.75,
            "bore mismatch CPU {void}, GPU {}, case {c:?}",
            h[1]
        );
    }
    eprintln!(
        "{} production cut samples, max CPU/GPU difference {maximum:.4} m",
        cases.len()
    );
}

fn execute_cut_samples(profile_data: &[[f32; 4]], cases: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let source = format!(
        "const ROAD_CUTS_ENABLED: bool = true;\n{}\n{}",
        include_str!("road_cuts.wgsl").replace("@group(2) @binding(21)", "@group(0) @binding(1)"),
        "@group(0) @binding(0) var<storage,read_write> results: array<vec4<f32>>;\n@group(0) @binding(2) var<storage,read> cases: array<vec4<f32>>;\n@compute @workgroup_size(1) fn test_cut(@builtin(global_invocation_id) id: vec3<u32>) { let c=cases[id.x]; results[id.x]=vec4<f32>(road_terrain_cut(c.xyz,c.w).height,road_tunnel_void(c.xyz,c.w),select(0.0,1.0,road_tunnel_lining_owns(c.xyz,c.w)),0.0); }"
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("Vulkan adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production road cut parity"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("road cut parity"),
        layout: None,
        module: &shader,
        entry_point: Some("test_cut"),
        compilation_options: Default::default(),
        cache: None,
    });
    let profile = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("road profile"),
        contents: bytemuck::cast_slice(profile_data),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cut samples"),
        contents: bytemuck::cast_slice(cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (cases.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cut results"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cut readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cut parity"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: profile.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: input.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(cases.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit(Some(encoder.finish()));
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap()
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let data = readback.slice(..).get_mapped_range();
    bytemuck::cast_slice::<u8, [f32; 4]>(&data).to_vec()
}

#[test]
#[ignore = "requires Vulkan; renders the production far-road pipeline offscreen"]
fn gpu_far_road_retains_subpixel_coverage_without_black_specks() {
    use bytemuck::Zeroable;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("road AA camera"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let mut camera = crate::planet::CameraUniform::zeroed();
    // 50 metres per pixel at 50 km: the real 10m-wide deck occupies 0.2px.
    camera.projection_matrix = [
        [31.25, 0.0, 0.0, 0.0],
        [0.0, 31.25, 0.0, 0.0],
        [0.0, 0.0, 0.0, -1.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    camera.camera_forward = [0.0, 0.0, -1.0, 0.0];
    camera.camera_right = [1.0, 0.0, 0.0, 0.0];
    camera.camera_up = [0.0, 1.0, 0.0, 0.0];
    camera.sun_direction = [0.0, 0.0, 1.0, 0.0];
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("road AA camera"),
        contents: bytemuck::bytes_of(&camera),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("road AA camera"),
        layout: &camera_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let pipeline = crate::road_render::create_pipeline(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        &camera_layout,
        true,
    );
    let frame = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("road AA frame"),
        size: 64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("road AA offscreen"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let colour = texture(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let view = colour.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("road AA pixels"),
        size: 64 * 64 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let render = |angle: f64, phase: f32, clear_depth: f32| {
        let tangent = DVec3::new(angle.sin(), angle.cos(), 0.0);
        let mut vertices = Vec::new();
        for (a, b) in [(-5000.0, 0.0), (0.0, 5000.0)] {
            crate::road::far_segment(
                &mut vertices,
                tangent * a,
                tangent * b,
                tangent * crate::road::HALF_WIDTH,
                tangent * crate::road::HALF_WIDTH,
            );
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("real far-road segment"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let data = [
            [phase * 50.0, 0.0, -50000.0, 64.0],
            [1.0, 0.0, 0.0, 64.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        queue.write_buffer(&frame, 0, bytemuck::cast_slice(&data));
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("far-road coverage regression"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_depth),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.set_vertex_buffer(1, frame.slice(..));
            pass.draw(0..vertices.len() as u32, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &colour,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| sender.send(r).unwrap());
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        receiver.recv().unwrap().unwrap();
        let pixels = readback.slice(..).get_mapped_range().to_vec();
        readback.unmap();
        pixels
    };
    for angle in [0.0, 0.35, std::f64::consts::FRAC_PI_4] {
        let mut energies = Vec::new();
        let mut peak = 0.0_f64;
        for i in 0..8 {
            let pixels = render(angle, i as f32 / 8.0, 0.0);
            if let Some(dir) = std::env::var_os("PLANET_ROAD_AA_TEST_CAPTURES") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let mut ppm = b"P6\n64 64\n255\n".to_vec();
                for pixel in pixels.chunks_exact(4) {
                    ppm.extend_from_slice(&pixel[..3]);
                }
                std::fs::write(dir.join(format!("road-{angle:.3}-{i}.ppm")), ppm).unwrap();
            }
            let mut energy = 0.0;
            for y in 8..56 {
                for x in 0..64 {
                    let dark = 1.0 - f64::from(pixels[(y * 64 + x) * 4]) / 255.0;
                    energy += dark;
                    peak = peak.max(dark);
                }
            }
            energies.push(energy / 48.0);
        }
        eprintln!("angle {angle:.3}, coverage energies {energies:?}, peak darkness {peak:.3}");
        let expected = 0.2 * (1.0 - 0.045 * (1.9 + 0.24)) / angle.cos();
        assert!(
            energies.iter().all(|v| (v / expected - 1.0).abs() < 0.08),
            "lost/overweighted subpixel road: {energies:?}, expected {expected}"
        );
        assert!(peak < 0.3, "subpixel road became a full dark pixel: {peak}");
    }
    let hidden = render(0.0, 0.5, 0.00004);
    assert!(
        hidden.chunks_exact(4).all(|p| p[0] == 255),
        "road painted over closer depth"
    );
}

#[test]
#[ignore = "requires Vulkan; production arbitrary-network excavation lookup"]
fn gpu_network_field_matches_cpu_for_negative_cells_bends_and_branching_bores() {
    use crate::road_network_field::{Field, Frame, Segment};
    for up in [DVec3::X, DVec3::new(1.0, 2.0, 3.0).normalize(), DVec3::Z] {
        let frame = Frame::new(up, crate::planet::planet_radius_meters()).unwrap();
        let segment = |a: [f64; 3], b: [f64; 3], tunnel| Segment {
            a: a.into(),
            b: b.into(),
            tunnel,
            reach: 48.0,
        };
        let field = Field::build(
            frame,
            vec![
                segment([-210.0, -75.0, 1200.0], [210.0, 75.0, 1220.0], false),
                segment([-120.0, 160.0, 1180.0], [0.0, 160.0, 1190.0], true),
                segment([0.0, 160.0, 1190.0], [120.0, 300.0, 1200.0], true),
                segment([0.0, 160.0, 1190.0], [180.0, 100.0, 1200.0], true),
            ],
        )
        .unwrap();
        let mut cases = Vec::new();
        for x in -24..=24 {
            for y in -12..=32 {
                for z in [1184.0, 1192.0, 1204.0, 1240.0] {
                    // Avoid exact cell and surface boundaries: radius-sized f32
                    // direction rounding otherwise changes the queried column.
                    let p = frame.world(DVec3::new(
                        x as f64 * 10.0 + 0.31,
                        y as f64 * 10.0 + 0.41,
                        z,
                    ));
                    let d = p.normalize().as_vec3();
                    cases.push([d.x, d.y, d.z, (p.length() - frame.radius) as f32]);
                }
            }
        }
        let results = execute_cut_samples(field.gpu_data(), &cases);
        let mut maximum = 0.0_f64;
        for (c, result) in cases.iter().zip(results) {
            let d = DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64);
            let height = field.height(d, c[3] as f64);
            let void = field.tunnel_void(d, c[3] as f64);
            maximum = maximum.max((height - result[0] as f64).abs());
            assert!(
                (height - result[0] as f64).abs() < 0.8,
                "cut CPU {height} GPU {result:?} case {c:?}"
            );
            // Outside indexed cells both use the negative sentinel; elsewhere
            // compare signs away from the sub-metre f32 precision boundary.
            if void.abs() > 0.8 && result[1].abs() > 0.8 {
                assert_eq!(
                    void > 0.0,
                    result[1] > 0.0,
                    "bore CPU {void} GPU {result:?} case {c:?}"
                );
            }
        }
        eprintln!(
            "network up={up:?}: {} GPU samples, max cut error {maximum:.4}m",
            cases.len()
        );
    }
}

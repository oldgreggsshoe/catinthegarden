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
        assert_eq!(
            h[3] > 0.5,
            plan.cuts.clears_tree(
                DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64),
                c[3] as f64
            ),
            "pilot tree clearance at {c:?}"
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
    execute_cut_samples_with_ownership(profile_data, cases, "road_tunnel_lining_owns(c.xyz,c.w)")
}

fn execute_cut_samples_with_ownership(
    profile_data: &[[f32; 4]],
    cases: &[[f32; 4]],
    ownership: &str,
) -> Vec<[f32; 4]> {
    let source = format!(
        "const ROAD_CUTS_ENABLED: bool = true;\n{}\n{}",
        include_str!("road_cuts.wgsl").replace("@group(2) @binding(21)", "@group(0) @binding(1)"),
        "@group(0) @binding(0) var<storage,read_write> results: array<vec4<f32>>;\n@group(0) @binding(2) var<storage,read> cases: array<vec4<f32>>;\n@compute @workgroup_size(1) fn test_cut(@builtin(global_invocation_id) id: vec3<u32>) { let c=cases[id.x]; results[id.x]=vec4<f32>(road_terrain_cut(c.xyz,c.w).height,road_tunnel_void(c.xyz,c.w),select(0.0,1.0,road_tunnel_lining_owns(c.xyz,c.w)),select(0.0,1.0,road_clears_tree(c.xyz,c.w))); }"
    );
    let source = source.replace("road_tunnel_lining_owns(c.xyz,c.w)", ownership);
    execute_cut_shader(profile_data, cases, source)
}

fn execute_cut_shader(
    profile_data: &[[f32; 4]],
    cases: &[[f32; 4]],
    source: String,
) -> Vec<[f32; 4]> {
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
        size: 80,
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
    let lighting =
        crate::road_lighting::binding(&device, &[[0.0; 4]; crate::road_lighting::MAX_LIGHTS + 1]);
    let render = |angle: f64, phase: f32, clear_depth: f32, camera_plane: bool| {
        let tangent = DVec3::new(angle.sin(), angle.cos(), 0.0);
        let mut vertices = Vec::new();
        let end = if camera_plane { 10.0 } else { 5000.0 };
        for (a, b) in [(-end, 0.0), (0.0, end)] {
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
        let data = if camera_plane {
            // A physical road 5km BELOW the eye cannot touch the view, even
            // when its cross-section straddles the eye plane in depth.
            [
                [0.0, -5000.0, -phase, 64.0],
                [1.0, 0.0, 0.0, 64.0],
                [0.0, 0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0; 4],
            ]
        } else {
            [
                [phase * 50.0, 0.0, -50000.0, 64.0],
                [1.0, 0.0, 0.0, 64.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0; 4],
            ]
        };
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
            pass.set_bind_group(1, &lighting, &[]);
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
            let pixels = render(angle, i as f32 / 8.0, 0.0, false);
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
    let hidden = render(0.0, 0.5, 0.00004, false);
    assert!(
        hidden.chunks_exact(4).all(|p| p[0] == 255),
        "road painted over closer depth"
    );
    for phase in [0.25, 1.0, 2.0, 4.0] {
        let hidden = render(std::f64::consts::FRAC_PI_2, phase, 0.0, true);
        assert!(
            hidden.chunks_exact(4).all(|p| p[0] == 255),
            "offscreen far road crossed the camera plane into the frame at depth {phase}"
        );
    }
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
            floor_depth: 6.0,
            junction_a: false,
            junction_b: false,
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

#[test]
#[ignore = "requires Vulkan; production forest road exclusion mask"]
fn gpu_roads_clear_tree_crowns_and_banks_but_retain_forest_above_bores() {
    use crate::road_network_field::{Field, Frame, Segment};
    for up in [DVec3::X, DVec3::new(1.0, 2.0, 3.0).normalize()] {
        let frame = Frame::new(up, crate::planet::planet_radius_meters()).unwrap();
        let field = Field::build(
            frame,
            vec![
                Segment {
                    a: DVec3::new(-100.0, 0.0, 100.0),
                    b: DVec3::new(100.0, 0.0, 100.0),
                    tunnel: false,
                    reach: 48.0,
                    floor_depth: 6.0,
                    junction_a: false,
                    junction_b: false,
                },
                Segment {
                    a: DVec3::new(-100.0, 200.0, 100.0),
                    b: DVec3::new(100.0, 200.0, 100.0),
                    tunnel: true,
                    reach: 48.0,
                    floor_depth: 6.0,
                    junction_a: false,
                    junction_b: false,
                },
            ],
        )
        .unwrap();
        let fixture = [
            (0.0, 90.0, true),
            (20.0, 90.0, true),
            (30.0, 90.0, false),
            (35.0, 130.0, true),
            (80.0, 130.0, false),
            (200.0, 130.0, false),
        ];
        let cases: Vec<_> = fixture
            .iter()
            .map(|&(y, h, _)| {
                let p = frame.world(DVec3::new(0.0, y, h));
                let d = p.normalize().as_vec3();
                [d.x, d.y, d.z, (p.length() - frame.radius) as f32]
            })
            .collect();
        for ((_, _, expected), result) in fixture
            .iter()
            .zip(execute_cut_samples(field.gpu_data(), &cases))
        {
            assert_eq!(result[3] > 0.5, *expected, "up={up:?}, result={result:?}");
        }
    }
}

#[test]
#[ignore = "requires Vulkan; shallow village apron raster ownership"]
fn gpu_shallow_approaches_remove_terrain_intruding_through_the_deck() {
    use crate::road_network_field::{Field, Frame, Segment};
    for up in [DVec3::X, DVec3::new(1.0, 2.0, 3.0).normalize()] {
        let frame = Frame::new(up, crate::planet::planet_radius_meters()).unwrap();
        let mut field = Field::build(
            frame,
            vec![Segment {
                a: DVec3::new(-100.0, 0.0, 100.0),
                b: DVec3::new(100.0, 0.0, 100.0),
                tunnel: false,
                reach: 64.0,
                floor_depth: 0.15,
                junction_a: false,
                junction_b: false,
            }],
        )
        .unwrap();
        field.set_geometry_window(glam::DVec2::ZERO, 1000.0);
        let fixtures = [
            (0.0, 100.1, true),
            (4.5, 100.1, true),
            (0.0, 99.6, true),
            (8.0, 100.1, false),
            (15.0, 125.0, false), // displaced terrain alone owns open banks
            (0.0, 98.0, false),
        ];
        let cases: Vec<_> = fixtures
            .iter()
            .map(|&(y, h, _)| {
                let p = frame.world(DVec3::new(0.0, y, h));
                let d = p.normalize().as_vec3();
                [d.x, d.y, d.z, (p.length() - frame.radius) as f32]
            })
            .collect();
        let result = execute_cut_samples_with_ownership(
            field.gpu_data(),
            &cases,
            "road_network_open_cut_owns(c.xyz,c.w)",
        );
        for ((_, _, expected), r) in fixtures.iter().zip(result) {
            assert_eq!(
                r[2] > 0.5,
                *expected,
                "shallow approach up={up:?} sample={r:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires Vulkan; real raster height for bore clipping, including skirts"]
fn gpu_bore_clipping_uses_drawn_height_not_the_unskirted_seabed() {
    use crate::road_network_field::{Field, Frame, Segment};
    let radius = 4e6;
    let eye = -18.0;
    let field = Field::build(
        Frame::new(DVec3::Z, radius).unwrap(),
        vec![Segment {
            a: DVec3::new(-2000.0, 0.0, -20.0),
            b: DVec3::new(2000.0, 0.0, -20.0),
            tunnel: true,
            reach: 64.0,
            floor_depth: 6.0,
            junction_a: false,
            junction_b: false,
        }],
    )
    .unwrap();
    let fixtures = [
        ([100.0, 0.0, 3.0], true),
        ([1000.0, 0.0, 3.0], true),
        ([100.0, 0.0, 9.0], false),
        ([100.0, 0.0, -12.0], false),
        ([100.0, 12.0, 3.0], false),
    ];
    let cases: Vec<[f32; 4]> = fixtures
        .iter()
        .map(|(v, _)| {
            let p = DVec3::from_array(*v) + DVec3::Z * (radius + eye);
            [
                v[0] as f32,
                v[1] as f32,
                v[2] as f32,
                (p.length() - radius) as f32,
            ]
        })
        .collect();
    let heights = execute_cut_samples_with_ownership(
        field.gpu_data(),
        &cases,
        "abs(road_raster_fragment_height(c.xyz,vec3<f32>(0.0,0.0,1.0),-18.0,4000000.0)-c.w)<0.005",
    );
    assert!(
        heights.iter().all(|r| r[2] > 0.5),
        "stable raster heights must match f64 reference"
    );
    let result = execute_cut_samples_with_ownership(
        field.gpu_data(),
        &cases,
        "road_tunnel_void(normalize(vec3<f32>(c.xy,3999982.0+c.z)),road_raster_fragment_height(c.xyz,vec3<f32>(0.0,0.0,1.0),-18.0,4000000.0))>0.0",
    );
    for ((_, expected), r) in fixtures.iter().zip(result) {
        assert_eq!(r[2] > 0.5, *expected);
    }
    // A skirt can hang inside the bore while its authoring seabed is outside.
    assert!(field.tunnel_void(DVec3::Z, 4.0) < 0.0);
    assert!(field.tunnel_void(DVec3::Z, -15.0) > 0.0);
}

#[test]
#[ignore = "requires Vulkan; circular village court cut, ownership and trees"]
fn gpu_parking_courts_match_cpu_without_zero_length_road_math() {
    use crate::road_network_field::{Field, Frame};
    for up in [DVec3::X, DVec3::new(1.0, 2.0, 3.0).normalize()] {
        let f = Frame::new(up, 4e6).unwrap();
        let mut field =
            Field::build(f, vec![crate::road_parking::segment(DVec3::Z * 1000.0)]).unwrap();
        field.set_geometry_window(glam::DVec2::ZERO, 1000.0);
        let mut cases = Vec::new();
        let mut expected = Vec::new();
        for angle in 0..16 {
            for radius in [0.0, 5.0, 19.5, 21.0, 25.5, 30.0, 39.5, 49.0] {
                let theta = angle as f64 * std::f64::consts::TAU / 16.0;
                let p = DVec3::new(theta.cos() * radius, theta.sin() * radius, 1005.0);
                let world = f.world(p);
                let d = world.normalize();
                let h = world.length() - f.radius;
                let v = d.as_vec3();
                cases.push([v.x, v.y, v.z, h as f32]);
                expected.push((field.height(d, h), field.clears_tree(d, h), radius < 19.8));
            }
        }
        let paved = execute_cut_samples_with_ownership(
            field.gpu_data(),
            &cases,
            "road_terrain_cut(c.xyz,c.w).paved",
        );
        for (r, (_, _, owns)) in paved.iter().zip(&expected) {
            assert_eq!(
                r[2] > 0.5,
                *owns,
                "cached soil banks must yield only to the actual pavement"
            );
        }
        let got = execute_cut_samples_with_ownership(
            field.gpu_data(),
            &cases,
            "road_network_open_cut_owns(c.xyz,c.w)",
        );
        for (r, (height, trees, owns)) in got.iter().zip(expected) {
            assert!(r.iter().all(|v| v.is_finite()), "{r:?}");
            assert!(
                // Same sub-metre f32 radius/direction allowance as the network
                // parity test above; the old point-as-road path misses by 5m.
                (r[0] as f64 - height).abs() < 0.8,
                "GPU {r:?}, CPU {height}"
            );
            assert_eq!(r[3] > 0.5, trees, "tree clearance");
            if owns {
                assert!(r[2] > 0.5, "pavement owns its pixels: {r:?}");
            }
        }
    }
}

#[test]
#[ignore = "requires Vulkan; exact excavation fast-path comparison"]
fn gpu_excavation_fast_paths_preserve_height_normals_flags_and_ownership() {
    use crate::road_network_field::{Field, Frame, Segment};
    let shader = include_str!("road_cuts.wgsl");
    assert!(shader.contains("if (list.z & 2u)==0u { return result; }"));
    assert!(shader.contains("if result.height==uncut { return result; }"));
    assert!(shader.contains("if (list.z & 2u)==0u { return false; }"));
    // Reference retains the old two-pass full candidate scan. Use the same
    // arithmetic, only removing the new shortcuts, and compare GPU bit patterns.
    let start = shader.find("fn road_network_terrain_cut(").unwrap();
    let end = shader.find("fn road_network_tunnel_void(").unwrap();
    let reference_cut = shader[start..end]
        .replace("road_network_terrain_cut", "reference_cut")
        .replace("if (list.z & 2u)==0u { return result; }", "")
        .replace("if result.height==uncut { return result; }", "");
    let start = shader.find("fn road_network_open_cut_owns(").unwrap();
    let end = start + shader[start..].find("\n}\n").unwrap() + 3;
    let reference_owns = shader[start..end]
        .replace("road_network_open_cut_owns", "reference_owns")
        .replace("if (list.z & 2u)==0u { return false; }", "");
    let source = format!(
        "const ROAD_CUTS_ENABLED: bool = true;\n{}\n{reference_cut}\n{reference_owns}\n{}",
        shader.replace("@group(2) @binding(21)", "@group(0) @binding(1)"),
        r#"
        @group(0) @binding(0) var<storage,read_write> results: array<vec4<f32>>;
        @group(0) @binding(2) var<storage,read> cases: array<vec4<f32>>;
        @compute @workgroup_size(1) fn test_cut(@builtin(global_invocation_id) id: vec3<u32>) {
            let c=cases[id.x];
            let fast=road_network_terrain_cut(c.xyz,c.w);
            let slow=reference_cut(c.xyz,c.w);
            let same=all(bitcast<vec4<u32>>(vec4<f32>(fast.height,fast.normal))
                ==bitcast<vec4<u32>>(vec4<f32>(slow.height,slow.normal)));
            results[id.x]=vec4<f32>(select(0.0,1.0,same),
                select(0.0,1.0,fast.lowered==slow.lowered && fast.paved==slow.paved),
                select(0.0,1.0,road_network_open_cut_owns(c.xyz,c.w)==reference_owns(c.xyz,c.w)),
                select(0.0,1.0,fast.lowered));
        }"#
    );
    for up in [DVec3::Z, DVec3::new(1.0, 2.0, 3.0).normalize()] {
        let frame = Frame::new(up, crate::planet::planet_radius_meters()).unwrap();
        let segment = |a: [f64; 3], b: [f64; 3], tunnel| Segment {
            a: a.into(),
            b: b.into(),
            tunnel,
            reach: 48.0,
            floor_depth: 6.0,
            junction_a: false,
            junction_b: false,
        };
        let mut field = Field::build(
            frame,
            vec![
                segment([-240.0, -120.0, 100.0], [240.0, 120.0, 120.0], false),
                segment([-240.0, 160.0, 80.0], [240.0, 160.0, 100.0], true),
                segment([-80.0, 160.0, 90.0], [120.0, 300.0, 100.0], true),
                crate::road_parking::segment(DVec3::new(160.0, -160.0, 110.0)),
            ],
        )
        .unwrap();
        field.set_geometry_window(glam::DVec2::ZERO, 1000.0);
        let mut cases = Vec::new();
        for x in -32..=32 {
            for y in -32..=40 {
                for z in [50.0, 100.0, 130.0, 200.0] {
                    let world = frame.world(DVec3::new(x as f64 * 8.0, y as f64 * 8.0, z));
                    let d = world.normalize().as_vec3();
                    cases.push([d.x, d.y, d.z, (world.length() - frame.radius) as f32]);
                }
            }
        }
        let results = execute_cut_shader(field.gpu_data(), &cases, source.clone());
        for (case, result) in cases.iter().zip(&results) {
            assert_eq!(
                &result[..3],
                &[1.0; 3],
                "fast path differs at {case:?}: {result:?}"
            );
        }
        assert!(
            results.iter().any(|r| r[3] > 0.5),
            "must exercise actual lowered terrain"
        );
        assert!(
            results.iter().any(|r| r[3] < 0.5),
            "must exercise unchanged terrain"
        );
    }
}

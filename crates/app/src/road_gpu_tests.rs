//! Execute the production terrain-cut lookup, not a Rust copy of its shader.
use glam::DVec3;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires Vulkan; run explicitly when the road cut field changes"]
fn gpu_road_cuts_match_cpu_at_bends_banks_ends_and_underpasses() {
    let plan = crate::road::Viaduct::survey(|d| {
        let x = d.dot(crate::road::across()) * crate::planet::planet_radius_meters();
        let y = d.dot(crate::road::forward()) * crate::planet::planet_radius_meters();
        Some(1900.0 + 200.0 * (-(x / 350.0).powi(2) - ((y - 8000.0) / 400.0).powi(2)).exp())
    })
    .unwrap();
    assert!(plan.route.points.iter().any(|p| p.x.abs() > 300.0));
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
    let source = format!(
        "const ROAD_CUTS_ENABLED: bool = true;\n{}\n{}",
        include_str!("road_cuts.wgsl").replace("@group(2) @binding(21)", "@group(0) @binding(1)"),
        "@group(0) @binding(0) var<storage,read_write> results: array<f32>;\n@group(0) @binding(2) var<storage,read> cases: array<vec4<f32>>;\n@compute @workgroup_size(1) fn test_cut(@builtin(global_invocation_id) id: vec3<u32>) { let c=cases[id.x]; results[id.x]=road_terrain_cut(c.xyz,c.w).height; }"
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
        contents: bytemuck::cast_slice(&plan.cuts.gpu_data()),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cut samples"),
        contents: bytemuck::cast_slice(&cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (cases.len() * 4) as u64;
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
    let heights = bytemuck::cast_slice::<u8, f32>(&data);
    let mut maximum = 0.0_f64;
    for (c, h) in cases.iter().zip(heights) {
        let expected = plan.cuts.height(
            DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64),
            c[3] as f64,
        );
        let error = (*h as f64 - expected).abs();
        maximum = maximum.max(error);
        assert!(
            error < 0.75,
            "CPU {expected}, GPU {h}, error {error}, case {c:?}"
        );
        assert!(*h <= c[3] + 0.001);
    }
    eprintln!(
        "{} production cut samples, max CPU/GPU difference {maximum:.4} m",
        cases.len()
    );
}

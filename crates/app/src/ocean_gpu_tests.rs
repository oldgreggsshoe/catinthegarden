//! Executes the production WGSL wave/normal path, rather than a Rust copy of
//! its algebra. Explicit opt-in: ordinary unit tests do not require a GPU.
use super::*;
use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a Vulkan GPU; run explicitly for ocean shader changes"]
fn gpu_ocean_normals_match_cpu_buoyancy_in_deep_and_breaking_water() {
    // A small test radius keeps f32 phase reduction out of this derivative
    // regression. Real-planet phase precision is a separate rendering concern.
    let test_body = crate::body::Body {
        radius_meters: 64.0,
        ..crate::body::PLANET
    };
    crate::body::with_body(test_body, || {
        let mut cases = Vec::new();
        for direction in [
            DVec3::new(0.836, 0.504, 0.216),
            DVec3::X,
            DVec3::Y,
            DVec3::new(-0.3, 0.8, 0.5),
            // Spawn-coast prototype: interior, both sides of the blend, and
            // exterior. Run with the opt-in both unset and enabled.
            DVec3::new(0.84285087, 0.49512231, 0.21084662),
            DVec3::new(0.84285087, 0.49512231, 0.21084662)
                + DVec3::new(0.48197104, -0.86880293, 0.11351380) * 0.0025,
            DVec3::new(0.84285087, 0.49512231, 0.21084662)
                + DVec3::new(0.48197104, -0.86880293, 0.11351380) * 0.0035,
            DVec3::new(0.84285087, 0.49512231, 0.21084662)
                + DVec3::new(0.48197104, -0.86880293, 0.11351380) * 0.005,
        ] {
            // Use the same rounded input in both implementations.
            let direction = direction.normalize().as_vec3().as_dvec3();
            for depth in [2.0, 20.0, 4000.0] {
                for time in [0.0, 7.0] {
                    cases.push((direction, depth, time));
                }
            }
        }
        let directions = cases
            .iter()
            .map(|(d, depth, _)| {
                format!(
                    "vec4<f32>({:?}, {:?}, {:?}, {:?})",
                    d.x as f32, d.y as f32, d.z as f32, *depth as f32,
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        let times = cases
            .iter()
            .map(|(_, _, time)| format!("{:?}", *time as f32))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!(
            "{}\n{}",
            crate::planet::shared_planet_shader_source(),
            format_args!(
                r#"
@group(0) @binding(1) var<storage, read_write> results: array<vec4<f32>>;
const cases = array<vec4<f32>, {count}>({directions});
const times = array<f32, {count}>({times});
@compute @workgroup_size(1)
fn test_ocean(@builtin(global_invocation_id) id: vec3<u32>) {{
    let sample = cases[id.x];
    let surface = ocean_surface(normalize(sample.xyz), times[id.x], 0.0, sample.w);
    results[id.x] = vec4<f32>(surface.normal, surface.vertical_displacement);
}}
"#,
                count = cases.len()
            )
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("Vulkan adapter");
        eprintln!("ocean normal test adapter: {}", adapter.get_info().name);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("GPU device");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production ocean derivative regression"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ocean derivative regression"),
            layout: None,
            module: &shader,
            entry_point: Some("test_ocean"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut camera = crate::planet::CameraUniform::zeroed();
        camera.flat_triangle_options[1] = GLOBAL_OCEAN_STORM_INTENSITY;
        camera.flat_triangle_options[2] = 1.0; // actual radial geometry; no shading-only ripples
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test ocean camera"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bytes = (cases.len() * size_of::<[f32; 4]>()) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test ocean results"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test ocean readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test ocean inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
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
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
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
        let rows: &[[f32; 4]] = bytemuck::cast_slice(&data);
        let mut failures = Vec::new();
        let mut maximum_normal_error = 0.0_f64;
        let mut maximum_height_error = 0.0_f64;
        for ((direction, depth, time), gpu) in cases.iter().zip(rows) {
            let direction = direction.normalize();
            let normal = (direction - global_wave_slope(direction, *time, *depth)).normalize();
            let height = global_wave_height_meters(direction, *time, *depth);
            let normal_error =
                normal.distance(DVec3::new(gpu[0] as f64, gpu[1] as f64, gpu[2] as f64));
            let height_error = (height - gpu[3] as f64).abs();
            maximum_normal_error = maximum_normal_error.max(normal_error);
            maximum_height_error = maximum_height_error.max(height_error);
            if !normal_error.is_finite()
                || !height_error.is_finite()
                || normal_error > 0.002
                || height_error > 0.02
            {
                failures.push(format!("direction={direction:?} depth={depth} time={time}: normal error={normal_error}, height error={height_error}"));
            }
        }
        eprintln!(
            "{} cases: maximum normal error {maximum_normal_error}, height error {maximum_height_error}",
            cases.len()
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}

#[test]
#[ignore = "requires a Vulkan GPU; run explicitly for ocean shader changes"]
fn gpu_ocean_refraction_matches_snell_and_fresnel() {
    check_ocean_optics(OpticsCase::LeavingWater);
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn gpu_ocean_air_to_water_and_visibility() {
    check_ocean_optics(OpticsCase::EnteringWater);
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn gpu_beach_sand_is_continuous_at_the_dry_land_join() {
    check_ocean_optics(OpticsCase::BeachSand);
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn gpu_ocean_underside_reflection_retains_bounded_skylight() {
    check_ocean_optics(OpticsCase::UndersideSkylight);
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn gpu_ocean_extinction_increases_smoothly_with_water_depth() {
    check_ocean_optics(OpticsCase::DepthExtinction);
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn gpu_ocean_underside_foam_blocks_directional_light_with_pale_scatter() {
    check_ocean_optics(OpticsCase::UndersideFoam);
}

#[test]
#[ignore = "requires a Vulkan GPU"]
fn gpu_local_view_altitude_recovers_depth_without_radius_cancellation() {
    check_ocean_optics(OpticsCase::LocalAltitude);
}

/// The SSR recovery divides out fog the snapshot already carries. That only
/// inverts if it uses the depth the fog was applied with -- the reflected
/// point's own -- and not the water column above the reflecting fragment.
#[test]
#[ignore = "requires a Vulkan GPU"]
fn gpu_water_fog_inverse_cancels_only_at_the_depth_it_was_applied_with() {
    check_ocean_optics(OpticsCase::FogInverseDepth);
}

#[derive(Clone, Copy)]
enum OpticsCase {
    LeavingWater,
    EnteringWater,
    BeachSand,
    UndersideSkylight,
    DepthExtinction,
    UndersideFoam,
    LocalAltitude,
    FogInverseDepth,
}

fn check_ocean_optics(case: OpticsCase) {
    let entering_water = matches!(case, OpticsCase::EnteringWater);
    let beach_sand = matches!(case, OpticsCase::BeachSand);
    let underside_skylight = matches!(case, OpticsCase::UndersideSkylight);
    let depth_extinction = matches!(case, OpticsCase::DepthExtinction);
    let underside_foam = matches!(case, OpticsCase::UndersideFoam);
    let local_altitude = matches!(case, OpticsCase::LocalAltitude);
    let fog_inverse_depth = matches!(case, OpticsCase::FogInverseDepth);
    let cases = [
        (DVec3::Y, DVec3::Y),
        (DVec3::new(0.6, 0.8, 0.0), DVec3::Y),
        (DVec3::new(0.75, 0.6614378278, 0.0).normalize(), DVec3::Y),
        (DVec3::new(0.8, 0.6, 0.0), DVec3::Y),
        (DVec3::Y, DVec3::new(0.4, 0.916515139, 0.0).normalize()),
        (-DVec3::Y, DVec3::Y),
    ];
    let rays = cases
        .iter()
        .map(|(ray, _)| {
            format!(
                "vec3<f32>({:?}, {:?}, {:?})",
                ray.x as f32, ray.y as f32, ray.z as f32
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let normals = cases
        .iter()
        .map(|(_, normal)| {
            format!(
                "vec3<f32>({:?}, {:?}, {:?})",
                normal.x as f32, normal.y as f32, normal.z as f32
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let evaluation = if underside_foam {
        "vec4<f32>(ocean_underside_with_foam(
            vec3<f32>(0.8, 0.7, 0.5), vec3<f32>(0.1, 0.4, 0.9),
            array<f32, 6>(0.0, 0.2, 0.4, 0.6, 0.82, 0.82)[id.x]), 1.0)"
    } else if local_altitude {
        "vec4<f32>(local_view_altitude_meters(array<vec3<f32>, 6>(
            vec3<f32>(0.0, 0.0, 0.0),
            vec3<f32>(0.0, -10.0, 0.0),
            vec3<f32>(0.0, 5.0, 0.0),
            vec3<f32>(0.0, 0.0, -50.0),
            vec3<f32>(30.0, 0.0, 40.0),
            vec3<f32>(20.6155281, -40.0, 0.0)
        )[id.x]))"
    } else if fog_inverse_depth {
        // The residual an inverse leaves when it un-fogs with `used` a fog that
        // was applied with `bed`. Only the amount depends on depth, so this is
        // the whole error. Even lanes match and must cancel exactly; odd lanes
        // are the mismatch the SSR recovery used to have.
        "vec4<f32>(
            ocean_water_fog_amount(vec3<f32>(0.0, 0.0, -30.0),
                array<f32, 6>(25.0, 25.0, 2.0, 2.0, 40.0, 40.0)[id.x])
            - ocean_water_fog_amount(vec3<f32>(0.0, 0.0, -30.0),
                array<f32, 6>(25.0, 1.0, 2.0, 30.0, 40.0, 1.0)[id.x])
        )"
    } else if depth_extinction {
        "vec4<f32>(ocean_depth_extinction_weight(array<f32, 6>(0.0, 1.0, 2.0, 10.0, 30.0, 100.0)[id.x]))"
    } else if underside_skylight {
        "vec4<f32>(ocean_underside_reflection_with_skylight(vec3<f32>(0.8, 0.7, 0.5), vec3<f32>(0.1, 0.4, 0.9)), 1.0)"
    } else if beach_sand {
        "vec4<f32>(beach_sand_albedo(array<f32, 6>(-1.0, 0.0, 0.01, 4.0, 20.0, 40.0)[id.x]), 1.0)"
    } else if entering_water {
        "vec4<f32>(ocean_air_to_water(select(-rays[id.x], rays[id.x], dot(rays[id.x], normals[id.x]) < 0.0), normals[id.x]), ocean_water_transmittance(f32(id.x) * 20.0))"
    } else {
        "ocean_water_to_air(rays[id.x], normals[id.x])"
    };
    let source = format!(
        "{}\n@group(0) @binding(1) var<storage, read_write> results: array<vec4<f32>>;
        const rays = array<vec3<f32>, 6>({rays});
        const normals = array<vec3<f32>, 6>({normals});
        @compute @workgroup_size(1)
        fn test_optics(@builtin(global_invocation_id) id: vec3<u32>) {{
            results[id.x] = {evaluation}
                + vec4<f32>(camera.flat_triangle_options.x);
        }}",
        crate::planet::shared_planet_shader_source(),
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("Vulkan adapter");
    eprintln!("ocean optics test adapter: {}", adapter.get_info().name);
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("GPU device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production ocean optics regression"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ocean optics regression"),
        layout: None,
        module: &shader,
        entry_point: Some("test_optics"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut camera = crate::planet::CameraUniform::zeroed();
    camera.flat_triangle_options[1] = GLOBAL_OCEAN_STORM_INTENSITY;
    camera.flat_triangle_options[2] = 1.0; // actual radial geometry; no shading-only ripples
    if local_altitude {
        // Radial straight up the view-space Y axis, eye 5m under the datum.
        camera.camera_planet_direction_view_altitude = [0.0, 1.0, 0.0, -5.0];
    }
    let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("test ocean camera"),
        contents: bytemuck::bytes_of(&camera),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bytes = (cases.len() * size_of::<[f32; 4]>()) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test ocean results"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test ocean readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test ocean inputs"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
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
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
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
    let rows: &[[f32; 4]] = bytemuck::cast_slice(&data);
    for (index, ((ray, normal), actual)) in cases.iter().zip(rows).enumerate() {
        if underside_foam {
            let foam = [0.0_f32, 0.2, 0.4, 0.6, 0.82, 0.82][index];
            let clear = [0.8_f32, 0.7, 0.5];
            let scattered = [0.432_f32, 0.54, 0.72];
            for channel in 0..3 {
                let expected = clear[channel] * (1.0 - foam) + scattered[channel] * foam;
                assert!(
                    (actual[channel] - expected).abs() < 0.0001,
                    "foam {foam}, channel {channel}: {} vs {expected}",
                    actual[channel]
                );
            }
            continue;
        }
        if local_altitude {
            let radius = catinthegarden_coretypes::PLANET_RADIUS_METERS;
            let points = [
                [0.0_f64, 0.0, 0.0],
                [0.0, -10.0, 0.0],
                [0.0, 5.0, 0.0],
                [0.0, 0.0, -50.0],
                [30.0, 0.0, 40.0],
                // The one row combining rise and horizontal offset. Kept for
                // that coverage only: it does not separate the expansion from
                // the direct sqrt form, because at this 4.0e6 radius the direct
                // form is accurate to about a millimetre anyway.
                [20.6155281, -40.0, 0.0],
            ];
            let point = points[index];
            let rise = point[1];
            let horizontal_squared =
                point[0] * point[0] + point[1] * point[1] + point[2] * point[2] - rise * rise;
            let expected = -5.0 + rise + horizontal_squared / (2.0 * (radius - 5.0));
            // A centimetre: tight enough to catch a sign error, the wrong
            // frame for the radial, or a dropped curvature term.
            assert!(
                (actual[0] as f64 - expected).abs() < 0.01,
                "altitude at {point:?}: {} vs {expected}",
                actual[0]
            );
            continue;
        }
        if fog_inverse_depth {
            let bed = [25.0_f64, 25.0, 2.0, 2.0, 40.0, 40.0][index];
            let used = [25.0_f64, 1.0, 2.0, 30.0, 40.0, 1.0][index];
            if bed == used {
                assert!(
                    (actual[0] as f64).abs() < 1.0e-6,
                    "a matched inverse must cancel exactly, got {}",
                    actual[0]
                );
            } else {
                // Large enough that no tolerance hides it: these are the rows of
                // the measured mismatch table, 0.11 to 0.58 of the whole fog.
                assert!(
                    (actual[0] as f64).abs() > 0.1,
                    "bed {bed} vs used {used} must not be dismissible, got {}",
                    actual[0]
                );
            }
            continue;
        }
        if depth_extinction {
            let depth = [0.0_f64, 1.0, 2.0, 10.0, 30.0, 100.0][index];
            let t = ((depth - 2.0) / 28.0).clamp(0.0, 1.0);
            let smooth = t * t * (3.0 - 2.0 * t);
            let expected = 0.15 + 0.85 * smooth;
            assert!(
                (actual[0] as f64 - expected).abs() < 0.0001,
                "depth {depth}: {} vs {expected}",
                actual[0]
            );
            assert!(actual[0] >= 0.15 && actual[0] <= 1.0);
            continue;
        }
        if underside_skylight {
            let expected = [0.625_f32, 0.625, 0.6];
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[channel]).abs() < 0.0001,
                    "underside skylight blend channel {channel}: {} vs {}",
                    actual[channel],
                    expected[channel]
                );
            }
            continue;
        }
        if beach_sand {
            let srgb = |v: f64| ((v + 0.055) / 1.055).powf(2.4);
            let dry = [srgb(0.94), srgb(0.89), srgb(0.70)];
            for channel in 0..3 {
                if index == 3 {
                    assert!(
                        (actual[channel] as f64 - dry[channel]).abs() > 0.05,
                        "retain the existing wet-sand treatment away from the join"
                    );
                } else {
                    assert!(
                        (actual[channel] as f64 - dry[channel]).abs() < 0.0001,
                        "sand discontinuity: sample {index} channel {channel}: {} vs {}",
                        actual[channel],
                        dry[channel]
                    );
                }
            }
            continue;
        }
        if entering_water {
            let incoming = if ray.dot(*normal) < 0.0 { *ray } else { -*ray };
            let eta = 1.0 / 1.333_f64;
            let cosine = -incoming.dot(*normal);
            let expected = eta * incoming
                + (eta * cosine - (1.0 - eta * eta * (1.0 - cosine * cosine)).sqrt()) * *normal;
            let actual_ray = DVec3::new(actual[0] as f64, actual[1] as f64, actual[2] as f64);
            assert!(
                actual_ray.distance(expected) < 0.00001,
                "case {index}: {actual:?}"
            );
            let transmission = (-(index as f64 * 20.0) * 50.0_f64.ln() / 100.0).exp();
            assert!((actual[3] as f64 - transmission).abs() < 0.00001);
            continue;
        }
        let eta = 1.333_f64;
        let cos_water = ray.dot(*normal).clamp(0.0, 1.0);
        let sin_air_squared = eta * eta * (1.0 - cos_water * cos_water);
        assert!(
            actual.iter().all(|x| x.is_finite()),
            "case {index}: {actual:?}"
        );
        if sin_air_squared >= 1.0 {
            assert_eq!(*actual, [0.0; 4], "total internal reflection case {index}");
            continue;
        }
        let cos_air = (1.0 - sin_air_squared).sqrt();
        let expected_ray = eta * *ray + (cos_air - eta * cos_water) * *normal;
        let rs = (eta * cos_water - cos_air) / (eta * cos_water + cos_air);
        let rp = (eta * cos_air - cos_water) / (eta * cos_air + cos_water);
        let expected_transmission = 1.0 - 0.5 * (rs * rs + rp * rp);
        let actual_ray = DVec3::new(actual[0] as f64, actual[1] as f64, actual[2] as f64);
        assert!(
            actual_ray.distance(expected_ray) < 0.002,
            "case {index}: {actual:?}"
        );
        assert!((actual[3] as f64 - expected_transmission).abs() < 0.002);
        if index == 4 {
            assert!(
                actual_ray.distance(*ray) > 0.1,
                "tilted wave must bend the sky lookup"
            );
        }
    }
}

#[test]
fn underside_reflection_is_bounded_and_confined_to_snapshot_pass() {
    let shader = include_str!("planet.wgsl");
    let reflection = shader
        .split("fn ocean_scene_reflection(")
        .nth(1)
        .unwrap()
        .split("\nfn ")
        .next()
        .unwrap();
    assert!(reflection.contains("reflect(normalize(surface_position), normal_view)"));
    assert!(reflection.contains("step <= 24u"));
    assert!(reflection.contains("ocean_reflection_scene_position(point)"));
    assert!(shader.contains("if farthest - nearest > max(0.5, nearest * 0.05)"));
    assert!(shader.contains("mix(fallback, reflected.rgb, reflected.w)"));
    assert!(reflection.contains("distance(resolved.xyz, hit) > max(0.25, pixel_span * 2.0)"));
    assert!(reflection.contains("color = ocean_depth_aware_distance_fog("));
    let legacy = shader
        .split("fn ocean_underside_fragment(")
        .nth(1)
        .unwrap()
        .split("\nfn ")
        .next()
        .unwrap();
    assert!(legacy.contains("let foam = ocean_foam_coverage("));
    assert!(legacy.contains("input.smooth_normal"));
    assert!(!legacy.contains("let surface = ocean_surface("));
    assert!(legacy.contains("vec4<f32>(0.0),\n                foam,"));
    assert!(!legacy.contains("ocean_scene_reflection("));
    let transmitting = shader
        .split("fn ocean_underside_reflecting_fragment(")
        .nth(1)
        .unwrap()
        .split("\nfn ")
        .next()
        .unwrap();
    assert!(transmitting.contains("let foam = ocean_foam_coverage("));
    assert!(transmitting.contains("input.smooth_normal"));
    assert!(!transmitting.contains("let surface = ocean_surface("));
    assert!(transmitting.contains(
        "vec4<f32>(mix(fallback, reflected.rgb, reflected.w), 1.0),\n                foam,"
    ));
    assert!(transmitting.contains("ocean_scene_reflection("));
}

#[test]
fn underside_diagnostics_expose_each_optical_term_without_entering_f9_cycle() {
    let shader = crate::planet::shared_planet_shader_source();
    assert!(shader.contains("RENDER_DEBUG_UNDERSIDE_TRANSMISSION"));
    assert!(shader.contains("return vec3<f32>(refraction.w)"));
    assert!(shader.contains("RENDER_DEBUG_UNDERSIDE_REFRACTED_SKY"));
    assert!(shader.contains("return physical_camera_sky_radiance(normalize(refraction.xyz));"));
    let raster = include_str!("planet.wgsl");
    assert!(raster.contains("RENDER_DEBUG_UNDERSIDE_REFLECTION_HIT"));
    assert!(raster.contains("return vec4<f32>(vec3<f32>(reflected.w), 1.0)"));

    // These are launch-only diagnostics, not additional F9 presentation modes.
    assert_eq!(
        crate::planet::RenderDebugMode::FlatTriangles.next(),
        crate::planet::RenderDebugMode::Final
    );
}

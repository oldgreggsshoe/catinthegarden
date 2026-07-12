mod debug;
mod planet;
mod scenario;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use wgpu::util::DeviceExt;
use winit::{
    event::{DeviceEvent, Event, MouseScrollDelta, WindowEvent},
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowAttributes},
};

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.08,
    g: 0.08,
    b: 0.09,
    a: 1.0,
};

struct State {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    depth_view: wgpu::TextureView,
    planet_pipeline: wgpu::RenderPipeline,
    planet_vertex_buffer: wgpu::Buffer,
    planet_index_buffer: wgpu::Buffer,
    planet_mesh: planet::CubeSphereMesh,
    camera: planet::OrbitCamera,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    started_at: Instant,
    egui_context: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    last_frame: Instant,
    fps: f32,
    debug_overlay_visible: bool,
    manual_screenshot_requested: bool,
    next_log_time: f64,
    capture_number: usize,
    scenario: Option<scenario::ScenarioRunner>,
    artifacts: debug::RunArtifacts,
    scenario_capture_failed: bool,
    mouse_captured: bool,
    profile_render: bool,
}

impl State {
    async fn new(window: Arc<Window>, scenario_name: Option<String>, profile_render: bool) -> Self {
        let scenario = scenario_name
            .as_deref()
            .map(scenario::ScenarioRunner::load)
            .transpose()
            .expect("scenario must be valid");
        let artifact_name = scenario
            .as_ref()
            .map_or("manual", scenario::ScenarioRunner::name);
        let (artifacts, log_writer) =
            debug::RunArtifacts::create(artifact_name).expect("test-run storage must be writable");
        debug::init_tracing(log_writer);
        tracing::info!(scenario = artifact_name, "run started");

        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("the window must provide a compatible surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("no suitable GPU adapter found");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("render device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("failed to create render device");

        let surface_capabilities = surface.get_capabilities(&adapter);
        let surface_format = surface_capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_capabilities.formats[0]);
        assert!(
            surface_capabilities
                .usages
                .contains(wgpu::TextureUsages::COPY_SRC),
            "the selected surface does not support screenshot readback"
        );
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let depth_view = create_depth_view(&device, size);

        let planet_mesh = planet::CubeSphereMesh::new();
        let camera = planet::OrbitCamera::default();
        let initial_vertices = planet_mesh.rebased_vertices(camera.world_position());
        let planet_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera-relative planet vertices"),
            contents: bytemuck::cast_slice(&initial_vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let planet_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube-sphere indices"),
            contents: bytemuck::cast_slice(planet_mesh.indices()),
            usage: wgpu::BufferUsages::INDEX,
        });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera projection"),
            size: size_of::<planet::CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("planet pipeline layout"),
            bind_group_layouts: &[Some(&camera_bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("planet.wgsl"));
        let planet_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flat-shaded cube-sphere pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[planet::RebasedVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let egui_context = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth_view,
            planet_pipeline,
            planet_vertex_buffer,
            planet_index_buffer,
            planet_mesh,
            camera,
            camera_buffer,
            camera_bind_group,
            started_at: Instant::now(),
            egui_context,
            egui_state,
            egui_renderer,
            last_frame: Instant::now(),
            fps: 0.0,
            debug_overlay_visible: true,
            manual_screenshot_requested: false,
            next_log_time: 0.0,
            capture_number: 0,
            scenario,
            artifacts,
            scenario_capture_failed: false,
            mouse_captured: false,
            profile_render,
        }
    }

    fn resize(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, size);
    }

    fn rotate_camera(&mut self, azimuth_delta: f64, elevation_delta: f64) {
        self.camera.orbit(azimuth_delta, elevation_delta);
    }

    fn look_camera(&mut self, yaw_delta: f64, pitch_delta: f64) {
        self.camera.look(yaw_delta, pitch_delta);
    }

    fn zoom_camera(&mut self, wheel_delta: f64) {
        self.camera.zoom(wheel_delta);
    }

    fn set_mouse_capture(&mut self, window: &Window, captured: bool) {
        if captured {
            let result = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            self.mouse_captured = result.is_ok();
            window.set_cursor_visible(!self.mouse_captured);
            if let Err(error) = result {
                tracing::warn!(%error, "cursor capture is unavailable");
            }
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.mouse_captured = false;
        }
    }

    fn render(&mut self, window: &Window) -> Option<bool> {
        let profile_started = Instant::now();
        let now = Instant::now();
        let frame_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if frame_time > 0.0 {
            self.fps = 1.0 / frame_time;
        }

        let (
            sim_time,
            write_log,
            scenario_capture,
            scenario_complete,
            solid_color_screen,
            hide_overlay,
            seam_gap_check,
            orbit_update,
        ) = if let Some(scenario) = self.scenario.as_mut() {
            let frame = scenario.advance();
            let orbit_update = scenario.orbit_settings().map(|(radius, elevation)| {
                (
                    radius,
                    elevation,
                    frame
                        .orbit_azimuth_radians
                        .expect("orbit scenario has an angle"),
                )
            });
            (
                frame.sim_time,
                frame.write_log,
                frame.capture_screenshot,
                frame.complete,
                scenario.renders_solid_color(),
                scenario.hides_overlay(),
                scenario.needs_seam_gap_check(),
                orbit_update,
            )
        } else {
            let sim_time = self.started_at.elapsed().as_secs_f64();
            let write_log = sim_time >= self.next_log_time;
            if write_log {
                self.next_log_time = sim_time + 0.5;
            }
            (sim_time, write_log, false, false, false, false, false, None)
        };
        if let Some((radius, elevation, azimuth)) = orbit_update {
            self.camera.orbit_radius_meters = radius;
            self.camera.elevation_radians = elevation;
            self.camera.azimuth_radians = azimuth;
            self.camera.look_at_origin();
        }
        let camera_world_position = self.camera.world_position();
        let camera_altitude = self.camera.orbit_radius_meters - planet::PLANET_RADIUS_METERS;
        let draw_calls = u32::from(!solid_color_screen);
        if write_log {
            self.artifacts.record_spatial_log(
                sim_time,
                camera_world_position.to_array(),
                camera_altitude,
                self.camera.azimuth_radians,
                self.camera.elevation_radians,
                frame_time * 1000.0,
                draw_calls,
            );
        }
        let simulation_ms = profile_started.elapsed().as_secs_f32() * 1_000.0;

        let mut paint_jobs = None;
        let mut textures_to_free = Vec::new();
        if !solid_color_screen && !hide_overlay {
            let raw_input = self.egui_state.take_egui_input(window);
            let show_debug_overlay = self.debug_overlay_visible;
            let fps = self.fps;
            let camera_position = camera_world_position;
            let camera_direction = self.camera.direction();
            let full_output = self.egui_context.run_ui(raw_input, |ui| {
                if show_debug_overlay {
                    let context = ui.ctx().clone();
                    egui::Window::new("Cat in the Garden")
                        .default_pos([12.0, 12.0])
                        .show(&context, |ui| {
                            ui.label("Phase 0.5: debug/test harness");
                            ui.label(format!("FPS: {fps:.0}"));
                            ui.label(format!(
                                "Camera: [{:.0}, {:.0}, {:.0}] m",
                                camera_position.x, camera_position.y, camera_position.z
                            ));
                            ui.label(format!(
                                "Direction: [{:.3}, {:.3}, {:.3}]",
                                camera_direction.x, camera_direction.y, camera_direction.z
                            ));
                            ui.label(format!("Altitude: {camera_altitude:.0} m  |  LOD: fixed"));
                            ui.label("F3: toggle overlay  |  F12: capture PNG");
                            ui.label("Mouse: orbit  |  Wheel: zoom  |  Esc/Q: quit");
                        });
                }
            });
            self.egui_state
                .handle_platform_output(window, full_output.platform_output);
            for (texture_id, image_delta) in &full_output.textures_delta.set {
                self.egui_renderer.update_texture(
                    &self.device,
                    &self.queue,
                    *texture_id,
                    image_delta,
                );
            }
            textures_to_free = full_output.textures_delta.free;
            paint_jobs = Some(
                self.egui_context
                    .tessellate(full_output.shapes, self.egui_context.pixels_per_point()),
            );
        }
        let egui_ms = profile_started.elapsed().as_secs_f32() * 1_000.0 - simulation_ms;
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.size.width, self.size.height],
            pixels_per_point: window.scale_factor() as f32,
        };

        let mut reconfigure_surface = false;
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) => output,
            wgpu::CurrentSurfaceTexture::Suboptimal(output) => {
                reconfigure_surface = true;
                output
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.resize(self.size);
                return None;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return None,
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render encoder"),
            });
        if let Some(paint_jobs) = &paint_jobs {
            self.egui_renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                paint_jobs,
                &screen_descriptor,
            );
        }

        let rebased_vertices = self
            .planet_mesh
            .rebased_vertices(self.camera.world_position());
        self.queue.write_buffer(
            &self.planet_vertex_buffer,
            0,
            bytemuck::cast_slice(&rebased_vertices),
        );
        let rebase_upload_ms =
            profile_started.elapsed().as_secs_f32() * 1_000.0 - simulation_ms - egui_ms;
        let camera_uniform = planet::CameraUniform::from_camera(
            &self.camera,
            self.size.width as f32 / self.size.height as f32,
        );
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cube-sphere pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            if !solid_color_screen {
                render_pass.set_pipeline(&self.planet_pipeline);
                render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.planet_vertex_buffer.slice(..));
                render_pass.set_index_buffer(
                    self.planet_index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                render_pass.draw_indexed(0..self.planet_mesh.indices().len() as u32, 0, 0..1);
            }
        }
        if let Some(paint_jobs) = &paint_jobs {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.egui_renderer.render(
                &mut render_pass.forget_lifetime(),
                paint_jobs,
                &screen_descriptor,
            );
        }

        let capture_requested = self.manual_screenshot_requested || scenario_capture;
        self.manual_screenshot_requested = false;
        let pending_capture = capture_requested.then(|| {
            self.capture_number += 1;
            debug::schedule_capture(
                &self.device,
                &mut encoder,
                &output.texture,
                self.size.width,
                self.size.height,
                self.config.format,
                self.capture_number,
            )
        });

        let encode_ms = profile_started.elapsed().as_secs_f32() * 1_000.0
            - simulation_ms
            - egui_ms
            - rebase_upload_ms;

        let submit_started = Instant::now();
        self.queue.submit(Some(encoder.finish()));
        let submit_ms = submit_started.elapsed().as_secs_f32() * 1_000.0;
        let capture_started = Instant::now();
        if let Some(pending_capture) = pending_capture {
            if let Err(error) = debug::finish_capture(
                &self.device,
                pending_capture,
                &mut self.artifacts,
                sim_time,
                solid_color_screen,
                seam_gap_check,
            ) {
                self.scenario_capture_failed = true;
                tracing::error!(%error, "screenshot capture failed");
            }
        }
        let capture_readback_ms = capture_started.elapsed().as_secs_f32() * 1_000.0;
        output.present();

        if self.profile_render && write_log {
            self.artifacts.record_render_profile(
                sim_time,
                simulation_ms,
                egui_ms,
                rebase_upload_ms,
                encode_ms,
                submit_ms,
                capture_readback_ms,
                profile_started.elapsed().as_secs_f32() * 1_000.0,
            );
        }

        for texture_id in &textures_to_free {
            self.egui_renderer.free_texture(texture_id);
        }

        if reconfigure_surface {
            self.resize(self.size);
        }

        if scenario_complete {
            let expected_screenshots = self
                .scenario
                .as_ref()
                .map_or(0, scenario::ScenarioRunner::expected_screenshots);
            let passed = !self.scenario_capture_failed
                && self.artifacts.screenshot_count() == expected_screenshots
                && self.artifacts.spatial_log_count()
                    >= self
                        .scenario
                        .as_ref()
                        .map_or(0, scenario::ScenarioRunner::expected_log_samples);
            self.artifacts.finish(passed).unwrap_or_else(
                |error| tracing::error!(%error, "could not finalize test-run manifest"),
            );
            tracing::info!(passed, "scenario completed");
            return Some(passed);
        }

        None
    }
}

fn main() {
    let launch_options = launch_options().unwrap_or_else(|error| panic!("{error}"));
    let scenario_failed = Arc::new(AtomicBool::new(false));
    let scenario_failed_in_loop = scenario_failed.clone();
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let window = Arc::new(
        event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("Cat in the Garden")
                    .with_inner_size(winit::dpi::LogicalSize::new(960.0, 640.0)),
            )
            .expect("failed to create window"),
    );
    let mut state = pollster::block_on(State::new(
        window.clone(),
        launch_options.scenario_name,
        launch_options.profile_render,
    ));
    state.set_mouse_capture(&window, true);

    event_loop
        .run(move |event, event_loop| match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => {
                if matches!(
                    &event,
                    WindowEvent::KeyboardInput { event, .. }
                        if event.state.is_pressed()
                            && matches!(
                                event.physical_key,
                                PhysicalKey::Code(KeyCode::Escape | KeyCode::KeyQ)
                            )
                ) {
                    event_loop.exit();
                    return;
                }
                let egui_response = state.egui_state.on_window_event(&window, &event);
                if egui_response.repaint {
                    window.request_redraw();
                }

                if !egui_response.consumed {
                    match event {
                        WindowEvent::CloseRequested => event_loop.exit(),
                        WindowEvent::Focused(focused) => state.set_mouse_capture(&window, focused),
                        WindowEvent::Resized(size) => state.resize(size),
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::F3) =>
                        {
                            state.debug_overlay_visible = !state.debug_overlay_visible;
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::F12) =>
                        {
                            state.manual_screenshot_requested = true;
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowLeft) =>
                        {
                            state.rotate_camera(-0.08, 0.0);
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowRight) =>
                        {
                            state.rotate_camera(0.08, 0.0);
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowUp) =>
                        {
                            state.rotate_camera(0.0, 0.05);
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state.is_pressed()
                                && event.physical_key == PhysicalKey::Code(KeyCode::ArrowDown) =>
                        {
                            state.rotate_camera(0.0, -0.05);
                            window.request_redraw();
                        }
                        WindowEvent::MouseWheel { delta, .. } => {
                            let wheel_delta = match delta {
                                MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                                MouseScrollDelta::PixelDelta(position) => position.y / 80.0,
                            };
                            state.zoom_camera(wheel_delta);
                            window.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            if let Some(passed) = state.render(&window) {
                                scenario_failed_in_loop.store(!passed, Ordering::Relaxed);
                                event_loop.exit();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::DeviceEvent {
                event: DeviceEvent::MouseMotion { delta },
                ..
            } if state.mouse_captured => {
                state.look_camera(delta.0 * 0.003, -delta.1 * 0.003);
                window.request_redraw();
            }
            Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .expect("event loop failed");

    if scenario_failed.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}

struct LaunchOptions {
    scenario_name: Option<String>,
    profile_render: bool,
}

fn launch_options() -> Result<LaunchOptions, String> {
    let mut options = LaunchOptions {
        scenario_name: None,
        profile_render: false,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--scenario" => {
                options.scenario_name = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--scenario requires a scenario name".to_owned())?,
                )
            }
            "--profile-render" => options.profile_render = true,
            _ => return Err(format!("unrecognized argument '{flag}'")),
        }
    }
    Ok(options)
}

fn create_depth_view(
    device: &wgpu::Device,
    size: winit::dpi::PhysicalSize<u32>,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("reversed-z depth texture"),
            size: wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

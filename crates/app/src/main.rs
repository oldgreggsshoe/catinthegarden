mod debug;
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
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowAttributes},
};

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.08,
    g: 0.08,
    b: 0.09,
    a: 1.0,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 3],
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

const TRIANGLE: [Vertex; 3] = [
    Vertex {
        position: [0.0, 0.6],
        color: [1.0, 0.48, 0.44],
    },
    Vertex {
        position: [-0.52, -0.36],
        color: [0.95, 0.38, 0.38],
    },
    Vertex {
        position: [0.52, -0.36],
        color: [1.0, 0.62, 0.54],
    },
];

struct State {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    triangle_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    time_buffer: wgpu::Buffer,
    time_bind_group: wgpu::BindGroup,
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
}

impl State {
    async fn new(window: Arc<Window>, scenario_name: Option<String>) -> Self {
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("triangle.wgsl"));
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("triangle vertices"),
            contents: bytemuck::cast_slice(&TRIANGLE),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let time_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("triangle rotation time"),
            size: size_of::<f32>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let time_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("triangle time layout"),
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
        let time_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("triangle time bind group"),
            layout: &time_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: time_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("triangle pipeline layout"),
            bind_group_layouts: &[Some(&time_bind_group_layout)],
            immediate_size: 0,
        });
        let triangle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rotating triangle pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Vertex::layout()],
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
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
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
            triangle_pipeline,
            vertex_buffer,
            time_buffer,
            time_bind_group,
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
    }

    fn render(&mut self, window: &Window) -> Option<bool> {
        let now = Instant::now();
        let frame_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if frame_time > 0.0 {
            self.fps = 1.0 / frame_time;
        }

        let (sim_time, write_log, scenario_capture, scenario_complete, solid_color_screen) =
            if let Some(scenario) = self.scenario.as_mut() {
                let frame = scenario.advance();
                (
                    frame.sim_time,
                    frame.write_log,
                    frame.capture_screenshot,
                    frame.complete,
                    scenario.renders_solid_color(),
                )
            } else {
                let sim_time = self.started_at.elapsed().as_secs_f64();
                let write_log = sim_time >= self.next_log_time;
                if write_log {
                    self.next_log_time = sim_time + 0.5;
                }
                (sim_time, write_log, false, false, false)
            };
        let draw_calls = u32::from(!solid_color_screen);
        if write_log {
            self.artifacts
                .record_spatial_log(sim_time, frame_time * 1000.0, draw_calls);
        }

        let mut paint_jobs = None;
        let mut textures_to_free = Vec::new();
        if !solid_color_screen {
            let raw_input = self.egui_state.take_egui_input(window);
            let show_debug_overlay = self.debug_overlay_visible;
            let fps = self.fps;
            let full_output = self.egui_context.run_ui(raw_input, |ui| {
                if show_debug_overlay {
                    let context = ui.ctx().clone();
                    egui::Window::new("Cat in the Garden")
                        .default_pos([12.0, 12.0])
                        .show(&context, |ui| {
                            ui.label("Phase 0.5: debug/test harness");
                            ui.label(format!("FPS: {fps:.0}"));
                            ui.label("F3: toggle overlay  |  F12: capture PNG");
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

        let elapsed_seconds = self.started_at.elapsed().as_secs_f32();
        self.queue
            .write_buffer(&self.time_buffer, 0, bytemuck::bytes_of(&elapsed_seconds));
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("triangle pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            if !solid_color_screen {
                render_pass.set_pipeline(&self.triangle_pipeline);
                render_pass.set_bind_group(0, &self.time_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.draw(0..TRIANGLE.len() as u32, 0..1);
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

        self.queue.submit(Some(encoder.finish()));
        if let Some(pending_capture) = pending_capture {
            if let Err(error) = debug::finish_capture(
                &self.device,
                pending_capture,
                &mut self.artifacts,
                sim_time,
                solid_color_screen,
            ) {
                self.scenario_capture_failed = true;
                tracing::error!(%error, "screenshot capture failed");
            }
        }
        output.present();

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
                && self.artifacts.spatial_log_count() >= 10;
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
    let scenario_name = scenario_argument().unwrap_or_else(|error| panic!("{error}"));
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
    let mut state = pollster::block_on(State::new(window.clone(), scenario_name));

    event_loop
        .run(move |event, event_loop| match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => {
                let egui_response = state.egui_state.on_window_event(&window, &event);
                if egui_response.repaint {
                    window.request_redraw();
                }

                if !egui_response.consumed {
                    match event {
                        WindowEvent::CloseRequested => event_loop.exit(),
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
            Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .expect("event loop failed");

    if scenario_failed.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}

fn scenario_argument() -> Result<Option<String>, String> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next() {
        None => Ok(None),
        Some(flag) if flag == "--scenario" => arguments
            .next()
            .map(Some)
            .ok_or_else(|| "--scenario requires a scenario name".to_owned()),
        Some(flag) => Err(format!("unrecognized argument '{flag}'")),
    }
}

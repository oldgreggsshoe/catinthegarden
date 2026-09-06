//! Opt-in two-body replay. Both terrain renderers remain resident throughout;
//! only the camera changes frame. The ordinary single-body path is untouched.
use super::*;
use glam::{DQuat, DVec3};

const STEP: f64 = 1.0 / 60.0;
const WARMUP: u32 = 180;
const END: u32 = 3240;
const CAPTURES: [u32; 7] = [240, 480, 1080, 1620, 2280, 2880, 3240];

#[derive(Clone, Copy)]
struct BodyFrame {
    origin: DVec3,
    rotation: DQuat,
}
impl BodyFrame {
    fn position(self, world: DVec3) -> DVec3 {
        self.rotation.conjugate() * (world - self.origin)
    }
    fn direction(self, world: DVec3) -> DVec3 {
        self.rotation.conjugate() * world
    }
}

struct Route {
    moon: BodyFrame,
    departure: DVec3,
    landing: DVec3,
    landing_normal: DVec3,
}
/// The eye has to clear the ground *around* it, not just the one sample beneath
/// it. Seating the camera 2m over the height directly below put it 2.05m under
/// terrain 2m away and 4.09m under terrain 10m away: the departure sits on
/// broken ground, so the surroundings overtop the point they are measured from.
/// The visible symptom was the ground drawn edge-on as a one-pixel line across
/// six frames of the pitch-up, with sky both sides of it.
const EYE_CLEARANCE_METERS: f64 = 2.0;
const GROUND_NEIGHBOURHOOD_METERS: f64 = 10.0;

/// Highest surface within `GROUND_NEIGHBOURHOOD_METERS` of `direction`, taken
/// over two rings so a narrow rise between the samples cannot slip through.
/// Returns `None` only where the centre itself has no surface, keeping the
/// caller's existing "leave the pose alone" fallback.
fn local_ground_height_meters(
    terrain: &terrain::TerrainRenderer,
    direction: DVec3,
    altitude_meters: f64,
    body_radius_meters: f64,
) -> Option<f64> {
    let mut highest = terrain.raster_surface_height_meters_at(direction, altitude_meters)?;
    let aside = if direction.z.abs() < 0.9 {
        DVec3::Z
    } else {
        DVec3::X
    };
    let u = direction.cross(aside).normalize();
    let v = direction.cross(u);
    for reach in [
        GROUND_NEIGHBOURHOOD_METERS * 0.5,
        GROUND_NEIGHBOURHOOD_METERS,
    ] {
        let offset = reach / body_radius_meters;
        for step in 0..8 {
            let angle = step as f64 / 8.0 * std::f64::consts::TAU;
            let around = (direction + (u * angle.cos() + v * angle.sin()) * offset).normalize();
            if let Some(height) = terrain.raster_surface_height_meters_at(around, altitude_meters) {
                highest = highest.max(height);
            }
        }
    }
    Some(highest)
}

fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
impl Route {
    fn new(planet_height: f64, moon_height: f64, moon_direction: DVec3) -> Self {
        let landing_normal = DVec3::new(-0.35, 0.30, 0.887).normalize();
        let moon = BodyFrame {
            origin: DVec3::X * 40_000_000.0,
            rotation: DQuat::from_rotation_arc(moon_direction, landing_normal),
        };
        Self {
            moon,
            departure: DVec3::X * (body::PLANET.radius_meters + planet_height + 2.0),
            landing: moon.origin + landing_normal * (body::MOON.radius_meters + moon_height + 2.0),
            landing_normal,
        }
    }
    fn pose(&self, time: f64) -> (DVec3, DVec3, DVec3, &'static str) {
        let exit = DVec3::X * 16_000_000.0;
        let approach = self.moon.origin + self.landing_normal * 10_000_000.0;
        let (position, phase) = if time < 8.0 {
            (self.departure, "planet_surface")
        } else if time < 20.0 {
            (
                self.departure.lerp(exit, ease((time - 8.0) / 12.0)),
                "ascent",
            )
        } else if time < 32.0 {
            (exit.lerp(approach, ease((time - 20.0) / 12.0)), "transfer")
        } else if time < 48.0 {
            // Exponential distance gives the last kilometres adequate residency
            // time rather than spending nearly the entire descent in vacuum.
            let t = ease((time - 32.0) / 16.0);
            let distance = ((approach - self.landing).length() + 1.0).ln();
            (
                self.landing + self.landing_normal * ((distance * (1.0 - t)).exp() - 1.0),
                "descent",
            )
        } else {
            (self.landing, "moon_surface")
        };
        let travel_look = (approach - exit).normalize();
        let toward_planet = -self.moon.origin.normalize();
        let tangent = (toward_planet
            - self.landing_normal * toward_planet.dot(self.landing_normal))
        .normalize();
        let lunar_look = (tangent + self.landing_normal * 0.25).normalize();
        let orientation = |forward: DVec3, up: DVec3| {
            let right = forward.cross(up).normalize();
            DQuat::from_mat3(&glam::DMat3::from_cols(
                right,
                right.cross(forward),
                -forward,
            ))
        };
        let ground = orientation(DVec3::Z, DVec3::X);
        let upward = orientation(DVec3::X, -DVec3::Z);
        let transit = orientation(travel_look, DVec3::Y);
        let descent = orientation(-self.landing_normal, DVec3::Y);
        let landed = orientation(lunar_look, self.landing_normal);
        let attitude = if time < 8.0 {
            ground.slerp(upward, ease((time - 4.0) / 4.0))
        } else if time < 20.0 {
            upward.slerp(transit, ease((time - 16.0) / 4.0))
        } else if time < 32.0 {
            transit.slerp(descent, ease((time - 28.0) / 4.0))
        } else {
            descent.slerp(landed, ease((time - 42.0) / 6.0))
        };
        (position, attitude * -DVec3::Z, attitude * DVec3::Y, phase)
    }
}

pub(super) struct SystemFlight {
    terrain: terrain::TerrainRenderer,
    atmosphere: atmosphere::AtmosphereRenderer,
    camera_buffer: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    route: Route,
    frame: u32,
    failed: bool,
    nearest_only: bool,
    size: winit::dpi::PhysicalSize<u32>,
    composite: Composite,
}
impl SystemFlight {
    pub(super) fn new(state: &mut State, camera_layout: &wgpu::BindGroupLayout) -> Self {
        assert_eq!(
            body::active(),
            body::PLANET,
            "planet_to_moon starts on --body planet"
        );
        assert_eq!(
            state.render_path,
            RenderPath::Raster,
            "planet_to_moon currently requires raster"
        );
        let planet_height = state
            .terrain
            .prepare_flight_start_surface_height_meters(DVec3::X, 0.0)
            .expect("planet departure height");
        let (atmosphere, mut terrain) = body::with_body(body::MOON, || {
            let atmosphere = atmosphere::AtmosphereRenderer::new(
                &state.device,
                &state.queue,
                hdr::HdrRenderer::SCENE_FORMAT,
                camera_layout,
            );
            let terrain = terrain::TerrainRenderer::new(
                &state.device,
                &state.queue,
                hdr::HdrRenderer::SCENE_FORMAT,
                camera_layout,
                terrain::create_shared_bind_group_layout(&state.device),
                state.weather_clouds.field_bind_group_layout(),
                atmosphere.surface_lighting_resources(),
                terrain::TerrainSource::Outmap(
                    find_outmap(MOON_OUTMAP_PATH).expect("moon outmap must be present"),
                ),
            )
            .expect("moon terrain must initialize");
            (atmosphere, terrain)
        });
        let direction = terrain
            .preferred_landing_direction()
            .expect("moon landing direction");
        let moon_height = body::with_body(body::MOON, || {
            terrain.prepare_flight_start_surface_height_meters(direction, 0.0)
        })
        .expect("moon landing height");
        let camera_buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("moon frame camera"),
            size: size_of::<planet::CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_group = state.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("moon frame camera"),
            layout: camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let composite = Composite::new(state, camera_layout);
        state.hdr.set_auto_exposure_enabled(&state.queue, false);
        state.hdr.set_hdr_effect_enabled(&state.queue, true);
        Self {
            terrain,
            atmosphere,
            camera_buffer,
            camera_group,
            route: Route::new(planet_height, moon_height, direction),
            frame: 0,
            failed: false,
            nearest_only: std::env::var("CATINGARDEN_SYSTEM_CONTROL").is_ok_and(|v| v == "nearest"),
            size: state.size,
            composite,
        }
    }

    pub(super) fn render(&mut self, state: &mut State) -> Option<bool> {
        // Fixed-size experiment: don't silently sample an old-size offscreen
        // target after an interactive resize.
        if self.size != state.size {
            tracing::error!("planet_to_moon requires a fixed render size; restart after resizing");
            return Some(false);
        }
        let now = Instant::now();
        let wall_ms = now.duration_since(state.last_frame).as_secs_f64() * 1000.0;
        state.last_frame = now;
        let step = self.frame.saturating_sub(WARMUP);
        let time = f64::from(step) * STEP;
        let (position, forward, up, phase) = self.route.pose(time);
        let mut moon_position = self.route.moon.position(position);
        let mut moon_forward = self.route.moon.direction(forward);
        let mut moon_up = self.route.moon.direction(up);
        if self.frame < WARMUP {
            moon_position = self.route.moon.position(self.route.landing);
            moon_forward = self
                .route
                .moon
                .direction(self.route.pose(END as f64 * STEP).1);
            moon_up = self.route.moon.direction(up);
        }
        let viewport = [state.size.width, state.size.height];
        let fov = 60_f64.to_radians();
        let planet_nearer = position.length() - body::PLANET.radius_meters
            < moon_position.length() - body::MOON.radius_meters;
        let draw_planet = !self.nearest_only || planet_nearer || self.frame < WARMUP;
        let draw_moon = !self.nearest_only || !planet_nearer || self.frame < WARMUP;
        state.terrain.set_distant_level_limit(distant_level(
            body::PLANET.radius_meters,
            position.length(),
            viewport[1],
            fov,
        ));
        self.terrain.set_distant_level_limit(distant_level(
            body::MOON.radius_meters,
            moon_position.length(),
            viewport[1],
            fov,
        ));
        let planet_stats = if draw_planet {
            state
                .terrain
                .update(
                    position,
                    forward,
                    up,
                    self.frame as f64 * STEP,
                    viewport,
                    fov,
                )
                .expect("planet system terrain update")
        } else {
            terrain::TerrainStats::default()
        };
        let moon_stats = if draw_moon {
            body::with_body(body::MOON, || {
                self.terrain.update(
                    moon_position,
                    moon_forward,
                    moon_up,
                    self.frame as f64 * STEP,
                    viewport,
                    fov,
                )
            })
            .expect("moon system terrain update")
        } else {
            terrain::TerrainStats::default()
        };
        if self.frame < WARMUP {
            // Source streaming may refine the landing height. Settle it before
            // the camera starts, never snap the body transform during travel.
            let d = self.route.moon.position(self.route.landing).normalize();
            if let Some(h) = body::with_body(body::MOON, || {
                self.terrain.raster_surface_height_meters_at(
                    d,
                    moon_position.length() - body::MOON.radius_meters,
                )
            }) {
                self.route.landing = self.route.moon.origin
                    + self.route.landing_normal * (body::MOON.radius_meters + h + 2.0);
            }
            if let Some(h) = local_ground_height_meters(
                &state.terrain,
                DVec3::X,
                position.length() - body::PLANET.radius_meters,
                body::PLANET.radius_meters,
            ) {
                self.route.departure =
                    DVec3::X * (body::PLANET.radius_meters + h + EYE_CLEARANCE_METERS);
            }
        }
        // Re-measured one frame after the last re-seat, against terrain that
        // has streamed further, so it is not the seating arithmetic read back.
        // It catches the eye sinking under nearby ground; it cannot catch
        // GROUND_NEIGHBOURHOOD_METERS itself being too short a reach.
        let departure_altitude = self.route.departure.length() - body::PLANET.radius_meters;
        if self.frame == WARMUP
            && let Some(ground) = local_ground_height_meters(
                &state.terrain,
                DVec3::X,
                departure_altitude,
                body::PLANET.radius_meters,
            )
            && departure_altitude < ground
        {
            tracing::error!(
                eye = departure_altitude,
                ground,
                "departure eye is below nearby ground"
            );
            self.failed = true;
        }
        let planet_altitude = position.length() - body::PLANET.radius_meters;
        let moon_altitude = moon_position.length() - body::MOON.radius_meters;
        let planet_clearance = if planet_altitude < 250_000.0 {
            state
                .terrain
                .raster_surface_height_meters_at(position.normalize(), planet_altitude)
                .map(|h| planet_altitude - h)
        } else {
            None
        };
        let moon_clearance = if moon_altitude < 250_000.0 {
            body::with_body(body::MOON, || {
                self.terrain
                    .raster_surface_height_meters_at(moon_position.normalize(), moon_altitude)
            })
            .map(|h| moon_altitude - h)
        } else {
            None
        };
        if self.frame >= WARMUP
            && (planet_clearance.is_some_and(|c| !c.is_finite() || c < 0.5)
                || moon_clearance.is_some_and(|c| !c.is_finite() || c < 0.5))
        {
            self.failed = true;
        }
        if step == END && !moon_clearance.is_some_and(|c| (0.5..=3.0).contains(&c)) {
            tracing::error!(
                ?moon_clearance,
                "moon landing clearance must be 0.5 to 3 metres"
            );
            self.failed = true;
        }
        let sun = planet::default_sun_direction();
        let uniform = camera_uniform(position, forward, up, sun, viewport);
        let moon_uniform = body::with_body(body::MOON, || {
            camera_uniform(
                moon_position,
                moon_forward,
                moon_up,
                self.route.moon.direction(sun),
                viewport,
            )
        });
        state
            .queue
            .write_buffer(&state.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        state
            .queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&moon_uniform));
        state.stars.update(&state.queue, viewport, 0.0);
        let output = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(o)
            | wgpu::CurrentSurfaceTexture::Suboptimal(o) => o,
            _ => return None,
        };
        let view = output.texture.create_view(&Default::default());
        let mut encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("two body replay"),
            });
        state
            .atmosphere
            .update(&mut encoder, &state.camera_bind_group);
        // The airless sky LUT is constant: initialize once during preflight,
        // not once per frame. Terrain retains its independent irradiance LUT.
        if self.frame == 0 {
            self.atmosphere.update(&mut encoder, &self.camera_group);
        }
        {
            let mut pass = scene_pass(
                &mut encoder,
                state.hdr.scene_view(),
                &state.depth_view,
                true,
            );
            state.atmosphere.draw(&mut pass, &state.camera_bind_group);
            if draw_planet {
                state.terrain.draw(
                    &mut pass,
                    &state.camera_bind_group,
                    state.weather_clouds.field_bind_group(),
                );
            }
        }
        {
            let mut pass = scene_pass(
                &mut encoder,
                &self.composite.colour,
                &self.composite.depth,
                true,
            );
            if draw_moon && self.frame >= WARMUP {
                self.terrain.draw(
                    &mut pass,
                    &self.camera_group,
                    state.weather_clouds.field_bind_group(),
                );
            }
        }
        {
            let mut pass = scene_pass(
                &mut encoder,
                state.hdr.scene_view(),
                &state.depth_view,
                false,
            );
            pass.set_pipeline(&self.composite.pipeline);
            pass.set_bind_group(0, &state.camera_bind_group, &[]);
            pass.set_bind_group(1, &self.composite.sky_group, &[]);
            pass.set_bind_group(2, &self.composite.inputs, &[]);
            pass.draw(0..3, 0..1);
            state.stars.draw(
                &mut pass,
                &state.camera_bind_group,
                state.weather_clouds.field_bind_group(),
            );
            state
                .weather_clouds
                .draw(&mut pass, &state.camera_bind_group);
            state.sun.draw_disc(
                &mut pass,
                &state.camera_bind_group,
                state.weather_clouds.field_bind_group(),
            );
        }
        state.hdr.encode_blur(&mut encoder, None);
        state.hdr.encode_bloom(&mut encoder, None);
        state.hdr.encode_tone_map(&mut encoder, &view, None);
        let capture = (self.frame >= WARMUP && CAPTURES.contains(&step)).then(|| {
            state.capture_number += 1;
            debug::schedule_capture(
                &state.device,
                &mut encoder,
                &output.texture,
                state.surface_size.width,
                state.surface_size.height,
                state.config.format,
                state.capture_number,
            )
        });
        state.queue.submit(Some(encoder.finish()));
        output.present();
        if self.frame >= WARMUP {
            let p = &planet_stats;
            let m = &moon_stats;
            state
                .artifacts
                .record_spatial_sample(debug::SpatialLogSample {
                    sim_time: time,
                    camera_world_position: position.to_array(),
                    latitude_degrees: (position.y / position.length()).asin().to_degrees(),
                    longitude_degrees: position.z.atan2(position.x).to_degrees(),
                    altitude_meters: planet_altitude,
                    velocity_meters_per_second: position
                        .distance(self.route.pose((time - STEP).max(0.0)).0)
                        / STEP,
                    orientation: "shared-space transfer".into(),
                    orientation_azimuth_radians: forward.z.atan2(forward.x),
                    orientation_elevation_radians: forward.y.asin(),
                    vertical_fov_degrees: 60.0,
                    sun_direction: sun.to_array(),
                    planet_rotation_radians: 0.0,
                    lod_level_histogram: std::array::from_fn(|i| {
                        p.level_histogram[i] + m.level_histogram[i]
                    }),
                    chunks_loaded: p.chunks_loaded + m.chunks_loaded,
                    chunks_unloaded: p.chunks_unloaded + m.chunks_unloaded,
                    frame_time_ms: wall_ms as f32,
                    draw_calls: p.draw_calls + m.draw_calls + 5,
                    max_seam_delta_m: p.max_seam_delta_meters.max(m.max_seam_delta_meters),
                    resident_chunks: p.resident_chunks + m.resident_chunks,
                    drawn_chunks: p.drawn_chunks + m.drawn_chunks,
                    terrain_triangles: p.terrain_triangles + m.terrain_triangles,
                    ocean_chunks: p.ocean_chunks + m.ocean_chunks,
                    ocean_triangles: p.ocean_triangles + m.ocean_triangles,
                    fallback_chunks: p.fallback_chunks + m.fallback_chunks,
                    source_level_delta_histogram: std::array::from_fn(|i| {
                        p.source_level_delta_histogram[i] + m.source_level_delta_histogram[i]
                    }),
                    resident_tiles: p.resident_tiles + m.resident_tiles,
                    tiles_loaded: p.tiles_loaded + m.tiles_loaded,
                    tiles_unloaded: p.tiles_unloaded + m.tiles_unloaded,
                    lod_thrash_events: p.lod_thrash_events + m.lod_thrash_events,
                    budget_limited: p.budget_limited || m.budget_limited,
                    exposure: 1.0,
                    ocean_wave_min_meters: 0.0,
                    ocean_wave_max_meters: 0.0,
                });
            tracing::info!(target: "catinthegarden::system_flight", time, phase, wall_ms, nearest_only = self.nearest_only, ?planet_clearance, ?moon_clearance,
                planet_chunks = planet_stats.drawn_chunks, moon_chunks = moon_stats.drawn_chunks,
                planet_triangles = planet_stats.terrain_triangles, moon_triangles = moon_stats.terrain_triangles,
                planet_fallbacks = planet_stats.fallback_chunks, moon_fallbacks = moon_stats.fallback_chunks,
                planet_altitude = position.length() - body::PLANET.radius_meters,
                moon_altitude = moon_position.length() - body::MOON.radius_meters,
                "two body frame");
        }
        if let Some(capture) = capture {
            if let Err(error) = debug::finish_capture(
                &state.device,
                capture,
                &mut state.artifacts,
                time,
                false,
                false,
            ) {
                tracing::error!(%error, "two body capture failed");
                self.failed = true;
            }
            let _ = state.log_writer.flush();
        }
        self.frame += 1;
        if step == END {
            let passed = !self.failed;
            if let Err(error) = state.artifacts.finish(passed) {
                tracing::error!(%error);
                return Some(false);
            }
            let _ = state.log_writer.flush();
            return Some(passed && state.artifacts.assertion_failure_reasons().is_empty());
        }
        None
    }
}

// L0 already has 32 segments per cube-face edge. At a <=100px projected
// radius its smooth-sphere chord error is <0.13px. Preserve the ordinary
// adaptive path once a body is large enough to resolve terrain geometry.
fn distant_level(radius: f64, distance: f64, height: u32, fov: f64) -> Option<u8> {
    if distance <= radius {
        return None;
    }
    let projected_radius = radius / (distance * distance - radius * radius).sqrt()
        * f64::from(height)
        / (2.0 * (fov * 0.5).tan());
    if projected_radius <= 100.0 {
        Some(0)
    } else if projected_radius <= 200.0 {
        Some(1)
    } else {
        None
    }
}

fn camera_uniform(
    position: DVec3,
    forward: DVec3,
    up: DVec3,
    sun: DVec3,
    size: [u32; 2],
) -> planet::CameraUniform {
    let mut camera = planet::OrbitCamera::default();
    camera.set_world_pose_with_up(position, position + forward, up);
    camera.set_vertical_fov_degrees_for_viewport(60.0, size[1]);
    let mut uniform = planet::CameraUniform::from_camera(
        &camera,
        size[0] as f32 / size[1] as f32,
        sun,
        0.0,
        0.0,
        planet::RenderDebugMode::Final,
        planet::FlatTriangleOutlineMode::default(),
        0.0,
    );
    // Every body must use the SAME reversed-Z mapping. Setting each near plane
    // from its own altitude makes cross-body depth comparisons meaningless.
    uniform.projection_matrix[3][2] = 0.1;
    uniform
}

fn scene_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    colour: &'a wgpu::TextureView,
    depth: &'a wgpu::TextureView,
    clear: bool,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("system body pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: colour,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(0.0)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

struct Composite {
    colour: wgpu::TextureView,
    depth: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    sky_group: wgpu::BindGroup,
    inputs: wgpu::BindGroup,
}
impl Composite {
    fn new(state: &State, camera_layout: &wgpu::BindGroupLayout) -> Self {
        let device = &state.device;
        let texture = |format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("system moon target"),
                    size: wgpu::Extent3d {
                        width: state.size.width,
                        height: state.size.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let colour = texture(hdr::HdrRenderer::SCENE_FORMAT);
        let depth = texture(wgpu::TextureFormat::Depth32Float);
        let tex_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("system sky layout"),
            entries: &[
                tex_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("system composite layout"),
            entries: &[
                tex_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                tex_entry(1, wgpu::TextureSampleType::Depth),
                tex_entry(2, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let lighting = state.atmosphere.surface_lighting_resources();
        let sky_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("system sky"),
            layout: &sky_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(lighting.sky_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(lighting.sky_view_sampler),
                },
            ],
        });
        let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("system composite inputs"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&colour),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(lighting.transmittance),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(lighting.physical_sampler),
                },
            ],
        });
        let source = format!(
            "{}\n{}\n{}",
            body::wgsl_constants(),
            include_str!("atmosphere.wgsl"),
            include_str!("system_composite.wgsl")
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("system composite shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("system composite pipeline layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&sky_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("system composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_system"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr::HdrRenderer::SCENE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            colour,
            depth,
            pipeline,
            sky_group,
            inputs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distant_geometry_budget_scales_with_projected_size() {
        let r = body::PLANET.radius_meters;
        assert_eq!(distant_level(r, r + 2.0, 720, 60_f64.to_radians()), None);
        assert_eq!(
            distant_level(r, 40_000_000.0, 720, 60_f64.to_radians()),
            Some(0)
        );
        assert_eq!(
            distant_level(r, 40_000_000.0, 2160, 60_f64.to_radians()),
            Some(1)
        );
        assert_eq!(
            distant_level(r, 40_000_000.0, 720, 10_f64.to_radians()),
            None
        );
    }
    #[test]
    fn route_is_continuous_and_clear_of_both_bodies() {
        let route = Route::new(4400.0, 22000.0, DVec3::X);
        let mut previous = route.pose(0.0).0;
        for i in 0..=54000 {
            let (p, f, up, _) = route.pose(i as f64 / 1000.0);
            assert!(p.is_finite() && f.is_finite());
            assert!((f.length() - 1.0).abs() < 1e-10);
            assert!(f.cross(up).length() > 0.1);
            assert!(p.length() > body::PLANET.radius_meters);
            assert!(route.moon.position(p).length() > body::MOON.radius_meters);
            assert!(p.distance(previous) < 6000.0, "camera jump at {i}");
            previous = p;
        }
        assert!(route.landing_normal.dot(planet::default_sun_direction()) > 0.5);
        assert!(
            route.landing_normal.dot(-DVec3::X) > 0.0,
            "planet must be above lunar horizon"
        );
    }
    #[test]
    fn surface_attitudes_have_local_up_and_no_boundary_snaps() {
        let route = Route::new(0.0, 0.0, DVec3::Y);
        assert!(route.pose(0.0).2.dot(DVec3::X) > 0.999);
        let (_, f, up, _) = route.pose(54.0);
        let expected_up = (route.landing_normal - f * f.dot(route.landing_normal)).normalize();
        assert!(up.dot(expected_up) > 0.999);
        for t in [4.0, 8.0, 16.0, 20.0, 28.0, 32.0, 42.0, 48.0] {
            let before = route.pose(t - 1e-5);
            let after = route.pose(t + 1e-5);
            assert!(before.1.dot(after.1) > 0.999999);
            assert!(before.2.dot(after.2) > 0.999999);
        }
    }
    #[test]
    fn frame_preserves_submetre_precision_and_light_direction() {
        let route = Route::new(0.0, 0.0, DVec3::Y);
        let local = DVec3::Y * (body::MOON.radius_meters + 0.001);
        let world = route.moon.origin + route.moon.rotation * local;
        assert!(route.moon.position(world).distance(local) < 1e-8);
        let sun = planet::default_sun_direction();
        assert!(
            (DVec3::Y.dot(route.moon.direction(sun)) - route.landing_normal.dot(sun)).abs() < 1e-12
        );
    }
    #[test]
    fn composite_shader_validates() {
        let source = body::with_body(body::PLANET, || {
            format!(
                "{}\n{}\n{}",
                body::wgsl_constants(),
                include_str!("atmosphere.wgsl"),
                include_str!("system_composite.wgsl")
            )
        });
        let module = wgpu::naga::front::wgsl::parse_str(&source).expect("composite WGSL parses");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("composite WGSL validates");
    }
}

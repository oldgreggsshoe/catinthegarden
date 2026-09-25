//! The moon in the planet's sky during ordinary play.
//!
//! Reuses the two-body replay's machinery (`system_flight`): a second, fully
//! resident moon terrain renderer drawn from its baked outmap at the coarse
//! level its projected size needs, rendered offscreen with the same
//! reversed-Z projection as the planet, then composited through the planet's
//! atmosphere (in-scatter in front, extinction through the air column) and
//! written into scene depth so terrain, clouds and the sun occlude correctly.
//!
//! The moon orbits at the replay's 40,000km centre distance with the Kepler
//! period for this planet's gravity (GM = g R^2): about 35 real hours, which
//! at the game's 80-minute day is 26.5 days, close to Earth's month. The orbit
//! runs on the planet-rotation clock, so time speed and F10 apply. It is
//! tidally locked, turned so the direction the moon shader already lights
//! with planetshine (`MOON_PLANET_SKY_DIRECTION`) faces the planet: earthshine
//! on the dark side then follows the planet's phase as seen from the moon,
//! strongest on a crescent. It starts on the first rendered frame near that
//! view. No lunar or solar eclipse shadowing yet.
use super::*;
use glam::{DQuat, DVec3};

/// Centre distance, matching the `planet_to_moon` replay.
const MOON_DISTANCE_METERS: f64 = 40_000_000.0;
/// Earthshine seen from the planet. The moon shader's planetshine (a fixed
/// fraction with its own visibility boost, tuned for standing on the moon)
/// comes to ~0.1% of a sunlit crescent here; the physical figure for this
/// geometry (planet albedo 0.3, planet radius / distance 0.1) is ~0.3%. At
/// the game's fixed night exposure either is black, so this lifts it to
/// ~0.6%: the dark limb is just visible on a dark night, as to an adapted
/// eye. Moonlight on the planet itself is not modelled.
const SKY_EARTHSHINE_EXTRA_GAIN: f32 = 5.0;
/// Where the moon is placed on the first frame: this far above the horizon.
const PLACEMENT_ELEVATION_DEGREES: f64 = 20.0;

/// On by default for ordinary launches; `CATINGARDEN_MOON=0` turns it off.
/// Scenarios keep their captures unchanged unless `CATINGARDEN_MOON=1`.
pub(super) fn enabled(is_scenario: bool) -> bool {
    match std::env::var("CATINGARDEN_MOON").ok().as_deref().map(str::trim) {
        Some("0" | "false" | "off") => false,
        Some("1" | "true" | "on") => true,
        _ => !is_scenario,
    }
}

pub(super) struct SkyMoon {
    terrain: terrain::TerrainRenderer,
    atmosphere: atmosphere::AtmosphereRenderer,
    camera_buffer: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    camera_layout: wgpu::BindGroupLayout,
    composite: system_flight::Composite,
    size: winit::dpi::PhysicalSize<u32>,
    orbit: Option<Orbit>,
    visible: bool,
    was_visible: bool,
    sky_lut_ready: bool,
}

impl SkyMoon {
    pub(super) fn new(state: &mut State, camera_layout: &wgpu::BindGroupLayout) -> Option<Self> {
        let started = Instant::now();
        let Some(root) = find_outmap(MOON_OUTMAP_PATH) else {
            tracing::warn!("sky moon disabled: moon outmap not found");
            return None;
        };
        let (atmosphere, terrain) = body::with_body(body::MOON, || {
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
                terrain::TerrainSource::Outmap(root),
                None,
            );
            (atmosphere, terrain)
        });
        let terrain = match terrain {
            Ok(terrain) => terrain,
            Err(error) => {
                tracing::warn!(%error, "sky moon disabled: moon terrain failed to load");
                return None;
            }
        };
        let camera_buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky moon camera"),
            size: size_of::<planet::CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_group = state.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky moon camera"),
            layout: camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let composite = system_flight::Composite::new(state, camera_layout);
        tracing::info!(
            target: "catinthegarden::startup",
            setup_ms = started.elapsed().as_secs_f64() * 1000.0,
            "sky moon enabled"
        );
        Some(Self {
            terrain,
            atmosphere,
            camera_buffer,
            camera_group,
            camera_layout: camera_layout.clone(),
            composite,
            size: state.size,
            orbit: None,
            visible: false,
            was_visible: false,
            sky_lut_ready: false,
        })
    }

    /// Places the moon (first call), updates its terrain and camera, and
    /// decides whether it is in view this frame.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        state: &State,
        planet_uniform: &planet::CameraUniform,
        camera_local: DVec3,
        forward_local: DVec3,
        up_local: DVec3,
        planet_rotation_radians: f64,
        planet_rotation_time: f64,
        presentation_time: f64,
        active: bool,
    ) {
        self.visible = false;
        if !active {
            return;
        }
        if self.size != state.size {
            self.composite = system_flight::Composite::new(state, &self.camera_layout);
            self.size = state.size;
        }
        let sun_local = planet::planet_local_vector(state.sun_direction, planet_rotation_radians);
        let orbit_angle = orbit_angle_radians(planet_rotation_time);
        let orbit = *self.orbit.get_or_insert_with(|| {
            let (position, rotation) = place(camera_local, forward_local, sun_local);
            let orbit = Orbit::new(
                planet::planet_world_vector(position, planet_rotation_radians),
                DQuat::from_rotation_y(planet_rotation_radians) * rotation,
                orbit_angle,
            );
            tracing::info!(
                target: "catinthegarden::startup",
                moon_direction = ?orbit.start_position.normalize().to_array(),
                inclination_degrees = orbit.normal.dot(DVec3::Y).clamp(-1.0, 1.0).acos().to_degrees(),
                period_days = orbital_period_days(),
                lit_fraction = lit_fraction(position, camera_local, sun_local),
                "sky moon placed"
            );
            orbit
        });
        let (origin_world, rotation_world) = orbit.at(orbit_angle);
        let origin = planet::planet_local_vector(origin_world, planet_rotation_radians);
        let rotation = DQuat::from_rotation_y(-planet_rotation_radians) * rotation_world;
        let to_body = |world: DVec3| rotation.conjugate() * world;
        let moon_camera = to_body(camera_local - origin);
        let moon_forward = to_body(forward_local);
        let moon_up = to_body(up_local);

        let fov = state.camera.vertical_fov_radians();
        let viewport = [state.size.width, state.size.height];
        let offset = origin - camera_local;
        let distance = offset.length();
        let angular_radius = (body::MOON.radius_meters / distance.max(body::MOON.radius_meters)).asin();
        let aspect = f64::from(viewport[0]) / f64::from(viewport[1].max(1));
        let half_diagonal = ((fov * 0.5).tan() * (1.0 + aspect * aspect).sqrt()).atan();
        let off_axis = forward_local.normalize().dot(offset / distance).clamp(-1.0, 1.0).acos();
        let in_frustum = off_axis < half_diagonal + angular_radius;
        // Below the planet's geometric horizon, whole disc included.
        let camera_radius = camera_local.length();
        let horizon = (body::PLANET.radius_meters / camera_radius.max(body::PLANET.radius_meters)).asin();
        let from_nadir = (-camera_local / camera_radius).dot(offset / distance).clamp(-1.0, 1.0).acos();
        let below_horizon = from_nadir + angular_radius < horizon;
        let visible = in_frustum && !below_horizon;
        if visible != self.was_visible {
            tracing::info!(
                target: "catinthegarden::sky_moon",
                visible,
                off_axis_degrees = off_axis.to_degrees(),
                elevation_degrees = from_nadir.to_degrees() - 90.0,
                "sky moon visibility changed"
            );
            self.was_visible = visible;
        }
        self.visible = visible;

        self.terrain.set_distant_level_limit(system_flight::distant_level(
            body::MOON.radius_meters,
            moon_camera.length(),
            viewport[1],
            fov,
        ));
        let updated = body::with_body(body::MOON, || {
            self.terrain
                .update(moon_camera, moon_forward, moon_up, presentation_time, viewport, fov)
        });
        if let Err(error) = updated {
            tracing::warn!(%error, "sky moon terrain update failed");
            self.visible = false;
            return;
        }
        // Same view space and the same reversed-Z projection as the planet, so
        // depths compare across bodies; only the body frame changes.
        let mut uniform = *planet_uniform;
        let xyzw = |v: DVec3, w: f32| [v.x as f32, v.y as f32, v.z as f32, w];
        let forward_body = moon_forward.normalize();
        let up_body = moon_up.normalize();
        let right_body = forward_body.cross(up_body).normalize();
        let up_body = right_body.cross(forward_body);
        let radial = moon_camera.normalize();
        uniform.camera_forward = xyzw(forward_body, 0.0);
        uniform.camera_right = xyzw(right_body, 0.0);
        uniform.camera_up = xyzw(up_body, 0.0);
        uniform.camera_planet_direction_view_altitude = xyzw(
            DVec3::new(radial.dot(right_body), radial.dot(up_body), -radial.dot(forward_body)),
            (moon_camera.length() - body::MOON.radius_meters) as f32,
        );
        uniform.sun_direction = xyzw(to_body(sun_local).normalize(), 0.0);
        uniform.flat_triangle_options = [planet_uniform.flat_triangle_options[0], 0.0, 0.0, 0.0];
        // Spare lane read by `planetshine_irradiance` on the moon: extra gain
        // on earthshine for the view from the planet (0 = the moon's own).
        uniform.camera_up[3] = SKY_EARTHSHINE_EXTRA_GAIN;
        state
            .queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Renders the moon offscreen; before the main scene pass.
    pub(super) fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        weather_field_bind_group: &wgpu::BindGroup,
    ) {
        if !self.visible {
            return;
        }
        // The airless sky LUT never changes: build it once.
        if !self.sky_lut_ready {
            self.atmosphere.update(encoder, &self.camera_group);
            self.sky_lut_ready = true;
        }
        let mut pass = system_flight::scene_pass(
            encoder,
            &self.composite.colour,
            &self.composite.depth,
            true,
        );
        self.terrain
            .draw(&mut pass, &self.camera_group, weather_field_bind_group);
    }

    /// Composites the moon into the scene; after opaque ground, before the
    /// sky (which only fills pixels still at far depth).
    pub(super) fn draw_composite(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        camera_bind_group: &wgpu::BindGroup,
    ) {
        if self.visible {
            self.composite.draw(render_pass, camera_bind_group);
        }
    }
}

/// Circular orbit in the planet's world (inertial) frame, prograde, in the
/// least-inclined plane through the starting position. The body turns with
/// the orbit, so the same face keeps looking at the planet.
#[derive(Clone, Copy)]
struct Orbit {
    start_position: DVec3,
    start_rotation: DQuat,
    normal: DVec3,
    start_angle: f64,
}

impl Orbit {
    fn new(start_position: DVec3, start_rotation: DQuat, start_angle: f64) -> Self {
        let radial = start_position.normalize();
        let normal = DVec3::Y - radial * DVec3::Y.dot(radial);
        let normal = if normal.length_squared() > 1.0e-12 {
            normal.normalize()
        } else {
            radial.any_orthonormal_vector()
        };
        Self { start_position, start_rotation, normal, start_angle }
    }

    fn at(self, angle: f64) -> (DVec3, DQuat) {
        let turn = DQuat::from_axis_angle(self.normal, angle - self.start_angle);
        (turn * self.start_position, turn * self.start_rotation)
    }
}

/// Circular-orbit period (real seconds) at `MOON_DISTANCE_METERS` for the
/// planet's surface gravity.
fn orbital_period_seconds() -> f64 {
    let gm = crate::surface_camera::GRAVITY_METERS_PER_SECOND_SQUARED
        * body::PLANET.radius_meters
        * body::PLANET.radius_meters;
    std::f64::consts::TAU * (MOON_DISTANCE_METERS.powi(3) / gm).sqrt()
}

/// The period in planet rotations: physics runs in real seconds, and at 100%
/// time speed the planet turns once per `INTERACTIVE_DAY_REAL_SECONDS`.
fn orbital_period_days() -> f64 {
    orbital_period_seconds() / INTERACTIVE_DAY_REAL_SECONDS
}

/// Orbital phase on the planet-rotation clock.
fn orbit_angle_radians(planet_rotation_time: f64) -> f64 {
    let days = planet_rotation_time / planet::PLANET_ROTATION_PERIOD_SECONDS;
    std::f64::consts::TAU * days / orbital_period_days()
}

/// Fraction of the visible disc that is sunlit, seen from the camera.
fn lit_fraction(moon: DVec3, camera: DVec3, sun: DVec3) -> f64 {
    let to_camera = (camera - moon).normalize();
    0.5 * (1.0 + to_camera.dot(sun.normalize()))
}

/// Chooses a first-frame position `PLACEMENT_ELEVATION_DEGREES` above the
/// horizon near the view direction, preferring the best-lit of a few
/// azimuths that stay in view, and turns the moon so the planet stands where
/// its shader expects it (`MOON_PLANET_SKY_DIRECTION`).
fn place(
    camera: DVec3,
    forward: DVec3,
    sun: DVec3,
) -> (DVec3, DQuat) {
    let up = camera.normalize();
    let mut horizontal = forward - up * forward.dot(up);
    if horizontal.length_squared() < 1.0e-12 {
        horizontal = up.any_orthonormal_vector();
    }
    let horizontal = horizontal.normalize();
    let elevation = PLACEMENT_ELEVATION_DEGREES.to_radians();
    // Kept within +-30 degrees so the first view always contains it.
    let best = [0.0_f64, 15.0, -15.0, 30.0, -30.0]
        .into_iter()
        .map(|azimuth| {
            let turned = DQuat::from_axis_angle(up, azimuth.to_radians()) * horizontal;
            let direction = (turned * elevation.cos() + up * elevation.sin()).normalize();
            // Centre on the ray from the camera, at the chosen centre distance.
            let b = camera.dot(direction);
            let t = -b + (b * b - camera.length_squared() + MOON_DISTANCE_METERS.powi(2)).sqrt();
            camera + direction * t
        })
        .max_by(|a, b| lit_fraction(*a, camera, sun).total_cmp(&lit_fraction(*b, camera, sun)))
        .expect("candidates");
    let facing = DVec3::from_array(catinthegarden_coretypes::moon::MOON_PLANET_SKY_DIRECTION).normalize();
    let rotation = DQuat::from_rotation_arc(facing, -best.normalize());
    (best, rotation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenarios_need_an_explicit_opt_in() {
        // Reads the environment; only meaningful when CATINGARDEN_MOON is unset.
        if std::env::var("CATINGARDEN_MOON").is_err() {
            assert!(enabled(false));
            assert!(!enabled(true));
        }
    }

    #[test]
    fn orbit_period_is_keplerian_and_about_a_month_of_game_days() {
        let hours = orbital_period_seconds() / 3600.0;
        assert!((34.0..37.0).contains(&hours), "{hours}");
        let days = orbital_period_days();
        assert!((25.0..28.0).contains(&days), "{days}");
        // One period brings it back.
        let orbit = Orbit::new(DVec3::new(3.0e7, 2.0e7, 1.0e7), DQuat::IDENTITY, 0.0);
        let (back, _) = orbit.at(std::f64::consts::TAU);
        assert!(back.distance(orbit.start_position) < 1.0e-3);
    }

    #[test]
    fn the_orbit_is_prograde_keeps_distance_and_stays_tidally_locked() {
        let start = DVec3::new(2.0e7, 1.5e7, -3.0e7).normalize() * MOON_DISTANCE_METERS;
        let facing = DVec3::from_array(catinthegarden_coretypes::moon::MOON_PLANET_SKY_DIRECTION).normalize();
        let rotation = DQuat::from_rotation_arc(facing, -start.normalize());
        let orbit = Orbit::new(start, rotation, 1.0);
        for angle in [1.3, 2.7, 4.0, 6.5] {
            let (position, rotation) = orbit.at(angle);
            assert!((position.length() - MOON_DISTANCE_METERS).abs() < 1.0);
            // The planet stays where the moon shader expects it.
            let planet_in_body = (rotation.conjugate() * -position).normalize();
            assert!(planet_in_body.distance(facing) < 1.0e-9);
        }
        // Prograde: angular momentum along the spin axis, like the planet.
        let (ahead, _) = orbit.at(1.01);
        assert!(start.cross(ahead - start).dot(DVec3::Y) > 0.0);
    }

    #[test]
    fn lit_fraction_is_full_opposite_the_sun_and_new_toward_it() {
        let camera = DVec3::X * 4_000_000.0;
        let moon = camera + DVec3::Z * MOON_DISTANCE_METERS;
        assert!((lit_fraction(moon, camera, -DVec3::Z) - 1.0).abs() < 1e-9);
        assert!(lit_fraction(moon, camera, DVec3::Z).abs() < 1e-9);
    }
}

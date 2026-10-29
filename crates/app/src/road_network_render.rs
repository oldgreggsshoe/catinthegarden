//! Shared near/far pipelines, bounded uploads and atomic mesh/field publication.
use crate::{
    road_network_field::Field, road_network_mesh::Snapshot, road_network_stream::Worker,
    terrain::TerrainRenderer,
};
use glam::DVec3;
use std::{path::PathBuf, sync::Arc};

pub fn enabled() -> bool {
    match std::env::var("PLANET_ROAD_NETWORK") {
        Ok(v) => matches!(v.trim(), "1" | "true" | "on"),
        Err(_) => crate::road::enabled(),
    }
}
pub fn initial_position(interactive: bool) -> DVec3 {
    initial_position_for_site(
        std::env::var("PLANET_ROAD_NETWORK_SITE").ok().as_deref(),
        interactive,
    )
}
fn initial_position_for_site(site: Option<&str>, interactive: bool) -> DVec3 {
    if site == Some("coastal_exit") {
        return DVec3::new(
            0.9659268192718296,
            0.18088239257812766,
            -0.18511331628658895,
        ) * (crate::planet::planet_radius_meters() + 19.954);
    }
    if site == Some("longest_undersea") || (site.is_none() && interactive) {
        return longest_undersea_start_position();
    }
    let direction = match site {
        Some("village") => {
            DVec3::new(2574139.618674488, -2554011.900402223, 1699659.6951081678).normalize()
        }
        Some("undersea") => return undersea_start_position(),
        Some("manual_snow") => {
            DVec3::new(1804537.8505246886, 2191767.3184277415, 2834020.239721973).normalize()
        }
        Some("manual_corners") => {
            DVec3::new(1818372.4030723977, 2163321.697198833, 2842753.1646584594).normalize()
        }
        _ => crate::road::up(),
    };
    direction * (crate::planet::planet_radius_meters() + 10_000.0)
}
/// Entrance to the longest completed regional sea component in the current
/// survey (PositiveX 8/152/152, formerly 19,144.74m over sea). Pinned, not a
/// per-launch scan or a global length claim. Coastal engineering now carries
/// the bore inland beneath the beach. The worker finds its actual dry approach.
pub(crate) fn longest_undersea_start_position() -> DVec3 {
    DVec3::new(0.965931572796729, 0.18451645492986005, -0.1814659046062214)
        * (crate::planet::planet_radius_meters() + 10.252)
}

pub(crate) fn undersea_start_position() -> DVec3 {
    let r = crate::planet::planet_radius_meters();
    let gateway = DVec3::new(-0.674991988920733, 0.2979456826095423, -0.674991988920733);
    let centre = crate::road_network::direction(
        planet_coretypes::TileKey {
            face: planet_coretypes::CubeFace::NegativeX,
            level: crate::road_network::REGION_LEVEL,
            x: 0,
            y: 184,
        },
        0.5,
        0.5,
    );
    gateway.lerp(centre, 0.001).normalize() * (r - 30.0)
}
const VERTEX_UPLOAD_BYTES: usize = 1024 * 1024;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    origin: [f32; 4],
    across: [f32; 4],
    forward: [f32; 4],
    up: [f32; 4],
    material_origin: [f32; 4],
}
struct Scene {
    snapshot: Snapshot,
    lighting: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    visible: Vec<(bool, bool)>,
}
struct Upload {
    generation: u64,
    scene: Scene,
    copied: usize,
}
pub struct NetworkRenderer {
    worker: Worker,
    material: crate::road_material::Material,
    active: Scene,
    pending: Option<Upload>,
    pipeline: wgpu::RenderPipeline,
    far_pipeline: wgpu::RenderPipeline,
    instance: wgpu::Buffer,
    inspection: Option<(DVec3, DVec3)>,
    signs: crate::road_signs::Renderer,
    overlay: crate::road_overlay::Renderer,
    active_generation: u64,
    pending_signs: Option<(u64, crate::road_signs::Mesh)>,
    sign_pose: Option<(DVec3, DVec3)>,
    traffic: crate::traffic::Controller,
    traffic_renderer: crate::traffic_render::Renderer,
}
impl NetworkRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        camera: &wgpu::BindGroupLayout,
        terrain: &mut TerrainRenderer,
        root: PathBuf,
        initial: DVec3,
        traffic_active: bool,
    ) -> Result<Self, String> {
        let (mut worker, snapshot) = Worker::start(root, initial)?;
        let inspection = snapshot.inspection;
        let centre = snapshot.build_centre;
        terrain.prepare_flight_start_surface_height_meters(
            centre.normalize(),
            centre.length() - crate::planet::planet_radius_meters(),
        );
        let pipeline = crate::road_render::create_network_pipeline(
            device,
            format,
            camera,
            terrain.shared_bind_group_layout(),
            false,
        );
        let far_pipeline = crate::road_render::create_network_pipeline(
            device,
            format,
            camera,
            terrain.shared_bind_group_layout(),
            true,
        );
        let instance = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("network camera-relative frame"),
            size: size_of::<Instance>() as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let overlay = crate::road_overlay::Renderer::new(device, format, camera, &instance);
        let material = crate::road_material::Material::new(device, queue);
        let mut active = Self::allocate(device, snapshot, &material);
        if !active.snapshot.vertices.is_empty() {
            queue.write_buffer(
                &active.vertices,
                0,
                bytemuck::cast_slice(&active.snapshot.vertices),
            );
        }
        drop(std::mem::take(&mut active.snapshot.vertices));
        terrain.set_network_road_cuts(active.snapshot.field.clone());
        terrain.set_village_parking(&active.snapshot.field.village_parking);
        tracing::info!(target:"planet::roads",source_version=worker.source_version,
            villages=active.snapshot.villages,regions=active.snapshot.keys.len(),segments=active.snapshot.segments,
            field_segments=active.snapshot.field.segments.len(),
            "road network startup region ready");
        worker.request(centre, true);
        let signs = crate::road_signs::Renderer::new(device, queue, format, camera);
        let mut renderer = Self {
            signs,
            overlay,
            active_generation: 0,
            pending_signs: None,
            sign_pose: None,
            traffic: crate::traffic::Controller::new(traffic_active),
            traffic_renderer: crate::traffic_render::Renderer::new(device, queue, format, camera),
            worker,
            material,
            active,
            pending: None,
            pipeline,
            far_pipeline,
            instance,
            inspection,
        };
        if std::env::var("PLANET_TRAFFIC_WAIT").as_deref() == Ok("1") {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            loop {
                let (eye, target) = renderer.inspection.unwrap_or((centre, centre + DVec3::Y));
                renderer.update_traffic(queue, 0.0, eye, (target - eye).normalize_or_zero(), None);
                if renderer.traffic.ready() {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    return Err("traffic readiness deadline".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        // Automated sign views must wait on the data, not assume that a fixed
        // number of rendered frames implies the worker has finished. Normal
        // interactive startup remains asynchronous.
        if std::env::var("PLANET_ROAD_SIGNS_WAIT").as_deref() == Ok("1") {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
            while renderer.signs.inspection.is_none() {
                renderer.prepare(device, queue, terrain);
                if std::time::Instant::now() > deadline {
                    return Err("road sign readiness deadline".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Ok(renderer)
    }
    fn allocate(
        device: &wgpu::Device,
        snapshot: Snapshot,
        material: &crate::road_material::Material,
    ) -> Scene {
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("streamed static network geometry"),
            size: (snapshot.vertices.len() * size_of::<crate::road::Vertex>()).max(36) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let visible = vec![(false, false); snapshot.chunks.len()];
        let lighting = crate::road_lighting::binding(device, &snapshot.lighting, material);
        Scene {
            lighting,
            snapshot,
            vertices,
            visible,
        }
    }
    pub fn plan(&self) -> &Arc<Field> {
        &self.active.snapshot.field
    }
    pub fn enclosed_view(
        &self,
        eye: DVec3,
        forward: DVec3,
        up: DVec3,
        fov: f64,
        aspect: f64,
    ) -> bool {
        self.active.snapshot.visibility.enclosed(
            &self.active.snapshot.field,
            eye,
            forward,
            up,
            fov,
            aspect,
        )
    }
    pub fn inspection_pose(&self) -> Option<(DVec3, DVec3)> {
        self.inspection
    }
    pub fn scenario_pose(&self, name: &str) -> Option<(DVec3, DVec3)> {
        let (eye, target) = self.inspection?;
        match name {
            "road_network_walk" if eye.length() < crate::planet::planet_radius_meters() => {
                // The sea preset begins just inside a shared gateway. Walk
                // across that cube-face seam, rather than farther inland.
                Some((eye, eye - (target - eye)))
            }
            "road_network_trial" | "road_network_walk" => Some((eye, target)),
            "road_network_signs" => self.sign_pose.or(Some((eye, target))),
            "road_traffic" => self.traffic.inspection_pose(false),
            "road_traffic_tunnel" => self.traffic.inspection_pose(true),
            "road_network_parking" => {
                let up = eye.normalize();
                let forward = (target - eye).normalize();
                let side = forward.cross(up).normalize();
                Some((
                    eye - forward * 50.0 + side * 28.0 + up * 42.0,
                    eye - forward * 25.0,
                ))
            }
            "road_network_overview" => {
                let up = eye.normalize();
                let forward = (target - eye).normalize();
                let side = forward.cross(up).normalize();
                Some((eye - side * 900.0 + up * 500.0, eye + forward * 250.0))
            }
            _ => None,
        }
    }
    /// Called before movement, camera uniforms or forest generation. Old
    /// meshes/CPU/GPU fields remain a consistent set until the final upload.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &mut TerrainRenderer,
    ) {
        self.overlay.prepare(
            device,
            self.active_generation,
            &self.active.snapshot.lines,
            &self.active.snapshot.overview,
        );
        if self
            .pending
            .as_ref()
            .is_some_and(|u| !self.worker.current(u.generation))
        {
            self.pending = None;
        }
        if let Some((generation, result)) = self.worker.take() {
            if self.worker.current(generation) {
                match result {
                    Ok(snapshot) => {
                        self.pending = Some(Upload {
                            generation,
                            scene: Self::allocate(device, snapshot, &self.material),
                            copied: 0,
                        })
                    }
                    Err(error) => {
                        tracing::warn!(target:"planet::roads",%error,"road region engineering rejected; retaining previous complete resident set")
                    }
                }
            }
        }
        if let Some(signs) = self.worker.take_signs() {
            self.pending_signs = Some(signs);
        }
        if self
            .pending_signs
            .as_ref()
            .is_some_and(|(g, _)| !self.worker.current(*g))
        {
            self.pending_signs = None;
        }
        if self
            .pending_signs
            .as_ref()
            .is_some_and(|(g, _)| *g == self.active_generation)
        {
            let (_, mesh) = self.pending_signs.take().unwrap();
            self.sign_pose = self.sign_pose.or(mesh.inspection);
            self.signs.upload(device, queue, mesh);
        }
        let Some(upload) = &mut self.pending else {
            return;
        };
        let data = bytemuck::cast_slice::<crate::road::Vertex, u8>(&upload.scene.snapshot.vertices);
        let end = (upload.copied + VERTEX_UPLOAD_BYTES).min(data.len());
        if end > upload.copied {
            queue.write_buffer(
                &upload.scene.vertices,
                upload.copied as u64,
                &data[upload.copied..end],
            );
        }
        upload.copied = end;
        if end == data.len() {
            let generation = upload.generation;
            let mut complete = self.pending.take().unwrap().scene;
            drop(std::mem::take(&mut complete.snapshot.vertices));
            terrain.set_network_road_cuts(complete.snapshot.field.clone());
            terrain.set_village_parking(&complete.snapshot.field.village_parking);
            tracing::info!(target:"planet::roads",regions=complete.snapshot.keys.len(),villages=complete.snapshot.villages,
                segments=complete.snapshot.segments,tunnel_segments=complete.snapshot.tunnels,
                field_segments=complete.snapshot.field.segments.len(),"road network resident set published");
            self.active_generation = generation;
            self.signs.clear();
            self.active = complete;
            self.overlay.prepare(
                device,
                self.active_generation,
                &self.active.snapshot.lines,
                &self.active.snapshot.overview,
            );
        }
    }
    pub fn update_traffic(
        &mut self,
        queue: &wgpu::Queue,
        dt: f64,
        eye: DVec3,
        forward: DVec3,
        player: Option<DVec3>,
    ) {
        self.traffic.update(
            &self.active.snapshot.field, &self.active.snapshot.lighting,
            dt, eye, forward, player,
        );
        self.traffic_renderer.update(queue, &self.traffic, &self.active.snapshot.field, eye, forward);
    }
    pub fn traffic_stats(&self) -> (usize, f64) {
        self.traffic.stats()
    }
    pub fn traffic_blocks_motion(&self, from: DVec3, to: DVec3, heading: DVec3) -> bool {
        self.traffic.blocks_motion(from, to, heading)
    }
    pub fn update_camera(
        &mut self,
        queue: &wgpu::Queue,
        eye: DVec3,
        forward: DVec3,
        viewport: [u32; 2],
    ) {
        if eye.length() - crate::planet::planet_radius_meters() < 50_000.0 {
            self.worker.request(eye, false);
        }
        for (chunk, state) in self
            .active
            .snapshot
            .chunks
            .iter()
            .zip(&mut self.active.visible)
        {
            let relative = chunk.centre - eye;
            state.0 = relative.length() < 25_000.0 + chunk.radius
                && relative.dot(forward) > -chunk.radius;
            state.1 = far_lod(relative.length(), chunk.radius, chunk.near.is_empty());
        }
        self.overlay.update_camera(queue, eye, viewport);
        let f = self.active.snapshot.field.frame;
        let v = |p: DVec3| [p.x as f32, p.y as f32, p.z as f32, 0.0];
        let mut frame = Instance {
            origin: v(f.up * f.radius - eye),
            across: v(f.across),
            forward: v(f.forward),
            up: v(f.up),
            material_origin: crate::road_render::material_origin(f.up * f.radius),
        };
        frame.origin[3] = viewport[0] as f32;
        frame.across[3] = viewport[1] as f32;
        queue.write_buffer(&self.instance, 0, bytemuck::bytes_of(&frame));
    }
    pub fn toggle_lines(&mut self) -> bool {
        self.overlay.toggle()
    }
    pub fn draw_lines<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: &'a wgpu::BindGroup) {
        self.overlay.draw(pass, camera);
    }
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera: &'a wgpu::BindGroup,
        shared: &'a wgpu::BindGroup,
        far: bool,
    ) {
        if far && !crate::subsystem_enabled("road_far") {
            return;
        }
        pass.set_pipeline(if far {
            &self.far_pipeline
        } else {
            &self.pipeline
        });
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, &self.active.lighting, &[]);
        pass.set_bind_group(2, shared, &[]);
        pass.set_vertex_buffer(0, self.active.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instance.slice(..));
        // Coalesce contiguous visible ranges just as the pilot does.
        let mut pending: Option<std::ops::Range<u32>> = None;
        for (chunk, &(visible, distant)) in
            self.active.snapshot.chunks.iter().zip(&self.active.visible)
        {
            let range = if far { &chunk.far } else { &chunk.near };
            if !visible || distant != far || range.is_empty() {
                if let Some(r) = pending.take() {
                    pass.draw(r, 0..1);
                }
                continue;
            }
            if let Some(p) = &mut pending {
                if p.end == range.start {
                    p.end = range.end;
                    continue;
                }
                pass.draw(p.clone(), 0..1);
            }
            pending = Some(range.clone());
        }
        if let Some(r) = pending {
            pass.draw(r, 0..1);
        }
        if !far {
            for (chunk, &(visible, _)) in
                self.active.snapshot.chunks.iter().zip(&self.active.visible)
            {
                if visible && !chunk.structures.is_empty() {
                    pass.draw(chunk.structures.clone(), 0..1);
                }
            }
            if crate::subsystem_enabled("road_signs") {
                self.signs.draw(pass, camera, &self.instance);
            }
            self.traffic_renderer.draw(pass, camera, &self.active.lighting);
        }
    }
}

fn far_lod(distance: f64, radius: f64, near_empty: bool) -> bool {
    distance - radius > 4000.0 || near_empty
}
#[cfg(test)]
mod tests {
    #[test]
    fn near_decks_do_not_become_far_ribbons_when_the_batch_centre_is_far_away() {
        assert!(!super::far_lod(10000.0, 9990.0, false));
        assert!(!super::far_lod(4500.0, 600.0, false));
        assert!(super::far_lod(4700.0, 600.0, false));
        assert!(super::far_lod(100.0, 50.0, true));
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;
    #[test]
    fn road_default_interactive_site_is_the_verified_undersea_entrance() {
        assert_eq!(
            initial_position_for_site(None, true),
            longest_undersea_start_position()
        );
        assert_eq!(
            initial_position_for_site(Some("longest_undersea"), false),
            longest_undersea_start_position()
        );
        let key = planet_coretypes::tile_key_for_direction(
            longest_undersea_start_position(),
            crate::road_network::REGION_LEVEL,
        );
        assert_eq!(
            (key.face, key.x, key.y),
            (planet_coretypes::CubeFace::PositiveX, 152, 152)
        );
    }
    #[test]
    fn road_replay_and_explicit_legacy_sites_retain_their_starts() {
        let mountain = crate::road::up() * (crate::planet::planet_radius_meters() + 10_000.0);
        assert_eq!(initial_position_for_site(None, false), mountain);
        assert_eq!(initial_position_for_site(Some("mountain"), true), mountain);
        assert_eq!(
            initial_position_for_site(Some("undersea"), true),
            undersea_start_position()
        );
        assert_ne!(
            initial_position_for_site(Some("village"), true),
            longest_undersea_start_position()
        );
    }
}

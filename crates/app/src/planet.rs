use glam::{DVec3, Mat4, Vec3, Vec4};

pub const PLANET_RADIUS_METERS: f64 = 4_000_000.0;
const SUBDIVISIONS: usize = 32;

/// A leaf in one of the six face-local quadtrees. Coordinates address a node
/// at `level`, so children are `(x * 2 + dx, y * 2 + dy)` at `level + 1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct QuadtreeNode {
    pub face: u8,
    pub level: u8,
    pub x: u32,
    pub y: u32,
}

impl QuadtreeNode {
    pub const fn root(face: u8) -> Self {
        Self {
            face,
            level: 0,
            x: 0,
            y: 0,
        }
    }

    pub fn children(self) -> [Self; 4] {
        let level = self.level + 1;
        let x = self.x * 2;
        let y = self.y * 2;
        [
            Self {
                face: self.face,
                level,
                x,
                y,
            },
            Self {
                face: self.face,
                level,
                x: x + 1,
                y,
            },
            Self {
                face: self.face,
                level,
                x,
                y: y + 1,
            },
            Self {
                face: self.face,
                level,
                x: x + 1,
                y: y + 1,
            },
        ]
    }
}

/// Screen-space LOD policy shared by all face trees. The hysteresis band
/// prevents a node from split/merge thrashing at the split boundary.
pub struct LodPolicy {
    pub split_pixels: f64,
    pub merge_pixels: f64,
    pub max_level: u8,
}

impl Default for LodPolicy {
    fn default() -> Self {
        Self {
            split_pixels: 2.0,
            merge_pixels: 1.25,
            max_level: 8,
        }
    }
}

impl LodPolicy {
    pub fn should_split(&self, projected_error_pixels: f64, level: u8) -> bool {
        level < self.max_level && projected_error_pixels > self.split_pixels
    }

    pub fn should_merge(&self, projected_error_pixels: f64) -> bool {
        projected_error_pixels < self.merge_pixels
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RebasedVertex {
    pub camera_relative_position: [f32; 3],
}

impl RebasedVertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub struct CubeSphereMesh {
    world_positions: Vec<DVec3>,
    indices: Vec<u32>,
}

impl CubeSphereMesh {
    pub fn new() -> Self {
        let faces = [
            (DVec3::X, -DVec3::Z, DVec3::Y),
            (-DVec3::X, DVec3::Z, DVec3::Y),
            (DVec3::Y, DVec3::X, -DVec3::Z),
            (-DVec3::Y, DVec3::X, DVec3::Z),
            (DVec3::Z, DVec3::X, DVec3::Y),
            (-DVec3::Z, -DVec3::X, DVec3::Y),
        ];
        let vertices_per_face = (SUBDIVISIONS + 1) * (SUBDIVISIONS + 1);
        let mut world_positions = Vec::with_capacity(faces.len() * vertices_per_face);
        let mut indices = Vec::with_capacity(faces.len() * SUBDIVISIONS * SUBDIVISIONS * 6);

        for (face_index, (normal, tangent_u, tangent_v)) in faces.into_iter().enumerate() {
            let face_start = (face_index * vertices_per_face) as u32;
            for y in 0..=SUBDIVISIONS {
                let v = y as f64 / SUBDIVISIONS as f64 * 2.0 - 1.0;
                for x in 0..=SUBDIVISIONS {
                    let u = x as f64 / SUBDIVISIONS as f64 * 2.0 - 1.0;
                    world_positions.push(
                        (normal + tangent_u * u + tangent_v * v).normalize() * PLANET_RADIUS_METERS,
                    );
                }
            }
            for y in 0..SUBDIVISIONS {
                for x in 0..SUBDIVISIONS {
                    let lower_left = face_start + (y * (SUBDIVISIONS + 1) + x) as u32;
                    let lower_right = lower_left + 1;
                    let upper_left = lower_left + (SUBDIVISIONS + 1) as u32;
                    let upper_right = upper_left + 1;
                    indices.extend_from_slice(&[
                        lower_left,
                        lower_right,
                        upper_left,
                        lower_right,
                        upper_right,
                        upper_left,
                    ]);
                }
            }
        }

        Self {
            world_positions,
            indices,
        }
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn rebased_vertices(&self, camera_world_position: DVec3) -> Vec<RebasedVertex> {
        self.world_positions
            .iter()
            .map(|world_position| RebasedVertex {
                camera_relative_position: (*world_position - camera_world_position)
                    .as_vec3()
                    .to_array(),
            })
            .collect()
    }

    #[cfg(test)]
    fn world_positions(&self) -> &[DVec3] {
        &self.world_positions
    }
}

pub struct OrbitCamera {
    pub azimuth_radians: f64,
    pub elevation_radians: f64,
    pub orbit_radius_meters: f64,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            azimuth_radians: 0.0,
            elevation_radians: 20.0_f64.to_radians(),
            orbit_radius_meters: 10_000_000.0,
        }
    }
}

impl OrbitCamera {
    pub fn world_position(&self) -> DVec3 {
        let horizontal_radius = self.orbit_radius_meters * self.elevation_radians.cos();
        DVec3::new(
            horizontal_radius * self.azimuth_radians.cos(),
            self.orbit_radius_meters * self.elevation_radians.sin(),
            horizontal_radius * self.azimuth_radians.sin(),
        )
    }

    pub fn view_projection(&self, aspect_ratio: f32) -> Mat4 {
        let world_position = self.world_position();
        let forward = (-world_position).normalize().as_vec3();
        let up = DVec3::Y.as_vec3();
        let view = Mat4::look_to_rh(Vec3::ZERO, forward, up);
        let altitude = self.orbit_radius_meters - PLANET_RADIUS_METERS;
        let near = (altitude * 0.01).clamp(0.05, 10.0) as f32;
        reversed_z_infinite_perspective(45.0_f32.to_radians(), aspect_ratio, near) * view
    }

    pub fn rotate(&mut self, azimuth_delta: f64, elevation_delta: f64) {
        self.azimuth_radians += azimuth_delta;
        self.elevation_radians = (self.elevation_radians + elevation_delta).clamp(-1.45, 1.45);
    }

    pub fn zoom(&mut self, wheel_delta: f64) {
        let minimum_radius = PLANET_RADIUS_METERS + 10.0;
        self.orbit_radius_meters = (self.orbit_radius_meters * (-wheel_delta * 0.12).exp())
            .clamp(minimum_radius, PLANET_RADIUS_METERS * 20.0);
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub view_projection: [[f32; 4]; 4],
}

impl CameraUniform {
    pub fn from_camera(camera: &OrbitCamera, aspect_ratio: f32) -> Self {
        Self {
            view_projection: camera.view_projection(aspect_ratio).to_cols_array_2d(),
        }
    }
}

fn reversed_z_infinite_perspective(
    vertical_fov_radians: f32,
    aspect_ratio: f32,
    near: f32,
) -> Mat4 {
    let focal_length = 1.0 / (vertical_fov_radians * 0.5).tan();
    Mat4::from_cols(
        Vec4::new(focal_length / aspect_ratio, 0.0, 0.0, 0.0),
        Vec4::new(0.0, focal_length, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::{CubeSphereMesh, LodPolicy, OrbitCamera, PLANET_RADIUS_METERS, QuadtreeNode};

    #[test]
    fn quadtree_children_tile_the_parent_node() {
        let children = QuadtreeNode::root(3).children();
        assert_eq!(
            children[0],
            QuadtreeNode {
                face: 3,
                level: 1,
                x: 0,
                y: 0
            }
        );
        assert_eq!(
            children[3],
            QuadtreeNode {
                face: 3,
                level: 1,
                x: 1,
                y: 1
            }
        );
        let policy = LodPolicy::default();
        assert!(policy.should_split(2.1, 0));
        assert!(policy.should_merge(1.0));
    }

    #[test]
    fn cube_sphere_vertices_are_on_the_planet_radius() {
        let mesh = CubeSphereMesh::new();
        assert_eq!(mesh.world_positions().len(), 6 * 33 * 33);
        assert_eq!(mesh.indices().len(), 6 * 32 * 32 * 6);
        assert!(
            mesh.world_positions()
                .iter()
                .all(|position| (position.length() - PLANET_RADIUS_METERS).abs() < 0.001)
        );
    }

    #[test]
    fn rebasing_uploads_relative_f32_offsets() {
        let mesh = CubeSphereMesh::new();
        let camera = OrbitCamera::default();
        let camera_position = camera.world_position();
        let vertices = mesh.rebased_vertices(camera_position);
        assert!(vertices.iter().all(|vertex| {
            vertex
                .camera_relative_position
                .iter()
                .all(|value| value.is_finite())
        }));
        assert!(
            vertices
                .iter()
                .any(|vertex| vertex.camera_relative_position[0] < -1_000_000.0)
        );
    }

    #[test]
    fn zoom_clamps_at_the_ten_meter_minimum_altitude() {
        let mut camera = OrbitCamera::default();
        camera.zoom(1_000.0);
        assert_eq!(camera.orbit_radius_meters, PLANET_RADIUS_METERS + 10.0);
    }
}

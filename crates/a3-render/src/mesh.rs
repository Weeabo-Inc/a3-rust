//! Indexed triangle meshes: CPU data, procedural shapes and GPU buffers.
//!
//! Winding: front faces are clockwise as seen from outside, the Direct3D convention RV's
//! left-handed world uses. For CPU data this means `(b - a) x (c - a)` points outwards.

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;

/// One mesh vertex: position, normal, texture coordinate and tangent (xyz + handedness in w).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 4],
}

impl Vertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Mesh data on the CPU, in model space (metres, RV axes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Append a quad given its corners clockwise as seen from the front.
    pub fn push_quad(&mut self, corners: [Vec3; 4], normal: Vec3, tangent: Vec3) {
        let base = self.vertices.len() as u32;
        let uvs = [
            Vec2::new(0.0, 1.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
        ];
        for (p, uv) in corners.iter().zip(uvs) {
            self.vertices.push(Vertex {
                position: p.to_array(),
                normal: normal.to_array(),
                uv: uv.to_array(),
                tangent: tangent.extend(1.0).to_array(),
            });
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Axis-aligned box centred on the origin.
    pub fn cuboid(size: Vec3) -> MeshData {
        let h = size * 0.5;
        let mut mesh = MeshData::default();
        let faces = [
            (Vec3::X, Vec3::Y),
            (-Vec3::X, Vec3::Y),
            (Vec3::Z, Vec3::Y),
            (-Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z),
            (-Vec3::Y, -Vec3::Z),
        ];
        for (n, up) in faces {
            // Seen from outside (looking along -n), screen right is n x up.
            let right = n.cross(up);
            let c = n * h;
            let u = right * h;
            let v = up * h;
            mesh.push_quad([c - u - v, c - u + v, c + u + v, c + u - v], n, right);
        }
        mesh
    }

    /// Horizontal square of edge `size` at y = 0, facing up, with UVs repeated `uv_repeat`
    /// times.
    pub fn ground_plane(size: f32, uv_repeat: f32) -> MeshData {
        let h = size * 0.5;
        let mut mesh = MeshData::default();
        mesh.push_quad(
            [
                Vec3::new(-h, 0.0, -h),
                Vec3::new(-h, 0.0, h),
                Vec3::new(h, 0.0, h),
                Vec3::new(h, 0.0, -h),
            ],
            Vec3::Y,
            Vec3::X,
        );
        for v in &mut mesh.vertices {
            v.uv = (Vec2::from(v.uv) * uv_repeat).to_array();
        }
        mesh
    }

    /// Latitude/longitude sphere centred on the origin.
    pub fn uv_sphere(radius: f32, segments: u32, rings: u32) -> MeshData {
        let segments = segments.max(3);
        let rings = rings.max(2);
        let mut mesh = MeshData::default();
        for ring in 0..=rings {
            let v = ring as f32 / rings as f32;
            let theta = v * std::f32::consts::PI;
            for seg in 0..=segments {
                let u = seg as f32 / segments as f32;
                let phi = u * std::f32::consts::TAU;
                let n = Vec3::new(
                    theta.sin() * phi.sin(),
                    theta.cos(),
                    theta.sin() * phi.cos(),
                );
                let tangent = Vec3::new(phi.cos(), 0.0, -phi.sin());
                mesh.vertices.push(Vertex {
                    position: (n * radius).to_array(),
                    normal: n.to_array(),
                    uv: [u, v],
                    tangent: tangent.extend(1.0).to_array(),
                });
            }
        }
        let stride = segments + 1;
        for ring in 0..rings {
            for seg in 0..segments {
                let a = ring * stride + seg;
                let b = a + stride;
                mesh.push_triangle_outward(a, b, a + 1);
                mesh.push_triangle_outward(a + 1, b, b + 1);
            }
        }
        mesh
    }

    /// Push a triangle, flipping it if needed so its front faces along the vertex normals.
    /// Degenerate triangles (at the poles of a sphere) are skipped.
    fn push_triangle_outward(&mut self, a: u32, b: u32, c: u32) {
        let p = |i: u32| Vec3::from(self.vertices[i as usize].position);
        let n = |i: u32| Vec3::from(self.vertices[i as usize].normal);
        let face = (p(b) - p(a)).cross(p(c) - p(a));
        if face.length_squared() < 1e-12 {
            return;
        }
        if face.dot(n(a) + n(b) + n(c)) >= 0.0 {
            self.indices.extend_from_slice(&[a, b, c]);
        } else {
            self.indices.extend_from_slice(&[a, c, b]);
        }
    }

    /// Radius of the bounding sphere around the model origin.
    pub fn bounding_radius(&self) -> f32 {
        self.vertices
            .iter()
            .map(|v| Vec3::from(v.position).length())
            .fold(0.0, f32::max)
    }
}

/// A mesh uploaded to the GPU.
#[derive(Debug)]
pub struct Mesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub bounding_radius: f32,
}

impl Mesh {
    pub fn upload(device: &wgpu::Device, data: &MeshData, label: Option<&str>) -> Mesh {
        Mesh {
            vertex_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label,
                contents: bytemuck::cast_slice(&data.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            index_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label,
                contents: bytemuck::cast_slice(&data.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: data.indices.len() as u32,
            bounding_radius: data.bounding_radius(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle's geometric normal agrees with its vertex normals.
    fn assert_outward(mesh: &MeshData) {
        for tri in mesh.indices.chunks(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| Vec3::from(mesh.vertices[i as usize].position))
                .collect();
            let n = Vec3::from(mesh.vertices[tri[0] as usize].normal);
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            assert!(face.dot(n) > 0.0, "triangle {tri:?} faces inward");
        }
    }

    #[test]
    fn cuboid_has_six_outward_faces() {
        let m = MeshData::cuboid(Vec3::new(2.0, 4.0, 6.0));
        assert_eq!((m.vertices.len(), m.indices.len()), (24, 36));
        assert_outward(&m);
        assert!((m.bounding_radius() - Vec3::new(1.0, 2.0, 3.0).length()).abs() < 1e-5);
    }

    #[test]
    fn ground_plane_faces_up() {
        let m = MeshData::ground_plane(10.0, 4.0);
        assert_outward(&m);
        assert!(m.vertices.iter().all(|v| v.normal == [0.0, 1.0, 0.0]));
        assert!(m.vertices.iter().any(|v| v.uv == [4.0, 4.0]));
    }

    #[test]
    fn sphere_triangles_face_outward() {
        let m = MeshData::uv_sphere(3.0, 12, 8);
        assert_outward(&m);
        assert!((m.bounding_radius() - 3.0).abs() < 1e-5);
    }
}

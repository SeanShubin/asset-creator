use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use super::spec::PrimitiveShape;

// =====================================================================
// RawMesh — intermediate mesh representation for CSG and conversion
// =====================================================================

#[derive(Clone)]
pub struct RawMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl RawMesh {
    pub fn to_bevy_mesh(self) -> Mesh {
        build_mesh(self.positions, self.normals, self.uvs, self.indices)
    }

    /// Merge another RawMesh into this one (simple concatenation, no CSG).
    pub fn merge(&mut self, other: &RawMesh) {
        let offset = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.uvs.extend_from_slice(&other.uvs);
        self.indices.extend(other.indices.iter().map(|i| i + offset));
    }

    /// Apply an affine transform to all positions and normals.
    pub fn apply_transform(&mut self, tf: &Transform) {
        let mat = tf.compute_matrix();
        let normal_mat = mat.inverse().transpose();
        for pos in &mut self.positions {
            let p = mat.transform_point3(Vec3::from(*pos));
            *pos = [p.x, p.y, p.z];
        }
        for normal in &mut self.normals {
            let n = normal_mat.transform_vector3(Vec3::from(*normal)).normalize();
            *normal = [n.x, n.y, n.z];
        }
    }
}

/// Generate a RawMesh for any primitive shape.
pub fn create_raw_mesh(shape: PrimitiveShape) -> RawMesh {
    match shape {
        PrimitiveShape::Box => create_raw_box(),
        PrimitiveShape::Wedge => create_raw_wedge(),
        PrimitiveShape::Corner => create_raw_corner(),
    }
}

// =====================================================================
// Raw mesh builders for primitives that previously used Bevy built-ins
// =====================================================================

/// Unit box: 1x1x1 centered at origin.
fn create_raw_box() -> RawMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // +Y (top)
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5],
        [0.0, 1.0, 0.0]);
    // -Y (bottom)
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [-0.5, -0.5, -0.5],
        [0.0, -1.0, 0.0]);
    // +Z (front)
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, -0.5, 0.5],
        [0.0, 0.0, 1.0]);
    // -Z (back)
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, -0.5, -0.5],
        [0.0, 0.0, -1.0]);
    // +X (right)
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [0.5, -0.5, -0.5],
        [1.0, 0.0, 0.0]);
    // -X (left)
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [-0.5, -0.5, 0.5],
        [-1.0, 0.0, 0.0]);

    RawMesh { positions, normals, uvs, indices }
}

// =====================================================================
// Wedge — box that tapers to zero height on one edge
// =====================================================================

pub fn create_wedge_mesh() -> Mesh {
    create_raw_wedge().to_bevy_mesh()
}

fn create_raw_wedge() -> RawMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    let slope_normal = Vec3::new(0.0, 1.0, 1.0).normalize();

    // Bottom face
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5,  0.5], [ 0.5, -0.5,  0.5], [ 0.5, -0.5, -0.5], [-0.5, -0.5, -0.5],
        [0.0, -1.0, 0.0]);

    // Back face
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [ 0.5, -0.5, -0.5], [ 0.5,  0.5, -0.5], [-0.5,  0.5, -0.5],
        [0.0, 0.0, -1.0]);

    // Slope face
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5,  0.5, -0.5], [ 0.5,  0.5, -0.5], [ 0.5, -0.5,  0.5], [-0.5, -0.5,  0.5],
        [0.0, slope_normal.y, slope_normal.z]);

    // Left face (triangle)
    add_triangle(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [-0.5,  0.5, -0.5], [-0.5, -0.5,  0.5],
        [-1.0, 0.0, 0.0]);

    // Right face (triangle)
    add_triangle(&mut positions, &mut normals, &mut uvs, &mut indices,
        [ 0.5, -0.5, -0.5], [ 0.5, -0.5,  0.5], [ 0.5,  0.5, -0.5],
        [1.0, 0.0, 0.0]);

    RawMesh { positions, normals, uvs, indices }
}

// =====================================================================
// Corner — tetrahedron filling one corner of the bounding box
// =====================================================================

/// Unit corner: fills the (-X, -Y, -Z) corner of a 1x1x1 cube.
/// Three right-triangle faces on the cube walls + one diagonal face.
/// Orient/flip determines which corner the solid fills.
pub fn create_unit_corner() -> Mesh {
    create_raw_corner().to_bevy_mesh()
}

fn create_raw_corner() -> RawMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // Vertices of the tetrahedron:
    // A = (-0.5, -0.5, -0.5)  — the corner
    // B = ( 0.5, -0.5, -0.5)  — along +X
    // C = (-0.5,  0.5, -0.5)  — along +Y
    // D = (-0.5, -0.5,  0.5)  — along +Z

    // Bottom face (Y = -0.5): normal -Y
    add_triangle(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [0.5, -0.5, -0.5],
        [0.0, -1.0, 0.0],
    );

    // Back face (Z = -0.5): normal -Z
    add_triangle(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [-0.5, 0.5, -0.5],
        [0.0, 0.0, -1.0],
    );

    // Left face (X = -0.5): normal -X
    add_triangle(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, -0.5, 0.5],
        [-1.0, 0.0, 0.0],
    );

    // Diagonal face: normal (+1,+1,+1) normalized
    let diag_normal = Vec3::new(1.0, 1.0, 1.0).normalize();
    add_triangle(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, -0.5],
        [diag_normal.x, diag_normal.y, diag_normal.z],
    );

    RawMesh { positions, normals, uvs, indices }
}

// =====================================================================
// Shared helpers
// =====================================================================

fn add_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3],
    normal: [f32; 3],
) {
    let base = positions.len() as u32;
    positions.extend_from_slice(&[p0, p1, p2, p3]);
    normals.extend_from_slice(&[normal, normal, normal, normal]);
    uvs.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
}

fn add_triangle(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    p0: [f32; 3], p1: [f32; 3], p2: [f32; 3],
    normal: [f32; 3],
) {
    let base = positions.len() as u32;
    positions.extend_from_slice(&[p0, p1, p2]);
    normals.extend_from_slice(&[normal, normal, normal]);
    uvs.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
    indices.extend_from_slice(&[base, base + 2, base + 1]);
}

fn build_mesh(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

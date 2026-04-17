use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use super::spec::PrimitiveShape;

// =====================================================================
// RawMesh — intermediate mesh representation for CSG and conversion
// =====================================================================

#[derive(Clone, Debug, Default)]
pub struct RawMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Per-vertex RGBA color. Always the same length as `positions`.
    /// The fusion step fills this with the per-cell authored color so
    /// a single fused mesh can carry cells of different colors without
    /// needing multiple materials.
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl RawMesh {
    pub fn to_bevy_mesh(self) -> Mesh {
        build_mesh(self.positions, self.normals, self.uvs, self.colors, self.indices)
    }

    /// Append a template mesh transformed into world space, with every
    /// vertex stamped with the given color. Used by the fusion step to
    /// accumulate cell geometry into a per-part mesh.
    pub fn append_transformed(
        &mut self,
        template: &RawMesh,
        world_tf: &Transform,
        color: [f32; 4],
    ) {
        let mat = world_tf.compute_matrix();
        let normal_mat = mat.inverse().transpose();
        let base = self.positions.len() as u32;

        for pos in &template.positions {
            let p = mat.transform_point3(Vec3::from(*pos));
            self.positions.push([p.x, p.y, p.z]);
        }
        for normal in &template.normals {
            let n = normal_mat.transform_vector3(Vec3::from(*normal)).normalize();
            self.normals.push([n.x, n.y, n.z]);
        }
        for uv in &template.uvs {
            self.uvs.push(*uv);
        }
        for _ in 0..template.positions.len() {
            self.colors.push(color);
        }
        for idx in &template.indices {
            self.indices.push(base + idx);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// Generate a RawMesh for any primitive shape.
pub fn create_raw_mesh(shape: PrimitiveShape) -> RawMesh {
    match shape {
        PrimitiveShape::Box => create_raw_box(),
        PrimitiveShape::Wedge => create_raw_wedge(),
        PrimitiveShape::Corner => create_raw_corner(),
        PrimitiveShape::InverseCorner => create_raw_inverse_corner(),
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

    RawMesh { positions, normals, uvs, colors: Vec::new(), indices }
}

// =====================================================================
// Wedge — box that tapers to zero height on one edge
// =====================================================================

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

    RawMesh { positions, normals, uvs, colors: Vec::new(), indices }
}

// =====================================================================
// Corner — tetrahedron filling one corner of the bounding box
// =====================================================================

/// Unit corner: fills the (-X, -Y, -Z) corner of a 1x1x1 cube.
/// Three right-triangle faces on the cube walls + one diagonal face.
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

    RawMesh { positions, normals, uvs, colors: Vec::new(), indices }
}

// =====================================================================
// InverseCorner — box with one corner clipped
// =====================================================================

/// Unit inverse corner: fills the cube minus the (-X, -Y, -Z) tetrahedron.
/// Complement of Corner. 10 triangles: 3 full quads (+X, +Y, +Z),
/// 3 clipped triangles (-X, -Y, -Z), 1 diagonal face.
fn create_raw_inverse_corner() -> RawMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // Key vertices:
    // B = ( 0.5, -0.5, -0.5)  — on clip plane, +X from clipped vertex
    // C = (-0.5,  0.5, -0.5)  — on clip plane, +Y
    // D = (-0.5, -0.5,  0.5)  — on clip plane, +Z
    // E = ( 0.5,  0.5, -0.5)
    // F = ( 0.5, -0.5,  0.5)
    // G = (-0.5,  0.5,  0.5)
    // H = ( 0.5,  0.5,  0.5)

    // +X face: quad B, F, H, E — normal +X
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5],
        [1.0, 0.0, 0.0]);

    // +Y face: quad C, E, H, G — normal +Y
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5],
        [0.0, 1.0, 0.0]);

    // +Z face: quad D, G, H, F — normal +Z
    // (0,2,1) → D,H,G. Cross = (H-D)×(G-D) = (1,1,0)×(0,1,0) = (0,0,1) ✓
    add_quad(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, -0.5, 0.5],
        [0.0, 0.0, 1.0]);

    // -X face (clipped): triangle D, C, G — normal -X
    // (0,2,1) → D,G,C. Cross = (G-D)×(C-D) = (0,1,0)×(0,1,-1) = (-1,0,0) ✓
    add_triangle(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, 0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5],
        [-1.0, 0.0, 0.0]);

    // -Y face (clipped): triangle F, B, D — normal -Y
    // (0,2,1) → F,D,B. Cross = (D-F)×(B-F) = (-1,0,0)×(0,0,-1) = (0,-1,0) ✓
    add_triangle(&mut positions, &mut normals, &mut uvs, &mut indices,
        [0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [-0.5, -0.5, 0.5],
        [0.0, -1.0, 0.0]);

    // -Z face (clipped): triangle C, B, E — normal -Z
    // (0,2,1) → C,E,B. Cross = (E-C)×(B-C) = (1,0,0)×(1,-1,0) = (0,0,-1) ✓
    add_triangle(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, 0.5, -0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5],
        [0.0, 0.0, -1.0]);

    // Diagonal face: triangle D, B, C — normal toward clipped vertex (-1,-1,-1)
    // (0,2,1) → D,C,B. Cross = (C-D)×(B-D) = (-1,-1,-1) ✓
    let diag_normal = Vec3::new(-1.0, -1.0, -1.0).normalize();
    add_triangle(&mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [-0.5, 0.5, -0.5],
        [diag_normal.x, diag_normal.y, diag_normal.z]);

    RawMesh { positions, normals, uvs, colors: Vec::new(), indices }
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
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if !colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

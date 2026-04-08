use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

/// Generates a dome mesh: a sphere cap with a given base radius and peak height.
/// The base sits at y=0, the peak rises to y=height.
pub fn create_dome_mesh(base_radius: f32, height: f32, rings: u32, segments: u32) -> Mesh {
    let sphere_radius = compute_sphere_radius(base_radius, height);
    let y_offset = sphere_radius - height;

    let (positions, normals, uvs) = generate_dome_vertices(
        base_radius, sphere_radius, y_offset, rings, segments,
    );
    let indices = generate_dome_indices(rings, segments);

    build_mesh(positions, normals, uvs, indices)
}

fn compute_sphere_radius(base_radius: f32, height: f32) -> f32 {
    (base_radius * base_radius + height * height) / (2.0 * height)
}

fn generate_dome_vertices(
    base_radius: f32,
    sphere_radius: f32,
    y_offset: f32,
    rings: u32,
    segments: u32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>) {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();

    for ring in 0..=rings {
        let t = ring as f32 / rings as f32;
        let r = t * base_radius;
        let y = cap_height_at_distance(r, sphere_radius, y_offset);

        for seg in 0..=segments {
            let angle = seg as f32 / segments as f32 * std::f32::consts::TAU;
            let x = r * angle.cos();
            let z = r * angle.sin();

            positions.push([x, y, z]);

            let nx = x / sphere_radius;
            let ny = (y + y_offset) / sphere_radius;
            let nz = z / sphere_radius;
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            normals.push([nx / len, ny / len, nz / len]);

            uvs.push([seg as f32 / segments as f32, t]);
        }
    }

    (positions, normals, uvs)
}

fn cap_height_at_distance(distance: f32, sphere_radius: f32, y_offset: f32) -> f32 {
    let d_clamped = distance.min(sphere_radius);
    (sphere_radius * sphere_radius - d_clamped * d_clamped).sqrt() - y_offset
}

fn generate_dome_indices(rings: u32, segments: u32) -> Vec<u32> {
    let mut indices = Vec::new();
    let verts_per_ring = segments + 1;

    for ring in 0..rings {
        for seg in 0..segments {
            let a = ring * verts_per_ring + seg;
            let b = a + 1;
            let c = a + verts_per_ring;
            let d = c + 1;

            indices.extend_from_slice(&[a, b, c]);
            indices.extend_from_slice(&[b, d, c]);
        }
    }

    indices
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

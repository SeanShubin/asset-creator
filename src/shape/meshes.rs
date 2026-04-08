use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

// =====================================================================
// Dome — convex sphere cap, normals pointing outward
// =====================================================================

pub fn create_dome_mesh(base_radius: f32, height: f32, rings: u32, segments: u32) -> Mesh {
    let sphere_radius = compute_sphere_radius(base_radius, height);
    let y_offset = sphere_radius - height;
    let center_offset = height / 2.0;

    let (mut positions, mut normals, mut uvs) = generate_cap_vertices(
        base_radius, sphere_radius, y_offset, center_offset, 1.0, rings, segments,
    );
    let mut indices = generate_grid_indices(rings, segments, false);

    // Add a flat bottom disc to close the dome
    add_disc(&mut positions, &mut normals, &mut uvs, &mut indices,
        base_radius, -center_offset, segments, false);

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Bowl — concave sphere cap, normals pointing inward
// =====================================================================

pub fn create_bowl_mesh(base_radius: f32, depth: f32, rings: u32, segments: u32) -> Mesh {
    let sphere_radius = compute_sphere_radius(base_radius, depth);
    let y_offset = sphere_radius - depth;
    let center_offset = depth / 2.0;

    let (mut positions, mut normals, mut uvs) = generate_cap_vertices(
        base_radius, sphere_radius, y_offset, center_offset, -1.0, rings, segments,
    );
    // Reverse winding so inside face renders
    let mut indices = generate_grid_indices(rings, segments, true);

    // Add a flat rim disc at the top (y = center_offset) to close the bowl
    add_disc(&mut positions, &mut normals, &mut uvs, &mut indices,
        base_radius, center_offset, segments, true);

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Cone — tapers from oval base to a point
// =====================================================================

pub fn create_cone_mesh(rings: u32, segments: u32) -> Mesh {
    // Unit cone: base radius 0.5 at y=-0.5, point at y=0.5
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();

    let slope = (0.5_f32).atan2(1.0); // angle of the cone surface

    for ring in 0..=rings {
        let t = ring as f32 / rings as f32;
        let y = -0.5 + t;
        let r = 0.5 * (1.0 - t); // radius shrinks linearly to 0

        for seg in 0..=segments {
            let angle = seg as f32 / segments as f32 * std::f32::consts::TAU;
            let x = r * angle.cos();
            let z = r * angle.sin();

            positions.push([x, y, z]);

            // Normal: outward from the cone surface
            let nx = angle.cos() * slope.cos();
            let ny = slope.sin();
            let nz = angle.sin() * slope.cos();
            normals.push([nx, ny, nz]);

            uvs.push([seg as f32 / segments as f32, t]);
        }
    }

    // Bottom cap center
    let bottom_center = positions.len() as u32;
    positions.push([0.0, -0.5, 0.0]);
    normals.push([0.0, -1.0, 0.0]);
    uvs.push([0.5, 0.0]);

    let mut indices = generate_grid_indices(rings, segments, false);

    // Bottom cap triangles
    let verts_per_ring = segments + 1;
    for seg in 0..segments {
        indices.extend_from_slice(&[bottom_center, seg + 1, seg]);
    }

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Wedge — box that tapers to zero height on one edge
// =====================================================================

pub fn create_wedge_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    let slope_normal = Vec3::new(0.0, 1.0, 1.0).normalize();

    // Bottom face (y = -0.5)
    add_quad(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5,  0.5], [ 0.5, -0.5,  0.5], [ 0.5, -0.5, -0.5], [-0.5, -0.5, -0.5],
        [0.0, -1.0, 0.0],
    );

    // Back face (z = -0.5)
    add_quad(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [ 0.5, -0.5, -0.5], [ 0.5,  0.5, -0.5], [-0.5,  0.5, -0.5],
        [0.0, 0.0, -1.0],
    );

    // Slope face (top, from back-top to front-bottom)
    add_quad(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5,  0.5, -0.5], [ 0.5,  0.5, -0.5], [ 0.5, -0.5,  0.5], [-0.5, -0.5,  0.5],
        [0.0, slope_normal.y, slope_normal.z],
    );

    // Left face (x = -0.5) — triangle
    add_triangle(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [-0.5, -0.5, -0.5], [-0.5,  0.5, -0.5], [-0.5, -0.5,  0.5],
        [-1.0, 0.0, 0.0],
    );

    // Right face (x = 0.5) — triangle
    add_triangle(
        &mut positions, &mut normals, &mut uvs, &mut indices,
        [ 0.5, -0.5, -0.5], [ 0.5, -0.5,  0.5], [ 0.5,  0.5, -0.5],
        [1.0, 0.0, 0.0],
    );

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Torus — ring/donut shape
// =====================================================================

pub fn create_torus_mesh(ring_segments: u32, cross_segments: u32) -> Mesh {
    // Unit torus: major radius 0.35, minor radius 0.15
    // Fits within a 1x0.3x1 bounding box centered at origin
    // Caller scales to fill bounds
    let major_r = 0.35;
    let minor_r = 0.15;

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();

    for ring in 0..=ring_segments {
        let ring_angle = ring as f32 / ring_segments as f32 * std::f32::consts::TAU;
        let ring_cos = ring_angle.cos();
        let ring_sin = ring_angle.sin();

        for cross in 0..=cross_segments {
            let cross_angle = cross as f32 / cross_segments as f32 * std::f32::consts::TAU;
            let cross_cos = cross_angle.cos();
            let cross_sin = cross_angle.sin();

            let x = (major_r + minor_r * cross_cos) * ring_cos;
            let y = minor_r * cross_sin;
            let z = (major_r + minor_r * cross_cos) * ring_sin;

            positions.push([x, y, z]);

            let nx = cross_cos * ring_cos;
            let ny = cross_sin;
            let nz = cross_cos * ring_sin;
            normals.push([nx, ny, nz]);

            uvs.push([ring as f32 / ring_segments as f32, cross as f32 / cross_segments as f32]);
        }
    }

    let indices = generate_grid_indices(ring_segments, cross_segments, false);

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Shared helpers
// =====================================================================

/// Add a flat circular disc to close an open mesh.
/// `y` is the height of the disc. `normal_up` true = normal points +Y, false = -Y.
fn add_disc(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    radius: f32,
    y: f32,
    segments: u32,
    normal_up: bool,
) {
    let center_idx = positions.len() as u32;
    let ny = if normal_up { 1.0 } else { -1.0 };

    positions.push([0.0, y, 0.0]);
    normals.push([0.0, ny, 0.0]);
    uvs.push([0.5, 0.5]);

    for seg in 0..=segments {
        let angle = seg as f32 / segments as f32 * std::f32::consts::TAU;
        positions.push([radius * angle.cos(), y, radius * angle.sin()]);
        normals.push([0.0, ny, 0.0]);
        uvs.push([0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin()]);
    }

    for seg in 0..segments {
        let a = center_idx;
        let b = center_idx + 1 + seg;
        let c = center_idx + 2 + seg;
        if normal_up {
            indices.extend_from_slice(&[a, b, c]);
        } else {
            indices.extend_from_slice(&[a, c, b]);
        }
    }
}

fn compute_sphere_radius(base_radius: f32, height: f32) -> f32 {
    (base_radius * base_radius + height * height) / (2.0 * height)
}

/// Generate vertices for a sphere cap (used by both Dome and Bowl).
/// `normal_sign`: 1.0 for outward normals (dome), -1.0 for inward (bowl).
fn generate_cap_vertices(
    base_radius: f32,
    sphere_radius: f32,
    y_offset: f32,
    center_offset: f32,
    normal_sign: f32,
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

            positions.push([x, y - center_offset, z]);

            let nx = x / sphere_radius * normal_sign;
            let ny = (y + y_offset) / sphere_radius * normal_sign;
            let nz = z / sphere_radius * normal_sign;
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

/// Generate triangle indices for a grid of (rows+1) x (cols+1) vertices.
/// If `reverse_winding` is true, triangles face the opposite direction.
fn generate_grid_indices(rows: u32, cols: u32, reverse_winding: bool) -> Vec<u32> {
    let mut indices = Vec::new();
    let verts_per_row = cols + 1;

    for row in 0..rows {
        for col in 0..cols {
            let a = row * verts_per_row + col;
            let b = a + 1;
            let c = a + verts_per_row;
            let d = c + 1;

            if reverse_winding {
                indices.extend_from_slice(&[a, c, b]);
                indices.extend_from_slice(&[b, c, d]);
            } else {
                indices.extend_from_slice(&[a, b, c]);
                indices.extend_from_slice(&[b, d, c]);
            }
        }
    }

    indices
}

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
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
    indices.extend_from_slice(&[base, base + 1, base + 2]);
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

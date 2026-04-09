use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

// =====================================================================
// Dome — convex ellipsoidal cap, normals pointing outward
// =====================================================================

/// Unit dome: half-ellipsoid, radius=0.5, height=1.0 (base at y=-0.5, peak at y=0.5).
/// Scaled by transform to fill bounds.
pub fn create_unit_dome(rings: u32, segments: u32) -> Mesh {
    let (mut positions, mut normals, mut uvs) = generate_ellipsoid_cap(
        0.5, 1.0, 1.0, rings, segments,
    );
    let mut indices = generate_grid_indices(rings, segments, true);

    add_disc(&mut positions, &mut normals, &mut uvs, &mut indices,
        0.5, -0.5, segments, false);

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Cone — tapers from oval base to a point
// =====================================================================

pub fn create_cone_mesh(rings: u32, segments: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();

    let slope = (0.5_f32).atan2(1.0);

    for ring in 0..=rings {
        let t = ring as f32 / rings as f32;
        let y = -0.5 + t;
        let r = 0.5 * (1.0 - t);

        for seg in 0..=segments {
            let angle = seg as f32 / segments as f32 * std::f32::consts::TAU;
            positions.push([r * angle.cos(), y, r * angle.sin()]);

            let nx = angle.cos() * slope.cos();
            let ny = slope.sin();
            let nz = angle.sin() * slope.cos();
            normals.push([nx, ny, nz]);
            uvs.push([seg as f32 / segments as f32, t]);
        }
    }

    let mut indices = generate_grid_indices(rings, segments, true);

    // Bottom cap with its own downward-facing vertices
    add_disc(&mut positions, &mut normals, &mut uvs, &mut indices,
        0.5, -0.5, segments, false);

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

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Torus — ring/donut shape
// =====================================================================

pub fn create_torus_mesh(ring_segments: u32, cross_segments: u32) -> Mesh {
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
            normals.push([cross_cos * ring_cos, cross_sin, cross_cos * ring_sin]);
            uvs.push([ring as f32 / ring_segments as f32, cross as f32 / cross_segments as f32]);
        }
    }

    let indices = generate_grid_indices(ring_segments, cross_segments, false);
    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Capsule — cylinder with hemispherical ends
// =====================================================================

/// Unit capsule: total height=1.0, radius=0.25 at equator.
/// Cylinder section in the middle, hemisphere caps on each end.
/// Hemisphere radius = 0.25, cylinder height = 0.5.
/// Scale non-uniformly to stretch: XZ scales the radius, Y scales the whole height.
pub fn create_unit_capsule(rings: u32, segments: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();

    let radius = 0.25_f32;
    let half_cyl = 0.25_f32; // cylinder half-height

    // Top hemisphere: equator at y=half_cyl, pole at y=half_cyl+radius
    for ring in 0..=rings {
        let t = ring as f32 / rings as f32;
        let angle = t * std::f32::consts::FRAC_PI_2;
        let r = radius * angle.cos();
        let y = half_cyl + radius * angle.sin();

        for seg in 0..=segments {
            let phi = seg as f32 / segments as f32 * std::f32::consts::TAU;
            positions.push([r * phi.cos(), y, r * phi.sin()]);
            normals.push([phi.cos() * angle.cos(), angle.sin(), phi.sin() * angle.cos()]);
            uvs.push([seg as f32 / segments as f32, t * 0.25]);
        }
    }

    // Cylinder section: two rings at y=half_cyl and y=-half_cyl
    let cyl_top_start = positions.len() as u32;
    for seg in 0..=segments {
        let phi = seg as f32 / segments as f32 * std::f32::consts::TAU;
        let x = radius * phi.cos();
        let z = radius * phi.sin();
        // Top ring
        positions.push([x, half_cyl, z]);
        normals.push([phi.cos(), 0.0, phi.sin()]);
        uvs.push([seg as f32 / segments as f32, 0.25]);
    }
    let cyl_bot_start = positions.len() as u32;
    for seg in 0..=segments {
        let phi = seg as f32 / segments as f32 * std::f32::consts::TAU;
        let x = radius * phi.cos();
        let z = radius * phi.sin();
        // Bottom ring
        positions.push([x, -half_cyl, z]);
        normals.push([phi.cos(), 0.0, phi.sin()]);
        uvs.push([seg as f32 / segments as f32, 0.75]);
    }

    // Bottom hemisphere: equator at y=-half_cyl, pole at y=-half_cyl-radius
    let bot_start = positions.len() as u32;
    for ring in 0..=rings {
        let t = ring as f32 / rings as f32;
        let angle = t * std::f32::consts::FRAC_PI_2;
        let r = radius * angle.cos();
        let y = -half_cyl - radius * angle.sin();

        for seg in 0..=segments {
            let phi = seg as f32 / segments as f32 * std::f32::consts::TAU;
            positions.push([r * phi.cos(), y, r * phi.sin()]);
            normals.push([phi.cos() * angle.cos(), -angle.sin(), phi.sin() * angle.cos()]);
            uvs.push([seg as f32 / segments as f32, 0.75 + t * 0.25]);
        }
    }

    // Top hemisphere indices
    let mut indices = generate_grid_indices(rings, segments, true);

    // Cylinder indices (connect top ring to bottom ring)
    let verts_per_ring = segments + 1;
    for seg in 0..segments {
        let a = cyl_top_start + seg;
        let b = a + 1;
        let c = cyl_bot_start + seg;
        let d = c + 1;
        indices.extend_from_slice(&[a, c, b]);
        indices.extend_from_slice(&[b, c, d]);
    }

    // Bottom hemisphere indices
    for ring in 0..rings {
        for seg in 0..segments {
            let a = bot_start + ring * verts_per_ring + seg;
            let b = a + 1;
            let c = a + verts_per_ring;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b]);
            indices.extend_from_slice(&[b, c, d]);
        }
    }

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Corner — tetrahedron filling one corner of the bounding box
// =====================================================================

/// Unit corner: fills the (-X, -Y, -Z) corner of a 1x1x1 cube.
/// Three right-triangle faces on the cube walls + one diagonal face.
/// Orient/flip determines which corner the solid fills.
pub fn create_unit_corner() -> Mesh {
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

    build_mesh(positions, normals, uvs, indices)
}

// =====================================================================
// Shared: ellipsoid cap generation (used by Dome)
// =====================================================================

/// Generate an ellipsoidal cap. The cap has base radius `base_radius` in XZ
/// and rises `height` in Y. Centered so base is at `y = -height/2` and peak at `y = height/2`.
/// `normal_sign`: 1.0 for outward (dome), -1.0 for inward (bowl).
fn compute_sphere_radius(base_radius: f32, height: f32) -> f32 {
    (base_radius * base_radius + height * height) / (2.0 * height)
}

fn generate_ellipsoid_cap(
    base_radius: f32,
    height: f32,
    normal_sign: f32,
    rings: u32,
    segments: u32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>) {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();

    let half_h = height / 2.0;

    // Ring 0 = outer edge (base), Ring N = center (peak).
    // This order means the outer ring comes first, matching the disc edge.
    for ring in 0..=rings {
        let t = ring as f32 / rings as f32;
        // t=0 → edge, t=1 → peak
        // Use sine curve for smooth distribution
        let angle = t * std::f32::consts::FRAC_PI_2;
        let r = base_radius * angle.cos();
        let y = half_h * angle.sin() - half_h; // base at -half_h, peak at 0...
        // Actually: we want base at -half_h, peak at +half_h
        let y = -half_h + height * angle.sin();

        for seg in 0..=segments {
            let phi = seg as f32 / segments as f32 * std::f32::consts::TAU;
            let x = r * phi.cos();
            let z = r * phi.sin();

            positions.push([x, y, z]);

            // Ellipsoid normal: (x/a², y/b², z/a²) where a=base_radius, b=height
            let nx = x / (base_radius * base_radius) * normal_sign;
            let ny = (y + half_h) / (height * height) * normal_sign;  // shift to 0-based for normal calc
            let nz = z / (base_radius * base_radius) * normal_sign;
            let len = (nx * nx + ny * ny + nz * nz).sqrt().max(0.0001);
            normals.push([nx / len, ny / len, nz / len]);

            uvs.push([seg as f32 / segments as f32, t]);
        }
    }

    (positions, normals, uvs)
}

// =====================================================================
// Shared helpers
// =====================================================================

/// Add a flat circular disc to close an open mesh.
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
            indices.extend_from_slice(&[a, c, b]);
        } else {
            indices.extend_from_slice(&[a, b, c]);
        }
    }
}

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

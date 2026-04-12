//! CSG operations using SDF-based evaluation via fidget.
//! Shapes stay mathematical until the final meshing step.

use fidget::context::Tree;
use super::definition::{Bounds, CombineMode};
use super::meshes::RawMesh;
use super::sdf::{collect_sdf_from_events, mesh_sdf};
use super::traversal::{walk_shape_tree, ColorMap};
use crate::registry::AssetRegistry;

// =====================================================================
// Stats tracking
// =====================================================================

#[derive(Debug, Clone, Default)]
pub struct CsgStats {
    pub input_union_count: u32,
    pub input_subtract_count: u32,
    pub input_clip_count: u32,
    pub output_tris: u32,
    pub mesh_time_ms: f64,
}

// =====================================================================
// Public API
// =====================================================================

/// Perform CSG on shape children using SDF evaluation.
/// Each child's geometry is converted to an SDF tree, combined with
/// min/max operations, then meshed once at the end.
pub fn perform_csg_from_children(
    children: &[super::definition::ShapeNode],
    colors: &ColorMap,
    registry: &AssetRegistry,
    parent_aabb: &Bounds,
) -> (RawMesh, CsgStats) {
    let mut stats = CsgStats::default();

    let mut union_sdfs: Vec<Tree> = Vec::new();
    let mut subtract_sdfs: Vec<Tree> = Vec::new();
    let mut clip_sdfs: Vec<Tree> = Vec::new();

    for child in children {
        let events = walk_shape_tree(child, colors, registry);
        let Some(sdf) = collect_sdf_from_events(&events) else { continue };

        match child.combine {
            CombineMode::Union => {
                stats.input_union_count += 1;
                union_sdfs.push(sdf);
            }
            CombineMode::Subtract => {
                stats.input_subtract_count += 1;
                subtract_sdfs.push(sdf);
            }
            CombineMode::Clip => {
                stats.input_clip_count += 1;
                clip_sdfs.push(sdf);
            }
        }
    }

    if union_sdfs.is_empty() {
        return (RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] }, stats);
    }

    // Combine SDFs: union = min, subtract = max(a, -b), intersect = max(a, b)
    let mut result = union_sdfs.into_iter().reduce(|a, b| a.min(b)).unwrap();

    for sub in subtract_sdfs {
        result = result.max(-sub);
    }

    for clip in clip_sdfs {
        result = result.max(clip);
    }

    // Mesh the combined SDF
    let mesh_start = std::time::Instant::now();
    let (shared_pos, shared_idx) = mesh_sdf(&result, parent_aabb);
    stats.mesh_time_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;

    // Unshare vertices: each triangle gets its own copy with a face normal.
    // Shared vertices produce inconsistent normals when multiple faces write to them.
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // Compute per-vertex normals from SDF gradient for correct orientation,
    // then use them to orient flat face normals from the triangle cross product.
    // This gives flat shading (correct for planar faces) with correct facing direction.
    let sdf_normals = compute_sdf_normals(&result, &shared_pos);

    for tri in shared_idx.chunks(3) {
        if tri.len() < 3 { continue; }
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let a = bevy::math::Vec3::from(shared_pos[i0]);
        let b = bevy::math::Vec3::from(shared_pos[i1]);
        let c = bevy::math::Vec3::from(shared_pos[i2]);
        let cross = (b - a).cross(c - a);
        if cross.length_squared() < 1e-10 { continue; }
        let mut face_n = cross.normalize();
        if face_n.is_nan() { continue; }

        // Use SDF gradient to determine correct facing direction
        let avg_sdf_n = bevy::math::Vec3::from(sdf_normals[i0])
            + bevy::math::Vec3::from(sdf_normals[i1])
            + bevy::math::Vec3::from(sdf_normals[i2]);
        if face_n.dot(avg_sdf_n) < 0.0 {
            face_n = -face_n;
        }

        let fn32 = [face_n.x, face_n.y, face_n.z];
        let base = positions.len() as u32;
        positions.push(shared_pos[i0]);
        positions.push(shared_pos[i1]);
        positions.push(shared_pos[i2]);
        normals.push(fn32);
        normals.push(fn32);
        normals.push(fn32);
        uvs.push([0.0, 0.0]);
        uvs.push([0.0, 0.0]);
        uvs.push([0.0, 0.0]);
        indices.push(base);
        indices.push(base + 1);
        indices.push(base + 2);
    }

    let mesh = RawMesh { positions, normals, uvs, indices };
    stats.output_tris = mesh.indices.len() as u32 / 3;
    (mesh, stats)
}

/// Compute normals from SDF gradient using central differences.
fn compute_sdf_normals(tree: &Tree, positions: &[[f32; 3]]) -> Vec<[f32; 3]> {
    use fidget::vm::VmShape;
    use fidget::shape::EzShape;

    let shape = VmShape::from(tree.clone());
    let mut eval = VmShape::new_float_slice_eval();
    let tape = shape.ez_float_slice_tape();

    let eps = 0.001;
    let n = positions.len();

    // Evaluate SDF at offset positions for central differences
    let mut px_plus = vec![0.0f32; n];
    let mut px_minus = vec![0.0f32; n];
    let mut py_plus = vec![0.0f32; n];
    let mut py_minus = vec![0.0f32; n];
    let mut pz_plus = vec![0.0f32; n];
    let mut pz_minus = vec![0.0f32; n];

    let xs: Vec<f32> = positions.iter().map(|p| p[0]).collect();
    let ys: Vec<f32> = positions.iter().map(|p| p[1]).collect();
    let zs: Vec<f32> = positions.iter().map(|p| p[2]).collect();

    let xs_plus: Vec<f32> = xs.iter().map(|&x| x + eps).collect();
    let xs_minus: Vec<f32> = xs.iter().map(|&x| x - eps).collect();
    let ys_plus: Vec<f32> = ys.iter().map(|&y| y + eps).collect();
    let ys_minus: Vec<f32> = ys.iter().map(|&y| y - eps).collect();
    let zs_plus: Vec<f32> = zs.iter().map(|&z| z + eps).collect();
    let zs_minus: Vec<f32> = zs.iter().map(|&z| z - eps).collect();

    if let Ok(r) = eval.eval(&tape, &xs_plus, &ys, &zs) { px_plus.copy_from_slice(r); }
    if let Ok(r) = eval.eval(&tape, &xs_minus, &ys, &zs) { px_minus.copy_from_slice(r); }
    if let Ok(r) = eval.eval(&tape, &xs, &ys_plus, &zs) { py_plus.copy_from_slice(r); }
    if let Ok(r) = eval.eval(&tape, &xs, &ys_minus, &zs) { py_minus.copy_from_slice(r); }
    if let Ok(r) = eval.eval(&tape, &xs, &ys, &zs_plus) { pz_plus.copy_from_slice(r); }
    if let Ok(r) = eval.eval(&tape, &xs, &ys, &zs_minus) { pz_minus.copy_from_slice(r); }

    let mut normals = Vec::with_capacity(n);
    for i in 0..n {
        let nx = px_plus[i] - px_minus[i];
        let ny = py_plus[i] - py_minus[i];
        let nz = pz_plus[i] - pz_minus[i];
        let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-8);
        normals.push([nx / len, ny / len, nz / len]);
    }

    normals
}

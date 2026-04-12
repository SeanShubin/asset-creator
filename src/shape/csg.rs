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
    let (shared_pos, shared_idx) = mesh_sdf(&result, parent_aabb);

    // Unshare vertices: each triangle gets its own copy with a face normal.
    // Shared vertices produce inconsistent normals when multiple faces write to them.
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    for tri in shared_idx.chunks(3) {
        if tri.len() < 3 { continue; }
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let a = bevy::math::Vec3::from(shared_pos[i0]);
        let b = bevy::math::Vec3::from(shared_pos[i1]);
        let c = bevy::math::Vec3::from(shared_pos[i2]);
        let n = (b - a).cross(c - a).normalize();
        let fn32 = [n.x, n.y, n.z];

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

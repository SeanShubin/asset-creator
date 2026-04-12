//! CSG operations using SDF-based evaluation via fidget.
//! Shapes stay mathematical until the final meshing step.
//! Results are cached to disk to avoid recomputation on subsequent loads.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use fidget::context::Tree;
use super::definition::{Bounds, CombineMode};
use super::meshes::RawMesh;
use super::sdf::{collect_sdf_from_events, mesh_sdf};
use super::traversal::{walk_shape_tree, ColorMap, ShapeEvent};
use crate::registry::AssetRegistry;

const CACHE_DIR: &str = "generated/csg-cache";

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

/// Perform CSG on shape children using SDF evaluation, with disk caching.
pub fn perform_csg_from_children(
    children: &[super::definition::ShapeNode],
    colors: &ColorMap,
    registry: &AssetRegistry,
    parent_aabb: &Bounds,
) -> (RawMesh, CsgStats) {
    perform_csg_impl(children, colors, registry, parent_aabb, true)
}

/// Perform CSG without caching (for stress tests).
pub fn perform_csg_uncached(
    children: &[super::definition::ShapeNode],
    colors: &ColorMap,
    registry: &AssetRegistry,
    parent_aabb: &Bounds,
) -> (RawMesh, CsgStats) {
    perform_csg_impl(children, colors, registry, parent_aabb, false)
}

/// Build an SDF from shape events, mesh it, and return a RawMesh with normals.
/// Used for the "Preview CSG mesh" toggle.
pub fn mesh_sdf_from_events(events: &[ShapeEvent], aabb: &Bounds) -> RawMesh {
    let Some(sdf) = collect_sdf_from_events(events) else {
        return RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };
    };

    let (shared_pos, shared_idx) = mesh_sdf(&sdf, aabb);
    build_raw_mesh_with_normals(&sdf, shared_pos, shared_idx)
}

fn perform_csg_impl(
    children: &[super::definition::ShapeNode],
    colors: &ColorMap,
    registry: &AssetRegistry,
    parent_aabb: &Bounds,
    use_cache: bool,
) -> (RawMesh, CsgStats) {
    let mut stats = CsgStats::default();

    // Check cache first
    let cache_key = if use_cache { Some(compute_cache_key(children, parent_aabb)) } else { None };
    if let Some(ref key) = cache_key {
        if let Some(cached) = load_cache(key) {
            stats.output_tris = cached.indices.len() as u32 / 3;
            stats.mesh_time_ms = 0.0;
            return (cached, stats);
        }
    }

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

    let mut result = union_sdfs.into_iter().reduce(|a, b| a.min(b)).unwrap();

    for sub in subtract_sdfs {
        result = result.max(-sub);
    }

    for clip in clip_sdfs {
        result = result.max(clip);
    }

    let mesh_start = std::time::Instant::now();
    let (shared_pos, shared_idx) = mesh_sdf(&result, parent_aabb);
    stats.mesh_time_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;

    let mesh = build_raw_mesh_with_normals(&result, shared_pos, shared_idx);
    stats.output_tris = mesh.indices.len() as u32 / 3;

    // Save to cache
    if let Some(ref key) = cache_key {
        save_cache(key, &mesh);
    }

    (mesh, stats)
}

// =====================================================================
// SDF mesh with oriented flat normals
// =====================================================================

/// Convert shared-vertex mesh from fidget into a RawMesh with per-face
/// flat normals oriented using the SDF gradient.
fn build_raw_mesh_with_normals(sdf: &Tree, shared_pos: Vec<[f32; 3]>, shared_idx: Vec<u32>) -> RawMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    let sdf_normals = compute_sdf_normals(sdf, &shared_pos);

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

    RawMesh { positions, normals, uvs, indices }
}

// =====================================================================
// Disk cache
// =====================================================================

/// Bump this when SDF logic, mesh builders, or normal computation changes.
/// Invalidates all cached CSG meshes.
const CACHE_VERSION: u32 = 2;

fn compute_cache_key(children: &[super::definition::ShapeNode], aabb: &Bounds) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    CACHE_VERSION.hash(&mut hasher);
    format!("{children:?}{aabb:?}").hash(&mut hasher);
    let hash = hasher.finish();
    PathBuf::from(CACHE_DIR).join(format!("{hash:016x}.mesh"))
}

fn load_cache(path: &PathBuf) -> Option<RawMesh> {
    let data = std::fs::read(path).ok()?;
    deserialize_mesh(&data)
}

fn save_cache(path: &PathBuf, mesh: &RawMesh) {
    let _ = std::fs::create_dir_all(CACHE_DIR);
    if let Some(data) = serialize_mesh(mesh) {
        let _ = std::fs::write(path, data);
    }
}

fn serialize_mesh(mesh: &RawMesh) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let n_pos = mesh.positions.len() as u32;
    let n_idx = mesh.indices.len() as u32;
    buf.extend_from_slice(&n_pos.to_le_bytes());
    buf.extend_from_slice(&n_idx.to_le_bytes());
    for p in &mesh.positions {
        for &v in p { buf.extend_from_slice(&v.to_le_bytes()); }
    }
    for n in &mesh.normals {
        for &v in n { buf.extend_from_slice(&v.to_le_bytes()); }
    }
    for u in &mesh.uvs {
        for &v in u { buf.extend_from_slice(&v.to_le_bytes()); }
    }
    for &i in &mesh.indices {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    Some(buf)
}

fn deserialize_mesh(data: &[u8]) -> Option<RawMesh> {
    if data.len() < 8 { return None; }
    let n_pos = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    let n_idx = u32::from_le_bytes(data[4..8].try_into().ok()?) as usize;

    let expected = 8 + n_pos * 12 + n_pos * 12 + n_pos * 8 + n_idx * 4;
    if data.len() < expected { return None; }

    let mut offset = 8;
    let read_f32 = |o: &mut usize| -> f32 {
        let v = f32::from_le_bytes(data[*o..*o + 4].try_into().unwrap());
        *o += 4;
        v
    };
    let read_u32 = |o: &mut usize| -> u32 {
        let v = u32::from_le_bytes(data[*o..*o + 4].try_into().unwrap());
        *o += 4;
        v
    };

    let mut positions = Vec::with_capacity(n_pos);
    for _ in 0..n_pos {
        positions.push([read_f32(&mut offset), read_f32(&mut offset), read_f32(&mut offset)]);
    }
    let mut normals = Vec::with_capacity(n_pos);
    for _ in 0..n_pos {
        normals.push([read_f32(&mut offset), read_f32(&mut offset), read_f32(&mut offset)]);
    }
    let mut uvs = Vec::with_capacity(n_pos);
    for _ in 0..n_pos {
        uvs.push([read_f32(&mut offset), read_f32(&mut offset)]);
    }
    let mut indices = Vec::with_capacity(n_idx);
    for _ in 0..n_idx {
        indices.push(read_u32(&mut offset));
    }

    Some(RawMesh { positions, normals, uvs, indices })
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

//! Compile a `SpecNode` tree into a tree of `CompiledShape`s — the
//! render-ready form consumed by the interpreter.
//!
//! Each `CompiledShape` corresponds to one authored `ShapePart`: a named
//! node in the spec tree. Its `meshes` field holds zero or more fused
//! `RawMesh`es (at most one per emissive flag for now) representing all
//! cells that belong to this part's subtree, except those that belong
//! to named sub-parts — those become child `CompiledShape`s.
//!
//! The compile step resolves symmetry expansion, import remapping, and
//! `subtract: true` all in integer cell space, then bakes the
//! surviving cells into per-part fused meshes with vertex colors. The
//! interpreter then spawns one Bevy entity per `CompiledShape` with the
//! fused meshes attached as child `Mesh3d` entities.
//!
//! This module is the one-way bridge from the integer spec to floats.
//! No downstream code references `SpecNode`.

use bevy::prelude::*;
use crate::registry::AssetRegistry;
use crate::util::Color3;
use super::meshes::{create_raw_mesh, RawMesh};
use super::spec::{
    apply_placement_to_bounds, aabb_for_parts, compose_placements,
    identity_placement, placements, remap_bounds_for_parts,
    Bounds, Placement, PrimitiveShape, SpecNode, SymOp, compose_orient, placements_for,
};

// =====================================================================
// Public output: the tree handed to the interpreter
// =====================================================================

/// The render-ready form of a compiled shape. Mirrors the `ShapePart`
/// hierarchy the user authored: one `CompiledShape` per named node.
/// Its own cells are baked into `meshes`; named sub-parts live in
/// `children`.
#[derive(Clone, Debug, Default)]
pub struct CompiledShape {
    pub name: Option<String>,
    /// Transform relative to the parent `CompiledShape` (or to the
    /// scene root if this is the top level). For named parts this
    /// equals the identity transform — each part's cells already carry
    /// their own world-space positions baked into the fused mesh.
    pub local_transform: Transform,
    pub meshes: Vec<FusedMesh>,
    pub children: Vec<CompiledShape>,
    pub subtract: bool,
}

/// A single fused mesh for one material configuration. Cells with the
/// same emissive flag are merged into one; cells with different flags
/// produce separate `FusedMesh`es under the same `CompiledShape`.
/// Per-vertex colors carry the authored cell color.
#[derive(Clone, Debug)]
pub struct FusedMesh {
    pub mesh: RawMesh,
    pub emissive: bool,
    /// Whether any cell in the mesh came from a primitive with a
    /// negative-determinant orientation (a mirrored copy). Controls
    /// face culling mode at spawn time.
    pub contains_mirrored: bool,
    /// True for subtract-primitive preview meshes. The interpreter
    /// renders these with `AlphaMode::Blend` so they appear as
    /// translucent overlays showing what volume is being carved.
    /// Only emitted when the shape is compiled directly (not via import).
    pub subtract_preview: bool,
}

// =====================================================================
// Tags → material properties
// =====================================================================

/// Resolve an ordered tag list into a color. Tags are processed left
/// to right: a color name sets the base, `lighten` and `darken`
/// modify it. Unrecognized tags are silently skipped.
/// Returns default grey if no color tag is found.
pub fn resolve_tags_color(tags: &[String]) -> Color3 {
    let mut color = None;
    for tag in tags {
        match tag.to_ascii_lowercase().as_str() {
            "red"     => color = Some(Color3(3, 0, 0)),
            "green"   => color = Some(Color3(0, 3, 0)),
            "blue"    => color = Some(Color3(0, 0, 3)),
            "cyan"    => color = Some(Color3(0, 3, 3)),
            "magenta" => color = Some(Color3(3, 0, 3)),
            "yellow"  => color = Some(Color3(3, 3, 0)),
            "white"   => color = Some(Color3(3, 3, 3)),
            "black"   => color = Some(Color3(0, 0, 0)),
            "lighten" => if let Some(ref mut c) = color {
                c.0 = (c.0 + 1).min(3);
                c.1 = (c.1 + 1).min(3);
                c.2 = (c.2 + 1).min(3);
            },
            "darken" => if let Some(ref mut c) = color {
                c.0 = c.0.saturating_sub(1);
                c.1 = c.1.saturating_sub(1);
                c.2 = c.2.saturating_sub(1);
            },
            _ => {}
        }
    }
    color.unwrap_or(Color3(2, 2, 2))
}

/// Check whether the tag list includes "emissive" (case-insensitive).
pub fn resolve_tags_emissive(tags: &[String]) -> bool {
    tags.iter().any(|t| t.eq_ignore_ascii_case("emissive"))
}

/// Convert orient ops + symmetry placement into a final Mat3.
fn orient_mat3(orient: &[SymOp], placement: Placement) -> Mat3 {
    super::csg::orient_placement_to_mat3(orient, placement)
}

// =====================================================================
// Integer → float transforms
// =====================================================================

/// Compute the mesh transform for a single cell. The unit mesh
/// (-0.5..0.5) is scaled by orient × size, then translated by size/2
/// so it fills the cell bounds from (0,0,0) to size relative to
/// bounds.min().
fn compute_mesh_transform(
    shape: PrimitiveShape,
    bounds: &Bounds,
    om: &Mat3,
    scale: (i32, i32, i32),
) -> Transform {
    let _ = shape;
    let isize = bounds.size();
    let size = (
        isize.0 as f32 / scale.0 as f32,
        isize.1 as f32 / scale.1 as f32,
        isize.2 as f32 / scale.2 as f32,
    );
    let mn = bounds.min();
    let position = Vec3::new(
        mn.0 as f32 / scale.0 as f32 + size.0 / 2.0,
        mn.1 as f32 / scale.1 as f32 + size.1 / 2.0,
        mn.2 as f32 / scale.2 as f32 + size.2 / 2.0,
    );

    let local_x_size = pick_size_for_direction(om.x_axis, size);
    let local_y_size = pick_size_for_direction(om.y_axis, size);
    let local_z_size = pick_size_for_direction(om.z_axis, size);

    let local_scale = Vec3::new(local_x_size, local_y_size, local_z_size);

    let col_x = om.x_axis * local_scale.x;
    let col_y = om.y_axis * local_scale.y;
    let col_z = om.z_axis * local_scale.z;

    let mat = Mat3::from_cols(col_x, col_y, col_z);
    let affine = bevy::math::Affine3A::from_mat3_translation(mat, position);
    Transform::from_matrix(bevy::math::Mat4::from(affine))
}

fn pick_size_for_direction(dir: Vec3, size: (f32, f32, f32)) -> f32 {
    if dir.x.abs() > 0.5 {
        size.0
    } else if dir.y.abs() > 0.5 {
        size.1
    } else {
        size.2
    }
}

// =====================================================================
// Compile — the single bridge from spec to render
// =====================================================================

/// Compile a `SpecNode` tree into a `CompiledShape` tree with fused
/// per-part meshes. Parts whose names appear in `hidden` are skipped:
/// no geometry, no CSG effect. They still appear as empty nodes in the
/// compiled tree so the parts-tree UI can toggle them back on.
pub fn compile(
    parts: &[SpecNode],
    registry: &AssetRegistry,
    hidden: &[String],
) -> CompiledShape {
    let templates = PrimitiveTemplates::new();
    let ctx = CompileCtx {
        flatten: false,
        registry,
        templates: &templates,
        hidden,
        is_direct: true,
    };
    // Pre-pass: collect all subtract primitives from every part
    // (stopping at import boundaries). Every group gets this full list
    // so subtracts carve into unions regardless of naming hierarchy.
    let mut all_subtracts = Vec::new();
    for part in parts {
        collect_subtracts(part, identity_placement(), (1, 1, 1), hidden, &mut all_subtracts);
    }
    // Walk each top-level part into a single root group.
    let mut group = GroupAccumulator::new();
    group.subtract_primitives.extend_from_slice(&all_subtracts);
    for part in parts {
        walk_into_group(
            part,
            identity_placement(),
            (1, 1, 1),
            &mut group,
            false,
            &ctx,
            &all_subtracts,
            "",
        );
    }
    group.finish(None, false, ctx.templates)
}

/// Production stats: triangle and draw call counts for the fully
/// fused shape with no named-group boundaries.
pub struct ProductionStats {
    pub triangles: usize,
    pub draw_calls: usize,
}

/// Compute production-quality stats for a shape. Resolves the entire
/// shape into a flat cell list (ignoring named groups), fuses adjacent
/// boxes, and counts the resulting triangles and draw calls.
pub fn production_stats(
    parts: &[SpecNode],
    registry: &AssetRegistry,
) -> ProductionStats {
    let templates = PrimitiveTemplates::new();
    let ctx = CompileCtx {
        flatten: true,
        registry,
        templates: &templates,
        hidden: &[],
        is_direct: false,
    };
    let mut all_subtracts = Vec::new();
    for part in parts {
        collect_subtracts(part, identity_placement(), (1, 1, 1), &[], &mut all_subtracts);
    }
    let mut group = GroupAccumulator::new();
    group.subtract_primitives.extend_from_slice(&all_subtracts);
    for part in parts {
        walk_into_group(
            part,
            identity_placement(),
            (1, 1, 1),
            &mut group,
            false,
            &ctx,
            &all_subtracts,
            "",
        );
    }
    let resolved = fuse_boxes(group.resolve(None));

    let mut has_normal = false;
    let mut has_emissive = false;
    for cell in &resolved {
        if cell.emissive { has_emissive = true; } else { has_normal = true; }
    }
    let draw_calls = has_normal as usize + has_emissive as usize;
    let triangles = count_triangles_with_face_culling(&resolved);

    ProductionStats { triangles, draw_calls }
}


struct CompileCtx<'a> {
    registry: &'a AssetRegistry,
    templates: &'a PrimitiveTemplates,
    hidden: &'a [String],
    is_direct: bool,
    /// When true, all nodes are walked into one flat group — no
    /// named child groups are created. Used by production_stats.
    flatten: bool,
}

struct PrimitiveTemplates {
    box_mesh: RawMesh,
    wedge_mesh: RawMesh,
    corner_mesh: RawMesh,
    inverse_corner_mesh: RawMesh,
}

impl PrimitiveTemplates {
    fn new() -> Self {
        Self {
            box_mesh: create_raw_mesh(PrimitiveShape::Box),
            wedge_mesh: create_raw_mesh(PrimitiveShape::Wedge),
            corner_mesh: create_raw_mesh(PrimitiveShape::Corner),
            inverse_corner_mesh: create_raw_mesh(PrimitiveShape::InverseCorner),
        }
    }

    fn get(&self, shape: PrimitiveShape) -> &RawMesh {
        match shape {
            PrimitiveShape::Box => &self.box_mesh,
            PrimitiveShape::Wedge => &self.wedge_mesh,
            PrimitiveShape::Corner => &self.corner_mesh,
            PrimitiveShape::InverseCorner => &self.inverse_corner_mesh,
        }
    }
}

/// A Union primitive recorded at accumulation time. Kept as its whole
/// authored extent — the decision whether to render it as one mesh or
/// to decompose into individual cells is deferred to fusion time,
/// where we can check for Subtract overlap.
struct UnionPrimitive {
    shape: PrimitiveShape,
    /// Post-placement bounds of this primitive in world cell space.
    world_bounds: Bounds,
    /// Final orientation matrix (authored orient composed with any
    /// symmetry-expansion placement).
    orient_mat: Mat3,
    color: Color3,
    emissive: bool,
    is_mirrored: bool,
    scale: (i32, i32, i32),
}

/// A Subtract primitive recorded at accumulation time. Same fields as
/// `UnionPrimitive` minus the color (subtracts don't contribute color)
/// and emissive flag. Preserved through fusion so we can do per-cell
/// CSG against every union primitive that shares its cells.
#[derive(Clone)]
struct SubtractPrimitive {
    shape: PrimitiveShape,
    world_bounds: Bounds,
    orient_mat: Mat3,
    scale: (i32, i32, i32),
}

/// Accumulator for primitives belonging to one group (one compiled part).
struct GroupAccumulator {
    union_primitives: Vec<UnionPrimitive>,
    subtract_primitives: Vec<SubtractPrimitive>,
    /// Subtract primitives recorded for translucent preview rendering.
    /// Only populated when the shape is compiled directly (not via import).
    preview_primitives: Vec<UnionPrimitive>,
    children: Vec<CompiledShape>,
}

/// Identity of an integer world cell, used to match Union/Subtract
/// overlaps per cell.
type CellKey = (i32, i32, i32);

/// A primitive after CSG resolution. Either a whole multi-cell
/// primitive (not affected by any subtract) or a single post-CSG
/// cell. This is the boundary between integer logic (what survives)
/// and float logic (turn it into a mesh).
struct ResolvedPrimitive {
    shape: PrimitiveShape,
    bounds: Bounds,
    orient_mat: Mat3,
    scale: (i32, i32, i32),
    color: Color3,
    emissive: bool,
    is_mirrored: bool,
    subtract_preview: bool,
}

impl GroupAccumulator {
    fn new() -> Self {
        Self {
            union_primitives: Vec::new(),
            subtract_primitives: Vec::new(),
            preview_primitives: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Resolve CSG: determine which primitives survive subtraction and
    /// what shape they become. Multi-cell primitives that don't touch
    /// any subtract are preserved whole. Primitives overlapping a
    /// subtract are decomposed into unit cells with per-cell CSG.
    fn resolve(&self, name: Option<&str>) -> Vec<ResolvedPrimitive> {
        let mut subtract_cells: std::collections::HashMap<CellKey, u64> =
            std::collections::HashMap::new();
        for sub in &self.subtract_primitives {
            let mn = sub.world_bounds.min();
            let mx = sub.world_bounds.max();
            let prim_center = Vec3::new(
                (mn.0 + mx.0) as f32 / (2.0 * sub.scale.0 as f32),
                (mn.1 + mx.1) as f32 / (2.0 * sub.scale.1 as f32),
                (mn.2 + mx.2) as f32 / (2.0 * sub.scale.2 as f32),
            );
            let prim_half_size = Vec3::new(
                (mx.0 - mn.0) as f32 / sub.scale.0 as f32,
                (mx.1 - mn.1) as f32 / sub.scale.1 as f32,
                (mx.2 - mn.2) as f32 / sub.scale.2 as f32,
            );
            enumerate_world_cells(&sub.world_bounds, sub.scale, |cell| {
                let sig = super::csg::compute_signature_at_cell(
                    sub.shape, sub.orient_mat, prim_center, prim_half_size, cell,
                );
                *subtract_cells.entry(cell).or_default() |= sig;
            });
        }

        let mut resolved = Vec::new();

        for prim in &self.union_primitives {
            if !primitive_touches_subtracts(prim, &subtract_cells) {
                resolved.push(ResolvedPrimitive {
                    shape: prim.shape,
                    bounds: prim.world_bounds,
                    orient_mat: prim.orient_mat,
                    scale: prim.scale,
                    color: prim.color,
                    emissive: prim.emissive,
                    is_mirrored: prim.is_mirrored,
                    subtract_preview: false,
                });
            } else {
                let mn = prim.world_bounds.min();
                let mx = prim.world_bounds.max();
                let wmin = (
                    floor_div(mn.0, prim.scale.0),
                    floor_div(mn.1, prim.scale.1),
                    floor_div(mn.2, prim.scale.2),
                );
                let wmax = (
                    ceil_div(mx.0, prim.scale.0),
                    ceil_div(mx.1, prim.scale.1),
                    ceil_div(mx.2, prim.scale.2),
                );
                for z in wmin.2..wmax.2 {
                    for y in wmin.1..wmax.1 {
                        for x in wmin.0..wmax.0 {
                            let cell = (x, y, z);
                            let sub_sig = subtract_cells.get(&cell).copied().unwrap_or(0);
                            if sub_sig == 0 {
                                resolved.push(ResolvedPrimitive {
                                    shape: prim.shape,
                                    bounds: Bounds(x, y, z, x + 1, y + 1, z + 1),
                                    orient_mat: prim.orient_mat,
                                    scale: (1, 1, 1),
                                    color: prim.color,
                                    emissive: prim.emissive,
                                    is_mirrored: prim.is_mirrored,
                                    subtract_preview: false,
                                });
                            } else {
                                match super::csg::cell_subtract_with_sig(
                                    (prim.shape, prim.orient_mat), sub_sig,
                                ) {
                                    super::csg::CellResult::Empty => {}
                                    super::csg::CellResult::Keep { shape, orient_mat } => {
                                        resolved.push(ResolvedPrimitive {
                                            shape,
                                            bounds: Bounds(x, y, z, x + 1, y + 1, z + 1),
                                            orient_mat,
                                            scale: (1, 1, 1),
                                            color: prim.color,
                                            emissive: prim.emissive,
                                            is_mirrored: prim.is_mirrored,
                                            subtract_preview: false,
                                        });
                                    }
                                    super::csg::CellResult::NotRepresentable { result_signature } => {
                                        error!(
                                            "subtract result at cell ({},{},{}) for '{}' not representable (sig={:016x})",
                                            x, y, z, name.unwrap_or("unnamed"), result_signature
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Subtract previews.
        for prim in &self.preview_primitives {
            resolved.push(ResolvedPrimitive {
                shape: prim.shape,
                bounds: prim.world_bounds,
                orient_mat: prim.orient_mat,
                scale: prim.scale,
                color: prim.color,
                emissive: false,
                is_mirrored: prim.is_mirrored,
                subtract_preview: true,
            });
        }

        resolved
    }

    /// Turn resolved primitives into fused meshes (editor path, no fusion).
    fn finish(self, name: Option<String>, subtract: bool, templates: &PrimitiveTemplates) -> CompiledShape {
        let resolved = self.resolve(name.as_deref());

        let mut emissive_mesh = RawMesh::default();
        let mut normal_mesh = RawMesh::default();
        let mut preview_mesh = RawMesh::default();
        let mut emissive_mirrored = false;
        let mut normal_mirrored = false;
        let mut preview_mirrored = false;

        for cell in &resolved {
            let mesh_tf = compute_mesh_transform(cell.shape, &cell.bounds, &cell.orient_mat, cell.scale);
            let (cr, cg, cb) = cell.color.to_rgb();

            if cell.subtract_preview {
                let rgba = [cr, cg, cb, 0.3];
                preview_mesh.append_transformed(templates.get(cell.shape), &mesh_tf, rgba);
                if cell.is_mirrored { preview_mirrored = true; }
            } else if cell.emissive {
                let rgba = [cr, cg, cb, 1.0];
                emissive_mesh.append_transformed(templates.get(cell.shape), &mesh_tf, rgba);
                if cell.is_mirrored { emissive_mirrored = true; }
            } else {
                let rgba = [cr, cg, cb, 1.0];
                normal_mesh.append_transformed(templates.get(cell.shape), &mesh_tf, rgba);
                if cell.is_mirrored { normal_mirrored = true; }
            }
        }

        let mut meshes = Vec::new();
        if !normal_mesh.is_empty() {
            meshes.push(FusedMesh { mesh: normal_mesh, emissive: false, contains_mirrored: normal_mirrored, subtract_preview: false });
        }
        if !emissive_mesh.is_empty() {
            meshes.push(FusedMesh { mesh: emissive_mesh, emissive: true, contains_mirrored: emissive_mirrored, subtract_preview: false });
        }
        if !preview_mesh.is_empty() {
            meshes.push(FusedMesh { mesh: preview_mesh, emissive: false, contains_mirrored: preview_mirrored, subtract_preview: true });
        }

        CompiledShape {
            name,
            local_transform: Transform::IDENTITY,
            meshes,
            children: self.children,
            subtract,
        }
    }
}

/// Merge adjacent unit-cell Box primitives with the same color and
/// emissive flag into larger rectangular prisms. Eliminates internal
/// faces — 8 unit boxes composing a 2×2×2 cube become one box (12
/// triangles instead of 96). Non-box shapes and multi-cell primitives
/// pass through unchanged.
fn fuse_boxes(cells: Vec<ResolvedPrimitive>) -> Vec<ResolvedPrimitive> {
    let mut result = Vec::new();
    let mut box_groups: std::collections::HashMap<(Color3, bool), Vec<(i32, i32, i32)>> =
        std::collections::HashMap::new();

    for cell in cells {
        let is_unit_box = cell.shape == PrimitiveShape::Box
            && !cell.subtract_preview
            && cell.scale == (1, 1, 1)
            && {
                let s = cell.bounds.size();
                s.0 == 1 && s.1 == 1 && s.2 == 1
            };

        if is_unit_box {
            let mn = cell.bounds.min();
            box_groups
                .entry((cell.color, cell.emissive))
                .or_default()
                .push(mn);
        } else {
            result.push(cell);
        }
    }

    for ((color, emissive), positions) in box_groups {
        for bounds in greedy_merge(positions) {
            result.push(ResolvedPrimitive {
                shape: PrimitiveShape::Box,
                bounds,
                orient_mat: Mat3::IDENTITY,
                scale: (1, 1, 1),
                color,
                emissive,
                is_mirrored: false,
                subtract_preview: false,
            });
        }
    }

    result
}

/// Greedy 3D box merge. Takes a set of unit-cell positions and returns
/// the smallest set of axis-aligned rectangular prisms that cover them
/// exactly.
fn greedy_merge(positions: Vec<(i32, i32, i32)>) -> Vec<Bounds> {
    let mut occupied: std::collections::HashSet<(i32, i32, i32)> = positions.into_iter().collect();
    let mut result = Vec::new();

    // Process cells in sorted order for deterministic results.
    let mut sorted: Vec<(i32, i32, i32)> = occupied.iter().copied().collect();
    sorted.sort_by(|a, b| a.2.cmp(&b.2).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0)));

    for start in sorted {
        if !occupied.contains(&start) {
            continue; // already claimed
        }

        // Extend along X as far as possible.
        let mut x_end = start.0 + 1;
        while occupied.contains(&(x_end, start.1, start.2)) {
            x_end += 1;
        }

        // Extend along Y: every row in the Y range must have the full X run.
        let mut y_end = start.1 + 1;
        'y: loop {
            for x in start.0..x_end {
                if !occupied.contains(&(x, y_end, start.2)) {
                    break 'y;
                }
            }
            y_end += 1;
        }

        // Extend along Z: every layer must have the full XY rectangle.
        let mut z_end = start.2 + 1;
        'z: loop {
            for y in start.1..y_end {
                for x in start.0..x_end {
                    if !occupied.contains(&(x, y, z_end)) {
                        break 'z;
                    }
                }
            }
            z_end += 1;
        }

        // Claim all cells in the merged prism.
        for z in start.2..z_end {
            for y in start.1..y_end {
                for x in start.0..x_end {
                    occupied.remove(&(x, y, z));
                }
            }
        }

        result.push(Bounds(start.0, start.1, start.2, x_end, y_end, z_end));
    }

    result
}

/// The six axis-aligned face directions. For each direction, the
/// offset to the neighbor cell and the face index in the primitive.
const FACE_DIRS: [(i32, i32, i32); 6] = [
    ( 0,  1,  0), // +Y
    ( 0, -1,  0), // -Y
    ( 0,  0,  1), // +Z
    ( 0,  0, -1), // -Z
    ( 1,  0,  0), // +X
    (-1,  0,  0), // -X
];

/// Returns a bitmask where bit i is set if the primitive has a full
/// axis-aligned face in direction i, accounting for orientation.
fn face_coverage(shape: PrimitiveShape, orient_mat: &Mat3) -> u8 {
    if shape == PrimitiveShape::Box {
        return 0b111111;
    }
    // All 6 axis faces exist for InverseCorner (3 quads + 3 triangles).
    if shape == PrimitiveShape::InverseCorner {
        return 0b111111;
    }
    let identity_faces: &[(usize, usize)] = match shape {
        PrimitiveShape::Wedge => &[(1, 2), (3, 2), (4, 1), (5, 1)],
        PrimitiveShape::Corner => &[(1, 1), (3, 1), (5, 1)],
        PrimitiveShape::Box | PrimitiveShape::InverseCorner => unreachable!(),
    };
    let mut mask = 0u8;
    for &(face_idx, _) in identity_faces {
        let (dx, dy, dz) = FACE_DIRS[face_idx];
        let world = *orient_mat * Vec3::new(dx as f32, dy as f32, dz as f32);
        for (i, &(fx, fy, fz)) in FACE_DIRS.iter().enumerate() {
            if world.dot(Vec3::new(fx as f32, fy as f32, fz as f32)) > 0.9 {
                mask |= 1 << i;
                break;
            }
        }
    }
    mask
}

/// Count triangles after face culling. Shared axis-aligned faces
/// between adjacent primitives are removed from both sides.
fn count_triangles_with_face_culling(resolved: &[ResolvedPrimitive]) -> usize {
    // Pass 1: build spatial lookup of all occupied cells with face coverage.
    let mut cells: std::collections::HashMap<(i32,i32,i32), u8> =
        std::collections::HashMap::new();

    for prim in resolved {
        if prim.subtract_preview { continue; }
        let mn = prim.bounds.min();
        let mx = prim.bounds.max();
        let coverage = face_coverage(prim.shape, &prim.orient_mat);
        for z in mn.2..mx.2 {
            for y in mn.1..mx.1 {
                for x in mn.0..mx.0 {
                    *cells.entry((x, y, z)).or_insert(0) |= coverage;
                }
            }
        }
    }

    // Pass 2: count surviving faces.
    let mut total = 0;
    for prim in resolved {
        if prim.subtract_preview { continue; }
        let mn = prim.bounds.min();
        let mx = prim.bounds.max();
        let is_multi_cell_box = prim.shape == PrimitiveShape::Box
            && (mx.0 - mn.0 > 1 || mx.1 - mn.1 > 1 || mx.2 - mn.2 > 1);

        if is_multi_cell_box {
            // Fused box: 6 faces. Each face is one quad (2 tris).
            // Cull only if ALL neighbor cells along the face boundary
            // have a covering face on the opposite side.
            total += count_fused_box_faces(&cells, mn, (mx.0, mx.1, mx.2));
        } else {
            // Unit cell: count per-face, plus non-axis faces (slopes).
            total += count_unit_cell_faces(prim, &cells);
        }
    }

    total
}

fn count_fused_box_faces(
    cells: &std::collections::HashMap<(i32,i32,i32), u8>,
    mn: (i32, i32, i32),
    mx: (i32, i32, i32),
) -> usize {
    let mut tris = 0;
    // +Y face: neighbors at y=mx.1
    if !(mn.0..mx.0).all(|x| (mn.2..mx.2).all(|z| cells.get(&(x, mx.1, z)).is_some_and(|&m| m & (1 << 1) != 0))) { tris += 2; }
    // -Y face: neighbors at y=mn.1-1
    if !(mn.0..mx.0).all(|x| (mn.2..mx.2).all(|z| cells.get(&(x, mn.1-1, z)).is_some_and(|&m| m & (1 << 0) != 0))) { tris += 2; }
    // +Z face: neighbors at z=mx.2
    if !(mn.0..mx.0).all(|x| (mn.1..mx.1).all(|y| cells.get(&(x, y, mx.2)).is_some_and(|&m| m & (1 << 3) != 0))) { tris += 2; }
    // -Z face: neighbors at z=mn.2-1
    if !(mn.0..mx.0).all(|x| (mn.1..mx.1).all(|y| cells.get(&(x, y, mn.2-1)).is_some_and(|&m| m & (1 << 2) != 0))) { tris += 2; }
    // +X face: neighbors at x=mx.0
    if !(mn.1..mx.1).all(|y| (mn.2..mx.2).all(|z| cells.get(&(mx.0, y, z)).is_some_and(|&m| m & (1 << 5) != 0))) { tris += 2; }
    // -X face: neighbors at x=mn.0-1
    if !(mn.1..mx.1).all(|y| (mn.2..mx.2).all(|z| cells.get(&(mn.0-1, y, z)).is_some_and(|&m| m & (1 << 4) != 0))) { tris += 2; }
    tris
}

fn count_unit_cell_faces(prim: &ResolvedPrimitive, cells: &std::collections::HashMap<(i32,i32,i32), u8>) -> usize {
    let pos = prim.bounds.min();
    let (axis_tris, non_axis_tris) = compute_face_tris(prim.shape, &prim.orient_mat);
    let mut tris = non_axis_tris;
    for (i, &(dx, dy, dz)) in FACE_DIRS.iter().enumerate() {
        if axis_tris[i] == 0 { continue; }
        let neighbor = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
        let opposite = i ^ 1;
        let covered = cells.get(&neighbor).is_some_and(|&m| m & (1 << opposite) != 0);
        if !covered {
            tris += axis_tris[i];
        }
    }
    tris
}

/// For a given shape and orientation, return (per-face triangle counts, non-axis triangle count).
fn compute_face_tris(shape: PrimitiveShape, orient_mat: &Mat3) -> ([usize; 6], usize) {
    match shape {
        PrimitiveShape::Box => ([2; 6], 0),
        PrimitiveShape::Wedge => {
            let mut ft = [0usize; 6];
            // Identity: bottom(-Y)=2, back(-Z)=2, right(+X)=1, left(-X)=1
            for &(face_idx, tris) in &[(1usize, 2usize), (3, 2), (4, 1), (5, 1)] {
                let (dx, dy, dz) = FACE_DIRS[face_idx];
                let world = *orient_mat * Vec3::new(dx as f32, dy as f32, dz as f32);
                for (i, &(fx, fy, fz)) in FACE_DIRS.iter().enumerate() {
                    if world.dot(Vec3::new(fx as f32, fy as f32, fz as f32)) > 0.9 {
                        ft[i] = tris;
                        break;
                    }
                }
            }
            (ft, 2) // slope
        }
        PrimitiveShape::Corner => {
            let mut ft = [0usize; 6];
            // Identity: bottom(-Y)=1, back(-Z)=1, left(-X)=1
            for &(face_idx, tris) in &[(1usize, 1usize), (3, 1), (5, 1)] {
                let (dx, dy, dz) = FACE_DIRS[face_idx];
                let world = *orient_mat * Vec3::new(dx as f32, dy as f32, dz as f32);
                for (i, &(fx, fy, fz)) in FACE_DIRS.iter().enumerate() {
                    if world.dot(Vec3::new(fx as f32, fy as f32, fz as f32)) > 0.9 {
                        ft[i] = tris;
                        break;
                    }
                }
            }
            (ft, 1) // diagonal
        }
        PrimitiveShape::InverseCorner => {
            let mut ft = [0usize; 6];
            // Identity: +X=2(quad), +Y=2(quad), +Z=2(quad), -X=1(tri), -Y=1(tri), -Z=1(tri)
            for &(face_idx, tris) in &[(0usize, 2usize), (1, 1), (2, 2), (3, 1), (4, 2), (5, 1)] {
                let (dx, dy, dz) = FACE_DIRS[face_idx];
                let world = *orient_mat * Vec3::new(dx as f32, dy as f32, dz as f32);
                for (i, &(fx, fy, fz)) in FACE_DIRS.iter().enumerate() {
                    if world.dot(Vec3::new(fx as f32, fy as f32, fz as f32)) > 0.9 {
                        ft[i] = tris;
                        break;
                    }
                }
            }
            (ft, 1) // diagonal
        }
    }
}

/// True if any cell this union primitive occupies has at least one
/// subtract primitive touching it.
fn primitive_touches_subtracts(
    prim: &UnionPrimitive,
    subtract_cells: &std::collections::HashMap<CellKey, u64>,
) -> bool {
    if subtract_cells.is_empty() {
        return false;
    }
    let mn = prim.world_bounds.min();
    let mx = prim.world_bounds.max();
    let wmin = (
        floor_div(mn.0, prim.scale.0),
        floor_div(mn.1, prim.scale.1),
        floor_div(mn.2, prim.scale.2),
    );
    let wmax = (
        ceil_div(mx.0, prim.scale.0),
        ceil_div(mx.1, prim.scale.1),
        ceil_div(mx.2, prim.scale.2),
    );
    for z in wmin.2..wmax.2 {
        for y in wmin.1..wmax.1 {
            for x in wmin.0..wmax.0 {
                if subtract_cells.contains_key(&(x, y, z)) {
                    return true;
                }
            }
        }
    }
    false
}

fn enumerate_world_cells<F: FnMut(CellKey)>(
    bounds: &Bounds,
    scale: (i32, i32, i32),
    mut f: F,
) {
    let mn = bounds.min();
    let mx = bounds.max();
    // Use integer division; the spec guarantees cells for non-import
    // shapes are aligned at integer positions. For imported non-aligned
    // bounds, floor/ceil rounding keeps the behavior conservative.
    let wmin = (
        floor_div(mn.0, scale.0),
        floor_div(mn.1, scale.1),
        floor_div(mn.2, scale.2),
    );
    let wmax = (
        ceil_div(mx.0, scale.0),
        ceil_div(mx.1, scale.1),
        ceil_div(mx.2, scale.2),
    );
    for z in wmin.2..wmax.2 {
        for y in wmin.1..wmax.1 {
            for x in wmin.0..wmax.0 {
                f((x, y, z));
            }
        }
    }
}

fn floor_div(a: i32, b: i32) -> i32 {
    let q = a / b;
    let r = a % b;
    if r < 0 { q - 1 } else { q }
}

fn ceil_div(a: i32, b: i32) -> i32 {
    let q = a / b;
    let r = a % b;
    if r > 0 { q + 1 } else { q }
}

/// Compile one named-group subtree starting at `spec`. Walks unnamed
/// descendants into the same group; recursively compiles named
/// descendants as children.
fn compile_group(
    spec: &SpecNode,
    inherited_placement: Placement,
    scale: (i32, i32, i32),
    ctx: &CompileCtx<'_>,
    all_subtracts: &[SubtractPrimitive],
    parent_path: &str,
) -> CompiledShape {
    let mut group = GroupAccumulator::new();
    group.subtract_primitives.extend_from_slice(all_subtracts);
    walk_into_group(spec, inherited_placement, scale, &mut group, true, ctx, all_subtracts, parent_path);
    group.finish(spec.effective_name().map(str::to_string), spec.subtract, ctx.templates)
}

fn build_path(parent_path: &str, spec: &SpecNode) -> String {
    if let Some(name) = spec.effective_name() {
        if parent_path.is_empty() { name.to_string() } else { format!("{parent_path}/{name}") }
    } else {
        parent_path.to_string()
    }
}

fn is_hidden(path: &str, ctx: &CompileCtx<'_>) -> bool {
    !path.is_empty() && ctx.hidden.iter().any(|h| h == path)
}

fn walk_into_group(
    spec: &SpecNode,
    inherited_placement: Placement,
    scale: (i32, i32, i32),
    group: &mut GroupAccumulator,
    is_group_root: bool,
    ctx: &CompileCtx<'_>,
    all_subtracts: &[SubtractPrimitive],
    parent_path: &str,
) {
    let node_path = build_path(parent_path, spec);

    // Named non-root nodes create child groups (unless flattening).
    if !ctx.flatten && !is_group_root && spec.effective_name().is_some() {
        let child = compile_group(spec, inherited_placement, scale, ctx, all_subtracts, parent_path);
        group.children.push(child);
        return;
    }

    // Hidden nodes skip their own geometry and CSG but still walk
    // children so they appear in the parts tree.
    if is_hidden(&node_path, ctx) {
        if ctx.flatten { return; }
        if let Some(ref import_name) = spec.import {
            walk_import(spec, import_name, inherited_placement, scale, group, ctx, &node_path);
        } else {
            for child in &spec.children {
                walk_into_group(child, inherited_placement, scale, group, false, ctx, all_subtracts, &node_path);
            }
        }
        return;
    }

    for (local, _suffix) in &placements_for(spec) {
        let combined = compose_placements(inherited_placement, *local);
        if let Some(ref import_name) = spec.import {
            walk_import(spec, import_name, combined, scale, group, ctx, &node_path);
        } else {
            walk_node_body(spec, combined, scale, group, ctx, all_subtracts, &node_path);
        }
    }
}

/// Handle a node's bounds, shape, and children under a given placement.
/// Excludes combinator dispatch (which `walk_into_group` does).
fn walk_node_body(
    spec: &SpecNode,
    placement: Placement,
    scale: (i32, i32, i32),
    group: &mut GroupAccumulator,
    ctx: &CompileCtx<'_>,
    all_subtracts: &[SubtractPrimitive],
    node_path: &str,
) {
    if let Some((shape, orient_p)) = spec.primitive() {
        let Some(bounds) = spec.bounds else {
            for child in &spec.children {
                walk_into_group(child, placement, scale, group, false, ctx, all_subtracts, node_path);
            }
            return;
        };

        let size = bounds.size();
        if size.0 == 0 || size.1 == 0 || size.2 == 0 {
            return;
        }

        if spec.subtract {
            if ctx.is_direct {
                add_primitive(shape, &bounds, placement, scale, orient_p, spec, &mut group.preview_primitives);
            }
        } else {
            add_primitive(shape, &bounds, placement, scale, orient_p, spec, &mut group.union_primitives);
        }
    }

    for child in &spec.children {
        walk_into_group(child, placement, scale, group, false, ctx, all_subtracts, node_path);
    }
}

/// Record a Union primitive as a whole entry. At fusion time we'll
/// decide whether to render it as one multi-cell mesh or decompose
/// into unit cells, based on whether any sibling Subtract claims
/// cells inside its extent.
fn add_primitive(
    shape: PrimitiveShape,
    bounds: &Bounds,
    placement: Placement,
    scale: (i32, i32, i32),
    orient_p: Placement,
    spec: &SpecNode,
    target: &mut Vec<UnionPrimitive>,
) {
    let world_bounds = apply_placement_to_bounds(placement, *bounds);
    let orient_mat = super::csg::placement_to_mat3(compose_placements(placement, orient_p));
    let is_mirrored = orient_mat.determinant() < 0.0;
    let color = resolve_tags_color(&spec.tags);
    let emissive = resolve_tags_emissive(&spec.tags);

    target.push(UnionPrimitive {
        shape,
        world_bounds,
        orient_mat,
        color,
        emissive,
        is_mirrored,
        scale,
    });
}

/// Pre-pass: collect all subtract primitives from the spec tree,
/// respecting symmetry expansion but stopping at import boundaries.
/// The result is a flat list shared by every group in the compile.
fn collect_subtracts(
    spec: &SpecNode,
    placement: Placement,
    scale: (i32, i32, i32),
    hidden: &[String],
    out: &mut Vec<SubtractPrimitive>,
) {
    if spec.import.is_some() {
        return;
    }
    if spec.effective_name().is_some_and(|n| hidden.iter().any(|h| h == n)) {
        return;
    }
    for (local, _) in &placements_for(spec) {
        let combined = compose_placements(placement, *local);
        if spec.subtract {
            if let (Some((shape, orient_p)), Some(bounds)) = (spec.primitive(), spec.bounds) {
                let world_bounds = apply_placement_to_bounds(combined, bounds);
                let orient_mat = super::csg::placement_to_mat3(compose_placements(combined, orient_p));
                out.push(SubtractPrimitive {
                    shape,
                    world_bounds,
                    orient_mat,
                    scale,
                });
            }
        }
        for child in &spec.children {
            collect_subtracts(child, combined, scale, hidden, out);
        }
    }
}


fn walk_import(
    import_node: &SpecNode,
    import_name: &str,
    inherited_placement: Placement,
    parent_scale: (i32, i32, i32),
    group: &mut GroupAccumulator,
    ctx: &CompileCtx<'_>,
    parent_path: &str,
) {
    let imported = match ctx.registry.get_shape(import_name) {
        Some(parts) => parts.to_vec(),
        None => {
            error!("Import '{}' not found in registry", import_name);
            return;
        }
    };

    let Some(native_aabb) = aabb_for_parts(&imported, ctx.registry) else {
        warn!("Import '{}' has no computable AABB — skipping", import_name);
        return;
    };
    // Use the node's explicit bounds, or fall back to the imported
    // shape's native AABB. Don't use aabb() here — it expands symmetry,
    // but symmetry is handled by the caller invoking walk_import once
    // per placement.
    let placement_bounds = import_node.bounds.unwrap_or(native_aabb);

    let remap_scale = Bounds::remap_scale(&native_aabb);
    let new_scale = (
        parent_scale.0 * remap_scale.0,
        parent_scale.1 * remap_scale.1,
        parent_scale.2 * remap_scale.2,
    );

    let mut remapped = imported;
    remap_bounds_for_parts(&mut remapped, &native_aabb, &placement_bounds, ctx.registry);

    // Collect subtracts from the imported parts so they carve into
    // sibling unions within the same import.
    let mut import_subtracts = Vec::new();
    for part in &remapped {
        collect_subtracts(part, inherited_placement, new_scale, ctx.hidden, &mut import_subtracts);
    }

    // The imported parts are inlined into THIS group. Subtract previews
    // are suppressed inside imports — the imported shape's own direct
    // compile shows them.
    // Imports suppress subtract previews and use their own subtract list.
    let import_ctx = CompileCtx {
        is_direct: false,
        ..*ctx
    };
    for part in &remapped {
        walk_into_group(
            part,
            inherited_placement,
            new_scale,
            group,
            false,
            &import_ctx,
            &import_subtracts,
            parent_path,
        );
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::spec::AnimState;

    fn leaf_box(name: &str, bounds: Bounds) -> SpecNode {
        SpecNode {
            name: Some(name.to_string()),
            bounds: Some(bounds),
            corner: None,
            clip: None,
            faces: None,
            tags: vec![],
            import: None,
            children: vec![],
            symmetry: vec![],
            subtract: false,
            animations: Vec::<AnimState>::new(),
        }
    }

    fn overall_bounds(compiled: &CompiledShape) -> ([f32; 3], [f32; 3]) {
        let mut mn = [f32::INFINITY; 3];
        let mut mx = [f32::NEG_INFINITY; 3];
        fn walk(node: &CompiledShape, mn: &mut [f32; 3], mx: &mut [f32; 3]) {
            for fused in &node.meshes {
                let (fmn, fmx) = mesh_bounds_raw(&fused.mesh);
                for i in 0..3 {
                    mn[i] = mn[i].min(fmn[i]);
                    mx[i] = mx[i].max(fmx[i]);
                }
            }
            for c in &node.children {
                walk(c, mn, mx);
            }
        }
        walk(compiled, &mut mn, &mut mx);
        (mn, mx)
    }

    fn mesh_bounds_raw(mesh: &RawMesh) -> ([f32; 3], [f32; 3]) {
        let mut mn = [f32::INFINITY; 3];
        let mut mx = [f32::NEG_INFINITY; 3];
        for p in &mesh.positions {
            for i in 0..3 {
                if p[i] < mn[i] { mn[i] = p[i]; }
                if p[i] > mx[i] { mx[i] = p[i]; }
            }
        }
        (mn, mx)
    }

    /// A multi-cell Box primitive must render as one single mesh whose
    /// vertex extent matches the authored bounds exactly. No decomposition.
    #[test]
    fn stretchy_box_renders_single_mesh_spanning_full_bounds() {
        let parts = vec![leaf_box("stretched", Bounds(1, 2, 3, 4, 5, 6))];
        let compiled = compile(&parts, &crate::registry::AssetRegistry::default(), &[]);

        let (mn, mx) = overall_bounds(&compiled);
        assert_eq!(mn, [1.0, 2.0, 3.0]);
        assert_eq!(mx, [4.0, 5.0, 6.0]);
    }

    /// Transitive stretching: shape A contains a stretchy leaf inside
    /// its native AABB. Shape B imports A at a placement that rescales
    /// A's coordinate space. The leaf must appear at the correctly
    /// rescaled world position — proving the stretch propagates
    /// through composition.
    #[test]
    fn transitive_stretching_through_nested_import() {
        // Shape A has two parts spanning a 4×4×4 native AABB:
        //   spacer: (-2, -2, -2, 0, 2, 2)  — the -X half
        //   leaf:   (0, -2, -2, 2, 2, 2)   — the +X half
        let shape_a_parts = vec![
            leaf_box("spacer", Bounds(-2, -2, -2, 0, 2, 2)),
            leaf_box("leaf", Bounds(0, -2, -2, 2, 2, 2)),
        ];

        let mut registry = crate::registry::AssetRegistry::default();
        registry.test_insert_shape("shape_a", shape_a_parts);

        // Shape B imports A at placement (-4, 0, -4, 4, 8, 4) — an
        // 8×8×8 cube. A's native 4×4×4 stretches by ×2 in each axis.
        let mut import_node = leaf_box("imported", Bounds(-4, 0, -4, 4, 8, 4));
        import_node.import = Some("shape_a".into());
        let shape_b_parts = vec![import_node];

        let compiled = compile(&shape_b_parts, &registry, &[]);

        // A's native AABB is (-2..2, -2..2, -2..2). After remap into
        // (-4..4, 0..8, -4..4) (×2 each axis), both halves stretch:
        //   spacer: x -2→0 → -4→0
        //   leaf:   x 0→2  → 0→4
        //   y: -2→2 → 0→8, z: -2→2 → -4→4
        // Combined mesh spans the full placement bounds.
        let (mn, mx) = overall_bounds(&compiled);
        assert_eq!(mn, [-4.0, 0.0, -4.0], "stretched mesh min");
        assert_eq!(mx, [4.0, 8.0, 4.0], "stretched mesh max");
    }

    #[test]
    fn greedy_merge_fuses_adjacent_cells_into_one_box() {
        // 2×2×2 cube of unit cells → one box.
        let mut cells = Vec::new();
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    cells.push((x, y, z));
                }
            }
        }
        let merged = greedy_merge(cells);
        assert_eq!(merged.len(), 1, "8 unit cells should merge into 1 box");
        assert_eq!(merged[0], Bounds(0, 0, 0, 2, 2, 2));
    }

    #[test]
    fn greedy_merge_keeps_disjoint_cells_separate() {
        let cells = vec![(0, 0, 0), (5, 5, 5)];
        let merged = greedy_merge(cells);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn eight_unit_boxes_production_stats_twelve_triangles() {
        // 8 unit boxes composing a 2×2×2 cube: production path fuses
        // them into one box = 12 triangles, 1 draw call.
        let mut parts = Vec::new();
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    parts.push(SpecNode {
                        name: Some(format!("b_{x}_{y}_{z}")),
                        bounds: Some(Bounds(x, y, z, x + 1, y + 1, z + 1)),
                        corner: None,
                        clip: None,
                        faces: None,
                        tags: vec!["red".into()],
                        import: None,
                        children: vec![],
                        symmetry: vec![],
                        subtract: false,
                        animations: vec![],
                    });
                }
            }
        }
        let stats = production_stats(&parts, &crate::registry::AssetRegistry::default());
        assert_eq!(stats.triangles, 12, "8 unit boxes should fuse into 12 triangles");
        assert_eq!(stats.draw_calls, 1);
    }

}


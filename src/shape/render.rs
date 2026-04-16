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
    Bounds, Combinator, Facing, Mirroring, Orientation, Placement, PrimitiveShape,
    Rotation, SignedAxis, SpecNode,
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

/// Resolve an ordered tag list into a color. The first recognized color
/// tag wins; unrecognized tags are silently skipped (future textures,
/// effects, etc.). Case-insensitive. Returns default grey if no color
/// tag is found.
pub fn resolve_tags_color(tags: &[String]) -> Color3 {
    for tag in tags {
        match tag.to_ascii_lowercase().as_str() {
            "red"     => return Color3(3, 0, 0),
            "green"   => return Color3(0, 3, 0),
            "blue"    => return Color3(0, 0, 3),
            "cyan"    => return Color3(0, 3, 3),
            "magenta" => return Color3(3, 0, 3),
            "yellow"  => return Color3(3, 3, 0),
            "white"   => return Color3(3, 3, 3),
            "black"   => return Color3(0, 0, 0),
            _ => {}
        }
    }
    Color3(1, 1, 1) // default grey
}

/// Check whether the tag list includes "emissive" (case-insensitive).
pub fn resolve_tags_emissive(tags: &[String]) -> bool {
    tags.iter().any(|t| t.eq_ignore_ascii_case("emissive"))
}

// =====================================================================
// Orientation → Mat3 conversion
// =====================================================================

/// Convert a discrete `Orientation` tuple plus an accumulated placement
/// from symmetry expansion into the final world→mesh matrix.
pub fn orientation_to_mat3(orient: &Orientation, placement: Placement) -> Mat3 {
    let base = base_orientation_matrix(orient);
    apply_placement_to_mat3(placement, base)
}

pub fn base_orientation_matrix(orient: &Orientation) -> Mat3 {
    let facing = facing_matrix(orient.facing());
    let mirror = match orient.mirroring() {
        Mirroring::NoMirror => Mat3::IDENTITY,
        Mirroring::Mirror => Mat3::from_cols(Vec3::NEG_X, Vec3::Y, Vec3::Z),
    };
    let rotation = rotation_matrix(orient.rotation());
    rotation * mirror * facing
}

fn facing_matrix(facing: Facing) -> Mat3 {
    use std::f32::consts::FRAC_PI_2;
    match facing {
        Facing::Front => Mat3::IDENTITY,
        Facing::Back => Mat3::from_quat(Quat::from_rotation_y(std::f32::consts::PI)),
        Facing::Left => Mat3::from_quat(Quat::from_rotation_y(-FRAC_PI_2)),
        Facing::Right => Mat3::from_quat(Quat::from_rotation_y(FRAC_PI_2)),
        Facing::Top => Mat3::from_quat(Quat::from_rotation_x(-FRAC_PI_2)),
        Facing::Bottom => Mat3::from_quat(Quat::from_rotation_x(FRAC_PI_2)),
    }
}

fn rotation_matrix(rotation: Rotation) -> Mat3 {
    use std::f32::consts::{FRAC_PI_2, PI};
    match rotation {
        Rotation::NoRotation => Mat3::IDENTITY,
        Rotation::RotateClockwise => Mat3::from_quat(Quat::from_rotation_z(-FRAC_PI_2)),
        Rotation::RotateHalf => Mat3::from_quat(Quat::from_rotation_z(PI)),
        Rotation::RotateCounter => Mat3::from_quat(Quat::from_rotation_z(FRAC_PI_2)),
    }
}

fn apply_placement_to_mat3(placement: Placement, m: Mat3) -> Mat3 {
    let apply_to_col = |col: Vec3| -> Vec3 {
        Vec3::new(
            signed_axis_project(placement.0, col),
            signed_axis_project(placement.1, col),
            signed_axis_project(placement.2, col),
        )
    };
    Mat3::from_cols(
        apply_to_col(m.x_axis),
        apply_to_col(m.y_axis),
        apply_to_col(m.z_axis),
    )
}

fn signed_axis_project(sa: SignedAxis, v: Vec3) -> f32 {
    let component = match sa {
        SignedAxis::PosX | SignedAxis::NegX => v.x,
        SignedAxis::PosY | SignedAxis::NegY => v.y,
        SignedAxis::PosZ | SignedAxis::NegZ => v.z,
    };
    if matches!(sa, SignedAxis::PosX | SignedAxis::PosY | SignedAxis::PosZ) {
        component
    } else {
        -component
    }
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
        );
    }
    group.finish(None, ctx.templates)
}

struct CompileCtx<'a> {
    registry: &'a AssetRegistry,
    templates: &'a PrimitiveTemplates,
    hidden: &'a [String],
    is_direct: bool,
}

struct PrimitiveTemplates {
    box_mesh: RawMesh,
    wedge_mesh: RawMesh,
    corner_mesh: RawMesh,
}

impl PrimitiveTemplates {
    fn new() -> Self {
        Self {
            box_mesh: create_raw_mesh(PrimitiveShape::Box),
            wedge_mesh: create_raw_mesh(PrimitiveShape::Wedge),
            corner_mesh: create_raw_mesh(PrimitiveShape::Corner),
        }
    }

    fn get(&self, shape: PrimitiveShape) -> &RawMesh {
        match shape {
            PrimitiveShape::Box => &self.box_mesh,
            PrimitiveShape::Wedge => &self.wedge_mesh,
            PrimitiveShape::Corner => &self.corner_mesh,
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

impl GroupAccumulator {
    fn new() -> Self {
        Self {
            union_primitives: Vec::new(),
            subtract_primitives: Vec::new(),
            preview_primitives: Vec::new(),
            children: Vec::new(),
        }
    }

    fn finish(self, name: Option<String>, templates: &PrimitiveTemplates) -> CompiledShape {
        // Per-cell subtract signature: for each cell any subtract
        // primitive touches, compute the actual subtract volume by
        // sampling at the cell's world position. Multiple subtracts
        // at the same cell are OR'd together.
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

        let mut emissive_mesh = RawMesh::default();
        let mut normal_mesh = RawMesh::default();
        let mut emissive_mirrored = false;
        let mut normal_mirrored = false;

        for prim in &self.union_primitives {
            // If the primitive's cells don't intersect any subtract,
            // render as a single multi-cell mesh — preserving authored
            // stretchy primitives. Otherwise decompose into unit cells
            // and run per-cell CSG (Option A semantics: the result must
            // be expressible as a primitive or nothing; anything else
            // is an authoring error).
            let target = if prim.emissive {
                &mut emissive_mesh
            } else {
                &mut normal_mesh
            };
            let (cr, cg, cb) = prim.color.to_rgb();
            let rgba = [cr, cg, cb, 1.0];

            let needs_decompose = primitive_touches_subtracts(prim, &subtract_cells);
            if !needs_decompose {
                let mesh_tf = compute_mesh_transform(
                    prim.shape,
                    &prim.world_bounds,
                    &prim.orient_mat,
                    prim.scale,
                );
                target.append_transformed(templates.get(prim.shape), &mesh_tf, rgba);
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
                            let cell_bounds = Bounds(x, y, z, x + 1, y + 1, z + 1);

                            let sub_sig = subtract_cells.get(&cell).copied().unwrap_or(0);
                            if sub_sig == 0 {
                                // No subtract here — render the
                                // union's unit primitive as-is.
                                let mesh_tf = compute_mesh_transform(
                                    prim.shape,
                                    &cell_bounds,
                                    &prim.orient_mat,
                                    (1, 1, 1),
                                );
                                target.append_transformed(
                                    templates.get(prim.shape),
                                    &mesh_tf,
                                    rgba,
                                );
                            } else {
                                let result = super::csg::cell_subtract_with_sig(
                                    (prim.shape, prim.orient_mat),
                                    sub_sig,
                                );
                                match result {
                                    super::csg::CellResult::Empty => {
                                        // Fully carved — skip.
                                    }
                                    super::csg::CellResult::Keep {
                                        shape: rs,
                                        orient_mat: rm,
                                    } => {
                                        let mesh_tf = compute_mesh_transform(
                                            rs, &cell_bounds, &rm, (1, 1, 1),
                                        );
                                        target.append_transformed(
                                            templates.get(rs),
                                            &mesh_tf,
                                            rgba,
                                        );
                                    }
                                    super::csg::CellResult::NotRepresentable {
                                        result_signature,
                                    } => {
                                        error!(
                                            "subtract result at cell {:?} for '{}' is not a primitive \
                                             (minuend={:?}, signature={:016x})",
                                            cell,
                                            name.as_deref().unwrap_or("unnamed"),
                                            prim.shape,
                                            result_signature
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if prim.is_mirrored {
                if prim.emissive {
                    emissive_mirrored = true;
                } else {
                    normal_mirrored = true;
                }
            }
        }

        let mut meshes = Vec::new();
        if !normal_mesh.is_empty() {
            meshes.push(FusedMesh {
                mesh: normal_mesh,
                emissive: false,
                contains_mirrored: normal_mirrored,
                subtract_preview: false,
            });
        }
        if !emissive_mesh.is_empty() {
            meshes.push(FusedMesh {
                mesh: emissive_mesh,
                emissive: true,
                contains_mirrored: emissive_mirrored,
                subtract_preview: false,
            });
        }

        // Subtract preview: translucent overlay of subtract primitives
        // so the author can see what volume is being carved.
        let mut preview_mesh = RawMesh::default();
        let mut preview_mirrored = false;
        for prim in &self.preview_primitives {
            let (cr, cg, cb) = prim.color.to_rgb();
            let rgba = [cr, cg, cb, 0.3];
            let mesh_tf = compute_mesh_transform(
                prim.shape,
                &prim.world_bounds,
                &prim.orient_mat,
                prim.scale,
            );
            preview_mesh.append_transformed(templates.get(prim.shape), &mesh_tf, rgba);
            if prim.is_mirrored {
                preview_mirrored = true;
            }
        }
        if !preview_mesh.is_empty() {
            meshes.push(FusedMesh {
                mesh: preview_mesh,
                emissive: false,
                contains_mirrored: preview_mirrored,
                subtract_preview: true,
            });
        }

        CompiledShape {
            name,
            local_transform: Transform::IDENTITY,
            meshes,
            children: self.children,
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
) -> CompiledShape {
    let mut group = GroupAccumulator::new();
    group.subtract_primitives.extend_from_slice(all_subtracts);
    walk_into_group(spec, inherited_placement, scale, &mut group, true, ctx, all_subtracts);
    group.finish(spec.effective_name().map(str::to_string), ctx.templates)
}

fn is_hidden(spec: &SpecNode, ctx: &CompileCtx<'_>) -> bool {
    if let Some(name) = spec.effective_name() {
        ctx.hidden.iter().any(|h| h == name)
    } else {
        false
    }
}

fn walk_into_group(
    spec: &SpecNode,
    inherited_placement: Placement,
    scale: (i32, i32, i32),
    group: &mut GroupAccumulator,
    is_group_root: bool,
    ctx: &CompileCtx<'_>,
    all_subtracts: &[SubtractPrimitive],
) {
    if !is_group_root && spec.effective_name().is_some() {
        let child = compile_group(spec, inherited_placement, scale, ctx, all_subtracts);
        group.children.push(child);
        return;
    }

    // Hidden nodes skip their own geometry and CSG but still walk
    // children so they appear in the parts tree.
    if is_hidden(spec, ctx) {
        match spec.combinator() {
            Combinator::Import(import_name) => {
                walk_import(spec, import_name, inherited_placement, scale, group, ctx);
            }
            _ => {
                for child in &spec.children {
                    walk_into_group(child, inherited_placement, scale, group, false, ctx, all_subtracts);
                }
            }
        }
        return;
    }

    match spec.combinator() {
        Combinator::Symmetry(sym) => {
            for (local, _suffix) in placements(sym) {
                let combined = compose_placements(inherited_placement, *local);
                walk_node_body(spec, combined, scale, group, ctx, all_subtracts);
            }
        }
        Combinator::Import(import_name) => {
            walk_import(spec, import_name, inherited_placement, scale, group, ctx);
        }
        Combinator::None => {
            walk_node_body(spec, inherited_placement, scale, group, ctx, all_subtracts);
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
) {
    if let Some(shape) = spec.shape {
        let Some(bounds) = spec.bounds else {
            warn!(
                "Shape '{}' has no bounds — skipping geometry",
                spec.effective_name().unwrap_or("unnamed")
            );
            for child in &spec.children {
                walk_into_group(child, placement, scale, group, false, ctx, all_subtracts);
            }
            return;
        };

        let size = bounds.size();
        if size.0 == 0 || size.1 == 0 || size.2 == 0 {
            error!(
                "'{}' has zero-size bounds ({},{},{}) — skipping",
                spec.effective_name().unwrap_or("unnamed"),
                size.0,
                size.1,
                size.2
            );
            return;
        }

        if spec.subtract {
            if ctx.is_direct {
                add_preview_primitive(shape, &bounds, placement, scale, spec.orient, spec, group);
            }
        } else {
            add_union_primitive(shape, &bounds, placement, scale, spec.orient, spec, group);
        }
    }

    for child in &spec.children {
        walk_into_group(child, placement, scale, group, false, ctx, all_subtracts);
    }
}

/// Record a Union primitive as a whole entry. At fusion time we'll
/// decide whether to render it as one multi-cell mesh or decompose
/// into unit cells, based on whether any sibling Subtract claims
/// cells inside its extent.
fn add_union_primitive(
    shape: PrimitiveShape,
    bounds: &Bounds,
    placement: Placement,
    scale: (i32, i32, i32),
    orient: Orientation,
    spec: &SpecNode,
    group: &mut GroupAccumulator,
) {
    let world_bounds = apply_placement_to_bounds(placement, *bounds);
    let orient_mat = orientation_to_mat3(&orient, placement);
    let is_mirrored = orient_mat.determinant() < 0.0;

    let color = resolve_tags_color(&spec.tags);
    let emissive = resolve_tags_emissive(&spec.tags);

    group.union_primitives.push(UnionPrimitive {
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
    let sym = spec.symmetry;
    for (local, _) in placements(sym) {
        let combined = compose_placements(placement, *local);
        if spec.subtract {
            if let (Some(shape), Some(bounds)) = (spec.shape, spec.bounds) {
                let world_bounds = apply_placement_to_bounds(combined, bounds);
                let orient_mat = orientation_to_mat3(&spec.orient, combined);
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

/// Record a subtract primitive as a translucent preview mesh so the
/// author can see the subtractive volume. Uses the authored color;
/// alpha is applied at fusion time.
fn add_preview_primitive(
    shape: PrimitiveShape,
    bounds: &Bounds,
    placement: Placement,
    scale: (i32, i32, i32),
    orient: Orientation,
    spec: &SpecNode,
    group: &mut GroupAccumulator,
) {
    let world_bounds = apply_placement_to_bounds(placement, *bounds);
    let orient_mat = orientation_to_mat3(&orient, placement);
    let is_mirrored = orient_mat.determinant() < 0.0;

    let color = resolve_tags_color(&spec.tags);

    group.preview_primitives.push(UnionPrimitive {
        shape,
        world_bounds,
        orient_mat,
        color,
        emissive: false,
        is_mirrored,
        scale,
    });
}

fn walk_import(
    import_node: &SpecNode,
    import_name: &str,
    inherited_placement: Placement,
    parent_scale: (i32, i32, i32),
    group: &mut GroupAccumulator,
    ctx: &CompileCtx<'_>,
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
    let placement_bounds = import_node.aabb(ctx.registry).unwrap_or(native_aabb);

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
        );
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::spec::{AnimState, Orientation, Symmetry};

    fn leaf_box(name: &str, bounds: Bounds) -> SpecNode {
        SpecNode {
            name: Some(name.to_string()),
            shape: Some(PrimitiveShape::Box),
            bounds: Some(bounds),
            orient: Orientation::default(),
            tags: vec![],
            import: None,
            children: vec![],
            symmetry: Symmetry::Single,
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
        import_node.shape = None;
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
}


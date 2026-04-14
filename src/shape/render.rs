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
//! `combine: Subtract` all in integer cell space, then bakes the
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
    apply_placement_to_bounds, compose_placements, identity_placement,
    placements, Bounds, CombineMode, Combinator, Facing, Mirroring, Orientation,
    Placement, PrimitiveShape, Rotation, SignedAxis, SpecNode,
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
}

// =====================================================================
// Colors
// =====================================================================

pub type ColorMap = Vec<(String, Color3)>;

pub fn merge_colors(parent: &ColorMap, child: &ColorMap) -> ColorMap {
    let mut merged = child.clone();
    for (pk, pv) in parent {
        if let Some(entry) = merged.iter_mut().find(|(k, _)| k == pk) {
            entry.1 = *pv;
        } else {
            merged.push((pk.clone(), *pv));
        }
    }
    merged
}

pub fn resolve_color(name: &str, colors: &ColorMap) -> Color3 {
    colors
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| {
            warn!("Color '{}' not found in color map, using default gray", name);
            Color3(1, 1, 1)
        })
}

fn apply_color_remapping(
    import_node: &SpecNode,
    imported_colors: &ColorMap,
    parent_colors: &ColorMap,
) -> ColorMap {
    if !import_node.color_map.is_empty() && !import_node.colors.is_empty() {
        warn!(
            "Node '{}' specifies both color_map and colors — using color_map",
            import_node.name.as_deref().unwrap_or("unnamed")
        );
    }

    if !import_node.color_map.is_empty() {
        imported_colors
            .iter()
            .map(|(child_name, child_val)| {
                if let Some(parent_name) = import_node.color_map.get(child_name) {
                    let resolved = resolve_color(parent_name, parent_colors);
                    (child_name.clone(), resolved)
                } else {
                    (child_name.clone(), *child_val)
                }
            })
            .collect()
    } else if !import_node.colors.is_empty() {
        imported_colors
            .iter()
            .enumerate()
            .map(|(i, (child_name, child_val))| {
                if let Some(parent_name) = import_node.colors.get(i) {
                    let resolved = resolve_color(parent_name, parent_colors);
                    (child_name.clone(), resolved)
                } else {
                    (child_name.clone(), *child_val)
                }
            })
            .collect()
    } else {
        merge_colors(parent_colors, imported_colors)
    }
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

fn base_orientation_matrix(orient: &Orientation) -> Mat3 {
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
/// per-part meshes. This is the only function that consumes `SpecNode`
/// fields for render purposes.
pub fn compile(
    spec: &SpecNode,
    registry: &AssetRegistry,
) -> CompiledShape {
    // Pre-compute the primitive templates once per compile to avoid
    // rebuilding them for every cell.
    let templates = PrimitiveTemplates::new();
    let ctx = CompileCtx {
        registry,
        templates: &templates,
    };
    let colors = spec.palette.clone();
    compile_group(
        spec,
        identity_placement(),
        (1, 1, 1),
        &colors,
        &ctx,
    )
}

struct CompileCtx<'a> {
    registry: &'a AssetRegistry,
    templates: &'a PrimitiveTemplates,
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

/// Accumulator for primitives belonging to one group (one compiled part).
struct GroupAccumulator {
    union_primitives: Vec<UnionPrimitive>,
    /// Subtract contributions collected as raw bounds; converted to
    /// a cell set at fusion time.
    subtract_bounds: Vec<(Bounds, Placement, (i32, i32, i32))>,
    children: Vec<CompiledShape>,
}

/// Identity of an integer world cell, used to match Union/Subtract
/// overlaps. Union cells whose position matches a Subtract position
/// get dropped before fusion.
type CellKey = (i32, i32, i32);

impl GroupAccumulator {
    fn new() -> Self {
        Self {
            union_primitives: Vec::new(),
            subtract_bounds: Vec::new(),
            children: Vec::new(),
        }
    }

    fn finish(self, name: Option<String>, templates: &PrimitiveTemplates) -> CompiledShape {
        // Turn subtract_bounds into a world-cell set.
        let mut subtract_cells: std::collections::HashSet<CellKey> =
            std::collections::HashSet::new();
        for (bounds, placement, scale) in &self.subtract_bounds {
            let transformed = apply_placement_to_bounds(*placement, *bounds);
            enumerate_world_cells(&transformed, *scale, |cell| {
                subtract_cells.insert(cell);
            });
        }

        let mut emissive_mesh = RawMesh::default();
        let mut normal_mesh = RawMesh::default();
        let mut emissive_mirrored = false;
        let mut normal_mirrored = false;

        for prim in &self.union_primitives {
            // If the primitive's cells don't intersect the subtract
            // set, render it as a single multi-cell mesh — preserving
            // the authored multi-cell shape (a 2x2x2 Wedge becomes one
            // big wedge, not 8 stacked unit wedges). If any of its
            // cells ARE subtracted, decompose into unit cells and drop
            // the subtracted ones.
            let target = if prim.emissive {
                &mut emissive_mesh
            } else {
                &mut normal_mesh
            };
            let (cr, cg, cb) = prim.color.to_rgb();
            let rgba = [cr, cg, cb, 1.0];

            let needs_decompose = primitive_overlaps_subtract(prim, &subtract_cells);
            if !needs_decompose {
                let mesh_tf = compute_mesh_transform(
                    prim.shape,
                    &prim.world_bounds,
                    &prim.orient_mat,
                    prim.scale,
                );
                target.append_transformed(templates.get(prim.shape), &mesh_tf, rgba);
            } else {
                // Per-cell decomposition: walk world cells, skip any
                // that are in the subtract set, render the rest as
                // unit primitives.
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
                            if subtract_cells.contains(&(x, y, z)) {
                                continue;
                            }
                            let cell_bounds = Bounds(x, y, z, x + 1, y + 1, z + 1);
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
            });
        }
        if !emissive_mesh.is_empty() {
            meshes.push(FusedMesh {
                mesh: emissive_mesh,
                emissive: true,
                contains_mirrored: emissive_mirrored,
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

/// Test whether any integer cell inside the primitive's world bounds
/// is claimed by the subtract set. If none are, the primitive can be
/// rendered as a single multi-cell mesh.
fn primitive_overlaps_subtract(
    prim: &UnionPrimitive,
    subtract_cells: &std::collections::HashSet<CellKey>,
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
                if subtract_cells.contains(&(x, y, z)) {
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
    inherited_colors: &ColorMap,
    ctx: &CompileCtx<'_>,
) -> CompiledShape {
    let mut group = GroupAccumulator::new();
    walk_into_group(
        spec,
        inherited_placement,
        scale,
        inherited_colors,
        &mut group,
        /* is_group_root */ true,
        ctx,
    );
    group.finish(spec.name.clone(), ctx.templates)
}

fn walk_into_group(
    spec: &SpecNode,
    inherited_placement: Placement,
    scale: (i32, i32, i32),
    inherited_colors: &ColorMap,
    group: &mut GroupAccumulator,
    is_group_root: bool,
    ctx: &CompileCtx<'_>,
) {
    let colors = if spec.palette.is_empty() {
        inherited_colors.clone()
    } else {
        merge_colors(inherited_colors, &spec.palette)
    };

    // Named non-root nodes start their own group as a child of this one.
    if !is_group_root && spec.name.is_some() {
        let child = compile_group(spec, inherited_placement, scale, &colors, ctx);
        group.children.push(child);
        return;
    }

    match spec.combinator() {
        Combinator::Symmetry(sym) => {
            for (local, _suffix) in placements(sym) {
                let combined = compose_placements(inherited_placement, *local);
                // Walk the same node's non-symmetric body (its bounds,
                // shape, and children) for each placement.
                walk_node_body(spec, combined, scale, &colors, group, ctx);
            }
        }
        Combinator::Import(import_name) => {
            walk_import(
                spec,
                import_name,
                inherited_placement,
                scale,
                &colors,
                group,
                ctx,
            );
        }
        Combinator::None => {
            walk_node_body(spec, inherited_placement, scale, &colors, group, ctx);
        }
    }
}

/// Handle a node's bounds, shape, and children under a given placement.
/// Excludes combinator dispatch (which `walk_into_group` does).
fn walk_node_body(
    spec: &SpecNode,
    placement: Placement,
    scale: (i32, i32, i32),
    colors: &ColorMap,
    group: &mut GroupAccumulator,
    ctx: &CompileCtx<'_>,
) {
    if let Some(shape) = spec.shape {
        let Some(bounds) = spec.bounds else {
            warn!(
                "Shape '{}' has no bounds — skipping geometry",
                spec.name.as_deref().unwrap_or("unnamed")
            );
            for child in &spec.children {
                walk_into_group(child, placement, scale, colors, group, false, ctx);
            }
            return;
        };

        let size = bounds.size();
        if size.0 == 0 || size.1 == 0 || size.2 == 0 {
            error!(
                "'{}' has zero-size bounds ({},{},{}) — skipping",
                spec.name.as_deref().unwrap_or("unnamed"),
                size.0,
                size.1,
                size.2
            );
            return;
        }

        match spec.combine {
            CombineMode::Subtract => {
                // Record the bounds so finish() can remove overlapping
                // Union cells. The orient/shape of a Subtract child is
                // irrelevant — only its cell set matters.
                group
                    .subtract_bounds
                    .push((bounds, placement, scale));
            }
            CombineMode::Union => {
                add_union_primitive(
                    shape, &bounds, placement, scale, spec.orient, colors, spec, group,
                );
            }
        }
    }

    for child in &spec.children {
        walk_into_group(child, placement, scale, colors, group, false, ctx);
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
    colors: &ColorMap,
    spec: &SpecNode,
    group: &mut GroupAccumulator,
) {
    let world_bounds = apply_placement_to_bounds(placement, *bounds);
    let orient_mat = orientation_to_mat3(&orient, placement);
    let is_mirrored = orient_mat.determinant() < 0.0;

    let color = spec
        .color
        .as_ref()
        .map(|name| resolve_color(name, colors))
        .unwrap_or_else(|| {
            warn!(
                "Shape '{}' has no color specified",
                spec.name.as_deref().unwrap_or("unnamed")
            );
            Color3(1, 1, 1)
        });

    group.union_primitives.push(UnionPrimitive {
        shape,
        world_bounds,
        orient_mat,
        color,
        emissive: spec.emissive,
        is_mirrored,
        scale,
    });
}

fn walk_import(
    import_node: &SpecNode,
    import_name: &str,
    inherited_placement: Placement,
    parent_scale: (i32, i32, i32),
    colors: &ColorMap,
    group: &mut GroupAccumulator,
    ctx: &CompileCtx<'_>,
) {
    let imported = match ctx.registry.get_shape(import_name) {
        Some(shape) => shape.clone(),
        None => {
            error!("Import '{}' not found in registry", import_name);
            return;
        }
    };

    let Some(native_aabb) = imported.compute_aabb() else {
        warn!("Import '{}' has no computable AABB — skipping", import_name);
        return;
    };
    let placement_bounds = import_node.bounds.unwrap_or(native_aabb);

    let remap_scale = Bounds::remap_scale(&native_aabb);
    let new_scale = (
        parent_scale.0 * remap_scale.0,
        parent_scale.1 * remap_scale.1,
        parent_scale.2 * remap_scale.2,
    );

    let mut remapped = imported;
    remapped.remap_bounds(&native_aabb, &placement_bounds);

    let import_colors = apply_color_remapping(import_node, &remapped.palette, colors);

    // The imported subtree's top-level node becomes part of THIS group
    // (same named part as the import_node). Walk it as if it were an
    // inlined child, not a new named group.
    walk_node_body(&remapped, inherited_placement, new_scale, &import_colors, group, ctx);
    for child in &remapped.children {
        walk_into_group(
            child,
            inherited_placement,
            new_scale,
            &import_colors,
            group,
            false,
            ctx,
        );
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::spec::{AnimState, CombineMode, Orientation, Symmetry};
    use std::collections::HashMap;

    fn leaf_box(name: &str, bounds: Bounds) -> SpecNode {
        SpecNode {
            name: Some(name.to_string()),
            shape: Some(PrimitiveShape::Box),
            bounds: Some(bounds),
            orient: Orientation::default(),
            palette: vec![],
            color: None,
            emissive: false,
            import: None,
            color_map: HashMap::new(),
            colors: vec![],
            children: vec![],
            symmetry: Symmetry::Single,
            combine: CombineMode::Union,
            animations: Vec::<AnimState>::new(),
        }
    }

    fn empty_container(name: &str, children: Vec<SpecNode>) -> SpecNode {
        let mut node = leaf_box(name, Bounds(0, 0, 0, 1, 1, 1));
        node.shape = None;
        node.bounds = None;
        node.children = children;
        node
    }

    fn mesh_bounds(mesh: &RawMesh) -> ([f32; 3], [f32; 3]) {
        let mut mn = [f32::INFINITY; 3];
        let mut mx = [f32::NEG_INFINITY; 3];
        for p in &mesh.positions {
            for i in 0..3 {
                if p[i] < mn[i] {
                    mn[i] = p[i];
                }
                if p[i] > mx[i] {
                    mx[i] = p[i];
                }
            }
        }
        (mn, mx)
    }

    fn find_fused_mesh(compiled: &CompiledShape) -> &RawMesh {
        fn walk<'a>(node: &'a CompiledShape) -> Option<&'a RawMesh> {
            if let Some(m) = node.meshes.first() {
                return Some(&m.mesh);
            }
            for c in &node.children {
                if let Some(m) = walk(c) {
                    return Some(m);
                }
            }
            None
        }
        walk(compiled).expect("expected at least one fused mesh in compiled tree")
    }

    /// A multi-cell Box primitive must render as one single mesh whose
    /// vertex extent matches the authored bounds exactly. No decomposition.
    #[test]
    fn stretchy_box_renders_single_mesh_spanning_full_bounds() {
        let spec = leaf_box("stretched", Bounds(1, 2, 3, 4, 5, 6));
        let compiled = compile(&spec, &crate::registry::AssetRegistry::default());

        // One primitive → one fused mesh on the root CompiledShape.
        assert_eq!(compiled.meshes.len(), 1, "expected exactly one fused mesh");
        let (mn, mx) = mesh_bounds(&compiled.meshes[0].mesh);
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
        // Shape A's native AABB is (-2, -2, -2, 2, 2, 2) — a 4×4×4 cube.
        // We establish the native extent by setting A's own bounds (A
        // is a container, not a primitive — shape: None).
        //
        // A's leaf occupies the +X half of A's cube: x ∈ [0, 2],
        // y, z ∈ [-2, 2].
        let leaf = leaf_box("leaf", Bounds(0, -2, -2, 2, 2, 2));
        let mut shape_a = empty_container("shape_a", vec![leaf]);
        shape_a.bounds = Some(Bounds(-2, -2, -2, 2, 2, 2));

        let mut registry = crate::registry::AssetRegistry::default();
        registry.test_insert_shape("shape_a", shape_a);

        // Shape B imports A at placement (-4, 0, -4, 4, 8, 4) — an
        // 8×8×8 cube. A's native 4×4×4 stretches by ×2 in each axis.
        let mut import_node = leaf_box("imported", Bounds(-4, 0, -4, 4, 8, 4));
        import_node.shape = None;
        import_node.import = Some("shape_a".into());
        let shape_b = empty_container("shape_b", vec![import_node]);

        let compiled = compile(&shape_b, &registry);

        // Within A's native cube the leaf occupied x ∈ [0, 2]
        // (50%..100%), y ∈ [-2, 2] (0%..100%), z ∈ [-2, 2] (0%..100%).
        // After A is stretched by ×2 into B's placement (-4..4, 0..8, -4..4):
        //   world x: 50%..100% of [-4, 4] → [0, 4]
        //   world y: 0%..100%  of [0, 8]  → [0, 8]
        //   world z: 0%..100%  of [-4, 4] → [-4, 4]
        let mesh = find_fused_mesh(&compiled);
        let (mn, mx) = mesh_bounds(mesh);
        assert_eq!(mn, [0.0, 0.0, -4.0], "stretched mesh min");
        assert_eq!(mx, [4.0, 8.0, 4.0], "stretched mesh max");
    }
}


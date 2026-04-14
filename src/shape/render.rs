//! Compile a `SpecNode` tree into a flat stream of `RenderEvent`s.
//!
//! This module is the one-way bridge from the integer specification to the
//! floating-point rendering pipeline. Everything downstream (`interpreter`,
//! `csg`, `sdf`) consumes `RenderEvent`s; none of them can reach back into
//! `SpecNode` fields. The only type from `super::spec` that appears here is
//! `SpecNode` itself (plus `Placement`, which is integer-only spec data),
//! and only in the signatures of the compile functions.

use bevy::prelude::*;
use crate::registry::AssetRegistry;
use crate::util::Color3;
use super::meshes::{RawMesh, create_raw_mesh};
use super::spec::{
    self, apply_placement_to_bounds, compose_placements, identity_placement,
    placements, Bounds, CombineMode, Combinator, Facing, Mirroring, Orientation,
    Placement, PrimitiveShape, Rotation, SignedAxis, SpecNode, Symmetry,
};

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
// RenderEvent — what the render pipeline consumes
// =====================================================================

/// A flat, ordered stream representing one rendered shape. Every float
/// field is pre-computed during `compile`.
///
/// Most variants carry only pre-baked float data. The one exception is
/// `AttachCsgGroup`, which deliberately forwards pre-transformed spec
/// children (along with their accumulated placement) so the interpreter
/// can store them on a `CsgGroup` component for later rebuilds when CSG
/// children are toggled. Placement is integer-only spec data, not render
/// data, so this is still a clean separation at the float boundary.
#[derive(Clone)]
pub enum RenderEvent {
    /// Entering a non-combinator node. Children follow until the matching ExitNode.
    EnterNode {
        name: Option<String>,
        local_tf: Transform,
    },
    /// Attach a CsgGroup component to the entity from the most recent
    /// EnterNode. Holds the pre-expanded spec children (each paired with
    /// its accumulated placement) so the group can re-run CSG when a
    /// child is toggled.
    AttachCsgGroup {
        children: Vec<(SpecNode, Placement)>,
        colors: ColorMap,
        scale: (i32, i32, i32),
    },
    /// The current node has a primitive shape to render.
    Geometry {
        name: Option<String>,
        has_children: bool,
        shape: PrimitiveShape,
        mesh_tf: Transform,
        is_mirrored: bool,
        color: Color3,
        emissive: bool,
    },
    /// A pre-computed mesh produced from CSG resolution inside an import.
    PrecomputedMesh {
        mesh: RawMesh,
        color: Color3,
    },
    /// Leaving the current node.
    ExitNode,
}

// =====================================================================
// Orientation → Mat3 conversion (the one place this happens)
// =====================================================================

/// Convert a discrete `Orientation` tuple plus an accumulated placement
/// from symmetry expansion into the final world→mesh matrix. The
/// placement is applied AFTER the orientation (pre-multiplied onto the
/// Mat3 derived from the orientation), so the authored orientation is
/// interpreted in the source frame and then the whole thing is
/// transformed by the symmetry placement.
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

/// Pre-multiply a `Mat3` by the signed-permutation matrix represented
/// by the given `Placement`. Each row of the result is the row of the
/// original matrix indexed by the placement's source axis, optionally
/// negated if the placement component is negative.
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

/// Compute the local transform for a node at its integer bounds, divided by
/// the accumulated import scale. Combinator nodes have no position of their
/// own (they're pass-through containers).
fn compute_local_transform(bounds: Option<&Bounds>, scale: (i32, i32, i32)) -> Transform {
    let position = match bounds {
        Some(b) => {
            let m = b.min();
            Vec3::new(
                m.0 as f32 / scale.0 as f32,
                m.1 as f32 / scale.1 as f32,
                m.2 as f32 / scale.2 as f32,
            )
        }
        None => Vec3::ZERO,
    };
    Transform::from_translation(position)
}

/// Compute the mesh transform. The unit mesh (centered at origin, -0.5 to 0.5)
/// is scaled by orient × size, then translated by size/2 so it fills the
/// bounds from (0,0,0) to size relative to the entity at bounds.min().
pub fn compute_mesh_transform(
    shape: PrimitiveShape,
    bounds: &Bounds,
    om: &Mat3,
    scale: (i32, i32, i32),
) -> Transform {
    let isize = bounds.size();
    let size = (
        isize.0 as f32 / scale.0 as f32,
        isize.1 as f32 / scale.1 as f32,
        isize.2 as f32 / scale.2 as f32,
    );

    let local_x_size = pick_size_for_direction(om.x_axis, size);
    let local_y_size = pick_size_for_direction(om.y_axis, size);
    let local_z_size = pick_size_for_direction(om.z_axis, size);

    let local_scale = match shape {
        PrimitiveShape::Torus => Vec3::new(local_x_size, local_y_size / 0.3, local_z_size),
        _ => Vec3::new(local_x_size, local_y_size, local_z_size),
    };

    let col_x = om.x_axis * local_scale.x;
    let col_y = om.y_axis * local_scale.y;
    let col_z = om.z_axis * local_scale.z;

    // Offset: the entity is at bounds.min(), the mesh center needs to be at
    // bounds.min() + size/2 = bounds.center(). So offset = size/2.
    let offset = Vec3::new(size.0 / 2.0, size.1 / 2.0, size.2 / 2.0);

    let mat = Mat3::from_cols(col_x, col_y, col_z);
    let affine = bevy::math::Affine3A::from_mat3_translation(mat, offset);
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

pub fn combine_transforms(parent: &Transform, child: &Transform) -> Transform {
    let parent_mat = parent.compute_matrix();
    let child_mat = child.compute_matrix();
    Transform::from_matrix(parent_mat * child_mat)
}

// =====================================================================
// Compile — the single bridge from spec to render
// =====================================================================

/// Compile a `SpecNode` tree into a flat sequence of `RenderEvent`s.
/// This is the ONLY function that consumes `SpecNode` fields for the
/// purpose of producing render data.
pub fn compile(
    spec: &SpecNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> Vec<RenderEvent> {
    compile_scaled(spec, colors, registry, (1, 1, 1))
}

/// Compile starting with an accumulated coordinate scale. Used from inside
/// an import where the imported subtree's bounds have been remapped.
pub fn compile_scaled(
    spec: &SpecNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) -> Vec<RenderEvent> {
    let mut events = Vec::new();
    walk_node(&mut events, spec, identity_placement(), colors, registry, scale);
    events
}

/// Compile a subtree with a pre-supplied accumulated placement. Used by
/// CSG rebuild, where each child in a CsgGroup was captured with its own
/// placement at spawn time and must be re-rendered with that placement
/// when a toggle happens.
pub fn compile_with_placement(
    spec: &SpecNode,
    placement: Placement,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) -> Vec<RenderEvent> {
    let mut events = Vec::new();
    walk_node(&mut events, spec, placement, colors, registry, scale);
    events
}

fn walk_node(
    events: &mut Vec<RenderEvent>,
    node: &SpecNode,
    inherited: Placement,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    let colors = if node.palette.is_empty() {
        colors.clone()
    } else {
        merge_colors(colors, &node.palette)
    };

    match node.combinator() {
        Combinator::Symmetry(sym) => {
            walk_symmetry(events, node, sym, inherited, &colors, registry, scale);
        }
        Combinator::Import(import_name) => {
            walk_import(events, node, import_name, inherited, &colors, registry, scale);
        }
        Combinator::None => {
            walk_single(events, node, inherited, &colors, registry, scale);
        }
    }
}

fn walk_single(
    events: &mut Vec<RenderEvent>,
    node: &SpecNode,
    inherited: Placement,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    // Transform this node's own bounds by the inherited placement before
    // computing the local transform and mesh transform.
    let transformed_bounds = node
        .bounds
        .as_ref()
        .map(|b| apply_placement_to_bounds(inherited, *b));

    let local_tf = compute_local_transform(transformed_bounds.as_ref(), scale);
    events.push(RenderEvent::EnterNode {
        name: node.name.clone(),
        local_tf,
    });

    if node.has_csg_children() {
        events.push(RenderEvent::AttachCsgGroup {
            children: spec::expand_symmetry_children(&node.children),
            colors: colors.clone(),
            scale,
        });
    }

    if let Some(ref b) = transformed_bounds {
        let size = b.size();
        if size.0 == 0 || size.1 == 0 || size.2 == 0 {
            error!(
                "'{}' has zero-size bounds ({},{},{}) — skipping",
                node.name.as_deref().unwrap_or("unnamed"),
                size.0,
                size.1,
                size.2
            );
            events.push(RenderEvent::ExitNode);
            return;
        }
    }

    if let Some(shape) = node.shape {
        let Some(bounds) = transformed_bounds else {
            warn!(
                "Shape '{}' has no bounds — skipping geometry",
                node.name.as_deref().unwrap_or("unnamed")
            );
            for child in &node.children {
                walk_node(events, child, inherited, colors, registry, scale);
            }
            events.push(RenderEvent::ExitNode);
            return;
        };
        let orient_mat = orientation_to_mat3(&node.orient, inherited);
        let mesh_tf = compute_mesh_transform(shape, &bounds, &orient_mat, scale);
        let is_mirrored = orient_mat.determinant() < 0.0;
        let color = node
            .color
            .as_ref()
            .map(|name| resolve_color(name, colors))
            .unwrap_or_else(|| {
                warn!(
                    "Shape '{}' has no color specified",
                    node.name.as_deref().unwrap_or("unnamed")
                );
                Color3(1, 1, 1)
            });
        events.push(RenderEvent::Geometry {
            name: node.name.clone(),
            has_children: !node.children.is_empty(),
            shape,
            mesh_tf,
            is_mirrored,
            color,
            emissive: node.emissive,
        });
    }

    for child in &node.children {
        walk_node(events, child, inherited, colors, registry, scale);
    }

    events.push(RenderEvent::ExitNode);
}

fn walk_symmetry(
    events: &mut Vec<RenderEvent>,
    node: &SpecNode,
    sym: Symmetry,
    inherited: Placement,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    let mut base = node.clone();
    base.symmetry = Symmetry::Single;

    for (local, suffix) in placements(sym) {
        let combined = compose_placements(inherited, *local);
        let mut copy = base.clone();
        if !suffix.is_empty() {
            if let Some(ref name) = copy.name {
                copy.name = Some(format!("{name}{suffix}"));
            }
        }
        walk_node(events, &copy, combined, colors, registry, scale);
    }
}

fn walk_import(
    events: &mut Vec<RenderEvent>,
    node: &SpecNode,
    import_name: &str,
    inherited: Placement,
    colors: &ColorMap,
    registry: &AssetRegistry,
    parent_scale: (i32, i32, i32),
) {
    let imported = match registry.get_shape(import_name) {
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
    let placement_bounds = node.bounds.unwrap_or(native_aabb);

    let remap_scale = Bounds::remap_scale(&native_aabb);
    let new_scale = (
        parent_scale.0 * remap_scale.0,
        parent_scale.1 * remap_scale.1,
        parent_scale.2 * remap_scale.2,
    );

    let mut remapped = imported;
    remapped.remap_bounds(&native_aabb, &placement_bounds);

    let import_colors = apply_color_remapping(node, &remapped.palette, colors);

    if remapped.has_csg_children() {
        walk_import_with_csg(events, &remapped, inherited, &import_colors, registry, new_scale);
    } else {
        walk_node(events, &remapped, inherited, &import_colors, registry, new_scale);
    }
}

/// Resolve CSG within an imported shape and emit the result as a single
/// pre-computed mesh. The import's internal subtract/clip operations are
/// fully applied here.
fn walk_import_with_csg(
    events: &mut Vec<RenderEvent>,
    remapped: &SpecNode,
    _inherited: Placement,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    let merged_colors = if remapped.palette.is_empty() {
        colors.clone()
    } else {
        merge_colors(colors, &remapped.palette)
    };

    let aabb = Bounds::enclosing(&remapped.children).unwrap_or(Bounds(-1, -1, -1, 1, 1, 1));

    // Pair each child with the identity placement: imports don't carry
    // their own symmetry expansion at this level. (Symmetry expansion
    // inside the imported subtree would happen recursively in
    // expand_symmetry_children when entering the subtree proper.)
    let paired: Vec<(SpecNode, Placement)> = remapped
        .children
        .iter()
        .map(|c| (c.clone(), identity_placement()))
        .collect();
    let (mesh, _stats) =
        super::csg::perform_csg_from_children(&paired, &merged_colors, registry, &aabb, scale);

    let color = remapped
        .children
        .iter()
        .find(|c| c.combine == CombineMode::Union)
        .and_then(|c| c.color.as_ref())
        .map(|name| resolve_color(name, &merged_colors))
        .unwrap_or(Color3(1, 1, 1));

    let local_tf = compute_local_transform(None, scale);
    events.push(RenderEvent::EnterNode {
        name: remapped.name.clone(),
        local_tf,
    });

    if !mesh.positions.is_empty() {
        events.push(RenderEvent::PrecomputedMesh { mesh, color });
    }

    events.push(RenderEvent::ExitNode);
}

// =====================================================================
// Collect raw mesh from events — used by CSG
// =====================================================================

/// Process render events into a single `RawMesh` with all transforms baked in.
pub fn collect_raw_mesh(events: &[RenderEvent]) -> RawMesh {
    let mut result = RawMesh {
        positions: vec![],
        normals: vec![],
        uvs: vec![],
        indices: vec![],
    };
    let mut tf_stack: Vec<Transform> = vec![Transform::IDENTITY];

    for event in events {
        match event {
            RenderEvent::EnterNode { local_tf, .. } => {
                let parent_world = *tf_stack.last().unwrap();
                let world = combine_transforms(&parent_world, local_tf);
                tf_stack.push(world);
            }
            RenderEvent::Geometry { shape, mesh_tf, .. } => {
                let world = *tf_stack.last().unwrap();
                let world_mesh_tf = combine_transforms(&world, mesh_tf);
                let mut raw = create_raw_mesh(*shape);
                raw.apply_transform(&world_mesh_tf);
                result.merge(&raw);
            }
            RenderEvent::PrecomputedMesh { mesh, .. } => {
                let world = *tf_stack.last().unwrap();
                let mut raw = mesh.clone();
                raw.apply_transform(&world);
                result.merge(&raw);
            }
            RenderEvent::AttachCsgGroup { .. } => {
                // No geometric effect — group metadata only.
            }
            RenderEvent::ExitNode => {
                tf_stack.pop();
            }
        }
    }

    result
}

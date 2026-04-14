//! Compile a `SpecNode` tree into a flat stream of `RenderEvent`s.
//!
//! This module is the one-way bridge from the integer specification to the
//! floating-point rendering pipeline. Everything downstream (`interpreter`,
//! `csg`, `sdf`) consumes `RenderEvent`s; none of them can reach back into
//! `SpecNode` fields. The only type from `super::spec` that appears here is
//! `SpecNode` itself, and only in the signatures of the compile functions.

use bevy::prelude::*;
use crate::registry::AssetRegistry;
use crate::util::Color3;
use super::meshes::{RawMesh, create_raw_mesh};
use super::spec::{
    self, Axis, Bounds, CombineMode, Combinator, Facing, Mirroring,
    Orientation, PrimitiveShape, Rotation, SpecNode,
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

/// A flat, ordered stream representing one rendered shape. Every field is
/// pre-computed during `compile`.
///
/// Most variants carry only pre-baked float data. The one exception is
/// `AttachCsgGroup`, which deliberately forwards a slice of the spec tree
/// to the render entity so the interpreter can store it on a `CsgGroup`
/// component for later rebuilds when CSG children are toggled. This is
/// the single, documented leak of `SpecNode` into the render layer — all
/// other variants are spec-free.
#[derive(Clone)]
pub enum RenderEvent {
    /// Entering a non-combinator node. Children follow until the matching ExitNode.
    EnterNode {
        name: Option<String>,
        local_tf: Transform,
    },
    /// Attach a CsgGroup component to the entity from the most recent
    /// EnterNode. Holds the pre-expanded spec children so the group can
    /// re-run CSG when a child is toggled.
    AttachCsgGroup {
        children: Vec<SpecNode>,
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

/// Convert a discrete `Orientation` tuple and a list of axis reflections
/// accumulated through mirror expansion into the world→mesh matrix.
pub fn orientation_to_mat3(orient: &Orientation, reflected_axes: &[Axis]) -> Mat3 {
    let mut mat = base_orientation_matrix(orient);
    for &axis in reflected_axes {
        reflect_mat3(&mut mat, axis);
    }
    mat
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

fn reflect_mat3(orient: &mut Mat3, axis: Axis) {
    match axis {
        Axis::X => {
            orient.x_axis.x = -orient.x_axis.x;
            orient.y_axis.x = -orient.y_axis.x;
            orient.z_axis.x = -orient.z_axis.x;
        }
        Axis::Y => {
            orient.x_axis.y = -orient.x_axis.y;
            orient.y_axis.y = -orient.y_axis.y;
            orient.z_axis.y = -orient.z_axis.y;
        }
        Axis::Z => {
            orient.x_axis.z = -orient.x_axis.z;
            orient.y_axis.z = -orient.y_axis.z;
            orient.z_axis.z = -orient.z_axis.z;
        }
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
    walk_node(&mut events, spec, colors, registry, scale);
    events
}

fn walk_node(
    events: &mut Vec<RenderEvent>,
    node: &SpecNode,
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
        Combinator::Mirror(axes) => {
            walk_mirror(events, node, axes, &colors, registry, scale);
        }
        Combinator::Import(import_name) => {
            walk_import(events, node, import_name, &colors, registry, scale);
        }
        Combinator::None => {
            let bounds_for_tf = if node.is_combinator() {
                None
            } else {
                node.bounds.as_ref()
            };
            let local_tf = compute_local_transform(bounds_for_tf, scale);
            events.push(RenderEvent::EnterNode {
                name: node.name.clone(),
                local_tf,
            });

            if node.has_csg_children() {
                events.push(RenderEvent::AttachCsgGroup {
                    children: spec::expand_mirror_children(&node.children),
                    colors: colors.clone(),
                    scale,
                });
            }

            if let Some(bounds) = &node.bounds {
                let size = bounds.size();
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
                let Some(bounds) = node.bounds else {
                    warn!(
                        "Shape '{}' has no bounds — skipping geometry",
                        node.name.as_deref().unwrap_or("unnamed")
                    );
                    for child in &node.children {
                        walk_node(events, child, &colors, registry, scale);
                    }
                    events.push(RenderEvent::ExitNode);
                    return;
                };
                let orient_mat = orientation_to_mat3(&node.orient, &node.reflected_axes);
                let mesh_tf = compute_mesh_transform(shape, &bounds, &orient_mat, scale);
                let is_mirrored = orient_mat.determinant() < 0.0;
                let color = node
                    .color
                    .as_ref()
                    .map(|name| resolve_color(name, &colors))
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
                walk_node(events, child, &colors, registry, scale);
            }

            events.push(RenderEvent::ExitNode);
        }
    }
}

fn walk_mirror(
    events: &mut Vec<RenderEvent>,
    node: &SpecNode,
    axes: &[Axis],
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    let mut base = node.clone();
    base.mirror = Vec::new();

    let combinations = spec::mirror_combinations(axes);
    for (flipped_axes, suffix) in &combinations {
        let mut copy = base.clone();
        for &axis in flipped_axes {
            spec::flip_bounds(&mut copy, axis);
        }
        for &axis in flipped_axes {
            spec::push_reflection(&mut copy, axis);
        }
        if !suffix.is_empty() {
            if let Some(ref name) = copy.name {
                copy.name = Some(format!("{name}_{suffix}"));
            }
        }
        walk_node(events, &copy, colors, registry, scale);
    }
}

fn walk_import(
    events: &mut Vec<RenderEvent>,
    node: &SpecNode,
    import_name: &str,
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
    let placement = node.bounds.unwrap_or(native_aabb);

    let remap_scale = Bounds::remap_scale(&native_aabb);
    let new_scale = (
        parent_scale.0 * remap_scale.0,
        parent_scale.1 * remap_scale.1,
        parent_scale.2 * remap_scale.2,
    );

    let mut remapped = imported;
    remapped.remap_bounds(&native_aabb, &placement);

    let import_colors = apply_color_remapping(node, &remapped.palette, colors);

    if remapped.has_csg_children() {
        walk_import_with_csg(events, &remapped, &import_colors, registry, new_scale);
    } else {
        walk_node(events, &remapped, &import_colors, registry, new_scale);
    }
}

/// Resolve CSG within an imported shape and emit the result as a single
/// pre-computed mesh. The import's internal subtract/clip operations are
/// fully applied here.
fn walk_import_with_csg(
    events: &mut Vec<RenderEvent>,
    remapped: &SpecNode,
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

    let (mesh, _stats) =
        super::csg::perform_csg_from_children(&remapped.children, &merged_colors, registry, &aabb, scale);

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

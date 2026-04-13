use bevy::prelude::*;
use super::definition::{Axis, Bounds, Combinator, PrimitiveShape, RepeatSpec, ShapeNode, reflect_orient};
use crate::registry::AssetRegistry;
use crate::util::Color3;
use super::meshes::{RawMesh, create_raw_mesh};

// =====================================================================
// Color context
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
    colors.iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| {
            warn!("Color '{}' not found in color map, using default gray", name);
            Color3(1, 1, 1)
        })
}

pub fn apply_color_remapping(
    import_node: &ShapeNode,
    imported_colors: &ColorMap,
    parent_colors: &ColorMap,
) -> ColorMap {
    if !import_node.color_map.is_empty() && !import_node.colors.is_empty() {
        warn!("Node '{}' specifies both color_map and colors — using color_map",
            import_node.name.as_deref().unwrap_or("unnamed"));
    }

    if !import_node.color_map.is_empty() {
        imported_colors.iter().map(|(child_name, child_val)| {
            if let Some(parent_name) = import_node.color_map.get(child_name) {
                let resolved = resolve_color(parent_name, parent_colors);
                (child_name.clone(), resolved)
            } else {
                (child_name.clone(), *child_val)
            }
        }).collect()
    } else if !import_node.colors.is_empty() {
        imported_colors.iter().enumerate().map(|(i, (child_name, child_val))| {
            if let Some(parent_name) = import_node.colors.get(i) {
                let resolved = resolve_color(parent_name, parent_colors);
                (child_name.clone(), resolved)
            } else {
                (child_name.clone(), *child_val)
            }
        }).collect()
    } else {
        merge_colors(parent_colors, imported_colors)
    }
}

// =====================================================================
// Transform computation
// =====================================================================


/// Compute the local transform for a node. Divides integer coordinates by
/// the accumulated scale to get correct world-space floats.
pub fn compute_local_transform(node: &ShapeNode, scale: (i32, i32, i32)) -> Transform {
    let position = if node.is_combinator() {
        Vec3::ZERO
    } else {
        match &node.bounds {
            Some(b) => {
                let m = b.min();
                Vec3::new(
                    m.0 as f32 / scale.0 as f32,
                    m.1 as f32 / scale.1 as f32,
                    m.2 as f32 / scale.2 as f32,
                )
            }
            None => Vec3::ZERO,
        }
    };

    let mut tf = Transform::from_translation(position);
    if let Some((degrees, axis)) = node.rotate {
        let rad = degrees.to_radians();
        tf.rotation = match axis {
            Axis::X => Quat::from_rotation_x(rad),
            Axis::Y => Quat::from_rotation_y(rad),
            Axis::Z => Quat::from_rotation_z(rad),
        };
    }
    tf
}

/// Compute the mesh transform. The unit mesh (centered at origin, -0.5 to 0.5)
/// is scaled by orient × size, then translated by size/2 so it fills
/// the bounds from (0,0,0) to (size) relative to the entity at bounds.min().
/// Divides by scale to convert from integer space to world space.
pub fn compute_mesh_transform(shape: PrimitiveShape, bounds: &Bounds, om: &Mat3, scale: (i32, i32, i32)) -> Transform {
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
    if dir.x.abs() > 0.5 { size.0 }
    else if dir.y.abs() > 0.5 { size.1 }
    else { size.2 }
}

/// Bounds center as Vec3 (float — only for camera/render positioning).
pub fn bounds_center(bounds: &Option<Bounds>) -> Vec3 {
    match bounds {
        Some(b) => {
            let c = b.center_f32();
            Vec3::new(c.0, c.1, c.2)
        }
        None => Vec3::ZERO,
    }
}

pub fn combine_transforms(parent: &Transform, child: &Transform) -> Transform {
    let parent_mat = parent.compute_matrix();
    let child_mat = child.compute_matrix();
    Transform::from_matrix(parent_mat * child_mat)
}

// =====================================================================
// Combinator helpers
// =====================================================================

fn reify_bounds(node: &mut ShapeNode) {
    if node.bounds.is_none() && node.shape.is_some() {
        warn!("Shape '{}' has no bounds — every shape must specify bounds",
            node.name.as_deref().unwrap_or("unnamed"));
    }
}

fn offset_bounds(bounds: &mut Option<Bounds>, axis: Axis, offset: f32) {
    let o = offset.round() as i32;
    if let Some(ref mut b) = bounds {
        match axis {
            Axis::X => { b.0 += o; b.3 += o; }
            Axis::Y => { b.1 += o; b.4 += o; }
            Axis::Z => { b.2 += o; b.5 += o; }
        }
    }
}

fn flip_node_bounds(node: &mut ShapeNode, axis: Axis) {
    reify_bounds(node);
    if let Some(ref mut b) = node.bounds {
        match axis {
            Axis::X => { let tmp = -b.0; b.0 = -b.3; b.3 = tmp; }
            Axis::Y => { let tmp = -b.1; b.1 = -b.4; b.4 = tmp; }
            Axis::Z => { let tmp = -b.2; b.2 = -b.5; b.5 = tmp; }
        }
    }
    for child in &mut node.children {
        flip_node_bounds(child, axis);
    }
}

fn reflect_orientation(node: &mut ShapeNode, axis: Axis) {
    if node.shape.is_some() {
        reflect_orient(&mut node.orient, axis);
    }
    for child in &mut node.children {
        reflect_orientation(child, axis);
    }
}

fn mirror_combinations(axes: &[Axis]) -> Vec<(Vec<Axis>, String)> {
    let n = axes.len();
    let count = 1 << n;
    let mut result = Vec::with_capacity(count);
    for bits in 0..count {
        let mut flipped = Vec::new();
        let mut suffix = String::new();
        for (i, &axis) in axes.iter().enumerate() {
            if bits & (1 << i) != 0 {
                flipped.push(axis);
                let letter = match axis { Axis::X => "x", Axis::Y => "y", Axis::Z => "z" };
                suffix.push_str(letter);
            }
        }
        let suffix = if suffix.is_empty() { String::new() } else { format!("m{suffix}") };
        result.push((flipped, suffix));
    }
    result
}

/// Expand mirror and repeat combinators on a list of children, producing
/// the same flat list that the walk generates as entities. Used so that
/// CsgGroup stores post-expansion children matching the entity tree.
pub fn expand_combinators(children: &[ShapeNode]) -> Vec<ShapeNode> {
    let mut result = Vec::new();
    for child in children {
        match child.combinator() {
            Combinator::Mirror(axes) => {
                let mut base = child.clone();
                base.mirror = Vec::new();
                for (flipped_axes, suffix) in &mirror_combinations(axes) {
                    let mut copy = base.clone();
                    for &axis in flipped_axes {
                        flip_node_bounds(&mut copy, axis);
                    }
                    for &axis in flipped_axes {
                        reflect_orientation(&mut copy, axis);
                    }
                    if !suffix.is_empty() {
                        if let Some(ref name) = copy.name {
                            copy.name = Some(format!("{name}_{suffix}"));
                        }
                    }
                    result.push(copy);
                }
            }
            Combinator::Repeat(repeat) => {
                let start = if repeat.center {
                    -(repeat.count as f32 - 1.0) * repeat.spacing * 0.5
                } else {
                    0.0
                };
                for i in 0..repeat.count {
                    let mut instance = child.clone();
                    instance.repeat = None;
                    reify_bounds(&mut instance);
                    offset_bounds(&mut instance.bounds, repeat.along, start + i as f32 * repeat.spacing);
                    if let Some(ref name) = instance.name {
                        instance.name = Some(format!("{name}_{i}"));
                    }
                    result.push(instance);
                }
            }
            _ => {
                result.push(child.clone());
            }
        }
    }
    result
}

// =====================================================================
// Shape events — intermediate representation from tree walk
// =====================================================================

#[derive(Clone)]
pub enum ShapeEvent {
    /// Entering a non-combinator node. Children follow until the matching ExitNode.
    EnterNode {
        node: ShapeNode,
        local_tf: Transform,
        colors: ColorMap,
        /// Accumulated coordinate scale from import nesting.
        scale: (i32, i32, i32),
    },
    /// The current node has a primitive shape to render.
    Geometry {
        node: ShapeNode,
        mesh_tf: Transform,
        colors: ColorMap,
    },
    /// A pre-computed mesh from CSG resolution within an import.
    /// The import's internal CSG is fully resolved; the result is a single mesh.
    PrecomputedMesh {
        mesh: RawMesh,
        color: Color3,
    },
    /// Leaving the current node.
    ExitNode,
}

// =====================================================================
// Tree walk — the single source of traversal logic
// =====================================================================

/// Walk a ShapeNode tree and produce a flat list of events.
/// Handles all combinator expansion (mirror, repeat, import) and transform
/// computation. This is the ONE place where traversal logic lives.
pub fn walk_shape_tree(
    node: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
) -> Vec<ShapeEvent> {
    walk_shape_tree_scaled(node, colors, registry, (1, 1, 1))
}

/// Walk a ShapeNode tree starting with an accumulated coordinate scale.
/// Used when the children's bounds have already been remapped (e.g. inside an import).
pub fn walk_shape_tree_scaled(
    node: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) -> Vec<ShapeEvent> {
    let mut events = Vec::new();
    walk_node(&mut events, node, colors, registry, scale);
    events
}

fn walk_node(
    events: &mut Vec<ShapeEvent>,
    node: &ShapeNode,
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
        Combinator::Repeat(repeat) => {
            walk_repeat(events, node, repeat, &colors, registry, scale);
        }
        Combinator::Import(import_name) => {
            walk_import(events, node, import_name, &colors, registry, scale);
        }
        Combinator::None => {
            let local_tf = compute_local_transform(node, scale);
            events.push(ShapeEvent::EnterNode {
                node: node.clone(),
                local_tf,
                colors: colors.clone(),
                scale,
            });

            if let Some(bounds) = &node.bounds {
                let size = bounds.size();
                if size.0 == 0 || size.1 == 0 || size.2 == 0 {
                    error!("'{}' has zero-size bounds ({},{},{}) — skipping",
                        node.name.as_deref().unwrap_or("unnamed"), size.0, size.1, size.2);
                    events.push(ShapeEvent::ExitNode);
                    return;
                }
            }

            if let Some(shape) = node.shape {
                let Some(bounds) = node.bounds else {
                    warn!("Shape '{}' has no bounds — skipping geometry",
                        node.name.as_deref().unwrap_or("unnamed"));
                    for child in &node.children {
                        walk_node(events, child, &colors, registry, scale);
                    }
                    events.push(ShapeEvent::ExitNode);
                    return;
                };
                let mesh_tf = compute_mesh_transform(shape, &bounds, &node.orient, scale);
                events.push(ShapeEvent::Geometry {
                    node: node.clone(),
                    mesh_tf,
                    colors: colors.clone(),
                });
            }

            for child in &node.children {
                walk_node(events, child, &colors, registry, scale);
            }

            events.push(ShapeEvent::ExitNode);
        }
    }
}

fn walk_mirror(
    events: &mut Vec<ShapeEvent>,
    node: &ShapeNode,
    axes: &[Axis],
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    let mut base = node.clone();
    base.mirror = Vec::new();

    let combinations = mirror_combinations(axes);
    for (flipped_axes, suffix) in &combinations {
        let mut copy = base.clone();
        for &axis in flipped_axes {
            flip_node_bounds(&mut copy, axis);
        }
        for &axis in flipped_axes {
            reflect_orientation(&mut copy, axis);
        }
        if !suffix.is_empty() {
            if let Some(ref name) = copy.name {
                copy.name = Some(format!("{name}_{suffix}"));
            }
        }
        walk_node(events, &copy, colors, registry, scale);
    }
}

fn walk_repeat(
    events: &mut Vec<ShapeEvent>,
    node: &ShapeNode,
    repeat: &RepeatSpec,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    let start = if repeat.center {
        -(repeat.count as f32 - 1.0) * repeat.spacing * 0.5
    } else {
        0.0
    };

    for i in 0..repeat.count {
        let mut instance = node.clone();
        instance.repeat = None;
        reify_bounds(&mut instance);
        offset_bounds(&mut instance.bounds, repeat.along, start + i as f32 * repeat.spacing);
        if let Some(ref name) = instance.name {
            instance.name = Some(format!("{name}_{i}"));
        }
        walk_node(events, &instance, colors, registry, scale);
    }
}

fn walk_import(
    events: &mut Vec<ShapeEvent>,
    node: &ShapeNode,
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
        walk_import_with_csg(events, node, &remapped, &import_colors, registry, new_scale);
    } else {
        walk_node(events, &remapped, &import_colors, registry, new_scale);
    }
}

/// Resolve CSG within an imported shape and emit the result as a pre-computed mesh.
/// The import's internal subtract/clip operations are fully applied here;
/// the parent sees only a computed shape, not the raw CSG structure.
fn walk_import_with_csg(
    events: &mut Vec<ShapeEvent>,
    _import_node: &ShapeNode,
    remapped: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
    scale: (i32, i32, i32),
) {
    let merged_colors = if remapped.palette.is_empty() {
        colors.clone()
    } else {
        merge_colors(colors, &remapped.palette)
    };

    let aabb = Bounds::enclosing(&remapped.children)
        .unwrap_or(Bounds(-1, -1, -1, 1, 1, 1));

    let (mesh, _stats) = super::csg::perform_csg_from_children(
        &remapped.children, &merged_colors, registry, &aabb, scale,
    );

    let color = remapped.children.iter()
        .find(|c| c.combine == super::definition::CombineMode::Union)
        .and_then(|c| c.color.as_ref())
        .map(|name| resolve_color(name, &merged_colors))
        .unwrap_or(Color3(1, 1, 1));

    let mut container = remapped.clone();
    container.children.clear();
    container.shape = None;

    let local_tf = compute_local_transform(&container, scale);
    events.push(ShapeEvent::EnterNode {
        node: container,
        local_tf,
        colors: merged_colors,
        scale,
    });

    if !mesh.positions.is_empty() {
        events.push(ShapeEvent::PrecomputedMesh { mesh, color });
    }

    // Also walk any non-CSG children that might exist alongside the CSG group.
    // (Currently all children of a CSG parent participate, but this is future-safe.)

    events.push(ShapeEvent::ExitNode);
}

// =====================================================================
// Collect raw mesh from events — used by CSG
// =====================================================================

/// Process shape events into a single RawMesh with transforms baked in.
pub fn collect_mesh_from_events(events: &[ShapeEvent]) -> RawMesh {
    let mut result = RawMesh { positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };
    let mut tf_stack: Vec<Transform> = vec![Transform::IDENTITY];

    for event in events {
        match event {
            ShapeEvent::EnterNode { local_tf, .. } => {
                let parent_world = *tf_stack.last().unwrap();
                let world = combine_transforms(&parent_world, local_tf);
                tf_stack.push(world);
            }
            ShapeEvent::Geometry { node, mesh_tf, .. } => {
                let world = *tf_stack.last().unwrap();
                let world_mesh_tf = combine_transforms(&world, mesh_tf);
                let mut raw = create_raw_mesh(node.shape.unwrap());
                raw.apply_transform(&world_mesh_tf);
                result.merge(&raw);
            }
            ShapeEvent::PrecomputedMesh { mesh, .. } => {
                let world = *tf_stack.last().unwrap();
                let mut raw = mesh.clone();
                raw.apply_transform(&world);
                result.merge(&raw);
            }
            ShapeEvent::ExitNode => {
                tf_stack.pop();
            }
        }
    }

    result
}

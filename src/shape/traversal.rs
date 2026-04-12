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


/// Compute the local transform for a node. Uses bounds min (always integer)
/// as the entity position. Combinators have no position.
pub fn compute_local_transform(node: &ShapeNode) -> Transform {
    let position = if node.is_combinator() {
        Vec3::ZERO
    } else {
        bounds_min_vec(&node.bounds)
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
pub fn compute_mesh_transform(shape: PrimitiveShape, bounds: &Bounds, om: &Mat3) -> Transform {
    let isize = bounds.size();
    let size = (isize.0 as f32, isize.1 as f32, isize.2 as f32);

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

/// Bounds min as Vec3 (integer values cast to f32).
pub fn bounds_min_vec(bounds: &Option<Bounds>) -> Vec3 {
    match bounds {
        Some(b) => {
            let m = b.min();
            Vec3::new(m.0 as f32, m.1 as f32, m.2 as f32)
        }
        None => Vec3::ZERO,
    }
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
    },
    /// The current node has a primitive shape to render.
    Geometry {
        node: ShapeNode,
        mesh_tf: Transform,
        colors: ColorMap,
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
    let mut events = Vec::new();
    walk_node(&mut events, node, colors, registry);
    events
}

fn walk_node(
    events: &mut Vec<ShapeEvent>,
    node: &ShapeNode,
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
    let colors = if node.palette.is_empty() {
        colors.clone()
    } else {
        merge_colors(colors, &node.palette)
    };

    match node.combinator() {
        Combinator::Mirror(axes) => {
            walk_mirror(events, node, axes, &colors, registry);
        }
        Combinator::Repeat(repeat) => {
            walk_repeat(events, node, repeat, &colors, registry);
        }
        Combinator::Import(import_name) => {
            walk_import(events, node, import_name, &colors, registry);
        }
        Combinator::None => {
            let local_tf = compute_local_transform(node);
            events.push(ShapeEvent::EnterNode {
                node: node.clone(),
                local_tf,
                colors: colors.clone(),
            });

            if let Some(shape) = node.shape {
                let Some(bounds) = node.bounds else {
                    warn!("Shape '{}' has no bounds — skipping geometry",
                        node.name.as_deref().unwrap_or("unnamed"));
                    for child in &node.children {
                        walk_node(events, child, &colors, registry);
                    }
                    events.push(ShapeEvent::ExitNode);
                    return;
                };
                let mesh_tf = compute_mesh_transform(shape, &bounds, &node.orient);
                events.push(ShapeEvent::Geometry {
                    node: node.clone(),
                    mesh_tf,
                    colors: colors.clone(),
                });
            }

            for child in &node.children {
                walk_node(events, child, &colors, registry);
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
        walk_node(events, &copy, colors, registry);
    }
}

fn walk_repeat(
    events: &mut Vec<ShapeEvent>,
    node: &ShapeNode,
    repeat: &RepeatSpec,
    colors: &ColorMap,
    registry: &AssetRegistry,
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
        walk_node(events, &instance, colors, registry);
    }
}

fn walk_import(
    events: &mut Vec<ShapeEvent>,
    node: &ShapeNode,
    import_name: &str,
    colors: &ColorMap,
    registry: &AssetRegistry,
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

    let mut remapped = imported;
    remapped.remap_bounds(&native_aabb, &placement);

    let import_colors = apply_color_remapping(node, &remapped.palette, colors);

    walk_node(events, &remapped, &import_colors, registry);
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
            ShapeEvent::ExitNode => {
                tf_stack.pop();
            }
        }
    }

    result
}

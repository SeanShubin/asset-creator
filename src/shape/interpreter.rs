use bevy::prelude::*;

use crate::registry::AssetRegistry;
use super::animation::ShapeAnimator;
use super::definition::{Axis, Bounds, PrimitiveShape, RepeatSpec, ShapeNode, SignedAxis, orient_matrix};

// =====================================================================
// Components
// =====================================================================

#[derive(Component, Clone, Debug)]
pub struct ShapePart {
    pub name: Option<String>,
}

#[derive(Component, Clone, Debug)]
pub struct BaseTransform(pub Transform);

#[derive(Component)]
pub struct ShapeRoot;

// =====================================================================
// Public API
// =====================================================================

pub fn load_shape(ron_str: &str) -> Result<ShapeNode, String> {
    let options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(ron_str).map_err(|e| format!("Failed to parse shape: {e}"))
}

pub fn spawn_shape(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: &ShapeNode,
    registry: &AssetRegistry,
) -> Entity {
    let position = bounds_center(&shape.bounds);
    let root_tf = Transform::from_translation(position);
    let root = commands.spawn((
        ShapeRoot,
        ShapePart { name: shape.name.clone() },
        BaseTransform(root_tf),
        ShapeAnimator::new(shape.animations.clone()),
        root_tf,
        Visibility::default(),
    )).id();

    let default_color = (0.5, 0.5, 0.5);
    process_node(commands, meshes, materials, root, shape, default_color, registry);
    root
}

pub fn despawn_shape(commands: &mut Commands, roots: &[Entity]) {
    for &e in roots {
        commands.entity(e).despawn_recursive();
    }
}

// =====================================================================
// Node processing
// =====================================================================

fn process_node(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    inherited_color: (f32, f32, f32),
    registry: &AssetRegistry,
) {
    let color = node.color.unwrap_or(inherited_color);

    if !node.mirror.is_empty() {
        process_mirror(commands, meshes, materials, parent, node, &node.mirror, color, registry);
        return;
    }

    if let Some(repeat) = &node.repeat {
        process_repeat(commands, meshes, materials, parent, node, repeat, color, registry);
        return;
    }

    if let Some(import_name) = &node.import {
        process_import(commands, meshes, materials, parent, node, import_name, color, registry);
        return;
    }

    attach_geometry(commands, meshes, materials, parent, node, color);

    for child in &node.children {
        spawn_child(commands, meshes, materials, parent, child, color, registry);
    }
}

// =====================================================================
// Import — resolve from registry cache
// =====================================================================

fn process_import(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    import_name: &str,
    color: (f32, f32, f32),
    registry: &AssetRegistry,
) {
    let imported = match registry.get_shape(import_name) {
        Some(shape) => shape.clone(),
        None => {
            error!("Import '{}' not found in registry", import_name);
            return;
        }
    };

    let native_bounds = imported.bounds.unwrap_or(Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5));
    let placement_bounds = node.bounds.unwrap_or(native_bounds);

    let native_size = native_bounds.size();
    let placement_center = placement_bounds.center();
    let placement_size = placement_bounds.size();

    let scale = Vec3::new(
        if native_size.0 > 0.001 { placement_size.0 / native_size.0 } else { 1.0 },
        if native_size.1 > 0.001 { placement_size.1 / native_size.1 } else { 1.0 },
        if native_size.2 > 0.001 { placement_size.2 / native_size.2 } else { 1.0 },
    );

    // Position is already handled by spawn_child via bounds_center.
    // Import entity only needs scale to map native size to placement size.
    let import_tf = Transform::from_scale(scale);

    let import_entity = commands.spawn((
        ShapePart { name: node.name.clone().or(Some(import_name.to_string())) },
        BaseTransform(import_tf),
        import_tf,
        Visibility::default(),
    )).id();
    commands.entity(parent).add_child(import_entity);

    let import_color = node.color.unwrap_or(color);
    attach_geometry(commands, meshes, materials, import_entity, &imported, import_color);
    for child in &imported.children {
        spawn_child(commands, meshes, materials, import_entity, child, import_color, registry);
    }
}

// =====================================================================
// Combinator handlers
// =====================================================================

fn process_repeat(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    repeat: &RepeatSpec,
    color: (f32, f32, f32),
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
        spawn_child(commands, meshes, materials, parent, &instance, color, registry);
    }
}

fn process_mirror(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    axes: &[Axis],
    color: (f32, f32, f32),
    registry: &AssetRegistry,
) {
    let mut base = node.clone();
    base.mirror = Vec::new();

    // Generate all 2^N combinations of axis flips
    let combinations = mirror_combinations(axes);
    for (flipped_axes, suffix) in &combinations {
        let mut copy = base.clone();
        for &axis in flipped_axes {
            flip_node_bounds(&mut copy, axis);
        }
        if !suffix.is_empty() {
            if let Some(ref name) = copy.name {
                copy.name = Some(format!("{name}_{suffix}"));
            }
        }
        spawn_child(commands, meshes, materials, parent, &copy, color, registry);
    }
}

/// Generate all 2^N combinations of flipping the given axes.
/// Returns vec of (flipped_axes, name_suffix).
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
                let letter = match axis {
                    Axis::X => "x",
                    Axis::Y => "y",
                    Axis::Z => "z",
                };
                suffix.push_str(letter);
            }
        }
        let suffix = if suffix.is_empty() { String::new() } else { format!("m{suffix}") };
        result.push((flipped, suffix));
    }

    result
}

// =====================================================================
// Child spawning
// =====================================================================

fn spawn_child(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    inherited_color: (f32, f32, f32),
    registry: &AssetRegistry,
) {
    let child_tf = build_child_transform(node);
    let child = commands.spawn((
        ShapePart { name: node.name.clone() },
        BaseTransform(child_tf),
        child_tf,
        Visibility::default(),
    )).id();
    commands.entity(parent).add_child(child);

    let color = node.color.unwrap_or(inherited_color);
    process_node(commands, meshes, materials, child, node, color, registry);
}

fn build_child_transform(node: &ShapeNode) -> Transform {
    // Nodes with combinators (mirror, repeat, import) are pass-through containers.
    // Their children carry the actual positioning, so the combinator node itself
    // should not add a position offset — otherwise the position is applied twice.
    let is_combinator = !node.mirror.is_empty() || node.repeat.is_some();
    let position = if is_combinator {
        Vec3::ZERO
    } else {
        bounds_center(&node.bounds)
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

// =====================================================================
// Geometry attachment
// =====================================================================

fn attach_geometry(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    color: (f32, f32, f32),
) {
    let Some(shape) = &node.shape else { return };
    let bounds = node.bounds.unwrap_or(Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5));
    let om = orient_matrix(&node.orient);
    let is_mirrored = om.determinant() < 0.0;
    let (mesh, material) = make_mesh(meshes, materials, *shape, color, node.emissive, is_mirrored);
    let mesh_tf = mesh_transform(*shape, &bounds, &node.orient);

    if node.children.is_empty() {
        commands.entity(parent).with_child((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            mesh_tf,
        ));
    } else {
        let shape_name = node.name.as_ref()
            .map(|n| format!("{n}_shape"))
            .unwrap_or_else(|| "shape".to_string());
        let shape_entity = commands.spawn((
            ShapePart { name: Some(shape_name) },
            BaseTransform(Transform::default()),
            Transform::default(),
            Visibility::default(),
        )).id();
        commands.entity(parent).add_child(shape_entity);
        commands.entity(shape_entity).with_child((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            mesh_tf,
        ));
    }
}

// =====================================================================
// Mesh creation
// =====================================================================

/// Compute the full mesh transform: rotation from orient, scale from bounds.
/// Handles the axis remapping so scale is applied correctly before rotation.
/// Build the mesh transform from bounds and orient.
///
/// The orient matrix M maps local axes to world axes. Each column is a signed unit vector.
/// We scale each column by the corresponding bounds dimension to get the full transform.
///
/// For a unit mesh in a 1x1x1 box:
///   world_pos = M * diag(size) * local_pos
///
/// This naturally handles both rotation and mirroring in one matrix,
/// without needing to decompose into separate rotation + scale.
fn mesh_transform(shape: PrimitiveShape, bounds: &Bounds, orient: &[SignedAxis]) -> Transform {
    let size = bounds.size();
    let om = orient_matrix(orient);

    // Each column of the orient matrix is a unit direction.
    // The local X axis of the mesh should span size along the orient's X direction.
    // But size is in world space, and we need to figure out which size dimension
    // maps to each local axis.
    //
    // The orient matrix column i tells us: "local axis i maps to world direction col_i."
    // The size along local axis i = the world size projected onto col_i's direction.
    // Since col_i is an axis-aligned unit vector, this is just picking the right component.

    let local_x_size = pick_size_for_direction(om.x_axis, size);
    let local_y_size = pick_size_for_direction(om.y_axis, size);
    let local_z_size = pick_size_for_direction(om.z_axis, size);

    // Torus has non-uniform unit dimensions
    let local_scale = match shape {
        PrimitiveShape::Torus => Vec3::new(local_x_size, local_y_size / 0.3, local_z_size),
        _ => Vec3::new(local_x_size, local_y_size, local_z_size),
    };

    // Build the final 3x3 matrix: orient columns scaled by local dimensions.
    // This IS the complete transform — rotation, scale, and mirror all in one.
    let col_x = om.x_axis * local_scale.x;
    let col_y = om.y_axis * local_scale.y;
    let col_z = om.z_axis * local_scale.z;

    let mat = bevy::math::Mat3::from_cols(col_x, col_y, col_z);
    let affine = bevy::math::Affine3A::from_mat3(mat);
    Transform::from_matrix(bevy::math::Mat4::from(affine))
}

/// Given a direction vector (signed unit axis) and world size (sx, sy, sz),
/// return the size component corresponding to that direction.
fn pick_size_for_direction(dir: Vec3, size: (f32, f32, f32)) -> f32 {
    if dir.x.abs() > 0.5 { size.0 }
    else if dir.y.abs() > 0.5 { size.1 }
    else { size.2 }
}

fn make_mesh(
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: PrimitiveShape,
    color: (f32, f32, f32),
    emissive: bool,
    is_mirrored: bool,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    let mesh = match shape {
        // Unit meshes — one per primitive type, scaled by transform
        PrimitiveShape::Box => meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        PrimitiveShape::Sphere => meshes.add(Sphere::new(0.5).mesh().build()),
        PrimitiveShape::Cylinder => meshes.add(Cylinder::new(0.5, 1.0).mesh().build()),
        PrimitiveShape::Cone => meshes.add(super::meshes::create_cone_mesh(24, 32)),
        PrimitiveShape::Dome => meshes.add(super::meshes::create_unit_dome(24, 32)),
        PrimitiveShape::Wedge => meshes.add(super::meshes::create_wedge_mesh()),
        PrimitiveShape::Torus => meshes.add(super::meshes::create_torus_mesh(32, 16)),
        PrimitiveShape::Corner => meshes.add(super::meshes::create_unit_corner()),
    };

    let base_color = Color::srgb(color.0, color.1, color.2);
    let cull_mode = if is_mirrored { None } else { Some(bevy::render::render_resource::Face::Back) };
    let material = if emissive {
        materials.add(StandardMaterial {
            base_color,
            emissive: base_color.into(),
            cull_mode,
            ..default()
        })
    } else {
        materials.add(StandardMaterial {
            base_color,
            cull_mode,
            ..default()
        })
    };

    (mesh, material)
}

// =====================================================================
// Helpers
// =====================================================================

fn bounds_center(bounds: &Option<Bounds>) -> Vec3 {
    match bounds {
        Some(b) => {
            let c = b.center();
            Vec3::new(c.0, c.1, c.2)
        }
        None => Vec3::ZERO,
    }
}

const DEFAULT_BOUNDS: Bounds = Bounds(-0.5, -0.5, -0.5, 0.5, 0.5, 0.5);

/// Ensure bounds are reified before transforming.
fn reify_bounds(node: &mut ShapeNode) {
    if node.bounds.is_none() && node.shape.is_some() {
        node.bounds = Some(DEFAULT_BOUNDS);
    }
}

/// Ensure orient is reified before transforming.
fn reify_orient(node: &mut ShapeNode) {
    if node.orient.is_empty() && node.shape.is_some() {
        node.orient = vec![SignedAxis::X, SignedAxis::Y, SignedAxis::Z];
    }
}

fn offset_bounds(bounds: &mut Option<Bounds>, axis: Axis, offset: f32) {
    if let Some(ref mut b) = bounds {
        match axis {
            Axis::X => { b.0 += offset; b.3 += offset; }
            Axis::Y => { b.1 += offset; b.4 += offset; }
            Axis::Z => { b.2 += offset; b.5 += offset; }
        }
    }
}

/// Flip a node's bounds and orient on the given axis. Recursively flips children.
fn flip_node_bounds(node: &mut ShapeNode, axis: Axis) {
    reify_bounds(node);
    reify_orient(node);

    if let Some(ref mut b) = node.bounds {
        match axis {
            Axis::X => { let tmp = -b.0; b.0 = -b.3; b.3 = tmp; }
            Axis::Y => { let tmp = -b.1; b.1 = -b.4; b.4 = tmp; }
            Axis::Z => { let tmp = -b.2; b.2 = -b.5; b.5 = tmp; }
        }
    }
    for sa in &mut node.orient {
        *sa = flip_signed_axis(*sa, axis);
    }
    for child in &mut node.children {
        flip_node_bounds(child, axis);
    }
}

fn flip_signed_axis(sa: SignedAxis, mirror_axis: Axis) -> SignedAxis {
    match (sa, mirror_axis) {
        (SignedAxis::X, Axis::X) => SignedAxis::NegX,
        (SignedAxis::NegX, Axis::X) => SignedAxis::X,
        (SignedAxis::Y, Axis::Y) => SignedAxis::NegY,
        (SignedAxis::NegY, Axis::Y) => SignedAxis::Y,
        (SignedAxis::Z, Axis::Z) => SignedAxis::NegZ,
        (SignedAxis::NegZ, Axis::Z) => SignedAxis::Z,
        _ => sa,
    }
}


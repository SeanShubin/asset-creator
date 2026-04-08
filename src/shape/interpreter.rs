use bevy::prelude::*;
use std::collections::HashMap;

use super::animation::ShapeAnimator;
use super::definition::{Axis, Bounds, PrimitiveShape, RepeatSpec, ShapeFile, ShapeNode, SignedAxis};

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

pub fn load_shape(ron_str: &str) -> Result<ShapeFile, String> {
    let options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(ron_str).map_err(|e| format!("Failed to parse shape: {e}"))
}

pub fn spawn_shape(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape_file: &ShapeFile,
) -> Entity {
    let root_tf = Transform::from_translation(to_vec3(shape_file.root.at));
    let root = commands.spawn((
        ShapeRoot,
        ShapePart { name: shape_file.root.name.clone() },
        BaseTransform(root_tf),
        ShapeAnimator::new(shape_file.animations.clone()),
        root_tf,
        Visibility::default(),
    )).id();

    let default_color = (0.5, 0.5, 0.5);
    process_node(commands, meshes, materials, root, &shape_file.root, &shape_file.templates, default_color);
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
    templates: &HashMap<String, ShapeNode>,
    inherited_color: (f32, f32, f32),
) {
    let color = node.color.unwrap_or(inherited_color);

    if let Some(template_name) = &node.template {
        if let Some(template) = templates.get(template_name) {
            let merged = merge_template(node, template);
            process_node(commands, meshes, materials, parent, &merged, templates, color);
            return;
        }
    }

    if let Some(repeat) = &node.repeat {
        process_repeat(commands, meshes, materials, parent, node, repeat, templates, color);
        return;
    }

    if let Some(axis) = &node.mirror {
        process_mirror(commands, meshes, materials, parent, node, *axis, templates, color);
        return;
    }

    attach_geometry(commands, meshes, materials, parent, node, color);

    for child in &node.children {
        spawn_child(commands, meshes, materials, parent, child, templates, color);
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
    templates: &HashMap<String, ShapeNode>,
    color: (f32, f32, f32),
) {
    let start = if repeat.center {
        -(repeat.count as f32 - 1.0) * repeat.spacing * 0.5
    } else {
        0.0
    };

    for i in 0..repeat.count {
        let mut instance = node.clone();
        instance.repeat = None;
        offset_along_axis(&mut instance.at, repeat.along, start + i as f32 * repeat.spacing);
        if let Some(ref name) = instance.name {
            instance.name = Some(format!("{name}_{i}"));
        }
        spawn_child(commands, meshes, materials, parent, &instance, templates, color);
    }
}

fn process_mirror(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node: &ShapeNode,
    axis: Axis,
    templates: &HashMap<String, ShapeNode>,
    color: (f32, f32, f32),
) {
    let mut original = node.clone();
    original.mirror = None;
    spawn_child(commands, meshes, materials, parent, &original, templates, color);

    let mirrored = mirror_node(&original, axis);
    spawn_child(commands, meshes, materials, parent, &mirrored, templates, color);
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
    templates: &HashMap<String, ShapeNode>,
    inherited_color: (f32, f32, f32),
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
    process_node(commands, meshes, materials, child, node, templates, color);
}

fn build_child_transform(node: &ShapeNode) -> Transform {
    // If bounds are specified, position is the center of the bounds.
    // Otherwise, use `at`.
    let position = if let Some(bounds) = &node.bounds {
        let c = bounds.center();
        Vec3::new(c.0, c.1, c.2)
    } else {
        to_vec3(node.at)
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
    let orient = node.orient.unwrap_or(SignedAxis::Y);

    let (mesh, material) = make_mesh(meshes, materials, *shape, &bounds, orient, color, node.emissive);
    let mesh_offset = node.pivot.map(to_vec3).unwrap_or(Vec3::ZERO);
    let mesh_scale = mesh_scale_for_shape(*shape, &bounds, orient);
    let mesh_rotation = mesh_rotation_for_orient(*shape, orient);
    let mesh_tf = Transform::from_translation(mesh_offset)
        .with_scale(mesh_scale)
        .with_rotation(mesh_rotation);

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

/// Compute the scale to apply to a unit mesh to fill the bounds.
fn mesh_scale_for_shape(shape: PrimitiveShape, bounds: &Bounds, orient: SignedAxis) -> Vec3 {
    let size = bounds.size();
    match shape {
        PrimitiveShape::Box => Vec3::ONE, // Box mesh is already sized correctly
        PrimitiveShape::Sphere => Vec3::new(size.0, size.1, size.2), // unit sphere (diameter 1) → ellipsoid
        PrimitiveShape::Cylinder => {
            // Unit cylinder: radius 0.5, height 1.0
            // Scale to fill bounds based on orient axis
            match orient.unsigned() {
                Axis::Y => Vec3::new(size.0, size.1, size.2),
                Axis::X => Vec3::new(size.1, size.0, size.2), // swapped after rotation
                Axis::Z => Vec3::new(size.0, size.2, size.1), // swapped after rotation
            }
        }
        PrimitiveShape::Dome => {
            // Dome mesh is generated at the correct size, but needs orient rotation
            match orient.unsigned() {
                Axis::Y => if orient.is_negative() { Vec3::new(1.0, -1.0, 1.0) } else { Vec3::ONE },
                Axis::X => if orient.is_negative() { Vec3::new(-1.0, 1.0, 1.0) } else { Vec3::ONE },
                Axis::Z => if orient.is_negative() { Vec3::new(1.0, 1.0, -1.0) } else { Vec3::ONE },
            }
        }
    }
}

/// Compute the rotation to orient a shape along the specified axis.
fn mesh_rotation_for_orient(shape: PrimitiveShape, orient: SignedAxis) -> Quat {
    match shape {
        PrimitiveShape::Box | PrimitiveShape::Sphere => Quat::IDENTITY,
        PrimitiveShape::Cylinder | PrimitiveShape::Dome => {
            match orient.unsigned() {
                Axis::Y => Quat::IDENTITY,
                Axis::X => Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                Axis::Z => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            }
        }
    }
}

fn make_mesh(
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: PrimitiveShape,
    bounds: &Bounds,
    orient: SignedAxis,
    color: (f32, f32, f32),
    emissive: bool,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    let size = bounds.size();
    let mesh = match shape {
        PrimitiveShape::Box => {
            meshes.add(Cuboid::new(size.0, size.1, size.2))
        }
        PrimitiveShape::Sphere => {
            // Unit sphere scaled to fill bounds as an ellipsoid
            meshes.add(Sphere::new(0.5).mesh().build())
        }
        PrimitiveShape::Cylinder => {
            // Unit cylinder scaled to fill bounds
            meshes.add(Cylinder::new(0.5, 1.0).mesh().build())
        }
        PrimitiveShape::Dome => {
            let (dome_radius, dome_height) = dome_dimensions_from_bounds(&size, orient);
            meshes.add(super::meshes::create_dome_mesh(dome_radius, dome_height, 24, 32))
        }
    };

    // Apply scaling and orientation to make the unit mesh fill the bounds
    // This is handled via the parent entity's transform, not the mesh itself
    // For Box, the mesh is already the right size
    // For Sphere, Cylinder, and Dome we need to apply scale

    let base_color = Color::srgb(color.0, color.1, color.2);
    let material = if emissive {
        materials.add(StandardMaterial {
            base_color,
            emissive: base_color.into(),
            ..default()
        })
    } else {
        materials.add(StandardMaterial::from_color(base_color))
    };

    (mesh, material)
}

/// Compute dome base radius and height from bounds, considering orientation.
fn dome_dimensions_from_bounds(size: &(f32, f32, f32), orient: SignedAxis) -> (f32, f32) {
    match orient.unsigned() {
        Axis::Y => {
            let radius = size.0.min(size.2) / 2.0;
            (radius, size.1)
        }
        Axis::X => {
            let radius = size.1.min(size.2) / 2.0;
            (radius, size.0)
        }
        Axis::Z => {
            let radius = size.0.min(size.1) / 2.0;
            (radius, size.2)
        }
    }
}

// =====================================================================
// Transform helpers
// =====================================================================

fn to_vec3(t: (f32, f32, f32)) -> Vec3 {
    Vec3::new(t.0, t.1, t.2)
}

fn offset_along_axis(at: &mut (f32, f32, f32), axis: Axis, offset: f32) {
    match axis {
        Axis::X => at.0 += offset,
        Axis::Y => at.1 += offset,
        Axis::Z => at.2 += offset,
    }
}

// =====================================================================
// Node manipulation
// =====================================================================

fn mirror_node(node: &ShapeNode, axis: Axis) -> ShapeNode {
    let mut m = node.clone();
    match axis {
        Axis::X => {
            m.at.0 = -m.at.0;
            if let Some(ref mut b) = m.bounds {
                let tmp = -b.0;
                b.0 = -b.3;
                b.3 = tmp;
            }
            if let Some(ref mut p) = m.pivot { p.0 = -p.0; }
        }
        Axis::Y => {
            m.at.1 = -m.at.1;
            if let Some(ref mut b) = m.bounds {
                let tmp = -b.1;
                b.1 = -b.4;
                b.4 = tmp;
            }
            if let Some(ref mut p) = m.pivot { p.1 = -p.1; }
        }
        Axis::Z => {
            m.at.2 = -m.at.2;
            if let Some(ref mut b) = m.bounds {
                let tmp = -b.2;
                b.2 = -b.5;
                b.5 = tmp;
            }
            if let Some(ref mut p) = m.pivot { p.2 = -p.2; }
        }
    }
    m.children = m.children.iter().map(|c| mirror_node(c, axis)).collect();
    if let Some(ref name) = m.name {
        m.name = Some(format!("{name}_mirrored"));
    }
    m
}

fn merge_template(instance: &ShapeNode, template: &ShapeNode) -> ShapeNode {
    ShapeNode {
        name: instance.name.clone().or(template.name.clone()),
        shape: instance.shape.or(template.shape),
        bounds: instance.bounds.or(template.bounds),
        at: instance.at,
        orient: instance.orient.or(template.orient),
        pivot: instance.pivot.or(template.pivot),
        color: instance.color.or(template.color),
        emissive: instance.emissive || template.emissive,
        rotate: instance.rotate.or(template.rotate),
        template: None,
        children: if instance.children.is_empty() {
            template.children.clone()
        } else {
            instance.children.clone()
        },
        mirror: instance.mirror.or(template.mirror),
        repeat: instance.repeat.clone().or(template.repeat.clone()),
    }
}

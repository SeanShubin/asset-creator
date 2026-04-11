use bevy::prelude::*;
use crate::registry::AssetRegistry;
use crate::util::Color3;
use super::animation::ShapeAnimator;
use super::csg;
use super::definition::{CombineMode, PrimitiveShape, ShapeNode};
use super::traversal::{
    ColorMap, ShapeEvent,
    bounds_center, resolve_color, walk_shape_tree, collect_mesh_from_events,
};

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

#[derive(Component, Clone)]
pub struct CsgGroup {
    pub children: Vec<ShapeNode>,
    pub colors: ColorMap,
}

#[derive(Component)]
pub struct CsgResult;

#[derive(Component)]
pub struct CsgChildState {
    pub active: Vec<bool>,
}

#[derive(Component)]
pub struct CsgMember;

// =====================================================================
// Public API
// =====================================================================

pub fn spawn_shape(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: &ShapeNode,
    registry: &AssetRegistry,
) -> Entity {
    validate_names(shape, "");

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

    let colors = shape.palette.clone();
    let events = walk_shape_tree(shape, &colors, registry);
    apply_events_as_entities(commands, meshes, materials, root, &events, registry);
    root
}

fn validate_names(node: &ShapeNode, path: &str) {
    let node_path = match &node.name {
        Some(name) => {
            if path.is_empty() { name.clone() } else { format!("{path}/{name}") }
        }
        None => {
            if node.shape.is_some() {
                warn!("Unnamed shape at path '{path}' — every shape should have a name");
            }
            path.to_string()
        }
    };

    let mut seen = std::collections::HashSet::new();
    for child in &node.children {
        if let Some(ref name) = child.name {
            if !seen.insert(name.clone()) {
                warn!("Duplicate child name '{}' at path '{}'", name, node_path);
            }
        }
    }

    for child in &node.children {
        validate_names(child, &node_path);
    }
}

pub fn despawn_shape(commands: &mut Commands, roots: &[Entity]) {
    for &e in roots {
        commands.entity(e).despawn_recursive();
    }
}

// =====================================================================
// Entity creation from shape events
// =====================================================================

struct CsgFrame {
    children: Vec<ShapeNode>,
    colors: ColorMap,
}

fn apply_events_as_entities(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    root: Entity,
    events: &[ShapeEvent],
    registry: &AssetRegistry,
) {
    let mut entity_stack: Vec<Entity> = vec![root];
    let mut csg_stack: Vec<Option<CsgFrame>> = vec![None];

    for event in events {
        match event {
            ShapeEvent::EnterNode { node, local_tf, colors } => {
                let parent = *entity_stack.last().unwrap();
                let parent_is_csg = csg_stack.last().unwrap().is_some();

                let entity = commands.spawn((
                    ShapePart { name: node.name.clone() },
                    BaseTransform(*local_tf),
                    *local_tf,
                    Visibility::default(),
                )).id();
                commands.entity(parent).add_child(entity);

                if parent_is_csg {
                    commands.entity(entity).insert(CsgMember);
                }

                let is_csg = node.has_csg_children();
                let csg_frame = if is_csg {
                    let all_active = vec![true; node.children.len()];
                    commands.entity(entity).insert((
                        CsgGroup {
                            children: node.children.clone(),
                            colors: colors.clone(),
                        },
                        CsgChildState { active: all_active },
                    ));
                    Some(CsgFrame {
                        children: node.children.clone(),
                        colors: colors.clone(),
                    })
                } else {
                    None
                };

                entity_stack.push(entity);
                csg_stack.push(csg_frame);
            }
            ShapeEvent::Geometry { node, mesh_tf, colors } => {
                let parent = *entity_stack.last().unwrap();
                let Some(shape) = node.shape else { continue };
                let om = node.orient;
                let is_mirrored = om.determinant() < 0.0;

                let color = node.color.as_ref()
                    .map(|name| resolve_color(name, colors))
                    .unwrap_or_else(|| {
                        warn!("Shape '{}' has no color specified",
                            node.name.as_deref().unwrap_or("unnamed"));
                        Color3(0.5, 0.5, 0.5)
                    });

                let (mesh, material) = make_mesh(meshes, materials, shape, color, node.emissive, is_mirrored);

                if node.children.is_empty() {
                    commands.entity(parent).with_child((
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        *mesh_tf,
                        Visibility::default(),
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
                        *mesh_tf,
                        Visibility::default(),
                    ));
                }
            }
            ShapeEvent::ExitNode => {
                let entity = entity_stack.pop().unwrap();
                let csg_frame = csg_stack.pop().unwrap();

                if let Some(frame) = csg_frame {
                    build_csg_mesh(
                        commands, meshes, materials,
                        entity, &frame.children, &frame.colors, registry,
                    );
                }
            }
        }
    }
}

// =====================================================================
// CSG mesh building
// =====================================================================

/// Build CSG mesh and attach it to the parent entity.
/// Called during initial spawn and during toggle rebuild.
pub fn build_csg_mesh(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    children: &[ShapeNode],
    colors: &ColorMap,
    registry: &AssetRegistry,
) {
    let mut union_meshes = Vec::new();
    let mut subtract_meshes = Vec::new();
    let mut clip_meshes = Vec::new();

    for child in children {
        let events = walk_shape_tree(child, colors, registry);
        let raw = collect_mesh_from_events(&events);
        if raw.positions.is_empty() { continue; }
        match child.combine {
            CombineMode::Union => union_meshes.push(raw),
            CombineMode::Subtract => subtract_meshes.push(raw),
            CombineMode::Clip => clip_meshes.push(raw),
        }
    }

    if union_meshes.is_empty() {
        return;
    }

    let result = csg::perform_csg_pipeline(union_meshes, subtract_meshes, clip_meshes);
    if result.positions.is_empty() {
        return;
    }

    let color = children.iter()
        .find(|c| c.combine == CombineMode::Union)
        .and_then(|c| c.color.as_ref())
        .map(|name| resolve_color(name, colors))
        .unwrap_or(Color3(0.5, 0.5, 0.5));

    let base_color = Color::srgb(color.0, color.1, color.2);
    let material = materials.add(StandardMaterial {
        base_color,
        cull_mode: None,
        ..default()
    });

    let mesh_handle = meshes.add(result.to_bevy_mesh());

    commands.entity(parent).with_child((
        CsgResult,
        Mesh3d(mesh_handle),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::default(),
    ));
}

// =====================================================================
// Mesh creation helpers
// =====================================================================

fn make_mesh(
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: PrimitiveShape,
    color: Color3,
    emissive: bool,
    is_mirrored: bool,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    let mesh = match shape {
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
// CSG systems
// =====================================================================

pub fn suppress_csg_member_meshes(
    mut commands: Commands,
    members: Query<(Entity, &Parent), With<CsgMember>>,
    children_query: Query<&Children>,
    mesh_entities: Query<Entity, With<Mesh3d>>,
    csg_results: Query<&CsgResult>,
) {
    for (member, parent) in &members {
        let has_csg_result = children_query.get(parent.get())
            .map(|children| children.iter().any(|e| csg_results.get(*e).is_ok()))
            .unwrap_or(false);

        let target_vis = if has_csg_result { Visibility::Hidden } else { Visibility::Inherited };
        set_descendant_mesh_visibility(&mut commands, member, &children_query, &mesh_entities, target_vis);
    }
}

fn set_descendant_mesh_visibility(
    commands: &mut Commands,
    entity: Entity,
    children_query: &Query<&Children>,
    mesh_entities: &Query<Entity, With<Mesh3d>>,
    vis: Visibility,
) {
    if let Ok(children) = children_query.get(entity) {
        for &child in children.iter() {
            if mesh_entities.get(child).is_ok() {
                if let Some(mut ec) = commands.get_entity(child) {
                    ec.insert(vis);
                }
            }
            set_descendant_mesh_visibility(commands, child, children_query, mesh_entities, vis);
        }
    }
}

pub fn rebuild_csg_on_toggle(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    registry: Res<AssetRegistry>,
    mut csg_groups: Query<(Entity, &CsgGroup, &mut CsgChildState, &Children)>,
    parts: Query<&ShapePart>,
    visibility: Query<&Visibility>,
    csg_results: Query<Entity, With<CsgResult>>,
) {
    for (parent, group, mut state, children) in &mut csg_groups {
        let part_children: Vec<Entity> = children.iter()
            .filter(|e| parts.get(**e).is_ok())
            .copied()
            .collect();

        let current_active: Vec<bool> = part_children.iter()
            .map(|&e| {
                visibility.get(e)
                    .map(|v| *v != Visibility::Hidden)
                    .unwrap_or(true)
            })
            .collect();

        if current_active == state.active {
            continue;
        }
        state.active = current_active.clone();

        for &child in children.iter() {
            if csg_results.get(child).is_ok() {
                if let Some(ec) = commands.get_entity(child) {
                    ec.despawn_recursive();
                }
            }
        }

        let active_children: Vec<ShapeNode> = group.children.iter()
            .zip(current_active.iter())
            .filter(|(_, active)| **active)
            .map(|(node, _)| node.clone())
            .collect();

        if active_children.is_empty() || !active_children.iter().any(|c| c.combine != CombineMode::Union) {
            continue;
        }

        build_csg_mesh(
            &mut commands, &mut meshes, &mut materials,
            parent, &active_children, &group.colors, &registry,
        );
    }
}

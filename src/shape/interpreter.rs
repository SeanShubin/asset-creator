use bevy::prelude::*;
use bevy::render::view::RenderLayers;
use crate::registry::AssetRegistry;
use crate::util::Color3;
use super::animation::ShapeAnimator;
use super::csg;
use super::render::{ColorMap, RenderEvent, compile, resolve_color};
use super::spec::{
    apply_placement_to_bounds, Bounds, CombineMode, Placement, PrimitiveShape, SpecNode,
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

/// CSG parent group. Holds the post-symmetry-expansion spec children —
/// each paired with its accumulated placement — so the group can rebuild
/// the CSG mesh when children are toggled. This is the ONE component
/// that holds `SpecNode` data in the render world; it never reads
/// individual fields, only forwards the list back to the CSG pipeline.
#[derive(Component, Clone)]
pub struct CsgGroup {
    pub children: Vec<(SpecNode, Placement)>,
    pub colors: ColorMap,
    pub scale: (i32, i32, i32),
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
    shape: &SpecNode,
    registry: &AssetRegistry,
) -> Entity {
    spawn_shape_with_layers(commands, meshes, materials, shape, registry, None)
}

/// Spawn a shape rendered entirely through the SDF/dual contouring pipeline.
/// Produces the same visual style as CSG output — flat shading, uniform mesh.
pub fn spawn_shape_as_sdf(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: &SpecNode,
    registry: &AssetRegistry,
) -> Entity {
    let root_tf = Transform::IDENTITY;
    let root = commands
        .spawn((
            ShapeRoot,
            ShapePart { name: shape.name.clone() },
            BaseTransform(root_tf),
            ShapeAnimator::new(shape.animations.clone()),
            root_tf,
            Visibility::default(),
        ))
        .id();

    let colors = shape.palette.clone();
    let aabb = shape.compute_aabb().unwrap_or(Bounds(-1, -1, -1, 1, 1, 1));

    let events = compile(shape, &colors, registry);
    let result = csg::mesh_sdf_from_events(&events, &aabb);
    info!("SDF preview: {} tris", result.indices.len() / 3);

    if !result.positions.is_empty() {
        let color = shape
            .children
            .first()
            .and_then(|c| c.color.as_ref())
            .map(|name| resolve_color(name, &colors))
            .unwrap_or(Color3(1, 1, 1));

        let (cr, cg, cb) = color.to_rgb();
        let base_color = Color::srgb(cr, cg, cb);
        let material = materials.add(StandardMaterial {
            base_color,
            cull_mode: None,
            ..default()
        });

        let mesh_handle = meshes.add(result.to_bevy_mesh());
        commands.entity(root).with_child((
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::default(),
        ));
    }

    root
}

pub fn spawn_shape_with_layers(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: &SpecNode,
    registry: &AssetRegistry,
    render_layers: Option<RenderLayers>,
) -> Entity {
    validate_names(shape, "");

    let root_tf = Transform::IDENTITY;
    let mut root_cmd = commands.spawn((
        ShapeRoot,
        ShapePart { name: shape.name.clone() },
        BaseTransform(root_tf),
        ShapeAnimator::new(shape.animations.clone()),
        root_tf,
        Visibility::default(),
    ));
    if let Some(ref layers) = render_layers {
        root_cmd.insert(layers.clone());
    }
    let root = root_cmd.id();

    let colors = shape.palette.clone();
    let events = compile(shape, &colors, registry);
    apply_events_as_entities(commands, meshes, materials, root, &events, registry, &render_layers);
    root
}

fn validate_names(node: &SpecNode, path: &str) {
    let node_path = match &node.name {
        Some(name) => {
            if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            }
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
// Entity creation from render events
// =====================================================================

struct CsgPendingBuild {
    parent: Entity,
    children: Vec<(SpecNode, Placement)>,
    colors: ColorMap,
    scale: (i32, i32, i32),
}

fn apply_events_as_entities(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    root: Entity,
    events: &[RenderEvent],
    registry: &AssetRegistry,
    render_layers: &Option<RenderLayers>,
) {
    let mut entity_stack: Vec<Entity> = vec![root];
    let mut pending_csg: Vec<Option<CsgPendingBuild>> = vec![None];
    let mut parent_is_csg_stack: Vec<bool> = vec![false];

    for event in events {
        match event {
            RenderEvent::EnterNode { name, local_tf } => {
                let parent = *entity_stack.last().unwrap();
                let parent_is_csg = *parent_is_csg_stack.last().unwrap();
                let entity = commands
                    .spawn((
                        ShapePart { name: name.clone() },
                        BaseTransform(*local_tf),
                        *local_tf,
                        Visibility::default(),
                    ))
                    .id();
                commands.entity(parent).add_child(entity);
                tag_render_layer(commands, entity, render_layers);

                if parent_is_csg {
                    commands.entity(entity).insert(CsgMember);
                }

                entity_stack.push(entity);
                pending_csg.push(None);
                parent_is_csg_stack.push(false);
            }
            RenderEvent::AttachCsgGroup {
                children,
                colors,
                scale,
            } => {
                let entity = *entity_stack.last().unwrap();
                let all_active = vec![true; children.len()];
                commands.entity(entity).insert((
                    CsgGroup {
                        children: children.clone(),
                        colors: colors.clone(),
                        scale: *scale,
                    },
                    CsgChildState { active: all_active },
                ));
                // Mark so children descending from this node get CsgMember
                if let Some(slot) = parent_is_csg_stack.last_mut() {
                    *slot = true;
                }
                // Remember we need to build the CSG mesh on ExitNode
                if let Some(slot) = pending_csg.last_mut() {
                    *slot = Some(CsgPendingBuild {
                        parent: entity,
                        children: children.clone(),
                        colors: colors.clone(),
                        scale: *scale,
                    });
                }
            }
            RenderEvent::Geometry {
                name,
                has_children,
                shape,
                mesh_tf,
                is_mirrored,
                color,
                emissive,
            } => {
                let parent = *entity_stack.last().unwrap();
                attach_mesh(
                    commands,
                    meshes,
                    materials,
                    parent,
                    name.as_deref(),
                    *has_children,
                    *shape,
                    *mesh_tf,
                    *is_mirrored,
                    *color,
                    *emissive,
                    render_layers,
                );
            }
            RenderEvent::PrecomputedMesh { mesh, color } => {
                let parent = *entity_stack.last().unwrap();
                attach_precomputed_mesh(commands, meshes, materials, parent, mesh, *color, render_layers);
            }
            RenderEvent::ExitNode => {
                entity_stack.pop();
                parent_is_csg_stack.pop();
                if let Some(Some(build)) = pending_csg.pop() {
                    build_csg_mesh(
                        commands,
                        meshes,
                        materials,
                        build.parent,
                        &build.children,
                        &build.colors,
                        registry,
                        render_layers,
                        build.scale,
                    );
                }
            }
        }
    }
}

// =====================================================================
// Mesh attachment
// =====================================================================

fn tag_render_layer(commands: &mut Commands, entity: Entity, render_layers: &Option<RenderLayers>) {
    if let Some(ref layers) = render_layers {
        commands.entity(entity).insert(layers.clone());
    }
}

fn attach_mesh(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    node_name: Option<&str>,
    has_children: bool,
    shape: PrimitiveShape,
    mesh_tf: Transform,
    is_mirrored: bool,
    color: Color3,
    emissive: bool,
    render_layers: &Option<RenderLayers>,
) {
    let (mesh, material) = make_mesh(meshes, materials, shape, color, emissive, is_mirrored);

    if !has_children {
        let mesh_entity = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                mesh_tf,
                Visibility::default(),
            ))
            .id();
        commands.entity(parent).add_child(mesh_entity);
        tag_render_layer(commands, mesh_entity, render_layers);
    } else {
        let shape_name = node_name
            .map(|n| format!("{n}_shape"))
            .unwrap_or_else(|| "shape".to_string());
        let shape_entity = commands
            .spawn((
                ShapePart { name: Some(shape_name) },
                BaseTransform(Transform::default()),
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        commands.entity(parent).add_child(shape_entity);
        tag_render_layer(commands, shape_entity, render_layers);

        let mesh_entity = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                mesh_tf,
                Visibility::default(),
            ))
            .id();
        commands.entity(shape_entity).add_child(mesh_entity);
        tag_render_layer(commands, mesh_entity, render_layers);
    }
}

fn attach_precomputed_mesh(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    mesh: &super::meshes::RawMesh,
    color: Color3,
    render_layers: &Option<RenderLayers>,
) {
    if mesh.positions.is_empty() {
        return;
    }

    let (cr, cg, cb) = color.to_rgb();
    let base_color = Color::srgb(cr, cg, cb);
    let material = materials.add(StandardMaterial {
        base_color,
        cull_mode: None,
        ..default()
    });

    let mesh_handle = meshes.add(mesh.clone().to_bevy_mesh());
    let mesh_entity = commands
        .spawn((
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    commands.entity(parent).add_child(mesh_entity);
    tag_render_layer(commands, mesh_entity, render_layers);
}

// =====================================================================
// CSG mesh building
// =====================================================================

pub fn build_csg_mesh(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    children: &[(SpecNode, Placement)],
    colors: &ColorMap,
    registry: &AssetRegistry,
    render_layers: &Option<RenderLayers>,
    scale: (i32, i32, i32),
) {
    // Compute the enclosing AABB by transforming each child's bounds by
    // its accumulated placement, so the SDF meshing region covers every
    // rendered copy.
    let mut mn = (i32::MAX, i32::MAX, i32::MAX);
    let mut mx = (i32::MIN, i32::MIN, i32::MIN);
    let mut found = false;
    for (child, placement) in children {
        if let Some(ref b) = child.bounds {
            let transformed = apply_placement_to_bounds(*placement, *b);
            let t_min = transformed.min();
            let t_max = transformed.max();
            if !found {
                mn = t_min; mx = t_max;
                found = true;
            } else {
                mn.0 = mn.0.min(t_min.0); mn.1 = mn.1.min(t_min.1); mn.2 = mn.2.min(t_min.2);
                mx.0 = mx.0.max(t_max.0); mx.1 = mx.1.max(t_max.1); mx.2 = mx.2.max(t_max.2);
            }
        }
    }
    let aabb = if found {
        Bounds(mn.0, mn.1, mn.2, mx.0, mx.1, mx.2)
    } else {
        Bounds(-1, -1, -1, 1, 1, 1)
    };

    let (result, stats) = csg::perform_csg_from_children(children, colors, registry, &aabb, scale);
    info!("CSG: {} tris in {:.0}ms", stats.output_tris, stats.mesh_time_ms);
    if result.positions.is_empty() {
        return;
    }

    let color = children
        .iter()
        .find(|(c, _)| c.combine == CombineMode::Union)
        .and_then(|(c, _)| c.color.as_ref())
        .map(|name| resolve_color(name, colors))
        .unwrap_or(Color3(1, 1, 1));

    let (cr, cg, cb) = color.to_rgb();
    let base_color = Color::srgb(cr, cg, cb);
    let material = materials.add(StandardMaterial {
        base_color,
        cull_mode: None,
        ..default()
    });

    let mesh_handle = meshes.add(result.to_bevy_mesh());

    let csg_entity = commands
        .spawn((
            CsgResult,
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    commands.entity(parent).add_child(csg_entity);
    if let Some(ref layers) = render_layers {
        commands.entity(csg_entity).insert(layers.clone());
    }
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
        PrimitiveShape::Wedge => meshes.add(super::meshes::create_wedge_mesh()),
        PrimitiveShape::Corner => meshes.add(super::meshes::create_unit_corner()),
    };

    let (cr, cg, cb) = color.to_rgb();
    let base_color = Color::srgb(cr, cg, cb);
    let cull_mode = if is_mirrored {
        None
    } else {
        Some(bevy::render::render_resource::Face::Back)
    };
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
        let has_csg_result = children_query
            .get(parent.get())
            .map(|children| children.iter().any(|e| csg_results.get(*e).is_ok()))
            .unwrap_or(false);

        let target_vis = if has_csg_result {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
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
        let part_children: Vec<Entity> = children
            .iter()
            .filter(|e| parts.get(**e).is_ok())
            .copied()
            .collect();

        let current_active: Vec<bool> = part_children
            .iter()
            .map(|&e| {
                visibility
                    .get(e)
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

        let active_children: Vec<(SpecNode, Placement)> = group
            .children
            .iter()
            .zip(current_active.iter())
            .filter(|(_, active)| **active)
            .map(|(pair, _)| pair.clone())
            .collect();

        if active_children.is_empty()
            || !active_children.iter().any(|(c, _)| c.combine != CombineMode::Union)
            || !active_children.iter().any(|(c, _)| c.combine == CombineMode::Union)
        {
            continue;
        }

        build_csg_mesh(
            &mut commands,
            &mut meshes,
            &mut materials,
            parent,
            &active_children,
            &group.colors,
            &registry,
            &None,
            group.scale,
        );
    }
}


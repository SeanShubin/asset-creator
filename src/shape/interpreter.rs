use bevy::prelude::*;
use bevy::render::view::RenderLayers;
use crate::registry::AssetRegistry;
use super::animation::ShapeAnimator;
use super::render::{compile, CompiledShape, FusedMesh};
use super::spec::SpecNode;

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

pub fn spawn_shape(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    shape: &SpecNode,
    registry: &AssetRegistry,
) -> Entity {
    spawn_shape_with_layers(commands, meshes, materials, shape, registry, None)
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

    let compiled = compile(shape, registry);
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

    // The compiled root's own meshes + children are spawned under the
    // ShapeRoot entity.
    attach_compiled(
        commands,
        meshes,
        materials,
        root,
        &compiled,
        &render_layers,
        /* is_root */ true,
    );

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
// Recursive spawn from CompiledShape
// =====================================================================

fn attach_compiled(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    compiled: &CompiledShape,
    render_layers: &Option<RenderLayers>,
    is_root: bool,
) {
    // The root of the compiled tree IS the ShapeRoot entity already;
    // attach its meshes and children directly. For non-root compiled
    // parts we spawn our own ShapePart entity as a child of `parent`.
    let entity = if is_root {
        parent
    } else {
        let part_entity = commands
            .spawn((
                ShapePart { name: compiled.name.clone() },
                BaseTransform(compiled.local_transform),
                compiled.local_transform,
                Visibility::default(),
            ))
            .id();
        commands.entity(parent).add_child(part_entity);
        tag_render_layer(commands, part_entity, render_layers);
        part_entity
    };

    for fused in &compiled.meshes {
        attach_fused_mesh(commands, meshes, materials, entity, fused, render_layers);
    }

    for child in &compiled.children {
        attach_compiled(
            commands,
            meshes,
            materials,
            entity,
            child,
            render_layers,
            /* is_root */ false,
        );
    }
}

fn attach_fused_mesh(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    parent: Entity,
    fused: &FusedMesh,
    render_layers: &Option<RenderLayers>,
) {
    if fused.mesh.is_empty() {
        return;
    }

    // Vertex colors in the mesh carry the per-cell authored color.
    // The material base is white so the vertex colors show through;
    // emissive shapes copy that same white-base and enable emissive.
    let base_color = Color::WHITE;
    let cull_mode = if fused.contains_mirrored {
        None
    } else {
        Some(bevy::render::render_resource::Face::Back)
    };
    let material = if fused.subtract_preview {
        materials.add(StandardMaterial {
            base_color,
            alpha_mode: AlphaMode::Blend,
            cull_mode,
            ..default()
        })
    } else if fused.emissive {
        materials.add(StandardMaterial {
            base_color,
            emissive: Color::WHITE.into(),
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

    let mesh_handle = meshes.add(fused.mesh.clone().to_bevy_mesh());
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

fn tag_render_layer(commands: &mut Commands, entity: Entity, render_layers: &Option<RenderLayers>) {
    if let Some(ref layers) = render_layers {
        commands.entity(entity).insert(layers.clone());
    }
}

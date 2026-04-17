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
    pub subtract: bool,
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
    name: &str,
    parts: &[SpecNode],
    registry: &AssetRegistry,
    hidden: &[String],
) -> Entity {
    spawn_shape_with_layers(commands, meshes, materials, name, parts, registry, None, hidden)
}

pub fn spawn_shape_with_layers(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    name: &str,
    parts: &[SpecNode],
    registry: &AssetRegistry,
    render_layers: Option<RenderLayers>,
    hidden: &[String],
) -> Entity {
    validate_parts(parts);

    let compiled = compile(parts, registry, hidden);
    let animations: Vec<_> = parts.iter()
        .flat_map(|p| p.animations.iter().cloned())
        .collect();
    let root_tf = Transform::IDENTITY;
    let mut root_cmd = commands.spawn((
        ShapeRoot,
        ShapePart { name: Some(name.to_string()), subtract: false },
        BaseTransform(root_tf),
        ShapeAnimator::new(animations),
        root_tf,
        Visibility::default(),
    ));
    if let Some(ref layers) = render_layers {
        root_cmd.insert(layers.clone());
    }
    let root = root_cmd.id();

    attach_compiled(
        commands,
        meshes,
        materials,
        root,
        &compiled,
        &render_layers,
        /* is_root */ true,
        hidden,
        "",
    );

    root
}

fn validate_parts(parts: &[SpecNode]) {
    let mut seen = std::collections::HashSet::new();
    for part in parts {
        if part.name.is_some() && part.import.is_some() {
            error!(
                "Part has both name '{}' and import '{}' — they are mutually exclusive",
                part.name.as_deref().unwrap_or(""),
                part.import.as_deref().unwrap_or(""),
            );
        }
        if let Some(name) = part.effective_name() {
            if !seen.insert(name.to_string()) {
                error!("Duplicate part name '{}'", name);
            }
        } else if part.shape.is_some() {
            warn!("Unnamed shape part — every shape should have a name");
        }
    }
    for part in parts {
        validate_names(part, part.effective_name().unwrap_or(""));
    }
}

fn validate_names(node: &SpecNode, path: &str) {
    let mut seen = std::collections::HashSet::new();
    for child in &node.children {
        if child.name.is_some() && child.import.is_some() {
            error!(
                "Node at '{}' has both name '{}' and import '{}' — they are mutually exclusive",
                path,
                child.name.as_deref().unwrap_or(""),
                child.import.as_deref().unwrap_or(""),
            );
        }
        if let Some(name) = child.effective_name() {
            if !seen.insert(name.to_string()) {
                error!("Duplicate child name '{}' at path '{}'", name, path);
            }
        }
    }

    for child in &node.children {
        let child_name = child.effective_name().unwrap_or("");
        let child_path = if path.is_empty() {
            child_name.to_string()
        } else if child_name.is_empty() {
            path.to_string()
        } else {
            format!("{path}/{child_name}")
        };
        validate_names(child, &child_path);
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
    hidden: &[String],
    parent_path: &str,
) {
    let node_path = if let Some(ref name) = compiled.name {
        if parent_path.is_empty() { name.clone() } else { format!("{parent_path}/{name}") }
    } else {
        parent_path.to_string()
    };

    let entity = if is_root {
        parent
    } else {
        let is_hidden = !node_path.is_empty()
            && hidden.iter().any(|h| h == &node_path);
        let vis = if is_hidden { Visibility::Hidden } else { Visibility::Visible };
        let part_entity = commands
            .spawn((
                ShapePart { name: compiled.name.clone(), subtract: compiled.subtract },
                BaseTransform(compiled.local_transform),
                compiled.local_transform,
                vis,
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
            false,
            hidden,
            &node_path,
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

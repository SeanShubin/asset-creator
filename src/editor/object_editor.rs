use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use std::path::PathBuf;

use crate::browser::ActiveEditor;
use crate::shape::{
    animate_shapes, ShapeAnimator, ShapePart, ShapeRoot,
    despawn_shape, load_shape, spawn_shape,
};
use super::orbit_camera::{self, OrbitCamera, OrbitState};

// =====================================================================
// Plugin
// =====================================================================

pub struct ObjectEditorPlugin;

impl Plugin for ObjectEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ObjectEditorState>()
            .init_resource::<OrbitState>()
            .add_systems(Update, (
                handle_activation,
                reload_shape.run_if(is_object_active),
                orbit_camera::orbit_camera.run_if(is_object_active),
                orbit_camera::orbit_zoom.run_if(is_object_active),
                keyboard_input.run_if(is_object_active),
                animate_shapes.run_if(is_object_active),
                part_tree_ui.run_if(is_object_active),
            ));
    }
}

fn is_object_active(active: Res<ActiveEditor>) -> bool {
    matches!(*active, ActiveEditor::Object { .. })
}

// =====================================================================
// Resources
// =====================================================================

#[derive(Resource, Default)]
struct ObjectEditorState {
    current_path: Option<PathBuf>,
    needs_reload: bool,
    spawned: bool,
    last_seen_editor: Option<ActiveEditor>,
}

#[derive(Component)]
struct ObjectEditorEntity;

// =====================================================================
// Activation / deactivation
// =====================================================================

fn handle_activation(
    active: Res<ActiveEditor>,
    mut state: ResMut<ObjectEditorState>,
    mut commands: Commands,
    existing_editor: Query<Entity, With<ObjectEditorEntity>>,
    existing_shapes: Query<Entity, With<ShapeRoot>>,
) {
    let current = (*active).clone();
    let changed = state.last_seen_editor.as_ref() != Some(&current);
    if !changed { return; }

    let was_object = matches!(&state.last_seen_editor, Some(ActiveEditor::Object { .. }));
    let is_object = matches!(&current, ActiveEditor::Object { .. });

    // Despawn if leaving object editor
    if was_object && !is_object {
        for entity in &existing_editor {
            commands.entity(entity).despawn();
        }
        let roots: Vec<Entity> = existing_shapes.iter().collect();
        despawn_shape(&mut commands, &roots);
        state.spawned = false;
        state.current_path = None;
    }

    // Spawn if entering object editor
    if let ActiveEditor::Object { ref path } = current {
        if !state.spawned {
            spawn_scene(&mut commands);
            state.spawned = true;
        }
        state.current_path = Some(path.clone());
        state.needs_reload = true;
        info!("Object editor activated for '{}'", path.display());
    }

    state.last_seen_editor = Some(current);
}

fn spawn_scene(commands: &mut Commands) {
    orbit_camera::spawn_orbit_camera(commands, ObjectEditorEntity);

    commands.spawn((
        ObjectEditorEntity,
        DirectionalLight {
            illuminance: 8000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.4, 0.0)),
    ));

    commands.insert_resource(AmbientLight {
        color: Color::WHITE,
        brightness: 200.0,
        ..default()
    });
}

// =====================================================================
// Shape loading
// =====================================================================

fn reload_shape(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<ObjectEditorState>,
    existing: Query<Entity, With<ShapeRoot>>,
) {
    if !state.needs_reload { return; }
    state.needs_reload = false;

    let Some(path) = &state.current_path else { return };

    let roots: Vec<Entity> = existing.iter().collect();
    despawn_shape(&mut commands, &roots);

    let ron_str = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to read '{}': {}", path.display(), e);
            return;
        }
    };

    let shape_file = match load_shape(&ron_str) {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to parse '{}': {}", path.display(), e);
            return;
        }
    };

    info!("Loaded shape from '{}' — {} children, {} templates, {} animations",
        path.display(),
        shape_file.root.children.len(),
        shape_file.templates.len(),
        shape_file.animations.len(),
    );

    spawn_shape(&mut commands, &mut meshes, &mut materials, &shape_file);
}

// =====================================================================
// Input
// =====================================================================

fn keyboard_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<ObjectEditorState>,
    mut animators: Query<&mut ShapeAnimator>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        state.needs_reload = true;
        info!("Reloading shape...");
    }
    if keys.just_pressed(KeyCode::Tab) {
        for mut animator in &mut animators {
            animator.cycle_state();
            info!("Animation: {}", animator.active_name());
        }
    }
}

// =====================================================================
// Part tree UI
// =====================================================================

fn part_tree_ui(
    mut contexts: EguiContexts,
    roots: Query<Entity, With<ShapeRoot>>,
    parts: Query<(&ShapePart, Option<&Children>, &Visibility)>,
    mut animators: Query<&mut ShapeAnimator>,
    mut commands: Commands,
) {
    let ctx = contexts.ctx_mut();
    let mut toggles: Vec<(Entity, Visibility)> = Vec::new();

    egui::SidePanel::left("part_tree").min_width(200.0).show(ctx, |ui| {
        animation_controls(ui, &roots, &mut animators);
        ui.heading("Part Tree");
        ui.separator();

        for root in &roots {
            draw_tree_node(ui, root, &parts, &mut toggles, 0, &[]);
        }
    });

    for (entity, vis) in toggles {
        commands.entity(entity).insert(vis);
    }
}

fn animation_controls(
    ui: &mut egui::Ui,
    roots: &Query<Entity, With<ShapeRoot>>,
    animators: &mut Query<&mut ShapeAnimator>,
) {
    for root in roots {
        if let Ok(mut animator) = animators.get_mut(root) {
            ui.heading("Animation");
            ui.horizontal(|ui| {
                ui.label("State:");
                if ui.button(animator.active_name()).clicked() {
                    animator.cycle_state();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Speed:");
                ui.add(egui::Slider::new(&mut animator.speed, 0.0..=5.0));
            });
            ui.separator();
        }
    }
}

// =====================================================================
// Tree rendering
// =====================================================================

fn draw_tree_node(
    ui: &mut egui::Ui,
    entity: Entity,
    parts: &Query<(&ShapePart, Option<&Children>, &Visibility)>,
    toggles: &mut Vec<(Entity, Visibility)>,
    depth: usize,
    ancestors: &[Entity],
) {
    let Ok((part, children, _vis)) = parts.get(entity) else { return };

    let state = compute_tri_state(entity, parts);
    let label = part.name.as_deref().unwrap_or("(unnamed)");
    let indent = "  ".repeat(depth);
    let icon = match state {
        TriState::Visible => "[+]",
        TriState::Hidden => "[-]",
        TriState::Mixed => "[~]",
    };

    if ui.selectable_label(false, format!("{indent}{icon} {label}")).clicked() {
        let new_vis = match state {
            TriState::Hidden => Visibility::Inherited,
            _ => Visibility::Hidden,
        };
        let mut subtree = Vec::new();
        collect_subtree(entity, parts, &mut subtree);
        for e in subtree {
            toggles.push((e, new_vis));
        }
        if new_vis == Visibility::Inherited {
            for &ancestor in ancestors {
                toggles.push((ancestor, Visibility::Inherited));
            }
        }
    }

    if let Some(children) = children {
        let mut path = ancestors.to_vec();
        path.push(entity);
        for &child in children.iter() {
            if parts.get(child).is_ok() {
                draw_tree_node(ui, child, parts, toggles, depth + 1, &path);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TriState { Visible, Hidden, Mixed }

fn compute_tri_state(
    entity: Entity,
    parts: &Query<(&ShapePart, Option<&Children>, &Visibility)>,
) -> TriState {
    let Ok((_part, children, vis)) = parts.get(entity) else { return TriState::Visible };
    let self_visible = *vis != Visibility::Hidden;

    let child_parts: Vec<Entity> = children
        .map(|c| c.iter().copied().filter(|e| parts.get(*e).is_ok()).collect())
        .unwrap_or_default();

    if child_parts.is_empty() {
        return if self_visible { TriState::Visible } else { TriState::Hidden };
    }

    let mut all_visible = self_visible;
    let mut all_hidden = !self_visible;
    for child in &child_parts {
        match compute_tri_state(*child, parts) {
            TriState::Visible => all_hidden = false,
            TriState::Hidden => all_visible = false,
            TriState::Mixed => { all_visible = false; all_hidden = false; }
        }
    }

    if all_visible { TriState::Visible }
    else if all_hidden { TriState::Hidden }
    else { TriState::Mixed }
}

fn collect_subtree(
    entity: Entity,
    parts: &Query<(&ShapePart, Option<&Children>, &Visibility)>,
    out: &mut Vec<Entity>,
) {
    if parts.get(entity).is_err() { return; }
    out.push(entity);
    if let Ok((_, Some(children), _)) = parts.get(entity) {
        for &child in children.iter() {
            collect_subtree(child, parts, out);
        }
    }
}

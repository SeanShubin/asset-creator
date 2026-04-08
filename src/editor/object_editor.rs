use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use std::path::PathBuf;

use crate::browser::ActiveEditor;
use crate::registry::AssetRegistry;
use crate::shape::{
    animate_shapes, ShapeAnimator, ShapePart, ShapeRoot,
    despawn_shape, load_shape, spawn_shape,
};
use super::orbit_camera::{self, OrbitCamera, OrbitState, ZoomLimits};

// =====================================================================
// Plugin
// =====================================================================

pub struct ObjectEditorPlugin;

impl Plugin for ObjectEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ObjectEditorState>()
            .init_resource::<OrbitState>()
            .init_resource::<ZoomLimits>()
            .add_systems(Update, (
                handle_activation,
                watch_shape_file.run_if(is_object_active),
                reload_shape.run_if(is_object_active),
                fit_camera_to_shape.run_if(is_object_active),
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
    last_file_check: f64,
    last_mtime: Option<std::time::SystemTime>,
    needs_fit: bool,
    fit_scale: f32,
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
        despawn_all(&mut commands, &existing_editor, &existing_shapes);
        state.spawned = false;
        state.current_path = None;
        state.last_mtime = None;
    }

    // Switching between shapes — despawn old shape, keep scene
    if was_object && is_object {
        let roots: Vec<Entity> = existing_shapes.iter().collect();
        despawn_shape(&mut commands, &roots);
    }

    // Spawn scene if entering object editor for the first time
    if is_object && !state.spawned {
        spawn_scene(&mut commands);
        state.spawned = true;
    }

    // Load the new shape
    if let ActiveEditor::Object { ref path } = current {
        state.current_path = Some(path.clone());
        state.needs_reload = true;
        state.last_mtime = None;
        info!("Object editor activated for '{}'", path.display());
    }

    state.last_seen_editor = Some(current);
}

fn despawn_all(
    commands: &mut Commands,
    editor_entities: &Query<Entity, With<ObjectEditorEntity>>,
    shape_roots: &Query<Entity, With<ShapeRoot>>,
) {
    for entity in editor_entities {
        commands.entity(entity).despawn_recursive();
    }
    let roots: Vec<Entity> = shape_roots.iter().collect();
    despawn_shape(commands, &roots);
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
// File watching — detect external edits to the shape file
// =====================================================================

fn watch_shape_file(
    mut state: ResMut<ObjectEditorState>,
    time: Res<Time>,
) {
    let Some(path) = state.current_path.clone() else { return };

    // Poll every 500ms
    let now = time.elapsed_secs_f64();
    if now - state.last_file_check < 0.5 { return; }
    state.last_file_check = now;

    let current_mtime = match std::fs::metadata(&path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return,
    };

    if state.last_mtime.is_some_and(|prev| current_mtime <= prev) {
        return;
    }

    state.last_mtime = Some(current_mtime);
    state.needs_reload = true;
}

// =====================================================================
// Shape loading
// =====================================================================

fn reload_shape(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<ObjectEditorState>,
    mut registry: ResMut<AssetRegistry>,
    existing: Query<Entity, With<ShapeRoot>>,
) {
    if !state.needs_reload { return; }
    state.needs_reload = false;

    let Some(path) = &state.current_path else { return };
    let path_str = path.display().to_string();

    let roots: Vec<Entity> = existing.iter().collect();
    despawn_shape(&mut commands, &roots);

    let ron_str = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            registry.set_error(path_str, format!("Read error: {e}"));
            return;
        }
    };

    let shape_file = match load_shape(&ron_str) {
        Ok(f) => f,
        Err(e) => {
            registry.set_error(path_str.clone(), e);
            return;
        }
    };

    registry.clear_error_for(&path_str);

    info!("Loaded shape from '{}' — {} children, {} templates, {} animations",
        path.display(),
        shape_file.root.children.len(),
        shape_file.templates.len(),
        shape_file.animations.len(),
    );

    spawn_shape(&mut commands, &mut meshes, &mut materials, &shape_file);
    state.needs_fit = true;
}

// =====================================================================
// Camera fitting
// =====================================================================

const LEFT_PANEL_PX: f32 = 280.0;
const RIGHT_PANEL_PX: f32 = 250.0;
const FIT_BORDER: f32 = 1.1; // 5% border on each side ≈ 10% total

fn fit_camera_to_shape(
    mut state: ResMut<ObjectEditorState>,
    mut camera: Query<(&mut Projection, &Camera), With<OrbitCamera>>,
    mut limits: ResMut<ZoomLimits>,
    mesh_aabbs: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
) {
    if !state.needs_fit { return; }

    // Wait until mesh AABBs are computed (takes one frame after spawning)
    if mesh_aabbs.is_empty() { return; }
    state.needs_fit = false;

    let (scene_min, scene_max) = compute_scene_aabb(&mesh_aabbs);
    let scene_size = scene_max - scene_min;
    let max_extent = scene_size.x.max(scene_size.y).max(scene_size.z);

    if max_extent < 0.001 { return; }

    let Ok((mut projection, camera)) = camera.get_single_mut() else { return };
    let Projection::Orthographic(ref mut ortho) = projection.as_mut() else { return };

    // Account for panels reducing usable viewport width
    let viewport_size = camera.logical_viewport_size().unwrap_or(Vec2::new(1100.0, 720.0));
    let usable_width = (viewport_size.x - LEFT_PANEL_PX - RIGHT_PANEL_PX).max(100.0);
    let usable_height = viewport_size.y;

    // Orthographic scale = world units per pixel * viewport half-height
    // We want the shape to fit in both width and height
    let scale_for_width = max_extent * FIT_BORDER / usable_width;
    let scale_for_height = max_extent * FIT_BORDER / usable_height;
    let fit_scale = scale_for_width.max(scale_for_height);

    ortho.scale = fit_scale;
    state.fit_scale = fit_scale;

    // Zoom in up to 2x size (scale = fit/2), zoom out to 1/10 size (scale = fit*10)
    limits.min = fit_scale / 2.0;
    limits.max = fit_scale * 10.0;
}

fn compute_scene_aabb(
    mesh_aabbs: &Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
) -> (Vec3, Vec3) {
    let mut scene_min = Vec3::splat(f32::MAX);
    let mut scene_max = Vec3::splat(f32::MIN);

    for (gtf, aabb) in mesh_aabbs {
        let world_center = gtf.transform_point(aabb.center.into());
        let scale = gtf.compute_transform().scale;
        let half_extents = Vec3::from(aabb.half_extents) * scale.abs();

        scene_min = scene_min.min(world_center - half_extents);
        scene_max = scene_max.max(world_center + half_extents);
    }

    if (scene_max - scene_min).length() < 0.01 {
        scene_min -= Vec3::splat(0.5);
        scene_max += Vec3::splat(0.5);
    }

    (scene_min, scene_max)
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
    orbit: Res<OrbitState>,
    camera: Query<&Projection, With<OrbitCamera>>,
    state: Res<ObjectEditorState>,
) {
    let ctx = contexts.ctx_mut();
    let mut toggles: Vec<(Entity, Visibility)> = Vec::new();

    egui::SidePanel::left("part_tree").min_width(200.0).show(ctx, |ui| {
        camera_info(ui, &orbit, &camera, &state);
        ui.separator();
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

fn camera_info(
    ui: &mut egui::Ui,
    orbit: &OrbitState,
    camera: &Query<&Projection, With<OrbitCamera>>,
    state: &ObjectEditorState,
) {
    let zoom_pct = if state.fit_scale > 0.0 {
        if let Ok(proj) = camera.get_single() {
            if let Projection::Orthographic(ortho) = proj {
                state.fit_scale / ortho.scale * 100.0
            } else { 100.0 }
        } else { 100.0 }
    } else { 100.0 };

    ui.label(format!("Yaw: {:.0}°  Pitch: {:.0}°", orbit.yaw, orbit.pitch));
    ui.label(format!("Zoom: {:.0}%", zoom_pct));
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

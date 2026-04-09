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
                watch_shape_changes.run_if(is_object_active),
                reload_shape.run_if(is_object_active),
                on_model_loaded.run_if(is_object_active),
                compute_stats.run_if(is_object_active),
                orbit_camera::orbit_camera.run_if(is_object_active),
                orbit_camera::orbit_zoom.run_if(is_object_active),
                keyboard_input.run_if(is_object_active),
                animate_shapes.run_if(is_object_active),
                update_light.run_if(is_object_active),
                part_tree_ui.run_if(is_object_active),
                draw_grid.run_if(is_object_active),
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
    last_shape_generation: u64,
    needs_fit: bool,
    needs_stats: bool,
    fit_scale: f32,
    stats: SceneStats,
}

#[derive(Default, Clone)]
struct SceneStats {
    parts: usize,
    triangles: usize,
    draw_calls: usize,
}

#[derive(Component)]
struct ObjectEditorEntity;

#[derive(Component)]
struct EditorLight;

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

    // Load the new shape and fit camera
    if let ActiveEditor::Object { ref path } = current {
        state.current_path = Some(path.clone());
        state.needs_reload = true;
        state.needs_fit = true;
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

    // Light direction chosen so that at default camera (yaw=45°, pitch=35°),
    // the three visible box faces get distinct brightness:
    //   top = brightest, one side = medium, other side = darkest
    // Rotating Y by -60° offsets the light strongly to one side.
    commands.spawn((
        ObjectEditorEntity,
        EditorLight,
        DirectionalLight {
            illuminance: 6000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::default(),
    ));

    commands.insert_resource(AmbientLight {
        color: Color::WHITE,
        brightness: 80.0,
        ..default()
    });
}

// =====================================================================
// File watching — detect external edits to the shape file
// =====================================================================

fn watch_shape_changes(
    mut state: ResMut<ObjectEditorState>,
    registry: Res<AssetRegistry>,
) {
    if registry.shape_generation != state.last_shape_generation {
        state.last_shape_generation = registry.shape_generation;
        state.needs_reload = true;
    }
}

// =====================================================================
// Shape loading
// =====================================================================

fn reload_shape(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<ObjectEditorState>,
    registry: Res<AssetRegistry>,
    existing: Query<Entity, With<ShapeRoot>>,
) {
    if !state.needs_reload { return; }
    state.needs_reload = false;

    let Some(path) = &state.current_path else { return };

    let roots: Vec<Entity> = existing.iter().collect();
    despawn_shape(&mut commands, &roots);

    let Some(shape_file) = registry.get_shape_by_path(path) else {
        error!("Shape at '{}' not found in registry", path.display());
        return;
    };

    spawn_shape(&mut commands, &mut meshes, &mut materials, shape_file, &registry);
    state.needs_stats = true;
}

// =====================================================================
// Camera fitting
// =====================================================================

// Zoom computation uses fixed projection angles (yaw=45, pitch=45) so that
// fit_scale is deterministic regardless of the user's current orbit angle.
// At these angles a unit cube projects to:
//   width  = max_extent * 1.414214  (sqrt(2))
//   height = max_extent * 1.707107  (1 + sqrt(2)/2)
const ZOOM_PROJ_WIDTH_RATIO: f32 = 1.414214;
const ZOOM_PROJ_HEIGHT_RATIO: f32 = 1.707107;
const LEFT_PANEL_PX: f32 = 280.0;
const RIGHT_PANEL_PX: f32 = 250.0;
const VIEWPORT_WIDTH: f32 = 1100.0;
const VIEWPORT_HEIGHT: f32 = 720.0;
const FIT_BORDER: f32 = 1.1;
const ZOOM_MIN_PCT: f32 = 10.0;
const ZOOM_MAX_PCT: f32 = 200.0;

/// Runs on shape switch: computes fit scale and sets initial zoom to 100%.
fn on_model_loaded(
    mut state: ResMut<ObjectEditorState>,
    mut camera: Query<(&mut Projection, &Camera), With<OrbitCamera>>,
    mut limits: ResMut<ZoomLimits>,
    mesh_aabbs: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
) {
    if !state.needs_fit { return; }
    if mesh_aabbs.is_empty() { return; }
    state.needs_fit = false;

    let fit_scale = compute_fit_scale(&mesh_aabbs);
    if fit_scale < 0.001 { return; }

    state.fit_scale = fit_scale;
    update_zoom_limits(&mut limits, fit_scale);

    if let Ok((mut projection, _)) = camera.get_single_mut() {
        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            ortho.scale = fit_scale;
        }
    }
}

/// Runs on every reload: updates stats, fit_scale, and zoom limits without changing zoom.
fn compute_stats(
    mut state: ResMut<ObjectEditorState>,
    mut limits: ResMut<ZoomLimits>,
    parts: Query<&ShapePart>,
    mesh_handles: Query<&Mesh3d>,
    mesh_assets: Res<Assets<Mesh>>,
    mesh_aabbs: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
) {
    if !state.needs_stats { return; }
    if mesh_handles.is_empty() { return; }
    state.needs_stats = false;

    let fit_scale = compute_fit_scale(&mesh_aabbs);
    if fit_scale > 0.001 {
        state.fit_scale = fit_scale;
        update_zoom_limits(&mut limits, fit_scale);
    }

    let part_count = parts.iter().count();
    let draw_calls = mesh_handles.iter().count();

    let mut triangle_count = 0;
    for mesh_handle in &mesh_handles {
        if let Some(mesh) = mesh_assets.get(&mesh_handle.0) {
            if let Some(indices) = mesh.indices() {
                triangle_count += indices.len() / 3;
            }
        }
    }

    state.stats = SceneStats {
        parts: part_count,
        triangles: triangle_count,
        draw_calls,
    };
}

/// Compute the orthographic scale at which the AABB fills the viewport with ~5% border.
/// Uses fixed projection angles (yaw=45, pitch=45) for deterministic results.
fn compute_fit_scale(
    mesh_aabbs: &Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
) -> f32 {
    let (scene_min, scene_max) = compute_scene_aabb(mesh_aabbs);
    let scene_size = scene_max - scene_min;

    if scene_size.length() < 0.001 { return 0.0; }

    // Project using fixed ratios derived from yaw=45, pitch=45
    let max_extent = scene_size.x.max(scene_size.y).max(scene_size.z);
    let proj_width = max_extent * ZOOM_PROJ_WIDTH_RATIO;
    let proj_height = max_extent * ZOOM_PROJ_HEIGHT_RATIO;

    let usable_width = VIEWPORT_WIDTH - LEFT_PANEL_PX - RIGHT_PANEL_PX;

    let scale_for_width = proj_width * FIT_BORDER / usable_width;
    let scale_for_height = proj_height * FIT_BORDER / VIEWPORT_HEIGHT;

    scale_for_width.max(scale_for_height)
}

fn update_zoom_limits(limits: &mut ZoomLimits, fit_scale: f32) {
    limits.min = fit_scale * 100.0 / ZOOM_MAX_PCT;  // 200% → scale = fit/2
    limits.max = fit_scale * 100.0 / ZOOM_MIN_PCT;   // 10% → scale = fit*10
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
// Light — follows camera so lighting is consistent regardless of orbit
// =====================================================================

/// The light direction is computed relative to the camera so that the
/// lit/shadowed pattern stays consistent as you orbit.
/// At default camera (yaw=45, pitch=35), the original fixed light was
/// Euler YXZ (-60°, -50°, 0°). We reproduce this by computing the light
/// rotation from the camera rotation with a fixed offset.
fn update_light(
    orbit: Res<OrbitState>,
    mut light: Query<&mut Transform, With<EditorLight>>,
) {
    let Ok(mut tf) = light.get_single_mut() else { return };

    // Camera rotation
    let cam_rot = Quat::from_euler(
        EulerRot::YXZ,
        orbit.yaw.to_radians(),
        -orbit.pitch.to_radians(),
        0.0,
    );

    // Fixed offset: light comes from upper-left relative to camera view
    let light_offset = Quat::from_euler(
        EulerRot::YXZ,
        15.0_f32.to_radians(),   // slightly left of camera
        -30.0_f32.to_radians(),  // above camera view
        0.0,
    );

    tf.rotation = cam_rot * light_offset;
}

// =====================================================================
// Grid
// =====================================================================

const GRID_HALF_SIZE: f32 = 5.0;
const GRID_LINES: u32 = 10;
const GRID_COLOR_XZ: Color = Color::srgba(0.3, 0.5, 0.3, 0.2);  // floor — greenish
const GRID_COLOR_XY: Color = Color::srgba(0.3, 0.3, 0.5, 0.2);  // behind-right — bluish
const GRID_COLOR_YZ: Color = Color::srgba(0.5, 0.3, 0.3, 0.2);  // behind-left — reddish
const AXIS_COLOR_X: Color = Color::srgba(0.8, 0.2, 0.2, 0.6);
const AXIS_COLOR_Y: Color = Color::srgba(0.2, 0.8, 0.2, 0.6);
const AXIS_COLOR_Z: Color = Color::srgba(0.2, 0.2, 0.8, 0.6);

fn draw_grid(mut gizmos: Gizmos, orbit: Res<OrbitState>) {
    let yaw_rad = orbit.yaw.to_radians();
    let pitch = orbit.pitch;

    // Floor (XZ plane): visible when looking from above (pitch > 0)
    if pitch > 0.0 {
        draw_offset_grid(&mut gizmos, GridPlane::XZ, -GRID_HALF_SIZE, GRID_COLOR_XZ);
        draw_floor_axes(&mut gizmos, -GRID_HALF_SIZE);
    }
    // Ceiling: visible when looking from below (pitch < 0)
    if pitch < 0.0 {
        draw_offset_grid(&mut gizmos, GridPlane::XZ, GRID_HALF_SIZE, GRID_COLOR_XZ);
        draw_floor_axes(&mut gizmos, GRID_HALF_SIZE);
    }

    // XY wall (Z offset): camera Z positive at yaw≈0 → wall at -Z (behind)
    let back_wall_z = if yaw_rad.cos() > 0.0 { -GRID_HALF_SIZE } else { GRID_HALF_SIZE };
    draw_offset_grid(&mut gizmos, GridPlane::XY, back_wall_z, GRID_COLOR_XY);

    // YZ wall (X offset): camera X positive at yaw≈90 → wall at -X (behind)
    let side_wall_x = if yaw_rad.sin() > 0.0 { -GRID_HALF_SIZE } else { GRID_HALF_SIZE };
    draw_offset_grid(&mut gizmos, GridPlane::YZ, side_wall_x, GRID_COLOR_YZ);

    // Y axis line on the side wall
    gizmos.line(
        Vec3::new(side_wall_x, -GRID_HALF_SIZE, 0.0),
        Vec3::new(side_wall_x, GRID_HALF_SIZE, 0.0),
        AXIS_COLOR_Y,
    );
}

fn draw_floor_axes(gizmos: &mut Gizmos, y: f32) {
    gizmos.line(
        Vec3::new(-GRID_HALF_SIZE, y, 0.0),
        Vec3::new(GRID_HALF_SIZE, y, 0.0),
        AXIS_COLOR_X,
    );
    gizmos.line(
        Vec3::new(0.0, y, -GRID_HALF_SIZE),
        Vec3::new(0.0, y, GRID_HALF_SIZE),
        AXIS_COLOR_Z,
    );
}

enum GridPlane { XZ, XY, YZ }

fn draw_offset_grid(gizmos: &mut Gizmos, plane: GridPlane, offset: f32, color: Color) {
    let step = GRID_HALF_SIZE * 2.0 / GRID_LINES as f32;

    for i in 0..=GRID_LINES {
        let t = -GRID_HALF_SIZE + i as f32 * step;

        let (a, b) = match plane {
            GridPlane::XZ => (
                Vec3::new(t, offset, -GRID_HALF_SIZE),
                Vec3::new(t, offset, GRID_HALF_SIZE),
            ),
            GridPlane::XY => (
                Vec3::new(t, -GRID_HALF_SIZE, offset),
                Vec3::new(t, GRID_HALF_SIZE, offset),
            ),
            GridPlane::YZ => (
                Vec3::new(offset, t, -GRID_HALF_SIZE),
                Vec3::new(offset, t, GRID_HALF_SIZE),
            ),
        };
        gizmos.line(a, b, color);

        let (c, d) = match plane {
            GridPlane::XZ => (
                Vec3::new(-GRID_HALF_SIZE, offset, t),
                Vec3::new(GRID_HALF_SIZE, offset, t),
            ),
            GridPlane::XY => (
                Vec3::new(-GRID_HALF_SIZE, t, offset),
                Vec3::new(GRID_HALF_SIZE, t, offset),
            ),
            GridPlane::YZ => (
                Vec3::new(offset, -GRID_HALF_SIZE, t),
                Vec3::new(offset, GRID_HALF_SIZE, t),
            ),
        };
        gizmos.line(c, d, color);
    }
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
    mut orbit: ResMut<OrbitState>,
    mut camera: Query<&mut Projection, With<OrbitCamera>>,
    state: Res<ObjectEditorState>,
) {
    let ctx = contexts.ctx_mut();
    let mut toggles: Vec<(Entity, Visibility)> = Vec::new();

    egui::SidePanel::left("part_tree").min_width(200.0).show(ctx, |ui| {
        camera_controls(ui, &mut orbit, &mut camera, &state);
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

const DEFAULT_YAW: f32 = 45.0;
const DEFAULT_PITCH: f32 = 45.0;

fn camera_controls(
    ui: &mut egui::Ui,
    orbit: &mut OrbitState,
    camera: &mut Query<&mut Projection, With<OrbitCamera>>,
    state: &ObjectEditorState,
) {
    ui.heading("Camera");

    // Editable yaw and pitch
    ui.horizontal(|ui| {
        ui.label("Yaw:");
        ui.add(egui::DragValue::new(&mut orbit.yaw).range(-180.0..=180.0).suffix("°").speed(1.0));
    });
    ui.horizontal(|ui| {
        ui.label("Pitch:");
        ui.add(egui::DragValue::new(&mut orbit.pitch).range(-89.9..=89.9).suffix("°").speed(1.0));
    });

    // Editable zoom as percentage
    let mut zoom_pct = current_zoom_pct(camera, state);
    ui.horizontal(|ui| {
        ui.label("Zoom:");
        if ui.add(egui::DragValue::new(&mut zoom_pct).range(10.0..=200.0).suffix("%").speed(1.0)).changed() {
            set_zoom_from_pct(camera, state, zoom_pct);
        }
    });

    // View direction buttons
    ui.horizontal(|ui| {
        if ui.button("Front").clicked() { orbit.yaw = 0.0; orbit.pitch = 0.0; }
        if ui.button("Right").clicked() { orbit.yaw = 90.0; orbit.pitch = 0.0; }
        if ui.button("Top").clicked() { orbit.yaw = 0.0; orbit.pitch = 89.9; }
    });
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() { orbit.yaw = 180.0; orbit.pitch = 0.0; }
        if ui.button("Left").clicked() { orbit.yaw = -90.0; orbit.pitch = 0.0; }
        if ui.button("Bottom").clicked() { orbit.yaw = 0.0; orbit.pitch = -89.9; }
        if ui.button("Reset").clicked() {
            orbit.yaw = DEFAULT_YAW;
            orbit.pitch = DEFAULT_PITCH;
            orbit.target = Vec3::ZERO;
            set_zoom_from_pct(camera, state, 100.0);
        }
    });

    // Scene stats
    ui.separator();
    ui.label(format!("Parts: {}  Tris: {}  Draws: {}",
        state.stats.parts, state.stats.triangles, state.stats.draw_calls));
}

fn current_zoom_pct(
    camera: &Query<&mut Projection, With<OrbitCamera>>,
    state: &ObjectEditorState,
) -> f32 {
    if state.fit_scale <= 0.0 { return 100.0; }
    if let Ok(proj) = camera.get_single() {
        if let Projection::Orthographic(ref ortho) = *proj {
            return state.fit_scale / ortho.scale * 100.0;
        }
    }
    100.0
}

fn set_zoom_from_pct(
    camera: &mut Query<&mut Projection, With<OrbitCamera>>,
    state: &ObjectEditorState,
    pct: f32,
) {
    if state.fit_scale <= 0.0 { return; }
    if let Ok(mut proj) = camera.get_single_mut() {
        if let Projection::Orthographic(ortho) = proj.as_mut() {
            ortho.scale = state.fit_scale / (pct / 100.0);
        }
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

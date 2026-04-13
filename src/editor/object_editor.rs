use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use std::path::PathBuf;

use crate::browser::ActiveEditor;
use crate::registry::AssetRegistry;
use crate::shape::{
    animate_shapes, rebuild_csg_on_toggle,
    ShapeAnimator, ShapePart, ShapeRoot,
    despawn_shape, spawn_shape, spawn_shape_as_sdf,
};
use super::orbit_camera::{self, CameraIntent, OrbitCamera, OrbitState, ZoomLimits};

// =====================================================================
// Plugin
// =====================================================================

pub struct ObjectEditorPlugin;

impl Plugin for ObjectEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorActivation>()
            .init_resource::<ShapeReloadState>()
            .init_resource::<CsgPreviewMode>()
            .init_resource::<CameraFitState>()
            .init_resource::<SceneStats>()
            .init_resource::<SceneBounds>()
            .init_resource::<OrbitState>()
            .init_resource::<ZoomLimits>()
            .init_resource::<CameraIntent>()
            .add_systems(Update, (
                // Phase 1: detect what needs to change
                (
                    handle_activation,
                    watch_shape_changes.run_if(is_object_active),
                    keyboard_input.run_if(is_object_active),
                ),
                // Phase 2: apply shape reload (depends on phase 1 setting needs_reload)
                reload_shape.run_if(is_object_active),
                // Phase 3: post-load processing (depends on phase 2 spawning entities)
                (
                    on_model_loaded.run_if(is_object_active),
                    compute_stats.run_if(is_object_active),
                ),
            ).chain())
            .add_systems(Update, (
                // Camera: input → intent → apply (chained)
                (
                    orbit_camera::read_camera_input.run_if(is_object_active),
                    orbit_camera::apply_orbit.run_if(is_object_active),
                    orbit_camera::apply_zoom.run_if(is_object_active),
                ).chain(),
                // Independent systems

                animate_shapes.run_if(is_object_active),
                update_light.run_if(is_object_active),
                // CSG toggle: UI → rebuild (may despawn) → flush (so suppress sees consistent state)
                (
                    part_tree_ui.run_if(is_object_active),
                    rebuild_csg_on_toggle.run_if(is_object_active),
                    bevy::ecs::schedule::apply_deferred,
                ).chain(),
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

/// Tracks which shape is active and whether the editor scene is spawned.
#[derive(Resource, Default)]
struct EditorActivation {
    current_path: Option<PathBuf>,
    spawned: bool,
    last_seen_editor: Option<ActiveEditor>,
}

/// Tracks when the shape needs to be reloaded from the registry.
#[derive(Resource, Default)]
struct ShapeReloadState {
    needs_reload: bool,
    last_shape_generation: u64,
}

/// When true, render the shape using SDF dual contouring (same as CSG output).
/// Only applies to shapes without CSG children.
#[derive(Resource, Default)]
struct CsgPreviewMode {
    enabled: bool,
    /// Whether the current shape has CSG — if so, the toggle is hidden.
    shape_has_csg: bool,
}

/// Camera fit state: computed on model load, used by zoom controls.
#[derive(Resource, Default)]
struct CameraFitState {
    needs_fit: bool,
    fit_scale: f32,
}

/// Display statistics for the scene.
#[derive(Resource, Default)]
struct SceneStats {
    needs_update: bool,
    parts: usize,
    triangles: usize,
    draw_calls: usize,
}

/// Scene AABB and derived values for grid sizing and zoom.
#[derive(Resource, Default)]
struct SceneBounds {
    fit_scale: f32,
    scene_min: Vec3,
    scene_max: Vec3,
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
    mut activation: ResMut<EditorActivation>,
    mut reload: ResMut<ShapeReloadState>,
    mut fit: ResMut<CameraFitState>,
    mut commands: Commands,
    existing_editor: Query<Entity, With<ObjectEditorEntity>>,
    existing_shapes: Query<Entity, With<ShapeRoot>>,
) {
    let current = (*active).clone();
    let changed = activation.last_seen_editor.as_ref() != Some(&current);
    if !changed { return; }

    let was_object = matches!(&activation.last_seen_editor, Some(ActiveEditor::Object { .. }));
    let is_object = matches!(&current, ActiveEditor::Object { .. });

    // Despawn if leaving object editor
    if was_object && !is_object {
        despawn_all(&mut commands, &existing_editor, &existing_shapes);
        activation.spawned = false;
        activation.current_path = None;
    }

    // Switching between shapes — despawn old shape, keep scene
    if was_object && is_object {
        let roots: Vec<Entity> = existing_shapes.iter().collect();
        despawn_shape(&mut commands, &roots);
    }

    // Spawn scene if entering object editor for the first time
    if is_object && !activation.spawned {
        spawn_scene(&mut commands);
        activation.spawned = true;
    }

    // Load the new shape and fit camera
    if let ActiveEditor::Object { ref path } = current {
        activation.current_path = Some(path.clone());
        reload.needs_reload = true;
        fit.needs_fit = true;
        info!("Object editor activated for '{}'", path.display());
    }

    activation.last_seen_editor = Some(current);
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
    mut reload: ResMut<ShapeReloadState>,
    registry: Res<AssetRegistry>,
) {
    if registry.shape_generation() != reload.last_shape_generation {
        reload.last_shape_generation = registry.shape_generation();
        reload.needs_reload = true;
    }
}

// =====================================================================
// Shape loading
// =====================================================================

fn reload_shape(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut reload: ResMut<ShapeReloadState>,
    mut stats: ResMut<SceneStats>,
    mut bounds: ResMut<SceneBounds>,
    mut preview: ResMut<CsgPreviewMode>,
    activation: Res<EditorActivation>,
    registry: Res<AssetRegistry>,
    existing: Query<Entity, With<ShapeRoot>>,
) {
    if !reload.needs_reload { return; }
    reload.needs_reload = false;

    let Some(path) = &activation.current_path else { return };

    let roots: Vec<Entity> = existing.iter().collect();
    despawn_shape(&mut commands, &roots);

    let Some(shape_file) = registry.get_shape_by_path(path) else {
        error!("Shape at '{}' not found in registry", path.display());
        return;
    };

    preview.shape_has_csg = shape_file.has_csg_children();

    if let Some(aabb) = shape_file.compute_aabb() {
        let min = aabb.min();
        let max = aabb.max();
        bounds.scene_min = Vec3::new(min.0 as f32, min.1 as f32, min.2 as f32);
        bounds.scene_max = Vec3::new(max.0 as f32, max.1 as f32, max.2 as f32);
    }

    if preview.enabled && !preview.shape_has_csg {
        spawn_shape_as_sdf(&mut commands, &mut meshes, &mut materials, shape_file, &registry);
    } else {
        spawn_shape(&mut commands, &mut meshes, &mut materials, shape_file, &registry);
    }
    stats.needs_update = true;
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
const LEFT_PANEL_MIN: f32 = 200.0;
const RIGHT_PANEL_MAX: f32 = 250.0;
const FIT_BORDER: f32 = 1.1;
const ZOOM_MIN_PCT: f32 = 10.0;
const ZOOM_MAX_PCT: f32 = 200.0;

/// Runs on shape switch: computes fit scale and sets initial zoom to 100%.
fn on_model_loaded(
    mut fit: ResMut<CameraFitState>,
    mut camera: Query<(&mut Projection, &Camera), With<OrbitCamera>>,
    mut limits: ResMut<ZoomLimits>,
    mesh_aabbs: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
    windows: Query<&Window>,
) {
    if !fit.needs_fit { return; }
    if mesh_aabbs.is_empty() { return; }
    fit.needs_fit = false;

    let window_size = windows.get_single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1100.0, 720.0));
    let fit_scale = compute_fit_scale(&mesh_aabbs, window_size);
    if fit_scale < 0.001 { return; }

    fit.fit_scale = fit_scale;
    update_zoom_limits(&mut limits, fit_scale);

    if let Ok((mut projection, _)) = camera.get_single_mut() {
        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            ortho.scale = fit_scale;
        }
    }
}

/// Runs on every reload: updates stats, fit_scale, and zoom limits without changing zoom.
fn compute_stats(
    mut stats: ResMut<SceneStats>,
    mut bounds: ResMut<SceneBounds>,
    mut limits: ResMut<ZoomLimits>,
    parts: Query<&ShapePart>,
    mesh_handles: Query<&Mesh3d>,
    mesh_assets: Res<Assets<Mesh>>,
    mesh_aabbs: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
    windows: Query<&Window>,
) {
    if !stats.needs_update { return; }
    if mesh_handles.is_empty() { return; }
    stats.needs_update = false;

    let window_size = windows.get_single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1100.0, 720.0));
    let fit_scale = compute_fit_scale(&mesh_aabbs, window_size);
    if fit_scale > 0.001 {
        bounds.fit_scale = fit_scale;
        update_zoom_limits(&mut limits, fit_scale);
    }

    stats.parts = parts.iter().count();
    stats.draw_calls = mesh_handles.iter().count();

    let mut triangle_count = 0;
    for mesh_handle in &mesh_handles {
        if let Some(mesh) = mesh_assets.get(&mesh_handle.0) {
            if let Some(indices) = mesh.indices() {
                triangle_count += indices.len() / 3;
            }
        }
    }

    stats.triangles = triangle_count;
}

/// Compute the orthographic scale at which the AABB fills the viewport with ~5% border.
/// Uses fixed projection angles (yaw=45, pitch=45) for deterministic results.
fn compute_fit_scale(
    mesh_aabbs: &Query<(&GlobalTransform, &bevy::render::primitives::Aabb), With<Mesh3d>>,
    window_size: Vec2,
) -> f32 {
    let (scene_min, scene_max) = compute_scene_aabb(mesh_aabbs);
    let scene_size = scene_max - scene_min;

    if scene_size.length() < 0.001 { return 0.0; }

    let max_extent = scene_size.x.max(scene_size.y).max(scene_size.z);
    let proj_width = max_extent * ZOOM_PROJ_WIDTH_RATIO;
    let proj_height = max_extent * ZOOM_PROJ_HEIGHT_RATIO;

    let usable_width = window_size.x - LEFT_PANEL_MIN - RIGHT_PANEL_MAX;

    let scale_for_width = proj_width * FIT_BORDER / usable_width;
    let scale_for_height = proj_height * FIT_BORDER / window_size.y;

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

const GRID_COLOR_XZ: Color = Color::srgba(0.3, 0.5, 0.3, 0.2);  // floor — greenish
const GRID_COLOR_XY: Color = Color::srgba(0.3, 0.3, 0.5, 0.2);  // behind-right — bluish
const GRID_COLOR_YZ: Color = Color::srgba(0.5, 0.3, 0.3, 0.2);  // behind-left — reddish
const AXIS_COLOR_X: Color = Color::srgba(0.8, 0.2, 0.2, 0.6);
const AXIS_COLOR_Y: Color = Color::srgba(0.2, 0.8, 0.2, 0.6);
const AXIS_COLOR_Z: Color = Color::srgba(0.2, 0.2, 0.8, 0.6);

fn draw_grid(
    mut gizmos: Gizmos,
    orbit: Res<OrbitState>,
    bounds: Res<SceneBounds>,
) {
    if bounds.scene_min == bounds.scene_max { return; }

    let yaw_rad = orbit.yaw.to_radians();
    let pitch = orbit.pitch;

    let scene_min = bounds.scene_min;
    let scene_max = bounds.scene_max;
    // Snap AABB to nearest integer first (handles float imprecision),
    // then add 1 unit margin
    let gmin = Vec3::new(
        scene_min.x.round() - 1.0,
        scene_min.y.round() - 1.0,
        scene_min.z.round() - 1.0,
    );
    let gmax = Vec3::new(
        scene_max.x.round() + 1.0,
        scene_max.y.round() + 1.0,
        scene_max.z.round() + 1.0,
    );

    // Floor (XZ plane): visible when looking from above (pitch > 0)
    if pitch > 0.0 {
        draw_plane_grid(&mut gizmos, GridPlane::XZ, gmin.y, gmin, gmax, GRID_COLOR_XZ);
        draw_floor_axes(&mut gizmos, gmin.y, gmin, gmax);
    }
    // Ceiling: visible when looking from below (pitch < 0)
    if pitch < 0.0 {
        draw_plane_grid(&mut gizmos, GridPlane::XZ, gmax.y, gmin, gmax, GRID_COLOR_XZ);
        draw_floor_axes(&mut gizmos, gmax.y, gmin, gmax);
    }

    // XY wall: camera Z positive → wall at gmin.z (behind)
    let wall_z = if yaw_rad.cos() > 0.0 { gmin.z } else { gmax.z };
    draw_plane_grid(&mut gizmos, GridPlane::XY, wall_z, gmin, gmax, GRID_COLOR_XY);

    // YZ wall: camera X positive → wall at gmin.x (behind)
    let wall_x = if yaw_rad.sin() > 0.0 { gmin.x } else { gmax.x };
    draw_plane_grid(&mut gizmos, GridPlane::YZ, wall_x, gmin, gmax, GRID_COLOR_YZ);

    // Y axis on side wall
    gizmos.line(Vec3::new(wall_x, gmin.y - 0.5, 0.0), Vec3::new(wall_x, gmax.y + 0.5, 0.0), AXIS_COLOR_Y);
}

fn draw_floor_axes(gizmos: &mut Gizmos, y: f32, gmin: Vec3, gmax: Vec3) {
    gizmos.line(Vec3::new(gmin.x - 0.5, y, 0.0), Vec3::new(gmax.x + 0.5, y, 0.0), AXIS_COLOR_X);
    gizmos.line(Vec3::new(0.0, y, gmin.z - 0.5), Vec3::new(0.0, y, gmax.z + 0.5), AXIS_COLOR_Z);
}

enum GridPlane { XZ, XY, YZ }

fn draw_plane_grid(gizmos: &mut Gizmos, plane: GridPlane, offset: f32, gmin: Vec3, gmax: Vec3, color: Color) {
    let (a_min, a_max, b_min, b_max) = match plane {
        GridPlane::XZ => (gmin.x, gmax.x, gmin.z, gmax.z),
        GridPlane::XY => (gmin.x, gmax.x, gmin.y, gmax.y),
        GridPlane::YZ => (gmin.y, gmax.y, gmin.z, gmax.z),
    };

    // Lines along the first axis at integer positions of the second axis
    let b_start = b_min.ceil() as i32;
    let b_end = b_max.floor() as i32;
    for i in b_start..=b_end {
        let t = i as f32;
        let extend = if i == 0 { 0.5 } else { 0.0 };
        let (a, b) = match plane {
            GridPlane::XZ => (Vec3::new(a_min - extend, offset, t), Vec3::new(a_max + extend, offset, t)),
            GridPlane::XY => (Vec3::new(a_min - extend, t, offset), Vec3::new(a_max + extend, t, offset)),
            GridPlane::YZ => (Vec3::new(offset, a_min - extend, t), Vec3::new(offset, a_max + extend, t)),
        };
        gizmos.line(a, b, color);
    }

    // Lines along the second axis at integer positions of the first axis
    let a_start = a_min.ceil() as i32;
    let a_end = a_max.floor() as i32;
    for i in a_start..=a_end {
        let t = i as f32;
        let extend = if i == 0 { 0.5 } else { 0.0 };
        let (a, b) = match plane {
            GridPlane::XZ => (Vec3::new(t, offset, b_min - extend), Vec3::new(t, offset, b_max + extend)),
            GridPlane::XY => (Vec3::new(t, b_min - extend, offset), Vec3::new(t, b_max + extend, offset)),
            GridPlane::YZ => (Vec3::new(offset, t, b_min - extend), Vec3::new(offset, t, b_max + extend)),
        };
        gizmos.line(a, b, color);
    }
}


// =====================================================================
// Input
// =====================================================================

fn keyboard_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut reload: ResMut<ShapeReloadState>,
    mut animators: Query<&mut ShapeAnimator>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        reload.needs_reload = true;
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
    fit: Res<CameraFitState>,
    stats: Res<SceneStats>,
    mut preview: ResMut<CsgPreviewMode>,
    mut reload: ResMut<ShapeReloadState>,
) {
    let Some(ctx) = contexts.try_ctx_mut() else { return };
    let mut toggles: Vec<(Entity, Visibility)> = Vec::new();

    egui::SidePanel::left("part_tree").min_width(200.0).show(ctx, |ui| {
        camera_controls(ui, &mut orbit, &mut camera, &fit, &stats);
        ui.separator();

        if !preview.shape_has_csg {
            if ui.checkbox(&mut preview.enabled, "Preview CSG mesh").changed() {
                reload.needs_reload = true;
            }
            ui.separator();
        }

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
    fit: &CameraFitState,
    stats: &SceneStats,
) {
    ui.heading("Camera");

    ui.horizontal(|ui| {
        ui.label("Yaw:");
        ui.add(egui::DragValue::new(&mut orbit.yaw).range(-180.0..=180.0).suffix("°").speed(1.0));
    });
    ui.horizontal(|ui| {
        ui.label("Pitch:");
        ui.add(egui::DragValue::new(&mut orbit.pitch).range(-89.9..=89.9).suffix("°").speed(1.0));
    });

    let mut zoom_pct = current_zoom_pct(camera, fit.fit_scale);
    ui.horizontal(|ui| {
        ui.label("Zoom:");
        if ui.add(egui::DragValue::new(&mut zoom_pct).range(10.0..=200.0).suffix("%").speed(1.0)).changed() {
            set_zoom_from_pct(camera, fit.fit_scale, zoom_pct);
        }
    });

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
            set_zoom_from_pct(camera, fit.fit_scale, 100.0);
        }
    });

    ui.separator();
    ui.label(format!("Parts: {}  Tris: {}  Draws: {}",
        stats.parts, stats.triangles, stats.draw_calls));
}

fn current_zoom_pct(
    camera: &Query<&mut Projection, With<OrbitCamera>>,
    fit_scale: f32,
) -> f32 {
    if fit_scale <= 0.0 { return 100.0; }
    if let Ok(proj) = camera.get_single() {
        if let Projection::Orthographic(ref ortho) = *proj {
            return fit_scale / ortho.scale * 100.0;
        }
    }
    100.0
}

fn set_zoom_from_pct(
    camera: &mut Query<&mut Projection, With<OrbitCamera>>,
    fit_scale: f32,
    pct: f32,
) {
    if fit_scale <= 0.0 { return; }
    if let Ok(mut proj) = camera.get_single_mut() {
        if let Projection::Orthographic(ortho) = proj.as_mut() {
            ortho.scale = fit_scale / (pct / 100.0);
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

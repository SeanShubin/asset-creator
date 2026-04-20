use bevy::prelude::*;
use bevy::camera::Viewport;
use bevy::camera::primitives::Aabb;
use bevy::gizmos::config::{GizmoConfigGroup, GizmoConfigStore};
use bevy::mesh::Mesh3d;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use std::path::PathBuf;

use crate::registry::{AssetRegistry, shape_name_from_path};
use crate::shape::{
    collect_occupancy, despawn_shape,
    production_stats, spawn_shape,
    PlacementCopy, ShapeAnimator, ShapePart, ShapeRoot,
};
use super::edits::{self, CommandHistory, WorkingShape};
use super::orbit_camera::{self, fit_for_aabb, CameraIntent, OrbitCamera, OrbitState, ZoomLimits};

// =====================================================================
// Plugin
// =====================================================================

pub struct ObjectEditorPlugin;

impl Plugin for ObjectEditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ReloadShape>()
            .init_resource::<CurrentShape>()
            .init_resource::<LoadedShape>()
            .init_resource::<ShapeReloadState>()
            .init_resource::<CameraFitState>()
            .init_resource::<SceneStats>()
            .init_resource::<SceneBounds>()
            .init_resource::<OrbitState>()
            .init_resource::<ZoomLimits>()
            .init_resource::<CameraIntent>()
            .init_resource::<ViewportRect>()
            .init_resource::<HiddenParts>()
            .init_resource::<CollidingParts>()
            .init_resource::<Selection>()
            .init_resource::<WorkingShape>()
            .init_resource::<CommandHistory>()
            // Selection wireframe gizmo group with a small negative
            // depth_bias to prevent z-fighting on coplanar faces.
            .init_gizmo_group::<SelectionFrontGizmos>()
            .add_systems(Startup, (spawn_scene, configure_selection_gizmos))
            .add_systems(Update, (
                (
                    edits::reset_working_on_shape_switch,
                    detect_shape_change,
                    watch_shape_changes,
                    edits::delete_selected,
                    edits::undo_redo,
                    keyboard_input,
                ),
                reload_shape,
                (on_model_loaded, compute_stats, edits::auto_save),
            ).chain())
            .add_systems(Update, (update_light, draw_grid, draw_selection_highlight))
            .add_systems(EguiPrimaryContextPass, (
                (
                    orbit_camera::read_camera_input,
                    orbit_camera::apply_orbit,
                    orbit_camera::apply_zoom,
                ).chain(),
                select_on_click,
                left_panel_ui,
                (
                    track_viewport_rect,
                    sync_camera_viewport,
                    sync_zoom_to_viewport,
                ).chain().after(left_panel_ui),
            ));
    }
}

// =====================================================================
// Resources
// =====================================================================

/// The shape the user has selected. UI writes to this; `detect_shape_change`
/// compares it against `LoadedShape` and fires `ReloadShape` on mismatch.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct CurrentShape {
    pub path: Option<PathBuf>,
}

/// The shape that's currently spawned in the scene. Compared against
/// `CurrentShape` to detect user selection changes.
#[derive(Resource, Default)]
struct LoadedShape {
    path: Option<PathBuf>,
}

/// Event: request a shape reload.
#[derive(Message)]
pub struct ReloadShape;

/// Tracks the last-seen shape generation for file-change detection.
#[derive(Resource, Default)]
struct ShapeReloadState {
    last_shape_generation: u64,
}

/// Camera fit state: computed on model load, used by zoom controls.
#[derive(Resource, Default)]
struct CameraFitState {
    needs_fit: bool,
    fit_scale: f32,
}

/// The central viewport rect — the screen area that's actually visible
/// to the user, after egui sidebars are subtracted. Tracks the egui
/// "available rect" each frame in both logical and physical pixels.
/// All fit/zoom computations and the camera viewport read this resource
/// so the abstraction boundary "what's visible" is in one place.
#[derive(Resource, Default, Clone, Copy, Debug)]
struct ViewportRect {
    logical_size: Vec2,
    physical_min: UVec2,
    physical_size: UVec2,
}

impl ViewportRect {
    /// True when the visible rect is large enough to render anything
    /// meaningful. Returns false during transient states where the
    /// window is so small that egui side panels can't fit.
    fn is_renderable(&self) -> bool {
        self.physical_size.x > 0 && self.physical_size.y > 0
    }
}

/// Display statistics for the scene.
#[derive(Resource, Default)]
struct SceneStats {
    needs_update: bool,
    parts: usize,
    triangles: Option<usize>,
    draw_calls: Option<usize>,
    /// Number of cell-level collisions detected in the current shape.
    /// Zero is the clean state; non-zero means two or more primitives
    /// claim the same integer cell. In the editor this is informational;
    /// non-interactive tools treat it as a hard error.
    collisions: usize,
    /// Receiver for background production stats computation.
    stats_receiver: Option<std::sync::Mutex<std::sync::mpsc::Receiver<(usize, usize, f64)>>>,
}

/// Scene AABB from the spec-level occupancy. Doesn't change when
/// parts are hidden/shown — keeps zoom and grid stable.
#[derive(Resource, Default)]
struct SceneBounds {
    scene_min: Vec3,
    scene_max: Vec3,
}

#[derive(Component)]
struct EditorLight;

/// Paths of parts the user has hidden in the parts tree. Paths are
/// slash-separated (e.g. "chassis_top/hole"). Passed to `compile`
/// so hidden parts produce no geometry or CSG effects.
#[derive(Resource, Default)]
struct HiddenParts {
    paths: Vec<String>,
}

/// Names of parts involved in cell collisions. Populated during
/// shape reload from occupancy data.
#[derive(Resource, Default)]
struct CollidingParts {
    names: Vec<String>,
}

/// The currently-selected part, identified by its hierarchical source path
/// (e.g. `"chassis_top/wheel"`). Clicking a derived copy resolves to the
/// source; all copies share one entity per source so this is unambiguous.
#[derive(Resource, Default, Clone, Debug)]
pub struct Selection {
    pub source_path: Option<String>,
}

// =====================================================================
// Scene setup
// =====================================================================

fn spawn_scene(mut commands: Commands) {
    orbit_camera::spawn_orbit_camera(&mut commands);

    // Light direction chosen so that at default camera (yaw=45°, pitch=35°),
    // the three visible box faces get distinct brightness:
    //   top = brightest, one side = medium, other side = darkest
    commands.spawn((
        EditorLight,
        DirectionalLight {
            illuminance: 6000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::default(),
    ));

    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 80.0,
        ..default()
    });
}

// =====================================================================
// Shape selection — detect user-driven changes
// =====================================================================

fn detect_shape_change(
    current: Res<CurrentShape>,
    mut loaded: ResMut<LoadedShape>,
    mut reload_events: MessageWriter<ReloadShape>,
    mut fit: ResMut<CameraFitState>,
    mut orbit: ResMut<OrbitState>,
    mut hidden: ResMut<HiddenParts>,
    mut selection: ResMut<Selection>,
    mut commands: Commands,
    existing_shapes: Query<Entity, With<ShapeRoot>>,
) {
    if current.path == loaded.path { return; }

    let roots: Vec<Entity> = existing_shapes.iter().collect();
    despawn_shape(&mut commands, &roots);
    hidden.paths.clear();
    selection.source_path = None;

    loaded.path = current.path.clone();

    if let Some(ref path) = current.path {
        reload_events.write(ReloadShape);
        fit.needs_fit = true;
        orbit.yaw = DEFAULT_YAW;
        orbit.pitch = DEFAULT_PITCH;
        orbit.target = Vec3::ZERO;
        info!("Loading shape '{}'", path.display());
    }
}

// =====================================================================
// File watching — detect external edits to the shape file
// =====================================================================

fn watch_shape_changes(
    mut reload: ResMut<ShapeReloadState>,
    registry: Res<AssetRegistry>,
    mut reload_events: MessageWriter<ReloadShape>,
) {
    if registry.shape_generation() != reload.last_shape_generation {
        reload.last_shape_generation = registry.shape_generation();
        reload_events.write(ReloadShape);
    }
}

// =====================================================================
// Shape loading
// =====================================================================

fn reload_shape(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut reload_events: MessageReader<ReloadShape>,
    mut stats: ResMut<SceneStats>,
    mut bounds: ResMut<SceneBounds>,
    working: Res<WorkingShape>,
    registry: Res<AssetRegistry>,
    existing: Query<Entity, With<ShapeRoot>>,
    hidden: Res<HiddenParts>,
    mut colliding: ResMut<CollidingParts>,
) {
    if reload_events.read().next().is_none() { return; }
    reload_events.clear();

    let Some(path) = &working.path else { return };

    let roots: Vec<Entity> = existing.iter().collect();
    despawn_shape(&mut commands, &roots);

    if working.parts.is_empty() {
        // Nothing to render — empty working copy, e.g., after Delete-all
        // or before the registry has populated.
        bounds.scene_min = Vec3::ZERO;
        bounds.scene_max = Vec3::ZERO;
        stats.collisions = 0;
        colliding.names.clear();
        return;
    }

    // Compute the cell-level occupancy index once per reload. This is the
    // single source of truth for scene AABB AND collision count.
    let occupancy = collect_occupancy(&working.parts, &registry);

    if let Some(aabb) = occupancy.aabb() {
        let min = aabb.min();
        let max = aabb.max();
        bounds.scene_min = Vec3::new(min.0 as f32, min.1 as f32, min.2 as f32);
        bounds.scene_max = Vec3::new(max.0 as f32, max.1 as f32, max.2 as f32);
    }

    stats.collisions = occupancy.collision_count();
    occupancy.warn_collisions(&format!("shape '{}'", path.display()));

    // Collect the names of parts involved in collisions for tree coloring.
    colliding.names.clear();
    for c in occupancy.collisions() {
        for path in [&c.first_path, &c.second_path] {
            let leaf = path.rsplit('/').next().unwrap_or(path);
            if !colliding.names.iter().any(|n| n == leaf) {
                colliding.names.push(leaf.to_string());
            }
        }
    }

    let name = shape_name_from_path(path);
    spawn_shape(&mut commands, &mut meshes, &mut materials, &name, &working.parts, &registry, &hidden.paths);
    stats.needs_update = true;
}

// =====================================================================
// Camera fitting
// =====================================================================

// Fit math uses fixed projection angles (yaw=45, pitch=45) so the result is
// deterministic regardless of the user's current orbit angle. The 8 AABB
// corners are projected through the view transform and actual screen-space
// extents are measured (see orbit_camera::fit_for_aabb).
const FIT_BORDER_PCT: f32 = 0.05;
const ZOOM_MIN_PCT: f32 = 10.0;
const ZOOM_MAX_PCT: f32 = 200.0;

/// Runs on shape switch: computes fit scale, sets initial zoom to 100%, and
/// targets the camera at the AABB center so off-origin shapes are centered.
fn on_model_loaded(
    mut fit: ResMut<CameraFitState>,
    mut camera: Query<&mut Projection, With<OrbitCamera>>,
    mut limits: ResMut<ZoomLimits>,
    mut orbit: ResMut<OrbitState>,
    bounds: Res<SceneBounds>,
    viewport: Res<ViewportRect>,
) {
    if !fit.needs_fit { return; }
    if !viewport.is_renderable() { return; }
    fit.needs_fit = false;

    let Some(result) = fit_for_bounds(&bounds, viewport.logical_size) else { return };
    if result.scale < 0.001 { return; }

    fit.fit_scale = result.scale;
    orbit.target = result.target;
    update_zoom_limits(&mut limits, result.scale);

    if let Ok(mut projection) = camera.single_mut() {
        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            ortho.scale = result.scale;
        }
    }
}

/// Runs on every reload: updates stats, fit_scale, and zoom limits without changing zoom.
fn compute_stats(
    mut stats: ResMut<SceneStats>,
    bounds: Res<SceneBounds>,
    mut limits: ResMut<ZoomLimits>,
    parts: Query<&ShapePart>,
    working: Res<WorkingShape>,
    registry: Res<AssetRegistry>,
    viewport: Res<ViewportRect>,
) {
    // Poll for background stats result.
    if let Some(ref rx_mutex) = stats.stats_receiver {
        let result = rx_mutex.lock().unwrap().try_recv().ok();
        if let Some((triangles, draw_calls, elapsed_ms)) = result {
            let frames = elapsed_ms / 16.7;
            info!("production stats: {triangles} tris, {draw_calls} draws ({elapsed_ms:.1}ms, ~{frames:.1} frames)");
            stats.triangles = Some(triangles);
            stats.draw_calls = Some(draw_calls);
        }
    }
    if stats.triangles.is_some() && stats.stats_receiver.is_some() {
        stats.stats_receiver = None;
    }

    if !stats.needs_update { return; }
    stats.needs_update = false;

    if viewport.is_renderable() {
        if let Some(result) = fit_for_bounds(&bounds, viewport.logical_size) {
            if result.scale > 0.001 {
                update_zoom_limits(&mut limits, result.scale);
            }
        }
    }

    stats.parts = parts.iter().count();

    if !working.parts.is_empty() {
        stats.triangles = None;
        stats.draw_calls = None;
        let parts_owned = working.parts.clone();
        let registry_owned = registry.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        stats.stats_receiver = Some(std::sync::Mutex::new(rx));
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let prod = production_stats(&parts_owned, &registry_owned);
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let _ = tx.send((prod.triangles, prod.draw_calls, elapsed_ms));
        });
    }
}

// =====================================================================
// Viewport tracking
// =====================================================================

/// Read the egui central rect — the area not covered by side panels — and
/// store it in the `ViewportRect` resource. Runs after all egui panels for
/// this frame have been drawn, so `available_rect()` reflects them all.
fn track_viewport_rect(
    mut contexts: EguiContexts,
    windows: Query<&Window>,
    mut viewport: ResMut<ViewportRect>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let Ok(window) = windows.single() else { return };

    let rect = ctx.available_rect();
    let logical_size = Vec2::new(rect.width().max(0.0), rect.height().max(0.0));

    let scale = window.scale_factor();
    let phys_min_x = (rect.min.x * scale).round().max(0.0) as u32;
    let phys_min_y = (rect.min.y * scale).round().max(0.0) as u32;
    let phys_w = (logical_size.x * scale).round().max(0.0) as u32;
    let phys_h = (logical_size.y * scale).round().max(0.0) as u32;

    // Clamp so position+size never exceeds the physical window, otherwise
    // wgpu rejects the viewport.
    let win_w = window.physical_width();
    let win_h = window.physical_height();
    let phys_min = UVec2::new(phys_min_x.min(win_w), phys_min_y.min(win_h));
    let phys_size = UVec2::new(
        phys_w.min(win_w.saturating_sub(phys_min.x)),
        phys_h.min(win_h.saturating_sub(phys_min.y)),
    );

    *viewport = ViewportRect {
        logical_size,
        physical_min: phys_min,
        physical_size: phys_size,
    };
}

/// Apply the `ViewportRect` to the camera so it only renders inside the
/// central area.
fn sync_camera_viewport(
    viewport: Res<ViewportRect>,
    mut camera: Query<&mut Camera, With<OrbitCamera>>,
) {
    if !viewport.is_renderable() { return; }
    let Ok(mut cam) = camera.single_mut() else { return };
    let new = Viewport {
        physical_position: viewport.physical_min,
        physical_size: viewport.physical_size,
        ..default()
    };
    let changed = match &cam.viewport {
        Some(v) => v.physical_position != new.physical_position
            || v.physical_size != new.physical_size,
        None => true,
    };
    if changed {
        cam.viewport = Some(new);
    }
}

/// On viewport size changes, recompute fit_scale and preserve zoom_pct.
fn sync_zoom_to_viewport(
    viewport: Res<ViewportRect>,
    mut fit: ResMut<CameraFitState>,
    mut limits: ResMut<ZoomLimits>,
    mut camera: Query<&mut Projection, With<OrbitCamera>>,
    bounds: Res<SceneBounds>,
) {
    if fit.needs_fit { return; }
    if !viewport.is_renderable() { return; }
    if fit.fit_scale <= 0.0 { return; }

    let Some(result) = fit_for_bounds(&bounds, viewport.logical_size) else { return };
    let new_fit = result.scale;
    if new_fit < 0.001 { return; }
    if (new_fit - fit.fit_scale).abs() < f32::EPSILON * fit.fit_scale.max(1.0) {
        return;
    }

    let old_fit = fit.fit_scale;
    fit.fit_scale = new_fit;
    update_zoom_limits(&mut limits, new_fit);

    if let Ok(mut projection) = camera.single_mut() {
        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            ortho.scale *= new_fit / old_fit;
        }
    }
}

/// Compute the camera fit (scale + look-at target) for the spec-level
/// scene bounds.
fn fit_for_bounds(bounds: &SceneBounds, viewport_size: Vec2) -> Option<orbit_camera::FitResult> {
    fit_for_aabb(
        bounds.scene_min, bounds.scene_max,
        viewport_size,
        DEFAULT_YAW, DEFAULT_PITCH,
        FIT_BORDER_PCT,
    )
}

fn update_zoom_limits(limits: &mut ZoomLimits, fit_scale: f32) {
    limits.min = fit_scale * 100.0 / ZOOM_MAX_PCT;
    limits.max = fit_scale * 100.0 / ZOOM_MIN_PCT;
}

// =====================================================================
// Light — follows camera so lighting is consistent regardless of orbit
// =====================================================================

fn update_light(
    orbit: Res<OrbitState>,
    mut light: Query<&mut Transform, With<EditorLight>>,
) {
    let Ok(mut tf) = light.single_mut() else { return };
    tf.rotation = orbit_camera::compute_light_rotation(orbit.yaw, orbit.pitch);
}

// =====================================================================
// Grid
// =====================================================================

const GRID_COLOR_XZ: Color = Color::srgba(0.3, 0.5, 0.3, 0.2);
const GRID_COLOR_XY: Color = Color::srgba(0.3, 0.3, 0.5, 0.2);
const GRID_COLOR_YZ: Color = Color::srgba(0.5, 0.3, 0.3, 0.2);
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

    if pitch > 0.0 {
        draw_plane_grid(&mut gizmos, GridPlane::XZ, gmin.y, gmin, gmax, GRID_COLOR_XZ);
        draw_floor_axes(&mut gizmos, gmin.y, gmin, gmax);
    }
    if pitch < 0.0 {
        draw_plane_grid(&mut gizmos, GridPlane::XZ, gmax.y, gmin, gmax, GRID_COLOR_XZ);
        draw_floor_axes(&mut gizmos, gmax.y, gmin, gmax);
    }

    let wall_z = if yaw_rad.cos() > 0.0 { gmin.z } else { gmax.z };
    draw_plane_grid(&mut gizmos, GridPlane::XY, wall_z, gmin, gmax, GRID_COLOR_XY);

    let wall_x = if yaw_rad.sin() > 0.0 { gmin.x } else { gmax.x };
    draw_plane_grid(&mut gizmos, GridPlane::YZ, wall_x, gmin, gmax, GRID_COLOR_YZ);

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
    mut reload_events: MessageWriter<ReloadShape>,
    mut animators: Query<&mut ShapeAnimator>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        reload_events.write(ReloadShape);
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
// Left-panel UI
// =====================================================================

fn left_panel_ui(
    mut contexts: EguiContexts,
    registry: Res<AssetRegistry>,
    mut current: ResMut<CurrentShape>,
    roots: Query<Entity, With<ShapeRoot>>,
    parts: Query<(&ShapePart, Option<&Children>, &Visibility)>,
    mut animators: Query<&mut ShapeAnimator>,
    mut orbit: ResMut<OrbitState>,
    mut camera: Query<&mut Projection, With<OrbitCamera>>,
    fit: Res<CameraFitState>,
    stats: Res<SceneStats>,
    mut hidden: ResMut<HiddenParts>,
    mut reload_events: MessageWriter<ReloadShape>,
    colliding: Res<CollidingParts>,
    mut selection: ResMut<Selection>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut toggles: Vec<(String, Visibility)> = Vec::new();
    let mut select_click: Option<String> = None;
    let selected_path = selection.source_path.clone();

    egui::SidePanel::left("object_editor_panel").min_width(220.0).show(ctx, |ui| {
        shape_list(ui, &registry, &mut current);
        ui.separator();

        camera_controls(ui, &mut orbit, &mut camera, &fit, &stats);
        ui.separator();

        animation_controls(ui, &roots, &mut animators);

        ui.heading("Part Tree");
        ui.separator();
        for root in &roots {
            draw_tree_node(
                ui, root, &parts, &mut toggles, &mut select_click,
                selected_path.as_deref(), 0, &[], &[], &colliding,
            );
        }

        if registry.has_errors() {
            ui.separator();
            error_list(ui, &registry);
        }
    });

    if let Some(path) = select_click {
        selection.source_path = Some(path);
    }

    if !toggles.is_empty() {
        let snapshot = hidden.paths.clone();
        for (path, vis) in &toggles {
            if path.is_empty() { continue; }
            if *vis == Visibility::Hidden {
                if !hidden.paths.contains(path) {
                    hidden.paths.push(path.clone());
                }
            } else {
                hidden.paths.retain(|p| p != path);
            }
        }
        if hidden.paths != snapshot {
            reload_events.write(ReloadShape);
        }
    }
}

fn shape_list(
    ui: &mut egui::Ui,
    registry: &AssetRegistry,
    current: &mut CurrentShape,
) {
    ui.heading("Shapes");
    for (key, path) in &registry.shape_entries() {
        let stem = key.strip_suffix(".shape.ron").unwrap_or(key);
        let is_active = current.path.as_deref() == Some(path.as_path());
        if ui.selectable_label(is_active, stem).clicked() {
            current.path = Some(path.clone());
        }
    }
}

fn error_list(ui: &mut egui::Ui, registry: &AssetRegistry) {
    ui.colored_label(egui::Color32::RED, "Errors");
    for error in registry.errors() {
        let filename = std::path::Path::new(&error.path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&error.path);
        ui.colored_label(egui::Color32::YELLOW, filename);
        ui.label(&error.message);
        ui.add_space(4.0);
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
    let tris = stats.triangles.map_or("...".to_string(), |t| t.to_string());
    let draws = stats.draw_calls.map_or("...".to_string(), |d| d.to_string());
    ui.label(format!(
        "Parts: {}  Tris: {}  Draws: {}  Collisions: {}",
        stats.parts, tris, draws, stats.collisions
    ));
}

fn current_zoom_pct(
    camera: &Query<&mut Projection, With<OrbitCamera>>,
    fit_scale: f32,
) -> f32 {
    if fit_scale <= 0.0 { return 100.0; }
    if let Ok(proj) = camera.single() {
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
    if let Ok(mut proj) = camera.single_mut() {
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

#[allow(clippy::too_many_arguments)]
fn draw_tree_node(
    ui: &mut egui::Ui,
    entity: Entity,
    parts: &Query<(&ShapePart, Option<&Children>, &Visibility)>,
    toggles: &mut Vec<(String, Visibility)>,
    select_click: &mut Option<String>,
    selected_path: Option<&str>,
    depth: usize,
    ancestors: &[Entity],
    name_path: &[String],
    colliding: &CollidingParts,
) {
    let Ok((part, children, _vis)) = parts.get(entity) else { return };

    let node_path = if depth == 0 {
        String::new()
    } else if let Some(name) = &part.name {
        if name_path.is_empty() { name.clone() } else { format!("{}/{name}", name_path.join("/")) }
    } else {
        name_path.join("/")
    };

    let state = compute_tri_state(entity, parts);
    let label = part.name.as_deref().unwrap_or("(unnamed)");
    let indent = "  ".repeat(depth);
    let icon = match state {
        TriState::Visible => "+",
        TriState::Hidden  => "-",
        TriState::Mixed   => "~",
    };

    let label_color = if part.subtract {
        Some(egui::Color32::from_rgb(100, 140, 255))
    } else if colliding.names.iter().any(|c| c == &node_path) {
        Some(egui::Color32::from_rgb(255, 80, 80))
    } else {
        None
    };

    let is_selected = !node_path.is_empty() && selected_path == Some(node_path.as_str());

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if !indent.is_empty() {
            ui.label(indent);
        }
        if ui.selectable_label(false, icon).clicked() {
            let new_vis = match state {
                TriState::Hidden => Visibility::Inherited,
                _ => Visibility::Hidden,
            };
            let mut subtree_paths = Vec::new();
            collect_subtree_paths(entity, parts, &node_path, &mut subtree_paths);
            for path in subtree_paths {
                toggles.push((path, new_vis));
            }
        }
        let label_widget = if let Some(color) = label_color {
            egui::RichText::new(label).color(color)
        } else {
            egui::RichText::new(label)
        };
        if ui.selectable_label(is_selected, label_widget).clicked() && !node_path.is_empty() {
            *select_click = Some(node_path.clone());
        }
    });

    if let Some(children) = children {
        let mut path = ancestors.to_vec();
        path.push(entity);
        let mut child_name_path = name_path.to_vec();
        if depth > 0 {
            if let Some(name) = &part.name {
                child_name_path.push(name.clone());
            }
        }
        for child in children.iter() {
            if parts.get(child).is_ok() {
                draw_tree_node(
                    ui, child, parts, toggles, select_click, selected_path,
                    depth + 1, &path, &child_name_path, colliding,
                );
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
        .map(|c| c.iter().filter(|e| parts.get(*e).is_ok()).collect())
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

/// Collect paths for an entity and all its descendants. `node_path`
/// is the already-built path for this entity (not rebuilt from name).
fn collect_subtree_paths(
    entity: Entity,
    parts: &Query<(&ShapePart, Option<&Children>, &Visibility)>,
    node_path: &str,
    out: &mut Vec<String>,
) {
    let Ok((_, children, _)) = parts.get(entity) else { return };
    if !node_path.is_empty() {
        out.push(node_path.to_string());
    }
    if let Some(children) = children {
        for child in children.iter() {
            if let Ok((child_part, _, _)) = parts.get(child) {
                let child_path = if let Some(ref name) = child_part.name {
                    if node_path.is_empty() { name.clone() } else { format!("{node_path}/{name}") }
                } else {
                    node_path.to_string()
                };
                collect_subtree_paths(child, parts, &child_path, out);
            }
        }
    }
}

// =====================================================================
// Selection — click in the viewport to select a part
// =====================================================================

/// Read a left-click in the central viewport and update `Selection.source_path`
/// to the part the user actually clicked. We test the ray against each part's
/// own mesh triangles (not just AABBs) — AABB picking would mis-resolve
/// overlapping fused meshes (e.g. an "edges" mesh whose perimeter-spanning
/// AABB covers the corner cells claimed by a separate "corners" part).
fn select_on_click(
    mut contexts: EguiContexts,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    parts: Query<(Entity, &ShapePart)>,
    is_part: Query<(), With<ShapePart>>,
    children_q: Query<&Children>,
    aabbs: Query<(&Aabb, &GlobalTransform)>,
    meshes: Query<(&Mesh3d, &GlobalTransform)>,
    mesh_assets: Res<Assets<Mesh>>,
    mut selection: ResMut<Selection>,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }

    let egui_wants = contexts.ctx_mut().map(|c| c.wants_pointer_input()).unwrap_or(false);
    if egui_wants { return; }

    let Ok(window) = windows.single() else { return; };
    let Some(cursor) = window.cursor_position() else { return; };
    let Ok((camera, cam_xf)) = cameras.single() else { return; };

    // `viewport_to_world` expects the cursor in window logical coords (it
    // subtracts the viewport's top-left internally). We just need to verify
    // the cursor is inside the visible viewport rect first.
    if let Some(vp_rect) = camera.logical_viewport_rect() {
        if !vp_rect.contains(cursor) { return; }
    }

    let Ok(ray) = camera.viewport_to_world(cam_xf, cursor) else { return; };
    let dir: Vec3 = ray.direction.into();

    let mut best: Option<(f32, String)> = None;
    for (entity, part) in &parts {
        // Skip the root pseudo-part — its path is empty and it has no own
        // geometry; clicking the whole-shape volume isn't a useful selection.
        if part.path.is_empty() { continue; }

        // Coarse AABB cull first — most parts won't overlap the ray at all.
        if let Some((mn, mx)) = own_world_aabb(entity, &children_q, &aabbs, &is_part) {
            if ray_aabb_intersect(ray.origin, dir, mn, mx).is_none() { continue; }
        } else {
            continue;
        }

        // Precise per-triangle test against the part's own meshes.
        let Ok(children) = children_q.get(entity) else { continue };
        for child in children.iter() {
            if is_part.get(child).is_ok() { continue; }
            let Ok((mesh3d, xf)) = meshes.get(child) else { continue };
            let Some(mesh) = mesh_assets.get(&mesh3d.0) else { continue };
            let Some(t) = ray_mesh_intersect(ray.origin, dir, mesh, xf) else { continue };
            if best.as_ref().is_none_or(|(t_best, _)| t < *t_best) {
                best = Some((t, part.path.clone()));
            }
        }
    }

    selection.source_path = best.map(|(_, p)| p);
}

/// Ray-vs-mesh test in world space using Möller–Trumbore against each
/// triangle. Returns the smallest non-negative `t` if the ray hits any
/// triangle. Triangles are transformed to world space using `xf` so `t`
/// is comparable across different parts with different transforms.
fn ray_mesh_intersect(
    origin: Vec3,
    dir: Vec3,
    mesh: &Mesh,
    xf: &GlobalTransform,
) -> Option<f32> {
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3()?;
    let indices = mesh.indices()?;

    let mut best: Option<f32> = None;
    let mut idx_iter = indices.iter();
    while let (Some(i0), Some(i1), Some(i2)) = (idx_iter.next(), idx_iter.next(), idx_iter.next()) {
        let v0 = xf.transform_point(Vec3::from_array(positions[i0]));
        let v1 = xf.transform_point(Vec3::from_array(positions[i1]));
        let v2 = xf.transform_point(Vec3::from_array(positions[i2]));
        if let Some(t) = ray_triangle_intersect(origin, dir, v0, v1, v2) {
            if best.is_none_or(|b| t < b) {
                best = Some(t);
            }
        }
    }
    best
}

/// Möller–Trumbore ray-triangle intersection. Returns `t` along the ray
/// (origin + t * dir) where it hits the triangle, or `None` if it misses
/// or hits behind the origin.
fn ray_triangle_intersect(
    origin: Vec3,
    dir: Vec3,
    v0: Vec3,
    v1: Vec3,
    v2: Vec3,
) -> Option<f32> {
    let edge1 = v1 - v0;
    let edge2 = v2 - v0;
    let h = dir.cross(edge2);
    let a = edge1.dot(h);
    if a.abs() < 1e-7 { return None; }
    let f = 1.0 / a;
    let s = origin - v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) { return None; }
    let q = s.cross(edge1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 { return None; }
    let t = f * edge2.dot(q);
    if t > 1e-6 { Some(t) } else { None }
}

/// Union the world AABBs of an entity's *direct* mesh children, excluding
/// any child that is itself a `ShapePart` (those represent nested named
/// sub-parts and own their own selection volume). Returns `None` if the
/// entity has no own geometry.
fn own_world_aabb(
    root: Entity,
    children_q: &Query<&Children>,
    aabbs: &Query<(&Aabb, &GlobalTransform)>,
    is_part: &Query<(), With<ShapePart>>,
) -> Option<(Vec3, Vec3)> {
    let children = children_q.get(root).ok()?;
    let mut result: Option<(Vec3, Vec3)> = None;
    for child in children.iter() {
        if is_part.get(child).is_ok() { continue; }
        let Ok((aabb, xf)) = aabbs.get(child) else { continue };
        let world = aabb_to_world(aabb, xf);
        result = Some(match result {
            Some((mn, mx)) => (mn.min(world.0), mx.max(world.1)),
            None => world,
        });
    }
    result
}

/// Transform a local-space `Aabb` by a `GlobalTransform` into a world-space
/// (min, max). The result is itself axis-aligned in world space — i.e. the
/// AABB of the transformed corners, not a tightly-rotated OBB.
fn aabb_to_world(aabb: &Aabb, xf: &GlobalTransform) -> (Vec3, Vec3) {
    let center = Vec3::from(aabb.center);
    let half = Vec3::from(aabb.half_extents);
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    for sx in [-1.0, 1.0] {
        for sy in [-1.0, 1.0] {
            for sz in [-1.0, 1.0] {
                let local = center + Vec3::new(sx * half.x, sy * half.y, sz * half.z);
                let world = xf.transform_point(local);
                mn = mn.min(world);
                mx = mx.max(world);
            }
        }
    }
    (mn, mx)
}

/// Slab-based ray-AABB intersection. Returns the nearest non-negative `t`
/// along the ray at which it enters the box, or `None` if it misses.
fn ray_aabb_intersect(origin: Vec3, dir: Vec3, mn: Vec3, mx: Vec3) -> Option<f32> {
    let inv = Vec3::new(
        if dir.x.abs() < 1e-8 { f32::INFINITY } else { 1.0 / dir.x },
        if dir.y.abs() < 1e-8 { f32::INFINITY } else { 1.0 / dir.y },
        if dir.z.abs() < 1e-8 { f32::INFINITY } else { 1.0 / dir.z },
    );
    let t0 = (mn - origin) * inv;
    let t1 = (mx - origin) * inv;
    let tmin = t0.min(t1);
    let tmax = t0.max(t1);
    let t_enter = tmin.x.max(tmin.y).max(tmin.z);
    let t_exit = tmax.x.min(tmax.y).min(tmax.z);
    if t_enter > t_exit || t_exit < 0.0 {
        None
    } else {
        Some(t_enter.max(0.0))
    }
}

/// Gizmo group for selection wireframe, with a small negative depth_bias
/// to prevent z-fighting on coplanar faces.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct SelectionFrontGizmos;

fn configure_selection_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (front_cfg, _) = store.config_mut::<SelectionFrontGizmos>();
    front_cfg.depth_bias = -0.001;
}

const CANONICAL_COLOR: Color = Color::srgb(1.0, 0.85, 0.2);   // bright yellow
const DERIVED_COLOR: Color = Color::srgb(0.65, 0.55, 0.15);   // dim yellow

/// Draw the actual mesh edges of the selected part. Canonical (placement
/// index 0) copies render brighter than symmetry-derived copies.
fn draw_selection_highlight(
    mut front: Gizmos<SelectionFrontGizmos>,
    selection: Res<Selection>,
    parts: Query<(Entity, &ShapePart)>,
    is_part: Query<(), With<ShapePart>>,
    children_q: Query<&Children>,
    meshes: Query<(&Mesh3d, &GlobalTransform, Option<&PlacementCopy>)>,
    mesh_assets: Res<Assets<Mesh>>,
) {
    let Some(ref selected_path) = selection.source_path else { return; };
    for (entity, part) in &parts {
        if part.path != *selected_path { continue; }
        let Ok(children) = children_q.get(entity) else { continue };
        for child in children.iter() {
            if is_part.get(child).is_ok() { continue; }
            let Ok((mesh3d, xf, copy)) = meshes.get(child) else { continue };
            let Some(mesh) = mesh_assets.get(&mesh3d.0) else { continue };
            let is_canonical = copy.is_none_or(|c| c.is_canonical);
            let color = if is_canonical { CANONICAL_COLOR } else { DERIVED_COLOR };
            draw_mesh_wireframe(&mut front, mesh, xf, color);
        }
    }
}

/// Stroke every triangle edge in `mesh` as gizmo line segments, transformed
/// to world space by `xf`. Generic over the gizmo config group so the same
/// helper drives both the front and behind passes.
fn draw_mesh_wireframe<C: GizmoConfigGroup>(
    gizmos: &mut Gizmos<C>,
    mesh: &Mesh,
    xf: &GlobalTransform,
    color: Color,
) {
    let Some(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()) else { return };
    let Some(indices) = mesh.indices() else { return };

    let mut idx_iter = indices.iter();
    while let (Some(i0), Some(i1), Some(i2)) = (idx_iter.next(), idx_iter.next(), idx_iter.next()) {
        let v0 = xf.transform_point(Vec3::from_array(positions[i0]));
        let v1 = xf.transform_point(Vec3::from_array(positions[i1]));
        let v2 = xf.transform_point(Vec3::from_array(positions[i2]));
        gizmos.line(v0, v1, color);
        gizmos.line(v1, v2, color);
        gizmos.line(v2, v0, color);
    }
}

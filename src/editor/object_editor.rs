use bevy::prelude::*;
use bevy::render::camera::{ClearColorConfig, Viewport};
use bevy::render::view::RenderLayers;
use bevy_egui::{EguiContexts, egui};
use std::path::PathBuf;

use crate::browser::{browser_ui, ActiveEditor};
use crate::registry::AssetRegistry;
use crate::shape::{
    animate_shapes, base_orientation_matrix, collect_occupancy, compile, despawn_shape,
    spawn_shape, CompiledShape, Facing, FusedMesh, Mirroring, Orientation, RawMesh,
    Rotation, ShapeAnimator, ShapePart, ShapeRoot,
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
            .init_resource::<CameraFitState>()
            .init_resource::<SceneStats>()
            .init_resource::<SceneBounds>()
            .init_resource::<OrbitState>()
            .init_resource::<ZoomLimits>()
            .init_resource::<CameraIntent>()
            .init_resource::<ViewportRect>()
            .init_resource::<SelectedPart>()
            .init_resource::<OrientationGridState>()
            .add_systems(Update, (
                // Phase 1: detect what needs to change
                (
                    handle_activation,
                    watch_shape_changes.run_if(is_object_active),
                    keyboard_input.run_if(is_object_active),
                ),
                // Phase 2: apply shape reload (depends on phase 1 setting needs_reload)
                reload_shape.run_if(is_object_active),
                // Phase 2b: orientation grid build/teardown — runs after reload
                // so a reload cleanly cancels any active grid preview.
                (
                    teardown_orientation_grid.run_if(is_object_active),
                    build_orientation_grid.run_if(is_object_active),
                ).chain(),
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
                animate_shapes.run_if(is_object_active),
                update_light.run_if(is_object_active),
                // UI must run before viewport tracking so egui's
                // available_rect reflects the panels for this frame.
                // The right-side browser panel is drawn by `browser_ui`
                // in a different plugin, so we explicitly order after it.
                part_tree_ui.run_if(is_object_active),
                (
                    track_viewport_rect.run_if(is_object_active),
                    sync_camera_viewport.run_if(is_object_active),
                    sync_zoom_to_viewport.run_if(is_object_active),
                    layout_orientation_cells.run_if(is_object_active),
                    draw_orientation_labels.run_if(is_object_active),
                ).chain().after(part_tree_ui).after(browser_ui),
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
    triangles: usize,
    draw_calls: usize,
    /// Number of cell-level collisions detected in the current shape.
    /// Zero is the clean state; non-zero means two or more primitives
    /// claim the same integer cell. In the editor this is informational;
    /// non-interactive tools treat it as a hard error.
    collisions: usize,
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

/// Marker for entities that belong to the orientation-grid preview.
/// Tagged on every mesh spawned by `build_orientation_grid` so
/// `teardown_orientation_grid` can wipe them cleanly and leave the
/// normal `ShapeRoot` tree alone.
#[derive(Component)]
struct OrientationGridEntity;

/// Per-cell camera marker. Carries the cell's grid index, its caption
/// text, and the orthographic `fit_scale` computed for its sub-viewport
/// at build time. `layout_orientation_cells` reads these each frame to
/// reposition and re-viewport cell cameras as the window resizes.
#[derive(Component)]
struct OrientationCell {
    index: usize,
    label: String,
    /// Maximum AABB extent of the (pre-orientation) flattened part.
    /// Used each frame to recompute the cell's orthographic scale
    /// from the current cell pixel dimensions.
    max_extent: f32,
}

/// The part the user has selected in the part tree. Drives the
/// "Show orientations" button target. Cleared on shape reload so a
/// stale entity can't be referenced.
#[derive(Resource, Default)]
struct SelectedPart {
    entity: Option<Entity>,
    /// Name path from shape root to the selected part (skipping unnamed
    /// ancestors). Used to walk the freshly compiled `CompiledShape`
    /// and find the subtree we want to preview.
    name_path: Vec<String>,
}

/// Active / pending state for the orientation preview.
/// `active` is the steady state while the grid is on screen;
/// `build_requested` / `teardown_requested` are one-frame flags set
/// by UI interactions and consumed by the build/teardown systems.
///
/// When `active`, the main orbit camera is disabled and the central
/// viewport is tiled by N cell cameras — one per unique orientation —
/// each rendering the part to its own sub-rectangle as if that
/// sub-rectangle were the whole viewport.
#[derive(Resource, Default)]
struct OrientationGridState {
    active: bool,
    build_requested: bool,
    teardown_requested: bool,
    /// Number of unique orientation cells. `layout_orientation_cells`
    /// derives `cols`/`rows` from this each frame so resizing the
    /// viewport never cares about a cached layout.
    cell_count: usize,
}

// =====================================================================
// Activation / deactivation
// =====================================================================

fn handle_activation(
    active: Res<ActiveEditor>,
    mut activation: ResMut<EditorActivation>,
    mut reload: ResMut<ShapeReloadState>,
    mut fit: ResMut<CameraFitState>,
    mut orbit: ResMut<OrbitState>,
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

    // Load the new shape, fit the camera, and reset the orbit to its
    // default angles. Resetting on every activation (not just the
    // initial spawn) means switching between shapes always presents
    // the new shape from the canonical default angle.
    if let ActiveEditor::Object { ref path } = current {
        activation.current_path = Some(path.clone());
        reload.needs_reload = true;
        fit.needs_fit = true;
        orbit.yaw = DEFAULT_YAW;
        orbit.pitch = DEFAULT_PITCH;
        orbit.target = Vec3::ZERO;
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
        // The light needs to reach every render layer we might use:
        // the default shape layer (0) and every orientation-cell layer.
        all_editor_layers(),
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
    mut selected: ResMut<SelectedPart>,
    mut grid: ResMut<OrientationGridState>,
    activation: Res<EditorActivation>,
    registry: Res<AssetRegistry>,
    existing: Query<Entity, With<ShapeRoot>>,
    grid_entities: Query<Entity, With<OrientationGridEntity>>,
) {
    if !reload.needs_reload { return; }
    reload.needs_reload = false;

    // Reload invalidates any previous entity references and cancels
    // the orientation preview — selection is cleared, grid is wiped.
    *selected = SelectedPart::default();
    if grid.active {
        for e in &grid_entities {
            commands.entity(e).despawn_recursive();
        }
    }
    *grid = OrientationGridState::default();

    let Some(path) = &activation.current_path else { return };

    let roots: Vec<Entity> = existing.iter().collect();
    despawn_shape(&mut commands, &roots);

    let Some(shape_file) = registry.get_shape_by_path(path) else {
        error!("Shape at '{}' not found in registry", path.display());
        return;
    };

    // Compute the cell-level occupancy index once per reload. This is the
    // single source of truth for scene AABB AND collision count.
    let occupancy = collect_occupancy(shape_file, &registry);

    if let Some(aabb) = occupancy.aabb() {
        let min = aabb.min();
        let max = aabb.max();
        bounds.scene_min = Vec3::new(min.0 as f32, min.1 as f32, min.2 as f32);
        bounds.scene_max = Vec3::new(max.0 as f32, max.1 as f32, max.2 as f32);
    }

    stats.collisions = occupancy.collision_count();
    if stats.collisions > 0 {
        warn!(
            "shape '{}' has {} cell-level collision(s)",
            path.display(),
            stats.collisions
        );
        for c in occupancy.collisions().iter().take(10) {
            warn!(
                "  collision at {:?}: '{}' vs '{}'",
                c.cell, c.first_path, c.second_path
            );
        }
        if occupancy.collisions().len() > 10 {
            warn!("  ... and {} more", occupancy.collisions().len() - 10);
        }
    }

    spawn_shape(&mut commands, &mut meshes, &mut materials, shape_file, &registry);
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
const FIT_BORDER: f32 = 1.1;
const ZOOM_MIN_PCT: f32 = 10.0;
const ZOOM_MAX_PCT: f32 = 200.0;

/// Runs on shape switch: computes fit scale and sets initial zoom to 100%.
fn on_model_loaded(
    mut fit: ResMut<CameraFitState>,
    mut camera: Query<&mut Projection, (With<OrbitCamera>, Without<OrientationCell>)>,
    mut limits: ResMut<ZoomLimits>,
    mesh_aabbs: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), (With<Mesh3d>, Without<OrientationCell>)>,
    viewport: Res<ViewportRect>,
    grid: Res<OrientationGridState>,
) {
    if !fit.needs_fit { return; }
    if grid.active { return; }
    if mesh_aabbs.is_empty() { return; }
    if !viewport.is_renderable() { return; }
    fit.needs_fit = false;

    let fit_scale = compute_fit_scale(&mesh_aabbs, viewport.logical_size);
    if fit_scale < 0.001 { return; }

    fit.fit_scale = fit_scale;
    update_zoom_limits(&mut limits, fit_scale);

    if let Ok(mut projection) = camera.get_single_mut() {
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
    viewport: Res<ViewportRect>,
) {
    if !stats.needs_update { return; }
    if mesh_handles.is_empty() { return; }
    stats.needs_update = false;

    if viewport.is_renderable() {
        let fit_scale = compute_fit_scale(&mesh_aabbs, viewport.logical_size);
        if fit_scale > 0.001 {
            bounds.fit_scale = fit_scale;
            update_zoom_limits(&mut limits, fit_scale);
        }
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

/// Compute the orthographic scale at which the AABB fills the viewport with ~5% border on
/// the constraining dimension. Uses fixed projection angles (yaw=45, pitch=45) for
/// deterministic results — the shape's own rotation is ignored; the fit is computed against
/// its AABB as if it were a single box.
fn compute_fit_scale<F: bevy::ecs::query::QueryFilter>(
    mesh_aabbs: &Query<(&GlobalTransform, &bevy::render::primitives::Aabb), F>,
    viewport_size: Vec2,
) -> f32 {
    if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 { return 0.0; }

    let (scene_min, scene_max) = compute_scene_aabb(mesh_aabbs);
    let scene_size = scene_max - scene_min;

    if scene_size.length() < 0.001 { return 0.0; }

    let max_extent = scene_size.x.max(scene_size.y).max(scene_size.z);
    let proj_width = max_extent * ZOOM_PROJ_WIDTH_RATIO;
    let proj_height = max_extent * ZOOM_PROJ_HEIGHT_RATIO;

    let scale_for_width = proj_width * FIT_BORDER / viewport_size.x;
    let scale_for_height = proj_height * FIT_BORDER / viewport_size.y;

    scale_for_width.max(scale_for_height)
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
    let Some(ctx) = contexts.try_ctx_mut() else { return };
    let Ok(window) = windows.get_single() else { return };

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
/// central area. When the visible area is degenerate (e.g. egui side
/// panels fill the whole window), hold the previous viewport rather than
/// setting a zero-size one, which wgpu would reject.
fn sync_camera_viewport(
    viewport: Res<ViewportRect>,
    grid: Res<OrientationGridState>,
    mut camera: Query<&mut Camera, (With<OrbitCamera>, Without<OrientationCell>)>,
) {
    if grid.active { return; }
    if !viewport.is_renderable() { return; }
    let Ok(mut cam) = camera.get_single_mut() else { return };
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

/// On viewport size changes (window resize, panel resize), recompute
/// `fit_scale` against the new viewport and update `ortho.scale` so that
/// the user's current zoom percentage is preserved — the object's apparent
/// size in the visible rect changes, but "100% is fit" remains true.
///
/// This keeps `fit.fit_scale` authoritative for the CURRENT viewport.
/// `on_model_loaded` handles the shape-switch case (needs_fit=true) and
/// sets zoom to 100%; this system handles the resize case (viewport
/// changed with the same shape loaded) and preserves zoom_pct.
fn sync_zoom_to_viewport(
    viewport: Res<ViewportRect>,
    grid: Res<OrientationGridState>,
    mut fit: ResMut<CameraFitState>,
    mut limits: ResMut<ZoomLimits>,
    mut camera: Query<&mut Projection, (With<OrbitCamera>, Without<OrientationCell>)>,
    mesh_aabbs: Query<(&GlobalTransform, &bevy::render::primitives::Aabb), (With<Mesh3d>, Without<OrientationCell>)>,
) {
    if grid.active { return; }
    if fit.needs_fit { return; }
    if !viewport.is_renderable() { return; }
    if mesh_aabbs.is_empty() { return; }
    if fit.fit_scale <= 0.0 { return; }

    let new_fit = compute_fit_scale(&mesh_aabbs, viewport.logical_size);
    if new_fit < 0.001 { return; }
    if (new_fit - fit.fit_scale).abs() < f32::EPSILON * fit.fit_scale.max(1.0) {
        return;
    }

    let old_fit = fit.fit_scale;
    fit.fit_scale = new_fit;
    update_zoom_limits(&mut limits, new_fit);

    if let Ok(mut projection) = camera.get_single_mut() {
        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            ortho.scale *= new_fit / old_fit;
        }
    }
}

fn update_zoom_limits(limits: &mut ZoomLimits, fit_scale: f32) {
    limits.min = fit_scale * 100.0 / ZOOM_MAX_PCT;  // 200% → scale = fit/2
    limits.max = fit_scale * 100.0 / ZOOM_MIN_PCT;   // 10% → scale = fit*10
}

fn compute_scene_aabb<F: bevy::ecs::query::QueryFilter>(
    mesh_aabbs: &Query<(&GlobalTransform, &bevy::render::primitives::Aabb), F>,
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
// Orientation grid preview — shows the selected part in every unique
// (Facing × Mirroring × Rotation) combination, laid out in a grid.
// =====================================================================

const ALL_FACINGS: [Facing; 6] = [
    Facing::Front, Facing::Back, Facing::Left, Facing::Right, Facing::Top, Facing::Bottom,
];
const ALL_MIRRORINGS: [Mirroring; 2] = [Mirroring::NoMirror, Mirroring::Mirror];
const ALL_ROTATIONS: [Rotation; 4] = [
    Rotation::NoRotation, Rotation::RotateClockwise, Rotation::RotateHalf, Rotation::RotateCounter,
];

fn facing_short(f: Facing) -> &'static str {
    match f {
        Facing::Front => "Fr", Facing::Back => "Bk",
        Facing::Left => "Lf", Facing::Right => "Rt",
        Facing::Top => "Tp", Facing::Bottom => "Bt",
    }
}
fn mirroring_short(m: Mirroring) -> &'static str {
    match m { Mirroring::NoMirror => "—", Mirroring::Mirror => "Mir" }
}
fn rotation_short(r: Rotation) -> &'static str {
    match r {
        Rotation::NoRotation => "0",
        Rotation::RotateClockwise => "CW",
        Rotation::RotateHalf => "180",
        Rotation::RotateCounter => "CCW",
    }
}

// Each cell mesh + camera gets a unique RenderLayers bit so cameras
// don't cross-render each other. Layer 0 is the normal shape; layer 1
// is render_export. Cells use layers 2..=MAX_CELL_LAYER.
const ORIENTATION_LAYER_BASE: usize = 2;
const MAX_ORIENTATION_CELLS: usize = 30;

/// Build a `RenderLayers` mask covering layer 0 (the normal shape) and
/// every layer a cell camera might use. Attached to the directional
/// light so it reaches all of them.
fn all_editor_layers() -> RenderLayers {
    let mut layers: Vec<usize> = vec![0];
    for i in 0..MAX_ORIENTATION_CELLS {
        layers.push(ORIENTATION_LAYER_BASE + i);
    }
    RenderLayers::from_layers(&layers)
}

fn teardown_orientation_grid(
    mut commands: Commands,
    mut grid: ResMut<OrientationGridState>,
    mut shapes: Query<&mut Visibility, With<ShapeRoot>>,
    mut fit: ResMut<CameraFitState>,
    mut main_camera: Query<&mut Camera, (With<OrbitCamera>, Without<OrientationCell>)>,
    grid_entities: Query<Entity, With<OrientationGridEntity>>,
) {
    if !grid.teardown_requested { return; }
    grid.teardown_requested = false;

    for e in &grid_entities {
        commands.entity(e).despawn_recursive();
    }
    // Re-show the normal shape and re-enable the main orbit camera.
    for mut vis in &mut shapes {
        *vis = Visibility::Inherited;
    }
    if let Ok(mut cam) = main_camera.get_single_mut() {
        cam.is_active = true;
    }
    grid.active = false;
    grid.cell_count = 0;
    fit.needs_fit = true;
}

fn build_orientation_grid(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut grid: ResMut<OrientationGridState>,
    mut shapes: Query<&mut Visibility, With<ShapeRoot>>,
    mut main_camera: Query<&mut Camera, (With<OrbitCamera>, Without<OrientationCell>)>,
    selected: Res<SelectedPart>,
    activation: Res<EditorActivation>,
    registry: Res<AssetRegistry>,
) {
    if !grid.build_requested { return; }
    grid.build_requested = false;

    let Some(path) = &activation.current_path else { return };
    let Some(spec) = registry.get_shape_by_path(path) else { return };

    let compiled = compile(spec, &registry);
    let Some(target) = find_compiled_by_path(&compiled, &selected.name_path) else {
        warn!("Orientation preview: selected part not found in compiled tree");
        return;
    };

    // Flatten ONLY the selected node's own meshes — not its children.
    // The user wants to see the part itself in each orientation, not
    // the full subtree.
    let mut flat = RawMesh::default();
    let self_tf = target.local_transform;
    for fused in &target.meshes {
        append_fused_with_colors(&mut flat, fused, &self_tf);
    }
    if flat.positions.is_empty() {
        warn!("Orientation preview: selected part has no geometry of its own");
        return;
    }
    let (centroid, extent) = aabb_centroid_and_extent(&flat);
    recenter_positions(&mut flat, centroid);

    let unique = unique_orientations(&flat);
    if unique.is_empty() { return; }

    let n = unique.len().min(MAX_ORIENTATION_CELLS);
    if unique.len() > MAX_ORIENTATION_CELLS {
        warn!(
            "Orientation preview: {} unique orientations, truncating to {}",
            unique.len(), MAX_ORIENTATION_CELLS
        );
    }

    for (i, (orient, mesh_rot)) in unique.into_iter().take(n).enumerate() {
        let layer = RenderLayers::layer(ORIENTATION_LAYER_BASE + i);

        let mesh_handle = meshes.add(mesh_rot.to_bevy_mesh());
        let material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            // Mirror orientations flip winding; render two-sided so the
            // cell doesn't show a hollow shell.
            cull_mode: None,
            ..default()
        });

        // The mesh sits at origin on its own render layer. The cell
        // camera (below) is the only one that sees it.
        commands.spawn((
            OrientationGridEntity,
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::default(),
            layer.clone(),
        ));

        let label = format!(
            "{} / {} / {}",
            facing_short(orient.facing()),
            mirroring_short(orient.mirroring()),
            rotation_short(orient.rotation()),
        );

        // Cell camera: own viewport + own render layer. Its transform
        // and viewport are set each frame by `layout_orientation_cells`;
        // here we seed with placeholders that will be overwritten.
        // `order = i as isize + 1` so cells render after the main
        // camera's layer (even though it's disabled, being explicit
        // keeps ordering deterministic). Only the first cell clears,
        // so later cells draw into the same framebuffer without
        // wiping previous cells.
        let clear = if i == 0 {
            ClearColorConfig::Custom(Color::srgb(0.05, 0.05, 0.08))
        } else {
            ClearColorConfig::None
        };
        commands.spawn((
            OrientationGridEntity,
            OrientationCell {
                index: i,
                label,
                max_extent: extent,
            },
            Camera3d::default(),
            Camera {
                order: (i as isize) + 1,
                clear_color: clear,
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scale: extent.max(1.0),
                ..OrthographicProjection::default_3d()
            }),
            Transform::default(),
            layer,
        ));
    }

    // Hide the normal shape and disable the main camera. Cell cameras
    // drive the central viewport entirely while the grid is active.
    for mut vis in &mut shapes {
        *vis = Visibility::Hidden;
    }
    if let Ok(mut cam) = main_camera.get_single_mut() {
        cam.is_active = false;
    }

    grid.active = true;
    grid.cell_count = n;
}

/// Per-frame layout: divide the central `ViewportRect` into a rows ×
/// cols grid, and for each `OrientationCell` camera, set its viewport
/// to its cell sub-rect, its transform to the current orbit pose
/// (looking at origin), and its orthographic `fit_scale` so the part
/// fills the cell as if the cell were the whole viewport.
fn layout_orientation_cells(
    viewport: Res<ViewportRect>,
    grid: Res<OrientationGridState>,
    orbit: Res<OrbitState>,
    mut cells: Query<(&mut OrientationCell, &mut Camera, &mut Projection, &mut Transform)>,
) {
    if !grid.active || grid.cell_count == 0 { return; }
    if !viewport.is_renderable() { return; }

    let n = grid.cell_count;
    let cols = (n as f32).sqrt().ceil().max(1.0) as usize;
    let rows = (n + cols - 1) / cols;

    let cell_w = viewport.physical_size.x / cols as u32;
    let cell_h = viewport.physical_size.y / rows as u32;
    if cell_w == 0 || cell_h == 0 { return; }

    // Recompute cell fit_scale once per frame from the cell pixel size.
    // The fit formula is the same as the main-viewport one: the part is
    // treated as a unit AABB projecting to width/height ratios at the
    // fixed (45°, 45°) projection angles.
    let cell_logical = Vec2::new(
        cell_w as f32 / viewport.physical_size.x as f32 * viewport.logical_size.x,
        cell_h as f32 / viewport.physical_size.y as f32 * viewport.logical_size.y,
    );
    let reserved_for_caption = 18.0_f32;
    let draw_logical_h = (cell_logical.y - reserved_for_caption).max(1.0);

    // The part's AABB extent is what we stored as `fit_scale` at build
    // time (in world units). Use it as the AABB max_extent.
    let (cam_position, _) = orbit_camera::compute_camera_pose(orbit.yaw, orbit.pitch, Vec3::ZERO);

    for (cell, mut camera, mut projection, mut transform) in &mut cells {
        let col = cell.index % cols;
        let row = cell.index / cols;
        let x = viewport.physical_min.x + col as u32 * cell_w;
        let y = viewport.physical_min.y + row as u32 * cell_h;

        let new_vp = Viewport {
            physical_position: UVec2::new(x, y),
            physical_size: UVec2::new(cell_w, cell_h),
            ..default()
        };
        let changed = match &camera.viewport {
            Some(v) => v.physical_position != new_vp.physical_position
                || v.physical_size != new_vp.physical_size,
            None => true,
        };
        if changed {
            camera.viewport = Some(new_vp);
        }

        // Orbit pose — all cells share the same camera angle, just
        // different viewports and different meshes.
        transform.translation = cam_position;
        transform.look_at(Vec3::ZERO, Vec3::Y);

        // Re-derive the orthographic fit from cell pixel dimensions
        // every frame so window/panel resizes rescale every cell to
        // keep the part framed with the same ~5% border policy as the
        // main viewport.
        let proj_width = cell.max_extent * ZOOM_PROJ_WIDTH_RATIO;
        let proj_height = cell.max_extent * ZOOM_PROJ_HEIGHT_RATIO;
        let scale_for_w = proj_width * FIT_BORDER / cell_logical.x;
        let scale_for_h = proj_height * FIT_BORDER / draw_logical_h;
        let new_scale = scale_for_w.max(scale_for_h);

        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            ortho.scale = new_scale;
        }
    }
}

/// Walk a `CompiledShape` tree by a name path. An empty path selects
/// the root. Each element must match a named child or compiled node
/// along the way.
fn find_compiled_by_path<'a>(root: &'a CompiledShape, path: &[String]) -> Option<&'a CompiledShape> {
    let mut cursor = root;
    for segment in path {
        let next = cursor.children.iter().find(|c| c.name.as_deref() == Some(segment.as_str()))?;
        cursor = next;
    }
    Some(cursor)
}

fn append_fused_with_colors(dst: &mut RawMesh, fused: &FusedMesh, tf: &Transform) {
    let mat = tf.compute_matrix();
    let base = dst.positions.len() as u32;
    for pos in &fused.mesh.positions {
        let p = mat.transform_point3(Vec3::from(*pos));
        dst.positions.push([p.x, p.y, p.z]);
    }
    for normal in &fused.mesh.normals {
        let n = mat.transform_vector3(Vec3::from(*normal)).normalize_or_zero();
        dst.normals.push([n.x, n.y, n.z]);
    }
    dst.uvs.extend_from_slice(&fused.mesh.uvs);
    dst.colors.extend_from_slice(&fused.mesh.colors);
    for idx in &fused.mesh.indices {
        dst.indices.push(base + idx);
    }
}

fn aabb_centroid_and_extent(mesh: &RawMesh) -> (Vec3, f32) {
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    for p in &mesh.positions {
        let v = Vec3::from(*p);
        mn = mn.min(v);
        mx = mx.max(v);
    }
    let size = mx - mn;
    let extent = size.x.max(size.y).max(size.z);
    ((mn + mx) * 0.5, extent)
}

fn recenter_positions(mesh: &mut RawMesh, centroid: Vec3) {
    for p in &mut mesh.positions {
        let v = Vec3::from(*p) - centroid;
        *p = [v.x, v.y, v.z];
    }
}

/// For each of the 48 orientation tuples, transform the centered mesh
/// and keep only visually distinct results. Dedup key is a sorted,
/// quantized vertex list (positions rounded to 1/1000 of a unit, plus
/// per-vertex RGBA rounded to bytes) — exact for cell-aligned geometry.
fn unique_orientations(flat: &RawMesh) -> Vec<(Orientation, RawMesh)> {
    use std::collections::HashSet;
    let mut seen: HashSet<Vec<[i32; 7]>> = HashSet::new();
    let mut out: Vec<(Orientation, RawMesh)> = Vec::new();

    for &f in &ALL_FACINGS {
        for &m in &ALL_MIRRORINGS {
            for &r in &ALL_ROTATIONS {
                let orient = Orientation(f, m, r);
                let mat = base_orientation_matrix(&orient);
                let rotated = transform_mesh(flat, mat);
                let key = canonical_key(&rotated);
                if seen.insert(key) {
                    out.push((orient, rotated));
                }
            }
        }
    }
    out
}

fn transform_mesh(src: &RawMesh, mat: Mat3) -> RawMesh {
    let mut out = RawMesh {
        positions: Vec::with_capacity(src.positions.len()),
        normals: Vec::with_capacity(src.normals.len()),
        uvs: src.uvs.clone(),
        colors: src.colors.clone(),
        indices: src.indices.clone(),
    };
    for p in &src.positions {
        let v = mat * Vec3::from(*p);
        out.positions.push([v.x, v.y, v.z]);
    }
    for n in &src.normals {
        // Orthogonal matrix: normals transform the same way as positions.
        let v = (mat * Vec3::from(*n)).normalize_or_zero();
        out.normals.push([v.x, v.y, v.z]);
    }
    // Negative-determinant matrix flips triangle winding — reverse
    // each triangle's index order so the front face still faces
    // outward. We render two-sided anyway, but keeping the winding
    // consistent makes the dedup key identical across equivalent
    // orientations that happen to produce the same vertex set with
    // different winding.
    if mat.determinant() < 0.0 {
        for tri in out.indices.chunks_exact_mut(3) {
            tri.swap(1, 2);
        }
    }
    out
}

fn canonical_key(mesh: &RawMesh) -> Vec<[i32; 7]> {
    let mut v: Vec<[i32; 7]> = Vec::with_capacity(mesh.positions.len());
    for (pos, color) in mesh.positions.iter().zip(mesh.colors.iter().chain(std::iter::repeat(&[1.0; 4]))) {
        v.push([
            (pos[0] * 1000.0).round() as i32,
            (pos[1] * 1000.0).round() as i32,
            (pos[2] * 1000.0).round() as i32,
            (color[0] * 255.0).round() as i32,
            (color[1] * 255.0).round() as i32,
            (color[2] * 255.0).round() as i32,
            (color[3] * 255.0).round() as i32,
        ]);
    }
    v.sort_unstable();
    v
}

/// Egui overlay: for each orientation cell, draw the caption at the
/// bottom-center of that cell's viewport rectangle. This is pure 2D —
/// no world projection needed, because the cell's viewport IS the
/// cell's screen-space rectangle.
fn draw_orientation_labels(
    mut contexts: EguiContexts,
    grid: Res<OrientationGridState>,
    cells: Query<(&OrientationCell, &Camera)>,
) {
    if !grid.active { return; }
    let Some(ctx) = contexts.try_ctx_mut() else { return };

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("orientation_labels"),
    ));
    let scale = ctx.pixels_per_point();

    for (cell, camera) in &cells {
        let Some(vp) = camera.viewport.as_ref() else { continue };
        // Convert the cell's physical viewport rectangle to egui logical
        // coords and anchor the caption at its bottom-center.
        let x = vp.physical_position.x as f32 / scale;
        let y = vp.physical_position.y as f32 / scale;
        let w = vp.physical_size.x as f32 / scale;
        let h = vp.physical_size.y as f32 / scale;
        let center_x = x + w * 0.5;
        let bottom_y = y + h - 2.0;
        painter.text(
            egui::pos2(center_x, bottom_y),
            egui::Align2::CENTER_BOTTOM,
            &cell.label,
            egui::FontId::monospace(12.0),
            egui::Color32::from_rgb(240, 240, 240),
        );
    }
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
    mut selected: ResMut<SelectedPart>,
    mut grid: ResMut<OrientationGridState>,
) {
    let Some(ctx) = contexts.try_ctx_mut() else { return };
    let mut toggles: Vec<(Entity, Visibility)> = Vec::new();
    let mut new_selection: Option<(Entity, Vec<String>)> = None;

    egui::SidePanel::left("part_tree").min_width(200.0).show(ctx, |ui| {
        camera_controls(ui, &mut orbit, &mut camera, &fit, &stats);
        ui.separator();

        orientation_controls(ui, &selected, &mut grid, &parts);
        ui.separator();

        animation_controls(ui, &roots, &mut animators);
        ui.heading("Part Tree");
        ui.separator();

        for root in &roots {
            draw_tree_node(
                ui, root, &parts, &mut toggles, &mut new_selection,
                selected.entity, 0, &[], &[],
            );
        }
    });

    if let Some((entity, name_path)) = new_selection {
        selected.entity = Some(entity);
        selected.name_path = name_path;
    }

    for (entity, vis) in toggles {
        commands.entity(entity).insert(vis);
    }
}

fn orientation_controls(
    ui: &mut egui::Ui,
    selected: &SelectedPart,
    grid: &mut OrientationGridState,
    parts: &Query<(&ShapePart, Option<&Children>, &Visibility)>,
) {
    ui.heading("Orientations");
    let selected_label = selected.entity
        .and_then(|e| parts.get(e).ok())
        .and_then(|(p, _, _)| p.name.clone())
        .unwrap_or_else(|| "(none)".to_string());
    ui.label(format!("Selected: {selected_label}"));

    ui.horizontal(|ui| {
        let has_selection = selected.entity.is_some();
        if grid.active {
            if ui.button("Hide orientations").clicked() {
                grid.teardown_requested = true;
            }
        } else {
            ui.add_enabled_ui(has_selection, |ui| {
                if ui.button("Show orientations").clicked() {
                    grid.build_requested = true;
                }
            });
        }
    });
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
    ui.label(format!(
        "Parts: {}  Tris: {}  Draws: {}  Collisions: {}",
        stats.parts, stats.triangles, stats.draw_calls, stats.collisions
    ));
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

#[allow(clippy::too_many_arguments)]
fn draw_tree_node(
    ui: &mut egui::Ui,
    entity: Entity,
    parts: &Query<(&ShapePart, Option<&Children>, &Visibility)>,
    toggles: &mut Vec<(Entity, Visibility)>,
    new_selection: &mut Option<(Entity, Vec<String>)>,
    selected_entity: Option<Entity>,
    depth: usize,
    ancestors: &[Entity],
    name_path: &[String],
) {
    let Ok((part, children, _vis)) = parts.get(entity) else { return };

    let state = compute_tri_state(entity, parts);
    let label = part.name.as_deref().unwrap_or("(unnamed)");
    let indent = "  ".repeat(depth);
    let icon = match state {
        TriState::Visible => "+",
        TriState::Hidden  => "-",
        TriState::Mixed   => "~",
    };
    let is_selected = selected_entity == Some(entity);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // Visibility toggle lives on the tri-state icon. Clicking the
        // row body selects the part without affecting visibility.
        if !indent.is_empty() {
            ui.label(indent);
        }
        if ui.selectable_label(false, icon).clicked() {
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
        if ui.selectable_label(is_selected, label).clicked() {
            // name_path holds the path to this node's PARENT; append
            // this node's own name (skipping the root, which is
            // represented by the empty path).
            let mut p = name_path.to_vec();
            if depth > 0 {
                if let Some(name) = &part.name {
                    p.push(name.clone());
                }
            }
            *new_selection = Some((entity, p));
        }
    });

    if let Some(children) = children {
        let mut path = ancestors.to_vec();
        path.push(entity);
        // The root's name IS the CompiledShape root itself, so we don't
        // include it in the walk path — `find_compiled_by_path` starts
        // at `compiled` and consumes one segment per descent into a
        // named child.
        let mut child_name_path = name_path.to_vec();
        if depth > 0 {
            if let Some(name) = &part.name {
                child_name_path.push(name.clone());
            }
        }
        for &child in children.iter() {
            if parts.get(child).is_ok() {
                draw_tree_node(
                    ui, child, parts, toggles, new_selection, selected_entity,
                    depth + 1, &path, &child_name_path,
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

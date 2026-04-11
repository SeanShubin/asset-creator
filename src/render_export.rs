use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::camera::RenderTarget;
use bevy::render::view::RenderLayers;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::path::{Path, PathBuf};

use crate::registry::AssetRegistry;
use crate::shape::spawn_shape_with_layers;

const EXPORT_RENDER_LAYER: usize = 1;
const RENDER_SIZE: u32 = 1024;
const ISO_DISTANCE: f32 = 15.0;
const DEFAULT_YAW: f32 = 45.0;
const DEFAULT_PITCH: f32 = 45.0;

// =====================================================================
// Plugin
// =====================================================================

#[derive(Component)]
struct ExportEntity;

pub struct RenderExportPlugin;

impl Plugin for RenderExportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderQueue>()
            .add_systems(Update, (
                queue_dirty_shapes,
                process_render_queue,
            ).chain());
    }
}

// =====================================================================
// Resources
// =====================================================================

#[derive(Resource, Default)]
struct RenderQueue {
    pending: Vec<RenderJob>,
    active: Option<ActiveRender>,
    last_generation: u64,
    initial_scan_done: bool,
}

struct RenderJob {
    shape_path: PathBuf,
    output_path: PathBuf,
}

struct ActiveRender {
    cleanup_entities: Vec<Entity>,
    screenshot_entity: Entity,
    frames_waited: u32,
}

// =====================================================================
// Render directory structure
// =====================================================================

fn shapes_dir() -> PathBuf { PathBuf::from("data/shapes") }
fn renders_dir() -> PathBuf { PathBuf::from("data/renders") }

fn shape_path_to_render_path(shape_path: &Path) -> Option<PathBuf> {
    let relative = shape_path.strip_prefix(shapes_dir()).ok()?;
    let mut render_path = renders_dir().join(relative);
    render_path.set_extension("png");
    Some(render_path)
}

fn needs_render(shape_path: &Path, render_path: &Path) -> bool {
    let shape_mtime = match std::fs::metadata(shape_path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let render_mtime = match std::fs::metadata(render_path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    shape_mtime > render_mtime
}

// =====================================================================
// Systems
// =====================================================================

fn queue_dirty_shapes(
    registry: Res<AssetRegistry>,
    mut queue: ResMut<RenderQueue>,
) {
    let current_gen = registry.shape_generation();

    if !queue.initial_scan_done {
        queue.initial_scan_done = true;
        let _ = std::fs::create_dir_all(renders_dir());

        for (_name, shape_path) in registry.shape_entries() {
            if let Some(render_path) = shape_path_to_render_path(&shape_path) {
                if needs_render(&shape_path, &render_path) {
                    queue.pending.push(RenderJob { shape_path, output_path: render_path });
                }
            }
        }

        clean_orphaned_renders(&registry);
        queue.last_generation = current_gen;
        return;
    }

    if current_gen == queue.last_generation { return; }
    queue.last_generation = current_gen;

    for (_name, shape_path) in registry.shape_entries() {
        if let Some(render_path) = shape_path_to_render_path(&shape_path) {
            if needs_render(&shape_path, &render_path) {
                if !queue.pending.iter().any(|j| j.shape_path == shape_path) {
                    queue.pending.push(RenderJob { shape_path, output_path: render_path });
                }
            }
        }
    }
}

fn clean_orphaned_renders(registry: &AssetRegistry) {
    let render_paths: std::collections::HashSet<PathBuf> = registry.shape_entries()
        .iter()
        .filter_map(|(_, path)| shape_path_to_render_path(path))
        .collect();

    if let Ok(entries) = walk_dir_recursive(&renders_dir()) {
        for png_path in entries {
            if png_path.extension().is_some_and(|e| e == "png") && !render_paths.contains(&png_path) {
                let _ = std::fs::remove_file(&png_path);
            }
        }
    }
}

fn walk_dir_recursive(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() { return Ok(files); }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_dir_recursive(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

/// Spawn shape + camera + light + screenshot all on the same frame.
/// The shape gets RenderLayers at creation time (no propagation needed).
fn process_render_queue(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut queue: ResMut<RenderQueue>,
    registry: Res<AssetRegistry>,
    entities: Query<Entity>,
) {
    // Wait for active render to complete
    if let Some(ref mut active) = queue.active {
        active.frames_waited += 1;

        let done = entities.get(active.screenshot_entity).is_err();
        let timed_out = active.frames_waited > 30;

        if done || timed_out {
            let cleanup = active.cleanup_entities.clone();
            queue.active = None;
            for entity in cleanup {
                if let Some(ec) = commands.get_entity(entity) {
                    ec.despawn_recursive();
                }
            }
        }
        return;
    }

    // Start the next job
    let Some(job) = queue.pending.pop() else { return };
    let Some(shape) = registry.get_shape_by_path(&job.shape_path) else { return };

    let fit_scale = compute_fit_from_shape(shape);
    let shape_center = shape.compute_aabb()
        .map(|b| { let c = b.center(); Vec3::new(c.0, c.1, c.2) })
        .unwrap_or(Vec3::ZERO);

    // Render target image
    let size = Extent3d { width: RENDER_SIZE, height: RENDER_SIZE, depth_or_array_layers: 1 };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("render_export"),
            size,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    image.resize(size);
    let image_handle = images.add(image);

    let (cam_pos, _) = compute_camera_pose(DEFAULT_YAW, DEFAULT_PITCH, shape_center);
    let export_layer = RenderLayers::layer(EXPORT_RENDER_LAYER);

    // Camera
    let camera = commands.spawn((
        ExportEntity,
        Camera3d::default(),
        Camera {
            target: RenderTarget::Image(image_handle.clone().into()),
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scale: fit_scale,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(cam_pos).looking_at(shape_center, Vec3::Y),
        export_layer.clone(),
    )).id();

    // Light
    let cam_rot = Quat::from_euler(EulerRot::YXZ, DEFAULT_YAW.to_radians(), -DEFAULT_PITCH.to_radians(), 0.0);
    let light_offset = Quat::from_euler(EulerRot::YXZ, 15.0_f32.to_radians(), -30.0_f32.to_radians(), 0.0);
    let light = commands.spawn((
        ExportEntity,
        DirectionalLight { illuminance: 6000.0, shadows_enabled: false, ..default() },
        Transform::from_rotation(cam_rot * light_offset),
        export_layer.clone(),
    )).id();

    // Shape — all entities get the export render layer at creation time
    let shape_root = spawn_shape_with_layers(
        &mut commands, &mut meshes, &mut materials, shape, &registry,
        Some(export_layer),
    );

    // Screenshot
    if let Some(parent) = job.output_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let screenshot_entity = commands.spawn(
        Screenshot::image(image_handle)
    ).observe(save_png_with_alpha(job.output_path)).id();

    queue.active = Some(ActiveRender {
        cleanup_entities: vec![camera, light, shape_root],
        screenshot_entity,
        frames_waited: 0,
    });
}

// =====================================================================
// Helpers
// =====================================================================

fn compute_camera_pose(yaw: f32, pitch: f32, target: Vec3) -> (Vec3, Quat) {
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw.to_radians(), -pitch.to_radians(), 0.0);
    let position = target + rotation * Vec3::new(0.0, 0.0, ISO_DISTANCE);
    (position, rotation)
}

fn save_png_with_alpha(path: PathBuf) -> impl FnMut(Trigger<ScreenshotCaptured>) {
    move |trigger| {
        let img = trigger.event().0.clone();
        match img.try_into_dynamic() {
            Ok(dyn_img) => {
                let rgba = dyn_img.to_rgba8();
                let has_content = rgba.pixels().any(|p| p[3] > 0);
                if !has_content {
                    warn!("Skipping blank render: {}", path.display());
                    return;
                }
                match rgba.save_with_format(&path, image::ImageFormat::Png) {
                    Ok(_) => info!("Rendered: {}", path.display()),
                    Err(e) => error!("Cannot save render {}: {e}", path.display()),
                }
            }
            Err(e) => error!("Cannot convert render image: {e}"),
        }
    }
}

fn compute_fit_from_shape(shape: &crate::shape::ShapeNode) -> f32 {
    let aabb = shape.compute_aabb();
    let Some(aabb) = aabb else { return 0.01 };

    let center = aabb.center();
    let shape_center = Vec3::new(center.0, center.1, center.2);
    let min = aabb.min();
    let max = aabb.max();

    let (cam_pos, _) = compute_camera_pose(DEFAULT_YAW, DEFAULT_PITCH, shape_center);
    let view = Transform::from_translation(cam_pos)
        .looking_at(shape_center, Vec3::Y)
        .compute_matrix()
        .inverse();

    let mut view_min = Vec3::splat(f32::MAX);
    let mut view_max = Vec3::splat(f32::MIN);

    for &x in &[min.0, max.0] {
        for &y in &[min.1, max.1] {
            for &z in &[min.2, max.2] {
                let view_pos = view.transform_point3(Vec3::new(x, y, z));
                view_min = view_min.min(view_pos);
                view_max = view_max.max(view_pos);
            }
        }
    }

    let proj_width = view_max.x - view_min.x;
    let proj_height = view_max.y - view_min.y;

    if proj_width < 0.001 && proj_height < 0.001 { return 0.01; }

    proj_width.max(proj_height) / RENDER_SIZE as f32
}

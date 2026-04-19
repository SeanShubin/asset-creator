use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages};
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::path::{Path, PathBuf};

use crate::editor::{compute_camera_pose, fit_for_aabb};
use crate::registry::{AssetRegistry, shape_name_from_path};
use crate::shape::{collect_occupancy, aabb_for_parts, spawn_shape_with_layers};

const EXPORT_RENDER_LAYER: usize = 1;
const RENDER_SIZE: u32 = 1024;
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
fn renders_dir() -> PathBuf { PathBuf::from("generated/renders") }

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
                if let Ok(mut ec) = commands.get_entity(entity) {
                    ec.despawn();
                }
            }
        }
        return;
    }

    // Start the next job
    let Some(job) = queue.pending.pop() else { return };
    let Some(shape) = registry.get_shape_by_path(&job.shape_path) else { return };

    let occupancy = collect_occupancy(shape, &registry);
    occupancy.warn_collisions(&format!("render export: '{}'", job.shape_path.display()));

    let image_handle = create_render_target(&mut images);
    let export_layer = RenderLayers::layer(EXPORT_RENDER_LAYER);
    let viewport_size = Vec2::new(RENDER_SIZE as f32, RENDER_SIZE as f32);
    let (fit_scale, shape_center) = match aabb_for_parts(shape, &registry).and_then(|aabb| {
        let min = aabb.min();
        let max = aabb.max();
        fit_for_aabb(
            Vec3::new(min.0 as f32, min.1 as f32, min.2 as f32),
            Vec3::new(max.0 as f32, max.1 as f32, max.2 as f32),
            viewport_size,
            DEFAULT_YAW, DEFAULT_PITCH,
            0.0,
        )
    }) {
        Some(r) => (r.scale, r.target),
        None => (0.01, Vec3::ZERO),
    };

    let camera = spawn_export_camera(&mut commands, &image_handle, fit_scale, shape_center, &export_layer);
    let light = spawn_export_light(&mut commands, &export_layer);
    let name = shape_name_from_path(&job.shape_path);
    let shape_root = spawn_shape_with_layers(
        &mut commands, &mut meshes, &mut materials, &name, shape, &registry,
        Some(export_layer), &[],
    );

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

fn create_render_target(images: &mut ResMut<Assets<Image>>) -> Handle<Image> {
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
    images.add(image)
}

fn spawn_export_camera(
    commands: &mut Commands,
    image_handle: &Handle<Image>,
    fit_scale: f32,
    shape_center: Vec3,
    layer: &RenderLayers,
) -> Entity {
    let (cam_pos, _) = compute_camera_pose(DEFAULT_YAW, DEFAULT_PITCH, shape_center);
    commands.spawn((
        ExportEntity,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(image_handle.clone().into()),
        Projection::Orthographic(OrthographicProjection {
            scale: fit_scale,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(cam_pos).looking_at(shape_center, Vec3::Y),
        layer.clone(),
    )).id()
}

fn spawn_export_light(commands: &mut Commands, layer: &RenderLayers) -> Entity {
    commands.spawn((
        ExportEntity,
        DirectionalLight { illuminance: 6000.0, shadows_enabled: false, ..default() },
        Transform::from_rotation(crate::editor::compute_light_rotation(DEFAULT_YAW, DEFAULT_PITCH)),
        layer.clone(),
    )).id()
}

fn save_png_with_alpha(path: PathBuf) -> impl FnMut(On<ScreenshotCaptured>) {
    move |on| {
        let img = on.event().image.clone();
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


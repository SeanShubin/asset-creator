use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::camera::RenderTarget;
use bevy::render::view::RenderLayers;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::path::{Path, PathBuf};

use crate::registry::AssetRegistry;
use crate::shape::spawn_shape;

const EXPORT_RENDER_LAYER: usize = 1;
const RENDER_SIZE: u32 = 1024;
const ISO_DISTANCE: f32 = 15.0;
const DEFAULT_YAW: f32 = 45.0;
const DEFAULT_PITCH: f32 = 45.0;
const FIT_BORDER: f32 = 1.1;
const ZOOM_PROJ_WIDTH_RATIO: f32 = 1.414214;
const ZOOM_PROJ_HEIGHT_RATIO: f32 = 1.707107;

// =====================================================================
// Plugin
// =====================================================================

#[derive(Component)]
struct ExportShape;

#[derive(Component)]
struct ExportEntity;

pub struct RenderExportPlugin;

impl Plugin for RenderExportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderQueue>()
            .add_systems(Update, (
                queue_dirty_shapes,
                process_render_queue,
                propagate_export_render_layers,
            ).chain());
    }
}

fn propagate_export_render_layers(
    mut commands: Commands,
    export_roots: Query<Entity, With<ExportShape>>,
    children_query: Query<&Children>,
    has_layer: Query<(), With<RenderLayers>>,
) {
    let layer = RenderLayers::layer(EXPORT_RENDER_LAYER);
    for root in &export_roots {
        propagate_layer_recursive(&mut commands, root, &children_query, &has_layer, &layer);
    }
}

fn propagate_layer_recursive(
    commands: &mut Commands,
    entity: Entity,
    children_query: &Query<&Children>,
    has_layer: &Query<(), With<RenderLayers>>,
    layer: &RenderLayers,
) {
    if let Ok(children) = children_query.get(entity) {
        for &child in children.iter() {
            if has_layer.get(child).is_err() {
                commands.entity(child).insert(layer.clone());
            }
            propagate_layer_recursive(commands, child, children_query, has_layer, layer);
        }
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
    screenshot_triggered: bool,
}

// =====================================================================
// Render directory structure
// =====================================================================

fn shapes_dir() -> PathBuf {
    PathBuf::from("data/shapes")
}

fn renders_dir() -> PathBuf {
    PathBuf::from("data/renders")
}

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
                    queue.pending.push(RenderJob {
                        shape_path,
                        output_path: render_path,
                    });
                }
            }
        }

        clean_orphaned_renders(&registry);
        queue.last_generation = current_gen;
        return;
    }

    if current_gen == queue.last_generation {
        return;
    }
    queue.last_generation = current_gen;

    for (_name, shape_path) in registry.shape_entries() {
        if let Some(render_path) = shape_path_to_render_path(&shape_path) {
            if needs_render(&shape_path, &render_path) {
                if !queue.pending.iter().any(|j| j.shape_path == shape_path) {
                    queue.pending.push(RenderJob {
                        shape_path,
                        output_path: render_path,
                    });
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
                info!("Removed orphaned render: {}", png_path.display());
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

fn process_render_queue(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut queue: ResMut<RenderQueue>,
    registry: Res<AssetRegistry>,
    entities: Query<Entity>,
) {
    if let Some(ref mut active) = queue.active {
        active.frames_waited += 1;

        if !active.screenshot_triggered && active.frames_waited >= 3 {
            active.screenshot_triggered = true;
        }

        let should_cleanup =
            (active.screenshot_triggered && entities.get(active.screenshot_entity).is_err())
            || active.frames_waited > 30;

        if should_cleanup {
            if active.frames_waited > 30 {
                warn!("Render export timed out, skipping");
            }
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

    let Some(job) = queue.pending.pop() else { return };

    let Some(shape) = registry.get_shape_by_path(&job.shape_path) else {
        warn!("Shape at '{}' not found for rendering", job.shape_path.display());
        return;
    };

    let fit_scale = compute_fit_from_shape(shape);

    let size = Extent3d {
        width: RENDER_SIZE,
        height: RENDER_SIZE,
        depth_or_array_layers: 1,
    };
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

    let shape_center = shape.compute_aabb()
        .map(|b| { let c = b.center(); Vec3::new(c.0, c.1, c.2) })
        .unwrap_or(Vec3::ZERO);
    let (cam_pos, _) = compute_camera_pose(DEFAULT_YAW, DEFAULT_PITCH, shape_center);

    let export_layer = RenderLayers::layer(EXPORT_RENDER_LAYER);

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

    let cam_rot = Quat::from_euler(
        EulerRot::YXZ,
        DEFAULT_YAW.to_radians(),
        -DEFAULT_PITCH.to_radians(),
        0.0,
    );
    let light_offset = Quat::from_euler(
        EulerRot::YXZ,
        15.0_f32.to_radians(),
        -30.0_f32.to_radians(),
        0.0,
    );
    let light = commands.spawn((
        ExportEntity,
        DirectionalLight {
            illuminance: 6000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(cam_rot * light_offset),
        export_layer.clone(),
    )).id();

    let shape_root = spawn_shape(&mut commands, &mut meshes, &mut materials, shape, &registry);
    commands.entity(shape_root).insert((export_layer, ExportShape));

    let output_path = job.output_path.clone();
    if let Some(parent) = output_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let screenshot_entity = commands.spawn(
        Screenshot::image(image_handle)
    ).observe(save_png_with_alpha(output_path)).id();

    queue.active = Some(ActiveRender {
        cleanup_entities: vec![camera, light, shape_root],
        screenshot_entity,
        frames_waited: 0,
        screenshot_triggered: false,
    });
}

fn compute_camera_pose(yaw: f32, pitch: f32, target: Vec3) -> (Vec3, Quat) {
    let pitch_rad = pitch.to_radians();
    let yaw_rad = yaw.to_radians();
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw_rad, -pitch_rad, 0.0);
    let position = target + rotation * Vec3::new(0.0, 0.0, ISO_DISTANCE);
    (position, rotation)
}

/// Like Bevy's save_to_disk but preserves the alpha channel.
fn save_png_with_alpha(path: PathBuf) -> impl FnMut(Trigger<ScreenshotCaptured>) {
    move |trigger| {
        let img = trigger.event().0.clone();
        match img.try_into_dynamic() {
            Ok(dyn_img) => {
                let rgba = dyn_img.to_rgba8();
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

    let size = aabb.size();
    let max_extent = size.0.max(size.1).max(size.2);
    if max_extent < 0.001 { return 0.01; }

    let proj_width = max_extent * ZOOM_PROJ_WIDTH_RATIO;
    let proj_height = max_extent * ZOOM_PROJ_HEIGHT_RATIO;
    let image_size = RENDER_SIZE as f32;

    let scale_for_width = proj_width * FIT_BORDER / image_size;
    let scale_for_height = proj_height * FIT_BORDER / image_size;

    scale_for_width.max(scale_for_height)
}

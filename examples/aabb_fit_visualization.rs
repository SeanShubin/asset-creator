//! Experiment 5: visualize the new fit_for_aabb math.
//!
//! Spawns three AABBs of distinctly different shapes (cube, tall thin,
//! long flat) into three viewports side by side. Each AABB is on its
//! own render layer and each viewport's camera sees only its layer.
//! Each orthographic camera is fit to its AABB using the new helper.
//!
//! Expected outcome: in each viewport, the AABB fills the visible rect
//! with ~5% margin on the constraining dimension. The non-constraining
//! dimension has at least that much margin. The AABB is centered.
//!
//! Console output prints the math (projected dims, scale, fill %) so
//! you can compare what the math predicts to what you actually see.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

const VIEWPORT_W: u32 = 320;
const VIEWPORT_H: u32 = 480;
const ISO_DISTANCE: f32 = 15.0;
const YAW: f32 = 45.0;
const PITCH: f32 = 45.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Exp 5: fit_for_aabb visualization".into(),
                resolution: bevy::window::WindowResolution::new(
                    VIEWPORT_W * 3,
                    VIEWPORT_H,
                ),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cases = [
        ("cube",     Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0),
            Color::srgb(0.8, 0.6, 0.2)),
        ("tall",     Vec3::new(-0.5, -3.0, -0.5), Vec3::new(0.5, 3.0, 0.5),
            Color::srgb(0.2, 0.8, 0.4)),
        ("flat",     Vec3::new(-3.0, -0.3, -1.0), Vec3::new(3.0, 0.3, 1.0),
            Color::srgb(0.4, 0.4, 0.9)),
    ];

    let viewport_size = Vec2::new(VIEWPORT_W as f32, VIEWPORT_H as f32);

    println!();
    println!("Each viewport is {VIEWPORT_W}×{VIEWPORT_H} physical pixels (assuming 100% DPI scale).");
    println!("Visible world dims = scale × viewport_pixels.");
    println!("Constraining dimension fills 1/(1+2*0.05) = 90.9% of viewport.");
    println!();
    println!("{:5}  {:>14}  {:>7}  {:>10}  {:>20}",
        "case", "projection W×H", "scale", "constrain", "fill % (W × H)");
    println!("{}", "-".repeat(72));

    for (i, (name, min, max, color)) in cases.iter().enumerate() {
        let layer = RenderLayers::layer(i + 1);
        let center = (*min + *max) * 0.5;
        let size = *max - *min;

        // The AABB itself, on its own render layer.
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::from_size(size))),
            MeshMaterial3d(materials.add(StandardMaterial::from_color(*color))),
            Transform::from_translation(center),
            layer.clone(),
        ));

        // A light per case, on the same layer.
        commands.spawn((
            DirectionalLight { illuminance: 6000.0, ..default() },
            Transform::from_xyz(2.0, 4.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
            layer.clone(),
        ));

        // Diagnostic math.
        let (proj_w, proj_h) = projected_extents(*min, *max, YAW, PITCH);
        let result = fit_for_aabb(*min, *max, viewport_size, YAW, PITCH, 0.05)
            .expect("non-degenerate aabb");
        let visible_w = viewport_size.x * result.scale;
        let visible_h = viewport_size.y * result.scale;
        let fill_w = proj_w / visible_w * 100.0;
        let fill_h = proj_h / visible_h * 100.0;
        let constrain = if fill_w > fill_h { "width" } else { "height" };
        println!("{name:5}  {:>6.2} × {:>4.2}  {:>7.4}  {:>10}  {:>6.1}% × {:>6.1}%",
            proj_w, proj_h, result.scale, constrain, fill_w, fill_h);

        // Camera fit to this AABB, restricted to this AABB's render layer.
        let (cam_pos, _) = compute_camera_pose(YAW, PITCH, result.target);
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: i as isize,
                viewport: Some(bevy::camera::Viewport {
                    physical_position: UVec2::new(VIEWPORT_W * i as u32, 0),
                    physical_size: UVec2::new(VIEWPORT_W, VIEWPORT_H),
                    ..default()
                }),
                clear_color: ClearColorConfig::Custom(
                    Color::srgb(0.05, 0.05 + i as f32 * 0.04, 0.10)
                ),
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scale: result.scale,
                ..OrthographicProjection::default_3d()
            }),
            Transform::from_translation(cam_pos).looking_at(result.target, Vec3::Y),
            layer,
        ));
    }
    println!();
    println!("Read the table: each viewport's constraining dim should fill ~91%");
    println!("(visible as a tight box-to-edge fit). The other dim has more margin.");
    println!();
}

/// Project an AABB's 8 corners through the view transform and return the
/// (width, height) of the screen-space bounding box. Mirrors fit_for_aabb's
/// internal computation for diagnostic reporting.
fn projected_extents(min: Vec3, max: Vec3, yaw_deg: f32, pitch_deg: f32) -> (f32, f32) {
    let target = (min + max) * 0.5;
    let (cam_pos, _) = compute_camera_pose(yaw_deg, pitch_deg, target);
    let view_inv = Transform::from_translation(cam_pos)
        .looking_at(target, Vec3::Y)
        .to_matrix()
        .inverse();
    let mut view_min = Vec3::splat(f32::MAX);
    let mut view_max = Vec3::splat(f32::MIN);
    for &x in &[min.x, max.x] {
        for &y in &[min.y, max.y] {
            for &z in &[min.z, max.z] {
                let view_pos = view_inv.transform_point3(Vec3::new(x, y, z));
                view_min = view_min.min(view_pos);
                view_max = view_max.max(view_pos);
            }
        }
    }
    (view_max.x - view_min.x, view_max.y - view_min.y)
}

// =====================================================================
// Inlined helpers (mirror src/editor/orbit_camera.rs)
// =====================================================================

struct FitResult {
    scale: f32,
    target: Vec3,
}

fn compute_camera_pose(yaw_deg: f32, pitch_deg: f32, target: Vec3) -> (Vec3, Quat) {
    let pitch_rad = pitch_deg.to_radians();
    let yaw_rad = yaw_deg.to_radians();
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw_rad, -pitch_rad, 0.0);
    let position = target + rotation * Vec3::new(0.0, 0.0, ISO_DISTANCE);
    (position, rotation)
}

fn fit_for_aabb(
    min: Vec3,
    max: Vec3,
    viewport_size: Vec2,
    yaw_deg: f32,
    pitch_deg: f32,
    border_pct: f32,
) -> Option<FitResult> {
    if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 { return None; }
    let size = max - min;
    if size.length() < 0.001 { return None; }

    let target = (min + max) * 0.5;

    let (cam_pos, _) = compute_camera_pose(yaw_deg, pitch_deg, target);
    let view_inv = Transform::from_translation(cam_pos)
        .looking_at(target, Vec3::Y)
        .to_matrix()
        .inverse();

    let mut view_min = Vec3::splat(f32::MAX);
    let mut view_max = Vec3::splat(f32::MIN);
    for &x in &[min.x, max.x] {
        for &y in &[min.y, max.y] {
            for &z in &[min.z, max.z] {
                let view_pos = view_inv.transform_point3(Vec3::new(x, y, z));
                view_min = view_min.min(view_pos);
                view_max = view_max.max(view_pos);
            }
        }
    }

    let proj_width = view_max.x - view_min.x;
    let proj_height = view_max.y - view_min.y;
    if proj_width < 0.001 && proj_height < 0.001 { return None; }

    let border_mult = 1.0 + border_pct * 2.0;
    let scale_for_width = proj_width * border_mult / viewport_size.x;
    let scale_for_height = proj_height * border_mult / viewport_size.y;

    Some(FitResult {
        scale: scale_for_width.max(scale_for_height),
        target,
    })
}

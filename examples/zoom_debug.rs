use bevy::prelude::*;
use bevy::math::EulerRot;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Zoom Debug".into(),
                resolution: bevy::window::WindowResolution::new(1100, 720),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, setup)
        .add_systems(Update, report_scale)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Camera at same position as the object editor default
    let yaw = 45.0_f32;
    let pitch = 35.264_f32;
    let distance = 15.0_f32;
    let yaw_rad = yaw.to_radians();
    let pitch_rad = pitch.to_radians();
    let rotation = Quat::from_euler(EulerRot::YXZ, -yaw_rad, -pitch_rad, 0.0);
    let position = rotation * Vec3::new(0.0, 0.0, distance);

    // Try a range of scales to find the one that makes a 2-unit box fill the viewport
    // We'll print the visible world-space extents for each scale
    let test_scale = 0.012; // the old default

    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scale: test_scale,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // A 2x2x2 box centered at origin
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(2.0, 2.0, 2.0))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(Color::srgb(0.5, 0.5, 0.6)))),
        Transform::default(),
    ));

    // Light
    commands.spawn((
        DirectionalLight { illuminance: 6000.0, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -60.0_f32.to_radians(), -50.0_f32.to_radians(), 0.0)),
    ));
}

fn report_scale(
    camera: Query<(&Projection, &Camera)>,
    mut reported: Local<bool>,
) {
    if *reported { return; }

    let Ok((proj, cam)) = camera.single() else { return };
    let viewport = cam.logical_viewport_size();

    if let Projection::Orthographic(ortho) = proj {
        let Some(vp) = viewport else { return };
        *reported = true;

        println!("\n=== Zoom Debug ===");
        println!("Viewport: {:.0} x {:.0} pixels", vp.x, vp.y);
        println!("Ortho scale: {}", ortho.scale);
        println!("Ortho area: {:?}", ortho.area);

        // The visible world height = (area.max.y - area.min.y) * scale
        let base_h = ortho.area.max.y - ortho.area.min.y;
        let base_w = ortho.area.max.x - ortho.area.min.x;
        let visible_h = base_h * ortho.scale;
        let visible_w = base_w * ortho.scale * (vp.x / vp.y);
        println!("Base area: {base_w} x {base_h}");
        println!("Visible world: {visible_w:.4} x {visible_h:.4}");
        println!();

        // What scale to make a 2-unit object fill the height with 10% border?
        let target = 2.0 * 1.1;
        let needed = target / base_h;
        println!("To fit 2.0 units in height: scale = {needed:.6}");

        // Account for panels: 280 left + 250 right = 530 pixels used
        let usable_w = vp.x - 280.0 - 250.0;
        let usable_aspect = usable_w / vp.y;
        println!("Usable viewport: {usable_w:.0} x {:.0}", vp.y);
        println!("Usable aspect: {usable_aspect:.3}");

        let needed_for_width = target / (base_w * usable_aspect);
        println!("To fit 2.0 units in usable width: scale = {needed_for_width:.6}");
        println!("Use: scale = max({needed:.6}, {needed_for_width:.6}) = {:.6}", needed.max(needed_for_width));

        // Also try: what does scale 0.012 show?
        println!();
        println!("At scale 0.012:");
        println!("  visible height = {:.4}", base_h * 0.012);
        println!("  visible width  = {:.4}", base_w * 0.012 * (vp.x / vp.y));
    }
}

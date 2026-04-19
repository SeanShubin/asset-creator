//! Experiment 4: placeholder Camera2d (order=-1) + Camera3d (order=0).
//!
//! Hypothesis: the placeholder Camera2d at order=-1 renders first
//! (clearing the screen), then the Camera3d at order=0 renders the
//! 3D scene on top. Egui draws via the placeholder camera's render
//! graph (since it owns `PrimaryEguiContext`) and overlays the panel
//! on top of everything.
//!
//! Expected outcome: red cube visible in the right portion of the
//! window, side panel from the placeholder on the left. Both the
//! 3D content and the egui overlay should be visible simultaneously.
//!
//! If the cube is missing, the placeholder is clobbering the 3D camera.
//! If the panel is missing, the placeholder isn't rendering egui.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Exp 4: placeholder + 3D camera (expect cube + panel)".into(),
                resolution: bevy::window::WindowResolution::new(900, 600),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_systems(Startup, setup)
        .add_systems(EguiPrimaryContextPass, panel_system)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Placeholder for egui context — renders FIRST (order = -1).
    commands.spawn((Camera2d, Camera { order: -1, ..default() }));

    // Editor 3D camera — renders ON TOP of the placeholder (default order = 0).
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(3.0, 3.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(Color::srgb(0.8, 0.2, 0.2)))),
    ));

    commands.spawn((
        DirectionalLight { illuminance: 6000.0, ..default() },
        Transform::from_xyz(2.0, 4.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn panel_system(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::SidePanel::left("test_panel").min_width(220.0).show(ctx, |ui| {
        ui.heading("Layering test");
        ui.label("This panel comes from the order=-1 Camera2d.");
        ui.separator();
        ui.label("The red cube to the right comes from the order=0 Camera3d.");
        ui.separator();
        ui.label("If both are visible, layering works.");
    });
}

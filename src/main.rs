mod editor;
mod noise;
mod surface;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use editor::SurfaceEditorPlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Asset Creator — Surface Editor".into(),
                    resolution: bevy::window::WindowResolution::new(1100.0, 720.0),
                    ..default()
                }),
                ..default()
            }),
            EguiPlugin,
            SurfaceEditorPlugin,
        ))
        .run();
}

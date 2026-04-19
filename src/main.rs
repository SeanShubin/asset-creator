mod editor;
mod logging;
mod registry;
mod render_export;
mod shape;
mod stress_test;
mod util;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use std::path::PathBuf;

use editor::CurrentShape;
use registry::AssetRegistry;

// bevy_egui 0.39 attaches the primary egui context to the first spawned
// camera and renders egui via that camera's render graph. Empirically,
// using the orbit Camera3d as the host produces a broken UI (panel
// mispositioned, 3D viewport blank). A dedicated Camera2d hosts egui;
// the orbit Camera3d (default order=0) renders the scene on top. The
// placeholder lives on a non-default render layer so 3D gizmos (default
// layer 0) aren't double-rendered as a viewport-center thumbnail.
const EGUI_HOST_LAYER: usize = 31;

fn main() {
    logging::init();

    if stress_test::is_stress_test() {
        let registry = registry::AssetRegistry::load_from_disk(std::path::Path::new("data"));
        stress_test::run(&registry);
        return;
    }

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Asset Creator".into(),
                    resolution: bevy::window::WindowResolution::new(1100, 720),
                    ..default()
                }),
                ..default()
            })
            .disable::<bevy::log::LogPlugin>(),
        EguiPlugin::default(),
        registry::RegistryPlugin::default(),
        shape::ShapePlugin,
        editor::ObjectEditorPlugin,
        render_export::RenderExportPlugin,
    ));

    let initial_path = resolve_initial_shape().or_else(|| {
        // No CLI shape specified — auto-load the first one in the registry
        // so the viewport isn't empty on startup.
        app.world()
            .resource::<AssetRegistry>()
            .shape_entries()
            .first()
            .map(|(_, path)| path.clone())
    });
    app.insert_resource(CurrentShape { path: initial_path });

    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((
            Camera2d,
            Camera { order: -1, ..default() },
            RenderLayers::layer(EGUI_HOST_LAYER),
        ));
    });

    info!("starting app");
    app.run();
    info!("clean exit");
}

/// Resolve the initial shape from CLI args: first positional argument wins.
fn resolve_initial_shape() -> Option<PathBuf> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-') && a.ends_with(".ron"))
        .map(PathBuf::from)
}

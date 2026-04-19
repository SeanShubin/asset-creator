mod browser;
mod editor;
mod logging;
mod noise;
mod registry;
mod render_export;
mod shape;
mod stress_test;
mod surface;
mod util;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

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
        browser::BrowserPlugin,
        editor::SurfaceEditorPlugin,
        editor::ObjectEditorPlugin,
        render_export::RenderExportPlugin,
    ));

    if let Some(editor) = browser::resolve_from_cli() {
        app.insert_resource(editor);
    }

    // bevy_egui 0.39 attaches the primary egui context to the first spawned
    // camera and renders egui via that camera's render graph. Editors spawn
    // their own cameras on activation, but the browser panel needs egui to
    // work before any editor is active. This placeholder owns the egui
    // context for the lifetime of the app. It runs at order=-1 so it draws
    // first (clearing the screen), and editor cameras (default order=0)
    // draw their content on top.
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((
            Camera2d,
            Camera { order: -1, ..default() },
        ));
    });

    info!("starting app");
    app.run();
    info!("clean exit");
}

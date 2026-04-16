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
                    resolution: bevy::window::WindowResolution::new(1100.0, 720.0),
                    ..default()
                }),
                ..default()
            })
            .disable::<bevy::log::LogPlugin>(),
        EguiPlugin,
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

    app.run();
}

mod editor;
mod logging;
mod registry;
mod render_export;
mod shape;
mod stress_test;
mod util;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, PrimaryEguiContext};
use std::path::PathBuf;

use editor::CurrentShape;
use registry::AssetRegistry;

// bevy_egui 0.39 attaches the primary egui context to the first camera it
// sees in `setup_primary_egui_context_system`. Empirically, using the
// orbit Camera3d as the host produces a broken UI (panel mispositioned,
// 3D viewport blank). We dedicate a Camera2d as the egui host, render
// nothing through it (it's on a non-default layer so 3D gizmos aren't
// double-rendered as a thumbnail), and let the orbit Camera3d render the
// scene at order=0. To make the host attachment deterministic regardless
// of plugin scheduling, we DISABLE bevy_egui's auto-create and explicitly
// insert `PrimaryEguiContext` on the placeholder at spawn time.
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

    // Disable bevy_egui's auto-create-primary-context. Otherwise its
    // `setup_primary_egui_context_system` (which runs in PreStartup with
    // no explicit ordering) might attach the primary context to whatever
    // camera it sees first — including the orbit Camera3d if scheduling
    // happens to put it ahead of our placeholder. Manual attachment below
    // is order-independent.
    app.world_mut()
        .resource_mut::<EguiGlobalSettings>()
        .auto_create_primary_context = false;

    // Spawn the placeholder Camera2d with `PrimaryEguiContext` already
    // attached. Order doesn't matter because we've disabled auto-create —
    // this is the only camera that ever gets the primary context.
    app.add_systems(PreStartup, |mut commands: Commands| {
        commands.spawn((
            Camera2d,
            Camera { order: -1, ..default() },
            RenderLayers::layer(EGUI_HOST_LAYER),
            PrimaryEguiContext,
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

mod browser;
mod editor;
mod noise;
mod registry;
mod shape;
mod surface;
mod util;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

fn main() {
    install_panic_hook();

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Asset Creator".into(),
                resolution: bevy::window::WindowResolution::new(1100.0, 720.0),
                ..default()
            }),
            ..default()
        }),
        EguiPlugin,
        registry::RegistryPlugin::default(),
        browser::BrowserPlugin,
        editor::SurfaceEditorPlugin,
        editor::ObjectEditorPlugin,
    ));

    if let Some(editor) = browser::resolve_from_cli() {
        app.insert_resource(editor);
    }

    app.run();
    write_exit_log("CLEAN EXIT");
}

// =====================================================================
// Crash logging
// =====================================================================

const EXIT_LOG: &str = "crash.log";

fn install_panic_hook() {
    write_exit_log("STARTED");
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let report = format!("PANIC at {}\n\n{info}\n\n{backtrace}", timestamp());
        let _ = std::fs::write(EXIT_LOG, &report);
        eprintln!("{report}");
    }));
}

fn write_exit_log(status: &str) {
    let _ = std::fs::write(EXIT_LOG, format!("{status} at {}", timestamp()));
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    let s = secs % 60;
    format!("{hours:02}:{mins:02}:{s:02} UTC")
}

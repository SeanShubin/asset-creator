mod browser;
mod editor;
mod noise;
mod registry;
mod shape;
mod surface;
mod util;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use std::path::PathBuf;

fn main() {
    install_panic_hook();
    let initial_editor = resolve_initial_editor();

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

    if let Some(editor) = initial_editor {
        app.insert_resource(editor);
    }

    app.run();
    write_exit_log("CLEAN EXIT");
}

// =====================================================================
// CLI — optional shortcut to jump directly into an editor
// =====================================================================

fn resolve_initial_editor() -> Option<browser::ActiveEditor> {
    let args: Vec<String> = std::env::args().collect();
    let subcommand = args.get(1).map(|s| s.as_str());

    match subcommand {
        Some("surface") => Some(resolve_surface_editor(&args[2..])),
        Some("object") => Some(resolve_object_editor(&args[2..])),
        Some(path) if !path.starts_with('-') && path.ends_with(".ron") => {
            if path.contains("shape") {
                Some(browser::ActiveEditor::Object { path: PathBuf::from(path) })
            } else {
                Some(resolve_surface_editor(&args[1..]))
            }
        }
        _ => None,
    }
}

fn resolve_surface_editor(args: &[String]) -> browser::ActiveEditor {
    if let Some(pos) = args.iter().position(|a| a == "--preset") {
        if let Some(name) = args.get(pos + 1) {
            return browser::ActiveEditor::Surface { name: name.clone() };
        }
    }
    if let Some(path_str) = args.iter().find(|a| !a.starts_with('-')) {
        // Load the file to get the surface name
        if let Ok(surface) = surface::load_surface_from_file(std::path::Path::new(path_str.as_str())) {
            return browser::ActiveEditor::Surface { name: surface.name };
        }
    }
    browser::ActiveEditor::Surface { name: "unnamed".into() }
}

fn resolve_object_editor(args: &[String]) -> browser::ActiveEditor {
    let path_str = args.iter().find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("data/shapes/scout_bot.shape.ron");
    browser::ActiveEditor::Object { path: PathBuf::from(path_str) }
}

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

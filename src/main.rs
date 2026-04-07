mod editor;
mod noise;
mod surface;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use editor::SurfaceEditorPlugin;
use surface::SurfaceDef;

fn main() {
    let initial_surface = resolve_surface_from_args();

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
            SurfaceEditorPlugin { initial_surface },
        ))
        .run();
}

fn resolve_surface_from_args() -> SurfaceDef {
    let args: Vec<String> = std::env::args().collect();

    if let Some(surface) = find_preset_arg(&args) {
        return surface;
    }

    if let Some(surface) = find_file_arg(&args) {
        return surface;
    }

    SurfaceDef::default()
}

fn find_preset_arg(args: &[String]) -> Option<SurfaceDef> {
    let pos = args.iter().position(|a| a == "--preset")?;
    let name = args.get(pos + 1)?;
    let surface = surface::preset_by_name(name).unwrap_or_else(|| {
        let names = surface::preset_names();
        eprintln!("Unknown preset '{}'. Available: {:?}", name, names);
        std::process::exit(1);
    });
    Some(surface)
}

fn find_file_arg(args: &[String]) -> Option<SurfaceDef> {
    let path_str = args.iter().skip(1).find(|a| !a.starts_with('-'))?;
    let path = std::path::Path::new(path_str);
    let surface = surface::load_surface_from_file(path).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });
    Some(surface)
}

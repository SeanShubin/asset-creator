use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use std::path::PathBuf;

use crate::registry::{AssetRegistry, DeleteSurface, SaveSurface};

// =====================================================================
// Active editor state
// =====================================================================

#[derive(Resource, Clone, Debug, PartialEq)]
pub enum ActiveEditor {
    None,
    Surface { name: String },
    Object { path: PathBuf },
}

impl Default for ActiveEditor {
    fn default() -> Self {
        Self::None
    }
}

// =====================================================================
// CLI resolution
// =====================================================================

/// Resolve the initial editor from CLI arguments.
pub fn resolve_from_cli() -> Option<ActiveEditor> {
    let args: Vec<String> = std::env::args().collect();
    let subcommand = args.get(1).map(|s| s.as_str());

    match subcommand {
        Some("surface") => Some(resolve_surface_args(&args[2..])),
        Some("object") => Some(resolve_object_args(&args[2..])),
        Some(path) if !path.starts_with('-') && path.ends_with(".ron") => {
            if path.contains("shape") {
                Some(ActiveEditor::Object { path: PathBuf::from(path) })
            } else {
                Some(resolve_surface_args(&args[1..]))
            }
        }
        _ => None,
    }
}

fn resolve_surface_args(args: &[String]) -> ActiveEditor {
    if let Some(pos) = args.iter().position(|a| a == "--preset") {
        if let Some(name) = args.get(pos + 1) {
            return ActiveEditor::Surface { name: name.clone() };
        }
    }
    if let Some(path_str) = args.iter().find(|a| !a.starts_with('-')) {
        if let Ok(surface) = crate::surface::load_surface_from_file(std::path::Path::new(path_str.as_str())) {
            return ActiveEditor::Surface { name: surface.name };
        }
    }
    ActiveEditor::Surface { name: "unnamed".into() }
}

fn resolve_object_args(args: &[String]) -> ActiveEditor {
    let path_str = args.iter().find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("data/shapes/scout_bot.shape.ron");
    ActiveEditor::Object { path: PathBuf::from(path_str) }
}

// =====================================================================
// Plugin
// =====================================================================

pub struct BrowserPlugin;

impl Plugin for BrowserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveEditor>()
            .add_systems(Update, browser_ui);
    }
}

// =====================================================================
// Browser UI
// =====================================================================

fn browser_ui(
    mut contexts: EguiContexts,
    registry: Res<AssetRegistry>,
    mut active: ResMut<ActiveEditor>,
    mut save_events: EventWriter<SaveSurface>,
    mut delete_events: EventWriter<DeleteSurface>,
) {
    let ctx = contexts.ctx_mut();

    egui::SidePanel::right("asset_browser").min_width(180.0).max_width(250.0).show(ctx, |ui| {
        ui.heading("Assets");
        ui.separator();

        surface_list(ui, &registry, &mut active, &mut save_events, &mut delete_events);
        ui.separator();
        shape_list(ui, &registry, &mut active);

        if registry.has_errors() {
            ui.separator();
            error_list(ui, &registry);
        }
    });
}

// =====================================================================
// Surface list
// =====================================================================

fn surface_list(
    ui: &mut egui::Ui,
    registry: &AssetRegistry,
    active: &mut ActiveEditor,
    save_events: &mut EventWriter<SaveSurface>,
    delete_events: &mut EventWriter<DeleteSurface>,
) {
    ui.label("Surfaces");

    let names = registry.surface_names();

    let mut to_delete: Option<String> = None;

    for name in &names {
        ui.horizontal(|ui| {
            let is_active = matches!(&*active, ActiveEditor::Surface { name: n } if n == name);
            if ui.selectable_label(is_active, name.as_str()).clicked() {
                *active = ActiveEditor::Surface { name: name.clone() };
            }
            if ui.small_button("x").clicked() {
                to_delete = Some(name.clone());
            }
        });
    }

    if let Some(ref name) = to_delete {
        delete_events.send(DeleteSurface { name: name.clone() });
        if matches!(&*active, ActiveEditor::Surface { name: n } if n == name) {
            *active = ActiveEditor::None;
        }
    }

    if ui.button("+ New Surface").clicked() {
        let name_refs: Vec<&String> = names.iter().collect();
        let new_name = generate_unique_name("surface", &name_refs);
        let mut surface = crate::surface::SurfaceDef::default();
        surface.name = new_name.clone();

        save_events.send(SaveSurface { name: new_name.clone(), data: surface });
        *active = ActiveEditor::Surface { name: new_name };
    }
}

// =====================================================================
// Shape list
// =====================================================================

fn shape_list(
    ui: &mut egui::Ui,
    registry: &AssetRegistry,
    active: &mut ActiveEditor,
) {
    ui.label("Shapes");

    let entries = registry.shape_entries();

    for (key, path) in &entries {
        let stem = key.strip_suffix(".shape.ron").unwrap_or(key);
        let is_active = matches!(&*active, ActiveEditor::Object { path: p } if *p == *path);
        if ui.selectable_label(is_active, stem).clicked() {
            *active = ActiveEditor::Object { path: path.clone() };
        }
    }
}

// =====================================================================
// Error display
// =====================================================================

fn error_list(ui: &mut egui::Ui, registry: &AssetRegistry) {
    ui.colored_label(egui::Color32::RED, "Errors");
    for error in registry.errors() {
        let filename = std::path::Path::new(&error.path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&error.path);
        ui.colored_label(egui::Color32::YELLOW, filename);
        ui.label(&error.message);
        ui.add_space(4.0);
    }
}

// =====================================================================
// Helpers
// =====================================================================

fn generate_unique_name(prefix: &str, existing: &[&String]) -> String {
    for i in 1.. {
        let candidate = if i == 1 {
            format!("new_{prefix}")
        } else {
            format!("new_{prefix}_{i}")
        };
        if !existing.iter().any(|n| **n == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

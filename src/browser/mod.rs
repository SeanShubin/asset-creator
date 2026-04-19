use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use std::path::PathBuf;

use crate::registry::AssetRegistry;

// =====================================================================
// Active editor state
// =====================================================================

#[derive(Resource, Clone, Debug, PartialEq)]
pub enum ActiveEditor {
    None,
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
        Some("object") => Some(resolve_object_args(&args[2..])),
        Some(path) if !path.starts_with('-') && path.ends_with(".ron") => {
            Some(ActiveEditor::Object { path: PathBuf::from(path) })
        }
        _ => None,
    }
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
            .add_systems(EguiPrimaryContextPass, browser_ui);
    }
}

// =====================================================================
// Browser UI
// =====================================================================

pub(crate) fn browser_ui(
    mut contexts: EguiContexts,
    registry: Res<AssetRegistry>,
    mut active: ResMut<ActiveEditor>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    egui::SidePanel::right("asset_browser").min_width(180.0).max_width(250.0).show(ctx, |ui| {
        ui.heading("Assets");
        ui.separator();

        shape_list(ui, &registry, &mut active);

        if registry.has_errors() {
            ui.separator();
            error_list(ui, &registry);
        }
    });
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

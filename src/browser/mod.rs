use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use std::path::PathBuf;

use crate::registry::AssetRegistry;

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
    mut registry: ResMut<AssetRegistry>,
    mut active: ResMut<ActiveEditor>,
) {
    let ctx = contexts.ctx_mut();

    egui::SidePanel::right("asset_browser").min_width(180.0).max_width(250.0).show(ctx, |ui| {
        ui.heading("Assets");
        ui.separator();

        surface_list(ui, &mut registry, &mut active);
        ui.separator();
        shape_list(ui, &mut active);

        if !registry.errors.is_empty() {
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
    registry: &mut AssetRegistry,
    active: &mut ActiveEditor,
) {
    ui.label("Surfaces");

    let mut names: Vec<String> = registry.surfaces.keys().cloned().collect();
    names.sort();

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

    if let Some(name) = to_delete {
        delete_surface(registry, active, &name);
    }

    if ui.button("+ New Surface").clicked() {
        let name_refs: Vec<&String> = names.iter().collect();
        let new_name = generate_unique_name("surface", &name_refs);
        let mut surface = crate::surface::SurfaceDef::default();
        surface.name = new_name.clone();

        let path = PathBuf::from("data/surfaces")
            .join(format!("{}.surface.ron", new_name));
        crate::registry::store::save_surface_to_file(&surface, &path);

        *active = ActiveEditor::Surface { name: new_name };
    }
}

fn delete_surface(registry: &mut AssetRegistry, active: &mut ActiveEditor, name: &str) {
    if let Some(registered) = registry.surfaces.remove(name) {
        if let Err(e) = std::fs::remove_file(&registered.path) {
            warn!("Failed to delete '{}': {}", registered.path.display(), e);
        }
    }

    // If we just deleted the active surface, deselect
    if matches!(&*active, ActiveEditor::Surface { name: n } if n == name) {
        *active = ActiveEditor::None;
    }
}

// =====================================================================
// Shape list
// =====================================================================

fn shape_list(
    ui: &mut egui::Ui,
    active: &mut ActiveEditor,
) {
    ui.label("Shapes");

    let shapes_dir = PathBuf::from("data/shapes");
    if let Ok(entries) = std::fs::read_dir(&shapes_dir) {
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "ron"))
            .collect();
        paths.sort();

        for path in &paths {
            let stem = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let is_active = matches!(&*active, ActiveEditor::Object { path: p } if *p == *path);
            if ui.selectable_label(is_active, stem).clicked() {
                *active = ActiveEditor::Object { path: path.clone() };
            }
        }
    }
}

// =====================================================================
// Error display
// =====================================================================

fn error_list(ui: &mut egui::Ui, registry: &AssetRegistry) {
    ui.colored_label(egui::Color32::RED, "Errors");
    for error in &registry.errors {
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

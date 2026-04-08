use bevy::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::shape::ShapeNode;
use crate::surface::SurfaceDef;
use super::watcher::FileWatcher;

// =====================================================================
// Registry types
// =====================================================================

#[derive(Clone, Debug)]
pub struct RegisteredAsset<T> {
    pub data: T,
    pub path: PathBuf,
    pub last_modified: SystemTime,
}

#[derive(Resource, Default)]
pub struct AssetRegistry {
    pub surfaces: HashMap<String, RegisteredAsset<SurfaceDef>>,
    pub shapes: HashMap<String, RegisteredAsset<ShapeNode>>,
    pub generation: u64,
    pub shape_generation: u64,
    pub errors: Vec<AssetError>,
}

#[derive(Clone, Debug)]
pub struct AssetError {
    pub path: String,
    pub message: String,
}

impl AssetRegistry {
    pub fn get_surface(&self, name: &str) -> Option<&SurfaceDef> {
        self.surfaces.get(name).map(|r| &r.data)
    }

    pub fn get_shape(&self, name: &str) -> Option<&ShapeNode> {
        self.shapes.get(name).map(|r| &r.data)
    }

    pub fn clear_error_for(&mut self, path: &str) {
        self.errors.retain(|e| e.path != path);
    }

    pub fn set_error(&mut self, path: String, message: String) {
        self.clear_error_for(&path);
        self.errors.push(AssetError { path, message });
    }
}

// =====================================================================
// Plugin
// =====================================================================

pub struct RegistryPlugin {
    pub data_dir: PathBuf,
}

impl Default for RegistryPlugin {
    fn default() -> Self {
        Self { data_dir: PathBuf::from("data") }
    }
}

impl Plugin for RegistryPlugin {
    fn build(&self, app: &mut App) {
        let mut registry = AssetRegistry::default();
        let data_dir = self.data_dir.clone();

        load_all_surfaces(&data_dir, &mut registry);
        load_all_shapes(&data_dir, &mut registry);
        info!("Registry loaded {} surfaces, {} shapes from '{}'",
            registry.surfaces.len(), registry.shapes.len(), data_dir.display());

        app.insert_resource(registry)
            .insert_resource(FileWatcher::new(data_dir))
            .add_systems(Update, poll_file_changes);
    }
}

// =====================================================================
// Initial loading
// =====================================================================

fn load_all_surfaces(data_dir: &Path, registry: &mut AssetRegistry) {
    load_ron_files(&data_dir.join("surfaces"), registry, is_surface_file, load_surface_into_registry);
}

fn load_all_shapes(data_dir: &Path, registry: &mut AssetRegistry) {
    load_ron_files(&data_dir.join("shapes"), registry, is_shape_file, load_shape_into_registry);
}

fn load_ron_files(
    dir: &Path,
    registry: &mut AssetRegistry,
    filter: fn(&Path) -> bool,
    loader: fn(&Path, &mut AssetRegistry),
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if filter(&path) {
            loader(&path, registry);
        }
    }
}

// =====================================================================
// Surface loading
// =====================================================================

fn load_surface_into_registry(path: &Path, registry: &mut AssetRegistry) {
    let path_str = path.display().to_string();

    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            registry.set_error(path_str, format!("Read error: {e}"));
            return;
        }
    };

    let options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    let surface: SurfaceDef = match options.from_str(&contents) {
        Ok(s) => s,
        Err(e) => {
            registry.set_error(path_str, format!("{e}"));
            return;
        }
    };

    registry.clear_error_for(&path_str);

    let last_modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let name = surface.name.clone();
    registry.surfaces.insert(name, RegisteredAsset {
        data: surface,
        path: path.to_path_buf(),
        last_modified,
    });
}

// =====================================================================
// Shape loading
// =====================================================================

fn load_shape_into_registry(path: &Path, registry: &mut AssetRegistry) {
    let path_str = path.display().to_string();

    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            registry.set_error(path_str, format!("Read error: {e}"));
            return;
        }
    };

    let options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    let shape: ShapeNode = match options.from_str(&contents) {
        Ok(s) => s,
        Err(e) => {
            registry.set_error(path_str, format!("{e}"));
            return;
        }
    };

    registry.clear_error_for(&path_str);

    let last_modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let name = shape.name.clone().unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    registry.shapes.insert(name, RegisteredAsset {
        data: shape,
        path: path.to_path_buf(),
        last_modified,
    });
}

pub fn save_surface_to_file(surface: &SurfaceDef, path: &Path) {
    let config = ron::ser::PrettyConfig::default();
    let ron_str = match ron::ser::to_string_pretty(surface, config) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to serialize surface: {}", e);
            return;
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    if let Err(e) = std::fs::write(path, &ron_str) {
        error!("Failed to write '{}': {}", path.display(), e);
    }
}

// =====================================================================
// File change polling
// =====================================================================

fn poll_file_changes(
    mut registry: ResMut<AssetRegistry>,
    mut watcher: ResMut<FileWatcher>,
    time: Res<Time>,
) {
    if !watcher.should_poll(time.elapsed_secs_f64()) {
        return;
    }

    let changed_paths = watcher.detect_changes();
    if changed_paths.is_empty() {
        return;
    }

    let mut surface_changed = false;
    let mut shape_changed = false;

    for path in &changed_paths {
        if is_surface_file(path) {
            load_surface_into_registry(path, &mut registry);
            surface_changed = true;
        }
        if is_shape_file(path) {
            load_shape_into_registry(path, &mut registry);
            shape_changed = true;
        }
    }

    if surface_changed {
        registry.generation += 1;
    }
    if shape_changed {
        registry.shape_generation += 1;
    }
}

fn is_surface_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "ron")
        && path.to_string_lossy().contains("surface")
}

fn is_shape_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "ron")
        && path.to_string_lossy().contains("shapes")
}

use bevy::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::shape::SpecNode;
use crate::surface::SurfaceDef;
use super::watcher::FileWatcher;

// =====================================================================
// Registry types
// =====================================================================

#[derive(Clone, Debug)]
struct RegisteredAsset<T: Clone> {
    data: T,
    path: PathBuf,
}

#[derive(Resource, Default, Clone)]
pub struct AssetRegistry {
    surfaces: HashMap<String, RegisteredAsset<SurfaceDef>>,
    shapes: HashMap<String, RegisteredAsset<Vec<SpecNode>>>,
    surface_generation: u64,
    shape_generation: u64,
    errors: Vec<AssetError>,
}

#[derive(Clone, Debug)]
pub struct AssetError {
    pub path: String,
    pub message: String,
}

impl AssetRegistry {
    // --- Surface accessors ---

    pub fn get_surface(&self, name: &str) -> Option<&SurfaceDef> {
        self.surfaces.get(name).map(|r| &r.data)
    }

    pub fn surface_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.surfaces.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn surface_path(&self, name: &str) -> Option<PathBuf> {
        self.surfaces.get(name).map(|r| r.path.clone())
    }

    pub fn has_surfaces(&self) -> bool {
        !self.surfaces.is_empty()
    }

    pub fn surface_generation(&self) -> u64 {
        self.surface_generation
    }

    pub fn upsert_surface(&mut self, name: String, data: SurfaceDef, path: PathBuf) {
        self.surfaces.insert(name, RegisteredAsset { data, path });
    }

    pub fn remove_surface(&mut self, name: &str) -> Option<PathBuf> {
        self.surfaces.remove(name).map(|r| r.path)
    }

    // --- Shape accessors ---

    pub fn get_shape(&self, name: &str) -> Option<&[SpecNode]> {
        if let Some(r) = self.shapes.get(name) {
            return Some(&r.data);
        }
        let with_ext = format!("{name}.shape.ron");
        if let Some(r) = self.shapes.get(&with_ext) {
            return Some(&r.data);
        }
        let suffix = format!("/{name}.shape.ron");
        let backslash_suffix = format!("\\{name}.shape.ron");
        for (key, r) in &self.shapes {
            if key.ends_with(&suffix) || key.ends_with(&backslash_suffix) || key == &with_ext {
                return Some(&r.data);
            }
        }
        None
    }

    pub fn get_shape_by_path(&self, path: &std::path::Path) -> Option<&[SpecNode]> {
        self.shapes.values()
            .find(|r| r.path == path)
            .map(|r| r.data.as_slice())
    }

    pub fn remove_by_path(&mut self, path: &std::path::Path) -> bool {
        let shape_key = self.shapes.iter()
            .find(|(_, r)| r.path == path)
            .map(|(k, _)| k.clone());
        if let Some(key) = shape_key {
            self.shapes.remove(&key);
            return true;
        }
        let surface_key = self.surfaces.iter()
            .find(|(_, r)| r.path == path)
            .map(|(k, _)| k.clone());
        if let Some(key) = surface_key {
            self.surfaces.remove(&key);
            return true;
        }
        false
    }

    pub fn shape_entries(&self) -> Vec<(String, PathBuf)> {
        let mut entries: Vec<(String, PathBuf)> = self.shapes.iter()
            .map(|(key, r)| (key.clone(), r.path.clone()))
            .collect();
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        entries
    }

    pub fn shape_generation(&self) -> u64 {
        self.shape_generation
    }

    // --- Error accessors ---

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn errors(&self) -> &[AssetError] {
        &self.errors
    }

    pub fn clear_error_for(&mut self, path: &str) {
        self.errors.retain(|e| e.path != path);
    }

    pub fn set_error(&mut self, path: String, message: String) {
        self.clear_error_for(&path);
        self.errors.push(AssetError { path, message });
    }

    /// Test-only helper: insert a `SpecNode` directly under a given key
    /// so downstream tests can exercise import resolution without
    /// touching the filesystem.
    #[cfg(test)]
    pub fn test_insert_shape(&mut self, key: impl Into<String>, data: Vec<SpecNode>) {
        self.shapes.insert(
            key.into(),
            RegisteredAsset {
                data,
                path: std::path::PathBuf::new(),
            },
        );
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

// =====================================================================
// Events — UI code fires these, registry handles the I/O
// =====================================================================

#[derive(Message)]
pub struct SaveSurface {
    pub name: String,
    pub data: SurfaceDef,
}

#[derive(Message)]
pub struct DeleteSurface {
    pub name: String,
}

impl AssetRegistry {
    /// Load all shapes and surfaces from the data directory without Bevy.
    pub fn load_from_disk(data_dir: &Path) -> Self {
        let mut registry = AssetRegistry::default();
        load_all_surfaces(data_dir, &mut registry);
        load_all_shapes(data_dir, &mut registry);
        registry
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
            .add_message::<SaveSurface>()
            .add_message::<DeleteSurface>()
            .add_systems(Update, (poll_file_changes, handle_save_surface, handle_delete_surface));
    }
}

fn handle_save_surface(
    mut events: MessageReader<SaveSurface>,
    mut registry: ResMut<AssetRegistry>,
) {
    for event in events.read() {
        let path = registry.surface_path(&event.name)
            .unwrap_or_else(|| {
                let filename = format!("{}.surface.ron", event.name.replace(' ', "_").to_lowercase());
                PathBuf::from("data/surfaces").join(filename)
            });

        save_surface_to_file(&event.data, &path);
        registry.upsert_surface(event.name.clone(), event.data.clone(), path);
    }
}

fn handle_delete_surface(
    mut events: MessageReader<DeleteSurface>,
    mut registry: ResMut<AssetRegistry>,
) {
    for event in events.read() {
        if let Some(path) = registry.remove_surface(&event.name) {
            if let Err(e) = std::fs::remove_file(&path) {
                warn!("Failed to delete '{}': {}", path.display(), e);
            }
        }
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
        if path.is_dir() {
            load_ron_files(&path, registry, filter, loader);
        } else if filter(&path) {
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

    let surface: SurfaceDef = match crate::util::parse_ron(&contents) {
        Ok(s) => s,
        Err(e) => {
            registry.set_error(path_str, format!("{e}"));
            return;
        }
    };

    registry.clear_error_for(&path_str);

    let name = surface.name.clone();
    registry.surfaces.insert(name, RegisteredAsset {
        data: surface,
        path: path.to_path_buf(),
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

    let parts: Vec<SpecNode> = match crate::util::parse_ron(&contents) {
        Ok(s) => s,
        Err(e) => {
            registry.set_error(path_str, format!("{e}"));
            return;
        }
    };

    registry.clear_error_for(&path_str);

    let key = shape_key_from_path(path);

    registry.shapes.insert(key, RegisteredAsset {
        data: parts,
        path: path.to_path_buf(),
    });
}


/// Derive a human-readable shape name from a file path.
/// e.g., "data/shapes/frz-b/assembly.shape.ron" → "frz-b/assembly"
pub fn shape_name_from_path(path: &Path) -> String {
    let key = shape_key_from_path(path);
    key.strip_suffix(".shape.ron").unwrap_or(&key).to_string()
}

/// Compute the registry key for a shape file: relative path from data/shapes/.
/// e.g., "data/shapes/wheel.shape.ron" → "wheel.shape.ron"
///        "data/shapes/robots/arm.shape.ron" → "robots/arm.shape.ron"
fn shape_key_from_path(path: &Path) -> String {
    // Try to strip the data/shapes/ prefix
    let shapes_dir = Path::new("data").join("shapes");
    if let Ok(relative) = path.strip_prefix(&shapes_dir) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    // Fallback: use the full path
    path.to_string_lossy().replace('\\', "/")
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

    let (changed_paths, deleted_paths) = watcher.detect_changes();
    if changed_paths.is_empty() && deleted_paths.is_empty() {
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

    for path in &deleted_paths {
        if registry.remove_by_path(path) {
            if is_surface_file(path) { surface_changed = true; }
            if is_shape_file(path) { shape_changed = true; }
        }
    }

    if surface_changed {
        registry.surface_generation += 1;
    }
    if shape_changed {
        registry.shape_generation += 1;
    }
}

fn is_surface_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".surface.ron"))
}

fn is_shape_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".shape.ron"))
}

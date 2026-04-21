use bevy::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const POLL_INTERVAL_SECS: f64 = 0.5;

#[derive(Resource)]
pub struct FileWatcher {
    data_dir: PathBuf,
    last_poll: f64,
    /// Tracks the last-seen mtime for every file, regardless of parse success.
    /// This prevents re-attempting a broken file every poll cycle.
    seen_mtimes: HashMap<PathBuf, SystemTime>,
}

impl FileWatcher {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir, last_poll: 0.0, seen_mtimes: HashMap::new() }
    }

    pub fn should_poll(&mut self, now: f64) -> bool {
        if now - self.last_poll < POLL_INTERVAL_SECS {
            return false;
        }
        self.last_poll = now;
        true
    }

    pub fn detect_changes(&mut self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut changed = Vec::new();
        let mut current_paths = std::collections::HashSet::new();
        collect_changed_files(&self.data_dir, &mut self.seen_mtimes, &mut changed, &mut current_paths);

        // Files in seen_mtimes but not on disk → deleted.
        let deleted: Vec<PathBuf> = self.seen_mtimes.keys()
            .filter(|p| !current_paths.contains(*p))
            .cloned()
            .collect();
        for p in &deleted {
            self.seen_mtimes.remove(p);
        }

        (changed, deleted)
    }
}

fn collect_changed_files(
    dir: &Path,
    seen_mtimes: &mut HashMap<PathBuf, SystemTime>,
    changed: &mut Vec<PathBuf>,
    current_paths: &mut std::collections::HashSet<PathBuf>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories (e.g. `.backups/`) so the watcher
            // doesn't trigger reloads on editor-managed snapshots.
            if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) {
                continue;
            }
            collect_changed_files(&path, seen_mtimes, changed, current_paths);
        } else if path.extension().is_some_and(|ext| ext == "ron") {
            current_paths.insert(path.clone());
            if has_new_modification(&path, seen_mtimes) {
                changed.push(path);
            }
        }
    }
}

fn has_new_modification(path: &Path, seen_mtimes: &mut HashMap<PathBuf, SystemTime>) -> bool {
    let current_mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return false,
    };

    let already_seen = seen_mtimes.get(path).is_some_and(|&prev| current_mtime <= prev);
    seen_mtimes.insert(path.to_path_buf(), current_mtime);
    !already_seen
}

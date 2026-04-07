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

    pub fn detect_changes(&mut self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        collect_changed_files(&self.data_dir, &mut self.seen_mtimes, &mut changed);
        changed
    }
}

fn collect_changed_files(
    dir: &Path,
    seen_mtimes: &mut HashMap<PathBuf, SystemTime>,
    changed: &mut Vec<PathBuf>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_changed_files(&path, seen_mtimes, changed);
        } else if path.extension().is_some_and(|ext| ext == "ron") {
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

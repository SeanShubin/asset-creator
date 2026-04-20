//! Editing infrastructure: the working copy, command history, and the
//! systems that turn user input (Delete, Ctrl+Z, etc.) into mutations on
//! the working copy plus persistence to disk.
//!
//! `WorkingShape` is the authoritative version of the currently-loaded
//! shape during an editing session. `reload_shape` and `compute_stats`
//! read from it; the registry stays as the "last loaded from disk"
//! snapshot. External edits to the active file are ignored — `WorkingShape`
//! wins. Edits to imports still hot-reload because imports are resolved
//! through the registry, not `WorkingShape`.

use bevy::prelude::*;
use std::path::PathBuf;

use crate::shape::SpecNode;
use super::object_editor::{CurrentShape, ReloadShape, Selection};

// =====================================================================
// Public API
// =====================================================================

/// The editor's working copy of the active shape's parts. Cloned from the
/// registry on shape switch; mutated in-place by edit commands; written
/// back to disk by the auto-save system.
#[derive(Resource, Default)]
pub struct WorkingShape {
    pub parts: Vec<SpecNode>,
    pub path: Option<PathBuf>,
    /// Set true on every committed edit; cleared after auto-save writes.
    pub dirty: bool,
    /// Seconds-since-startup of the most recent edit. Used by auto-save
    /// to debounce — we only write after a quiet period.
    pub last_edit_secs: Option<f64>,
}

/// Undo/redo stacks of edit commands. Cleared on shape switch.
#[derive(Resource, Default)]
pub struct CommandHistory {
    undo: Vec<EditCommand>,
    redo: Vec<EditCommand>,
}

/// A reversible edit operation. Each variant carries enough captured state
/// (after `apply`) to invert itself.
#[derive(Clone, Debug)]
pub enum EditCommand {
    Delete(DeleteOp),
}

#[derive(Clone, Debug)]
pub struct DeleteOp {
    /// Source path of the node to delete (e.g. `"chassis_top/wheel"`).
    pub path: String,
    /// Captured during `apply` so `invert` can restore the node to the
    /// same place. None before the first apply.
    captured: Option<(String, usize, SpecNode)>, // (parent_path, index, node)
}

impl EditCommand {
    fn apply(&mut self, parts: &mut Vec<SpecNode>) -> bool {
        match self {
            EditCommand::Delete(op) => {
                let Some((parent_path, idx, node)) = remove_at_path(parts, &op.path) else {
                    return false;
                };
                op.captured = Some((parent_path, idx, node));
                true
            }
        }
    }

    fn invert(&self, parts: &mut Vec<SpecNode>) -> bool {
        match self {
            EditCommand::Delete(op) => {
                let Some((parent_path, idx, node)) = &op.captured else { return false };
                insert_at_path(parts, parent_path, *idx, node.clone())
            }
        }
    }
}

// =====================================================================
// Tree walking — find / remove / insert SpecNode by slash-separated path
// =====================================================================

/// Remove the node identified by `path` and return `(parent_path, index, node)`
/// for later restoration. Returns `None` if no node matches.
pub fn remove_at_path(parts: &mut Vec<SpecNode>, path: &str) -> Option<(String, usize, SpecNode)> {
    if path.is_empty() { return None; }
    let segments: Vec<&str> = path.split('/').collect();
    let parent_path = if segments.len() > 1 {
        segments[..segments.len() - 1].join("/")
    } else {
        String::new()
    };
    let (idx, node) = remove_at_segments(parts, &segments)?;
    Some((parent_path, idx, node))
}

fn remove_at_segments(parts: &mut Vec<SpecNode>, segments: &[&str]) -> Option<(usize, SpecNode)> {
    if segments.len() == 1 {
        let idx = parts.iter().position(|n| n.effective_name() == Some(segments[0]))?;
        let removed = parts.remove(idx);
        return Some((idx, removed));
    }
    let head = segments[0];
    let rest = &segments[1..];
    let idx = parts.iter().position(|n| n.effective_name() == Some(head))?;
    remove_at_segments(&mut parts[idx].children, rest)
}

/// Insert `node` at `index` inside the node at `parent_path` (or at the
/// top level if `parent_path` is empty). Returns false if the parent path
/// doesn't resolve.
pub fn insert_at_path(
    parts: &mut Vec<SpecNode>,
    parent_path: &str,
    index: usize,
    node: SpecNode,
) -> bool {
    let segments: Vec<&str> = if parent_path.is_empty() {
        Vec::new()
    } else {
        parent_path.split('/').collect()
    };
    insert_at_segments(parts, &segments, index, node)
}

fn insert_at_segments(
    parts: &mut Vec<SpecNode>,
    segments: &[&str],
    index: usize,
    node: SpecNode,
) -> bool {
    if segments.is_empty() {
        let clamped = index.min(parts.len());
        parts.insert(clamped, node);
        return true;
    }
    let head = segments[0];
    let rest = &segments[1..];
    let Some(idx) = parts.iter().position(|n| n.effective_name() == Some(head)) else {
        return false;
    };
    insert_at_segments(&mut parts[idx].children, rest, index, node)
}

// =====================================================================
// Systems
// =====================================================================

/// Press Delete or Backspace to remove the selected part from the working
/// shape. Pushes a `DeleteCommand` onto the undo stack so it can be undone.
pub fn delete_selected(
    keys: Res<ButtonInput<KeyCode>>,
    mut working: ResMut<WorkingShape>,
    mut history: ResMut<CommandHistory>,
    mut selection: ResMut<Selection>,
    mut reload_events: MessageWriter<ReloadShape>,
    time: Res<Time>,
) {
    if !(keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace)) {
        return;
    }
    let Some(path) = selection.source_path.clone() else { return };
    if path.is_empty() { return; }

    let mut cmd = EditCommand::Delete(DeleteOp { path: path.clone(), captured: None });
    if !cmd.apply(&mut working.parts) {
        warn!("delete: path '{}' not found in working shape", path);
        return;
    }
    history.undo.push(cmd);
    history.redo.clear();
    working.dirty = true;
    working.last_edit_secs = Some(time.elapsed_secs_f64());
    selection.source_path = None;
    reload_events.write(ReloadShape);
    info!("deleted '{}'", path);
}

/// Ctrl+Z undoes; Ctrl+Shift+Z (or Ctrl+Y) redoes.
pub fn undo_redo(
    keys: Res<ButtonInput<KeyCode>>,
    mut working: ResMut<WorkingShape>,
    mut history: ResMut<CommandHistory>,
    mut selection: ResMut<Selection>,
    mut reload_events: MessageWriter<ReloadShape>,
    time: Res<Time>,
) {
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if !ctrl { return; }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    let want_undo = keys.just_pressed(KeyCode::KeyZ) && !shift;
    let want_redo = (keys.just_pressed(KeyCode::KeyZ) && shift) || keys.just_pressed(KeyCode::KeyY);

    if want_undo {
        if let Some(cmd) = history.undo.pop() {
            if !cmd.invert(&mut working.parts) {
                warn!("undo: invert failed; discarding command");
                return;
            }
            working.dirty = true;
            working.last_edit_secs = Some(time.elapsed_secs_f64());
            selection.source_path = None;
            history.redo.push(cmd);
            reload_events.write(ReloadShape);
            info!("undo");
        }
    } else if want_redo {
        if let Some(mut cmd) = history.redo.pop() {
            if !cmd.apply(&mut working.parts) {
                warn!("redo: apply failed; discarding command");
                return;
            }
            working.dirty = true;
            working.last_edit_secs = Some(time.elapsed_secs_f64());
            selection.source_path = None;
            history.undo.push(cmd);
            reload_events.write(ReloadShape);
            info!("redo");
        }
    }
}

/// Auto-save: when `working.dirty` and the last edit was at least
/// `AUTOSAVE_DEBOUNCE_SECS` ago, serialize and write to disk.
const AUTOSAVE_DEBOUNCE_SECS: f64 = 0.25;

pub fn auto_save(
    mut working: ResMut<WorkingShape>,
    time: Res<Time>,
) {
    if !working.dirty { return; }
    let Some(last) = working.last_edit_secs else { return };
    if time.elapsed_secs_f64() - last < AUTOSAVE_DEBOUNCE_SECS { return; }
    let Some(path) = working.path.clone() else { return };

    match write_shape_to_disk(&path, &working.parts) {
        Ok(()) => {
            working.dirty = false;
            info!("saved '{}'", path.display());
        }
        Err(e) => {
            error!("save '{}' failed: {}", path.display(), e);
            // Leave dirty=true so we'll retry on the next quiet period.
        }
    }
}

fn write_shape_to_disk(path: &std::path::Path, parts: &[SpecNode]) -> Result<(), String> {
    let config = ron::ser::PrettyConfig::default();
    let ron_str = ron::ser::to_string_pretty(parts, config)
        .map_err(|e| format!("serialize: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {e}"))?;
    }
    std::fs::write(path, ron_str).map_err(|e| format!("write: {e}"))
}

/// Reset the working copy and clear history when `CurrentShape` changes.
/// Runs before `detect_shape_change` so the new shape's parts get cloned in.
pub fn reset_working_on_shape_switch(
    current: Res<CurrentShape>,
    mut working: ResMut<WorkingShape>,
    mut history: ResMut<CommandHistory>,
    registry: Res<crate::registry::AssetRegistry>,
) {
    if working.path == current.path {
        return;
    }
    working.parts.clear();
    working.path = current.path.clone();
    working.dirty = false;
    working.last_edit_secs = None;
    history.undo.clear();
    history.redo.clear();

    if let Some(ref path) = current.path {
        if let Some(parts) = registry.get_shape_by_path(path) {
            working.parts = parts.to_vec();
        }
    }
}

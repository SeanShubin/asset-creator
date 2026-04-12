mod animation;
mod csg;
mod definition;
mod interpreter;
mod meshes;
mod sdf;
mod traversal;

use bevy::prelude::*;

pub use animation::{animate_shapes, ShapeAnimator};
pub use csg::{CsgStats, perform_csg_uncached};
pub use definition::{Bounds, CombineMode, ShapeNode};
pub use interpreter::{despawn_shape, spawn_shape, spawn_shape_with_layers, rebuild_csg_on_toggle, suppress_csg_member_meshes, ShapePart, ShapeRoot};
pub use traversal::{walk_shape_tree, collect_mesh_from_events, ColorMap, ShapeEvent};

/// Plugin that maintains shape system invariants.
/// Register this once; it ensures CSG member meshes are always suppressed
/// regardless of which editor or consumer spawns shapes.
pub struct ShapePlugin;

impl Plugin for ShapePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (
            bevy::ecs::schedule::apply_deferred,
            suppress_csg_member_meshes,
        ).chain());
    }
}

mod animation;
mod csg;
mod interpreter;
mod meshes;
mod render;
mod sdf;
mod spec;

use bevy::prelude::*;

pub use animation::{animate_shapes, ShapeAnimator};
pub use csg::{CsgStats, perform_csg_uncached};
pub use interpreter::{
    despawn_shape, rebuild_csg_on_toggle, spawn_shape, spawn_shape_as_sdf,
    spawn_shape_with_layers, suppress_csg_member_meshes, ShapePart, ShapeRoot,
};
pub use render::{collect_raw_mesh, compile, ColorMap, RenderEvent};
pub use spec::{Bounds, CombineMode, SpecNode};

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

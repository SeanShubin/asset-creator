mod animation;
mod interpreter;
mod meshes;
mod render;
mod spec;

use bevy::prelude::*;

pub use animation::{animate_shapes, ShapeAnimator};
pub use interpreter::{
    despawn_shape, spawn_shape, spawn_shape_with_layers, ShapePart, ShapeRoot,
};
pub use render::{compile, CompiledShape};
#[allow(unused_imports)]
pub use spec::{
    collect_occupancy, identity_placement, Bounds, Collision, CombineMode,
    Occupancy, Placement, SpecNode, Symmetry,
};

/// Plugin placeholder for shape-system invariants. Currently empty
/// because the cell-level fusion pipeline has no runtime maintenance
/// systems — everything is computed at shape spawn time.
pub struct ShapePlugin;

impl Plugin for ShapePlugin {
    fn build(&self, _app: &mut App) {}
}

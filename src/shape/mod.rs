mod animation;
pub mod csg;
mod interpreter;
mod meshes;
mod render;
pub mod spec;

use bevy::prelude::*;

pub use animation::ShapeAnimator;
pub use interpreter::{
    despawn_shape, spawn_shape, spawn_shape_with_layers, ShapePart, ShapeRoot,
};
pub use render::{compile, production_stats, CompiledShape, FusedMesh};
pub use meshes::RawMesh;
#[allow(unused_imports)]
pub use spec::{
    collect_occupancy, aabb_for_parts, identity_placement, Bounds,
    Collision, Occupancy, Placement, SpecNode, SymOp,
};

/// Plugin placeholder for shape-system invariants. Currently empty
/// because the cell-level fusion pipeline has no runtime maintenance
/// systems — everything is computed at shape spawn time.
pub struct ShapePlugin;

impl Plugin for ShapePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animation::animate_shapes);
    }
}

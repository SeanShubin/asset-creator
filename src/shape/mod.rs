mod animation;
mod csg;
mod interpreter;
mod meshes;
mod render;
mod spec;

use bevy::prelude::*;

pub use animation::{animate_shapes, ShapeAnimator};
pub use interpreter::{
    despawn_shape, spawn_shape, spawn_shape_with_layers, ShapePart, ShapeRoot,
};
pub use render::{base_orientation_matrix, compile, CompiledShape, FusedMesh};
pub use meshes::RawMesh;
#[allow(unused_imports)]
pub use spec::{
    collect_occupancy, aabb_for_parts, identity_placement, Bounds,
    Collision, Facing, Mirroring, Occupancy, Orientation, Placement, Rotation,
    SpecNode, Symmetry,
};

/// Plugin placeholder for shape-system invariants. Currently empty
/// because the cell-level fusion pipeline has no runtime maintenance
/// systems — everything is computed at shape spawn time.
pub struct ShapePlugin;

impl Plugin for ShapePlugin {
    fn build(&self, _app: &mut App) {}
}

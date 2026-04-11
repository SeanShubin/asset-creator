mod animation;
mod csg;
mod definition;
mod interpreter;
mod meshes;
mod traversal;

pub use animation::{animate_shapes, ShapeAnimator};
pub use definition::ShapeNode;
pub use interpreter::{despawn_shape, spawn_shape, rebuild_csg_on_toggle, suppress_csg_member_meshes, ShapePart, ShapeRoot};

mod animation;
mod definition;
mod interpreter;
mod meshes;

pub use animation::{animate_shapes, ShapeAnimator};
pub use definition::ShapeNode;
pub use interpreter::{despawn_shape, spawn_shape, ShapePart, ShapeRoot};

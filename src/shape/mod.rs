mod animation;
mod definition;
mod interpreter;
mod meshes;

pub use animation::{animate_shapes, ShapeAnimator};
pub use definition::{AnimState, Axis, PrimitiveShape, ShapeFile, ShapeNode};
pub use interpreter::{despawn_shape, load_shape, spawn_shape, BaseTransform, ShapePart, ShapeRoot};

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Clone, Debug)]
pub struct ShapeFile {
    #[serde(default)]
    pub templates: HashMap<String, ShapeNode>,
    pub root: ShapeNode,
    #[serde(default)]
    pub animations: Vec<AnimState>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ShapeNode {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub shape: Option<PrimitiveShape>,
    #[serde(default)]
    pub at: (f32, f32, f32),
    #[serde(default)]
    pub pivot: Option<(f32, f32, f32)>,
    #[serde(default)]
    pub color: Option<(f32, f32, f32)>,
    #[serde(default)]
    pub emissive: bool,
    #[serde(default)]
    pub orient: Option<Axis>,
    #[serde(default)]
    pub rotate: Option<(f32, Axis)>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub children: Vec<ShapeNode>,
    #[serde(default)]
    pub mirror: Option<Axis>,
    #[serde(default)]
    pub repeat: Option<RepeatSpec>,
}

#[derive(Deserialize, Clone, Debug)]
pub enum PrimitiveShape {
    Box { size: (f32, f32, f32) },
    Sphere { radius: f32 },
    Cylinder { radius: f32, height: f32 },
}

#[derive(Deserialize, Clone, Copy, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RepeatSpec {
    pub count: u32,
    pub spacing: f32,
    pub along: Axis,
    #[serde(default)]
    pub center: bool,
}

// =====================================================================
// Animation data
// =====================================================================

#[derive(Deserialize, Clone, Debug)]
pub enum JointMotion {
    Oscillate {
        amplitude: f32,
        speed: f32,
        #[serde(default)]
        offset: f32,
    },
    Spin {
        rate: f32,
    },
    Bob {
        amplitude: f32,
        freq: f32,
    },
}

#[derive(Deserialize, Clone, Debug)]
pub struct AnimChannel {
    pub part: String,
    pub property: AnimProperty,
    pub motion: JointMotion,
    pub axis: Axis,
}

#[derive(Deserialize, Clone, Copy, Debug)]
pub enum AnimProperty {
    Rotation,
    Translation,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AnimState {
    pub name: String,
    pub channels: Vec<AnimChannel>,
}

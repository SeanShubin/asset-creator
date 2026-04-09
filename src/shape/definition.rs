use bevy::math::{Mat3, Quat, Vec3};
use serde::Deserialize;
use std::collections::HashMap;
use crate::util::Color3;

/// A shape node is both the file format and the node type.
/// A `.shape.ron` file IS a ShapeNode.
#[derive(Deserialize, Clone, Debug)]
pub struct ShapeNode {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub shape: Option<PrimitiveShape>,
    #[serde(default)]
    pub bounds: Option<Bounds>,
    #[serde(default, deserialize_with = "deserialize_orient")]
    pub orient: Mat3,
    #[serde(default)]
    pub colors: HashMap<String, Color3>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub emissive: bool,
    #[serde(default)]
    pub rotate: Option<(f32, Axis)>,
    #[serde(default)]
    pub import: Option<String>,
    #[serde(default)]
    pub children: Vec<ShapeNode>,
    #[serde(default)]
    pub mirror: Vec<Axis>,
    #[serde(default)]
    pub repeat: Option<RepeatSpec>,
    #[serde(default)]
    pub animations: Vec<AnimState>,
}

/// What kind of combinator this node is, if any.
pub enum Combinator<'a> {
    Mirror(&'a [Axis]),
    Repeat(&'a RepeatSpec),
    Import(&'a str),
    None,
}

impl ShapeNode {
    /// Determine what kind of combinator this node is.
    /// Combinators generate multiple children or redirect to other shapes.
    /// A node is at most one combinator type; priority: mirror > repeat > import.
    pub fn combinator(&self) -> Combinator<'_> {
        if !self.mirror.is_empty() {
            Combinator::Mirror(&self.mirror)
        } else if let Some(ref repeat) = self.repeat {
            Combinator::Repeat(repeat)
        } else if let Some(ref import) = self.import {
            Combinator::Import(import)
        } else {
            Combinator::None
        }
    }

    /// Whether this node is a combinator (mirror, repeat, or import).
    /// Combinators are pass-through containers — they don't have their own position.
    pub fn is_combinator(&self) -> bool {
        !matches!(self.combinator(), Combinator::None)
    }

    /// Compute the AABB enclosing this node and all descendants.
    pub fn compute_aabb(&self) -> Option<Bounds> {
        let mut min = (f32::MAX, f32::MAX, f32::MAX);
        let mut max = (f32::MIN, f32::MIN, f32::MIN);
        let mut found = false;

        self.collect_bounds(&mut min, &mut max, &mut found);

        if found {
            Some(Bounds(min.0, min.1, min.2, max.0, max.1, max.2))
        } else {
            None
        }
    }

    fn collect_bounds(&self, min: &mut (f32, f32, f32), max: &mut (f32, f32, f32), found: &mut bool) {
        if let Some(b) = &self.bounds {
            let b_min = b.min();
            let b_max = b.max();

            // Include this node's bounds
            include_point(min, max, b_min, found);
            include_point(min, max, b_max, found);

            // Include mirrored copies
            for &axis in &self.mirror {
                let (mir_min, mir_max) = reflect_extents(b_min, b_max, axis);
                include_point(min, max, mir_min, found);
                include_point(min, max, mir_max, found);
            }

            // Include repeated copies
            if let Some(ref repeat) = self.repeat {
                let start = if repeat.center {
                    -(repeat.count as f32 - 1.0) * repeat.spacing * 0.5
                } else {
                    0.0
                };
                let last_offset = start + (repeat.count as f32 - 1.0) * repeat.spacing;
                let (first, last) = match repeat.along {
                    Axis::X => ((b_min.0 + start, b_min.1, b_min.2), (b_max.0 + last_offset, b_max.1, b_max.2)),
                    Axis::Y => ((b_min.0, b_min.1 + start, b_min.2), (b_max.0, b_max.1 + last_offset, b_max.2)),
                    Axis::Z => ((b_min.0, b_min.1, b_min.2 + start), (b_max.0, b_max.1, b_max.2 + last_offset)),
                };
                include_point(min, max, first, found);
                include_point(min, max, last, found);
            }
        }
        for child in &self.children {
            child.collect_bounds(min, max, found);
        }
    }

    /// Remap all bounds in this node and its descendants from one coordinate space to another.
    /// Also remaps repeat spacing to match the new coordinate scale.
    pub fn remap_bounds(&mut self, from: &Bounds, to: &Bounds) {
        if let Some(ref mut b) = self.bounds {
            *b = b.remap(from, to);
        }
        if let Some(ref mut repeat) = self.repeat {
            let from_size = from.size();
            let to_size = to.size();
            let scale = match repeat.along {
                Axis::X => if from_size.0.abs() > 0.001 { to_size.0 / from_size.0 } else { 1.0 },
                Axis::Y => if from_size.1.abs() > 0.001 { to_size.1 / from_size.1 } else { 1.0 },
                Axis::Z => if from_size.2.abs() > 0.001 { to_size.2 / from_size.2 } else { 1.0 },
            };
            repeat.spacing *= scale;
        }
        for child in &mut self.children {
            child.remap_bounds(from, to);
        }
    }
}

fn include_point(min: &mut (f32, f32, f32), max: &mut (f32, f32, f32), p: (f32, f32, f32), found: &mut bool) {
    min.0 = min.0.min(p.0);
    min.1 = min.1.min(p.1);
    min.2 = min.2.min(p.2);
    max.0 = max.0.max(p.0);
    max.1 = max.1.max(p.1);
    max.2 = max.2.max(p.2);
    *found = true;
}

fn reflect_extents(b_min: (f32, f32, f32), b_max: (f32, f32, f32), axis: Axis) -> ((f32, f32, f32), (f32, f32, f32)) {
    match axis {
        Axis::X => ((-b_max.0, b_min.1, b_min.2), (-b_min.0, b_max.1, b_max.2)),
        Axis::Y => ((b_min.0, -b_max.1, b_min.2), (b_max.0, -b_min.1, b_max.2)),
        Axis::Z => ((b_min.0, b_min.1, -b_max.2), (b_max.0, b_max.1, -b_min.2)),
    }
}

// =====================================================================
// Primitives
// =====================================================================

#[derive(Deserialize, Clone, Copy, Debug)]
pub enum PrimitiveShape {
    Box,
    Sphere,
    Cylinder,
    Dome,
    Cone,
    Wedge,
    Torus,
    Corner,
}

// =====================================================================
// Bounds
// =====================================================================

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Bounds(pub f32, pub f32, pub f32, pub f32, pub f32, pub f32);

impl Bounds {
    pub fn center(&self) -> (f32, f32, f32) {
        ((self.0 + self.3) / 2.0, (self.1 + self.4) / 2.0, (self.2 + self.5) / 2.0)
    }

    pub fn size(&self) -> (f32, f32, f32) {
        ((self.3 - self.0).abs(), (self.4 - self.1).abs(), (self.5 - self.2).abs())
    }

    pub fn min(&self) -> (f32, f32, f32) {
        (self.0.min(self.3), self.1.min(self.4), self.2.min(self.5))
    }

    pub fn max(&self) -> (f32, f32, f32) {
        (self.0.max(self.3), self.1.max(self.4), self.2.max(self.5))
    }

    /// Remap this bounds from `from` coordinate space into `to` coordinate space.
    /// Each corner is mapped: to_min + (point - from_min) * (to_size / from_size)
    pub fn remap(&self, from: &Bounds, to: &Bounds) -> Bounds {
        let remap_component = |val: f32, from_min: f32, from_size: f32, to_min: f32, to_size: f32| -> f32 {
            if from_size.abs() < 0.001 { to_min } else {
                to_min + (val - from_min) * (to_size / from_size)
            }
        };

        let from_min = from.min();
        let from_size = from.size();
        let to_min = to.min();
        let to_size = to.size();

        Bounds(
            remap_component(self.0, from_min.0, from_size.0, to_min.0, to_size.0),
            remap_component(self.1, from_min.1, from_size.1, to_min.1, to_size.1),
            remap_component(self.2, from_min.2, from_size.2, to_min.2, to_size.2),
            remap_component(self.3, from_min.0, from_size.0, to_min.0, to_size.0),
            remap_component(self.4, from_min.1, from_size.1, to_min.1, to_size.1),
            remap_component(self.5, from_min.2, from_size.2, to_min.2, to_size.2),
        )
    }
}

// =====================================================================
// Axes
// =====================================================================

#[derive(Deserialize, Clone, Copy, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

// =====================================================================
// Orient — stored as Mat3, deserialized from (Facing, Mirroring, Rotation)
// =====================================================================

/// Deserialization helper: parse the human-readable tuple and convert to Mat3.
fn deserialize_orient<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Mat3, D::Error> {
    let tuple = Option::<OrientTuple>::deserialize(deserializer)?;
    Ok(tuple.map(|t| t.to_matrix()).unwrap_or(Mat3::IDENTITY))
}

/// Human-readable orient tuple — only used for RON deserialization.
#[derive(Deserialize)]
struct OrientTuple(
    #[serde(default)] Facing,
    #[serde(default)] Mirroring,
    #[serde(default)] Rotation,
);

impl OrientTuple {
    fn to_matrix(&self) -> Mat3 {
        let facing = facing_matrix(self.0);
        let mirror = match self.1 {
            Mirroring::NoMirror => Mat3::IDENTITY,
            Mirroring::Mirror => Mat3::from_cols(Vec3::NEG_X, Vec3::Y, Vec3::Z),
        };
        let rotation = rotation_matrix(self.2);
        rotation * mirror * facing
    }
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub enum Facing {
    #[default]
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub enum Mirroring {
    #[default]
    NoMirror,
    Mirror,
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub enum Rotation {
    #[default]
    NoRotation,
    RotateClockwise,
    RotateHalf,
    RotateCounter,
}

/// Reflect a world axis in an orient matrix.
/// This is the operation performed by the mirror combinator on reflected copies.
pub fn reflect_orient(orient: &mut Mat3, axis: Axis) {
    // Negating a row of the local→world matrix reflects that world axis.
    // Row i corresponds to the world axis i component across all columns.
    match axis {
        Axis::X => {
            orient.x_axis.x = -orient.x_axis.x;
            orient.y_axis.x = -orient.y_axis.x;
            orient.z_axis.x = -orient.z_axis.x;
        }
        Axis::Y => {
            orient.x_axis.y = -orient.x_axis.y;
            orient.y_axis.y = -orient.y_axis.y;
            orient.z_axis.y = -orient.z_axis.y;
        }
        Axis::Z => {
            orient.x_axis.z = -orient.x_axis.z;
            orient.y_axis.z = -orient.y_axis.z;
            orient.z_axis.z = -orient.z_axis.z;
        }
    }
}

fn facing_matrix(facing: Facing) -> Mat3 {
    use std::f32::consts::FRAC_PI_2;
    match facing {
        Facing::Front => Mat3::IDENTITY,
        Facing::Back => Mat3::from_quat(Quat::from_rotation_y(std::f32::consts::PI)),
        Facing::Left => Mat3::from_quat(Quat::from_rotation_y(-FRAC_PI_2)),
        Facing::Right => Mat3::from_quat(Quat::from_rotation_y(FRAC_PI_2)),
        Facing::Top => Mat3::from_quat(Quat::from_rotation_x(-FRAC_PI_2)),
        Facing::Bottom => Mat3::from_quat(Quat::from_rotation_x(FRAC_PI_2)),
    }
}

fn rotation_matrix(rotation: Rotation) -> Mat3 {
    use std::f32::consts::{FRAC_PI_2, PI};
    match rotation {
        Rotation::NoRotation => Mat3::IDENTITY,
        Rotation::RotateClockwise => Mat3::from_quat(Quat::from_rotation_z(-FRAC_PI_2)),
        Rotation::RotateHalf => Mat3::from_quat(Quat::from_rotation_z(PI)),
        Rotation::RotateCounter => Mat3::from_quat(Quat::from_rotation_z(FRAC_PI_2)),
    }
}

// =====================================================================
// Repeat
// =====================================================================

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

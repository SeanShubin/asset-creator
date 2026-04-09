use bevy::math::{Mat3, Quat, Vec3};
use serde::Deserialize;
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
    #[serde(default)]
    pub orient: Vec<SignedAxis>,
    #[serde(default)]
    pub color: Option<Color3>,
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

impl ShapeNode {
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
            min.0 = min.0.min(b_min.0);
            min.1 = min.1.min(b_min.1);
            min.2 = min.2.min(b_min.2);
            max.0 = max.0.max(b_max.0);
            max.1 = max.1.max(b_max.1);
            max.2 = max.2.max(b_max.2);
            *found = true;
        }
        for child in &self.children {
            child.collect_bounds(min, max, found);
        }
    }

    /// Remap all bounds in this node and its descendants from one coordinate space to another.
    pub fn remap_bounds(&mut self, from: &Bounds, to: &Bounds) {
        if let Some(ref mut b) = self.bounds {
            *b = b.remap(from, to);
        }
        for child in &mut self.children {
            child.remap_bounds(from, to);
        }
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

#[derive(Deserialize, Clone, Copy, Debug)]
pub enum SignedAxis {
    X,
    NegX,
    Y,
    NegY,
    Z,
    NegZ,
}

impl Default for SignedAxis {
    fn default() -> Self {
        Self::Y
    }
}

// =====================================================================
// Orient — interpret a list of signed axes as orientation
//   []         → identity (no rotation)
//   [axis]     → single axis: primary axis points along `axis`
//   [r, u, f]  → full frame: right, up, forward
// =====================================================================

/// Compute the 3x3 orient matrix from the orient specification.
/// This maps local (X,Y,Z) to world directions.
/// For 0 axes: identity. For 1 axis: single-axis rotation.
/// For 3 axes: columns are (right, up, forward).
pub fn orient_matrix(orient: &[SignedAxis]) -> Mat3 {
    match orient.len() {
        0 => Mat3::IDENTITY,
        1 => Mat3::from_quat(single_axis_rotation(orient[0])),
        3 => Mat3::from_cols(
            signed_axis_to_vec3(orient[0]),
            signed_axis_to_vec3(orient[1]),
            signed_axis_to_vec3(orient[2]),
        ),
        _ => {
            bevy::log::warn!("orient must have 0, 1, or 3 axes, got {}", orient.len());
            Mat3::IDENTITY
        }
    }
}

fn single_axis_rotation(axis: SignedAxis) -> Quat {
    match axis {
        SignedAxis::Y => Quat::IDENTITY,
        SignedAxis::NegY => Quat::from_rotation_x(std::f32::consts::PI),
        SignedAxis::X => Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2),
        SignedAxis::NegX => Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        SignedAxis::Z => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        SignedAxis::NegZ => Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
    }
}

fn signed_axis_to_vec3(axis: SignedAxis) -> Vec3 {
    match axis {
        SignedAxis::X => Vec3::X,
        SignedAxis::NegX => Vec3::NEG_X,
        SignedAxis::Y => Vec3::Y,
        SignedAxis::NegY => Vec3::NEG_Y,
        SignedAxis::Z => Vec3::Z,
        SignedAxis::NegZ => Vec3::NEG_Z,
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

use bevy::math::{Mat3, Quat, Vec3};
use serde::Deserialize;

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
    pub color: Option<(f32, f32, f32)>,
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

impl SignedAxis {
    pub fn unsigned(self) -> Axis {
        match self {
            Self::X | Self::NegX => Axis::X,
            Self::Y | Self::NegY => Axis::Y,
            Self::Z | Self::NegZ => Axis::Z,
        }
    }

    pub fn is_negative(self) -> bool {
        matches!(self, Self::NegX | Self::NegY | Self::NegZ)
    }
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

pub fn orient_to_quat(orient: &[SignedAxis]) -> Quat {
    match orient.len() {
        0 => Quat::IDENTITY,
        1 => single_axis_rotation(orient[0]),
        3 => full_frame_rotation(orient[0], orient[1], orient[2]),
        _ => {
            bevy::log::warn!("orient must have 0, 1, or 3 axes, got {}", orient.len());
            Quat::IDENTITY
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

fn full_frame_rotation(right: SignedAxis, up: SignedAxis, forward: SignedAxis) -> Quat {
    let r = signed_axis_to_vec3(right);
    let u = signed_axis_to_vec3(up);
    let f = signed_axis_to_vec3(forward);
    let mat = Mat3::from_cols(r, u, f);
    Quat::from_mat3(&mat)
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

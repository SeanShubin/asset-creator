//! Pure-integer shape specification.
//!
//! This module owns the authored data model: the `SpecNode` tree loaded from
//! `.shape.ron` files and every operation that manipulates authored shape
//! data. Everything here is integer. No `f32`, no `Mat3`, no `Transform`,
//! no `Vec3`. The single float conversion for camera positioning
//! (`Bounds::center_f32`) is deliberately isolated and used only by render
//! and camera-fit code — it is not part of the spec pipeline.
//!
//! The rendering layer (`super::render`) is the only consumer of `SpecNode`.
//! Nothing here imports `super::render`, so data flows in one direction:
//! file → spec → render → Bevy entities.

use serde::Deserialize;
use serde::de::{self, MapAccess, Visitor};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use crate::registry::AssetRegistry;
use crate::util::Color3;

// =====================================================================
// SpecNode — the authored shape tree
// =====================================================================

/// A node in the authored shape tree. A `.shape.ron` file IS a `SpecNode`.
#[derive(Deserialize, Clone, Debug)]
pub struct SpecNode {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub shape: Option<PrimitiveShape>,
    #[serde(default)]
    pub bounds: Option<Bounds>,
    #[serde(default)]
    pub orient: Orientation,
    #[serde(default, deserialize_with = "deserialize_ordered_map")]
    pub palette: Vec<(String, Color3)>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub emissive: bool,
    #[serde(default)]
    pub import: Option<String>,
    #[serde(default)]
    pub color_map: HashMap<String, String>,
    #[serde(default)]
    pub colors: Vec<String>,
    #[serde(default)]
    pub children: Vec<SpecNode>,
    #[serde(default)]
    pub mirror: Vec<MirrorAxis>,
    #[serde(default)]
    pub combine: CombineMode,
    #[serde(default)]
    pub animations: Vec<AnimState>,
    /// Mirror axes this node has been reflected along via mirror expansion.
    /// Never read from the file — populated only during
    /// `expand_mirror_children` and consumed by the render layer when
    /// composing the final orientation matrix.
    #[serde(skip)]
    pub reflected_axes: Vec<MirrorAxis>,
}

impl SpecNode {
    /// Determine what kind of combinator this node is.
    /// A node is at most one combinator type; priority: mirror > import.
    pub fn combinator(&self) -> Combinator<'_> {
        if !self.mirror.is_empty() {
            Combinator::Mirror(&self.mirror)
        } else if let Some(ref import) = self.import {
            Combinator::Import(import)
        } else {
            Combinator::None
        }
    }

    /// Whether any children use CSG (Subtract or Clip combine modes).
    pub fn has_csg_children(&self) -> bool {
        self.children.iter().any(|c| c.combine != CombineMode::Union)
    }

    /// Whether this node is a combinator (mirror or import).
    /// Combinators are pass-through containers — they don't have their own position.
    pub fn is_combinator(&self) -> bool {
        !matches!(self.combinator(), Combinator::None)
    }

    /// Compute the AABB enclosing this node and all descendants.
    /// Integer arithmetic throughout.
    pub fn compute_aabb(&self) -> Option<Bounds> {
        let mut min = (i32::MAX, i32::MAX, i32::MAX);
        let mut max = (i32::MIN, i32::MIN, i32::MIN);
        let mut found = false;

        self.collect_bounds(&mut min, &mut max, &mut found);

        if found {
            Some(Bounds(min.0, min.1, min.2, max.0, max.1, max.2))
        } else {
            None
        }
    }

    pub(super) fn collect_bounds(
        &self,
        min: &mut (i32, i32, i32),
        max: &mut (i32, i32, i32),
        found: &mut bool,
    ) {
        if let Some(b) = &self.bounds {
            let b_min = b.min();
            let b_max = b.max();

            // Enumerate every subset of the mirror list and apply its
            // composition in list order. For orthogonal axes
            // (`X`/`Y`/`Z`) individual reflections would suffice — they
            // commute and each subset's AABB is just the convex hull of
            // single-axis reflections. But the diagonal axes
            // (`XY`/`XZ`/`YZ`) don't commute with the orthogonal ones, so
            // cross-term copies like `{X, XY}` visit quadrants no single
            // reflection reaches. We enumerate all 2^n subsets to cover
            // every copy, and since `mirror.len()` is bounded at 6 the
            // cost is at most 64 transforms per node.
            let combos = mirror_combinations(&self.mirror);
            for (subset, _) in &combos {
                let (sub_min, sub_max) = compose_mirror_sequence(b_min, b_max, subset);
                include_point(min, max, sub_min, found);
                include_point(min, max, sub_max, found);
            }
        }
        for child in &self.children {
            child.collect_bounds(min, max, found);
        }
    }

    /// Remap all bounds in this node and its descendants from one coordinate
    /// space to another. Pure integer multiplication — no division, no
    /// rounding, no precision loss. The result lives in a coordinate space
    /// scaled by from_size per axis; the render compile step divides by the
    /// accumulated scale when converting to world floats.
    pub fn remap_bounds(&mut self, from: &Bounds, to: &Bounds) {
        if let Some(ref mut b) = self.bounds {
            *b = b.remap(from, to);
        }
        for child in &mut self.children {
            child.remap_bounds(from, to);
        }
    }

    /// Validate static invariants of this spec tree. Returns a description
    /// of the first violation found, or `Ok(())` if the tree is clean.
    /// Currently checks: mirror lists cannot contain all three diagonal
    /// planes (XY, XZ, YZ), because that combination produces duplicate
    /// transforms due to the group relation in S_3.
    pub fn validate(&self) -> Result<(), SpecValidationError> {
        self.validate_impl("")
    }

    fn validate_impl(&self, path: &str) -> Result<(), SpecValidationError> {
        let node_path = match &self.name {
            Some(name) => {
                if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}/{name}")
                }
            }
            None => path.to_string(),
        };

        let has_xy = self.mirror.contains(&MirrorAxis::XY);
        let has_xz = self.mirror.contains(&MirrorAxis::XZ);
        let has_yz = self.mirror.contains(&MirrorAxis::YZ);
        if has_xy && has_xz && has_yz {
            return Err(SpecValidationError::AllThreeDiagonals {
                path: node_path,
            });
        }

        for child in &self.children {
            child.validate_impl(&node_path)?;
        }
        Ok(())
    }
}

/// Errors produced by `SpecNode::validate`.
#[derive(Debug, Clone)]
pub enum SpecValidationError {
    /// A mirror list contains all three diagonal planes (XY, XZ, YZ).
    /// This always produces duplicate transforms: the three transpositions
    /// of S_3 have a non-trivial relation (any two generate the same group
    /// as all three), so 2^3 = 8 subsets can only map to 6 distinct
    /// permutations, leaving two pairs of coincident copies.
    AllThreeDiagonals { path: String },
}

impl std::fmt::Display for SpecValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpecValidationError::AllThreeDiagonals { path } => {
                write!(
                    f,
                    "node '{path}' has mirror list containing all three diagonal planes (XY, XZ, YZ); \
                     this combination produces duplicate mirror copies and is never valid"
                )
            }
        }
    }
}

impl std::error::Error for SpecValidationError {}

fn include_point(
    min: &mut (i32, i32, i32),
    max: &mut (i32, i32, i32),
    p: (i32, i32, i32),
    found: &mut bool,
) {
    min.0 = min.0.min(p.0);
    min.1 = min.1.min(p.1);
    min.2 = min.2.min(p.2);
    max.0 = max.0.max(p.0);
    max.1 = max.1.max(p.1);
    max.2 = max.2.max(p.2);
    *found = true;
}

/// Compose a sequence of mirror transformations onto an AABB, applying
/// each axis in order. Used by `collect_bounds` to compute the full
/// enclosing AABB across every mirror subset. Each individual
/// `reflect_mirror_extents` call preserves ordering (min ≤ max), so the
/// chained result is still a valid canonicalized AABB.
pub(super) fn compose_mirror_sequence(
    b_min: (i32, i32, i32),
    b_max: (i32, i32, i32),
    axes: &[MirrorAxis],
) -> ((i32, i32, i32), (i32, i32, i32)) {
    let mut cur_min = b_min;
    let mut cur_max = b_max;
    for &axis in axes {
        let (new_min, new_max) = reflect_mirror_extents(cur_min, cur_max, axis);
        cur_min = new_min;
        cur_max = new_max;
    }
    (cur_min, cur_max)
}

/// Compute the AABB of a mirror copy for the given mirror axis. X/Y/Z flip
/// the corresponding coordinate; XY/XZ/YZ swap the corresponding coordinate
/// pair (reflection across the diagonal plane y=x, z=x, z=y respectively).
pub(super) fn reflect_mirror_extents(
    b_min: (i32, i32, i32),
    b_max: (i32, i32, i32),
    axis: MirrorAxis,
) -> ((i32, i32, i32), (i32, i32, i32)) {
    match axis {
        MirrorAxis::X => ((-b_max.0, b_min.1, b_min.2), (-b_min.0, b_max.1, b_max.2)),
        MirrorAxis::Y => ((b_min.0, -b_max.1, b_min.2), (b_max.0, -b_min.1, b_max.2)),
        MirrorAxis::Z => ((b_min.0, b_min.1, -b_max.2), (b_max.0, b_max.1, -b_min.2)),
        MirrorAxis::XY => ((b_min.1, b_min.0, b_min.2), (b_max.1, b_max.0, b_max.2)),
        MirrorAxis::XZ => ((b_min.2, b_min.1, b_min.0), (b_max.2, b_max.1, b_max.0)),
        MirrorAxis::YZ => ((b_min.0, b_min.2, b_min.1), (b_max.0, b_max.2, b_max.1)),
    }
}

// =====================================================================
// Combine mode
// =====================================================================

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum CombineMode {
    #[default]
    Union,
    Subtract,
    Clip,
}

/// What kind of combinator this node is, if any.
pub enum Combinator<'a> {
    Mirror(&'a [MirrorAxis]),
    Import(&'a str),
    None,
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

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds(pub i32, pub i32, pub i32, pub i32, pub i32, pub i32);

impl Bounds {
    /// Compute the AABB enclosing a list of spec nodes.
    pub fn enclosing(nodes: &[SpecNode]) -> Option<Bounds> {
        let mut min = (i32::MAX, i32::MAX, i32::MAX);
        let mut max = (i32::MIN, i32::MIN, i32::MIN);
        let mut found = false;
        for node in nodes {
            node.collect_bounds(&mut min, &mut max, &mut found);
        }
        if found {
            Some(Bounds(min.0, min.1, min.2, max.0, max.1, max.2))
        } else {
            None
        }
    }

    /// Center as float — used for camera positioning and render export.
    /// This is the one float escape hatch and it is NEVER called during
    /// spec-side processing.
    pub fn center_f32(&self) -> (f32, f32, f32) {
        (
            (self.0 + self.3) as f32 / 2.0,
            (self.1 + self.4) as f32 / 2.0,
            (self.2 + self.5) as f32 / 2.0,
        )
    }

    pub fn size(&self) -> (i32, i32, i32) {
        (
            (self.3 - self.0).abs(),
            (self.4 - self.1).abs(),
            (self.5 - self.2).abs(),
        )
    }

    pub fn min(&self) -> (i32, i32, i32) {
        (self.0.min(self.3), self.1.min(self.4), self.2.min(self.5))
    }

    pub fn max(&self) -> (i32, i32, i32) {
        (self.0.max(self.3), self.1.max(self.4), self.2.max(self.5))
    }

    /// Remap this bounds from `from` to `to` coordinate space using only
    /// integer multiplication. No division, no rounding, no precision loss.
    /// The result is in units scaled by from_size per axis.
    /// Formula: result = to_min * from_size + (val - from_min) * to_size
    pub fn remap(&self, from: &Bounds, to: &Bounds) -> Bounds {
        let from_min = from.min();
        let from_size = from.size();
        let to_min = to.min();
        let to_size = to.size();

        let remap = |val: i32, from_min: i32, from_size: i32, to_min: i32, to_size: i32| -> i32 {
            if from_size == 0 {
                to_min
            } else {
                to_min * from_size + (val - from_min) * to_size
            }
        };

        Bounds(
            remap(self.0, from_min.0, from_size.0, to_min.0, to_size.0),
            remap(self.1, from_min.1, from_size.1, to_min.1, to_size.1),
            remap(self.2, from_min.2, from_size.2, to_min.2, to_size.2),
            remap(self.3, from_min.0, from_size.0, to_min.0, to_size.0),
            remap(self.4, from_min.1, from_size.1, to_min.1, to_size.1),
            remap(self.5, from_min.2, from_size.2, to_min.2, to_size.2),
        )
    }

    /// Returns the per-axis scale factor introduced by a remap from `from`.
    /// This is from_size — the denominator the caller must accumulate.
    pub fn remap_scale(from: &Bounds) -> (i32, i32, i32) {
        let s = from.size();
        (s.0.max(1), s.1.max(1), s.2.max(1))
    }
}

// =====================================================================
// Axes
// =====================================================================

/// Coordinate axis, used for things like animation channels. This is
/// distinct from `MirrorAxis` — a coordinate axis is a direction, a mirror
/// axis is a reflection plane (including diagonal planes).
#[derive(Deserialize, Clone, Copy, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// Reflection plane used by the mirror combinator.
///
/// - `X` / `Y` / `Z` reflect across the plane perpendicular to that
///   coordinate axis (i.e. the `x = 0`, `y = 0`, or `z = 0` plane).
/// - `XY` / `XZ` / `YZ` reflect across the diagonal plane that swaps the
///   corresponding coordinate pair (i.e. `y = x`, `z = x`, or `z = y`).
///
/// The three diagonal variants have a non-trivial group relation: any
/// mirror list containing all three produces duplicate copies. The spec
/// validator rejects such lists.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorAxis {
    X,
    Y,
    Z,
    XY,
    XZ,
    YZ,
}

// =====================================================================
// Orientation — the authored discrete orientation tuple
// =====================================================================

/// Authored orientation: a discrete (facing, mirroring, rotation) combination.
/// The rendering layer converts this to a `Mat3` when it needs to compute
/// a mesh transform. Storing the tuple instead of the derived matrix keeps
/// the spec integer-pure.
#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub struct Orientation(
    #[serde(default)] pub Facing,
    #[serde(default)] pub Mirroring,
    #[serde(default)] pub Rotation,
);

impl Orientation {
    pub fn facing(self) -> Facing {
        self.0
    }
    pub fn mirroring(self) -> Mirroring {
        self.1
    }
    pub fn rotation(self) -> Rotation {
        self.2
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

// =====================================================================
// Animation data — carried through the spec but never interpreted here
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

// =====================================================================
// Combinator expansion helpers — pure integer
// =====================================================================

/// Enumerate all 2^n subsets of the given mirror axes, returning each
/// subset paired with a suffix string for name tagging.
pub(super) fn mirror_combinations(
    axes: &[MirrorAxis],
) -> Vec<(Vec<MirrorAxis>, String)> {
    let n = axes.len();
    let count = 1 << n;
    let mut result = Vec::with_capacity(count);
    for bits in 0..count {
        let mut flipped = Vec::new();
        let mut suffix = String::new();
        for (i, &axis) in axes.iter().enumerate() {
            if bits & (1 << i) != 0 {
                flipped.push(axis);
                suffix.push_str(mirror_axis_suffix(axis));
            }
        }
        let suffix = if suffix.is_empty() {
            String::new()
        } else {
            format!("m{suffix}")
        };
        result.push((flipped, suffix));
    }
    result
}

fn mirror_axis_suffix(axis: MirrorAxis) -> &'static str {
    match axis {
        MirrorAxis::X => "x",
        MirrorAxis::Y => "y",
        MirrorAxis::Z => "z",
        MirrorAxis::XY => "xy",
        MirrorAxis::XZ => "xz",
        MirrorAxis::YZ => "yz",
    }
}

/// Recursively apply a mirror-axis bounds transformation to the node and
/// its entire subtree. For `X`/`Y`/`Z` this negates the corresponding pair
/// of integer coordinates; for `XY`/`XZ`/`YZ` it swaps the corresponding
/// pair of coordinates (reflection across the diagonal plane).
pub(super) fn flip_bounds(node: &mut SpecNode, axis: MirrorAxis) {
    warn_if_missing_bounds(node);
    if let Some(ref mut b) = node.bounds {
        match axis {
            MirrorAxis::X => {
                let tmp = -b.0;
                b.0 = -b.3;
                b.3 = tmp;
            }
            MirrorAxis::Y => {
                let tmp = -b.1;
                b.1 = -b.4;
                b.4 = tmp;
            }
            MirrorAxis::Z => {
                let tmp = -b.2;
                b.2 = -b.5;
                b.5 = tmp;
            }
            MirrorAxis::XY => {
                std::mem::swap(&mut b.0, &mut b.1);
                std::mem::swap(&mut b.3, &mut b.4);
            }
            MirrorAxis::XZ => {
                std::mem::swap(&mut b.0, &mut b.2);
                std::mem::swap(&mut b.3, &mut b.5);
            }
            MirrorAxis::YZ => {
                std::mem::swap(&mut b.1, &mut b.2);
                std::mem::swap(&mut b.4, &mut b.5);
            }
        }
    }
    for child in &mut node.children {
        flip_bounds(child, axis);
    }
}

/// Recursively push an extra reflection axis onto every node in the subtree
/// that has a shape. The render layer consumes `reflected_axes` when it
/// computes the final orientation matrix for each geometry node.
pub(super) fn push_reflection(node: &mut SpecNode, axis: MirrorAxis) {
    if node.shape.is_some() {
        node.reflected_axes.push(axis);
    }
    for child in &mut node.children {
        push_reflection(child, axis);
    }
}

fn warn_if_missing_bounds(node: &SpecNode) {
    if node.bounds.is_none() && node.shape.is_some() {
        bevy::prelude::warn!(
            "Shape '{}' has no bounds — every shape must specify bounds",
            node.name.as_deref().unwrap_or("unnamed")
        );
    }
}

/// Expand mirror combinators on a list of children into a flat list of
/// pre-mirrored `SpecNode`s. Each copy has its integer bounds flipped and
/// its `reflected_axes` populated so that the render layer can derive the
/// correct orientation matrix.
///
/// This is used by CSG rebuild to flatten children into the same sequence
/// the render walker would produce, without running the render walker itself.
pub fn expand_mirror_children(children: &[SpecNode]) -> Vec<SpecNode> {
    let mut result = Vec::new();
    for child in children {
        match child.combinator() {
            Combinator::Mirror(axes) => {
                let mut base = child.clone();
                base.mirror = Vec::new();
                for (flipped_axes, suffix) in &mirror_combinations(axes) {
                    let mut copy = base.clone();
                    for &axis in flipped_axes {
                        flip_bounds(&mut copy, axis);
                    }
                    for &axis in flipped_axes {
                        push_reflection(&mut copy, axis);
                    }
                    if !suffix.is_empty() {
                        if let Some(ref name) = copy.name {
                            copy.name = Some(format!("{name}_{suffix}"));
                        }
                    }
                    result.push(copy);
                }
            }
            _ => {
                result.push(child.clone());
            }
        }
    }
    result
}

// =====================================================================
// Cell occupancy — integer-space scene index
// =====================================================================

/// A unit cell in integer world space. Every primitive instance claims
/// one or more of these.
pub type CellPos = (i32, i32, i32);

/// A single cell collision: two primitives (identified by their authored
/// path from the root of the spec tree) both claim the same cell.
#[derive(Debug, Clone)]
pub struct Collision {
    pub first_path: String,
    pub second_path: String,
    pub cell: CellPos,
}

/// Global cell-level index of a compiled shape. Every primitive instance
/// (post-mirror-expansion, post-import-remapping) contributes its
/// integer cells to the index. The resulting structure supports two
/// queries in O(1) per cell:
///
/// 1. **AABB**: min and max over all claimed cells.
/// 2. **Collision detection**: any cell that more than one primitive
///    tried to claim is recorded as a `Collision`.
///
/// The cell model is the long-term source of truth for shape geometry
/// and will eventually replace the subset-based AABB path. For now both
/// coexist: `SpecNode::compute_aabb` still uses subset enumeration and
/// is called on the unexpanded tree (no registry), while `Occupancy`
/// handles the global scene and participates in validation.
pub struct Occupancy {
    cells: HashMap<CellPos, String>,
    collisions: Vec<Collision>,
}

impl Occupancy {
    fn new() -> Self {
        Self {
            cells: HashMap::new(),
            collisions: Vec::new(),
        }
    }

    /// Number of collisions detected. Zero means the spec satisfies the
    /// cell-uniqueness invariant.
    pub fn collision_count(&self) -> usize {
        self.collisions.len()
    }

    /// Full list of cell collisions. Each entry names the two primitives
    /// that both claimed the cell plus the cell coordinate.
    pub fn collisions(&self) -> &[Collision] {
        &self.collisions
    }

    /// AABB enclosing all occupied cells. Returns `None` if nothing was
    /// placed. Each occupied cell `(x, y, z)` occupies the unit cube
    /// `(x, y, z)` to `(x+1, y+1, z+1)`, so the result's max components
    /// are one greater than the max cell coordinate.
    pub fn aabb(&self) -> Option<Bounds> {
        let mut iter = self.cells.keys();
        let first = *iter.next()?;
        let mut mn = first;
        let mut mx = first;
        for &(x, y, z) in iter {
            if x < mn.0 {
                mn.0 = x;
            }
            if y < mn.1 {
                mn.1 = y;
            }
            if z < mn.2 {
                mn.2 = z;
            }
            if x > mx.0 {
                mx.0 = x;
            }
            if y > mx.1 {
                mx.1 = y;
            }
            if z > mx.2 {
                mx.2 = z;
            }
        }
        Some(Bounds(mn.0, mn.1, mn.2, mx.0 + 1, mx.1 + 1, mx.2 + 1))
    }

    fn claim(&mut self, cell: CellPos, path: &str) {
        match self.cells.entry(cell) {
            Entry::Vacant(e) => {
                e.insert(path.to_string());
            }
            Entry::Occupied(e) => {
                self.collisions.push(Collision {
                    first_path: e.get().clone(),
                    second_path: path.to_string(),
                    cell,
                });
            }
        }
    }
}

/// Walk a `SpecNode` tree, expanding mirror combinators and imports,
/// and build an `Occupancy` index of every cell claimed by every
/// primitive instance.
///
/// Runs in integer arithmetic only. Cell positions are world cells —
/// for shapes containing imports, the per-import scale is tracked and
/// scaled integer bounds are divided down to world cells at each leaf.
/// The `remap_bounds` invariant (scaled values are always exact multiples
/// of the scale factor) guarantees the division is lossless.
pub fn collect_occupancy(spec: &SpecNode, registry: &AssetRegistry) -> Occupancy {
    let mut occ = Occupancy::new();
    walk_for_occupancy(&mut occ, spec, "", (1, 1, 1), registry);
    occ
}

fn walk_for_occupancy(
    occ: &mut Occupancy,
    node: &SpecNode,
    parent_path: &str,
    scale: (i32, i32, i32),
    registry: &AssetRegistry,
) {
    let base_path = append_path(parent_path, node.name.as_deref());

    match node.combinator() {
        Combinator::Mirror(axes) => {
            let mut base = node.clone();
            base.mirror = Vec::new();
            for (flipped, suffix) in &mirror_combinations(axes) {
                let mut copy = base.clone();
                for &axis in flipped {
                    flip_bounds(&mut copy, axis);
                }
                for &axis in flipped {
                    push_reflection(&mut copy, axis);
                }
                let copy_path = if suffix.is_empty() {
                    base_path.clone()
                } else {
                    format!("{base_path}_{suffix}")
                };
                walk_for_occupancy(occ, &copy, &copy_path, scale, registry);
            }
        }
        Combinator::Import(import_name) => {
            let Some(imported) = registry.get_shape(import_name) else {
                // Missing imports are reported elsewhere (render path);
                // occupancy silently skips them so the HUD doesn't
                // report phantom collisions for a broken import.
                return;
            };
            let Some(native_aabb) = imported.compute_aabb() else {
                return;
            };
            let placement = node.bounds.unwrap_or(native_aabb);

            let remap_scale = Bounds::remap_scale(&native_aabb);
            let new_scale = (
                scale.0 * remap_scale.0,
                scale.1 * remap_scale.1,
                scale.2 * remap_scale.2,
            );

            let mut remapped = imported.clone();
            remapped.remap_bounds(&native_aabb, &placement);
            walk_for_occupancy(occ, &remapped, &base_path, new_scale, registry);
        }
        Combinator::None => {
            if let (Some(_), Some(bounds)) = (node.shape, node.bounds.as_ref()) {
                claim_cells(occ, bounds, scale, &base_path);
            }
            for child in &node.children {
                walk_for_occupancy(occ, child, &base_path, scale, registry);
            }
        }
    }
}

fn claim_cells(
    occ: &mut Occupancy,
    bounds: &Bounds,
    scale: (i32, i32, i32),
    path: &str,
) {
    let mn = bounds.min();
    let mx = bounds.max();
    // Divide scaled integer bounds down to world cells. When the primitive
    // is cell-aligned (no fractional position), floor_div and ceil_div
    // both return the exact cell. When the primitive sits at sub-cell
    // positions — which happens inside imports whose placement-to-native
    // size ratio is not an integer — we round outward: floor for min,
    // ceil for max. That conservatively over-claims the smallest integer
    // cell box containing the primitive. This may produce false-positive
    // collisions at cell boundaries between two non-aligned imports, but
    // never false negatives, which is the right tradeoff for the
    // informational HUD stat. The long-term cell-level architecture will
    // disallow non-integer scaling imports entirely.
    let wmin = (
        floor_div(mn.0, scale.0),
        floor_div(mn.1, scale.1),
        floor_div(mn.2, scale.2),
    );
    let wmax = (
        ceil_div(mx.0, scale.0),
        ceil_div(mx.1, scale.1),
        ceil_div(mx.2, scale.2),
    );

    for z in wmin.2..wmax.2 {
        for y in wmin.1..wmax.1 {
            for x in wmin.0..wmax.0 {
                occ.claim((x, y, z), path);
            }
        }
    }
}

/// Floor division for a signed dividend and a strictly positive divisor.
/// Unlike Rust's `/`, this rounds toward negative infinity, which is what
/// we want when computing the minimum cell an integer position belongs to.
fn floor_div(a: i32, b: i32) -> i32 {
    debug_assert!(b > 0, "scale must be positive");
    let q = a / b;
    let r = a % b;
    if r < 0 {
        q - 1
    } else {
        q
    }
}

/// Ceiling division for a signed dividend and a strictly positive divisor.
/// Rounds toward positive infinity, used when computing the exclusive
/// upper cell bound.
fn ceil_div(a: i32, b: i32) -> i32 {
    debug_assert!(b > 0, "scale must be positive");
    let q = a / b;
    let r = a % b;
    if r > 0 {
        q + 1
    } else {
        q
    }
}

fn append_path(parent: &str, name: Option<&str>) -> String {
    match (parent.is_empty(), name) {
        (true, Some(n)) => n.to_string(),
        (true, None) => String::new(),
        (false, Some(n)) => format!("{parent}/{n}"),
        (false, None) => parent.to_string(),
    }
}

// =====================================================================
// Serde helpers
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf_spec(bounds: Bounds, mirror: Vec<MirrorAxis>) -> SpecNode {
        SpecNode {
            name: Some("leaf".into()),
            shape: Some(PrimitiveShape::Box),
            bounds: Some(bounds),
            orient: Orientation::default(),
            palette: vec![],
            color: None,
            emissive: false,
            import: None,
            color_map: HashMap::new(),
            colors: vec![],
            children: vec![],
            mirror,
            combine: CombineMode::Union,
            animations: vec![],
            reflected_axes: vec![],
        }
    }

    /// `[X, Y, XY]` should produce 8 copies with 8 distinct AABBs,
    /// forming the D_4 symmetry group of a square in the XY plane.
    #[test]
    fn d4_mirror_produces_eight_unique_copies() {
        let mut seen = std::collections::HashSet::new();
        let combos =
            mirror_combinations(&[MirrorAxis::X, MirrorAxis::Y, MirrorAxis::XY]);
        assert_eq!(combos.len(), 8);
        for (flipped, _suffix) in &combos {
            let mut node = leaf_spec(Bounds(1, 3, 0, 2, 5, 1), vec![]);
            for &axis in flipped {
                flip_bounds(&mut node, axis);
            }
            let b = node.bounds.unwrap();
            assert!(
                seen.insert((b.min(), b.max())),
                "duplicate AABB for mirror subset {:?}",
                flipped
            );
        }
        assert_eq!(seen.len(), 8);
    }

    /// `[XY, XZ, YZ]` collides: 8 subsets → 6 unique permutations.
    #[test]
    fn three_diagonals_rejected_by_validate() {
        let spec = leaf_spec(
            Bounds(1, 3, 0, 2, 5, 1),
            vec![MirrorAxis::XY, MirrorAxis::XZ, MirrorAxis::YZ],
        );
        let err = spec.validate().expect_err("should reject three diagonals");
        assert!(matches!(
            err,
            SpecValidationError::AllThreeDiagonals { .. }
        ));
    }

    /// Any 2-diagonal subset should be accepted.
    #[test]
    fn two_diagonals_accepted() {
        for combo in &[
            vec![MirrorAxis::XY, MirrorAxis::XZ],
            vec![MirrorAxis::XY, MirrorAxis::YZ],
            vec![MirrorAxis::XZ, MirrorAxis::YZ],
        ] {
            let spec = leaf_spec(Bounds(1, 3, 0, 2, 5, 1), combo.clone());
            assert!(spec.validate().is_ok(), "rejected valid combo {:?}", combo);
        }
    }

    /// `collect_bounds` must enumerate all 2^n mirror subsets when the
    /// list contains diagonals. Single-axis union is wrong because
    /// cross-term copies like `{X, XY}` visit quadrants no individual
    /// reflection reaches. Regression for the "blue grid two cells too
    /// close" bug in cornered_cube.shape.ron.
    #[test]
    fn mixed_axis_and_diagonal_mirror_aabb_covers_cross_terms() {
        let spec = leaf_spec(
            Bounds(1, -1, -1, 3, 1, 1),
            vec![MirrorAxis::X, MirrorAxis::XY, MirrorAxis::XZ],
        );
        let aabb = spec.compute_aabb().expect("should produce an aabb");
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    /// A lone cell-aligned box with no mirror claims exactly its
    /// integer cells and reports no collisions.
    #[test]
    fn occupancy_lone_box_has_no_collisions() {
        let spec = leaf_spec(Bounds(0, 0, 0, 2, 2, 2), vec![]);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        assert_eq!(
            occ.aabb(),
            Some(Bounds(0, 0, 0, 2, 2, 2)),
            "aabb should wrap exactly the claimed cells"
        );
    }

    /// Two boxes that share no cells produce no collisions, and the
    /// AABB spans the union.
    #[test]
    fn occupancy_two_disjoint_boxes() {
        let child_a = leaf_spec(Bounds(0, 0, 0, 2, 2, 2), vec![]);
        let mut child_b = leaf_spec(Bounds(3, 0, 0, 5, 2, 2), vec![]);
        child_b.name = Some("b".into());
        let mut parent = leaf_spec(Bounds(0, 0, 0, 1, 1, 1), vec![]);
        parent.shape = None;
        parent.bounds = None;
        parent.name = Some("parent".into());
        parent.children = vec![child_a, child_b];

        let occ = collect_occupancy(&parent, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        assert_eq!(occ.aabb(), Some(Bounds(0, 0, 0, 5, 2, 2)));
    }

    /// Two boxes whose bounds overlap trigger a collision with both
    /// paths recorded.
    #[test]
    fn occupancy_overlapping_boxes_collide() {
        let mut child_a = leaf_spec(Bounds(0, 0, 0, 3, 2, 2), vec![]);
        child_a.name = Some("a".into());
        let mut child_b = leaf_spec(Bounds(2, 0, 0, 5, 2, 2), vec![]);
        child_b.name = Some("b".into());
        let mut parent = leaf_spec(Bounds(0, 0, 0, 1, 1, 1), vec![]);
        parent.shape = None;
        parent.bounds = None;
        parent.name = Some("parent".into());
        parent.children = vec![child_a, child_b];

        let occ = collect_occupancy(&parent, &AssetRegistry::default());
        // Overlap region is (2..3, 0..2, 0..2) = 4 cells, each a collision.
        assert_eq!(occ.collision_count(), 4);
        let c = &occ.collisions()[0];
        assert!(c.first_path.ends_with("/a") || c.first_path == "a");
        assert!(c.second_path.ends_with("/b") || c.second_path == "b");
    }

    /// A centered box with mirror [X] produces two overlapping copies
    /// (each one gets the whole AABB), so every cell is a collision.
    /// This is the motivating "centered sphere mirrored across X" case.
    #[test]
    fn occupancy_centered_mirror_collides_with_itself() {
        let spec = leaf_spec(Bounds(-1, -1, -1, 1, 1, 1), vec![MirrorAxis::X]);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        // Both copies land on the same 8 cells.
        assert_eq!(occ.collision_count(), 8);
    }

    /// An off-origin box with mirror [X] produces two copies on
    /// opposite sides with no overlap.
    #[test]
    fn occupancy_off_origin_mirror_does_not_collide() {
        let spec = leaf_spec(Bounds(2, 0, 0, 3, 1, 1), vec![MirrorAxis::X]);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        assert_eq!(occ.aabb(), Some(Bounds(-3, 0, 0, 3, 1, 1)));
    }

    /// The D_4 diagonal mirror test shape produces 8 unique copies;
    /// with asymmetric bounds they should all be disjoint and cell
    /// occupancy should report zero collisions.
    #[test]
    fn occupancy_d4_diagonal_mirror_no_collision() {
        let spec = leaf_spec(
            Bounds(1, 3, 0, 2, 5, 1),
            vec![MirrorAxis::X, MirrorAxis::Y, MirrorAxis::XY],
        );
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        // 8 copies × 2 cells each (the 1×2×1 box).
        let aabb = occ.aabb().expect("should have cells");
        // AABB spans ±5 on x and y, 0..1 on z.
        assert_eq!(aabb.min(), (-5, -5, 0));
        assert_eq!(aabb.max(), (5, 5, 1));
    }

    /// Non-integer scaling during import remap (e.g. a native-size-16 axis
    /// placed in a size-9 slot) used to trip a divisibility assertion in
    /// `claim_cells`. Regression check: the same configuration must be
    /// handled without panicking and must over-claim conservatively.
    #[test]
    fn claim_cells_handles_non_integer_scale() {
        // Scale=(4, 4, 16); bounds=(8, -4, -48, 12, 0, 24) mirrors the
        // remapped block-e/melee body. 24/16 = 1.5 — sub-cell position
        // in z, which must not panic.
        let mut occ = Occupancy::new();
        let scale = (4, 4, 16);
        let bounds = Bounds(8, -4, -48, 12, 0, 24);
        claim_cells(&mut occ, &bounds, scale, "leaf");
        // z: floor(-48/16)=-3, ceil(24/16)=2 → cells z=-3..2 (5 cells)
        // x: floor(8/4)=2, ceil(12/4)=3 → cells x=2..3 (1 cell)
        // y: floor(-4/4)=-1, ceil(0/4)=0 → cells y=-1..0 (1 cell)
        // Total: 1 * 1 * 5 = 5 cells
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (2, -1, -3));
        assert_eq!(aabb.max(), (3, 0, 2));
    }

    #[test]
    fn floor_div_handles_signs() {
        assert_eq!(floor_div(10, 3), 3);
        assert_eq!(floor_div(-10, 3), -4);
        assert_eq!(floor_div(0, 3), 0);
        assert_eq!(floor_div(9, 3), 3);
        assert_eq!(floor_div(-9, 3), -3);
    }

    #[test]
    fn ceil_div_handles_signs() {
        assert_eq!(ceil_div(10, 3), 4);
        assert_eq!(ceil_div(-10, 3), -3);
        assert_eq!(ceil_div(0, 3), 0);
        assert_eq!(ceil_div(9, 3), 3);
        assert_eq!(ceil_div(-9, 3), -3);
    }

    /// The original `[X, Y, Z]` should still work and produce 8 copies
    /// at the 8 octants (regression check).
    #[test]
    fn orthogonal_mirror_still_works() {
        let mut seen = std::collections::HashSet::new();
        let combos =
            mirror_combinations(&[MirrorAxis::X, MirrorAxis::Y, MirrorAxis::Z]);
        assert_eq!(combos.len(), 8);
        for (flipped, _) in &combos {
            let mut node = leaf_spec(Bounds(3, 3, 3, 4, 4, 4), vec![]);
            for &axis in flipped {
                flip_bounds(&mut node, axis);
            }
            let b = node.bounds.unwrap();
            assert!(seen.insert((b.min(), b.max())));
        }
        assert_eq!(seen.len(), 8);
    }
}

/// Deserialize a RON map into a Vec preserving insertion order.
fn deserialize_ordered_map<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(String, Color3)>, D::Error> {
    struct OrderedMapVisitor;

    impl<'de> Visitor<'de> for OrderedMapVisitor {
        type Value = Vec<(String, Color3)>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map of string to color")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut entries = Vec::new();
            while let Some((key, value)) = map.next_entry::<String, Color3>()? {
                entries.push((key, value));
            }
            Ok(entries)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }
    }

    deserializer.deserialize_map(OrderedMapVisitor)
}

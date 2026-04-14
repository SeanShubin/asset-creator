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
    /// Named symmetry pattern. `Single` means "render once, no copies".
    /// All other variants produce multiple copies via a hand-curated
    /// placement table (see `placements`). There is no generator-list
    /// composition: each variant's placement set is validated by
    /// construction to contain no duplicates.
    #[serde(default)]
    pub symmetry: Symmetry,
    #[serde(default)]
    pub combine: CombineMode,
    #[serde(default)]
    pub animations: Vec<AnimState>,
}

impl SpecNode {
    /// Determine what kind of combinator this node is.
    /// A node is at most one combinator type; priority: symmetry > import.
    pub fn combinator(&self) -> Combinator<'_> {
        if self.symmetry != Symmetry::Single {
            Combinator::Symmetry(self.symmetry)
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

    /// Compute the AABB enclosing this node and all descendants.
    /// Integer arithmetic throughout. Symmetry expansion is handled by
    /// applying each placement to the node's bounds and including the
    /// result in the running min/max.
    pub fn compute_aabb(&self) -> Option<Bounds> {
        let mut min = (i32::MAX, i32::MAX, i32::MAX);
        let mut max = (i32::MIN, i32::MIN, i32::MIN);
        let mut found = false;

        self.collect_bounds(identity_placement(), &mut min, &mut max, &mut found);

        if found {
            Some(Bounds(min.0, min.1, min.2, max.0, max.1, max.2))
        } else {
            None
        }
    }

    pub(super) fn collect_bounds(
        &self,
        inherited: Placement,
        min: &mut (i32, i32, i32),
        max: &mut (i32, i32, i32),
        found: &mut bool,
    ) {
        // For each placement this node's symmetry produces, compose with the
        // inherited placement and walk the resulting transformed subtree.
        for (local, _) in placements(self.symmetry) {
            let combined = compose_placements(inherited, *local);
            self.collect_bounds_single(combined, min, max, found);
        }
    }

    fn collect_bounds_single(
        &self,
        placement: Placement,
        min: &mut (i32, i32, i32),
        max: &mut (i32, i32, i32),
        found: &mut bool,
    ) {
        if let Some(b) = &self.bounds {
            let transformed = apply_placement_to_bounds(placement, *b);
            let tmin = transformed.min();
            let tmax = transformed.max();
            include_point(min, max, tmin, found);
            include_point(min, max, tmax, found);
        }
        for child in &self.children {
            child.collect_bounds(placement, min, max, found);
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
    Symmetry(Symmetry),
    Import(&'a str),
    None,
}

// =====================================================================
// Primitives
// =====================================================================

#[derive(Deserialize, Clone, Copy, Debug)]
pub enum PrimitiveShape {
    Box,
    Wedge,
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
            node.collect_bounds(identity_placement(), &mut min, &mut max, &mut found);
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

// =====================================================================
// Axes
// =====================================================================

/// Coordinate axis, used for things like animation channels. This is
/// distinct from `SignedAxis` — a coordinate axis is a direction label,
/// a signed axis carries direction AND sign.
#[derive(Deserialize, Clone, Copy, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// A coordinate axis with a sign. Used as the components of a
/// `Placement`, which describes where each output axis draws from.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignedAxis {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl SignedAxis {
    fn axis_index(self) -> usize {
        match self {
            SignedAxis::PosX | SignedAxis::NegX => 0,
            SignedAxis::PosY | SignedAxis::NegY => 1,
            SignedAxis::PosZ | SignedAxis::NegZ => 2,
        }
    }

    fn is_positive(self) -> bool {
        matches!(self, SignedAxis::PosX | SignedAxis::PosY | SignedAxis::PosZ)
    }

    fn negate(self) -> SignedAxis {
        match self {
            SignedAxis::PosX => SignedAxis::NegX,
            SignedAxis::NegX => SignedAxis::PosX,
            SignedAxis::PosY => SignedAxis::NegY,
            SignedAxis::NegY => SignedAxis::PosY,
            SignedAxis::PosZ => SignedAxis::NegZ,
            SignedAxis::NegZ => SignedAxis::PosZ,
        }
    }
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
// Symmetry and Placements
// =====================================================================

/// Named symmetry pattern. Each variant expands to a curated list of
/// placements that produces distinct copies with no duplicates. The user
/// picks a pattern by name; there is no generator composition.
///
/// All copies are signed permutations of the coordinate axes — elements
/// of the cube symmetry group B₃ (order 48). The variants here are the
/// most commonly useful subsets for cell-grid authoring.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Symmetry {
    /// Identity only: 1 copy.
    #[default]
    Single,
    /// Mirror across the x=0 plane: 2 copies (±X).
    PairX,
    /// Mirror across the y=0 plane: 2 copies (±Y).
    PairY,
    /// Mirror across the z=0 plane: 2 copies (±Z).
    PairZ,
    /// Mirror across x=0 and y=0: 4 copies in the XY plane.
    QuadXY,
    /// Mirror across x=0 and z=0: 4 copies in the XZ plane.
    QuadXZ,
    /// Mirror across y=0 and z=0: 4 copies in the YZ plane.
    QuadYZ,
    /// Mirror across all three axis planes: 8 copies (all octants).
    Octants,
    /// The 6 face cells of a cube — one copy on each of ±X, ±Y, ±Z faces.
    Faces,
    /// The 12 edge cells of a cube — 4 edges parallel to each axis.
    Edges,
}

/// A signed permutation of the coordinate axes. `Placement(a, b, c)`
/// means: the new copy's world X axis comes from the source's `a`,
/// world Y from `b`, world Z from `c`. The `a`, `b`, `c` values are
/// `SignedAxis` variants that carry both axis and sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Placement(pub SignedAxis, pub SignedAxis, pub SignedAxis);

/// Identity placement: no change to the primitive.
pub const fn identity_placement() -> Placement {
    Placement(SignedAxis::PosX, SignedAxis::PosY, SignedAxis::PosZ)
}

/// Return the hand-curated list of `(placement, name_suffix)` pairs for
/// a given symmetry. Each placement produces one rendered copy. The
/// suffix is appended to the node's name to distinguish copies in the
/// entity tree for animation-channel lookup.
pub fn placements(symmetry: Symmetry) -> &'static [(Placement, &'static str)] {
    use SignedAxis::*;
    match symmetry {
        Symmetry::Single => &[(Placement(PosX, PosY, PosZ), "")],
        Symmetry::PairX => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
        ],
        Symmetry::PairY => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, NegY, PosZ), "_my"),
        ],
        Symmetry::PairZ => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, PosY, NegZ), "_mz"),
        ],
        Symmetry::QuadXY => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
            (Placement(PosX, NegY, PosZ), "_my"),
            (Placement(NegX, NegY, PosZ), "_mxy"),
        ],
        Symmetry::QuadXZ => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
            (Placement(PosX, PosY, NegZ), "_mz"),
            (Placement(NegX, PosY, NegZ), "_mxz"),
        ],
        Symmetry::QuadYZ => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, NegY, PosZ), "_my"),
            (Placement(PosX, PosY, NegZ), "_mz"),
            (Placement(PosX, NegY, NegZ), "_myz"),
        ],
        Symmetry::Octants => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
            (Placement(PosX, NegY, PosZ), "_my"),
            (Placement(NegX, NegY, PosZ), "_mxy"),
            (Placement(PosX, PosY, NegZ), "_mz"),
            (Placement(NegX, PosY, NegZ), "_mxz"),
            (Placement(PosX, NegY, NegZ), "_myz"),
            (Placement(NegX, NegY, NegZ), "_mxyz"),
        ],
        Symmetry::Faces => &[
            // Six face cells. Designed for a source box sitting on the
            // +X face of a symmetric cube: y-range and z-range of the
            // source are symmetric around origin. To move the +X face
            // to the ∓Y or ∓Z face, the source's offset (nonsymmetric)
            // X axis is routed to the target's offset slot with the
            // correct sign; the other two slots are filled by the
            // source's symmetric axes.
            (Placement(PosX, PosY, PosZ), "_px"),
            (Placement(NegX, PosY, PosZ), "_nx"),
            (Placement(PosY, PosX, PosZ), "_py"),
            (Placement(PosY, NegX, PosZ), "_ny"),
            (Placement(PosY, PosZ, PosX), "_pz"),
            (Placement(PosY, PosZ, NegX), "_nz"),
        ],
        Symmetry::Edges => &[
            // Twelve edge cells. Designed for a source wedge running
            // parallel to the source's X axis, with its outer corner
            // in the +Y+Z quadrant. Source's X range is symmetric
            // (the edge's long axis); source's Y and Z are offset.
            //
            // For X-parallel edges, negate source Y/Z to walk the 4
            // (±Y, ±Z) positions. For Y-parallel, route source X (the
            // symmetric axis) to the world Y slot and fill the other
            // two slots with source Y/Z. For Z-parallel, likewise.
            //
            // 4 X-parallel edges:
            (Placement(PosX, PosY, PosZ), "_x_py_pz"),
            (Placement(PosX, NegY, PosZ), "_x_ny_pz"),
            (Placement(PosX, PosY, NegZ), "_x_py_nz"),
            (Placement(PosX, NegY, NegZ), "_x_ny_nz"),
            // 4 Y-parallel edges (source x → world y; source y/z → world x/z):
            (Placement(PosY, PosX, PosZ), "_y_px_pz"),
            (Placement(NegY, PosX, PosZ), "_y_nx_pz"),
            (Placement(PosY, PosX, NegZ), "_y_px_nz"),
            (Placement(NegY, PosX, NegZ), "_y_nx_nz"),
            // 4 Z-parallel edges (source x → world z; source y/z → world x/y):
            (Placement(PosY, PosZ, PosX), "_z_px_py"),
            (Placement(NegY, PosZ, PosX), "_z_nx_py"),
            (Placement(PosY, NegZ, PosX), "_z_px_ny"),
            (Placement(NegY, NegZ, PosX), "_z_nx_ny"),
        ],
    }
}

/// Apply a placement to an integer `Bounds`. Each world axis range is
/// drawn from the source axis (and sign) named by the corresponding
/// placement component. Result is canonicalized (`min ≤ max` per axis).
pub(super) fn apply_placement_to_bounds(placement: Placement, b: Bounds) -> Bounds {
    let (mn_x, mx_x) = resolve_axis_range(placement.0, &b);
    let (mn_y, mx_y) = resolve_axis_range(placement.1, &b);
    let (mn_z, mx_z) = resolve_axis_range(placement.2, &b);
    Bounds(mn_x, mn_y, mn_z, mx_x, mx_y, mx_z)
}

fn resolve_axis_range(sa: SignedAxis, b: &Bounds) -> (i32, i32) {
    let mn = b.min();
    let mx = b.max();
    let (lo, hi) = match sa.axis_index() {
        0 => (mn.0, mx.0),
        1 => (mn.1, mx.1),
        _ => (mn.2, mx.2),
    };
    if sa.is_positive() {
        (lo, hi)
    } else {
        (-hi, -lo)
    }
}

/// Compose two placements: `outer ∘ inner`. The resulting placement
/// applied to a value produces the same result as applying `inner` first
/// and then `outer`.
pub(super) fn compose_placements(outer: Placement, inner: Placement) -> Placement {
    Placement(
        compose_axis(outer.0, inner),
        compose_axis(outer.1, inner),
        compose_axis(outer.2, inner),
    )
}

fn compose_axis(outer: SignedAxis, inner: Placement) -> SignedAxis {
    // The outer signed axis picks one of inner's components (by axis index)
    // and composes signs (negating if outer is negative).
    let picked = match outer.axis_index() {
        0 => inner.0,
        1 => inner.1,
        _ => inner.2,
    };
    if outer.is_positive() {
        picked
    } else {
        picked.negate()
    }
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
/// (post-symmetry-expansion, post-import-remapping) contributes its
/// integer cells to the index.
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

    pub fn collision_count(&self) -> usize {
        self.collisions.len()
    }

    pub fn collisions(&self) -> &[Collision] {
        &self.collisions
    }

    pub fn aabb(&self) -> Option<Bounds> {
        let mut iter = self.cells.keys();
        let first = *iter.next()?;
        let mut mn = first;
        let mut mx = first;
        for &(x, y, z) in iter {
            if x < mn.0 { mn.0 = x; }
            if y < mn.1 { mn.1 = y; }
            if z < mn.2 { mn.2 = z; }
            if x > mx.0 { mx.0 = x; }
            if y > mx.1 { mx.1 = y; }
            if z > mx.2 { mx.2 = z; }
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

/// Walk a `SpecNode` tree, expanding symmetry and imports, and build
/// an `Occupancy` index of every cell claimed by every primitive instance.
pub fn collect_occupancy(spec: &SpecNode, registry: &AssetRegistry) -> Occupancy {
    let mut occ = Occupancy::new();
    walk_for_occupancy(&mut occ, spec, "", identity_placement(), (1, 1, 1), registry);
    occ
}

fn walk_for_occupancy(
    occ: &mut Occupancy,
    node: &SpecNode,
    parent_path: &str,
    inherited: Placement,
    scale: (i32, i32, i32),
    registry: &AssetRegistry,
) {
    let base_path = append_path(parent_path, node.name.as_deref());

    match node.combinator() {
        Combinator::Symmetry(sym) => {
            for (local, suffix) in placements(sym) {
                let combined = compose_placements(inherited, *local);
                let path = if suffix.is_empty() {
                    base_path.clone()
                } else {
                    format!("{base_path}{suffix}")
                };
                walk_single_for_occupancy(occ, node, &path, combined, scale, registry);
            }
        }
        Combinator::Import(import_name) => {
            let Some(imported) = registry.get_shape(import_name) else { return };
            let Some(native_aabb) = imported.compute_aabb() else { return };
            let placement_bounds = node.bounds.unwrap_or(native_aabb);

            let remap_scale = Bounds::remap_scale(&native_aabb);
            let new_scale = (
                scale.0 * remap_scale.0,
                scale.1 * remap_scale.1,
                scale.2 * remap_scale.2,
            );

            let mut remapped = imported.clone();
            remapped.remap_bounds(&native_aabb, &placement_bounds);
            walk_for_occupancy(occ, &remapped, &base_path, inherited, new_scale, registry);
        }
        Combinator::None => {
            walk_single_for_occupancy(occ, node, &base_path, inherited, scale, registry);
        }
    }
}

fn walk_single_for_occupancy(
    occ: &mut Occupancy,
    node: &SpecNode,
    path: &str,
    placement: Placement,
    scale: (i32, i32, i32),
    registry: &AssetRegistry,
) {
    if let (Some(_), Some(bounds)) = (node.shape, node.bounds.as_ref()) {
        let transformed = apply_placement_to_bounds(placement, *bounds);
        claim_cells(occ, &transformed, scale, path);
    }
    for child in &node.children {
        walk_for_occupancy(occ, child, path, placement, scale, registry);
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
    // Integer floor/ceil division to world cells. When a primitive sits
    // at sub-cell positions (possible inside imports with non-integer
    // scaling ratios) we round outward: floor for min, ceil for max.
    // That over-claims the smallest integer cell box containing the
    // primitive. For the cell-level architecture target, non-aligned
    // imports will eventually be disallowed, but the round-outward
    // behavior prevents false negatives (missed collisions) in the
    // meantime.
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
fn floor_div(a: i32, b: i32) -> i32 {
    debug_assert!(b > 0, "scale must be positive");
    let q = a / b;
    let r = a % b;
    if r < 0 { q - 1 } else { q }
}

/// Ceiling division for a signed dividend and a strictly positive divisor.
fn ceil_div(a: i32, b: i32) -> i32 {
    debug_assert!(b > 0, "scale must be positive");
    let q = a / b;
    let r = a % b;
    if r > 0 { q + 1 } else { q }
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
// Expand symmetry for CSG rebuild
// =====================================================================

/// Flatten symmetry combinators on a list of children into a flat list
/// of pre-transformed `(SpecNode, Placement)` pairs. Used by CSG rebuild
/// to produce the same sequence the render walker would.
///
/// Each output pair represents one rendered copy with its accumulated
/// placement. The caller applies the placement to bounds / orient at
/// the appropriate moment.
pub fn expand_symmetry_children(children: &[SpecNode]) -> Vec<(SpecNode, Placement)> {
    let mut result = Vec::new();
    for child in children {
        if child.symmetry != Symmetry::Single {
            let mut base = child.clone();
            base.symmetry = Symmetry::Single;
            for (placement, suffix) in placements(child.symmetry) {
                let mut copy = base.clone();
                if !suffix.is_empty() {
                    if let Some(ref name) = copy.name {
                        copy.name = Some(format!("{name}{suffix}"));
                    }
                }
                result.push((copy, *placement));
            }
        } else {
            result.push((child.clone(), identity_placement()));
        }
    }
    result
}

// =====================================================================
// Serde helpers
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf_spec(bounds: Bounds, symmetry: Symmetry) -> SpecNode {
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
            symmetry,
            combine: CombineMode::Union,
            animations: vec![],
        }
    }

    #[test]
    fn identity_placement_is_no_op_on_bounds() {
        let b = Bounds(1, 2, 3, 4, 5, 6);
        assert_eq!(apply_placement_to_bounds(identity_placement(), b), b);
    }

    #[test]
    fn flip_x_placement_negates_x_range() {
        let b = Bounds(1, 2, 3, 4, 5, 6);
        let p = Placement(SignedAxis::NegX, SignedAxis::PosY, SignedAxis::PosZ);
        let result = apply_placement_to_bounds(p, b);
        assert_eq!(result.min(), (-4, 2, 3));
        assert_eq!(result.max(), (-1, 5, 6));
    }

    #[test]
    fn swap_xy_placement_swaps_ranges() {
        let b = Bounds(1, 3, 0, 2, 5, 1);
        let p = Placement(SignedAxis::PosY, SignedAxis::PosX, SignedAxis::PosZ);
        let result = apply_placement_to_bounds(p, b);
        assert_eq!(result.min(), (3, 1, 0));
        assert_eq!(result.max(), (5, 2, 1));
    }

    #[test]
    fn compose_identity_is_identity() {
        let p = Placement(SignedAxis::NegX, SignedAxis::PosZ, SignedAxis::NegY);
        assert_eq!(compose_placements(identity_placement(), p), p);
        assert_eq!(compose_placements(p, identity_placement()), p);
    }

    #[test]
    fn compose_is_associative_on_sample() {
        // Two flips then a swap should equal the same in any grouping.
        let a = Placement(SignedAxis::NegX, SignedAxis::PosY, SignedAxis::PosZ);
        let b = Placement(SignedAxis::PosX, SignedAxis::NegY, SignedAxis::PosZ);
        let c = Placement(SignedAxis::PosY, SignedAxis::PosX, SignedAxis::PosZ);

        let ab_then_c = compose_placements(c, compose_placements(b, a));
        let a_then_bc = compose_placements(compose_placements(c, b), a);
        assert_eq!(ab_then_c, a_then_bc);
    }

    #[test]
    fn symmetry_placements_are_distinct_and_correct_count() {
        // Every variant's placement table must have unique entries.
        for sym in [
            Symmetry::Single,
            Symmetry::PairX,
            Symmetry::PairY,
            Symmetry::PairZ,
            Symmetry::QuadXY,
            Symmetry::QuadXZ,
            Symmetry::QuadYZ,
            Symmetry::Octants,
            Symmetry::Faces,
            Symmetry::Edges,
        ] {
            let ps = placements(sym);
            let mut seen = std::collections::HashSet::new();
            for (p, _) in ps {
                assert!(
                    seen.insert(*p),
                    "symmetry {:?} has duplicate placement {:?}",
                    sym,
                    p
                );
            }
        }
        assert_eq!(placements(Symmetry::Single).len(), 1);
        assert_eq!(placements(Symmetry::PairX).len(), 2);
        assert_eq!(placements(Symmetry::Octants).len(), 8);
        assert_eq!(placements(Symmetry::Faces).len(), 6);
        assert_eq!(placements(Symmetry::Edges).len(), 12);
    }

    #[test]
    fn faces_symmetry_produces_six_distinct_world_cells() {
        // Source: a box that covers +X face of a 3×3×3 grid.
        let spec = leaf_spec(Bounds(1, -1, -1, 3, 1, 1), Symmetry::Faces);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        // AABB should span ±3 on all three axes (cell range).
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    #[test]
    fn edges_symmetry_produces_twelve_distinct_world_cells() {
        // Source: a +X-parallel edge cell at +Y+Z in the 3×3×3 grid.
        let spec = leaf_spec(Bounds(-1, 1, 1, 1, 3, 3), Symmetry::Edges);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    #[test]
    fn octants_symmetry_produces_eight_corners() {
        let spec = leaf_spec(Bounds(1, 1, 1, 3, 3, 3), Symmetry::Octants);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    #[test]
    fn centered_pair_x_collides_with_itself() {
        // The motivating "sphere at origin mirrored across X" case:
        // both copies land on the same cells, so every cell is a collision.
        let spec = leaf_spec(Bounds(-1, -1, -1, 1, 1, 1), Symmetry::PairX);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 8);
    }

    #[test]
    fn off_origin_pair_x_does_not_collide() {
        let spec = leaf_spec(Bounds(2, 0, 0, 3, 1, 1), Symmetry::PairX);
        let occ = collect_occupancy(&spec, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        assert_eq!(occ.aabb(), Some(Bounds(-3, 0, 0, 3, 1, 1)));
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

    #[test]
    fn claim_cells_handles_non_integer_scale() {
        let mut occ = Occupancy::new();
        let scale = (4, 4, 16);
        let bounds = Bounds(8, -4, -48, 12, 0, 24);
        claim_cells(&mut occ, &bounds, scale, "leaf");
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (2, -1, -3));
        assert_eq!(aabb.max(), (3, 0, 2));
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

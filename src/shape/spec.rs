//! Integer shape specification and occupancy.
//!
//! This module owns the authored data model: the `SpecNode` tree loaded
//! from `.shape.ron` files, occupancy (cell-level collision detection),
//! and AABB computation. All spatial data here is integer. Float
//! conversions for camera positioning (`Bounds::center_f32`) and CSG
//! cell-in-primitive checks (delegated to `super::csg`) are isolated
//! escape hatches.
//!
//! Data flows: file → spec → render → Bevy entities. Nothing here
//! imports `super::render`.

use serde::Deserialize;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use crate::registry::AssetRegistry;

// =====================================================================
// SpecNode — the authored shape tree
// =====================================================================

/// A node in the authored shape list. A `.shape.ron` file is a
/// `Vec<SpecNode>` — a flat array of parts.
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
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub import: Option<String>,
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
    pub subtract: bool,
    #[serde(default)]
    pub animations: Vec<AnimState>,
}

impl SpecNode {
    /// The effective name of this node. Import nodes that don't specify
    /// an explicit name use the last path segment of the import path
    /// (e.g. `"frz-b/chassis"` → `"chassis"`). Name and import are
    /// mutually exclusive.
    pub fn effective_name(&self) -> Option<&str> {
        if let Some(ref name) = self.name {
            return Some(name.as_str());
        }
        if let Some(ref import) = self.import {
            return Some(import.rsplit('/').next().unwrap_or(import));
        }
        None
    }


    /// Compute the AABB of this node. Handles all node types:
    /// explicit bounds, imports (resolved from registry), symmetry
    /// expansion, and children — callers never need to know which case
    /// applies.
    pub fn aabb(&self, registry: &AssetRegistry) -> Option<Bounds> {
        let mut min = (i32::MAX, i32::MAX, i32::MAX);
        let mut max = (i32::MIN, i32::MIN, i32::MIN);
        let mut found = false;
        self.collect_bounds(identity_placement(), &mut min, &mut max, &mut found, registry);
        if found {
            Some(Bounds(min.0, min.1, min.2, max.0, max.1, max.2))
        } else {
            None
        }
    }

    fn collect_bounds(
        &self,
        inherited: Placement,
        min: &mut (i32, i32, i32),
        max: &mut (i32, i32, i32),
        found: &mut bool,
        registry: &AssetRegistry,
    ) {
        for (local, _) in placements(self.symmetry) {
            let combined = compose_placements(inherited, *local);
            self.collect_bounds_single(combined, min, max, found, registry);
        }
    }

    fn collect_bounds_single(
        &self,
        placement: Placement,
        min: &mut (i32, i32, i32),
        max: &mut (i32, i32, i32),
        found: &mut bool,
        registry: &AssetRegistry,
    ) {
        if let Some(ref import_name) = self.import {
            // Use explicit bounds if provided, otherwise resolve
            // from the imported shape's own AABB.
            let resolved = self.bounds.or_else(|| {
                registry.get_shape(import_name)
                    .and_then(|parts| aabb_for_parts(parts, registry))
            });
            if let Some(b) = resolved {
                let transformed = apply_placement_to_bounds(placement, b);
                include_point(min, max, transformed.min(), found);
                include_point(min, max, transformed.max(), found);
            }
        } else {
            if let Some(b) = &self.bounds {
                let transformed = apply_placement_to_bounds(placement, *b);
                include_point(min, max, transformed.min(), found);
                include_point(min, max, transformed.max(), found);
            }
            for child in &self.children {
                child.collect_bounds(placement, min, max, found, registry);
            }
        }
    }

    /// Remap bounds from one coordinate space to another. Handles all
    /// node types: explicit bounds are remapped directly, import nodes
    /// without explicit bounds get their bounds resolved from the
    /// registry first so the remap has something to transform.
    pub fn remap_bounds(&mut self, from: &Bounds, to: &Bounds, registry: &AssetRegistry) {
        if self.bounds.is_none() {
            if let Some(ref import) = self.import {
                self.bounds = registry.get_shape(import)
                    .and_then(|parts| aabb_for_parts(parts, registry));
            }
        }
        if let Some(ref mut b) = self.bounds {
            *b = b.remap(from, to);
        }
        for child in &mut self.children {
            child.remap_bounds(from, to, registry);
        }
    }
}

// =====================================================================
// Free functions for operating over a flat parts list (Vec<SpecNode>)
// =====================================================================

/// Compute the AABB enclosing all parts and their descendants.
pub fn aabb_for_parts(parts: &[SpecNode], registry: &AssetRegistry) -> Option<Bounds> {
    let mut min = (i32::MAX, i32::MAX, i32::MAX);
    let mut max = (i32::MIN, i32::MIN, i32::MIN);
    let mut found = false;
    for part in parts {
        part.collect_bounds(identity_placement(), &mut min, &mut max, &mut found, registry);
    }
    if found {
        Some(Bounds(min.0, min.1, min.2, max.0, max.1, max.2))
    } else {
        None
    }
}

/// Remap all bounds in every part from one coordinate space to another.
pub fn remap_bounds_for_parts(parts: &mut [SpecNode], from: &Bounds, to: &Bounds, registry: &AssetRegistry) {
    for part in parts {
        part.remap_bounds(from, to, registry);
    }
}

// =====================================================================
// Primitives
// =====================================================================

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[allow(non_camel_case_types)]
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Symmetry {
    /// Identity only: 1 copy.
    #[default]
    Single,
    /// Mirror across the x=0 plane: 2 copies.
    MirrorX,
    /// Mirror across the y=0 plane: 2 copies.
    MirrorY,
    /// Mirror across the z=0 plane: 2 copies.
    MirrorZ,
    /// Mirror across x=0 and y=0: 4 copies.
    MirrorXY,
    /// Mirror across x=0 and z=0: 4 copies.
    MirrorXZ,
    /// Mirror across y=0 and z=0: 4 copies.
    MirrorYZ,
    /// Mirror across all three axis planes: 8 copies.
    MirrorXYZ,
    /// Mirror across x=0, then rotate 90° around Y: 4 copies.
    MirrorX_SpinY,
    /// Mirror across y=0, then rotate 90° around Z: 4 copies.
    MirrorY_SpinZ,
    /// Mirror across z=0, then rotate 90° around X: 4 copies.
    MirrorZ_SpinX,
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
        Symmetry::MirrorX => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
        ],
        Symmetry::MirrorY => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, NegY, PosZ), "_my"),
        ],
        Symmetry::MirrorZ => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, PosY, NegZ), "_mz"),
        ],
        Symmetry::MirrorXY => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
            (Placement(PosX, NegY, PosZ), "_my"),
            (Placement(NegX, NegY, PosZ), "_mxy"),
        ],
        Symmetry::MirrorXZ => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
            (Placement(PosX, PosY, NegZ), "_mz"),
            (Placement(NegX, PosY, NegZ), "_mxz"),
        ],
        Symmetry::MirrorYZ => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, NegY, PosZ), "_my"),
            (Placement(PosX, PosY, NegZ), "_mz"),
            (Placement(PosX, NegY, NegZ), "_myz"),
        ],
        Symmetry::MirrorXYZ => &[
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
            (Placement(PosX, NegY, PosZ), "_my"),
            (Placement(NegX, NegY, PosZ), "_mxy"),
            (Placement(PosX, PosY, NegZ), "_mz"),
            (Placement(NegX, PosY, NegZ), "_mxz"),
            (Placement(PosX, NegY, NegZ), "_myz"),
            (Placement(NegX, NegY, NegZ), "_mxyz"),
        ],
        Symmetry::MirrorX_SpinY => &[
            // Mirror across x=0, then rotate both copies 90° around Y.
            // Produces 4 copies on the ±X and ±Z sides.
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(NegX, PosY, PosZ), "_mx"),
            (Placement(PosZ, PosY, NegX), "_sy"),
            (Placement(PosZ, PosY, PosX), "_mx_sy"),
        ],
        Symmetry::MirrorY_SpinZ => &[
            // Mirror across y=0, then rotate both copies 90° around Z.
            // Produces 4 copies on the ±Y and ±X sides.
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, NegY, PosZ), "_my"),
            (Placement(NegY, PosX, PosZ), "_sz"),
            (Placement(PosY, PosX, PosZ), "_my_sz"),
        ],
        Symmetry::MirrorZ_SpinX => &[
            // Mirror across z=0, then rotate both copies 90° around X.
            // Produces 4 copies on the ±Z and ±Y sides.
            (Placement(PosX, PosY, PosZ), ""),
            (Placement(PosX, PosY, NegZ), "_mz"),
            (Placement(PosX, NegZ, PosY), "_sx"),
            (Placement(PosX, PosZ, PosY), "_mz_sx"),
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

/// A subtract volume recorded during the occupancy walk. After all
/// union cells are claimed, cells fully inside a subtract are removed.
struct SubtractVolume {
    shape: PrimitiveShape,
    orient: Orientation,
    placement: Placement,
    bounds: Bounds,
}

/// Global cell-level index of a compiled shape. Every primitive instance
/// (post-symmetry-expansion, post-import-remapping) contributes its
/// integer cells to the index. Subtract primitives remove cells after
/// the initial claim pass.
pub struct Occupancy {
    cells: HashMap<CellPos, String>,
    subtracts: Vec<SubtractVolume>,
    collisions: Vec<Collision>,
}

impl Occupancy {
    fn new() -> Self {
        Self {
            cells: HashMap::new(),
            subtracts: Vec::new(),
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

/// Walk a `SpecNode` tree and build an `Occupancy` index of every cell
/// claimed by every primitive instance in this shape's own integer
/// coordinate space.
///
/// **Per-shape, opaque imports.** This walker does NOT descend into
/// imported subtrees. When it encounters an import combinator, it
/// claims the import's placement bounds as a single opaque region
/// attributed to the import's name, then stops. The imported shape's
/// internal consistency is the responsibility of that shape's own
/// collect_occupancy check (run separately against the registry entry).
///
/// This keeps every coordinate in the collision path integer, attributes
/// authoring errors to the shape that actually caused them, and removes
/// the need for scale tracking or rational comparison across import
/// boundaries.
pub fn collect_occupancy(parts: &[SpecNode], registry: &AssetRegistry) -> Occupancy {
    let mut occ = Occupancy::new();
    for part in parts {
        walk_for_occupancy(&mut occ, part, "", identity_placement(), registry);
    }
    // Remove cells fully covered by subtract volumes.
    apply_subtracts(&mut occ);
    occ
}

fn apply_subtracts(occ: &mut Occupancy) {
    if occ.subtracts.is_empty() { return; }
    let subtracts: Vec<SubtractVolume> = std::mem::take(&mut occ.subtracts);
    occ.cells.retain(|&cell, _| {
        for sub in &subtracts {
            let mn = sub.bounds.min();
            let mx = sub.bounds.max();
            if cell.0 >= mn.0 && cell.0 < mx.0
                && cell.1 >= mn.1 && cell.1 < mx.1
                && cell.2 >= mn.2 && cell.2 < mx.2
            {
                if super::csg::is_cell_inside_primitive(sub.shape, &sub.orient, sub.placement, &sub.bounds, cell) {
                    return false; // cell is subtracted
                }
            }
        }
        true // cell survives
    });
    // Collisions were recorded at claim time before subtracts were
    // applied. Remove collisions for cells that no longer exist.
    occ.collisions.retain(|c| occ.cells.contains_key(&c.cell));
}

fn walk_for_occupancy(
    occ: &mut Occupancy,
    node: &SpecNode,
    parent_path: &str,
    inherited: Placement,
    registry: &AssetRegistry,
) {
    let base_path = append_path(parent_path, node.effective_name());

    let sym = node.symmetry;
    for (local, suffix) in placements(sym) {
        let combined = compose_placements(inherited, *local);
        let path = if suffix.is_empty() {
            base_path.clone()
        } else {
            format!("{base_path}{suffix}")
        };
        if let Some(ref import_name) = node.import {
            walk_import_for_occupancy(occ, node, import_name, &path, combined, registry);
        } else {
            walk_single_for_occupancy(occ, node, &path, combined, registry);
        }
    }
}

fn walk_import_for_occupancy(
    occ: &mut Occupancy,
    import_node: &SpecNode,
    import_name: &str,
    parent_path: &str,
    inherited: Placement,
    registry: &AssetRegistry,
) {
    let Some(imported) = registry.get_shape(import_name) else {
        // Import not in registry — fall back to claiming explicit bounds
        // as an opaque region.
        if let Some(bounds) = import_node.bounds {
            let transformed = apply_placement_to_bounds(inherited, bounds);
            claim_cells(occ, &transformed, parent_path);
        }
        return;
    };
    // Run occupancy on the imported shape independently, then merge
    // its post-subtract cells into our occupancy, remapped to the
    // parent's coordinate space.
    let imported_occ = collect_occupancy(imported, registry);

    let Some(native_aabb) = imported_occ.aabb() else { return };
    let placement_bounds = import_node.bounds.unwrap_or(native_aabb);

    // Remap each surviving cell from the imported shape's coordinate
    // space to the parent's. The import fills placement_bounds; the
    // imported shape's native extent is native_aabb. We map each cell
    // from native to placement space.
    let native_min = native_aabb.min();
    let native_size = native_aabb.size();
    let place_min = placement_bounds.min();
    let place_size = placement_bounds.size();

    for (&cell, _) in &imported_occ.cells {
        // Map cell from imported coords to placement coords.
        // Each axis: parent_cell = place_min + (cell - native_min) * place_size / native_size
        // This must be integer. For axis-aligned imports where place_size is a
        // multiple of native_size, this is exact. Otherwise we floor.
        let remap_axis = |c: i32, n_min: i32, n_size: i32, p_min: i32, p_size: i32| -> (i32, i32) {
            if n_size == 0 { return (p_min, p_min + 1); }
            let lo = p_min + (c - n_min) * p_size / n_size;
            let hi = p_min + (c + 1 - n_min) * p_size / n_size;
            (lo, hi)
        };

        let (x_lo, x_hi) = remap_axis(cell.0, native_min.0, native_size.0, place_min.0, place_size.0);
        let (y_lo, y_hi) = remap_axis(cell.1, native_min.1, native_size.1, place_min.1, place_size.1);
        let (z_lo, z_hi) = remap_axis(cell.2, native_min.2, native_size.2, place_min.2, place_size.2);

        for z in z_lo..z_hi {
            for y in y_lo..y_hi {
                for x in x_lo..x_hi {
                    let parent_cell = (x, y, z);
                    let transformed = apply_placement_to_cell(inherited, parent_cell);
                    occ.claim(transformed, parent_path);
                }
            }
        }
    }
}

fn apply_placement_to_cell(placement: Placement, cell: (i32, i32, i32)) -> (i32, i32, i32) {
    let resolve = |sa: SignedAxis| -> i32 {
        let val = match sa.axis_index() {
            0 => cell.0,
            1 => cell.1,
            _ => cell.2,
        };
        if sa.is_positive() { val } else { -val - 1 }
    };
    (resolve(placement.0), resolve(placement.1), resolve(placement.2))
}

fn walk_single_for_occupancy(
    occ: &mut Occupancy,
    node: &SpecNode,
    path: &str,
    placement: Placement,
    registry: &AssetRegistry,
) {
    if let (Some(shape), Some(bounds)) = (node.shape, node.bounds.as_ref()) {
        let transformed = apply_placement_to_bounds(placement, *bounds);
        if node.subtract {
            occ.subtracts.push(SubtractVolume {
                shape,
                orient: node.orient,
                placement,
                bounds: transformed,
            });
        } else {
            claim_cells(occ, &transformed, path);
        }
    }
    for child in &node.children {
        walk_for_occupancy(occ, child, path, placement, registry);
    }
}

/// Claim every integer cell inside the given bounds, attributing each
/// to the given authoring path. Cells are unit-sized integer cubes;
/// this is valid because per-shape checking never crosses an import
/// boundary, so every coordinate is integer in the current shape's space.
fn claim_cells(occ: &mut Occupancy, bounds: &Bounds, path: &str) {
    let mn = bounds.min();
    let mx = bounds.max();
    for z in mn.2..mx.2 {
        for y in mn.1..mx.1 {
            for x in mn.0..mx.0 {
                occ.claim((x, y, z), path);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf_spec(bounds: Bounds, symmetry: Symmetry) -> SpecNode {
        SpecNode {
            name: Some("leaf".into()),
            shape: Some(PrimitiveShape::Box),
            bounds: Some(bounds),
            orient: Orientation::default(),
            tags: vec![],
            import: None,
            children: vec![],
            symmetry,
            subtract: false,
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
            Symmetry::MirrorX,
            Symmetry::MirrorY,
            Symmetry::MirrorZ,
            Symmetry::MirrorXY,
            Symmetry::MirrorXZ,
            Symmetry::MirrorYZ,
            Symmetry::MirrorXYZ,
            Symmetry::MirrorX_SpinY,
            Symmetry::MirrorY_SpinZ,
            Symmetry::MirrorZ_SpinX,
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
        assert_eq!(placements(Symmetry::MirrorX).len(), 2);
        assert_eq!(placements(Symmetry::MirrorXYZ).len(), 8);
        assert_eq!(placements(Symmetry::MirrorX_SpinY).len(), 4);
        assert_eq!(placements(Symmetry::MirrorY_SpinZ).len(), 4);
        assert_eq!(placements(Symmetry::MirrorZ_SpinX).len(), 4);
        assert_eq!(placements(Symmetry::Faces).len(), 6);
        assert_eq!(placements(Symmetry::Edges).len(), 12);
    }

    #[test]
    fn faces_symmetry_produces_six_distinct_world_cells() {
        // Source: a box that covers +X face of a 3×3×3 grid.
        let spec = leaf_spec(Bounds(1, -1, -1, 3, 1, 1), Symmetry::Faces);
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
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
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    #[test]
    fn octants_symmetry_produces_eight_corners() {
        let spec = leaf_spec(Bounds(1, 1, 1, 3, 3, 3), Symmetry::MirrorXYZ);
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    #[test]
    fn centered_pair_x_collides_with_itself() {
        // The motivating "sphere at origin mirrored across X" case:
        // both copies land on the same cells, so every cell is a collision.
        let spec = leaf_spec(Bounds(-1, -1, -1, 1, 1, 1), Symmetry::MirrorX);
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 8);
    }

    #[test]
    fn off_origin_pair_x_does_not_collide() {
        let spec = leaf_spec(Bounds(2, 0, 0, 3, 1, 1), Symmetry::MirrorX);
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        assert_eq!(occ.aabb(), Some(Bounds(-3, 0, 0, 3, 1, 1)));
    }

    /// Per-shape collision: imports are opaque. Placing two imports
    /// whose placement AABBs overlap is reported as a collision
    /// attributed to the current shape (not the imported shapes).
    #[test]
    fn overlapping_import_placements_are_reported() {
        // Shape root with two import children at overlapping placements.
        let mut shape_a_import = leaf_spec(
            Bounds(0, 0, 0, 3, 3, 3),
            Symmetry::Single,
        );
        shape_a_import.name = Some("a".into());
        shape_a_import.shape = None;
        shape_a_import.import = Some("dummy_a".into());

        let mut shape_b_import = leaf_spec(
            Bounds(2, 0, 0, 5, 3, 3),
            Symmetry::Single,
        );
        shape_b_import.name = Some("b".into());
        shape_b_import.shape = None;
        shape_b_import.import = Some("dummy_b".into());

        let parts = vec![shape_a_import, shape_b_import];

        // No actual imports in the registry; the walker doesn't need
        // to resolve them — it just claims their placement bounds.
        let occ = collect_occupancy(&parts, &AssetRegistry::default());
        // Overlap region: x ∈ [2, 3], y ∈ [0, 3], z ∈ [0, 3] = 9 cells.
        assert_eq!(occ.collision_count(), 9);
    }

    /// Per-shape collision: an import placement that DOESN'T overlap a
    /// sibling primitive in the current shape's own coords reports
    /// no collision, even if the imported shape's internal contents
    /// would (hypothetically) occupy other cells in world space.
    #[test]
    fn non_overlapping_imports_pass() {
        let mut import_a = leaf_spec(Bounds(0, 0, 0, 3, 3, 3), Symmetry::Single);
        import_a.name = Some("a".into());
        import_a.shape = None;
        import_a.import = Some("dummy".into());

        let mut native = leaf_spec(Bounds(-5, 0, 0, -2, 3, 3), Symmetry::Single);
        native.name = Some("native".into());

        let parts = vec![import_a, native];

        let occ = collect_occupancy(&parts, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
    }
}


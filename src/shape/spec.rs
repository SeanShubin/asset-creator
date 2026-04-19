//! Integer shape specification and occupancy.
//!
//! This module owns the authored data model: the `SpecNode` tree loaded
//! from `.shape.ron` files, occupancy (cell-level collision detection),
//! and AABB computation. All spatial data here is integer. Float
//! conversions for CSG
//! cell-in-primitive checks (delegated to `super::csg`) are isolated
//! escape hatches.
//!
//! Data flows: file → spec → render → Bevy entities. Nothing here
//! imports `super::render`. The spec module calls `super::csg` for
//! CSG signature computation during symmetry deduplication — this is
//! the one intentional cross-module dependency.

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
    pub bounds: Option<Bounds>,
    /// Wedge: 2 faces adjacent to the filled half.
    #[serde(default)]
    pub faces: Option<[Face; 2]>,
    /// Corner: 3 faces meeting at the filled vertex.
    #[serde(default)]
    pub corner: Option<[Face; 3]>,
    /// InverseCorner: 3 faces meeting at the clipped vertex.
    #[serde(default)]
    pub clip: Option<[Face; 3]>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub import: Option<String>,
    #[serde(default)]
    pub children: Vec<SpecNode>,
    /// Single transform applied before symmetry. Composed left-to-right
    /// into one placement. Used for orienting imports.
    #[serde(default)]
    pub rotate: Vec<SymOp>,
    /// Symmetry generators. The system takes the closure and deduplicates.
    #[serde(default)]
    pub symmetry: Vec<SymOp>,
    #[serde(default)]
    pub subtract: bool,
    #[serde(default)]
    pub animations: Vec<AnimState>,
}

impl SpecNode {
    /// Infer the primitive shape and orientation from corner/clip/fill.
    /// Returns None for container nodes (no geometry) and Box nodes
    /// (no corner/clip/fill specified but bounds present).
    pub fn primitive(&self) -> Option<(PrimitiveShape, Placement)> {
        self.bounds?;

        if let Some(f) = self.corner {
            return Some((PrimitiveShape::Corner, faces_to_placement(&f)));
        }
        if let Some(f) = self.clip {
            return Some((PrimitiveShape::InverseCorner, faces_to_placement(&f)));
        }
        if let Some(f) = self.faces {
            return Some((PrimitiveShape::Wedge, faces_to_placement(&f)));
        }
        Some((PrimitiveShape::Box, identity_placement()))
    }

    /// Convenience: the shape type, or None for containers.
    pub fn shape(&self) -> Option<PrimitiveShape> {
        self.primitive().map(|(s, _)| s)
    }

    /// The full orientation: primitive's own placement composed with
    /// the `rotate` field. For imports without a primitive, `rotate`
    /// alone determines the orientation.
    pub fn orient_placement(&self) -> Placement {
        let prim_p = self.primitive().map(|(_, p)| p).unwrap_or(identity_placement());
        let rotate_p = compose_orient(&self.rotate);
        compose_placements(rotate_p, prim_p)
    }

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


    fn collect_bounds(
        &self,
        inherited: Placement,
        min: &mut (i32, i32, i32),
        max: &mut (i32, i32, i32),
        found: &mut bool,
        registry: &AssetRegistry,
    ) {
        for (local, _) in &placements_for(self) {
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
    /// Box with one corner clipped — the complement of Corner.
    /// Fills where x + y + z >= -0.5 in identity orientation.
    InverseCorner,
}

/// A face of the bounding box, used to specify which sides of the
/// cell are adjacent to filled geometry. Determines the primitive
/// shape and orientation.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    MinX, MaxX,
    MinY, MaxY,
    MinZ, MaxZ,
}

// =====================================================================
// SymOp — the unified operation for orient and symmetry
// =====================================================================

/// A single geometric operation. Used by both `orient` (composed
/// left-to-right into one transform) and `symmetry` (closure taken
/// over generators, deduplicated by signature).
#[allow(non_camel_case_types)]
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymOp {
    MirrorX,
    MirrorY,
    MirrorZ,
    /// 90° rotation in the XY plane (X toward Y).
    Rotate90_XY,
    /// 90° rotation in the XZ plane (X toward Z).
    Rotate90_XZ,
    /// 90° rotation in the YZ plane (Y toward Z).
    Rotate90_YZ,
    /// 180° rotation in the XY plane.
    Rotate180_XY,
    /// 180° rotation in the XZ plane.
    Rotate180_XZ,
    /// 180° rotation in the YZ plane.
    Rotate180_YZ,
}

impl SymOp {
    pub fn to_placement(self) -> Placement {
        use SignedAxis::*;
        match self {
            SymOp::MirrorX     => Placement(NegX, PosY, PosZ),
            SymOp::MirrorY     => Placement(PosX, NegY, PosZ),
            SymOp::MirrorZ     => Placement(PosX, PosY, NegZ),
            // XY plane: X→Y, Y→−X (CCW from +Z)
            SymOp::Rotate90_XY  => Placement(NegY, PosX, PosZ),
            // XZ plane: X→Z, Z→−X (CCW from +Y)
            SymOp::Rotate90_XZ  => Placement(NegZ, PosY, PosX),
            // YZ plane: Y→Z, Z→−Y (CCW from +X)
            SymOp::Rotate90_YZ  => Placement(PosX, NegZ, PosY),
            SymOp::Rotate180_XY => Placement(NegX, NegY, PosZ),
            SymOp::Rotate180_XZ => Placement(NegX, PosY, NegZ),
            SymOp::Rotate180_YZ => Placement(PosX, NegY, NegZ),
        }
    }
}

/// Compose a sequence of operations left-to-right into one placement.
/// Used by `orient` to produce a single transform.
pub fn compose_orient(ops: &[SymOp]) -> Placement {
    let mut result = identity_placement();
    for op in ops {
        result = compose_placements(op.to_placement(), result);
    }
    result
}

// =====================================================================
// Bounds
// =====================================================================

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds(pub i32, pub i32, pub i32, pub i32, pub i32, pub i32);

impl Bounds {

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
// Shape inference from corner/clip/fill
// =====================================================================

/// Compute the placement from a list of faces. Each face specifies
/// one axis direction. The identity primitives fill the Min side of
/// each axis, so MinX/MinY/MinZ → identity. MaxX/MaxY/MaxZ → mirror.
///
/// For Wedge (2 faces): the identity fills y+z ≤ 0 (MinY, MinZ sides).
/// The two faces determine which two axes are involved in the cut and
/// which side is filled. The third axis (not mentioned) is the ridge.
///
/// For Corner/InverseCorner (3 faces): all three axes specified.
fn faces_to_placement(faces: &[Face]) -> Placement {
    use SignedAxis::*;

    if faces.len() == 3 {
        // Corner/InverseCorner: each face determines one axis mirror.
        let mut px = PosX; let mut py = PosY; let mut pz = PosZ;
        for &f in faces {
            match f {
                Face::MinX => px = PosX, Face::MaxX => px = NegX,
                Face::MinY => py = PosY, Face::MaxY => py = NegY,
                Face::MinZ => pz = PosZ, Face::MaxZ => pz = NegZ,
            }
        }
        Placement(px, py, pz)
    } else if faces.len() == 2 {
        // Wedge: identity fills y+z ≤ 0, ridge along X.
        // We need to map the identity's Y and Z cut axes to the
        // world axes specified by the two faces, and route the
        // ridge (identity X) to the unspecified world axis.
        //
        // Each face specifies a world axis and sign:
        //   MinX → world X, min side (PosX preserves)
        //   MaxX → world X, max side (NegX mirrors)
        //   etc.
        let face_to_signed_axis = |f: Face| -> SignedAxis {
            match f {
                Face::MinX => PosX, Face::MaxX => NegX,
                Face::MinY => PosY, Face::MaxY => NegY,
                Face::MinZ => PosZ, Face::MaxZ => NegZ,
            }
        };
        let sa0 = face_to_signed_axis(faces[0]);
        let sa1 = face_to_signed_axis(faces[1]);

        // Determine the ridge axis (the one not mentioned).
        // Build placement: world_ridge = identity_X,
        //                  world_cut0 = identity_Y (with sign),
        //                  world_cut1 = identity_Z (with sign).
        // The placement slots are indexed by WORLD axis.
        let mut slots = [PosX, PosY, PosZ]; // [world_x, world_y, world_z]

        // Assign sa0 to identity_Y, sa1 to identity_Z.
        // The sign of sa0/sa1 tells us whether to mirror.
        // The axis of sa0/sa1 tells us which world slot to fill.
        let axis_idx = |sa: SignedAxis| -> usize {
            match sa {
                PosX | NegX => 0,
                PosY | NegY => 1,
                PosZ | NegZ => 2,
            }
        };
        let to_identity_axis = |sa: SignedAxis, identity: SignedAxis| -> SignedAxis {
            // If face says Min (Pos), identity axis is positive.
            // If face says Max (Neg), identity axis is negative (mirrored).
            if sa.is_positive() { identity } else { identity.negate() }
        };

        slots[axis_idx(sa0)] = to_identity_axis(sa0, PosY);
        slots[axis_idx(sa1)] = to_identity_axis(sa1, PosZ);

        // The ridge axis is the one not touched by sa0 or sa1.
        // It gets identity_X (PosX).
        let ridge_idx = (0..3).find(|&i| i != axis_idx(sa0) && i != axis_idx(sa1)).unwrap();
        slots[ridge_idx] = PosX;

        Placement(slots[0], slots[1], slots[2])
    } else {
        identity_placement()
    }
}

// =====================================================================
// Placements
// =====================================================================

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

/// Compute placements from symmetry generators. Takes the closure of
/// the generator operations and deduplicates by (bounds, CSG signature).
/// Works for any combination of operations — mirrors, rotations, or both.
/// Compute placements from a SpecNode's symmetry generators.
pub fn placements_for(spec: &SpecNode) -> Vec<(Placement, String)> {
    let shape = spec.shape().unwrap_or(PrimitiveShape::Box);
    let orient_p = spec.orient_placement();
    placements(&spec.symmetry, spec.bounds, Some(shape), orient_p)
}

pub fn placements(
    generators: &[SymOp],
    bounds: Option<Bounds>,
    shape: Option<PrimitiveShape>,
    orient_placement: Placement,
) -> Vec<(Placement, String)> {
    let mut set = vec![identity_placement()];
    for op in generators {
        let gen = op.to_placement();
        loop {
            let mut new_found = false;
            let current = set.clone();
            for existing in &current {
                let new = compose_placements(gen, *existing);
                if !set.contains(&new) {
                    set.push(new);
                    new_found = true;
                }
            }
            if !new_found { break; }
        }
    }

    // Deduplicate by (canonicalized bounds, signature).
    let shape = shape.unwrap_or(PrimitiveShape::Box);
    let bounds = bounds.unwrap_or(Bounds(0, 0, 0, 1, 1, 1));

    let mut result: Vec<(Placement, Bounds, u64)> = Vec::new();
    for p in &set {
        let transformed = apply_placement_to_bounds(*p, bounds);
        let canon = Bounds(
            transformed.min().0, transformed.min().1, transformed.min().2,
            transformed.max().0, transformed.max().1, transformed.max().2,
        );
        let combined = compose_placements(*p, orient_placement);
        let mat = super::csg::placement_to_mat3(combined);
        let sig = super::csg::compute_signature(shape, mat);

        if !result.iter().any(|(_, b, s)| *b == canon && *s == sig) {
            result.push((*p, canon, sig));
        }
    }

    result.into_iter()
        .enumerate()
        .map(|(i, (p, _, _))| {
            let suffix = if i == 0 { String::new() } else { format!("_{i}") };
            (p, suffix)
        })
        .collect()
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
    orient_placement: Placement,
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

    /// Log collision warnings, showing up to 10 details.
    pub fn warn_collisions(&self, label: &str) {
        if self.collisions.is_empty() { return; }
        bevy::prelude::warn!(
            "{} has {} cell-level collision(s)",
            label, self.collisions.len()
        );
        for c in self.collisions.iter().take(10) {
            bevy::prelude::warn!(
                "  collision at {:?}: '{}' vs '{}'",
                c.cell, c.first_path, c.second_path
            );
        }
        if self.collisions.len() > 10 {
            bevy::prelude::warn!("  ... and {} more", self.collisions.len() - 10);
        }
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
                if super::csg::is_cell_inside_primitive(sub.shape, sub.orient_placement, &sub.bounds, cell) {
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

    for (local, suffix) in &placements_for(node) {
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
    if let (Some((shape, orient_p)), Some(bounds)) = (node.primitive(), node.bounds.as_ref()) {
        let transformed = apply_placement_to_bounds(placement, *bounds);
        if node.subtract {
            occ.subtracts.push(SubtractVolume {
                shape,
                orient_placement: compose_placements(placement, orient_p),
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

    fn leaf_spec(bounds: Bounds, symmetry: Vec<SymOp>) -> SpecNode {
        SpecNode {
            name: Some("leaf".into()),
            bounds: Some(bounds),
            corner: None,
            clip: None,
            faces: None,
            tags: vec![],
            import: None,
            children: vec![],
            rotate: vec![],
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
    fn symmetry_placements_correct_counts() {
        let id = identity_placement();
        assert_eq!(placements(&[], None, None, id).len(), 1);
        assert_eq!(placements(&[SymOp::MirrorX], None, None, id).len(), 2);
        assert_eq!(placements(&[SymOp::MirrorX, SymOp::MirrorY, SymOp::MirrorZ], None, None, id).len(), 8);
        assert_eq!(placements(&[SymOp::MirrorX, SymOp::Rotate90_XZ], None, None, id).len(), 4);
        assert_eq!(placements(&[SymOp::MirrorY, SymOp::Rotate90_XY], None, None, id).len(), 4);
        assert_eq!(placements(&[SymOp::MirrorZ, SymOp::Rotate90_YZ], None, None, id).len(), 4);
        let rot = &[SymOp::Rotate90_XY, SymOp::Rotate90_XZ, SymOp::Rotate90_YZ];
        assert_eq!(placements(rot, Some(Bounds(1, -1, -1, 3, 1, 1)), Some(PrimitiveShape::Box), id).len(), 6);
        assert_eq!(placements(rot, Some(Bounds(-1, 1, 1, 1, 3, 3)), Some(PrimitiveShape::Wedge), id).len(), 12);
    }

    #[test]
    fn faces_symmetry_produces_six_distinct_world_cells() {
        // Source: a box that covers +X face of a 3×3×3 grid.
        let spec = leaf_spec(Bounds(1, -1, -1, 3, 1, 1), vec![SymOp::Rotate90_XY, SymOp::Rotate90_XZ, SymOp::Rotate90_YZ]);
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
        let spec = leaf_spec(Bounds(-1, 1, 1, 1, 3, 3), vec![SymOp::Rotate90_XY, SymOp::Rotate90_XZ, SymOp::Rotate90_YZ]);
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    #[test]
    fn octants_symmetry_produces_eight_corners() {
        let spec = leaf_spec(Bounds(1, 1, 1, 3, 3, 3), vec![SymOp::MirrorX, SymOp::MirrorY, SymOp::MirrorZ]);
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
        let aabb = occ.aabb().unwrap();
        assert_eq!(aabb.min(), (-3, -3, -3));
        assert_eq!(aabb.max(), (3, 3, 3));
    }

    #[test]
    fn centered_box_mirror_x_deduplicates() {
        // A centered Box mirrored across X produces the same geometry
        // — signature-based dedup gives 1 copy, no collisions.
        let spec = leaf_spec(Bounds(-1, -1, -1, 1, 1, 1), vec![SymOp::MirrorX]);
        let occ = collect_occupancy(&[spec], &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
    }

    #[test]
    fn off_origin_pair_x_does_not_collide() {
        let spec = leaf_spec(Bounds(2, 0, 0, 3, 1, 1), vec![SymOp::MirrorX]);
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
            vec![],
        );
        shape_a_import.name = Some("a".into());
        shape_a_import.import = Some("dummy_a".into());

        let mut shape_b_import = leaf_spec(
            Bounds(2, 0, 0, 5, 3, 3),
            vec![],
        );
        shape_b_import.name = Some("b".into());
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
        let mut import_a = leaf_spec(Bounds(0, 0, 0, 3, 3, 3), vec![]);
        import_a.name = Some("a".into());
        import_a.import = Some("dummy".into());

        let mut native = leaf_spec(Bounds(-5, 0, 0, -2, 3, 3), vec![]);
        native.name = Some("native".into());

        let parts = vec![import_a, native];

        let occ = collect_occupancy(&parts, &AssetRegistry::default());
        assert_eq!(occ.collision_count(), 0);
    }
}


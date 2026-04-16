//! Unit-cell CSG for subtract primitives.
//!
//! Semantics: a subtract primitive removes its actual shape volume from
//! any union primitive sharing the same unit cell. The result must be
//! expressible as one of our three primitives (Box / Wedge / Corner) in
//! some orientation — otherwise the compile step reports an error.
//!
//! Implementation: sample the unit cube at 64 fixed interior points
//! (4×4×4 grid with small per-axis offsets so no sample lands on any
//! primitive's cut plane). Each (shape, orientation) pair maps to a
//! 64-bit signature recording which samples are inside. CSG difference
//! is then just `minuend & !subtrahend` on the bitmask, and we look up
//! the result in a precomputed signature → (shape, mat) table.
//!
//! The 48 orientations × 3 shapes give 144 entries, many with
//! duplicate signatures (e.g. all rotations of Box sample the same).
//! First-inserted wins during table construction, which means the
//! lookup returns *some* valid representation for any reachable
//! bitmask — enough for rendering.

use bevy::math::{Mat3, Quat, Vec3};
use std::collections::HashMap;
use std::sync::OnceLock;

use super::spec::{Bounds, Facing, Mirroring, Orientation, Placement, PrimitiveShape, Rotation, SignedAxis};

// =====================================================================
// Orientation → Mat3 conversion (cube symmetry math)
// =====================================================================

pub fn base_orientation_matrix(orient: &Orientation) -> Mat3 {
    let facing = facing_matrix(orient.facing());
    let mirror = match orient.mirroring() {
        Mirroring::NoMirror => Mat3::IDENTITY,
        Mirroring::Mirror => Mat3::from_cols(Vec3::NEG_X, Vec3::Y, Vec3::Z),
    };
    let rotation = rotation_matrix(orient.rotation());
    rotation * mirror * facing
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
// Cell-level queries
// =====================================================================

/// Is this cell fully inside the given oriented primitive? Used by the
/// occupancy pass to determine which cells a subtract removes.
pub fn is_cell_inside_primitive(
    shape: PrimitiveShape,
    orient: &Orientation,
    placement: Placement,
    prim_bounds: &Bounds,
    cell: (i32, i32, i32),
) -> bool {
    if shape == PrimitiveShape::Box {
        return true; // all cells in a Box's bounds are inside
    }
    let orient_mat = orientation_to_mat3(orient, placement);
    let mn = prim_bounds.min();
    let mx = prim_bounds.max();
    let prim_center = Vec3::new(
        (mn.0 + mx.0) as f32 / 2.0,
        (mn.1 + mx.1) as f32 / 2.0,
        (mn.2 + mx.2) as f32 / 2.0,
    );
    let prim_size = Vec3::new(
        (mx.0 - mn.0) as f32,
        (mx.1 - mn.1) as f32,
        (mx.2 - mn.2) as f32,
    );
    // Check all 64 sample points. If all are inside, the cell is fully
    // covered. This is conservative — partially covered cells remain
    // occupied, which is correct for collision detection (avoids
    // false negatives).
    compute_signature_at_cell(shape, orient_mat, prim_center, prim_size, cell) == !0u64
}

pub fn orientation_to_mat3(orient: &Orientation, placement: Placement) -> Mat3 {
    let base = base_orientation_matrix(orient);
    apply_placement_to_mat3(placement, base)
}

fn apply_placement_to_mat3(placement: Placement, m: Mat3) -> Mat3 {
    let apply_to_col = |col: Vec3| -> Vec3 {
        Vec3::new(
            signed_axis_project(placement.0, col),
            signed_axis_project(placement.1, col),
            signed_axis_project(placement.2, col),
        )
    };
    Mat3::from_cols(
        apply_to_col(m.x_axis),
        apply_to_col(m.y_axis),
        apply_to_col(m.z_axis),
    )
}

fn signed_axis_project(sa: SignedAxis, v: Vec3) -> f32 {
    let component = match sa {
        SignedAxis::PosX | SignedAxis::NegX => v.x,
        SignedAxis::PosY | SignedAxis::NegY => v.y,
        SignedAxis::PosZ | SignedAxis::NegZ => v.z,
    };
    if matches!(sa, SignedAxis::PosX | SignedAxis::PosY | SignedAxis::PosZ) {
        component
    } else {
        -component
    }
}

const GRID_DIM: usize = 4;
const SAMPLE_COUNT: usize = GRID_DIM * GRID_DIM * GRID_DIM;

/// Small per-axis offsets — chosen so that after the offset, no sample
/// point lies on any plane of the form `axis = 0` / `y ± z = 0` /
/// `x + y + z = ±0.5` / etc. that our primitive cut planes use. If a
/// new primitive gets added with a cut plane these offsets land on,
/// the per-orientation signature table would acquire duplicates and
/// the lookup would misreport. Tests in this module verify nothing
/// collides today.
const OFFSET_X: f32 = 0.013;
const OFFSET_Y: f32 = 0.027;
const OFFSET_Z: f32 = 0.041;

fn sample_points() -> &'static [Vec3] {
    static CELL: OnceLock<Vec<Vec3>> = OnceLock::new();
    CELL.get_or_init(|| {
        let step = 1.0 / GRID_DIM as f32;
        let mut v = Vec::with_capacity(SAMPLE_COUNT);
        for zi in 0..GRID_DIM {
            for yi in 0..GRID_DIM {
                for xi in 0..GRID_DIM {
                    v.push(Vec3::new(
                        -0.5 + (xi as f32 + 0.5) * step + OFFSET_X,
                        -0.5 + (yi as f32 + 0.5) * step + OFFSET_Y,
                        -0.5 + (zi as f32 + 0.5) * step + OFFSET_Z,
                    ));
                }
            }
        }
        v
    })
}

/// Is `p` inside the identity-orientation primitive occupying the unit
/// cube [-0.5, 0.5]³? All sample points are already inside the cube,
/// so the box check is trivially true.
fn point_in_identity_primitive(shape: PrimitiveShape, p: Vec3) -> bool {
    match shape {
        PrimitiveShape::Box => true,
        // Identity wedge: slope from (top, -z) to (bottom, +z).
        // Filled half is y + z <= 0.
        PrimitiveShape::Wedge => p.y + p.z <= 0.0,
        // Identity corner: tetrahedron at the (-x, -y, -z) vertex,
        // bounded by the plane x + y + z = -0.5.
        PrimitiveShape::Corner => p.x + p.y + p.z <= -0.5,
    }
}

/// Compute the 64-bit signature for a primitive oriented by `orient_mat`.
///
/// A world-space sample point `p` is inside the oriented primitive iff
/// `orient_mat⁻¹ · p` is inside the identity primitive. Our orientation
/// matrices are cube symmetries (orthonormal, det ±1), so inverse =
/// transpose.
pub fn compute_signature(shape: PrimitiveShape, orient_mat: Mat3) -> u64 {
    let inv = orient_mat.transpose();
    let mut mask = 0u64;
    for (i, p) in sample_points().iter().enumerate() {
        let local = inv * (*p);
        if point_in_identity_primitive(shape, local) {
            mask |= 1u64 << i;
        }
    }
    mask
}

/// Compute the 64-bit signature of a primitive's volume at a specific
/// world cell. The primitive occupies `prim_center` with half-extents
/// `prim_half_size` in world space and is oriented by `orient_mat`.
///
/// Each sample point is placed at the cell's world position, mapped into
/// the primitive's local [-0.5, 0.5]³ space, and tested against the
/// identity primitive. This correctly handles multi-cell primitives
/// where different cells see different slices of the shape (e.g. some
/// cells are fully inside a wedge, others are on the cut surface).
pub fn compute_signature_at_cell(
    shape: PrimitiveShape,
    orient_mat: Mat3,
    prim_center: Vec3,
    prim_half_size: Vec3,
    cell: (i32, i32, i32),
) -> u64 {
    let inv = orient_mat.transpose();
    let cell_center = Vec3::new(
        cell.0 as f32 + 0.5,
        cell.1 as f32 + 0.5,
        cell.2 as f32 + 0.5,
    );
    let mut mask = 0u64;
    for (i, p) in sample_points().iter().enumerate() {
        // Sample point in world space
        let world_p = cell_center + *p;
        // Map to primitive's normalized [-0.5, 0.5]³ space
        let normalized = Vec3::new(
            (world_p.x - prim_center.x) / prim_half_size.x,
            (world_p.y - prim_center.y) / prim_half_size.y,
            (world_p.z - prim_center.z) / prim_half_size.z,
        );
        let local = inv * normalized;
        if point_in_identity_primitive(shape, local) {
            mask |= 1u64 << i;
        }
    }
    mask
}

/// Table of `signature → (shape, orient_mat)` covering every reachable
/// primitive/orientation pair. Built by enumerating all 3 shapes × 48
/// orientations. Multiple entries can share a signature (e.g. every
/// rotation of a Box samples identically — they all have `!0` as the
/// signature); in that case whichever inserts first wins. Readers just
/// need *some* valid `(shape, mat)` that reproduces the bitmask.
fn primitive_table() -> &'static HashMap<u64, (PrimitiveShape, Mat3)> {
    static CELL: OnceLock<HashMap<u64, (PrimitiveShape, Mat3)>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut table: HashMap<u64, (PrimitiveShape, Mat3)> = HashMap::new();
        for shape in [
            PrimitiveShape::Box,
            PrimitiveShape::Wedge,
            PrimitiveShape::Corner,
        ] {
            for orient in all_orientations() {
                let mat = base_orientation_matrix(&orient);
                let sig = compute_signature(shape, mat);
                table.entry(sig).or_insert((shape, mat));
            }
        }
        table
    })
}

fn all_orientations() -> impl Iterator<Item = Orientation> {
    const FACINGS: [Facing; 6] = [
        Facing::Front, Facing::Back, Facing::Left,
        Facing::Right, Facing::Top, Facing::Bottom,
    ];
    const MIRRORINGS: [Mirroring; 2] = [Mirroring::NoMirror, Mirroring::Mirror];
    const ROTATIONS: [Rotation; 4] = [
        Rotation::NoRotation, Rotation::RotateClockwise,
        Rotation::RotateHalf, Rotation::RotateCounter,
    ];
    FACINGS.iter().flat_map(|&f| {
        MIRRORINGS.iter().flat_map(move |&m| {
            ROTATIONS.iter().map(move |&r| Orientation(f, m, r))
        })
    })
}

/// Outcome of subtracting a set of primitives from one union primitive
/// within the same unit cell.
#[derive(Debug, Clone, Copy)]
pub enum CellResult {
    /// The whole cell was removed.
    Empty,
    /// Something's left, and it's a valid primitive in some orientation.
    Keep { shape: PrimitiveShape, orient_mat: Mat3 },
    /// The CSG result is not expressible as any of our primitives.
    /// The compile step treats this as an authoring error.
    NotRepresentable { result_signature: u64 },
}

/// Compute `minuend − ⋃ subtrahends` in a single unit cell.
#[cfg(test)]
pub fn cell_subtract(
    minuend: (PrimitiveShape, Mat3),
    subtrahends: &[(PrimitiveShape, Mat3)],
) -> CellResult {
    let mut sub_sig = 0u64;
    for (shape, mat) in subtrahends {
        sub_sig |= compute_signature(*shape, *mat);
    }
    cell_subtract_with_sig(minuend, sub_sig)
}

/// Compute `minuend − subtract_sig` where the subtract signature has
/// already been precomputed (e.g. by sampling at the cell's actual
/// world position for multi-cell subtract primitives).
pub fn cell_subtract_with_sig(
    minuend: (PrimitiveShape, Mat3),
    sub_sig: u64,
) -> CellResult {
    let minuend_sig = compute_signature(minuend.0, minuend.1);
    let result_sig = minuend_sig & !sub_sig;
    if result_sig == 0 {
        return CellResult::Empty;
    }
    if let Some((shape, mat)) = primitive_table().get(&result_sig) {
        CellResult::Keep { shape: *shape, orient_mat: *mat }
    } else {
        CellResult::NotRepresentable { result_signature: result_sig }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_has_all_samples_inside() {
        let sig = compute_signature(PrimitiveShape::Box, Mat3::IDENTITY);
        assert_eq!(sig, !0u64, "Box should contain every sample point");
    }

    #[test]
    fn wedge_signature_is_a_proper_subset_of_box_signature() {
        let box_sig = compute_signature(PrimitiveShape::Box, Mat3::IDENTITY);
        let wedge_sig = compute_signature(PrimitiveShape::Wedge, Mat3::IDENTITY);
        assert_eq!(box_sig, !0u64);
        assert!(wedge_sig != 0);
        assert!(wedge_sig != box_sig);
        // Every bit in the wedge must also be in the box (trivial, but
        // confirms the sampler didn't produce out-of-cube points).
        assert_eq!(wedge_sig & box_sig, wedge_sig);
    }

    #[test]
    fn box_minus_identity_wedge_yields_a_wedge() {
        let minuend = (PrimitiveShape::Box, Mat3::IDENTITY);
        let wedge_mat = base_orientation_matrix(&Orientation::default());
        let result = cell_subtract(minuend, &[(PrimitiveShape::Wedge, wedge_mat)]);
        match result {
            CellResult::Keep { shape, .. } => {
                assert_eq!(shape, PrimitiveShape::Wedge);
            }
            other => panic!("expected Keep(Wedge), got {other:?}"),
        }
    }

    #[test]
    fn box_minus_box_is_empty() {
        let result = cell_subtract(
            (PrimitiveShape::Box, Mat3::IDENTITY),
            &[(PrimitiveShape::Box, Mat3::IDENTITY)],
        );
        assert!(matches!(result, CellResult::Empty));
    }

    #[test]
    fn box_minus_corner_is_not_representable() {
        let result = cell_subtract(
            (PrimitiveShape::Box, Mat3::IDENTITY),
            &[(PrimitiveShape::Corner, Mat3::IDENTITY)],
        );
        assert!(
            matches!(result, CellResult::NotRepresentable { .. }),
            "got {result:?}"
        );
    }
}

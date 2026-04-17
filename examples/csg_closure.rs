//! Check whether the current primitive set is closed under CSG subtraction.
//!
//! For every pair of primitives in every orientation, compute A & !B.
//! If the result is non-zero and not in the signature table, report it.
//!
//! Usage: cargo run --example csg_closure

use bevy::math::{Mat3, Vec3};
use std::collections::{HashMap, HashSet};

// =====================================================================
// Reproduce the minimal CSG infrastructure inline
// =====================================================================

const GRID_DIM: usize = 4;
const OFFSET_X: f32 = 0.013;
const OFFSET_Y: f32 = 0.027;
const OFFSET_Z: f32 = 0.041;

fn sample_points() -> Vec<Vec3> {
    let step = 1.0 / GRID_DIM as f32;
    let mut v = Vec::new();
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
}

#[derive(Clone, Copy, Debug)]
enum Shape { Box, Wedge, Corner, InverseCorner }

const ALL_SHAPES: [Shape; 4] = [Shape::Box, Shape::Wedge, Shape::Corner, Shape::InverseCorner];

fn point_inside(shape: Shape, p: Vec3) -> bool {
    match shape {
        Shape::Box => true,
        Shape::Wedge => p.y + p.z <= 0.0,
        Shape::Corner => p.x + p.y + p.z <= -0.5,
        Shape::InverseCorner => p.x + p.y + p.z >= -0.5,
    }
}

fn compute_signature(shape: Shape, orient_mat: Mat3, samples: &[Vec3]) -> u64 {
    let inv = orient_mat.transpose();
    let mut mask = 0u64;
    for (i, p) in samples.iter().enumerate() {
        let local = inv * (*p);
        if point_inside(shape, local) {
            mask |= 1u64 << i;
        }
    }
    mask
}

/// Generate all 48 signed permutation matrices (cube symmetry group).
fn all_48_matrices() -> Vec<Mat3> {
    // Columns = where each source basis vector goes.
    // rot90 around Y: X→-Z, Y→Y, Z→X
    let rot90_y = Mat3::from_cols(
        Vec3::new(0.0, 0.0, 1.0),   // X goes to +Z
        Vec3::new(0.0, 1.0, 0.0),   // Y stays
        Vec3::new(-1.0, 0.0, 0.0),  // Z goes to -X
    );
    // rot90 around X: X→X, Y→-Z, Z→Y
    let rot90_x = Mat3::from_cols(
        Vec3::new(1.0, 0.0, 0.0),   // X stays
        Vec3::new(0.0, 0.0, -1.0),  // Y goes to -Z
        Vec3::new(0.0, 1.0, 0.0),   // Z goes to Y
    );
    // mirror X: X→-X, Y→Y, Z→Z
    let mirror_x = Mat3::from_cols(
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::Z,
    );

    fn mat_key(m: &Mat3) -> [i32; 9] {
        let c = m.to_cols_array();
        c.map(|v| (v * 10.0).round() as i32)
    }

    let mut set: Vec<Mat3> = vec![Mat3::IDENTITY];
    let mut keys: HashSet<[i32; 9]> = HashSet::new();
    keys.insert(mat_key(&Mat3::IDENTITY));

    for gen in [rot90_y, rot90_x, mirror_x] {
        loop {
            let mut new_found = false;
            let current = set.clone();
            for existing in &current {
                for new in [gen * (*existing), (*existing) * gen] {
                    let k = mat_key(&new);
                    if keys.insert(k) {
                        set.push(new);
                        new_found = true;
                    }
                }
            }
            if !new_found { break; }
        }
    }
    set
}

fn main() {
    let samples = sample_points();
    let matrices = all_48_matrices();
    println!("{} orientation matrices (expect 48)", matrices.len());

    // Build signature table: all valid signatures.
    let mut sig_to_shape: HashMap<u64, (Shape, usize)> = HashMap::new();
    let mut valid_sigs: HashSet<u64> = HashSet::new();
    valid_sigs.insert(0); // empty

    for &shape in &ALL_SHAPES {
        for (mi, mat) in matrices.iter().enumerate() {
            let sig = compute_signature(shape, *mat, &samples);
            valid_sigs.insert(sig);
            sig_to_shape.entry(sig).or_insert((shape, mi));
        }
    }
    println!("{} unique valid signatures", valid_sigs.len());

    // Check all A - B pairs.
    let mut failures = 0;
    let mut checked = 0u64;
    let mut failure_sigs: HashSet<u64> = HashSet::new();

    for &shape_a in &ALL_SHAPES {
        for mat_a in &matrices {
            let sig_a = compute_signature(shape_a, *mat_a, &samples);

            for &shape_b in &ALL_SHAPES {
                for mat_b in &matrices {
                    let sig_b = compute_signature(shape_b, *mat_b, &samples);
                    let result = sig_a & !sig_b;
                    checked += 1;

                    if result != 0 && !valid_sigs.contains(&result) {
                        if failures < 10 {
                            println!(
                                "NOT REPRESENTABLE: {:?} - {:?} = sig {:016x} ({} bits set)",
                                shape_a, shape_b, result, result.count_ones()
                            );
                        }
                        failures += 1;
                        failure_sigs.insert(result);
                    }
                }
            }
        }
    }

    println!("\nChecked {} pairs", checked);
    println!("{} not representable ({} unique signatures)", failures, failure_sigs.len());

    if failures == 0 {
        println!("\nCSG CLOSED: all subtraction results are representable!");
    } else {
        println!("\nCSG NOT CLOSED: {} unique result signatures need new primitives", failure_sigs.len());
        // Show how many bits each unrepresentable result has (indicates shape complexity).
        let mut bit_counts: Vec<u32> = failure_sigs.iter().map(|s| s.count_ones()).collect();
        bit_counts.sort();
        bit_counts.dedup();
        println!("Bit counts of unrepresentable results: {:?}", bit_counts);
    }
}

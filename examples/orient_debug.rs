use bevy::math::{Mat3, Quat, Vec3};

fn main() {
    test_orient("default [X,Y,Z]", Vec3::X, Vec3::Y, Vec3::Z);
    test_orient("mirrored [NegX,Y,Z]", Vec3::NEG_X, Vec3::Y, Vec3::Z);
    test_orient("flipped Z [X,Y,NegZ]", Vec3::X, Vec3::Y, Vec3::NEG_Z);
    test_orient("rotated 180 Y [NegX,Y,NegZ]", Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z);
    test_orient("rotated 90 Y [Z,Y,NegX]", Vec3::Z, Vec3::Y, Vec3::NEG_X);
}

fn test_orient(label: &str, right: Vec3, up: Vec3, forward: Vec3) {
    let mat = Mat3::from_cols(right, up, forward);
    let det = mat.determinant();
    let quat = Quat::from_mat3(&mat);

    println!("{label}:");
    println!("  det={det:.3}  quat=({:.3}, {:.3}, {:.3}, {:.3})",
        quat.x, quat.y, quat.z, quat.w);
    println!("  is_nan: x={} y={} z={} w={}",
        quat.x.is_nan(), quat.y.is_nan(), quat.z.is_nan(), quat.w.is_nan());

    // Test inverse * size
    let size = Vec3::new(2.0, 2.0, 4.0);
    let inv = quat.inverse();
    let local = (inv * size).abs();
    println!("  size={size}  local_size={local}");
    println!();
}

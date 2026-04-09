use bevy::math::{EulerRot, Quat, Vec3};

fn main() {
    for yaw in [0.0_f32, 45.0, 90.0, -90.0, 180.0] {
        let pitch = 45.0_f32;
        let yaw_rad = yaw.to_radians();
        let pitch_rad = pitch.to_radians();
        // Updated: uses +yaw_rad (no negation)
        let rotation = Quat::from_euler(EulerRot::YXZ, yaw_rad, -pitch_rad, 0.0);
        let position = rotation * Vec3::new(0.0, 0.0, 15.0);
        println!("yaw={yaw:>5.0}  camera at ({:>6.2}, {:>6.2}, {:>6.2})", position.x, position.y, position.z);
    }
}

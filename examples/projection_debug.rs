use bevy::math::{EulerRot, Quat, Vec3};

fn main() {
    // Compute projection at yaw=45, pitch=45 (the fixed angles for zoom computation)
    let yaw = 45.0_f32;
    let pitch = 45.0_f32;
    let yaw_rad = yaw.to_radians();
    let pitch_rad = pitch.to_radians();
    let rotation = Quat::from_euler(EulerRot::YXZ, -yaw_rad, -pitch_rad, 0.0);

    let camera_right = rotation * Vec3::X;
    let camera_up = rotation * Vec3::Y;

    println!("Fixed zoom angles: yaw={yaw}, pitch={pitch}");
    println!("Camera right: ({:.6}, {:.6}, {:.6})", camera_right.x, camera_right.y, camera_right.z);
    println!("Camera up:    ({:.6}, {:.6}, {:.6})", camera_up.x, camera_up.y, camera_up.z);

    // Project a 2x2x2 unit box centered at origin
    let corners = [
        Vec3::new(-1.0, -1.0, -1.0), Vec3::new( 1.0, -1.0, -1.0),
        Vec3::new(-1.0,  1.0, -1.0), Vec3::new( 1.0,  1.0, -1.0),
        Vec3::new(-1.0, -1.0,  1.0), Vec3::new( 1.0, -1.0,  1.0),
        Vec3::new(-1.0,  1.0,  1.0), Vec3::new( 1.0,  1.0,  1.0),
    ];

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for corner in &corners {
        let sx = corner.dot(camera_right);
        let sy = corner.dot(camera_up);
        min_x = min_x.min(sx);
        max_x = max_x.max(sx);
        min_y = min_y.min(sy);
        max_y = max_y.max(sy);
    }

    let proj_width = max_x - min_x;
    let proj_height = max_y - min_y;

    println!("\n2x2x2 box projected at yaw={yaw}, pitch={pitch}:");
    println!("  Width:  {:.6}", proj_width);
    println!("  Height: {:.6}", proj_height);

    // Ratio: projected size / AABB max extent
    // For any AABB with max_extent M, projected_width = M * ratio_w, projected_height = M * ratio_h
    // (This works because projection is linear)
    println!("\nProjection ratios (divide by max_extent 2.0):");
    println!("  Width ratio:  {:.6}", proj_width / 2.0);
    println!("  Height ratio: {:.6}", proj_height / 2.0);

    // Compute fit scale for 1100x720 window with 280+250 panels
    let viewport_width = 1100.0_f32;
    let viewport_height = 720.0_f32;
    let left_panel = 280.0_f32;
    let right_panel = 250.0_f32;
    let usable_width = viewport_width - left_panel - right_panel;
    let border = 1.1;

    let scale_w = proj_width * border / usable_width;
    let scale_h = proj_height * border / viewport_height;
    let fit_scale = scale_w.max(scale_h);

    println!("\nFor 1100x720 window, 280+250 panels:");
    println!("  Usable width: {usable_width}");
    println!("  Scale for width:  {:.6}", scale_w);
    println!("  Scale for height: {:.6}", scale_h);
    println!("  Fit scale: {:.6}", fit_scale);

    // Verify buffers
    let visible_w = usable_width * fit_scale;
    let visible_h = viewport_height * fit_scale;
    let buffer_w = (visible_w - proj_width) / visible_w * 100.0 / 2.0;
    let buffer_h = (visible_h - proj_height) / visible_h * 100.0 / 2.0;
    println!("\nBuffers at 100% zoom:");
    println!("  Horizontal: {:.1}% each side", buffer_w);
    println!("  Vertical:   {:.1}% each side", buffer_h);
}

use bevy::math::{EulerRot, Quat, Vec3};

fn main() {
    let yaw = 45.0_f32;
    let pitch = 35.264_f32;
    let yaw_rad = yaw.to_radians();
    let pitch_rad = pitch.to_radians();
    let rotation = Quat::from_euler(EulerRot::YXZ, -yaw_rad, -pitch_rad, 0.0);

    // Camera basis vectors (what directions map to screen X and screen Y)
    let camera_right = rotation * Vec3::X;
    let camera_up = rotation * Vec3::Y;
    let camera_forward = rotation * Vec3::NEG_Z;

    println!("Camera right:   ({:.4}, {:.4}, {:.4})", camera_right.x, camera_right.y, camera_right.z);
    println!("Camera up:      ({:.4}, {:.4}, {:.4})", camera_up.x, camera_up.y, camera_up.z);
    println!("Camera forward: ({:.4}, {:.4}, {:.4})", camera_forward.x, camera_forward.y, camera_forward.z);

    // Project the 8 corners of a 2x2x2 box centered at origin onto screen space
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
        // Orthographic projection: screen_x = dot(corner, camera_right)
        //                          screen_y = dot(corner, camera_up)
        let sx = corner.dot(camera_right);
        let sy = corner.dot(camera_up);
        min_x = min_x.min(sx);
        max_x = max_x.max(sx);
        min_y = min_y.min(sy);
        max_y = max_y.max(sy);
    }

    let proj_width = max_x - min_x;
    let proj_height = max_y - min_y;

    println!("\n2x2x2 box projected:");
    println!("  Screen X range: {:.4} to {:.4} (width: {:.4})", min_x, max_x, proj_width);
    println!("  Screen Y range: {:.4} to {:.4} (height: {:.4})", min_y, max_y, proj_height);

    // Now compute the required ortho scale
    // visible_width = viewport_width_pixels * scale
    // visible_height = viewport_height_pixels * scale
    // We need: proj_width * 1.1 <= usable_width * scale
    //          proj_height * 1.1 <= viewport_height * scale
    let viewport_width = 1100.0_f32;
    let viewport_height = 720.0_f32;
    let left_panel = 280.0_f32;
    let right_panel = 250.0_f32;
    let usable_width = viewport_width - left_panel - right_panel;
    let border = 1.1;

    let scale_for_width = proj_width * border / usable_width;
    let scale_for_height = proj_height * border / viewport_height;
    let fit_scale = scale_for_width.max(scale_for_height);

    println!("\nViewport: {viewport_width} x {viewport_height}");
    println!("Usable width: {usable_width}");
    println!("Scale for width:  {:.6}", scale_for_width);
    println!("Scale for height: {:.6}", scale_for_height);
    println!("Fit scale: {:.6}", fit_scale);

    // Verify: at this scale, how many world units are visible?
    println!("\nAt fit_scale {:.6}:", fit_scale);
    println!("  Visible width:  {:.4} (need {:.4})", usable_width * fit_scale, proj_width * border);
    println!("  Visible height: {:.4} (need {:.4})", viewport_height * fit_scale, proj_height * border);
}

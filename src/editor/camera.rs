use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;

#[derive(Component)]
pub struct PanZoomCamera;

pub fn zoom_camera(
    mut camera: Query<&mut Projection, With<PanZoomCamera>>,
    mut scroll: MessageReader<MouseWheel>,
) {
    for ev in scroll.read() {
        for mut projection in &mut camera {
            if let Projection::Orthographic(ortho) = projection.as_mut() {
                let factor = 1.0 - ev.y * 0.1;
                ortho.scale = (ortho.scale * factor).clamp(0.1, 10.0);
            }
        }
    }
}

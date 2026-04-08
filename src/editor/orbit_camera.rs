use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy_egui::EguiContexts;

const ISO_DISTANCE: f32 = 15.0;
const DEFAULT_PITCH: f32 = 35.264;
const DEFAULT_YAW: f32 = 45.0;
const DEFAULT_ZOOM: f32 = 0.012;
const ZOOM_MIN: f32 = 0.002;
const ZOOM_MAX: f32 = 0.5;

#[derive(Component)]
pub struct OrbitCamera;

#[derive(Resource)]
pub struct OrbitState {
    pub yaw: f32,
    pub pitch: f32,
    pub target: Vec3,
}

impl Default for OrbitState {
    fn default() -> Self {
        Self { yaw: DEFAULT_YAW, pitch: DEFAULT_PITCH, target: Vec3::ZERO }
    }
}

pub fn spawn_orbit_camera<M: Component>(commands: &mut Commands, marker: M) {
    let (position, _) = compute_camera_pose(DEFAULT_YAW, DEFAULT_PITCH, Vec3::ZERO);
    commands.spawn((
        marker,
        OrbitCamera,
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scale: DEFAULT_ZOOM,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

pub fn orbit_camera(
    mut orbit: ResMut<OrbitState>,
    mut camera: Query<(&mut Transform, &Projection), With<OrbitCamera>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: EventReader<MouseMotion>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut contexts: EguiContexts,
) {
    let egui_wants = contexts.ctx_mut().wants_pointer_input();
    let Ok((mut tf, proj)) = camera.get_single_mut() else { return };
    let scale = orthographic_scale(proj);

    handle_orbit_input(&mut orbit, &mouse, &mut motion, &keys, &time, &tf, scale, egui_wants);
    update_camera_transform(&mut tf, &orbit);
}

#[derive(Resource)]
pub struct ZoomLimits {
    pub min: f32,
    pub max: f32,
}

impl Default for ZoomLimits {
    fn default() -> Self {
        Self { min: ZOOM_MIN, max: ZOOM_MAX }
    }
}

pub fn orbit_zoom(
    mut camera: Query<&mut Projection, With<OrbitCamera>>,
    mut scroll: EventReader<MouseWheel>,
    limits: Res<ZoomLimits>,
) {
    for ev in scroll.read() {
        for mut proj in &mut camera {
            if let Projection::Orthographic(ortho) = proj.as_mut() {
                let delta = match ev.unit {
                    MouseScrollUnit::Line => -ev.y * 0.15,
                    MouseScrollUnit::Pixel => -ev.y * 0.002,
                };
                ortho.scale = (ortho.scale * (1.0 + delta)).clamp(limits.min, limits.max);
            }
        }
    }
}

fn handle_orbit_input(
    orbit: &mut OrbitState,
    mouse: &ButtonInput<MouseButton>,
    motion: &mut EventReader<MouseMotion>,
    keys: &ButtonInput<KeyCode>,
    time: &Time,
    tf: &Transform,
    scale: f32,
    egui_wants: bool,
) {
    if mouse.pressed(MouseButton::Middle) {
        for ev in motion.read() {
            let right = tf.right();
            let up = tf.up();
            orbit.target += (-ev.delta.x * right + ev.delta.y * up) * scale;
        }
    } else if mouse.pressed(MouseButton::Left) && !egui_wants {
        for ev in motion.read() {
            orbit.yaw += ev.delta.x * 0.3;
            orbit.pitch = (orbit.pitch + ev.delta.y * 0.3).clamp(-89.9, 89.9);
        }
    } else {
        motion.clear();
    }

    let speed = 60.0 * time.delta_secs();
    if keys.pressed(KeyCode::ArrowLeft) { orbit.yaw += speed; }
    if keys.pressed(KeyCode::ArrowRight) { orbit.yaw -= speed; }
    if keys.pressed(KeyCode::ArrowUp) { orbit.pitch = (orbit.pitch + speed).min(89.9); }
    if keys.pressed(KeyCode::ArrowDown) { orbit.pitch = (orbit.pitch - speed).max(-89.9); }
}

fn update_camera_transform(tf: &mut Transform, orbit: &OrbitState) {
    let (position, _) = compute_camera_pose(orbit.yaw, orbit.pitch, orbit.target);
    tf.translation = position;
    tf.look_at(orbit.target, Vec3::Y);
}

fn compute_camera_pose(yaw: f32, pitch: f32, target: Vec3) -> (Vec3, Quat) {
    let pitch_rad = pitch.to_radians();
    let yaw_rad = yaw.to_radians();
    let rotation = Quat::from_euler(EulerRot::YXZ, -yaw_rad, -pitch_rad, 0.0);
    let position = target + rotation * Vec3::new(0.0, 0.0, ISO_DISTANCE);
    (position, rotation)
}

fn orthographic_scale(projection: &Projection) -> f32 {
    match projection {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    }
}

use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy_egui::EguiContexts;

const ISO_DISTANCE: f32 = 15.0;
const DEFAULT_PITCH: f32 = 45.0;
const DEFAULT_YAW: f32 = 45.0;
const DEFAULT_ZOOM: f32 = 0.012;
const ZOOM_MIN: f32 = 0.002;
const ZOOM_MAX: f32 = 0.5;

// =====================================================================
// Components and resources
// =====================================================================

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

/// Camera intent: what the user wants the camera to do this frame.
/// Written by the input system, read by the camera system.
#[derive(Resource, Default)]
pub struct CameraIntent {
    pub orbit_delta: Vec2,   // (yaw, pitch) change in degrees
    pub pan_delta: Vec2,     // screen-space pan in pixels
    pub zoom_delta: f32,     // scroll amount
}

// =====================================================================
// Spawning
// =====================================================================

pub fn spawn_orbit_camera(commands: &mut Commands) {
    let (position, _) = compute_camera_pose(DEFAULT_YAW, DEFAULT_PITCH, Vec3::ZERO);
    commands.spawn((
        OrbitCamera,
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scale: DEFAULT_ZOOM,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

// =====================================================================
// Input interpretation — reads raw input, writes CameraIntent
// =====================================================================

pub fn read_camera_input(
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<MouseMotion>,
    mut scroll: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut contexts: EguiContexts,
    mut intent: ResMut<CameraIntent>,
) {
    let egui_wants = contexts.ctx_mut().map(|c| c.wants_pointer_input()).unwrap_or(false);

    // Reset intent each frame
    *intent = CameraIntent::default();

    // Mouse orbit (left drag)
    if mouse.pressed(MouseButton::Left) && !egui_wants {
        for ev in motion.read() {
            intent.orbit_delta.x -= ev.delta.x * 0.3;
            intent.orbit_delta.y += ev.delta.y * 0.3;
        }
    } else if mouse.pressed(MouseButton::Middle) && !egui_wants {
        // Mouse pan (middle drag)
        for ev in motion.read() {
            intent.pan_delta.x -= ev.delta.x;
            intent.pan_delta.y += ev.delta.y;
        }
    } else {
        motion.clear();
    }

    // Scroll zoom
    for ev in scroll.read() {
        intent.zoom_delta += match ev.unit {
            MouseScrollUnit::Line => -ev.y * 0.15,
            MouseScrollUnit::Pixel => -ev.y * 0.002,
        };
    }

    // Arrow key orbit
    let speed = 60.0 * time.delta_secs();
    if keys.pressed(KeyCode::ArrowRight) { intent.orbit_delta.x += speed; }
    if keys.pressed(KeyCode::ArrowLeft) { intent.orbit_delta.x -= speed; }
    if keys.pressed(KeyCode::ArrowUp) { intent.orbit_delta.y += speed; }
    if keys.pressed(KeyCode::ArrowDown) { intent.orbit_delta.y -= speed; }
}

// =====================================================================
// Camera systems — read CameraIntent, update camera state
// =====================================================================

pub fn apply_orbit(
    intent: Res<CameraIntent>,
    mut orbit: ResMut<OrbitState>,
    mut camera: Query<(&mut Transform, &Projection), With<OrbitCamera>>,
) {
    let Ok((mut tf, proj)) = camera.single_mut() else { return };
    let scale = orthographic_scale(proj);

    // Apply orbit
    orbit.yaw += intent.orbit_delta.x;
    orbit.pitch = (orbit.pitch + intent.orbit_delta.y).clamp(-89.9, 89.9);

    // Apply pan
    if intent.pan_delta != Vec2::ZERO {
        let right = tf.right();
        let up = tf.up();
        orbit.target += (intent.pan_delta.x * right + intent.pan_delta.y * up) * scale;
    }

    update_camera_transform(&mut tf, &orbit);
}

pub fn apply_zoom(
    intent: Res<CameraIntent>,
    mut camera: Query<&mut Projection, With<OrbitCamera>>,
    limits: Res<ZoomLimits>,
) {
    if intent.zoom_delta == 0.0 { return; }
    for mut proj in &mut camera {
        if let Projection::Orthographic(ortho) = proj.as_mut() {
            ortho.scale = (ortho.scale * (1.0 + intent.zoom_delta)).clamp(limits.min, limits.max);
        }
    }
}

// =====================================================================
// Internals
// =====================================================================

fn update_camera_transform(tf: &mut Transform, orbit: &OrbitState) {
    let (position, _) = compute_camera_pose(orbit.yaw, orbit.pitch, orbit.target);
    tf.translation = position;
    tf.look_at(orbit.target, Vec3::Y);
}

pub fn compute_camera_pose(yaw: f32, pitch: f32, target: Vec3) -> (Vec3, Quat) {
    let pitch_rad = pitch.to_radians();
    let yaw_rad = yaw.to_radians();
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw_rad, -pitch_rad, 0.0);
    let position = target + rotation * Vec3::new(0.0, 0.0, ISO_DISTANCE);
    (position, rotation)
}

// =====================================================================
// AABB → orthographic camera fit
// =====================================================================

/// Result of fitting an orthographic camera to an AABB at a given view angle.
pub struct FitResult {
    /// Orthographic projection scale that frames the AABB in `viewport_size`.
    pub scale: f32,
    /// AABB center — the camera should look at this point.
    pub target: Vec3,
}

/// Compute the orthographic scale and look-at target that frames an AABB
/// in a given viewport at the given view angles. The AABB's eight corners
/// are projected into view space and the actual screen-space extents are
/// measured (no cube approximation). The constraining dimension fills the
/// viewport with `border_pct` margin per side; the non-constraining
/// dimension has at least that much margin.
pub fn fit_for_aabb(
    min: Vec3,
    max: Vec3,
    viewport_size: Vec2,
    yaw_deg: f32,
    pitch_deg: f32,
    border_pct: f32,
) -> Option<FitResult> {
    if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 { return None; }
    let size = max - min;
    if size.length() < 0.001 { return None; }

    let target = (min + max) * 0.5;

    let (cam_pos, _) = compute_camera_pose(yaw_deg, pitch_deg, target);
    let view_inv = Transform::from_translation(cam_pos)
        .looking_at(target, Vec3::Y)
        .to_matrix()
        .inverse();

    let mut view_min = Vec3::splat(f32::MAX);
    let mut view_max = Vec3::splat(f32::MIN);
    for &x in &[min.x, max.x] {
        for &y in &[min.y, max.y] {
            for &z in &[min.z, max.z] {
                let view_pos = view_inv.transform_point3(Vec3::new(x, y, z));
                view_min = view_min.min(view_pos);
                view_max = view_max.max(view_pos);
            }
        }
    }

    let proj_width = view_max.x - view_min.x;
    let proj_height = view_max.y - view_min.y;
    if proj_width < 0.001 && proj_height < 0.001 { return None; }

    let border_mult = 1.0 + border_pct * 2.0;
    let scale_for_width = proj_width * border_mult / viewport_size.x;
    let scale_for_height = proj_height * border_mult / viewport_size.y;

    Some(FitResult {
        scale: scale_for_width.max(scale_for_height),
        target,
    })
}

/// Compute the directional light rotation for a given camera yaw/pitch.
/// The light is offset slightly left and above the camera view.
pub fn compute_light_rotation(yaw: f32, pitch: f32) -> Quat {
    let cam_rot = Quat::from_euler(EulerRot::YXZ, yaw.to_radians(), -pitch.to_radians(), 0.0);
    let light_offset = Quat::from_euler(EulerRot::YXZ, 15.0_f32.to_radians(), -30.0_f32.to_radians(), 0.0);
    cam_rot * light_offset
}

fn orthographic_scale(projection: &Projection) -> f32 {
    match projection {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    }
}

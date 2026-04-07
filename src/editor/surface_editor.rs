use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::{EguiContexts, egui};

use crate::surface::{self, PatternType, SurfaceDef, preset_by_name, preset_names};
use super::camera::{PanZoomCamera, zoom_camera};

const PREVIEW_SIZE: u32 = 512;

pub struct SurfaceEditorPlugin {
    pub initial_surface: SurfaceDef,
}

impl Plugin for SurfaceEditorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EditorState::new(self.initial_surface.clone()))
            .insert_resource(RenderDirty(true))
            .add_systems(Startup, setup_preview)
            .add_systems(Update, (parameter_ui, regenerate_preview, zoom_camera, pan_camera));
    }
}

// =====================================================================
// Resources
// =====================================================================

#[derive(Resource)]
struct EditorState {
    surface: SurfaceDef,
    active_preset: Option<usize>,
}

impl EditorState {
    fn new(surface: SurfaceDef) -> Self {
        Self { surface, active_preset: None }
    }
}

#[derive(Resource)]
struct RenderDirty(bool);

#[derive(Component)]
struct PreviewSprite;

// =====================================================================
// Startup
// =====================================================================

fn setup_preview(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn((
        PanZoomCamera,
        Camera2d,
        Projection::Orthographic(OrthographicProjection::default_2d()),
    ));

    let image = Image::new_fill(
        Extent3d { width: PREVIEW_SIZE, height: PREVIEW_SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let handle = images.add(image);

    commands.spawn((
        PreviewSprite,
        Sprite {
            image: handle,
            custom_size: Some(Vec2::new(PREVIEW_SIZE as f32, PREVIEW_SIZE as f32)),
            ..default()
        },
    ));
}

// =====================================================================
// Preview regeneration
// =====================================================================

fn regenerate_preview(
    mut dirty: ResMut<RenderDirty>,
    state: Res<EditorState>,
    mut images: ResMut<Assets<Image>>,
    sprites: Query<&Sprite, With<PreviewSprite>>,
) {
    if !dirty.0 { return; }
    dirty.0 = false;

    let Ok(sprite) = sprites.get_single() else { return };
    let Some(image) = images.get_mut(&sprite.image) else { return };

    let pixels = surface::render_surface(&state.surface, PREVIEW_SIZE, PREVIEW_SIZE);
    image.data = pixels;
}

// =====================================================================
// UI
// =====================================================================

fn parameter_ui(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    mut dirty: ResMut<RenderDirty>,
) {
    let ctx = contexts.ctx_mut();

    egui::SidePanel::left("surface_params").min_width(280.0).show(ctx, |ui| {
        ui.heading("Surface Editor");
        ui.separator();

        if preset_selector(ui, &mut state) { dirty.0 = true; }
        ui.separator();

        if color_editor(ui, "Base Color", &mut state.surface.base_color) { dirty.0 = true; }
        if color_editor(ui, "Color Variation", &mut state.surface.color_variation) { dirty.0 = true; }
        if pattern_selector(ui, &mut state.surface.pattern) { dirty.0 = true; }

        if state.surface.pattern == PatternType::Stripe {
            ui.label("Stripe Angle");
            if ui.add(egui::Slider::new(&mut state.surface.stripe_angle, 0.0..=360.0).suffix("°")).changed() {
                dirty.0 = true;
            }
        }

        ui.label("Noise Scale");
        if ui.add(egui::Slider::new(&mut state.surface.noise_scale, 0.01..=40.0).logarithmic(true)).changed() {
            dirty.0 = true;
        }

        ui.label("Noise Octaves");
        let mut octaves = state.surface.noise_octaves as i32;
        if ui.add(egui::Slider::new(&mut octaves, 1..=10)).changed() {
            state.surface.noise_octaves = octaves as u32;
            dirty.0 = true;
        }

        ui.label("Seed");
        if ui.add(egui::DragValue::new(&mut state.surface.seed).range(0..=9999)).changed() {
            dirty.0 = true;
        }

        ui.separator();
        if secondary_color_editor(ui, &mut state.surface) { dirty.0 = true; }

        ui.separator();
        if speckle_editor(ui, &mut state.surface) { dirty.0 = true; }
    });
}

fn preset_selector(ui: &mut egui::Ui, state: &mut EditorState) -> bool {
    let mut changed = false;
    ui.label("Presets");
    for (i, name) in preset_names().iter().enumerate() {
        let selected = state.active_preset == Some(i);
        if ui.selectable_label(selected, *name).clicked() {
            if let Some(preset) = preset_by_name(name) {
                state.surface = preset;
                state.active_preset = Some(i);
                changed = true;
            }
        }
    }
    changed
}

fn color_editor(ui: &mut egui::Ui, label: &str, color: &mut (f32, f32, f32)) -> bool {
    ui.label(label);
    let mut rgb = [color.0, color.1, color.2];
    let changed = ui.color_edit_button_rgb(&mut rgb).changed();
    if changed {
        *color = (rgb[0], rgb[1], rgb[2]);
    }
    changed
}

fn pattern_selector(ui: &mut egui::Ui, pattern: &mut PatternType) -> bool {
    let patterns = [
        ("Perlin", PatternType::Perlin),
        ("Cellular", PatternType::Cellular),
        ("Ridged", PatternType::Ridged),
        ("Stripe", PatternType::Stripe),
        ("Marble", PatternType::Marble),
        ("Turbulence", PatternType::Turbulence),
        ("Domain Warp", PatternType::DomainWarp),
    ];

    let mut changed = false;
    ui.label("Pattern");
    for (label, value) in &patterns {
        if ui.radio_value(pattern, value.clone(), *label).changed() {
            changed = true;
        }
    }
    changed
}

fn secondary_color_editor(ui: &mut egui::Ui, surface: &mut SurfaceDef) -> bool {
    let mut changed = false;
    let mut has_secondary = surface.secondary_color.is_some();

    if ui.checkbox(&mut has_secondary, "Secondary Color").changed() {
        surface.secondary_color = if has_secondary {
            Some((0.3, 0.3, 0.3))
        } else {
            None
        };
        changed = true;
    }

    if let Some(ref mut color) = surface.secondary_color {
        let mut rgb = [color.0, color.1, color.2];
        if ui.color_edit_button_rgb(&mut rgb).changed() {
            *color = (rgb[0], rgb[1], rgb[2]);
            changed = true;
        }
    }

    changed
}

fn speckle_editor(ui: &mut egui::Ui, surface: &mut SurfaceDef) -> bool {
    let mut changed = false;

    ui.label("Speckle Density");
    if ui.add(egui::Slider::new(&mut surface.speckle_density, 0.0..=0.2)).changed() {
        changed = true;
    }

    if surface.speckle_density > 0.0 {
        if color_editor(ui, "Speckle Color", &mut surface.speckle_color) {
            changed = true;
        }
    }

    changed
}

// =====================================================================
// Camera pan (delegates low-level math to helpers)
// =====================================================================

fn pan_camera(
    mut camera: Query<(&mut Transform, &Projection), With<PanZoomCamera>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut motion: EventReader<MouseMotion>,
    time: Res<Time>,
    mut contexts: EguiContexts,
) {
    let egui_wants = contexts.ctx_mut().wants_pointer_input();
    let Ok((mut transform, projection)) = camera.get_single_mut() else { return };
    let scale = match projection {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    };

    if mouse.pressed(MouseButton::Middle) && !egui_wants {
        for ev in motion.read() {
            transform.translation.x -= ev.delta.x * scale;
            transform.translation.y += ev.delta.y * scale;
        }
    } else {
        motion.clear();
    }

    let speed = 200.0 * scale * time.delta_secs();
    if keys.pressed(KeyCode::ArrowLeft) { transform.translation.x -= speed; }
    if keys.pressed(KeyCode::ArrowRight) { transform.translation.x += speed; }
    if keys.pressed(KeyCode::ArrowUp) { transform.translation.y += speed; }
    if keys.pressed(KeyCode::ArrowDown) { transform.translation.y -= speed; }
}

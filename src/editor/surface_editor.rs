use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::{EguiContexts, egui};

use crate::browser::ActiveEditor;
use crate::registry::{AssetRegistry, store::save_surface_to_file};
use crate::surface::{self, PatternType, SurfaceDef, preset_by_name, preset_names};
use super::camera::{PanZoomCamera, zoom_camera};

const PREVIEW_SIZE: u32 = 512;

// =====================================================================
// Plugin
// =====================================================================

pub struct SurfaceEditorPlugin;

impl Plugin for SurfaceEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SurfaceEditorState>()
            .init_resource::<EditorDirty>()
            .add_systems(Update, (
                // Phase 1: detect changes from activation or registry
                (
                    handle_activation,
                    sync_from_registry.run_if(is_surface_active),
                ),
                // Phase 2: UI reads/writes surface params
                parameter_ui.run_if(is_surface_active),
                // Phase 3: apply dirty flags
                (
                    regenerate_preview.run_if(is_surface_active),
                    persist_to_file.run_if(is_surface_active),
                ),
            ).chain())
            .add_systems(Update, (
                zoom_camera.run_if(is_surface_active),
                pan_camera.run_if(is_surface_active),
            ));
    }
}

fn is_surface_active(active: Res<ActiveEditor>) -> bool {
    matches!(*active, ActiveEditor::Surface { .. })
}

// =====================================================================
// Resources
// =====================================================================

#[derive(Resource, Default)]
struct SurfaceEditorState {
    surface: SurfaceDef,
    active_preset: Option<usize>,
    registry_generation: u64,
    spawned: bool,
    last_seen_editor: Option<ActiveEditor>,
}

#[derive(Resource, Default)]
struct EditorDirty {
    preview: bool,
    file: bool,
}

#[derive(Component)]
struct SurfaceEditorEntity;

#[derive(Component)]
struct PreviewSprite;

// =====================================================================
// Activation / deactivation
// =====================================================================

fn handle_activation(
    active: Res<ActiveEditor>,
    mut state: ResMut<SurfaceEditorState>,
    mut dirty: ResMut<EditorDirty>,
    registry: Res<AssetRegistry>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    existing: Query<Entity, With<SurfaceEditorEntity>>,
) {
    let current = (*active).clone();
    let changed = state.last_seen_editor.as_ref() != Some(&current);
    if !changed { return; }

    let was_surface = matches!(&state.last_seen_editor, Some(ActiveEditor::Surface { .. }));
    let is_surface = matches!(&current, ActiveEditor::Surface { .. });

    // Despawn if leaving surface editor
    if was_surface && !is_surface {
        despawn_editor(&mut commands, &existing);
        state.spawned = false;
    }

    // Spawn if entering surface editor
    if is_surface && !state.spawned {
        spawn_editor(&mut commands, &mut images);
        state.spawned = true;
        dirty.preview = true;
    }

    // Load the selected surface
    if let ActiveEditor::Surface { ref name } = current {
        if let Some(surface) = registry.get_surface(name) {
            state.surface = surface.clone();
            state.active_preset = None;
            dirty.preview = true;
        }
    }

    state.last_seen_editor = Some(current);
}

fn spawn_editor(commands: &mut Commands, images: &mut ResMut<Assets<Image>>) {
    commands.spawn((
        SurfaceEditorEntity,
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
        SurfaceEditorEntity,
        PreviewSprite,
        Sprite {
            image: handle,
            custom_size: Some(Vec2::new(PREVIEW_SIZE as f32, PREVIEW_SIZE as f32)),
            ..default()
        },
    ));
}

fn despawn_editor(commands: &mut Commands, entities: &Query<Entity, With<SurfaceEditorEntity>>) {
    for entity in entities {
        commands.entity(entity).despawn_recursive();
    }
}

// =====================================================================
// Registry sync
// =====================================================================

fn sync_from_registry(
    registry: Res<AssetRegistry>,
    mut state: ResMut<SurfaceEditorState>,
    mut dirty: ResMut<EditorDirty>,
) {
    if registry.generation == state.registry_generation { return; }
    state.registry_generation = registry.generation;

    if let Some(updated) = registry.get_surface(&state.surface.name) {
        if *updated != state.surface {
            state.surface = updated.clone();
            dirty.preview = true;
        }
    }
}

// =====================================================================
// Preview regeneration
// =====================================================================

fn regenerate_preview(
    mut dirty: ResMut<EditorDirty>,
    state: Res<SurfaceEditorState>,
    mut images: ResMut<Assets<Image>>,
    sprites: Query<&Sprite, With<PreviewSprite>>,
) {
    if !dirty.preview { return; }
    dirty.preview = false;

    let Ok(sprite) = sprites.get_single() else { return };
    let Some(image) = images.get_mut(&sprite.image) else { return };

    let pixels = surface::render_surface(&state.surface, PREVIEW_SIZE, PREVIEW_SIZE);
    image.data = pixels;
}

// =====================================================================
// File persistence
// =====================================================================

fn persist_to_file(
    mut dirty: ResMut<EditorDirty>,
    state: Res<SurfaceEditorState>,
    mut registry: ResMut<AssetRegistry>,
) {
    if !dirty.file { return; }
    dirty.file = false;

    let name = &state.surface.name;
    let path = registry.surfaces.get(name)
        .map(|r| r.path.clone())
        .unwrap_or_else(|| {
            let filename = format!("{}.surface.ron", name.replace(' ', "_").to_lowercase());
            std::path::PathBuf::from("data/surfaces").join(filename)
        });

    save_surface_to_file(&state.surface, &path);

    registry.surfaces.insert(name.clone(), crate::registry::store::RegisteredAsset {
        data: state.surface.clone(),
        path,
    });
}

// =====================================================================
// UI
// =====================================================================

fn parameter_ui(
    mut contexts: EguiContexts,
    mut state: ResMut<SurfaceEditorState>,
    mut dirty: ResMut<EditorDirty>,
    registry: Res<AssetRegistry>,
) {
    let ctx = contexts.ctx_mut();

    egui::SidePanel::left("surface_params").min_width(280.0).show(ctx, |ui| {
        ui.heading("Surface Editor");
        ui.separator();

        if preset_selector(ui, &mut state) {
            dirty.preview = true;
            dirty.file = true;
        }

        if registry_surface_selector(ui, &mut state, &registry) {
            dirty.preview = true;
        }

        ui.separator();
        surface_name_editor(ui, &mut state, &mut dirty);
        ui.separator();

        if color_editor_with_commit(ui, "Base Color", &mut state.surface.base_color, &mut dirty) {}
        if color_editor_with_commit(ui, "Color Variation", &mut state.surface.color_variation, &mut dirty) {}
        if pattern_selector_widget(ui, &mut state.surface.pattern) {
            dirty.preview = true;
            dirty.file = true;
        }

        if state.surface.pattern == PatternType::Stripe {
            slider_with_commit(ui, "Stripe Angle", &mut state.surface.stripe_angle, 0.0..=360.0, "°", &mut dirty);
        }

        slider_with_commit(ui, "Noise Scale", &mut state.surface.noise_scale, 0.01..=40.0, "", &mut dirty);

        ui.label("Noise Octaves");
        let mut octaves = state.surface.noise_octaves as i32;
        let resp = ui.add(egui::Slider::new(&mut octaves, 1..=10));
        if resp.changed() {
            state.surface.noise_octaves = octaves as u32;
            dirty.preview = true;
        }
        if resp.drag_stopped() || resp.lost_focus() { dirty.file = true; }

        ui.label("Seed");
        let resp = ui.add(egui::DragValue::new(&mut state.surface.seed).range(0..=9999));
        if resp.changed() { dirty.preview = true; }
        if resp.lost_focus() { dirty.file = true; }

        ui.separator();
        secondary_color_editor(ui, &mut state.surface, &mut dirty);
        ui.separator();
        speckle_editor(ui, &mut state.surface, &mut dirty);
    });
}

// =====================================================================
// UI helpers
// =====================================================================

fn preset_selector(ui: &mut egui::Ui, state: &mut SurfaceEditorState) -> bool {
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

fn registry_surface_selector(
    ui: &mut egui::Ui,
    state: &mut SurfaceEditorState,
    registry: &AssetRegistry,
) -> bool {
    if registry.surfaces.is_empty() { return false; }
    let mut changed = false;
    ui.separator();
    ui.label("Saved Surfaces");
    let mut names: Vec<&String> = registry.surfaces.keys().collect();
    names.sort();
    for name in names {
        let selected = state.surface.name == *name;
        if ui.selectable_label(selected, name.as_str()).clicked() {
            if let Some(surface) = registry.get_surface(name) {
                state.surface = surface.clone();
                state.active_preset = None;
                changed = true;
            }
        }
    }
    changed
}

fn surface_name_editor(ui: &mut egui::Ui, state: &mut SurfaceEditorState, dirty: &mut EditorDirty) {
    ui.label("Name");
    let resp = ui.text_edit_singleline(&mut state.surface.name);
    if resp.lost_focus() {
        dirty.file = true;
        dirty.preview = true;
    }
}

fn slider_with_commit(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    dirty: &mut EditorDirty,
) {
    ui.label(label);
    let mut slider = egui::Slider::new(value, range);
    if !suffix.is_empty() { slider = slider.suffix(suffix); }
    let resp = ui.add(slider);
    if resp.changed() { dirty.preview = true; }
    if resp.drag_stopped() || resp.lost_focus() { dirty.file = true; }
}

fn color_editor_with_commit(
    ui: &mut egui::Ui,
    label: &str,
    color: &mut (f32, f32, f32),
    dirty: &mut EditorDirty,
) -> bool {
    ui.label(label);
    let mut rgb = [color.0, color.1, color.2];
    let resp = ui.color_edit_button_rgb(&mut rgb);
    if resp.changed() {
        *color = (rgb[0], rgb[1], rgb[2]);
        dirty.preview = true;
    }
    if resp.drag_stopped() || resp.lost_focus() { dirty.file = true; }
    resp.changed()
}

fn pattern_selector_widget(ui: &mut egui::Ui, pattern: &mut PatternType) -> bool {
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

fn secondary_color_editor(ui: &mut egui::Ui, surface: &mut SurfaceDef, dirty: &mut EditorDirty) {
    let mut has_secondary = surface.secondary_color.is_some();
    if ui.checkbox(&mut has_secondary, "Secondary Color").changed() {
        surface.secondary_color = if has_secondary { Some((0.3, 0.3, 0.3)) } else { None };
        dirty.preview = true;
        dirty.file = true;
    }
    if let Some(ref mut color) = surface.secondary_color {
        color_editor_with_commit(ui, "Secondary", color, dirty);
    }
}

fn speckle_editor(ui: &mut egui::Ui, surface: &mut SurfaceDef, dirty: &mut EditorDirty) {
    slider_with_commit(ui, "Speckle Density", &mut surface.speckle_density, 0.0..=0.2, "", dirty);
    if surface.speckle_density > 0.0 {
        color_editor_with_commit(ui, "Speckle Color", &mut surface.speckle_color, dirty);
    }
}

// =====================================================================
// Camera pan
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

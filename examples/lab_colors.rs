//! Displays all RGB color combinations for a given step count.
//! Steps are applied per-channel in RGB space.
//! Hover any swatch to see RGB values ready for .shape.ron palettes.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, egui};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "RGB Color Grid".into(),
                resolution: bevy::window::WindowResolution::new(1200, 900),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .init_resource::<ColorGridState>()
        .add_systems(Update, ui_system)
        .run();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy { Red, Green, Blue, Spectrum, Saturation, Brightness }

impl SortBy {
    fn label(&self) -> &'static str {
        match self {
            SortBy::Red => "Red",
            SortBy::Green => "Green",
            SortBy::Blue => "Blue",
            SortBy::Spectrum => "Spectrum",
            SortBy::Saturation => "Saturation",
            SortBy::Brightness => "Brightness",
        }
    }
}

const SORT_OPTIONS: &[SortBy] = &[
    SortBy::Red, SortBy::Green, SortBy::Blue,
    SortBy::Spectrum, SortBy::Saturation, SortBy::Brightness,
];

#[derive(Resource)]
struct ColorGridState {
    steps: u32,
    sort_by: SortBy,
}

impl Default for ColorGridState {
    fn default() -> Self {
        Self { steps: 3, sort_by: SortBy::Spectrum }
    }
}

struct PaletteColor {
    r: f32,
    g: f32,
    b: f32,
}

impl PaletteColor {
    fn hue(&self) -> f32 {
        let hsla: Hsla = Srgba::new(self.r, self.g, self.b, 1.0).into();
        hsla.hue
    }

    fn saturation(&self) -> f32 {
        let hsla: Hsla = Srgba::new(self.r, self.g, self.b, 1.0).into();
        hsla.saturation
    }

    fn lightness(&self) -> f32 {
        let hsla: Hsla = Srgba::new(self.r, self.g, self.b, 1.0).into();
        hsla.lightness
    }
}

fn step_values(steps: u32) -> Vec<f32> {
    (0..steps).map(|i| i as f32 / (steps - 1).max(1) as f32).collect()
}

fn step_label(steps: u32) -> String {
    step_values(steps)
        .iter()
        .map(|v| format!("{}%", (v * 100.0).round() as u32))
        .collect::<Vec<_>>()
        .join(", ")
}

fn generate_palette(steps: u32) -> Vec<PaletteColor> {
    let values = step_values(steps);
    let mut colors = Vec::new();
    for &r in &values {
        for &g in &values {
            for &b in &values {
                colors.push(PaletteColor { r, g, b });
            }
        }
    }
    colors
}

fn sort_key(c: &PaletteColor, sort_by: SortBy) -> (i32, i32, i32) {
    let q = |v: f32| (v * 10000.0) as i32;
    match sort_by {
        SortBy::Red => (q(c.r), q(c.g), q(c.b)),
        SortBy::Green => (q(c.g), q(c.r), q(c.b)),
        SortBy::Blue => (q(c.b), q(c.r), q(c.g)),
        SortBy::Spectrum => (q(c.hue() / 360.0), q(c.saturation()), q(c.lightness())),
        SortBy::Saturation => (q(c.saturation()), q(c.hue() / 360.0), q(c.lightness())),
        SortBy::Brightness => (q(c.lightness()), q(c.hue() / 360.0), q(c.saturation())),
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut state: ResMut<ColorGridState>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let mut colors = generate_palette(state.steps);
    let sort_by = state.sort_by;
    colors.sort_by_key(|c| sort_key(c, sort_by));

    let total = colors.len();

    egui::TopBottomPanel::top("controls").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Steps:");
            for &n in &[2u32, 3, 4, 5] {
                if ui.selectable_label(state.steps == n, format!("{n}")).clicked() {
                    state.steps = n;
                }
            }
            ui.label(format!("({})  →  {} colors", step_label(state.steps), total));

            ui.separator();
            ui.label("Sort:");
            for &s in SORT_OPTIONS {
                if ui.selectable_label(state.sort_by == s, s.label()).clicked() {
                    state.sort_by = s;
                }
            }
        });
    });

    let cols = (total as f32).sqrt().ceil().max(1.0) as usize;

    egui::CentralPanel::default().show(ctx, |ui| {
        let available = ui.available_size();
        let swatch_size = ((available.x / cols as f32) - 2.0).max(4.0).min(80.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(1.0, 1.0);
                for color in &colors {
                    let egui_color = egui::Color32::from_rgb(
                        (color.r * 255.0) as u8,
                        (color.g * 255.0) as u8,
                        (color.b * 255.0) as u8,
                    );

                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(swatch_size, swatch_size),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(rect, 0.0, egui_color);

                    if response.hovered() {
                        response.on_hover_ui(|ui| {
                            ui.label(format!(
                                "RGB: ({:.0}, {:.0}, {:.0})",
                                color.r * 255.0,
                                color.g * 255.0,
                                color.b * 255.0,
                            ));
                            ui.label(format!(
                                "RON: ({}, {}, {})",
                                (color.r * 3.0).round() as u8,
                                (color.g * 3.0).round() as u8,
                                (color.b * 3.0).round() as u8,
                            ));
                        });
                    }
                }
            });
        });
    });
}

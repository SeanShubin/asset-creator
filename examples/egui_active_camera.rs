//! Experiment 3: egui with an ACTIVE camera at startup. Baseline.
//!
//! Hypothesis: a single active Camera2d at startup is sufficient for
//! egui to attach its primary context AND render. This is the minimal
//! working case.
//!
//! Expected outcome: panel renders. If this fails, our entire mental
//! model of bevy_egui is wrong.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Exp 3: egui with ACTIVE camera (expect panel)".into(),
                resolution: bevy::window::WindowResolution::new(800, 200),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, panel_system)
        .run();
}

fn panel_system(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Hello from egui");
        ui.label("An active Camera2d at startup is sufficient.");
    });
}

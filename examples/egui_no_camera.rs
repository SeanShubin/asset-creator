//! Experiment 1: egui with NO camera spawned.
//!
//! Hypothesis: bevy_egui 0.39 attaches `PrimaryEguiContext` to the first
//! camera that gets spawned. With no camera ever spawned, no egui context
//! exists and `ctx_mut()` returns `Err`. The panel system runs but early-
//! returns silently — the window stays blank.
//!
//! Expected outcome: blank window. Stdout shows
//! `ctx_mut() returned Err(...)`.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Exp 1: egui with NO camera (expect blank window)".into(),
                resolution: bevy::window::WindowResolution::new(800, 200),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_systems(EguiPrimaryContextPass, panel_system)
        .add_systems(Update, log_state_once)
        .run();
}

fn panel_system(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("If you can read this, the hypothesis is wrong.");
    });
}

fn log_state_once(mut contexts: EguiContexts, mut printed: Local<bool>) {
    if *printed { return; }
    *printed = true;
    match contexts.ctx_mut() {
        Ok(_) => println!("ctx_mut() returned Ok — egui context exists (hypothesis WRONG)"),
        Err(e) => println!("ctx_mut() returned Err: {e:?} — no primary context (hypothesis CONFIRMED)"),
    }
}

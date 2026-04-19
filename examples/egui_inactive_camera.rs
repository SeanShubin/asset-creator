//! Experiment 2: egui with an INACTIVE camera at startup.
//!
//! Hypothesis: a Camera with `is_active: false` does receive the
//! `PrimaryEguiContext` (so `ctx_mut()` succeeds), but inactive cameras
//! don't run their render graph, so egui has no draw target and the
//! panel never appears on screen.
//!
//! Expected outcome: blank window even though stdout shows
//! `ctx_mut() returned Ok`. This was the symptom that made us switch
//! to an active-but-low-order placeholder camera.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Exp 2: egui with INACTIVE camera (expect blank window)".into(),
                resolution: bevy::window::WindowResolution::new(800, 200),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((Camera2d, Camera { is_active: false, ..default() }));
        })
        .add_systems(EguiPrimaryContextPass, panel_system)
        .add_systems(Update, log_state_once)
        .run();
}

fn panel_system(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("If you can read this, inactive cameras DO render egui.");
    });
}

fn log_state_once(mut contexts: EguiContexts, mut printed: Local<bool>) {
    if *printed { return; }
    *printed = true;
    match contexts.ctx_mut() {
        Ok(_) => println!("ctx_mut() returned Ok — context attached to inactive camera"),
        Err(e) => println!("ctx_mut() returned Err: {e:?}"),
    }
}

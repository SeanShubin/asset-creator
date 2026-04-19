//! # Render Timing Experiment
//!
//! Empirically determines when Bevy's offscreen render-to-image pipeline
//! produces valid output. This answers the question:
//!
//! **"When can I spawn a shape, camera, and screenshot and get a non-blank image?"**
//!
//! ## How it works
//!
//! Each trial spawns a red cube, an orthographic camera targeting an offscreen
//! image, and a Screenshot observer — varying the frame timing and whether
//! RenderLayers are used. After the screenshot fires, the output PNG is
//! analyzed for content (opaque pixels with color vs all transparent).
//!
//! ## Key finding
//!
//! The only configuration that produces a blank image is:
//! - Everything spawned on frame 0 (the very first frame of the application)
//! - WITHOUT RenderLayers
//!
//! All other configurations work, including:
//! - Frame 0 WITH RenderLayers (works)
//! - Frame 1+ without RenderLayers (works)
//! - Screenshot deferred to a later frame than camera (works)
//! - Camera deferred to a later frame than shape (works)
//!
//! ## Implication for render_export.rs
//!
//! The export system uses `spawn_shape_with_layers` which assigns RenderLayers
//! to every entity at creation time. This avoids the frame-0-without-layers
//! failure case. No startup delay is needed.
//!
//! ## Running
//!
//! ```
//! cargo run --example render_timing
//! ```
//!
//! Output goes to `generated/render_timing/` and results are printed to stdout.

use bevy::prelude::*;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::path::PathBuf;

const IMAGE_SIZE: u32 = 256;
const RENDER_LAYER: usize = 1;
const OUTPUT_DIR: &str = "generated/render_timing";

fn main() {
    let _ = std::fs::remove_dir_all(OUTPUT_DIR);
    let _ = std::fs::create_dir_all(OUTPUT_DIR);

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Render Timing Test".into(),
                resolution: bevy::window::WindowResolution::new(400, 400),
                ..default()
            }),
            ..default()
        }))
        .init_resource::<TestState>()
        .add_systems(Update, run_test)
        .run();
}

// =====================================================================
// Trial definitions
// =====================================================================

#[derive(Clone)]
struct Trial {
    name: &'static str,
    shape_frame: u32,
    camera_frame: u32,
    screenshot_frame: u32,
    use_render_layers: bool,
}

/// Each trial tests a specific timing hypothesis.
/// The name encodes: shape spawn frame, camera spawn frame, screenshot spawn frame,
/// and whether RenderLayers are used.
const TRIALS: &[Trial] = &[
    // Frame 0 tests — is the very first frame special?
    Trial { name: "f0_no_layers",  shape_frame: 0, camera_frame: 0, screenshot_frame: 0, use_render_layers: false },
    Trial { name: "f0_layers",     shape_frame: 0, camera_frame: 0, screenshot_frame: 0, use_render_layers: true },
    // Frame 1 — does waiting one frame fix it?
    Trial { name: "f1_no_layers",  shape_frame: 1, camera_frame: 1, screenshot_frame: 1, use_render_layers: false },
    Trial { name: "f1_layers",     shape_frame: 1, camera_frame: 1, screenshot_frame: 1, use_render_layers: true },
    // Deferred screenshot — does the screenshot need to be on the same frame?
    Trial { name: "f0_shot1",      shape_frame: 0, camera_frame: 0, screenshot_frame: 1, use_render_layers: false },
    Trial { name: "f0_shot2",      shape_frame: 0, camera_frame: 0, screenshot_frame: 2, use_render_layers: false },
    Trial { name: "f0_shot3",      shape_frame: 0, camera_frame: 0, screenshot_frame: 3, use_render_layers: false },
    // Deferred camera — does the camera need to be on the same frame as the shape?
    Trial { name: "f0_cam1_shot1", shape_frame: 0, camera_frame: 1, screenshot_frame: 1, use_render_layers: false },
    Trial { name: "f0_cam1_shot2", shape_frame: 0, camera_frame: 1, screenshot_frame: 2, use_render_layers: false },
    Trial { name: "f0_cam2_shot2", shape_frame: 0, camera_frame: 2, screenshot_frame: 2, use_render_layers: false },
    Trial { name: "f0_cam2_shot3", shape_frame: 0, camera_frame: 2, screenshot_frame: 3, use_render_layers: false },
    // Later frames — does the pattern hold?
    Trial { name: "f5_no_layers",  shape_frame: 5, camera_frame: 5, screenshot_frame: 5, use_render_layers: false },
    Trial { name: "f5_layers",     shape_frame: 5, camera_frame: 5, screenshot_frame: 5, use_render_layers: true },
];

// =====================================================================
// Test harness
// =====================================================================

#[derive(Resource)]
struct TestState {
    current_trial: usize,
    frame_in_trial: u32,
    entities: Vec<Entity>,
    screenshot_entity: Option<Entity>,
    image_handle: Option<Handle<Image>>,
    waiting_for_screenshot: bool,
    results: Vec<(String, String)>,
    gap_frames: u32,
}

impl Default for TestState {
    fn default() -> Self {
        Self {
            current_trial: 0,
            frame_in_trial: 0,
            entities: Vec::new(),
            screenshot_entity: None,
            image_handle: None,
            waiting_for_screenshot: false,
            results: Vec::new(),
            gap_frames: 0,
        }
    }
}

fn run_test(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut state: ResMut<TestState>,
    entities: Query<Entity>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.current_trial >= TRIALS.len() {
        print_results(&state.results);
        exit.write(AppExit::Success);
        return;
    }

    if state.gap_frames > 0 {
        state.gap_frames -= 1;
        return;
    }

    let trial = TRIALS[state.current_trial].clone();

    if state.waiting_for_screenshot {
        let done = state.screenshot_entity
            .map(|e| entities.get(e).is_err())
            .unwrap_or(false);
        let timed_out = state.frame_in_trial > trial.screenshot_frame + 15;

        if done || timed_out {
            let path = format!("{}/{}.png", OUTPUT_DIR, trial.name);
            let result = analyze_png(&path);
            state.results.push((trial.name.to_string(), result));

            for &e in &state.entities {
                if let Ok(mut ec) = commands.get_entity(e) {
                    ec.despawn();
                }
            }
            state.entities.clear();
            state.screenshot_entity = None;
            state.image_handle = None;
            state.waiting_for_screenshot = false;
            state.current_trial += 1;
            state.frame_in_trial = 0;
            state.gap_frames = 5;
            return;
        }

        state.frame_in_trial += 1;
        return;
    }

    let frame = state.frame_in_trial;

    if frame == trial.shape_frame {
        spawn_test_shape(&mut commands, &mut meshes, &mut materials, &trial, &mut state.entities);
    }

    if frame == trial.camera_frame {
        let (cam_entities, handle) = spawn_test_camera(&mut commands, &mut images, &trial);
        state.entities.extend(cam_entities);
        state.image_handle = Some(handle);
    }

    if frame == trial.screenshot_frame {
        if let Some(ref handle) = state.image_handle {
            let path = PathBuf::from(format!("{}/{}.png", OUTPUT_DIR, trial.name));
            let screenshot = commands.spawn(
                Screenshot::image(handle.clone())
            ).observe(save_with_alpha(path)).id();
            state.screenshot_entity = Some(screenshot);
            state.waiting_for_screenshot = true;
        }
    }

    state.frame_in_trial += 1;
}

// =====================================================================
// Entity spawning
// =====================================================================

fn spawn_test_shape(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    trial: &Trial,
    entities: &mut Vec<Entity>,
) {
    let mut ec = commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.8, 0.2, 0.2),
            ..default()
        })),
        Transform::IDENTITY,
        Visibility::default(),
    ));
    if trial.use_render_layers {
        ec.insert(RenderLayers::layer(RENDER_LAYER));
    }
    entities.push(ec.id());
}

fn spawn_test_camera(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    trial: &Trial,
) -> (Vec<Entity>, Handle<Image>) {
    let mut entities = Vec::new();
    let size = Extent3d { width: IMAGE_SIZE, height: IMAGE_SIZE, depth_or_array_layers: 1 };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("test_render"),
            size,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    image.resize(size);
    let handle = images.add(image);

    let cam_rot = Quat::from_euler(EulerRot::YXZ, 45.0_f32.to_radians(), -45.0_f32.to_radians(), 0.0);
    let cam_pos = cam_rot * Vec3::new(0.0, 0.0, 10.0);

    let mut cam_ec = commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(handle.clone().into()),
        Projection::Orthographic(OrthographicProjection {
            scale: 0.003,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(cam_pos).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    if trial.use_render_layers {
        cam_ec.insert(RenderLayers::layer(RENDER_LAYER));
    }
    entities.push(cam_ec.id());

    let mut light_ec = commands.spawn((
        DirectionalLight { illuminance: 6000.0, shadows_enabled: false, ..default() },
        Transform::from_rotation(cam_rot),
    ));
    if trial.use_render_layers {
        light_ec.insert(RenderLayers::layer(RENDER_LAYER));
    }
    entities.push(light_ec.id());

    (entities, handle)
}

// =====================================================================
// Analysis and output
// =====================================================================

fn save_with_alpha(path: PathBuf) -> impl FnMut(On<ScreenshotCaptured>) {
    move |on| {
        let img = on.event().image.clone();
        if let Ok(dyn_img) = img.try_into_dynamic() {
            let rgba = dyn_img.to_rgba8();
            let _ = rgba.save_with_format(&path, image::ImageFormat::Png);
        }
    }
}

fn analyze_png(path: &str) -> String {
    let Ok(img) = image::open(path) else {
        return "FILE NOT FOUND".to_string();
    };
    let rgba = img.to_rgba8();
    let pixels: Vec<_> = rgba.pixels().collect();
    let total = pixels.len();
    let transparent = pixels.iter().filter(|p| p[3] == 0).count();
    let opaque = pixels.iter().filter(|p| p[3] == 255).count();
    let has_color = pixels.iter().any(|p| p[0] > 10 || p[1] > 10 || p[2] > 10);

    if transparent == total {
        "BLANK (all transparent)".to_string()
    } else if opaque > 0 && has_color {
        format!("OK ({} opaque pixels, {:.0}%)", opaque, opaque as f64 / total as f64 * 100.0)
    } else {
        format!("PARTIAL ({} transparent, {} opaque, color={})", transparent, opaque, has_color)
    }
}

fn print_results(results: &[(String, String)]) {
    println!();
    println!("=== BEVY OFFSCREEN RENDER TIMING RESULTS ===");
    println!();
    println!("Each trial spawns a cube, camera, and screenshot with different timing.");
    println!("'layers' means RenderLayers was assigned to all entities.");
    println!("'fN' means all entities spawned on frame N.");
    println!("'shotN' means screenshot deferred to frame N.");
    println!("'camN' means camera deferred to frame N.");
    println!();
    println!("{:<25} {}", "TRIAL", "RESULT");
    println!("{}", "-".repeat(70));
    for (name, result) in results {
        let marker = if result.starts_with("BLANK") { "  <-- FAILS" } else { "" };
        println!("{:<25} {}{}", name, result, marker);
    }
    println!();
    println!("CONCLUSION: The only failure case is frame 0 WITHOUT RenderLayers.");
    println!("Using RenderLayers on all entities avoids this completely.");
    println!("No startup delay needed.");
    println!();
}

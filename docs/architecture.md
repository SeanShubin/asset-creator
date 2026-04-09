# Architecture

Application structure, plugin organization, and shared systems.

## High-Level Structure

The asset creator is a single Bevy application that launches with an asset browser. All editors are accessible from one `cargo run` invocation. The left sidebar always shows the asset browser (all known assets organized by type), and the main viewport displays the active editor. Switching editors despawns the previous editor's entities and spawns new ones, while the asset registry persists across switches.

CLI arguments can skip the browser and jump directly into a specific editor:
- `cargo run` — asset browser with no editor active
- `cargo run -- surface --preset Marble` — jump to surface editor
- `cargo run -- object data/shapes/scout_bot.shape.ron` — jump to object editor

```
asset-creator/
  src/
    main.rs              # App entry point, plugin registration
    browser/
      mod.rs             # Asset browser plugin (sidebar UI, editor switching)
    editor/
      surface_editor.rs  # Surface editor (2D preview, parameter UI)
      object_editor.rs   # 3D object editor (shape viewer, part tree, animation)
      orbit_camera.rs    # 3D orbit camera
      camera.rs          # 2D pan/zoom camera
    noise/               # Pure math noise function library
    surface/             # Surface definition, renderer, presets, RON loader
    shape/               # Shape interpreter (RON -> Bevy entities), animation
    registry/            # Asset registry, file watcher, persistence
  data/
    surfaces/            # RON surface definitions
    shapes/              # RON shape files
    tilesets/            # RON tileset definitions (future)
    decals/              # RON decal definitions (future)
    worlds/              # RON world definitions (future)
  assets/
    generated/           # Export output directory
    shaders/             # WGSL shader assets
```

## Application Lifecycle

1. **Startup**: The registry scans `data/` and loads all RON files by type
2. **Asset browser**: The left sidebar displays all known assets grouped by type (surfaces, shapes, etc.) with "New" buttons for each type
3. **Editor activation**: Clicking an asset opens its editor in the main viewport. The previous editor's entities (cameras, sprites, meshes) are despawned and the new editor's entities are spawned.
4. **Live editing**: Changes in one editor are immediately visible in others via the shared registry. Editing a surface updates any object that references it.
5. **File persistence**: UI changes auto-save to disk on interaction completion. External file edits are detected by the file watcher within 500ms.

## Editor Switching

Editors are activated and deactivated dynamically. Each editor module provides:
- A `spawn` function that creates its cameras, lights, sprites, and UI state
- A `despawn` function that cleans up all its entities
- Systems that only run when the editor is active (gated by a marker resource)

```rust
#[derive(Resource)]
enum ActiveEditor {
    None,
    Surface { name: String },
    Object { path: PathBuf },
}
```

The browser plugin watches for changes to `ActiveEditor` and handles the spawn/despawn lifecycle.

## Bevy Plugin Organization

Each editor is a self-contained Bevy plugin that registers its own:
- **Resources**: Parameter structs, dirty flags, editor state
- **Startup systems**: Scene setup, camera spawning, initial asset loading
- **Update systems**: Input handling, parameter application, texture regeneration
- **UI systems**: egui panel rendering (runs in `EguiPrimaryContextPass`)

### Plugin Pattern

```rust
pub struct SurfaceEditorPlugin {
    pub initial_surface: String,
}

impl Plugin for SurfaceEditorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SurfaceParams::default())
           .insert_resource(SurfaceDirty(true))
           .add_systems(Startup, setup_preview)
           .add_systems(EguiPrimaryContextPass, parameter_ui)
           .add_systems(Update, (regenerate_preview, camera_zoom, camera_pan));
    }
}
```

## Asset Registry

The asset registry is the central data store. RON files on disk are the source of truth. All editors read from and write to the registry, which synchronizes with the filesystem.

### Registry Design

```rust
#[derive(Resource)]
struct AssetRegistry {
    surfaces: HashMap<String, RegisteredAsset<SurfaceDef>>,
    // Future: shapes, tilesets, decals, worlds
}

struct RegisteredAsset<T> {
    data: T,
    path: PathBuf,
    last_modified: SystemTime,
}
```

The registry:
- Scans `data/` on startup, loading all RON files by type
- Polls file modification times periodically (every 500ms) to detect external changes
- Provides named lookup: `registry.surfaces.get("rusted_steel")`
- When an editor modifies an asset through the UI, it updates both the registry and writes to disk

### File Watching

External changes (text editor saves) are detected by comparing `last_modified` timestamps on a timer. This avoids OS-specific file watcher dependencies while being fast enough to feel immediate.

### Cross-Editor Reactivity

When a surface changes (either from UI or file reload), any object referencing that surface by name sees the update automatically because:
1. The registry entry is updated
2. A generation counter or change event signals dependent editors
3. Dependent editors rebuild their materials/textures from the new data

## Dirty-Flag Rendering

Editors use two levels of dirty flags to separate preview responsiveness from file persistence:

### Preview Dirty

Set on every `changed()` from egui widgets. Triggers immediate re-render of the viewport preview. Fires continuously during slider drags for real-time feedback.

### File Dirty

Set only on interaction completion signals:
- `drag_stopped()` for sliders and color pickers (fires once on mouse release)
- `lost_focus()` for text inputs (fires when user clicks away or presses enter)
- `clicked()` for buttons and radio selectors (already discrete actions)

This ensures the preview stays responsive during interaction while the file is only written when the user commits a change. Moving a slider from 0.20 to 0.80 produces one file write, not sixty.

```rust
#[derive(Resource)]
struct EditorDirty {
    preview: bool,  // regenerate the viewport
    file: bool,     // write to disk
}
```

## Camera Systems

### 3D Orbit Camera

Used by the object editor, surface editor (3D mode), and any 3D preview:

- **Orbit state**: `yaw`, `pitch`, `target` point
- **Left mouse drag**: Orbit (yaw/pitch)
- **Middle mouse drag**: Pan (moves target point along camera right/up)
- **Scroll wheel**: Zoom (orthographic scale)
- **Arrow keys**: Orbit

The camera uses orthographic projection at a fixed distance for consistent isometric-style viewing. The default view angle is 45 degrees yaw, 35 degrees pitch.

```rust
let rotation = Quat::from_euler(EulerRot::YXZ, -yaw_rad, -pitch_rad, 0.0);
let position = target + rotation * Vec3::new(0.0, 0.0, ISO_DISTANCE);
transform.look_at(target, Vec3::Y);
```

### Zoom Specification

The zoom system provides deterministic, angle-independent fit scaling for the 3D object editor.

**Fit scale** is the orthographic scale at which the object fills the viewport with approximately 5% buffer on each side of the constraining dimension. It is computed using fixed projection angles (yaw=45°, pitch=45°) so the result does not depend on the user's current orbit.

**Projection math** (computed once, stored as constants):
- At yaw=45°, pitch=45°, a unit AABB projects to:
  - Screen width = `max_extent * sqrt(2)` ≈ `max_extent * 1.414`
  - Screen height = `max_extent * (1 + sqrt(2)/2)` ≈ `max_extent * 1.707`
- The constraining dimension (width or height) is whichever requires the larger scale
- Usable viewport width = window width - left panel - right panel

**Zoom percentage:**
- 100% = fit scale (object fills viewport with ~5% buffer)
- 200% = maximum zoom in (`ortho.scale = fit_scale / 2`)
- 10% = maximum zoom out (`ortho.scale = fit_scale * 10`)
- Formula: `zoom_pct = fit_scale / ortho.scale * 100`

**Behavior on shape switch** (clicking a different shape in browser):
- Recompute fit_scale from new AABB
- Set zoom to 100%
- Reset orbit to default angles

**Behavior on file edit reload** (RON file changes externally):
- Recompute fit_scale and zoom limits from new AABB
- Do NOT change ortho.scale or orbit angles
- Zoom percentage may shift (because fit_scale changed)
- If current zoom exceeds new limits, it remains until the user scrolls, then clamps

### 2D Pan/Zoom Camera

Used by surface editor (2D mode), tileset, decal, and world editors:

- **Middle mouse drag**: Pan
- **Scroll wheel**: Zoom (orthographic scale, clamped 0.1-10.0)
- **Arrow keys**: Pan at speed proportional to zoom level

## egui Integration

UI panels use `bevy_egui` with systems running in the `EguiPrimaryContextPass` schedule:

```rust
fn ui_panel(
    mut contexts: EguiContexts,
    mut params: ResMut<SurfaceParams>,
    mut dirty: ResMut<SurfaceDirty>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::SidePanel::left("panel_id").min_width(280.0).show(ctx, |ui| {
        // Parameter widgets...
        if ui.add(egui::Slider::new(&mut params.noise_scale, 1.0..=40.0)).changed() {
            dirty.0 = true;
        }
    });
}
```

All editors use a left-side panel (`SidePanel::left`) with a minimum width of 280px. Camera input checks `ctx.wants_pointer_input()` to avoid conflicts with the UI.

## Texture-to-Image Pipeline

Several editors render procedural content to a Bevy `Image` displayed on a `Sprite`:

1. Create an `Image` with `Rgba8UnormSrgb` format at the desired resolution
2. Add it to `Assets<Image>` and reference from a `Sprite`
3. In the regeneration system, write pixel data directly to `image.data`

```rust
let image = Image::new_fill(
    Extent3d { width: 512, height: 512, depth_or_array_layers: 1 },
    TextureDimension::D2,
    &[0, 0, 0, 255],
    TextureFormat::Rgba8UnormSrgb,
    RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
);
```

## Shape Interpreter

The shape interpreter (`shared/shape.rs`) converts RON data into a Bevy entity hierarchy:

1. **Parse**: `ron::de::from_str` deserializes the RON text into a `ShapeNode` directly (no wrapper struct)
2. **Import expansion**: Import references are resolved by loading the referenced `.shape.ron` file and scaling it to fit the placement bounds
3. **Mirror expansion**: Nodes with `mirror: [X]` are duplicated with negated coordinates for each axis
4. **Repeat expansion**: Nodes with `repeat` are duplicated along the specified axis
5. **Entity spawning**: Each node becomes a Bevy entity with `Transform`, `Mesh3d`, `MeshMaterial3d`, and `ShapePart` components
6. **Animation setup**: Animation states defined on any node are collected into `ShapeAnimator` components

### Component Hierarchy

```
ShapeRoot (root entity)
  ├── ShapePart("chassis") + Mesh3d + MeshMaterial3d + BaseTransform
  │     ├── ShapePart("head") + Mesh3d + MeshMaterial3d + BaseTransform
  │     │     └── ShapePart("eye") + Mesh3d + MeshMaterial3d + BaseTransform
  │     ├── ShapePart("arm_left") + Mesh3d + MeshMaterial3d + BaseTransform
  │     └── ShapePart("arm_right") + Mesh3d + MeshMaterial3d + BaseTransform  (mirrored)
  └── ShapeAnimator (animation states + phase + speed)
```

## Export Pipeline

Editors that support export can run headless from the command line:

1. Parse CLI arguments for preset and export path
2. Generate pixel data using the same procedural functions as the interactive editor
3. Write to PNG via the `image` crate
4. Exit without opening a window

```bash
cargo run -- tileset --preset Concrete --export wall.png --tile-size 128
```

## Dependencies

| Crate       | Purpose                                               |
| ----------- | ----------------------------------------------------- |
| `bevy`      | ECS framework, rendering, windowing, asset management |
| `bevy_egui` | Immediate-mode UI for editor panels                   |
| `noise`     | Perlin, Simplex noise generators                      |
| `ron`       | RON deserialization                                   |
| `serde`     | Serialization/deserialization derive macros           |
| `image`     | PNG export                                            |

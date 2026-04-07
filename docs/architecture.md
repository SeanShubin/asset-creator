# Architecture

Application structure, plugin organization, and shared systems.

## High-Level Structure

The asset creator is a Bevy application organized as a set of editor modes, each implemented as a Bevy plugin. All editors share common infrastructure for camera control, UI panels, dirty-flag rendering, and RON deserialization.

```
asset-creator/
  src/
    main.rs              # CLI argument parsing, editor mode dispatch
    editors/
      surface.rs         # Surface editor (visual appearance, 2D/3D preview)
      object.rs          # 3D object editor (shape viewer + part tree)
      tileset.rs         # 47-blob autotile tileset editor
      decal.rs           # SDF shape composer for decals
      world.rs           # Biome terrain generator
    shared/
      camera.rs          # Orbit camera (3D) and pan/zoom camera (2D)
      noise.rs           # Noise function library
      sdf.rs             # SDF primitives and boolean operations
      shape.rs           # Shape description interpreter (RON -> Bevy entities)
      surface.rs         # Surface definition, 2D renderer, WGSL material bridge
      ui.rs              # Common egui panel helpers
    shaders/
      surface.wgsl       # World-space procedural surface shader
  data/
    surfaces/            # Example RON surface definitions
    shapes/              # Example RON shape files
    tilesets/            # Example RON tileset definitions
    decals/              # Example RON decal definitions
    worlds/              # Example RON world definitions
  assets/
    generated/           # Export output directory
    shaders/             # WGSL shader assets
```

## Bevy Plugin Organization

Each editor is a self-contained Bevy plugin that registers its own:
- **Resources**: Parameter structs, dirty flags, editor state
- **Startup systems**: Scene setup, camera spawning, initial asset loading
- **Update systems**: Input handling, parameter application, texture regeneration
- **UI systems**: egui panel rendering (runs in `EguiPrimaryContextPass`)

### Plugin Pattern

```rust
pub struct TextureEditorPlugin {
    pub params: TexParams,
}

impl Plugin for TextureEditorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.params.clone())
           .insert_resource(TexDirty(true))
           .add_systems(Startup, spawn_camera_and_tileset)
           .add_systems(EguiPrimaryContextPass, parameter_ui)
           .add_systems(Update, (regenerate_tileset, camera_zoom, camera_pan));
    }
}
```

## Dirty-Flag Rendering

All editors use a dirty-flag pattern to avoid unnecessary recomputation:

1. A `Dirty(bool)` resource tracks whether parameters have changed
2. UI systems set `dirty = true` when any parameter is modified
3. The regeneration system checks `dirty`, does work only when true, then sets `dirty = false`

```rust
#[derive(Resource)]
struct RenderDirty(bool);

fn regenerate_texture(
    mut dirty: ResMut<RenderDirty>,
    params: Res<BlenderParams>,
    // ...
) {
    if !dirty.0 { return; }
    dirty.0 = false;
    // ... expensive texture generation ...
}
```

This ensures the viewport stays responsive during parameter editing while avoiding per-frame recomputation.

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

1. **Parse**: `ron::de::from_str` deserializes the RON text into `ShapeFile`
2. **Template expansion**: Template references are resolved from the `templates` map
3. **Mirror expansion**: Nodes with `mirror: X` are duplicated with negated X coordinates
4. **Repeat expansion**: Nodes with `repeat` are duplicated along the specified axis
5. **Entity spawning**: Each node becomes a Bevy entity with `Transform`, `Mesh3d`, `MeshMaterial3d`, and `ShapePart` components
6. **Animation setup**: A `ShapeAnimator` component is attached to the root with all animation states

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

| Crate | Purpose |
|-------|---------|
| `bevy` | ECS framework, rendering, windowing, asset management |
| `bevy_egui` | Immediate-mode UI for editor panels |
| `noise` | Perlin, Simplex noise generators |
| `ron` | RON deserialization |
| `serde` | Serialization/deserialization derive macros |
| `image` | PNG export |

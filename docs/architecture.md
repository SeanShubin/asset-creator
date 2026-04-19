# Architecture

Application structure, plugin organization, and shared systems.

## High-Level Structure

The asset creator is a single Bevy application focused on interactive 3D shape editing. It opens with the object editor active. The left side panel lists the shapes discovered in `data/shapes/` (and provides camera/animation/part-tree controls); the central viewport renders the current shape.

CLI:
- `cargo run` — open with no shape loaded; pick from the list
- `cargo run -- data/shapes/scout_bot.shape.ron` — open with that shape loaded

```
asset-creator/
  src/
    main.rs              # App entry point, plugin registration, CLI
    editor/
      object_editor.rs   # 3D shape viewer, left-panel UI, part tree, animation
      orbit_camera.rs    # 3D orbit camera and input handling
    shape/               # Shape interpreter (RON → Bevy entities), CSG, animation
    registry/            # Shape registry, file watcher, persistence
    render_export.rs     # Headless PNG export of every shape file
  data/
    shapes/              # `.shape.ron` files
  generated/
    renders/             # Auto-rendered PNGs (one per shape)
  assets/
    shaders/             # WGSL shader assets (currently unused)
```

## Application Lifecycle

1. **Startup**: `RegistryPlugin` scans `data/shapes/` and loads every `.shape.ron`. `ObjectEditorPlugin` spawns the orbit camera, directional light, and ambient light.
2. **Initial selection**: If a path was passed on the CLI, `CurrentShape.path` is set and the editor loads it on the first frame. Otherwise the user picks from the shape list.
3. **Live editing**: External edits to `.shape.ron` files are picked up by the file watcher within ~500ms; the registry's `shape_generation` counter increments and the editor reloads.
4. **Render export**: The render-export system queues any shape whose `.shape.ron` is newer than its corresponding PNG in `generated/renders/` and processes them one at a time in the background.

## Selection model

Shape selection is a single resource:

```rust
#[derive(Resource, Default)]
pub struct CurrentShape {
    pub path: Option<PathBuf>,
}
```

The shape-list UI writes to `CurrentShape.path`. A `detect_shape_change` system compares it against an internal `LoadedShape` resource; when they differ, it despawns the old shape entities, fires a `ReloadShape` message, and resets the orbit. There is no editor-switching scaffolding because there is only one editor.

## Shape Registry

The shape registry is the central data store. RON files on disk are the source of truth.

```rust
#[derive(Resource, Default)]
struct AssetRegistry {
    shapes: HashMap<String, ShapeEntry>,
    shape_generation: u64,
    errors: Vec<AssetError>,
}

struct ShapeEntry {
    parts: Vec<SpecNode>,
    path: PathBuf,
}
```

- Recursively scans `data/shapes/` on startup, loading every `.shape.ron`.
- Polls file modification times every 500ms via `FileWatcher` to detect external changes.
- Provides named lookup: `registry.get_shape("frz-b/chassis")` (with-or-without `.shape.ron` suffix).
- Records parse errors per-path so the editor's error panel can surface them without crashing the app.

The name `AssetRegistry` is broader than the current shape-only contents — kept that way intentionally so the structure can widen if outputs from sibling tools (textures, decals) eventually land in this app.

## File Watching

External changes are detected by comparing `last_modified` timestamps on a 500ms timer. This avoids OS-specific file-watcher dependencies while staying responsive enough that "save in your editor" feels immediate. When a shape changes, `shape_generation` increments; the object editor watches that counter and triggers a reload.

## Coordinate System

### Integer Coordinate Principle

All spatial coordinates in `.shape.ron` files are integers (`i32`). Floats appear only at the rendering boundary, when vertex positions are sent to the GPU. This gives several properties:

- **AABB tests are exact.** `min <= point && point <= max` with no epsilon.
- **Cell-level collision detection is exact.** Two primitives claiming the same integer cell is a bit-exact comparison.
- **Spatial hashing is trivial.** Integer coordinates map directly to grid cells without floor/round.
- **CSG plane classification is exact.** No ambiguity zone around splitting planes.

**Scale through nesting.** When a shape is imported at smaller bounds, the internal coordinates become fractional in the world frame — but the format author never leaves whole numbers. The `Bounds::remap` operation rescales by integer multiplication only; no division, no rounding. See `composition-model.md` for the full rationale.

**Rotations preserve integers.** All symmetry operations (`MirrorX/Y/Z`, `Rotate90_*`, `Rotate180_*`) are signed axis permutations — they map integer coordinates to integer coordinates.

### World Space (3D)

Bevy's right-handed, Y-up coordinate system:

| Axis | Direction                                 | Color |
| ---- | ----------------------------------------- | ----- |
| +X   | Right                                     | Red   |
| +Y   | Up                                        | Green |
| +Z   | Toward the camera (forward/out of screen) | Blue  |

Background grid walls in the editor match these axis colors:
- **Red wall** (YZ plane) — behind-left at default view
- **Green floor** (XZ plane) — below
- **Blue wall** (XY plane) — behind-right at default view

### Screen Space (2D Projection)

At the default camera angle (yaw=45°, pitch=45°), the 3D axes project to screen as:

| 3D Direction | Screen Direction       |
| ------------ | ---------------------- |
| +Y           | Straight up (north)    |
| +X           | Down-right (southeast) |
| +Z           | Down-left (southwest)  |

The three visible faces of a cube at default view are top (+Y), right (+X), and left (+Z).

### Camera Orbit

The camera orbits around a target point using **yaw** and **pitch**:

**Yaw** (horizontal orbit, -180° to +180°):
- 0° = Front — camera on the +Z side, looking at the -Z face
- Positive yaw = orbit right (counter-clockwise from above)
- ±90° = Right / Left
- ±180° = Back

**Pitch** (vertical orbit, -89.9° to +89.9°):
- 0° = Level
- ±89.9° = Top / Bottom

```rust
let rotation = Quat::from_euler(EulerRot::YXZ, yaw_rad, -pitch_rad, 0.0);
let position = target + rotation * Vec3::new(0.0, 0.0, ISO_DISTANCE);
transform.look_at(target, Vec3::Y);
```

**Controls:**
- **Left mouse drag**: orbit
- **Middle mouse drag**: pan (moves target along camera right/up)
- **Scroll wheel**: zoom (orthographic scale)
- **Arrow keys**: orbit
- **R**: reload current shape from disk
- **Tab**: cycle animation state

### Lighting

The directional light follows the camera orbit with a fixed offset so the lit/shadowed pattern stays consistent regardless of orbit angle. Light always appears to come from the upper-left of the screen.

### Background Grid

| Grid          | Plane | Visible when                       |
| ------------- | ----- | ---------------------------------- |
| Green floor   | XZ    | Pitch > 0 (looking from above)     |
| Green ceiling | XZ    | Pitch < 0 (looking from below)     |
| Red wall      | YZ    | Always (flips X position with yaw) |
| Blue wall     | XY    | Always (flips Z position with yaw) |

Grid cells are 1 world unit. Colored axis lines mark the origin on each plane.

### Zoom Specification

The zoom system provides deterministic, angle-independent fit scaling.

**Fit scale** is the orthographic scale at which the object fills the visible viewport with ~5% buffer per side on the constraining dimension. Computed using fixed projection angles (yaw=45°, pitch=45°) against the scene AABB (treated as a single box) so the result is independent of the user's current orbit.

**Visible viewport** is the egui central rect — the screen area left over after side panels have been drawn. Tracked each frame in the `ViewportRect` resource (logical and physical pixels). The `Camera.viewport` is set to this rect; all fit/zoom math reads viewport size from this resource.

**Zoom percentage:**
- 100% = fit scale (object fills viewport with ~5% buffer per side)
- 200% = maximum zoom in (`ortho.scale = fit_scale / 2`)
- 10% = maximum zoom out (`ortho.scale = fit_scale * 10`)
- Formula: `zoom_pct = fit_scale / ortho.scale * 100`

**On shape switch:** recompute fit_scale, set zoom to 100%, reset orbit to default angles.
**On file edit reload:** recompute fit_scale and zoom limits; do NOT change `ortho.scale` or orbit.
**On window/panel resize:** recompute fit_scale against the new viewport and scale `ortho.scale` by `new_fit / old_fit` so the current zoom percentage is preserved. If the visible viewport is degenerate (zero w/h), hold the previous valid state until a renderable viewport returns.

## egui Integration

UI runs in `EguiPrimaryContextPass` so the egui context is initialized when systems read it. The object editor panel is a single `SidePanel::left` containing: shape list, camera controls, animation controls, part tree, and an error list when the registry has parse errors.

Camera input checks `ctx.wants_pointer_input()` to avoid stealing drags from the UI.

## Shape Interpreter

The interpreter (`src/shape/`) converts authored RON into a Bevy entity hierarchy:

1. **Parse**: `ron::de::from_str` deserializes a `Vec<SpecNode>`.
2. **Symmetry expansion**: Nodes with `symmetry: [...]` are duplicated under the closure of the listed operations and deduplicated by canonicalized bounds + CSG signature.
3. **Import expansion**: Import references are resolved against the registry; the imported shape's coordinates are remapped to the placement bounds via integer multiplication.
4. **CSG fusion**: Per-cell occupancy resolves overlaps; subtract volumes remove cells; the result is a single integer-exact mesh per shape root.
5. **Entity spawning**: Each node becomes a Bevy entity with `Transform`, `Mesh3d`, `MeshMaterial3d`, and `ShapePart` components.
6. **Animation setup**: Animation states defined on any node are collected into `ShapeAnimator` components on the root.

### Component Hierarchy

```
ShapeRoot (root entity)
  ├── ShapePart("chassis") + Mesh3d + MeshMaterial3d
  │     ├── ShapePart("head") + Mesh3d + MeshMaterial3d
  │     │     └── ShapePart("eye") + Mesh3d + MeshMaterial3d
  │     ├── ShapePart("arm") + Mesh3d + MeshMaterial3d
  │     └── ShapePart("arm_1") + Mesh3d + MeshMaterial3d  (mirrored copy)
  └── ShapeAnimator (animation states + phase + speed)
```

## Render Export

`render_export.rs` runs in the same Bevy app and renders every `.shape.ron` to a PNG in `generated/renders/`. Renders happen on a dedicated render layer so they don't appear in the editor viewport. See [`render-export.md`](render-export.md) for the full pipeline.

## Future tools

Other docs in `docs/future/` describe planned standalone tools whose outputs are intended to feed the asset creator:
- **Surface editor** — procedural texture/material authoring
- **Tileset editor** — 47-blob autotile generation
- **Decal editor** — surface-overlay SDF compositions
- **World editor** — biome-based terrain generation

These are not part of the current codebase. The asset creator's `tags` field on each `SpecNode` is the eventual integration point — a tag would resolve to a surface output produced by the surface tool.

## Dependencies

| Crate       | Purpose                                               |
| ----------- | ----------------------------------------------------- |
| `bevy`      | ECS framework, rendering, windowing, asset management |
| `bevy_egui` | Immediate-mode UI for the editor panel                |
| `ron`       | RON deserialization                                   |
| `serde`     | Serialization/deserialization derive macros           |
| `image`     | PNG export                                            |

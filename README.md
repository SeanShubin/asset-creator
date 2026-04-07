# Asset Creator

A Rust/Bevy application for procedurally generating and live-editing game assets. Everything is driven by a human-readable RON text format, with real-time preview and interactive parameter tweaking via egui panels.

## Editors

| Editor | Description |
|--------|-------------|
| [Surface Editor](docs/surface-editor.md) | Define visual appearance once, render as 2D pixels or 3D shader materials |
| [Object Editor](docs/object-editor.md) | Live edit and preview 3D objects from RON definitions, with animations and hierarchical part trees |
| [Tileset Editor](docs/tileset-editor.md) | 47-blob autotile tileset generation with beveled lighting and procedural surfaces |
| [Decal Editor](docs/decal-editor.md) | 2D overlays applied on top of 3D object surfaces, composed from SDF primitives |
| [World Editor](docs/world-editor.md) | Biome-based terrain generation with noise-driven elevation, moisture, and drainage layers |

## Core Concepts

| Topic | Description |
|-------|-------------|
| [RON Format](docs/ron-format.md) | The text-based data format used to define all assets procedurally |
| [Noise Functions](docs/noise-functions.md) | Procedural noise primitives used across all editors |
| [SDF Primitives](docs/sdf-primitives.md) | Signed distance field shapes and boolean operations |
| [Architecture](docs/architecture.md) | Application structure, plugin organization, and shared systems |

## Design Principles

- **Text-first**: Every asset is defined in RON and can be version-controlled, diffed, and generated programmatically
- **Live preview**: All parameter changes are reflected immediately in the viewport
- **Procedural generation**: Noise-based textures, SDF shapes, and parametric geometry eliminate the need for external image editors
- **Composable**: Editors share primitives -- surfaces are referenced by name from tilesets, 3D objects, and world biomes; decals layer onto any surface
- **Export**: Assets can be exported as PNGs, tileset sheets, or mesh data from the command line without opening the GUI

## Tech Stack

- **Rust** + **Bevy** (ECS game engine)
- **egui** (via `bevy_egui`) for editor UI panels
- **RON** (Rusty Object Notation) for all asset definitions
- **noise** crate for procedural generation (Perlin, Simplex, Voronoi)
- **Custom WGSL shaders** for real-time procedural surfaces on 3D geometry
- **serde** for deserialization of RON data

## Quick Start

```bash
# Launch the asset browser (access all editors from one window)
cargo run

# Jump directly to a specific editor
cargo run -- surface --preset Marble
cargo run -- object data/shapes/scout_bot.shape.ron
```

## Controls (shared across editors)

| Input | Action |
|-------|--------|
| Scroll wheel | Zoom in/out |
| Middle mouse drag | Pan (2D) / Orbit (3D) |
| Left mouse drag | Orbit (3D editors) / Drag shapes (SDF editor) |
| Arrow keys | Pan / Orbit |
| Left panel | Parameter tweaking via egui |

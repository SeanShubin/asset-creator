# Asset Creator

A Rust/Bevy application for interactive 3D shape editing. Shapes are defined in human-readable RON files and previewed live in the viewport, with hot reload on external file edits.

## Dependency Upgrades

One-time tooling install:

```bash
cargo install cargo-outdated cargo-edit
```

Check current vs latest versions:

```bash
cargo outdated --root-deps-only   # just the crates declared in Cargo.toml
cargo outdated                    # full tree, direct + transitive
```

Upgrade everything (including breaking bumps) and verify it still builds — run each line, in order:

```bash
cargo upgrade --incompatible
cargo update
cargo check
```

Before bumping Bevy specifically, confirm `bevy_egui` has a matching release, and skim the [Bevy migration guides](https://bevy.org/learn/migration-guides/) for each version you cross.

## Documentation

| Topic                                          | Description                                                             |
| ---------------------------------------------- | ----------------------------------------------------------------------- |
| [Object Editor](docs/object-editor.md)         | The editor UI: shape list, camera, animations, part tree                |
| [RON Format](docs/ron-format.md)               | `.shape.ron` syntax and the `SpecNode` tree                             |
| [Composition Model](docs/composition-model.md) | Why bounds-based stretchy primitives, not a fixed-cell grid             |
| [Architecture](docs/architecture.md)           | Application structure, plugin organization, registry, coordinate system |
| [Render Export](docs/render-export.md)         | The headless PNG export pipeline                                        |
| [CSG Normals](docs/csg-normals.md)             | How fused mesh normals are computed                                     |

[`docs/future/`](docs/future/) contains design references for tools that are
planned but not implemented in this codebase: surface (texture) editor,
tileset editor, decal editor, world editor, plus the SDF and noise libraries
those tools would share. The asset creator's `tags` field on each `SpecNode`
is the eventual integration point for surface outputs.

## Design Principles

- **Text-first**: Shapes are defined in RON — version-controllable, diff-friendly, scriptable
- **Live preview**: External file edits hot-reload into the viewport within ~500ms
- **Integer-exact**: All authoring coordinates are integer; floats appear only at the GPU boundary

## Tech Stack

- **Rust** + **Bevy** (ECS game engine)
- **egui** (via `bevy_egui`) for the editor panel
- **RON** (Rusty Object Notation) for shape definitions
- **serde** for deserialization
- **image** for PNG export

## Quick Start

```bash
# Launch the editor; pick a shape from the left panel
cargo run

# Open with a specific shape loaded
cargo run -- data/shapes/scout_bot.shape.ron
```

## Controls

| Input             | Action                    |
| ----------------- | ------------------------- |
| Left mouse drag   | Orbit camera              |
| Middle mouse drag | Pan camera                |
| Scroll wheel      | Zoom (orthographic scale) |
| Arrow keys        | Orbit camera              |
| R                 | Reload current shape      |
| Tab               | Cycle animation state     |
| Left panel        | Shape list, camera, parts |

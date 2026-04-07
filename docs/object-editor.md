# Object Editor

Live edit and preview 3D objects defined in RON format, with support for animations, hierarchical part trees, templates, and symmetry combinators.

## Overview

The object editor loads a RON shape file and renders it as a tree of Bevy entities with meshes and materials. Changes to the RON file can be hot-reloaded, and the part tree is displayed in an egui side panel where individual parts can be toggled, inspected, and animated.

## Shape Definition Format

Objects are defined in RON files with this structure:

```ron
(
    templates: {
        "wheel": (
            shape: Cylinder(radius: 0.18, height: 0.1),
            color: (0.25, 0.25, 0.28),
        ),
    },
    root: (
        name: "robot",
        children: [
            (
                name: "chassis",
                shape: Box(size: (0.7, 0.35, 0.5)),
                at: (0.0, 0.55, 0.0),
                color: (0.45, 0.45, 0.50),
                children: [
                    (
                        name: "head",
                        shape: Sphere(radius: 0.18),
                        at: (0.0, 0.35, 0.0),
                        color: (0.55, 0.55, 0.60),
                        children: [
                            (
                                name: "eye",
                                shape: Sphere(radius: 0.06),
                                at: (0.0, 0.0, 0.15),
                                color: (0.9, 0.2, 0.1),
                                emissive: true,
                            ),
                        ],
                    ),
                    (
                        name: "arm",
                        shape: Box(size: (0.12, 0.4, 0.12)),
                        at: (0.41, 0.12, 0.0),
                        color: (0.25, 0.25, 0.28),
                        mirror: X,
                    ),
                ],
            ),
        ],
    ),
    animations: [
        (
            name: "walk",
            channels: [
                (part: "arm", property: Rotation, axis: X,
                 motion: Oscillate(amplitude: 0.25, speed: 10.0, offset: 0.0)),
            ],
        ),
    ],
)
```

## Primitive Shapes

| Shape      | Parameters                 | Description         |
| ---------- | -------------------------- | ------------------- |
| `Box`      | `size: (w, h, d)`          | Axis-aligned cuboid |
| `Sphere`   | `radius: f32`              | UV sphere           |
| `Cylinder` | `radius: f32, height: f32` | Vertical cylinder   |

## Node Properties

| Property   | Type              | Description                                                                                                                                                    |
| ---------- | ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`     | `String`          | Identifier used for animation targeting and UI display                                                                                                         |
| `shape`    | `PrimitiveShape`  | Optional geometry for this node                                                                                                                                |
| `at`       | `(f32, f32, f32)` | Position relative to parent                                                                                                                                    |
| `pivot`    | `(f32, f32, f32)` | Rotation pivot offset (defaults to node origin)                                                                                                                |
| `color`    | `(f32, f32, f32)` | RGB color (0.0-1.0), used when no surface is specified                                                                                                         |
| `surface`  | `String`          | Name of a [surface](surface-editor.md) to apply to this part                                                                                                   |
| `emissive` | `bool`            | Whether the material emits light                                                                                                                               |
| `orient`   | `Axis`            | Reorient the shape's primary axis (default Y). `X` rotates 90° around Z (Y→X), `Z` rotates 90° around X (Y→Z). Useful for cylinders which are Y-up by default. |
| `rotate`   | `(f32, Axis)`     | Static rotation in degrees around an axis. Converted to radians internally.                                                                                    |
| `template` | `String`          | Name of a template to instantiate                                                                                                                              |
| `children` | `[ShapeNode]`     | Child nodes in the hierarchy                                                                                                                                   |
| `mirror`   | `Axis`            | Duplicate this subtree mirrored across the given axis                                                                                                          |
| `repeat`   | `RepeatSpec`      | Repeat this node along an axis                                                                                                                                 |
| `decals`   | `[DecalInstance]` | [Decals](decal-editor.md) applied to this part's geometry                                                                                                      |

## Templates

Templates are reusable shape subtrees defined in the `templates` map. Reference them by name:

```ron
templates: {
    "wheel": (
        shape: Cylinder(radius: 0.18, height: 0.1),
        color: (0.25, 0.25, 0.28),
        orient: Z,
    ),
},
root: (
    children: [
        (template: "wheel", at: (0.4, 0.18, 0.15)),
        (template: "wheel", at: (0.4, 0.18, -0.15)),
    ],
)
```

### Template Overrides

Instance fields override template fields. If the instance specifies a property, it wins; otherwise the template's value is used. If the instance has children, they replace the template's children entirely.

```ron
// Red wheel -- overrides the template's color
(template: "wheel", at: (0.4, 0.18, 0.15), color: (1.0, 0.0, 0.0))
```

Templates can be nested (a template can reference another template).

## Mirror Combinator

The `mirror` property duplicates a node (and all its children) reflected across an axis. This is how you define symmetric robots, vehicles, or creatures with a single arm/leg definition:

```ron
(
    name: "arm",
    shape: Box(size: (0.12, 0.4, 0.12)),
    at: (0.41, 0.0, 0.0),
    mirror: X,  // creates a second arm at (-0.41, 0.0, 0.0)
)
```

## Repeat Combinator

The `repeat` property duplicates a node multiple times along an axis:

```ron
(
    name: "segment",
    shape: Box(size: (0.1, 0.1, 0.1)),
    repeat: (count: 5, spacing: 0.15, along: Z, center: true),
)
```

Repeat duplicates the entire subtree. If a repeated node has children, every copy includes all children.

## Animation System

Animations are defined as named states, each containing channels that target specific parts by name.

### Motion Types

| Motion      | Parameters                 | Description                                     |
| ----------- | -------------------------- | ----------------------------------------------- |
| `Oscillate` | `amplitude, speed, offset` | `sin(phase * speed + offset) * amplitude`       |
| `Spin`      | `rate`                     | Continuous rotation: `phase * rate`             |
| `Bob`       | `amplitude, freq`          | Time-based sine bob (independent of walk phase) |

### Animation Properties

| Property      | Description                                |
| ------------- | ------------------------------------------ |
| `Rotation`    | Rotate around the specified axis (radians) |
| `Translation` | Translate along the specified axis         |

### Example: Multi-state Animation

```ron
animations: [
    (
        name: "idle",
        channels: [
            (part: "head", property: Rotation, axis: Y,
             motion: Oscillate(amplitude: 0.1, speed: 2.0, offset: 0.0)),
        ],
    ),
    (
        name: "walk",
        channels: [
            (part: "arm", property: Rotation, axis: X,
             motion: Oscillate(amplitude: 0.25, speed: 10.0, offset: 0.0)),
            (part: "leg", property: Rotation, axis: X,
             motion: Oscillate(amplitude: 0.4, speed: 10.0, offset: 3.14)),
        ],
    ),
]
```

## Part Tree UI

The egui side panel displays the full part hierarchy with:

- Tri-state visibility toggles: `[+]` visible, `[-]` hidden, `[~]` mixed
- Clicking a node toggles visibility for the entire subtree
- Ancestor nodes are automatically shown when revealing a hidden subtree
- Animation state selector and speed slider

## Camera Controls

| Input             | Action                                   |
| ----------------- | ---------------------------------------- |
| Left mouse drag   | Orbit camera                             |
| Middle mouse drag | Pan camera                               |
| Scroll wheel      | Zoom (orthographic scale)                |
| Arrow keys        | Orbit camera                             |
| R                 | Reload shape file                        |
| F1                | Toggle debug gizmos (part origins, axes) |
| Tab               | Cycle animation state                    |

## Surface and Decal Integration

Object nodes can reference [surfaces](surface-editor.md) by name for procedural appearance, and have [decals](decal-editor.md) applied on top. When a surface is specified, it is compiled into a WGSL shader material evaluated in world space, flowing seamlessly across all parts that share the same surface. If no surface is specified, the `color` field provides a flat `StandardMaterial`.

## Command Line

```bash
# Load default shape
cargo run -- object

# Load specific RON file
cargo run -- object data/shapes/scout_bot.ron
```

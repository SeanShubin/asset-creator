# Object Editor

Live edit and preview 3D objects defined in RON format, with support for animations, hierarchical part trees, imports, and symmetry combinators.

## Overview

The object editor loads a RON shape file and renders it as a tree of Bevy entities with meshes and materials. Changes to the RON file can be hot-reloaded, and the part tree is displayed in an egui side panel where individual parts can be toggled, inspected, and animated.

## Shape Definition Format

A `.shape.ron` file is a `ShapeNode` directly -- no wrapper struct needed:

```ron
(
    name: "robot",
    children: [
        (
            name: "chassis",
            shape: Box,
            bounds: (-0.35, 0.375, -0.25, 0.35, 0.725, 0.25),
            color: (0.45, 0.45, 0.50),
            children: [
                (
                    name: "head",
                    shape: Sphere,
                    bounds: (-0.18, 0.725, -0.18, 0.18, 1.085, 0.18),
                    color: (0.55, 0.55, 0.60),
                    children: [
                        (
                            name: "eye",
                            shape: Sphere,
                            bounds: (-0.06, 0.87, 0.09, 0.06, 0.99, 0.21),
                            color: (0.9, 0.2, 0.1),
                            emissive: true,
                        ),
                    ],
                ),
                (
                    name: "arm",
                    shape: Box,
                    bounds: (0.35, 0.475, -0.06, 0.47, 0.875, 0.06),
                    color: (0.25, 0.25, 0.28),
                    mirror: [X],
                ),
            ],
        ),
    ],
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

Primitives have no parameters -- all sizing comes from `bounds`.

| Shape      | Description                                |
| ---------- | ------------------------------------------ |
| `Box`      | Axis-aligned cuboid                        |
| `Sphere`   | UV sphere                                  |
| `Cylinder` | Cylinder along its orient axis (default Y) |
| `Dome`     | Half-sphere along its orient axis          |
| `Cone`     | Cone along its orient axis                 |
| `Wedge`    | Triangular prism (ramp shape)              |
| `Torus`    | Torus around its orient axis               |

## Node Properties

| Property     | Type                             | Description                                                                                                  |
| ------------ | -------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `name`       | `String`                         | Identifier used for animation targeting and UI display                                                       |
| `shape`      | `PrimitiveShape`                 | Optional geometry for this node (Box, Sphere, Cylinder, Dome, Cone, Wedge, Torus)                            |
| `bounds`     | `(f32, f32, f32, f32, f32, f32)` | Two corners of the bounding box `(x1, y1, z1, x2, y2, z2)`. Shape fills the box; center determines position. |
| `color`      | `(f32, f32, f32)`                | RGB color (0.0-1.0), used when no surface is specified                                                       |
| `surface`    | `String`                         | Name of a [surface](surface-editor.md) to apply to this part                                                 |
| `emissive`   | `bool`                           | Whether the material emits light                                                                             |
| `orient`     | `SignedAxis`                     | Signed axis (X, -X, Y, -Y, Z, -Z) for directional shapes (Cylinder, Cone, Dome, Torus). Default Y.           |
| `rotate`     | `(f32, Axis)`                    | Static rotation in degrees around an axis. Converted to radians internally.                                  |
| `import`     | `String`                         | Name of another `.shape.ron` file to import. The imported shape is scaled to fit the placement bounds.       |
| `children`   | `[ShapeNode]`                    | Child nodes in the hierarchy                                                                                 |
| `mirror`     | `[Axis]`                         | List of axes to mirror across. `[X]` = 2 copies, `[X, Z]` = 4, `[X, Y, Z]` = 8.                              |
| `repeat`     | `RepeatSpec`                     | Repeat this node along an axis                                                                               |
| `animations` | `[Animation]`                    | Named animation states with per-part motion channels (can be on any node)                                    |
| `decals`     | `[DecalInstance]`                | [Decals](decal-editor.md) applied to this part's geometry                                                    |

## Imports

Imports reference another `.shape.ron` file by name. The imported shape is scaled to fit the placement bounds:

```ron
(
    name: "vehicle",
    children: [
        (name: "front_wheel", import: "wheel",
         bounds: (0.4, 0.0, 0.15, 0.6, 0.36, 0.45),
         mirror: [X]),
        (name: "rear_wheel", import: "wheel",
         bounds: (0.4, 0.0, -0.45, 0.6, 0.36, -0.15),
         mirror: [X]),
    ],
)
```

## Mirror Combinator

The `mirror` property takes a list of axes and duplicates a node (and all its children) reflected across each axis. This is how you define symmetric robots, vehicles, or creatures with a single arm/leg definition:

```ron
(
    name: "arm",
    shape: Box,
    bounds: (0.35, 0.475, -0.06, 0.47, 0.875, 0.06),
    mirror: [X],  // creates a second arm mirrored across X (2 copies total)
)
```

Multiple axes multiply copies: `[X]` = 2, `[X, Z]` = 4, `[X, Y, Z]` = 8.

## Repeat Combinator

The `repeat` property duplicates a node multiple times along an axis:

```ron
(
    name: "segment",
    shape: Box,
    bounds: (-0.05, -0.05, -0.05, 0.05, 0.05, 0.05),
    repeat: (count: 5, spacing: 0.15, along: Z, center: true),
)
```

Repeat duplicates the entire subtree. If a repeated node has children, every copy includes all children.

## Animation System

Animations are defined as named states, each containing channels that target specific parts by name. Any node can have an `animations` field -- it is not limited to the root node.

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

# RON Format

All assets in the application are defined in RON (Rusty Object Notation), a human-readable data format native to the Rust ecosystem. RON files are the single source of truth for procedural generation -- every shape, texture, tileset, decal, and world can be fully described in text.

## Why RON

- **Version control**: RON files diff cleanly in git, unlike binary assets
- **Procedural generation**: Scripts and tools can generate RON programmatically
- **Human-readable**: Edit assets in any text editor
- **Type-safe**: RON maps directly to Rust structs via serde deserialization
- **Composable**: Imports and references allow reuse across files

## Syntax Overview

RON is similar to Rust literal syntax:

```ron
(
    // Structs use parentheses
    name: "scout_bot",
    health: 100,

    // Tuples
    position: (1, 2, 3),

    // Enums (no parameters)
    shape: Box,

    // Bounding box (two corners, integer coordinates)
    bounds: (-2, -1, -2, 2, 1, 2),

    // Options
    color: Some((0.8, 0.2, 0.1)),
    secondary: None,

    // Lists
    children: [
        (name: "part_a"),
        (name: "part_b"),
    ],
)
```

## Asset File Types

| Extension      | Editor         | Root Type    | Description                                                     |
| -------------- | -------------- | ------------ | --------------------------------------------------------------- |
| `.surface.ron` | Surface Editor | `SurfaceDef` | Visual appearance definition (color, noise, pattern, roughness) |
| `.shape.ron`   | Object Editor  | `ShapeNode`  | 3D object with imports and animations                           |
| `.tileset.ron` | Tileset Editor | `TilesetDef` | 47-blob tileset referencing face/edge surfaces                  |
| `.decal.ron`   | Decal Editor   | `DecalDef`   | SDF shape composition for surface overlays                      |
| `.world.ron`   | World Editor   | `WorldDef`   | Biome terrain generation parameters                             |

## Shape Files (`.shape.ron`)

A `.shape.ron` file is a `ShapeNode` directly -- no wrapper struct. The format supports hierarchical composition:

```ron
(
    name: "vehicle",
    children: [
        (
            name: "chassis",
            shape: Box,
            bounds: (-0.7, 0.25, -0.4, 0.7, 0.75, 0.4),
            surface: "rusted_steel",
            decals: [
                (decal: "insignia", center: (0.0, 0.0, 0.41), scale: 0.3),
            ],
            children: [
                (name: "wheel", import: "wheel",
                 bounds: (0.5, 0.0, 0.2, 0.7, 0.36, 0.5),
                 mirror: [X, Z]),
            ],
        ),
    ],
    animations: [
        (
            name: "drive",
            channels: [
                (part: "wheel", property: Rotation, axis: Z,
                 motion: Spin(rate: 8.0)),
            ],
        ),
    ],
)
```

Key features:
- **Imports**: Reference other `.shape.ron` files by name, scaled to fit placement bounds
- **Surfaces**: Named [surface](surface-editor.md) references for visual appearance
- **Decals**: [Decal](decal-editor.md) instances placed on geometry via triplanar projection
- **Bounds**: `(x1, y1, z1, x2, y2, z2)` two corners of bounding box; shape fills the box, center determines position
- **Mirror**: `mirror: [X]` duplicates a subtree reflected across an axis. Multiple axes: `[X, Z]` = 4 copies
- **Repeat**: `repeat: (count: 5, spacing: 0.15, along: Z)` for linear arrays
- **Animations**: Named states with per-part motion channels (can be on any node)

See [Object Editor](object-editor.md) for the complete node property reference.

## Surface Files (`.surface.ron`)

```ron
(
    name: "rusted_steel",
    base_color: (0.45, 0.3, 0.2),
    color_variation: (0.15, 0.1, 0.05),
    noise_scale: 6.0,
    noise_octaves: 4,
    pattern: Ridged,
    roughness: 0.8,
    seed: 42,
)
```

Surfaces define visual appearance independently of output format. The same surface renders as CPU pixels (tilesets, exports) or GPU shaders (3D objects). See [Surface Editor](surface-editor.md) for parameter details.

## Tileset Files (`.tileset.ron`)

```ron
(
    name: "stone_wall_tileset",
    outer_fraction: 0.0,
    middle_fraction: 0.25,
    middle_surface: "stone_slope",
    bevel_angle: 45.0,
    inner_fraction: 0.75,
    inner_surface: "stone_face",
    light_angle: 135.0,
)
```

Tilesets define up to three concentric zones (outer, middle, inner) with named surface references. See [Tileset Editor](tileset-editor.md) for the complete reference.

## Decal Files (`.decal.ron`)

Defines the decal design (SDF shape composition):

```ron
(
    name: "insignia",
    shapes: [
        (primitive: Circle, x: 0.0, y: 0.0, size_a: 0.2),
        (primitive: Circle, x: 0.0, y: 0.0, size_a: 0.17,
         op: Subtraction),
        (primitive: Box, x: 0.0, y: 0.0, size_a: 0.15, size_b: 0.03,
         op: Union),
    ],
    color: (0.8, 0.5, 0.1),
)
```

Decals are placed on 3D objects as instances with position, scale, and projection mode:

```ron
decals: [
    (decal: "insignia", center: (0.0, 0.0, 0.41), scale: 0.3),
    (decal: "scratch", center: (0.2, 0.1, 0.0), scale: 0.5,
     projection: Planar(Z), opacity: 0.6),
]
```

Decals use triplanar projection by default, wrapping around edges, spheres, and arbitrary geometry. See [Decal Editor](decal-editor.md) for the primitive, operation, and projection reference.

## World Files (`.world.ron`)

```ron
(
    seed: 42,
    elevation_freq: 3.0,
    moisture_freq: 4.0,
    drainage_freq: 5.0,
    octaves: 6,
    lacunarity: 2.0,
    gain: 0.5,
    blend_width: 0.06,
    blend_samples: 8,
    detail_strength: 0.3,
    detail_freq: 20.0,
)
```

See [World Editor](world-editor.md) for the complete parameter reference.

## Angle Convention

All angle values in RON files are specified in **degrees**. They follow math convention: 0° = right (+X), counter-clockwise positive. Code converts to radians at the deserialization boundary.

Examples:
- `light_angle: 135.0` = upper-left
- `stripe_angle: 0.0` = horizontal lines
- `stripe_angle: 90.0` = vertical lines
- `bevel_angle: 45.0` = 45° slope

## Deserialization

All RON files are deserialized using `serde` with `ron::de::from_str`:

```rust
let ron_str = std::fs::read_to_string(path)?;
let shape_node: ShapeNode = ron::de::from_str(&ron_str)?;
```

Fields use `#[serde(default)]` extensively so that most properties are optional -- you only need to specify what differs from defaults.
